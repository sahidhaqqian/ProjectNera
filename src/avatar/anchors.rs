//! Project the live avatar skeleton into desktop support anchors and ground Death poses.
use super::NeraAvatar;
use crate::{
    animation::{NeraAnimationState, VisualState},
    desktop_physics,
    environment::{DesktopEnvironment, PlacementState},
    interaction::{DesktopPetInteraction, InteractionState},
    settings::NeraSettings,
};
use bevy::prelude::*;
use bevy_vrm1::prelude::*;

/// VrmBone `foot` is located around the ankle; shift the standing support
/// plane down to the visible sole in logical window pixels.
const FOOT_SOLE_OFFSET_LOGICAL: f32 = 12.0;

/// Local/world-space offset from the VRM hips bone toward the point that should
/// visually touch a sitting surface. Tune this after `sitting.vrma` is wired.
/// +Y is up; the small +Z term moves the anchor slightly toward Nera's rear in
/// the current avatar orientation.
const SEAT_OFFSET_WORLD: Vec3 = Vec3::new(0.0, -0.12, 0.08);

/// VRMA is a 3D pose, while the Windows desktop support surface is a 2D
/// screen-space line. During Death01 we gently reconcile those two spaces by
/// translating only Nera's 3D root after the fall has begun. We never move the
/// native HWND for this correction.
const DEATH_CONTACT_MARGIN_LOGICAL: f32 = 10.0;
const DEATH_MAX_ROOT_DROP_WORLD: f32 = 1.50;

/// Fallback floor line (as a fraction of Nera's own window height) used when
/// Death01 starts without a recent standing `feet_anchor_logical` sample.
/// This can happen when Death is triggered while sitting/attached and the
/// standing-feet cache has never been established for the current placement.
const DEATH_FALLBACK_FLOOR_RATIO: f32 = 0.88;
#[derive(Resource, Debug)]
pub(crate) struct DeathVisualGrounding {
    active: bool,
    base_root_y: f32,
    floor_y_logical: Option<f32>,
}

impl Default for DeathVisualGrounding {
    fn default() -> Self {
        Self {
            active: false,
            base_root_y: 0.0,
            floor_y_logical: None,
        }
    }
}

/// Local-viewport fallback floor used only when Death01 starts without a
/// recent standing feet sample. Normal standing deaths still use the cached
/// `feet_anchor_logical`, which remains the authoritative desktop contact line.
#[cfg(target_os = "windows")]
fn death_fallback_floor_y_logical(environment: &DesktopEnvironment) -> Option<f32> {
    let rect = environment.nera_rect?;
    let scale = environment.nera_scale_factor.max(0.25) as f32;

    Some(rect.height().max(1) as f32 / scale * DEATH_FALLBACK_FLOOR_RATIO)
}

fn support_anchor_geometry(settings: &NeraSettings) -> (f32, f32) {
    (settings.character_scale(), settings.ground_offset_logical())
}

