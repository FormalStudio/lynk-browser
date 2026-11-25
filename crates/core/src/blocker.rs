//! The compiled lists and the user's own rules answering for pages: what a request gets, the
//! policy a document's `$csp=` rules add, what goes at the top of a document's head (hide rules
//! and scriptlets for its address) and which generic rules hide the classes and ids a page
//! reports. Sync, so the request filter can ask from Chromium's IO thread. It neither knows
//! which sites blocking is off for nor reaches a page.

use std::collections::HashSet;

use adblock::Engine;
use adblock::request::Request;
use base64::Engine as _;

/// What a request gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Block,
    /// The request goes to this address instead (`$removeparam=` and the like).
    Redirect(String),
    /// Answered without the network with a stand-in (`$redirect=`), such as a no-op script.
    Respond {
        mime: String,
        body: Vec<u8>,
    },
}

pub struct Blocker {
    lists: Engine,
    user: Engine,
}

/// A document's cosmetics: rules that hide by address, and the scriptlets for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Cosmetics {
    pub hide: Vec<String>,
    pub script: String,
    /// The page has the generic rules switched off (`$generichide`).
    pub generic_off: bool,
    /// Procedural rules (`:has-text()`, `:upward()`, `:remove()` and the like) as adblock-rust's
    /// JSON, `{selector: [{type, arg}], action}`, sorted. They need the page's DOM to run.
    pub procedural: Vec<String>,
    exceptions: HashSet<String>,
}

impl Blocker {
    pub fn new(lists: Engine, user: Engine) -> Self {
        Self { lists, user }
    }

    /// `source` is the page's address, `kind` Chromium's resource type name (`script`, `image`,
    /// `sub_frame`).
    pub fn check(&self, url: &str, source: &str, kind: &str, method: &str) -> Decision {
        let Ok(request) = Request::new(url, source, kind, method) else { return Decision::Allow };
        let result = self.lists.check_network_request(&request);
        if let Some((mime, body)) = result.redirect.as_deref().and_then(data_url) {
            return Decision::Respond { mime, body };
        }
        if result.should_block() {
            return Decision::Block;
        }
        result.rewritten_url.map_or(Decision::Allow, Decision::Redirect)
    }

    /// The policy `$csp=` rules add to a document at `url`.
    pub fn csp(&self, url: &str, source: &str, kind: &str) -> Option<String> {
        let request = Request::new(url, source, kind, "GET").ok()?;
        self.lists.get_csp_directives(&request).filter(|p| !p.trim().is_empty())
    }

    /// The lists' and the user's cosmetics for a document at `url`.
    pub fn cosmetics(&self, url: &str) -> Cosmetics {
        let lists = self.lists.url_cosmetic_resources(url);
        let user = self.user.url_cosmetic_resources(url);
        let mut hide: Vec<String> = lists.hide_selectors.into_iter().chain(user.hide_selectors).collect();
        hide.sort();
        hide.dedup();
        let mut procedural: Vec<String> = lists.procedural_actions.into_iter().chain(user.procedural_actions).collect();
        procedural.sort();
        procedural.dedup();
        let script = [lists.injected_script, user.injected_script].join("\n").trim().to_owned();
        // uBO's scriptlets read a `scriptletGlobals` that its own injector declares around them.
        let script = if script.is_empty() {
            script
        } else {
            format!("(function () {{ const scriptletGlobals = {{}};\n{script}\n}})();")
        };
        Cosmetics { hide, script, generic_off: lists.generichide, procedural, exceptions: lists.exceptions }
    }

    /// The generic rules that hide some of `classes` and `ids`, for a page at `cosmetics`' address.
    pub fn generic(&self, cosmetics: &Cosmetics, classes: &[String], ids: &[String]) -> Vec<String> {
        if cosmetics.generic_off {
            return Vec::new();
        }
        self.lists.hidden_class_id_selectors(classes, ids, &cosmetics.exceptions)
    }
}

impl Cosmetics {
    /// The HTML for the top of the document's head: a style sheet hiding the matched elements,
    /// then the scriptlets, which run before any of the page's own scripts. Empty when there is
    /// nothing to add.
    pub fn head(&self) -> String {
        let mut out = String::new();
        if !self.hide.is_empty() {
            out.push_str("<style>");
            out.push_str(&hide_rule(&self.hide).replace("</", "<\\/"));
            out.push_str("</style>");
        }
        if !self.script.trim().is_empty() {
            out.push_str("<script>");
            out.push_str(&self.script.replace("</script", "<\\/script"));
            out.push_str("</script>");
        }
        out
    }
}

/// A rule per selector hiding it: one the browser cannot parse would void a rule it shared.
pub fn hide_rule(selectors: &[String]) -> String {
    selectors.iter().map(|s| format!("{s}{{display:none!important}}")).collect()
}

