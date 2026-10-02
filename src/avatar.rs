//! Avatar scene, gaze ownership, and pose-derived support anchors.
//! Scheduling remains in main so cross-subsystem ordering stays explicit.
mod anchors;

#[cfg(all(debug_assertions, target_os = "windows"))]
pub(crate) use anchors::assert_smoke_ground_contact;
pub(crate) use anchors::seat_contact_world;
pub(crate) use anchors::{
    AvatarVisualGrounding, preserve_grounded_pose, preserve_seated_hips, restore_visual_grounding,
    update_avatar_anchors,
};

use crate::{
    animation::{NeraAnimationState, spawn_animation_library},
    character::ActiveCharacter,
    environment::DesktopEnvironment,
    interaction::cursor_position_in_nera_window,
    settings::NeraSettings,
};
use bevy::{prelude::*, window::PrimaryWindow};
use bevy_vrm1::{prelude::*, vrm::body_tracking::BodyTracking};

#[derive(Component)]
pub(crate) struct NeraAvatar;

#[derive(Component)]
pub(crate) struct NeraMainCamera;

#[derive(Component)]
pub(crate) struct NeraMainLight;

#[derive(Component)]
pub(crate) struct LookAtTarget;

/// Project a stable desktop point onto a camera-facing world plane.
pub(crate) fn desktop_point_to_world(
    point: Vec2,
    environment: &DesktopEnvironment,
    camera: &Camera,
    view: &GlobalTransform,
    z: f32,
) -> Option<Vec3> {
    let rect = environment.nera_rect?;
    let local = (point - Vec2::new(rect.left as f32, rect.top as f32))
        / environment.nera_scale_factor.max(0.25) as f32;
    let ray = camera.viewport_to_world(view, local).ok()?;
    if ray.direction.z.abs() < 0.0001 {
        return None;
    }
    let distance = (z - ray.origin.z) / ray.direction.z;
    (distance.is_finite() && distance >= 0.0).then(|| ray.get_point(distance))
}

pub(crate) fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Name::new("Nera Camera"),
        NeraMainCamera,
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        Msaa::Off,
        Transform::from_xyz(0.0, 0.8, 4.5).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
    ));
}

pub(crate) fn setup_lighting(mut commands: Commands) {
    commands.spawn((
        Name::new("Nera Key Light"),
        NeraMainLight,
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: false,
            contact_shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.7, -0.5, 0.0)),
    ));
}

// -----------------------------------------------------------------------------
// Nera Spawning
// -----------------------------------------------------------------------------

pub(crate) fn spawn_nera(mut commands: Commands, asset_server: Res<AssetServer>) {
    info!("Loading Nera VRM 1.0...");

    let target = commands
        .spawn((
            Name::new("LookAt Target"),
            LookAtTarget,
            Transform::from_xyz(0.0, 1.0, 2.0),
        ))
        .id();

    commands
        .spawn((
            Name::new("Nera"),
            NeraAvatar,
            // Reveal only once the startup entrance has a loaded rig and pose.
            Visibility::Hidden,
            VrmHandle(asset_server.load(crate::character::DEFAULT_CHARACTER)),
            LookAt::Target(target),
            Transform::from_xyz(0.0, -0.85, 0.0),
        ))
        .with_children(|parent| {
            spawn_animation_library(parent, &asset_server);
        });
}

