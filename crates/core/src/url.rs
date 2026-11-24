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

static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?is)^([a-z][a-z0-9+.-]*):(.*)$").unwrap());
static IPV4: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{1,3}(\.\d{1,3}){3}$").unwrap());

/// Schemes that load as typed, `//` or not (`file:/tmp/a.html`, `https:example.com`). Any other
/// `word:` without `//` is a host and port or a user name.
const BARE_SCHEMES: &[&str] =
    &["http", "https", "file", "about", "data", "mailto", "tel", "view-source", "blob", "magnet"];
/// Names no public suffix lists that developers type anyway.
const LOCAL_SUFFIXES: &[&str] = &["localhost", "test", "local", "internal"];

pub fn encode_component(text: &str) -> String {
    utf8_percent_encode(text, COMPONENT).to_string()
}

pub fn is_search(url: &str, search_prefix: &str) -> bool {
    url.starts_with(search_prefix)
}

/// Omnibox text to a loadable address. Text names a site when its host is `localhost`, an IP
/// address or a name under a known suffix, so `localhost:8080`, `bücher.de` and `[::1]:3000`
/// navigate while `node.js` and `native desktop` search. A public name loads over https; an IP
/// address, `localhost` and a local suffix over http, as development servers and routers answer.
/// A `javascript:` address searches: the field never runs script on the page behind it.
pub fn to_url(raw: &str, search_prefix: &str) -> Option<String> {
    let q = typed_text(raw);
    if q.is_empty() {
        return None;
    }
    if let Some(c) = SCHEME.captures(&q) {
        let scheme = c[1].to_ascii_lowercase();
        if scheme != "javascript" && (c[2].starts_with("//") || BARE_SCHEMES.contains(&scheme.as_str())) {
            return Some(q);
        }
    }
    let address = format!("https://{q}");
    if names_a_site(&q, &address) {
        return Some(if is_local(&address) { format!("http://{q}") } else { address });
    }
    Some(format!("{search_prefix}{}", encode_component(&q)))
}

/// Pasted text loses its line breaks: a wrapped address joins up, and lines of prose become one
/// search. Tabs read as spaces.
pub fn typed_text(raw: &str) -> String {
    let lines: Vec<&str> = raw.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let joined = lines.concat();
    let text = if joined.contains(char::is_whitespace) { lines.join(" ") } else { joined };
    text.replace('\t', " ")
}

fn names_a_site(q: &str, address: &str) -> bool {
    if q.contains(char::is_whitespace) {
        return false;
    }
    let Ok(u) = Url::parse(address) else { return false };
    match u.host() {
        Some(url::Host::Ipv6(_)) => true,
        Some(url::Host::Ipv4(_)) => IPV4.is_match(typed_host(q)),
        Some(url::Host::Domain(d)) => {
            let last = d.rsplit('.').next().unwrap_or(d);
            let resolvable = d.len() <= 253 && d.split('.').all(|l| l.len() <= 63);
            resolvable
                && (d == "localhost"
                    || (d.contains('.')
                        && (LOCAL_SUFFIXES.contains(&last) || psl::suffix(d.as_bytes()).is_some_and(|s| s.is_known()))))
        }
        None => false,
    }
}

fn is_local(address: &str) -> bool {
    match Url::parse(address).ok().as_ref().and_then(Url::host) {
        Some(url::Host::Domain(d)) => LOCAL_SUFFIXES.contains(&d.rsplit('.').next().unwrap_or(d)),
        Some(_) => true,
        None => false,
    }
}

/// The host as typed: no user name, port or path. `[::1]` keeps its brackets.
fn typed_host(q: &str) -> &str {
    let end = q.find(['/', '?', '#']).unwrap_or(q.len());
    let authority = &q[..end];
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    match host.find(']') {
        Some(i) if host.starts_with('[') => &host[..=i],
        _ => host.split(':').next().unwrap_or(host),
    }
}

/// A host or address without its leading `www.`, which nobody types and no site means.
pub fn strip_www(host: &str) -> &str {
    host.strip_prefix("www.").unwrap_or(host)
}

/// The site a web page belongs to: its lower-case host without `www.`. Empty for anything that
/// is not `http:` or `https:`.
pub fn site_of(url: &str) -> String {
    match Url::parse(url) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => strip_www(&u.host_str().unwrap_or("").to_lowercase()).to_owned(),
        _ => String::new(),
    }
}

