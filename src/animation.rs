//! Project Nera — Phase 5.4 serious animation controller.
//!
//! Design goals:
//! - `animasi_basic/` is the small runtime animation set.
//! - Base pose, support transitions, and temporary interactions are separate.
//! - One-shot lifetimes use the VRMA's real `VrmaDuration` instead of guessed
//!   millisecond constants.
//! - Sitting is a real sequence:
//!       Sitting_Enter -> Sitting_Idle_Loop -> Sitting_Exit
//! - Dragging uses Floating.
//! - Head/chest clicks and head strokes are semantic reactions.
//! - Praying is registered now but only becomes active when the future menu
//!   lifecycle (Phase 5.7) explicitly opens/closes it.

use std::time::Duration;

use bevy::{animation::RepeatAnimation, prelude::*};
use bevy_vrm1::prelude::*;

use crate::{
    environment::{AvatarAnchor, DesktopEnvironment, PlacementState},
    interaction::{
        BodyPart, NeraClicked, NeraDragStarted, NeraHeadPatted, NeraReleased,
    },
};

/// Three deliberate head hits inside this window trigger Death01.
/// The combo expires naturally so occasional clicks do not accumulate forever.
const DAMAGE_COMBO_WINDOW: Duration = Duration::from_millis(2500);

/// Short cross-fades hide one-frame bind/T-pose flashes that some converted
/// VRMA clips contain at their first sample.
const BASE_BLEND: Duration = Duration::from_millis(100);
const REACTION_BLEND: Duration = Duration::from_millis(140);
const HEAD_STROKE_BLEND: Duration = Duration::from_millis(180);
const SUPPORT_BLEND: Duration = Duration::from_millis(100);
const DEATH_BLEND: Duration = Duration::from_millis(160);

