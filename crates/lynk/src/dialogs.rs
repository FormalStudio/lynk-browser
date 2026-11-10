//! Dialog buttons in the platform's order: the action last on macOS, first elsewhere, with the
//! "always" choice between it and Cancel. Every dialog the app draws lays its buttons out here.

use nd::platform::{Target, target};

/// `[open, always, cancel]` on macOS, `[cancel, always, open]` elsewhere; `always` is left
/// out when the dialog has none.
pub fn button_order<T>(open: T, always: Option<T>, cancel: T) -> Vec<T> {
    let mut buttons: Vec<T> = [Some(open), always, Some(cancel)].into_iter().flatten().collect();
    if target() != Target::MacOs {
        buttons.reverse();
    }
    buttons
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buttons_follow_the_platform() {
        let order = button_order("open", Some("always"), "cancel");
        if target() == Target::MacOs {
            assert_eq!(order, ["open", "always", "cancel"]);
        } else {
            assert_eq!(order, ["cancel", "always", "open"]);
        }
        assert_eq!(button_order("open", None, "cancel").len(), 2);
    }
}
