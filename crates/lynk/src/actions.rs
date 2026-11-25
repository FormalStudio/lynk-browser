//! Every command the menu bar, the keyboard and (later) the command bar share, as one ordered
//! table. A row names its action, label, chord and handler; the menu bar is built from the
//! rows in table order, so an item shows its chord and the two never disagree. A Chrome-style
//! page passes a chord the main menu declares to the menu before it takes it itself, so every
//! chord that must work over a page is a menu item.
//!
//! A module adds its command as a row in [`COMMANDS`], at the place its menu item goes, and
//! menu items that are not commands (the Tabs menu's tab list) as a row in [`MENU_ITEMS`].
//! Rows whose `handler` is `None` belong to a feature another module brings.

use lynk_core::keys;
use nd::core::WindowId;
use nd::platform::{Target, target};
use nd::prelude::*;

use nd::webview::WebView;

use crate::app;
use crate::shell;
use crate::tab::{Tab, TabId};
use crate::window::BrowserWindow;

/// The key context of a browser window's root.
pub const CONTEXT: &str = "Browser";

actions!(
    lynk,
    [
        NewTab,
        NewWindow,
        NewPrivateWindow,
        OpenAddress,
        OpenSwitcher,
        CloseTab,
        ReopenClosedTab,
        SavePage,
        OpenSettings,
        Quit,
        CopyAddress,
        FindInPage,
        FindNext,
        FindPrevious,
        Reload,
        Stop,
        ToggleFullScreen,
        ToggleSidebar,
        SwitchLayout,
        ShowDownloads,
        ShowBookmarks,
        BookmarkPage,
        ReadingMode,
        ToggleAdBlocking,
        HideElement,
        FloatVideo,
        ZoomIn,
        ZoomOut,
        ResetZoom,
        ViewSource,
        DevTools,
        NextTab,
        PreviousTab,
        TogglePin,
        ResetPinnedPage,
        ToggleMute,
        SleepTab,
        CloseOtherTabs,
        DuplicateTab,
        MoveTabLeft,
        MoveTabRight,
        MoveTabToNewWindow,
        Back,
        Forward,
        OpenExtensions,
        OpenWebStore,
        ShowHistory,
        ClearBrowsingData,
        Print,
        SelectLastTab,
        NewProfile,
        RenameProfile,
        DeleteProfile,
    ]
);

/// Brings a profile forward by id, from the foot's profile menu or the command bar.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = lynk, no_json)]
pub struct SwitchProfile(pub String);

/// The tab at this place in the window, Cmd+1 to Cmd+8.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = lynk, no_json)]
pub struct SelectTabAt(pub usize);

/// One tab by id, from the Tabs menu once a window has more than nine.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = lynk, no_json)]
pub struct SelectTab(pub TabId);

