//! History: every page a tab commits goes into `history.sqlite` in the profile through
//! `lynk_core::history`, its title backfilled when the page names itself. The recorder hears pages
//! through `page::OBSERVERS`; the command bar reads matches through [`search`]. It must not rank
//! or draw.

use lynk_core::history::{History, Visit};
use nd::prelude::*;

use crate::page::PageEvent;
use crate::tab::TabId;
use crate::window::BrowserWindow;

struct Store(History);

/// The profile's history, opened on first use. Opening is one SQLite file on nd-data's own
/// thread, waited for once; `None` when the file cannot be opened, and nothing is kept.
fn store(cx: &mut App) -> Option<History> {
    if let Some(Store(history)) = cx.try_global::<Store>() {
        return Some(history.clone());
    }
    let path = nd::system::app_dirs().ensure_data().ok()?.join("history.sqlite");
    match crate::process::block_on(History::open(&path)) {
        Ok(history) => {
            cx.set_global(Store(history.clone()));
            Some(history)
        }
        Err(error) => {
            eprintln!("lynk: history: {error}");
            None
        }
    }
}

/// Records each committed page and its title as the tab's page reports them.
pub fn record(win: &mut BrowserWindow, id: TabId, event: &PageEvent, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let title = match event {
        PageEvent::Navigated(_) => None,
        PageEvent::TitleChanged(e) => Some(e.title.clone()),
        _ => return,
    };
    let Some(url) = win.tab(id).map(|t| t.url.clone()) else { return };
    let Some(history) = store(cx) else { return };
    nd::system::spawn_detached(async move {
        let done = match title {
            None => history.record_visit(&url, "").await,
            Some(title) => history.record_title(&url, &title).await,
        };
        if let Err(error) = done {
            eprintln!("lynk: history: {error}");
        }
    });
}

/// The pages matching `query`, newest first, for the command bar; `done` runs on the main
/// thread.
pub fn search(cx: &mut App, query: String, done: impl FnOnce(Vec<Visit>, &mut App) + 'static) {
    let Some(history) = store(cx) else { return };
    nd::system::spawn(async move { history.search(&query, LIMIT).await.unwrap_or_default() }, done);
}

const LIMIT: u32 = 8;
