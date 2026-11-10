//! Windows and their tabs as `session.json` keeps them, and where Chrome puts a tab a page
//! opened.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use nd_settings::Store;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::{lenient_items, null_default};

pub const SESSION_VERSION: u32 = 2;
pub const WINDOW_WIDTH: f64 = 1280.0;
pub const WINDOW_HEIGHT: f64 = 800.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionTab {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub url: String,
    #[serde(deserialize_with = "null_default")]
    pub title: String,
    /// Pinned tabs sort ahead of the rest, and the list order mirrors that grouping, so cycling
    /// and the Tabs menu read in the order the sidebar draws.
    #[serde(deserialize_with = "bool_or_false")]
    pub pinned: bool,
    /// Where the tab was when it was pinned, which Reset to Pinned Page goes back to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned_url: Option<String>,
}

/// Tab ids are unique across windows, which lets a tab move between them and keep everything
/// the app keys by tab.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionWindow {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "lenient_items")]
    pub tabs: Vec<SessionTab>,
    #[serde(deserialize_with = "null_default")]
    pub active_id: String,
    #[serde(deserialize_with = "null_default")]
    pub width: f64,
    #[serde(deserialize_with = "null_default")]
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionState {
    #[serde(deserialize_with = "lenient_items")]
    pub windows: Vec<SessionWindow>,
    #[serde(deserialize_with = "counter")]
    pub next_tab_id: u64,
    #[serde(deserialize_with = "counter")]
    pub next_window_id: u64,
    /// Page zoom keyed by host, the way every desktop browser scopes it.
    #[serde(deserialize_with = "null_default")]
    pub zoom_by_host: BTreeMap<String, f64>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self { windows: Vec::new(), next_tab_id: 1, next_window_id: 1, zoom_by_host: BTreeMap::new() }
    }
}

fn counter<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(Option::<u64>::deserialize(d)?.unwrap_or(1))
}

fn bool_or_false<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(matches!(Value::deserialize(d)?, Value::Bool(true)))
}

/// Version 1 kept one window's worth of state at the top level.
fn migrate(from: u32, data: Value) -> Result<Value, String> {
    if from >= 2 {
        return Ok(data);
    }
    let field = |key: &str| data.get(key).filter(|v| !v.is_null()).cloned();
    Ok(json!({
        "windows": [{
            "id": "w1",
            "tabs": field("tabs").unwrap_or_else(|| json!([])),
            "activeId": field("activeId").unwrap_or_else(|| json!("")),
            "width": field("windowWidth").unwrap_or_else(|| json!(WINDOW_WIDTH)),
            "height": field("windowHeight").unwrap_or_else(|| json!(WINDOW_HEIGHT)),
        }],
        "nextTabId": field("nextTabId").unwrap_or_else(|| json!(1)),
        "nextWindowId": 2,
        "zoomByHost": field("zoomByHost").unwrap_or_else(|| json!({})),
    }))
}

/// `session.json` in `dir`, version 1 and 2 files read. Call [`normalize`] on what it holds
/// before restoring from it.
pub fn open(dir: impl Into<PathBuf>) -> Store<SessionState> {
    Store::open(dir.into().join("session.json"), SESSION_VERSION, Some(migrate))
}

pub fn blank_tab(id: impl Into<String>) -> SessionTab {
    SessionTab { id: id.into(), ..SessionTab::default() }
}

fn or_default(v: f64, fallback: f64) -> f64 {
    if v > 0.0 && v.is_finite() { v } else { fallback }
}

/// A new tab is stored with an empty address, so a stored `about:blank` is never one the user
/// opened: it is a page-opened tab whose destination never arrived, and restoring it restores
/// a dead row.
fn normalize_tabs(tabs: &[SessionTab]) -> Vec<SessionTab> {
    let kept = tabs.iter().filter(|t| t.url != "about:blank");
    let (pinned, rest): (Vec<_>, Vec<_>) = kept.cloned().partition(|t| t.pinned);
    pinned.into_iter().chain(rest).collect()
}

