//! Page zoom: the chords and menu steps, the foot's zoom glyph while a page is not at 100%, and
//! its popover. A level is kept per host and applies to every tab on that host.

use lynk_core::url::host_of;
use lynk_core::zoom::{ZOOM_NOTICE, is_default_zoom, step_zoom, zoom_percent};
use nd::prelude::*;
use nd::webview::{ZoomChanged, ZoomSource};

use crate::app;
use crate::tab::TabId;
use crate::window::BrowserWindow;

/// The popover off the zoom glyph. A chord or a menu step opens it for [`ZOOM_NOTICE`]; a click
/// on the glyph opens it until it is dismissed.
#[derive(Default)]
pub struct ZoomPopover {
    pub open: bool,
    /// Bumped by every step, so a notice timer from an earlier step leaves it alone.
    notice: u64,
    /// Opened by a click, which no timer closes.
    pinned: bool,
}

impl ZoomPopover {
    pub fn close(&mut self) {
        self.open = false;
        self.pinned = false;
    }
}

/// Steps the shown tab's zoom, as a chord or the View menu does.
fn step(win: &mut BrowserWindow, factor: impl FnOnce(f64) -> f64, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.active_tab() else { return };
    let (id, url, current) = (tab.id, tab.url.clone(), tab.rt.zoom);
    set(win, id, &url, factor(current), cx);
    win.zoom.open = true;
    win.zoom.notice += 1;
    let notice = win.zoom.notice;
    let this = cx.weak_entity();
    nd::system::after(ZOOM_NOTICE, move |cx| {
        let _ = this.update(cx, |win, cx| {
            if win.zoom.notice == notice && !win.zoom.pinned {
                win.zoom.open = false;
                cx.notify();
            }
        });
    });
    cx.notify();
}

/// Keeps `factor` for `url`'s host and applies it to every tab on that host, in every window.
fn set(win: &mut BrowserWindow, id: TabId, url: &str, factor: f64, cx: &mut Context<BrowserWindow>) {
    if host_of(url).is_empty() {
        if let Some(tab) = win.tab_mut(id) {
            tab.rt.zoom = factor;
            if let Some(page) = &tab.page {
                page.set_zoom(factor);
            }
        }
        return;
    }
    app::remember_zoom(cx, url, factor);
    win.apply_zoom_everywhere(cx);
    let me = win.id;
    for (other, entity) in app::windows(cx) {
        if other != me {
            entity.update(cx, |w, cx| w.apply_zoom_everywhere(cx));
        }
    }
}

pub fn zoom_in(win: &mut BrowserWindow, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    step(win, |z| step_zoom(z, true), cx);
}

pub fn zoom_out(win: &mut BrowserWindow, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    step(win, |z| step_zoom(z, false), cx);
}

pub fn reset(win: &mut BrowserWindow, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    step(win, |_| 1., cx);
}

/// The page zoomed itself (ctrl-wheel, its own chord): the level is kept for the host as a
/// step would keep it. The echo of a level the app set is left alone, since one from an earlier
/// step can arrive after a later step and would put the earlier level back.
pub fn page_zoomed(win: &mut BrowserWindow, id: TabId, e: &ZoomChanged, cx: &mut Context<BrowserWindow>) {
    if e.source != ZoomSource::Page {
        return;
    }
    let Some(tab) = win.tab_mut(id) else { return };
    tab.rt.zoom = e.factor;
    let url = tab.url.clone();
    app::remember_zoom(cx, &url, e.factor);
    cx.notify();
}

/// The foot's zoom glyph, while the shown page is not at 100%.
pub fn glyph(win: &BrowserWindow, _: &mut Window, cx: &mut Context<BrowserWindow>) -> Option<AnyElement> {
    let factor = win.active_tab()?.rt.zoom;
    if is_default_zoom(factor) {
        return None;
    }
    let percent = zoom_percent(factor);
    let mut anchor =
        div().id("zoom-anchor").child(icon_button("zoom", Icon::Search).tooltip(format!("Zoom: {percent}")).on_click(
            cx.listener(|win, _: &ClickEvent, _, cx| {
                let open = !win.zoom.open;
                win.zoom.open = open;
                win.zoom.pinned = open;
                cx.notify();
            }),
        ));
    if win.zoom.open {
        anchor = anchor.child(popover_body(&percent, cx));
    }
    Some(anchor.into_any_element())
}

fn popover_body(percent: &str, cx: &mut Context<BrowserWindow>) -> AnyElement {
    let t = cx.theme();
    let (gap, pad) = (t.spacing.sm, t.spacing.md);
    // Beside the glyph, over the page: the foot sits at the window's leading edge, where a
    // popover above it would hang out of the window.
    popover("zoom-popover")
        .placement(Placement::Trailing)
        .on_dismiss(cx.listener(|win, _: &Dismissed, _, cx| {
            win.zoom.close();
            cx.notify();
        }))
        .child(
            h_flex()
                .gap(gap)
                .p(pad)
                .items_center()
                .child(
                    icon_button("zoom-out", Icon::Minus)
                        .tooltip("Zoom Out")
                        .on_click(cx.listener(|win, _: &ClickEvent, window, cx| zoom_out(win, window, cx))),
                )
                .child(div().min_w(px(48.)).flex().justify_center().child(label(percent.to_owned()).id("zoom-value")))
                .child(
                    icon_button("zoom-in", Icon::Plus)
                        .tooltip("Zoom In")
                        .on_click(cx.listener(|win, _: &ClickEvent, window, cx| zoom_in(win, window, cx))),
                )
                .child(
                    button("zoom-reset", "Reset")
                        .on_click(cx.listener(|win, _: &ClickEvent, window, cx| reset(win, window, cx))),
                ),
        )
        .into_any_element()
}
