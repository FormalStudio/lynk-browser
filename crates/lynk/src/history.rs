//! History: every page a tab commits goes into its profile's `history.sqlite` through
//! `lynk_core::history`, its title backfilled when the page names itself. The recorder hears pages
//! through `page::OBSERVERS`; the command bar reads matches through [`search`]. It must not rank
//! or draw.

use std::collections::HashMap;

use lynk_core::history::{History, Visit};
use nd::prelude::*;

use crate::page::PageEvent;
use crate::tab::TabId;
use crate::window::BrowserWindow;

#[derive(Default)]
struct Stores(HashMap<String, History>);

/// `profile`'s history, opened on first use. Opening is one SQLite file on nd-data's own
/// thread, waited for once; `None` when the file cannot be opened, and nothing is kept.
fn store(cx: &mut App, profile: &str) -> Option<History> {
    if let Some(history) = cx.try_global::<Stores>().and_then(|s| s.0.get(profile)) {
        return Some(history.clone());
    }
    let path = crate::profile::data_dir(cx, profile).join("history.sqlite");
    match crate::process::block_on(History::open(&path)) {
        Ok(history) => {
            if !cx.has_global::<Stores>() {
                cx.set_global(Stores::default());
            }
            cx.global_mut::<Stores>().0.insert(profile.to_owned(), history.clone());
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
    let profile = win.profile.clone();
    let Some(history) = store(cx, &profile) else { return };
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
pub fn search(cx: &mut App, profile: &str, query: String, done: impl FnOnce(Vec<Visit>, &mut App) + 'static) {
    let Some(history) = store(cx, profile) else { return };
    nd::system::spawn(async move { history.search(&query, LIMIT).await.unwrap_or_default() }, done);
}

const LIMIT: u32 = 8;
