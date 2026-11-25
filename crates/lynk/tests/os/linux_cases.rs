//! The drive's Linux-only cases: what the compositor, XWayland and the desktop can do to a
//! Chrome-style page that macOS has no counterpart for. Each case starts its own headless
//! Hyprland (`os/linux.rs`), so scales, outputs and workspaces never leak between cases.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

use ndshell::hyprland::OUTPUT;
use ndshell::wayland::CapturedFrame;

use super::*;

/// ALPHA's link block, `#cde`, 200 CSS px tall across the page.
const BLOCK: [u8; 3] = [0xcc, 0xdd, 0xee];

/// A canvas stream with a tone on it, played in a video element once the page is clicked.
const MEDIA: &str = r#"<!doctype html><title>Media</title>
<body style="margin:0;height:400px;background:#222" onclick="go()"><video id="v" width="320" height="240"></video>
<script>
let started = false;
async function go() {
  if (started) return;
  started = true;
  const c = document.createElement("canvas");
  c.width = 320; c.height = 240;
  const g = c.getContext("2d");
  let n = 0;
  setInterval(() => { g.fillStyle = `hsl(${n++ % 360} 80% 50%)`; g.fillRect(0, 0, 320, 240); }, 33);
  const ac = new AudioContext();
  const tone = ac.createOscillator();
  const out = ac.createMediaStreamDestination();
  tone.connect(out);
  tone.start();
  const stream = c.captureStream(30);
  stream.addTrack(out.stream.getAudioTracks()[0]);
  v.srcObject = stream;
  await v.play();
  setInterval(() => { document.title = `Playing ${v.currentTime.toFixed(1)} ${ac.state} ${v.muted}`; }, 250);
}
</script>"#;

