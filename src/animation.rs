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
    rng::NeraRng,
};

#[cfg(target_os = "windows")]
use crate::action_menu::{
    AnimationMenuChoice,
    NeraActionMenuCommand,
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
const ALARM_BLEND: Duration = Duration::from_millis(120);

/// Phase 5.8 idle variation. A desktop pet that repeats one loop for hours
/// reads as a screensaver, so an idle Nera occasionally fidgets on her own.
///
/// The delay band is wide on purpose: a fixed interval is a pattern the user
/// notices within minutes, which is worse than no variation at all.
const IDLE_VARIATION_MIN_DELAY: Duration = Duration::from_secs(25);
const IDLE_VARIATION_MAX_DELAY: Duration = Duration::from_secs(75);

/// Longer than `BASE_BLEND`: a fidget eases in and out of Idle rather than
/// cutting, so it reads as a continuation of standing rather than a reaction.
const IDLE_VARIATION_BLEND: Duration = Duration::from_millis(220);

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlarmAnimationCommand {
    Start,
    Stop,
}

// -----------------------------------------------------------------------------
// Animation vocabulary
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum BasePose {
    #[default]
    Idle,
    Sitting,
}

/// Phase 5.8 idle micro-actions — short one-shot fidgets played on top of Idle.
///
/// Deliberately limited to fidgets. `Clapping.vrma`, `Goodbye.vrma` and
/// `Jump.vrma` are converted and unused too, but a desktop pet that randomly
/// applauds or waves goodbye while you are trying to work reads as noise
/// rather than life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum IdleMicroAction {
    LookAround,
    Relax,
    Sleepy,
    Blush,
}

