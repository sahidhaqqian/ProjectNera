//! Semantic feedback API for gameplay and future mood callers.
mod particles;
mod sfx;
use crate::{
    animation::{NeraAnimationState, VisualState},
    desktop_physics::DesktopPhysicsState,
    interaction::{DesktopPetInteraction, InteractionState, NeraHeadPatted},
    presentation::PresentationState,
};
use bevy::prelude::*;
pub(crate) use particles::update_particles;
#[cfg(debug_assertions)]
pub(crate) use particles::{MAX_PARTICLES, PetParticle};
pub(crate) use particles::{PetFx, PetFxAnchor, PetFxKind};
pub(crate) use sfx::{PetSfx, PetSfxKind};

pub(crate) struct PetFeedbackPlugin;
impl Plugin for PetFeedbackPlugin {
    fn build(&self, app: &mut App) {
        sfx::install(app);
        particles::install(app);
        app.add_systems(
            Update,
            interaction_feedback
                .after(crate::animation::apply_animation_state)
                .before(sfx::play_sfx),
        );
    }
}

#[derive(Default)]
struct FeedbackHistory {
    falling: bool,
    visual: Option<VisualState>,
}

fn interaction_feedback(
    mut pats: MessageReader<NeraHeadPatted>,
    mut touches: MessageReader<crate::animation::touch::TouchReactionAccepted>,
    animation: Res<NeraAnimationState>,
    physics: Res<DesktopPhysicsState>,
    presentation: Res<PresentationState>,
    interaction: Res<DesktopPetInteraction>,
    cloud: Res<crate::cloud::CloudRuntime>,
    library: Res<crate::animation::LibraryPlayback>,
    mut previous: Local<FeedbackHistory>,
    mut sfx: MessageWriter<PetSfx>,
    mut fx: MessageWriter<PetFx>,
) {
    let visual = animation.visual_state();
    let headpat = pats.read().count() > 0;
    for touch in touches.read() {
        if !presentation.is_desktop_pet()
            || cloud.mode() == Some(crate::cloud::CloudMode::Rest)
            || interaction.state == InteractionState::Dragging
            || visual != VisualState::TouchReaction(touch.clip)
        {
            continue;
        }
        if touch.angry_mark {
            fx.write(PetFx {
                kind: PetFxKind::AngryMark,
                anchor: PetFxAnchor::Head,
            });
        }
        if let Some((face, _)) = crate::expression::basic::for_visual(visual, None) {
            use crate::expression::NeraExpressionPreset as Face;
            let sound = match face {
                Face::Joy => Some(PetSfxKind::Happy),
                Face::Angry => Some(PetSfxKind::Angry),
                Face::Timid | Face::Embarrassed => Some(PetSfxKind::Embarrassed),
                _ => None,
            };
            if let Some(kind) = sound {
                sfx.write(PetSfx(kind));
            }
        }
    }
    if presentation.is_desktop_pet() || presentation.is_screensaver() {
        if headpat
            && visual == VisualState::HeadStroke
            && cloud.mode() != Some(crate::cloud::CloudMode::Rest)
        {
            sfx.write(PetSfx(PetSfxKind::HeadPat));
            fx.write(PetFx {
                kind: PetFxKind::Heart,
                anchor: PetFxAnchor::Head,
            });
        }
        if presentation.is_desktop_pet()
            && previous.falling
            && !physics.falling
            && physics.support.is_some()
            && !cloud.owns_placement()
            && interaction.state != InteractionState::Dragging
        {
            sfx.write(PetSfx(PetSfxKind::Landing));
            fx.write(PetFx {
                kind: PetFxKind::LandingPuff,
                anchor: PetFxAnchor::Feet,
            });
        }
        if previous.visual != Some(visual) && !matches!(visual, VisualState::TouchReaction(_)) {
            if matches!(
                visual,
                VisualState::StartupGreeting
                    | VisualState::Screensaver(
                        crate::animation::screensaver::ScreensaverClip::Waving
                    )
            ) {
                sfx.write(PetSfx(PetSfxKind::Greeting));
            }
            let path = if let VisualState::LibraryAnimation(entity) = visual {
                library.path_for(entity)
            } else {
                None
            };
            if let Some((face, _)) = crate::expression::basic::for_visual(visual, path) {
                use crate::expression::NeraExpressionPreset as Face;
                match face {
                    Face::Angry => {
                        sfx.write(PetSfx(PetSfxKind::Angry));
                        fx.write(PetFx {
                            kind: PetFxKind::AngryMark,
                            anchor: PetFxAnchor::Head,
                        });
                    }
                    Face::Joy if visual == VisualState::AlarmJump => {
                        sfx.write(PetSfx(PetSfxKind::Happy));
                    }
                    Face::Embarrassed if !headpat && visual != VisualState::HeadStroke => {
                        sfx.write(PetSfx(PetSfxKind::Embarrassed));
                    }
                    _ => {}
                }
            }
        }
    }
    previous.falling = physics.falling;
    previous.visual = Some(visual);
}

