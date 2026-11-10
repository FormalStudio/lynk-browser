//! The downloads list (`downloads.json`), the names files land under, and the line under each
//! row. Chromium runs every transfer; this only decides where it goes and keeps the record.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use nd_settings::Store;
use serde::{Deserialize, Serialize};

use crate::url::{file_name_from_url, host_of};
use crate::{lenient_items, now_ms, radix36};

/// `Pending` is asked for but not answered (a save panel is up); `Dangerous` finished into an
/// unconfirmed file that waits for Keep or Discard. The rest are the engine's own states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadState {
    Pending,
    #[default]
    InProgress,
    Paused,
    Interrupted,
    Complete,
    Cancelled,
    Dangerous,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DownloadItem {
    /// The app's own id, stable across restarts.
    pub id: String,
    /// The engine's id, per run, while the engine still has the download.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_id: Option<String>,
    pub url: String,
    pub name: String,
    /// Where the file ends up.
    pub path: PathBuf,
    /// Where a dangerous file sits until it is kept: Chrome's "Unconfirmed" name, so nothing
    /// opens it by accident.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub part_path: Option<PathBuf>,
    pub state: DownloadState,
    pub received: u64,
    /// 0 while the size is unknown.
    pub total: u64,
    /// Bytes per second, as the engine last reported it.
    pub speed: u64,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    /// Why an interrupted download stopped, in the engine's words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl DownloadItem {
    pub fn is_active(&self) -> bool {
        matches!(self.state, DownloadState::Pending | DownloadState::InProgress | DownloadState::Paused)
    }

    /// Still expects to create its file, or holds an unconfirmed one.
    fn holds_path(&self) -> bool {
        self.is_active() || self.state == DownloadState::Dangerous
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadsState {
    #[serde(deserialize_with = "lenient_items")]
    pub items: Vec<DownloadItem>,
}

/// Oldest entries past this are dropped from the list, never from the disk.
const KEPT: usize = 200;

/// `downloads.json` in `dir`, with what a run that ended mid-download left behind made
/// retryable.
pub fn open(dir: impl Into<PathBuf>) -> Store<DownloadsState> {
    let store = Store::<DownloadsState>::open(dir.into().join("downloads.json"), 1, None);
    let current = store.get();
    let normalized = normalize(&current);
    if normalized != *current {
        store.update(|s| *s = normalized);
    }
    store
}

/// The engine is gone after a restart, so an unfinished download can only be retried.
pub fn normalize(state: &DownloadsState) -> DownloadsState {
    let items = state
        .items
        .iter()
        .filter(|d| !d.id.is_empty())
        .map(|d| {
            let mut d = DownloadItem { engine_id: None, ..d.clone() };
            if d.is_active() {
                d.state = DownloadState::Interrupted;
                d.speed = 0;
                d.reason = Some("shutdown".into());
            }
            d
        })
        .collect();
    DownloadsState { items }
}

impl DownloadsState {
    pub fn add(&mut self, item: DownloadItem) {
        self.items.retain(|d| d.id != item.id);
        self.items.insert(0, item);
        self.items.truncate(KEPT);
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut DownloadItem> {
        self.items.iter_mut().find(|d| d.id == id)
    }

    pub fn remove(&mut self, id: &str) {
        self.items.retain(|d| d.id != id);
    }

    /// Clear keeps whatever is still running or waiting on the user.
    pub fn clear(&mut self) {
        self.items.retain(DownloadItem::holds_path);
    }

    /// Paths the list still expects to create, leaving out `except`.
    pub fn reserved_paths(&self, except: Option<&str>) -> HashSet<PathBuf> {
        self.items.iter().filter(|d| d.holds_path() && Some(d.id.as_str()) != except).map(|d| d.path.clone()).collect()
    }

    /// Keep: the unconfirmed file takes its real name, or the next free one if something took
    /// that name meanwhile.
    pub fn keep_dangerous(&mut self, id: &str) -> std::io::Result<()> {
        let taken = self.reserved_paths(Some(id));
        let Some(d) = self.get_mut(id) else { return Ok(()) };
        let Some(part) = d.part_path.clone() else { return Ok(()) };
        let path = if d.path.exists() {
            let dir = d.path.parent().map(Path::to_path_buf).unwrap_or_default();
            unique_path(&dir, &d.name, &taken)
        } else {
            d.path.clone()
        };
        std::fs::rename(&part, &path)?;
        d.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        d.path = path;
        d.part_path = None;
        d.state = DownloadState::Complete;
        Ok(())
    }

    /// Discard: the unconfirmed file goes, and so does the row.
    pub fn discard_dangerous(&mut self, id: &str) {
        if let Some(part) = self.items.iter().find(|d| d.id == id).and_then(|d| d.part_path.clone()) {
            let _ = std::fs::remove_file(part);
        }
        self.remove(id);
    }
}

/// The folder picked in Settings, else the system's Downloads folder.
pub fn download_dir(chosen: &str) -> PathBuf {
    if chosen.is_empty() { dirs::home_dir().unwrap_or_default().join("Downloads") } else { PathBuf::from(chosen) }
}

/// Node's `path.extname`: the last dot and what follows, empty for a leading dot only.
fn extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if !name[..i].chars().all(|c| c == '.') => &name[i..],
        _ => "",
    }
}

fn ext_lower(name: &str) -> String {
    extension(name).trim_start_matches('.').to_lowercase()
}

/// A name nothing on disk and no running download has taken. The engine only creates the
/// target when it finishes, so the running ones have to be counted, or two downloads of one
/// file would race for it.
pub fn unique_path(dir: &Path, name: &str, taken: &HashSet<PathBuf>) -> PathBuf {
    let ext = extension(name);
    let stem = &name[..name.len() - ext.len()];
    let mut candidate = dir.join(name);
    let mut n = 2;
    while candidate.exists() || taken.contains(&candidate) {
        candidate = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    candidate
}

/// File types that run code the moment they are opened. A `.dmg` or `.pkg` is left alone the
/// way Chrome leaves it: Gatekeeper and the installer already stand between it and the system,
/// and a warning on every app download teaches people to click through warnings.
pub fn dangerous_types(mac: bool) -> &'static [&'static str] {
    if mac {
        &[
            "jar",
            "jnlp",
            "app",
            "command",
            "tool",
            "terminal",
            "workflow",
            "action",
            "scpt",
            "applescript",
            "sh",
            "zsh",
            "bash",
        ]
    } else {
        &[
            "jar", "jnlp", "sh", "bash", "zsh", "csh", "ksh", "run", "desktop", "appimage", "py", "pl", "rb", "deb",
            "rpm",
        ]
    }
}

pub fn is_dangerous(name: &str, mac: bool) -> bool {
    dangerous_types(mac).contains(&ext_lower(name).as_str())
}

/// Chrome's name for a dangerous file until it is kept, six digits that need not be unique.
pub fn unconfirmed_name() -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    format!("Unconfirmed {}.crdownload", 100_000 + nanos % 900_000)
}

