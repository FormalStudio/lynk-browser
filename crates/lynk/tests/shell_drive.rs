//! Drives the real app over nd's automation socket against pages served from this process: one
//! case per shell box of `docs/nd-parity.md`. Every launch gets its own data directory (nd-test
//! makes one per launch, and the Chromium profile lives in it), so nothing of the user's is
//! read or written. Captures land in the test's target tmp dir.
//!
//! It opens real windows and moves the real pointer, so it runs only under nextest or with
//! `ND_REAL=1`. On macOS that is the user's display; on Linux each app gets a private headless
//! Hyprland (`os/linux.rs`, run through `scripts/linux-gate drive`).
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[cfg_attr(target_os = "macos", path = "os/macos.rs")]
#[cfg_attr(target_os = "linux", path = "os/linux.rs")]
mod os;

/// What only Linux can break: the compositor's scale, outputs, tiling and workspaces, layouts,
/// selections, crashes and the profile's path.
#[cfg(target_os = "linux")]
#[path = "os/linux_cases.rs"]
mod linux_cases;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use nd_test::{AppHandle, LaunchOptions, expect};
use os::{App, Modifier, key};

fn real_allowed() -> bool {
    let allowed = std::env::var_os("NEXTEST").is_some() || std::env::var_os("ND_REAL").is_some_and(|v| v == "1");
    if !allowed {
        println!("skipped: opens a real window; run with ND_REAL=1");
    }
    allowed
}

const ALPHA: &str = r#"<!doctype html><title>Alpha</title><link rel="icon" href="/icon.png">
<body style="margin:0;font:16px sans-serif">
<a id="go" href="/beta" style="display:block;height:200px;background:#cde">To beta</a>
<p>needle one, needle two, needle three</p>
<a href="/beta" target="_blank" style="display:block;height:100px;background:#edc">Beta in a new tab</a>
</body>"#;
const BETA: &str = "<!doctype html><title>Beta</title><body><p>The second page.</p></body>";
/// Goes to the address after `?` eight seconds after it loads, by itself.
const LATER: &str = "<!doctype html><title>Later</title><script>\
setTimeout(() => location.replace(decodeURIComponent(location.search.slice(1))), 8000)</script>";
const LEAVE: &str = r#"<!doctype html><title>Leave</title>
<body style="margin:0"><div style="height:400px;background:#fdd">Click me first</div>
<script>addEventListener("beforeunload", e => { e.preventDefault(); e.returnValue = ""; });</script></body>"#;

/// Scrolls on a click and puts where it is in its title, with a stamp of the load.
/// Takes its title once it has painted: Chromium drops a press that reaches a page before its
/// first paint (the release still arrives), and a cached load can name itself before that.
const LONG: &str = r#"<!doctype html>
<body style="margin:0;height:5000px;background:#fff" onclick="window.scrollTo(0, 600)">
<script>new PerformanceObserver(() => document.title ||= "Long").observe({ type: "paint", buffered: true });
onscroll = () => document.title = "At " + Math.round(scrollY) + " " + Math.round(performance.timeOrigin);</script></body>"#;
/// Hangs its renderer on a click.
const BUSY: &str = r#"<!doctype html><title>Busy</title>
<body style="margin:0;height:400px" onclick="setTimeout(() => { for (;;) {} }, 0)"></body>"#;

/// Says it was pressed, for a click as soon as the page has a title.
const PRESS: &str = r#"<!doctype html><title>Press</title>
<body style="margin:0;height:400px;background:#dfd" onpointerdown="document.title = 'Pressed'"></body>"#;

/// Its title is "Keys:" and every printable key the page gets. Its text is painted at once:
/// Chromium drops input to a page until its first contentful paint, up to 500 ms.
const KEYS: &str = r#"<!doctype html><title>Keys:</title>
<body style="margin:0;height:600px" onkeydown="if (event.key.length == 1) document.title += event.key"><p>Keys</p></body>"#;

const FULL: &str = r#"<!doctype html><title>Full</title>
<body style="margin:0"><button style="width:100%;height:300px" onclick="document.documentElement.requestFullscreen()">Go fullscreen</button></body>"#;

/// One element per procedural operator and action, each a user rule on 127.0.0.1 in
/// `PROCEDURAL_RULES`, and elements the rules must leave alone. The page checks itself and says in
/// its title what is still wrong, `Procedural ok` once nothing is.
const PROCEDURAL: &str = r#"<!doctype html><title>Procedural</title><style>.pre::before{content:"AD"}</style>
<div class=post id=t1>Sponsored post</div><div class=post id=c1>A post</div>
<div class=card id=t2><span class=badge>Promoted</span></div>
<div id=t3><p><b class=mark>x</b></p></div>
<div id=t4 data-ad-slot=top>slot</div><div id=c4 data-ad-slot=side>side</div>
<div id=t5 class=float style="position:fixed;top:0">float</div>
<div id=t6 class=long>a long enough block of text here</div><div id=c6 class=long>short</div>
<section id=t7><h2>Ad</h2></section><section id=c7><h2>News</h2></section>
<div id=t8 class=pathy>path</div>
<div id=t9 class=gone>gone</div>
<div id=t10 class=restyle>restyle</div>
<div id=t11 class=pre>before</div>
<div id=t12 class="rmclass ad">class</div>
<div id=t13 data-track=1>attr</div>
<p class=lead>Ad below</p><div id=t14 class=next>next</div>
<script>
const shown = (id) => getComputedStyle(document.getElementById(id)).display !== 'none';
const checks = {
  t1: () => !shown('t1'), c1: () => shown('c1'), t2: () => !shown('t2'), t3: () => !shown('t3'),
  t4: () => !shown('t4'), c4: () => shown('c4'), t5: () => !shown('t5'), t6: () => !shown('t6'),
  c6: () => shown('c6'), t7: () => !shown('t7'), c7: () => shown('c7'), t8: () => !shown('t8'),
  t9: () => !document.getElementById('t9'),
  t10: () => getComputedStyle(document.getElementById('t10')).opacity === '0.5',
  t11: () => !shown('t11'), t12: () => !document.getElementById('t12').classList.contains('ad'),
  t13: () => !document.getElementById('t13').hasAttribute('data-track'), t14: () => !shown('t14'),
};
const check = () => {
  const wrong = Object.keys(checks).filter((k) => { try { return !checks[k](); } catch (_) { return true; } });
  document.title = 'Procedural ' + (wrong.length ? wrong.join(' ') : 'ok');
  if (wrong.length) setTimeout(check, 200);
};
setTimeout(check, 200);
</script>"#;
/// The rules `PROCEDURAL` checks, one per element.
const PROCEDURAL_RULES: &[&str] = &[
    ".post:has-text(Sponsored)",
    ".badge:has-text(Promoted):upward(.card)",
    ".mark:upward(2)",
    "div:matches-attr(\"data-ad-slot\"=\"top\")",
    ".float:matches-css(position: fixed)",
    ".long:min-text-length(20)",
    ":xpath(//section[h2=\"Ad\"])",
    ".pathy:matches-path(/procedural)",
    ".gone:remove()",
    ".restyle:style(opacity: 0.5)",
    ".pre:matches-css-before(content: /AD/)",
    ".rmclass:remove-class(ad)",
    "[data-track]:remove-attr(data-track)",
    ".lead:has-text(Ad) + .next",
];
/// `ADS` again under a policy that refuses inline script and style, its own script served from
/// the page's origin. A user scriptlet sets `lynkSet` and a user rule hides `.promo`; the title
/// says what got through: `Csp <promo> <scriptlet>`.
const CSP: &str = r#"<!doctype html><title>Csp</title><div class="promo">Promo</div><p>Content</p>
<script src="/csp.js"></script>"#;
const CSP_SCRIPT: &str = "setInterval(() => { document.title = \
    `Csp ${getComputedStyle(document.querySelector('.promo')).display} ${window.lynkSet === 1 ? 'set' : 'unset'}`; }, 200);";
const CSP_POLICY: &str = "Content-Security-Policy: script-src 'self'; style-src 'self'\r\n";
/// Frames the address after `?`, which tells it what became of its `.ad_banner`; the title says
/// it: `Framed <display>`.
const FRAMED: &str = r#"<!doctype html><title>Framed</title>
<script>addEventListener('message', (e) => { document.title = 'Framed ' + e.data; });</script>
<iframe id=frame width=400 height=200></iframe>
<script>document.getElementById('frame').src = decodeURIComponent(location.search.slice(1));</script>"#;
/// An element EasyList's generic `##.ad_banner` hides, reported to the framing page.
const AD_FRAME: &str = r#"<!doctype html><div class="ad_banner">Ad</div><p>Frame</p>
<script>setInterval(() => parent.postMessage(getComputedStyle(document.querySelector('.ad_banner')).display, '*'), 200);</script>"#;
/// Requests the uBO lists answer: `/468x60/*$image,redirect=1x1.gif` swaps the banner for a 1x1
/// stand-in, and EasyList's `/pagead/conversion.js$script` is stopped in a frame from another
/// site, given in the query. The title reports both: `Net <banner> <frame's script>`.
const NET: &str = r#"<!doctype html><title>Net</title>
<img id=banner src="/468x60/net.png">
<iframe id=frame width=400 height=200></iframe>
<script>
let frame = 'waiting';
addEventListener('message', (e) => { frame = e.data; });
document.getElementById('frame').src = decodeURIComponent(location.search.slice(1));
const b = document.getElementById('banner');
setInterval(() => { document.title = `Net ${!b.complete ? 'loading' : b.naturalWidth === 1 ? 'swapped' : 'kept'} ${frame}`; }, 200);
</script>"#;
const NET_FRAME: &str = r#"<!doctype html><script src="/pagead/conversion.js"></script>
<script>setInterval(() => parent.postMessage(window.converted ? 'ran' : 'stopped', '*'), 200);</script>"#;
/// A page with an icon of its own.
const ICONIC: &str = "<!doctype html><title>Iconic</title><link rel=icon href=/icon.svg><p>Its own icon.";
const ICON: &str =
    "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16'><rect width='16' height='16' fill='#c33'/></svg>";
/// No colours of its own: Chromium's default white is what it sits on.
const PLAIN: &str = "<!doctype html><title>Plain</title><p>No colours of its own.";
/// Two requests EasyList stops wherever they are served (`/pagead/conversion.js$script`,
/// `/banner_ads/*`), an element its generic `##.ad_banner` hides (except on 127.0.0.1, where
/// EasyList's `@@://127.0.0.1$generichide` turns generic rules off) and one the user hid on
/// 127.0.0.1. The title reports what each did: `Ads <ad_banner> <promo> <script>`.
/// A banner the page acts on when pressed and an article. The title says what each is, `Hide
/// <banner> <article>`, `Hide pressed` once the page took a press on the banner.
const HIDE: &str = r#"<!doctype html><title>Hide</title>
<body style="margin:0;font:16px system-ui">
<div id=banner style="height:160px;background:#fd8;display:grid;place-items:center" onpointerdown="document.title = 'Hide pressed'">Sponsored banner</div>
<article id=article style="height:400px;padding:24px">An article worth keeping</article>
<script>
const state = (id) => getComputedStyle(document.getElementById(id)).display === 'none' ? 'gone' : 'shown';
const report = () => { if (document.title === 'Hide pressed') return;
  document.title = `Hide ${state('banner')} ${state('article')}`; setTimeout(report, 100); };
report();
</script>"#;

/// An article long enough for Readability and to scroll, on a green page.
const ARTICLE: &str = r#"<!doctype html><title>Article</title>
<body style="margin:0;background:#dfd"><nav>Home · News · About</nav>
<article><h1>A long walk along the coast</h1>
<p>The path leaves the harbour past the fish market and climbs onto the cliffs, where the wind comes straight off the water and the gulls hang still above the edge for minutes at a time.</p>
<p>Two hours on, the first cove opens below: a crescent of grey shingle, a lifeboat ramp, and a café that has served the same three cakes since anyone in the village can remember.</p>
<p>From there the walk turns inland through gorse and bracken, following an old miners' track whose stones are polished by a century of boots, past the ruins of an engine house with its chimney still standing.</p>
<p>The last stretch drops back to the sea at a beach that only appears at low tide, so the walk is best timed by the tables pinned to the harbour noticeboard rather than by the clock.</p>
<p>By evening the light goes gold and then pink over the headland, and the lighthouse starts its slow sweep across the bay while the café stacks its chairs for the night.</p>
<p>Ten more paragraphs could follow here about the birds, the boats and the weather, and a reader would scroll past each of them to reach the end of the page.</p>
<p>The tide tables, the bus back to the harbour and the opening hours of the café are all on the noticeboard by the lifeboat station, beside a map of the path.</p>
<p>Walkers who want a shorter day can take the bus out to the engine house and walk only the last stretch back to the beach and the harbour beyond it.</p>
</article></body>"#;

/// A small video and a large playing one, both fed from a canvas, and a title that says which is
/// in picture-in-picture: `Video <id>` or `Video none`.
const VIDEO: &str = r#"<!doctype html><title>Video none</title>
<body style="margin:0;background:#222">
<video id=small muted style="width:160px;height:90px"></video>
<video id=big muted style="width:640px;height:360px"></video>
<script>
const feed = (hue) => { const c = document.createElement('canvas'); c.width = 320; c.height = 180;
  const g = c.getContext('2d'); let t = 0;
  setInterval(() => { g.fillStyle = `hsl(${hue + t++ % 60} 70% 50%)`; g.fillRect(0, 0, 320, 180); }, 33);
  return c.captureStream(30); };
small.srcObject = feed(0); big.srcObject = feed(200);
small.onloadedmetadata = () => small.pause();
big.onloadedmetadata = () => big.play();
for (const v of [small, big]) {
  v.addEventListener('enterpictureinpicture', () => document.title = 'Video ' + v.id);
  v.addEventListener('leavepictureinpicture', () => document.title = 'Video none');
}
</script>"#;

const ADS: &str = r#"<!doctype html><title>Ads</title>
<script src="/pagead/conversion.js"></script>
<img src="/banner_ads/b.png" alt="">
<div class="ad_banner">Ad</div><div class="promo">Promo</div><p>Content</p>
<script>
const shown = s => getComputedStyle(document.querySelector(s)).display;
setInterval(() => { document.title = `Ads ${shown('.ad_banner')} ${shown('.promo')} ${window.converted ? 'ran' : 'stopped'}`; }, 200);
</script>"#;
const DARK_SCHEME: &str = "<!doctype html><meta name=color-scheme content=dark><title>Dark</title><p>Dark only.";
const BOTH_SCHEMES: &str =
    "<!doctype html><meta name=color-scheme content=\"light dark\"><title>Both</title><p>Either scheme.";

/// A title no sidebar row has room for.
const LONG_TITLE: &str =
    "<!doctype html><title>A page whose title runs on well past the width of any sidebar row</title>";

/// Opens pop-ups as fast as it can, never from a click.
const POPUPS: &str = r#"<!doctype html><title>Popups</title>
<body><script>setInterval(() => window.open("/beta?" + Math.random()), 20);</script></body>"#;

/// Serves the fixture pages on a free local port for the rest of the test process, each request
/// on its own thread, since `/slow` never answers.
fn serve() -> String {
    serve_at("127.0.0.1:0")
}

/// The fixture pages on `addr`, for a page that must not be on 127.0.0.1.
fn serve_at(addr: &str) -> String {
    let listener = TcpListener::bind(addr).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            std::thread::spawn(move || respond(stream));
        }
    });
    base
}

/// Every request the test server has answered, by path and query.
static REQUESTS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn requested(target: &str) -> bool {
    REQUESTS.lock().unwrap().iter().any(|t| t == target)
}

fn respond(mut stream: std::net::TcpStream) {
    let mut line = String::new();
    let mut reader = BufReader::new(&stream);
    let _ = reader.read_line(&mut line);
    let mut cookie = String::new();
    let mut header = String::new();
    while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("cookie")
        {
            cookie = value.trim().to_owned();
        }
        header.clear();
    }
    let target = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    REQUESTS.lock().unwrap().push(target.clone());
    let path = target.split('?').next().unwrap_or("/");
    // `/468x60/` is a banner the uBO lists answer with a 1x1 stand-in; unanswered, it is the icon.
    if path == "/icon.png" || path.starts_with("/468x60/") {
        let icon = include_bytes!("fixtures/icon.png");
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            icon.len()
        );
        let _ = stream.write_all(icon);
        return;
    }
    let page = |body: &str| ("200 OK", String::new(), body.to_owned());
    let (status, extra, body) = match path {
        "/alpha" => page(ALPHA),
        "/beta" => page(BETA),
        "/later" => page(LATER),
        "/leave" => page(LEAVE),
        "/full" => page(FULL),
        "/long" => page(LONG),
        "/busy" => page(BUSY),
        "/press" => page(PRESS),
        "/keys" => page(KEYS),
        "/popups" => page(POPUPS),
        "/plain" => page(PLAIN),
        "/iconic" => page(ICONIC),
        "/procedural" => page(PROCEDURAL),
        "/framed" => page(FRAMED),
        "/ad-frame" => page(AD_FRAME),
        "/net" => page(NET),
        "/net-frame" => page(NET_FRAME),
        "/csp" => ("200 OK", CSP_POLICY.to_owned(), CSP.to_owned()),
        "/csp.js" => ("200 OK", "Content-Type: text/javascript\r\n".to_owned(), CSP_SCRIPT.to_owned()),
        "/icon.svg" => ("200 OK", "Content-Type: image/svg+xml\r\n".to_owned(), ICON.to_owned()),
        "/ads" => page(ADS),
        "/hide" => page(HIDE),
        "/article" => page(ARTICLE),
        "/video" => page(VIDEO),
        "/pagead/conversion.js" => ("200 OK", String::new(), "window.converted = true;".to_owned()),
        "/dark-scheme" => page(DARK_SCHEME),
        "/both-schemes" => page(BOTH_SCHEMES),
        "/long-title" => page(LONG_TITLE),
        "/many" => page(&format!("<!doctype html><title>Many</title><p>{}", "needle ".repeat(10_000))),
        // 50 MB of text under a title.
        "/big" => page(&format!("<!doctype html><title>Big</title><pre>{}", "x".repeat(50 << 20))),
        // A cookie that outlives the session, and a page whose title is the cookie it was sent.
        "/set-cookie" => (
            "200 OK",
            "Set-Cookie: who=one; Path=/; Max-Age=3600\r\n".to_owned(),
            "<!doctype html><title>Set</title>".to_owned(),
        ),
        "/cookie" => {
            page(&format!("<!doctype html><title>Cookie {}</title>", if cookie.is_empty() { "none" } else { &cookie }))
        }
        // The cookie set, then the page that reads it back, in the same tab.
        "/set-cookie-read" => (
            "200 OK",
            "Set-Cookie: who=one; Path=/; Max-Age=3600\r\n".to_owned(),
            "<!doctype html><script>location.replace('/cookie')</script>".to_owned(),
        ),
        // localStorage written, then the page that reads it back. Its title carries a number of
        // its own load, so a title kept from before a restart does not read as a fresh one.
        "/store-set" => page("<!doctype html><script>localStorage.x='kept';location.replace('/store')</script>"),
        "/store" => {
            page("<!doctype html><script>document.title='Stored '+(localStorage.x||'none')+' #'+Math.random()</script>")
        }
        "/loop" => ("302 Found", "Location: /loop\r\n".to_owned(), String::new()),
        "/late" => {
            std::thread::sleep(Duration::from_secs(3));
            page("<!doctype html><title>Late</title><p>Late")
        }
        "/slow" => {
            std::thread::sleep(Duration::from_secs(600));
            return;
        }
        _ => ("404 Not Found", String::new(), "<!doctype html><title>Missing</title>".to_owned()),
    };
    let kind = if extra.contains("Content-Type") { "" } else { "Content-Type: text/html\r\n" };
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\n{kind}{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn options(url: &str) -> LaunchOptions {
    LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
        .env("LYNK_URL", url)
        .env("ND_CEF_STYLE", "chrome")
        .env("LYNK_SEARCHENGINE", "google")
}

fn launch(url: &str) -> App {
    os::launch(options(url))
}

/// Launches on `urls`, a tab each, as a launch from the command line does.
fn launch_with(urls: &[String]) -> App {
    let options = urls.iter().fold(
        LaunchOptions::new(env!("CARGO_BIN_EXE_lynk")).env("ND_CEF_STYLE", "chrome").env("LYNK_SEARCHENGINE", "google"),
        |o, url| o.arg(url),
    );
    os::launch(options)
}

/// A profile of the test's own, which outlives one launch so the next can restore it.
fn profile(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn profile_options(profile: &std::path::Path, urls: &[String]) -> LaunchOptions {
    urls.iter().fold(
        LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
            .env("ND_CEF_STYLE", "chrome")
            .env("ND_APP_DATA_DIR", profile.display().to_string()),
        |o, url| o.arg(url),
    )
}

/// Launches on `profile` with `urls` as the command line's.
fn launch_on(profile: &std::path::Path, urls: &[String]) -> App {
    let app = os::launch(profile_options(profile, urls));
    os::in_front(&app);
    app
}

/// Quits the app as the system asks an app to (`os::quit`), and waits for it to go.
fn quit(app: &App) {
    os::quit(app);
    exited(app, 15);
}

/// The profile's `session.json`, its data.
fn session_of(profile: &std::path::Path) -> serde_json::Value {
    let text = std::fs::read_to_string(profile.join("session.json")).unwrap();
    let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    doc["data"].take()
}

/// The app's frontmost browser window, skipping popovers and other small windows.
fn front_window(app: &App) -> os::Win {
    on_screen_window(app).expect("the app has a window on screen")
}

fn on_screen_window(app: &App) -> Option<os::Win> {
    os::windows(app).into_iter().find(|w| w.bounds.2 >= 400. && w.bounds.3 >= 300.)
}

/// The front window's width, for a wait: none while the window is off the screen's list.
fn width_of(app: &App) -> Option<f64> {
    on_screen_window(app).map(|w| w.bounds.2)
}

/// Whether the key window is in fullscreen with macOS done moving it there; until then its
/// frame may have grown already, but it takes no input.
fn settled_fullscreen(app: &App) -> bool {
    app.windows()
        .ok()
        .and_then(|w| w.as_array().cloned())
        .is_some_and(|ws| ws.iter().any(|w| w["key"] == true && w["fullscreen"] == true))
}

/// Resizes the front window to `width` points, keeping its height, as a drag of its edge would.
fn resize(app: &App, width: u32) {
    os::resize(app, width, front_window(app).bounds.3 as u32);
    std::thread::sleep(Duration::from_millis(800));
}

/// A real click at `(x, y)` in the window's points. A Chrome-style page lives in Chromium's own
/// child window, which events posted to the app's window never reach; `launch_app` holds the
/// machine's real-input lock for the run on macOS.
fn real_click(app: &App, x: f32, y: f32) {
    os::in_front(app);
    let (wx, wy, _, _) = front_window(app).bounds;
    os::click(app, (wx + f64::from(x), wy + f64::from(y)));
    std::thread::sleep(Duration::from_millis(400));
}

/// A composited capture of the window, glass and pages included, which the automation
/// socket's layer render leaves out.
fn shot(app: &App, name: &str) -> PathBuf {
    let path = os::shot_path(name);
    os::capture(app, &front_window(app), &path);
    println!("screenshot {}", path.display());
    path
}

/// Waits until `check` holds, failing with the tab titles and the app's stderr after `secs`.
fn until(app: &App, what: &str, secs: u64, check: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if check(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    panic!("timed out waiting for {what}, tabs {:?}\n{}", titles(app), app.stderr_tail(80).join("\n"));
}

fn text_of(app: &AppHandle, test_id: &str) -> Option<String> {
    let tree = app.tree(None).ok()?;
    tree.windows
        .iter()
        .find_map(|w| w.root.iter().find(|n| n.test_id.as_deref() == Some(test_id)).map(|n| n.name.clone()))
}

/// The app answered and shows no visible `test_id`; an app that gave no tree lacks nothing.
fn lacks(app: &AppHandle, test_id: &str) -> bool {
    app.tree(None).is_ok_and(|t| {
        !t.windows.iter().any(|w| w.root.iter().any(|n| n.test_id.as_deref() == Some(test_id) && n.visible))
    })
}

fn has(app: &AppHandle, test_id: &str) -> bool {
    shows(app, |id| id == test_id)
}

fn has_prefix(app: &AppHandle, prefix: &str) -> bool {
    shows(app, |id| id.starts_with(prefix))
}

fn shows(app: &AppHandle, id: impl Fn(&str) -> bool) -> bool {
    app.tree(None).is_ok_and(|t| {
        t.windows.iter().any(|w| w.root.iter().any(|n| n.test_id.as_deref().is_some_and(&id) && n.visible))
    })
}

/// The tab rows' titles of the first window, in sidebar order.
fn titles(app: &AppHandle) -> Vec<String> {
    let Ok(tree) = app.tree(None) else { return Vec::new() };
    tree.windows.first().map(window_titles).unwrap_or_default()
}

fn window_titles(window: &nd_test::SemanticWindow) -> Vec<String> {
    let mut rows: Vec<(f32, String)> = window
        .root
        .iter()
        .filter(|n| n.test_id.as_deref().is_some_and(|t| t.starts_with("tab-title-")))
        .map(|n| (n.bounds.map_or(0., |b| b.origin.y), n.name.clone()))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, t)| t).collect()
}

fn window_count(app: &AppHandle) -> usize {
    app.tree(None).map_or(0, |t| t.windows.len())
}

fn press(app: &AppHandle, keys: &str) {
    app.keyboard().press(&os::chord(keys)).unwrap();
    std::thread::sleep(Duration::from_millis(300));
}

/// Clicks inside the page, `y` points below the content area's top.
fn click_page(app: &App, y: f32) {
    let b = app.get_by_test_id("content").bounding_box().unwrap().expect("the page is laid out");
    real_click(app, b.x + b.width / 2., b.y + y);
}

#[test]
fn real_tabs_navigation_and_layout() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the first page's title", 30, |a| titles(a) == ["Alpha"]);
    expect(&app.get_by_test_id("controls-row")).to_be_visible().unwrap();
    expect(&app.get_by_test_id(os::NEW_TAB)).to_be_visible().unwrap();
    resize(&app, 1200);
    shot(&app, "sidebar-1200");
    resize(&app, 720);
    shot(&app, "sidebar-720");
    resize(&app, 1200);

    // A link loads in place; back and forward walk the page's history.
    click_page(&app, 100.);
    until(&app, "the link to load Beta", 15, |a| titles(a) == ["Beta"]);
    press(&app, "cmd-[");
    until(&app, "back to Alpha", 15, |a| titles(a) == ["Alpha"]);
    press(&app, "cmd-]");
    until(&app, "forward to Beta", 15, |a| titles(a) == ["Beta"]);
    press(&app, "cmd-[");
    until(&app, "back to Alpha again", 15, |a| titles(a) == ["Alpha"]);

    // Cmd+T opens a tab through the command bar; Cmd+1 and Cmd+9 select by place.
    open_in_new_tab(&app, &format!("{base}/long"));
    until(&app, "the second tab", 15, |a| titles(a) == ["Alpha", "Long"]);
    press(&app, "cmd-1");
    until(&app, "Alpha shown", 10, |a| shown_title(a) == "Alpha");
    press(&app, "cmd-9");
    until(&app, "the last tab shown", 10, |a| shown_title(a) == "Long");
    press(&app, "cmd-1");
    until(&app, "Alpha shown again", 10, |a| shown_title(a) == "Alpha");

    // A target=_blank link opens a tab right after its opener, in front, not at the end.
    click_page(&app, 300.);
    until(&app, "the page-opened tab", 15, |a| titles(a) == ["Alpha", "Beta", "Long"]);
    press(&app, "cmd-9");
    until(&app, "the last tab shown", 10, |a| shown_title(a) == "Long");
    press(&app, "cmd-shift-[");
    until(&app, "previous tab", 10, |a| shown_title(a) == "Beta");

    // Cmd+W closes the shown tab; Shift+Cmd+T brings it back from the stack.
    press(&app, "cmd-w");
    until(&app, "Beta closed", 10, |a| titles(a) == ["Alpha", "Long"]);
    press(&app, "cmd-shift-t");
    until(&app, "Beta reopened", 15, |a| titles(a) == ["Alpha", "Long", "Beta"]);

    // Shift+Cmd+B hides the sidebar and shows it again.
    press(&app, "cmd-shift-b");
    until(&app, "the sidebar hidden", 10, |a| lacks(a, "controls-row"));
    shot(&app, "sidebar-hidden");
    press(&app, "cmd-shift-b");
    until(&app, "the sidebar back", 10, |a| has(a, "controls-row"));
}

