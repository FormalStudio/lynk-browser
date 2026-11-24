//! Windows and their tabs as `session.json` keeps them, and where Chrome puts a tab a page
//! opened.
//!
//! A window lists its tabs' pages in `tabs` and arranges them in one sidebar tree per space
//! (`spaces`). The two always name the same tabs: [`normalize`] restores that on read, and the
//! operations here keep it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use nd_settings::Store;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use crate::archive::ArchivedTab;
use crate::mru::Mru;
use crate::sidebar_tree::{Item, SidebarTree};
use crate::spaces::{DEFAULT_SPACE_NAME, SiteRule, Space, WindowSpaces, neighbour};
use crate::{lenient_items, null_default};

pub const SESSION_VERSION: u32 = 3;
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
    /// Mirrors the tab's place in its sidebar tree, which decides. The list order is the
    /// sidebar's drawing order, so cycling and the Tabs menu read the way the sidebar draws.
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
    #[serde(deserialize_with = "null_default")]
    pub spaces: WindowSpaces,
    #[serde(deserialize_with = "null_default")]
    pub mru: Mru,
}

impl SessionWindow {
    /// Puts `tabs` in drawing order and their `pinned` flags in step with the trees.
    pub fn sync_tabs(&mut self) {
        let order: HashMap<&str, usize> = self.spaces.tab_ids().into_iter().enumerate().map(|(i, t)| (t, i)).collect();
        self.tabs.sort_by_key(|t| order.get(t.id.as_str()).copied().unwrap_or(usize::MAX));
        for t in &mut self.tabs {
            t.pinned = self.spaces.sidebars.values().any(|tree| tree.is_pinned(&t.id));
        }
    }

    /// Adds a tab to the space on show, through [`SidebarTree::add_tab`]'s placement.
    pub fn add_tab(&mut self, tab: SessionTab, front: Option<&str>, root_index: usize) -> bool {
        if self.tabs.iter().any(|t| t.id == tab.id) || self.spaces.tree_mut().add_tab(&tab.id, front, root_index).is_err() {
            return false;
        }
        self.tabs.push(tab);
        self.sync_tabs();
        true
    }
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
    /// In the order their numbers run.
    #[serde(deserialize_with = "lenient_items")]
    pub spaces: Vec<Space>,
    #[serde(deserialize_with = "lenient_items")]
    pub space_rules: Vec<SiteRule>,
    /// Numbers the ids of spaces, separators, folders and splits.
    #[serde(deserialize_with = "counter")]
    pub next_item_id: u64,
    /// Newest first.
    #[serde(deserialize_with = "lenient_items")]
    pub archive: Vec<ArchivedTab>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            windows: Vec::new(),
            next_tab_id: 1,
            next_window_id: 1,
            zoom_by_host: BTreeMap::new(),
            spaces: Vec::new(),
            space_rules: Vec::new(),
            next_item_id: 1,
            archive: Vec::new(),
        }
    }
}

impl SessionState {
    /// A fresh id for a space (`s`), separator, folder or split (`i`).
    pub fn alloc_item_id(&mut self, prefix: char) -> String {
        let id = format!("{prefix}{}", self.next_item_id);
        self.next_item_id += 1;
        id
    }

    pub fn alloc_tab_id(&mut self) -> String {
        let id = format!("t{}", self.next_tab_id);
        self.next_tab_id += 1;
        id
    }

    /// Closes a tab in whichever window holds it. The window keeps showing what it showed, or
    /// the neighbour in its sidebar when the closed tab was on show; a window left with no tabs
    /// stays for the caller to close.
    pub fn close_tab(&mut self, tab_id: &str) -> bool {
        let Some(w) = self.windows.iter_mut().find(|w| w.tabs.iter().any(|t| t.id == tab_id)) else { return false };
        let order: Vec<String> = w.spaces.tree().map(|t| t.tab_ids().iter().map(|s| (*s).to_owned()).collect()).unwrap_or_default();
        w.tabs.retain(|t| t.id != tab_id);
        w.mru.closed(tab_id);
        for tree in w.spaces.sidebars.values_mut() {
            tree.close_tab(tab_id);
        }
        if w.active_id == tab_id {
            w.active_id = neighbour(&order, tab_id).unwrap_or_default();
        }
        w.sync_tabs();
        true
    }
}

