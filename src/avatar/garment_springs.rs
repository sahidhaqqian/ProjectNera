//! A zero-gravity skirt compatibility fallback, using VRM's existing solver.
//! Source assets, authored nonzero gravity, and other spring chains stay intact.
use bevy::prelude::*;
use bevy_vrm1::prelude::{HipsBoneEntity, SpringJointProps};

use super::NeraAvatar;

fn fallback_power(name: &str, props: &SpringJointProps) -> Option<f32> {
    let is_skirt = name
        .as_bytes()
        .windows(5)
        .any(|part| part.eq_ignore_ascii_case(b"skirt"));
    if !is_skirt
        || props.gravity_power != 0.0
        || !props.stiffness.is_finite()
        || props.stiffness < 0.0
        || !props
            .gravity_dir
            .try_normalize()
            .is_some_and(|dir| dir.dot(Vec3::NEG_Y) > 0.98)
    {
        return None;
    }
    // Just exceed the authored restoring force so an inverted garment can
    // fall down instead of retaining an upside-down rest direction. Both
    // forces use the same time factor in bevy_vrm1's Verlet spring solver.
    Some((props.stiffness * 1.25).clamp(0.1, 2.0))
}

pub(crate) fn apply_gravity(
    avatars: Query<&HipsBoneEntity, With<NeraAvatar>>,
    parents: Query<&ChildOf>,
    mut joints: Query<(Entity, &Name, &mut SpringJointProps)>,
) {
    let Ok(hips) = avatars.single() else { return };
    let mut count = 0;
    for (entity, name, mut props) in &mut joints {
        let Some(power) = fallback_power(name.as_str(), &props) else {
            continue;
        };
        // VRMA source/preview rigs may share names with the visible skeleton.
        // Only adjust descendants of the active avatar's humanoid hips.
        let mut ancestor = entity;
        let mut live = false;
        for _ in 0..256 {
            if ancestor == hips.0 {
                live = true;
                break;
            }
            let Ok(parent) = parents.get(ancestor) else {
                break;
            };
            ancestor = parent.parent();
        }
        if live {
            props.gravity_power = power;
            count += 1;
        }
    }
    if count > 0 {
        info!(
            "Skirt spring gravity fallback applied to {count} zero-gravity joints; authored stiffness/colliders retained"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn zero_gravity() -> SpringJointProps {
        SpringJointProps {
            gravity_dir: Vec3::NEG_Y,
            stiffness: 0.5,
            drag_force: 0.6,
            ..default()
        }
    }

    #[test]
    fn only_zero_gravity_skirts_receive_a_restoring_force_relative_fallback() {
        let p = zero_gravity();
        assert_eq!(fallback_power("J_Sec_L_SkirtFront1_01", &p), Some(0.625));
        assert!(fallback_power("J_Sec_Hair1_01", &p).is_none());
        assert!(fallback_power("Cape", &p).is_none());
        assert!(
            fallback_power(
                "Skirt",
                &SpringJointProps {
                    gravity_power: 0.3,
                    ..p
                }
            )
            .is_none()
        );
        assert!(
            fallback_power(
                "Skirt",
                &SpringJointProps {
                    stiffness: f32::NAN,
                    ..p
                }
            )
            .is_none()
        );
        assert!(
            fallback_power(
                "Skirt",
                &SpringJointProps {
                    gravity_dir: Vec3::X,
                    ..p
                }
            )
            .is_none()
        );
    }

    #[test]
    fn visible_rig_only_and_repeated_updates_and_replacement_are_safe() {
        let mut world = World::new();
        let hips = world.spawn_empty().id();
        let avatar = world.spawn((NeraAvatar, HipsBoneEntity(hips))).id();
        let garment = world
            .spawn((Name::new("SkirtFront"), zero_gravity(), ChildOf(hips)))
            .id();
        let hair = world
            .spawn((Name::new("Hair"), zero_gravity(), ChildOf(hips)))
            .id();
        let source_hips = world.spawn(ChildOf(avatar)).id();
        let preview = world
            .spawn((
                Name::new("SkirtFront"),
                zero_gravity(),
                ChildOf(source_hips),
            ))
            .id();
        for _ in 0..4 {
            world.run_system_once(apply_gravity).unwrap();
        }
        let adjusted = *world.get::<SpringJointProps>(garment).unwrap();
        assert_eq!(adjusted.gravity_power, 0.625);
        assert_eq!(adjusted.stiffness, 0.5);
        assert_eq!(adjusted.drag_force, 0.6);
        assert_eq!(
            world.get::<SpringJointProps>(hair).unwrap().gravity_power,
            0.0
        );
        assert_eq!(
            world
                .get::<SpringJointProps>(preview)
                .unwrap()
                .gravity_power,
            0.0
        );
        world.despawn(avatar);
        let next_hips = world.spawn_empty().id();
        world.spawn((NeraAvatar, HipsBoneEntity(next_hips)));
        let next = world
            .spawn((Name::new("Skirt"), zero_gravity(), ChildOf(next_hips)))
            .id();
        world.run_system_once(apply_gravity).unwrap();
        assert_eq!(
            world.get::<SpringJointProps>(next).unwrap().gravity_power,
            0.625
        );
    }
}
