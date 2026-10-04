use super::*;
use crate::{
    animation::{BasePose, NeraAnimationClips},
    settings::NeraSettings,
};
use std::time::Duration;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        bevy::app::TaskPoolPlugin::default(),
        bevy::asset::AssetPlugin::default(),
    ))
    .init_asset::<Mesh>()
    .init_asset::<StandardMaterial>()
    .init_resource::<Time<Real>>()
    .init_resource::<PresentationState>()
    .init_resource::<DesktopEnvironment>()
    .init_resource::<DesktopPhysicsState>()
    .init_resource::<NeraAnimationState>()
    .init_resource::<DesktopPetInteraction>()
    .init_resource::<CloudRuntime>()
    .insert_resource(visual::CloudVisualAssets::procedural())
    .init_resource::<NeraSettings>()
    .add_message::<CloudCommand>()
    .add_message::<PetFx>()
    .add_message::<PetSfx>()
    .add_systems(Update, update_cloud);
    app.world_mut().spawn((NeraAvatar, Initialized));
    app.world_mut().spawn((Window::default(), PrimaryWindow));
    app.world_mut()
        .resource_mut::<DesktopEnvironment>()
        .nera_rect = Some(crate::platform::PhysicalRect {
        left: -800,
        top: 100,
        right: -200,
        bottom: 1050,
    });
    app
}

#[test]
fn real_command_pipeline_is_idempotent_and_reclaims_visual_hierarchy() {
    let mut app = app();
    for mode in [CloudMode::Rest, CloudMode::Ride] {
        for _ in 0..200 {
            app.world_mut().write_message(CloudCommand::Summon(mode));
        }
        app.update();
        assert_eq!(app.world().resource::<CloudRuntime>().mode(), Some(mode));
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<visual::CloudVisual>>()
                .iter(app.world())
                .count(),
            1
        );
        assert_eq!(
            app.world().resource::<NeraAnimationState>().base_pose(),
            BasePose::Sitting
        );
    }
    for _ in 0..200 {
        app.world_mut().write_message(CloudCommand::Dismiss);
    }
    app.update();
    assert!(!app.world().resource::<CloudRuntime>().owns_placement());
    assert_eq!(
        app.world().resource::<NeraAnimationState>().base_pose(),
        BasePose::Idle
    );
    app.world_mut()
        .resource_mut::<Time<Real>>()
        .advance_by(Duration::from_secs(1));
    app.update();
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<visual::CloudVisual>>()
            .iter(app.world())
            .count(),
        0
    );
    assert_eq!(
        app.world_mut()
            .query_filtered::<Entity, With<Mesh3d>>()
            .iter(app.world())
            .count(),
        0,
        "all five cloud lobes reclaimed"
    );
}

#[test]
fn unexpected_visual_destruction_and_hidden_pet_release_ownership() {
    let mut app = app();
    app.world_mut()
        .write_message(CloudCommand::Summon(CloudMode::Ride));
    app.update();
    let visual = app.world().resource::<CloudRuntime>().visual().unwrap();
    app.world_mut().despawn(visual);
    app.update();
    assert!(!app.world().resource::<CloudRuntime>().owns_placement());
    assert_eq!(
        app.world().resource::<NeraAnimationState>().base_pose(),
        BasePose::Idle
    );
    app.world_mut()
        .write_message(CloudCommand::Summon(CloudMode::Rest));
    app.update();
    for mut window in app
        .world_mut()
        .query::<&mut Window>()
        .iter_mut(app.world_mut())
    {
        window.visible = false;
    }
    app.update();
    assert!(!app.world().resource::<CloudRuntime>().owns_placement());
    assert!(app.world().resource::<NeraSettings>().gravity_enabled);
}

#[test]
fn dismissal_preserves_disabled_gravity_preference() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<NeraSettings>()
        .gravity_enabled = false;
    app.world_mut()
        .write_message(CloudCommand::Summon(CloudMode::Ride));
    app.update();
    app.world_mut().write_message(CloudCommand::Dismiss);
    app.update();
    assert!(!app.world().resource::<NeraSettings>().gravity_enabled);
    assert!(!app.world().resource::<CloudRuntime>().owns_placement());
}

