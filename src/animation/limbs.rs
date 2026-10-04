//! Short additive skeletal reactions, layered over either standing or sitting.
use super::{NeraAnimationState, TemporaryAction, VisualState};
use crate::interaction::{BodyPart, NeraLiveBone};
use bevy::prelude::*;
use bevy_vrm1::prelude::*;
use std::time::Duration;

pub(super) const REACTION_DURATION: Duration = Duration::from_millis(950);

#[derive(Component)]
struct LimbOffset(Quat);

pub(super) fn install(app: &mut App) {
    app.add_systems(PreUpdate, restore_limb_pose).add_systems(
        PostUpdate,
        apply_limb_pose
            .after(bevy::app::AnimationSystems)
            .after(crate::cloud::stabilize_cloud_pose)
            .before(VrmSystemSets::Constraints),
    );
}

fn restore_limb_pose(mut bones: Query<(&mut Transform, &mut LimbOffset)>) {
    for (mut transform, mut offset) in &mut bones {
        transform.rotation *= offset.0.inverse();
        offset.0 = Quat::IDENTITY;
    }
}

fn reaction_angles(part: BodyPart, bone: &str, progress: f32) -> Vec3 {
    if !matches!(
        part,
        BodyPart::LeftLeg | BodyPart::RightLeg | BodyPart::LeftFoot | BodyPart::RightFoot
    ) {
        return Vec3::ZERO;
    }
    let t = progress.clamp(0.0, 1.0);
    let envelope = (std::f32::consts::PI * t).sin().powi(2);
    let wiggle = (t * std::f32::consts::TAU * 2.0).sin() * envelope;
    let left = matches!(part, BodyPart::LeftLeg | BodyPart::LeftFoot);
    let side = if left { "left" } else { "right" };
    let sign = if left { 1.0 } else { -1.0 };
    if !bone.starts_with(side) {
        return Vec3::ZERO;
    }
    match bone.strip_prefix(side).unwrap_or_default() {
        "UpperLeg" => Vec3::new(-0.22 * envelope, 0.0, sign * 0.06 * envelope),
        "LowerLeg" => Vec3::new(0.45 * envelope, 0.0, 0.0),
        "Foot" => Vec3::new(-0.22 * wiggle, sign * 0.1 * wiggle, 0.0),
        "Toes" => Vec3::new(0.12 * wiggle, 0.0, 0.0),
        _ => Vec3::ZERO,
    }
}

fn apply_limb_pose(
    state: Res<NeraAnimationState>,
    mut bones: Query<
        (Entity, &VrmBone, &mut Transform, Option<&mut LimbOffset>),
        With<NeraLiveBone>,
    >,
    mut commands: Commands,
) {
    let VisualState::LimbReaction(part) = state.visual_state() else {
        return;
    };
    let remaining = state
        .active_actions
        .iter()
        .find(|active| active.action == TemporaryAction::LimbReaction(part))
        .and_then(|active| active.remaining)
        .unwrap_or(Duration::ZERO);
    let progress = 1.0 - remaining.as_secs_f32() / REACTION_DURATION.as_secs_f32();
    for (entity, bone, mut transform, offset) in &mut bones {
        let angles = reaction_angles(part, &bone.0, progress);
        if angles == Vec3::ZERO {
            continue;
        }
        let rotation = Quat::from_euler(EulerRot::XYZ, angles.x, angles.y, angles.z);
        transform.rotation *= rotation;
        if let Some(mut offset) = offset {
            offset.0 = rotation;
        } else {
            commands.entity(entity).insert(LimbOffset(rotation));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arms_and_hands_have_no_procedural_offsets() {
        for part in [
            BodyPart::LeftArm,
            BodyPart::RightArm,
            BodyPart::LeftHand,
            BodyPart::RightHand,
        ] {
            for bone in ["leftUpperArm", "leftLowerArm", "rightHand", "rightUpperLeg"] {
                for progress in [0.0, 0.125, 0.5, 0.875, 1.0] {
                    assert_eq!(reaction_angles(part, bone, progress), Vec3::ZERO);
                }
            }
        }
    }

    #[test]
    fn leg_and_foot_reactions_keep_their_original_numeric_shape() {
        for (part, side, sign) in [
            (BodyPart::LeftLeg, "left", 1.0),
            (BodyPart::LeftFoot, "left", 1.0),
            (BodyPart::RightLeg, "right", -1.0),
            (BodyPart::RightFoot, "right", -1.0),
        ] {
            for (bone, progress, expected) in [
                ("UpperLeg", 0.25, Vec3::new(-0.11, 0.0, sign * 0.03)),
                ("LowerLeg", 0.25, Vec3::new(0.225, 0.0, 0.0)),
                (
                    "Foot",
                    0.125,
                    Vec3::new(-0.032218255, sign * 0.014644661, 0.0),
                ),
                ("Toes", 0.125, Vec3::new(0.017573593, 0.0, 0.0)),
            ] {
                assert!(
                    reaction_angles(part, &format!("{side}{bone}"), progress).distance(expected)
                        < 0.000001
                );
            }
        }
    }

    #[test]
    fn limb_reactions_are_side_specific_and_return_to_the_base_pose() {
        for part in [BodyPart::LeftLeg, BodyPart::LeftFoot] {
            let bone = if part == BodyPart::LeftLeg {
                "leftUpperLeg"
            } else {
                "leftLowerLeg"
            };
            assert_eq!(reaction_angles(part, bone, 0.0), Vec3::ZERO);
            assert!(reaction_angles(part, bone, 0.5).length() > 0.1);
            assert!(reaction_angles(part, bone, 1.0).length() < 0.00001);
            assert_eq!(reaction_angles(part, "rightLowerLeg", 0.5), Vec3::ZERO);
        }
    }
    #[test]
    fn limb_clicks_expire_and_yield_to_dragging_and_alarms() {
        let mut state = NeraAnimationState::default();
        state.request_one_shot(
            TemporaryAction::LimbReaction(BodyPart::RightFoot),
            REACTION_DURATION,
        );
        assert_eq!(
            state.visual_state(),
            VisualState::LimbReaction(BodyPart::RightFoot)
        );
        state.start_persistent_action(TemporaryAction::Floating);
        assert_eq!(state.visual_state(), VisualState::Floating);
        state.finish_action(TemporaryAction::Floating);
        state.start_persistent_action(TemporaryAction::AlarmWave);
        assert_eq!(state.visual_state(), VisualState::AlarmWave);
        state.finish_action(TemporaryAction::AlarmWave);
        state.advance(REACTION_DURATION);
        assert_eq!(state.visual_state(), VisualState::Idle);
    }
}
