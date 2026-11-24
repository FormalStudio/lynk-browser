//! Auto-archive: which loose tabs are due under the user's rule, and the archive list that can
//! put each one back where it was. Pinned tabs, tabs in folders or splits, audible tabs and the
//! tab on show never archive. It does not track when tabs were shown; the shell passes that in.

use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::null_default;
use crate::session::{SessionState, SessionTab, TabId, window_of_tab};
use crate::sidebar_tree::{Dest, SidebarTree};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ArchiveAfter {
    #[default]
    Never,
    Hours12,
    Day1,
    Days7,
    Days30,
}

pub const ARCHIVE_AFTER: [(ArchiveAfter, &str); 5] = [
    (ArchiveAfter::Never, "Never"),
    (ArchiveAfter::Hours12, "After 12 hours"),
    (ArchiveAfter::Day1, "After a day"),
    (ArchiveAfter::Days7, "After 7 days"),
    (ArchiveAfter::Days30, "After 30 days"),
];

const HOUR_MS: i64 = 60 * 60 * 1000;

impl ArchiveAfter {
    pub fn millis(self) -> Option<i64> {
        match self {
            Self::Never => None,
            Self::Hours12 => Some(12 * HOUR_MS),
            Self::Day1 => Some(24 * HOUR_MS),
            Self::Days7 => Some(7 * 24 * HOUR_MS),
            Self::Days30 => Some(30 * 24 * HOUR_MS),
        }
    }
}

/// The oldest entries fall off past this many.
pub const ARCHIVE_LIMIT: usize = 100;

/// The loose tabs in `tree` due for the archive. A tab with no recorded last-shown time is
/// never due: its age is unknown.
pub fn due(
    tree: &SidebarTree,
    last_shown: &HashMap<String, i64>,
    audible: &HashSet<String>,
    active: &str,
    now: i64,
    rule: ArchiveAfter,
) -> Vec<String> {
    let Some(after) = rule.millis() else { return Vec::new() };
    tree.loose_tabs()
        .into_iter()
        .filter(|id| *id != active && !audible.contains(*id))
        .filter(|id| last_shown.get(*id).is_some_and(|t| now - t >= after))
        .map(str::to_owned)
        .collect()
}

/// An archived tab and where it sat: the rows above and below it in its space's list (empty at
/// either end) and its index there, for when both are gone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ArchivedTab {
    #[serde(deserialize_with = "null_default")]
    pub tab: SessionTab,
    #[serde(deserialize_with = "null_default")]
    pub window_id: String,
    #[serde(deserialize_with = "null_default")]
    pub space_id: String,
    #[serde(deserialize_with = "null_default")]
    pub after: String,
    #[serde(deserialize_with = "null_default")]
    pub before: String,
    #[serde(deserialize_with = "null_default")]
    pub index: usize,
    #[serde(deserialize_with = "null_default")]
    pub archived_at: i64,
}

/// Moves loose tabs to the archive, newest first. Ids that are not loose tabs are skipped.
/// Every tab's neighbours are read before any tab goes, so tabs archived together come back in
/// their order.
pub fn archive_tabs(state: &mut SessionState, ids: &[String], now: i64) -> usize {
    let mut entries: Vec<ArchivedTab> = Vec::new();
    for id in ids {
        let Some(w) = state.windows.iter().find(|w| w.tab(id).is_some()) else { continue };
        let Some(space) = w.spaces.space_of_tab(id) else { continue };
        let tree = &w.spaces.sidebars[space];
        if !tree.loose_tabs().contains(&id.as_str()) || entries.iter().any(|e| e.tab.id == *id) {
            continue;
        }
        let index = tree.row_index(id).unwrap_or_default();
        entries.push(ArchivedTab {
            tab: w.tab(id).cloned().unwrap_or_default(),
            window_id: w.id.clone(),
            space_id: space.to_owned(),
            after: tree.row_above(id).unwrap_or_default().to_owned(),
            before: tree.items.get(index + 1).map(|i| i.id().to_owned()).unwrap_or_default(),
            index,
            archived_at: now,
        });
    }
    let moved = entries.len();
    for entry in entries {
        state.close_tab(&entry.tab.id);
        state.archive.insert(0, entry);
    }
    state.archive.truncate(ARCHIVE_LIMIT);
    moved
}

