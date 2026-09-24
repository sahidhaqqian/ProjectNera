//! Translate events and idle scheduling into animation requests.
use std::time::Duration;
use crate::rng::NeraRng;

use bevy::prelude::*;

use crate::{
    environment::{DesktopEnvironment, PlacementState},
    interaction::{BodyPart, NeraClicked, NeraDragStarted, NeraHeadPatted, NeraReleased},
};

#[cfg(target_os = "windows")]
use crate::action_menu::{AnimationMenuChoice, NeraActionMenuCommand};

use super::{
    AlarmAnimationCommand, IDLE_VARIATION_MAX_DELAY, IDLE_VARIATION_MIN_DELAY,
    IdleMicroAction, NeraAnimationClips, NeraAnimationState, TemporaryAction, VisualState,
};

#[derive(Resource, Default)]
pub(super) struct AlarmAnimationRuntime {
    pub(super) requested: bool,
    pub(super) started: bool,
    pub(super) waiting_logged: bool,
}
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

    state.observe_sitting_surface(sitting_surface, enter_duration, exit_duration);
}

pub(crate) fn sync_interaction_animation_actions(
    mut drag_started: MessageReader<NeraDragStarted>,
    mut clicked: MessageReader<NeraClicked>,
    mut head_patted: MessageReader<NeraHeadPatted>,
    mut released: MessageReader<NeraReleased>,
    clips: Res<NeraAnimationClips>,
    mut state: ResMut<NeraAnimationState>,
    mut environment: ResMut<DesktopEnvironment>,
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

    // Death01 depicts Nera collapsing to the ground. Keeping her glued to a
    // window/taskbar attachment while that plays is what produced the
    // "floating" death: the sitting follow-system kept re-snapping her native
    // window to the seat surface every frame, and the ground-reconciliation
    // pass below only runs for `PlacementState::Free`. Detach immediately so
    // she visually falls off whatever she was sitting on before the ground
    // pass takes over.
    if state.death_active() && environment.placement != PlacementState::Free {
        environment.detach_nera();
        info!("Nera detached: Death01 triggered while sitting/attached");
    }

    if head_patted.read().next().is_some() {
        if let Some(clip) = clips.head_stroke {
            state.request_one_shot(TemporaryAction::HeadStroke, clip.duration);
        } else {
            warn!("HeadStroke requested before ShakeHeadNo1.vrma was loaded");
        }
    }

    if released.read().any(|event| event.was_dragging) {
        state.finish_action(TemporaryAction::Floating);
    }
}

pub(super) fn sync_alarm_animation(
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

    let (Some(wave), Some(jump)) = (clips.alarm_wave, clips.alarm_jump) else {
        if !runtime.waiting_logged {
            info!("Nera alarm animation waiting for Wave + JumpHappy VRMAs...");
            runtime.waiting_logged = true;
        }
        return;
    };

    // Persistent Wave sits underneath the one-shot intro.
    state.start_persistent_action(TemporaryAction::AlarmWave);

    // Jump has higher priority. When its *real loaded clip duration* expires,
    // visual_state() naturally falls back to the persistent Wave.
    state.request_one_shot(TemporaryAction::AlarmJump, jump.duration);

    runtime.started = true;
    runtime.waiting_logged = false;

    info!(
        "Nera alarm animation started | jump={:?} wave={:?}",
        jump.duration, wave.duration,
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
            state
                .active_actions
                .retain(|active| !active.action.is_reaction());
            state.bump_revision();

            info!("Nera menu animation reset -> base {:?}", state.base_pose());
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

        state.request_one_shot(TemporaryAction::MenuAnimation(choice), clip.duration);
    }
}

pub(crate) fn tick_animation_actions(time: Res<Time>, mut state: ResMut<NeraAnimationState>) {
    state.advance(time.delta());
}

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
