//! The sidebar: the controls row with the window's own buttons, the pinned tiles, the tab list
//! that scrolls, and the foot. Rows and tiles drag as one kind: a row dropped on the tiles pins
//! its tab, a tile dropped on the list unpins it, and tiles reorder among themselves. Separators
//! are rows of the list too, drawn by `separators`.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use lynk_core::session::{tab_id, tab_number};
use lynk_core::settings::{LynkSettings, PinStyle};
use lynk_core::sidebar_tree::{Item, SidebarTree};
use lynk_core::tabstate::tile_letter;
use nd::core::WindowId;
use nd::platform::{Target, target};
use nd::prelude::*;
use nd::virtual_list::{RowDropped, SelectEvent, Selection, list};

use crate::actions;
use crate::app;
use crate::tab::TabId;
use crate::window::BrowserWindow;

// The column's numbers (docs/sidebar.md, and the 0.5.11 app's `Sidebar.tsx` where it is
// silent). Every backend draws these; nothing below takes a size from the theme.
pub const WIDTH: f32 = 240.;
pub const TAB_ROW_HEIGHT: f32 = 28.;
pub const ROW_GAP: f32 = 2.;
/// A row and the gap under it.
pub const ROW_HEIGHT: f32 = TAB_ROW_HEIGHT + ROW_GAP;
const CONTROLS_HEIGHT: f32 = 32.;
/// macOS: the column's leading and trailing margin, and the close light's distance from the
/// window's left and top edges, as a window with a toolbar places it.
const MAC_MARGIN: f32 = 19.;
const LIGHT: f32 = 14.;
/// Elsewhere: the column's padding.
const INSET: f32 = 8.;
const ROW_RADIUS: f32 = 8.;

/// The column's leading and trailing margin on this platform.
pub fn margin() -> f32 {
    if target() == Target::MacOs { MAC_MARGIN } else { INSET }
}

/// Inside a row: before the favicon, between favicon and title, after the trailing slot.
pub(crate) const ROW_LEAD: f32 = 6.;
pub(crate) const ICON_GAP: f32 = 8.;
pub(crate) const ROW_TAIL: f32 = 2.;
const FAVICON: f32 = 16.;
/// Kept on every row, so a title never reflows as the pointer crosses the list.
pub(crate) const TRAIL_WIDTH: f32 = 24.;
/// The close button's hit area and the load spinner, inside the trailing slot.
const CLOSE_BOX: f32 = 20.;
const CLOSE_RADIUS: f32 = 6.;
const SPINNER: f32 = 14.;
/// Fills, as a share of the primary ink so one number holds in light and dark.
const SHOWN_FILL: f32 = 0.12;
const HOVER_FILL: f32 = 0.06;
const CLOSE_HOVER_FILL: f32 = 0.10;
/// A pressed row or button gives a little.
const PRESSED_SCALE: f32 = 0.98;
/// A sleeping tab's row, the resting New tab row and a resting close glyph.
const ASLEEP: f32 = 0.5;
const QUIET: f32 = 0.6;
/// The pinned grid (docs/sidebar.md): tiles at least 40 wide and 0.74 as tall, 6 apart, three
/// columns in a window narrower than about 900, four from there.
const TILE_MIN: f32 = 40.;
const TILE_ASPECT: f32 = 0.74;
const TILE_GAP: f32 = 6.;
const TILE_RADIUS: f32 = 10.;
const TILE_ICON: f32 = 20.;
/// The tab on show is the brighter pill: its glass tinted this much white.
/// Tile fills as black (light) or white (dark) at these alphas: at rest, under the pointer, and
/// the tab on show, which also takes the hairline. The shown fill is the reference's live pin
/// grey, loud enough to read as selected without the letter.
/// The empty pinned block: a strip a drag can find, and the target it opens into.
const EMPTY_TARGET_REST: f32 = 12.;
const EMPTY_TARGET_OPEN: f32 = 36.;
const TILE_FILLS: [(f32, f32); 3] = [(0.0314, 0.0432), (0.0419, 0.0595), (0.0995, 0.1243)];
const FOUR_COLUMNS_FROM: f64 = 900.;
/// Between the tiles and the first row.
const PINS_BOTTOM: f32 = 8.;
/// What rows and tiles drag as, each carrying its tab id.
const TAB_KIND: &str = "tab";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SidebarRow {
    Tab(TabId),
    /// A separator by its id.
    Separator(SharedString),
    /// The quiet "New tab" row after the tabs. GNOME's idiom is the bare plus in the foot
    /// instead, which is where Linux has it.
    NewTab,
}

