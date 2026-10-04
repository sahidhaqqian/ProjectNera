use super::*;
use crate::{
    animation::{
        TemporaryAction, VisualState, sync_interaction_animation_actions, touch_test_clips,
    },
    character::CharacterImportStatus,
    environment::DesktopEnvironment,
    interaction::{
        DesktopPetInteraction, InteractionState, NeraClicked, NeraDragStarted, NeraHeadPatted,
        NeraPointerPosition, NeraReleased,
    },
};

fn app() -> App {
    let mut app = App::new();
    let mut clips = touch_test_clips(app.world_mut());
    clips.hit_head = Some(super::super::playback::ClipSlot {
        entity: app.world_mut().spawn_empty().id(),
        duration: Duration::from_secs(2),
    });
    clips.hit_chest = clips.hit_head;
    clips.head_stroke = clips.hit_head;
    app.init_resource::<Time>()
        .init_resource::<TouchReactionRuntime>()
        .init_resource::<NeraAnimationState>()
        .init_resource::<DesktopEnvironment>()
        .init_resource::<DesktopPetInteraction>()
        .init_resource::<CharacterImportStatus>()
        .insert_resource(clips)
        .add_message::<NeraClicked>()
        .add_message::<NeraDragStarted>()
        .add_message::<NeraHeadPatted>()
        .add_message::<NeraReleased>()
        .add_message::<TouchReactionAccepted>()
        .add_systems(Update, sync_interaction_animation_actions);
    app
}

fn pointer() -> NeraPointerPosition {
    NeraPointerPosition {
        screen: IVec2::ZERO,
        local: Vec2::ZERO,
    }
}
fn click(app: &mut App, part: BodyPart) -> Option<TouchReactionAccepted> {
    app.world_mut().write_message(NeraClicked {
        body_part: part,
        pointer: pointer(),
    });
    app.update();
    app.world_mut()
        .resource_mut::<Messages<TouchReactionAccepted>>()
        .drain()
        .last()
}
fn wait(app: &mut App, delta: Duration) {
    app.world_mut().resource_mut::<Time>().advance_by(delta);
}

#[test]
fn hand_sequence_uses_normal_pool_no_repeat_then_escalation_and_restarts() {
    let mut app = app();
    let first = click(&mut app, BodyPart::LeftHand).unwrap();
    assert!(TouchReactionClip::NORMAL_HAND.contains(&first.clip));
    assert_eq!(first.hand_step, 1);
    assert!(!first.angry_mark);
    wait(&mut app, TOUCH_REACTION_COOLDOWN);
    let second = click(&mut app, BodyPart::RightHand).unwrap();
    assert!(TouchReactionClip::NORMAL_HAND.contains(&second.clip));
    assert_ne!(first.clip, second.clip);
    for step in 3..=7 {
        wait(&mut app, TOUCH_REACTION_COOLDOWN);
        let reaction = click(&mut app, BodyPart::LeftHand).unwrap();
        assert_eq!(reaction.hand_step, step);
        assert_eq!(
            reaction.clip,
            if step <= 4 {
                TouchReactionClip::Dismissive
            } else {
                TouchReactionClip::Shrugging
            }
        );
        assert_eq!(reaction.angry_mark, matches!(step, 4 | 7));
        assert_eq!(
            crate::expression::basic::for_visual(VisualState::TouchReaction(reaction.clip), None)
                .unwrap()
                .0,
            crate::expression::NeraExpressionPreset::Angry
        );
    }
    assert_eq!(
        app.world().resource::<TouchReactionRuntime>().hand_streak(),
        0
    );
    wait(&mut app, TOUCH_REACTION_COOLDOWN);
    let next = click(&mut app, BodyPart::RightHand).unwrap();
    assert_eq!(next.hand_step, 1);
    assert!(TouchReactionClip::NORMAL_HAND.contains(&next.clip));
}

