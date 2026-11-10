//! uBlock Origin's own release is the source of everything the blocker uses: its filter lists
//! (bundled snapshots, refreshed from the CDNs uBO itself fetches from), its scriptlets and its
//! `redirect=` resources. The pinned release is what a fresh profile downloads; a newer one is
//! picked up weekly, checked against the SHA-256 GitHub publishes for it.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

use adblock::lists::FilterFormat;
use adblock::resources::{MimeType, PermissionMask, Resource, ResourceType};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub sha256: String,
}

pub const PINNED_VERSION: &str = "1.75.0";
pub const PINNED_SHA256: &str = "393cf95709d1074d4022970e9014e434395c53a822387f1e25f43be97cf4b582";

pub fn pinned() -> Release {
    Release { version: PINNED_VERSION.into(), sha256: PINNED_SHA256.into() }
}

const LATEST: &str = "https://api.github.com/repos/gorhill/uBlock/releases/latest";

pub fn zip_url(version: &str) -> String {
    format!("https://github.com/gorhill/uBlock/releases/download/{version}/uBlock0_{version}.chromium.zip")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(timeout)).build().into()
}

/// The release zip from GitHub. Blocking; run it off the main thread.
pub fn download_zip(version: &str) -> Result<Vec<u8>, String> {
    let url = zip_url(version);
    let mut response = agent(Duration::from_secs(120)).get(&url).call().map_err(|e| format!("{url}: {e}"))?;
    response.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().map_err(|e| format!("{url}: {e}"))
}

/// Unpacks a release zip into `into`. Everything sits in one top folder
/// (`uBlock0.chromium/`), which is dropped; an entry that would land outside is refused rather
/// than half-extracted.
pub fn extract(bytes: &[u8], into: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("not a zip archive: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("zip entry {i}: {e}"))?;
        let Some(name) = entry.enclosed_name() else { return Err(format!("zip entry escapes: {}", entry.name())) };
        let inner: PathBuf = name.components().skip(1).collect();
        if entry.is_dir() || inner.as_os_str().is_empty() {
            continue;
        }
        let out = into.join(inner);
        std::fs::create_dir_all(out.parent().unwrap_or(into))
            .and_then(|()| std::fs::File::create(&out))
            .and_then(|mut file| std::io::copy(&mut entry, &mut file))
            .map_err(|e| format!("{}: {e}", out.display()))?;
    }
    Ok(())
}

/// `<root>/ubo/<version>/` holding the extracted chromium build, fetching it with `fetch` and
/// verifying it first when it is not there yet. A zip whose digest is not `release.sha256` is
/// never extracted.
pub fn ensure_release(
    root: &Path,
    release: &Release,
    fetch: impl FnOnce(&str) -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    let dir = root.join("ubo").join(&release.version);
    if dir.join("assets/assets.json").exists() {
        return Ok(dir);
    }
    let bytes = fetch(&release.version)?;
    let got = sha256_hex(&bytes);
    if got != release.sha256 {
        return Err(format!("uBlock Origin {}: sha256 {got}, expected {}", release.version, release.sha256));
    }
    let staging = root.join("ubo").join(format!("{}.partial", release.version));
    let _ = std::fs::remove_dir_all(&staging);
    extract(&bytes, &staging)?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&staging, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

/// The newest release and its zip's digest from GitHub's answer, or `None` when it publishes
/// no digest for it.
pub fn parse_latest_release(json: &str) -> Option<Release> {
    let release: Value = serde_json::from_str(json).ok()?;
    let version = release.get("tag_name")?.as_str()?.to_owned();
    let zip = release
        .get("assets")?
        .as_array()?
        .iter()
        .find(|a| a.get("name").and_then(Value::as_str).is_some_and(|n| n.ends_with(".chromium.zip")))?;
    let sha256 = zip.get("digest")?.as_str()?.strip_prefix("sha256:")?.to_owned();
    Some(Release { version, sha256 })
}

/// `None` when GitHub is out of reach. Blocking.
pub fn latest_release() -> Option<Release> {
    let mut response =
        agent(Duration::from_secs(30)).get(LATEST).header("accept", "application/vnd.github+json").call().ok()?;
    parse_latest_release(&response.body_mut().read_to_string().ok()?)
}

pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |v: &str| -> Vec<u64> {
        v.split('.')
            .map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0))
            .collect()
    };
    let (pa, pb) = (parts(a), parts(b));
    for i in 0..pa.len().max(pb.len()) {
        let d = pa.get(i).unwrap_or(&0).cmp(pb.get(i).unwrap_or(&0));
        if d.is_ne() {
            return d;
        }
    }
    std::cmp::Ordering::Equal
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilterList {
    /// uBO's asset key (`ublock-filters`, `easylist`).
    pub key: String,
    /// Where uBO itself fetches the list; the first that answers wins.
    pub remote: Vec<String>,
    /// The snapshot bundled in the release.
    pub bundled: PathBuf,
    pub hosts: bool,
}

