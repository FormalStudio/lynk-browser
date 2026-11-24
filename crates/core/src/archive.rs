//! Auto-archive: loose tabs not shown for a while leave the sidebar for the archive, which can
//! put each one back where it was. Pinned tabs, tabs in folders or splits, audible tabs and the
//! tab on show never archive.

use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::null_default;
use crate::session::{SessionState, SessionTab};
use crate::sidebar_tree::{Item, SidebarTree};

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

/// An archived tab and where it sat: the row it followed in its space's list (empty at the
/// top) and its index there, for when that row is gone.
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
    pub index: usize,
    #[serde(deserialize_with = "null_default")]
    pub archived_at: i64,
}

/// Moves loose tabs to the archive, newest first. Ids that are not loose tabs are skipped.
pub fn archive_tabs(state: &mut SessionState, ids: &[String], now: i64) -> usize {
    let mut moved = 0;
    for id in ids {
        let Some(w) = state.windows.iter().find(|w| w.tabs.iter().any(|t| t.id == *id)) else { continue };
        let Some(space) = w.spaces.space_of_tab(id) else { continue };
        let tree = &w.spaces.sidebars[space];
        let Some(index) = tree.items.iter().position(|i| matches!(i, Item::Tab { id: t } if t == id)) else {
            continue;
        };
        let entry = ArchivedTab {
            tab: w.tabs.iter().find(|t| t.id == *id).cloned().unwrap_or_default(),
            window_id: w.id.clone(),
            space_id: space.to_owned(),
            after: index.checked_sub(1).map(|i| tree.items[i].id().to_owned()).unwrap_or_default(),
            index,
            archived_at: now,
        };
        state.close_tab(id);
        state.archive.insert(0, entry);
        moved += 1;
    }
    state.archive.truncate(ARCHIVE_LIMIT);
    moved
}

/// Puts archive entry `at` back as a loose tab: under the row it followed, else at its old
/// index, in its window and space when they still exist, else in the space the first window
/// shows. Returns the window it went to and the tab's id, which is new if its old one is taken.
pub fn restore(state: &mut SessionState, at: usize) -> Option<(String, String)> {
    if at >= state.archive.len() || state.windows.is_empty() {
        return None;
    }
    let mut entry = state.archive.remove(at);
    let taken = state.windows.iter().any(|w| w.tabs.iter().any(|t| t.id == entry.tab.id));
    if taken || entry.tab.id.is_empty() {
        entry.tab.id = state.alloc_tab_id();
    }
    let spaces_known: HashSet<String> = state.spaces.iter().map(|s| s.id.clone()).collect();
    let w = match state.windows.iter().position(|w| w.id == entry.window_id) {
        Some(i) => &mut state.windows[i],
        None => &mut state.windows[0],
    };
    let tree = if spaces_known.contains(&entry.space_id) {
        w.spaces.sidebars.entry(entry.space_id.clone()).or_default()
    } else {
        w.spaces.tree_mut()
    };
    let index = if entry.after.is_empty() {
        0
    } else {
        tree.items.iter().position(|i| i.id() == entry.after).map_or(entry.index, |i| i + 1)
    };
    let id = entry.tab.id.clone();
    entry.tab.pinned = false;
    tree.items.insert(index.min(tree.items.len()), Item::tab(id.clone()));
    w.tabs.push(entry.tab);
    w.sync_tabs();
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
            assert!(w.add_tab(blank_tab(id), None, 99));
        }
        let tree = w.spaces.tree_mut();
        tree.pin("a", 0).unwrap();
        tree.new_folder("f", FolderKind::Plain, "F", "", None, 99).unwrap();
        tree.move_row("e", crate::sidebar_tree::Dest::Folder { id: "f".into(), index: 0 }).unwrap();
        w.sync_tabs();
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

        // The row it followed is gone: it goes back to its index.
        archive_tabs(&mut s, &["c".into()], 8);
        s.close_tab("b");
        restore(&mut s, 0).unwrap();
        let rows: Vec<&str> = s.windows[0].spaces.tree().unwrap().items.iter().map(Item::id).collect();
        assert_eq!(rows, ["t1", "d", "c", "f"]);
        assert!(restore(&mut s, 0).is_none());
    }

    #[test]
    fn a_restored_tab_whose_id_was_reused_gets_a_new_one() {
        let mut s = state();
        archive_tabs(&mut s, &["d".into()], 1);
        assert!(s.windows[0].add_tab(blank_tab("d"), None, 0));
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
}
