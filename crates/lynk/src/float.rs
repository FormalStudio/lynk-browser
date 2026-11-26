//! Floating video, Alt+Cmd+P: the page's video in Chromium's own picture-in-picture window, which
//! stays above every window on screen; the same command brings it back. `requestPictureInPicture`
//! answers only a user's action, so the script runs with a user gesture. It owns the lift, not
//! the window, which is Chromium's.

use nd::prelude::*;
use nd::webview::{UserGesture, WebView};

use crate::page::PageEvent;
use crate::tab::TabId;
use crate::window::BrowserWindow;

const WORLD: &str = "lynk-float";
/// What the world posts: `"on"`, `"off"` when the floating video came back, `"none"` when there
/// was no video to lift, or `"error:<message>"` when Chromium refused.
const MESSAGE: &str = "lynkFloat";

/// The video chosen is the one playing, the largest on screen when several are, else the largest
/// with a picture. Same-origin frames are searched too; a player in another site's frame is out
/// of reach of the page's own script, which is Chromium's rule. The answer is posted, since the
/// lift is a promise the evaluation does not wait for.
const FLOAT: &str = r#"(async () => {
  const post = (m) => window.webkit?.messageHandlers?.lynkFloat?.postMessage(m);
  const docs = [document];
  for (const f of document.querySelectorAll("iframe")) {
    try { if (f.contentDocument) docs.push(f.contentDocument); } catch (_) {}
  }
  for (const d of docs) {
    if (d.pictureInPictureElement) { await d.exitPictureInPicture(); return post("off"); }
  }
  const videos = docs.flatMap((d) => [...d.querySelectorAll("video")]).filter((v) => v.readyState >= 1);
  const area = (v) => { const r = v.getBoundingClientRect(); return Math.max(0, r.width) * Math.max(0, r.height); };
  const playing = videos.filter((v) => !v.paused && !v.ended);
  const pool = playing.length ? playing : videos.filter((v) => v.videoWidth > 0);
  if (!pool.length) return post("none");
  const video = pool.sort((a, b) => area(b) - area(a))[0];
  // A site's player can opt out of the button it draws; the command is the user asking outright.
  video.disablePictureInPicture = false;
  try { await video.requestPictureInPicture(); post("on"); }
  catch (e) { post("error:" + (e && e.message ? e.message : String(e))); }
})()"#;

/// Sets up a new page to answer the command.
pub fn prepare(view: &WebView) {
    view.register_script_message(MESSAGE, Some(WORLD), ["*"]);
}

/// The Float Video command: lifts tab `id`'s video into picture-in-picture, or brings it back.
pub fn toggle(win: &mut BrowserWindow, id: TabId, _: &mut Window, _: &mut Context<BrowserWindow>) {
    if let Some(view) = win.tab(id).and_then(|t| t.view()) {
        view.execute_javascript_in_frame(FLOAT, Some(WORLD), None, UserGesture::With, |_, _| {});
    }
}

/// A page observer: says so when there was nothing to float or Chromium refused.
pub fn observe(win: &mut BrowserWindow, id: TabId, event: &PageEvent, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let PageEvent::ScriptMessage(m) = event else { return };
    if m.name != MESSAGE || win.tab(id).is_none() {
        return;
    }
    match m.body.as_str() {
        Some("none") => win.toast("No video to float on this page"),
        Some(answer) if answer.starts_with("error:") => {
            eprintln!("lynk: float video: {answer}");
            win.toast("Unable to float this video");
        }
        _ => return,
    }
    cx.notify();
}