impl FilterList {
    pub fn format(&self) -> FilterFormat {
        if self.hosts { FilterFormat::Hosts } else { FilterFormat::Standard }
    }
}

/// The lists uBO enables on a fresh install, read from the release's own `assets.json` so the
/// set follows uBO's defaults.
pub fn default_lists(release: &Path) -> Result<Vec<FilterList>, String> {
    let path = release.join("assets/assets.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let assets: serde_json::Map<String, Value> =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut lists = Vec::new();
    for (key, entry) in &assets {
        if entry.get("content").and_then(Value::as_str) != Some("filters")
            || entry.get("off") == Some(&Value::Bool(true))
        {
            continue;
        }
        let urls: Vec<&str> = match entry.get("contentURL") {
            Some(Value::String(u)) => vec![u.as_str()],
            Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
            _ => Vec::new(),
        };
        let Some(bundled) =
            urls.iter().filter(|u| u.starts_with("assets/")).map(|u| release.join(u)).find(|p| p.exists())
        else {
            continue;
        };
        let mut remote: Vec<String> = entry
            .get("cdnURLs")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        remote
            .extend(urls.iter().filter(|u| u.starts_with("http:") || u.starts_with("https:")).map(|u| (*u).to_owned()));
        // Peter Lowe's list is fetched in hosts-file form, like uBO does.
        lists.push(FilterList { key: key.clone(), remote, bundled, hosts: key.starts_with("plowe") });
    }
    Ok(lists)
}

/// adblock-rust's resources for one release. The redirect resources come from the release;
/// scriptlets are functions in uBO's ES modules, which only a JavaScript engine can turn into
/// sources, so they come from `resources-<version>.json` (the 0.5.11 app wrote one per release,
/// and the app writes one with [`save_resources`] once a page has evaluated them). Entries of
/// the cache file the release also provides are taken from the release.
pub fn load_resources(adblock_dir: &Path, release_dir: &Path, version: &str) -> Vec<Resource> {
    let war = release_dir.join("web_accessible_resources");
    let map = release_dir.join("js/redirect-resources.js");
    let mut out: Vec<Resource> = if war.is_dir() && map.is_file() {
        // The assembler panics on a file it cannot parse; a broken release still blocks.
        std::panic::catch_unwind(|| {
            adblock::resources::resource_assembler::assemble_web_accessible_resources(&war, &map)
        })
        .unwrap_or_default()
    } else {
        Vec::new()
    };
    let cached: Vec<Resource> = std::fs::read_to_string(resources_path(adblock_dir, version))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    for r in cached {
        if !out.iter().any(|o| o.name == r.name) {
            out.push(r);
        }
    }
    out
}

/// One entry of uBO's `builtinScriptlets` as a page evaluating the release's
/// `js/resources/scriptlets.js` reports it, `source` being `fn.toString()`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scriptlet {
    pub name: String,
    pub aliases: Vec<String>,
    pub source: String,
    pub dependencies: Vec<String>,
}