#[cfg(target_os = "windows")]
pub(crate) fn update_look_at_target(
    time: Res<Time>,
    settings: Res<NeraSettings>,
    character: Res<ActiveCharacter>,
    cloud: Res<crate::cloud::CloudRuntime>,
    environment: Res<DesktopEnvironment>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    mut target_query: Query<&mut Transform, With<LookAtTarget>>,
) {
    let Some(window) = window_query.iter().next() else {
        return;
    };

    let Some(mut target_transform) = target_query.iter_mut().next() else {
        return;
    };

    // Keep a stable neutral point in front of Nera. Turning Follow Mouse off
    // eases the target back here instead of snapping the gaze in one frame.
    let neutral = Vec3::new(0.0, 0.5, 2.0);

    let desired = if settings.follow_mouse && cloud.mode() != Some(crate::cloud::CloudMode::Rest) {
        cursor_position_in_nera_window(&environment)
            .map(|cursor_pos| {
                let intensity = f32::from(settings.follow_intensity.clamp(25, 100)) / 100.0;

                let map_x = horizontal_gaze_x(
                    cursor_pos.x,
                    window.resolution.width(),
                    intensity,
                    character.migrated_from_vrm0,
                );

                let map_y =
                    0.5 + (0.5 - cursor_pos.y / window.resolution.height()) * 4.0 * intensity;

                Vec3::new(map_x, map_y, 2.0)
            })
            .unwrap_or(neutral)
    } else {
        neutral
    };

    // Frame-rate independent exponential smoothing. A value around 12 keeps
    // gaze responsive while removing the robotic cursor-to-bone snapping.
    let blend = 1.0 - (-12.0 * time.delta_secs()).exp();
    target_transform.translation = target_transform.translation.lerp(desired, blend);
}

fn horizontal_gaze_x(cursor_x: f32, window_width: f32, intensity: f32, legacy: bool) -> f32 {
    let x = (cursor_x / window_width.max(1.0) - 0.5) * 4.0 * intensity;
    if legacy { -x } else { x }
}

/// VRMA reactions such as HitHead and ShakeHeadNo1 need temporary ownership of
/// the head bones. `bevy_vrm1` LookAt runs after animation, so the animation
/// suppression rule remains authoritative over the user Follow Mouse settings.
pub(crate) fn sync_look_at_for_animation(
    animation: Res<NeraAnimationState>,
    settings: Res<NeraSettings>,
    cloud: Option<Res<crate::cloud::CloudRuntime>>,
    target_query: Query<Entity, With<LookAtTarget>>,
    avatar_query: Query<(Entity, Option<&LookAt>, Option<&BodyTracking>), With<NeraAvatar>>,
    mut commands: Commands,
) {
    let Some(target) = target_query.iter().next() else {
        return;
    };

    let Some((avatar, current_look_at, current_body_tracking)) = avatar_query.iter().next() else {
        return;
    };

    // The master Follow Mouse switch changes the target between cursor and
    // neutral. Keep LookAt attached (when Eyes is enabled) so switching the
    // master off can visibly ease back to neutral instead of freezing mid-gaze.
    // Rest owns the entire head pose, including its loading/fallback seated loop.
    // Derive suppression from the attachment owner; user preferences remain intact.
    let resting = cloud.is_some_and(|cloud| cloud.mode() == Some(crate::cloud::CloudMode::Rest));
    let allow_look_at = settings.follow_eyes && !animation.suppresses_look_at() && !resting;
    let allow_body_tracking = allow_look_at && settings.follow_head;

    if allow_look_at {
        if current_look_at.is_none() {
            commands.entity(avatar).insert(LookAt::Target(target));
        }
    } else if current_look_at.is_some() {
        commands.entity(avatar).remove::<LookAt>();
    }

    // bevy_vrm1's BodyTracking extends LookAt from the eyes into the upper
    // body. Keep it opt-in and remove it whenever LookAt is unavailable so it
    // cannot fight reaction / transition VRMAs.
    if allow_body_tracking {
        if current_body_tracking.is_none() {
            commands.entity(avatar).insert(BodyTracking::default());
        }
    } else if current_body_tracking.is_some() {
        commands.entity(avatar).remove::<BodyTracking>();
    }
}

