//! Favicons cached by origin, not by page: one icon per site is what a sidebar shows, and a tab
//! restored before its page loads already has its icon. Each origin keeps one icon per
//! appearance, because a site may pick its icon by `prefers-color-scheme` (GitHub's white
//! octocat in dark mode vanishes on a light tile). The cache is `favicons/light/` and
//! `favicons/dark/` holding one file per encoded origin with the `data:` URL verbatim, the
//! 0.5.11 layout.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine as _;
use percent_encoding::percent_decode_str;
use regex::Regex;

use crate::url::encode_component;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Appearance {
    #[default]
    Light,
    Dark,
}

impl Appearance {
    fn dir(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn other(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }
}

/// What an icon is drawn in: near-white marks on nothing, near-black marks on nothing, or
/// anything else (its own backdrop, colour, a file that cannot be decoded).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Light,
    Dark,
    Mixed,
}

#[derive(Clone, Debug)]
struct Icon {
    data_url: String,
    ink: Ink,
}

/// The cache, read into memory whole at open: one small image per site and appearance, and a
/// sidebar row must render its icon in the same frame as its title.
#[derive(Debug, Default)]
pub struct Favicons {
    dir: Option<PathBuf>,
    memory: HashMap<String, HashMap<Appearance, Icon>>,
    current: Appearance,
}

pub fn origin_of(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) if u.origin().is_tuple() => u.origin().ascii_serialization(),
        _ => String::new(),
    }
}

impl Favicons {
    /// Opens the cache in `<dir>/favicons`. A half-written entry is a cache miss, never a
    /// startup failure.
    pub fn open(dir: &Path) -> Self {
        let root = dir.join("favicons");
        let mut out = Self { dir: Some(root.clone()), ..Self::default() };
        for appearance in [Appearance::Light, Appearance::Dark] {
            let sub = root.join(appearance.dir());
            let _ = std::fs::create_dir_all(&sub);
            let Ok(entries) = std::fs::read_dir(&sub) else { continue };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let Ok(origin) = percent_decode_str(&name).decode_utf8() else { continue };
                let Ok(data_url) = std::fs::read_to_string(entry.path()) else { continue };
                out.insert(origin.into_owned(), appearance, data_url);
            }
        }
        out
    }

    fn insert(&mut self, origin: String, appearance: Appearance, data_url: String) {
        let ink = ink_of(&data_url);
        self.memory.entry(origin).or_default().insert(appearance, Icon { data_url, ink });
    }

    /// The appearance icons are read for, and recorded under when no other is named. `true`
    /// when it changed, so the caller knows to redraw.
    pub fn set_appearance(&mut self, appearance: Appearance) -> bool {
        let changed = self.current != appearance;
        self.current = appearance;
        changed
    }

    pub fn appearance(&self) -> Appearance {
        self.current
    }

    /// The icon the site gave for this appearance. Without one, the other appearance's icon
    /// stands in unless its ink would vanish on this background, which leaves the row to its
    /// placeholder until the page is seen again in this appearance.
    pub fn favicon_for(&self, url: &str) -> Option<&str> {
        let variants = self.memory.get(&origin_of(url))?;
        if let Some(own) = variants.get(&self.current) {
            return Some(&own.data_url);
        }
        let other = variants.get(&self.current.other())?;
        readable(other.ink, self.current).then_some(other.data_url.as_str())
    }

    /// Keeps `data_url` for the page's origin under `appearance` (the current one when `None`),
    /// the one the page was showing when it named the icon. `true` when something changed.
    pub fn remember(&mut self, url: &str, data_url: &str, appearance: Option<Appearance>) -> bool {
        let appearance = appearance.unwrap_or(self.current);
        let origin = origin_of(url);
        if origin.is_empty() || data_url.is_empty() {
            return false;
        }
        if self.memory.get(&origin).and_then(|v| v.get(&appearance)).is_some_and(|i| i.data_url == data_url) {
            return false;
        }
        if let Some(root) = &self.dir {
            // An unwritable cache still works for this session.
            let _ = std::fs::write(root.join(appearance.dir()).join(encode_component(&origin)), data_url);
        }
        self.insert(origin, appearance, data_url.to_owned());
        true
    }
}

