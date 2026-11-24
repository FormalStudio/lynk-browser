//! `session.json` in the profile: the windows open, their frames, their tabs in sidebar order,
//! the tab each shows and where each page was scrolled, as core's session (version 3). Every
//! change schedules a write of the whole session, which nd's store debounces and writes through
//! a temp file and a rename, so a crash or a `kill -9` leaves the last complete session. The
//! write that is still pending goes out as the app quits.
//!
//! At launch the session comes back with every tab asleep until it is shown: only the tab each
//! window shows loads. A file that does not read, or comes from a newer version, is set aside by
//! the store and the browser starts fresh. Chromium has no way to hand a page its back and
//! forward list, so a restored tab starts its history at the restored address.

use std::time::Duration;

use lynk_core::session::{self as stored, SessionState, SessionTab, SessionWindow, tab_id, tab_number};
use lynk_core::sidebar_tree::SidebarTree;
use lynk_core::spaces::WindowSpaces;
use nd::prelude::*;
use nd::settings::Store;

use crate::app;
use crate::tab::{Tab, TabId};

/// The session store and whether a write is already on its way.
struct Session {
    store: Store<SessionState>,
    scheduled: bool,
    /// Set while windows come back, so a half-restored session is never written.
    restoring: bool,
}

/// A window's place on screen: its top-left corner in global points (none when the OS placed
/// it and never said where) and its content size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub origin: Option<(f64, f64)>,
    pub width: f64,
    pub height: f64,
}

/// One window as the session keeps it, ready to open.
pub struct Restored {
    pub tabs: Vec<Tab>,
    pub tree: SidebarTree,
    pub active: Option<TabId>,
    pub frame: Frame,
}

/// Opens `dir`'s session and returns its windows, or none when it holds nothing but new tab
/// pages (a first launch, or a profile whose last window showed only the new tab page). From
/// here on every change is written back.
pub fn open(cx: &mut App, dir: &std::path::Path) -> Vec<Restored> {
    let store = stored::open(dir);
    if let Some(why) = store.load_error() {
        eprintln!("lynk: session.json was set aside and the session starts fresh: {why}");
    }
    let state = store.get();
    let browser = cx.global_mut::<app::Browser>();
    browser.zoom_by_host = state.zoom_by_host.clone();
    browser.raise_tab_ids(state.next_tab_id.saturating_sub(1));
    let windows = if state.windows.iter().flat_map(|w| &w.tabs).all(|t| t.url.is_empty()) {
        Vec::new()
    } else {
        state.windows.iter().map(restored).collect()
    };
    cx.set_global(Session { store, scheduled: false, restoring: true });
    cx.on_quit(|cx| {
        write(cx);
        if let Some(Err(e)) = cx.try_global::<Session>().map(|s| s.store.flush()) {
            eprintln!("lynk: session.json was not written as the app quit: {e}");
        }
    });
    windows
}

/// The windows are open: changes from here on are the user's and are written.
pub fn restored_all(cx: &mut App) {
    if let Some(session) = cx.try_global_mut::<Session>() {
        session.restoring = false;
    }
    changed(cx);
}

fn restored(w: &SessionWindow) -> Restored {
    let tabs: Vec<Tab> = w
        .tabs
        .iter()
        .filter_map(|t| {
            let mut tab = Tab::new(tab_number(&t.id)?, t.url.clone());
            tab.title = t.title.clone();
            tab.scroll = t.scroll.map(|[x, y]| (x, y)).filter(|s| *s != (0., 0.));
            Some(tab)
        })
        .collect();
    Restored {
        tabs,
        tree: w.spaces.tree().cloned().unwrap_or_default(),
        active: tab_number(&w.active_id),
        frame: Frame { origin: w.x.zip(w.y), width: w.width, height: w.height },
    }
}

/// Something the session keeps changed: a write goes out once the current update is over.
pub fn changed(cx: &mut App) {
    let Some(session) = cx.try_global_mut::<Session>() else { return };
    if session.scheduled || session.restoring {
        return;
    }
    session.scheduled = true;
    nd::system::after(Duration::ZERO, |cx| {
        if let Some(session) = cx.try_global_mut::<Session>() {
            session.scheduled = false;
        }
        write(cx);
    });
}

/// Hands the store the session as it is now. Windows still registered are kept, the last one
/// to close included: a relaunch brings back what was open as the app went.
fn write(cx: &mut App) {
    let Some(session) = cx.try_global::<Session>() else { return };
    if session.restoring {
        return;
    }
    let windows: Vec<SessionWindow> = app::windows(cx)
        .iter()
        .enumerate()
        .map(|(n, (_, w))| {
            let w = w.read(cx);
            let frame = w.frame;
            let tabs = w
                .tabs()
                .into_iter()
                .map(|t| SessionTab {
                    id: tab_id(t.id),
                    url: t.url.clone(),
                    title: t.title.clone(),
                    scroll: t.scroll.map(|(x, y)| [x, y]),
                    ..SessionTab::default()
                })
                .collect();
            SessionWindow {
                id: format!("w{}", n + 1),
                tabs,
                active_id: tab_id(w.active),
                width: frame.width,
                height: frame.height,
                x: frame.origin.map(|(x, _)| x),
                y: frame.origin.map(|(_, y)| y),
                ..SessionWindow::default()
            }
        })
        .collect();
    if windows.is_empty() {
        return;
    }
    let browser = cx.global::<app::Browser>();
    let (next_tab, zoom) = (browser.last_tab_id() + 1, browser.zoom_by_host.clone());
    let trees: Vec<SidebarTree> = app::windows(cx).iter().map(|(_, w)| w.read(cx).tree().clone()).collect();
    let session = cx.global::<Session>();
    session.store.update(|s| {
        let space = s.spaces.first().map(|s| s.id.clone()).unwrap_or_default();
        // What the shell does not track yet (a pin's page, fields a newer build added) is kept
        // from the tab as it was stored.
        let kept: std::collections::HashMap<String, SessionTab> =
            s.windows.iter().flat_map(|w| &w.tabs).map(|t| (t.id.clone(), t.clone())).collect();
        let windows = windows
            .into_iter()
            .zip(trees)
            .map(|(mut w, tree)| {
                for t in &mut w.tabs {
                    if let Some(old) = kept.get(&t.id) {
                        t.pinned_url = old.pinned_url.clone();
                        t.unknown = old.unknown.clone();
                    }
                }
                SessionWindow { spaces: WindowSpaces::single(&space, tree), ..w }
            })
            .collect();
        *s = stored::normalize(&SessionState {
            windows,
            next_tab_id: next_tab,
            next_window_id: s.next_window_id,
            zoom_by_host: zoom,
            ..s.clone()
        });
    });
}
