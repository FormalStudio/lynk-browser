//! The built-in blocker in the shell: core's [`Blocker`] on nd's request filter, the sites it is
//! off for, how many requests it stopped on each tab's page, and the padlock row that switches it
//! for a site. Rules by address and scriptlets reach a document through the filter's head; the
//! generic rules answer the classes and ids a page reports from a world of its own.
//!
//! The engine is built off the main thread at launch, from the compiled cache when the lists are
//! unchanged; until then every request is let through. The lists are refreshed daily and the uBO
//! release weekly after it is in place, and a change rebuilds it. A release's scriptlets are
//! functions in its ES modules, so the first launch on a release evaluates them once in a hidden
//! page that loads the release over the `lynk-ubo:` scheme, and the engine is rebuilt with them.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use lynk_core::adblock::{self, BlockingState};
use lynk_core::blocker::{Blocker, Decision, hide_rule};
use lynk_core::omnibox::{BlockingFacts, blocked_sentence};
use lynk_core::ublock;
use lynk_core::url::site_of;
use nd::core::WindowId;
use nd::prelude::*;
use nd::settings::Store;
use nd::webview::{
    FilterRequest, FilterResponse, JsValue, RequestFilter, SchemeRequest, SchemeResponse, ScriptMessage, UserGesture,
    UserScript, Verdict, WebView, webview,
};

use crate::page::PageEvent;
use crate::tab::{Tab, TabId};
use crate::window::{BrowserWindow, TabEvent};

/// The message a page's `lynk-blocker` world posts: `{url, first}` as the document starts,
/// `{url, refused}` when its policy refused the head's inline `script` or `style`, and
/// `{url, classes, ids}` with the classes and ids it uses.
const COSMETIC_MESSAGE: &str = "lynkCosmetic";
const COSMETIC_WORLD: &str = "lynk-blocker";
/// Says the document started, listens for its policy refusing inline code, then reports the
/// classes and ids not reported yet: once the document has parsed, then as nodes arrive, at most
/// every half second.
const COSMETIC_REPORT: &str = "(() => { const seen = new Set(), refused = new Set(); let wait = 0; \
    const send = (body) => window.webkit?.messageHandlers?.lynkCosmetic?.postMessage({ url: location.href, ...body }); \
    send({ first: true }); \
    document.addEventListener('securitypolicyviolation', (e) => { \
      const kind = e.effectiveDirective.split('-')[0]; \
      if (e.blockedURI === 'inline' && (kind === 'script' || kind === 'style') && !refused.has(kind)) { \
        refused.add(kind); send({ refused: kind }); } }); \
    const post = () => { wait = 0; const classes = [], ids = []; \
      for (const el of document.querySelectorAll('[class],[id]')) { \
        for (const c of el.classList) if (!seen.has('.' + c)) { seen.add('.' + c); classes.push(c); } \
        if (el.id && !seen.has('#' + el.id)) { seen.add('#' + el.id); ids.push(el.id); } } \
      if (classes.length || ids.length) send({ classes, ids }); }; \
    const later = () => { if (!wait) wait = setTimeout(post, 500); }; \
    const start = () => { post(); new MutationObserver(later).observe(document.documentElement, \
      { childList: true, subtree: true, attributes: true, attributeFilter: ['class', 'id'] }); }; \
    if (document.readyState === 'loading') addEventListener('DOMContentLoaded', start); else start(); })()";
/// Adds a style sheet to the document from the world. A constructed sheet is outside the page's
/// content security policy, which can refuse the head's `<style>`.
const ADOPT: &str = "(css) => { const s = new CSSStyleSheet(); s.replaceSync(css); \
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, s]; }";

