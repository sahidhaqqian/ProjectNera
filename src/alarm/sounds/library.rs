//! Content-addressed alarm files live in the existing application-data namespace.
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use bevy::{audio::AudioSource, prelude::*};
use rodio::Source;
use serde::{Deserialize, Serialize};

use super::{DEFAULT_SOUND, RANDOM_SOUND, SOUNDS};

const MAX_IMPORTS: usize = 32;
const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SECONDS: u64 = 300;
const MAX_CACHED_IMPORTS: usize = 2;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ImportedSound {
    pub(crate) id: u16,
    pub(crate) name: String,
    fingerprint: String,
    filename: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Catalog {
    #[serde(default = "first_import_id")]
    next_id: u16,
    entries: Vec<ImportedSound>,
}

fn first_import_id() -> u16 {
    5
}

struct CachedSound {
    id: u16,
    fingerprint: String,
    handle: Handle<AudioSource>,
}

#[derive(Resource)]
pub(crate) struct AlarmSoundLibrary {
    directory: PathBuf,
    catalog: Catalog,
    cache: Vec<CachedSound>,
    rng: crate::rng::NeraRng,
}

impl Default for AlarmSoundLibrary {
    fn default() -> Self {
        if cfg!(test) {
            return Self::load(
                std::env::temp_dir()
                    .join(format!("nera-empty-alarm-library-{}", std::process::id())),
            );
        }
        let directory = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("io.github.sahidhaqqi.projectnera")
            .join("alarm-sounds");
        Self::load(directory)
    }
}

impl AlarmSoundLibrary {
    pub(crate) fn load(directory: PathBuf) -> Self {
        let mut catalog = fs::read(directory.join("catalog.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Catalog>(&bytes).ok())
            .unwrap_or_default();
        catalog.entries.retain(|entry| {
            entry.id >= first_import_id()
                && entry.id < super::FIRST_EXTRA_BUNDLED_SOUND_ID
                && owned_filename(&entry.filename, &entry.fingerprint)
        });
        catalog.entries.truncate(MAX_IMPORTS);
        let mut seen = std::collections::HashSet::new();
        catalog.entries.retain(|entry| seen.insert(entry.id));
        catalog.next_id = catalog.next_id.max(first_import_id()).max(
            catalog
                .entries
                .iter()
                .map(|entry| entry.id.saturating_add(1))
                .max()
                .unwrap_or(0),
        );
        Self {
            directory,
            catalog,
            cache: Vec::new(),
            rng: default(),
        }
    }

    pub(crate) fn entries(&self) -> &[ImportedSound] {
        &self.catalog.entries
    }

    pub(crate) fn name(&self, id: u16) -> &str {
        if id == RANDOM_SOUND {
            return "Random";
        }
        if let Some(index) = super::bundled_index(id) {
            return SOUNDS[index].0;
        }
        self.catalog
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .map_or("Classic (unavailable sound)", |entry| entry.name.as_str())
    }

    pub(crate) fn next(&self, id: u16) -> u16 {
        let choices: Vec<_> = super::BUNDLED_SOUND_IDS
            .into_iter()
            .chain(self.catalog.entries.iter().map(|entry| entry.id))
            .chain(Some(RANDOM_SOUND))
            .collect();
        let index = choices
            .iter()
            .position(|choice| *choice == id)
            .unwrap_or(choices.len() - 1);
        choices[(index + 1) % choices.len()]
    }

    fn persist(&self) -> Result<(), String> {
        fs::create_dir_all(&self.directory).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(&self.catalog).map_err(|e| e.to_string())?;
        let staging = self.directory.join("catalog-next.json");
        fs::write(&staging, bytes).map_err(|e| e.to_string())?;
        fs::rename(&staging, self.directory.join("catalog.json")).map_err(|e| e.to_string())
    }

    pub(crate) fn import(&mut self, source: &Path) -> Result<(u16, bool), String> {
        let extension = source
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|ext| matches!(ext.as_str(), "mp3" | "ogg" | "wav"))
            .ok_or("Choose an MP3, OGG, or WAV sound.")?;
        let bytes = read_sound(source)?;
        validate_audio(&bytes)?;
        let fingerprint = blake3::hash(&bytes).to_hex().to_string();
        if let Some(entry) = self
            .catalog
            .entries
            .iter()
            .find(|entry| entry.fingerprint == fingerprint)
        {
            // Reimport also repairs a missing/replaced catalog file using verified bytes.
            fs::create_dir_all(&self.directory).map_err(|e| e.to_string())?;
            fs::write(self.directory.join(&entry.filename), bytes).map_err(|e| e.to_string())?;
            self.cache.retain(|cached| cached.id != entry.id);
            return Ok((entry.id, true));
        }
        if self.catalog.entries.len() >= MAX_IMPORTS
            || self.catalog.next_id >= super::FIRST_EXTRA_BUNDLED_SOUND_ID
        {
            return Err("Alarm sound library is full (32 imported sounds).".into());
        }
        let id = self.catalog.next_id;
        let filename = format!("{fingerprint}.{extension}");
        fs::create_dir_all(&self.directory).map_err(|e| e.to_string())?;
        fs::write(self.directory.join(&filename), bytes).map_err(|e| e.to_string())?;
        self.catalog.next_id += 1;
        self.catalog.entries.push(ImportedSound {
            id,
            name: source
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("Imported sound")
                .chars()
                .take(48)
                .collect(),
            fingerprint,
            filename: filename.clone(),
        });
        if let Err(error) = self.persist() {
            self.catalog.entries.pop();
            self.catalog.next_id = id;
            let _ = fs::remove_file(self.directory.join(filename));
            return Err(error);
        }
        Ok((id, false))
    }

    pub(crate) fn remove(&mut self, id: u16) -> Result<bool, String> {
        let Some(index) = self.catalog.entries.iter().position(|entry| entry.id == id) else {
            return Ok(false); // Built-ins never enter the removable catalog.
        };
        let removed = self.catalog.entries.remove(index);
        if let Err(error) = self.persist() {
            self.catalog.entries.insert(index, removed);
            return Err(error);
        }
        if let Err(error) = fs::remove_file(self.directory.join(&removed.filename))
            && error.kind() != std::io::ErrorKind::NotFound
        {
            // Keep the registration if the file could not be removed.
            self.catalog.entries.insert(index, removed);
            let _ = self.persist();
            return Err(error.to_string());
        }
        self.cache.retain(|cached| cached.id != id);
        Ok(true)
    }

    fn imported_handle(
        &mut self,
        id: u16,
        audio: &mut Assets<AudioSource>,
    ) -> Option<Handle<AudioSource>> {
        let entry = self.catalog.entries.iter().find(|entry| entry.id == id)?;
        // Reads occur only on trigger/preview, never on audio repeat or per frame.
        let bytes = read_sound(&self.directory.join(&entry.filename)).ok()?;
        let fingerprint = blake3::hash(&bytes).to_hex().to_string();
        if fingerprint != entry.fingerprint {
            return None;
        }
        if let Some(index) = self
            .cache
            .iter()
            .position(|cached| cached.id == id && cached.fingerprint == fingerprint)
        {
            let cached = self.cache.remove(index);
            let handle = cached.handle.clone();
            self.cache.push(cached);
            return Some(handle);
        }
        validate_audio(&bytes).ok()?;
        let handle = audio.add(AudioSource {
            bytes: bytes.into(),
        });
        self.cache.retain(|cached| cached.id != id);
        if self.cache.len() >= MAX_CACHED_IMPORTS {
            self.cache.remove(0);
        }
        self.cache.push(CachedSound {
            id,
            fingerprint,
            handle: handle.clone(),
        });
        Some(handle)
    }

    /// Resolves Random exactly once. The ringing entity retains this handle for
    /// every repeat, independent of editor selection/deletion and subsequent RNG.
    pub(crate) fn resolve(
        &mut self,
        selection: u16,
        bundled: &[Handle<AudioSource>; SOUNDS.len()],
        audio: &mut Assets<AudioSource>,
    ) -> Handle<AudioSource> {
        if selection != RANDOM_SOUND {
            return super::bundled_index(selection)
                .and_then(|index| bundled.get(index))
                .cloned()
                .or_else(|| self.imported_handle(selection, audio))
                .unwrap_or_else(|| bundled[DEFAULT_SOUND as usize].clone());
        }
        let mut choices: Vec<_> = super::BUNDLED_SOUND_IDS
            .into_iter()
            .enumerate()
            .filter(|(index, _)| audio.contains(&bundled[*index]))
            .map(|(_, id)| id)
            .chain(self.catalog.entries.iter().map(|entry| entry.id))
            .collect();
        while let Some(index) = self.rng.pick_index(choices.len()) {
            let id = choices.swap_remove(index);
            if let Some(handle) = super::bundled_index(id)
                .and_then(|index| bundled.get(index))
                .cloned()
                .or_else(|| self.imported_handle(id, audio))
            {
                return handle;
            }
        }
        bundled[DEFAULT_SOUND as usize].clone()
    }
}

pub(crate) fn clear_deleted_references(settings: &mut crate::settings::NeraSettings, id: u16) {
    if settings.alarm_sound == id {
        settings.alarm_sound = DEFAULT_SOUND;
    }
    let mut alarms = settings.alarm_configs();
    let mut changed = false;
    for alarm in &mut alarms {
        if alarm.sound == id {
            alarm.sound = DEFAULT_SOUND;
            changed = true;
        }
    }
    if changed {
        settings.store_alarm_configs(&alarms);
    }
}

fn owned_filename(filename: &str, fingerprint: &str) -> bool {
    fingerprint.len() == 64
        && fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
        && ["mp3", "ogg", "wav"]
            .iter()
            .any(|ext| filename == format!("{fingerprint}.{ext}"))
}

fn read_sound(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|_| "Sound file is missing or cannot be read.")?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_BYTES {
        return Err("Alarm sounds must be 8 MiB or smaller.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_BYTES {
        return Err("Alarm sounds must contain audio and be 8 MiB or smaller.".into());
    }
    Ok(bytes)
}

fn validate_audio(bytes: &[u8]) -> Result<(), String> {
    validate_container_metadata(bytes)?;
    let mut decoder = rodio::Decoder::builder()
        .with_byte_len(bytes.len() as u64)
        .with_data(Cursor::new(bytes.to_vec()))
        .build()
        .map_err(|_| "Invalid or unsupported sound file.".to_string())?;
    let channels = u64::from(decoder.channels().get());
    let rate = u64::from(decoder.sample_rate().get());
    if channels > 2
        || rate > 192_000
        || decoder
            .total_duration()
            .is_some_and(|d| d.as_secs() > MAX_SECONDS)
    {
        return Err("Use a mono/stereo alarm sound no longer than five minutes.".into());
    }
    let limit = rate.saturating_mul(channels).saturating_mul(MAX_SECONDS);
    let mut count = 0;
    for sample in &mut decoder {
        count += 1;
        if !sample.is_finite() || count > limit {
            return Err("Invalid sound or audio longer than five minutes.".into());
        }
    }
    if count == 0 {
        return Err("The file contains no decodable audio.".into());
    }
    Ok(())
}

/// Rodio's WAV/Vorbis backends assume positive rate/channel metadata. Check
/// these container invariants before constructing Bevy's shared decoder, so a
/// malformed user file never reaches those assertions (including abort builds).
fn validate_container_metadata(bytes: &[u8]) -> Result<(), String> {
    let invalid = || "Invalid or unsupported sound file.".to_string();
    let word = |at: usize| -> Option<u16> {
        Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
    };
    let dword = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    if bytes.starts_with(b"RIFF") {
        if bytes.get(8..12) != Some(b"WAVE")
            || dword(4).is_none_or(|len| u64::from(len) + 8 > bytes.len() as u64)
        {
            return Err(invalid());
        }
        let mut offset = 12;
        let mut format_seen = false;
        let mut data_seen = false;
        while offset + 8 <= bytes.len() {
            let len = dword(offset + 4).ok_or_else(invalid)? as usize;
            let end = offset
                .checked_add(8)
                .and_then(|v| v.checked_add(len))
                .ok_or_else(invalid)?;
            if end > bytes.len() {
                return Err(invalid());
            }
            match &bytes[offset..offset + 4] {
                b"fmt " => {
                    if len < 16
                        || word(offset + 10).is_none_or(|channels| !(1..=2).contains(&channels))
                        || dword(offset + 12).is_none_or(|rate| !(1..=192_000).contains(&rate))
                        || word(offset + 20).is_none_or(|align| align == 0)
                    {
                        return Err(invalid());
                    }
                    format_seen = true;
                }
                b"data" => data_seen = len > 0,
                _ => {}
            }
            offset = end + (len & 1);
        }
        if !format_seen || !data_seen {
            return Err(invalid());
        }
    } else if bytes.starts_with(b"OggS") {
        let segments = usize::from(*bytes.get(26).ok_or_else(invalid)?);
        let body = 27 + segments;
        let table = bytes.get(27..body).ok_or_else(invalid)?;
        let length: usize = table.iter().map(|size| usize::from(*size)).sum();
        if body + length > bytes.len()
            || bytes.get(body..body + 7) != Some(b"\x01vorbis")
            || bytes
                .get(body + 11)
                .is_none_or(|channels| !(1..=2).contains(channels))
            || dword(body + 12).is_none_or(|rate| !(1..=192_000).contains(&rate))
        {
            return Err(invalid());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let path = std::env::temp_dir().join(format!(
                "nera-alarm-sounds-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn source(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::copy(crate::nera_asset_file("sound/clock_beeps.wav"), &path).unwrap();
            path
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn added_bundled_choices_preserve_imported_id_five_and_resolve_their_own_assets() {
        let dir = TestDirectory::new();
        let catalog_dir = dir.0.join("library");
        let mut library = AlarmSoundLibrary::load(catalog_dir.clone());
        let (import_id, _) = library.import(&dir.source("My alarm.wav")).unwrap();
        assert_eq!(import_id, 5, "existing imported IDs must remain unchanged");
        let mut library = AlarmSoundLibrary::load(catalog_dir);
        assert_eq!(library.name(import_id), "My alarm");
        assert_eq!(library.entries()[0].id, import_id);
        let mut audio = Assets::<AudioSource>::default();
        let bundled = std::array::from_fn(|_| {
            audio.add(AudioSource {
                bytes: vec![0].into(),
            })
        });
        let imported = library.resolve(import_id, &bundled, &mut audio);
        assert!(bundled.iter().all(|sound| sound.id() != imported.id()));
        for (index, id) in super::super::BUNDLED_SOUND_IDS.into_iter().enumerate() {
            assert_eq!(library.name(id), SOUNDS[index].0);
            assert_eq!(
                library.resolve(id, &bundled, &mut audio).id(),
                bundled[index].id()
            );
            assert!(!library.remove(id).unwrap(), "built-ins must be protected");
        }
        let choices: Vec<_> = super::super::BUNDLED_SOUND_IDS
            .into_iter()
            .chain([import_id, RANDOM_SOUND])
            .collect();
        for (index, id) in choices.iter().copied().enumerate() {
            assert_eq!(library.next(id), choices[(index + 1) % choices.len()]);
        }
        library.rng = crate::rng::NeraRng::seeded(1234);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..128 {
            let handle = library.resolve(RANDOM_SOUND, &bundled, &mut audio);
            assert!(
                handle.id() == imported.id()
                    || bundled.iter().any(|sound| sound.id() == handle.id())
            );
            seen.insert(handle.id());
        }
        assert_eq!(
            seen.len(),
            SOUNDS.len() + 1,
            "Random includes all built-ins and imports"
        );
    }

    #[test]
    fn content_identity_survives_rename_restart_delete_and_reimport() {
        let dir = TestDirectory::new();
        let mut library = AlarmSoundLibrary::load(dir.0.join("library"));
        let first = dir.source("alarm.wav");
        let renamed = dir.source("alarm-copy.wav");
        let (id, duplicate) = library.import(&first).unwrap();
        assert!(!duplicate);
        assert_eq!(library.import(&renamed).unwrap(), (id, true));
        assert_eq!(library.entries().len(), 1);
        let mut restored = AlarmSoundLibrary::load(dir.0.join("library"));
        assert_eq!(restored.entries()[0].id, id);
        assert!(!restored.remove(DEFAULT_SOUND).unwrap());
        assert!(restored.remove(id).unwrap());
        assert!(restored.entries().is_empty());
        let (next, _) = restored.import(&renamed).unwrap();
        assert!(
            next > id,
            "deleted IDs must not alias a newly imported file"
        );
    }

    #[test]
    fn supported_formats_decode_and_cache_is_bounded_while_live_handles_survive_delete() {
        let dir = TestDirectory::new();
        let mut library = AlarmSoundLibrary::load(dir.0.join("library"));
        let mut audio = Assets::<AudioSource>::default();
        let bundled = std::array::from_fn(|_| {
            audio.add(AudioSource {
                bytes: vec![0].into(),
            })
        });
        let mut selected = Vec::new();
        for path in [
            "sound/alarm.mp3",
            "sound/chime.ogg",
            "sound/clock_beeps.wav",
        ] {
            let (id, _) = library.import(&crate::nera_asset_file(path)).unwrap();
            let handle = library.resolve(id, &bundled, &mut audio);
            assert_ne!(handle.id(), bundled[0].id());
            selected.push((id, handle));
        }
        assert_eq!(library.cache.len(), MAX_CACHED_IMPORTS);
        let (id, handle) = selected.remove(0);
        assert!(library.remove(id).unwrap());
        assert!(
            audio.contains(&handle),
            "a live alarm retains its selected bytes after file deletion"
        );
        for _ in 0..10 {
            for (id, _) in &selected {
                library.resolve(*id, &bundled, &mut audio);
            }
            assert!(library.cache.len() <= MAX_CACHED_IMPORTS);
        }
    }

    #[test]
    fn import_rejects_bad_extensions_empty_invalid_and_oversize_files_without_registration() {
        let dir = TestDirectory::new();
        let mut library = AlarmSoundLibrary::load(dir.0.join("library"));
        let unsupported = dir.source("alarm.exe");
        assert!(library.import(&unsupported).is_err());
        for bytes in [
            Vec::new(),
            b"this is not audio".to_vec(),
            vec![0; MAX_BYTES as usize + 1],
        ] {
            let path = dir.0.join("invalid.wav");
            fs::write(&path, bytes).unwrap();
            assert!(library.import(&path).is_err());
        }
        assert!(library.entries().is_empty());
        assert!(!library.directory.join("catalog.json").exists());
    }

    #[test]
    fn invalid_wav_metadata_and_truncated_audio_are_rejected_before_decoder_assertions() {
        let mut bytes = fs::read(crate::nera_asset_file("sound/clock_beeps.wav")).unwrap();
        let fmt = bytes
            .windows(4)
            .position(|window| window == b"fmt ")
            .unwrap();
        bytes[fmt + 12..fmt + 16].fill(0);
        assert!(validate_audio(&bytes).is_err());
        let mut bytes = fs::read(crate::nera_asset_file("sound/clock_beeps.wav")).unwrap();
        bytes[fmt + 10..fmt + 12].fill(0);
        assert!(validate_audio(&bytes).is_err());
        let mut bytes = fs::read(crate::nera_asset_file("sound/clock_beeps.wav")).unwrap();
        bytes.truncate(bytes.len() / 2);
        assert!(validate_audio(&bytes).is_err());
    }

    #[test]
    fn missing_corrupt_and_unknown_selections_fall_back_and_reimport_repairs_content() {
        let dir = TestDirectory::new();
        let mut library = AlarmSoundLibrary::load(dir.0.join("library"));
        let source = dir.source("alarm.wav");
        let (id, _) = library.import(&source).unwrap();
        let stored = library.directory.join(&library.entries()[0].filename);
        let mut audio = Assets::<AudioSource>::default();
        let bundled = std::array::from_fn(|_| {
            audio.add(AudioSource {
                bytes: vec![0].into(),
            })
        });
        let imported = library.resolve(id, &bundled, &mut audio);
        assert_ne!(imported.id(), bundled[0].id());
        fs::write(&stored, b"corrupted").unwrap();
        assert_eq!(
            library.resolve(id, &bundled, &mut audio).id(),
            bundled[0].id()
        );
        fs::remove_file(&stored).unwrap();
        assert_eq!(
            library.resolve(id, &bundled, &mut audio).id(),
            bundled[0].id()
        );
        assert_eq!(
            library.resolve(5000, &bundled, &mut audio).id(),
            bundled[0].id()
        );
        assert_eq!(library.import(&source).unwrap(), (id, true));
        assert_ne!(
            library.resolve(id, &bundled, &mut audio).id(),
            bundled[0].id()
        );
    }

    #[test]
    fn deleted_selection_clears_quick_and_persisted_alarm_references() {
        let mut settings = crate::settings::NeraSettings::default();
        settings.alarm_sound = 50;
        let mut alarm = crate::settings::AlarmConfig::default_new(1);
        alarm.sound = 50;
        let mut random = crate::settings::AlarmConfig::default_new(2);
        random.sound = RANDOM_SOUND;
        settings.store_alarm_configs(&[alarm, random]);
        clear_deleted_references(&mut settings, 50);
        assert_eq!(settings.alarm_sound, DEFAULT_SOUND);
        assert_eq!(settings.alarm_configs()[0].sound, DEFAULT_SOUND);
        assert_eq!(settings.alarm_configs()[1].sound, RANDOM_SOUND);
    }

    #[test]
    fn catalog_paths_cannot_escape_the_owned_directory() {
        assert!(!owned_filename("../alarm.wav", &"a".repeat(64)));
        assert!(!owned_filename("a.wav", ""));
        assert!(owned_filename(
            &format!("{}.wav", "a".repeat(64)),
            &"a".repeat(64)
        ));
    }
}
