//! Profiles in the running app: the list in core's `profiles.json`, and switching, creating,
//! renaming and deleting them from the sidebar foot's profile glyph and the command bar. A
//! window's profile is `BrowserWindow::profile`, fixed as it opens. This module opens windows
//! through `session` and `app` only, and keeps no tab state; its name popover's state is its
//! own, per window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lynk_core::profiles::{self, DEFAULT_ID, ProfileError, ProfilesState, menu_name};
use nd::core::WindowId;
use nd::prelude::*;
use nd::settings::Store;
use nd::webview::Profile as Engine;

use crate::actions::{DeleteProfile, NewProfile, RenameProfile, SwitchProfile};
use crate::app;
use crate::tab::TabId;
use crate::window::BrowserWindow;

struct Profiles {
    root: PathBuf,
    store: Store<ProfilesState>,
}

/// What the name popover is doing in a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Naming {
    New,
    Rename,
}

#[derive(Clone)]
struct Edit {
    naming: Naming,
    text: String,
    error: Option<&'static str>,
}

#[derive(Default)]
struct Edits(HashMap<WindowId, Edit>);

const FIELD: &str = "profile-name";
const POPOVER_WIDTH: f32 = 260.;

/// Reads `profiles.json` under `root`, first removing what deleted profiles left behind, before
/// Chromium starts and opens any of it.
pub fn install(cx: &mut App, root: &Path) {
    let store = profiles::open(root);
    let left = store.get().leftovers.clone();
    if !left.is_empty() {
        let kept: Vec<PathBuf> =
            left.into_iter().filter(|d| std::fs::remove_dir_all(d).is_err() && d.exists()).collect();
        store.update(|s| s.leftovers = kept);
    }
    cx.set_global(Profiles { root: root.to_path_buf(), store });
    // The profiles with windows as the app quits are the ones the next launch opens.
    cx.on_quit(|cx| {
        let open: Vec<String> = app::windows(cx).iter().map(|(_, w)| w.read(cx).profile.clone()).collect();
        let Some(p) = cx.try_global::<Profiles>() else { return };
        p.store.update(|s| {
            for profile in &mut s.profiles {
                profile.open = open.contains(&profile.id);
            }
        });
        if let Err(e) = p.store.flush() {
            eprintln!("lynk: profiles.json was not written as the app quit: {e}");
        }
    });
}

/// The list; before `install` (unit tests) only `Default`.
fn state(cx: &App) -> std::sync::Arc<ProfilesState> {
    cx.try_global::<Profiles>().map_or_else(Default::default, |p| p.store.get())
}

/// Every profile's id, `Default` first.
pub fn ids(cx: &App) -> Vec<String> {
    state(cx).profiles.iter().map(|p| p.id.clone()).collect()
}

/// The profiles a launch opens.
pub fn open_ids(cx: &App) -> Vec<String> {
    state(cx).open_ids().into_iter().map(str::to_owned).collect()
}

pub fn name(cx: &App, id: &str) -> String {
    state(cx).get(id).map(|p| p.name.clone()).unwrap_or_default()
}

/// The profiles other than `id`, `(id, name)`, for the command bar.
pub fn others(cx: &App, id: &str) -> Vec<(String, String)> {
    state(cx).profiles.iter().filter(|p| p.id != id).map(|p| (p.id.clone(), p.name.clone())).collect()
}

/// Where `id` keeps its session and history, made if it is not there yet.
pub fn data_dir(cx: &App, id: &str) -> PathBuf {
    let root = cx.try_global::<Profiles>().map_or_else(|| nd::system::app_dirs().data(), |p| p.root.clone());
    let dir = profiles::data_dir(&root, id);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("lynk: profile directory {}: {e}", dir.display());
    }
    dir
}

/// The Chromium store `id`'s pages are made in.
pub fn engine(id: &str) -> Engine {
    profiles::chromium_name(id).map_or(Engine::Default, |name| Engine::Named(name.to_owned()))
}

fn set_open(cx: &App, id: &str, open: bool) {
    if let Some(p) = cx.try_global::<Profiles>() {
        p.store.update(|s| s.set_open(id, open));
    }
}

/// The last window of `id` went while others stay.
pub fn closed(cx: &App, id: &str) {
    set_open(cx, id, false);
}

