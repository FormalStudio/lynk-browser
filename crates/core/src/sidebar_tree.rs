//! One space's sidebar: pinned tiles, then an ordered list of rows. A row is a tab, a named
//! separator, a folder or a split. Folders hold tabs and splits, never another folder or a
//! separator, and a split always holds exactly two tabs. Every operation keeps those rules and
//! keeps each tab id in the tree once.
//!
//! Rows are addressed by id: tab ids and the ids of separators, folders and splits share one
//! namespace, which the session hands out.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{lenient_items, null_default};

/// Which half of a split has focus: the first is the left card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Half {
    #[default]
    First,
    Second,
}

impl Half {
    fn index(self) -> usize {
        match self {
            Self::First => 0,
            Self::Second => 1,
        }
    }
}

pub const SPLIT_RATIO_MIN: f64 = 0.2;
pub const SPLIT_RATIO_MAX: f64 = 0.8;

/// Two tabs shown side by side. `ratio` is the first card's share of the width.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Split {
    pub id: String,
    pub tabs: [String; 2],
    pub ratio: f64,
    #[serde(default, deserialize_with = "null_default")]
    pub focus: Half,
}

impl Split {
    pub fn focused(&self) -> &str {
        &self.tabs[self.focus.index()]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderKind {
    #[default]
    Plain,
    /// A working set: a tab opened while one of its tabs is in front joins it.
    Project,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    #[serde(default, deserialize_with = "null_default")]
    pub kind: FolderKind,
    #[serde(default, deserialize_with = "null_default")]
    pub name: String,
    /// An icon name from the shell's icon set; empty draws the plain folder glyph.
    #[serde(default, deserialize_with = "null_default")]
    pub icon: String,
    /// Hue in degrees for the card's fill; `None` takes the space's own colour.
    #[serde(default)]
    pub tint: Option<u16>,
    #[serde(default, deserialize_with = "null_default")]
    pub collapsed: bool,
    #[serde(default, deserialize_with = "lenient_items")]
    pub children: Vec<Child>,
}

/// What a folder holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Child {
    Tab { id: String },
    Split(Split),
}

/// One row of the list under the pins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Item {
    Tab {
        id: String,
    },
    Separator {
        id: String,
        #[serde(default, deserialize_with = "null_default")]
        name: String,
    },
    Folder(Folder),
    Split(Split),
}

impl Item {
    pub fn id(&self) -> &str {
        match self {
            Self::Tab { id } | Self::Separator { id, .. } => id,
            Self::Folder(f) => &f.id,
            Self::Split(s) => &s.id,
        }
    }

    pub fn tab(id: impl Into<String>) -> Self {
        Self::Tab { id: id.into() }
    }
}

impl Child {
    pub fn id(&self) -> &str {
        match self {
            Self::Tab { id } => id,
            Self::Split(s) => &s.id,
        }
    }

    fn into_item(self) -> Item {
        match self {
            Self::Tab { id } => Item::Tab { id },
            Self::Split(s) => Item::Split(s),
        }
    }

