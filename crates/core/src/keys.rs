//! Every keyboard shortcut the app declares, in the menu's spelling (`primary` is Cmd on macOS
//! and Ctrl elsewhere), the user's rebindings over them, and the search the shortcut editor
//! runs. The menu bar binds from here and the command bar's hints read from here, so a hint
//! never names a key that does nothing. Nothing here listens for keys.
//!
//! Cmd+Tab belongs to macOS's app switcher and never reaches the app, so tabs step with
//! Shift+Cmd+] and Shift+Cmd+[ there and with Ctrl+Tab elsewhere. The floating video is
//! Alt+Cmd+P, not Shift+Cmd+P, which 1Password takes system-wide for Quick Access.

use std::collections::BTreeMap;

pub struct Key {
    pub id: &'static str,
    /// What the shortcut editor lists and searches.
    pub name: &'static str,
    /// The default chord on macOS and on Linux and Windows; empty for a command with none.
    mac: &'static str,
    other: &'static str,
    /// Whether the shell has a command for it yet. Set it when the shell adds one.
    pub bound: bool,
}

const fn k(id: &'static str, name: &'static str, mac: &'static str, other: &'static str) -> Key {
    Key { id, name, mac, other, bound: true }
}

const fn new(id: &'static str, name: &'static str, mac: &'static str, other: &'static str) -> Key {
    Key { id, name, mac, other, bound: false }
}

const KEYS: &[Key] = &[
    k("new-tab", "New tab", "primary+t", "primary+t"),
    k("new-window", "New window", "primary+n", "primary+n"),
    k("private", "New private window", "primary+shift+n", "primary+shift+n"),
    k("address", "Open address", "primary+l", "primary+l"),
    k("switcher", "Switch tabs", "primary+k", "primary+k"),
    k("close-tab", "Close tab", "primary+w", "primary+w"),
    k("reopen-tab", "Reopen closed tab", "primary+shift+t", "primary+shift+t"),
    k("settings", "Settings", "primary+comma", "primary+comma"),
    // macOS's app menu owns Cmd+Q.
    k("quit", "Quit", "", "primary+q"),
    k("find", "Find in page", "primary+f", "primary+f"),
    k("find-next", "Find next", "primary+g", "primary+g"),
    k("find-previous", "Find previous", "primary+shift+g", "primary+shift+g"),
    k("reload", "Reload", "primary+r", "primary+r"),
    k("stop", "Stop loading", "primary+period", ""),
    k("copy-address", "Copy address", "primary+shift+c", "primary+shift+c"),
    k("back", "Back", "primary+[", "primary+["),
    k("forward", "Forward", "primary+]", "primary+]"),
    k("toggle-sidebar", "Hide sidebar", "primary+shift+b", "primary+shift+b"),
    k("save-page", "Save page", "primary+s", "primary+s"),
    k("print", "Print", "primary+p", "primary+p"),
    k("layout", "Switch layout", "primary+alt+s", "primary+alt+s"),
    k("zoom-in", "Zoom in", "primary+plus", "primary+plus"),
    k("zoom-out", "Zoom out", "primary+minus", "primary+minus"),
    k("zoom-reset", "Actual size", "primary+0", "primary+0"),
    k("reader", "Reading mode", "primary+shift+r", "primary+shift+r"),
    k("float", "Floating video", "primary+alt+p", "primary+alt+p"),
    k("hide-element", "Hide an element", "primary+shift+h", "primary+shift+h"),
    k("next-tab", "Next tab", "primary+shift+bracketright", "primary+tab"),
    k("prev-tab", "Previous tab", "primary+shift+bracketleft", "primary+shift+tab"),
    // The panels take Chrome's own chords on each platform.
    k("history", "History", "primary+y", "primary+h"),
    k("downloads", "Downloads", "primary+shift+j", "primary+j"),
    k("bookmarks", "Bookmarks", "primary+alt+b", "primary+shift+o"),
    k("bookmark-page", "Bookmark page", "primary+d", "primary+d"),
    k("view-source", "View source", "primary+alt+u", "primary+u"),
    k("devtools", "Developer tools", "primary+alt+i", "primary+shift+i"),
    k("clear-data", "Clear browsing data", "primary+shift+backspace", "primary+shift+delete"),
    k("fullscreen", "Full screen", "primary+ctrl+f", "F11"),
    k("pin-tab", "Pin or unpin tab", "primary+shift+d", "primary+shift+d"),
    new("open-as-split", "Open as split", "primary+enter", "primary+enter"),
    new("tidy", "Tidy tabs", "primary+alt+t", "primary+alt+t"),
    new("add-separator", "Add separator below", "", ""),
    new("new-folder", "New folder", "", ""),
    new("move-to-root", "Move to root", "", ""),
    new("archive", "Archive", "", ""),
    // Spaces go by Arc's Ctrl+number on macOS; Ctrl+number is the tab chord elsewhere, so
    // Alt+number, which Lynk leaves free there.
    new("space-1", "Go to space 1", "ctrl+1", "alt+1"),
    new("space-2", "Go to space 2", "ctrl+2", "alt+2"),
    new("space-3", "Go to space 3", "ctrl+3", "alt+3"),
    new("space-4", "Go to space 4", "ctrl+4", "alt+4"),
    new("space-5", "Go to space 5", "ctrl+5", "alt+5"),
    new("space-6", "Go to space 6", "ctrl+6", "alt+6"),
    new("space-7", "Go to space 7", "ctrl+7", "alt+7"),
    new("space-8", "Go to space 8", "ctrl+8", "alt+8"),
    new("space-9", "Go to space 9", "ctrl+9", "alt+9"),
];

