mod environment;
mod platform;

#[cfg(target_os = "windows")]
mod tray;

use environment::{
    AvatarAnchor,
    DesktopEnvironment,
    DesktopEnvironmentPlugin,
    PlacementState,
};

use std::{
    num::NonZeroU32,
    time::{Duration, Instant},
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
use tray::TrayPlugin;

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
const SITTING_VRMA_PATH: &str = "animations/sitting.vrma";

/// VrmBone `foot` is located around the ankle; shift the standing support
/// plane down to the visible sole in logical window pixels.
const FOOT_SOLE_OFFSET_LOGICAL: f32 = 12.0;

/// Local/world-space offset from the VRM hips bone toward the point that should
/// visually touch a sitting surface. Tune this after `sitting.vrma` is wired.
/// +Y is up; the small +Z term moves the anchor slightly toward Nera's rear in
/// the current avatar orientation.
const SEAT_OFFSET_WORLD: Vec3 = Vec3::new(0.0, -0.12, 0.08);

// -----------------------------------------------------------------------------
// Mate-Engine-style sitting interaction tuning
// -----------------------------------------------------------------------------

/// Mate Engine X3 waits one second before window sitting is armed. This keeps
/// quick desktop movement from accidentally turning into a sitting action.
const MIN_DRAG_HOLD_TO_SIT: Duration = Duration::from_millis(550);

/// Cursor must have actually moved before sitting can arm.
const MIN_DRAG_PIXELS_TO_SIT_AT_96_DPI: f32 = 4.0;

/// Once sitting has been committed, temporarily lock the attachment so the
/// same mouse hold cannot immediately pull Nera off the surface again.
const SIT_MOVEMENT_LOCK: Duration = Duration::from_millis(500);

/// Pulling this far away while already sitting breaks the attachment and gives
/// the cursor ownership of Nera again.
const UNSNAP_DRAG_DISTANCE_AT_96_DPI: f32 = 24.0;

/// Short time-based safety after leaving a sitting surface.
const UNSNAP_COOLDOWN: Duration = Duration::from_millis(300);

/// Spatial guard around the old hip/seat probe. Nera must leave this zone
/// before the old surface can immediately catch her again.
const UNSNAP_GUARD_RADIUS_AT_96_DPI: f32 = 240.0;

/// Extra vertical separation requirement for the current drag after unsnapping.
const UNSNAP_VERTICAL_BAND_AT_96_DPI: f32 = 24.0;

// -----------------------------------------------------------------------------
// Resources & Components
// -----------------------------------------------------------------------------

#[derive(Resource, Default)]
struct DesktopPetInteraction {
    hovering_nera: bool,

    /// Grab point inside Nera's native window. Some means the mouse is holding
    /// Nera, even while an attachment temporarily keeps the HWND pinned.
    drag_offset: Option<IVec2>,
    drag_start_cursor: Option<IVec2>,
    drag_started_at: Option<Instant>,

    /// Cursor position captured when a sitting attachment is established. A
    /// meaningful movement away from it unsnaps Nera without doing Seat->Feet
    /// HWND rebasing first.
    snap_cursor: Option<IVec2>,

    /// Short post-sit input lock. Until this expires, an attached Nera cannot
    /// be pulled off the surface even if LMB remains held.
    sitting_lock_until: Option<Instant>,

    /// Mate-style anti-resnap safety.
    unsnap_cooldown_until: Option<Instant>,
    guard_center: Option<IVec2>,
    recent_unsnap_surface_y: Option<i32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NeraPose {
    #[default]
    Idle,
    Sitting,
}

#[derive(Resource, Default)]
struct NeraAnimations {
    idle: Option<Entity>,
    sitting: Option<Entity>,
    current: Option<NeraPose>,
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
    let mut app = App::new();

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Nera".into(),
                    transparent: true,
                    decorations: false,
                    window_level: WindowLevel::AlwaysOnTop,
                    resolution: (600, 950).into(),
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
    );

    app.add_plugins((VrmPlugin, VrmaPlugin, DesktopEnvironmentPlugin,));

    #[cfg(target_os = "windows")]
    app.add_plugins(TrayPlugin);

    app.insert_resource(ClearColor(Color::NONE))
        .init_resource::<DesktopPetInteraction>()
        .init_resource::<NeraAnimations>()
        .add_systems(
            Startup,
            (setup_camera, setup_lighting, spawn_nera),
        )
        .add_systems(
            Update,
            (
                attach_bone_colliders,
                update_look_at_target,
                // Placement must change before animation state is selected.
                // This lets one click immediately perform Sitting -> Idle and
                // one release immediately perform Idle -> Sitting.
                drag_nera_window,
                sync_nera_pose,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            update_click_through.after(VrmSystemSets::SpringBone),
        )
        // Mate-style pose-first calibration: sample the newly evaluated sitting
        // pose, then commit a pending attachment one frame later using Seat.
        .add_systems(
            Last,
            (
                update_avatar_anchors,
                settle_pending_sit,
            )
                .chain(),
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
        Transform::from_xyz(0.0, 0.8, 4.5).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
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
            Transform::from_xyz(0.0, -0.85, 0.0),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Name::new("Nera Idle Animation"),
                    VrmaHandle(asset_server.load(IDLE_VRMA_PATH)),
                ))
                .observe(register_idle_animation);

            parent
                .spawn((
                    Name::new("Nera Sitting Animation"),
                    VrmaHandle(asset_server.load(SITTING_VRMA_PATH)),
                ))
                .observe(register_sitting_animation);
        });
}

