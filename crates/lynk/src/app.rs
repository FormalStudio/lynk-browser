//! What every window shares: the windows themselves and tab ids, and per profile the closed-tab
//! stack and the zoom levels kept per host. Windows open and close through here.

use std::collections::{BTreeMap, HashMap};

use lynk_core::session::{WINDOW_HEIGHT, WINDOW_WIDTH};
use lynk_core::sidebar_tree::SidebarTree;
use nd::core::WindowId;
use nd::prelude::*;

use crate::actions;
use crate::session::{Frame, Restored};
use crate::tab::{Tab, TabId};
use crate::window::BrowserWindow;

/// How many closed tabs Shift+Cmd+T can bring back.
const CLOSED_LIMIT: usize = 25;
pub const MIN_WIDTH: f32 = 720.;
pub const MIN_HEIGHT: f32 = 480.;

#[derive(Default)]
pub struct Browser {
    windows: Vec<(WindowId, Entity<BrowserWindow>)>,
    next_tab: TabId,
    next_window: u64,
    /// Addresses of closed tabs with the profile each was in, newest last.
    closed: Vec<(String, String)>,
    /// Zoom levels per host, per profile.
    pub zoom_by_host: HashMap<String, BTreeMap<String, f64>>,
    menus: Vec<(SharedString, bool)>,
}

pub fn install(cx: &mut App) {
    cx.set_global(Browser::default());
}

impl Browser {
    /// Tab ids from here on start past `last`, which a restored session already handed out.
    pub fn raise_tab_ids(&mut self, last: TabId) {
        self.next_tab = self.next_tab.max(last);
    }

    pub fn last_tab_id(&self) -> TabId {
        self.next_tab
    }
}

pub fn next_tab_id(cx: &mut App) -> TabId {
    let browser = cx.global_mut::<Browser>();
    browser.next_tab += 1;
    browser.next_tab
}

/// A tab's address goes on its profile's closed stack, unless it never had one.
pub fn remember_closed(cx: &mut App, profile: &str, tab: &Tab) {
    if tab.url.is_empty() {
        return;
    }
    let closed = &mut cx.global_mut::<Browser>().closed;
    closed.push((profile.to_owned(), tab.url.clone()));
    if closed.iter().filter(|(p, _)| p == profile).count() > CLOSED_LIMIT
        && let Some(oldest) = closed.iter().position(|(p, _)| p == profile)
    {
        closed.remove(oldest);
    }
}

/// The last tab closed in `profile`.
pub fn pop_closed(cx: &mut App, profile: &str) -> Option<String> {
    let closed = &mut cx.global_mut::<Browser>().closed;
    let at = closed.iter().rposition(|(p, _)| p == profile)?;
    Some(closed.remove(at).1)
}

/// What a deleted profile leaves behind in memory goes with it.
pub fn forget_profile(cx: &mut App, profile: &str) {
    let browser = cx.global_mut::<Browser>();
    browser.closed.retain(|(p, _)| p != profile);
    browser.zoom_by_host.remove(profile);
}

pub fn zoom_for(cx: &App, profile: &str, url: &str) -> f64 {
    cx.global::<Browser>().zoom_by_host.get(profile).map_or(1., |levels| lynk_core::zoom::zoom_for(levels, url))
}

pub fn remember_zoom(cx: &mut App, profile: &str, url: &str, factor: f64) {
    let levels = cx.global_mut::<Browser>().zoom_by_host.entry(profile.to_owned()).or_default();
    lynk_core::zoom::remember_zoom(levels, url, factor);
}

/// The windows of `profile`, oldest first.
pub fn windows_of(cx: &App, profile: &str) -> Vec<(WindowId, Entity<BrowserWindow>)> {
    windows(cx).into_iter().filter(|(_, w)| w.read(cx).profile == profile).collect()
}

pub fn windows(cx: &App) -> Vec<(WindowId, Entity<BrowserWindow>)> {
    cx.global::<Browser>().windows.clone()
}

pub fn window(cx: &App, id: WindowId) -> Option<Entity<BrowserWindow>> {
    cx.global::<Browser>().windows.iter().find(|(w, _)| *w == id).map(|(_, e)| e.clone())
}

/// The menu bar lives in the first window; every other one carries its own menu.
pub fn is_first(cx: &App, id: WindowId) -> bool {
    cx.global::<Browser>().windows.first().is_some_and(|(w, _)| *w == id)
}

pub fn forget_window(cx: &mut App, id: WindowId) {
    cx.global_mut::<Browser>().windows.retain(|(w, _)| *w != id);
}

/// The key browser window, else the newest open one: the one the menu bar and a forwarded
/// launch act on. The last window to close stays registered until the app terminates.
pub fn key_window(cx: &App) -> Option<WindowId> {
    let browser = cx.global::<Browser>();
    let open = |id: &WindowId| cx.window_ids().any(|w| w == *id);
    cx.active_window()
        .filter(|id| browser.windows.iter().any(|(w, _)| w == id))
        .or_else(|| browser.windows.iter().rev().map(|(w, _)| *w).find(open))
}

