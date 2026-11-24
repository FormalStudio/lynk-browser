//! One browser window's state: its tabs, their order in the window's `SidebarTree`, the tab on
//! show, and what the window shows around them. Tabs are addressed by id. Every change to the
//! tabs goes through the methods here; `page.rs` feeds them what the pages report and `shell.rs`
//! draws the result.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use lynk_core::session::{OpenDisposition, Placement, place_opened_tab, tab_id, tab_number};
use lynk_core::sidebar_tree::{Dest, SidebarTree};
use lynk_core::spaces::neighbour;
use nd::core::WindowId;
use nd::prelude::*;
use nd::virtual_list::ListState;
use nd::webview::WebView;

use crate::app;
use crate::sidebar::SidebarRow;
use crate::tab::{Page, Tab, TabId};

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

/// How long a tab going to sleep waits for its page to say where it is scrolled.
const SCROLL_READ_LIMIT: Duration = Duration::from_millis(500);

/// What happened to one of a window's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabEvent {
    Opened,
    Closed,
    /// Taken out to another window.
    Moved,
    Shown,
}

/// Hears every tab event of every window. A lane keeps what it needs in its own global.
pub type TabListener = fn(&mut BrowserWindow, TabId, TabEvent, &mut Context<BrowserWindow>);

/// The tab event listeners, called in order after the window's own change.
pub const TAB_EVENTS: &[TabListener] = &[crate::zoom::tab_event, crate::site_info::tab_event];

/// Drops what a module keeps for a window that has gone.
pub type WindowListener = fn(&mut App, WindowId);

/// Called as a window goes, so no module keeps state for it.
pub const WINDOW_CLOSED: &[WindowListener] = &[crate::zoom::forget, crate::site_info::forget, crate::sidebar::forget];

pub struct BrowserWindow {
    pub id: WindowId,
    /// In no order; `tree` holds the sidebar's.
    tabs: Vec<Tab>,
    tree: SidebarTree,
    pub active: TabId,
    pub sidebar_hidden: bool,
    /// The tab whose page holds element fullscreen, which takes the whole window.
    pub page_fullscreen: Option<TabId>,
    /// F11 on Linux: the window fullscreen with the sidebar put away.
    pub browser_fullscreen: bool,
    /// Whether the window was already in fullscreen when the page or F11 took it there, which
    /// leaving keeps.
    fullscreen_before: Option<bool>,
    /// The window's close is waiting on its pages' `beforeunload`.
    closing: bool,
    /// Tabs whose close waits on their page's `beforeunload`, apart from the window's close.
    closing_tabs: Vec<TabId>,
    pub toasts: Vec<Toast>,
    pub(crate) rows: ListState<SidebarRow>,
    next_toast: u64,
}

impl BrowserWindow {
    /// A window on `tabs`, which are new unless `moved` (one tab moved out of another window).
    pub fn new(id: WindowId, tabs: Vec<Tab>, active: Option<TabId>, moved: bool, cx: &mut Context<Self>) -> Self {
        let active = active.or_else(|| tabs.first().map(|t| t.id)).unwrap_or_default();
        let mut tree = SidebarTree::default();
        for tab in &tabs {
            tree.add_tab(&tab_id(tab.id), None, usize::MAX).expect("tab ids are unique");
        }
        let mut this = Self {
            id,
            tabs,
            tree,
            active,
            sidebar_hidden: false,
            page_fullscreen: None,
            browser_fullscreen: false,
            fullscreen_before: None,
            closing: false,
            closing_tabs: Vec::new(),
            toasts: Vec::new(),
            rows: ListState::new(cx).estimated_row_height(px(crate::sidebar::ROW_HEIGHT)),
            next_toast: 0,
        };
        if !moved {
            for id in this.order() {
                this.tab_event(id, TabEvent::Opened, cx);
            }
        }
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
        self.active_tab().and_then(Tab::view)
    }

    /// The tab ids in sidebar order.
    pub fn order(&self) -> Vec<TabId> {
        self.tree.tab_ids().into_iter().filter_map(tab_number).collect()
    }

