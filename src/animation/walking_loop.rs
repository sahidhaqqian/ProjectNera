//! Loop only the steady stride in the supplied Walking.vrma. The full take
//! includes a slow walk-in, so repeating it looks like stopping every 2.9 s.
//! Sampling a subrange keeps the original file and the VRM retarget graph intact.
use std::collections::HashMap;

use bevy::prelude::*;
use bevy_vrm1::prelude::ChildSearcher;

use super::{NeraAnimationClips, NeraAnimationState, VisualState};
use crate::{avatar::NeraAvatar, interaction::NeraLiveBone};

// Frames 36..69 at 24 Hz: a complete steady left/right stride with the closest
// matching endpoint poses in this take. The walk-in at 0..1 s is never repeated.
const START: f32 = 36.0 / 24.0;
const END: f32 = 69.0 / 24.0;
const PERIOD: f32 = END - START;
const SEAM_BLEND: f32 = 0.12;

#[derive(Resource, Default)]
pub(crate) struct WalkingCycle {
    active: bool,
    phase: f32,
    wrapped: bool,
    seam_elapsed: f32,
    previous: HashMap<Entity, Transform>,
    seam_from: HashMap<Entity, Transform>,
}

impl WalkingCycle {
    fn reset(&mut self) {
        self.active = false;
        self.phase = 0.0;
        self.wrapped = false;
        self.previous.clear();
        self.seam_from.clear();
    }

    fn advance(&mut self, dt: f32) -> f32 {
        self.wrapped = false;
        if !self.active {
            self.active = true;
            self.phase = 0.0;
        } else {
            let next = self.phase + dt.clamp(0.0, 0.05);
            self.wrapped = next >= PERIOD;
            self.phase = next.rem_euclid(PERIOD);
        }
        START + self.phase
    }
}

pub(super) fn seek_walking_cycle(
    time: Res<Time>,
    state: Res<NeraAnimationState>,
    clips: Res<NeraAnimationClips>,
    mut cycle: ResMut<WalkingCycle>,
    avatar: Query<Entity, With<NeraAvatar>>,
    searcher: ChildSearcher,
    mut players: Query<(&mut AnimationPlayer, &AnimationTransitions)>,
) {
    if state.visual_state() != VisualState::Walking
        || !clips
            .walking
            .is_some_and(|clip| clip.duration.as_secs_f32() >= END)
    {
        cycle.reset();
        return;
    }
    let Some(root) = avatar
        .iter()
        .next()
        .and_then(|entity| searcher.find_root_bone(entity))
    else {
        cycle.reset();
        return;
    };
    let Ok((mut player, transitions)) = players.get_mut(root) else {
        cycle.reset();
        return;
    };
    let Some(active) = transitions
        .get_main_animation()
        .and_then(|index| player.animation_mut(index))
    else {
        cycle.reset();
        return;
    };
    // Sample before animate_targets in this same frame. Seeking does not
    // restart playback, replay PlayVrma, or fade back through the standing pose.
    active.set_seek_time(cycle.advance(time.delta_secs()));
}

pub(crate) fn smooth_walking_cycle(
    time: Res<Time>,
    mut cycle: ResMut<WalkingCycle>,
    mut bones: Query<(Entity, &mut Transform), With<NeraLiveBone>>,
) {
    if !cycle.active {
        return;
    }
    if cycle.wrapped {
        let WalkingCycle {
            previous,
            seam_from,
            ..
        } = &mut *cycle;
        seam_from.clone_from(previous);
        cycle.seam_elapsed = 0.0;
    }
    cycle.seam_elapsed += time.delta_secs().clamp(0.0, 0.05);
    let blend = (cycle.seam_elapsed / SEAM_BLEND).clamp(0.0, 1.0);
    for (entity, mut pose) in &mut bones {
        if let Some(from) = cycle.seam_from.get(&entity) {
            *pose = blend_pose(from, &pose, blend);
        }
        cycle.previous.insert(entity, *pose);
    }
    if blend >= 1.0 {
        cycle.seam_from.clear();
    }
}

