//! `$DOVE_HOME/cloud-keys.json`: one API key per cloud provider, readable
//! only by this user. Never in `dove.db`, settings, transcripts or anything
//! synced, and never in an agent's environment.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Keys {
    #[serde(default)]
    keys: BTreeMap<String, String>,
}

// Keys never reach logs through `{:?}`.
impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keys").field("providers", &self.keys.keys().collect::<Vec<_>>()).finish()
    }
}

impl Keys {
    pub fn path() -> PathBuf {
        super::dove_home().join("cloud-keys.json")
    }

    /// The saved keys; empty when the file is missing or unreadable.
    pub fn load() -> Self {
        std::fs::read(Self::path()).ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default()
    }

    pub fn get(&self, provider: &str) -> Option<&str> {
        self.keys.get(provider).map(String::as_str).filter(|k| !k.is_empty())
    }

    pub fn has(&self, provider: &str) -> bool {
        self.get(provider).is_some()
    }

    /// Set (or, with an empty key, remove) `provider`'s key in memory.
    pub fn set(&mut self, provider: &str, key: &str) {
        let key = key.trim();
        if key.is_empty() {
            self.keys.remove(provider);
        } else {
            self.keys.insert(provider.to_string(), key.to_string());
        }
    }

    /// Write the file (0600).
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        super::write_private(&path, &serde_json::to_vec_pretty(self)?)
    }

    /// Load, set one key, save.
    pub fn store(provider: &str, key: &str) -> Result<()> {
        let mut keys = Self::load();
        keys.set(provider, key);
        keys.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_trims_and_empty_removes() {
        let mut k = Keys::default();
        k.set("instabox", "  ibk_abc \n");
        assert_eq!(k.get("instabox"), Some("ibk_abc"));
        k.set("instabox", "");
        assert!(!k.has("instabox"));
    }

    #[test]
    fn debug_never_prints_keys() {
        let mut k = Keys::default();
        k.set("instabox", "ibk_secret");
        assert!(!format!("{k:?}").contains("ibk_secret"));
    }

    #[test]
    fn round_trips_through_json() {
        let mut k = Keys::default();
        k.set("e2b", "e2b_x");
        let back: Keys = serde_json::from_slice(&serde_json::to_vec(&k).unwrap()).unwrap();
        assert_eq!(back.get("e2b"), Some("e2b_x"));
    }
}
