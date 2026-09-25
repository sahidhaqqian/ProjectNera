//! Load a gallery selection on the desktop rig, then use the normal controller.
use std::{collections::HashMap, time::Duration};

use bevy::prelude::*;
use bevy_vrm1::prelude::*;

use super::{NeraAnimationClips, NeraAnimationState, TemporaryAction, playback::ClipSlot};
use crate::avatar::NeraAvatar;

#[derive(Message)]
pub(crate) struct PlayLibraryAnimation {
    pub path: String,
}

#[derive(Resource, Default)]
pub(super) struct LibraryPlayback {
    // Keep loaded children in place: removing them would renumber the VRM graph
    // nodes used by existing production clips and their retarget tables.
    clips: HashMap<String, Entity>,
    requested: Option<String>,
    loading: Option<(String, Entity, Handle<VrmaAsset>)>,
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

pub(super) fn sync_library_animation(
    mut requests: MessageReader<PlayLibraryAnimation>,
    avatar: Query<Entity, (With<NeraAvatar>, With<Initialized>)>,
    assets: Res<AssetServer>,
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
    if let Some((path, entity, handle)) = runtime.loading.as_ref() {
        if assets
            .get_load_state(handle.id())
            .is_some_and(|s| s.is_failed())
        {
            warn!("Could not load gallery animation: {path}");
            commands.entity(*entity).despawn();
            let path = path.clone();
            runtime.clips.remove(&path);
            if runtime.requested.as_ref() == Some(&path) {
                runtime.requested = None;
            }
            runtime.loading = None;
        } else if ready.get(*entity).is_ok_and(|(_, clip)| clip.warmup == 0) {
            runtime.loading = None;
        }
    }
    let Some(path) = runtime.requested.clone() else {
        return;
    };
    if let Some(&entity) = runtime.clips.get(&path) {
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
    let handle = assets.load(path.clone());
    let entity = commands
        .spawn((
            Name::new(format!("Nera Library: {path}")),
            VrmaHandle(handle.clone()),
            ChildOf(root),
        ))
        .observe(loaded)
        .id();
    runtime.clips.insert(path.clone(), entity);
    runtime.loading = Some((path, entity, handle));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::{BasePose, VisualState, playback::clip_for_visual};

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
