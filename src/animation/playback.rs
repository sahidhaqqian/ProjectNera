//! Animation clip library, load observers, and VRMA playback.
use std::time::Duration;

use bevy::{animation::RepeatAnimation, prelude::*};
use bevy_vrm1::prelude::*;

use super::state::TouchReactionClip;
use crate::environment::{AvatarAnchor, DesktopEnvironment};

use super::{
    ALARM_BLEND, BASE_BLEND, BasePose, DEATH_BLEND, HEAD_STROKE_BLEND, IDLE_VARIATION_BLEND,
    IdleMicroAction, NeraAnimationState, PersonalityAction, REACTION_BLEND, SUPPORT_BLEND,
    SittingPose, VisualState,
};

/// Pose-space contact policy. Desktop/cloud/presentation still exclusively own
/// placement; this only tells avatar projection whether feet should meet it.
/// Future grounded Dance/Music clips can opt into the same policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GroundingPolicy {
    Grounded,
    BodyContact,
    Authored,
}

pub(crate) fn grounding_policy(visual: VisualState, library_path: Option<&str>) -> GroundingPolicy {
    use GroundingPolicy::*;
    match visual {
        VisualState::Death => BodyContact,
        VisualState::Screensaver(_)
        | VisualState::FocusActivity(crate::focus::FocusPreset::Work) => Authored,
        VisualState::Floating
        | VisualState::Falling
        | VisualState::Landing
        | VisualState::AlarmJump
        | VisualState::AlarmWave
        | VisualState::SittingEnter
        | VisualState::SittingLoop
        | VisualState::SittingExit
        | VisualState::DodgeWhileSitting => Authored,
        VisualState::IdleMicro(action) if action.base_pose() == BasePose::Sitting => Authored,
        VisualState::LibraryAnimation(_) => match library_path {
            Some(DEATH_VRMA_PATH) => BodyContact,
            Some(path)
                if TouchReactionClip::ALL
                    .iter()
                    .any(|clip| clip.path() == path) =>
            {
                Grounded
            }
            // The bundled library includes intentionally airborne and seated
            // clips. Unknown imports retain their authored motion too: guessing
            // contact from a filename could erase a custom jump.
            Some(
                IDLE_VRMA_PATH
                | WALKING_VRMA_PATH
                | HIT_HEAD_VRMA_PATH
                | HIT_CHEST_VRMA_PATH
                | HEAD_STROKE_VRMA_PATH
                | PRAYING_VRMA_PATH
                | IDLE_LOOK_AROUND_VRMA_PATH
                | IDLE_RELAX_VRMA_PATH
                | IDLE_SLEEPY_VRMA_PATH
                | IDLE_BLUSH_VRMA_PATH
                | IDLE_CATWALK_TWIST_VRMA_PATH
                | IDLE_BORED_VRMA_PATH
                | IDLE_ANGRY_GESTURE_VRMA_PATH
                | "animations/animasi_basic/161_Waving.vrma",
            ) => Grounded,
            _ => Authored,
        },
        _ => Grounded,
    }
}

#[cfg(target_os = "windows")]
use crate::action_menu::AnimationMenuChoice;

#[derive(Clone, Copy, Debug)]
pub(super) struct ClipSlot {
    pub(super) entity: Entity,
    pub(super) duration: Duration,
}

