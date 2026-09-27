//! Animation clip library, load observers, and VRMA playback.
use std::time::Duration;

use bevy::{animation::RepeatAnimation, prelude::*};
use bevy_vrm1::prelude::*;

use crate::environment::{AvatarAnchor, DesktopEnvironment};

use super::{
    ALARM_BLEND, BASE_BLEND, BasePose, DEATH_BLEND, HEAD_STROKE_BLEND, IDLE_VARIATION_BLEND,
    IdleMicroAction, NeraAnimationState, PersonalityAction, REACTION_BLEND, SUPPORT_BLEND,
    SittingPose, VisualState,
};

#[cfg(target_os = "windows")]
use crate::action_menu::AnimationMenuChoice;

#[derive(Clone, Copy, Debug)]
pub(super) struct ClipSlot {
    pub(super) entity: Entity,
    pub(super) duration: Duration,
}

#[derive(Resource, Default)]
pub(crate) struct NeraAnimationClips {
    pub(super) library: Option<ClipSlot>,
    pub(super) idle: Option<ClipSlot>,
    pub(super) walking: Option<ClipSlot>,
    pub(super) floating: Option<ClipSlot>,
    pub(super) falling_to_landing: Option<ClipSlot>,
    pub(super) death: Option<ClipSlot>,
    pub(super) alarm_wave: Option<ClipSlot>,
    pub(super) alarm_jump: Option<ClipSlot>,
    pub(super) hit_head: Option<ClipSlot>,
    pub(super) hit_chest: Option<ClipSlot>,
    pub(super) dodge_while_sitting: Option<ClipSlot>,
    pub(super) head_stroke: Option<ClipSlot>,
    pub(super) praying: Option<ClipSlot>,
    pub(super) sitting_enter: Option<ClipSlot>,
    pub(super) sitting_loop: Option<ClipSlot>,
    pub(super) sitting_2: Option<ClipSlot>,
    pub(super) sitting_4: Option<ClipSlot>,
    // Legacy resolver slot only: detach is immediate, so no exit clip is preloaded.
    pub(super) sitting_exit: Option<ClipSlot>,

    // Phase 5.8 idle micro-actions. Loaded asynchronously like everything
    // else, so the scheduler must treat a missing slot as "not available yet"
    // rather than as an error.
    pub(super) idle_look_around: Option<ClipSlot>,
    pub(super) idle_relax: Option<ClipSlot>,
    pub(super) idle_sleepy: Option<ClipSlot>,
    pub(super) idle_blush: Option<ClipSlot>,
    pub(super) idle_catwalk_twist: Option<ClipSlot>,
    pub(super) idle_bored: Option<ClipSlot>,
    pub(super) idle_angry_gesture: Option<ClipSlot>,
    pub(super) idle_sitting_rubbing_arm: Option<ClipSlot>,

    // Legacy menu resolver slots remain empty. The current radial menu has no
    // animation buttons, so these stock clips are not preloaded.
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
    pub(super) fn sitting_pose(&self, pose: SittingPose) -> Option<ClipSlot> {
        match pose {
            SittingPose::Default => self.sitting_loop,
            SittingPose::Sitting2 => self.sitting_2,
            SittingPose::Sitting4 => self.sitting_4,
        }
    }

    pub(crate) fn walking_ready(&self) -> bool {
        self.walking.is_some_and(|clip| !clip.duration.is_zero())
    }

    /// Only usable, loaded clips are candidates for autonomous behavior.
    pub(crate) fn idle_micro_duration(&self, action: IdleMicroAction) -> Option<Duration> {
        self.idle_micro(action)
            .filter(|clip| !clip.duration.is_zero())
            .map(|clip| clip.duration)
    }

    /// Optional intents currently reuse LookAround. Other intents are explicit
    /// unavailable capabilities until their clips are registered; no additional
    /// VRMA files are required or searched for by this lookup.
    pub(crate) fn personality_duration(&self, action: PersonalityAction) -> Option<Duration> {
        self.idle_micro_duration(action.idle_micro()?)
    }

    pub(crate) fn falling_to_landing_duration(&self) -> Duration {
        self.falling_to_landing
            .map(|clip| clip.duration)
            .unwrap_or(Duration::ZERO)
    }

