//! Small rest-pose-aware arm layer for the sideways cloud sleep loop.
use super::CloudRuntime;
use crate::{
    animation::{NeraAnimationClips, NeraAnimationPlayback, NeraAnimationState, VisualState},
    interaction::NeraLiveBone,
};
use bevy::prelude::*;
use bevy_vrm1::prelude::{RestGlobalTransform, RestTransform, VrmBone};

const BLEND_SECONDS: f32 = 0.3;
const ARM_BONES: usize = 8;

#[derive(Resource, Default)]
pub(super) struct SleepingArms {
    owner: Option<Entity>,
    blend: f32,
    raw: Vec<(Entity, Quat)>,
}

impl SleepingArms {
    #[cfg(debug_assertions)]
    pub(super) fn applied_count(&self) -> usize {
        self.raw.len()
    }
}

fn active_owner(cloud: &CloudRuntime, visual: VisualState, sleeping: bool) -> Option<Entity> {
    cloud
        .attachment()
        .filter(|_| cloud.is_lying_rest() && visual == VisualState::SittingLoop && sleeping)
        .map(|attachment| attachment.visual)
}

pub(super) fn restore(
    mut arms: ResMut<SleepingArms>,
    mut bones: Query<&mut Transform, With<NeraLiveBone>>,
) {
    for (entity, raw) in arms.raw.drain(..) {
        if let Ok(mut transform) = bones.get_mut(entity) {
            transform.rotation = raw;
        }
    }
}

fn arm_rotation(name: &str) -> Option<Quat> {
    let (left, joint) = name
        .strip_prefix("left")
        .map(|joint| (true, joint))
        .or_else(|| name.strip_prefix("right").map(|joint| (false, joint)))?;
    let side = if left { 1.0 } else { -1.0 };
    let rest_axis = Vec3::X * side;
    // Both elbows point down the torso, away from the face. One forearm lies
    // alongside the waist; the other bends gently onto the front of the torso.
    let upper_direction = Vec3::new(side * 0.18, -0.96, 0.22).normalize();
    let upper = Quat::from_rotation_arc(rest_axis, upper_direction);
    let forearm_direction = if left {
        Vec3::new(0.0, -0.97, 0.22)
    } else {
        Vec3::new(0.86, -0.42, 0.28)
    }
    .normalize();
    match joint {
        "Shoulder" => Some(Quat::IDENTITY),
        "UpperArm" => Some(upper),
        "LowerArm" => Some(Quat::from_rotation_arc(
            rest_axis,
            upper.inverse() * forearm_direction,
        )),
        "Hand" => Some(Quat::from_rotation_x(-0.08)),
        _ => None,
    }
}

fn retarget_rotation(normalized: Quat, rest: &RestTransform, global: &RestGlobalTransform) -> Quat {
    // Same normalized humanoid-space conversion used by the VRMA retargeter.
    rest.0.rotation * global.0.rotation().inverse() * normalized * global.0.rotation()
}