fn attached(mode: CloudMode, world: &mut World) -> CloudRuntime {
    let avatar = world.spawn(NeraAvatar).id();
    let visual = world.spawn(visual::CloudVisual::default()).id();
    let mut cloud = CloudRuntime::default();
    cloud.summon(mode, avatar, visual, Vec2::new(-500.0, 450.0));
    cloud
}
#[test]
fn rest_selects_two_stable_nonrepeating_poses_without_affecting_ride() {
    let mut app = app();
    for expected in [RestPose::Lying, RestPose::Seated, RestPose::Lying] {
        app.world_mut()
            .write_message(CloudCommand::Summon(CloudMode::Rest));
        app.update();
        assert_eq!(
            app.world().resource::<CloudRuntime>().rest_pose(),
            Some(expected)
        );
        for _ in 0..20 {
            app.world_mut()
                .write_message(CloudCommand::Summon(CloudMode::Rest));
            app.world_mut()
                .resource_mut::<Time<Real>>()
                .advance_by(Duration::from_secs(5));
            app.update();
            assert_eq!(
                app.world().resource::<CloudRuntime>().rest_pose(),
                Some(expected)
            );
        }
        app.world_mut()
            .write_message(CloudCommand::Summon(CloudMode::Ride));
        app.update();
        assert_eq!(app.world().resource::<CloudRuntime>().rest_pose(), None);
        assert!(app.world().resource::<CloudRuntime>().is_riding());
        app.world_mut().write_message(CloudCommand::Dismiss);
        app.update();
        assert_eq!(app.world().resource::<CloudRuntime>().rest_pose(), None);
    }
}
#[test]
fn lying_rest_contact_uses_world_down_and_scale_without_changing_ride_contact() {
    let mut world = World::new();
    let rest = attached(CloudMode::Rest, &mut world);
    let ride = attached(CloudMode::Ride, &mut world);
    // Body mode and user size compose on the same hips contact; neither cloud
    // attachment nor the user's selected Rest pose changes when it is resized.
    for scale in [0.62, 1.0, 1.5 * 0.62, 1.5] {
        let hips = GlobalTransform::from(
            Transform::from_xyz(0.3, 0.5, -0.2)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2))
                .with_scale(Vec3::splat(scale)),
        );
        let expected = hips.translation() - Vec3::Y * 0.1 * scale;
        assert!(rest.contact_world(&hips, true).distance(expected) < 0.00001);
        assert_eq!(
            rest.contact_world(&hips, false),
            crate::avatar::seat_contact_world(&hips)
        );
        assert_eq!(
            ride.contact_world(&hips, true),
            crate::avatar::seat_contact_world(&hips)
        );
    }
}
#[test]
fn repeated_body_proportion_changes_preserve_both_cloud_attachment_owners_and_seat() {
    let mut world = World::new();
    for mode in [CloudMode::Rest, CloudMode::Ride] {
        let cloud = attached(mode, &mut world);
        let attachment = cloud.attachment().unwrap();
        let rest_pose = cloud.rest_pose();
        for scale in [0.62, 1.0].into_iter().cycle().take(100) {
            let hips = GlobalTransform::from(
                Transform::from_xyz(0.3, 0.5, -0.2).with_scale(Vec3::splat(scale)),
            );
            let contact = cloud.contact_world(&hips, mode == CloudMode::Rest);
            assert!(contact.is_finite());
            let current = cloud.attachment().unwrap();
            assert_eq!(current.avatar, attachment.avatar);
            assert_eq!(current.visual, attachment.visual);
            assert_eq!(current.seat, attachment.seat);
            assert_eq!(cloud.mode(), Some(mode));
            assert_eq!(cloud.rest_pose(), rest_pose);
        }
    }
}

