//! Spaces: named sidebars with their own pins, tree and colours. A window shows one space at a
//! time and keeps a sidebar for each space it has visited, the way Lynk keeps pins per window.
//! With spaces switched off nothing here moves: the window keeps the space it shows, and
//! switching, moving tabs and link rules do nothing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use url::Url;

use crate::null_default;
use crate::sidebar_tree::SidebarTree;
use crate::url::strip_www;

/// The hue pair a space tints the window, sidebar, cards and menus from, in degrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Theme {
    pub hue: u16,
    pub accent_hue: u16,
}

impl Default for Theme {
    fn default() -> Self {
        Self { hue: 220, accent_hue: 260 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Space {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    #[serde(deserialize_with = "null_default")]
    pub theme: Theme,
}

pub const DEFAULT_SPACE_NAME: &str = "Personal";

/// Links to `host` and its subdomains open in `space_id`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SiteRule {
    #[serde(deserialize_with = "null_default")]
    pub host: String,
    #[serde(deserialize_with = "null_default")]
    pub space_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpaceError {
    Off,
    UnknownSpace,
    UnknownTab,
}

/// The space Ctrl+`number` goes to, 1 to 9 in the order spaces are listed.
pub fn space_at(spaces: &[Space], number: usize) -> Option<&Space> {
    (1..=9).contains(&number).then(|| spaces.get(number - 1)).flatten()
}

/// The number a space answers to in menus, when it has one.
pub fn space_number(spaces: &[Space], id: &str) -> Option<usize> {
    spaces.iter().position(|s| s.id == id).map(|i| i + 1).filter(|n| *n <= 9)
}

fn bare_host(host: &str) -> String {
    strip_www(&host.trim().trim_end_matches('.').to_ascii_lowercase()).to_owned()
}

/// Adds or replaces the rule for a host. One rule per host; `www.` is the same site.
pub fn set_rule(rules: &mut Vec<SiteRule>, host: &str, space_id: &str) {
    let host = bare_host(host);
    if host.is_empty() {
        return;
    }
    rules.retain(|r| bare_host(&r.host) != host);
    rules.push(SiteRule { host, space_id: space_id.into() });
}

/// The space a link belongs in by the rules, the most specific host winning. `None` when no
/// rule matches, the rule names a space that is gone, or spaces are off.
pub fn space_for_link<'a>(rules: &'a [SiteRule], spaces: &[Space], url: &str, enabled: bool) -> Option<&'a str> {
    if !enabled {
        return None;
    }
    let host = bare_host(Url::parse(url).ok()?.host_str()?);
    rules
        .iter()
        .filter(|r| spaces.iter().any(|s| s.id == r.space_id))
        .filter(|r| {
            let want = bare_host(&r.host);
            !want.is_empty() && (host == want || host.ends_with(&format!(".{want}")))
        })
        .max_by_key(|r| bare_host(&r.host).len())
        .map(|r| r.space_id.as_str())
}

/// One window's sidebars, one per space it has shown, and the space on show.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WindowSpaces {
    #[serde(deserialize_with = "null_default")]
    pub shown: String,
    #[serde(deserialize_with = "null_default")]
    pub sidebars: BTreeMap<String, SidebarTree>,
}

impl WindowSpaces {
    pub fn single(space_id: &str, tree: SidebarTree) -> Self {
        Self { shown: space_id.into(), sidebars: BTreeMap::from([(space_id.to_owned(), tree)]) }
    }

    /// The sidebar on show.
    pub fn tree(&self) -> Option<&SidebarTree> {
        self.sidebars.get(&self.shown)
    }

    pub fn tree_mut(&mut self) -> &mut SidebarTree {
        self.sidebars.entry(self.shown.clone()).or_default()
    }

    pub fn space_of_tab(&self, tab: &str) -> Option<&str> {
        self.sidebars.iter().find(|(_, t)| t.contains_tab(tab)).map(|(id, _)| id.as_str())
    }

    /// Every tab the window holds: the space on show first, then the others by id.
    pub fn tab_ids(&self) -> Vec<&str> {
        let shown = self.tree().into_iter();
        let rest = self.sidebars.iter().filter(|(id, _)| **id != self.shown).map(|(_, t)| t);
        shown.chain(rest).flat_map(SidebarTree::tab_ids).collect()
    }

