//! The drive's macOS side: the app on the user's display, real input through the HID system,
//! window bounds and captures from the window server, resizes and menus through System Events.

use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use nd_test::{AppHandle, LaunchOptions, launch_app};
pub use ndshell::input::Modifier;

pub type Code = u16;

/// macOS virtual key codes for the keys the cases post through the HID system.
pub mod key {
    use super::Code;
    pub const F: Code = 3;
    pub const I: Code = 34;
    pub const K: Code = 40;
    pub const L: Code = 37;
    pub const T: Code = 17;
    pub const V: Code = 9;
    pub const ESCAPE: Code = 53;
    pub const ENTER: Code = 36;
    pub const TAB: Code = 48;
    pub const BACKSPACE: Code = 51;
    pub const DOWN: Code = 125;
    pub const PAGE_UP: Code = 116;
    pub const PAGE_DOWN: Code = 121;
    pub const UP: Code = 126;
    pub const LEFT: Code = 123;
    pub const RIGHT: Code = 124;
    pub const A: Code = 0;
    pub const F1: Code = 122;
    pub const F7: Code = 98;
    pub const F12: Code = 111;
    pub const R: Code = 15;
    pub const W: Code = 13;
}

/// The keys that take the caret to the start and the end of a field's line: Cmd+arrows, as in
/// any AppKit field, where Home and End only scroll.
pub const LINE_START: (Code, &[Modifier]) = (key::LEFT, &[Modifier::Command]);
pub const LINE_END: (Code, &[Modifier]) = (key::RIGHT, &[Modifier::Command]);

/// The modifier of the app's shortcuts.
pub const CMD: Modifier = Modifier::Command;

/// Chrome's developer tools chord besides F12.
pub const DEVTOOLS: (&str, Code, &[Modifier]) = ("Alt+Cmd+I", key::I, &[Modifier::Option, Modifier::Command]);
/// The sidebar's "New tab" row; Linux has a plus in the foot instead.
pub const NEW_TAB: &str = "new-tab-row";
/// Captures are PNGs from the window server.
pub const SHOT_EXT: &str = "png";

/// The key code of a lowercase letter or a space, for typing into an open menu.
pub fn key_of(c: char) -> Code {
    const LETTERS: &str = "asdfhgzxcv_bqweryt123465=97-80]ou[ip_lj'k;\\,/nm.";
    if c == ' ' {
        return 49;
    }
    LETTERS.find(c).map(|i| i as Code).expect("a key for the letter")
}

/// A chord in the drive's spelling (`cmd-t`) as the automation keyboard takes it.
pub fn chord(keys: &str) -> String {
    keys.to_owned()
}

/// The display the app runs on: the user's own.
pub struct Display;

impl Display {
    pub fn start() -> Self {
        Self
    }

    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        Command::new(program)
    }
}

/// A launched app.
pub struct App {
    handle: AppHandle,
}

impl Deref for App {
    type Target = AppHandle;
    fn deref(&self) -> &AppHandle {
        &self.handle
    }
}

/// Launches the app and brings it forward: a launched test app is not, and key events would go
/// elsewhere. The accessibility call fails until the app has registered with the window server,
/// so it retries.
/// An app launched outside `launch`, which may have lost a race for its profile.
impl From<AppHandle> for App {
    fn from(handle: AppHandle) -> Self {
        Self { handle }
    }
}

pub fn launch(options: LaunchOptions) -> App {
    let app = App { handle: launch_app(options).unwrap() };
    activate(&app);
    app
}

