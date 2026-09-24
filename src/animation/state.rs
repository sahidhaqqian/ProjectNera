//! Animation vocabulary, request priority, and authoritative runtime state.
use std::time::Duration;

use bevy::prelude::*;

#[cfg(target_os = "windows")]
use crate::action_menu::AnimationMenuChoice;

/// Three deliberate head hits inside this window trigger Death01.
/// The combo expires naturally so occasional clicks do not accumulate forever.
pub(super) const DAMAGE_COMBO_WINDOW: Duration = Duration::from_millis(2500);

/// Short cross-fades hide one-frame bind/T-pose flashes that some converted
/// VRMA clips contain at their first sample.
pub(super) const BASE_BLEND: Duration = Duration::from_millis(100);
pub(super) const REACTION_BLEND: Duration = Duration::from_millis(140);
pub(super) const HEAD_STROKE_BLEND: Duration = Duration::from_millis(180);
pub(super) const SUPPORT_BLEND: Duration = Duration::from_millis(100);
pub(super) const DEATH_BLEND: Duration = Duration::from_millis(160);
pub(super) const ALARM_BLEND: Duration = Duration::from_millis(120);

/// Phase 5.8 idle variation. A desktop pet that repeats one loop for hours
/// reads as a screensaver, so an idle Nera occasionally fidgets on her own.
///
/// The delay band is wide on purpose: a fixed interval is a pattern the user
/// notices within minutes, which is worse than no variation at all.
pub(super) const IDLE_VARIATION_MIN_DELAY: Duration = Duration::from_secs(25);
pub(super) const IDLE_VARIATION_MAX_DELAY: Duration = Duration::from_secs(75);

