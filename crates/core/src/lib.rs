//! Everything Lynk does that does not draw: address parsing, settings, the session, the
//! omnibox's ranking, history, bookmarks, favicons, downloads, permissions, zoom, extensions and
//! the ad blocker's lists. No nd dependency beyond `nd-data` and `nd-settings`.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};

pub mod bookmarks;
pub mod downloads;
pub mod extensions;
pub mod favicons;
pub mod history;
pub mod keys;
pub mod omnibox;
pub mod pages;
pub mod permissions;
pub mod session;
pub mod settings;
pub mod tabstate;
pub mod url;
pub mod webstore;
pub mod zoom;

/// Milliseconds since the Unix epoch, JavaScript's `Date.now()`, which every file the 0.5.11 app
/// wrote is stamped with.
pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// JavaScript's `n.toString(36)`, which the old ids are built from.
pub(crate) fn radix36(mut n: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).expect("ascii digits")
}

/// A list where an entry of the wrong shape is dropped rather than failing the whole file: a
/// hand-edited or half-migrated file keeps what it can.
pub(crate) fn lenient_items<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    let raw = Option::<Vec<serde_json::Value>>::deserialize(d)?.unwrap_or_default();
    Ok(raw.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn radix36_matches_javascript() {
        assert_eq!(super::radix36(0), "0");
        assert_eq!(super::radix36(35), "z");
        assert_eq!(super::radix36(1_700_000_000_000), "loyw3v28");
    }
}
