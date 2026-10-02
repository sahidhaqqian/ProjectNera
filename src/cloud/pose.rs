//! Preserve an evaluated seated lower body when standing-authored upper-body reactions play.
use super::CloudRuntime;
use crate::{
    animation::{NeraAnimationState, VisualState},
    interaction::NeraLiveBone,
};
use bevy::prelude::*;
use bevy_vrm1::prelude::VrmBone;

#[derive(Resource, Default)]
pub(super) struct SleepDirection(Option<(Entity, Quat)>);

pub(super) fn restore_sleep_direction(
    mut direction: ResMut<SleepDirection>,
    mut bones: Query<&mut Transform, With<NeraLiveBone>>,
) {
    if let Some((entity, raw)) = direction.0.take() {
        if let Ok(mut hips) = bones.get_mut(entity) {
            hips.rotation = raw;
        }
    }
}

fn right_sleep_rotation(raw: Quat) -> Quat {
    // Roll in the screen plane: head moves right without turning the face away.
    Quat::from_rotation_z(std::f32::consts::PI) * raw
}

pub(super) fn orient_sleeping_hips(
    cloud: Res<CloudRuntime>,
    animation: Res<NeraAnimationState>,
    clips: Res<crate::animation::NeraAnimationClips>,
    playback: Res<crate::animation::NeraAnimationPlayback>,
    avatar: Query<&bevy_vrm1::prelude::HipsBoneEntity, With<crate::avatar::NeraAvatar>>,
    mut bones: Query<&mut Transform, With<NeraLiveBone>>,
    mut direction: ResMut<SleepDirection>,
) {
    if !cloud.is_lying_rest()
        || animation.visual_state() != VisualState::SittingLoop
        || !clips.sleeping_idle_playing(&playback)
    {
        return;
    }
    let Ok(entity) = avatar.single() else {
        return;
    };
    let Ok(mut hips) = bones.get_mut(entity.0) else {
        return;
    };
    direction.0 = Some((entity.0, hips.rotation));
    hips.rotation = right_sleep_rotation(hips.rotation);
}

