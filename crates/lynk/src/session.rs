//! `session.json` in each profile: the profile's windows, their frames, their tabs in sidebar
//! order, the tab each shows and where each page was scrolled, as core's session (version 3).
//! Each profile with windows open has its own store, written from its own windows only. Every
//! change schedules a write of the whole session, which nd's store debounces and writes through
//! a temp file and a rename, so a crash or a `kill -9` leaves the last complete session. The
//! write that is still pending goes out as the app quits.
//!
//! At launch the session comes back with every tab asleep until it is shown: only the tab each
//! window shows loads. Pins come back in their tree with the page each was pinned at. A file
//! that does not read, or comes from a newer version, is set aside by the store and the browser
//! starts fresh. Chromium has no way to hand a page its back and forward list, so a restored tab
//! starts its history at the restored address.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use lynk_core::session::{self as stored, SessionState, SessionTab, SessionWindow, tab_id, tab_number};
use lynk_core::sidebar_tree::SidebarTree;
use lynk_core::spaces::WindowSpaces;
use nd::prelude::*;
use nd::settings::Store;

use crate::app;
use crate::tab::{Tab, TabId};
use crate::window::BrowserWindow;

/// Each open profile's store and whether a write is already on its way.
#[derive(Default)]
struct Sessions {
    stores: HashMap<String, Store<SessionState>>,
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
    pub profile: String,
    pub tabs: Vec<Tab>,
    pub tree: SidebarTree,
    pub active: Option<TabId>,
    pub frame: Frame,
}

/// Starts keeping sessions: from here on every change is written back as the app quits too.
pub fn install(cx: &mut App) {
    cx.set_global(Sessions { restoring: true, ..Sessions::default() });
    cx.on_quit(|cx| {
        write(cx);
        let Some(sessions) = cx.try_global::<Sessions>() else { return };
        for (profile, store) in &sessions.stores {
            if let Err(e) = store.flush() {
                eprintln!("lynk: session.json of profile {profile} was not written as the app quit: {e}");
            }
        }
    });
}

/// Tab ids are unique across every profile, so a launch starts past the last one any profile's
/// session handed out, open or not.
pub fn raise_tab_ids(cx: &mut App, dir: &Path) {
    let next = stored::open(dir).get().next_tab_id;
    cx.global_mut::<app::Browser>().raise_tab_ids(next.saturating_sub(1));
}

/// Opens `profile`'s session in `dir` and returns its windows, or none when it holds nothing
/// but new tab pages (a first launch, or a profile whose last window showed only the new tab
/// page).
pub fn open(cx: &mut App, profile: &str, dir: &Path) -> Vec<Restored> {
    let store = stored::open(dir);
    if let Some(why) = store.load_error() {
        eprintln!("lynk: session.json of profile {profile} was set aside and starts fresh: {why}");
    }
    let state = store.get();
    let browser = cx.global_mut::<app::Browser>();
    browser.zoom_by_host.insert(profile.to_owned(), state.zoom_by_host.clone());
    browser.raise_tab_ids(state.next_tab_id.saturating_sub(1));
    let windows = if state.windows.iter().flat_map(|w| &w.tabs).all(|t| t.url.is_empty()) {
        Vec::new()
    } else {
        state.windows.iter().map(|w| restored(profile, w)).collect()
    };
    cx.global_mut::<Sessions>().stores.insert(profile.to_owned(), store);
    windows
}

/// `profile`'s last window is going while others stay: its session is written with that window
/// in it, for the next time the profile opens, and the profile is put away.
pub fn close_profile(cx: &mut App, last: &BrowserWindow) {
    let Some(store) = cx.global_mut::<Sessions>().stores.remove(&last.profile) else { return };
    let next_tab = cx.global::<app::Browser>().last_tab_id() + 1;
    let zoom = cx.global::<app::Browser>().zoom_by_host.get(&last.profile).cloned().unwrap_or_default();
    store.update(|s| *s = state_of(s, vec![record(last, 0)], next_tab, zoom));
    if let Err(e) = store.flush() {
        eprintln!("lynk: session.json of profile {} was not written: {e}", last.profile);
    }
}

/// A deleted profile's session is dropped unwritten.
pub fn forget(cx: &mut App, profile: &str) {
    cx.global_mut::<Sessions>().stores.remove(profile);
}

/// The windows are open: changes from here on are the user's and are written.
pub fn restored_all(cx: &mut App) {
    if let Some(sessions) = cx.try_global_mut::<Sessions>() {
        sessions.restoring = false;
    }
    changed(cx);
}

