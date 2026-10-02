//! Project Nera — Phase 5.55 Presentation Mode foundation.
//!
//! This module deliberately separates:
//!
//!     desktop world / pet behavior
//!
//! from:
//!
//!     how Nera is presented to the user right now.
//!
//! Normal mode keeps the compact transparent pet HWND.
//! Attention mode keeps the proven compact transparent Bevy HWND, moves it
//! into a prominent monitor position, and reframes the camera so Nera becomes
//! large. Phase 5.6 will place the alarm controls in a separate native panel.
//!
//! This avoids monitor-sized Bevy transparent windows, which are unreliable
//! with the current Windows compositor/backend combination.
//!
//! Phase 5.6 Alarm will drive this through `PresentationCommand`.
//! Ctrl+Alt+P is only a Phase-5.55 debug toggle and can be removed later.

use bevy::{
    prelude::*,
    window::{
        CursorOptions,
        PrimaryWindow,
        WindowLevel,
        WindowPosition,
    },
};

use bevy_vrm1::prelude::VrmBone;

use crate::{
    animation::NeraAnimationState,
    environment::{
        AvatarAnchor,
        DesktopEnvironment,
        PlacementState,
    },
    interaction::{DesktopPetInteraction, HeadPatGesture, HoveredBodyPart, NeraLiveBone},
};

// -----------------------------------------------------------------------------
// Presentation vocabulary
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AttentionKind {
    Alarm,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum PresentationMode {
    #[default]
    DesktopPet,
    Attention(AttentionKind),
}

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PresentationCommand {
    EnterAttention(AttentionKind),
    ExitAttention,
}

/// Emitted after the window/camera has actually entered attention mode.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PresentationEntered {
    pub kind: AttentionKind,
}

/// Emitted after the original desktop-pet presentation has been restored.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PresentationExited {
    pub kind: AttentionKind,
}

// -----------------------------------------------------------------------------
// Snapshot / state
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct PresentationSnapshot {
    window_left: i32,
    window_top: i32,
    window_width: u32,
    window_height: u32,
    window_level: WindowLevel,
    cursor_hit_test: bool,
    camera_transform: Transform,

    // Desktop-pet world state is suspended during Attention and restored
    // exactly when the presentation closes.
    placement: PlacementState,
    attached_anchor: AvatarAnchor,
    hidden_for_target_suspend: bool,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct PresentationState {
    mode: PresentationMode,
    snapshot: Option<PresentationSnapshot>,

    /// 0..1 entrance animation progress for Attention Mode.
    attention_enter_progress: f32,

    /// Film-camera style framing lock. Animation bones may move inside the
    /// frame, but the camera does not chase Wave/Jump hand motion.
    attention_locked_center: Option<Vec2>,

    /// Once Attention starts, the transparent canvas never shrinks during the
    /// same session. This avoids HWND resize jitter during looping animation.
    attention_locked_window_width: u32,

    /// Camera may back up if an extreme pose genuinely needs more room, but it
    /// never zooms back in until Attention ends.
    attention_peak_camera_z: f32,
}

impl PresentationState {

    pub(crate) fn is_attention(&self) -> bool {
        matches!(self.mode, PresentationMode::Attention(_))
    }

    pub(crate) fn is_desktop_pet(&self) -> bool {
        self.mode == PresentationMode::DesktopPet
    }
}

// -----------------------------------------------------------------------------
// Tuning
// -----------------------------------------------------------------------------

/// Attention is intentionally a *portrait* presentation, not a giant full-body
/// model. The camera frames roughly from hips/waist to above the head.
const ATTENTION_UPPER_BODY_VIEW_FRACTION: f32 = 0.80;

/// Horizontal occupancy target. Leave some room so hands/sleeves are not
/// clipped when JumpHappy or other attention animations spread the arms.
const ATTENTION_UPPER_BODY_HORIZONTAL_FRACTION: f32 = 0.84;

/// The transparent presentation canvas may widen to fit expressive arm poses,
/// but it still avoids becoming a true monitor-sized Bevy window.
const ATTENTION_MAX_WINDOW_WIDTH_RATIO: f32 = 0.88;

/// Small world-space allowance beyond measured arm/hand bones for sleeves,
/// fingers, hair, and mesh thickness.
const UPPER_BODY_HORIZONTAL_MARGIN_RATIO: f32 = 0.14;

/// Presentation window height relative to the active monitor.
///
/// The window stays compact enough to preserve Windows transparency behavior,
/// but tall enough for a large half-body portrait.
const ATTENTION_WINDOW_MONITOR_HEIGHT_RATIO: f32 = 0.78;

/// Extra space above the head for hair / maid headpiece, expressed relative to
/// head-to-hips humanoid span.
const UPPER_BODY_TOP_MARGIN_RATIO: f32 = 0.18;

/// Small crop allowance below the hips so the waist doesn't feel chopped at
/// exactly the humanoid hips joint.
const UPPER_BODY_BOTTOM_MARGIN_RATIO: f32 = 0.06;

/// Avoid pathological rig measurements during loading.
const MIN_UPPER_BODY_WORLD_HEIGHT: f32 = 0.45;

/// Entrance slide duration. The compact transparent HWND starts below the
/// monitor and eases upward into its final bottom-anchored attention position.
const ATTENTION_ENTER_SECONDS: f32 = 0.38;

// -----------------------------------------------------------------------------
// Plugin
// -----------------------------------------------------------------------------

pub(crate) struct PresentationPlugin;

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PresentationState>()
            .add_message::<PresentationCommand>()
            .add_message::<PresentationEntered>()
            .add_message::<PresentationExited>()
            .add_systems(
                Update,
                (
                    debug_toggle_attention,
                    handle_presentation_commands,
                )
                    .chain()
                    .before(crate::startup_entrance::prepare_startup_entrance),
            )
            // Last intentionally wins over Phase 4 window-follow systems.
            // This avoids changing working sitting/tracking code in Phase 5.55.
            .add_systems(Last, maintain_attention_presentation);
    }
}