/// The row under the pointer in each window, whose close button shows.
#[derive(Default)]
struct Hovered(HashMap<WindowId, TabId>);

fn hovered_row(cx: &App, win: WindowId) -> Option<TabId> {
    cx.try_global::<Hovered>()?.0.get(&win).copied()
}

/// A window listener: the window's hovered row goes with it.
pub fn forget(cx: &mut App, win: WindowId) {
    set_hovered(cx, win, None);
}

fn set_hovered(cx: &mut App, win: WindowId, id: Option<TabId>) {
    if !cx.has_global::<Hovered>() {
        cx.set_global(Hovered::default());
    }
    let rows = &mut cx.global_mut::<Hovered>().0;
    match id {
        Some(id) => rows.insert(win, id),
        None => rows.remove(&win),
    };
}

/// One slot of the foot for the window, or nothing while it has nothing to show.
pub type Slot = fn(&BrowserWindow, &mut Window, &mut Context<BrowserWindow>) -> Option<AnyElement>;

/// The foot, leading to trailing: settings, the padlock, zoom (while not 100%), blocked pop-ups,
/// a secondary window's own menu, the profile, the extensions (the puzzle, then pinned actions),
/// downloads.
pub const FOOT: &[Slot] =
    &[settings, crate::site_info::padlock, crate::zoom::glyph, window_menu, crate::profile::glyph];

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
        use actions::*;
        let Some(win) = app::window(cx, me) else { return menu };
        let win = win.read(cx);
        let others: Vec<(WindowId, String)> = app::windows_of(cx, &win.profile)
            .into_iter()
            .filter(|(id, _)| *id != me)
            .map(|(id, w)| (id, w.read(cx).window_label()))
            .collect();
        let item = |command| menu_item(command, win, cx);
        let mut menu = menu
            .entry(item(command_for::<NewTab>()))
            .entry(item(command_for::<NewWindow>()))
            .entry(item(command_for::<CloseTab>()))
            .entry(item(command_for::<SleepTab>()))
            .separator()
            .entry(item(command_for::<MoveTabLeft>()))
            .entry(item(command_for::<MoveTabRight>()))
            .entry(item(command_for::<MoveTabToNewWindow>()));
        if !others.is_empty() {
            let targets = others.into_iter().map(|(id, label)| MenuItem::action(label, MoveTabToWindow(id)));
            menu = menu.submenu(Menu::new("Move Tab to Window").items(targets));
        }
        menu.separator()
            .entry(item(command_for::<FindInPage>()))
            .entry(item(command_for::<ShowHistory>()))
            .entry(item(command_for::<ShowDownloads>()))
            .entry(item(command_for::<ShowBookmarks>()))
            .entry(item(command_for::<OpenSettings>()))
    })
    .tooltip("Main Menu");
    Some(button.into_any_element())
}

