//! Content identity is separate from source names and VRM 0.x migration output.
use std::{fs::File, io::Read, path::Path};

use super::CharacterEntry;

pub(super) struct RegisteredImport {
    pub path: String,
    pub fingerprint: String,
    pub stored_fingerprint: String,
    pub source_path: String,
    pub duplicate: bool,
}

pub(super) fn fingerprint(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > super::super::MAX_MODEL_BYTES {
        return Err("Character exceeds the 128 MiB import limit.".into());
    }
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        size += count as u64;
        if size > super::super::MAX_MODEL_BYTES {
            return Err("Character exceeds the 128 MiB import limit.".into());
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

pub(super) fn import_registered(
    source: &Path,
    directory: &Path,
    entries: &[CharacterEntry],
) -> Result<RegisteredImport, String> {
    if !source
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("vrm"))
    {
        return Err("Choose a character .vrm file.".into());
    }
    let canonical = source.canonicalize().map_err(|e| e.to_string())?;
    let source_path = canonical.to_string_lossy().into_owned();
    let raw = fingerprint(&canonical)?;
    let reusable = |entry: &CharacterEntry| {
        if entry.path == super::DEFAULT_CHARACTER {
            super::super::preview_model_file(&entry.path).is_some_and(|path| path.is_file())
        } else {
            super::saved_filename(&entry.path).is_some_and(|file| directory.join(file).is_file())
        }
    };
    let duplicate = |entry: &CharacterEntry, stored: String| RegisteredImport {
        path: entry.path.clone(),
        fingerprint: raw.clone(),
        stored_fingerprint: stored,
        source_path: source_path.clone(),
        duplicate: true,
    };
    // Verify content even for a known canonical path: a file may have been replaced.
    if let Some(entry) = entries.iter().find(|entry| {
        reusable(entry)
            && entry.source_path.as_deref() == Some(&source_path)
            && entry.fingerprint == raw
    }) {
        return Ok(duplicate(entry, entry.stored_fingerprint.clone()));
    }
    if let Some(entry) = entries.iter().find(|entry| {
        reusable(entry) && (entry.fingerprint == raw || entry.stored_fingerprint == raw)
    }) {
        return Ok(duplicate(entry, entry.stored_fingerprint.clone()));
    }
    let migrated = super::super::read_valid_model(&canonical)?;
    let stored = blake3::hash(&migrated.bytes).to_hex().to_string();
    // Legacy entries only know the stored (converted) content, not the original VRM 0.x bytes.
    if let Some(entry) = entries
        .iter()
        .find(|entry| reusable(entry) && entry.stored_fingerprint == stored)
    {
        return Ok(duplicate(entry, stored));
    }
    let imported = super::super::store_imported(migrated, directory)?;
    Ok(RegisteredImport {
        path: imported.path,
        fingerprint: raw,
        stored_fingerprint: stored,
        source_path,
        duplicate: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renamed_content_and_canonical_path_reuse_without_copy_and_removal_allows_reimport() {
        let dir = std::env::temp_dir().join(format!("nera-content-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.vrm");
        std::fs::copy(
            crate::nera_asset_file(super::super::DEFAULT_CHARACTER),
            &source,
        )
        .unwrap();
        let first = import_registered(&source, &dir, &[]).unwrap();
        assert!(!first.duplicate);
        let entry = CharacterEntry {
            path: first.path.clone(),
            fingerprint: first.fingerprint,
            stored_fingerprint: first.stored_fingerprint,
            source_path: Some(first.source_path),
            ..Default::default()
        };
        let renamed = dir.join("renamed.vrm");
        std::fs::copy(&source, &renamed).unwrap();
        let files_before = std::fs::read_dir(&dir).unwrap().count();
        for path in [&source, &renamed] {
            let reused = import_registered(path, &dir, std::slice::from_ref(&entry)).unwrap();
            assert!(reused.duplicate);
            assert_eq!(reused.path, first.path);
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), files_before);
        }
        let legacy = CharacterEntry {
            fingerprint: String::new(),
            source_path: None,
            ..entry
        };
        assert!(
            import_registered(&source, &dir, &[legacy])
                .unwrap()
                .duplicate
        );
        let copy = dir.join(super::super::saved_filename(&first.path).unwrap());
        std::fs::remove_file(&copy).unwrap();
        assert!(!import_registered(&source, &dir, &[]).unwrap().duplicate);
        assert!(source.is_file());
        for path in [source, renamed, copy] {
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn fingerprint_is_stable_and_content_based_not_filename_based() {
        let dir = std::env::temp_dir().join(format!("nera-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("same.vrm");
        std::fs::write(&path, b"abc").unwrap();
        let old = fingerprint(&path).unwrap();
        assert_eq!(old, blake3::hash(b"abc").to_hex().to_string());
        std::fs::write(&path, b"different content").unwrap();
        assert_ne!(old, fingerprint(&path).unwrap());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