/// Brings `id` forward: its front window, else the windows its session kept, else a new one.
pub fn switch_to(cx: &mut App, id: &str) {
    if let Some((front, _)) = app::windows_of(cx, id).pop() {
        cx.activate_window(front);
        return;
    }
    if state(cx).get(id).is_none() {
        return;
    }
    set_open(cx, id, true);
    let dir = data_dir(cx, id);
    let restored = crate::session::open(cx, id, &dir);
    if restored.is_empty() {
        app::open_window_in(cx, id.to_owned(), Vec::new(), None);
    }
    for window in restored {
        app::open_restored(cx, window);
    }
    crate::session::changed(cx);
}

/// The foot's profile glyph and its menu: every profile, the window's own checked, then New,
/// Rename and Delete.
pub fn glyph(win: &BrowserWindow, _: &mut Window, cx: &mut Context<BrowserWindow>) -> Option<AnyElement> {
    let me = win.id;
    let mine = win.profile.clone();
    let tip = name(cx, &mine);
    let button = menu_button("profile-menu", Icon::User, move |menu, _, cx| {
        let list = state(cx);
        let mut menu = menu;
        for p in &list.profiles {
            let item = MenuItem::action(menu_name(&p.name), SwitchProfile(p.id.clone())).checked(p.id == mine);
            menu = menu.entry(item);
        }
        menu.separator()
            .entry(MenuItem::action("New Profile\u{2026}", NewProfile))
            .entry(MenuItem::action("Rename Profile\u{2026}", RenameProfile))
            .entry(MenuItem::action("Delete Profile\u{2026}", DeleteProfile).enabled(mine != DEFAULT_ID))
            // The glyph sits in the window's foot: its menu opens upward, over the column.
            .above()
    })
    .tooltip(format!("Profile: {tip}"));
    let mut anchor = div().id("profile-anchor").child(button);
    if let Some(edit) = cx.try_global::<Edits>().and_then(|e| e.0.get(&me)).cloned() {
        anchor = anchor.child(name_popover(&edit, cx));
    }
    Some(anchor.into_any_element())
}

fn name_popover(edit: &Edit, cx: &mut Context<BrowserWindow>) -> AnyElement {
    let t = cx.theme();
    let (gap, pad) = (t.spacing.sm, t.spacing.md);
    let (title, action) = match edit.naming {
        Naming::New => ("New Profile", "Create"),
        Naming::Rename => ("Rename Profile", "Rename"),
    };
    let error = edit.error.map(|e| label(e).id("profile-name-error").secondary().into_any_element());
    popover("profile-popover")
        .placement(Placement::Trailing)
        .on_dismiss(cx.listener(|win, _: &Dismissed, _, cx| {
            stop(cx, win.id);
            cx.notify();
        }))
        .child(
            v_flex()
                .w(px(POPOVER_WIDTH))
                .gap(gap)
                .p(pad)
                .child(label(title).font_weight(FontWeight::SEMIBOLD))
                .child(
                    text_field(FIELD)
                        .label("Profile name")
                        .placeholder("Name")
                        .text(edit.text.clone())
                        .on_change(cx.listener(|win, e: &TextChanged, _, cx| {
                            if let Some(edit) = editing(cx, win.id) {
                                edit.text = e.text.to_string();
                                edit.error = None;
                            }
                            cx.notify();
                        }))
                        .on_submit(cx.listener(|win, _: &TextSubmitted, _, cx| commit(win, cx))),
                )
                .children(error)
                .child(
                    h_flex()
                        .gap(gap)
                        .justify_end()
                        .child(button("profile-cancel", "Cancel").on_click(cx.listener(
                            |win, _: &ClickEvent, _, cx| {
                                stop(cx, win.id);
                                cx.notify();
                            },
                        )))
                        .child(
                            button("profile-save", action)
                                .on_click(cx.listener(|win, _: &ClickEvent, _, cx| commit(win, cx))),
                        ),
                ),
        )
        .into_any_element()
}

fn editing(cx: &mut App, win: WindowId) -> Option<&mut Edit> {
    cx.try_global_mut::<Edits>()?.0.get_mut(&win)
}

fn stop(cx: &mut App, win: WindowId) {
    if let Some(edits) = cx.try_global_mut::<Edits>() {
        edits.0.remove(&win);
    }
}

/// A window listener: its name popover goes with it.
pub fn forget(cx: &mut App, win: WindowId) {
    stop(cx, win);
}

