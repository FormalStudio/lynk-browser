//! Every row the settings page shows: what it is called, the one sentence under it, its
//! control and where it applies. Values live in [`LynkSettings`] under the row's id, so they
//! persist and default the way the rest of the settings do. Buttons have no value.
//!
//! `{cmd}` in a title or description is the primary modifier as a chord writes it ("⌘" or
//! "Ctrl+"), and `{cmd-click}` and `{alt-click}` name a modified click ("Command-click",
//! "Ctrl-click"); read them through [`Setting::title`] and [`Setting::description`].

use serde_json::Value;

use crate::settings::LynkSettings;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Platform {
    Mac,
    Linux,
    Windows,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

const ALL: &[Platform] = &[Platform::Mac, Platform::Linux, Platform::Windows];
const MAC: &[Platform] = &[Platform::Mac];
const NOT_MAC: &[Platform] = &[Platform::Linux, Platform::Windows];
const ARC: &[Platform] = &[Platform::Mac, Platform::Windows];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Section {
    General,
    Appearance,
    Tabs,
    SpacesAndPins,
    KeyboardAndMouse,
    Extensions,
}

pub const SECTIONS: [Section; 6] = [
    Section::General,
    Section::Appearance,
    Section::Tabs,
    Section::SpacesAndPins,
    Section::KeyboardAndMouse,
    Section::Extensions,
];

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Tabs => "Tabs",
            Self::SpacesAndPins => "Spaces and pins",
            Self::KeyboardAndMouse => "Keyboard and mouse",
            Self::Extensions => "Extensions",
        }
    }
}

/// `(stored value, label)`.
pub type Options = &'static [(&'static str, &'static str)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Toggle,
    Choice(Options),
    /// A choice drawn as pictures of each option.
    Thumbnails(Options),
    Path,
    Text,
    Shortcut,
    Button,
}

/// The row is disabled unless `id`'s value passes `when`, and then reads `reason` in place of
/// its description.
#[derive(Clone, Copy, Debug)]
pub struct Requires {
    pub id: &'static str,
    pub when: fn(&Value) -> bool,
    pub reason: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct Setting {
    /// The key in [`LynkSettings`]; for a button, the command it runs.
    pub id: &'static str,
    pub section: Section,
    title: &'static str,
    description: &'static str,
    pub control: Control,
    pub platforms: &'static [Platform],
    pub requires: Option<Requires>,
}

fn modifiers(text: &str, mac: bool) -> String {
    let (cmd, cmd_click, alt_click) =
        if mac { ("⌘", "Command-click", "Option-click") } else { ("Ctrl+", "Ctrl-click", "Alt-click") };
    text.replace("{cmd-click}", cmd_click).replace("{alt-click}", alt_click).replace("{cmd}", cmd)
}

impl Setting {
    pub fn title(&self, mac: bool) -> String {
        modifiers(self.title, mac)
    }

    pub fn description(&self, mac: bool) -> String {
        modifiers(self.description, mac)
    }

    pub fn stored(&self) -> bool {
        self.control != Control::Button
    }

    pub fn available(&self, platform: Platform) -> bool {
        self.platforms.contains(&platform)
    }

    /// The value a new profile starts with; `None` for a button.
    pub fn default_value(&self) -> Option<Value> {
        value(&LynkSettings::default(), self.id)
    }
}

const fn row(
    id: &'static str,
    section: Section,
    title: &'static str,
    description: &'static str,
    control: Control,
) -> Setting {
    Setting { id, section, title, description, control, platforms: ALL, requires: None }
}

const fn only(mut s: Setting, platforms: &'static [Platform]) -> Setting {
    s.platforms = platforms;
    s
}

const fn needs(mut s: Setting, id: &'static str, when: fn(&Value) -> bool, reason: &'static str) -> Setting {
    s.requires = Some(Requires { id, when, reason });
    s
}

fn is_true(v: &Value) -> bool {
    v == &Value::Bool(true)
}

fn not_light(v: &Value) -> bool {
    v.as_str() != Some("light")
}

fn not_never(v: &Value) -> bool {
    v.as_str() != Some("never")
}

fn not_off(v: &Value) -> bool {
    v.as_str() != Some("off")
}

fn non_empty(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.is_empty())
}

fn sidebar_layout(v: &Value) -> bool {
    matches!(v.as_str(), Some("sidebar" | "left" | "right"))
}

const SPACES_REASON: &str = "Only while spaces are on";