#[test]
fn chibi_cloud_pose_keeps_its_rendered_face_inside_canvas_and_preserves_placement_owner() {
    use crate::avatar::body_mode::{AvatarBodyMode, AvatarBodyModePlugin, BodyProportions};
    use bevy::camera::{CameraProjection, RenderTargetInfo};
    use bevy_vrm1::prelude::{HipsBoneEntity, RestTransform, VrmBone};

    let mut projection = PerspectiveProjection::default();
    projection.aspect_ratio = 600.0 / 950.0;
    let mut camera = Camera::default();
    camera.computed.clip_from_view = projection.get_clip_from_view();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: UVec2::new(600, 950),
        scale_factor: 1.0,
    });
    let view = GlobalTransform::from(Transform::from_xyz(0.0, 0.8, 4.5));

    for mode in [CloudMode::Rest, CloudMode::Ride] {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(NeraSettings {
                body_mode: "chibi".into(),
                ..default()
            })
            .add_message::<PetFx>()
            .add_message::<PetSfx>();
        let cloud = attached(mode, app.world_mut());
        let attachment = cloud.attachment().unwrap();
        let raw_root = Transform::from_xyz(0.0, -0.85, 0.0);
        app.world_mut()
            .entity_mut(attachment.avatar)
            .insert((Initialized, raw_root));
        // A valid imported rest rig can have a low authored lying pose.
        // Its head stays inside this viewport unless a standing-foot drop is
        // incorrectly added on top of the cloud's existing seat ownership.
        let hips_rest = Transform::from_xyz(0.0, 0.8, 0.0);
        let hips_pose =
            Transform::from_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
        let hips = app
            .world_mut()
            .spawn((
                VrmBone::from("hips"),
                hips_pose,
                RestTransform(hips_rest),
                ChildOf(attachment.avatar),
            ))
            .id();
        let head_pose = Transform::from_xyz(0.0, 0.45, 0.0);
        let head = app
            .world_mut()
            .spawn((
                VrmBone::from("head"),
                head_pose,
                RestTransform(head_pose),
                ChildOf(hips),
            ))
            .id();
        let foot_pose = Transform::from_xyz(0.0, -0.8, 0.0);
        app.world_mut().spawn((
            VrmBone::from("leftFoot"),
            foot_pose,
            RestTransform(foot_pose),
            ChildOf(hips),
        ));
        app.world_mut()
            .entity_mut(attachment.avatar)
            .insert(HipsBoneEntity(hips));
        app.insert_resource(cloud).add_plugins(AvatarBodyModePlugin);
        app.update();
        assert_eq!(
            *app.world().resource::<AvatarBodyMode>(),
            AvatarBodyMode::Chibi
        );
        assert_eq!(
            app.world().resource::<BodyProportions>().root_offset_y(),
            0.0
        );
        assert_eq!(
            *app.world().get::<Transform>(attachment.avatar).unwrap(),
            raw_root
        );
        let revision = app.world().resource::<BodyProportions>().revision();
        let head_world = GlobalTransform::from(raw_root)
            .mul_transform(*app.world().get::<Transform>(hips).unwrap())
            .mul_transform(*app.world().get::<Transform>(head).unwrap())
            .translation();
        let face = camera.world_to_viewport(&view, head_world).unwrap();
        assert!(face.x >= 0.0 && face.x <= 600.0 && face.y >= 0.0 && face.y <= 950.0);
        let standing_offset = app
            .world()
            .resource::<BodyProportions>()
            .standing_root_offset_y();
        assert!(standing_offset < 0.0);
        let dropped_face = camera
            .world_to_viewport(&view, head_world + Vec3::Y * standing_offset)
            .unwrap();
        assert!(
            dropped_face.y > 950.0,
            "fixture must expose the clipped lying pose"
        );

        for _ in 0..5 {
            app.update();
            assert_eq!(
                *app.world().get::<Transform>(attachment.avatar).unwrap(),
                raw_root
            );
            assert_eq!(
                app.world().resource::<BodyProportions>().root_offset_y(),
                0.0
            );
        }

        // The initial Normal -> Chibi transition legitimately emits feedback.
        // MinimalPlugins keeps these messages buffered; measure only subsequent
        // cloud ownership changes below rather than relying on message aging.
        app.world_mut().resource_mut::<Messages<PetFx>>().clear();
        app.world_mut().resource_mut::<Messages<PetSfx>>().clear();

        // Changing only cloud placement ownership changes the derived root
        // correction, while preserving selected proportions and their revision.
        for _ in 0..10 {
            app.world_mut().resource_mut::<CloudRuntime>().dismiss();
            app.update();
            let root = *app.world().get::<Transform>(attachment.avatar).unwrap();
            assert!(
                (root.translation.y - raw_root.translation.y - standing_offset).abs() < 0.00001
            );
            assert_eq!(
                app.world().resource::<BodyProportions>().root_offset_y(),
                standing_offset
            );
            assert_eq!(
                app.world().resource::<BodyProportions>().revision(),
                revision
            );
            app.world_mut().resource_mut::<CloudRuntime>().summon(
                mode,
                attachment.avatar,
                attachment.visual,
                attachment.seat,
            );
            app.update();
            assert_eq!(
                *app.world().get::<Transform>(attachment.avatar).unwrap(),
                raw_root
            );
            assert_eq!(
                app.world().resource::<BodyProportions>().root_offset_y(),
                0.0
            );
            assert_eq!(
                app.world().resource::<BodyProportions>().revision(),
                revision
            );
            let current = app.world().resource::<CloudRuntime>().attachment().unwrap();
            assert_eq!(current.avatar, attachment.avatar);
            assert_eq!(current.visual, attachment.visual);
            assert_eq!(current.seat, attachment.seat);
            assert_eq!(current.mode, mode);
            assert_eq!(
                app.world().get::<Transform>(hips).unwrap().scale,
                Vec3::splat(0.62)
            );
            assert_eq!(
                app.world().get::<Transform>(head).unwrap().scale,
                Vec3::splat(1.9)
            );
            assert!(app.world().resource::<Messages<PetFx>>().is_empty());
            assert!(app.world().resource::<Messages<PetSfx>>().is_empty());
        }
        // Keep the test's saved setting in sync to avoid invoking disk settings
        // persistence while exercising the authoritative mode resource.
        *app.world_mut().resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Normal;
        app.world_mut().resource_mut::<NeraSettings>().body_mode = "normal".into();
        for _ in 0..5 {
            app.update();
            assert_eq!(
                *app.world().get::<Transform>(attachment.avatar).unwrap(),
                raw_root
            );
            assert_eq!(*app.world().get::<Transform>(hips).unwrap(), hips_pose);
            assert_eq!(*app.world().get::<Transform>(head).unwrap(), head_pose);
            assert_eq!(
                app.world().resource::<BodyProportions>().root_offset_y(),
                0.0
            );
            assert_eq!(app.world().resource::<BodyProportions>().body_scale(), 1.0);
            assert!(app.world().resource::<CloudRuntime>().owns_placement());
        }
    }
}