impl BrowserWindow {
    /// Hands the list its rows, each versioned by what it draws so a change re-renders it.
    pub fn sync_rows(&mut self, cx: &mut Context<Self>) {
        let hovered = hovered_row(cx, self.id);
        let rows: Vec<(SidebarRow, u64)> = list_rows(self.tree())
            .into_iter()
            .filter_map(|(row, _)| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                match &row {
                    SidebarRow::Tab(id) => {
                        let t = self.tab(*id)?;
                        (
                            t.label(),
                            &t.favicon,
                            t.rt.loading,
                            t.is_asleep(),
                            self.active == t.id,
                            hovered == Some(t.id),
                        )
                            .hash(&mut h);
                    }
                    SidebarRow::Separator(id) => {
                        crate::separators::drawn(cx, self.id, id, separator_name(self.tree(), id)).hash(&mut h)
                    }
                    SidebarRow::NewTab => {}
                }
                Some((row, h.finish()))
            })
            .chain((target() != Target::Linux).then_some((SidebarRow::NewTab, 0)))
            .collect();
        self.rows.set_items_versioned(rows, cx);
        let shown = if self.is_pinned(self.active) { Vec::new() } else { vec![SidebarRow::Tab(self.active)] };
        self.rows.set_selected(shown, cx);
    }

    pub fn sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mac = target() == Target::MacOs;
        let margin = margin();
        // The close light's top edge sits MAC_MARGIN down, centred in the controls row.
        let controls = drag_region()
            .id("controls-row")
            .flex()
            .flex_row()
            .h(px(CONTROLS_HEIGHT))
            .flex_none()
            .items_center()
            .when(mac, |row| row.mt(px(MAC_MARGIN - (CONTROLS_HEIGHT - LIGHT) / 2.)))
            .child(window_controls().embedded());
        // The rows draw their own fills; the list's selection is for the keyboard and assistive
        // tech.
        let rows = list("tabs", &self.rows, cx.processor(Self::row))
            .own_highlight()
            .estimated_row_height(px(ROW_HEIGHT))
            .selection(Selection::single())
            .selectable(|row, _| matches!(row, SidebarRow::Tab(_)))
            .drag_rows(TAB_KIND, |row| match row {
                SidebarRow::Tab(id) => Some(tab_id(*id).into()),
                SidebarRow::Separator(id) => Some(id.clone()),
                SidebarRow::NewTab => None,
            })
            .accepts_drops([TAB_KIND])
            .on_drop(cx.listener(|win, e: &RowDropped<SidebarRow>, _, cx| {
                let slot = tree_slot(win.tree(), e.index);
                match tab_number(&e.item.id) {
                    Some(id) => win.drop_in_list(id, slot, cx),
                    None => win.drop_separator(&e.item.id, slot, cx),
                }
            }))
            .on_select(cx.listener(|win, e: &SelectEvent<SidebarRow>, window, cx| match e.key {
                // A click on the row on show opens the command bar on its address.
                SidebarRow::Tab(id) if id == win.active => {
                    cx.dispatch_action(window.id(), &actions::OpenAddress);
                    win.sync_rows(cx);
                }
                SidebarRow::Tab(id) => win.select(id, cx),
                SidebarRow::Separator(_) | SidebarRow::NewTab => {}
            }))
            .flex_1()
            .min_h_0();
        let me = self.id;
        v_flex()
            .id("sidebar")
            .size_full()
            .px(px(margin))
            .when(!mac, |col| col.pt(px(INSET)))
            .context_menu(move |menu, _, cx| column_menu(menu, me, cx))
            .child(controls)
            .children(self.pinned(cx))
            .child(rows)
            .into_any_element()
    }

    pub fn foot(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // The glyphs stand on the favicons' column.
        let mac = target() == Target::MacOs;
        let margin = margin() + ROW_LEAD;
        // On macOS the foot sits as far up from the bottom edge as the rows sit in from the left.
        let bottom = if mac { MAC_MARGIN - ROW_LEAD } else { INSET };
        let mut foot =
            h_flex().id("sidebar-foot").flex_none().items_center().gap(px(ROW_GAP)).px(px(margin)).pb(px(bottom));
        for slot in FOOT {
            if let Some(el) = slot(self, window, cx) {
                foot = foot.child(el);
            }
        }
        if target() == Target::Linux {
            let plus =
                icon_button("sidebar-new-tab", Icon::Plus).tooltip("New Tab").on_click(|_: &ClickEvent, window, cx| {
                    cx.dispatch_action(window.id(), &actions::NewTab);
                });
            foot = foot.child(div().flex_1()).child(plus);
        }
        foot.into_any_element()
    }

    /// Arc's favourites grid, or nothing while no tab is pinned.
    fn pinned(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pins: Vec<TabId> = self.tree().pins.iter().filter_map(|p| tab_number(p)).collect();
        if pins.is_empty() {
            return Some(self.pin_target(cx));
        }
        let columns = if self.frame.width < FOUR_COLUMNS_FROM { 3 } else { 4 };
        let tiles: Vec<Tile> = pins.iter().filter_map(|id| Some(tile(tab_id(*id), self.tile(*id, cx)?))).collect();
        let grid = tile_grid("pins")
            .max_columns(columns)
            .min_tile_width(TILE_MIN)
            .aspect(TILE_ASPECT)
            .gap(TILE_GAP)
            .reflow(Animation::ease_out(std::time::Duration::from_millis(200)))
            .tiles(tiles)
            .draggable_tiles(TAB_KIND)
            .accepts_drops([TAB_KIND])
            .on_move(cx.listener(|win, e: &TileMoved, _, cx| {
                if let Some(id) = tab_number(&e.id) {
                    win.pin_at(id, e.index, cx);
                }
            }))
            .on_drop(cx.listener(|win, e: &Dropped, _, cx| {
                if let (Some(id), Some(slot)) = (tab_number(&e.item.id), &e.slot) {
                    win.pin_at(id, slot.index, cx);
                }
            }));
        Some(div().id("pinned").flex_none().pb(px(PINS_BOTTOM)).child(grid).into_any_element())
    }

    /// The pinned block with nothing pinned: a strip under the controls that a dragged tab grows
    /// into a tile-high target, and pins on a drop.
    fn pin_target(&self, cx: &mut Context<Self>) -> AnyElement {
        let over = self.pin_target;
        let theme = cx.theme();
        let target = div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(TILE_RADIUS))
            .border_1()
            .border_color(theme.border.subtle)
            .bg(theme.text.primary.opacity(TILE_FILLS[1].0))
            .child(label("Drop to pin").id("pin-target-label").secondary());
        div()
            .id("pin-target")
            .flex_none()
            .w_full()
            .h(px(if over { EMPTY_TARGET_OPEN } else { EMPTY_TARGET_REST }))
            .mb(px(if over { PINS_BOTTOM } else { 0. }))
            .accepts_drops([TAB_KIND])
            .on_drag_over(cx.listener(|win, e: &DragOver, _, cx| {
                let over = e.phase != DragOverPhase::Exited;
                if win.pin_target != over {
                    win.pin_target = over;
                    cx.notify();
                }
            }))
            .on_drop(cx.listener(|win, e: &Dropped, _, cx| {
                win.pin_target = false;
                if let Some(id) = tab_number(&e.item.id) {
                    win.pin_at(id, 0, cx);
                }
                cx.notify();
            }))
            .when(over, |strip| strip.child(target))
            .into_any_element()
    }

    /// One pinned tile: the site's icon, or its letter until there is one (only the letter under
    /// `pinStyle = "letters"`). The tab on show takes the darker fill and the hairline, its letter
    /// in full ink.
    fn tile(&self, id: TabId, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = self.tab(id)?;
        let me = self.id;
        let active = self.active == id;
        let mac = target() == Target::MacOs;
        let icons = cx.settings::<LynkSettings>().pin_style == PinStyle::Icons;
        let face = match tab.favicon.as_ref().filter(|_| icons) {
            Some(url) => image(url.to_string())
                .size(px(TILE_ICON))
                .on_error(cx.listener(move |win, _: &ImageFailed, _, cx| {
                    if let Some(tab) = win.tab_mut(id) {
                        tab.favicon = None;
                    }
                    win.changed(cx);
                }))
                .into_any_element(),
            None => {
                let letter = label(tile_letter(&tab.label())).id(format!("pin-letter-{id}"));
                if active { letter } else { letter.secondary() }.into_any_element()
            }
        };
        let theme = cx.theme();
        let dark = theme.is_dark();
        let ground = hsla(0., 0., if dark { 1. } else { 0. }, 1.);
        let [rest, hover, shown] =
            TILE_FILLS.map(|(light, dark_alpha)| ground.opacity(if dark { dark_alpha } else { light }));
        let hairline = theme.border.subtle;
        let face = div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(TILE_RADIUS))
            .when(active, |f| f.bg(shown).border_1().border_color(hairline))
            .when(!active, |f| f.bg(rest).hover(|s| s.bg(hover)))
            .child(div().when(tab.is_asleep(), |f| f.opacity(ASLEEP)).child(face));
        let body = if mac {
            glass()
                .size_full()
                .rounded(px(TILE_RADIUS))
                .variant(GlassVariant::Regular)
                .interactive()
                .child(face)
                .into_any_element()
        } else {
            face.into_any_element()
        };
        Some(
            div()
                .id(format!("pin-{id}"))
                .role(
                    nd::core::semantic::Role::Button,
                    if tab.muted { format!("{} - Audio muted", tab.label()) } else { tab.label() },
                )
                .size_full()
                .active(|pose| pose.scale(PRESSED_SCALE))
                .on_tap(cx.listener(move |win, _: &TapEvent, window, cx| {
                    if win.active == id {
                        cx.dispatch_action(window.id(), &actions::OpenAddress);
                    } else {
                        win.select(id, cx);
                    }
                }))
                .context_menu(move |menu, _, cx| pin_menu(menu, me, id, cx))
                .child(body)
                .into_any_element(),
        )
    }

    fn row(&mut self, row: SidebarRow, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let id = match row {
            SidebarRow::Tab(id) => id,
            SidebarRow::Separator(id) => {
                let name = separator_name(self.tree(), &id).to_owned();
                return crate::separators::row(self, id, &name, cx);
            }
            SidebarRow::NewTab => return new_tab_row(cx),
        };
        let Some(tab) = self.tab(id) else { return div().into_any_element() };
        let me = self.id;
        let active = self.active == id;
        let hovered = hovered_row(cx, me) == Some(id);
        let ink = cx.theme().text.primary;
        let favicon = match &tab.favicon {
            // A page names its icon before anyone knows it loads; one that fails gives way to
            // the globe.
            Some(url) => div()
                .id(format!("tab-favicon-{id}"))
                .flex_none()
                .child(image(url.to_string()).size(px(FAVICON)).on_error(cx.listener(
                    move |win, _: &ImageFailed, _, cx| {
                        if let Some(tab) = win.tab_mut(id) {
                            tab.favicon = None;
                        }
                        win.changed(cx);
                    },
                )))
                .into_any_element(),
            None => icon(Icon::Globe, FAVICON).into_any_element(),
        };
        let title = label(tab.label()).id(format!("tab-title-{id}")).truncate();
        let title = if active { title } else { title.secondary() };
        // The spinner gives way to the close button under the pointer, so a loading tab can
        // still be closed from its row.
        let trail: AnyElement = if tab.rt.loading && !hovered {
            spinner().id(format!("tab-spinner-{id}")).size(px(SPINNER)).into_any_element()
        } else if hovered || active {
            close_button(id, tab.label(), ink, cx)
        } else {
            div().into_any_element()
        };
        h_flex()
            .id(format!("tab-row-{id}"))
            .h(px(TAB_ROW_HEIGHT))
            .mb(px(ROW_GAP))
            .pl(px(ROW_LEAD))
            .pr(px(ROW_TAIL))
            .gap(px(ICON_GAP))
            .items_center()
            .rounded(px(ROW_RADIUS))
            .when(active, |row| row.bg(ink.opacity(SHOWN_FILL)))
            .when(!active, |row| row.hover(|s| s.bg(ink.opacity(HOVER_FILL))))
            .active(|pose| pose.scale(PRESSED_SCALE))
            .on_hover(cx.listener(move |win, over: &bool, _, cx| {
                let was = hovered_row(cx, win.id);
                let now = if *over { Some(id) } else { was.filter(|h| *h != id) };
                if was != now {
                    set_hovered(cx, win.id, now);
                    win.sync_rows(cx);
                    cx.notify();
                }
            }))
            // The list keeps the shown row selected, so a click on it selects nothing new.
            .when(active, |row| {
                row.on_tap(cx.listener(|_, _: &TapEvent, window, cx| {
                    cx.dispatch_action(window.id(), &actions::OpenAddress);
                }))
            })
            .context_menu(move |menu, _, cx| row_menu(menu, me, id, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(ICON_GAP))
                    .items_center()
                    .when(tab.is_asleep(), |content| content.opacity(ASLEEP))
                    .child(favicon)
                    .child(title.flex_1().min_w_0()),
            )
            .child(div().w(px(TRAIL_WIDTH)).flex_none().flex().items_center().justify_center().child(trail))
            .into_any_element()
    }
}

