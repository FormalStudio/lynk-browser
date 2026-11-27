//! A tab's page in the window: the web view with what it reports wired into the window's state,
//! and the pages the app draws in its place (the new tab page, "Unable to load this page",
//! "Aw, Snap!").
//!
//! A page only reports while it is mounted, so every live page stays mounted: the one on show
//! fills the content area, the rest sit in a zero-size holder. Mounting a page elsewhere moves
//! it without a reload.
//!
//! Other modules hear a tab's page through [`OBSERVERS`]: each runs, in order, on every
//! [`PageEvent`] after the window has taken it in.

use lynk_core::pages::{NativePage, native_page};
use lynk_core::tabstate::SecurityFacts;
use lynk_core::url::display_url;
use nd::prelude::*;
use nd::webview::*;

use crate::actions;
use crate::tab::{Live, LoadError, Page, Tab, TabId};
use crate::window::{BrowserWindow, Open};

/// What a page reported, for the modules that listen in.
pub enum PageEvent<'a> {
    Navigated(&'a Navigate),
    TitleChanged(&'a TitleChanged),
    LoadingChanged(&'a LoadingChanged),
    FaviconChanged(&'a FaviconChanged),
    SecurityChanged(&'a SecurityChanged),
    AudioStateChanged(&'a AudioStateChanged),
    PopupBlocked(&'a PopupBlocked),
    PermissionRequest(&'a PermissionRequest),
    PermissionRequestDismissed(&'a PermissionRequestDismissed),
    DownloadRequested(&'a DownloadRequested),
    DownloadUpdated(&'a DownloadUpdated),
    ContextMenuItemClicked(&'a ContextMenuItemClicked),
    ScriptMessage(&'a ScriptMessage),
    RequestBlocked(&'a RequestBlocked),
    ExternalProtocol(&'a ExternalProtocol),
    ExtensionPopupOpened(&'a ExtensionPopupOpened),
    ExtensionPopupResized(&'a ExtensionPopupResized),
    ExtensionPopupClosed(&'a ExtensionPopupClosed),
}

pub type Observer = fn(&mut BrowserWindow, TabId, &PageEvent, &mut Window, &mut Context<BrowserWindow>);

/// The modules listening to every tab's page, in the order they hear an event.
pub const OBSERVERS: &[Observer] = &[
    crate::history::record,
    crate::blocking::observe,
    crate::hider::observe,
    crate::reader::observe,
    crate::save_page::observe,
    crate::float::observe,
];

fn observe(win: &mut BrowserWindow, id: TabId, event: PageEvent, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    for observer in OBSERVERS {
        observer(win, id, &event, window, cx);
    }
}

fn element_id(id: TabId, live: &Live) -> SharedString {
    format!("page-{id}-{}", live.key).into()
}

/// The page of `tab`, wired to the window.
pub fn mount(tab: &Tab, live: &Live, cx: &mut Context<BrowserWindow>) -> AnyElement {
    let id = tab.id;
    let page = live.view.clone();
    webview(element_id(id, live), &live.view)
        .size_full()
        .on_navigate(cx.listener(move |w, e: &Navigate, window, cx| {
            navigated(w, id, &e.url, cx);
            observe(w, id, PageEvent::Navigated(e), window, cx);
        }))
        .on_title_changed(cx.listener(move |w, e: &TitleChanged, window, cx| {
            if let Some(tab) = w.tab_mut(id) {
                // A restored tab never shows about:blank for a title.
                if !(e.title == "about:blank" && !tab.url.is_empty()) {
                    tab.title = e.title.clone();
                }
            }
            if w.active == id {
                let label = w.active_tab().map(Tab::label).unwrap_or_default();
                cx.set_window_title(w.id, label);
            }
            w.changed(cx);
            observe(w, id, PageEvent::TitleChanged(e), window, cx);
        }))
        .on_loading_changed(cx.listener(move |w, e: &LoadingChanged, window, cx| {
            loading_changed(w, id, e.loading, cx);
            observe(w, id, PageEvent::LoadingChanged(e), window, cx);
        }))
        .on_load_progress(cx.listener(move |w, e: &LoadProgress, _, cx| {
            if let Some(tab) = w.tab_mut(id) {
                tab.rt.progress = e.progress;
            }
            cx.notify();
        }))
        .on_back_available(cx.listener(move |w, e: &BackAvailable, _, cx| {
            if let Some(tab) = w.tab_mut(id) {
                tab.rt.can_go_back = e.available;
            }
            w.changed(cx);
        }))
        .on_forward_available(cx.listener(move |w, e: &ForwardAvailable, _, cx| {
            if let Some(tab) = w.tab_mut(id) {
                tab.rt.can_go_forward = e.available;
            }
            w.changed(cx);
        }))
        .on_load_failed(cx.listener(move |w, e: &LoadFailed, _, cx| {
            if let Some(tab) = w.tab_mut(id) {
                tab.rt.error = Some(LoadError { url: e.url.clone(), error: e.error.clone() });
            }
            cx.notify();
        }))
        .on_render_process_gone(cx.listener(move |w, e: &RenderProcessGone, _, cx| {
            if let Some(tab) = w.tab_mut(id) {
                tab.rt.crashed = Some(e.reason.clone());
                tab.rt.loading = false;
            }
            w.changed(cx);
        }))
        .on_new_window(cx.listener(move |w, e: &NewWindow, _, cx| new_window(w, id, e, cx)))
        .on_window_closed(cx.listener(move |w, _: &WindowClosed, _, cx| {
            w.remove(id, cx);
        }))
        .on_favicon_changed(cx.listener(move |w, e: &FaviconChanged, window, cx| {
            if let Some(tab) = w.tab_mut(id) {
                tab.favicon = (!e.icon_url.is_empty()).then(|| e.icon_url.clone().into());
            }
            w.changed(cx);
            observe(w, id, PageEvent::FaviconChanged(e), window, cx);
        }))
        .on_find_result(cx.listener(move |w, e: &FindResult, _, cx| crate::find::result(w, id, e, cx)))
        .on_zoom_changed(cx.listener(move |w, e: &ZoomChanged, _, cx| crate::zoom::page_zoomed(w, id, e, cx)))
        .on_fullscreen_changed(
            cx.listener(move |w, e: &FullscreenChanged, _, cx| w.page_fullscreen_changed(id, e.fullscreen, cx)),
        )
        .on_close_approved(cx.listener(move |w, e: &CloseApproved, _, cx| w.close_answered(id, &page, e.approved, cx)))
        .on_chrome_command(cx.listener(move |w, e: &ChromeCommand, _, cx| chrome_command(w, e.command, cx)))
        .on_security_changed(cx.listener(move |w, e: &SecurityChanged, window, cx| {
            if let Some(tab) = w.tab_mut(id) {
                tab.rt.security =
                    SecurityFacts { secure: e.secure, insecure_content: e.insecure_content, error: e.error.clone() };
            }
            cx.notify();
            observe(w, id, PageEvent::SecurityChanged(e), window, cx);
        }))
        .on_audio_state_changed(cx.listener(move |w, e: &AudioStateChanged, window, cx| {
            observe(w, id, PageEvent::AudioStateChanged(e), window, cx)
        }))
        .on_popup_blocked(
            cx.listener(move |w, e: &PopupBlocked, window, cx| observe(w, id, PageEvent::PopupBlocked(e), window, cx)),
        )
        .on_permission_request(cx.listener(move |w, e: &PermissionRequest, window, cx| {
            observe(w, id, PageEvent::PermissionRequest(e), window, cx)
        }))
        .on_permission_request_dismissed(cx.listener(move |w, e: &PermissionRequestDismissed, window, cx| {
            observe(w, id, PageEvent::PermissionRequestDismissed(e), window, cx)
        }))
        .on_download_requested(cx.listener(move |w, e: &DownloadRequested, window, cx| {
            observe(w, id, PageEvent::DownloadRequested(e), window, cx)
        }))
        .on_download_updated(cx.listener(move |w, e: &DownloadUpdated, window, cx| {
            observe(w, id, PageEvent::DownloadUpdated(e), window, cx)
        }))
        .on_context_menu_item_clicked(cx.listener(move |w, e: &ContextMenuItemClicked, window, cx| {
            observe(w, id, PageEvent::ContextMenuItemClicked(e), window, cx)
        }))
        .on_script_message(cx.listener(move |w, e: &ScriptMessage, window, cx| {
            if e.name == crate::tab::SCROLL_MESSAGE {
                scrolled(w, id, &e.body, cx);
                return;
            }
            observe(w, id, PageEvent::ScriptMessage(e), window, cx)
        }))
        .on_request_blocked(cx.listener(move |w, e: &RequestBlocked, window, cx| {
            observe(w, id, PageEvent::RequestBlocked(e), window, cx)
        }))
        .on_external_protocol(cx.listener(move |w, e: &ExternalProtocol, window, cx| {
            observe(w, id, PageEvent::ExternalProtocol(e), window, cx)
        }))
        .on_extension_popup_opened(cx.listener(move |w, e: &ExtensionPopupOpened, window, cx| {
            observe(w, id, PageEvent::ExtensionPopupOpened(e), window, cx)
        }))
        .on_extension_popup_resized(cx.listener(move |w, e: &ExtensionPopupResized, window, cx| {
            observe(w, id, PageEvent::ExtensionPopupResized(e), window, cx)
        }))
        .on_extension_popup_closed(cx.listener(move |w, e: &ExtensionPopupClosed, window, cx| {
            observe(w, id, PageEvent::ExtensionPopupClosed(e), window, cx)
        }))
        .into_any_element()
}

fn navigated(w: &mut BrowserWindow, id: TabId, url: &str, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = w.tab_mut(id) else { return };
    if url == "about:blank" && !tab.url.is_empty() {
        return;
    }
    if let Some(native) = native_page(url) {
        // Chromium's own page for something the app draws natively: the page goes back to
        // where it was and the app's surface opens instead.
        let action: Option<Box<dyn Action>> = match native {
            NativePage::NewTab => None,
            NativePage::Downloads => Some(Box::new(actions::ShowDownloads)),
            NativePage::History => Some(Box::new(actions::ShowHistory)),
            NativePage::Bookmarks => Some(Box::new(actions::ShowBookmarks)),
            NativePage::TabSearch => Some(Box::new(actions::OpenSwitcher)),
        };
        match (tab.view(), tab.rt.can_go_back) {
            (Some(page), true) => page.go_back(),
            _ => {
                tab.url.clear();
                tab.title.clear();
                tab.reset(Page::Waiting);
            }
        }
        if let Some(action) = action {
            cx.dispatch_action(w.id, action.as_ref());
        }
        w.changed(cx);
        return;
    }
    // Chromium reports a source view at the address it shows the source of; the tab keeps
    // `view-source:`, so it reads as no web page and restores as a source view.
    if tab.url.strip_prefix("view-source:") != Some(url) {
        tab.url = url.to_owned();
    }
    tab.rt.error = None;
    tab.find.count = None;
    tab.find.found = None;
    w.apply_zoom(id, cx);
    w.changed(cx);
}

/// The page settled at a new scroll position; the session keeps it.
fn scrolled(w: &mut BrowserWindow, id: TabId, at: &JsValue, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = w.tab_mut(id) else { return };
    let at = at.as_array().and_then(|p| Some((p.first()?.as_f64()?, p.get(1)?.as_f64()?)));
    let scroll = at.filter(|s| *s != (0., 0.));
    if tab.scroll != scroll {
        tab.scroll = scroll;
        crate::session::changed(cx);
    }
}

fn loading_changed(w: &mut BrowserWindow, id: TabId, loading: bool, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = w.tab_mut(id) else { return };
    tab.rt.loading = loading;
    if loading {
        tab.rt.progress = 0.;
    } else if let Some(live) = tab.live_mut()
        && let Some((x, y)) = live.scroll_back.take()
    {
        // A page that slept gets its scroll position back once it has loaded again.
        live.view.execute_javascript(format!("scrollTo({x}, {y})"), |_, _| {});
    }
    w.changed(cx);
}

fn new_window(w: &mut BrowserWindow, opener: TabId, e: &NewWindow, cx: &mut Context<BrowserWindow>) {
    let url = e.url.clone();
    let id = crate::app::next_tab_id(cx);
    let tab = match &e.popup {
        Some(popup) => Tab::with_page(id, url, popup.clone()),
        None if url.is_empty() || url == "about:blank" => return,
        None => Tab::new(id, url),
    };
    match e.disposition {
        Disposition::ForegroundTab | Disposition::Popup => {
            w.insert(tab, Open::FromPage { opener, foreground: true }, cx);
        }
        Disposition::BackgroundTab => {
            w.insert(tab, Open::FromPage { opener, foreground: false }, cx);
        }
        Disposition::Window => {
            crate::app::open_window_in(cx, w.profile.clone(), vec![tab], Some(id));
        }
    }
}

/// A Chrome command the focused page took: it reaches the app as the app's own command.
pub(crate) fn chrome_command(w: &mut BrowserWindow, command: BrowserCommand, cx: &mut Context<BrowserWindow>) {
    use actions::*;
    let action: Box<dyn Action> = match command {
        BrowserCommand::NewTab => Box::new(NewTab),
        BrowserCommand::NewWindow => Box::new(NewWindow),
        BrowserCommand::NewPrivateWindow => Box::new(NewPrivateWindow),
        BrowserCommand::ReopenClosedTab => Box::new(ReopenClosedTab),
        BrowserCommand::CloseTab => Box::new(CloseTab),
        BrowserCommand::NextTab => Box::new(NextTab),
        BrowserCommand::PreviousTab => Box::new(PreviousTab),
        BrowserCommand::History => Box::new(ShowHistory),
        BrowserCommand::Downloads => Box::new(ShowDownloads),
        BrowserCommand::Bookmarks => Box::new(ShowBookmarks),
        BrowserCommand::BookmarkPage => Box::new(BookmarkPage),
        BrowserCommand::TabSearch => Box::new(OpenSwitcher),
        BrowserCommand::Settings => Box::new(OpenSettings),
        BrowserCommand::Extensions => Box::new(OpenExtensions),
        BrowserCommand::ClearBrowsingData => Box::new(ClearBrowsingData),
        BrowserCommand::SavePage => Box::new(SavePage),
        BrowserCommand::ViewSource => Box::new(ViewSource),
        BrowserCommand::Find => Box::new(FindInPage),
        BrowserCommand::FindNext => Box::new(FindNext),
        BrowserCommand::FindPrevious => Box::new(FindPrevious),
        BrowserCommand::FocusAddress => Box::new(OpenAddress),
        BrowserCommand::Fullscreen => Box::new(ToggleFullScreen),
        BrowserCommand::Print => Box::new(Print),
        BrowserCommand::DevTools => Box::new(DevTools),
        // The window is leased to this listener, where `request_close` would find no close
        // handler and close it unasked; the handler's own check runs here instead.
        BrowserCommand::CloseWindow => {
            if w.should_close(cx) {
                cx.close_window(w.id);
            }
            return;
        }
        BrowserCommand::Quit => {
            cx.quit();
            return;
        }
        // The task manager, Home and Open File are Chrome's own surfaces with nothing in Lynk.
        BrowserCommand::TaskManager | BrowserCommand::Home | BrowserCommand::OpenFile => return,
    };
    cx.dispatch_action(w.id, action.as_ref());
}

/// What the content area shows for the tab on show, in front of its page.
pub fn cover(tab: &Tab, cx: &mut Context<BrowserWindow>) -> Option<AnyElement> {
    let id = tab.id;
    if let Some(reason) = &tab.rt.crashed {
        let mut description = "Something went wrong while displaying this webpage.".to_owned();
        if !reason.is_empty() {
            description.push_str(&format!("\n\nError code: {reason}"));
        }
        return Some(
            status_page("Aw, Snap!")
                .id("crash-page")
                .icon(Icon::TriangleAlert)
                .description(description)
                .child(button("crash-reload", "Reload").on_click(cx.listener(move |w, _: &ClickEvent, _, cx| {
                    if let Some(tab) = w.tab_mut(id) {
                        tab.rt.crashed = None;
                        if let Some(page) = tab.view() {
                            page.reload();
                        }
                    }
                    w.changed(cx);
                })))
                .into_any_element(),
        );
    }
    if let Some(error) = &tab.rt.error {
        let url = error.url.clone();
        return Some(
            status_page("Unable to load this page")
                .id("error-page")
                .icon(Icon::Globe)
                .description(format!("{}: {}", display_url(&error.url), error.error))
                .child(button("error-retry", "Try Again").on_click(cx.listener(move |w, _: &ClickEvent, _, cx| {
                    w.navigate(id, url.clone(), cx);
                })))
                .into_any_element(),
        );
    }
    if tab.url.is_empty() {
        return Some(new_tab_page(cx));
    }
    None
}

fn new_tab_page(cx: &mut Context<BrowserWindow>) -> AnyElement {
    let gap = cx.theme().spacing.sm;
    v_flex()
        .id("new-tab-page")
        .size_full()
        .items_center()
        .justify_center()
        .gap(gap)
        .child(button("new-tab-search", "Search or enter address").icon(Icon::Search).on_click(
            |_: &ClickEvent, window, cx| {
                cx.dispatch_action(window.id(), &actions::OpenAddress);
            },
        ))
        .child(label("Search with DuckDuckGo").id("new-tab-engine").secondary())
        .into_any_element()
}
