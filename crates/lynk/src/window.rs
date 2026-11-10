//! One browser window's state: its tabs in sidebar order, the tab on show, and what the window
//! shows around them. Tabs are addressed by id. Every change to the tabs goes through the
//! methods here; `page.rs` feeds them what the pages report and `shell.rs` draws the result.

use lynk_core::session::{OpenDisposition, Placement, blank_tab, place_opened_tab};
use nd::core::WindowId;
use nd::prelude::*;
use nd::virtual_list::ListState;
use nd::webview::WebView;

use crate::app;
use crate::sidebar::SidebarRow;
use crate::tab::{Tab, TabId};
use crate::zoom::ZoomPopover;

/// Where a new tab goes and whether it is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    /// At the end, shown.
    End,
    /// At an index, shown or not.
    At { index: usize, foreground: bool },
    /// Where Chrome puts a tab `opener`'s page asked for.
    FromPage { opener: TabId, foreground: bool },
}

/// What the menu bar reads of the tab on show.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MenuFacts {
    pub pinned: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// The tab has an address (it is not the new tab page).
    pub has_address: bool,
    /// Its address is http, https or file.
    pub web_page: bool,
}

pub struct BrowserWindow {
    pub id: WindowId,
    pub tabs: Vec<Tab>,
    pub active: TabId,
    pub sidebar_hidden: bool,
    /// The tab whose page holds element fullscreen, which takes the whole window.
    pub page_fullscreen: Option<TabId>,
    /// F11 on Linux: the window fullscreen with the sidebar put away.
    pub browser_fullscreen: bool,
    /// The window's close is waiting on its pages' `beforeunload`.
    pub closing: bool,
    pub zoom: ZoomPopover,
    /// The padlock's site information popover is up.
    pub site_info_open: bool,
    pub toasts: Vec<Toast>,
    pub(crate) rows: ListState<SidebarRow>,
    /// The row under the pointer, whose close button shows.
    pub(crate) hovered: Option<TabId>,
    next_toast: u64,
}

impl BrowserWindow {
    pub fn new(id: WindowId, tabs: Vec<Tab>, active: Option<TabId>, cx: &mut Context<Self>) -> Self {
        let active = active.or_else(|| tabs.first().map(|t| t.id)).unwrap_or_default();
        let mut this = Self {
            id,
            tabs,
            active,
            sidebar_hidden: false,
            page_fullscreen: None,
            browser_fullscreen: false,
            closing: false,
            zoom: ZoomPopover::default(),
            site_info_open: false,
            toasts: Vec::new(),
            rows: ListState::new(cx).estimated_row_height(px(crate::sidebar::ROW_HEIGHT)),
            hovered: None,
            next_toast: 0,
        };
        this.show(active, cx);
        this.sync_rows(cx);
        this
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tab(self.active)
    }

    pub fn active_page(&self) -> Option<&WebView> {
        self.active_tab().and_then(|t| t.page.as_ref())
    }

    fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// Re-renders the window and brings the menu bar up to date.
    pub fn changed(&mut self, cx: &mut Context<Self>) {
        self.sync_rows(cx);
        cx.notify();
        app::refresh_menus_for(self, cx);
    }

    pub fn menu_facts(&self) -> MenuFacts {
        let tab = self.active_tab();
        let url = tab.map_or("", |t| t.url.as_str());
        MenuFacts {
            pinned: tab.is_some_and(|t| t.pinned),
            can_go_back: tab.is_some_and(|t| t.rt.can_go_back),
            can_go_forward: tab.is_some_and(|t| t.rt.can_go_forward),
            has_address: !url.is_empty(),
            web_page: ["http:", "https:", "file:"].iter().any(|s| url.starts_with(s)),
        }
    }

    /// The window's name in another window's "Move Tab to Window" menu.
    pub fn window_label(&self) -> String {
        let (title, url) = self.active_tab().map_or(("", ""), |t| (t.title.as_str(), t.url.as_str()));
        lynk_core::tabstate::window_label(title, url, self.tabs.len())
    }

