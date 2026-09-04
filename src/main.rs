use std::{
    num::NonZeroU32,
    time::Duration,
};

use bevy::{
    animation::RepeatAnimation,
    prelude::*,
    render::{
        RenderPlugin,
        settings::{MemoryHints, WgpuSettings},
    },
    window::{
        CursorOptions,
        PresentMode,
        PrimaryWindow,
        WindowLevel,
        WindowPlugin,
        WindowPosition,
    },
};

use bevy_vrm1::prelude::*;

#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::POINT,
    UI::WindowsAndMessaging::GetCursorPos,
};

// -----------------------------------------------------------------------------
// Nera - Phase 2 (Final: Manual Drag + LookAt + Analytic Bone Colliders)
// -----------------------------------------------------------------------------

//NERA PHASE 2

//Window
//├─ Transparent          ✅
//├─ Borderless           ✅
//├─ Always on top        ✅
//├─ Skip taskbar         ✅
//└─ Manual drag          ✅

//Avatar
//├─ VRM 1.0              ✅
//├─ VRMA idle            ✅
//├─ SpringBone           ✅
//└─ LookAt global cursor ✅

//Interaction
//├─ Global cursor        ✅
//├─ Sphere hitboxes      ✅
//├─ Capsule hitboxes     ✅
//├─ Animated bone follow ✅
//├─ Click-through        ✅
//└─ Drag lock            ✅

const VRM_PATH: &str = "vrm/nera.vrm";
const IDLE_VRMA_PATH: &str = "animations/idle.vrma";

// -----------------------------------------------------------------------------
// Resources & Components
// -----------------------------------------------------------------------------

#[derive(Resource, Default)]
struct DesktopPetInteraction {
    hovering_nera: bool,
    drag_offset: Option<IVec2>,
}

#[derive(Component)]
struct LookAtTarget;

#[derive(Component)]
enum NeraCollider {
    Sphere {
        radius: f32,
    },
    Capsule {
        radius: f32,
        end_bone: &'static str,
    },
}

// -----------------------------------------------------------------------------
// Main
// -----------------------------------------------------------------------------

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Nera".into(),
                        transparent: true,
                        decorations: false,
                        window_level: WindowLevel::AlwaysOnTop,
                        resolution: (600, 800).into(),
                        resizable: false,
                        position: WindowPosition::At(IVec2::new(200, 200)),
                        present_mode: PresentMode::AutoVsync,
                        desired_maximum_frame_latency: NonZeroU32::new(1),
                        has_shadow: false,
                        skip_taskbar: true,
                        ..default()
                    }),
                    primary_cursor_options: Some(CursorOptions {
                        hit_test: false,
                        ..default()
                    }),
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: WgpuSettings {
                        memory_hints: MemoryHints::MemoryUsage,
                        ..default()
                    }
                    .into(),
                    ..default()
                }),
        )
        .add_plugins((VrmPlugin, VrmaPlugin))
        .insert_resource(ClearColor(Color::NONE))
        .init_resource::<DesktopPetInteraction>()
        .add_systems(
            Startup,
            (setup_camera, setup_lighting, spawn_nera),
        )
        .add_systems(
            Update,
            (
                attach_bone_colliders,
                update_look_at_target,
                drag_nera_window,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            update_click_through.after(VrmSystemSets::SpringBone),
        )
        .run();
}

// -----------------------------------------------------------------------------
// Camera & Lighting
// -----------------------------------------------------------------------------

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Name::new("Nera Camera"),
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        Msaa::Off,
        Transform::from_xyz(0.0, 1.0, 4.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
    ));
}

fn setup_lighting(mut commands: Commands) {
    commands.spawn((
        Name::new("Nera Key Light"),
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: false,
            contact_shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.7, -0.5, 0.0)),
    ));
}

// -----------------------------------------------------------------------------
// Nera Spawning
// -----------------------------------------------------------------------------

fn spawn_nera(mut commands: Commands, asset_server: Res<AssetServer>) {
    info!("Loading Nera VRM 1.0...");

    let target = commands
        .spawn((
            Name::new("LookAt Target"),
            LookAtTarget,
            Transform::from_xyz(0.0, 1.0, 2.0),
        ))
        .id();

    commands
        .spawn((
            Name::new("Nera"),
            VrmHandle(asset_server.load(VRM_PATH)),
            LookAt::Target(target),
            Transform::from_xyz(0.0, -1.0, 0.0),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Name::new("Nera Idle Animation"),
                    VrmaHandle(asset_server.load(IDLE_VRMA_PATH)),
                ))
                .observe(play_idle_animation);
        });
}