/// Runs a document's procedural rules (adblock-rust's JSON) over it, then again as it changes.
/// Each rule's operators narrow the document down step by step; what is left is hidden, or gets
/// the rule's action. Called with the rules, once per document.
const PROCEDURAL: &str = r#"(rules) => { if (window.lynkProcedural) return; window.lynkProcedural = true;
  const regex = (arg) => { const m = /^\/(.*)\/([dgimsuy]*)$/.exec(arg); return m ? new RegExp(m[1], m[2]) : null; };
  const test = (arg) => { const r = regex(arg); return r ? (s) => r.test(s) : (s) => s.includes(arg); };
  const exact = (arg) => { const r = regex(arg); return r ? (s) => r.test(s) : (s) => s === arg; };
  const unquote = (s) => s.trim().replace(/^"(.*)"$/s, '$1');
  const step = (nodes, op) => {
    const out = new Set(), a = op.arg;
    for (const n of nodes) {
      switch (op.type) {
        case 'css-selector': {
          const sel = a.trim();
          if (n === document) { document.querySelectorAll(sel).forEach((e) => out.add(e)); break; }
          if (sel[0] === '+' || sel[0] === '~') {
            const rest = sel.slice(1).trim();
            for (let e = n.nextElementSibling; e; e = e.nextElementSibling) {
              if (e.matches(rest)) out.add(e);
              if (sel[0] === '+') break;
            }
          } else n.querySelectorAll(':scope ' + sel).forEach((e) => out.add(e));
          break;
        }
        case 'has-text': if (test(a)(n.textContent)) out.add(n); break;
        case 'min-text-length': if (n.textContent.length >= Number(a)) out.add(n); break;
        case 'matches-path': if (test(a)(location.pathname + location.search)) out.add(n); break;
        case 'matches-attr': {
          const [name, value] = a.split(/=(.*)/s).map(unquote);
          const [nm, vm] = [exact(name), value === undefined ? null : exact(value)];
          if (n.getAttributeNames?.().some((k) => nm(k) && (!vm || vm(n.getAttribute(k))))) out.add(n);
          break;
        }
        case 'matches-css': case 'matches-css-before': case 'matches-css-after': {
          const at = a.indexOf(':'), prop = a.slice(0, at).trim(), value = a.slice(at + 1).trim();
          const pseudo = op.type === 'matches-css' ? null : '::' + op.type.slice(12);
          if (exact(value)(getComputedStyle(n, pseudo).getPropertyValue(prop))) out.add(n);
          break;
        }
        case 'upward': {
          let e = n;
          if (/^\d+$/.test(a.trim())) for (let i = Number(a); e && i > 0; i--) e = e.parentElement;
          else e = n.parentElement?.closest(a);
          if (e) out.add(e);
          break;
        }
        case 'xpath': {
          const r = document.evaluate(a, n, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
          for (let i = 0; i < r.snapshotLength; i++) if (r.snapshotItem(i).nodeType === 1) out.add(r.snapshotItem(i));
          break;
        }
      }
    }
    return [...out];
  };
  const act = (e, action) => {
    switch (action?.type) {
      case 'remove': e.remove(); break;
      case 'style': e.style.cssText += ';' + action.arg; break;
      case 'remove-attr': e.removeAttribute(action.arg); break;
      case 'remove-class': e.classList.remove(action.arg); break;
      default: e.style.setProperty('display', 'none', 'important');
    }
  };
  const run = () => { for (const r of rules) {
    try { let nodes = [document]; for (const op of r.selector) nodes = step(nodes, op); nodes.forEach((e) => act(e, r.action)); }
    catch (_) {} } };
  run();
  let wait = 0;
  new MutationObserver(() => { if (!wait) wait = setTimeout(() => { wait = 0; run(); }, 100); })
    .observe(document, { childList: true, subtree: true, attributes: true, characterData: true });
}"#;

const SCRIPTLET_SCHEME: &str = "lynk-ubo";
const SCRIPTLET_ORIGIN: &str = "lynk-ubo://release";
/// The message the hidden page posts the scriptlets to, `[{name, aliases, source, dependencies}]`.
const SCRIPTLET_MESSAGE: &str = "lynkScriptlets";
const SCRIPTLET_PAGE: &str = "<!doctype html><script type=module>\
    import { builtinScriptlets } from './js/resources/scriptlets.js'; \
    window.webkit.messageHandlers.lynkScriptlets.postMessage(builtinScriptlets.map(s => ({ \
      name: s.name, aliases: s.aliases ?? [], source: s.fn.toString(), dependencies: s.dependencies ?? [] })));\
    </script>";

