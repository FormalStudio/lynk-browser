//! The command bar: nd's command palette over the window, opened by Cmd+T, Cmd+L, Cmd+K and a
//! click on the address. Core ranks the rows (`lynk_core::omnibox`) and turns typed text into an
//! address (`lynk_core::url`); this module keeps one bar's state per window, feeds core and runs
//! the row picked. `docs/omnibox.md` is the spec. It must not rank, parse or fix up text itself.

use std::collections::HashMap;

use lynk_core::omnibox::{OmniInput, OmniMode, OmniRow, OmniTab, OmniTarget, OmniVisit, omni_rows};
use lynk_core::settings::LynkSettings;
use lynk_core::url::{field_address, to_url};
use nd::AppSettings;
use nd::core::WindowId;
use nd::platform::{Target, target};
use nd::prelude::*;

use crate::actions::{self, Command};
use crate::tab::TabId;
use crate::window::{BrowserWindow, Open, TabEvent};

/// One window's open bar. The window has none while it is closed.
#[derive(Clone, Debug)]
struct Bar {
    mode: OmniMode,
    target: OmniTarget,
    /// What the field holds as typed, without a completion it shows. A present starts it from
    /// the shown address for Cmd+L, from nothing otherwise.
    query: String,
    /// History matching the query, as the last answer that is still current brought it.
    history: Vec<OmniVisit>,
    /// Which history search is current: an answer to an older one is dropped.
    asked: u64,
    /// Whether the last edit added text. A deletion keeps the completion away until the next
    /// letter, as the field does, so the rows stop offering it too.
    completes: bool,
}

#[derive(Default)]
struct Bars(HashMap<WindowId, Bar>);

/// Each window's tabs, most recently shown first, for the order core lists them in.
#[derive(Default)]
struct Shown(HashMap<WindowId, Vec<TabId>>);

fn shown(cx: &mut App) -> &mut HashMap<WindowId, Vec<TabId>> {
    if !cx.has_global::<Shown>() {
        cx.set_global(Shown::default());
    }
    &mut cx.global_mut::<Shown>().0
}

/// Keeps the shown order: a tab shown goes first, one that leaves the window goes.
pub fn tab_event(win: &mut BrowserWindow, id: TabId, event: TabEvent, cx: &mut Context<BrowserWindow>) {
    reorder(shown(cx).entry(win.id).or_default(), id, event);
}

fn reorder(order: &mut Vec<TabId>, id: TabId, event: TabEvent) {
    match event {
        TabEvent::Shown => {
            order.retain(|t| *t != id);
            order.insert(0, id);
        }
        TabEvent::Closed | TabEvent::Moved => order.retain(|t| *t != id),
        TabEvent::Opened => {}
    }
}

pub fn forget(cx: &mut App, win: WindowId) {
    shown(cx).remove(&win);
    bars(cx).remove(&win);
}

fn bar(cx: &App, win: WindowId) -> Option<&Bar> {
    cx.try_global::<Bars>()?.0.get(&win)
}

fn bars(cx: &mut App) -> &mut HashMap<WindowId, Bar> {
    if !cx.has_global::<Bars>() {
        cx.set_global(Bars::default());
    }
    &mut cx.global_mut::<Bars>().0
}

/// Opens the bar, or reseeds the one already open: never a second.
fn present(win: &BrowserWindow, mode: OmniMode, query: String, target: OmniTarget, cx: &mut Context<BrowserWindow>) {
    bars(cx).insert(win.id, Bar { mode, target, query, history: Vec::new(), asked: 0, completes: true });
    ask_history(win, cx);
    cx.notify();
}

/// Searches history for the bar's query; the answer lands only if no later search was asked.
fn ask_history(w: &BrowserWindow, cx: &mut Context<BrowserWindow>) {
    let (win, profile) = (w.id, w.profile.clone());
    let Some(bar) = bars(cx).get_mut(&win) else { return };
    if bar.mode != OmniMode::Address {
        return;
    }
    bar.asked += 1;
    let (asked, query) = (bar.asked, bar.query.trim().to_owned());
    if query.is_empty() {
        bar.history.clear();
        return;
    }
    let this = cx.weak_entity();
    crate::history::search(cx, &profile, query, move |visits, cx| {
        let Some(bar) = bars(cx).get_mut(&win).filter(|b| b.asked == asked) else { return };
        bar.history = visits.into_iter().map(|v| OmniVisit { url: v.url, title: v.title }).collect();
        let _ = this.update(cx, |_, cx| cx.notify());
    });
}

/// Cmd+T: an address for a new tab. No tab exists until Return, so a bar put away leaves none.
pub fn new_tab(win: &mut BrowserWindow, _: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    present(win, OmniMode::Address, String::new(), OmniTarget::NewTab, cx);
}

/// Cmd+L, and a click on the address: the shown address, all of it selected. A second Cmd+L
/// puts the bar away.
pub fn open_address(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    if close(win, cx).is_some() {
        return;
    }
    let url = win.tab(id).map(|t| t.url.clone()).unwrap_or_default();
    present(win, OmniMode::Address, url, OmniTarget::Current, cx);
}

