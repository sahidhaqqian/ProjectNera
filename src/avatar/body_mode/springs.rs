//! Transition-only bridge to bevy_vrm1 0.9.2's registered spring state.
//! The crate exposes spring chains, but keeps their inertial state private.
//! Use its existing reflection registration instead of duplicating the solver.
use bevy::{
    ecs::reflect::ReflectComponent,
    prelude::*,
    reflect::{ReflectMut, ReflectRef, structs::Struct},
};
use bevy_vrm1::prelude::{HipsBoneEntity, RestTransform, SpringRoot};
use std::any::TypeId;

const SPRING_STATE_TYPE: &str = "bevy_vrm1::vrm::spring_bone::SpringJointState";

#[derive(Component, Clone, Copy)]
struct SpringProportionBaseline {
    length: f32,
    rotation: Quat,
    matrix: Mat4,
    axis: Vec3,
}

fn adapter(world: &World) -> Option<ReflectComponent> {
    let registry = world.get_resource::<AppTypeRegistry>()?.read();
    let registration = registry.get_with_type_path(SPRING_STATE_TYPE)?;
    let info = registration.type_info().as_struct().ok()?;
    let expected = [
        ("prev_tail", TypeId::of::<Vec3>()),
        ("current_tail", TypeId::of::<Vec3>()),
        ("bone_axis", TypeId::of::<Vec3>()),
        ("bone_length", TypeId::of::<f32>()),
        ("initial_local_matrix", TypeId::of::<Mat4>()),
        ("initial_local_rotation", TypeId::of::<Quat>()),
    ];
    if !expected
        .iter()
        .all(|(name, ty)| info.field(name).is_some_and(|field| field.type_id() == *ty))
    {
        return None;
    }
    registration.data::<ReflectComponent>().cloned()
}

/// Preflight before changing the rig. Missing joint states are permitted here:
/// the dependency creates them one update after it creates a SpringRoot.
pub(super) fn adapter_available(world: &World, avatar: Entity) -> bool {
    let chains = chains(world, avatar);
    if chains.iter().all(|chain| chain.joints.len() < 2) {
        return true;
    }
    adapter(world).is_some()
        && chains.iter().all(|chain| {
            chain.joints.windows(2).all(|pair| {
                let head = world.get::<Transform>(pair[0]);
                let tail = world.get::<Transform>(pair[1]);
                head.is_some_and(|head| head.to_matrix().is_finite())
                    && tail.is_some_and(|tail| {
                        tail.to_matrix().is_finite() && tail.translation.try_normalize().is_some()
                    })
            })
        })
}

fn schema_valid(state: &dyn Struct) -> bool {
    state
        .field("prev_tail")
        .is_some_and(|value| value.try_downcast_ref::<Vec3>().is_some())
        && state
            .field("current_tail")
            .is_some_and(|value| value.try_downcast_ref::<Vec3>().is_some())
        && state
            .field("bone_axis")
            .is_some_and(|value| value.try_downcast_ref::<Vec3>().is_some())
        && state
            .field("bone_length")
            .is_some_and(|value| value.try_downcast_ref::<f32>().is_some())
        && state
            .field("initial_local_matrix")
            .is_some_and(|value| value.try_downcast_ref::<Mat4>().is_some())
        && state
            .field("initial_local_rotation")
            .is_some_and(|value| value.try_downcast_ref::<Quat>().is_some())
}

fn chains(world: &World, avatar: Entity) -> Vec<SpringRoot> {
    // A VRM also owns its VRMA source rigs; only walk the rendered skeleton.
    let root = world
        .get::<HipsBoneEntity>(avatar)
        .map_or(avatar, |hips| hips.0);
    let mut stack = vec![root];
    let mut result = Vec::new();
    while let Some(entity) = stack.pop() {
        if let Some(spring) = world.get::<SpringRoot>(entity) {
            result.push(spring.clone());
        }
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
    }
    result
}

