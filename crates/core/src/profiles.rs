//! `profiles.json` at the app data root: the profiles there are, their names, which had windows
//! open at quit, and the directories a deleted profile left behind. Where each profile keeps its
//! files is decided here too. It opens no window and starts no Chromium context.
//!
//! The profile that existed before profiles did is [`DEFAULT_ID`]: its files are the root's own
//! and its Chromium store is `cef/Default`, so it needs no migration. Every other profile keeps
//! its files in `profiles/<id>/` and its Chromium store in `cef/profile-<id>/`, which nd-cef
//! derives from the id. Ids are `p` and a number, so a name never reaches a path.

use std::path::{Path, PathBuf};

use nd_settings::Store;
use serde::{Deserialize, Serialize};

use crate::{lenient_items, null_default};

pub const DEFAULT_ID: &str = "default";
pub const DEFAULT_NAME: &str = "Default";
/// Longer names are refused; the menu truncates what it cannot show.
pub const MAX_NAME_CHARS: usize = 100;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Profile {
    #[serde(deserialize_with = "null_default")]
    pub id: String,
    #[serde(deserialize_with = "null_default")]
    pub name: String,
    /// It had windows open as the app quit, so the next launch brings them back.
    #[serde(deserialize_with = "null_default")]
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProfilesState {
    /// `Default` first, then in the order they were made.
    #[serde(deserialize_with = "lenient_items")]
    pub profiles: Vec<Profile>,
    #[serde(deserialize_with = "null_default")]
    pub next_id: u64,
    /// Directories of deleted profiles that could not be removed while Chromium held them,
    /// removed at the next launch before Chromium starts.
    #[serde(deserialize_with = "lenient_items")]
    pub leftovers: Vec<PathBuf>,
}

impl Default for ProfilesState {
    fn default() -> Self {
        Self {
            profiles: vec![Profile { id: DEFAULT_ID.into(), name: DEFAULT_NAME.into(), open: true }],
            next_id: 2,
            leftovers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileError {
    EmptyName,
    NameTooLong,
    /// Another profile has this name, compared without case.
    NameTaken,
    UnknownProfile,
    /// `Default` holds the files that were there before profiles; it is renamed, never deleted.
    DefaultStays,
}

impl ProfileError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::EmptyName => "A profile needs a name.",
            Self::NameTooLong => "That name is too long.",
            Self::NameTaken => "Another profile has that name.",
            Self::UnknownProfile => "That profile no longer exists.",
            Self::DefaultStays => "The Default profile cannot be deleted.",
        }
    }
}

/// `profiles.json` in `root`, with `Default` always present and first and every id once.
pub fn open(root: impl Into<PathBuf>) -> Store<ProfilesState> {
    let store = Store::<ProfilesState>::open(root.into().join("profiles.json"), 1, None);
    let loaded = store.get();
    let normal = normalize(&loaded);
    if normal != *loaded {
        store.update(|s| *s = normal);
    }
    store
}

fn normalize(state: &ProfilesState) -> ProfilesState {
    let mut out = ProfilesState { profiles: Vec::new(), ..state.clone() };
    let default = state.profiles.iter().find(|p| p.id == DEFAULT_ID);
    out.profiles.push(match default {
        Some(p) => {
            Profile { name: if p.name.trim().is_empty() { DEFAULT_NAME.into() } else { p.name.clone() }, ..p.clone() }
        }
        None => ProfilesState::default().profiles.remove(0),
    });
    for p in &state.profiles {
        let well_formed = number(&p.id).is_some() && !p.name.trim().is_empty();
        if well_formed && !out.profiles.iter().any(|q| q.id == p.id) {
            out.profiles.push(p.clone());
        }
    }
    let highest = out.profiles.iter().filter_map(|p| number(&p.id)).max().unwrap_or(1);
    out.next_id = out.next_id.max(highest + 1);
    if !out.profiles.iter().any(|p| p.open) {
        out.profiles[0].open = true;
    }
    out
}

fn number(id: &str) -> Option<u64> {
    id.strip_prefix('p')?.parse().ok()
}

/// A name as the user typed it, trimmed, or why it cannot be used. `except` is the profile
/// being renamed, whose own name is no clash.
fn check_name(state: &ProfilesState, name: &str, except: Option<&str>) -> Result<String, ProfileError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ProfileError::EmptyName);
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ProfileError::NameTooLong);
    }
    let lower = name.to_lowercase();
    if state.profiles.iter().any(|p| Some(p.id.as_str()) != except && p.name.to_lowercase() == lower) {
        return Err(ProfileError::NameTaken);
    }
    Ok(name.to_owned())
}

