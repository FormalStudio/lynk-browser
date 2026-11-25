//! DevTools for a tab: docked in the window's page area, below or beside the page with a
//! divider the user drags, or in a window of their own, where that browser window last had them
//! (core's `devtools::Dock`). It keeps the DevTools page of every tab that has them open and
//! each window's dock; the shown tab's DevTools follow the dock, the others wait in the window's
//! holder like any page not on show. It asks a tab's page for DevTools and touches it no further.

use std::collections::HashMap;

use lynk_core::devtools::{Dock, Side};
use lynk_core::url::display_url;
use nd::components::split::Axis;
use nd::core::WindowId;
use nd::prelude::*;
use nd::webview::{ChromeCommand, FullscreenChanged, WebView, WebViewElement, WindowClosed, webview};

use crate::app;
use crate::tab::{Tab, TabId};
use crate::window::{BrowserWindow, TabEvent};

/// Neither side of a docked split goes below these, in points along the split.
const PAGE_MIN_WIDTH: f32 = 240.;
const PAGE_MIN_HEIGHT: f32 = 160.;
const TOOLS_MIN_WIDTH: f32 = 240.;
const TOOLS_MIN_HEIGHT: f32 = 120.;
/// The dock controls' bar over the DevTools, a sidebar row's height.
const BAR_HEIGHT: f32 = 28.;
const WINDOW_WIDTH: f32 = 1000.;
const WINDOW_HEIGHT: f32 = 700.;

/// One tab's open DevTools.
struct Open {
    /// The browser window of the tab.
    browser: WindowId,
    /// The page they inspect; a tab whose page was replaced or slept has them closed.
    inspected: WebView,
    tools: WebView,
    /// Their own window, while they are undocked.
    window: Option<WindowId>,
}

#[derive(Default)]
struct State {
    docks: HashMap<WindowId, Dock>,
    open: HashMap<TabId, Open>,
}

fn state(cx: &mut App) -> &mut State {
    if !cx.has_global::<State>() {
        cx.set_global(State::default());
    }
    cx.global_mut::<State>()
}

fn dock_of(cx: &App, win: WindowId) -> Dock {
    cx.try_global::<State>().and_then(|s| s.docks.get(&win)).copied().unwrap_or_default()
}

/// The DevTools of `tab` while its page is the one they inspect.
fn tools_of(cx: &App, tab: &Tab) -> Option<WebView> {
    let open = cx.try_global::<State>()?.open.get(&tab.id)?;
    (tab.view() == Some(&open.inspected)).then(|| open.tools.clone())
}

fn element_key(tab: TabId) -> SharedString {
    format!("devtools-{tab}").into()
}

/// The Developer Tools command: opens the tab's DevTools where the window keeps them, or closes
/// them.
pub fn toggle(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let open = win.tab(id).is_some_and(|t| tools_of(cx, t).is_some());
    if open {
        close(cx, id);
        win.changed(cx);
        return;
    }
    let Some(page) = win.tab(id).and_then(Tab::view).cloned() else { return };
    let Some(tools) = page.dev_tools() else { return };
    let browser = win.id;
    if let Some(old) = state(cx).open.insert(id, Open { browser, inspected: page, tools, window: None }) {
        close_window_of(cx, old);
    }
    follow_dock(win, cx);
    win.changed(cx);
}

/// Closes `tab`'s DevTools, docked or in their window.
fn close(cx: &mut App, tab: TabId) {
    if let Some(open) = state(cx).open.remove(&tab) {
        close_window_of(cx, open);
    }
}

fn close_window_of(cx: &mut App, open: Open) {
    if let Some(window) = open.window {
        cx.close_window(window);
    }
}

/// Puts the shown tab's DevTools where the window's dock says: into their own window when it
/// says so, back into the page area otherwise.
fn follow_dock(win: &BrowserWindow, cx: &mut App) {
    let side = dock_of(cx, win.id).side;
    let tabs: Vec<TabId> = win.order();
    let mut to_close = Vec::new();
    let mut to_open = None;
    if let Some(s) = cx.try_global::<State>() {
        for (tab, open) in s.open.iter().filter(|(t, _)| tabs.contains(t)) {
            match (side, open.window) {
                (Side::Window, None) if *tab == win.active => to_open = Some(*tab),
                (Side::Bottom | Side::Right, Some(w)) => to_close.push((*tab, w)),
                _ => {}
            }
        }
    }
    for (tab, w) in to_close {
        if let Some(open) = state(cx).open.get_mut(&tab) {
            open.window = None;
        }
        cx.close_window(w);
    }
    if let (Some(tab), Some(t)) = (to_open, win.active_tab()) {
        let title = format!("Developer Tools: {}", display_url(&t.url));
        open_window(cx, tab, title);
    }
}