    /// The page or the window holds fullscreen, and the sidebar is put away for it.
    pub fn immersive(&self) -> bool {
        self.page_fullscreen.is_some() || self.browser_fullscreen
    }

    /// Opens a tab at `url` (empty for the new tab page) and returns its id.
    pub fn open_tab(&mut self, url: impl Into<String>, open: Open, cx: &mut Context<Self>) -> TabId {
        let tab = Tab::new(app::next_tab_id(cx), url);
        self.insert(tab, open, cx)
    }

    /// Adds `tab` where `open` says, and shows it when it is in front.
    pub fn insert(&mut self, mut tab: Tab, open: Open, cx: &mut Context<Self>) -> TabId {
        let id = tab.id;
        let Placement { index, foreground } = match open {
            Open::End => Placement { index: self.tabs.len(), foreground: true },
            Open::At { index, foreground } => Placement { index: index.min(self.tabs.len()), foreground },
            Open::FromPage { opener, foreground } => {
                tab.opener = Some(opener);
                self.placement(opener, foreground)
            }
        };
        self.tabs.insert(index, tab);
        if foreground {
            self.show(id, cx);
        }
        self.changed(cx);
        id
    }

    fn placement(&self, opener: TabId, foreground: bool) -> Placement {
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .map(|t| {
                let mut s = blank_tab(t.id.to_string());
                s.pinned = t.pinned;
                s
            })
            .collect();
        let openers = self.tabs.iter().filter_map(|t| t.opener.map(|o| (t.id.to_string(), o.to_string()))).collect();
        let disposition = if foreground { OpenDisposition::ForegroundTab } else { OpenDisposition::BackgroundTab };
        place_opened_tab(&tabs, &opener.to_string(), &openers, Some(disposition))
    }

    /// Shows `id`, giving it its page if it has none yet.
    pub fn select(&mut self, id: TabId, cx: &mut Context<Self>) {
        if self.tab(id).is_none() || self.active == id {
            return;
        }
        self.show(id, cx);
        self.changed(cx);
    }

    fn show(&mut self, id: TabId, cx: &mut Context<Self>) {
        // Chrome leaves a page's fullscreen when its tab stops being the one shown.
        if let Some(full) = self.page_fullscreen.filter(|f| *f != id) {
            if let Some(page) = self.tab(full).and_then(|t| t.page.as_ref()) {
                page.exit_fullscreen();
            }
            self.page_fullscreen = None;
            self.sync_window_fullscreen(cx);
        }
        self.active = id;
        self.zoom.close();
        self.site_info_open = false;
        if let Some(tab) = self.tab_mut(id) {
            tab.wake();
        }
        self.apply_zoom(id, cx);
        if let Some(title) = self.active_tab().map(|t| t.label()) {
            cx.set_window_title(self.id, title);
        }
    }

    /// Steps through the tabs in sidebar order, wrapping around.
    pub fn cycle(&mut self, step: isize, cx: &mut Context<Self>) {
        let (Some(at), len) = (self.index_of(self.active), self.tabs.len() as isize) else { return };
        if len < 2 {
            return;
        }
        let next = (at as isize + step).rem_euclid(len) as usize;
        self.select(self.tabs[next].id, cx);
    }

    pub fn select_at(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(id) = self.tabs.get(index).map(|t| t.id) {
            self.select(id, cx);
        }
    }

    /// Closes `id` once its page agrees (`beforeunload`, "Leave site?"). A tab with no page has
    /// nothing to ask.
    pub fn close_tab(&mut self, id: TabId, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        match &tab.page {
            Some(page) if tab.rt.crashed.is_none() => {
                tab.rt.closing = true;
                page.request_close();
            }
            _ => self.remove(id, cx),
        }
    }