/// Serves `/alpha`, `/media`, and `/slow`, which sends its head and never finishes, each
/// connection on its own thread.
fn serve_linux() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut stream in listener.incoming().map_while(Result::ok) {
            std::thread::spawn(move || {
                let mut line = String::new();
                let _ = BufReader::new(&stream).read_line(&mut line);
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let body = match path.as_str() {
                    "/alpha" => ALPHA,
                    "/media" => MEDIA,
                    "/slow" => {
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<title>Slow</title>");
                        std::thread::sleep(Duration::from_secs(60));
                        return;
                    }
                    _ => "<!doctype html><title>Missing</title>",
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    base
}

fn near(c: [u8; 3], to: [u8; 3]) -> bool {
    c.iter().zip(to).all(|(a, b)| a.abs_diff(b) <= 12)
}

/// ALPHA's block pixels inside `win`, every fourth pixel, at scale 1.
fn block_pixels(frame: &CapturedFrame, win: &os::Win) -> u32 {
    let (x, y, w, h) = win.bounds;
    let xs = (x.max(0.) as u32..((x + w) as u32).min(frame.width)).step_by(4);
    let ys = (y.max(0.) as u32..((y + h) as u32).min(frame.height)).step_by(4);
    ys.flat_map(|y| xs.clone().map(move |x| (x, y))).filter(|&(x, y)| near(frame.rgb(x, y), BLOCK)).count() as u32
}

/// Whether ALPHA is drawn in `win`: its block shows.
fn painted(app: &App, win: &os::Win) -> bool {
    block_pixels(&app.output(), win) > 500
}

/// The longest run of `BLOCK` along row `y` from `x0` to `x1`.
fn block_run(frame: &CapturedFrame, y: u32, x0: u32, x1: u32) -> u32 {
    let (mut best, mut run) = (0, 0);
    for x in x0..x1.min(frame.width) {
        run = if near(frame.rgb(x, y), BLOCK) { run + 1 } else { 0 };
        best = best.max(run);
    }
    best
}

/// The page fills the window beside the sidebar: the block's run along a row of it is the
/// window's width less the sidebar and its divider.
fn assert_page_fills(app: &App, win: &os::Win, what: &str) {
    let (x, y, w, _) = win.bounds;
    let run = block_run(&app.output(), (y + 100.) as u32, x as u32, (x + w) as u32);
    let expected = w - f64::from(lynk::sidebar::WIDTH);
    assert!((f64::from(run) - expected).abs() <= 4., "{what}: the page is {run} px wide in a {w} px window");
}

fn alpha_options(base: &str) -> LaunchOptions {
    options(&format!("{base}/alpha"))
}

/// A fullscreen the compositor started (a Hyprland binding) is the window's, not Lynk's: the
/// sidebar stays, and F11 leaves it.
#[test]
fn real_f11_leaves_the_compositors_fullscreen() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    let width = front_window(&app).bounds.2;
    os::wm_fullscreen(&app, true);
    until(&app, "the compositor's fullscreen", 10, |a| a.wm_fullscreen() && width_of(a).is_some_and(|w| w > width));
    std::thread::sleep(Duration::from_millis(500));
    assert!(has(&app, "controls-row"), "the sidebar stays in the compositor's fullscreen");
    click_page(&app, 100.);
    hid(&app, key::F11, &[]);
    until(&app, "out of the compositor's fullscreen", 10, |a| !a.wm_fullscreen() && width_of(a) == Some(width));
    assert!(has(&app, "controls-row"));
    // F11 again is Lynk's own fullscreen, and F11 after it leaves that too.
    hid(&app, key::F11, &[]);
    until(&app, "F11's fullscreen", 10, |a| a.wm_fullscreen() && lacks(a, "controls-row"));
    hid(&app, key::F11, &[]);
    until(&app, "out of F11's fullscreen", 10, |a| !a.wm_fullscreen() && has(a, "controls-row"));
}

/// At 1.25 and 1.5, with XWayland scaled by the compositor and with `force_zero_scaling`, the
/// page and the app around it share one scale: a CSS pixel is as big as one of the app's.
#[test]
fn real_scale_and_xwayland_zero_scaling() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for (scale, zero) in [("1.25", false), ("1.25", true), ("1.5", false), ("1.5", true)] {
        let lua = format!(
            "hl.monitor({{ output = \"{OUTPUT}\", mode = \"1920x1200@60\", position = \"0x0\", scale = \"{scale}\" }})\n\
             hl.config({{ xwayland = {{ force_zero_scaling = {zero} }} }})"
        );
        // 1920x1200 divides by both scales; Hyprland moves a scale that does not to one that does.
        let options = os::Options { width: 1920, height: 1200, extra_config: lua, ..os::Options::default() };
        let app = os::launch_on(alpha_options(&base), os::Display::with(options));
        until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
        std::thread::sleep(Duration::from_secs(1));
        let frame = app.output();
        let name = format!("scale-{scale}-{}", if zero { "zero" } else { "scaled" });
        frame.write_bmp(&os::shot_path(&name)).unwrap();
        // The window fills the output at these scales, so its place comes from the compositor, in
        // logical pixels, and the app's own width says what scale it draws at.
        let s: f64 = scale.parse().unwrap();
        let win = front_window(&app);
        let col = ((win.bounds.0 + win.bounds.2 * 0.75) * s) as u32;
        let y0 = (win.bounds.1 * s) as u32;
        let logical = app.windows().unwrap()[0]["frame"]["width"].as_f64().expect("the window's frame");
        let app_scale = win.bounds.2 * s / logical;
        let mut run = 0;
        for y in y0..frame.height {
            if near(frame.rgb(col, y), BLOCK) {
                run += 1;
            } else if run > 0 {
                break;
            }
        }
        let page_scale = f64::from(run) / 200.;
        assert!(
            (page_scale / app_scale - 1.).abs() < 0.04,
            "{name}: the page draws at {page_scale:.3}x and the app at {app_scale:.3}x"
        );
    }
}

/// A second output the window moves to goes away under it, and the first output changes mode
/// while the page is shown: the page comes back drawn, filling its window, and takes clicks.
#[test]
fn real_output_unplugged_and_resized() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    app.ok("output create headless HEADLESS-2");
    app.ok(r#"eval hl.monitor({ output = "HEADLESS-2", mode = "1280x800@60", position = "1600x0", scale = "1" })"#);
    until(&app, "the second output", 10, |a| {
        a.hyprland().request("j/monitors").is_ok_and(|m| m.contains("HEADLESS-2"))
    });
    let address = front_window(&app).address;
    app.dispatch(&format!(r#"hl.dsp.window.move({{ window = "address:{address}", monitor = "HEADLESS-2" }})"#));
    until(&app, "the window on the second output", 10, |a| front_window(a).bounds.0 >= 1600.);
    app.ok("output remove HEADLESS-2");
    until(&app, "the window back on the first", 10, |a| on_screen_window(a).is_some_and(|w| w.bounds.0 < 1600.));
    // Its workspace comes along to the first output, behind the one shown there.
    app.dispatch(&format!(r#"hl.dsp.focus({{ window = "address:{address}" }})"#));
    std::thread::sleep(Duration::from_secs(1));
    shot(&app, "output-back");
    assert!(painted(&app, &front_window(&app)), "the page draws on the output it came back to");
    app.ok(&format!(
        r#"eval hl.monitor({{ output = "{OUTPUT}", mode = "1280x720@60", position = "0x0", scale = "1" }})"#
    ));
    std::thread::sleep(Duration::from_millis(1500));
    shot(&app, "output-resized");
    assert!(painted(&app, &front_window(&app)), "the page draws after the output's mode changed");
    click_page(&app, 100.);
    until(&app, "the link to load Beta", 15, |a| titles(a) == ["Beta"]);
}

/// Both windows tiled side by side, then floated again: each time the page fills its half or its
/// window, drawn.
#[test]
fn real_tiled_half_and_floated_again() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    let alpha = front_window(&app).address;
    press(&app, "cmd-n");
    until(&app, "two windows mapped", 20, |a| os::windows(a).len() == 2);
    for action in ["unset", "set"] {
        for w in os::windows(&app) {
            app.dispatch(&format!(
                r#"hl.dsp.window.float({{ window = "address:{}", action = "{action}" }})"#,
                w.address
            ));
        }
        std::thread::sleep(Duration::from_millis(1500));
        let win = os::windows(&app).into_iter().find(|w| w.address == alpha).unwrap();
        if action == "unset" {
            assert!(win.bounds.2 <= 801., "tiled to half the output: {:?}", win.bounds);
        }
        shot(&app, &format!("tiled-{action}"));
        assert!(painted(&app, &win), "the page draws once the window is {action} floating");
        assert_page_fills(&app, &win, &format!("float {action}"));
    }
}

/// Away on another workspace and back: the page draws again, not blank, and takes a click.
#[test]
fn real_workspace_switch_repaints() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    let win = front_window(&app);
    app.dispatch(r#"hl.dsp.focus({ workspace = "2" })"#);
    std::thread::sleep(Duration::from_millis(800));
    assert!(!painted(&app, &win), "workspace 2 shows nothing of the window");
    app.dispatch(r#"hl.dsp.focus({ workspace = "1" })"#);
    std::thread::sleep(Duration::from_millis(800));
    assert!(painted(&app, &win), "the page draws again on its workspace");
    click_page(&app, 100.);
    until(&app, "the link to load Beta", 15, |a| titles(a) == ["Beta"]);
}

fn find_text(app: &App) -> Option<String> {
    app.get_by_test_id("find-field").value().ok().flatten()
}

/// Waits for the find field to hold `want`, failing with what it holds instead.
fn field_becomes(app: &App, want: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while find_text(app).as_deref() != Some(want) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(150));
    }
    assert_eq!(find_text(app).as_deref(), Some(want));
}

/// Typing into a field across a layout switch, then a dead key: US "ab", German "z" on the
/// key US has "y" on, and US International's dead acute before "e". The address field comes
/// with the command bar; the find field is the shell's text field until then.
#[test]
fn real_layout_switch_and_dead_key() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    // The session's layouts, as its input config gives them: US, German, US International.
    app.input().set_layout("us,de,us", ",,intl").unwrap();
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    press(&app, "cmd-f");
    until(&app, "the find bar", 10, |a| has(a, "find-bar"));
    // Over the page, where X hands the keys to Chromium's window.
    let (wx, wy, ww, wh) = front_window(&app).bounds;
    app.input().move_to(wx + ww * 0.7, wy + wh * 0.6);
    // A switch reaches the app as an XKB state change apart from the keys; a person's pause
    // after switching keeps the two in order.
    let switch = |group: u32| {
        app.input().set_group(group).unwrap();
        std::thread::sleep(Duration::from_millis(300));
    };
    app.input().key(30, &[]).unwrap();
    app.input().key(48, &[]).unwrap();
    switch(1);
    app.input().key(21, &[]).unwrap();
    switch(2);
    app.input().key(40, &[]).unwrap();
    app.input().key(18, &[]).unwrap();
    field_becomes(&app, "abz\u{e9}");
}

/// A page that finishes loading while the find field has the keyboard leaves it there: Chromium
/// takes focus as its page loads, and nd follows only a press in the page's window.
#[test]
fn real_a_loading_page_leaves_the_keyboard_where_it_is() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/late"));
    press(&app, "cmd-f");
    until(&app, "the find bar", 10, |a| has(a, "find-bar"));
    let (wx, wy, ww, wh) = front_window(&app).bounds;
    app.input().move_to(wx + ww * 0.7, wy + wh * 0.6);
    app.input().key(30, &[]).unwrap();
    assert!(titles(&app) != ["Late"], "the page loaded before the find field had the keyboard");
    until(&app, "the page", 30, |a| titles(a) == ["Late"]);
    std::thread::sleep(Duration::from_millis(500));
    app.input().key(48, &[]).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    field_becomes(&app, "ab");
}

/// Ctrl+V pastes the clipboard a Wayland client set across XWayland, and a middle click the
/// primary selection a double click on the page made. The page's engine runs in this process,
/// so a selection read that blocked the main thread would never get its answer.
#[test]
fn real_clipboard_and_primary_paste() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    press(&app, "cmd-f");
    until(&app, "the find bar", 10, |a| has(a, "find-bar"));
    std::thread::sleep(Duration::from_millis(500));
    shot(&app, "find-bar");
    let mut copy = app.command("wl-copy").args(["--foreground", "needle"]).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(500));
    app.input().key(47, &[Modifier::Control]).unwrap();
    field_becomes(&app, "needle");
    let _ = copy.kill();
    let _ = copy.wait();
    // The primary selection: a double click on a word of the page selects it, and a middle click
    // in the field puts it at the caret. Hyprland's XWayland bridge carries only the clipboard,
    // so a Wayland client's primary selection never reaches X ones and is not tried here.
    let field = app.get_by_test_id("find-field").bounding_box().unwrap().expect("the field is laid out");
    let page = app.get_by_test_id("content").bounding_box().unwrap().expect("the page is laid out");
    let (wx, wy, _, _) = front_window(&app).bounds;
    let at = (wx + f64::from(field.x + field.width / 2.), wy + f64::from(field.y + field.height / 2.));
    // "one", after the first "needle", which the find has selected.
    let word = (wx + f64::from(page.x) + 70., wy + f64::from(page.y) + 226.);
    // A hand's double click: the second press a moment after the first, not in the same instant.
    app.input().click(word.0, word.1).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    app.input().click(word.0, word.1).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    // The double click gave the page the keyboard, which X routes by the focus window.
    // shortcut: a key within ~16 ms of a click that moves focus from a page to an nd field goes
    // to the page, since nd sets X's focus back only once it has seen the click (8 to 16 ms,
    // median 11, over 39 clicks on g815). Past ~30 ms, or once an automation tool hits it, keep
    // X's focus on GPUI's window and forward keys to the page instead.
    app.input().click(at.0, at.1).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    app.input().key(30, &[Modifier::Control]).unwrap();
    app.input().key(14, &[]).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    app.input().middle_click(at.0, at.1).unwrap();
    field_becomes(&app, "one");
    // Ctrl+C on the page and Ctrl+V in the field: the page's own process owns that clipboard.
    app.input().click(word.0, word.1).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    app.input().click(word.0, word.1).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    app.input().key(46, &[Modifier::Control]).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    app.input().click(at.0, at.1).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    app.input().key(30, &[Modifier::Control]).unwrap();
    app.input().key(14, &[]).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    app.input().key(47, &[Modifier::Control]).unwrap();
    field_becomes(&app, "one");
}