#[test]
fn real_find_in_page() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    press(&app, "cmd-f");
    until(&app, "the find bar", 10, |a| has(a, "find-bar"));
    app.get_by_test_id("find-field").type_text("needle").unwrap();
    until(&app, "three matches", 10, |a| text_of(a, "find-count").as_deref() == Some("3 matches"));
    shot(&app, "find");
    press(&app, "cmd-g");
    press(&app, "cmd-shift-g");
    app.get_by_test_id("find-field").fill("haystack").unwrap();
    until(&app, "no matches", 10, |a| text_of(a, "find-count").as_deref() == Some("No matches"));
    press(&app, "escape");
    until(&app, "the bar closed", 10, |a| lacks(a, "find-bar"));
}

#[test]
fn real_zoom_steps_and_glyph() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    assert!(lacks(&app, "zoom"), "no glyph at 100%");
    press(&app, "cmd-=");
    until(&app, "the zoom popover", 10, |a| text_of(a, "zoom-value").as_deref() == Some("110%"));
    shot(&app, "zoom-popover");
    press(&app, "cmd--");
    press(&app, "cmd--");
    until(&app, "90%", 10, |a| text_of(a, "zoom-value").as_deref() == Some("90%"));
    // The notice goes after 1.5 s; the glyph stays while the page is not at 100%.
    until(&app, "the popover gone", 10, |a| lacks(a, "zoom-value"));
    assert!(has(&app, "zoom"));
    press(&app, "cmd-0");
    until(&app, "the glyph gone at 100%", 10, |a| lacks(a, "zoom"));
}

#[test]
fn real_windows_and_closing() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    press(&app, "cmd-n");
    until(&app, "a second window", 10, |a| window_count(a) == 2);
    until(&app, "its own menu", 10, |a| has(a, "window-menu"));
    // The new window lists its new tab before any page event arrives.
    until(&app, "the second window's rows", 10, |a| {
        a.tree(None).is_ok_and(|t| {
            t.windows.get(1).is_some_and(|w| {
                let row = |id: &str| w.root.iter().any(|n| n.test_id.as_deref() == Some(id) && n.visible);
                row(os::NEW_TAB) && w.root.iter().any(|n| n.name == "New Tab")
            })
        })
    });
    shot(&app, "second-window");
    // The second window's last tab takes the window with it.
    press(&app, "cmd-w");
    until(&app, "back to one window", 10, |a| window_count(a) == 1);
    assert_eq!(titles(&app), ["Alpha"]);
}

#[test]
fn real_beforeunload_holds_the_close() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/leave"));
    until(&app, "the page", 30, |a| titles(a) == ["Leave"]);
    // A page earns its beforeunload prompt only once the user touched it.
    click_page(&app, 100.);
    open_in_new_tab(&app, &format!("{base}/beta"));
    press(&app, "cmd-1");
    press(&app, "cmd-w");
    std::thread::sleep(Duration::from_secs(1));
    shot(&app, "leave-site");
    until(&app, "Beta's title", 10, |a| titles(a) == ["Leave", "Beta"]);
    // Esc takes Cancel, which keeps the tab.
    press(&app, "escape");
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(titles(&app), ["Leave", "Beta"], "Cancel keeps the tab");
    // Enter takes Chrome's dialog's default, Leave.
    press(&app, "cmd-w");
    std::thread::sleep(Duration::from_secs(1));
    press(&app, "enter");
    until(&app, "the tab gone once the page agreed", 10, |a| titles(a) == ["Beta"]);

    // The window's close button asks the page too, and Cancel keeps the window.
    press(&app, "cmd-shift-t");
    until(&app, "the page back", 15, |a| titles(a) == ["Beta", "Leave"]);
    click_page(&app, 100.);
    os::close_window(&app, &front_window(&app));
    std::thread::sleep(Duration::from_secs(1));
    // The prompt is a window of its own on macOS and drawn in the page on Linux.
    assert_eq!(window_count(&app), 1, "the window waits on its page's answer, under its prompt");
    press(&app, "escape");
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(titles(&app), ["Beta", "Leave"], "Cancel keeps the window");
    // The last window going quits the app.
    os::close_window(&app, &front_window(&app));
    std::thread::sleep(Duration::from_secs(1));
    press(&app, "enter");
    exited(&app, 15);
}

/// Whether the process has exited. The test holds a launched app unreaped, so an exited one
/// lingers as a zombie.
fn gone(pid: u32) -> bool {
    let out = std::process::Command::new("ps").args(["-o", "stat=", "-p", &pid.to_string()]).output().unwrap();
    let stat = String::from_utf8_lossy(&out.stdout);
    stat.trim().is_empty() || stat.starts_with('Z')
}

/// Waits for the app to exit.
fn exited(app: &AppHandle, secs: u64) {
    let pid = app.pid();
    let gone = || gone(pid);
    let deadline = Instant::now() + Duration::from_secs(secs);
    while !gone() {
        assert!(
            Instant::now() < deadline,
            "the app is still running with {} windows: {:?}",
            window_count(app),
            app.stderr_tail(30)
        );
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Sends SIGTERM and waits for the app to exit, then reads the profile's Preferences, which
/// Chromium writes out as it shuts down.
fn terminate(app: &AppHandle) -> serde_json::Value {
    let pid = app.pid().to_string();
    assert!(std::process::Command::new("kill").args(["-TERM", &pid]).status().unwrap().success());
    exited(app, 15);
    let path = app.dir().join("data/cef/Default/Preferences");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} is cut off: {e}", path.display()))
}

#[test]
fn real_sigterm_quits_cleanly() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    // Chromium marks the exit Normal only once its shutdown has flushed the profile.
    assert_eq!(terminate(&app)["profile"]["exit_type"], "Normal");
}

#[test]
fn real_profile_keeps_chrome_bubbles_off() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    let prefs = terminate(&app);
    // Save password, autofill save and translate each open a bubble off Chrome's toolbar.
    assert_eq!(prefs["credentials_enable_service"], false);
    assert_eq!(prefs["autofill"]["profile_enabled"], false);
    assert_eq!(prefs["autofill"]["credit_card_enabled"], false);
    assert_eq!(prefs["translate"]["enabled"], false);
}

#[test]
fn real_one_instance_per_profile() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    // A second launch on the same profile hands its URL over and exits.
    let mut second = app
        .command(env!("CARGO_BIN_EXE_lynk"))
        .arg(format!("{base}/beta"))
        .env("ND_APP_DATA_DIR", app.dir().join("data"))
        .env("ND_CEF_STYLE", "chrome")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = second.try_wait().unwrap() {
            break status;
        }
        if Instant::now() > deadline {
            let _ = second.kill();
            panic!("the second launch kept running");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(status.success(), "{status}");
    until(&app, "the handed-over tab", 15, |a| titles(a) == ["Alpha", "Beta"]);
    assert_eq!(window_count(&app), 1);
}

/// A key through the system's keyboard, which reaches a Chrome-style page's own window as typing
/// does; the automation socket's keys reach only the app's.
fn hid(app: &App, code: os::Code, modifiers: &[Modifier]) {
    os::key(app, code, modifiers);
    std::thread::sleep(Duration::from_millis(600));
}

#[test]
fn real_fullscreen_page_and_browser() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/full"));
    until(&app, "the page", 30, |a| titles(a) == ["Full"]);
    let width = front_window(&app).bounds.2;
    // The page's own fullscreen takes the window with it and hides the sidebar; Esc leaves.
    click_page(&app, 100.);
    until(&app, "page fullscreen", 10, |a| settled_fullscreen(a) && lacks(a, "controls-row"));
    shot(&app, "page-fullscreen");
    // The page fills the window, edge to edge. A capture of a fullscreen window also takes the
    // title bar strip parked above the screen, which is no part of the page.
    let (_, _, w, h) = front_window(&app).bounds;
    let page = app.get_by_test_id("content").bounding_box().unwrap().expect("the page is laid out");
    assert_eq!(
        (page.x, page.y, f64::from(page.width), f64::from(page.height)),
        (0., 0., w, h),
        "the page fills the fullscreen window"
    );
    hid(&app, key::ESCAPE, &[]);
    until(&app, "out of page fullscreen", 10, |a| width_of(a) == Some(width) && has(a, "controls-row"));
    browser_fullscreen(&app, width);
}

/// The browser's fullscreen: macOS's own Enter Full Screen, the sidebar kept. A page's
/// fullscreen inside it, left with Esc, leaves the window as the user had it.
#[cfg(target_os = "macos")]
fn browser_fullscreen(app: &App, width: f64) {
    hid(app, key::F, &[Modifier::Control, Modifier::Command]);
    until(app, "browser fullscreen", 10, settled_fullscreen);
    assert!(has(app, "controls-row"));
    click_page(app, 100.);
    until(app, "page fullscreen in browser fullscreen", 10, |a| lacks(a, "controls-row"));
    hid(app, key::ESCAPE, &[]);
    until(app, "out of page fullscreen", 10, |a| has(a, "controls-row"));
    // A wrong exit would start within this.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(settled_fullscreen(app), "the window stays in the user's fullscreen");
    hid(app, key::F, &[Modifier::Control, Modifier::Command]);
    until(app, "out of browser fullscreen", 10, |a| width_of(a) == Some(width));
}

/// The browser's fullscreen on Linux: F11, Chrome's, with the sidebar put away. A page's
/// fullscreen inside it, left with Esc, leaves the window in F11's fullscreen.
#[cfg(target_os = "linux")]
fn browser_fullscreen(app: &App, width: f64) {
    hid(app, key::F11, &[]);
    until(app, "browser fullscreen", 10, |a| width_of(a).is_some_and(|w| w > width) && lacks(a, "controls-row"));
    click_page(app, 100.);
    std::thread::sleep(Duration::from_millis(1000));
    hid(app, key::ESCAPE, &[]);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(front_window(app).bounds.2 > width, "the window stays in F11's fullscreen");
    assert!(lacks(app, "controls-row"), "the sidebar stays away in F11's fullscreen");
    hid(app, key::F11, &[]);
    until(app, "out of browser fullscreen", 10, |a| width_of(a) == Some(width) && has(a, "controls-row"));
}

/// The window takes no input while macOS still moves it into fullscreen, though its frame has
/// already grown; the app says it is fullscreen only once the move is over, and a click then
/// reaches the page.
#[cfg(target_os = "macos")]
#[test]
fn real_fullscreen_is_reported_once_the_move_is_over() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/full"));
    until(&app, "the page", 30, |a| titles(a) == ["Full"]);
    let width = front_window(&app).bounds.2;
    os::in_front(&app);
    // The windows' key and main state and the app in front at the keypress: the Window menu's
    // item needs a key window, for a run where macOS never takes it into fullscreen.
    let before = format!(
        "page {:?}, window {:?}, {:?}, in front: {} (lynk {})",
        box_of(&app, "content"),
        front_window(&app).bounds,
        app.windows(),
        os::front_app(),
        app.pid()
    );
    os::key(&app, key::F, &[Modifier::Control, Modifier::Command]);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut grown_while_moving = false;
    while !settled_fullscreen(&app) {
        if Instant::now() > deadline {
            shot(&app, "fullscreen-unsettled");
            panic!("the window never settled in fullscreen; at the keypress {before}; now {:?}", app.windows());
        }
        grown_while_moving |= width_of(&app).is_some_and(|w| w > width);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(grown_while_moving, "the frame grew before the move was over");
    click_page(&app, 100.);
    until(&app, "page fullscreen in browser fullscreen", 10, |a| lacks(a, "controls-row"));
    hid(&app, key::ESCAPE, &[]);
    until(&app, "out of page fullscreen", 10, |a| has(a, "controls-row"));
    assert!(settled_fullscreen(&app), "the window stays in the user's fullscreen");
    hid(&app, key::F, &[Modifier::Control, Modifier::Command]);
    until(&app, "out of browser fullscreen", 10, |a| width_of(a) == Some(width));
}

#[test]
fn real_dev_tools_from_the_page() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for (name, code, modifiers) in [("F12", key::F12, &[][..]), os::DEVTOOLS] {
        let app = launch(&format!("{base}/alpha"));
        until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
        // Focus in the page, where Chrome sees the chord first.
        click_page(&app, 500.);
        hid(&app, code, modifiers);
        until(&app, &format!("devtools from {name}"), 20, |a| has(a, "devtools-bar"));
        shot(&app, &format!("devtools-{}", name.replace('+', "-")));
        // Docked in the page area: no window of their own.
        assert_eq!(window_count(&app), 1, "DevTools open docked, not in a window");
        // The same chord from inside DevTools closes them.
        hid(&app, code, modifiers);
        until(&app, &format!("devtools closed by {name}"), 10, |a| lacks(a, "devtools-bar"));
    }
}

/// `test_id`'s box in window points.
fn box_of(app: &AppHandle, test_id: &str) -> nd_test::BoundingBox {
    app.get_by_test_id(test_id).bounding_box().unwrap().unwrap_or_else(|| panic!("{test_id} is laid out"))
}

/// A real click in the middle of `test_id`, in whichever of the app's windows has it.
fn click_id(app: &App, test_id: &str) {
    click_node(app, test_id, |n| n.test_id.as_deref() == Some(test_id), 0.5);
}

/// A real click on the first node `pick` takes, `fx` of the way across it.
fn click_node(app: &App, what: &str, pick: impl Fn(&nd_test::SemanticNode) -> bool, fx: f32) {
    // Forward, but not `in_front`: while the DevTools have the keyboard the key window is
    // Chromium's, none of the app's own.
    os::activate(app);
    let tree = app.tree(None).unwrap();
    let (frame, b) = tree
        .windows
        .iter()
        .find_map(|w| {
            let node = w.root.iter().find(|n| pick(n) && n.visible)?;
            // The frame takes in a title bar above the content the bounds are measured in.
            let content = w.root.iter().next()?.bounds?;
            let mut frame = w.frame?;
            frame.origin.y += frame.size.height - content.size.height;
            Some((frame, node.bounds?))
        })
        .unwrap_or_else(|| panic!("{what} is laid out in a window"));
    let at = (frame.origin.x + b.origin.x + b.size.width * fx, frame.origin.y + b.origin.y + b.size.height / 2.);
    os::click(app, (f64::from(at.0), f64::from(at.1)));
    std::thread::sleep(Duration::from_millis(400));
}

/// A real drag from `(x, y)` in the window's points by `(dx, dy)`.
fn real_drag(app: &App, (x, y): (f32, f32), dx: f64, dy: f64) {
    os::in_front(app);
    let (wx, wy, _, _) = front_window(app).bounds;
    let from = (wx + f64::from(x), wy + f64::from(y));
    os::hold(app, from);
    for step in 1..=30 {
        let f = f64::from(step) / 30.;
        os::move_to(app, (from.0 + dx * f, from.1 + dy * f));
        std::thread::sleep(Duration::from_millis(10));
    }
    os::release(app, (from.0 + dx, from.1 + dy));
    std::thread::sleep(Duration::from_millis(400));
}

/// Launches on `urls` under `appearance`.
fn launch_in(appearance: &str, urls: &[String]) -> App {
    let theme = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lynk-theme-{appearance}.toml"));
    std::fs::write(&theme, format!("appearance = \"{appearance}\"\n")).unwrap();
    let options = urls.iter().fold(
        LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
            .env("ND_CEF_STYLE", "chrome")
            .env("ND_THEME_TOML", theme.display().to_string()),
        |o, url| o.arg(url),
    );
    let app = os::launch(options);
    os::in_front(&app);
    app
}

/// Opens the shown tab's DevTools with F12 from inside its page, docked where the window keeps
/// them.
fn open_dev_tools(app: &App) {
    // Below every fixture's links and buttons.
    click_page(app, 450.);
    hid(app, key::F12, &[]);
    until(app, "devtools docked", 20, |a| has(a, "devtools-bar"));
    std::thread::sleep(Duration::from_millis(800));
}

/// The docked divider stops at both panes' minimums, below and beside the page, in either
/// appearance and in a 720 pt window; the bar's controls at rest, under the pointer and held
/// down. Captures land as `shell-devtools-<state>-<appearance>.png`.
#[test]
fn real_dev_tools_divider_stops_at_both_limits() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for appearance in ["dark", "light"] {
        let app = launch_in(appearance, &[format!("{base}/alpha")]);
        until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
        open_dev_tools(&app);
        shot(&app, &format!("devtools-rest-{appearance}"));
        // Under the pointer, then held down.
        os::in_front(&app);
        let (wx, wy, _, _) = front_window(&app).bounds;
        let b = box_of(&app, "devtools-dock-right");
        let at = (wx + f64::from(b.x + b.width / 2.), wy + f64::from(b.y + b.height / 2.));
        os::move_to(&app, at);
        std::thread::sleep(Duration::from_millis(600));
        shot(&app, &format!("devtools-hover-{appearance}"));
        // The side already chosen, so letting go changes nothing.
        let b = box_of(&app, "devtools-dock-bottom");
        let at = (wx + f64::from(b.x + b.width / 2.), wy + f64::from(b.y + b.height / 2.));
        os::hold(&app, at);
        std::thread::sleep(Duration::from_millis(400));
        shot(&app, &format!("devtools-pressed-{appearance}"));
        os::release(&app, at);
        assert!(has(&app, "devtools-bar"));
        // GPUI's buttons on Linux take no keyboard focus.
        #[cfg(target_os = "macos")]
        {
            app.get_by_test_id("devtools-undock").focus().unwrap();
            std::thread::sleep(Duration::from_millis(300));
            shot(&app, &format!("devtools-focused-{appearance}"));
        }

        // Below the page: grabbed 3 pt off the line on the DevTools' side, past the top, then past
        // the bottom. The page's side of the grab is the page's own window on Linux.
        let area = box_of(&app, "devtools-split");
        let line = box_of(&app, "devtools").y;
        real_drag(&app, (area.x + area.width / 2., line + 3.), 0., -2000.);
        let page = box_of(&app, "content");
        assert!((page.height - 160.).abs() <= 1., "the page keeps 160 pt: {page:?}");
        shot(&app, &format!("devtools-bottom-max-{appearance}"));
        let line = box_of(&app, "devtools").y;
        real_drag(&app, (area.x + area.width / 2., line + 3.), 0., 2000.);
        let tools = box_of(&app, "devtools");
        assert!((tools.height - 120.).abs() <= 1., "the DevTools keep 120 pt: {tools:?}");
        shot(&app, &format!("devtools-bottom-min-{appearance}"));

        // Beside the page, the same both ways.
        click_id(&app, "devtools-dock-right");
        until(&app, "docked right", 10, |a| box_of(a, "devtools").x > box_of(a, "content").x);
        let line = box_of(&app, "devtools").x;
        let bar = box_of(&app, "devtools-bar");
        real_drag(&app, (line + 3., bar.y + bar.height / 2.), -2000., 0.);
        let page = box_of(&app, "content");
        assert!((page.width - 240.).abs() <= 1., "the page keeps 240 pt: {page:?}");
        shot(&app, &format!("devtools-right-max-{appearance}"));
        let line = box_of(&app, "devtools").x;
        real_drag(&app, (line + 3., bar.y + bar.height / 2.), 2000., 0.);
        let tools = box_of(&app, "devtools");
        assert!((tools.width - 240.).abs() <= 1., "the DevTools keep 240 pt: {tools:?}");
        shot(&app, &format!("devtools-right-min-{appearance}"));

        // The smallest window keeps both panes their minimums.
        resize(&app, 720);
        until(&app, "the 720 pt layout", 10, |a| box_of(a, "devtools-split").width < 500.);
        let (page, tools) = (box_of(&app, "content"), box_of(&app, "devtools"));
        assert!(page.width >= 239. && tools.width >= 239., "both panes keep 240 pt: {page:?} {tools:?}");
        shot(&app, &format!("devtools-720-{appearance}"));
    }
}

/// Docked right, closed and opened again where they were, undocked into a window of their own
/// with a long title, and docked back from there.
#[test]
fn real_dev_tools_dock_undock_redock() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/long-title"));
    until(&app, "the page", 30, |a| titles(a).first().is_some_and(|t| t.starts_with("A page")));
    open_dev_tools(&app);
    assert!(box_of(&app, "devtools").y > box_of(&app, "content").y, "a window's first DevTools sit below");
    click_id(&app, "devtools-dock-right");
    until(&app, "docked right", 10, |a| box_of(a, "devtools").x > box_of(a, "content").x);
    // The window remembers the side.
    click_id(&app, "devtools-close");
    until(&app, "closed", 10, |a| lacks(a, "devtools-bar"));
    open_dev_tools(&app);
    assert!(box_of(&app, "devtools").x > box_of(&app, "content").x, "they come back beside the page");
    // Into a window of their own; the page takes the whole area back.
    click_id(&app, "devtools-undock");
    until(&app, "the DevTools window", 10, |a| window_count(a) == 2);
    std::thread::sleep(Duration::from_millis(1200));
    shot(&app, "devtools-window");
    assert!(lacks(&app, "devtools-split"), "the page has the area to itself");
    assert!(app.tree(None).unwrap().windows.iter().any(|w| w.title.starts_with("Developer Tools: ")));
    // Docked back from the window's own control: the window goes, the dock returns below.
    click_id(&app, "devtools-dock-bottom");
    until(&app, "redocked", 10, |a| window_count(a) == 1 && has(a, "devtools-bar"));
    assert!(box_of(&app, "devtools").y > box_of(&app, "content").y, "redocked below the page");
    shot(&app, "devtools-redocked");
}

/// DevTools belong to their tab: another tab shows without them and theirs shows with them, the
/// page's fullscreen hides them and leaving it brings them back, the browser's keeps them, and
/// closing the tab closes them.
#[test]
fn real_dev_tools_follow_their_tab() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch_with(&[format!("{base}/full"), format!("{base}/beta")]);
    until(&app, "the first page", 30, |a| titles(a).first().is_some_and(|t| t == "Full"));
    open_dev_tools(&app);
    let page = box_of(&app, "content");
    let (x, y) = row_at(&app, 1);
    real_click(&app, x, y);
    until(&app, "Beta without DevTools", 15, |a| titles(a) == ["Full", "Beta"] && lacks(a, "devtools-bar"));
    let full = box_of(&app, "content");
    assert!(full.height > page.height, "Beta has the whole area: {full:?}");
    let (x, y) = row_at(&app, 0);
    real_click(&app, x, y);
    until(&app, "Full with its DevTools", 10, |a| has(a, "devtools-bar"));
    // The page's fullscreen: the page alone, edge to edge; Esc brings the DevTools back.
    let width = front_window(&app).bounds.2;
    click_page(&app, 100.);
    until(&app, "page fullscreen", 10, |a| width_of(a).is_some_and(|w| w > width) && lacks(a, "devtools-bar"));
    std::thread::sleep(Duration::from_millis(1500));
    shot(&app, "devtools-page-fullscreen");
    hid(&app, key::ESCAPE, &[]);
    until(&app, "out of page fullscreen", 10, |a| width_of(a) == Some(width));
    until(&app, "the DevTools back", 10, |a| has(a, "devtools-bar"));
    // The browser's fullscreen keeps them docked.
    #[cfg(target_os = "macos")]
    {
        hid(&app, key::F, &[Modifier::Control, Modifier::Command]);
        until(&app, "browser fullscreen", 10, |a| width_of(a).is_some_and(|w| w > width));
        std::thread::sleep(Duration::from_millis(1500));
        assert!(has(&app, "devtools-bar"), "DevTools stay docked in the browser's fullscreen");
        shot(&app, "devtools-browser-fullscreen");
        hid(&app, key::F, &[Modifier::Control, Modifier::Command]);
        until(&app, "out of browser fullscreen", 10, |a| width_of(a) == Some(width));
    }
    // Closing the tab closes them with it.
    click_page(&app, 450.);
    hid(&app, key::W, &[os::CMD]);
    until(&app, "Full closed", 10, |a| titles(a) == ["Beta"]);
    assert!(lacks(&app, "devtools-bar"), "the closed tab's DevTools went with it");
    assert_eq!(window_count(&app), 1);
}

/// The DevTools' own title, address, icon, loads and history never reach their tab: docked, in a
/// window of their own and after they close, the tab keeps its page's title and icon, the bar its
/// address, and Back stays on offer.
#[test]
fn real_dev_tools_leave_their_tab_as_it_was() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let address = format!("{}/iconic", base.trim_start_matches("http://"));
    let app = launch(&format!("{base}/plain"));
    until(&app, "the page", 30, |a| titles(a) == ["Plain"]);
    // A second page in the tab, so it has one to go back to.
    open_bar(&app, key::L);
    type_real(&app, &format!("{base}/iconic"));
    assert_eq!(bar_field(&app), Some(format!("{base}/iconic")), "the typed address");
    hid(&app, key::ENTER, &[]);
    let iconic = |a: &App| titles(a) == ["Iconic"] && has_prefix(a, "tab-favicon-");
    until(&app, "the page with its icon", 30, iconic);
    let unchanged = |app: &App, when: &str| {
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(titles(app), ["Iconic"], "the tab's title {when}");
        assert!(has_prefix(app, "tab-favicon-"), "the tab's icon {when}");
        assert!(lacks(app, "load-bar"), "no load on the tab {when}");
        // Into the page, near its right edge, which a DevTools window of their own leaves clear;
        // the keyboard goes back to the page from the DevTools.
        click_node(app, "the page", |n| n.test_id.as_deref() == Some("content"), 0.95);
        open_bar(app, key::L);
        let field = bar_field(app).unwrap_or_default();
        assert!(field.contains(&address) && !field.contains("devtools"), "the bar {when}: {field}");
        shot(app, &format!("devtools-left-{}", when.rsplit(' ').next().unwrap_or(when)));
        // Commands come up only when they would run.
        type_real(app, "back");
        until(app, "the Back command", 10, |a| bar_rows(a).iter().any(|r| r.starts_with("Back")));
        hid(app, key::ESCAPE, &[]);
        bar_closed(app);
    };
    open_dev_tools(&app);
    unchanged(&app, "with DevTools docked");
    click_id(&app, "devtools-undock");
    until(&app, "the DevTools window", 10, |a| window_count(a) == 2);
    unchanged(&app, "with DevTools in a window");
    // F12 from the page closes them wherever they are.
    click_node(&app, "the page", |n| n.test_id.as_deref() == Some("content"), 0.95);
    hid(&app, key::F12, &[]);
    until(&app, "DevTools closed", 10, |a| window_count(a) == 1);
    unchanged(&app, "after DevTools closed");
}

/// A profile whose blocker runs on uBO 1.75.0's bundled lists with nothing due from the network,
/// and with `.promo` hidden on 127.0.0.1 by the user. The release zip comes from `LYNK_UBO_ZIP`,
/// else the test's tmp dir, which a first run fills from GitHub.
fn blocking_profile(name: &str) -> PathBuf {
    use lynk_core::ublock::{PINNED_SHA256, PINNED_VERSION, download_zip, sha256_hex};
    let dir = profile(name);
    let zip = std::env::var_os("LYNK_UBO_ZIP").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("uBlock0_{PINNED_VERSION}.chromium.zip"))
    });
    if !zip.exists() {
        let bytes = download_zip(PINNED_VERSION).unwrap_or_else(|e| panic!("{e}; set LYNK_UBO_ZIP"));
        assert_eq!(sha256_hex(&bytes), PINNED_SHA256, "the downloaded uBO {PINNED_VERSION} zip");
        std::fs::write(&zip, &bytes).unwrap();
    }
    let bytes = std::fs::read(&zip).unwrap_or_else(|e| panic!("{}: {e}; set LYNK_UBO_ZIP", zip.display()));
    let adblock = lynk_core::adblock::adblock_dir(&dir).unwrap();
    let release = adblock.join("ubo").join(lynk_core::ublock::PINNED_VERSION);
    lynk_core::ublock::extract(&bytes, &release).unwrap();
    let lists = lynk_core::ublock::default_lists(&release).unwrap();
    let now = lynk_core::now_ms();
    let store = lynk_core::adblock::open(&dir);
    store.update(|s| {
        s.last_release_check = now;
        for list in &lists {
            s.fetched.insert(list.key.clone(), now);
        }
        s.hide("127.0.0.1", ".promo", "Promo", "", now);
    });
    drop(store);
    dir
}

