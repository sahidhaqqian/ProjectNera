//! Phase 5.3 — Animation State Machine.
//!
//! Two-layer controller that cleanly separates **base poses** (driven by
//! `PlacementState`) from **temporary actions** (driven by interactions,
//! alarms, etc.).
//!
//! ```text
//! PlacementState ──► BasePose   (loops forever)
//!                        +
//! Interaction    ──► ActionAnimation (plays once, returns to base)
//! ```
//!
//! Future phases simply call `AnimationController::request_action()` without
//! touching the VRMA playback internals.

use std::time::Duration;

use bevy::{animation::RepeatAnimation, prelude::*};
use bevy_vrm1::prelude::*;

use crate::environment::{AvatarAnchor, DesktopEnvironment, PlacementState};

// -----------------------------------------------------------------------------
// Enums
// -----------------------------------------------------------------------------

/// Base-layer pose, mapped deterministically from `PlacementState`.
/// Loops forever. Only one base pose is active at a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum BasePose {
    #[default]
    Idle,
    Sitting,
    // Future: Falling
}

/// Temporary action that plays **once** over the current base pose,
/// then auto-returns to the base animation.
///
/// The enum is intentionally empty for Phase 5.3. Phase 5.4 will add
/// HeadPat, Wave, etc. The full engine is built and ready.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ActionAnimation {
    // Phase 5.4: HeadPat, Wave, Surprised
    // Phase 5.5: Alarm
    // Phase 5.6: Landing
    _Placeholder, // removed once real variants arrive
}

/// Determines which animation wins when multiple compete.
///
/// A higher-priority action cancels a lower one.
/// Same-or-lower priority is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum AnimationPriority {
    /// Idle, Sitting — the fallback.
    Base,
    /// HeadPat, Wave — nice but interruptible.
    Reaction,
    /// Alarm — important, overrides reactions.
    Important,
    /// Dragging, Falling — physics-driven, cannot be interrupted.
    Forced,
}

// -----------------------------------------------------------------------------
// Controller
// -----------------------------------------------------------------------------

/// What is actually playing on the VRMA layer right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlayingState {
    Base(BasePose),
    Action(ActionAnimation),
}

/// A temporary action currently in progress.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ActiveAction {
    pub(crate) action: ActionAnimation,
    pub(crate) priority: AnimationPriority,
    pub(crate) remaining: Duration,
}

/// Central animation resource. Owned by the `AnimationPlugin`.
///
/// Other systems read `base_pose()` and `is_action_active()`.
/// Future phases call `request_action()` to trigger temporary animations.
#[derive(Resource)]
pub(crate) struct AnimationController {
    // -- base layer -----------------------------------------------------------
    base_pose: BasePose,

    // -- action layer ---------------------------------------------------------
    active_action: Option<ActiveAction>,

    // -- VRMA handles (filled by LoadedVrma observers) ------------------------
    idle_vrma: Option<Entity>,
    sitting_vrma: Option<Entity>,
    // Future: action VRMAs registered here

    // -- playback tracking (avoid redundant PlayVrma triggers) ----------------
    playing: Option<PlayingState>,
    startup_done: bool,
}

impl Default for AnimationController {
    fn default() -> Self {
        Self {
            base_pose: BasePose::Idle,
            active_action: None,
            idle_vrma: None,
            sitting_vrma: None,
            playing: None,
            startup_done: false,
        }
    }
}

impl AnimationController {
    // -- public read API ------------------------------------------------------

    /// Current base pose (read-only).
    pub(crate) fn base_pose(&self) -> BasePose {
        self.base_pose
    }

    /// Whether a temporary action is currently playing.
    #[allow(dead_code)]
    pub(crate) fn is_action_active(&self) -> bool {
        self.active_action.is_some()
    }

    // -- public write API -----------------------------------------------------

    /// Try to play a temporary action animation.
    ///
    /// Returns `true` if the action was accepted. Returns `false` if a
    /// higher-or-equal priority action is already in progress.
    #[allow(dead_code)]
    pub(crate) fn request_action(
        &mut self,
        action: ActionAnimation,
        priority: AnimationPriority,
        duration: Duration,
    ) -> bool {
        // A higher-priority action is already running — reject.
        if let Some(ref current) = self.active_action {
            if current.priority >= priority {
                return false;
            }
        }

        self.active_action = Some(ActiveAction {
            action,
            priority,
            remaining: duration,
        });

        info!(
            "Animation action started: {:?} priority={:?} duration={:?}",
            action, priority, duration
        );

        true
    }

    /// Force-cancel the current action and return to the base pose.
    #[allow(dead_code)]
    pub(crate) fn cancel_action(&mut self) {
        if let Some(action) = self.active_action.take() {
            info!("Animation action cancelled: {:?}", action.action);
        }
    }

    // -- internal helpers -----------------------------------------------------

    /// VRMA entity for the given base pose, if loaded.
    fn vrma_for_base(&self, pose: BasePose) -> Option<Entity> {
        match pose {
            BasePose::Idle => self.idle_vrma,
            BasePose::Sitting => self.sitting_vrma,
        }
    }