/// The name a download is saved under when nothing better is known. A name is a name, never a
/// path: a server can suggest `../x`.
pub fn download_name(url: &str, suggested: Option<&str>) -> String {
    let raw = suggested.filter(|s| !s.is_empty()).map_or_else(|| file_name_from_url(url), str::to_owned);
    let base = raw.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("");
    let name = base.trim_start_matches('.');
    if name.is_empty() { "download".into() } else { name.to_owned() }
}

const PAGE_EXTENSIONS: &[&str] = &["html", "htm", "xhtml", "shtml", "php", "asp", "aspx", "jsp", "cgi"];

/// Save Page As names a web page after its title, as Chrome does. A file that is not a page (a
/// PDF, an image) keeps its own name.
pub fn saved_page_name(title: &str, url: &str, suggested: Option<&str>) -> String {
    let own = download_name(url, suggested);
    let ext = ext_lower(&own);
    if !ext.is_empty() && !PAGE_EXTENSIONS.contains(&ext.as_str()) {
        return own;
    }
    let replaced: String =
        title.chars().map(|c| if "\\/:*?\"<>|".contains(c) || (c as u32) < 0x20 { '_' } else { c }).collect();
    let clean: String = replaced.trim().trim_start_matches('.').chars().take(200).collect();
    if !clean.is_empty() {
        format!("{clean}.html")
    } else if ext.is_empty() {
        format!("{own}.html")
    } else {
        own
    }
}

