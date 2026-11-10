//! Find in page: Cmd+F opens a bar over the page's top trailing corner, Cmd+G and Shift+Cmd+G
//! step through the matches, Esc closes it. Each tab keeps its own query and count.

use lynk_core::tabstate::{FindState, find_failed, find_summary};
use nd::prelude::*;
use nd::webview::{FindOptions, FindResult};

use crate::tab::TabId;
use crate::window::BrowserWindow;

/// The key context of the bar, where Esc closes it.
pub const CONTEXT: &str = "FindBar";
pub const WIDTH: f32 = 420.;
const FIELD: &str = "find-field";

actions!(lynk_find, [Close]);

pub fn open(win: &mut BrowserWindow, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.tab_mut(win.active) else { return };
    tab.find.open = true;
    window.focus(FIELD);
    cx.notify();
}

pub fn next(win: &mut BrowserWindow, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    step(win, true, window, cx);
}

pub fn previous(win: &mut BrowserWindow, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    step(win, false, window, cx);
}

/// Cmd+G with the bar closed opens it on the last query, as Chrome does.
fn step(win: &mut BrowserWindow, forward: bool, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.tab_mut(win.active) else { return };
    if !tab.find.open {
        tab.find.open = true;
        window.focus(FIELD);
    }
    if let Some(page) = &tab.page
        && !tab.find.query.is_empty()
    {
        if forward { page.find_next() } else { page.find_previous() }
    }
    cx.notify();
}

pub fn close(win: &mut BrowserWindow, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.tab_mut(win.active) else { return };
    tab.find = FindState { query: std::mem::take(&mut tab.find.query), ..FindState::default() };
    if let Some(page) = &tab.page {
        page.stop_find();
        page.focus();
    }
    cx.notify();
}

fn search(win: &mut BrowserWindow, id: TabId, query: String, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.tab_mut(id) else { return };
    tab.find.count = None;
    tab.find.found = None;
    if let Some(page) = &tab.page {
        if query.is_empty() { page.stop_find() } else { page.find(query.clone(), FindOptions::default()) }
    }
    tab.find.query = query;
    cx.notify();
}

pub fn result(win: &mut BrowserWindow, id: TabId, e: &FindResult, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.tab_mut(id) else { return };
    if tab.find.query.is_empty() {
        return;
    }
    tab.find.found = Some(e.match_found);
    tab.find.count = e.match_count;
    cx.notify();
}

/// The bar for the shown tab, while it is open.
pub fn bar(win: &BrowserWindow, cx: &mut Context<BrowserWindow>) -> Option<AnyElement> {
    let tab = win.active_tab().filter(|t| t.find.open)?;
    let id = tab.id;
    let t = cx.theme();
    let (gap, pad, radius, surface) = (t.spacing.xs, t.spacing.sm, t.radius.md, t.surface.raised);
    let summary = find_summary(&tab.find);
    let failed = find_failed(&tab.find);
    let count = label(summary).id("find-count").tone(if failed { LabelTone::Danger } else { LabelTone::Secondary });
    Some(
        h_flex()
            .id("find-bar")
            .key_context(CONTEXT)
            .on_action(cx.listener(|win, _: &Close, window, cx| close(win, window, cx)))
            .w(px(WIDTH))
            .gap(gap)
            .p(pad)
            .items_center()
            .rounded(radius)
            .bg(surface)
            .shadow_md()
            .child(
                text_field(FIELD)
                    .placeholder("Find in Page")
                    .text(tab.find.query.clone())
                    .on_change(cx.listener(move |win, e: &TextChanged, _, cx| search(win, id, e.text.to_string(), cx)))
                    .on_submit(cx.listener(|win, _: &TextSubmitted, window, cx| next(win, window, cx)))
                    .flex_1(),
            )
            .child(count)
            .child(
                icon_button("find-previous", Icon::ChevronUp)
                    .tooltip("Previous match")
                    .on_click(cx.listener(|win, _: &ClickEvent, window, cx| previous(win, window, cx))),
            )
            .child(
                icon_button("find-next", Icon::ChevronDown)
                    .tooltip("Next match")
                    .on_click(cx.listener(|win, _: &ClickEvent, window, cx| next(win, window, cx))),
            )
            .child(
                icon_button("find-close", Icon::X)
                    .tooltip("Close")
                    .on_click(cx.listener(|win, _: &ClickEvent, window, cx| close(win, window, cx))),
            )
            .into_any_element(),
    )
}