/// The row's close button: a quiet glyph that comes up to full ink with its own fill under the
/// pointer, on a hit area larger than the glyph. The native button inside keeps the keyboard and
/// assistive tech able to press it.
fn close_button(id: TabId, label: String, ink: Hsla, cx: &mut Context<BrowserWindow>) -> AnyElement {
    let button = icon_button(format!("tab-close-{id}"), Icon::X)
        .tooltip(format!("Close {label}"))
        .size(px(CLOSE_BOX))
        .on_click(cx.listener(move |win, _: &ClickEvent, _, cx| win.close_tab(id, cx)));
    div()
        .size(px(CLOSE_BOX))
        .flex_none()
        .rounded(px(CLOSE_RADIUS))
        .opacity(QUIET)
        .hover(|s| s.bg(ink.opacity(CLOSE_HOVER_FILL)).opacity(1.))
        .active(|pose| pose.scale(PRESSED_SCALE))
        .child(button)
        .into_any_element()
}

/// A pinned tile's menu: the few commands that make sense for a page kept on purpose.
fn pin_menu(menu: ContextMenu, win: WindowId, id: TabId, cx: &App) -> ContextMenu {
    use actions::*;
    let Some(win) = app::window(cx, win) else { return menu };
    let win = win.read(cx);
    let item = |command| item_for_tab(command, win, id, cx);
    menu.entry(item(command_for::<ResetPinnedPage>()))
        .entry(item(command_for::<TogglePin>()))
        .entry(item(command_for::<ToggleMute>()))
        .entry(item(command_for::<SleepTab>()))
        .separator()
        .entry(item(command_for::<CloseTab>()))
}