    /// The tabs in sidebar order.
    pub fn tabs(&self) -> Vec<&Tab> {
        self.order().into_iter().filter_map(|id| self.tab(id)).collect()
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// `id`'s row in the sidebar list, the index `Open::At` takes.
    pub fn index_of(&self, id: TabId) -> Option<usize> {
        self.tree.root_row_of(&tab_id(id))
    }

    pub fn is_pinned(&self, id: TabId) -> bool {
        self.tree.is_pinned(&tab_id(id))
    }

    /// The tab that takes over from `id` when it leaves: the next in the sidebar, else the one
    /// before.
    fn neighbour(&self, id: TabId) -> Option<TabId> {
        let order: Vec<String> = self.tree.tab_ids().into_iter().map(str::to_owned).collect();
        neighbour(&order, &tab_id(id)).as_deref().and_then(tab_number)
    }

    /// The window is closing, or every tab it has is: a launch must not land in it.
    pub fn pending_close(&self) -> bool {
        self.closing || self.tabs.iter().all(|t| self.closing_tabs.contains(&t.id))
    }

    fn tab_event(&mut self, id: TabId, event: TabEvent, cx: &mut Context<Self>) {
        for listener in TAB_EVENTS {
            listener(self, id, event, cx);
        }
        #[cfg(test)]
        tests::record(id, event);
    }

    /// Re-renders the window and brings the menu bar up to date.
    pub fn changed(&mut self, cx: &mut Context<Self>) {
        self.sync_rows(cx);
        cx.notify();
        app::refresh_menus_for(self, cx);
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
    pub fn insert(&mut self, tab: Tab, open: Open, cx: &mut Context<Self>) -> TabId {
        self.place(tab, open, true, cx)
    }

    fn place(&mut self, mut tab: Tab, open: Open, opened: bool, cx: &mut Context<Self>) -> TabId {
        let id = tab.id;
        let Placement { index, foreground } = match open {
            Open::End => Placement { index: usize::MAX, foreground: true },
            Open::At { index, foreground } => Placement { index, foreground },
            Open::FromPage { opener, foreground } => {
                tab.opener = Some(opener);
                self.placement(opener, foreground)
            }
        };
        self.tree.add_tab(&tab_id(id), None, index).expect("tab ids are unique");
        self.tabs.push(tab);
        if opened {
            self.tab_event(id, TabEvent::Opened, cx);
        }
        if foreground {
            self.show(id, cx);
        }
        self.changed(cx);
        id
    }

    fn placement(&self, opener: TabId, foreground: bool) -> Placement {
        let openers: HashMap<String, String> =
            self.tabs.iter().filter_map(|t| t.opener.map(|o| (tab_id(t.id), tab_id(o)))).collect();
        let disposition = if foreground { OpenDisposition::ForegroundTab } else { OpenDisposition::BackgroundTab };
        place_opened_tab(&self.tree, &tab_id(opener), &openers, Some(disposition))
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
            if let Some(page) = self.tab(full).and_then(Tab::view) {
                page.exit_fullscreen();
            }
            self.page_fullscreen = None;
            self.sync_window_fullscreen(cx);
        }
        self.active = id;
        if let Some(tab) = self.tab_mut(id) {
            tab.wake();
        }
        self.apply_zoom(id, cx);
        if let Some(title) = self.active_tab().map(|t| t.label()) {
            cx.set_window_title(self.id, title);
        }
        self.tab_event(id, TabEvent::Shown, cx);
    }

    /// Steps through the tabs in sidebar order, wrapping around.
    pub fn cycle(&mut self, step: isize, cx: &mut Context<Self>) {
        let order = self.order();
        let (Some(at), len) = (order.iter().position(|t| *t == self.active), order.len() as isize) else { return };
        if len < 2 {
            return;
        }
        let next = (at as isize + step).rem_euclid(len) as usize;
        self.select(order[next], cx);
    }

    pub fn select_at(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(id) = self.order().get(index).copied() {
            self.select(id, cx);
        }
    }

    /// Asks `id`'s page whether it may go, unless it was asked already and has not answered.
    /// `false` for a tab with nothing to ask: no page, or a crashed one.
    fn ask(&mut self, id: TabId) -> bool {
        let Some(tab) = self.tab_mut(id).filter(|t| t.rt.crashed.is_none()) else { return false };
        let Some(live) = tab.live_mut() else { return false };
        if !live.asked {
            live.asked = true;
            live.view.request_close();
        }
        true
    }

    fn waiting_on_pages(&self) -> bool {
        self.tabs.iter().any(|t| matches!(&t.page, Page::Live(live) if live.asked))
    }

    /// Closes `id` once its page agrees (`beforeunload`, "Leave site?"). A tab with no page has
    /// nothing to ask.
    /// A second close of the shown tab while its page has not answered closes the next tab that
    /// is not already closing, so Close Tab pressed at speed never drops a press.
    pub fn close_tab(&mut self, id: TabId, cx: &mut Context<Self>) {
        let id = if id == self.active && self.closing_tabs.contains(&id) {
            let order = self.order();
            let at = order.iter().position(|t| *t == id).unwrap_or(0);
            let after = order[at..].iter().chain(order[..at].iter().rev());
            let Some(&next) = after.into_iter().find(|t| !self.closing_tabs.contains(t)) else { return };
            next
        } else {
            id
        };
        if !self.ask(id) {
            self.remove(id, cx);
        } else if !self.closing_tabs.contains(&id) {
            self.closing_tabs.push(id);
        }
    }

    /// The page answered whether it may go.
    pub fn close_answered(&mut self, id: TabId, approved: bool, cx: &mut Context<Self>) {
        let Some(live) = self.tab_mut(id).and_then(Tab::live_mut) else { return };
        if !std::mem::take(&mut live.asked) {
            return;
        }
        let closing_tab = self.closing_tabs.contains(&id);
        self.closing_tabs.retain(|t| *t != id);
        if !approved {
            // A page that keeps itself open keeps its window open too.
            self.closing = false;
        } else if closing_tab {
            self.remove(id, cx);
        } else {
            self.settle_close(cx);
        }
    }

    /// Closes the window once a close it started has no page left to hear from.
    fn settle_close(&mut self, cx: &mut Context<Self>) {
        if self.closing && !self.waiting_on_pages() {
            self.close_window(cx);
        }
    }

    /// Takes `id` out with no question asked: the page closed itself, or agreed to go. The last
    /// tab takes its window with it.
    pub fn remove(&mut self, id: TabId, cx: &mut Context<Self>) {
        let Some(at) = self.tabs.iter().position(|t| t.id == id) else { return };
        let next = self.neighbour(id);
        let tab = self.tabs.remove(at);
        self.tree.close_tab(&tab_id(id));
        self.closing_tabs.retain(|t| *t != id);
        app::remember_closed(cx, &tab);
        self.tab_event(id, TabEvent::Closed, cx);
        if self.tabs.is_empty() {
            self.close_window(cx);
            return;
        }
        if let Some(next) = next.filter(|_| self.active == id) {
            self.show(next, cx);
        }
        if self.page_fullscreen == Some(id) {
            self.page_fullscreen = None;
            self.sync_window_fullscreen(cx);
        }
        self.changed(cx);
        self.settle_close(cx);
    }

    /// The window's close, from its close button or the last tab going. Every page that could
    /// object is asked first; the window goes once they all agree.
    pub fn should_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.closing {
            return false;
        }
        let mut waiting = false;
        for id in self.order() {
            waiting |= self.ask(id);
        }
        if !waiting {
            self.let_go(cx);
            return true;
        }
        self.closing = true;
        false
    }

