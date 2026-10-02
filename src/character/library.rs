//! Persistent model catalog and independently controlled desktop companions.
use super::roster::{CharacterRoster, MAX_CHARACTERS, RosterRequest, SlotView};
use super::{DEFAULT_CHARACTER, character_directory, saved_filename};
use crate::settings::NeraSettings;
use bevy::prelude::*;
use bevy::{
    tasks::{IoTaskPool, Task, poll_once},
    window::PrimaryWindow,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, Write},
    process::{Child, Command, Stdio},
    sync::{Mutex, mpsc},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CharacterEntry {
    pub(crate) path: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) facial_targets: usize,
}

#[derive(Resource)]
pub(crate) struct CharacterCatalog(pub(crate) Vec<CharacterEntry>);

impl Default for CharacterCatalog {
    fn default() -> Self {
        let mut catalog = Self(vec![CharacterEntry {
            path: DEFAULT_CHARACTER.into(),
            name: "Nera".into(),
            facial_targets: 44,
        }]);
        let directory = character_directory();
        if let Ok(bytes) = std::fs::read(directory.join("catalog.json"))
            && let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes)
            && let Some(entries) = value.as_array()
        {
            for entry in entries {
                if let (Some(path), Some(name)) = (entry["path"].as_str(), entry["name"].as_str())
                    && saved_filename(path).is_some_and(|file| directory.join(file).is_file())
                {
                    catalog.insert(path, name);
                }
            }
        }
        // Recover models imported before the catalog existed, or after a failed catalog save.
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = format!("characters://{name}");
                if saved_filename(&path).is_some() && entry.path().is_file() {
                    let (label, _) = model_metadata(&entry.path());
                    let label = label
                        .filter(|label| label != "Unknown")
                        .unwrap_or_else(|| format!("Imported Character {}", catalog.0.len()));
                    catalog.insert(&path, &label);
                }
            }
        }
        catalog
    }
}

impl CharacterCatalog {
    fn insert(&mut self, path: &str, name: &str) {
        if !self.0.iter().any(|entry| entry.path == path) {
            let metadata =
                saved_filename(path).map(|file| model_metadata(&character_directory().join(file)));
            let name = if name.starts_with("model-") {
                metadata
                    .as_ref()
                    .and_then(|(name, _)| name.clone())
                    .filter(|name| name != "Unknown")
                    .unwrap_or_else(|| format!("Imported Character {}", self.0.len()))
            } else {
                name.into()
            };
            self.0.push(CharacterEntry {
                path: path.into(),
                name: name.chars().take(48).collect(),
                facial_targets: metadata.map_or(44, |(_, targets)| targets),
            });
        }
    }

    pub(super) fn remember(&mut self, path: &str, name: &str) -> Result<(), String> {
        self.insert(path, name);
        let value: Vec<_> = self
            .0
            .iter()
            .skip(1)
            .map(|entry| serde_json::json!({ "path": entry.path, "name": entry.name }))
            .collect();
        let directory = character_directory();
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let temporary = directory.join(format!("catalog-{}.tmp", std::process::id()));
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::fs::rename(temporary, directory.join("catalog.json")).map_err(|e| e.to_string())
    }
}

pub(crate) struct Companion {
    pub(crate) id: u64,
    pub(crate) name: String,
    path: String,
    child: Child,
}

#[derive(Resource)]
pub(crate) struct CharacterInstances {
    pub(crate) companions: Vec<Companion>,
    next_id: u64,
    slots: Vec<Option<u64>>,
}

impl Default for CharacterInstances {
    fn default() -> Self {
        Self {
            companions: Vec::new(),
            next_id: 0,
            slots: vec![None, None],
        }
    }
}

impl Drop for CharacterInstances {
    fn drop(&mut self) {
        // Closing the owning pipe asks each companion to exit, including its gallery worker.
        for companion in &mut self.companions {
            companion.child.stdin.take();
        }
    }
}

