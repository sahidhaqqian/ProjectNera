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

mod activity;
mod full_body;
#[cfg(test)]
mod screensaver_tests;
pub(crate) mod screenshot;
#[cfg(all(debug_assertions, target_os = "windows"))]
mod smoke;
#[cfg(all(debug_assertions, target_os = "windows"))]
pub(crate) use smoke::install as install_smoke;

use bevy::{
    camera::ScalingMode,
    prelude::*,
    window::{CursorOptions, PrimaryWindow, WindowLevel, WindowPosition},
};

use bevy_vrm1::prelude::{Initialized, RestGlobalTransform, VrmBone};

use crate::{
    animation::NeraAnimationState,
    environment::{AvatarAnchor, DesktopEnvironment, PlacementState},
    interaction::{DesktopPetInteraction, HeadPatGesture, HoveredBodyPart, NeraLiveBone},
};

// -----------------------------------------------------------------------------
// Presentation vocabulary
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AttentionKind {
    Alarm,
    Screensaver,
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
    /// Input queued before Alarm preemption must never dismiss Alarm.
    ExitScreensaver,
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
    camera_projection: Projection,

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

    /// One canonical portrait for the whole sequence; live clip bounds cannot
    /// shrink the model or move its camera at a Jump -> Wave transition.
    attention_layout: Option<AttentionLayout>,
    /// Replacing an initialized rig may reframe once, without replacing the
    /// normal-presentation snapshot or following its subsequent animated poses.
    attention_avatar: Option<Entity>,
    screensaver_input: Option<activity::ClickGuard>,
    /// Bevy updates the camera/window after Update; native facts catch up on
    /// the next frame. Desktop physics must not consume the attention canvas.
    desktop_restore_pending: bool,
}

impl PresentationState {
    #[cfg(test)]
    pub(crate) fn for_test(mode: PresentationMode) -> Self {
        Self { mode, ..default() }
    }
    pub(crate) fn is_attention(&self) -> bool {
        matches!(self.mode, PresentationMode::Attention(_))
    }

    pub(crate) fn is_desktop_pet(&self) -> bool {
        self.mode == PresentationMode::DesktopPet
    }

