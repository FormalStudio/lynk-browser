//! The element hider, Shift+Cmd+H: point at an element, click, and it is gone on that site from
//! then on. The picker sleeps in a world of its own in every page's main frame until the command
//! wakes it; a pick becomes the user's `site##selector` rule in the blocker, which puts it in the
//! head of every later document there. Esc or the command again puts the picker away. It owns
//! the picker, not the rules: those are the blocker's.

use nd::prelude::*;
use nd::webview::{UserGesture, UserScript, WebView};

use crate::page::PageEvent;
use crate::tab::TabId;
use crate::window::BrowserWindow;

const MESSAGE: &str = "lynkHider";
const WORLD: &str = "lynk-hider";

/// Defines `window.lynkHider.toggle()`. While on, it outlines the element under the pointer with
/// its name, swallows every press, and on pointerdown hides that element and posts
/// `{pick: {selector, label, note}}`. Its chrome lives in a closed shadow root with constructed
/// sheets, out of reach of the page's styles and content security policy.
const PICKER: &str = r#"(() => {
  if (window.lynkHider) return;
  const post = (m) => window.webkit?.messageHandlers?.lynkHider?.postMessage(m);
  const SHEET = `
    :host { all: initial; position: fixed; inset: 0; z-index: 2147483647; pointer-events: none;
      --blue: #007aff; --pill: rgb(28 28 30 / 0.94); --ring: rgb(255 255 255 / 0); color-scheme: light; }
    @media (prefers-color-scheme: dark) {
      :host { --blue: #0a84ff; --pill: rgb(44 44 46 / 0.96); --ring: rgb(255 255 255 / 0.12); } }
    .frame { position: fixed; display: none; box-sizing: border-box; border-radius: 4px;
      border: 2px solid var(--blue); background: color-mix(in srgb, var(--blue) 12%, transparent);
      box-shadow: 0 0 0 1px rgb(255 255 255 / 0.7); }
    .tag { position: absolute; left: -2px; max-width: 320px; overflow: hidden; text-overflow: ellipsis;
      white-space: nowrap; padding: 2px 7px; border-radius: 5px; background: var(--blue); color: #fff;
      font: 500 11px/16px system-ui, -apple-system, sans-serif; }
    .hint { position: fixed; top: 12px; left: 50%; translate: -50% 0; padding: 7px 12px;
      border-radius: 9px; background: var(--pill); color: #fff; white-space: nowrap;
      font: 500 12px/16px system-ui, -apple-system, sans-serif; letter-spacing: -0.01em;
      box-shadow: 0 0 0 1px var(--ring), 0 4px 16px rgb(0 0 0 / 0.18), 0 1px 3px rgb(0 0 0 / 0.12);
      animation: in 160ms cubic-bezier(0.23, 1, 0.32, 1); }
    .hint kbd { font: inherit; padding: 0 5px; margin: 0 1px; border-radius: 4px; background: rgb(255 255 255 / 0.16); }
    @keyframes in { from { opacity: 0; translate: -50% -4px; } }
    @media (prefers-reduced-motion: reduce) { .hint { animation-name: fade; } @keyframes fade { from { opacity: 0; } } }`;
  let host = null, frame, tag, target = null, live = false;
  const cursor = new CSSStyleSheet();
  cursor.replaceSync('*, *::before, *::after { cursor: crosshair !important; }');

  const chrome = () => {
    if (host) return;
    host = document.createElement('lynk-hider');
    const root = host.attachShadow({ mode: 'closed' });
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(SHEET);
    root.adoptedStyleSheets = [sheet];
    frame = document.createElement('div');
    frame.className = 'frame';
    tag = document.createElement('div');
    tag.className = 'tag';
    frame.append(tag);
    const hint = document.createElement('div');
    hint.className = 'hint';
    hint.append('Click an element to hide it on this site  ');
    const esc = document.createElement('kbd');
    esc.textContent = 'esc';
    hint.append(esc, ' to finish');
    root.append(frame, hint);
  };

  const known = { nav: 'Navigation', header: 'Header', footer: 'Footer', aside: 'Sidebar', form: 'Form',
    dialog: 'Dialog', video: 'Video', img: 'Image', button: 'Button', iframe: 'Embed', figure: 'Figure', table: 'Table' };
  const clip = (text, n) => text.length > n ? text.slice(0, n) + '…' : text;
  const name = (el) => {
    const said = el.getAttribute('aria-label') || el.getAttribute('title');
    if (said && said.trim()) return clip(said.trim(), 40);
    const t = el.tagName.toLowerCase();
    if (known[t]) return known[t];
    const role = el.getAttribute('role');
    if (role) return role[0].toUpperCase() + role.slice(1);
    const text = (el.innerText || '').trim().replace(/\s+/g, ' ');
    return text ? clip(text, 40) : t;
  };
  // Size and place: two sidebars read alike, they rarely share a shape and a corner.
  const shape = (el) => {
    const r = el.getBoundingClientRect(), cx = r.left + r.width / 2, cy = r.top + r.height / 2;
    const side = cx < innerWidth / 3 ? 'left' : cx > innerWidth * 2 / 3 ? 'right' : 'centre';
    const band = cy < innerHeight / 3 ? 'top' : cy > innerHeight * 2 / 3 ? 'bottom' : 'middle';
    return `${Math.round(r.width)}×${Math.round(r.height)} · ${band} ${side}`;
  };
  // A class worth hanging a rule on: a word, not a build artefact.
  const steady = (c) => /^[a-zA-Z][\w-]{2,29}$/.test(c) && !/\d{3,}/.test(c) && !/^(css|sc|jsx|emotion|svelte|styles?)-/.test(c);
  const unique = (sel) => { try { return document.querySelectorAll(sel).length === 1; } catch (_) { return false; } };
  const selectorFor = (el) => {
    if (el.id && unique('#' + CSS.escape(el.id))) return '#' + CSS.escape(el.id);
    for (const hook of ['data-testid', 'data-test', 'data-qa', 'data-cy', 'aria-label', 'name', 'role']) {
      const v = el.getAttribute(hook);
      const s = v && `${el.tagName.toLowerCase()}[${hook}="${CSS.escape(v)}"]`;
      if (s && unique(s)) return s;
    }
    const classes = [...el.classList].filter(steady);
    const byClass = classes.length && el.tagName.toLowerCase() + '.' + classes.map(CSS.escape).join('.');
    if (byClass && unique(byClass)) return byClass;
    const parts = [];
    for (let node = el; node && node.nodeType === 1 && node !== document.documentElement; node = node.parentElement) {
      if (node.id && unique('#' + CSS.escape(node.id))) { parts.unshift('#' + CSS.escape(node.id)); break; }
      const t = node.tagName.toLowerCase(), parent = node.parentElement;
      if (!parent) { parts.unshift(t); break; }
      const kin = [...parent.children].filter((c) => c.tagName === node.tagName);
      parts.unshift(kin.length > 1 ? `${t}:nth-of-type(${kin.indexOf(node) + 1})` : t);
    }
    return parts.join(' > ');
  };

  const under = (e) => {
    const el = document.elementFromPoint(e.clientX, e.clientY);
    return el && el !== host && el !== document.documentElement && el !== document.body ? el : null;
  };
  const place = (el) => {
    const r = el.getBoundingClientRect();
    Object.assign(frame.style, { display: 'block', left: r.left + 'px', top: r.top + 'px', width: r.width + 'px', height: r.height + 'px' });
    tag.textContent = name(el);
    // Above the element when there is room, inside its top edge when there is not.
    tag.style.top = r.top >= 22 ? '-21px' : '2px';
    tag.style.left = (r.left < 0 ? -r.left + 2 : -2) + 'px';
  };
  const onMove = (e) => { const el = live && under(e); if (el) { target = el; place(el); } };
  // The element under a pointer that has not moved since: the picker woke, or a pick reflowed
  // the page under it.
  const hovered = () => {
    const el = [...document.querySelectorAll(':hover')].pop();
    if (live && el && el !== host && el !== document.documentElement && el !== document.body) { target = el; place(el); }
  };
  // Pages act on pointerdown or mousedown and are gone before a click lands, so every kind of
  // press is swallowed while picking.
  const swallow = (e) => { if (!live) return; e.preventDefault(); e.stopPropagation(); e.stopImmediatePropagation(); };
  const onPress = (e) => {
    if (!live) return;
    swallow(e);
    const el = under(e) || target;
    if (!el) return;
    post({ pick: { selector: selectorFor(el), label: name(el), note: shape(el) } });
    el.style.setProperty('display', 'none', 'important');
    target = null;
    frame.style.display = 'none';
    requestAnimationFrame(() => requestAnimationFrame(hovered));
  };
  const onKey = (e) => { if (live && e.key === 'Escape') { swallow(e); off(); } };
  const presses = ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click', 'dblclick', 'auxclick', 'contextmenu', 'touchstart'];
  const listen = (add) => {
    const f = add ? addEventListener : removeEventListener;
    f('pointermove', onMove, true);
    f('mousemove', onMove, true);
    f('keydown', onKey, true);
    for (const kind of presses) f(kind, kind === 'pointerdown' ? onPress : swallow, true);
  };
  const on = () => {
    live = true;
    chrome();
    document.documentElement.append(host);
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, cursor];
    listen(true);
    hovered();
  };
  const off = () => {
    live = false;
    target = null;
    host.remove();
    frame.style.display = 'none';
    document.adoptedStyleSheets = document.adoptedStyleSheets.filter((s) => s !== cursor);
    listen(false);
  };
  window.lynkHider = { toggle: () => (live ? off() : on()), get live() { return live; } };
})()"#;

