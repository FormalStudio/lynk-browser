//! Page zoom: Chromium's own preset levels, so a step from the menu, the popover and a chord
//! inside the page (which the engine serves itself) all land on the same values. Levels are kept
//! per host, the way every desktop browser scopes them.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::url::host_of;

pub const ZOOM_STEPS: [f64; 17] =
    [0.25, 0.33, 0.5, 0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5, 3.0, 4.0, 5.0];
pub const ZOOM_MIN: f64 = ZOOM_STEPS[0];
pub const ZOOM_MAX: f64 = ZOOM_STEPS[ZOOM_STEPS.len() - 1];

/// How long the popover stays up after a chord or a menu step, the time Chrome gives its bubble.
pub const ZOOM_NOTICE: Duration = Duration::from_millis(1500);

/// The next preset above or below `current`. A factor between two presets (from an older
/// session) steps to the nearest one in that direction.
pub fn step_zoom(current: f64, up: bool) -> f64 {
    if up {
        ZOOM_STEPS.iter().copied().find(|z| *z > current + 0.001).unwrap_or(ZOOM_MAX)
    } else {
        ZOOM_STEPS.iter().rev().copied().find(|z| *z < current - 0.001).unwrap_or(ZOOM_MIN)
    }
}

pub fn clamp_zoom(factor: f64) -> f64 {
    ((factor * 100.0).round() / 100.0).clamp(ZOOM_MIN, ZOOM_MAX)
}

pub fn is_default_zoom(factor: f64) -> bool {
    (factor - 1.0).abs() < 0.005
}

pub fn zoom_percent(factor: f64) -> String {
    format!("{}%", (factor * 100.0).round())
}

/// The level kept for the page's host, 1 when there is none.
pub fn zoom_for(zoom_by_host: &BTreeMap<String, f64>, url: &str) -> f64 {
    zoom_by_host.get(&host_of(url)).copied().unwrap_or(1.0)
}

/// Keeps `factor` for the page's host. `false` for a page with no host, which has no level of
/// its own.
pub fn remember_zoom(zoom_by_host: &mut BTreeMap<String, f64>, url: &str, factor: f64) -> bool {
    let host = host_of(url);
    if host.is_empty() {
        return false;
    }
    zoom_by_host.insert(host, clamp_zoom(factor));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_chromiums_presets() {
        assert_eq!(step_zoom(1.0, true), 1.1);
        assert_eq!(step_zoom(1.0, false), 0.9);
        assert_eq!(step_zoom(1.17, true), 1.25);
        assert_eq!(step_zoom(1.17, false), 1.1);
        assert_eq!(step_zoom(5.0, true), ZOOM_MAX);
        assert_eq!(step_zoom(0.25, false), ZOOM_MIN);
        assert_eq!(clamp_zoom(9.0), 5.0);
        assert_eq!(clamp_zoom(1.234), 1.23);
        assert!(is_default_zoom(1.004) && !is_default_zoom(1.1));
        assert_eq!(zoom_percent(0.67), "67%");
        assert_eq!(zoom_percent(1.0), "100%");
    }

    #[test]
    fn levels_are_kept_per_host() {
        let mut map = BTreeMap::new();
        assert!(remember_zoom(&mut map, "https://a.test:8443/x", 1.25));
        assert!(!remember_zoom(&mut map, "", 2.0));
        assert_eq!(zoom_for(&map, "https://a.test:8443/y"), 1.25);
        assert_eq!(zoom_for(&map, "https://a.test/"), 1.0);
    }
}