/// A video element with a sound track plays on, with no sound server in the run's session.
#[test]
fn real_video_with_sound_plays() {
    if !real_allowed() {
        return;
    }
    let base = serve_linux();
    let app = launch(&format!("{base}/media"));
    until(&app, "the page", 30, |a| titles(a) == ["Media"]);
    click_page(&app, 100.);
    until(&app, "two seconds played, unmuted, its audio running", 15, |a| {
        titles(a).first().is_some_and(|t| {
            let parts: Vec<&str> = t.split(' ').collect();
            parts.len() == 4
                && parts[0] == "Playing"
                && parts[1].parse::<f64>().is_ok_and(|s| s >= 2.)
                && parts[2] == "running"
                && parts[3] == "false"
        })
    });
    shot(&app, "media");
}

/// `chrome://crash` takes its tab to the crash page and leaves the others working;
/// `chrome://gpucrash` takes the GPU process down, and the pages draw again on its successor.
#[test]
fn real_renderer_and_gpu_crash() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let options = LaunchOptions::new(env!("CARGO_BIN_EXE_lynk"))
        .arg(format!("{base}/alpha"))
        .arg("chrome://crash")
        .arg("chrome://gpucrash")
        .env("ND_CEF_STYLE", "chrome");
    let app = os::launch(options);
    until(&app, "the page", 30, |a| titles(a).first().is_some_and(|t| t == "Alpha"));
    press(&app, "cmd-2");
    until(&app, "the crash page", 15, |a| has(a, "crash-page"));
    shot(&app, "renderer-crash");
    press(&app, "cmd-1");
    until(&app, "Alpha shown", 10, |a| lacks(a, "crash-page"));
    press(&app, "cmd-3");
    std::thread::sleep(Duration::from_secs(3));
    press(&app, "cmd-1");
    // The page waits on Chromium's next GPU process: 0.7 to 3.1 s on g815 whether nd tells it the
    // page is shown, steps its size, or does neither, where a plain tab switch draws in 40 ms.
    until(&app, "the page drawn on the next GPU process", 10, |a| painted(a, &front_window(a)));
    shot(&app, "after-gpu-crash");
    click_page(&app, 100.);
    until(&app, "the link to load Beta", 15, |a| titles(a).first().is_some_and(|t| t == "Beta"));
}