/// Clear the last simulated rotations before the new proportions propagate.
/// This runs once per body-mode or character transition, after animation has
/// evaluated and before VRM's first manual transform propagation.
pub(super) fn restore_spring_rotations(world: &mut World, avatar: Entity) {
    let adapter = adapter(world);
    for chain in chains(world, avatar) {
        // The final joint is the chain's tail; it has no simulated state.
        for &entity in chain
            .joints
            .iter()
            .take(chain.joints.len().saturating_sub(1))
        {
            let rotation = world
                .get::<SpringProportionBaseline>(entity)
                .map(|base| base.rotation)
                .or_else(|| world.get::<RestTransform>(entity).map(|rest| rest.rotation))
                .or_else(|| {
                    let reflected = adapter.as_ref()?.reflect(world.get_entity(entity).ok()?)?;
                    let ReflectRef::Struct(state) = reflected.reflect_ref() else {
                        return None;
                    };
                    state
                        .field("initial_local_rotation")?
                        .try_downcast_ref::<Quat>()
                        .copied()
                });
            let Some(rotation) = rotation.filter(|rotation| rotation.is_finite()) else {
                continue;
            };
            if let Some(mut transform) = world.get_mut::<Transform>(entity) {
                transform.rotation = rotation;
            }
        }
    }
}