const SETTLE_SECONDS: f32 = 0.22;
#[derive(Resource, Default)]
pub(crate) struct CloudSeatPose {
    owner: Option<Entity>,
    settling: f32,
    bones: Vec<(Entity, Transform)>,
    base_clip: Option<Entity>,
}
impl CloudSeatPose {
    pub(crate) fn is_ready_for(&self, cloud: &CloudRuntime) -> bool {
        self.owner.is_some()
            && self.owner == cloud.attachment().map(|a| a.visual)
            && !self.bones.is_empty()
    }
}
fn seat_bone(name: &str) -> bool {
    matches!(
        name,
        "hips"
            | "leftUpperLeg"
            | "rightUpperLeg"
            | "leftLowerLeg"
            | "rightLowerLeg"
            | "leftFoot"
            | "rightFoot"
            | "leftToes"
            | "rightToes"
    )
}
pub(crate) fn stabilize_cloud_pose(
    time: Res<Time>,
    cloud: Option<Res<CloudRuntime>>,
    animation: Res<NeraAnimationState>,
    clips: Option<Res<crate::animation::NeraAnimationClips>>,
    mut pose: ResMut<CloudSeatPose>,
    mut bones: Query<(Entity, &VrmBone, &mut Transform), With<NeraLiveBone>>,
) {
    let owner = cloud
        .as_ref()
        .and_then(|cloud| cloud.attachment())
        .map(|a| a.visual);
    if owner != pose.owner {
        pose.owner = owner;
        pose.settling = SETTLE_SECONDS;
        pose.bones.clear();
    }
    if owner.is_none() {
        return;
    }
    let seated = animation.visual_state() == VisualState::SittingLoop;
    let base_clip = cloud
        .as_ref()
        .filter(|cloud| cloud.is_lying_rest())
        .and_then(|_| {
            clips
                .as_ref()
                .and_then(|clips| clips.sleeping_idle_entity())
        });
    if seated && base_clip != pose.base_clip {
        pose.base_clip = base_clip;
        pose.settling = SETTLE_SECONDS;
        pose.bones.clear();
    }
    if seated {
        pose.settling = (pose.settling - time.delta_secs().min(0.05)).max(0.0);
        if pose.settling <= 0.0 {
            pose.bones.clear();
            for (entity, bone, transform) in &mut bones {
                if seat_bone(bone.as_str()) {
                    pose.bones.push((entity, *transform));
                }
            }
            return;
        }
    } else {
        pose.settling = SETTLE_SECONDS;
    }
    for &(entity, base) in &pose.bones {
        if let Ok((_, _, mut transform)) = bones.get_mut(entity) {
            *transform = base;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        animation::{SittingPose, head_stroke_test_clips, sync_interaction_animation_actions},
        avatar::NeraAvatar,
        cloud::{CloudMode, visual::CloudVisual},
        environment::DesktopEnvironment,
        interaction::{
            NeraClicked, NeraDragStarted, NeraHeadPatted, NeraPointerPosition, NeraReleased,
        },
    };
    use std::time::Duration;
    #[test]
    fn both_rest_poses_ignore_all_body_clicks_and_headpat_but_drag_and_ride_still_work() {
        use crate::{cloud::RestPose, interaction::BodyPart};
        use bevy::ecs::system::RunSystemOnce;
        for expected in [RestPose::Lying, RestPose::Seated] {
            let mut app = App::new();
            app.init_resource::<NeraAnimationState>()
                .init_resource::<DesktopEnvironment>()
                .init_resource::<crate::desktop_physics::DesktopPhysicsState>()
                .add_message::<NeraClicked>()
                .add_message::<NeraDragStarted>()
                .add_message::<NeraHeadPatted>()
                .add_message::<NeraReleased>()
                .add_message::<crate::pet_feedback::PetSfx>()
                .add_message::<crate::pet_feedback::PetFx>()
                .add_systems(Update, sync_interaction_animation_actions);
            let avatar = app.world_mut().spawn(NeraAvatar).id();
            let visual = app.world_mut().spawn(CloudVisual::default()).id();
            let hips = app.world_mut().spawn_empty().id();
            let mut cloud = CloudRuntime::default();
            if expected == RestPose::Seated {
                cloud.summon(CloudMode::Rest, avatar, visual, Vec2::ZERO);
                cloud.dismiss();
            }
            cloud.summon(CloudMode::Rest, avatar, visual, Vec2::ZERO);
            app.insert_resource(cloud);
            app.insert_resource(CloudSeatPose {
                owner: Some(visual),
                bones: vec![(hips, Transform::default())],
                ..default()
            });
            let clips = head_stroke_test_clips(app.world_mut());
            app.insert_resource(clips);
            app.world_mut()
                .resource_mut::<NeraAnimationState>()
                .enter_cloud_seat(SittingPose::Sitting4);
            let pointer = NeraPointerPosition {
                screen: IVec2::ZERO,
                local: Vec2::ZERO,
            };
            for body_part in [
                BodyPart::Head,
                BodyPart::Torso,
                BodyPart::LeftArm,
                BodyPart::RightArm,
                BodyPart::LeftHand,
                BodyPart::RightHand,
                BodyPart::LeftLeg,
                BodyPart::RightLeg,
                BodyPart::LeftFoot,
                BodyPart::RightFoot,
            ] {
                app.world_mut()
                    .write_message(NeraClicked { pointer, body_part });
                app.world_mut().write_message(NeraHeadPatted { pointer });
                app.update();
                assert_eq!(
                    app.world().resource::<NeraAnimationState>().visual_state(),
                    VisualState::SittingLoop
                );
            }
            assert_eq!(
                app.world().resource::<CloudRuntime>().rest_pose(),
                Some(expected)
            );
            app.world_mut()
                .run_system_once(
                    |mut cloud: ResMut<CloudRuntime>,
                     mut environment: ResMut<DesktopEnvironment>,
                     mut physics: ResMut<crate::desktop_physics::DesktopPhysicsState>,
                     mut animation: ResMut<NeraAnimationState>,
                     mut sfx: MessageWriter<crate::pet_feedback::PetSfx>,
                     mut fx: MessageWriter<crate::pet_feedback::PetFx>| {
                        super::super::begin_cloud_drag(
                            &mut cloud,
                            &mut environment,
                            &mut physics,
                            &mut animation,
                            &mut sfx,
                            &mut fx,
                        );
                    },
                )
                .unwrap();
            assert!(!app.world().resource::<CloudRuntime>().owns_placement());
            app.world_mut().write_message(NeraDragStarted {
                screen: IVec2::ZERO,
                body_part: BodyPart::LeftHand,
            });
            app.update();
            assert_eq!(
                app.world().resource::<NeraAnimationState>().visual_state(),
                VisualState::Floating
            );
            app.world_mut().write_message(NeraReleased {
                pointer,
                body_part: BodyPart::LeftHand,
                was_dragging: true,
            });
            app.update();
            assert_eq!(
                app.world().resource::<NeraAnimationState>().visual_state(),
                VisualState::Idle
            );
            app.world_mut().write_message(NeraClicked {
                pointer,
                body_part: BodyPart::LeftHand,
            });
            app.update();
            assert_eq!(
                app.world().resource::<NeraAnimationState>().visual_state(),
                VisualState::LimbReaction(BodyPart::LeftHand)
            );
            app.world_mut().resource_mut::<CloudRuntime>().summon(
                CloudMode::Ride,
                avatar,
                visual,
                Vec2::ZERO,
            );
            app.world_mut()
                .resource_mut::<NeraAnimationState>()
                .enter_cloud_seat(SittingPose::Default);
            app.world_mut().write_message(NeraHeadPatted { pointer });
            app.update();
            assert_eq!(
                app.world().resource::<NeraAnimationState>().visual_state(),
                VisualState::HeadStroke
            );
        }
    }
    #[test]
    fn sleeping_roll_points_head_right_preserves_face_front_and_restores_without_accumulation() {
        let raw = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let right = right_sleep_rotation(raw);
        assert!((right * Vec3::Y).x > 0.99);
        assert!((right * Vec3::Z).distance(raw * Vec3::Z) < 0.00001);
        let mut app = App::new();
        app.init_resource::<SleepDirection>()
            .add_systems(Update, restore_sleep_direction);
        let hips = app
            .world_mut()
            .spawn((NeraLiveBone, Transform::from_rotation(raw)))
            .id();
        for _ in 0..1000 {
            app.world_mut().get_mut::<Transform>(hips).unwrap().rotation = right;
            app.world_mut().resource_mut::<SleepDirection>().0 = Some((hips, raw));
            app.update();
            assert_eq!(app.world().get::<Transform>(hips).unwrap().rotation, raw);
            assert!(app.world().resource::<SleepDirection>().0.is_none());
        }
        app.update();
        assert_eq!(app.world().get::<Transform>(hips).unwrap().rotation, raw);
    }
    #[test]
    fn upper_body_reaction_and_return_blend_keep_exact_calibrated_seat_bones() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<CloudSeatPose>()
            .init_resource::<NeraAnimationState>()
            .init_resource::<DesktopEnvironment>()
            .add_message::<NeraClicked>()
            .add_message::<NeraDragStarted>()
            .add_message::<NeraHeadPatted>()
            .add_message::<NeraReleased>()
            .add_systems(
                Update,
                (sync_interaction_animation_actions, stabilize_cloud_pose).chain(),
            );
        let avatar = app.world_mut().spawn(NeraAvatar).id();
        let visual = app.world_mut().spawn(CloudVisual::default()).id();
        let mut cloud = CloudRuntime::default();
        cloud.summon(CloudMode::Ride, avatar, visual, Vec2::ZERO);
        app.insert_resource(cloud);
        let clips = head_stroke_test_clips(app.world_mut());
        app.insert_resource(clips);
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .enter_cloud_seat(SittingPose::Default);
        let seated = Transform::from_xyz(0.1, 0.55, 0.2).with_rotation(Quat::from_rotation_x(0.7));
        let hips = app
            .world_mut()
            .spawn((NeraLiveBone, VrmBone("hips".into()), seated))
            .id();
        let leg = app
            .world_mut()
            .spawn((NeraLiveBone, VrmBone("leftUpperLeg".into()), seated))
            .id();
        let head = app
            .world_mut()
            .spawn((NeraLiveBone, VrmBone("head".into()), Transform::default()))
            .id();
        let headpat = NeraHeadPatted {
            pointer: NeraPointerPosition {
                screen: IVec2::ZERO,
                local: Vec2::ZERO,
            },
        };
        app.world_mut().write_message(headpat);
        app.update();
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::SittingLoop
        );
        for _ in 0..6 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(Duration::from_millis(50));
            app.update();
        }
        app.world_mut().write_message(headpat);
        for bone in [hips, leg, head] {
            app.world_mut()
                .get_mut::<Transform>(bone)
                .unwrap()
                .translation
                .y += 2.0;
        }
        app.update();
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::HeadStroke
        );
        assert_eq!(*app.world().get::<Transform>(hips).unwrap(), seated);
        assert_eq!(*app.world().get::<Transform>(leg).unwrap(), seated);
        assert_eq!(
            app.world().get::<Transform>(head).unwrap().translation.y,
            2.0
        );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .enter_cloud_seat(SittingPose::Default);
        app.world_mut()
            .get_mut::<Transform>(hips)
            .unwrap()
            .translation
            .y = 8.0;
        app.update();
        assert_eq!(*app.world().get::<Transform>(hips).unwrap(), seated);
        app.world_mut().resource_mut::<CloudRuntime>().dismiss();
        app.world_mut()
            .get_mut::<Transform>(hips)
            .unwrap()
            .translation
            .y = 9.0;
        app.update();
        assert_eq!(
            app.world().get::<Transform>(hips).unwrap().translation.y,
            9.0
        );
        assert!(app.world().resource::<CloudSeatPose>().bones.is_empty());
    }
}