    fn close_window(&mut self, cx: &mut Context<Self>) {
        self.let_go(cx);
        cx.close_window(self.id);
    }

    /// A window that goes closes its tabs, puts them on the closed stack and leaves the
    /// registry. The last one keeps its tabs and its entry: the app is quitting, and it is what a
    /// relaunch restores.
    fn let_go(&mut self, cx: &mut Context<Self>) {
        for id in self.order() {
            self.tab_event(id, TabEvent::Closed, cx);
        }
        for listener in WINDOW_CLOSED {
            listener(cx, self.id);
        }
        if app::windows(cx).len() < 2 {
            return;
        }
        for id in self.order() {
            if let Some(tab) = self.tab(id) {
                app::remember_closed(cx, tab);
            }
        }
        self.tabs.clear();
        self.tree = SidebarTree::default();
        app::forget_window(cx, self.id);
    }

    /// Shift+Cmd+T: the last tab closed in any window, here.
    pub fn reopen(&mut self, cx: &mut Context<Self>) {
        if let Some(url) = app::pop_closed(cx) {
            self.open_tab(url, Open::End, cx);
        }
    }

    /// Every other tab that is not pinned: pinned tabs are kept on purpose.
    pub fn close_others(&mut self, id: TabId, cx: &mut Context<Self>) {
        let others: Vec<TabId> = self.order().into_iter().filter(|t| *t != id && !self.is_pinned(*t)).collect();
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
        if to < 0 || to as usize >= self.tree.items.len() {
            return;
        }
        if self.tree.move_row(&tab_id(id), Dest::Root(to as usize)).is_ok() {
            self.changed(cx);
        }
    }