/// What the filter reads on Chromium's IO thread.
#[derive(Default)]
struct Shared {
    blocker: RwLock<Option<Arc<Blocker>>>,
    off: RwLock<HashSet<String>>,
}

static SHARED: LazyLock<Shared> = LazyLock::new(Shared::default);

/// The blocker for a request from a page at `source`, unless blocking is off there.
fn blocker_for(source: &str) -> Option<Arc<Blocker>> {
    if SHARED.off.read().unwrap_or_else(PoisonError::into_inner).contains(&site_of(source)) {
        return None;
    }
    SHARED.blocker.read().unwrap_or_else(PoisonError::into_inner).clone()
}

struct State {
    store: Arc<Store<BlockingState>>,
    dir: PathBuf,
    /// The addresses stopped on each tab's page since its load began. A set, since Chromium asks
    /// for a resource more than once in one load (its preload scanner, then the parser).
    blocked: HashMap<TabId, HashSet<String>>,
    scriptlets: Option<Evaluating>,
}

/// The hidden page evaluating a release's scriptlets, held in one browser window.
struct Evaluating {
    page: WebView,
    release: PathBuf,
    version: String,
    window: WindowId,
}

struct Filter;

fn source<'a>(r: &FilterRequest<'a>) -> &'a str {
    if r.first_party.is_empty() { r.initiator } else { r.first_party }
}

impl RequestFilter for Filter {
    fn request(&self, r: &FilterRequest<'_>) -> Verdict {
        let Some(blocker) = blocker_for(source(r)) else { return Verdict::Allow };
        match blocker.check(r.url, source(r), r.kind.name(), r.method) {
            Decision::Allow => Verdict::Allow,
            Decision::Block => Verdict::Block,
            Decision::Redirect(url) => Verdict::Redirect(url),
            Decision::Respond { mime, body } => Verdict::Respond { mime, body: body.into() },
        }
    }

    fn response(&self, r: &FilterRequest<'_>, _: &FilterResponse<'_>) -> Option<String> {
        blocker_for(source(r))?.csp(r.url, source(r), r.kind.name())
    }

    fn head(&self, r: &FilterRequest<'_>, _: &FilterResponse<'_>) -> Option<String> {
        Some(blocker_for(source(r))?.cosmetics(r.url).head())
    }
}

/// Opens `blocking.json`, puts the filter in place and builds the engine off the main thread.
pub fn init(cx: &mut App) {
    let data = nd::system::app_dirs().data();
    let dir = match adblock::adblock_dir(&data) {
        Ok(dir) => dir,
        Err(e) => {
            eprintln!("lynk: ad blocking is off: {}: {e}", data.display());
            return;
        }
    };
    if let Err(e) = nd::webview::register_scheme(SCRIPTLET_SCHEME) {
        eprintln!("lynk: scriptlets are off: {e}");
    }
    let store = Arc::new(adblock::open(&data));
    set_off(&store.get());
    cx.set_global(State { store: store.clone(), dir: dir.clone(), blocked: HashMap::new(), scriptlets: None });
    nd::webview::set_request_filter(Filter);
    build(store, dir);
}

fn set_off(state: &BlockingState) {
    *SHARED.off.write().unwrap_or_else(PoisonError::into_inner) = state.off_sites.iter().cloned().collect();
}

/// Builds the engine from what is on disk, has the scriptlets evaluated when the release has
/// none cached yet, then refreshes what is due and rebuilds when that changed anything. Call it
/// on the main thread.
fn build(store: Arc<Store<BlockingState>>, dir: PathBuf) {
    let (missing, scriptlets) = tokio::sync::oneshot::channel();
    nd::system::spawn(async move { scriptlets.await.ok() }, |missing, cx| {
        if let Some((release, version)) = missing {
            evaluate(cx, release, version);
        }
    });
    std::thread::spawn(move || {
        let Some((release, lists)) = prepared(&store, &dir) else { return };
        install(&store, &dir, &release, &lists);
        let version = store.get().release.version.clone();
        if !ublock::resources_path(&dir, &version).exists() {
            let _ = missing.send((release.clone(), version));
        }
        let now = lynk_core::now_ms();
        let moved = adblock::update_release(&store, &dir, now, false, ublock::latest_release, ublock::download_zip);
        let fresh = adblock::refresh_lists(&store, &dir, &lists, now, false, adblock::fetch_list);
        if (matches!(moved, Ok(Some(_))) || fresh > 0)
            && let Some((release, lists)) = prepared(&store, &dir)
        {
            install(&store, &dir, &release, &lists);
        }
    });
}

