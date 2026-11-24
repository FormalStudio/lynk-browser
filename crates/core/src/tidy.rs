//! Tidy: sorts a sidebar's loose tabs into groups and writes a named separator above each,
//! using the same separators the user makes by hand. A text model can propose the groups;
//! grouping by site needs none. Pinned tabs, folders, projects and splits are left alone.

use std::collections::{HashMap, HashSet};

use crate::sidebar_tree::{Item, SidebarTree};

/// What the model is told about one tab, and all it is told: no page content.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TidyTab {
    pub id: String,
    pub title: String,
    pub host: String,
    /// The tab it was opened from, when that is known.
    pub opener: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TidyGroup {
    pub name: String,
    pub tabs: Vec<String>,
}

/// A text model that groups tabs: Apple's system model on macOS, a local one the user chose
/// elsewhere. Its answer is checked by [`validate`] before anything moves. Called off the main
/// thread.
pub trait TabGrouper {
    fn group(&self, tabs: &[TidyTab]) -> Result<Vec<TidyGroup>, String>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TidyError {
    NothingToTidy,
    /// A loose tab the answer left out.
    Missing(String),
    /// An id that is not a loose tab: made up, pinned, or inside a folder or split.
    Invented(String),
    Repeated(String),
    EmptyName,
    EmptyGroup,
    /// The list changed since the tidy, so undo would lose that change.
    Stale,
}

/// Which grouping a tidy used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Model,
    Site,
}

pub const OTHER_GROUP: &str = "Other";

fn site(host: &str) -> String {
    let host = host.trim().to_ascii_lowercase();
    host.strip_prefix("www.").map_or(host.clone(), str::to_owned)
}

/// The fallback that needs no model: one group per site with two or more tabs, named by the
/// site, in the order sites first appear; the rest under "Other" at the end. A tab with no host
/// (a new tab) goes with the tab that opened it.
pub fn group_by_site(tabs: &[TidyTab]) -> Vec<TidyGroup> {
    let hosts: HashMap<&str, &str> = tabs.iter().map(|t| (t.id.as_str(), t.host.as_str())).collect();
    let site_of = |t: &TidyTab| {
        let own = site(&t.host);
        if !own.is_empty() {
            return own;
        }
        t.opener.as_deref().and_then(|o| hosts.get(o)).map(|h| site(h)).unwrap_or_default()
    };
    let mut order: Vec<String> = Vec::new();
    let mut by_site: HashMap<String, Vec<String>> = HashMap::new();
    for t in tabs {
        let s = site_of(t);
        if !by_site.contains_key(&s) {
            order.push(s.clone());
        }
        by_site.entry(s).or_default().push(t.id.clone());
    }
    let mut groups = Vec::new();
    let mut other = Vec::new();
    for s in order {
        let ids = by_site.remove(&s).unwrap_or_default();
        if s.is_empty() || ids.len() < 2 {
            other.extend(ids);
        } else {
            groups.push(TidyGroup { name: s, tabs: ids });
        }
    }
    if !other.is_empty() {
        groups.push(TidyGroup { name: OTHER_GROUP.into(), tabs: other });
    }
    groups
}

/// Accepts an answer only if it places every loose tab exactly once, names no other tab, and
/// gives every group a name and at least one tab. Names come back trimmed.
pub fn validate(loose: &[&str], answer: Vec<TidyGroup>) -> Result<Vec<TidyGroup>, TidyError> {
    if loose.is_empty() {
        return Err(TidyError::NothingToTidy);
    }
    let wanted: HashSet<&str> = loose.iter().copied().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for g in answer {
        let name = g.name.trim().to_owned();
        if name.is_empty() {
            return Err(TidyError::EmptyName);
        }
        if g.tabs.is_empty() {
            return Err(TidyError::EmptyGroup);
        }
        for id in &g.tabs {
            if !wanted.contains(id.as_str()) {
                return Err(TidyError::Invented(id.clone()));
            }
            if !seen.insert(id.clone()) {
                return Err(TidyError::Repeated(id.clone()));
            }
        }
        out.push(TidyGroup { name, tabs: g.tabs });
    }
    if let Some(missing) = loose.iter().find(|id| !seen.contains(**id)) {
        return Err(TidyError::Missing((*missing).to_owned()));
    }
    Ok(out)
}

