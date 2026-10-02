//! Small shared meshes, bounded event bursts, and real-time expiry. No emitter engine.
use crate::{
    avatar::{NeraMainCamera, desktop_point_to_world},
    environment::DesktopEnvironment,
    interaction::NeraLiveBone,
    presentation::PresentationState,
    settings::NeraSettings,
};
use bevy::{
    asset::RenderAssetUsages,
    light::{NotShadowCaster, NotShadowReceiver},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
    window::PrimaryWindow,
};
use bevy_vrm1::prelude::VrmBone;
use std::time::Duration;

pub(crate) const MAX_PARTICLES: usize = 32;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum PetFxKind {
    HeadPatSparkle,
    LandingPuff,
    AngryMark,
    SleepZ,
    CloudPuff,
}
impl PetFxKind {
    fn count(self) -> usize {
        match self {
            Self::HeadPatSparkle => 3,
            Self::LandingPuff | Self::CloudPuff => 4,
            _ => 1,
        }
    }
    fn lifetime(self) -> f32 {
        match self {
            Self::SleepZ => 1.8,
            Self::AngryMark => 0.9,
            _ => 0.75,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum PetFxAnchor {
    Head,
    Feet,
    Desktop(Vec2),
}
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct PetFx {
    pub(crate) kind: PetFxKind,
    pub(crate) anchor: PetFxAnchor,
}

#[derive(Component)]
pub(crate) struct PetParticle {
    position: Vec2,
    velocity: Vec2,
    remaining: f32,
    lifetime: f32,
    size: f32,
}
impl PetParticle {
    fn advance(&mut self, dt: f32) -> bool {
        self.remaining -= dt;
        self.position += self.velocity * dt.min(0.1);
        self.remaining > 0.0
    }
}
#[derive(Resource, Default)]
pub(crate) struct FxAssets {
    entries: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
}
#[derive(Resource, Default)]
pub(crate) struct FxCooldown {
    emitted_at: [Option<Duration>; 5],
}
impl FxCooldown {
    fn admit(&mut self, kind: PetFxKind, now: Duration, count: usize) -> usize {
        let last = &mut self.emitted_at[kind as usize];
        if count >= MAX_PARTICLES
            || last.is_some_and(|t| now.saturating_sub(t) < Duration::from_millis(250))
        {
            return 0;
        }
        *last = Some(now);
        kind.count().min(MAX_PARTICLES - count)
    }
}
pub(super) fn install(app: &mut App) {
    app.init_resource::<FxAssets>()
        .init_resource::<FxCooldown>()
        .add_message::<PetFx>()
        .add_systems(Startup, create_fx_assets);
}

fn glyph_mesh(rectangles: &[(Vec2, Vec2, f32)]) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for &(center, half, rotation) in rectangles {
        let start = positions.len() as u32;
        for corner in [
            Vec2::new(-half.x, -half.y),
            Vec2::new(half.x, -half.y),
            half,
            Vec2::new(-half.x, half.y),
        ] {
            let point = Mat2::from_angle(rotation) * corner + center;
            positions.push([point.x, point.y, 0.0]);
            normals.push([0.0, 0.0, 1.0]);
        }
        indices.extend([start, start + 1, start + 2, start, start + 2, start + 3]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}
fn create_fx_assets(
    mut assets: ResMut<FxAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let sparkle = glyph_mesh(&[
        (Vec2::ZERO, Vec2::new(0.12, 0.5), 0.0),
        (Vec2::ZERO, Vec2::new(0.5, 0.12), 0.0),
    ]);
    let z = glyph_mesh(&[
        (Vec2::new(0.0, 0.35), Vec2::new(0.40, 0.08), 0.0),
        (Vec2::ZERO, Vec2::new(0.48, 0.08), 0.72),
        (Vec2::new(0.0, -0.35), Vec2::new(0.40, 0.08), 0.0),
    ]);
    let angry = glyph_mesh(&[
        (Vec2::new(-0.25, 0.25), Vec2::new(0.22, 0.065), 0.0),
        (Vec2::new(-0.25, 0.25), Vec2::new(0.065, 0.22), 0.0),
        (Vec2::new(0.25, -0.25), Vec2::new(0.22, 0.065), 0.0),
        (Vec2::new(0.25, -0.25), Vec2::new(0.065, 0.22), 0.0),
    ]);
    for (mesh, color) in [
        (sparkle, Color::srgb(1.0, 0.79, 0.22)),
        (
            Mesh::from(Circle::new(0.5)),
            Color::srgba(0.78, 0.88, 0.94, 0.55),
        ),
        (angry, Color::srgb(0.96, 0.20, 0.27)),
        (z, Color::srgb(0.32, 0.75, 0.86)),
        (
            Mesh::from(Circle::new(0.5)),
            Color::srgba(0.88, 0.95, 1.0, 0.65),
        ),
    ] {
        assets.entries.push((
            meshes.add(mesh),
            materials.add(StandardMaterial {
                base_color: color,
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                cull_mode: None,
                ..default()
            }),
        ));
    }
}

fn anchor_point(
    anchor: PetFxAnchor,
    environment: &DesktopEnvironment,
    camera: &Camera,
    view: &GlobalTransform,
    bones: &Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
) -> Option<Vec2> {
    let rect = environment.nera_rect?;
    let scale = environment.nera_scale_factor.max(0.25) as f32;
    let origin = Vec2::new(rect.left as f32, rect.top as f32);
    match anchor {
        PetFxAnchor::Desktop(point) => point.is_finite().then_some(point),
        PetFxAnchor::Feet => environment
            .feet_anchor_logical
            .map(|point| origin + point * scale),
        PetFxAnchor::Head => bones
            .iter()
            .find(|(bone, _)| bone.as_str() == "head")
            .and_then(|(_, transform)| {
                camera
                    .world_to_viewport(view, transform.translation() + Vec3::Y * 0.10)
                    .ok()
            })
            .map(|point| origin + point * scale),
    }
}

/// Last follows native Cloud/seat alignment, so particles stay in desktop space during drag.
pub(crate) fn update_particles(
    time: Res<Time<Real>>,
    mut requests: MessageReader<PetFx>,
    assets: Res<FxAssets>,
    mut cooldown: ResMut<FxCooldown>,
    environment: Res<DesktopEnvironment>,
    settings: Res<NeraSettings>,
    presentation: Res<PresentationState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<NeraMainCamera>>,
    bones: Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
    mut particles: Query<
        (
            Entity,
            &mut PetParticle,
            &mut Transform,
            &mut GlobalTransform,
        ),
        (Without<NeraMainCamera>, Without<NeraLiveBone>),
    >,
    mut commands: Commands,
) {
    let visible = presentation.is_desktop_pet() && windows.iter().any(|w| w.visible);
    let view = camera.single().ok();
    let mut count = 0;
    for (entity, mut particle, mut transform, mut global) in &mut particles {
        if !particle.advance(time.delta_secs()) || !visible {
            commands.entity(entity).despawn();
            continue;
        }
        count += 1;
        if let Some((camera, view)) = view
            && let Some(point) =
                desktop_point_to_world(particle.position, &environment, camera, view, 0.6)
        {
            transform.translation = point;
            transform.scale =
                Vec3::splat(particle.size * (particle.remaining / particle.lifetime).sqrt());
            *global = GlobalTransform::from(*transform);
        }
    }
    for request in requests.read() {
        if !visible {
            continue;
        }
        let Some((camera, view)) = view else { continue };
        let Some(origin) = anchor_point(request.anchor, &environment, camera, view, &bones) else {
            continue;
        };
        let admitted = cooldown.admit(request.kind, time.elapsed(), count);
        let Some((mesh, material)) = assets.entries.get(request.kind as usize) else {
            continue;
        };
        let scale = environment.nera_scale_factor.max(0.25) as f32 * settings.character_scale();
        for index in 0..admitted {
            let spread = (index as f32 - (admitted - 1) as f32 * 0.5) * 15.0;
            let offset = match request.kind {
                PetFxKind::AngryMark | PetFxKind::SleepZ => Vec2::new(32.0, -22.0),
                PetFxKind::HeadPatSparkle => Vec2::new(spread, -12.0 - (index % 2) as f32 * 10.0),
                _ => Vec2::new(spread, 0.0),
            };
            let position = origin + offset * scale;
            let Some(world) = desktop_point_to_world(position, &environment, camera, view, 0.6)
            else {
                continue;
            };
            let size = match request.kind {
                PetFxKind::LandingPuff | PetFxKind::CloudPuff => 0.10,
                _ => 0.08,
            } * settings.character_scale();
            let transform = Transform::from_translation(world).with_scale(Vec3::splat(size));
            commands.spawn((
                Name::new("Nera Interaction FX"),
                PetParticle {
                    position,
                    velocity: Vec2::new(spread * 0.5, -25.0) * scale,
                    remaining: request.kind.lifetime(),
                    lifetime: request.kind.lifetime(),
                    size,
                },
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                transform,
                GlobalTransform::from(transform),
                NotShadowCaster,
                NotShadowReceiver,
            ));
            count += 1;
        }
    }
}
pub(super) fn clear(world: &mut World) {
    let particles: Vec<_> = world
        .query_filtered::<Entity, With<PetParticle>>()
        .iter(world)
        .collect();
    for particle in particles {
        world.despawn(particle);
    }
    if let Some(mut requests) = world.get_resource_mut::<Messages<PetFx>>() {
        requests.clear();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ecs_cleanup_expires_the_entire_budget_without_a_camera_or_native_window() {
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .init_resource::<FxAssets>()
            .init_resource::<FxCooldown>()
            .init_resource::<DesktopEnvironment>()
            .init_resource::<NeraSettings>()
            .init_resource::<PresentationState>()
            .add_message::<PetFx>()
            .add_systems(Update, update_particles);
        for _ in 0..MAX_PARTICLES {
            app.world_mut().spawn((
                PetParticle {
                    position: Vec2::ZERO,
                    velocity: Vec2::ONE,
                    remaining: 1.8,
                    lifetime: 1.8,
                    size: 1.0,
                },
                Transform::default(),
                GlobalTransform::default(),
            ));
        }
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_secs(3));
        app.update();
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<PetParticle>>()
                .iter(app.world())
                .count(),
            0
        );
        app.update();
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<PetParticle>>()
                .iter(app.world())
                .count(),
            0
        );
    }
    #[test]
    fn every_effect_expires_even_after_a_long_frame() {
        for kind in [
            PetFxKind::HeadPatSparkle,
            PetFxKind::LandingPuff,
            PetFxKind::AngryMark,
            PetFxKind::SleepZ,
            PetFxKind::CloudPuff,
        ] {
            let mut particle = PetParticle {
                position: Vec2::ZERO,
                velocity: Vec2::ONE,
                remaining: kind.lifetime(),
                lifetime: kind.lifetime(),
                size: 1.0,
            };
            assert!(particle.advance(0.1));
            assert!(!particle.advance(10.0));
        }
    }
    #[test]
    fn burst_budget_and_debounce_are_bounded_over_many_events() {
        let mut cooldown = FxCooldown::default();
        let mut count = 0;
        for i in 0..10_000 {
            count += cooldown.admit(
                PetFxKind::HeadPatSparkle,
                Duration::from_millis(i * 250),
                count,
            );
            assert!(count <= MAX_PARTICLES);
        }
        assert_eq!(count, MAX_PARTICLES);
        assert_eq!(
            cooldown.admit(PetFxKind::CloudPuff, Duration::from_secs(3000), count),
            0
        );
        assert_eq!(
            cooldown.admit(PetFxKind::HeadPatSparkle, Duration::from_secs(3000), 0),
            3
        );
        assert_eq!(
            cooldown.admit(PetFxKind::HeadPatSparkle, Duration::from_secs(3000), 3),
            0
        );
    }
}