#[test]
fn timeout_uses_last_accepted_hand_and_cooldown_bursts_do_not_advance() {
    let mut app = app();
    click(&mut app, BodyPart::LeftHand).unwrap();
    for _ in 0..100 {
        assert!(click(&mut app, BodyPart::RightHand).is_none());
    }
    assert_eq!(
        app.world().resource::<TouchReactionRuntime>().hand_streak(),
        1
    );
    wait(&mut app, HAND_STREAK_TIMEOUT - Duration::from_millis(1));
    assert_eq!(click(&mut app, BodyPart::RightHand).unwrap().hand_step, 2);
    wait(&mut app, HAND_STREAK_TIMEOUT);
    app.update();
    assert_eq!(
        app.world().resource::<TouchReactionRuntime>().hand_streak(),
        0
    );
    assert_eq!(click(&mut app, BodyPart::LeftHand).unwrap().hand_step, 1);
}

#[test]
fn head_hand_torso_and_arm_break_consecutive_hand_streak() {
    let mut app = app();
    assert!(click(&mut app, BodyPart::Head).is_none());
    assert_eq!(click(&mut app, BodyPart::LeftHand).unwrap().hand_step, 1);
    assert!(click(&mut app, BodyPart::Torso).is_none());
    assert_eq!(
        app.world().resource::<TouchReactionRuntime>().hand_streak(),
        0
    );
    wait(&mut app, TOUCH_REACTION_COOLDOWN);
    assert_eq!(click(&mut app, BodyPart::RightHand).unwrap().hand_step, 1);
    assert!(click(&mut app, BodyPart::LeftArm).is_some());
    assert_eq!(
        app.world().resource::<TouchReactionRuntime>().hand_streak(),
        0
    );
}

#[test]
fn arm_pool_is_independent_and_never_uses_hand_or_procedural_reactions() {
    let mut app = app();
    let first = click(&mut app, BodyPart::LeftArm).unwrap();
    assert!(TouchReactionClip::ARM.contains(&first.clip));
    assert_eq!(first.hand_step, 0);
    assert!(click(&mut app, BodyPart::RightArm).is_none());
    assert_eq!(click(&mut app, BodyPart::RightHand).unwrap().hand_step, 1);
    wait(&mut app, TOUCH_REACTION_COOLDOWN);
    let second = click(&mut app, BodyPart::RightArm).unwrap();
    assert!(TouchReactionClip::ARM.contains(&second.clip));
    assert_ne!(first.clip, second.clip);
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::TouchReaction(second.clip)
    );
}

#[test]
fn drag_from_hand_never_counts_or_uncovers_an_old_touch_clip() {
    let mut app = app();
    click(&mut app, BodyPart::LeftHand).unwrap();
    app.world_mut().write_message(NeraDragStarted {
        body_part: BodyPart::LeftHand,
        screen: IVec2::ZERO,
    });
    app.update();
    assert_eq!(
        app.world().resource::<TouchReactionRuntime>().hand_streak(),
        0
    );
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::Floating
    );
    wait(&mut app, TOUCH_REACTION_COOLDOWN);
    assert!(click(&mut app, BodyPart::RightHand).is_none());
    app.world_mut().write_message(NeraReleased {
        body_part: BodyPart::LeftHand,
        pointer: pointer(),
        was_dragging: true,
    });
    app.update();
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::Idle
    );
    app.world_mut()
        .resource_mut::<DesktopPetInteraction>()
        .state = InteractionState::Dragging;
    assert!(click(&mut app, BodyPart::LeftHand).is_none());
    assert_eq!(
        app.world().resource::<TouchReactionRuntime>().hand_streak(),
        0
    );
}

#[test]
fn alarm_death_fall_and_menu_ownership_do_not_queue_touch_reactions() {
    for action in [
        TemporaryAction::AlarmWave,
        TemporaryAction::AlarmJump,
        TemporaryAction::Death,
        TemporaryAction::Falling,
        TemporaryAction::Praying,
    ] {
        let mut app = app();
        click(&mut app, BodyPart::LeftHand).unwrap();
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .start_persistent_action(action);
        wait(&mut app, TOUCH_REACTION_COOLDOWN);
        assert!(click(&mut app, BodyPart::RightHand).is_none(), "{action:?}");
        assert!(click(&mut app, BodyPart::LeftArm).is_none(), "{action:?}");
        assert_eq!(
            app.world().resource::<TouchReactionRuntime>().hand_streak(),
            0
        );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .finish_action(action);
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::Idle
        );
    }
}