/// SIGTERM while a page is still loading quits as cleanly as an idle one.
#[test]
fn real_sigterm_mid_load() {
    if !real_allowed() {
        return;
    }
    let base = serve_linux();
    let app = launch(&format!("{base}/slow"));
    until(&app, "the slow page's title", 30, |a| titles(a) == ["Slow"]);
    assert_eq!(terminate(&app)["profile"]["exit_type"], "Normal");
}

/// A profile in a directory whose path has spaces and non-ASCII letters starts, loads and
/// shuts down cleanly.
#[test]
fn real_profile_path_with_spaces_and_unicode() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let data = std::env::temp_dir().join(format!("Lynk prøfil ✓ {}", std::process::id()));
    let app = os::launch(alpha_options(&base).env("ND_APP_DATA_DIR", data.to_str().unwrap()));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    let pid = app.pid().to_string();
    assert!(std::process::Command::new("kill").args(["-TERM", &pid]).status().unwrap().success());
    exited(&app, 15);
    let prefs = std::fs::read_to_string(data.join("cef/Default/Preferences")).expect("the profile's Preferences");
    let _ = std::fs::remove_dir_all(&data);
    let prefs: serde_json::Value = serde_json::from_str(&prefs).unwrap();
    assert_eq!(prefs["profile"]["exit_type"], "Normal");
}

