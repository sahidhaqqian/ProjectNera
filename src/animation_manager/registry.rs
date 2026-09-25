//! Filesystem-backed animation catalogue. Keys are paths relative to `assets/animations`.
//! The manager owns only its bookmark metadata; production clip registration is untouched.
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub(super) struct AnimationEntry {
    pub key: String,
    pub display_name: String,
    pub source: String,
    pub emotion: &'static str,
}

#[derive(Debug)]
pub(super) struct AnimationRegistry {
    root: PathBuf,
    bookmark_file: PathBuf,
    pub entries: Vec<AnimationEntry>,
    pub bookmarks: BTreeSet<String>,
}

impl AnimationRegistry {
    pub fn new(root: PathBuf, bookmark_file: PathBuf) -> Self {
        let bookmarks = fs::read_to_string(&bookmark_file)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        let mut registry = Self {
            root,
            bookmark_file,
            entries: Vec::new(),
            bookmarks,
        };
        registry.refresh();
        registry
    }

    pub fn refresh(&mut self) {
        let mut paths = Vec::new();
        collect_vrma(&self.root, &mut paths);
        self.entries = paths
            .into_iter()
            .filter_map(|path| {
                let relative = path.strip_prefix(&self.root).ok()?;
                let key = relative.to_string_lossy().replace('\\', "/");
                let stem = path.file_stem()?.to_string_lossy();
                let display_name = stem
                    .trim_start_matches(|c: char| c.is_ascii_digit() || c == '_')
                    .replace(['_', '-'], " ");
                let source = relative
                    .parent()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Nera".into());
                let lower = stem.to_ascii_lowercase();
                let emotion = if lower.contains("happy") || lower.contains("dance") {
                    "Happy"
                } else if lower.contains("sad") {
                    "Sad"
                } else if lower.contains("angry") {
                    "Angry"
                } else if lower.contains("sleep") {
                    "Sleepy"
                } else {
                    "Neutral"
                };
                Some(AnimationEntry {
                    key,
                    display_name,
                    source,
                    emotion,
                })
            })
            .collect();
        self.entries.sort_by(|a, b| {
            a.display_name
                .to_ascii_lowercase()
                .cmp(&b.display_name.to_ascii_lowercase())
                .then(a.key.cmp(&b.key))
        });
        let known: BTreeSet<_> = self
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect();
        self.bookmarks.retain(|key| known.contains(key.as_str()));
    }

    pub fn path(&self, key: &str) -> Option<PathBuf> {
        self.entries
            .iter()
            .any(|entry| entry.key == key)
            .then(|| self.root.join(key))
    }