/// Rebuild inertia and segment lengths from the new propagated proportions.
/// Schedule after `PropagateAfterExpressions` and before `SpringBone`.
/// `false` means a loaded chain could not be reset safely; the caller may retry
/// while spring initialization is still pending or disable the transform.
pub(super) fn reset_springs(world: &mut World, avatar: Entity) -> bool {
    let chains = chains(world, avatar);
    if chains.iter().all(|chain| chain.joints.len() < 2) {
        return true;
    }
    let Some(adapter) = adapter(world) else {
        warn!(
            "Chibi spring reset unavailable: bevy_vrm1 spring state reflection is not registered"
        );
        return false;
    };

    // The baseline preserves the dependency's exact initial local solver data
    // on return to Normal. Only Chibi uses measured world segment lengths.
    let normal = world
        .get_resource::<super::BodyProportions>()
        .is_some_and(|body| body.body_scale() == 1.0);
    let mut complete = true;
    for chain in chains {
        for pair in chain.joints.windows(2) {
            let head = pair[0];
            let tail = pair[1];
            let (Some(head_transform), Some(head_global), Some(tail_transform), Some(tail_global)) = (
                world.get::<Transform>(head).copied(),
                world.get::<GlobalTransform>(head).copied(),
                world.get::<Transform>(tail).copied(),
                world.get::<GlobalTransform>(tail).copied(),
            ) else {
                complete = false;
                continue;
            };
            let world_tail = tail_global.translation();
            let mut length = head_global.translation().distance(world_tail);
            let Some(mut axis) = tail_transform.translation.try_normalize() else {
                complete = false;
                continue;
            };
            let tail_position = match chain.center_node.0 {
                Some(center) => world.get::<GlobalTransform>(center).and_then(|center| {
                    let inverse = center.to_matrix().inverse();
                    inverse
                        .is_finite()
                        .then(|| inverse.transform_point3(world_tail))
                }),
                None => Some(world_tail),
            };
            let Some(tail_position) = tail_position.filter(|position| position.is_finite()) else {
                complete = false;
                continue;
            };
            if !length.is_finite()
                || length <= f32::EPSILON
                || !head_transform.to_matrix().is_finite()
                || !head_transform.rotation.is_finite()
            {
                complete = false;
                continue;
            }
            let existing_baseline = world.get::<SpringProportionBaseline>(head).copied();
            let Ok(mut entity) = world.get_entity_mut(head) else {
                complete = false;
                continue;
            };
            let Some(mut reflected) = adapter.reflect_mut(&mut entity) else {
                // SpringRoot can precede its private state by one update.
                complete = false;
                continue;
            };
            let ReflectMut::Struct(state) = reflected.reflect_mut() else {
                complete = false;
                continue;
            };
            // Validate the pinned crate's schema before mutating any field.
            if !schema_valid(state) {
                warn!("Chibi spring reset unavailable: unexpected bevy_vrm1 spring state schema");
                complete = false;
                continue;
            }
            let baseline = existing_baseline.unwrap_or_else(|| SpringProportionBaseline {
                length: *state
                    .field("bone_length")
                    .unwrap()
                    .try_downcast_ref::<f32>()
                    .unwrap(),
                axis: *state
                    .field("bone_axis")
                    .unwrap()
                    .try_downcast_ref::<Vec3>()
                    .unwrap(),
                matrix: *state
                    .field("initial_local_matrix")
                    .unwrap()
                    .try_downcast_ref::<Mat4>()
                    .unwrap(),
                rotation: *state
                    .field("initial_local_rotation")
                    .unwrap()
                    .try_downcast_ref::<Quat>()
                    .unwrap(),
            });
            let (matrix, rotation) = if normal {
                length = baseline.length;
                axis = baseline.axis;
                (baseline.matrix, baseline.rotation)
            } else {
                (head_transform.to_matrix(), head_transform.rotation)
            };
            let updates: [(&str, &dyn bevy::reflect::PartialReflect); 6] = [
                ("prev_tail", &tail_position),
                ("current_tail", &tail_position),
                ("bone_axis", &axis),
                ("bone_length", &length),
                ("initial_local_matrix", &matrix),
                ("initial_local_rotation", &rotation),
            ];
            for (name, value) in updates {
                if state
                    .field_mut(name)
                    .is_none_or(|field| field.try_apply(value).is_err())
                {
                    complete = false;
                }
            }
            drop(reflected);
            if existing_baseline.is_none() {
                entity.insert(baseline);
            }
        }
    }
    complete
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_vrm1::vrm::spring_bone::{SpringCenterNode, SpringColliders, SpringJoints};

    // The real state is private but registered under this exact reflection path.
    #[derive(Component, Reflect, Default)]
    #[reflect(Component)]
    #[type_path = "bevy_vrm1::vrm::spring_bone"]
    struct SpringJointState {
        prev_tail: Vec3,
        current_tail: Vec3,
        bone_axis: Vec3,
        bone_length: f32,
        initial_local_matrix: Mat4,
        initial_local_rotation: Quat,
    }

    fn rig(with_registry: bool, with_center: bool) -> (World, Entity, Entity, Entity) {
        let mut world = World::new();
        if with_registry {
            let registry = AppTypeRegistry::default();
            registry.write().register::<SpringJointState>();
            world.insert_resource(registry);
        }
        let avatar = world.spawn_empty().id();
        let head = world
            .spawn((
                Transform::from_rotation(Quat::from_rotation_z(0.7)),
                RestTransform(Transform::IDENTITY),
                GlobalTransform::from_translation(Vec3::new(0.0, 2.0, 0.0)),
                SpringJointState {
                    prev_tail: Vec3::splat(-5.0),
                    current_tail: Vec3::splat(5.0),
                    bone_length: 2.0,
                    ..default()
                },
                ChildOf(avatar),
            ))
            .id();
        let tail = world
            .spawn((
                Transform::from_xyz(0.0, 1.0, 0.0),
                GlobalTransform::from_translation(Vec3::new(0.0, 2.62, 0.0)),
                ChildOf(head),
            ))
            .id();
        let center = with_center.then(|| {
            world
                .spawn(GlobalTransform::from_translation(Vec3::new(0.0, 2.0, 0.0)))
                .id()
        });
        world.entity_mut(head).insert(SpringRoot {
            joints: SpringJoints(vec![head, tail]),
            colliders: SpringColliders::default(),
            center_node: SpringCenterNode(center),
        });
        world.entity_mut(avatar).insert(HipsBoneEntity(head));
        (world, avatar, head, tail)
    }

    #[test]
    fn transition_resets_inertia_and_measures_scaled_world_segment() {
        let (mut world, avatar, head, _) = rig(true, false);
        restore_spring_rotations(&mut world, avatar);
        assert_eq!(
            world.get::<Transform>(head).unwrap().rotation,
            Quat::IDENTITY
        );
        assert!(reset_springs(&mut world, avatar));
        let state = world.get::<SpringJointState>(head).unwrap();
        assert_eq!(state.prev_tail, state.current_tail);
        assert_eq!(state.current_tail, Vec3::new(0.0, 2.62, 0.0));
        assert!((state.bone_length - 0.62).abs() < 0.00001);
        assert_eq!(state.bone_axis, Vec3::Y);
        assert_eq!(state.initial_local_rotation, Quat::IDENTITY);
    }

    #[test]
    fn center_space_reset_and_return_to_normal_do_not_accumulate() {
        let (mut world, avatar, head, tail) = rig(true, true);
        assert!(reset_springs(&mut world, avatar));
        assert!(
            (world.get::<SpringJointState>(head).unwrap().current_tail.y - 0.62).abs() < 0.00001
        );
        for _ in 0..8 {
            *world.get_mut::<GlobalTransform>(tail).unwrap() =
                GlobalTransform::from_translation(Vec3::new(0.0, 3.0, 0.0));
            assert!(reset_springs(&mut world, avatar));
            assert_eq!(
                world.get::<SpringJointState>(head).unwrap().bone_length,
                1.0
            );
            *world.get_mut::<GlobalTransform>(tail).unwrap() =
                GlobalTransform::from_translation(Vec3::new(0.0, 2.62, 0.0));
            assert!(reset_springs(&mut world, avatar));
            assert!(
                (world.get::<SpringJointState>(head).unwrap().bone_length - 0.62).abs() < 0.00001
            );
        }
    }

    #[test]
    fn absent_reflection_and_invalid_chain_geometry_fail_safely() {
        let (mut world, avatar, head, tail) = rig(false, false);
        assert!(!adapter_available(&world, avatar));
        assert!(!reset_springs(&mut world, avatar));
        assert_eq!(
            world.get::<SpringJointState>(head).unwrap().bone_length,
            2.0
        );
        let registry = AppTypeRegistry::default();
        registry.write().register::<SpringJointState>();
        world.insert_resource(registry);
        assert!(adapter_available(&world, avatar));
        world.get_mut::<Transform>(tail).unwrap().translation = Vec3::ZERO;
        assert!(!adapter_available(&world, avatar));
        assert!(!reset_springs(&mut world, avatar));
        assert_eq!(
            world.get::<SpringJointState>(head).unwrap().bone_length,
            2.0
        );
    }

    #[test]
    fn avatar_without_spring_chains_needs_no_private_adapter() {
        let mut world = World::new();
        let avatar = world.spawn_empty().id();
        assert!(reset_springs(&mut world, avatar));
    }

    #[test]
    fn missing_rest_transform_uses_original_private_rotation() {
        let (mut world, avatar, head, _) = rig(true, false);
        world.entity_mut(head).remove::<RestTransform>();
        world
            .get_mut::<SpringJointState>(head)
            .unwrap()
            .initial_local_rotation = Quat::from_rotation_x(0.2);
        restore_spring_rotations(&mut world, avatar);
        assert_eq!(
            world.get::<Transform>(head).unwrap().rotation,
            Quat::from_rotation_x(0.2)
        );
    }

    #[test]
    fn normal_restores_original_solver_length_and_matrix_exactly() {
        let (mut world, avatar, head, _) = rig(true, false);
        let original_matrix = world
            .get::<SpringJointState>(head)
            .unwrap()
            .initial_local_matrix;
        assert!(reset_springs(&mut world, avatar));
        assert!((world.get::<SpringJointState>(head).unwrap().bone_length - 0.62).abs() < 0.00001);
        world.init_resource::<super::super::BodyProportions>();
        assert!(reset_springs(&mut world, avatar));
        let state = world.get::<SpringJointState>(head).unwrap();
        assert_eq!(state.bone_length, 2.0);
        assert_eq!(state.initial_local_matrix, original_matrix);
        assert_eq!(state.prev_tail, state.current_tail);
    }

    #[test]
    fn pinned_vrm_dependency_registers_the_expected_private_schema() {
        let mut app = App::new();
        app.add_plugins(bevy_vrm1::vrm::spring_bone::VrmSpringBonePlugin);
        assert!(adapter(app.world()).is_some());
    }
}