pub(crate) fn clear_for_character_change(world: &mut World) {
    particles::clear(world);
    sfx::clear(world);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch_app() -> App {
        let mut app = app();
        let clips = crate::animation::touch_test_clips(app.world_mut());
        app.insert_resource(clips)
            .init_resource::<Time>()
            .init_resource::<crate::animation::touch::TouchReactionRuntime>()
            .init_resource::<crate::character::CharacterImportStatus>()
            .init_resource::<crate::environment::DesktopEnvironment>()
            .add_message::<crate::interaction::NeraClicked>()
            .add_message::<crate::interaction::NeraDragStarted>()
            .add_message::<crate::interaction::NeraReleased>()
            .add_systems(
                Update,
                crate::animation::sync_interaction_animation_actions.before(interaction_feedback),
            );
        app
    }

    fn hand_click(app: &mut App) {
        app.world_mut()
            .write_message(crate::interaction::NeraClicked {
                body_part: crate::interaction::BodyPart::LeftHand,
                pointer: crate::interaction::NeraPointerPosition {
                    screen: IVec2::ZERO,
                    local: Vec2::ZERO,
                },
            });
    }

    fn drain_feedback(app: &mut App) -> (Vec<PetFx>, Vec<PetSfx>) {
        let fx = app
            .world_mut()
            .resource_mut::<Messages<PetFx>>()
            .drain()
            .collect();
        let sfx = app
            .world_mut()
            .resource_mut::<Messages<PetSfx>>()
            .drain()
            .collect();
        (fx, sfx)
    }

    #[test]
    fn only_accepted_hand_steps_four_and_seven_emit_one_angry_mark() {
        use crate::animation::touch::{TOUCH_REACTION_COOLDOWN, TouchReactionAccepted};
        let mut app = touch_app();
        for step in 1..=7 {
            if step > 1 {
                app.world_mut()
                    .resource_mut::<Time>()
                    .advance_by(TOUCH_REACTION_COOLDOWN);
            }
            hand_click(&mut app);
            app.update();
            let accepted: Vec<_> = app
                .world_mut()
                .resource_mut::<Messages<TouchReactionAccepted>>()
                .drain()
                .collect();
            assert_eq!(accepted.len(), 1);
            assert_eq!(accepted[0].hand_step, step);
            let (fx, sfx) = drain_feedback(&mut app);
            let expected_marks = usize::from(matches!(step, 4 | 7));
            assert_eq!(fx.len(), expected_marks, "hand step {step}");
            assert!(fx.iter().all(|effect| effect.kind == PetFxKind::AngryMark));
            if step >= 3 {
                assert_eq!(sfx.len(), 1);
                assert_eq!(sfx[0].0, PetSfxKind::Angry);
            }
            app.update();
            let (fx, sfx) = drain_feedback(&mut app);
            assert!(
                fx.is_empty() && sfx.is_empty(),
                "accepted feedback replayed at step {step}"
            );
        }
    }

    #[test]
    fn rejected_cooldown_bursts_and_loading_clicks_emit_no_touch_feedback() {
        use crate::animation::touch::{TOUCH_REACTION_COOLDOWN, TouchReactionAccepted};
        let mut app = touch_app();
        hand_click(&mut app);
        app.update();
        app.world_mut()
            .resource_mut::<Messages<TouchReactionAccepted>>()
            .drain()
            .for_each(drop);
        drain_feedback(&mut app);
        for _ in 0..100 {
            hand_click(&mut app);
        }
        app.update();
        assert!(
            app.world()
                .resource::<Messages<TouchReactionAccepted>>()
                .is_empty()
        );
        let (fx, sfx) = drain_feedback(&mut app);
        assert!(fx.is_empty() && sfx.is_empty());
        assert_eq!(
            app.world()
                .resource::<crate::animation::touch::TouchReactionRuntime>()
                .hand_streak(),
            1
        );
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(TOUCH_REACTION_COOLDOWN);
        app.world_mut()
            .resource_mut::<crate::character::CharacterImportStatus>()
            .loading = true;
        hand_click(&mut app);
        app.update();
        assert!(
            app.world()
                .resource::<Messages<TouchReactionAccepted>>()
                .is_empty()
        );
        let (fx, sfx) = drain_feedback(&mut app);
        assert!(fx.is_empty() && sfx.is_empty());
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::Idle
        );
    }

    #[test]
    fn a_superseded_or_drag_blocked_accepted_touch_cannot_emit_feedback() {
        use crate::animation::{TouchReactionClip, touch::TouchReactionAccepted};
        for dragging in [false, true] {
            let mut app = app();
            assert!(
                app.world_mut()
                    .resource_mut::<NeraAnimationState>()
                    .request_touch_reaction(
                        TouchReactionClip::Dismissive,
                        std::time::Duration::from_secs(2),
                    )
            );
            app.world_mut().write_message(TouchReactionAccepted {
                clip: TouchReactionClip::Dismissive,
                hand_step: 4,
                angry_mark: true,
            });
            if dragging {
                app.world_mut()
                    .resource_mut::<DesktopPetInteraction>()
                    .state = InteractionState::Dragging;
            } else {
                app.world_mut()
                    .resource_mut::<NeraAnimationState>()
                    .set_physics_falling(true);
            }
            app.update();
            let (fx, sfx) = drain_feedback(&mut app);
            assert!(fx.is_empty() && sfx.is_empty());
            app.update();
            let (fx, sfx) = drain_feedback(&mut app);
            assert!(fx.is_empty() && sfx.is_empty());
        }
    }

    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<NeraAnimationState>()
            .init_resource::<DesktopPhysicsState>()
            .init_resource::<PresentationState>()
            .init_resource::<DesktopPetInteraction>()
            .init_resource::<crate::cloud::CloudRuntime>()
            .init_resource::<crate::animation::LibraryPlayback>()
            .add_message::<NeraHeadPatted>()
            .add_message::<crate::animation::touch::TouchReactionAccepted>()
            .add_message::<PetSfx>()
            .add_message::<PetFx>()
            .add_systems(Update, interaction_feedback);
        app
    }
    #[test]
    fn startup_wave_emits_one_greeting_without_a_duplicate_happy_cue() {
        let mut app = app();
        assert!(
            app.world_mut()
                .resource_mut::<NeraAnimationState>()
                .request_startup_greeting(std::time::Duration::from_secs(3))
        );
        app.update();
        let (_, sounds) = drain_feedback(&mut app);
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].0, PetSfxKind::Greeting);
        app.update();
        assert!(app.world().resource::<Messages<PetSfx>>().is_empty());
    }

    #[test]
    fn screensaver_wave_uses_the_same_greeting_once() {
        use crate::presentation::{AttentionKind, PresentationMode};
        let mut app = app();
        app.insert_resource(PresentationState::for_test(PresentationMode::Attention(
            AttentionKind::Screensaver,
        )));
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .request_action_for_test(
                crate::animation::TemporaryAction::Screensaver(
                    crate::animation::screensaver::ScreensaverClip::Waving,
                ),
                std::time::Duration::from_secs(3),
            );
        app.update();
        let (_, sounds) = drain_feedback(&mut app);
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].0, PetSfxKind::Greeting);
        app.update();
        assert!(app.world().resource::<Messages<PetSfx>>().is_empty());
    }
    #[test]
    fn headpat_burst_is_consumed_once_not_replayed_on_subsequent_updates() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .request_action_for_test(
                crate::animation::TemporaryAction::HeadStroke,
                std::time::Duration::from_secs(2),
            );
        for _ in 0..100 {
            app.world_mut().write_message(NeraHeadPatted {
                pointer: crate::interaction::NeraPointerPosition {
                    screen: IVec2::ZERO,
                    local: Vec2::ZERO,
                },
            });
        }
        app.update();
        let effects: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<PetFx>>()
            .drain()
            .collect();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].kind, PetFxKind::Heart);
        let sounds: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<PetSfx>>()
            .drain()
            .collect();
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].0, PetSfxKind::HeadPat);
        app.update();
        assert!(app.world().resource::<Messages<PetFx>>().is_empty());
        assert!(app.world().resource::<Messages<PetSfx>>().is_empty());
    }
    #[test]
    fn an_overridden_headpat_does_not_start_hearts_or_sound() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .request_action_for_test(
                crate::animation::TemporaryAction::HeadStroke,
                std::time::Duration::from_secs(2),
            );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .set_physics_falling(true);
        app.world_mut().write_message(NeraHeadPatted {
            pointer: crate::interaction::NeraPointerPosition {
                screen: IVec2::ZERO,
                local: Vec2::ZERO,
            },
        });
        app.update();
        assert!(app.world().resource::<Messages<PetFx>>().is_empty());
        assert!(app.world().resource::<Messages<PetSfx>>().is_empty());
    }
    #[test]
    fn real_support_touchdown_emits_once_but_drag_cancellation_does_not() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<DesktopPhysicsState>()
            .falling = true;
        app.update();
        {
            let mut physics = app.world_mut().resource_mut::<DesktopPhysicsState>();
            physics.falling = false;
            physics.support = Some(crate::desktop_physics::PhysicsSupport::Desktop {
                monitor: crate::platform::NativeMonitorHandle::from_raw(1),
            });
        }
        app.update();
        let effects: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<PetFx>>()
            .drain()
            .collect();
        assert_eq!(effects.len(), 1);
        assert_eq!(effects[0].kind, PetFxKind::LandingPuff);
        let sounds: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<PetSfx>>()
            .drain()
            .collect();
        assert_eq!(sounds.len(), 1);
        assert_eq!(sounds[0].0, PetSfxKind::Landing);
        app.update();
        assert!(app.world().resource::<Messages<PetFx>>().is_empty());
        app.world_mut()
            .resource_mut::<DesktopPhysicsState>()
            .falling = true;
        app.update();
        app.world_mut()
            .resource_mut::<DesktopPhysicsState>()
            .reset_for_position_recovery();
        app.world_mut()
            .resource_mut::<DesktopPetInteraction>()
            .state = InteractionState::Dragging;
        app.update();
        assert!(app.world().resource::<Messages<PetFx>>().is_empty());
    }
}
