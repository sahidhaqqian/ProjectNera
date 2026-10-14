//! Load a gallery selection on the desktop rig, then use the normal controller.
use std::{collections::HashMap, time::Duration};

use bevy::{ecs::system::SystemParam, prelude::*};
use bevy_vrm1::prelude::*;

use super::{NeraAnimationClips, NeraAnimationState, TemporaryAction, playback::ClipSlot};
use crate::avatar::NeraAvatar;

const MAX_LIBRARY_CLIPS: usize = 32;
const MAX_LIBRARY_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_CLIP_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const LOAD_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Message)]
pub(crate) struct PlayLibraryAnimation {
    pub path: String,
}

#[derive(Message)]
pub(crate) struct LibraryAnimationError {
    pub path: String,
    pub reason: String,
}

struct LoadingClip {
    path: String,
    handle: Handle<VrmaAsset>,
    entity: Option<Entity>,
    started: Duration,
    source_bytes: u64,
}

#[derive(Clone, Copy)]
struct CachedClip {
    entity: Entity,
    source_bytes: u64,
    last_used: u64,
    body_contact: bool,
}

#[derive(Component)]
pub(super) struct LibrarySource {
    pub asset: Handle<VrmaAsset>,
}

#[derive(Resource, Default)]
pub(crate) struct LibraryPlayback {
    clips: HashMap<String, CachedClip>,
    source_bytes: u64,
    access_clock: u64,
    requested: Option<String>,
    requested_auto: bool,
    loading: Option<LoadingClip>,
}

impl LibraryPlayback {
    pub(crate) fn request_auto_dance(&mut self, path: String) {
        if self.manual_pending() {
            return;
        }
        self.requested = Some(path);
        self.requested_auto = true;
    }
    pub(crate) fn cancel_auto_dance(&mut self) {
        if self.requested_auto {
            self.requested = None;
            self.requested_auto = false;
        }
    }
    pub(crate) fn auto_dance_pending(&self, path: &str) -> bool {
        self.requested_auto && self.requested.as_deref() == Some(path)
    }
    pub(crate) fn manual_pending(&self) -> bool {
        self.requested.is_some() && !self.requested_auto
    }
    pub(crate) fn path_for(&self, entity: Entity) -> Option<&str> {
        self.clips
            .iter()
            .find(|(_, clip)| clip.entity == entity)
            .map(|(path, _)| path.as_str())
    }
    pub(crate) fn grounding_path_for(&self, entity: Entity) -> Option<&str> {
        self.clips
            .iter()
            .find(|(_, clip)| clip.entity == entity)
            .map(|(path, clip)| {
                if clip.body_contact {
                    super::playback::DEATH_VRMA_PATH
                } else {
                    path.as_str()
                }
            })
    }
    fn validate_source_size(source_bytes: u64) -> Result<(), &'static str> {
        if source_bytes > MAX_CLIP_SOURCE_BYTES {
            return Err("Animation exceeds the 16 MiB desktop playback limit.");
        }
        Ok(())
    }

    pub(super) fn cache_clip(&mut self, path: String, entity: Entity, source_bytes: u64) {
        // Library/bookmark copies can have a different path from the protected
        // base clip. Preserve its contact semantics by identity, never by name.
        // Read once on admission; no filesystem work occurs in pose evaluation.
        let body_contact = path == super::playback::DEATH_VRMA_PATH
            || same_clip_contents(
                &crate::nera_asset_directory().join(&path),
                &crate::nera_asset_directory().join(super::playback::DEATH_VRMA_PATH),
            );
        self.access_clock = self.access_clock.wrapping_add(1);
        self.source_bytes = self.source_bytes.saturating_add(source_bytes);
        self.clips.insert(
            path,
            CachedClip {
                entity,
                source_bytes,
                last_used: self.access_clock,
                body_contact,
            },
        );
    }

    fn cached_entity(&mut self, path: &str) -> Option<Entity> {
        self.access_clock = self.access_clock.wrapping_add(1);
        let clip = self.clips.get_mut(path)?;
        clip.last_used = self.access_clock;
        Some(clip.entity)
    }

    fn evict_for(&mut self, source_bytes: u64, protected: &[Entity]) -> Vec<Entity> {
        let mut evicted = Vec::new();
        while self.clips.len() >= MAX_LIBRARY_CLIPS
            || self.source_bytes.saturating_add(source_bytes) > MAX_LIBRARY_SOURCE_BYTES
        {
            let candidate = self
                .clips
                .iter()
                .filter(|(_, clip)| !protected.contains(&clip.entity))
                .min_by(|(path_a, clip_a), (path_b, clip_b)| {
                    clip_a
                        .last_used
                        .cmp(&clip_b.last_used)
                        .then_with(|| path_a.cmp(path_b))
                })
                .map(|(path, _)| path.clone());
            let Some(path) = candidate else { break };
            let clip = self.clips.remove(&path).expect("LRU candidate exists");
            self.source_bytes = self.source_bytes.saturating_sub(clip.source_bytes);
            evicted.push(clip.entity);
            info!("Evicted desktop animation cache entry: {path}");
        }
        evicted
    }

    fn reject(
        &mut self,
        path: String,
        reason: String,
        errors: &mut MessageWriter<LibraryAnimationError>,
    ) {
        warn!("Could not play gallery animation {path}: {reason}");
        let automatic = self.requested.as_ref() == Some(&path) && self.requested_auto;
        if self.requested.as_ref() == Some(&path) {
            self.requested = None;
            self.requested_auto = false;
        }
        if !automatic {
            errors.write(LibraryAnimationError { path, reason });
        }
    }
}