/// The user's procedural rules on a local page: every operator adblock-rust reports narrows to
/// its element and every action applies, while the elements beside them stay.
#[test]
fn real_blocking_runs_procedural_rules() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let dir = blocking_profile("procedural");
    let store = lynk_core::adblock::open(&dir);
    store.update(|s| {
        for rule in PROCEDURAL_RULES {
            s.hide("127.0.0.1", rule, "Procedural", "", 1);
        }
    });
    drop(store);
    let app = launch_on(&dir, &[format!("{base}/procedural")]);
    until(&app, "the page", 30, |a| titles(a).first().is_some_and(|t| t.starts_with("Procedural")));
    // The engine compiles off the main thread; the page loaded before it may have gone through.
    let deadline = Instant::now() + Duration::from_secs(90);
    while titles(&app) != ["Procedural ok"] {
        assert!(Instant::now() < deadline, "procedural rules missed, title {:?}", titles(&app));
        click_page(&app, 300.);
        hid(&app, key::R, &[os::CMD]);
        std::thread::sleep(Duration::from_secs(3));
    }
    shot(&app, "blocking-procedural");
}

/// A page whose policy refuses inline script and style still gets the user's hide rule and
/// scriptlet.
#[test]
fn real_blocking_reaches_a_page_under_a_strict_policy() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let dir = blocking_profile("csp");
    let store = lynk_core::adblock::open(&dir);
    store.update(|s| {
        s.hide("127.0.0.1", "+js(set-constant, lynkSet, 1)", "Scriptlet", "", 1);
    });
    drop(store);
    let app = launch_on(&dir, &[format!("{base}/csp")]);
    until(&app, "the page", 30, |a| titles(a).first().is_some_and(|t| t.starts_with("Csp")));
    // The engine compiles and the scriptlets load off the main thread; the page loaded before
    // them went through.
    let deadline = Instant::now() + Duration::from_secs(90);
    while titles(&app) != ["Csp none set"] {
        assert!(
            Instant::now() < deadline,
            "the policy kept the blocker out, title {:?}\n{}",
            titles(&app),
            app.stderr_tail(60).join("\n")
        );
        click_page(&app, 300.);
        hid(&app, key::R, &[os::CMD]);
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// EasyList's generic rule reaches a frame from another site, which runs in a process of its
/// own: the frame reports its classes and gets the matching rules back.
#[test]
fn real_blocking_hides_in_a_cross_site_frame() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let v6 = serve_at("[::1]:0");
    let dir = blocking_profile("framed");
    let app = launch_on(&dir, &[format!("{base}/framed?{v6}/ad-frame")]);
    until(&app, "the page", 30, |a| titles(a).first().is_some_and(|t| t.starts_with("Framed")));
    // The engine compiles off the main thread; the page loaded before it went through.
    let deadline = Instant::now() + Duration::from_secs(90);
    while titles(&app) != ["Framed none"] {
        assert!(Instant::now() < deadline, "the frame kept its ad, title {:?}", titles(&app));
        click_page(&app, 300.);
        hid(&app, key::R, &[os::CMD]);
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// The uBO lists on requests, on by default in a profile that only has them: a redirect rule
/// answers an image with its stand-in, and a frame from another site, in a process of its own,
/// has EasyList's script stopped.
#[test]
fn real_blocking_blocks_and_redirects_requests() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let v6 = serve_at("[::1]:0");
    let dir = blocking_profile("net");
    let app = launch_on(&dir, &[format!("{base}/net?{v6}/net-frame")]);
    until(&app, "the page", 30, |a| titles(a).first().is_some_and(|t| t.starts_with("Net")));
    // The engine compiles off the main thread; the page loaded before it went through.
    let deadline = Instant::now() + Duration::from_secs(90);
    while titles(&app) != ["Net swapped stopped"] {
        assert!(Instant::now() < deadline, "the requests went through, title {:?}", titles(&app));
        click_page(&app, 300.);
        hid(&app, key::R, &[os::CMD]);
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// The blocker on a local page: EasyList's requests stopped and counted in the padlock, the
/// user's rule hiding its element from the head, EasyList's generic rule held off on 127.0.0.1 by
/// its own exception and hiding its element on `[::1]`; the switch turns it all off for the site
/// once the page reloads, and on again.
#[test]
fn real_blocking_counts_hides_and_switches_off() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let dir = blocking_profile("blocking");
    let app = launch_on(&dir, &[format!("{base}/ads")]);
    until(&app, "the page", 30, |a| titles(a).first().is_some_and(|t| t.starts_with("Ads")));
    // The engine compiles off the main thread; the page loaded before it may have gone through.
    let deadline = Instant::now() + Duration::from_secs(90);
    while titles(&app) != ["Ads block none stopped"] {
        assert!(
            Instant::now() < deadline,
            "the blocker never took the page, title {:?}\n{}",
            titles(&app),
            app.stderr_tail(60).join("\n")
        );
        click_page(&app, 300.);
        hid(&app, key::R, &[os::CMD]);
        std::thread::sleep(Duration::from_secs(2));
    }
    click_id(&app, "site-info");
    until(&app, "the padlock popover", 10, |a| has(a, "site-blocking-switch"));
    assert_eq!(text_of(&app, "site-blocking-title").as_deref(), Some("Block Ads and Trackers"));
    until(&app, "the count", 10, |a| {
        text_of(a, "site-blocking-count").as_deref() == Some("2 requests blocked on this page")
    });
    shot(&app, "blocking-padlock");
    click_id(&app, "site-blocking-switch");
    until(&app, "blocking off after the reload", 20, |a| titles(a) == ["Ads block block ran"]);
    if lacks(&app, "site-blocking-switch") {
        click_id(&app, "site-info");
    }
    until(&app, "the off row", 10, |a| text_of(a, "site-blocking-count").as_deref() == Some("Off for this site"));
    shot(&app, "blocking-padlock-off");
    click_id(&app, "site-blocking-switch");
    until(&app, "blocking back on", 20, |a| titles(a) == ["Ads block none stopped"]);
    // Elsewhere the generic rule applies, and the user's rule for 127.0.0.1 does not.
    let v6 = serve_at("[::1]:0");
    open_bar(&app, key::L);
    os::type_text(&app, &format!("{v6}/ads\n"));
    until(&app, "the generic rule on [::1]", 30, |a| titles(a) == ["Ads none block stopped"]);
}

/// The element hider on a local page, in light then dark: Shift+Cmd+H outlines the element
/// under the pointer and a press hides it without the page hearing the press; Esc puts the
/// picker away, so the next press reaches the page; the element stays hidden across a reload,
/// from the head; Show Hidden Elements brings it back.
#[test]
fn real_hide_element_picks_keeps_and_restores() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for appearance in ["light", "dark"] {
        let dir = blocking_profile(&format!("hider-{appearance}"));
        let theme = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lynk-hider-{appearance}.toml"));
        std::fs::write(&theme, format!("appearance = \"{appearance}\"\n")).unwrap();
        let app = os::launch(
            profile_options(&dir, &[format!("{base}/hide")]).env("ND_THEME_TOML", theme.display().to_string()),
        );
        os::in_front(&app);
        until(&app, "the page", 30, |a| titles(a) == ["Hide shown shown"]);
        // An app that quit just before hands activation on to the next in line a moment after this
        // one came forward, and an inactive app's page hears no pointer moves.
        os::activate(&app);
        let b = app.get_by_test_id("content").bounding_box().unwrap().expect("the page is laid out");
        let (wx, wy, _, _) = front_window(&app).bounds;
        let at = |y: f32| (wx + f64::from(b.x + b.width / 2.), wy + f64::from(b.y + y));
        // The outline is the picker's blue, drawn on the hovered element's left edge; read off the
        // capture. A move can land before the picker listens, so the pointer stirs a point while
        // it waits, as a hand would.
        let outlined = |name: &str, y: f32| {
            let deadline = Instant::now() + Duration::from_secs(3);
            for stir in 0.. {
                if stir > 0 {
                    os::move_to(&app, at(y + (stir % 2) as f32));
                    std::thread::sleep(Duration::from_millis(150));
                }
                let path = shot(&app, name);
                let edge = os::pixel(&app, &front_window(&app), &path, (f64::from(b.x) + 1., f64::from(b.y + y)));
                if edge[2] >= 200 && edge[0] <= 60 {
                    return;
                }
                assert!(
                    Instant::now() < deadline,
                    "{name}: no outline on the hovered element, edge {edge:?}\n{}\n{}",
                    os::focus_report(&app),
                    app.stderr_tail(40).join("\n")
                );
                std::thread::sleep(Duration::from_millis(300));
            }
        };
        // The pointer may still rest where the last run's window was; a page that appeared under
        // it never saw it enter, and hears no moves until it does.
        os::move_to(&app, (wx - 40., wy + 40.));
        std::thread::sleep(Duration::from_millis(300));
        click_page(&app, 300.);
        hid(&app, key::H, &[os::CMD, Modifier::Shift]);
        os::move_to(&app, at(60.));
        std::thread::sleep(Duration::from_millis(300));
        os::move_to(&app, at(80.));
        outlined(&format!("hider-outline-{appearance}"), 80.);
        os::click(&app, at(80.));
        until(&app, "the banner hidden, the page none the wiser", 10, |a| titles(a) == ["Hide gone shown"]);
        os::move_to(&app, at(280.));
        std::thread::sleep(Duration::from_millis(300));
        os::move_to(&app, at(300.));
        outlined(&format!("hider-next-{appearance}"), 300.);
        hid(&app, key::ESCAPE, &[]);
        os::click(&app, at(60.));
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(titles(&app), ["Hide gone shown"], "a press after Esc hid the article");
        let rules = lynk_core::adblock::open(&dir).get().user_rules();
        assert!(rules.contains("127.0.0.1###banner"), "{rules}");
        // The engine compiles off the main thread; until it is in, a reload shows the banner.
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            hid(&app, key::R, &[os::CMD]);
            std::thread::sleep(Duration::from_secs(2));
            if titles(&app) == ["Hide gone shown"] {
                break;
            }
            assert!(Instant::now() < deadline, "the reload showed the banner, title {:?}", titles(&app));
        }
        open_bar(&app, key::K);
        type_real(&app, "show hidden");
        let rows =
            until_rows(&app, "Show Hidden Elements", |r| r.iter().any(|r| r.starts_with("Show Hidden Elements")));
        for _ in 0..rows.iter().position(|r| r.starts_with("Show Hidden Elements")).unwrap() {
            hid(&app, key::DOWN, &[]);
        }
        hid(&app, key::ENTER, &[]);
        until(&app, "the banner back after the reload", 20, |a| titles(a) == ["Hide shown shown"]);
        assert!(!lynk_core::adblock::open(&dir).get().user_rules().contains("#banner"));
        quit(&app);
    }
}

/// Alt+Cmd+U (Ctrl+U) from a focused page opens its source in a new tab right of it, ahead of the
/// tab after it, and does nothing from the source tab, which is no web page.
#[test]
fn real_view_source_opens_to_the_right() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = os::launch(
        LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
            .arg(format!("{base}/alpha"))
            .arg(format!("{base}/beta"))
            .env("ND_CEF_STYLE", "chrome"),
    );
    os::in_front(&app);
    until(&app, "two tabs", 30, |a| titles(a).len() == 2 && titles(a)[0] == "Alpha");
    click_page(&app, 200.);
    let (code, modifiers) = os::VIEW_SOURCE;
    hid(&app, code, modifiers);
    let source = format!("view-source:{}/alpha", base.trim_start_matches("http://"));
    until(&app, "the source right of its page", 20, |a| {
        let t = titles(a);
        t.len() == 3 && t[0] == "Alpha" && t[1] == source && t[2].ends_with("/beta")
    });
    shot(&app, "view-source");
    click_page(&app, 200.);
    hid(&app, code, modifiers);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(titles(&app).len(), 3, "the source tab opened a source of its own: {:?}", titles(&app));
}

/// Cmd+P (Ctrl+P) from the page, and on macOS Print from the File menu, each ask the system for a
/// print dialog for the page; dismissed, it leaves the page as it was.
#[test]
fn real_print_asks_for_the_dialog() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let (app, dialog) = os::launch_printing(
        LaunchOptions::new(env!("CARGO_BIN_EXE_lynk")).arg(format!("{base}/alpha")).env("ND_CEF_STYLE", "chrome"),
    );
    os::in_front(&app);
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    let printed = |what: &str| {
        let deadline = Instant::now() + Duration::from_secs(15);
        let title = loop {
            if let Some(title) = dialog.asked(&app) {
                break title;
            }
            if Instant::now() > deadline {
                let seen = shot(&app, "print-missing");
                panic!("no print dialog {what}, {}\n{}", seen.display(), app.stderr_tail(40).join("\n"));
            }
            std::thread::sleep(Duration::from_millis(200));
        };
        if cfg!(target_os = "linux") {
            assert_eq!(title, "Alpha", "the dialog {what} named another page");
        }
        shot(&app, &format!("print-{}", what.replace(' ', "-")));
        dialog.dismiss(&app);
        let deadline = Instant::now() + Duration::from_secs(10);
        while dialog.asked(&app).is_some() {
            assert!(Instant::now() < deadline, "the print dialog {what} stayed after Cancel");
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    click_page(&app, 200.);
    hid(&app, key::P, &[os::CMD]);
    printed("from the page");
    #[cfg(target_os = "macos")]
    {
        os::menu(&app, "File", "Print\u{2026}");
        printed("from the menu");
    }
    assert_eq!(titles(&app), ["Alpha"]);
}

/// Cmd+S (Ctrl+S) from the page asks where to save it, proposing its title as an HTML file, and
/// writes the page there as the server sends it to this profile, cookie included.
#[test]
fn real_save_page_writes_the_page_with_its_cookies() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let dir = profile("save-page");
    let into = dir.join("saved");
    std::fs::create_dir_all(&into).unwrap();
    let (app, dialog) = os::launch_saving(profile_options(&dir, &[format!("{base}/set-cookie")]), &into);
    os::in_front(&app);
    until(&app, "the cookie set", 30, |a| titles(a) == ["Set"]);
    open_bar(&app, key::L);
    os::type_text(&app, &format!("{base}/cookie\n"));
    until(&app, "the page reading its cookie", 30, |a| titles(a) == ["Cookie who=one"]);
    click_page(&app, 200.);
    hid(&app, key::S, &[os::CMD]);
    let deadline = Instant::now() + Duration::from_secs(15);
    let proposed = loop {
        if let Some(name) = dialog.save(&app) {
            break name;
        }
        if Instant::now() > deadline {
            let seen = shot(&app, "save-missing");
            panic!("no save dialog, {}\n{}", seen.display(), app.stderr_tail(40).join("\n"));
        }
        std::thread::sleep(Duration::from_millis(300));
    };
    assert_eq!(proposed, "Cookie who=one.html");
    let saved = into.join(&proposed);
    let deadline = Instant::now() + Duration::from_secs(20);
    while std::fs::read_to_string(&saved).map_or(true, |t| !t.contains("</title>")) {
        assert!(Instant::now() < deadline, "nothing saved at {}\n{}", saved.display(), app.stderr_tail(40).join("\n"));
        std::thread::sleep(Duration::from_millis(300));
    }
    assert_eq!(std::fs::read_to_string(&saved).unwrap(), "<!doctype html><title>Cookie who=one</title>");
    assert_eq!(titles(&app), ["Cookie who=one"]);
}

/// Alt+Cmd+P lifts the playing video, not the larger paused one or the first, into Chromium's
/// picture-in-picture window, the same chord brings it back, and a page with no video says so.
#[test]
fn real_float_video_lifts_the_playing_one_and_brings_it_back() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/video"));
    os::in_front(&app);
    until(&app, "the videos", 30, |a| titles(a) == ["Video none"]);
    let windows = os::every_window(&app);
    std::thread::sleep(Duration::from_secs(1));
    let (code, modifiers) = os::FLOAT;
    hid(&app, code, modifiers);
    until(&app, "the playing video floating", 15, |a| titles(a) == ["Video big"]);
    until(&app, "the picture-in-picture window", 10, |a| os::every_window(a) > windows);
    shot(&app, "float-on");
    // Hyprland gives the keyboard to the new window; the user clicks back into the page.
    click_page(&app, 500.);
    hid(&app, code, modifiers);
    until(&app, "the video back in its page", 15, |a| titles(a) == ["Video none"]);
    until(&app, "the picture-in-picture window gone", 10, |a| os::every_window(a) == windows);
    open_bar(&app, key::L);
    os::type_text(&app, &format!("{base}/alpha\n"));
    until(&app, "a page with no video", 30, |a| titles(a) == ["Alpha"]);
    hid(&app, code, modifiers);
    until(&app, "the no-video toast", 10, |a| {
        a.tree(None)
            .is_ok_and(|t| t.windows.iter().any(|w| w.root.iter().any(|n| n.name == "No video to float on this page")))
    });
}

/// Reading mode in light then dark: Shift+Cmd+R lays the article over the green page in the
/// appearance's ground, the command turns to Leave Reading Mode, Space scrolls the reader, Esc
/// leaves, a same-document navigation leaves, and a page with no article says so.
#[test]
fn real_reading_mode_enters_scrolls_and_leaves() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for (appearance, ground) in [("light", [255u8, 255, 255]), ("dark", [0x1c, 0x1c, 0x1c])] {
        let dir = profile(&format!("reader-{appearance}"));
        let theme = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lynk-reader-{appearance}.toml"));
        std::fs::write(&theme, format!("appearance = \"{appearance}\"\n")).unwrap();
        let app = os::launch(
            profile_options(&dir, &[format!("{base}/article")]).env("ND_THEME_TOML", theme.display().to_string()),
        );
        os::in_front(&app);
        until(&app, "the article", 30, |a| titles(a) == ["Article"]);
        let backdrop = |name: &str| page_backdrop(&app, &format!("reader-{name}-{appearance}"));
        let green = [0xdd, 0xff, 0xdd];
        let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3);
        let until_ground = |what: &str, want: [u8; 3]| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let got = backdrop(what);
                if near(got, want) {
                    return;
                }
                assert!(Instant::now() < deadline, "{what}: wanted {want:?}, got {got:?}");
                std::thread::sleep(Duration::from_millis(300));
            }
        };
        until_ground("page", green);
        let (code, modifiers) = os::READER;
        click_page(&app, 300.);
        hid(&app, code, modifiers);
        until_ground("on", ground);
        let before = page_column(&app, &format!("reader-top-{appearance}"));
        hid(&app, key::SPACE, &[]);
        std::thread::sleep(Duration::from_millis(800));
        let after = page_column(&app, &format!("reader-spaced-{appearance}"));
        assert_ne!(before, after, "Space left the reader where it was");
        hid(&app, key::PAGE_UP, &[]);
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(
            page_column(&app, &format!("reader-up-{appearance}")),
            before,
            "Page Up left the reader below its top"
        );
        hid(&app, key::PAGE_DOWN, &[]);
        std::thread::sleep(Duration::from_millis(800));
        let paged = page_column(&app, &format!("reader-paged-{appearance}"));
        assert_ne!(before, paged, "Page Down left the reader where it was");
        open_bar(&app, key::K);
        type_real(&app, "reading mode");
        until_rows(&app, "Leave Reading Mode", |r| r.iter().any(|r| r.starts_with("Leave Reading Mode")));
        hid(&app, key::ESCAPE, &[]);
        bar_closed(&app);
        // The bar took the keyboard; the reader has it again once the page is clicked.
        click_page(&app, 300.);
        hid(&app, key::ESCAPE, &[]);
        until_ground("left by Esc", green);
        open_bar(&app, key::K);
        type_real(&app, "reading mode");
        until_rows(&app, "Reading Mode again", |r| r.iter().any(|r| r.starts_with("Reading Mode")));
        hid(&app, key::ESCAPE, &[]);
        bar_closed(&app);
        click_page(&app, 300.);
        hid(&app, code, modifiers);
        until_ground("on again", ground);
        open_bar(&app, key::L);
        os::type_text(&app, &format!("{base}/article#later\n"));
        until_ground("left by a same-document navigation", green);
        open_bar(&app, key::L);
        os::type_text(&app, &format!("{base}/alpha\n"));
        until(&app, "a page with no article", 30, |a| titles(a) == ["Alpha"]);
        click_page(&app, 200.);
        hid(&app, code, modifiers);
        until(&app, "the no-article toast", 10, |a| {
            a.tree(None).is_ok_and(|t| {
                t.windows.iter().any(|w| w.root.iter().any(|n| n.name == "No article to read on this page"))
            })
        });
        shot(&app, &format!("reader-none-{appearance}"));
        quit(&app);
    }
}

#[test]
fn real_chrome_only_keys_open_nothing() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    click_page(&app, 500.);
    // Every window of the app on every layer, however small: F7's caret browsing prompt is a
    // small dialog, and a panel or popover sits above the normal layer.
    let windows = |app: &App| os::every_window(app);
    let before = windows(&app);
    // The count sees a window off the normal layer: a row's menu. GPUI draws its menus inside
    // the window on Linux, so there is no such window to see.
    #[cfg(target_os = "macos")]
    {
        let (wx, wy, _, _) = front_window(&app).bounds;
        let at = row_at(&app, 0);
        os::right_click(&app, (wx + f64::from(at.0), wy + f64::from(at.1)));
        until(&app, "the row menu counted", 5, |a| windows(a) > before);
        hid(&app, key::ESCAPE, &[]);
        until(&app, "the row menu gone", 5, |a| windows(a) == before);
        click_page(&app, 500.);
    }
    // Caret browsing, the task manager and help: Chrome UI with no place in Lynk.
    for (name, code, modifiers) in
        [("F7", key::F7, &[][..]), ("Shift+Esc", key::ESCAPE, &[Modifier::Shift][..]), ("F1", key::F1, &[][..])]
    {
        hid(&app, code, modifiers);
        std::thread::sleep(Duration::from_millis(900));
        assert_eq!(windows(&app), before, "{name} opened a window, dialog or panel");
        assert_eq!(titles(&app), ["Alpha"], "{name} opened a tab");
    }
    // F11 belongs to macOS (Show Desktop), so it goes through the app's own key handling only.
    // On Linux it is the browser's fullscreen, which `real_fullscreen_page_and_browser` covers.
    #[cfg(target_os = "macos")]
    {
        press(&app, "f11");
        std::thread::sleep(Duration::from_millis(900));
        assert_eq!(windows(&app), before, "F11 opened a window, dialog or panel");
        assert_eq!(titles(&app), ["Alpha"], "F11 opened a tab");
    }
}

const IDENTITY: &str = r#"<!doctype html><title>identity probe</title><body>probe<script>
(async () => {
  const frame = document.createElement("iframe");
  document.body.append(frame);
  const stock = new Set(Object.getOwnPropertyNames(frame.contentWindow));
  const extra = Object.getOwnPropertyNames(window).filter((k) => !stock.has(k) && !/^\d+$/.test(k));
  let consoleTouchedStack = false;
  const e = new Error("probe");
  Object.defineProperty(e, "stack", { get() { consoleTouchedStack = true; return ""; } });
  console.debug(e);
  const d = navigator.userAgentData;
  const high = d ? await d.getHighEntropyValues(["architecture", "bitness", "fullVersionList", "model", "platformVersion", "uaFullVersion"]) : null;
  const sub = await (await fetch("/headers")).json();
  const report = {
    webdriver: navigator.webdriver, userAgent: navigator.userAgent, languages: navigator.languages,
    brands: d ? d.brands : null, high, extraGlobals: extra, consoleTouchedStack, subresourceHeaders: sub,
  };
  await fetch("/report", { method: "POST", body: JSON.stringify(report) });
})();
</script>"#;

/// One request off `stream`: its path, lowercased headers and body.
fn read_request(stream: &std::net::TcpStream) -> (String, serde_json::Map<String, serde_json::Value>, Vec<u8>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let _ = reader.read_line(&mut line);
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let mut headers = serde_json::Map::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.trim_end().split_once(':') {
            headers.insert(name.to_ascii_lowercase(), value.trim().into());
        }
    }
    let length = headers.get("content-length").and_then(|v| v.as_str()?.parse().ok()).unwrap_or(0);
    let mut body = vec![0; length];
    let _ = std::io::Read::read_exact(&mut reader, &mut body);
    (path, headers, body)
}

