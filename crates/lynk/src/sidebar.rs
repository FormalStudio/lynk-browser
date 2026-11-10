//! The sidebar: the controls row with the window's own buttons, the tab list that scrolls, and
//! the foot. Plain rows with their menus; pinned tiles and drag come later.

use std::hash::{Hash, Hasher};

use nd::core::WindowId;
use nd::prelude::*;
use nd::virtual_list::{SelectEvent, Selection, list};

use crate::actions::{self, ForTab, MoveTabToWindow, RowCommand};
use crate::app;
use crate::tab::TabId;
use crate::window::BrowserWindow;

pub const TAB_ROW_HEIGHT: f32 = 28.;
pub const ROW_GAP: f32 = 2.;
/// A row and the gap under it.
pub const ROW_HEIGHT: f32 = TAB_ROW_HEIGHT + ROW_GAP;
const CONTROLS_HEIGHT: f32 = 32.;
const MAC_MARGIN: f32 = 19.;
/// Half a traffic light's width.
const LIGHT_RADIUS: f32 = 7.;
const TRAIL_WIDTH: f32 = 24.;
const FAVICON: f32 = 16.;
pub const WIDTH: f32 = 240.;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SidebarRow {
    Tab(TabId),
    /// The quiet "New tab" row after the tabs. GNOME's idiom is the bare plus in the foot
    /// instead, which is where Linux has it.
    NewTab,
}

/// One slot of the foot for the window, or nothing while it has nothing to show.
pub type Slot = fn(&BrowserWindow, &mut Window, &mut Context<BrowserWindow>) -> Option<AnyElement>;

/// The foot, leading to trailing: settings, the padlock, zoom (while not 100%), blocked pop-ups,
/// a secondary window's own menu, the extensions (the puzzle, then pinned actions), downloads.
pub const FOOT: &[Slot] = &[settings, crate::site_info::padlock, crate::zoom::glyph, window_menu];

fn settings(_: &BrowserWindow, _: &mut Window, _: &mut Context<BrowserWindow>) -> Option<AnyElement> {
    Some(
        icon_button("sidebar-settings", Icon::Settings)
            .tooltip("Settings")
            .on_click(|_: &ClickEvent, window, cx| {
                cx.dispatch_action(window.id(), &actions::OpenSettings);
            })
            .into_any_element(),
    )
}

/// The menu a window other than the first carries for what acts on it, moving its tab above
/// all, which a drag must never be the only way to do.
fn window_menu(win: &BrowserWindow, _: &mut Window, cx: &mut Context<BrowserWindow>) -> Option<AnyElement> {
    if app::is_first(cx, win.id) {
        return None;
    }
    let me = win.id;
    let button = menu_button("window-menu", Icon::Menu, move |menu, _, cx| {
        let Some(win) = app::window(cx, me) else { return menu };
        let win = win.read(cx);
        let at = win.tabs.iter().position(|t| t.id == win.active).unwrap_or(0);
        let count = win.tabs.len();
        let others: Vec<(WindowId, String)> = app::windows(cx)
            .into_iter()
            .filter(|(id, _)| *id != me)
            .map(|(id, w)| (id, w.read(cx).window_label()))
            .collect();
        let mut menu = menu
            .item("New Tab", actions::NewTab)
            .item("New Window", actions::NewWindow)
            .item("Close Tab", actions::CloseTab)
            .entry(MenuItem::action("Put Tab to Sleep", actions::SleepTab).enabled(win.can_sleep(win.active)))
            .separator()
            .entry(MenuItem::action("Move Tab Left", actions::MoveTabLeft).enabled(at > 0))
            .entry(MenuItem::action("Move Tab Right", actions::MoveTabRight).enabled(at + 1 < count))
            .entry(MenuItem::action("Move Tab to New Window", actions::MoveTabToNewWindow).enabled(count > 1));
        if !others.is_empty() {
            let targets = others.into_iter().map(|(id, label)| MenuItem::action(label, MoveTabToWindow(id)));
            menu = menu.submenu(Menu::new("Move Tab to Window").items(targets));
        }
        menu.separator()
            .item("Find in Page", actions::FindInPage)
            .item("History", actions::ShowHistory)
            .item("Downloads", actions::ShowDownloads)
            .item("Bookmarks", actions::ShowBookmarks)
            .item("Settings", actions::OpenSettings)
    })
    .tooltip("Main Menu");
    Some(button.into_any_element())
}