    fn tabs(&self) -> Vec<&str> {
        match self {
            Self::Tab { id } => vec![id],
            Self::Split(s) => s.tabs.iter().map(String::as_str).collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TreeError {
    UnknownRow,
    /// The operation takes a tab and was given another kind of row.
    NotATab,
    /// A separator or a folder can only sit in the list itself.
    RootOnly,
    /// A pinned tab cannot join a split or a folder.
    Pinned,
    /// A tab already in a split cannot start another one.
    InSplit,
    SameTab,
    AlreadyPresent,
    /// The tab sits inside a folder or split where the operation does not apply.
    NotLoose,
    /// A split ratio outside [`SPLIT_RATIO_MIN`] to [`SPLIT_RATIO_MAX`].
    BadRatio,
}

/// Where a row goes. The index counts the destination as it is once the row has left its old
/// place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dest {
    Pins(usize),
    Root(usize),
    Folder { id: String, index: usize },
}

/// A collapsed folder's row: up to three tabs whose favicons it shows, and how many tabs it
/// holds in all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderSummary {
    pub favicons: Vec<String>,
    pub count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Container {
    Pins,
    Root,
    /// Holds the folder's index in the root list.
    Folder(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TabLoc {
    container: Container,
    index: usize,
    half: Option<Half>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SidebarTree {
    #[serde(deserialize_with = "null_default")]
    pub pins: Vec<String>,
    #[serde(deserialize_with = "lenient_items")]
    pub items: Vec<Item>,
    /// The tab this sidebar showed last, which switching back to its space shows again.
    #[serde(deserialize_with = "null_default")]
    pub last_active: String,
}

impl SidebarTree {
    /// Every tab in drawing order: pins, then the list with folders and splits opened up.
    pub fn tab_ids(&self) -> Vec<&str> {
        let mut out: Vec<&str> = self.pins.iter().map(String::as_str).collect();
        for item in &self.items {
            match item {
                Item::Tab { id } => out.push(id),
                Item::Separator { .. } => {}
                Item::Split(s) => out.extend(s.tabs.iter().map(String::as_str)),
                Item::Folder(f) => out.extend(f.children.iter().flat_map(Child::tabs)),
            }
        }
        out
    }

    pub fn contains_tab(&self, id: &str) -> bool {
        self.locate(id).is_some()
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.pins.iter().any(|p| p == id)
    }

    /// Tabs sitting directly in the list: not pinned, not in a folder, not half of a split.
    pub fn loose_tabs(&self) -> Vec<&str> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::Tab { id } => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }

    pub fn folder(&self, id: &str) -> Option<&Folder> {
        self.items.iter().find_map(|i| match i {
            Item::Folder(f) if f.id == id => Some(f),
            _ => None,
        })
    }

    fn folder_mut(&mut self, id: &str) -> Option<&mut Folder> {
        self.items.iter_mut().find_map(|i| match i {
            Item::Folder(f) if f.id == id => Some(f),
            _ => None,
        })
    }

    /// The folder holding a tab, directly or as half of a split.
    pub fn folder_of(&self, tab: &str) -> Option<&Folder> {
        match self.locate(tab)?.container {
            Container::Folder(at) => match &self.items[at] {
                Item::Folder(f) => Some(f),
                _ => None,
            },
            _ => None,
        }
    }

    /// The split holding a tab, wherever it sits.
    pub fn split_of(&self, tab: &str) -> Option<&Split> {
        let loc = self.locate(tab)?;
        loc.half?;
        match (loc.container, &self.items) {
            (Container::Root, items) => match &items[loc.index] {
                Item::Split(s) => Some(s),
                _ => None,
            },
            (Container::Folder(at), items) => match &items[at] {
                Item::Folder(f) => match &f.children[loc.index] {
                    Child::Split(s) => Some(s),
                    Child::Tab { .. } => None,
                },
                _ => None,
            },
            (Container::Pins, _) => None,
        }
    }

    /// Fails on the first broken rule: a tab or row id seen twice, a split whose two halves are
    /// one tab, a ratio outside the allowed range.
    pub fn check(&self) -> Result<(), TreeError> {
        let mut seen = HashSet::new();
        for id in self.tab_ids() {
            if !seen.insert(id) {
                return Err(TreeError::AlreadyPresent);
            }
        }
        let mut splits = Vec::new();
        for item in &self.items {
            if !matches!(item, Item::Tab { .. }) && !seen.insert(item.id()) {
                return Err(TreeError::AlreadyPresent);
            }
            match item {
                Item::Split(s) => splits.push(s),
                Item::Folder(f) => {
                    for c in &f.children {
                        if let Child::Split(s) = c {
                            if !seen.insert(&s.id) {
                                return Err(TreeError::AlreadyPresent);
                            }
                            splits.push(s);
                        }
                    }
                }
                _ => {}
            }
        }
        for s in splits {
            if s.tabs[0] == s.tabs[1] {
                return Err(TreeError::SameTab);
            }
            if !(SPLIT_RATIO_MIN..=SPLIT_RATIO_MAX).contains(&s.ratio) {
                return Err(TreeError::BadRatio);
            }
        }
        Ok(())
    }

    /// Brings a tree read from disk back within the rules: tabs `keep` rejects and repeats go,
    /// a split left with one tab becomes that tab, a ratio out of range is clamped.
    pub fn repair(&mut self, keep: impl Fn(&str) -> bool) {
        let mut seen: HashSet<String> = HashSet::new();
        let mut take = |id: &str| keep(id) && seen.insert(id.to_owned());
        self.pins.retain(|id| take(id));
        let mut rows = HashSet::new();
        let mut items = Vec::new();
        for item in std::mem::take(&mut self.items) {
            match item {
                Item::Tab { id } => {
                    if take(&id) {
                        items.push(Item::Tab { id });
                    }
                }
                Item::Separator { id, name } => {
                    if rows.insert(id.clone()) {
                        items.push(Item::Separator { id, name });
                    }
                }
                Item::Split(s) => {
                    items.extend(repair_split(s, &mut take, &mut rows).into_iter().map(Child::into_item));
                }
                Item::Folder(mut f) => {
                    if !rows.insert(f.id.clone()) {
                        continue;
                    }
                    let mut children = Vec::new();
                    for child in std::mem::take(&mut f.children) {
                        match child {
                            Child::Tab { id } => {
                                if take(&id) {
                                    children.push(Child::Tab { id });
                                }
                            }
                            Child::Split(s) => children.extend(repair_split(s, &mut take, &mut rows)),
                        }
                    }
                    f.children = children;
                    items.push(Item::Folder(f));
                }
            }
        }
        self.items = items;
    }

    fn locate(&self, tab: &str) -> Option<TabLoc> {
        if let Some(index) = self.pins.iter().position(|p| p == tab) {
            return Some(TabLoc { container: Container::Pins, index, half: None });
        }
        for (at, item) in self.items.iter().enumerate() {
            match item {
                Item::Tab { id } if id == tab => {
                    return Some(TabLoc { container: Container::Root, index: at, half: None });
                }
                Item::Split(s) => {
                    if let Some(h) = half_of(s, tab) {
                        return Some(TabLoc { container: Container::Root, index: at, half: Some(h) });
                    }
                }
                Item::Folder(f) => {
                    for (index, c) in f.children.iter().enumerate() {
                        let half = match c {
                            Child::Tab { id } if id == tab => None,
                            Child::Split(s) => match half_of(s, tab) {
                                Some(h) => Some(h),
                                None => continue,
                            },
                            Child::Tab { .. } => continue,
                        };
                        return Some(TabLoc { container: Container::Folder(at), index, half });
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Where a row that is not a tab sits: `Root` with its index, or the folder it is in.
    fn locate_row(&self, id: &str) -> Option<(Container, usize)> {
        for (at, item) in self.items.iter().enumerate() {
            if item.id() == id && !matches!(item, Item::Tab { .. }) {
                return Some((Container::Root, at));
            }
            if let Item::Folder(f) = item
                && let Some(index) = f.children.iter().position(|c| matches!(c, Child::Split(s) if s.id == id))
            {
                return Some((Container::Folder(at), index));
            }
        }
        None
    }

    /// Takes a tab out wherever it is. Out of a split, the other half stays where the split
    /// was as a plain tab.
    fn take_tab(&mut self, tab: &str) -> Option<TabLoc> {
        let loc = self.locate(tab)?;
        let remainder = |s: &Split, h: Half| s.tabs[1 - h.index()].clone();
        match (loc.container, loc.half) {
            (Container::Pins, _) => {
                self.pins.remove(loc.index);
            }
            (Container::Root, None) => {
                self.items.remove(loc.index);
            }
            (Container::Root, Some(h)) => {
                let Item::Split(s) = &self.items[loc.index] else { unreachable!() };
                self.items[loc.index] = Item::Tab { id: remainder(s, h) };
            }
            (Container::Folder(at), half) => {
                let Item::Folder(f) = &mut self.items[at] else { unreachable!() };
                match half {
                    None => {
                        f.children.remove(loc.index);
                    }
                    Some(h) => {
                        let Child::Split(s) = &f.children[loc.index] else { unreachable!() };
                        f.children[loc.index] = Child::Tab { id: remainder(s, h) };
                    }
                }
            }
        }
        Some(loc)
    }

    /// Takes out a row that is not a tab: a separator, folder or split.
    fn take_row(&mut self, id: &str) -> Option<Item> {
        match self.locate_row(id)? {
            (Container::Root, at) => Some(self.items.remove(at)),
            (Container::Folder(at), index) => {
                let Item::Folder(f) = &mut self.items[at] else { unreachable!() };
                Some(f.children.remove(index).into_item())
            }
            (Container::Pins, _) => None,
        }
    }

    fn insert(&mut self, item: Item, dest: &Dest) -> Result<(), TreeError> {
        match dest {
            Dest::Pins(index) => {
                let Item::Tab { id } = item else { return Err(TreeError::NotATab) };
                let at = (*index).min(self.pins.len());
                self.pins.insert(at, id);
            }
            Dest::Root(index) => {
                let at = (*index).min(self.items.len());
                self.items.insert(at, item);
            }
            Dest::Folder { id, index } => {
                let child = match item {
                    Item::Tab { id } => Child::Tab { id },
                    Item::Split(s) => Child::Split(s),
                    _ => return Err(TreeError::RootOnly),
                };
                let f = self.folder_mut(id).ok_or(TreeError::UnknownRow)?;
                let at = (*index).min(f.children.len());
                f.children.insert(at, child);
            }
        }
        Ok(())
    }

    /// Checks that `row` may go to `dest` before anything moves.
    fn allowed(&self, row: &str, dest: &Dest) -> Result<(), TreeError> {
        let is_tab = self.contains_tab(row);
        if !is_tab && self.locate_row(row).is_none() {
            return Err(TreeError::UnknownRow);
        }
        match dest {
            Dest::Pins(_) if !is_tab => Err(TreeError::NotATab),
            Dest::Folder { id, .. } => {
                if self.folder(id).is_none() {
                    return Err(TreeError::UnknownRow);
                }
                if is_tab {
                    return Ok(());
                }
                match self.items.iter().find(|i| i.id() == row) {
                    Some(Item::Folder(_) | Item::Separator { .. }) => Err(TreeError::RootOnly),
                    _ => Ok(()),
                }
            }
            _ => Ok(()),
        }
    }

    /// Moves a row: reorder within the list (across separators too), into or out of a folder,
    /// to the pins (drop to pin) or out of them. A tab moved out of a split leaves its other
    /// half in place as a plain tab; a split id moves the pair.
    pub fn move_row(&mut self, row: &str, dest: Dest) -> Result<(), TreeError> {
        self.allowed(row, &dest)?;
        let item = if self.contains_tab(row) {
            self.take_tab(row);
            Item::tab(row)
        } else {
            self.take_row(row).ok_or(TreeError::UnknownRow)?
        };
        self.insert(item, &dest)
    }

    pub fn pin(&mut self, tab: &str, index: usize) -> Result<(), TreeError> {
        self.move_row(tab, Dest::Pins(index))
    }

    pub fn unpin(&mut self, tab: &str, root_index: usize) -> Result<(), TreeError> {
        if !self.is_pinned(tab) {
            return Err(TreeError::UnknownRow);
        }
        self.move_row(tab, Dest::Root(root_index))
    }

    /// The list's row holding `tab`: its own row, its split's, or its folder's.
    fn root_index_of(&self, tab: &str) -> Option<usize> {
        let loc = self.locate(tab)?;
        match loc.container {
            Container::Pins => None,
            Container::Root => Some(loc.index),
            Container::Folder(at) => Some(at),
        }
    }

    /// "Add separator below" on a tab: a separator right under the tab's row in the list, or
    /// under its folder when the tab is inside one.
    pub fn add_separator_below(&mut self, tab: &str, id: &str, name: &str) -> Result<(), TreeError> {
        let at = self.root_index_of(tab).ok_or(TreeError::NotLoose)?;
        self.ensure_new_row(id)?;
        self.items.insert(at + 1, Item::Separator { id: id.into(), name: name.trim().into() });
        Ok(())
    }

    fn ensure_new_row(&self, id: &str) -> Result<(), TreeError> {
        if self.contains_tab(id) || self.locate_row(id).is_some() {
            Err(TreeError::AlreadyPresent)
        } else {
            Ok(())
        }
    }

    /// Renames a separator or a folder.
    pub fn rename(&mut self, id: &str, name: &str) -> Result<(), TreeError> {
        let name = name.trim().to_owned();
        for item in &mut self.items {
            match item {
                Item::Separator { id: sid, name: n } if sid == id => {
                    *n = name;
                    return Ok(());
                }
                Item::Folder(f) if f.id == id => {
                    f.name = name;
                    return Ok(());
                }
                _ => {}
            }
        }
        Err(TreeError::UnknownRow)
    }

    /// Removes a separator; the tabs around it stay where they are.
    pub fn delete_separator(&mut self, id: &str) -> Result<(), TreeError> {
        let at = self
            .items
            .iter()
            .position(|i| matches!(i, Item::Separator { id: s, .. } if s == id))
            .ok_or(TreeError::UnknownRow)?;
        self.items.remove(at);
        Ok(())
    }

    /// "Move to root" on a tab inside a folder: its row (the split, when it is half of one)
    /// goes into the list directly under the folder.
    pub fn move_to_root(&mut self, tab: &str) -> Result<(), TreeError> {
        let loc = self.locate(tab).ok_or(TreeError::UnknownRow)?;
        let Container::Folder(at) = loc.container else { return Err(TreeError::NotLoose) };
        let Item::Folder(f) = &mut self.items[at] else { unreachable!() };
        let row = f.children.remove(loc.index).into_item();
        self.items.insert(at + 1, row);
        Ok(())
    }

    pub fn new_folder(
        &mut self,
        id: &str,
        kind: FolderKind,
        name: &str,
        icon: &str,
        tint: Option<u16>,
        root_index: usize,
    ) -> Result<(), TreeError> {
        self.ensure_new_row(id)?;
        let folder = Folder {
            id: id.into(),
            kind,
            name: name.trim().into(),
            icon: icon.into(),
            tint,
            collapsed: false,
            children: Vec::new(),
        };
        self.items.insert(root_index.min(self.items.len()), Item::Folder(folder));
        Ok(())
    }

    pub fn set_collapsed(&mut self, folder: &str, collapsed: bool) -> Result<(), TreeError> {
        self.folder_mut(folder).ok_or(TreeError::UnknownRow)?.collapsed = collapsed;
        Ok(())
    }

    /// Puts `b` beside `a` as one split, where `a` was. `a` has focus at an even ratio.
    pub fn make_split(&mut self, id: &str, a: &str, b: &str) -> Result<(), TreeError> {
        if a == b {
            return Err(TreeError::SameTab);
        }
        self.ensure_new_row(id)?;
        for tab in [a, b] {
            let loc = self.locate(tab).ok_or(TreeError::UnknownRow)?;
            if loc.container == Container::Pins {
                return Err(TreeError::Pinned);
            }
            if loc.half.is_some() {
                return Err(TreeError::InSplit);
            }
        }
        self.take_tab(b);
        let loc = self.locate(a).expect("a is still in the tree");
        let split = Split { id: id.into(), tabs: [a.into(), b.into()], ratio: 0.5, focus: Half::First };
        match loc.container {
            Container::Root => self.items[loc.index] = Item::Split(split),
            Container::Folder(at) => {
                let Item::Folder(f) = &mut self.items[at] else { unreachable!() };
                f.children[loc.index] = Child::Split(split);
            }
            Container::Pins => unreachable!(),
        }
        Ok(())
    }

    /// Breaks a split into its two tabs, in order, where it was.
    pub fn dissolve_split(&mut self, id: &str) -> Result<(), TreeError> {
        let (container, index) = self.locate_row(id).ok_or(TreeError::UnknownRow)?;
        match container {
            Container::Root => {
                let Item::Split(s) = self.items.remove(index) else { return Err(TreeError::UnknownRow) };
                let [x, y] = s.tabs;
                self.items.splice(index..index, [Item::tab(x), Item::tab(y)]);
            }
            Container::Folder(at) => {
                let Item::Folder(f) = &mut self.items[at] else { unreachable!() };
                let Child::Split(s) = f.children.remove(index) else { unreachable!() };
                let [x, y] = s.tabs;
                f.children.splice(index..index, [Child::Tab { id: x }, Child::Tab { id: y }]);
            }
            Container::Pins => unreachable!(),
        }
        Ok(())
    }

    pub fn set_split_ratio(&mut self, id: &str, ratio: f64) -> Result<(), TreeError> {
        let ratio = if ratio.is_finite() { ratio.clamp(SPLIT_RATIO_MIN, SPLIT_RATIO_MAX) } else { 0.5 };
        self.split_mut(id).ok_or(TreeError::UnknownRow)?.ratio = ratio;
        Ok(())
    }

    fn split_mut(&mut self, id: &str) -> Option<&mut Split> {
        let (container, index) = self.locate_row(id)?;
        match container {
            Container::Root => match &mut self.items[index] {
                Item::Split(s) => Some(s),
                _ => None,
            },
            Container::Folder(at) => match &mut self.items[at] {
                Item::Folder(f) => match &mut f.children[index] {
                    Child::Split(s) => Some(s),
                    Child::Tab { .. } => None,
                },
                _ => None,
            },
            Container::Pins => None,
        }
    }

    /// Showing a tab that is half of a split gives that half focus.
    pub fn focus_tab(&mut self, tab: &str) {
        let Some(id) = self.split_of(tab).map(|s| s.id.clone()) else { return };
        let s = self.split_mut(&id).expect("split located");
        s.focus = if s.tabs[0] == tab { Half::First } else { Half::Second };
    }

    /// Closes a tab. Closing half of a split leaves the other half as a plain tab in place.
    pub fn close_tab(&mut self, tab: &str) -> bool {
        let closed = self.take_tab(tab).is_some();
        if closed && self.last_active == tab {
            self.last_active.clear();
        }
        closed
    }

    /// A new tab. With a project's tab in front it joins that project right after the front
    /// tab's row; otherwise it goes into the list at `root_index`.
    pub fn add_tab(&mut self, id: &str, front: Option<&str>, root_index: usize) -> Result<(), TreeError> {
        self.ensure_new_row(id)?;
        if let Some(front) = front
            && let Some(loc) = self.locate(front)
            && let Container::Folder(at) = loc.container
            && let Item::Folder(f) = &mut self.items[at]
            && f.kind == FolderKind::Project
        {
            f.children.insert(loc.index + 1, Child::Tab { id: id.into() });
            return Ok(());
        }
        self.items.insert(root_index.min(self.items.len()), Item::tab(id));
        Ok(())
    }

    /// A peek promoted to a tab lands directly under the tab it came from: after that tab's
    /// row in its folder or in the list. Under a pinned tab, it leads the list.
    pub fn add_promoted_peek(&mut self, id: &str, parent: &str) -> Result<(), TreeError> {
        self.ensure_new_row(id)?;
        let loc = self.locate(parent).ok_or(TreeError::UnknownRow)?;
        match loc.container {
            Container::Pins => self.items.insert(0, Item::tab(id)),
            Container::Root => self.items.insert(loc.index + 1, Item::tab(id)),
            Container::Folder(at) => {
                let Item::Folder(f) = &mut self.items[at] else { unreachable!() };
                f.children.insert(loc.index + 1, Child::Tab { id: id.into() });
            }
        }
        Ok(())
    }

    /// What a collapsed folder's row shows.
    pub fn folder_summary(&self, id: &str) -> Option<FolderSummary> {
        let f = self.folder(id)?;
        let tabs: Vec<&str> = f.children.iter().flat_map(Child::tabs).collect();
        Some(FolderSummary { favicons: tabs.iter().take(3).map(|t| (*t).to_owned()).collect(), count: tabs.len() })
    }
}

fn half_of(s: &Split, tab: &str) -> Option<Half> {
    if s.tabs[0] == tab {
        Some(Half::First)
    } else if s.tabs[1] == tab {
        Some(Half::Second)
    } else {
        None
    }
}

fn repair_split(mut s: Split, take: &mut impl FnMut(&str) -> bool, rows: &mut HashSet<String>) -> Vec<Child> {
    let kept: Vec<String> = s.tabs.iter().filter(|t| take(t)).cloned().collect();
    if kept.len() == 2 && rows.insert(s.id.clone()) {
        s.ratio = if s.ratio.is_finite() { s.ratio.clamp(SPLIT_RATIO_MIN, SPLIT_RATIO_MAX) } else { 0.5 };
        return vec![Child::Split(s)];
    }
    // One tab left, or a repeated split id: the tabs stay as plain rows.
    kept.into_iter().map(|id| Child::Tab { id }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(pins: &[&str], items: Vec<Item>) -> SidebarTree {
        SidebarTree { pins: pins.iter().map(|p| (*p).into()).collect(), items, last_active: String::new() }
    }

    fn tab(id: &str) -> Item {
        Item::tab(id)
    }

    fn sep(id: &str, name: &str) -> Item {
        Item::Separator { id: id.into(), name: name.into() }
    }

    fn folder(id: &str, kind: FolderKind, children: Vec<Child>) -> Item {
        Item::Folder(Folder {
            id: id.into(),
            kind,
            name: id.into(),
            icon: String::new(),
            tint: None,
            collapsed: false,
            children,
        })
    }

    fn ctab(id: &str) -> Child {
        Child::Tab { id: id.into() }
    }

    fn split(id: &str, a: &str, b: &str) -> Split {
        Split { id: id.into(), tabs: [a.into(), b.into()], ratio: 0.5, focus: Half::First }
    }

    fn rows(t: &SidebarTree) -> Vec<&str> {
        t.items.iter().map(Item::id).collect()
    }

    fn sample() -> SidebarTree {
        tree(
            &["p1", "p2"],
            vec![
                tab("a"),
                tab("b"),
                sep("s1", "Work"),
                tab("c"),
                folder("f1", FolderKind::Plain, vec![ctab("d"), Child::Split(split("x1", "e", "f"))]),
                folder("f2", FolderKind::Project, vec![ctab("g")]),
            ],
        )
    }

    #[test]
    fn tab_ids_read_in_drawing_order_and_the_sample_is_valid() {
        let t = sample();
        assert_eq!(t.tab_ids(), ["p1", "p2", "a", "b", "c", "d", "e", "f", "g"]);
        assert_eq!(t.loose_tabs(), ["a", "b", "c"]);
        assert_eq!(t.check(), Ok(()));
    }

    #[test]
    fn rows_reorder_across_separators() {
        let mut t = sample();
        t.move_row("a", Dest::Root(2)).unwrap();
        assert_eq!(rows(&t), ["b", "s1", "a", "c", "f1", "f2"]);
        t.move_row("s1", Dest::Root(0)).unwrap();
        assert_eq!(rows(&t), ["s1", "b", "a", "c", "f1", "f2"]);
        assert_eq!(t.move_row("s1", Dest::Folder { id: "f1".into(), index: 0 }), Err(TreeError::RootOnly));
        assert_eq!(t.move_row("f2", Dest::Folder { id: "f1".into(), index: 0 }), Err(TreeError::RootOnly));
        assert_eq!(t.move_row("f2", Dest::Pins(0)), Err(TreeError::NotATab));
        assert_eq!(t.move_row("nope", Dest::Root(0)), Err(TreeError::UnknownRow));
        assert_eq!(t.check(), Ok(()));
    }

    #[test]
    fn drop_to_pin_and_back() {
        let mut t = sample();
        t.pin("c", 1).unwrap();
        assert_eq!(t.pins, ["p1", "c", "p2"]);
        assert!(!t.loose_tabs().contains(&"c"));
        t.unpin("p1", 0).unwrap();
        assert_eq!(t.pins, ["c", "p2"]);
        assert_eq!(rows(&t)[0], "p1");
        assert_eq!(t.unpin("a", 0), Err(TreeError::UnknownRow));
        // Pinning half of a split leaves the other half as a tab in the folder.
        t.pin("e", 9).unwrap();
        assert_eq!(t.pins, ["c", "p2", "e"]);
        assert_eq!(t.folder("f1").unwrap().children, vec![ctab("d"), ctab("f")]);
        assert_eq!(t.check(), Ok(()));
    }

    #[test]
    fn tabs_move_in_and_out_of_folders_and_splits_move_whole() {
        let mut t = sample();
        t.move_row("a", Dest::Folder { id: "f2".into(), index: 0 }).unwrap();
        assert_eq!(t.folder("f2").unwrap().children, vec![ctab("a"), ctab("g")]);
        t.move_row("x1", Dest::Root(0)).unwrap();
        assert!(matches!(&t.items[0], Item::Split(s) if s.id == "x1"));
        assert_eq!(t.folder("f1").unwrap().children, vec![ctab("d")]);
        t.move_row("x1", Dest::Folder { id: "f2".into(), index: 5 }).unwrap();
        assert_eq!(t.split_of("e").unwrap().id, "x1");
        assert_eq!(t.folder_of("f").unwrap().id, "f2");
        assert_eq!(t.check(), Ok(()));
    }

    #[test]
    fn separators_are_added_below_renamed_and_deleted_without_losing_tabs() {
        let mut t = sample();
        t.add_separator_below("a", "s2", "  Hey ").unwrap();
        assert_eq!(rows(&t), ["a", "s2", "b", "s1", "c", "f1", "f2"]);
        assert_eq!(t.items[1], sep("s2", "Hey"));
        t.add_separator_below("d", "s3", "").unwrap();
        assert_eq!(rows(&t), ["a", "s2", "b", "s1", "c", "f1", "s3", "f2"]);
        assert_eq!(t.add_separator_below("p1", "s4", ""), Err(TreeError::NotLoose));
        assert_eq!(t.add_separator_below("a", "b", ""), Err(TreeError::AlreadyPresent));
        t.rename("s1", "Later").unwrap();
        t.rename("f1", "Daily").unwrap();
        assert_eq!(t.folder("f1").unwrap().name, "Daily");
        assert_eq!(t.rename("a", "x"), Err(TreeError::UnknownRow));
        let before = t.tab_ids().len();
        t.delete_separator("s1").unwrap();
        assert_eq!(t.tab_ids().len(), before);
        assert_eq!(t.delete_separator("f1"), Err(TreeError::UnknownRow));
    }

    #[test]
    fn move_to_root_lands_under_the_folder() {
        let mut t = sample();
        t.move_to_root("d").unwrap();
        assert_eq!(rows(&t), ["a", "b", "s1", "c", "f1", "d", "f2"]);
        t.move_to_root("f").unwrap();
        assert_eq!(rows(&t), ["a", "b", "s1", "c", "f1", "x1", "d", "f2"]);
        assert_eq!(t.move_to_root("a"), Err(TreeError::NotLoose));
    }

    #[test]
    fn splits_are_made_and_dissolved_in_place() {
        let mut t = sample();
        t.make_split("x2", "b", "a").unwrap();
        assert_eq!(rows(&t), ["x2", "s1", "c", "f1", "f2"]);
        assert_eq!(t.split_of("a").unwrap().tabs, ["b".to_owned(), "a".to_owned()]);
        assert_eq!(t.make_split("x3", "c", "a"), Err(TreeError::InSplit));
        assert_eq!(t.make_split("x3", "c", "p1"), Err(TreeError::Pinned));
        assert_eq!(t.make_split("x3", "c", "c"), Err(TreeError::SameTab));
        // A split made from a folder's tab stays in that folder.
        t.make_split("x3", "g", "c").unwrap();
        assert_eq!(t.folder("f2").unwrap().children, vec![Child::Split(split("x3", "g", "c"))]);
        t.dissolve_split("x2").unwrap();
        assert_eq!(rows(&t), ["b", "a", "s1", "f1", "f2"]);
        t.dissolve_split("x3").unwrap();
        assert_eq!(t.folder("f2").unwrap().children, vec![ctab("g"), ctab("c")]);
        assert_eq!(t.check(), Ok(()));
    }

    #[test]
    fn closing_half_a_split_leaves_the_other_as_a_tab() {
        let mut t = sample();
        t.make_split("x2", "a", "b").unwrap();
        t.last_active = "a".into();
        assert!(t.close_tab("a"));
        assert_eq!(rows(&t)[0], "b");
        assert!(t.split_of("b").is_none());
        assert!(t.last_active.is_empty());
        assert!(t.close_tab("f"));
        assert_eq!(t.folder("f1").unwrap().children, vec![ctab("d"), ctab("e")]);
        assert!(!t.close_tab("f"));
        assert_eq!(t.check(), Ok(()));
    }

    #[test]
    fn split_ratio_and_focus() {
        let mut t = sample();
        t.set_split_ratio("x1", 0.95).unwrap();
        assert_eq!(t.split_of("e").unwrap().ratio, SPLIT_RATIO_MAX);
        t.set_split_ratio("x1", f64::NAN).unwrap();
        assert_eq!(t.split_of("e").unwrap().ratio, 0.5);
        t.focus_tab("f");
        assert_eq!(t.split_of("e").unwrap().focused(), "f");
        assert_eq!(t.set_split_ratio("f1", 0.5), Err(TreeError::UnknownRow));
    }

    #[test]
    fn a_new_tab_joins_the_project_in_front() {
        let mut t = sample();
        t.add_tab("n1", Some("g"), 0).unwrap();
        assert_eq!(t.folder("f2").unwrap().children, vec![ctab("g"), ctab("n1")]);
        // A plain folder's tab in front does not pull the new tab in.
        t.add_tab("n2", Some("d"), 0).unwrap();
        assert_eq!(rows(&t)[0], "n2");
        t.add_tab("n3", None, 99).unwrap();
        assert_eq!(*rows(&t).last().unwrap(), "n3");
        assert_eq!(t.add_tab("a", None, 0), Err(TreeError::AlreadyPresent));
        // A project's split in front: the new tab follows the split.
        t.make_split("x9", "g", "n1").unwrap();
        t.add_tab("n4", Some("n1"), 0).unwrap();
        assert_eq!(t.folder("f2").unwrap().children[1], ctab("n4"));
    }

    #[test]
    fn a_promoted_peek_lands_under_its_parent() {
        let mut t = sample();
        t.add_promoted_peek("k1", "a").unwrap();
        assert_eq!(rows(&t)[..3], ["a", "k1", "b"]);
        t.add_promoted_peek("k2", "d").unwrap();
        assert_eq!(t.folder("f1").unwrap().children[1], ctab("k2"));
        t.add_promoted_peek("k3", "p1").unwrap();
        assert_eq!(rows(&t)[0], "k3");
        assert_eq!(t.add_promoted_peek("k4", "gone"), Err(TreeError::UnknownRow));
    }

    #[test]
    fn a_collapsed_folder_shows_three_favicons_and_a_count() {
        let mut t = sample();
        t.add_tab("h", Some("g"), 0).unwrap();
        t.move_row("a", Dest::Folder { id: "f1".into(), index: 9 }).unwrap();
        let s = t.folder_summary("f1").unwrap();
        assert_eq!(s.favicons, ["d", "e", "f"]);
        assert_eq!(s.count, 4);
        assert_eq!(t.folder_summary("f2").unwrap(), FolderSummary { favicons: vec!["g".into(), "h".into()], count: 2 });
        t.new_folder("f3", FolderKind::Plain, " Empty ", "book-open", Some(30), 0).unwrap();
        assert_eq!(t.folder_summary("f3").unwrap(), FolderSummary { favicons: vec![], count: 0 });
        assert_eq!(t.folder("f3").unwrap().name, "Empty");
        t.set_collapsed("f3", true).unwrap();
        assert!(t.folder("f3").unwrap().collapsed);
    }

    #[test]
    fn repair_drops_repeats_and_unknown_tabs_and_heals_splits() {
        let mut t = tree(
            &["a", "a"],
            vec![
                tab("a"),
                tab("b"),
                Item::Split(Split { id: "x1".into(), tabs: ["b".into(), "c".into()], ratio: 3.0, focus: Half::Second }),
                folder("f1", FolderKind::Plain, vec![Child::Split(split("x2", "gone", "d")), ctab("e")]),
                folder("f1", FolderKind::Plain, vec![ctab("z")]),
            ],
        );
        t.repair(|id| id != "gone");
        assert_eq!(t.pins, ["a"]);
        assert_eq!(rows(&t), ["b", "c", "f1"]);
        assert_eq!(t.folder("f1").unwrap().children, vec![ctab("d"), ctab("e")]);
        assert_eq!(t.check(), Ok(()));
        let mut ok = tree(&[], vec![Item::Split(Split { ratio: 3.0, ..split("x1", "a", "b") })]);
        ok.repair(|_| true);
        assert_eq!(ok.split_of("a").unwrap().ratio, SPLIT_RATIO_MAX);
    }

    #[test]
    fn check_catches_broken_trees() {
        assert_eq!(tree(&["a"], vec![tab("a")]).check(), Err(TreeError::AlreadyPresent));
        assert_eq!(tree(&[], vec![Item::Split(split("x", "a", "a"))]).check(), Err(TreeError::AlreadyPresent));
        assert_eq!(tree(&[], vec![sep("s", ""), sep("s", "")]).check(), Err(TreeError::AlreadyPresent));
        let wide = Split { ratio: 0.99, ..split("x", "a", "b") };
        assert_eq!(tree(&[], vec![Item::Split(wide)]).check(), Err(TreeError::BadRatio));
    }

    #[test]
    fn a_tree_round_trips_through_json_and_skips_bad_rows() {
        let t = sample();
        let text = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<SidebarTree>(&text).unwrap(), t);
        let odd: SidebarTree = serde_json::from_str(
            r#"{"pins":null,"items":[{"type":"tab","id":"a"},{"type":"spaceship"},{"type":"separator","id":"s","name":null},{"type":"folder","id":"f","children":null,"kind":null}],"lastActive":null}"#,
        )
        .unwrap();
        assert_eq!(rows(&odd), ["a", "s", "f"]);
        assert!(odd.pins.is_empty() && odd.folder("f").unwrap().children.is_empty());
    }
}