/// Fetches an icon the engine named by address (macOS hands over the icon's own address) and
/// returns it as a `data:` URL. Blocking; run it off the main thread.
pub fn fetch_data_url(icon_url: &str) -> Option<String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).build().into();
    let mut response = agent.get(icon_url).call().ok()?;
    let kind = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("image/png").to_owned();
    // Plenty of sites answer a missing icon with a 200 HTML page; kept, it would stand in for
    // the site's letter with an image nothing can draw.
    if !kind.starts_with("image/") {
        return None;
    }
    let bytes = response.body_mut().with_config().limit(4 * 1024 * 1024).read_to_vec().ok()?;
    Some(format!("data:{kind};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

pub fn readable(ink: Ink, on: Appearance) -> bool {
    ink != match on {
        Appearance::Light => Ink::Light,
        Appearance::Dark => Ink::Dark,
    }
}

pub fn readable_on(data_url: &str, on: Appearance) -> bool {
    readable(ink_of(data_url), on)
}

pub fn ink_of(data_url: &str) -> Ink {
    let Some(rest) = data_url.strip_prefix("data:") else { return Ink::Mixed };
    let Some((head, body)) = rest.split_once(',') else { return Ink::Mixed };
    let bytes = if head.ends_with(";base64") {
        let clean: String = body.chars().filter(|c| !c.is_whitespace()).collect();
        let Ok(b) = base64::engine::general_purpose::STANDARD
            .decode(clean.trim_end_matches('='))
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(clean.trim_end_matches('=')))
        else {
            return Ink::Mixed;
        };
        b
    } else {
        match percent_decode_str(body).decode_utf8() {
            Ok(text) => text.into_owned().into_bytes(),
            Err(_) => return Ink::Mixed,
        }
    };
    if head.starts_with("image/svg") {
        return svg_ink(&String::from_utf8_lossy(&bytes));
    }
    png_in(&bytes).map_or(Ink::Mixed, png_ink)
}

/// Relative luminance, 0 to 1, of sRGB channels in 0 to 255.
fn luminance(r: u8, g: u8, b: u8) -> f64 {
    let lin = |c: u8| {
        let s = f64::from(c) / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

const LIGHT: f64 = 0.7;
const DARK: f64 = 0.05;

/// Light or dark when nine in ten of the painted pixels (or colours) are; antialiased edges and
/// a stray accent do not make an icon readable.
fn verdict(lums: &[f64]) -> Ink {
    if lums.is_empty() {
        return Ink::Mixed;
    }
    let need = lums.len() as f64 * 0.9;
    if lums.iter().filter(|l| **l >= LIGHT).count() as f64 >= need {
        Ink::Light
    } else if lums.iter().filter(|l| **l <= DARK).count() as f64 >= need {
        Ink::Dark
    } else {
        Ink::Mixed
    }
}

fn parse_color(value: &str) -> Option<(u8, u8, u8)> {
    static RGB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^rgba?\(\s*(\d+)[\s,]+(\d+)[\s,]+(\d+)").unwrap());
    let v = value.trim().to_lowercase();
    match v.as_str() {
        "white" => return Some((255, 255, 255)),
        "black" => return Some((0, 0, 0)),
        _ => {}
    }
    if let Some(hex) = v.strip_prefix('#')
        && (3..=8).contains(&hex.len())
        && hex.chars().all(|c| c.is_ascii_hexdigit())
    {
        let full: String =
            if hex.len() <= 4 { hex[..3].chars().flat_map(|c| [c, c]).collect() } else { hex[..6].to_owned() };
        let at = |i: usize| u8::from_str_radix(&full[i..i + 2], 16).ok();
        return Some((at(0)?, at(2)?, at(4)?));
    }
    let c = RGB.captures(&v)?;
    let channel = |i: usize| c[i].parse::<u32>().ok().map(|n| n.min(255) as u8);
    Some((channel(1)?, channel(2)?, channel(3)?))
}

/// The colours an SVG paints with, from its attributes and styles. One the parser cannot read
/// (a gradient, `currentColor`, a name outside the table) makes it mixed; a shape with no fill
/// at all paints black.
fn svg_ink(svg: &str) -> Ink {
    // A dark-mode rule inside the file is not applied when the icon is drawn as an image; only
    // the default rules count.
    static MEDIA: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"@media[^{]*prefers-color-scheme[^{]*\{(?:[^{}]*\{[^{}]*\})*[^{}]*\}").unwrap());
    static PAINT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"(?:fill|stroke|stop-color)\s*[:=]\s*["']?([^"';>\s)]+\)?)"#).unwrap());
    static SHAPE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"<(path|rect|circle|ellipse|polygon|text)\b").unwrap());
    let text = MEDIA.replace_all(svg, "");
    let mut lums = Vec::new();
    for m in PAINT.captures_iter(&text) {
        let value = &m[1];
        if value == "none" || value == "transparent" {
            continue;
        }
        match parse_color(value) {
            Some((r, g, b)) => lums.push(luminance(r, g, b)),
            None => return Ink::Mixed,
        }
    }
    if lums.is_empty() {
        return if SHAPE.is_match(&text) { Ink::Dark } else { Ink::Mixed };
    }
    verdict(&lums)
}

const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn u16le(b: &[u8], at: usize) -> Option<usize> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize)
}

