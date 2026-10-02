//! Replaceable, lazily loaded visual. No positioning or physics authority here.
use super::{APPEAR_SECONDS, CloudRuntime, CloudState, DISAPPEAR_SECONDS};
use crate::{
    avatar::{NeraMainCamera, desktop_point_to_world},
    environment::DesktopEnvironment,
    interaction::{DesktopPetInteraction, InteractionState},
    settings::NeraSettings,
};
use bevy::world_serialization::{WorldAsset, WorldAssetRoot};
use bevy::{
    light::{NotShadowCaster, NotShadowReceiver},
    prelude::*,
};

#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct CloudAssets<'w> {
    pub(super) visuals: ResMut<'w, CloudVisualAssets>,
    pub(super) server: Res<'w, AssetServer>,
    pub(super) meshes: ResMut<'w, Assets<Mesh>>,
    pub(super) materials: ResMut<'w, Assets<StandardMaterial>>,
}

const MODEL: &str = "vrm/Cloud/nera_cloud_1024.glb";
const MODEL_SCALE: f32 = 0.28;
// Existing seated clips let the legs hang. Sit on the GLB's measured front rim,
// not its center/outer lobe peak, so that rim cannot intersect the shins.
const MODEL_SEAT: Vec3 = Vec3::new(0.0, 0.94, 1.4);
const FALLBACK_SEAT: Vec3 = Vec3::new(0.0, 1.16, 0.8);
// Keep the high back lobes behind the face while supporting the lying pelvis.
const MODEL_REST_CONTACT: Vec3 = Vec3::new(0.0, 1.12, 0.95);
const FALLBACK_REST_CONTACT: Vec3 = Vec3::new(0.0, 1.32, 0.5);

#[derive(Component, Default)]
pub(crate) struct CloudVisual {
    seat_local: Vec3,
    rest_local: Vec3,
    seat_depth: f32,
}

#[derive(Resource, Default)]
pub(super) struct CloudVisualAssets {
    model: Option<Handle<WorldAsset>>,
    checked: bool,
    fallback: Option<(Handle<Mesh>, Handle<StandardMaterial>)>,
}

impl CloudVisualAssets {
    #[cfg(test)]
    pub(super) fn procedural() -> Self {
        Self {
            checked: true,
            ..default()
        }
    }
    pub(super) fn spawn(
        &mut self,
        server: &AssetServer,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        commands: &mut Commands,
    ) -> Entity {
        if !self.checked {
            self.checked = true;
            if crate::nera_asset_file(MODEL).is_file() {
                self.model = Some(server.load(GltfAssetLabel::Scene(0).from_asset(MODEL)));
            }
        }
        let mut root = commands.spawn((
            Name::new("Nera Cloud"),
            CloudVisual {
                seat_local: if self.model.is_some() {
                    MODEL_SEAT
                } else {
                    FALLBACK_SEAT
                },
                seat_depth: 0.0,
                rest_local: if self.model.is_some() {
                    MODEL_REST_CONTACT
                } else {
                    FALLBACK_REST_CONTACT
                },
            },
            Transform::default(),
            Visibility::Hidden,
        ));
        if let Some(scene) = &self.model {
            root.insert(WorldAssetRoot(scene.clone()));
        } else {
            let (mesh, material) = self.fallback.get_or_insert_with(|| {
                (
                    meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap()),
                    materials.add(StandardMaterial {
                        base_color: Color::srgb(0.90, 0.94, 1.0),
                        perceptual_roughness: 1.0,
                        ..default()
                    }),
                )
            });
            root.with_children(|parent| {
                for (x, y, radius) in [
                    (-1.6, 0.60, 0.9),
                    (-0.8, 0.85, 0.9),
                    (0.0, 0.80, 1.0),
                    (0.8, 0.65, 0.95),
                    (1.6, 0.55, 0.8),
                ] {
                    parent.spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_xyz(x, y, 0.0).with_scale(Vec3::new(
                            radius,
                            radius * 0.6,
                            radius,
                        )),
                        NotShadowCaster,
                        NotShadowReceiver,
                    ));
                }
            });
        }
        root.id()
    }
}

pub(crate) fn cloud_hit(
    cloud: &CloudRuntime,
    cursor: Vec2,
    environment: &DesktopEnvironment,
    camera: &Camera,
    view: &GlobalTransform,
    visuals: &Query<&GlobalTransform, With<CloudVisual>>,
) -> bool {
    let Some(attachment) = cloud.attachment() else {
        return false;
    };
    let Some(rect) = environment.nera_rect else {
        return false;
    };
    let Ok(root) = visuals.get(attachment.visual) else {
        return false;
    };
    let center = root.transform_point(Vec3::new(0.0, 0.85, 0.0));
    let point = Vec2::new(rect.left as f32, rect.top as f32)
        + cursor * environment.nera_scale_factor.max(0.25) as f32;
    let Some(world) = desktop_point_to_world(point, environment, camera, view, center.z) else {
        return false;
    };
    let local = root.affine().inverse().transform_point3(world);
    let delta = (local.truncate() - Vec2::Y * 0.85) / Vec2::new(2.47, 0.85);
    delta.length_squared() <= 1.0
}