// -----------------------------------------------------------------------------
// Debug entry point
// -----------------------------------------------------------------------------

fn debug_toggle_attention(
    keyboard: Res<ButtonInput<KeyCode>>,
    state: Res<PresentationState>,
    mut commands: MessageWriter<PresentationCommand>,
) {
    // Do not use F9 here. On many laptops / screenshot tools the physical
    // screenshot key is multiplexed with a function key and could
    // accidentally exit Attention Mode while trying to capture it.
    let ctrl = keyboard.pressed(KeyCode::ControlLeft)
        || keyboard.pressed(KeyCode::ControlRight);
    let alt = keyboard.pressed(KeyCode::AltLeft)
        || keyboard.pressed(KeyCode::AltRight);

    if !(ctrl && alt && keyboard.just_pressed(KeyCode::KeyP)) {
        return;
    }

    if state.is_attention() {
        commands.write(PresentationCommand::ExitAttention);
    } else {
        commands.write(PresentationCommand::EnterAttention(
            AttentionKind::Alarm,
        ));
    }
}

// -----------------------------------------------------------------------------
// Enter / Exit
// -----------------------------------------------------------------------------

/// Drop all pointer state when Attention takes over or hands back.
///
/// `drag_nera_window` is gated on `desktop_pet_active`, so an alarm firing
/// mid-gesture means the LMB release edge is never observed. Left alone,
/// `update_click_through` keeps forcing `hit_test = true` for the whole
/// captured session *and after it*, swallowing desktop clicks until the user
/// happens to complete another full press+release over Nera.
fn reset_pointer_state(
    interaction: &mut DesktopPetInteraction,
    hovered: &mut HoveredBodyPart,
    head_pat: &mut HeadPatGesture,
) {
    interaction.reset_for_attention();
    hovered.part = None;
    head_pat.reset_stroke();
}