/// The commands the shell binds today.
pub fn ids() -> impl Iterator<Item = &'static str> {
    KEYS.iter().filter(|k| k.bound).map(|k| k.id)
}

/// Every command, bound or not, in table order.
pub fn all() -> &'static [Key] {
    KEYS
}

fn key(id: &str) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.id == id)
}

/// The default chord.
pub fn chord(id: &str, mac: bool) -> Option<&'static str> {
    key(id).map(|k| if mac { k.mac } else { k.other }).filter(|c| !c.is_empty())
}

/// Chrome's tab chords: the first eight tabs by number, and 9 for the last.
pub fn tab_key(index: usize, count: usize) -> Option<String> {
    if index + 1 == count && index >= 8 {
        Some("primary+9".into())
    } else if index < 8 {
        Some(format!("primary+{}", index + 1))
    } else {
        None
    }
}

/// The user's rebindings, by command id; an empty chord leaves the command without one.
/// Stored in the settings as `shortcuts`.
pub type Overrides = BTreeMap<String, String>;

/// The chord a command answers to with the user's rebindings applied.
pub fn effective(id: &str, mac: bool, overrides: &Overrides) -> Option<String> {
    match overrides.get(id) {
        Some(c) if c.is_empty() => None,
        Some(c) => Some(c.clone()),
        None => chord(id, mac).map(str::to_owned),
    }
}

const MODIFIERS: [&str; 4] = ["primary", "ctrl", "alt", "shift"];

/// A chord in one spelling: modifiers in a fixed order, the key in lower case. `None` for an
/// unknown modifier, no key, or a plain key without a modifier (only F keys stand alone).
pub fn normalize_chord(chord: &str) -> Option<String> {
    let parts: Vec<String> = chord.split('+').map(|p| p.trim().to_lowercase()).collect();
    let (key, mods) = parts.split_last()?;
    if key.is_empty() || MODIFIERS.contains(&key.as_str()) || mods.iter().any(|m| !MODIFIERS.contains(&m.as_str())) {
        return None;
    }
    let function_key =
        key.len() > 1 && key.starts_with('f') && key[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n));
    if mods.is_empty() && !function_key {
        return None;
    }
    let mut out: Vec<String> =
        MODIFIERS.iter().filter(|m| mods.iter().any(|x| x == *m)).map(|m| (*m).to_owned()).collect();
    out.push(if function_key { key.to_uppercase() } else { key.clone() });
    Some(out.join("+"))
}

/// A chord as the platform hears it, for comparing one spelling with another: modifier and key
/// aliases resolved, and `ctrl` read as `primary` on Linux and Windows, where it is.
fn canonical(chord: &str, mac: bool) -> Option<String> {
    let parts: Vec<&str> = chord
        .split('+')
        .map(|p| match p.trim().to_lowercase().as_str() {
            "cmd" | "command" => "primary",
            "ctrl" | "control" if !mac => "primary",
            "control" => "ctrl",
            "option" | "opt" => "alt",
            "[" => "bracketleft",
            "]" => "bracketright",
            "," => "comma",
            "." => "period",
            "-" => "minus",
            "return" => "enter",
            "del" => "delete",
            "esc" => "escape",
            _ => p,
        })
        .collect();
    normalize_chord(&parts.join("+"))
}