#[test]
fn real_identity_holds_together() {
    if !real_allowed() {
        return;
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/probe", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut document = serde_json::Map::new();
        for stream in listener.incoming().map_while(Result::ok) {
            let mut stream = stream;
            let (path, headers, body) = read_request(&stream);
            let (kind, extra, body) = match path.as_str() {
                "/probe" => {
                    document = headers;
                    let hints = "Sec-CH-UA, Sec-CH-UA-Full-Version-List, Sec-CH-UA-Platform, Sec-CH-UA-Platform-Version, Sec-CH-UA-Arch, Sec-CH-UA-Bitness, Sec-CH-UA-Model";
                    ("text/html", format!("Accept-CH: {hints}\r\n"), IDENTITY.to_owned())
                }
                "/headers" => ("application/json", String::new(), serde_json::Value::Object(headers).to_string()),
                "/report" => {
                    let report: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
                    let _ = tx.send((report, document.clone()));
                    ("text/plain", String::new(), "ok".to_owned())
                }
                _ => ("text/plain", String::new(), String::new()),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    // Launched as a user launches it: no automation socket, a throwaway profile.
    let data = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("identity-{}", std::process::id()));
    let display = os::Display::start();
    let mut app = display
        .command(env!("CARGO_BIN_EXE_lynk"))
        .env("ND_APP_DATA_DIR", &data)
        .env("LYNK_URL", &url)
        .env("ND_CEF_STYLE", "chrome")
        .env_remove("ND_AUTOMATION")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let reported = rx.recv_timeout(Duration::from_secs(60));
    let _ = std::process::Command::new("kill").args(["-TERM", &app.id().to_string()]).status();
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.try_wait().unwrap().is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = app.kill();
    let _ = app.wait();
    let _ = std::fs::remove_dir_all(&data);
    let (report, document) = reported.expect("the page never reported");

    let ua = report["userAgent"].as_str().unwrap_or_default();
    let major = ua.split("Chrome/").nth(1).and_then(|v| v.split('.').next()).unwrap_or_default().to_owned();
    let brands = report["brands"].as_array().cloned().unwrap_or_default();
    let name = |b: &serde_json::Value| b["brand"].as_str().unwrap_or_default().to_owned();
    let header = |map: &serde_json::Map<String, serde_json::Value>, key: &str| {
        map.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_owned()
    };
    let sub = report["subresourceHeaders"].as_object().cloned().unwrap_or_default();
    let mut problems = Vec::new();
    if report["webdriver"] != false {
        problems.push(format!("navigator.webdriver is {}", report["webdriver"]));
    }
    if ua.to_ascii_lowercase().contains("headless")
        || brands.iter().any(|b| name(b).to_ascii_lowercase().contains("headless"))
    {
        problems.push("a headless product in the UA or brands".to_owned());
    }
    if major.is_empty() {
        problems.push(format!("no Chrome/<major> in the UA: {ua}"));
    }
    if !brands.iter().any(|b| name(b) == "Chromium") {
        problems.push(format!("brands lack Chromium: {brands:?}"));
    }
    for b in &brands {
        let brand = name(b).to_ascii_lowercase();
        if !(brand.contains("not") && brand.contains("brand")) && b["version"] != major.as_str() {
            problems.push(format!("brand {} is v{}, the UA says {major}", name(b), b["version"]));
        }
    }
    let list = report["high"]["fullVersionList"].as_array().cloned().unwrap_or_default();
    if !list
        .iter()
        .any(|b| name(b) == "Chromium" && b["version"].as_str().is_some_and(|v| v.starts_with(&format!("{major}."))))
    {
        problems.push("fullVersionList disagrees with the UA".to_owned());
    }
    if !header(&document, "sec-ch-ua").contains(&format!("\"Chromium\";v=\"{major}\"")) {
        problems.push(format!("the sec-ch-ua header disagrees: {}", header(&document, "sec-ch-ua")));
    }
    if !header(&sub, "sec-ch-ua-full-version-list").contains(&format!("\"Chromium\";v=\"{major}.")) {
        problems.push("the full version list hint disagrees with the UA".to_owned());
    }
    if header(&document, "user-agent") != ua {
        problems.push("the user-agent header differs from navigator.userAgent".to_owned());
    }
    if header(&document, "accept-language").is_empty() {
        problems.push("no accept-language header".to_owned());
    }
    if report["languages"].as_array().is_none_or(Vec::is_empty) {
        problems.push("navigator.languages is empty".to_owned());
    }
    if report["extraGlobals"].as_array().is_some_and(|g| !g.is_empty()) {
        problems.push(format!("globals the page did not define: {}", report["extraGlobals"]));
    }
    if report["consoleTouchedStack"] == true {
        problems.push("console.debug serialised an error's stack (a devtools Runtime session on the page)".to_owned());
    }
    assert!(problems.is_empty(), "{}\n{report}", problems.join("; "));
}

/// The one window's tab rows, top to bottom, by tab id. `None` when the tree cannot be read,
/// the app has other than one window, or no row is drawn, so a failed read never counts.
fn window_rows(app: &AppHandle) -> Option<(nd_test::SemanticWindow, Vec<String>)> {
    let [window] = <[_; 1]>::try_from(app.tree(None).ok()?.windows).ok()?;
    let mut rows: Vec<(f32, String)> = window
        .root
        .iter()
        .filter(|n| n.visible)
        .filter_map(|n| Some((n.bounds?.origin.y, n.test_id.as_deref()?.strip_prefix("tab-row-")?.to_owned())))
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let rows: Vec<String> = rows.into_iter().map(|r| r.1).collect();
    (!rows.is_empty()).then_some((window, rows))
}

/// How many pages the window holds. `None` unless [`window_rows`] reads the window.
fn live_pages(app: &AppHandle) -> Option<usize> {
    let (window, _) = window_rows(app)?;
    Some(window.root.iter().filter(|n| page_of(n).is_some()).count())
}

/// The tab id of a page node, `page-{tab}-{page}`.
fn page_of(n: &nd_test::SemanticNode) -> Option<&str> {
    let (tab, page) = n.test_id.as_deref()?.strip_prefix("page-")?.split_once('-')?;
    (tab.parse::<u64>().is_ok() && page.parse::<u64>().is_ok()).then_some(tab)
}

/// Whether the tab in the `index`th row is asleep: its row still drawn and no page left for it.
fn asleep(app: &AppHandle, index: usize) -> bool {
    window_rows(app).is_some_and(|(window, rows)| {
        rows.get(index).is_some_and(|tab| !window.root.iter().any(|n| page_of(n) == Some(tab.as_str())))
    })
}

/// Puts the first tab, on show, to sleep from the menus: the menu bar's Tabs menu on macOS, the
/// tab row's menu on Linux, which has no menu bar.
fn sleep_first_tab(app: &App) {
    #[cfg(target_os = "macos")]
    os::menu(app, "Tabs", "Put Tab to Sleep");
    #[cfg(target_os = "linux")]
    pick_menu(app, row_at(app, 0), "Put Tab to Sleep");
    std::thread::sleep(Duration::from_millis(500));
}

#[test]
fn real_first_click_reaches_a_new_page() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    // nd-cef lifts Chromium's view into the page as the page loads, and the move sizes it to
    // nothing; left so, it takes no clicks until nd-cef's relift timer puts the size back,
    // which nd-cef logs at debug. Each launch is a fresh page.
    for _ in 0..3 {
        let options = LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
            .env("LYNK_URL", format!("{base}/press"))
            .env("ND_CEF_STYLE", "chrome")
            .env("ND_LOG", "info,nd_cef=debug");
        let app = os::launch(options);
        let deadline = Instant::now() + Duration::from_secs(30);
        while titles(&app) != ["Press"] {
            assert!(Instant::now() < deadline, "the page never loaded");
            std::thread::sleep(Duration::from_millis(10));
        }
        let b = app.get_by_test_id("content").bounding_box().unwrap().expect("the page is laid out");
        // Chromium drops input that reaches a page before its first paint, so the click waits
        // for the page under it, as a user's would.
        let at = (b.x + b.width / 2., b.y + 100.);
        until(&app, "the page drawn under the click", 10, |a| {
            let w = front_window(a);
            let path = shot(a, "first-click");
            let [r, g, b] = os::pixel(a, &w, &path, (f64::from(at.0), f64::from(at.1)));
            g > r.saturating_add(16) && g > b.saturating_add(16)
        });
        real_click(&app, at.0, at.1);
        until(&app, "the first click on the page", 5, |a| titles(a) == ["Pressed"]);
        let resized: Vec<String> =
            app.stderr_tail(400).into_iter().filter(|l| l.contains("lifted page view was resized")).collect();
        assert!(resized.is_empty(), "the page's view was left at the wrong size: {resized:?}");
    }
}

#[test]
fn real_sleep_keeps_the_scroll() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/long"));
    until(&app, "the page", 30, |a| titles(a) == ["Long"]);
    click_page(&app, 100.);
    until(&app, "the page scrolled", 15, |a| titles(a).first().is_some_and(|t| t.starts_with("At 600 ")));
    let before = titles(&app)[0].clone();
    open_in_new_tab(&app, &format!("{base}/beta"));
    press(&app, "cmd-1");
    sleep_first_tab(&app);
    until(&app, "the tab asleep", 15, |a| asleep(a, 0) && live_pages(a) == Some(1));
    // Woken, it loads again and goes back to where it was scrolled.
    press(&app, "cmd-1");
    until(&app, "the scroll back in a new load", 15, |a| {
        titles(a).first().is_some_and(|t| t.starts_with("At 600 ") && *t != before)
    });
}

#[test]
fn real_busy_page_still_sleeps() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/busy"));
    until(&app, "the page", 30, |a| titles(a) == ["Busy"]);
    click_page(&app, 100.);
    open_in_new_tab(&app, &format!("{base}/beta"));
    // A renderer that never answers where it is scrolled does not keep its tab awake.
    std::thread::sleep(Duration::from_secs(1));
    let at = row_at(&app, 0);
    pick_menu(&app, at, "Put Tab");
    until(&app, "the busy tab asleep", 5, |a| asleep(a, 0) && live_pages(a) == Some(1));
}

#[test]
fn real_launch_urls_open_as_tabs() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let options = LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
        .arg(format!("{base}/alpha"))
        .arg(format!("{base}/beta"))
        .env("ND_CEF_STYLE", "chrome");
    let app = os::launch(options);
    // The second loads once it is first shown; until then its row shows its address.
    until(&app, "a tab per URL", 30, |a| {
        let t = titles(a);
        t.len() == 2 && t[0] == "Alpha" && t[1].ends_with("/beta")
    });
}

#[test]
fn real_put_tab_to_sleep_and_wake() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    click_page(&app, 300.);
    until(&app, "a second tab", 15, |a| titles(a) == ["Alpha", "Beta"] && live_pages(a) == Some(2));
    // A sleeping tab keeps its row and gives its page back; selecting it wakes it.
    press(&app, "cmd-1");
    sleep_first_tab(&app);
    until(&app, "the tab asleep", 15, |a| asleep(a, 0) && live_pages(a) == Some(1));
    assert_eq!(titles(&app), ["Alpha", "Beta"]);
    shot(&app, "tab-asleep");
    press(&app, "cmd-1");
    until(&app, "the tab awake", 15, |a| live_pages(a) == Some(2));
}

/// Right-clicks `at` (window points) and picks the item whose title starts with `item`, typed
/// into the menu as a user selects by typing.
fn pick_menu(app: &App, at: (f32, f32), item: &str) {
    os::in_front(app);
    let (wx, wy, _, _) = front_window(app).bounds;
    let before = os::open_menus(app);
    os::right_click(app, (wx + f64::from(at.0), wy + f64::from(at.1)));
    // Typed only once the menu is up: letters sent to a menu still opening are lost,
    // and type-select then lands on another item.
    let deadline = Instant::now() + Duration::from_secs(10);
    while os::open_menus(app) <= before {
        if Instant::now() >= deadline {
            shot(app, "menu-never-opened");
            panic!("the menu for {item} never opened at {at:?}; rows {:?}", rows(app));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    for c in item.to_ascii_lowercase().chars() {
        os::key(app, os::key_of(c), &[]);
        std::thread::sleep(Duration::from_millis(60));
    }
    hid(app, key::ENTER, &[]);
}

/// The title of the app's first window, which is the shown tab's.
fn shown_title(app: &AppHandle) -> String {
    let windows = app.windows().unwrap();
    windows[0]["title"].as_str().unwrap_or_default().to_owned()
}

/// The first window's tab rows top to bottom: test id and centre, in window points.
fn rows(app: &AppHandle) -> Vec<(String, (f32, f32))> {
    let tree = app.tree(None).unwrap_or_else(|e| panic!("{e:?}\n{}", os::main_thread_of(app.pid())));
    let mut rows: Vec<_> = tree.windows[0]
        .root
        .iter()
        .filter(|n| n.visible)
        .filter_map(|n| {
            let r = n.bounds?;
            let id = n.test_id.clone().filter(|t| t.starts_with("tab-row-"))?;
            Some((id, (r.origin.x + r.size.width / 2., r.origin.y + r.size.height / 2.)))
        })
        .collect();
    rows.sort_by(|a, b| a.1.1.total_cmp(&b.1.1));
    rows
}

/// The centre of the sidebar's `index`th tab row, in window points.
fn row_at(app: &AppHandle, index: usize) -> (f32, f32) {
    rows(app)[index].1
}

/// The sidebar answers the keyboard and assistive tech, not only the pointer: the list's arrow
/// keys pick a tab, and a row's close button presses with Space.
#[test]
fn real_sidebar_controls_answer_the_keyboard() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch_with(&[format!("{base}/alpha"), format!("{base}/beta")]);
    // The second tab loads only once shown.
    until(&app, "both rows", 30, |a| titles(a).len() == 2 && shown_title(a) == "Alpha");
    let (key, other) = ("down", "Beta");
    app.get_by_test_id("tabs").press(key).unwrap();
    until(&app, "the other tab, picked by key", 10, |a| shown_title(a) == other);
    let close =
        |a: &AppHandle| rows(a).into_iter().map(|r| r.0.replace("tab-row-", "tab-close-")).find(|id| has(a, id));
    until(&app, "the shown row's close button", 10, |a| close(a).is_some());
    let button = app.get_by_test_id(&close(&app).unwrap());
    // GPUI's buttons take no keyboard focus, so on Linux the press is assistive tech's.
    if cfg!(target_os = "macos") {
        button.press("space").unwrap()
    } else {
        button.click().unwrap()
    }
    until(&app, "the shown tab, closed by key", 10, |a| titles(a).len() == 1);
}

#[test]
fn real_tab_row_menu() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    pick_menu(&app, row_at(&app, 0), "Duplicate");
    until(&app, "the duplicate", 15, |a| titles(a) == ["Alpha", "Alpha"]);
    open_in_new_tab(&app, &format!("{base}/beta"));
    // A right click on another tab's row opens its menu and leaves the shown tab alone.
    let (wx, wy, _, _) = front_window(&app).bounds;
    let at = row_at(&app, 0);
    os::right_click(&app, (wx + f64::from(at.0), wy + f64::from(at.1)));
    std::thread::sleep(Duration::from_millis(600));
    hid(&app, key::ESCAPE, &[]);
    assert_eq!(shown_title(&app), "Beta", "the shown tab after a menu on another row");
    // The row's menu acts on its own tab, not the one on show.
    let duplicate = rows(&app)[1].0.clone();
    pick_menu(&app, row_at(&app, 1), "Close Other");
    until(&app, "only the second tab", 15, |a| titles(a) == ["Alpha"]);
    assert_eq!(rows(&app).into_iter().map(|r| r.0).collect::<Vec<_>>(), [duplicate], "the duplicate is the one left");
    // Off any tab: the column's menu.
    let last = row_at(&app, titles(&app).len() - 1);
    pick_menu(&app, (last.0, last.1 + 160.), "New Tab");
    until(&app, "the bar from the column menu", 10, |a| bar_field(a).is_some());
    type_real(&app, &format!("{base}/beta"));
    hid(&app, key::ENTER, &[]);
    until(&app, "a tab from the column menu", 10, |a| titles(a).len() == 2);
}

#[test]
fn real_fifty_tabs_opened_and_closed_at_speed() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    // Each through the command bar, typed as soon as the bar shows.
    for n in 0..50 {
        open_bar(&app, key::T);
        os::type_text(&app, &format!("{base}/beta?{n}\n"));
    }
    until(&app, "the fifty tabs", 60, |a| live_pages(a) == Some(51));
    let keys = app.keyboard();
    // Fifty closes leave exactly the first tab only if all fifty opened: one too few would take
    // the window, and the app with it.
    for _ in 0..50 {
        keys.press(&os::chord("cmd-w")).unwrap();
    }
    until(&app, "back to the first tab", 30, |a| titles(a) == ["Alpha"]);
}

#[test]
fn real_window_of_thirty_live_tabs_closes() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let urls: Vec<String> = (0..30).map(|n| format!("{base}/beta?{n}")).collect();
    let app = launch_with(&urls);
    // The list is virtual, so its rows' order in the tree is not the tabs' order.
    until(&app, "the first page", 30, |a| titles(a).iter().any(|t| t == "Beta"));
    shot(&app, "thirty");
    // Next Tab through all of them gives each its page.
    for _ in 0..29 {
        app.keyboard().press(&os::chord("cmd-shift-]")).unwrap();
    }
    until(&app, "thirty live pages", 60, |a| live_pages(a) == Some(30));
    // The last window closing quits.
    os::close_window(&app, &front_window(&app));
    exited(&app, 30);
}

#[test]
fn real_sigterm_during_a_load_still_flushes_the_profile() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/slow"));
    until(&app, "the load under way", 30, |a| has(a, "load-bar"));
    assert_eq!(terminate(&app)["profile"]["exit_type"], "Normal");
}

#[test]
fn real_renderer_crash_shows_aw_snap() {
    if !real_allowed() {
        return;
    }
    let app = launch("chrome://crash");
    until(&app, "Aw, Snap!", 30, |a| has(a, "crash-page"));
    shot(&app, "crashed");
    // The app itself carries on.
    open_in_new_tab(&app, "data:text/html,<title>After</title>");
    until(&app, "a new tab", 10, |a| titles(a).len() == 2);
}

#[test]
fn real_unreachable_addresses_show_the_error_page() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    // A DNS failure, a refused connection (as offline), a redirect loop, an IDN host that does
    // not resolve.
    let urls = [
        "http://lynk-drive.invalid/".to_owned(),
        "http://127.0.0.1:1/".to_owned(),
        format!("{base}/loop"),
        "http://\u{4f8b}\u{3048}.\u{30c6}\u{30b9}\u{30c8}/".to_owned(),
    ];
    let app = launch_with(&urls);
    for n in 1..=urls.len() {
        press(&app, &format!("cmd-{n}"));
        until(&app, &format!("the error page for {}", urls[n - 1]), 30, |a| has(a, "error-page"));
    }
    shot(&app, "error-page");
}

#[test]
fn real_odd_but_valid_pages_load() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let file = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lynk-file-page.html");
    std::fs::write(&file, "<!doctype html><title>File</title>on disk").unwrap();
    let long_query = "a".repeat(4000);
    let urls = [
        format!("{base}/missing"),
        "data:text/html,<title>Data</title>inline".to_owned(),
        format!("file://{}", file.display()),
        format!("{base}/beta?{long_query}"),
        format!("{base}/\u{fc}n\u{ef}c\u{f6}d\u{e9}?q=\u{65e5}\u{672c}"),
        format!("{base}/big"),
    ];
    let app = launch_with(&urls);
    // A 404 is the site's own page, not the app's error.
    let expect = ["Missing", "Data", "File", "Beta", "Missing", "Big"];
    for (n, title) in expect.iter().enumerate() {
        press(&app, &format!("cmd-{}", n + 1));
        until(&app, &format!("{title} loaded"), 60, |a| titles(a).get(n).is_some_and(|t| t == title));
        assert!(lacks(&app, "error-page"), "{title} is no error");
    }
}

#[test]
fn real_popup_loop_is_held_off() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/popups"));
    until(&app, "the page", 30, |a| titles(a) == ["Popups"]);
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(titles(&app), ["Popups"], "pop-ups without a click open nothing");
    assert_eq!(window_count(&app), 1);
}

#[test]
fn real_find_with_ten_thousand_matches_and_none() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/many"));
    until(&app, "the page", 30, |a| titles(a) == ["Many"]);
    press(&app, "cmd-f");
    until(&app, "the find bar", 10, |a| has(a, "find-bar"));
    app.get_by_test_id("find-field").type_text("needle").unwrap();
    until(&app, "every match counted", 30, |a| text_of(a, "find-count").as_deref() == Some("10000 matches"));
    app.get_by_test_id("find-field").fill("nothing like it").unwrap();
    until(&app, "no matches", 10, |a| text_of(a, "find-count").as_deref() == Some("No matches"));
}

#[test]
fn real_zoom_stops_at_both_limits() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    for _ in 0..15 {
        press(&app, "cmd-=");
    }
    until(&app, "500%", 10, |a| text_of(a, "zoom-value").as_deref() == Some("500%"));
    // Every level seen on the way down, so a stop says where and what had key.
    let mut seen: Vec<Option<String>> = Vec::new();
    for _ in 0..25 {
        press(&app, "cmd--");
        let value = text_of(&app, "zoom-value");
        if seen.last() != Some(&value) {
            seen.push(value);
        }
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut value = seen.last().cloned().flatten();
    while value.as_deref() != Some("25%") {
        let windows = app.windows().ok().and_then(|w| w.as_array().cloned()).unwrap_or_default();
        let key: Vec<_> = windows.iter().map(|w| (w["title"].clone(), w["key"].clone())).collect();
        assert!(Instant::now() < deadline, "the zoom stopped at {value:?} after {seen:?}, windows {key:?}");
        std::thread::sleep(Duration::from_millis(150));
        value = text_of(&app, "zoom-value");
    }
}

#[test]
fn real_back_and_forward_spammed_during_loads() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    click_page(&app, 100.);
    until(&app, "Beta", 15, |a| titles(a) == ["Beta"]);
    let keys = app.keyboard();
    for _ in 0..20 {
        keys.press(&os::chord("cmd-[")).unwrap();
        keys.press(&os::chord("cmd-]")).unwrap();
    }
    keys.press(&os::chord("cmd-[")).unwrap();
    until(&app, "settled on Alpha", 15, |a| titles(a) == ["Alpha"]);
}

#[test]
fn real_resize_to_the_minimum_during_a_load() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/slow"));
    until(&app, "the load under way", 30, |a| has(a, "load-bar"));
    os::resize(&app, 600, 400);
    let (_, _, w, h) = front_window(&app).bounds;
    assert_eq!((w, h), (720., 480.), "the window stops at its minimum size");
    assert!(has(&app, "controls-row") && has(&app, "load-bar"));
    shot(&app, "minimum");
}

#[test]
fn real_closing_a_fullscreen_page_leaves_fullscreen() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch_with(&[format!("{base}/full"), format!("{base}/beta")]);
    until(&app, "the page", 30, |a| titles(a).first().is_some_and(|t| t == "Full"));
    let width = front_window(&app).bounds.2;
    click_page(&app, 100.);
    until(&app, "page fullscreen", 10, settled_fullscreen);
    press(&app, "cmd-w");
    until(&app, "the tab gone and the window back", 15, |a| titles(a).len() == 1 && width_of(a) == Some(width));
    assert!(has(&app, "controls-row"));
}

#[test]
fn real_tab_moves_between_windows_without_a_reload() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch_with(&[format!("{base}/long"), format!("{base}/alpha")]);
    until(&app, "the first page", 30, |a| titles(a).first().is_some_and(|t| t == "Long"));
    click_page(&app, 100.);
    until(&app, "the page scrolled", 15, |a| titles(a).first().is_some_and(|t| t.starts_with("At 600 ")));
    let stamped = titles(&app)[0].clone();
    pick_menu(&app, row_at(&app, 0), "Move Tab to New");
    until(&app, "a second window", 10, |a| window_count(a) == 2);
    let moved =
        |a: &App| a.tree(None).is_ok_and(|t| t.windows.iter().any(|w| w.root.iter().any(|n| n.name == stamped)));
    until(&app, "the moved page as it was", 10, moved);
    let left = |a: &App| a.tree(None).is_ok_and(|t| t.windows.iter().any(|w| window_titles(w) == ["Alpha"]));
    until(&app, "the other tab left behind", 5, left);
}

/// The colour the page shows near its bottom right corner, where nothing of its own is drawn,
/// read from a capture of the window.
/// A column of the page's pixels down its middle, read off a fresh capture, to tell two apart.
fn page_column(app: &App, name: &str) -> Vec<[u8; 3]> {
    let path = shot(app, name);
    let b = app.get_by_test_id("content").bounding_box().unwrap().expect("the page is laid out");
    let win = front_window(app);
    (1..40)
        .map(|i| {
            os::pixel(app, &win, &path, (f64::from(b.x + b.width / 2.), f64::from(b.y + b.height * i as f32 / 40.)))
        })
        .collect()
}

fn page_backdrop(app: &App, name: &str) -> [u8; 3] {
    let path = shot(app, name);
    let b = app.get_by_test_id("content").bounding_box().unwrap().expect("the page is laid out");
    os::pixel(app, &front_window(app), &path, (f64::from(b.x + b.width) - 40., f64::from(b.y + b.height) - 40.))
}

#[test]
fn real_a_page_without_colours_sits_on_white() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/plain"));
    until(&app, "the plain page", 30, |a| titles(a) == ["Plain"]);
    std::thread::sleep(Duration::from_secs(1));
    let plain = page_backdrop(&app, "page-plain");
    assert!(plain.iter().all(|c| *c >= 250), "a page with no colours of its own is white, got {plain:?}");
}

/// A page that declares its colour scheme keeps Chromium's canvas for it: dark for `dark` and for
/// `light dark` under a dark appearance, white for `light dark` under a light one.
#[test]
fn real_a_page_with_a_colour_scheme_keeps_its_canvas() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for (appearance, path, title, dark) in [
        ("dark", "/dark-scheme", "Dark", true),
        ("dark", "/both-schemes", "Both", true),
        ("light", "/both-schemes", "Both", false),
    ] {
        let theme = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lynk-theme-{appearance}.toml"));
        std::fs::write(&theme, format!("appearance = \"{appearance}\"\n")).unwrap();
        let options = LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
            .env("ND_CEF_STYLE", "chrome")
            .env("ND_THEME_TOML", theme.display().to_string())
            .arg(format!("{base}{path}"));
        let app = os::launch(options);
        os::in_front(&app);
        until(&app, "the page", 30, |a| titles(a) == [title]);
        std::thread::sleep(Duration::from_secs(1));
        let canvas = page_backdrop(&app, &format!("page-scheme-{title}-{appearance}"));
        let ok = if dark { canvas.iter().all(|c| *c <= 40) } else { canvas.iter().all(|c| *c >= 250) };
        assert!(ok, "{path} under a {appearance} appearance, got {canvas:?}");
    }
}

/// A `light dark` page follows the appearance when it flips under the open page.
#[test]
fn real_a_light_dark_page_follows_an_appearance_flip() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let theme = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lynk-theme-flip.toml");
    std::fs::write(&theme, "appearance = \"light\"\n").unwrap();
    let options = LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
        .env("ND_CEF_STYLE", "chrome")
        .env("ND_THEME_TOML", theme.display().to_string())
        .arg(format!("{base}/both-schemes"));
    let app = os::launch(options);
    os::in_front(&app);
    until(&app, "the page", 30, |a| titles(a) == ["Both"]);
    std::thread::sleep(Duration::from_secs(1));
    let light = page_backdrop(&app, "page-flip-light");
    assert!(light.iter().all(|c| *c >= 250), "light first, got {light:?}");
    for (appearance, dark) in [("dark", true), ("light", false)] {
        std::fs::write(&theme, format!("appearance = \"{appearance}\"\n")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let canvas = page_backdrop(&app, &format!("page-flip-{appearance}"));
            let ok = if dark { canvas.iter().all(|c| *c <= 40) } else { canvas.iter().all(|c| *c >= 250) };
            if ok {
                break;
            }
            assert!(Instant::now() < deadline, "after a flip to {appearance}, got {canvas:?}");
        }
    }
}

/// The sidebar's rows in every state at once, in each appearance and at the minimum width:
/// shown, under the pointer, loading, asleep, a long title, no favicon, the New Tab row and the
/// foot, then a row held down. Captures land as `shell-sidebar-states-<appearance>[-720].png`
/// and `shell-sidebar-pressed-<appearance>.png`.
#[test]
fn real_sidebar_draws_every_row_state() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for appearance in ["dark", "light"] {
        let theme = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lynk-theme-{appearance}.toml"));
        std::fs::write(&theme, format!("appearance = \"{appearance}\"\n")).unwrap();
        let options = ["/alpha", "/long-title", "/slow", "/beta"]
            .iter()
            .fold(LaunchOptions::new(env!("CARGO_BIN_EXE_lynk")).env("ND_CEF_STYLE", "chrome"), |o, path| {
                o.arg(format!("{base}{path}"))
            })
            .env("ND_THEME_TOML", theme.display().to_string());
        let app = os::launch(options);
        until(&app, "the first page", 30, |a| titles(a).first().is_some_and(|t| t == "Alpha"));
        // The slow tab keeps loading behind the shown one; Beta loads, then sleeps.
        press(&app, "cmd-3");
        until(&app, "the never-ending load", 10, |a| has(a, "load-bar"));
        press(&app, "cmd-4");
        until(&app, "Beta", 15, |a| titles(a).get(3).is_some_and(|t| t == "Beta"));
        press(&app, "cmd-2");
        until(&app, "the long title", 15, |a| titles(a).get(1).is_some_and(|t| t.starts_with("A page")));
        press(&app, "cmd-1");
        pick_menu(&app, row_at(&app, 3), "Put Tab");
        until(&app, "Beta asleep", 15, |a| asleep(a, 3) && live_pages(a) == Some(3));
        for suffix in ["", "-720"] {
            if suffix == "-720" {
                resize(&app, 720);
            }
            os::in_front(&app);
            let (wx, wy, _, _) = front_window(&app).bounds;
            let at = row_at(&app, 1);
            os::move_to(&app, (wx + f64::from(at.0), wy + f64::from(at.1)));
            std::thread::sleep(Duration::from_millis(600));
            shot(&app, &format!("sidebar-states-{appearance}{suffix}"));
            sidebar_geometry_holds(&app);
        }
        // A row held down gives a little.
        let (wx, wy, _, _) = front_window(&app).bounds;
        let at = row_at(&app, 1);
        let at = (wx + f64::from(at.0), wy + f64::from(at.1));
        os::hold(&app, at);
        std::thread::sleep(Duration::from_millis(400));
        shot(&app, &format!("sidebar-pressed-{appearance}"));
        os::release(&app, at);
    }
}

/// The column's numbers, read off the shown row: 28 tall, the margin in from the window's left edge and
/// the sidebar's trailing edge, the close button inside its trailing slot with air on every side,
/// and the title ending before it.
fn sidebar_geometry_holds(app: &AppHandle) {
    let bounds =
        |id: &str| app.get_by_test_id(id).bounding_box().unwrap().unwrap_or_else(|| panic!("{id} is laid out"));
    let row_id = rows(app)[0].0.clone();
    let id = row_id.strip_prefix("tab-row-").unwrap();
    let (row, close, title) = (bounds(&row_id), bounds(&format!("tab-close-{id}")), bounds(&format!("tab-title-{id}")));
    let margin = lynk::sidebar::margin();
    assert_eq!((row.x, row.width, row.height), (margin, 240. - 2. * margin, 28.), "the shown row");
    let (gap_right, gap_top) = (row.x + row.width - (close.x + close.width), close.y - row.y);
    assert!(gap_right >= 4. && gap_top >= 4., "the close button keeps clear of the row's edges: {close:?} in {row:?}");
    assert!(title.x + title.width <= close.x, "the title stops before the close button: {title:?}, {close:?}");
}