fn prepared(store: &Store<BlockingState>, dir: &Path) -> Option<(PathBuf, Vec<ublock::FilterList>)> {
    let release = adblock::prepare_release(store, dir, ublock::download_zip)
        .inspect_err(|e| eprintln!("lynk: ad blocking is off: {e}"))
        .ok()?;
    let lists = ublock::default_lists(&release).inspect_err(|e| eprintln!("lynk: ad blocking is off: {e}")).ok()?;
    Some((release, lists))
}

fn install(store: &Store<BlockingState>, dir: &Path, release: &Path, lists: &[ublock::FilterList]) {
    let version = store.get().release.version.clone();
    let resources = || ublock::load_resources(dir, release, &version);
    let key = adblock::engine_key(dir, &version, lists);
    let engine = adblock::restore_engine(dir, &key, resources()).unwrap_or_else(|| {
        let engine = adblock::build_engine(dir, lists, resources());
        if let Err(e) = adblock::save_engine(dir, &key, &engine) {
            eprintln!("lynk: the compiled lists were not cached: {e}");
        }
        engine
    });
    let blocker = Blocker::new(engine, &store.get(), resources());
    *SHARED.blocker.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(blocker));
}

/// Puts the user's rules as they stand now in the blocker, keeping its compiled lists.
fn user_rules_changed(store: &Store<BlockingState>) {
    let mut blocker = SHARED.blocker.write().unwrap_or_else(PoisonError::into_inner);
    if let Some(b) = blocker.as_ref() {
        *blocker = Some(Arc::new(b.with_user(&store.get())));
    }
}

/// Hides `selector` on `site` from the next document there on.
pub fn hide(site: &str, selector: &str, label: &str, note: &str, cx: &mut App) {
    let Some(state) = cx.try_global::<State>() else { return };
    let store = state.store.clone();
    let mut changed = false;
    store.update(|s| changed = s.hide(site, selector, label, note, lynk_core::now_ms()));
    if changed {
        user_rules_changed(&store);
    }
}

/// Shows every element hidden on `site` again, from the next document there on.
pub fn restore_hidden(site: &str, cx: &mut App) {
    let Some(state) = cx.try_global::<State>() else { return };
    let store = state.store.clone();
    store.update(|s| s.restore_hidden(site, None));
    user_rules_changed(&store);
}

/// Opens the hidden page for `release`'s scriptlets in the front browser window.
fn evaluate(cx: &mut App, release: PathBuf, version: String) {
    let Some((window, entity)) = crate::app::windows(cx).into_iter().next() else { return };
    let page = WebView::new();
    page.register_script_message(SCRIPTLET_MESSAGE, None, [SCRIPTLET_ORIGIN]);
    page.load(format!("{SCRIPTLET_ORIGIN}/scriptlets.html"));
    cx.global_mut::<State>().scriptlets = Some(Evaluating { page, release, version, window });
    entity.update(cx, |_, cx| cx.notify());
}

/// The hidden scriptlet page while `win` holds it, for the window's holder.
pub fn held(win: &BrowserWindow, cx: &App) -> Option<AnyElement> {
    let e = cx.try_global::<State>()?.scriptlets.as_ref().filter(|e| e.window == win.id)?;
    let release = e.release.clone();
    let page = webview("ubo-scriptlets", &e.page)
        .on_scheme_request(move |r: &SchemeRequest, _, _| serve(&release, r))
        .on_script_message(|m: &ScriptMessage, _, cx| scriptlets_posted(m, cx));
    Some(div().size_0().child(page).into_any_element())
}

