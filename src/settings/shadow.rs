//! Desktop-friendly graphics glue for Project Nera.
//!
//! Shadow modes:
//! - Simple: a cheap layered ellipse under the standing feet.
//! - Character: the actual animated VRM casts a real Bevy shadow map onto a
//!   transparent "shadow catcher" plane. The catcher shader outputs only the
//!   shadow contribution, so the Windows desktop still shows through.
//!
//! This follows the same high-level idea used by Mate Engine's ShadowOnly
//! receiver instead of reconstructing a silhouette from humanoid bones.

use bevy::{
    light::NotShadowCaster,
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
};
use bevy_vrm1::prelude::VrmBone;

use crate::{
    desktop_physics::DesktopPhysicsState,
    environment::{DesktopEnvironment, PlacementState},
    interaction::{DesktopPetInteraction, InteractionState, NeraLiveBone},
};

use super::NeraSettings;

const BASE_LIGHT_ILLUMINANCE: f32 = 10_000.0;
const SHADOW_ONLY_SHADER: &str = "shaders/nera_shadow_only.wgsl";

const SIMPLE_SHADOW_Y: f32 = -0.92;
const SIMPLE_SHADOW_Z: f32 = -0.12;

/// Large enough to catch a long directional silhouette while remaining cheap:
/// it is still just one quad and fully transparent outside the actual shadow.
const CHARACTER_RECEIVER_SIZE: f32 = 7.0;
const CHARACTER_RECEIVER_GROUND_EPSILON: f32 = 0.018;

#[derive(Component)]
struct SimpleShadowLayer {
    material: Handle<StandardMaterial>,
    edge: f32,
    alpha_weight: f32,
}

/// StandardMaterial gives the extension access to Bevy's normal PBR lighting
/// and shadow sampling. The extension converts that result into black alpha
/// only where the receiver is actually shadowed.
type ShadowOnlyMaterial = ExtendedMaterial<StandardMaterial, ShadowOnlyExtension>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
struct ShadowOnlyExtension {
    #[uniform(100)]
    opacity: f32,
}

impl MaterialExtension for ShadowOnlyExtension {
    fn fragment_shader() -> ShaderRef {
        SHADOW_ONLY_SHADER.into()
    }
}

#[derive(Component)]
struct CharacterShadowReceiver {
    material: Handle<ShadowOnlyMaterial>,
}

pub(super) struct NeraShadowPlugin;

impl Plugin for NeraShadowPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ShadowOnlyMaterial>::default())
            .add_systems(
                Startup,
                (spawn_simple_shadow, spawn_character_shadow_receiver),
            )
            .add_systems(
                Update,
                (
                    apply_simple_shadow,
                    apply_character_shadow_receiver,
                    apply_light_settings,
                ),
            );
    }
}

fn transparent_shadow_material() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgba(0.0, 0.0, 0.0, 0.0),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    }
}

fn spawn_simple_shadow(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // One circle mesh is shared by all rings. Slight Z separation keeps alpha
    // sorting deterministic while the layered rings fake a soft edge cheaply.
    let mesh = meshes.add(Circle::new(1.0));

    for (index, edge, alpha_weight) in [
        (0usize, 0.0f32, 0.62f32),
        (1usize, 0.55f32, 0.27f32),
        (2usize, 1.0f32, 0.11f32),
    ] {
        let material = materials.add(transparent_shadow_material());

        commands.spawn((
            Name::new(format!("Nera Simple Shadow {index}")),
            SimpleShadowLayer {
                material: material.clone(),
                edge,
                alpha_weight,
            },
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_xyz(
                0.04,
                SIMPLE_SHADOW_Y,
                SIMPLE_SHADOW_Z - index as f32 * 0.002,
            ),
            Visibility::Hidden,
        ));
    }
}