#[cfg(target_os = "windows")]
pub(crate) fn update_avatar_anchors(
    interaction: Res<DesktopPetInteraction>,
    animation: Res<NeraAnimationState>,
    settings: Res<NeraSettings>,
    mut physics: ResMut<desktop_physics::DesktopPhysicsState>,
    mut grounding: ResMut<DeathVisualGrounding>,
    mut environment: ResMut<DesktopEnvironment>,
    mut applied_support_geometry: Local<Option<(f32, f32)>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<super::NeraMainCamera>>,
    mut avatar_query: Query<
        (&mut Transform, &GlobalTransform, Option<&HipsBoneEntity>),
        With<NeraAvatar>,
    >,
    children_query: Query<&Children>,
    bone_query: Query<(&VrmBone, &GlobalTransform)>,
) {
    // Seat/hips must keep updating even during a free drag because it is the
    // live sitting probe. Feet stay cached from the last standing frame so
    // dangling feet from sitting/death clips never become the rigid support.
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };

    let Ok((mut avatar_transform, avatar_global, hips_bone_entity)) = avatar_query.single_mut()
    else {
        return;
    };

    // IMPORTANT:
    // `VrmBone` is not unique to the rendered VRM. Every loaded VRMA also owns
    // humanoid bone entities for retargeting. A global `Query<&VrmBone>` was
    // therefore mixing Nera's live skeleton with every animation-source
    // skeleton (the Death diagnostic exposed >1000 bones).
    //
    // The rendered VRM entity receives `HipsBoneEntity`, which points at the
    // actual live hips node. Walking only that hierarchy isolates Nera's real
    // animated skeleton and excludes every VRMA source rig.
    let Some(hips_bone_entity) = hips_bone_entity else {
        return;
    };

    let character_scale = settings.character_scale();
    let standing_support_offset =
        FOOT_SOLE_OFFSET_LOGICAL * character_scale + settings.ground_offset_logical();

    let mut feet = Vec::with_capacity(2);
    let mut death_support_points = Vec::with_capacity(64);
    let mut seat = None;

    let mut bone_stack = vec![hips_bone_entity.0];

    while let Some(entity) = bone_stack.pop() {
        if let Ok(children) = children_query.get(entity) {
            for &child in children {
                bone_stack.push(child);
            }
        }

        let Ok((bone, transform)) = bone_query.get(entity) else {
            continue;
        };

        let world = transform.translation();

        if animation.death_active() {
            if let Ok(viewport) = camera.world_to_viewport(camera_transform, world) {
                death_support_points.push(viewport);
            }
        }

        match bone.as_str() {
            "leftFoot" | "rightFoot" => {
                if let Ok(viewport) = camera.world_to_viewport(camera_transform, world) {
                    feet.push(viewport);
                }
            }

            "hips" => {
                // The hips bone is the pelvis center, not the actual contact
                // point with a chair/window edge.
                let seat_world = transform.transform_point(SEAT_OFFSET_WORLD);

                if let Ok(viewport) = camera.world_to_viewport(camera_transform, seat_world) {
                    seat = Some(viewport);
                }
            }

            _ => {}
        }
    }

    // Feet means *standing support*. A sideways walking gait must not replace
    // this cache either: a drag begun mid-walk resets physics and later reuses
    // this standing sample when acquiring support after release.
    if environment.placement == PlacementState::Free
        && interaction.state != InteractionState::Dragging
        && interaction.drag_offset.is_none()
        && !matches!(animation.visual_state(), VisualState::Death | VisualState::Walking)
        && !feet.is_empty()
    {
        let x = feet.iter().map(|point| point.x).sum::<f32>() / feet.len() as f32;

        let y = feet
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max)
            + standing_support_offset;

        environment.set_feet_anchor(Vec2::new(x, y));

        // Remembered HWND positions change settings on every walking step.
        // Only geometry changes may replace the rigid standing support with
        // the current pose; otherwise the gait would move its own floor.
        // Retain a pending geometry change while airborne, then apply it on
        // the next supported frame.
        let geometry = support_anchor_geometry(&settings);
        if *applied_support_geometry != Some(geometry) && !physics.falling {
            physics.refresh_support_anchor(environment.feet_anchor_logical);
            *applied_support_geometry = Some(geometry);
        }
    }

    // ---------------------------------------------------------------------
    // Death01 visual grounding
    // ---------------------------------------------------------------------
    //
    // VRMA animates a 3D humanoid pose. The desktop/taskbar/window support is a
    // 2D screen-space line. A normal VRM viewer owns a 3D floor plane, so a
    // "lying down" clip naturally reads as grounded there; Project Nera has no
    // such world floor. Keep the native HWND fixed and reconcile the mismatch
    // inside the 3D scene by constraining Nera's live skeleton to the desktop
    // floor. The VRMA still owns the pose; this only removes any vertical air
    // gap that the desktop renderer would otherwise expose.
    let death_active = animation.death_active();

    if death_active
        && environment.placement == PlacementState::Free
        && interaction.state != InteractionState::Dragging
        && interaction.drag_offset.is_none()
    {
        if !grounding.active {
            grounding.active = true;
            grounding.base_root_y = avatar_transform.translation.y;
            grounding.floor_y_logical = environment
                .feet_anchor_logical
                .map(|anchor| anchor.y)
                .or_else(|| death_fallback_floor_y_logical(&environment));
        }

        if let (Some(floor_y), Some(lowest_y)) = (
            grounding.floor_y_logical,
            death_support_points
                .iter()
                .map(|point| point.y)
                .max_by(|a, b| a.total_cmp(b)),
        ) {
            // Death01 is authored in a normal 3D scene, while Project Nera's
            // "floor" is a desktop-space 2D line. Do not ease toward contact:
            // enforce contact immediately whenever the live animated skeleton
            // is visibly above the floor.
            //
            // Viewport Y grows downward, so a positive gap means the body is
            // hovering and the avatar root must move downward (-world Y).
            let desired_lowest_y = floor_y - DEATH_CONTACT_MARGIN_LOGICAL * character_scale;

            let missing_drop_px = desired_lowest_y - lowest_y;

            if missing_drop_px > 0.5 {
                let root_world = avatar_global.translation();

                let pixels_per_world_y = camera
                    .world_to_viewport(camera_transform, root_world)
                    .ok()
                    .zip(
                        camera
                            .world_to_viewport(camera_transform, root_world + Vec3::Y)
                            .ok(),
                    )
                    .map(|(a, b)| (b.y - a.y).abs())
                    .filter(|value| value.is_finite() && *value > 0.001);

                if let Some(pixels_per_world_y) = pixels_per_world_y {
                    let needed_world = missing_drop_px / pixels_per_world_y;
                    let max_drop = DEATH_MAX_ROOT_DROP_WORLD * character_scale.max(0.25);

                    let already_dropped =
                        (grounding.base_root_y - avatar_transform.translation.y).max(0.0);

                    let remaining_drop = (max_drop - already_dropped).max(0.0);

                    // Hard floor constraint: consume the entire required
                    // correction this frame instead of lerping/catching up.
                    let correction = needed_world.min(remaining_drop);

                    if correction > 0.0001 {
                        avatar_transform.translation.y -= correction;
                    }
                }
            }
        }
    } else if grounding.active {
        // Restore the authored desktop-pet root as soon as Death01 releases
        // ownership. The next VRMA transition then starts from the same root
        // position as every normal animation.
        avatar_transform.translation.y = grounding.base_root_y;

        grounding.active = false;
        grounding.floor_y_logical = None;
    }

    // Seat is pose-dependent and should follow the evaluated hips while Nera
    // is attached/sitting. It is harmless to refresh it while Free as well.
    if let Some(seat) = seat {
        environment.set_seat_anchor(seat);
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use crate::platform::PhysicalRect;

    #[test]
    fn moving_the_window_or_toggling_walking_does_not_change_support_geometry() {
        let mut settings = NeraSettings::default();
        let standing_geometry = support_anchor_geometry(&settings);
        settings.window_x += 200;
        settings.window_y -= 100;
        settings.walking_enabled = !settings.walking_enabled;
        settings.follow_mouse = !settings.follow_mouse;
        assert_eq!(support_anchor_geometry(&settings), standing_geometry);

        settings.character_size = 125;
        assert_ne!(support_anchor_geometry(&settings), standing_geometry);
        let resized_geometry = support_anchor_geometry(&settings);
        settings.ground_offset = 10;
        assert_ne!(support_anchor_geometry(&settings), resized_geometry);
    }

    #[test]
    fn death_fallback_uses_logical_height_independent_of_desktop_origin() {
        let environment = DesktopEnvironment {
            nera_rect: Some(PhysicalRect {
                left: -1800,
                top: -300,
                right: -600,
                bottom: 1600,
            }),
            nera_scale_factor: 2.0,
            ..default()
        };
        assert_eq!(death_fallback_floor_y_logical(&environment), Some(836.0));
    }

    #[test]
    fn death_fallback_requires_a_window_and_clamps_degenerate_geometry() {
        let mut environment = DesktopEnvironment::default();
        assert_eq!(death_fallback_floor_y_logical(&environment), None);
        environment.nera_rect = Some(PhysicalRect::default());
        environment.nera_scale_factor = 0.0;
        assert_eq!(death_fallback_floor_y_logical(&environment), Some(3.52));
    }
}