#[derive(Resource, Default)]
pub(crate) struct NeraAnimationClips {
    pub(super) focus_typing: Option<ClipSlot>,
    pub(super) screensaver: [Option<ClipSlot>; 9],
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
    pub(super) treat: Option<ClipSlot>,
    pub(super) praying: Option<ClipSlot>,
    pub(super) sitting_enter: Option<ClipSlot>,
    pub(super) sitting_loop: Option<ClipSlot>,
    pub(super) sitting_2: Option<ClipSlot>,
    pub(super) sitting_4: Option<ClipSlot>,
    pub(super) sleeping_idle: Option<ClipSlot>,
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
    /// Authored touch performances, registered only after their VRMA is ready.
    pub(super) touch_reactions: [Option<ClipSlot>; 8],

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
    pub(crate) fn focus_activity_ready(&self, preset: crate::focus::FocusPreset) -> bool {
        self.focus_activity(preset).is_some()
    }
    fn focus_activity(&self, preset: crate::focus::FocusPreset) -> Option<ClipSlot> {
        match preset {
            crate::focus::FocusPreset::Study => self.praying,
            crate::focus::FocusPreset::Work => self.focus_typing,
        }
    }
    pub(crate) fn treat_duration(&self) -> Option<Duration> {
        self.treat
            .filter(|clip| !clip.duration.is_zero())
            .map(|clip| clip.duration)
    }
    pub(crate) fn treat_playing(&self, playback: &NeraAnimationPlayback) -> bool {
        self.treat
            .is_some_and(|clip| playback.current_clip == Some(clip.entity))
    }
    fn touch_reaction(&self, clip: TouchReactionClip) -> Option<ClipSlot> {
        self.touch_reactions[clip.index()].filter(|slot| !slot.duration.is_zero())
    }

    pub(crate) fn touch_reaction_duration(&self, clip: TouchReactionClip) -> Option<Duration> {
        self.touch_reaction(clip).map(|slot| slot.duration)
    }

    pub(crate) fn touch_reaction_entity(&self, clip: TouchReactionClip) -> Option<Entity> {
        self.touch_reaction(clip).map(|slot| slot.entity)
    }

    pub(crate) fn touch_reaction_playing(
        &self,
        clip: TouchReactionClip,
        playback: &NeraAnimationPlayback,
    ) -> bool {
        self.touch_reaction_entity(clip)
            .is_some_and(|entity| playback.current_clip == Some(entity))
    }

    /// Read the existing action clock; feedback must not own a second VRMA timer.
    pub(crate) fn head_stroke_progress(&self, state: &NeraAnimationState) -> Option<f32> {
        if state.visual_state() != VisualState::HeadStroke {
            return None;
        }
        let duration = self.head_stroke?.duration.as_secs_f32();
        if duration <= 0.0 {
            return None;
        }
        let remaining = state
            .active_actions
            .iter()
            .find(|active| active.action == super::TemporaryAction::HeadStroke)?
            .remaining?
            .as_secs_f32();
        Some((1.0 - remaining / duration).clamp(0.0, 1.0))
    }

    #[cfg(debug_assertions)]
    pub(crate) fn head_stroke_playing(&self, playback: &NeraAnimationPlayback) -> bool {
        self.head_stroke
            .is_some_and(|clip| playback.current_clip == Some(clip.entity))
    }

    pub(crate) fn sleeping_idle_entity(&self) -> Option<Entity> {
        self.sleeping_idle
            .filter(|clip| !clip.duration.is_zero())
            .map(|clip| clip.entity)
    }

