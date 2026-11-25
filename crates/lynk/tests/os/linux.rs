//! The drive's Linux side: each app on a private headless Hyprland with XWayland (a Chrome-style
//! page is Chromium's own X11 window inside the app's), its virtual pointer and keyboard, window
//! geometry and fullscreen from Hyprland's IPC, and captures of its output.

use std::cell::RefCell;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use nd_test::{AppHandle, LaunchOptions, launch_app};
use ndshell::hyprland::Hyprland;
pub use ndshell::hyprland::Options;
pub use ndshell::wayland::Modifier;
use ndshell::wayland::{CapturedFrame, VirtualInput};

pub type Code = u32;

/// Linux evdev codes for the keys the cases press on the compositor's keyboard.
pub mod key {
    use super::Code;
    pub const I: Code = 23;
    pub const K: Code = 37;
    pub const L: Code = 38;
    pub const T: Code = 20;
    pub const V: Code = 47;
    pub const ESCAPE: Code = 1;
    pub const ENTER: Code = 28;
    pub const TAB: Code = 15;
    pub const BACKSPACE: Code = 14;
    pub const DOWN: Code = 108;
    pub const LEFT: Code = 105;
    pub const RIGHT: Code = 106;
    pub const HOME: Code = 102;
    pub const END: Code = 107;
    pub const A: Code = 30;
    pub const F1: Code = 59;
    pub const F7: Code = 65;
    pub const F11: Code = 87;
    pub const F12: Code = 88;
    pub const R: Code = 19;
    pub const W: Code = 17;
}

/// The keys that take the caret to the start and the end of a field's line.
pub const LINE_START: (Code, &[Modifier]) = (key::HOME, &[]);
pub const LINE_END: (Code, &[Modifier]) = (key::END, &[]);

/// The modifier of the app's shortcuts.
pub const CMD: Modifier = Modifier::Control;

pub const DEVTOOLS: (&str, Code, &[Modifier]) = ("Ctrl+Shift+I", key::I, &[Modifier::Control, Modifier::Shift]);
pub const NEW_TAB: &str = "sidebar-new-tab";
/// ndshell writes frames as BMP; `scripts/linux-gate shots` brings them back.
pub const SHOT_EXT: &str = "bmp";

/// The evdev code of a lowercase letter or a space, for typing into an open menu.
pub fn key_of(c: char) -> Code {
    key_of_char(c).0
}

/// The evdev code of a printable ASCII character on the compositor's US layout, and whether it
/// takes Shift.
fn key_of_char(c: char) -> (Code, bool) {
    const PLAIN: [(&str, Code); 4] =
        [("1234567890-=", 2), ("qwertyuiop[]", 16), ("asdfghjkl;'`", 30), ("zxcvbnm,./", 44)];
    const SHIFTED: [(&str, Code); 4] =
        [("!@#$%^&*()_+", 2), ("QWERTYUIOP{}", 16), ("ASDFGHJKL:\"~", 30), ("ZXCVBNM<>?", 44)];
    match c {
        ' ' => return (57, false),
        '\n' => return (key::ENTER, false),
        '\t' => return (key::TAB, false),
        '\\' => return (43, false),
        '|' => return (43, true),
        _ => {}
    }
    let find = |rows: [(&str, Code); 4]| rows.iter().find_map(|(row, first)| row.find(c).map(|i| first + i as Code));
    find(PLAIN)
        .map(|code| (code, false))
        .or_else(|| find(SHIFTED).map(|code| (code, true)))
        .unwrap_or_else(|| panic!("no key for {c:?} on the US layout"))
}

/// A chord in the drive's spelling (`cmd-t`) as Linux binds it: Ctrl for Cmd, and Chrome's
/// Ctrl+Tab pair for stepping tabs, which macOS spells with brackets.
pub fn chord(keys: &str) -> String {
    match keys {
        "cmd-shift-[" => "ctrl-shift-tab".into(),
        "cmd-shift-]" => "ctrl-tab".into(),
        _ => keys.replace("cmd-", "ctrl-"),
    }
}

/// A private headless Hyprland with XWayland.
pub struct Display {
    hyprland: Hyprland,
}

impl Display {
    pub fn start() -> Self {
        Self::with(Options::default())
    }

    pub fn with(options: Options) -> Self {
        let hyprland =
            Hyprland::start(Options { xwayland: true, ..options }).unwrap_or_else(|e| panic!("headless Hyprland: {e}"));
        Self { hyprland }
    }

    /// `program` as a client of this instance.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        self.hyprland.command(program)
    }
}