impl ProfilesState {
    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }

    /// A new profile, closed until a window opens in it. Returns its id.
    pub fn create(&mut self, name: &str) -> Result<String, ProfileError> {
        let name = check_name(self, name, None)?;
        let id = format!("p{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.profiles.push(Profile { id: id.clone(), name, open: false });
        Ok(id)
    }

    pub fn rename(&mut self, id: &str, name: &str) -> Result<(), ProfileError> {
        let name = check_name(self, name, Some(id))?;
        let p = self.profiles.iter_mut().find(|p| p.id == id).ok_or(ProfileError::UnknownProfile)?;
        p.name = name;
        Ok(())
    }

    /// Takes the profile out of the list and returns the directories to remove.
    pub fn delete(&mut self, root: &Path, id: &str) -> Result<Vec<PathBuf>, ProfileError> {
        if id == DEFAULT_ID {
            return Err(ProfileError::DefaultStays);
        }
        let at = self.profiles.iter().position(|p| p.id == id).ok_or(ProfileError::UnknownProfile)?;
        self.profiles.remove(at);
        if !self.profiles.iter().any(|p| p.open) {
            self.profiles[0].open = true;
        }
        Ok(vec![data_dir(root, id), chromium_dir(root, id)])
    }

    pub fn set_open(&mut self, id: &str, open: bool) {
        if let Some(p) = self.profiles.iter_mut().find(|p| p.id == id) {
            p.open = open;
        }
    }

    /// The profiles whose windows a launch brings back, `Default` first.
    pub fn open_ids(&self) -> Vec<&str> {
        self.profiles.iter().filter(|p| p.open).map(|p| p.id.as_str()).collect()
    }
}

/// Where `id` keeps the files core writes: its session, history, favicons, bookmarks.
pub fn data_dir(root: &Path, id: &str) -> PathBuf {
    if id == DEFAULT_ID { root.to_path_buf() } else { root.join("profiles").join(id) }
}

/// The name nd-cef keeps `id`'s Chromium store under (`cef/profile-<name>`), `None` for
/// `Default`, which is Chromium's own default profile.
pub fn chromium_name(id: &str) -> Option<&str> {
    (id != DEFAULT_ID).then_some(id)
}

fn chromium_dir(root: &Path, id: &str) -> PathBuf {
    root.join("cef").join(format!("profile-{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_install_and_a_profile_from_before_profiles_both_have_default_open() {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path());
        assert_eq!(store.get().open_ids(), [DEFAULT_ID]);
        assert_eq!(data_dir(dir.path(), DEFAULT_ID), dir.path(), "Default's files stay at the root");
        assert_eq!(chromium_name(DEFAULT_ID), None);
    }

    #[test]
    fn names_are_trimmed_unique_and_bounded() {
        let mut s = ProfilesState::default();
        let lead = "\u{1F680} Work and play ";
        let long = lead.to_owned() + &"x".repeat(80 - lead.chars().count());
        assert_eq!(long.chars().count(), 80);
        let id = s.create(&format!("  {long}  ")).unwrap();
        assert_eq!(id, "p2");
        assert_eq!(s.get(&id).unwrap().name, long.trim());
        assert_eq!(s.create("default"), Err(ProfileError::NameTaken));
        assert_eq!(s.create("   "), Err(ProfileError::EmptyName));
        assert_eq!(s.create(&"y".repeat(MAX_NAME_CHARS + 1)), Err(ProfileError::NameTooLong));
        assert!(s.create(&"y".repeat(MAX_NAME_CHARS)).is_ok());
        s.rename(&id, "Work").unwrap();
        s.rename(&id, "WORK").unwrap();
        assert_eq!(s.rename("p9", "x"), Err(ProfileError::UnknownProfile));
        assert!(!s.get(&id).unwrap().open, "a new profile opens with its first window");
        assert_eq!(chromium_name(&id), Some("p2"));
    }

    #[test]
    fn delete_names_both_directories_and_keeps_default() {
        let root = Path::new("/data/lynk");
        let mut s = ProfilesState::default();
        let id = s.create("Work").unwrap();
        s.set_open(&id, true);
        s.set_open(DEFAULT_ID, false);
        assert_eq!(s.delete(root, DEFAULT_ID), Err(ProfileError::DefaultStays));
        let dirs = s.delete(root, &id).unwrap();
        assert_eq!(dirs, [PathBuf::from("/data/lynk/profiles/p2"), PathBuf::from("/data/lynk/cef/profile-p2")]);
        assert_eq!(s.open_ids(), [DEFAULT_ID], "deleting the only open profile leaves Default to open");
        assert_eq!(s.create("Next").unwrap(), "p3", "an id is never handed out twice");
    }

    #[test]
    fn a_hand_edited_file_keeps_what_it_can() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("profiles.json"),
            r#"{"version":1,"data":{"profiles":[{"id":"p7","name":"Work","open":false},null,
               {"id":"p7","name":"Twice"},{"id":"../x","name":"Bad"},{"id":"p8","name":" "}],"nextId":null}}"#,
        )
        .unwrap();
        let s = open(dir.path()).get().as_ref().clone();
        let ids: Vec<&str> = s.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, [DEFAULT_ID, "p7"]);
        assert_eq!(s.next_id, 8);
        assert_eq!(s.open_ids(), [DEFAULT_ID]);
    }
}
