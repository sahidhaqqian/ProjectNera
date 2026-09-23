//! Semantic body hit testing and the bone colliders that implement it.
use super::BodyPart;
use bevy::prelude::*;
use bevy_vrm1::prelude::*;

#[derive(Component)]
pub(crate) struct NeraCollider {
    body_part: BodyPart,
    shape: NeraColliderShape,
}

#[derive(Clone, Copy)]
enum NeraColliderShape {
    Sphere { radius: f32 },
    Capsule { radius: f32, end_bone: &'static str },
}
fn ray_sphere_distance(ray_origin: Vec3, ray_dir: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let oc = ray_origin - center;
    let b = oc.dot(ray_dir);
    let c = oc.length_squared() - radius * radius;
    let discriminant = b * b - c;

    if discriminant < 0.0 {
        return None;
    }

    let sqrt_d = discriminant.sqrt();
    let near = -b - sqrt_d;
    let far = -b + sqrt_d;

    if near >= 0.0 {
        Some(near)
    } else if far >= 0.0 {
        Some(far)
    } else {
        None
    }
}

fn ray_capsule_distance(
    ray_origin: Vec3,
    ray_dir: Vec3,
    pa: Vec3,
    pb: Vec3,
    radius: f32,
) -> Option<f32> {
    let ba = pb - pa;
    let oa = ray_origin - pa;

    let baba = ba.dot(ba);
    if baba < 1e-8 {
        return ray_sphere_distance(ray_origin, ray_dir, pa, radius);
    }

    let bard = ba.dot(ray_dir);
    let baoa = ba.dot(oa);
    let rdoa = ray_dir.dot(oa);
    let oaoa = oa.dot(oa);

    let a = baba - bard * bard;
    let b = baba * rdoa - baoa * bard;
    let c = baba * oaoa - baoa * baoa - radius * radius * baba;

    let mut nearest: Option<f32> = None;

    // Infinite-cylinder intersection clipped to the bone segment.
    if a.abs() >= 1e-6 {
        let h = b * b - a * c;

        if h >= 0.0 {
            let t = (-b - h.sqrt()) / a;
            let y = baoa + t * bard;

            if t >= 0.0 && y > 0.0 && y < baba {
                nearest = Some(t);
            }
        }
    }

    // End caps. Keep the closest positive hit so overlapping body colliders
    // remain deterministic instead of depending on ECS query iteration order.
    for center in [pa, pb] {
        if let Some(t) = ray_sphere_distance(ray_origin, ray_dir, center, radius) {
            nearest = Some(match nearest {
                Some(current) => current.min(t),
                None => t,
            });
        }
    }

    nearest
}

fn vrm_bone_position(bone_query: &Query<(&VrmBone, &GlobalTransform)>, name: &str) -> Option<Vec3> {
    bone_query
        .iter()
        .find_map(|(bone, transform)| (bone.as_str() == name).then(|| transform.translation()))
}

fn project_vrm_bone(
    name: &str,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    bone_query: &Query<(&VrmBone, &GlobalTransform)>,
) -> Option<Vec2> {
    let world = vrm_bone_position(bone_query, name)?;
    camera.world_to_viewport(camera_transform, world).ok()
}

/// Screen-space semantic head hit-test.
///
/// The humanoid `head` joint is not guaranteed to sit at the visual center of
/// a VRM's hair/face. Eye bones, however, are much more stable visually. We
/// therefore build a small ellipse around the rendered face using the eye
/// separation as scale. This matches what the user actually sees and avoids
/// the old "click empty air above the head" problem.
pub(super) fn head_part_at_cursor(
    cursor: Vec2,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    bone_query: &Query<(&VrmBone, &GlobalTransform)>,
    character_scale: f32,
) -> bool {
    let left_eye = project_vrm_bone("leftEye", camera, camera_transform, bone_query);
    let right_eye = project_vrm_bone("rightEye", camera, camera_transform, bone_query);

    let (center, radius_x, radius_y) = match (left_eye, right_eye) {
        (Some(left), Some(right)) => {
            let eye_mid = (left + right) * 0.5;
            let eye_distance = left.distance(right).max(1.0);

            // Visual head center is slightly above the eye line in viewport
            // coordinates (Y grows downward).
            let radius_x =
                (eye_distance * 2.0).clamp(30.0 * character_scale, 48.0 * character_scale);
            let radius_y = (radius_x * 1.20).clamp(36.0 * character_scale, 58.0 * character_scale);
            let center = eye_mid + Vec2::new(0.0, -radius_y * 0.22);
            (center, radius_x, radius_y)
        }
        _ => {
            // VRMs without eye bones still get a conservative fallback based
            // on the projected head joint, shifted down toward the visible face.
            let Some(head) = project_vrm_bone("head", camera, camera_transform, bone_query) else {
                return false;
            };
            (
                head + Vec2::new(0.0, 10.0 * character_scale),
                34.0 * character_scale,
                42.0 * character_scale,
            )
        }
    };

    let dx = (cursor.x - center.x) / radius_x;
    let dy = (cursor.y - center.y) / radius_y;
    dx * dx + dy * dy <= 1.0
}

pub(super) fn nearest_nera_body_part(
    ray_origin: Vec3,
    ray_dir: Vec3,
    colliders_query: &Query<(&GlobalTransform, &NeraCollider)>,
    bone_query: &Query<(&VrmBone, &GlobalTransform)>,
    character_scale: f32,
) -> Option<BodyPart> {
    let mut nearest: Option<(BodyPart, f32)> = None;

    for (bone_transform, collider) in colliders_query.iter() {
        let pa = bone_transform.translation();

        let distance = match collider.shape {
            NeraColliderShape::Sphere { radius } => {
                ray_sphere_distance(ray_origin, ray_dir, pa, radius * character_scale)
            }
            NeraColliderShape::Capsule { radius, end_bone } => {
                let pb = bone_query.iter().find_map(|(bone, transform)| {
                    (bone.as_str() == end_bone).then(|| transform.translation())
                });

                match pb {
                    Some(pb) => {
                        ray_capsule_distance(ray_origin, ray_dir, pa, pb, radius * character_scale)
                    }
                    None => ray_sphere_distance(ray_origin, ray_dir, pa, radius * character_scale),
                }
            }
        };

        let Some(distance) = distance else {
            continue;
        };

        if nearest
            .map(|(_, current_distance)| distance < current_distance)
            .unwrap_or(true)
        {
            nearest = Some((collider.body_part, distance));
        }
    }

    nearest.map(|(body_part, _)| body_part)
}

// -----------------------------------------------------------------------------
// Click Through & Bone Colliders
// -----------------------------------------------------------------------------

pub(crate) fn attach_bone_colliders(
    mut commands: Commands,
    bones: Query<(Entity, &VrmBone), Added<VrmBone>>,
) {
    use BodyPart::*;

    for (entity, bone) in &bones {
        let collider = match bone.as_str() {
            // Head is intentionally handled in screen-space by
            // `head_part_at_cursor()`. A generic 3D sphere around the humanoid
            // head joint proved too dependent on how each VRM places that joint.
            "head" => None,

            "hips" => Some(NeraCollider {
                body_part: Torso,
                shape: NeraColliderShape::Capsule {
                    radius: 0.18,
                    end_bone: "spine",
                },
            }),
            "spine" => Some(NeraCollider {
                body_part: Torso,
                shape: NeraColliderShape::Capsule {
                    radius: 0.19,
                    end_bone: "chest",
                },
            }),
            "chest" => Some(NeraCollider {
                body_part: Torso,
                shape: NeraColliderShape::Capsule {
                    radius: 0.20,
                    end_bone: "upperChest",
                },
            }),
            "upperChest" => Some(NeraCollider {
                body_part: Torso,
                shape: NeraColliderShape::Capsule {
                    radius: 0.18,
                    end_bone: "neck",
                },
            }),

            "leftShoulder" => Some(NeraCollider {
                body_part: LeftArm,
                shape: NeraColliderShape::Capsule {
                    radius: 0.10,
                    end_bone: "leftUpperArm",
                },
            }),
            "leftUpperArm" => Some(NeraCollider {
                body_part: LeftArm,
                shape: NeraColliderShape::Capsule {
                    radius: 0.10,
                    end_bone: "leftLowerArm",
                },
            }),
            "leftLowerArm" => Some(NeraCollider {
                body_part: LeftArm,
                shape: NeraColliderShape::Capsule {
                    radius: 0.09,
                    end_bone: "leftHand",
                },
            }),
            "leftHand" => Some(NeraCollider {
                body_part: LeftHand,
                shape: NeraColliderShape::Sphere { radius: 0.10 },
            }),

            "rightShoulder" => Some(NeraCollider {
                body_part: RightArm,
                shape: NeraColliderShape::Capsule {
                    radius: 0.10,
                    end_bone: "rightUpperArm",
                },
            }),
            "rightUpperArm" => Some(NeraCollider {
                body_part: RightArm,
                shape: NeraColliderShape::Capsule {
                    radius: 0.10,
                    end_bone: "rightLowerArm",
                },
            }),
            "rightLowerArm" => Some(NeraCollider {
                body_part: RightArm,
                shape: NeraColliderShape::Capsule {
                    radius: 0.09,
                    end_bone: "rightHand",
                },
            }),
            "rightHand" => Some(NeraCollider {
                body_part: RightHand,
                shape: NeraColliderShape::Sphere { radius: 0.10 },
            }),

            "leftUpperLeg" => Some(NeraCollider {
                body_part: LeftLeg,
                shape: NeraColliderShape::Capsule {
                    radius: 0.13,
                    end_bone: "leftLowerLeg",
                },
            }),
            "leftLowerLeg" => Some(NeraCollider {
                body_part: LeftLeg,
                shape: NeraColliderShape::Capsule {
                    radius: 0.11,
                    end_bone: "leftFoot",
                },
            }),
            "leftFoot" => Some(NeraCollider {
                body_part: LeftFoot,
                shape: NeraColliderShape::Sphere { radius: 0.11 },
            }),

            "rightUpperLeg" => Some(NeraCollider {
                body_part: RightLeg,
                shape: NeraColliderShape::Capsule {
                    radius: 0.13,
                    end_bone: "rightLowerLeg",
                },
            }),
            "rightLowerLeg" => Some(NeraCollider {
                body_part: RightLeg,
                shape: NeraColliderShape::Capsule {
                    radius: 0.11,
                    end_bone: "rightFoot",
                },
            }),
            "rightFoot" => Some(NeraCollider {
                body_part: RightFoot,
                shape: NeraColliderShape::Sphere { radius: 0.11 },
            }),
            _ => None,
        };

        if let Some(collider) = collider {
            commands.entity(entity).insert(collider);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn ray_hits_use_forward_surface_even_when_origin_is_inside() {
        assert_eq!(
            ray_sphere_distance(Vec3::ZERO, Vec3::Z, Vec3::Z * 3.0, 1.0),
            Some(2.0)
        );
        assert_eq!(
            ray_sphere_distance(Vec3::ZERO, Vec3::Z, Vec3::ZERO, 1.0),
            Some(1.0)
        );
        assert_eq!(
            ray_sphere_distance(Vec3::ZERO, Vec3::Z, Vec3::NEG_Z * 3.0, 1.0),
            None
        );
    }

    #[test]
    fn capsule_handles_degenerate_bones_and_parallel_rays() {
        let center = Vec3::Z * 3.0;
        assert_eq!(
            ray_capsule_distance(Vec3::ZERO, Vec3::Z, center, center, 1.0),
            Some(2.0)
        );
        assert_eq!(
            ray_capsule_distance(Vec3::ZERO, Vec3::Z, center, center * 2.0, 1.0),
            Some(2.0)
        );
    }

    #[test]
    fn nearest_body_part_is_independent_of_collider_spawn_order() {
        for reverse in [false, true] {
            let mut world = World::new();
            let mut colliders = [(BodyPart::Torso, 5.0), (BodyPart::LeftHand, 2.0)];
            if reverse {
                colliders.reverse();
            }
            for (body_part, z) in colliders {
                world.spawn((
                    GlobalTransform::from_translation(Vec3::Z * z),
                    NeraCollider {
                        body_part,
                        shape: NeraColliderShape::Sphere { radius: 0.5 },
                    },
                ));
            }
            let hit = world
                .run_system_once(
                    |colliders: Query<(&GlobalTransform, &NeraCollider)>,
                     bones: Query<(&VrmBone, &GlobalTransform)>| {
                        nearest_nera_body_part(Vec3::ZERO, Vec3::Z, &colliders, &bones, 1.0)
                    },
                )
                .unwrap();
            assert_eq!(hit, Some(BodyPart::LeftHand));
        }
    }
}