/// Quitting and launching again brings back the window where it was and as big, its tabs in
/// order with their titles, the tab it showed, and each page's scroll; only the shown tab loads
/// until another is shown. A launch with an address adds it to the restored window.
#[test]
fn real_a_relaunch_restores_the_session() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let profile = profile("session-restore");
    let urls: Vec<String> = ["/alpha", "/long", "/beta"].iter().map(|p| format!("{base}{p}")).collect();
    let app = launch_on(&profile, &urls);
    until(&app, "three tabs", 30, |a| rows(a).len() == 3 && titles(a)[0] == "Alpha");
    // The app places and sizes its window on macOS; on Linux the compositor does.
    #[cfg(target_os = "macos")]
    {
        os::move_window(&app, 180, 140);
        os::resize(&app, 1100, 760);
        std::thread::sleep(Duration::from_millis(800));
    }
    let (x, y) = row_at(&app, 1);
    real_click(&app, x, y);
    until(&app, "the long page", 30, |a| titles(a)[1] == "Long");
    click_page(&app, 300.);
    until(&app, "the long page scrolled", 15, |a| titles(a)[1].starts_with("At 600 "));
    let (x, y) = row_at(&app, 2);
    real_click(&app, x, y);
    until(&app, "beta shown", 30, |a| shown_title(a) == "Beta");
    let before = front_window(&app).bounds;
    std::thread::sleep(Duration::from_secs(1));
    quit(&app);

    let saved = session_of(&profile);
    let window = &saved["windows"][0];
    let tabs = window["tabs"].as_array().unwrap();
    let saved_urls: Vec<&str> = tabs.iter().map(|t| t["url"].as_str().unwrap()).collect();
    assert_eq!(saved_urls, urls.iter().map(String::as_str).collect::<Vec<_>>(), "{saved}");
    assert_eq!(window["activeId"], tabs[2]["id"]);
    assert_eq!(tabs[1]["scroll"], serde_json::json!([0.0, 600.0]), "{saved}");
    #[cfg(target_os = "macos")]
    assert_eq!((window["x"].as_f64(), window["y"].as_f64()), (Some(before.0), Some(before.1)), "{saved}");
    drop(app);

    let app = launch_on(&profile, &[]);
    until(&app, "the restored tabs", 30, |a| rows(a).len() == 3);
    let restored = titles(&app);
    assert_eq!(restored[0], "Alpha");
    assert!(restored[1].starts_with("At 600 "), "{restored:?}");
    assert_eq!(restored[2], "Beta");
    until(&app, "beta shown again", 30, |a| shown_title(a) == "Beta");
    assert_eq!(live_pages(&app), Some(1), "only the shown tab loads");
    let after = front_window(&app).bounds;
    #[cfg(target_os = "macos")]
    assert!(
        (after.0 - before.0).abs() < 2. && (after.1 - before.1).abs() < 2. && (after.2 - before.2).abs() < 2.,
        "the window came back at {after:?}, not {before:?}"
    );
    let _ = (before, after);
    let stamp = restored[1].clone();
    let (x, y) = row_at(&app, 1);
    real_click(&app, x, y);
    until(&app, "the long page back at its scroll", 30, |a| {
        let title = &titles(a)[1];
        title.starts_with("At 600 ") && *title != stamp
    });
    quit(&app);
    drop(app);

    let app = launch_on(&profile, &[format!("{base}/press")]);
    until(&app, "the restored tabs and the launch's", 30, |a| rows(a).len() == 4);
    until(&app, "the launch's page", 30, |a| titles(a).last().is_some_and(|t| t == "Press"));
    quit(&app);
}

/// Writes `windows` as the profile's session, version 3.
fn write_session(profile: &std::path::Path, windows: serde_json::Value) {
    let doc = serde_json::json!({ "version": 3, "data": { "windows": windows } });
    std::fs::write(profile.join("session.json"), doc.to_string()).unwrap();
}

/// One window of `n` tabs at `base/alpha?<i>`, titled `T<i>`, showing tab `shown`.
fn many_tabs(base: &str, n: usize, shown: usize) -> serde_json::Value {
    let tabs: Vec<_> = (1..=n)
        .map(|i| serde_json::json!({ "id": format!("t{i}"), "url": format!("{base}/alpha?{i}"), "title": format!("T{i}") }))
        .collect();
    serde_json::json!([{ "id": "w1", "tabs": tabs, "activeId": format!("t{shown}"), "width": 1280, "height": 800 }])
}

fn saved_urls(profile: &std::path::Path) -> Vec<String> {
    session_of(profile)["windows"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|w| w["tabs"].as_array().unwrap().iter().map(|t| t["url"].as_str().unwrap().to_owned()))
        .collect()
}

/// A session cut short, not JSON or written by a newer version is set aside whole, and the
/// browser starts fresh on the launch's address.
#[test]
fn real_a_broken_or_newer_session_starts_fresh() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for (name, text) in [
        ("cut", r#"{"version":3,"data":{"windows":[{"id":"w1","tabs":[{"id":"t1","url":"#),
        ("garbage", "\u{0}\u{1}not json"),
        ("newer", r#"{"version":99,"data":{"windows":[]}}"#),
    ] {
        let profile = profile(&format!("session-{name}"));
        std::fs::write(profile.join("session.json"), text).unwrap();
        let app = launch_on(&profile, &[format!("{base}/alpha")]);
        until(&app, "a fresh window", 30, |a| titles(a) == ["Alpha"]);
        let aside = std::fs::read_dir(&profile)
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| e.file_name().to_string_lossy().starts_with("session.json.corrupt-"))
            .unwrap_or_else(|| panic!("{name}: the broken session was not set aside"));
        assert_eq!(std::fs::read_to_string(aside.path()).unwrap(), text, "{name}: kept as it was");
        quit(&app);
        drop(app);
        assert_eq!(saved_urls(&profile), [format!("{base}/alpha")], "{name}");
    }
}

/// A quit inside the store's debounce still writes the change that came just before it.
#[test]
fn real_a_quit_writes_the_last_change() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let profile = profile("session-quit");
    let app = launch_on(&profile, &[format!("{base}/alpha")]);
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    open_in_new_tab(&app, &format!("{base}/beta"));
    until(&app, "the new tab written", 10, |_| {
        profile.join("session.json").exists()
            && session_of(&profile)["windows"][0]["tabs"].as_array().map(Vec::len) == Some(2)
    });
    // Shown by the next key, then quit before the store's debounce has passed. The key can
    // reach the page first and come back to the app later, so the quit waits for the switch.
    app.keyboard().press(&os::chord("cmd-1")).unwrap();
    until(&app, "the first tab shown", 5, |a| shown_title(a) == "Alpha");
    quit(&app);
    drop(app);
    let saved = session_of(&profile);
    let window = &saved["windows"][0];
    assert_eq!(window["activeId"], window["tabs"][0]["id"], "{saved}");
}

/// `kill -9` while changes are being written leaves the last session written whole: it reads
/// and restores every time.
#[test]
fn real_kill_9_never_leaves_a_broken_session() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let profile = profile("session-kill");
    let mut expected = 1;
    let mut app = launch_on(&profile, &[format!("{base}/alpha")]);
    for round in 0..4 {
        until(&app, "the restored tabs", 30, |a| rows(a).len() == expected);
        for n in 0..3 {
            open_in_new_tab(&app, &format!("{base}/beta?{round}-{n}"));
        }
        expected += 3;
        until(&app, "the new tabs", 10, |a| rows(a).len() == expected);
        // Mid-way through the store's debounce and write, at a different point each round.
        std::thread::sleep(Duration::from_millis(150 + 120 * round));
        let pid = app.pid().to_string();
        assert!(std::process::Command::new("kill").args(["-KILL", &pid]).status().unwrap().success());
        exited(&app, 10);
        drop(app);
        let saved = session_of(&profile);
        let tabs = saved["windows"][0]["tabs"].as_array().unwrap().len();
        assert!(tabs == expected || tabs == expected - 3, "round {round}: {tabs} tabs saved: {saved}");
        expected = tabs;
        app = launch_on(&profile, &[]);
    }
    until(&app, "the last restore", 30, |a| rows(a).len() == expected);
    quit(&app);
}

/// Two hundred tabs come back asleep, only the shown one loading, and a quit while they are
/// still coming up loses none of them.
#[test]
fn real_two_hundred_tabs_restore_asleep() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let profile = profile("session-200");
    write_session(&profile, many_tabs(&base, 200, 100));
    let urls = saved_urls(&profile);
    let app = launch_on(&profile, &[]);
    until(&app, "a window", 30, |a| window_count(a) == 1);
    quit(&app);
    drop(app);
    assert_eq!(saved_urls(&profile), urls, "the quit during the restore kept every tab in order");

    let started = Instant::now();
    let app = launch_on(&profile, &[]);
    until(&app, "the shown tab", 30, |a| shown_title(a) == "Alpha");
    let took = started.elapsed();
    assert_eq!(live_pages(&app), Some(1), "only the shown tab loads");
    assert!(took < Duration::from_secs(10), "two hundred tabs took {took:?} to come up");
    quit(&app);
    drop(app);
    assert_eq!(saved_urls(&profile), urls);
    assert_eq!(session_of(&profile)["windows"][0]["activeId"], "t100");
}

/// A tab whose address no longer answers comes back, shows why it did not load, and stays.
#[test]
fn real_a_restored_tab_that_no_longer_loads_stays() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let profile = profile("session-dead");
    let dead = "http://127.0.0.1:9/gone".to_owned();
    write_session(
        &profile,
        serde_json::json!([{ "id": "w1", "activeId": "t2", "width": 1280, "height": 800, "tabs": [
            { "id": "t1", "url": format!("{base}/alpha"), "title": "Alpha" },
            { "id": "t2", "url": dead, "title": "Gone" },
        ] }]),
    );
    let app = launch_on(&profile, &[]);
    until(&app, "the error page", 30, |a| has(a, "error-page"));
    assert_eq!(rows(&app).len(), 2);
    quit(&app);
    drop(app);
    assert_eq!(saved_urls(&profile), [format!("{base}/alpha"), dead]);
}

/// Two launches at the same moment on one profile: one browser comes up with the session and
/// the other's address, and the session holds each tab once.
/// macOS only for now: on Linux the second launch has to reach the first's private display.
#[cfg(target_os = "macos")]
#[test]
fn real_two_launches_at_once_share_one_session() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let profile = profile("session-twice");
    write_session(&profile, many_tabs(&base, 2, 1));
    let other = std::process::Command::new(env!("CARGO_BIN_EXE_lynk"))
        .arg(format!("{base}/beta"))
        .env("ND_APP_DATA_DIR", &profile)
        .env("ND_CEF_STYLE", "chrome")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut other = Reaped(other);
    let other = &mut other.0;
    // Either launch can take the profile. This one can open its automation socket and still
    // lose, hand its address over and quit, so the one left running is the one that won.
    let this = nd_test::launch_app(profile_options(&profile, &[format!("{base}/press")])).ok();
    let deadline = Instant::now() + Duration::from_secs(20);
    let won = loop {
        let this_gone = this.as_ref().is_none_or(|a| gone(a.pid()));
        if this_gone {
            break None;
        }
        if other.try_wait().unwrap().is_some() {
            break this;
        }
        assert!(
            Instant::now() < deadline,
            "both launches kept running\nthe other:\n{}\nthis one:\n{}",
            os::main_thread_of(other.id()),
            this.as_ref().map(|a| os::main_thread_of(a.pid())).unwrap_or_default()
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    match won {
        Some(app) => {
            let app = os::App::from(app);
            os::in_front(&app);
            until(&app, "both launches' tabs", 30, |a| rows(a).len() == 4);
            quit(&app);
        }
        None => {
            // The other runs without automation, so its session file says when both tabs are in.
            let deadline = Instant::now() + Duration::from_secs(30);
            while saved_urls(&profile).len() < 4 {
                assert!(Instant::now() < deadline, "the session never held both launches' tabs");
                std::thread::sleep(Duration::from_millis(250));
            }
            let _ = std::process::Command::new("kill").args(["-TERM", &other.id().to_string()]).status();
            let _ = other.wait();
        }
    }
    let mut urls = saved_urls(&profile);
    urls.sort();
    let mut expected: Vec<String> =
        ["/alpha?1", "/alpha?2", "/beta", "/press"].iter().map(|p| format!("{base}{p}")).collect();
    expected.sort();
    assert_eq!(urls, expected);
}

/// A child process killed when the case ends, a failed one included.
#[cfg(target_os = "macos")]
struct Reaped(std::process::Child);

#[cfg(target_os = "macos")]
impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The main display's frame in global points, `(x, y, width, height)`.
#[cfg(target_os = "macos")]
fn main_display() -> (f64, f64, f64, f64) {
    let script = "ObjC.import('AppKit'); const f = $.NSScreen.screens.objectAtIndex(0).frame; \
                  [f.origin.x, f.origin.y, f.size.width, f.size.height].join(',')";
    let out = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", script]).output().unwrap();
    let n: Vec<f64> = String::from_utf8_lossy(&out.stdout).split(',').map(|v| v.trim().parse().unwrap()).collect();
    (n[0], n[1], n[2], n[3])
}

/// A window kept on a display that is gone comes back on one that is there.
#[cfg(target_os = "macos")]
#[test]
fn real_a_window_from_a_display_that_is_gone_comes_back_on_screen() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let profile = profile("session-display");
    write_session(
        &profile,
        serde_json::json!([{ "id": "w1", "activeId": "t1", "width": 1000, "height": 700, "x": 30000, "y": -20000,
            "tabs": [{ "id": "t1", "url": format!("{base}/alpha"), "title": "Alpha" }] }]),
    );
    let app = launch_on(&profile, &[]);
    until(&app, "the window", 30, |a| on_screen_window(a).is_some());
    let (x, y, w, h) = front_window(&app).bounds;
    let screen = main_display();
    assert!(
        x >= screen.0 && y >= screen.1 && x + w <= screen.0 + screen.2 + 1. && y + h <= screen.1 + screen.3 + 1.,
        "the window is at {:?}, off the display {screen:?}",
        (x, y, w, h)
    );
    assert!((w - 1000.).abs() < 2. && (h - 700.).abs() < 30., "it kept its size: {w}x{h}");
    quit(&app);
}

/// The command bar's field as its search box reads, `None` while the bar is closed. A completion
/// it shows follows a `|`.
fn bar_field(app: &AppHandle) -> Option<String> {
    let tree = app.tree(None).ok()?;
    tree.windows.iter().find_map(|w| {
        w.root.iter().find(|n| format!("{:?}", n.role) == "SearchBox").map(|n| n.value.clone().unwrap_or_default())
    })
}

/// Opens the bar with Cmd (Ctrl on Linux) and `code` (T, L, K) on the system's keyboard and waits
/// for it.
#[track_caller]
fn open_bar(app: &App, code: os::Code) {
    os::in_front(app);
    hid(app, code, &[os::CMD]);
    bar_wait(app, true);
}

#[track_caller]
fn bar_closed(app: &App) {
    bar_wait(app, false);
}

/// Waits for the bar to be open or closed, with a capture and the app's windows when it is not.
#[track_caller]
fn bar_wait(app: &App, open: bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while bar_field(app).is_some() != open {
        if Instant::now() > deadline {
            shot(app, "bar-timeout");
            panic!(
                "the bar never {}: windows {:?}, keys {:?}, app said {:#?}",
                if open { "opened" } else { "closed" },
                os::windows(app),
                app.windows(),
                app.stderr_tail(25)
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Types on the system's keyboard, character by character, as a keyboard would.
fn type_real(app: &App, text: &str) {
    os::type_text(app, text);
    std::thread::sleep(Duration::from_millis(400));
}

/// Pastes `text` with Cmd+V (Ctrl+V on Linux), from the real clipboard.
fn paste(app: &App, text: &str) {
    os::set_clipboard(app, text);
    hid(app, key::V, &[os::CMD]);
}

/// Opens `url` in a new tab through the bar, as typed.
#[track_caller]
fn open_in_new_tab(app: &App, url: &str) {
    let before = titles(app).len();
    open_bar(app, key::T);
    type_real(app, url);
    hid(app, key::ENTER, &[]);
    bar_closed(app);
    until(app, "the new tab", 15, |a| titles(a).len() == before + 1);
}

/// The shown tab's address, as the bar Cmd+L opens on it holds it.
#[track_caller]
fn address(app: &App) -> String {
    open_bar(app, key::L);
    let field = bar_field(app).unwrap_or_default();
    hid(app, key::ESCAPE, &[]);
    bar_closed(app);
    field
}

/// Types `text` over a Cmd+L selection and goes there with Return.
#[track_caller]
fn go_to(app: &App, text: &str) {
    open_bar(app, key::L);
    type_real(app, text);
    hid(app, key::ENTER, &[]);
    bar_closed(app);
}

/// The bar's panel shows a turn after the bar opens, later still while a load keeps the main
/// thread busy. Keys typed the instant the bar is in the tree all land in its field, none in the
/// page behind.
#[test]
fn real_command_bar_keeps_keys_typed_during_a_load() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    for n in 0..20 {
        go_to(&app, &format!("{base}/slow?{n}"));
        until(&app, "the never-ending load", 10, |a| has(a, "load-bar"));
        open_bar(&app, key::L);
        let typed = format!("{base}/alpha?{n}");
        os::type_text(&app, &typed);
        let deadline = Instant::now() + Duration::from_secs(5);
        while bar_field(&app).and_then(|f| f.split('|').next().map(str::to_owned)).as_deref() != Some(typed.as_str()) {
            assert!(
                Instant::now() < deadline,
                "round {n}: the field holds {:?}\n{}",
                bar_field(&app),
                app.stderr_tail(60).join("\n")
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        hid(&app, key::ESCAPE, &[]);
        bar_closed(&app);
    }
    assert_eq!(titles(&app).len(), 1, "no search for keys the page took");
}

/// The bar's field as typed, without a completion it shows.
fn bar_typed(app: &App) -> String {
    bar_field(app).and_then(|f| f.split('|').next().map(str::to_owned)).unwrap_or_default()
}

/// A 300-character address stays inside the bar's field, with the caret keys, typing at its end
/// and a 2,000-character paste.
#[test]
fn real_command_bar_long_address_stays_in_its_field() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let long = format!("{base}/beta?{}", "segment/".repeat(36));
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    go_to(&app, &long);
    until(&app, "Beta", 15, |a| titles(a) == ["Beta"]);

    // Right of the card, a field that does not clip shows the address's tail.
    open_bar(&app, key::T);
    type_real(&app, "b");
    let edge = |what: &str| {
        shot(&app, what);
        #[cfg(target_os = "linux")]
        return os::right_edge(&app, &front_window(&app), front_window(&app).bounds.2 * 0.2, 3);
        #[cfg(not(target_os = "linux"))]
        Vec::<[u8; 3]>::new()
    };
    let clear = edge("bar-long-reference");
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
    let clipped = |what: &str| {
        let seen = edge(what);
        let off =
            clear.iter().zip(&seen).filter(|(a, b)| a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > 24)).count();
        assert_eq!(off, 0, "{what}: {off} pixels right of the bar changed");
    };

    open_bar(&app, key::L);
    let address = bar_typed(&app);
    assert!(address.len() >= 300, "the bar holds the address: {address:?}");
    clipped("bar-long-selected");
    hid(&app, os::LINE_END.0, os::LINE_END.1);
    clipped("bar-long-end");
    type_real(&app, "x");
    assert_eq!(bar_typed(&app), format!("{address}x"), "the line's end, then typing, adds at the end");
    hid(&app, os::LINE_START.0, os::LINE_START.1);
    type_real(&app, "y");
    hid(&app, key::RIGHT, &[]);
    type_real(&app, "z");
    hid(&app, key::LEFT, &[]);
    hid(&app, key::LEFT, &[]);
    type_real(&app, "w");
    assert_eq!(
        bar_typed(&app),
        format!("yw{}z{}x", &address[..1], &address[1..]),
        "the line's start, Right and Left move the caret"
    );
    clipped("bar-long-home");

    let pasted = format!("{base}/beta?{}", "p".repeat(2000 - base.len() - 6));
    hid(&app, key::A, &[os::CMD]);
    paste(&app, &pasted);
    until(&app, "the pasted address", 5, |a| bar_typed(a) == pasted);
    clipped("bar-long-pasted");
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
    quit(&app);
}

/// A 300-character address in the bar: the caret keys, typing at its end and a 2,000-character
/// paste edit it where the caret is. The line's ends are Cmd+arrows on macOS, as in any AppKit
/// field, and Home and End elsewhere.
#[test]
fn real_command_bar_long_address_edits_at_the_caret() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let long = format!("{base}/beta?{}", "segment/".repeat(36));
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    go_to(&app, &long);
    until(&app, "Beta", 15, |a| titles(a) == ["Beta"]);

    open_bar(&app, key::L);
    let address = bar_typed(&app);
    assert!(address.len() >= 300, "the bar holds the address: {address:?}");
    shot(&app, "bar-long-selected");
    hid(&app, os::LINE_END.0, os::LINE_END.1);
    shot(&app, "bar-long-end");
    type_real(&app, "x");
    assert_eq!(bar_typed(&app), format!("{address}x"), "the line's end, then typing, adds at the end");
    hid(&app, os::LINE_START.0, os::LINE_START.1);
    type_real(&app, "y");
    hid(&app, key::RIGHT, &[]);
    type_real(&app, "z");
    hid(&app, key::LEFT, &[]);
    hid(&app, key::LEFT, &[]);
    type_real(&app, "w");
    assert_eq!(
        bar_typed(&app),
        format!("yw{}z{}x", &address[..1], &address[1..]),
        "the line's start, Right and Left move the caret"
    );
    shot(&app, "bar-long-home");

    let pasted = format!("{base}/beta?{}", "p".repeat(2000 - base.len() - 6));
    hid(&app, key::A, &[os::CMD]);
    paste(&app, &pasted);
    until(&app, "the pasted address", 5, |a| bar_typed(a) == pasted);
    shot(&app, "bar-long-pasted");
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
    quit(&app);
}

#[test]
fn real_command_bar_opens_sites() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let port = base.rsplit(':').next().unwrap().to_owned();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    let google = |q: &str| lynk_core::url::to_url(q, "https://www.google.com/search?q=").unwrap();

    // Cmd+T is a bar, not a tab: Escape mid-typing leaves no blank tab behind, and neither does
    // Return on an empty field.
    open_bar(&app, key::T);
    type_real(&app, "half typ");
    shot(&app, "bar-typing");
    assert_eq!(bar_field(&app).as_deref(), Some("half typ"));
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
    open_bar(&app, key::T);
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    assert_eq!(titles(&app), ["Alpha"], "no dead blank tab");

    // An IP with a port opens in a new tab; Cmd+L shows its address, all of it.
    open_in_new_tab(&app, &format!("127.0.0.1:{port}/beta"));
    until(&app, "Beta in its own tab", 15, |a| titles(a) == ["Alpha", "Beta"]);
    assert_eq!(address(&app), format!("{base}/beta"));

    // Cmd+L, typing over the selection: localhost with a port loads in the same tab.
    go_to(&app, &format!("localhost:{port}/alpha"));
    until(&app, "Alpha over Beta", 15, |a| titles(a) == ["Alpha", "Alpha"]);
    assert_eq!(address(&app), format!("http://localhost:{port}/alpha"));

    // A full address with its scheme.
    go_to(&app, &format!("{base}/beta"));
    until(&app, "Beta again", 15, |a| titles(a) == ["Alpha", "Beta"]);

    // A search with spaces and punctuation goes to the configured engine, as one query. The engine
    // rewrites its own address once it answers, so the tab is read as it leaves.
    let query = "rust lang: what's new? & why/how #2";
    go_to(&app, query);
    let searched = address(&app);
    assert!(searched == google(query) || searched.starts_with("https://www.google.com/"), "{searched}");
    until(&app, "the search results", 30, |a| titles(a).get(1).is_some_and(|t| t.starts_with(query)));

    // A bare domain and an internationalised one are addresses, not searches.
    go_to(&app, "example.com");
    until(&app, "the bare domain", 15, |a| address(a).starts_with("https://example.com"));
    // The compositor's virtual keyboard types no letters outside its US layout, so Linux pastes.
    if cfg!(target_os = "macos") {
        go_to(&app, "b\u{fc}cher.de");
    } else {
        open_bar(&app, key::L);
        paste(&app, "b\u{fc}cher.de");
        hid(&app, key::ENTER, &[]);
        bar_closed(&app);
    }
    until(&app, "the IDN host's page", 30, |a| titles(a).get(1).is_some_and(|t| t.contains("b\u{fc}cher.de")));

    // data: and file: addresses load as typed.
    go_to(&app, "data:text/html,<title>Typed data</title>hi");
    until(&app, "the data page", 15, |a| titles(a) == ["Alpha", "Typed data"]);
    let file = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lynk-typed-file.html");
    std::fs::write(&file, "<!doctype html><title>Typed file</title>on disk").unwrap();
    go_to(&app, &format!("file://{}", file.display()));
    until(&app, "the file page", 15, |a| titles(a) == ["Alpha", "Typed file"]);

    // Pasted text keeps no trailing newline, and a 2000 character address goes in whole.
    open_bar(&app, key::L);
    paste(&app, &format!("{base}/beta?pasted\n"));
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    until(&app, "the pasted address", 15, |a| address(a) == format!("{base}/beta?pasted"));
    let long = format!("{base}/beta?{}", "q".repeat(2000 - base.len() - 6));
    assert_eq!(long.len(), 2000);
    open_bar(&app, key::L);
    paste(&app, &long);
    shot(&app, "bar-long-url");
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    until(&app, "the long address", 15, |a| address(a) == long);

    // Typing while the page is still loading takes over from the load.
    go_to(&app, &format!("{base}/slow"));
    until(&app, "the never-ending load", 10, |a| has(a, "load-bar"));
    go_to(&app, &format!("{base}/alpha"));
    until(&app, "Alpha over the slow load", 15, |a| titles(a) == ["Alpha", "Alpha"]);

    // A host that does not resolve shows the error page, and the bar still works over it.
    go_to(&app, "http://lynk-drive.invalid/");
    until(&app, "the error page", 30, |a| has(a, "error-page"));
    go_to(&app, &format!("{base}/beta"));
    until(&app, "Beta after the error", 15, |a| titles(a) == ["Alpha", "Beta"] && !has(a, "error-page"));

    // A click on the shown tab's row opens the bar on its address.
    let (x, y) = row_at(&app, 1);
    real_click(&app, x, y);
    bar_wait(&app, true);
    assert_eq!(bar_field(&app), Some(format!("{base}/beta")));
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);

    // A second Cmd+L puts the bar away.
    open_bar(&app, key::L);
    hid(&app, key::L, &[os::CMD]);
    bar_closed(&app);
    drop(app);

    // On the new tab page, Cmd+T loads in place: a blank tab has nothing to keep.
    let app = launch("");
    until(&app, "the new tab page", 30, |a| has(a, "new-tab-page"));
    open_bar(&app, key::T);
    type_real(&app, &format!("{base}/beta"));
    hid(&app, key::ENTER, &[]);
    until(&app, "Beta in the blank tab", 15, |a| titles(a) == ["Beta"]);
}

/// Another app taking focus leaves the bar open, and coming back gives its field the keys again,
/// as it had them.
/// macOS only for now: the other app is Finder.
#[cfg(target_os = "macos")]
#[test]
fn real_command_bar_survives_an_app_switch() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    open_bar(&app, key::T);
    type_real(&app, "lynk-");
    let finder = std::process::Command::new("pgrep").args(["-x", "Finder"]).output().unwrap();
    let finder: u32 = String::from_utf8_lossy(&finder.stdout).trim().parse().expect("Finder runs");
    ndshell::input::activate(finder);
    let away =
        |a: &App| a.windows().ok().is_some_and(|w| w.as_array().is_some_and(|ws| ws.iter().all(|w| w["key"] != true)));
    until(&app, "another app in front", 10, away);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(bar_field(&app).as_deref(), Some("lynk-"), "the bar stays open while away");
    // The bar's panel takes key back, not the window under it.
    ndshell::input::activate(app.pid());
    std::thread::sleep(Duration::from_millis(600));
    type_real(&app, "back");
    assert_eq!(bar_field(&app).as_deref(), Some("lynk-back"), "the field has the keys again");
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
    assert_eq!(titles(&app), ["Alpha"]);
}

/// A page in a window behind loads a site of its own while the command bar of the window in
/// front has the keyboard: Chromium takes key for the page's widget, and it goes back to the bar
/// in front, not to the window behind.
#[test]
fn real_command_bar_keeps_the_keys_while_a_window_behind_loads() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let port = base.rsplit(':').next().unwrap().to_owned();
    let app = launch(&format!("{base}/later?http%3A%2F%2Flocalhost%3A{port}%2Fbeta"));
    until(&app, "the page", 30, |a| titles(a) == ["Later"]);
    press(&app, "cmd-n");
    until(&app, "a second window", 10, |a| window_count(a) == 2);
    open_bar(&app, key::T);
    type_real(&app, "lynk-");
    let behind_loaded = |a: &App| a.tree(None).is_ok_and(|t| t.windows.iter().any(|w| window_titles(w) == ["Beta"]));
    until(&app, "the window behind on its new site", 20, behind_loaded);
    std::thread::sleep(Duration::from_secs(1));
    type_real(&app, "back");
    assert_eq!(bar_field(&app).as_deref(), Some("lynk-back"), "the bar in front kept the keys");
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
}