impl BrowserWindow {
    /// Hands the list its rows, each versioned by what it draws so a change re-renders it.
    pub fn sync_rows(&mut self, cx: &mut Context<Self>) {
        let rows: Vec<(SidebarRow, u64)> = self
            .tabs
            .iter()
            .map(|t| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                (t.label(), &t.favicon, t.rt.loading, t.asleep, self.active == t.id, self.hovered == Some(t.id))
                    .hash(&mut h);
                (SidebarRow::Tab(t.id), h.finish())
            })
            .chain((!linux()).then_some((SidebarRow::NewTab, 0)))
            .collect();
        self.rows.set_items_versioned(rows, cx);
        self.rows.set_selected(vec![SidebarRow::Tab(self.active)], cx);
    }

    pub fn sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mac = nd::platform::target() == nd::platform::Target::MacOs;
        let gap = cx.theme().spacing.xs;
        // On macOS the close button's centre sits MAC_MARGIN from the window's left and top
        // edges, as the system places it in a window with a title bar.
        let controls = drag_region()
            .id("controls-row")
            .flex()
            .flex_row()
            .h(px(CONTROLS_HEIGHT))
            .flex_none()
            .items_center()
            .when(mac, |row| row.mt(px(MAC_MARGIN - CONTROLS_HEIGHT / 2.)).pl(px(MAC_MARGIN - LIGHT_RADIUS)))
            .child(window_controls().embedded());
        let rows = list("tabs", &self.rows, cx.processor(Self::row))
            .inset()
            .estimated_row_height(px(ROW_HEIGHT))
            .selection(Selection::single())
            .selectable(|row, _| matches!(row, SidebarRow::Tab(_)))
            .on_select(cx.listener(|win, e: &SelectEvent<SidebarRow>, window, cx| match e.key {
                // A click on the row on show opens the command bar on its address.
                SidebarRow::Tab(id) if id == win.active => {
                    cx.dispatch_action(window.id(), &actions::OpenAddress);
                    win.sync_rows(cx);
                }
                SidebarRow::Tab(id) => win.select(id, cx),
                SidebarRow::NewTab => {}
            }))
            .flex_1()
            .min_h_0();
        v_flex()
            .id("sidebar")
            .size_full()
            .context_menu(|menu, _, _| column_menu(menu))
            .child(controls)
            .child(rows)
            .gap(gap)
            .into_any_element()
    }

    pub fn foot(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let (gap, pad) = (t.spacing.xs, t.spacing.sm);
        let mut foot = h_flex().id("sidebar-foot").flex_none().items_center().gap(gap).px(pad).pb(pad);
        for slot in FOOT {
            if let Some(el) = slot(self, window, cx) {
                foot = foot.child(el);
            }
        }
        if linux() {
            let plus =
                icon_button("sidebar-new-tab", Icon::Plus).tooltip("New Tab").on_click(|_: &ClickEvent, window, cx| {
                    cx.dispatch_action(window.id(), &actions::NewTab);
                });
            foot = foot.child(div().flex_1()).child(plus);
        }
        foot.into_any_element()
    }

    fn row(&mut self, row: SidebarRow, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let id = match row {
            SidebarRow::Tab(id) => id,
            SidebarRow::NewTab => return new_tab_row(cx),
        };
        let Some(tab) = self.tab(id) else { return div().into_any_element() };
        let active = self.active == id;
        let hovered = self.hovered == Some(id);
        let t = cx.theme();
        let (gap, pad, hover_fill) = (t.spacing.sm, t.spacing.sm, t.surface.raised_hover);
        let favicon = match &tab.favicon {
            // A page names its icon before anyone knows it loads; one that fails gives way to
            // the globe.
            Some(url) => image(url.to_string())
                .size(px(FAVICON))
                .on_error(cx.listener(move |win, _: &ImageFailed, _, cx| {
                    if let Some(tab) = win.tab_mut(id) {
                        tab.favicon = None;
                    }
                    win.changed(cx);
                }))
                .into_any_element(),
            None => icon(Icon::Globe, FAVICON).into_any_element(),
        };
        let title = label(tab.label()).id(format!("tab-title-{id}")).truncate();
        let title = if active { title } else { title.secondary() };
        let trail: AnyElement = if hovered || active {
            icon_button(format!("tab-close-{id}"), Icon::X)
                .tooltip(format!("Close {}", tab.label()))
                .on_click(cx.listener(move |win, _: &ClickEvent, _, cx| win.close_tab(id, cx)))
                .into_any_element()
        } else if tab.rt.loading {
            spinner().id(format!("tab-spinner-{id}")).into_any_element()
        } else {
            div().into_any_element()
        };
        let facts =
            RowFacts { sleeps: self.can_sleep(id), addressed: !tab.url.is_empty(), alone: self.tabs.len() == 1 };
        h_flex()
            .id(format!("tab-row-{id}"))
            .h(px(TAB_ROW_HEIGHT))
            .mb(px(ROW_GAP))
            .px(pad)
            .gap(gap)
            .items_center()
            .rounded(t.radius.md)
            .when(!active, |row| row.hover(|s| s.bg(hover_fill)))
            .when(tab.asleep, |row| row.opacity(0.6))
            .on_hover(cx.listener(move |win, over: &bool, _, cx| {
                let now = if *over { Some(id) } else { win.hovered.filter(|h| *h != id) };
                if win.hovered != now {
                    win.hovered = now;
                    win.sync_rows(cx);
                    cx.notify();
                }
            }))
            .context_menu(move |menu, _, _| row_menu(menu, id, facts))
            .child(favicon)
            .child(title.flex_1().min_w_0())
            .child(div().w(px(TRAIL_WIDTH)).flex_none().flex().justify_center().child(trail))
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
struct RowFacts {
    sleeps: bool,
    addressed: bool,
    /// The window's only tab, which a new window would take as it is.
    alone: bool,
}

/// A tab row's menu. Pin and mute arrive with their lanes, so they show disabled.
fn row_menu(menu: ContextMenu, id: TabId, facts: RowFacts) -> ContextMenu {
    let item = |label: &'static str, command| MenuItem::action(label, ForTab(id, command));
    menu.entry(item("Reload", RowCommand::Reload))
        .entry(item("Duplicate Tab", RowCommand::Duplicate).enabled(facts.addressed))
        .entry(item("Copy Address", RowCommand::CopyAddress).enabled(facts.addressed))
        .entry(MenuItem::action("Pin Tab", actions::TogglePin).enabled(false))
        .entry(MenuItem::action("Mute Tab", actions::ToggleMute).enabled(false))
        .entry(item("Move Tab to New Window", RowCommand::MoveToNewWindow).enabled(!facts.alone))
        .entry(item("Put Tab to Sleep", RowCommand::Sleep).enabled(facts.sleeps))
        .separator()
        .entry(item("Close Tab", RowCommand::Close))
        .entry(item("Close Other Tabs", RowCommand::CloseOthers))
}

/// The menu on the column off any tab. The compact layout arrives with its lane.
fn column_menu(menu: ContextMenu) -> ContextMenu {
    menu.item("New Tab", actions::NewTab)
        .item("Hide Sidebar", actions::ToggleSidebar)
        .entry(MenuItem::action("Use Compact Layout", actions::SwitchLayout).enabled(false))
}

fn linux() -> bool {
    nd::platform::target() == nd::platform::Target::Linux
}

fn new_tab_row(cx: &mut Context<BrowserWindow>) -> AnyElement {
    let t = cx.theme();
    let (gap, pad, hover_fill) = (t.spacing.sm, t.spacing.sm, t.surface.raised_hover);
    h_flex()
        .id("new-tab-row")
        .h(px(TAB_ROW_HEIGHT))
        .px(pad)
        .gap(gap)
        .items_center()
        .rounded(t.radius.md)
        .opacity(0.6)
        .hover(|s| s.bg(hover_fill).opacity(1.))
        .on_tap(cx.listener(|win, _: &TapEvent, window, cx| crate::shell::new_tab(win, window, cx)))
        .child(icon(Icon::Plus, FAVICON))
        .child(label("New tab").secondary())
        .into_any_element()
}