/// Cmd+K: an open tab, or a command.
pub fn switcher(win: &mut BrowserWindow, _: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    present(win, OmniMode::Switcher, String::new(), OmniTarget::Current, cx);
}

fn close(win: &BrowserWindow, cx: &mut Context<BrowserWindow>) -> Option<Bar> {
    let bar = bars(cx).remove(&win.id);
    cx.notify();
    bar
}

/// The bar, while it is open.
pub fn overlay(win: &BrowserWindow, _: &mut Window, cx: &mut Context<BrowserWindow>) -> Option<AnyElement> {
    let bar = bar(cx, win.id)?.clone();
    let placeholder = match bar.mode {
        OmniMode::Address => "Search or enter address",
        OmniMode::Switcher => "Switch to a tab or run a command",
    };
    let items: Vec<PaletteItem> = rows(win, &bar, cx)
        .into_iter()
        .map(|row| item(OmniRow { completion: row.completion.filter(|_| bar.completes), ..row }))
        .collect();
    Some(
        command_palette("omnibox")
            .open(true)
            .placeholder(placeholder)
            .query(bar.query)
            .items(items)
            .on_query_change(cx.listener(|win, e: &PaletteQueryChanged, _, cx| {
                if let Some(bar) = bars(cx).get_mut(&win.id) {
                    bar.completes = e.text.len() > bar.query.len();
                    bar.query = e.text.to_string();
                }
                ask_history(win, cx);
                cx.notify();
            }))
            .on_activate(cx.listener(|win, e: &PaletteActivated, window, cx| activate(win, &e.id, window, cx)))
            .on_submit(cx.listener(|win, e: &PaletteSubmitted, _, cx| {
                if let Some(bar) = close(win, cx) {
                    submit(win, &bar, &e.text, cx);
                }
            }))
            .on_cancel(cx.listener(|win, _: &PaletteCancelled, _, cx| {
                close(win, cx);
            }))
            .into_any_element(),
    )
}

/// What core lists for the bar: every row whose command the shell runs.
fn rows(win: &BrowserWindow, bar: &Bar, cx: &App) -> Vec<OmniRow> {
    let order = cx.try_global::<Shown>().and_then(|s| s.0.get(&win.id));
    let rank = |id: TabId| order.and_then(|o| o.iter().position(|t| *t == id)).unwrap_or(usize::MAX);
    let mut others: Vec<_> = win.tabs().into_iter().filter(|t| t.id != win.active).collect();
    // Stable: tabs never shown keep the window's order after the ones that were.
    others.sort_by_key(|t| rank(t.id));
    let tabs: Vec<OmniTab> = others
        .into_iter()
        .map(|t| OmniTab { id: t.id.to_string(), title: t.title.clone(), url: t.url.clone() })
        .collect();
    let settings = cx.settings::<LynkSettings>();
    let engine = settings.engine();
    let pinned = win.is_pinned(win.active);
    let input = OmniInput {
        mode: bar.mode,
        query: &bar.query,
        target: bar.target,
        tabs: &tabs,
        history: &bar.history,
        engine_name: engine.name,
        search_prefix: engine.prefix,
        chromium: true,
        pinned,
        can_sleep: win.can_sleep(win.active),
        reading: false,
        blocking: None,
        favicon: &|_| None,
        profiles: &crate::profile::others(cx, &win.profile),
        default_profile: win.profile == lynk_core::profiles::DEFAULT_ID,
        mac: target() == Target::MacOs,
    };
    omni_rows(&input)
        .into_iter()
        .filter(|row| match row.id.strip_prefix("cmd:") {
            Some(id) => command(id).is_some_and(|c| c.runs(win, win.active, cx)),
            None => true,
        })
        .collect()
}

fn item(row: OmniRow) -> PaletteItem {
    palette_item(row.id, row.title)
        .icon(icon_of(row.icon_name.unwrap_or_default()))
        .when_some(row.subtitle, |item, s| item.subtitle(s))
        .when_some(row.hint, |item, h| item.hint(h))
        .when_some(row.completion, |item, c| item.completion(c))
}

/// Core names its row symbols in the GTK icon theme's words.
fn icon_of(name: &str) -> Icon {
    match name.trim_end_matches("-symbolic") {
        "system-search" => Icon::Search,
        "document-open-recent" => Icon::Clock,
        "go-previous" => Icon::ChevronLeft,
        "go-next" => Icon::ChevronRight,
        "view-refresh" => Icon::RotateCw,
        "window-close" => Icon::X,
        "edit-copy" => Icon::Copy,
        "edit-find" => Icon::Search,
        "tab-new" | "window-new" => Icon::Plus,
        "view-pin" => Icon::Pin,
        "weather-clear-night" => Icon::Moon,
        "folder-download" => Icon::Download,
        "preferences-system" => Icon::Settings,
        "text-x-generic" | "document-save" => Icon::FileText,
        "utilities-terminal" => Icon::Code,
        "view-conceal" => Icon::EyeOff,
        "view-reveal" => Icon::Eye,
        "edit-delete" => Icon::Trash,
        "channel-secure" | "security-low" | "security-high" => Icon::Lock,
        _ => Icon::Globe,
    }
}

