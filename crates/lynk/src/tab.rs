//! One tab: what it shows, the page that shows it, and what that page last reported. A tab has
//! no page until it is first shown (a restored or background tab waits), none while it sleeps,
//! and none on the new tab page. It does not know its window: `window.rs` orders and shows tabs.

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
        }
    }
}

/// Where a tab's page is in its life.
pub enum Page {
    /// None yet: the tab was not shown since it was restored or opened in the background, or it
    /// is the new tab page.
    Waiting,
    /// Put to sleep, scrolled to `scroll`; shown again, it loads once more and goes back there.
    Asleep { scroll: Option<(f64, f64)> },
    Live(Live),
}

pub struct Live {
    pub view: WebView,
    /// Where to scroll once the load that woke the page ends.
    pub scroll_back: Option<(f64, f64)>,
}

pub struct Tab {
    pub id: TabId,
    /// The address the tab shows; empty on the new tab page.
    pub url: String,
    pub title: String,
    pub page: Page,
    /// The tab whose page opened this one, for Chrome's placement of the next one it opens.
    pub opener: Option<TabId>,
    pub favicon: Option<SharedString>,
    pub rt: Runtime,
    pub find: FindState,
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
            page: Page::Waiting,
            opener: None,
            favicon: None,
            rt: Runtime::default(),
            find: FindState::default(),
        }
    }

    /// A tab around a page that already exists: a popup a page opened and holds.
    pub fn with_page(id: TabId, url: impl Into<String>, view: WebView) -> Self {
        Self { page: Page::Live(Live { view, scroll_back: None }), ..Self::new(id, url) }
    }

    pub fn label(&self) -> String {
        tab_label(&self.title, &self.url)
    }

    pub fn hover(&self) -> String {
        tab_hover(&self.title, &self.url)
    }

    /// The live page's view.
    pub fn view(&self) -> Option<&WebView> {
        match &self.page {
            Page::Live(live) => Some(&live.view),
            _ => None,
        }
    }

    pub fn live_mut(&mut self) -> Option<&mut Live> {
        match &mut self.page {
            Page::Live(live) => Some(live),
            _ => None,
        }
    }

    /// Its address is http, https or file.
    pub fn is_web_page(&self) -> bool {
        ["http:", "https:", "file:"].iter().any(|s| self.url.starts_with(s))
    }

    pub fn is_live(&self) -> bool {
        matches!(self.page, Page::Live(_))
    }

    pub fn is_asleep(&self) -> bool {
        matches!(self.page, Page::Asleep { .. })
    }

    /// Gives the tab its page the first time it is shown, and again after it slept.
    pub fn wake(&mut self) {
        let scroll_back = match self.page {
            Page::Live(_) => return,
            _ if self.url.is_empty() => return,
            Page::Waiting => None,
            Page::Asleep { scroll } => scroll,
        };
        let view = WebView::with_options(page_options());
        view.load(self.url.clone());
        self.page = Page::Live(Live { view, scroll_back });
    }

    /// Drops the page for `page` with everything it reported.
    pub fn reset(&mut self, page: Page) {
        self.page = page;
        self.rt = Runtime::default();
        self.find = FindState::default();
    }

    /// Loads `url`. Chromium refuses to walk an existing page to `chrome:` and the other
    /// special schemes, so those get a fresh page created at the address.
    pub fn navigate(&mut self, url: String) {
        self.rt.error = None;
        self.rt.crashed = None;
        if let Some(view) = self.view().filter(|_| !url.is_empty() && self.url == url) {
            view.reload();
            return;
        }
        self.url = url.clone();
        match self.view() {
            Some(view) if !create_at_url(&url) => view.load(url),
            _ => {
                // A fresh page starts at 100%; the window applies the host's level to it.
                self.reset(Page::Waiting);
                self.wake();
            }
        }
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
    fn a_tab_wakes_where_it_slept() {
        let mut tab = Tab::new(1, "https://example.com/");
        tab.rt.loading = true;
        tab.reset(Page::Asleep { scroll: Some((0., 600.)) });
        assert!(tab.is_asleep());
        assert!(!tab.rt.loading);
        tab.wake();
        assert!(tab.live_mut().is_some_and(|live| live.scroll_back == Some((0., 600.))));
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
