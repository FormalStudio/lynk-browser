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
const LEAVE: &str = r#"<!doctype html><title>Leave</title>
<body style="margin:0"><div style="height:400px;background:#fdd">Click me first</div>
<script>addEventListener("beforeunload", e => { e.preventDefault(); e.returnValue = ""; });</script></body>"#;

/// Scrolls on a click and puts where it is in its title, with a stamp of the load.
const LONG: &str = r#"<!doctype html><title>Long</title>
<body style="margin:0;height:5000px" onclick="window.scrollTo(0, 600)">
<script>onscroll = () => document.title = "At " + Math.round(scrollY) + " " + Math.round(performance.timeOrigin);</script></body>"#;
/// Hangs its renderer on a click.
const BUSY: &str = r#"<!doctype html><title>Busy</title>
<body style="margin:0;height:400px" onclick="setTimeout(() => { for (;;) {} }, 0)"></body>"#;

/// Says it was pressed, for a click as soon as the page has a title.
const PRESS: &str = r#"<!doctype html><title>Press</title>
<body style="margin:0;height:400px;background:#dfd" onpointerdown="document.title = 'Pressed'"></body>"#;

const FULL: &str = r#"<!doctype html><title>Full</title>
<body style="margin:0"><button style="width:100%;height:300px" onclick="document.documentElement.requestFullscreen()">Go fullscreen</button></body>"#;

/// Serves the fixture pages on a free local port for the rest of the test process.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            let mut stream = stream;
            let mut line = String::new();
            let _ = BufReader::new(&stream).read_line(&mut line);
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (status, body) = match path.as_str() {
                "/alpha" => ("200 OK", ALPHA),
                "/beta" => ("200 OK", BETA),
                "/leave" => ("200 OK", LEAVE),
                "/full" => ("200 OK", FULL),
                "/long" => ("200 OK", LONG),
                "/busy" => ("200 OK", BUSY),
                "/press" => ("200 OK", PRESS),
                _ => ("404 Not Found", "<!doctype html><title>Missing</title>"),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    base
}

fn launch(url: &str) -> AppHandle {
    let options = LaunchOptions::new(env!("CARGO_BIN_EXE_lynk")).env("LYNK_URL", url).env("ND_CEF_STYLE", "chrome");
    let app = launch_app(options).unwrap();
    activate(app.pid());
    app
}

