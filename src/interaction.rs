//! Semantic interaction vocabulary and its single native pointer pipeline.
//! System scheduling stays in main to preserve ordering with animation,
//! desktop physics, presentation ownership, and pose-derived avatar anchors.
mod headpat;
mod hit_testing;

#[cfg(target_os = "windows")]
use crate::alarm_settings;
#[cfg(target_os = "windows")]
use crate::animation_manager;
use crate::{
    environment::{DesktopEnvironment, PlacementState},
    platform,
    settings::{self, NeraSettings},
};
use bevy::{
    prelude::*,
    window::{CursorOptions, PrimaryWindow},
};
use bevy_vrm1::prelude::*;
pub(crate) use hit_testing::NeraLiveBone;
pub(crate) use hit_testing::mark_nera_live_bones;
use hit_testing::{NeraCollider, head_part_at_cursor, nearest_nera_body_part};
use std::time::{Duration, Instant};

pub(crate) use headpat::{HeadPatGesture, detect_head_pat};
pub(crate) use hit_testing::attach_bone_colliders;

/// Read-only probes of the production hit route for native validation.
#[cfg(debug_assertions)]
pub(crate) fn smoke_parts_at_cursor(
    world: &mut World,
    cursors: Vec<Vec2>,
) -> Vec<Option<BodyPart>> {
    use bevy::ecs::system::RunSystemOnce;
    world
        .run_system_once(
            move |settings: Res<NeraSettings>,
                  proportions: Option<Res<crate::avatar::body_mode::BodyProportions>>,
                  camera: Query<
                (&Camera, &GlobalTransform),
                With<crate::avatar::NeraMainCamera>,
            >,
                  colliders: Query<(&GlobalTransform, &NeraCollider)>,
                  bones: Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>| {
                let Ok((camera, view)) = camera.single() else {
                    return vec![None; cursors.len()];
                };
                let body_scale = proportions.as_ref().map_or(1.0, |body| body.body_scale());
                let head_scale = proportions.as_ref().map_or(1.0, |body| body.head_scale());
                let scale = settings.character_scale() * body_scale;
                cursors
                    .iter()
                    .copied()
                    .map(|cursor| {
                        if head_part_at_cursor(cursor, camera, view, &bones, scale * head_scale) {
                            Some(BodyPart::Head)
                        } else {
                            camera.viewport_to_world(view, cursor).ok().and_then(|ray| {
                                nearest_nera_body_part(
                                    ray.origin,
                                    ray.direction.into(),
                                    &colliders,
                                    &bones,
                                    scale,
                                )
                            })
                        }
                    })
                    .collect()
            },
        )
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InteractionState {
    #[default]
    None,
    Hover,
    Pressed,
    Dragging,
}

/// Semantic body regions produced by Phase 5.2 hit detection.
///
/// Left/right are kept separate now so future reactions can distinguish, for
/// example, a left-hand click from a right-hand click without changing the
/// interaction message API again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BodyPart {
    Head,
    Torso,
    LeftArm,
    RightArm,
    LeftHand,
    RightHand,
    LeftLeg,
    RightLeg,
    LeftFoot,
    RightFoot,
}

/// Current semantic body part under the cursor.
///
/// This is deliberately separate from `DesktopPetInteraction`: hover is
/// useful to future behavior/UI systems even when no click is in progress.
/// Click/drag messages still capture the body part at mouse-down so animation
/// cannot change the meaning of an in-progress gesture.
#[derive(Resource, Default, Debug)]
pub(crate) struct HoveredBodyPart {
    pub(crate) part: Option<BodyPart>,
}

/// Pointer coordinates captured at the moment a semantic interaction fires.
///
/// `screen` uses physical desktop pixels (useful for native menus/window work),
/// while `local` uses Nera-window logical coordinates (useful for Phase 5.2
/// raycasts/body-part detection).
#[derive(Clone, Copy, Debug)]
pub(crate) struct NeraPointerPosition {
    pub(crate) screen: IVec2,
    pub(crate) local: Vec2,
}

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraClicked {
    pub(crate) pointer: NeraPointerPosition,
    pub(crate) body_part: BodyPart,
}

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraRightClicked {
    pub(crate) pointer: NeraPointerPosition,
    pub(crate) body_part: BodyPart,
}

