//! Chromium pages whose job belongs to the browser's own UI, the way Arc replaced them: every
//! route to one (typed, a link, an extension's `chrome.tabs.create`, a restored tab) lands on the
//! app's native surface. `docs/native-pages.md` has the reasoning, and the pages that stay
//! Chromium's.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativePage {
    Downloads,
    History,
    Bookmarks,
    NewTab,
    TabSearch,
}

const HOSTS: &[(&str, NativePage)] = &[
    ("downloads", NativePage::Downloads),
    ("history", NativePage::History),
    ("history-clusters", NativePage::History),
    ("bookmarks", NativePage::Bookmarks),
    ("newtab", NativePage::NewTab),
    ("new-tab-page", NativePage::NewTab),
    ("new-tab-page-third-party", NativePage::NewTab),
    ("tab-search.top-chrome", NativePage::TabSearch),
    ("tab-search", NativePage::TabSearch),
];

pub fn native_page(url: &str) -> Option<NativePage> {
    let u = url.trim().to_lowercase();
    // Chromium rewrites these about: forms to chrome://, and the local NTP is the new tab page
    // under another name.
    match u.as_str() {
        "about:newtab" => return Some(NativePage::NewTab),
        "about:downloads" => return Some(NativePage::Downloads),
        "about:history" => return Some(NativePage::History),
        "about:bookmarks" => return Some(NativePage::Bookmarks),
        _ if u.starts_with("chrome-search://local-ntp") => return Some(NativePage::NewTab),
        _ => {}
    }
    let rest = u.strip_prefix("chrome://")?;
    let end = rest.find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-'));
    let host = &rest[..end.unwrap_or(rest.len())];
    if host.is_empty() {
        return None;
    }
    HOSTS.iter().find(|(h, _)| *h == host).map(|(_, page)| *page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chromiums_own_pages_map_to_the_apps_surfaces() {
        assert_eq!(native_page("chrome://downloads"), Some(NativePage::Downloads));
        assert_eq!(native_page("chrome://downloads/?q=report"), Some(NativePage::Downloads));
        assert_eq!(native_page("about:downloads"), Some(NativePage::Downloads));
        assert_eq!(native_page("chrome://newtab/"), Some(NativePage::NewTab));
        assert_eq!(native_page("chrome://new-tab-page/"), Some(NativePage::NewTab));
        assert_eq!(native_page("chrome-search://local-ntp/local-ntp.html"), Some(NativePage::NewTab));
        assert_eq!(native_page("chrome://tab-search.top-chrome/"), Some(NativePage::TabSearch));
        assert_eq!(native_page("chrome://history/?q=news"), Some(NativePage::History));
        assert_eq!(native_page("chrome://bookmarks/?id=1"), Some(NativePage::Bookmarks));
    }

    #[test]
    fn the_pages_that_stay_chromiums_are_left_alone() {
        for url in [
            "chrome://settings",
            "chrome://extensions",
            "chrome://flags",
            "chrome://settings/clearBrowserData",
            "https://downloads.example.com/",
        ] {
            assert_eq!(native_page(url), None, "{url}");
        }
    }
}
