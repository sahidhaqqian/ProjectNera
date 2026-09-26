//! One entrance per process: fall from above, land, notice the user, wave once.
//! Native movement remains owned by desktop_physics; animation owns every pose.
use std::time::Duration;

use bevy::{ecs::system::SystemParam, prelude::*, window::PrimaryWindow};
use bevy_vrm1::prelude::HipsBoneEntity;

use crate::{
    alarm::AlarmState,
    animation::{NeraAnimationClips, NeraAnimationState, VisualState},
    avatar::NeraAvatar,
    desktop_physics::DesktopPhysicsState,
    environment::{DesktopEnvironment, PlacementState},
    interaction::{DesktopPetInteraction, InteractionState},
    platform,
    presentation::PresentationState,
    settings::{NeraSettings, SettingsInputGate},
};

const NOTICE_DELAY: Duration = Duration::from_millis(400);
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);
const ENTRANCE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Waiting,
    Falling,
    Landing,
    Noticing(Duration),
    Greeting,
    Done,
}

#[derive(Resource, Default)]
pub(crate) struct StartupEntrance {
    phase: Phase,
    elapsed: Duration,
    ready_frames: u8,
    cancel_requested: bool,
}

impl StartupEntrance {
    pub(crate) fn is_waiting(&self) -> bool {
        self.phase == Phase::Waiting
    }

    pub(crate) fn blocks_position_save(&self) -> bool {
        self.phase != Phase::Done
    }

    pub(crate) fn request_cancel(&mut self) {
        self.cancel_requested = true;
    }

    fn complete(&mut self) {
        self.phase = Phase::Done;
        self.ready_frames = 0;
    }

    /// Returns true exactly once when the greeting should begin. A busy pose or
    /// lost support cancels the remaining script rather than resuming it later.
    fn advance(&mut self, visual: VisualState, grounded: bool, dt: Duration) -> bool {
        match self.phase {
            Phase::Falling => {
                if grounded {
                    self.phase = match visual {
                        VisualState::Landing => Phase::Landing,
                        VisualState::Idle => Phase::Noticing(NOTICE_DELAY),
                        _ => Phase::Done,
                    };
                } else if visual != VisualState::Falling {
                    self.complete();
                }
            }
            Phase::Landing => {
                if !grounded {
                    self.complete();
                } else if visual == VisualState::Idle {
                    self.phase = Phase::Noticing(NOTICE_DELAY);
                } else if visual != VisualState::Landing {
                    self.complete();
                }
            }
            Phase::Noticing(remaining) => {
                if !grounded || visual != VisualState::Idle {
                    self.complete();
                } else if dt >= remaining {
                    self.phase = Phase::Greeting;
                    return true;
                } else {
                    self.phase = Phase::Noticing(remaining - dt);
                }
            }
            Phase::Greeting => {
                if !grounded || visual != VisualState::StartupGreeting {
                    self.complete();
                }
            }
            Phase::Waiting | Phase::Done => {}
        }
        false
    }
}

pub(crate) fn physics_allowed(entrance: Res<StartupEntrance>) -> bool {
    !entrance.is_waiting()
}

pub(crate) fn autonomous_allowed(entrance: Res<StartupEntrance>) -> bool {
    !entrance.blocks_position_save()
}

/// Invisible loading geometry must not capture clicks before the entrance.
pub(crate) fn pet_is_revealed(entrance: Res<StartupEntrance>) -> bool {
    !entrance.is_waiting()
}

#[derive(SystemParam)]
pub(crate) struct StartupContext<'w, 's> {
    settings: Res<'w, NeraSettings>,
    presentation: Res<'w, PresentationState>,
    alarm: Res<'w, AlarmState>,
    interaction: Res<'w, DesktopPetInteraction>,
    settings_gate: Res<'w, SettingsInputGate>,
    #[cfg(target_os = "windows")]
    alarm_gate: Res<'w, crate::alarm_settings::AlarmSettingsInputGate>,
    primary: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
}

impl StartupContext<'_, '_> {
    fn interrupted(&self) -> bool {
        #[cfg(target_os = "windows")]
        if !crate::alarm_settings::nera_input_available(Res::clone(&self.alarm_gate)) {
            return true;
        }
        !self.presentation.is_desktop_pet()
            || self.alarm.is_ringing()
            || !crate::settings::nera_input_available(Res::clone(&self.settings_gate))
            || matches!(
                self.interaction.state,
                InteractionState::Pressed | InteractionState::Dragging
            )
            || self.interaction.right_pressed_body_part.is_some()
            || self
                .primary
                .iter()
                .next()
                .is_some_and(|window| !window.visible)
    }
}

