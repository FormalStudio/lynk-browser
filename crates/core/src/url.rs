//! Address field text and the addresses pages report.

use std::sync::LazyLock;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, utf8_percent_encode};
use regex::Regex;
use url::Url;

/// What JavaScript's `encodeURIComponent` leaves alone, so a search address is byte for byte
/// what the old app built.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[a-z][a-z0-9+.-]*:").unwrap());
static DOTTED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\S+\.\S{2,}$").unwrap());
static LOCALHOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^localhost(:\d+)?(/|$)").unwrap());

pub fn encode_component(text: &str) -> String {
    utf8_percent_encode(text, COMPONENT).to_string()
}

pub fn is_search(url: &str, search_prefix: &str) -> bool {
    url.starts_with(search_prefix)
}

/// Omnibox text to a loadable address. A bare word with no dot is a search, so
/// `localhost:8080` and `example.com` navigate while `native desktop` searches.
pub fn to_url(raw: &str, search_prefix: &str) -> Option<String> {
    let q = raw.trim();
    if q.is_empty() {
        return None;
    }
    // `localhost:` would otherwise read as a scheme.
    if LOCALHOST.is_match(q) {
        return Some(format!("https://{q}"));
    }
    if SCHEME.is_match(q) {
        return Some(q.to_owned());
    }
    if DOTTED.is_match(q) {
        return Some(format!("https://{q}"));
    }
    Some(format!("{search_prefix}{}", encode_component(q)))
}

/// The WHATWG `host`: hostname and a port that is not the scheme's default.
pub fn host_of(url: &str) -> String {
    Url::parse(url).map(|u| host_port(&u)).unwrap_or_default()
}

pub(crate) fn host_port(u: &Url) -> String {
    let host = u.host_str().unwrap_or("");
    match u.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    }
}

/// The WHATWG `search`: empty for no query and for a bare `?`.
pub(crate) fn search_of(u: &Url) -> String {
    match u.query() {
        Some(q) if !q.is_empty() => format!("?{q}"),
        _ => String::new(),
    }
}

/// Sidebar caption and omnibox text: host plus path, without the scheme noise.
pub fn display_url(url: &str) -> String {
    if url.is_empty() {
        return String::new();
    }
    match Url::parse(url) {
        Ok(u) => {
            let path = if u.path() == "/" { "" } else { u.path() };
            format!("{}{path}{}", host_port(&u), search_of(&u))
        }
        Err(_) => url.to_owned(),
    }
}

/// What the address field's text means. The field shows the address without its scheme, so
/// handed back untouched it is the page's own address: an `http:` page is not sent to https.
pub fn field_address(raw: &str, current: &str) -> String {
    if !current.is_empty() && raw.trim() == display_url(current) { current.to_owned() } else { raw.to_owned() }
}

/// Last path segment of a download address, or a neutral fallback.
pub fn file_name_from_url(url: &str) -> String {
    let Ok(u) = Url::parse(url) else { return "download".into() };
    let last = u.path().split('/').rfind(|s| !s.is_empty());
    match last {
        Some(name) => percent_decode_str(name).decode_utf8().map_or_else(|_| "download".into(), |s| s.into_owned()),
        None => "download".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIX: &str = "https://search.test/?q=";

    #[test]
    fn an_untouched_address_keeps_the_pages_own_scheme() {
        let page = "http://127.0.0.1:8080/deep/path?q=1";
        assert_eq!(field_address("127.0.0.1:8080/deep/path?q=1", page), page);
        assert_eq!(field_address(" 127.0.0.1:8080/deep/path?q=1 ", page), page);
        assert_eq!(to_url(&field_address("127.0.0.1:8080/deep/path?q=1", page), PREFIX).as_deref(), Some(page));
    }

    #[test]
    fn an_edited_address_is_typed_text() {
        assert_eq!(field_address("example.com", "http://127.0.0.1:8080/"), "example.com");
        assert_eq!(field_address("example.com", ""), "example.com");
    }

    #[test]
    fn typed_text_becomes_an_address_or_a_search() {
        assert_eq!(to_url("  ", PREFIX), None);
        assert_eq!(to_url("example.com", PREFIX).as_deref(), Some("https://example.com"));
        assert_eq!(to_url("localhost:8080/x", PREFIX).as_deref(), Some("https://localhost:8080/x"));
        assert_eq!(to_url("mailto:a@b.c", PREFIX).as_deref(), Some("mailto:a@b.c"));
        assert_eq!(to_url("native desktop", PREFIX).as_deref(), Some("https://search.test/?q=native%20desktop"));
        assert_eq!(to_url("a.b", PREFIX).as_deref(), Some("https://search.test/?q=a.b"));
        assert_eq!(to_url("c++ & (rust)!", PREFIX).as_deref(), Some("https://search.test/?q=c%2B%2B%20%26%20(rust)!"));
    }

    #[test]
    fn display_and_file_names() {
        assert_eq!(display_url("https://www.example.com/"), "www.example.com");
        assert_eq!(display_url("https://example.com:8443/a?b=1"), "example.com:8443/a?b=1");
        assert_eq!(display_url("http://example.com:80/a?"), "example.com/a");
        assert_eq!(display_url("not a url"), "not a url");
        assert_eq!(host_of("about:blank"), "");
        assert_eq!(file_name_from_url("https://x.test/files/r%C3%A9sum%C3%A9.pdf"), "résumé.pdf");
        assert_eq!(file_name_from_url("https://x.test/"), "download");
        assert_eq!(file_name_from_url("https://x.test/%FF"), "download");
    }
}
