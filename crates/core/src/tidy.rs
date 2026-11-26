//! Tidy: sorts a sidebar's loose tabs into groups and writes a named separator above each,
//! using the same separators the user makes by hand. The groups come from a text model the
//! shell runs, checked here before anything moves, or from grouping by site, which needs no
//! model. Pinned tabs, folders, projects and splits are left alone. Running a model is the
//! shell's; this module only sees its answer.

use std::collections::{HashMap, HashSet};

use crate::sidebar_tree::{Item, SidebarTree};
use crate::url::strip_www;

/// What the model is told about one tab, and all it is told: no page content.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TidyTab {
    pub id: String,
    pub title: String,
    /// The tab's host with its port, `localhost:3000`; empty for `file:`, `about:` and new tabs.
    pub host: String,
    /// The tab it was opened from, when that is known.
    pub opener: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TidyGroup {
    pub name: String,
    pub tabs: Vec<String>,
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
    TooManyGroups,
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
/// More separators than this is a list cut into pieces, not tidied.
pub const MAX_GROUPS: usize = 12;
/// A separator name longer than this is cut, at a character boundary.
pub const MAX_NAME_CHARS: usize = 40;

/// The fallback that needs no model: one group per host (`www.` aside, subdomains and ports
/// apart) with two or more tabs, named by the host, in the order hosts first appear; the rest
/// under "Other" at the end, which also takes the hosts that first appear after the group limit
/// is reached. A tab with no host goes with the tab that opened it.
pub fn group_by_site(tabs: &[TidyTab]) -> Vec<TidyGroup> {
    let site = |host: &str| strip_www(&host.trim().to_lowercase()).to_owned();
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
    // Past the limit the hosts seen first keep their groups and the rest join Other.
    let room = if other.is_empty() && groups.len() <= MAX_GROUPS { MAX_GROUPS } else { MAX_GROUPS - 1 };
    if groups.len() > room {
        let overflow: Vec<String> = groups.drain(room..).flat_map(|g| g.tabs).collect();
        other.splice(0..0, overflow);
    }
    if !other.is_empty() {
        groups.push(TidyGroup { name: OTHER_GROUP.into(), tabs: other });
    }
    groups
}

/// Accepts an answer only if it places every loose tab exactly once, names no other tab, gives
/// every group a name and a tab, and keeps to [`MAX_GROUPS`]. Names come back trimmed and cut
/// to [`MAX_NAME_CHARS`].
pub fn validate(loose: &[&str], answer: Vec<TidyGroup>) -> Result<Vec<TidyGroup>, TidyError> {
    if loose.is_empty() {
        return Err(TidyError::NothingToTidy);
    }
    if answer.len() > MAX_GROUPS {
        return Err(TidyError::TooManyGroups);
    }
    let wanted: HashSet<&str> = loose.iter().copied().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for g in answer {
        let name: String = g.name.trim().chars().take(MAX_NAME_CHARS).collect();
        let name = name.trim_end().to_owned();
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

impl TidyUndo {
    /// Whether [`undo`] would still put the list back: nothing changed since the tidy.
    pub fn holds(&self, tree: &SidebarTree) -> bool {
        tree.items == self.after
    }
}

/// The Tidy action: the model's answer when there is one and it holds up, else grouping by
/// site. `new_id` names each separator.
pub fn tidy(
    tree: &mut SidebarTree,
    tabs: &[TidyTab],
    answer: Option<Vec<TidyGroup>>,
    mut new_id: impl FnMut() -> String,
) -> Result<(TidyUndo, Source), TidyError> {
    let loose = tree.loose_tabs();
    let (groups, source) = match answer.map(|a| validate(&loose, a)) {
        Some(Ok(groups)) => (groups, Source::Model),
        _ => (validate(&loose, group_by_site(tabs))?, Source::Site),
    };
    let groups = groups.into_iter().map(|g| ((new_id(), g.name), g.tabs)).collect();
    let before = tree.regroup_loose(groups);
    Ok((TidyUndo { before, after: tree.items.clone() }, source))
}

/// Puts the list back exactly as it was before the tidy, if nothing changed since.
pub fn undo(tree: &mut SidebarTree, undo: TidyUndo) -> Result<(), TidyError> {
    if tree.restore_rows(undo.before, &undo.after) { Ok(()) } else { Err(TidyError::Stale) }
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

    /// Forty tabs across twelve hosts as a real sidebar has them.
    fn real() -> (SidebarTree, Vec<TidyTab>) {
        let hosts = [
            "github.com",
            "www.github.com",
            "gist.github.com",
            "docs.rs",
            "127.0.0.1:8080",
            "[::1]:3000",
            "localhost:3000",
            "localhost:5173",
            "",
            "",
            "news.ycombinator.com",
            "mail.google.com",
        ];
        let mut tabs = Vec::new();
        for n in 0..40 {
            let host = hosts[n % hosts.len()];
            let title = if n == 7 || n == 19 { "Untitled".to_owned() } else { format!("Tab {n}") };
            let opener = host.is_empty().then(|| format!("x{}", n - 1 - n % hosts.len() % 8));
            tabs.push(TidyTab { id: format!("x{n}"), title, host: host.into(), opener });
        }
        let tree =
            SidebarTree { items: tabs.iter().map(|t| Item::tab(t.id.clone())).collect(), ..SidebarTree::default() };
        (tree, tabs)
    }

    #[test]
    fn grouping_by_site_needs_two_tabs_and_follows_openers() {
        let (_, tabs) = sample();
        assert_eq!(group_by_site(&tabs), [g("github.com", &["a", "c", "d"]), g("Other", &["b"])]);
        assert_eq!(group_by_site(&[]), []);
    }

    #[test]
    fn forty_real_tabs_group_by_host_and_port() {
        let (mut tree, tabs) = real();
        let groups = group_by_site(&tabs);
        let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "github.com",
                "gist.github.com",
                "docs.rs",
                "127.0.0.1:8080",
                "[::1]:3000",
                "localhost:3000",
                "localhost:5173",
                "news.ycombinator.com",
                "mail.google.com"
            ]
        );
        // github.com and www.github.com are one site; file: and about: tabs follow their opener.
        assert_eq!(groups[0].tabs.len(), 4 + 4);
        assert_eq!(groups[6].tabs, ["x7", "x8", "x9", "x19", "x20", "x21", "x31", "x32", "x33"]);
        assert_eq!(groups.iter().map(|g| g.tabs.len()).sum::<usize>(), 40);
        let (_, source) = tidy(&mut tree, &tabs, None, ids()).unwrap();
        assert_eq!(source, Source::Site);
        assert_eq!(tree.loose_tabs().len(), 40);
        tree.check().unwrap();
    }

    #[test]
    fn a_models_answer_must_place_every_loose_tab_once() {
        let loose = ["a", "b", "c"];
        assert_eq!(validate(&loose, vec![g(" Dev ", &["a", "c"]), g("News", &["b"])]).unwrap()[0].name, "Dev");
        assert_eq!(validate(&loose, vec![g("Dev", &["a", "c"])]), Err(TidyError::Missing("b".into())));
        assert_eq!(validate(&loose, vec![g("Dev", &["a", "b", "c", "zz"])]), Err(TidyError::Invented("zz".into())));
        assert_eq!(validate(&loose, vec![g("Dev", &["a", "b", "c", "p"])]), Err(TidyError::Invented("p".into())));
        assert_eq!(
            validate(&loose, vec![g("Dev", &["a", "b"]), g("X", &["b", "c"])]),
            Err(TidyError::Repeated("b".into()))
        );
        assert_eq!(validate(&loose, vec![g(" ", &["a", "b", "c"])]), Err(TidyError::EmptyName));
        assert_eq!(validate(&loose, vec![g("Dev", &["a", "b", "c"]), g("X", &[])]), Err(TidyError::EmptyGroup));
        assert_eq!(validate(&[], vec![]), Err(TidyError::NothingToTidy));
    }

    #[test]
    fn real_bad_answers_fall_back_to_sites() {
        let (tree, tabs) = real();
        let all: Vec<String> = tabs.iter().map(|t| t.id.clone()).collect();
        let half = |r: std::ops::Range<usize>| all[r].to_vec();
        let named = |name: &str, tabs: Vec<String>| TidyGroup { name: name.into(), tabs };
        let mut dup = half(0..40);
        dup.push("x3".into());
        let answers = [
            ("duplicates a tab", vec![named("All", dup)]),
            ("omits one", vec![named("All", half(0..39))]),
            ("invents one", vec![named("All", half(0..40)), named("Ghost", vec!["x99".into()])]),
            ("empty group", vec![named("All", half(0..40)), named("None", vec![])]),
            (
                "200 groups",
                (0..200).map(|n| named(&format!("G{n}"), all.get(n).cloned().into_iter().collect())).collect(),
            ),
        ];
        for (case, answer) in answers {
            let mut t = tree.clone();
            let (_, source) = tidy(&mut t, &tabs, Some(answer), ids()).unwrap();
            assert_eq!(source, Source::Site, "{case}");
            t.check().unwrap();
            assert_eq!(t.loose_tabs().len(), 40, "{case}");
        }
    }

    #[test]
    fn a_ten_kilobyte_name_is_cut() {
        let (mut tree, tabs) = real();
        let all: Vec<String> = tabs.iter().map(|t| t.id.clone()).collect();
        let long = "é".repeat(10 * 1024);
        let (_, source) = tidy(&mut tree, &tabs, Some(vec![TidyGroup { name: long, tabs: all }]), ids()).unwrap();
        assert_eq!(source, Source::Model);
        let Item::Separator { name, .. } = &tree.items[0] else { panic!("a separator leads") };
        assert_eq!(name.chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn tidy_writes_separators_leaves_pins_and_folders_and_undo_restores() {
        let (mut tree, tabs) = sample();
        let original = tree.clone();
        let answer = vec![g("Dev", &["c", "a"]), g("Read", &["b", "d"])];
        let (undo_it, source) = tidy(&mut tree, &tabs, Some(answer), ids()).unwrap();
        assert_eq!(source, Source::Model);
        let rows: Vec<&str> = tree.items.iter().map(Item::id).collect();
        assert_eq!(rows, ["i1", "c", "a", "i2", "b", "d", "f"]);
        assert_eq!(tree.items[0], Item::Separator { id: "i1".into(), name: "Dev".into() });
        assert_eq!(tree.pins, ["p"]);
        assert_eq!(tree.folder("f"), original.folder("f"));
        tree.check().unwrap();
        undo(&mut tree, undo_it).unwrap();
        assert_eq!(tree, original);
    }

    #[test]
    fn undo_refuses_once_the_list_changed() {
        let (mut tree, tabs) = sample();
        let (u, _) = tidy(&mut tree, &tabs, None, ids()).unwrap();
        assert!(u.holds(&tree));
        tree.close_tab("b");
        assert!(!u.holds(&tree));
        assert_eq!(undo(&mut tree, u), Err(TidyError::Stale));
    }

    #[test]
    fn nothing_loose_is_nothing_to_tidy() {
        let mut tree = SidebarTree { pins: vec!["p".into()], ..SidebarTree::default() };
        assert_eq!(tidy(&mut tree, &[], None, ids()).map(|r| r.1), Err(TidyError::NothingToTidy));
    }

    #[test]
    fn grouping_by_site_keeps_to_the_group_limit() {
        let tabs: Vec<TidyTab> = (0..26).map(|n| t(&format!("x{n}"), &format!("h{}.test", n / 2), None)).collect();
        let mut tree =
            SidebarTree { items: tabs.iter().map(|t| Item::tab(t.id.clone())).collect(), ..SidebarTree::default() };
        let (_, source) = tidy(&mut tree, &tabs, None, ids()).unwrap();
        assert_eq!(source, Source::Site);
        let groups = group_by_site(&tabs);
        assert_eq!(groups.len(), MAX_GROUPS);
        assert_eq!(groups.last().unwrap(), &g("Other", &["x22", "x23", "x24", "x25"]));
    }
}
