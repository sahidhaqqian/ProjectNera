mod animation;
mod alarm;

mod desktop_physics;

#[cfg(target_os = "windows")]
mod action_menu;
mod environment;
mod interaction;
mod platform;
mod presentation;
mod settings;

#[cfg(target_os = "windows")]
mod tray;

use animation::{
    NeraAnimationPlugin,
    NeraAnimationState,
    apply_animation_state,
    register_angry_animation,
    register_death_animation,
    register_falling_animation,
    register_falling_to_landing_animation,
    register_floating_animation,
    register_head_stroke_animation,
    register_hit_chest_animation,
    register_hit_head_animation,
    register_idle_animation,
    register_praying_animation,
    register_sad_animation,
    register_sitting_enter_animation,
    register_sitting_exit_animation,
    register_sitting_loop_animation,
    register_surprised_animation,
    register_thinking_animation,
    register_wave_animation,
    sync_environment_animation_state,
    sync_interaction_animation_actions,
    tick_animation_actions,
};

use environment::{
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
    NeraHeadPatted,
    NeraPointerPosition,
    NeraReleased,
    NeraRightClicked,
};

use std::{
    num::NonZeroU32,
    time::{Duration, Instant},
};

use bevy::{
    asset::AssetPlugin,
    prelude::*,
    settings::SettingsPlugin,
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

use bevy_vrm1::{
    prelude::*,
    vrm::body_tracking::BodyTracking,
};

use presentation::PresentationPlugin;
use settings::{NeraSettings, NeraSettingsPlugin};

#[cfg(target_os = "windows")]
use action_menu::ActionMenuPlugin;

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

// Phase 5 runtime animation set. Large animation libraries stay as stock;
// only stable animations used by current behavior live in animasi_basic.
const IDLE_VRMA_PATH: &str =
    "animations/animasi_basic/idle.vrma";
const FLOATING_VRMA_PATH: &str =
    "animations/animasi_basic/Floating.vrma";
const DEATH_VRMA_PATH: &str =
    "animations/animasi_basic/Death01.vrma";
const FALLING_VRMA_PATH: &str =
    "animations/Falling.vrma";
const FALLING_TO_LANDING_VRMA_PATH: &str =
    "animations/Falling_To_Landing.vrma";
const HIT_HEAD_VRMA_PATH: &str =
    "animations/animasi_basic/Hit_Head.vrma";
const HIT_CHEST_VRMA_PATH: &str =
    "animations/animasi_basic/Hit_Chest.vrma";
const HEAD_STROKE_VRMA_PATH: &str =
    "animations/animasi_basic/ShakeHeadNo1.vrma";
const PRAYING_VRMA_PATH: &str =
    "animations/animasi_basic/Praying.vrma";
const SITTING_ENTER_VRMA_PATH: &str =
    "animations/animasi_basic/Sitting_Enter.vrma";
const SITTING_LOOP_VRMA_PATH: &str =
    "animations/animasi_basic/Sitting_Idle_Loop.vrma";
const SITTING_EXIT_VRMA_PATH: &str =
    "animations/animasi_basic/Sitting_Exit.vrma";

// Phase 5.5 menu-triggered animation library.
// Only these five are preloaded; the hundreds of other downloaded VRMAs stay
// as stock and consume no runtime animation slots yet.
const MENU_WAVE_VRMA_PATH: &str =
    "animations/Clawatar_animations/161_Waving.vrma";
const MENU_THINKING_VRMA_PATH: &str =
    "animations/VRMA/Thinking.vrma";
const MENU_ANGRY_VRMA_PATH: &str =
    "animations/VRMA/Angry.vrma";
const MENU_SAD_VRMA_PATH: &str =
    "animations/VRMA/Sad.vrma";
const MENU_SURPRISED_VRMA_PATH: &str =
    "animations/VRMA/Surprised.vrma";

/// VrmBone `foot` is located around the ankle; shift the standing support
/// plane down to the visible sole in logical window pixels.
const FOOT_SOLE_OFFSET_LOGICAL: f32 = 12.0;

/// Local/world-space offset from the VRM hips bone toward the point that should
/// visually touch a sitting surface. Tune this after `sitting.vrma` is wired.
/// +Y is up; the small +Z term moves the anchor slightly toward Nera's rear in
/// the current avatar orientation.
const SEAT_OFFSET_WORLD: Vec3 = Vec3::new(0.0, -0.12, 0.08);

/// Additional visual clearance used when a horizontal Death01 pose is grounded.
/// The support point is measured from animated body bones in screen space.
const DEATH_GROUND_MARGIN_LOGICAL: f32 = 8.0;

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
// Phase 5.4 head-stroke gesture tuning
// -----------------------------------------------------------------------------

/// "Elus kepala" accepts deliberate back-and-forth motion on either axis.
/// No mouse button is required, so it never competes with click-vs-drag.
const HEAD_STROKE_WINDOW: Duration = Duration::from_millis(900);
const HEAD_STROKE_COOLDOWN: Duration = Duration::from_millis(1200);
const HEAD_STROKE_MIN_TRAVEL_AT_96_DPI: f32 = 62.0;
const HEAD_STROKE_MIN_SEGMENT_AT_96_DPI: f32 = 10.0;
const HEAD_STROKE_JITTER_AT_96_DPI: f32 = 2.0;
const HEAD_STROKE_MIN_REVERSALS: u8 = 1;

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


#[derive(Resource, Default)]
struct HeadPatGesture {
    started_at: Option<Instant>,
    last_cursor: Option<IVec2>,
    total_travel: f32,

    x_direction: i8,
    y_direction: i8,
    x_segment: i32,
    y_segment: i32,
    x_reversals: u8,
    y_reversals: u8,

    cooldown_until: Option<Instant>,
}

impl HeadPatGesture {
    fn reset_stroke(&mut self) {
        self.started_at = None;
        self.last_cursor = None;
        self.total_travel = 0.0;
        self.x_direction = 0;
        self.y_direction = 0;
        self.x_segment = 0;
        self.y_segment = 0;
        self.x_reversals = 0;
        self.y_reversals = 0;
    }

    fn begin_stroke(&mut self, cursor: IVec2, now: Instant) {
        self.reset_stroke();
        self.started_at = Some(now);
        self.last_cursor = Some(cursor);
    }
}

#[derive(Resource, Default)]
struct DeathGrounding {
    /// Desktop-space Y of Nera's standing floor. Refreshed while Nera is
    /// normally standing and then frozen throughout Death01.
    ground_y_physical: Option<i32>,
    was_death_active: bool,
}

#[derive(Component)]
struct NeraAvatar;

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

/// Bevy 0.19 resolves file assets relative to the executable directory when
/// BEVY_ASSET_ROOT / CARGO_MANIFEST_DIR are not present at runtime. `cargo run`
/// therefore points at `target/debug/assets`, while Project Nera keeps assets
/// in the repository root. Prefer the repository assets while developing, and
/// fall back to a normal `assets` folder beside the executable for packaged builds.
fn nera_asset_path() -> String {
    let development_assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");

    if development_assets.is_dir() {
        development_assets.to_string_lossy().into_owned()
    } else {
        "assets".to_string()
    }
}

fn main() {
    let mut app = App::new();

    // Load persistent Nera settings before engine-facing glue systems run.
    app.add_plugins(SettingsPlugin::new("io.github.sahidhaqqi.projectnera"));

    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: nera_asset_path(),
                ..default()
            })
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
                    // Keep the native pet hidden until persisted settings have
                    // been loaded. NeraSettingsPlugin reveals it on the first
                    // update unless "Start hidden" is enabled.
                    visible: false,
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
        PresentationPlugin,
        alarm::AlarmPlugin,
        NeraSettingsPlugin,
    ));

    #[cfg(target_os = "windows")]
    app.add_plugins((TrayPlugin, ActionMenuPlugin));

    app.insert_resource(ClearColor(Color::NONE))
        .init_resource::<DesktopPetInteraction>()
        .init_resource::<HoveredBodyPart>()
        .init_resource::<DeathGrounding>()
        .init_resource::<HeadPatGesture>()
        .init_resource::<desktop_physics::DesktopPhysicsState>()
        .add_systems(
            Startup,
            (setup_camera, setup_lighting, spawn_nera),
        )
        .add_systems(
            Update,
            (
                attach_bone_colliders,
                update_look_at_target,
                // A secondary Settings window owns application mouse input while
                // it is open. Reset Nera capture state first so Settings clicks
                // cannot become body hits / drags through global ButtonInput.
                suspend_nera_interaction_for_settings
                    .run_if(settings::nera_input_blocked),
                // Native placement changes first, then Phase 5.3 derives the
                // base pose, consumes temporary interaction actions, resolves
                // priority, and finally applies the correct VRMA/anchor.
                drag_nera_window
                    .run_if(presentation::desktop_pet_active)
                    .run_if(settings::nera_input_available),
                desktop_physics::update_desktop_physics
                    .run_if(presentation::desktop_pet_active),
                sync_environment_animation_state,
                sync_interaction_animation_actions,
                tick_animation_actions,
                sync_look_at_for_animation,
                apply_animation_state,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            update_click_through
                .after(VrmSystemSets::SpringBone)
                .run_if(settings::nera_input_available),
        )
        .add_systems(
            PostUpdate,
            detect_head_pat
                .after(update_click_through)
                .run_if(presentation::desktop_pet_active)
                .run_if(settings::nera_input_available),
        )
        // Pose-first support calibration: Sitting_Enter starts in Update,
        // then Last samples its animated Seat and commits the attachment.
        // The attachment keeps Seat aligned for the rest of the entry clip.
        .add_systems(
            Last,
            (
                update_avatar_anchors,
                settle_pending_sit.run_if(presentation::desktop_pet_active),
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
            NeraAvatar,
            VrmHandle(asset_server.load(VRM_PATH)),
            LookAt::Target(target),
            Transform::from_xyz(0.0, -0.85, 0.0),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Name::new("Nera Idle"),
                    VrmaHandle(asset_server.load(IDLE_VRMA_PATH)),
                ))
                .observe(register_idle_animation);

            parent
                .spawn((
                    Name::new("Nera Floating"),
                    VrmaHandle(asset_server.load(FLOATING_VRMA_PATH)),
                ))
                .observe(register_floating_animation);

            parent
                .spawn((
                    Name::new("Nera Death Reaction"),
                    VrmaHandle(asset_server.load(DEATH_VRMA_PATH)),
                ))
                .observe(register_death_animation);

            parent
                .spawn((
                    Name::new("Nera Falling"),
                    VrmaHandle(asset_server.load(FALLING_VRMA_PATH)),
                ))
                .observe(register_falling_animation);

            parent
                .spawn((
                    Name::new("Nera Falling To Landing"),
                    VrmaHandle(asset_server.load(FALLING_TO_LANDING_VRMA_PATH)),
                ))
                .observe(register_falling_to_landing_animation);

            parent
                .spawn((
                    Name::new("Nera Hit Head"),
                    VrmaHandle(asset_server.load(HIT_HEAD_VRMA_PATH)),
                ))
                .observe(register_hit_head_animation);

            parent
                .spawn((
                    Name::new("Nera Hit Chest"),
                    VrmaHandle(asset_server.load(HIT_CHEST_VRMA_PATH)),
                ))
                .observe(register_hit_chest_animation);

            parent
                .spawn((
                    Name::new("Nera Head Stroke Reaction"),
                    VrmaHandle(asset_server.load(HEAD_STROKE_VRMA_PATH)),
                ))
                .observe(register_head_stroke_animation);

            parent
                .spawn((
                    Name::new("Nera Praying Menu Pose"),
                    VrmaHandle(asset_server.load(PRAYING_VRMA_PATH)),
                ))
                .observe(register_praying_animation);

            parent
                .spawn((
                    Name::new("Nera Sitting Enter"),
                    VrmaHandle(asset_server.load(SITTING_ENTER_VRMA_PATH)),
                ))
                .observe(register_sitting_enter_animation);

            parent
                .spawn((
                    Name::new("Nera Sitting Loop"),
                    VrmaHandle(asset_server.load(SITTING_LOOP_VRMA_PATH)),
                ))
                .observe(register_sitting_loop_animation);

            parent
                .spawn((
                    Name::new("Nera Sitting Exit"),
                    VrmaHandle(asset_server.load(SITTING_EXIT_VRMA_PATH)),
                ))
                .observe(register_sitting_exit_animation);

            parent
                .spawn((
                    Name::new("Nera Menu Wave"),
                    VrmaHandle(asset_server.load(MENU_WAVE_VRMA_PATH)),
                ))
                .observe(register_wave_animation);

            parent
                .spawn((
                    Name::new("Nera Menu Thinking"),
                    VrmaHandle(asset_server.load(MENU_THINKING_VRMA_PATH)),
                ))
                .observe(register_thinking_animation);

            parent
                .spawn((
                    Name::new("Nera Menu Angry"),
                    VrmaHandle(asset_server.load(MENU_ANGRY_VRMA_PATH)),
                ))
                .observe(register_angry_animation);

            parent
                .spawn((
                    Name::new("Nera Menu Sad"),
                    VrmaHandle(asset_server.load(MENU_SAD_VRMA_PATH)),
                ))
                .observe(register_sad_animation);

            parent
                .spawn((
                    Name::new("Nera Menu Surprised"),
                    VrmaHandle(asset_server.load(MENU_SURPRISED_VRMA_PATH)),
                ))
                .observe(register_surprised_animation);
        });
}

