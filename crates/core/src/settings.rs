//! What the user sets. The keys and defaults are the 0.5.11 app's `settings.json`, so its file
//! reads unchanged through [`LynkSettings::from_legacy`].

use std::collections::BTreeMap;
use std::path::Path;

use nd_settings::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::permissions::{PermissionDecision, SitePermissions};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SearchEngineId {
    #[default]
    Duckduckgo,
    Google,
    Bing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchEngine {
    pub id: SearchEngineId,
    pub name: &'static str,
    /// Everything before the URL-encoded query. A search is recognised by testing an address
    /// against this prefix.
    pub prefix: &'static str,
}

pub const SEARCH_ENGINES: [SearchEngine; 3] = [
    SearchEngine { id: SearchEngineId::Duckduckgo, name: "DuckDuckGo", prefix: "https://duckduckgo.com/?q=" },
    SearchEngine { id: SearchEngineId::Google, name: "Google", prefix: "https://www.google.com/search?q=" },
    SearchEngine { id: SearchEngineId::Bing, name: "Bing", prefix: "https://www.bing.com/search?q=" },
];

pub fn engine_of(id: SearchEngineId) -> &'static SearchEngine {
    SEARCH_ENGINES.iter().find(|e| e.id == id).unwrap_or(&SEARCH_ENGINES[0])
}

/// `Sidebar` is the vertical tab column; `Compact` moves the tab affordances into one toolbar
/// row, for reading a single page.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    #[default]
    Sidebar,
    Compact,
}

pub const LAYOUTS: [(Layout, &str); 2] = [(Layout::Sidebar, "Sidebar"), (Layout::Compact, "Compact")];

/// How a pinned tile reads: the site's icon (its letter until there is one), or only the letter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PinStyle {
    #[default]
    Icons,
    Letters,
}

pub const PIN_STYLES: [(PinStyle, &str); 2] = [(PinStyle::Icons, "Icons"), (PinStyle::Letters, "Letters")];

#[derive(Settings, Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct LynkSettings {
    /// Where typed text that is not an address is searched.
    pub search_engine: SearchEngineId,
    /// Where new windows open. Empty opens the new tab page.
    pub homepage: String,
    /// Start with the pinned tabs and a new tab rather than last time's tabs.
    pub fresh_window: bool,
    pub layout: Layout,
    /// How pinned tabs read in the sidebar.
    pub pin_style: PinStyle,
    /// What each site may do, by origin and then by permission type. Only the Allow and Block
    /// buttons write here; a dismissed prompt is a deny for that request alone.
    pub site_permissions: SitePermissions,
    /// Extension ids with a toolbar button, in pin order. An id no longer installed stays, so
    /// reinstalling the extension brings its button back.
    pub pinned_extensions: Vec<String>,
    /// Where downloads are saved. Empty means the system's Downloads folder.
    pub download_dir: String,
    /// Show a save panel for every download instead of saving straight away.
    pub ask_where_to_save: bool,
}

impl LynkSettings {
    /// The settings a 0.5.11 `settings.json` holds, its `data` object. A missing key or one of
    /// the wrong shape takes its default; a file from before `freshWindow` existed has
    /// `restoreOnLaunch`, and that one off is the same fresh start.
    pub fn from_legacy(data: &Value) -> Self {
        let d = Self::default();
        let get = |key: &str| data.get(key);
        let string = |key: &str, fallback: String| get(key).and_then(Value::as_str).map_or(fallback, str::to_owned);
        Self {
            search_engine: parse(data, "searchEngine").unwrap_or(d.search_engine),
            homepage: string("homepage", d.homepage),
            fresh_window: match get("freshWindow") {
                Some(Value::Bool(b)) => *b,
                _ => get("restoreOnLaunch") == Some(&Value::Bool(false)),
            },
            layout: parse(data, "layout").unwrap_or(d.layout),
            pin_style: parse(data, "pinStyle").unwrap_or(d.pin_style),
            site_permissions: legacy_site_permissions(get("sitePermissions")),
            pinned_extensions: match get("pinnedExtensions") {
                Some(Value::Array(ids)) => {
                    let mut out: Vec<String> = Vec::new();
                    for id in ids.iter().filter_map(Value::as_str).filter(|id| !id.is_empty()) {
                        if !out.iter().any(|o| o == id) {
                            out.push(id.to_owned());
                        }
                    }
                    out
                }
                _ => Vec::new(),
            },
            download_dir: string("downloadDir", d.download_dir),
            ask_where_to_save: get("askWhereToSave") == Some(&Value::Bool(true)),
        }
    }

    /// Reads a 0.5.11 `settings.json` (`{"version": 1, "data": {..}}`); `None` when there is no
    /// readable file.
    pub fn read_legacy(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        let doc: Value = serde_json::from_str(&text).ok()?;
        Some(Self::from_legacy(doc.get("data").unwrap_or(&Value::Null)))
    }