use Control::{Button, Path, Shortcut, Text, Toggle};
use Section::{Appearance, Extensions, General, KeyboardAndMouse, SpacesAndPins, Tabs};

pub static SETTINGS: &[Setting] = &[
    // General
    row("makeDefault", General, "Default browser", "Open links from other apps in Lynk.", Button),
    row(
        "importBrowser",
        General,
        "Import from another browser",
        "Bring over bookmarks, history and passwords.",
        Button,
    ),
    row(
        "searchEngine",
        General,
        "Search engine",
        "Where text that is not an address is searched.",
        Control::Choice(&[("duckduckgo", "DuckDuckGo"), ("google", "Google"), ("bing", "Bing")]),
    ),
    row("homepage", General, "Homepage", "The page new windows open. Leave it empty for the new tab page.", Text),
    row(
        "freshWindow",
        General,
        "Start fresh",
        "Open with your pins and a new tab instead of last time's tabs.",
        Toggle,
    ),
    row(
        "mailApp",
        General,
        "Email links open in",
        "The app that writes an email when you click an email address.",
        Path,
    ),
    row("syncFolder", General, "Sync through a folder", "Keep spaces, pins and settings in a folder you choose.", Path),
    row("confirmQuit", General, "Confirm before quitting", "Ask before closing every window at once.", Toggle),
    row(
        "downloadDir",
        General,
        "Save downloads to",
        "The folder downloads go to. Empty uses your Downloads folder.",
        Path,
    ),
    row("askWhereToSave", General, "Ask where to save each file", "Show a save panel for every download.", Toggle),
    row(
        "alwaysShowDownloads",
        General,
        "Always show the downloads button",
        "Keep the button in the sidebar even with nothing downloading.",
        Toggle,
    ),
    row(
        "downloadsIn",
        General,
        "Open downloads in",
        "Where the downloads list opens.",
        Control::Choice(&[("panel", "A panel"), ("tab", "A tab")]),
    ),
    // Appearance
    row(
        "appearance",
        Appearance,
        "Light and dark",
        "Pick a look, or follow the system.",
        Control::Thumbnails(&[("light", "Light"), ("dark", "Dark"), ("auto", "Auto")]),
    ),
    row(
        "layout",
        Appearance,
        "Tab layout",
        "Where your tabs sit around the page.",
        Control::Thumbnails(&[
            ("sidebar", "Sidebar"),
            ("compact", "Compact"),
            ("top", "Top"),
            ("left", "Left"),
            ("right", "Right"),
            ("iconsOnly", "Icons only"),
        ]),
    ),
    needs(
        row(
            "hideToIcons",
            Appearance,
            "Hiding the sidebar leaves its icons",
            "Hide Sidebar shrinks the column to a strip of icons.",
            Toggle,
        ),
        "layout",
        sidebar_layout,
        "Only with a sidebar layout",
    ),
    needs(
        row(
            "narrowToIcons",
            Appearance,
            "Narrow to icons",
            "Dragging the sidebar narrow turns it into a strip of icons.",
            Toggle,
        ),
        "layout",
        sidebar_layout,
        "Only with a sidebar layout",
    ),
    needs(
        row("darkenPages", Appearance, "Darken pages", "Give pages that have no dark look a dark one.", Toggle),
        "appearance",
        not_light,
        "Only while the appearance isn't Light",
    ),
    row(
        "devtoolsFollowAppearance",
        Appearance,
        "Developer tools follow light and dark",
        "Off keeps developer tools dark.",
        Toggle,
    ),
    row(
        "readerFollowsAppearance",
        Appearance,
        "Reading mode follows light and dark",
        "Off keeps reading mode light.",
        Toggle,
    ),
    // Tabs
    row(
        "newTabPosition",
        Tabs,
        "New tabs open",
        "Where a new tab goes in the list.",
        Control::Choice(&[("top", "At the top"), ("next", "Below the tab on show"), ("end", "At the end")]),
    ),
    row(
        "keepLastTab",
        Tabs,
        "Keep a tab when the last one closes",
        "Closing the last tab leaves a new tab instead of closing the window.",
        Toggle,
    ),
    row(
        "closeGoesBack",
        Tabs,
        "Closing a tab returns to the one before",
        "Show the tab you used last instead of the one below.",
        Toggle,
    ),
    row(
        "swipeBackCloses",
        Tabs,
        "Swiping back closes a link's tab",
        "On a tab a link opened, going back past its first page closes it.",
        Toggle,
    ),
    row(
        "undoReopensTab",
        Tabs,
        "{cmd}Z reopens a closed tab",
        "Undo outside a text field brings back the last tab you closed.",
        Toggle,
    ),
    row(
        "switchToOpened",
        Tabs,
        "Ctrl+Tab goes to a link opened behind",
        "The switcher starts on the tab you just opened in the background.",
        Toggle,
    ),
    row(
        "switcherSkipsPinned",
        Tabs,
        "Ctrl+Tab leaves out pinned tabs",
        "The switcher only cycles through tabs in the list.",
        Toggle,
    ),
    row(
        "loadWhenShown",
        Tabs,
        "Load background tabs when shown",
        "Tabs opened behind wait to load until you look at them.",
        Toggle,
    ),
    row(
        "popupsAsWindows",
        Tabs,
        "Sized pop-ups open as small windows",
        "A page that opens a window of a set size gets one.",
        Toggle,
    ),
    row(
        "sleepTabs",
        Tabs,
        "Sleep unused tabs",
        "Free memory from tabs you have not looked at for a while.",
        Control::Choice(&[("never", "Never"), ("later", "Later"), ("normal", "Normal"), ("sooner", "Sooner")]),
    ),
    needs(
        row("dimSleeping", Tabs, "Dim sleeping tabs", "Sleeping tabs read fainter in the sidebar.", Toggle),
        "sleepTabs",
        not_never,
        "Only while unused tabs sleep",
    ),
    row(
        "archiveAfter",
        Tabs,
        "Archive tabs",
        "Move tabs in the list you have not looked at to the archive. Pinned tabs and folders stay.",
        Control::Choice(&[
            ("never", "Never"),
            ("hours12", "After 12 hours"),
            ("day1", "After a day"),
            ("days7", "After 7 days"),
            ("days30", "After 30 days"),
        ]),
    ),
    // Spaces and pins
    row(
        "setUpLikeArc",
        SpacesAndPins,
        "Set up like Arc",
        "Turn on spaces, pins and the sidebar the way Arc has them.",
        Button,
    ),
    only(
        row("importArc", SpacesAndPins, "Import from Arc", "Bring over your Arc spaces and pinned tabs.", Button),
        ARC,
    ),
    row(
        "spacesEnabled",
        SpacesAndPins,
        "Spaces",
        "Keep separate sets of tabs and pins, each with its own colours.",
        Toggle,
    ),
    needs(
        row(
            "numberKeysSwitchSpaces",
            SpacesAndPins,
            "{cmd}1 to {cmd}9 switch spaces",
            "Number shortcuts go to a space instead of a tab.",
            Toggle,
        ),
        "spacesEnabled",
        is_true,
        SPACES_REASON,
    ),
    needs(
        row(
            "spaceButtons",
            SpacesAndPins,
            "A button for each space",
            "Show one button per space at the foot of the sidebar.",
            Toggle,
        ),
        "spacesEnabled",
        is_true,
        SPACES_REASON,
    ),
    needs(
        row(
            "siteRules",
            SpacesAndPins,
            "Send sites to a space",
            "Pick sites whose links always open in a given space.",
            Button,
        ),
        "spacesEnabled",
        is_true,
        SPACES_REASON,
    ),
    needs(
        row(
            "askSpaceForLinks",
            SpacesAndPins,
            "Ask where links from other apps open",
            "Choose a space for each link another app opens.",
            Toggle,
        ),
        "spacesEnabled",
        is_true,
        SPACES_REASON,
    ),
    row(
        "pinStyle",
        SpacesAndPins,
        "Pinned tiles show",
        "The site's icon, or only its first letter.",
        Control::Choice(&[("icons", "Icons"), ("letters", "Letters")]),
    ),
    row(
        "pinsKeepSite",
        SpacesAndPins,
        "Pins keep their site",
        "Links to another site from a pinned tab open in a new tab.",
        Toggle,
    ),
    row("pinnedRows", SpacesAndPins, "Pins as rows", "List pinned tabs as rows with titles instead of tiles.", Toggle),
    row(
        "pinClickNewTab",
        SpacesAndPins,
        "{cmd-click} on a pin opens a new tab",
        "Open the pin's site in a new tab and leave the pin as it is.",
        Toggle,
    ),
    row(
        "floatingNewTab",
        SpacesAndPins,
        "New tab as a floating field",
        "A new tab starts as an address field over the page.",
        Toggle,
    ),
    // Keyboard and mouse
    row(
        "peekTrigger",
        KeyboardAndMouse,
        "Peek at a link",
        "How to open a link in a card over the page.",
        Control::Choice(&[("off", "Off"), ("shiftClick", "Shift-click"), ("middleHold", "Hold the middle button")]),
    ),
    only(
        row("forceClickPeek", KeyboardAndMouse, "Force click peeks", "Press harder on a link to peek at it.", Toggle),
        MAC,
    ),
    needs(
        row(
            "peekStaysWithTab",
            KeyboardAndMouse,
            "Peeks stay with their tab",
            "Switching away and back keeps the peek open.",
            Toggle,
        ),
        "peekTrigger",
        not_off,
        "Only while peek is on",
    ),
    row(
        "altClickSplit",
        KeyboardAndMouse,
        "{alt-click} opens a split",
        "Open the link beside the page instead of downloading it.",
        Toggle,
    ),
    only(
        row(
            "middleClickScroll",
            KeyboardAndMouse,
            "Middle button scrolls",
            "Press the middle button and move the mouse to scroll.",
            Toggle,
        ),
        NOT_MAC,
    ),
    row(
        "smoothScrolling",
        KeyboardAndMouse,
        "Smooth wheel scrolling",
        "A wheel turn glides instead of jumping.",
        Toggle,
    ),
    only(
        row(
            "bounceAtEnds",
            KeyboardAndMouse,
            "Bounce at the ends",
            "Pages stretch and spring back when you scroll past an end.",
            Toggle,
        ),
        NOT_MAC,
    ),
    row(
        "historySwipe",
        KeyboardAndMouse,
        "Hold a back swipe to pick a page",
        "Keep swiping back to choose from the tab's history.",
        Toggle,
    ),
    row(
        "globalAddressKey",
        KeyboardAndMouse,
        "Address field from any app",
        "A shortcut that opens Lynk's address field over whatever app is in front.",
        Shortcut,
    ),
    needs(
        row(
            "globalOpensSmallWindow",
            KeyboardAndMouse,
            "Open that result in a small window",
            "Pages from that field open in a small window instead of a tab.",
            Toggle,
        ),
        "globalAddressKey",
        non_empty,
        "Only while a shortcut is set",
    ),
    row("editShortcuts", KeyboardAndMouse, "Keyboard shortcuts", "See and change every shortcut.", Button),
    // Extensions
    row("addExtension", Extensions, "Add from the Chrome Web Store", "Paste an extension's store link or id.", Button),
    row(
        "importExtensions",
        Extensions,
        "Bring extensions from another browser",
        "Install the extensions another browser has.",
        Button,
    ),
    row("loadUnpacked", Extensions, "Load unpacked", "Load an extension from a folder, for developing one.", Button),
];