fn play_idle_animation(trigger: On<LoadedVrma>, mut commands: Commands) {
    commands.trigger(PlayVrma {
        repeat: RepeatAnimation::Forever,
        transition_duration: Duration::ZERO,
        vrma: trigger.vrma,
        reset_spring_bones: false,
    });
}

// -----------------------------------------------------------------------------
// LookAt Target Tracker
// -----------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn update_look_at_target(
    window_query: Query<&Window, With<PrimaryWindow>>,
    mut target_query: Query<&mut Transform, With<LookAtTarget>>,
) {
    let Some(window) = window_query.iter().next() else { return };
    let Some(mut target_transform) = target_query.iter_mut().next() else { return };
    let Some(cursor_pos) = cursor_position_in_window(window) else { return };

    let map_x = (cursor_pos.x / window.resolution.width() - 0.5) * 4.0;
    let map_y = (0.5 - cursor_pos.y / window.resolution.height()) * 4.0 + 0.5;

    target_transform.translation = Vec3::new(map_x, map_y, 2.0);
}

// -----------------------------------------------------------------------------
// Windows Cursor & Mathematics
// -----------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn global_cursor_position() -> Option<IVec2> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).ok()?; }
    Some(IVec2::new(point.x, point.y))
}

#[cfg(target_os = "windows")]
fn cursor_position_in_window(window: &Window) -> Option<Vec2> {
    let global_cursor = global_cursor_position()?;
    let WindowPosition::At(window_position) = window.position else { return None };

    let local_physical = global_cursor - window_position;
    let scale_factor = window.scale_factor();

    Some(Vec2::new(
        local_physical.x as f32 / scale_factor,
        local_physical.y as f32 / scale_factor,
    ))
}

fn ray_intersects_sphere(ray_origin: Vec3, ray_dir: Vec3, center: Vec3, radius: f32) -> bool {
    let oc = ray_origin - center;
    let b = oc.dot(ray_dir);
    let c = oc.length_squared() - radius * radius;
    let discriminant = b * b - c;

    if discriminant >= 0.0 {
        let sqrt_d = discriminant.sqrt();
        return (-b - sqrt_d) >= 0.0 || (-b + sqrt_d) >= 0.0;
    }
    false
}

fn ray_intersects_capsule(ray_origin: Vec3, ray_dir: Vec3, pa: Vec3, pb: Vec3, radius: f32) -> bool {
    let ba = pb - pa;
    let oa = ray_origin - pa;

    let baba = ba.dot(ba);
    let bard = ba.dot(ray_dir);
    let baoa = ba.dot(oa);
    let rdoa = ray_dir.dot(oa);
    let oaoa = oa.dot(oa);

    let a = baba - bard * bard;
    let b = baba * rdoa - baoa * bard;
    let c = baba * oaoa - baoa * baoa - radius * radius * baba;

    if a.abs() < 1e-6 {
        return ray_intersects_sphere(ray_origin, ray_dir, pa, radius)
            || ray_intersects_sphere(ray_origin, ray_dir, pb, radius);
    }

    let h = b * b - a * c;

    if h < 0.0 {
        return false;
    }

    let t = (-b - h.sqrt()) / a;
    let y = baoa + t * bard;

    if y > 0.0 && y < baba && t >= 0.0 {
        return true;
    }

    ray_intersects_sphere(ray_origin, ray_dir, pa, radius)
        || ray_intersects_sphere(ray_origin, ray_dir, pb, radius)
}

// -----------------------------------------------------------------------------
// Click Through & Bone Colliders
// -----------------------------------------------------------------------------