/// The scriptlets as adblock-rust resources, the shape Brave builds its own in. uBO's `*.fn`
/// entries are helpers other scriptlets depend on, never injected by name; a scriptlet whose
/// source starts with `function name(` is called with the filter's arguments.
pub fn scriptlet_resources(scriptlets: &[Scriptlet]) -> Vec<Resource> {
    scriptlets
        .iter()
        .map(|s| Resource {
            name: s.name.clone(),
            aliases: s.aliases.clone(),
            kind: ResourceType::Mime(if s.name.ends_with(".fn") {
                MimeType::FnJavascript
            } else {
                MimeType::ApplicationJavascript
            }),
            content: base64::engine::general_purpose::STANDARD.encode(&s.source),
            dependencies: s.dependencies.clone(),
            permission: PermissionMask::default(),
        })
        .collect()
}

pub fn resources_path(adblock_dir: &Path, version: &str) -> PathBuf {
    adblock_dir.join(format!("resources-{version}.json"))
}

pub fn save_resources(adblock_dir: &Path, version: &str, resources: &[Resource]) -> std::io::Result<()> {
    let text = serde_json::to_string(resources).map_err(std::io::Error::other)?;
    write_replacing(&resources_path(adblock_dir, version), &text)
}

/// Writes through a `.partial` file and a rename, so a reader never sees half a file.
pub(crate) fn write_replacing(path: &Path, text: &str) -> std::io::Result<()> {
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)
}

/// A key that changes whenever any input to the compiled engine does.
pub fn lists_key(version: &str, paths: &[PathBuf]) -> String {
    let mut parts = vec![version.to_owned()];
    for p in paths {
        let meta = std::fs::metadata(p).ok();
        let size = meta.as_ref().map_or(0, std::fs::Metadata::len);
        let mtime = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        parts.push(format!("{}:{size}:{mtime}", p.display()));
    }
    sha256_hex(parts.join("\n").as_bytes())
}