/// What [`undo`] needs to put the list back.
#[derive(Clone, Debug, PartialEq)]
pub struct TidyUndo {
    before: Vec<Item>,
    after: Vec<Item>,
}

/// Rewrites the list as the groups, each under a new named separator, followed by the
/// folders and splits in their order. The separators that were there go: the groups replace
/// them.
pub fn apply(
    tree: &mut SidebarTree,
    groups: Vec<TidyGroup>,
    mut new_id: impl FnMut() -> String,
) -> Result<TidyUndo, TidyError> {
    let groups = validate(&tree.loose_tabs(), groups)?;
    let before = tree.items.clone();
    let mut items = Vec::new();
    for g in groups {
        items.push(Item::Separator { id: new_id(), name: g.name });
        items.extend(g.tabs.into_iter().map(Item::tab));
    }
    items.extend(before.iter().filter(|i| matches!(i, Item::Folder(_) | Item::Split(_))).cloned());
    tree.items = items;
    Ok(TidyUndo { before, after: tree.items.clone() })
}

/// Puts the list back exactly as it was before the tidy, if nothing changed since.
pub fn undo(tree: &mut SidebarTree, undo: TidyUndo) -> Result<(), TidyError> {
    if tree.items != undo.after {
        return Err(TidyError::Stale);
    }
    tree.items = undo.before;
    Ok(())
}