fn u32le(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}

fn u32be(b: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// The PNG in `bytes`: the file itself, or the first PNG frame of an ICO.
fn png_in(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.starts_with(PNG_SIG) {
        return Some(bytes);
    }
    if bytes.len() > 6 && u16le(bytes, 0)? == 0 && u16le(bytes, 2)? == 1 {
        for i in 0..u16le(bytes, 4)? {
            let entry = 6 + i * 16;
            if entry + 16 > bytes.len() {
                break;
            }
            let size = u32le(bytes, entry + 8)?;
            let offset = u32le(bytes, entry + 12)?;
            let frame = bytes.get(offset..offset.saturating_add(size).min(bytes.len()))?;
            if frame.starts_with(PNG_SIG) {
                return Some(frame);
            }
        }
    }
    None
}

/// 8-bit, non-interlaced PNGs: what a favicon almost always is. Anything else reads as mixed,
/// which keeps the icon.
fn png_ink(png: &[u8]) -> Ink {
    png_ink_checked(png).unwrap_or(Ink::Mixed)
}

fn png_ink_checked(png: &[u8]) -> Option<Ink> {
    let (mut width, mut height, mut depth, mut kind, mut interlace) = (0usize, 0usize, 0u8, 0u8, 0u8);
    let mut palette: Option<&[u8]> = None;
    let mut alphas: Option<&[u8]> = None;
    let mut idat = Vec::new();
    let mut at = 8;
    while at + 8 <= png.len() {
        let length = u32be(png, at)?;
        let chunk = &png[at + 4..at + 8];
        let data = png.get(at + 8..(at + 8 + length).min(png.len()))?;
        match chunk {
            b"IHDR" if data.len() >= 13 => {
                width = u32be(data, 0)?;
                height = u32be(data, 4)?;
                depth = data[8];
                kind = data[9];
                interlace = data[12];
            }
            b"PLTE" => palette = Some(data),
            b"tRNS" => alphas = Some(data),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {}
        }
        at += 12 + length;
    }
    let channels = match kind {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return Some(Ink::Mixed),
    };
    if depth != 8 || interlace != 0 || width == 0 || width * height > 512 * 512 {
        return Some(Ink::Mixed);
    }
    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(idat.as_slice()).read_to_end(&mut raw).ok()?;
    let stride = width * channels;
    if raw.len() < (stride + 1) * height {
        return Some(Ink::Mixed);
    }
    let mut pixels = vec![0u8; stride * height];
    for y in 0..height {
        let filter = raw[y * (stride + 1)];
        let line = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..stride {
            let a = if x >= channels { pixels[y * stride + x - channels] } else { 0 };
            let b = if y > 0 { pixels[(y - 1) * stride + x] } else { 0 };
            let c = if x >= channels && y > 0 { pixels[(y - 1) * stride + x - channels] } else { 0 };
            let add = match filter {
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) >> 1) as u8,
                4 => {
                    let p = i16::from(a) + i16::from(b) - i16::from(c);
                    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => 0,
            };
            pixels[y * stride + x] = line[x].wrapping_add(add);
        }
    }
    let mut lums = Vec::new();
    let mut opaque = 0;
    for i in 0..width * height {
        let at = i * channels;
        let (r, g, b, alpha) = match kind {
            3 => {
                let index = pixels[at] as usize;
                let palette = palette?;
                let rgb = palette.get(index * 3..index * 3 + 3)?;
                (rgb[0], rgb[1], rgb[2], alphas.and_then(|a| a.get(index)).copied().unwrap_or(255))
            }
            0 => (pixels[at], pixels[at], pixels[at], 255),
            4 => (pixels[at], pixels[at], pixels[at], pixels[at + 1]),
            2 => (pixels[at], pixels[at + 1], pixels[at + 2], 255),
            _ => (pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]),
        };
        if alpha < 128 {
            continue;
        }
        opaque += 1;
        lums.push(luminance(r, g, b));
    }
    // An icon that fills its square brings its own backdrop.
    if opaque as f64 > (width * height) as f64 * 0.9 {
        return Some(Ink::Mixed);
    }
    Some(verdict(&lums))
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn svg(body: &str) -> String {
        format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(body))
    }

    fn github_light() -> String {
        svg(r##"<svg fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M16 0" fill="#24292E"/></svg>"##)
    }

    fn github_dark() -> String {
        svg(r#"<svg fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M16 0" fill="white"/></svg>"#)
    }

    fn crc(buf: &[u8]) -> u32 {
        let mut c = !0u32;
        for b in buf {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 != 0 { (c >> 1) ^ 0xedb8_8320 } else { c >> 1 };
            }
        }
        !c
    }

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc(&body).to_be_bytes());
        out
    }

    /// A 16x16 RGBA PNG: a `color` glyph in the middle 8x8 with the rest clear, or the whole
    /// square filled.
    fn png(color: [u8; 3], filled: bool) -> String {
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&16u32.to_be_bytes());
        ihdr[4..8].copy_from_slice(&16u32.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = 6;
        let mut rows = Vec::new();
        for y in 0..16 {
            rows.push(0);
            for x in 0..16 {
                let on = filled || ((4..12).contains(&x) && (4..12).contains(&y));
                rows.extend_from_slice(&color);
                rows.push(if on { 255 } else { 0 });
            }
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&rows).unwrap();
        let mut file = PNG_SIG.to_vec();
        file.extend(chunk(b"IHDR", &ihdr));
        file.extend(chunk(b"IDAT", &z.finish().unwrap()));
        file.extend(chunk(b"IEND", &[]));
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(file))
    }

    #[test]
    fn an_icons_ink_is_read_from_svg_paint_and_png_pixels() {
        assert_eq!(ink_of(&github_dark()), Ink::Light);
        assert_eq!(ink_of(&github_light()), Ink::Dark);
        assert_eq!(ink_of(&svg(r#"<svg><path d="M0 0"/></svg>"#)), Ink::Dark);
        assert_eq!(ink_of(&svg(r##"<svg><path fill="#ff6600"/><path fill="white"/></svg>"##)), Ink::Mixed);
        assert_eq!(ink_of(&svg(r##"<svg><path fill="url(#g)"/></svg>"##)), Ink::Mixed);
        let dark_rule =
            r#"<svg><style>path{fill:#111;}@media (prefers-color-scheme: dark){path{fill:#fff;}}</style><path/></svg>"#;
        assert_eq!(ink_of(&svg(dark_rule)), Ink::Dark);
        assert_eq!(ink_of(&png([255, 255, 255], false)), Ink::Light);
        assert_eq!(ink_of(&png([10, 10, 10], false)), Ink::Dark);
        assert_eq!(ink_of(&png([255, 255, 255], true)), Ink::Mixed);
        assert_eq!(ink_of(&png([230, 80, 20], false)), Ink::Mixed);
        assert_eq!(ink_of("data:image/x-icon;base64,AAAB"), Ink::Mixed);
        assert!(!readable_on(&github_dark(), Appearance::Light));
        assert!(readable_on(&github_dark(), Appearance::Dark));
    }

    #[test]
    fn each_appearance_keeps_its_own_icon_and_the_other_stands_in_only_where_it_shows() {
        let store = tempfile::tempdir().unwrap();
        let mut f = Favicons::open(store.path());
        f.set_appearance(Appearance::Dark);
        let orange = png([230, 80, 20], false);
        assert!(f.remember("https://github.com/oven-sh/bun", &github_dark(), None));
        assert!(f.remember("https://example.com/", &orange, None));
        assert!(!f.remember("https://example.com/b", &orange, None));
        assert_eq!(f.favicon_for("https://github.com/"), Some(github_dark().as_str()));

        // Light, with only the dark icons known: the white octocat would vanish.
        assert!(f.set_appearance(Appearance::Light));
        assert_eq!(f.favicon_for("https://github.com/"), None);
        assert_eq!(f.favicon_for("https://example.com/a"), Some(orange.as_str()));

        // Seen in light, it keeps both.
        f.remember("https://github.com/", &github_light(), None);
        assert_eq!(f.favicon_for("https://github.com/"), Some(github_light().as_str()));
        f.set_appearance(Appearance::Dark);
        assert_eq!(f.favicon_for("https://github.com/"), Some(github_dark().as_str()));
        let names = |d: &str| -> Vec<String> {
            let mut v: Vec<String> = std::fs::read_dir(store.path().join("favicons").join(d))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        };
        assert_eq!(names("light"), [encode_component("https://github.com")]);
        assert_eq!(names("dark").len(), 2);

        let reopened = Favicons::open(store.path());
        assert_eq!(reopened.favicon_for("https://github.com/x"), Some(github_light().as_str()));
        assert!(!f.remember("about:blank", &orange, None));
    }

    #[test]
    fn a_fetched_icon_becomes_a_data_url_and_an_html_answer_is_refused() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for (stream, kind) in server.incoming().zip(["image/png", "text/html"]) {
                let mut stream = stream.unwrap();
                let mut buf = [0u8; 1024];
                let _ = std::io::Read::read(&mut stream, &mut buf);
                let reply = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc"
                );
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });
        let url = format!("http://127.0.0.1:{port}/favicon.ico");
        assert_eq!(fetch_data_url(&url).as_deref(), Some("data:image/png;base64,YWJj"));
        assert_eq!(fetch_data_url(&url), None);
    }
}
