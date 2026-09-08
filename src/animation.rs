//! Phase 5.3 — Animation State Machine.
//!
//! Logical animation state is intentionally separate from VRMA playback.
//! `BasePose` describes where Nera belongs in the world, while temporary
//! actions describe what she is doing right now. Higher-priority actions can
//! temporarily win without destroying the base pose, so ending the action
//! naturally returns Nera to the correct Idle/Sitting state.

use std::time::Duration;

use bevy::{
    animation::RepeatAnimation,
    prelude::*,
};
use bevy_vrm1::prelude::*;

use crate::{
    environment::{
        AvatarAnchor,
        DesktopEnvironment,
        PlacementState,
    },
    interaction::{
        NeraDragStarted,
        NeraReleased,
    },
};

// -----------------------------------------------------------------------------
// Logical state
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum BasePose {
    #[default]
    Idle,
    Sitting,
    /// Reserved for Phase 5.6. It already participates in priority resolution
    /// so gravity can plug in without redesigning the animation state model.
    #[allow(dead_code)]
    Falling,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TemporaryAction {
    /// Real Phase 5.3 action driven by the existing interaction pipeline.
    Dragging,
    /// Reserved for Phase 5.4 expressions/reactions.
    #[allow(dead_code)]
    Reaction,
    /// Reserved for Phase 5.5 alarm behavior.
    #[allow(dead_code)]
    Alarm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(crate) enum AnimationPriority {
    Idle = 10,
    Sitting = 20,
    Reaction = 60,
    Alarm = 70,
    Dragging = 80,
    Falling = 90,
}

impl BasePose {
    fn priority(self) -> AnimationPriority {
        match self {
            BasePose::Idle => AnimationPriority::Idle,
            BasePose::Sitting => AnimationPriority::Sitting,
            BasePose::Falling => AnimationPriority::Falling,
        }
    }
}

impl TemporaryAction {
    fn priority(self) -> AnimationPriority {
        match self {
            TemporaryAction::Reaction => AnimationPriority::Reaction,
            TemporaryAction::Alarm => AnimationPriority::Alarm,
            TemporaryAction::Dragging => AnimationPriority::Dragging,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedAnimationState {
    Base(BasePose),
    Action(TemporaryAction),
}

/// Single logical source of truth for Nera's animation behavior.
///
/// Multiple temporary actions may remain active at once. The highest priority
/// one wins visually/logically, but lower-priority actions are not destroyed.
/// Example: Alarm + Dragging resolves to Dragging; when Dragging ends, Alarm
/// becomes effective again instead of being lost.
#[derive(Clone, Copy, Debug)]
struct ActiveAction {
    action: TemporaryAction,
    /// `None` means event-driven/persistent (Dragging). Timed reactions and
    /// alarms use `Some(remaining)` and expire automatically.
    remaining: Option<Duration>,
}

#[derive(Resource, Debug)]
pub(crate) struct NeraAnimationState {
    base_pose: BasePose,
    active_actions: Vec<ActiveAction>,
}

impl Default for NeraAnimationState {
    fn default() -> Self {
        Self {
            base_pose: BasePose::Idle,
            active_actions: Vec::with_capacity(3),
        }
    }
}

impl NeraAnimationState {
    pub(crate) fn base_pose(&self) -> BasePose {
        self.base_pose
    }

    pub(crate) fn set_base_pose(&mut self, pose: BasePose) -> bool {
        if self.base_pose == pose {
            return false;
        }

        let previous = self.base_pose;
        self.base_pose = pose;
        info!("Nera base pose: {:?} -> {:?}", previous, pose);
        true
    }

    /// Starts an event-driven action that remains active until explicitly
    /// finished. Dragging uses this lifecycle.
    pub(crate) fn start_action(&mut self, action: TemporaryAction) -> bool {
        if self.active_actions.iter().any(|active| active.action == action) {
            return false;
        }

        self.active_actions.push(ActiveAction {
            action,
            remaining: None,
        });
        info!(
            "Nera temporary action +{:?} priority={:?}",
            action,
            action.priority()
        );
        true
    }

    /// Starts or refreshes a timed action. This is the Phase 5.4/5.5 entry
    /// point for reactions and alarms that should return automatically.
    #[allow(dead_code)]
    pub(crate) fn request_timed_action(
        &mut self,
        action: TemporaryAction,
        duration: Duration,
    ) -> bool {
        if let Some(active) = self
            .active_actions
            .iter_mut()
            .find(|active| active.action == action)
        {
            active.remaining = Some(duration);
            info!(
                "Nera temporary action refreshed {:?} duration={:?}",
                action, duration
            );
            return false;
        }

        self.active_actions.push(ActiveAction {
            action,
            remaining: Some(duration),
        });
        info!(
            "Nera temporary action +{:?} priority={:?} duration={:?}",
            action,
            action.priority(),
            duration
        );
        true
    }

    pub(crate) fn finish_action(&mut self, action: TemporaryAction) -> bool {
        let old_len = self.active_actions.len();
        self.active_actions.retain(|active| active.action != action);

        if self.active_actions.len() == old_len {
            return false;
        }

        info!("Nera temporary action -{:?}", action);
        true
    }

    #[allow(dead_code)]
    pub(crate) fn cancel_action(&mut self, action: TemporaryAction) -> bool {
        self.finish_action(action)
    }

    fn advance_actions(&mut self, delta: Duration) {
        let mut expired = Vec::new();

        for active in &mut self.active_actions {
            let Some(remaining) = active.remaining.as_mut() else {
                continue;
            };

            if *remaining <= delta {
                expired.push(active.action);
            } else {
                *remaining -= delta;
            }
        }

        if expired.is_empty() {
            return;
        }

        self.active_actions
            .retain(|active| !expired.contains(&active.action));

        for action in expired {
            info!(
                "Nera temporary action expired {:?}; resolving back to next priority/base",
                action
            );
        }
    }

    fn resolved(&self) -> ResolvedAnimationState {
        let mut resolved = ResolvedAnimationState::Base(self.base_pose);
        let mut priority = self.base_pose.priority();

        for active in &self.active_actions {
            let action_priority = active.action.priority();
            if action_priority > priority {
                resolved = ResolvedAnimationState::Action(active.action);
                priority = action_priority;
            }
        }

        resolved
    }
}

// -----------------------------------------------------------------------------
// VRMA playback
// -----------------------------------------------------------------------------

#[derive(Resource, Default)]
pub(crate) struct NeraAnimationClips {
    idle: Option<Entity>,
    sitting: Option<Entity>,
}

#[derive(Resource, Default)]
pub(crate) struct NeraAnimationPlayback {
    current_clip: Option<Entity>,
    resolved: Option<ResolvedAnimationState>,
}

pub(crate) struct NeraAnimationPlugin;

impl Plugin for NeraAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NeraAnimationState>()
            .init_resource::<NeraAnimationClips>()
            .init_resource::<NeraAnimationPlayback>();
    }
}

pub(crate) fn register_idle_animation(
    trigger: On<LoadedVrma>,
    mut clips: ResMut<NeraAnimationClips>,
) {
    clips.idle = Some(trigger.vrma);
    info!("Idle VRMA loaded.");
}

pub(crate) fn register_sitting_animation(
    trigger: On<LoadedVrma>,
    mut clips: ResMut<NeraAnimationClips>,
) {
    clips.sitting = Some(trigger.vrma);
    info!("Sitting VRMA loaded.");
}

/// Environment placement is the source of truth for the current base pose.
///
/// Falling is reserved for Phase 5.6; until gravity exists, Free means Idle
/// and every pending/committed support attachment means Sitting.
pub(crate) fn sync_animation_base_pose(
    environment: Res<DesktopEnvironment>,
    mut state: ResMut<NeraAnimationState>,
) {
    let desired = match environment.placement {
        PlacementState::Free => BasePose::Idle,
        PlacementState::PendingWindow { .. }
        | PlacementState::PendingTaskbar { .. }
        | PlacementState::Window(_)
        | PlacementState::Taskbar(_) => BasePose::Sitting,
    };

    state.set_base_pose(desired);
}

/// Phase 5.3's first real temporary action is Dragging.
///
/// This is deliberately driven by semantic interaction messages rather than
/// mouse state, keeping animation behavior decoupled from Win32 drag details.
pub(crate) fn sync_interaction_animation_actions(
    mut drag_started: MessageReader<NeraDragStarted>,
    mut released: MessageReader<NeraReleased>,
    mut state: ResMut<NeraAnimationState>,
) {
    if drag_started.read().next().is_some() {
        state.start_action(TemporaryAction::Dragging);
    }

    if released.read().any(|event| event.was_dragging) {
        state.finish_action(TemporaryAction::Dragging);
    }
}

/// Advances timed temporary actions. Persistent actions such as Dragging are
/// unaffected and remain active until their semantic end event arrives.
pub(crate) fn tick_animation_actions(
    time: Res<Time>,
    mut state: ResMut<NeraAnimationState>,
) {
    state.advance_actions(time.delta());
}

/// Applies the resolved logical state to the avatar.
///
/// Phase 5.3 has only Idle/Sitting VRMA assets. Temporary actions already
/// participate in priority resolution, but until their own clips arrive in
/// 5.4/5.5 they intentionally fall back to the current base-pose clip.
/// This proves the state machine without inventing placeholder animations.
pub(crate) fn apply_animation_state(
    mut commands: Commands,
    mut environment: ResMut<DesktopEnvironment>,
    state: Res<NeraAnimationState>,
    clips: Res<NeraAnimationClips>,
    mut playback: ResMut<NeraAnimationPlayback>,
) {
    let anchor = match state.base_pose() {
        BasePose::Idle | BasePose::Falling => AvatarAnchor::Feet,
        BasePose::Sitting => AvatarAnchor::Seat,
    };
    environment.set_attached_anchor(anchor);

    let resolved = state.resolved();

    if playback.resolved != Some(resolved) {
        info!(
            "Nera animation resolved -> {:?} | base={:?}",
            resolved,
            state.base_pose()
        );
        playback.resolved = Some(resolved);
    }

    // No dedicated action clips exist yet, so actions preserve the current
    // base animation. Phase 5.4 can extend this match with reaction VRMAs
    // without touching state/priority semantics.
    let desired_clip = match state.base_pose() {
        BasePose::Idle => clips.idle,
        BasePose::Sitting => clips.sitting,
        BasePose::Falling => clips.idle,
    };

    let Some(vrma) = desired_clip else {
        return;
    };

    if playback.current_clip == Some(vrma) {
        return;
    }

    commands.trigger(PlayVrma {
        repeat: RepeatAnimation::Forever,
        transition_duration: Duration::ZERO,
        vrma,
        reset_spring_bones: false,
    });

    match state.base_pose() {
        BasePose::Idle => info!("Nera playback -> Idle | anchor -> Feet"),
        BasePose::Sitting => {
            info!("Nera playback -> Sitting | pose-first Seat calibration")
        }
        BasePose::Falling => {
            info!("Nera playback -> Falling fallback | anchor -> Feet")
        }
    }

    playback.current_clip = Some(vrma);
}

// -----------------------------------------------------------------------------
// State-machine tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_action_returns_to_sitting_base() {
        let mut state = NeraAnimationState::default();
        state.set_base_pose(BasePose::Sitting);
        state.start_action(TemporaryAction::Reaction);

        assert_eq!(
            state.resolved(),
            ResolvedAnimationState::Action(TemporaryAction::Reaction)
        );

        state.finish_action(TemporaryAction::Reaction);

        assert_eq!(
            state.resolved(),
            ResolvedAnimationState::Base(BasePose::Sitting)
        );
    }

    #[test]
    fn higher_priority_action_temporarily_overrides_lower_one() {
        let mut state = NeraAnimationState::default();
        state.start_action(TemporaryAction::Alarm);
        state.start_action(TemporaryAction::Dragging);

        assert_eq!(
            state.resolved(),
            ResolvedAnimationState::Action(TemporaryAction::Dragging)
        );

        state.finish_action(TemporaryAction::Dragging);

        assert_eq!(
            state.resolved(),
            ResolvedAnimationState::Action(TemporaryAction::Alarm)
        );
    }

    #[test]
    fn timed_action_expires_and_returns_to_base() {
        let mut state = NeraAnimationState::default();
        state.set_base_pose(BasePose::Sitting);
        state.request_timed_action(
            TemporaryAction::Reaction,
            Duration::from_millis(500),
        );

        state.advance_actions(Duration::from_millis(499));
        assert_eq!(
            state.resolved(),
            ResolvedAnimationState::Action(TemporaryAction::Reaction)
        );

        state.advance_actions(Duration::from_millis(1));
        assert_eq!(
            state.resolved(),
            ResolvedAnimationState::Base(BasePose::Sitting)
        );
    }

    #[test]
    fn falling_overrides_temporary_actions() {
        let mut state = NeraAnimationState::default();
        state.start_action(TemporaryAction::Dragging);
        state.set_base_pose(BasePose::Falling);

        assert_eq!(
            state.resolved(),
            ResolvedAnimationState::Base(BasePose::Falling)
        );
    }
}