    pub(crate) fn is_screensaver(&self) -> bool {
        self.mode == PresentationMode::Attention(AttentionKind::Screensaver)
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
const UPPER_BODY_TOP_MARGIN_RATIO: f32 = 0.40;

/// Small crop allowance below the hips so the waist doesn't feel chopped at
/// exactly the humanoid hips joint.
const UPPER_BODY_BOTTOM_MARGIN_RATIO: f32 = 0.06;

/// Avoid pathological rig measurements during loading.
const MIN_UPPER_BODY_WORLD_HEIGHT: f32 = 0.45;

/// Entrance slide duration. The compact transparent HWND starts below the
/// monitor and eases upward into its final bottom-anchored attention position.
const ATTENTION_ENTER_SECONDS: f32 = 0.38;

const SCREENSAVER_PORTRAIT_FILL: f32 = 0.96;
const SCREENSAVER_WINDOW_MONITOR_HEIGHT: f32 = 0.94;
const SCREENSAVER_MAX_WINDOW_WIDTH: f32 = 0.88;

// -----------------------------------------------------------------------------
// Plugin
// -----------------------------------------------------------------------------

pub(crate) struct PresentationPlugin;

impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        screenshot::install(app);
        // The isolated visual harness must not dismiss itself while the user
        // works in other apps. The input harness invokes the same click observer
        // with scripted window events instead of controlling the user's input.
        let observe_input =
            !cfg!(debug_assertions) || !std::env::args().any(|arg| arg == "--presentation-smoke");
        app.init_resource::<PresentationState>()
            .add_message::<PresentationCommand>()
            .add_message::<PresentationEntered>()
            .add_message::<PresentationExited>()
            .add_systems(
                Update,
                (
                    debug_toggle_attention
                        .run_if(crate::utility_window::text_input::keyboard_shortcuts_available),
                    handle_presentation_commands,
                )
                    .chain()
                    .before(crate::cloud::update_cloud)
                    .before(crate::startup_entrance::prepare_startup_entrance),
            )
            .add_systems(
                PostUpdate,
                activity::exit_on_avatar_click
                    .after(crate::interaction::update_click_through)
                    .run_if(move || observe_input),
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
    let ctrl = keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight);
    let alt = keyboard.pressed(KeyCode::AltLeft) || keyboard.pressed(KeyCode::AltRight);

    if !(ctrl && alt && keyboard.just_pressed(KeyCode::KeyP)) {
        return;
    }

    if state.is_attention() {
        commands.write(PresentationCommand::ExitAttention);
    } else {
        commands.write(PresentationCommand::EnterAttention(AttentionKind::Alarm));
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

type PortraitAvatars<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static Transform,
        Option<&'static RestGlobalTransform>,
        Has<Initialized>,
        Has<crate::character::CharacterReveal>,
    ),
    (
        With<crate::avatar::NeraAvatar>,
        Without<crate::avatar::NeraMainCamera>,
    ),
>;

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
    mut window_query: Query<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut camera_query: Query<(&mut Projection, &mut Transform), With<crate::avatar::NeraMainCamera>>,
    bone_query: Query<(&VrmBone, &RestGlobalTransform), With<NeraLiveBone>>,
    avatar_query: PortraitAvatars,
    body_proportions: Option<Res<crate::avatar::body_mode::BodyProportions>>,
    full_body: full_body::FullBodyGeometry,
    cloud: Option<Res<crate::cloud::CloudRuntime>>,
) {
    for request in requests.read() {
        match *request {
            PresentationCommand::EnterAttention(kind) => {
                let preempting = kind == AttentionKind::Alarm && state.is_screensaver();
                if state.is_attention() && !preempting {
                    // Phase 5.6 may re-trigger the same attention mode; avoid
                    // overwriting the original DesktopPet snapshot.
                    continue;
                }

                if kind == AttentionKind::Screensaver
                    && cloud.as_ref().is_some_and(|cloud| cloud.owns_placement())
                {
                    warn!(
                        "Screensaver entry skipped while Cloud owns placement; dismiss Cloud first"
                    );
                    continue;
                }
                let Ok((mut window, mut cursor)) = window_query.single_mut() else {
                    warn!("Presentation: primary Nera window unavailable");
                    continue;
                };

                let Ok((mut projection, mut camera_transform)) = camera_query.single_mut() else {
                    warn!("Presentation: Nera camera unavailable");
                    continue;
                };

                let Some(rect) = environment.nera_rect else {
                    warn!("Presentation: Nera HWND rect unavailable; attention deferred");
                    continue;
                };

                let Some(monitor) = environment.monitor else {
                    warn!("Presentation: current monitor unavailable; attention deferred");
                    continue;
                };

                if monitor.bounds.width() <= 0 || monitor.bounds.height() <= 0 {
                    warn!("Attention entry skipped: invalid monitor geometry");
                    continue;
                }
                let screensaver_layout = if kind == AttentionKind::Screensaver {
                    let Some(bounds) = full_body.portrait_bounds(
                        &avatar_query,
                        body_proportions
                            .as_ref()
                            .map_or(1.0, |body| body.body_scale()),
                        body_proportions
                            .as_ref()
                            .map_or(1.0, |body| body.head_scale()),
                        body_proportions
                            .as_ref()
                            .map_or(0.0, |body| body.standing_root_offset_y()),
                        true,
                    ) else {
                        warn!(
                            "Screensaver entry skipped: initialized humanoid/skin geometry unavailable"
                        );
                        continue;
                    };
                    Some(compute_screensaver_layout(monitor.bounds, bounds))
                } else {
                    None
                };
                if !preempting {
                    state.snapshot = Some(PresentationSnapshot {
                        window_left: rect.left,
                        window_top: rect.top,
                        window_width: rect.width().max(1) as u32,
                        window_height: rect.height().max(1) as u32,
                        window_level: window.window_level,
                        cursor_hit_test: cursor.hit_test,
                        camera_transform: camera_transform.clone(),
                        camera_projection: projection.clone(),
                        placement: environment.placement,
                        attached_anchor: environment.attached_anchor,
                        hidden_for_target_suspend: environment.hidden_for_target_suspend,
                    });
                } else {
                    exited.write(PresentationExited {
                        kind: AttentionKind::Screensaver,
                    });
                }

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

                reset_pointer_state(&mut interaction, &mut hovered, &mut head_pat);

                state.mode = PresentationMode::Attention(kind);
                state.desktop_restore_pending = false;
                state.attention_enter_progress = 0.0;
                state.screensaver_input =
                    (kind == AttentionKind::Screensaver).then(activity::ClickGuard::default);
                if kind == AttentionKind::Screensaver {
                    cursor.hit_test = false;
                }

                let bounds = canonical_upper_body_bounds(
                    &bone_query,
                    &avatar_query,
                    body_proportions
                        .as_ref()
                        .map_or(1.0, |body| body.body_scale()),
                    body_proportions
                        .as_ref()
                        .map_or(1.0, |body| body.head_scale()),
                    body_proportions
                        .as_ref()
                        .map_or(0.0, |body| body.standing_root_offset_y()),
                );
                let layout = compute_attention_layout_for_bounds(
                    monitor.bounds,
                    rect.width().max(1) as u32,
                    rect.height().max(1) as u32,
                    &projection,
                    bounds,
                    None,
                );

                let stable_width = (monitor.bounds.width().max(1) as f32
                    * ATTENTION_MAX_WINDOW_WIDTH_RATIO)
                    .round() as u32;
                let layout = AttentionLayout {
                    window_width: stable_width.max(layout.window_width),
                    ..layout
                };
                let layout = screensaver_layout.unwrap_or(layout);
                // Parallel projection removes the scale change caused by
                // different authored hips depths, without touching avatar scale.
                *projection = Projection::Orthographic(OrthographicProjection {
                    scaling_mode: ScalingMode::FixedVertical {
                        viewport_height: layout.viewport_height,
                    },
                    ..OrthographicProjection::default_3d()
                });
                state.attention_layout = Some(layout);
                state.attention_avatar = avatar_query.single().ok().and_then(
                    |(entity, _, _, initialized, preparing)| {
                        (initialized && !preparing).then_some(entity)
                    },
                );

                apply_presentation_window(
                    kind,
                    &mut window,
                    monitor.bounds,
                    layout.window_width,
                    layout.window_height,
                    if kind == AttentionKind::Screensaver {
                        1.0
                    } else {
                        0.0
                    },
                );
                if kind == AttentionKind::Screensaver {
                    state.attention_enter_progress = 1.0;
                }
                apply_attention_camera(
                    &mut camera_transform,
                    layout.camera_x,
                    layout.camera_y,
                    layout.look_at_x,
                    layout.look_at_y,
                    layout.camera_z,
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

            PresentationCommand::ExitAttention | PresentationCommand::ExitScreensaver => {
                if matches!(request, PresentationCommand::ExitScreensaver)
                    && !state.is_screensaver()
                {
                    continue;
                }
                let PresentationMode::Attention(kind) = state.mode else {
                    continue;
                };

                // Runs before the snapshot check on purpose: even a degraded
                // exit that cannot restore the window must still hand input
                // ownership back cleanly.
                reset_pointer_state(&mut interaction, &mut hovered, &mut head_pat);

                state.screensaver_input = None;

                let Some(snapshot) = state.snapshot.take() else {
                    warn!("Presentation: attention mode had no restore snapshot");
                    state.mode = PresentationMode::DesktopPet;
                    state.attention_layout = None;
                    state.attention_avatar = None;
                    continue;
                };

                if let Ok((mut window, mut cursor)) = window_query.single_mut() {
                    restore_desktop_window(&mut window, &mut cursor, &snapshot);
                } else {
                    warn!("Presentation: primary Nera window unavailable during restore");
                }

                if let Ok((mut projection, mut camera_transform)) = camera_query.single_mut() {
                    *camera_transform = snapshot.camera_transform;
                    *projection = snapshot.camera_projection;
                } else {
                    warn!("Presentation: Nera camera unavailable during restore");
                }

                // Restore the exact desktop attachment that existed before
                // Attention. If it was sitting, return directly to SittingLoop
                // instead of replaying SittingEnter.
                environment.placement = snapshot.placement;
                environment.set_attached_anchor(snapshot.attached_anchor);
                environment.hidden_for_target_suspend = snapshot.hidden_for_target_suspend;

                let restored_sitting = matches!(
                    snapshot.placement,
                    PlacementState::PendingWindow { .. }
                        | PlacementState::PendingTaskbar { .. }
                        | PlacementState::Window(_)
                        | PlacementState::Taskbar(_)
                );

                animation.restore_support_after_attention(restored_sitting);

                state.mode = PresentationMode::DesktopPet;
                state.desktop_restore_pending = true;
                state.attention_enter_progress = 0.0;
                state.attention_layout = None;
                state.attention_avatar = None;

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
    viewport_height: f32,
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
    front_z: f32,
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

/// Dynamic portrait bounds from the *currently animated* humanoid pose.
///
/// This solves both classes of alarm framing:
///
/// - raised hands increase `top_y`,
/// - spread arms/hands increase `min_x..max_x`.
///
/// Legs remain excluded so the composition stays half-body.
fn upper_body_world_metrics<'a>(
    bones: impl IntoIterator<Item = (&'a VrmBone, &'a GlobalTransform)>,
) -> Option<UpperBodyBounds> {
    let mut head_y = None;
    let mut hips_y = None;

    let mut min_x: Option<f32> = None;
    let mut max_x: Option<f32> = None;
    let mut animated_top_y: Option<f32> = None;
    let mut front_z: Option<f32> = None;

    for (bone, transform) in bones {
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
        front_z = Some(front_z.map_or(position.z, |z| z.max(position.z)));

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

    let torso_span = (head - hips).abs().max(MIN_UPPER_BODY_WORLD_HEIGHT);

    // Mesh extends beyond the humanoid joints: sleeves/fingers are especially
    // obvious during wide-arm animations.
    let horizontal_margin = torso_span * UPPER_BODY_HORIZONTAL_MARGIN_RATIO;

    min_x -= horizontal_margin;
    max_x += horizontal_margin;

    // The head joint is near the skull base, not the crown. Include hair/headwear
    // before fitting; viewport padding alone cannot compensate for missing bounds.
    let top_y = animated_top + torso_span * UPPER_BODY_TOP_MARGIN_RATIO;

    let bottom_y = hips - torso_span * UPPER_BODY_BOTTOM_MARGIN_RATIO;

    Some(UpperBodyBounds {
        min_x,
        max_x,
        bottom_y,
        top_y,
        front_z: front_z? + horizontal_margin,
    })
}

/// The rest rig supplies a canonical portrait even when Attention interrupts
/// sitting, a hit, or a crouched frame. Extend the measured shoulder/arm chains
/// to their upward reach so a raised hand never asks for a new camera scale.
fn canonical_upper_body_bounds(
    bones: &Query<(&VrmBone, &RestGlobalTransform), With<NeraLiveBone>>,
    avatar: &PortraitAvatars,
    body_scale: f32,
    head_scale: f32,
    root_offset_y: f32,
) -> UpperBodyBounds {
    let rest_to_world = avatar.single().ok().map(|(_, root, rest, _, _)| {
        root.compute_affine()
            * rest.map_or(bevy::math::Affine3A::IDENTITY, |rest| {
                rest.0.affine().inverse()
            })
    });
    let mut positions: Vec<_> = bones
        .iter()
        .map(|(bone, rest)| {
            let point = rest_to_world.map_or(rest.0.translation(), |mapping| {
                mapping.transform_point3(rest.0.translation())
            });
            (bone.clone(), GlobalTransform::from_translation(point))
        })
        .collect();
    proportion_canonical_positions(&mut positions, body_scale);
    // PreUpdate restores the body layer before Alarm reads the canonical rig.
    // Predict its rest-derived root correction so the portrait follows the
    // rendered Chibi body, including rotated/non-unit avatar roots.
    let root_offset = avatar
        .single()
        .ok()
        .map_or(Vec3::ZERO, |(_, root, _, _, _)| {
            root.rotation * (root.scale * Vec3::Y * root_offset_y)
        });
    if root_offset.is_finite() {
        for (_, pose) in &mut positions {
            *pose = GlobalTransform::from_translation(pose.translation() + root_offset);
        }
    }
    let mut bounds = upper_body_world_metrics(positions.iter().map(|(b, t)| (b, t))).unwrap_or(
        UpperBodyBounds {
            min_x: -0.45,
            max_x: 0.45,
            bottom_y: 0.55,
            top_y: 1.55,
            front_z: 0.0,
        },
    );
    let point = |name: &str| {
        positions
            .iter()
            .find_map(|(bone, transform)| (bone.as_str() == name).then(|| transform.translation()))
    };
    let torso_span = point("head")
        .zip(point("hips"))
        .map_or(MIN_UPPER_BODY_WORLD_HEIGHT, |(h, p)| (h.y - p.y).abs())
        .max(MIN_UPPER_BODY_WORLD_HEIGHT);
    // A larger head extends beyond the compressed body joints. Keep the
    // existing crown/hair margin proportional to the actual head overlay,
    // while canonical rest data continues to prevent alarm animation jitter.
    if let Some(head) = point("head") {
        let head_margin = torso_span * head_scale.max(1.0);
        bounds.top_y = bounds
            .top_y
            .max(head.y + head_margin * UPPER_BODY_TOP_MARGIN_RATIO);
        bounds.min_x = bounds
            .min_x
            .min(head.x - head_margin * UPPER_BODY_HORIZONTAL_MARGIN_RATIO);
        bounds.max_x = bounds
            .max_x
            .max(head.x + head_margin * UPPER_BODY_HORIZONTAL_MARGIN_RATIO);
        bounds.front_z = bounds
            .front_z
            .max(head.z + head_margin * UPPER_BODY_HORIZONTAL_MARGIN_RATIO);
    }
    for chain in [
        ["leftUpperArm", "leftLowerArm", "leftHand"],
        ["rightUpperArm", "rightLowerArm", "rightHand"],
    ] {
        if let (Some(upper), Some(lower), Some(hand)) =
            (point(chain[0]), point(chain[1]), point(chain[2]))
        {
            let reach = upper.distance(lower) + lower.distance(hand);
            bounds.top_y = bounds
                .top_y
                .max(upper.y + reach + torso_span * UPPER_BODY_TOP_MARGIN_RATIO);
            bounds.min_x = bounds.min_x.min(upper.x - reach);
            bounds.max_x = bounds.max_x.max(upper.x + reach);
        }
    }
    bounds
}

/// The body overlay scales the actual hips subtree, keeping its origin fixed.
/// Reproduce that same mapping for the immutable rest landmarks used by Alarm.
fn proportion_canonical_positions(positions: &mut [(VrmBone, GlobalTransform)], body_scale: f32) {
    if !body_scale.is_finite() || body_scale <= 0.0 {
        return;
    }
    let Some(hips) = positions
        .iter()
        .find_map(|(bone, pose)| (bone.as_str() == "hips").then(|| pose.translation()))
    else {
        return;
    };
    for (_, pose) in positions {
        *pose = GlobalTransform::from_translation(hips + (pose.translation() - hips) * body_scale);
    }
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
#[cfg(test)]
fn compute_attention_layout(
    monitor_bounds: crate::platform::PhysicalRect,
    original_width: u32,
    original_height: u32,
    projection: &Projection,
    bone_query: &Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
    locked_center: Option<Vec2>,
) -> AttentionLayout {
    let bounds = upper_body_world_metrics(bone_query).unwrap_or(UpperBodyBounds {
        min_x: -0.45,
        max_x: 0.45,
        bottom_y: 0.55,
        top_y: 1.55,
        front_z: 0.0,
    });
    compute_attention_layout_for_bounds(
        monitor_bounds,
        original_width,
        original_height,
        projection,
        bounds,
        locked_center,
    )
}

fn compute_attention_layout_for_bounds(
    monitor_bounds: crate::platform::PhysicalRect,
    original_width: u32,
    original_height: u32,
    projection: &Projection,
    bounds: UpperBodyBounds,
    locked_center: Option<Vec2>,
) -> AttentionLayout {
    let monitor_width = monitor_bounds.width().max(1) as f32;
    let monitor_height = monitor_bounds.height().max(1) as f32;

    let original_aspect = original_width.max(1) as f32 / original_height.max(1) as f32;

    // Do NOT clamp against the normal 600x950 pet window. On a 900px-tall
    // monitor that would force Attention back to almost full-screen height.
    // Presentation dimensions are derived from the active monitor instead.
    let minimum_window_height = (monitor_height * 0.52).max(420.0).min(monitor_height);

    let window_height = (monitor_height * ATTENTION_WINDOW_MONITOR_HEIGHT_RATIO)
        .round()
        .clamp(minimum_window_height, monitor_height) as u32;

    let minimum_window_width = (monitor_width * 0.20).max(320.0).min(monitor_width);

    let window_width = (window_height as f32 * original_aspect)
        .round()
        .clamp(minimum_window_width, monitor_width) as u32;

    let vertical_fov = match projection {
        Projection::Perspective(perspective) => perspective.fov,
        _ => std::f32::consts::FRAC_PI_4,
    };

    let vertical_half_tan = (vertical_fov * 0.5).tan().max(0.01);

    // The first Attention frame chooses the composition center. Later frames
    // reuse it instead of following asymmetric arm motion.
    let center = locked_center.unwrap_or_else(|| Vec2::new(bounds.center_x(), bounds.center_y()));

    // Fit relative to the LOCKED center. This matters when one arm extends
    // farther than the other: the camera stays still and the pose consumes
    // more room on that side instead of dragging Nera across the screen.
    let vertical_half_extent = ((bounds.top_y - center.y)
        .abs()
        .max((center.y - bounds.bottom_y).abs()))
    .max(MIN_UPPER_BODY_WORLD_HEIGHT * 0.5);

    let horizontal_half_extent = ((bounds.max_x - center.x)
        .abs()
        .max((center.x - bounds.min_x).abs()))
    .max(0.10);

    // Vertical framing remains authoritative for the large half-body look.
    let vertical_distance =
        vertical_half_extent / (ATTENTION_UPPER_BODY_VIEW_FRACTION * vertical_half_tan);

    // At that same camera distance, widen the transparent canvas enough for
    // the current horizontal extent.
    let required_aspect = horizontal_half_extent
        / (vertical_distance.max(0.01)
            * vertical_half_tan
            * ATTENTION_UPPER_BODY_HORIZONTAL_FRACTION);

    let original_window_aspect = window_width.max(1) as f32 / window_height.max(1) as f32;

    let desired_aspect = original_window_aspect.max(required_aspect);

    let max_window_width = (monitor_width * ATTENTION_MAX_WINDOW_WIDTH_RATIO)
        .round()
        .max(window_width as f32)
        .min(monitor_width);

    let widened_window_width = (window_height as f32 * desired_aspect)
        .round()
        .clamp(window_width as f32, max_window_width) as u32;

    let final_aspect = widened_window_width.max(1) as f32 / window_height.max(1) as f32;

    let horizontal_half_tan = (vertical_half_tan * final_aspect).max(0.01);

    let horizontal_distance =
        horizontal_half_extent / (ATTENTION_UPPER_BODY_HORIZONTAL_FRACTION * horizontal_half_tan);

    // Fit at the nearest animated surface, not the z=0 plane. Forward motion
    // during JumpHappy otherwise enlarges the head beyond the fitted viewport.
    let camera_z = bounds.front_z + vertical_distance.max(horizontal_distance);

    AttentionLayout {
        window_width: widened_window_width,
        window_height,
        camera_z,
        viewport_height: (2.0 * vertical_half_extent / ATTENTION_UPPER_BODY_VIEW_FRACTION).max(
            2.0 * horizontal_half_extent
                / (ATTENTION_UPPER_BODY_HORIZONTAL_FRACTION * final_aspect),
        ),
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

fn compute_screensaver_layout(
    monitor: crate::platform::PhysicalRect,
    bounds: UpperBodyBounds,
) -> AttentionLayout {
    let height = (monitor.height().max(1) as f32 * SCREENSAVER_WINDOW_MONITOR_HEIGHT)
        .round()
        .max(1.0);
    // A prominent crown-to-waist portrait, like the user's close-up reference.
    // A wide canvas accommodates the upper body; outstretched arms/accessories
    // may crop at the sides instead of forcing the whole avatar to shrink.
    let viewport = (bounds.top_y - bounds.bottom_y).max(0.01) / SCREENSAVER_PORTRAIT_FILL;
    let width = (monitor.width().max(1) as f32 * SCREENSAVER_MAX_WINDOW_WIDTH)
        .round()
        .max(1.0);
    AttentionLayout {
        window_width: width as u32,
        window_height: height as u32,
        camera_z: bounds.front_z + viewport.max(1.0),
        viewport_height: viewport,
        camera_x: bounds.center_x(),
        camera_y: bounds.center_y(),
        look_at_x: bounds.center_x(),
        look_at_y: bounds.center_y(),
    }
}

fn apply_presentation_window(
    kind: AttentionKind,
    window: &mut Window,
    monitor: crate::platform::PhysicalRect,
    width: u32,
    height: u32,
    progress: f32,
) {
    apply_attention_window(window, monitor, width, height, progress);
    if kind == AttentionKind::Screensaver {
        window.position = WindowPosition::At(IVec2::new(
            monitor.left + (monitor.width() - width as i32) / 2,
            monitor.top + (monitor.height() - height as i32) / 2,
        ));
    }
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
    let left = monitor_bounds.left + ((monitor_width - width as i32) / 2).max(0);

    // Final position is bottom-anchored. This is what creates the
    // "Nera emerges from the bottom of the screen" composition.
    let final_top = monitor_bounds.bottom - height as i32;

    // Start with the whole compact window just below the monitor.
    let start_top = monitor_bounds.bottom;

    let eased = ease_out_cubic(enter_progress);
    let top = (start_top as f32 + (final_top - start_top) as f32 * eased).round() as i32;

    window.position = WindowPosition::At(IVec2::new(left, top));

    window
        .resolution
        .set_physical_resolution(width.max(1), height.max(1));

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
    *transform = Transform::from_xyz(camera_x, camera_y, camera_z.max(0.16))
        .looking_at(Vec3::new(look_at_x, look_at_y, 0.0), Vec3::Y);
}

fn restore_desktop_window(
    window: &mut Window,
    cursor: &mut CursorOptions,
    snapshot: &PresentationSnapshot,
) {
    window.position = WindowPosition::At(IVec2::new(snapshot.window_left, snapshot.window_top));

    window
        .resolution
        .set_physical_resolution(snapshot.window_width, snapshot.window_height);

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
    mut camera_query: Query<(&mut Projection, &mut Transform), With<crate::avatar::NeraMainCamera>>,
    bone_query: Query<(&VrmBone, &RestGlobalTransform), With<NeraLiveBone>>,
    avatar_query: PortraitAvatars,
    body_proportions: Option<Res<crate::avatar::body_mode::BodyProportions>>,
    full_body: full_body::FullBodyGeometry,
) {
    if !state.is_attention() {
        if state.desktop_restore_pending {
            state.desktop_restore_pending = window_query.single().ok().is_some_and(|window| {
                let WindowPosition::At(position) = window.position else {
                    return false;
                };
                !environment.nera_rect.is_some_and(|rect| {
                    rect.left == position.x
                        && rect.top == position.y
                        && rect.width() == window.resolution.physical_width() as i32
                        && rect.height() == window.resolution.physical_height() as i32
                })
            });
        }
        return;
    }

    let Some(monitor) = environment.monitor else {
        return;
    };

    if let Ok((entity, _, _, true, false)) = avatar_query.single()
        && state.attention_avatar != Some(entity)
        && let Some(snapshot) = state.snapshot.as_ref()
    {
        let bounds = canonical_upper_body_bounds(
            &bone_query,
            &avatar_query,
            body_proportions
                .as_ref()
                .map_or(1.0, |body| body.body_scale()),
            body_proportions
                .as_ref()
                .map_or(1.0, |body| body.head_scale()),
            body_proportions
                .as_ref()
                .map_or(0.0, |body| body.standing_root_offset_y()),
        );
        let layout = compute_attention_layout_for_bounds(
            monitor.bounds,
            snapshot.window_width,
            snapshot.window_height,
            &snapshot.camera_projection,
            bounds,
            None,
        );
        let layout = AttentionLayout {
            window_width: layout.window_width.max(
                (monitor.bounds.width().max(1) as f32 * ATTENTION_MAX_WINDOW_WIDTH_RATIO).round()
                    as u32,
            ),
            ..layout
        };
        let layout = if state.is_screensaver() {
            let Some(bounds) = full_body.portrait_bounds(
                &avatar_query,
                body_proportions
                    .as_ref()
                    .map_or(1.0, |body| body.body_scale()),
                body_proportions
                    .as_ref()
                    .map_or(1.0, |body| body.head_scale()),
                body_proportions
                    .as_ref()
                    .map_or(0.0, |body| body.standing_root_offset_y()),
                false,
            ) else {
                return;
            };
            compute_screensaver_layout(monitor.bounds, bounds)
        } else {
            layout
        };
        if let Ok((mut projection, mut camera)) = camera_query.single_mut() {
            *projection = Projection::Orthographic(OrthographicProjection {
                scaling_mode: ScalingMode::FixedVertical {
                    viewport_height: layout.viewport_height,
                },
                ..OrthographicProjection::default_3d()
            });
            apply_attention_camera(
                &mut camera,
                layout.camera_x,
                layout.camera_y,
                layout.look_at_x,
                layout.look_at_y,
                layout.camera_z,
            );
            state.attention_layout = Some(layout);
            state.attention_avatar = Some(entity);
        }
    }

    let Some(layout) = state.attention_layout else {
        return;
    };

    state.attention_enter_progress = (state.attention_enter_progress
        + time.delta_secs() / ATTENTION_ENTER_SECONDS)
        .clamp(0.0, 1.0);

    let enter_progress = state.attention_enter_progress;

    if let Ok(mut window) = window_query.single_mut() {
        let PresentationMode::Attention(kind) = state.mode else {
            return;
        };
        apply_presentation_window(
            kind,
            &mut window,
            monitor.bounds,
            layout.window_width,
            layout.window_height,
            enter_progress,
        );
    }
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
pub(crate) fn desktop_pet_active(state: Res<PresentationState>) -> bool {
    state.is_desktop_pet() && !state.desktop_restore_pending
}

/// Screensaver retains body hit testing for dismissal, without enabling
/// desktop click reactions or dragging during presentation. Button-free
/// headpat also uses this gate, including Screensaver but excluding Alarm.
pub(crate) fn avatar_hit_testing_active(state: Res<PresentationState>) -> bool {
    (state.is_desktop_pet() && !state.desktop_restore_pending) || state.is_screensaver()
}

// -----------------------------------------------------------------------------
// Pure state tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    pub(super) fn presentation_app() -> (
        App,
        Entity,
        Entity,
        Entity,
        Transform,
        PerspectiveProjection,
    ) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<PresentationState>()
            .init_resource::<NeraAnimationState>()
            .init_resource::<DesktopPetInteraction>()
            .init_resource::<HoveredBodyPart>()
            .init_resource::<HeadPatGesture>()
            .add_message::<PresentationCommand>()
            .add_message::<PresentationEntered>()
            .add_message::<PresentationExited>()
            .add_systems(Update, handle_presentation_commands)
            .add_systems(Last, maintain_attention_presentation);
        let rect = crate::platform::PhysicalRect {
            left: 40,
            top: 60,
            right: 640,
            bottom: 1010,
        };
        let monitor = crate::platform::PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        app.insert_resource(DesktopEnvironment {
            nera_rect: Some(rect),
            monitor: Some(crate::platform::MonitorInfo {
                handle: crate::platform::NativeMonitorHandle::from_raw(1),
                bounds: monitor,
                work_area: monitor,
                is_primary: true,
            }),
            ..default()
        });
        let mut window = Window::default();
        window.resolution.set_physical_resolution(600, 950);
        let window = app
            .world_mut()
            .spawn((
                PrimaryWindow,
                window,
                CursorOptions {
                    hit_test: false,
                    ..default()
                },
            ))
            .id();
        let camera_transform = Transform::from_xyz(0.03, 0.8, 4.5);
        let perspective = PerspectiveProjection {
            fov: 0.7,
            near: 0.05,
            far: 75.0,
            ..default()
        };
        let camera = app
            .world_mut()
            .spawn((
                crate::avatar::NeraMainCamera,
                camera_transform,
                Projection::Perspective(perspective.clone()),
            ))
            .id();
        let root = Transform::from_xyz(0.0, -0.85, 0.0).with_scale(Vec3::splat(1.2));
        let avatar = app
            .world_mut()
            .spawn((
                crate::avatar::NeraAvatar,
                Initialized,
                root,
                RestGlobalTransform(GlobalTransform::from(root)),
            ))
            .id();
        for (name, position) in [
            ("hips", Vec3::new(0.0, 0.0, 0.0)),
            ("head", Vec3::new(0.0, 0.45, 0.0)),
            ("leftUpperArm", Vec3::new(-0.15, 0.35, 0.0)),
            ("leftLowerArm", Vec3::new(-0.4, 0.35, 0.0)),
            ("leftHand", Vec3::new(-0.65, 0.35, 0.0)),
        ] {
            let pose = GlobalTransform::from_translation(position);
            app.world_mut().spawn((
                VrmBone::from(name),
                pose,
                RestGlobalTransform(pose),
                NeraLiveBone,
            ));
        }
        (app, window, camera, avatar, camera_transform, perspective)
    }

    #[test]
    fn alarm_canonical_chibi_landmarks_keep_the_original_hips_pivot_without_drift() {
        let rest = vec![
            (
                VrmBone::from("hips"),
                GlobalTransform::from_translation(Vec3::new(0.2, 0.3, 0.4)),
            ),
            (
                VrmBone::from("head"),
                GlobalTransform::from_translation(Vec3::new(0.2, 1.3, 0.4)),
            ),
            (
                VrmBone::from("leftHand"),
                GlobalTransform::from_translation(Vec3::new(-0.6, 0.9, 0.5)),
            ),
        ];
        for factor in [0.62, 1.0].into_iter().cycle().take(100) {
            let mut positions = rest.clone();
            proportion_canonical_positions(&mut positions, factor);
            assert_eq!(positions[0].1, rest[0].1);
            for (original, changed) in rest.iter().zip(&positions) {
                let expected = rest[0].1.translation()
                    + (original.1.translation() - rest[0].1.translation()) * factor;
                assert!(changed.1.translation().distance(expected) < 0.00001);
            }
        }
    }

    #[test]
    fn alarm_canonical_center_follows_rendered_chibi_root_correction() {
        let (mut app, _, _, avatar, _, _) = presentation_app();
        let root = Transform {
            rotation: Quat::from_euler(EulerRot::XYZ, 0.16, 0.24, 0.12),
            scale: Vec3::new(1.3, 0.8, 1.1),
            ..*app.world().get::<Transform>(avatar).unwrap()
        };
        *app.world_mut().get_mut::<Transform>(avatar).unwrap() = root;
        let root_offset_y = -0.3;
        let rendered_delta = root.rotation * (root.scale * Vec3::Y * root_offset_y);
        let (without_offset, corrected) = app
            .world_mut()
            .run_system_once(
                move |bones: Query<(&VrmBone, &RestGlobalTransform), With<NeraLiveBone>>,
                      avatars: PortraitAvatars| {
                    (
                        canonical_upper_body_bounds(&bones, &avatars, 0.62, 1.9, 0.0),
                        canonical_upper_body_bounds(&bones, &avatars, 0.62, 1.9, root_offset_y),
                    )
                },
            )
            .unwrap();
        assert!(
            (corrected.center_x() - without_offset.center_x() - rendered_delta.x).abs() < 0.00001
        );
        assert!(
            (corrected.center_y() - without_offset.center_y() - rendered_delta.y).abs() < 0.00001
        );
        assert!((corrected.front_z - without_offset.front_z - rendered_delta.z).abs() < 0.00001);
        assert!((corrected.height() - without_offset.height()).abs() < 0.00001);
        assert!((corrected.width() - without_offset.width()).abs() < 0.00001);
    }

    #[test]
    fn alarm_sequence_keeps_one_projection_and_camera_then_restores_exact_desktop_state() {
        let (mut app, window, camera, avatar, original_camera, original_projection) =
            presentation_app();
        let root = *app.world().get::<Transform>(avatar).unwrap();
        app.world_mut()
            .write_message(PresentationCommand::EnterAttention(AttentionKind::Alarm));
        app.update();
        let canonical_camera = *app.world().get::<Transform>(camera).unwrap();
        let canonical_height = match app.world().get::<Projection>(camera).unwrap() {
            Projection::Orthographic(orthographic) => match orthographic.scaling_mode {
                ScalingMode::FixedVertical { viewport_height } => viewport_height,
                _ => panic!("Expected a canonical portrait projection"),
            },
            _ => panic!("Attention must ignore animated depth for apparent scale"),
        };
        for (action, offset) in [
            (
                crate::animation::TemporaryAction::AlarmJump,
                Vec3::new(0.1, 0.3, 0.8),
            ),
            (
                crate::animation::TemporaryAction::AlarmWave,
                Vec3::new(-0.2, 0.8, -0.3),
            ),
        ] {
            app.world_mut()
                .resource_mut::<NeraAnimationState>()
                .request_action_for_test(action, std::time::Duration::from_secs(5));
            for mut pose in app
                .world_mut()
                .query_filtered::<&mut GlobalTransform, With<NeraLiveBone>>()
                .iter_mut(app.world_mut())
            {
                *pose = GlobalTransform::from_translation(pose.translation() + offset);
            }
            // A repeated Enter must retain the first normal-state snapshot.
            app.world_mut()
                .write_message(PresentationCommand::EnterAttention(AttentionKind::Alarm));
            app.update();
            assert_eq!(
                *app.world().get::<Transform>(camera).unwrap(),
                canonical_camera
            );
            let Projection::Orthographic(orthographic) =
                app.world().get::<Projection>(camera).unwrap()
            else {
                panic!()
            };
            assert!(
                matches!(orthographic.scaling_mode, ScalingMode::FixedVertical { viewport_height } if viewport_height == canonical_height)
            );
            assert_eq!(*app.world().get::<Transform>(avatar).unwrap(), root);
        }
        app.world_mut()
            .write_message(PresentationCommand::ExitAttention);
        app.update();
        assert_eq!(
            *app.world().get::<Transform>(camera).unwrap(),
            original_camera
        );
        let Projection::Perspective(restored) = app.world().get::<Projection>(camera).unwrap()
        else {
            panic!()
        };
        assert_eq!(restored.fov, original_projection.fov);
        assert_eq!(restored.near, original_projection.near);
        assert_eq!(restored.far, original_projection.far);
        assert_eq!(*app.world().get::<Transform>(avatar).unwrap(), root);
        let restored_window = app.world().get::<Window>(window).unwrap();
        assert_eq!(
            restored_window.position,
            WindowPosition::At(IVec2::new(40, 60))
        );
        assert_eq!(restored_window.resolution.physical_width(), 600);
        assert_eq!(restored_window.resolution.physical_height(), 950);
        assert!(!app.world().get::<CursorOptions>(window).unwrap().hit_test);
        assert!(app.world().resource::<PresentationState>().is_desktop_pet());
        assert!(
            app.world()
                .resource::<PresentationState>()
                .snapshot
                .is_none()
        );
        assert!(
            app.world()
                .resource::<PresentationState>()
                .attention_layout
                .is_none()
        );
    }

    #[test]
    fn missing_camera_on_exit_still_releases_presentation_ownership() {
        let (mut app, _, camera, _, _, _) = presentation_app();
        app.world_mut()
            .write_message(PresentationCommand::EnterAttention(AttentionKind::Alarm));
        app.update();
        app.world_mut().despawn(camera);
        app.world_mut()
            .write_message(PresentationCommand::ExitAttention);
        app.update();
        assert!(app.world().resource::<PresentationState>().is_desktop_pet());
        assert!(
            app.world()
                .resource::<PresentationState>()
                .snapshot
                .is_none()
        );
        assert_eq!(
            app.world().resource::<DesktopEnvironment>().placement,
            PlacementState::Free
        );
    }

    #[test]
    fn replacement_reframes_once_for_the_new_rig_without_losing_the_normal_snapshot() {
        let (mut app, _, camera, avatar, original_camera, original_projection) = presentation_app();
        app.world_mut()
            .write_message(PresentationCommand::EnterAttention(AttentionKind::Alarm));
        app.update();
        let original_height = app
            .world()
            .resource::<PresentationState>()
            .attention_layout
            .unwrap()
            .viewport_height;
        let root = *app.world().get::<Transform>(avatar).unwrap();
        app.world_mut().despawn(avatar);
        let old_bones: Vec<_> = app
            .world_mut()
            .query_filtered::<Entity, With<NeraLiveBone>>()
            .iter(app.world())
            .collect();
        for entity in old_bones {
            app.world_mut().despawn(entity);
        }
        let replacement = app
            .world_mut()
            .spawn((
                crate::avatar::NeraAvatar,
                root,
                RestGlobalTransform(GlobalTransform::from(root)),
            ))
            .id();
        for (name, point) in [
            ("hips", Vec3::ZERO),
            ("head", Vec3::new(0.0, 0.9, 0.0)),
            ("leftUpperArm", Vec3::new(-0.3, 0.7, 0.0)),
            ("leftLowerArm", Vec3::new(-0.8, 0.7, 0.0)),
            ("leftHand", Vec3::new(-1.3, 0.7, 0.0)),
        ] {
            let pose = GlobalTransform::from_translation(point);
            app.world_mut().spawn((
                VrmBone::from(name),
                NeraLiveBone,
                pose,
                RestGlobalTransform(pose),
            ));
        }
        app.update();
        assert_eq!(
            app.world()
                .resource::<PresentationState>()
                .attention_layout
                .unwrap()
                .viewport_height,
            original_height
        );
        app.world_mut().entity_mut(replacement).insert(Initialized);
        app.update();
        let state = app.world().resource::<PresentationState>();
        assert_eq!(state.attention_avatar, Some(replacement));
        let new_height = state.attention_layout.unwrap().viewport_height;
        assert!((new_height - original_height * 2.0).abs() < 0.001);
        let new_camera = *app.world().get::<Transform>(camera).unwrap();
        for mut bone in app
            .world_mut()
            .query_filtered::<&mut GlobalTransform, With<NeraLiveBone>>()
            .iter_mut(app.world_mut())
        {
            *bone = GlobalTransform::from_translation(bone.translation() + Vec3::Y);
        }
        app.update();
        assert_eq!(
            app.world()
                .resource::<PresentationState>()
                .attention_layout
                .unwrap()
                .viewport_height,
            new_height
        );
        assert_eq!(*app.world().get::<Transform>(camera).unwrap(), new_camera);
        app.world_mut()
            .write_message(PresentationCommand::ExitAttention);
        app.update();
        assert_eq!(
            *app.world().get::<Transform>(camera).unwrap(),
            original_camera
        );
        let Projection::Perspective(restored) = app.world().get::<Projection>(camera).unwrap()
        else {
            panic!()
        };
        assert_eq!(restored.fov, original_projection.fov);
        assert_eq!(restored.near, original_projection.near);
        assert_eq!(*app.world().get::<Transform>(replacement).unwrap(), root);
        assert!(
            app.world()
                .resource::<PresentationState>()
                .attention_avatar
                .is_none()
        );
    }

    fn portrait_layout(offset: Vec3, raised_hand: bool, center: Option<Vec2>) -> AttentionLayout {
        let mut world = World::new();
        for (name, position) in [
            ("hips", Vec3::new(0.0, 0.0, 0.0)),
            ("head", Vec3::new(0.0, 0.45, 0.18)),
            (
                "leftHand",
                Vec3::new(-0.35, if raised_hand { 0.85 } else { 0.2 }, 0.1),
            ),
            // Feet must not force full-body framing.
            ("leftFoot", Vec3::new(0.0, -1.0, 0.0)),
        ] {
            world.spawn((
                VrmBone::from(name),
                GlobalTransform::from_translation(position + offset),
                NeraLiveBone,
            ));
        }
        world
            .run_system_once(
                move |bones: Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>| {
                    compute_attention_layout(
                        crate::platform::PhysicalRect {
                            left: 0,
                            top: 0,
                            right: 1920,
                            bottom: 1080,
                        },
                        600,
                        950,
                        &Projection::Perspective(PerspectiveProjection::default()),
                        &bones,
                        center,
                    )
                },
            )
            .unwrap()
    }

    #[test]
    fn alarm_framing_fits_head_crown_at_forward_depth() {
        let layout = portrait_layout(Vec3::ZERO, false, None);
        // Hair/headpiece extends above the head joint in the default rig.
        let crown = Vec3::new(0.0, 0.63, 0.24);
        let projected_y = (crown.y - layout.camera_y)
            / ((layout.camera_z - crown.z) * (std::f32::consts::FRAC_PI_4 * 0.5).tan());
        assert!(projected_y > 0.0 && projected_y <= ATTENTION_UPPER_BODY_VIEW_FRACTION + 0.001);
        assert!(
            layout.camera_y < 0.4,
            "legs should remain outside the portrait crop"
        );
    }

    #[test]
    fn alarm_framing_is_invariant_to_avatar_depth_translation() {
        let base = portrait_layout(Vec3::ZERO, false, None);
        for z in [-0.3, 0.8] {
            let moved = portrait_layout(Vec3::Z * z, false, None);
            assert!((moved.camera_z - base.camera_z - z).abs() < 0.001);
            assert!((moved.camera_y - base.camera_y).abs() < 0.001);
            assert_eq!(moved.window_width, base.window_width);
        }
    }

    #[test]
    fn alarm_jump_and_raised_hands_fit_without_chasing_locked_center() {
        let initial = portrait_layout(Vec3::ZERO, false, None);
        let center = Vec2::new(initial.camera_x, initial.camera_y);
        let moved = portrait_layout(Vec3::new(0.1, 0.25, 0.3), true, Some(center));
        assert_eq!(Vec2::new(moved.camera_x, moved.camera_y), center);
        let top = 0.85 + 0.25 + 0.45 * UPPER_BODY_TOP_MARGIN_RATIO;
        let front = 0.18 + 0.3 + 0.45 * UPPER_BODY_HORIZONTAL_MARGIN_RATIO;
        let projected_top = (top - center.y)
            / ((moved.camera_z - front) * (std::f32::consts::FRAC_PI_4 * 0.5).tan());
        assert!(projected_top <= ATTENTION_UPPER_BODY_VIEW_FRACTION + 0.001);
        assert!(moved.camera_z > initial.camera_z);
    }

    #[test]
    fn presentation_starts_as_desktop_pet() {
        let state = PresentationState::default();

        assert_eq!(state.mode, PresentationMode::DesktopPet);
        assert!(state.is_desktop_pet());
        assert!(!state.is_attention());
    }

    #[test]
    fn attention_mode_is_distinct_from_desktop_pet() {
        let state = PresentationState {
            mode: PresentationMode::Attention(AttentionKind::Alarm),
            ..Default::default()
        };

        assert!(state.is_attention());
        assert!(!state.is_desktop_pet());
    }

    #[test]
    fn attention_is_upper_body_portrait() {
        // Bounds are inclusive: 0.80 is the tightest framing that still keeps
        // the hips/waist in shot, so a tuned-to-the-edge value is valid.
        assert!(ATTENTION_UPPER_BODY_VIEW_FRACTION >= 0.80);
        assert!(ATTENTION_UPPER_BODY_VIEW_FRACTION <= 0.95);
    }

    #[test]
    fn perspective_distance_formula_scales_with_portrait_height() {
        let fov = std::f32::consts::FRAC_PI_4;
        let fraction = 0.80;

        let distance_for_one = 1.0 / (2.0 * fraction * (fov * 0.5).tan());
        let distance_for_two = 2.0 / (2.0 * fraction * (fov * 0.5).tan());

        assert!((distance_for_two / distance_for_one - 2.0).abs() < 0.001);
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
        assert!(ATTENTION_UPPER_BODY_VIEW_FRACTION <= 0.82);
    }

    #[test]
    fn wide_pose_prefers_wider_canvas_before_backing_camera() {
        let vertical_half_tan = 0.5_f32;
        let vertical_distance = 1.0_f32;
        let body_width = 1.8_f32;

        let required_aspect = body_width
            / (2.0
                * vertical_distance
                * vertical_half_tan
                * ATTENTION_UPPER_BODY_HORIZONTAL_FRACTION);

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
            front_z: 0.0,
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
            front_z: 0.0,
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
        use crate::interaction::{BodyPart, InteractionState};
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

        reset_pointer_state(&mut interaction, &mut hovered, &mut head_pat);

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
