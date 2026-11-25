//! The browser window as drawn: the sidebar as the split view's sidebar item, the page
//! full-bleed beside it with the load bar and the find bar over its top, and the window's
//! overlays. The handlers for the commands the shell owns are here too.

use nd::core::WindowId;
use nd::prelude::*;

use crate::actions::{self, CONTEXT, MoveTabToWindow, SelectTab, SelectTabAt, SwitchProfile};
use crate::app;
use crate::page;
use crate::sidebar;
use crate::tab::TabId;
use crate::window::{BrowserWindow, Open};

/// Something drawn over the whole window, above the page: the command bar, a panel.
pub type Overlay = fn(&BrowserWindow, &mut Window, &mut Context<BrowserWindow>) -> Option<AnyElement>;

/// The window's overlays, back to front.
pub const OVERLAYS: &[Overlay] = &[crate::omnibox::overlay];

/// The load bar's sliver while a load has made no progress yet.
const LOAD_SLIVER: f32 = 0.08;
const LOAD_BAR: f32 = 2.;

impl Render for BrowserWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hidden = self.sidebar_hidden || self.immersive();
        let side = self.sidebar(cx);
        let content = self.content(cx);
        let toasts = self.toasts.clone();
        let toasts =
            toast_overlay("toasts").toasts(toasts).on_dismiss(cx.listener(|win, e: &ToastDismissed, _, cx| {
                win.toasts.retain(|t| t.id != e.id);
                cx.notify();
            }));
        let layers: Vec<AnyElement> = OVERLAYS.iter().filter_map(|layer| layer(self, window, cx)).collect();
        // The toast layer lets clicks through to the page; a full-size div would take them all,
        // so one wraps the layers only while there are any.
        let overlay = if layers.is_empty() {
            toasts.into_any_element()
        } else {
            div().size_full().children(layers).child(toasts).into_any_element()
        };
        let foot = self.foot(window, cx);
        // No toolbar and no title band: the sidebar and the page run to the window's top edge.
        let root = scaffold()
            .content_under_toolbar()
            .key_context(CONTEXT)
            .sidebar(
                nd::prelude::sidebar()
                    .collapsed(hidden)
                    .width(px(sidebar::WIDTH))
                    .under_toolbar()
                    .footer(foot)
                    .on_collapse_change(cx.listener(|win, e: &PaneCollapsed, _, cx| {
                        // The split view folds the sidebar itself in a narrow window or at the
                        // divider; that is the user hiding it.
                        if !win.immersive() && win.sidebar_hidden != e.collapsed {
                            win.sidebar_hidden = e.collapsed;
                            win.changed(cx);
                        }
                    }))
                    .child(side),
            )
            .content(content)
            .overlay(overlay)
            .on_action(cx.listener(|win, a: &SelectTabAt, _, cx| win.select_at(a.0, cx)))
            .on_action(cx.listener(|win, a: &SelectTab, _, cx| win.select(a.0, cx)))
            .on_action(cx.listener(|win, a: &MoveTabToWindow, _, cx| move_tab(win, win.active, Some(a.0), cx)))
            .on_action(cx.listener(actions::run_for_tab))
            .on_action(cx.listener(|win, a: &SwitchProfile, window, cx| crate::profile::switch(win, a, window, cx)));
        actions::listen_all(root, cx)
    }
}

impl BrowserWindow {
    /// The page on show with what goes over it, and every other live page in a zero-size
    /// holder so it keeps reporting.
    fn content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut shown = div().id("content").size_full().relative();
        let mut held = div().id("held-pages").size_0().overflow_hidden().absolute();
        let active = self.active;
        let mut cover = None;
        for tab in self.tabs() {
            let Some(live) = tab.live() else { continue };
            let el = page::mount(tab, live, cx);
            if tab.id == active {
                shown = shown.child(el);
            } else {
                held = held.child(div().size_0().child(el));
            }
        }
        if let Some(tab) = self.active_tab() {
            cover = page::cover(tab, cx);
            if let Some(bar) = self.load_bar(cx) {
                shown = shown.child(bar);
            }
        }
        if let Some(cover) = cover {
            let bg = cx.theme().surface.base;
            shown = shown.child(div().absolute().inset_0().bg(bg).child(cover));
        }
        if let Some(bar) = crate::find::bar(self, cx) {
            let pad = cx.theme().spacing.sm;
            shown = shown.child(div().absolute().top(pad).right(pad).child(bar));
        }
        shown.child(held).into_any_element()
    }

    /// A 2 pt line in the secondary ink along the page's top edge while it loads.
    fn load_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = self.active_tab()?;
        if !tab.rt.loading || tab.rt.crashed.is_some() {
            return None;
        }
        let fraction = (tab.rt.progress as f32).clamp(LOAD_SLIVER, 1.);
        let ink = cx.theme().text.secondary;
        Some(
            div()
                .id("load-bar")
                .absolute()
                .top_0()
                .left_0()
                .h(px(LOAD_BAR))
                .w(relative(fraction))
                .bg(ink)
                .into_any_element(),
        )
    }
}

pub fn copy_address(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(url) = win.tab(id).map(|t| t.url.clone()).filter(|u| !u.is_empty()) else { return };
    nd::clipboard::write_text(cx, url);
    win.toast("Link copied");
    cx.notify();
}

pub fn reload(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.tab_mut(id) else { return };
    tab.rt.error = None;
    tab.rt.crashed = None;
    match tab.view() {
        Some(page) => page.reload(),
        None => tab.wake(),
    }
    win.changed(cx);
}

pub fn toggle_fullscreen(win: &mut BrowserWindow, _: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    if let Some(page) = win.page_fullscreen.and_then(|id| win.tab(id)).and_then(|t| t.view()) {
        page.exit_fullscreen();
        return;
    }
    win.browser_fullscreen = !win.browser_fullscreen;
    win.sync_window_fullscreen(cx);
    win.changed(cx);
}

pub fn toggle_sidebar(win: &mut BrowserWindow, _: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    win.sidebar_hidden = !win.sidebar_hidden;
    win.changed(cx);
}

/// `view-source:` of the page, in a tab to the right of it.
pub fn view_source(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let (Some(at), Some(tab)) = (win.index_of(id), win.tab(id).filter(|t| t.is_web_page())) else { return };
    let url = format!("view-source:{}", tab.url);
    win.open_tab(url, Open::At { index: at + 1, foreground: true }, cx);
}

pub(crate) fn move_tab(win: &mut BrowserWindow, id: TabId, target: Option<WindowId>, cx: &mut Context<BrowserWindow>) {
    let target = match target {
        // A window on its way out would take the tab with it, and a page never crosses profiles.
        Some(id) => {
            match app::window(cx, id).filter(|w| !w.read(cx).pending_close() && w.read(cx).profile == win.profile) {
                Some(entity) => Some(entity),
                None => return,
            }
        }
        None => None,
    };
    let Some(tab) = win.take(id, cx) else { return };
    match target {
        Some(entity) => entity.update(cx, |w, cx| w.adopt(tab, cx)),
        None => {
            app::open_window_around(cx, win.profile.clone(), tab);
        }
    }
}