    /// The page answered whether it may go.
    pub fn close_answered(&mut self, id: TabId, approved: bool, cx: &mut Context<Self>) {
        let Some(tab) = self.tab_mut(id) else { return };
        if !std::mem::take(&mut tab.rt.closing) {
            return;
        }
        if !approved {
            // A page that keeps itself open keeps its window open too.
            self.closing = false;
            for tab in &mut self.tabs {
                tab.rt.closing = false;
            }
            return;
        }
        if self.closing {
            if self.tabs.iter().all(|t| !t.rt.closing) {
                self.close_window(cx);
            }
            return;
        }
        self.remove(id, cx);
    }

    /// Takes `id` out with no question asked: the page closed itself, or agreed to go. The last
    /// tab takes its window with it.
    pub fn remove(&mut self, id: TabId, cx: &mut Context<Self>) {
        let Some(at) = self.index_of(id) else { return };
        let tab = self.tabs.remove(at);
        app::remember_closed(cx, &tab);
        if self.tabs.is_empty() {
            self.close_window(cx);
            return;
        }
        if self.active == id {
            let next = self.tabs[at.min(self.tabs.len() - 1)].id;
            self.show(next, cx);
        }
        if self.page_fullscreen == Some(id) {
            self.page_fullscreen = None;
            self.sync_window_fullscreen(cx);
        }
        self.changed(cx);
    }

