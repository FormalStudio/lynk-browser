//! Every keyboard shortcut the app declares, in the menu's spelling (`primary` is Cmd on macOS
//! and Ctrl elsewhere). The menu bar binds from here and the command bar's hints read from here,
//! so a hint never names a key that does nothing.
//!
//! Cmd+Tab belongs to macOS's app switcher and never reaches the app, so tabs step with
//! Shift+Cmd+] and Shift+Cmd+[ there and with Ctrl+Tab elsewhere. The floating video is
//! Alt+Cmd+P, not Shift+Cmd+P, which 1Password takes system-wide for Quick Access.

/// `(id, macOS chord, Linux chord)`.
const KEYS: &[(&str, &str, &str)] = &[
    ("new-tab", "primary+t", "primary+t"),
    ("new-window", "primary+n", "primary+n"),
    ("private", "primary+shift+n", "primary+shift+n"),
    ("address", "primary+l", "primary+l"),
    ("switcher", "primary+k", "primary+k"),
    ("close-tab", "primary+w", "primary+w"),
    ("reopen-tab", "primary+shift+t", "primary+shift+t"),
    ("settings", "primary+comma", "primary+comma"),
    ("find", "primary+f", "primary+f"),
    ("find-next", "primary+g", "primary+g"),
    ("find-previous", "primary+shift+g", "primary+shift+g"),
    ("reload", "primary+r", "primary+r"),
    ("copy-address", "primary+shift+c", "primary+shift+c"),
    ("back", "primary+[", "primary+["),
    ("forward", "primary+]", "primary+]"),
    ("toggle-sidebar", "primary+shift+b", "primary+shift+b"),
    ("save-page", "primary+s", "primary+s"),
    ("layout", "primary+alt+s", "primary+alt+s"),
    ("zoom-in", "primary+plus", "primary+plus"),
    ("zoom-out", "primary+minus", "primary+minus"),
    ("zoom-reset", "primary+0", "primary+0"),
    ("reader", "primary+shift+r", "primary+shift+r"),
    ("float", "primary+alt+p", "primary+alt+p"),
    ("hide-element", "primary+shift+h", "primary+shift+h"),
    ("next-tab", "primary+shift+bracketright", "primary+tab"),
    ("prev-tab", "primary+shift+bracketleft", "primary+shift+tab"),
    // The panels take Chrome's own chords on each platform.
    ("history", "primary+y", "primary+h"),
    ("downloads", "primary+shift+j", "primary+j"),
    ("bookmarks", "primary+alt+b", "primary+shift+o"),
    ("bookmark-page", "primary+d", "primary+d"),
    ("view-source", "primary+alt+u", "primary+u"),
    ("devtools", "primary+alt+i", "primary+shift+i"),
    ("clear-data", "primary+shift+backspace", "primary+shift+delete"),
    ("fullscreen", "primary+ctrl+f", "F11"),
];

pub fn ids() -> impl Iterator<Item = &'static str> {
    KEYS.iter().map(|(id, _, _)| *id)
}

pub fn chord(id: &str, mac: bool) -> Option<&'static str> {
    KEYS.iter().find(|(k, _, _)| *k == id).map(|(_, m, l)| if mac { *m } else { *l })
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

/// `primary+shift+t` as the platform writes it: "⇧⌘T" on macOS, "Ctrl+Shift+T" elsewhere.
pub fn shortcut_label(keys: &str, mac: bool) -> String {
    let mut parts: Vec<&str> = keys.split('+').collect();
    let key = parts.pop().unwrap_or("");
    let shown = match key {
        "comma" => ",".into(),
        "plus" => "+".into(),
        "minus" => "-".into(),
        "tab" => "Tab".into(),
        "bracketleft" => "[".into(),
        "bracketright" => "]".into(),
        "delete" => "Del".into(),
        "backspace" => "⌫".into(),
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
            let mut chords: Vec<&str> = ids().filter_map(|id| chord(id, mac)).collect();
            let all = chords.len();
            chords.sort_unstable();
            chords.dedup();
            assert_eq!(chords.len(), all, "mac={mac}");
        }
    }

    #[test]
    fn shortcut_labels_follow_the_platform() {
        assert_eq!(shortcut_label("primary+shift+t", true), "⇧⌘T");
        assert_eq!(shortcut_label("primary+comma", true), "⌘,");
        assert_eq!(shortcut_label("primary+shift+t", false), "Ctrl+Shift+T");
        assert_eq!(shortcut_label("primary+tab", false), "Ctrl+Tab");
        assert_eq!(shortcut_label("F11", false), "F11");
        assert_eq!(shortcut_label("primary+ctrl+f", true), "⌃⌘F");
    }

    #[test]
    fn tab_chords_number_the_first_eight_and_nine_is_the_last() {
        assert_eq!(tab_key(0, 3).as_deref(), Some("primary+1"));
        assert_eq!(tab_key(7, 12).as_deref(), Some("primary+8"));
        assert_eq!(tab_key(8, 12), None);
        assert_eq!(tab_key(11, 12).as_deref(), Some("primary+9"));
        assert_eq!(tab_key(8, 9).as_deref(), Some("primary+9"));
    }
}