fn handle_presentation_commands(
    mut requests: MessageReader<PresentationCommand>,
    mut entered: MessageWriter<PresentationEntered>,
    mut exited: MessageWriter<PresentationExited>,
    mut state: ResMut<PresentationState>,
    mut environment: ResMut<DesktopEnvironment>,
    mut animation: ResMut<NeraAnimationState>,
    mut interaction: ResMut<DesktopPetInteraction>,
    mut hovered: ResMut<HoveredBodyPart>,
    mut head_pat: ResMut<HeadPatGesture>,
    mut window_query: Query<
        (&mut Window, &mut CursorOptions),
        With<PrimaryWindow>,
    >,
    mut camera_query: Query<
        (&Projection, &mut Transform),
        With<crate::avatar::NeraMainCamera>,
    >,
    bone_query: Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
) {
    for request in requests.read() {
        match *request {
            PresentationCommand::EnterAttention(kind) => {
                if state.is_attention() {
                    // Phase 5.6 may re-trigger the same attention mode; avoid
                    // overwriting the original DesktopPet snapshot.
                    continue;
                }

                let Ok((mut window, cursor)) = window_query.single_mut() else {
                    warn!("Presentation: primary Nera window unavailable");
                    continue;
                };

                let Ok((projection, mut camera_transform)) =
                    camera_query.single_mut()
                else {
                    warn!("Presentation: Nera camera unavailable");
                    continue;
                };

                let Some(rect) = environment.nera_rect else {
                    warn!(
                        "Presentation: Nera HWND rect unavailable; attention deferred"
                    );
                    continue;
                };

                let Some(monitor) = environment.monitor else {
                    warn!(
                        "Presentation: current monitor unavailable; attention deferred"
                    );
                    continue;
                };

                state.snapshot = Some(PresentationSnapshot {
                    window_left: rect.left,
                    window_top: rect.top,
                    window_width: rect.width().max(1) as u32,
                    window_height: rect.height().max(1) as u32,
                    window_level: window.window_level,
                    cursor_hit_test: cursor.hit_test,
                    camera_transform: camera_transform.clone(),
                    placement: environment.placement,
                    attached_anchor: environment.attached_anchor,
                    hidden_for_target_suspend:
                        environment.hidden_for_target_suspend,
                });

                // Attention owns Nera completely. Do not keep the desktop
                // sitting attachment logically active: otherwise Phase 4,
                // animation support state, and presentation all compete.
                //
                // target_window itself is intentionally preserved so a window
                // attachment can be restored after Attention.
                environment.placement = PlacementState::Free;
                environment.hidden_for_target_suspend = false;
                environment.set_attached_anchor(AvatarAnchor::Feet);

                // Drop Sitting/SittingExit immediately. Alarm presentation is
                // not part of the sitting state machine.
                animation.suspend_support_for_attention();

                reset_pointer_state(
                    &mut interaction,
                    &mut hovered,
                    &mut head_pat,
                );

                state.mode = PresentationMode::Attention(kind);
                state.attention_enter_progress = 0.0;

                let layout = compute_attention_layout(
                    monitor.bounds,
                    rect.width().max(1) as u32,
                    rect.height().max(1) as u32,
                    projection,
                    &bone_query,
                    None,
                );

                state.attention_locked_center = Some(
                    Vec2::new(
                        layout.camera_x,
                        layout.camera_y,
                    ),
                );

                // Preallocate a wide, stable transparent canvas for the whole
                // alarm presentation. This removes repeated HWND resizes when
                // Wave moves its hands left/right.
                state.attention_locked_window_width = (
                    monitor.bounds.width().max(1) as f32
                        * ATTENTION_MAX_WINDOW_WIDTH_RATIO
                )
                    .round() as u32;

                state.attention_locked_window_width =
                    state.attention_locked_window_width
                        .max(layout.window_width);

                state.attention_peak_camera_z = layout.camera_z;

                apply_attention_window(
                    &mut window,
                    monitor.bounds,
                    state.attention_locked_window_width,
                    layout.window_height,
                    0.0,
                );
                apply_attention_camera(
                    &mut camera_transform,
                    layout.camera_x,
                    layout.camera_y,
                    layout.look_at_x,
                    layout.look_at_y,
                    state.attention_peak_camera_z,
                );

                info!(
                    "Presentation -> Attention({kind:?}) | monitor={}x{} @ {},{}",
                    monitor.bounds.width(),
                    monitor.bounds.height(),
                    monitor.bounds.left,
                    monitor.bounds.top,
                );

                entered.write(PresentationEntered { kind });
            }

            PresentationCommand::ExitAttention => {
                let PresentationMode::Attention(kind) = state.mode else {
                    continue;
                };

                // Runs before the snapshot check on purpose: even a degraded
                // exit that cannot restore the window must still hand input
                // ownership back cleanly.
                reset_pointer_state(
                    &mut interaction,
                    &mut hovered,
                    &mut head_pat,
                );

                let Some(snapshot) = state.snapshot.take() else {
                    warn!(
                        "Presentation: attention mode had no restore snapshot"
                    );
                    state.mode = PresentationMode::DesktopPet;
                    state.attention_locked_center = None;
                    state.attention_locked_window_width = 0;
                    state.attention_peak_camera_z = 0.0;
                    continue;
                };

                let Ok((mut window, mut cursor)) =
                    window_query.single_mut()
                else {
                    warn!(
                        "Presentation: primary Nera window unavailable during restore"
                    );
                    continue;
                };

                let Ok((_projection, mut camera_transform)) =
                    camera_query.single_mut()
                else {
                    warn!(
                        "Presentation: Nera camera unavailable during restore"
                    );
                    continue;
                };

                restore_desktop_window(
                    &mut window,
                    &mut cursor,
                    &snapshot,
                );
                *camera_transform = snapshot.camera_transform;

                // Restore the exact desktop attachment that existed before
                // Attention. If it was sitting, return directly to SittingLoop
                // instead of replaying SittingEnter.
                environment.placement = snapshot.placement;
                environment.set_attached_anchor(snapshot.attached_anchor);
                environment.hidden_for_target_suspend =
                    snapshot.hidden_for_target_suspend;

                let restored_sitting = matches!(
                    snapshot.placement,
                    PlacementState::PendingWindow { .. }
                        | PlacementState::PendingTaskbar { .. }
                        | PlacementState::Window(_)
                        | PlacementState::Taskbar(_)
                );

                animation.restore_support_after_attention(
                    restored_sitting,
                );

                state.mode = PresentationMode::DesktopPet;
                state.attention_enter_progress = 0.0;
                state.attention_locked_center = None;
                state.attention_locked_window_width = 0;
                state.attention_peak_camera_z = 0.0;

                info!(
                    "Presentation -> DesktopPet | restored={}x{} @ {},{}",
                    snapshot.window_width,
                    snapshot.window_height,
                    snapshot.window_left,
                    snapshot.window_top,
                );

                exited.write(PresentationExited { kind });
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Attention presentation
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct AttentionLayout {
    window_width: u32,
    window_height: u32,
    camera_z: f32,
    camera_x: f32,
    camera_y: f32,
    look_at_x: f32,
    look_at_y: f32,
}

/// Measure the portrait crop directly from humanoid semantics.
///
/// We frame:
///
///     hair/head margin
///          ↓
///   max(HEAD / HANDS / ARMS)
///          │
///        torso
///          │
///        HIPS
///          ↓
///     tiny waist margin
///
/// Legs and feet are deliberately excluded from the camera-distance formula,
/// so they fall below the viewport instead of shrinking Nera to fit full-body.
#[derive(Clone, Copy, Debug)]
struct UpperBodyBounds {
    min_x: f32,
    max_x: f32,
    bottom_y: f32,
    top_y: f32,
}

impl UpperBodyBounds {
    fn width(self) -> f32 {
        (self.max_x - self.min_x).abs().max(0.20)
    }

    fn height(self) -> f32 {
        (self.top_y - self.bottom_y)
            .abs()
            .max(MIN_UPPER_BODY_WORLD_HEIGHT)
    }

    fn center_x(self) -> f32 {
        (self.min_x + self.max_x) * 0.5
    }

    fn center_y(self) -> f32 {
        (self.bottom_y + self.top_y) * 0.5
    }
}

/// Dynamic 2D portrait bounds from the *currently animated* humanoid pose.
///
/// This solves both classes of alarm framing:
///
/// - raised hands increase `top_y`,
/// - spread arms/hands increase `min_x..max_x`.
///
/// Legs remain excluded so the composition stays half-body.
fn upper_body_world_metrics(
    bone_query: &Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
) -> Option<UpperBodyBounds> {
    let mut head_y = None;
    let mut hips_y = None;

    let mut min_x: Option<f32> = None;
    let mut max_x: Option<f32> = None;
    let mut animated_top_y: Option<f32> = None;

    for (bone, transform) in bone_query {
        let position = transform.translation();

        let relevant = matches!(
            bone.as_str(),
            "head"
                | "neck"
                | "chest"
                | "upperChest"
                | "hips"
                | "leftShoulder"
                | "rightShoulder"
                | "leftUpperArm"
                | "rightUpperArm"
                | "leftLowerArm"
                | "rightLowerArm"
                | "leftHand"
                | "rightHand"
        );

        if !relevant {
            continue;
        }

        min_x = Some(
            min_x
                .map(|value| value.min(position.x))
                .unwrap_or(position.x),
        );

        max_x = Some(
            max_x
                .map(|value| value.max(position.x))
                .unwrap_or(position.x),
        );

        animated_top_y = Some(
            animated_top_y
                .map(|value| value.max(position.y))
                .unwrap_or(position.y),
        );

        match bone.as_str() {
            "head" => head_y = Some(position.y),
            "hips" => hips_y = Some(position.y),
            _ => {}
        }
    }

    let head = head_y?;
    let hips = hips_y?;
    let mut min_x = min_x?;
    let mut max_x = max_x?;
    let animated_top = animated_top_y.unwrap_or(head);

    let torso_span =
        (head - hips).abs().max(MIN_UPPER_BODY_WORLD_HEIGHT);

    // Mesh extends beyond the humanoid joints: sleeves/fingers are especially
    // obvious during wide-arm animations.
    let horizontal_margin =
        torso_span * UPPER_BODY_HORIZONTAL_MARGIN_RATIO;

    min_x -= horizontal_margin;
    max_x += horizontal_margin;

    let top_y =
        animated_top + torso_span * UPPER_BODY_TOP_MARGIN_RATIO;

    let bottom_y =
        hips - torso_span * UPPER_BODY_BOTTOM_MARGIN_RATIO;

    Some(UpperBodyBounds {
        min_x,
        max_x,
        bottom_y,
        top_y,
    })
}

/// Perspective camera distance for a target portrait height:
///
/// projected_fraction
///     = world_height / (2 * distance * tan(fov_y / 2))
///
/// therefore:
///
/// distance
///     = world_height
///       / (2 * projected_fraction * tan(fov_y / 2))
fn compute_attention_layout(
    monitor_bounds: crate::platform::PhysicalRect,
    original_width: u32,
    original_height: u32,
    projection: &Projection,
    bone_query: &Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
    locked_center: Option<Vec2>,
) -> AttentionLayout {
    let monitor_width = monitor_bounds.width().max(1) as f32;
    let monitor_height = monitor_bounds.height().max(1) as f32;

    let original_aspect =
        original_width.max(1) as f32 / original_height.max(1) as f32;

    // Do NOT clamp against the normal 600x950 pet window. On a 900px-tall
    // monitor that would force Attention back to almost full-screen height.
    // Presentation dimensions are derived from the active monitor instead.
    let minimum_window_height =
        (monitor_height * 0.52).max(420.0).min(monitor_height);

    let window_height =
        (monitor_height * ATTENTION_WINDOW_MONITOR_HEIGHT_RATIO)
            .round()
            .clamp(
                minimum_window_height,
                monitor_height,
            ) as u32;

    let minimum_window_width =
        (monitor_width * 0.20).max(320.0).min(monitor_width);

    let window_width =
        (window_height as f32 * original_aspect)
            .round()
            .clamp(
                minimum_window_width,
                monitor_width,
            ) as u32;

    let bounds =
        upper_body_world_metrics(bone_query)
            .unwrap_or(UpperBodyBounds {
                min_x: -0.45,
                max_x: 0.45,
                bottom_y: 0.55,
                top_y: 1.55,
            });

    let vertical_fov = match projection {
        Projection::Perspective(perspective) => perspective.fov,
        _ => std::f32::consts::FRAC_PI_4,
    };

    let vertical_half_tan =
        (vertical_fov * 0.5).tan().max(0.01);

    // The first Attention frame chooses the composition center. Later frames
    // reuse it instead of following asymmetric arm motion.
    let center = locked_center.unwrap_or_else(|| {
        Vec2::new(
            bounds.center_x(),
            bounds.center_y(),
        )
    });

    // Fit relative to the LOCKED center. This matters when one arm extends
    // farther than the other: the camera stays still and the pose consumes
    // more room on that side instead of dragging Nera across the screen.
    let vertical_half_extent = (
        (bounds.top_y - center.y)
            .abs()
            .max((center.y - bounds.bottom_y).abs())
    )
        .max(MIN_UPPER_BODY_WORLD_HEIGHT * 0.5);

    let horizontal_half_extent = (
        (bounds.max_x - center.x)
            .abs()
            .max((center.x - bounds.min_x).abs())
    )
        .max(0.10);

    // Vertical framing remains authoritative for the large half-body look.
    let vertical_distance =
        vertical_half_extent
            / (
                ATTENTION_UPPER_BODY_VIEW_FRACTION
                    * vertical_half_tan
            );

    // At that same camera distance, widen the transparent canvas enough for
    // the current horizontal extent.
    let required_aspect =
        horizontal_half_extent
            / (
                vertical_distance.max(0.01)
                    * vertical_half_tan
                    * ATTENTION_UPPER_BODY_HORIZONTAL_FRACTION
            );

    let original_window_aspect =
        window_width.max(1) as f32
            / window_height.max(1) as f32;

    let desired_aspect =
        original_window_aspect.max(required_aspect);

    let max_window_width =
        (monitor_width * ATTENTION_MAX_WINDOW_WIDTH_RATIO)
            .round()
            .max(window_width as f32)
            .min(monitor_width);

    let widened_window_width =
        (window_height as f32 * desired_aspect)
            .round()
            .clamp(
                window_width as f32,
                max_window_width,
            ) as u32;

    let final_aspect =
        widened_window_width.max(1) as f32
            / window_height.max(1) as f32;

    let horizontal_half_tan =
        (vertical_half_tan * final_aspect)
            .max(0.01);

    let horizontal_distance =
        horizontal_half_extent
            / (
                ATTENTION_UPPER_BODY_HORIZONTAL_FRACTION
                    * horizontal_half_tan
            );

    let camera_z =
        vertical_distance.max(horizontal_distance);

    AttentionLayout {
        window_width: widened_window_width,
        window_height,
        camera_z,
        camera_x: center.x,
        camera_y: center.y,
        look_at_x: center.x,
        look_at_y: center.y,
    }
}

fn ease_out_cubic(t: f32) -> f32 {
    let clamped = t.clamp(0.0, 1.0);
    1.0 - (1.0 - clamped).powi(3)
}

fn apply_attention_window(
    window: &mut Window,
    monitor_bounds: crate::platform::PhysicalRect,
    width: u32,
    height: u32,
    enter_progress: f32,
) {
    let monitor_width = monitor_bounds.width().max(1);

    // Horizontal center.
    let left = monitor_bounds.left
        + ((monitor_width - width as i32) / 2).max(0);

    // Final position is bottom-anchored. This is what creates the
    // "Nera emerges from the bottom of the screen" composition.
    let final_top = monitor_bounds.bottom - height as i32;

    // Start with the whole compact window just below the monitor.
    let start_top = monitor_bounds.bottom;

    let eased = ease_out_cubic(enter_progress);
    let top = (
        start_top as f32
            + (final_top - start_top) as f32 * eased
    )
        .round() as i32;

    window.position = WindowPosition::At(IVec2::new(left, top));

    window.resolution.set_physical_resolution(
        width.max(1),
        height.max(1),
    );

    window.transparent = true;
    window.window_level = WindowLevel::AlwaysOnTop;
}

fn apply_attention_camera(
    transform: &mut Transform,
    camera_x: f32,
    camera_y: f32,
    look_at_x: f32,
    look_at_y: f32,
    camera_z: f32,
) {
    *transform = Transform::from_xyz(
        camera_x,
        camera_y,
        camera_z.max(0.16),
    )
    .looking_at(
        Vec3::new(look_at_x, look_at_y, 0.0),
        Vec3::Y,
    );
}

fn restore_desktop_window(
    window: &mut Window,
    cursor: &mut CursorOptions,
    snapshot: &PresentationSnapshot,
) {
    window.position = WindowPosition::At(IVec2::new(
        snapshot.window_left,
        snapshot.window_top,
    ));

    window.resolution.set_physical_resolution(
        snapshot.window_width,
        snapshot.window_height,
    );

    window.window_level = snapshot.window_level;
    cursor.hit_test = snapshot.cursor_hit_test;
}

/// Phase 4 environment/window-follow systems still run while Attention is
/// active. Instead of invasively adding Presentation dependencies across all
/// of those stable modules, Phase 5.55 reasserts the presentation at `Last`.
///
/// In Phase 5.7 Desktop Physics we can formalize a shared desktop-world system
/// set, but this keeps the new boundary small and regression-safe today.
fn maintain_attention_presentation(
    time: Res<Time>,
    mut state: ResMut<PresentationState>,
    environment: Res<DesktopEnvironment>,
    mut window_query: Query<&mut Window, With<PrimaryWindow>>,
    mut camera_query: Query<
        (&Projection, &mut Transform),
        With<crate::avatar::NeraMainCamera>,
    >,
    bone_query: Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
) {
    if !state.is_attention() {
        return;
    }

    let Some(monitor) = environment.monitor else {
        return;
    };

    let Some(snapshot) = state.snapshot.as_ref() else {
        return;
    };

    let original_width = snapshot.window_width;
    let original_height = snapshot.window_height;

    state.attention_enter_progress =
        (
            state.attention_enter_progress
                + time.delta_secs() / ATTENTION_ENTER_SECONDS
        )
            .clamp(0.0, 1.0);

    let enter_progress = state.attention_enter_progress;

    let Ok((projection, mut camera_transform)) =
        camera_query.single_mut()
    else {
        return;
    };

    let layout = compute_attention_layout(
        monitor.bounds,
        original_width,
        original_height,
        projection,
        &bone_query,
        state.attention_locked_center,
    );

    // Never shrink the attention canvas or zoom back in during the same
    // presentation. Animation loops therefore cannot create resize/zoom
    // oscillation.
    state.attention_locked_window_width =
        state.attention_locked_window_width
            .max(layout.window_width);

    state.attention_peak_camera_z =
        state.attention_peak_camera_z
            .max(layout.camera_z);

    let stable_width = state.attention_locked_window_width;
    let stable_camera_z = state.attention_peak_camera_z;

    if let Ok(mut window) = window_query.single_mut() {
        apply_attention_window(
            &mut window,
            monitor.bounds,
            stable_width,
            layout.window_height,
            enter_progress,
        );
    }

    apply_attention_camera(
        &mut camera_transform,
        layout.camera_x,
        layout.camera_y,
        layout.look_at_x,
        layout.look_at_y,
        stable_camera_z,
    );
}

// -----------------------------------------------------------------------------
// Run conditions for future integration
// -----------------------------------------------------------------------------

/// Future systems that should only operate on the small desktop pet can use:
///
/// `.run_if(presentation::desktop_pet_active)`
///
/// Phase 5.55 does not force this across old systems yet; the Last-stage
/// attention authority is enough for the foundation test.
pub(crate) fn desktop_pet_active(
    state: Res<PresentationState>,
) -> bool {
    state.is_desktop_pet()
}


// -----------------------------------------------------------------------------
// Pure state tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_starts_as_desktop_pet() {
        let state = PresentationState::default();

        assert_eq!(
            state.mode,
            PresentationMode::DesktopPet
        );
        assert!(state.is_desktop_pet());
        assert!(!state.is_attention());
    }

    #[test]
    fn attention_mode_is_distinct_from_desktop_pet() {
        let state = PresentationState {
            mode: PresentationMode::Attention(
                AttentionKind::Alarm,
            ),
            ..Default::default()
        };

        assert!(state.is_attention());
        assert!(!state.is_desktop_pet());
    }

    #[test]
    fn attention_is_upper_body_portrait() {
        // Bounds are inclusive: 0.80 is the tightest framing that still keeps
        // the hips/waist in shot, so a tuned-to-the-edge value is valid.
        assert!(
            ATTENTION_UPPER_BODY_VIEW_FRACTION >= 0.80
        );
        assert!(
            ATTENTION_UPPER_BODY_VIEW_FRACTION <= 0.95
        );
    }

    #[test]
    fn perspective_distance_formula_scales_with_portrait_height() {
        let fov = std::f32::consts::FRAC_PI_4;
        let fraction = 0.80;

        let distance_for_one =
            1.0 / (2.0 * fraction * (fov * 0.5).tan());
        let distance_for_two =
            2.0 / (2.0 * fraction * (fov * 0.5).tan());

        assert!(
            (distance_for_two / distance_for_one - 2.0).abs()
                < 0.001
        );
    }
    #[test]
    fn attention_entry_eases_from_below_to_final_position() {
        assert!((ease_out_cubic(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < f32::EPSILON);
        assert!(ease_out_cubic(0.5) > 0.5);
    }

    #[test]
    fn attention_portrait_keeps_dynamic_headroom() {
        // Real raised-hand extent is measured from the animated skeleton;
        // this ratio is now only a safety margin.
        assert!(UPPER_BODY_TOP_MARGIN_RATIO >= 0.15);
        assert!(
            ATTENTION_UPPER_BODY_VIEW_FRACTION
                <= 0.82
        );
    }

    #[test]
    fn wide_pose_prefers_wider_canvas_before_backing_camera() {
        let vertical_half_tan = 0.5_f32;
        let vertical_distance = 1.0_f32;
        let body_width = 1.8_f32;

        let required_aspect =
            body_width
                / (
                    2.0
                        * vertical_distance
                        * vertical_half_tan
                        * ATTENTION_UPPER_BODY_HORIZONTAL_FRACTION
                );

        assert!(required_aspect > 1.0);
        assert!(ATTENTION_MAX_WINDOW_WIDTH_RATIO < 1.0);
    }

    #[test]
    fn portrait_bounds_center_tracks_asymmetric_pose() {
        let bounds = UpperBodyBounds {
            min_x: -0.4,
            max_x: 0.8,
            bottom_y: 0.5,
            top_y: 1.6,
        };

        assert!((bounds.center_x() - 0.2).abs() < 0.001);
        assert!((bounds.center_y() - 1.05).abs() < 0.001);
    }

    #[test]
    fn locked_center_does_not_follow_asymmetric_wave_pose() {
        let locked = Vec2::new(0.0, 1.0);

        let bounds = UpperBodyBounds {
            min_x: -0.3,
            max_x: 1.0,
            bottom_y: 0.5,
            top_y: 1.7,
        };

        let horizontal_half_extent = (bounds.max_x - locked.x)
            .abs()
            .max((locked.x - bounds.min_x).abs());

        assert!((locked.x - 0.0).abs() < f32::EPSILON);
        assert!((horizontal_half_extent - 1.0).abs() < 0.001);
    }

    #[test]
    fn monotonic_framing_never_shrinks_inside_attention() {
        let previous_width = 1200_u32;
        let requested_width = 900_u32;
        let stable_width = previous_width.max(requested_width);

        let previous_z = 1.4_f32;
        let requested_z = 1.1_f32;
        let stable_z = previous_z.max(requested_z);

        assert_eq!(stable_width, 1200);
        assert!((stable_z - 1.4).abs() < 0.001);
    }

    /// Phase 5.75 regression: an alarm firing mid-drag freezes the pointer
    /// state machine, because `drag_nera_window` is gated off and never sees
    /// the LMB release. A frozen `Pressed`/`Dragging` makes
    /// `update_click_through` keep `hit_test = true`, so the pet window would
    /// swallow desktop clicks for the rest of the session.
    #[test]
    fn attention_handoff_drops_a_frozen_drag_gesture() {
        use crate::interaction::{
            BodyPart,
            InteractionState,
        };
        use std::time::Instant;

        let now = Instant::now();

        let mut interaction = DesktopPetInteraction {
            momentum: Default::default(),
            state: InteractionState::Dragging,
            pressed_body_part: Some(BodyPart::Head),
            right_pressed_body_part: Some(BodyPart::Torso),
            drag_offset: Some(IVec2::new(12, 34)),
            drag_start_cursor: Some(IVec2::new(100, 200)),
            last_drag_cursor: Some(IVec2::new(140, 260)),
            drag_started_at: Some(now),
            right_press_origin: Some(IVec2::new(90, 190)),
            snap_cursor: Some(IVec2::new(80, 180)),
            sitting_lock_until: Some(now),
            unsnap_cooldown_until: Some(now),
            guard_center: Some(IVec2::new(70, 170)),
            recent_unsnap_surface_y: Some(1080),
        };

        let mut hovered = HoveredBodyPart {
            part: Some(BodyPart::Head),
        };

        let mut head_pat = HeadPatGesture {
            started_at: Some(now),
            last_cursor: Some(IVec2::new(10, 10)),
            total_travel: 120.0,
            x_reversals: 3,
            ..Default::default()
        };

        reset_pointer_state(
            &mut interaction,
            &mut hovered,
            &mut head_pat,
        );

        assert_eq!(interaction.state, InteractionState::None);
        assert!(interaction.pressed_body_part.is_none());
        assert!(interaction.right_pressed_body_part.is_none());
        assert!(interaction.drag_offset.is_none());
        assert!(interaction.drag_start_cursor.is_none());
        assert!(interaction.last_drag_cursor.is_none());
        assert!(interaction.drag_started_at.is_none());
        assert!(interaction.right_press_origin.is_none());
        assert!(interaction.snap_cursor.is_none());

        // Anti-resnap guards are recorded against the pre-Attention window
        // rect, which Attention resizes and then restores.
        assert!(interaction.sitting_lock_until.is_none());
        assert!(interaction.unsnap_cooldown_until.is_none());
        assert!(interaction.guard_center.is_none());
        assert!(interaction.recent_unsnap_surface_y.is_none());

        assert!(hovered.part.is_none());
        assert!(head_pat.started_at.is_none());
        assert!(head_pat.last_cursor.is_none());
        assert_eq!(head_pat.total_travel, 0.0);
        assert_eq!(head_pat.x_reversals, 0);
    }

    /// The Settings path must keep its original, narrower reset: it only loses
    /// pointer capture, never the anti-resnap guards, because the pet HWND is
    /// not resized while Settings is open.
    #[test]
    fn settings_reset_preserves_unsnap_guards() {
        use crate::interaction::InteractionState;
        use std::time::Instant;

        let now = Instant::now();

        let mut interaction = DesktopPetInteraction {
            state: InteractionState::Pressed,
            drag_offset: Some(IVec2::new(4, 5)),
            unsnap_cooldown_until: Some(now),
            guard_center: Some(IVec2::new(60, 70)),
            recent_unsnap_surface_y: Some(900),
            ..Default::default()
        };

        interaction.reset_gesture_state();

        assert_eq!(interaction.state, InteractionState::None);
        assert!(interaction.drag_offset.is_none());

        assert!(interaction.unsnap_cooldown_until.is_some());
        assert_eq!(interaction.guard_center, Some(IVec2::new(60, 70)));
        assert_eq!(interaction.recent_unsnap_surface_y, Some(900));
    }

}
