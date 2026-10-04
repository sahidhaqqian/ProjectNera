//! Geometry regressions for the optional thigh-segment overlay.
use super::*;
use bevy::ecs::system::RunSystemOnce;

struct LegRig {
    avatar: Entity,
    hips: Entity,
    head: Entity,
    thighs: [Entity; 2],
    knees: [Entity; 2],
    feet: [Entity; 2],
    toes: [Entity; 2],
    all: Vec<Entity>,
}

fn test_world() -> World {
    let mut world = World::new();
    world.init_resource::<AvatarBodyMode>();
    world.init_resource::<BodyProportions>();
    world.init_resource::<Messages<PetFx>>();
    world.init_resource::<Messages<PetSfx>>();
    world
}

fn local_bone(world: &mut World, parent: Entity, name: &str, pose: Transform) -> Entity {
    world
        .spawn((
            VrmBone::from(name),
            pose,
            RestTransform(pose),
            ChildOf(parent),
        ))
        .id()
}

/// Compose the actual local hierarchy, retaining any nonuniform-scale affine
/// terms. No production proportion metadata is used by these geometry checks.
fn global(world: &World, entity: Entity) -> GlobalTransform {
    let mut chain = Vec::new();
    let mut current = entity;
    loop {
        chain.push(*world.get::<Transform>(current).unwrap());
        let Some(parent) = world.get::<ChildOf>(current) else {
            break;
        };
        current = parent.parent();
    }
    chain
        .into_iter()
        .rev()
        .fold(GlobalTransform::IDENTITY, |global, local| {
            global.mul_transform(local)
        })
}

fn segment(world: &World, parent: Entity, child: Entity) -> f32 {
    global(world, parent)
        .translation()
        .distance(global(world, child).translation())
}

fn foot_y(world: &World, rig: &LegRig) -> f32 {
    rig.feet
        .iter()
        .map(|foot| global(world, *foot).translation().y)
        .min_by(f32::total_cmp)
        .unwrap()
}

fn fixture(world: &mut World, sign: f32, thigh_length: f32, helper: bool) -> LegRig {
    let root = Transform::from_xyz(0.4, -0.85, -0.2)
        .with_rotation(Quat::from_rotation_y(0.35))
        .with_scale(Vec3::new(1.2, 1.3, 1.1));
    let avatar = world.spawn((NeraAvatar, Initialized, root)).id();
    world.entity_mut(avatar).insert(RestTransform(root));
    let hips = local_bone(
        world,
        avatar,
        "hips",
        Transform::from_xyz(0.02, 0.94, -0.03)
            .with_rotation(Quat::from_rotation_x(0.12))
            .with_scale(Vec3::new(1.17, 0.91, 1.03)),
    );
    let head = local_bone(
        world,
        hips,
        "head",
        Transform::from_xyz(0.01, 0.54, 0.02).with_scale(Vec3::new(0.95, 1.02, 1.05)),
    );
    let mut all = vec![avatar, hips, head];
    let mut thighs = Vec::new();
    let mut knees = Vec::new();
    let mut feet = Vec::new();
    let mut toes = Vec::new();
    for (side, x) in [("left", 0.075), ("right", -0.075)] {
        // Positive local Y resembles Nera's flipped raw thigh orientation;
        // negative local Y resembles the imported Alicia hierarchy.
        let rotation = if sign > 0.0 {
            Quat::from_rotation_z(std::f32::consts::PI)
        } else {
            Quat::IDENTITY
        };
        let thigh = local_bone(
            world,
            hips,
            &format!("{side}UpperLeg"),
            Transform::from_xyz(x, -0.06, 0.005)
                .with_rotation(rotation)
                .with_scale(Vec3::new(1.04, 0.97, 1.02)),
        );
        all.push(thigh);
        let knee_parent = if helper && side == "left" {
            let pose =
                Transform::from_xyz(0.004, 0.0, 0.003).with_rotation(Quat::from_rotation_y(0.11));
            let entity = world
                .spawn((pose, RestTransform(pose), ChildOf(thigh)))
                .id();
            all.push(entity);
            entity
        } else {
            thigh
        };
        let knee = local_bone(
            world,
            knee_parent,
            &format!("{side}LowerLeg"),
            Transform::from_xyz(x * 0.08, sign * thigh_length, -0.002)
                .with_rotation(Quat::from_rotation_x(-0.06))
                .with_scale(Vec3::new(1.07, 0.96, 1.02)),
        );
        let foot = local_bone(
            world,
            knee,
            &format!("{side}Foot"),
            Transform::from_xyz(x * 0.1, sign * 0.41, 0.026)
                .with_rotation(Quat::from_rotation_y(-0.2))
                .with_scale(Vec3::new(1.03, 1.01, 0.99)),
        );
        let toe = local_bone(
            world,
            foot,
            &format!("{side}Toes"),
            Transform::from_xyz(0.02, sign * 0.04, 0.09),
        );
        // An unrelated thigh attachment must retain its imported local pose.
        let cloth_pose = Transform::from_xyz(0.015, 0.12, 0.07);
        let cloth = world
            .spawn((cloth_pose, RestTransform(cloth_pose), ChildOf(thigh)))
            .id();
        all.extend([knee, foot, toe, cloth]);
        thighs.push(thigh);
        knees.push(knee);
        feet.push(foot);
        toes.push(toe);
    }
    world.entity_mut(avatar).insert(HipsBoneEntity(hips));
    for &entity in &all {
        let rest = global(world, entity);
        world.entity_mut(entity).insert(RestGlobalTransform(rest));
    }
    LegRig {
        avatar,
        hips,
        head,
        thighs: thighs.try_into().unwrap(),
        knees: knees.try_into().unwrap(),
        feet: feet.try_into().unwrap(),
        toes: toes.try_into().unwrap(),
        all,
    }
}

