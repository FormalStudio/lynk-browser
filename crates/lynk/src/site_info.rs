//! The padlock in the sidebar's foot and its site information popover: the host and what the
//! connection is, then the rows other modules add. It keeps which windows have the popover up and
//! changes nothing of the window's tabs.

use lynk_core::tabstate::{Security, security_of};
use lynk_core::url::host_of;
use nd::prelude::*;

use std::collections::HashSet;

use nd::core::WindowId;

use crate::tab::{Tab, TabId};
use crate::window::{BrowserWindow, TabEvent};

/// The windows with the popover up.
#[derive(Default)]
struct Open(HashSet<WindowId>);

fn is_open(cx: &App, win: WindowId) -> bool {
    cx.try_global::<Open>().is_some_and(|o| o.0.contains(&win))
}

fn set_open(cx: &mut App, win: WindowId, open: bool) {
    if !cx.has_global::<Open>() {
        cx.set_global(Open::default());
    }
    let windows = &mut cx.global_mut::<Open>().0;
    if open {
        windows.insert(win);
    } else {
        windows.remove(&win);
    }
}

/// A window listener: the window's popover goes with it.
pub fn forget(cx: &mut App, win: WindowId) {
    set_open(cx, win, false);
}

/// A tab listener: the popover closes when another tab is shown.
pub fn tab_event(win: &mut BrowserWindow, _: TabId, event: TabEvent, cx: &mut Context<BrowserWindow>) {
    if event == TabEvent::Shown && is_open(cx, win.id) {
        set_open(cx, win.id, false);
    }
}

/// A row of the popover for the tab on show, or nothing when it has none to show.
pub type Row = fn(&BrowserWindow, &Tab, &mut Window, &mut Context<BrowserWindow>) -> Option<AnyElement>;

/// The popover's rows under the security sentence, in order: the ad blocking switch with its
/// blocked count, then the site's permissions.
pub const ROWS: &[Row] = &[];

fn icon_of(security: Security) -> Icon {
    match security {
        Security::None => Icon::Info,
        Security::Secure => Icon::Lock,
        Security::Mixed | Security::Invalid | Security::Insecure => Icon::TriangleAlert,
    }
}

pub fn security(tab: &Tab) -> Security {
    security_of(&tab.url, &tab.rt.security)
}

/// The padlock, opening the popover.
pub fn padlock(win: &BrowserWindow, window: &mut Window, cx: &mut Context<BrowserWindow>) -> Option<AnyElement> {
    let tab = win.active_tab()?;
    let security = security(tab);
    let mut anchor = div().id("site-info-anchor").child(
        icon_button("site-info", icon_of(security)).tooltip(security.tooltip()).on_click(cx.listener(
            |win, _: &ClickEvent, _, cx| {
                let open = !is_open(cx, win.id);
                set_open(cx, win.id, open);
                cx.notify();
            },
        )),
    );
    if is_open(cx, win.id) {
        anchor = anchor.child(body(win, tab, security, window, cx));
    }
    Some(anchor.into_any_element())
}

fn body(
    win: &BrowserWindow,
    tab: &Tab,
    security: Security,
    window: &mut Window,
    cx: &mut Context<BrowserWindow>,
) -> AnyElement {
    let host = host_of(&tab.url);
    let t = cx.theme();
    let (gap, pad) = (t.spacing.sm, t.spacing.md);
    let mut column = v_flex().gap(gap).p(pad).w(px(280.));
    if !host.is_empty() {
        column = column.child(label(host).id("site-info-host").text_style(TextStyle::Headline));
    }
    column = column.child(label(security.tooltip()).id("site-info-security").secondary().wraps());
    for row in ROWS {
        if let Some(row) = row(win, tab, window, cx) {
            column = column.child(row);
        }
    }
    popover("site-info-popover")
        .placement(Placement::Trailing)
        .on_dismiss(cx.listener(|win, _: &Dismissed, _, cx| {
            set_open(cx, win.id, false);
            cx.notify();
        }))
        .child(column)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use nd::test::TestApp;

    use super::*;
    use crate::app;

    #[test]
    fn showing_another_tab_closes_the_popover() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let tabs = vec![Tab::new(app::next_tab_id(&mut cx), ""), Tab::new(app::next_tab_id(&mut cx), "")];
        let w = app::open_window(&mut cx, tabs, None);
        cx.run_until_idle();
        set_open(&mut cx, w, true);
        app::window(&cx, w).unwrap().update(&mut cx, |win, cx| win.select(2, cx));
        cx.run_until_idle();
        assert!(!is_open(&cx, w));
    }

    #[test]
    fn a_closed_window_keeps_no_popover() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let a = app::open_window(&mut cx, Vec::new(), None);
        let _b = app::open_window(&mut cx, Vec::new(), None);
        cx.run_until_idle();
        set_open(&mut cx, a, true);
        assert!(cx.request_close(a));
        assert!(cx.global::<Open>().0.is_empty());
    }
}
