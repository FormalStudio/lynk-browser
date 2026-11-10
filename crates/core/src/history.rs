//! Every visit, in the 0.5.11 app's `history.sqlite` (`visits(id, url, title, ts)`), read and
//! written in place. `ts` is milliseconds since the Unix epoch.

use std::path::Path;

use nd_data::{Database, OpenOptions, SqliteExecutor, Value};
use serde::Deserialize;

use crate::now_ms;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Visit {
    pub url: String,
    pub title: String,
    pub ts: i64,
}

#[derive(Clone)]
pub struct History {
    db: Database,
}

impl History {
    pub async fn open(path: &Path) -> nd_data::Result<Self> {
        Self::with(Database::open(OpenOptions::file(path)).await?).await
    }

    pub async fn open_memory() -> nd_data::Result<Self> {
        Self::with(Database::open_memory().await?).await
    }

    async fn with(db: Database) -> nd_data::Result<Self> {
        db.mutate(
            "CREATE TABLE IF NOT EXISTS visits (id INTEGER PRIMARY KEY, url TEXT NOT NULL, title TEXT NOT NULL DEFAULT '', ts INTEGER NOT NULL)",
            Vec::new(),
        )
        .await?;
        Ok(Self { db })
    }

    pub async fn record_visit(&self, url: &str, title: &str) -> nd_data::Result<()> {
        self.record_visit_at(url, title, now_ms()).await
    }

    pub async fn record_visit_at(&self, url: &str, title: &str, ts: i64) -> nd_data::Result<()> {
        if url.is_empty() {
            return Ok(());
        }
        self.db
            .mutate("INSERT INTO visits (url, title, ts) VALUES (?, ?, ?)", vec![url.into(), title.into(), ts.into()])
            .await?;
        Ok(())
    }

    /// Titles arrive after the navigation, so the newest row for the address is backfilled.
    pub async fn record_title(&self, url: &str, title: &str) -> nd_data::Result<()> {
        if url.is_empty() || title.is_empty() {
            return Ok(());
        }
        self.db
            .mutate(
                "UPDATE visits SET title = ? WHERE id = (SELECT id FROM visits WHERE url = ? ORDER BY id DESC LIMIT 1)",
                vec![title.into(), url.into()],
            )
            .await?;
        Ok(())
    }

    /// The command bar's history rows: newest first, one row per address. An empty query is
    /// the most recent visits.
    pub async fn search(&self, query: &str, limit: u32) -> nd_data::Result<Vec<Visit>> {
        let q = query.trim();
        if q.is_empty() {
            return self.recent(limit).await;
        }
        let like = format!("%{q}%");
        self.db
            .query_as(
                "SELECT url, title, MAX(ts) AS ts FROM visits WHERE url LIKE ? OR title LIKE ? GROUP BY url ORDER BY ts DESC LIMIT ?",
                vec![like.clone().into(), like.into(), limit.into()],
            )
            .await
    }

    /// Addresses the command bar may complete the text to, most visited first. Only the host
    /// is matched here; `omnibox::completion_for` picks the one to use.
    pub async fn completion_candidates(&self, query: &str, limit: u32) -> nd_data::Result<Vec<String>> {
        let q = query.trim().to_lowercase();
        if q.is_empty() || q.chars().any(char::is_whitespace) {
            return Ok(Vec::new());
        }
        let p: String =
            q.chars().flat_map(|c| if matches!(c, '\\' | '%' | '_') { vec!['\\', c] } else { vec![c] }).collect();
        let mut params: Vec<Value> = ["http://", "https://", "http://www.", "https://www."]
            .iter()
            .map(|scheme| Value::from(format!("{scheme}{p}%")))
            .collect();
        params.push(limit.into());
        let rows: Vec<UrlRow> = self
            .db
            .query_as(
                "SELECT url FROM visits WHERE url LIKE ? ESCAPE '\\' OR url LIKE ? ESCAPE '\\' \
                 OR url LIKE ? ESCAPE '\\' OR url LIKE ? ESCAPE '\\' \
                 GROUP BY url ORDER BY COUNT(*) DESC, MAX(ts) DESC LIMIT ?",
                params,
            )
            .await?;
        Ok(rows.into_iter().map(|r| r.url).collect())
    }

    /// Newest first, one row per address.
    pub async fn recent(&self, limit: u32) -> nd_data::Result<Vec<Visit>> {
        self.db
            .query_as(
                "SELECT url, title, MAX(ts) AS ts FROM visits GROUP BY url ORDER BY ts DESC LIMIT ?",
                vec![limit.into()],
            )
            .await
    }

