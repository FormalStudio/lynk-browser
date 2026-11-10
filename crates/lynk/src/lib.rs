//! Lynk on NativeDesktop: Arc-style windows with the tab sidebar and Chromium pages in Chrome
//! style. `docs/nd-parity.md` is the checklist this is built against.
//!
//! The places other modules plug into are plain ordered lists: commands and menu items in
//! `actions`, the sidebar's foot slots in `sidebar::FOOT`, the padlock popover's rows in
//! `site_info::ROWS`, page event observers in `page::OBSERVERS` and window overlays in
//! `shell::OVERLAYS`. Tabs and windows open through `BrowserWindow::open_tab` and
//! `app::open_window`.

pub mod actions;
pub mod app;
pub mod dialogs;
pub mod find;
pub mod page;
pub mod process;
pub mod shell;
pub mod sidebar;
pub mod site_info;
pub mod tab;
pub mod window;
pub mod zoom;

use nd::prelude::*;

pub fn app() -> Application {
    Application::new(nd::manifest!())
}

/// Runs the browser, unless another instance holds this profile, which then gets this launch.
pub fn run() -> nd::Result<()> {
    app().run(|cx| {
        if process::start(cx) {
            init(cx);
        }
    })
}

pub fn init(cx: &mut App) {
    app::install(cx);
    cx.bind_keys(actions::bindings());
    let url = std::env::var("LYNK_URL").unwrap_or_default();
    let tab = tab::Tab::new(app::next_tab_id(cx), url);
    app::open_window(cx, vec![tab], None);
    app::refresh_menus(cx);
}