/// Extra lifecycle message beyond the minimum roadmap. Consumers that care
/// about drag setup do not need to infer the first `NeraDragged` frame.
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraDragStarted {
    pub(crate) screen: IVec2,
    pub(crate) body_part: BodyPart,
}

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraDragged {
    pub(crate) screen: IVec2,
    pub(crate) delta: IVec2,
}

/// Semantic head-stroke/pat gesture emitted after deliberate back-and-forth
/// cursor movement over the Head collider. Horizontal, vertical, and natural
/// diagonal strokes are accepted. It is mouse-button-free so it cannot race
/// the already-locked click-vs-drag classifier.
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraHeadPatted {
    pub(crate) pointer: NeraPointerPosition,
}

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraReleased {
    pub(crate) pointer: NeraPointerPosition,
    pub(crate) body_part: BodyPart,
    pub(crate) was_dragging: bool,
}

mod momentum;
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraThrow {
    pub(crate) velocity: Vec2,
}

pub(crate) struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<NeraClicked>()
            .add_message::<NeraRightClicked>()
            .add_message::<NeraDragStarted>()
            .add_message::<NeraDragged>()
            .add_message::<NeraHeadPatted>()
            .add_message::<NeraReleased>()
            .add_message::<NeraThrow>()
            // Debug consumer only. Keeping it in PostUpdate guarantees the
            // Update-stage producer has already emitted this frame's messages.
            .add_systems(PostUpdate, log_interaction_messages);
    }
}

fn log_interaction_messages(
    mut clicked: MessageReader<NeraClicked>,
    mut right_clicked: MessageReader<NeraRightClicked>,
    mut drag_started: MessageReader<NeraDragStarted>,
    mut dragged: MessageReader<NeraDragged>,
    mut head_patted: MessageReader<NeraHeadPatted>,
    mut released: MessageReader<NeraReleased>,
) {
    for event in clicked.read() {
        info!(
            "Nera interaction -> Click {:?} screen={:?} local={:?}",
            event.body_part, event.pointer.screen, event.pointer.local
        );
    }

    for event in right_clicked.read() {
        info!(
            "Nera interaction -> RightClick {:?} screen={:?} local={:?}",
            event.body_part, event.pointer.screen, event.pointer.local
        );
    }

    for event in drag_started.read() {
        info!(
            "Nera interaction -> DragStarted {:?} @ {:?}",
            event.body_part, event.screen
        );
    }

    for event in dragged.read() {
        trace!(
            "Nera interaction -> Dragged @ {:?}, delta={:?}",
            event.screen, event.delta
        );
    }

    for event in head_patted.read() {
        info!(
            "Nera interaction -> HeadStroke screen={:?} local={:?}",
            event.pointer.screen, event.pointer.local
        );
    }

    for event in released.read() {
        info!(
            "Nera interaction -> Released {:?} screen={:?} local={:?} (was_dragging={})",
            event.body_part, event.pointer.screen, event.pointer.local, event.was_dragging
        );
    }
}

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
#[derive(Resource, Default)]
pub(crate) struct DesktopPetInteraction {
    pub(crate) momentum: momentum::DragMomentum,
    pub(crate) state: InteractionState,

    /// Phase 5.2 captures semantic body parts at mouse-down so animation or
    /// cursor movement cannot change a gesture's meaning before release.
    pub(crate) pressed_body_part: Option<BodyPart>,
    pub(crate) right_pressed_body_part: Option<BodyPart>,

    /// Grab point inside Nera's native window. Some means LMB currently owns
    /// the interaction, even while an attachment temporarily keeps the HWND pinned.
    pub(crate) drag_offset: Option<IVec2>,
    pub(crate) drag_start_cursor: Option<IVec2>,
    pub(crate) last_drag_cursor: Option<IVec2>,
    pub(crate) drag_started_at: Option<Instant>,

    /// Capture RMB until release so click-through cannot hand the release to
    /// the window underneath Nera. A small travel guard prevents right-drag
    /// gestures from opening the future Phase 5.7 action menu.
    pub(crate) right_press_origin: Option<IVec2>,

    /// Cursor position captured when a sitting attachment is established. A
    /// meaningful movement away from it unsnaps Nera without doing Seat->Feet
    /// HWND rebasing first.
    pub(crate) snap_cursor: Option<IVec2>,

    /// Short post-sit input lock. Until this expires, an attached Nera cannot
    /// be pulled off the surface even if LMB remains held.
    pub(crate) sitting_lock_until: Option<Instant>,

