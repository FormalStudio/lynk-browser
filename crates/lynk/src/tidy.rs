//! "Tidy Tabs": the window's loose tabs sorted into groups under named separators by
//! `lynk_core::tidy`, the same separators a user adds by hand, and "Undo Tidy" while the list is
//! as the tidy left it. The groups come from the system's on-device text model when it has one
//! (Apple's on macOS), checked by core, and otherwise from the site each tab is on.

use std::collections::HashMap;

use lynk_core::session::{tab_id, tab_number};
use lynk_core::tidy::{MODEL_INSTRUCTIONS, TidyGroup, TidyTab, TidyUndo, model_prompt, parse_answer};
use lynk_core::url::host_of;
use nd::core::WindowId;
use nd::prelude::*;

use crate::tab::TabId;
use crate::window::BrowserWindow;

/// Each window's last tidy, which its undo puts back.
#[derive(Default)]
struct Undos(HashMap<WindowId, TidyUndo>);

pub fn forget(cx: &mut App, win: WindowId) {
    if let Some(undos) = cx.try_global_mut::<Undos>() {
        undos.0.remove(&win);
    }
}

/// What the grouping is told about each loose tab: its title, host and opener, nothing of
/// the page.
fn loose_tabs(win: &BrowserWindow) -> Vec<TidyTab> {
    win.tree()
        .loose_tabs()
        .into_iter()
        .filter_map(|id| {
            let tab = win.tab(tab_number(id)?)?;
            Some(TidyTab {
                id: id.to_owned(),
                title: tab.label(),
                host: host_of(&tab.url),
                opener: tab.opener.map(tab_id),
            })
        })
        .collect()
}

pub fn can_tidy(win: &BrowserWindow, _: TabId, _: &App) -> bool {
    win.tree().loose_tabs().len() >= 2
}

pub fn tidy(win: &mut BrowserWindow, _: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    if !nd::system::text_model::available() {
        apply(win, None, cx);
        return;
    }
    let id = win.id;
    let answer = nd::system::text_model::respond(MODEL_INSTRUCTIONS, &model_prompt(&loose_tabs(win)));
    nd::system::spawn(answer, move |answer, cx| {
        let answer = answer.inspect_err(|e| eprintln!("lynk: tidy: the text model failed: {e}")).ok();
        if let Some(win) = crate::app::window(cx, id) {
            // Held to the list as it is now, which may have changed while the model thought.
            win.update(cx, |win, cx| apply(win, answer.as_deref().and_then(parse_answer), cx));
        }
    });
}

fn apply(win: &mut BrowserWindow, answer: Option<Vec<TidyGroup>>, cx: &mut Context<BrowserWindow>) {
    let tabs = loose_tabs(win);
    let Some(undo) = win.tidy(&tabs, answer, cx) else { return };
    if !cx.has_global::<Undos>() {
        cx.set_global(Undos::default());
    }
    cx.global_mut::<Undos>().0.insert(win.id, undo);
}

pub fn can_undo(win: &BrowserWindow, _: TabId, cx: &App) -> bool {
    cx.try_global::<Undos>().and_then(|u| u.0.get(&win.id)).is_some_and(|u| u.holds(win.tree()))
}

pub fn undo(win: &mut BrowserWindow, _: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(undo) = cx.try_global_mut::<Undos>().and_then(|u| u.0.remove(&win.id)) else { return };
    win.undo_tidy(undo, cx);
}
