//! The built-in blocker's state (`blocking.json` and `adblock/`, the 0.5.11 layout): which uBO
//! release is in use, when each list was fetched, the sites it is off for and the elements the
//! user hid. On by default; switched off per site, and an element hidden on a site stays hidden
//! as a `site##selector` rule.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use adblock::Engine;
use adblock::lists::{FilterSet, ParseOptions};
use adblock::resources::Resource;
use nd_settings::Store;
use serde::{Deserialize, Serialize};

use crate::ublock::{self, FilterList, Release, write_replacing};

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;
pub const WEEK_MS: i64 = 7 * DAY_MS;

/// One thing the user took off a site: the selector the rule is made of, and words for the list
/// of what is hidden.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Veil {
    pub selector: String,
    pub label: String,
    pub note: String,
    pub date: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BlockingState {
    /// The uBO release in use; a newer one replaces it once downloaded and verified.
    pub release: Release,
    /// When each list was last fetched, by uBO asset key.
    pub fetched: BTreeMap<String, i64>,
    pub last_release_check: i64,
    /// Sites (hostnames without `www.`) blocking is off for.
    pub off_sites: Vec<String>,
    pub hidden: BTreeMap<String, Vec<Veil>>,
}

impl Default for BlockingState {
    fn default() -> Self {
        Self {
            release: ublock::pinned(),
            fetched: BTreeMap::new(),
            last_release_check: 0,
            off_sites: Vec::new(),
            hidden: BTreeMap::new(),
        }
    }
}

pub fn open(dir: impl Into<PathBuf>) -> Store<BlockingState> {
    Store::open(dir.into().join("blocking.json"), 1, None)
}

/// `<profile>/adblock`, with its `lists/` folder made.
pub fn adblock_dir(profile: &Path) -> std::io::Result<PathBuf> {
    let dir = profile.join("adblock");
    std::fs::create_dir_all(dir.join("lists"))?;
    Ok(dir)
}

/// The name a per-site switch and a hidden element are kept under.
pub fn site_of(url: &str) -> String {
    if !(url.starts_with("http:") || url.starts_with("https:")) {
        return String::new();
    }
    match url::Url::parse(url) {
        Ok(u) => {
            let host = u.host_str().unwrap_or("").to_lowercase();
            host.strip_prefix("www.").map_or(host.clone(), str::to_owned)
        }
        Err(_) => String::new(),
    }
}

impl BlockingState {
    pub fn blocking_on(&self, site: &str) -> bool {
        !self.off_sites.iter().any(|s| s == site)
    }

    /// Whether requests from a page at `first_party` are filtered.
    pub fn filters(&self, first_party: &str) -> bool {
        self.blocking_on(&site_of(first_party))
    }

    pub fn set_blocking_on(&mut self, site: &str, on: bool) {
        if site.is_empty() {
            return;
        }
        self.off_sites.retain(|s| s != site);
        if !on {
            self.off_sites.push(site.to_owned());
        }
    }

    /// `false` when nothing changed: no site, or that selector is already hidden there.
    pub fn hide(&mut self, site: &str, selector: &str, label: &str, note: &str, date: i64) -> bool {
        if site.is_empty() {
            return false;
        }
        let list = self.hidden.entry(site.to_owned()).or_default();
        if list.iter().any(|v| v.selector == selector) {
            return false;
        }
        list.push(Veil { selector: selector.into(), label: label.into(), note: note.into(), date });
        true
    }

    /// Shows one hidden element again, or every one on the site when `selector` is `None`.
    pub fn restore_hidden(&mut self, site: &str, selector: Option<&str>) {
        if let (Some(selector), Some(list)) = (selector, self.hidden.get_mut(site)) {
            list.retain(|v| v.selector != selector);
            if !list.is_empty() {
                return;
            }
        }
        self.hidden.remove(site);
    }

    pub fn hidden_on(&self, site: &str) -> &[Veil] {
        self.hidden.get(site).map_or(&[], Vec::as_slice)
    }