fn attach_bone_colliders(
    mut commands: Commands,
    bones: Query<(Entity, &VrmBone), Added<VrmBone>>,
) {
    for (entity, bone) in &bones {
        let collider = match bone.as_str() {
            "head" => Some(NeraCollider::Sphere { radius: 0.18 }),
            "hips" => Some(NeraCollider::Capsule { radius: 0.18, end_bone: "spine" }),
            "spine" => Some(NeraCollider::Capsule { radius: 0.19, end_bone: "chest" }),
            "chest" => Some(NeraCollider::Capsule { radius: 0.20, end_bone: "upperChest" }),
            "upperChest" => Some(NeraCollider::Capsule { radius: 0.18, end_bone: "neck" }),
            
            "leftShoulder" => Some(NeraCollider::Capsule { radius: 0.10, end_bone: "leftUpperArm" }),
            "leftUpperArm" => Some(NeraCollider::Capsule { radius: 0.10, end_bone: "leftLowerArm" }),
            "leftLowerArm" => Some(NeraCollider::Capsule { radius: 0.09, end_bone: "leftHand" }),
            
            "rightShoulder" => Some(NeraCollider::Capsule { radius: 0.10, end_bone: "rightUpperArm" }),
            "rightUpperArm" => Some(NeraCollider::Capsule { radius: 0.10, end_bone: "rightLowerArm" }),
            "rightLowerArm" => Some(NeraCollider::Capsule { radius: 0.09, end_bone: "rightHand" }),
            
            "leftHand" | "rightHand" => Some(NeraCollider::Sphere { radius: 0.10 }),
            
            "leftUpperLeg" => Some(NeraCollider::Capsule { radius: 0.13, end_bone: "leftLowerLeg" }),
            "leftLowerLeg" => Some(NeraCollider::Capsule { radius: 0.11, end_bone: "leftFoot" }),
            
            "rightUpperLeg" => Some(NeraCollider::Capsule { radius: 0.13, end_bone: "rightLowerLeg" }),
            "rightLowerLeg" => Some(NeraCollider::Capsule { radius: 0.11, end_bone: "rightFoot" }),
            
            "leftFoot" | "rightFoot" => Some(NeraCollider::Sphere { radius: 0.11 }),
            _ => None,
        };

        if let Some(col) = collider {
            commands.entity(entity).insert(col);
        }
    }
}

#[cfg(target_os = "windows")]
fn update_click_through(
    mut interaction: ResMut<DesktopPetInteraction>,
    mut window_query: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    colliders_query: Query<(&GlobalTransform, &NeraCollider)>,
    bone_query: Query<(&VrmBone, &GlobalTransform)>,
) {
    let Ok((window, mut cursor_options)) = window_query.single_mut() else { return };
    let Ok((camera, camera_transform)) = camera_query.single() else { return };

    let mut hovering = false;

    if let Some(cursor_pos) = cursor_position_in_window(window) {
        if let Ok(ray) = camera.viewport_to_world(camera_transform, cursor_pos) {
            let ray_origin = ray.origin;
            let ray_dir = ray.direction.into();

            for (bone_transform, collider) in colliders_query.iter() {
                let pa = bone_transform.translation();

                let hit = match collider {
                    NeraCollider::Sphere { radius } => {
                        ray_intersects_sphere(ray_origin, ray_dir, pa, *radius)
                    }
                    NeraCollider::Capsule { radius, end_bone } => {
                        let pb = bone_query.iter().find_map(|(bone, transform)| {
                            if bone.as_str() == *end_bone {
                                Some(transform.translation())
                            } else {
                                None
                            }
                        });

                        if let Some(pb) = pb {
                            ray_intersects_capsule(ray_origin, ray_dir, pa, pb, *radius)
                        } else {
                            ray_intersects_sphere(ray_origin, ray_dir, pa, *radius)
                        }
                    }
                };

                if hit {
                    hovering = true;
                    break;
                }
            }
        }
    }

    if interaction.drag_offset.is_some() {
        hovering = true;
    }

    if hovering == interaction.hovering_nera {
        return;
    }

    interaction.hovering_nera = hovering;
    cursor_options.hit_test = hovering;
}

// -----------------------------------------------------------------------------
// Window Dragging
// -----------------------------------------------------------------------------

fn drag_nera_window(
    mouse: Res<ButtonInput<MouseButton>>,
    mut interaction: ResMut<DesktopPetInteraction>,
    mut window_query: Query<&mut Window, With<PrimaryWindow>>,
) {
    let Ok(mut window) = window_query.single_mut() else { return };
    let Some(global_cursor) = global_cursor_position() else { return };

    if interaction.hovering_nera && mouse.just_pressed(MouseButton::Left) {
        if let WindowPosition::At(window_pos) = window.position {
            interaction.drag_offset = Some(global_cursor - window_pos);
        }
    }

    if mouse.just_released(MouseButton::Left) {
        interaction.drag_offset = None;
    }

    if let Some(offset) = interaction.drag_offset {
        window.position = WindowPosition::At(global_cursor - offset);
    }
}