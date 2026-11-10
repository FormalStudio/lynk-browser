//! What the command bar lists, in order, for one query. The window hands in its tabs, the
//! history hits and the engine and gets rows back. `docs/omnibox.md` is the spec.
//!
//! Typed text always gets its own row, ahead of any open tab, so Return on what was typed
//! searches or loads it and a tab is only switched to when its row is picked. Cmd+T and Cmd+L
//! rank the typed address, open tabs, history and commands; Cmd+K is the switcher, open tabs then
//! commands. Nothing typed leaves the machine until Return.

use url::Url;

use crate::keys::{chord, shortcut_label};
use crate::url::{display_url, host_of, is_search, to_url};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OmniRow {
    /// `go:<url>` (the completed address), `url` (the text as typed), `tab:<id>`, `hist:<url>`
    /// or `cmd:<id>`.
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub icon_name: Option<&'static str>,
    pub icon_data: Option<String>,
    pub hint: Option<String>,
    pub completion: Option<String>,
}

/// Cmd+L and Cmd+T ask where to go; Cmd+K asks which open page, or what to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmniMode {
    Address,
    Switcher,
}

/// Where Enter sends an address: Cmd+T opens it in a new tab, Cmd+L and Cmd+K load it in the
/// tab that is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OmniTarget {
    Current,
    NewTab,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmniTab {
    pub id: String,
    pub title: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OmniVisit {
    pub url: String,
    pub title: String,
}

pub struct OmniCommand {
    pub id: &'static str,
    pub title: &'static str,
    pub icon_name: &'static str,
    /// The shortcut it shares with the menu (`keys`), shown as the row's hint.
    pub keys: Option<&'static str>,
    /// Also found by these words, which are not in the title.
    pub aka: &'static str,
    pub pinned_only: bool,
}

const fn cmd(
    id: &'static str,
    title: &'static str,
    icon_name: &'static str,
    keys: Option<&'static str>,
    aka: &'static str,
) -> OmniCommand {
    OmniCommand { id, title, icon_name, keys, aka, pinned_only: false }
}

/// Every action the window has, so the bar can stand in for each button.
pub const COMMANDS: &[OmniCommand] = &[
    cmd("new-tab", "New Tab", "tab-new-symbolic", Some("new-tab"), ""),
    cmd("new-window", "New Window", "window-new-symbolic", Some("new-window"), ""),
    cmd("private", "New Private Window", "view-conceal-symbolic", Some("private"), ""),
    cmd("close-tab", "Close Tab", "window-close-symbolic", Some("close-tab"), ""),
    cmd("close-other-tabs", "Close Other Tabs", "window-close-symbolic", None, ""),
    cmd("reopen-tab", "Reopen Closed Tab", "edit-undo-symbolic", Some("reopen-tab"), ""),
    cmd("next-tab", "Next Tab", "go-next-symbolic", Some("next-tab"), ""),
    cmd("prev-tab", "Previous Tab", "go-previous-symbolic", Some("prev-tab"), ""),
    cmd("pin-tab", "Pin Tab", "view-pin-symbolic", None, ""),
    cmd("sleep-tab", "Put Tab to Sleep", "weather-clear-night-symbolic", None, "unload discard memory"),
    OmniCommand { pinned_only: true, ..cmd("reset-pinned", "Reset to Pinned Page", "go-home-symbolic", None, "") },
    cmd("duplicate-tab", "Duplicate Tab", "edit-copy-symbolic", None, ""),
    cmd("move-new-window", "Move Tab to New Window", "window-new-symbolic", None, ""),
    cmd("back", "Back", "go-previous-symbolic", Some("back"), ""),
    cmd("forward", "Forward", "go-next-symbolic", Some("forward"), ""),
    cmd("reload", "Reload", "view-refresh-symbolic", Some("reload"), ""),
    cmd("save-page", "Save Page As…", "document-save-symbolic", Some("save-page"), "download html"),
    cmd("view-source", "View Page Source", "text-x-generic-symbolic", Some("view-source"), "html code"),
    cmd("devtools", "Developer Tools", "utilities-terminal-symbolic", Some("devtools"), "inspect console devtools"),
    cmd("copy-address", "Copy Address", "edit-copy-symbolic", Some("copy-address"), "url link"),
    cmd("find", "Find in Page", "edit-find-symbolic", Some("find"), ""),
    cmd("zoom-in", "Zoom In", "zoom-in-symbolic", Some("zoom-in"), ""),
    cmd("zoom-out", "Zoom Out", "zoom-out-symbolic", Some("zoom-out"), ""),
    cmd("zoom-reset", "Actual Size", "zoom-original-symbolic", Some("zoom-reset"), "reset zoom"),
    cmd("reader", "Reading Mode", "text-x-generic-symbolic", Some("reader"), "reader article"),
    cmd("float", "Float Video", "video-x-generic-symbolic", Some("float"), "picture in picture pip"),
    cmd(
        "blocking",
        "Allow Ads on This Site",
        "security-low-symbolic",
        None,
        "adblock ad blocker ublock trackers block ads",
    ),
    cmd(
        "hide-element",
        "Hide Element",
        "view-conceal-symbolic",
        Some("hide-element"),
        "adblock remove annoyance picker",
    ),
    cmd("restore-hidden", "Show Hidden Elements", "view-reveal-symbolic", None, "adblock unhide restore"),
    cmd("update-lists", "Update Filter Lists", "view-refresh-symbolic", None, "adblock ublock easylist"),
    cmd("site-info", "Site Settings", "channel-secure-symbolic", None, "permissions security"),
    cmd("downloads-all", "Downloads", "folder-download-symbolic", Some("downloads"), ""),
    cmd("history", "History", "document-open-recent-symbolic", Some("history"), ""),
    cmd("bookmarks", "Bookmarks", "user-bookmarks-symbolic", Some("bookmarks"), ""),
    cmd("bookmark-page", "Bookmark This Page", "bookmark-new-symbolic", Some("bookmark-page"), ""),
    cmd(
        "clear-data",
        "Clear Browsing Data…",
        "edit-delete-symbolic",
        Some("clear-data"),
        "cookies cache history delete",
    ),
    cmd("extensions", "Extensions", "application-x-addon-symbolic", None, ""),
    cmd("extensions-page", "Manage Extensions", "application-x-addon-symbolic", None, ""),
    cmd("webstore", "Chrome Web Store", "web-browser-symbolic", None, ""),
    cmd("layout", "Switch Layout", "sidebar-show-symbolic", Some("layout"), "sidebar compact"),
    cmd("settings", "Settings", "preferences-system-symbolic", Some("settings"), "preferences"),
];

/// Commands that need Chromium: extensions, and the page scripts only it accepts.
const CHROMIUM_ONLY: &[&str] = &[
    "extensions",
    "extensions-page",
    "webstore",
    "reader",
    "float",
    "blocking",
    "hide-element",
    "restore-hidden",
    "update-lists",
    "save-page",
    "devtools",
    "clear-data",
];

/// A command's title for a tab that is or is not pinned: the pin command names what it does.
pub fn command_title(id: &str, pinned: bool) -> String {
    if id == "pin-tab" && pinned {
        return "Unpin Tab".into();
    }
    COMMANDS.iter().find(|c| c.id == id).map_or_else(|| id.to_owned(), |c| c.title.to_owned())
}

/// A host without its `www.`, which nobody types.
pub fn bare_host(url: &str) -> String {
    let host = host_of(url);
    host.strip_prefix("www.").map_or(host.clone(), str::to_owned)
}

/// What you would have typed to get somewhere: no scheme, no `www.`.
pub fn place_key(url: &str) -> String {
    let shown = display_url(url);
    shown.strip_prefix("www.").map_or(shown.clone(), str::to_owned)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// What the field completes to.
    pub text: String,
    /// What Enter loads for it.
    pub url: String,
}

/// Inline completion: the first place whose bare host, or failing that whose host and path,
/// starts with the text.
pub fn completion_for(query: &str, candidates: &[String]) -> Option<Completion> {
    let q = query.trim().to_lowercase();
    if q.is_empty() || q.chars().any(char::is_whitespace) || q.contains("://") {
        return None;
    }
    for url in candidates {
        let Ok(u) = Url::parse(url) else { continue };
        if u.scheme() != "http" && u.scheme() != "https" {
            continue;
        }
        let host = bare_host(url);
        if host.starts_with(&q) {
            return Some(Completion { text: host, url: format!("{}/", u.origin().ascii_serialization()) });
        }
        let full = place_key(url);
        if full.to_lowercase().starts_with(&q) {
            return Some(Completion { text: full, url: url.clone() });
        }
    }
    None
}

/// The built-in blocker on the showing tab's site, which names its commands.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockingFacts {
    pub site: String,
    pub on: bool,
    pub hidden: usize,
    pub blocked: usize,
}

