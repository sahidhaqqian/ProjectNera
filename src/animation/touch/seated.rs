//! Hold the calibrated seated lower body during standing-authored touch clips.
//! Native support still owns placement, and CloudSeatPose remains the cloud owner.
use std::time::Duration;

use bevy::prelude::*;
use bevy_vrm1::prelude::{HipsBoneEntity, Initialized, VrmBone, VrmSystemSets};

use crate::{
    animation::{BASE_BLEND, BasePose, NeraAnimationState, SittingPose, VisualState},
    avatar::NeraAvatar,
    environment::{DesktopEnvironment, PlacementState, TaskbarEdge},
    interaction::{DesktopPetInteraction, InteractionState, NeraLiveBone},
    platform::{NativeMonitorHandle, NativeWindowHandle},
};

const SETTLE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Window(NativeWindowHandle),
    Taskbar(NativeMonitorHandle, TaskbarEdge),
}

fn attached_surface(placement: PlacementState) -> Option<Surface> {
    match placement {
        PlacementState::Window(attachment) => Some(Surface::Window(attachment.target)),
        PlacementState::Taskbar(attachment) => {
            Some(Surface::Taskbar(attachment.monitor, attachment.edge))
        }
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Owner {
    avatar: Entity,
    hips: Entity,
    surface: Surface,
    pose: SittingPose,
}

#[derive(Resource)]
pub(crate) struct TouchSeatPose {
    owner: Option<Owner>,
    settling: Duration,
    returning: Duration,
    holding_touch: bool,
    bones: Vec<(Entity, Transform)>,
    /// Exact evaluated transforms replaced in the previous PostUpdate.
    raw: Vec<(Entity, Transform)>,
}

impl Default for TouchSeatPose {
    fn default() -> Self {
        Self {
            owner: None,
            settling: SETTLE,
            returning: Duration::ZERO,
            holding_touch: false,
            bones: Vec::new(),
            raw: Vec::new(),
        }
    }
}

impl TouchSeatPose {
    pub(crate) fn is_ready_for(
        &self,
        placement: PlacementState,
        sitting_pose: SittingPose,
    ) -> bool {
        self.is_ready()
            && self.owner.is_some_and(|owner| {
                Some(owner.surface) == attached_surface(placement) && owner.pose == sitting_pose
            })
    }

    pub(crate) fn is_ready(&self) -> bool {
        self.owner
            .is_some_and(|owner| self.bones.iter().any(|(entity, _)| *entity == owner.hips))
    }
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<TouchSeatPose>()
        .add_systems(
            Update,
            restore_raw_pose.before(crate::animation::sync_interaction_animation_actions),
        )
        .add_systems(
            PostUpdate,
            preserve_touch_seat
                .after(bevy::app::AnimationSystems)
                .after(crate::avatar::preserve_seated_hips)
                .before(crate::avatar::body_mode::BodyProportionSet::Apply)
                .before(VrmSystemSets::Constraints),
        );
}

pub(crate) fn clear(world: &mut World) {
    if let Some(pose) = world.remove_resource::<TouchSeatPose>() {
        for (entity, raw) in pose.raw {
            if let Some(mut transform) = world.get_mut::<Transform>(entity) {
                *transform = raw;
            }
        }
        world.insert_resource(TouchSeatPose::default());
    }
}

// Run after the existing PreUpdate Chibi restoration so the two reversible
// layers never overwrite each other's raw knee translations on detach.
fn restore_raw_pose(
    mut pose: ResMut<TouchSeatPose>,
    mut bones: Query<&mut Transform, With<NeraLiveBone>>,
) {
    for (entity, raw) in pose.raw.drain(..) {
        if let Ok(mut transform) = bones.get_mut(entity) {
            *transform = raw;
        }
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

fn finite(transform: &Transform) -> bool {
    transform.translation.is_finite()
        && transform.rotation.is_finite()
        && transform.scale.is_finite()
}

fn preserve_touch_seat(
    time: Res<Time>,
    animation: Res<NeraAnimationState>,
    environment: Res<DesktopEnvironment>,
    interaction: Option<Res<DesktopPetInteraction>>,
    presentation: Option<Res<crate::presentation::PresentationState>>,
    cloud: Option<Res<crate::cloud::CloudRuntime>>,
    mut pose: ResMut<TouchSeatPose>,
    avatars: Query<
        (Entity, &HipsBoneEntity),
        (
            With<NeraAvatar>,
            With<Initialized>,
            Without<crate::character::CharacterReveal>,
        ),
    >,
    mut bones: Query<(Entity, &VrmBone, &mut Transform), With<NeraLiveBone>>,
) {
    let owner = avatars.single().ok().and_then(|(avatar, hips)| {
        attached_surface(environment.placement).map(|surface| Owner {
            avatar,
            hips: hips.0,
            surface,
            pose: animation.selected_sitting_pose(),
        })
    });
    let blocked = owner.is_none()
        || animation.base_pose() != BasePose::Sitting
        || cloud.as_ref().is_some_and(|cloud| cloud.owns_placement())
        || presentation
            .as_ref()
            .is_some_and(|state| !state.is_desktop_pet())
        || interaction.as_ref().is_some_and(|input| {
            input.state == InteractionState::Dragging || input.drag_offset.is_some()
        });
    if blocked {
        *pose = TouchSeatPose::default();
        return;
    }
    if owner != pose.owner {
        *pose = TouchSeatPose { owner, ..default() };
    }
    let visual = animation.visual_state();
    let touch = matches!(
        visual,
        VisualState::HeadStroke
            | VisualState::TouchReaction(_)
            | VisualState::Treat
            | VisualState::FocusCelebrate
            | VisualState::FocusActivity(crate::focus::FocusPreset::Study)
    );
    let returning = visual == VisualState::SittingLoop && pose.holding_touch;
    if touch {
        pose.holding_touch = true;
        pose.returning = BASE_BLEND;
        pose.settling = SETTLE;
    } else if returning {
        pose.returning = pose
            .returning
            .saturating_sub(time.delta().min(Duration::from_millis(50)));
        if pose.returning.is_zero() {
            pose.holding_touch = false;
        }
    } else if matches!(
        visual,
        VisualState::SittingLoop | VisualState::FocusActivity(crate::focus::FocusPreset::Work)
    ) {
        pose.settling = pose
            .settling
            .saturating_sub(time.delta().min(Duration::from_millis(50)));
        if pose.settling.is_zero() {
            let snapshot: Vec<_> = bones
                .iter_mut()
                .filter(|(_, bone, transform)| seat_bone(bone.as_str()) && finite(transform))
                .map(|(entity, _, transform)| (entity, *transform))
                .collect();
            if owner.is_some_and(|owner| snapshot.iter().any(|(entity, _)| *entity == owner.hips)) {
                pose.bones = snapshot;
            }
        }
        return;
    } else {
        // A different reaction owns its own pose; never freeze it under Touch.
        pose.holding_touch = false;
        pose.returning = Duration::ZERO;
        pose.settling = SETTLE;
        return;
    }
    if touch || returning {
        let snapshot = pose.bones.clone();
        for (entity, seated) in snapshot {
            if let Ok((_, _, mut transform)) = bones.get_mut(entity)
                && finite(&transform)
            {
                pose.raw.push((entity, *transform));
                *transform = seated;
            }
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use crate::{
        animation::{TemporaryAction, TouchReactionClip},
        environment::{TaskbarAttachment, WindowAttachment},
    };
    use bevy::ecs::system::RunSystemOnce;

    fn surface() -> PlacementState {
        PlacementState::Taskbar(TaskbarAttachment {
            monitor: NativeMonitorHandle::from_raw(1),
            edge: TaskbarEdge::Bottom,
            along_offset: 200,
        })
    }

    fn fixture() -> (App, Entity, Entity, Entity) {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<NeraAnimationState>()
            .insert_resource(DesktopEnvironment {
                placement: surface(),
                ..default()
            });
        install(&mut app);
        let seated_hips =
            Transform::from_xyz(0.02, 0.55, 0.04).with_rotation(Quat::from_rotation_x(0.25));
        let hips = app
            .world_mut()
            .spawn((NeraLiveBone, VrmBone::from("hips"), seated_hips))
            .id();
        let knee = app
            .world_mut()
            .spawn((
                NeraLiveBone,
                VrmBone::from("leftLowerLeg"),
                Transform::from_xyz(0.0, 0.3, 0.0).with_rotation(Quat::from_rotation_x(1.0)),
            ))
            .id();
        let head = app
            .world_mut()
            .spawn((
                NeraLiveBone,
                VrmBone::from("head"),
                Transform::from_xyz(0.0, 0.5, 0.0),
            ))
            .id();
        app.world_mut()
            .spawn((NeraAvatar, Initialized, HipsBoneEntity(hips)));
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .enter_cloud_seat(SittingPose::Default);
        for _ in 0..5 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(Duration::from_millis(50));
            app.update();
        }
        assert!(app.world().resource::<TouchSeatPose>().is_ready());
        (app, hips, knee, head)
    }

    #[test]
    fn authored_touch_holds_only_cached_seat_and_restores_exact_raw_pose() {
        let (mut app, hips, knee, head) = fixture();
        let seated_hips = *app.world().get::<Transform>(hips).unwrap();
        let seated_knee = *app.world().get::<Transform>(knee).unwrap();
        let raw_hips =
            Transform::from_xyz(0.2, 1.0, -0.1).with_rotation(Quat::from_rotation_z(0.3));
        let raw_knee = Transform::from_xyz(0.0, 0.4, 0.02).with_scale(Vec3::splat(1.2));
        let raw_head = Transform::from_xyz(0.0, 0.7, 0.0);
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .request_one_shot(
                TemporaryAction::TouchReaction(TouchReactionClip::Wave1),
                Duration::from_secs(2),
            );
        for _ in 0..100 {
            // Stand in for a freshly evaluated full-body authored pose.
            app.world_mut().run_system_once(restore_raw_pose).unwrap();
            *app.world_mut().get_mut::<Transform>(hips).unwrap() = raw_hips;
            *app.world_mut().get_mut::<Transform>(knee).unwrap() = raw_knee;
            *app.world_mut().get_mut::<Transform>(head).unwrap() = raw_head;
            app.world_mut()
                .run_system_once(preserve_touch_seat)
                .unwrap();
            assert_eq!(*app.world().get::<Transform>(hips).unwrap(), seated_hips);
            assert_eq!(*app.world().get::<Transform>(knee).unwrap(), seated_knee);
            assert_eq!(*app.world().get::<Transform>(head).unwrap(), raw_head);
            app.world_mut().run_system_once(restore_raw_pose).unwrap();
            assert_eq!(*app.world().get::<Transform>(hips).unwrap(), raw_hips);
            assert_eq!(*app.world().get::<Transform>(knee).unwrap(), raw_knee);
        }
    }
    #[test]
    fn study_preserves_the_calibrated_seat_but_keeps_authored_reading_upper_body() {
        let (mut app, hips, knee, head) = fixture();
        let seated_hips = *app.world().get::<Transform>(hips).unwrap();
        let seated_knee = *app.world().get::<Transform>(knee).unwrap();
        assert!(
            app.world_mut()
                .resource_mut::<NeraAnimationState>()
                .request_focus_activity(crate::focus::FocusPreset::Study)
        );
        for i in 0..30 {
            app.world_mut().run_system_once(restore_raw_pose).unwrap();
            let reading_head = Transform::from_rotation(Quat::from_rotation_x(i as f32 * 0.01));
            *app.world_mut().get_mut::<Transform>(head).unwrap() = reading_head;
            app.world_mut()
                .get_mut::<Transform>(hips)
                .unwrap()
                .translation
                .y = 1.2;
            app.world_mut().get_mut::<Transform>(knee).unwrap().rotation = Quat::IDENTITY;
            app.world_mut()
                .run_system_once(preserve_touch_seat)
                .unwrap();
            assert_eq!(*app.world().get::<Transform>(hips).unwrap(), seated_hips);
            assert_eq!(*app.world().get::<Transform>(knee).unwrap(), seated_knee);
            assert_eq!(*app.world().get::<Transform>(head).unwrap(), reading_head);
        }
        app.world_mut().run_system_once(restore_raw_pose).unwrap();
        assert_eq!(
            app.world().get::<Transform>(hips).unwrap().translation.y,
            1.2
        );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .cancel_focus_activity();
        app.world_mut()
            .resource_mut::<DesktopEnvironment>()
            .placement = PlacementState::Free;
        app.update();
        assert!(!app.world().resource::<TouchSeatPose>().is_ready());
    }

    #[test]
    fn work_typing_keeps_authored_seated_body_instead_of_the_previous_sit_variant() {
        let (mut app, hips, knee, _) = fixture();
        assert!(
            app.world_mut()
                .resource_mut::<NeraAnimationState>()
                .request_focus_activity(crate::focus::FocusPreset::Work)
        );
        let typed_hips =
            Transform::from_xyz(0.0, 0.65, 0.1).with_rotation(Quat::from_rotation_x(0.15));
        let typed_knee = Transform::from_rotation(Quat::from_rotation_x(1.2));
        *app.world_mut().get_mut::<Transform>(hips).unwrap() = typed_hips;
        *app.world_mut().get_mut::<Transform>(knee).unwrap() = typed_knee;
        app.world_mut()
            .run_system_once(preserve_touch_seat)
            .unwrap();
        assert_eq!(*app.world().get::<Transform>(hips).unwrap(), typed_hips);
        assert_eq!(*app.world().get::<Transform>(knee).unwrap(), typed_knee);
        assert!(app.world().resource::<TouchSeatPose>().raw.is_empty());
    }

    #[test]
    fn headpat_preserves_seated_legs_and_restores_the_authored_pose() {
        let (mut app, hips, knee, head) = fixture();
        let seated = *app.world().get::<Transform>(hips).unwrap();
        let bent = *app.world().get::<Transform>(knee).unwrap();
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .request_one_shot(TemporaryAction::HeadStroke, Duration::from_secs(2));
        let raw = Transform::from_xyz(0.0, 1.1, 0.0);
        *app.world_mut().get_mut::<Transform>(hips).unwrap() = raw;
        *app.world_mut().get_mut::<Transform>(knee).unwrap() = Transform::IDENTITY;
        *app.world_mut().get_mut::<Transform>(head).unwrap() = raw;
        app.world_mut()
            .run_system_once(preserve_touch_seat)
            .unwrap();
        assert_eq!(*app.world().get::<Transform>(hips).unwrap(), seated);
        assert_eq!(*app.world().get::<Transform>(knee).unwrap(), bent);
        assert_eq!(*app.world().get::<Transform>(head).unwrap(), raw);
        app.world_mut().run_system_once(restore_raw_pose).unwrap();
        assert_eq!(*app.world().get::<Transform>(hips).unwrap(), raw);
        assert_eq!(
            *app.world().get::<Transform>(knee).unwrap(),
            Transform::IDENTITY
        );
    }

    #[test]
    fn return_blend_holds_seat_then_releases_and_detach_clears_cache() {
        let (mut app, hips, _, _) = fixture();
        let seated = *app.world().get::<Transform>(hips).unwrap();
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .request_one_shot(
                TemporaryAction::TouchReaction(TouchReactionClip::Wave1),
                Duration::from_secs(2),
            );
        app.update();
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .finish_action(TemporaryAction::TouchReaction(TouchReactionClip::Wave1));
        for _ in 0..2 {
            app.world_mut().run_system_once(restore_raw_pose).unwrap();
            app.world_mut()
                .get_mut::<Transform>(hips)
                .unwrap()
                .translation
                .y = 1.2;
            app.world_mut()
                .run_system_once(preserve_touch_seat)
                .unwrap();
            assert_eq!(*app.world().get::<Transform>(hips).unwrap(), seated);
        }
        app.world_mut().run_system_once(restore_raw_pose).unwrap();
        app.world_mut()
            .get_mut::<Transform>(hips)
            .unwrap()
            .translation
            .y = 0.6;
        app.world_mut()
            .run_system_once(preserve_touch_seat)
            .unwrap();
        assert_eq!(
            app.world().get::<Transform>(hips).unwrap().translation.y,
            0.6
        );
        app.world_mut()
            .resource_mut::<DesktopEnvironment>()
            .placement = PlacementState::Free;
        app.update();
        assert!(!app.world().resource::<TouchSeatPose>().is_ready());
    }

    #[test]
    fn admission_rejects_changed_support_or_pose_before_postupdate_invalidates_old_cache() {
        let (app, _, _, _) = fixture();
        let pose = app.world().resource::<TouchSeatPose>();
        assert!(pose.is_ready_for(surface(), SittingPose::Default));
        assert!(!pose.is_ready_for(surface(), SittingPose::Sitting2));
        assert!(!pose.is_ready_for(
            PlacementState::Window(WindowAttachment {
                target: NativeWindowHandle::from_raw(2),
                support_offset_x: 100,
            }),
            SittingPose::Default,
        ));
        assert!(!pose.is_ready_for(PlacementState::Free, SittingPose::Default));
        assert!(
            pose.is_ready(),
            "the old cache still exists during this admission check"
        );
    }

    #[test]
    fn new_support_pose_and_character_clear_cached_owner_but_native_position_does_not() {
        let (mut app, _, _, _) = fixture();
        app.world_mut()
            .resource_mut::<DesktopEnvironment>()
            .placement = PlacementState::Taskbar(TaskbarAttachment {
            along_offset: 450,
            ..match surface() {
                PlacementState::Taskbar(attachment) => attachment,
                _ => unreachable!(),
            }
        });
        app.update();
        assert!(app.world().resource::<TouchSeatPose>().is_ready());
        app.world_mut()
            .resource_mut::<DesktopEnvironment>()
            .placement = PlacementState::Window(WindowAttachment {
            target: NativeWindowHandle::from_raw(2),
            support_offset_x: 100,
        });
        app.update();
        assert!(!app.world().resource::<TouchSeatPose>().is_ready());
        for _ in 0..5 {
            app.update();
        }
        assert!(app.world().resource::<TouchSeatPose>().is_ready());
        clear(app.world_mut());
        assert!(!app.world().resource::<TouchSeatPose>().is_ready());
    }
}
