//! Reading mode, Shift+Cmd+R: Readability (Firefox's reader view, vendored in `assets/`) picks
//! the article from a copy of the page, and the reader lays it over the page in a closed shadow
//! root, in the old app's type and colours, following light and dark. The page underneath is
//! never touched, so leaving puts nothing back: the address, the scroll and anything playing are
//! where they were. Esc, the command again or a navigation leaves. It owns which tabs read, not
//! the page's content.

use std::collections::HashSet;
use std::sync::LazyLock;

use nd::prelude::*;
use nd::webview::{JsValue, UserGesture, WebView};

use crate::page::PageEvent;
use crate::tab::TabId;
use crate::window::BrowserWindow;

const WORLD: &str = "lynk-reader";
/// What the world posts when Esc took the reader away.
const MESSAGE: &str = "lynkReader";

/// Returns `"on"`, `"off"` when the reader was up and went, or `"none"` when the page has no
/// article. Readability's `module` is shadowed so it does not try to export itself.
static TOGGLE: LazyLock<String> = LazyLock::new(|| {
    include_str!("../assets/reader.js")
        .replace("/*STYLE*/", &JsValue::String(include_str!("../assets/reader.css").into()).to_string())
        .replace("/*READABILITY*/", include_str!("../assets/readability/Readability.js"))
});
const LEAVE: &str = "window.lynkReader?.leave()";

/// The tabs whose page shows the reader.
#[derive(Default)]
struct Reading(HashSet<TabId>);

/// Sets up a new page to say when Esc left the reader.
pub fn prepare(view: &WebView) {
    view.register_script_message(MESSAGE, Some(WORLD), ["*"]);
}

/// Whether tab `id` shows the reader, for the command's label.
pub fn reading(_: &BrowserWindow, id: TabId, cx: &App) -> bool {
    cx.try_global::<Reading>().is_some_and(|r| r.0.contains(&id))
}

/// The Reading Mode command: the reader over tab `id`'s page, or the page again.
pub fn toggle(win: &mut BrowserWindow, id: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let Some(view) = win.tab(id).and_then(|t| t.view()) else { return };
    // Space and Page Down go to the reader, wherever the keyboard was.
    view.focus();
    let entity = cx.entity().downgrade();
    view.execute_javascript_in_frame(TOGGLE.as_str(), Some(WORLD), None, UserGesture::Without, move |answer, cx| {
        let answer = answer.ok();
        let state = answer.as_ref().and_then(|v| v.as_str()).unwrap_or("off");
        if !cx.has_global::<Reading>() {
            cx.set_global(Reading::default());
        }
        let reading = &mut cx.global_mut::<Reading>().0;
        if state == "on" {
            reading.insert(id);
        } else {
            reading.remove(&id);
        }
        let _ = entity.update(cx, |w, cx| {
            if state == "none" {
                w.toast("No article to read on this page");
            }
            cx.notify();
        });
    });
}

/// A page observer: a navigation, same-document ones included, leaves the reader, which would
/// otherwise outlive the article it was made from; Esc in the reader says it left.
pub fn observe(win: &mut BrowserWindow, id: TabId, event: &PageEvent, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let left = match event {
        PageEvent::Navigated(_) => {
            if let Some(view) = win.tab(id).and_then(|t| t.view()).filter(|_| reading(win, id, cx)) {
                view.execute_javascript_in_frame(LEAVE, Some(WORLD), None, UserGesture::Without, |_, _| {});
            }
            true
        }
        PageEvent::ScriptMessage(m) => m.name == MESSAGE,
        _ => false,
    };
    if left && cx.try_global_mut::<Reading>().is_some_and(|r| r.0.remove(&id)) {
        cx.notify();
    }
}

/// A tab listener: a closed tab reads no more.
pub fn tab_event(_: &mut BrowserWindow, id: TabId, event: crate::window::TabEvent, cx: &mut Context<BrowserWindow>) {
    if event == crate::window::TabEvent::Closed
        && let Some(r) = cx.try_global_mut::<Reading>()
    {
        r.0.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toggle_carries_readability_and_the_style_as_one_expression() {
        assert!(TOGGLE.starts_with("(() => {") && TOGGLE.trim_end().ends_with("})()"));
        assert!(TOGGLE.contains("function Readability("));
        assert!(TOGGLE.contains("prefers-color-scheme: dark"));
        assert!(!TOGGLE.contains("/*STYLE*/") && !TOGGLE.contains("/*READABILITY*/"));
    }
}
