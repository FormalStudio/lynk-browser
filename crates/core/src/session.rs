//! `session.json`: the shape it keeps windows, tabs, spaces and sidebars in, reading every
//! version it has had, and bringing what it holds back within the rules on load ([`normalize`],
//! the decode boundary). Also where Chrome puts a tab a page opened. Changing a loaded session
//! is `window_tabs`'s; nothing here moves a tab.
//!
//! A window keeps its tabs' pages in `tabs` (in no particular order) and arranges them in one
//! sidebar tree per space; the trees alone say where a tab is and whether it is pinned.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use nd_settings::Store;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value, json};

use crate::archive::ArchivedTab;
use crate::mru::Mru;
use crate::sidebar_tree::{Child, Dest, Item, SidebarTree};
use crate::spaces::{DEFAULT_SPACE_NAME, SiteRule, Space, WindowSpaces};
use crate::{lenient_items, null_default};

pub const SESSION_VERSION: u32 = 3;
pub const WINDOW_WIDTH: f64 = 1280.0;
pub const WINDOW_HEIGHT: f64 = 800.0;

/// A tab's id: `t` and a number the session hands out ([`tab_id`]). The shell keys its tabs
/// by that number.
pub type TabId = String;

pub fn tab_id(number: u64) -> TabId {
    format!("t{number}")
}

/// The number in a tab id, `None` for an id the session did not hand out.
pub fn tab_number(id: &str) -> Option<u64> {
    id.strip_prefix('t')?.parse().ok()
}

/// Fields a newer build added are kept as they were and written back, so opening a session in
/// an older build loses nothing.
type Unknown = Map<String, Value>;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SessionTab {
    #[serde(deserialize_with = "null_default")]
    pub id: TabId,
    #[serde(deserialize_with = "null_default")]
    pub url: String,
    #[serde(deserialize_with = "null_default")]
    pub title: String,
    /// Where the tab was when it was pinned, which Reset to Pinned Page goes back to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned_url: Option<String>,
    /// Where the page was scrolled, `[x, y]` in CSS pixels, put back when the tab first loads.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "null_default")]
    pub scroll: Option<[f64; 2]>,
    #[serde(flatten)]
    pub unknown: Unknown,
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
    pub active_id: TabId,
    #[serde(deserialize_with = "null_default")]
    pub width: f64,
    #[serde(deserialize_with = "null_default")]
    pub height: f64,
    /// The window's top-left corner in global points from the top-left of the primary
    /// display; absent for a window the OS places.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "null_default")]
    pub x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "null_default")]
    pub y: Option<f64>,
    #[serde(deserialize_with = "null_default")]
    pub spaces: WindowSpaces,
    #[serde(deserialize_with = "null_default")]
    pub mru: Mru,
    #[serde(flatten)]
    pub unknown: Unknown,
}

impl SessionWindow {
    pub fn tab(&self, id: &str) -> Option<&SessionTab> {
        self.tabs.iter().find(|t| t.id == id)
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
    #[serde(flatten)]
    pub unknown: Unknown,
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
            unknown: Unknown::new(),
        }
    }
}

impl SessionState {
    /// A fresh id for a space (`s`), separator, folder or split (`i`).
    pub fn alloc_item_id(&mut self, prefix: char) -> String {
        let id = format!("{prefix}{}", self.next_item_id);
        self.next_item_id = self.next_item_id.saturating_add(1);
        id
    }

    // shortcut: a counter at u64::MAX hands out its last id again; upgrade if a session ever
    // gets near 2^64 tabs or windows, which no profile can.
    pub fn alloc_tab_id(&mut self) -> TabId {
        let id = tab_id(self.next_tab_id);
        self.next_tab_id = self.next_tab_id.saturating_add(1);
        id
    }

    pub fn alloc_window_id(&mut self) -> String {
        let id = format!("w{}", self.next_window_id);
        self.next_window_id = self.next_window_id.saturating_add(1);
        id
    }
}

fn counter<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(Option::<u64>::deserialize(d)?.unwrap_or(1))
}