/// Walking.vrma advances almost two metres along its local +Z axis. Desktop
/// movement owns that travel, so preserve the vertical gait while constraining
/// the live hips to their rest X/Z. This must run after AnimationSystems and
/// before VrmSystemSets::Constraints, which precedes VRM transform propagation.
pub(crate) fn apply_walking_pose(
    animation: Res<NeraAnimationState>,
    walking: Res<crate::walking::WalkingRuntime>,
    mut original_rotation: Local<Option<(Entity, Quat)>>,
    mut avatar_query: Query<(Entity, &mut Transform, &HipsBoneEntity), With<NeraAvatar>>,
    mut bones: Query<(&mut Transform, &RestTransform), Without<NeraAvatar>>,
) {
    let Ok((entity, mut avatar, hips)) = avatar_query.single_mut() else {
        return;
    };
    let direction = if matches!(
        animation.visual_state(),
        crate::animation::VisualState::Idle | crate::animation::VisualState::Walking
    ) {
        walking.direction()
    } else {
        0.0
    };

    if direction == 0.0 {
        if let Some((owner, rotation)) = original_rotation.take() {
            if owner == entity {
                avatar.rotation = rotation;
            }
        }
        return;
    }

    let (_, rotation) = *original_rotation.get_or_insert((entity, avatar.rotation));
    // VRM's forward is +Z. Positive yaw turns it toward camera-right (+X).
    avatar.rotation = rotation * Quat::from_rotation_y(direction * std::f32::consts::FRAC_PI_2);
    if animation.visual_state() == crate::animation::VisualState::Walking
        && let Ok((mut transform, rest)) = bones.get_mut(hips.0)
    {
        constrain_walking_hips(&mut transform, &rest.0);
    }
}

fn constrain_walking_hips(transform: &mut Transform, rest: &Transform) {
    transform.translation.x = rest.translation.x;
    transform.translation.z = rest.translation.z;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walking_root_motion_is_removed_without_erasing_gait_or_bone_rotation() {
        let rest = Transform::from_xyz(0.02, 0.9, -0.03);
        let original = Transform {
            translation: Vec3::new(-0.04, 0.94, 1.88),
            rotation: Quat::from_rotation_y(0.15),
            scale: Vec3::splat(0.95),
        };
        let mut hips = original;
        constrain_walking_hips(&mut hips, &rest);
        assert_eq!(hips.translation, Vec3::new(0.02, 0.94, -0.03));
        assert_eq!(hips.rotation, original.rotation);
        assert_eq!(hips.scale, original.scale);
    }

    #[test]
    fn legacy_vrm0_gaze_corrects_the_horizontal_coordinate_direction() {
        assert!(horizontal_gaze_x(75.0, 100.0, 1.0, false) > 0.0);
        assert!(horizontal_gaze_x(75.0, 100.0, 1.0, true) < 0.0);
        assert!(horizontal_gaze_x(25.0, 100.0, 1.0, true) > 0.0);
    }

    #[test]
    fn animation_temporarily_owns_gaze_then_restores_user_tracking() {
        let mut app = App::new();
        app.init_resource::<NeraAnimationState>()
            .insert_resource(NeraSettings {
                follow_mouse: false,
                follow_eyes: true,
                follow_head: true,
                ..default()
            })
            .add_systems(Update, sync_look_at_for_animation);
        app.world_mut().spawn(LookAtTarget);
        let avatar = app.world_mut().spawn(NeraAvatar).id();

        // Disabling cursor following still permits neutral eye/head tracking.
        app.update();
        assert!(app.world().get::<LookAt>(avatar).is_some());
        assert!(app.world().get::<BodyTracking>(avatar).is_some());

        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .set_praying_menu(true);
        app.update();
        assert!(app.world().get::<LookAt>(avatar).is_none());
        assert!(app.world().get::<BodyTracking>(avatar).is_none());

        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .set_praying_menu(false);
        app.update();
        assert!(app.world().get::<LookAt>(avatar).is_some());
        assert!(app.world().get::<BodyTracking>(avatar).is_some());

        app.world_mut().resource_mut::<NeraSettings>().follow_eyes = false;
        app.update();
        assert!(app.world().get::<LookAt>(avatar).is_none());
        assert!(app.world().get::<BodyTracking>(avatar).is_none());
    }
}