/// Real sites over the network, each captured: they load to a title of their own and draw more
/// than a flat page.
#[test]
fn real_sites_load() {
    if !real_allowed() {
        return;
    }
    for (name, url) in [
        ("youtube", "https://www.youtube.com/"),
        ("github", "https://github.com/"),
        ("google-sign-in", "https://accounts.google.com/"),
    ] {
        let app = launch(url);
        until(&app, name, 45, |a| titles(a).first().is_some_and(|t| !t.is_empty() && !t.contains('/')));
        std::thread::sleep(Duration::from_secs(4));
        let path: PathBuf = shot(&app, &format!("site-{name}"));
        let frame = app.output();
        let (x, y, w, h) = front_window(&app).bounds;
        let mut colors = std::collections::HashSet::new();
        for py in (y as u32..(y + h) as u32).step_by(7) {
            for px in ((x + 240.) as u32..(x + w) as u32).step_by(7) {
                colors.insert(frame.rgb(px, py));
            }
        }
        assert!(colors.len() > 8, "{name} drew {} colours: {}", colors.len(), path.display());
        println!("{name}: {:?}", titles(&app));
    }
}

/// `scripts/linux-gate install` runs this on the build it installed, with the library path
/// emptied, so the binary has to find CEF beside itself and the rest on the host.
#[test]
fn installed_build_loads_a_page() {
    let Ok(bin) = std::env::var("LYNK_INSTALLED") else { return };
    let url = format!("{}/press", serve());
    let app = os::launch(LaunchOptions::new(bin).env("LYNK_URL", url).env("LD_LIBRARY_PATH", ""));
    until(&app, "the page", 30, |a| titles(a) == ["Press"]);
    std::thread::sleep(Duration::from_secs(1));
    shot(&app, "installed");
}