/// A launched app with the compositor it runs on. The app goes first on drop.
pub struct App {
    handle: AppHandle,
    input: RefCell<VirtualInput>,
    display: Display,
}

impl Deref for App {
    type Target = AppHandle;
    fn deref(&self) -> &AppHandle {
        &self.handle
    }
}

pub fn launch(options: LaunchOptions) -> App {
    launch_on(options, Display::start())
}

pub fn launch_on(mut options: LaunchOptions, display: Display) -> App {
    for (key, value) in display.hyprland.client_env() {
        options = options.env(key, value);
    }
    let handle = launch_app(options).unwrap();
    let input = RefCell::new(VirtualInput::connect(&display.hyprland).unwrap());
    let app = App { handle, input, display };
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while windows(&app).is_empty() {
        assert!(std::time::Instant::now() < deadline, "no window mapped:\n{}", app.display.hyprland.log_tail());
        std::thread::sleep(Duration::from_millis(100));
    }
    app
}

/// The colour at `at`, in the window's points, read from the output as it is now.
pub fn pixel(app: &App, win: &Win, _shot: &Path, at: (f64, f64)) -> [u8; 3] {
    app.output().rgb((win.bounds.0 + at.0) as u32, (win.bounds.1 + at.1) as u32)
}

/// The window's pixels in the last `width` points before its right edge, every `step` points,
/// from one frame.
pub fn right_edge(app: &App, win: &Win, width: f64, step: usize) -> Vec<[u8; 3]> {
    let frame = app.output();
    let (right, bottom) = (win.bounds.0 + win.bounds.2, win.bounds.1 + win.bounds.3);
    let xs = ((right - width).max(0.) as u32..(right as u32).min(frame.width)).step_by(step);
    let ys = (win.bounds.1.max(0.) as u32..(bottom as u32).min(frame.height)).step_by(step);
    ys.flat_map(|y| xs.clone().map(move |x| (x, y))).map(|(x, y)| frame.rgb(x, y)).collect()
}

/// Hyprland gives the keyboard to the window under a click, so there is no app to bring forward.
pub fn in_front(_: &App) {}

pub fn activate(_: &App) {}

/// Where a hung process's main thread waits, as the kernel reports it; reading its stack needs
/// ptrace, which Yama allows only on the test's own children.
/// Where the focus went, for a failure that finds no key window. The compositor's state is in
/// the failure's own capture here.
pub fn focus_report(_: &App) -> String {
    String::new()
}

pub fn main_thread_of(pid: u32) -> String {
    let wchan = std::fs::read_to_string(format!("/proc/{pid}/wchan")).unwrap_or_default();
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
    format!("main thread of {pid} waits in {wchan:?}, stat {stat}")
}

impl App {
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        self.display.command(program)
    }

    pub fn hyprland(&self) -> &Hyprland {
        &self.display.hyprland
    }

    pub fn input(&self) -> std::cell::RefMut<'_, VirtualInput> {
        self.input.borrow_mut()
    }

    /// One IPC request that answers "ok": `eval <lua>`, `output remove <name>`.
    pub fn ok(&self, request: &str) {
        let reply = self.display.hyprland.request(request).unwrap();
        assert_eq!(reply.trim(), "ok", "{request}");
    }

    /// The whole output as presented, in physical pixels.
    pub fn output(&self) -> CapturedFrame {
        ndshell::wayland::screenshot(&self.display.hyprland).unwrap()
    }

    /// One Lua dispatcher over IPC, failing the case on anything but "ok".
    pub fn dispatch(&self, call: &str) {
        self.ok(&format!("dispatch {call}"));
    }

    /// Whether the compositor has the front window in fullscreen.
    pub fn wm_fullscreen(&self) -> bool {
        front_client(self).is_some_and(|c| c["fullscreen"].as_i64().unwrap_or(0) != 0)
    }
}

/// The cursor the app's X server shows now, as XFixes reports it: its name (empty for a cursor
/// made without one) and its image's size and pixels.
pub fn cursor(app: &App) -> (String, u16, u16, Vec<u32>) {
    use x11rb::protocol::xfixes::ConnectionExt as _;
    let display = app.display.hyprland.client_env().into_iter().find(|(k, _)| *k == "DISPLAY").expect("XWayland").1;
    let (conn, _) = x11rb::connect(Some(&display)).unwrap();
    conn.xfixes_query_version(5, 0).unwrap().reply().unwrap();
    let c = conn.xfixes_get_cursor_image_and_name().unwrap().reply().unwrap();
    (String::from_utf8_lossy(&c.name).into_owned(), c.width, c.height, c.cursor_image)
}