pub fn setting(id: &str) -> Option<&'static Setting> {
    SETTINGS.iter().find(|s| s.id == id)
}

/// The rows a platform shows, in page order.
pub fn rows(platform: Platform) -> impl Iterator<Item = &'static Setting> {
    SETTINGS.iter().filter(move |s| s.available(platform))
}

/// A setting's current value; `None` for a button or an unknown id.
pub fn value(settings: &LynkSettings, id: &str) -> Option<Value> {
    if !setting(id)?.stored() {
        return None;
    }
    serde_json::to_value(settings).ok()?.get(id).cloned()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetError {
    Unknown,
    /// The value is not one this setting takes; nothing changed.
    Invalid,
}

/// Changes one setting. A value of the wrong shape changes nothing.
pub fn set(settings: &mut LynkSettings, id: &str, new: Value) -> Result<(), SetError> {
    if !setting(id).is_some_and(Setting::stored) {
        return Err(SetError::Unknown);
    }
    let mut all = serde_json::to_value(&*settings).map_err(|_| SetError::Invalid)?;
    all[id] = new;
    *settings = serde_json::from_value(all).map_err(|_| SetError::Invalid)?;
    Ok(())
}

/// Why a row is disabled right now, shown as its description; `None` while it is enabled.
pub fn disabled_reason(settings: &LynkSettings, id: &str) -> Option<&'static str> {
    let req = setting(id)?.requires?;
    let v = value(settings, req.id).unwrap_or(Value::Null);
    (!(req.when)(&v)).then_some(req.reason)
}

