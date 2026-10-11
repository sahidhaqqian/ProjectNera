//! Read-only canonical skin bounds. Never changes avatar/bone transforms.
use super::{PortraitAvatars, UpperBodyBounds};
use crate::interaction::NeraLiveBone;
use bevy::{
    ecs::system::SystemParam,
    mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    prelude::*,
};
use bevy_vrm1::prelude::{RestGlobalTransform, VrmBone};

#[derive(SystemParam)]
pub(super) struct FullBodyGeometry<'w, 's> {
    meshes: Option<Res<'w, Assets<Mesh>>>,
    binds: Option<Res<'w, Assets<SkinnedMeshInverseBindposes>>>,
    skins: Query<'w, 's, (Entity, &'static Mesh3d, &'static SkinnedMesh)>,
    rest: Query<
        'w,
        's,
        (
            Entity,
            &'static RestGlobalTransform,
            Option<&'static VrmBone>,
            Has<NeraLiveBone>,
        ),
    >,
    parents: Query<'w, 's, &'static ChildOf>,
}

impl FullBodyGeometry<'_, '_> {
    /// Screensaver's close-up keeps the measured crown/hair and crops at the
    /// humanoid hips. Legs remain below the viewport instead of determining zoom.
    pub(super) fn portrait_bounds(
        &self,
        avatars: &PortraitAvatars,
        body: f32,
        head: f32,
        root_offset: f32,
        predict_root: bool,
    ) -> Option<UpperBodyBounds> {
        let mut bounds = self.bounds(avatars, body, head, root_offset, predict_root)?;
        let (avatar, root, rest_root, _, _) = avatars.single().ok()?;
        let inverse_root = rest_root.map_or(Mat4::IDENTITY, |rest| rest.0.to_matrix().inverse());
        let hips = self.rest.iter().find_map(|(entity, rest, bone, live)| {
            (live
                && bone.is_some_and(|bone| bone.as_str() == "hips")
                && self.descendant(entity, avatar))
            .then(|| inverse_root.transform_point3(rest.0.translation()))
        })?;
        let root = root.to_matrix()
            * Mat4::from_translation(Vec3::Y * if predict_root { root_offset } else { 0.0 });
        let hips = root.transform_point3(hips);
        if !hips.is_finite() || bounds.top_y - hips.y < 0.01 {
            return None;
        }
        let half_width = bounds.width() * 0.5;
        bounds.min_x = hips.x - half_width;
        bounds.max_x = hips.x + half_width;
        bounds.bottom_y = hips.y;
        Some(bounds)
    }

    fn descendant(&self, mut entity: Entity, ancestor: Entity) -> bool {
        for _ in 0..128 {
            if entity == ancestor {
                return true;
            }
            let Ok(parent) = self.parents.get(entity) else {
                return false;
            };
            entity = parent.parent();
        }
        false
    }