/// The shell's command for a core command id.
fn command(id: &str) -> Option<&'static Command> {
    use actions::*;
    let name = match id {
        "new-tab" => NewTab.name(),
        "new-window" => NewWindow.name(),
        "private" => NewPrivateWindow.name(),
        "close-tab" => CloseTab.name(),
        "close-other-tabs" => CloseOtherTabs.name(),
        "reopen-tab" => ReopenClosedTab.name(),
        "next-tab" => NextTab.name(),
        "prev-tab" => PreviousTab.name(),
        "pin-tab" => TogglePin.name(),
        "new-profile" => NewProfile.name(),
        "rename-profile" => RenameProfile.name(),
        "delete-profile" => DeleteProfile.name(),
        "reset-pinned" => ResetPinnedPage.name(),
        "sleep-tab" => SleepTab.name(),
        "duplicate-tab" => DuplicateTab.name(),
        "move-new-window" => MoveTabToNewWindow.name(),
        "back" => Back.name(),
        "forward" => Forward.name(),
        "reload" => Reload.name(),
        "save-page" => SavePage.name(),
        "view-source" => ViewSource.name(),
        "devtools" => DevTools.name(),
        "copy-address" => CopyAddress.name(),
        "find" => FindInPage.name(),
        "zoom-in" => ZoomIn.name(),
        "zoom-out" => ZoomOut.name(),
        "zoom-reset" => ResetZoom.name(),
        "reader" => ReadingMode.name(),
        "float" => FloatVideo.name(),
        "blocking" => ToggleAdBlocking.name(),
        "hide-element" => HideElement.name(),
        "downloads-all" => ShowDownloads.name(),
        "history" => ShowHistory.name(),
        "bookmarks" => ShowBookmarks.name(),
        "bookmark-page" => BookmarkPage.name(),
        "clear-data" => ClearBrowsingData.name(),
        "extensions" => OpenExtensions.name(),
        "webstore" => OpenWebStore.name(),
        "layout" => SwitchLayout.name(),
        "settings" => OpenSettings.name(),
        _ => return None,
    };
    actions::COMMANDS.iter().find(|c| c.action().name() == name)
}

/// Return on a row, or a click on it.
fn activate(win: &mut BrowserWindow, id: &str, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(bar) = close(win, cx) else { return };
    if id == "url" {
        let typed = bar.query.clone();
        return submit(win, &bar, &typed, cx);
    }
    if let Some(url) = id.strip_prefix("go:").or_else(|| id.strip_prefix("hist:")) {
        return go(win, &bar, url.to_owned(), cx);
    }
    if let Some(tab) = id.strip_prefix("tab:").and_then(|t| t.parse().ok()) {
        return win.select(tab, cx);
    }
    if let Some(profile) = id.strip_prefix("profile:") {
        return crate::profile::switch(win, &actions::SwitchProfile(profile.to_owned()), window, cx);
    }
    if let Some(handler) = id.strip_prefix("cmd:").and_then(command).and_then(|c| c.handler) {
        handler(win, win.active, window, cx);
    }
}

/// Return on the typed text: searched, or loaded when it is an address. An empty field does
/// nothing.
fn submit(win: &mut BrowserWindow, bar: &Bar, typed: &str, cx: &mut Context<BrowserWindow>) {
    let current = win.active_tab().map(|t| t.url.clone()).unwrap_or_default();
    let prefix = cx.settings::<LynkSettings>().engine().prefix;
    if let Some(url) = to_url(&field_address(typed, &current), prefix) {
        go(win, bar, url, cx);
    }
}

/// Loads `url` where the bar sends it: a new tab after Cmd+T, unless the tab on show is the
/// new tab page, which has nothing to keep.
fn go(win: &mut BrowserWindow, bar: &Bar, url: String, cx: &mut Context<BrowserWindow>) {
    let blank = win.active_tab().is_none_or(|t| t.url.is_empty());
    if bar.target == OmniTarget::NewTab && !blank {
        win.open_tab(url, Open::End, cx);
    } else {
        win.navigate(win.active, url, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::reorder;
    use crate::window::TabEvent;

    #[test]
    fn tabs_line_up_most_recently_shown_first() {
        let (a, b, c) = (1, 2, 3);
        let mut order = Vec::new();
        for (id, event) in [(a, TabEvent::Shown), (b, TabEvent::Shown), (c, TabEvent::Opened), (c, TabEvent::Shown)] {
            reorder(&mut order, id, event);
        }
        assert_eq!(order, [c, b, a], "an opened tab waits until it is shown");
        reorder(&mut order, a, TabEvent::Shown);
        assert_eq!(order, [a, c, b]);
        reorder(&mut order, c, TabEvent::Closed);
        reorder(&mut order, b, TabEvent::Moved);
        assert_eq!(order, [a], "a tab that leaves the window leaves the order");
    }
}
