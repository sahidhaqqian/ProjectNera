//! Validate and stage imported VRM 1.0 models before replacing the live avatar.
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use bevy::{
    asset::{
        AssetApp,
        io::{AssetSourceBuilder, file::FileAssetReader},
    },
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, poll_once},
};
use bevy_vrm1::prelude::*;

use crate::{
    avatar::{LookAtTarget, NeraAvatar},
    settings::NeraSettings,
};

pub(crate) mod library;
pub(crate) mod roster;

pub(crate) const DEFAULT_CHARACTER: &str = "vrm/nera_256.vrm";
const MAX_MODEL_BYTES: u64 = 128 * 1024 * 1024;
const LOAD_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Message)]
pub(crate) struct ImportCharacter(pub PathBuf);

#[derive(Message)]
pub(crate) struct RestoreDefaultCharacter;

#[derive(Resource, Default)]
pub(crate) struct CharacterImportStatus {
    pub(crate) message: String,
    pub(crate) loading: bool,
    pub(crate) selection_notice: (u64, String),
    pub(crate) thumbnail_revision: u64,
}

#[derive(Resource)]
pub(crate) struct ActiveCharacter {
    pub(crate) asset_path: String,
    pub(crate) migrated_from_vrm0: bool,
}

impl Default for ActiveCharacter {
    fn default() -> Self {
        Self {
            asset_path: DEFAULT_CHARACTER.to_owned(),
            migrated_from_vrm0: false,
        }
    }
}

#[derive(Resource, Default)]
struct CharacterImport {
    copy: Option<Task<Result<ImportedCharacter, String>>>,
    pending: Option<PendingCharacter>,
    label: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct ImportedCharacter {
    path: String,
    converted: bool,
    migrated_from_vrm0: bool,
}

struct PendingCharacter {
    path: String,
    converted: bool,
    migrated_from_vrm0: bool,
    handle: Handle<VrmAsset>,
    started: Duration,
    entity: Option<Entity>,
}

#[derive(Component)]
pub(crate) struct CharacterReveal {
    visibility: Visibility,
    ready_frames: u8,
    started: Duration,
    converted: bool,
}

fn character_directory() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("io.github.sahidhaqqi.projectnera")
        .join("characters")
}

/// Resolve only catalog-owned models and the built-in avatar for preview metadata.
pub(crate) fn preview_model_file(asset_path: &str) -> Option<PathBuf> {
    if asset_path == DEFAULT_CHARACTER {
        Some(crate::nera_asset_file(asset_path))
    } else {
        saved_filename(asset_path).map(|name| character_directory().join(name))
    }
}

/// Register before AssetPlugin, including in the Animation Manager worker.
pub(crate) fn register_asset_source(app: &mut App) {
    let directory = character_directory();
    app.register_asset_source(
        "characters",
        AssetSourceBuilder::new(move || Box::new(FileAssetReader::new(directory.clone()))),
    );
}

pub(crate) struct CharacterPlugin;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        library::install(app);
        app.init_resource::<ActiveCharacter>()
            .init_resource::<CharacterImportStatus>()
            .init_resource::<CharacterImport>()
            .add_message::<ImportCharacter>()
            .add_message::<RestoreDefaultCharacter>()
            .add_systems(Startup, restore_saved_character)
            .add_systems(
                Update,
                (receive_import, update_import)
                    .chain()
                    .before(crate::interaction::mark_nera_live_bones)
                    .before(crate::animation::apply_animation_state),
            )
            .add_systems(
                PostUpdate,
                reveal_character
                    .after(bevy::app::AnimationSystems)
                    .before(VrmSystemSets::Constraints),
            );
    }
}

fn saved_filename(asset_path: &str) -> Option<&str> {
    let name = asset_path.strip_prefix("characters://")?;
    if !name.starts_with("model-")
        || !name.ends_with(".vrm")
        || name.contains(['/', '\\', ':'])
        || name.contains("..")
    {
        return None;
    }
    Some(name)
}