    /// The greeting shares the alarm's existing wave asset, but has its own
    /// one-shot lifetime and priority. No extra VRMA or model is loaded.
    pub(crate) fn startup_greeting_duration(&self) -> Duration {
        self.alarm_wave
            .map(|clip| clip.duration)
            .unwrap_or(Duration::ZERO)
    }

    /// The entrance is one visible sequence. Do not start the drop until its
    /// final greeting is also retargeted for the active character.
    pub(crate) fn startup_intro_ready(&self) -> bool {
        [
            self.idle,
            self.floating,
            self.falling_to_landing,
            self.alarm_wave,
        ]
        .into_iter()
        .all(|slot| slot.is_some_and(|clip| !clip.duration.is_zero()))
    }

    /// The startup intro is optional, but the pet must not be revealed before
    /// its rig has received a usable base pose. This lets startup distinguish
    /// "Idle is already playing" from "every entrance clip is available".
    pub(crate) fn idle_playback_ready(&self, playback: &NeraAnimationPlayback) -> bool {
        self.idle.is_some_and(|clip| {
            !clip.duration.is_zero() && playback.current_clip == Some(clip.entity)
        })
    }

    pub(super) fn idle_micro(&self, action: IdleMicroAction) -> Option<ClipSlot> {
        match action {
            IdleMicroAction::LookAround => self.idle_look_around,
            IdleMicroAction::Relax => self.idle_relax,
            IdleMicroAction::Sleepy => self.idle_sleepy,
            IdleMicroAction::Blush => self.idle_blush,
            IdleMicroAction::CatwalkIdleTwistR => self.idle_catwalk_twist,
            IdleMicroAction::Bored => self.idle_bored,
            IdleMicroAction::AngryGesture => self.idle_angry_gesture,
            IdleMicroAction::SittingRubbingArm => self.idle_sitting_rubbing_arm,
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
register_clip!(register_walking_animation, walking, "Walking");
register_clip!(register_floating_animation, floating, "Floating");
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
    register_sitting_dodge_animation,
    dodge_while_sitting,
    "DodgeWhileSitting"
);
register_clip!(register_head_stroke_animation, head_stroke, "ShakeHeadNo1");
register_clip!(register_praying_animation, praying, "Praying");
register_clip!(
    register_sitting_enter_animation,
    sitting_enter,
    "SittingEnter"
);
register_clip!(register_sitting_loop_animation, sitting_loop, "SittingLoop");

// Phase 5.8 idle micro-actions.
register_clip!(
    register_idle_look_around_animation,
    idle_look_around,
    "Idle/LookAround"
);
register_clip!(register_idle_relax_animation, idle_relax, "Idle/Relax");
register_clip!(register_idle_sleepy_animation, idle_sleepy, "Idle/Sleepy");
register_clip!(register_idle_blush_animation, idle_blush, "Idle/Blush");
register_clip!(
    register_idle_catwalk_twist_animation,
    idle_catwalk_twist,
    "Idle/CatwalkIdleTwistR"
);
register_clip!(register_idle_bored_animation, idle_bored, "Idle/Bored");
register_clip!(
    register_idle_angry_gesture_animation,
    idle_angry_gesture,
    "Idle/AngryGesture"
);
register_clip!(register_sitting_2_animation, sitting_2, "Sitting/Sitting2");
register_clip!(register_sitting_4_animation, sitting_4, "Sitting/Sitting4");
register_clip!(
    register_idle_sitting_rubbing_arm_animation,
    idle_sitting_rubbing_arm,
    "Sitting/RubbingArm"
);

// Phase 5 runtime animation set. Large animation libraries stay as stock;
// only stable animations used by current behavior live in animasi_basic.
const IDLE_VRMA_PATH: &str = "animations/animasi_basic/idle.vrma";
const WALKING_VRMA_PATH: &str = "animations/animasi_basic/Walking.vrma";
const FLOATING_VRMA_PATH: &str = "animations/animasi_basic/Floating.vrma";
const DEATH_VRMA_PATH: &str = "animations/animasi_basic/Death01.vrma";
const FALLING_TO_LANDING_VRMA_PATH: &str = "animations/animasi_basic/Falling_To_Landing.vrma";
const ALARM_JUMP_VRMA_PATH: &str = "animations/animasi_basic/jump_happy.vrma";
const ALARM_WAVE_VRMA_PATH: &str = "animations/animasi_basic/161_Waving.vrma";
const HIT_HEAD_VRMA_PATH: &str = "animations/animasi_basic/Hit_Head.vrma";
const HIT_CHEST_VRMA_PATH: &str = "animations/animasi_basic/Hit_Chest.vrma";
const SITTING_DODGE_VRMA_PATH: &str = "animations/animasi_basic/Dodge_While_Sitting.vrma";
const HEAD_STROKE_VRMA_PATH: &str = "animations/animasi_basic/ShakeHeadNo1.vrma";
const PRAYING_VRMA_PATH: &str = "animations/animasi_basic/Praying.vrma";
const SITTING_ENTER_VRMA_PATH: &str = "animations/animasi_basic/Sitting_Enter.vrma";
const SITTING_LOOP_VRMA_PATH: &str = "animations/animasi_basic/Sitting_Idle_Loop.vrma";

// Phase 5.8 idle micro-actions. Fidgets only: Clapping/Goodbye/Jump are also
// converted and unused, but a pet that randomly applauds while you work reads
// as noise rather than life.
const IDLE_LOOK_AROUND_VRMA_PATH: &str = "animations/animasi_basic/LookAround.vrma";
const IDLE_RELAX_VRMA_PATH: &str = "animations/animasi_basic/Relax.vrma";
const IDLE_SLEEPY_VRMA_PATH: &str = "animations/animasi_basic/Sleepy.vrma";
const IDLE_BLUSH_VRMA_PATH: &str = "animations/animasi_basic/Blush.vrma";
const IDLE_CATWALK_TWIST_VRMA_PATH: &str = "animations/animasi_basic/Catwalk_Idle_Twist_R.vrma";
const IDLE_BORED_VRMA_PATH: &str = "animations/animasi_basic/Bored.vrma";
const IDLE_ANGRY_GESTURE_VRMA_PATH: &str = "animations/animasi_basic/Angry_Gesture.vrma";
const IDLE_SITTING_2_VRMA_PATH: &str = "animations/animasi_basic/Sitting_2.vrma";
const IDLE_SITTING_4_VRMA_PATH: &str = "animations/animasi_basic/Sitting_4.vrma";
const IDLE_SITTING_RUBBING_ARM_VRMA_PATH: &str =
    "animations/animasi_basic/Sitting_Rubbing_Arm.vrma";
pub(crate) fn spawn_animation_library(
    parent: &mut bevy::ecs::hierarchy::ChildSpawnerCommands<'_>,
    asset_server: &AssetServer,
) {
    parent
        .spawn((
            Name::new("Nera Idle"),
            VrmaHandle(asset_server.load(IDLE_VRMA_PATH)),
        ))
        .observe(register_idle_animation);

    parent
        .spawn((
            Name::new("Nera Walking"),
            VrmaHandle(asset_server.load(WALKING_VRMA_PATH)),
        ))
        .observe(register_walking_animation);

    parent
        .spawn((
            Name::new("Nera Floating"),
            VrmaHandle(asset_server.load(FLOATING_VRMA_PATH)),
        ))
        .observe(register_floating_animation);

    parent
        .spawn((
            Name::new("Nera Death Reaction"),
            VrmaHandle(asset_server.load(DEATH_VRMA_PATH)),
        ))
        .observe(register_death_animation);

    parent
        .spawn((
            Name::new("Nera Falling To Landing"),
            VrmaHandle(asset_server.load(FALLING_TO_LANDING_VRMA_PATH)),
        ))
        .observe(register_falling_to_landing_animation);

    // These clips serve both alarm mode and the startup greeting. Keeping
    // every rig-bound source in this function guarantees that imports and the
    // saved-character restore attach them to the actual active VRM hierarchy.
    parent
        .spawn((
            Name::new("Nera Alarm JumpHappy"),
            VrmaHandle(asset_server.load(ALARM_JUMP_VRMA_PATH)),
        ))
        .observe(register_alarm_jump_animation);

    parent
        .spawn((
            Name::new("Nera Alarm Wave"),
            VrmaHandle(asset_server.load(ALARM_WAVE_VRMA_PATH)),
        ))
        .observe(register_alarm_wave_animation);

    parent
        .spawn((
            Name::new("Nera Hit Head"),
            VrmaHandle(asset_server.load(HIT_HEAD_VRMA_PATH)),
        ))
        .observe(register_hit_head_animation);

    parent
        .spawn((
            Name::new("Nera Hit Chest"),
            VrmaHandle(asset_server.load(HIT_CHEST_VRMA_PATH)),
        ))
        .observe(register_hit_chest_animation);

    parent
        .spawn((
            Name::new("Nera Sitting Dodge"),
            VrmaHandle(asset_server.load(SITTING_DODGE_VRMA_PATH)),
        ))
        .observe(register_sitting_dodge_animation);

    parent
        .spawn((
            Name::new("Nera Head Stroke Reaction"),
            VrmaHandle(asset_server.load(HEAD_STROKE_VRMA_PATH)),
        ))
        .observe(register_head_stroke_animation);

    parent
        .spawn((
            Name::new("Nera Praying Menu Pose"),
            VrmaHandle(asset_server.load(PRAYING_VRMA_PATH)),
        ))
        .observe(register_praying_animation);

    parent
        .spawn((
            Name::new("Nera Sitting Enter"),
            VrmaHandle(asset_server.load(SITTING_ENTER_VRMA_PATH)),
        ))
        .observe(register_sitting_enter_animation);

    parent
        .spawn((
            Name::new("Nera Sitting Loop"),
            VrmaHandle(asset_server.load(SITTING_LOOP_VRMA_PATH)),
        ))
        .observe(register_sitting_loop_animation);

    // Phase 5.8 idle micro-actions. Same child-of-"Nera" pattern as
    // everything else here: bevy_vrm1 needs the VRM ancestry to
    // retarget the clip.
    parent
        .spawn((
            Name::new("Nera Idle LookAround"),
            VrmaHandle(asset_server.load(IDLE_LOOK_AROUND_VRMA_PATH)),
        ))
        .observe(register_idle_look_around_animation);

    parent
        .spawn((
            Name::new("Nera Idle Relax"),
            VrmaHandle(asset_server.load(IDLE_RELAX_VRMA_PATH)),
        ))
        .observe(register_idle_relax_animation);

    parent
        .spawn((
            Name::new("Nera Idle Sleepy"),
            VrmaHandle(asset_server.load(IDLE_SLEEPY_VRMA_PATH)),
        ))
        .observe(register_idle_sleepy_animation);

    parent
        .spawn((
            Name::new("Nera Idle Blush"),
            VrmaHandle(asset_server.load(IDLE_BLUSH_VRMA_PATH)),
        ))
        .observe(register_idle_blush_animation);

    parent
        .spawn((
            Name::new("Nera Idle Catwalk Twist"),
            VrmaHandle(asset_server.load(IDLE_CATWALK_TWIST_VRMA_PATH)),
        ))
        .observe(register_idle_catwalk_twist_animation);
    parent
        .spawn((
            Name::new("Nera Idle Bored"),
            VrmaHandle(asset_server.load(IDLE_BORED_VRMA_PATH)),
        ))
        .observe(register_idle_bored_animation);
    parent
        .spawn((
            Name::new("Nera Idle Angry Gesture"),
            VrmaHandle(asset_server.load(IDLE_ANGRY_GESTURE_VRMA_PATH)),
        ))
        .observe(register_idle_angry_gesture_animation);
    parent
        .spawn((
            Name::new("Nera Sitting Variation 2"),
            VrmaHandle(asset_server.load(IDLE_SITTING_2_VRMA_PATH)),
        ))
        .observe(register_sitting_2_animation);
    parent
        .spawn((
            Name::new("Nera Sitting Variation 4"),
            VrmaHandle(asset_server.load(IDLE_SITTING_4_VRMA_PATH)),
        ))
        .observe(register_sitting_4_animation);
    parent
        .spawn((
            Name::new("Nera Sitting Rubbing Arm"),
            VrmaHandle(asset_server.load(IDLE_SITTING_RUBBING_ARM_VRMA_PATH)),
        ))
        .observe(register_idle_sitting_rubbing_arm_animation);
}

#[derive(Resource, Default)]
pub(crate) struct NeraAnimationPlayback {
    pub(super) current_clip: Option<Entity>,
    visual: Option<VisualState>,
    revision: u64,
}

pub(super) fn clip_for_visual(
    visual: VisualState,
    clips: &NeraAnimationClips,
) -> (Option<ClipSlot>, RepeatAnimation, &'static str) {
    match visual {
        VisualState::LibraryAnimation(entity) => (
            clips.library.filter(|clip| clip.entity == entity),
            RepeatAnimation::Count(1),
            "Library",
        ),
        VisualState::Idle => (clips.idle, RepeatAnimation::Forever, "Idle"),
        VisualState::Walking => (clips.walking, RepeatAnimation::Forever, "Walking"),
        VisualState::StartupGreeting => (
            clips.alarm_wave,
            RepeatAnimation::Count(1),
            "Startup/Greeting",
        ),
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
        VisualState::DodgeWhileSitting => (
            clips.dodge_while_sitting,
            RepeatAnimation::Count(1),
            "DodgeWhileSitting",
        ),
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

pub(super) fn clip_for_state(
    state: &NeraAnimationState,
    clips: &NeraAnimationClips,
) -> (Option<ClipSlot>, RepeatAnimation, &'static str) {
    if state.visual_state() == VisualState::SittingLoop {
        return (
            selected_sitting_clip(state, clips),
            RepeatAnimation::Forever,
            "SittingLoop",
        );
    }
    clip_for_visual(state.visual_state(), clips)
}

fn selected_sitting_clip(
    state: &NeraAnimationState,
    clips: &NeraAnimationClips,
) -> Option<ClipSlot> {
    clips
        .sitting_pose(state.sitting_pose.unwrap_or_default())
        .or(clips.sitting_loop)
}

fn base_fallback(state: &NeraAnimationState, clips: &NeraAnimationClips) -> Option<ClipSlot> {
    match state.base_pose() {
        BasePose::Idle => clips.idle,
        BasePose::Sitting => selected_sitting_clip(state, clips).or(clips.idle),
    }
}

pub(super) fn transition_for_visual(visual: VisualState) -> Duration {
    match visual {
        VisualState::LibraryAnimation(_) => REACTION_BLEND,
        VisualState::HeadStroke => HEAD_STROKE_BLEND,
        VisualState::Falling | VisualState::Landing => SUPPORT_BLEND,
        VisualState::HitHead | VisualState::HitChest | VisualState::DodgeWhileSitting => {
            REACTION_BLEND
        }
        VisualState::Death => DEATH_BLEND,
        VisualState::AlarmJump | VisualState::AlarmWave => ALARM_BLEND,
        #[cfg(target_os = "windows")]
        VisualState::MenuAnimation(_) => REACTION_BLEND,
        VisualState::SittingEnter | VisualState::SittingExit => SUPPORT_BLEND,
        VisualState::IdleMicro(_) => IDLE_VARIATION_BLEND,
        VisualState::StartupGreeting => IDLE_VARIATION_BLEND,
        VisualState::Idle
        | VisualState::Walking
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
    let leaving_walk =
        playback.visual == Some(VisualState::Walking) && visual != VisualState::Walking;

    if visual_changed {
        info!(
            "Nera animation -> {:?} | base={:?}",
            visual,
            state.base_pose()
        );
        playback.visual = Some(visual);
    }

    let (requested, requested_repeat, requested_label) = clip_for_state(&state, &clips);

    let (clip, repeat, label, using_requested) = if let Some(clip) = requested {
        (clip, requested_repeat, requested_label, true)
    } else {
        if visual_changed {
            warn!(
                "Nera clip unavailable for {:?}; using base fallback",
                visual
            );
        }

        let Some(fallback) = base_fallback(&state, &clips) else {
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

    // Walking's horizontal hips motion is constrained after animation. Once
    // another pose takes over, do not blend its unconstrained source motion
    // into that pose: even a short fade could move Nera out of her window.
    let transition_duration = if leaving_walk {
        Duration::ZERO
    } else {
        transition_for_visual(visual)
    };

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
