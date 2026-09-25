//! Avatar scene, gaze ownership, and pose-derived support anchors.
//! Scheduling remains in main so cross-subsystem ordering stays explicit.
mod anchors;

pub(crate) use anchors::{DeathVisualGrounding, update_avatar_anchors};

use crate::{
    animation::{NeraAnimationState, spawn_animation_library},
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
pub(crate) struct LookAtTarget;

const VRM_PATH: &str = "vrm/nera_256.vrm";

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
            VrmHandle(asset_server.load(VRM_PATH)),
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

    let desired = if settings.follow_mouse {
        cursor_position_in_nera_window(&environment)
            .map(|cursor_pos| {
                let intensity = f32::from(settings.follow_intensity.clamp(25, 100)) / 100.0;

                let map_x = (cursor_pos.x / window.resolution.width() - 0.5) * 4.0 * intensity;

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

/// VRMA reactions such as HitHead and ShakeHeadNo1 need temporary ownership of
/// the head bones. `bevy_vrm1` LookAt runs after animation, so the animation
/// suppression rule remains authoritative over the user Follow Mouse settings.
pub(crate) fn sync_look_at_for_animation(
    animation: Res<NeraAnimationState>,
    settings: Res<NeraSettings>,
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
    let allow_look_at = settings.follow_eyes && !animation.suppresses_look_at();
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

#[cfg(test)]
mod tests {
    use super::*;

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