static SEQ: AtomicU64 = AtomicU64::new(0);

pub fn new_download_id() -> String {
    format!("d{}{}", radix36(now_ms() as u64), radix36(SEQ.fetch_add(1, Ordering::Relaxed)))
}

/// A long name cut in the middle, so the extension survives, for the popover. AppKit sizes a
/// truncating label from its whole text, so an uncut name would widen the popover.
pub fn short_name(name: &str, max: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= max {
        return name.to_owned();
    }
    let tail = 16.min(max / 3);
    let head: String = chars[..max - tail - 1].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{head}…{end}")
}

/// Decimal units, as Finder and Files report sizes.
pub fn format_bytes(n: u64) -> String {
    if n < 1000 {
        return format!("{n} {}", if n == 1 { "byte" } else { "bytes" });
    }
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1000.0 && u < UNITS.len() - 1 {
        v /= 1000.0;
        u += 1;
    }
    if v >= 100.0 { format!("{} {}", v.round(), UNITS[u]) } else { format!("{v:.1} {}", UNITS[u]) }
}

pub fn format_remaining(seconds: f64) -> String {
    if seconds < 60.0 {
        return format!("{} s left", seconds.round().max(1.0));
    }
    if seconds < 3600.0 {
        return format!("{} min left", (seconds / 60.0).round());
    }
    let h = (seconds / 3600.0).floor();
    let m = ((seconds % 3600.0) / 60.0).round();
    if m > 0.0 { format!("{h} h {m} min left") } else { format!("{h} h left") }
}

fn reason_text(reason: Option<&str>) -> &'static str {
    match reason {
        Some("network") => "Network error",
        Some("server") => "Server problem",
        Some("file") => "Unable to save the file",
        Some("noSpace") => "Disk full",
        Some("shutdown") => "Stopped when the browser quit",
        Some("crash") => "Stopped unexpectedly",
        Some("cancelled") => "Cancelled",
        _ => "Failed",
    }
}

/// The one line under a download's name. `file_exists` is checked against the disk whenever
/// the row shows.
pub fn download_status(d: &DownloadItem, file_exists: bool) -> String {
    let size = if d.total > 0 {
        format!("{} of {}", format_bytes(d.received), format_bytes(d.total))
    } else {
        format_bytes(d.received)
    };
    match d.state {
        DownloadState::Pending => "Waiting for a location".into(),
        DownloadState::InProgress if d.speed == 0 => size,
        DownloadState::InProgress => {
            let rate = format!("{}/s", format_bytes(d.speed));
            if d.total == 0 {
                format!("{size} · {rate}")
            } else {
                let left = d.total.saturating_sub(d.received) as f64 / d.speed as f64;
                format!("{size} · {rate} · {}", format_remaining(left))
            }
        }
        DownloadState::Paused => format!("Paused · {size}"),
        DownloadState::Interrupted => format!("{} · {size}", reason_text(d.reason.as_deref())),
        DownloadState::Cancelled => "Cancelled".into(),
        DownloadState::Dangerous => "Can harm your computer".into(),
        DownloadState::Complete if !file_exists => "Moved or deleted".into(),
        DownloadState::Complete => {
            let bytes = format_bytes(if d.total > 0 { d.total } else { d.received });
            let host = host_of(&d.url);
            if host.is_empty() { bytes } else { format!("{bytes} · {host}") }
        }
    }
}