    /// The window's close, from its close button or the last tab going. Every page that could
    /// object is asked first; the window goes once they all agree.
    pub fn should_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.closing {
            return false;
        }
        let mut asked = false;
        for tab in &mut self.tabs {
            if let Some(page) = tab.page.as_ref().filter(|_| tab.rt.crashed.is_none()) {
                tab.rt.closing = true;
                page.request_close();
                asked = true;
            }
        }
        if !asked {
            for tab in std::mem::take(&mut self.tabs) {
                app::remember_closed(cx, &tab);
            }
            app::forget_window(cx, self.id);
            return true;
        }
        self.closing = true;
        false
    }

    fn close_window(&mut self, cx: &mut Context<Self>) {
        for tab in std::mem::take(&mut self.tabs) {
            app::remember_closed(cx, &tab);
        }
        app::forget_window(cx, self.id);
        cx.close_window(self.id);
    }

    /// Shift+Cmd+T: the last tab closed in any window, here.
    pub fn reopen(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = app::pop_closed(cx) {
            self.open_tab(url, Open::End, cx);
        }
    }

    /// Every other tab that is not pinned: pinned tabs are kept on purpose.
    pub fn close_others(&mut self, id: TabId, cx: &mut Context<Self>) {
        let others: Vec<TabId> = self.tabs.iter().filter(|t| t.id != id && !t.pinned).map(|t| t.id).collect();
        for other in others {
            self.close_tab(other, cx);
        }
    }

    pub fn duplicate(&mut self, id: TabId, cx: &mut Context<Self>) {
        let (Some(at), Some(url)) = (self.index_of(id), self.tab(id).map(|t| t.url.clone())) else { return };
        self.open_tab(url, Open::At { index: at + 1, foreground: true }, cx);
    }

    /// Moves `id` one place left (`-1`) or right (`1`) in the sidebar.
    pub fn move_by(&mut self, id: TabId, delta: isize, cx: &mut Context<Self>) {
        let Some(at) = self.index_of(id) else { return };
        let to = at as isize + delta;
        if to < 0 || to as usize >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(at);
        self.tabs.insert(to as usize, tab);
        self.changed(cx);
    }

    /// Takes `id` out to move it to another window, its page still live. The last tab cannot
    /// leave: its window would close under it.
    pub fn take(&mut self, id: TabId, cx: &mut Context<Self>) -> Option<Tab> {
        if self.tabs.len() < 2 {
            return None;
        }
        let at = self.index_of(id)?;
        if self.page_fullscreen == Some(id) {
            if let Some(page) = &self.tabs[at].page {
                page.exit_fullscreen();
            }
            self.page_fullscreen = None;
            self.sync_window_fullscreen(cx);
        }
        let mut tab = self.tabs.remove(at);
        tab.opener = None;
        if self.active == id {
            let next = self.tabs[at.min(self.tabs.len() - 1)].id;
            self.show(next, cx);
        }
        self.changed(cx);
        Some(tab)
    }

    /// A tab handed over from another window, shown at the end.
    pub fn adopt(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.insert(tab, Open::End, cx);
    }

    /// A tab with a page to let go of. The one on show can sleep only when there is another to
    /// show instead.
    pub fn can_sleep(&self, id: TabId) -> bool {
        self.tab(id).is_some_and(|t| t.is_live() && !t.url.is_empty()) && (self.active != id || self.tabs.len() > 1)
    }

    /// Put Tab to Sleep: closes the page, which gives its memory back, and keeps the tab's place,
    /// title, icon and address. Its scroll position is read first and put back when it wakes.
    pub fn sleep(&mut self, id: TabId, cx: &mut Context<Self>) {
        let Some(page) = self.tab(id).filter(|_| self.can_sleep(id)).and_then(|t| t.page.clone()) else { return };
        let this = cx.weak_entity();
        page.execute_javascript("[scrollX, scrollY]", move |result, cx| {
            let scroll = result.ok().and_then(|v| {
                let pair = v.as_array()?;
                Some((pair.first()?.as_f64()?, pair.get(1)?.as_f64()?))
            });
            let _ = this.update(cx, |this, cx| this.fall_asleep(id, scroll.filter(|s| *s != (0., 0.)), cx));
        });
    }

    fn fall_asleep(&mut self, id: TabId, scroll: Option<(f64, f64)>, cx: &mut Context<Self>) {
        // The page may have been closed or shown while its scroll was read.
        if !self.can_sleep(id) {
            return;
        }
        if self.active == id {
            // The nearest tab still awake takes over, so sleeping one page loads no other.
            let at = self.index_of(id).unwrap_or(0);
            let next = self
                .tabs
                .iter()
                .enumerate()
                .filter(|(_, t)| t.id != id)
                .min_by_key(|(i, t)| (!t.is_live(), i.abs_diff(at) * 2 + usize::from(*i < at)))
                .map(|(_, t)| t.id);
            if let Some(next) = next {
                self.show(next, cx);
            }
        }
        if let Some(tab) = self.tab_mut(id) {
            tab.sleep(scroll);
        }
        self.changed(cx);
    }

    /// Loads `url` in `id`. Entering the address already shown reloads it.
    pub fn navigate(&mut self, id: TabId, url: String, cx: &mut Context<Self>) {
        if let Some(tab) = self.tab_mut(id) {
            tab.navigate(url);
        }
        self.apply_zoom(id, cx);
        self.changed(cx);
    }

    /// Gives `id`'s page the level kept for its host.
    pub fn apply_zoom(&mut self, id: TabId, cx: &mut Context<Self>) {
        let Some(url) = self.tab(id).map(|t| t.url.clone()) else { return };
        let factor = app::zoom_for(cx, &url);
        let Some(tab) = self.tab_mut(id) else { return };
        if let Some(page) = &tab.page
            && tab.rt.zoom != factor
        {
            tab.rt.zoom = factor;
            page.set_zoom(factor);
        }
    }

    /// Every tab's page at its host's level, after a level changed.
    pub fn apply_zoom_everywhere(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<TabId> = self.tabs.iter().map(|t| t.id).collect();
        for id in ids {
            self.apply_zoom(id, cx);
        }
        cx.notify();
    }

    /// Asks the window to enter or leave fullscreen to match what the page and the user asked.
    pub fn sync_window_fullscreen(&mut self, cx: &mut Context<Self>) {
        let on = self.immersive();
        cx.set_window_fullscreen(self.id, on);
    }

    pub fn toast(&mut self, title: impl Into<SharedString>) {
        self.next_toast += 1;
        self.toasts.push(toast(format!("toast-{}", self.next_toast), title));
    }
}