/// The app's mapped windows, most recently focused first.
fn clients(app: &App) -> Vec<serde_json::Value> {
    let Ok(reply) = app.display.hyprland.request("j/clients") else { return Vec::new() };
    let parsed: serde_json::Value = serde_json::from_str(&reply).unwrap_or_default();
    let pid = i64::from(app.pid());
    let mut ours: Vec<serde_json::Value> = parsed
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["pid"].as_i64() == Some(pid) && c["mapped"] != false && c["hidden"] != true)
        .cloned()
        .collect();
    ours.sort_by_key(|c| c["focusHistoryID"].as_i64().unwrap_or(i64::MAX));
    ours
}

fn front_client(app: &App) -> Option<serde_json::Value> {
    clients(app).into_iter().find(|c| c["size"][0].as_f64() >= Some(400.) && c["size"][1].as_f64() >= Some(300.))
}

fn front(app: &App) -> Win {
    windows(app).into_iter().find(|w| w.bounds.2 >= 400. && w.bounds.3 >= 300.).expect("the app has a window")
}

/// One of the app's windows, in the output's logical pixels.
#[derive(Clone, Debug)]
pub struct Win {
    pub bounds: (f64, f64, f64, f64),
    pub address: String,
}

pub fn windows(app: &App) -> Vec<Win> {
    clients(app)
        .into_iter()
        .map(|c| {
            let n = |v: &serde_json::Value| v.as_f64().unwrap_or(0.);
            Win {
                bounds: (n(&c["at"][0]), n(&c["at"][1]), n(&c["size"][0]), n(&c["size"][1])),
                address: c["address"].as_str().unwrap_or_default().to_owned(),
            }
        })
        .collect()
}

/// How many of the app's windows are menus or popovers: Hyprland lists them among its clients with
/// no layer of their own, so this counts every window.
pub fn windows_above(app: &App) -> usize {
    windows(app).len()
}

/// How many windows the app has, however small: the compositor lists X dialogs among its clients.
pub fn every_window(app: &App) -> usize {
    windows(app).len()
}

pub fn click(app: &App, at: (f64, f64)) {
    app.input.borrow_mut().click(at.0, at.1).unwrap();
}

pub fn move_to(app: &App, at: (f64, f64)) {
    app.input.borrow_mut().move_to(at.0, at.1);
}

/// Moves the pointer to `at` with the left button down, between a `hold` and a `release`.
pub fn drag_move(app: &App, at: (f64, f64)) {
    move_to(app, at);
}

/// Presses the left button at `at` and keeps it down.
pub fn hold(app: &App, at: (f64, f64)) {
    let mut input = app.input.borrow_mut();
    input.move_to(at.0, at.1);
    input.sync().unwrap();
    input.button(true);
    input.sync().unwrap();
}

/// Lets go of the button `hold` pressed.
pub fn release(app: &App, _: (f64, f64)) {
    let mut input = app.input.borrow_mut();
    input.button(false);
    input.sync().unwrap();
}

/// Drags the left button from `from` by `(dx, dy)` in `steps` moves.
pub fn drag(app: &App, from: (f64, f64), dx: f64, dy: f64, steps: u32) {
    app.input.borrow_mut().drag(from, dx, dy, steps, Duration::from_millis(16)).unwrap();
}

/// Scrolls the wheel over `at` by `dy` points, negative down as on macOS, in wheel notches.
pub fn scroll(app: &App, at: (f64, f64), dy: i32, _steps: u32) {
    app.input.borrow_mut().scroll(at.0, at.1, -dy / 100).unwrap();
    std::thread::sleep(Duration::from_millis(120));
}