impl CharacterInstances {
    pub(crate) fn roster(
        &self,
        catalog: &CharacterCatalog,
        active: &super::ActiveCharacter,
        visible: bool,
        status: &str,
    ) -> CharacterRoster {
        let slots = self
            .slots
            .iter()
            .enumerate()
            .map(|(slot, id)| {
                let path = if slot == 0 {
                    visible.then(|| active.asset_path.clone())
                } else {
                    id.and_then(|id| {
                        self.companions
                            .iter()
                            .find(|c| c.id == id)
                            .map(|c| c.path.clone())
                    })
                };
                let name = path
                    .as_ref()
                    .and_then(|path| catalog.0.iter().find(|e| &e.path == path))
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                SlotView {
                    used: path.is_some(),
                    path,
                    name,
                }
            })
            .collect();
        CharacterRoster {
            models: catalog.0.clone(),
            slots,
            can_add: self.slots.len() < MAX_CHARACTERS && instance_id().is_none(),
            status: status.into(),
        }
    }
    #[cfg(debug_assertions)]
    pub(crate) fn process_ids(&self) -> Vec<u32> {
        self.companions
            .iter()
            .map(|companion| companion.child.id())
            .collect()
    }
}

#[derive(Message, Clone)]
pub(crate) enum CharacterLibraryRequest {
    Select(String),
    Spawn(String),
    Close(u64),
    SpawnAt { slot: usize, path: String },
}

#[derive(Resource, Default)]
struct LibraryImport(
    Option<(
        usize,
        String,
        Task<Result<super::ImportedCharacter, String>>,
    )>,
);

fn model_metadata(path: &std::path::Path) -> (Option<String>, usize) {
    use std::io::Read;
    let read = || -> Option<(Option<String>, usize)> {
        let mut file = std::fs::File::open(path).ok()?;
        let mut header = [0; 20];
        file.read_exact(&mut header).ok()?;
        if &header[..4] != b"glTF" {
            return None;
        }
        let length = u32::from_le_bytes(header[12..16].try_into().ok()?) as usize;
        if length > 16 * 1024 * 1024 {
            return None;
        }
        let mut bytes = vec![0; length];
        file.read_exact(&mut bytes).ok()?;
        let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
        let name = value
            .pointer("/extensions/VRMC_vrm/meta/name")
            .or_else(|| value.pointer("/extensions/VRM/meta/title"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        let targets = value
            .get("meshes")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|mesh| {
                mesh.pointer("/primitives/0/targets")
                    .and_then(serde_json::Value::as_array)
            })
            .map(Vec::len)
            .sum();
        Some((name, targets))
    };
    read().unwrap_or_default()
}

pub(crate) fn instance_id() -> Option<String> {
    let args: Vec<_> = std::env::args().collect();
    let index = args.iter().position(|arg| arg == "--character-instance")?;
    args.get(index + 1)
        .filter(|id| id.chars().all(|c| c.is_ascii_digit()))
        .cloned()
}

pub(crate) fn configure_instance(settings: &mut NeraSettings) {
    if instance_id().is_none() {
        return;
    }
    settings.start_hidden = false;
    settings.remember_position = true;
    settings.character_asset = std::env::var("NERA_INSTANCE_CHARACTER").unwrap_or_default();
    settings.alarm_enabled = false;
    settings.alarm_book = "v1|".into();
    settings.alarm_book_initialized = true;
    if let Ok(value) = std::env::var("NERA_INSTANCE_SETTINGS")
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&value)
    {
        settings.window_x = value["x"].as_i64().unwrap_or(200) as i32;
        settings.window_y = value["y"].as_i64().unwrap_or(200) as i32;
        settings.character_size = value["size"].as_u64().unwrap_or(100).clamp(25, 200) as u16;
        settings.fps_limit = value["fps"].as_u64().unwrap_or(24).min(144) as u16;
        settings.master_volume = value["volume"].as_u64().unwrap_or(100).min(100) as u8;
        settings.audio_muted = value["muted"].as_bool().unwrap_or(false);
        settings.walking_enabled = value["walking"].as_bool().unwrap_or(false);
        settings.gravity_enabled = value["gravity"].as_bool().unwrap_or(true);
        settings.throw_enabled = value["throw"].as_bool().unwrap_or(true);
        settings.throw_intensity = value["throw_intensity"].as_u64().unwrap_or(50).min(100) as u8;
        settings.brightness = value["brightness"].as_u64().unwrap_or(100).clamp(25, 200) as u8;
        settings.shadow_enabled = value["shadow"].as_bool().unwrap_or(true);
    }
}