/// A generic icon by kind of file, the freedesktop name the backends map to their own.
pub fn file_icon(name: &str) -> &'static str {
    let ext = ext_lower(name);
    let is = |list: &[&str]| list.contains(&ext.as_str());
    if is(&["png", "jpg", "jpeg", "gif", "webp", "heic", "svg", "bmp", "tiff", "avif"]) {
        "image-x-generic-symbolic"
    } else if is(&["mp4", "mov", "mkv", "webm", "avi", "m4v"]) {
        "video-x-generic-symbolic"
    } else if is(&["mp3", "m4a", "wav", "flac", "ogg", "aac", "opus"]) {
        "audio-x-generic-symbolic"
    } else if is(&["zip", "gz", "tgz", "bz2", "xz", "7z", "rar", "tar", "zst", "dmg", "pkg", "deb", "rpm"]) {
        "package-x-generic-symbolic"
    } else {
        "text-x-generic-symbolic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> DownloadItem {
        DownloadItem {
            id: "d1".into(),
            url: "https://example.com/files/report.pdf".into(),
            name: "report.pdf".into(),
            path: "/tmp/report.pdf".into(),
            state: DownloadState::InProgress,
            received: 2_000_000,
            total: 10_000_000,
            speed: 1_000_000,
            ..DownloadItem::default()
        }
    }

    #[test]
    fn status_line_reads_size_speed_and_time_left() {
        let with = |f: fn(&mut DownloadItem)| {
            let mut d = base();
            f(&mut d);
            d
        };
        assert_eq!(download_status(&base(), false), "2.0 MB of 10.0 MB · 1.0 MB/s · 8 s left");
        assert_eq!(download_status(&with(|d| d.state = DownloadState::Paused), false), "Paused · 2.0 MB of 10.0 MB");
        let net = with(|d| {
            d.state = DownloadState::Interrupted;
            d.reason = Some("network".into());
        });
        assert_eq!(download_status(&net, false), "Network error · 2.0 MB of 10.0 MB");
        let done = with(|d| {
            d.state = DownloadState::Complete;
            d.received = 10_000_000;
        });
        assert_eq!(download_status(&done, true), "10.0 MB · example.com");
        assert_eq!(download_status(&with(|d| d.state = DownloadState::Complete), false), "Moved or deleted");
        assert_eq!(download_status(&with(|d| d.total = 0), false), "2.0 MB · 1.0 MB/s");
    }

    #[test]
    fn sizes_use_decimal_units() {
        assert_eq!(format_bytes(1), "1 byte");
        assert_eq!(format_bytes(999), "999 bytes");
        assert_eq!(format_bytes(1_500), "1.5 KB");
        assert_eq!(format_bytes(123_456_789), "123 MB");
        assert_eq!(format_remaining(3600.0 * 2.0 + 60.0 * 5.0), "2 h 5 min left");
        assert_eq!(format_remaining(0.2), "1 s left");
    }

    #[test]
    fn a_new_name_skips_files_on_disk_and_downloads_still_running() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path();
        std::fs::write(dir.join("a.txt"), "").unwrap();
        assert_eq!(unique_path(dir, "a.txt", &HashSet::new()), dir.join("a (2).txt"));
        assert_eq!(unique_path(dir, "a.txt", &HashSet::from([dir.join("a (2).txt")])), dir.join("a (3).txt"));
        assert_eq!(unique_path(dir, "b", &HashSet::new()), dir.join("b"));
    }

    #[test]
    fn a_suggested_name_never_escapes_the_folder() {
        assert_eq!(download_name("https://x/y", Some("../../etc/passwd")), "passwd");
        assert_eq!(download_name("https://x/y", Some(".hidden")), "hidden");
        assert_eq!(download_name("https://x/files/r.pdf", None), "r.pdf");
        assert_eq!(download_name("https://x/y", Some("..\\..\\x.exe")), "x.exe");
    }

    #[test]
    fn files_that_run_on_open_are_dangerous_installers_are_not() {
        for mac in [true, false] {
            assert!(is_dangerous("x.sh", mac));
            assert!(is_dangerous("x.JAR", mac));
            assert!(!is_dangerous("x.pdf", mac));
            assert!(!is_dangerous("x.dmg", mac));
        }
        assert!(is_dangerous("x.app", true) && !is_dangerous("x.app", false));
        assert!(is_dangerous("x.AppImage", false) && !is_dangerous("x.appimage", true));
        assert!(unconfirmed_name().starts_with("Unconfirmed ") && unconfirmed_name().ends_with(".crdownload"));
    }

    #[test]
    fn a_download_cut_off_by_quitting_comes_back_as_retryable() {
        let out = normalize(&DownloadsState {
            items: vec![DownloadItem { engine_id: Some("cefdownload-3".into()), ..base() }],
        });
        assert_eq!(out.items[0].state, DownloadState::Interrupted);
        assert_eq!(out.items[0].reason.as_deref(), Some("shutdown"));
        assert_eq!(out.items[0].speed, 0);
        assert_eq!(out.items[0].engine_id, None);
    }

    #[test]
    fn downloads_json_keeps_two_hundred_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = open(dir.path());
            store.update(|s| {
                for i in 0..205 {
                    s.add(DownloadItem { id: format!("d{i}"), ..base() });
                }
            });
            assert_eq!(store.get().items.len(), 200);
            assert_eq!(store.get().items[0].id, "d204");
        }
        let store = open(dir.path());
        assert_eq!(store.get().items.len(), 200);
        assert!(store.get().items.iter().all(|d| d.state == DownloadState::Interrupted));
        store.update(|s| s.clear());
        assert!(store.get().items.is_empty());
    }

    #[test]
    fn keep_and_discard_an_unconfirmed_file() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("Unconfirmed 123456.crdownload");
        std::fs::write(&part, "x").unwrap();
        std::fs::write(dir.path().join("run.sh"), "taken").unwrap();
        let mut s = DownloadsState::default();
        s.add(DownloadItem {
            name: "run.sh".into(),
            path: dir.path().join("run.sh"),
            part_path: Some(part.clone()),
            state: DownloadState::Dangerous,
            ..base()
        });
        s.keep_dangerous("d1").unwrap();
        assert_eq!(s.items[0].path, dir.path().join("run (2).sh"));
        assert_eq!(s.items[0].name, "run (2).sh");
        assert_eq!(s.items[0].state, DownloadState::Complete);
        assert!(!part.exists());

        std::fs::write(&part, "y").unwrap();
        s.add(DownloadItem {
            id: "d2".into(),
            part_path: Some(part.clone()),
            state: DownloadState::Dangerous,
            ..base()
        });
        s.discard_dangerous("d2");
        assert!(!part.exists());
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn a_long_name_is_cut_in_the_middle_and_keeps_its_extension() {
        let cut = short_name("Quarterly report, final revision, signed copy for the board meeting.pdf", 44);
        assert_eq!(cut.chars().count(), 44);
        assert!(cut.ends_with(".pdf") && cut.contains('…'));
        assert_eq!(
            short_name("quarterly-report-2026-for-the-board.txt", 44),
            "quarterly-report-2026-for-the-board.txt"
        );
    }

    #[test]
    fn a_saved_page_is_named_after_its_title_other_files_keep_their_name() {
        assert_eq!(saved_page_name("Audit: page", "http://127.0.0.1/t.html", Some("t.html")), "Audit_ page.html");
        assert_eq!(saved_page_name("Home", "https://example.com/", Some("download")), "Home.html");
        assert_eq!(saved_page_name("", "https://example.com/a/index.php", None), "index.php");
        assert_eq!(saved_page_name("", "https://example.com/", None), "download.html");
        assert_eq!(saved_page_name("Report", "https://example.com/files/report.pdf", None), "report.pdf");
        assert_eq!(file_icon("a.PNG"), "image-x-generic-symbolic");
    }
}
