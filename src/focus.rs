//! Activity intent and monotonic session clock, independent of real alarms.
use std::time::Duration;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

mod activity;
pub(crate) use activity::FocusProp;
#[cfg(debug_assertions)]
pub(crate) mod smoke;

use crate::{
    animation::{NeraAnimationClips, NeraAnimationState, VisualState},
    interaction::{DesktopPetInteraction, InteractionState},
    pet_feedback::{PetFx, PetFxAnchor, PetFxKind, PetSfx, PetSfxKind},
    presentation::PresentationState,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum FocusPreset {
    #[default]
    Study,
    Work,
}
impl FocusPreset {
    pub(crate) fn lazy_clip(self) -> crate::animation::LazyNeraVrma {
        use crate::animation::LazyNeraVrma;
        match self {
            Self::Study => LazyNeraVrma::Praying,
            Self::Work => LazyNeraVrma::FocusTyping,
        }
    }
    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Study => "Nera settles in with an open book for quiet reading.",
            Self::Work => "Nera sits at her laptop and types alongside you.",
        }
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Study => "Study",
            Self::Work => "Work",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FocusLength {
    #[default]
    Minutes25,
    Minutes50,
    Custom,
    Endless,
}

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FocusCommand {
    Preset(FocusPreset),
    Length(FocusLength),
    CustomDelta(i16),
    Start,
    Stop,
}

struct Session {
    preset: FocusPreset,
    timer: Option<Timer>,
    paused: bool,
    ride_requested: bool,
    ride_observed: bool,
}

#[derive(Resource)]
pub(crate) struct FocusSession {
    pub(crate) preset: FocusPreset,
    pub(crate) length: FocusLength,
    pub(crate) custom_minutes: u16,
    session: Option<Session>,
    completed: bool,
    completion_pending: bool,
    completion_notice: Option<FocusPreset>,
    stop_reason: Option<&'static str>,
    release_ride: bool,
}
impl Default for FocusSession {
    fn default() -> Self {
        Self {
            preset: FocusPreset::Study,
            length: FocusLength::Minutes25,
            custom_minutes: 35,
            session: None,
            completed: false,
            completion_pending: false,
            completion_notice: None,
            stop_reason: None,
            release_ride: false,
        }
    }
}
impl FocusSession {
    pub(crate) fn active(&self) -> bool {
        self.session.is_some()
    }
    pub(crate) fn remaining(&self) -> Option<Duration> {
        self.session.as_ref()?.timer.as_ref().map(Timer::remaining)
    }
    pub(crate) fn running_preset(&self) -> Option<FocusPreset> {
        self.session.as_ref().map(|session| session.preset)
    }
    pub(crate) fn sfx_factor(&self, kind: PetSfxKind) -> f32 {
        if self.active() && kind != PetSfxKind::HeadPat {
            0.25
        } else {
            1.0
        }
    }
    pub(crate) fn status(&self) -> String {
        match self.session.as_ref() {
            Some(session) => match self.remaining() {
                Some(remaining) => {
                    let seconds = remaining.as_secs_f64().ceil() as u64;
                    format!(
                        "{} · {:02}:{:02} remaining{}",
                        session.preset.label(),
                        seconds / 60,
                        seconds % 60,
                        if session.paused {
                            " · Paused / preparing activity"
                        } else {
                            " · In progress"
                        }
                    )
                }
                None => format!(
                    "{} · Endless{}",
                    session.preset.label(),
                    if session.paused {
                        " · Paused / preparing activity"
                    } else {
                        " · In progress"
                    }
                ),
            },
            None if self.completed => "Completed — nice work!".into(),
            None => self
                .stop_reason
                .unwrap_or("Ready · choose an activity and press Start")
                .into(),
        }
    }
    fn apply(&mut self, command: FocusCommand) {
        match command {
            FocusCommand::Preset(preset) => self.preset = preset,
            FocusCommand::Length(length) => self.length = length,
            FocusCommand::CustomDelta(delta) => {
                self.custom_minutes =
                    (i32::from(self.custom_minutes) + i32::from(delta)).clamp(1, 480) as u16;
                self.length = FocusLength::Custom;
            }
            FocusCommand::Start => {
                let previous = self.session.take();
                let keep_ride = self.preset == FocusPreset::Work
                    && previous
                        .as_ref()
                        .is_some_and(|session| session.ride_requested);
                self.release_ride |= !keep_ride
                    && previous
                        .as_ref()
                        .is_some_and(|session| session.ride_requested);
                let minutes = match self.length {
                    FocusLength::Minutes25 => Some(25),
                    FocusLength::Minutes50 => Some(50),
                    FocusLength::Custom => Some(u64::from(self.custom_minutes.clamp(1, 480))),
                    FocusLength::Endless => None,
                };
                self.session = Some(Session {
                    preset: self.preset,
                    paused: false,
                    ride_requested: keep_ride,
                    ride_observed: keep_ride
                        && previous
                            .as_ref()
                            .is_some_and(|session| session.ride_observed),
                    timer: minutes.map(|minutes| {
                        Timer::new(Duration::from_secs(minutes * 60), TimerMode::Once)
                    }),
                });
                self.completed = false;
                self.completion_pending = false;
                self.completion_notice = None;
                self.stop_reason = None;
                info!("Focus started | {:?} {:?}", self.preset, self.length);
            }
            FocusCommand::Stop => {
                self.release_ride |= self
                    .session
                    .take()
                    .is_some_and(|session| session.ride_requested);
                self.completion_pending = false;
                self.completed = false;
                self.completion_notice = None;
                self.stop_reason = None;
                info!("Focus stopped; normal behavior resumed");
            }
        }
    }
    fn advance(&mut self, dt: Duration) {
        if self.session.as_ref().is_some_and(|session| session.paused) {
            return;
        }
        if let Some(timer) = self
            .session
            .as_mut()
            .and_then(|session| session.timer.as_mut())
        {
            timer.tick(dt);
            if timer.just_finished() {
                if let Some(session) = self.session.take() {
                    self.release_ride |= session.ride_requested;
                    self.completion_notice = Some(session.preset);
                }
                self.completed = true;
                self.completion_pending = true;
                info!("Focus completed; normal behavior resumed");
            }
        }
    }
    fn interrupt(&mut self, reason: &'static str) {
        self.apply(FocusCommand::Stop);
        self.stop_reason = Some(reason);
    }
    fn restore_preferences(&mut self, settings: &crate::settings::NeraSettings) {
        self.preset = if settings.focus_mode == 1 {
            FocusPreset::Work
        } else {
            FocusPreset::Study
        };
        self.length = match settings.focus_duration {
            1 => FocusLength::Minutes50,
            2 => FocusLength::Custom,
            3 => FocusLength::Endless,
            _ => FocusLength::Minutes25,
        };
        self.custom_minutes = settings.focus_custom_minutes.clamp(1, 480);
    }
    fn store_preferences(&self, settings: &mut crate::settings::NeraSettings) {
        settings.focus_mode = u8::from(self.preset == FocusPreset::Work);
        settings.focus_duration = match self.length {
            FocusLength::Minutes25 => 0,
            FocusLength::Minutes50 => 1,
            FocusLength::Custom => 2,
            FocusLength::Endless => 3,
        };
        settings.focus_custom_minutes = self.custom_minutes;
    }
}

pub(crate) struct FocusPlugin;
#[cfg(debug_assertions)]
impl FocusSession {
    pub(crate) fn smoke_finish(&mut self) {
        // Advance only the Focus clock; real alarms retain their Instant deadlines.
        if let Some(remaining) = self.remaining() {
            self.advance(remaining);
        }
    }
}
impl Plugin for FocusPlugin {
    fn build(&self, app: &mut App) {
        activity::install(app);
        app.init_resource::<FocusSession>()
            .add_message::<FocusCommand>()
            .add_systems(
                Startup,
                |settings: Res<crate::settings::NeraSettings>, mut focus: ResMut<FocusSession>| {
                    focus.restore_preferences(&settings)
                },
            )
            .add_systems(
                Update,
                update_focus
                    .after(crate::animation::sync_interaction_animation_actions)
                    .before(crate::animation::apply_animation_state)
                    .before(crate::pet_behavior::update_pet_behavior),
            );
    }
}

#[derive(SystemParam)]
pub(crate) struct FocusContext<'w> {
    interaction: Res<'w, DesktopPetInteraction>,
    presentation: Res<'w, PresentationState>,
    loading: Option<Res<'w, crate::character::CharacterImportStatus>>,
    cloud: Res<'w, crate::cloud::CloudRuntime>,
    environment: Res<'w, crate::environment::DesktopEnvironment>,
    cloud_seat: Res<'w, crate::cloud::CloudSeatPose>,
    desktop_seat: Res<'w, crate::animation::touch::seated::TouchSeatPose>,
    entrance: Res<'w, crate::startup_entrance::StartupEntrance>,
    settings: ResMut<'w, crate::settings::NeraSettings>,
    alarm: Option<Res<'w, crate::alarm::AlarmState>>,
}

