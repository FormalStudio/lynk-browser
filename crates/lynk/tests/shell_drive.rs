//! Drives the real app over nd's automation socket against pages served from this process: one
//! case per shell box of `docs/nd-parity.md`. Every launch gets its own data directory (nd-test
//! makes one per launch, and the Chromium profile lives in it), so nothing of the user's is
//! read or written. Captures land in the test's target tmp dir.
//!
//! It opens real windows and moves the real pointer, so it runs only under nextest or with
//! `ND_REAL=1`, and on macOS until the Linux gate drives it.
#![cfg(target_os = "macos")]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use nd_test::{AppHandle, LaunchOptions, expect, launch_app};
use ndshell::input::Modifier;

fn real_allowed() -> bool {
    let allowed = std::env::var_os("NEXTEST").is_some() || std::env::var_os("ND_REAL").is_some_and(|v| v == "1");
    if !allowed {
        println!("skipped: opens a real window; run with ND_REAL=1");
    }
    allowed
}

const ALPHA: &str = r#"<!doctype html><title>Alpha</title>
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
const DARK_SCHEME: &str = "<!doctype html><meta name=color-scheme content=dark><title>Dark</title><p>Dark only.";
const BOTH_SCHEMES: &str =
    "<!doctype html><meta name=color-scheme content=\"light dark\"><title>Both</title><p>Either scheme.";

/// A title no sidebar row has room for.
const LONG_TITLE: &str =
    "<!doctype html><title>A page whose title runs on well past the width of any sidebar row</title>";

/// Opens pop-ups as fast as it can, never from a click.
const POPUPS: &str = r#"<!doctype html><title>Popups</title>
<body><script>setInterval(() => window.open("/beta?" + Math.random()), 20);</script></body>"#;

/// The close light's centre, in window points: its edges sit 19 in from the window's left and top
/// (docs/sidebar.md), and it is 14 across.
const CLOSE_LIGHT: (f32, f32) = (26., 26.);

/// Serves the fixture pages on a free local port for the rest of the test process, each request
/// on its own thread, since `/slow` never answers.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
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
    let _ = BufReader::new(&stream).read_line(&mut line);
    let target = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let path = target.split('?').next().unwrap_or("/");
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
        "/dark-scheme" => page(DARK_SCHEME),
        "/both-schemes" => page(BOTH_SCHEMES),
        "/long-title" => page(LONG_TITLE),
        "/many" => page(&format!("<!doctype html><title>Many</title><p>{}", "needle ".repeat(10_000))),
        // 50 MB of text under a title.
        "/big" => page(&format!("<!doctype html><title>Big</title><pre>{}", "x".repeat(50 << 20))),
        "/loop" => ("302 Found", "Location: /loop\r\n".to_owned(), String::new()),
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

fn launch(url: &str) -> AppHandle {
    let options = LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
        .env("LYNK_URL", url)
        .env("ND_CEF_STYLE", "chrome")
        .env("LYNK_SEARCHENGINE", "google");
    let app = launch_app(options).unwrap();
    activate(app.pid());
    app
}

/// Launches on `urls`, a tab each, as a launch from the command line does.
fn launch_with(urls: &[String]) -> AppHandle {
    let options = urls.iter().fold(
        LaunchOptions::new(env!("CARGO_BIN_EXE_lynk")).env("ND_CEF_STYLE", "chrome").env("LYNK_SEARCHENGINE", "google"),
        |o, url| o.arg(url),
    );
    let app = launch_app(options).unwrap();
    activate(app.pid());
    app
}