/// A menu over a live page: the page is Chromium's X window, stacked above everything GPUI
/// draws, so the menu shows and takes clicks only if the page cuts itself around it.
#[test]
fn real_menu_over_a_live_page() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    let win = front_window(&app);
    let (_, y) = rows(&app)[0].1;
    // The menu opens right and down from the pointer, over the page's block.
    let at = (lynk::sidebar::WIDTH - 60., y);
    let over = |app: &App| {
        let w = front_window(app);
        let area =
            os::Win { bounds: (w.bounds.0 + f64::from(at.0) + 80., w.bounds.1 + f64::from(at.1) + 10., 80., 40.), ..w };
        block_pixels(&app.output(), &area)
    };
    let before = over(&app);
    assert!(before > 100, "the block is under where the menu opens: {before}");
    os::right_click(&app, (win.bounds.0 + f64::from(at.0), win.bounds.1 + f64::from(at.1)));
    std::thread::sleep(Duration::from_millis(600));
    let path = shot(&app, "menu-over-page");
    assert_eq!(over(&app), 0, "the page shows through the menu: {}", path.display());
    // GPUI's menus are not in the automation tree: Duplicate Tab is the second 28 pt row from
    // the pointer, and this point on it lies over the page.
    real_click(&app, lynk::sidebar::WIDTH + 20., at.1 + 47.);
    until(&app, "the duplicate", 15, |a| titles(a) == ["Alpha", "Alpha"]);
    // Closed, the page is whole and live again.
    until(&app, "the block back", 5, |a| over(a) >= before - 10);
}

/// Lynk's own layers on the page, the find bar and the load bar, stacked over Chromium's X
/// window: they show, and the find bar's buttons take clicks.
#[test]
fn real_bars_over_a_live_page() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    // Inside the bar's rounded corners, where the page shows by design.
    let block_in = |app: &App, id: &str| {
        let b = app.get_by_test_id(id).bounding_box().unwrap().expect("laid out");
        let w = front_window(app);
        let (x, y) = (w.bounds.0 + f64::from(b.x) + 8., w.bounds.1 + f64::from(b.y) + 8.);
        let area = os::Win { bounds: (x, y, f64::from(b.width) - 16., f64::from(b.height) - 16.), ..w };
        block_pixels(&app.output(), &area)
    };
    press(&app, "cmd-f");
    until(&app, "the find bar", 5, |a| has(a, "find-bar"));
    std::thread::sleep(Duration::from_millis(300));
    let path = shot(&app, "find-bar-over-page");
    assert_eq!(block_in(&app, "find-bar"), 0, "the page shows through the find bar: {}", path.display());
    let close = app.get_by_test_id("find-close").bounding_box().unwrap().expect("laid out");
    real_click(&app, close.x + close.width / 2., close.y + close.height / 2.);
    until(&app, "the find bar closed", 5, |a| lacks(a, "find-bar"));
    assert!(painted(&app, &front_window(&app)), "the page is whole again");

    let app = launch(&format!("{base}/slow"));
    until(&app, "the load under way", 30, |a| has(a, "load-bar"));
    std::thread::sleep(Duration::from_millis(300));
    let path = shot(&app, "load-bar-over-page");
    let bar = app.get_by_test_id("load-bar").bounding_box().unwrap().expect("laid out");
    let win = front_window(&app);
    let (x, y) = (win.bounds.0 + f64::from(bar.x) + 2., win.bounds.1 + f64::from(bar.y + bar.height / 2.));
    let frame = app.output();
    let (on, under) = (frame.rgb(x as u32, y as u32), frame.rgb(x as u32, (y + 20.) as u32));
    assert!(!near(on, under), "the load bar does not show over the page, {on:?} on {under:?}: {}", path.display());
}

