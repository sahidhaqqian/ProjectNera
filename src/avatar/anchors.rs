//! Project the live pose into desktop anchors and preserve its selected support plane.
use std::time::Duration;

use super::{NeraAvatar, body_mode::BodyProportions};
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

pub(crate) fn seat_contact_world(hips: &GlobalTransform) -> Vec3 {
    hips.transform_point(SEAT_OFFSET_WORLD)
}

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
#[derive(Resource, Debug, Default)]
pub(crate) struct AvatarVisualGrounding {
    owner: Option<Entity>,
    death_floor_y_logical: Option<f32>,
    /// A reversible pose correction, never a native placement or scale owner.
    raw_root_y: Option<(Entity, f32)>,
    support_geometry: Option<(Entity, f32, f32)>,
}

#[cfg(debug_assertions)]
impl AvatarVisualGrounding {
    pub(crate) fn raw_root_y_for_validation(&self) -> Option<f32> {
        self.raw_root_y.map(|(_, y)| y)
    }
}

/// Restore before pose evaluation and character replacement. Otherwise the next
/// sample (or replacement rig) would inherit yesterday's visual correction.
pub(crate) fn restore_visual_grounding(
    mut grounding: ResMut<AvatarVisualGrounding>,
    mut avatars: Query<&mut Transform, With<NeraAvatar>>,
) {
    if let Some((entity, y)) = grounding.raw_root_y.take()
        && let Ok(mut transform) = avatars.get_mut(entity)
    {
        transform.translation.y = y;
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

pub(crate) fn standing_support_offset(settings: &NeraSettings, body_scale: f32) -> f32 {
    FOOT_SOLE_OFFSET_LOGICAL * settings.character_scale() * body_scale
        + settings.ground_offset_logical()
}

/// The footprint can change while a gait owns the pose; its contact plane cannot.
fn standing_anchor_after_body_change(standing: Vec2, feet: &[Vec2]) -> Option<Vec2> {
    let valid: Vec<_> = feet.iter().filter(|point| point.is_finite()).collect();
    if valid.is_empty() || !standing.is_finite() {
        return None;
    }
    let x = valid.iter().map(|point| point.x).sum::<f32>() / valid.len() as f32;
    Some(Vec2::new(x, standing.y))
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
        // Only settled neutral standing may calibrate collision geometry.
        // Imported hips motion/reactions must not silently move the floor.
        if turning || visual != VisualState::Idle {
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

/// Solve the required root movement at the actual contact depth. A screen-space
/// pixel/world ratio at the avatar origin is inaccurate for forward-moving VRMA
/// hips and feet in a perspective camera.
pub(crate) fn contact_y_correction(
    camera: &Camera,
    view: &GlobalTransform,
    contact: Vec3,
    target_y: f32,
) -> Option<f32> {
    let viewport = camera.world_to_viewport(view, contact).ok()?;
    let ray = camera
        .viewport_to_world(view, Vec2::new(viewport.x, target_y))
        .ok()?;
    if ray.direction.z.abs() < 0.0001 {
        return None;
    }
    let distance = (contact.z - ray.origin.z) / ray.direction.z;
    let correction = ray.get_point(distance).y - contact.y;
    (distance.is_finite() && distance >= 0.0 && correction.is_finite()).then_some(correction)
}

/// Opt-in native smoke assertion: measure the fully propagated rendered pose,
/// not the corrective root transform or an unevaluated test skeleton.
#[cfg(all(debug_assertions, target_os = "windows"))]
pub(crate) fn assert_smoke_ground_contact(world: &mut World) {
    use bevy::ecs::system::RunSystemOnce;
    world
        .run_system_once(
            |physics: Res<desktop_physics::DesktopPhysicsState>,
             environment: Res<DesktopEnvironment>,
             settings: Res<NeraSettings>,
             motion: Option<Res<crate::animation::LibraryMotion>>,
             proportions: Option<Res<BodyProportions>>,
             camera: Query<(&Camera, &GlobalTransform), With<super::NeraMainCamera>>,
             bones: Query<(&VrmBone, &GlobalTransform), With<crate::interaction::NeraLiveBone>>| {
                assert!(!physics.falling && physics.support.is_some());
                let (camera, view) = camera.single().unwrap();
                let foot_y = bones
                    .iter()
                    .filter(|(bone, _)| matches!(bone.as_str(), "leftFoot" | "rightFoot"))
                    .filter_map(|(_, pose)| camera.world_to_viewport(view, pose.translation()).ok())
                    .map(|point| point.y)
                    .max_by(f32::total_cmp)
                    .unwrap();
                let anchor = physics
                    .support_anchor_logical()
                    .or(environment.feet_anchor_logical)
                    .unwrap();
                let margin = standing_support_offset(
                        &settings,
                        proportions.as_ref().map_or(1.0, |body| body.body_scale()),
                    );
                // The checkpoint owner can move the HWND after physics sampled
                // its local anchor. Compare in the current window coordinate frame.
                let target = motion.as_ref().and_then(|motion| motion.smoke_contact_target(&environment, margin))
                    .unwrap_or(anchor.y - margin);
                assert!(
                    (foot_y - target).abs() < 2.0,
                    "rendered grounded foot escaped support: foot={foot_y} target={target}"
                );
            },
        )
        .unwrap();
}

#[cfg(target_os = "windows")]
pub(crate) fn preserve_grounded_pose(
    animation: Res<NeraAnimationState>,
    owners: AnchorOwners,
    settings: Res<NeraSettings>,
    physics: Res<desktop_physics::DesktopPhysicsState>,
    environment: Res<DesktopEnvironment>,
    interaction: Res<DesktopPetInteraction>,
    presentation: Res<crate::presentation::PresentationState>,
    mut grounding: ResMut<AvatarVisualGrounding>,
    camera: Query<(&Camera, &GlobalTransform), With<super::NeraMainCamera>>,
    mut avatars: Query<
        (
            Entity,
            &mut Transform,
            &HipsBoneEntity,
            Has<crate::character::CharacterReveal>,
        ),
        With<NeraAvatar>,
    >,
    bones: Query<(&VrmBone, &GlobalTransform), With<crate::interaction::NeraLiveBone>>,
) {
    if owners
        .motion
        .as_ref()
        .is_some_and(|motion| motion.owns_placement())
    {
        return;
    }
    use crate::animation::{GroundingPolicy, grounding_policy};
    let Ok((entity, mut root, hips, preparing_character)) = avatars.single_mut() else {
        *grounding = AvatarVisualGrounding::default();
        return;
    };
    if grounding.owner != Some(entity) {
        *grounding = AvatarVisualGrounding {
            owner: Some(entity),
            ..default()
        };
    }
    let visual = animation.visual_state();
    let path = match visual {
        VisualState::LibraryAnimation(clip) => owners
            .library
            .as_ref()
            .and_then(|l| l.grounding_path_for(clip)),
        _ => None,
    };
    let policy = grounding_policy(visual, path);
    if preparing_character
        || presentation.is_attention()
        || environment.placement != PlacementState::Free
        || owners
            .cloud
            .as_ref()
            .is_some_and(|cloud| cloud.owns_placement())
        || interaction.state == InteractionState::Dragging
        || interaction.drag_offset.is_some()
        || physics.falling
        || policy == GroundingPolicy::Authored
        || (policy == GroundingPolicy::Grounded && animation.base_pose() != BasePose::Idle)
    {
        grounding.death_floor_y_logical = None;
        return;
    }
    let Ok((camera, view)) = camera.single() else {
        return;
    };
    let character_scale = settings.character_scale();
    let target_y = if policy == GroundingPolicy::BodyContact {
        if grounding.death_floor_y_logical.is_none() {
            grounding.death_floor_y_logical = environment
                .feet_anchor_logical
                .map(|a| a.y)
                .or_else(|| death_fallback_floor_y_logical(&environment));
        }
        let Some(floor) = grounding.death_floor_y_logical else {
            return;
        };
        floor - DEATH_CONTACT_MARGIN_LOGICAL * character_scale
    } else {
        grounding.death_floor_y_logical = None;
        // Resizing/replacing a rig must first calibrate neutral standing; it
        // cannot reuse the previous character's scale/contact geometry.
        if grounding.support_geometry != Some(support_anchor_geometry(&settings, hips.0)) {
            return;
        }
        let Some(anchor) = physics
            .support_anchor_logical()
            .or(environment.feet_anchor_logical)
        else {
            return;
        };
        anchor.y
            - standing_support_offset(
                &settings,
                owners
                    .proportions
                    .as_ref()
                    .map_or(1.0, |body| body.body_scale()),
            )
    };
    let contact = bones
        .iter()
        .filter(|(bone, _)| {
            policy == GroundingPolicy::BodyContact
                || matches!(bone.as_str(), "leftFoot" | "rightFoot")
        })
        .filter_map(|(_, pose)| {
            let world = pose.translation();
            camera
                .world_to_viewport(view, world)
                .ok()
                .map(|p| (p.y, world))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0));
    let Some((_, contact)) = contact else { return };
    let Some(mut correction) = contact_y_correction(camera, view, contact, target_y) else {
        return;
    };
    if policy == GroundingPolicy::BodyContact {
        // Preserve Death's existing downward-only bounded body contact policy.
        correction = correction.clamp(-DEATH_MAX_ROOT_DROP_WORLD * character_scale.max(0.25), 0.0);
    }
    if correction.abs() > 0.0001 {
        grounding.raw_root_y = Some((entity, root.translation.y));
        root.translation.y += correction;
    }
}

fn seated_fidget_or_dodge(visual: VisualState) -> bool {
    match visual {
        VisualState::DodgeWhileSitting
        | VisualState::TouchReaction(_)
        | VisualState::FocusActivity(_) => true,
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
    cloud: Option<Res<crate::cloud::CloudRuntime>>,
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
        environment.placement != PlacementState::Free
            || cloud.as_ref().is_some_and(|cloud| cloud.owns_placement()),
        time.delta(),
        &mut pose,
    );
}

#[cfg(target_os = "windows")]
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct AnchorOwners<'w> {
    motion: Option<Res<'w, crate::animation::LibraryMotion>>,
    library: Option<Res<'w, crate::animation::LibraryPlayback>>,
    cloud: Option<Res<'w, crate::cloud::CloudRuntime>>,
    clips: Option<Res<'w, crate::animation::NeraAnimationClips>>,
    proportions: Option<Res<'w, BodyProportions>>,
}

/// Projection caches remember which applied geometry they have sampled. A
/// seated/airborne change leaves standing calibration pending until it is safe.
#[derive(Default)]
pub(crate) struct BodyAnchorRevision {
    standing: Option<u64>,
    seat: Option<u64>,
}

#[cfg(target_os = "windows")]
pub(crate) fn update_avatar_anchors(
    time: Res<Time>,
    interaction: Res<DesktopPetInteraction>,
    animation: Res<NeraAnimationState>,
    owners: AnchorOwners,
    walking: Res<crate::walking::WalkingRuntime>,
    settings: Res<NeraSettings>,
    mut physics: ResMut<desktop_physics::DesktopPhysicsState>,
    mut grounding: ResMut<AvatarVisualGrounding>,
    mut environment: ResMut<DesktopEnvironment>,
    mut standing_sample: Local<StandingAnchorSampling>,
    mut seat_sample: Local<SeatAnchorSampling>,
    mut body_revision: Local<BodyAnchorRevision>,
    camera_query: Query<(&Camera, &GlobalTransform), With<super::NeraMainCamera>>,
    mut avatar_query: Query<
        (
            Option<&HipsBoneEntity>,
            Has<crate::character::CharacterReveal>,
        ),
        With<NeraAvatar>,
    >,
    children_query: Query<&Children>,
    bone_query: Query<(&VrmBone, &GlobalTransform)>,
) {
    if owners
        .motion
        .as_ref()
        .is_some_and(|motion| motion.owns_placement())
    {
        return;
    }
    let cloud = owners.cloud;
    let standing_sample_ready =
        standing_sample.ready(animation.visual_state(), walking.active(), time.delta());
    let seat_sample_ready = seat_sample.ready(
        animation.visual_state(),
        environment.placement != PlacementState::Free
            || cloud.as_ref().is_some_and(|cloud| cloud.owns_placement()),
        time.delta(),
    );
    // Seat/hips must keep updating even during a free drag because it is the
    // live sitting probe. Feet stay cached from the last standing frame so
    // dangling feet from sitting/death clips never become the rigid support.
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };

    let Ok((hips_bone_entity, preparing_character)) = avatar_query.single_mut() else {
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

    let revision = owners
        .proportions
        .as_ref()
        .map_or(0, |body| body.revision());
    let body_geometry_changed = body_revision.seat != Some(revision);
    let standing_geometry_changed = body_revision.standing != Some(revision);
    let support_offset = standing_support_offset(
        &settings,
        owners
            .proportions
            .as_ref()
            .map_or(1.0, |body| body.body_scale()),
    );

    let mut feet = Vec::with_capacity(2);
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

        match bone.as_str() {
            "leftFoot" | "rightFoot" => {
                if let Ok(viewport) = camera.world_to_viewport(camera_transform, world) {
                    feet.push(viewport);
                }
            }

            "hips" => {
                // The hips bone is the pelvis center, not the actual contact
                // point with a chair/window edge.
                let sleeping = cloud.as_ref().is_some_and(|cloud| cloud.is_lying_rest())
                    && owners
                        .clips
                        .as_ref()
                        .is_some_and(|clips| clips.sleeping_idle_entity().is_some());
                let seat_world = cloud.as_ref().map_or_else(
                    || seat_contact_world(transform),
                    |cloud| cloud.contact_world(transform, sleeping),
                );

                if let Ok(viewport) = camera.world_to_viewport(camera_transform, seat_world) {
                    seat = Some(viewport);
                }
            }

            _ => {}
        }
    }

    // A body transition changes the horizontal footprint even during walking.
    // Keep physics' rigid contact plane and support identity; the pose-grounding
    // pass has already aligned the resized feet with that same plane.
    if standing_geometry_changed
        && environment.placement == PlacementState::Free
        && !cloud.as_ref().is_some_and(|cloud| cloud.owns_placement())
        && interaction.state != InteractionState::Dragging
        && interaction.drag_offset.is_none()
        && !physics.falling
        && grounding.support_geometry
            == Some(support_anchor_geometry(&settings, hips_bone_entity.0))
        && let Some(standing) = physics
            .support_anchor_logical()
            .or(environment.feet_anchor_logical)
        && let Some(anchor) = standing_anchor_after_body_change(standing, &feet)
    {
        environment.set_feet_anchor(anchor);
        physics.refresh_support_anchor(Some(anchor));
        body_revision.standing = Some(revision);
    }

    // Feet means *standing support*. A sideways walking gait must not replace
    // this cache either: a drag begun mid-walk resets physics and later reuses
    // this standing sample when acquiring support after release.
    if environment.placement == PlacementState::Free
        && !cloud.as_ref().is_some_and(|cloud| cloud.owns_placement())
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
            + support_offset;

        environment.set_feet_anchor(Vec2::new(x, y));

        // Remembered HWND positions change settings on every walking step.
        // Only geometry changes may replace the rigid standing support with
        // the current pose; otherwise the gait would move its own floor.
        // A replacement rig also changes geometry even at the same size. Keep
        // that refresh pending through loading, sitting and airborne poses,
        // then apply it once a standing sample can safely own the support.
        let geometry = support_anchor_geometry(&settings, hips_bone_entity.0);
        if grounding.support_geometry != Some(geometry) && !physics.falling {
            physics.refresh_support_anchor(environment.feet_anchor_logical);
            grounding.support_geometry = Some(geometry);
        }
        if !physics.falling {
            body_revision.standing = Some(revision);
        }
    }

    // Calibrate normal sitting/entry, but retain its support through seated
    // gestures and their outgoing blend. Free probes still follow live hips.
    if (seat_sample_ready || body_geometry_changed)
        && let Some(seat) = seat
    {
        environment.set_seat_anchor(seat);
        if body_geometry_changed {
            environment.realign_sitting_attachment();
        }
        body_revision.seat = Some(revision);
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    use crate::platform::PhysicalRect;
    use bevy::{
        camera::{CameraProjection, RenderTargetInfo},
        ecs::system::RunSystemOnce,
    };

    fn grounding_world() -> (
        World,
        Entity,
        Entity,
        Entity,
        Camera,
        GlobalTransform,
        Transform,
    ) {
        let mut world = World::new();
        let mut projection = PerspectiveProjection::default();
        projection.aspect_ratio = 600.0 / 950.0;
        let mut camera = Camera::default();
        camera.computed.clip_from_view = projection.get_clip_from_view();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: UVec2::new(600, 950),
            scale_factor: 1.0,
        });
        let view = GlobalTransform::from(Transform::from_xyz(0.0, 0.8, 4.5));
        world.spawn((super::super::NeraMainCamera, camera.clone(), view));
        let hips = world.spawn_empty().id();
        let root = Transform {
            translation: Vec3::new(0.0, -0.85, 0.0),
            rotation: Quat::from_rotation_y(0.1),
            scale: Vec3::splat(1.2),
        };
        let avatar = world.spawn((NeraAvatar, root, HipsBoneEntity(hips))).id();
        let feet = world
            .spawn((
                crate::interaction::NeraLiveBone,
                VrmBone::from("leftFoot"),
                GlobalTransform::from_translation(Vec3::ZERO),
            ))
            .id();
        let settings = NeraSettings::default();
        let floor = camera.world_to_viewport(&view, Vec3::ZERO).unwrap().y
            + FOOT_SOLE_OFFSET_LOGICAL * settings.character_scale();
        world.insert_resource(DesktopEnvironment {
            feet_anchor_logical: Some(Vec2::new(300.0, floor)),
            ..default()
        });
        world.insert_resource(AvatarVisualGrounding {
            owner: Some(avatar),
            support_geometry: Some(support_anchor_geometry(&settings, hips)),
            ..default()
        });
        world.insert_resource(settings);
        world.init_resource::<NeraAnimationState>();
        world.init_resource::<desktop_physics::DesktopPhysicsState>();
        world.init_resource::<DesktopPetInteraction>();
        world.init_resource::<crate::presentation::PresentationState>();
        (world, avatar, hips, feet, camera, view, root)
    }

    #[test]
    fn body_mode_footprint_refresh_preserves_support_plane_over_repeated_toggles() {
        let standing = Vec2::new(300.0, 780.0);
        let mut cached = standing;
        for factor in [0.62, 1.0].into_iter().cycle().take(100) {
            let feet = [
                Vec2::new(300.0 - 30.0 * factor, 720.0 * factor),
                Vec2::new(300.0 + 30.0 * factor, 725.0 * factor),
            ];
            cached = standing_anchor_after_body_change(cached, &feet).unwrap();
            assert_eq!(cached, standing);
        }
        assert!(standing_anchor_after_body_change(standing, &[]).is_none());
        assert!(standing_anchor_after_body_change(standing, &[Vec2::NAN]).is_none());
    }

    #[test]
    fn body_mode_scales_the_existing_sole_margin_without_scaling_user_ground_offset() {
        let settings = NeraSettings {
            character_size: 150,
            ground_offset: 10,
            ..default()
        };
        assert!(
            (standing_support_offset(&settings, 0.62)
                - (FOOT_SOLE_OFFSET_LOGICAL * settings.character_scale() * 0.62
                    + settings.ground_offset_logical()))
            .abs()
                < 0.0001
        );
        assert!(
            (standing_support_offset(&settings, 1.0)
                - (FOOT_SOLE_OFFSET_LOGICAL * settings.character_scale()
                    + settings.ground_offset_logical()))
            .abs()
                < 0.0001
        );
    }

    #[test]
    fn grounded_pose_preserves_screen_support_at_the_animated_contact_depth_without_drift() {
        let (mut world, avatar, _, feet, camera, view, raw) = grounding_world();
        let target = world
            .resource::<DesktopEnvironment>()
            .feet_anchor_logical
            .unwrap()
            .y
            - FOOT_SOLE_OFFSET_LOGICAL;
        for depth in [-0.4, 0.0, 0.8].into_iter().cycle().take(90) {
            let contact = Vec3::new(0.08, 0.35, depth);
            *world.get_mut::<GlobalTransform>(feet).unwrap() =
                GlobalTransform::from_translation(contact);
            world.run_system_once(preserve_grounded_pose).unwrap();
            let grounded = *world.get::<Transform>(avatar).unwrap();
            let corrected = contact + Vec3::Y * (grounded.translation.y - raw.translation.y);
            assert!((camera.world_to_viewport(&view, corrected).unwrap().y - target).abs() < 0.001);
            assert_eq!(grounded.rotation, raw.rotation);
            assert_eq!(grounded.scale, raw.scale);
            world.run_system_once(restore_visual_grounding).unwrap();
            assert_eq!(*world.get::<Transform>(avatar).unwrap(), raw);
        }
    }

    #[test]
    fn death_body_grounding_is_downward_only_bounded_and_reversible() {
        for height in [-1.0, 0.35, 20.0].into_iter().cycle().take(60) {
            let (mut world, avatar, _, contact, camera, view, raw) = grounding_world();
            world
                .resource_mut::<NeraAnimationState>()
                .request_action_for_test(
                    crate::animation::TemporaryAction::Death,
                    Duration::from_secs(2),
                );
            let point = Vec3::new(0.0, height, 0.4);
            *world.get_mut::<GlobalTransform>(contact).unwrap() =
                GlobalTransform::from_translation(point);
            let floor = world
                .resource::<DesktopEnvironment>()
                .feet_anchor_logical
                .unwrap()
                .y;
            let expected =
                contact_y_correction(&camera, &view, point, floor - DEATH_CONTACT_MARGIN_LOGICAL)
                    .unwrap()
                    .clamp(-DEATH_MAX_ROOT_DROP_WORLD, 0.0);
            world.run_system_once(preserve_grounded_pose).unwrap();
            let pose = *world.get::<Transform>(avatar).unwrap();
            assert!((pose.translation.y - raw.translation.y - expected).abs() < 0.00001);
            assert!(pose.translation.y <= raw.translation.y);
            assert_eq!(pose.rotation, raw.rotation);
            assert_eq!(pose.scale, raw.scale);
            world.run_system_once(restore_visual_grounding).unwrap();
            assert_eq!(*world.get::<Transform>(avatar).unwrap(), raw);
        }
    }

    #[test]
    fn airborne_falling_and_drag_do_not_take_standing_contact_ownership() {
        use crate::animation::TemporaryAction;
        for action in [
            TemporaryAction::Floating,
            TemporaryAction::Falling,
            TemporaryAction::AlarmJump,
        ] {
            let (mut world, avatar, _, feet, _, _, raw) = grounding_world();
            world
                .resource_mut::<NeraAnimationState>()
                .request_action_for_test(action, Duration::from_secs(2));
            *world.get_mut::<GlobalTransform>(feet).unwrap() =
                GlobalTransform::from_translation(Vec3::Y);
            world.run_system_once(preserve_grounded_pose).unwrap();
            assert_eq!(*world.get::<Transform>(avatar).unwrap(), raw);
        }
        for dragged in [false, true] {
            let (mut world, avatar, _, feet, _, _, raw) = grounding_world();
            if dragged {
                world.resource_mut::<DesktopPetInteraction>().state = InteractionState::Dragging;
            } else {
                world
                    .resource_mut::<desktop_physics::DesktopPhysicsState>()
                    .falling = true;
            }
            *world.get_mut::<GlobalTransform>(feet).unwrap() =
                GlobalTransform::from_translation(Vec3::Y);
            world.run_system_once(preserve_grounded_pose).unwrap();
            assert_eq!(*world.get::<Transform>(avatar).unwrap(), raw);
        }
    }

    #[test]
    fn replacement_and_resizing_wait_for_fresh_neutral_geometry() {
        let (mut world, avatar, hips, feet, _, _, raw) = grounding_world();
        *world.get_mut::<GlobalTransform>(feet).unwrap() =
            GlobalTransform::from_translation(Vec3::Y);
        world.resource_mut::<NeraSettings>().character_size = 150;
        world.run_system_once(preserve_grounded_pose).unwrap();
        assert_eq!(*world.get::<Transform>(avatar).unwrap(), raw);
        world.despawn(avatar);
        let replacement = world.spawn((NeraAvatar, raw, HipsBoneEntity(hips))).id();
        world.run_system_once(preserve_grounded_pose).unwrap();
        assert_eq!(*world.get::<Transform>(replacement).unwrap(), raw);
        assert!(
            world
                .resource::<AvatarVisualGrounding>()
                .support_geometry
                .is_none()
        );
    }

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
            .chain([
                VisualState::DodgeWhileSitting,
                VisualState::FocusActivity(crate::focus::FocusPreset::Work),
            ])
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