#[test]
fn both_modes_enter_then_settle_and_repeated_summon_is_bounded() {
    for mode in [CloudMode::Rest, CloudMode::Ride] {
        let mut world = World::new();
        let mut cloud = attached(mode, &mut world);
        assert!(cloud.owns_placement());
        assert_eq!(cloud.mode(), Some(mode));
        let a = cloud.attachment().unwrap();
        for _ in 0..1000 {
            cloud.summon(mode, a.avatar, a.visual, a.seat);
        }
        cloud.advance(APPEAR_SECONDS + 0.1);
        assert!(matches!(cloud.state, CloudState::Attached(_)));
        assert_eq!(cloud.attachment().unwrap().seat, a.seat);
        assert_eq!(
            world
                .query_filtered::<Entity, With<visual::CloudVisual>>()
                .iter(&world)
                .count(),
            1
        );
    }
}
#[test]
fn rest_drag_releases_immediately_while_ride_drag_remains_attached() {
    let mut world = World::new();
    let mut rest = attached(CloudMode::Rest, &mut world);
    assert!(rest.begin_drag().is_some());
    assert!(!rest.owns_placement());
    assert!(matches!(rest.state, CloudState::Disappearing { .. }));
    assert!(rest.begin_drag().is_none());
    let mut ride = attached(CloudMode::Ride, &mut world);
    let seat = ride.attachment().unwrap().seat;
    assert!(ride.begin_drag().is_none());
    assert!(ride.is_riding());
    assert_eq!(ride.attachment().unwrap().seat, seat);
}
#[test]
fn repeated_dismiss_and_visual_destruction_never_leave_attachment() {
    let mut world = World::new();
    let mut cloud = attached(CloudMode::Ride, &mut world);
    let visual = cloud.visual().unwrap();
    assert!(cloud.dismiss().is_some());
    for _ in 0..1000 {
        assert!(cloud.dismiss().is_none());
    }
    assert!(!cloud.owns_placement());
    assert_eq!(cloud.advance(DISAPPEAR_SECONDS + 0.1), Some(visual));
    assert!(matches!(cloud.state, CloudState::Hidden));
    assert_eq!(cloud.advance(10.0), None);
}
#[test]
fn cloud_suspends_real_physics_system_and_dismiss_restores_normal_owner() {
    let mut app = App::new();
    let cloud = attached(CloudMode::Ride, app.world_mut());
    app.insert_resource(cloud)
        .init_resource::<Time>()
        .init_resource::<DesktopEnvironment>()
        .init_resource::<NeraSettings>()
        .init_resource::<DesktopPetInteraction>()
        .init_resource::<NeraAnimationClips>()
        .init_resource::<NeraAnimationState>()
        .init_resource::<DesktopPhysicsState>()
        .add_message::<crate::interaction::NeraThrow>()
        .add_systems(Update, crate::desktop_physics::update_desktop_physics);
    app.world_mut()
        .resource_mut::<DesktopPhysicsState>()
        .falling = true;
    app.world_mut()
        .resource_mut::<DesktopPhysicsState>()
        .vertical_velocity = 500.0;
    app.world_mut()
        .resource_mut::<NeraAnimationState>()
        .set_physics_falling(true);
    app.world_mut()
        .write_message(crate::interaction::NeraThrow {
            velocity: Vec2::splat(900.0),
        });
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_millis(16));
    app.update();
    let physics = app.world().resource::<DesktopPhysicsState>();
    assert!(!physics.falling && !physics.throwing && physics.support.is_none());
    assert_eq!(physics.vertical_velocity, 0.0);
    assert!(app.world().resource::<NeraSettings>().gravity_enabled);
    app.world_mut().resource_mut::<CloudRuntime>().dismiss();
    assert!(!app.world().resource::<CloudRuntime>().owns_placement());
    app.update();
    assert!(app.world().resource::<NeraSettings>().gravity_enabled);
}
#[test]
fn character_cleanup_destroys_cloud_and_clears_seated_and_physics_state() {
    let mut world = World::new();
    let cloud = attached(CloudMode::Rest, &mut world);
    let visual = cloud.visual().unwrap();
    world.insert_resource(cloud);
    world.init_resource::<DesktopEnvironment>();
    world.init_resource::<DesktopPhysicsState>();
    let mut animation = NeraAnimationState::default();
    animation.enter_cloud_seat(SittingPose::Sitting4);
    world.insert_resource(animation);
    clear_for_character_change(&mut world);
    clear_for_character_change(&mut world);
    assert!(world.get_entity(visual).is_err());
    assert!(!world.resource::<CloudRuntime>().owns_placement());
    assert_eq!(
        world.resource::<DesktopEnvironment>().attached_anchor,
        AvatarAnchor::Feet
    );
    assert_eq!(
        world.resource::<NeraAnimationState>().base_pose(),
        BasePose::Idle
    );
}