/// Longer than `BASE_BLEND`: a fidget eases in and out of Idle rather than
/// cutting, so it reads as a continuation of standing rather than a reaction.
pub(super) const IDLE_VARIATION_BLEND: Duration = Duration::from_millis(220);

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
    pub(super) const ALL: [IdleMicroAction; 4] = [
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
    LibraryAnimation(Entity),
    /// Phase 5.5: persistent while the right-click menu is open.
    Praying,
    /// Phase 5.8: scheduled fidget, always outranked by real interaction.
    IdleMicro(IdleMicroAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum SupportTransition {
    SittingEnter,
    SittingExit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(super) enum AnimationPriority {
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
            Self::HitHead | Self::HitChest | Self::HeadStroke => AnimationPriority::Reaction,
            Self::Praying => AnimationPriority::Menu,
            Self::LibraryAnimation(_) => AnimationPriority::Reaction,
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

    pub(super) fn is_reaction(self) -> bool {
        self.priority() == AnimationPriority::Reaction
    }

    fn is_critical(self) -> bool {
        self.priority() == AnimationPriority::Critical
    }

    /// True for the Phase 5.8 fidget layer, which holds at most one member at a
    /// time: two overlapping micro-actions would both sit at priority 40 and
    /// `highest_action` would pick between them arbitrarily.
    pub(super) fn is_idle_variation(self) -> bool {
        matches!(self, Self::IdleMicro(_))
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ActiveAction {
    pub(super) action: TemporaryAction,
    /// `None` means event-driven/persistent. Floating and future Praying use
    /// this. One-shot reactions use the actual VRMA duration.
    pub(super) remaining: Option<Duration>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TimedSupportTransition {
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
    LibraryAnimation(Entity),
    Praying,
    /// Phase 5.8 scheduled fidget.
    IdleMicro(IdleMicroAction),
}

// -----------------------------------------------------------------------------
// Runtime state
// -----------------------------------------------------------------------------

#[derive(Resource, Debug)]
pub(crate) struct NeraAnimationState {
    pub(super) base_pose: BasePose,
    pub(super) active_actions: Vec<ActiveAction>,
    pub(super) support_transition: Option<TimedSupportTransition>,
    pub(super) was_on_sitting_surface: bool,

    pub(super) damage_hit_count: u8,
    pub(super) damage_combo_remaining: Option<Duration>,

    /// Changes whenever an action/transition is explicitly requested. This
    /// lets repeated HitHead clicks replay the same VRMA from frame zero.
    pub(super) revision: u64,
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

    pub(super) fn bump_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    pub(super) fn set_base_pose(&mut self, pose: BasePose) {
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

        // `Floating` is persistent and is only ever released by
        // `NeraReleased { was_dragging: true }`. An alarm that fires mid-drag
        // gates `drag_nera_window` off before that message exists, so without
        // this Nera returns from Attention stuck in the Floating pose at
        // priority 80 — above every reaction.
        self.finish_action(TemporaryAction::Floating);

        self.bump_revision();

        info!("Nera support suspended for Attention");
    }

    /// Restore the desktop base pose without replaying SittingEnter.
    ///
    /// The attachment geometry itself is restored by PresentationState. This
    /// method only re-synchronizes the animation state with that snapshot.
    pub(crate) fn restore_support_after_attention(&mut self, sitting: bool) {
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

    pub(super) fn start_persistent_action(&mut self, action: TemporaryAction) {
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
        info!("Nera action +{:?} priority={:?}", action, action.priority());
    }

    pub(super) fn request_one_shot(&mut self, action: TemporaryAction, duration: Duration) {
        if duration.is_zero() {
            warn!("Nera ignored {:?}: VRMA duration unavailable/zero", action);
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
            self.active_actions
                .retain(|active| !active.action.is_reaction() && !active.action.is_critical());
        } else if action.is_idle_variation() {
            // One fidget at a time: they all sit at `IdleVariation`, so
            // overlapping members would leave `highest_action` picking between
            // two equal priorities arbitrarily.
            self.active_actions
                .retain(|active| !active.action.is_idle_variation());
        } else {
            self.active_actions.retain(|active| active.action != action);
        }

        self.active_actions.push(ActiveAction {
            action,
            remaining: Some(duration),
        });
        self.bump_revision();

        info!("Nera reaction -> {:?} duration={:?}", action, duration);
    }

    pub(super) fn reset_damage_combo(&mut self) {
        self.damage_hit_count = 0;
        self.damage_combo_remaining = None;
    }

    /// Registers one damage reaction from either Head or Torso.
    ///
    /// Any three valid damage hits inside `DAMAGE_COMBO_WINDOW` trigger
    /// Death01. This intentionally allows Head+Chest mixed combos.
    pub(super) fn register_damage_hit(
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
                self.request_one_shot(TemporaryAction::Death, death_duration);
            } else {
                warn!(
                    "Death01 duration unavailable; falling back to {:?}",
                    reaction
                );
                self.request_one_shot(reaction, reaction_duration);
            }
            return;
        }

        info!(
            "Nera damage combo {}/3 via {:?}",
            self.damage_hit_count, reaction
        );

        self.request_one_shot(reaction, reaction_duration);
    }

    pub(super) fn finish_action(&mut self, action: TemporaryAction) {
        let before = self.active_actions.len();
        self.active_actions.retain(|active| active.action != action);

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

        self.request_one_shot(TemporaryAction::IdleMicro(action), duration);

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

    fn begin_support_transition(&mut self, kind: SupportTransition, duration: Duration) {
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

    pub(super) fn observe_sitting_surface(
        &mut self,
        sitting_surface: bool,
        enter_duration: Duration,
        _exit_duration: Duration,
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
            self.begin_support_transition(SupportTransition::SittingEnter, enter_duration);
        } else {
            // Desktop detach is immediate. Waiting for SittingExit before drag
            // caused cursor/window divergence, and lifecycle detaches such as
            // maximize/minimize should fall immediately once support vanishes.
            self.set_base_pose(BasePose::Idle);

            if self.support_transition.take().is_some() {
                self.bump_revision();
            }
        }
    }

    pub(super) fn advance(&mut self, delta: Duration) {
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
                info!("Nera damage combo expired at {}/3", self.damage_hit_count);
            }
            self.reset_damage_combo();
        }

        let transition_finished = match self.support_transition.as_mut() {
            Some(transition) if transition.remaining <= delta => Some(transition.kind),
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
                TemporaryAction::MenuAnimation(choice) => VisualState::MenuAnimation(choice),
                TemporaryAction::Praying => VisualState::Praying,
                TemporaryAction::LibraryAnimation(entity) => VisualState::LibraryAnimation(entity),
                TemporaryAction::IdleMicro(action) => VisualState::IdleMicro(action),
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
    ///
    /// Physics can revalidate the same support for more than one frame around
    /// a native-window edge. Never restart an already-playing landing clip: one
    /// physical fall owns exactly one Falling_To_Landing transition.
    pub(crate) fn land_from_fall(&mut self, landing_duration: Duration) {
        self.finish_action(TemporaryAction::Falling);

        if self
            .active_actions
            .iter()
            .any(|active| active.action == TemporaryAction::Landing)
        {
            return;
        }

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

            VisualState::LibraryAnimation(_) => true,

            #[cfg(target_os = "windows")]
            VisualState::MenuAnimation(_) => true,

            _ => false,
        }
    }
}
