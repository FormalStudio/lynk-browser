//! Pages kept on purpose, in `bookmarks.json`. Flat: the pinned tabs are where a set of pages is
//! organised; this is the list of addresses worth coming back to.

use std::path::PathBuf;

use nd_settings::Store;
use serde::{Deserialize, Serialize};

use crate::{lenient_items, now_ms, radix36};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bookmark {
    pub id: String,
    pub url: String,
    pub title: String,
    pub added: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BookmarksState {
    /// Newest first. An entry without an address is dropped on load.
    #[serde(deserialize_with = "lenient_items")]
    pub items: Vec<Bookmark>,
}

pub fn open(dir: impl Into<PathBuf>) -> Store<BookmarksState> {
    let store = Store::<BookmarksState>::open(dir.into().join("bookmarks.json"), 1, None);
    if store.get().items.iter().any(|b| b.url.is_empty()) {
        store.update(|s| s.items.retain(|b| !b.url.is_empty()));
    }
    store
}

impl BookmarksState {
    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.items.iter().any(|b| b.url == url)
    }

    /// Nothing is asked: the title is the page's. `false` when there was nothing to add.
    pub fn add(&mut self, url: &str, title: &str) -> bool {
        if url.is_empty() || self.is_bookmarked(url) {
            return false;
        }
        let added = now_ms();
        self.items.insert(
            0,
            Bookmark { id: format!("b{}", radix36(added as u64)), url: url.into(), title: title.into(), added },
        );
        true
    }

    pub fn remove(&mut self, url: &str) {
        self.items.retain(|b| b.url != url);
    }

    pub fn matching(&self, query: &str) -> Vec<&Bookmark> {
        let q = query.trim().to_lowercase();
        self.items
            .iter()
            .filter(|b| q.is_empty() || format!("{} {}", b.title, b.url).to_lowercase().contains(&q))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmarks_are_newest_first_and_never_twice() {
        let mut s = BookmarksState::default();
        assert!(s.add("https://a.test/", "A"));
        assert!(s.add("https://b.test/", "Bee"));
        assert!(!s.add("https://a.test/", "again"));
        assert!(!s.add("", "none"));
        let urls: Vec<&str> = s.items.iter().map(|b| b.url.as_str()).collect();
        assert_eq!(urls, ["https://b.test/", "https://a.test/"]);
        assert_eq!(s.matching("bee").len(), 1);
        assert_eq!(s.matching("").len(), 2);
        s.remove("https://a.test/");
        assert!(!s.is_bookmarked("https://a.test/"));
    }

    #[test]
    fn a_bookmarks_file_with_bad_entries_keeps_the_good_ones() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("bookmarks.json"),
            r#"{"version":1,"data":{"items":[{"id":"b1","url":"https://a.test/","title":"A","added":1},{"id":"b2","url":"","title":"","added":2},{"url":7},null]}}"#,
        )
        .unwrap();
        let store = open(dir.path());
        assert!(store.load_error().is_none());
        assert_eq!(store.get().items.len(), 1);
        assert_eq!(store.get().items[0].title, "A");
    }
}