fn spawn_character_shadow_receiver(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ShadowOnlyMaterial>>,
) {
    let mesh = meshes.add(
        Plane3d::default()
            .mesh()
            .size(CHARACTER_RECEIVER_SIZE, CHARACTER_RECEIVER_SIZE),
    );

    let material = materials.add(ExtendedMaterial {
        base: StandardMaterial {
            // The shader uses the standard PBR lighting result only as a
            // shadow mask. The white base makes lit pixels easy to reject.
            base_color: Color::WHITE,
            perceptual_roughness: 1.0,
            metallic: 0.0,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        },
        extension: ShadowOnlyExtension { opacity: 0.4 },
    });

    commands.spawn((
        Name::new("Nera Character Shadow Receiver"),
        CharacterShadowReceiver {
            material: material.clone(),
        },
        Mesh3d(mesh),
        MeshMaterial3d(material),
        // The invisible catcher receives the VRM shadow but must never put its
        // own huge quad into the directional-light depth map.
        NotShadowCaster,
        Transform::default(),
        Visibility::Hidden,
    ));
}

fn shadow_is_grounded(
    settings: &NeraSettings,
    environment: &DesktopEnvironment,
    physics: &DesktopPhysicsState,
    interaction: &DesktopPetInteraction,
) -> bool {
    settings.shadow_enabled
        && environment.placement == PlacementState::Free
        && interaction.state != InteractionState::Dragging
        && !physics.falling
        && environment.feet_anchor_logical.is_some()
}

fn apply_simple_shadow(
    settings: Res<NeraSettings>,
    environment: Res<DesktopEnvironment>,
    physics: Res<DesktopPhysicsState>,
    interaction: Res<DesktopPetInteraction>,
    bones: Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<crate::avatar::NeraMainCamera>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut layers: Query<(&SimpleShadowLayer, &mut Transform, &mut Visibility)>,
) {
    let should_show = shadow_is_grounded(
        &settings,
        &environment,
        &physics,
        &interaction,
    ) && settings.uses_simple_shadow();

    let opacity = settings.shadow_opacity.clamp(0, 100) as f32 / 100.0;
    let softness = settings.shadow_softness.clamp(25, 100) as f32 / 100.0;
    let character_scale = settings.character_scale();
    let ground = should_show
        .then(|| shadow_ground_with_user_offset(&settings, &bones, &camera_query))
        .flatten();
    let visible = should_show && ground.is_some();

    for (layer, mut transform, mut visibility) in &mut layers {
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };

        // The inner ring stays compact while the outer rings expand as the
        // softness setting rises. Squashing Y makes the circles read as a blob
        // shadow despite being rendered in Nera's normal 3D camera.
        let spread_x = 1.0 + layer.edge * 0.36 * softness;
        let spread_y = 1.0 + layer.edge * 0.95 * softness;
        transform.scale = Vec3::new(
            0.55 * spread_x * character_scale,
            0.075 * spread_y * character_scale,
            1.0,
        );

        if let Some(ground) = ground {
            transform.translation.x = ground.x + 0.04 * character_scale;
            transform.translation.y = ground.y - 0.01;
        }

        if let Some(mut material) = materials.get_mut(&layer.material) {
            material.base_color = Color::srgba(
                0.0,
                0.0,
                0.0,
                (opacity * layer.alpha_weight).clamp(0.0, 1.0),
            );
        }
    }
}

fn bone_position(
    bones: &Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
    name: &str,
) -> Option<Vec3> {
    bones
        .iter()
        .find_map(|(bone, transform)| {
            (bone.as_str() == name).then(|| transform.translation())
        })
}

/// World-space support plane for the real shadow catcher.
///
/// Use the lower of the two animated feet for Y so an idle animation that
/// lifts one foot does not pull the whole receiver into the air. X/Z are kept
/// centered between the feet and the generous plane size catches the cast.
fn character_shadow_ground(
    bones: &Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
) -> Option<Vec3> {
    match (
        bone_position(bones, "leftFoot"),
        bone_position(bones, "rightFoot"),
    ) {
        (Some(left), Some(right)) => Some(Vec3::new(
            (left.x + right.x) * 0.5,
            left.y.min(right.y) - CHARACTER_RECEIVER_GROUND_EPSILON,
            (left.z + right.z) * 0.5,
        )),
        (Some(foot), None) | (None, Some(foot)) => Some(Vec3::new(
            foot.x,
            foot.y - CHARACTER_RECEIVER_GROUND_EPSILON,
            foot.z,
        )),
        (None, None) => None,
    }
}