    /// Mate-style anti-resnap safety.
    pub(crate) unsnap_cooldown_until: Option<Instant>,
    pub(crate) guard_center: Option<IVec2>,
    pub(crate) recent_unsnap_surface_y: Option<i32>,
}

impl DesktopPetInteraction {
    /// Drop every in-flight pointer gesture.
    ///
    /// `ButtonInput<MouseButton>` is application-global while `drag_nera_window`
    /// is gated on `presentation::desktop_pet_active`. Whenever another surface
    /// takes input ownership — the Settings window, or an Attention
    /// presentation — the LMB release edge can pass unseen and leave `state`
    /// frozen at `Pressed`/`Dragging`. `update_click_through` deliberately keeps
    /// `hit_test = true` while captured, so a frozen state would swallow every
    /// desktop click until the user happened to complete another full
    /// press+release over Nera.
    pub(crate) fn reset_gesture_state(&mut self) {
        self.momentum.clear();
        self.state = InteractionState::None;
        self.pressed_body_part = None;
        self.right_pressed_body_part = None;
        self.drag_offset = None;
        self.drag_start_cursor = None;
        self.last_drag_cursor = None;
        self.drag_started_at = None;
        self.right_press_origin = None;
        self.snap_cursor = None;
    }

    /// Stronger reset for Attention enter/exit.
    ///
    /// Presentation resizes and relocates the pet HWND into a large portrait
    /// crop and then restores the original rect afterwards. The anti-resnap
    /// guards are recorded in *desktop* pixels against the pre-Attention rect,
    /// so keeping them would let a stale `guard_center` /
    /// `recent_unsnap_surface_y` veto or force a snap the moment DesktopPet
    /// resumes. They have no meaning across the round-trip.
    pub(crate) fn reset_for_attention(&mut self) {
        self.reset_gesture_state();
        self.sitting_lock_until = None;
        self.unsnap_cooldown_until = None;
        self.guard_center = None;
        self.recent_unsnap_surface_y = None;
    }
}
#[cfg(target_os = "windows")]
pub(crate) fn nera_utility_input_available(
    settings_gate: Res<settings::SettingsInputGate>,
    alarm_gate: Res<alarm_settings::AlarmSettingsInputGate>,
    manager_gate: Res<animation_manager::AnimationManagerInputGate>,
) -> bool {
    settings::nera_input_available(settings_gate)
        && alarm_settings::nera_input_available(alarm_gate)
        && !manager_gate.pointer_over_window
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn nera_utility_input_available(
    settings_gate: Res<settings::SettingsInputGate>,
) -> bool {
    settings::nera_input_available(settings_gate)
}

#[cfg(target_os = "windows")]
pub(crate) fn nera_utility_input_blocked(
    settings_gate: Res<settings::SettingsInputGate>,
    alarm_gate: Res<alarm_settings::AlarmSettingsInputGate>,
    manager_gate: Res<animation_manager::AnimationManagerInputGate>,
) -> bool {
    settings::nera_input_blocked(settings_gate)
        || alarm_settings::nera_input_blocked(alarm_gate)
        || manager_gate.pointer_over_window
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn nera_utility_input_blocked(settings_gate: Res<settings::SettingsInputGate>) -> bool {
    settings::nera_input_blocked(settings_gate)
}

#[cfg(target_os = "windows")]
pub(crate) fn cursor_position_in_nera_window(environment: &DesktopEnvironment) -> Option<Vec2> {
    let global_cursor = platform::global_cursor_position()?;

    let rect = environment.nera_rect?;

    let local_x = global_cursor.x - rect.left;

    let local_y = global_cursor.y - rect.top;

    let scale = environment.nera_scale_factor as f32;

    if scale <= 0.0 {
        return None;
    }

    Some(Vec2::new(local_x as f32 / scale, local_y as f32 / scale))
}

#[cfg(target_os = "windows")]
pub(super) fn nera_pointer_position(
    screen: IVec2,
    environment: &DesktopEnvironment,
) -> NeraPointerPosition {
    NeraPointerPosition {
        screen,
        local: cursor_position_in_nera_window(environment).unwrap_or_default(),
    }
}
#[cfg(target_os = "windows")]
pub(crate) fn settle_pending_sit(
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

    interaction.snap_cursor =
        platform::global_cursor_position().map(|point| IVec2::new(point.x, point.y));

    interaction.sitting_lock_until = Some(Instant::now() + SIT_MOVEMENT_LOCK);

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

#[cfg(target_os = "windows")]
pub(crate) fn suspend_nera_interaction_for_settings(
    mut interaction: ResMut<DesktopPetInteraction>,
    mut hovered: ResMut<HoveredBodyPart>,
    mut head_pat: ResMut<HeadPatGesture>,
    mut window_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    // Settings is a real second Bevy window, but ButtonInput<MouseButton> is
    // application-global. Clear every *active gesture* so a Settings click
    // cannot continue a PrimaryWindow press/drag that happened previously.
    interaction.reset_gesture_state();

    hovered.part = None;
    head_pat.reset_stroke();

    // Critical when Nera itself is Always-On-Top and visually overlaps the
    // Settings panel: let native hit-testing pass through the pet HWND.
    if let Ok(mut cursor_options) = window_query.single_mut() {
        cursor_options.hit_test = false;
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn update_click_through(
    settings: Res<NeraSettings>,
    proportions: Option<Res<crate::avatar::body_mode::BodyProportions>>,
    cloud: Option<Res<crate::cloud::CloudRuntime>>,
    environment: Res<DesktopEnvironment>,
    mut interaction: ResMut<DesktopPetInteraction>,
    mut hovered: ResMut<HoveredBodyPart>,
    mut window_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<crate::avatar::NeraMainCamera>>,
    colliders_query: Query<(&GlobalTransform, &NeraCollider)>,
    bone_query: Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>,
    cloud_visuals: Query<&GlobalTransform, With<crate::cloud::CloudVisual>>,
) {
    let Ok(mut cursor_options) = window_query.single_mut() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.single() else {
        return;
    };
    let character_scale = settings.character_scale();
    let body_scale = proportions.as_ref().map_or(1.0, |body| body.body_scale());
    let head_scale = proportions.as_ref().map_or(1.0, |body| body.head_scale());

    let hovered_body_part = cursor_position_in_nera_window(&environment).and_then(|cursor_pos| {
        // Head gets a visual screen-space test first. Other body regions
        // continue to use deterministic nearest 3D collider distance.
        if head_part_at_cursor(
            cursor_pos,
            camera,
            camera_transform,
            &bone_query,
            character_scale * body_scale * head_scale,
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
                    character_scale * body_scale,
                )
            })
    });

    hovered.part = hovered_body_part;
    let hovering = hovered_body_part.is_some()
        || cloud.as_ref().is_some_and(|cloud| {
            cursor_position_in_nera_window(&environment).is_some_and(|cursor| {
                crate::cloud::cloud_hit(
                    cloud,
                    cursor,
                    &environment,
                    camera,
                    camera_transform,
                    &cloud_visuals,
                )
            })
        });
    if hovering && hovered.part.is_none() {
        hovered.part = Some(BodyPart::Torso);
    }

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

#[cfg(target_os = "windows")]
pub(crate) fn drag_nera_window(
    mouse: Res<ButtonInput<MouseButton>>,
    settings: Res<crate::settings::NeraSettings>,
    mut interaction: ResMut<DesktopPetInteraction>,
    mut environment: ResMut<DesktopEnvironment>,
    hovered: Res<HoveredBodyPart>,
    mut clicked: MessageWriter<NeraClicked>,
    mut right_clicked: MessageWriter<NeraRightClicked>,
    mut drag_started: MessageWriter<NeraDragStarted>,
    mut dragged: MessageWriter<NeraDragged>,
    mut released: MessageWriter<NeraReleased>,
    mut thrown: MessageWriter<NeraThrow>,
    mut cloud: ResMut<crate::cloud::CloudRuntime>,
    mut physics: ResMut<crate::desktop_physics::DesktopPhysicsState>,
    mut animation: ResMut<crate::animation::NeraAnimationState>,
    mut sfx: MessageWriter<crate::pet_feedback::PetSfx>,
    mut fx: MessageWriter<crate::pet_feedback::PetFx>,
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
    if interaction.state == InteractionState::Hover && mouse.just_pressed(MouseButton::Right) {
        if let Some(body_part) = hovered.part {
            interaction.right_press_origin = Some(cursor);
            interaction.right_pressed_body_part = Some(body_part);
        }
    }

    if mouse.just_released(MouseButton::Right) {
        let origin = interaction.right_press_origin.take();
        let body_part = interaction.right_pressed_body_part.take();

        if let (Some(origin), Some(body_part)) = (origin, body_part) {
            let right_click_threshold = (DRAG_START_DISTANCE_AT_96_DPI
                * environment.nera_scale_factor as f32)
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
    if interaction.state == InteractionState::Hover && mouse.just_pressed(MouseButton::Left) {
        if let (Some(rect), Some(body_part)) = (environment.nera_rect, hovered.part) {
            interaction.state = InteractionState::Pressed;
            interaction.pressed_body_part = Some(body_part);
            interaction.drag_offset = Some(IVec2::new(
                global_cursor.x - rect.left,
                global_cursor.y - rect.top,
            ));
            interaction.drag_start_cursor = Some(cursor);
            interaction.last_drag_cursor = Some(cursor);
            interaction.drag_started_at = Some(now);
            interaction.momentum.clear();
            interaction.momentum.sample(now, cursor);

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
    if mouse.pressed(MouseButton::Left) && interaction.state == InteractionState::Pressed {
        let drag_threshold = (DRAG_START_DISTANCE_AT_96_DPI * environment.nera_scale_factor as f32)
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
            crate::cloud::begin_cloud_drag(
                &mut cloud,
                &mut environment,
                &mut physics,
                &mut animation,
                &mut sfx,
                &mut fx,
            );
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
        // ------------------------------------------------------------------
        // Attached -> user pulls away -> detach immediately.
        //
        // SittingExit is intentionally not part of desktop detachment anymore.
        // Cursor ownership stays continuous, so drag never accumulates a large
        // offset while an exit animation is playing.
        // ------------------------------------------------------------------
        if matches!(
            environment.placement,
            PlacementState::Window(_) | PlacementState::Taskbar(_)
        ) {
            let unsnap_distance = (UNSNAP_DRAG_DISTANCE_AT_96_DPI
                * environment.nera_scale_factor as f32)
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
                interaction.recent_unsnap_surface_y = environment.current_attachment_surface_y();
                interaction.unsnap_cooldown_until = Some(now + UNSNAP_COOLDOWN);

                environment.begin_nera_drag();
                interaction.snap_cursor = None;
            }
        }

        if let Some(previous) = interaction.last_drag_cursor {
            let delta = cursor - previous;
            if delta != IVec2::ZERO {
                dragged.write(NeraDragged {
                    screen: cursor,
                    delta,
                });
            }
        }
        interaction.last_drag_cursor = Some(cursor);

        // ------------------------------------------------------------------
        // Free drag -> native HWND follows cursor immediately after detach.
        // Pending/attached states intentionally keep the HWND pinned.
        // ------------------------------------------------------------------
        if environment.placement == PlacementState::Free {
            interaction.momentum.sample(now, cursor);
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
                    let guard_radius = (UNSNAP_GUARD_RADIUS_AT_96_DPI
                        * environment.nera_scale_factor as f32)
                        .round()
                        .max(1.0) as i32;

                    let delta = probe - center;
                    guard_blocked = delta.length_squared() < guard_radius * guard_radius;

                    if !guard_blocked {
                        interaction.guard_center = None;
                    }
                }

                if let Some(surface_y) = interaction.recent_unsnap_surface_y {
                    let vertical_band = (UNSNAP_VERTICAL_BAND_AT_96_DPI
                        * environment.nera_scale_factor as f32)
                        .round()
                        .max(1.0) as i32;

                    vertical_blocked = (probe.y - surface_y).abs() < vertical_band;

                    if !vertical_blocked {
                        interaction.recent_unsnap_surface_y = None;
                    }
                }
            }

            let moving_fast = settings.throw_strength() > 0.0
                && interaction
                    .momentum
                    .velocity(environment.nera_scale_factor as f32)
                    != Vec2::ZERO;
            if !cloud.owns_placement()
                && hold_ready
                && !moving_fast
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
        if was_dragging
            && !cloud.is_riding()
            && environment.placement == PlacementState::Free
            && settings.throw_strength() > 0.0
        {
            interaction.momentum.sample(now, cursor);
            let velocity = interaction
                .momentum
                .velocity(environment.nera_scale_factor as f32);
            if velocity != Vec2::ZERO {
                thrown.write(NeraThrow { velocity });
            }
        }
        interaction.momentum.clear();
        let pointer = nera_pointer_position(cursor, &environment);

        if let Some(body_part) = interaction.pressed_body_part {
            if !was_dragging {
                clicked.write(NeraClicked { pointer, body_part });
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
