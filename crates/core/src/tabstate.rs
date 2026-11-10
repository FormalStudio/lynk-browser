//! Per-tab facts the sidebar, the padlock and the find bar read.

use crate::url::display_url;

/// What the padlock says. `None` is not a verdict: it is the new tab page and anything else
/// that never had a chance to be encrypted, and warning there would spend the indicator's
/// credibility on a non-event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Security {
    #[default]
    None,
    Secure,
    Mixed,
    Insecure,
    Invalid,
}

impl Security {
    /// `None` is the plain site-information mark rather than the globe, which is also the
    /// favicon of a site without one: compact draws the two side by side.
    pub fn icon(self) -> &'static str {
        match self {
            Self::None => "help-about-symbolic",
            Self::Secure => "channel-secure-symbolic",
            Self::Mixed | Self::Invalid => "dialog-warning-symbolic",
            Self::Insecure => "channel-insecure-symbolic",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Self::None => "No page loaded",
            Self::Secure => "This page uses a valid certificate",
            Self::Mixed => "Parts of this page are not encrypted",
            Self::Insecure => "This page is not encrypted",
            Self::Invalid => "This site's certificate could not be verified",
        }
    }
}

/// The TLS facts the engine reports for a page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SecurityFacts {
    pub secure: bool,
    pub insecure_content: bool,
    pub error: Option<String>,
}

/// What the facts mean for this address. A page that never had a chance to be encrypted
/// (`file:`, an extension page, `about:`) is not "insecure", it is not a site.
pub fn security_of(url: &str, facts: &SecurityFacts) -> Security {
    if facts.error.as_deref().is_some_and(|e| !e.is_empty()) {
        Security::Invalid
    } else if facts.insecure_content {
        Security::Mixed
    } else if facts.secure {
        Security::Secure
    } else if url.starts_with("http://") {
        Security::Insecure
    } else {
        Security::None
    }
}

/// What a tab's speaker shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabAudio {
    Playing,
    Muted,
}

impl TabAudio {
    pub fn icon(self) -> &'static str {
        match self {
            Self::Playing => "audio-volume-high-symbolic",
            Self::Muted => "audio-volume-muted-symbolic",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Self::Playing => "Mute tab",
            Self::Muted => "Unmute tab",
        }
    }
}

pub fn tab_audio(audible: bool, muted: bool) -> Option<TabAudio> {
    if muted { Some(TabAudio::Muted) } else { audible.then_some(TabAudio::Playing) }
}

/// `count` and `found` stay `None` until the engine answers, so the bar never claims a result
/// it does not have yet. WebKit reports only whether something matched, so `count` can stay
/// `None` with `found` set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindState {
    pub open: bool,
    pub query: String,
    pub count: Option<u32>,
    pub found: Option<bool>,
}

/// A search the engine answered with nothing. A query not reported on yet is not a failure,
/// and an empty query is not a search.
pub fn find_failed(find: &FindState) -> bool {
    !find.query.is_empty() && (find.count == Some(0) || find.found == Some(false))
}

pub fn find_summary(find: &FindState) -> String {
    if find.query.is_empty() {
        return String::new();
    }
    match (find.count, find.found) {
        (Some(1), _) => "1 match".into(),
        (Some(n), _) => format!("{n} matches"),
        (None, None) => String::new(),
        (None, Some(true)) => "Found".into(),
        (None, Some(false)) => "No matches".into(),
    }
}

/// Addresses a page has to be created at: Chromium refuses a renderer-initiated navigation from
/// `about:blank` to any of these, so a restored `chrome://` tab would come back blank.
pub fn create_at_url(url: &str) -> bool {
    ["chrome:", "chrome-extension:", "devtools:", "view-source:"].iter().any(|s| url.starts_with(s))
}

/// What a tab calls itself wherever it is listed: its page's title, the address until there is
/// one, and "New Tab" until there is an address.
pub fn tab_label(title: &str, url: &str) -> String {
    if !title.is_empty() {
        title.to_owned()
    } else if !url.is_empty() {
        display_url(url)
    } else {
        "New Tab".into()
    }
}

/// Hovering a tab, row, chip or tile: its title, and under it the whole address.
pub fn tab_hover(title: &str, url: &str) -> String {
    if url.is_empty() {
        "New Tab".into()
    } else if title.is_empty() {
        url.to_owned()
    } else {
        format!("{title}\n{url}")
    }
}

/// How a window is named where another window offers to send a tab to it: after the tab it
/// shows, the way Chrome's Move Tab menu names one.
pub fn window_label(active_title: &str, active_url: &str, count: usize) -> String {
    let label = tab_label(active_title, active_url);
    match count {
        0 | 1 => label,
        2 => format!("{label} and 1 more tab"),
        n => format!("{label} and {} more tabs", n - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_padlock_reads_the_facts_for_this_address() {
        let none = SecurityFacts::default();
        assert_eq!(security_of("http://a.test/", &none), Security::Insecure);
        assert_eq!(security_of("file:///x", &none), Security::None);
        assert_eq!(security_of("https://a.test/", &SecurityFacts { secure: true, ..none.clone() }), Security::Secure);
        let mixed = SecurityFacts { secure: true, insecure_content: true, error: None };
        assert_eq!(security_of("https://a.test/", &mixed), Security::Mixed);
        let bad = SecurityFacts { error: Some("cert".into()), ..mixed };
        assert_eq!(security_of("https://a.test/", &bad), Security::Invalid);
        assert_eq!(Security::Invalid.tooltip(), "This site's certificate could not be verified");
    }

    #[test]
    fn find_summary_never_claims_an_answer_it_does_not_have() {
        let f = |query: &str, count, found| FindState { open: true, query: query.into(), count, found };
        assert_eq!(find_summary(&f("", Some(3), None)), "");
        assert_eq!(find_summary(&f("a", None, None)), "");
        assert_eq!(find_summary(&f("a", Some(1), None)), "1 match");
        assert_eq!(find_summary(&f("a", Some(0), None)), "0 matches");
        assert_eq!(find_summary(&f("a", None, Some(true))), "Found");
        assert_eq!(find_summary(&f("a", None, Some(false))), "No matches");
        assert!(find_failed(&f("a", Some(0), None)));
        assert!(!find_failed(&f("a", None, None)));
        assert!(!find_failed(&f("", Some(0), None)));
    }

    #[test]
    fn labels_and_audio() {
        assert_eq!(tab_label("", ""), "New Tab");
        assert_eq!(tab_label("", "https://a.test/x"), "a.test/x");
        assert_eq!(tab_hover("A", "https://a.test/"), "A\nhttps://a.test/");
        assert_eq!(window_label("A", "", 3), "A and 2 more tabs");
        assert_eq!(window_label("A", "", 2), "A and 1 more tab");
        assert_eq!(tab_audio(true, true), Some(TabAudio::Muted));
        assert_eq!(tab_audio(true, false).map(TabAudio::tooltip), Some("Mute tab"));
        assert_eq!(tab_audio(false, false), None);
        assert!(create_at_url("chrome://settings") && create_at_url("view-source:https://a.test/"));
        assert!(!create_at_url("https://chrome.test/"));
    }
}