fn restore_saved_character(
    settings: Res<NeraSettings>,
    mut runtime: ResMut<CharacterImport>,
    mut status: ResMut<CharacterImportStatus>,
) {
    if settings.character_asset.is_empty() {
        return;
    }
    let Some(name) = saved_filename(&settings.character_asset) else {
        status.message = "Saved character path is invalid; using Nera.".into();
        return;
    };
    let path = character_directory().join(name);
    let directory = character_directory();
    runtime.label = Some(name.to_owned());
    runtime.copy = Some(IoTaskPool::get().spawn(async move { import_model(&path, &directory) }));
    status.loading = true;
    status.message = "Loading saved character...".into();
}

fn receive_import(
    mut imports: MessageReader<ImportCharacter>,
    mut restore: MessageReader<RestoreDefaultCharacter>,
    mut runtime: ResMut<CharacterImport>,
    mut status: ResMut<CharacterImportStatus>,
    release: Option<Res<library::release::ReleaseGuard>>,
    mut commands: Commands,
) {
    if release.as_ref().is_some_and(|guard| guard.busy()) {
        imports.clear();
    }
    let selected = imports.read().last().map(|request| request.0.clone());
    let restore = restore.read().next().is_some();
    if selected.is_none() && !restore {
        return;
    }
    if let Some(pending) = runtime.pending.take()
        && let Some(entity) = pending.entity
    {
        commands.entity(entity).despawn();
    }
    // Last selection wins. Completed copies are immutable private assets, so
    // cancelling a previous task cannot overwrite a newer character.
    runtime.label = selected
        .as_ref()
        .and_then(|path| path.file_stem())
        .map(|name| name.to_string_lossy().into_owned());
    runtime.copy = Some(IoTaskPool::get().spawn(async move {
        if restore {
            Ok(ImportedCharacter {
                path: DEFAULT_CHARACTER.to_owned(),
                converted: false,
                migrated_from_vrm0: false,
            })
        } else {
            import_model(&selected.unwrap(), &character_directory())
        }
    }));
    status.loading = true;
    status.message = "Loading character...".into();
}

fn update_import(world: &mut World) {
    world.resource_scope(|world, mut runtime: Mut<CharacterImport>| {
        if let Some(task) = runtime.copy.as_mut()
            && let Some(result) = block_on(poll_once(task))
        {
            runtime.copy = None;
            match result {
                Ok(imported) => {
                    if imported.path != DEFAULT_CHARACTER {
                        let label = runtime.label.take().unwrap_or_else(|| "Character".into());
                        if let Err(error) = world
                            .resource_mut::<library::CharacterCatalog>()
                            .remember(&imported.path, &label)
                        {
                            warn!("Character catalog save failed: {error}");
                        }
                    }
                    runtime.pending = Some(PendingCharacter {
                        handle: world.resource::<AssetServer>().load(imported.path.clone()),
                        path: imported.path,
                        converted: imported.converted,
                        migrated_from_vrm0: imported.migrated_from_vrm0,
                        started: world.resource::<Time<Real>>().elapsed(),
                        entity: None,
                    });
                }
                Err(reason) => import_failed(world, reason),
            }
        }
        let Some(mut pending) = runtime.pending.take() else {
            return;
        };
        let assets = world.resource::<AssetServer>();
        let failure = if assets
            .get_load_state(pending.handle.id())
            .is_some_and(|s| s.is_failed())
            || assets
                .get_recursive_dependency_load_state(pending.handle.id())
                .is_some_and(|s| s.is_failed())
        {
            Some("Character could not be loaded. The previous character is unchanged.".to_owned())
        } else if world
            .resource::<Time<Real>>()
            .elapsed()
            .saturating_sub(pending.started)
            >= LOAD_TIMEOUT
        {
            Some("Character loading timed out. The previous character is unchanged.".to_owned())
        } else {
            None
        };
        if let Some(reason) = failure {
            if let Some(entity) = pending.entity {
                world.despawn(entity);
            }
            import_failed(world, reason);
            return;
        }
        if pending.entity.is_none() && assets.is_loaded_with_dependencies(pending.handle.id()) {
            // Source bytes were validated before loading. Scene initialization
            // happens on a hidden, unmarked rig, so desktop queries keep the old avatar.
            pending.entity = Some(
                world
                    .spawn((
                        Name::new("Loading character"),
                        Visibility::Hidden,
                        VrmHandle(pending.handle.clone()),
                        Transform::from_xyz(0.0, -0.85, 0.0),
                    ))
                    .id(),
            );
        }
        if let Some(entity) = pending.entity
            && world.get::<Initialized>(entity).is_some()
            && world.get::<HipsBoneEntity>(entity).is_some()
        {
            commit_character(
                world,
                entity,
                &pending.path,
                pending.converted,
                pending.migrated_from_vrm0,
            );
            return;
        }
        runtime.pending = Some(pending);
    });
}