/// Apply the user-facing Ground Offset to the shadow support plane in render
/// world space. Physics stores that setting in logical screen pixels, while
/// the shadow receiver lives in 3D world units, so convert the requested
/// vertical screen delta using the active 3D camera projection.
///
/// This keeps the desktop support calibration and the visible shadow using the
/// same visual ground reference instead of letting Ground Offset move Nera
/// against a shadow that is still pinned to the raw foot bones.
fn shadow_ground_with_user_offset(
    settings: &NeraSettings,
    bones: &Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
    camera_query: &Query<(&Camera, &GlobalTransform), With<crate::avatar::NeraMainCamera>>,
) -> Option<Vec3> {
    let mut ground = character_shadow_ground(bones)?;
    let offset_pixels = settings.ground_offset_logical();

    if offset_pixels.abs() <= f32::EPSILON {
        return Some(ground);
    }

    let Some((camera, camera_transform)) = camera_query.iter().next() else {
        return Some(ground);
    };

    // Probe a small +Y world displacement around the current feet to derive
    // the local logical-pixels-per-world-unit ratio. This remains correct if
    // the camera projection/window resolution changes later.
    const PROBE_WORLD_Y: f32 = 0.10;
    let Ok(base_viewport) = camera.world_to_viewport(camera_transform, ground) else {
        return Some(ground);
    };
    let Ok(up_viewport) = camera.world_to_viewport(
        camera_transform,
        ground + Vec3::Y * PROBE_WORLD_Y,
    ) else {
        return Some(ground);
    };

    // Viewport Y grows downward. Moving +Y in world space normally decreases
    // viewport Y, so this value is positive for Nera's current camera.
    let pixels_per_world_y =
        (base_viewport.y - up_viewport.y) / PROBE_WORLD_Y;

    if pixels_per_world_y.abs() > 0.001 {
        // Positive Ground Offset means a contact point lower on screen.
        // Therefore the receiver moves toward -Y in world space.
        ground.y -= offset_pixels / pixels_per_world_y;
    }

    Some(ground)
}

fn apply_character_shadow_receiver(
    settings: Res<NeraSettings>,
    environment: Res<DesktopEnvironment>,
    physics: Res<DesktopPhysicsState>,
    interaction: Res<DesktopPetInteraction>,
    bones: Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<crate::avatar::NeraMainCamera>>,
    mut materials: ResMut<Assets<ShadowOnlyMaterial>>,
    mut receivers: Query<(
        &CharacterShadowReceiver,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    let should_show = shadow_is_grounded(
        &settings,
        &environment,
        &physics,
        &interaction,
    ) && settings.uses_character_shadow();

    let ground = if should_show {
        shadow_ground_with_user_offset(&settings, &bones, &camera_query)
    } else {
        None
    };

    let opacity = settings.shadow_opacity.clamp(0, 100) as f32 / 100.0;

    for (receiver, mut transform, mut visibility) in &mut receivers {
        let Some(ground) = ground else {
            *visibility = Visibility::Hidden;
            continue;
        };

        transform.translation = ground;
        transform.rotation = Quat::IDENTITY;
        transform.scale = Vec3::ONE;
        *visibility = Visibility::Visible;

        if let Some(mut material) = materials.get_mut(&receiver.material) {
            material.extension.opacity = opacity;
        }
    }
}

/// One source of truth for the visible key light and real shadow-map cost.
/// Character shadows allocate/render shadow maps only while the receiver is
/// actually useful. Simple mode stays on the cheap fake ellipse path.
fn apply_light_settings(
    settings: Res<NeraSettings>,
    environment: Res<DesktopEnvironment>,
    physics: Res<DesktopPhysicsState>,
    interaction: Res<DesktopPetInteraction>,
    mut lights: Query<&mut DirectionalLight>,
) {
    let multiplier = settings.brightness.clamp(50, 200) as f32 / 100.0;
    let character_shadow_active = shadow_is_grounded(
        &settings,
        &environment,
        &physics,
        &interaction,
    ) && settings.uses_character_shadow();

    for mut light in &mut lights {
        light.illuminance = BASE_LIGHT_ILLUMINANCE * multiplier;
        light.shadow_maps_enabled = character_shadow_active;
        // Contact shadows are screen-space and are not the desktop projection
        // effect we want here. Keep them disabled even in Character mode.
        light.contact_shadows_enabled = false;
    }
}