/// Tiled four to the output, the window is narrower than its minimum size: a tiling layout is
/// entitled to give a tile any size, and Hyprland honours the hint only for a floating window
/// (the drag in `real_resize_to_the_minimum_during_a_load` stops at it). The split view folds the
/// sidebar away at that width, and the page fills the tile.
#[test]
fn real_tiled_below_the_minimum() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    let alpha = front_window(&app).address;
    for n in 2..=4 {
        press(&app, "cmd-n");
        until(&app, "another window mapped", 20, |a| os::windows(a).len() == n);
    }
    let mut windows = os::windows(&app);
    windows.sort_by_key(|w| w.address == alpha);
    for w in windows {
        app.dispatch(&format!(r#"hl.dsp.window.float({{ window = "address:{}", action = "unset" }})"#, w.address));
    }
    std::thread::sleep(Duration::from_millis(1500));
    let win = os::windows(&app).into_iter().find(|w| w.address == alpha).unwrap();
    assert!(win.bounds.2 < 720., "tiled narrower than the minimum: {:?}", win.bounds);
    os::capture(&app, &win, &os::shot_path("tiled-below-minimum"));
    let (x, y, w, _) = win.bounds;
    let run = block_run(&app.output(), (y + 100.) as u32, x as u32, (x + w) as u32);
    assert!((f64::from(run) - w).abs() <= 4., "the page is {run} px wide in a {w} px tile");
}

/// The pointer over a link, a text field, a paragraph's text and an empty spot: the X server
/// shows the page's own cursor for each (a hand, an I-beam, an I-beam, the arrow). Chromium sets
/// it on its window only when nd leaves the change to it. The images go to the run's shots.
#[test]
fn real_the_cursor_follows_the_page() {
    if !real_allowed() {
        return;
    }
    let page = "data:text/html,<style>body{margin:0;font:20px sans-serif}a,input,p{display:block;height:60px;margin:0}</style><a href=%23x>a link to follow</a><input value=field><p>Plain text in a paragraph</p>";
    let app = launch(page);
    until(&app, "the page", 30, |a| a.get_by_test_id("content").bounding_box().ok().flatten().is_some());
    std::thread::sleep(Duration::from_secs(1));
    let page = app.get_by_test_id("content").bounding_box().unwrap().unwrap();
    let (wx, wy, _, _) = front_window(&app).bounds;
    let at = |dy: f64| (wx + f64::from(page.x) + 40., wy + f64::from(page.y) + dy);
    let over = |what: &str, point: (f64, f64)| {
        os::move_to(&app, point);
        std::thread::sleep(Duration::from_millis(500));
        let cursor = os::cursor(&app);
        write_cursor(&cursor, &format!("cursor-{what}"));
        cursor.3
    };
    let link = over("link", at(20.));
    let field = over("field", at(90.));
    let text = over("text", at(140.));
    let empty = over("empty", at(400.));
    let back = over("link-again", at(20.));
    assert_ne!(link, empty, "a hand over the link");
    assert_ne!(field, empty, "an I-beam over the field");
    assert_ne!(field, link);
    assert_eq!(text, field, "an I-beam over the paragraph's text");
    assert_eq!(back, link, "the hand again over the link");
}

/// A cursor image as XFixes reports it (premultiplied ARGB), as a PNG under the run's shots.
fn write_cursor((_, width, height, pixels): &(String, u16, u16, Vec<u32>), name: &str) {
    let path = os::shot_path(name).with_extension("png");
    let mut encoder = png::Encoder::new(std::fs::File::create(&path).unwrap(), u32::from(*width), u32::from(*height));
    encoder.set_color(png::ColorType::Rgba);
    let mut writer = encoder.write_header().unwrap();
    let rgba: Vec<u8> =
        pixels.iter().flat_map(|p| [(p >> 16) as u8, (p >> 8) as u8, *p as u8, (p >> 24) as u8]).collect();
    writer.write_image_data(&rgba).unwrap();
    println!("screenshot {}", path.display());
}

/// How a quit is asked for.
#[derive(Clone, Copy, Debug)]
enum Quit {
    /// Ctrl+Q.
    Key,
    /// The command bar's Quit.
    Command,
    /// The compositor's close of the last window (Super+Q, `killactive`).
    Close,
    /// SIGTERM, as a service manager or `kill` sends.
    Term,
}

/// Asks `app` to quit as `how` does; its windows go at once and the process within 2 s.
#[track_caller]
fn quit_by(app: &App, how: Quit) {
    match how {
        Quit::Key => os::key(app, os::key_of('q'), &[os::CMD]),
        Quit::Command => {
            open_bar(app, key::K);
            type_real(app, "Quit");
            let is_quit = |r: &String| r.starts_with("Quit") && r.ends_with("Ctrl+Q");
            until(app, "the Quit row", 5, |a| bar_rows(a).iter().any(is_quit));
            for _ in 0..bar_rows(app).iter().position(is_quit).unwrap() {
                os::key(app, key::DOWN, &[]);
            }
            os::key(app, key::ENTER, &[]);
        }
        Quit::Close => os::close_window(app, &front_window(app)),
        Quit::Term => os::quit(app),
    }
    let start = Instant::now();
    until(app, &format!("the windows gone after {how:?}"), 1, |a| os::windows(a).is_empty());
    exited(app, 2);
    println!("quit by {how:?} in {} ms", start.elapsed().as_millis());
}

/// Every way to quit ends the app at once and saves the session.
#[test]
fn real_every_way_to_quit() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for how in [Quit::Key, Quit::Command, Quit::Close, Quit::Term] {
        let profile = profile(&format!("quit-{how:?}"));
        let app = launch_on(&profile, &[format!("{base}/alpha"), format!("{base}/beta")]);
        until(&app, "the pages", 30, |a| titles(a).first().is_some_and(|t| t == "Alpha"));
        quit_by(&app, how);
        let tabs = session_of(&profile)["windows"][0]["tabs"].as_array().map(Vec::len);
        assert_eq!(tabs, Some(2), "{how:?} saved both tabs");
    }
}