fn register_idle_animation(
    trigger: On<LoadedVrma>,
    mut commands: Commands,
    mut animations: ResMut<NeraAnimations>,
) {
    animations.idle = Some(trigger.vrma);

    // Idle is the startup pose. Do this once; later changes are handled by
    // `sync_nera_pose`.
    if animations.current.is_none() {
        commands.trigger(PlayVrma {
            repeat: RepeatAnimation::Forever,
            transition_duration: Duration::ZERO,
            vrma: trigger.vrma,
            reset_spring_bones: false,
        });

        animations.current = Some(NeraPose::Idle);
        info!("Idle VRMA loaded. Starting animation.");
    }
}

fn register_sitting_animation(
    trigger: On<LoadedVrma>,
    mut animations: ResMut<NeraAnimations>,
) {
    animations.sitting = Some(trigger.vrma);
    info!("Sitting VRMA loaded.");
}

fn sync_nera_pose(
    mut commands: Commands,
    mut environment: ResMut<DesktopEnvironment>,
    mut animations: ResMut<NeraAnimations>,
) {
    let desired = match environment.placement {
        PlacementState::Free => NeraPose::Idle,
        PlacementState::PendingWindow { .. }
        | PlacementState::PendingTaskbar { .. }
        | PlacementState::Window(_)
        | PlacementState::Taskbar(_) => NeraPose::Sitting,
    };

    let anchor = match desired {
        NeraPose::Idle => AvatarAnchor::Feet,
        NeraPose::Sitting => AvatarAnchor::Seat,
    };

    if animations.current == Some(desired) {
        environment.set_attached_anchor(anchor);
        return;
    }

    let vrma = match desired {
        NeraPose::Idle => animations.idle,
        NeraPose::Sitting => animations.sitting,
    };

    let Some(vrma) = vrma else {
        if desired == NeraPose::Idle {
            environment.set_attached_anchor(AvatarAnchor::Feet);
        }
        return;
    };

    // Deterministic pose-first alignment is more important here than blending.
    // Once the mechanics are stable we can add visual smoothing without letting
    // an in-progress blend drive native HWND geometry.
    commands.trigger(PlayVrma {
        repeat: RepeatAnimation::Forever,
        transition_duration: Duration::ZERO,
        vrma,
        reset_spring_bones: false,
    });

    environment.set_attached_anchor(anchor);

    match desired {
        NeraPose::Idle => info!("Nera pose -> Idle | anchor -> Feet"),
        NeraPose::Sitting => info!("Nera pose -> Sitting | pose-first Seat calibration"),
    }

    animations.current = Some(desired);
}