pub fn drop_other_releases(root: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(root.join("ubo")) else { return };
    for entry in entries.flatten() {
        if entry.file_name() != keep {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::Write;

    use super::*;

    pub(crate) const FILTERS: &str = "||ads.example.com^\nexample.org##.banner\n";

    /// A release zip with two default lists, one switched off, and one redirect resource.
    pub(crate) fn release_zip() -> Vec<u8> {
        let assets = serde_json::json!({
            "assets.json": { "content": "internal", "contentURL": "assets/assets.json" },
            "ublock-filters": {
                "content": "filters",
                "contentURL": ["https://raw.test/filters.txt", "assets/ublock/filters.txt"],
                "cdnURLs": ["https://cdn.test/filters.txt"]
            },
            "plowe-0": { "content": "filters", "contentURL": ["https://pgl.test/hosts", "assets/thirdparties/pgl.txt"] },
            "fanboy-off": { "content": "filters", "off": true, "contentURL": "assets/thirdparties/fanboy.txt" }
        });
        let files: [(&str, String); 6] = [
            ("uBlock0.chromium/assets/assets.json", assets.to_string()),
            ("uBlock0.chromium/assets/ublock/filters.txt", FILTERS.into()),
            ("uBlock0.chromium/assets/thirdparties/pgl.txt", "127.0.0.1 tracker.test\n".into()),
            ("uBlock0.chromium/assets/thirdparties/fanboy.txt", "||fanboy.test^\n".into()),
            ("uBlock0.chromium/web_accessible_resources/noop.js", "(function() {})();\n".into()),
            (
                "uBlock0.chromium/js/redirect-resources.js",
                "export default new Map([\n    [ 'noop.js', {\n        alias: 'noopjs',\n    } ],\n]);\n".into(),
            ),
        ];
        let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, body) in files {
            out.start_file(name, options).unwrap();
            out.write_all(body.as_bytes()).unwrap();
        }
        out.finish().unwrap().into_inner()
    }

    pub(crate) fn fixture_release(root: &Path) -> (Release, PathBuf) {
        let zip = release_zip();
        let release = Release { version: "1.80.0".into(), sha256: sha256_hex(&zip) };
        let dir = ensure_release(root, &release, |_| Ok(zip)).unwrap();
        (release, dir)
    }

    #[test]
    fn a_release_with_the_wrong_hash_is_never_extracted() {
        let root = tempfile::tempdir().unwrap();
        let wrong = Release { version: "1.80.0".into(), sha256: "0".repeat(64) };
        let err = ensure_release(root.path(), &wrong, |_| Ok(release_zip())).unwrap_err();
        assert!(err.contains("sha256"), "{err}");
        assert!(!root.path().join("ubo/1.80.0").exists());
        assert!(!root.path().join("ubo/1.80.0.partial").exists());
    }

    #[test]
    fn a_verified_release_is_extracted_once_without_its_top_folder() {
        let root = tempfile::tempdir().unwrap();
        let (release, dir) = fixture_release(root.path());
        assert_eq!(dir, root.path().join("ubo/1.80.0"));
        assert!(dir.join("assets/ublock/filters.txt").exists());
        let again = ensure_release(root.path(), &release, |_| Err("no second download".into())).unwrap();
        assert_eq!(again, dir);
        assert_eq!(PINNED_SHA256.len(), 64);
    }

    #[test]
    fn default_lists_follow_the_releases_assets() {
        let root = tempfile::tempdir().unwrap();
        let (_, dir) = fixture_release(root.path());
        let mut lists = default_lists(&dir).unwrap();
        lists.sort_by(|a, b| a.key.cmp(&b.key));
        let keys: Vec<&str> = lists.iter().map(|l| l.key.as_str()).collect();
        assert_eq!(keys, ["plowe-0", "ublock-filters"]);
        assert!(lists[0].hosts && !lists[1].hosts);
        assert_eq!(lists[1].remote, ["https://cdn.test/filters.txt", "https://raw.test/filters.txt"]);
        assert_eq!(lists[1].bundled, dir.join("assets/ublock/filters.txt"));
        let resources = load_resources(root.path(), &dir, "1.80.0");
        assert!(resources.iter().any(|r| r.name == "noop.js" && r.aliases == ["noopjs"]));
    }

    #[test]
    fn evaluated_scriptlets_inject_with_their_helpers() {
        let scriptlets = [
            Scriptlet {
                name: "mark-helper.fn".into(),
                source: "function markHelper(v) { window.marked = v; }".into(),
                ..Scriptlet::default()
            },
            Scriptlet {
                name: "mark-page.js".into(),
                aliases: vec!["mark.js".into()],
                source: "function markPage(v) { markHelper(v); }".into(),
                dependencies: vec!["mark-helper.fn".into()],
            },
        ];
        let resources = scriptlet_resources(&scriptlets);
        assert_eq!(resources[0].kind, ResourceType::Mime(MimeType::FnJavascript));
        assert_eq!(resources[1].kind, ResourceType::Mime(MimeType::ApplicationJavascript));
        let mut engine = adblock::Engine::new_with_list_text("example.org##+js(mark, 7)");
        engine.use_resources(resources);
        let script = engine.url_cosmetic_resources("https://example.org/").injected_script;
        assert!(script.contains("function markHelper(v)"), "{script}");
        assert!(script.contains("markPage"), "{script}");
    }

    #[test]
    fn releases_compare_by_number_and_the_latest_needs_a_digest() {
        use std::cmp::Ordering;
        assert_eq!(compare_versions("1.75.0", "1.9.1"), Ordering::Greater);
        assert_eq!(compare_versions("1.75", "1.75.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.75.0", "1.75.1"), Ordering::Less);
        let body = serde_json::json!({
            "tag_name": "1.76.0",
            "assets": [{ "name": "uBlock0_1.76.0.firefox.signed.xpi" }, { "name": "uBlock0_1.76.0.chromium.zip", "digest": "sha256:abc" }]
        });
        assert_eq!(
            parse_latest_release(&body.to_string()),
            Some(Release { version: "1.76.0".into(), sha256: "abc".into() })
        );
        let no_digest =
            serde_json::json!({ "tag_name": "1.76.0", "assets": [{ "name": "uBlock0_1.76.0.chromium.zip" }] });
        assert_eq!(parse_latest_release(&no_digest.to_string()), None);
    }
}
