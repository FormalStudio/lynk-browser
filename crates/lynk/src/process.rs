//! The process around the windows: one running instance per profile, which a later launch
//! hands its URLs to, and SIGTERM quitting the way the Quit command does, so Chromium flushes
//! the profile instead of being cut off mid-write.

use std::sync::OnceLock;

use nd::prelude::*;
use nd_ipc::instance::{Instance, Launch, Primary};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use crate::app;
use crate::window::Open;

/// What the process learns off the main thread.
enum Event {
    Launch(Launch),
    Terminate,
}

/// Runs `future` to its end on the app's runtime, blocking the caller.
pub(crate) fn block_on<F: std::future::Future>(future: F) -> F::Output {
    runtime().block_on(future)
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("lynk-rt")
            .enable_all()
            .build()
            .expect("a tokio runtime")
    })
}

/// Takes the profile's instance lock and serves later launches and SIGTERM, or hands `launch` to
/// the instance holding the lock and quits. `false` when this launch quits. The lock lives in the
/// app data dir, which is the profile.
///
/// Runs inside `Application::run`: nd-cef claims fds 3 and 4 for Chromium's pipe as the run
/// starts, which would take them from a runtime opened earlier. Chromium itself starts with the
/// first page, after this.
pub fn start(cx: &mut App, launch: &Launch) -> bool {
    let endpoint = nd_ipc::Endpoint::for_app("lynk", "instance");
    let primary = match runtime().block_on(nd_ipc::instance::acquire(&endpoint, launch)) {
        Ok(Instance::Primary(primary)) => Some(primary),
        Ok(Instance::Forwarded) => {
            cx.quit();
            return false;
        }
        Err(error) => {
            eprintln!("lynk: single instance: {error}");
            None
        }
    };
    serve(primary);
    true
}

fn serve(primary: Option<Primary>) {
    let (tx, rx) = unbounded_channel();
    let rt = runtime();
    if let Some(mut primary) = primary {
        let tx = tx.clone();
        rt.spawn(async move {
            while let Some(launch) = primary.next_launch().await {
                if tx.send(Event::Launch(launch)).is_err() {
                    break;
                }
            }
        });
    }
    // Made here rather than in the task: nd-cef puts back the signal handlers it found when
    // Chromium starts, which would drop one a worker thread installed in the meantime.
    let terminations = {
        let _rt = rt.enter();
        terminations()
    };
    match terminations {
        Ok(mut term) => {
            rt.spawn(async move {
                while term.recv().await.is_some() {
                    if tx.send(Event::Terminate).is_err() {
                        break;
                    }
                }
            });
        }
        Err(error) => eprintln!("lynk: termination signal: {error}"),
    }
    next(rx);
}

/// SIGTERM, which a service manager or `kill` sends.
#[cfg(unix)]
fn terminations() -> std::io::Result<tokio::signal::unix::Signal> {
    use tokio::signal::unix::{SignalKind, signal};
    signal(SignalKind::terminate())
}

/// The console closing, Windows's nearest to SIGTERM.
#[cfg(windows)]
fn terminations() -> std::io::Result<tokio::signal::windows::CtrlClose> {
    tokio::signal::windows::ctrl_close()
}

fn next(mut rx: UnboundedReceiver<Event>) {
    nd::system::spawn(
        async move {
            let event = rx.recv().await;
            (event, rx)
        },
        |(event, rx), cx| {
            let Some(event) = event else { return };
            handle(event, cx);
            next(rx);
        },
    );
}

fn handle(event: Event, cx: &mut App) {
    match event {
        Event::Terminate => cx.quit(),
        Event::Launch(launch) => open_launch(cx, launch.urls),
    }
}

/// A later launch's URLs, as tabs at the end of the window the menu bar acts on, else the newest
/// open one. A window waiting to close, or whose every tab is, would take them down with it, so
/// with no other window they get one of their own.
pub fn open_launch(cx: &mut App, urls: Vec<String>) {
    let open: Vec<_> = cx.window_ids().collect();
    let target = app::key_window(cx)
        .into_iter()
        .chain(app::windows(cx).into_iter().rev().map(|(id, _)| id))
        .filter(|id| open.contains(id))
        .find_map(|id| app::window(cx, id).filter(|w| !w.read(cx).pending_close()).map(|w| (id, w)));
    let Some((id, win)) = target else {
        let tabs = urls.into_iter().map(|url| crate::tab::Tab::new(app::next_tab_id(cx), url)).collect();
        app::open_window(cx, tabs, None);
        return;
    };
    win.update(cx, |win, cx| {
        for url in urls {
            win.open_tab(url, Open::End, cx);
        }
    });
    cx.activate_window(id);
}