// -----------------------------------------------------------------------------
// LookAt Target Tracker
// -----------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn update_look_at_target(
    environment: Res<DesktopEnvironment>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    mut target_query: Query<&mut Transform, With<LookAtTarget>>,
) {
    let Some(window) =
        window_query.iter().next()
    else {
        return;
    };

    let Some(mut target_transform) =
        target_query.iter_mut().next()
    else {
        return;
    };

    let Some(cursor_pos) =
        cursor_position_in_nera_window(&environment)
    else {
        return;
    };

    let map_x =
        (cursor_pos.x / window.resolution.width() - 0.5)
            * 4.0;

    let map_y =
        (0.5 - cursor_pos.y / window.resolution.height())
            * 4.0
            + 0.5;

    target_transform.translation =
        Vec3::new(map_x, map_y, 2.0);
}

// -----------------------------------------------------------------------------
// Windows Cursor & Mathematics
// -----------------------------------------------------------------------------



#[cfg(target_os = "windows")]
fn cursor_position_in_nera_window(
    environment: &DesktopEnvironment,
) -> Option<Vec2> {
    let global_cursor =
        platform::global_cursor_position()?;

    let rect =
        environment.nera_rect?;

    let local_x =
        global_cursor.x - rect.left;

    let local_y =
        global_cursor.y - rect.top;

    let scale =
        environment.nera_scale_factor as f32;

    if scale <= 0.0 {
        return None;
    }

    Some(Vec2::new(
        local_x as f32 / scale,
        local_y as f32 / scale,
    ))
}

// -----------------------------------------------------------------------------
// Phase 4 avatar anchors
// -----------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn update_avatar_anchors(
    interaction: Res<DesktopPetInteraction>,
    mut environment: ResMut<DesktopEnvironment>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    bone_query: Query<(&VrmBone, &GlobalTransform)>,
) {
    // Seat/hips must keep updating even during a free drag because it is the
    // live sitting probe. Feet, on the other hand, stays cached from the last
    // standing frame so dangling feet from sitting.vrma never become the
    // standing support anchor.
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };

    let mut feet = Vec::with_capacity(2);
    let mut seat = None;

    for (bone, transform) in &bone_query {
        match bone.as_str() {
            "leftFoot" | "rightFoot" => {
                if let Ok(viewport) = camera.world_to_viewport(
                    camera_transform,
                    transform.translation(),
                ) {
                    feet.push(viewport);
                }
            }

            "hips" => {
                // The hips bone is the pelvis center, not the actual contact
                // point with a chair/window edge. Offset it slightly down/back
                // to approximate the seat/butt support point.
                let seat_world = transform.transform_point(SEAT_OFFSET_WORLD);

                if let Ok(viewport) = camera.world_to_viewport(
                    camera_transform,
                    seat_world,
                ) {
                    seat = Some(viewport);
                }
            }

            _ => {}
        }
    }

    // Feet means *standing support*. Do not overwrite it with the dangling
    // feet from sitting.vrma. That cached idle anchor is what allows a clean
    // Seat -> Feet handoff when Nera is unpinned.
    if environment.placement == PlacementState::Free
        && interaction.drag_offset.is_none()
        && !feet.is_empty()
    {
        let x =
            feet.iter().map(|point| point.x).sum::<f32>() / feet.len() as f32;

        let y = feet
            .iter()
            .map(|point| point.y)
            .fold(f32::NEG_INFINITY, f32::max)
            + FOOT_SOLE_OFFSET_LOGICAL;

        environment.set_feet_anchor(Vec2::new(x, y));
    }

    // Seat is pose-dependent and should follow the evaluated hips while Nera
    // is attached/sitting. It is harmless to refresh it while Free as well.
    if let Some(seat) = seat {
        environment.set_seat_anchor(seat);
    }
}