    pub(super) fn bounds(
        &self,
        avatars: &PortraitAvatars,
        body: f32,
        head: f32,
        root_offset: f32,
        predict_root: bool,
    ) -> Option<UpperBodyBounds> {
        let (avatar, root, rest_root, true, false) = avatars.single().ok()? else {
            return None;
        };
        let meshes = self.meshes.as_ref()?;
        let binds = self.binds.as_ref()?;
        let rest_root = rest_root.map_or(Mat4::IDENTITY, |r| r.0.to_matrix());
        let inverse_root = rest_root.inverse();
        let landmark = |name: &str| {
            self.rest.iter().find_map(|(entity, rest, bone, live)| {
                (live
                    && bone.map(|bone| bone.as_str()) == Some(name)
                    && self.descendant(entity, avatar))
                .then(|| inverse_root.transform_point3(rest.0.translation()))
            })
        };
        let hips = landmark("hips")?;
        let head_point = landmark("head")?;
        let foot = [landmark("leftFoot"), landmark("rightFoot")]
            .into_iter()
            .flatten()
            .min_by(|a, b| a.y.total_cmp(&b.y))?;
        if !hips.is_finite()
            || !head_point.is_finite()
            || !foot.is_finite()
            || head_point.y <= foot.y
            || body <= 0.0
        {
            return None;
        }
        // The existing body owner exposes its rest-derived standing correction.
        // Invert that measurement to recover thigh compression, rather than
        // copying Chibi constants or introducing a second proportion owner.
        let leg_delta_y = ((foot.y - hips.y) * (1.0 - body) - root_offset) / body;
        let reference_segment = ["left", "right"]
            .into_iter()
            .filter_map(|side| {
                Some((
                    landmark(&format!("{side}LowerLeg"))? - landmark(&format!("{side}UpperLeg"))?,
                    landmark(&format!("{side}Foot"))?,
                ))
            })
            .min_by(|a, b| a.1.y.total_cmp(&b.1.y))
            .map(|(segment, _)| segment);
        let compression = reference_segment
            .filter(|s| s.y.abs() > 0.00001)
            .map_or(0.0, |s| leg_delta_y / s.y);
        // Find semantic ancestors through the actual imported hierarchy.
        let semantic_ancestor = |mut joint: Entity, name: &str| {
            for _ in 0..128 {
                if self
                    .rest
                    .get(joint)
                    .ok()
                    .and_then(|(_, _, bone, live)| live.then_some(bone).flatten())
                    .is_some_and(|b| b.as_str() == name)
                {
                    return true;
                }
                let Ok(parent) = self.parents.get(joint) else {
                    return false;
                };
                joint = parent.parent();
            }
            false
        };
        let root = root.to_matrix()
            * Mat4::from_translation(Vec3::Y * if predict_root { root_offset } else { 0.0 });
        let body_matrix = Mat4::from_translation(hips)
            * Mat4::from_scale(Vec3::splat(body))
            * Mat4::from_translation(-hips);
        let head_matrix = Mat4::from_translation(head_point)
            * Mat4::from_scale(Vec3::splat(head))
            * Mat4::from_translation(-head_point);
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for (entity, mesh, skin) in &self.skins {
            if !self.descendant(entity, avatar) {
                continue;
            }
            let Some(mesh) = meshes.get(&mesh.0) else {
                continue;
            };
            let Some(bind) = binds.get(&skin.inverse_bindposes) else {
                continue;
            };
            // Vertex arrays have already moved to RenderWorld in Nera. These
            // compact joint bounds intentionally survive GPU-only extraction.
            let Some(compact) = mesh.skinned_mesh_bounds() else {
                continue;
            };
            let matrices: Vec<_> = skin
                .joints
                .iter()
                .zip(bind.iter())
                .map(|(&joint, bind)| {
                    let rest = self.rest.get(joint).map_or_else(
                        |_| bind.inverse(),
                        |(_, r, _, _)| inverse_root * r.0.to_matrix(),
                    );
                    let mut local = rest;
                    for side in ["left", "right"] {
                        if semantic_ancestor(joint, &format!("{side}LowerLeg")) {
                            if let (Some(upper), Some(lower)) = (
                                landmark(&format!("{side}UpperLeg")),
                                landmark(&format!("{side}LowerLeg")),
                            ) {
                                local =
                                    Mat4::from_translation((lower - upper) * compression) * local;
                            }
                        }
                    }
                    if semantic_ancestor(joint, "head") {
                        local = head_matrix * local;
                    }
                    root * if semantic_ancestor(joint, "hips") {
                        body_matrix
                    } else {
                        Mat4::IDENTITY
                    } * local
                        * *bind
                })
                .collect();
            for (index, bounds) in compact.iter() {
                let Some(matrix) = matrices.get(index.0 as usize) else {
                    continue;
                };
                for x in [-1.0, 1.0] {
                    for y in [-1.0, 1.0] {
                        for z in [-1.0, 1.0] {
                            let point = matrix.transform_point3(
                                bounds.center + bounds.half_size * Vec3::new(x, y, z),
                            );
                            if point.is_finite() {
                                min = min.min(point);
                                max = max.max(point);
                            }
                        }
                    }
                }
            }
        }
        if !min.is_finite() || !max.is_finite() || max.y - min.y < 0.01 {
            return None;
        }
        Some(UpperBodyBounds {
            min_x: min.x,
            max_x: max.x,
            bottom_y: min.y,
            top_y: max.y,
            front_z: max.z,
        })
    }
}