    /// The History panel: newest first, one row per address, matching the address or the title.
    pub async fn matching(&self, query: &str, limit: u32) -> nd_data::Result<Vec<Visit>> {
        let like = format!("%{}%", query.trim());
        self.db
            .query_as(
                "SELECT url, MAX(title) AS title, MAX(ts) AS ts FROM visits WHERE url LIKE ? OR title LIKE ? GROUP BY url ORDER BY ts DESC LIMIT ?",
                vec![like.clone().into(), like.into(), limit.into()],
            )
            .await
    }

    /// Every visit to one address.
    pub async fn forget(&self, url: &str) -> nd_data::Result<()> {
        self.db.mutate("DELETE FROM visits WHERE url = ?", vec![url.into()]).await?;
        Ok(())
    }

    pub async fn clear(&self) -> nd_data::Result<()> {
        self.db.mutate("DELETE FROM visits", Vec::new()).await?;
        Ok(())
    }

    pub async fn close(self) -> nd_data::Result<()> {
        self.db.close().await
    }
}

#[derive(Deserialize)]
struct UrlRow {
    url: String,
}

#[cfg(test)]
mod tests {
    use futures_executor::block_on;

    use super::*;

    #[test]
    fn visits_are_one_row_per_address_newest_first() {
        block_on(async {
            let h = History::open_memory().await.unwrap();
            h.record_visit_at("https://a.test/", "", 1).await.unwrap();
            h.record_visit_at("https://b.test/", "B", 2).await.unwrap();
            h.record_visit_at("https://a.test/", "", 3).await.unwrap();
            h.record_visit_at("", "nothing", 4).await.unwrap();
            h.record_title("https://a.test/", "A").await.unwrap();
            let recent = h.recent(10).await.unwrap();
            let urls: Vec<&str> = recent.iter().map(|v| v.url.as_str()).collect();
            assert_eq!(urls, ["https://a.test/", "https://b.test/"]);
            assert_eq!(recent[0].ts, 3);
            assert_eq!(recent[0].title, "A");
            assert_eq!(h.search("b.te", 5).await.unwrap().len(), 1);
            assert_eq!(h.search("", 1).await.unwrap()[0].url, "https://a.test/");
            assert_eq!(h.matching("", 500).await.unwrap()[0].title, "A");
            h.forget("https://a.test/").await.unwrap();
            assert_eq!(h.recent(10).await.unwrap().len(), 1);
            h.clear().await.unwrap();
            assert!(h.recent(10).await.unwrap().is_empty());
        });
    }

    #[test]
    fn completion_candidates_match_the_host_most_visited_first() {
        block_on(async {
            let h = History::open_memory().await.unwrap();
            for (url, ts) in [
                ("https://www.github.com/x", 1),
                ("https://github.com/y", 2),
                ("https://github.com/y", 3),
                ("https://gitlab.com/", 4),
            ] {
                h.record_visit_at(url, "", ts).await.unwrap();
            }
            h.record_visit_at("https://a.test/?q=git", "", 5).await.unwrap();
            assert_eq!(
                h.completion_candidates("GITH", 8).await.unwrap(),
                ["https://github.com/y", "https://www.github.com/x"]
            );
            assert_eq!(h.completion_candidates("git", 1).await.unwrap(), ["https://github.com/y"]);
            assert!(h.completion_candidates("git hub", 8).await.unwrap().is_empty());
            assert!(h.completion_candidates("g_t", 8).await.unwrap().is_empty());
        });
    }

    #[test]
    fn an_old_history_file_is_read_and_written_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        {
            let conn = nd_data::rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE visits (id INTEGER PRIMARY KEY, url TEXT NOT NULL, title TEXT NOT NULL DEFAULT '', ts INTEGER NOT NULL);
                 INSERT INTO visits (url, title, ts) VALUES ('https://old.test/', 'Old', 1700000000000);",
            )
            .unwrap();
        }
        block_on(async {
            let h = History::open(&path).await.unwrap();
            assert_eq!(h.recent(10).await.unwrap()[0].title, "Old");
            h.record_visit("https://new.test/", "New").await.unwrap();
            h.close().await.unwrap();
        });
        let conn = nd_data::rusqlite::Connection::open(&path).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM visits", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 2);
    }
}