/// A page's origin, `https://a.test:8443`; empty where the origin is opaque (`file:`, `data:`,
/// `about:`), which no two pages share.
pub fn origin_of(url: &str) -> String {
    match Url::parse(url) {
        Ok(u) if u.origin().is_tuple() => u.origin().ascii_serialization(),
        _ => String::new(),
    }
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
    fn sites_and_origins() {
        assert_eq!(site_of("https://WWW.Example.com/a"), "example.com");
        assert_eq!(site_of("http://www.a.test:8080/"), "a.test");
        assert_eq!(site_of("chrome://settings"), "");
        assert_eq!(site_of("file:///etc/hosts"), "");
        assert_eq!(strip_www("www.a.test"), "a.test");
        assert_eq!(strip_www("wwwa.test"), "wwwa.test");
        assert_eq!(origin_of("https://a.test:8443/x"), "https://a.test:8443");
        assert_eq!(origin_of("https://a.test:443/x"), "https://a.test");
        assert_eq!(origin_of("file:///x"), "");
        assert_eq!(origin_of("data:text/plain,hi"), "");
        assert_eq!(origin_of(""), "");
    }

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
        assert_eq!(to_url("localhost:8080/x", PREFIX).as_deref(), Some("http://localhost:8080/x"));
        assert_eq!(to_url("mailto:a@b.c", PREFIX).as_deref(), Some("mailto:a@b.c"));
        assert_eq!(to_url("native desktop", PREFIX).as_deref(), Some("https://search.test/?q=native%20desktop"));
        assert_eq!(to_url("a.b", PREFIX).as_deref(), Some("https://search.test/?q=a.b"));
        assert_eq!(to_url("example.com:8080", PREFIX).as_deref(), Some("https://example.com:8080"));
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

    #[test]
    fn hosts_people_type() {
        let go = |q: &str| to_url(q, PREFIX).unwrap();
        let searched = |q: &str| go(q).starts_with(PREFIX);
        // IDN and punycode reach the same host; a mixed-script lookalike shows as punycode.
        assert_eq!(go("bücher.de"), "https://bücher.de");
        assert_eq!(go("xn--bcher-kva.de"), "https://xn--bcher-kva.de");
        assert_eq!(display_url(&go("аpple.com")), "xn--pple-43d.com");
        assert_eq!(go("localhost"), "http://localhost");
        assert_eq!(go("app.localhost:5173/x"), "http://app.localhost:5173/x");
        assert_eq!(go("printer.local"), "http://printer.local");
        assert_eq!(go("192.168.1.1:8080/admin"), "http://192.168.1.1:8080/admin");
        assert_eq!(go("[::1]:3000"), "http://[::1]:3000");
        assert_eq!(go("[2001:db8::1]"), "http://[2001:db8::1]");
        assert_eq!(go("user:pass@example.com"), "https://user:pass@example.com");
        assert!(searched("1.5"), "a number is not an address");
        assert!(searched("node.js"), "js is no suffix");
        assert!(searched("readme.txt"));
        assert!(!searched("readme.md"), "md is Moldova");
    }

    #[test]
    fn pasted_and_hostile_text() {
        let go = |q: &str| to_url(q, PREFIX).unwrap();
        assert_eq!(go("https://exam\nple.com/a\r\n/b"), "https://example.com/a/b");
        assert_eq!(go("first line\nsecond\tline"), format!("{PREFIX}first%20line%20second%20line"));
        assert_eq!(go("javascript:alert(1)"), format!("{PREFIX}javascript%3Aalert(1)"));
        assert_eq!(go("JavaScript://x.test"), format!("{PREFIX}JavaScript%3A%2F%2Fx.test"));
        assert_eq!(go("data:text/html,<b>hi</b>"), "data:text/html,<b>hi</b>");
        assert_eq!(go("file:/tmp/report.html"), "file:/tmp/report.html");
        assert_eq!(go("https:example.com"), "https:example.com");
        let long = "a".repeat(4000);
        assert_eq!(go(&long), format!("{PREFIX}{long}"));
        let long_host = format!("{}.com", "a".repeat(4000));
        assert!(go(&long_host).starts_with(PREFIX), "no label is longer than 63");
    }
}