fn import_failed(world: &mut World, reason: String) {
    warn!("Character import: {reason}");
    let mut status = world.resource_mut::<CharacterImportStatus>();
    status.loading = false;
    status.message = reason;
}

fn commit_character(
    world: &mut World,
    entity: Entity,
    path: &str,
    converted: bool,
    migrated_from_vrm0: bool,
) {
    crate::cloud::clear_for_character_change(world);
    crate::pet_feedback::clear_for_character_change(world);
    let old: Vec<_> = world
        .query_filtered::<Entity, With<NeraAvatar>>()
        .iter(world)
        .collect();
    let mut transform = old
        .first()
        .and_then(|old| world.get::<Transform>(*old))
        .copied()
        .unwrap_or_else(|| Transform::from_xyz(0.0, -0.85, 0.0));
    let visibility = old
        .first()
        .map(|old| replacement_visibility(world, *old))
        .unwrap_or(Visibility::Inherited);
    // The walking pose system reapplies direction from the model's neutral yaw.
    transform.rotation = Quat::IDENTITY;
    let target = world
        .query_filtered::<Entity, With<LookAtTarget>>()
        .iter(world)
        .next();
    crate::animation::reset_for_character_change(world);
    for old in old {
        world.despawn(old);
    }
    world
        .entity_mut(entity)
        .insert((NeraAvatar, transform, Visibility::Hidden));
    if visibility != Visibility::Hidden {
        let started = world.resource::<Time<Real>>().elapsed();
        world.entity_mut(entity).insert(CharacterReveal {
            visibility,
            ready_frames: 0,
            started,
            converted,
        });
    }
    if let Some(target) = target {
        world.entity_mut(entity).insert(LookAt::Target(target));
    }
    let asset_server = world.resource::<AssetServer>().clone();
    world.commands().entity(entity).with_children(|parent| {
        crate::animation::spawn_animation_library(parent, &asset_server);
    });
    let mut active = world.resource_mut::<ActiveCharacter>();
    active.asset_path = path.to_owned();
    active.migrated_from_vrm0 = migrated_from_vrm0;
    world.resource_mut::<NeraSettings>().character_asset = if path == DEFAULT_CHARACTER {
        String::new()
    } else {
        path.to_owned()
    };
    crate::settings::save_settings_deferred(&mut world.commands());
    let mut status = world.resource_mut::<CharacterImportStatus>();
    status.loading = visibility != Visibility::Hidden;
    status.message = if converted && status.loading {
        "VRM 0.x converted to 1.0; preparing character animation..."
    } else if converted {
        "VRM 0.x converted to 1.0; character changed."
    } else if status.loading {
        "Preparing character animation..."
    } else {
        "Character changed."
    }
    .into();
    info!("Character changed to {path}");
}

fn replacement_visibility(world: &World, entity: Entity) -> Visibility {
    world
        .get::<CharacterReveal>(entity)
        .map(|pending| pending.visibility)
        .or_else(|| world.get::<Visibility>(entity).copied())
        .unwrap_or(Visibility::Inherited)
}