pub struct OmniInput<'a> {
    pub mode: OmniMode,
    pub query: &'a str,
    pub target: OmniTarget,
    /// Open tabs other than the one showing, most recently shown first.
    pub tabs: &'a [OmniTab],
    /// History matches, best first. Completion picks from these and nothing else: only places
    /// you have been.
    pub history: &'a [OmniVisit],
    pub engine_name: &'a str,
    pub search_prefix: &'a str,
    /// Chromium-only commands are dropped when the engine cannot host them.
    pub chromium: bool,
    pub pinned: bool,
    /// The sleep command is left out for a tab that cannot sleep.
    pub can_sleep: bool,
    pub reading: bool,
    /// The site commands are left out for a page with no site.
    pub blocking: Option<BlockingFacts>,
    pub favicon: &'a dyn Fn(&str) -> Option<String>,
    /// Hints and chords for macOS rather than Linux.
    pub mac: bool,
}

const TABS_SHOWN: usize = 4;
const SWITCHER_TABS: usize = 8;

pub fn omni_rows(input: &OmniInput<'_>) -> Vec<OmniRow> {
    match input.mode {
        OmniMode::Switcher => switcher_rows(input),
        OmniMode::Address => address_rows(input),
    }
}

/// Cmd+T and Cmd+L: what Enter does with the typed text (the completed address first when there
/// is one), open tabs, history, then commands.
fn address_rows(input: &OmniInput<'_>) -> Vec<OmniRow> {
    let query = input.query.trim();
    let lowered = query.to_lowercase();
    let mut rows = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let open_hint = if input.target == OmniTarget::NewTab { "Open in New Tab" } else { "Open" };

    if !query.is_empty() {
        let urls: Vec<String> = input.history.iter().map(|v| v.url.clone()).collect();
        if let Some(done) = completion_for(query, &urls) {
            rows.push(OmniRow {
                id: format!("go:{}", done.url),
                title: done.text.clone(),
                icon_data: (input.favicon)(&done.url),
                icon_name: Some("web-browser-symbolic"),
                hint: Some(open_hint.into()),
                completion: Some(done.text),
                ..OmniRow::default()
            });
            seen.push(done.url);
        }
        if let Some((url, row)) = typed_row(query, input, open_hint)
            && !seen.contains(&url)
        {
            rows.push(row);
        }
    }

    // An open tab is offered as a switch, never also as a history row that would open it a
    // second time.
    seen.extend(input.tabs.iter().map(|t| t.url.clone()));
    rows.extend(tab_rows(input.tabs, &lowered, TABS_SHOWN, input.favicon));

    for v in input.history {
        if seen.contains(&v.url) {
            continue;
        }
        seen.push(v.url.clone());
        let shown = display_url(&v.url);
        rows.push(OmniRow {
            id: format!("hist:{}", v.url),
            title: if v.title.is_empty() { shown.clone() } else { v.title.clone() },
            // A page with no title of its own shows its address once, not twice.
            subtitle: (!v.title.is_empty() && v.title != shown).then_some(shown),
            icon_data: (input.favicon)(&v.url),
            icon_name: Some("document-open-recent-symbolic"),
            hint: Some(open_hint.into()),
            completion: None,
        });
    }

    rows.extend(command_rows(input, &lowered));
    rows
}

