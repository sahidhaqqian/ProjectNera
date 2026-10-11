//! Accepted Hand/Arm reaction selection. Animation state remains the pose owner.
use std::time::Duration;

use bevy::{ecs::system::SystemParam, prelude::*};

use super::{NeraAnimationClips, NeraAnimationState, TouchReactionClip};
use crate::{interaction::BodyPart, rng::NeraRng};

pub(crate) mod seated;

pub(crate) const HAND_STREAK_TIMEOUT: Duration = Duration::from_millis(4500);
pub(crate) const TOUCH_REACTION_COOLDOWN: Duration = Duration::from_millis(650);

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct TouchReactionAccepted {
    pub(crate) clip: TouchReactionClip,
    /// Zero for Arm; 1..=7 for the accepted semantic Hand sequence.
    pub(crate) hand_step: u8,
    pub(crate) angry_mark: bool,
}

#[derive(Resource, Default)]
pub(crate) struct TouchReactionRuntime {
    hand_count: u8,
    last_hand_at: Option<Duration>,
    hand_cooldown_until: Duration,
    arm_cooldown_until: Duration,
    last_normal_hand: Option<TouchReactionClip>,
    last_arm: Option<TouchReactionClip>,
    rng: NeraRng,
}

impl TouchReactionRuntime {
    pub(crate) fn hand_streak(&self) -> u8 {
        self.hand_count
    }

    pub(super) fn break_hand_streak(&mut self) {
        self.hand_count = 0;
        self.last_hand_at = None;
    }

    pub(super) fn expire(&mut self, now: Duration) {
        if self
            .last_hand_at
            .is_some_and(|last| now.saturating_sub(last) >= HAND_STREAK_TIMEOUT)
        {
            self.break_hand_streak();
        }
    }

    /// Commit selection only after admission and a usable duration. Rejected
    /// mouse messages never consume a streak step, history, or cooldown.
    pub(super) fn accept(
        &mut self,
        part: BodyPart,
        now: Duration,
        clips: &NeraAnimationClips,
        state: &mut NeraAnimationState,
    ) -> Option<TouchReactionAccepted> {
        self.expire(now);
        if !state.can_request_touch_reaction() {
            return None;
        }
        let hand = matches!(part, BodyPart::LeftHand | BodyPart::RightHand);
        let arm = matches!(part, BodyPart::LeftArm | BodyPart::RightArm);
        if !hand && !arm {
            return None;
        }
        if now
            < if hand {
                self.hand_cooldown_until
            } else {
                self.arm_cooldown_until
            }
        {
            return None;
        }
        let step = if hand { self.hand_count + 1 } else { 0 };
        let pool: &[TouchReactionClip] = match step {
            0 => &TouchReactionClip::ARM,
            1 | 2 => &TouchReactionClip::NORMAL_HAND,
            3 | 4 => &[TouchReactionClip::Dismissive],
            _ => &[TouchReactionClip::Shrugging],
        };
        let previous = if hand {
            self.last_normal_hand
        } else {
            self.last_arm
        };
        let mut candidates: Vec<_> = pool
            .iter()
            .copied()
            .filter(|clip| clips.touch_reaction_duration(*clip).is_some())
            .collect();
        if candidates.len() > 1 {
            candidates.retain(|clip| Some(*clip) != previous);
        }
        let clip = candidates[self.rng.pick_index(candidates.len())?];
        if !state.request_touch_reaction(clip, clips.touch_reaction_duration(clip)?) {
            return None;
        }
        state.reset_damage_combo();
        if hand {
            self.hand_cooldown_until = now.saturating_add(TOUCH_REACTION_COOLDOWN);
            self.hand_count = step;
            self.last_hand_at = Some(now);
            if step <= 2 {
                self.last_normal_hand = Some(clip);
            }
            if step == 7 {
                self.break_hand_streak();
            }
        } else {
            self.arm_cooldown_until = now.saturating_add(TOUCH_REACTION_COOLDOWN);
            self.last_arm = Some(clip);
            self.break_hand_streak();
        }
        Some(TouchReactionAccepted {
            clip,
            hand_step: step,
            angry_mark: matches!(step, 4 | 7),
        })
    }
}

/// Optional context keeps lightweight existing interaction tests usable. The
/// production plugin always installs runtime and the accepted-message channel.
#[derive(SystemParam)]
pub(crate) struct TouchReactionContext<'w> {
    pub(super) time: Option<Res<'w, Time>>,
    pub(super) runtime: Option<ResMut<'w, TouchReactionRuntime>>,
    pub(super) accepted: Option<ResMut<'w, Messages<TouchReactionAccepted>>>,
    pub(super) interaction: Option<Res<'w, crate::interaction::DesktopPetInteraction>>,
    pub(super) presentation: Option<Res<'w, crate::presentation::PresentationState>>,
    pub(super) character: Option<Res<'w, crate::character::CharacterImportStatus>>,
    pub(super) seat_pose: Option<Res<'w, seated::TouchSeatPose>>,
}

impl TouchReactionContext<'_> {
    pub(super) fn headpat_blocked(&self) -> bool {
        self.interaction
            .as_ref()
            .is_some_and(|input| input.state == crate::interaction::InteractionState::Dragging)
            || self
                .presentation
                .as_ref()
                .is_some_and(|state| !(state.is_desktop_pet() || state.is_screensaver()))
            || self.character.as_ref().is_some_and(|status| status.loading)
    }
    pub(super) fn blocked(&self) -> bool {
        self.interaction
            .as_ref()
            .is_some_and(|input| input.state == crate::interaction::InteractionState::Dragging)
            || self
                .presentation
                .as_ref()
                .is_some_and(|state| !state.is_desktop_pet())
            || self.character.as_ref().is_some_and(|status| status.loading)
    }
}

#[cfg(debug_assertions)]
pub(crate) mod smoke;
#[cfg(test)]
mod tests;