// -----------------------------------------------------------------------------
// LookAt Target Tracker
// -----------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn update_look_at_target(
    time: Res<Time>,
    settings: Res<NeraSettings>,
    environment: Res<DesktopEnvironment>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    mut target_query: Query<&mut Transform, With<LookAtTarget>>,
) {
    let Some(window) = window_query.iter().next() else {
        return;
    };

    let Some(mut target_transform) = target_query.iter_mut().next() else {
        return;
    };

    // Keep a stable neutral point in front of Nera. Turning Follow Mouse off
    // eases the target back here instead of snapping the gaze in one frame.
    let neutral = Vec3::new(0.0, 0.5, 2.0);

    let desired = if settings.follow_mouse {
        cursor_position_in_nera_window(&environment)
            .map(|cursor_pos| {
                let intensity =
                    f32::from(settings.follow_intensity.clamp(25, 100)) / 100.0;

                let map_x =
                    (cursor_pos.x / window.resolution.width() - 0.5)
                        * 4.0
                        * intensity;

                let map_y =
                    0.5
                        + (0.5 - cursor_pos.y / window.resolution.height())
                            * 4.0
                            * intensity;

                Vec3::new(map_x, map_y, 2.0)
            })
            .unwrap_or(neutral)
    } else {
        neutral
    };

    // Frame-rate independent exponential smoothing. A value around 12 keeps
    // gaze responsive while removing the robotic cursor-to-bone snapping.
    let blend = 1.0 - (-12.0 * time.delta_secs()).exp();
    target_transform.translation =
        target_transform.translation.lerp(desired, blend);
}


