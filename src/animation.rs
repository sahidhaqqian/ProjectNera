//! Project Nera — Phase 5.4 serious animation controller.
//!
//! Design goals:
//! - `animasi_basic/` is the small runtime animation set.
//! - Base pose, support transitions, and temporary interactions are separate.
//! - One-shot lifetimes use the VRMA's real `VrmaDuration` instead of guessed
//!   millisecond constants.
//! - Each attachment chooses one sitting position, then loops it until detach.
//! - Dragging uses Floating.
//! - Head/chest clicks and head strokes are semantic reactions.
//! - Praying follows the menu lifecycle only while Nera is standing.

#[cfg(test)]
use std::time::Duration;

use bevy::prelude::*;

mod library;
mod playback;
mod state;
mod systems;
mod walking_loop;

pub(crate) use library::{LibraryAnimationError, PlayLibraryAnimation};
pub(crate) use playback::*;
pub(crate) use state::*;
pub(crate) use systems::*;
pub(crate) use walking_loop::smooth_walking_cycle;

pub(crate) struct NeraAnimationPlugin;

/// Clip entities and their graph indices belong to one rig. Keep the current
/// seated/standing behavior, but discard every reference to the replaced rig.
pub(crate) fn reset_for_character_change(world: &mut World) {
    world.insert_resource(NeraAnimationClips::default());
    world.insert_resource(NeraAnimationPlayback::default());
    world.insert_resource(library::LibraryPlayback::default());
    world.insert_resource(walking_loop::WalkingCycle::default());
    world
        .resource_mut::<NeraAnimationState>()
        .active_actions
        .retain(|active| !matches!(active.action, TemporaryAction::LibraryAnimation(_)));
}

pub(crate) fn character_animation_ready(world: &World) -> bool {
    let (requested, _, _) = playback::clip_for_state(
        world.resource::<NeraAnimationState>(),
        world.resource::<NeraAnimationClips>(),
    );
    requested.is_some_and(|clip| {
        world.resource::<NeraAnimationPlayback>().current_clip == Some(clip.entity)
    })
}

#[cfg(test)]
mod character_swap_tests {
    use super::*;

    #[test]
    fn character_swap_preserves_seated_pose_and_rejects_standing_fallback_reveal() {
        let mut world = World::new();
        let idle = world.spawn_empty().id();
        let seated = world.spawn_empty().id();
        let mut state = NeraAnimationState::default();
        state.set_base_pose(BasePose::Sitting);
        state.sitting_pose = Some(SittingPose::Sitting2);
        world.insert_resource(state);
        world.insert_resource(NeraAnimationClips {
            idle: Some(playback::ClipSlot {
                entity: idle,
                duration: Duration::from_secs(1),
            }),
            sitting_2: Some(playback::ClipSlot {
                entity: seated,
                duration: Duration::from_secs(1),
            }),
            ..default()
        });
        world.init_resource::<NeraAnimationPlayback>();
        world.resource_mut::<NeraAnimationPlayback>().current_clip = Some(idle);
        assert!(!character_animation_ready(&world));
        world.resource_mut::<NeraAnimationPlayback>().current_clip = Some(seated);
        assert!(character_animation_ready(&world));
        reset_for_character_change(&mut world);
        assert_eq!(
            world.resource::<NeraAnimationState>().base_pose(),
            BasePose::Sitting,
        );
        assert_eq!(
            world.resource::<NeraAnimationState>().sitting_pose,
            Some(SittingPose::Sitting2),
        );
        assert!(!character_animation_ready(&world));
    }
}

#[cfg(test)]
pub(crate) fn install_library_test_systems(app: &mut App) {
    app.init_resource::<NeraAnimationState>()
        .init_resource::<NeraAnimationClips>()
        .init_resource::<NeraAnimationPlayback>()
        .init_resource::<crate::environment::DesktopEnvironment>()
        .init_resource::<library::LibraryPlayback>()
        .add_message::<PlayLibraryAnimation>()
        .add_message::<LibraryAnimationError>()
        .add_systems(
            Update,
            (library::sync_library_animation, apply_animation_state).chain(),
        );
}