pub(super) fn apply(
    time: Res<Time>,
    cloud: Res<CloudRuntime>,
    animation: Res<NeraAnimationState>,
    clips: Res<NeraAnimationClips>,
    playback: Res<NeraAnimationPlayback>,
    mut arms: ResMut<SleepingArms>,
    mut bones: Query<
        (
            Entity,
            &VrmBone,
            &RestTransform,
            &RestGlobalTransform,
            &mut Transform,
        ),
        With<NeraLiveBone>,
    >,
) {
    let owner = active_owner(
        &cloud,
        animation.visual_state(),
        clips.sleeping_idle_playing(&playback),
    );
    if owner != arms.owner {
        arms.owner = owner;
        arms.blend = 0.0;
    }
    if owner.is_none() {
        return;
    }
    arms.blend = (arms.blend + time.delta_secs().min(0.05) / BLEND_SECONDS).min(1.0);
    let weight = arms.blend * arms.blend * (3.0 - 2.0 * arms.blend);
    for (entity, bone, rest, global, mut transform) in &mut bones {
        let Some(normalized) = arm_rotation(bone.as_str()) else {
            continue;
        };
        if arms.raw.len() == ARM_BONES {
            break;
        }
        arms.raw.push((entity, transform.rotation));
        transform.rotation = transform
            .rotation
            .slerp(retarget_rotation(normalized, rest, global), weight);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud::CloudMode;

    #[test]
    fn arm_layer_only_owns_loaded_rest_sleep_and_yields_to_reactions_detach_and_replacement() {
        let mut world = World::new();
        let avatar = world.spawn_empty().id();
        let visual = world.spawn_empty().id();
        let mut cloud = CloudRuntime::default();
        assert_eq!(active_owner(&cloud, VisualState::SittingLoop, true), None);
        cloud.summon(CloudMode::Rest, avatar, visual, Vec2::ZERO);
        assert_eq!(
            active_owner(&cloud, VisualState::SittingLoop, true),
            Some(visual)
        );
        assert_eq!(active_owner(&cloud, VisualState::SittingLoop, false), None);
        for reaction in [
            VisualState::HeadStroke,
            VisualState::Floating,
            VisualState::AlarmWave,
        ] {
            assert_eq!(active_owner(&cloud, reaction, true), None);
        }
        cloud.begin_drag();
        assert_eq!(active_owner(&cloud, VisualState::SittingLoop, true), None);
        cloud.summon(CloudMode::Ride, avatar, visual, Vec2::ZERO);
        assert_eq!(active_owner(&cloud, VisualState::SittingLoop, true), None);
        let replacement = world.spawn_empty().id();
        cloud.summon(CloudMode::Rest, avatar, replacement, Vec2::ZERO);
        assert_eq!(active_owner(&cloud, VisualState::SittingLoop, true), None);
        cloud.dismiss();
        cloud.summon(CloudMode::Rest, avatar, replacement, Vec2::ZERO);
        assert_eq!(
            active_owner(&cloud, VisualState::SittingLoop, true),
            Some(replacement)
        );
        cloud.dismiss();
        assert_eq!(active_owner(&cloud, VisualState::SittingLoop, true), None);
    }

    #[test]
    fn sleeping_elbows_and_hands_stay_below_shoulders_with_relaxed_wrists() {
        for (side, axis) in [("left", Vec3::X), ("right", Vec3::NEG_X)] {
            let upper = arm_rotation(&format!("{side}UpperArm")).unwrap();
            let lower = arm_rotation(&format!("{side}LowerArm")).unwrap();
            let elbow = upper * axis * 0.3;
            let hand = elbow + upper * lower * axis * 0.25;
            assert!(elbow.y < -0.25);
            assert!(hand.y < -0.2, "{side} hand reaches up towards the face");
            assert!(hand.z > 0.0, "hand must lie in front of the torso");
            assert!(
                arm_rotation(&format!("{side}Hand"))
                    .unwrap()
                    .angle_between(Quat::IDENTITY)
                    < 0.1
            );
        }
        assert!(arm_rotation("head").is_none());
        assert!(arm_rotation("leftIndexProximal").is_none());
    }

    #[test]
    fn arm_retargeting_handles_rotated_rest_axes_and_restore_never_accumulates() {
        let rest = RestTransform(Transform::from_rotation(Quat::from_rotation_z(0.7)));
        let global = RestGlobalTransform(GlobalTransform::from(Transform::from_rotation(
            Quat::from_rotation_y(1.2),
        )));
        let target = arm_rotation("rightUpperArm").unwrap();
        let local = retarget_rotation(target, &rest, &global);
        let recovered =
            global.0.rotation() * rest.0.rotation.inverse() * local * global.0.rotation().inverse();
        assert!(recovered.angle_between(target) < 0.001);
        assert!(
            retarget_rotation(Quat::IDENTITY, &rest, &global).angle_between(rest.0.rotation)
                < 0.001
        );

        let raw = Quat::from_rotation_y(0.4);
        let mut app = App::new();
        app.init_resource::<SleepingArms>()
            .add_systems(Update, restore);
        let entity = app
            .world_mut()
            .spawn((NeraLiveBone, Transform::from_rotation(raw)))
            .id();
        for _ in 0..1000 {
            app.world_mut()
                .get_mut::<Transform>(entity)
                .unwrap()
                .rotation = local;
            app.world_mut()
                .resource_mut::<SleepingArms>()
                .raw
                .push((entity, raw));
            app.update();
            assert_eq!(app.world().get::<Transform>(entity).unwrap().rotation, raw);
            assert!(app.world().resource::<SleepingArms>().raw.is_empty());
        }
        app.world_mut().despawn(entity);
        app.world_mut()
            .resource_mut::<SleepingArms>()
            .raw
            .push((entity, raw));
        app.update();
        assert!(app.world().resource::<SleepingArms>().raw.is_empty());
    }
}