/// Version 1 kept one window's worth of state at the top level. Version 2 had a flat list of
/// tabs per window, pins flagged first: each window's list becomes one space's pins and loose
/// tabs, in order.
fn migrate(from: u32, data: Value) -> Result<Value, String> {
    let mut data = if from < 2 { migrate_v1(&data) } else { data };
    if from >= 3 {
        return Ok(data);
    }
    // Tab ids are unique across windows, so one set serves every window.
    let mut pins: HashSet<String> = HashSet::new();
    for w in data.get_mut("windows").and_then(Value::as_array_mut).into_iter().flatten() {
        for t in w.get_mut("tabs").and_then(Value::as_array_mut).into_iter().flatten() {
            let Some(t) = t.as_object_mut() else { continue };
            if t.remove("pinned") == Some(Value::Bool(true))
                && let Some(id) = t.get("id").and_then(Value::as_str)
            {
                pins.insert(id.to_owned());
            }
        }
    }
    let mut state: SessionState = serde_json::from_value(data).map_err(|e| e.to_string())?;
    let space = state.alloc_item_id('s');
    state.spaces = vec![Space { id: space.clone(), name: DEFAULT_SPACE_NAME.into(), ..Space::default() }];
    for w in &mut state.windows {
        let mut tree = SidebarTree { last_active: w.active_id.clone(), ..SidebarTree::default() };
        for t in w.tabs.iter().filter(|t| pins.contains(&t.id)) {
            let _ = tree.insert_tab(&t.id, Dest::Pins(usize::MAX));
        }
        for t in w.tabs.iter().filter(|t| !pins.contains(&t.id)) {
            let _ = tree.insert_tab(&t.id, Dest::Root(usize::MAX));
        }
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

/// `session.json` in `dir`: version 1, 2 and 3 files read, and what they hold already through
/// [`normalize`]. A file that does not read (cut short, empty, not an object) or comes from a
/// newer version is set aside as `session.json.corrupt-<time>` and the session starts fresh;
/// [`Store::load_error`] says why.
pub fn open(dir: impl Into<PathBuf>) -> Store<SessionState> {
    let store = Store::open(dir.into().join("session.json"), SESSION_VERSION, Some(migrate));
    let loaded = store.get();
    let normal = normalize(&loaded);
    if normal != *loaded {
        store.update(|s| *s = normal);
    }
    store
}

pub fn blank_tab(id: impl Into<TabId>) -> SessionTab {
    SessionTab { id: id.into(), ..SessionTab::default() }
}

fn or_default(v: f64, fallback: f64) -> f64 {
    if v > 0.0 && v.is_finite() { v } else { fallback }
}

/// A new tab is stored with an empty address, so a stored `about:blank` is never one the user
/// opened: it is a page-opened tab whose destination never arrived, and restoring it restores
/// a dead row.
fn restorable(t: &SessionTab) -> bool {
    t.url != "about:blank" && !t.id.is_empty()
}

/// The number after an id's prefix letter, for keeping counters ahead of ids already handed out.
fn id_number(id: &str) -> u64 {
    id.get(1..).and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// A restored session has at least one space and one window, every window at least one tab in
/// the space it shows, every tab exactly once across the session's trees and pages, and
/// counters ahead of every id in use. A window whose every tab was dropped goes with them,
/// unless it is the only one, which keeps its folders and separators. The tab on show leads its
/// window's recency order and has its split's focus.
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
        unknown: state.unknown.clone(),
    };
    let mut space_ids = HashSet::new();
    for space in &state.spaces {
        if !space.id.is_empty() && space_ids.insert(space.id.clone()) {
            out.spaces.push(space.clone());
        }
    }
    let item_ids = state.spaces.iter().map(|s| s.id.as_str()).chain(
        state.windows.iter().flat_map(|w| w.spaces.sidebars.values().flat_map(|t| t.items.iter().flat_map(row_ids))),
    );
    out.next_item_id = out.next_item_id.max(item_ids.map(id_number).max().unwrap_or(0).saturating_add(1));
    let tab_numbers = state.windows.iter().flat_map(|w| w.tabs.iter()).chain(state.archive.iter().map(|a| &a.tab));
    out.next_tab_id = out.next_tab_id.max(tab_numbers.map(|t| id_number(&t.id)).max().unwrap_or(0).saturating_add(1));
    let window_numbers = state.windows.iter().map(|w| id_number(&w.id));
    out.next_window_id = out.next_window_id.max(window_numbers.max().unwrap_or(0).saturating_add(1));
    if out.spaces.is_empty() {
        let id = out.alloc_item_id('s');
        space_ids.insert(id.clone());
        out.spaces.push(Space { id, name: DEFAULT_SPACE_NAME.into(), ..Space::default() });
    }
    let first_space = out.spaces[0].id.clone();
    out.space_rules = state.space_rules.iter().filter(|r| space_ids.contains(&r.space_id)).cloned().collect();

    // Rows may not take a tab's id anywhere: tabs move between windows and come back from the
    // archive.
    let tab_ids: HashSet<String> = state
        .windows
        .iter()
        .flat_map(|w| w.tabs.iter().map(|t| t.id.clone()))
        .chain(state.archive.iter().map(|a| a.tab.id.clone()))
        .collect();
    let mut claimed: HashSet<String> = HashSet::new();
    let mut seen_pages: HashSet<String> = HashSet::new();
    let mut windows: Vec<(bool, SessionWindow)> = Vec::new();
    for w in &state.windows {
        let tabs: Vec<SessionTab> =
            w.tabs.iter().filter(|t| restorable(t) && seen_pages.insert(t.id.clone())).cloned().collect();
        let known: HashSet<&str> = tabs.iter().map(|t| t.id.as_str()).collect();
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
                let mut new_id = || loop {
                    let id = out.alloc_item_id('i');
                    if !tab_ids.contains(&id) {
                        break id;
                    }
                };
                tree.repair(|t| known.contains(t) && claimed.insert(t.to_owned()), &tab_ids, &mut new_id);
            }
        }
        let shown = spaces.tree_mut();
        for t in &tabs {
            if claimed.insert(t.id.clone()) {
                let _ = shown.insert_tab(&t.id, Dest::Root(usize::MAX));
            }
        }
        let had_tabs = !known.is_empty();
        // Both or neither: one coordinate alone places the window nowhere sensible.
        let at = w.x.zip(w.y).filter(|(x, y)| x.is_finite() && y.is_finite());
        let window = SessionWindow {
            id: w.id.clone(),
            active_id: w.active_id.clone(),
            width: or_default(w.width, WINDOW_WIDTH),
            height: or_default(w.height, WINDOW_HEIGHT),
            x: at.map(|(x, _)| x),
            y: at.map(|(_, y)| y),
            spaces,
            mru: w.mru.clone(),
            unknown: w.unknown.clone(),
            tabs,
        };
        windows.push((had_tabs, window));
    }
    if windows.iter().any(|(had_tabs, _)| *had_tabs) {
        windows.retain(|(had_tabs, _)| *had_tabs);
    } else {
        windows.truncate(1);
    }
    let mut windows: Vec<SessionWindow> = windows.into_iter().map(|(_, w)| w).collect();
    if windows.is_empty() {
        windows.push(SessionWindow {
            id: out.alloc_window_id(),
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
            spaces: WindowSpaces::single(&first_space, SidebarTree::default()),
            ..SessionWindow::default()
        });
    }
    for w in &mut windows {
        if w.spaces.tree_mut().tab_ids().is_empty() {
            let id = out.alloc_tab_id();
            w.tabs.push(blank_tab(id.clone()));
            let _ = w.spaces.tree_mut().insert_tab(&id, Dest::Root(usize::MAX));
        }
        let shown = w.spaces.tree().expect("space on show has a sidebar");
        let active = [&w.active_id, &shown.last_active]
            .into_iter()
            .find(|id| shown.contains_tab(id))
            .cloned()
            .unwrap_or_else(|| shown.tab_ids()[0].to_owned());
        let in_window: HashSet<String> = w.tabs.iter().map(|t| t.id.clone()).collect();
        w.mru.retain(|t| in_window.contains(t));
        w.show_tab(&active);
    }
    out.windows = windows;
    // An archived tab is nowhere else: one whose id a live tab or an earlier entry holds takes
    // a new one.
    let mut held: HashSet<String> = out.windows.iter().flat_map(|w| w.tabs.iter().map(|t| t.id.clone())).collect();
    for i in 0..out.archive.len() {
        if out.archive[i].tab.id.is_empty() || !held.insert(out.archive[i].tab.id.clone()) {
            let id = out.alloc_tab_id();
            held.insert(id.clone());
            out.archive[i].tab.id = id;
        }
    }
    out
}

