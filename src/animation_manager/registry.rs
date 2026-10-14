//! Filesystem-backed animation catalogue. Keys are paths relative to `assets/animations`.
//! The manager owns only its bookmark metadata; production clip registration is untouched.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{self, Read, Write},
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
                    bevy::log::warn!("Could not migrate animation bookmark {key}: {error}");
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
        if !valid_relative_key(key) || !self.entries.iter().any(|entry| entry.key == key) {
            return None;
        }
        let path = self.root.join(key);
        let root = self.root.canonicalize().ok()?;
        let actual = path.canonicalize().ok()?;
        let relative = actual
            .strip_prefix(&root)
            .ok()?
            .to_string_lossy()
            .replace('\\', "/");
        if is_runtime_animation(&relative) && !is_runtime_animation(key) {
            return None;
        }
        Some(path)
    }

    fn create_owned_directory(&self, directory: &Path) -> io::Result<()> {
        let root = self.root.canonicalize()?;
        let mut existing = directory;
        while !existing.exists() {
            existing = existing.parent().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Animation directory is outside the library",
                )
            })?;
        }
        if !existing.canonicalize()?.starts_with(&root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Linked directory is outside the animation library",
            ));
        }
        fs::create_dir_all(directory)?;
        if !directory.canonicalize()?.starts_with(root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Animation directory escaped the library",
            ));
        }
        Ok(())
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
        let previous = self.dance_pool.clone();
        if self.dance_pool.contains(old) {
            self.dance_pool.remap(old, new);
            if let Err(error) = self.dance_pool.save(&self.dance_file) {
                self.dance_pool = previous;
                return Err(error);
            }
        }
        Ok(())
    }

    /// Compensate an already-completed file move when metadata cannot be saved.
    fn persist_move(
        &mut self,
        source: &Path,
        destination: &Path,
        old_key: &str,
        new_key: &str,
        old_bookmarks: BTreeSet<String>,
        old_origins: BTreeMap<String, String>,
    ) -> io::Result<()> {
        let old_dance = self.dance_pool.clone();
        let bookmarks_changed = self.bookmarks != old_bookmarks || self.origins != old_origins;
        let saved = if bookmarks_changed {
            self.save_bookmarks()
        } else {
            Ok(())
        }
        .and_then(|_| self.remap_dance(old_key, new_key));
        if let Err(error) = saved {
            match fs::rename(destination, source) {
                Ok(()) => {
                    self.bookmarks = old_bookmarks;
                    self.origins = old_origins;
                    self.dance_pool = old_dance;
                    if bookmarks_changed && let Err(recovery) = self.save_bookmarks() {
                        bevy::log::warn!(
                            "Animation move rolled back, but bookmark metadata recovery failed: {recovery}"
                        );
                    }
                }
                Err(recovery) => {
                    // The actual moved file remains authoritative. Reflect it
                    // in the live catalog even if the disk is no longer writable.
                    self.dance_pool.remap(old_key, new_key);
                    bevy::log::warn!(
                        "Animation metadata failed: {error}; file rollback also failed: {recovery}"
                    );
                }
            }
            self.refresh();
            return Err(error);
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
        self.create_owned_directory(&directory)?;
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
        self.persist_move(
            &source,
            &destination,
            key,
            &new_key,
            old_bookmarks,
            old_origins,
        )?;
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
        self.create_owned_directory(directory)?;
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
        self.persist_move(
            &source,
            &destination,
            key,
            &origin,
            old_bookmarks,
            old_origins,
        )?;
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
        let old_bookmarks = self.bookmarks.clone();
        let old_origins = self.origins.clone();
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
        }
        self.persist_move(
            &old,
            &destination,
            key,
            &new_key,
            old_bookmarks,
            old_origins,
        )?;
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
        self.bookmarks.remove(key);
        self.origins.remove(key);
        let dance_saved = self.dance_pool.save(&self.dance_file);
        let bookmarks_saved = self.save_bookmarks();
        self.refresh();
        dance_saved.and(bookmarks_saved)
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
        let mut bytes = Vec::new();
        fs::File::open(source)?
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "VRMA exceeds the 16 MiB playback limit",
            ));
        }
        crate::animation::validate_vrma_import(&bytes)
            .map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason))?;
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
        self.create_owned_directory(&directory)?;
        let destination = directory.join(name);
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)?;
        if let Err(error) = output.write_all(&bytes).and_then(|_| output.sync_all()) {
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
    let Ok(root) = directory.canonicalize() else {
        return;
    };
    let mut pending = vec![directory.to_owned()];
    let mut visited = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        let Ok(actual) = directory.canonicalize() else {
            continue;
        };
        if !actual.starts_with(&root) || !visited.insert(actual) {
            continue;
        }
        let Ok(items) = fs::read_dir(directory) else {
            continue;
        };
        for item in items.flatten() {
            let Ok(kind) = item.file_type() else { continue };
            // Linked collections are not owned imports; never recursively
            // traverse them or expose their originals to rename/delete.
            if kind.is_symlink() {
                continue;
            }
            let path = item.path();
            if kind.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("vrma"))
                && path.canonicalize().is_ok_and(|p| p.starts_with(&root))
            {
                paths.push(path);
            }
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

    #[cfg(windows)]
    #[test]
    fn junctions_cannot_expose_or_modify_originals_outside_the_library() {
        let base = std::env::temp_dir().join(format!("nera-junction-audit-{}", std::process::id()));
        let root = base.join("assets");
        let outside = base.join("originals");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let original = outside.join("keep.vrma");
        fs::write(&original, b"original bytes").unwrap();
        let link = root.join("Imported");
        let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', "''"));
        let command = format!(
            "New-Item -ItemType Junction -Path {} -Target {} | Out-Null",
            quote(&link),
            quote(&outside)
        );
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &command])
            .status()
            .unwrap();
        assert!(
            status.success(),
            "Native NTFS junction fixture must be created"
        );
        let mut registry = AnimationRegistry::new(root.clone(), base.join("bookmarks.txt"));
        assert!(
            registry.entries.is_empty(),
            "Linked originals must not enter the mutable catalogue"
        );
        registry.entries.push(AnimationEntry {
            key: "Imported/keep.vrma".into(),
            display_name: "keep".into(),
            source: "Imported".into(),
            emotion: "Neutral",
        });
        assert!(registry.delete("Imported/keep.vrma").is_err());
        assert!(registry.rename("Imported/keep.vrma", "renamed").is_err());
        registry.entries.clear();
        assert!(registry.import(&original).is_err());
        assert_eq!(fs::read(&original).unwrap(), b"original bytes");
        fs::remove_dir(link).unwrap(); // Remove the junction itself, never its target.
        fs::remove_file(original).unwrap();
        fs::remove_dir(outside).unwrap();
        fs::remove_dir(root).unwrap();
        fs::remove_dir(base).unwrap();
    }

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
    fn metadata_write_failure_rolls_back_moves_and_refreshes_an_irreversible_delete() {
        for operation in ["rename", "bookmark", "unbookmark", "delete"] {
            let root = std::env::temp_dir().join(format!(
                "nera-metadata-failure-{}-{operation}",
                std::process::id()
            ));
            fs::create_dir_all(root.join("assets")).unwrap();
            fs::write(root.join("assets/choice.vrma"), b"fixture").unwrap();
            let mut registry =
                AnimationRegistry::new(root.join("assets"), root.join("bookmarks.txt"));
            let key = if operation == "bookmark" {
                "choice.vrma".to_owned()
            } else {
                registry.toggle_bookmark("choice.vrma").unwrap()
            };
            registry.toggle_dance(&key).unwrap();
            let bookmarks = registry.bookmarks.clone();
            let pool = registry.dance_pool.clone();
            fs::write(root.join("blocked"), b"not a directory").unwrap();
            registry.dance_file = root.join("blocked/dance.json");
            let result = match operation {
                "rename" => registry.rename(&key, "renamed").map(|_| ()),
                "bookmark" | "unbookmark" => registry.toggle_bookmark(&key).map(|_| ()),
                _ => registry.delete(&key),
            };
            assert!(result.is_err());
            if operation == "delete" {
                assert!(registry.entries.is_empty());
                assert!(!registry.bookmarks.contains(&key));
                assert!(!registry.dance_pool.contains(&key));
                assert!(!root.join("assets").join(&key).exists());
            } else {
                assert!(root.join("assets").join(&key).is_file());
                assert_eq!(registry.entries[0].key, key);
                assert_eq!(registry.bookmarks, bookmarks);
                assert_eq!(registry.dance_pool, pool);
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn import_rejects_invalid_vrma_and_preserves_the_original_source() {
        let root =
            std::env::temp_dir().join(format!("nera-import-preflight-{}", std::process::id()));
        fs::create_dir_all(root.join("assets")).unwrap();
        let source = root.join("wrong.vrma");
        fs::write(&source, b"not a VRMA").unwrap();
        let mut registry = AnimationRegistry::new(root.join("assets"), root.join("bookmarks.txt"));
        assert!(registry.import(&source).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"not a VRMA");
        assert!(registry.entries.is_empty());
        let valid = root.join("valid.vrma");
        fs::copy(
            crate::nera_asset_file("animations/animasi_basic/idle.vrma"),
            &valid,
        )
        .unwrap();
        assert_eq!(registry.import(&valid).unwrap(), "Imported/valid.vrma");
        assert_eq!(
            fs::read(&valid).unwrap(),
            fs::read(root.join("assets/Imported/valid.vrma")).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
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
