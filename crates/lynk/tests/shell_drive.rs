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

const FULL: &str = r#"<!doctype html><title>Full</title>
<body style="margin:0"><button style="width:100%;height:300px" onclick="document.documentElement.requestFullscreen()">Go fullscreen</button></body>"#;

/// No colours of its own: Chromium's default white is what it sits on.
const PLAIN: &str = "<!doctype html><title>Plain</title><p>No colours of its own.";
/// Two requests EasyList stops wherever they are served (`/pagead/conversion.js$script`,
/// `/banner_ads/*`), an element its generic `##.ad_banner` hides (except on 127.0.0.1, where
/// EasyList's `@@://127.0.0.1$generichide` turns generic rules off) and one the user hid on
/// 127.0.0.1. The title reports what each did: `Ads <ad_banner> <promo> <script>`.
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
    let path = target.split('?').next().unwrap_or("/");
    if path == "/icon.png" {
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
        "/popups" => page(POPUPS),
        "/plain" => page(PLAIN),
        "/ads" => page(ADS),
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
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
    app.tree(None).is_ok_and(|t| {
        t.windows.iter().any(|w| w.root.iter().any(|n| n.test_id.as_deref() == Some(test_id) && n.visible))
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
    os::key(&app, key::F, &[Modifier::Control, Modifier::Command]);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut grown_while_moving = false;
    while !settled_fullscreen(&app) {
        assert!(Instant::now() < deadline, "the window never settled in fullscreen");
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

/// The DevTools' own title, address, icon and loads never reach their tab: docked, in a window of
/// their own and after they close, the tab keeps its page's title and the bar its address.
#[test]
fn real_dev_tools_leave_their_tab_as_it_was() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/plain"));
    until(&app, "the page", 30, |a| titles(a) == ["Plain"]);
    let address = format!("{}/plain", base.trim_start_matches("http://"));
    let unchanged = |app: &App, when: &str| {
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(titles(app), ["Plain"], "the tab's title {when}");
        assert!(lacks(app, "load-bar"), "no load on the tab {when}");
        // Into the page, near its right edge, which a DevTools window of their own leaves clear;
        // the keyboard goes back to the page from the DevTools.
        click_node(app, "the page", |n| n.test_id.as_deref() == Some("content"), 0.95);
        open_bar(app, key::L);
        let field = bar_field(app).unwrap_or_default();
        assert!(field.contains(&address) && !field.contains("devtools"), "the bar {when}: {field}");
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
/// else the test's tmp dir.
fn blocking_profile(name: &str) -> PathBuf {
    let dir = profile(name);
    let zip = std::env::var_os("LYNK_UBO_ZIP").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("uBlock0_{}.chromium.zip", lynk_core::ublock::PINNED_VERSION))
    });
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

/// The live pages of the first window, a sleeping tab having none. `None` when the app gave no
/// tree or has no window, so a wait for none never passes on a crash.
fn live_pages(app: &AppHandle) -> Option<usize> {
    let tree = app.tree(None).ok()?;
    tree.windows.first().map(|w| {
        w.root
            .iter()
            .filter(|n| {
                // `page-{tab}-{page}`: the tab's id, then which of its pages this is.
                n.test_id
                    .as_deref()
                    .and_then(|t| t.strip_prefix("page-"))
                    .and_then(|id| id.split_once('-'))
                    .is_some_and(|(tab, page)| tab.parse::<u64>().is_ok() && page.parse::<u64>().is_ok())
            })
            .count()
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
    until(&app, "the tab asleep", 15, |a| live_pages(a) == Some(1));
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
    until(&app, "the busy tab asleep", 5, |a| live_pages(a) == Some(1));
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
    until(&app, "the tab asleep", 15, |a| live_pages(a) == Some(1));
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
    let before = os::windows_above(app);
    os::right_click(app, (wx + f64::from(at.0), wy + f64::from(at.1)));
    // Typed only once the menu's own window is up: letters sent to a menu still opening are lost,
    // and type-select then lands on another item.
    let deadline = Instant::now() + Duration::from_secs(10);
    while os::windows_above(app) <= before {
        assert!(Instant::now() < deadline, "the menu for {item} never opened");
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
fn real_close_others_over_every_page_state() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    // Shown and live, asleep, loading and never finishing, never loaded.
    let app =
        launch_with(&[format!("{base}/alpha"), format!("{base}/long"), format!("{base}/slow"), format!("{base}/beta")]);
    until(&app, "the first page", 30, |a| titles(a).first().is_some_and(|t| t == "Alpha"));
    press(&app, "cmd-2");
    until(&app, "the second page", 15, |a| titles(a).get(1).is_some_and(|t| t.starts_with("Long")));
    press(&app, "cmd-3");
    until(&app, "the never-ending load", 10, |a| has(a, "load-bar"));
    press(&app, "cmd-1");
    pick_menu(&app, row_at(&app, 1), "Put Tab");
    until(&app, "the second tab asleep", 15, |a| live_pages(a) == Some(2));
    pick_menu(&app, row_at(&app, 0), "Close Other");
    until(&app, "only the shown tab", 15, |a| titles(a) == ["Alpha"]);
    assert_eq!(live_pages(&app), Some(1));
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
        until(&app, "Beta asleep", 15, |a| live_pages(a) == Some(3));
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
    // Shown by the next key, then quit before the store's debounce has passed.
    app.keyboard().press(&os::chord("cmd-1")).unwrap();
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
            let (wx, wy, _, _) = front_window(&app).bounds;
            let at = row_at(&app, 3);
            os::scroll(&app, (wx + f64::from(at.0), wy + f64::from(at.1)), -800, 8);
            until(&app, "the list scrolled", 10, |a| rows(a) != before_rows);
            let after: Vec<_> = settled_tiles(&app).into_iter().map(|t| t.1).collect();
            assert_eq!(after, laid.iter().map(|t| t.1).collect::<Vec<_>>(), "the tiles moved with the list");
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
    let at = loop {
        let rows = bar_rows(app);
        if let Some(at) = rows.iter().position(|r| r.starts_with(row)) {
            break at;
        }
        assert!(Instant::now() < deadline, "no {row} row for {typed:?}: {rows:?}");
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
    // Keys typed the moment the switched-to window comes forward can land before its bar does.
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
    let app = launch_on(&dir, &[]);
    until(&app, "both profiles back, still apart", 30, |a| all_titles(a) == ["Cookie none", "Cookie who=one"]);
    let key_title = |a: &App| {
        a.windows().ok().and_then(|w| w.as_array().cloned()).and_then(|ws| {
            ws.iter().find(|w| w["key"] == true).map(|w| w["title"].as_str().unwrap_or_default().to_owned())
        })
    };
    for (row, title) in [("Switch to Default", "Cookie who=one"), ("Switch to Work", "Cookie none")] {
        os::in_front(&app);
        // The profile in front offers no switch to itself.
        if key_title(&app).as_deref() != Some(title) {
            run_command(&app, &row.to_lowercase(), row);
        }
        until(&app, title, 10, |a| key_title(a).as_deref() == Some(title));
    }
    quit(&app);
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
            press(&app, "cmd-shift-]");
            press(&app, "cmd-shift-]");
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
