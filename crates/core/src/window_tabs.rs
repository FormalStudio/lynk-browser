//! Changing a loaded session's tabs: open, close, move between windows, and the fresh start.
//! Each change keeps a window's pages and its sidebar trees naming the same tabs. Where a tab
//! goes inside a tree is `sidebar_tree`'s to decide; reading and repairing the file is
//! `session`'s.

use std::collections::BTreeMap;

use crate::session::{SessionState, SessionTab, SessionWindow, TabId, blank_tab, normalize};
use crate::sidebar_tree::{Dest, TreeError};
use crate::spaces::{WindowSpaces, neighbour};

impl SessionWindow {
    /// A new tab in the space on show, placed by [`crate::sidebar_tree::SidebarTree::add_tab`]:
    /// into the project in front, else at `root_index` in the list.
    pub fn add_tab(&mut self, tab: SessionTab, front: Option<&str>, root_index: usize) -> Result<(), TreeError> {
        if self.tab(&tab.id).is_some() {
            return Err(TreeError::AlreadyPresent);
        }
        self.spaces.tree_mut().add_tab(&tab.id, front, root_index)?;
        self.tabs.push(tab);
        Ok(())
    }

    /// A tab into `space`'s sidebar at `dest`, which need not be the space on show.
    pub fn insert_tab(&mut self, tab: SessionTab, space: &str, dest: Dest) -> Result<(), TreeError> {
        if self.tab(&tab.id).is_some() {
            return Err(TreeError::AlreadyPresent);
        }
        self.spaces.sidebars.entry(space.to_owned()).or_default().insert_tab(&tab.id, dest)?;
        self.tabs.push(tab);
        Ok(())
    }

    /// Puts a tab of the space on show on show: it leads the recency order and gets its split's
    /// focus. An empty id shows nothing, for a space with no tabs. `false` for a tab not in the
    /// space on show.
    pub fn show_tab(&mut self, tab_id: &str) -> bool {
        if tab_id.is_empty() {
            self.active_id.clear();
            return true;
        }
        let tree = self.spaces.tree_mut();
        if !tree.contains_tab(tab_id) {
            return false;
        }
        tree.focus_tab(tab_id);
        self.mru.shown(tab_id);
        self.active_id = tab_id.to_owned();
        true
    }

    /// Takes a tab's page out of the window and its tree. The tab on show hands over to its
    /// neighbour in the sidebar on show, or to nothing when that sidebar is left empty.
    fn take(&mut self, tab_id: &str) -> Option<SessionTab> {
        let at = self.tabs.iter().position(|t| t.id == tab_id)?;
        let order: Vec<TabId> =
            self.spaces.tree().map(|t| t.tab_ids()).unwrap_or_default().into_iter().map(str::to_owned).collect();
        for tree in self.spaces.sidebars.values_mut() {
            tree.close_tab(tab_id);
        }
        self.mru.closed(tab_id);
        if self.active_id == tab_id {
            self.show_tab(&neighbour(&order, tab_id).unwrap_or_default());
        }
        Some(self.tabs.remove(at))
    }
}

impl SessionState {
    /// Closes a tab in whichever window holds it. A window left with no tabs stays for the
    /// caller to close.
    pub fn close_tab(&mut self, tab_id: &str) -> bool {
        self.windows.iter_mut().any(|w| w.take(tab_id).is_some())
    }
}

/// Moves a tab to `dest` in the space `to_window` shows, where it becomes the tab on show.
/// Within one window this is a move between rows or spaces. The source window keeps showing what
/// it showed, or the neighbour when the moved tab was on show; a window left with no tabs stays
/// for the caller to close. Nothing moves when `dest` does not exist.
pub fn move_tab_in(state: &mut SessionState, tab_id: &str, to_window: &str, dest: Dest) -> Result<(), TreeError> {
    let target_at = state.windows.iter().position(|w| w.id == to_window).ok_or(TreeError::UnknownRow)?;
    let from = state.windows.iter().position(|w| w.tab(tab_id).is_some()).ok_or(TreeError::UnknownRow)?;
    if let Dest::Folder { id, .. } = &dest
        && state.windows[target_at].spaces.tree().and_then(|t| t.folder(id)).is_none()
    {
        return Err(TreeError::UnknownRow);
    }
    let tab = state.windows[from].take(tab_id).expect("tab is in its window");
    let target = &mut state.windows[target_at];
    let space = target.spaces.shown.clone();
    target.insert_tab(tab, &space, dest)?;
    target.show_tab(tab_id);
    Ok(())
}

