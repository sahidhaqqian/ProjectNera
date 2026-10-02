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
    if presentation.is_desktop_pet() {
        if headpat && cloud.mode() != Some(crate::cloud::CloudMode::Rest) {
            sfx.write(PetSfx(PetSfxKind::HeadPat));
            fx.write(PetFx {
                kind: PetFxKind::Heart,
                anchor: PetFxAnchor::Head,
            });
        }
        if previous.falling
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
        if previous.visual != Some(visual) {
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
                    Face::Joy
                        if matches!(
                            visual,
                            VisualState::StartupGreeting | VisualState::AlarmJump
                        ) =>
                    {
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
    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<NeraAnimationState>()
            .init_resource::<DesktopPhysicsState>()
            .init_resource::<PresentationState>()
            .init_resource::<DesktopPetInteraction>()
            .init_resource::<crate::cloud::CloudRuntime>()
            .init_resource::<crate::animation::LibraryPlayback>()
            .add_message::<NeraHeadPatted>()
            .add_message::<PetSfx>()
            .add_message::<PetFx>()
            .add_systems(Update, interaction_feedback);
        app
    }
    #[test]
    fn headpat_burst_is_consumed_once_not_replayed_on_subsequent_updates() {
        let mut app = app();
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
