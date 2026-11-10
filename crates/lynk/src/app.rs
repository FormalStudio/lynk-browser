//! What every window shares: the windows themselves, tab ids, the closed-tab stack and the zoom
//! levels kept per host. Windows open and close through here.

use std::collections::BTreeMap;

use lynk_core::session::{WINDOW_HEIGHT, WINDOW_WIDTH};
use nd::core::WindowId;
use nd::prelude::*;

use crate::actions;
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
    closed: Vec<String>,
    pub zoom_by_host: BTreeMap<String, f64>,
    menus: Vec<(SharedString, bool)>,
}

pub fn install(cx: &mut App) {
    cx.set_global(Browser::default());
}

pub fn next_tab_id(cx: &mut App) -> TabId {
    let browser = cx.global_mut::<Browser>();
    browser.next_tab += 1;
    browser.next_tab
}

/// A tab's address goes on the closed stack, unless it never had one.
pub fn remember_closed(cx: &mut App, tab: &Tab) {
    if tab.url.is_empty() {
        return;
    }
    let closed = &mut cx.global_mut::<Browser>().closed;
    closed.push(tab.url.clone());
    if closed.len() > CLOSED_LIMIT {
        closed.remove(0);
    }
}

pub fn pop_closed(cx: &mut App) -> Option<String> {
    cx.global_mut::<Browser>().closed.pop()
}

pub fn zoom_for(cx: &App, url: &str) -> f64 {
    lynk_core::zoom::zoom_for(&cx.global::<Browser>().zoom_by_host, url)
}

pub fn remember_zoom(cx: &mut App, url: &str, factor: f64) {
    lynk_core::zoom::remember_zoom(&mut cx.global_mut::<Browser>().zoom_by_host, url, factor);
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

/// The key browser window, else the newest: the one the menu bar and a forwarded launch act on.
pub fn key_window(cx: &App) -> Option<WindowId> {
    let browser = cx.global::<Browser>();
    cx.active_window()
        .filter(|id| browser.windows.iter().any(|(w, _)| w == id))
        .or_else(|| browser.windows.last().map(|(w, _)| *w))
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

/// Opens a window on `tabs` showing `active`; no tabs means one new tab page.
pub fn open_window(cx: &mut App, mut tabs: Vec<Tab>, active: Option<TabId>) -> WindowId {
    if tabs.is_empty() {
        tabs.push(Tab::new(next_tab_id(cx), ""));
    }
    let browser = cx.global_mut::<Browser>();
    browser.next_window += 1;
    let options = WindowOptions::main(format!("browser-{}", browser.next_window))
        .title("Lynk Browser")
        .size(size(px(WINDOW_WIDTH as f32), px(WINDOW_HEIGHT as f32)))
        .min_size(size(px(MIN_WIDTH), px(MIN_HEIGHT)))
        .titlebar(Titlebar::Custom);
    let handle = cx.open_window(options, move |window, cx| {
        let id = window.id();
        window.on_should_close(move |_, cx| {
            self::window(cx, id).is_none_or(|w| w.update(cx, |w, cx| w.should_close(cx)))
        });
        window.on_activate(|_, cx| refresh_menus(cx));
        cx.new(|cx| BrowserWindow::new(id, tabs, active, cx))
    });
    let id = handle.id();
    let entity = handle.root().clone();
    cx.global_mut::<Browser>().windows.push((id, entity));
    id
}