    /// Avatar anchor appropriate for the given base pose.
    fn anchor_for_base(pose: BasePose) -> AvatarAnchor {
        match pose {
            BasePose::Idle => AvatarAnchor::Feet,
            BasePose::Sitting => AvatarAnchor::Seat,
        }
    }
}

// -----------------------------------------------------------------------------
// Plugin
// -----------------------------------------------------------------------------

pub(crate) struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AnimationController>();
    }
}

// -----------------------------------------------------------------------------
// VRMA registration observers (called from spawn_nera)
// -----------------------------------------------------------------------------

/// Observer for the idle VRMA. Stores the handle and kicks the first animation.
pub(crate) fn register_idle_animation(
    trigger: On<LoadedVrma>,
    mut commands: Commands,
    mut controller: ResMut<AnimationController>,
) {
    controller.idle_vrma = Some(trigger.vrma);

    // Idle is the startup pose. Kick it immediately.
    if !controller.startup_done {
        commands.trigger(PlayVrma {
            repeat: RepeatAnimation::Forever,
            transition_duration: Duration::ZERO,
            vrma: trigger.vrma,
            reset_spring_bones: false,
        });

        controller.playing = Some(PlayingState::Base(BasePose::Idle));
        controller.startup_done = true;
        info!("Idle VRMA loaded. Starting animation.");
    }
}

/// Observer for the sitting VRMA.
pub(crate) fn register_sitting_animation(
    trigger: On<LoadedVrma>,
    mut controller: ResMut<AnimationController>,
) {
    controller.sitting_vrma = Some(trigger.vrma);
    info!("Sitting VRMA loaded.");
}

// -----------------------------------------------------------------------------
// Systems
// -----------------------------------------------------------------------------

/// Maps `PlacementState` to `BasePose`. Pure data — no PlayVrma here.
pub(crate) fn update_base_pose(
    environment: Res<DesktopEnvironment>,
    mut controller: ResMut<AnimationController>,
) {
    let desired = match environment.placement {
        PlacementState::Free => BasePose::Idle,
        PlacementState::PendingWindow { .. }
        | PlacementState::PendingTaskbar { .. }
        | PlacementState::Window(_)
        | PlacementState::Taskbar(_) => BasePose::Sitting,
    };

    if controller.base_pose != desired {
        info!("base_pose -> {:?}", desired);
        controller.base_pose = desired;
    }
}

/// Counts down `ActiveAction::remaining`. Clears expired actions.
pub(crate) fn tick_active_action(
    time: Res<Time>,
    mut controller: ResMut<AnimationController>,
) {
    let mut finished = false;
    let mut action_name = None;

    if let Some(ref mut action) = controller.active_action {
        let dt = time.delta();
        if action.remaining <= dt {
            finished = true;
            action_name = Some(action.action);
        } else {
            action.remaining -= dt;
        }
    }

    if finished {
        if let Some(name) = action_name {
            info!(
                "Animation action finished: {:?}, returning to base {:?}",
                name, controller.base_pose
            );
        }
        controller.active_action = None;
    }
}

/// Decides what should actually play on the VRMA layer.
///
/// Compares `playing` vs the desired state (action if active, else base pose).
/// Emits `PlayVrma` only when a change is needed. Also sets `AvatarAnchor`.
pub(crate) fn sync_animation(
    mut commands: Commands,
    mut environment: ResMut<DesktopEnvironment>,
    mut controller: ResMut<AnimationController>,
) {
    // -- determine desired state ----------------------------------------------
    let desired_playing = if let Some(ref action) = controller.active_action {
        PlayingState::Action(action.action)
    } else {
        PlayingState::Base(controller.base_pose)
    };

    // -- anchor always follows the base pose, even during actions -------------
    let anchor = AnimationController::anchor_for_base(controller.base_pose);
    environment.set_attached_anchor(anchor);

    // -- skip if already playing the right thing ------------------------------
    if controller.playing == Some(desired_playing) {
        return;
    }

    // -- resolve VRMA entity --------------------------------------------------
    let (vrma, repeat) = match desired_playing {
        PlayingState::Base(pose) => {
            let Some(vrma) = controller.vrma_for_base(pose) else {
                // Asset not loaded yet. For Idle fallback, still set anchor.
                if pose == BasePose::Idle {
                    environment.set_attached_anchor(AvatarAnchor::Feet);
                }
                return;
            };
            (vrma, RepeatAnimation::Forever)
        }
        PlayingState::Action(_action) => {
            // Future: look up action VRMA from a registry.
            // For now no action VRMAs exist, so fall through to base.
            let Some(vrma) = controller.vrma_for_base(controller.base_pose) else {
                return;
            };
            (vrma, RepeatAnimation::Count(1))
        }
    };

    // -- fire PlayVrma --------------------------------------------------------
    commands.trigger(PlayVrma {
        repeat,
        transition_duration: Duration::ZERO,
        vrma,
        reset_spring_bones: false,
    });

    match desired_playing {
        PlayingState::Base(pose) => {
            info!("Nera animation -> {:?} | anchor -> {:?}", pose, anchor);
        }
        PlayingState::Action(action) => {
            info!("Nera animation -> action {:?} | anchor stays {:?}", action, anchor);
        }
    }

    controller.playing = Some(desired_playing);
}
