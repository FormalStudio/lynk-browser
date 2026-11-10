//! One tab: what it shows, the page that shows it, and what that page last reported. A tab has
//! no page until it is first shown (a restored or background tab waits), none while it sleeps,
//! and none on the new tab page.

use lynk_core::tabstate::{FindState, SecurityFacts, create_at_url, tab_hover, tab_label};
use nd::prelude::*;
use nd::webview::{WebView, WebViewOptions};

/// Unique across every window, so a tab keeps its id when it moves.
pub type TabId = u64;

#[derive(Clone, Debug, PartialEq)]
pub struct LoadError {
    pub url: String,
    pub error: String,
}

/// What the page last reported, reset when it goes away.
#[derive(Clone, Debug, PartialEq)]
pub struct Runtime {
    pub loading: bool,
    pub progress: f64,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// "Unable to load this page" is up in place of the page.
    pub error: Option<LoadError>,
    /// The renderer is gone and "Aw, Snap!" is up, with the reason.
    pub crashed: Option<String>,
    /// The page's zoom factor as last set or reported.
    pub zoom: f64,
    /// What the engine last said about the page's TLS, for the padlock.
    pub security: SecurityFacts,
    /// The page holds element fullscreen.
    pub fullscreen: bool,
    /// The page was asked whether it may go (`beforeunload`).
    pub closing: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            loading: false,
            progress: 0.,
            can_go_back: false,
            can_go_forward: false,
            error: None,
            crashed: None,
            zoom: 1.,
            security: SecurityFacts::default(),
            fullscreen: false,
            closing: false,
        }
    }
}

pub struct Tab {
    pub id: TabId,
    /// The address the tab shows; empty on the new tab page.
    pub url: String,
    pub title: String,
    pub pinned: bool,
    pub page: Option<WebView>,
    pub asleep: bool,
    /// The tab whose page opened this one, for Chrome's placement of the next one it opens.
    pub opener: Option<TabId>,
    pub favicon: Option<SharedString>,
    pub rt: Runtime,
    pub find: FindState,
    /// Where a page put to sleep was scrolled to, put back once it loads again.
    pub scroll: Option<(f64, f64)>,
}

pub fn page_options() -> WebViewOptions {
    WebViewOptions::default().adopt_popups().inspectable(true)
}

impl Tab {
    /// A tab at `url` whose page waits until it is shown.
    pub fn new(id: TabId, url: impl Into<String>) -> Self {
        Self {
            id,
            url: url.into(),
            title: String::new(),
            pinned: false,
            page: None,
            asleep: false,
            opener: None,
            favicon: None,
            rt: Runtime::default(),
            find: FindState::default(),
            scroll: None,
        }
    }

    /// A tab around a page that already exists: a popup a page opened and holds.
    pub fn with_page(id: TabId, url: impl Into<String>, page: WebView) -> Self {
        Self { page: Some(page), ..Self::new(id, url) }
    }

    pub fn label(&self) -> String {
        tab_label(&self.title, &self.url)
    }

    pub fn hover(&self) -> String {
        tab_hover(&self.title, &self.url)
    }

    pub fn is_live(&self) -> bool {
        self.page.is_some()
    }

    /// Gives the tab its page the first time it is shown, and again after it slept.
    pub fn wake(&mut self) {
        if self.page.is_some() || self.url.is_empty() {
            return;
        }
        let page = WebView::with_options(page_options());
        page.load(self.url.clone());
        self.page = Some(page);
        self.asleep = false;
    }

    /// Loads `url`. Chromium refuses to walk an existing page to `chrome:` and the other
    /// special schemes, so those get a fresh page created at the address.
    pub fn navigate(&mut self, url: String) {
        self.rt.error = None;
        self.rt.crashed = None;
        if !url.is_empty() && self.url == url && self.page.is_some() {
            if let Some(page) = &self.page {
                page.reload();
            }
            return;
        }
        self.url = url.clone();
        match &self.page {
            Some(page) if !create_at_url(&url) => page.load(url),
            _ => {
                self.page = None;
                // A fresh page starts at 100%; the window applies the host's level to it.
                self.rt = Runtime::default();
                self.wake();
            }
        }
    }

    /// Closes the page and keeps the tab's place, title, icon and address.
    pub fn sleep(&mut self, scroll: Option<(f64, f64)>) {
        self.page = None;
        self.asleep = true;
        self.scroll = scroll;
        self.find = FindState::default();
        self.rt = Runtime::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_tab_page_has_no_page_to_wake() {
        let mut tab = Tab::new(1, "");
        tab.wake();
        assert!(!tab.is_live());
        assert_eq!(tab.label(), "New Tab");
    }

    #[test]
    fn a_tab_labels_itself_by_title_then_address() {
        let mut tab = Tab::new(1, "https://example.com/a");
        assert_eq!(tab.label(), "example.com/a");
        tab.title = "Example".into();
        assert_eq!(tab.label(), "Example");
        assert_eq!(tab.hover(), "Example\nhttps://example.com/a");
    }
}
