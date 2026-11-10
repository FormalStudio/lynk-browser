//! The Chrome Web Store: the page fixup that hides its "Switch to Chrome" nags, the script that
//! holds an install until the app's own dialog answers, and the lines that dialog shows.
//!
//! The store decides "is this Google Chrome" on the server, from the `X-Browser-Validation`
//! header only Google's own builds can sign; no Chromium build passes. The verdict only drives
//! two nags: the banner (state 17 of the item's info box, whose "Install Chrome" link carries
//! jslog 276336) and the "Switch to Chrome?" promo in the header. The install button works off
//! `chrome.webstorePrivate`. Hiding by CSS rather than removing nodes lets the info box come
//! back for the store's other states (incognito, admin blocked) on the same page.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

pub const STORE_ORIGINS: &[&str] = &["https://chromewebstore.google.com/*"];
pub const STORE_CHANNEL: &str = "ndStore";
pub const FIXUP_WORLD: &str = "nb-webstore";

fn store_css() -> String {
    ["[jscontroller=\"o2G9me\"]:has([jslog^=\"276336\"])", "[role=\"dialog\"][aria-labelledby=\"promo-header\"]"]
        .iter()
        .map(|rule| format!("{rule}{{display:none!important}}"))
        .collect()
}

/// Document start, in its own world, store pages only.
pub fn fixup_script() -> String {
    let css = serde_json::to_string(&store_css()).expect("a string serializes");
    format!(
        r#"(() => {{
  const style = document.createElement("style");
  style.textContent = {css};
  const place = () => (document.head || document.documentElement)?.append(style);
  if (document.documentElement) place();
  else new MutationObserver((_, o) => document.documentElement && (o.disconnect(), place())).observe(document, {{ childList: true }});
}})();"#
    )
}

/// Main world, document start, store pages only. It runs before the store's own code, so the
/// store never holds the unwrapped `beginInstallWithManifest3`. A cancel never reaches
/// Chromium; an accept lets the call through with the engine told to answer Chromium's own
/// prompt unseen.
pub fn store_script() -> String {
    format!(
        r#"(() => {{
  if (window.__ndStoreAnswer) return;
  const pending = new Map();
  let begin = null;
  let api = null;
  window.__ndStoreAnswer = (id, accept) => {{
    const held = pending.get(id);
    if (!held) return "none";
    pending.delete(id);
    if (!accept) {{
      held.cancel();
      return "cancelled";
    }}
    if (held.callback) begin.call(api, held.details, held.callback);
    else begin.call(api, held.details).then(held.resolve, held.reject);
    return "sent";
  }};
  const hook = () => {{
    const wp = window.chrome && window.chrome.webstorePrivate;
    if (!wp || typeof wp.beginInstallWithManifest3 !== "function") return false;
    api = wp;
    begin = wp.beginInstallWithManifest3;
    wp.beginInstallWithManifest3 = function (details, callback) {{
      const ask = (held) => {{
        pending.set(details.id, held);
        window.webkit.messageHandlers.{STORE_CHANNEL}.postMessage(JSON.stringify({{
          id: details.id, name: details.localizedName || "", iconUrl: details.iconUrl || "", manifest: details.manifest || "",
        }}));
      }};
      if (typeof callback === "function") {{
        ask({{ details, callback, cancel: () => callback("user_cancelled") }});
        return undefined;
      }}
      return new Promise((resolve, reject) => ask({{ details, resolve, reject, cancel: () => reject(new Error("User cancelled install")) }}));
    }};
    return true;
  }};
  if (!hook()) {{
    const timer = setInterval(() => {{ if (hook()) clearInterval(timer); }}, 50);
    setTimeout(() => clearInterval(timer), 30000);
  }}
}})()"#
    )
}

pub fn store_answer_script(id: &str, accept: bool) -> String {
    let id = serde_json::to_string(id).expect("a string serializes");
    format!(r#"window.__ndStoreAnswer ? window.__ndStoreAnswer({id}, {accept}) : "none""#)
}

/// What the store page asks the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreRequest {
    pub id: String,
    pub name: String,
    pub icon_url: String,
    pub manifest: String,
}