#[derive(Resource)]
struct OwnerPipe(Mutex<mpsc::Receiver<Option<String>>>);

pub(super) fn install(app: &mut App) {
    app.init_resource::<CharacterCatalog>()
        .init_resource::<CharacterInstances>()
        .init_resource::<LibraryImport>()
        .add_message::<CharacterLibraryRequest>()
        .add_message::<RosterRequest>()
        .add_systems(Update, (poll_library_import, handle_requests).chain());
    if instance_id().is_some() {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                if line.is_err() || matches!(line.as_deref(), Ok("EXIT")) {
                    break;
                }
                if let Ok(line) = line
                    && let Some(json) = line.strip_prefix("MODEL\t")
                    && let Ok(path) = serde_json::from_str::<String>(json)
                {
                    let _ = sender.send(Some(path));
                }
            }
            let _ = sender.send(None);
        });
        app.insert_resource(OwnerPipe(Mutex::new(receiver)))
            .add_systems(Update, exit_with_owner);
        #[cfg(debug_assertions)]
        if std::env::var_os("NERA_COMPANION_SMOKE").is_some() {
            app.add_systems(Last, report_companion_smoke);
        }
    }
}

#[cfg(debug_assertions)]
fn report_companion_smoke(
    time: Res<Time<Real>>,
    mut recorded: Local<bool>,
    avatars: Query<
        &Visibility,
        (
            With<crate::avatar::NeraAvatar>,
            With<bevy_vrm1::prelude::Initialized>,
        ),
    >,
    bones: Query<(), With<crate::interaction::NeraLiveBone>>,
    character: Res<super::ActiveCharacter>,
    settings: Res<NeraSettings>,
) {
    if *recorded
        || time.elapsed_secs() < 5.0
        || !avatars
            .iter()
            .any(|visibility| *visibility != Visibility::Hidden)
    {
        return;
    }
    *recorded = true;
    let report = serde_json::json!({"pid":std::process::id(),"character":character.asset_path,
        "live_bones":bones.iter().count(),"x":settings.window_x,"alarm_count":settings.alarm_configs().len()});
    let directory = std::env::temp_dir().join("nera-feature-smoke");
    let _ = std::fs::create_dir_all(&directory);
    let _ = std::fs::write(
        directory.join(format!("companion-{}.json", std::process::id())),
        serde_json::to_vec_pretty(&report).unwrap(),
    );
}

fn exit_with_owner(
    pipe: Res<OwnerPipe>,
    mut requests: MessageWriter<CharacterLibraryRequest>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Ok(receiver) = pipe.0.lock() {
        for event in receiver.try_iter() {
            match event {
                Some(path) => {
                    requests.write(CharacterLibraryRequest::Select(path));
                }
                None => {
                    exit.write(AppExit::Success);
                }
            }
        }
    }
}

fn poll_library_import(
    mut job: ResMut<LibraryImport>,
    mut catalog: ResMut<CharacterCatalog>,
    mut status: ResMut<super::CharacterImportStatus>,
    mut requests: MessageWriter<RosterRequest>,
) {
    let Some((_, _, task)) = job.0.as_mut() else {
        return;
    };
    let Some(result) = bevy::tasks::block_on(poll_once(task)) else {
        return;
    };
    let (slot, name, _) = job.0.take().unwrap();
    status.loading = false;
    match result {
        Ok(imported) => {
            if let Err(error) = catalog.remember(&imported.path, &name) {
                warn!("Catalog save: {error}");
            }
            requests.write(RosterRequest::Assign {
                slot,
                path: imported.path,
            });
        }
        Err(error) => status.message = error,
    }
}