/// Chords the system keeps, and the editing chords a page's text fields need.
fn reserved(mac: bool) -> &'static [&'static str] {
    if mac {
        &[
            "primary+q",
            "primary+h",
            "primary+m",
            "primary+tab",
            "primary+space",
            "primary+c",
            "primary+v",
            "primary+x",
            "primary+a",
            "primary+z",
            "primary+shift+z",
        ]
    } else {
        &["alt+F4", "alt+tab", "primary+c", "primary+v", "primary+x", "primary+a", "primary+z", "primary+y"]
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebindError {
    UnknownCommand,
    BadChord,
    Reserved,
    /// Another command already has the chord; its id, or `tab-N` for a tab's number chord.
    Conflict(String),
}

/// What already answers to `chord` (canonical) other than command `id`.
fn taken(overrides: &Overrides, id: &str, chord: &str, mac: bool) -> Result<(), RebindError> {
    if reserved(mac).iter().any(|r| canonical(r, mac).as_deref() == Some(chord)) {
        return Err(RebindError::Reserved);
    }
    if let Some(n) = (1..=9).find(|n| canonical(&format!("primary+{n}"), mac).as_deref() == Some(chord)) {
        return Err(RebindError::Conflict(format!("tab-{n}")));
    }
    let holds = |k: &Key| effective(k.id, mac, overrides).and_then(|c| canonical(&c, mac)).as_deref() == Some(chord);
    match KEYS.iter().find(|k| k.id != id && holds(k)) {
        Some(other) => Err(RebindError::Conflict(other.id.into())),
        None => Ok(()),
    }
}

/// Gives a command a new chord, or none with an empty one. A chord another command answers to,
/// in any spelling, is refused with that command's id, so the editor can offer to swap.
pub fn rebind(overrides: &mut Overrides, id: &str, chord: &str, mac: bool) -> Result<(), RebindError> {
    if key(id).is_none() {
        return Err(RebindError::UnknownCommand);
    }
    if chord.trim().is_empty() {
        overrides.insert(id.into(), String::new());
        return Ok(());
    }
    let chord = canonical(chord, mac).ok_or(RebindError::BadChord)?;
    taken(overrides, id, &chord, mac)?;
    if self::chord(id, mac).and_then(|c| canonical(c, mac)) == Some(chord.clone()) {
        overrides.remove(id);
    } else {
        overrides.insert(id.into(), chord);
    }
    Ok(())
}

/// Chords more than one command answers to, which a hand-edited settings file can hold. The
/// editor marks these commands; the menu binds the first.
pub fn conflicts(overrides: &Overrides, mac: bool) -> Vec<(String, Vec<&'static str>)> {
    let mut by_chord: BTreeMap<String, Vec<&'static str>> = BTreeMap::new();
    for k in KEYS {
        if let Some(c) = effective(k.id, mac, overrides) {
            by_chord.entry(canonical(&c, mac).unwrap_or(c)).or_default().push(k.id);
        }
    }
    by_chord.into_iter().filter(|(_, ids)| ids.len() > 1).collect()
}

/// Back to the default chord, unless another command has taken it since.
pub fn reset(overrides: &mut Overrides, id: &str, mac: bool) -> Result<(), RebindError> {
    if let Some(default) = chord(id, mac).and_then(|c| canonical(c, mac)) {
        taken(overrides, id, &default, mac)?;
    }
    overrides.remove(id);
    Ok(())
}

/// Commands whose name contains every word of `query`, in table order.
pub fn search(query: &str) -> Vec<&'static Key> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    KEYS.iter().filter(|k| words.iter().all(|w| k.name.to_lowercase().contains(w.as_str()))).collect()
}

/// `primary+shift+t` as the platform writes it: "⇧⌘T" on macOS, "Ctrl+Shift+T" elsewhere.
pub fn shortcut_label(keys: &str, mac: bool) -> String {
    let mut parts: Vec<&str> = keys.split('+').collect();
    let key = parts.pop().unwrap_or("");
    let shown = match key {
        "comma" => ",".into(),
        "period" => ".".into(),
        "plus" => "+".into(),
        "minus" => "-".into(),
        "tab" => "Tab".into(),
        "bracketleft" => "[".into(),
        "bracketright" => "]".into(),
        "delete" => "Del".into(),
        "backspace" => "⌫".into(),
        "enter" if mac => "↩".into(),
        "enter" => "Enter".into(),
        "space" => "Space".into(),
        other => other.to_uppercase(),
    };
    if mac {
        // macOS orders modifiers ⌃⌥⇧⌘ whatever order they were declared in.
        let mods: String = [("ctrl", "⌃"), ("alt", "⌥"), ("shift", "⇧"), ("primary", "⌘")]
            .iter()
            .filter(|(m, _)| parts.contains(m))
            .map(|(_, glyph)| *glyph)
            .collect();
        return format!("{mods}{shown}");
    }
    let mut out: Vec<String> = parts
        .iter()
        .map(|m| match *m {
            "primary" | "ctrl" => "Ctrl".to_owned(),
            "shift" => "Shift".to_owned(),
            "alt" => "Alt".to_owned(),
            other => other.to_owned(),
        })
        .collect();
    out.push(shown);
    out.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_two_declared_shortcuts_share_a_chord() {
        for mac in [true, false] {
            let mut chords: Vec<&str> = KEYS.iter().filter_map(|k| chord(k.id, mac)).collect();
            let all = chords.len();
            chords.sort_unstable();
            chords.dedup();
            assert_eq!(chords.len(), all, "mac={mac}");
            for c in &chords {
                assert_eq!(normalize_chord(c).as_deref(), Some(*c), "{c} is not in one spelling");
                assert!(!reserved(mac).contains(c), "{c}");
            }
        }
        let mut ids: Vec<&str> = KEYS.iter().map(|k| k.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), KEYS.len());
    }

    #[test]
    fn shortcut_labels_follow_the_platform() {
        assert_eq!(shortcut_label("primary+shift+t", true), "⇧⌘T");
        assert_eq!(shortcut_label("primary+comma", true), "⌘,");
        assert_eq!(shortcut_label("primary+shift+t", false), "Ctrl+Shift+T");
        assert_eq!(shortcut_label("primary+tab", false), "Ctrl+Tab");
        assert_eq!(shortcut_label("F11", false), "F11");
        assert_eq!(shortcut_label("primary+ctrl+f", true), "⌃⌘F");
        assert_eq!(shortcut_label("primary+enter", true), "⌘↩");
        assert_eq!(shortcut_label("primary+enter", false), "Ctrl+Enter");
        assert_eq!(shortcut_label("ctrl+1", true), "⌃1");
    }

    #[test]
    fn every_command_has_both_forms_from_one_row() {
        assert_eq!(chord("open-as-split", true), Some("primary+enter"));
        assert_eq!(chord("space-2", true), Some("ctrl+2"));
        assert_eq!(chord("space-2", false), Some("alt+2"));
        assert_eq!(chord("tidy", false), Some("primary+alt+t"));
        assert_eq!(chord("new-folder", true), None);
        assert!(ids().all(|id| key(id).unwrap().bound));
        assert!(!ids().any(|id| id == "tidy"));
        assert!(KEYS.iter().all(|k| !k.name.is_empty()));
    }

    #[test]
    fn tab_chords_number_the_first_eight_and_nine_is_the_last() {
        assert_eq!(tab_key(0, 3).as_deref(), Some("primary+1"));
        assert_eq!(tab_key(7, 12).as_deref(), Some("primary+8"));
        assert_eq!(tab_key(8, 12), None);
        assert_eq!(tab_key(11, 12).as_deref(), Some("primary+9"));
        assert_eq!(tab_key(8, 9).as_deref(), Some("primary+9"));
    }

    #[test]
    fn chords_read_in_one_spelling() {
        assert_eq!(normalize_chord("Shift + Primary + T").as_deref(), Some("primary+shift+t"));
        assert_eq!(normalize_chord("f5").as_deref(), Some("F5"));
        assert_eq!(normalize_chord("t"), None);
        assert_eq!(normalize_chord("primary+shift"), None);
        assert_eq!(normalize_chord("hyper+t"), None);
        assert_eq!(normalize_chord(""), None);
    }

    #[test]
    fn rebinding_refuses_conflicts_and_round_trips() {
        let mut o = Overrides::new();
        assert_eq!(rebind(&mut o, "tidy", "primary+t", true), Err(RebindError::Conflict("new-tab".into())));
        assert_eq!(rebind(&mut o, "tidy", "primary+3", true), Err(RebindError::Conflict("tab-3".into())));
        assert_eq!(rebind(&mut o, "tidy", "primary+q", true), Err(RebindError::Reserved));
        assert_eq!(rebind(&mut o, "tidy", "primary+v", false), Err(RebindError::Reserved));
        assert_eq!(rebind(&mut o, "tidy", "alt+F4", false), Err(RebindError::Reserved));
        assert_eq!(rebind(&mut o, "tidy", "q", true), Err(RebindError::BadChord));
        assert_eq!(rebind(&mut o, "nope", "primary+e", true), Err(RebindError::UnknownCommand));
        assert!(o.is_empty());
        rebind(&mut o, "tidy", "shift+primary+e", true).unwrap();
        assert_eq!(effective("tidy", true, &o).as_deref(), Some("primary+shift+e"));
        // Freeing a chord lets another command take it.
        rebind(&mut o, "new-tab", "", true).unwrap();
        assert_eq!(effective("new-tab", true, &o), None);
        rebind(&mut o, "new-folder", "primary+t", true).unwrap();
        assert_eq!(effective("new-folder", true, &o).as_deref(), Some("primary+t"));
        // Setting a default back drops the override.
        rebind(&mut o, "tidy", "primary+alt+t", true).unwrap();
        assert!(!o.contains_key("tidy"));
        assert_eq!(reset(&mut o, "new-tab", true), Err(RebindError::Conflict("new-folder".into())));
    }

    #[test]
    fn two_commands_on_one_chord_from_a_hand_edited_file_are_reported() {
        let mut o = Overrides::new();
        assert!(conflicts(&o, true).is_empty() && conflicts(&o, false).is_empty());
        o.insert("tidy".into(), "primary+t".into());
        o.insert("archive".into(), "primary+t".into());
        assert_eq!(conflicts(&o, true), [("primary+t".to_owned(), vec!["new-tab", "tidy", "archive"])]);
    }

    #[test]
    fn search_runs_over_command_names() {
        let names = |q: &str| search(q).iter().map(|k| k.id).collect::<Vec<_>>();
        assert_eq!(names("split"), ["open-as-split"]);
        assert_eq!(names("TAB new"), ["new-tab"]);
        assert_eq!(names("space 3"), ["space-3"]);
        assert!(names("zebra").is_empty());
        assert_eq!(names("").len(), KEYS.len());
    }

    #[test]
    fn spellings_of_one_chord_conflict_alike() {
        let mut o = Overrides::new();
        assert_eq!(rebind(&mut o, "reload", "ctrl+t", false), Err(RebindError::Conflict("new-tab".into())));
        assert_eq!(rebind(&mut o, "reload", "Control+T", false), Err(RebindError::Conflict("new-tab".into())));
        assert_eq!(rebind(&mut o, "reload", "ctrl+1", false), Err(RebindError::Conflict("tab-1".into())));
        assert_eq!(rebind(&mut o, "reload", "ctrl+c", false), Err(RebindError::Reserved));
        assert_eq!(rebind(&mut o, "reload", "cmd+q", true), Err(RebindError::Reserved));
        assert_eq!(rebind(&mut o, "reload", "primary+bracketleft", true), Err(RebindError::Conflict("back".into())));
        assert_eq!(rebind(&mut o, "back", "primary+[", true), Ok(()));
        assert!(o.is_empty(), "the default in another spelling is no override");
        o.insert("reload".into(), "ctrl+t".into());
        assert_eq!(conflicts(&o, false).len(), 1, "a hand-edited spelling still conflicts");
    }

    #[test]
    fn a_reset_never_takes_a_chord_another_command_holds() {
        let mut o = Overrides::new();
        rebind(&mut o, "new-tab", "primary+alt+n", false).unwrap();
        rebind(&mut o, "reload", "primary+t", false).unwrap();
        let before = o.clone();
        assert_eq!(reset(&mut o, "new-tab", false), Err(RebindError::Conflict("reload".into())));
        assert_eq!(o, before);
        rebind(&mut o, "reload", "", false).unwrap();
        assert_eq!(reset(&mut o, "new-tab", false), Ok(()));
        assert_eq!(effective("new-tab", false, &o).as_deref(), Some("primary+t"));
    }
}