impl Plugin for NeraAnimationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NeraAnimationState>()
            .init_resource::<NeraAnimationClips>()
            .init_resource::<NeraAnimationPlayback>()
            .init_resource::<AlarmAnimationRuntime>()
            .init_resource::<library::LibraryPlayback>()
            .init_resource::<walking_loop::WalkingCycle>()
            .add_systems(
                PostUpdate,
                walking_loop::seek_walking_cycle
                    .after(bevy::animation::advance_animations)
                    .before(bevy::animation::animate_targets)
                    .in_set(bevy::app::AnimationSystems),
            )
            .add_systems(
                PostUpdate,
                smooth_walking_cycle
                    .after(bevy::app::AnimationSystems)
                    .before(bevy_vrm1::prelude::VrmSystemSets::Constraints),
            )
            .add_message::<PlayLibraryAnimation>()
            .add_message::<LibraryAnimationError>()
            .add_systems(
                Update,
                library::sync_library_animation
                    .before(crate::startup_entrance::prepare_startup_entrance)
                    .before(crate::pet_behavior::update_pet_behavior)
                    .before(tick_animation_actions),
            )
            .add_message::<AlarmAnimationCommand>()
            .add_systems(
                Update,
                sync_alarm_animation
                    .before(crate::startup_entrance::prepare_startup_entrance)
                    .before(crate::pet_behavior::update_pet_behavior),
            );

        #[cfg(target_os = "windows")]
        app.add_systems(
            Update,
            sync_action_menu_animation
                .before(crate::startup_entrance::prepare_startup_entrance)
                .before(crate::pet_behavior::update_pet_behavior),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    use crate::action_menu::AnimationMenuChoice;

    #[test]
    fn startup_greeting_runs_once_then_releases_idle_and_gaze() {
        let mut state = NeraAnimationState::default();
        assert!(!state.request_startup_greeting(Duration::ZERO));
        assert!(state.request_startup_greeting(Duration::from_secs(3)));
        let revision = state.revision;
        assert!(!state.request_startup_greeting(Duration::from_secs(3)));
        assert_eq!(state.revision, revision, "the same wave must not restart");
        assert_eq!(state.visual_state(), VisualState::StartupGreeting);
        assert!(state.suppresses_look_at());
        assert!(!state.set_walking(true));
        assert!(!state.request_idle_variation(IdleMicroAction::Relax, Duration::from_secs(2)));

        state.advance(Duration::from_secs(2));
        assert_eq!(state.visual_state(), VisualState::StartupGreeting);
        state.advance(Duration::from_secs(1));
        assert_eq!(state.visual_state(), VisualState::Idle);
        assert!(!state.suppresses_look_at());
        state.advance(Duration::from_secs(60));
        assert_eq!(state.visual_state(), VisualState::Idle);
    }

    #[test]
    fn startup_greeting_is_rejected_until_standing_idle() {
        let mut state = NeraAnimationState::default();
        state.set_physics_falling(true);
        assert!(!state.request_startup_greeting(Duration::from_secs(3)));
        state.land_from_fall(Duration::from_secs(2));
        assert!(!state.request_startup_greeting(Duration::from_secs(3)));
        state.advance(Duration::from_secs(2));
        assert!(state.set_walking(true));
        assert!(!state.request_startup_greeting(Duration::from_secs(3)));
        state.set_walking(false);
        state.set_base_pose(BasePose::Sitting);
        assert!(!state.request_startup_greeting(Duration::from_secs(3)));
        state.set_base_pose(BasePose::Idle);
        assert!(state.request_startup_greeting(Duration::from_secs(3)));
    }

    #[test]
    fn interactions_cancel_startup_greeting_without_resuming_its_tail() {
        for action in [
            TemporaryAction::Floating,
            TemporaryAction::Falling,
            TemporaryAction::Praying,
            TemporaryAction::AlarmWave,
        ] {
            let mut state = NeraAnimationState::default();
            assert!(state.request_startup_greeting(Duration::from_secs(20)));
            state.start_persistent_action(action);
            assert!(!state.request_startup_greeting(Duration::from_secs(20)));
            state.finish_action(action);
            assert_eq!(state.visual_state(), VisualState::Idle, "{action:?}");
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
            assert!(state.request_startup_greeting(Duration::from_secs(20)));
            state.request_one_shot(action, Duration::from_secs(1));
            assert!(!state.request_startup_greeting(Duration::from_secs(20)));
            state.advance(Duration::from_secs(1));
            assert_eq!(state.visual_state(), VisualState::Idle, "{action:?}");
        }
    }

    #[test]
    fn startup_greeting_cancels_for_attention_sitting_and_unavailable_reactions() {
        let mut state = NeraAnimationState::default();
        assert!(state.request_startup_greeting(Duration::from_secs(20)));
        state.suspend_support_for_attention();
        state.restore_support_after_attention(false);
        assert_eq!(state.visual_state(), VisualState::Idle);

        assert!(state.request_startup_greeting(Duration::from_secs(20)));
        state.observe_sitting_surface(true, Duration::from_secs(1), Duration::ZERO);
        state.advance(Duration::from_secs(1));
        state.observe_sitting_surface(false, Duration::from_secs(1), Duration::ZERO);
        assert_eq!(state.visual_state(), VisualState::Idle);

        assert!(state.request_startup_greeting(Duration::from_secs(20)));
        state.request_one_shot(TemporaryAction::HitHead, Duration::ZERO);
        assert_eq!(state.visual_state(), VisualState::Idle);

        assert!(state.request_startup_greeting(Duration::from_secs(20)));
        state.cancel_startup_greeting();
        assert_eq!(state.visual_state(), VisualState::Idle);
    }

    #[test]
    fn startup_and_alarm_share_the_wave_with_different_repeat_modes() {
        let clips = NeraAnimationClips {
            alarm_wave: Some(clip(3)),
            ..Default::default()
        };
        let (greeting, greeting_repeat, _) = clip_for_visual(VisualState::StartupGreeting, &clips);
        let (alarm, alarm_repeat, _) = clip_for_visual(VisualState::AlarmWave, &clips);
        assert_eq!(greeting.unwrap().entity, alarm.unwrap().entity);
        assert_eq!(greeting_repeat, bevy::animation::RepeatAnimation::Count(1));
        assert_eq!(alarm_repeat, bevy::animation::RepeatAnimation::Forever);
        assert_eq!(clips.startup_greeting_duration(), Duration::from_secs(3));
    }

    #[test]
    fn startup_readiness_requires_the_whole_loaded_sequence() {
        let mut clips = NeraAnimationClips::default();
        assert_eq!(clips.startup_greeting_duration(), Duration::ZERO);
        assert!(!clips.startup_intro_ready());
        clips.idle = Some(clip(3));
        assert!(!clips.startup_intro_ready());
        clips.floating = Some(clip(3));
        assert!(!clips.startup_intro_ready());
        clips.falling_to_landing = Some(clip(2));
        assert!(!clips.startup_intro_ready());
        clips.alarm_wave = Some(clip(0));
        assert!(!clips.startup_intro_ready());
        clips.alarm_wave = Some(clip(3));
        assert!(clips.startup_intro_ready());
        clips.floating = Some(clip(0));
        assert!(!clips.startup_intro_ready());
    }

    #[test]
    fn walking_loops_without_restarting_and_pauses_idle_variations() {
        let mut state = NeraAnimationState::default();
        assert!(state.set_walking(true));
        let revision = state.revision;
        assert!(state.set_walking(true));
        assert_eq!(state.revision, revision);
        assert!(state.suppresses_look_at());
        assert!(!state.request_idle_variation(IdleMicroAction::Relax, Duration::from_secs(3)));

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

    #[test]
    fn action_menu_preserves_seated_idle_enter_fidgets_and_dodge() {
        for visual in [
            VisualState::SittingLoop,
            VisualState::SittingEnter,
            VisualState::IdleMicro(IdleMicroAction::SittingRubbingArm),
            VisualState::DodgeWhileSitting,
        ] {
            let mut state = NeraAnimationState::default();
            let enter_duration = if visual == VisualState::SittingEnter {
                Duration::from_secs(2)
            } else {
                Duration::ZERO
            };
            state.observe_sitting_surface(true, enter_duration, Duration::ZERO);
            match visual {
                VisualState::IdleMicro(action) => {
                    assert!(state.request_idle_variation(action, Duration::from_secs(3)));
                }
                VisualState::DodgeWhileSitting => {
                    state.request_sitting_dodge(Duration::from_secs(3));
                }
                _ => {}
            }
            assert_eq!(state.visual_state(), visual);

            state.set_praying_menu(true);
            assert_eq!(state.visual_state(), visual);
            state.set_praying_menu(false);
            assert_eq!(state.visual_state(), visual);
            assert_eq!(state.base_pose(), BasePose::Sitting);
            assert!(state.was_on_sitting_surface);
        }
    }

    #[test]
    fn sitting_cancels_an_open_menus_praying_without_resuming_it_after_detach() {
        let mut state = NeraAnimationState::default();
        state.set_praying_menu(true);
        assert_eq!(state.visual_state(), VisualState::Praying);

        state.observe_sitting_surface(true, Duration::from_secs(2), Duration::ZERO);
        assert_eq!(state.visual_state(), VisualState::SittingEnter);
        state.advance(Duration::from_secs(2));
        assert_eq!(state.visual_state(), VisualState::SittingLoop);
        state.observe_sitting_surface(false, Duration::ZERO, Duration::ZERO);
        assert_eq!(state.visual_state(), VisualState::Idle);
        state.set_praying_menu(false);
        assert_eq!(state.visual_state(), VisualState::Idle);

        // A fresh open while standing still uses the original menu pose.
        state.set_praying_menu(true);
        assert_eq!(state.visual_state(), VisualState::Praying);
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
            idle_catwalk_twist: Some(clip(3)),
            idle_bored: Some(clip(4)),
            idle_angry_gesture: Some(clip(3)),
            idle_sitting_rubbing_arm: Some(clip(3)),
            ..Default::default()
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

        // Sitting down cancels the standing fidget, including its hidden tail.
        state.set_base_pose(BasePose::Sitting);

        assert_eq!(state.visual_state(), VisualState::SittingLoop);
        assert!(
            !state
                .active_actions
                .iter()
                .any(|active| active.action.is_idle_variation())
        );
        state.set_base_pose(BasePose::Idle);
        assert_eq!(state.visual_state(), VisualState::Idle);
    }

    #[test]
    fn idle_variations_require_their_own_pose_and_end_on_detach() {
        for action in IdleMicroAction::ALL {
            let mut state = NeraAnimationState::default();
            let opposite = if action.base_pose() == BasePose::Idle {
                BasePose::Sitting
            } else {
                BasePose::Idle
            };
            state.set_base_pose(opposite);
            assert!(!state.request_idle_variation(action, Duration::from_secs(3)));
            state.set_base_pose(action.base_pose());
            assert!(state.request_idle_variation(action, Duration::from_secs(3)));
            assert_eq!(state.visual_state(), VisualState::IdleMicro(action));
            state.advance(Duration::from_secs(3));
            assert!(matches!(
                state.visual_state(),
                VisualState::Idle | VisualState::SittingLoop
            ));
            assert!(state.request_idle_variation(action, Duration::from_secs(3)));
            state.set_base_pose(opposite);
            state.set_base_pose(action.base_pose());
            assert!(!matches!(state.visual_state(), VisualState::IdleMicro(_)));
        }
        for pose in [BasePose::Idle, BasePose::Sitting] {
            assert!(
                IdleMicroAction::for_pose(pose)
                    .iter()
                    .all(|action| action.base_pose() == pose)
            );
        }
    }

    #[test]
    fn seated_head_and_chest_hits_dodge_without_detaching_or_counting_toward_death() {
        use crate::environment::{
            AvatarAnchor, DesktopEnvironment, PlacementState, TaskbarAttachment, TaskbarEdge,
        };
        use crate::interaction::{
            BodyPart, NeraClicked, NeraDragStarted, NeraHeadPatted, NeraPointerPosition,
            NeraReleased,
        };
        use crate::platform::NativeMonitorHandle;

        let mut app = App::new();
        app.init_resource::<NeraAnimationState>()
            .init_resource::<NeraAnimationPlayback>()
            .insert_resource(NeraAnimationClips {
                idle: Some(clip(1)),
                sitting_enter: Some(clip(2)),
                sitting_loop: Some(clip(2)),
                hit_head: Some(clip(1)),
                hit_chest: Some(clip(1)),
                death: Some(clip(3)),
                dodge_while_sitting: Some(clip(2)),
                ..Default::default()
            })
            .init_resource::<DesktopEnvironment>()
            .add_message::<NeraClicked>()
            .add_message::<NeraDragStarted>()
            .add_message::<NeraHeadPatted>()
            .add_message::<NeraReleased>()
            .add_systems(
                Update,
                (
                    sync_interaction_animation_actions,
                    sync_environment_animation_state,
                    apply_animation_state,
                )
                    .chain(),
            );
        let pending = PlacementState::PendingTaskbar {
            monitor: NativeMonitorHandle::from_raw(1),
            edge: TaskbarEdge::Bottom,
            settle_frames: 1,
        };
        app.world_mut()
            .resource_mut::<DesktopEnvironment>()
            .placement = pending;

        // An early hit must leave the enter clip and calibration intact,
        // including when attachment arrives before animation's support sync.
        let click = NeraClicked {
            pointer: NeraPointerPosition {
                screen: IVec2::ZERO,
                local: Vec2::ZERO,
            },
            body_part: BodyPart::Head,
        };
        app.world_mut().write_message(click);
        app.update();
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::SittingEnter
        );
        assert_eq!(
            app.world().resource::<DesktopEnvironment>().placement,
            pending
        );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .advance(Duration::from_secs(2));
        app.world_mut().write_message(click);
        app.update();
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::SittingLoop
        );
        assert_eq!(
            app.world().resource::<DesktopEnvironment>().placement,
            pending
        );

        let placement = PlacementState::Taskbar(TaskbarAttachment {
            monitor: NativeMonitorHandle::from_raw(1),
            edge: TaskbarEdge::Bottom,
            along_offset: 100,
        });
        app.world_mut()
            .resource_mut::<DesktopEnvironment>()
            .placement = placement;

        // Only fully seated hits dodge, keeping the same support and never
        // advancing the standing damage combo.
        for body_part in [BodyPart::Head, BodyPart::Torso, BodyPart::Head] {
            app.world_mut().write_message(NeraClicked {
                pointer: NeraPointerPosition {
                    screen: IVec2::ZERO,
                    local: Vec2::ZERO,
                },
                body_part,
            });
            app.update();
            let state = app.world().resource::<NeraAnimationState>();
            assert_eq!(state.visual_state(), VisualState::DodgeWhileSitting);
            assert_eq!(state.damage_hit_count, 0);
            assert!(state.damage_combo_remaining.is_none());
            let environment = app.world().resource::<DesktopEnvironment>();
            assert_eq!(environment.placement, placement);
            assert_eq!(environment.attached_anchor, AvatarAnchor::Seat);
        }
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .advance(Duration::from_secs(2));
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::SittingLoop
        );

        // The original mixed head/chest combo still works after standing up.
        app.world_mut()
            .resource_mut::<DesktopEnvironment>()
            .placement = PlacementState::Free;
        app.update();
        for body_part in [BodyPart::Head, BodyPart::Torso, BodyPart::Head] {
            app.world_mut().write_message(NeraClicked {
                pointer: NeraPointerPosition {
                    screen: IVec2::ZERO,
                    local: Vec2::ZERO,
                },
                body_part,
            });
            app.update();
        }
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::Death
        );
    }

    #[test]
    fn seated_dodge_yields_to_drag_and_never_resumes_after_detach() {
        let mut state = NeraAnimationState::default();
        state.observe_sitting_surface(true, Duration::from_secs(2), Duration::ZERO);
        state.request_sitting_dodge(Duration::from_secs(3));
        assert_eq!(state.visual_state(), VisualState::SittingEnter);
        state.advance(Duration::from_secs(2));
        assert_eq!(state.visual_state(), VisualState::SittingLoop);
        state.request_sitting_dodge(Duration::from_secs(3));
        assert_eq!(state.visual_state(), VisualState::DodgeWhileSitting);
        assert!(state.suppresses_look_at());
        state.start_persistent_action(TemporaryAction::Floating);
        state.observe_sitting_surface(false, Duration::ZERO, Duration::ZERO);
        assert_eq!(state.visual_state(), VisualState::Floating);
        state.finish_action(TemporaryAction::Floating);
        state.observe_sitting_surface(true, Duration::ZERO, Duration::ZERO);
        assert_eq!(state.visual_state(), VisualState::SittingLoop);

        // A clip that has not loaded yet leaves the seated base pose intact.
        state.request_sitting_dodge(Duration::ZERO);
        assert_eq!(state.visual_state(), VisualState::SittingLoop);
        assert!(state.active_actions.is_empty());
    }

    #[test]
    fn request_idle_variation_is_rejected_when_nera_is_not_idle() {
        let mut state = NeraAnimationState::default();
        state.start_persistent_action(TemporaryAction::Floating);

        assert!(!state.request_idle_variation(IdleMicroAction::Sleepy, Duration::from_secs(3),));
        assert_eq!(state.visual_state(), VisualState::Floating);
    }

    #[test]
    fn idle_capabilities_exclude_missing_and_zero_length_clips() {
        let mut clips = NeraAnimationClips::default();
        for action in IdleMicroAction::ALL {
            assert_eq!(clips.idle_micro_duration(action), None);
        }
        clips.idle_relax = Some(clip(0));
        assert_eq!(clips.idle_micro_duration(IdleMicroAction::Relax), None);
        clips.idle_relax = Some(clip(4));
        assert_eq!(
            clips.idle_micro_duration(IdleMicroAction::Relax),
            Some(Duration::from_secs(4)),
        );

        let mut state = NeraAnimationState::default();
        assert!(!state.request_idle_variation(IdleMicroAction::Relax, Duration::ZERO));
        assert_eq!(state.visual_state(), VisualState::Idle);
        assert!(state.active_actions.is_empty());
    }

    #[test]
    fn optional_personality_intents_fall_back_without_reserving_the_pose() {
        let mut state = NeraAnimationState::default();
        let mut clips = NeraAnimationClips::default();
        for action in [
            PersonalityAction::Stretch,
            PersonalityAction::Yawn,
            PersonalityAction::LookAround,
            PersonalityAction::SitFidget,
            PersonalityAction::Turn,
            PersonalityAction::StandUp,
            PersonalityAction::SitDown,
        ] {
            assert_eq!(clips.personality_duration(action), None);
            assert!(!state.request_personality_action(action, &clips));
            assert_eq!(state.visual_state(), VisualState::Idle);
            assert!(state.active_actions.is_empty());
        }
        clips.idle_look_around = Some(clip(0));
        assert!(!state.request_personality_action(PersonalityAction::LookAround, &clips));
        assert_eq!(state.revision, 0);

        clips.idle_look_around = Some(clip(3));
        assert!(state.request_personality_action(PersonalityAction::LookAround, &clips));
        assert_eq!(
            state.visual_state(),
            VisualState::IdleMicro(IdleMicroAction::LookAround)
        );
        state.advance(Duration::from_secs(3));
        assert_eq!(state.visual_state(), VisualState::Idle);
    }

    #[test]
    fn cancelling_idle_variation_keeps_interactions_and_never_resumes_a_tail() {
        let mut state = NeraAnimationState::default();
        assert!(state.request_idle_variation(IdleMicroAction::Relax, Duration::from_secs(4)));
        state.cancel_idle_variation();
        assert_eq!(state.visual_state(), VisualState::Idle);
        state.cancel_idle_variation();
        state.advance(Duration::from_secs(1));
        assert_eq!(state.visual_state(), VisualState::Idle);

        state.start_persistent_action(TemporaryAction::Floating);
        state.cancel_idle_variation();
        assert_eq!(state.visual_state(), VisualState::Floating);
        let clips = clips_with_all_micro_actions();
        assert!(!state.request_personality_action(PersonalityAction::LookAround, &clips));
        assert!(!state.request_personality_action(PersonalityAction::Stretch, &clips));
        state.finish_action(TemporaryAction::Floating);
        assert_eq!(state.visual_state(), VisualState::Idle);
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
        assert_eq!(
            IdleMicroAction::ALL.len(),
            IdleMicroAction::STANDING.len() + IdleMicroAction::SITTING.len()
        );
    }

    #[test]
    fn sitting_position_changes_only_on_new_attachment_and_survives_reactions() {
        use crate::environment::{
            DesktopEnvironment, PlacementState, TaskbarAttachment, TaskbarEdge,
        };
        use crate::platform::NativeMonitorHandle;
        use bevy::animation::RepeatAnimation;

        #[derive(Resource, Default)]
        struct PlayedClip(Option<Entity>);

        let mut app = App::new();
        let slots: Vec<_> = (0..5)
            .map(|_| ClipSlot {
                entity: app.world_mut().spawn_empty().id(),
                duration: Duration::from_secs(2),
            })
            .collect();
        app.init_resource::<DesktopEnvironment>()
            .init_resource::<NeraAnimationPlayback>()
            .init_resource::<PlayedClip>()
            .add_observer(
                |event: On<bevy_vrm1::prelude::PlayVrma>, mut played: ResMut<PlayedClip>| {
                    played.0 = Some(event.vrma);
                },
            )
            .insert_resource(NeraAnimationState {
                sitting_rng: crate::rng::NeraRng::seeded(19),
                ..Default::default()
            })
            .insert_resource(NeraAnimationClips {
                sitting_loop: Some(slots[0]),
                sitting_2: Some(slots[1]),
                sitting_4: Some(slots[2]),
                idle_sitting_rubbing_arm: Some(slots[3]),
                dodge_while_sitting: Some(slots[4]),
                ..Default::default()
            })
            .add_systems(
                Update,
                (sync_environment_animation_state, apply_animation_state).chain(),
            );
        let pending = PlacementState::PendingTaskbar {
            monitor: NativeMonitorHandle::from_raw(1),
            edge: TaskbarEdge::Bottom,
            settle_frames: 1,
        };
        let committed = PlacementState::Taskbar(TaskbarAttachment {
            monitor: NativeMonitorHandle::from_raw(1),
            edge: TaskbarEdge::Bottom,
            along_offset: 100,
        });
        let mut previous = None;
        let mut seen = Vec::new();
        for _ in 0..30 {
            app.world_mut()
                .resource_mut::<DesktopEnvironment>()
                .placement = pending;
            app.update();
            let selected = app
                .world()
                .resource::<NeraAnimationState>()
                .sitting_pose
                .unwrap();
            assert_ne!(
                Some(selected),
                previous,
                "a new seat should vary the position"
            );
            previous = Some(selected);
            if !seen.contains(&selected) {
                seen.push(selected);
            }
            app.world_mut()
                .resource_mut::<DesktopEnvironment>()
                .placement = committed;
            app.update();

            // Many timer intervals, menu interaction, fidget, hit, and Attention
            // must all return to the same chosen loop on this attachment.
            for _ in 0..10 {
                let mut state = app.world_mut().resource_mut::<NeraAnimationState>();
                state.advance(Duration::from_secs(60));
                state.set_praying_menu(true);
                assert_eq!(state.visual_state(), VisualState::SittingLoop);
                state.set_praying_menu(false);
                assert!(state.request_idle_variation(
                    IdleMicroAction::SittingRubbingArm,
                    Duration::from_secs(1)
                ));
                app.update();
                assert_eq!(
                    app.world().resource::<PlayedClip>().0,
                    Some(slots[3].entity)
                );
                let mut state = app.world_mut().resource_mut::<NeraAnimationState>();
                state.advance(Duration::from_secs(1));
                state.request_sitting_dodge(Duration::from_secs(1));
                app.update();
                assert_eq!(
                    app.world().resource::<PlayedClip>().0,
                    Some(slots[4].entity)
                );
                let mut state = app.world_mut().resource_mut::<NeraAnimationState>();
                state.advance(Duration::from_secs(1));
                state.suspend_support_for_attention();
                state.restore_support_after_attention(true);
                app.update();
                let state = app.world().resource::<NeraAnimationState>();
                assert_eq!(state.sitting_pose, Some(selected));
                assert_eq!(state.visual_state(), VisualState::SittingLoop);
                let clips = app.world().resource::<NeraAnimationClips>();
                let expected = clips.sitting_pose(selected).unwrap().entity;
                assert_eq!(clip_for_state(state, clips).1, RepeatAnimation::Forever);
                assert_eq!(app.world().resource::<PlayedClip>().0, Some(expected));
            }
            app.world_mut()
                .resource_mut::<DesktopEnvironment>()
                .placement = PlacementState::Free;
            app.update();
            assert_eq!(
                app.world().resource::<NeraAnimationState>().visual_state(),
                VisualState::Idle
            );
        }
        assert_eq!(
            seen.len(),
            3,
            "all three sitting positions must be reachable"
        );
    }

    #[test]
    fn sitting_position_ignores_unloaded_clips_and_does_not_switch_when_they_arrive() {
        use crate::environment::{DesktopEnvironment, PlacementState, TaskbarEdge};
        use crate::platform::NativeMonitorHandle;
        let mut app = App::new();
        app.init_resource::<NeraAnimationState>()
            .init_resource::<DesktopEnvironment>()
            .insert_resource(NeraAnimationClips {
                sitting_loop: Some(clip(2)),
                sitting_2: Some(clip(0)), // Not usable yet.
                ..Default::default()
            })
            .add_systems(Update, sync_environment_animation_state);
        let seat = PlacementState::PendingTaskbar {
            monitor: NativeMonitorHandle::from_raw(1),
            edge: TaskbarEdge::Bottom,
            settle_frames: 1,
        };
        app.world_mut()
            .resource_mut::<DesktopEnvironment>()
            .placement = seat;
        app.update();
        assert_eq!(
            app.world().resource::<NeraAnimationState>().sitting_pose,
            Some(SittingPose::Default)
        );
        app.world_mut()
            .resource_mut::<NeraAnimationClips>()
            .sitting_2 = Some(clip(3));
        app.update();
        assert_eq!(
            app.world().resource::<NeraAnimationState>().sitting_pose,
            Some(SittingPose::Default)
        );
        app.world_mut()
            .resource_mut::<DesktopEnvironment>()
            .placement = PlacementState::Free;
        app.update();
        app.world_mut()
            .resource_mut::<DesktopEnvironment>()
            .placement = seat;
        app.update();
        assert_eq!(
            app.world().resource::<NeraAnimationState>().sitting_pose,
            Some(SittingPose::Sitting2)
        );
    }
}