/// Thirty quits in a row with pages loaded, by turns of the key, the compositor and SIGTERM.
#[test]
fn real_thirty_quits_in_a_row() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    for n in 0..30 {
        let how = [Quit::Key, Quit::Close, Quit::Term][n % 3];
        let app = launch_with(&[format!("{base}/alpha"), format!("{base}/beta?{n}")]);
        until(&app, "the page", 30, |a| titles(a).first().is_some_and(|t| t == "Alpha"));
        quit_by(&app, how);
    }
}

/// A quit while a page is still loading, with DevTools open, and with two windows.
#[test]
fn real_quit_mid_load_with_devtools_and_two_windows() {
    if !real_allowed() {
        return;
    }
    let base = serve();
    let app = launch(&format!("{base}/slow"));
    until(&app, "the never-ending load", 15, |a| has(a, "load-bar"));
    quit_by(&app, Quit::Key);

    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    hid(&app, key::F12, &[]);
    until(&app, "DevTools", 15, |a| os::windows(a).len() == 2 || has(a, "devtools"));
    quit_by(&app, Quit::Term);

    // DevTools is Chromium's own window and has the keyboard: Ctrl+Q still quits.
    let app = launch(&format!("{base}/alpha"));
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    hid(&app, key::F12, &[]);
    until(&app, "DevTools", 15, |a| os::windows(a).len() == 2);
    let devtools = ("dev.formalsnake.Lynk".to_owned(), "Lynk DevTools".to_owned());
    until(&app, "DevTools named after the app", 5, |a| os::window_names(a).contains(&devtools));
    quit_by(&app, Quit::Key);

    let profile = profile("quit-two-windows");
    let app = launch_on(&profile, &[format!("{base}/alpha")]);
    until(&app, "the page", 30, |a| titles(a) == ["Alpha"]);
    press(&app, "cmd-n");
    until(&app, "a second window", 10, |a| window_count(a) == 2);
    quit_by(&app, Quit::Key);
    assert_eq!(session_of(&profile)["windows"].as_array().map(Vec::len), Some(2), "both windows saved");
}
