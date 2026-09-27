//! Translate interactions and desktop events into animation requests.
use std::time::Duration;

use bevy::prelude::*;

use crate::{
    environment::{DesktopEnvironment, PlacementState},
    interaction::{BodyPart, NeraClicked, NeraDragStarted, NeraHeadPatted, NeraReleased},
};

#[cfg(target_os = "windows")]
use crate::action_menu::{AnimationMenuChoice, NeraActionMenuCommand};

use super::{AlarmAnimationCommand, NeraAnimationClips, NeraAnimationState, TemporaryAction};

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
    observe_environment_support(&environment, &clips, &mut state);
}

fn on_sitting_surface(placement: PlacementState) -> bool {
    matches!(
        placement,
        PlacementState::PendingWindow { .. }
            | PlacementState::PendingTaskbar { .. }
            | PlacementState::Window(_)
            | PlacementState::Taskbar(_)
    )
}

fn observe_environment_support(
    environment: &DesktopEnvironment,
    clips: &NeraAnimationClips,
    state: &mut NeraAnimationState,
) {
    let enter_duration = clips
        .sitting_enter
        .map(|clip| clip.duration)
        .unwrap_or(Duration::ZERO);
    let exit_duration = clips
        .sitting_exit
        .map(|clip| clip.duration)
        .unwrap_or(Duration::ZERO);

    state.observe_sitting_surface(
        on_sitting_surface(environment.placement),
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
    mut environment: ResMut<DesktopEnvironment>,
) {
    if drag_started.read().next().is_some() {
        state.start_persistent_action(TemporaryAction::Floating);
    }

    for event in clicked.read() {
        if matches!(event.body_part, BodyPart::Head | BodyPart::Torso)
            && on_sitting_surface(environment.placement)
        {
            // Native support may have changed earlier in this frame, before
            // the regular support sync. Use actual placement for hit routing.
            observe_environment_support(&environment, &clips, &mut state);
            state.reset_damage_combo();
            // Let the base seated pose calibrate its anchor before playing a
            // clip authored with a different hip position. Early hits are
            // consumed, so they cannot fall through to standing damage.
            if matches!(
                environment.placement,
                PlacementState::Window(_) | PlacementState::Taskbar(_)
            ) {
                state.request_sitting_dodge(
                    clips
                        .dodge_while_sitting
                        .map(|clip| clip.duration)
                        .unwrap_or(Duration::ZERO),
                );
            }
            continue;
        }
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