/// Cmd+K: with nothing typed the first row is the page you were just on, so Cmd+K then Return
/// goes back to it; with text typed the first row searches or loads it.
fn switcher_rows(input: &OmniInput<'_>) -> Vec<OmniRow> {
    let query = input.query.trim();
    let lowered = query.to_lowercase();
    let mut rows: Vec<OmniRow> = typed_row(query, input, "Open").map(|(_, row)| row).into_iter().collect();
    rows.extend(tab_rows(input.tabs, &lowered, SWITCHER_TABS, input.favicon));
    rows.extend(command_rows(input, &lowered));
    rows
}

/// What Enter does with the text as typed: search it, or open it when it is an address.
fn typed_row(query: &str, input: &OmniInput<'_>, open_hint: &str) -> Option<(String, OmniRow)> {
    let url = to_url(query, input.search_prefix)?;
    let row = if is_search(&url, input.search_prefix) {
        OmniRow {
            id: "url".into(),
            title: query.to_owned(),
            subtitle: Some(format!("{} Search", input.engine_name)),
            icon_name: Some("system-search-symbolic"),
            hint: Some("Search".into()),
            ..OmniRow::default()
        }
    } else {
        let shown = display_url(&url);
        OmniRow {
            id: "url".into(),
            title: if shown.is_empty() { query.to_owned() } else { shown },
            icon_name: Some("web-browser-symbolic"),
            hint: Some(open_hint.into()),
            ..OmniRow::default()
        }
    };
    Some((url, row))
}