/// Clicks the button titled `title` in the app's front window: GPUI draws alerts and dialogs on
/// Linux, and nd puts them in the automation tree. The alert shows a frame after the click that
/// asked for it, so the button is looked for until it is laid out.
pub fn click_button(app: &App, title: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let r = loop {
        let tree = app.tree(None).unwrap();
        let found = tree
            .windows
            .iter()
            .filter(|w| w.key)
            .find_map(|w| w.root.iter().find(|n| format!("{:?}", n.role) == "Button" && n.name == title)?.bounds);
        if let Some(found) = found {
            break found;
        }
        if std::time::Instant::now() > deadline {
            let dialogs: Vec<_> = tree
                .windows
                .iter()
                .flat_map(|w| w.root.iter().filter(|n| format!("{:?}", n.role) == "Dialog"))
                .map(|d| (d.name.clone(), d.children.iter().map(|b| b.name.clone()).collect::<Vec<_>>()))
                .collect();
            panic!("no {title} button in the key window; dialogs showing: {dialogs:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let (x, y, _, _) = front(app).bounds;
    click(app, (x + f64::from(r.origin.x + r.size.width / 2.), y + f64::from(r.origin.y + r.size.height / 2.)));
    std::thread::sleep(Duration::from_millis(500));
}

/// Whether a drawn alert in one of the app's windows shows a button called `title`.
pub fn alert_button_shown(app: &App, title: &str) -> bool {
    let Ok(tree) = app.tree(None) else { return false };
    tree.windows
        .iter()
        .flat_map(|w| w.root.iter())
        .any(|n| format!("{:?}", n.role) == "Dialog" && n.children.iter().any(|b| b.name == title))
}

pub fn right_click(app: &App, at: (f64, f64)) {
    app.input.borrow_mut().right_click(at.0, at.1).unwrap();
}

pub fn key(app: &App, code: Code, modifiers: &[Modifier]) {
    app.input.borrow_mut().key(code, modifiers).unwrap();
}

/// Types `text` on the compositor's keyboard, character by character.
pub fn type_text(app: &App, text: &str) {
    for c in text.chars() {
        let (code, shift) = key_of_char(c);
        key(app, code, if shift { &[Modifier::Shift] } else { &[] });
    }
}

/// Puts `text` on the compositor's clipboard (wl-copy serves it from the background until the
/// next copy) and waits for XWayland's bridge to offer it to X clients, which the app is.
pub fn set_clipboard(app: &App, text: &str) {
    use std::io::Write;
    let mut copy = app.command("wl-copy").stdin(std::process::Stdio::piped()).spawn().unwrap();
    copy.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    assert!(copy.wait().unwrap().success());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let offered =
        || app.command("xsel").args(["--clipboard", "--output"]).output().is_ok_and(|o| o.stdout == text.as_bytes());
    while !offered() {
        assert!(std::time::Instant::now() < deadline, "XWayland never offered the clipboard to X");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The output as presented, cut to `win`.
pub fn capture(app: &App, win: &Win, path: &Path) {
    let frame = app.output();
    let clamp = |v: f64, max: u32| (v.max(0.) as u32).min(max);
    let (x0, y0) = (clamp(win.bounds.0, frame.width), clamp(win.bounds.1, frame.height));
    let (x1, y1) = (clamp(win.bounds.0 + win.bounds.2, frame.width), clamp(win.bounds.1 + win.bounds.3, frame.height));
    let stride = (x1 - x0) * 4;
    let mut data = Vec::with_capacity((stride * (y1 - y0)) as usize);
    for y in y0..y1 {
        let row = (y * frame.stride + x0 * 4) as usize;
        data.extend_from_slice(&frame.data[row..row + stride as usize]);
    }
    let cut = CapturedFrame { width: x1 - x0, height: y1 - y0, stride, data, ..frame };
    cut.write_bmp(path).unwrap();
}

/// Resizes the front window toward `width` by `height` pixels with a drag of its bottom-right
/// corner, which stops at the window's minimum size. Hyprland's resize dispatcher sets any size.
pub fn resize(app: &App, width: u32, height: u32) {
    let (x, y, w, h) = front(app).bounds;
    let corner = (x + w + 1., y + h + 1.);
    let by = (f64::from(width) - w, f64::from(height) - h);
    app.input.borrow_mut().drag(corner, by.0, by.1, 30, Duration::from_millis(16)).unwrap();
}

/// SIGTERM, the way a service manager or `kill` quits the app.
pub fn quit(app: &App) {
    let pid = app.pid().to_string();
    assert!(Command::new("kill").args(["-TERM", &pid]).status().unwrap().success());
}

/// Closes the front window as the compositor's close binding does.
pub fn close_window(app: &App, front: &Win) {
    app.dispatch(&format!("hl.dsp.window.close({{ window = \"address:{}\" }})", front.address));
}

/// The compositor's own fullscreen of the front window, as a Hyprland binding toggles it.
pub fn wm_fullscreen(app: &App, on: bool) {
    let address = front(app).address;
    let action = if on { "set" } else { "unset" };
    app.dispatch(&format!(
        "hl.dsp.window.fullscreen({{ window = \"address:{address}\", action = \"{action}\", mode = \"fullscreen\" }})"
    ));
}

pub fn shot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("shell-{name}.{SHOT_EXT}"))
}