/// A command, named by its action, run on a tab that need not be the one shown: a tab row's
/// menu item.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = lynk, no_json)]
pub struct ForTab(pub TabId, pub &'static str);

/// Moves the shown tab, its live page included, to another window.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = lynk, no_json)]
pub struct MoveTabToWindow(pub WindowId);

/// Runs a command on a tab of the window: the shown one, or a row's.
pub type Handler = fn(&mut BrowserWindow, TabId, &mut Window, &mut Context<BrowserWindow>);
/// Reads a tab of the window: whether an item is enabled, or shows its other label.
pub type Probe = fn(&BrowserWindow, TabId, &App) -> bool;
type Listen = fn(Scaffold, Handler, &mut Context<BrowserWindow>) -> Scaffold;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuId {
    File,
    Edit,
    View,
    Tabs,
    Go,
    Extensions,
    History,
}

const MENU_ORDER: [(MenuId, &str); 7] = [
    (MenuId::File, "File"),
    (MenuId::Edit, "Edit"),
    (MenuId::View, "View"),
    (MenuId::Tabs, "Tabs"),
    (MenuId::Go, "Go"),
    (MenuId::Extensions, "Extensions"),
    (MenuId::History, "History"),
];

pub struct Command {
    pub label: &'static str,
    /// The label while the probe holds: "Show Sidebar" for "Hide Sidebar".
    pub flipped: Option<(&'static str, Probe)>,
    pub enabled: Option<Probe>,
    /// The id of its chord in `lynk_core::keys`.
    pub key: Option<&'static str>,
    /// Chords bound ahead of the core one, which the menu does not show (F12, Cmd+=).
    pub extra_keys: &'static [&'static str],
    pub menu: Option<MenuId>,
    pub separator_before: bool,
    /// Bound and listed on macOS too; false for the command macOS gives the Window menu.
    pub on_mac: bool,
    pub handler: Option<Handler>,
    action: fn() -> Box<dyn Action>,
    bind: fn(&'static str) -> KeyBinding,
    listen: Listen,
}

fn boxed<A: Action + Default>() -> Box<dyn Action> {
    Box::new(A::default())
}

fn bind<A: Action + Default>(keys: &'static str) -> KeyBinding {
    KeyBinding::new(keys, A::default(), Some(CONTEXT))
}

fn listen<A: Action>(root: Scaffold, handler: Handler, cx: &mut Context<BrowserWindow>) -> Scaffold {
    root.on_action(cx.listener(move |this, _: &A, window, cx| handler(this, this.active, window, cx)))
}

/// Close Tab from a key, the menu bar or the page means the shown tab and moves on while it
/// waits on its page; a row's Close Tab means that row, however often it is picked.
fn listen_close_shown(root: Scaffold, _: Handler, cx: &mut Context<BrowserWindow>) -> Scaffold {
    root.on_action(cx.listener(|this, _: &CloseTab, _, cx| this.close_shown(cx)))
}

impl Command {
    const fn new<A: Action + Default>(label: &'static str) -> Self {
        Self {
            label,
            flipped: None,
            enabled: None,
            key: None,
            extra_keys: &[],
            menu: None,
            separator_before: false,
            on_mac: true,
            handler: None,
            action: boxed::<A>,
            bind: bind::<A>,
            listen: listen::<A>,
        }
    }
    const fn listens(mut self, listen: Listen) -> Self {
        self.listen = listen;
        self
    }
    const fn key(mut self, id: &'static str) -> Self {
        self.key = Some(id);
        self
    }
    const fn extra_keys(mut self, keys: &'static [&'static str]) -> Self {
        self.extra_keys = keys;
        self
    }
    const fn menu(mut self, menu: MenuId) -> Self {
        self.menu = Some(menu);
        self
    }
    const fn after_separator(mut self) -> Self {
        self.separator_before = true;
        self
    }
    const fn flips(mut self, label: &'static str, when: Probe) -> Self {
        self.flipped = Some((label, when));
        self
    }
    const fn enabled(mut self, when: Probe) -> Self {
        self.enabled = Some(when);
        self
    }
    const fn not_on_mac(mut self) -> Self {
        self.on_mac = false;
        self
    }
    const fn handler(mut self, handler: Handler) -> Self {
        self.handler = Some(handler);
        self
    }

    pub fn action(&self) -> Box<dyn Action> {
        (self.action)()
    }

    fn here(&self) -> bool {
        self.on_mac || target() != Target::MacOs
    }

    /// The label as tab `id` of `win` makes it read.
    pub fn label_for(&self, win: &BrowserWindow, id: TabId, cx: &App) -> &'static str {
        match self.flipped {
            Some((label, when)) if when(win, id, cx) => label,
            _ => self.label,
        }
    }

    fn enabled_for(&self, win: &BrowserWindow, id: TabId, cx: &App) -> bool {
        self.enabled.is_none_or(|when| when(win, id, cx))
    }

    /// A module handles it and it is enabled for tab `id`.
    pub fn runs(&self, win: &BrowserWindow, id: TabId, cx: &App) -> bool {
        self.handler.is_some() && self.here() && self.enabled_for(win, id, cx)
    }
}

/// The row for action `A`.
pub fn command_for<A: Action + Default>() -> &'static Command {
    let name = A::default().name();
    COMMANDS.iter().find(|c| c.action().name() == name).expect("every action has a row")
}

/// `command` as a menu item acting on tab `id`, disabled while no module handles it.
pub fn item_for_tab(command: &Command, win: &BrowserWindow, id: TabId, cx: &App) -> MenuItem {
    let enabled = command.handler.is_some() && command.enabled_for(win, id, cx);
    MenuItem::action(command.label_for(win, id, cx), ForTab(id, command.action().name())).enabled(enabled)
}