fn frame(world: &mut World) {
    world.run_system_once(restore_proportions).unwrap();
    world.run_system_once(apply_proportions).unwrap();
}

fn near(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.00001,
        "actual {actual}, expected {expected}"
    );
}

fn near_vec(actual: Vec3, expected: Vec3) {
    assert!(
        actual.distance(expected) < 0.00001,
        "actual {actual:?}, expected {expected:?}"
    );
}

#[test]
fn upper_segment_shortens_without_changing_shin_or_foot_even_when_knee_bends() {
    for knee_angle in [0.0, 1.12] {
        let mut world = test_world();
        let rig = fixture(&mut world, -1.0, 0.363, false);
        for &knee in &rig.knees {
            world.get_mut::<Transform>(knee).unwrap().rotation = Quat::from_rotation_x(knee_angle);
        }
        let lengths: Vec<_> = (0..2)
            .map(|side| {
                (
                    segment(&world, rig.thighs[side], rig.knees[side]),
                    segment(&world, rig.knees[side], rig.feet[side]),
                    segment(&world, rig.feet[side], rig.toes[side]),
                )
            })
            .collect();
        let raw: Vec<_> = rig
            .knees
            .iter()
            .chain(&rig.feet)
            .chain(&rig.thighs)
            .map(|entity| (*entity, *world.get::<Transform>(*entity).unwrap()))
            .collect();
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
        frame(&mut world);
        for (side, &(upper, lower, foot)) in lengths.iter().enumerate() {
            near(
                segment(&world, rig.thighs[side], rig.knees[side]),
                upper * CHIBI_BODY_SCALE * CHIBI_UPPER_LEG_FACTOR,
            );
            near(
                segment(&world, rig.knees[side], rig.feet[side]),
                lower * CHIBI_BODY_SCALE,
            );
            near(
                segment(&world, rig.feet[side], rig.toes[side]),
                foot * CHIBI_BODY_SCALE,
            );
        }
        for (entity, pose) in raw {
            let current = world.get::<Transform>(entity).unwrap();
            assert_eq!(current.scale, pose.scale);
            assert_eq!(current.rotation, pose.rotation);
        }
    }
}

#[test]
fn one_hundred_toggles_restore_every_imported_local_and_custom_root_exactly() {
    let mut world = test_world();
    let rig = fixture(&mut world, 1.0, 0.3315, false);
    world.get_mut::<Transform>(rig.avatar).unwrap().rotation =
        Quat::from_euler(EulerRot::XYZ, 0.13, 0.31, -0.09);
    let original: Vec<_> = rig
        .all
        .iter()
        .map(|entity| (*entity, *world.get::<Transform>(*entity).unwrap()))
        .collect();
    for _ in 0..100 {
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
        frame(&mut world);
        let chibi: Vec<_> = rig
            .all
            .iter()
            .map(|entity| (*entity, *world.get::<Transform>(*entity).unwrap()))
            .collect();
        frame(&mut world);
        for &(entity, pose) in &chibi {
            assert_eq!(*world.get::<Transform>(entity).unwrap(), pose);
        }
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Normal;
        frame(&mut world);
        for &(entity, pose) in &original {
            assert_eq!(*world.get::<Transform>(entity).unwrap(), pose);
        }
    }
}

