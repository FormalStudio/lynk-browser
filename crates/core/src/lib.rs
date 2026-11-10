//! Everything Lynk does that does not draw: address parsing, settings, the session, the
//! omnibox's ranking, history, bookmarks, favicons, downloads, permissions, zoom, extensions and
//! the ad blocker's lists. No nd dependency beyond `nd-data` and `nd-settings`.

pub mod pages;
pub mod permissions;
pub mod session;
pub mod settings;
pub mod tabstate;
pub mod url;