/// JavaScript's `String(v ?? "")`.
fn text(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// The message body the store script posts; `None` unless it names a real extension id.
pub fn parse_store_request(body: &str) -> Option<StoreRequest> {
    let raw: Value = serde_json::from_str(body).ok()?;
    let id = raw.get("id")?.as_str()?;
    if id.len() != 32 || !id.bytes().all(|b| (b'a'..=b'p').contains(&b)) {
        return None;
    }
    Some(StoreRequest {
        id: id.to_owned(),
        name: text(raw.get("name")),
        icon_url: text(raw.get("iconUrl")),
        manifest: text(raw.get("manifest")),
    })
}

/// Chrome's own wording for the permissions that carry a warning in its prompt. The rest
/// (storage, alarms, cookies without hosts) carry none there either.
const WARNINGS: &[(&str, &str)] = &[
    ("tabs", "Read your browsing history"),
    ("webNavigation", "Read your browsing history"),
    ("history", "Read and change your browsing history"),
    ("topSites", "Read a list of your most frequently visited websites"),
    ("bookmarks", "Read and change your bookmarks"),
    ("downloads", "Manage your downloads"),
    ("nativeMessaging", "Communicate with cooperating native applications"),
    ("notifications", "Display notifications"),
    ("clipboardRead", "Read data you copy and paste"),
    ("clipboardWrite", "Modify data you copy and paste"),
    ("privacy", "Change your privacy-related settings"),
    ("management", "Manage your apps, extensions, and themes"),
    ("declarativeNetRequest", "Block content on any page"),
    ("geolocation", "Detect your physical location"),
    ("contentSettings", "Change your settings that control websites' access to features"),
    ("proxy", "Read and change all your data on all websites"),
    ("debugger", "Access the page debugger backend"),
    ("desktopCapture", "Capture content of your screen"),
    ("tabCapture", "Read and change all your data on all websites"),
];

static ALL_HOSTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(<all_urls>|\*://\*/\*|https?://\*/\*|\*://\*/)$").unwrap());
static PATTERN_HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:\*|https?|wss?|ftp)://([^/]+)/").unwrap());

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The lines under "It can:" in the app's prompt, in Chrome's order: site access first, then the
/// rest, each said once.
pub fn permission_lines(manifest_json: &str) -> Vec<String> {
    let Ok(manifest) = serde_json::from_str::<Value>(manifest_json) else { return Vec::new() };
    let permissions = strings(manifest.get("permissions"));
    let mut patterns = strings(manifest.get("host_permissions"));
    patterns.extend(permissions.iter().filter(|p| p.contains("://") || *p == "<all_urls>").cloned());
    if let Some(scripts) = manifest.get("content_scripts").and_then(Value::as_array) {
        for script in scripts {
            patterns.extend(strings(script.get("matches")));
        }
    }
    let mut lines: Vec<String> = Vec::new();
    if patterns.iter().any(|p| ALL_HOSTS.is_match(p)) {
        lines.push("Read and change all your data on all websites".into());
    } else {
        let mut hosts: Vec<String> = Vec::new();
        for p in &patterns {
            let Some(c) = PATTERN_HOST.captures(p) else { continue };
            let host = c[1].strip_prefix("*.").unwrap_or(&c[1]).to_owned();
            if !host.is_empty() && !hosts.contains(&host) {
                hosts.push(host);
            }
        }
        match hosts.as_slice() {
            [] => {}
            [one] => lines.push(format!("Read and change your data on {one}")),
            [head @ .., last] if hosts.len() <= 3 => {
                lines.push(format!("Read and change your data on {} and {last}", head.join(", ")));
            }
            _ => lines.push("Read and change your data on a number of websites".into()),
        }
    }
    for p in &permissions {
        if let Some((_, line)) = WARNINGS.iter().find(|(k, _)| k == p)
            && !lines.iter().any(|l| l == line)
        {
            lines.push((*line).to_owned());
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_store_request_names_a_real_extension_id() {
        let id = "aeblfdkhhhdcdjpifhhbdiojplfjncoa";
        let body = json!({ "id": id, "name": "1Password", "iconUrl": "", "manifest": "{}" }).to_string();
        assert_eq!(parse_store_request(&body).map(|r| r.name).as_deref(), Some("1Password"));
        assert_eq!(parse_store_request(&json!({ "id": "../x", "name": "x" }).to_string()), None);
        assert_eq!(parse_store_request("not json"), None);
    }

    #[test]
    fn the_prompt_says_what_chromes_says() {
        let all = json!({ "permissions": ["tabs", "storage", "nativeMessaging", "webNavigation"], "host_permissions": ["<all_urls>"] });
        assert_eq!(
            permission_lines(&all.to_string()),
            [
                "Read and change all your data on all websites",
                "Read your browsing history",
                "Communicate with cooperating native applications",
            ]
        );
        let some = json!({ "content_scripts": [{ "matches": ["https://*.example.com/*", "https://a.test/*"] }] });
        assert_eq!(permission_lines(&some.to_string()), ["Read and change your data on example.com and a.test"]);
        let many = json!({ "host_permissions": ["https://a.test/*", "https://b.test/*", "https://c.test/*", "https://d.test/*"] });
        assert_eq!(permission_lines(&many.to_string()), ["Read and change your data on a number of websites"]);
        assert!(permission_lines(&json!({ "permissions": ["storage"] }).to_string()).is_empty());
        assert!(permission_lines("{").is_empty());
    }

    #[test]
    fn page_scripts_carry_the_channel_and_the_answer() {
        assert!(store_script().contains("window.webkit.messageHandlers.ndStore.postMessage"));
        assert!(
            fixup_script().contains(r#"[jscontroller=\"o2G9me\"]:has([jslog^=\"276336\"]){display:none!important}"#)
        );
        assert_eq!(
            store_answer_script("abc", true),
            r#"window.__ndStoreAnswer ? window.__ndStoreAnswer("abc", true) : "none""#
        );
    }
}
