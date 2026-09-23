//! VRMA clip registry and load observers.
use std::time::Duration;

use bevy::prelude::*;
use bevy_vrm1::prelude::*;

use super::IdleMicroAction;

#[derive(Clone, Copy, Debug)]
pub(super) struct ClipSlot {
    pub(super) entity: Entity,
    pub(super) duration: Duration,
}

#[derive(Resource, Default)]
pub(crate) struct NeraAnimationClips {
    pub(super) idle: Option<ClipSlot>,
    pub(super) floating: Option<ClipSlot>,
    pub(super) falling: Option<ClipSlot>,
    pub(super) falling_to_landing: Option<ClipSlot>,
    pub(super) death: Option<ClipSlot>,
    pub(super) alarm_wave: Option<ClipSlot>,
    pub(super) alarm_jump: Option<ClipSlot>,
    pub(super) hit_head: Option<ClipSlot>,
    pub(super) hit_chest: Option<ClipSlot>,
    pub(super) head_stroke: Option<ClipSlot>,
    pub(super) praying: Option<ClipSlot>,
    pub(super) sitting_enter: Option<ClipSlot>,
    pub(super) sitting_loop: Option<ClipSlot>,
    pub(super) sitting_exit: Option<ClipSlot>,

    // Phase 5.8 idle micro-actions. Loaded asynchronously like everything
    // else, so the scheduler must treat a missing slot as "not available yet"
    // rather than as an error.
    pub(super) idle_look_around: Option<ClipSlot>,
    pub(super) idle_relax: Option<ClipSlot>,
    pub(super) idle_sleepy: Option<ClipSlot>,
    pub(super) idle_blush: Option<ClipSlot>,

    #[cfg(target_os = "windows")]
    pub(super) wave: Option<ClipSlot>,
    #[cfg(target_os = "windows")]
    pub(super) thinking: Option<ClipSlot>,
    #[cfg(target_os = "windows")]
    pub(super) angry: Option<ClipSlot>,
    #[cfg(target_os = "windows")]
    pub(super) sad: Option<ClipSlot>,
    #[cfg(target_os = "windows")]
    pub(super) surprised: Option<ClipSlot>,
}

impl NeraAnimationClips {
    pub(crate) fn falling_to_landing_duration(&self) -> Duration {
        self.falling_to_landing
            .map(|clip| clip.duration)
            .unwrap_or(Duration::ZERO)
    }

    pub(super) fn idle_micro(&self, action: IdleMicroAction) -> Option<ClipSlot> {
        match action {
            IdleMicroAction::LookAround => self.idle_look_around,
            IdleMicroAction::Relax => self.idle_relax,
            IdleMicroAction::Sleepy => self.idle_sleepy,
            IdleMicroAction::Blush => self.idle_blush,
        }
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

            info!(concat!($label, " VRMA loaded | duration={:?}"), duration);
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
register_clip!(register_head_stroke_animation, head_stroke, "ShakeHeadNo1");
register_clip!(register_praying_animation, praying, "Praying");
register_clip!(
    register_sitting_enter_animation,
    sitting_enter,
    "SittingEnter"
);
register_clip!(register_sitting_loop_animation, sitting_loop, "SittingLoop");
register_clip!(register_sitting_exit_animation, sitting_exit, "SittingExit");

// Phase 5.8 idle micro-actions.
register_clip!(
    register_idle_look_around_animation,
    idle_look_around,
    "Idle/LookAround"
);
register_clip!(register_idle_relax_animation, idle_relax, "Idle/Relax");
register_clip!(register_idle_sleepy_animation, idle_sleepy, "Idle/Sleepy");
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
register_clip!(register_surprised_animation, surprised, "Menu/Surprised");
