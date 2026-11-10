//! The extension rows the puzzle popover and the pinned buttons draw, joined from the engine's
//! two lists, and Chrome's popup bounds.

use std::collections::HashMap;

/// An installed extension as the engine's registry reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstalledExtension {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub icon_url: String,
    pub options_url: String,
}

/// An extension's action as the engine reports it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtensionAction {
    pub id: String,
    pub popup_url: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtensionRow {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// Drawable icon bytes. An action's icon is a `chrome-extension://` address no native image
    /// loader can fetch; the registry's is a `data:` URI. Empty when the extension ships none,
    /// and the puzzle glyph stands in.
    pub icon_data: String,
    /// Empty for an action with no popup; a click on one is `chrome.action.onClicked`, which
    /// the engine runs.
    pub popup_url: String,
    /// Empty when the extension declares no options page.
    pub options_url: String,
}

pub fn extension_rows(registry: &[InstalledExtension], actions: &[ExtensionAction]) -> Vec<ExtensionRow> {
    let by_id: HashMap<&str, &ExtensionAction> = actions.iter().map(|a| (a.id.as_str(), a)).collect();
    registry
        .iter()
        .map(|e| ExtensionRow {
            id: e.id.clone(),
            name: e.name.clone(),
            enabled: e.enabled,
            icon_data: if e.icon_url.starts_with("data:") { e.icon_url.clone() } else { String::new() },
            popup_url: by_id.get(e.id.as_str()).map(|a| a.popup_url.clone()).unwrap_or_default(),
            options_url: e.options_url.clone(),
        })
        .collect()
}

/// The rows with a toolbar button, in pin order rather than registry order. A pinned id with
/// nothing installed behind it draws nothing.
pub fn pinned_rows<'a>(rows: &'a [ExtensionRow], pinned: &[String]) -> Vec<&'a ExtensionRow> {
    pinned.iter().filter_map(|id| rows.iter().find(|r| &r.id == id)).collect()
}

/// The page a hidden view shows so the action's live state can be read: any document of the
/// extension answers for it, so it is the manifest, which runs none of the extension's code. A
/// popup document runs as if opened: 1Password's asked the desktop app to unlock on every
/// launch, then closed itself and left the probe dead.
pub fn probe_url(id: &str) -> String {
    format!("chrome-extension://{id}/manifest.json")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeVariant {
    Neutral,
    Accent,
    Success,
    Warning,
    Error,
}

/// The badge colour an extension asked for, as the nearest of the app's badge variants. A grey
/// or colourless badge is the neutral one.
pub fn badge_variant(rgba: &[u8]) -> BadgeVariant {
    let channel = |i: usize| f64::from(rgba.get(i).copied().unwrap_or(0));
    let (r, g, b) = (channel(0), channel(1), channel(2));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max == 0.0 || (max - min) / max < 0.25 {
        return BadgeVariant::Neutral;
    }
    let hue = 60.0
        * if max == r {
            ((g - b) / (max - min) + 6.0) % 6.0
        } else if max == g {
            (b - r) / (max - min) + 2.0
        } else {
            (r - g) / (max - min) + 4.0
        };
    if !(20.0..330.0).contains(&hue) {
        BadgeVariant::Error
    } else if hue < 70.0 {
        BadgeVariant::Warning
    } else if hue < 170.0 {
        BadgeVariant::Success
    } else {
        BadgeVariant::Accent
    }
}

/// An uninstalled id stays pinned on purpose: reinstalling brings its button back.
pub fn toggle_pinned(pinned: &mut Vec<String>, id: &str) {
    if let Some(at) = pinned.iter().position(|p| p == id) {
        pinned.remove(at);
    } else {
        pinned.push(id.to_owned());
    }
}

/// Chrome's own popup bounds, which a popup document is written against. The app owns the size
/// because a popup in an app view is not sized by its document.
pub const POPUP_MIN: f64 = 25.0;
pub const POPUP_MAX_WIDTH: f64 = 800.0;
pub const POPUP_MAX_HEIGHT: f64 = 600.0;
pub const POPUP_DEFAULT_WIDTH: f64 = 360.0;
pub const POPUP_DEFAULT_HEIGHT: f64 = 520.0;

pub fn clamp_popup(width: f64, height: f64) -> (f64, f64) {
    let fit =
        |v: f64, max: f64, fallback: f64| if v.is_finite() && v >= POPUP_MIN { v.round().min(max) } else { fallback };
    (fit(width, POPUP_MAX_WIDTH, POPUP_DEFAULT_WIDTH), fit(height, POPUP_MAX_HEIGHT, POPUP_DEFAULT_HEIGHT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_join_the_registry_and_the_actions_in_pin_order() {
        let registry = [
            InstalledExtension {
                id: "a".into(),
                name: "A".into(),
                enabled: true,
                icon_url: "data:image/png;base64,x".into(),
                options_url: String::new(),
            },
            InstalledExtension {
                id: "b".into(),
                name: "B".into(),
                enabled: false,
                icon_url: "chrome-extension://b/i.png".into(),
                options_url: "chrome-extension://b/o.html".into(),
            },
        ];
        let actions = [ExtensionAction { id: "b".into(), popup_url: "chrome-extension://b/p.html".into() }];
        let rows = extension_rows(&registry, &actions);
        assert_eq!(rows[0].icon_data, "data:image/png;base64,x");
        assert_eq!(rows[0].popup_url, "");
        assert_eq!(rows[1].icon_data, "");
        assert_eq!(rows[1].popup_url, "chrome-extension://b/p.html");
        let pinned = vec!["b".to_owned(), "gone".to_owned(), "a".to_owned()];
        let ids: Vec<&str> = pinned_rows(&rows, &pinned).iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["b", "a"]);
        let mut pins = pinned;
        toggle_pinned(&mut pins, "gone");
        toggle_pinned(&mut pins, "c");
        assert_eq!(pins, ["b", "a", "c"]);
        assert_eq!(probe_url("a"), "chrome-extension://a/manifest.json");
    }

    #[test]
    fn badges_and_popup_sizes() {
        assert_eq!(badge_variant(&[0, 0, 0, 0]), BadgeVariant::Neutral);
        assert_eq!(badge_variant(&[120, 120, 128, 255]), BadgeVariant::Neutral);
        assert_eq!(badge_variant(&[220, 20, 20, 255]), BadgeVariant::Error);
        assert_eq!(badge_variant(&[240, 180, 0, 255]), BadgeVariant::Warning);
        assert_eq!(badge_variant(&[20, 200, 60, 255]), BadgeVariant::Success);
        assert_eq!(badge_variant(&[30, 90, 240, 255]), BadgeVariant::Accent);
        assert_eq!(clamp_popup(10.0, f64::NAN), (POPUP_DEFAULT_WIDTH, POPUP_DEFAULT_HEIGHT));
        assert_eq!(clamp_popup(1200.4, 300.6), (800.0, 301.0));
    }
}