impl IdleMicroAction {
    /// Stable order, so the scheduler can walk the candidates without
    /// allocating and so a seeded `NeraRng` replays identically in tests.
    const ALL: [IdleMicroAction; 4] = [
        IdleMicroAction::LookAround,
        IdleMicroAction::Relax,
        IdleMicroAction::Sleepy,
        IdleMicroAction::Blush,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            IdleMicroAction::LookAround => "Idle/LookAround",
            IdleMicroAction::Relax => "Idle/Relax",
            IdleMicroAction::Sleepy => "Idle/Sleepy",
            IdleMicroAction::Blush => "Idle/Blush",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TemporaryAction {
    Floating,
    Falling,
    Landing,
    HitHead,
    HitChest,
    HeadStroke,
    Death,
    AlarmWave,
    AlarmJump,
    #[cfg(target_os = "windows")]
    MenuAnimation(AnimationMenuChoice),
    /// Phase 5.5: persistent while the right-click menu is open.
    Praying,
    /// Phase 5.8: scheduled fidget, always outranked by real interaction.
    IdleMicro(IdleMicroAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SupportTransition {
    SittingEnter,
    SittingExit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
enum AnimationPriority {
    /// Phase 5.8 idle fidgets. Below every real interaction layer, so a
    /// micro-action can never delay a reaction, a drag, physics, or the alarm.
    IdleVariation = 40,
    Reaction = 60,
    Menu = 70,
    Floating = 80,
    Physics = 85,
    Critical = 90,
    Alarm = 100,
    AlarmIntro = 110,
}

impl TemporaryAction {
    fn priority(self) -> AnimationPriority {
        match self {
            Self::HitHead | Self::HitChest | Self::HeadStroke => {
                AnimationPriority::Reaction
            }
            Self::Praying => AnimationPriority::Menu,
            #[cfg(target_os = "windows")]
            Self::MenuAnimation(_) => AnimationPriority::Reaction,
            Self::Floating => AnimationPriority::Floating,
            Self::Falling | Self::Landing => AnimationPriority::Physics,
            Self::Death => AnimationPriority::Critical,
            Self::AlarmWave => AnimationPriority::Alarm,
            Self::AlarmJump => AnimationPriority::AlarmIntro,
            Self::IdleMicro(_) => AnimationPriority::IdleVariation,
        }
    }

    fn is_reaction(self) -> bool {
        self.priority() == AnimationPriority::Reaction
    }

    fn is_critical(self) -> bool {
        self.priority() == AnimationPriority::Critical
    }

    /// True for the Phase 5.8 fidget layer, which holds at most one member at a
    /// time: two overlapping micro-actions would both sit at priority 40 and
    /// `highest_action` would pick between them arbitrarily.
    fn is_idle_variation(self) -> bool {
        matches!(self, Self::IdleMicro(_))
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
pub(crate) enum VisualState {
    Idle,
    SittingLoop,
    SittingEnter,
    SittingExit,
    Floating,
    Falling,
    Landing,
    HitHead,
    HitChest,
    HeadStroke,
    Death,
    AlarmWave,
    AlarmJump,
    #[cfg(target_os = "windows")]
    MenuAnimation(AnimationMenuChoice),
    Praying,
    /// Phase 5.8 scheduled fidget.
    IdleMicro(IdleMicroAction),
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
        if pose == BasePose::Sitting {
            // Idle fidgets are standing clips. If Nera becomes seated while one
            // is active, discard it instead of hiding it under Sitting and
            // letting the tail resume if she stands again quickly.
            let before = self.active_actions.len();
            self.active_actions
                .retain(|active| !active.action.is_idle_variation());

            if before != self.active_actions.len() {
                info!("Nera idle variation cancelled by Sitting");
            }
        }

        if self.base_pose != pose {
            info!("Nera base pose: {:?} -> {:?}", self.base_pose, pose);
            self.base_pose = pose;
        }
    }

    /// Phase 5.55 Presentation Mode temporarily leaves the desktop support
    /// world. Do not play SittingExit: Attention/Alarm owns the avatar now.
    pub(crate) fn suspend_support_for_attention(&mut self) {
        self.support_transition = None;
        self.was_on_sitting_surface = false;
        self.set_base_pose(BasePose::Idle);

        // Attention owns the avatar completely. Keep only the alarm actions
        // that are part of the Attention presentation itself; every desktop
        // interaction action (including persistent Floating and any unfinished
        // idle/reaction tail) is stale once control crosses this boundary.
        let before = self.active_actions.len();
        self.active_actions.retain(|active| {
            matches!(
                active.action,
                TemporaryAction::AlarmWave | TemporaryAction::AlarmJump
            )
        });
        let cleared = before - self.active_actions.len();

        // A pre-alarm 1/3 or 2/3 damage combo must not survive an Attention
        // round-trip and turn an unrelated later poke into Death.
        self.reset_damage_combo();
        self.bump_revision();

        info!(
            "Nera support suspended for Attention | cleared {} desktop action(s)",
            cleared,
        );
    }

    /// Restore the desktop base pose without replaying SittingEnter.
    ///
    /// The attachment geometry itself is restored by PresentationState. This
    /// method only re-synchronizes the animation state with that snapshot.
    pub(crate) fn restore_support_after_attention(
        &mut self,
        sitting: bool,
    ) {
        self.support_transition = None;
        self.was_on_sitting_surface = sitting;

        self.set_base_pose(if sitting {
            BasePose::Sitting
        } else {
            BasePose::Idle
        });

        self.bump_revision();

        info!(
            "Nera support restored after Attention -> {:?}",
            self.base_pose
        );
    }

    fn start_persistent_action(&mut self, action: TemporaryAction) {
        if self
            .active_actions
            .iter()
            .any(|active| active.action == action)
        {
            return;
        }

        // Same reasoning as `request_one_shot`: every persistent action
        // outranks a fidget, so drop it instead of letting its tail resume
        // when the persistent state ends.
        self.active_actions
            .retain(|active| !active.action.is_idle_variation());

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

        // A fidget never survives anything else being requested. It is the
        // lowest layer, so resuming its tail once a reaction ends would look
        // like a glitch rather than a continuation.
        if !action.is_idle_variation() {
            self.active_actions
                .retain(|active| !active.action.is_idle_variation());
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
        } else if action.is_idle_variation() {
            // One fidget at a time: they all sit at `IdleVariation`, so
            // overlapping members would leave `highest_action` picking between
            // two equal priorities arbitrarily.
            self.active_actions
                .retain(|active| !active.action.is_idle_variation());
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

    /// Phase 5.8 idle variation entry point.
    ///
    /// Returns `false` when Nera is not actually idle, so the scheduler can
    /// distinguish a rejected fidget from an accepted one and keep its cooldown
    /// instead of burning it on a frame where nothing could play.
    pub(crate) fn request_idle_variation(
        &mut self,
        action: IdleMicroAction,
        duration: Duration,
    ) -> bool {
        if self.visual_state() != VisualState::Idle {
            return false;
        }

        self.request_one_shot(
            TemporaryAction::IdleMicro(action),
            duration,
        );

        true
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
            .filter(|active| !self.suppressed_by_sitting(active.action))
            .max_by_key(|active| active.action.priority())
            .map(|active| active.action)
    }

    /// Actions that must not play while Nera is attached to a seat.
    ///
    /// Both are standing clips. Playing one would visually pull her off the
    /// surface her Seat anchor was calibrated against, so they stay logically
    /// alive but never win the pose. A micro-action can only *start* while
    /// `visual_state()` is Idle, but she can be sat down mid-fidget.
    fn suppressed_by_sitting(&self, action: TemporaryAction) -> bool {
        if self.base_pose != BasePose::Sitting {
            return false;
        }

        matches!(
            action,
            TemporaryAction::Floating | TemporaryAction::IdleMicro(_)
        )
    }

    /// The single authoritative answer to "what is Nera doing right now".
    ///
    /// Phase 5.8 derives the facial expression from this rather than from raw
    /// interaction messages, so the face and the body can never disagree.
    pub(crate) fn visual_state(&self) -> VisualState {
        if let Some(transition) = self.support_transition {
            return match transition.kind {
                SupportTransition::SittingEnter => VisualState::SittingEnter,
                SupportTransition::SittingExit => VisualState::SittingExit,
            };
        }

        if let Some(action) = self.highest_action() {
            return match action {
                TemporaryAction::Floating => VisualState::Floating,
                TemporaryAction::Falling => VisualState::Falling,
                TemporaryAction::Landing => VisualState::Landing,
                TemporaryAction::HitHead => VisualState::HitHead,
                TemporaryAction::HitChest => VisualState::HitChest,
                TemporaryAction::HeadStroke => VisualState::HeadStroke,
                TemporaryAction::Death => VisualState::Death,
                TemporaryAction::AlarmWave => VisualState::AlarmWave,
                TemporaryAction::AlarmJump => VisualState::AlarmJump,
                #[cfg(target_os = "windows")]
                TemporaryAction::MenuAnimation(choice) => {
                    VisualState::MenuAnimation(choice)
                }
                TemporaryAction::Praying => VisualState::Praying,
                TemporaryAction::IdleMicro(action) => {
                    VisualState::IdleMicro(action)
                }
            };
        }

        match self.base_pose {
            BasePose::Idle => VisualState::Idle,
            BasePose::Sitting => VisualState::SittingLoop,
        }
    }

    /// Phase 5.7 persistent freefall layer. It sits above drag Floating but
    /// below critical/alarm actions, so existing high-priority behavior wins.
    pub(crate) fn set_physics_falling(&mut self, falling: bool) {
        let active = self
            .active_actions
            .iter()
            .any(|active| active.action == TemporaryAction::Falling);

        if falling == active {
            return;
        }

        if falling {
            // A new fall cancels any stale landing one-shot.
            self.active_actions
                .retain(|active| active.action != TemporaryAction::Landing);
            self.start_persistent_action(TemporaryAction::Falling);
        } else {
            self.finish_action(TemporaryAction::Falling);
        }
    }

    /// Cancel the complete Phase 5.7 motion layer when another native owner
    /// takes over (dragging or Phase 4 sitting). This keeps input responsive
    /// even if the landing one-shot had not finished yet.
    pub(crate) fn cancel_physics_motion(&mut self) {
        let before = self.active_actions.len();
        self.active_actions.retain(|active| {
            !matches!(
                active.action,
                TemporaryAction::Falling | TemporaryAction::Landing
            )
        });

        if before != self.active_actions.len() {
            self.bump_revision();
            info!("Nera physics animation cancelled");
        }
    }

    /// Switch Falling -> Falling_To_Landing using the real VRMA duration.
    pub(crate) fn land_from_fall(&mut self, landing_duration: Duration) {
        self.finish_action(TemporaryAction::Falling);
        self.active_actions
            .retain(|active| active.action != TemporaryAction::Landing);

        if landing_duration.is_zero() {
            warn!("Falling_To_Landing duration unavailable; landing directly to base pose");
            self.bump_revision();
            return;
        }

        self.request_one_shot(TemporaryAction::Landing, landing_duration);
    }

    pub(crate) fn death_active(&self) -> bool {
        self.visual_state() == VisualState::Death
    }

    /// Head-dominant clips must temporarily own the head bones. bevy_vrm1
    /// applies LookAt after animation, so leaving LookAt enabled would partly
    /// overwrite ShakeHead/HitHead every frame.
    pub(crate) fn suppresses_look_at(&self) -> bool {
        match self.visual_state() {
            VisualState::Falling
            | VisualState::Landing
            | VisualState::HitHead
            | VisualState::HitChest
            | VisualState::HeadStroke
            | VisualState::Death
            | VisualState::AlarmWave
            | VisualState::AlarmJump
            | VisualState::Praying
            | VisualState::SittingEnter
            | VisualState::SittingExit => true,

            #[cfg(target_os = "windows")]
            VisualState::MenuAnimation(_) => true,

            _ => false,
        }
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
    falling: Option<ClipSlot>,
    falling_to_landing: Option<ClipSlot>,
    death: Option<ClipSlot>,
    alarm_wave: Option<ClipSlot>,
    alarm_jump: Option<ClipSlot>,
    hit_head: Option<ClipSlot>,
    hit_chest: Option<ClipSlot>,
    head_stroke: Option<ClipSlot>,
    praying: Option<ClipSlot>,
    sitting_enter: Option<ClipSlot>,
    sitting_loop: Option<ClipSlot>,
    sitting_exit: Option<ClipSlot>,

    // Phase 5.8 idle micro-actions. Loaded asynchronously like everything
    // else, so the scheduler must treat a missing slot as "not available yet"
    // rather than as an error.
    idle_look_around: Option<ClipSlot>,
    idle_relax: Option<ClipSlot>,
    idle_sleepy: Option<ClipSlot>,
    idle_blush: Option<ClipSlot>,

    #[cfg(target_os = "windows")]
    wave: Option<ClipSlot>,
    #[cfg(target_os = "windows")]
    thinking: Option<ClipSlot>,
    #[cfg(target_os = "windows")]
    angry: Option<ClipSlot>,
    #[cfg(target_os = "windows")]
    sad: Option<ClipSlot>,
    #[cfg(target_os = "windows")]
    surprised: Option<ClipSlot>,
}

impl NeraAnimationClips {
    pub(crate) fn falling_to_landing_duration(&self) -> Duration {
        self.falling_to_landing
            .map(|clip| clip.duration)
            .unwrap_or(Duration::ZERO)
    }

    fn idle_micro(&self, action: IdleMicroAction) -> Option<ClipSlot> {
        match action {
            IdleMicroAction::LookAround => self.idle_look_around,
            IdleMicroAction::Relax => self.idle_relax,
            IdleMicroAction::Sleepy => self.idle_sleepy,
            IdleMicroAction::Blush => self.idle_blush,
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct NeraAnimationPlayback {
    current_clip: Option<Entity>,
    visual: Option<VisualState>,
    revision: u64,
}

/// Alarm may trigger before asynchronous VRMA loading has finished.
/// Keep the request pending instead of entering an unavailable VisualState.
#[derive(Resource, Default)]
struct AlarmAnimationRuntime {
    requested: bool,
    started: bool,
    waiting_logged: bool,
}

/// Phase 5.8 idle variation scheduler.
///
/// Counts down only while Nera is genuinely idle, so a long interaction
/// postpones variety rather than spending it on frames where nothing could
/// play. Shaped like `AlarmAnimationRuntime`: a small runtime resource beside
/// the authoritative `NeraAnimationState`, not a second state machine.
#[derive(Resource)]
pub(crate) struct IdleVariationRuntime {
    cooldown: Duration,
    /// Avoids the same fidget twice in a row, which is the pattern users
    /// notice fastest.
    last: Option<IdleMicroAction>,
    rng: NeraRng,
}

impl Default for IdleVariationRuntime {
    fn default() -> Self {
        let mut rng = NeraRng::from_entropy();
        let cooldown = random_idle_delay(&mut rng);

        Self {
            cooldown,
            last: None,
            rng,
        }
    }
}

impl IdleVariationRuntime {
    fn tick(
        &mut self,
        delta: Duration,
        state: &mut NeraAnimationState,
        clips: &NeraAnimationClips,
    ) {
        if state.visual_state() != VisualState::Idle {
            return;
        }

        self.cooldown = self.cooldown.saturating_sub(delta);

        if !self.cooldown.is_zero() {
            return;
        }

        let Some((action, duration)) =
            pick_idle_micro(clips, self.last, &mut self.rng)
        else {
            // No micro-action has finished loading yet. Retry next frame
            // instead of arming a full 25-75s delay for an asset that may
            // arrive a moment from now.
            return;
        };

        if !state.request_idle_variation(action, duration) {
            return;
        }

        self.last = Some(action);
        self.cooldown = self.next_delay();
    }

    fn next_delay(&mut self) -> Duration {
        random_idle_delay(&mut self.rng)
    }
}

fn random_idle_delay(rng: &mut NeraRng) -> Duration {
    Duration::from_secs_f32(rng.range_f32(
        IDLE_VARIATION_MIN_DELAY.as_secs_f32(),
        IDLE_VARIATION_MAX_DELAY.as_secs_f32(),
    ))
}

/// Choose a loaded micro-action, starting from a random offset.
///
/// Walking the list rather than re-rolling means a clip that has not finished
/// loading is skipped instead of stalling the scheduler, and no allocation is
/// needed. `avoid` demotes the previous fidget to a fallback, so a single
/// loaded clip still plays rather than the scheduler giving up.
fn pick_idle_micro(
    clips: &NeraAnimationClips,
    avoid: Option<IdleMicroAction>,
    rng: &mut NeraRng,
) -> Option<(IdleMicroAction, Duration)> {
    let all = IdleMicroAction::ALL;

    let start = rng.pick_index(all.len())?;
    let mut fallback: Option<(IdleMicroAction, Duration)> = None;

    for offset in 0..all.len() {
        let candidate = all[(start + offset) % all.len()];

        let Some(clip) = clips.idle_micro(candidate) else {
            continue;
        };

        // A zero duration means the VRMA reported no length, which
        // `request_one_shot` would reject anyway.
        if clip.duration.is_zero() {
            continue;
        }

        let found = (candidate, clip.duration);

        if Some(candidate) == avoid {
            fallback.get_or_insert(found);
            continue;
        }

        return Some(found);
    }

    fallback
}

pub(crate) fn schedule_idle_variation(
    time: Res<Time>,
    clips: Res<NeraAnimationClips>,
    mut state: ResMut<NeraAnimationState>,
    mut runtime: ResMut<IdleVariationRuntime>,
) {
    runtime.tick(time.delta(), &mut state, &clips);
}

pub(crate) struct NeraAnimationPlugin;

impl Plugin for NeraAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NeraAnimationState>()
            .init_resource::<NeraAnimationClips>()
            .init_resource::<NeraAnimationPlayback>()
            .init_resource::<AlarmAnimationRuntime>()
            .init_resource::<IdleVariationRuntime>()
            .add_message::<AlarmAnimationCommand>()
            .add_systems(
                Update,
                sync_alarm_animation,
            );

        #[cfg(target_os = "windows")]
        app.add_systems(
            Update,
            sync_action_menu_animation,
        );
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
register_clip!(register_falling_animation, falling, "Falling");
register_clip!(
    register_falling_to_landing_animation,
    falling_to_landing,
    "FallingToLanding"
);
register_clip!(register_death_animation, death, "Death01");
register_clip!(register_alarm_wave_animation, alarm_wave, "Alarm/Wave");
register_clip!(register_alarm_jump_animation, alarm_jump, "Alarm/JumpHappy");
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

// Phase 5.8 idle micro-actions.
register_clip!(
    register_idle_look_around_animation,
    idle_look_around,
    "Idle/LookAround"
);
register_clip!(
    register_idle_relax_animation,
    idle_relax,
    "Idle/Relax"
);
register_clip!(
    register_idle_sleepy_animation,
    idle_sleepy,
    "Idle/Sleepy"
);
register_clip!(register_idle_blush_animation, idle_blush, "Idle/Blush");

#[cfg(target_os = "windows")]
register_clip!(register_wave_animation, wave, "Menu/Wave");
#[cfg(target_os = "windows")]
register_clip!(register_thinking_animation, thinking, "Menu/Thinking");
#[cfg(target_os = "windows")]
register_clip!(register_angry_animation, angry, "Menu/Angry");
#[cfg(target_os = "windows")]
register_clip!(register_sad_animation, sad, "Menu/Sad");
#[cfg(target_os = "windows")]
register_clip!(
    register_surprised_animation,
    surprised,
    "Menu/Surprised"
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

pub(crate) fn sync_alarm_animation(
    mut commands: MessageReader<AlarmAnimationCommand>,
    clips: Res<NeraAnimationClips>,
    mut state: ResMut<NeraAnimationState>,
    mut runtime: ResMut<AlarmAnimationRuntime>,
) {
    for command in commands.read() {
        match command {
            AlarmAnimationCommand::Start => {
                runtime.requested = true;
                runtime.started = false;
                runtime.waiting_logged = false;

                // Clear any stale alarm action from an earlier interrupted
                // presentation before starting a new sequence.
                state.finish_action(TemporaryAction::AlarmJump);
                state.finish_action(TemporaryAction::AlarmWave);
            }

            AlarmAnimationCommand::Stop => {
                runtime.requested = false;
                runtime.started = false;
                runtime.waiting_logged = false;

                state.finish_action(TemporaryAction::AlarmJump);
                state.finish_action(TemporaryAction::AlarmWave);
                state.bump_revision();
            }
        }
    }

    if !runtime.requested || runtime.started {
        return;
    }

    let (Some(wave), Some(jump)) =
        (clips.alarm_wave, clips.alarm_jump)
    else {
        if !runtime.waiting_logged {
            info!(
                "Nera alarm animation waiting for Wave + JumpHappy VRMAs..."
            );
            runtime.waiting_logged = true;
        }
        return;
    };

    // Persistent Wave sits underneath the one-shot intro.
    state.start_persistent_action(TemporaryAction::AlarmWave);

    // Jump has higher priority. When its *real loaded clip duration* expires,
    // visual_state() naturally falls back to the persistent Wave.
    state.request_one_shot(
        TemporaryAction::AlarmJump,
        jump.duration,
    );

    runtime.started = true;
    runtime.waiting_logged = false;

    info!(
        "Nera alarm animation started | jump={:?} wave={:?}",
        jump.duration,
        wave.duration,
    );
}

#[cfg(target_os = "windows")]
pub(crate) fn sync_action_menu_animation(
    mut commands: MessageReader<NeraActionMenuCommand>,
    clips: Res<NeraAnimationClips>,
    mut state: ResMut<NeraAnimationState>,
) {
    for command in commands.read() {
        let NeraActionMenuCommand::Animation(choice) = *command else {
            // Alarm is consumed by Phase 5.6. Settings gets its own UI later.
            continue;
        };

        if choice == AnimationMenuChoice::ResetBasePose {
            // Do not fake a Sitting pose in mid-air. Reset means: return to
            // the correct environment-owned base pose (Idle or real Sitting).
            state.active_actions.retain(|active| {
                !active.action.is_reaction()
            });
            state.bump_revision();

            info!(
                "Nera menu animation reset -> base {:?}",
                state.base_pose()
            );
            continue;
        }

        let clip = match choice {
            AnimationMenuChoice::Wave => clips.wave,
            AnimationMenuChoice::Thinking => clips.thinking,
            AnimationMenuChoice::Angry => clips.angry,
            AnimationMenuChoice::Sad => clips.sad,
            AnimationMenuChoice::Surprised => clips.surprised,
            AnimationMenuChoice::ResetBasePose => None,
        };

        let Some(clip) = clip else {
            warn!(
                "Nera menu animation {:?} requested before its VRMA loaded",
                choice
            );
            continue;
        };

        state.request_one_shot(
            TemporaryAction::MenuAnimation(choice),
            clip.duration,
        );
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
        // Phase 5.7 uses the existing Floating pose while airborne. The
        // dedicated Falling clip is intentionally not used because its body
        // pose does not match the desktop-pet fall presentation.
        VisualState::Falling => {
            (clips.floating, RepeatAnimation::Forever, "Floating/Fall")
        }
        VisualState::Landing => {
            (
                clips.falling_to_landing,
                RepeatAnimation::Count(1),
                "FallingToLanding",
            )
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
        VisualState::AlarmJump => {
            (
                clips.alarm_jump,
                RepeatAnimation::Count(1),
                "Alarm/JumpHappy",
            )
        }
        VisualState::AlarmWave => {
            (
                clips.alarm_wave,
                RepeatAnimation::Forever,
                "Alarm/Wave",
            )
        }
        #[cfg(target_os = "windows")]
        VisualState::MenuAnimation(choice) => {
            match choice {
                AnimationMenuChoice::Wave => {
                    (clips.wave, RepeatAnimation::Count(1), "Menu/Wave")
                }
                AnimationMenuChoice::Thinking => {
                    (
                        clips.thinking,
                        RepeatAnimation::Count(1),
                        "Menu/Thinking",
                    )
                }
                AnimationMenuChoice::Angry => {
                    (clips.angry, RepeatAnimation::Count(1), "Menu/Angry")
                }
                AnimationMenuChoice::Sad => {
                    (clips.sad, RepeatAnimation::Count(1), "Menu/Sad")
                }
                AnimationMenuChoice::Surprised => {
                    (
                        clips.surprised,
                        RepeatAnimation::Count(1),
                        "Menu/Surprised",
                    )
                }
                AnimationMenuChoice::ResetBasePose => {
                    (None, RepeatAnimation::Forever, "Menu/Reset")
                }
            }
        }
        VisualState::Praying => {
            (clips.praying, RepeatAnimation::Forever, "Praying")
        }
        VisualState::IdleMicro(action) => (
            clips.idle_micro(action),
            RepeatAnimation::Count(1),
            action.label(),
        ),
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
        VisualState::Falling | VisualState::Landing => SUPPORT_BLEND,
        VisualState::HitHead | VisualState::HitChest => REACTION_BLEND,
        VisualState::Death => DEATH_BLEND,
        VisualState::AlarmJump | VisualState::AlarmWave => ALARM_BLEND,
        #[cfg(target_os = "windows")]
        VisualState::MenuAnimation(_) => REACTION_BLEND,
        VisualState::SittingEnter | VisualState::SittingExit => SUPPORT_BLEND,
        VisualState::IdleMicro(_) => IDLE_VARIATION_BLEND,
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

    /// Phase 5.75 regression: an alarm that fires mid-drag never produces
    /// `NeraReleased { was_dragging: true }`, because `drag_nera_window` is
    /// gated off by `desktop_pet_active`. `Floating` is persistent, so it would
    /// survive the whole Attention round-trip at priority 80 and outrank every
    /// reaction on the way back.
    #[test]
    fn attention_suspend_drops_the_stale_floating_action() {
        let mut state = NeraAnimationState::default();
        state.start_persistent_action(TemporaryAction::Floating);
        assert_eq!(state.visual_state(), VisualState::Floating);

        state.suspend_support_for_attention();

        assert!(!state
            .active_actions
            .iter()
            .any(|active| active.action == TemporaryAction::Floating));
        assert_eq!(state.visual_state(), VisualState::Idle);
        assert_eq!(state.base_pose(), BasePose::Idle);

        // The point of the fix: reactions must win again after returning.
        state.restore_support_after_attention(false);
        state.request_one_shot(
            TemporaryAction::HitHead,
            Duration::from_secs(1),
        );
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

    #[cfg(target_os = "windows")]
    #[test]
    fn action_menu_praying_returns_to_base_when_closed() {
        let mut state = NeraAnimationState::default();

        state.set_praying_menu(true);
        assert_eq!(state.visual_state(), VisualState::Praying);

        state.set_praying_menu(false);
        assert_eq!(state.visual_state(), VisualState::Idle);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn menu_animation_is_a_timed_reaction() {
        let mut state = NeraAnimationState::default();

        state.request_one_shot(
            TemporaryAction::MenuAnimation(AnimationMenuChoice::Wave),
            Duration::from_millis(500),
        );

        assert_eq!(
            state.visual_state(),
            VisualState::MenuAnimation(AnimationMenuChoice::Wave)
        );

        state.advance(Duration::from_millis(500));

        assert_eq!(state.visual_state(), VisualState::Idle);
    }

    #[test]
    fn attention_suspends_sitting_without_exit_transition() {
        let mut state = NeraAnimationState::default();

        state.observe_sitting_surface(
            true,
            Duration::from_millis(300),
            Duration::from_millis(300),
        );

        state.suspend_support_for_attention();

        assert_eq!(state.base_pose(), BasePose::Idle);
        assert!(state.support_transition.is_none());
        assert!(!state.was_on_sitting_surface);
    }

    #[test]
    fn attention_restore_returns_directly_to_sitting_base() {
        let mut state = NeraAnimationState::default();

        state.suspend_support_for_attention();
        state.restore_support_after_attention(true);

        assert_eq!(state.base_pose(), BasePose::Sitting);
        assert!(state.support_transition.is_none());
        assert!(state.was_on_sitting_surface);
    }

    #[test]
    fn alarm_jump_overrides_wave_then_wave_remains() {
        let mut state = NeraAnimationState::default();

        state.start_persistent_action(TemporaryAction::AlarmWave);
        state.request_one_shot(
            TemporaryAction::AlarmJump,
            Duration::from_millis(500),
        );

        assert_eq!(state.visual_state(), VisualState::AlarmJump);

        state.advance(Duration::from_millis(500));

        assert_eq!(state.visual_state(), VisualState::AlarmWave);
    }

    #[test]
    fn stopping_alarm_returns_to_base_pose() {
        let mut state = NeraAnimationState::default();

        state.start_persistent_action(TemporaryAction::AlarmWave);
        state.request_one_shot(
            TemporaryAction::AlarmJump,
            Duration::from_millis(500),
        );

        state.finish_action(TemporaryAction::AlarmJump);
        state.finish_action(TemporaryAction::AlarmWave);

        assert_eq!(state.visual_state(), VisualState::Idle);
    }

    #[test]
    fn alarm_runtime_starts_pending_not_started() {
        let runtime = AlarmAnimationRuntime::default();

        assert!(!runtime.requested);
        assert!(!runtime.started);
        assert!(!runtime.waiting_logged);
    }

    // -------------------------------------------------------------------------
    // Phase 5.8 idle variation
    // -------------------------------------------------------------------------

    fn clip(seconds: u64) -> ClipSlot {
        ClipSlot {
            entity: Entity::PLACEHOLDER,
            duration: Duration::from_secs(seconds),
        }
    }

    fn clips_with_all_micro_actions() -> NeraAnimationClips {
        NeraAnimationClips {
            idle_look_around: Some(clip(3)),
            idle_relax: Some(clip(4)),
            idle_sleepy: Some(clip(5)),
            idle_blush: Some(clip(2)),
            ..Default::default()
        }
    }

    fn runtime_with(
        seed: u64,
        cooldown: Duration,
    ) -> IdleVariationRuntime {
        IdleVariationRuntime {
            cooldown,
            last: None,
            rng: NeraRng::seeded(seed),
        }
    }

    /// The whole point of putting fidgets at the bottom: nothing Nera does in
    /// response to the user may ever be delayed by variety.
    #[test]
    fn idle_variation_outranks_nothing() {
        let idle = AnimationPriority::IdleVariation;

        assert!(idle < AnimationPriority::Reaction);
        assert!(idle < AnimationPriority::Menu);
        assert!(idle < AnimationPriority::Floating);
        assert!(idle < AnimationPriority::Physics);
        assert!(idle < AnimationPriority::Critical);
        assert!(idle < AnimationPriority::Alarm);
        assert!(idle < AnimationPriority::AlarmIntro);
    }

    #[test]
    fn a_reaction_cancels_a_fidget_rather_than_letting_its_tail_resume() {
        let mut state = NeraAnimationState::default();

        assert!(state.request_idle_variation(
            IdleMicroAction::LookAround,
            Duration::from_secs(4),
        ));
        assert!(matches!(
            state.visual_state(),
            VisualState::IdleMicro(_)
        ));

        state.request_one_shot(
            TemporaryAction::HeadStroke,
            Duration::from_secs(2),
        );
        assert_eq!(state.visual_state(), VisualState::HeadStroke);

        // The fidget must be *gone*, not merely outranked: resuming its last
        // two seconds after a head pat would read as a glitch.
        state.advance(Duration::from_secs(3));
        assert_eq!(state.visual_state(), VisualState::Idle);
    }

    #[test]
    fn sitting_cancels_a_fidget_instead_of_resuming_its_tail_later() {
        let mut state = NeraAnimationState::default();

        assert!(state.request_idle_variation(
            IdleMicroAction::Relax,
            Duration::from_secs(4),
        ));

        // Micro-action clips are standing clips. Once Nera becomes seated, the
        // fidget is invalidated rather than merely hidden under the seat pose.
        state.set_base_pose(BasePose::Sitting);

        assert_eq!(state.visual_state(), VisualState::SittingLoop);
        assert!(!state
            .active_actions
            .iter()
            .any(|active| active.action.is_idle_variation()));

        // Standing again must not reveal the old fidget tail.
        state.set_base_pose(BasePose::Idle);
        assert_eq!(state.visual_state(), VisualState::Idle);
    }

    #[test]
    fn request_idle_variation_is_rejected_when_nera_is_not_idle() {
        let mut state = NeraAnimationState::default();
        state.start_persistent_action(TemporaryAction::Floating);

        assert!(!state.request_idle_variation(
            IdleMicroAction::Sleepy,
            Duration::from_secs(3),
        ));
        assert_eq!(state.visual_state(), VisualState::Floating);
    }

    #[test]
    fn scheduler_waits_out_its_cooldown_before_fidgeting() {
        let clips = clips_with_all_micro_actions();
        let mut state = NeraAnimationState::default();
        let mut runtime =
            runtime_with(5, Duration::from_secs(10));

        runtime.tick(Duration::from_millis(500), &mut state, &clips);

        assert_eq!(state.visual_state(), VisualState::Idle);
        assert!(runtime.cooldown < Duration::from_secs(10));

        for _ in 0..24 {
            runtime.tick(Duration::from_millis(500), &mut state, &clips);
        }

        assert!(matches!(
            state.visual_state(),
            VisualState::IdleMicro(_)
        ));
    }

    #[test]
    fn scheduler_does_not_count_down_while_nera_is_busy() {
        let clips = clips_with_all_micro_actions();
        let mut state = NeraAnimationState::default();
        state.start_persistent_action(TemporaryAction::Floating);

        let mut runtime =
            runtime_with(5, Duration::from_secs(10));

        for _ in 0..100 {
            runtime.tick(Duration::from_secs(1), &mut state, &clips);
        }

        // A long interaction postpones variety instead of spending it.
        assert_eq!(runtime.cooldown, Duration::from_secs(10));
        assert_eq!(state.visual_state(), VisualState::Floating);
    }

    #[test]
    fn firing_a_fidget_rearms_the_cooldown() {
        let clips = clips_with_all_micro_actions();
        let mut state = NeraAnimationState::default();
        let mut runtime = runtime_with(9, Duration::ZERO);

        runtime.tick(Duration::ZERO, &mut state, &clips);

        assert!(matches!(
            state.visual_state(),
            VisualState::IdleMicro(_)
        ));
        assert!(runtime.cooldown >= IDLE_VARIATION_MIN_DELAY);
        assert!(runtime.last.is_some());
    }

    /// The most noticeable failure mode of a fidget system is the same clip
    /// twice in a row, which is a pattern rather than variety.
    #[test]
    fn scheduler_avoids_repeating_the_same_fidget() {
        let clips = clips_with_all_micro_actions();
        let mut state = NeraAnimationState::default();
        let mut runtime = runtime_with(11, Duration::ZERO);

        let mut played = Vec::new();

        for _ in 0..12 {
            runtime.cooldown = Duration::ZERO;
            runtime.tick(Duration::ZERO, &mut state, &clips);

            let VisualState::IdleMicro(action) = state.visual_state()
            else {
                panic!(
                    "expected a fidget, got {:?}",
                    state.visual_state()
                );
            };
            played.push(action);

            // Let the clip run out so the scheduler may fire again.
            state.advance(Duration::from_secs(30));
            assert_eq!(state.visual_state(), VisualState::Idle);
        }

        assert!(
            played.windows(2).all(|pair| pair[0] != pair[1]),
            "repeated a fidget back to back: {played:?}",
        );
    }

    #[test]
    fn next_delay_stays_inside_the_variation_band() {
        let mut runtime = runtime_with(77, Duration::ZERO);

        for _ in 0..1000 {
            let delay = runtime.next_delay();

            assert!(delay >= IDLE_VARIATION_MIN_DELAY, "{delay:?}");
            assert!(delay <= IDLE_VARIATION_MAX_DELAY, "{delay:?}");
        }
    }

    #[test]
    fn pick_idle_micro_skips_clips_that_have_not_loaded() {
        let clips = NeraAnimationClips {
            idle_sleepy: Some(clip(5)),
            ..Default::default()
        };

        let mut rng = NeraRng::seeded(3);

        // Any random start offset must still land on the one usable clip.
        for _ in 0..50 {
            let picked = pick_idle_micro(&clips, None, &mut rng)
                .expect("one clip is loaded");

            assert_eq!(picked.0, IdleMicroAction::Sleepy);
            assert_eq!(picked.1, Duration::from_secs(5));
        }
    }

    #[test]
    fn pick_idle_micro_returns_none_when_nothing_is_usable() {
        let mut rng = NeraRng::seeded(3);

        assert!(pick_idle_micro(
            &NeraAnimationClips::default(),
            None,
            &mut rng,
        )
        .is_none());

        // A zero duration means the VRMA reported no length, which
        // `request_one_shot` would reject anyway.
        let zero_length = NeraAnimationClips {
            idle_relax: Some(clip(0)),
            ..Default::default()
        };

        assert!(
            pick_idle_micro(&zero_length, None, &mut rng).is_none()
        );
    }

    /// With only one clip loaded the anti-repeat rule must degrade to a
    /// fallback, never to silence.
    #[test]
    fn avoid_falls_back_rather_than_giving_up() {
        let clips = NeraAnimationClips {
            idle_blush: Some(clip(2)),
            ..Default::default()
        };

        let mut rng = NeraRng::seeded(3);

        let picked = pick_idle_micro(
            &clips,
            Some(IdleMicroAction::Blush),
            &mut rng,
        );

        assert_eq!(
            picked.map(|(action, _)| action),
            Some(IdleMicroAction::Blush),
        );
    }

    /// VRMAs load asynchronously, so at startup there is a window where the
    /// scheduler is ready but no clip is. It must wait rather than arm a full
    /// delay for an asset that is about to arrive.
    #[test]
    fn scheduler_keeps_waiting_while_clips_are_still_loading() {
        let clips = NeraAnimationClips::default();
        let mut state = NeraAnimationState::default();
        let mut runtime = runtime_with(13, Duration::ZERO);

        for _ in 0..100 {
            runtime.tick(Duration::from_secs(1), &mut state, &clips);
        }

        assert_eq!(state.visual_state(), VisualState::Idle);
        assert!(runtime.cooldown.is_zero());
        assert!(runtime.last.is_none());
    }

    #[test]
    fn fidget_blend_is_longer_than_the_base_blend() {
        // A fidget eases in and out of Idle; cutting would read as a twitch.
        assert!(IDLE_VARIATION_BLEND > BASE_BLEND);
        assert_eq!(
            transition_for_visual(VisualState::IdleMicro(
                IdleMicroAction::LookAround
            )),
            IDLE_VARIATION_BLEND,
        );
    }

    #[test]
    fn every_micro_action_has_a_clip_slot_and_a_label() {
        let clips = clips_with_all_micro_actions();

        for action in IdleMicroAction::ALL {
            assert!(
                clips.idle_micro(action).is_some(),
                "{action:?} has no clip slot",
            );
            assert!(!action.label().is_empty());
        }

        // The clip lookup must be a bijection over the candidate list, or the
        // scheduler would silently play the wrong animation.
        assert_eq!(IdleMicroAction::ALL.len(), 4);
    }

}