    pub(crate) fn sleeping_idle_playing(&self, playback: &NeraAnimationPlayback) -> bool {
        self.sleeping_idle_entity()
            .is_some_and(|entity| playback.current_clip == Some(entity))
    }

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
            IdleMicroAction::CursorNotice
            | IdleMicroAction::CursorReach
            | IdleMicroAction::CursorCatch
            | IdleMicroAction::CursorEscape
            | IdleMicroAction::EdgePeek => self.idle,
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

#[derive(Component)]
struct TouchReactionLoader(TouchReactionClip);

fn register_touch_reaction_animation(
    trigger: On<LoadedVrma>,
    loaders: Query<&TouchReactionLoader>,
    durations: Query<&VrmaDuration>,
    mut clips: ResMut<NeraAnimationClips>,
) {
    let Ok(loader) = loaders.get(trigger.vrma) else {
        return;
    };
    let duration = durations
        .get(trigger.vrma)
        .map(|duration| duration.0)
        .unwrap_or(Duration::ZERO);
    clips.touch_reactions[loader.0.index()] = Some(ClipSlot {
        entity: trigger.vrma,
        duration,
    });
    info!("{} VRMA loaded | duration={duration:?}", loader.0.label());
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
register_clip!(register_treat_animation, treat, "Treat/Drinking");
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
    register_focus_typing_animation,
    focus_typing,
    "Focus/Typing"
);
register_clip!(
    register_sitting_enter_animation,
    sitting_enter,
    "SittingEnter"
);
register_clip!(register_sitting_loop_animation, sitting_loop, "SittingLoop");
register_clip!(
    register_sleeping_idle_animation,
    sleeping_idle,
    "Cloud/SleepingIdle"
);

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
const SLEEPING_IDLE_VRMA_PATH: &str = "animations/animasi_basic/Sleeping_Idle.vrma";

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

/// Runtime animations that are useful but do not need to consume memory from
/// process start. Their loader entities stay attached to the active VRM rig;
/// the actual `VrmaHandle` is inserted only when a feature first requests it.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LazyNeraVrma {
    FocusTyping,
    Screensaver(super::screensaver::ScreensaverClip),
    SleepingIdle,
    AlarmJump,
    Praying,
    IdleSleepy,
    IdleBlush,
    IdleCatwalkTwist,
    IdleBored,
    IdleAngryGesture,
}

impl LazyNeraVrma {
    fn path(self) -> &'static str {
        match self {
            Self::FocusTyping => "animations/animasi_basic/Typing.vrma",
            Self::Screensaver(clip) => clip.path(),
            Self::SleepingIdle => SLEEPING_IDLE_VRMA_PATH,
            Self::AlarmJump => ALARM_JUMP_VRMA_PATH,
            Self::Praying => PRAYING_VRMA_PATH,
            Self::IdleSleepy => IDLE_SLEEPY_VRMA_PATH,
            Self::IdleBlush => IDLE_BLUSH_VRMA_PATH,
            Self::IdleCatwalkTwist => IDLE_CATWALK_TWIST_VRMA_PATH,
            Self::IdleBored => IDLE_BORED_VRMA_PATH,
            Self::IdleAngryGesture => IDLE_ANGRY_GESTURE_VRMA_PATH,
        }
    }

    pub(crate) fn for_idle_micro(action: IdleMicroAction) -> Option<Self> {
        match action {
            IdleMicroAction::Sleepy => Some(Self::IdleSleepy),
            IdleMicroAction::Blush => Some(Self::IdleBlush),
            IdleMicroAction::CatwalkIdleTwistR => Some(Self::IdleCatwalkTwist),
            IdleMicroAction::Bored => Some(Self::IdleBored),
            IdleMicroAction::AngryGesture => Some(Self::IdleAngryGesture),
            IdleMicroAction::LookAround
            | IdleMicroAction::Relax
            | IdleMicroAction::SittingRubbingArm
            | IdleMicroAction::CursorNotice
            | IdleMicroAction::CursorReach
            | IdleMicroAction::CursorCatch
            | IdleMicroAction::CursorEscape
            | IdleMicroAction::EdgePeek => None,
        }
    }
}

#[derive(Component)]
pub(crate) struct LazyNeraVrmaRequested;

/// Start loading one cold VRMA directly on its pre-created loader entity.
/// No queue/message resource is required, so this is safe even during startup.
pub(crate) fn request_lazy_vrma(
    kind: LazyNeraVrma,
    commands: &mut Commands,
    asset_server: &AssetServer,
    loaders: &Query<(Entity, &LazyNeraVrma, Option<&LazyNeraVrmaRequested>)>,
) -> bool {
    for (entity, loader_kind, requested) in loaders.iter() {
        if *loader_kind != kind {
            continue;
        }

        if requested.is_none() {
            commands.entity(entity).insert((
                VrmaHandle(asset_server.load(kind.path())),
                LazyNeraVrmaRequested,
            ));
            info!(
                "Nera lazy VRMA requested | kind={kind:?} path={}",
                kind.path()
            );
        }
        return true;
    }

    false
}

