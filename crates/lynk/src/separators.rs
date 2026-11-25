//! Named separators in the sidebar's list: one added under a tab opens with its name field
//! focused, a rename edits the name in place, a delete leaves the tabs around it, and it drags
//! like a tab row. The tree keeps them (`lynk_core::sidebar_tree`); this module keeps which one
//! each window is naming and draws the row.

use std::collections::HashMap;

use nd::core::WindowId;
use nd::prelude::*;

use crate::actions::{self, DeleteSeparator, RenameSeparator};
use crate::app;
use crate::sidebar::{ICON_GAP, ROW_GAP, ROW_LEAD, ROW_TAIL, TRAIL_WIDTH};
use crate::tab::TabId;
use crate::window::BrowserWindow;
use lynk_core::session::tab_id;

const FIELD: &str = "separator-name";
pub const HEIGHT: f32 = 24.;
/// The rule, as a share of the primary ink so one number holds in light and dark.
const RULE: f32 = 0.16;

/// The separator each window is naming.
#[derive(Default)]
struct Naming(HashMap<WindowId, String>);

fn naming(cx: &App, win: WindowId) -> Option<&str> {
    cx.try_global::<Naming>()?.0.get(&win).map(String::as_str)
}

pub fn forget(cx: &mut App, win: WindowId) {
    if let Some(naming) = cx.try_global_mut::<Naming>() {
        naming.0.remove(&win);
    }
}

fn start(win: &mut BrowserWindow, id: String, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    if !cx.has_global::<Naming>() {
        cx.set_global(Naming::default());
    }
    cx.global_mut::<Naming>().0.insert(win.id, id);
    window.focus(FIELD);
    win.sync_rows(cx);
    cx.notify();
}

/// Ends the naming: `name` is kept, `None` leaves the name the separator had.
fn finish(win: &mut BrowserWindow, id: &str, name: Option<&str>, cx: &mut Context<BrowserWindow>) {
    if naming(cx, win.id) != Some(id) {
        return;
    }
    forget(cx, win.id);
    if let Some(name) = name {
        win.rename_separator(id, name, cx);
    }
    win.sync_rows(cx);
    cx.notify();
}

/// "Add Separator Below": under the tab's row, its name field focused.
pub fn add_below(win: &mut BrowserWindow, tab: TabId, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    let id = app::next_item_id(cx);
    if win.add_separator_below(tab, &id, cx) {
        start(win, id, window, cx);
    }
}

pub fn can_add_below(win: &BrowserWindow, tab: TabId, _: &App) -> bool {
    win.tab(tab).is_some() && !win.is_pinned(tab)
}

/// The separator the keyboard's commands act on: the nearest one above the tab.
fn above(win: &BrowserWindow, tab: TabId) -> Option<String> {
    win.tree().separator_above(&tab_id(tab)).map(str::to_owned)
}

pub fn has_above(win: &BrowserWindow, tab: TabId, _: &App) -> bool {
    above(win, tab).is_some()
}

pub fn rename_above(win: &mut BrowserWindow, tab: TabId, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    if let Some(id) = above(win, tab) {
        start(win, id, window, cx);
    }
}

pub fn delete_above(win: &mut BrowserWindow, tab: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    if let Some(id) = above(win, tab) {
        delete(win, &id, cx);
    }
}

/// The separator's own menu item, or the bar's.
pub fn rename(win: &mut BrowserWindow, a: &RenameSeparator, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    start(win, a.0.to_string(), window, cx);
}

pub fn delete_row(win: &mut BrowserWindow, a: &DeleteSeparator, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    delete(win, &a.0, cx);
}

fn delete(win: &mut BrowserWindow, id: &str, cx: &mut Context<BrowserWindow>) {
    if naming(cx, win.id) == Some(id) {
        forget(cx, win.id);
    }
    win.delete_separator(id, cx);
}

/// What the row's version hashes: a change to either redraws it.
pub fn drawn(cx: &App, win: WindowId, id: &str, name: &str) -> (String, bool) {
    (name.to_owned(), naming(cx, win) == Some(id))
}

/// A hairline across the row, after the name when it has one; while it is being named, the
/// field in the name's place.
pub fn row(win: &BrowserWindow, id: SharedString, name: &str, cx: &mut Context<BrowserWindow>) -> AnyElement {
    let ink = cx.theme().text.primary;
    let editing = naming(cx, win.id) == Some(id.as_ref());
    let me = win.id;
    let name_part: Option<AnyElement> = if editing {
        let (done, cancelled) = (id.clone(), id.clone());
        Some(
            text_field(FIELD)
                .placeholder("Separator name")
                .text(name.to_owned())
                .select_on_focus(0..name.chars().count())
                .on_end_editing(cx.listener(move |win, e: &TextEditingEnded, _, cx| {
                    finish(win, &done, Some(&e.text), cx);
                }))
                .on_cancel(cx.listener(move |win, _: &TextCancelled, _, cx| finish(win, &cancelled, None, cx)))
                .flex_1()
                .into_any_element(),
        )
    } else {
        (!name.is_empty()).then(|| {
            label(name.to_owned())
                .id(format!("separator-title-{id}"))
                .text_style(TextStyle::Caption1)
                .secondary()
                .truncate()
                .min_w_0()
                .into_any_element()
        })
    };
    let rule = (!editing).then(|| div().flex_1().min_w(px(12.)).h(px(1.)).bg(ink.opacity(RULE)));
    let menu_id = id.clone();
    h_flex()
        .id(format!("separator-{id}"))
        .h(px(HEIGHT))
        .mb(px(ROW_GAP))
        .pl(px(ROW_LEAD))
        .pr(px(ROW_TAIL + TRAIL_WIDTH / 2.))
        .gap(px(ICON_GAP))
        .items_center()
        .context_menu(move |menu, _, _| separator_menu(menu, me, &menu_id))
        .children(name_part)
        .children(rule)
        .into_any_element()
}

fn separator_menu(menu: ContextMenu, _: WindowId, id: &SharedString) -> ContextMenu {
    menu.item("Rename Separator…", actions::RenameSeparator(id.clone()))
        .item("Delete Separator", actions::DeleteSeparator(id.clone()))
}
