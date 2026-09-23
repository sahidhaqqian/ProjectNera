//! Resolve animation state to a loaded clip and apply VRMA playback.
use std::time::Duration;

use bevy::{animation::RepeatAnimation, prelude::*};
use bevy_vrm1::prelude::*;

use crate::environment::{AvatarAnchor, DesktopEnvironment};

use super::clips::{ClipSlot, NeraAnimationClips};
use super::{
    ALARM_BLEND, BASE_BLEND, BasePose, DEATH_BLEND, HEAD_STROKE_BLEND, IDLE_VARIATION_BLEND,
    NeraAnimationState, REACTION_BLEND, SUPPORT_BLEND, VisualState,
};

#[cfg(target_os = "windows")]
use crate::action_menu::AnimationMenuChoice;

#[derive(Resource, Default)]
pub(crate) struct NeraAnimationPlayback {
    current_clip: Option<Entity>,
    visual: Option<VisualState>,
    revision: u64,
}

pub(super) fn clip_for_visual(
    visual: VisualState,
    clips: &NeraAnimationClips,
) -> (Option<ClipSlot>, RepeatAnimation, &'static str) {
    match visual {
        VisualState::Idle => (clips.idle, RepeatAnimation::Forever, "Idle"),
        VisualState::SittingLoop => (clips.sitting_loop, RepeatAnimation::Forever, "SittingLoop"),
        VisualState::SittingEnter => (
            clips.sitting_enter,
            RepeatAnimation::Count(1),
            "SittingEnter/Landing",
        ),
        VisualState::SittingExit => (clips.sitting_exit, RepeatAnimation::Count(1), "SittingExit"),
        VisualState::Floating => (clips.floating, RepeatAnimation::Forever, "Floating"),
        // Phase 5.7 uses the existing Floating pose while airborne. The
        // dedicated Falling clip is intentionally not used because its body
        // pose does not match the desktop-pet fall presentation.
        VisualState::Falling => (clips.floating, RepeatAnimation::Forever, "Floating/Fall"),
        VisualState::Landing => (
            clips.falling_to_landing,
            RepeatAnimation::Count(1),
            "FallingToLanding",
        ),
        VisualState::HitHead => (clips.hit_head, RepeatAnimation::Count(1), "HitHead"),
        VisualState::HitChest => (clips.hit_chest, RepeatAnimation::Count(1), "HitChest"),
        VisualState::HeadStroke => (clips.head_stroke, RepeatAnimation::Count(1), "ShakeHeadNo1"),
        VisualState::Death => (clips.death, RepeatAnimation::Count(1), "Death01"),
        VisualState::AlarmJump => (
            clips.alarm_jump,
            RepeatAnimation::Count(1),
            "Alarm/JumpHappy",
        ),
        VisualState::AlarmWave => (clips.alarm_wave, RepeatAnimation::Forever, "Alarm/Wave"),
        #[cfg(target_os = "windows")]
        VisualState::MenuAnimation(choice) => match choice {
            AnimationMenuChoice::Wave => (clips.wave, RepeatAnimation::Count(1), "Menu/Wave"),
            AnimationMenuChoice::Thinking => {
                (clips.thinking, RepeatAnimation::Count(1), "Menu/Thinking")
            }
            AnimationMenuChoice::Angry => (clips.angry, RepeatAnimation::Count(1), "Menu/Angry"),
            AnimationMenuChoice::Sad => (clips.sad, RepeatAnimation::Count(1), "Menu/Sad"),
            AnimationMenuChoice::Surprised => {
                (clips.surprised, RepeatAnimation::Count(1), "Menu/Surprised")
            }
            AnimationMenuChoice::ResetBasePose => (None, RepeatAnimation::Forever, "Menu/Reset"),
        },
        VisualState::Praying => (clips.praying, RepeatAnimation::Forever, "Praying"),
        VisualState::IdleMicro(action) => (
            clips.idle_micro(action),
            RepeatAnimation::Count(1),
            action.label(),
        ),
    }
}

fn base_fallback(base: BasePose, clips: &NeraAnimationClips) -> Option<ClipSlot> {
    match base {
        BasePose::Idle => clips.idle,
        BasePose::Sitting => clips.sitting_loop.or(clips.idle),
    }
}

pub(super) fn transition_for_visual(visual: VisualState) -> Duration {
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

    let (requested, requested_repeat, requested_label) = clip_for_visual(visual, &clips);

    let (clip, repeat, label, using_requested) = if let Some(clip) = requested {
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

        (fallback, RepeatAnimation::Forever, "BaseFallback", false)
    };

    let non_base_visual = !matches!(visual, VisualState::Idle | VisualState::SittingLoop);

    let replay_requested =
        non_base_visual && using_requested && playback.revision != state.revision;

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
        label, anchor, clip.duration, transition_duration
    );
}
