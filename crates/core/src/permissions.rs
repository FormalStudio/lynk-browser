//! Site permissions: the sentence a prompt reads, and the decisions kept per origin.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What a page asked for, held until it is answered. An unanswered id leaves the page waiting
/// for ever, so every prompt that leaves the queue answers first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionPrompt {
    pub id: String,
    /// A prompt belongs to its tab, not to the window: a background tab's request waits until
    /// that tab is looked at.
    pub tab_id: String,
    pub origin: String,
    pub types: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecision {
    Allow,
    Block,
}

/// `Dismiss` refuses without recording a decision, for a prompt the user waved away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionResult {
    Allow,
    Deny,
    Dismiss,
}

/// Decisions by origin, then by permission type. Only Allow and Block write here.
pub type SitePermissions = BTreeMap<String, BTreeMap<String, PermissionDecision>>;

/// One request can carry several types (`getUserMedia({audio, video})`), sent as one
/// comma-separated string.
pub fn split_types(types: &str) -> Vec<String> {
    types.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned).collect()
}

/// The verb is separate from the noun because "use your camera" and "send you notifications"
/// cannot share one.
const PHRASES: &[(&str, &str, &str)] = &[
    ("geolocation", "use", "your location"),
    ("camera", "use", "your camera"),
    ("cameraPanTiltZoom", "use", "your camera"),
    ("microphone", "use", "your microphone"),
    ("notifications", "send", "you notifications"),
    ("clipboard", "read", "your clipboard"),
    ("midiSysex", "use", "your MIDI devices"),
    ("sensors", "use", "your device sensors"),
    ("idleDetection", "see", "when you are away"),
    ("windowManagement", "manage", "windows on your screens"),
    ("localFonts", "see", "the fonts on this device"),
    ("fileSystemAccess", "use", "files on this device"),
    ("storageAccess", "use", "its cookies on this site"),
    ("topLevelStorageAccess", "use", "its cookies on this site"),
    ("protectedMediaIdentifier", "play", "protected media"),
    ("multipleDownloads", "download", "several files"),
    ("pointerLock", "take", "control of your pointer"),
    ("keyboardLock", "take", "control of your keyboard"),
    ("vrSession", "start", "a virtual reality session"),
    ("arSession", "start", "an augmented reality session"),
    ("handTracking", "track", "your hands"),
    ("localNetwork", "reach", "devices on your network"),
    ("localNetworkAccess", "reach", "devices on your network"),
    ("loopbackNetwork", "reach", "software on this device"),
    ("registerProtocolHandler", "open", "links of its own kind"),
    ("webAppInstallation", "install", "itself as an app"),
    ("diskQuota", "store", "more data on this device"),
    ("capturedSurfaceControl", "control", "the screen it is sharing"),
    ("identityProvider", "sign", "you in"),
];

/// A type with no sentence of its own is spaced out from its camelCase name, so the prompt
/// still says something true.
fn phrase(kind: &str) -> (String, String) {
    if let Some((_, verb, noun)) = PHRASES.iter().find(|(k, _, _)| *k == kind) {
        return ((*verb).to_owned(), (*noun).to_owned());
    }
    let mut words = String::new();
    let mut prev: Option<char> = None;
    for c in kind.chars() {
        if c.is_ascii_uppercase() && prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit()) {
            words.push(' ');
        }
        words.push(c);
        prev = Some(c);
    }
    ("use".into(), words.to_lowercase())
}

const EXTERNAL_PREFIX: &str = "externalProtocol:";

/// A site allowed to open another application's links (`mailto:`, `zoommtg:`) without asking
/// is kept beside its permissions, one type per scheme.
pub fn external_protocol_type(scheme: &str) -> String {
    format!("{EXTERNAL_PREFIX}{scheme}")
}