    /// Takes `id` out to move it to another window, its page still live. The last tab cannot
    /// leave: its window would close under it.
    pub fn take(&mut self, id: TabId, cx: &mut Context<Self>) -> Option<Tab> {
        if self.tabs.len() < 2 {
            return None;
        }
        let at = self.tabs.iter().position(|t| t.id == id)?;
        if self.page_fullscreen == Some(id) {
            if let Some(page) = self.tabs[at].view() {
                page.exit_fullscreen();
            }
            self.page_fullscreen = None;
            self.sync_window_fullscreen(cx);
        }
        let next = self.neighbour(id);
        let mut tab = self.tabs.remove(at);
        self.tree.close_tab(&tab_id(id));
        self.closing_tabs.retain(|t| *t != id);
        tab.opener = None;
        self.tab_event(id, TabEvent::Moved, cx);
        if let Some(next) = next.filter(|_| self.active == id) {
            self.show(next, cx);
        }
        self.changed(cx);
        self.settle_close(cx);
        Some(tab)
    }

    /// A tab handed over from another window, shown at the end. Its `Moved` fired as it left.
    pub fn adopt(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.place(tab, Open::End, false, cx);
    }

    /// A tab with a page to let go of. The one on show can sleep only when there is another to
    /// show instead.
    /// A page waiting on a close is never put to sleep: its answer settles that close.
    pub fn can_sleep(&self, id: TabId) -> bool {
        self.tab(id).is_some_and(|t| matches!(&t.page, Page::Live(live) if !live.asked) && !t.url.is_empty())
            && !self.closing
            && (self.active != id || self.tabs.len() > 1)
    }

    /// Put Tab to Sleep: closes the page, which gives its memory back, and keeps the tab's place,
    /// title, icon and address. Its scroll position is read first and put back when it wakes.
    pub fn sleep(&mut self, id: TabId, cx: &mut Context<Self>) {
        let Some(page) = self.tab(id).filter(|_| self.can_sleep(id)).and_then(|t| t.view().cloned()) else { return };
        // A page that does not answer in time (a busy renderer) sleeps all the same, at the top.
        let answered = Rc::new(Cell::new(false));
        let (this, once) = (cx.weak_entity(), answered.clone());
        page.execute_javascript("[scrollX, scrollY]", move |result, cx| {
            if once.replace(true) {
                return;
            }
            let scroll = result.ok().and_then(|v| {
                let pair = v.as_array()?;
                Some((pair.first()?.as_f64()?, pair.get(1)?.as_f64()?))
            });
            let _ = this.update(cx, |this, cx| this.fall_asleep(id, scroll.filter(|s| *s != (0., 0.)), cx));
        });
        let this = cx.weak_entity();
        nd::system::after(SCROLL_READ_LIMIT, move |cx| {
            if !answered.replace(true) {
                let _ = this.update(cx, |this, cx| this.fall_asleep(id, None, cx));
            }
        });
    }

