//! Load a gallery selection on the desktop rig, then use the normal controller.
use std::{collections::HashMap, time::Duration};

use bevy::prelude::*;
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
}

#[derive(Resource, Default)]
pub(crate) struct LibraryPlayback {
    clips: HashMap<String, CachedClip>,
    source_bytes: u64,
    access_clock: u64,
    requested: Option<String>,
    loading: Option<LoadingClip>,
}

impl LibraryPlayback {
    pub(crate) fn path_for(&self, entity: Entity) -> Option<&str> {
        self.clips
            .iter()
            .find(|(_, clip)| clip.entity == entity)
            .map(|(path, _)| path.as_str())
    }
    fn validate_source_size(source_bytes: u64) -> Result<(), &'static str> {
        if source_bytes > MAX_CLIP_SOURCE_BYTES {
            return Err("Animation exceeds the 16 MiB desktop playback limit.");
        }
        Ok(())
    }

    fn cache_clip(&mut self, path: String, entity: Entity, source_bytes: u64) {
        self.access_clock = self.access_clock.wrapping_add(1);
        self.source_bytes = self.source_bytes.saturating_add(source_bytes);
        self.clips.insert(
            path,
            CachedClip {
                entity,
                source_bytes,
                last_used: self.access_clock,
            },
        );
    }

    fn cached_entity(&mut self, path: &str) -> Option<Entity> {
        self.access_clock = self.access_clock.wrapping_add(1);
        let clip = self.clips.get_mut(path)?;
        clip.last_used = self.access_clock;
        Some(clip.entity)
    }

    fn evict_for(&mut self, source_bytes: u64, protected: Option<Entity>) -> Vec<Entity> {
        let mut evicted = Vec::new();
        while self.clips.len() >= MAX_LIBRARY_CLIPS
            || self.source_bytes.saturating_add(source_bytes) > MAX_LIBRARY_SOURCE_BYTES
        {
            let candidate = self
                .clips
                .iter()
                .filter(|(_, clip)| Some(clip.entity) != protected)
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
        if self.requested.as_ref() == Some(&path) {
            self.requested = None;
        }
        errors.write(LibraryAnimationError { path, reason });
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

fn validate_asset(asset: &VrmaAsset, clips: &Assets<AnimationClip>) -> Result<(), &'static str> {
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
) {
    // Last click wins, including when the previous selection is still loading.
    if let Some(request) = requests.read().last() {
        runtime.requested = Some(request.path.clone());
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
    if let Some(entity) = runtime.cached_entity(&path) {
        let Ok((_, loaded)) = ready.get(entity) else {
            return;
        };
        if loaded.warmup != 0 {
            return;
        }
        runtime.requested = None;
        if loaded.duration.is_zero() {
            return;
        }
        clips.library = Some(ClipSlot {
            entity,
            duration: loaded.duration,
        });
        state.request_one_shot(TemporaryAction::LibraryAnimation(entity), loaded.duration);
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
    let protected = match state.visual_state() {
        super::VisualState::LibraryAnimation(entity) => Some(entity),
        _ => None,
    };
    for entity in runtime.evict_for(source_bytes, protected) {
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
            let evicted = runtime.evict_for(1024, Some(retained[1].1));
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
        let evicted = runtime.evict_for(MAX_CLIP_SOURCE_BYTES, None);
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