fn open_window(cx: &mut App, tab: TabId, title: String) {
    let options = WindowOptions::main(format!("devtools-{tab}"))
        .title(title)
        .size(size(px(WINDOW_WIDTH), px(WINDOW_HEIGHT)))
        .min_size(size(px(TOOLS_MIN_WIDTH * 2.), px(TOOLS_MIN_HEIGHT * 3.)));
    let handle = cx.open_window(options, move |window, cx| {
        let id = window.id();
        // The user closing the window closes the DevTools; docking them closes it without asking.
        window.on_should_close(move |_, cx| {
            let undocked =
                cx.try_global::<State>().and_then(|s| s.open.get(&tab)).is_some_and(|o| o.window == Some(id));
            if undocked {
                close(cx, tab);
                refresh_owner(cx, tab);
            }
            true
        });
        cx.new(|_| ToolsWindow { tab })
    });
    if let Some(open) = state(cx).open.get_mut(&tab) {
        open.window = Some(handle.id());
    }
}

fn owner(cx: &App, tab: TabId) -> Option<Entity<BrowserWindow>> {
    app::windows(cx).into_iter().map(|(_, e)| e).find(|e| e.read(cx).tab(tab).is_some())
}

/// The browser window showing `tab`, re-rendered.
fn refresh_owner(cx: &mut App, tab: TabId) {
    if let Some(entity) = owner(cx, tab) {
        entity.update(cx, |w, cx| w.changed(cx));
    }
}

/// A dock control was pressed, in the page area or in the DevTools' own window.
fn set_side(cx: &mut App, tab: TabId, side: Side) {
    let Some(entity) = owner(cx, tab) else { return };
    entity.update(cx, |w, cx| {
        let mut dock = dock_of(cx, w.id);
        dock.side = side;
        state(cx).docks.insert(w.id, dock);
        follow_dock(w, cx);
        w.changed(cx);
    });
}

/// A tab listener: a closed tab's DevTools close, a moved tab's too (the window it moves to
/// docks its own way), and the tab coming on show takes the window's dock.
pub fn tab_event(win: &mut BrowserWindow, id: TabId, event: TabEvent, cx: &mut Context<BrowserWindow>) {
    match event {
        TabEvent::Closed | TabEvent::Moved => close(cx, id),
        TabEvent::Shown => follow_dock(win, cx),
        TabEvent::Opened => {}
    }
}

/// A window listener: its dock and its tabs' DevTools go with it.
pub fn forget(cx: &mut App, win: WindowId) {
    if !cx.has_global::<State>() {
        return;
    }
    let s = state(cx);
    s.docks.remove(&win);
    let gone: Vec<TabId> = s.open.iter().filter(|(_, o)| o.browser == win).map(|(t, _)| *t).collect();
    for tab in gone {
        close(cx, tab);
    }
}

/// The page area of `win` with the shown tab's DevTools docked in it, or the page area alone.
pub fn dock(win: &BrowserWindow, area: AnyElement, cx: &mut Context<BrowserWindow>) -> AnyElement {
    let dock = dock_of(cx, win.id);
    let shown = win
        .active_tab()
        .filter(|_| win.page_fullscreen.is_none())
        .and_then(|t| tools_of(cx, t).map(|tools| (t.id, tools)));
    let (Some((tab, tools)), Some(ratio)) = (shown, dock.ratio()) else { return area };
    let horizontal = dock.side == Side::Right;
    let (page_min, tools_min) =
        if horizontal { (PAGE_MIN_WIDTH, TOOLS_MIN_WIDTH) } else { (PAGE_MIN_HEIGHT, TOOLS_MIN_HEIGHT) };
    let browser = win.id;
    split("devtools-split", if horizontal { Axis::Horizontal } else { Axis::Vertical })
        .label("Developer Tools")
        .size_full()
        .ratio(ratio)
        .first_pane(split_pane().min_size(px(page_min)).child(area))
        .second_pane(split_pane().min_size(px(tools_min)).child(panel(tab, &tools, dock.side, cx)))
        .on_resize(move |e: &SplitResized, _, cx| {
            let mut dock = dock_of(cx, browser);
            dock.resized(e.ratio);
            state(cx).docks.insert(browser, dock);
            if let Some(w) = app::window(cx, browser) {
                w.update(cx, |_, cx| cx.notify());
            }
        })
        .into_any_element()
}