    pub fn toggle_bookmark(&mut self, key: &str) -> io::Result<()> {
        if self.path(key).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Animation not found",
            ));
        }
        if !self.bookmarks.remove(key) {
            self.bookmarks.insert(key.to_owned());
        }
        self.save_bookmarks()
    }

    pub fn rename(&mut self, key: &str, proposed: &str) -> io::Result<String> {
        let old = self
            .path(key)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Animation not found"))?;
        let name = validated_vrma_name(proposed)?;
        if self.entries.iter().any(|entry| {
            entry.key != key
                && entry
                    .key
                    .rsplit('/')
                    .next()
                    .is_some_and(|existing| existing.eq_ignore_ascii_case(&name))
        }) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "An animation with that filename already exists",
            ));
        }
        let parent = old.parent().expect("catalogue file has a parent");
        let destination = parent.join(&name);
        let old_name = old.file_name().unwrap().to_string_lossy();
        if name.eq_ignore_ascii_case(&old_name) && name == old_name {
            return Ok(key.to_owned());
        }
        if fs::read_dir(parent)?.filter_map(Result::ok).any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(&name)
                && entry.path() != old
        }) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "An animation with that filename already exists",
            ));
        }
        fs::rename(&old, &destination)?;
        let new_key = destination
            .strip_prefix(&self.root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if self.bookmarks.remove(key) {
            self.bookmarks.insert(new_key.clone());
            self.save_bookmarks()?;
        }
        self.refresh();
        Ok(new_key)
    }

    pub fn delete(&mut self, key: &str) -> io::Result<()> {
        let path = self
            .path(key)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Animation not found"))?;
        fs::remove_file(path)?;
        self.bookmarks.remove(key);
        self.save_bookmarks()?;
        self.refresh();
        Ok(())
    }

    pub fn import(&mut self, source: &Path) -> io::Result<String> {
        if !source.is_file()
            || !source
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("vrma"))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Choose a .vrma file",
            ));
        }
        let name = validated_vrma_name(&source.file_name().unwrap().to_string_lossy())?;
        if self.entries.iter().any(|entry| {
            entry
                .key
                .rsplit('/')
                .next()
                .is_some_and(|existing| existing.eq_ignore_ascii_case(&name))
        }) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "An animation with that filename already exists",
            ));
        }
        let directory = self.root.join("Imported");
        fs::create_dir_all(&directory)?;
        let destination = directory.join(name);
        let mut input = fs::File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)?;
        if let Err(error) = io::copy(&mut input, &mut output) {
            drop(output);
            let _ = fs::remove_file(&destination);
            return Err(error);
        }
        self.refresh();
        Ok(destination
            .strip_prefix(&self.root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/"))
    }

    fn save_bookmarks(&self) -> io::Result<()> {
        if let Some(parent) = self.bookmark_file.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = self.bookmark_file.with_extension("tmp");
        let mut output = fs::File::create(&temporary)?;
        for key in &self.bookmarks {
            writeln!(output, "{key}")?;
        }
        output.sync_all()?;
        fs::rename(temporary, &self.bookmark_file)
    }
}

fn collect_vrma(directory: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(items) = fs::read_dir(directory) else {
        return;
    };
    for item in items.flatten() {
        let path = item.path();
        if path.is_dir() {
            collect_vrma(&path, paths);
        } else if path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("vrma"))
        {
            paths.push(path);
        }
    }
}

pub(super) fn validated_vrma_name(proposed: &str) -> io::Result<String> {
    let trimmed = proposed.trim();
    let stem = if trimmed.to_ascii_lowercase().ends_with(".vrma") {
        &trimmed[..trimmed.len() - 5]
    } else {
        trimmed
    };
    if stem.is_empty()
        || stem == "."
        || stem == ".."
        || stem.ends_with(['.', ' '])
        || stem.encode_utf16().count() > 250
        || stem
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(
            &stem
                .split('.')
                .next()
                .unwrap_or("")
                .to_ascii_uppercase()
                .as_str(),
        )
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Invalid Windows filename",
        ));
    }
    Ok(format!("{stem}.vrma"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_validation_rejects_windows_unsafe_names() {
        for name in ["", "a/b", "x?", "CON", "LPT1.txt", "a.", "a\\b"] {
            assert!(validated_vrma_name(name).is_err(), "{name}");
        }
        assert_eq!(
            validated_vrma_name("Dance_Happy").unwrap(),
            "Dance_Happy.vrma"
        );
    }

    #[test]
    fn physical_rename_preserves_bookmark_and_conflict_is_safe() {
        let root = std::env::temp_dir().join(format!("nera-registry-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(root.join("assets/Idle.vrma"), b"vrma").unwrap();
        fs::write(root.join("assets/Other.vrma"), b"vrma").unwrap();
        let mut registry = AnimationRegistry::new(root.join("assets"), root.join("bookmarks.txt"));
        registry.toggle_bookmark("Idle.vrma").unwrap();
        assert!(registry.rename("Idle.vrma", "other").is_err());
        let key = registry.rename("Idle.vrma", "New_Idle").unwrap();
        assert_eq!(key, "New_Idle.vrma");
        assert!(registry.bookmarks.contains(&key));
        assert!(root.join("assets/New_Idle.vrma").exists());
        registry.delete(&key).unwrap();
        assert!(!registry.bookmarks.contains(&key));
        fs::remove_dir_all(root).unwrap();
    }
}