pub(crate) fn prepare_startup_entrance(
    time: Res<Time>,
    context: StartupContext,
    clips: Res<NeraAnimationClips>,
    mut entrance: ResMut<StartupEntrance>,
    mut physics: ResMut<DesktopPhysicsState>,
    mut environment: ResMut<DesktopEnvironment>,
    mut animation: ResMut<NeraAnimationState>,
    mut avatar: Query<(&mut Visibility, Option<&HipsBoneEntity>), With<NeraAvatar>>,
) {
    if entrance.phase == Phase::Done {
        return;
    }
    entrance.elapsed += time.delta();
    let waiting = entrance.is_waiting();
    let timeout = if waiting {
        LOAD_TIMEOUT
    } else {
        ENTRANCE_TIMEOUT
    };
    let visual_interrupted = if waiting {
        animation.visual_state() != VisualState::Idle
    } else {
        !matches!(
            animation.visual_state(),
            VisualState::Idle
                | VisualState::Falling
                | VisualState::Landing
                | VisualState::StartupGreeting
        )
    };
    // Start Hidden deliberately skips the intro. Showing from the tray later
    // must not unexpectedly reposition a pet whose entrance was never visible.
    if entrance.cancel_requested
        || (waiting && context.settings.start_hidden)
        || context.interrupted()
        || visual_interrupted
        || environment.placement != PlacementState::Free
        || entrance.elapsed >= timeout
    {
        entrance.complete();
        physics.cancel_startup_drop();
        animation.cancel_startup_greeting();
        if !context.presentation.is_desktop_pet() {
            physics.reset_for_position_recovery();
        }
        for (mut visibility, _) in &mut avatar {
            *visibility = Visibility::Inherited;
        }
        return;
    }
    if !waiting {
        return;
    }

    let ready = clips.startup_intro_ready()
        && environment
            .feet_anchor_logical
            .is_some_and(|anchor| anchor.is_finite())
        && environment.monitor.is_some()
        && environment.nera_rect.is_some()
        && environment
            .nera_hwnd
            .is_some_and(platform::window_is_visible)
        && avatar.iter().any(|(_, hips)| hips.is_some());
    entrance.ready_frames = if ready {
        entrance.ready_frames.saturating_add(1)
    } else {
        0
    };
    // LoadedVrma precedes the final graph bake. Two complete updates also let
    // native position restore and the standing feet projection settle first.
    if entrance.ready_frames < 3 {
        return;
    }
    if physics.begin_startup_drop(&mut environment, &mut animation) {
        entrance.phase = Phase::Falling;
        entrance.elapsed = Duration::ZERO;
        info!("Nera startup entrance: Falling -> Landing -> Greeting");
    } else {
        entrance.complete();
        warn!("Nera startup entrance skipped: native placement unavailable");
    }
    for (mut visibility, _) in &mut avatar {
        *visibility = Visibility::Inherited;
    }
}