fn handle_requests(
    mut requests: MessageReader<CharacterLibraryRequest>,
    mut roster: MessageReader<RosterRequest>,
    mut job: ResMut<LibraryImport>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    catalog: Res<CharacterCatalog>,
    settings: Res<NeraSettings>,
    mut instances: ResMut<CharacterInstances>,
    mut imports: MessageWriter<super::ImportCharacter>,
    mut restore: MessageWriter<super::RestoreDefaultCharacter>,
    mut status: ResMut<super::CharacterImportStatus>,
) {
    let mut changed = false;
    instances
        .bypass_change_detection()
        .companions
        .retain_mut(|companion| {
            let running = matches!(companion.child.try_wait(), Ok(None));
            changed |= !running;
            running
        });
    if changed {
        let live: Vec<_> = instances.companions.iter().map(|c| c.id).collect();
        for slot in &mut instances.slots {
            if slot.is_some_and(|id| !live.contains(&id)) {
                *slot = None;
            }
        }
        instances.set_changed();
    }
    let mut pending: Vec<_> = requests.read().cloned().collect();
    for request in roster.read().filter(|request| request.valid()) {
        match request {
            RosterRequest::AddSlot
                if instances.slots.len() < MAX_CHARACTERS && instance_id().is_none() =>
            {
                instances.slots.push(None)
            }
            RosterRequest::Clear { slot } if *slot < instances.slots.len() => {
                if *slot == 0 {
                    if let Ok(mut window) = windows.single_mut() {
                        window.visible = false;
                    }
                } else if let Some(id) = instances.slots[*slot] {
                    pending.push(CharacterLibraryRequest::Close(id));
                }
            }
            RosterRequest::Assign { slot, path }
                if *slot < instances.slots.len() && catalog.0.iter().any(|e| e.path == *path) =>
            {
                if *slot == 0 {
                    if let Ok(mut window) = windows.single_mut() {
                        window.visible = true;
                    }
                    pending.push(CharacterLibraryRequest::Select(path.clone()));
                } else if let Some(id) = instances.slots[*slot] {
                    if let Some(companion) = instances.companions.iter_mut().find(|c| c.id == id)
                        && let Some(pipe) = companion.child.stdin.as_mut()
                    {
                        if writeln!(pipe, "MODEL\t{}", serde_json::to_string(path).unwrap()).is_ok()
                        {
                            companion.path = path.clone();
                            companion.name = catalog
                                .0
                                .iter()
                                .find(|e| e.path == *path)
                                .unwrap()
                                .name
                                .clone();
                        }
                    }
                } else {
                    pending.push(CharacterLibraryRequest::SpawnAt {
                        slot: *slot,
                        path: path.clone(),
                    });
                }
            }
            RosterRequest::Import { slot, path }
                if *slot < instances.slots.len() && job.0.is_none() =>
            {
                let name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let source = path.clone();
                job.0 = Some((
                    *slot,
                    name,
                    IoTaskPool::get()
                        .spawn(async move { super::import_model(&source, &character_directory()) }),
                ));
                status.message = "Importing character...".into();
                status.loading = true;
            }
            _ => {}
        }
    }
    for request in &pending {
        match request {
            CharacterLibraryRequest::Close(id) => {
                if let Some(companion) = instances
                    .companions
                    .iter_mut()
                    .find(|entry| entry.id == *id)
                    && let Some(pipe) = companion.child.stdin.as_mut()
                {
                    let _ = writeln!(pipe, "EXIT");
                    companion.child.stdin.take();
                }
            }
            CharacterLibraryRequest::Select(path) => {
                if !catalog.0.iter().any(|entry| entry.path == *path) {
                    continue;
                }
                if path == DEFAULT_CHARACTER {
                    restore.write(super::RestoreDefaultCharacter);
                } else if let Some(file) = saved_filename(path) {
                    imports.write(super::ImportCharacter(character_directory().join(file)));
                }
            }
            CharacterLibraryRequest::Spawn(path)
            | CharacterLibraryRequest::SpawnAt { path, .. } => {
                let Some(entry) = catalog.0.iter().find(|entry| entry.path == *path) else {
                    continue;
                };
                if instance_id().is_some() {
                    status.message = "Add companions from the main Nera window.".into();
                    continue;
                }
                let slot = if let CharacterLibraryRequest::SpawnAt { slot, .. } = request {
                    *slot
                } else if let Some(slot) =
                    (1..instances.slots.len()).find(|&slot| instances.slots[slot].is_none())
                {
                    slot
                } else {
                    instances.slots.len()
                };
                if slot == 0
                    || slot > instances.slots.len()
                    || slot >= MAX_CHARACTERS
                    || instances.slots.get(slot).is_some_and(Option::is_some)
                {
                    status.message = "Maximum 8 characters. Clear a slot first.".into();
                    continue;
                }
                if slot == instances.slots.len() {
                    instances.slots.push(None);
                }
                instances.next_id += 1;
                let id = instances.next_id;
                let session = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let snapshot = serde_json::json!({
                    "x": settings.window_x.saturating_add(120 * (id % 5) as i32),
                    "y": settings.window_y, "size": settings.character_size, "fps": settings.fps_limit,
                    "volume": settings.master_volume, "muted": settings.audio_muted,
                    "walking": settings.walking_enabled, "brightness": settings.brightness,
                    "shadow": settings.shadow_enabled,
                    "gravity": settings.gravity_enabled,
                    "throw": settings.throw_enabled, "throw_intensity": settings.throw_intensity,
                });
                let result = std::env::current_exe().and_then(|executable| {
                    let mut command = Command::new(executable);
                    command
                        .args(["--character-instance", &session.to_string()])
                        .env(
                            "NERA_INSTANCE_CHARACTER",
                            if path == DEFAULT_CHARACTER { "" } else { path },
                        )
                        .env("NERA_INSTANCE_SETTINGS", snapshot.to_string())
                        .stdin(Stdio::piped())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null());
                    #[cfg(debug_assertions)]
                    if std::env::args().any(|arg| arg == "--feature-smoke") {
                        command.env("NERA_COMPANION_SMOKE", "1");
                    }
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        command.creation_flags(0x0800_0000);
                    }
                    command.spawn()
                });
                match result {
                    Ok(child) => {
                        instances.companions.push(Companion {
                            id,
                            name: entry.name.clone(),
                            path: path.clone(),
                            child,
                        });
                        instances.slots[slot] = Some(id);
                        status.message = format!("Added {} to the desktop.", entry.name);
                    }
                    Err(error) => status.message = format!("Cannot add character: {error}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slots_start_with_two_stop_at_eight_and_primary_clear_is_local() {
        let mut app = App::new();
        app.init_resource::<NeraSettings>()
            .init_resource::<CharacterInstances>()
            .init_resource::<LibraryImport>()
            .insert_resource(CharacterCatalog(vec![CharacterEntry {
                path: DEFAULT_CHARACTER.into(),
                name: "Nera".into(),
                facial_targets: 44,
            }]))
            .init_resource::<super::super::CharacterImportStatus>()
            .add_message::<CharacterLibraryRequest>()
            .add_message::<RosterRequest>()
            .add_message::<super::super::ImportCharacter>()
            .add_message::<super::super::RestoreDefaultCharacter>()
            .add_systems(Update, handle_requests);
        let primary = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        assert_eq!(app.world().resource::<CharacterInstances>().slots.len(), 2);
        for _ in 0..12 {
            app.world_mut().write_message(RosterRequest::AddSlot);
        }
        app.update();
        assert_eq!(app.world().resource::<CharacterInstances>().slots.len(), 8);
        app.world_mut()
            .write_message(RosterRequest::Clear { slot: 0 });
        app.update();
        assert!(!app.world().get::<Window>(primary).unwrap().visible);
        app.world_mut().write_message(RosterRequest::Assign {
            slot: 0,
            path: DEFAULT_CHARACTER.into(),
        });
        app.update();
        assert!(app.world().get::<Window>(primary).unwrap().visible);
        assert!(
            app.world()
                .resource::<CharacterInstances>()
                .companions
                .is_empty()
        );
    }
    #[test]
    fn catalog_deduplicates_models_and_retains_the_bundled_character() {
        let mut catalog = CharacterCatalog(vec![CharacterEntry {
            path: DEFAULT_CHARACTER.into(),
            name: "Nera".into(),
            facial_targets: 44,
        }]);
        catalog.insert("characters://model-abc.vrm", "Alice");
        catalog.insert("characters://model-abc.vrm", "Alice again");
        catalog.insert("characters://model-def.vrm", "Bob");
        assert_eq!(catalog.0.len(), 3);
        assert_eq!(catalog.0[1].name, "Alice");
    }
}