#[test]
fn new_animation_translation_delta_and_rotation_scale_writes_are_preserved() {
    let mut world = test_world();
    let rig = fixture(&mut world, -1.0, 0.363, false);
    let knee = rig.knees[0];
    let rest = world.get::<RestTransform>(knee).unwrap().0;
    *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
    frame(&mut world);
    for (delta, angle) in [
        (Vec3::new(0.015, -0.025, 0.03), 0.8),
        (Vec3::new(-0.01, 0.04, -0.02), -0.6),
    ] {
        world.run_system_once(restore_proportions).unwrap();
        let authored = Transform {
            translation: rest.translation + delta,
            rotation: Quat::from_rotation_x(angle),
            scale: Vec3::new(0.77, 1.16, 0.94),
        };
        *world.get_mut::<Transform>(knee).unwrap() = authored;
        world.get_mut::<Transform>(rig.hips).unwrap().scale = Vec3::new(1.15, 0.9, 1.02);
        world.run_system_once(apply_proportions).unwrap();
        let actual = *world.get::<Transform>(knee).unwrap();
        near_vec(
            actual.translation,
            rest.translation * CHIBI_UPPER_LEG_FACTOR + delta,
        );
        assert_eq!(actual.rotation, authored.rotation);
        assert_eq!(actual.scale, authored.scale);
        assert_eq!(
            world.get::<Transform>(rig.hips).unwrap().scale,
            Vec3::new(1.15, 0.9, 1.02) * CHIBI_BODY_SCALE
        );
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Normal;
        frame(&mut world);
        assert_eq!(*world.get::<Transform>(knee).unwrap(), authored);
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
    }
}

#[test]
fn opposite_imported_axes_work_and_helper_parent_safely_keeps_base_chibi() {
    for sign in [-1.0, 1.0] {
        let mut world = test_world();
        let rig = fixture(&mut world, sign, 0.35, false);
        let raw: Vec<_> = rig
            .knees
            .iter()
            .map(|entity| world.get::<Transform>(*entity).unwrap().translation)
            .collect();
        *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
        frame(&mut world);
        for (side, translation) in raw.into_iter().enumerate() {
            near_vec(
                world.get::<Transform>(rig.knees[side]).unwrap().translation,
                translation * CHIBI_UPPER_LEG_FACTOR,
            );
        }
    }
    let mut world = test_world();
    let rig = fixture(&mut world, -1.0, 0.35, true);
    let knees = rig
        .knees
        .map(|entity| *world.get::<Transform>(entity).unwrap());
    let hips_scale = world.get::<Transform>(rig.hips).unwrap().scale;
    let head_scale = world.get::<Transform>(rig.head).unwrap().scale;
    *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
    frame(&mut world);
    for (side, raw) in knees.iter().enumerate() {
        assert_eq!(*world.get::<Transform>(rig.knees[side]).unwrap(), *raw);
    }
    assert_eq!(
        world.resource::<BodyProportions>().body_scale(),
        CHIBI_BODY_SCALE
    );
    assert_eq!(
        world.get::<Transform>(rig.hips).unwrap().scale,
        hips_scale * CHIBI_BODY_SCALE
    );
    assert_eq!(
        world.get::<Transform>(rig.head).unwrap().scale,
        head_scale * CHIBI_HEAD_SCALE
    );
}

#[test]
fn replacing_character_uses_new_segment_and_rest_ground_geometry() {
    let mut world = test_world();
    let old = fixture(&mut world, -1.0, 0.28, false);
    *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
    frame(&mut world);
    world.despawn(old.avatar);
    let new = fixture(&mut world, 1.0, 0.46, false);
    let original: Vec<_> = new
        .all
        .iter()
        .map(|entity| (*entity, *world.get::<Transform>(*entity).unwrap()))
        .collect();
    let original_foot_y = foot_y(&world, &new);
    let rest_knees = new
        .knees
        .map(|entity| world.get::<Transform>(entity).unwrap().translation);
    frame(&mut world);
    for (side, rest) in rest_knees.into_iter().enumerate() {
        near_vec(
            world.get::<Transform>(new.knees[side]).unwrap().translation,
            rest * CHIBI_UPPER_LEG_FACTOR,
        );
    }
    near(foot_y(&world, &new), original_foot_y);
    *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Normal;
    frame(&mut world);
    for (entity, pose) in original {
        assert_eq!(*world.get::<Transform>(entity).unwrap(), pose);
    }
}

#[test]
fn shortened_rest_hierarchy_keeps_neutral_foot_height_with_or_without_root_rest_global() {
    for omit_root_rest_global in [false, true] {
        for sign in [-1.0, 1.0] {
            let mut world = test_world();
            let rig = fixture(&mut world, sign, 0.37, false);
            if omit_root_rest_global {
                world.entity_mut(rig.avatar).remove::<RestGlobalTransform>();
            }
            let original_foot_y = foot_y(&world, &rig);
            let raw_root = *world.get::<Transform>(rig.avatar).unwrap();
            *world.resource_mut::<AvatarBodyMode>() = AvatarBodyMode::Chibi;
            frame(&mut world);
            near(foot_y(&world, &rig), original_foot_y);
            let corrected_root = world.get::<Transform>(rig.avatar).unwrap();
            assert_eq!(corrected_root.scale, raw_root.scale);
            assert_eq!(corrected_root.rotation, raw_root.rotation);
            assert!(corrected_root.translation.y < raw_root.translation.y);
        }
    }
}