fn same_clip_contents(source: &std::path::Path, reference: &std::path::Path) -> bool {
    let (Ok(source_info), Ok(reference_info)) = (source.metadata(), reference.metadata()) else {
        return false;
    };
    if source_info.len() == 0 || source_info.len() != reference_info.len() {
        return false;
    }
    fn fingerprint(path: &std::path::Path) -> Option<blake3::Hash> {
        use std::io::Read;
        let mut file = std::fs::File::open(path).ok()?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let bytes = file.read(&mut buffer).ok()?;
            if bytes == 0 {
                break;
            }
            hasher.update(&buffer[..bytes]);
        }
        Some(hasher.finalize())
    }
    match (fingerprint(source), fingerprint(reference)) {
        (Some(source), Some(reference)) => source == reference,
        _ => false,
    }
}

#[derive(Component)]
pub(super) struct LibraryClipReady {
    duration: Duration,
    warmup: u8,
}

fn loaded(trigger: On<LoadedVrma>, durations: Query<&VrmaDuration>, mut commands: Commands) {
    if let Ok(duration) = durations.get(trigger.vrma) {
        commands.entity(trigger.vrma).insert(LibraryClipReady {
            duration: duration.0,
            warmup: 3,
        });
    }
}

fn validate_bone_mapping(
    animation_extension: &serde_json::Value,
    model_extension: Option<&serde_json::Value>,
    node_count: usize,
) -> Result<(), &'static str> {
    if let Some(model) = model_extension {
        validate_bone_mapping(animation_extension, None, node_count)?;
        return validate_bone_mapping(model, None, node_count);
    }
    // The dependency prefers VRMC_vrm when both extensions are present. Check
    // the mapping it will actually deserialize, including its required hips.
    let extension = model_extension.unwrap_or(animation_extension);
    let bones = extension
        .pointer("/humanoid/humanBones")
        .and_then(|value| value.as_object())
        .filter(|bones| !bones.is_empty())
        .ok_or("VRMA has no humanoid bone mapping.")?;
    // bevy_vrm1's humanoid initialization unwraps this exact registry key.
    // A nonempty partial mapping without hips would panic before our timeout.
    if !bones.contains_key("hips") {
        return Err("VRMA is missing the required hips bone mapping.");
    }
    if bones.values().any(|bone| {
        bone.get("node")
            .and_then(|node| node.as_u64())
            .is_none_or(|node| node as usize >= node_count)
    }) {
        return Err("VRMA has an invalid humanoid bone mapping.");
    }
    Ok(())
}

/// Preflight self-contained user imports before publishing them in the library.
/// Preview/runtime still validate the actual Bevy clip before attaching VrmaHandle.
pub(crate) fn validate_import(bytes: &[u8]) -> Result<(), &'static str> {
    let gltf = bevy::gltf::gltf::Gltf::from_slice(bytes).map_err(|_| "Invalid glTF/VRMA file.")?;
    let document = &gltf.document;
    let extension = document
        .extension_value("VRMC_vrm_animation")
        .ok_or("Missing VRMC_vrm_animation extension.")?;
    validate_bone_mapping(
        extension,
        document.extension_value("VRMC_vrm"),
        document.nodes().len(),
    )?;
    if document.scenes().next().is_none() {
        return Err("VRMA has no scene.");
    }
    let blob = gltf
        .blob
        .as_deref()
        .ok_or("Use a self-contained binary VRMA file.")?;
    if document.buffers().len() != 1 {
        return Err("VRMA must have one embedded buffer.");
    }
    let mut first_playable = false;
    for (index, animation) in document.animations().enumerate() {
        let mut duration = 0.0_f32;
        for channel in animation.channels() {
            let reader = channel.reader(|buffer| (buffer.index() == 0).then_some(blob));
            let input = reader.read_inputs().ok_or("Invalid animation input.")?;
            let mut previous = None;
            for time in input {
                if !time.is_finite() || time < 0.0 || previous.is_some_and(|last| time <= last) {
                    return Err("Invalid animation keyframe times.");
                }
                previous = Some(time);
                duration = duration.max(time);
            }
            if reader.read_outputs().is_none() {
                return Err("Invalid animation output.");
            }
        }
        if Duration::try_from_secs_f64(f64::from(duration)).is_err() {
            return Err("Invalid animation duration.");
        }
        if index == 0 {
            first_playable = duration > 0.0;
        }
    }
    if !first_playable {
        return Err("VRMA has no playable keyframes.");
    }
    Ok(())
}