    /// The rules the user made: every hidden element as `site##selector`.
    pub fn user_rules(&self) -> String {
        let lines: Vec<String> = self
            .hidden
            .iter()
            .flat_map(|(site, veils)| veils.iter().map(move |v| format!("{site}##{}", v.selector)))
            .collect();
        lines.join("\n")
    }

    pub fn list_due(&self, key: &str, now: i64, force: bool) -> bool {
        force || now - self.fetched.get(key).copied().unwrap_or(0) >= DAY_MS
    }

    pub fn release_check_due(&self, now: i64, force: bool) -> bool {
        force || now - self.last_release_check > WEEK_MS
    }

    /// When the lists in use were last fetched, for the command bar's subtitle; `None` before the
    /// first refresh, when the release's own snapshots are in use.
    pub fn lists_updated_at(&self) -> Option<i64> {
        self.fetched.values().copied().max()
    }
}

/// A captive portal, an error page or a cut-off answer is not a filter list.
pub fn looks_like_a_list(text: &str) -> bool {
    let head = text.trim_start().get(..15).unwrap_or(text.trim_start()).to_lowercase();
    text.len() >= 256 && !head.starts_with("<!doctype") && !head.starts_with("<html")
}

/// The list file in use: the last refresh when there was one, the release's snapshot otherwise.
pub fn list_path(adblock_dir: &Path, list: &FilterList) -> PathBuf {
    let fresh = adblock_dir.join("lists").join(format!("{}.txt", list.key));
    if fresh.exists() { fresh } else { list.bundled.clone() }
}

/// Daily refresh from the addresses uBO itself uses, mirrors in order. `fetch` returns the body
/// of a successful answer. A list is replaced only by a whole list: a page that does not look
/// like one moves on to the next mirror, and the new file is written beside the old and renamed
/// over it. Returns how many lists changed.
pub fn refresh_lists(
    state: &mut BlockingState,
    adblock_dir: &Path,
    lists: &[FilterList],
    now: i64,
    force: bool,
    mut fetch: impl FnMut(&str) -> Option<String>,
) -> usize {
    let mut changed = 0;
    for list in lists {
        if !state.list_due(&list.key, now, force) {
            continue;
        }
        for url in &list.remote {
            let Some(text) = fetch(url).filter(|t| looks_like_a_list(t)) else { continue };
            let out = adblock_dir.join("lists").join(format!("{}.txt", list.key));
            state.fetched.insert(list.key.clone(), now);
            if std::fs::read_to_string(&out).is_ok_and(|old| old == text) {
                break;
            }
            if write_replacing(&out, &text).is_ok() {
                changed += 1;
                break;
            }
        }
    }
    changed
}

/// One list from the network with a 30 s limit, `None` for anything but a 200. Blocking.
pub fn fetch_list(url: &str) -> Option<String> {
    let agent: ureq::Agent =
        ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(30))).build().into();
    let mut response = agent.get(url).call().ok()?;
    response.body_mut().with_config().limit(32 * 1024 * 1024).read_to_string().ok()
}

/// The weekly release check. With `latest` newer than the release in use, it is fetched,
/// verified and extracted, then becomes the release, and the older ones go. Returns the new
/// release's folder when it moved.
pub fn update_release(
    state: &mut BlockingState,
    adblock_dir: &Path,
    now: i64,
    force: bool,
    latest: impl FnOnce() -> Option<Release>,
    fetch_zip: impl FnOnce(&str) -> Result<Vec<u8>, String>,
) -> Result<Option<PathBuf>, String> {
    if !state.release_check_due(now, force) {
        return Ok(None);
    }
    state.last_release_check = now;
    let Some(latest) = latest() else { return Ok(None) };
    if ublock::compare_versions(&latest.version, &state.release.version).is_le() {
        return Ok(None);
    }
    let dir = ublock::ensure_release(adblock_dir, &latest, fetch_zip)?;
    ublock::drop_other_releases(adblock_dir, &latest.version);
    state.release = latest;
    Ok(Some(dir))
}

