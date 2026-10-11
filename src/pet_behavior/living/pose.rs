//! A bounded two-bone reach on the evaluated humanoid pose. Imported axes,
//! translations and proportions are never replaced.
use super::*;
use crate::avatar::body_mode::BodyProportionSet;
use bevy_vrm1::prelude::VrmSystemSets;

#[derive(Resource, Default)]
struct ReachOverlay(Vec<(Entity, Quat)>);

pub(super) fn install(app: &mut App) {
    app.init_resource::<ReachOverlay>()
        .add_systems(PreUpdate, clear)
        .add_systems(
            PostUpdate,
            apply
                .after(BodyProportionSet::Apply)
                .before(VrmSystemSets::Constraints),
        );
}

pub(super) fn clear(world: &mut World) {
    let Some(mut overlay) = world.get_resource_mut::<ReachOverlay>() else {
        return;
    };
    let offsets = std::mem::take(&mut overlay.0);
    for (entity, original) in offsets {
        if let Some(mut pose) = world.get_mut::<Transform>(entity) {
            pose.rotation = original;
        }
    }
}

fn matrix(world: &World, entity: Entity) -> Option<Mat4> {
    let mut chain = vec![];
    let mut node = entity;
    for _ in 0..96 {
        chain.push(world.get::<Transform>(node)?.to_matrix());
        if let Some(parent) = world.get::<ChildOf>(node) {
            node = parent.parent();
        } else {
            return Some(
                chain
                    .iter()
                    .rev()
                    .fold(Mat4::IDENTITY, |m, local| m * local),
            );
        }
    }
    None
}

fn ancestor(world: &World, entity: Entity, expected: Entity) -> bool {
    let mut node = entity;
    for _ in 0..96 {
        if node == expected {
            return true;
        }
        let Some(parent) = world.get::<ChildOf>(node) else {
            return false;
        };
        node = parent.parent();
    }
    false
}

fn solve_elbow(shoulder: Vec3, elbow: Vec3, hand: Vec3, target: Vec3) -> Option<(Vec3, Vec3)> {
    let upper = shoulder.distance(elbow);
    let lower = elbow.distance(hand);
    if !upper.is_finite()
        || !lower.is_finite()
        || upper < 0.005
        || lower < 0.005
        || !target.is_finite()
    {
        return None;
    }
    let axis = (target - shoulder).try_normalize()?;
    let distance = shoulder.distance(target).clamp(
        (upper - lower).abs() + (upper + lower) * 0.02,
        (upper + lower) * 0.94,
    );
    let along = (upper * upper - lower * lower + distance * distance) / (2.0 * distance);
    let bend = (upper * upper - along * along).max(0.0).sqrt();
    let pole = Vec3::NEG_Y + Vec3::Z * 0.45;
    let perpendicular = (pole - axis * pole.dot(axis))
        .try_normalize()
        .or_else(|| axis.any_orthonormal_vector().try_normalize())?;
    Some((
        shoulder + axis * along + perpendicular * bend,
        shoulder + axis * distance,
    ))
}

fn rotate_toward(
    world: &mut World,
    entity: Entity,
    current: Vec3,
    desired: Vec3,
    amount: f32,
) -> bool {
    let (Some(from), Some(to), Some(parent)) = (
        current.try_normalize(),
        desired.try_normalize(),
        world.get::<ChildOf>(entity).map(|p| p.parent()),
    ) else {
        return false;
    };
    let Some(parent_world) = matrix(world, parent) else {
        return false;
    };
    let (_, parent_rotation, _) = parent_world.to_scale_rotation_translation();
    if !parent_rotation.is_finite() {
        return false;
    }
    let delta = Quat::IDENTITY.slerp(Quat::from_rotation_arc(from, to), amount.clamp(0.0, 1.0));
    let Some(mut transform) = world.get_mut::<Transform>(entity) else {
        return false;
    };
    let original = transform.rotation;
    transform.rotation =
        (parent_rotation.inverse() * delta * parent_rotation * original).normalize();
    world
        .resource_mut::<ReachOverlay>()
        .0
        .push((entity, original));
    true
}

fn apply(world: &mut World) {
    let Some(pose) = world.resource::<PetBehavior>().cursor_pose() else {
        return;
    };
    if pose.amount <= 0.0
        || !matches!(world.resource::<NeraAnimationState>().visual_state(), VisualState::IdleMicro(action) if is_living(action))
    {
        return;
    }
    if !ancestor(world, pose.lower, pose.upper) || !ancestor(world, pose.hand, pose.lower) {
        return;
    }
    let positions = |world: &World| {
        Some((
            matrix(world, pose.upper)?.transform_point3(Vec3::ZERO),
            matrix(world, pose.lower)?.transform_point3(Vec3::ZERO),
            matrix(world, pose.hand)?.transform_point3(Vec3::ZERO),
        ))
    };
    let Some((shoulder, elbow, hand)) = positions(world) else {
        return;
    };
    let length = shoulder.distance(elbow) + elbow.distance(hand);
    let target = {
        let mut cameras =
            world.query_filtered::<(&Camera, &GlobalTransform), With<NeraMainCamera>>();
        let Some((camera, view)) = cameras.iter(world).next() else {
            return;
        };
        crate::avatar::desktop_point_to_world(
            pose.target,
            world.resource::<DesktopEnvironment>(),
            camera,
            view,
            shoulder.z + length * 0.45,
        )
    };
    let Some(target) = target else {
        return;
    };
    let Some((desired_elbow, desired_hand)) = solve_elbow(shoulder, elbow, hand, target) else {
        return;
    };
    if !rotate_toward(
        world,
        pose.upper,
        elbow - shoulder,
        desired_elbow - shoulder,
        pose.amount,
    ) {
        return;
    }
    let Some((_, elbow, hand)) = positions(world) else {
        return;
    };
    rotate_toward(
        world,
        pose.lower,
        hand - elbow,
        desired_hand - elbow,
        pose.amount,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn solver_preserves_segment_lengths_and_handles_custom_scale_and_far_targets() {
        for scale in [0.62, 1.0, 1.8] {
            let shoulder = Vec3::new(0.0, 1.0, 0.0);
            let elbow = shoulder + Vec3::X * 0.3 * scale;
            let hand = elbow + Vec3::X * 0.24 * scale;
            for target in [Vec3::new(0.2, 1.1, 0.3), Vec3::splat(100.0)] {
                let (e, h) = solve_elbow(shoulder, elbow, hand, target).unwrap();
                assert!((shoulder.distance(e) - 0.3 * scale).abs() < 0.00001);
                assert!((e.distance(h) - 0.24 * scale).abs() < 0.00001);
                assert!(h.distance(shoulder) < 0.54 * scale);
            }
        }
        assert!(solve_elbow(Vec3::ZERO, Vec3::ZERO, Vec3::X, Vec3::Y).is_none());
    }
    #[test]
    fn reach_restores_exact_pose_over_repeated_frames() {
        let mut world = World::new();
        world.init_resource::<ReachOverlay>();
        let parent = world
            .spawn(Transform::from_rotation(Quat::from_rotation_y(0.3)))
            .id();
        let original = Transform::from_rotation(Quat::from_rotation_z(0.2));
        let bone = world.spawn((original, ChildOf(parent))).id();
        for _ in 0..100 {
            assert!(rotate_toward(&mut world, bone, Vec3::X, Vec3::Z, 0.75));
            assert_ne!(
                world.get::<Transform>(bone).unwrap().rotation,
                original.rotation
            );
            clear(&mut world);
            assert_eq!(*world.get::<Transform>(bone).unwrap(), original);
        }
    }
}