/// A tab row's menu, acting on the row's tab.
fn row_menu(menu: ContextMenu, win: WindowId, id: TabId, cx: &App) -> ContextMenu {
    use actions::*;
    let Some(win) = app::window(cx, win) else { return menu };
    let win = win.read(cx);
    let item = |command| item_for_tab(command, win, id, cx);
    menu.entry(item(command_for::<Reload>()))
        .entry(item(command_for::<DuplicateTab>()))
        .entry(item(command_for::<CopyAddress>()))
        .entry(item(command_for::<TogglePin>()))
        .entry(item(command_for::<ToggleMute>()))
        .entry(item(command_for::<MoveTabToNewWindow>()))
        .entry(item(command_for::<SleepTab>()))
        .separator()
        .entry(item(command_for::<AddSeparatorBelow>()))
        .separator()
        .entry(item(command_for::<CloseTab>()))
        .entry(item(command_for::<CloseOtherTabs>()))
        .entry(item(command_for::<CloseTabsBelow>()))
}

/// The list's rows in drawing order, each with the index of the tree row it belongs to: a tab
/// in a folder or split draws as a row of its own under its container's index.
fn list_rows(tree: &SidebarTree) -> Vec<(SidebarRow, usize)> {
    let tab = |id: &str, at| tab_number(id).map(|n| (SidebarRow::Tab(n), at));
    tree.items
        .iter()
        .enumerate()
        .flat_map(|(at, item)| -> Vec<(SidebarRow, usize)> {
            match item {
                Item::Tab { id } => tab(id, at).into_iter().collect(),
                Item::Separator { id, .. } => vec![(SidebarRow::Separator(id.clone().into()), at)],
                Item::Folder(_) | Item::Split(_) => {
                    let mut one = SidebarTree { items: vec![item.clone()], ..SidebarTree::default() };
                    one.pins.clear();
                    one.tab_ids().into_iter().filter_map(|id| tab(id, at)).collect()
                }
            }
        })
        .collect()
}