/// The release folder to run on: the one in use, else the pinned one when the newer release
/// can no longer be fetched (the state moves back to it).
pub fn prepare_release(
    state: &mut BlockingState,
    adblock_dir: &Path,
    mut fetch_zip: impl FnMut(&str) -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    match ublock::ensure_release(adblock_dir, &state.release, &mut fetch_zip) {
        Ok(dir) => Ok(dir),
        Err(_) if state.release != ublock::pinned() => {
            state.release = ublock::pinned();
            ublock::ensure_release(adblock_dir, &state.release, fetch_zip)
        }
        Err(e) => Err(e),
    }
}

/// Compiles the lists in use with the release's resources.
pub fn build_engine(adblock_dir: &Path, lists: &[FilterList], resources: Vec<Resource>) -> Engine {
    let mut set = FilterSet::new(false);
    for list in lists {
        if let Ok(text) = std::fs::read_to_string(list_path(adblock_dir, list)) {
            set.add_filter_list(text, ParseOptions { format: list.format(), ..ParseOptions::default() });
        }
    }
    let mut engine = Engine::new_with_filter_set(set);
    engine.use_resources(resources);
    engine
}

/// The user's own rules as an engine of their own, so hiding an element never recompiles the
/// lists. Its cosmetic answers are added to the main engine's.
pub fn user_engine(state: &BlockingState) -> Engine {
    let mut set = FilterSet::new(false);
    set.add_filter_list(state.user_rules(), ParseOptions::default());
    Engine::new_with_filter_set(set)
}

/// The cache key for the compiled engine: the release and every list file it was built from.
pub fn engine_key(adblock_dir: &Path, version: &str, lists: &[FilterList]) -> String {
    let mut paths: Vec<PathBuf> = lists.iter().map(|l| list_path(adblock_dir, l)).collect();
    paths.push(ublock::resources_path(adblock_dir, version));
    ublock::lists_key(version, &paths)
}

/// Writes the compiled engine to `engine.bin`, its key first on its own line.
pub fn save_engine(adblock_dir: &Path, key: &str, engine: &Engine) -> std::io::Result<()> {
    let mut bytes = format!("{key}\n").into_bytes();
    bytes.extend(engine.serialize());
    let path = adblock_dir.join("engine.bin");
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, &path)
}

/// The cached engine when it was built from exactly these inputs. Slow enough to keep off the
/// main thread.
pub fn restore_engine(adblock_dir: &Path, key: &str, resources: Vec<Resource>) -> Option<Engine> {
    let bytes = std::fs::read(adblock_dir.join("engine.bin")).ok()?;
    let body = bytes.strip_prefix(format!("{key}\n").as_bytes())?;
    let mut engine = Engine::default();
    engine.deserialize(body).ok()?;
    engine.use_resources(resources);
    Some(engine)
}

#[cfg(test)]
mod tests {
    use adblock::request::Request;

    use super::*;
    use crate::ublock::tests::{FILTERS, fixture_release, release_zip};
    use crate::ublock::{default_lists, load_resources, sha256_hex};

    fn list_text(marker: &str) -> String {
        format!("! Title: {marker}\n||{marker}.test^\n{}", "! padding\n".repeat(40))
    }

