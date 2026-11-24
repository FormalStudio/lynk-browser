//! Most recently used order per window, for the visual tab switcher. Holding the switcher's
//! modifier starts a [`Cycle`] over a snapshot of the order; the order changes only when the
//! modifier is released and the cycle commits.

use serde::{Deserialize, Deserializer, Serialize};

/// Tab ids, most recently shown first. Saved with the window, so it survives quitting and
/// reopening.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Mru(#[serde(deserialize_with = "ids")] pub Vec<String>);

fn ids<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let raw = Option::<Vec<serde_json::Value>>::deserialize(d)?.unwrap_or_default();
    Ok(raw.into_iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
}

impl Mru {
    /// A tab came on show.
    pub fn shown(&mut self, id: &str) {
        self.0.retain(|t| t != id);
        self.0.insert(0, id.to_owned());
    }

    pub fn closed(&mut self, id: &str) {
        self.0.retain(|t| t != id);
    }

    /// Drops ids that are not tabs any more, and repeats.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        let mut seen = std::collections::HashSet::new();
        self.0.retain(|t| keep(t) && seen.insert(t.clone()));
    }

    /// Every tab in `tabs` (drawing order) by recency: those ever shown first, the rest after
    /// them in drawing order.
    pub fn order(&self, tabs: &[&str]) -> Vec<String> {
        let mut out: Vec<String> = self.0.iter().filter(|t| tabs.contains(&t.as_str())).cloned().collect();
        out.extend(tabs.iter().filter(|t| !self.0.iter().any(|m| m == *t)).map(|t| (*t).to_owned()));
        out
    }
}

/// One hold of the switcher key: the cards in recency order and the one selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cycle {
    cards: Vec<String>,
    at: usize,
}

impl Cycle {
    /// Starts on the tab after `active`. The tab on show leads the cards; `skip` leaves others
    /// out (pinned tabs, when the setting says so). `None` with nothing to switch to.
    pub fn start(mru: &Mru, tabs: &[&str], active: &str, skip: impl Fn(&str) -> bool) -> Option<Self> {
        let mut cards: Vec<String> = vec![active.to_owned()];
        cards.extend(mru.order(tabs).into_iter().filter(|t| t != active && !skip(t)));
        (cards.len() > 1).then_some(Self { cards, at: 1 })
    }

    pub fn next(&mut self) {
        self.at = (self.at + 1) % self.cards.len();
    }

    pub fn previous(&mut self) {
        self.at = (self.at + self.cards.len() - 1) % self.cards.len();
    }

    /// Clicking a card selects it.
    pub fn select(&mut self, id: &str) {
        if let Some(at) = self.cards.iter().position(|c| c == id) {
            self.at = at;
        }
    }

    pub fn cards(&self) -> &[String] {
        &self.cards
    }

    pub fn selected(&self) -> &str {
        &self.cards[self.at]
    }

    /// The modifier was released: the selected tab goes on show and to the front of the order.
    pub fn commit(self, mru: &mut Mru) -> String {
        let id = self.cards[self.at].clone();
        mru.shown(&id);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mru(ids: &[&str]) -> Mru {
        Mru(ids.iter().map(|t| (*t).to_owned()).collect())
    }

    #[test]
    fn showing_and_closing_update_the_order() {
        let mut m = Mru::default();
        for id in ["a", "b", "c", "a"] {
            m.shown(id);
        }
        assert_eq!(m, mru(&["a", "c", "b"]));
        m.closed("c");
        assert_eq!(m.order(&["d", "b", "a"]), ["a", "b", "d"]);
    }

    #[test]
    fn a_held_cycle_reorders_only_on_release() {
        let mut m = mru(&["a", "b", "c", "d"]);
        let tabs = ["d", "c", "b", "a"];
        let mut cycle = Cycle::start(&m, &tabs, "a", |_| false).unwrap();
        assert_eq!(cycle.selected(), "b");
        cycle.next();
        cycle.next();
        assert_eq!(cycle.selected(), "d");
        assert_eq!(m, mru(&["a", "b", "c", "d"]));
        cycle.next();
        assert_eq!(cycle.selected(), "a");
        cycle.previous();
        cycle.previous();
        assert_eq!(cycle.selected(), "c");
        assert_eq!(cycle.commit(&mut m), "c");
        assert_eq!(m, mru(&["c", "a", "b", "d"]));
        // One press and release goes back to the previous tab.
        let quick = Cycle::start(&m, &tabs, "c", |_| false).unwrap();
        assert_eq!(quick.commit(&mut m), "a");
    }

    #[test]
    fn a_cycle_can_skip_tabs_and_needs_somewhere_to_go() {
        let m = mru(&["a", "p", "b"]);
        let mut cycle = Cycle::start(&m, &["p", "a", "b"], "a", |t| t == "p").unwrap();
        assert_eq!(cycle.cards(), ["a", "b"]);
        cycle.select("a");
        assert_eq!(cycle.selected(), "a");
        assert!(Cycle::start(&m, &["a"], "a", |_| false).is_none());
    }

    #[test]
    fn the_order_round_trips_and_skips_junk() {
        let m = mru(&["a", "b"]);
        let text = serde_json::to_string(&m).unwrap();
        assert_eq!(text, r#"["a","b"]"#);
        assert_eq!(serde_json::from_str::<Mru>(r#"["a",3,null,"b"]"#).unwrap(), m);
        let mut dup = mru(&["a", "x", "a", "b"]);
        dup.retain(|t| t != "x");
        assert_eq!(dup, m);
    }
}
