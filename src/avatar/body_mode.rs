//! Runtime proportions layered over the evaluated pose of the same VRM.
//!
//! VRMA targets the raw glTF bones. Preserve topology, RestTransform and all
//! animation targets: remove only our previous scale overlay in PreUpdate,
//! then compose it after animation/cloud/walking/seated pose evaluation. Gaze,
//! skinning, support anchors and springs consequently see the same final rig.
#[cfg(test)]
mod leg_tests;
#[cfg(all(debug_assertions, target_os = "windows"))]
mod smoke;
mod springs;
#[cfg(all(debug_assertions, target_os = "windows"))]
pub(crate) use smoke::install as install_smoke;

use bevy::prelude::*;
use bevy_vrm1::prelude::*;

use super::NeraAvatar;
use crate::{
    interaction::{DesktopPetInteraction, InteractionState},
    pet_feedback::{PetFx, PetFxAnchor, PetFxKind, PetSfx, PetSfxKind},
    presentation::PresentationState,
    settings::NeraSettings,
};

const CHIBI_BODY_SCALE: f32 = 0.62;
const CHIBI_HEAD_SCALE: f32 = 1.9;
const CHIBI_UPPER_LEG_FACTOR: f32 = 0.85;

/// The sole semantic owner of the user's body-mode selection. The setting is
/// just its on-disk encoding; runtime rig data below describes the applied pose.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AvatarBodyMode {
    #[default]
    Normal,
    Chibi,
}

impl AvatarBodyMode {
    fn saved(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Chibi => "chibi",
        }
    }

    fn from_saved(value: &str) -> Self {
        if value == "chibi" {
            Self::Chibi
        } else {
            Self::Normal
        }
    }

    fn toggled(self) -> Self {
        match self {
            Self::Normal => Self::Chibi,
            Self::Chibi => Self::Normal,
        }
    }
}

#[derive(Message)]
pub(crate) struct ToggleBodyMode;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BodyProportionSet {
    Apply,
}

struct UpperLegOverlay {
    knee: Entity,
    /// Imported knee offset in its actual upper-leg parent's axes.
    rest_translation: Vec3,
    rest_delta_in_avatar: Vec3,
    raw_translation: Option<Vec3>,
}

struct RigProportions {
    avatar: Entity,
    hips: Entity,
    head: Entity,
    /// Immutable imported local proportions, including non-unit custom scales.
    original_hips: Transform,
    original_head: Transform,
    /// Original standing foot height relative to hips in avatar-local space.
    /// Keeps the compressed skeleton's neutral support base stable for every
    /// pose, including authored jumps and dragging where grounding yields.
    rest_hips_to_feet_y: f32,
    legs: Vec<UpperLegOverlay>,
    /// Change of the lowest neutral foot after only the thigh refinement.
    /// Measured through the imported hierarchy, before the existing body scale.
    rest_leg_foot_delta_y: f32,
    /// The exact evaluated scale before our most recent overlay. Rotation and
    /// translation remain entirely owned by animation and existing pose layers.
    pose_scales: Option<(Vec3, Vec3)>,
    raw_root_translation: Option<Vec3>,
    applied_root_offset_y: f32,
    applied: AvatarBodyMode,
}

#[derive(Resource, Default)]
pub(crate) struct BodyProportions {
    rig: Option<RigProportions>,
    unsupported: Option<Entity>,
    revision: u64,
    spring_reset: bool,
}

impl BodyProportions {
    pub(crate) fn body_scale(&self) -> f32 {
        if self
            .rig
            .as_ref()
            .is_some_and(|rig| rig.applied == AvatarBodyMode::Chibi)
        {
            CHIBI_BODY_SCALE
        } else {
            1.0
        }
    }

