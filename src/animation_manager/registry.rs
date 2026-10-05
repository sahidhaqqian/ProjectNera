//! Filesystem-backed animation catalogue. Keys are paths relative to `assets/animations`.
//! The manager owns only its bookmark metadata; production clip registration is untouched.
use std::{
    collections::{BTreeMap, BTreeSet},
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
    origins: BTreeMap<String, String>,
    pub(crate) dance_pool: crate::dance_pool::DancePool,
    dance_file: PathBuf,
}

impl AnimationRegistry {
    pub fn new(root: PathBuf, bookmark_file: PathBuf) -> Self {
        let mut bookmarks = BTreeSet::new();
        let mut origins = BTreeMap::new();
        for line in fs::read_to_string(&bookmark_file)
            .unwrap_or_default()
            .lines()
        {
            let (key, origin) = line.split_once('\t').unwrap_or((line, ""));
            if valid_relative_key(key) {
                bookmarks.insert(key.to_owned());
                if valid_relative_key(origin) {
                    origins.insert(key.to_owned(), origin.to_owned());
                }
            }
        }
        let dance_file = bookmark_file.with_file_name("dance_pool.json");
        let mut registry = Self {
            dance_pool: crate::dance_pool::DancePool::load(&dance_file),
            dance_file,
            root,
            bookmark_file,
            entries: Vec::new(),
            bookmarks,
            origins,
        };
        registry.refresh();
        // Older versions stored only a bookmark flag. Move those files into the
        // dedicated directory when the manager starts, retaining their origin.
        for key in registry.bookmarks.clone() {
            if !key.starts_with("Bookmarks/") {
                if let Err(error) = registry.move_into_bookmarks(&key) {
                    eprintln!("Could not migrate animation bookmark {key}: {error}");
                }
            }
        }
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
            super::base::base_order(&a.key)
                .unwrap_or(usize::MAX)
                .cmp(&super::base::base_order(&b.key).unwrap_or(usize::MAX))
                .then_with(|| {
                    a.display_name
                        .to_ascii_lowercase()
                        .cmp(&b.display_name.to_ascii_lowercase())
                })
                .then(a.key.cmp(&b.key))
        });
        let known: BTreeSet<_> = self
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect();
        self.bookmarks
            .retain(|key| known.contains(key.as_str()) && !super::base::is_base_animation(key));
        for key in known {
            if key.starts_with("Bookmarks/") {
                self.bookmarks.insert(key.to_owned());
            }
        }
        self.origins.retain(|key, _| self.bookmarks.contains(key));
    }

    pub fn path(&self, key: &str) -> Option<PathBuf> {
        self.entries
            .iter()
            .any(|entry| entry.key == key)
            .then(|| self.root.join(key))
    }

    pub fn toggle_dance(&mut self, key: &str) -> io::Result<()> {
        if self.path(key).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Animation not found",
            ));
        }
        let previous = self.dance_pool.clone();
        if self.dance_pool.contains(key) {
            self.dance_pool.remove(key);
        } else {
            self.dance_pool.add(key);
        }
        if let Err(error) = self.dance_pool.save(&self.dance_file) {
            self.dance_pool = previous;
            return Err(error);
        }
        Ok(())
    }
    fn remap_dance(&mut self, old: &str, new: &str) -> io::Result<()> {
        if self.dance_pool.contains(old) {
            self.dance_pool.remap(old, new);
            self.dance_pool.save(&self.dance_file)?;
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn toggle_bookmark(&mut self, key: &str) -> io::Result<String> {
        ensure_editable(key)?;
        if self.path(key).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Animation not found",
            ));
        }
        if self.bookmarks.contains(key) {
            if !key.starts_with("Bookmarks/") {
                self.bookmarks.remove(key);
                self.save_bookmarks()?;
                return Ok(key.to_owned());
            }
            self.move_out_of_bookmarks(key)
        } else {
            self.move_into_bookmarks(key)
        }
    }

    fn move_into_bookmarks(&mut self, key: &str) -> io::Result<String> {
        let source = self
            .path(key)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Animation not found"))?;
        if key.starts_with("Bookmarks/") {
            return Ok(key.to_owned());
        }
        if is_runtime_animation(key) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "This animation is used by Nera and cannot be moved",
            ));
        }
        let directory = self.root.join("Bookmarks");
        fs::create_dir_all(&directory)?;
        let filename = source.file_name().unwrap().to_string_lossy();
        let stem = source.file_stem().unwrap().to_string_lossy();
        let mut destination = directory.join(filename.as_ref());
        let mut suffix = 2;
        while file_name_exists_case_insensitive(
            &directory,
            destination.file_name().unwrap().to_str().unwrap(),
        )? {
            destination = directory.join(format!("{stem} ({suffix}).vrma"));
            suffix += 1;
        }
        let new_key = format!(
            "Bookmarks/{}",
            destination.file_name().unwrap().to_string_lossy()
        );
        fs::rename(&source, &destination)?;
        let old_bookmarks = self.bookmarks.clone();
        let old_origins = self.origins.clone();
        self.bookmarks.remove(key);
        self.bookmarks.insert(new_key.clone());
        self.origins.insert(new_key.clone(), key.to_owned());
        if let Err(error) = self.save_bookmarks() {
            self.bookmarks = old_bookmarks;
            self.origins = old_origins;
            let _ = fs::rename(&destination, &source);
            return Err(error);
        }
        self.remap_dance(key, &new_key)?;
        self.refresh();
        Ok(new_key)
    }

    fn move_out_of_bookmarks(&mut self, key: &str) -> io::Result<String> {
        let source = self
            .path(key)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Animation not found"))?;
        let origin = self
            .origins
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.trim_start_matches("Bookmarks/").to_owned());
        if !valid_relative_key(&origin) || origin.starts_with("Bookmarks/") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid original animation path",
            ));
        }
        let destination = self.root.join(&origin);
        let directory = destination.parent().unwrap();
        fs::create_dir_all(directory)?;
        if file_name_exists_case_insensitive(
            directory,
            destination.file_name().unwrap().to_str().unwrap(),
        )? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Original animation filename is already in use",
            ));
        }
        fs::rename(&source, &destination)?;
        let old_bookmarks = self.bookmarks.clone();
        let old_origins = self.origins.clone();
        self.bookmarks.remove(key);
        self.origins.remove(key);
        if let Err(error) = self.save_bookmarks() {
            self.bookmarks = old_bookmarks;
            self.origins = old_origins;
            let _ = fs::rename(&destination, &source);
            return Err(error);
        }
        self.remap_dance(key, &origin)?;
        self.refresh();
        Ok(origin)
    }

    pub fn rename(&mut self, key: &str, proposed: &str) -> io::Result<String> {
        ensure_editable(key)?;
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
            if let Some(origin) = self.origins.remove(key) {
                let original_parent = Path::new(&origin).parent().unwrap_or(Path::new(""));
                let updated_origin = original_parent
                    .join(&name)
                    .to_string_lossy()
                    .replace('\\', "/");
                self.origins.insert(new_key.clone(), updated_origin);
            }
            self.save_bookmarks()?;
        }
        self.remap_dance(key, &new_key)?;
        self.refresh();
        Ok(new_key)
    }

    pub fn delete(&mut self, key: &str) -> io::Result<()> {
        ensure_editable(key)?;
        let path = self
            .path(key)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Animation not found"))?;
        fs::remove_file(path)?;
        self.dance_pool.remove(key);
        self.dance_pool.save(&self.dance_file)?;
        self.bookmarks.remove(key);
        self.origins.remove(key);
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
            if let Some(origin) = self.origins.get(key) {
                writeln!(output, "{key}\t{origin}")?;
            } else {
                writeln!(output, "{key}")?;
            }
        }
        output.sync_all()?;
        fs::rename(temporary, &self.bookmark_file)
    }
}