/// Brings the app forward and waits until macOS has it in front. The accessibility call fails
/// until the app has registered with the window server, and succeeds while another app (Finder,
/// after the last test's app quit) keeps the front, so it retries until the front app is this one.
pub fn activate(app: &App) {
    // A loaded machine takes a while to register a Chromium app with the window server.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !(ndshell::input::activate(app.pid()) && front_pid() == Some(app.pid())) {
        assert!(
            Instant::now() < deadline,
            "the app would not come forward; in front: {}\n{}",
            front_app(),
            app.stderr_tail(40).join("\n")
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    std::thread::sleep(Duration::from_millis(300));
}

/// Waits until one of the app's windows is key, bringing the app forward again meanwhile. A click
/// on a window of an app that is not active only activates it, and an app the test quit a moment
/// earlier hands activation to the next app in line after the new one came forward.
pub fn in_front(app: &App) {
    let key =
        |a: &App| a.windows().ok().is_some_and(|w| w.as_array().is_some_and(|ws| ws.iter().any(|w| w["key"] == true)));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !key(app) {
        assert!(
            Instant::now() < deadline,
            "no window of the app became key; in front: {}, windows {:?}\n{}\n{}",
            front_app(),
            app.windows(),
            focus_report(app),
            app.stderr_tail(40).join("\n")
        );
        ndshell::input::activate(app.pid());
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Every window of the process as accessibility sees it, the ones nd does not list included
/// (Chromium's), each with whether it is main and focused, and which one the process reports
/// focused, and the element that has the keyboard: where a key went when none of the app's own
/// windows or fields has it.
pub fn focus_report(app: &App) -> String {
    let script = format!(
        r#"tell application "System Events" to tell (first process whose unix id is {pid})
            set out to "focused window: "
            try
                set out to out & (name of (value of attribute "AXFocusedWindow"))
            on error
                set out to out & "none"
            end try
            try
                set e to value of attribute "AXFocusedUIElement"
                set out to out & linefeed & "focused element: " & (role of e) & " " & (description of e) & " value " & (value of e as text)
            on error
                set out to out & linefeed & "focused element: none"
            end try
            repeat with w in windows
                set out to out & linefeed & "  " & (name of w) & " role " & (subrole of w) & " main " & (value of attribute "AXMain" of w) & " focused " & (value of attribute "AXFocused" of w) & " at " & (position of w as text) & " size " & (size of w as text)
            end repeat
            return out
        end tell"#,
        pid = app.pid()
    );
    let out = Command::new("osascript").args(["-e", &script]).output();
    out.map(|o| format!("{}{}", String::from_utf8_lossy(&o.stdout).trim(), String::from_utf8_lossy(&o.stderr).trim()))
        .unwrap_or_default()
}

/// The top of a process's main thread stack, sampled for a second: where a hung app is stuck.
pub fn main_thread_of(pid: u32) -> String {
    let out = Command::new("sample").args([&pid.to_string(), "1"]).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let main = text.lines().skip_while(|l| !l.contains("com.apple.main-thread")).take(60);
    main.collect::<Vec<_>>().join("\n")
}

/// The pid of the app macOS has in front. `lsappinfo` prints it as `"pid"=N` for a bundled app
/// and `pid = N` for a bare binary like the test's.
fn front_pid() -> Option<u32> {
    let info = front_app();
    let at = info.find("pid")? + 3;
    info[at..].trim_start_matches(['"', ' ', '=']).split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// The app macOS has in front, by name and pid.
pub fn front_app() -> String {
    Command::new("sh")
        .args(["-c", "lsappinfo info -only name -only pid $(lsappinfo front)"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().replace('\n', " "))
        .unwrap_or_default()
}

impl App {
    /// A command for a second process of the app on the same display.
    pub fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        Command::new(program)
    }
}

/// One of the app's windows on screen, in screen points.
#[derive(Clone, Debug)]
pub struct Win {
    pub bounds: (f64, f64, f64, f64),
    number: u32,
}

/// The app's windows on screen, frontmost first.
pub fn windows(app: &App) -> Vec<Win> {
    ndshell::window_list::of_pid(app.pid()).into_iter().map(|w| Win { bounds: w.bounds, number: w.number }).collect()
}

pub fn click(_: &App, at: (f64, f64)) {
    ndshell::input::click(at);
}

pub fn move_to(_: &App, at: (f64, f64)) {
    ndshell::input::move_to(at);
}

/// Moves the pointer to `at` with the left button down, between a `hold` and a `release`.
pub fn drag_move(_: &App, at: (f64, f64)) {
    ndshell::input::mouse(objc2_core_graphics::CGEventType::LeftMouseDragged, at);
}

/// Drags the left button from `from` by `(dx, dy)` in `steps` moves.
pub fn drag(_: &App, from: (f64, f64), dx: f64, dy: f64, steps: u32) {
    ndshell::input::drag(from, dx, dy, steps);
}

/// Scrolls the wheel over `at` by `dy` points, negative down.
pub fn scroll(_: &App, at: (f64, f64), dy: i32, steps: u32) {
    ndshell::input::scroll(at, dy, steps);
}

/// Clicks the button titled `title` in the app's alert, sheet or window.
pub fn click_button(app: &App, title: &str) {
    let script = format!(
        r#"tell application "System Events" to tell (first process whose unix id is {pid})
            repeat with w in windows
                if exists button "{title}" of w then
                    click button "{title}" of w
                    return
                end if
                if exists sheet 1 of w then
                    if exists button "{title}" of sheet 1 of w then
                        click button "{title}" of sheet 1 of w
                        return
                    end if
                end if
            end repeat
            error "no button {title}"
        end tell"#,
        pid = app.pid()
    );
    let status = Command::new("osascript").args(["-e", &script]).status().unwrap();
    assert!(status.success(), "no {title} button");
    std::thread::sleep(Duration::from_millis(500));
}

/// Whether an alert of the app shows a button called `title`, in a window or a sheet.
pub fn alert_button_shown(app: &App, title: &str) -> bool {
    let script = format!(
        r#"tell application "System Events" to tell (first process whose unix id is {pid})
            repeat with w in windows
                if exists button "{title}" of w then return "yes"
                if exists sheet 1 of w then
                    if exists button "{title}" of sheet 1 of w then return "yes"
                end if
            end repeat
            return "no"
        end tell"#,
        pid = app.pid()
    );
    let out = Command::new("osascript").args(["-e", &script]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim() == "yes"
}

/// How many menus and popovers the app shows: its windows above the normal layer.
pub fn open_menus(app: &App) -> usize {
    let pid = app.pid();
    ndshell::window_list::every_layer_of_pid(pid).len() - ndshell::window_list::of_pid(pid).len()
}

/// How many windows the app has on every layer, however small.
pub fn every_window(app: &App) -> usize {
    ndshell::window_list::every_layer_of_pid(app.pid()).len()
}

pub fn hold(_: &App, at: (f64, f64)) {
    ndshell::input::hold(at);
}

pub fn release(_: &App, at: (f64, f64)) {
    ndshell::input::release(at);
}

pub fn right_click(_: &App, at: (f64, f64)) {
    ndshell::input::right_click(at);
}

#[track_caller]
pub fn key(app: &App, code: Code, modifiers: &[Modifier]) {
    still_in_front(app);
    ndshell::input::key(code, modifiers);
}

/// Types `text` at the HID tap, character by character.
#[track_caller]
pub fn type_text(app: &App, text: &str) {
    still_in_front(app);
    ndshell::input::type_text(text);
}

/// HID keys go to whichever app macOS has in front. An app that took the front since the case
/// brought this one forward would take them instead, so the case fails naming it.
#[track_caller]
fn still_in_front(app: &App) {
    assert!(front_pid() == Some(app.pid()), "another app took the front before a key; in front: {}", front_app());
}

pub fn set_clipboard(_: &App, text: &str) {
    use std::io::Write;
    // pbcopy reads its input in the locale's encoding, MacRoman without one.
    let mut copy =
        Command::new("pbcopy").env("LANG", "en_US.UTF-8").stdin(std::process::Stdio::piped()).spawn().unwrap();
    copy.stdin.take().unwrap().write_all(text.as_bytes()).unwrap();
    assert!(copy.wait().unwrap().success());
}

/// A composited capture of `win`, glass and pages included, which the automation socket's layer
/// render leaves out.
pub fn capture(_: &App, win: &Win, path: &Path) {
    ndshell::window_list::capture(win.number, path).unwrap();
}

/// The colour at `at`, in the window's points, read from `shot`, a capture of `win`.
pub fn pixel(_: &App, win: &Win, shot: &Path, at: (f64, f64)) -> [u8; 3] {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(shot).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    let scale = f64::from(info.width) / win.bounds.2;
    let (x, y) = ((at.0 * scale) as usize, (at.1 * scale) as usize);
    let i = y * info.line_size + x * info.color_type.samples();
    [buf[i], buf[i + 1], buf[i + 2]]
}

/// Resizes the front window to `width` points, keeping its height, as a drag of its edge would.
pub fn resize(app: &App, width: u32, height: u32) {
    let pid = app.pid();
    let script = format!(
        "tell application \"System Events\" to set size of front window of (first process whose unix id is {pid}) to {{{width}, {height}}}"
    );
    let status = Command::new("osascript").args(["-e", &script]).status().unwrap();
    assert!(status.success(), "the window would not resize");
    // System Events leaves the window inactive, and a click on an inactive page only activates it.
    activate(app);
}

/// Moves the front window's top-left corner to `(x, y)` in global points.
pub fn move_window(app: &App, x: i32, y: i32) {
    let pid = app.pid();
    let script = format!(
        "tell application \"System Events\" to set position of front window of (first process whose unix id is {pid}) to {{{x}, {y}}}"
    );
    let status = Command::new("osascript").args(["-e", &script]).status().unwrap();
    assert!(status.success(), "the window would not move");
    activate(app);
    std::thread::sleep(Duration::from_millis(800));
}

/// SIGTERM, the way a service manager or `kill` quits the app.
pub fn quit(app: &App) {
    let pid = app.pid().to_string();
    assert!(Command::new("kill").args(["-TERM", &pid]).status().unwrap().success());
}

/// Picks `item` in the menu bar's `menu` as a user does, through System Events.
pub fn menu(app: &App, menu: &str, item: &str) {
    let script = format!(
        "tell application \"System Events\" to tell (first process whose unix id is {}) to click menu item \"{item}\" of menu \"{menu}\" of menu bar 1",
        app.pid()
    );
    let status = Command::new("osascript").args(["-e", &script]).status().unwrap();
    assert!(status.success(), "no {menu} > {item}");
}

/// Closes the front window as its close button does: the traffic light's centre, in window
/// points.
/// Clicks the close light, whose edges sit 19 in from the window's left and top
/// (docs/sidebar.md) and which is 14 across.
pub fn close_window(app: &App, front: &Win) {
    click(app, (front.bounds.0 + 26., front.bounds.1 + 26.));
}

pub fn shot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("shell-{name}.{SHOT_EXT}"))
}