fn tab_rows(tabs: &[OmniTab], lowered: &str, cap: usize, favicon: &dyn Fn(&str) -> Option<String>) -> Vec<OmniRow> {
    let mut rows = Vec::new();
    // A page open twice, as a pinned tile and a tab, is one place to go back to: the most
    // recently shown of the two takes the row.
    let mut listed: Vec<&str> = Vec::new();
    for t in tabs {
        if rows.len() >= cap {
            break;
        }
        if !t.url.is_empty() && listed.contains(&t.url.as_str()) {
            continue;
        }
        let shown_url = display_url(&t.url);
        let label = if !t.title.is_empty() {
            t.title.clone()
        } else if !shown_url.is_empty() {
            shown_url.clone()
        } else {
            "New Tab".into()
        };
        if !lowered.is_empty() && !format!("{label} {}", t.url).to_lowercase().contains(lowered) {
            continue;
        }
        if !t.url.is_empty() {
            listed.push(&t.url);
        }
        rows.push(OmniRow {
            id: format!("tab:{}", t.id),
            subtitle: (!shown_url.is_empty() && shown_url != label).then_some(shown_url),
            title: label,
            icon_data: if t.url.is_empty() { None } else { favicon(&t.url) },
            icon_name: Some("web-browser-symbolic"),
            hint: Some("Switch to Tab".into()),
            completion: None,
        });
    }
    rows
}

/// Titles that name what the command will do on the showing tab.
fn command_title_for(id: &str, input: &OmniInput<'_>) -> String {
    match (id, &input.blocking) {
        ("reader", _) if input.reading => "Leave Reading Mode".into(),
        ("blocking", Some(b)) => {
            if b.on {
                format!("Allow Ads on {}", b.site)
            } else {
                format!("Block Ads on {}", b.site)
            }
        }
        ("restore-hidden", Some(b)) => format!("Show Hidden Elements on {}", b.site),
        _ => command_title(id, input.pinned),
    }
}

fn command_subtitle(id: &str, input: &OmniInput<'_>) -> Option<String> {
    let b = input.blocking.as_ref()?;
    match id {
        "blocking" if b.on => Some(blocked_sentence(b.blocked)),
        "blocking" => Some("Ads and trackers load on this site".into()),
        "restore-hidden" if b.hidden == 1 => Some("1 element hidden".into()),
        "restore-hidden" => Some(format!("{} elements hidden", b.hidden)),
        _ => None,
    }
}

/// What the blocker stopped on the showing page.
pub fn blocked_sentence(count: usize) -> String {
    match count {
        0 => "Nothing blocked on this page".into(),
        1 => "1 request blocked on this page".into(),
        n => format!("{n} requests blocked on this page"),
    }
}