/// Puts the sleeping picker in a new page's main frame.
pub fn prepare(view: &WebView) {
    view.register_script_message(MESSAGE, Some(WORLD), ["*"]);
    view.add_user_script(UserScript::new("lynk-hider", PICKER).in_world(WORLD).at_start());
}

/// The Hide Element command: wakes the picker on tab `id`'s page, or puts it away.
pub fn toggle(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.tab(id) else { return };
    let (Some(view), Some(_)) = (tab.view(), crate::blocking::facts(tab, cx)) else { return };
    view.focus();
    view.execute_javascript_in_frame("window.lynkHider?.toggle()", Some(WORLD), None, UserGesture::Without, |_, _| {});
}

/// Whether tab `id`'s site has elements the user hid, for Show Hidden Elements.
pub fn has_hidden(win: &BrowserWindow, id: TabId, cx: &App) -> bool {
    win.tab(id).and_then(|t| crate::blocking::facts(t, cx)).is_some_and(|f| f.hidden > 0)
}

/// Show Hidden Elements: every element hidden on the tab's site comes back once its page reloads.
pub fn restore(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.tab(id) else { return };
    let Some(facts) = crate::blocking::facts(tab, cx) else { return };
    crate::blocking::restore_hidden(&facts.site, cx);
    if let Some(view) = tab.view() {
        view.reload();
    }
    cx.notify();
}

/// A page observer: a pick becomes a rule for the page's site.
pub fn observe(win: &mut BrowserWindow, id: TabId, event: &PageEvent, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let PageEvent::ScriptMessage(m) = event else { return };
    if m.name != MESSAGE {
        return;
    }
    let pick = &m.body["pick"];
    let (Some(selector), Some(tab)) = (pick["selector"].as_str().filter(|s| !s.is_empty()), win.tab(id)) else {
        return;
    };
    let Some(facts) = crate::blocking::facts(tab, cx) else { return };
    let text = |key: &str| pick[key].as_str().unwrap_or_default().to_owned();
    crate::blocking::hide(&facts.site, selector, &text("label"), &text("note"), cx);
    cx.notify();
}
