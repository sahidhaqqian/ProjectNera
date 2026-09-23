//! Idle micro-action scheduling.
use std::time::Duration;

use bevy::prelude::*;

use crate::rng::NeraRng;

use super::{
    IDLE_VARIATION_MAX_DELAY, IDLE_VARIATION_MIN_DELAY, IdleMicroAction, NeraAnimationClips,
    NeraAnimationState, VisualState,
};

/// Phase 5.8 idle variation scheduler.
///
/// Counts down only while Nera is genuinely idle, so a long interaction
/// postpones variety rather than spending it on frames where nothing could
/// play. Shaped like `AlarmAnimationRuntime`: a small runtime resource beside
/// the authoritative `NeraAnimationState`, not a second state machine.
#[derive(Resource)]
pub(crate) struct IdleVariationRuntime {
    pub(super) cooldown: Duration,
    /// Avoids the same fidget twice in a row, which is the pattern users
    /// notice fastest.
    pub(super) last: Option<IdleMicroAction>,
    pub(super) rng: NeraRng,
}

impl Default for IdleVariationRuntime {
    fn default() -> Self {
        Self {
            cooldown: IDLE_VARIATION_MIN_DELAY,
            last: None,
            rng: NeraRng::from_entropy(),
        }
    }
}

impl IdleVariationRuntime {
    pub(super) fn tick(
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

        let Some((action, duration)) = pick_idle_micro(clips, self.last, &mut self.rng) else {
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

    pub(super) fn next_delay(&mut self) -> Duration {
        Duration::from_secs_f32(self.rng.range_f32(
            IDLE_VARIATION_MIN_DELAY.as_secs_f32(),
            IDLE_VARIATION_MAX_DELAY.as_secs_f32(),
        ))
    }
}

/// Choose a loaded micro-action, starting from a random offset.
///
/// Walking the list rather than re-rolling means a clip that has not finished
/// loading is skipped instead of stalling the scheduler, and no allocation is
/// needed. `avoid` demotes the previous fidget to a fallback, so a single
/// loaded clip still plays rather than the scheduler giving up.
pub(super) fn pick_idle_micro(
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