fn command_rows(input: &OmniInput<'_>, lowered: &str) -> Vec<OmniRow> {
    let site = input.blocking.as_ref().map_or("", |b| b.site.as_str());
    let hidden = input.blocking.as_ref().map_or(0, |b| b.hidden);
    let mut rows = Vec::new();
    for c in COMMANDS {
        if !input.chromium && CHROMIUM_ONLY.contains(&c.id) {
            continue;
        }
        if (c.id == "sleep-tab" && !input.can_sleep) || (c.pinned_only && !input.pinned) {
            continue;
        }
        if (c.id == "blocking" || c.id == "hide-element") && site.is_empty() {
            continue;
        }
        if c.id == "restore-hidden" && hidden == 0 {
            continue;
        }
        let title = command_title_for(c.id, input);
        if !lowered.is_empty() && !format!("{title} {}", c.aka).to_lowercase().contains(lowered) {
            continue;
        }
        let blocking_off = c.id == "blocking" && !input.blocking.as_ref().is_some_and(|b| b.on);
        rows.push(OmniRow {
            id: format!("cmd:{}", c.id),
            subtitle: command_subtitle(c.id, input),
            title,
            icon_name: Some(if blocking_off { "security-high-symbolic" } else { c.icon_name }),
            icon_data: None,
            hint: c.keys.and_then(|k| chord(k, input.mac)).map(|k| shortcut_label(k, input.mac)),
            completion: None,
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_icon(_: &str) -> Option<String> {
        None
    }

    fn base<'a>() -> OmniInput<'a> {
        OmniInput {
            mode: OmniMode::Address,
            query: "",
            target: OmniTarget::Current,
            tabs: &[],
            history: &[],
            engine_name: "Google",
            search_prefix: "https://search.test/?q=",
            chromium: true,
            pinned: false,
            can_sleep: false,
            reading: false,
            blocking: None,
            favicon: &no_icon,
            mac: false,
        }
    }

    fn tab(id: &str, title: &str, url: &str) -> OmniTab {
        OmniTab { id: id.into(), title: title.into(), url: url.into() }
    }

    fn visit(url: &str, title: &str) -> OmniVisit {
        OmniVisit { url: url.into(), title: title.into() }
    }

    fn ids(rows: &[OmniRow]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    fn find<'r>(rows: &'r [OmniRow], id: &str) -> Option<&'r OmniRow> {
        rows.iter().find(|r| r.id == id)
    }

    #[test]
    fn completes_to_the_bare_host_first_then_host_and_path() {
        let urls =
            ["https://www.github.com/anthropics/claude", "https://news.ycombinator.com/item?id=1"].map(String::from);
        let done = |text: &str, url: &str| Some(Completion { text: text.into(), url: url.into() });
        assert_eq!(completion_for("git", &urls), done("github.com", "https://www.github.com/"));
        assert_eq!(
            completion_for("GitHub.com/an", &urls),
            done("github.com/anthropics/claude", "https://www.github.com/anthropics/claude")
        );
        assert_eq!(completion_for("news", &urls).map(|c| c.text).as_deref(), Some("news.ycombinator.com"));
        assert_eq!(completion_for("git hub", &urls), None);
        assert_eq!(completion_for("https://git", &urls), None);
        assert_eq!(completion_for("zzz", &urls), None);
    }

    #[test]
    fn the_completed_address_leads_then_the_typed_row_tabs_history_commands() {
        let tabs = [tab("t2", "GitHub issues", "https://github.com/issues")];
        let history =
            [visit("https://github.com/pulls", "Pull requests"), visit("https://github.com/issues", "GitHub issues")];
        let rows = omni_rows(&OmniInput { query: "gith", tabs: &tabs, history: &history, ..base() });
        assert_eq!(ids(&rows), ["go:https://github.com/", "url", "tab:t2", "hist:https://github.com/pulls"]);
        assert_eq!(rows[0].completion.as_deref(), Some("github.com"));
        assert_eq!(rows[1].title, "gith");
        assert_eq!(rows[1].subtitle.as_deref(), Some("Google Search"));
        assert_eq!(rows[1].hint.as_deref(), Some("Search"));
        assert_eq!(rows[2].hint.as_deref(), Some("Switch to Tab"));
        assert_eq!(rows[3].hint.as_deref(), Some("Open"));
    }

    #[test]
    fn a_page_open_as_a_pinned_tile_and_as_a_tab_is_listed_once_as_the_one_shown_last() {
        let url = "https://github.com/FormalSnake";
        let tabs = [
            tab("row", "FormalSnake (FormalSnake) · GitHub", url),
            tab("pin", "FormalSnake (FormalSnake) · GitHub", url),
            tab("a", "New Tab", ""),
            tab("b", "New Tab", ""),
        ];
        for mode in [OmniMode::Address, OmniMode::Switcher] {
            let rows = omni_rows(&OmniInput { mode, tabs: &tabs, ..base() });
            let tab_ids: Vec<&str> = ids(&rows).into_iter().filter(|id| id.starts_with("tab:")).collect();
            assert_eq!(tab_ids, ["tab:row", "tab:a", "tab:b"]);
        }
    }

    #[test]
    fn completion_comes_from_history_only_never_from_an_open_tab() {
        let tabs = [tab("t1", "Ex", "https://example.com/")];
        let rows = omni_rows(&OmniInput { query: "exa", tabs: &tabs, ..base() });
        assert!(rows.iter().all(|r| r.completion.is_none()));
    }

    #[test]
    fn a_new_tab_bar_says_where_enter_goes() {
        let rows = omni_rows(&OmniInput { target: OmniTarget::NewTab, query: "example.com", ..base() });
        assert_eq!((rows[0].id.as_str(), rows[0].title.as_str()), ("url", "example.com"));
        assert_eq!(rows[0].hint.as_deref(), Some("Open in New Tab"));
    }

    #[test]
    fn an_untitled_page_names_its_address_once() {
        let history = [visit("https://example.com/a", "")];
        let rows = omni_rows(&OmniInput { history: &history, ..base() });
        assert_eq!(rows[0].title, "example.com/a");
        assert_eq!(rows[0].subtitle, None);
    }

    #[test]
    fn the_switcher_lists_open_tabs_first_then_every_command() {
        let tabs: Vec<OmniTab> =
            (0..10).map(|i| tab(&format!("t{i}"), &format!("Tab {i}"), &format!("https://s{i}.com/"))).collect();
        let history = [visit("https://h.com/", "H")];
        let rows =
            omni_rows(&OmniInput { mode: OmniMode::Switcher, tabs: &tabs, history: &history, mac: true, ..base() });
        assert_eq!(rows.iter().filter(|r| r.id.starts_with("tab:")).count(), 8);
        assert_eq!(rows[0].id, "tab:t0");
        assert_eq!(rows[0].hint.as_deref(), Some("Switch to Tab"));
        assert!(!rows.iter().any(|r| r.id.starts_with("hist:")));
        assert_eq!(find(&rows, "cmd:reopen-tab").and_then(|r| r.hint.as_deref()), Some("⇧⌘T"));
        assert!(find(&rows, "cmd:copy-address").is_some());
    }

    #[test]
    fn a_filtered_switcher_searches_first_then_matching_tabs_and_commands() {
        let s = |query, pinned, chromium| {
            omni_rows(&OmniInput { mode: OmniMode::Switcher, query, pinned, chromium, ..base() })
        };
        assert_eq!(ids(&s("zoom", false, true)), ["url", "cmd:zoom-in", "cmd:zoom-out", "cmd:zoom-reset"]);
        let titles: Vec<String> = s("unpin", true, true).into_iter().map(|r| r.title).collect();
        assert_eq!(titles, ["unpin", "Unpin Tab"]);
        assert_eq!(ids(&s("exten", false, false)), ["url"]);
    }

    #[test]
    fn typed_text_matching_an_open_tabs_title_or_host_searches_first_the_tab_second() {
        let tabs = [tab("z", "Home ⚡ Zig Programming Language", "https://ziglang.org/")];
        for mode in [OmniMode::Address, OmniMode::Switcher] {
            for query in ["zig", "ziglang", "programming"] {
                let rows = omni_rows(&OmniInput { mode, query, tabs: &tabs, ..base() });
                assert_eq!((rows[0].id.as_str(), rows[0].title.as_str()), ("url", query));
                assert_eq!(rows[0].subtitle.as_deref(), Some("Google Search"));
                assert_eq!(rows[0].hint.as_deref(), Some("Search"));
                assert_eq!(rows[1].id, "tab:z");
                assert_eq!(rows[1].hint.as_deref(), Some("Switch to Tab"));
            }
            let rows = omni_rows(&OmniInput { mode, query: "ziglang.org", tabs: &tabs, ..base() });
            assert_eq!((rows[0].id.as_str(), rows[0].title.as_str()), ("url", "ziglang.org"));
            assert_eq!(rows[0].hint.as_deref(), Some("Open"));
            assert_eq!(rows[1].id, "tab:z");
        }
    }

    #[test]
    fn an_inline_completion_of_an_open_tabs_address_opens_it_and_the_switch_stays_its_own_row() {
        let tabs = [tab("z", "Home ⚡ Zig Programming Language", "https://ziglang.org/")];
        let history = [visit("https://ziglang.org/", "Zig")];
        let rows = omni_rows(&OmniInput { query: "zig", tabs: &tabs, history: &history, ..base() });
        assert_eq!(ids(&rows), ["go:https://ziglang.org/", "url", "tab:z"]);
        assert_eq!(rows[0].completion.as_deref(), Some("ziglang.org"));
        assert_eq!(rows[0].hint.as_deref(), Some("Open"));
    }

    #[test]
    fn hints_come_from_the_declared_shortcuts() {
        let hint = |mac, id: &str| {
            let rows = omni_rows(&OmniInput { mode: OmniMode::Switcher, mac, ..base() });
            find(&rows, &format!("cmd:{id}")).and_then(|r| r.hint.clone())
        };
        assert_eq!(hint(true, "next-tab").as_deref(), Some("⇧⌘]"));
        assert_eq!(hint(true, "prev-tab").as_deref(), Some("⇧⌘["));
        assert_eq!(hint(true, "private").as_deref(), Some("⇧⌘N"));
        assert_eq!(hint(true, "zoom-in").as_deref(), Some("⌘+"));
        assert_eq!(hint(false, "next-tab").as_deref(), Some("Ctrl+Tab"));
        assert_eq!(hint(false, "prev-tab").as_deref(), Some("Ctrl+Shift+Tab"));
    }

    #[test]
    fn put_tab_to_sleep_is_offered_only_for_a_tab_that_can_sleep() {
        let s = |query, can_sleep| omni_rows(&OmniInput { mode: OmniMode::Switcher, query, can_sleep, ..base() });
        assert_eq!(ids(&s("sleep", true)), ["url", "cmd:sleep-tab"]);
        assert_eq!(ids(&s("sleep", false)), ["url"]);
        assert_eq!(ids(&s("memory", true)), ["url", "cmd:sleep-tab"]);
    }

    #[test]
    fn reading_mode_and_the_floating_video_are_commands_with_their_chords() {
        let s = |query, mac, reading, chromium| {
            omni_rows(&OmniInput { mode: OmniMode::Switcher, query, mac, reading, chromium, ..base() })
        };
        let hint = |rows: &[OmniRow], id: &str| find(rows, id).and_then(|r| r.hint.clone());
        assert_eq!(hint(&s("read", true, false, true), "cmd:reader").as_deref(), Some("⇧⌘R"));
        assert_eq!(hint(&s("read", false, false, true), "cmd:reader").as_deref(), Some("Ctrl+Shift+R"));
        let reading = s("", false, true, true);
        assert_eq!(find(&reading, "cmd:reader").map(|r| r.title.as_str()), Some("Leave Reading Mode"));
        assert_eq!(hint(&reading, "cmd:float").as_deref(), Some("Ctrl+Alt+P"));
        assert_eq!(hint(&s("float", true, false, true), "cmd:float").as_deref(), Some("⌥⌘P"));
        assert!(ids(&s("pip", false, false, true)).contains(&"cmd:float"));
        let webkit = s("", false, false, false);
        assert!(find(&webkit, "cmd:reader").is_none() && find(&webkit, "cmd:float").is_none());
    }

    #[test]
    fn the_blockers_commands_name_the_site_and_what_they_will_do() {
        let facts = BlockingFacts { site: "example.com".into(), on: true, hidden: 0, blocked: 12 };
        let s = |query, blocking: BlockingFacts| {
            omni_rows(&OmniInput { mode: OmniMode::Switcher, query, blocking: Some(blocking), ..base() })
        };
        let rows = s("ads", facts.clone());
        let row = find(&rows, "cmd:blocking").unwrap();
        assert_eq!(row.title, "Allow Ads on example.com");
        assert_eq!(row.subtitle.as_deref(), Some("12 requests blocked on this page"));
        let off = s("ads", BlockingFacts { on: false, ..facts.clone() });
        assert_eq!(find(&off, "cmd:blocking").unwrap().title, "Block Ads on example.com");
        assert_eq!(find(&off, "cmd:blocking").unwrap().icon_name, Some("security-high-symbolic"));
        assert!(find(&rows, "cmd:restore-hidden").is_none());
        let shown = s("hidden", BlockingFacts { hidden: 2, ..facts.clone() });
        assert_eq!(find(&shown, "cmd:restore-hidden").and_then(|r| r.subtitle.as_deref()), Some("2 elements hidden"));
        let blank = s("ads", BlockingFacts { site: String::new(), ..facts });
        assert!(find(&blank, "cmd:blocking").is_none() && find(&blank, "cmd:hide-element").is_none());
    }
}