/// Runs a [`ForTab`] item.
pub fn run_for_tab(win: &mut BrowserWindow, a: &ForTab, window: &mut Window, cx: &mut Context<BrowserWindow>) {
    let handler = COMMANDS.iter().find(|c| c.action().name() == a.1).and_then(|c| c.handler);
    if let Some(handler) = handler.filter(|_| win.tab(a.0).is_some()) {
        handler(win, a.0, window, cx);
    }
}

fn web_page(w: &BrowserWindow, id: TabId, _: &App) -> bool {
    w.tab(id).is_some_and(Tab::is_web_page)
}
fn has_address(w: &BrowserWindow, id: TabId, _: &App) -> bool {
    w.tab(id).is_some_and(|t| !t.url.is_empty())
}
fn view_of(w: &BrowserWindow, id: TabId) -> Option<&WebView> {
    w.tab(id).and_then(Tab::view)
}

/// Keys bound inside one part of the window, under that part's key context, off the menus.
pub const CONTEXT_KEYS: &[fn() -> KeyBinding] = &[crate::find::close_key];

pub const COMMANDS: &[Command] = &[
    Command::new::<NewTab>("New Tab").key("new-tab").menu(MenuId::File).handler(crate::omnibox::new_tab),
    Command::new::<NewWindow>("New Window").key("new-window").menu(MenuId::File).handler(|w, _, _, cx| {
        app::open_window_in(cx, w.profile.clone(), Vec::new(), None);
    }),
    Command::new::<NewPrivateWindow>("New Private Window").key("private").menu(MenuId::File),
    Command::new::<OpenAddress>("Open Address Bar")
        .key("address")
        .menu(MenuId::File)
        .handler(crate::omnibox::open_address),
    Command::new::<OpenSwitcher>("Switch Tabs").key("switcher").menu(MenuId::File).handler(crate::omnibox::switcher),
    Command::new::<CloseTab>("Close Tab")
        .key("close-tab")
        .menu(MenuId::File)
        .handler(|w, id, _, cx| w.close_tab(id, cx))
        .listens(listen_close_shown),
    Command::new::<ReopenClosedTab>("Reopen Closed Tab")
        .key("reopen-tab")
        .menu(MenuId::File)
        .handler(|w, _, _, cx| w.reopen(cx)),
    Command::new::<SavePage>("Save Page As\u{2026}").key("save-page").menu(MenuId::File).enabled(web_page),
    Command::new::<OpenSettings>("Settings").key("settings").menu(MenuId::File).after_separator(),
    // macOS's app menu has its own Quit.
    Command::new::<Quit>("Quit").key("quit").menu(MenuId::File).not_on_mac().handler(|_, _, _, cx| cx.quit()),
    Command::new::<CopyAddress>("Copy Address")
        .key("copy-address")
        .menu(MenuId::Edit)
        .after_separator()
        .enabled(has_address)
        .handler(shell::copy_address),
    Command::new::<FindInPage>("Find in Page").key("find").menu(MenuId::Edit).handler(crate::find::open),
    Command::new::<FindNext>("Find Next").key("find-next").menu(MenuId::Edit).handler(crate::find::next),
    Command::new::<FindPrevious>("Find Previous")
        .key("find-previous")
        .menu(MenuId::Edit)
        .handler(crate::find::previous),
    Command::new::<Reload>("Reload").key("reload").menu(MenuId::View).handler(shell::reload),
    Command::new::<Stop>("Stop")
        .key("stop")
        .menu(MenuId::View)
        .enabled(|w, id, _| w.tab(id).is_some_and(|t| t.rt.loading))
        .handler(|w, id, _, _| {
            if let Some(page) = view_of(w, id) {
                page.stop();
            }
        }),
    // macOS gives fullscreen to the Window menu's own item, Ctrl+Cmd+F.
    Command::new::<ToggleFullScreen>("Full Screen")
        .key("fullscreen")
        .menu(MenuId::View)
        .not_on_mac()
        .flips("Exit Full Screen", |w, _, _| w.browser_fullscreen)
        .handler(shell::toggle_fullscreen),
    Command::new::<ToggleSidebar>("Hide Sidebar")
        .key("toggle-sidebar")
        .menu(MenuId::View)
        .flips("Show Sidebar", |w, _, _| w.sidebar_hidden)
        .handler(shell::toggle_sidebar),
    Command::new::<SwitchLayout>("Use Compact Layout").key("layout").menu(MenuId::View),
    Command::new::<ShowDownloads>("Downloads").key("downloads").menu(MenuId::View).after_separator(),
    Command::new::<ShowBookmarks>("Bookmarks").key("bookmarks").menu(MenuId::View),
    Command::new::<BookmarkPage>("Bookmark This Page").key("bookmark-page").menu(MenuId::View).enabled(web_page),
    Command::new::<ReadingMode>("Reading Mode").key("reader").menu(MenuId::View),
    Command::new::<ToggleAdBlocking>("Allow Ads on This Site")
        .menu(MenuId::View)
        .flips("Block Ads on This Site", crate::blocking::off_here)
        .enabled(web_page)
        .handler(crate::blocking::toggle),
    Command::new::<HideElement>("Hide Element").key("hide-element").menu(MenuId::View).enabled(web_page),
    Command::new::<FloatVideo>("Float Video").key("float").menu(MenuId::View),
    Command::new::<ZoomIn>("Zoom In")
        .key("zoom-in")
        .extra_keys(&["secondary-="])
        .menu(MenuId::View)
        .after_separator()
        .handler(crate::zoom::zoom_in),
    Command::new::<ZoomOut>("Zoom Out").key("zoom-out").menu(MenuId::View).handler(crate::zoom::zoom_out),
    Command::new::<ResetZoom>("Reset Zoom").key("zoom-reset").menu(MenuId::View).handler(crate::zoom::reset),
    Command::new::<ViewSource>("View Page Source")
        .key("view-source")
        .menu(MenuId::View)
        .after_separator()
        .enabled(web_page)
        .handler(shell::view_source),
    Command::new::<DevTools>("Developer Tools")
        .key("devtools")
        .extra_keys(&["f12"])
        .menu(MenuId::View)
        .enabled(has_address)
        .handler(crate::devtools::toggle),
    Command::new::<NextTab>("Next Tab").key("next-tab").menu(MenuId::Tabs).handler(|w, _, _, cx| w.cycle(1, cx)),
    Command::new::<PreviousTab>("Previous Tab")
        .key("prev-tab")
        .menu(MenuId::Tabs)
        .handler(|w, _, _, cx| w.cycle(-1, cx)),
    Command::new::<TogglePin>("Pin Tab")
        .key("pin-tab")
        .menu(MenuId::Tabs)
        .flips("Unpin Tab", |w, id, _| w.is_pinned(id))
        .handler(|w, id, _, cx| w.set_pinned(id, !w.is_pinned(id), cx)),
    Command::new::<ResetPinnedPage>("Reset to Pinned Page")
        .menu(MenuId::Tabs)
        .enabled(|w, id, _| w.can_reset_pin(id))
        .handler(|w, id, _, cx| w.reset_pin(id, cx)),
    Command::new::<ToggleMute>("Mute Tab")
        .flips("Unmute Tab", |w, id, _| w.tab(id).is_some_and(|t| t.muted))
        .enabled(has_address)
        .handler(|w, id, _, cx| {
            let muted = w.tab(id).is_some_and(|t| t.muted);
            w.set_muted(id, !muted, cx);
        }),
    Command::new::<SleepTab>("Put Tab to Sleep")
        .menu(MenuId::Tabs)
        .enabled(|w, id, _| w.can_sleep(id))
        .handler(|w, id, _, cx| w.sleep(id, cx)),
    Command::new::<CloseOtherTabs>("Close Other Tabs").handler(|w, id, _, cx| w.close_others(id, cx)),
    Command::new::<DuplicateTab>("Duplicate Tab").enabled(has_address).handler(|w, id, _, cx| w.duplicate(id, cx)),
    Command::new::<MoveTabLeft>("Move Tab Left")
        .enabled(|w, id, _| w.order().first().is_some_and(|t| *t != id))
        .handler(|w, id, _, cx| w.move_by(id, -1, cx)),
    Command::new::<MoveTabRight>("Move Tab Right")
        .enabled(|w, id, _| w.order().last().is_some_and(|t| *t != id))
        .handler(|w, id, _, cx| w.move_by(id, 1, cx)),
    Command::new::<MoveTabToNewWindow>("Move Tab to New Window")
        .enabled(|w, _, _| w.tab_count() > 1)
        .handler(|w, id, _, cx| shell::move_tab(w, id, None, cx)),
    Command::new::<Back>("Back")
        .key("back")
        .menu(MenuId::Go)
        .enabled(|w, id, _| w.tab(id).is_some_and(|t| t.rt.can_go_back))
        .handler(|w, id, _, _| {
            if let Some(page) = view_of(w, id) {
                page.go_back();
            }
        }),
    Command::new::<Forward>("Forward")
        .key("forward")
        .menu(MenuId::Go)
        .enabled(|w, id, _| w.tab(id).is_some_and(|t| t.rt.can_go_forward))
        .handler(|w, id, _, _| {
            if let Some(page) = view_of(w, id) {
                page.go_forward();
            }
        }),
    Command::new::<OpenExtensions>("Extensions").menu(MenuId::Extensions),
    Command::new::<OpenWebStore>("Chrome Web Store").menu(MenuId::Extensions),
    Command::new::<ShowHistory>("Show All History").key("history").menu(MenuId::History),
    Command::new::<ClearBrowsingData>("Clear Browsing Data\u{2026}").key("clear-data").menu(MenuId::History),
    Command::new::<Print>("Print\u{2026}"),
    Command::new::<NewProfile>("New Profile\u{2026}").handler(crate::profile::new_profile),
    Command::new::<RenameProfile>("Rename Profile\u{2026}").handler(crate::profile::rename_profile),
    Command::new::<DeleteProfile>("Delete Profile\u{2026}")
        .enabled(|w, _, _| w.profile != lynk_core::profiles::DEFAULT_ID)
        .handler(crate::profile::delete_profile),
    Command::new::<SelectLastTab>("Select Last Tab").handler(|w, _, _, cx| {
        if let Some(last) = w.tab_count().checked_sub(1) {
            w.select_at(last, cx);
        }
    }),
];