/// VRMA reactions such as HitHead and ShakeHeadNo1 need temporary ownership of
/// the head bones. `bevy_vrm1` LookAt runs after animation, so the animation
/// suppression rule remains authoritative over the user Follow Mouse settings.
fn sync_look_at_for_animation(
    animation: Res<NeraAnimationState>,
    settings: Res<NeraSettings>,
    target_query: Query<Entity, With<LookAtTarget>>,
    avatar_query: Query<
        (Entity, Option<&LookAt>, Option<&BodyTracking>),
        With<NeraAvatar>,
    >,
    mut commands: Commands,
) {
    let Some(target) = target_query.iter().next() else {
        return;
    };

    let Some((avatar, current_look_at, current_body_tracking)) =
        avatar_query.iter().next()
    else {
        return;
    };

    // The master Follow Mouse switch changes the target between cursor and
    // neutral. Keep LookAt attached (when Eyes is enabled) so switching the
    // master off can visibly ease back to neutral instead of freezing mid-gaze.
    let allow_look_at = settings.follow_eyes && !animation.suppresses_look_at();
    let allow_body_tracking = allow_look_at && settings.follow_head;

    if allow_look_at {
        if current_look_at.is_none() {
            commands.entity(avatar).insert(LookAt::Target(target));
        }
    } else if current_look_at.is_some() {
        commands.entity(avatar).remove::<LookAt>();
    }

    // bevy_vrm1's BodyTracking extends LookAt from the eyes into the upper
    // body. Keep it opt-in and remove it whenever LookAt is unavailable so it
    // cannot fight reaction / transition VRMAs.
    if allow_body_tracking {
        if current_body_tracking.is_none() {
            commands.entity(avatar).insert(BodyTracking::default());
        }
    } else if current_body_tracking.is_some() {
        commands.entity(avatar).remove::<BodyTracking>();
    }
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
    animation: Res<NeraAnimationState>,
    settings: Res<NeraSettings>,
    mut physics: ResMut<desktop_physics::DesktopPhysicsState>,
    mut grounding: ResMut<DeathGrounding>,
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

    let character_scale = settings.character_scale();
    let standing_support_offset =
        FOOT_SOLE_OFFSET_LOGICAL * character_scale + settings.ground_offset_logical();

    let mut feet = Vec::with_capacity(2);
    let mut death_support_points = Vec::with_capacity(12);
    let mut seat = None;

    for (bone, transform) in &bone_query {
        if matches!(
            bone.as_str(),
            "head"
                | "hips"
                | "chest"
                | "leftHand"
                | "rightHand"
                | "leftFoot"
                | "rightFoot"
                | "leftLowerLeg"
                | "rightLowerLeg"
        ) {
            if let Ok(viewport) = camera.world_to_viewport(
                camera_transform,
                transform.translation(),
            ) {
                death_support_points.push(viewport);
            }
        }

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
            + standing_support_offset;

        environment.set_feet_anchor(Vec2::new(x, y));

        // Physics intentionally freezes its standing anchor during normal
        // animation. A settings change can alter avatar scale/ground contact,
        // so refresh that rigid-body anchor from the newly evaluated geometry.
        if settings.is_changed() && !physics.falling && !animation.death_active() {
            physics.refresh_support_anchor(environment.feet_anchor_logical);
        }
    }

    // ---------------------------------------------------------------------
    // Death01 ground stabilization
    // ---------------------------------------------------------------------
    //
    // Cache the standing floor continuously *before* Death starts. A death
    // animation can move the feet on its very first evaluated frame, so using
    // the current Death-frame feet would capture the wrong baseline.
    //
    // During Death, keep the lowest animated body point on that frozen
    // desktop-space floor line. When the one-shot finishes, re-align the
    // standing feet to the exact same floor before normal anchor sampling
    // resumes.
    if environment.placement == PlacementState::Free
        && interaction.state != InteractionState::Dragging
        && interaction.drag_offset.is_none()
    {
        let death_active = animation.death_active();
        let scale = environment.nera_scale_factor.max(0.25) as f32;

        if let Some(rect) = environment.nera_rect {
            let standing_feet_y = (!feet.is_empty()).then(|| {
                feet
                    .iter()
                    .map(|point| point.y)
                    .fold(f32::NEG_INFINITY, f32::max)
                    + standing_support_offset
            });

            if !death_active && !grounding.was_death_active {
                // Normal standing frame: continuously refresh the floor.
                if let Some(feet_y) = standing_feet_y {
                    grounding.ground_y_physical = Some(
                        rect.top + (feet_y * scale).round() as i32
                    );
                }
            }

            if death_active {
                if !grounding.was_death_active {
                    info!(
                        "Death grounding locked floor y={:?}",
                        grounding.ground_y_physical
                    );
                }

                if let (Some(hwnd), Some(ground_y), Some(lowest_y)) = (
                    environment.nera_hwnd,
                    grounding.ground_y_physical,
                    death_support_points
                        .iter()
                        .map(|point| point.y)
                        .max_by(|a, b| a.total_cmp(b)),
                ) {
                    let support_y =
                        lowest_y + DEATH_GROUND_MARGIN_LOGICAL * character_scale;

                    let desired_top =
                        ground_y - (support_y * scale).round() as i32;

                    if (desired_top - rect.top).abs() >= 1 {
                        let desired = platform::PhysicalPoint::new(
                            rect.left,
                            desired_top,
                        );

                        if platform::move_window(hwnd, desired).is_ok() {
                            environment.nera_rect =
                                platform::window_rect(hwnd);
                        }
                    }
                }
            } else if grounding.was_death_active {
                if let (Some(hwnd), Some(ground_y), Some(feet_y)) = (
                    environment.nera_hwnd,
                    grounding.ground_y_physical,
                    standing_feet_y,
                ) {
                    let desired_top =
                        ground_y - (feet_y * scale).round() as i32;

                    let desired = platform::PhysicalPoint::new(
                        rect.left,
                        desired_top,
                    );

                    if platform::move_window(hwnd, desired).is_ok() {
                        environment.nera_rect =
                            platform::window_rect(hwnd);
                    }
                }

                info!("Death grounding released -> standing floor restored");
            }

            grounding.was_death_active = death_active;
        }
    } else if !animation.death_active() {
        grounding.was_death_active = false;
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
    // Commit as soon as the evaluated Sitting_Enter pose provides a Seat
    // anchor. Once attached, Phase 4 keeps the animated Seat aligned to the
    // support while the entry clip continues, eliminating the old
    // "floating first, snap on SittingLoop" behavior.
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

fn vrm_bone_position(
    bone_query: &Query<(&VrmBone, &GlobalTransform)>,
    name: &str,
) -> Option<Vec3> {
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
fn head_part_at_cursor(
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
            let radius_x = (eye_distance * 2.0)
                .clamp(30.0 * character_scale, 48.0 * character_scale);
            let radius_y = (radius_x * 1.20)
                .clamp(36.0 * character_scale, 58.0 * character_scale);
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

fn nearest_nera_body_part(
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
                ray_sphere_distance(
                    ray_origin,
                    ray_dir,
                    pa,
                    radius * character_scale,
                )
            }
            NeraColliderShape::Capsule { radius, end_bone } => {
                let pb = bone_query.iter().find_map(|(bone, transform)| {
                    (bone.as_str() == end_bone).then(|| transform.translation())
                });

                match pb {
                    Some(pb) => ray_capsule_distance(
                        ray_origin,
                        ray_dir,
                        pa,
                        pb,
                        radius * character_scale,
                    ),
                    None => ray_sphere_distance(
                        ray_origin,
                        ray_dir,
                        pa,
                        radius * character_scale,
                    ),
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
            // Head is intentionally handled in screen-space by
            // `head_part_at_cursor()`. A generic 3D sphere around the humanoid
            // head joint proved too dependent on how each VRM places that joint.
            "head" => None,

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
fn suspend_nera_interaction_for_settings(
    mut interaction: ResMut<DesktopPetInteraction>,
    mut hovered: ResMut<HoveredBodyPart>,
    mut head_pat: ResMut<HeadPatGesture>,
    mut window_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    // Settings is a real second Bevy window, but ButtonInput<MouseButton> is
    // application-global. Clear every *active gesture* so a Settings click
    // cannot continue a PrimaryWindow press/drag that happened previously.
    interaction.state = InteractionState::None;
    interaction.pressed_body_part = None;
    interaction.right_pressed_body_part = None;
    interaction.drag_offset = None;
    interaction.drag_start_cursor = None;
    interaction.last_drag_cursor = None;
    interaction.drag_started_at = None;
    interaction.right_press_origin = None;
    interaction.snap_cursor = None;

    hovered.part = None;
    head_pat.reset_stroke();

    // Critical when Nera itself is Always-On-Top and visually overlaps the
    // Settings panel: let native hit-testing pass through the pet HWND.
    if let Ok(mut cursor_options) = window_query.single_mut() {
        cursor_options.hit_test = false;
    }
}

#[cfg(target_os = "windows")]
fn update_click_through(
    settings: Res<NeraSettings>,
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
    let character_scale = settings.character_scale();

    let hovered_body_part = cursor_position_in_nera_window(&environment)
        .and_then(|cursor_pos| {
            // Head gets a visual screen-space test first. Other body regions
            // continue to use deterministic nearest 3D collider distance.
            if head_part_at_cursor(
                cursor_pos,
                camera,
                camera_transform,
                &bone_query,
                character_scale,
            ) {
                return Some(BodyPart::Head);
            }

            camera
                .viewport_to_world(camera_transform, cursor_pos)
                .ok()
                .and_then(|ray| {
                    nearest_nera_body_part(
                        ray.origin,
                        ray.direction.into(),
                        &colliders_query,
                        &bone_query,
                        character_scale,
                    )
                })
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
// Phase 5.4 Head Pat Gesture
// -----------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn detect_head_pat(
    mouse: Res<ButtonInput<MouseButton>>,
    hovered: Res<HoveredBodyPart>,
    environment: Res<DesktopEnvironment>,
    mut gesture: ResMut<HeadPatGesture>,
    mut head_patted: MessageWriter<NeraHeadPatted>,
) {
    // Click/drag/right-click keep their own gesture ownership.
    if mouse.pressed(MouseButton::Left) || mouse.pressed(MouseButton::Right) {
        gesture.reset_stroke();
        return;
    }

    if hovered.part != Some(BodyPart::Head) {
        gesture.reset_stroke();
        return;
    }

    let Some(point) = platform::global_cursor_position() else {
        gesture.reset_stroke();
        return;
    };

    let cursor = IVec2::new(point.x, point.y);
    let now = Instant::now();

    if gesture
        .cooldown_until
        .map(|until| now < until)
        .unwrap_or(false)
    {
        gesture.last_cursor = Some(cursor);
        return;
    }

    if gesture
        .started_at
        .map(|started| now.duration_since(started) > HEAD_STROKE_WINDOW)
        .unwrap_or(true)
    {
        gesture.begin_stroke(cursor, now);
        return;
    }

    let Some(previous) = gesture.last_cursor else {
        gesture.begin_stroke(cursor, now);
        return;
    };

    let delta = cursor - previous;
    gesture.last_cursor = Some(cursor);

    let scale = environment.nera_scale_factor.max(0.25) as f32;
    let jitter =
        (HEAD_STROKE_JITTER_AT_96_DPI * scale).round().max(1.0) as i32;
    let segment_min =
        (HEAD_STROKE_MIN_SEGMENT_AT_96_DPI * scale).round().max(1.0) as i32;
    let min_travel =
        HEAD_STROKE_MIN_TRAVEL_AT_96_DPI * scale;

    let delta_f = Vec2::new(delta.x as f32, delta.y as f32);
    let frame_travel = delta_f.length();

    if frame_travel < jitter as f32 {
        return;
    }

    gesture.total_travel += frame_travel;

    // Track reversal independently on X and Y so horizontal, vertical, and
    // natural diagonal/circular petting gestures all work.
    if delta.x.abs() >= jitter {
        let direction = delta.x.signum() as i8;

        if gesture.x_direction == 0 || gesture.x_direction == direction {
            gesture.x_direction = direction;
            gesture.x_segment += delta.x.abs();
        } else {
            if gesture.x_segment >= segment_min {
                gesture.x_reversals =
                    gesture.x_reversals.saturating_add(1);
            }
            gesture.x_direction = direction;
            gesture.x_segment = delta.x.abs();
        }
    }

    if delta.y.abs() >= jitter {
        let direction = delta.y.signum() as i8;

        if gesture.y_direction == 0 || gesture.y_direction == direction {
            gesture.y_direction = direction;
            gesture.y_segment += delta.y.abs();
        } else {
            if gesture.y_segment >= segment_min {
                gesture.y_reversals =
                    gesture.y_reversals.saturating_add(1);
            }
            gesture.y_direction = direction;
            gesture.y_segment = delta.y.abs();
        }
    }

    let reversals =
        gesture.x_reversals.saturating_add(gesture.y_reversals);

    if gesture.total_travel < min_travel
        || reversals < HEAD_STROKE_MIN_REVERSALS
    {
        return;
    }

    head_patted.write(NeraHeadPatted {
        pointer: nera_pointer_position(cursor, &environment),
    });

    info!(
        "Nera head-stroke recognized: travel={:.1}px x_rev={} y_rev={}",
        gesture.total_travel,
        gesture.x_reversals,
        gesture.y_reversals,
    );

    gesture.cooldown_until = Some(now + HEAD_STROKE_COOLDOWN);
    gesture.reset_stroke();
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
                // Pending* placement. Sitting_Enter owns the landing transition before Seat calibration;
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