fn start(win: &BrowserWindow, naming: Naming, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    let text = if naming == Naming::Rename { name(cx, &win.profile) } else { String::new() };
    if !cx.has_global::<Edits>() {
        cx.set_global(Edits::default());
    }
    cx.global_mut::<Edits>().0.insert(win.id, Edit { naming, text, error: None });
    window.focus(FIELD);
    cx.notify();
}

fn commit(win: &mut BrowserWindow, cx: &mut Context<BrowserWindow>) {
    let Some(edit) = cx.try_global::<Edits>().and_then(|e| e.0.get(&win.id)) else { return };
    let (naming, text) = (edit.naming, edit.text.clone());
    let profile = win.profile.clone();
    let store = &cx.global::<Profiles>().store;
    let mut made: Result<Option<String>, ProfileError> = Ok(None);
    store.update(|s| {
        made = match naming {
            Naming::New => s.create(&text).map(Some),
            Naming::Rename => s.rename(&profile, &text).map(|()| None),
        };
    });
    match made {
        Err(e) => {
            if let Some(edit) = editing(cx, win.id) {
                edit.error = Some(e.message());
            }
            cx.notify();
        }
        Ok(new) => {
            stop(cx, win.id);
            win.changed(cx);
            app::refresh_menus_for(win, cx);
            if let Some(id) = new {
                let window = app::open_window_in(cx, id.clone(), Vec::new(), None);
                set_open(cx, &id, true);
                let dir = data_dir(cx, &id);
                crate::session::open(cx, &id, &dir);
                cx.activate_window(window);
            }
        }
    }
}

/// New Profile from the menu or the command bar.
pub fn new_profile(win: &mut BrowserWindow, _: TabId, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    start(win, Naming::New, window, cx);
}

pub fn rename_profile(win: &mut BrowserWindow, _: TabId, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    start(win, Naming::Rename, window, cx);
}

/// Delete Profile asks first, naming what goes, then closes the profile's windows without
/// asking their pages and removes its files. Chromium keeps its store open while the app runs,
/// so that directory goes at the next launch.
pub fn delete_profile(win: &mut BrowserWindow, _: TabId, _: &mut Window, cx: &mut Context<BrowserWindow>) {
    let id = win.profile.clone();
    if id == DEFAULT_ID {
        return;
    }
    let who = name(cx, &id);
    // Cancel comes first: it is what Return, Esc and an alert that never answers all give.
    let alert = Alert::new(format!("Delete the profile \u{201c}{who}\u{201d}?"))
        .style(AlertStyle::Warning)
        .message(
            "Its windows close, and its tabs, pins, history, cookies, saved passwords, site settings and \
             extensions are deleted from this computer. This cannot be undone.",
        )
        .cancel("Cancel")
        .destructive("Delete Profile");
    let answer = nd::system::window::alert(cx, win.id, alert);
    nd::system::spawn(answer, move |answer, cx| {
        if answer.0 == 1 {
            delete(cx, &id);
        }
    });
}

fn delete(cx: &mut App, id: &str) {
    let root = cx.global::<Profiles>().root.clone();
    let mut gone = Err(ProfileError::UnknownProfile);
    cx.global::<Profiles>().store.update(|s| gone = s.delete(&root, id));
    let Ok(mut dirs) = gone else { return };
    crate::session::forget(cx, id);
    let windows = app::windows_of(cx, id);
    if windows.len() == app::windows(cx).len() {
        switch_to(cx, DEFAULT_ID);
    }
    for (_, w) in windows {
        w.update(cx, |w, cx| w.discard(cx));
    }
    app::forget_profile(cx, id);
    // The Chromium store stays open in this run; it goes before the next launch starts Chromium.
    let chromium = dirs.split_off(1);
    let kept = dirs.into_iter().filter(|d| std::fs::remove_dir_all(d).is_err() && d.exists());
    let later: Vec<PathBuf> = kept.chain(chromium).collect();
    cx.global::<Profiles>().store.update(|s| s.leftovers.extend(later));
    app::refresh_menus(cx);
}

/// A profile picked from the foot's menu or the command bar. Switching opens windows, which
/// cannot happen while this window is being updated.
pub fn switch(win: &mut BrowserWindow, a: &SwitchProfile, _: &mut Window, _: &mut Context<BrowserWindow>) {
    if a.0 != win.profile {
        let id = a.0.clone();
        nd::system::after(std::time::Duration::ZERO, move |cx| switch_to(cx, &id));
    }
}