/// `data:<mime>;base64,<body>`, the form adblock-rust hands a `redirect=` resource in.
fn data_url(url: &str) -> Option<(String, Vec<u8>)> {
    let (head, body) = url.strip_prefix("data:")?.split_once(',')?;
    let mime = head.strip_suffix(";base64")?;
    let body = base64::engine::general_purpose::STANDARD.decode(body).ok()?;
    Some((mime.to_owned(), body))
}

#[cfg(test)]
mod tests {
    use adblock::lists::{FilterFormat, FilterSet, ParseOptions};
    use adblock::resources::{MimeType, PermissionMask, Resource, ResourceType};

    use super::*;
    use crate::adblock::{BlockingState, user_engine};
    use crate::ublock::{Scriptlet, scriptlet_resources};

    /// Lines as they stand in EasyList, EasyPrivacy, uBlock filters and Peter Lowe's list.
    const EASYLIST: &str = r#"! Title: EasyList
||doubleclick.net^
||googlesyndication.com^$third-party
||securepubads.g.doubleclick.net/tag/js/gpt.js$script,redirect=googletagservices_gpt.js,important
@@||cdn.taboola.com/libtrc/static/thumbnails/$image,domain=example.com
##.adsbygoogle
##.ad-banner
###ad-slot
reddit.com##shreddit-ad-post
example.com#@#.ad-banner
@@||news.example^$generichide
youtube.com##+js(set, ytInitialPlayerResponse.adPlacements, undefined)
||thepiratebay.org^$csp=script-src 'self' 'unsafe-inline'
$removeparam=utm_source
"#;
    const EASYPRIVACY: &str = "! Title: EasyPrivacy\n||google-analytics.com^$third-party\n||hotjar.com^$third-party\n";
    const PLOWE: &str = "127.0.0.1 adnxs.com\n127.0.0.1 criteo.com\n";

    fn blocker(state: &BlockingState) -> Blocker {
        let mut set = FilterSet::new(false);
        set.add_filter_list(EASYLIST.into(), ParseOptions::default());
        set.add_filter_list(EASYPRIVACY.into(), ParseOptions::default());
        set.add_filter_list(PLOWE.into(), ParseOptions { format: FilterFormat::Hosts, ..ParseOptions::default() });
        let mut lists = Engine::new_with_filter_set(set);
        let gpt = Resource {
            name: "googletagservices_gpt.js".into(),
            aliases: vec![],
            kind: ResourceType::Mime(MimeType::ApplicationJavascript),
            content: base64::engine::general_purpose::STANDARD.encode("window.googletag={cmd:[]};"),
            dependencies: vec![],
            permission: PermissionMask::default(),
        };
        let set = Scriptlet {
            name: "set-constant.js".into(),
            aliases: vec!["set.js".into()],
            source: "function setConstant(chain = '', value = '') { window.__set = [chain, value]; }".into(),
            dependencies: vec![],
        };
        let mut resources = scriptlet_resources(&[set]);
        resources.push(gpt);
        lists.use_resources(resources.clone());
        let mut user = user_engine(state);
        user.use_resources(resources);
        Blocker::new(lists, user)
    }

    #[test]
    fn real_ad_and_tracker_addresses_are_blocked_and_first_party_content_is_not() {
        let b = blocker(&BlockingState::default());
        let page = "https://www.theguardian.com/world";
        let block = |url: &str, kind: &str| b.check(url, page, kind, "GET") == Decision::Block;
        assert!(block("https://ad.doubleclick.net/ddm/adj/N123/B456;sz=300x250", "script"));
        assert!(block("https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js", "script"));
        assert!(block("https://www.google-analytics.com/analytics.js", "script"));
        assert!(block("https://static.hotjar.com/c/hotjar-123.js?sv=6", "script"));
        assert!(block("https://ib.adnxs.com/ut/v3/prebid", "xmlhttprequest"));
        assert!(block("https://static.criteo.com/js/ld/publishertag.js", "script"));
        assert!(!block("https://assets.guim.co.uk/javascripts/app.js", "script"));
        assert!(!block("https://i.guim.co.uk/img/media/photo.jpg", "image"));
        // `$third-party`: Google's own pages load from their own ad domain.
        let own =
            b.check("https://pagead2.googlesyndication.com/x.js", "https://googlesyndication.com/", "script", "GET");
        assert_eq!(own, Decision::Allow);
    }