    pub fn engine(&self) -> &'static SearchEngine {
        engine_of(self.search_engine)
    }
}

fn parse<T: serde::de::DeserializeOwned>(data: &Value, key: &str) -> Option<T> {
    serde_json::from_value(data.get(key)?.clone()).ok()
}

/// Both levels are checked: a file hand-edited into the wrong shape must not decide what a site
/// may do.
fn legacy_site_permissions(saved: Option<&Value>) -> SitePermissions {
    let mut out = SitePermissions::new();
    let Some(Value::Object(origins)) = saved else { return out };
    for (origin, types) in origins {
        let Value::Object(types) = types else { continue };
        let kept: BTreeMap<String, PermissionDecision> = types
            .iter()
            .filter_map(|(kind, decision)| {
                serde_json::from_value::<PermissionDecision>(decision.clone()).ok().map(|d| (kind.clone(), d))
            })
            .collect();
        if !kept.is_empty() {
            out.insert(origin.clone(), kept);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn defaults_match_the_old_settings_json() {
        let d = LynkSettings::default();
        assert_eq!(d.search_engine, SearchEngineId::Duckduckgo);
        assert_eq!(d.homepage, "");
        assert!(!d.fresh_window);
        assert_eq!(d.layout, Layout::Sidebar);
        assert_eq!(d.pin_style, PinStyle::Icons);
        assert!(d.site_permissions.is_empty() && d.pinned_extensions.is_empty());
        assert_eq!(d.download_dir, "");
        assert!(!d.ask_where_to_save);
        let keys: Vec<String> = serde_json::to_value(&d).unwrap().as_object().unwrap().keys().cloned().collect();
        let mut want = vec![
            "searchEngine",
            "homepage",
            "freshWindow",
            "layout",
            "pinStyle",
            "sitePermissions",
            "pinnedExtensions",
            "downloadDir",
            "askWhereToSave",
        ];
        want.sort_unstable();
        let mut keys = keys;
        keys.sort();
        assert_eq!(keys, want);
    }

    #[test]
    fn reopen_on_launch_off_becomes_a_fresh_window() {
        assert!(LynkSettings::from_legacy(&json!({ "restoreOnLaunch": false })).fresh_window);
        assert!(!LynkSettings::from_legacy(&json!({ "restoreOnLaunch": true })).fresh_window);
        assert!(!LynkSettings::from_legacy(&json!({ "freshWindow": false, "restoreOnLaunch": false })).fresh_window);
        assert!(!LynkSettings::from_legacy(&json!({})).fresh_window);
    }

    #[test]
    fn malformed_values_fall_back_to_defaults() {
        let s = LynkSettings::from_legacy(&json!({
            "searchEngine": "altavista",
            "homepage": 3,
            "layout": "ribbon",
            "pinStyle": "letters",
            "pinnedExtensions": ["a", "", 7, "a", "b"],
            "sitePermissions": {
                "https://a.test": { "camera": "allow", "microphone": "maybe" },
                "https://b.test": "allow",
                "https://c.test": { "camera": 1 }
            },
            "downloadDir": null,
            "askWhereToSave": "yes"
        }));
        assert_eq!(s.search_engine, SearchEngineId::Duckduckgo);
        assert_eq!(s.homepage, "");
        assert_eq!(s.layout, Layout::Sidebar);
        assert_eq!(s.pin_style, PinStyle::Letters);
        assert_eq!(s.pinned_extensions, ["a", "b"]);
        assert_eq!(s.site_permissions.len(), 1);
        assert_eq!(s.site_permissions["https://a.test"].len(), 1);
        assert_eq!(s.download_dir, "");
        assert!(!s.ask_where_to_save);
        assert_eq!(LynkSettings::from_legacy(&Value::Null), LynkSettings::default());
    }

    #[test]
    fn a_legacy_file_and_a_config_toml_read_the_same_keys() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join("settings.json");
        std::fs::write(&legacy, r#"{"version":1,"data":{"searchEngine":"google","layout":"compact"}}"#).unwrap();
        let s = LynkSettings::read_legacy(&legacy).unwrap();
        assert_eq!(s.engine().name, "Google");
        assert_eq!(s.layout, Layout::Compact);

        std::fs::write(dir.path().join("config.toml"), "searchEngine = \"bing\"\npinStyle = \"letters\"\n").unwrap();
        let options = nd_settings::Options::new("lynk").paths(nd_settings::Paths::in_dir(dir.path())).env(Vec::new());
        let handle = nd_settings::SettingsHandle::<LynkSettings>::load(options).unwrap();
        assert_eq!(handle.get().search_engine, SearchEngineId::Bing);
        assert_eq!(handle.get().pin_style, PinStyle::Letters);
        assert_eq!(handle.get().homepage, "");
    }
}