/// The ids of a row and the splits in it that are not tabs: separators, folders and splits.
pub(crate) fn row_ids(item: &Item) -> Vec<&str> {
    match item {
        Item::Tab { .. } => Vec::new(),
        Item::Folder(f) => std::iter::once(f.id.as_str())
            .chain(f.children.iter().filter_map(|c| match c {
                Child::Split(s) => Some(s.id.as_str()),
                Child::Tab { .. } => None,
            }))
            .collect(),
        other => vec![other.id()],
    }
}

/// The window a tab lives in, or `None` for a tab that is gone.
pub fn window_of_tab<'a>(state: &'a SessionState, tab_id: &str) -> Option<&'a SessionWindow> {
    state.windows.iter().find(|w| w.tab(tab_id).is_some())
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

/// Where Chrome puts a tab a page opened in `tree`, as the list index
/// [`SidebarTree::add_tab`] takes, and whether it is shown. A tab in front goes right after its
/// opener's row. One behind it goes after the opener's other tabs still next to it, so a run of
/// ctrl-clicks reads top to bottom in click order. A pinned opener's tab leads the list.
/// `openers` maps a tab to the tab that opened it.
pub fn place_opened_tab(
    tree: &SidebarTree,
    opener_id: &str,
    openers: &HashMap<String, String>,
    disposition: Option<OpenDisposition>,
) -> Placement {
    let foreground = disposition != Some(OpenDisposition::BackgroundTab);
    let start = if tree.is_pinned(opener_id) { Some(0) } else { tree.root_row_of(opener_id).map(|from| from + 1) };
    let Some(mut index) = start else { return Placement { index: tree.items.len(), foreground } };
    while !foreground
        && let Some(Item::Tab { id }) = tree.items.get(index)
        && openers.get(id).is_some_and(|o| o == opener_id)
    {
        index += 1;
    }
    Placement { index, foreground }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str, url: &str) -> SessionTab {
        SessionTab { id: id.into(), url: url.into(), title: id.into(), ..SessionTab::default() }
    }

    fn window(id: &str, tabs: Vec<SessionTab>, active: &str, width: f64, height: f64) -> SessionWindow {
        SessionWindow { id: id.into(), tabs, active_id: active.into(), width, height, ..SessionWindow::default() }
    }

    fn drawn(w: &SessionWindow) -> Vec<&str> {
        w.spaces.tab_ids()
    }

    fn write(text: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("session.json"), text).unwrap();
        dir
    }

    fn load(text: &str) -> (SessionState, Option<String>) {
        let dir = write(text);
        let store = open(dir.path());
        let error = store.load_error().map(str::to_owned);
        (store.get().as_ref().clone(), error)
    }

    fn check_all(s: &SessionState) {
        let mut seen = HashSet::new();
        for w in &s.windows {
            let pages: HashSet<&str> = w.tabs.iter().map(|t| t.id.as_str()).collect();
            let drawn: HashSet<&str> = w.spaces.tab_ids().into_iter().collect();
            assert_eq!(pages, drawn, "window {} pages and trees differ", w.id);
            assert_eq!(pages.len(), w.tabs.len(), "window {} lists a page twice", w.id);
            for tree in w.spaces.sidebars.values() {
                tree.check().unwrap();
            }
            for id in pages {
                assert!(seen.insert(id.to_owned()), "{id} in two windows");
            }
            assert!(w.spaces.tree().unwrap().contains_tab(&w.active_id));
            assert_eq!(w.mru.0.first(), Some(&w.active_id), "window {} recency leads with the tab on show", w.id);
        }
    }

    /// A session of `n` tabs, each with a long address, so its file runs to `n` kilobytes.
    fn big(n: usize) -> SessionState {
        let tabs: Vec<SessionTab> =
            (1..=n).map(|i| tab(&format!("t{i}"), &format!("https://a.test/{}", "x".repeat(1000)))).collect();
        normalize(&SessionState { windows: vec![window("w1", tabs, "t1", 900., 700.)], ..SessionState::default() })
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_profile_keeps_its_session_and_the_browser_going() {
        use std::os::unix::fs::PermissionsExt;
        let dir = write(
            r#"{"version":3,"data":{"windows":[{"id":"w1","tabs":[{"id":"t1","url":"https://a.test/"}],"activeId":"t1"}]}}"#,
        );
        let before = std::fs::read_to_string(dir.path().join("session.json")).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        let store = open(dir.path());
        assert_eq!(store.load_error(), None);
        store.update(|s| *s = big(3));
        assert!(store.flush().is_err(), "nothing can be written");
        assert!(store.flush().is_err(), "and the change stays pending, to go out once it can");
        assert_eq!(std::fs::read_to_string(dir.path().join("session.json")).unwrap(), before);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        store.flush().unwrap();
        assert_eq!(open(dir.path()).get().windows[0].tabs.len(), 3);
    }

    /// A full disk mid-write: a 1 MB disk image filled to the last byte.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_full_disk_keeps_the_last_whole_session() {
        let scratch = tempfile::tempdir().unwrap();
        let image = scratch.path().join("full.dmg");
        let mount = scratch.path().join("mnt");
        std::fs::create_dir(&mount).unwrap();
        let run = |args: &[&std::ffi::OsStr]| {
            let out = std::process::Command::new("hdiutil").args(args).output().unwrap();
            assert!(out.status.success(), "hdiutil {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        };
        run(&["create".as_ref(), "-size".as_ref(), "1m".as_ref(), "-fs".as_ref(), "HFS+".as_ref(), image.as_os_str()]);
        run(&["attach".as_ref(), "-nobrowse".as_ref(), "-mountpoint".as_ref(), mount.as_os_str(), image.as_os_str()]);
        struct Detach(PathBuf);
        impl Drop for Detach {
            fn drop(&mut self) {
                let _ = std::process::Command::new("hdiutil")
                    .args(["detach".as_ref(), "-force".as_ref(), self.0.as_os_str()])
                    .output();
            }
        }
        let _detach = Detach(mount.clone());
        let store = open(&mount);
        store.update(|s| *s = big(2));
        store.flush().unwrap();
        let before = std::fs::read_to_string(mount.join("session.json")).unwrap();
        let mut filler = std::fs::File::create(mount.join("filler")).unwrap();
        let block = vec![0u8; 4096];
        while std::io::Write::write_all(&mut filler, &block).is_ok() {}
        drop(filler);
        store.update(|s| *s = big(200));
        assert!(store.flush().is_err(), "the disk is full");
        assert_eq!(std::fs::read_to_string(mount.join("session.json")).unwrap(), before, "the last session is whole");
        let leftovers: Vec<_> = std::fs::read_dir(&mount)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "no half-written file is left: {leftovers:?}");
        assert_eq!(open(&mount).get().windows[0].tabs.len(), 2);
    }

    #[test]
    fn a_window_keeps_its_place_and_a_tab_its_scroll() {
        let (s, error) = load(
            r#"{"version":3,"data":{"windows":[
                {"id":"w1","tabs":[{"id":"t1","url":"https://a.test/","title":"A","scroll":[0,640]}],
                 "activeId":"t1","width":900,"height":700,"x":-1400,"y":120},
                {"id":"w2","tabs":[{"id":"t2","url":"https://b.test/","title":"B","scroll":null}],
                 "activeId":"t2","width":900,"height":700,"x":40}]}}"#,
        );
        assert_eq!(error, None);
        check_all(&s);
        assert_eq!((s.windows[0].x, s.windows[0].y), (Some(-1400.), Some(120.)));
        assert_eq!(s.windows[0].tabs[0].scroll, Some([0., 640.]));
        assert_eq!((s.windows[1].x, s.windows[1].y), (None, None), "one coordinate places nothing");
        assert_eq!(s.windows[1].tabs[0].scroll, None);
    }

    #[test]
    fn a_pages_tab_lands_where_chrome_puts_it() {
        let list = |ids: &[&str]| SidebarTree {
            pins: vec!["p".into()],
            items: ids.iter().map(|id| Item::tab(*id)).collect(),
            ..SidebarTree::default()
        };
        let tabs = list(&["a", "b", "c"]);
        let none = HashMap::new();
        let at = |index, foreground| Placement { index, foreground };
        assert_eq!(place_opened_tab(&tabs, "a", &none, Some(OpenDisposition::ForegroundTab)), at(1, true));
        assert_eq!(place_opened_tab(&tabs, "a", &none, Some(OpenDisposition::Popup)), at(1, true));
        assert_eq!(place_opened_tab(&tabs, "a", &none, None), at(1, true));
        assert_eq!(place_opened_tab(&tabs, "a", &none, Some(OpenDisposition::BackgroundTab)), at(1, false));
        // Two earlier ctrl-clicks from a sit right after it: the third goes after them.
        let run = list(&["a", "x", "y", "c"]);
        let openers: HashMap<String, String> = [("x", "a"), ("y", "a")].map(|(k, v)| (k.into(), v.into())).into();
        assert_eq!(place_opened_tab(&run, "a", &openers, Some(OpenDisposition::BackgroundTab)), at(3, false));
        assert_eq!(place_opened_tab(&run, "a", &openers, Some(OpenDisposition::ForegroundTab)), at(1, true));
        assert_eq!(place_opened_tab(&tabs, "p", &none, Some(OpenDisposition::ForegroundTab)), at(0, true));
        assert_eq!(place_opened_tab(&tabs, "gone", &none, Some(OpenDisposition::BackgroundTab)), at(3, false));
    }

    #[test]
    fn a_pages_tab_lands_after_its_opener_across_separators() {
        let mut tree = SidebarTree {
            items: vec![Item::Separator { id: "s".into(), name: "Work".into() }, Item::tab("a"), Item::tab("c")],
            ..SidebarTree::default()
        };
        let at = place_opened_tab(&tree, "a", &HashMap::new(), Some(OpenDisposition::ForegroundTab));
        tree.add_tab("n", Some("a"), at.index).unwrap();
        let rows: Vec<&str> = tree.items.iter().map(Item::id).collect();
        assert_eq!(rows, ["s", "a", "n", "c"]);
    }

    #[test]
    fn a_window_left_without_tabs_keeps_its_folders_and_separators() {
        let mut w = window("w1", vec![tab("t1", "about:blank")], "t1", 0.0, 0.0);
        let mut tree = SidebarTree::default();
        tree.insert_tab("t1", Dest::Root(0)).unwrap();
        tree.new_folder("i1", crate::sidebar_tree::FolderKind::Plain, "Daily", "", None, 0).unwrap();
        tree.add_separator_below("t1", "i2", "Later").unwrap();
        w.spaces = WindowSpaces::single("s1", tree);
        let spaces = vec![Space { id: "s1".into(), ..Space::default() }];
        let out = normalize(&SessionState { windows: vec![w], spaces, ..SessionState::default() });
        let tree = out.windows[0].spaces.tree().unwrap();
        assert!(tree.folder("i1").is_some());
        assert!(tree.items.iter().any(|i| i.id() == "i2"));
        check_all(&out);
    }

    #[test]
    fn a_null_pin_drops_alone() {
        let (s, _) = load(
            r#"{"version":3,"data":{"windows":[{"id":"w1","tabs":[{"id":"t1","url":"https://a.test/"}],
            "activeId":"t1","spaces":{"shown":"s1","sidebars":{"s1":{"pins":[null,"t1",7]}}}}],
            "spaces":[{"id":"s1","name":"Personal"}]}}"#,
        );
        assert_eq!(s.windows[0].id, "w1");
        assert_eq!(s.windows[0].spaces.tree().unwrap().pins, ["t1"]);
    }

    #[test]
    fn a_row_named_like_a_tab_gives_way_to_the_tab() {
        let mut w = window("w1", vec![tab("t1", "https://a.test/"), tab("t2", "https://b.test/")], "t2", 0.0, 0.0);
        let tree = SidebarTree {
            items: vec![Item::Separator { id: "t1".into(), name: "Odd".into() }, Item::tab("t2")],
            ..SidebarTree::default()
        };
        w.spaces = WindowSpaces::single("s1", tree);
        let out = normalize(&SessionState { windows: vec![w], ..SessionState::default() });
        let tree = out.windows[0].spaces.tree().unwrap();
        assert!(tree.contains_tab("t1"), "page t1 has a row");
        assert_eq!(tree.items.iter().filter(|i| i.id() == "t1").count(), 1);
        check_all(&out);
    }

    #[test]
    fn a_separator_named_t1_in_a_window_with_no_tabs_gives_way_to_the_blank_tab() {
        let (s, _) = load(
            r#"{"version":3,"data":{"windows":[{"id":"w1","tabs":[],"activeId":"",
            "spaces":{"shown":"s1","sidebars":{"s1":{"items":[{"type":"separator","id":"t1","name":"Later"}]}}}}],
            "spaces":[{"id":"s1","name":"Personal"}],"nextTabId":1}}"#,
        );
        let w = &s.windows[0];
        assert_eq!(w.tabs.len(), 1);
        let tree = w.spaces.tree().unwrap();
        assert!(tree.contains_tab(&w.tabs[0].id));
        let separators: Vec<&Item> = tree.items.iter().filter(|i| matches!(i, Item::Separator { .. })).collect();
        assert_eq!(separators.len(), 1);
        assert_eq!(separators[0], &Item::Separator { id: separators[0].id().to_owned(), name: "Later".into() });
        assert_ne!(separators[0].id(), w.tabs[0].id);
        check_all(&s);
    }

    #[test]
    fn background_tabs_from_the_only_pinned_tab_keep_click_order() {
        let mut tree = SidebarTree { pins: vec!["p".into()], ..SidebarTree::default() };
        let mut openers = HashMap::new();
        for id in ["x1", "x2", "x3"] {
            let at = place_opened_tab(&tree, "p", &openers, Some(OpenDisposition::BackgroundTab));
            tree.add_tab(id, Some("p"), at.index).unwrap();
            openers.insert(id.to_owned(), "p".to_owned());
        }
        assert_eq!(tree.tab_ids(), ["p", "x1", "x2", "x3"]);
    }

    #[test]
    fn an_empty_session_at_the_last_window_number_opens() {
        let out = normalize(&SessionState { next_window_id: u64::MAX, ..SessionState::default() });
        assert_eq!(out.windows.len(), 1);
    }

    #[test]
    fn tab_ids_carry_the_shells_number() {
        assert_eq!(tab_id(7), "t7");
        assert_eq!(tab_number("t7"), Some(7));
        assert_eq!(tab_number(&tab_id(u64::MAX)), Some(u64::MAX));
        assert_eq!(tab_number("x7"), None);
        assert_eq!(tab_number("t"), None);
    }

    #[test]
    fn normalize_drops_dead_rows_and_keeps_one_window() {
        let state = SessionState {
            windows: vec![
                window("w1", vec![tab("t1", "about:blank")], "t1", 0.0, 0.0),
                window("w2", vec![tab("t2", "https://b.test/"), tab("t3", "https://c.test/")], "gone", 0.0, 600.0),
            ],
            next_tab_id: 4,
            next_window_id: 3,
            ..SessionState::default()
        };
        let out = normalize(&state);
        assert_eq!(out.windows.len(), 1);
        assert_eq!(drawn(&out.windows[0]), ["t2", "t3"]);
        assert_eq!(out.windows[0].active_id, "t2");
        assert_eq!((out.windows[0].width, out.windows[0].height), (WINDOW_WIDTH, 600.0));
        check_all(&out);

        let empty = normalize(&SessionState::default());
        assert_eq!(empty.windows[0].id, "w1");
        assert_eq!(drawn(&empty.windows[0]), ["t1"]);
        assert_eq!((empty.next_tab_id, empty.next_window_id), (2, 2));
    }

    #[test]
    fn the_real_old_profile_opens_as_one_space_with_its_tab() {
        let (s, error) = load(include_str!("../fixtures/session-0511-real.json"));
        assert_eq!(error, None);
        assert_eq!(s.windows.len(), 1);
        let w = &s.windows[0];
        assert_eq!((w.id.as_str(), w.active_id.as_str()), ("w1", "t1"));
        assert_eq!((w.width, w.height), (1280.0, 544.0));
        assert_eq!(drawn(w), ["t1"]);
        assert_eq!(w.tabs[0], blank_tab("t1"));
        assert_eq!((s.next_tab_id, s.next_window_id), (2, 2));
        assert_eq!(s.spaces.len(), 1);
        check_all(&s);
    }

    #[test]
    fn a_version_two_file_opens_as_one_space_of_pins_and_loose_tabs() {
        let (s, error) = load(include_str!("../fixtures/session-v2.json"));
        assert_eq!(error, None);
        assert_eq!((s.spaces[0].id.as_str(), s.spaces[0].name.as_str()), ("s1", DEFAULT_SPACE_NAME));
        assert_eq!(normalize(&s), s, "a loaded session is already normal");
        let w = &s.windows[0];
        assert_eq!(drawn(w), ["t1", "t2", "t3", "t4", "t5"]);
        assert_eq!(w.active_id, "t3");
        assert_eq!((w.width, w.height), (1440.0, 900.0));
        assert_eq!(w.tab("t2").unwrap().pinned_url.as_deref(), Some("https://calendar.example.com/"));
        assert!(w.tabs.iter().all(|t| t.unknown.is_empty()), "the old pinned flag is not carried");
        let tree = w.spaces.tree().unwrap();
        assert_eq!(w.spaces.shown, "s1");
        assert_eq!(tree.pins, ["t1", "t2"]);
        assert_eq!(tree.loose_tabs(), ["t3", "t4", "t5"]);
        assert_eq!(s.windows[1].id, "w3");
        assert_eq!(s.windows[1].spaces.tree().unwrap().loose_tabs(), ["t7"]);
        assert_eq!((s.next_tab_id, s.next_window_id, s.next_item_id), (8, 4, 2));
        assert_eq!(s.zoom_by_host["github.com"], 1.1);
        check_all(&s);
    }

    #[test]
    fn version_three_round_trips_through_the_store() {
        let (mut s, _) = load(include_str!("../fixtures/session-v2.json"));
        let sep = s.alloc_item_id('i');
        let folder = s.alloc_item_id('i');
        let tree = s.windows[0].spaces.tree_mut();
        tree.add_separator_below("t3", &sep, "Reading").unwrap();
        tree.new_folder(&folder, crate::sidebar_tree::FolderKind::Project, "Lynk", "code", Some(280), 9).unwrap();
        tree.move_row("t5", Dest::Folder { id: folder.clone(), index: 0 }).unwrap();
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
        assert_eq!(back.get().windows, s.windows);
        assert_eq!(back.get().next_item_id, 10);
        let text = std::fs::read_to_string(dir.path().join("session.json")).unwrap();
        assert!(text.contains("\"version\": 3"), "{text}");
        assert!(!text.contains("\"pinned\""), "pins live in the tree only");
    }

    #[test]
    fn normalize_keeps_each_tab_once_and_fills_an_empty_space() {
        let mut w1 = window("w1", vec![tab("a", "https://a.test/"), tab("b", "https://b.test/")], "a", 1.0, 1.0);
        let tree =
            SidebarTree { items: vec![Item::tab("a"), Item::tab("ghost"), Item::tab("a")], ..SidebarTree::default() };
        w1.spaces = WindowSpaces::single("s1", tree);
        w1.spaces.sidebars.insert("s2".into(), SidebarTree { pins: vec!["b".into()], ..SidebarTree::default() });
        w1.spaces.sidebars.insert("gone".into(), SidebarTree { pins: vec!["a".into()], ..SidebarTree::default() });
        let w2 = window("w2", vec![tab("a", "https://dup.test/"), tab("c", "https://c.test/")], "a", 1.0, 1.0);
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
        // w2's copy of "a" is gone, and its own tab survives.
        assert_eq!(drawn(&s.windows[1]), ["c"]);
        assert_eq!(s.windows[1].tabs.len(), 1);
        assert_eq!(s.windows[1].spaces.shown, "s1");
        check_all(&s);

        let mut only_other = window("w1", vec![tab("b", "https://b.test/")], "b", 1.0, 1.0);
        only_other.spaces =
            WindowSpaces::single("s2", SidebarTree { items: vec![Item::tab("b")], ..SidebarTree::default() });
        only_other.spaces.shown = "s1".into();
        let s = normalize(&SessionState { windows: vec![only_other], next_tab_id: 5, ..state });
        let w = &s.windows[0];
        assert_eq!(w.spaces.tree().unwrap().tab_ids(), ["t5"]);
        assert_eq!(w.active_id, "t5");
        assert_eq!(drawn(w), ["t5", "b"]);
        assert_eq!(s.next_tab_id, 6);
        check_all(&s);
    }

    #[test]
    fn recency_survives_closing_and_reopening_the_app() {
        let (mut s, _) = load(include_str!("../fixtures/session-v2.json"));
        for id in ["t5", "t3", "t1"] {
            s.windows[0].show_tab(id);
        }
        s.windows[0].mru.shown("gone");
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.update(|state| *state = s.clone());
        drop(store);
        assert_eq!(open(dir.path()).get().windows[0].mru.0, ["t1", "t3", "t5"]);
    }

    #[test]
    fn nulls_in_session_json_take_their_defaults() {
        let (s, error) = load(
            r#"{"version":2,"data":{"windows":[{"id":"w1","tabs":[{"id":"t1","url":"https://a.test/","title":null,"pinned":null}],"activeId":null,"width":null,"height":null},{"id":"w2","tabs":null}],"nextTabId":null,"nextWindowId":3,"zoomByHost":null}}"#,
        );
        assert_eq!(error, None);
        assert_eq!(drawn(&s.windows[0]), ["t1"]);
        assert_eq!(s.windows[0].tabs[0].title, "");
        assert_eq!((s.windows[0].width, s.windows[0].height), (WINDOW_WIDTH, WINDOW_HEIGHT));
        assert_eq!(s.windows.len(), 1);
        // A null counter would hand out t1 again; it moves past the ids in use.
        assert_eq!((s.next_tab_id, s.next_window_id), (2, 3));

        let (s, error) = load(r#"{"version":2,"data":{"windows":null}}"#);
        assert_eq!(error, None);
        assert_eq!(s.windows.len(), 1);
    }

    #[test]
    fn session_json_versions_one_and_two_are_read() {
        let (s, error) = load(
            r#"{"version":1,"data":{"tabs":[{"id":"t1","url":"https://a.test/","title":"A"},{"id":"t2","url":"https://b.test/","title":"B","pinned":"yes"}],"activeId":"t2","nextTabId":3,"windowWidth":1000,"zoomByHost":{"a.test":1.25}}}"#,
        );
        assert_eq!(error, None);
        assert_eq!(s.windows.len(), 1);
        assert_eq!(s.windows[0].id, "w1");
        assert_eq!(s.windows[0].spaces.tree().unwrap().loose_tabs(), ["t1", "t2"]);
        assert_eq!(s.windows[0].active_id, "t2");
        assert_eq!((s.windows[0].width, s.windows[0].height), (1000.0, WINDOW_HEIGHT));
        assert_eq!((s.next_tab_id, s.next_window_id), (3, 2));
        assert_eq!(s.zoom_by_host["a.test"], 1.25);
        let text = serde_json::to_value(&s).unwrap();
        assert!(text["windows"][0]["tabs"][0].get("pinnedUrl").is_none());
    }

    #[test]
    fn a_cut_short_empty_or_null_file_starts_fresh_and_is_set_aside() {
        let full = include_str!("../fixtures/session-v2.json");
        for (name, text) in
            [("cut short", &full[..full.len() / 2]), ("empty", ""), ("null", "null"), ("no data", r#"{"version":3}"#)]
        {
            let dir = write(text);
            let store = open(dir.path());
            assert!(store.load_error().is_some(), "{name}");
            let s = store.get();
            assert_eq!(s.windows.len(), 1, "{name}");
            assert_eq!(drawn(&s.windows[0]), ["t1"], "{name}");
            check_all(&s);
            let aside = std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .any(|e| e.file_name().to_string_lossy().starts_with("session.json.corrupt-"));
            assert!(aside, "{name}: the unreadable file is kept");
        }
    }

    #[test]
    fn a_newer_versions_file_is_set_aside_whole() {
        let dir = write(r#"{"version":4,"data":{"windows":[]}}"#);
        let store = open(dir.path());
        assert!(store.load_error().unwrap().contains("newer"));
    }

    #[test]
    fn a_session_the_shell_wrote_before_pins_opens_as_default_and_keeps_pins_from_then_on() {
        let dir = write(include_str!("../fixtures/session-shell-before-pins.json"));
        let profiles = crate::profiles::open(dir.path());
        assert_eq!(profiles.get().open_ids(), [crate::profiles::DEFAULT_ID]);
        let root = crate::profiles::data_dir(dir.path(), crate::profiles::DEFAULT_ID);
        let store = open(&root);
        assert_eq!(store.load_error(), None);
        let s = store.get();
        let w = &s.windows[0];
        assert_eq!(drawn(w), ["t2", "t3"]);
        assert!(w.spaces.tree().unwrap().pins.is_empty());
        assert!(w.tabs.iter().all(|t| t.pinned_url.is_none()));

        store.update(|s| {
            let w = &mut s.windows[0];
            w.spaces.tree_mut().set_pinned("t3", true).unwrap();
            w.tabs[1].pinned_url = Some(w.tabs[1].url.clone());
            w.tabs[1].url = "http://127.0.0.1:8765/alpha.html#later".into();
        });
        store.flush().unwrap();
        let again = open(&root).get();
        let w = &again.windows[0];
        assert_eq!(w.spaces.tree().unwrap().pins, ["t3"]);
        assert_eq!(w.spaces.tree().unwrap().loose_tabs(), ["t2"]);
        let pin = w.tab("t3").unwrap();
        assert_eq!(pin.pinned_url.as_deref(), Some("http://127.0.0.1:8765/beta.html"));
        assert!(crate::tabstate::left_pinned_page(pin.pinned_url.as_deref(), &pin.url));
        check_all(&again);
    }

    #[test]
    fn fields_a_newer_build_added_survive_a_round_trip() {
        let text = r#"{"version":3,"data":{"windows":[{"id":"w1","tabs":[{"id":"t1","url":"https://a.test/","title":"A","muted":true}],"activeId":"t1","width":900,"height":700,"spaces":{"shown":"s1","sidebars":{"s1":{"pins":[],"items":[{"type":"tab","id":"t1"}],"lastActive":"t1"}}},"mru":["t1"],"zoomLevel":2}],"nextTabId":2,"nextWindowId":2,"zoomByHost":{},"spaces":[{"id":"s1","name":"Personal","theme":{"hue":220,"accentHue":260}}],"spaceRules":[],"nextItemId":2,"archive":[],"futureThing":{"a":[1,2]}}}"#;
        let dir = write(text);
        let store = open(dir.path());
        assert_eq!(store.load_error(), None);
        let s = store.get();
        assert_eq!(s.unknown["futureThing"], json!({"a": [1, 2]}));
        assert_eq!(s.windows[0].unknown["zoomLevel"], json!(2));
        assert_eq!(s.windows[0].tabs[0].unknown["muted"], json!(true));
        let written = serde_json::to_value(&*s).unwrap();
        assert_eq!(written["futureThing"], json!({"a": [1, 2]}));
        assert_eq!(written["windows"][0]["tabs"][0]["muted"], json!(true));
    }

    #[test]
    fn duplicate_ids_within_and_across_windows_keep_the_first() {
        let (s, error) = load(
            r#"{"version":2,"data":{"windows":[{"id":"w1","tabs":[{"id":"t1","url":"https://a.test/"},{"id":"t1","url":"https://b.test/"},{"id":"t2","url":"https://c.test/","pinned":true}],"activeId":"t1"},{"id":"w2","tabs":[{"id":"t2","url":"https://d.test/"},{"id":"t3","url":"https://e.test/"}],"activeId":"t2"}],"nextTabId":4,"nextWindowId":3}}"#,
        );
        assert_eq!(error, None);
        assert_eq!(drawn(&s.windows[0]), ["t2", "t1"]);
        assert_eq!(s.windows[0].tab("t1").unwrap().url, "https://a.test/");
        assert_eq!(drawn(&s.windows[1]), ["t3"]);
        assert_eq!(s.windows[1].active_id, "t3");
        check_all(&s);
    }

    #[test]
    fn five_thousand_tabs_load_in_order() {
        let tabs: Vec<Value> = (1..=5000)
            .map(|n| json!({"id": format!("t{n}"), "url": format!("https://h{}.test/{n}", n % 97), "pinned": n <= 10}))
            .collect();
        let text = json!({"version": 2, "data": {"windows": [{"id": "w1", "tabs": tabs, "activeId": "t4000"}], "nextTabId": 5001}}).to_string();
        let (s, error) = load(&text);
        assert_eq!(error, None);
        let tree = s.windows[0].spaces.tree().unwrap();
        assert_eq!(tree.pins.len(), 10);
        assert_eq!(tree.loose_tabs().len(), 4990);
        assert_eq!(tree.loose_tabs()[0], "t11");
        assert_eq!(s.windows[0].active_id, "t4000");
        check_all(&s);
    }

    #[test]
    fn ids_at_the_numeric_limit_do_not_overflow() {
        let max = u64::MAX;
        let text = format!(
            r#"{{"version":2,"data":{{"windows":[{{"id":"w{max}","tabs":[{{"id":"t{max}","url":"https://a.test/"}},{{"id":"t99999999999999999999999","url":"https://b.test/"}}],"activeId":"t{max}"}}],"nextTabId":{max},"nextWindowId":{max}}}}}"#
        );
        let (mut s, error) = load(&text);
        assert_eq!(error, None);
        assert_eq!(drawn(&s.windows[0]).len(), 2);
        assert_eq!(s.next_tab_id, max);
        let _ = s.alloc_tab_id();
        let _ = s.alloc_item_id('i');
        check_all(&s);
    }

    #[test]
    fn odd_text_in_titles_and_addresses_round_trips() {
        let big = "x".repeat(100 * 1024);
        let odd = [
            ("t1", "https://例え.テスト/😀?q=🦀", "Grüße 😀👩‍👩‍👧 family"),
            ("t2", "https://example.test/שלום", "שלום עולם مرحبا بالعالم"),
            ("t3", "https://nul.test/a\u{0}b", "before\u{0}after"),
            ("t4", &format!("https://big.test/{big}"), &big),
        ];
        let state = SessionState {
            windows: vec![window(
                "w1",
                odd.iter().map(|(id, url, title)| SessionTab { title: (*title).into(), ..tab(id, url) }).collect(),
                "t1",
                900.0,
                700.0,
            )],
            ..SessionState::default()
        };
        let state = normalize(&state);
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        store.update(|s| *s = state.clone());
        store.flush().unwrap();
        drop(store);
        let back = open(dir.path());
        assert_eq!(back.load_error(), None);
        assert_eq!(*back.get(), state);
        assert_eq!(back.get().windows[0].tab("t3").unwrap().title, "before\u{0}after");
    }
}