    fn fall_asleep(&mut self, id: TabId, scroll: Option<(f64, f64)>, cx: &mut Context<Self>) {
        // The page may have been closed or shown while its scroll was read.
        if !self.can_sleep(id) {
            return;
        }
        if self.active == id {
            // The nearest tab still awake takes over, so sleeping one page loads no other.
            let tabs = self.tabs();
            let at = tabs.iter().position(|t| t.id == id).unwrap_or(0);
            let next = tabs
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
            tab.reset(Page::Asleep { scroll });
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
        self.set_zoom(id, factor);
    }

    /// Sets `id`'s page to `factor`, whatever its host keeps.
    pub fn set_zoom(&mut self, id: TabId, factor: f64) {
        let Some(tab) = self.tab_mut(id) else { return };
        if let Some(page) = tab.view().filter(|_| tab.rt.zoom != factor).cloned() {
            tab.rt.zoom = factor;
            page.set_zoom(factor);
        }
    }

    /// The page holding element fullscreen entered or left it; only the shown tab's takes the
    /// window.
    pub fn page_fullscreen_changed(&mut self, id: TabId, fullscreen: bool, cx: &mut Context<Self>) {
        self.page_fullscreen =
            if fullscreen && self.active == id { Some(id) } else { self.page_fullscreen.filter(|f| *f != id) };
        if let Some(tab) = self.tab_mut(id) {
            tab.rt.fullscreen = fullscreen;
        }
        self.sync_window_fullscreen(cx);
        self.changed(cx);
    }

    /// Every tab's page at its host's level, after a level changed.
    pub fn apply_zoom_everywhere(&mut self, cx: &mut Context<Self>) {
        for id in self.order() {
            self.apply_zoom(id, cx);
        }
        cx.notify();
    }

    /// Asks the window to enter or leave fullscreen to match what the page and the user asked.
    pub fn sync_window_fullscreen(&mut self, cx: &mut Context<Self>) {
        if self.immersive() {
            if self.fullscreen_before.is_none() {
                self.fullscreen_before = Some(cx.window_fullscreen(self.id));
            }
            cx.set_window_fullscreen(self.id, true);
        } else if self.fullscreen_before.take() == Some(false) {
            cx.set_window_fullscreen(self.id, false);
        }
    }

    pub fn toast(&mut self, title: impl Into<SharedString>) {
        self.next_toast += 1;
        self.toasts.push(toast(format!("toast-{}", self.next_toast), title));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use nd::test::TestApp;

    use super::*;
    use crate::shell::move_tab;
    use TabEvent::{Closed, Moved, Opened, Shown};

    thread_local! {
        static HEARD: RefCell<Vec<(TabId, TabEvent)>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn record(id: TabId, event: TabEvent) {
        HEARD.with(|h| h.borrow_mut().push((id, event)));
    }

    fn heard() -> Vec<(TabId, TabEvent)> {
        HEARD.with(|h| std::mem::take(&mut *h.borrow_mut()))
    }

    fn with(cx: &mut TestApp, id: WindowId, f: impl FnOnce(&mut BrowserWindow, &mut Context<BrowserWindow>)) {
        app::window(cx, id).unwrap().update(cx, f);
        cx.run_until_idle();
    }

    fn tabs(cx: &TestApp, id: WindowId) -> Vec<TabId> {
        app::window(cx, id).unwrap().read(cx).order()
    }

    #[test]
    fn each_tab_event_fires_once_on_every_path() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let blank = |cx: &mut TestApp| Tab::new(app::next_tab_id(cx), "");
        let first = vec![blank(&mut cx), blank(&mut cx)];
        let a = app::open_window(&mut cx, first, None);
        cx.run_until_idle();
        assert_eq!(heard(), [(1, Opened), (2, Opened), (1, Shown)], "a window's first tabs");

        let b = app::open_window(&mut cx, Vec::new(), None);
        cx.run_until_idle();
        assert_eq!(heard(), [(3, Opened), (3, Shown)], "New Window");

        with(&mut cx, a, |w, cx| {
            w.open_tab("", Open::End, cx);
        });
        assert_eq!(heard(), [(4, Opened), (4, Shown)], "a new tab");

        with(&mut cx, a, |w, cx| move_tab(w, 4, Some(b), cx));
        assert_eq!(heard(), [(4, Moved), (2, Shown), (4, Shown)], "to an existing window");
        assert_eq!(tabs(&cx, b), [3, 4]);

        with(&mut cx, b, |w, cx| move_tab(w, 4, None, cx));
        assert_eq!(heard(), [(4, Moved), (3, Shown), (4, Shown)], "to a new window");
        let c = app::windows(&cx).last().unwrap().0;
        assert_eq!(tabs(&cx, c), [4]);

        with(&mut cx, a, |w, cx| w.close_tab(2, cx));
        assert_eq!(heard(), [(2, Closed), (1, Shown)], "a tab");

        assert!(cx.request_close(a));
        cx.run_until_idle();
        assert_eq!(heard(), [(1, Closed)], "a window");
        assert!(cx.request_close(b));
        cx.run_until_idle();
        assert_eq!(heard(), [(3, Closed)], "another window");
        assert!(cx.request_close(c));
        cx.run_until_idle();
        assert_eq!(heard(), [(4, Closed)], "the last window");
    }

    #[test]
    fn a_launch_after_the_last_window_closed_gets_a_window() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let a = app::open_window(&mut cx, Vec::new(), None);
        cx.run_until_idle();
        assert_eq!(app::key_window(&cx), Some(a));
        assert!(cx.request_close(a));
        cx.run_until_idle();
        assert_eq!(app::key_window(&cx), None);

        crate::process::open_launch(&mut cx, vec!["https://example.com/".into()]);
        cx.run_until_idle();
        let b = app::key_window(&cx).expect("the launch opens a window");
        assert_ne!(b, a);
        assert_eq!(urls(&cx, b), ["https://example.com/"]);
    }

    fn live(cx: &mut TestApp) -> Tab {
        Tab::with_page(app::next_tab_id(cx), "https://example.com/", WebView::new())
    }

    fn urls(cx: &TestApp, id: WindowId) -> Vec<String> {
        app::window(cx, id).unwrap().read(cx).tabs().iter().map(|t| t.url.clone()).collect()
    }

    fn launch(cx: &mut TestApp) {
        crate::process::open_launch(cx, vec!["https://example.org/".into()]);
        cx.run_until_idle();
    }

    #[test]
    fn a_launch_skips_a_window_whose_last_tab_is_closing() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let first = vec![live(&mut cx)];
        let a = app::open_window(&mut cx, first, None);
        cx.run_until_idle();
        with(&mut cx, a, |w, cx| w.close_tab(1, cx));
        launch(&mut cx);
        assert_eq!(tabs(&cx, a), [1], "the launch stays out of the closing window");
        with(&mut cx, a, |w, cx| w.close_answered(1, true, cx));
        assert!(!cx.window_ids().any(|w| w == a));
        let b = app::key_window(&cx).expect("the launch has its own window");
        assert_eq!(urls(&cx, b), ["https://example.org/"]);
    }