fn reveal_character(world: &mut World) {
    let ready = crate::animation::character_animation_ready(world);
    let now = world.resource::<Time<Real>>().elapsed();
    let mut finished = Vec::new();
    for (entity, mut reveal, mut visibility) in world
        .query::<(Entity, &mut CharacterReveal, &mut Visibility)>()
        .iter_mut(world)
    {
        reveal.ready_frames = if ready {
            reveal.ready_frames.saturating_add(1)
        } else {
            0
        };
        let timed_out = now.saturating_sub(reveal.started) >= LOAD_TIMEOUT;
        if reveal.ready_frames >= 3 || timed_out {
            *visibility = reveal.visibility;
            finished.push((entity, timed_out, reveal.converted));
        }
    }
    for (entity, timed_out, converted) in finished {
        world.entity_mut(entity).remove::<CharacterReveal>();
        if world
            .get_resource::<CharacterImport>()
            .is_some_and(|runtime| runtime.copy.is_some() || runtime.pending.is_some())
        {
            continue;
        }
        let mut status = world.resource_mut::<CharacterImportStatus>();
        status.loading = false;
        status.message = if timed_out {
            "Character loaded, but its animation is not ready."
        } else if converted {
            "VRM 0.x converted to 1.0; character changed."
        } else {
            "Character changed."
        }
        .into();
    }
}

fn read_valid_model(path: &Path) -> Result<crate::vrm0_migration::MigratedVrm, String> {
    if !path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("vrm"))
    {
        return Err("Choose a character .vrm file (animations use .vrma).".into());
    }
    let file = std::fs::File::open(path).map_err(|e| format!("Cannot open character: {e}"))?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_MODEL_BYTES {
        return Err("Character exceeds the 128 MiB import limit.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_MODEL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_MODEL_BYTES {
        return Err("Character exceeds the 128 MiB import limit.".into());
    }
    let migrated = crate::vrm0_migration::migrate_vrm0(bytes)?;
    if migrated.bytes.len() as u64 > MAX_MODEL_BYTES {
        return Err("Converted character exceeds the 128 MiB import limit.".into());
    }
    validate_glb(&migrated.bytes)?;
    Ok(migrated)
}

fn import_model(path: &Path, directory: &Path) -> Result<ImportedCharacter, String> {
    let migrated = read_valid_model(path)?;
    store_imported(migrated, directory)
}