/// A restored session has at least one window, and every window at least one tab. A window
/// whose every tab was dropped goes with them, unless it is the only one.
pub fn normalize(state: &SessionState) -> SessionState {
    let mut next_tab_id = state.next_tab_id;
    let mut next_window_id = state.next_window_id;
    let mut windows = Vec::new();
    for w in &state.windows {
        let tabs = normalize_tabs(&w.tabs);
        let Some(first) = tabs.first() else { continue };
        let active_id = if tabs.iter().any(|t| t.id == w.active_id) { w.active_id.clone() } else { first.id.clone() };
        windows.push(SessionWindow {
            id: w.id.clone(),
            tabs,
            active_id,
            width: or_default(w.width, WINDOW_WIDTH),
            height: or_default(w.height, WINDOW_HEIGHT),
        });
    }
    if windows.is_empty() {
        let id = format!("t{next_tab_id}");
        next_tab_id += 1;
        let first = state.windows.first();
        let window_id = match first {
            Some(w) => w.id.clone(),
            None => {
                next_window_id += 1;
                format!("w{}", next_window_id - 1)
            }
        };
        windows.push(SessionWindow {
            id: window_id,
            tabs: vec![blank_tab(id.clone())],
            active_id: id,
            width: or_default(first.map_or(0.0, |w| w.width), WINDOW_WIDTH),
            height: or_default(first.map_or(0.0, |w| w.height), WINDOW_HEIGHT),
        });
    }
    SessionState { windows, next_tab_id, next_window_id, zoom_by_host: state.zoom_by_host.clone() }
}

/// The window a tab lives in, or `None` for a tab that is gone.
pub fn window_of_tab<'a>(state: &'a SessionState, tab_id: &str) -> Option<&'a SessionWindow> {
    state.windows.iter().find(|w| w.tabs.iter().any(|t| t.id == tab_id))
}

/// How a page asked for a tab, as the engine reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenDisposition {
    ForegroundTab,
    BackgroundTab,
    Window,
    Popup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub index: usize,
    pub foreground: bool,
}

/// Where Chrome puts a tab a page opened, and whether it is shown. A tab in front goes right
/// after its opener. One behind it goes after the opener's other tabs still next to it, so a run
/// of ctrl-clicks reads left to right in click order. Neither lands in the pinned block.
/// `openers` maps a tab to the tab that opened it.
pub fn place_opened_tab(
    tabs: &[SessionTab],
    opener_id: &str,
    openers: &HashMap<String, String>,
    disposition: Option<OpenDisposition>,
) -> Placement {
    let foreground = disposition != Some(OpenDisposition::BackgroundTab);
    let Some(from) = tabs.iter().position(|t| t.id == opener_id) else {
        return Placement { index: tabs.len(), foreground };
    };
    let mut index = from + 1;
    if !foreground {
        while index < tabs.len() && openers.get(&tabs[index].id).is_some_and(|o| o == opener_id) {
            index += 1;
        }
    }
    while index < tabs.len() && tabs[index].pinned {
        index += 1;
    }
    Placement { index, foreground }
}

/// Moves a tab to `index` in `to_window`'s list, where it becomes the tab on show. Within one
/// window this is a reorder. The source window keeps showing what it showed, or its neighbour
/// when the moved tab was on show; a window left with no tabs stays empty for the caller to
/// close. A pinned tab stays in the pinned block and an unpinned one outside it.
pub fn move_tab_in(state: &mut SessionState, tab_id: &str, to_window: &str, index: usize) {
    if !state.windows.iter().any(|w| w.id == to_window) {
        return;
    }
    let Some(from) = state.windows.iter().position(|w| w.tabs.iter().any(|t| t.id == tab_id)) else { return };
    let source = &mut state.windows[from];
    let at = source.tabs.iter().position(|t| t.id == tab_id).expect("tab is in its window");
    let tab = source.tabs.remove(at);
    if source.active_id == tab_id && source.id != to_window {
        source.active_id =
            source.tabs.get(at.min(source.tabs.len().saturating_sub(1))).map(|t| t.id.clone()).unwrap_or_default();
    }
    let target = state.windows.iter_mut().find(|w| w.id == to_window).expect("target window exists");
    let boundary = target.tabs.iter().filter(|t| t.pinned).count();
    let (lo, hi) = if tab.pinned { (0, boundary) } else { (boundary, target.tabs.len()) };
    target.tabs.insert(index.clamp(lo, hi), tab);
    target.active_id = tab_id.to_owned();
}

