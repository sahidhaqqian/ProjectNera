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

#[cfg(test)]
use std::time::Duration;

use bevy::prelude::*;

mod playback;
mod library;
mod state;
mod systems;

pub(crate) use playback::*;
pub(crate) use library::PlayLibraryAnimation;
pub(crate) use state::*;
pub(crate) use systems::*;

pub(crate) struct NeraAnimationPlugin;

#[cfg(test)]
pub(crate) fn install_library_test_systems(app: &mut App) {
    app.init_resource::<NeraAnimationState>()
        .init_resource::<NeraAnimationClips>()
        .init_resource::<NeraAnimationPlayback>()
        .init_resource::<crate::environment::DesktopEnvironment>()
        .init_resource::<library::LibraryPlayback>()
        .add_message::<PlayLibraryAnimation>()
        .add_systems(Update, (library::sync_library_animation, apply_animation_state).chain());
}

impl Plugin for NeraAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NeraAnimationState>()
            .init_resource::<NeraAnimationClips>()
            .init_resource::<NeraAnimationPlayback>()
            .init_resource::<AlarmAnimationRuntime>()
            .init_resource::<IdleVariationRuntime>()
            .init_resource::<library::LibraryPlayback>()
            .add_message::<PlayLibraryAnimation>()
            .add_systems(
                Update,
                library::sync_library_animation
                    .before(crate::walking::update_walking)
                    .before(tick_animation_actions),
            )
            .add_message::<AlarmAnimationCommand>()
            .add_systems(
                Update,
                sync_alarm_animation.before(crate::walking::update_walking),
            );

        #[cfg(target_os = "windows")]
        app.add_systems(
            Update,
            sync_action_menu_animation.before(crate::walking::update_walking),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::NeraRng;

    #[cfg(target_os = "windows")]
    use crate::action_menu::AnimationMenuChoice;

    #[test]
    fn walking_loops_without_restarting_and_pauses_idle_variations() {
        let mut state = NeraAnimationState::default();
        assert!(state.set_walking(true));
        let revision = state.revision;
        assert!(state.set_walking(true));
        assert_eq!(state.revision, revision);
        assert!(state.suppresses_look_at());
        assert!(!state.request_idle_variation(IdleMicroAction::Relax, Duration::from_secs(3)));

        let mut runtime = runtime_with(7, Duration::from_secs(10));
        runtime.tick(
            Duration::from_secs(20),
            &mut state,
            &clips_with_all_micro_actions(),
        );
        assert_eq!(runtime.cooldown, Duration::from_secs(10));
        state.advance(Duration::from_secs(60));
        assert_eq!(state.visual_state(), VisualState::Walking);

        state.set_walking(false);
        assert_eq!(state.visual_state(), VisualState::Idle);
        assert!(!state.suppresses_look_at());
    }

    #[test]
    fn interactions_interrupt_walking_without_resuming_it_afterwards() {
        for action in [
            TemporaryAction::Floating,
            TemporaryAction::Falling,
            TemporaryAction::Praying,
            TemporaryAction::AlarmWave,
        ] {
            let mut state = NeraAnimationState::default();
            assert!(state.set_walking(true));
            state.start_persistent_action(action);
            assert!(!state.set_walking(true), "{action:?} must own the pose");
            state.finish_action(action);
            assert_eq!(state.visual_state(), VisualState::Idle);
        }

        for action in [
            TemporaryAction::HeadStroke,
            TemporaryAction::HitHead,
            TemporaryAction::HitChest,
            TemporaryAction::Death,
            TemporaryAction::Landing,
            TemporaryAction::AlarmJump,
            TemporaryAction::LibraryAnimation(Entity::PLACEHOLDER),
        ] {
            let mut state = NeraAnimationState::default();
            assert!(state.set_walking(true));
            state.request_one_shot(action, Duration::from_secs(1));
            assert!(!state.set_walking(true), "{action:?} must own the pose");
            state.advance(Duration::from_secs(2));
            assert_eq!(state.visual_state(), VisualState::Idle);
        }
    }

    #[test]
    fn walking_waits_for_idle_and_is_cancelled_by_sitting_or_attention() {
        let mut state = NeraAnimationState::default();
        assert!(state.request_idle_variation(IdleMicroAction::Relax, Duration::from_secs(3)));
        assert!(!state.set_walking(true));
        state.advance(Duration::from_secs(3));
        assert!(state.set_walking(true));

        state.observe_sitting_surface(true, Duration::from_secs(1), Duration::ZERO);
        assert!(!state.set_walking(true));
        state.advance(Duration::from_secs(1));
        assert!(!state.set_walking(true));
        state.observe_sitting_surface(false, Duration::from_secs(1), Duration::ZERO);
        assert_eq!(state.visual_state(), VisualState::Idle);

        assert!(state.set_walking(true));
        state.suspend_support_for_attention();
        state.restore_support_after_attention(false);
        assert_eq!(state.visual_state(), VisualState::Idle);
    }

    #[test]
    fn walking_requires_a_loaded_nonzero_clip_and_uses_looped_playback() {
        let mut clips = NeraAnimationClips::default();
        assert!(!clips.walking_ready());
        clips.walking = Some(clip(0));
        assert!(!clips.walking_ready());
        clips.walking = Some(clip(3));
        assert!(clips.walking_ready());
        let (slot, repeat, _) = clip_for_visual(VisualState::Walking, &clips);
        assert_eq!(slot.unwrap().duration, Duration::from_secs(3));
        assert_eq!(repeat, bevy::animation::RepeatAnimation::Forever);
    }

    #[test]
    fn floating_overrides_reaction_then_reaction_returns() {
        let mut state = NeraAnimationState::default();
        state.request_one_shot(TemporaryAction::HitHead, Duration::from_secs(1));
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

        assert!(
            !state
                .active_actions
                .iter()
                .any(|active| active.action == TemporaryAction::Floating)
        );
        assert_eq!(state.visual_state(), VisualState::Idle);
        assert_eq!(state.base_pose(), BasePose::Idle);

        // The point of the fix: reactions must win again after returning.
        state.restore_support_after_attention(false);
        state.request_one_shot(TemporaryAction::HitHead, Duration::from_secs(1));
        assert_eq!(state.visual_state(), VisualState::HitHead);
    }

    #[test]
    fn new_reaction_replaces_old_reaction() {
        let mut state = NeraAnimationState::default();
        state.request_one_shot(TemporaryAction::HitHead, Duration::from_secs(1));
        state.request_one_shot(TemporaryAction::HitChest, Duration::from_secs(1));

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

        state.observe_sitting_surface(true, Duration::from_millis(500), Duration::from_millis(400));

        assert_eq!(state.visual_state(), VisualState::SittingEnter);

        state.advance(Duration::from_millis(500));

        assert_eq!(state.visual_state(), VisualState::SittingLoop);
    }

    #[test]
    fn leaving_sitting_reveals_floating_immediately() {
        let mut state = NeraAnimationState::default();

        state.observe_sitting_surface(true, Duration::from_millis(1), Duration::from_millis(300));
        state.advance(Duration::from_millis(1));
        state.start_persistent_action(TemporaryAction::Floating);

        // Floating is hidden while sitting.
        assert_eq!(state.visual_state(), VisualState::SittingLoop);

        state.observe_sitting_surface(false, Duration::from_millis(1), Duration::from_millis(300));

        // No SittingExit detachment gate anymore.
        assert_eq!(state.visual_state(), VisualState::Floating);
    }

    #[test]
    fn timed_reaction_returns_to_idle() {
        let mut state = NeraAnimationState::default();
        state.request_one_shot(TemporaryAction::HeadStroke, Duration::from_millis(250));

        state.advance(Duration::from_millis(250));

        assert_eq!(state.visual_state(), VisualState::Idle);
    }
    #[test]
    fn third_damage_hit_triggers_death_and_resets_combo() {
        let mut state = NeraAnimationState::default();
        let hit = Duration::from_millis(500);
        let death = Duration::from_secs(2);

        state.register_damage_hit(TemporaryAction::HitHead, hit, death);
        assert_eq!(state.visual_state(), VisualState::HitHead);

        state.register_damage_hit(TemporaryAction::HitChest, hit, death);
        assert_eq!(state.visual_state(), VisualState::HitChest);

        state.register_damage_hit(TemporaryAction::HitChest, hit, death);
        assert_eq!(state.visual_state(), VisualState::Death);
        assert_eq!(state.damage_hit_count, 0);
        assert!(state.damage_combo_remaining.is_none());
    }

    #[test]
    fn damage_combo_expires() {
        let mut state = NeraAnimationState::default();
        let hit = Duration::from_millis(500);
        let death = Duration::from_secs(2);

        state.register_damage_hit(TemporaryAction::HitChest, hit, death);
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

        state.observe_sitting_surface(true, Duration::from_millis(300), Duration::from_millis(300));

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
        state.request_one_shot(TemporaryAction::AlarmJump, Duration::from_millis(500));

        assert_eq!(state.visual_state(), VisualState::AlarmJump);

        state.advance(Duration::from_millis(500));

        assert_eq!(state.visual_state(), VisualState::AlarmWave);
    }

    #[test]
    fn stopping_alarm_returns_to_base_pose() {
        let mut state = NeraAnimationState::default();

        state.start_persistent_action(TemporaryAction::AlarmWave);
        state.request_one_shot(TemporaryAction::AlarmJump, Duration::from_millis(500));

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

    fn runtime_with(seed: u64, cooldown: Duration) -> IdleVariationRuntime {
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

        assert!(state.request_idle_variation(IdleMicroAction::LookAround, Duration::from_secs(4),));
        assert!(matches!(state.visual_state(), VisualState::IdleMicro(_)));

        state.request_one_shot(TemporaryAction::HeadStroke, Duration::from_secs(2));
        assert_eq!(state.visual_state(), VisualState::HeadStroke);

        // The fidget must be *gone*, not merely outranked: resuming its last
        // two seconds after a head pat would read as a glitch.
        state.advance(Duration::from_secs(3));
        assert_eq!(state.visual_state(), VisualState::Idle);
    }

    #[test]
    fn a_fidget_never_pulls_nera_out_of_a_seat() {
        let mut state = NeraAnimationState::default();

        assert!(state.request_idle_variation(IdleMicroAction::Relax, Duration::from_secs(4),));

        // Sat down mid-fidget. Micro-action clips are standing clips, so it
        // stays logically alive but must lose the pose.
        state.set_base_pose(BasePose::Sitting);

        assert_eq!(state.visual_state(), VisualState::SittingLoop);
        assert!(
            state
                .active_actions
                .iter()
                .any(|active| active.action.is_idle_variation())
        );
    }

    #[test]
    fn request_idle_variation_is_rejected_when_nera_is_not_idle() {
        let mut state = NeraAnimationState::default();
        state.start_persistent_action(TemporaryAction::Floating);

        assert!(!state.request_idle_variation(IdleMicroAction::Sleepy, Duration::from_secs(3),));
        assert_eq!(state.visual_state(), VisualState::Floating);
    }

    #[test]
    fn scheduler_waits_out_its_cooldown_before_fidgeting() {
        let clips = clips_with_all_micro_actions();
        let mut state = NeraAnimationState::default();
        let mut runtime = runtime_with(5, Duration::from_secs(10));

        runtime.tick(Duration::from_millis(500), &mut state, &clips);

        assert_eq!(state.visual_state(), VisualState::Idle);
        assert!(runtime.cooldown < Duration::from_secs(10));

        for _ in 0..24 {
            runtime.tick(Duration::from_millis(500), &mut state, &clips);
        }

        assert!(matches!(state.visual_state(), VisualState::IdleMicro(_)));
    }

    #[test]
    fn scheduler_does_not_count_down_while_nera_is_busy() {
        let clips = clips_with_all_micro_actions();
        let mut state = NeraAnimationState::default();
        state.start_persistent_action(TemporaryAction::Floating);

        let mut runtime = runtime_with(5, Duration::from_secs(10));

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

        assert!(matches!(state.visual_state(), VisualState::IdleMicro(_)));
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

            let VisualState::IdleMicro(action) = state.visual_state() else {
                panic!("expected a fidget, got {:?}", state.visual_state());
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
            let picked = pick_idle_micro(&clips, None, &mut rng).expect("one clip is loaded");

            assert_eq!(picked.0, IdleMicroAction::Sleepy);
            assert_eq!(picked.1, Duration::from_secs(5));
        }
    }

    #[test]
    fn pick_idle_micro_returns_none_when_nothing_is_usable() {
        let mut rng = NeraRng::seeded(3);

        assert!(pick_idle_micro(&NeraAnimationClips::default(), None, &mut rng,).is_none());

        // A zero duration means the VRMA reported no length, which
        // `request_one_shot` would reject anyway.
        let zero_length = NeraAnimationClips {
            idle_relax: Some(clip(0)),
            ..Default::default()
        };

        assert!(pick_idle_micro(&zero_length, None, &mut rng).is_none());
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

        let picked = pick_idle_micro(&clips, Some(IdleMicroAction::Blush), &mut rng);

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
            transition_for_visual(VisualState::IdleMicro(IdleMicroAction::LookAround)),
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