pub(crate) fn ensure_state_lazy_vrmas(
    state: Res<NeraAnimationState>,
    clips: Res<NeraAnimationClips>,
    cloud: Option<Res<crate::cloud::CloudRuntime>>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    loaders: Query<(Entity, &LazyNeraVrma, Option<&LazyNeraVrmaRequested>)>,
) {
    if cloud.as_ref().is_some_and(|cloud| cloud.is_lying_rest())
        && clips.sleeping_idle_entity().is_none()
    {
        request_lazy_vrma(
            LazyNeraVrma::SleepingIdle,
            &mut commands,
            &asset_server,
            &loaders,
        );
    }
    if state.visual_state() == VisualState::Praying && clips.praying.is_none() {
        request_lazy_vrma(
            LazyNeraVrma::Praying,
            &mut commands,
            &asset_server,
            &loaders,
        );
    }
}
pub(crate) fn spawn_animation_library(
    parent: &mut bevy::ecs::hierarchy::ChildSpawnerCommands<'_>,
    asset_server: &AssetServer,
) {
    parent
        .spawn((
            Name::new("Treat/Drinking"),
            VrmaHandle(asset_server.load(crate::treat::DRINKING_PATH)),
        ))
        .observe(register_treat_animation);
    for clip in super::screensaver::ScreensaverClip::ALL {
        parent
            .spawn((
                Name::new(format!("Screensaver {clip:?} (lazy)")),
                LazyNeraVrma::Screensaver(clip),
            ))
            .observe(super::screensaver::register);
    }
    for clip in TouchReactionClip::ALL {
        parent
            .spawn((
                Name::new(clip.label()),
                TouchReactionLoader(clip),
                VrmaHandle(asset_server.load(clip.path())),
            ))
            .observe(register_touch_reaction_animation);
    }

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
            Name::new("Nera Alarm JumpHappy (lazy)"),
            LazyNeraVrma::AlarmJump,
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
            Name::new("Nera Praying Menu Pose (lazy)"),
            LazyNeraVrma::Praying,
        ))
        .observe(register_praying_animation);

    parent
        .spawn((
            Name::new("Nera Focus Typing (lazy)"),
            LazyNeraVrma::FocusTyping,
        ))
        .observe(register_focus_typing_animation);

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

    parent
        .spawn((
            Name::new("Nera Sleeping Idle (lazy)"),
            LazyNeraVrma::SleepingIdle,
        ))
        .observe(register_sleeping_idle_animation);

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
            Name::new("Nera Idle Sleepy (lazy)"),
            LazyNeraVrma::IdleSleepy,
        ))
        .observe(register_idle_sleepy_animation);

    parent
        .spawn((Name::new("Nera Idle Blush (lazy)"), LazyNeraVrma::IdleBlush))
        .observe(register_idle_blush_animation);

    parent
        .spawn((
            Name::new("Nera Idle Catwalk Twist (lazy)"),
            LazyNeraVrma::IdleCatwalkTwist,
        ))
        .observe(register_idle_catwalk_twist_animation);
    parent
        .spawn((Name::new("Nera Idle Bored (lazy)"), LazyNeraVrma::IdleBored))
        .observe(register_idle_bored_animation);
    parent
        .spawn((
            Name::new("Nera Idle Angry Gesture (lazy)"),
            LazyNeraVrma::IdleAngryGesture,
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

#[cfg(test)]
pub(crate) fn touch_test_clips(world: &mut World) -> NeraAnimationClips {
    let mut clips = NeraAnimationClips::default();
    for clip in TouchReactionClip::ALL {
        clips.touch_reactions[clip.index()] = Some(ClipSlot {
            entity: world.spawn_empty().id(),
            duration: Duration::from_secs(2),
        });
    }
    clips
}

pub(super) fn clip_for_visual(
    visual: VisualState,
    clips: &NeraAnimationClips,
) -> (Option<ClipSlot>, RepeatAnimation, &'static str) {
    match visual {
        VisualState::Screensaver(kind) => (
            clips.screensaver[kind.index()],
            if kind == super::screensaver::ScreensaverClip::Breathing {
                RepeatAnimation::Forever
            } else {
                RepeatAnimation::Count(1)
            },
            "Screensaver",
        ),
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
        VisualState::Treat => (clips.treat, RepeatAnimation::Count(1), "Treat/Drinking"),
        VisualState::FocusCelebrate => (
            clips.alarm_wave,
            RepeatAnimation::Count(1),
            "Focus/Complete",
        ),
        VisualState::FocusActivity(preset) => (
            clips.focus_activity(preset),
            RepeatAnimation::Forever,
            match preset {
                crate::focus::FocusPreset::Study => "Focus/Study",
                crate::focus::FocusPreset::Work => "Focus/Work",
            },
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
        VisualState::TouchReaction(clip) => (
            clips.touch_reaction(clip),
            RepeatAnimation::Count(1),
            clip.label(),
        ),
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
        VisualState::LimbReaction(_) => (None, RepeatAnimation::Forever, "LimbReaction"),
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

pub(super) fn clip_for_cloud(
    state: &NeraAnimationState,
    clips: &NeraAnimationClips,
    rest_pose: Option<crate::cloud::RestPose>,
) -> (Option<ClipSlot>, RepeatAnimation, &'static str) {
    if rest_pose == Some(crate::cloud::RestPose::Lying)
        && state.visual_state() == VisualState::SittingLoop
    {
        if let Some(clip) = clips.sleeping_idle.filter(|clip| !clip.duration.is_zero()) {
            return (Some(clip), RepeatAnimation::Forever, "Cloud/SleepingIdle");
        }
    }
    clip_for_state(state, clips)
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
        VisualState::LimbReaction(_)
        | VisualState::TouchReaction(_)
        | VisualState::Treat
        | VisualState::FocusCelebrate => REACTION_BLEND,
        VisualState::FocusActivity(_) => IDLE_VARIATION_BLEND,
        VisualState::Screensaver(_) => IDLE_VARIATION_BLEND,
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
    cloud: Option<Res<crate::cloud::CloudRuntime>>,
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

    let rest_pose = cloud.as_ref().and_then(|cloud| cloud.rest_pose());
    let (requested, requested_repeat, requested_label) = clip_for_cloud(&state, &clips, rest_pose);

    let (clip, repeat, label, using_requested) = if let Some(clip) = requested {
        (clip, requested_repeat, requested_label, true)
    } else {
        if visual_changed && !matches!(visual, VisualState::LimbReaction(_)) {
            warn!(
                "Nera clip unavailable for {:?}; using base fallback",
                visual
            );
        }

        let cloud_base = if rest_pose == Some(crate::cloud::RestPose::Lying)
            && state.base_pose() == BasePose::Sitting
        {
            clips.sleeping_idle.filter(|clip| !clip.duration.is_zero())
        } else {
            None
        };
        let Some(fallback) = cloud_base.or_else(|| base_fallback(&state, &clips)) else {
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
    let transition_duration = if leaving_walk
        || (matches!(visual, VisualState::LibraryAnimation(_)) && !state.auto_dance_active())
    {
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

#[cfg(test)]
mod touch_playback_tests {
    use super::*;
    use crate::animation::TemporaryAction;

    #[derive(Resource, Default)]
    struct Played(Vec<(Entity, RepeatAnimation, Duration)>);

    #[test]
    fn each_loaded_touch_slot_has_its_own_actual_duration_and_one_shot_dispatch() {
        let mut world = World::new();
        world.init_resource::<NeraAnimationClips>();
        let avatar = world.spawn_empty().id();
        for (index, clip) in TouchReactionClip::ALL.into_iter().enumerate() {
            let duration = Duration::from_millis(1300 + index as u64 * 137);
            let entity = world
                .spawn((TouchReactionLoader(clip), VrmaDuration(duration)))
                .observe(register_touch_reaction_animation)
                .id();
            world.trigger(LoadedVrma {
                vrm: avatar,
                vrma: entity,
            });
            let clips = world.resource::<NeraAnimationClips>();
            assert_eq!(clips.touch_reaction_duration(clip), Some(duration));
            assert_eq!(clips.touch_reaction_entity(clip), Some(entity));
            let mut state = NeraAnimationState::default();
            assert!(state.request_touch_reaction(clip, duration));
            let (slot, repeat, label) = clip_for_state(&state, clips);
            assert_eq!(slot.unwrap().entity, entity);
            assert_eq!(repeat, RepeatAnimation::Count(1));
            assert_eq!(label, clip.label());
            assert_eq!(transition_for_visual(state.visual_state()), REACTION_BLEND);
            assert_eq!(
                grounding_policy(state.visual_state(), None),
                GroundingPolicy::Grounded
            );
            assert_eq!(
                grounding_policy(VisualState::LibraryAnimation(entity), Some(clip.path())),
                GroundingPolicy::Grounded
            );
            state.advance(duration);
            assert_eq!(state.visual_state(), VisualState::Idle);
        }
    }

    #[test]
    fn unavailable_and_zero_duration_touch_clips_never_report_a_ready_capability() {
        let mut world = World::new();
        let mut clips = NeraAnimationClips::default();
        for clip in TouchReactionClip::ALL {
            assert_eq!(clips.touch_reaction_duration(clip), None);
            assert_eq!(clips.touch_reaction_entity(clip), None);
            let entity = world.spawn_empty().id();
            clips.touch_reactions[clip.index()] = Some(ClipSlot {
                entity,
                duration: Duration::ZERO,
            });
            assert_eq!(clips.touch_reaction_duration(clip), None);
            assert_eq!(clips.touch_reaction_entity(clip), None);
            assert!(!clips.touch_reaction_playing(
                clip,
                &NeraAnimationPlayback {
                    current_clip: Some(entity),
                    ..default()
                }
            ));
            assert!(
                clip_for_visual(VisualState::TouchReaction(clip), &clips)
                    .0
                    .is_none()
            );
        }
    }

    #[test]
    fn accepted_touch_dispatches_and_replays_then_restores_the_selected_seated_clip() {
        let mut app = App::new();
        let mut clips = touch_test_clips(app.world_mut());
        let seated = app.world_mut().spawn_empty().id();
        clips.sitting_2 = Some(ClipSlot {
            entity: seated,
            duration: Duration::from_secs(5),
        });
        let touch = TouchReactionClip::ShoulderRubbing;
        let touch_entity = clips.touch_reaction_entity(touch).unwrap();
        app.insert_resource(clips)
            .init_resource::<NeraAnimationState>()
            .init_resource::<NeraAnimationPlayback>()
            .init_resource::<DesktopEnvironment>()
            .init_resource::<Played>()
            .add_systems(Update, apply_animation_state)
            .add_observer(|trigger: On<PlayVrma>, mut played: ResMut<Played>| {
                played
                    .0
                    .push((trigger.vrma, trigger.repeat, trigger.transition_duration));
            });
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .enter_cloud_seat(SittingPose::Sitting2);
        app.update();
        assert_eq!(
            app.world().resource::<NeraAnimationPlayback>().current_clip,
            Some(seated)
        );
        for _ in 0..2 {
            assert!(
                app.world_mut()
                    .resource_mut::<NeraAnimationState>()
                    .request_touch_reaction(touch, Duration::from_secs(2))
            );
            app.update();
            assert_eq!(
                app.world().resource::<NeraAnimationPlayback>().current_clip,
                Some(touch_entity)
            );
            assert!(
                app.world()
                    .resource::<NeraAnimationClips>()
                    .touch_reaction_playing(touch, app.world().resource::<NeraAnimationPlayback>())
            );
        }
        let played = &app.world().resource::<Played>().0;
        assert_eq!(
            played
                .iter()
                .filter(|(entity, _, _)| *entity == touch_entity)
                .count(),
            2
        );
        assert!(
            played
                .iter()
                .filter(|(entity, _, _)| *entity == touch_entity)
                .all(|(_, repeat, blend)| *repeat == RepeatAnimation::Count(1)
                    && *blend == REACTION_BLEND)
        );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .advance(Duration::from_secs(2));
        app.update();
        assert_eq!(
            app.world().resource::<NeraAnimationPlayback>().current_clip,
            Some(seated)
        );
        assert_eq!(
            app.world().resource::<DesktopEnvironment>().attached_anchor,
            AvatarAnchor::Seat
        );
        assert!(
            !app.world()
                .resource::<NeraAnimationState>()
                .active_actions
                .iter()
                .any(|active| matches!(active.action, TemporaryAction::TouchReaction(_)))
        );
    }
}

#[cfg(test)]
mod cloud_sleep_tests {
    use super::*;
    use crate::{animation::TemporaryAction, cloud::RestPose};

    #[test]
    fn contact_policy_is_explicit_for_grounded_airborne_seated_and_unknown_library_clips() {
        use GroundingPolicy::*;
        let visual = VisualState::LibraryAnimation(Entity::PLACEHOLDER);
        for path in [
            IDLE_VRMA_PATH,
            WALKING_VRMA_PATH,
            HEAD_STROKE_VRMA_PATH,
            PRAYING_VRMA_PATH,
            IDLE_LOOK_AROUND_VRMA_PATH,
            IDLE_RELAX_VRMA_PATH,
            HIT_HEAD_VRMA_PATH,
            ALARM_WAVE_VRMA_PATH,
        ] {
            assert_eq!(grounding_policy(visual, Some(path)), Grounded);
        }
        for path in [
            ALARM_JUMP_VRMA_PATH,
            FLOATING_VRMA_PATH,
            FALLING_TO_LANDING_VRMA_PATH,
            SITTING_ENTER_VRMA_PATH,
            SITTING_LOOP_VRMA_PATH,
            SLEEPING_IDLE_VRMA_PATH,
            "custom/JumpingDance.vrma",
            "custom/unknown.vrma",
        ] {
            assert_eq!(grounding_policy(visual, Some(path)), Authored);
        }
        assert_eq!(grounding_policy(visual, Some(DEATH_VRMA_PATH)), BodyContact);
        assert_eq!(grounding_policy(VisualState::HeadStroke, None), Grounded);
        assert_eq!(grounding_policy(VisualState::AlarmJump, None), Authored);
    }

    #[test]
    fn headpat_body_remains_distinct_from_idle_blush() {
        assert_eq!(
            HEAD_STROKE_VRMA_PATH,
            "animations/animasi_basic/ShakeHeadNo1.vrma"
        );
        assert_ne!(HEAD_STROKE_VRMA_PATH, IDLE_BLUSH_VRMA_PATH);
        assert!(crate::nera_asset_file(HEAD_STROKE_VRMA_PATH).is_file());
        let mut world = World::new();
        let headpat = ClipSlot {
            entity: world.spawn_empty().id(),
            duration: Duration::from_secs(2),
        };
        let clips = NeraAnimationClips {
            head_stroke: Some(headpat),
            ..default()
        };
        let mut state = NeraAnimationState::default();
        state.request_one_shot(TemporaryAction::HeadStroke, headpat.duration);
        let (clip, repeat, label) = clip_for_state(&state, &clips);
        assert_eq!(clip.unwrap().entity, headpat.entity);
        assert_eq!(repeat, RepeatAnimation::Count(1));
        assert_eq!(label, "ShakeHeadNo1");
    }

    #[test]
    fn headpat_feedback_phase_reads_the_clip_clock_and_releases_on_interruption() {
        for seconds in [2, 4] {
            let mut world = World::new();
            let clip = ClipSlot {
                entity: world.spawn_empty().id(),
                duration: Duration::from_secs(seconds),
            };
            let clips = NeraAnimationClips {
                head_stroke: Some(clip),
                ..default()
            };
            let mut state = NeraAnimationState::default();
            assert_eq!(clips.head_stroke_progress(&state), None);
            state.request_one_shot(TemporaryAction::HeadStroke, clip.duration);
            assert_eq!(clips.head_stroke_progress(&state), Some(0.0));
            state.advance(clip.duration / 4);
            assert_eq!(clips.head_stroke_progress(&state), Some(0.25));
            state.request_one_shot(TemporaryAction::HeadStroke, clip.duration);
            assert_eq!(clips.head_stroke_progress(&state), Some(0.0));
            state.set_physics_falling(true);
            assert_eq!(clips.head_stroke_progress(&state), None);
            state.set_physics_falling(false);
            state.advance(clip.duration);
            assert_eq!(clips.head_stroke_progress(&state), None);
            assert_eq!(
                NeraAnimationClips::default().head_stroke_progress(&state),
                None
            );
        }
    }

    #[test]
    fn rest_loops_sleep_but_ride_support_reactions_and_detach_keep_their_own_clips() {
        let mut world = World::new();
        let mut slot = || ClipSlot {
            entity: world.spawn_empty().id(),
            duration: Duration::from_secs(2),
        };
        let idle = slot();
        let sitting = slot();
        let sleeping = slot();
        let headpat = slot();
        let clips = NeraAnimationClips {
            idle: Some(idle),
            sitting_4: Some(sitting),
            sleeping_idle: Some(sleeping),
            head_stroke: Some(headpat),
            ..default()
        };
        let mut state = NeraAnimationState::default();
        state.enter_cloud_seat(SittingPose::Sitting4);
        for pose in [None, Some(RestPose::Seated)] {
            assert_eq!(
                clip_for_cloud(&state, &clips, pose).0.unwrap().entity,
                sitting.entity
            );
        }
        let (rest, repeat, _) = clip_for_cloud(&state, &clips, Some(RestPose::Lying));
        assert_eq!(rest.unwrap().entity, sleeping.entity);
        assert_eq!(repeat, RepeatAnimation::Forever);
        state.request_one_shot(TemporaryAction::HeadStroke, headpat.duration);
        assert_eq!(
            clip_for_cloud(&state, &clips, Some(RestPose::Lying))
                .0
                .unwrap()
                .entity,
            headpat.entity
        );
        state.advance(headpat.duration);
        assert_eq!(
            clip_for_cloud(&state, &clips, Some(RestPose::Lying))
                .0
                .unwrap()
                .entity,
            sleeping.entity
        );
        state.restore_support_after_attention(false);
        assert_eq!(
            clip_for_cloud(&state, &clips, Some(RestPose::Lying))
                .0
                .unwrap()
                .entity,
            idle.entity
        );
    }

    #[test]
    fn cold_or_zero_duration_sleep_falls_back_then_switches_when_ready() {
        let mut world = World::new();
        let sitting = ClipSlot {
            entity: world.spawn_empty().id(),
            duration: Duration::from_secs(2),
        };
        let sleep = world.spawn_empty().id();
        let mut state = NeraAnimationState::default();
        state.enter_cloud_seat(SittingPose::Sitting4);
        let mut clips = NeraAnimationClips {
            sitting_4: Some(sitting),
            ..default()
        };
        for duration in [None, Some(Duration::ZERO), Some(Duration::from_secs(7))] {
            clips.sleeping_idle = duration.map(|duration| ClipSlot {
                entity: sleep,
                duration,
            });
            let expected = if duration.is_some_and(|duration| !duration.is_zero()) {
                sleep
            } else {
                sitting.entity
            };
            assert_eq!(
                clip_for_cloud(&state, &clips, Some(RestPose::Lying))
                    .0
                    .unwrap()
                    .entity,
                expected
            );
        }
        assert!(crate::nera_asset_file(SLEEPING_IDLE_VRMA_PATH).is_file());
        assert!(!crate::nera_asset_file("animations/Bookmarks/Sleeping_Idle.vrma").exists());
    }
}