/// Rebuilds the menu bar from the window it acts on. Called where no window is being updated
/// (a window becoming key, startup); a window that changed calls [`refresh_menus_for`].
pub fn refresh_menus(cx: &mut App) {
    let Some(win) = key_window(cx).and_then(|id| window(cx, id)) else { return };
    let menus = actions::menus(win.read(cx), cx);
    set_menus_if_changed(menus, cx);
}

/// Rebuilds the menu bar from `win`, when it is the window the menu bar acts on.
pub fn refresh_menus_for(win: &BrowserWindow, cx: &mut App) {
    if key_window(cx) == Some(win.id) {
        let menus = actions::menus(win, cx);
        set_menus_if_changed(menus, cx);
    }
}

fn set_menus_if_changed(menus: Vec<Menu>, cx: &mut App) {
    let signature = actions::signature(&menus);
    if cx.global::<Browser>().menus == signature {
        return;
    }
    cx.global_mut::<Browser>().menus = signature;
    set_menus(cx, menus);
}

/// Opens a window on new `tabs` showing `active` in the key window's profile, else `Default`;
/// no tabs means one new tab page.
pub fn open_window(cx: &mut App, tabs: Vec<Tab>, active: Option<TabId>) -> WindowId {
    let profile = key_window(cx)
        .and_then(|id| window(cx, id))
        .map_or_else(|| lynk_core::profiles::DEFAULT_ID.to_owned(), |w| w.read(cx).profile.clone());
    open_window_in(cx, profile, tabs, active)
}

/// Opens a window of `profile` on new `tabs` showing `active`; no tabs means one new tab page.
pub fn open_window_in(cx: &mut App, profile: String, mut tabs: Vec<Tab>, active: Option<TabId>) -> WindowId {
    if tabs.is_empty() {
        tabs.push(Tab::new(next_tab_id(cx), ""));
    }
    open(cx, profile, tabs, active, false, None)
}

/// Opens a window the session kept, where it was and as big, with its tabs in its order.
pub fn open_restored(cx: &mut App, restored: Restored) -> WindowId {
    let Restored { profile, tabs, tree, active, frame } = restored;
    open(cx, profile, tabs, active, false, Some((tree, frame)))
}

/// Opens a window of `profile` around `tab`, moved out of another of its windows.
pub fn open_window_around(cx: &mut App, profile: String, tab: Tab) -> WindowId {
    let active = Some(tab.id);
    open(cx, profile, vec![tab], active, true, None)
}

fn open(
    cx: &mut App,
    profile: String,
    tabs: Vec<Tab>,
    active: Option<TabId>,
    moved: bool,
    kept: Option<(SidebarTree, Frame)>,
) -> WindowId {
    let (tree, frame) = kept.map_or((None, None), |(tree, frame)| (Some(tree), Some(frame)));
    let frame = frame.unwrap_or(Frame { origin: None, width: WINDOW_WIDTH, height: WINDOW_HEIGHT });
    let browser = cx.global_mut::<Browser>();
    browser.next_window += 1;
    let mut options = WindowOptions::main(format!("browser-{}", browser.next_window))
        .title("Lynk Browser")
        .size(size(px((frame.width as f32).max(MIN_WIDTH)), px((frame.height as f32).max(MIN_HEIGHT))))
        .min_size(size(px(MIN_WIDTH), px(MIN_HEIGHT)))
        .titlebar(Titlebar::Custom);
    if let Some((x, y)) = frame.origin {
        options = options.origin(nd::core::Point { x: x as f32, y: y as f32 });
    }
    let handle = cx.open_window(options, move |window, cx| {
        let id = window.id();
        window.on_should_close(move |_, cx| {
            self::window(cx, id).is_none_or(|w| w.update(cx, |w, cx| w.should_close(cx)))
        });
        window.on_activate(|_, cx| refresh_menus(cx));
        window.on_frame_changed(move |window, cx| {
            let origin = window.origin().map(|o| (f64::from(o.x), f64::from(o.y)));
            let size = window.viewport_size();
            let frame = Frame { origin, width: f64::from(size.width), height: f64::from(size.height) };
            if let Some(w) = self::window(cx, id) {
                // The pinned tiles take their column count from the window's width.
                w.update(cx, |w, cx| {
                    w.frame = frame;
                    cx.notify();
                });
            }
            crate::session::changed(cx);
        });
        cx.new(|cx| BrowserWindow::new(id, profile, tabs, active, moved, (tree, frame), cx))
    });
    let id = handle.id();
    let entity = handle.root().clone();
    cx.global_mut::<Browser>().windows.push((id, entity));
    id
}