    #[test]
    fn the_per_site_switch_and_hidden_elements() {
        let mut s = BlockingState::default();
        assert_eq!(s.release, ublock::pinned());
        assert_eq!(site_of("https://WWW.Example.com/a"), "example.com");
        assert_eq!(site_of("chrome://settings"), "");
        assert!(s.filters("https://www.example.com/"));
        s.set_blocking_on("example.com", false);
        s.set_blocking_on("example.com", false);
        assert_eq!(s.off_sites, ["example.com"]);
        assert!(!s.filters("https://example.com/x"));
        s.set_blocking_on("example.com", true);
        assert!(s.blocking_on("example.com"));
        s.set_blocking_on("", false);
        assert!(s.off_sites.is_empty());

        assert!(s.hide("a.test", ".ad", "Ad", "", 1));
        assert!(!s.hide("a.test", ".ad", "Ad", "", 2));
        assert!(s.hide("a.test", "#promo", "Promo", "", 3));
        assert!(s.hide("b.test", "div.x", "X", "", 4));
        assert!(!s.hide("", ".y", "", "", 5));
        assert_eq!(s.user_rules(), "a.test##.ad\na.test###promo\nb.test##div.x");
        s.restore_hidden("a.test", Some(".ad"));
        assert_eq!(s.hidden_on("a.test").len(), 1);
        s.restore_hidden("a.test", Some("#promo"));
        assert!(!s.hidden.contains_key("a.test"));
        s.restore_hidden("b.test", None);
        assert!(s.hidden.is_empty());
    }

    #[test]
    fn user_rules_hide_elements_on_their_site_only() {
        let mut s = BlockingState::default();
        s.hide("a.test", ".ad", "Ad", "", 1);
        let engine = user_engine(&s);
        assert!(engine.url_cosmetic_resources("https://a.test/page").hide_selectors.contains(".ad"));
        assert!(engine.url_cosmetic_resources("https://b.test/page").hide_selectors.is_empty());
    }

