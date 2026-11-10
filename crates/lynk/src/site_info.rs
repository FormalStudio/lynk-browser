//! The padlock in the sidebar's foot and its site information popover: the host and what the
//! connection is, then the rows other modules add.

use lynk_core::tabstate::{Security, security_of};
use lynk_core::url::host_of;
use nd::prelude::*;

use crate::tab::Tab;
use crate::window::BrowserWindow;

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
                win.site_info_open = !win.site_info_open;
                cx.notify();
            },
        )),
    );
    if win.site_info_open {
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
            win.site_info_open = false;
            cx.notify();
        }))
        .child(column)
        .into_any_element()
}