fn serve(release: &Path, r: &SchemeRequest) {
    let path = r.url.strip_prefix(SCRIPTLET_ORIGIN).unwrap_or_default();
    let response = if path == "/scriptlets.html" {
        SchemeResponse::ok("text/html", SCRIPTLET_PAGE)
    } else if let Some((file, mime)) = ublock::release_file(release, path)
        && let Ok(body) = std::fs::read(file)
    {
        SchemeResponse::ok(mime, body)
    } else {
        SchemeResponse::ok("text/plain", "").status(404)
    };
    r.responder.respond(response);
}

/// Caches the scriptlets the hidden page posted, closes it and rebuilds the engine with them.
fn scriptlets_posted(m: &ScriptMessage, cx: &mut App) {
    if m.name != SCRIPTLET_MESSAGE || !m.allowed(&[SCRIPTLET_ORIGIN.to_owned()]) {
        return;
    }
    let Some(state) = cx.try_global::<State>() else { return };
    let Some(e) = &state.scriptlets else { return };
    if let Err(why) = ublock::save_scriptlets(&state.dir, &e.version, &m.body) {
        eprintln!("lynk: scriptlets are off: {why}");
    }
    let (store, dir, window) = (state.store.clone(), state.dir.clone(), e.window);
    cx.global_mut::<State>().scriptlets = None;
    if let Some(w) = crate::app::window(cx, window) {
        w.update(cx, |_, cx| cx.notify());
    }
    build(store, dir);
}

/// A window listener: a hidden scriptlet page it held goes; the next launch tries again.
pub fn forget(cx: &mut App, win: WindowId) {
    if let Some(state) = cx.try_global_mut::<State>()
        && state.scriptlets.as_ref().is_some_and(|e| e.window == win)
    {
        state.scriptlets = None;
    }
}

/// Sets up a new page to report the classes and ids it uses.
pub fn prepare(view: &WebView) {
    view.register_script_message(COSMETIC_MESSAGE, Some(COSMETIC_WORLD), ["*"]);
    view.add_user_script(
        UserScript::new("lynk-cosmetic", COSMETIC_REPORT).in_world(COSMETIC_WORLD).at_start().all_frames(),
    );
}

/// A page observer: counts what the filter stopped, starting over with each load, and hides
/// what the generic rules match among the classes and ids a page reported.
pub fn observe(win: &mut BrowserWindow, id: TabId, event: &PageEvent, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(state) = cx.try_global::<State>() else { return };
    match event {
        PageEvent::RequestBlocked(e) => {
            cx.global_mut::<State>().blocked.entry(id).or_default().insert(e.url.clone());
            cx.notify();
        }
        PageEvent::LoadingChanged(e) if e.loading => {
            if state.blocked.contains_key(&id) {
                cx.global_mut::<State>().blocked.remove(&id);
                cx.notify();
            }
        }
        PageEvent::ScriptMessage(m) if m.name == COSMETIC_MESSAGE => {
            if let Some(view) = win.tab(id).and_then(Tab::view) {
                cosmetic_message(view, m);
            }
        }
        _ => {}
    }
}

/// Answers the `lynk-blocker` world. The head already holds the address's hide rules and
/// scriptlets, unless the page's policy refused them: the rules go in again as a constructed
/// sheet and, when refused, the scriptlets run through the DevTools protocol, which the policy
/// does not govern.
fn cosmetic_message(view: &WebView, m: &ScriptMessage) {
    let Some(url) = m.body["url"].as_str() else { return };
    let Some(blocker) = blocker_for(url) else { return };
    let cosmetics = blocker.cosmetics(url);
    if m.body["first"].as_bool() == Some(true) {
        if !cosmetics.hide.is_empty() {
            adopt(view, m, &hide_rule(&cosmetics.hide));
        }
        if !cosmetics.procedural.is_empty() {
            // Each rule is JSON already, so the list is one too.
            let code = format!("({PROCEDURAL})([{}])", cosmetics.procedural.join(","));
            run_in(view, m, code, Some(COSMETIC_WORLD));
        }
        return;
    }
    if m.body["refused"].as_str() == Some("script") {
        if !cosmetics.script.is_empty() {
            run_in(view, m, as_script(&cosmetics.script), None);
        }
        return;
    }
    let strings = |key: &str| -> Vec<String> {
        m.body[key]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
            .unwrap_or_default()
    };
    let selectors = blocker.generic(&cosmetics, &strings("classes"), &strings("ids"));
    if !selectors.is_empty() {
        adopt(view, m, &hide_rule(&selectors));
    }
}