/// The bar's rows as assistive tech reads them ("title, subtitle, hint"), top first.
/// Waits until the bar's rows pass `check`, failing with the rows it shows.
#[track_caller]
fn until_rows(app: &App, what: &str, check: impl Fn(&[String]) -> bool) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let rows = bar_rows(app);
        if check(&rows) {
            return rows;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}: field {:?}, rows {rows:#?}", bar_field(app));
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn bar_rows(app: &AppHandle) -> Vec<String> {
    let Ok(tree) = app.tree(None) else { return Vec::new() };
    // The rows are parts of the palette that share its bounds, so the tree's order is theirs.
    tree.windows
        .iter()
        .find(|w| w.root.iter().any(|n| format!("{:?}", n.role) == "SearchBox"))
        .map(|w| w.root.iter().filter(|n| format!("{:?}", n.role) == "ListItem").map(|n| n.name.clone()).collect())
        .unwrap_or_default()
}

/// History, open tabs and commands come up as you type: the address completes inline from the
/// places you have been, the keys and the pointer pick rows, and an open tab is switched to rather
/// than opened twice.
#[test]
fn real_command_bar_suggests() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let host = base.trim_start_matches("http://").to_owned();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    // Two places in history, and Beta open in a second tab.
    go_to(&app, &format!("{base}/plain"));
    until(&app, "Plain", 15, |a| titles(a) == ["Plain"]);
    open_in_new_tab(&app, &format!("{base}/beta"));
    until(&app, "Beta", 15, |a| titles(a) == ["Plain", "Beta"]);

    // Inline completion from history: the host and its port, shown selected after what was typed.
    let port = base.rsplit(':').next().unwrap().to_owned();
    open_bar(&app, key::T);
    type_real(&app, "127.0.0");
    until(&app, "the completed host", 5, |a| bar_field(a) == Some(format!("127.0.0|.1:{port}")));
    // Backspace takes the completion away, and it stays away until the next letter.
    hid(&app, key::BACKSPACE, &[]);
    until(&app, "the completion gone", 5, |a| bar_field(a).as_deref() == Some("127.0.0"));
    type_real(&app, ".");
    until(&app, "completing again", 5, |a| bar_field(a) == Some(format!("127.0.0.|1:{port}")));
    // Tab takes it; then a path finishes from history.
    hid(&app, key::TAB, &[]);
    until(&app, "the host taken", 5, |a| bar_field(a).as_deref() == Some(host.as_str()));
    type_real(&app, "/pl");
    until(&app, "the path completed", 5, |a| bar_field(a) == Some(format!("{host}/pl|ain")));
    let rows = bar_rows(&app);
    assert!(rows.first().is_some_and(|r| r.starts_with(&format!("{host}/plain"))), "{rows:?}");
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    until(&app, "the completed address in a new tab", 15, |a| titles(a) == ["Plain", "Beta", "Plain"]);

    // An open tab is offered as a switch: Down, then Return, shows it instead of opening it again.
    open_bar(&app, key::L);
    type_real(&app, "Bet");
    until(&app, "the open tab offered", 5, |a| {
        bar_rows(a).iter().any(|r| r.starts_with("Beta") && r.ends_with("Switch to Tab"))
    });
    let rows = bar_rows(&app);
    let at = rows.iter().position(|r| r.ends_with("Switch to Tab")).unwrap();
    for _ in 0..at {
        hid(&app, key::DOWN, &[]);
    }
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    until(&app, "Beta shown", 10, |a| shown_title(a) == "Beta");
    assert_eq!(titles(&app), ["Plain", "Beta", "Plain"], "switched, not opened again");

    // Cmd+K with nothing typed: the first row is the tab shown before, and Return goes back.
    open_bar(&app, key::K);
    let rows = bar_rows(&app);
    assert!(rows.first().is_some_and(|r| r.starts_with("Plain") && r.ends_with("Switch to Tab")), "{rows:?}");
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    until(&app, "back to the tab before", 10, |a| shown_title(a) == "Plain");

    // A click on a row runs it.
    open_bar(&app, key::K);
    let rows = bar_rows(&app);
    let at = rows.iter().position(|r| r.starts_with("Beta") && r.ends_with("Switch to Tab")).expect("Beta offered");
    // The card sits at 18% of the window; under it the field (about 26 pt), the gaps and the
    // separator (37 pt), then 40 pt rows. Not `real_click`: the bar's panel has key, not the
    // window it waits for.
    let (wx, wy, _, wh) = front_window(&app).bounds;
    os::click(&app, (wx + 600., wy + (wh * 0.18).round() + 26. + 37. + 40. * at as f64 + 20.));
    bar_closed(&app);
    until(&app, "Beta by the pointer", 10, |a| shown_title(a) == "Beta");

    // Typing fast while the rows come back from history keeps every letter, in order.
    open_bar(&app, key::T);
    type_real(&app, &format!("{host}/beta?fast-typing"));
    assert_eq!(bar_field(&app), Some(format!("{host}/beta?fast-typing")));
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
}

/// The bar's search boxes across the app's windows, with their placeholders.
fn bar_boxes(app: &App) -> Vec<Option<String>> {
    let Ok(tree) = app.tree(None) else { return Vec::new() };
    tree.windows
        .iter()
        .flat_map(|w| w.root.iter().filter(|n| format!("{:?}", n.role) == "SearchBox").map(|n| n.placeholder.clone()))
        .collect()
}

/// Opening the bar while it is open starts it over in the new mode and never stacks a second
/// bar; Cmd+L puts it away instead, and a bar put away leaves no tab behind.
#[test]
fn real_command_bar_reopens_in_place() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    let switcher = Some("Switch to a tab or run a command".to_owned());
    let address = Some("Search or enter address".to_owned());

    open_bar(&app, key::T);
    type_real(&app, "half");
    assert_eq!(bar_field(&app).as_deref(), Some("half"));
    hid(&app, key::K, &[os::CMD]);
    until(&app, "the switcher, empty", 5, |a| {
        bar_field(a).as_deref() == Some("") && bar_boxes(a) == [switcher.clone()]
    });
    type_real(&app, "tab");
    hid(&app, key::T, &[os::CMD]);
    until(&app, "the address bar, empty", 5, |a| {
        bar_field(a).as_deref() == Some("") && bar_boxes(a) == [address.clone()]
    });
    type_real(&app, "x");
    hid(&app, key::L, &[os::CMD]);
    bar_closed(&app);
    assert!(bar_boxes(&app).is_empty());
    assert_eq!(titles(&app), ["Alpha"], "no tab from a bar put away");
    quit(&app);
}

/// The rows come in the spec's order. Address mode: what was typed, then the open tabs that match,
/// then history newest first, never an open tab twice. The switcher: what was typed, then the
/// open tabs most recently shown first.
#[test]
fn real_command_bar_rows_come_in_order() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/beta?old"));
    until(&app, "the page", 30, |a| titles(a) == ["Beta"]);
    go_to(&app, &format!("{base}/beta?new"));
    until(&app, "the newer Beta", 15, |a| address(a).ends_with("?new"));
    go_to(&app, &format!("{base}/plain"));
    until(&app, "Plain", 15, |a| titles(a) == ["Plain"]);
    open_in_new_tab(&app, &format!("{base}/beta"));
    until(&app, "Beta open", 15, |a| titles(a) == ["Plain", "Beta"]);
    open_bar(&app, key::K);
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    until(&app, "Plain shown again", 10, |a| shown_title(a) == "Plain");

    open_bar(&app, key::T);
    type_real(&app, "Beta");
    let rows = until_rows(&app, "the rows for Beta", |r| r.len() >= 4);
    assert!(rows[0].starts_with("Beta") && rows[0].ends_with("Search"), "what was typed comes first: {rows:?}");
    assert!(rows[1].starts_with("Beta") && rows[1].ends_with("Switch to Tab"), "then the open tab: {rows:?}");
    // After Cmd+T a history row opens in a new tab, and says so.
    assert!(
        rows[2].contains("beta?new") && rows[2].ends_with("Open in New Tab"),
        "then history, newest first: {rows:?}"
    );
    assert!(rows[3].contains("beta?old") && rows[3].ends_with("Open in New Tab"), "{rows:?}");
    let beta = format!("{}/beta,", base.trim_start_matches("http://"));
    assert!(
        !rows.iter().any(|r| r.contains(&beta) && r.ends_with("Open")),
        "the open tab is not history too: {rows:?}"
    );
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);

    // Shown in turn: Beta, then a new Alpha tab. The switcher offers Beta before Plain, though
    // Plain comes first in the sidebar.
    open_bar(&app, key::K);
    type_real(&app, "Beta");
    let rows = until_rows(&app, "Beta offered", |r| r.iter().any(|r| r.ends_with("Switch to Tab")));
    let at = rows.iter().position(|r| r.ends_with("Switch to Tab")).unwrap();
    for _ in 0..at {
        hid(&app, key::DOWN, &[]);
    }
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    until(&app, "Beta shown", 10, |a| shown_title(a) == "Beta");
    open_in_new_tab(&app, &format!("{base}/alpha"));
    until(&app, "Alpha", 15, |a| titles(a) == ["Plain", "Beta", "Alpha"]);
    open_bar(&app, key::K);
    type_real(&app, "127.0.0.1");
    let rows =
        until_rows(&app, "both tabs offered", |r| r.iter().filter(|r| r.ends_with("Switch to Tab")).count() == 2);
    let tabs: Vec<_> = rows.iter().filter(|r| r.ends_with("Switch to Tab")).collect();
    assert!(!rows[0].ends_with("Switch to Tab"), "what was typed comes first: {rows:?}");
    assert!(tabs[0].starts_with("Beta") && tabs[1].starts_with("Plain"), "most recently shown first: {rows:?}");
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
    quit(&app);
}

/// Nothing typed in the bar is fetched before Return: no preload of the address, no prefetch of
/// the completion.
#[test]
fn real_command_bar_fetches_nothing_before_return() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let host = base.trim_start_matches("http://").to_owned();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    go_to(&app, &format!("{base}/beta?visited"));
    until(&app, "Beta", 15, |a| titles(a) == ["Beta"]);

    open_bar(&app, key::T);
    type_real(&app, &format!("{host}/beta?vis"));
    until(&app, "the completion", 5, |a| bar_field(a) == Some(format!("{host}/beta?vis|ited")));
    type_real(&app, "x");
    std::thread::sleep(Duration::from_secs(3));
    for target in ["/beta?visx", "/beta?vis", "/beta?v"] {
        assert!(!requested(target), "{target} fetched while typing");
    }
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    until(&app, "the typed address", 15, |_| requested("/beta?visx"));
    quit(&app);
}

/// Esc, or a click outside the bar's panel, puts the bar away and gives the keyboard back to the
/// page.
#[test]
fn real_command_bar_hands_the_keys_back() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/keys"));
    until(&app, "the page", 30, |a| titles(a).first().is_some_and(|t| t.starts_with("Keys")));
    // Low on the page, well under the bar's card.
    let content = app.get_by_test_id("content").bounding_box().unwrap().expect("the page is laid out");
    let low = (content.x + content.width / 2., content.y + content.height * 0.85);
    let keys = |a: &App| titles(a).first().cloned().unwrap_or_default();
    click_page(&app, content.height * 0.85);
    type_real(&app, "a");
    until(&app, "the page's key", 5, |a| keys(a) == "Keys:a");

    open_bar(&app, key::K);
    type_real(&app, "zz");
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
    type_real(&app, "b");
    until(&app, "the key after Esc", 5, |a| keys(a) == "Keys:ab");

    open_bar(&app, key::K);
    type_real(&app, "zz");
    // Not `real_click`: the bar's panel has the keyboard, not the window that waits for it.
    let (wx, wy, _, _) = front_window(&app).bounds;
    os::click(&app, (wx + f64::from(low.0), wy + f64::from(low.1)));
    bar_closed(&app);
    type_real(&app, "c");
    until(&app, "the key after a click outside", 5, |a| keys(a) == "Keys:abc");
    quit(&app);
}

/// The typed row names the search engine and Return searches there: DuckDuckGo on a fresh
/// profile, Google or Bing when the settings say so.
#[test]
fn real_command_bar_searches_the_chosen_engine() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for (setting, name, host) in [
        (None, "DuckDuckGo", "duckduckgo.com"),
        (Some("google"), "Google", "google."),
        (Some("bing"), "Bing", "bing.com"),
    ] {
        let fresh = profile(&format!("engine-{name}"));
        let options = profile_options(&fresh, &[format!("{base}/alpha")]);
        let app = os::launch(match setting {
            Some(id) => options.env("LYNK_SEARCHENGINE", id),
            None => options,
        });
        os::in_front(&app);
        until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
        open_bar(&app, key::L);
        type_real(&app, "lynk drive words");
        until_rows(&app, "the search row", |r| {
            r.first().is_some_and(|r| r == &format!("lynk drive words, {name} Search, Search"))
        });
        hid(&app, key::ENTER, &[]);
        bar_closed(&app);
        until(&app, "the engine's page", 15, |a| address(a).contains(host));
        quit(&app);
    }
}

/// Commands are rows with their shortcut as the hint, spelled as the system spells it, and a
/// command that toggles names what it will do next (Pin Tab, then Unpin Tab).
#[test]
fn real_command_bar_runs_commands() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    let reopen =
        if cfg!(target_os = "macos") { "Reopen Closed Tab, ⇧⌘T" } else { "Reopen Closed Tab, Ctrl+Shift+T" };

    open_bar(&app, key::K);
    type_real(&app, "reopen");
    until_rows(&app, "the reopen command with its chord", |r| r.iter().any(|r| r == reopen));
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);

    open_bar(&app, key::K);
    type_real(&app, "pin tab");
    let rows = until_rows(&app, "Pin Tab", |r| r.iter().any(|r| r.starts_with("Pin Tab")));
    for _ in 0..rows.iter().position(|r| r.starts_with("Pin Tab")).unwrap() {
        hid(&app, key::DOWN, &[]);
    }
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    open_bar(&app, key::K);
    type_real(&app, "pin tab");
    until_rows(&app, "the command turned to Unpin Tab", |r| {
        r.iter().any(|r| r.starts_with("Unpin Tab")) && !r.iter().any(|r| r.starts_with("Pin Tab"))
    });
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
    quit(&app);
}

/// Up and Down, Page Up and Page Down move the highlight and Return runs the highlighted row; new
/// results highlight the top row again; Cmd+Return (Ctrl+Return off macOS) goes to the text as
/// typed, past a completion.
#[test]
fn real_command_bar_moves_the_highlight() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let host = base.trim_start_matches("http://").to_owned();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    go_to(&app, &format!("{base}/plain"));
    until(&app, "Plain", 15, |a| titles(a) == ["Plain"]);
    open_in_new_tab(&app, &format!("{base}/beta"));
    until(&app, "Beta", 15, |a| titles(a) == ["Plain", "Beta"]);
    let offered = |rows: &[String]| rows.last().is_some_and(|r| r.starts_with("Plain") && r.ends_with("Switch to Tab"));

    // The typed row first, the open tab last: Page Down, Page Up, Page Down, then Return switches.
    open_bar(&app, key::K);
    type_real(&app, "Plai");
    until_rows(&app, "Plain offered last", offered);
    hid(&app, key::PAGE_DOWN, &[]);
    hid(&app, key::PAGE_UP, &[]);
    hid(&app, key::PAGE_DOWN, &[]);
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    until(&app, "Plain by Page Down", 10, |a| shown_title(a) == "Plain");

    // Plain is shown now, so Beta is the open tab offered. Down to it, then a letter that
    // changes the results takes the highlight back to the top row: Return searches in this tab
    // instead of switching.
    let beta_last =
        |rows: &[String]| rows.last().is_some_and(|r| r.starts_with("Beta") && r.ends_with("Switch to Tab"));
    open_bar(&app, key::K);
    type_real(&app, "Bet");
    until_rows(&app, "Beta offered last", beta_last);
    hid(&app, key::DOWN, &[]);
    type_real(&app, "a");
    until_rows(&app, "Beta offered for Beta", beta_last);
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    // The engine may answer with a consent page first, so the search reads as Plain giving way.
    until(&app, "a search in the shown tab", 15, |a| {
        let t = titles(a);
        t.len() == 2 && !t.contains(&"Plain".to_owned())
    });
    assert_ne!(shown_title(&app), "Beta", "the top row ran, not the open tab");

    // Down, then Up, back to the typed row: Return searches again rather than switching.
    open_bar(&app, key::K);
    type_real(&app, "Bet");
    until_rows(&app, "Beta offered last", beta_last);
    hid(&app, key::DOWN, &[]);
    hid(&app, key::UP, &[]);
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    std::thread::sleep(Duration::from_secs(2));
    assert_ne!(shown_title(&app), "Beta", "Up left the open tab");
    assert_eq!(titles(&app).len(), 2);

    // Cmd+Return goes to the typed text and leaves the completion behind.
    open_bar(&app, key::T);
    type_real(&app, &format!("{host}/pl"));
    until(&app, "the completed path", 5, |a| bar_field(a) == Some(format!("{host}/pl|ain")));
    hid(&app, key::ENTER, &[os::CMD]);
    bar_closed(&app);
    until(&app, "the typed address in a new tab", 15, |a| titles(a).len() == 3);
    assert_eq!(address(&app), format!("{base}/pl"));
    quit(&app);
}

/// A capture for the report, under the worktree's `target/report-shots`, at the display's scale.
#[cfg(target_os = "macos")]
fn report_shot(app: &App, name: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/report-shots");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.png"));
    os::capture(app, &front_window(app), &path);
    println!("screenshot {}", path.display());
}

#[cfg(target_os = "macos")]
#[test]
fn real_command_bar_looks() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let long = format!("{base}/beta?{}", "segment/".repeat(240));
    for appearance in ["light", "dark"] {
        let theme = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lynk-theme-{appearance}.toml"));
        std::fs::write(&theme, format!("appearance = \"{appearance}\"\n")).unwrap();
        let urls = [format!("{base}/alpha"), format!("{base}/beta"), long.clone()];
        let options = urls.iter().fold(
            LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
                .env("ND_CEF_STYLE", "chrome")
                .env("LYNK_SEARCHENGINE", "google")
                .env("ND_THEME_TOML", theme.to_str().unwrap()),
            |o, url| o.arg(url),
        );
        let app = os::launch(options);
        until(&app, "the first page", 30, |a| titles(a).first().is_some_and(|t| t == "Alpha"));
        for width in [1200, 720] {
            resize(&app, width);
            let name = |what: &str| format!("bar-{appearance}-{width}-{what}");
            open_bar(&app, key::T);
            report_shot(&app, &name("empty"));
            type_real(&app, "bet");
            report_shot(&app, &name("few"));
            hid(&app, key::ESCAPE, &[]);
            bar_closed(&app);
            open_bar(&app, key::K);
            report_shot(&app, &name("many"));
            hid(&app, key::ESCAPE, &[]);
            bar_closed(&app);
            press(&app, "cmd-3");
            open_bar(&app, key::L);
            report_shot(&app, &name("long-url"));
            hid(&app, key::ESCAPE, &[]);
            bar_closed(&app);
            press(&app, "cmd-1");
            open_bar(&app, key::T);
            type_real(&app, "\u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}");
            report_shot(&app, &name("rtl"));
            hid(&app, key::ESCAPE, &[]);
            bar_closed(&app);
            open_bar(&app, key::T);
            type_real(&app, "\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{691c}\u{7d22} \u{30c6}\u{30b9}\u{30c8}");
            report_shot(&app, &name("cjk"));
            hid(&app, key::ESCAPE, &[]);
            bar_closed(&app);
        }
    }
}

/// The tiles once two reads 150 ms apart agree: a reorder glides them to their slots.
fn settled_tiles(app: &App) -> Vec<(String, (f32, f32, f32, f32))> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last = tiles(app);
    loop {
        std::thread::sleep(Duration::from_millis(150));
        let now = tiles(app);
        if now == last || Instant::now() > deadline {
            return now;
        }
        last = now;
    }
}

/// The first window's pinned tiles in reading order: their tab's label and their frame, in
/// window points.
fn tiles(app: &App) -> Vec<(String, (f32, f32, f32, f32))> {
    let Ok(tree) = app.tree(None) else { return Vec::new() };
    let Some(window) = tree.windows.first() else { return Vec::new() };
    let mut tiles: Vec<_> = window
        .root
        .iter()
        .filter(|n| {
            n.visible
                && n.test_id.as_deref().and_then(|t| t.strip_prefix("pin-")).is_some_and(|t| t.parse::<u64>().is_ok())
        })
        .filter_map(|n| {
            let r = n.bounds?;
            Some((n.name.clone(), (r.origin.x, r.origin.y, r.size.width, r.size.height)))
        })
        .collect();
    tiles.sort_by(|a, b| a.1.1.total_cmp(&b.1.1).then(a.1.0.total_cmp(&b.1.0)));
    tiles
}

fn tile_names(app: &App) -> Vec<String> {
    tiles(app).into_iter().map(|t| t.0).collect()
}

fn tile_at(app: &App, index: usize) -> (f32, f32) {
    let (_, (x, y, w, h)) = tiles(app)[index].clone();
    (x + w / 2., y + h / 2.)
}

/// Pin and unpin from the row's menu, the tile's menu and Shift+Cmd+D: a pin goes to the end of
/// the tiles and leaves the list, an unpinned tab heads the list, Next and Previous Tab walk the
/// tiles then the rows, and Close Other Tabs spares the pins.
#[test]
fn real_pin_and_unpin() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch_with(&[format!("{base}/alpha"), format!("{base}/beta"), format!("{base}/press")]);
    until(&app, "three rows", 30, |a| titles(a).len() == 3 && shown_title(a) == "Alpha");
    pick_menu(&app, row_at(&app, 1), "Pin Tab");
    until(&app, "beta pinned", 10, |a| tiles(a).len() == 1 && titles(a).len() == 2);
    assert!(tile_names(&app)[0].ends_with("beta"), "{:?}", tile_names(&app));
    assert_eq!(titles(&app)[0], "Alpha");
    // The shown tab pinned by its chord stays shown.
    press(&app, "cmd-shift-d");
    until(&app, "alpha pinned after beta", 10, |a| tile_names(a).len() == 2 && titles(a).len() == 1);
    assert_eq!(shown_title(&app), "Alpha");
    shot(&app, "pins-shown-tile");
    press(&app, "cmd-shift-]");
    until(&app, "next is the first row", 10, |a| shown_title(a) == "Press");
    press(&app, "cmd-shift-]");
    until(&app, "next wraps to the first tile", 10, |a| shown_title(a) == "Beta");
    press(&app, "cmd-shift-[");
    until(&app, "previous goes back to the row", 10, |a| shown_title(a) == "Press");
    press(&app, "cmd-shift-[");
    until(&app, "previous from the row is the last tile", 10, |a| shown_title(a) == "Alpha");
    open_in_new_tab(&app, &format!("{base}/beta?2"));
    until(&app, "a second row", 10, |a| titles(a).len() == 2);
    pick_menu(&app, row_at(&app, 0), "Close Other");
    until(&app, "close others spared the pins", 15, |a| titles(a).len() == 1 && tile_names(a).len() == 2);
    // The last row pinned leaves the list empty.
    pick_menu(&app, row_at(&app, 0), "Pin Tab");
    until(&app, "every tab pinned", 10, |a| titles(a).is_empty() && tile_names(a).len() == 3);
    pick_menu(&app, tile_at(&app, 0), "Unpin");
    until(&app, "the first tile unpinned to the head of the list", 10, |a| {
        titles(a) == ["Beta"] && tile_names(a).len() == 2
    });
    pick_menu(&app, tile_at(&app, 1), "Close Tab");
    until(&app, "a pinned tab closed", 10, |a| tile_names(a).len() == 1);
}

/// Whether the tab of the `index`th tile, in reading order, is asleep: no page left for it.
fn tile_asleep(app: &App, index: usize) -> bool {
    let Some((window, _)) = window_rows(app) else { return false };
    let mut tiles: Vec<(f32, f32, &str)> = window
        .root
        .iter()
        .filter(|n| n.visible)
        .filter_map(|n| {
            let tab = n.test_id.as_deref()?.strip_prefix("pin-").filter(|t| t.parse::<u64>().is_ok())?;
            Some((n.bounds?.origin.y, n.bounds?.origin.x, tab))
        })
        .collect();
    tiles.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    tiles.get(index).is_some_and(|(_, _, tab)| !window.root.iter().any(|n| page_of(n) == Some(*tab)))
}

/// Every item of a tile's menu, picked with the real pointer: Mute Tab mutes the tile's tab and
/// the item flips to Unmute Tab, Put Tab to Sleep drops its page and keeps the tile, Reset to
/// Pinned Page goes home, Unpin Tab heads the list and Close Tab closes it.
#[test]
fn real_pinned_tile_menu() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch_with(&[format!("{base}/alpha"), format!("{base}/beta"), format!("{base}/press")]);
    until(&app, "three rows", 30, |a| titles(a).len() == 3 && shown_title(a) == "Alpha");
    // Tabs load when first shown; Beta needs a page for Put Tab to Sleep to take away.
    press(&app, "cmd-2");
    until(&app, "Beta shown", 15, |a| shown_title(a) == "Beta");
    press(&app, "cmd-1");
    until(&app, "Alpha shown", 15, |a| shown_title(a) == "Alpha");
    pick_menu(&app, row_at(&app, 0), "Pin Tab");
    until(&app, "alpha pinned", 10, |a| tiles(a).len() == 1 && titles(a).len() == 2);
    pick_menu(&app, row_at(&app, 0), "Pin Tab");
    until(&app, "beta pinned", 10, |a| tiles(a).len() == 2 && titles(a).len() == 1);
    assert_eq!(shown_title(&app), "Alpha");

    pick_menu(&app, tile_at(&app, 0), "Mute Tab");
    until(&app, "alpha muted", 10, |a| tile_names(a)[0].ends_with(" - Audio muted"));
    pick_menu(&app, tile_at(&app, 0), "Unmute Tab");
    until(&app, "alpha unmuted", 10, |a| !tile_names(a)[0].ends_with(" - Audio muted"));

    until(&app, "beta loaded behind", 15, |a| !tile_asleep(a, 1));
    pick_menu(&app, tile_at(&app, 1), "Put Tab to Sleep");
    until(&app, "beta asleep with its tile", 10, |a| tile_asleep(a, 1) && tiles(a).len() == 2);

    click_page(&app, 100.);
    until(&app, "the pin left its page", 15, |a| shown_title(a) == "Beta");
    pick_menu(&app, tile_at(&app, 0), "Reset");
    until(&app, "back at its pinned page", 15, |a| shown_title(a) == "Alpha");

    pick_menu(&app, tile_at(&app, 1), "Unpin");
    until(&app, "beta heads the list", 10, |a| tiles(a).len() == 1 && titles(a).first().is_some_and(|t| t == "Beta"));
    pick_menu(&app, tile_at(&app, 0), "Close Tab");
    until(&app, "the pin closed", 10, |a| tiles(a).is_empty() && titles(a).len() == 2);
    quit(&app);
}

/// Writes one `width` window as the profile's session: `pins` pinned tabs at `base/alpha?p<i>`,
/// then `rows` tabs at `base/beta?<i>`, showing the first pin.
fn write_pinned(profile: &std::path::Path, base: &str, pins: usize, rows: usize, width: u32) {
    let mut tabs = Vec::new();
    for i in 1..=pins {
        let url = format!("{base}/alpha?p{i}");
        tabs.push(serde_json::json!({ "id": format!("t{i}"), "url": url, "title": format!("P{i}"), "pinnedUrl": url }));
    }
    for i in 1..=rows {
        tabs.push(serde_json::json!({ "id": format!("t{}", pins + i), "url": format!("{base}/beta?{i}"), "title": format!("R{i}") }));
    }
    let pin_ids: Vec<String> = (1..=pins).map(|i| format!("t{i}")).collect();
    let items: Vec<_> =
        (1..=rows).map(|i| serde_json::json!({ "type": "tab", "id": format!("t{}", pins + i) })).collect();
    let doc = serde_json::json!({ "version": 3, "data": {
        "spaces": [{ "id": "s1", "name": "Space" }],
        "windows": [{ "id": "w1", "tabs": tabs, "activeId": "t1", "width": width, "height": 800,
            "spaces": { "shown": "s1", "sidebars": { "s1": { "pins": pin_ids, "items": items } } } }],
    } });
    std::fs::write(profile.join("session.json"), doc.to_string()).unwrap();
}