/// "Start with a fresh window": the pinned tabs of every window in one window, in front of a new
/// tab that opens the homepage when one is set. Numbering carries over, so a fresh id never
/// collides with one still in the store.
pub fn fresh_start(stored: &SessionState, homepage: &str) -> SessionState {
    let first = stored.windows.first();
    let mut tabs: Vec<SessionTab> = stored
        .windows
        .iter()
        .flat_map(|w| w.tabs.iter().filter(|t| t.pinned && t.url != "about:blank"))
        .cloned()
        .collect();
    let id = format!("t{}", stored.next_tab_id);
    tabs.push(SessionTab { url: homepage.to_owned(), ..blank_tab(id.clone()) });
    SessionState {
        windows: vec![SessionWindow {
            id: first.map_or_else(|| format!("w{}", stored.next_window_id), |w| w.id.clone()),
            tabs,
            active_id: id,
            width: or_default(first.map_or(0.0, |w| w.width), WINDOW_WIDTH),
            height: or_default(first.map_or(0.0, |w| w.height), WINDOW_HEIGHT),
        }],
        next_tab_id: stored.next_tab_id + 1,
        next_window_id: if first.is_some() { stored.next_window_id } else { stored.next_window_id + 1 },
        zoom_by_host: stored.zoom_by_host.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str, url: &str, pinned: bool) -> SessionTab {
        SessionTab { id: id.into(), url: url.into(), title: id.into(), pinned, pinned_url: None }
    }

    fn window(id: &str, tabs: Vec<SessionTab>, active: &str, width: f64, height: f64) -> SessionWindow {
        SessionWindow { id: id.into(), tabs, active_id: active.into(), width, height }
    }

    fn ids(w: &SessionWindow) -> Vec<&str> {
        w.tabs.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn a_fresh_start_keeps_every_windows_pins_in_one_window_a_new_tab_in_front() {
        let stored = SessionState {
            windows: vec![
                window(
                    "w1",
                    vec![tab("t1", "https://a.test/", true), tab("t2", "https://b.test/", false)],
                    "t2",
                    900.0,
                    700.0,
                ),
                window(
                    "w2",
                    vec![tab("t3", "https://c.test/", true), tab("t4", "https://d.test/", false)],
                    "t4",
                    1280.0,
                    800.0,
                ),
            ],
            next_tab_id: 5,
            next_window_id: 3,
            ..SessionState::default()
        };
        let fresh = normalize(&fresh_start(&stored, ""));
        assert_eq!(fresh.windows.len(), 1);
        assert_eq!(ids(&fresh.windows[0]), ["t1", "t3", "t5"]);
        assert_eq!(fresh.windows[0].active_id, "t5");
        assert_eq!(fresh.windows[0].tabs[2].url, "");
        assert_eq!(fresh.windows[0].width, 900.0);
        assert_eq!(fresh.next_tab_id, 6);
    }

    #[test]
    fn a_fresh_start_opens_the_homepage_when_one_is_set() {
        let fresh = normalize(&fresh_start(&SessionState::default(), "https://home.test/"));
        let urls: Vec<&str> = fresh.windows[0].tabs.iter().map(|t| t.url.as_str()).collect();
        assert_eq!(urls, ["https://home.test/"]);
    }

    #[test]
    fn a_pages_tab_lands_where_chrome_puts_it() {
        let tabs = vec![
            tab("p", "https://pin.test/", true),
            tab("a", "https://a.test/", false),
            tab("b", "https://b.test/", false),
            tab("c", "https://c.test/", false),
        ];
        let none = HashMap::new();
        let at = |index, foreground| Placement { index, foreground };
        assert_eq!(place_opened_tab(&tabs, "a", &none, Some(OpenDisposition::ForegroundTab)), at(2, true));
        assert_eq!(place_opened_tab(&tabs, "a", &none, Some(OpenDisposition::Popup)), at(2, true));
        assert_eq!(place_opened_tab(&tabs, "a", &none, None), at(2, true));
        assert_eq!(place_opened_tab(&tabs, "a", &none, Some(OpenDisposition::BackgroundTab)), at(2, false));
        // Two earlier ctrl-clicks from a sit right after it: the third goes after them.
        let run = vec![
            tab("a", "https://a.test/", false),
            tab("x", "https://x.test/", false),
            tab("y", "https://y.test/", false),
            tab("c", "https://c.test/", false),
        ];
        let openers: HashMap<String, String> = [("x", "a"), ("y", "a")].map(|(k, v)| (k.into(), v.into())).into();
        assert_eq!(place_opened_tab(&run, "a", &openers, Some(OpenDisposition::BackgroundTab)), at(3, false));
        assert_eq!(place_opened_tab(&run, "a", &openers, Some(OpenDisposition::ForegroundTab)), at(1, true));
        assert_eq!(place_opened_tab(&tabs, "p", &none, Some(OpenDisposition::ForegroundTab)), at(1, true));
        assert_eq!(place_opened_tab(&tabs, "gone", &none, Some(OpenDisposition::BackgroundTab)), at(4, false));
    }

    #[test]
    fn normalize_drops_dead_rows_and_keeps_one_window() {
        let state = SessionState {
            windows: vec![
                window("w1", vec![tab("t1", "about:blank", false)], "t1", 0.0, 0.0),
                window(
                    "w2",
                    vec![tab("t2", "https://b.test/", false), tab("t3", "https://c.test/", true)],
                    "gone",
                    0.0,
                    600.0,
                ),
            ],
            next_tab_id: 4,
            next_window_id: 3,
            ..SessionState::default()
        };
        let out = normalize(&state);
        assert_eq!(out.windows.len(), 1);
        assert_eq!(ids(&out.windows[0]), ["t3", "t2"]);
        assert_eq!(out.windows[0].active_id, "t3");
        assert_eq!((out.windows[0].width, out.windows[0].height), (WINDOW_WIDTH, 600.0));

        let empty = normalize(&SessionState::default());
        assert_eq!(empty.windows[0].id, "w1");
        assert_eq!(ids(&empty.windows[0]), ["t1"]);
        assert_eq!((empty.next_tab_id, empty.next_window_id), (2, 2));
    }

    #[test]
    fn a_moved_tab_keeps_its_block_and_the_source_shows_a_neighbour() {
        let mut state = SessionState {
            windows: vec![
                window(
                    "w1",
                    vec![
                        tab("p", "https://p.test/", true),
                        tab("a", "https://a.test/", false),
                        tab("b", "https://b.test/", false),
                    ],
                    "a",
                    1.0,
                    1.0,
                ),
                window(
                    "w2",
                    vec![tab("q", "https://q.test/", true), tab("c", "https://c.test/", false)],
                    "c",
                    1.0,
                    1.0,
                ),
            ],
            ..SessionState::default()
        };
        move_tab_in(&mut state, "a", "w2", 0);
        assert_eq!(ids(&state.windows[0]), ["p", "b"]);
        assert_eq!(state.windows[0].active_id, "b");
        assert_eq!(ids(&state.windows[1]), ["q", "a", "c"]);
        assert_eq!(state.windows[1].active_id, "a");
        move_tab_in(&mut state, "p", "w2", 9);
        assert_eq!(ids(&state.windows[1]), ["q", "p", "a", "c"]);
        assert!(state.windows[0].tabs.iter().all(|t| t.id == "b"));
    }

    #[test]
    fn nulls_in_session_json_take_their_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("session.json"),
            r#"{"version":2,"data":{"windows":[{"id":"w1","tabs":[{"id":"t1","url":"https://a.test/","title":null,"pinned":null}],"activeId":null,"width":null,"height":null},{"id":"w2","tabs":null}],"nextTabId":null,"nextWindowId":3,"zoomByHost":null}}"#,
        )
        .unwrap();
        let store = open(dir.path());
        assert!(store.load_error().is_none(), "{:?}", store.load_error());
        let s = normalize(&store.get());
        assert_eq!(ids(&s.windows[0]), ["t1"]);
        assert_eq!(s.windows[0].tabs[0].title, "");
        assert_eq!((s.windows[0].width, s.windows[0].height), (WINDOW_WIDTH, WINDOW_HEIGHT));
        assert_eq!(s.windows.len(), 1);
        assert_eq!((s.next_tab_id, s.next_window_id), (1, 3));
        drop(store);

        let gone = tempfile::tempdir().unwrap();
        std::fs::write(gone.path().join("session.json"), r#"{"version":2,"data":{"windows":null}}"#).unwrap();
        let store = open(gone.path());
        assert!(store.load_error().is_none());
        assert_eq!(normalize(&store.get()).windows.len(), 1);
    }

    #[test]
    fn session_json_versions_one_and_two_are_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("session.json"),
            r#"{"version":1,"data":{"tabs":[{"id":"t1","url":"https://a.test/","title":"A"},{"id":"t2","url":"https://b.test/","title":"B","pinned":"yes"}],"activeId":"t2","nextTabId":3,"windowWidth":1000,"zoomByHost":{"a.test":1.25}}}"#,
        )
        .unwrap();
        let store = open(dir.path());
        assert!(store.load_error().is_none(), "{:?}", store.load_error());
        let s = normalize(&store.get());
        assert_eq!(s.windows.len(), 1);
        assert_eq!(s.windows[0].id, "w1");
        assert_eq!(ids(&s.windows[0]), ["t1", "t2"]);
        assert_eq!(s.windows[0].active_id, "t2");
        assert_eq!((s.windows[0].width, s.windows[0].height), (1000.0, WINDOW_HEIGHT));
        assert_eq!((s.next_tab_id, s.next_window_id), (3, 2));
        assert_eq!(s.zoom_by_host["a.test"], 1.25);
        drop(store);

        let v2 = tempfile::tempdir().unwrap();
        std::fs::write(
            v2.path().join("session.json"),
            r#"{"version":2,"data":{"windows":[{"id":"w1","tabs":[{"id":"t1","url":"","title":"","pinned":false}],"activeId":"t1","width":1280,"height":544}],"nextTabId":2,"nextWindowId":2,"zoomByHost":{}}}"#,
        )
        .unwrap();
        let store = open(v2.path());
        assert!(store.load_error().is_none());
        assert_eq!(store.get().windows[0].height, 544.0);
        let text = serde_json::to_value(&*store.get()).unwrap();
        assert!(text["windows"][0]["tabs"][0].get("pinnedUrl").is_none());
    }
}