/// Menu items that are not commands, appended to their menu after its commands.
pub type Items = fn(&BrowserWindow, &App) -> Vec<MenuItem>;

pub const MENU_ITEMS: &[(MenuId, Items)] = &[(MenuId::Tabs, tab_items)];

/// `primary+shift+bracketright` as a key binding spells it: `secondary-shift-]`.
fn binding_keys(chord: &str) -> String {
    chord
        .split('+')
        .map(|part| match part {
            "primary" => "secondary",
            "comma" => ",",
            "plus" => "+",
            "minus" => "-",
            "bracketleft" => "[",
            "bracketright" => "]",
            "period" => ".",
            "F11" => "f11",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// The key bindings, built once at startup. Where an action has two chords the menu shows the
/// last one, so the core chord goes last.
pub fn bindings() -> Vec<KeyBinding> {
    let mut bindings = Vec::new();
    for command in COMMANDS.iter().filter(|c| c.here()) {
        for keys in command.extra_keys {
            bindings.push((command.bind)(keys));
        }
        if let Some(chord) = command.key.and_then(|id| keys::chord(id, target() == Target::MacOs)) {
            // Bindings live for the whole run; the spelling is made once at startup.
            bindings.push((command.bind)(binding_keys(chord).leak()));
        }
    }
    const TAB_KEYS: [&str; 8] = [
        "secondary-1",
        "secondary-2",
        "secondary-3",
        "secondary-4",
        "secondary-5",
        "secondary-6",
        "secondary-7",
        "secondary-8",
    ];
    for (n, keys) in TAB_KEYS.into_iter().enumerate() {
        bindings.push(KeyBinding::new(keys, SelectTabAt(n), Some(CONTEXT)));
    }
    bindings.push(KeyBinding::new("secondary-9", SelectLastTab, Some(CONTEXT)));
    bindings.extend(CONTEXT_KEYS.iter().map(|key| key()));
    bindings
}

/// The root's listeners for every command with a handler. A command macOS keeps off its menus
/// still listens, for the page's Chrome command that dispatches it.
pub fn listen_all(mut root: Scaffold, cx: &mut Context<BrowserWindow>) -> Scaffold {
    for command in COMMANDS {
        if let Some(handler) = command.handler {
            root = (command.listen)(root, handler, cx);
        }
    }
    root
}

/// The Tabs menu's item for the tab at `index` of `count`: Chrome's chords, the first eight by
/// number and 9 for the last.
fn tab_item(index: usize, count: usize, id: TabId, label: String) -> MenuItem {
    if index + 1 == count && index >= 8 {
        MenuItem::action(label, SelectLastTab)
    } else if index < 8 {
        MenuItem::action(label, SelectTabAt(index))
    } else {
        MenuItem::action(label, SelectTab(id))
    }
}

fn tab_items(win: &BrowserWindow, _: &App) -> Vec<MenuItem> {
    let tabs = win.tabs();
    let count = tabs.len();
    let mut items = vec![MenuItem::separator()];
    items.extend(tabs.iter().enumerate().map(|(i, t)| tab_item(i, count, t.id, t.label())));
    items
}

/// `command` as the menu bar shows it, for the shown tab of `win`.
pub fn menu_item(command: &Command, win: &BrowserWindow, cx: &App) -> MenuItem {
    let item = MenuItem::Action {
        label: command.label_for(win, win.active, cx).into(),
        action: command.action(),
        icon: None,
        destructive: false,
        enabled: true,
        checked: false,
    };
    item.enabled(command.enabled_for(win, win.active, cx))
}

/// The menu bar for the window it acts on.
pub fn menus(win: &BrowserWindow, cx: &App) -> Vec<Menu> {
    let mut bar = vec![Menu::app([])];
    for (id, title) in MENU_ORDER {
        let mut items = Vec::new();
        if id == MenuId::Edit {
            // Roles rather than `Menu::edit`, whose Find submenu would take Cmd+F from Find in
            // Page.
            items.extend([
                MenuItem::role(MenuRole::Undo),
                MenuItem::role(MenuRole::Redo),
                MenuItem::separator(),
                MenuItem::role(MenuRole::Cut),
                MenuItem::role(MenuRole::Copy),
                MenuItem::role(MenuRole::Paste),
                MenuItem::role(MenuRole::SelectAll),
            ]);
        }
        for command in COMMANDS.iter().filter(|c| c.menu == Some(id) && c.here()) {
            if command.separator_before && !items.is_empty() {
                items.push(MenuItem::separator());
            }
            items.push(menu_item(command, win, cx));
        }
        for (_, extra) in MENU_ITEMS.iter().filter(|(menu, _)| *menu == id) {
            items.extend(extra(win, cx));
        }
        bar.push(Menu::new(title).items(items));
    }
    bar.push(Menu::window([]));
    bar
}

/// What the menu bar shows, to tell whether it changed: every item's label and whether it is
/// enabled.
pub fn signature(menus: &[Menu]) -> Vec<(SharedString, bool)> {
    fn walk(items: &[MenuItem], out: &mut Vec<(SharedString, bool)>) {
        for item in items {
            match item {
                MenuItem::Action { label, enabled, .. } => out.push((label.clone(), *enabled)),
                MenuItem::Submenu(menu) => walk(&menu.items, out),
                MenuItem::Separator => out.push(("-".into(), true)),
                MenuItem::Role(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    for menu in menus {
        out.push((menu.title.clone(), true));
        walk(&menu.items, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ninth_chord_is_the_last_tab_once_there_are_nine() {
        let is = |i: usize, expected: &dyn Action| match tab_item(i, 10, i as TabId + 1, String::new()) {
            MenuItem::Action { action, .. } => action.partial_eq(expected),
            _ => false,
        };
        assert!(is(0, &SelectTabAt(0)));
        assert!(is(7, &SelectTabAt(7)));
        assert!(is(8, &SelectTab(9)));
        assert!(is(9, &SelectLastTab));
    }

    #[test]
    fn the_file_menu_is_the_checklists() {
        let file: Vec<&str> = COMMANDS.iter().filter(|c| c.menu == Some(MenuId::File)).map(|c| c.label).collect();
        assert_eq!(
            file,
            [
                "New Tab",
                "New Window",
                "New Private Window",
                "Open Address Bar",
                "Switch Tabs",
                "Close Tab",
                "Reopen Closed Tab",
                "Save Page As\u{2026}",
                "Settings",
                "Quit"
            ]
        );
    }

    #[test]
    fn every_core_chord_has_a_command() {
        for id in keys::ids() {
            assert!(COMMANDS.iter().any(|c| c.key == Some(id)), "{id}");
        }
    }

    #[test]
    fn core_chords_spell_as_key_bindings() {
        assert_eq!(binding_keys("primary+shift+bracketright"), "secondary-shift-]");
        assert_eq!(binding_keys("primary+plus"), "secondary-+");
        assert_eq!(binding_keys("primary+comma"), "secondary-,");
        assert_eq!(binding_keys("F11"), "f11");
    }
}