    #[test]
    fn a_launch_skips_a_window_waiting_on_its_close() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let first = vec![live(&mut cx), Tab::new(app::next_tab_id(&mut cx), "")];
        let a = app::open_window(&mut cx, first, None);
        cx.run_until_idle();
        assert!(!cx.request_close(a), "the page is asked first");
        launch(&mut cx);
        assert_eq!(tabs(&cx, a), [1, 2]);
        with(&mut cx, a, |w, cx| w.close_answered(1, true, cx));
        assert!(!cx.window_ids().any(|w| w == a));
        let b = app::key_window(&cx).expect("the launch has its own window");
        assert_eq!(urls(&cx, b), ["https://example.org/"]);
    }

    /// Two windows, so the one that closes leaves the registry; `a` has two live tabs.
    fn closing_pair(cx: &mut TestApp) -> (WindowId, WindowId) {
        app::install(cx);
        let first = vec![live(cx), live(cx)];
        let a = app::open_window(cx, first, None);
        let b = app::open_window(cx, Vec::new(), None);
        cx.run_until_idle();
        assert!(!cx.request_close(a));
        with(cx, a, |w, cx| w.close_answered(2, true, cx));
        (a, b)
    }

    fn gone(cx: &TestApp, id: WindowId) -> bool {
        app::window(cx, id).is_none() && !cx.window_ids().any(|w| w == id)
    }

    #[test]
    fn a_page_waiting_on_the_window_close_stays_awake() {
        let mut cx = TestApp::new();
        let (a, _) = closing_pair(&mut cx);
        // A scroll read that started before the close lands now.
        with(&mut cx, a, |w, cx| w.fall_asleep(1, None, cx));
        with(&mut cx, a, |w, cx| w.close_answered(1, true, cx));
        assert!(gone(&cx, a));
    }

    #[test]
    fn a_window_close_settles_when_its_last_awaited_page_goes() {
        let mut cx = TestApp::new();
        let (a, _) = closing_pair(&mut cx);
        // The page closed itself instead of answering.
        with(&mut cx, a, |w, cx| w.remove(1, cx));
        assert!(gone(&cx, a));

        let mut cx = TestApp::new();
        let (a, b) = closing_pair(&mut cx);
        with(&mut cx, a, |w, cx| move_tab(w, 1, Some(b), cx));
        assert!(gone(&cx, a), "moved out");
        assert_eq!(tabs(&cx, b), [3, 1]);
    }

    #[test]
    fn cancel_on_one_page_keeps_the_other_tab_closes() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let first = vec![live(&mut cx), live(&mut cx), live(&mut cx)];
        let a = app::open_window(&mut cx, first, None);
        cx.run_until_idle();
        with(&mut cx, a, |w, cx| {
            w.close_tab(1, cx);
            w.close_tab(1, cx);
            w.close_tab(2, cx);
        });
        with(&mut cx, a, |w, cx| w.close_answered(1, false, cx));
        with(&mut cx, a, |w, cx| w.close_answered(2, true, cx));
        assert_eq!(tabs(&cx, a), [1, 3], "Cancel on one page leaves another page's close standing");
        // A repeated close of one page makes one tab close, settled by its one answer.
        with(&mut cx, a, |w, cx| w.close_answered(1, true, cx));
        assert_eq!(tabs(&cx, a), [1, 3], "a refused close is over");
    }

    #[test]
    fn a_tab_close_then_a_window_close_ask_the_page_once() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let first = vec![live(&mut cx), live(&mut cx)];
        let a = app::open_window(&mut cx, first, None);
        let _b = app::open_window(&mut cx, Vec::new(), None);
        cx.run_until_idle();
        with(&mut cx, a, |w, cx| w.close_tab(1, cx));
        assert!(!cx.request_close(a));
        cx.run_until_idle();
        let asked = |cx: &TestApp| {
            app::window(cx, a).unwrap().read(cx).tabs().iter().filter(|t| matches!(&t.page, Page::Live(l) if l.asked)).count()
        };
        assert_eq!(asked(&cx), 2);
        // Tab 1's one answer closes its tab; tab 2's then closes the window.
        with(&mut cx, a, |w, cx| w.close_answered(1, true, cx));
        assert_eq!(tabs(&cx, a), [2]);
        with(&mut cx, a, |w, cx| w.close_answered(2, true, cx));
        assert!(gone(&cx, a));
    }

    #[test]
    fn close_tab_pressed_at_speed_closes_one_tab_per_press() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let first = vec![live(&mut cx), live(&mut cx), live(&mut cx), live(&mut cx)];
        let a = app::open_window(&mut cx, first, None);
        cx.run_until_idle();
        let shown = app::window(&cx, a).unwrap().read(&cx).active;
        for _ in 0..3 {
            with(&mut cx, a, |w, cx| w.close_tab(shown, cx));
        }
        let closing = app::window(&cx, a).unwrap().read(&cx).closing_tabs.clone();
        assert_eq!(closing.len(), 3);
        for id in closing {
            with(&mut cx, a, |w, cx| w.close_answered(id, true, cx));
        }
        assert_eq!(tabs(&cx, a).len(), 1);
    }

    #[test]
    fn tabs_keep_sidebar_order_through_moves_and_closes() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let a = app::open_window(&mut cx, Vec::new(), None);
        cx.run_until_idle();
        with(&mut cx, a, |w, cx| {
            w.open_tab("https://a.test/", Open::End, cx);
            w.open_tab("https://b.test/", Open::At { index: 1, foreground: false }, cx);
            w.move_by(1, 1, cx);
        });
        assert_eq!(tabs(&cx, a), [3, 1, 2]);
        with(&mut cx, a, |w, cx| w.close_tab(1, cx));
        assert_eq!(tabs(&cx, a), [3, 2]);
        let shown = app::window(&cx, a).unwrap().read(&cx).active;
        assert_eq!(shown, 2, "the shown tab hands over to the next one");
    }

    #[test]
    fn a_row_item_reads_its_own_tab() {
        use crate::actions::{DuplicateTab, command_for, item_for_tab};
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let tabs = vec![Tab::new(app::next_tab_id(&mut cx), "https://example.com/"), Tab::new(app::next_tab_id(&mut cx), "")];
        let a = app::open_window(&mut cx, tabs, Some(2));
        cx.run_until_idle();
        let win = app::window(&cx, a).unwrap();
        let enabled = |id| match item_for_tab(command_for::<DuplicateTab>(), win.read(&cx), id, &cx) {
            MenuItem::Action { enabled, .. } => enabled,
            _ => unreachable!("a command is an action item"),
        };
        assert!(enabled(1), "the row's tab has an address");
        assert!(!enabled(2), "the shown new tab page has none");
    }
}