    #[test]
    fn a_redirect_rule_answers_with_its_stand_in_and_an_exception_lets_through() {
        let b = blocker(&BlockingState::default());
        let gpt = b.check("https://securepubads.g.doubleclick.net/tag/js/gpt.js", "https://cnn.com/", "script", "GET");
        let Decision::Respond { mime, body } = gpt else { panic!("{gpt:?}") };
        assert_eq!((mime.as_str(), body.as_slice()), ("application/javascript", &b"window.googletag={cmd:[]};"[..]));
        let thumb = "https://cdn.taboola.com/libtrc/static/thumbnails/a.jpg";
        assert_eq!(b.check(thumb, "https://example.com/", "image", "GET"), Decision::Allow);
        let stripped = b.check("https://shop.test/p?utm_source=mail&id=4", "https://shop.test/", "document", "GET");
        assert_eq!(stripped, Decision::Redirect("https://shop.test/p?id=4".into()));
    }

    #[test]
    fn a_csp_rule_adds_its_policy_to_its_site_only() {
        let b = blocker(&BlockingState::default());
        assert_eq!(
            b.csp("https://thepiratebay.org/search", "https://thepiratebay.org/search", "document").as_deref(),
            Some("script-src 'self' 'unsafe-inline'")
        );
        assert_eq!(b.csp("https://example.com/", "https://example.com/", "document"), None);
    }

    #[test]
    fn cosmetics_by_address_go_in_the_head_and_scriptlets_run_there() {
        let mut state = BlockingState::default();
        state.hide("www.reddit.com", "div.promoted", "Promoted", "", 1);
        let b = blocker(&state);
        let reddit = b.cosmetics("https://www.reddit.com/r/rust/");
        assert_eq!(reddit.hide, ["div.promoted", "shreddit-ad-post"]);
        assert_eq!(
            reddit.head(),
            "<style>div.promoted{display:none!important}shreddit-ad-post{display:none!important}</style>"
        );
        let yt = b.cosmetics("https://www.youtube.com/watch?v=x");
        assert!(yt.hide.is_empty());
        let head = yt.head();
        assert!(head.starts_with("<script>") && head.contains("setConstant"), "{head}");
        assert!(head.contains("ytInitialPlayerResponse.adPlacements"), "{head}");
        assert_eq!(b.cosmetics("https://example.org/").head(), "");
    }

    #[test]
    fn generic_rules_hide_the_classes_and_ids_a_page_reports_with_its_exceptions() {
        let b = blocker(&BlockingState::default());
        let classes = vec!["adsbygoogle".to_owned(), "ad-banner".to_owned(), "article".to_owned()];
        let ids = vec!["ad-slot".to_owned(), "main".to_owned()];
        let mut on = b.generic(&b.cosmetics("https://www.bbc.com/news"), &classes, &ids);
        on.sort();
        assert_eq!(on, ["#ad-slot", ".ad-banner", ".adsbygoogle"]);
        let mut excepted = b.generic(&b.cosmetics("https://example.com/"), &classes, &ids);
        excepted.sort();
        assert_eq!(excepted, ["#ad-slot", ".adsbygoogle"]);
        assert!(b.generic(&b.cosmetics("https://news.example/"), &classes, &ids).is_empty());
    }

    #[test]
    fn head_text_never_closes_its_own_tag() {
        let c = Cosmetics {
            hide: vec!["a[title='</style>']".into()],
            script: "x('</script>')".into(),
            ..Cosmetics::default()
        };
        let head = c.head();
        assert_eq!(head.matches("</style>").count(), 1);
        assert_eq!(head.matches("</script>").count(), 1);
    }

    #[test]
    fn procedural_rules_from_the_lists_and_the_user_come_as_json() {
        let mut state = BlockingState::default();
        state.hide("example.com", ".post:has-text(Sponsored)", "Sponsored", "", 1);
        state.hide("example.com", ".promo:remove()", "Promo", "", 1);
        let b = blocker(&state);
        let c = b.cosmetics("https://example.com/");
        assert_eq!(c.procedural.len(), 2, "{:?}", c.procedural);
        let rules: Vec<serde_json::Value> = c.procedural.iter().map(|r| serde_json::from_str(r).unwrap()).collect();
        assert!(rules.iter().any(|r| r["selector"][1]["type"] == "has-text" && r["selector"][1]["arg"] == "Sponsored"));
        assert!(rules.iter().any(|r| r["action"]["type"] == "remove"));
        assert!(c.hide.is_empty(), "procedural rules stay out of the style sheet: {:?}", c.hide);
        assert!(b.cosmetics("https://other.example/").procedural.is_empty());
    }

    #[test]
    fn the_users_scriptlets_join_the_lists() {
        let mut state = BlockingState::default();
        state.hide("example.com", "+js(set-constant, lynkSet, 1)", "Scriptlet", "", 1);
        let script = blocker(&state).cosmetics("https://example.com/").script;
        assert!(script.contains("lynkSet"), "{script}");
        assert!(script.starts_with("(function () { const scriptletGlobals = {};"), "{script}");
    }
}