/// `scriptlets` as if a `<script>` of the page ran them. Scriptlets tell their own reads from the
/// page's by `document.currentScript` (`set-constant` answers the page only when it differs), and
/// code run through the DevTools protocol has none, as the page's timers and handlers have none.
fn as_script(scriptlets: &str) -> String {
    format!(
        "(() => {{ const own = document.createElement('script'); \
         Object.defineProperty(document, 'currentScript', {{ configurable: true, get: () => own }}); \
         try {{ (function () {{\n{scriptlets}\n}})(); }} finally {{ delete document.currentScript; }} }})()"
    )
}

fn adopt(view: &WebView, m: &ScriptMessage, css: &str) {
    let css = JsValue::String(css.to_owned());
    run_in(view, m, format!("({ADOPT})({css})"), Some(COSMETIC_WORLD));
}

/// Runs `code` in the frame `m` came from, without a user gesture: the page did nothing to earn
/// one.
fn run_in(view: &WebView, m: &ScriptMessage, code: String, world: Option<&str>) {
    let frame = (!m.frame.is_empty()).then_some(m.frame.as_str());
    view.execute_javascript_in_frame(code, world, frame, UserGesture::Without, |_, _| {});
}

/// A tab listener: a closed tab's count goes.
pub fn tab_event(_: &mut BrowserWindow, id: TabId, event: TabEvent, cx: &mut Context<BrowserWindow>) {
    if event == TabEvent::Closed && cx.has_global::<State>() {
        cx.global_mut::<State>().blocked.remove(&id);
    }
}

/// The blocker's facts for `tab`, for the command bar and the padlock; `None` off the web.
pub fn facts(tab: &Tab, cx: &App) -> Option<BlockingFacts> {
    let state = cx.try_global::<State>()?;
    let site = site_of(&tab.url);
    if site.is_empty() || !tab.is_web_page() {
        return None;
    }
    let s = state.store.get();
    Some(BlockingFacts {
        on: s.blocking_on(&site),
        hidden: s.hidden_on(&site).len(),
        blocked: state.blocked.get(&tab.id).map_or(0, HashSet::len),
        site,
    })
}

/// Whether blocking is off for tab `id`'s site, for the command's label.
pub fn off_here(win: &BrowserWindow, id: TabId, cx: &App) -> bool {
    win.tab(id).and_then(|t| facts(t, cx)).is_some_and(|f| !f.on)
}

/// The Allow Ads command: switches blocking for the tab's site and reloads its page.
pub fn toggle(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.tab(id) else { return };
    let Some(facts) = facts(tab, cx) else { return };
    let store = cx.global::<State>().store.clone();
    store.update(|s| s.set_blocking_on(&facts.site, !facts.on));
    set_off(&store.get());
    if let Some(view) = tab.view() {
        view.reload();
    }
    cx.notify();
}

/// The padlock popover's row: the switch for the site, with what was blocked on the page.
pub fn row(win: &BrowserWindow, tab: &Tab, _: &mut Window, cx: &mut Context<BrowserWindow>) -> Option<AnyElement> {
    let facts = facts(tab, cx)?;
    let detail = if facts.on { blocked_sentence(facts.blocked) } else { "Off for this site".into() };
    let (win_id, tab_id) = (win.id, tab.id);
    Some(
        h_flex()
            .id("site-blocking")
            .gap(cx.theme().spacing.sm)
            .items_center()
            .child(
                v_flex().flex_1().min_w_0().child(label("Block Ads and Trackers").id("site-blocking-title")).child(
                    label(detail).id("site-blocking-count").secondary().text_style(TextStyle::Caption1).truncate(),
                ),
            )
            .child(switch("site-blocking-switch").label("Block Ads and Trackers").checked(facts.on).on_toggle(
                move |_, window, cx| {
                    if let Some(w) = crate::app::window(cx, win_id) {
                        w.update(cx, |w, cx| toggle(w, tab_id, window, cx));
                    }
                },
            ))
            .into_any_element(),
    )
}