    /// Shows space `number` (1 to 9). The sidebar left remembers its tab on show, and the one
    /// arrived at shows what it showed last, else its first tab. `active` is left empty when the
    /// space has no tab yet, for the caller to open one.
    pub fn switch(
        &mut self,
        spaces: &[Space],
        number: usize,
        enabled: bool,
        active: &mut String,
    ) -> Result<bool, SpaceError> {
        if !enabled {
            return Err(SpaceError::Off);
        }
        let target = space_at(spaces, number).ok_or(SpaceError::UnknownSpace)?.id.clone();
        if target == self.shown {
            return Ok(false);
        }
        self.tree_mut().last_active = active.clone();
        self.shown = target;
        let tree = self.tree_mut();
        *active = if tree.contains_tab(&tree.last_active) {
            tree.last_active.clone()
        } else {
            tree.tab_ids().first().map_or_else(String::new, |t| (*t).to_owned())
        };
        Ok(true)
    }

    /// Moves a tab to another space's sidebar in this window, as a loose tab at the top of its
    /// list. A tab that was on show hands over to its neighbour in the space it left.
    pub fn move_tab(
        &mut self,
        spaces: &[Space],
        tab: &str,
        to_space: &str,
        enabled: bool,
        active: &mut String,
    ) -> Result<(), SpaceError> {
        if !enabled {
            return Err(SpaceError::Off);
        }
        if !spaces.iter().any(|s| s.id == to_space) {
            return Err(SpaceError::UnknownSpace);
        }
        let from = self.space_of_tab(tab).ok_or(SpaceError::UnknownTab)?.to_owned();
        if from == to_space {
            return Ok(());
        }
        let source = self.sidebars.get_mut(&from).expect("space located");
        let order: Vec<String> = source.tab_ids().iter().map(|t| (*t).to_owned()).collect();
        source.close_tab(tab);
        if *active == tab {
            *active = neighbour(&order, tab).unwrap_or_default();
        }
        let target = self.sidebars.entry(to_space.to_owned()).or_default();
        target.add_tab(tab, None, 0).map_err(|_| SpaceError::UnknownTab)?;
        Ok(())
    }
}