fn counter<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(Option::<u64>::deserialize(d)?.unwrap_or(1))
}

fn bool_or_false<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(matches!(Value::deserialize(d)?, Value::Bool(true)))
}

/// Version 1 kept one window's worth of state at the top level. Version 2 had a flat list of
/// tabs per window: each becomes one space whose pins and loose tabs are that list.
fn migrate(from: u32, data: Value) -> Result<Value, String> {
    let data = if from < 2 { migrate_v1(&data) } else { data };
    if from >= 3 {
        return Ok(data);
    }
    let mut state: SessionState = serde_json::from_value(data).map_err(|e| e.to_string())?;
    let space = state.alloc_item_id('s');
    state.spaces = vec![Space { id: space.clone(), name: DEFAULT_SPACE_NAME.into(), ..Space::default() }];
    for w in &mut state.windows {
        let (pins, rest): (Vec<_>, Vec<_>) = w.tabs.iter().partition(|t| t.pinned);
        let tree = SidebarTree {
            pins: pins.iter().map(|t| t.id.clone()).collect(),
            items: rest.iter().map(|t| Item::tab(t.id.clone())).collect(),
            last_active: w.active_id.clone(),
        };
        w.spaces = WindowSpaces::single(&space, tree);
    }
    serde_json::to_value(&state).map_err(|e| e.to_string())
}

fn migrate_v1(data: &Value) -> Value {
    let field = |key: &str| data.get(key).filter(|v| !v.is_null()).cloned();
    json!({
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
    })
}

/// `session.json` in `dir`, version 1, 2 and 3 files read. Call [`normalize`] on what it holds
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
fn restorable(t: &SessionTab) -> bool {
    t.url != "about:blank"
}