fn store_imported(
    migrated: crate::vrm0_migration::MigratedVrm,
    directory: &Path,
) -> Result<ImportedCharacter, String> {
    let bytes = migrated.bytes;
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    let name = format!("model-{:016x}.vrm", hash.finish());
    std::fs::create_dir_all(directory)
        .map_err(|e| format!("Cannot create character library: {e}"))?;
    let destination = directory.join(&name);
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
    {
        Ok(mut file) => {
            if let Err(error) = file.write_all(&bytes) {
                drop(file);
                let _ = std::fs::remove_file(&destination);
                return Err(format!("Cannot save character: {error}"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if std::fs::read(&destination).map_err(|e| e.to_string())? != bytes {
                return Err("Stored character file differs; import was not applied.".into());
            }
        }
        Err(error) => return Err(format!("Cannot save character: {error}")),
    }
    Ok(ImportedCharacter {
        path: format!("characters://{name}"),
        converted: migrated.converted,
        migrated_from_vrm0: migrated.migrated_from_vrm0,
    })
}

fn validate_glb(bytes: &[u8]) -> Result<(), String> {
    let u32_at = |offset| {
        bytes
            .get(offset..offset + 4)
            .and_then(|v| v.try_into().ok())
            .map(u32::from_le_bytes)
    };
    if bytes.get(..4) != Some(b"glTF")
        || u32_at(4) != Some(2)
        || u32_at(8).map(|v| v as usize) != Some(bytes.len())
        || u32_at(16) != Some(0x4E4F534A)
    {
        return Err("This is not a valid binary VRM 1.0 file.".into());
    }
    let end = u32_at(12)
        .and_then(|len| 20usize.checked_add(len as usize))
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "Character JSON chunk is incomplete.".to_owned())?;
    let document: serde_json::Value = serde_json::from_slice(&bytes[20..end])
        .map_err(|_| "Character metadata is invalid.".to_owned())?;
    validate_document(&document)
}

fn validate_document(document: &serde_json::Value) -> Result<(), String> {
    let extension = document.pointer("/extensions/VRMC_vrm").ok_or_else(|| {
        if document.pointer("/extensions/VRM").is_some() {
            "VRM 0.x conversion did not produce valid VRM 1.0 data.".to_owned()
        } else {
            "This file has no VRM 1.0 character data.".to_owned()
        }
    })?;
    if extension.get("specVersion").and_then(|v| v.as_str()) != Some("1.0") {
        return Err("This character must be exported as VRM 1.0.".into());
    }
    // Parse the dependency's exact extension types before its initialization:
    // its spring-bone path unwraps malformed data instead of reporting an error.
    let parsed = serde_json::from_value::<VrmExtensions>(document["extensions"].clone())
        .map_err(|e| format!("Unsupported or invalid VRM metadata: {e}"))?;
    if let Some(spring_bone) = parsed.vrmc_spring_bone
        && spring_bone.springs.iter().any(|spring| {
            spring
                .collider_groups
                .iter()
                .flatten()
                .any(|index| *index >= spring_bone.collider_groups.len())
        })
    {
        return Err("Character has an invalid spring collider group index.".into());
    }
    if document
        .get("scenes")
        .and_then(|v| v.as_array())
        .is_none_or(|scenes| scenes.is_empty())
    {
        return Err("Character has no usable scene.".into());
    }
    let nodes = document
        .get("nodes")
        .and_then(|v| v.as_array())
        .ok_or("Character has no skeleton.")?;
    if let Some(expressions) = parsed.vrmc_vrm.expressions {
        for bind in expressions
            .preset
            .values()
            .flat_map(|preset| preset.morph_target_binds.iter().flatten())
        {
            let mesh = nodes
                .get(bind.node)
                .and_then(|node| node.get("mesh"))
                .and_then(|mesh| mesh.as_u64())
                .and_then(|index| document.get("meshes")?.as_array()?.get(index as usize))
                .ok_or("Character has an invalid expression mesh.")?;
            let primitives = mesh
                .get("primitives")
                .and_then(|value| value.as_array())
                .ok_or("Character expression mesh has no primitives.")?;
            if primitives.is_empty()
                || primitives.iter().any(|primitive| {
                    primitive
                        .get("targets")
                        .and_then(|targets| targets.as_array())
                        .is_none_or(|targets| bind.index >= targets.len())
                })
                || nodes[bind.node]
                    .get("weights")
                    .and_then(|weights| weights.as_array())
                    .is_some_and(|weights| bind.index >= weights.len())
                || mesh
                    .get("weights")
                    .and_then(|weights| weights.as_array())
                    .is_some_and(|weights| bind.index >= weights.len())
            {
                return Err("Character has an invalid expression morph target index.".into());
            }
        }
    }
    let bones = extension
        .pointer("/humanoid/humanBones")
        .and_then(|v| v.as_object())
        .ok_or("Character has no humanoid mapping.")?;
    for bone in [
        "hips",
        "spine",
        "head",
        "leftUpperArm",
        "leftLowerArm",
        "leftHand",
        "rightUpperArm",
        "rightLowerArm",
        "rightHand",
        "leftUpperLeg",
        "leftLowerLeg",
        "leftFoot",
        "rightUpperLeg",
        "rightLowerLeg",
        "rightFoot",
    ] {
        if !bones.contains_key(bone) {
            return Err(format!("Character is missing the required {bone} bone."));
        }
    }
    if bones.values().any(|bone| {
        bone.get("node")
            .and_then(|v| v.as_u64())
            .is_none_or(|node| node as usize >= nodes.len())
    }) {
        return Err("Character has an invalid humanoid bone index.".into());
    }
    for field in ["buffers", "images"] {
        if document
            .get(field)
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("uri").and_then(|v| v.as_str()))
            .any(|uri| !uri.starts_with("data:"))
        {
            return Err("Export a self-contained VRM with textures embedded in the file.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn bundled_document() -> Value {
        let bytes = include_bytes!("../assets/vrm/nera_256.vrm");
        let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        serde_json::from_slice(&bytes[20..20 + length]).unwrap()
    }

    #[test]
    fn bundled_vrm_and_saved_asset_paths_are_supported() {
        assert!(validate_glb(include_bytes!("../assets/vrm/nera_256.vrm")).is_ok());
        assert_eq!(
            saved_filename("characters://model-123.vrm"),
            Some("model-123.vrm")
        );
        for path in [
            "characters://../model-x.vrm",
            "characters://model-../x.vrm",
            "C:/model-a.vrm",
            "characters://model-a.vrma",
        ] {
            assert_eq!(saved_filename(path), None);
        }
    }

    #[test]
    fn unsupported_and_malformed_models_fail_before_rig_initialization() {
        assert!(
            validate_document(&json!({"extensions": {"VRM": {}}}))
                .unwrap_err()
                .contains("VRM 0.x")
        );
        assert!(validate_document(&json!({"extensions": {"VRMC_vrm_animation": {}}})).is_err());
        let document = bundled_document();
        let mut invalid = document.clone();
        invalid["extensions"]["VRMC_vrm"]["humanoid"]["humanBones"]
            .as_object_mut()
            .unwrap()
            .remove("hips");
        assert!(validate_document(&invalid).unwrap_err().contains("hips"));
        let mut invalid = document.clone();
        invalid["extensions"]["VRMC_vrm"]["humanoid"]["humanBones"]["hips"]["node"] = json!(999999);
        assert!(
            validate_document(&invalid)
                .unwrap_err()
                .contains("bone index")
        );
        let mut invalid = document.clone();
        invalid["extensions"]["VRMC_springBone"] = json!({"springs": "broken"});
        assert!(validate_document(&invalid).is_err());
        let mut invalid = document.clone();
        invalid["extensions"]["VRMC_springBone"] = json!({
            "specVersion": "1.0", "colliderGroups": [],
            "springs": [{"name": "hair", "joints": [], "colliderGroups": [100]}]
        });
        assert!(
            validate_document(&invalid)
                .unwrap_err()
                .contains("collider group index")
        );
        let mut invalid = document.clone();
        let bind = invalid["extensions"]["VRMC_vrm"]["expressions"]["preset"]
            .as_object_mut()
            .unwrap()
            .values_mut()
            .find_map(|preset| {
                preset
                    .get_mut("morphTargetBinds")?
                    .as_array_mut()?
                    .first_mut()
            })
            .unwrap();
        bind["index"] = json!(999999);
        assert!(
            validate_document(&invalid)
                .unwrap_err()
                .contains("morph target index")
        );
        let mut invalid = document;
        invalid["images"][0]["uri"] = json!("../external.png");
        assert!(
            validate_document(&invalid)
                .unwrap_err()
                .contains("self-contained")
        );
        assert!(validate_glb(b"glTF").is_err());
    }

    #[test]
    fn import_is_self_contained_and_deduplicates_without_overwriting_source() {
        let directory = std::env::temp_dir().join(format!(
            "nera-character-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/vrm/nera_256.vrm");
        let first = import_model(&source, &directory).unwrap();
        let second = import_model(&source, &directory).unwrap();
        assert_eq!(first, second);
        assert!(!first.converted);
        let destination = directory.join(saved_filename(&first.path).unwrap());
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(&destination).unwrap()
        );
        std::fs::remove_file(destination).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn legacy_import_is_converted_to_a_private_vrm1_file() {
        let directory = std::env::temp_dir().join(format!(
            "nera-character-legacy-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/vrm/nera0.0.vrm");
        let original = std::fs::read(&source).unwrap();
        let imported = import_model(&source, &directory).unwrap();
        assert!(imported.converted);
        assert!(imported.migrated_from_vrm0);
        let destination = directory.join(saved_filename(&imported.path).unwrap());
        let converted = std::fs::read(&destination).unwrap();
        assert!(validate_glb(&converted).is_ok());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_ne!(converted, original);
        std::fs::remove_file(destination).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn stalled_rig_keeps_previous_avatar_and_persisted_character() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()))
            .init_asset::<VrmAsset>()
            .init_resource::<CharacterImport>()
            .init_resource::<CharacterImportStatus>()
            .init_resource::<ActiveCharacter>()
            .init_resource::<NeraSettings>();
        let old = app
            .world_mut()
            .spawn((NeraAvatar, Visibility::Visible))
            .id();
        let candidate = app.world_mut().spawn(Visibility::Hidden).id();
        // A handle with no loading task reproduces a VRM whose asset loaded but
        // never produced a humanoid rig, without relying on wall-clock sleeps.
        let handle = app
            .world_mut()
            .resource_mut::<Assets<VrmAsset>>()
            .reserve_handle();
        app.world_mut().resource_mut::<CharacterImport>().pending = Some(PendingCharacter {
            path: "characters://model-new.vrm".into(),
            converted: false,
            migrated_from_vrm0: false,
            handle,
            started: Duration::ZERO,
            entity: Some(candidate),
        });
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(LOAD_TIMEOUT);
        update_import(app.world_mut());
        assert!(app.world().get::<NeraAvatar>(old).is_some());
        assert_eq!(
            app.world().get::<Visibility>(old),
            Some(&Visibility::Visible)
        );
        assert!(app.world().get_entity(candidate).is_err());
        assert_eq!(
            app.world().resource::<ActiveCharacter>().asset_path,
            DEFAULT_CHARACTER
        );
        assert!(
            app.world()
                .resource::<NeraSettings>()
                .character_asset
                .is_empty()
        );
        assert!(app.world().resource::<CharacterImport>().pending.is_none());
        let status = app.world().resource::<CharacterImportStatus>();
        assert!(!status.loading);
        assert!(status.message.contains("timed out"));
    }

    #[test]
    fn missing_animation_cannot_leave_a_loaded_character_invisible_forever() {
        let mut world = World::new();
        world.init_resource::<crate::animation::NeraAnimationPlayback>();
        world.init_resource::<crate::animation::NeraAnimationState>();
        world.init_resource::<crate::animation::NeraAnimationClips>();
        world.init_resource::<CharacterImportStatus>();
        world.init_resource::<Time<Real>>();
        let avatar = world
            .spawn((
                Visibility::Hidden,
                CharacterReveal {
                    visibility: Visibility::Inherited,
                    ready_frames: 0,
                    started: Duration::ZERO,
                    converted: false,
                },
            ))
            .id();
        reveal_character(&mut world);
        assert_eq!(world.get::<Visibility>(avatar), Some(&Visibility::Hidden));
        world.resource_mut::<Time<Real>>().advance_by(LOAD_TIMEOUT);
        reveal_character(&mut world);
        assert_eq!(
            world.get::<Visibility>(avatar),
            Some(&Visibility::Inherited)
        );
        assert!(world.get::<CharacterReveal>(avatar).is_none());
        assert!(
            world
                .resource::<CharacterImportStatus>()
                .message
                .contains("animation is not ready")
        );
    }

    #[test]
    fn replacing_a_character_during_reveal_keeps_the_intended_visibility() {
        let mut world = World::new();
        let first = world
            .spawn((
                Visibility::Hidden,
                CharacterReveal {
                    visibility: Visibility::Inherited,
                    ready_frames: 0,
                    started: Duration::ZERO,
                    converted: false,
                },
            ))
            .id();
        assert_eq!(replacement_visibility(&world, first), Visibility::Inherited);
        let startup_avatar = world.spawn(Visibility::Hidden).id();
        assert_eq!(
            replacement_visibility(&world, startup_avatar),
            Visibility::Hidden
        );
    }
}