fn restored(profile: &str, w: &SessionWindow) -> Restored {
    let tabs: Vec<Tab> = w
        .tabs
        .iter()
        .filter_map(|t| {
            let mut tab = Tab::new(tab_number(&t.id)?, t.url.clone());
            tab.title = t.title.clone();
            tab.scroll = t.scroll.map(|[x, y]| (x, y)).filter(|s| *s != (0., 0.));
            tab.pinned_url = t.pinned_url.clone();
            Some(tab)
        })
        .collect();
    Restored {
        profile: profile.to_owned(),
        tabs,
        tree: w.spaces.tree().cloned().unwrap_or_default(),
        active: tab_number(&w.active_id),
        frame: Frame { origin: w.x.zip(w.y), width: w.width, height: w.height },
    }
}

/// Something the session keeps changed: a write goes out once the current update is over.
pub fn changed(cx: &mut App) {
    let Some(sessions) = cx.try_global_mut::<Sessions>() else { return };
    if sessions.scheduled || sessions.restoring {
        return;
    }
    sessions.scheduled = true;
    nd::system::after(Duration::ZERO, |cx| {
        if let Some(sessions) = cx.try_global_mut::<Sessions>() {
            sessions.scheduled = false;
        }
        write(cx);
    });
}

/// One window as the session keeps it, with its tree, numbered `n` among its profile's.
fn record(w: &BrowserWindow, n: usize) -> (SessionWindow, SidebarTree) {
    let frame = w.frame;
    let tabs = w
        .tabs()
        .into_iter()
        .map(|t| SessionTab {
            id: tab_id(t.id),
            url: t.url.clone(),
            title: t.title.clone(),
            scroll: t.scroll.map(|(x, y)| [x, y]),
            pinned_url: t.pinned_url.clone().filter(|_| w.is_pinned(t.id)),
            ..SessionTab::default()
        })
        .collect();
    let window = SessionWindow {
        id: format!("w{}", n + 1),
        tabs,
        active_id: tab_id(w.active),
        width: frame.width,
        height: frame.height,
        x: frame.origin.map(|(x, _)| x),
        y: frame.origin.map(|(_, y)| y),
        ..SessionWindow::default()
    };
    (window, w.tree().clone())
}

/// `old` with `windows` in place of its own.
fn state_of(
    old: &SessionState,
    windows: Vec<(SessionWindow, SidebarTree)>,
    next_tab_id: u64,
    zoom_by_host: std::collections::BTreeMap<String, f64>,
) -> SessionState {
    let space = old.spaces.first().map(|s| s.id.clone()).unwrap_or_default();
    // Fields a newer build added are kept from the tab as it was stored.
    let kept: HashMap<&str, &SessionTab> =
        old.windows.iter().flat_map(|w| &w.tabs).map(|t| (t.id.as_str(), t)).collect();
    let windows = windows
        .into_iter()
        .map(|(mut w, tree)| {
            for t in &mut w.tabs {
                if let Some(old) = kept.get(t.id.as_str()) {
                    t.unknown = old.unknown.clone();
                }
            }
            SessionWindow { spaces: WindowSpaces::single(&space, tree), ..w }
        })
        .collect();
    stored::normalize(&SessionState {
        windows,
        next_tab_id,
        next_window_id: old.next_window_id,
        zoom_by_host,
        ..old.clone()
    })
}

/// Hands each open profile's store the session as it is now. Windows still registered are kept,
/// the last one to close included: a relaunch brings back what was open as the app went. A
/// profile with no window registered keeps the file it has.
fn write(cx: &mut App) {
    let Some(sessions) = cx.try_global::<Sessions>() else { return };
    if sessions.restoring {
        return;
    }
    let profiles: Vec<String> = sessions.stores.keys().cloned().collect();
    let next_tab = cx.global::<app::Browser>().last_tab_id() + 1;
    for profile in profiles {
        let windows: Vec<(SessionWindow, SidebarTree)> =
            app::windows_of(cx, &profile).iter().enumerate().map(|(n, (_, w))| record(w.read(cx), n)).collect();
        if windows.is_empty() {
            continue;
        }
        let zoom = cx.global::<app::Browser>().zoom_by_host.get(&profile).cloned().unwrap_or_default();
        if let Some(store) = cx.global::<Sessions>().stores.get(&profile) {
            store.update(|s| *s = state_of(s, windows, next_tab, zoom));
        }
    }
}