/// 1, 4, 5 and 9 pins at 720 and 1200 pt: three columns under 900 pt and four from there, tiles
/// at least 40 wide and 0.74 of that tall, a short last row on the same pitch, and the tiles
/// where they were while the list under them scrolls.
#[test]
fn real_pinned_grid_at_two_widths() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for width in [720u32, 1200] {
        let columns = if width < 900 { 3 } else { 4 };
        for n in [1usize, 4, 5, 9] {
            let profile = profile(&format!("pins-grid-{n}-{width}"));
            write_pinned(&profile, &base, n, 40, width);
            let app = launch_on(&profile, &[]);
            until(&app, "the tiles", 30, |a| tiles(a).len() == n && rows(a).len() > 5);
            let laid = settled_tiles(&app);
            let top = laid[0].1.1;
            let first_row: Vec<f32> = laid.iter().filter(|t| (t.1.1 - top).abs() < 1.).map(|t| t.1.0).collect();
            assert_eq!(first_row.len(), n.min(columns), "{n} pins at {width}: {laid:?}");
            let mut ys: Vec<f32> = laid.iter().map(|t| t.1.1).collect();
            ys.dedup_by(|a, b| (*a - *b).abs() < 1.);
            assert_eq!(ys.len(), n.div_ceil(columns), "{n} pins at {width}: rows of tiles");
            for (name, (x, _, w, h)) in &laid {
                assert!(*w >= 40., "{name} is {w} wide at {width}");
                assert!((h - w * 0.74).abs() <= 1.5, "{name} is {w} by {h}");
                assert!(first_row.iter().any(|f| (f - x).abs() < 1.), "{name} at x {x} is off the pitch {first_row:?}");
            }
            shot(&app, &format!("pins-{n}-at-{width}"));
            let before_rows = rows(&app);
            let fixed = |a: &App| ["controls-row", "sidebar-foot"].map(|id| a.get_by_test_id(id).bounding_box().unwrap());
            let fixed_before = fixed(&app);
            let (wx, wy, _, _) = front_window(&app).bounds;
            let at = row_at(&app, 3);
            os::scroll(&app, (wx + f64::from(at.0), wy + f64::from(at.1)), -800, 8);
            until(&app, "the list scrolled", 10, |a| rows(a) != before_rows);
            let after: Vec<_> = settled_tiles(&app).into_iter().map(|t| t.1).collect();
            assert_eq!(after, laid.iter().map(|t| t.1).collect::<Vec<_>>(), "the tiles moved with the list");
            assert_eq!(fixed(&app), fixed_before, "the controls row or the foot moved with the list");
            quit(&app);
            drop(app);
        }
    }
}

/// A pin taken to another page comes back there after a restart, the only live page, and Reset
/// to Pinned Page takes it home. The session keeps the page it was pinned at.
#[test]
fn real_pin_reset_after_a_restart() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let profile = profile("pins-restart");
    let app = launch_on(&profile, &[format!("{base}/alpha"), format!("{base}/press")]);
    until(&app, "two rows", 30, |a| titles(a).len() == 2 && shown_title(a) == "Alpha");
    pick_menu(&app, row_at(&app, 0), "Pin Tab");
    until(&app, "alpha pinned", 10, |a| tiles(a).len() == 1 && titles(a).len() == 1);
    click_page(&app, 100.);
    until(&app, "the pin left its page", 15, |a| shown_title(a) == "Beta");
    quit(&app);
    drop(app);
    let saved = session_of(&profile);
    let pin = saved["windows"][0]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["pinnedUrl"].is_string())
        .cloned()
        .expect("the pin kept its page");
    assert_eq!(pin["pinnedUrl"], format!("{base}/alpha"));
    assert_eq!(pin["url"], format!("{base}/beta"));

    let app = launch_on(&profile, &[]);
    until(&app, "the pin back", 30, |a| tiles(a).len() == 1 && titles(a).len() == 1);
    until(&app, "the pin's page", 30, |a| shown_title(a) == "Beta");
    assert_eq!(live_pages(&app), Some(1), "only the shown tab loads");
    pick_menu(&app, tile_at(&app, 0), "Reset");
    until(&app, "back at its pinned page", 15, |a| shown_title(a) == "Alpha");
    quit(&app);
}

/// Drags with the real pointer: a row dropped on the tiles pins, tiles reorder, and a tile
/// dropped on the list unpins.
#[test]
fn real_drag_between_the_list_and_the_pins() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch_with(&[format!("{base}/alpha"), format!("{base}/beta"), format!("{base}/press")]);
    until(&app, "three rows", 30, |a| titles(a).len() == 3);
    pick_menu(&app, row_at(&app, 0), "Pin Tab");
    until(&app, "one tile", 10, |a| tiles(a).len() == 1 && titles(a).len() == 2);
    let (wx, wy, _, _) = front_window(&app).bounds;
    let global = |p: (f32, f32)| (wx + f64::from(p.0), wy + f64::from(p.1));

    let from = row_at(&app, 1);
    let (_, (tx, ty, tw, th)) = tiles(&app)[0].clone();
    let to = (tx + tw * 1.6, ty + th / 2.);
    os::drag(&app, global(from), f64::from(to.0 - from.0), f64::from(to.1 - from.1), 16);
    until(&app, "the row pinned by a drop", 10, |a| tiles(a).len() == 2 && titles(a).len() == 1);
    shot(&app, "pins-dropped-row");

    let order = tile_names(&app);
    let (a, b) = (tile_at(&app, 0), tile_at(&app, 1));
    os::drag(&app, global(b), f64::from(a.0 - b.0 - 8.), f64::from(a.1 - b.1), 16);
    until(&app, "the tiles swapped", 10, |a| tile_names(a) == [order[1].clone(), order[0].clone()]);

    let tile = tile_at(&app, 0);
    let row = row_at(&app, 0);
    os::drag(&app, global(tile), f64::from(row.0 - tile.0), f64::from(row.1 - tile.1 - 4.), 16);
    until(&app, "the tile unpinned by a drop", 10, |a| tiles(a).len() == 1 && titles(a).len() == 2);
    quit(&app);
}

/// Runs a command from the bar: types `typed` and takes the row that starts with `row`.
#[track_caller]
fn run_command(app: &App, typed: &str, row: &str) {
    open_bar(app, key::K);
    type_real(app, typed);
    let deadline = Instant::now() + Duration::from_secs(10);
    // Rows read before the field holds all of `typed` can belong to a shorter query and reorder.
    let at = loop {
        let field = bar_field(app).unwrap_or_default();
        let rows = bar_rows(app);
        if field.split('|').next() == Some(typed)
            && let Some(at) = rows.iter().position(|r| r.starts_with(row))
        {
            break at;
        }
        assert!(Instant::now() < deadline, "no {row} row for {typed:?}, field {field:?}: {rows:?}");
        std::thread::sleep(Duration::from_millis(150));
    };
    for _ in 0..at {
        hid(app, key::DOWN, &[]);
        std::thread::sleep(Duration::from_millis(80));
    }
    hid(app, key::ENTER, &[]);
    bar_closed(app);
}

/// Makes a profile called `name` from the command bar, which opens a window in it.
#[track_caller]
fn new_profile(app: &App, name: &str) {
    let before = window_count(app);
    run_command(app, "new profile", "New Profile");
    until(app, "the name field", 10, |a| has(a, "profile-name"));
    paste(app, name);
    std::thread::sleep(Duration::from_millis(300));
    hid(app, key::ENTER, &[]);
    until(app, "a window in the new profile", 20, |a| window_count(a) == before + 1);
}

/// Each window's title, which is its shown tab's.
fn all_titles(app: &App) -> Vec<String> {
    let windows = app.windows().unwrap_or_default();
    let mut titles: Vec<String> = windows
        .as_array()
        .map(|ws| ws.iter().map(|w| w["title"].as_str().unwrap_or_default().to_owned()).collect())
        .unwrap_or_default();
    titles.sort();
    titles
}

/// The title of the app's key window.
fn key_title(app: &AppHandle) -> Option<String> {
    app.windows()
        .ok()
        .and_then(|w| w.as_array().cloned())
        .and_then(|ws| ws.iter().find(|w| w["key"] == true).map(|w| w["title"].as_str().unwrap_or_default().to_owned()))
}

/// Each profile's cookie store as Chromium left it on disk: `<dir>: <host> <name>, ...`.
/// Each row of `sql`'s one text column, comma separated; "" for a store that is not there yet.
/// Read in-process: a host without the sqlite3 tool would otherwise read every store as empty.
fn sqlite_rows(db: &std::path::Path, sql: &str) -> String {
    use nd_data::rusqlite::{Connection, OpenFlags};
    if !db.exists() {
        return String::new();
    }
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap_or_else(|e| panic!("opening {}: {e}", db.display()));
    let mut statement = conn.prepare(sql).unwrap_or_else(|e| panic!("{sql} on {}: {e}", db.display()));
    let rows = statement.query_map([], |r| r.get::<_, String>(0)).unwrap_or_else(|e| panic!("{sql}: {e}"));
    rows.map(|r| r.unwrap_or_else(|e| panic!("{sql}: {e}"))).collect::<Vec<_>>().join(", ")
}