    #[test]
    fn a_partial_or_captive_portal_download_never_replaces_a_list() {
        let root = tempfile::tempdir().unwrap();
        let dir = adblock_dir(root.path()).unwrap();
        let (_, release) = fixture_release(&dir);
        let lists: Vec<FilterList> =
            default_lists(&release).unwrap().into_iter().filter(|l| l.key == "ublock-filters").collect();
        let mut s = BlockingState::default();

        let portal = format!("<!DOCTYPE html><html><body>Sign in to the Wi-Fi{}</body></html>", " ".repeat(300));
        let n = refresh_lists(&mut s, &dir, &lists, DAY_MS, false, |url| match url {
            "https://cdn.test/filters.txt" => Some(portal.clone()),
            _ => Some("||cut.test^".into()),
        });
        assert_eq!(n, 0);
        assert!(!s.fetched.contains_key("ublock-filters"));
        assert_eq!(list_path(&dir, &lists[0]), lists[0].bundled);

        let n = refresh_lists(&mut s, &dir, &lists, DAY_MS, false, |url| {
            (url == "https://raw.test/filters.txt").then(|| list_text("fresh"))
        });
        assert_eq!(n, 1);
        assert_eq!(s.fetched["ublock-filters"], DAY_MS);
        let fresh = dir.join("lists/ublock-filters.txt");
        assert_eq!(list_path(&dir, &lists[0]), fresh);
        assert!(!dir.join("lists/ublock-filters.txt.partial").exists());

        // Fetched less than a day ago: left alone unless forced. The same text changes nothing.
        let mut asked = 0;
        assert_eq!(
            refresh_lists(&mut s, &dir, &lists, DAY_MS + 1000, false, |_| {
                asked += 1;
                None
            }),
            0
        );
        assert_eq!(asked, 0);
        assert_eq!(refresh_lists(&mut s, &dir, &lists, DAY_MS + 1000, true, |_| Some(list_text("fresh"))), 0);
        assert_eq!(s.fetched["ublock-filters"], DAY_MS + 1000);
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), list_text("fresh"));
    }

    #[test]
    fn a_newer_release_is_taken_weekly_once_verified() {
        let root = tempfile::tempdir().unwrap();
        let dir = adblock_dir(root.path()).unwrap();
        let zip = release_zip();
        let latest = Release { version: "9.0.0".into(), sha256: sha256_hex(&zip) };
        std::fs::create_dir_all(dir.join("ubo/1.75.0")).unwrap();
        let mut s = BlockingState { last_release_check: 1, ..BlockingState::default() };

        let not_due = update_release(&mut s, &dir, 2, false, || panic!("not due"), |_| panic!("not due"));
        assert_eq!(not_due, Ok(None));

        let bad = Release { sha256: "f".repeat(64), ..latest.clone() };
        let z = zip.clone();
        assert!(update_release(&mut s, &dir, WEEK_MS + 2, false, || Some(bad), move |_| Ok(z)).is_err());
        assert_eq!(s.release, ublock::pinned());
        assert_eq!(s.last_release_check, WEEK_MS + 2);

        let z = zip.clone();
        let moved = update_release(&mut s, &dir, WEEK_MS + 3, true, || Some(latest.clone()), move |_| Ok(z)).unwrap();
        assert_eq!(moved, Some(dir.join("ubo/9.0.0")));
        assert_eq!(s.release, latest);
        assert!(!dir.join("ubo/1.75.0").exists());
        let older = Release { version: "1.0.0".into(), ..latest.clone() };
        assert_eq!(update_release(&mut s, &dir, WEEK_MS * 3, false, || Some(older), |_| panic!("older")), Ok(None));
    }

    #[test]
    fn a_release_that_cannot_be_fetched_falls_back_to_the_pinned_one() {
        let root = tempfile::tempdir().unwrap();
        let dir = adblock_dir(root.path()).unwrap();
        let mut s = BlockingState {
            release: Release { version: "9.9.9".into(), sha256: "0".repeat(64) },
            ..BlockingState::default()
        };
        let err = prepare_release(&mut s, &dir, |_| Err("offline".into())).unwrap_err();
        assert_eq!(err, "offline");
        assert_eq!(s.release, ublock::pinned());
    }

    #[test]
    fn the_engine_blocks_from_the_lists_and_comes_back_from_its_cache() {
        let root = tempfile::tempdir().unwrap();
        let dir = adblock_dir(root.path()).unwrap();
        let (release, release_dir) = fixture_release(&dir);
        let lists = default_lists(&release_dir).unwrap();
        let resources = || load_resources(&dir, &release_dir, &release.version);
        let engine = build_engine(&dir, &lists, resources());
        let check = |e: &Engine, url: &str| {
            e.check_network_request(&Request::new(url, "https://site.test/", "script", "GET").unwrap()).should_block()
        };
        assert!(check(&engine, "https://ads.example.com/a.js"));
        assert!(check(&engine, "https://tracker.test/t.js"));
        assert!(!check(&engine, "https://fanboy.test/f.js"));
        assert!(!check(&engine, "https://cdn.site.test/app.js"));
        assert!(FILTERS.contains("##.banner"));
        assert!(engine.url_cosmetic_resources("https://example.org/").hide_selectors.contains(".banner"));

        let key = engine_key(&dir, &release.version, &lists);
        save_engine(&dir, &key, &engine).unwrap();
        assert!(restore_engine(&dir, "other key", resources()).is_none());
        let restored = restore_engine(&dir, &key, resources()).unwrap();
        assert!(check(&restored, "https://ads.example.com/a.js"));

        std::fs::write(dir.join("lists/ublock-filters.txt"), list_text("changed")).unwrap();
        assert_ne!(engine_key(&dir, &release.version, &lists), key);
    }

    #[test]
    fn blocking_json_from_the_old_app_reads_back() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("blocking.json"),
            r#"{"version":1,"data":{"release":{"version":"1.75.0","sha256":"393cf95709d1074d4022970e9014e434395c53a822387f1e25f43be97cf4b582"},"fetched":{"easylist":1791299416563},"lastReleaseCheck":1791299416563,"offSites":["news.test"],"hidden":{"a.test":[{"selector":".ad","label":"Ad","note":"","date":1}]}}}"#,
        )
        .unwrap();
        let store = open(root.path());
        assert!(store.load_error().is_none());
        let s = store.get();
        assert!(!s.blocking_on("news.test"));
        assert_eq!(s.user_rules(), "a.test##.ad");
        assert_eq!(s.lists_updated_at(), Some(1791299416563));
    }
}