fn blend_pose(from: &Transform, to: &Transform, t: f32) -> Transform {
    Transform {
        translation: from.translation.lerp(to.translation, t),
        rotation: from.rotation.slerp(to.rotation, t),
        scale: from.scale.lerp(to.scale, t),
    }
}

#[cfg(test)]
mod tests {
    use super::super::playback::ClipSlot;
    use super::*;
    use bevy::animation::graph::AnimationNodeIndex;
    use bevy_vrm1::prelude::Vrm;
    use std::time::Duration;

    #[test]
    fn repeats_only_the_steady_stride_for_the_entire_walk() {
        let mut cycle = WalkingCycle::default();
        assert_eq!(cycle.advance(1.0 / 60.0), START);
        let mut loops = 0;
        for _ in 0..600 {
            let sample = cycle.advance(1.0 / 60.0);
            assert!((START..END).contains(&sample));
            loops += usize::from(cycle.wrapped);
        }
        assert_eq!(loops, 7);
        cycle.reset();
        assert_eq!(cycle.advance(1.0 / 60.0), START);
        assert!(!cycle.wrapped);
    }

    #[test]
    fn seam_blends_pose_without_an_idle_frame() {
        let end = Transform::from_rotation(Quat::from_rotation_x(0.3));
        let start = Transform::from_rotation(Quat::from_rotation_x(0.5));
        assert_eq!(blend_pose(&end, &start, 0.0), end);
        let middle = blend_pose(&end, &start, 0.5);
        assert!(middle.rotation.angle_between(Quat::from_rotation_x(0.4)) < 0.001);
        assert_eq!(blend_pose(&end, &start, 1.0), start);
    }

    #[test]
    fn seek_changes_only_walk_and_does_not_restart_or_modify_other_avatars() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<NeraAnimationState>()
            .init_resource::<NeraAnimationClips>()
            .init_resource::<WalkingCycle>()
            .add_systems(Update, (seek_walking_cycle, smooth_walking_cycle).chain());
        let node = AnimationNodeIndex::new(1);
        let mut player = AnimationPlayer::default();
        let mut transitions = AnimationTransitions::default();
        transitions
            .play(&mut player, node, Duration::ZERO)
            .set_seek_time(0.2);
        let root = app
            .world_mut()
            .spawn((
                Name::new(Vrm::ROOT_BONE),
                player.clone(),
                transitions.clone(),
            ))
            .id();
        let other = app.world_mut().spawn((player, transitions)).id();
        app.world_mut().spawn(NeraAvatar).add_child(root);
        app.world_mut().resource_mut::<NeraAnimationClips>().walking = Some(ClipSlot {
            entity: root,
            duration: Duration::from_secs_f32(2.916667),
        });
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(1.0 / 60.0));
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .set_walking(true);
        app.update();
        assert_eq!(
            app.world()
                .get::<AnimationPlayer>(root)
                .unwrap()
                .animation(node)
                .unwrap()
                .seek_time(),
            START
        );
        app.update();
        assert!(
            app.world()
                .get::<AnimationPlayer>(root)
                .unwrap()
                .animation(node)
                .unwrap()
                .seek_time()
                > START
        );
        assert_eq!(
            app.world()
                .get::<AnimationPlayer>(other)
                .unwrap()
                .animation(node)
                .unwrap()
                .seek_time(),
            0.2
        );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .set_walking(false);
        let stopped = app
            .world()
            .get::<AnimationPlayer>(root)
            .unwrap()
            .animation(node)
            .unwrap()
            .seek_time();
        app.update();
        assert!(!app.world().resource::<WalkingCycle>().active);
        assert_eq!(
            app.world()
                .get::<AnimationPlayer>(root)
                .unwrap()
                .animation(node)
                .unwrap()
                .seek_time(),
            stopped
        );
    }
}