/// The number after an id's prefix letter, for keeping counters ahead of ids already handed out.
fn id_number(id: &str) -> u64 {
    id.get(1..).and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// A restored session has at least one space and one window, every window at least one tab in
/// the space it shows, and every tab exactly once across the session's trees. A window whose
/// every tab was dropped goes with them, unless it is the only one.
pub fn normalize(state: &SessionState) -> SessionState {
    let mut out = SessionState {
        windows: Vec::new(),
        next_tab_id: state.next_tab_id,
        next_window_id: state.next_window_id,
        zoom_by_host: state.zoom_by_host.clone(),
        spaces: Vec::new(),
        space_rules: Vec::new(),
        next_item_id: state.next_item_id,
        archive: state.archive.clone(),
    };
    let mut space_ids = HashSet::new();
    for space in &state.spaces {
        if !space.id.is_empty() && space_ids.insert(space.id.clone()) {
            out.spaces.push(space.clone());
        }
    }
    let item_ids = state.spaces.iter().map(|s| s.id.as_str()).chain(state.windows.iter().flat_map(|w| {
        w.spaces.sidebars.values().flat_map(|t| t.items.iter().flat_map(row_ids))
    }));
    out.next_item_id = out.next_item_id.max(item_ids.map(id_number).max().unwrap_or(0) + 1);
    if out.spaces.is_empty() {
        let id = out.alloc_item_id('s');
        space_ids.insert(id.clone());
        out.spaces.push(Space { id, name: DEFAULT_SPACE_NAME.into(), ..Space::default() });
    }
    let first_space = out.spaces[0].id.clone();
    out.space_rules = state.space_rules.iter().filter(|r| space_ids.contains(&r.space_id)).cloned().collect();

    let mut claimed: HashSet<String> = HashSet::new();
    for w in &state.windows {
        let mut tabs: Vec<SessionTab> = Vec::new();
        for t in w.tabs.iter().filter(|t| restorable(t)) {
            if !t.id.is_empty() && !claimed.contains(&t.id) && !tabs.iter().any(|k| k.id == t.id) {
                tabs.push(t.clone());
            }
        }
        if tabs.is_empty() {
            continue;
        }
        let known: HashSet<String> = tabs.iter().map(|t| t.id.clone()).collect();
        let mut spaces = w.spaces.clone();
        spaces.sidebars.retain(|id, _| space_ids.contains(id));
        if !space_ids.contains(&spaces.shown) {
            spaces.shown = first_space.clone();
        }
        // The space on show claims its tabs first, so a tab listed twice stays where it is seen.
        let mut order: Vec<String> = vec![spaces.shown.clone()];
        order.extend(spaces.sidebars.keys().filter(|k| **k != spaces.shown).cloned());
        for id in &order {
            if let Some(tree) = spaces.sidebars.get_mut(id) {
                tree.repair(|t| known.contains(t) && claimed.insert(t.to_owned()));
            }
        }
        let shown = spaces.tree_mut();
        let untracked: Vec<&SessionTab> = tabs.iter().filter(|t| !claimed.contains(&t.id)).collect();
        for t in untracked {
            claimed.insert(t.id.clone());
            if t.pinned {
                shown.pins.push(t.id.clone());
            } else {
                shown.items.push(Item::tab(t.id.clone()));
            }
        }
        if shown.tab_ids().is_empty() {
            let id = out.alloc_tab_id();
            tabs.push(blank_tab(id.clone()));
            spaces.tree_mut().items.push(Item::tab(id));
        }
        let shown = spaces.tree().expect("space on show has a sidebar");
        let active_id = [&w.active_id, &shown.last_active]
            .into_iter()
            .find(|id| shown.contains_tab(id))
            .cloned()
            .unwrap_or_else(|| shown.tab_ids()[0].to_owned());
        let mut mru = w.mru.clone();
        mru.retain(|t| tabs.iter().any(|k| k.id == t));
        let mut window = SessionWindow {
            id: w.id.clone(),
            tabs,
            active_id,
            width: or_default(w.width, WINDOW_WIDTH),
            height: or_default(w.height, WINDOW_HEIGHT),
            spaces,
            mru,
        };
        window.sync_tabs();
        out.windows.push(window);
    }
    if out.windows.is_empty() {
        let id = out.alloc_tab_id();
        let first = state.windows.first();
        let window_id = match first {
            Some(w) => w.id.clone(),
            None => {
                out.next_window_id += 1;
                format!("w{}", out.next_window_id - 1)
            }
        };
        let tree = SidebarTree { items: vec![Item::tab(id.clone())], ..SidebarTree::default() };
        out.windows.push(SessionWindow {
            id: window_id,
            tabs: vec![blank_tab(id.clone())],
            active_id: id,
            width: or_default(first.map_or(0.0, |w| w.width), WINDOW_WIDTH),
            height: or_default(first.map_or(0.0, |w| w.height), WINDOW_HEIGHT),
            spaces: WindowSpaces::single(&first_space, tree),
            mru: Mru::default(),
        });
    }
    out
}

fn row_ids(item: &Item) -> Vec<&str> {
    match item {
        Item::Tab { .. } => Vec::new(),
        Item::Folder(f) => std::iter::once(f.id.as_str())
            .chain(f.children.iter().filter_map(|c| match c {
                crate::sidebar_tree::Child::Split(s) => Some(s.id.as_str()),
                crate::sidebar_tree::Child::Tab { .. } => None,
            }))
            .collect(),
        other => vec![other.id()],
    }
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

/// Moves a tab to `index` in `to_window`, into the space that window shows, where it becomes
/// the tab on show. `index` counts the pins and then the sidebar's rows. Within one window this
/// is a reorder. The source window keeps showing what it showed, or its neighbour when the moved
/// tab was on show; a window left with no tabs stays empty for the caller to close. A pinned tab
/// stays pinned and an unpinned one goes into the list.
pub fn move_tab_in(state: &mut SessionState, tab_id: &str, to_window: &str, index: usize) {
    if !state.windows.iter().any(|w| w.id == to_window) {
        return;
    }
    let Some(from) = state.windows.iter().position(|w| w.tabs.iter().any(|t| t.id == tab_id)) else { return };
    let source = &mut state.windows[from];
    let order: Vec<String> = source.tabs.iter().map(|t| t.id.clone()).collect();
    let at = order.iter().position(|t| t == tab_id).expect("tab is in its window");
    let tab = source.tabs.remove(at);
    for tree in source.spaces.sidebars.values_mut() {
        tree.close_tab(tab_id);
    }
    if source.active_id == tab_id && source.id != to_window {
        source.active_id = neighbour(&order, tab_id).unwrap_or_default();
    }
    source.sync_tabs();
    let target = state.windows.iter_mut().find(|w| w.id == to_window).expect("target window exists");
    let tree = target.spaces.tree_mut();
    if tab.pinned {
        tree.pins.insert(index.min(tree.pins.len()), tab.id.clone());
    } else {
        let at = index.saturating_sub(tree.pins.len()).min(tree.items.len());
        tree.items.insert(at, Item::tab(tab.id.clone()));
    }
    target.tabs.push(tab);
    target.active_id = tab_id.to_owned();
    target.sync_tabs();
}

/// "Start with a fresh window": every window's pins and folders, space by space, in one window,
/// with a new tab that opens the homepage when one is set in the space the first window showed.
/// Numbering carries over, so a fresh id never collides with one still in the store.
pub fn fresh_start(stored: &SessionState, homepage: &str) -> SessionState {
    let mut s = normalize(stored);
    let windows = std::mem::take(&mut s.windows);
    let first = &windows[0];
    let mut spaces = WindowSpaces { shown: first.spaces.shown.clone(), sidebars: BTreeMap::new() };
    let mut tabs: Vec<SessionTab> = Vec::new();
    for w in &windows {
        for (space, tree) in &w.spaces.sidebars {
            let kept = spaces.sidebars.entry(space.clone()).or_default();
            kept.pins.extend(tree.pins.iter().cloned());
            kept.items.extend(tree.items.iter().filter(|i| matches!(i, Item::Folder(_))).cloned());
        }
    }
    for w in &windows {
        tabs.extend(w.tabs.iter().filter(|t| spaces.space_of_tab(&t.id).is_some()).cloned());
    }
    let id = s.alloc_tab_id();
    tabs.push(SessionTab { url: homepage.to_owned(), ..blank_tab(id.clone()) });
    spaces.tree_mut().items.push(Item::tab(id.clone()));
    let mut window = SessionWindow {
        id: first.id.clone(),
        tabs,
        active_id: id,
        width: first.width,
        height: first.height,
        spaces,
        mru: Mru::default(),
    };
    window.sync_tabs();
    s.windows = vec![window];
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidebar_tree::Item;

    fn tab(id: &str, url: &str, pinned: bool) -> SessionTab {
        SessionTab { id: id.into(), url: url.into(), title: id.into(), pinned, pinned_url: None }
    }

    fn window(id: &str, tabs: Vec<SessionTab>, active: &str, width: f64, height: f64) -> SessionWindow {
        SessionWindow { id: id.into(), tabs, active_id: active.into(), width, height, ..SessionWindow::default() }
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
        let mut state = normalize(&SessionState {
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
        });
        move_tab_in(&mut state, "a", "w2", 0);
        assert_eq!(ids(&state.windows[0]), ["p", "b"]);
        assert_eq!(state.windows[0].active_id, "b");
        assert_eq!(ids(&state.windows[1]), ["q", "a", "c"]);
        assert_eq!(state.windows[1].active_id, "a");
        move_tab_in(&mut state, "p", "w2", 9);
        assert_eq!(ids(&state.windows[1]), ["q", "p", "a", "c"]);
        assert!(state.windows[0].tabs.iter().all(|t| t.id == "b"));
        let tree = state.windows[1].spaces.tree().unwrap();
        assert_eq!(tree.pins, ["q", "p"]);
        assert_eq!(tree.loose_tabs(), ["a", "c"]);
        assert_eq!(state.windows[0].spaces.tree().unwrap().tab_ids(), ["b"]);
    }

    fn fixture() -> SessionState {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("session.json"), include_str!("../fixtures/session-v2.json")).unwrap();
        let store = open(dir.path());
        assert!(store.load_error().is_none(), "{:?}", store.load_error());
        store.get().as_ref().clone()
    }

    #[test]
    fn a_version_two_file_opens_as_one_space_of_pins_and_loose_tabs() {
        let raw = fixture();
        assert_eq!(raw.spaces.len(), 1);
        assert_eq!((raw.spaces[0].id.as_str(), raw.spaces[0].name.as_str()), ("s1", DEFAULT_SPACE_NAME));
        let s = normalize(&raw);
        assert_eq!(s, raw, "a migrated file is already normal");
        let w = &s.windows[0];
        assert_eq!(ids(w), ["t1", "t2", "t3", "t4", "t5"]);
        assert_eq!(w.active_id, "t3");
        assert_eq!((w.width, w.height), (1440.0, 900.0));
        assert_eq!(w.tabs[1].pinned_url.as_deref(), Some("https://calendar.example.com/"));
        let tree = w.spaces.tree().unwrap();
        assert_eq!(w.spaces.shown, "s1");
        assert_eq!(tree.pins, ["t1", "t2"]);
        assert_eq!(tree.loose_tabs(), ["t3", "t4", "t5"]);
        assert_eq!(s.windows[1].id, "w3");
        assert_eq!(s.windows[1].spaces.tree().unwrap().loose_tabs(), ["t7"]);
        assert_eq!((s.next_tab_id, s.next_window_id, s.next_item_id), (8, 4, 2));
        assert_eq!(s.zoom_by_host["github.com"], 1.1);
    }

    #[test]
    fn version_three_round_trips_through_the_store() {
        let mut s = normalize(&fixture());
        let sep = s.alloc_item_id('i');
        let folder = s.alloc_item_id('i');
        let w = &mut s.windows[0];
        let tree = w.spaces.tree_mut();
        tree.add_separator_below("t3", &sep, "Reading").unwrap();
        tree.new_folder(&folder, crate::sidebar_tree::FolderKind::Project, "Lynk", "code", Some(280), 9).unwrap();
        tree.move_row("t5", crate::sidebar_tree::Dest::Folder { id: folder.clone(), index: 0 }).unwrap();
        tree.make_split("i9", "t3", "t4").unwrap();
        s.spaces.push(Space { id: "s3".into(), name: "Work".into(), ..Space::default() });
        crate::spaces::set_rule(&mut s.space_rules, "github.com", "s3");
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.update(|state| *state = s.clone());
        store.flush().unwrap();
        drop(store);
        let back = open(dir.path());
        assert!(back.load_error().is_none());
        assert_eq!(*back.get(), s);
        assert_eq!(normalize(&back.get()).next_item_id, 10);
        let text = std::fs::read_to_string(dir.path().join("session.json")).unwrap();
        assert!(text.contains("\"version\": 3") || text.contains("\"version\":3"), "{text}");
    }

    #[test]
    fn normalize_keeps_each_tab_once_and_fills_an_empty_space() {
        let mut w1 = window("w1", vec![tab("a", "https://a.test/", false), tab("b", "https://b.test/", false)], "a", 1.0, 1.0);
        let tree = SidebarTree { items: vec![Item::tab("a"), Item::tab("ghost"), Item::tab("a")], ..SidebarTree::default() };
        w1.spaces = WindowSpaces::single("s1", tree);
        w1.spaces.sidebars.insert("s2".into(), SidebarTree { pins: vec!["b".into()], ..SidebarTree::default() });
        w1.spaces.sidebars.insert("gone".into(), SidebarTree { pins: vec!["a".into()], ..SidebarTree::default() });
        let w2 = window("w2", vec![tab("a", "https://dup.test/", false), tab("c", "https://c.test/", true)], "a", 1.0, 1.0);
        let state = SessionState {
            windows: vec![w1, w2],
            spaces: vec![
                Space { id: "s1".into(), ..Space::default() },
                Space { id: "s2".into(), ..Space::default() },
                Space { id: "s1".into(), ..Space::default() },
            ],
            space_rules: vec![SiteRule { host: "x.test".into(), space_id: "gone".into() }],
            next_tab_id: 10,
            ..SessionState::default()
        };
        let s = normalize(&state);
        assert_eq!(s.spaces.len(), 2);
        assert!(s.space_rules.is_empty());
        let w = &s.windows[0];
        assert_eq!(w.spaces.sidebars.keys().collect::<Vec<_>>(), ["s1", "s2"]);
        assert_eq!(w.spaces.tree().unwrap().tab_ids(), ["a"]);
        assert_eq!(w.spaces.sidebars["s2"].pins, ["b"]);
        assert_eq!(ids(w), ["a", "b"]);
        assert!(w.tabs[1].pinned);
        // w2's copy of "a" is gone, and its own pin survives.
        assert_eq!(ids(&s.windows[1]), ["c"]);
        assert_eq!(s.windows[1].spaces.shown, "s1");

        let mut only_other = window("w1", vec![tab("b", "https://b.test/", false)], "b", 1.0, 1.0);
        only_other.spaces = WindowSpaces::single("s2", SidebarTree { items: vec![Item::tab("b")], ..SidebarTree::default() });
        only_other.spaces.shown = "s1".into();
        let s = normalize(&SessionState { windows: vec![only_other], next_tab_id: 5, ..state });
        let w = &s.windows[0];
        assert_eq!(w.spaces.tree().unwrap().tab_ids(), ["t5"]);
        assert_eq!(w.active_id, "t5");
        assert_eq!(ids(w), ["t5", "b"]);
        assert_eq!(s.next_tab_id, 6);
    }

    #[test]
    fn closing_and_adding_tabs_keep_the_list_and_tree_together() {
        let mut s = normalize(&fixture());
        assert!(s.close_tab("t3"));
        assert_eq!(s.windows[0].active_id, "t4");
        assert!(!s.close_tab("t3"));
        let id = s.alloc_tab_id();
        assert!(s.windows[0].add_tab(blank_tab(id.clone()), None, 0));
        assert!(!s.windows[0].add_tab(blank_tab(id), None, 0));
        assert_eq!(ids(&s.windows[0]), ["t1", "t2", "t8", "t4", "t5"]);
        assert_eq!(normalize(&s), s);
    }

    #[test]
    fn recency_survives_closing_and_reopening_the_app() {
        let mut s = normalize(&fixture());
        for id in ["t5", "t3", "t1"] {
            s.windows[0].mru.shown(id);
        }
        s.windows[0].mru.shown("gone");
        s.close_tab("t3");
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.update(|state| *state = s.clone());
        drop(store);
        let back = normalize(&open(dir.path()).get());
        assert_eq!(back.windows[0].mru.0, ["t1", "t5"]);
    }

    #[test]
    fn a_fresh_start_keeps_folders() {
        let mut s = normalize(&fixture());
        let tree = s.windows[0].spaces.tree_mut();
        tree.new_folder("i5", crate::sidebar_tree::FolderKind::Plain, "Daily", "", None, 0).unwrap();
        tree.move_row("t5", crate::sidebar_tree::Dest::Folder { id: "i5".into(), index: 0 }).unwrap();
        s.windows[0].sync_tabs();
        let fresh = fresh_start(&s, "");
        assert_eq!(fresh.windows.len(), 1);
        assert_eq!(ids(&fresh.windows[0]), ["t1", "t2", "t5", "t8"]);
        assert_eq!(fresh.windows[0].active_id, "t8");
        assert_eq!(normalize(&fresh), fresh);
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