// -----------------------------------------------------------------------------
// Animation vocabulary
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum BasePose {
    #[default]
    Idle,
    Sitting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TemporaryAction {
    Floating,
    HitHead,
    HitChest,
    HeadStroke,
    Death,
    /// Phase 5.7: persistent while the right-click menu is open.
    #[allow(dead_code)]
    Praying,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SupportTransition {
    SittingEnter,
    SittingExit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
enum AnimationPriority {
    Reaction = 60,
    Menu = 70,
    Floating = 80,
    Critical = 90,
}

impl TemporaryAction {
    fn priority(self) -> AnimationPriority {
        match self {
            Self::HitHead | Self::HitChest | Self::HeadStroke => {
                AnimationPriority::Reaction
            }
            Self::Praying => AnimationPriority::Menu,
            Self::Floating => AnimationPriority::Floating,
            Self::Death => AnimationPriority::Critical,
        }
    }

    fn is_reaction(self) -> bool {
        self.priority() == AnimationPriority::Reaction
    }

    fn is_critical(self) -> bool {
        self.priority() == AnimationPriority::Critical
    }
}

#[derive(Clone, Copy, Debug)]
struct ActiveAction {
    action: TemporaryAction,
    /// `None` means event-driven/persistent. Floating and future Praying use
    /// this. One-shot reactions use the actual VRMA duration.
    remaining: Option<Duration>,
}

#[derive(Clone, Copy, Debug)]
struct TimedSupportTransition {
    kind: SupportTransition,
    remaining: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisualState {
    Idle,
    SittingLoop,
    SittingEnter,
    SittingExit,
    Floating,
    HitHead,
    HitChest,
    HeadStroke,
    Death,
    Praying,
}

// -----------------------------------------------------------------------------
// Runtime state
// -----------------------------------------------------------------------------

#[derive(Resource, Debug)]
pub(crate) struct NeraAnimationState {
    base_pose: BasePose,
    active_actions: Vec<ActiveAction>,
    support_transition: Option<TimedSupportTransition>,
    was_on_sitting_surface: bool,

    damage_hit_count: u8,
    damage_combo_remaining: Option<Duration>,

    /// Changes whenever an action/transition is explicitly requested. This
    /// lets repeated HitHead clicks replay the same VRMA from frame zero.
    revision: u64,
}

impl Default for NeraAnimationState {
    fn default() -> Self {
        Self {
            base_pose: BasePose::Idle,
            active_actions: Vec::with_capacity(4),
            support_transition: None,
            was_on_sitting_surface: false,
            damage_hit_count: 0,
            damage_combo_remaining: None,
            revision: 0,
        }
    }
}

impl NeraAnimationState {
    pub(crate) fn base_pose(&self) -> BasePose {
        self.base_pose
    }

    fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn set_base_pose(&mut self, pose: BasePose) {
        if self.base_pose != pose {
            info!("Nera base pose: {:?} -> {:?}", self.base_pose, pose);
            self.base_pose = pose;
        }
    }

    fn start_persistent_action(&mut self, action: TemporaryAction) {
        if self
            .active_actions
            .iter()
            .any(|active| active.action == action)
        {
            return;
        }

        self.active_actions.push(ActiveAction {
            action,
            remaining: None,
        });
        self.bump_revision();
        info!(
            "Nera action +{:?} priority={:?}",
            action,
            action.priority()
        );
    }

    fn request_one_shot(&mut self, action: TemporaryAction, duration: Duration) {
        if duration.is_zero() {
            warn!(
                "Nera ignored {:?}: VRMA duration unavailable/zero",
                action
            );
            return;
        }

        // Only one semantic reaction at a time. Higher-level persistent
        // states (Floating / future Praying) remain alive independently.
        if action.is_reaction() {
            // Normal reactions replace other normal reactions, but never
            // cancel an active critical Death reaction.
            self.active_actions
                .retain(|active| !active.action.is_reaction());
        } else if action.is_critical() {
            // Death owns the reaction layer completely.
            self.active_actions.retain(|active| {
                !active.action.is_reaction()
                    && !active.action.is_critical()
            });
        } else {
            self.active_actions
                .retain(|active| active.action != action);
        }

        self.active_actions.push(ActiveAction {
            action,
            remaining: Some(duration),
        });
        self.bump_revision();

        info!(
            "Nera reaction -> {:?} duration={:?}",
            action, duration
        );
    }

    fn reset_damage_combo(&mut self) {
        self.damage_hit_count = 0;
        self.damage_combo_remaining = None;
    }

    /// Registers one damage reaction from either Head or Torso.
    ///
    /// Any three valid damage hits inside `DAMAGE_COMBO_WINDOW` trigger
    /// Death01. This intentionally allows Head+Chest mixed combos.
    fn register_damage_hit(
        &mut self,
        reaction: TemporaryAction,
        reaction_duration: Duration,
        death_duration: Duration,
    ) {
        debug_assert!(matches!(
            reaction,
            TemporaryAction::HitHead | TemporaryAction::HitChest
        ));

        if self.damage_combo_remaining.is_none() {
            self.damage_hit_count = 0;
        }

        self.damage_hit_count = self.damage_hit_count.saturating_add(1);
        self.damage_combo_remaining = Some(DAMAGE_COMBO_WINDOW);

        if self.damage_hit_count >= 3 {
            info!("Nera damage combo 3/3 -> Death");
            self.reset_damage_combo();

            if !death_duration.is_zero() {
                self.request_one_shot(
                    TemporaryAction::Death,
                    death_duration,
                );
            } else {
                warn!(
                    "Death01 duration unavailable; falling back to {:?}",
                    reaction
                );
                self.request_one_shot(
                    reaction,
                    reaction_duration,
                );
            }
            return;
        }

        info!(
            "Nera damage combo {}/3 via {:?}",
            self.damage_hit_count,
            reaction
        );

        self.request_one_shot(
            reaction,
            reaction_duration,
        );
    }

    fn finish_action(&mut self, action: TemporaryAction) {
        let before = self.active_actions.len();
        self.active_actions
            .retain(|active| active.action != action);

        if before != self.active_actions.len() {
            info!("Nera action -{:?}", action);
        }
    }

    /// Phase 5.7 menu hook. The menu system should call this when the actual
    /// menu opens/closes; right-click alone is not enough because we need the
    /// menu lifecycle to know when Praying should end.
    #[allow(dead_code)]
    pub(crate) fn set_praying_menu(&mut self, open: bool) {
        if open {
            self.start_persistent_action(TemporaryAction::Praying);
        } else {
            self.finish_action(TemporaryAction::Praying);
        }
    }

    fn begin_support_transition(
        &mut self,
        kind: SupportTransition,
        duration: Duration,
    ) {
        if duration.is_zero() {
            warn!(
                "Nera support transition {:?} skipped: clip duration unavailable",
                kind
            );
            self.support_transition = None;
            return;
        }

        self.support_transition = Some(TimedSupportTransition {
            kind,
            remaining: duration,
        });
        self.bump_revision();

        match kind {
            SupportTransition::SittingEnter => info!(
                "Nera support transition -> SittingEnter (landing) duration={:?}",
                duration
            ),
            SupportTransition::SittingExit => info!(
                "Nera support transition -> SittingExit duration={:?}",
                duration
            ),
        }
    }

    fn observe_sitting_surface(
        &mut self,
        sitting_surface: bool,
        enter_duration: Duration,
        exit_duration: Duration,
    ) {
        if sitting_surface == self.was_on_sitting_surface {
            self.set_base_pose(if sitting_surface {
                BasePose::Sitting
            } else {
                BasePose::Idle
            });
            return;
        }

        self.was_on_sitting_surface = sitting_surface;

        if sitting_surface {
            self.set_base_pose(BasePose::Sitting);
            self.begin_support_transition(
                SupportTransition::SittingEnter,
                enter_duration,
            );
        } else {
            self.set_base_pose(BasePose::Idle);
            self.begin_support_transition(
                SupportTransition::SittingExit,
                exit_duration,
            );
        }
    }

    fn advance(&mut self, delta: Duration) {
        let mut expired_actions = Vec::new();

        for active in &mut self.active_actions {
            let Some(remaining) = active.remaining.as_mut() else {
                continue;
            };

            if *remaining <= delta {
                expired_actions.push(active.action);
            } else {
                *remaining -= delta;
            }
        }

        if !expired_actions.is_empty() {
            self.active_actions
                .retain(|active| !expired_actions.contains(&active.action));

            for action in expired_actions {
                info!("Nera reaction finished -> {:?}", action);
            }
        }

        let combo_expired = match self.damage_combo_remaining.as_mut() {
            Some(remaining) if *remaining <= delta => true,
            Some(remaining) => {
                *remaining -= delta;
                false
            }
            None => false,
        };

        if combo_expired {
            if self.damage_hit_count > 0 {
                info!(
                    "Nera damage combo expired at {}/3",
                    self.damage_hit_count
                );
            }
            self.reset_damage_combo();
        }

        let transition_finished = match self.support_transition.as_mut() {
            Some(transition) if transition.remaining <= delta => {
                Some(transition.kind)
            }
            Some(transition) => {
                transition.remaining -= delta;
                None
            }
            None => None,
        };

        if let Some(kind) = transition_finished {
            self.support_transition = None;
            info!("Nera support transition finished -> {:?}", kind);
        }
    }

    fn highest_action(&self) -> Option<TemporaryAction> {
        self.active_actions
            .iter()
            .filter(|active| {
                // While attached/pending, Floating stays logically alive but
                // never visually pulls Nera out of the sitting pose.
                !(self.base_pose == BasePose::Sitting
                    && active.action == TemporaryAction::Floating)
            })
            .max_by_key(|active| active.action.priority())
            .map(|active| active.action)
    }

    fn visual_state(&self) -> VisualState {
        if let Some(transition) = self.support_transition {
            return match transition.kind {
                SupportTransition::SittingEnter => VisualState::SittingEnter,
                SupportTransition::SittingExit => VisualState::SittingExit,
            };
        }

        if let Some(action) = self.highest_action() {
            return match action {
                TemporaryAction::Floating => VisualState::Floating,
                TemporaryAction::HitHead => VisualState::HitHead,
                TemporaryAction::HitChest => VisualState::HitChest,
                TemporaryAction::HeadStroke => VisualState::HeadStroke,
                TemporaryAction::Death => VisualState::Death,
                TemporaryAction::Praying => VisualState::Praying,
            };
        }

        match self.base_pose {
            BasePose::Idle => VisualState::Idle,
            BasePose::Sitting => VisualState::SittingLoop,
        }
    }

    pub(crate) fn death_active(&self) -> bool {
        self.visual_state() == VisualState::Death
    }

    /// Head-dominant clips must temporarily own the head bones. bevy_vrm1
    /// applies LookAt after animation, so leaving LookAt enabled would partly
    /// overwrite ShakeHead/HitHead every frame.
    pub(crate) fn suppresses_look_at(&self) -> bool {
        matches!(
            self.visual_state(),
            VisualState::HitHead
                | VisualState::HitChest
                | VisualState::HeadStroke
                | VisualState::Death
                | VisualState::Praying
                | VisualState::SittingEnter
                | VisualState::SittingExit
        )
    }
}

// -----------------------------------------------------------------------------
// Clip registry — entity + real VRMA duration
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct ClipSlot {
    entity: Entity,
    duration: Duration,
}

#[derive(Resource, Default)]
pub(crate) struct NeraAnimationClips {
    idle: Option<ClipSlot>,
    floating: Option<ClipSlot>,
    death: Option<ClipSlot>,
    hit_head: Option<ClipSlot>,
    hit_chest: Option<ClipSlot>,
    head_stroke: Option<ClipSlot>,
    praying: Option<ClipSlot>,
    sitting_enter: Option<ClipSlot>,
    sitting_loop: Option<ClipSlot>,
    sitting_exit: Option<ClipSlot>,
}

#[derive(Resource, Default)]
pub(crate) struct NeraAnimationPlayback {
    current_clip: Option<Entity>,
    visual: Option<VisualState>,
    revision: u64,
}

pub(crate) struct NeraAnimationPlugin;

impl Plugin for NeraAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NeraAnimationState>()
            .init_resource::<NeraAnimationClips>()
            .init_resource::<NeraAnimationPlayback>();
    }
}

macro_rules! register_clip {
    ($fn_name:ident, $field:ident, $label:literal) => {
        pub(crate) fn $fn_name(
            trigger: On<LoadedVrma>,
            duration_query: Query<&VrmaDuration>,
            mut clips: ResMut<NeraAnimationClips>,
        ) {
            let duration = duration_query
                .get(trigger.vrma)
                .map(|duration| duration.0)
                .unwrap_or(Duration::ZERO);

            clips.$field = Some(ClipSlot {
                entity: trigger.vrma,
                duration,
            });

            info!(
                concat!($label, " VRMA loaded | duration={:?}"),
                duration
            );
        }
    };
}

register_clip!(register_idle_animation, idle, "Idle");
register_clip!(register_floating_animation, floating, "Floating");
register_clip!(register_death_animation, death, "Death01");
register_clip!(register_hit_head_animation, hit_head, "HitHead");
register_clip!(register_hit_chest_animation, hit_chest, "HitChest");
register_clip!(
    register_head_stroke_animation,
    head_stroke,
    "ShakeHeadNo1"
);
register_clip!(register_praying_animation, praying, "Praying");
register_clip!(
    register_sitting_enter_animation,
    sitting_enter,
    "SittingEnter"
);
register_clip!(
    register_sitting_loop_animation,
    sitting_loop,
    "SittingLoop"
);
register_clip!(
    register_sitting_exit_animation,
    sitting_exit,
    "SittingExit"
);

// -----------------------------------------------------------------------------
// State synchronization
// -----------------------------------------------------------------------------

pub(crate) fn sync_environment_animation_state(
    environment: Res<DesktopEnvironment>,
    clips: Res<NeraAnimationClips>,
    mut state: ResMut<NeraAnimationState>,
) {
    let sitting_surface = matches!(
        environment.placement,
        PlacementState::PendingWindow { .. }
            | PlacementState::PendingTaskbar { .. }
            | PlacementState::Window(_)
            | PlacementState::Taskbar(_)
    );

    let enter_duration = clips
        .sitting_enter
        .map(|clip| clip.duration)
        .unwrap_or(Duration::ZERO);
    let exit_duration = clips
        .sitting_exit
        .map(|clip| clip.duration)
        .unwrap_or(Duration::ZERO);

    state.observe_sitting_surface(
        sitting_surface,
        enter_duration,
        exit_duration,
    );
}

pub(crate) fn sync_interaction_animation_actions(
    mut drag_started: MessageReader<NeraDragStarted>,
    mut clicked: MessageReader<NeraClicked>,
    mut head_patted: MessageReader<NeraHeadPatted>,
    mut released: MessageReader<NeraReleased>,
    clips: Res<NeraAnimationClips>,
    mut state: ResMut<NeraAnimationState>,
) {
    if drag_started.read().next().is_some() {
        state.start_persistent_action(TemporaryAction::Floating);
    }

    for event in clicked.read() {
        match event.body_part {
            BodyPart::Head => {
                if let Some(hit_clip) = clips.hit_head {
                    let death_duration = clips
                        .death
                        .map(|clip| clip.duration)
                        .unwrap_or(Duration::ZERO);

                    state.register_damage_hit(
                        TemporaryAction::HitHead,
                        hit_clip.duration,
                        death_duration,
                    );
                } else {
                    warn!("HitHead requested before Hit_Head.vrma was loaded");
                }
            }
            BodyPart::Torso => {
                if let Some(hit_clip) = clips.hit_chest {
                    let death_duration = clips
                        .death
                        .map(|clip| clip.duration)
                        .unwrap_or(Duration::ZERO);

                    state.register_damage_hit(
                        TemporaryAction::HitChest,
                        hit_clip.duration,
                        death_duration,
                    );
                } else {
                    warn!("HitChest requested before Hit_Chest.vrma was loaded");
                }
            }
            _ => {
                state.reset_damage_combo();
            }
        }
    }

    if head_patted.read().next().is_some() {
        if let Some(clip) = clips.head_stroke {
            state.request_one_shot(
                TemporaryAction::HeadStroke,
                clip.duration,
            );
        } else {
            warn!("HeadStroke requested before ShakeHeadNo1.vrma was loaded");
        }
    }

    if released.read().any(|event| event.was_dragging) {
        state.finish_action(TemporaryAction::Floating);
    }
}

pub(crate) fn tick_animation_actions(
    time: Res<Time>,
    mut state: ResMut<NeraAnimationState>,
) {
    state.advance(time.delta());
}

// -----------------------------------------------------------------------------
// Playback
// -----------------------------------------------------------------------------

fn clip_for_visual(
    visual: VisualState,
    clips: &NeraAnimationClips,
) -> (Option<ClipSlot>, RepeatAnimation, &'static str) {
    match visual {
        VisualState::Idle => {
            (clips.idle, RepeatAnimation::Forever, "Idle")
        }
        VisualState::SittingLoop => {
            (clips.sitting_loop, RepeatAnimation::Forever, "SittingLoop")
        }
        VisualState::SittingEnter => {
            (clips.sitting_enter, RepeatAnimation::Count(1), "SittingEnter/Landing")
        }
        VisualState::SittingExit => {
            (clips.sitting_exit, RepeatAnimation::Count(1), "SittingExit")
        }
        VisualState::Floating => {
            (clips.floating, RepeatAnimation::Forever, "Floating")
        }
        VisualState::HitHead => {
            (clips.hit_head, RepeatAnimation::Count(1), "HitHead")
        }
        VisualState::HitChest => {
            (clips.hit_chest, RepeatAnimation::Count(1), "HitChest")
        }
        VisualState::HeadStroke => {
            (clips.head_stroke, RepeatAnimation::Count(1), "ShakeHeadNo1")
        }
        VisualState::Death => {
            (clips.death, RepeatAnimation::Count(1), "Death01")
        }
        VisualState::Praying => {
            (clips.praying, RepeatAnimation::Forever, "Praying")
        }
    }
}

fn base_fallback(
    base: BasePose,
    clips: &NeraAnimationClips,
) -> Option<ClipSlot> {
    match base {
        BasePose::Idle => clips.idle,
        BasePose::Sitting => clips.sitting_loop.or(clips.idle),
    }
}

fn transition_for_visual(visual: VisualState) -> Duration {
    match visual {
        VisualState::HeadStroke => HEAD_STROKE_BLEND,
        VisualState::HitHead | VisualState::HitChest => REACTION_BLEND,
        VisualState::Death => DEATH_BLEND,
        VisualState::SittingEnter | VisualState::SittingExit => SUPPORT_BLEND,
        VisualState::Idle
        | VisualState::SittingLoop
        | VisualState::Floating
        | VisualState::Praying => BASE_BLEND,
    }
}

pub(crate) fn apply_animation_state(
    mut commands: Commands,
    mut environment: ResMut<DesktopEnvironment>,
    state: Res<NeraAnimationState>,
    clips: Res<NeraAnimationClips>,
    mut playback: ResMut<NeraAnimationPlayback>,
) {
    let anchor = match state.base_pose() {
        BasePose::Idle => AvatarAnchor::Feet,
        BasePose::Sitting => AvatarAnchor::Seat,
    };
    environment.set_attached_anchor(anchor);

    let visual = state.visual_state();
    let visual_changed = playback.visual != Some(visual);

    if visual_changed {
        info!(
            "Nera animation -> {:?} | base={:?}",
            visual,
            state.base_pose()
        );
        playback.visual = Some(visual);
    }

    let (requested, requested_repeat, requested_label) =
        clip_for_visual(visual, &clips);

    let (clip, repeat, label, using_requested) =
        if let Some(clip) = requested {
            (clip, requested_repeat, requested_label, true)
        } else {
            if visual_changed {
                warn!(
                    "Nera clip unavailable for {:?}; using base fallback",
                    visual
                );
            }

            let Some(fallback) = base_fallback(state.base_pose(), &clips) else {
                return;
            };

            (
                fallback,
                RepeatAnimation::Forever,
                "BaseFallback",
                false,
            )
        };

    let non_base_visual = !matches!(
        visual,
        VisualState::Idle | VisualState::SittingLoop
    );

    let replay_requested = non_base_visual
        && using_requested
        && playback.revision != state.revision;

    if playback.current_clip == Some(clip.entity) && !replay_requested {
        return;
    }

    let transition_duration = transition_for_visual(visual);

    commands.trigger(PlayVrma {
        repeat,
        transition_duration,
        vrma: clip.entity,
        reset_spring_bones: false,
    });

    playback.current_clip = Some(clip.entity);
    playback.revision = state.revision;

    info!(
        "Nera playback -> {} | anchor={:?} | clip_duration={:?} | blend={:?}",
        label,
        anchor,
        clip.duration,
        transition_duration
    );
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floating_overrides_reaction_then_reaction_returns() {
        let mut state = NeraAnimationState::default();
        state.request_one_shot(
            TemporaryAction::HitHead,
            Duration::from_secs(1),
        );
        state.start_persistent_action(TemporaryAction::Floating);

        assert_eq!(state.visual_state(), VisualState::Floating);

        state.finish_action(TemporaryAction::Floating);
        assert_eq!(state.visual_state(), VisualState::HitHead);
    }

    #[test]
    fn new_reaction_replaces_old_reaction() {
        let mut state = NeraAnimationState::default();
        state.request_one_shot(
            TemporaryAction::HitHead,
            Duration::from_secs(1),
        );
        state.request_one_shot(
            TemporaryAction::HitChest,
            Duration::from_secs(1),
        );

        assert_eq!(state.visual_state(), VisualState::HitChest);
        assert_eq!(
            state
                .active_actions
                .iter()
                .filter(|active| active.action.is_reaction())
                .count(),
            1
        );
    }

    #[test]
    fn sitting_enter_transitions_to_sitting_loop() {
        let mut state = NeraAnimationState::default();

        state.observe_sitting_surface(
            true,
            Duration::from_millis(500),
            Duration::from_millis(400),
        );

        assert_eq!(state.visual_state(), VisualState::SittingEnter);

        state.advance(Duration::from_millis(500));

        assert_eq!(state.visual_state(), VisualState::SittingLoop);
    }

    #[test]
    fn leaving_sitting_plays_exit_before_floating() {
        let mut state = NeraAnimationState::default();

        state.observe_sitting_surface(
            true,
            Duration::from_millis(1),
            Duration::from_millis(300),
        );
        state.advance(Duration::from_millis(1));
        state.start_persistent_action(TemporaryAction::Floating);

        // Floating is hidden while sitting.
        assert_eq!(state.visual_state(), VisualState::SittingLoop);

        state.observe_sitting_surface(
            false,
            Duration::from_millis(1),
            Duration::from_millis(300),
        );

        assert_eq!(state.visual_state(), VisualState::SittingExit);

        state.advance(Duration::from_millis(300));
        assert_eq!(state.visual_state(), VisualState::Floating);
    }

    #[test]
    fn timed_reaction_returns_to_idle() {
        let mut state = NeraAnimationState::default();
        state.request_one_shot(
            TemporaryAction::HeadStroke,
            Duration::from_millis(250),
        );

        state.advance(Duration::from_millis(250));

        assert_eq!(state.visual_state(), VisualState::Idle);
    }
    #[test]
    fn third_damage_hit_triggers_death_and_resets_combo() {
        let mut state = NeraAnimationState::default();
        let hit = Duration::from_millis(500);
        let death = Duration::from_secs(2);

        state.register_damage_hit(
            TemporaryAction::HitHead,
            hit,
            death,
        );
        assert_eq!(state.visual_state(), VisualState::HitHead);

        state.register_damage_hit(
            TemporaryAction::HitChest,
            hit,
            death,
        );
        assert_eq!(state.visual_state(), VisualState::HitChest);

        state.register_damage_hit(
            TemporaryAction::HitChest,
            hit,
            death,
        );
        assert_eq!(state.visual_state(), VisualState::Death);
        assert_eq!(state.damage_hit_count, 0);
        assert!(state.damage_combo_remaining.is_none());
    }

    #[test]
    fn damage_combo_expires() {
        let mut state = NeraAnimationState::default();
        let hit = Duration::from_millis(500);
        let death = Duration::from_secs(2);

        state.register_damage_hit(
            TemporaryAction::HitChest,
            hit,
            death,
        );
        assert_eq!(state.damage_hit_count, 1);

        state.advance(DAMAGE_COMBO_WINDOW);

        assert_eq!(state.damage_hit_count, 0);
        assert!(state.damage_combo_remaining.is_none());
    }

}