/// The permission's own name, for a row listing what a site has been given.
pub fn permission_name(kind: &str) -> String {
    if let Some(scheme) = kind.strip_prefix(EXTERNAL_PREFIX) {
        return format!("Open {scheme} links");
    }
    let (_, noun) = phrase(kind);
    let noun =
        ["your ", "you ", "its ", "the ", "a ", "an "].iter().find_map(|p| noun.strip_prefix(p)).unwrap_or(&noun);
    let mut chars = noun.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn join_clauses(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

/// One sentence however many types the request carries. Types sharing a verb share it ("use
/// your camera and your microphone"); the others get a clause each.
pub fn permission_sentence(host: &str, types: &[String]) -> String {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for kind in types {
        let (verb, noun) = phrase(kind);
        match groups.last_mut() {
            Some((last, nouns)) if *last == verb => nouns.push(noun),
            _ => groups.push((verb, vec![noun])),
        }
    }
    let who = if host.is_empty() { "This page" } else { host };
    if groups.is_empty() {
        return format!("{who} wants to use a feature that needs your permission.");
    }
    let clauses: Vec<String> = groups.iter().map(|(verb, nouns)| format!("{verb} {}", join_clauses(nouns))).collect();
    format!("{who} wants to {}.", join_clauses(&clauses))
}

/// Chromium reports an origin with a trailing slash (`http://host:port/`) while a URL's origin
/// has none, and the two have to key the same decision.
pub fn normalize_origin(origin: &str) -> &str {
    origin.trim_end_matches('/')
}

/// The answer on record, or `None` when any of the request's types was never decided. A
/// mixed request is asked again rather than half answered: one verdict answers the whole id.
pub fn remembered_decision(saved: &SitePermissions, origin: &str, types: &[String]) -> Option<PermissionDecision> {
    let for_origin = saved.get(normalize_origin(origin))?;
    let mut answer = None;
    for kind in types {
        let decision = *for_origin.get(kind)?;
        if answer.is_some_and(|a| a != decision) {
            return Some(PermissionDecision::Block);
        }
        answer = Some(decision);
    }
    answer
}

pub fn remember_decision(saved: &mut SitePermissions, origin: &str, types: &[String], decision: PermissionDecision) {
    let for_origin = saved.entry(normalize_origin(origin).to_owned()).or_default();
    for kind in types {
        for_origin.insert(kind.clone(), decision);
    }
}

pub fn forget_origin(saved: &mut SitePermissions, origin: &str) {
    saved.remove(normalize_origin(origin));
}

/// What a site has been given, in a stable order so the list does not reshuffle as decisions
/// are added.
pub fn decisions_for(saved: &SitePermissions, origin: &str) -> Vec<(String, PermissionDecision)> {
    let mut out: Vec<(String, PermissionDecision)> = saved
        .get(normalize_origin(origin))
        .map(|m| m.iter().map(|(k, v)| (k.clone(), *v)).collect())
        .unwrap_or_default();
    out.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()).then_with(|| a.0.cmp(&b.0)));
    out
}

/// The origin a decision is keyed by, empty for a view with no address (a new tab).
pub fn origin_of(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => normalize_origin(&u.origin().ascii_serialization()).to_owned(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_request_with_no_type_still_reads_as_a_sentence() {
        assert_eq!(
            permission_sentence("example.com", &split_types("")),
            "example.com wants to use a feature that needs your permission."
        );
        assert_eq!(permission_sentence("", &[]), "This page wants to use a feature that needs your permission.");
    }

    #[test]
    fn types_sharing_a_verb_share_it_an_unknown_type_is_spelled_out() {
        assert_eq!(
            permission_sentence("example.com", &types(&["camera", "microphone"])),
            "example.com wants to use your camera and your microphone."
        );
        assert_eq!(
            permission_sentence("example.com", &types(&["webPrinting"])),
            "example.com wants to use web printing."
        );
        assert_eq!(
            permission_sentence("a.test", &types(&["camera", "notifications", "geolocation"])),
            "a.test wants to use your camera, send you notifications and use your location."
        );
    }

    #[test]
    fn names_and_remembered_decisions() {
        assert_eq!(permission_name("camera"), "Camera");
        assert_eq!(permission_name("notifications"), "Notifications");
        assert_eq!(permission_name(&external_protocol_type("mailto")), "Open mailto links");
        let mut saved = SitePermissions::new();
        remember_decision(&mut saved, "https://a.test/", &types(&["camera"]), PermissionDecision::Allow);
        assert_eq!(remembered_decision(&saved, "https://a.test", &types(&["camera"])), Some(PermissionDecision::Allow));
        assert_eq!(remembered_decision(&saved, "https://a.test", &types(&["camera", "microphone"])), None);
        remember_decision(&mut saved, "https://a.test", &types(&["microphone"]), PermissionDecision::Block);
        assert_eq!(
            remembered_decision(&saved, "https://a.test", &types(&["camera", "microphone"])),
            Some(PermissionDecision::Block)
        );
        assert_eq!(decisions_for(&saved, "https://a.test/").len(), 2);
        forget_origin(&mut saved, "https://a.test/");
        assert!(saved.is_empty());
        assert_eq!(origin_of("https://a.test:8443/x"), "https://a.test:8443");
        assert_eq!(origin_of(""), "");
    }
}
