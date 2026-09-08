mod animation;
mod environment;
mod interaction;
mod platform;

#[cfg(target_os = "windows")]
mod tray;

use animation::{
    NeraAnimationPlugin,
    apply_animation_state,
    register_idle_animation,
    register_sitting_animation,
    sync_animation_base_pose,
    sync_interaction_animation_actions,
    tick_animation_actions,
};

use environment::{
    AvatarAnchor,
    DesktopEnvironment,
    DesktopEnvironmentPlugin,
    PlacementState,
};

use interaction::{
    BodyPart,
    HoveredBodyPart,
    InteractionPlugin,
    InteractionState,
    NeraClicked,
    NeraDragStarted,
    NeraDragged,
    NeraPointerPosition,
    NeraReleased,
    NeraRightClicked,
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

/// Left mouse press becomes a real drag only after the cursor moves this far.
/// Keeping this DPI-aware also prevents tiny hand jitter from turning a click
/// into a drag. Phase 4 sitting uses the same deliberate-drag threshold.
const DRAG_START_DISTANCE_AT_96_DPI: f32 = 4.0;

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
    state: InteractionState,

    /// Phase 5.2 captures semantic body parts at mouse-down so animation or
    /// cursor movement cannot change a gesture's meaning before release.
    pressed_body_part: Option<BodyPart>,
    right_pressed_body_part: Option<BodyPart>,

    /// Grab point inside Nera's native window. Some means LMB currently owns
    /// the interaction, even while an attachment temporarily keeps the HWND pinned.
    drag_offset: Option<IVec2>,
    drag_start_cursor: Option<IVec2>,
    last_drag_cursor: Option<IVec2>,
    drag_started_at: Option<Instant>,

    /// Capture RMB until release so click-through cannot hand the release to
    /// the window underneath Nera. A small travel guard prevents right-drag
    /// gestures from opening the future Phase 5.7 action menu.
    right_press_origin: Option<IVec2>,

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


#[derive(Component)]
struct LookAtTarget;

#[derive(Component)]
struct NeraCollider {
    body_part: BodyPart,
    shape: NeraColliderShape,
}

#[derive(Clone, Copy)]
enum NeraColliderShape {
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

    app.add_plugins((
        VrmPlugin,
        VrmaPlugin,
        DesktopEnvironmentPlugin,
        InteractionPlugin,
        NeraAnimationPlugin,
    ));

    #[cfg(target_os = "windows")]
    app.add_plugins(TrayPlugin);

    app.insert_resource(ClearColor(Color::NONE))
        .init_resource::<DesktopPetInteraction>()
        .init_resource::<HoveredBodyPart>()
        .add_systems(
            Startup,
            (setup_camera, setup_lighting, spawn_nera),
        )
        .add_systems(
            Update,
            (
                attach_bone_colliders,
                update_look_at_target,
                // Native placement changes first, then Phase 5.3 derives the
                // base pose, consumes temporary interaction actions, resolves
                // priority, and finally applies the correct VRMA/anchor.
                drag_nera_window,
                sync_animation_base_pose,
                sync_interaction_animation_actions,
                tick_animation_actions,
                apply_animation_state,
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

#[cfg(target_os = "windows")]
fn nera_pointer_position(
    screen: IVec2,
    environment: &DesktopEnvironment,
) -> NeraPointerPosition {
    NeraPointerPosition {
        screen,
        local: cursor_position_in_nera_window(environment).unwrap_or_default(),
    }
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
        && interaction.state != InteractionState::Dragging
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

fn ray_sphere_distance(
    ray_origin: Vec3,
    ray_dir: Vec3,
    center: Vec3,
    radius: f32,
) -> Option<f32> {
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

fn nearest_nera_body_part(
    ray_origin: Vec3,
    ray_dir: Vec3,
    colliders_query: &Query<(&GlobalTransform, &NeraCollider)>,
    bone_query: &Query<(&VrmBone, &GlobalTransform)>,
) -> Option<BodyPart> {
    let mut nearest: Option<(BodyPart, f32)> = None;

    for (bone_transform, collider) in colliders_query.iter() {
        let pa = bone_transform.translation();

        let distance = match collider.shape {
            NeraColliderShape::Sphere { radius } => {
                ray_sphere_distance(ray_origin, ray_dir, pa, radius)
            }
            NeraColliderShape::Capsule { radius, end_bone } => {
                let pb = bone_query.iter().find_map(|(bone, transform)| {
                    (bone.as_str() == end_bone).then(|| transform.translation())
                });

                match pb {
                    Some(pb) => ray_capsule_distance(ray_origin, ray_dir, pa, pb, radius),
                    None => ray_sphere_distance(ray_origin, ray_dir, pa, radius),
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

fn attach_bone_colliders(
    mut commands: Commands,
    bones: Query<(Entity, &VrmBone), Added<VrmBone>>,
) {
    use BodyPart::*;

    for (entity, bone) in &bones {
        let collider = match bone.as_str() {
            "head" => Some(NeraCollider {
                body_part: Head,
                shape: NeraColliderShape::Sphere { radius: 0.18 },
            }),

            "hips" => Some(NeraCollider {
                body_part: Torso,
                shape: NeraColliderShape::Capsule { radius: 0.18, end_bone: "spine" },
            }),
            "spine" => Some(NeraCollider {
                body_part: Torso,
                shape: NeraColliderShape::Capsule { radius: 0.19, end_bone: "chest" },
            }),
            "chest" => Some(NeraCollider {
                body_part: Torso,
                shape: NeraColliderShape::Capsule { radius: 0.20, end_bone: "upperChest" },
            }),
            "upperChest" => Some(NeraCollider {
                body_part: Torso,
                shape: NeraColliderShape::Capsule { radius: 0.18, end_bone: "neck" },
            }),

            "leftShoulder" => Some(NeraCollider {
                body_part: LeftArm,
                shape: NeraColliderShape::Capsule { radius: 0.10, end_bone: "leftUpperArm" },
            }),
            "leftUpperArm" => Some(NeraCollider {
                body_part: LeftArm,
                shape: NeraColliderShape::Capsule { radius: 0.10, end_bone: "leftLowerArm" },
            }),
            "leftLowerArm" => Some(NeraCollider {
                body_part: LeftArm,
                shape: NeraColliderShape::Capsule { radius: 0.09, end_bone: "leftHand" },
            }),
            "leftHand" => Some(NeraCollider {
                body_part: LeftHand,
                shape: NeraColliderShape::Sphere { radius: 0.10 },
            }),

            "rightShoulder" => Some(NeraCollider {
                body_part: RightArm,
                shape: NeraColliderShape::Capsule { radius: 0.10, end_bone: "rightUpperArm" },
            }),
            "rightUpperArm" => Some(NeraCollider {
                body_part: RightArm,
                shape: NeraColliderShape::Capsule { radius: 0.10, end_bone: "rightLowerArm" },
            }),
            "rightLowerArm" => Some(NeraCollider {
                body_part: RightArm,
                shape: NeraColliderShape::Capsule { radius: 0.09, end_bone: "rightHand" },
            }),
            "rightHand" => Some(NeraCollider {
                body_part: RightHand,
                shape: NeraColliderShape::Sphere { radius: 0.10 },
            }),

            "leftUpperLeg" => Some(NeraCollider {
                body_part: LeftLeg,
                shape: NeraColliderShape::Capsule { radius: 0.13, end_bone: "leftLowerLeg" },
            }),
            "leftLowerLeg" => Some(NeraCollider {
                body_part: LeftLeg,
                shape: NeraColliderShape::Capsule { radius: 0.11, end_bone: "leftFoot" },
            }),
            "leftFoot" => Some(NeraCollider {
                body_part: LeftFoot,
                shape: NeraColliderShape::Sphere { radius: 0.11 },
            }),

            "rightUpperLeg" => Some(NeraCollider {
                body_part: RightLeg,
                shape: NeraColliderShape::Capsule { radius: 0.13, end_bone: "rightLowerLeg" },
            }),
            "rightLowerLeg" => Some(NeraCollider {
                body_part: RightLeg,
                shape: NeraColliderShape::Capsule { radius: 0.11, end_bone: "rightFoot" },
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

#[cfg(target_os = "windows")]
fn update_click_through(
    environment: Res<DesktopEnvironment>,
    mut interaction: ResMut<DesktopPetInteraction>,
    mut hovered: ResMut<HoveredBodyPart>,
    mut window_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    colliders_query: Query<(&GlobalTransform, &NeraCollider)>,
    bone_query: Query<(&VrmBone, &GlobalTransform)>,
) {
    let Ok(mut cursor_options) = window_query.single_mut() else { return };
    let Ok((camera, camera_transform)) = camera_query.single() else { return };

    let hovered_body_part = cursor_position_in_nera_window(&environment)
        .and_then(|cursor_pos| camera.viewport_to_world(camera_transform, cursor_pos).ok())
        .and_then(|ray| {
            nearest_nera_body_part(
                ray.origin,
                ray.direction.into(),
                &colliders_query,
                &bone_query,
            )
        });

    hovered.part = hovered_body_part;
    let hovering = hovered_body_part.is_some();

    // Once a button has captured Nera, keep receiving input even if animation
    // moves the current collider away from the cursor. The semantic body part
    // was captured at press time and remains stable until release.
    if matches!(
        interaction.state,
        InteractionState::Pressed | InteractionState::Dragging
    ) || interaction.right_press_origin.is_some()
    {
        cursor_options.hit_test = true;
        return;
    }

    let next_state = if hovering {
        InteractionState::Hover
    } else {
        InteractionState::None
    };

    interaction.state = next_state;
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
    hovered: Res<HoveredBodyPart>,
    mut clicked: MessageWriter<NeraClicked>,
    mut right_clicked: MessageWriter<NeraRightClicked>,
    mut drag_started: MessageWriter<NeraDragStarted>,
    mut dragged: MessageWriter<NeraDragged>,
    mut released: MessageWriter<NeraReleased>,
) {
    let Some(hwnd) = environment.nera_hwnd else {
        return;
    };

    let Some(global_cursor) = platform::global_cursor_position() else {
        return;
    };

    let cursor = IVec2::new(global_cursor.x, global_cursor.y);
    let now = Instant::now();

    // Right click is emitted on release, not press. Capture the gesture while
    // RMB is down so click-through cannot lose the release to another window.
    if interaction.state == InteractionState::Hover
        && mouse.just_pressed(MouseButton::Right)
    {
        if let Some(body_part) = hovered.part {
            interaction.right_press_origin = Some(cursor);
            interaction.right_pressed_body_part = Some(body_part);
        }
    }

    if mouse.just_released(MouseButton::Right) {
        let origin = interaction.right_press_origin.take();
        let body_part = interaction.right_pressed_body_part.take();

        if let (Some(origin), Some(body_part)) = (origin, body_part) {
            let right_click_threshold = (
                DRAG_START_DISTANCE_AT_96_DPI
                    * environment.nera_scale_factor as f32
            )
                .round()
                .max(1.0) as i32;

            let delta = cursor - origin;
            if delta.length_squared() <= right_click_threshold * right_click_threshold {
                right_clicked.write(NeraRightClicked {
                    pointer: nera_pointer_position(cursor, &environment),
                    body_part,
                });
            }
        }
    }

    // ----------------------------------------------------------------------
    // Hover -> Pressed
    // ----------------------------------------------------------------------
    // Mouse-down only captures the gesture. It is NOT a drag yet. This is the
    // core Phase 5.1 distinction that prevents head clicks from being treated
    // as window movement once body-part reactions arrive in 5.2/5.4.
    if interaction.state == InteractionState::Hover
        && mouse.just_pressed(MouseButton::Left)
    {
        if let (Some(rect), Some(body_part)) =
            (environment.nera_rect, hovered.part)
        {
            interaction.state = InteractionState::Pressed;
            interaction.pressed_body_part = Some(body_part);
            interaction.drag_offset = Some(IVec2::new(
                global_cursor.x - rect.left,
                global_cursor.y - rect.top,
            ));
            interaction.drag_start_cursor = Some(cursor);
            interaction.last_drag_cursor = Some(cursor);
            interaction.drag_started_at = Some(now);

            if matches!(
                environment.placement,
                PlacementState::Window(_) | PlacementState::Taskbar(_)
            ) {
                interaction.snap_cursor = Some(cursor);
            }
        }
    }

    // ----------------------------------------------------------------------
    // Pressed -> Dragging
    // ----------------------------------------------------------------------
    if mouse.pressed(MouseButton::Left)
        && interaction.state == InteractionState::Pressed
    {
        let drag_threshold = (
            DRAG_START_DISTANCE_AT_96_DPI
                * environment.nera_scale_factor as f32
        )
            .round()
            .max(1.0) as i32;

        let moved_enough = interaction
            .drag_start_cursor
            .map(|origin| {
                let delta = cursor - origin;
                delta.length_squared() >= drag_threshold * drag_threshold
            })
            .unwrap_or(false);

        if moved_enough {
            interaction.state = InteractionState::Dragging;
            interaction.last_drag_cursor = Some(cursor);

            if let Some(body_part) = interaction.pressed_body_part {
                drag_started.write(NeraDragStarted {
                    screen: cursor,
                    body_part,
                });
            }
        }
    }

    if mouse.pressed(MouseButton::Left)
        && interaction.state == InteractionState::Dragging
        && interaction.drag_offset.is_some()
    {
        if let Some(previous) = interaction.last_drag_cursor {
            let delta = cursor - previous;
            if delta != IVec2::ZERO {
                dragged.write(NeraDragged { screen: cursor, delta });
            }
        }
        interaction.last_drag_cursor = Some(cursor);

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

            // Mate Engine arms sitting only after a deliberate hold AND a real
            // drag. A click can never arm sitting now because Pressed is kept
            // separate from Dragging.
            let hold_ready = interaction
                .drag_started_at
                .map(|started| now.duration_since(started) >= MIN_DRAG_HOLD_TO_SIT)
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
                && !cooldown_blocked
                && !guard_blocked
                && !vertical_blocked
            {
                // IMPORTANT: this does NOT move the HWND. It only creates a
                // Pending* placement. sync_animation_base_pose switches to sitting first;
                // Last then samples Seat and commits the attachment later.
                environment.try_begin_sit_from_probe();
            }
        }
    }

    // ----------------------------------------------------------------------
    // Pressed -> Click, Dragging -> Release
    // ----------------------------------------------------------------------
    if mouse.just_released(MouseButton::Left)
        && matches!(
            interaction.state,
            InteractionState::Pressed | InteractionState::Dragging
        )
    {
        let was_dragging = interaction.state == InteractionState::Dragging;
        let pointer = nera_pointer_position(cursor, &environment);

        if let Some(body_part) = interaction.pressed_body_part {
            if !was_dragging {
                clicked.write(NeraClicked {
                    pointer,
                    body_part,
                });
            }

            released.write(NeraReleased {
                pointer,
                body_part,
                was_dragging,
            });
        }

        interaction.state = InteractionState::Hover;
        interaction.pressed_body_part = None;
        interaction.drag_offset = None;
        interaction.drag_start_cursor = None;
        interaction.last_drag_cursor = None;
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
