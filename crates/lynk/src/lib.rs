//! Lynk on NativeDesktop: Arc-style windows with the tab sidebar and Chromium pages in Chrome
//! style. `docs/nd-parity.md` is the checklist this is built against.
//!
//! The places other modules plug into are plain ordered lists: commands and menu items in
//! `actions` (with the keys bound to one part of the window in `actions::CONTEXT_KEYS`), tab
//! events in `window::TAB_EVENTS`, the sidebar's foot slots in `sidebar::FOOT`, the padlock
//! popover's rows in `site_info::ROWS`, page event observers in `page::OBSERVERS` and window
//! overlays in `shell::OVERLAYS`. Tabs and windows open through `BrowserWindow::open_tab` and
//! `app::open_window`.

pub mod actions;
pub mod app;
pub mod find;
pub mod history;
pub mod omnibox;
pub mod page;
pub mod process;
pub mod session;
pub mod shell;
pub mod sidebar;
pub mod site_info;
pub mod tab;
pub mod window;
pub mod zoom;

use nd::prelude::*;
use nd_ipc::instance::Launch;

pub fn app() -> Application {
    Application::new(nd::manifest!()).settings::<lynk_core::settings::LynkSettings>()
}

/// Runs the browser, unless another instance holds this profile, which then gets this launch.
pub fn run() -> nd::Result<()> {
    app().run(|cx| {
        let launch = Launch::from_env();
        if process::start(cx, &launch) {
            init(cx, launch.urls);
        }
    })
}

/// Brings back the session's windows, with `urls` as new tabs in the front one. With no session
/// to restore, opens one window on `urls`, one tab each; with none, on `LYNK_URL` or the new tab
/// page.
pub fn init(cx: &mut App, mut urls: Vec<String>) {
    app::install(cx);
    cx.bind_keys(actions::bindings());
    let restored = session::open(cx, &nd::system::app_dirs().data());
    if restored.is_empty() {
        if urls.is_empty() {
            urls.push(std::env::var("LYNK_URL").unwrap_or_default());
        }
        let tabs = urls.into_iter().map(|url| tab::Tab::new(app::next_tab_id(cx), url)).collect();
        app::open_window(cx, tabs, None);
    } else {
        for window in restored {
            app::open_restored(cx, window);
        }
        if !urls.is_empty() {
            process::open_launch(cx, urls);
        }
    }
    session::restored_all(cx);
    app::refresh_menus(cx);
}