fn profile_cookies(dir: &std::path::Path) -> String {
    let stores = [dir.join("cef/Default/Cookies")].into_iter().chain(
        std::fs::read_dir(dir.join("cef"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("profile-"))
            .map(|e| e.path().join("Cookies")),
    );
    stores
        .map(|db| {
            let rows = sqlite_rows(&db, "select host_key || ' ' || name from cookies");
            format!("{}: [{rows}]", db.strip_prefix(dir).unwrap_or(&db).display())
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Every window with its profile's tabs, key state and frame, and `profiles.json`, for a
/// restore that did not come back as it should.
fn profile_windows(app: &AppHandle, dir: &std::path::Path) -> String {
    let tree = app.tree(None).map(|t| {
        t.windows.iter().map(|w| format!("{:?} tabs {:?}", w.title, window_titles(w))).collect::<Vec<_>>().join("; ")
    });
    format!(
        "windows {}\ntabs {}\nprofiles {:?}",
        app.windows().unwrap_or_default(),
        tree.unwrap_or_default(),
        profiles_in(dir)
    )
}

/// `profiles.json`: each profile's id, name and whether it opens at launch.
fn profiles_in(dir: &std::path::Path) -> Vec<(String, String, bool)> {
    let text = std::fs::read_to_string(dir.join("profiles.json")).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    doc["data"]["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap().to_owned(), p["name"].as_str().unwrap().to_owned(), p["open"] == true))
        .collect()
}

/// Two profiles with windows open together keep their cookies apart, come back together after
/// a restart, and switching brings each one's window forward. A cookie set in Default is sent
/// to Default's pages only.
#[test]
fn real_two_profiles_keep_their_cookies_apart() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let dir = profile("profiles-two");
    let app = launch_on(&dir, &[format!("{base}/set-cookie")]);
    until(&app, "the cookie set", 30, |a| shown_title(a) == "Set");
    new_profile(&app, "Work");
    go_to(&app, &format!("{base}/cookie"));
    until(&app, "Work sent no cookie", 15, |a| all_titles(a).contains(&"Cookie none".to_owned()));
    shot(&app, "profiles-two-windows");
    run_command(&app, "switch to default", "Switch to Default");
    key_within(&app, &dir, "Set", 10);
    open_bar(&app, key::T);
    paste(&app, &format!("{base}/cookie"));
    std::thread::sleep(Duration::from_millis(300));
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    until(&app, "Default sent its cookie", 15, |a| all_titles(a) == ["Cookie none", "Cookie who=one"]);
    let profiles = profiles_in(&dir);
    assert_eq!(profiles.len(), 2, "{profiles:?}");
    assert_eq!(profiles[1].1, "Work");
    let work = profiles[1].0.clone();
    quit(&app);
    drop(app);
    assert!(profiles_in(&dir).iter().all(|p| p.2), "both open at quit");
    assert!(dir.join("profiles").join(&work).join("session.json").exists(), "Work keeps its own session");

    // Each window's shown page reloads at launch, sending what its profile has kept.
    let kept = profile_cookies(&dir);
    let app = launch_on(&dir, &[]);
    let deadline = Instant::now() + Duration::from_secs(30);
    while all_titles(&app) != ["Cookie none", "Cookie who=one"] {
        if Instant::now() > deadline {
            shot(&app, "profiles-two-restart-timeout");
            panic!(
                "timed out restoring both profiles, still apart\ncookies kept at quit: {kept}\n{}\n{}",
                profile_windows(&app, &dir),
                app.stderr_tail(80).join("\n")
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    // A relaunch leaves exactly one of the restored windows key, with no click.
    let keys = |a: &AppHandle| {
        a.windows().ok().and_then(|w| w.as_array().map(|ws| ws.iter().filter(|w| w["key"] == true).count()))
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while keys(&app) != Some(1) {
        if Instant::now() > deadline {
            shot(&app, "profiles-two-restart-no-key");
            panic!(
                "{:?} windows key after the relaunch, not one\n{}\n{}\n{}",
                keys(&app),
                profile_windows(&app, &dir),
                os::focus_report(&app),
                app.stderr_tail(40).join("\n")
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    for (row, title) in [("Switch to Default", "Cookie who=one"), ("Switch to Work", "Cookie none")] {
        os::in_front(&app);
        // The profile in front offers no switch to itself.
        if key_title(&app).as_deref() != Some(title) {
            run_command(&app, &row.to_lowercase(), row);
        }
        key_within(&app, &dir, title, 10);
    }
    quit(&app);
}

/// Waits `secs` for the window showing `title` to be key. Past that it watches 30 s more, so the
/// failure says whether the window was late or never came, with every window's state.
#[track_caller]
fn key_within(app: &App, dir: &std::path::Path, title: &str, secs: u64) {
    let start = Instant::now();
    let is_key = |a: &AppHandle| key_title(a).as_deref() == Some(title);
    while start.elapsed() < Duration::from_secs(secs) {
        if is_key(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let state = profile_windows(app, dir);
    let focus = os::focus_report(app);
    shot(app, "key-timeout");
    let late = loop {
        if is_key(app) {
            break Some(start.elapsed());
        }
        if start.elapsed() > Duration::from_secs(secs + 30) {
            break None;
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    panic!(
        "{title:?} not key within {secs}s ({}); at the timeout\n{state}\n{focus}\n{}",
        late.map_or("never, after 30 s more".to_owned(), |t| format!("key after {t:?}")),
        app.stderr_tail(40).join("\n")
    );
}

/// What a page or the user changed moments before a quit is there at the next launch: a cookie,
/// localStorage, a zoom level and the history rows. The quit comes within a second or two of the
/// changes, before any engine timer would have written them. Every kind is checked before the
/// case fails, so a failure names all that was lost.
#[test]
fn real_quit_right_after_a_change_keeps_it() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let dir = profile("quit-fast");
    let app = launch_on(&dir, &[format!("{base}/store-set")]);
    until(&app, "the stored value", 30, |a| shown_title(a).starts_with("Stored kept"));
    let stored = shown_title(&app);
    open_bar(&app, key::T);
    paste(&app, &format!("{base}/set-cookie-read"));
    std::thread::sleep(Duration::from_millis(300));
    hid(&app, key::ENTER, &[]);
    bar_closed(&app);
    until(&app, "the cookie sent back", 15, |a| shown_title(a) == "Cookie who=one");
    let set = Instant::now();
    press(&app, "cmd-=");
    until(&app, "110%", 10, |a| text_of(a, "zoom-value").as_deref() == Some("110%"));
    let since = set.elapsed();
    quit(&app);
    drop(app);

    let mut lost = Vec::new();
    let cookies = profile_cookies(&dir);
    if !cookies.contains("who") {
        lost.push(format!("cookie on disk: {cookies}"));
    }
    let zoom = session_of(&dir)["zoomByHost"].clone();
    if zoom.as_object().is_none_or(|z| z.values().all(|f| f.as_f64() != Some(1.1))) {
        lost.push(format!("zoom in session.json: {zoom}"));
    }
    let visits = sqlite_rows(&dir.join("history.sqlite"), "select url from visits");
    if !["/store", "/cookie"].iter().all(|p| visits.contains(p)) {
        lost.push(format!("history rows: [{visits}]"));
    }

    // The shown tab reloads at launch and sends what the profile kept; the other loads when shown.
    let app = launch_on(&dir, &[]);
    until(&app, "the cookie page reloaded", 30, |a| shown_title(a).starts_with("Cookie "));
    if shown_title(&app) != "Cookie who=one" {
        lost.push(format!("cookie sent after launch: {:?}", shown_title(&app)));
    }
    if !has(&app, "zoom") {
        lost.push("zoom glyph after launch".to_owned());
    }
    press(&app, "cmd-1");
    until(&app, "the stored page reloaded", 15, |a| shown_title(a).starts_with("Stored ") && shown_title(a) != stored);
    if !shown_title(&app).starts_with("Stored kept") {
        lost.push(format!("localStorage after launch: {:?}", shown_title(&app)));
    }
    quit(&app);
    assert!(lost.is_empty(), "lost at a quit {since:?} after the cookie was set:\n{}", lost.join("\n"));
}

/// Deleting the profile whose window is open asks first, closes that window without asking its
/// pages, removes its files, and the profile does not come back after a restart. A name of 80
/// characters with spaces and an emoji is kept as typed.
#[test]
fn real_delete_the_open_profile() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let dir = profile("profiles-delete");
    let lead = "\u{1F680} Work and play ";
    let long = lead.to_owned() + &"x".repeat(80 - lead.chars().count());
    let app = launch_on(&dir, &[format!("{base}/alpha")]);
    until(&app, "the first window", 30, |a| shown_title(a) == "Alpha");
    new_profile(&app, &long);
    let profiles = profiles_in(&dir);
    assert_eq!(profiles[1].1, long, "the name as typed");
    let id = profiles[1].0.clone();
    go_to(&app, &format!("{base}/leave"));
    until(&app, "the page in the new profile", 15, |a| all_titles(a).contains(&"Leave".to_owned()));
    // A page that would ask before it is left does not hold the delete up. Two windows each
    // have a page, so the click goes by the front window's own frame.
    let width = front_window(&app).bounds.2 as f32;
    real_click(&app, width * 0.65, 160.);
    run_command(&app, "delete profile", "Delete Profile");
    std::thread::sleep(Duration::from_secs(1));
    shot(&app, "profiles-delete-asks");
    assert!(os::alert_button_shown(&app, "Cancel"), "Delete Profile asks first");
    os::click_button(&app, "Cancel");
    until(&app, "the alert gone after Cancel", 10, |a| !os::alert_button_shown(a, "Cancel"));
    assert_eq!(window_count(&app), 2, "Cancel keeps the profile");
    run_command(&app, "delete profile", "Delete Profile");
    std::thread::sleep(Duration::from_secs(1));
    os::click_button(&app, "Delete Profile");
    until(&app, "the alert gone after Delete", 10, |a| !os::alert_button_shown(a, "Cancel"));
    until(&app, "its window gone", 15, |a| window_count(a) == 1);
    assert_eq!(all_titles(&app), ["Alpha"]);
    assert_eq!(profiles_in(&dir).len(), 1);
    assert!(!dir.join("profiles").join(&id).exists(), "its files are gone");
    quit(&app);
    drop(app);

    let app = launch_on(&dir, &[]);
    until(&app, "only Default back", 30, |a| window_count(a) == 1 && all_titles(a) == ["Alpha"]);
    quit(&app);
    drop(app);
    assert!(!dir.join("cef").join(format!("profile-{id}")).exists(), "Chromium's store went at the next launch");
}

/// A capture of the window's region of the screen, menus and popovers over it included.
#[cfg(target_os = "macos")]
fn screen_shot(app: &App, name: &str) -> PathBuf {
    let (x, y, w, h) = front_window(app).bounds;
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("shell-{name}.png"));
    let region = format!("-R{x},{y},{w},{h}");
    let status = std::process::Command::new("screencapture").args(["-x", &region]).arg(&path).status().unwrap();
    assert!(status.success());
    println!("screenshot {}", path.display());
    path
}

/// Captures of the pins and the profile controls in light and dark at 720 and 1200 pt: tiles at
/// rest, under the pointer and pressed, the tile menu with Reset to Pinned Page off, the profile
/// menu holding an 80 character name, and the name popover refusing a taken name.
#[cfg(target_os = "macos")]
#[test]
fn real_pins_and_profiles_captures() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let long = "\u{1F680} Work and play ".to_owned() + &"x".repeat(64);
    for appearance in ["light", "dark"] {
        let theme = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("lynk-theme-{appearance}.toml"));
        std::fs::write(&theme, format!("appearance = \"{appearance}\"\n")).unwrap();
        for width in [720u32, 1200] {
            let dir = profile(&format!("pins-shots-{appearance}-{width}"));
            write_pinned(&dir, &base, 5, 12, width);
            std::fs::write(
                dir.join("profiles.json"),
                serde_json::json!({ "version": 1, "data": { "profiles": [
                    { "id": "default", "name": "Default", "open": true },
                    { "id": "p2", "name": long, "open": false },
                ], "nextId": 3 } })
                .to_string(),
            )
            .unwrap();
            let options = LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
                .env("ND_CEF_STYLE", "chrome")
                .env("ND_THEME_TOML", theme.display().to_string())
                .env("ND_APP_DATA_DIR", dir.display().to_string());
            let app = os::launch(options);
            until(&app, "the tiles", 30, |a| tiles(a).len() == 5 && shown_title(a) == "Alpha");
            // Pins 2 and 3 wake and show the page's favicon; 4 and 5 sleep on their letter.
            // One press at a time: a second chord sent while the first pin still wakes can be lost.
            for left in [3, 2] {
                let before = shown_title(&app);
                press(&app, "cmd-shift-]");
                let deadline = Instant::now() + Duration::from_secs(20);
                while letters(&app) != left {
                    assert!(
                        Instant::now() < deadline,
                        "{left} tiles never on their letter: showing {:?} (was {before:?}), windows {:?}\n{}\n{}",
                        shown_title(&app),
                        app.windows(),
                        os::focus_report(&app),
                        app.stderr_tail(20).join("\n")
                    );
                    std::thread::sleep(Duration::from_millis(150));
                }
            }
            real_click(&app, tile_at(&app, 0).0, tile_at(&app, 0).1);
            let deadline = Instant::now() + Duration::from_secs(20);
            while letters(&app) != 2 {
                if Instant::now() > deadline {
                    shot(&app, &format!("pins-favicons-timeout-{appearance}-{width}"));
                    panic!("{} tiles still draw a letter", letters(&app));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            std::thread::sleep(Duration::from_secs(1));
            let name = |state: &str| format!("pins-{state}-{appearance}-{width}");
            shot(&app, &name("rest"));
            let (wx, wy, _, _) = front_window(&app).bounds;
            let global = |p: (f32, f32)| (wx + f64::from(p.0), wy + f64::from(p.1));
            os::move_to(&app, global(tile_at(&app, 2)));
            std::thread::sleep(Duration::from_millis(600));
            shot(&app, &name("hover"));
            os::hold(&app, global(tile_at(&app, 2)));
            std::thread::sleep(Duration::from_millis(300));
            shot(&app, &name("pressed"));
            os::release(&app, global(tile_at(&app, 2)));
            std::thread::sleep(Duration::from_millis(600));
            os::right_click(&app, global(tile_at(&app, 0)));
            std::thread::sleep(Duration::from_millis(700));
            screen_shot(&app, &name("tile-menu-reset-off"));
            press(&app, "escape");
            profile_menu_at_the_display_bottom(&app, &format!("profiles-menu-{appearance}-{width}"));
            run_command(&app, "new profile", "New Profile");
            until(&app, "the name field", 10, |a| has(a, "profile-name"));
            type_real(&app, "default");
            hid(&app, key::ENTER, &[]);
            until(&app, "the name refused", 10, |a| has(a, "profile-name-error"));
            shot(&app, &format!("profiles-name-taken-{appearance}-{width}"));
            press(&app, "escape");
            quit(&app);
            drop(app);
        }
    }
}

/// Under `pinStyle = "letters"` a loaded pin whose page has a favicon still draws its letter.
#[test]
fn real_letters_pin_style_keeps_the_letter() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let profile = profile("pins-letters");
    write_pinned(&profile, &base, 2, 3, 1000);
    std::fs::create_dir_all(profile.join("config")).unwrap();
    std::fs::write(profile.join("config/config.toml"), "pinStyle = \"letters\"\n").unwrap();
    let app = launch_on(&profile, &[]);
    until(&app, "the pinned page", 30, |a| tiles(a).len() == 2 && shown_title(a) == "Alpha");
    // Long enough for the page's icon to have replaced the letter under the icons style.
    std::thread::sleep(Duration::from_secs(3));
    shot(&app, "pins-letters-style");
    assert_eq!(letters(&app), 2, "both tiles draw their letter");
    quit(&app);
}

/// The pinned tiles drawing their letter, not a favicon.
fn letters(app: &App) -> usize {
    let Ok(tree) = app.tree(None) else { return 0 };
    tree.windows.first().map_or(0, |w| {
        w.root
            .iter()
            .filter(|n| n.visible && n.test_id.as_deref().is_some_and(|t| t.starts_with("pin-letter-")))
            .count()
    })
}

/// The main display's size in points.
#[cfg(target_os = "macos")]
fn display_size() -> (f64, f64) {
    let out = std::process::Command::new("osascript")
        .args([
            "-l",
            "JavaScript",
            "-e",
            "ObjC.import('AppKit'); var f = $.NSScreen.mainScreen.frame; '0,0,' + f.size.width + ',' + f.size.height",
        ])
        .output()
        .unwrap();
    let n: Vec<f64> = String::from_utf8_lossy(&out.stdout).split(',').filter_map(|v| v.trim().parse().ok()).collect();
    (n[2], n[3])
}

/// The open menu's frame in global points: the app's one on-screen window above the normal
/// layer.
#[cfg(target_os = "macos")]
fn open_menu_frame(app: &App) -> Option<(f64, f64, f64, f64)> {
    let normal: Vec<u32> = ndshell::window_list::of_pid(app.pid()).iter().map(|w| w.number).collect();
    ndshell::window_list::every_layer_of_pid(app.pid())
        .into_iter()
        .find(|w| !normal.contains(&w.number) && w.bounds.3 > 40.)
        .map(|w| w.bounds)
}

/// Parks the window on the display's bottom edge, opens the foot's profile menu and captures the
/// whole display: the menu opens upward from the glyph and stays on screen.
#[cfg(target_os = "macos")]
fn profile_menu_at_the_display_bottom(app: &App, name: &str) {
    let (_, display_height) = display_size();
    let (x, _, _, h) = front_window(app).bounds;
    os::move_window(app, x as i32, (display_height - h) as i32);
    let (wx, wy, _, _) = front_window(app).bounds;
    let glyph = app.get_by_test_id("profile-menu").bounding_box().unwrap().expect("the profile glyph");
    real_click(app, glyph.x + glyph.width / 2., glyph.y + glyph.height / 2.);
    std::thread::sleep(Duration::from_millis(700));
    screen_shot(app, name);
    let menu = open_menu_frame(app).expect("the profile menu is open");
    press(app, "escape");
    let glyph_top = wy + f64::from(glyph.y);
    assert!(menu.1 + menu.3 <= glyph_top + 2., "the menu opens upward from the glyph: {menu:?}, glyph top {glyph_top}");
    assert!(menu.1 >= 0. && menu.1 + menu.3 <= display_height, "the menu stays on screen: {menu:?}");
    assert!(menu.0 >= wx - 1., "the menu starts at the glyph, inside the window: {menu:?}");
}

/// A tab dragged to the top of a sidebar with nothing pinned finds a pin target there, and
/// dropping it pins the tab.
#[test]
fn real_a_row_dropped_on_the_empty_pinned_block_pins() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch_with(&[format!("{base}/alpha"), format!("{base}/press")]);
    until(&app, "two rows", 30, |a| titles(a).len() == 2 && tiles(a).is_empty());
    let (wx, wy, _, _) = front_window(&app).bounds;
    let global = |p: (f32, f32)| (wx + f64::from(p.0), wy + f64::from(p.1));
    let strip = app.get_by_test_id("pin-target").bounding_box().unwrap().expect("the empty pinned block");
    let from = global(row_at(&app, 1));
    let to = global((strip.x + strip.width / 2., strip.y + strip.height / 2.));
    os::hold(&app, from);
    for i in 1..=16 {
        let t = f64::from(i) / 16.;
        let at = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        os::drag_move(&app, at);
        std::thread::sleep(Duration::from_millis(30));
    }
    until(&app, "the pin target", 5, |a| has(a, "pin-target-label"));
    shot(&app, "pins-empty-target");
    let open = app.get_by_test_id("pin-target").bounding_box().unwrap().expect("the target");
    os::release(&app, global((open.x + open.width / 2., open.y + open.height / 2.)));
    until(&app, "the row pinned", 10, |a| tiles(a).len() == 1 && titles(a).len() == 1);
    assert!(lacks(&app, "pin-target"), "the target gives way to the tiles");
    quit(&app);
}

/// Every window's nodes with `test_id`.
fn count_ids(app: &App, test_id: &str) -> usize {
    let Ok(tree) = app.tree(None) else { return 0 };
    tree.windows.iter().flat_map(|w| w.root.iter()).filter(|n| n.test_id.as_deref() == Some(test_id)).count()
}

/// A profile is renamed in place; a profile whose last window closed opens again with its
/// session when switched to; history and zoom levels stay with the profile they were made in,
/// on the same host.
#[test]
fn real_rename_and_reopen_a_closed_profile() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let dir = profile("profiles-closed");
    let app = launch_on(&dir, &[format!("{base}/alpha")]);
    until(&app, "the first window", 30, |a| shown_title(a) == "Alpha");
    new_profile(&app, "Work");
    go_to(&app, &format!("{base}/press"));
    until(&app, "Work on Press", 15, |a| all_titles(a).contains(&"Press".to_owned()));
    go_to(&app, &format!("{base}/beta"));
    until(&app, "Work on Beta", 15, |a| all_titles(a).contains(&"Beta".to_owned()));

    run_command(&app, "rename profile", "Rename Profile");
    until(&app, "the name field", 10, |a| has(a, "profile-name"));
    hid(&app, os::key_of('a'), &[os::CMD]);
    paste(&app, "Office");
    std::thread::sleep(Duration::from_millis(300));
    hid(&app, key::ENTER, &[]);
    until(&app, "the popover gone", 10, |a| lacks(a, "profile-name"));
    assert_eq!(profiles_in(&dir)[1].1, "Office", "renamed in place, same id");

    // Zoom on Office's page leaves Default's page on the same host at 100%.
    press(&app, "cmd-=");
    until(&app, "Office zoomed", 10, |a| count_ids(a, "zoom") == 1);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(count_ids(&app, "zoom"), 1, "only Office's window shows a zoom level");

    // History: Office has been to Press, Default has not.
    open_bar(&app, key::T);
    type_real(&app, "press");
    until(&app, "Office's history", 10, |a| bar_rows(a).iter().any(|r| r.starts_with("Press")));
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);

    os::close_window(&app, &front_window(&app));
    until(&app, "Office's window closed", 15, |a| window_count(a) == 1);
    until(&app, "a profile with no window is closed", 5, |_| !profiles_in(&dir)[1].2);
    open_bar(&app, key::T);
    type_real(&app, "press");
    std::thread::sleep(Duration::from_secs(1));
    let rows = bar_rows(&app);
    assert!(!rows.iter().any(|r| r.starts_with("Press")), "Default's history has no Press: {rows:?}");
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
    assert_eq!(count_ids(&app, "zoom"), 0, "Default's page on the same host is at 100%");

    run_command(&app, "switch to office", "Switch to Office");
    until(&app, "Office back with its session", 20, |a| window_count(a) == 2 && all_titles(a) == ["Alpha", "Beta"]);
    until(&app, "and its zoom", 10, |a| count_ids(a, "zoom") == 1);
    assert!(profiles_in(&dir)[1].2, "open again");
    quit(&app);
}

/// A row dragged along the list shows the slot it would drop into, and drops there.
#[test]
fn real_a_row_dragged_in_the_list_marks_its_slot() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch_with(&[format!("{base}/alpha"), format!("{base}/beta"), format!("{base}/press")]);
    until(&app, "three rows", 30, |a| titles(a).len() == 3);
    let before = rows(&app);
    let (wx, wy, _, _) = front_window(&app).bounds;
    let global = |p: (f32, f32)| (wx + f64::from(p.0), wy + f64::from(p.1));
    // Between the second and the third row.
    let from = global(before[0].1);
    let to = global(((before[1].1.0 + before[2].1.0) / 2., (before[1].1.1 + before[2].1.1) / 2.));
    os::hold(&app, from);
    for i in 1..=20 {
        let t = f64::from(i) / 20.;
        let at = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        os::drag_move(&app, at);
        std::thread::sleep(Duration::from_millis(30));
    }
    std::thread::sleep(Duration::from_millis(400));
    shot(&app, "list-slot-line");
    os::release(&app, to);
    until(&app, "the first row moved into the slot", 10, |a| {
        let now: Vec<String> = rows(a).into_iter().map(|r| r.0).collect();
        now == [before[1].0.clone(), before[0].0.clone(), before[2].0.clone()]
    });
    quit(&app);
}

/// The first window's list top to bottom: a tab as its title, a separator as `| <name>`.
fn sidebar_order(app: &AppHandle) -> Vec<String> {
    let Ok(tree) = app.tree(None) else { return Vec::new() };
    let window = &tree.windows[0];
    let name_of = |id: &str| {
        window
            .root
            .iter()
            .find(|n| n.test_id.as_deref() == Some(&format!("separator-title-{id}")))
            .map_or(String::new(), |n| n.name.clone())
    };
    let mut rows: Vec<(f32, String)> = window
        .root
        .iter()
        .filter(|n| n.visible)
        .filter_map(|n| {
            let y = n.bounds?.origin.y;
            let id = n.test_id.as_deref()?;
            if id.starts_with("tab-title-") {
                Some((y, n.name.clone()))
            } else if let Some(sep) = id.strip_prefix("separator-")
                && !sep.starts_with("title-")
                && sep != "name"
            {
                Some((y, format!("| {}", name_of(sep)).trim_end().to_owned()))
            } else {
                None
            }
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, r)| r).collect()
}

/// The centre of the first window's separator row named `name` (`""` for an unnamed one), in
/// window points.
fn separator_at(app: &App, name: &str) -> (f32, f32) {
    let tree = app.tree(None).unwrap();
    let window = &tree.windows[0];
    let order = sidebar_order(app);
    let wanted = format!("| {name}").trim_end().to_owned();
    let nth = order.iter().filter(|r| r.starts_with('|')).position(|r| *r == wanted).expect("the separator");
    let mut seps: Vec<_> = window
        .root
        .iter()
        .filter(|n| {
            n.test_id
                .as_deref()
                .and_then(|t| t.strip_prefix("separator-"))
                .is_some_and(|s| !s.starts_with("title-") && s != "name")
        })
        .filter_map(|n| n.bounds)
        .collect();
    seps.sort_by(|a, b| a.origin.y.total_cmp(&b.origin.y));
    let r = seps[nth];
    (r.origin.x + r.size.width / 2., r.origin.y + r.size.height / 2.)
}

/// Waits for the first window's list to read `want`, naming what it read when it does not.
#[track_caller]
fn until_order(app: &App, what: &str, want: &[&str]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while sidebar_order(app) != want {
        if Instant::now() > deadline {
            shot(app, "order-timeout");
            panic!("timed out waiting for {what}: the list reads {:?}, not {want:?}", sidebar_order(app));
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// The separator name field's text and whether it has the keyboard, while one is open.
fn name_field(app: &AppHandle) -> Option<(String, bool)> {
    let tree = app.tree(None).ok()?;
    tree.windows.iter().find_map(|w| {
        w.root
            .iter()
            .find(|n| n.test_id.as_deref() == Some("separator-name"))
            .map(|n| (n.value.clone().unwrap_or_default(), n.focused))
    })
}

/// Named separators: one added under a tab from its menu opens with its name field focused and
/// Enter keeps the name, Esc leaves an unnamed rule, the bar renames and deletes the one nearest
/// above the shown tab, a separator drags like a row, and all of it comes back after a restart.
#[test]
fn real_named_separators() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let dir = profile("separators");
    let app = launch_on(&dir, &[format!("{base}/alpha"), format!("{base}/beta"), format!("{base}/press")]);
    until(&app, "three rows", 30, |a| titles(a).len() == 3);
    for (n, title) in ["Alpha", "Beta", "Press"].into_iter().enumerate() {
        press(&app, &format!("cmd-{}", n + 1));
        until(&app, title, 15, |a| shown_title(a) == title);
    }
    press(&app, "cmd-1");

    pick_menu(&app, row_at(&app, 0), "Add Separator");
    until(&app, "the name field focused", 10, |a| name_field(a).is_some_and(|f| f.1));
    type_real(&app, "Reading");
    let deadline = Instant::now() + Duration::from_secs(10);
    while name_field(&app).is_none_or(|f| f.0 != "Reading") {
        assert!(
            Instant::now() < deadline,
            "the typed name never reached its field: {:?}\nwindows {:?}\n{}\n{}",
            name_field(&app),
            app.windows(),
            os::focus_report(&app),
            app.stderr_tail(30).join("\n")
        );
        std::thread::sleep(Duration::from_millis(150));
    }
    hid(&app, key::ENTER, &[]);
    until_order(&app, "the named separator", &["Alpha", "| Reading", "Beta", "Press"]);
    shot(&app, "separator-named");

    pick_menu(&app, row_at(&app, 1), "Add Separator");
    until(&app, "the second name field", 10, |a| has(a, "separator-name"));
    hid(&app, key::ESCAPE, &[]);
    until_order(&app, "an unnamed rule", &["Alpha", "| Reading", "Beta", "|", "Press"]);

    press(&app, "cmd-3");
    until(&app, "Press shown", 10, |a| shown_title(a) == "Press");
    run_command(&app, "rename separator", "Rename Separator");
    until(&app, "the rename field", 10, |a| has(a, "separator-name"));
    type_real(&app, "Later");
    hid(&app, key::ENTER, &[]);
    until_order(&app, "the rule named", &["Alpha", "| Reading", "Beta", "| Later", "Press"]);

    // The separator drags to the end of the list, past the tabs.
    let (wx, wy, _, _) = front_window(&app).bounds;
    let global = |p: (f32, f32)| (wx + f64::from(p.0), wy + f64::from(p.1));
    let from = global(separator_at(&app, "Reading"));
    let last = row_at(&app, 2);
    let to = global((last.0, last.1 + 14.));
    os::hold(&app, from);
    for i in 1..=20 {
        let t = f64::from(i) / 20.;
        let at = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
        os::drag_move(&app, at);
        std::thread::sleep(Duration::from_millis(30));
    }
    std::thread::sleep(Duration::from_millis(400));
    shot(&app, "separator-drag");
    os::release(&app, to);
    until_order(&app, "the separator at the end", &["Alpha", "Beta", "| Later", "Press", "| Reading"]);

    run_command(&app, "delete separator", "Delete Separator");
    until_order(&app, "Later deleted", &["Alpha", "Beta", "Press", "| Reading"]);
    quit(&app);
    drop(app);

    let app = launch_on(&dir, &[]);
    until(&app, "three rows again", 30, |a| titles(a).len() == 3);
    until_order(&app, "the separator after a restart", &["Alpha", "Beta", "Press", "| Reading"]);
    quit(&app);
}

/// The bar's Add Separator Below lands under the shown tab; the separator's menu renames it with
/// its name selected and deletes it with the tabs kept; it takes no hover fill and a click never
/// shows it; a tab drags across it, and it dropped on the pins goes back to its place.
#[test]
fn real_separator_menu_and_bar() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch_with(&[format!("{base}/alpha"), format!("{base}/beta"), format!("{base}/press")]);
    until(&app, "three rows", 30, |a| titles(a).len() == 3);
    for (n, title) in ["Alpha", "Beta", "Press"].into_iter().enumerate() {
        press(&app, &format!("cmd-{}", n + 1));
        until(&app, title, 15, |a| shown_title(a) == title);
    }
    press(&app, "cmd-2");
    until(&app, "Beta shown", 15, |a| shown_title(a) == "Beta");
    run_command(&app, "add separator", "Add Separator Below");
    until(&app, "the name field focused", 10, |a| name_field(a).is_some_and(|f| f.1));
    type_real(&app, "Work");
    until(&app, "the name typed", 10, |a| name_field(a).is_some_and(|f| f.0 == "Work"));
    hid(&app, key::ENTER, &[]);
    until_order(&app, "the separator under the shown tab", &["Alpha", "Beta", "| Work", "Press"]);

    pick_menu(&app, separator_at(&app, "Work"), "Rename Separator");
    until(&app, "the rename field with the name", 10, |a| name_field(a).is_some_and(|f| f.1 && f.0 == "Work"));
    type_real(&app, "Play");
    until(&app, "the selected name replaced", 10, |a| name_field(a).is_some_and(|f| f.0 == "Play"));
    hid(&app, key::ENTER, &[]);
    until_order(&app, "renamed from its menu", &["Alpha", "Beta", "| Play", "Press"]);

    let win = front_window(&app);
    let (wx, wy, _, _) = win.bounds;
    let sep = separator_at(&app, "Play");
    let sidebar = app.get_by_test_id("sidebar").bounding_box().unwrap().expect("the sidebar is laid out");
    let probe = (f64::from(sidebar.x + sidebar.width - 24.), f64::from(sep.1 - 7.));
    os::move_to(&app, (wx + 400., wy + 300.));
    std::thread::sleep(Duration::from_millis(400));
    let rest = os::pixel(&app, &win, &shot(&app, "separator-rest"), probe);
    os::move_to(&app, (wx + probe.0, wy + probe.1));
    std::thread::sleep(Duration::from_millis(400));
    let hovered = os::pixel(&app, &win, &shot(&app, "separator-hover"), probe);
    assert_eq!(rest, hovered, "the separator took a hover fill");
    real_click(&app, sep.0, sep.1);
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(shown_title(&app), "Beta", "a click on the separator changed the shown tab");

    let global = |p: (f32, f32)| (wx + f64::from(p.0), wy + f64::from(p.1));
    let from = row_at(&app, 2);
    let to = row_at(&app, 1);
    os::drag(&app, global(from), f64::from(to.0 - from.0), f64::from(to.1 - from.1 - 6.), 20);
    until_order(&app, "Press dragged across the separator", &["Alpha", "Press", "Beta", "| Play"]);

    pick_menu(&app, row_at(&app, 0), "Pin Tab");
    until(&app, "alpha pinned", 10, |a| tiles(a).len() == 1);
    until_order(&app, "the list without alpha", &["Press", "Beta", "| Play"]);
    let from = separator_at(&app, "Play");
    let (_, (tx, ty, tw, th)) = tiles(&app)[0].clone();
    os::drag(&app, global(from), f64::from(tx + tw * 1.5 - from.0), f64::from(ty + th / 2. - from.1), 20);
    std::thread::sleep(Duration::from_millis(600));
    until_order(&app, "the separator back in its place", &["Press", "Beta", "| Play"]);
    assert_eq!(tiles(&app).len(), 1, "the separator made a tile");

    pick_menu(&app, separator_at(&app, "Play"), "Delete Separator");
    until_order(&app, "deleted from its menu, the tabs kept", &["Press", "Beta"]);
    quit(&app);
}

/// Tidy Tabs groups the loose tabs under named separators, the same rows as the ones made by
/// hand: by the system's on-device model where it has one, else by site; Undo Tidy puts the list
/// back, and once the list changes it is no longer offered.
#[test]
fn real_tidy_tabs_groups_by_site_and_undoes() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let other = base.replace("127.0.0.1", "localhost");
    let app = launch_with(&[
        format!("{base}/alpha"),
        format!("{other}/beta"),
        format!("{base}/plain"),
        format!("{other}/press"),
    ]);
    until(&app, "four rows", 30, |a| titles(a).len() == 4);
    for (n, title) in ["Alpha", "Beta", "Plain", "Press"].into_iter().enumerate() {
        press(&app, &format!("cmd-{}", n + 1));
        until(&app, title, 15, |a| shown_title(a) == title);
    }
    let untidy = sidebar_order(&app);
    assert_eq!(untidy, ["Alpha", "Beta", "Plain", "Press"]);
    let (here, there) = (base.trim_start_matches("http://"), other.trim_start_matches("http://"));
    let tidied =
        [format!("| {here}"), "Alpha".into(), "Plain".into(), format!("| {there}"), "Beta".into(), "Press".into()];

    // Where the system has an on-device model its groups win, so the order is the model's: every
    // tab once, each group under a named separator.
    let grouped = |a: &App| {
        let order = sidebar_order(a);
        if a.stderr_tail(400).iter().any(|l| l.contains("lynk: tidy grouped by Model")) {
            let mut tabs: Vec<_> = order.iter().filter(|r| !r.starts_with('|')).cloned().collect();
            tabs.sort();
            order.first().is_some_and(|r| r.len() > 2 && r.starts_with("| "))
                && tabs == ["Alpha", "Beta", "Plain", "Press"]
                && order.windows(2).all(|w| !(w[0].starts_with('|') && w[1].starts_with('|')))
        } else {
            order == tidied
        }
    };
    run_command(&app, "tidy tabs", "Tidy Tabs");
    until(&app, "the tabs grouped", 30, grouped);
    shot(&app, "tidy-grouped");
    run_command(&app, "undo tidy", "Undo Tidy");
    until(&app, "the list as it was", 10, |a| sidebar_order(a) == untidy);

    run_command(&app, "tidy tabs", "Tidy Tabs");
    until(&app, "grouped again", 30, grouped);
    press(&app, "cmd-w");
    until(&app, "a tab closed", 10, |a| titles(a).len() == 3);
    open_bar(&app, key::K);
    type_real(&app, "undo tidy");
    std::thread::sleep(Duration::from_millis(600));
    let rows = bar_rows(&app);
    assert!(!rows.iter().any(|r| r.starts_with("Undo Tidy")), "undo offered over a change: {rows:?}");
    hid(&app, key::ESCAPE, &[]);
    bar_closed(&app);
    quit(&app);
}

/// The centre of `test_id` in window points.
fn centre_of(app: &AppHandle, test_id: &str) -> Option<(f32, f32)> {
    let b = app.get_by_test_id(test_id).bounding_box().ok()??;
    Some((b.x + b.width / 2., b.y + b.height / 2.))
}

/// A restored window: one pin, then `Alpha` shown and three rows never loaded, `R2` (beta),
/// `R3` (a load that never ends) and `R4` (press).
fn write_restored(profile: &std::path::Path, base: &str) {
    let tab = |id: &str, url: String, title: &str| serde_json::json!({ "id": id, "url": url, "title": title });
    let mut pin = tab("t1", format!("{base}/alpha?p1"), "P1");
    pin["pinnedUrl"] = pin["url"].clone();
    let tabs = [
        pin,
        tab("t2", format!("{base}/alpha"), "Alpha"),
        tab("t3", format!("{base}/beta"), "R2"),
        tab("t4", format!("{base}/slow"), "R3"),
        tab("t5", format!("{base}/press"), "R4"),
    ];
    let items: Vec<_> =
        ["t2", "t3", "t4", "t5"].iter().map(|id| serde_json::json!({ "type": "tab", "id": id })).collect();
    let doc = serde_json::json!({ "version": 3, "data": {
        "spaces": [{ "id": "s1", "name": "Space" }],
        "windows": [{ "id": "w1", "tabs": tabs, "activeId": "t2", "width": 1100, "height": 800,
            "spaces": { "shown": "s1", "sidebars": { "s1": { "pins": ["t1"], "items": items } } } }],
    } });
    std::fs::write(profile.join("session.json"), doc.to_string()).unwrap();
}

/// Hovers the `index`th row with the real pointer and clicks the close button it shows. Returns
/// where the pointer was left.
fn close_by_x(app: &App, index: usize) -> (f32, f32) {
    let (wx, wy, _, _) = front_window(app).bounds;
    let (row, at) = rows(app)[index].clone();
    let close = row.replace("tab-row-", "tab-close-");
    os::move_to(app, (wx + f64::from(at.0), wy + f64::from(at.1)));
    until(app, "the hovered row's close button", 10, |a| has(a, &close));
    let at = centre_of(app, &close).expect("the close button is laid out");
    real_click(app, at.0, at.1);
    at
}

/// Shows the `index`th row with a real click and waits for its page.
fn show_row(app: &App, index: usize, title: &str) {
    let at = row_at(app, index);
    real_click(app, at.0, at.1);
    until(app, "the clicked tab shown", 15, |a| shown_title(a).starts_with(title));
}

/// Whether every page in the window belongs to a tab still drawn, as a row or a pin.
fn no_page_left_behind(app: &AppHandle) -> bool {
    let Ok(tree) = app.tree(None) else { return false };
    let tabs: Vec<&str> = tree.windows[0]
        .root
        .iter()
        .filter_map(|n| {
            let id = n.test_id.as_deref()?;
            id.strip_prefix("tab-row-").or_else(|| id.strip_prefix("pin-"))
        })
        .collect();
    tree.windows[0].root.iter().filter_map(page_of).all(|tab| tabs.contains(&tab))
}

/// One basic tab action from one entry point: what it does to [`write_restored`]'s window, the
/// rows' titles and the pins it leaves, and the shown tab's title where the action decides it.
struct TabCase {
    name: &'static str,
    act: fn(&App, &str),
    rows: &'static [&'static str],
    pins: usize,
    shown: Option<&'static str>,
}

const TAB_CASES: &[TabCase] = &[
    TabCase {
        name: "x on a never-loaded row",
        act: |a, _| _ = close_by_x(a, 1),
        rows: &["Alpha", "R3", "R4"],
        pins: 1,
        shown: Some("Alpha"),
    },
    TabCase {
        name: "x on the last row",
        act: |a, _| _ = close_by_x(a, 3),
        rows: &["Alpha", "R2", "R3"],
        pins: 1,
        shown: Some("Alpha"),
    },
    TabCase {
        name: "x, then the row that slides under the pointer",
        act: |a, _| {
            let at = close_by_x(a, 1);
            until(a, "the first close", 10, |a| titles(a).len() == 3);
            let next = rows(a)[1].0.replace("tab-row-", "tab-close-");
            until(a, "the next row's close button under the pointer", 10, |a| has(a, &next));
            real_click(a, at.0, at.1);
        },
        rows: &["Alpha", "R4"],
        pins: 1,
        shown: Some("Alpha"),
    },
    TabCase {
        name: "x on a loading row",
        act: |a, _| {
            let at = row_at(a, 2);
            real_click(a, at.0, at.1);
            until(a, "the never-ending load", 10, |a| has(a, "load-bar"));
            show_row(a, 0, "Alpha");
            close_by_x(a, 2);
        },
        rows: &["Alpha", "R2", "R4"],
        pins: 1,
        shown: Some("Alpha"),
    },
    TabCase {
        name: "row menu Close Tab",
        act: |a, _| pick_menu(a, row_at(a, 1), "Close Tab"),
        rows: &["Alpha", "R3", "R4"],
        pins: 1,
        shown: Some("Alpha"),
    },
    TabCase {
        name: "close shortcut after picking the row by key",
        act: |a, _| {
            a.get_by_test_id("tabs").press("down").unwrap();
            until(a, "the next tab, picked by key", 15, |a| shown_title(a) == "Beta");
            // A key sent while another app holds the front goes to that app.
            os::in_front(a);
            press(a, "cmd-w");
        },
        rows: &["Alpha", "R3", "R4"],
        pins: 1,
        shown: None,
    },
    TabCase {
        name: "pin menu Close Tab",
        act: |a, _| pick_menu(a, tile_at(a, 0), "Close Tab"),
        rows: &["Alpha", "R2", "R3", "R4"],
        pins: 0,
        shown: Some("Alpha"),
    },
    TabCase {
        name: "Close Other Tabs over asleep, loading and never-loaded rows spares the pin",
        act: |a, _| {
            show_row(a, 1, "Beta");
            show_row(a, 2, "");
            until(a, "the never-ending load", 10, |a| has(a, "load-bar"));
            show_row(a, 0, "Alpha");
            pick_menu(a, row_at(a, 1), "Put Tab");
            until(a, "the second tab asleep", 15, |a| asleep(a, 1));
            pick_menu(a, row_at(a, 0), "Close Other");
        },
        rows: &["Alpha"],
        pins: 1,
        shown: Some("Alpha"),
    },
    TabCase {
        name: "Close Tabs Below",
        act: |a, _| pick_menu(a, row_at(a, 1), "Close Tabs Below"),
        rows: &["Alpha", "R2"],
        pins: 1,
        shown: Some("Alpha"),
    },
    TabCase {
        // Reopened at the end of the list, where Open::End puts it.
        name: "reopen a closed background tab",
        act: |a, _| {
            close_by_x(a, 1);
            until(a, "the close", 10, |a| titles(a).len() == 3);
            os::in_front(a);
            press(a, "cmd-shift-t");
        },
        rows: &["Alpha", "R3", "R4", "Beta"],
        pins: 1,
        shown: None,
    },
    TabCase {
        name: "row menu Duplicate",
        act: |a, _| pick_menu(a, row_at(a, 1), "Duplicate"),
        rows: &["Alpha", "R2", "Beta", "R3", "R4"],
        pins: 1,
        shown: Some("Beta"),
    },
    TabCase {
        name: "row menu Pin Tab",
        act: |a, _| pick_menu(a, row_at(a, 1), "Pin Tab"),
        rows: &["Alpha", "R3", "R4"],
        pins: 2,
        shown: Some("Alpha"),
    },
    TabCase {
        name: "pin menu Unpin",
        act: |a, _| pick_menu(a, tile_at(a, 0), "Unpin"),
        rows: &["P1", "Alpha", "R2", "R3", "R4"],
        pins: 0,
        shown: Some("Alpha"),
    },
    TabCase {
        name: "new tab button",
        act: |a, base| {
            let at = centre_of(a, os::NEW_TAB).expect("the new tab button");
            real_click(a, at.0, at.1);
            until(a, "the bar", 10, |a| bar_field(a).is_some());
            type_real(a, &format!("{base}/beta?new"));
            hid(a, key::ENTER, &[]);
        },
        rows: &["Alpha", "R2", "R3", "R4", "Beta"],
        pins: 1,
        shown: Some("Beta"),
    },
    TabCase {
        name: "switch by clicking a row",
        act: |a, _| show_row(a, 1, "Beta"),
        rows: &["Alpha", "Beta", "R3", "R4"],
        pins: 1,
        shown: Some("Beta"),
    },
];

/// Every basic tab action from each of its entry points, with the real pointer, on a restored
/// window of a pin, a shown tab and rows never loaded. Each leaves exactly the tabs it should,
/// keeps the shown tab where it is not the one acted on, leaves no page behind for a closed tab,
/// and a restart brings back the same window.
#[test]
fn real_tab_actions_over_every_page_state() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for case in TAB_CASES {
        let profile = profile("tab-actions");
        write_restored(&profile, &base);
        let app = launch_on(&profile, &[]);
        until(&app, "the restored window", 30, |a| {
            titles(a) == ["Alpha", "R2", "R3", "R4"] && tiles(a).len() == 1 && shown_title(a) == "Alpha"
        });
        // Every case starts with the app in front; a shared screen hands the front to whatever
        // runs beside the drive, and the case's keys and clicks would go there.
        os::in_front(&app);
        (case.act)(&app, &base);
        let left = |a: &App| titles(a) == case.rows && tiles(a).len() == case.pins;
        until(&app, case.name, 15, left);
        if let Some(shown) = case.shown {
            until(&app, &format!("{}: the shown tab", case.name), 15, |a| shown_title(a) == shown);
        }
        until(&app, &format!("{}: no page for a closed tab", case.name), 10, |a| no_page_left_behind(a));
        quit(&app);
        drop(app);
        let app = launch_on(&profile, &[]);
        until(&app, &format!("{}: the same window after a restart", case.name), 30, |a| {
            left(a) && case.shown.is_none_or(|s| shown_title(a) == s)
        });
        quit(&app);
    }
}