/// A profile of the test's own, which outlives one launch so the next can restore it.
fn profile(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Launches on `profile` with `urls` as the command line's.
fn launch_on(profile: &std::path::Path, urls: &[String]) -> AppHandle {
    let options = urls.iter().fold(
        LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
            .env("ND_CEF_STYLE", "chrome")
            .env("ND_APP_DATA_DIR", profile.display().to_string()),
        |o, url| o.arg(url),
    );
    let app = launch_app(options).unwrap();
    activate(app.pid());
    app
}

/// SIGTERM, the way a service manager or `kill` quits the app, and the wait for it to go.
fn quit(app: &AppHandle) {
    let pid = app.pid().to_string();
    assert!(std::process::Command::new("kill").args(["-TERM", &pid]).status().unwrap().success());
    exited(app, 15);
}

/// The profile's `session.json`, its data.
fn session_of(profile: &std::path::Path) -> serde_json::Value {
    let text = std::fs::read_to_string(profile.join("session.json")).unwrap();
    let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    doc["data"].take()
}

/// Moves the front window's top-left corner to `(x, y)` in global points.
fn move_to(app: &AppHandle, x: i32, y: i32) {
    let pid = app.pid();
    let script = format!(
        "tell application \"System Events\" to set position of front window of (first process whose unix id is {pid}) to {{{x}, {y}}}"
    );
    let status = std::process::Command::new("osascript").args(["-e", &script]).status().unwrap();
    assert!(status.success(), "the window would not move");
    activate(pid);
    std::thread::sleep(Duration::from_millis(800));
}

/// Brings the app forward: a launched test app is not, and key events would go elsewhere. The
/// accessibility call fails until the app has registered with the window server, so it retries.
fn activate(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ndshell::input::activate(pid) {
        assert!(Instant::now() < deadline, "the app would not come forward; in front: {}", front_app());
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::sleep(Duration::from_millis(300));
}

/// `pid`'s frontmost browser window, skipping popovers and other small windows.
fn front_window(app: &AppHandle) -> ndshell::window_list::WindowInfo {
    on_screen_window(app).expect("the app has a window on screen")
}

fn on_screen_window(app: &AppHandle) -> Option<ndshell::window_list::WindowInfo> {
    ndshell::window_list::of_pid(app.pid()).into_iter().find(|w| w.bounds.2 >= 400. && w.bounds.3 >= 300.)
}

/// The front window's width, for a wait: none while the window is off the screen's list.
fn width_of(app: &AppHandle) -> Option<f64> {
    on_screen_window(app).map(|w| w.bounds.2)
}

/// Whether the key window is in fullscreen with macOS done moving it there; until then its
/// frame may have grown already, but it takes no input.
fn settled_fullscreen(app: &AppHandle) -> bool {
    app.windows()
        .ok()
        .and_then(|w| w.as_array().cloned())
        .is_some_and(|ws| ws.iter().any(|w| w["key"] == true && w["fullscreen"] == true))
}

/// Resizes the front window to `width` points, keeping its height, as a drag of its edge would.
fn resize(app: &AppHandle, width: u32) {
    resize_to(app, width, front_window(app).bounds.3 as u32);
}

fn resize_to(app: &AppHandle, width: u32, height: u32) {
    let pid = app.pid();
    let script = format!(
        "tell application \"System Events\" to set size of front window of (first process whose unix id is {pid}) to {{{width}, {height}}}"
    );
    let status = std::process::Command::new("osascript").args(["-e", &script]).status().unwrap();
    assert!(status.success(), "the window would not resize");
    // System Events leaves the window inactive, and a click on an inactive page only activates it.
    activate(pid);
    std::thread::sleep(Duration::from_millis(800));
}

/// A real click at `(x, y)` in the window's points. A Chrome-style page lives in Chromium's own
/// child window, which events posted to the app's window never reach; `launch_app` holds the
/// machine's real-input lock for the run.
fn real_click(app: &AppHandle, x: f32, y: f32) {
    in_front(app);
    let (wx, wy, _, _) = front_window(app).bounds;
    ndshell::input::click((wx + f64::from(x), wy + f64::from(y)));
    std::thread::sleep(Duration::from_millis(400));
}

/// Waits until one of the app's windows is key, bringing the app forward again meanwhile. A click
/// on a window of an app that is not active only activates it, and an app the test quit a moment
/// earlier hands activation to the next app in line after the new one came forward.
fn in_front(app: &AppHandle) {
    let key = |a: &AppHandle| {
        a.windows().ok().is_some_and(|w| w.as_array().is_some_and(|ws| ws.iter().any(|w| w["key"] == true)))
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while !key(app) {
        assert!(
            Instant::now() < deadline,
            "no window of the app became key; in front: {}, windows {:?}",
            front_app(),
            app.windows()
        );
        ndshell::input::activate(app.pid());
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The app macOS has in front, by name and pid.
fn front_app() -> String {
    std::process::Command::new("sh")
        .args(["-c", "lsappinfo info -only name -only pid $(lsappinfo front)"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().replace('\n', " "))
        .unwrap_or_default()
}

/// A composited capture of the window, glass and pages included, which the automation
/// socket's layer render leaves out.
fn shot(app: &AppHandle, name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("shell-{name}.png"));
    ndshell::window_list::capture(front_window(app).number, &path).unwrap();
    println!("screenshot {}", path.display());
    path
}

/// Waits until `check` holds, failing with the tab titles and the app's stderr after `secs`.
fn until(app: &AppHandle, what: &str, secs: u64, check: impl Fn(&AppHandle) -> bool) {
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
    app.keyboard().press(keys).unwrap();
    std::thread::sleep(Duration::from_millis(300));
}

/// Clicks inside the page, `y` points below the content area's top.
fn click_page(app: &AppHandle, y: f32) {
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
    expect(&app.get_by_test_id("new-tab-row")).to_be_visible().unwrap();
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
                row("new-tab-row") && w.root.iter().any(|n| n.name == "New Tab")
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
    let close = CLOSE_LIGHT;
    real_click(&app, close.0, close.1);
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        ndshell::window_list::of_pid(app.pid()).len() > 2,
        "the window waits on its page's answer, under its prompt"
    );
    press(&app, "escape");
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(titles(&app), ["Beta", "Leave"], "Cancel keeps the window");
    // The last window going quits the app.
    real_click(&app, close.0, close.1);
    std::thread::sleep(Duration::from_secs(1));
    press(&app, "enter");
    exited(&app, 15);
}

/// Waits for the app to exit.
fn exited(app: &AppHandle, secs: u64) {
    let pid = app.pid().to_string();
    // The test holds the child unreaped, so an exited app lingers as a zombie.
    let gone = || {
        let out = std::process::Command::new("ps").args(["-o", "stat=", "-p", &pid]).output().unwrap();
        let stat = String::from_utf8_lossy(&out.stdout);
        stat.trim().is_empty() || stat.starts_with('Z')
    };
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
    let mut second = std::process::Command::new(env!("CARGO_BIN_EXE_lynk"))
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

/// macOS virtual key codes for the keys the cases post through the HID system.
mod key {
    pub const K: u16 = 40;
    pub const L: u16 = 37;
    pub const T: u16 = 17;
    pub const V: u16 = 9;
    pub const RETURN: u16 = 36;
    pub const TAB: u16 = 48;
    pub const BACKSPACE: u16 = 51;
    pub const DOWN: u16 = 125;
    pub const F: u16 = 3;
    pub const I: u16 = 34;
    pub const ESCAPE: u16 = 53;
    pub const F1: u16 = 122;
    pub const F7: u16 = 98;
    pub const F12: u16 = 111;
}

/// A key through the HID system, which reaches a Chrome-style page's own window as typing does;
/// the automation socket's keys reach only the app's.
fn hid(code: u16, modifiers: &[Modifier]) {
    ndshell::input::key(code, modifiers);
    std::thread::sleep(Duration::from_millis(600));
}

/// The app's windows on screen, popovers and bubbles left out. A Chrome-style page adds a
/// window of its own over the app's.
fn on_screen(app: &AppHandle) -> usize {
    ndshell::window_list::of_pid(app.pid()).iter().filter(|w| w.bounds.2 >= 300. && w.bounds.3 >= 200.).count()
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
    hid(key::ESCAPE, &[]);
    until(&app, "out of page fullscreen", 10, |a| width_of(a) == Some(width) && has(a, "controls-row"));
    // The browser's: macOS's own Enter Full Screen, the sidebar kept.
    hid(key::F, &[Modifier::Control, Modifier::Command]);
    until(&app, "browser fullscreen", 10, settled_fullscreen);
    assert!(has(&app, "controls-row"));
    // The page's fullscreen inside it, left with Esc, leaves the window as the user had it.
    click_page(&app, 100.);
    until(&app, "page fullscreen in browser fullscreen", 10, |a| lacks(a, "controls-row"));
    hid(key::ESCAPE, &[]);
    until(&app, "out of page fullscreen", 10, |a| has(a, "controls-row"));
    // A wrong exit would start within this.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(settled_fullscreen(&app), "the window stays in the user's fullscreen");
    hid(key::F, &[Modifier::Control, Modifier::Command]);
    until(&app, "out of browser fullscreen", 10, |a| width_of(a) == Some(width));
}

/// The window takes no input while macOS still moves it into fullscreen, though its frame has
/// already grown; the app says it is fullscreen only once the move is over, and a click then
/// reaches the page.
#[test]
fn real_fullscreen_is_reported_once_the_move_is_over() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/full"));
    until(&app, "the page", 30, |a| titles(a) == ["Full"]);
    let width = front_window(&app).bounds.2;
    in_front(&app);
    ndshell::input::key(key::F, &[Modifier::Control, Modifier::Command]);
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
    hid(key::ESCAPE, &[]);
    until(&app, "out of page fullscreen", 10, |a| has(a, "controls-row"));
    assert!(settled_fullscreen(&app), "the window stays in the user's fullscreen");
    hid(key::F, &[Modifier::Control, Modifier::Command]);
    until(&app, "out of browser fullscreen", 10, |a| width_of(a) == Some(width));
}

#[test]
fn real_dev_tools_from_the_page() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for (name, code, modifiers) in
        [("F12", key::F12, &[][..]), ("Alt+Cmd+I", key::I, &[Modifier::Option, Modifier::Command][..])]
    {
        let app = launch(&format!("{base}/alpha"));
        until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
        // Focus in the page, where Chrome sees the chord first.
        click_page(&app, 500.);
        let before = on_screen(&app);
        hid(code, modifiers);
        until(&app, &format!("devtools from {name}"), 10, |a| on_screen(a) == before + 1);
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
    let windows = |app: &AppHandle| ndshell::window_list::every_layer_of_pid(app.pid()).len();
    let before = windows(&app);
    // The count sees a window off the normal layer: a row's menu.
    let (wx, wy, _, _) = front_window(&app).bounds;
    let at = row_at(&app, 0);
    ndshell::input::right_click((wx + f64::from(at.0), wy + f64::from(at.1)));
    until(&app, "the row menu counted", 5, |a| windows(a) > before);
    hid(key::ESCAPE, &[]);
    until(&app, "the row menu gone", 5, |a| windows(a) == before);
    click_page(&app, 500.);
    // Caret browsing, the task manager and help: Chrome UI with no place in Lynk.
    for (name, code, modifiers) in
        [("F7", key::F7, &[][..]), ("Shift+Esc", key::ESCAPE, &[Modifier::Shift][..]), ("F1", key::F1, &[][..])]
    {
        hid(code, modifiers);
        std::thread::sleep(Duration::from_millis(900));
        assert_eq!(windows(&app), before, "{name} opened a window, dialog or panel");
        assert_eq!(titles(&app), ["Alpha"], "{name} opened a tab");
    }
    // F11 belongs to macOS (Show Desktop), so it goes through the app's own key handling only.
    press(&app, "f11");
    std::thread::sleep(Duration::from_millis(900));
    assert_eq!(windows(&app), before, "F11 opened a window, dialog or panel");
    assert_eq!(titles(&app), ["Alpha"], "F11 opened a tab");
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
    let mut app = std::process::Command::new(env!("CARGO_BIN_EXE_lynk"))
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

/// Picks `item` in the menu bar's `menu` as a user does, through System Events.
fn menu(app: &AppHandle, menu: &str, item: &str) {
    let script = format!(
        "tell application \"System Events\" to tell (first process whose unix id is {}) to click menu item \"{item}\" of menu \"{menu}\" of menu bar 1",
        app.pid()
    );
    let status = std::process::Command::new("osascript").args(["-e", &script]).status().unwrap();
    assert!(status.success(), "no {menu} > {item}");
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
        let app = launch_app(options).unwrap();
        activate(app.pid());
        let deadline = Instant::now() + Duration::from_secs(30);
        while titles(&app) != ["Press"] {
            assert!(Instant::now() < deadline, "the page never loaded");
            std::thread::sleep(Duration::from_millis(10));
        }
        let b = app.get_by_test_id("content").bounding_box().unwrap().expect("the page is laid out");
        real_click(&app, b.x + b.width / 2., b.y + 100.);
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
    menu(&app, "Tabs", "Put Tab to Sleep");
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
    let app = launch_app(options).unwrap();
    activate(app.pid());
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
    menu(&app, "Tabs", "Put Tab to Sleep");
    until(&app, "the tab asleep", 15, |a| live_pages(a) == Some(1));
    assert_eq!(titles(&app), ["Alpha", "Beta"]);
    shot(&app, "tab-asleep");
    press(&app, "cmd-1");
    until(&app, "the tab awake", 15, |a| live_pages(a) == Some(2));
}

/// The macOS key code of a lowercase letter or a space, for typing into an open menu.
fn key_of(c: char) -> u16 {
    const LETTERS: &str = "asdfhgzxcv_bqweryt123465=97-80]ou[ip_lj'k;\\,/nm.";
    if c == ' ' {
        return 49;
    }
    LETTERS.find(c).map(|i| i as u16).expect("a key for the letter")
}

/// Right-clicks `at` (window points) and picks the item whose title starts with `item`, typed
/// into the menu as a user selects by typing.
fn pick_menu(app: &AppHandle, at: (f32, f32), item: &str) {
    in_front(app);
    let (wx, wy, _, _) = front_window(app).bounds;
    ndshell::input::right_click((wx + f64::from(at.0), wy + f64::from(at.1)));
    std::thread::sleep(Duration::from_millis(600));
    for c in item.to_ascii_lowercase().chars() {
        ndshell::input::key(key_of(c), &[]);
        std::thread::sleep(Duration::from_millis(60));
    }
    hid(36, &[]);
}

/// The title of the app's first window, which is the shown tab's.
fn shown_title(app: &AppHandle) -> String {
    let windows = app.windows().unwrap();
    windows[0]["title"].as_str().unwrap_or_default().to_owned()
}

/// The first window's tab rows top to bottom: test id and centre, in window points.
fn rows(app: &AppHandle) -> Vec<(String, (f32, f32))> {
    let tree = app.tree(None).unwrap();
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
    app.get_by_test_id(&close(&app).unwrap()).press("space").unwrap();
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
    ndshell::input::right_click((wx + f64::from(at.0), wy + f64::from(at.1)));
    std::thread::sleep(Duration::from_millis(600));
    hid(key::ESCAPE, &[]);
    assert_eq!(shown_title(&app), "Beta", "the shown tab after a menu on another row");
    // The row's menu acts on its own tab, not the one on show.
    let duplicate = rows(&app)[1].0.clone();
    pick_menu(&app, row_at(&app, 1), "Close Other");
    until(&app, "only the second tab", 15, |a| titles(a) == ["Alpha"]);
    assert_eq!(rows(&app).into_iter().map(|r| r.0).collect::<Vec<_>>(), [duplicate], "the duplicate is the one left");
    // Off any tab: the column's menu.
    let n = app.get_by_test_id("new-tab-row").bounding_box().unwrap().unwrap();
    pick_menu(&app, (n.x + n.width / 2., n.y + n.height + 120.), "New Tab");
    until(&app, "the bar from the column menu", 10, |a| bar_field(a).is_some());
    type_real(&format!("{base}/beta"));
    hid(key::RETURN, &[]);
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
        ndshell::input::type_text(&format!("{base}/beta?{n}\n"));
    }
    until(&app, "the fifty tabs", 60, |a| live_pages(a) == Some(51));
    let keys = app.keyboard();
    // Fifty closes leave exactly the first tab only if all fifty opened: one too few would take
    // the window, and the app with it.
    for _ in 0..50 {
        keys.press("cmd-w").unwrap();
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
        app.keyboard().press("cmd-shift-]").unwrap();
    }
    until(&app, "thirty live pages", 60, |a| live_pages(a) == Some(30));
    // The last window closing quits.
    real_click(&app, CLOSE_LIGHT.0, CLOSE_LIGHT.1);
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
        keys.press("cmd-[").unwrap();
        keys.press("cmd-]").unwrap();
    }
    keys.press("cmd-[").unwrap();
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
    resize_to(&app, 600, 400);
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
        |a: &AppHandle| a.tree(None).is_ok_and(|t| t.windows.iter().any(|w| w.root.iter().any(|n| n.name == stamped)));
    until(&app, "the moved page as it was", 10, moved);
    let left = |a: &AppHandle| a.tree(None).is_ok_and(|t| t.windows.iter().any(|w| window_titles(w) == ["Alpha"]));
    until(&app, "the other tab left behind", 5, left);
}

/// The colour the page shows near its bottom right corner, where nothing of its own is drawn,
/// read from a capture of the window.
fn page_backdrop(app: &AppHandle, name: &str) -> [u8; 3] {
    let path = shot(app, name);
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let scale = f64::from(info.width) / front_window(app).bounds.2;
    let b = app.get_by_test_id("content").bounding_box().unwrap().expect("the page is laid out");
    let x = ((f64::from(b.x + b.width) - 40.) * scale) as usize;
    let y = ((f64::from(b.y + b.height) - 40.) * scale) as usize;
    let channels = info.color_type.samples();
    let at = y * info.line_size + x * channels;
    [buf[at], buf[at + 1], buf[at + 2]]
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
        let app = launch_app(options).unwrap();
        activate(app.pid());
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
    let app = launch_app(options).unwrap();
    activate(app.pid());
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
        let app = launch_app(options).unwrap();
        activate(app.pid());
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
            in_front(&app);
            let (wx, wy, _, _) = front_window(&app).bounds;
            let at = row_at(&app, 1);
            ndshell::input::move_to((wx + f64::from(at.0), wy + f64::from(at.1)));
            std::thread::sleep(Duration::from_millis(600));
            shot(&app, &format!("sidebar-states-{appearance}{suffix}"));
            sidebar_geometry_holds(&app);
        }
        // A row held down gives a little.
        let (wx, wy, _, _) = front_window(&app).bounds;
        let at = row_at(&app, 1);
        let at = (wx + f64::from(at.0), wy + f64::from(at.1));
        ndshell::input::hold(at);
        std::thread::sleep(Duration::from_millis(400));
        shot(&app, &format!("sidebar-pressed-{appearance}"));
        ndshell::input::release(at);
    }
}

/// The column's numbers, read off the shown row: 28 tall, 19 in from the window's left edge and
/// the sidebar's trailing edge, the close button inside its trailing slot with air on every side,
/// and the title ending before it.
fn sidebar_geometry_holds(app: &AppHandle) {
    let bounds =
        |id: &str| app.get_by_test_id(id).bounding_box().unwrap().unwrap_or_else(|| panic!("{id} is laid out"));
    let row_id = rows(app)[0].0.clone();
    let id = row_id.strip_prefix("tab-row-").unwrap();
    let (row, close, title) = (bounds(&row_id), bounds(&format!("tab-close-{id}")), bounds(&format!("tab-title-{id}")));
    assert_eq!((row.x, row.width, row.height), (19., 240. - 2. * 19., 28.), "the shown row");
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
    move_to(&app, 180, 140);
    resize_to(&app, 1100, 760);
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
    assert!(
        (after.0 - before.0).abs() < 2. && (after.1 - before.1).abs() < 2. && (after.2 - before.2).abs() < 2.,
        "the window came back at {after:?}, not {before:?}"
    );
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
    app.keyboard().press("cmd-1").unwrap();
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
    let options = LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
        .env("ND_CEF_STYLE", "chrome")
        .env("ND_APP_DATA_DIR", profile.display().to_string())
        .arg(format!("{base}/press"));
    match launch_app(options) {
        Ok(app) => {
            // This launch holds the profile; the other hands its address over and quits.
            let deadline = Instant::now() + Duration::from_secs(20);
            while other.try_wait().unwrap().is_none() {
                assert!(Instant::now() < deadline, "the other launch kept running");
                std::thread::sleep(Duration::from_millis(100));
            }
            activate(app.pid());
            until(&app, "both launches' tabs", 30, |a| rows(a).len() == 4);
            quit(&app);
        }
        Err(_) => {
            // This launch lost the race, handed its address to the other and quit; the other
            // runs without automation, so its session file says when both tabs are in.
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
struct Reaped(std::process::Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The main display's frame in global points, `(x, y, width, height)`.
fn main_display() -> (f64, f64, f64, f64) {
    let script = "ObjC.import('AppKit'); const f = $.NSScreen.screens.objectAtIndex(0).frame; \
                  [f.origin.x, f.origin.y, f.size.width, f.size.height].join(',')";
    let out = std::process::Command::new("osascript").args(["-l", "JavaScript", "-e", script]).output().unwrap();
    let n: Vec<f64> = String::from_utf8_lossy(&out.stdout).split(',').map(|v| v.trim().parse().unwrap()).collect();
    (n[0], n[1], n[2], n[3])
}

/// A window kept on a display that is gone comes back on one that is there.
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

/// Opens the bar with Cmd and `code` (T, L, K) through the HID system and waits for it.
#[track_caller]
fn open_bar(app: &AppHandle, code: u16) {
    in_front(app);
    hid(code, &[Modifier::Command]);
    bar_wait(app, true);
}

#[track_caller]
fn bar_closed(app: &AppHandle) {
    bar_wait(app, false);
}

/// Waits for the bar to be open or closed, with a capture and the app's windows when it is not.
#[track_caller]
fn bar_wait(app: &AppHandle, open: bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while bar_field(app).is_some() != open {
        if Instant::now() > deadline {
            shot(app, "bar-timeout");
            let windows = ndshell::window_list::of_pid(app.pid());
            let front = std::process::Command::new("sh")
                .args(["-c", "lsappinfo info -only name $(lsappinfo front)"])
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
            panic!(
                "the bar never {}: windows {windows:?}, keys {:?}, front app {front:?}, app said {:#?}",
                if open { "opened" } else { "closed" },
                app.windows(),
                app.stderr_tail(25)
            );
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Types at the HID tap, character by character, as a keyboard would.
fn type_real(text: &str) {
    ndshell::input::type_text(text);
    std::thread::sleep(Duration::from_millis(400));
}

/// Pastes `text` with Cmd+V, from the real pasteboard.
fn paste(text: &str) {
    let mut copy = std::process::Command::new("pbcopy").stdin(std::process::Stdio::piped()).spawn().unwrap();
    copy.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    assert!(copy.wait().unwrap().success());
    hid(key::V, &[Modifier::Command]);
}

/// Opens `url` in a new tab through the bar, as typed.
#[track_caller]
fn open_in_new_tab(app: &AppHandle, url: &str) {
    let before = titles(app).len();
    open_bar(app, key::T);
    type_real(url);
    hid(key::RETURN, &[]);
    bar_closed(app);
    until(app, "the new tab", 15, |a| titles(a).len() == before + 1);
}

/// The shown tab's address, as the bar Cmd+L opens on it holds it.
#[track_caller]
fn address(app: &AppHandle) -> String {
    open_bar(app, key::L);
    let field = bar_field(app).unwrap_or_default();
    hid(key::ESCAPE, &[]);
    bar_closed(app);
    field
}

/// Types `text` over a Cmd+L selection and goes there with Return.
#[track_caller]
fn go_to(app: &AppHandle, text: &str) {
    open_bar(app, key::L);
    type_real(text);
    hid(key::RETURN, &[]);
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
        ndshell::input::type_text(&typed);
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
        hid(key::ESCAPE, &[]);
        bar_closed(&app);
    }
    assert_eq!(titles(&app).len(), 1, "no search for keys the page took");
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
    type_real("half typ");
    shot(&app, "bar-typing");
    assert_eq!(bar_field(&app).as_deref(), Some("half typ"));
    hid(key::ESCAPE, &[]);
    bar_closed(&app);
    open_bar(&app, key::T);
    hid(key::RETURN, &[]);
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
    go_to(&app, "b\u{fc}cher.de");
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
    paste(&format!("{base}/beta?pasted\n"));
    hid(key::RETURN, &[]);
    bar_closed(&app);
    until(&app, "the pasted address", 15, |a| address(a) == format!("{base}/beta?pasted"));
    let long = format!("{base}/beta?{}", "q".repeat(2000 - base.len() - 6));
    assert_eq!(long.len(), 2000);
    open_bar(&app, key::L);
    paste(&long);
    shot(&app, "bar-long-url");
    hid(key::RETURN, &[]);
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
    hid(key::ESCAPE, &[]);
    bar_closed(&app);

    // A second Cmd+L puts the bar away.
    open_bar(&app, key::L);
    hid(key::L, &[Modifier::Command]);
    bar_closed(&app);
    drop(app);

    // On the new tab page, Cmd+T loads in place: a blank tab has nothing to keep.
    let app = launch("");
    until(&app, "the new tab page", 30, |a| has(a, "new-tab-page"));
    open_bar(&app, key::T);
    type_real(&format!("{base}/beta"));
    hid(key::RETURN, &[]);
    until(&app, "Beta in the blank tab", 15, |a| titles(a) == ["Beta"]);
}

/// Another app taking focus leaves the bar open, and coming back gives its field the keys again,
/// as it had them.
#[test]
fn real_command_bar_survives_an_app_switch() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    open_bar(&app, key::T);
    type_real("lynk-");
    let finder = std::process::Command::new("pgrep").args(["-x", "Finder"]).output().unwrap();
    let finder: u32 = String::from_utf8_lossy(&finder.stdout).trim().parse().expect("Finder runs");
    ndshell::input::activate(finder);
    let away = |a: &AppHandle| {
        a.windows().ok().is_some_and(|w| w.as_array().is_some_and(|ws| ws.iter().all(|w| w["key"] != true)))
    };
    until(&app, "another app in front", 10, away);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(bar_field(&app).as_deref(), Some("lynk-"), "the bar stays open while away");
    // The bar's panel takes key back, not the window under it.
    ndshell::input::activate(app.pid());
    std::thread::sleep(Duration::from_millis(600));
    type_real("back");
    assert_eq!(bar_field(&app).as_deref(), Some("lynk-back"), "the field has the keys again");
    hid(key::ESCAPE, &[]);
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
    type_real("lynk-");
    let behind_loaded =
        |a: &AppHandle| a.tree(None).is_ok_and(|t| t.windows.iter().any(|w| window_titles(w) == ["Beta"]));
    until(&app, "the window behind on its new site", 20, behind_loaded);
    std::thread::sleep(Duration::from_secs(1));
    type_real("back");
    assert_eq!(bar_field(&app).as_deref(), Some("lynk-back"), "the bar in front kept the keys");
    hid(key::ESCAPE, &[]);
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
    type_real("127.0.0");
    until(&app, "the completed host", 5, |a| bar_field(a) == Some(format!("127.0.0|.1:{port}")));
    // Backspace takes the completion away, and it stays away until the next letter.
    hid(key::BACKSPACE, &[]);
    until(&app, "the completion gone", 5, |a| bar_field(a).as_deref() == Some("127.0.0"));
    type_real(".");
    until(&app, "completing again", 5, |a| bar_field(a) == Some(format!("127.0.0.|1:{port}")));
    // Tab takes it; then a path finishes from history.
    hid(key::TAB, &[]);
    until(&app, "the host taken", 5, |a| bar_field(a).as_deref() == Some(host.as_str()));
    type_real("/pl");
    until(&app, "the path completed", 5, |a| bar_field(a) == Some(format!("{host}/pl|ain")));
    let rows = bar_rows(&app);
    assert!(rows.first().is_some_and(|r| r.starts_with(&format!("{host}/plain"))), "{rows:?}");
    hid(key::RETURN, &[]);
    bar_closed(&app);
    until(&app, "the completed address in a new tab", 15, |a| titles(a) == ["Plain", "Beta", "Plain"]);

    // An open tab is offered as a switch: Down, then Return, shows it instead of opening it again.
    open_bar(&app, key::L);
    type_real("Bet");
    until(&app, "the open tab offered", 5, |a| {
        bar_rows(a).iter().any(|r| r.starts_with("Beta") && r.ends_with("Switch to Tab"))
    });
    let rows = bar_rows(&app);
    let at = rows.iter().position(|r| r.ends_with("Switch to Tab")).unwrap();
    for _ in 0..at {
        hid(key::DOWN, &[]);
    }
    hid(key::RETURN, &[]);
    bar_closed(&app);
    until(&app, "Beta shown", 10, |a| shown_title(a) == "Beta");
    assert_eq!(titles(&app), ["Plain", "Beta", "Plain"], "switched, not opened again");

    // Cmd+K with nothing typed: the first row is the tab shown before, and Return goes back.
    open_bar(&app, key::K);
    let rows = bar_rows(&app);
    assert!(rows.first().is_some_and(|r| r.starts_with("Plain") && r.ends_with("Switch to Tab")), "{rows:?}");
    hid(key::RETURN, &[]);
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
    ndshell::input::click((wx + 600., wy + (wh * 0.18).round() + 26. + 37. + 40. * at as f64 + 20.));
    bar_closed(&app);
    until(&app, "Beta by the pointer", 10, |a| shown_title(a) == "Beta");

    // Typing fast while the rows come back from history keeps every letter, in order.
    open_bar(&app, key::T);
    type_real(&format!("{host}/beta?fast-typing"));
    assert_eq!(bar_field(&app), Some(format!("{host}/beta?fast-typing")));
    hid(key::ESCAPE, &[]);
    bar_closed(&app);
}

/// A capture for the report, under the worktree's `target/report-shots`, at the display's scale.
fn report_shot(app: &AppHandle, name: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/report-shots");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{name}.png"));
    ndshell::window_list::capture(front_window(app).number, &path).unwrap();
    println!("screenshot {}", path.display());
}

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
        let app = launch_app(options).unwrap();
        activate(app.pid());
        until(&app, "the first page", 30, |a| titles(a).first().is_some_and(|t| t == "Alpha"));
        for width in [1200, 720] {
            resize(&app, width);
            let name = |what: &str| format!("bar-{appearance}-{width}-{what}");
            open_bar(&app, key::T);
            report_shot(&app, &name("empty"));
            type_real("bet");
            report_shot(&app, &name("few"));
            hid(key::ESCAPE, &[]);
            bar_closed(&app);
            open_bar(&app, key::K);
            report_shot(&app, &name("many"));
            hid(key::ESCAPE, &[]);
            bar_closed(&app);
            press(&app, "cmd-3");
            open_bar(&app, key::L);
            report_shot(&app, &name("long-url"));
            hid(key::ESCAPE, &[]);
            bar_closed(&app);
            press(&app, "cmd-1");
            open_bar(&app, key::T);
            type_real("\u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}");
            report_shot(&app, &name("rtl"));
            hid(key::ESCAPE, &[]);
            bar_closed(&app);
            open_bar(&app, key::T);
            type_real("\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{691c}\u{7d22} \u{30c6}\u{30b9}\u{30c8}");
            report_shot(&app, &name("cjk"));
            hid(key::ESCAPE, &[]);
            bar_closed(&app);
        }
    }
}
