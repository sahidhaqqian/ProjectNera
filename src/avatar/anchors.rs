//! Project the live avatar skeleton into desktop support anchors and ground Death poses.
use std::time::Duration;

use super::NeraAvatar;
use crate::{
    animation::{BasePose, NeraAnimationState, VisualState},
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

fn support_anchor_geometry(settings: &NeraSettings, hips: Entity) -> (Entity, f32, f32) {
    (
        hips,
        settings.character_scale(),
        settings.ground_offset_logical(),
    )
}

/// Keep the last standing sample until the idle blend has removed the seated
/// or sideways pose. Native physics may need that sample on the first frame
/// after detaching, before any new standing pose has been evaluated.
const STANDING_ANCHOR_SETTLE: Duration = Duration::from_millis(250);

#[derive(Default)]
pub(crate) struct StandingAnchorSampling {
    settle_remaining: Duration,
}

impl StandingAnchorSampling {
    fn ready(&mut self, visual: VisualState, turning: bool, delta: Duration) -> bool {
        if turning
            || seated_fidget_or_dodge(visual)
            || matches!(
                visual,
                VisualState::Walking
                    | VisualState::SittingEnter
                    | VisualState::SittingLoop
                    | VisualState::SittingExit
                    | VisualState::Death
                    | VisualState::LibraryAnimation(_)
            )
        {
            self.settle_remaining = STANDING_ANCHOR_SETTLE;
            return false;
        }
        // A delayed frame must not instantly admit an unevaluated old pose.
        self.settle_remaining = self
            .settle_remaining
            .saturating_sub(delta.min(Duration::from_millis(50)));
        self.settle_remaining.is_zero()
    }
}

fn seated_fidget_or_dodge(visual: VisualState) -> bool {
    match visual {
        VisualState::DodgeWhileSitting => true,
        VisualState::IdleMicro(action) => action.base_pose() == BasePose::Sitting,
        _ => false,
    }
}

/// A seated gesture moves the body relative to its support. Recalibrating the
/// seat from those hips would make the native window chase the gesture instead.
/// Wait through the return blend before sampling the ordinary seated pose again.
#[derive(Default)]
pub(crate) struct SeatAnchorSampling {
    settle_remaining: Duration,
}

impl SeatAnchorSampling {
    fn ready(&mut self, visual: VisualState, attached: bool, delta: Duration) -> bool {
        if !attached {
            // Free/dragging probes must immediately track the live pose so the
            // next drop can find a new seat without using the old attachment.
            self.settle_remaining = Duration::ZERO;
            return true;
        }
        if seated_fidget_or_dodge(visual) {
            self.settle_remaining = STANDING_ANCHOR_SETTLE;
            return false;
        }
        self.settle_remaining = self
            .settle_remaining
            .saturating_sub(delta.min(Duration::from_millis(50)));
        self.settle_remaining.is_zero()
    }
}

#[derive(Default)]
pub(crate) struct SeatedHipsAnchor {
    hips: Option<(Entity, Vec3)>,
    sampling: SeatAnchorSampling,
}

impl SeatedHipsAnchor {
    fn apply(
        &mut self,
        entity: Entity,
        visual: VisualState,
        attached: bool,
        delta: Duration,
        pose: &mut Transform,
    ) {
        if !attached {
            *self = Self::default();
            return;
        }
        if self.hips.is_some_and(|(owner, _)| owner != entity) {
            *self = Self::default();
        }
        let ready = self.sampling.ready(visual, true, delta);
        if seated_fidget_or_dodge(visual) || (!ready && visual == VisualState::SittingLoop) {
            if let Some((_, translation)) = self.hips {
                pose.translation = translation;
            }
        } else if ready && matches!(visual, VisualState::SittingEnter | VisualState::SittingLoop) {
            self.hips = Some((entity, pose.translation));
        }
    }
}

/// Imported seated clips have different authored pelvis heights. Keep the
/// calibrated seated translation while their rotations still perform the
/// gesture, without making the native attachment or the avatar jump to a new
/// seat. Run before VRM constraints/transform propagation, like walking's
/// existing root-motion correction.
pub(crate) fn preserve_seated_hips(
    time: Res<Time>,
    animation: Res<NeraAnimationState>,
    environment: Res<DesktopEnvironment>,
    mut anchor: Local<SeatedHipsAnchor>,
    avatar: Query<&HipsBoneEntity, With<NeraAvatar>>,
    mut bones: Query<&mut Transform, Without<NeraAvatar>>,
) {
    let Ok(hips) = avatar.single() else {
        *anchor = SeatedHipsAnchor::default();
        return;
    };
    let Ok(mut pose) = bones.get_mut(hips.0) else {
        return;
    };
    anchor.apply(
        hips.0,
        animation.visual_state(),
        environment.placement != PlacementState::Free,
        time.delta(),
        &mut pose,
    );
}

#[cfg(target_os = "windows")]
pub(crate) fn update_avatar_anchors(
    time: Res<Time>,
    interaction: Res<DesktopPetInteraction>,
    animation: Res<NeraAnimationState>,
    library: Option<Res<crate::animation::LibraryPlayback>>,
    walking: Res<crate::walking::WalkingRuntime>,
    settings: Res<NeraSettings>,
    mut physics: ResMut<desktop_physics::DesktopPhysicsState>,
    mut grounding: ResMut<DeathVisualGrounding>,
    mut environment: ResMut<DesktopEnvironment>,
    mut applied_support_geometry: Local<Option<(Entity, f32, f32)>>,
    mut standing_sample: Local<StandingAnchorSampling>,
    mut seat_sample: Local<SeatAnchorSampling>,
    camera_query: Query<(&Camera, &GlobalTransform), With<super::NeraMainCamera>>,
    mut avatar_query: Query<
        (
            &mut Transform,
            &GlobalTransform,
            Option<&HipsBoneEntity>,
            Has<crate::character::CharacterReveal>,
        ),
        With<NeraAvatar>,
    >,
    children_query: Query<&Children>,
    bone_query: Query<(&VrmBone, &GlobalTransform)>,
) {
    let death_pose = animation.death_active() || matches!(animation.visual_state(), VisualState::LibraryAnimation(entity) if library.as_ref().and_then(|library| library.path_for(entity)).is_some_and(|path| path == "animations/animasi_basic/Death01.vrma"));
    let standing_sample_ready =
        standing_sample.ready(animation.visual_state(), walking.active(), time.delta());
    let seat_sample_ready = seat_sample.ready(
        animation.visual_state(),
        environment.placement != PlacementState::Free,
        time.delta(),
    );
    // Seat/hips must keep updating even during a free drag because it is the
    // live sitting probe. Feet stay cached from the last standing frame so
    // dangling feet from sitting/death clips never become the rigid support.
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };

    let Ok((mut avatar_transform, avatar_global, hips_bone_entity, preparing_character)) =
        avatar_query.single_mut()
    else {
        return;
    };
    // Preserve the previous contact points while a replacement rig is hidden
    // for pose preparation. Initial startup still calibrates its hidden rig.
    if preparing_character {
        return;
    }

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

        if death_pose {
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
        && standing_sample_ready
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
        // A replacement rig also changes geometry even at the same size. Keep
        // that refresh pending through loading, sitting and airborne poses,
        // then apply it once a standing sample can safely own the support.
        let geometry = support_anchor_geometry(&settings, hips_bone_entity.0);
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
    let death_active = death_pose;

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

    // Calibrate normal sitting/entry, but retain its support through seated
    // gestures and their outgoing blend. Free probes still follow live hips.
    if seat_sample_ready && let Some(seat) = seat {
        environment.set_seat_anchor(seat);
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use crate::platform::PhysicalRect;

    #[test]
    fn standing_cache_waits_for_sitting_and_walking_blends_to_settle() {
        for visual in [
            VisualState::SittingEnter,
            VisualState::SittingLoop,
            VisualState::SittingExit,
            VisualState::Walking,
            VisualState::Death,
        ] {
            let mut sampling = StandingAnchorSampling::default();
            assert!(sampling.ready(VisualState::Idle, false, Duration::ZERO));
            assert!(!sampling.ready(visual, false, Duration::from_secs(1)));
            for _ in 0..4 {
                assert!(!sampling.ready(VisualState::Idle, false, Duration::from_millis(50)));
            }
            assert!(sampling.ready(VisualState::Idle, false, Duration::from_millis(50)));
        }
    }

    #[test]
    fn seated_gestures_preserve_calibrated_anchors_through_the_return_blend() {
        use crate::animation::IdleMicroAction;

        for visual in IdleMicroAction::SITTING
            .into_iter()
            .map(VisualState::IdleMicro)
            .chain([VisualState::DodgeWhileSitting])
        {
            let mut seat = SeatAnchorSampling::default();
            let mut feet = StandingAnchorSampling::default();
            assert!(seat.ready(VisualState::SittingLoop, true, Duration::ZERO));
            assert!(!seat.ready(visual, true, Duration::from_secs(1)));
            assert!(!feet.ready(visual, false, Duration::from_secs(1)));
            // Even a long frame cannot resample the still-blending outgoing pose.
            assert!(!seat.ready(VisualState::SittingLoop, true, Duration::from_secs(1),));
            for _ in 0..3 {
                assert!(!seat.ready(VisualState::SittingLoop, true, Duration::from_millis(50),));
            }
            assert!(seat.ready(VisualState::SittingLoop, true, Duration::from_millis(50),));
        }
    }

    #[test]
    fn detaching_during_a_seated_gesture_immediately_restores_the_live_seat_probe() {
        let mut sampling = SeatAnchorSampling::default();
        assert!(!sampling.ready(VisualState::DodgeWhileSitting, true, Duration::ZERO));
        assert!(sampling.ready(VisualState::Floating, false, Duration::ZERO));
        assert!(sampling.ready(VisualState::SittingEnter, true, Duration::ZERO));
    }

    #[test]
    fn seated_gestures_keep_the_calibrated_pelvis_without_erasing_their_rotation() {
        use crate::animation::IdleMicroAction;

        let mut world = World::default();
        let hips = world.spawn_empty().id();
        for visual in IdleMicroAction::SITTING
            .into_iter()
            .map(VisualState::IdleMicro)
            .chain([VisualState::DodgeWhileSitting])
        {
            let mut anchor = SeatedHipsAnchor::default();
            let mut entering = Transform::from_xyz(0.01, 0.85, -0.2);
            anchor.apply(
                hips,
                VisualState::SittingEnter,
                true,
                Duration::ZERO,
                &mut entering,
            );
            let mut seated = Transform::from_xyz(0.02, 0.65, -0.3);
            anchor.apply(
                hips,
                VisualState::SittingLoop,
                true,
                Duration::ZERO,
                &mut seated,
            );
            let authored = Transform {
                translation: Vec3::new(0.0, 0.2, 0.1),
                rotation: Quat::from_rotation_z(0.25),
                scale: Vec3::splat(0.95),
            };
            let mut gesture = authored;
            anchor.apply(hips, visual, true, Duration::ZERO, &mut gesture);
            assert_eq!(gesture.translation, seated.translation);
            assert_eq!(gesture.rotation, authored.rotation);
            assert_eq!(gesture.scale, authored.scale);

            // The outgoing crossfade may still carry the imported translation.
            let mut blended = authored;
            anchor.apply(
                hips,
                VisualState::SittingLoop,
                true,
                Duration::from_secs(1),
                &mut blended,
            );
            assert_eq!(blended.translation, seated.translation);
            let mut detached = authored;
            anchor.apply(
                hips,
                VisualState::Floating,
                false,
                Duration::ZERO,
                &mut detached,
            );
            assert_eq!(detached, authored);
            assert!(anchor.hips.is_none());
        }
    }

    #[test]
    fn a_new_rig_or_missing_seated_sample_never_reuses_an_old_pelvis() {
        let mut world = World::default();
        let hips = world.spawn_empty().id();
        let replacement = world.spawn_empty().id();
        let mut anchor = SeatedHipsAnchor::default();
        let mut unknown = Transform::from_xyz(0.0, 0.4, 0.0);
        anchor.apply(
            hips,
            VisualState::DodgeWhileSitting,
            true,
            Duration::ZERO,
            &mut unknown,
        );
        assert_eq!(unknown.translation.y, 0.4);

        anchor = SeatedHipsAnchor::default();
        let mut seated = Transform::from_xyz(0.0, 0.6, 0.0);
        anchor.apply(
            hips,
            VisualState::SittingLoop,
            true,
            Duration::ZERO,
            &mut seated,
        );
        let mut new_pose = Transform::from_xyz(0.0, 0.9, 0.0);
        anchor.apply(
            replacement,
            VisualState::DodgeWhileSitting,
            true,
            Duration::ZERO,
            &mut new_pose,
        );
        assert_eq!(new_pose.translation.y, 0.9);
        assert!(anchor.hips.is_none());
    }

    #[test]
    fn turning_while_idle_cannot_replace_standing_cache_even_after_a_hitch() {
        let mut sampling = StandingAnchorSampling::default();
        for _ in 0..10 {
            assert!(!sampling.ready(VisualState::Idle, true, Duration::from_millis(50)));
        }
        assert!(!sampling.ready(VisualState::Idle, false, Duration::from_secs(10)));
        // A fresh turn during the blend must restart the whole settling period.
        assert!(!sampling.ready(VisualState::Idle, true, Duration::ZERO));
        assert_eq!(sampling.settle_remaining, STANDING_ANCHOR_SETTLE);
        for _ in 0..4 {
            assert!(!sampling.ready(VisualState::Idle, false, Duration::from_millis(50)));
        }
        assert!(sampling.ready(VisualState::Idle, false, Duration::from_millis(50)));
    }

    #[test]
    fn moving_the_window_or_toggling_walking_does_not_change_support_geometry() {
        let mut settings = NeraSettings::default();
        let mut world = World::new();
        let hips = world.spawn_empty().id();
        let replacement = world.spawn_empty().id();
        let standing_geometry = support_anchor_geometry(&settings, hips);
        settings.window_x += 200;
        settings.window_y -= 100;
        settings.walking_enabled = !settings.walking_enabled;
        settings.follow_mouse = !settings.follow_mouse;
        assert_eq!(support_anchor_geometry(&settings, hips), standing_geometry);
        assert_ne!(
            support_anchor_geometry(&settings, replacement),
            standing_geometry,
            "a replacement character must refresh standing support at the same settings"
        );

        settings.character_size = 125;
        assert_ne!(support_anchor_geometry(&settings, hips), standing_geometry);
        let resized_geometry = support_anchor_geometry(&settings, hips);
        settings.ground_offset = 10;
        assert_ne!(support_anchor_geometry(&settings, hips), resized_geometry);
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