#[cfg(target_os = "windows")]
fn settle_pending_sit(
    mut interaction: ResMut<DesktopPetInteraction>,
    mut environment: ResMut<DesktopEnvironment>,
) {
    if !environment.settle_pending_sit() {
        return;
    }

    interaction.snap_cursor = platform::global_cursor_position()
        .map(|point| IVec2::new(point.x, point.y));

    interaction.sitting_lock_until =
        Some(Instant::now() + SIT_MOVEMENT_LOCK);

    info!(
        "Nera sitting movement locked for {} ms",
        SIT_MOVEMENT_LOCK.as_millis(),
    );

    // A successful sit consumes the old unsnap guard, matching Mate Engine's
    // behavior when a new target is accepted.
    interaction.unsnap_cooldown_until = None;
    interaction.guard_center = None;
    interaction.recent_unsnap_surface_y = None;
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
    environment: Res<DesktopEnvironment>,
    mut interaction: ResMut<DesktopPetInteraction>,
    mut window_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    colliders_query: Query<(&GlobalTransform, &NeraCollider)>,
    bone_query: Query<(&VrmBone, &GlobalTransform)>,
) {
    let Ok( mut cursor_options) = window_query.single_mut() else { return };
    let Ok((camera, camera_transform)) = camera_query.single() else { return };

    let mut hovering = false;

    if let Some(cursor_pos) = cursor_position_in_nera_window(&environment) {
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

#[cfg(target_os = "windows")]
fn drag_nera_window(
    mouse: Res<ButtonInput<MouseButton>>,
    mut interaction: ResMut<DesktopPetInteraction>,
    mut environment: ResMut<DesktopEnvironment>,
) {
    let Some(hwnd) = environment.nera_hwnd else {
        return;
    };

    let Some(global_cursor) = platform::global_cursor_position() else {
        return;
    };

    let cursor = IVec2::new(global_cursor.x, global_cursor.y);
    let now = Instant::now();

    // Mouse-down starts a HOLD. If Nera is already sitting we deliberately keep
    // the attachment intact until the cursor meaningfully pulls away.
    if interaction.hovering_nera && mouse.just_pressed(MouseButton::Left) {
        if let Some(rect) = environment.nera_rect {
            interaction.drag_offset = Some(IVec2::new(
                global_cursor.x - rect.left,
                global_cursor.y - rect.top,
            ));
            interaction.drag_start_cursor = Some(cursor);
            interaction.drag_started_at = Some(now);

            if matches!(
                environment.placement,
                PlacementState::Window(_) | PlacementState::Taskbar(_)
            ) {
                interaction.snap_cursor = Some(cursor);
            }
        }
    }

    if mouse.pressed(MouseButton::Left) && interaction.drag_offset.is_some() {
        // ------------------------------------------------------------------
        // Attached -> user pulls away -> unsnap without Seat->Feet rebase.
        // ------------------------------------------------------------------
        if matches!(
            environment.placement,
            PlacementState::Window(_) | PlacementState::Taskbar(_)
        ) {
            let unsnap_distance = (
                UNSNAP_DRAG_DISTANCE_AT_96_DPI
                    * environment.nera_scale_factor as f32
            )
                .round()
                .max(1.0) as i32;

            let sitting_locked = interaction
                .sitting_lock_until
                .map(|until| now < until)
                .unwrap_or(false);

            let moved_away = interaction
                .snap_cursor
                .map(|origin| {
                    let delta = cursor - origin;
                    delta.length_squared() >= unsnap_distance * unsnap_distance
                })
                .unwrap_or(false);

            if moved_away && !sitting_locked {
                // Capture the old probe/surface before breaking the attachment.
                interaction.guard_center = environment
                    .seat_probe_global()
                    .map(|point| IVec2::new(point.x, point.y));
                interaction.recent_unsnap_surface_y =
                    environment.current_attachment_surface_y();
                interaction.unsnap_cooldown_until = Some(now + UNSNAP_COOLDOWN);

                environment.begin_nera_drag();
                interaction.snap_cursor = None;
            }
        }

        // ------------------------------------------------------------------
        // Free drag -> native HWND follows cursor.
        // Pending/attached states intentionally keep the HWND pinned.
        // ------------------------------------------------------------------
        if environment.placement == PlacementState::Free {
            if let Some(offset) = interaction.drag_offset {
                let new_position = platform::PhysicalPoint::new(
                    global_cursor.x - offset.x,
                    global_cursor.y - offset.y,
                );

                if platform::move_window(hwnd, new_position).is_ok() {
                    environment.nera_rect = platform::window_rect(hwnd);
                }
            }

            // Mate Engine arms sitting only after a deliberate one-second hold
            // AND at least a tiny real movement from the drag start.
            let hold_ready = interaction
                .drag_started_at
                .map(|started| now.duration_since(started) >= MIN_DRAG_HOLD_TO_SIT)
                .unwrap_or(false);

            let min_drag = (
                MIN_DRAG_PIXELS_TO_SIT_AT_96_DPI
                    * environment.nera_scale_factor as f32
            )
                .round()
                .max(1.0) as i32;

            let moved_for_sit = interaction
                .drag_start_cursor
                .map(|origin| {
                    let delta = cursor - origin;
                    delta.x.abs() >= min_drag || delta.y.abs() >= min_drag
                })
                .unwrap_or(false);

            let cooldown_blocked = interaction
                .unsnap_cooldown_until
                .map(|until| now < until)
                .unwrap_or(false);

            let mut guard_blocked = false;
            let mut vertical_blocked = false;

            if let Some(probe) = environment.seat_probe_global() {
                let probe = IVec2::new(probe.x, probe.y);

                if let Some(center) = interaction.guard_center {
                    let guard_radius = (
                        UNSNAP_GUARD_RADIUS_AT_96_DPI
                            * environment.nera_scale_factor as f32
                    )
                        .round()
                        .max(1.0) as i32;

                    let delta = probe - center;
                    guard_blocked = delta.length_squared()
                        < guard_radius * guard_radius;

                    if !guard_blocked {
                        interaction.guard_center = None;
                    }
                }

                if let Some(surface_y) = interaction.recent_unsnap_surface_y {
                    let vertical_band = (
                        UNSNAP_VERTICAL_BAND_AT_96_DPI
                            * environment.nera_scale_factor as f32
                    )
                        .round()
                        .max(1.0) as i32;

                    vertical_blocked = (probe.y - surface_y).abs() < vertical_band;

                    if !vertical_blocked {
                        interaction.recent_unsnap_surface_y = None;
                    }
                }
            }

            if hold_ready
                && moved_for_sit
                && !cooldown_blocked
                && !guard_blocked
                && !vertical_blocked
            {
                // IMPORTANT: this does NOT move the HWND. It only creates a
                // Pending* placement. sync_nera_pose switches to sitting first;
                // Last then samples Seat and commits the attachment later.
                environment.try_begin_sit_from_probe();
            }
        }
    }

    if mouse.just_released(MouseButton::Left) {
        // Release never performs snap detection anymore. Sitting is acquired
        // while dragging, after the hold/probe rules above have been satisfied.
        interaction.drag_offset = None;
        interaction.drag_start_cursor = None;
        interaction.drag_started_at = None;
        interaction.snap_cursor = None;

        // Mate's vertical recent-unsnap check is only relevant to the current
        // drag. The spatial guard is intentionally retained until Nera leaves it.
        interaction.recent_unsnap_surface_y = None;

        if interaction
            .unsnap_cooldown_until
            .map(|until| now >= until)
            .unwrap_or(false)
        {
            interaction.unsnap_cooldown_until = None;
        }
    }
}