pub(crate) fn validate_asset(
    asset: &VrmaAsset,
    clips: &Assets<AnimationClip>,
) -> Result<(), &'static str> {
    let source = asset
        .gltf
        .source
        .as_ref()
        .ok_or("VRMA source is missing.")?;
    let extension = source
        .document
        .extension_value("VRMC_vrm_animation")
        .ok_or("This file is glTF but has no VRMC_vrm_animation extension.")?;
    validate_bone_mapping(
        extension,
        source.document.extension_value("VRMC_vrm"),
        asset.gltf.nodes.len(),
    )?;
    if asset.gltf.scenes.is_empty() {
        return Err("VRMA has no scene.");
    }
    let clip = asset
        .gltf
        .animations
        .first()
        .and_then(|handle| clips.get(handle))
        .ok_or("VRMA has no animation clip.")?;
    if !clip.duration().is_finite() || clip.duration() <= 0.0 || clip.curves().is_empty() {
        return Err("VRMA has no playable keyframes.");
    }
    validate_clip_durations(&asset.gltf.animations, clips)?;
    Ok(())
}

fn validate_clip_durations(
    handles: &[Handle<AnimationClip>],
    clips: &Assets<AnimationClip>,
) -> Result<(), &'static str> {
    // The dependency converts the maximum duration of every referenced clip,
    // even though desktop playback uses the first. Reject overflowing or
    // invalid secondary durations before Duration::from_secs_f64 can panic.
    for clip in handles.iter().filter_map(|handle| clips.get(handle)) {
        if Duration::try_from_secs_f64(f64::from(clip.duration())).is_err() {
            return Err("VRMA contains an invalid or overflowing animation duration.");
        }
    }
    Ok(())
}

#[derive(SystemParam)]
pub(super) struct LibraryAdmission<'w> {
    interaction: Option<Res<'w, crate::interaction::DesktopPetInteraction>>,
    presentation: Option<Res<'w, crate::presentation::PresentationState>>,
    loading: Option<Res<'w, crate::character::CharacterImportStatus>>,
}
impl LibraryAdmission<'_> {
    fn blocked(&self) -> bool {
        self.interaction.as_ref().is_some_and(|i| {
            i.state == crate::interaction::InteractionState::Dragging || i.drag_offset.is_some()
        }) || self
            .presentation
            .as_ref()
            .is_some_and(|p| !p.is_desktop_pet())
            || self.loading.as_ref().is_some_and(|s| s.loading)
    }
}