/// The tab that takes over when `gone` leaves a list in `order`: the next one, else the one
/// before.
pub(crate) fn neighbour(order: &[String], gone: &str) -> Option<String> {
    let at = order.iter().position(|t| t == gone)?;
    order.get(at + 1).or_else(|| at.checked_sub(1).and_then(|i| order.get(i))).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidebar_tree::Item;

    fn space(id: &str, name: &str) -> Space {
        Space { id: id.into(), name: name.into(), theme: Theme::default() }
    }

    fn tree(pins: &[&str], tabs: &[&str]) -> SidebarTree {
        SidebarTree {
            pins: pins.iter().map(|p| (*p).into()).collect(),
            items: tabs.iter().map(|t| Item::tab(*t)).collect(),
            last_active: String::new(),
        }
    }

    fn two_spaces() -> (Vec<Space>, WindowSpaces) {
        let spaces = vec![space("s1", "Work"), space("s2", "Personal")];
        let mut w = WindowSpaces::single("s1", tree(&["p"], &["a", "b"]));
        w.sidebars.insert("s2".into(), tree(&["q"], &["c"]));
        (spaces, w)
    }

    #[test]
    fn switching_by_number_remembers_each_spaces_tab() {
        let (spaces, mut w) = two_spaces();
        let mut active = "b".to_owned();
        assert_eq!(w.switch(&spaces, 2, true, &mut active), Ok(true));
        assert_eq!((w.shown.as_str(), active.as_str()), ("s2", "q"));
        active = "c".into();
        assert_eq!(w.switch(&spaces, 1, true, &mut active), Ok(true));
        assert_eq!(active, "b");
        assert_eq!(w.switch(&spaces, 1, true, &mut active), Ok(false));
        w.switch(&spaces, 2, true, &mut active).unwrap();
        assert_eq!(active, "c");
        assert_eq!(w.switch(&spaces, 3, true, &mut active), Err(SpaceError::UnknownSpace));
        assert_eq!(w.switch(&spaces, 0, true, &mut active), Err(SpaceError::UnknownSpace));
        assert_eq!(w.switch(&spaces, 1, false, &mut active), Err(SpaceError::Off));
        assert_eq!(w.shown, "s2");
    }

    #[test]
    fn a_space_never_visited_has_no_tab_for_the_caller_to_fill() {
        let spaces = vec![space("s1", "Work"), space("s2", "Personal")];
        let mut w = WindowSpaces::single("s1", tree(&[], &["a"]));
        let mut active = "a".to_owned();
        w.switch(&spaces, 2, true, &mut active).unwrap();
        assert_eq!(active, "");
        assert!(w.tree().unwrap().tab_ids().is_empty());
    }

    #[test]
    fn a_tab_moves_to_the_top_of_another_space() {
        let (spaces, mut w) = two_spaces();
        let mut active = "a".to_owned();
        w.move_tab(&spaces, "a", "s2", true, &mut active).unwrap();
        assert_eq!(active, "b");
        assert_eq!(w.sidebars["s2"].tab_ids(), ["q", "a", "c"]);
        assert_eq!(w.sidebars["s1"].tab_ids(), ["p", "b"]);
        w.move_tab(&spaces, "p", "s2", true, &mut active).unwrap();
        assert_eq!(w.sidebars["s2"].loose_tabs(), ["p", "a", "c"]);
        assert_eq!(w.move_tab(&spaces, "b", "s9", true, &mut active), Err(SpaceError::UnknownSpace));
        assert_eq!(w.move_tab(&spaces, "zz", "s2", true, &mut active), Err(SpaceError::UnknownTab));
        assert_eq!(w.move_tab(&spaces, "b", "s2", false, &mut active), Err(SpaceError::Off));
        assert_eq!(w.tab_ids(), ["b", "q", "p", "a", "c"]);
    }

    #[test]
    fn link_rules_pick_the_most_specific_host() {
        let spaces = vec![space("s1", "Work"), space("s2", "Personal")];
        let mut rules = Vec::new();
        set_rule(&mut rules, "WWW.GitHub.com", "s1");
        set_rule(&mut rules, "gist.github.com", "s2");
        set_rule(&mut rules, "github.com", "s1");
        set_rule(&mut rules, "gone.test", "s9");
        set_rule(&mut rules, " ", "s1");
        assert_eq!(rules.len(), 3);
        assert_eq!(space_for_link(&rules, &spaces, "https://github.com/a", true), Some("s1"));
        assert_eq!(space_for_link(&rules, &spaces, "https://www.github.com/", true), Some("s1"));
        assert_eq!(space_for_link(&rules, &spaces, "https://gist.github.com/x", true), Some("s2"));
        assert_eq!(space_for_link(&rules, &spaces, "https://notgithub.com/", true), None);
        assert_eq!(space_for_link(&rules, &spaces, "https://gone.test/", true), None);
        assert_eq!(space_for_link(&rules, &spaces, "https://github.com/", false), None);
        assert_eq!(space_for_link(&rules, &spaces, "not a url", true), None);
    }

    #[test]
    fn numbers_cover_the_first_nine_spaces() {
        let spaces: Vec<Space> = (1..=10).map(|n| space(&format!("s{n}"), "x")).collect();
        assert_eq!(space_at(&spaces, 9).unwrap().id, "s9");
        assert!(space_at(&spaces, 10).is_none());
        assert_eq!(space_number(&spaces, "s3"), Some(3));
        assert_eq!(space_number(&spaces, "s10"), None);
    }

    #[test]
    fn spaces_round_trip_and_nulls_default() {
        let (_, w) = two_spaces();
        let text = serde_json::to_string(&w).unwrap();
        assert_eq!(serde_json::from_str::<WindowSpaces>(&text).unwrap(), w);
        let s: Space = serde_json::from_str(r#"{"id":"s1","name":null,"theme":null}"#).unwrap();
        assert_eq!(s.theme, Theme::default());
        assert_eq!(s.name, "");
    }
}
