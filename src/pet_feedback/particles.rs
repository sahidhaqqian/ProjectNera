//! Shared meshes, bounded semantic feedback and real-time particle expiry.
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
    Heart,
    LandingPuff,
    AngryMark,
    SleepZ,
    CloudPuff,
}
impl PetFxKind {
    fn count(self) -> usize {
        match self {
            Self::Heart => 1,
            Self::LandingPuff | Self::CloudPuff => 4,
            _ => 1,
        }
    }
    fn lifetime(self) -> f32 {
        match self {
            Self::Heart => 0.85,
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
    /// Hearts follow an age-based arc; other effects retain their linear drift.
    heart_origin: Option<Vec2>,
}
impl PetParticle {
    fn advance(&mut self, dt: f32) -> bool {
        self.remaining -= dt;
        if let Some(origin) = self.heart_origin {
            let age = (self.lifetime - self.remaining).clamp(0.0, self.lifetime);
            let t = age / self.lifetime;
            self.position = origin
                + self.velocity * age
                + Vec2::X * self.velocity.x * 0.22 * (std::f32::consts::PI * t).sin();
        } else {
            self.position += self.velocity * dt.min(0.1);
        }
        self.remaining > 0.0
    }

    fn visual_scale(&self) -> f32 {
        if self.heart_origin.is_none() {
            return (self.remaining / self.lifetime).max(0.0).sqrt();
        }
        let age = self.lifetime - self.remaining;
        let appear = smoothstep(age / 0.13);
        let disappear = smoothstep(self.remaining / 0.32);
        appear * disappear
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Clone, Copy)]
struct HeartSequence {
    anchor: PetFxAnchor,
    next_pulse: u8,
    right_next: bool,
}
impl HeartSequence {
    /// Four pulses span the actual clip. Skip missed pulses after a stalled
    /// frame rather than releasing several hearts simultaneously.
    fn next(&mut self, phase: f32) -> Option<u8> {
        let due = (phase.clamp(0.0, 1.0) * 4.0).floor().min(3.0) as u8;
        if due < self.next_pulse || phase >= 1.0 {
            return None;
        }
        self.next_pulse = due + 1;
        let side = u8::from(self.right_next);
        self.right_next = !self.right_next;
        Some(side)
    }
}
#[derive(Resource, Default)]
pub(crate) struct FxAssets {
    entries: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
}
#[derive(Resource, Default)]
pub(crate) struct FxCooldown {
    emitted_at: [Option<Duration>; 5],
    heart_sequence: Option<HeartSequence>,
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
/// One shared flat heart, with 32 outline vertices and no texture allocation.
fn heart_mesh() -> Mesh {
    const SEGMENTS: u32 = 32;
    let mut positions = vec![[0.0, -0.04, 0.0]];
    let mut indices = Vec::with_capacity(SEGMENTS as usize * 3);
    for i in 0..SEGMENTS {
        let t = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        let x = 16.0 * t.sin().powi(3) / 32.0;
        let y = (13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos())
            / 32.0;
        positions.push([x, y, 0.0]);
        indices.extend([0, i + 1, (i + 1) % SEGMENTS + 1]);
    }
    let normals = vec![[0.0, 0.0, 1.0]; positions.len()];
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
    let heart = heart_mesh();
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
        (heart, Color::srgb(1.0, 0.28, 0.48)),
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
    animation: Res<crate::animation::NeraAnimationState>,
    clips: Res<crate::animation::NeraAnimationClips>,
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
    let headpat_phase = visible
        .then(|| clips.head_stroke_progress(&animation))
        .flatten();
    if headpat_phase.is_none() {
        cooldown.heart_sequence = None;
    }
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
            transform.scale = Vec3::splat(particle.size * particle.visual_scale());
            if particle.heart_origin.is_some() {
                let age = (particle.lifetime - particle.remaining) / particle.lifetime;
                let side = particle.velocity.x.signum();
                transform.rotation = Quat::from_rotation_z(
                    side * (0.12 + 0.14 * (age * std::f32::consts::PI).sin()),
                );
            }
            *global = GlobalTransform::from(*transform);
        }
    }
    let mut bursts = Vec::new();
    for request in requests.read().copied() {
        if !visible {
            continue;
        }
        let admitted = cooldown.admit(request.kind, time.elapsed(), count);
        if admitted == 0 {
            continue;
        }
        if request.kind == PetFxKind::Heart && headpat_phase.is_some() {
            let right_next = cooldown
                .heart_sequence
                .is_some_and(|sequence| sequence.right_next);
            cooldown.heart_sequence = Some(HeartSequence {
                anchor: request.anchor,
                next_pulse: 0,
                right_next,
            });
        } else {
            bursts.push((request, admitted, 0));
            count += admitted;
        }
    }
    if let Some(phase) = headpat_phase
        && let Some(sequence) = cooldown.heart_sequence.as_mut()
        && let Some(index) = sequence.next(phase)
        && count < MAX_PARTICLES
    {
        bursts.push((
            PetFx {
                kind: PetFxKind::Heart,
                anchor: sequence.anchor,
            },
            1,
            index,
        ));
    }
    for (request, admitted, heart_index) in bursts {
        let Some((camera, view)) = view else { continue };
        let Some(origin) = anchor_point(request.anchor, &environment, camera, view, &bones) else {
            continue;
        };
        let Some((mesh, material)) = assets.entries.get(request.kind as usize) else {
            continue;
        };
        let scale = environment.nera_scale_factor.max(0.25) as f32 * settings.character_scale();
        for index in 0..admitted {
            let spread = (index as f32 - (admitted - 1) as f32 * 0.5) * 15.0;
            let offset = match request.kind {
                PetFxKind::AngryMark | PetFxKind::SleepZ => Vec2::new(32.0, -22.0),
                PetFxKind::Heart => {
                    Vec2::new(if heart_index % 2 == 0 { -34.0 } else { 34.0 }, -8.0)
                }
                _ => Vec2::new(spread, 0.0),
            };
            let position = origin + offset * scale;
            let Some(world) = desktop_point_to_world(position, &environment, camera, view, 0.6)
            else {
                continue;
            };
            let size = match request.kind {
                PetFxKind::LandingPuff | PetFxKind::CloudPuff => 0.10,
                PetFxKind::Heart => 0.065,
                _ => 0.08,
            } * settings.character_scale();
            let heart = request.kind == PetFxKind::Heart;
            let side = if heart_index % 2 == 0 { -1.0 } else { 1.0 };
            let transform = Transform::from_translation(world).with_scale(Vec3::splat(if heart {
                0.0
            } else {
                size
            }));
            commands.spawn((
                Name::new("Nera Interaction FX"),
                PetParticle {
                    position,
                    velocity: if heart {
                        Vec2::new(side * 18.0, -38.0)
                    } else {
                        Vec2::new(spread * 0.5, -25.0)
                    } * scale,
                    remaining: request.kind.lifetime(),
                    lifetime: request.kind.lifetime(),
                    size,
                    heart_origin: heart.then_some(position),
                },
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                transform,
                GlobalTransform::from(transform),
                NotShadowCaster,
                NotShadowReceiver,
            ));
        }
    }
}
pub(super) fn clear(world: &mut World) {
    if let Some(mut cooldown) = world.get_resource_mut::<FxCooldown>() {
        cooldown.heart_sequence = None;
    }
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
    fn hearts_alternate_one_at_a_time_across_the_whole_clip_without_catchup_bursts() {
        let mut sequence = HeartSequence {
            anchor: PetFxAnchor::Head,
            next_pulse: 0,
            right_next: false,
        };
        for (phase, side) in [(0.0, 0), (0.25, 1), (0.50, 0), (0.75, 1)] {
            assert_eq!(sequence.next(phase), Some(side));
            assert_eq!(sequence.next(phase + 0.01), None);
        }
        assert_eq!(sequence.next(1.0), None);
        let mut stalled = HeartSequence {
            anchor: PetFxAnchor::Head,
            next_pulse: 0,
            right_next: false,
        };
        assert_eq!(stalled.next(0.0), Some(0));
        assert_eq!(stalled.next(0.70), Some(1));
        assert_eq!(stalled.next(0.70), None);
        assert_eq!(stalled.next(0.80), Some(0));
    }

    #[test]
    fn heart_arcs_are_frame_rate_independent_and_appear_disappear_smoothly() {
        let heart = || PetParticle {
            position: Vec2::ZERO,
            velocity: Vec2::new(-18.0, -38.0),
            remaining: 0.85,
            lifetime: 0.85,
            size: 0.065,
            heart_origin: Some(Vec2::ZERO),
        };
        let mut one_frame = heart();
        let mut many_frames = heart();
        assert_eq!(one_frame.visual_scale(), 0.0);
        one_frame.advance(0.425);
        for _ in 0..17 {
            many_frames.advance(0.025);
        }
        assert!((one_frame.position - many_frames.position).length() < 0.001);
        assert!(one_frame.position.x < 0.0 && one_frame.position.y < 0.0);
        assert_eq!(one_frame.visual_scale(), 1.0);
        one_frame.advance(0.36);
        assert!(one_frame.visual_scale() > 0.0 && one_frame.visual_scale() < 0.2);
        assert!(!one_frame.advance(0.1));
        assert_eq!(one_frame.visual_scale(), 0.0);
    }

    #[test]
    fn replacement_clears_pending_hearts_as_well_as_particles() {
        let mut world = World::new();
        let mut cooldown = FxCooldown::default();
        cooldown.heart_sequence = Some(HeartSequence {
            anchor: PetFxAnchor::Head,
            next_pulse: 1,
            right_next: true,
        });
        world.insert_resource(cooldown);
        world.init_resource::<Messages<PetFx>>();
        world.write_message(PetFx {
            kind: PetFxKind::Heart,
            anchor: PetFxAnchor::Head,
        });
        clear(&mut world);
        assert!(world.resource::<FxCooldown>().heart_sequence.is_none());
        assert!(world.resource::<Messages<PetFx>>().is_empty());
    }
    #[test]
    fn ecs_cleanup_expires_the_entire_budget_without_a_camera_or_native_window() {
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .init_resource::<FxAssets>()
            .init_resource::<FxCooldown>()
            .init_resource::<DesktopEnvironment>()
            .init_resource::<NeraSettings>()
            .init_resource::<crate::animation::NeraAnimationState>()
            .init_resource::<crate::animation::NeraAnimationClips>()
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
                    heart_origin: None,
                },
                Transform::default(),
                GlobalTransform::default(),
            ));
        }
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.world_mut().resource_mut::<FxCooldown>().heart_sequence = Some(HeartSequence {
            anchor: PetFxAnchor::Head,
            next_pulse: 1,
            right_next: true,
        });
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_secs(3));
        app.update();
        assert!(
            app.world()
                .resource::<FxCooldown>()
                .heart_sequence
                .is_none()
        );
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
            PetFxKind::Heart,
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
                heart_origin: (kind == PetFxKind::Heart).then_some(Vec2::ZERO),
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
            count += cooldown.admit(PetFxKind::Heart, Duration::from_millis(i * 250), count);
            assert!(count <= MAX_PARTICLES);
        }
        assert_eq!(count, MAX_PARTICLES);
        assert_eq!(
            cooldown.admit(PetFxKind::CloudPuff, Duration::from_secs(3000), count),
            0
        );
        assert_eq!(
            cooldown.admit(PetFxKind::Heart, Duration::from_secs(3000), 0),
            1
        );
        assert_eq!(
            cooldown.admit(PetFxKind::Heart, Duration::from_secs(3000), 3),
            0
        );
    }
}