pub(super) fn sync_library_animation(
    mut requests: MessageReader<PlayLibraryAnimation>,
    avatar: Query<Entity, (With<NeraAvatar>, With<Initialized>)>,
    assets: Res<AssetServer>,
    vrma_assets: Res<Assets<VrmaAsset>>,
    animation_clips: Res<Assets<AnimationClip>>,
    time: Res<Time<Real>>,
    mut errors: MessageWriter<LibraryAnimationError>,
    mut runtime: ResMut<LibraryPlayback>,
    mut ready: Query<(Entity, &mut LibraryClipReady)>,
    initializing: Query<&ChildOf, Or<(With<VrmaHandle>, (With<Vrma>, Without<Initialized>))>>,
    mut clips: ResMut<NeraAnimationClips>,
    mut state: ResMut<NeraAnimationState>,
    mut commands: Commands,
    admission: LibraryAdmission,
) {
    // Last click wins, including when the previous selection is still loading.
    if let Some(request) = requests.read().last() {
        runtime.requested = Some(request.path.clone());
        runtime.requested_auto = false;
        state.cancel_auto_dance();
    }
    if runtime.manual_pending() && admission.blocked() {
        let path = runtime.requested.take().unwrap();
        runtime.reject(
            path,
            "Another interaction or presentation currently owns Nera.".into(),
            &mut errors,
        );
    }
    for (_, mut loaded) in &mut ready {
        loaded.warmup = loaded.warmup.saturating_sub(1);
    }
    if let Some(mut loading) = runtime.loading.take() {
        let clip_ready = loading
            .entity
            .and_then(|entity| ready.get(entity).ok().map(|(_, clip)| clip.warmup == 0));
        // LoadedVrma already committed this clip to the graph. Its remaining
        // warmup is bounded by frames; a suspended app must not time it out and
        // remove a successfully registered node when it resumes.
        let failure = if clip_ready.is_some() {
            None
        } else if assets
            .get_load_state(loading.handle.id())
            .is_some_and(|s| s.is_failed())
            || assets
                .get_recursive_dependency_load_state(loading.handle.id())
                .is_some_and(|s| s.is_failed())
        {
            Some("The animation file could not be loaded.".to_owned())
        } else if time.elapsed().saturating_sub(loading.started) >= LOAD_TIMEOUT {
            Some("Animation loading timed out. Select another animation or try again.".to_owned())
        } else if loading.entity.is_none()
            && assets.is_loaded_with_dependencies(loading.handle.id())
        {
            vrma_assets
                .get(&loading.handle)
                .map(|asset| validate_asset(asset, &animation_clips))
                .transpose()
                .err()
                .map(str::to_owned)
        } else {
            None
        };
        if let Some(reason) = failure {
            if let Some(entity) = loading.entity {
                commands.entity(entity).despawn();
            }
            runtime.reject(loading.path, reason, &mut errors);
        } else if clip_ready == Some(true) {
            runtime.cache_clip(loading.path, loading.entity.unwrap(), loading.source_bytes);
        } else {
            if loading.entity.is_none()
                && assets.is_loaded_with_dependencies(loading.handle.id())
                && vrma_assets.contains(loading.handle.id())
                && let Ok(root) = avatar.single()
                && !initializing.iter().any(|parent| parent.parent() == root)
            {
                // Validation happens before the dependency can consume VrmaHandle.
                loading.entity = Some(
                    commands
                        .spawn((
                            Name::new(format!("Nera Library: {}", loading.path)),
                            VrmaHandle(loading.handle.clone()),
                            LibrarySource {
                                asset: loading.handle.clone(),
                            },
                            ChildOf(root),
                        ))
                        .observe(loaded)
                        .id(),
                );
                loading.started = time.elapsed();
            }
            runtime.loading = Some(loading);
        }
    }
    let Some(path) = runtime.requested.clone() else {
        return;
    };
    let cached = runtime.cached_entity(&path).and_then(|entity| {
        if ready.get(entity).is_ok() {
            Some(entity)
        } else {
            if let Some(stale) = runtime.clips.remove(&path) {
                runtime.source_bytes = runtime.source_bytes.saturating_sub(stale.source_bytes);
            }
            None
        }
    });
    if let Some(entity) = cached {
        let (_, loaded) = ready.get(entity).expect("cached entity checked above");
        if loaded.warmup != 0 {
            return;
        }
        if loaded.duration.is_zero() {
            runtime.reject(
                path,
                "Animation has no playable duration.".into(),
                &mut errors,
            );
            return;
        }
        let automatic = runtime.requested_auto;
        runtime.requested = None;
        runtime.requested_auto = false;
        clips.library = Some(ClipSlot {
            entity,
            duration: loaded.duration,
        });
        if automatic {
            if state.request_auto_dance(entity, loaded.duration) {
                info!(
                    "AutoDance playing {path} duration={:.2}s",
                    loaded.duration.as_secs_f32()
                );
            }
        } else {
            info!(
                "Library playback | path={path} grounding={:?}",
                super::grounding_policy(
                    super::VisualState::LibraryAnimation(entity),
                    runtime.grounding_path_for(entity)
                )
            );
            state.request_one_shot(TemporaryAction::LibraryAnimation(entity), loaded.duration);
        }
        return;
    }
    let Ok(root) = avatar.single() else {
        return;
    };
    // Append one fully baked clip at a time. If B finished loading before A,
    // the dependency would insert A ahead of B and renumber the playing node.
    if runtime.loading.is_some() || initializing.iter().any(|parent| parent.parent() == root) {
        return;
    }
    let source_bytes =
        match std::fs::metadata(std::path::Path::new(&crate::nera_asset_path()).join(&path)) {
            Ok(metadata) => metadata.len(),
            Err(error) => {
                runtime.reject(
                    path,
                    format!("Cannot read animation file: {error}"),
                    &mut errors,
                );
                return;
            }
        };
    if let Err(reason) = LibraryPlayback::validate_source_size(source_bytes) {
        runtime.reject(path, reason.into(), &mut errors);
        return;
    }
    let protected: Vec<_> = state
        .active_actions
        .iter()
        .filter_map(|active| match active.action {
            TemporaryAction::LibraryAnimation(entity) | TemporaryAction::AutoDance(entity) => {
                Some(entity)
            }
            _ => None,
        })
        .collect();
    for entity in runtime.evict_for(source_bytes, &protected) {
        commands.entity(entity).despawn();
    }
    runtime.loading = Some(LoadingClip {
        handle: assets.load(path.clone()),
        path,
        entity: None,
        started: time.elapsed(),
        source_bytes,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{BasePose, VisualState, playback::clip_for_visual};
    #[test]
    fn stale_cached_entity_is_removed_and_the_pending_selection_can_load_again() {
        let mut app = library_app(crate::nera_asset_path());
        app.finish();
        app.cleanup();
        let entity = app
            .world_mut()
            .spawn(LibraryClipReady {
                duration: Duration::from_secs(2),
                warmup: 0,
            })
            .id();
        let path = "animations/animasi_basic/idle.vrma";
        app.world_mut()
            .resource_mut::<LibraryPlayback>()
            .cache_clip(path.into(), entity, 100);
        app.world_mut().despawn(entity);
        app.world_mut()
            .write_message(PlayLibraryAnimation { path: path.into() });
        app.update();
        let runtime = app.world().resource::<LibraryPlayback>();
        assert!(runtime.clips.is_empty());
        assert_eq!(runtime.source_bytes, 0);
        assert_eq!(
            runtime
                .loading
                .as_ref()
                .map(|loading| loading.path.as_str()),
            Some(path)
        );
    }
    #[test]
    fn rejected_auto_dance_releases_request_flags_without_a_manual_ui_error() {
        for automatic in [false, true] {
            let mut app = library_app(crate::nera_asset_path());
            app.finish();
            app.cleanup();
            let path = "animations/nera-missing-audit-fixture.vrma";
            if automatic {
                app.world_mut()
                    .resource_mut::<LibraryPlayback>()
                    .request_auto_dance(path.into());
            } else {
                app.world_mut()
                    .write_message(PlayLibraryAnimation { path: path.into() });
            }
            app.update();
            let runtime = app.world().resource::<LibraryPlayback>();
            assert!(runtime.requested.is_none());
            assert!(!runtime.requested_auto);
            assert_eq!(
                app.world()
                    .resource::<Messages<LibraryAnimationError>>()
                    .is_empty(),
                automatic
            );
        }
    }

    #[test]
    fn body_contact_aliases_require_identical_contents_not_a_filename() {
        let directory =
            std::env::temp_dir().join(format!("nera-contact-identity-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let reference = directory.join("base.vrma");
        let renamed = directory.join("renamed-bookmark.vrma");
        std::fs::write(&reference, b"known clip contents").unwrap();
        std::fs::write(&renamed, b"known clip contents").unwrap();
        assert!(same_clip_contents(&renamed, &reference));
        // Equal length but different content must not inherit Death semantics.
        std::fs::write(&renamed, b"other clip contents").unwrap();
        assert!(!same_clip_contents(&renamed, &reference));
        assert!(!same_clip_contents(
            &directory.join("missing.vrma"),
            &reference
        ));
        std::fs::remove_file(reference).unwrap();
        std::fs::remove_file(renamed).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn library_contact_alias_retains_its_real_path_and_protected_contact_policy() {
        let mut library = LibraryPlayback::default();
        let entity = Entity::from_raw_u32(123).unwrap();
        let path = "animations/Bookmarks/RenamedReaction.vrma";
        library.clips.insert(
            path.into(),
            CachedClip {
                entity,
                source_bytes: 100,
                last_used: 0,
                body_contact: true,
            },
        );
        assert_eq!(library.path_for(entity), Some(path));
        assert_eq!(
            library.grounding_path_for(entity),
            Some(super::super::playback::DEATH_VRMA_PATH)
        );
        assert_eq!(
            crate::animation::grounding_policy(
                VisualState::LibraryAnimation(entity),
                library.grounding_path_for(entity)
            ),
            crate::animation::GroundingPolicy::BodyContact
        );
        assert!(!super::super::library_motion::uses_checkpoint_motion(
            VisualState::LibraryAnimation(entity),
            library.grounding_path_for(entity)
        ));
        library.clips.get_mut(path).unwrap().body_contact = false;
        assert_eq!(library.grounding_path_for(entity), Some(path));
        assert_eq!(
            crate::animation::grounding_policy(
                VisualState::LibraryAnimation(entity),
                library.grounding_path_for(entity)
            ),
            crate::animation::GroundingPolicy::Authored
        );
    }

    #[test]
    fn renamed_protected_death_is_recognized_when_admitted_to_the_library_cache() {
        let source = crate::nera_asset_file(super::super::playback::DEATH_VRMA_PATH);
        let copy =
            std::env::temp_dir().join(format!("nera-renamed-contact-{}.vrma", std::process::id()));
        std::fs::copy(source, &copy).unwrap();
        let path = copy.to_string_lossy().into_owned();
        let entity = Entity::from_raw_u32(124).unwrap();
        let mut library = LibraryPlayback::default();
        library.cache_clip(path.clone(), entity, copy.metadata().unwrap().len());
        assert_eq!(library.path_for(entity), Some(path.as_str()));
        assert_eq!(
            library.grounding_path_for(entity),
            Some(super::super::playback::DEATH_VRMA_PATH)
        );
        assert!(!super::super::library_motion::uses_checkpoint_motion(
            VisualState::LibraryAnimation(entity),
            library.grounding_path_for(entity)
        ));
        std::fs::remove_file(copy).unwrap();
    }

    #[test]
    fn auto_dance_uses_cached_library_duration_and_manual_playback_wins() {
        let mut app = library_app(crate::nera_asset_path());
        app.finish();
        app.cleanup();
        let entity = app
            .world_mut()
            .spawn(LibraryClipReady {
                duration: Duration::from_secs(2),
                warmup: 0,
            })
            .id();
        let path = "animations/test-selected.vrma";
        {
            let mut library = app.world_mut().resource_mut::<LibraryPlayback>();
            library.cache_clip(path.into(), entity, 100);
            library.request_auto_dance(path.into());
        }
        app.update();
        assert!(
            app.world()
                .resource::<NeraAnimationState>()
                .auto_dance_active()
        );
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::LibraryAnimation(entity)
        );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .advance(Duration::from_secs(2));
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::Idle
        );
        app.world_mut()
            .resource_mut::<LibraryPlayback>()
            .request_auto_dance(path.into());
        app.world_mut()
            .write_message(PlayLibraryAnimation { path: path.into() });
        app.update();
        assert!(
            !app.world()
                .resource::<NeraAnimationState>()
                .auto_dance_active()
        );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .cancel_auto_dance();
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::LibraryAnimation(entity)
        );
    }

    fn library_app(asset_path: String) -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                file_path: asset_path,
                ..default()
            },
        ))
        .init_asset::<VrmaAsset>()
        .init_asset::<AnimationClip>()
        .init_resource::<NeraAnimationState>()
        .init_resource::<NeraAnimationClips>()
        .init_resource::<LibraryPlayback>()
        .add_message::<PlayLibraryAnimation>()
        .add_message::<LibraryAnimationError>()
        .add_systems(Update, sync_library_animation);
        app.world_mut().spawn((NeraAvatar, Initialized));
        app
    }

    #[test]
    fn stalled_attached_clip_is_removed_and_the_latest_selection_can_start() {
        use bevy::time::TimeUpdateStrategy;
        let mut app = library_app(crate::nera_asset_path());
        app.finish();
        app.cleanup();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(LOAD_TIMEOUT));
        let root = app
            .world_mut()
            .query_filtered::<Entity, With<NeraAvatar>>()
            .single(app.world())
            .unwrap();
        let stuck = app.world_mut().spawn((Vrma, ChildOf(root))).id();
        let next = "animations/animasi_basic/idle.vrma";
        {
            let mut runtime = app.world_mut().resource_mut::<LibraryPlayback>();
            runtime.requested = Some(next.into());
            runtime.loading = Some(LoadingClip {
                path: "stuck.vrma".into(),
                handle: default(),
                entity: Some(stuck),
                started: Duration::ZERO,
                source_bytes: 100,
            });
        }
        app.update();
        app.update();
        assert!(app.world().get_entity(stuck).is_err());
        assert!(app.world().resource::<LibraryPlayback>().loading.is_none());
        app.update();
        let runtime = app.world().resource::<LibraryPlayback>();
        assert_eq!(
            runtime.loading.as_ref().map(|clip| clip.path.as_str()),
            Some(next)
        );
        assert!(runtime.clips.is_empty());
        assert_eq!(runtime.source_bytes, 0);
    }

    #[test]
    fn suspension_during_ready_warmup_never_discards_a_registered_graph_node() {
        use bevy::time::TimeUpdateStrategy;
        let mut app = library_app(crate::nera_asset_path());
        app.finish();
        app.cleanup();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(LOAD_TIMEOUT));
        let entity = app
            .world_mut()
            .spawn(LibraryClipReady {
                duration: Duration::from_secs(2),
                warmup: 3,
            })
            .id();
        {
            let mut runtime = app.world_mut().resource_mut::<LibraryPlayback>();
            runtime.requested = Some("registered.vrma".into());
            runtime.loading = Some(LoadingClip {
                path: "registered.vrma".into(),
                handle: default(),
                entity: Some(entity),
                started: Duration::ZERO,
                source_bytes: 100,
            });
        }
        for _ in 0..3 {
            app.update();
        }
        let runtime = app.world().resource::<LibraryPlayback>();
        assert_eq!(runtime.clips["registered.vrma"].entity, entity);
        assert!(runtime.loading.is_none());
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::LibraryAnimation(entity)
        );
        assert!(
            app.world()
                .resource::<Messages<LibraryAnimationError>>()
                .is_empty()
        );
    }

    #[test]
    fn lru_eviction_keeps_recent_and_current_clips() {
        let mut app = library_app(crate::nera_asset_path());
        app.finish();
        app.cleanup();
        let retained: Vec<_> = (0..MAX_LIBRARY_CLIPS)
            .map(|index| {
                let entity = app
                    .world_mut()
                    .spawn(LibraryClipReady {
                        duration: Duration::from_secs(2),
                        warmup: 0,
                    })
                    .id();
                (format!("cached-{index}.vrma"), entity)
            })
            .collect();
        {
            let mut runtime = app.world_mut().resource_mut::<LibraryPlayback>();
            for (path, entity) in &retained {
                runtime.cache_clip(path.clone(), *entity, 1024);
            }
            assert_eq!(runtime.cached_entity("cached-0.vrma"), Some(retained[0].1));
            let evicted = runtime.evict_for(1024, &[retained[1].1]);
            assert_eq!(evicted, vec![retained[2].1]);
            assert!(runtime.clips.contains_key("cached-0.vrma"));
            assert!(runtime.clips.contains_key("cached-1.vrma"));
            assert!(!runtime.clips.contains_key("cached-2.vrma"));
            runtime.cache_clip("new.vrma".into(), Entity::PLACEHOLDER, 1024);
            assert_eq!(runtime.clips.len(), MAX_LIBRARY_CLIPS);
            assert_eq!(runtime.source_bytes, MAX_LIBRARY_CLIPS as u64 * 1024);
        }
    }

    #[test]
    fn full_cache_evicts_and_starts_the_new_selection_without_restart_error() {
        let mut app = library_app(crate::nera_asset_path());
        app.finish();
        app.cleanup();
        for index in 0..MAX_LIBRARY_CLIPS {
            let entity = app
                .world_mut()
                .spawn(LibraryClipReady {
                    duration: Duration::from_secs(1),
                    warmup: 0,
                })
                .id();
            app.world_mut()
                .resource_mut::<LibraryPlayback>()
                .cache_clip(format!("cached-{index}.vrma"), entity, 1024);
        }
        app.world_mut().write_message(PlayLibraryAnimation {
            path: "animations/animasi_basic/idle.vrma".into(),
        });

        app.update();

        let runtime = app.world().resource::<LibraryPlayback>();
        assert_eq!(runtime.clips.len(), MAX_LIBRARY_CLIPS - 1);
        assert_eq!(
            runtime
                .loading
                .as_ref()
                .map(|loading| loading.path.as_str()),
            Some("animations/animasi_basic/idle.vrma")
        );
        assert!(
            app.world()
                .resource::<Messages<LibraryAnimationError>>()
                .is_empty()
        );
    }

    #[test]
    fn source_budget_rejects_only_oversized_files_and_evicts_for_total_budget() {
        let mut runtime = LibraryPlayback::default();
        assert!(LibraryPlayback::validate_source_size(MAX_CLIP_SOURCE_BYTES).is_ok());
        assert!(LibraryPlayback::validate_source_size(MAX_CLIP_SOURCE_BYTES + 1).is_err());
        for index in 0..4 {
            runtime.cache_clip(
                format!("large-{index}.vrma"),
                Entity::from_bits(index + 1),
                MAX_CLIP_SOURCE_BYTES,
            );
        }
        assert_eq!(runtime.source_bytes, MAX_LIBRARY_SOURCE_BYTES);
        let evicted = runtime.evict_for(MAX_CLIP_SOURCE_BYTES, &[]);
        assert_eq!(evicted.len(), 1);
        assert_eq!(
            runtime.source_bytes,
            MAX_LIBRARY_SOURCE_BYTES - MAX_CLIP_SOURCE_BYTES
        );
    }

    #[test]
    fn incomplete_humanoid_mapping_is_rejected_before_dependency_can_unwrap_hips() {
        use serde_json::json;
        let partial = json!({"humanoid":{"humanBones":{"spine":{"node":0}}}});
        assert_eq!(
            validate_bone_mapping(&partial, None, 1),
            Err("VRMA is missing the required hips bone mapping.")
        );
        for node in [json!(null), json!(-1), json!(0.5), json!("0"), json!(2)] {
            let invalid = json!({"humanoid":{"humanBones":{"hips":{"node":node}}}});
            assert!(validate_bone_mapping(&invalid, None, 2).is_err());
        }
        let valid = json!({"humanoid":{"humanBones":{"hips":{"node":0},"spine":{"node":1}}}});
        assert!(validate_bone_mapping(&valid, None, 2).is_ok());
        // A valid animation extension must not hide a malformed VRMC_vrm map,
        // since the dependency prioritizes the latter in a combined file.
        assert!(validate_bone_mapping(&valid, Some(&partial), 2).is_err());
        assert!(validate_bone_mapping(&valid, Some(&valid), 2).is_ok());
    }

    #[test]
    fn malformed_secondary_clip_duration_is_rejected_before_dependency_conversion() {
        let mut clips = Assets::<AnimationClip>::default();
        let mut first = AnimationClip::default();
        first.set_duration(2.0);
        let first = clips.add(first);
        let mut second = AnimationClip::default();
        second.set_duration(5.0);
        let second = clips.add(second);
        let referenced = [first, second.clone()];
        assert!(validate_clip_durations(&referenced, &clips).is_ok());
        for duration in [f32::MAX, f32::INFINITY, f32::NAN, -1.0] {
            clips.get_mut(&second).unwrap().set_duration(duration);
            assert!(validate_clip_durations(&referenced, &clips).is_err());
            assert!(validate_clip_durations(&referenced[..1], &clips).is_ok());
        }
    }

    #[test]
    fn ordinary_gltf_loaded_successfully_is_rejected_before_rig_attachment() {
        use bevy::{
            gltf::GltfPlugin, mesh::MeshPlugin, window::RequestRedraw,
            world_serialization::WorldSerializationPlugin,
        };
        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let fixture = Fixture(std::env::temp_dir().join(format!(
                "nera-library-validation-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )));
        std::fs::create_dir(&fixture.0).unwrap();
        std::fs::write(fixture.0.join("not-vrma.vrma"), r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"name":"hips"}]}"#).unwrap();
        let mut app = library_app(fixture.0.to_string_lossy().into_owned());
        // This is the real bevy_vrm1 asset loader, without a window or renderer.
        // It accepts glTF as a VrmaAsset even when the VRMA extension is absent.
        app.add_plugins((WorldSerializationPlugin, MeshPlugin, GltfPlugin::default()))
            .init_asset::<AnimationGraph>()
            .add_message::<RequestRedraw>()
            .add_plugins(VrmaPlugin);
        app.finish();
        app.cleanup();
        // This real file is valid glTF, but is not a VRMA. AssetServer still
        // reports Loaded, reproducing the original failure mode.
        let handle: Handle<VrmaAsset> = app.world().resource::<AssetServer>().load("not-vrma.vrma");
        let deadline = std::time::Instant::now() + LOAD_TIMEOUT;
        loop {
            app.update();
            if app
                .world()
                .resource::<AssetServer>()
                .is_loaded_with_dependencies(handle.id())
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "VRMA fixture did not load"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let next = "animations/animasi_basic/Relax.vrma";
        {
            let mut runtime = app.world_mut().resource_mut::<LibraryPlayback>();
            runtime.requested = Some(next.into());
            runtime.loading = Some(LoadingClip {
                path: "invalid.vrma".into(),
                handle,
                entity: None,
                started: Duration::ZERO,
                source_bytes: 100,
            });
        }
        app.update();
        let runtime = app.world().resource::<LibraryPlayback>();
        assert_eq!(
            runtime.loading.as_ref().map(|clip| clip.path.as_str()),
            Some(next)
        );
        assert!(runtime.clips.is_empty());
        let errors: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<LibraryAnimationError>>()
            .drain()
            .collect();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].reason.contains("VRMC_vrm_animation"));
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<VrmaHandle>>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn gallery_selection_replays_and_returns_to_original_base() {
        let mut world = World::new();
        let first = world.spawn_empty().id();
        let second = world.spawn_empty().id();
        for base in [BasePose::Idle, BasePose::Sitting] {
            let mut state = NeraAnimationState::default();
            state.set_base_pose(base);
            state.request_one_shot(
                TemporaryAction::LibraryAnimation(first),
                Duration::from_secs(1),
            );
            assert_eq!(state.visual_state(), VisualState::LibraryAnimation(first));
            assert!(state.suppresses_look_at());
            let revision = state.revision;
            state.request_one_shot(
                TemporaryAction::LibraryAnimation(first),
                Duration::from_secs(1),
            );
            assert_ne!(state.revision, revision);
            state.request_one_shot(
                TemporaryAction::LibraryAnimation(second),
                Duration::from_secs(2),
            );
            assert_eq!(state.visual_state(), VisualState::LibraryAnimation(second));
            assert_eq!(state.active_actions.len(), 1);
            let mut clips = NeraAnimationClips::default();
            clips.library = Some(ClipSlot {
                entity: second,
                duration: Duration::from_secs(2),
            });
            assert_eq!(
                clip_for_visual(state.visual_state(), &clips)
                    .0
                    .unwrap()
                    .entity,
                second
            );
            state.advance(Duration::from_secs(2));
            assert_eq!(
                state.visual_state(),
                if base == BasePose::Idle {
                    VisualState::Idle
                } else {
                    VisualState::SittingLoop
                }
            );
        }
    }

    #[test]
    fn gallery_does_not_override_alarm() {
        let mut state = NeraAnimationState::default();
        state.start_persistent_action(TemporaryAction::AlarmWave);
        state.request_one_shot(
            TemporaryAction::LibraryAnimation(Entity::PLACEHOLDER),
            Duration::from_secs(1),
        );
        assert_eq!(state.visual_state(), VisualState::AlarmWave);
    }
}