#[test]
fn rest_releases_gaze_on_dismiss_drag_interruption_replacement_and_ride() {
    use crate::avatar::{LookAtTarget, sync_look_at_for_animation};
    use bevy_vrm1::{prelude::LookAt, vrm::body_tracking::BodyTracking};
    for exit in 0..5 {
        for follow_head in [false, true] {
            let mut app = app();
            app.world_mut().resource_mut::<NeraSettings>().follow_eyes = true;
            app.world_mut().resource_mut::<NeraSettings>().follow_head = follow_head;
            app.add_systems(Update, sync_look_at_for_animation.after(update_cloud));
            app.world_mut().spawn(LookAtTarget);
            let avatar = app
                .world_mut()
                .query_filtered::<Entity, With<NeraAvatar>>()
                .single(app.world())
                .unwrap();
            app.world_mut()
                .write_message(CloudCommand::Summon(CloudMode::Rest));
            app.update();
            assert!(app.world().get::<LookAt>(avatar).is_none());
            assert!(app.world().get::<BodyTracking>(avatar).is_none());
            match exit {
                0 => {
                    app.world_mut().write_message(CloudCommand::Dismiss);
                }
                1 => {
                    app.world_mut().resource_mut::<CloudRuntime>().begin_drag();
                }
                2 => {
                    app.world_mut()
                        .query_filtered::<&mut Window, With<PrimaryWindow>>()
                        .single_mut(app.world_mut())
                        .unwrap()
                        .visible = false;
                }
                3 => {
                    clear_for_character_change(app.world_mut());
                }
                _ => {
                    app.world_mut()
                        .write_message(CloudCommand::Summon(CloudMode::Ride));
                }
            }
            app.update();
            assert!(app.world().get::<LookAt>(avatar).is_some(), "exit {exit}");
            assert_eq!(
                app.world().get::<BodyTracking>(avatar).is_some(),
                follow_head
            );
            assert!(app.world().resource::<NeraSettings>().follow_eyes);
            assert_eq!(
                app.world().resource::<NeraSettings>().follow_head,
                follow_head
            );
        }
    }
}