/// One section's matches for "Find a setting".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SectionMatches {
    pub section: Section,
    pub ids: Vec<&'static str>,
}

impl SectionMatches {
    pub fn count(&self) -> usize {
        self.ids.len()
    }
}

/// Rows whose title, description or option labels contain every word of `query`, grouped by
/// section in page order. Sections without a match are left out, for the nav to dim.
pub fn search(query: &str, platform: Platform) -> Vec<SectionMatches> {
    let mac = platform == Platform::Mac;
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<SectionMatches> = Vec::new();
    for s in rows(platform) {
        let options = match s.control {
            Control::Choice(o) | Control::Thumbnails(o) => o.iter().map(|(_, l)| *l).collect::<Vec<_>>().join(" "),
            _ => String::new(),
        };
        let hay = format!("{} {} {} {}", s.title(mac), s.description(mac), options, s.section.title()).to_lowercase();
        if !words.iter().all(|w| hay.contains(w.as_str())) {
            continue;
        }
        match out.last_mut() {
            Some(m) if m.section == s.section => m.ids.push(s.id),
            _ => out.push(SectionMatches { section: s.section, ids: vec![s.id] }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stored_row_is_a_settings_key_and_every_key_has_a_row() {
        let defaults = serde_json::to_value(LynkSettings::default()).unwrap();
        let keys = defaults.as_object().unwrap();
        for s in SETTINGS.iter().filter(|s| s.stored()) {
            assert!(keys.contains_key(s.id), "{} has no settings key", s.id);
        }
        let not_rows = ["sitePermissions", "pinnedExtensions", "shortcuts"];
        for key in keys.keys() {
            assert!(setting(key).is_some() || not_rows.contains(&key.as_str()), "{key} has no row");
        }
        let mut ids: Vec<&str> = SETTINGS.iter().map(|s| s.id).collect();
        let all = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), all);
    }

    #[test]
    fn extras_default_to_off() {
        // What Chromium does on its own stays on.
        let on = ["smoothScrolling"];
        for s in SETTINGS.iter().filter(|s| s.control == Toggle) {
            assert_eq!(s.default_value(), Some(Value::Bool(on.contains(&s.id))), "{}", s.id);
        }
        let d = |id| setting(id).unwrap().default_value().unwrap();
        assert_eq!(d("appearance"), "auto");
        assert_eq!(d("layout"), "sidebar");
        assert_eq!(d("archiveAfter"), "never");
        assert_eq!(d("sleepTabs"), "never");
        assert_eq!(d("peekTrigger"), "off");
        assert_eq!(d("newTabPosition"), "end");
        assert_eq!(d("globalAddressKey"), "");
        assert!(setting("makeDefault").unwrap().default_value().is_none());
    }

    #[test]
    fn every_option_is_a_value_its_setting_takes() {
        for s in SETTINGS {
            if let Control::Choice(options) | Control::Thumbnails(options) = s.control {
                for (stored, _) in options {
                    let mut settings = LynkSettings::default();
                    set(&mut settings, s.id, Value::from(*stored))
                        .unwrap_or_else(|e| panic!("{}={stored}: {e:?}", s.id));
                    assert_eq!(value(&settings, s.id), Some(Value::from(*stored)));
                }
            }
        }
    }

    #[test]
    fn a_bad_value_changes_nothing() {
        let mut s = LynkSettings::default();
        assert_eq!(set(&mut s, "appearance", Value::from("purple")), Err(SetError::Invalid));
        assert_eq!(set(&mut s, "confirmQuit", Value::from(3)), Err(SetError::Invalid));
        assert_eq!(set(&mut s, "makeDefault", Value::Bool(true)), Err(SetError::Unknown));
        assert_eq!(set(&mut s, "nope", Value::Bool(true)), Err(SetError::Unknown));
        assert_eq!(s, LynkSettings::default());
        set(&mut s, "confirmQuit", Value::Bool(true)).unwrap();
        assert!(s.confirm_quit);
    }

    #[test]
    fn dependent_rows_say_why_they_are_off() {
        let mut s = LynkSettings::default();
        assert_eq!(disabled_reason(&s, "numberKeysSwitchSpaces"), Some(SPACES_REASON));
        assert_eq!(disabled_reason(&s, "dimSleeping"), Some("Only while unused tabs sleep"));
        assert_eq!(disabled_reason(&s, "darkenPages"), None);
        assert_eq!(disabled_reason(&s, "globalOpensSmallWindow"), Some("Only while a shortcut is set"));
        set(&mut s, "spacesEnabled", Value::Bool(true)).unwrap();
        set(&mut s, "appearance", Value::from("light")).unwrap();
        set(&mut s, "layout", Value::from("top")).unwrap();
        set(&mut s, "globalAddressKey", Value::from("primary+shift+space")).unwrap();
        assert_eq!(disabled_reason(&s, "numberKeysSwitchSpaces"), None);
        assert_eq!(disabled_reason(&s, "darkenPages"), Some("Only while the appearance isn't Light"));
        assert_eq!(disabled_reason(&s, "narrowToIcons"), Some("Only with a sidebar layout"));
        assert_eq!(disabled_reason(&s, "globalOpensSmallWindow"), None);
        for row in SETTINGS.iter().filter_map(|r| r.requires) {
            assert!(setting(row.id).is_some_and(Setting::stored), "{}", row.id);
        }
    }

    #[test]
    fn search_groups_matches_by_section_with_counts() {
        let found = search("dark", Platform::Mac);
        let sections: Vec<(Section, usize)> = found.iter().map(|m| (m.section, m.count())).collect();
        assert_eq!(sections, [(Appearance, 4)]);
        assert_eq!(found[0].ids[0], "appearance");
        let tabs = search("ctrl+tab", Platform::Linux);
        assert_eq!(tabs[0].ids, ["switchToOpened", "switcherSkipsPinned"]);
        let spaces = search("space", Platform::Linux);
        assert!(spaces.iter().any(|m| m.section == SpacesAndPins && m.count() >= 5));
        assert!(search("  ", Platform::Mac).is_empty());
        assert!(search("zebra", Platform::Mac).is_empty());
        // Option labels match too.
        assert_eq!(search("icons only", Platform::Mac)[0].ids, ["layout"]);
    }

    fn load_toml(text: &str) -> Result<LynkSettings, String> {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.toml"), text).unwrap();
        let options = nd_settings::Options::new("lynk").paths(nd_settings::Paths::in_dir(dir.path())).env(Vec::new());
        nd_settings::SettingsHandle::<LynkSettings>::load(options)
            .map(|h| h.get().as_ref().clone())
            .map_err(|e| e.to_string())
    }

    #[test]
    fn the_real_old_settings_file_reads_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, include_str!("../fixtures/settings-0511-real.json")).unwrap();
        assert_eq!(LynkSettings::read_legacy(&path), Some(LynkSettings::default()));
    }

    #[test]
    fn a_config_toml_with_good_new_keys_loads() {
        let s = load_toml("sleepTabs = \"later\"\nsmoothScrolling = false\nlayout = \"iconsOnly\"\n").unwrap();
        assert_eq!(s.sleep_tabs, crate::settings::SleepTabs::Later);
        assert!(!s.smooth_scrolling);
        assert_eq!(s.layout, crate::settings::Layout::IconsOnly);
    }

    // shortcut: nd-settings fails the whole load on one bad key, and the shell then starts on
    // defaults; upgrade when nd can drop a single bad key and keep the rest.
    #[test]
    fn a_removed_key_a_wrong_type_or_an_unknown_choice_fails_the_load_naming_the_key() {
        for (text, key) in [
            ("removedLongAgo = true\n", "removedLongAgo"),
            ("confirmQuit = \"yes\"\n", "confirmQuit"),
            ("appearance = \"purple\"\n", "appearance"),
            ("archiveAfter = 3\n", "archiveAfter"),
        ] {
            let err = load_toml(text).expect_err(key);
            assert!(err.contains(key), "{key}: {err}");
        }
    }

    #[test]
    fn platform_rows_and_modifier_names() {
        assert!(rows(Platform::Mac).any(|s| s.id == "forceClickPeek"));
        assert!(!rows(Platform::Linux).any(|s| s.id == "forceClickPeek" || s.id == "importArc"));
        assert!(!rows(Platform::Mac).any(|s| s.id == "bounceAtEnds"));
        let undo = setting("undoReopensTab").unwrap();
        assert_eq!(undo.title(true), "⌘Z reopens a closed tab");
        assert_eq!(undo.title(false), "Ctrl+Z reopens a closed tab");
        assert_eq!(setting("numberKeysSwitchSpaces").unwrap().title(false), "Ctrl+1 to Ctrl+9 switch spaces");
        assert_eq!(setting("pinClickNewTab").unwrap().title(true), "Command-click on a pin opens a new tab");
        assert_eq!(setting("altClickSplit").unwrap().title(false), "Alt-click opens a split");
        for s in SETTINGS {
            let text = format!("{} {}", s.title(true), s.description(false));
            assert!(!text.contains('{') && !text.contains('\u{2014}'), "{}", s.id);
            assert!(s.description(true).ends_with('.'), "{}", s.id);
        }
    }
}
