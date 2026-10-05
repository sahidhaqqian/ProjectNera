//! Curated references to the existing animation registry; never copies VRMA files.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Resource, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DancePool {
    pub(crate) keys: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_are_unique_safe_and_persistent_without_asset_copies() {
        let root = std::env::temp_dir().join(format!("nera-dance-pool-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("pool.json");
        let mut pool = DancePool::default();
        assert!(pool.keys.is_empty());
        assert!(pool.add("Bookmarks/user-selected.vrma"));
        assert!(!pool.add("bookmarks/USER-selected.vrma"));
        assert!(!pool.add("../secret.vrma"));
        pool.save(&path).unwrap();
        assert_eq!(DancePool::load(&path), pool);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        pool.remap("Bookmarks/user-selected.vrma", "Bookmarks/renamed.vrma");
        assert_eq!(pool.keys, vec!["Bookmarks/renamed.vrma"]);
        pool.remove("Bookmarks/renamed.vrma");
        pool.save(&path).unwrap();
        assert!(DancePool::load(&path).keys.is_empty());
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn corrupt_missing_and_unsafe_configuration_falls_back_safely() {
        let path =
            std::env::temp_dir().join(format!("nera-dance-corrupt-{}.json", std::process::id()));
        fs::write(&path, "broken").unwrap();
        assert!(DancePool::load(&path).keys.is_empty());
        fs::write(&path, r#"{"keys":["../secret.vrma","a.vrma","A.vrma"]}"#).unwrap();
        assert_eq!(DancePool::load(&path).keys, vec!["a.vrma"]);
        fs::remove_file(&path).unwrap();
        assert!(DancePool::load(&path).keys.is_empty());
    }
}

impl DancePool {
    pub(crate) fn file() -> PathBuf {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("ProjectNera/dance_pool.json")
    }
    pub(crate) fn valid_key(key: &str) -> bool {
        key.len() <= 1024
            && !key.is_empty()
            && key.to_ascii_lowercase().ends_with(".vrma")
            && key.split('/').all(|part| {
                !part.is_empty()
                    && part != "."
                    && part != ".."
                    && !part
                        .chars()
                        .any(|c| c.is_control() || "<>:\"\\|?*".contains(c))
            })
    }
    pub(crate) fn load(path: &Path) -> Self {
        // Bound optional configuration before reading it, including corrupt files.
        if fs::metadata(path).is_ok_and(|m| m.len() > 256 * 1024) {
            return Self::default();
        }
        let mut pool: Self = fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        pool.keys
            .retain(|k| Self::valid_key(k) && seen.insert(k.to_ascii_lowercase()));
        pool.keys.truncate(256);
        pool
    }
    pub(crate) fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("json.tmp");
        fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        fs::rename(temporary, path)
    }
    pub(crate) fn contains(&self, key: &str) -> bool {
        self.keys.iter().any(|k| k.eq_ignore_ascii_case(key))
    }
    pub(crate) fn add(&mut self, key: &str) -> bool {
        if !Self::valid_key(key) || self.contains(key) || self.keys.len() >= 256 {
            return false;
        }
        self.keys.push(key.into());
        true
    }
    pub(crate) fn remove(&mut self, key: &str) {
        self.keys.retain(|k| !k.eq_ignore_ascii_case(key));
    }
    pub(crate) fn remap(&mut self, old: &str, new: &str) {
        if self.contains(old) {
            self.remove(old);
            self.add(new);
        }
    }
}