/// The gap the list marked before row `slot`, as a gap in the tree's own list.
fn tree_slot(tree: &SidebarTree, slot: usize) -> usize {
    list_rows(tree).get(slot).map_or(tree.items.len(), |(_, at)| *at)
}

pub fn separator_name<'a>(tree: &'a SidebarTree, id: &str) -> &'a str {
    tree.items
        .iter()
        .find_map(|i| match i {
            Item::Separator { id: s, name } if s == id => Some(name.as_str()),
            _ => None,
        })
        .unwrap_or_default()
}

/// The menu on the column off any tab. The compact layout arrives with its lane.
fn column_menu(menu: ContextMenu, win: WindowId, cx: &App) -> ContextMenu {
    let menu = menu
        .item("New Tab", actions::NewTab)
        .item("Hide Sidebar", actions::ToggleSidebar)
        .entry(MenuItem::action("Use Compact Layout", actions::SwitchLayout).enabled(false));
    let Some(win) = app::window(cx, win) else { return menu };
    let win = win.read(cx);
    let item = |command| actions::item_for_tab(command, win, win.active, cx);
    menu.separator()
        .entry(item(actions::command_for::<actions::TidyTabs>()))
        .entry(item(actions::command_for::<actions::UndoTidy>()))
}

fn new_tab_row(cx: &mut Context<BrowserWindow>) -> AnyElement {
    let ink = cx.theme().text.primary;
    h_flex()
        .id("new-tab-row")
        .h(px(TAB_ROW_HEIGHT))
        .pl(px(ROW_LEAD))
        .pr(px(ROW_TAIL))
        .gap(px(ICON_GAP))
        .items_center()
        .rounded(px(ROW_RADIUS))
        .opacity(QUIET)
        .hover(|s| s.bg(ink.opacity(HOVER_FILL)).opacity(1.))
        .active(|pose| pose.scale(PRESSED_SCALE))
        .on_tap(cx.listener(|_, _: &TapEvent, window, cx| {
            cx.dispatch_action(window.id(), &actions::NewTab);
        }))
        .child(icon(Icon::Plus, FAVICON))
        .child(label("New tab").secondary())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use nd::test::TestApp;

    use super::*;

    #[test]
    fn a_closed_window_keeps_no_hovered_row() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let a = app::open_window(&mut cx, Vec::new(), None);
        let _b = app::open_window(&mut cx, Vec::new(), None);
        cx.run_until_idle();
        set_hovered(&mut cx, a, Some(1));
        assert!(cx.request_close(a));
        assert_eq!(hovered_row(&cx, a), None);
    }

    #[test]
    fn a_gap_in_the_list_is_the_same_gap_in_the_tree() {
        let json = r#"{"pins":["t9"],"items":[
            {"type":"tab","id":"t1"},
            {"type":"separator","id":"i1","name":"Work"},
            {"type":"split","id":"i2","tabs":["t2","t3"]},
            {"type":"tab","id":"t4"}],"lastActive":""}"#;
        let tree: SidebarTree = serde_json::from_str(json).unwrap();
        let rows: Vec<_> = list_rows(&tree);
        assert_eq!(
            rows,
            [
                (SidebarRow::Tab(1), 0),
                (SidebarRow::Separator("i1".into()), 1),
                (SidebarRow::Tab(2), 2),
                (SidebarRow::Tab(3), 2),
                (SidebarRow::Tab(4), 3),
            ]
        );
        assert_eq!((0..=5).map(|slot| tree_slot(&tree, slot)).collect::<Vec<_>>(), [0, 1, 2, 2, 3, 4]);
        assert_eq!(separator_name(&tree, "i1"), "Work");
    }
}