pub(crate) fn animate_cloud_visual(
    time: Res<Time<Real>>,
    cloud: Res<CloudRuntime>,
    environment: Res<DesktopEnvironment>,
    settings: Res<NeraSettings>,
    interaction: Res<DesktopPetInteraction>,
    clips: Res<crate::animation::NeraAnimationClips>,
    camera: Query<(&Camera, &GlobalTransform), With<NeraMainCamera>>,
    mut roots: Query<(&mut Transform, &mut Visibility, &mut CloudVisual)>,
    avatar: Query<&bevy_vrm1::prelude::HipsBoneEntity, With<crate::avatar::NeraAvatar>>,
    children: Query<&Children>,
    locals: Query<&Transform, Without<CloudVisual>>,
    mut globals: Query<&mut GlobalTransform, Without<NeraMainCamera>>,
) {
    let Some(entity) = cloud.visual() else { return };
    let (seat, size) = match cloud.state {
        CloudState::Hidden => return,
        CloudState::Appearing {
            attachment,
            elapsed,
        } => (attachment.seat, (elapsed / APPEAR_SECONDS).clamp(0.01, 1.0)),
        CloudState::Attached(a) => (a.seat, 1.0),
        CloudState::Disappearing {
            seat, remaining, ..
        } => (seat, (remaining / DISAPPEAR_SECONDS).clamp(0.01, 1.0)),
    };
    let Ok((camera, view)) = camera.single() else {
        return;
    };
    let sleeping = cloud.is_lying_rest() && clips.sleeping_idle_entity().is_some();
    let live_depth = avatar
        .single()
        .ok()
        .and_then(|hips| globals.get(hips.0).ok())
        .map(|hips| cloud.contact_world(hips, sleeping))
        .map(|point| point.z);
    let Ok((mut transform, mut visibility, mut visual)) = roots.get_mut(entity) else {
        return;
    };
    if cloud.owns_placement() {
        if sleeping {
            visual.seat_local = visual.rest_local;
        }
        if let Some(depth) = live_depth {
            visual.seat_depth = depth;
        }
    }
    let Some(world_seat) =
        desktop_point_to_world(seat, &environment, camera, view, visual.seat_depth)
    else {
        return;
    };
    let scale = MODEL_SCALE * settings.character_scale();
    let bob = if interaction.state == InteractionState::Dragging || !cloud.owns_placement() {
        0.0
    } else {
        (time.elapsed_secs() * 1.6).sin() * 0.006 * settings.character_scale()
    };
    *visibility = Visibility::Visible;
    *transform = seat_transform(world_seat, visual.seat_local, scale * size, bob);
    let root_global = GlobalTransform::from(*transform);
    // Last runs after pose-derived seat calibration and transform propagation.
    // Update only this small visual hierarchy, not the entire avatar/world.
    if let Ok(mut global) = globals.get_mut(entity) {
        *global = root_global;
    }
    let mut stack = vec![(entity, root_global)];
    while let Some((parent, global)) = stack.pop() {
        if let Ok(children) = children.get(parent) {
            for child in children.iter() {
                if let Ok(local) = locals.get(child) {
                    let next = global.mul_transform(*local);
                    if let Ok(mut target) = globals.get_mut(child) {
                        *target = next;
                    }
                    stack.push((child, next));
                }
            }
        }
    }
}

fn seat_transform(contact: Vec3, seat_local: Vec3, scale: f32, bob: f32) -> Transform {
    Transform::from_translation(contact - seat_local * scale + Vec3::Y * bob)
        .with_scale(Vec3::splat(scale))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_seat_remains_at_contact_during_scaling_and_only_visual_bob_moves_it() {
        let contact = Vec3::new(-0.3, 0.75, 0.2);
        for seat in [MODEL_SEAT, FALLBACK_SEAT] {
            for character_scale in [0.6, 1.0, 1.5] {
                for appearance in [0.01, 0.5, 1.0] {
                    for bob in [-0.006, 0.0, 0.006] {
                        let root = seat_transform(
                            contact,
                            seat,
                            MODEL_SCALE * character_scale * appearance,
                            bob,
                        );
                        assert!(
                            (root.transform_point(seat) - contact - Vec3::Y * bob).length()
                                < 0.00001
                        );
                    }
                }
            }
        }
    }
}
