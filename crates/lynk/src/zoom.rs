//! Page zoom: the chords and menu steps, the foot's zoom glyph while a page is not at 100%, and
//! its popover, whose state this module keeps per window. A level is kept per host and applies
//! to every tab on that host. The window's tab state is changed through its methods only.

use lynk_core::url::host_of;
use lynk_core::zoom::{ZOOM_NOTICE, is_default_zoom, step_zoom, zoom_percent};
use nd::prelude::*;
use nd::webview::{ZoomChanged, ZoomSource};

use std::collections::HashMap;

use nd::core::WindowId;

use crate::app;
use crate::tab::TabId;
use crate::window::{BrowserWindow, TabEvent};

/// The popover off the zoom glyph. A chord or a menu step opens it for [`ZOOM_NOTICE`]; a click
/// on the glyph opens it until it is dismissed.
#[derive(Default)]
struct Popover {
    open: bool,
    /// Bumped by every step, so a notice timer from an earlier step leaves it alone.
    notice: u64,
    /// Opened by a click, which no timer closes.
    pinned: bool,
}

/// Each window's popover.
#[derive(Default)]
struct Popovers(HashMap<WindowId, Popover>);

fn is_open(cx: &App, win: WindowId) -> bool {
    cx.try_global::<Popovers>().and_then(|p| p.0.get(&win)).is_some_and(|p| p.open)
}

fn state(cx: &mut App, win: WindowId) -> &mut Popover {
    if !cx.has_global::<Popovers>() {
        cx.set_global(Popovers::default());
    }
    cx.global_mut::<Popovers>().0.entry(win).or_default()
}

/// The popover of a window that has one; never makes an entry.
fn existing(cx: &mut App, win: WindowId) -> Option<&mut Popover> {
    if !cx.has_global::<Popovers>() {
        return None;
    }
    cx.global_mut::<Popovers>().0.get_mut(&win)
}

fn close(cx: &mut App, win: WindowId) {
    if let Some(p) = existing(cx, win) {
        p.open = false;
        p.pinned = false;
    }
}

/// A window listener: the window's popover goes with it.
pub fn forget(cx: &mut App, win: WindowId) {
    if cx.has_global::<Popovers>() {
        cx.global_mut::<Popovers>().0.remove(&win);
    }
}

/// A tab listener: the popover closes when another tab is shown.
pub fn tab_event(win: &mut BrowserWindow, _: TabId, event: TabEvent, cx: &mut Context<BrowserWindow>) {
    if event == TabEvent::Shown && is_open(cx, win.id) {
        close(cx, win.id);
    }
}

/// Steps tab `id`'s zoom, as a chord or the View menu does.
fn step(win: &mut BrowserWindow, id: TabId, factor: impl FnOnce(f64) -> f64, cx: &mut Context<BrowserWindow>) {
    let Some(tab) = win.tab(id) else { return };
    let (url, current) = (tab.url.clone(), tab.rt.zoom);
    set(win, id, &url, factor(current), cx);
    let me = win.id;
    let p = state(cx, me);
    p.open = true;
    p.notice += 1;
    let notice = p.notice;
    let this = cx.weak_entity();
    nd::system::after(ZOOM_NOTICE, move |cx| {
        let Some(p) = existing(cx, me) else { return };
        if p.notice == notice && !p.pinned {
            p.open = false;
            let _ = this.update(cx, |_, cx| cx.notify());
        }
    });
    cx.notify();
}

/// Keeps `factor` for `url`'s host and applies it to every tab on that host, in every window.
fn set(win: &mut BrowserWindow, id: TabId, url: &str, factor: f64, cx: &mut Context<BrowserWindow>) {
    if host_of(url).is_empty() {
        win.set_zoom(id, factor);
        return;
    }
    app::remember_zoom(cx, &win.profile, url, factor);
    win.apply_zoom_everywhere(cx);
    let me = win.id;
    for (other, entity) in app::windows(cx) {
        if other != me {
            entity.update(cx, |w, cx| w.apply_zoom_everywhere(cx));
        }
    }
}

pub fn zoom_in(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    step(win, id, |z| step_zoom(z, true), cx);
}

pub fn zoom_out(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    step(win, id, |z| step_zoom(z, false), cx);
}

pub fn reset(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    step(win, id, |_| 1., cx);
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
    let profile = win.profile.clone();
    app::remember_zoom(cx, &profile, &url, e.factor);
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
                let p = state(cx, win.id);
                p.open = !p.open;
                p.pinned = p.open;
                cx.notify();
            }),
        ));
    if is_open(cx, win.id) {
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
            close(cx, win.id);
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
                        .on_click(cx.listener(|win, _: &ClickEvent, window, cx| zoom_out(win, win.active, window, cx))),
                )
                .child(div().min_w(px(48.)).flex().justify_center().child(label(percent.to_owned()).id("zoom-value")))
                .child(
                    icon_button("zoom-in", Icon::Plus)
                        .tooltip("Zoom In")
                        .on_click(cx.listener(|win, _: &ClickEvent, window, cx| zoom_in(win, win.active, window, cx))),
                )
                .child(
                    button("zoom-reset", "Reset")
                        .on_click(cx.listener(|win, _: &ClickEvent, window, cx| reset(win, win.active, window, cx))),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use nd::test::TestApp;

    use super::*;
    use crate::tab::Tab;

    #[test]
    fn a_closed_window_keeps_no_zoom_popover() {
        let mut cx = TestApp::new();
        app::install(&mut cx);
        let tabs = vec![Tab::new(app::next_tab_id(&mut cx), "")];
        let a = app::open_window(&mut cx, tabs, None);
        let _b = app::open_window(&mut cx, Vec::new(), None);
        cx.run_until_idle();
        app::window(&cx, a).unwrap().update(&mut cx, |w, cx| step(w, 1, |z| step_zoom(z, true), cx));
        assert!(is_open(&cx, a));
        assert!(cx.request_close(a));
        assert!(cx.global::<Popovers>().0.is_empty());
        // The notice timer of that step finds nothing to close and makes no entry.
        close(&mut cx, a);
        assert!(cx.global::<Popovers>().0.is_empty());
    }
}