/// Puts archive entry `at` back as a loose tab: under the row it followed, else over the row
/// that followed it, else at its old index, in its window and space when they still exist, else in the space the first window
/// shows. Returns the window it went to and the tab's id, which is new if its old one is taken.
pub fn restore(state: &mut SessionState, at: usize) -> Option<(String, TabId)> {
    if at >= state.archive.len() || state.windows.is_empty() {
        return None;
    }
    let mut entry = state.archive.remove(at);
    if entry.tab.id.is_empty() || window_of_tab(state, &entry.tab.id).is_some() {
        entry.tab.id = state.alloc_tab_id();
    }
    let space_known = state.spaces.iter().any(|s| s.id == entry.space_id);
    let w = match state.windows.iter().position(|w| w.id == entry.window_id) {
        Some(i) => &mut state.windows[i],
        None => &mut state.windows[0],
    };
    let space = if space_known { entry.space_id.clone() } else { w.spaces.shown.clone() };
    let tree = w.spaces.sidebars.get(&space);
    let index = match tree {
        _ if entry.after.is_empty() => 0,
        Some(tree) => match (tree.row_index(&entry.after), tree.row_index(&entry.before)) {
            (Some(above), _) => above + 1,
            (None, Some(below)) => below,
            (None, None) => entry.index,
        },
        None => 0,
    };
    let id = entry.tab.id.clone();
    w.insert_tab(entry.tab, &space, Dest::Root(index)).ok()?;
    Some((w.id.clone(), id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{blank_tab, normalize};
    use crate::sidebar_tree::{Child, FolderKind};

    fn state() -> SessionState {
        let mut s = normalize(&SessionState::default());
        let w = &mut s.windows[0];
        for id in ["a", "b", "c", "d", "e"] {
            w.add_tab(blank_tab(id), None, 99).unwrap();
        }
        let tree = w.spaces.tree_mut();
        tree.pin("a", 0).unwrap();
        tree.new_folder("f", FolderKind::Plain, "F", "", None, 99).unwrap();
        tree.move_row("e", crate::sidebar_tree::Dest::Folder { id: "f".into(), index: 0 }).unwrap();
        w.active_id = "t1".into();
        s
    }

    #[test]
    fn only_old_quiet_loose_tabs_are_due() {
        let s = state();
        let tree = s.windows[0].spaces.tree().unwrap();
        let now = 100 * HOUR_MS;
        let shown: HashMap<String, i64> =
            ["a", "b", "c", "d", "e", "t1"].iter().map(|id| ((*id).to_owned(), 0)).collect();
        let audible: HashSet<String> = ["c".to_owned()].into();
        assert_eq!(due(tree, &shown, &audible, "t1", now, ArchiveAfter::Hours12), ["b", "d"]);
        assert!(due(tree, &shown, &audible, "t1", now, ArchiveAfter::Never).is_empty());
        assert!(due(tree, &shown, &audible, "t1", now, ArchiveAfter::Days7).is_empty());
        assert_eq!(due(tree, &HashMap::new(), &audible, "t1", now, ArchiveAfter::Hours12), Vec::<String>::new());
        let mut recent = shown.clone();
        recent.insert("b".into(), now - HOUR_MS);
        assert_eq!(due(tree, &recent, &audible, "t1", now, ArchiveAfter::Hours12), ["d"]);
    }

    #[test]
    fn archived_tabs_come_back_where_they_were() {
        let mut s = state();
        let before = s.windows[0].spaces.tree().unwrap().items.clone();
        assert_eq!(archive_tabs(&mut s, &["c".into(), "a".into(), "e".into()], 7), 1);
        assert_eq!(s.archive[0].after, "b");
        assert!(!s.windows[0].tabs.iter().any(|t| t.id == "c"));
        assert_eq!(restore(&mut s, 0), Some(("w1".into(), "c".into())));
        assert_eq!(s.windows[0].spaces.tree().unwrap().items, before);
        assert!(s.archive.is_empty());
        assert_eq!(normalize(&s), s);

        let rows = |s: &SessionState| -> Vec<String> {
            s.windows[0].spaces.tree().unwrap().items.iter().map(|i| i.id().to_owned()).collect()
        };
        // The row it followed is gone: it goes back over the row that followed it.
        archive_tabs(&mut s, &["c".into()], 8);
        s.close_tab("b");
        restore(&mut s, 0).unwrap();
        assert_eq!(rows(&s), ["t1", "c", "d", "f"]);
        // Both are gone: back to its index.
        archive_tabs(&mut s, &["c".into()], 9);
        s.close_tab("t1");
        s.close_tab("d");
        restore(&mut s, 0).unwrap();
        assert_eq!(rows(&s), ["f", "c"]);
        assert!(restore(&mut s, 0).is_none());
    }

    #[test]
    fn a_restored_tab_whose_id_was_reused_gets_a_new_one() {
        let mut s = state();
        archive_tabs(&mut s, &["d".into()], 1);
        s.windows[0].add_tab(blank_tab("d"), None, 0).unwrap();
        let (_, id) = restore(&mut s, 0).unwrap();
        assert_ne!(id, "d");
        assert_eq!(s.windows[0].spaces.tree().unwrap().check(), Ok(()));
        assert_eq!(s.windows[0].spaces.tree().unwrap().folder("f").unwrap().children, vec![Child::Tab { id: "e".into() }]);
    }

    #[test]
    fn the_archive_keeps_the_newest_entries() {
        let mut s = state();
        s.archive = vec![ArchivedTab::default(); ARCHIVE_LIMIT];
        archive_tabs(&mut s, &["b".into()], 9);
        assert_eq!(s.archive.len(), ARCHIVE_LIMIT);
        assert_eq!(s.archive[0].tab.id, "b");
    }

    #[test]
    fn tabs_archived_together_come_back_in_their_order() {
        let mut s = normalize(&SessionState::default());
        let first = s.windows[0].active_id.clone();
        for id in ["x", "a", "b", "y"] {
            s.windows[0].add_tab(blank_tab(id), None, 99).unwrap();
        }
        archive_tabs(&mut s, &["a".into(), "b".into()], 0);
        let at = |s: &SessionState, id: &str| s.archive.iter().position(|e| e.tab.id == id).unwrap();
        let i = at(&s, "a");
        restore(&mut s, i).unwrap();
        let i = at(&s, "b");
        restore(&mut s, i).unwrap();
        assert_eq!(s.windows[0].spaces.tab_ids(), [first.as_str(), "x", "a", "b", "y"]);
        archive_tabs(&mut s, &["a".into(), "b".into()], 0);
        let i = at(&s, "b");
        restore(&mut s, i).unwrap();
        let i = at(&s, "a");
        restore(&mut s, i).unwrap();
        assert_eq!(s.windows[0].spaces.tab_ids(), [first.as_str(), "x", "a", "b", "y"]);
    }
}