impl FocusContext<'_> {
    fn seat_ready(&self, preset: FocusPreset, animation: &NeraAnimationState) -> bool {
        use crate::environment::PlacementState;
        if self.cloud.is_riding() {
            return self.cloud_seat.is_ready_for(&self.cloud);
        }
        match self.environment.placement {
            PlacementState::Taskbar(_) | PlacementState::Window(_)
                if preset == FocusPreset::Study
                    || matches!(self.environment.placement, PlacementState::Taskbar(_)) =>
            {
                self.desktop_seat.is_ready_for(
                    self.environment.placement,
                    animation.selected_sitting_pose(),
                )
            }
            PlacementState::Free => preset == FocusPreset::Study,
            _ => false,
        }
    }
}

pub(crate) fn update_focus(
    time: Res<Time<Real>>,
    mut requests: MessageReader<FocusCommand>,
    mut commands: Commands,
    mut focus: ResMut<FocusSession>,
    mut animation: ResMut<NeraAnimationState>,
    clips: Res<NeraAnimationClips>,
    mut context: FocusContext,
    asset_server: Res<AssetServer>,
    loaders: Query<(
        Entity,
        &crate::animation::LazyNeraVrma,
        Option<&crate::animation::LazyNeraVrmaRequested>,
    )>,
    mut fx: MessageWriter<PetFx>,
    mut sfx: MessageWriter<PetSfx>,
    mut cloud_commands: MessageWriter<crate::cloud::CloudCommand>,
    #[cfg(target_os = "windows")] mut notifications: MessageWriter<crate::tray::TrayNotification>,
) {
    // Account for the whole real frame even after a stall; rendering's capped
    // virtual step must not stretch a 25-minute focus session.
    for command in requests.read() {
        focus.apply(*command);
        if matches!(
            command,
            FocusCommand::Preset(_) | FocusCommand::Length(_) | FocusCommand::CustomDelta(_)
        ) {
            focus.store_preferences(&mut context.settings);
            crate::settings::save_settings_deferred(&mut commands);
        }
    }
    let critical = !context.presentation.is_desktop_pet()
        || context.alarm.as_ref().is_some_and(|alarm| {
            matches!(
                alarm.status_at(std::time::Instant::now()),
                crate::alarm::AlarmStatus::Ringing
            )
        })
        || context
            .loading
            .as_ref()
            .is_some_and(|status| status.loading)
        || matches!(context.cloud.mode(), Some(crate::cloud::CloudMode::Rest))
        || matches!(
            animation.visual_state(),
            VisualState::AlarmJump | VisualState::AlarmWave | VisualState::Death
        );
    if critical && (focus.active() || focus.completion_pending) {
        focus.interrupt("Session ended for another activity or presentation");
    }
    let safe = !critical
        && !context.entrance.blocks_position_save()
        && context.interaction.state != InteractionState::Dragging
        && context.interaction.drag_offset.is_none();
    let on_taskbar = matches!(
        context.environment.placement,
        crate::environment::PlacementState::Taskbar(_)
    );
    let at_taskbar = on_taskbar
        || matches!(
            context.environment.placement,
            crate::environment::PlacementState::PendingTaskbar { .. }
        );
    if safe && focus.running_preset() == Some(FocusPreset::Work) {
        let session = focus.session.as_mut().unwrap();
        if context.cloud.is_riding() {
            session.ride_observed = true;
        } else if session.ride_requested && session.ride_observed {
            focus.interrupt("Work ended when Ride Cloud was dismissed");
        } else if !at_taskbar
            && !session.ride_requested
            && matches!(
                animation.visual_state(),
                VisualState::Idle | VisualState::SittingLoop
            )
        {
            cloud_commands.write(crate::cloud::CloudCommand::Summon(
                crate::cloud::CloudMode::Ride,
            ));
            session.ride_requested = true;
            info!("Focus Work: requesting existing Ride Cloud seat before desk/typing");
        }
    }
    if let Some(preset) = focus.running_preset() {
        let ready = clips.focus_activity_ready(preset);
        if !ready {
            crate::animation::request_lazy_vrma(
                preset.lazy_clip(),
                &mut commands,
                &asset_server,
                &loaders,
            );
        }
        // Standing-authored reading preserves the existing seated lower body.
        // Wait for that owner's evaluated pose instead of caching the reading
        // pose as the seat when Start arrives during a sitting transition.
        let seat_ready = context.seat_ready(preset, &animation);
        let playing = safe && ready && seat_ready && animation.request_focus_activity(preset);
        if let Some(session) = focus.session.as_mut() {
            session.paused = !playing;
        }
        if !playing {
            animation.cancel_focus_activity();
        }
        focus.advance(time.delta());
    }
    if let Some(preset) = focus.completion_notice.take() {
        #[cfg(target_os = "windows")]
        notifications.write(crate::tray::TrayNotification {
            title: format!("{} finished", preset.label()),
            body: "Your Focus session is complete. Time for a short break!".into(),
        });
    }
    if !focus.active() {
        animation.cancel_focus_activity();
    }
    let releasing_ride = focus.release_ride && context.cloud.is_riding();
    if focus.release_ride {
        // Release only a Ride introduced by Focus, never a user's Rest switch.
        if context.cloud.is_riding() {
            cloud_commands.write(crate::cloud::CloudCommand::Dismiss);
        }
        focus.release_ride = false;
    }
    let celebrate_safe = safe
        && !releasing_ride
        && (!context.cloud.owns_placement() || context.cloud.is_riding())
        && matches!(
            animation.visual_state(),
            VisualState::Idle | VisualState::SittingLoop
        );
    if focus.completion_pending
        && celebrate_safe
        && animation.request_focus_celebration(clips.startup_greeting_duration())
    {
        focus.completion_pending = false;
        for _ in 0..4 {
            fx.write(PetFx {
                kind: PetFxKind::Heart,
                anchor: PetFxAnchor::Head,
            });
        }
        sfx.write(PetSfx(PetSfxKind::Happy));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completion_notice_is_once_per_finished_session_and_never_on_stop_or_endless() {
        let mut focus = FocusSession::default();
        for preset in [FocusPreset::Study, FocusPreset::Work] {
            focus.apply(FocusCommand::Preset(preset));
            focus.apply(FocusCommand::Start);
            focus.advance(Duration::from_secs(1499));
            assert!(focus.completion_notice.is_none());
            focus.advance(Duration::from_secs(1));
            assert_eq!(focus.completion_notice.take(), Some(preset));
            focus.advance(Duration::from_secs(60));
            assert!(focus.completion_notice.is_none());
            focus.apply(FocusCommand::Start);
            focus.apply(FocusCommand::Stop);
            assert!(focus.completion_notice.is_none());
            focus.apply(FocusCommand::Length(FocusLength::Endless));
            focus.apply(FocusCommand::Start);
            focus.advance(Duration::from_secs(86400));
            assert!(focus.completion_notice.is_none());
            focus.apply(FocusCommand::Length(FocusLength::Minutes25));
        }
    }
    #[test]
    fn work_reuses_typing_and_releases_only_the_ride_it_requested() {
        assert_eq!(
            FocusPreset::Work.lazy_clip(),
            crate::animation::LazyNeraVrma::FocusTyping
        );
        let mut focus = FocusSession::default();
        focus.apply(FocusCommand::Preset(FocusPreset::Work));
        focus.apply(FocusCommand::Start);
        focus.apply(FocusCommand::Stop);
        assert!(
            !focus.release_ride,
            "a user's existing seat remains owned by its subsystem"
        );
        focus.apply(FocusCommand::Start);
        focus.session.as_mut().unwrap().ride_requested = true;
        focus.session.as_mut().unwrap().ride_observed = true;
        focus.apply(FocusCommand::Start);
        assert!(focus.session.as_ref().unwrap().ride_observed);
        assert!(
            !focus.release_ride,
            "restarting Work keeps the same requested Ride"
        );
        focus.apply(FocusCommand::Stop);
        assert!(focus.release_ride);
        focus.release_ride = false;
        focus.apply(FocusCommand::Start);
        focus.session.as_mut().unwrap().ride_requested = true;
        focus.advance(Duration::from_secs(1500));
        assert!(focus.completion_pending && focus.release_ride);
    }
    #[test]
    fn both_modes_repeat_safely_and_pause_countdown_during_interaction() {
        let mut focus = FocusSession::default();
        for _ in 0..8 {
            for preset in [FocusPreset::Study, FocusPreset::Work] {
                focus.apply(FocusCommand::Preset(preset));
                focus.apply(FocusCommand::Start);
                assert_eq!(focus.running_preset(), Some(preset));
                focus.session.as_mut().unwrap().paused = true;
                focus.advance(Duration::from_secs(15));
                assert_eq!(focus.remaining(), Some(Duration::from_secs(1500)));
                focus.session.as_mut().unwrap().paused = false;
                focus.advance(Duration::from_secs(15));
                assert_eq!(focus.remaining(), Some(Duration::from_secs(1485)));
                focus.apply(FocusCommand::Stop);
                assert!(!focus.active() && !focus.completion_pending);
                assert_eq!(focus.sfx_factor(PetSfxKind::Happy), 1.0);
            }
        }
    }
    #[test]
    fn preferences_restore_without_a_running_session_or_altering_behavior_settings() {
        let mut settings = crate::settings::NeraSettings {
            walking_enabled: true,
            master_volume: 73,
            ..default()
        };
        let mut focus = FocusSession::default();
        focus.apply(FocusCommand::Preset(FocusPreset::Work));
        focus.apply(FocusCommand::CustomDelta(12));
        focus.apply(FocusCommand::Start);
        focus.store_preferences(&mut settings);
        let mut restored = FocusSession::default();
        restored.restore_preferences(&settings);
        assert_eq!(restored.preset, FocusPreset::Work);
        assert_eq!(restored.length, FocusLength::Custom);
        assert_eq!(restored.custom_minutes, 47);
        assert!(!restored.active());
        assert!(settings.walking_enabled);
        assert_eq!(settings.master_volume, 73);
        settings.focus_mode = 255;
        settings.focus_duration = 255;
        settings.focus_custom_minutes = 0;
        restored.restore_preferences(&settings);
        assert_eq!(restored.preset, FocusPreset::Study);
        assert_eq!(restored.length, FocusLength::Minutes25);
        assert_eq!(restored.custom_minutes, 1);
    }
    #[test]
    fn critical_interruption_ends_the_session_without_completion_reward() {
        let mut focus = FocusSession::default();
        focus.apply(FocusCommand::Start);
        focus.interrupt("Alarm takes priority");
        assert!(!focus.active() && !focus.completion_pending);
        assert_eq!(focus.status(), "Alarm takes priority");
        assert_eq!(focus.sfx_factor(PetSfxKind::Happy), 1.0);
    }

    #[test]
    fn finite_presets_custom_and_endless_use_the_session_clock() {
        let mut focus = FocusSession::default();
        for (length, seconds) in [
            (FocusLength::Minutes25, 1500),
            (FocusLength::Minutes50, 3000),
            (FocusLength::Custom, 2100),
        ] {
            focus.apply(FocusCommand::Length(length));
            focus.apply(FocusCommand::Start);
            assert_eq!(focus.remaining(), Some(Duration::from_secs(seconds)));
            focus.advance(Duration::from_secs(77));
            assert_eq!(focus.remaining(), Some(Duration::from_secs(seconds - 77)));
            focus.advance(Duration::from_secs(seconds));
            assert!(!focus.active() && focus.completion_pending);
        }
        focus.apply(FocusCommand::Length(FocusLength::Endless));
        focus.apply(FocusCommand::Start);
        focus.advance(Duration::from_secs(86400));
        assert!(focus.active());
        assert_eq!(focus.remaining(), None);
        assert!(!focus.completion_pending);
    }

    #[test]
    fn stop_and_restart_clear_completion_and_restore_sound_without_changing_settings() {
        let settings = crate::settings::NeraSettings::default();
        let previous = (settings.walking_enabled, settings.master_volume);
        let mut focus = FocusSession::default();
        focus.apply(FocusCommand::Start);
        assert_eq!(focus.sfx_factor(PetSfxKind::Happy), 0.25);
        assert_eq!(focus.sfx_factor(PetSfxKind::HeadPat), 1.0);
        focus.advance(Duration::from_secs(1500));
        assert!(focus.completion_pending);
        focus.apply(FocusCommand::Stop);
        assert!(!focus.active() && !focus.completion_pending);
        assert_eq!(focus.sfx_factor(PetSfxKind::Happy), 1.0);
        focus.apply(FocusCommand::Start);
        assert!(!focus.completion_pending);
        assert_eq!((settings.walking_enabled, settings.master_volume), previous);
    }

    #[test]
    fn custom_duration_is_bounded_and_running_preset_is_not_edited_in_place() {
        let mut focus = FocusSession::default();
        focus.apply(FocusCommand::CustomDelta(-1000));
        assert_eq!(focus.custom_minutes, 1);
        focus.apply(FocusCommand::Start);
        focus.apply(FocusCommand::Preset(FocusPreset::Work));
        assert!(focus.status().starts_with("Study"));
        focus.apply(FocusCommand::CustomDelta(1000));
        assert_eq!(focus.custom_minutes, 480);
        assert_eq!(focus.remaining(), Some(Duration::from_secs(60)));
    }

    #[test]
    fn focus_clock_and_stop_never_change_a_real_alarm_deadline() {
        use crate::alarm::{AlarmState, AlarmStatus};
        let alarm = AlarmState::for_test(AlarmStatus::Armed {
            remaining: Duration::from_secs(300),
            duration: Duration::from_secs(300),
        });
        let now = std::time::Instant::now();
        let original = alarm.status_at(now);
        let mut focus = FocusSession::default();
        focus.apply(FocusCommand::Start);
        focus.advance(Duration::from_secs(1500));
        focus.apply(FocusCommand::Stop);
        assert_eq!(alarm.status_at(now), original);
    }
}