/// The DevTools of `win`'s tabs that are open but not on show in it, for its holder.
pub fn held(win: &BrowserWindow, cx: &App) -> Vec<AnyElement> {
    let docked = dock_of(cx, win.id).ratio().is_some() && win.page_fullscreen.is_none();
    win.tabs()
        .into_iter()
        .filter(|t| !(docked && t.id == win.active))
        .filter_map(|t| {
            let open = cx.try_global::<State>()?.open.get(&t.id)?;
            let tools = tools_of(cx, t).filter(|_| open.window.is_none())?;
            Some(div().size_0().child(tools_view(t.id, &tools)).into_any_element())
        })
        .collect()
}

/// The dock controls over the DevTools, then the DevTools.
fn panel(tab: TabId, tools: &WebView, side: Side, cx: &mut App) -> AnyElement {
    v_flex()
        .id("devtools")
        .size_full()
        .child(bar(tab, side, cx))
        .child(tools_view(tab, tools).flex_1().min_h_0())
        .into_any_element()
}

/// The DevTools page, docked or held. Chromium reports the inspected page leaving fullscreen to
/// its DevTools while they are open, so that goes on to the tab.
fn tools_view(tab: TabId, tools: &WebView) -> WebViewElement {
    webview(element_key(tab), tools)
        .on_window_closed(move |_: &WindowClosed, _, cx| {
            close(cx, tab);
            refresh_owner(cx, tab);
        })
        .on_fullscreen_changed(move |e: &FullscreenChanged, _, cx| {
            if let Some(entity) = owner(cx, tab) {
                entity.update(cx, |w, cx| w.page_fullscreen_changed(tab, e.fullscreen, cx));
            }
        })
        // Chromium's shortcuts inside the DevTools, F12 among them, mean what they mean in the
        // page's window.
        .on_chrome_command(move |e: &ChromeCommand, _, cx| {
            if let Some(entity) = owner(cx, tab) {
                entity.update(cx, |w, cx| crate::page::chrome_command(w, e.command, cx));
            }
        })
}

fn bar(tab: TabId, side: Side, cx: &App) -> AnyElement {
    let t = cx.theme();
    let side_button = |id: &'static str, icon: Icon, tip: &'static str, this: Side| {
        icon_button(id, icon)
            .tooltip(tip)
            .checked(side == this)
            .on_click(move |_: &ClickEvent, _, cx| set_side(cx, tab, this))
    };
    let mut row = h_flex()
        .id("devtools-bar")
        .h(px(BAR_HEIGHT))
        .flex_none()
        .px(t.spacing.sm)
        .gap(t.spacing.xs)
        .items_center()
        .bg(t.surface.base)
        .border_b_1()
        .border_color(t.separator)
        // The DevTools' own window already says what it is in its title.
        .child(match side {
            Side::Window => div().flex_1().into_any_element(),
            _ => label("Developer Tools")
                .secondary()
                .text_style(TextStyle::Caption1)
                .truncate()
                .flex_1()
                .into_any_element(),
        })
        .child(side_button("devtools-dock-bottom", Icon::PanelBottom, "Dock to Bottom", Side::Bottom))
        .child(side_button("devtools-dock-right", Icon::PanelRight, "Dock to Right", Side::Right))
        .child(side_button("devtools-undock", Icon::AppWindow, "Open in Window", Side::Window));
    if side != Side::Window {
        row = row.child(icon_button("devtools-close", Icon::X).tooltip("Close Developer Tools").on_click(
            move |_: &ClickEvent, _, cx| {
                close(cx, tab);
                refresh_owner(cx, tab);
            },
        ));
    }
    row.into_any_element()
}

/// The DevTools' own window: the dock controls, then the DevTools.
struct ToolsWindow {
    tab: TabId,
}

impl Render for ToolsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tools = cx.try_global::<State>().and_then(|s| s.open.get(&self.tab)).map(|o| o.tools.clone());
        let bg = cx.theme().surface.base;
        let mut root = v_flex().size_full().bg(bg);
        if let Some(tools) = tools {
            root = root.child(panel(self.tab, &tools, Side::Window, cx));
        }
        root
    }
}