fn valid_relative_key(key: &str) -> bool {
    !key.is_empty()
        && key.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part
                    .chars()
                    .any(|c| c.is_control() || "<>:\"\\|?*".contains(c))
        })
        && key.to_ascii_lowercase().ends_with(".vrma")
}

fn file_name_exists_case_insensitive(directory: &Path, filename: &str) -> io::Result<bool> {
    Ok(fs::read_dir(directory)?
        .filter_map(Result::ok)
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(filename)
        }))
}

fn is_runtime_animation(key: &str) -> bool {
    super::base::is_base_animation(key)
}

fn ensure_editable(key: &str) -> io::Result<()> {
    if is_runtime_animation(key) {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Animasi Dasar belongs to Nera and cannot be renamed, deleted, or moved",
        ))
    } else {
        Ok(())
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
    fn dance_pool_references_existing_files_and_follows_bookmark_rename_delete() {
        let root = std::env::temp_dir().join(format!("nera-dance-registry-{}", std::process::id()));
        fs::create_dir_all(root.join("assets/Imported")).unwrap();
        fs::write(root.join("assets/Imported/user-choice.vrma"), b"fixture").unwrap();
        let metadata = root.join("bookmarks.txt");
        let mut registry = AnimationRegistry::new(root.join("assets"), metadata.clone());
        assert!(registry.toggle_dance("missing.vrma").is_err());
        registry.toggle_dance("Imported/user-choice.vrma").unwrap();
        assert_eq!(registry.entries.len(), 1);
        let key = registry
            .toggle_bookmark("Imported/user-choice.vrma")
            .unwrap();
        assert_eq!(registry.dance_pool.keys, vec![key.clone()]);
        let key = registry.rename(&key, "selected-renamed").unwrap();
        let restored = AnimationRegistry::new(root.join("assets"), metadata);
        assert_eq!(restored.dance_pool.keys, vec![key.clone()]);
        assert_eq!(restored.entries.len(), 1);
        registry.toggle_dance(&key).unwrap();
        assert!(registry.dance_pool.keys.is_empty());
        registry.toggle_dance(&key).unwrap();
        registry.delete(&key).unwrap();
        assert!(registry.dance_pool.keys.is_empty());
        fs::remove_file(root.join("dance_pool.json")).unwrap();
        fs::remove_file(root.join("bookmarks.txt")).unwrap();
        fs::remove_dir(root.join("assets/Imported")).unwrap();
        fs::remove_dir(root.join("assets/Bookmarks")).unwrap();
        fs::remove_dir(root.join("assets")).unwrap();
        fs::remove_dir(root).unwrap();
    }

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
        let bookmarked = registry.toggle_bookmark("Idle.vrma").unwrap();
        assert_eq!(bookmarked, "Bookmarks/Idle.vrma");
        assert!(registry.rename(&bookmarked, "other").is_err());
        let key = registry.rename(&bookmarked, "New_Idle").unwrap();
        assert_eq!(key, "Bookmarks/New_Idle.vrma");
        assert!(registry.bookmarks.contains(&key));
        assert!(root.join("assets/Bookmarks/New_Idle.vrma").exists());
        let restored = registry.toggle_bookmark(&key).unwrap();
        assert_eq!(restored, "New_Idle.vrma");
        assert!(root.join("assets/New_Idle.vrma").exists());
        let key = registry.toggle_bookmark(&restored).unwrap();
        registry.delete(&key).unwrap();
        assert!(!registry.bookmarks.contains(&key));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_bookmarks_move_to_folder_and_restore_original_paths() {
        let root =
            std::env::temp_dir().join(format!("nera-bookmark-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("assets/Mixamo")).unwrap();
        fs::write(root.join("assets/Mixamo/Walk.vrma"), b"vrma").unwrap();
        fs::write(root.join("bookmarks.txt"), b"Mixamo/Walk.vrma\n").unwrap();
        let mut registry = AnimationRegistry::new(root.join("assets"), root.join("bookmarks.txt"));
        assert!(root.join("assets/Bookmarks/Walk.vrma").exists());
        assert!(!root.join("assets/Mixamo/Walk.vrma").exists());
        assert!(registry.bookmarks.contains("Bookmarks/Walk.vrma"));
        assert_eq!(
            registry.toggle_bookmark("Bookmarks/Walk.vrma").unwrap(),
            "Mixamo/Walk.vrma"
        );
        assert!(root.join("assets/Mixamo/Walk.vrma").exists());
        assert!(registry.bookmarks.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bookmark_collision_gets_distinct_name_and_restores_source() {
        let root =
            std::env::temp_dir().join(format!("nera-bookmark-collision-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("assets/A")).unwrap();
        fs::create_dir_all(root.join("assets/B")).unwrap();
        fs::write(root.join("assets/A/Walk.vrma"), b"a").unwrap();
        fs::write(root.join("assets/B/Walk.vrma"), b"b").unwrap();
        let mut registry = AnimationRegistry::new(root.join("assets"), root.join("bookmarks.txt"));
        assert_eq!(
            registry.toggle_bookmark("A/Walk.vrma").unwrap(),
            "Bookmarks/Walk.vrma"
        );
        let second = registry.toggle_bookmark("B/Walk.vrma").unwrap();
        assert_eq!(second, "Bookmarks/Walk (2).vrma");
        let mut registry = AnimationRegistry::new(root.join("assets"), root.join("bookmarks.txt"));
        fs::write(root.join("assets/B/Walk.vrma"), b"conflict").unwrap();
        assert!(registry.toggle_bookmark(&second).is_err());
        assert!(root.join("assets/Bookmarks/Walk (2).vrma").exists());
        fs::remove_file(root.join("assets/B/Walk.vrma")).unwrap();
        assert_eq!(registry.toggle_bookmark(&second).unwrap(), "B/Walk.vrma");
        assert_eq!(fs::read(root.join("assets/B/Walk.vrma")).unwrap(), b"b");
        fs::remove_dir_all(root).unwrap();
    }
}