/// "Start with a fresh window": every window's pins and folders, space by space, in one window,
/// with a new tab that opens the homepage when one is set, in the space the first window
/// showed. Numbering carries over, so a fresh id never collides with one still in the store.
pub fn fresh_start(stored: &SessionState, homepage: &str) -> SessionState {
    let mut s = normalize(stored);
    let windows = std::mem::take(&mut s.windows);
    let first = &windows[0];
    let mut spaces = WindowSpaces { shown: first.spaces.shown.clone(), sidebars: BTreeMap::new() };
    for w in &windows {
        for (space, tree) in &w.spaces.sidebars {
            spaces.sidebars.entry(space.clone()).or_default().absorb(tree.saved_part());
        }
    }
    let mut window = SessionWindow { id: first.id.clone(), width: first.width, height: first.height, ..SessionWindow::default() };
    let kept: Vec<SessionTab> =
        windows.iter().flat_map(|w| w.tabs.iter()).filter(|t| spaces.space_of_tab(&t.id).is_some()).cloned().collect();
    window.spaces = spaces;
    window.tabs = kept;
    let id = s.alloc_tab_id();
    let shown = window.spaces.shown.clone();
    window
        .insert_tab(SessionTab { url: homepage.to_owned(), ..blank_tab(id.clone()) }, &shown, Dest::Root(usize::MAX))
        .expect("a fresh id is new");
    window.show_tab(&id);
    s.windows = vec![window];
    s
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::Lcg;
    use crate::mru::Mru;
    use crate::archive::{archive_tabs, restore};
    use crate::session::{open, row_ids, tab_number};
    use crate::sidebar_tree::{FolderKind, SidebarTree};
    use crate::spaces::Space;

    fn fixture() -> SessionState {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("session.json"), include_str!("../fixtures/session-v2.json")).unwrap();
        open(dir.path()).get().as_ref().clone()
    }

    fn drawn(w: &SessionWindow) -> Vec<&str> {
        w.spaces.tab_ids()
    }

    fn pages_match_trees(s: &SessionState) {
        for w in &s.windows {
            let mut pages: Vec<&str> = w.tabs.iter().map(|t| t.id.as_str()).collect();
            let mut drawn = w.spaces.tab_ids();
            pages.sort_unstable();
            drawn.sort_unstable();
            assert_eq!(pages, drawn, "window {}", w.id);
        }
    }

    #[test]
    fn a_moved_tab_lands_at_its_destination_and_the_source_shows_a_neighbour() {
        let mut s = fixture();
        // w1: pins t1 t2, list t3 t4 t5, showing t3. w3: list t7.
        move_tab_in(&mut s, "t3", "w3", Dest::Root(0)).unwrap();
        assert_eq!(drawn(&s.windows[0]), ["t1", "t2", "t4", "t5"]);
        assert_eq!(s.windows[0].active_id, "t4");
        assert_eq!(drawn(&s.windows[1]), ["t3", "t7"]);
        assert_eq!(s.windows[1].active_id, "t3");
        move_tab_in(&mut s, "t1", "w3", Dest::Pins(9)).unwrap();
        assert_eq!(s.windows[1].spaces.tree().unwrap().pins, ["t1"]);
        assert_eq!(move_tab_in(&mut s, "t2", "w3", Dest::Folder { id: "nope".into(), index: 0 }), Err(TreeError::UnknownRow));
        assert!(s.windows[0].tab("t2").is_some(), "a refused move leaves the tab where it was");
        assert_eq!(move_tab_in(&mut s, "t2", "w9", Dest::Root(0)), Err(TreeError::UnknownRow));
        // Within a window it is a reorder, and the tab stays on show.
        s.windows[0].active_id = "t5".into();
        move_tab_in(&mut s, "t5", "w1", Dest::Root(0)).unwrap();
        assert_eq!(drawn(&s.windows[0]), ["t2", "t5", "t4"]);
        assert_eq!(s.windows[0].active_id, "t5");
        pages_match_trees(&s);
    }

    #[test]
    fn closing_and_adding_tabs_keep_pages_and_trees_together() {
        let mut s = fixture();
        assert!(s.close_tab("t3"));
        assert_eq!(s.windows[0].active_id, "t4");
        assert!(!s.close_tab("t3"));
        let id = s.alloc_tab_id();
        s.windows[0].add_tab(blank_tab(id.clone()), None, 0).unwrap();
        assert_eq!(s.windows[0].add_tab(blank_tab(id), None, 0), Err(TreeError::AlreadyPresent));
        assert_eq!(drawn(&s.windows[0]), ["t1", "t2", "t8", "t4", "t5"]);
        assert_eq!(normalize(&s), s);
    }

    #[test]
    fn a_fresh_start_keeps_every_windows_pins_and_folders_and_opens_the_homepage() {
        let mut s = fixture();
        let tree = s.windows[0].spaces.tree_mut();
        tree.new_folder("i5", FolderKind::Plain, "Daily", "", None, 0).unwrap();
        tree.move_row("t5", Dest::Folder { id: "i5".into(), index: 0 }).unwrap();
        s.windows[1].spaces.tree_mut().pin("t7", 0).unwrap();
        let fresh = fresh_start(&s, "https://home.test/");
        assert_eq!(fresh.windows.len(), 1);
        let w = &fresh.windows[0];
        assert_eq!(drawn(w), ["t1", "t2", "t7", "t5", "t8"]);
        assert_eq!(w.active_id, "t8");
        assert_eq!(w.tab("t8").unwrap().url, "https://home.test/");
        assert_eq!((w.width, w.height), (1440.0, 900.0));
        assert_eq!(fresh.next_tab_id, 9);
        assert_eq!(normalize(&fresh), fresh);
        pages_match_trees(&fresh);
    }

    /// One window showing space s1 with a, b, c; space s2 holds h. Recency a, c, b.
    fn two_spaces() -> SessionState {
        let mut shown = SidebarTree::default();
        for t in ["a", "b", "c"] {
            shown.insert_tab(t, Dest::Root(usize::MAX)).unwrap();
        }
        let mut hidden = SidebarTree::default();
        hidden.insert_tab("h", Dest::Root(0)).unwrap();
        let spaces = WindowSpaces { shown: "s1".into(), sidebars: BTreeMap::from([("s1".into(), shown), ("s2".into(), hidden)]) };
        let window = SessionWindow {
            id: "w1".into(),
            tabs: ["a", "b", "c", "h"].map(blank_tab).into(),
            active_id: "a".into(),
            spaces,
            mru: Mru(["a", "c", "b"].map(String::from).into()),
            ..SessionWindow::default()
        };
        let spaces = ["s1", "s2"].map(|id| Space { id: id.into(), ..Space::default() }).into();
        SessionState { windows: vec![window], spaces, ..SessionState::default() }
    }

    /// The rules a window keeps between operations: pages and rows agree; the tab on show is in
    /// the space on show (nothing is on show only when that space is empty), heads the recency
    /// order, and has its split's focus.
    fn check_window(w: &SessionWindow) -> Result<(), String> {
        let mut pages: Vec<&str> = w.tabs.iter().map(|t| t.id.as_str()).collect();
        let mut rows = w.spaces.tab_ids();
        pages.sort_unstable();
        rows.sort_unstable();
        if pages != rows {
            return Err(format!("pages {pages:?} and rows {rows:?} differ"));
        }
        let shown = w.spaces.tree().map(SidebarTree::tab_ids).unwrap_or_default();
        if w.active_id.is_empty() {
            return if shown.is_empty() { Ok(()) } else { Err(format!("nothing on show beside {shown:?}")) };
        }
        if !shown.contains(&w.active_id.as_str()) {
            return Err(format!("{} on show from a hidden space", w.active_id));
        }
        if w.mru.0.first() != Some(&w.active_id) {
            return Err(format!("recency {:?} does not lead with {}", w.mru.0, w.active_id));
        }
        match w.spaces.tree().and_then(|t| t.split_of(&w.active_id)) {
            Some(s) if s.focused() != w.active_id => Err(format!("split {} focuses {}", s.id, s.focused())),
            _ => Ok(()),
        }
    }

    #[test]
    fn closing_the_last_tab_on_show_shows_nothing_from_a_hidden_space() {
        let mut s = two_spaces();
        for t in ["b", "c", "a"] {
            s.close_tab(t);
        }
        assert_eq!(s.windows[0].active_id, "");
        check_window(&s.windows[0]).unwrap();
    }

    #[test]
    fn a_neighbour_in_a_split_comes_on_show_with_the_splits_focus() {
        let mut s = two_spaces();
        let tree = s.windows[0].spaces.tree_mut();
        tree.make_split("x", "b", "c").unwrap();
        tree.focus_tab("c");
        s.close_tab("a");
        assert_eq!(s.windows[0].active_id, "b");
        check_window(&s.windows[0]).unwrap();
    }

    #[test]
    fn the_tab_that_comes_on_show_leads_the_recency_order() {
        let mut s = two_spaces();
        s.close_tab("a");
        assert_eq!(s.windows[0].mru.0, ["b", "c"]);
        move_tab_in(&mut s, "c", "w1", Dest::Root(0)).unwrap();
        assert_eq!(s.windows[0].mru.0, ["c", "b"]);
        check_window(&s.windows[0]).unwrap();
    }

    /// Every window's rules, and across the session: a tab is in exactly one of a window's
    /// sidebars and the archive, no two rows share an id, and no row id is a tab's id, live,
    /// archived, or one the session could hand out later.
    fn check_session(s: &SessionState) -> Result<(), String> {
        for w in &s.windows {
            check_window(w).map_err(|e| format!("in {}: {e}", w.id))?;
        }
        let mut tabs: HashSet<&str> = HashSet::new();
        let placed = s.windows.iter().flat_map(|w| w.spaces.tab_ids()).chain(s.archive.iter().map(|a| a.tab.id.as_str()));
        for id in placed {
            if !tabs.insert(id) {
                return Err(format!("tab {id} is in more than one of the sidebars and the archive"));
            }
        }
        let mut rows = HashSet::new();
        let trees = s.windows.iter().flat_map(|w| w.spaces.sidebars.values());
        for id in trees.flat_map(|t| t.items.iter().flat_map(row_ids)) {
            if !rows.insert(id) {
                return Err(format!("row id {id} twice"));
            }
            if tabs.contains(id) || tab_number(id).is_some() {
                return Err(format!("row id {id} is a tab's"));
            }
        }
        Ok(())
    }

    fn loaded(text: &str) -> SessionState {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("session.json"), text).unwrap();
        open(dir.path()).get().as_ref().clone()
    }

    /// Two windows and two spaces from a file with clashing ids: a separator and a folder named
    /// like tabs, a split id twice, a page in two windows, a null pin, null counters, an archived
    /// tab named like the next row id and one whose id a live tab holds.
    const MALFORMED: &str = r#"{"version":3,"data":{
        "windows":[
            {"id":"w1","tabs":[{"id":"t1","url":"https://a.test/"},{"id":"t2","url":"https://b.test/"},
                {"id":"t3","url":"https://c.test/"},{"id":"t4","url":"https://d.test/"}],
             "activeId":"t3","mru":["t9",null,"t2"],
             "spaces":{"shown":"s2","sidebars":{
                "s1":{"pins":[null,"t1"],"items":[{"type":"separator","id":"t2","name":"Odd"},
                    {"type":"split","id":"i1","tabs":["t3","t4"],"focus":"second"}]},
                "s2":{"items":[{"type":"folder","id":"t7","kind":"plain","name":"F","children":[]},
                    {"type":"split","id":"i1","tabs":["t2","t9"]}]}}}},
            {"id":"w2","tabs":[{"id":"t3","url":"https://c.test/"},{"id":"t5","url":"https://e.test/"}],
             "activeId":"t5","spaces":{"shown":"s1","sidebars":{"s1":{"items":[
                {"type":"separator","id":"t5","name":""},{"type":"tab","id":"t5"}]}}}}],
        "spaces":[{"id":"s1","name":"Personal"},{"id":"s2","name":"Work"}],
        "archive":[{"tab":{"id":"i2","url":"https://f.test/"},"windowId":"w1","spaceId":"s1","above":["t2"]},
            {"tab":{"id":"t1","url":"https://a.test/"},"windowId":"w9","spaceId":"s9"}],
        "nextTabId":null,"nextItemId":null}}"#;

    /// Sol's: a window with no tabs and a separator named `t1`.
    const SEPARATOR_T1: &str = r#"{"version":3,"data":{"windows":[{"id":"w1","tabs":[],"activeId":"",
        "spaces":{"shown":"s1","sidebars":{"s1":{"items":[{"type":"separator","id":"t1","name":"Later"}]}}}}],
        "spaces":[{"id":"s1","name":"Personal"},{"id":"s2","name":"Work"}],"nextTabId":1}}"#;

    /// Random operations as the shell makes them, with every rule checked after each.
    fn walk(mut s: SessionState, seed: u64, steps: usize) {
        check_session(&s).unwrap_or_else(|e| panic!("seed {seed} at the start: {e}"));
        let spaces = s.spaces.clone();
        let mut rng = Lcg(seed);
        for step in 0..steps {
            let wi = rng.below(s.windows.len());
            let all: Vec<String> = s.windows[wi].tabs.iter().map(|t| t.id.clone()).collect();
            let shown: Vec<String> =
                s.windows[wi].spaces.tree().map(|t| t.tab_ids()).unwrap_or_default().into_iter().map(String::from).collect();
            let any = all.get(rng.below(all.len())).cloned().unwrap_or_default();
            let on_show = shown.get(rng.below(shown.len())).cloned().unwrap_or_default();
            let other = shown.get(rng.below(shown.len())).cloned().unwrap_or_default();
            let n = rng.below(8);
            let op = rng.below(11);
            match op {
                0 | 1 => {
                    let id = s.alloc_tab_id();
                    let w = &mut s.windows[wi];
                    let front = w.active_id.clone();
                    if w.add_tab(blank_tab(id.clone()), Some(&front), n).is_ok() {
                        w.show_tab(&id);
                    }
                }
                2 => {
                    s.close_tab(&any);
                }
                3 => {
                    let to = s.windows[rng.below(s.windows.len())].id.clone();
                    let _ = move_tab_in(&mut s, &any, &to, Dest::Root(n));
                }
                4 => {
                    let id = s.alloc_item_id('i');
                    let w = &mut s.windows[wi];
                    // A new split comes on show, its first half focused.
                    if w.spaces.tree_mut().make_split(&id, &on_show, &other).is_ok() {
                        w.show_tab(&on_show);
                    }
                }
                5 => {
                    s.windows[wi].show_tab(&on_show);
                }
                6 => {
                    // A split away from the tab on show keeps the focus it was left with, so a
                    // later hand-off can land on its other half.
                    let w = &mut s.windows[wi];
                    let tree = w.spaces.tree_mut();
                    if tree.split_of(&on_show).is_some_and(|sp| !sp.tabs.contains(&w.active_id)) {
                        tree.focus_tab(&on_show);
                    }
                }
                7 => {
                    let w = &mut s.windows[wi];
                    let to = &spaces[n % spaces.len()].id;
                    let mut active = w.active_id.clone();
                    if w.spaces.move_tab(&spaces, &any, to, true, &mut active).is_ok() {
                        w.show_tab(&active);
                    }
                }
                8 => {
                    archive_tabs(&mut s, &[any], step as i64);
                }
                9 => {
                    if !s.archive.is_empty() {
                        let at = rng.below(s.archive.len());
                        let id = s.archive[at].tab.id.clone();
                        let back = restore(&mut s, at);
                        assert_eq!(back.is_some(), !s.archive.iter().any(|e| e.tab.id == id), "seed {seed} step {step}");
                    }
                }
                _ => {
                    let w = &mut s.windows[wi];
                    let mut active = w.active_id.clone();
                    if w.spaces.switch(&spaces, 1 + n % spaces.len(), true, &mut active) == Ok(true) {
                        w.show_tab(&active);
                    }
                }
            }
            check_session(&s).unwrap_or_else(|e| panic!("seed {seed} step {step} op {op}: {e}"));
        }
        assert!(s.windows.iter().map(|w| w.tabs.len()).sum::<usize>() > 5, "seed {seed}: the walk built up tabs");
    }

    #[test]
    fn no_walk_of_window_operations_strands_the_tab_on_show() {
        let mut s = two_spaces();
        let mut second = s.windows[0].clone();
        second.id = "w2".into();
        second.tabs.clear();
        second.active_id.clear();
        second.mru = Mru::default();
        second.spaces.sidebars.values_mut().for_each(|t| *t = SidebarTree::default());
        s.windows.push(second);
        walk(s, 0x5eed, 5000);
    }

    #[test]
    fn walks_from_malformed_files_keep_every_rule() {
        for seed in 1..=8 {
            walk(loaded(MALFORMED), seed, 1500);
            walk(loaded(SEPARATOR_T1), seed, 1500);
        }
    }
}