/// Brings the app forward: a launched test app is not, and key events would go elsewhere. The
/// accessibility call fails until the app has registered with the window server, so it retries.
fn activate(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ndshell::input::activate(pid) {
        assert!(Instant::now() < deadline, "the app would not come forward");
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

/// Resizes the front window to `width` points, keeping its height, as a drag of its edge would.
fn resize(app: &AppHandle, width: u32) {
    let height = front_window(app).bounds.3 as u32;
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
    let (wx, wy, _, _) = front_window(app).bounds;
    ndshell::input::click((wx + f64::from(x), wy + f64::from(y)));
    std::thread::sleep(Duration::from_millis(400));
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

fn has(app: &AppHandle, test_id: &str) -> bool {
    app.tree(None).is_ok_and(|t| {
        t.windows.iter().any(|w| w.root.iter().any(|n| n.test_id.as_deref() == Some(test_id) && n.visible))
    })
}

/// The tab rows' titles of the first window, in sidebar order.
fn titles(app: &AppHandle) -> Vec<String> {
    let Ok(tree) = app.tree(None) else { return Vec::new() };
    let Some(window) = tree.windows.first() else { return Vec::new() };
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

    // Cmd+T opens the new tab page; Cmd+1 and Cmd+9 select by place.
    press(&app, "cmd-t");
    until(&app, "the new tab page", 10, |a| has(a, "new-tab-page"));
    assert_eq!(titles(&app), ["Alpha", "New Tab"]);
    press(&app, "cmd-1");
    until(&app, "Alpha shown", 10, |a| !has(a, "new-tab-page"));
    press(&app, "cmd-9");
    until(&app, "the last tab shown", 10, |a| has(a, "new-tab-page"));
    press(&app, "cmd-1");
    until(&app, "Alpha shown again", 10, |a| !has(a, "new-tab-page"));

    // A target=_blank link opens a tab right after its opener, in front, not at the end.
    click_page(&app, 300.);
    until(&app, "the page-opened tab", 15, |a| titles(a) == ["Alpha", "Beta", "New Tab"]);
    press(&app, "cmd-9");
    until(&app, "the last tab shown", 10, |a| has(a, "new-tab-page"));
    press(&app, "cmd-shift-[");
    until(&app, "previous tab", 10, |a| !has(a, "new-tab-page"));

    // Cmd+W closes the shown tab; Shift+Cmd+T brings it back from the stack.
    press(&app, "cmd-w");
    until(&app, "Beta closed", 10, |a| titles(a) == ["Alpha", "New Tab"]);
    press(&app, "cmd-shift-t");
    until(&app, "Beta reopened", 15, |a| titles(a) == ["Alpha", "New Tab", "Beta"]);

    // Shift+Cmd+B hides the sidebar and shows it again.
    press(&app, "cmd-shift-b");
    until(&app, "the sidebar hidden", 10, |a| !has(a, "controls-row"));
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
    until(&app, "the bar closed", 10, |a| !has(a, "find-bar"));
}

#[test]
fn real_zoom_steps_and_glyph() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    assert!(!has(&app, "zoom"), "no glyph at 100%");
    press(&app, "cmd-=");
    until(&app, "the zoom popover", 10, |a| text_of(a, "zoom-value").as_deref() == Some("110%"));
    shot(&app, "zoom-popover");
    press(&app, "cmd--");
    press(&app, "cmd--");
    until(&app, "90%", 10, |a| text_of(a, "zoom-value").as_deref() == Some("90%"));
    // The notice goes after 1.5 s; the glyph stays while the page is not at 100%.
    until(&app, "the popover gone", 10, |a| !has(a, "zoom-value"));
    assert!(has(&app, "zoom"));
    press(&app, "cmd-0");
    until(&app, "the glyph gone at 100%", 10, |a| !has(a, "zoom"));
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
    press(&app, "cmd-t");
    until(&app, "a second tab", 10, |a| titles(a).len() == 2);
    press(&app, "cmd-1");
    press(&app, "cmd-w");
    std::thread::sleep(Duration::from_secs(1));
    shot(&app, "leave-site");
    assert_eq!(titles(&app), ["Leave", "New Tab"], "the tab waits on its page's answer");
    // Esc takes Cancel, which keeps the tab.
    press(&app, "escape");
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(titles(&app), ["Leave", "New Tab"], "Cancel keeps the tab");
    // Enter takes Chrome's dialog's default, Leave.
    press(&app, "cmd-w");
    std::thread::sleep(Duration::from_secs(1));
    press(&app, "enter");
    until(&app, "the tab gone once the page agreed", 10, |a| titles(a) == ["New Tab"]);

    // The window's close button asks the page too, and Cancel keeps the window.
    press(&app, "cmd-shift-t");
    until(&app, "the page back", 15, |a| titles(a) == ["New Tab", "Leave"]);
    click_page(&app, 100.);
    let close = (19., 19.);
    real_click(&app, close.0, close.1);
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        ndshell::window_list::of_pid(app.pid()).len() > 2,
        "the window waits on its page's answer, under its prompt"
    );
    press(&app, "escape");
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(titles(&app), ["New Tab", "Leave"], "Cancel keeps the window");
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
        assert!(Instant::now() < deadline, "the app is still running: {:?}", app.stderr_tail(30));
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
    until(&app, "page fullscreen", 10, |a| width_of(a).is_some_and(|w| w > width) && !has(a, "controls-row"));
    // The window server cannot capture a window while it moves into its fullscreen space.
    std::thread::sleep(Duration::from_millis(1500));
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
    until(&app, "browser fullscreen", 10, |a| width_of(a).is_some_and(|w| w > width));
    assert!(has(&app, "controls-row"));
    // The page's fullscreen inside it, left with Esc, leaves the window as the user had it.
    click_page(&app, 100.);
    until(&app, "page fullscreen in browser fullscreen", 10, |a| !has(a, "controls-row"));
    hid(key::ESCAPE, &[]);
    until(&app, "out of page fullscreen", 10, |a| has(a, "controls-row"));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(front_window(&app).bounds.2 > width, "the window stays in the user's fullscreen");
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
    let before = on_screen(&app);
    // Caret browsing, the task manager and help: Chrome UI with no place in Lynk.
    for (name, code, modifiers) in
        [("F7", key::F7, &[][..]), ("Shift+Esc", key::ESCAPE, &[Modifier::Shift][..]), ("F1", key::F1, &[][..])]
    {
        hid(code, modifiers);
        std::thread::sleep(Duration::from_millis(900));
        assert_eq!(on_screen(&app), before, "{name} opened a window");
        assert_eq!(titles(&app), ["Alpha"], "{name} opened a tab");
    }
    // F11 belongs to macOS (Show Desktop), so it goes through the app's own key handling only.
    press(&app, "f11");
    std::thread::sleep(Duration::from_millis(900));
    assert_eq!(on_screen(&app), before, "F11 opened a window");
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

/// The live pages of the first window, a sleeping tab having none.
fn live_pages(app: &AppHandle) -> usize {
    let Ok(tree) = app.tree(None) else { return 0 };
    tree.windows.first().map_or(0, |w| {
        w.root
            .iter()
            .filter(|n| {
                n.test_id.as_deref().and_then(|t| t.strip_prefix("page-")).is_some_and(|id| id.parse::<u64>().is_ok())
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
    press(&app, "cmd-t");
    until(&app, "a second tab", 10, |a| titles(a).len() == 2);
    press(&app, "cmd-1");
    menu(&app, "Tabs", "Put Tab to Sleep");
    until(&app, "the tab asleep", 15, |a| live_pages(a) == 0);
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
    press(&app, "cmd-t");
    until(&app, "a second tab", 10, |a| titles(a).len() == 2);
    // A renderer that never answers where it is scrolled does not keep its tab awake.
    std::thread::sleep(Duration::from_secs(1));
    let at = row_at(&app, 0);
    pick_menu(&app, at, "Put Tab");
    until(&app, "the busy tab asleep", 5, |a| live_pages(a) == 0);
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
    until(&app, "a second tab", 15, |a| titles(a) == ["Alpha", "Beta"] && live_pages(a) == 2);
    // A sleeping tab keeps its row and gives its page back; selecting it wakes it.
    press(&app, "cmd-1");
    menu(&app, "Tabs", "Put Tab to Sleep");
    until(&app, "the tab asleep", 15, |a| live_pages(a) == 1);
    assert_eq!(titles(&app), ["Alpha", "Beta"]);
    shot(&app, "tab-asleep");
    press(&app, "cmd-1");
    until(&app, "the tab awake", 15, |a| live_pages(a) == 2);
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

/// The centre of the sidebar's `index`th tab row, in window points.
fn row_at(app: &AppHandle, index: usize) -> (f32, f32) {
    let tree = app.tree(None).unwrap();
    let mut rows: Vec<_> = tree.windows[0]
        .root
        .iter()
        .filter(|n| n.test_id.as_deref().is_some_and(|t| t.starts_with("tab-row-")) && n.visible)
        .filter_map(|n| n.bounds)
        .collect();
    rows.sort_by(|a, b| a.origin.y.total_cmp(&b.origin.y));
    let r = rows[index];
    (r.origin.x + r.size.width / 2., r.origin.y + r.size.height / 2.)
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
    press(&app, "cmd-t");
    until(&app, "a third tab", 10, |a| titles(a).len() == 3);
    // A right click on another tab's row opens its menu and leaves the shown tab alone.
    let (wx, wy, _, _) = front_window(&app).bounds;
    let at = row_at(&app, 0);
    ndshell::input::right_click((wx + f64::from(at.0), wy + f64::from(at.1)));
    std::thread::sleep(Duration::from_millis(600));
    hid(key::ESCAPE, &[]);
    assert_eq!(shown_title(&app), "New Tab", "the shown tab after a menu on another row");
    // The row's menu acts on its own tab, not the one on show.
    pick_menu(&app, row_at(&app, 1), "Close Other");
    until(&app, "only the second tab", 15, |a| titles(a) == ["Alpha"]);
    // Off any tab: the column's menu.
    let n = app.get_by_test_id("new-tab-row").bounding_box().unwrap().unwrap();
    pick_menu(&app, (n.x + n.width / 2., n.y + n.height + 120.), "New Tab");
    until(&app, "a tab from the column menu", 10, |a| titles(a).len() == 2);
}