/// The Tidy action: the model's groups when there is a model and its answer holds up, else
/// grouping by site.
pub fn tidy(
    tree: &mut SidebarTree,
    tabs: &[TidyTab],
    model: Option<&dyn TabGrouper>,
    new_id: impl FnMut() -> String,
) -> Result<(TidyUndo, Source), TidyError> {
    let loose = tree.loose_tabs();
    let proposed = model.and_then(|m| m.group(tabs).ok()).and_then(|g| validate(&loose, g).ok());
    match proposed {
        Some(groups) => apply(tree, groups, new_id).map(|u| (u, Source::Model)),
        None => apply(tree, group_by_site(tabs), new_id).map(|u| (u, Source::Site)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidebar_tree::{Child, Folder, FolderKind};

    fn t(id: &str, host: &str, opener: Option<&str>) -> TidyTab {
        TidyTab { id: id.into(), title: id.into(), host: host.into(), opener: opener.map(Into::into) }
    }

    fn g(name: &str, tabs: &[&str]) -> TidyGroup {
        TidyGroup { name: name.into(), tabs: tabs.iter().map(|t| (*t).into()).collect() }
    }

    fn sample() -> (SidebarTree, Vec<TidyTab>) {
        let tree = SidebarTree {
            pins: vec!["p".into()],
            items: vec![
                Item::tab("a"),
                Item::Separator { id: "s0".into(), name: "Mine".into() },
                Item::tab("b"),
                Item::Folder(Folder {
                    id: "f".into(),
                    kind: FolderKind::Project,
                    name: "Ask".into(),
                    icon: String::new(),
                    tint: None,
                    collapsed: false,
                    children: vec![Child::Tab { id: "in".into() }],
                }),
                Item::tab("c"),
                Item::tab("d"),
            ],
            last_active: String::new(),
        };
        let tabs = vec![
            t("a", "www.github.com", None),
            t("b", "news.test", None),
            t("c", "github.com", None),
            t("d", "", Some("c")),
        ];
        (tree, tabs)
    }

    fn ids() -> impl FnMut() -> String {
        let mut n = 0;
        move || {
            n += 1;
            format!("i{n}")
        }
    }

    #[test]
    fn grouping_by_site_needs_two_tabs_and_follows_openers() {
        let (_, tabs) = sample();
        assert_eq!(group_by_site(&tabs), [g("github.com", &["a", "c", "d"]), g("Other", &["b"])]);
        assert_eq!(group_by_site(&[]), []);
    }

    #[test]
    fn a_models_answer_must_place_every_loose_tab_once() {
        let loose = ["a", "b", "c"];
        assert_eq!(validate(&loose, vec![g(" Dev ", &["a", "c"]), g("News", &["b"])]).unwrap()[0].name, "Dev");
        assert_eq!(validate(&loose, vec![g("Dev", &["a", "c"])]), Err(TidyError::Missing("b".into())));
        assert_eq!(validate(&loose, vec![g("Dev", &["a", "b", "c", "zz"])]), Err(TidyError::Invented("zz".into())));
        assert_eq!(validate(&loose, vec![g("Dev", &["a", "b", "c", "p"])]), Err(TidyError::Invented("p".into())));
        assert_eq!(validate(&loose, vec![g("Dev", &["a", "b"]), g("X", &["b", "c"])]), Err(TidyError::Repeated("b".into())));
        assert_eq!(validate(&loose, vec![g(" ", &["a", "b", "c"])]), Err(TidyError::EmptyName));
        assert_eq!(validate(&loose, vec![g("Dev", &["a", "b", "c"]), g("X", &[])]), Err(TidyError::EmptyGroup));
        assert_eq!(validate(&[], vec![]), Err(TidyError::NothingToTidy));
    }

    #[test]
    fn apply_writes_separators_and_leaves_pins_and_folders_alone_and_undo_restores() {
        let (mut tree, _) = sample();
        let original = tree.clone();
        let undo_it = apply(&mut tree, vec![g("Dev", &["c", "a"]), g("Read", &["b", "d"])], ids()).unwrap();
        let rows: Vec<&str> = tree.items.iter().map(Item::id).collect();
        assert_eq!(rows, ["i1", "c", "a", "i2", "b", "d", "f"]);
        assert_eq!(tree.items[0], Item::Separator { id: "i1".into(), name: "Dev".into() });
        assert_eq!(tree.pins, ["p"]);
        assert_eq!(tree.folder("f"), original.folder("f"));
        assert_eq!(tree.check(), Ok(()));
        undo(&mut tree, undo_it).unwrap();
        assert_eq!(tree, original);
    }

    #[test]
    fn undo_refuses_once_the_list_changed() {
        let (mut tree, tabs) = sample();
        let (u, source) = tidy(&mut tree, &tabs, None, ids()).unwrap();
        assert_eq!(source, Source::Site);
        tree.close_tab("b");
        assert_eq!(undo(&mut tree, u), Err(TidyError::Stale));
    }

    struct Fixed(Result<Vec<TidyGroup>, String>);

    impl TabGrouper for Fixed {
        fn group(&self, _: &[TidyTab]) -> Result<Vec<TidyGroup>, String> {
            self.0.clone()
        }
    }

    #[test]
    fn a_bad_or_failed_answer_falls_back_to_sites() {
        let (tree, tabs) = sample();
        let good = Fixed(Ok(vec![g("All", &["a", "b", "c", "d"])]));
        let invented = Fixed(Ok(vec![g("All", &["a", "b", "c", "d", "in"])]));
        let failed = Fixed(Err("no model".into()));
        let mut t1 = tree.clone();
        assert_eq!(tidy(&mut t1, &tabs, Some(&good), ids()).unwrap().1, Source::Model);
        let mut t2 = tree.clone();
        assert_eq!(tidy(&mut t2, &tabs, Some(&invented), ids()).unwrap().1, Source::Site);
        assert_eq!(t2.folder("f").unwrap().children, vec![Child::Tab { id: "in".into() }]);
        let mut t3 = tree;
        assert_eq!(tidy(&mut t3, &tabs, Some(&failed), ids()).unwrap().1, Source::Site);
    }
}