pub(crate) fn advance_startup_entrance(
    time: Res<Time>,
    context: StartupContext,
    environment: Res<DesktopEnvironment>,
    mut physics: ResMut<DesktopPhysicsState>,
    clips: Res<NeraAnimationClips>,
    mut animation: ResMut<NeraAnimationState>,
    mut entrance: ResMut<StartupEntrance>,
) {
    if matches!(entrance.phase, Phase::Waiting | Phase::Done) {
        return;
    }
    if context.interrupted() || environment.placement != PlacementState::Free {
        entrance.complete();
    } else {
        let grounded = !physics.falling && physics.support.is_some();
        if entrance.advance(animation.visual_state(), grounded, time.delta()) {
            if !animation.request_startup_greeting(clips.startup_greeting_duration()) {
                entrance.complete();
            } else {
                info!("Nera startup greeting: wave once");
            }
        }
    }
    if entrance.phase == Phase::Done {
        physics.cancel_startup_drop();
        animation.cancel_startup_greeting();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless_entrance() -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<NeraSettings>()
            .init_resource::<PresentationState>()
            .init_resource::<AlarmState>()
            .init_resource::<DesktopPetInteraction>()
            .init_resource::<SettingsInputGate>()
            .init_resource::<NeraAnimationClips>()
            .init_resource::<StartupEntrance>()
            .init_resource::<DesktopPhysicsState>()
            .init_resource::<DesktopEnvironment>()
            .init_resource::<NeraAnimationState>()
            .add_systems(
                Update,
                (prepare_startup_entrance, advance_startup_entrance).chain(),
            );
        #[cfg(target_os = "windows")]
        app.init_resource::<crate::alarm_settings::AlarmSettingsInputGate>();
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        let avatar = app.world_mut().spawn((NeraAvatar, Visibility::Hidden)).id();
        // There is no Winit, renderer, native HWND, or asset loader in this app.
        (app, avatar)
    }

    #[test]
    fn missing_assets_wait_hidden_then_release_control_on_timeout() {
        let (mut app, avatar) = headless_entrance();
        app.update();
        assert!(app.world().resource::<StartupEntrance>().is_waiting());
        assert_eq!(
            app.world().get::<Visibility>(avatar),
            Some(&Visibility::Hidden)
        );
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(LOAD_TIMEOUT);
        app.update();
        assert!(
            !app.world()
                .resource::<StartupEntrance>()
                .blocks_position_save()
        );
        assert_eq!(
            app.world().get::<Visibility>(avatar),
            Some(&Visibility::Inherited)
        );
        assert!(!app.world().resource::<DesktopPhysicsState>().falling);
    }

    #[test]
    fn hidden_start_skips_entrance_and_later_show_does_not_replay_it() {
        let (mut app, avatar) = headless_entrance();
        app.world_mut().resource_mut::<NeraSettings>().start_hidden = true;
        app.update();
        assert_eq!(app.world().resource::<StartupEntrance>().phase, Phase::Done);
        assert_eq!(
            app.world().get::<Visibility>(avatar),
            Some(&Visibility::Inherited)
        );
        app.world_mut().resource_mut::<NeraSettings>().start_hidden = false;
        app.update();
        assert_eq!(app.world().resource::<StartupEntrance>().phase, Phase::Done);
        assert!(!app.world().resource::<DesktopPhysicsState>().falling);
    }

    #[test]
    fn reset_during_loading_reveals_pet_and_press_cancels_active_greeting() {
        let (mut app, avatar) = headless_entrance();
        app.world_mut()
            .resource_mut::<StartupEntrance>()
            .request_cancel();
        app.update();
        assert_eq!(app.world().resource::<StartupEntrance>().phase, Phase::Done);
        assert_eq!(
            app.world().get::<Visibility>(avatar),
            Some(&Visibility::Inherited)
        );

        let (mut app, _) = headless_entrance();
        app.world_mut().resource_mut::<StartupEntrance>().phase = Phase::Greeting;
        assert!(
            app.world_mut()
                .resource_mut::<NeraAnimationState>()
                .request_startup_greeting(Duration::from_secs(3))
        );
        app.world_mut()
            .resource_mut::<DesktopPetInteraction>()
            .state = InteractionState::Pressed;
        app.update();
        assert_eq!(app.world().resource::<StartupEntrance>().phase, Phase::Done);
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::Idle
        );
    }

    #[test]
    fn landing_finishes_before_notice_and_exactly_one_greeting() {
        let mut entrance = StartupEntrance {
            phase: Phase::Falling,
            ..default()
        };
        assert!(!entrance.advance(VisualState::Falling, false, Duration::from_secs(1)));
        assert!(!entrance.advance(VisualState::Landing, true, Duration::from_millis(20)));
        for _ in 0..10 {
            assert!(!entrance.advance(VisualState::Landing, true, Duration::from_millis(100)));
        }
        assert!(!entrance.advance(VisualState::Idle, true, Duration::from_millis(20)));
        assert!(!entrance.advance(VisualState::Idle, true, Duration::from_millis(200)));
        assert!(entrance.advance(VisualState::Idle, true, Duration::from_millis(200)));
        assert!(!entrance.advance(VisualState::StartupGreeting, true, Duration::from_secs(1)));
        assert!(!entrance.advance(VisualState::Idle, true, Duration::from_millis(20)));
        assert_eq!(entrance.phase, Phase::Done);
        assert!(!entrance.blocks_position_save());
        assert!(!entrance.advance(VisualState::Idle, true, Duration::from_secs(60)));
    }

    #[test]
    fn reaction_or_lost_support_cancels_the_remaining_sequence() {
        for phase in [
            Phase::Falling,
            Phase::Landing,
            Phase::Noticing(NOTICE_DELAY),
            Phase::Greeting,
        ] {
            let mut entrance = StartupEntrance { phase, ..default() };
            assert!(!entrance.advance(VisualState::HitHead, true, Duration::ZERO));
            assert_eq!(entrance.phase, Phase::Done);
            assert!(!entrance.advance(VisualState::Idle, true, Duration::from_secs(20)));
        }
        let mut entrance = StartupEntrance {
            phase: Phase::Noticing(NOTICE_DELAY),
            ..default()
        };
        assert!(!entrance.advance(VisualState::Falling, false, Duration::from_secs(1)));
        assert_eq!(entrance.phase, Phase::Done);
    }
}