    pub(crate) fn head_scale(&self) -> f32 {
        if self.body_scale() == CHIBI_BODY_SCALE {
            CHIBI_HEAD_SCALE
        } else {
            1.0
        }
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn root_offset_y(&self) -> f32 {
        self.rig
            .as_ref()
            .map_or(0.0, |rig| rig.applied_root_offset_y)
    }

    /// Predict the correction after a placement owner releases the avatar.
    /// Attention uses the standing pose even when entered from a cloud.
    pub(crate) fn standing_root_offset_y(&self) -> f32 {
        self.rig.as_ref().map_or(0.0, |rig| {
            if rig.applied == AvatarBodyMode::Chibi {
                rig.rest_hips_to_feet_y * (1.0 - CHIBI_BODY_SCALE)
                    - rig.rest_leg_foot_delta_y * CHIBI_BODY_SCALE
            } else {
                0.0
            }
        })
    }
}

pub(crate) struct AvatarBodyModePlugin;

impl Plugin for AvatarBodyModePlugin {
    fn build(&self, app: &mut App) {
        let mode = app
            .world()
            .get_resource::<NeraSettings>()
            .map_or(AvatarBodyMode::Normal, |settings| {
                AvatarBodyMode::from_saved(&settings.body_mode)
            });
        app.insert_resource(mode)
            .init_resource::<BodyProportions>()
            .add_message::<ToggleBodyMode>()
            .add_systems(Update, select_body_mode)
            .add_systems(
                PreUpdate,
                restore_proportions.after(super::restore_visual_grounding),
            )
            .add_systems(
                PostUpdate,
                apply_proportions
                    .in_set(BodyProportionSet::Apply)
                    .after(bevy::app::AnimationSystems)
                    .after(crate::animation::smooth_walking_cycle)
                    .after(crate::cloud::stabilize_cloud_pose)
                    .after(super::apply_walking_pose)
                    .after(super::preserve_seated_hips)
                    .before(VrmSystemSets::Constraints),
            )
            .add_systems(
                PostUpdate,
                reset_changed_springs
                    .after(VrmSystemSets::PropagateAfterExpressions)
                    .before(VrmSystemSets::SpringBone),
            );
    }
}

fn select_body_mode(
    mut toggles: MessageReader<ToggleBodyMode>,
    mut mode: ResMut<AvatarBodyMode>,
    mut settings: ResMut<NeraSettings>,
    mut commands: Commands,
) {
    for _ in toggles.read() {
        *mode = mode.toggled();
    }
    if settings.body_mode != mode.saved() {
        settings.body_mode = mode.saved().into();
        crate::settings::save_settings_deferred(&mut commands);
    }
}

fn restore_proportions(world: &mut World) {
    world.resource_scope(|world, mut proportions: Mut<BodyProportions>| {
        if let Some(rig) = proportions.rig.as_mut() {
            for leg in &mut rig.legs {
                if let Some(raw) = leg.raw_translation.take()
                    && let Some(mut knee) = world.get_mut::<Transform>(leg.knee)
                {
                    knee.translation = raw;
                }
            }
            if let Some((hips_scale, head_scale)) = rig.pose_scales.take() {
                if let Some(mut hips) = world.get_mut::<Transform>(rig.hips) {
                    hips.scale = hips_scale;
                }
                if let Some(mut head) = world.get_mut::<Transform>(rig.head) {
                    head.scale = head_scale;
                }
            }
            // Pose grounding stores the base-corrected root Y. Restore it
            // first, then remove our base correction to recover the exact raw
            // root consumed by native placement/settings/presentation.
            if let Some(raw) = rig.raw_root_translation.take()
                && let Some(mut root) = world.get_mut::<Transform>(rig.avatar)
            {
                root.translation = raw;
            }
        }
    });
}

fn safe_transform(transform: &Transform) -> bool {
    transform.translation.is_finite()
        && transform.rotation.is_finite()
        && transform.scale.is_finite()
        && transform.scale.abs().min_element() > 0.00001
        && transform.scale.abs().max_element() < 100_000.0
}

fn rest_position_in_avatar(world: &World, avatar: Entity, bone: Entity) -> Option<Vec3> {
    let point = rest_matrix_in_avatar(world, avatar, bone)?.transform_point3(Vec3::ZERO);
    point.is_finite().then_some(point)
}

fn rest_matrix_in_avatar(world: &World, avatar: Entity, bone: Entity) -> Option<Mat4> {
    if let (Some(root), Some(bone)) = (
        world.get::<RestGlobalTransform>(avatar),
        world.get::<RestGlobalTransform>(bone),
    ) {
        let inverse = root.0.to_matrix().inverse();
        let matrix = inverse * bone.0.to_matrix();
        return (inverse.is_finite() && matrix.is_finite()).then_some(matrix);
    }
    // Some custom/isolated rigs omit the root's rest-global component. Compose
    // their actual imported local hierarchy instead of assuming parent names
    // or the default model's bone axes and dimensions.
    let mut current = bone;
    let mut matrix = Mat4::IDENTITY;
    let mut visited = std::collections::HashSet::new();
    while current != avatar {
        if !visited.insert(current) {
            return None;
        }
        let local = world
            .get::<RestTransform>(current)
            .map(|rest| rest.0)
            .or_else(|| world.get::<Transform>(current).copied())?;
        if !safe_transform(&local) {
            return None;
        }
        matrix = local.to_matrix() * matrix;
        current = world.get::<ChildOf>(current)?.parent();
    }
    matrix.is_finite().then_some(matrix)
}

fn capture_upper_legs(
    world: &World,
    avatar: Entity,
    mapped: &[(Entity, String)],
) -> Option<Vec<UpperLegOverlay>> {
    let mut legs = Vec::with_capacity(2);
    for (upper_name, knee_name) in [
        ("leftUpperLeg", "leftLowerLeg"),
        ("rightUpperLeg", "rightLowerLeg"),
    ] {
        let find_unique = |name: &str| {
            let mut matches = mapped.iter().filter(|(_, bone)| bone == name);
            let entity = matches.next()?.0;
            matches.next().is_none().then_some(entity)
        };
        let upper = find_unique(upper_name)?;
        let knee = find_unique(knee_name)?;
        if world.get::<ChildOf>(knee)?.parent() != upper {
            return None;
        }
        let rest = world.get::<RestTransform>(knee)?.0;
        if !safe_transform(&rest) || rest.translation.length_squared() < 0.000001 {
            return None;
        }
        let rest_delta_in_avatar = rest_matrix_in_avatar(world, avatar, upper)?
            .transform_vector3(rest.translation * (CHIBI_UPPER_LEG_FACTOR - 1.0));
        if !rest_delta_in_avatar.is_finite() {
            return None;
        }
        legs.push(UpperLegOverlay {
            knee,
            rest_translation: rest.translation,
            rest_delta_in_avatar,
            raw_translation: None,
        });
    }
    Some(legs)
}

fn leg_rest_foot_delta(
    world: &World,
    avatar: Entity,
    hips: Entity,
    feet: &[Entity],
    legs: &[UpperLegOverlay],
    original_lowest_y: f32,
) -> Option<f32> {
    let mut lowest = f32::INFINITY;
    for &foot in feet {
        let mut point = rest_position_in_avatar(world, avatar, foot)?;
        let mut current = foot;
        while current != hips {
            if let Some(leg) = legs.iter().find(|leg| leg.knee == current) {
                point += leg.rest_delta_in_avatar;
                break;
            }
            current = world.get::<ChildOf>(current)?.parent();
        }
        lowest = lowest.min(point.y);
    }
    let delta = lowest - original_lowest_y;
    delta.is_finite().then_some(delta)
}

/// Resolve semantic mappings exclusively within this loaded VRM. The VRMA
/// source skeletons also carry VrmBone, so a global bone-name lookup is unsafe.
fn capture_rig(
    world: &World,
    avatar: Entity,
    hips: Entity,
) -> Result<RigProportions, &'static str> {
    let mut stack = vec![hips];
    let mut visited = std::collections::HashSet::new();
    let mut heads = Vec::new();
    let mut feet = Vec::new();
    let mut mapped = Vec::new();
    while let Some(entity) = stack.pop() {
        if !visited.insert(entity) {
            return Err("cyclic humanoid hierarchy");
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
        if let Some(bone) = world.get::<VrmBone>(entity) {
            mapped.push((entity, bone.as_str().to_string()));
            if !world.get::<Transform>(entity).is_some_and(safe_transform)
                || world
                    .get::<RestTransform>(entity)
                    .is_some_and(|rest| !safe_transform(&rest.0))
                || world
                    .get::<RestGlobalTransform>(entity)
                    .is_some_and(|rest| !rest.0.to_matrix().is_finite())
            {
                return Err("non-finite or degenerate intermediate humanoid bone");
            }
            match bone.as_str() {
                "head" => heads.push(entity),
                "leftFoot" | "rightFoot" => feet.push(entity),
                _ => {}
            }
        }
    }
    if heads.len() != 1
        || feet.is_empty()
        || world.get::<VrmBone>(hips).map(|bone| bone.as_str()) != Some("hips")
    {
        return Err("requires mapped hips, one descendant head and at least one foot");
    }
    let head = heads[0];
    let original_hips = world
        .get::<RestTransform>(hips)
        .map(|rest| rest.0)
        .ok_or("hips rest proportions unavailable")?;
    let original_head = world
        .get::<RestTransform>(head)
        .map(|rest| rest.0)
        .ok_or("head rest proportions unavailable")?;
    if !safe_transform(&original_hips)
        || !safe_transform(&original_head)
        || !world.get::<Transform>(hips).is_some_and(safe_transform)
        || !world.get::<Transform>(head).is_some_and(safe_transform)
    {
        return Err("non-finite or degenerate humanoid proportions");
    }
    if !springs::adapter_available(world, avatar) {
        return Err("spring geometry or registered bevy_vrm1 spring adapter unsupported");
    }
    let rest_hips = rest_position_in_avatar(world, avatar, hips)
        .ok_or("standing hips rest position unavailable")?;
    let rest_feet_y = feet
        .iter()
        .map(|foot| rest_position_in_avatar(world, avatar, *foot).map(|point| point.y))
        .collect::<Option<Vec<_>>>()
        .and_then(|feet| feet.into_iter().min_by(f32::total_cmp))
        .ok_or("standing feet rest position unavailable")?;
    let rest_hips_to_feet_y = rest_feet_y - rest_hips.y;
    if !rest_hips_to_feet_y.is_finite() || rest_hips_to_feet_y >= -0.00001 {
        return Err("standing rest feet must be below hips in avatar space");
    }
    // Refinement is optional. Keep V1 Chibi intact for custom helper chains or
    // missing leg mappings; never impose a default-model axis or one-sided leg.
    let (legs, rest_leg_foot_delta_y) = capture_upper_legs(world, avatar, &mapped)
        .and_then(|legs| {
            leg_rest_foot_delta(world, avatar, hips, &feet, &legs, rest_feet_y)
                .map(|delta| (legs, delta))
        })
        .unwrap_or_else(|| {
            warn!("Chibi upper-leg refinement skipped: missing/unsupported paired knee mapping");
            (Vec::new(), 0.0)
        });
    Ok(RigProportions {
        avatar,
        hips,
        head,
        original_hips,
        original_head,
        rest_hips_to_feet_y,
        legs,
        rest_leg_foot_delta_y,
        pose_scales: None,
        raw_root_translation: None,
        applied_root_offset_y: 0.0,
        applied: AvatarBodyMode::Normal,
    })
}

fn transition_safe(world: &World) -> bool {
    !world
        .get_resource::<PresentationState>()
        .is_some_and(|state| state.is_attention())
        && !world
            .get_resource::<DesktopPetInteraction>()
            .is_some_and(|input| {
                input.state == InteractionState::Dragging || input.drag_offset.is_some()
            })
}

fn apply_proportions(world: &mut World) {
    let desired = *world.resource::<AvatarBodyMode>();
    let safe = transition_safe(world);
    let cloud_owns_root = world
        .get_resource::<crate::cloud::CloudRuntime>()
        .is_some_and(|cloud| cloud.owns_placement());
    let avatar = world
        .query_filtered::<(Entity, &HipsBoneEntity), (With<NeraAvatar>, With<Initialized>)>()
        .single(world)
        .ok()
        .map(|(entity, hips)| (entity, hips.0));
    world.resource_scope(|world, mut proportions: Mut<BodyProportions>| {
        let Some((avatar, hips)) = avatar else {
            return;
        };
        if proportions
            .rig
            .as_ref()
            .is_some_and(|rig| rig.avatar != avatar || rig.hips != hips)
        {
            proportions.rig = None;
        }
        if proportions.rig.is_none() {
            if proportions.unsupported == Some(avatar) {
                return;
            }
            match capture_rig(world, avatar, hips) {
                Ok(rig) => {
                    debug!(
                        "Chibi rig captured: imported hips scale {:?}, head scale {:?}",
                        rig.original_hips.scale, rig.original_head.scale
                    );
                    proportions.rig = Some(rig);
                    proportions.unsupported = None;
                }
                Err(reason) => {
                    warn!("Chibi disabled for avatar {avatar:?}: {reason}");
                    proportions.unsupported = Some(avatar);
                    return;
                }
            }
        }
        let rig = proportions.rig.as_mut().unwrap();
        let previous = rig.applied;
        let applied = if safe { desired } else { previous };
        let Some(hips_pose) = world.get::<Transform>(rig.hips).copied() else {
            return;
        };
        let Some(head_pose) = world.get::<Transform>(rig.head).copied() else {
            return;
        };
        let Some(root_pose) = world.get::<Transform>(rig.avatar).copied() else {
            return;
        };
        let Some(knee_poses) = rig
            .legs
            .iter()
            .map(|leg| {
                world
                    .get::<Transform>(leg.knee)
                    .copied()
                    .filter(safe_transform)
            })
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        if !safe_transform(&hips_pose) || !safe_transform(&head_pose) || !safe_transform(&root_pose)
        {
            return;
        }
        // Read the evaluated pose, never multiply yesterday's Chibi result.
        let previous_root_offset_y = rig.applied_root_offset_y;
        rig.applied_root_offset_y = 0.0;
        if applied == AvatarBodyMode::Chibi {
            rig.pose_scales = Some((hips_pose.scale, head_pose.scale));
            world.get_mut::<Transform>(rig.hips).unwrap().scale =
                hips_pose.scale * CHIBI_BODY_SCALE;
            world.get_mut::<Transform>(rig.head).unwrap().scale =
                head_pose.scale * CHIBI_HEAD_SCALE;
            // Shorten only the imported femur contribution. Keep authored VRMA
            // translation deltas, rotations, shin/foot scales and upper-leg
            // skirt siblings intact; scaling the upper bone would affect them.
            for (leg, pose) in rig.legs.iter_mut().zip(knee_poses) {
                leg.raw_translation = Some(pose.translation);
                world.get_mut::<Transform>(leg.knee).unwrap().translation =
                    pose.translation + leg.rest_translation * (CHIBI_UPPER_LEG_FACTOR - 1.0);
            }
            // Cloud placement follows the evaluated pelvis rather than feet.
            // Applying a standing-leg correction to its lying pose would push
            // the face and mount below the transparent canvas on shorter rigs.
            if !cloud_owns_root {
                let base_offset = rig.rest_hips_to_feet_y * (1.0 - CHIBI_BODY_SCALE)
                    - rig.rest_leg_foot_delta_y * CHIBI_BODY_SCALE;
                rig.raw_root_translation = Some(root_pose.translation);
                rig.applied_root_offset_y = base_offset;
                world.get_mut::<Transform>(rig.avatar).unwrap().translation = root_pose.translation
                    + root_pose.rotation * (root_pose.scale * Vec3::Y * base_offset);
            }
        }
        rig.applied = applied;
        let root_owner_changed = rig.applied_root_offset_y != previous_root_offset_y;
        if applied != previous || root_owner_changed {
            springs::restore_spring_rotations(world, avatar);
            proportions.spring_reset = true;
        }
        if applied != previous {
            proportions.revision = proportions.revision.wrapping_add(1);
            world.write_message(PetFx {
                kind: PetFxKind::CloudPuff,
                anchor: PetFxAnchor::Head,
            });
            world.write_message(PetSfx(PetSfxKind::Happy));
            info!("Avatar body mode applied: {applied:?}");
        }
    });
}

fn reset_changed_springs(world: &mut World) {
    let avatar = {
        let mut proportions = world.resource_mut::<BodyProportions>();
        if !proportions.spring_reset {
            return;
        }
        proportions.spring_reset = false;
        proportions.rig.as_ref().map(|rig| rig.avatar)
    };
    if let Some(avatar) = avatar
        && !springs::reset_springs(world, avatar)
    {
        world.resource_mut::<BodyProportions>().spring_reset = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::{
        system::{RunSystemOnce, SystemState},
        world::CommandQueue,
    };

    fn rig(world: &mut World, scale: f32) -> (Entity, Entity, Entity) {
        let root = world
            .spawn((NeraAvatar, Initialized, Transform::default()))
            .id();
        let hips_pose = Transform::from_xyz(0.02, 0.9, -0.03).with_scale(Vec3::splat(scale));
        let hips = world
            .spawn((
                VrmBone::from("hips"),
                hips_pose,
                RestTransform(hips_pose),
                ChildOf(root),
            ))
            .id();
        let head_pose = Transform::from_xyz(0.0, 0.55, 0.0).with_scale(Vec3::new(0.95, 1.0, 1.02));
        let head = world
            .spawn((
                VrmBone::from("head"),
                head_pose,
                RestTransform(head_pose),
                ChildOf(hips),
            ))
            .id();
        world.spawn((
            VrmBone::from("leftFoot"),
            Transform::from_xyz(0.0, -0.8, 0.0),
            ChildOf(hips),
        ));
        world.entity_mut(root).insert(HipsBoneEntity(hips));
        (root, hips, head)
    }

    fn world() -> World {
        let mut world = World::new();
        world.init_resource::<AvatarBodyMode>();
        world.init_resource::<BodyProportions>();
        world.init_resource::<Messages<PetFx>>();
        world.init_resource::<Messages<PetSfx>>();
        world
    }

    fn frame(world: &mut World) {
        world.run_system_once(restore_proportions).unwrap();
        world.run_system_once(apply_proportions).unwrap();
    }

    #[test]
    fn repeated_toggle_restores_exact_custom_local_proportions_without_drift() {
        let mut world = world();
        let (avatar, hips, head) = rig(&mut world, 1.27);
        let original_root = Transform::from_xyz(0.4, -0.85, -0.2)
            .with_rotation(Quat::from_rotation_y(0.7))
            .with_scale(Vec3::new(1.2, 1.3, 1.1));
        *world.get_mut::<Transform>(avatar).unwrap() = original_root;
        let original_hips = *world.get::<Transform>(hips).unwrap();
        let original_head = *world.get::<Transform>(head).unwrap();
        for _ in 0..100 {
            *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
            frame(&mut world);
            let root = world.get::<Transform>(avatar).unwrap();
            let local_offset = world.resource::<BodyProportions>().root_offset_y();
            assert!(local_offset < 0.0);
            assert_eq!(
                root.translation,
                original_root.translation
                    + original_root.rotation * (original_root.scale * Vec3::Y * local_offset)
            );
            assert_eq!(root.rotation, original_root.rotation);
            assert_eq!(root.scale, original_root.scale);
            assert_eq!(
                world.get::<Transform>(hips).unwrap().scale,
                original_hips.scale * CHIBI_BODY_SCALE
            );
            assert_eq!(
                world.get::<Transform>(head).unwrap().scale,
                original_head.scale * CHIBI_HEAD_SCALE
            );
            frame(&mut world);
            *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Normal;
            frame(&mut world);
            assert_eq!(*world.get::<Transform>(hips).unwrap(), original_hips);
            assert_eq!(*world.get::<Transform>(head).unwrap(), original_head);
            assert_eq!(*world.get::<Transform>(avatar).unwrap(), original_root);
            assert_eq!(
                world
                    .resource::<BodyProportions>()
                    .rig
                    .as_ref()
                    .unwrap()
                    .original_hips,
                original_hips
            );
            assert_eq!(
                world
                    .resource::<BodyProportions>()
                    .rig
                    .as_ref()
                    .unwrap()
                    .original_head,
                original_head
            );
        }
    }

    #[test]
    fn animation_change_can_write_pose_and_scale_without_overwriting_layer() {
        let mut world = world();
        let (_, hips, head) = rig(&mut world, 1.0);
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
        frame(&mut world);
        world.run_system_once(restore_proportions).unwrap();
        let pose = Transform {
            translation: Vec3::new(0.2, 0.7, -0.1),
            rotation: Quat::from_rotation_x(0.6),
            scale: Vec3::splat(1.13),
        };
        *world.get_mut::<Transform>(hips).unwrap() = pose;
        world.run_system_once(apply_proportions).unwrap();
        let evaluated = *world.get::<Transform>(hips).unwrap();
        assert_eq!(evaluated.translation, pose.translation);
        assert_eq!(evaluated.rotation, pose.rotation);
        assert_eq!(evaluated.scale, pose.scale * CHIBI_BODY_SCALE);
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Normal;
        frame(&mut world);
        assert_eq!(*world.get::<Transform>(hips).unwrap(), pose);
        assert_eq!(
            world.get::<Transform>(head).unwrap().scale,
            Vec3::new(0.95, 1.0, 1.02)
        );
    }

    #[test]
    fn replacement_inherits_selection_and_never_old_rig_scales() {
        let mut world = world();
        let (old, _, _) = rig(&mut world, 1.0);
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
        frame(&mut world);
        world.despawn(old);
        let (_, hips, _) = rig(&mut world, 0.84);
        frame(&mut world);
        assert_eq!(
            world.get::<Transform>(hips).unwrap().scale,
            Vec3::splat(0.84) * CHIBI_BODY_SCALE
        );
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Normal;
        frame(&mut world);
        assert_eq!(
            world.get::<Transform>(hips).unwrap().scale,
            Vec3::splat(0.84)
        );
    }

    #[test]
    fn dragging_defers_transition_and_applies_selection_on_release() {
        let mut world = world();
        let (_, hips, _) = rig(&mut world, 1.0);
        world.init_resource::<DesktopPetInteraction>();
        world.resource_mut::<DesktopPetInteraction>().state = InteractionState::Dragging;
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
        frame(&mut world);
        assert_eq!(world.get::<Transform>(hips).unwrap().scale, Vec3::ONE);
        world.resource_mut::<DesktopPetInteraction>().state = InteractionState::None;
        frame(&mut world);
        assert_eq!(
            world.get::<Transform>(hips).unwrap().scale,
            Vec3::splat(CHIBI_BODY_SCALE)
        );
    }

    #[test]
    fn authored_animation_and_drag_keep_the_same_neutral_body_base() {
        use crate::animation::{
            GroundingPolicy, NeraAnimationState, TemporaryAction, grounding_policy,
        };
        let mut world = world();
        let (avatar, hips, _) = rig(&mut world, 1.0);
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
        frame(&mut world);
        let standing_root = *world.get::<Transform>(avatar).unwrap();
        assert!((standing_root.translation.y - (-0.8 * (1.0 - CHIBI_BODY_SCALE))).abs() < 0.00001);

        let clip = world.spawn_empty().id();
        let mut animation = NeraAnimationState::default();
        animation.request_action_for_test(
            TemporaryAction::LibraryAnimation(clip),
            std::time::Duration::from_secs(10),
        );
        assert_eq!(
            grounding_policy(
                animation.visual_state(),
                Some("animations/imported/custom_jump.vrma")
            ),
            GroundingPolicy::Authored
        );
        world.insert_resource(animation);
        world.run_system_once(restore_proportions).unwrap();
        // Authored hip travel remains untouched by the static base correction.
        world.get_mut::<Transform>(hips).unwrap().translation.y += 0.25;
        let authored_translation = world.get::<Transform>(hips).unwrap().translation;
        world.run_system_once(apply_proportions).unwrap();
        assert_eq!(*world.get::<Transform>(avatar).unwrap(), standing_root);
        assert_eq!(
            world.get::<Transform>(hips).unwrap().translation,
            authored_translation
        );

        world.init_resource::<DesktopPetInteraction>();
        world.resource_mut::<DesktopPetInteraction>().state = InteractionState::Dragging;
        frame(&mut world);
        assert_eq!(*world.get::<Transform>(avatar).unwrap(), standing_root);
        assert_eq!(
            world.get::<Transform>(hips).unwrap().translation,
            authored_translation
        );
    }

    #[test]
    fn toggling_updates_saved_mode_and_queues_deferred_persistence_once() {
        let mut world = world();
        world.init_resource::<NeraSettings>();
        world.init_resource::<Messages<ToggleBodyMode>>();
        world.write_message(ToggleBodyMode);
        let command_world = World::new();
        let mut queue = CommandQueue::default();
        let mut params = SystemState::<(
            MessageReader<ToggleBodyMode>,
            ResMut<AvatarBodyMode>,
            ResMut<NeraSettings>,
        )>::new(&mut world);
        let (toggles, mode, settings) = params.get_mut(&mut world).unwrap();
        select_body_mode(
            toggles,
            mode,
            settings,
            Commands::new(&mut queue, &command_world),
        );
        assert_eq!(*world.resource::<AvatarBodyMode>(), AvatarBodyMode::Chibi);
        assert_eq!(world.resource::<NeraSettings>().body_mode, "chibi");
        assert!(
            !queue.is_empty(),
            "changed mode must queue the existing deferred save command"
        );
        queue.apply(&mut world);
        let (toggles, mode, settings) = params.get_mut(&mut world).unwrap();
        select_body_mode(
            toggles,
            mode,
            settings,
            Commands::new(&mut queue, &command_world),
        );
        assert!(
            queue.is_empty(),
            "an unchanged preference must not restart the save timer"
        );
    }

    #[test]
    fn saved_chibi_startup_waits_for_humanoid_then_applies_exactly_once() {
        let mut app = App::new();
        app.insert_resource(NeraSettings {
            body_mode: "chibi".into(),
            ..default()
        })
        .add_message::<PetFx>()
        .add_message::<PetSfx>()
        .add_plugins(AvatarBodyModePlugin);
        app.update();
        assert_eq!(
            *app.world().resource::<AvatarBodyMode>(),
            AvatarBodyMode::Chibi
        );
        assert!(app.world().resource::<BodyProportions>().rig.is_none());
        let (avatar, hips, _) = rig(app.world_mut(), 1.0);
        app.update();
        assert_eq!(
            app.world().get::<Transform>(hips).unwrap().scale,
            Vec3::splat(CHIBI_BODY_SCALE)
        );
        let root = *app.world().get::<Transform>(avatar).unwrap();
        assert_eq!(app.world().resource::<BodyProportions>().revision(), 1);
        app.update();
        assert_eq!(*app.world().get::<Transform>(avatar).unwrap(), root);
        assert_eq!(app.world().resource::<BodyProportions>().revision(), 1);
    }

    #[test]
    fn malformed_avatar_fails_safely_and_saved_mode_waits_for_rig() {
        let mut world = world();
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
        frame(&mut world);
        assert!(world.resource::<BodyProportions>().rig.is_none());
        let (avatar, _, head) = rig(&mut world, 1.0);
        world.despawn(head);
        frame(&mut world);
        assert_eq!(
            world.resource::<BodyProportions>().unsupported,
            Some(avatar)
        );
        assert_eq!(AvatarBodyMode::from_saved("chibi"), AvatarBodyMode::Chibi);
        assert_eq!(AvatarBodyMode::from_saved("future"), AvatarBodyMode::Normal);
    }
}
