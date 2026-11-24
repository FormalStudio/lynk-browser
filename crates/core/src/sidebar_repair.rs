//! Bringing a sidebar tree read from disk back within the rules `sidebar_tree` keeps: the
//! decode boundary for trees. Called from session loading only; operations never need it.

use std::collections::HashSet;

use crate::sidebar_tree::{Child, Item, SidebarTree, Split};

impl SidebarTree {
    /// Tabs `keep` rejects and repeats go, a split left with one tab becomes that tab, and a
    /// repeated separator, folder or split id keeps its first row (a split's tabs stay as plain
    /// rows).
    pub(crate) fn repair(&mut self, mut keep: impl FnMut(&str) -> bool) {
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
                Item::Split(s) => items.extend(repair_split(s, &mut take, &mut rows).into_iter().map(Child::into_item)),
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

    /// The rules a repaired tree and every operation keep: each tab once, each row id once, a
    /// split's two halves two tabs. Folders inside folders cannot be written down at all.
    #[cfg(test)]
    pub(crate) fn check(&self) -> Result<(), String> {
        let mut seen = HashSet::new();
        for id in self.tab_ids() {
            if !seen.insert(id.to_owned()) {
                return Err(format!("tab {id} twice"));
            }
        }
        let mut splits: Vec<&Split> = Vec::new();
        for item in &self.items {
            if !matches!(item, Item::Tab { .. }) && !seen.insert(item.id().to_owned()) {
                return Err(format!("row {} twice", item.id()));
            }
            match item {
                Item::Split(s) => splits.push(s),
                Item::Folder(f) => {
                    for c in &f.children {
                        if let Child::Split(s) = c {
                            if !seen.insert(s.id.clone()) {
                                return Err(format!("row {} twice", s.id));
                            }
                            splits.push(s);
                        }
                    }
                }
                _ => {}
            }
        }
        match splits.iter().find(|s| s.tabs[0] == s.tabs[1]) {
            Some(s) => Err(format!("split {} holds one tab twice", s.id)),
            None => Ok(()),
        }
    }
}

fn repair_split(s: Split, take: &mut impl FnMut(&str) -> bool, rows: &mut HashSet<String>) -> Vec<Child> {
    let kept: Vec<String> = s.tabs.iter().filter(|t| take(t)).cloned().collect();
    if kept.len() == 2 && rows.insert(s.id.clone()) {
        return vec![Child::Split(s)];
    }
    kept.into_iter().map(|id| Child::Tab { id }).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::sidebar_tree::{Dest, Folder, FolderKind, Half, Ratio};
    use crate::spaces::{Space, WindowSpaces};

    fn split(id: &str, a: &str, b: &str) -> Split {
        Split { id: id.into(), tabs: [a.into(), b.into()], ratio: Ratio::EVEN, focus: Half::First }
    }

    fn folder(id: &str, children: Vec<Child>) -> Item {
        Item::Folder(Folder {
            id: id.into(),
            kind: FolderKind::Plain,
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

    #[test]
    fn repair_drops_repeats_and_unknown_tabs_and_heals_splits() {
        let mut t = SidebarTree {
            pins: vec!["a".into(), "a".into()],
            items: vec![
                Item::tab("a"),
                Item::tab("b"),
                Item::Split(split("x1", "b", "c")),
                folder("f1", vec![Child::Split(split("x2", "gone", "d")), ctab("e")]),
                folder("f1", vec![ctab("z")]),
                Item::Split(split("x1", "y", "w")),
            ],
            last_active: String::new(),
        };
        t.repair(|id| id != "gone");
        assert_eq!(t.pins, ["a"]);
        let rows: Vec<&str> = t.items.iter().map(Item::id).collect();
        assert_eq!(rows, ["b", "c", "f1", "x1"], "the first x1 broke up, so the second keeps its id");
        assert_eq!(t.folder("f1").unwrap().children, vec![ctab("d"), ctab("e")]);
        assert_eq!(t.check(), Ok(()));
    }

    #[test]
    fn check_catches_broken_trees() {
        let broken = |pins: Vec<&str>, items: Vec<Item>| SidebarTree {
            pins: pins.into_iter().map(Into::into).collect(),
            items,
            last_active: String::new(),
        };
        assert!(broken(vec!["a"], vec![Item::tab("a")]).check().is_err());
        assert!(broken(vec![], vec![Item::Split(split("x", "a", "a"))]).check().is_err());
        let sep = || Item::Separator { id: "s".into(), name: String::new() };
        assert!(broken(vec![], vec![sep(), sep()]).check().is_err());
    }

    /// A seeded generator, so a failing walk replays exactly.
    struct Lcg(u64);

    impl Lcg {
        fn below(&mut self, n: usize) -> usize {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % n.max(1)
        }
    }

    #[test]
    fn no_walk_of_ten_thousand_operations_breaks_a_rule_or_loses_a_tab() {
        let spaces = vec![Space { id: "s1".into(), ..Space::default() }, Space { id: "s2".into(), ..Space::default() }];
        let mut w = WindowSpaces::single("s1", SidebarTree::default());
        w.sidebars.insert("s2".into(), SidebarTree::default());
        let mut alive: BTreeSet<String> = BTreeSet::new();
        let mut next = 0u64;
        let mut active = String::new();
        let mut rng = Lcg(0x5eed);
        for step in 0..10_000 {
            let tabs: Vec<String> = alive.iter().cloned().collect();
            let pick = |rng: &mut Lcg| tabs.get(rng.below(tabs.len())).cloned().unwrap_or_default();
            let space = ["s1", "s2"][rng.below(2)];
            let tree = w.sidebars.get_mut(space).unwrap();
            let rows: Vec<String> = tree.items.iter().map(|i| i.id().to_owned()).collect();
            let folders: Vec<String> =
                tree.items.iter().filter(|i| matches!(i, Item::Folder(_))).map(|i| i.id().to_owned()).collect();
            let row = rows.get(rng.below(rows.len())).cloned().unwrap_or_default();
            let n = rng.below(64);
            let op = rng.below(14);
            next += 1;
            let fresh = format!("n{next}");
            match op {
                0 | 1 => {
                    if tree.add_tab(&fresh, Some(&pick(&mut rng)), n).is_ok() {
                        alive.insert(fresh);
                    }
                }
                2 => {
                    let _ = tree.move_row(&row, Dest::Root(n));
                }
                3 => {
                    let _ = tree.pin(&pick(&mut rng), n);
                }
                4 => {
                    let _ = tree.unpin(&pick(&mut rng), n);
                }
                5 => {
                    let _ = tree.make_split(&fresh, &pick(&mut rng), &pick(&mut rng));
                }
                6 => {
                    let _ = tree.dissolve_split(&row);
                }
                7 => {
                    let t = pick(&mut rng);
                    if w.sidebars.values_mut().any(|tree| tree.close_tab(&t)) {
                        alive.remove(&t);
                    }
                }
                8 => {
                    let _ = tree.new_folder(&fresh, FolderKind::Project, "p", "", None, n);
                }
                9 => {
                    if let Some(f) = folders.get(rng.below(folders.len())) {
                        let _ = tree.move_row(&pick(&mut rng), Dest::Folder { id: f.clone(), index: n });
                        let _ = tree.move_row(&row, Dest::Folder { id: f.clone(), index: n });
                    }
                }
                10 => {
                    let _ = tree.move_to_root(&pick(&mut rng));
                }
                11 => {
                    let _ = tree.add_separator_below(&pick(&mut rng), &fresh, "x");
                    let _ = tree.delete_separator(&row);
                }
                12 => {
                    if tree.add_promoted_peek(&fresh, &pick(&mut rng)).is_ok() {
                        alive.insert(fresh);
                    }
                }
                _ => {
                    let to = ["s1", "s2"][rng.below(2)];
                    let _ = w.move_tab(&spaces, &pick(&mut rng), to, true, &mut active);
                }
            }
            let mut seen: Vec<String> = Vec::new();
            for (id, tree) in &w.sidebars {
                tree.check().unwrap_or_else(|e| panic!("step {step} op {op} in {id}: {e}"));
                seen.extend(tree.tab_ids().iter().map(|t| (*t).to_owned()));
            }
            let total = seen.len();
            let seen: BTreeSet<String> = seen.into_iter().collect();
            assert_eq!(total, seen.len(), "step {step} op {op}: a tab is in two spaces");
            assert_eq!(seen, alive, "step {step} op {op}: tabs lost or invented");
        }
        assert!(alive.len() > 10, "the walk should build up a real tree, has {}", alive.len());
    }
}