#[test]
fn missing_or_zero_duration_clip_does_not_consume_streak_step() {
    let mut app = app();
    click(&mut app, BodyPart::LeftHand).unwrap();
    wait(&mut app, TOUCH_REACTION_COOLDOWN);
    click(&mut app, BodyPart::RightHand).unwrap();
    app.world_mut()
        .resource_mut::<NeraAnimationClips>()
        .touch_reactions[TouchReactionClip::Dismissive.index()] = None;
    wait(&mut app, TOUCH_REACTION_COOLDOWN);
    assert!(click(&mut app, BodyPart::LeftHand).is_none());
    assert_eq!(
        app.world().resource::<TouchReactionRuntime>().hand_streak(),
        2
    );
    let mut clips = touch_test_clips(app.world_mut());
    clips.touch_reactions[TouchReactionClip::Dismissive.index()]
        .as_mut()
        .unwrap()
        .duration = Duration::ZERO;
    app.insert_resource(clips);
    assert!(click(&mut app, BodyPart::LeftHand).is_none());
    assert_eq!(
        app.world().resource::<TouchReactionRuntime>().hand_streak(),
        2
    );
    let clips = touch_test_clips(app.world_mut());
    app.insert_resource(clips);
    assert_eq!(click(&mut app, BodyPart::RightHand).unwrap().hand_step, 3);
}

#[test]
fn headpat_breaks_hand_streak_without_incrementing_it() {
    let mut app = app();
    click(&mut app, BodyPart::LeftHand).unwrap();
    app.world_mut()
        .write_message(NeraHeadPatted { pointer: pointer() });
    app.update();
    assert_eq!(
        app.world().resource::<TouchReactionRuntime>().hand_streak(),
        0
    );
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::HeadStroke
    );
}

#[test]
fn loading_and_replacement_clear_history_but_leave_chibi_owned() {
    let mut app = app();
    app.insert_resource(crate::avatar::body_mode::AvatarBodyMode::Chibi);
    click(&mut app, BodyPart::LeftHand).unwrap();
    app.world_mut()
        .resource_mut::<CharacterImportStatus>()
        .loading = true;
    wait(&mut app, TOUCH_REACTION_COOLDOWN);
    assert!(click(&mut app, BodyPart::RightHand).is_none());
    assert_eq!(
        app.world().resource::<TouchReactionRuntime>().hand_streak(),
        0
    );
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::Idle
    );
    crate::animation::reset_for_character_change(app.world_mut());
    assert_eq!(
        *app.world()
            .resource::<crate::avatar::body_mode::AvatarBodyMode>(),
        crate::avatar::body_mode::AvatarBodyMode::Chibi
    );
    app.world_mut()
        .resource_mut::<CharacterImportStatus>()
        .loading = false;
    let clips = touch_test_clips(app.world_mut());
    app.insert_resource(clips);
    assert_eq!(click(&mut app, BodyPart::LeftHand).unwrap().hand_step, 1);
    app.world_mut()
        .resource_mut::<NeraAnimationState>()
        .advance(Duration::from_secs(2));
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::Idle
    );
}

#[test]
fn leg_and_foot_dispatch_remain_procedural_and_break_hand_streak() {
    for part in [
        BodyPart::LeftLeg,
        BodyPart::RightLeg,
        BodyPart::LeftFoot,
        BodyPart::RightFoot,
    ] {
        let mut app = app();
        click(&mut app, BodyPart::LeftHand).unwrap();
        assert!(click(&mut app, part).is_none());
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::LimbReaction(part)
        );
        assert_eq!(
            app.world().resource::<TouchReactionRuntime>().hand_streak(),
            0
        );
    }
}
