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
    for scale in [0.6, 1.0, 1.5] {
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
