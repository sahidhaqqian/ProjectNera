//! Project Nera — Phase 5.6 Alarm runtime.
//!
//! Flow:
//!
//! radial Alarm preset
//!      ↓
//! deadline
//!      ↓
//! Presentation::Attention(Alarm)
//!      ↓
//! alarm.mp3 loop
//!      ↓
//! JumpHappy once
//!      ↓
//! Wave loop
//!      ↓
//! presentation dismissed
//!      ↓
//! stop sound + stop alarm animation + restore DesktopPet

use std::time::{
    Duration,
    Instant,
};

use bevy::{
    audio::{
        AudioPlayer,
        AudioSink,
        AudioSinkPlayback,
        AudioSource,
        PlaybackSettings,
    },
    prelude::*,
};

use bevy_vrm1::prelude::VrmaHandle;

use crate::{
    action_menu::{
        AlarmMenuChoice,
        NeraActionMenuCommand,
    },
    animation::{
        AlarmAnimationCommand,
        register_alarm_jump_animation,
        register_alarm_wave_animation,
    },
    presentation::{
        AttentionKind,
        PresentationCommand,
        PresentationExited,
        PresentationState,
    },
    settings::{
        AlarmConfig,
        NeraSettings,
        save_settings_deferred,
    },
};

#[cfg(target_os = "windows")]
use crate::platform::{
    LocalDateTime,
    local_datetime,
};

const ALARM_SOUND_PATH: &str = "sound/alarm.mp3";
const ALARM_JUMP_VRMA_PATH: &str =
    "animations/animasi_basic/jump_happy.vrma";
const ALARM_WAVE_VRMA_PATH: &str =
    "animations/animasi_basic/161_Waving.vrma";

/// If the alarm asked for Attention but presentation never leaves DesktopPet,
/// something failed before the dismiss UI could be created. Never leave a
/// looping alarm with no reachable dismissal path.
const ALARM_ATTENTION_FAILSAFE: Duration = Duration::from_secs(2);

/// Default snooze interval used by the alarm panel.
pub(crate) const DEFAULT_SNOOZE_DURATION: Duration = Duration::from_secs(5 * 60);

#[derive(Resource, Clone, Debug)]
pub(crate) struct AlarmDisplayInfo {
    pub(crate) label: String,
    pub(crate) hour: u8,
    pub(crate) minute: u8,
}

impl Default for AlarmDisplayInfo {
    fn default() -> Self {
        Self {
            label: "Alarm".to_string(),
            hour: 7,
            minute: 30,
        }
    }
}

/// Runtime controls emitted by the alarm presentation UI.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlarmControlCommand {
    Snooze(Duration),
}

#[derive(Clone, Copy, Debug)]
enum AlarmPhase {
    Idle,

    Armed {
        deadline: Instant,
        duration: Duration,
    },

    Ringing {
        started_at: Instant,
    },
}

#[derive(Resource, Debug)]
pub(crate) struct AlarmState {
    phase: AlarmPhase,
}

impl Default for AlarmState {
    fn default() -> Self {
        Self {
            phase: AlarmPhase::Idle,
        }
    }
}

impl AlarmState {
    pub(crate) fn is_ringing(&self) -> bool {
        matches!(self.phase, AlarmPhase::Ringing { .. })
    }

    pub(crate) fn remaining(&self) -> Option<Duration> {
        let AlarmPhase::Armed { deadline, .. } = self.phase else {
            return None;
        };

        Some(deadline.saturating_duration_since(Instant::now()))
    }
}

#[derive(Resource)]
struct AlarmAssets {
    sound: Handle<AudioSource>,
}

/// Every alarm audio entity carries this marker.
///
/// Cleanup always stops/despawns *all* marked entities. This makes the alarm
/// robust even if a future bug accidentally creates more than one player.
#[derive(Component)]
struct AlarmAudioPlayback;

/// Alarm VRMAs must live under the Nera VRM entity, matching the proven
/// loading/retargeting pattern used by Idle/Sitting/reaction animations.
#[derive(Resource, Default)]
struct AlarmVrmaBootstrap {
    spawned: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AlarmMinuteKey {
    year: u16,
    month: u16,
    day: u16,
    hour: u16,
    minute: u16,
}

#[derive(Resource, Default)]
struct CustomAlarmRuntime {
    last_fired: Vec<(u32, AlarmMinuteKey)>,
    last_checked: Option<AlarmMinuteKey>,
    cached_book: String,
    cached_initialized: bool,
}

impl CustomAlarmRuntime {
    fn fired_this_minute(&self, id: u32, key: AlarmMinuteKey) -> bool {
        self.last_fired
            .iter()
            .any(|(stored_id, stored_key)| *stored_id == id && *stored_key == key)
    }

    fn mark_fired(&mut self, id: u32, key: AlarmMinuteKey) {
        if let Some((_, stored_key)) = self
            .last_fired
            .iter_mut()
            .find(|(stored_id, _)| *stored_id == id)
        {
            *stored_key = key;
        } else {
            self.last_fired.push((id, key));
        }
    }

    fn retain_ids(&mut self, alarms: &[AlarmConfig]) {
        self.last_fired
            .retain(|(id, _)| alarms.iter().any(|alarm| alarm.id == *id));
    }
}

pub(crate) struct AlarmPlugin;

impl Plugin for AlarmPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AlarmState>()
            .init_resource::<AlarmVrmaBootstrap>()
            .init_resource::<CustomAlarmRuntime>()
            .init_resource::<AlarmDisplayInfo>()
            .add_message::<AlarmControlCommand>()
            .add_systems(
                Startup,
                load_alarm_audio,
            )
            .add_systems(
                Update,
                (
                    ensure_alarm_vrmas_under_nera,
                    consume_alarm_menu_commands,
                    consume_alarm_control_commands,
                    tick_custom_alarm_schedule,
                    tick_alarm_deadline,
                    cleanup_after_presentation_exit,
                    abort_ringing_if_attention_never_started,
                )
                    .chain(),
            );
    }
}

fn load_alarm_audio(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    commands.insert_resource(AlarmAssets {
        sound: asset_server.load(ALARM_SOUND_PATH),
    });

    info!(
        "Nera alarm audio requested | {}",
        ALARM_SOUND_PATH,
    );
}

/// Attach alarm VRMAs to the actual Nera VRM root.
///
/// Existing Project Nera animations are loaded as children of the entity named
/// "Nera". Doing the same here gives bevy_vrm1 the VRM ancestry it needs for
/// retargeting instead of creating unrelated standalone VRMA worlds.
fn ensure_alarm_vrmas_under_nera(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut bootstrap: ResMut<AlarmVrmaBootstrap>,
    named_entities: Query<(Entity, &Name)>,
) {
    if bootstrap.spawned {
        return;
    }

    let Some((nera, _)) = named_entities
        .iter()
        .find(|(_, name)| name.as_str() == "Nera")
    else {
        return;
    };

    commands.entity(nera).with_children(|parent| {
        parent
            .spawn((
                Name::new("Nera Alarm JumpHappy"),
                VrmaHandle(
                    asset_server.load(ALARM_JUMP_VRMA_PATH),
                ),
            ))
            .observe(register_alarm_jump_animation);

        parent
            .spawn((
                Name::new("Nera Alarm Wave"),
                VrmaHandle(
                    asset_server.load(ALARM_WAVE_VRMA_PATH),
                ),
            ))
            .observe(register_alarm_wave_animation);
    });

    bootstrap.spawned = true;

    info!(
        "Nera alarm VRMAs attached to VRM root | jump={} wave={}",
        ALARM_JUMP_VRMA_PATH,
        ALARM_WAVE_VRMA_PATH,
    );
}

fn preset_duration(choice: AlarmMenuChoice) -> Option<Duration> {
    match choice {
        AlarmMenuChoice::FiveMinutes => {
            Some(Duration::from_secs(5 * 60))
        }
        AlarmMenuChoice::TenMinutes => {
            Some(Duration::from_secs(10 * 60))
        }
        AlarmMenuChoice::FifteenMinutes => {
            Some(Duration::from_secs(15 * 60))
        }
        AlarmMenuChoice::ThirtyMinutes => {
            Some(Duration::from_secs(30 * 60))
        }
        AlarmMenuChoice::FortyFiveMinutes => {
            Some(Duration::from_secs(45 * 60))
        }
        AlarmMenuChoice::OneHour => {
            Some(Duration::from_secs(60 * 60))
        }
        AlarmMenuChoice::TwoHours => {
            Some(Duration::from_secs(2 * 60 * 60))
        }

        // Zero delay fires on the next `tick_alarm_deadline`, so the test
        // action reuses the one real trigger path instead of duplicating it.
        // This replaces the old Ctrl+Alt+A debug shortcut.
        AlarmMenuChoice::TestNow => Some(Duration::ZERO),

        AlarmMenuChoice::Cancel => None,
    }
}

fn consume_alarm_menu_commands(
    mut menu_commands: MessageReader<NeraActionMenuCommand>,
    mut state: ResMut<AlarmState>,
    mut display: ResMut<AlarmDisplayInfo>,
    mut presentation: MessageWriter<PresentationCommand>,
) {
    for command in menu_commands.read() {
        let NeraActionMenuCommand::Alarm(choice) = *command else {
            continue;
        };

        match choice {
            AlarmMenuChoice::Cancel => {
                match state.phase {
                    AlarmPhase::Idle => {
                        info!("Alarm cancel requested: no active alarm");
                    }

                    AlarmPhase::Armed { .. } => {
                        state.phase = AlarmPhase::Idle;
                        info!("Nera alarm cancelled");
                    }

                    AlarmPhase::Ringing { .. } => {
                        // Cleanup happens from PresentationExited so all alarm
                        // exit paths have one single source of truth.
                        presentation.write(
                            PresentationCommand::ExitAttention,
                        );
                    }
                }
            }

            preset => {
                if state.is_ringing() {
                    warn!(
                        "Alarm preset {:?} ignored: alarm is already ringing;                          dismiss it first",
                        preset,
                    );
                    continue;
                }

                let Some(duration) = preset_duration(preset) else {
                    continue;
                };

                state.phase = AlarmPhase::Armed {
                    deadline: Instant::now() + duration,
                    duration,
                };
                display.label = "Quick Alarm".to_string();

                info!(
                    "Nera alarm armed -> {:?} ({:?})",
                    preset,
                    duration,
                );
            }
        }
    }
}

fn consume_alarm_control_commands(
    mut commands: Commands,
    mut controls: MessageReader<AlarmControlCommand>,
    mut state: ResMut<AlarmState>,
    mut presentation: MessageWriter<PresentationCommand>,
    mut animation: MessageWriter<AlarmAnimationCommand>,
    alarm_audio: Query<
        (Entity, Option<&AudioSink>),
        With<AlarmAudioPlayback>,
    >,
) {
    for control in controls.read() {
        let AlarmControlCommand::Snooze(duration) = *control;

        if !state.is_ringing() {
            warn!("Alarm snooze ignored: alarm is not currently ringing");
            continue;
        }

        let mut stopped = 0usize;

        // Snooze is a real lifecycle transition, not a second alarm player:
        // stop the current presentation/audio first, then arm the next deadline.
        for (entity, sink) in &alarm_audio {
            if let Some(sink) = sink {
                sink.stop();
            }

            commands.entity(entity).despawn();
            stopped += 1;
        }

        animation.write(AlarmAnimationCommand::Stop);

        state.phase = AlarmPhase::Armed {
            deadline: Instant::now() + duration,
            duration,
        };

        // Presentation teardown restores the exact DesktopPet snapshot. The
        // normal PresentationExited alarm cleanup intentionally sees Armed here
        // and leaves the newly scheduled snooze intact.
        presentation.write(PresentationCommand::ExitAttention);

        info!(
            "Nera alarm snoozed for {:?} | {} audio player(s) stopped",
            duration,
            stopped,
        );
    }
}

#[cfg(target_os = "windows")]
fn repeat_bit_for_windows_day(day_of_week: u16) -> Option<u8> {
    Some(match day_of_week {
        // Windows SYSTEMTIME: Sunday=0, Monday=1, ... Saturday=6.
        0 => 6,
        1 => 0,
        2 => 1,
        3 => 2,
        4 => 3,
        5 => 4,
        6 => 5,
        _ => return None,
    })
}

#[cfg(target_os = "windows")]
fn custom_alarm_matches(alarm: &AlarmConfig, now: LocalDateTime) -> bool {
    if !alarm.enabled
        || now.hour != alarm.hour as u16
        || now.minute != alarm.minute as u16
    {
        return false;
    }

    if alarm.mode == 0 {
        return now.year == alarm.year
            && now.month == alarm.month as u16
            && now.day == alarm.day as u16;
    }

    let Some(repeat_bit) = repeat_bit_for_windows_day(now.day_of_week) else {
        return false;
    };

    alarm.repeat_days & (1 << repeat_bit) != 0
}

#[cfg(target_os = "windows")]
fn tick_custom_alarm_schedule(
    mut commands: Commands,
    mut settings: ResMut<NeraSettings>,
    mut state: ResMut<AlarmState>,
    mut runtime: ResMut<CustomAlarmRuntime>,
    mut display: ResMut<AlarmDisplayInfo>,
) {
    if !matches!(state.phase, AlarmPhase::Idle) {
        return;
    }

    let alarm_data_changed = runtime.cached_book != settings.alarm_book
        || runtime.cached_initialized != settings.alarm_book_initialized;

    let now = local_datetime();
    let key = AlarmMinuteKey {
        year: now.year,
        month: now.month,
        day: now.day,
        hour: now.hour,
        minute: now.minute,
    };

    // Alarm schedules have minute precision. Avoid parsing the persistent alarm
    // book every render frame; re-evaluate only when the minute rolls over or
    // the user changes alarm settings.
    if runtime.last_checked == Some(key) && !alarm_data_changed {
        return;
    }
    runtime.last_checked = Some(key);
    runtime.cached_book.clone_from(&settings.alarm_book);
    runtime.cached_initialized = settings.alarm_book_initialized;

    let mut alarms = settings.alarm_configs();
    runtime.retain_ids(&alarms);

    if alarms.is_empty() {
        return;
    }

    let Some(index) = alarms
        .iter()
        .position(|alarm| {
            custom_alarm_matches(alarm, now)
                && !runtime.fired_this_minute(alarm.id, key)
        })
    else {
        return;
    };

    let due = alarms[index].clone();
    runtime.mark_fired(due.id, key);

    if due.mode == 0 {
        alarms[index].enabled = false;
        settings.store_alarm_configs(&alarms);
        save_settings_deferred(&mut commands);
    }

    display.label = due.label.clone();
    display.hour = due.hour;
    display.minute = due.minute;

    state.phase = AlarmPhase::Armed {
        deadline: Instant::now(),
        duration: Duration::ZERO,
    };

    info!(
        "Nera custom alarm due -> id={} label={} {:02}:{:02}",
        due.id,
        due.label,
        due.hour,
        due.minute,
    );
}

#[cfg(not(target_os = "windows"))]
fn tick_custom_alarm_schedule() {}

fn tick_alarm_deadline(
    mut commands: Commands,
    assets: Res<AlarmAssets>,
    mut state: ResMut<AlarmState>,
    mut presentation: MessageWriter<PresentationCommand>,
    mut animation: MessageWriter<AlarmAnimationCommand>,
    mut display: ResMut<AlarmDisplayInfo>,
    existing_audio: Query<
        (Entity, Option<&AudioSink>),
        With<AlarmAudioPlayback>,
    >,
) {
    let AlarmPhase::Armed {
        deadline,
        duration,
    } = state.phase
    else {
        return;
    };

    if Instant::now() < deadline {
        return;
    }

    #[cfg(target_os = "windows")]
    if display.label == "Quick Alarm" {
        let now = local_datetime();
        display.hour = now.hour as u8;
        display.minute = now.minute as u8;
    }

    // Defense-in-depth: there must never be more than one alarm player.
    // Clean any orphan left by an interrupted/reloaded alarm session.
    for (entity, sink) in &existing_audio {
        if let Some(sink) = sink {
            sink.stop();
        }
        commands.entity(entity).despawn();
        warn!(
            "Removed orphan Nera alarm audio entity {:?} before trigger",
            entity,
        );
    }

    commands.spawn((
        Name::new("Nera Alarm Audio"),
        AlarmAudioPlayback,
        AudioPlayer::new(assets.sound.clone()),
        PlaybackSettings::LOOP,
    ));

    state.phase = AlarmPhase::Ringing {
        started_at: Instant::now(),
    };

    presentation.write(
        PresentationCommand::EnterAttention(
            AttentionKind::Alarm,
        ),
    );

    animation.write(AlarmAnimationCommand::Start);

    info!(
        "Nera alarm TRIGGERED | original_duration={:?} | sound=loop",
        duration
    );
}

fn cleanup_after_presentation_exit(
    mut commands: Commands,
    mut exited: MessageReader<PresentationExited>,
    mut state: ResMut<AlarmState>,
    mut animation: MessageWriter<AlarmAnimationCommand>,
    alarm_audio: Query<
        (Entity, Option<&AudioSink>),
        With<AlarmAudioPlayback>,
    >,
) {
    for event in exited.read() {
        if event.kind != AttentionKind::Alarm {
            continue;
        }

        let AlarmPhase::Ringing { .. } = state.phase else {
            continue;
        };

        let mut stopped = 0usize;

        // Stop every alarm player, not merely the latest one. Normally this
        // loop contains exactly one entity.
        for (entity, sink) in &alarm_audio {
            if let Some(sink) = sink {
                sink.stop();
            }

            commands.entity(entity).despawn();
            stopped += 1;
        }

        animation.write(AlarmAnimationCommand::Stop);
        state.phase = AlarmPhase::Idle;

        info!(
            "Nera alarm dismissed -> {} audio player(s) stopped/despawned,              animation stopped",
            stopped,
        );
    }
}

fn should_abort_failed_attention(
    presentation_is_desktop_pet: bool,
    ringing_for: Duration,
) -> bool {
    presentation_is_desktop_pet
        && ringing_for >= ALARM_ATTENTION_FAILSAFE
}

/// Defense-in-depth for a failed Attention transition.  The dismiss box is
/// spawned from `PresentationEntered`, so if presentation never leaves
/// DesktopPet there is otherwise no clickable path that can stop the looping
/// audio.  This is intentionally alarm-owned rather than dismiss-window-owned:
/// it also covers failures that happen before that window can be requested.
fn abort_ringing_if_attention_never_started(
    mut commands: Commands,
    presentation_state: Res<PresentationState>,
    mut state: ResMut<AlarmState>,
    mut animation: MessageWriter<AlarmAnimationCommand>,
    alarm_audio: Query<
        (Entity, Option<&AudioSink>),
        With<AlarmAudioPlayback>,
    >,
) {
    let AlarmPhase::Ringing { started_at } = state.phase else {
        return;
    };

    if !should_abort_failed_attention(
        presentation_state.is_desktop_pet(),
        started_at.elapsed(),
    ) {
        return;
    }

    let mut stopped = 0usize;
    for (entity, sink) in &alarm_audio {
        if let Some(sink) = sink {
            sink.stop();
        }

        commands.entity(entity).despawn();
        stopped += 1;
    }

    animation.write(AlarmAnimationCommand::Stop);
    state.phase = AlarmPhase::Idle;

    error!(
        "Nera alarm Attention failed to start within {:?}; stopped {} audio player(s) and returned alarm runtime to Idle",
        ALARM_ATTENTION_FAILSAFE,
        stopped,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_durations_are_correct() {
        assert_eq!(
            preset_duration(AlarmMenuChoice::FiveMinutes),
            Some(Duration::from_secs(300)),
        );
        assert_eq!(
            preset_duration(AlarmMenuChoice::TenMinutes),
            Some(Duration::from_secs(600)),
        );
        assert_eq!(
            preset_duration(AlarmMenuChoice::FifteenMinutes),
            Some(Duration::from_secs(900)),
        );
        assert_eq!(
            preset_duration(AlarmMenuChoice::ThirtyMinutes),
            Some(Duration::from_secs(1800)),
        );
        assert_eq!(
            preset_duration(AlarmMenuChoice::FortyFiveMinutes),
            Some(Duration::from_secs(2700)),
        );
        assert_eq!(
            preset_duration(AlarmMenuChoice::OneHour),
            Some(Duration::from_secs(3600)),
        );
        assert_eq!(
            preset_duration(AlarmMenuChoice::TwoHours),
            Some(Duration::from_secs(7200)),
        );
    }

    /// "Test Now" must arm an already-expired deadline so the next
    /// `tick_alarm_deadline` fires it through the single real trigger path.
    #[test]
    fn test_now_arms_an_already_expired_deadline() {
        let duration = preset_duration(AlarmMenuChoice::TestNow)
            .expect("Test Now is a timer preset");

        assert_eq!(duration, Duration::ZERO);

        let deadline = Instant::now() + duration;
        assert!(Instant::now() >= deadline);
    }

    #[test]
    fn default_snooze_is_five_minutes() {
        assert_eq!(
            DEFAULT_SNOOZE_DURATION,
            Duration::from_secs(300),
        );
    }

    #[test]
    fn cancel_is_not_a_timer_preset() {
        assert_eq!(
            preset_duration(AlarmMenuChoice::Cancel),
            None,
        );
    }

    #[test]
    fn idle_alarm_is_not_ringing() {
        let state = AlarmState::default();
        assert!(!state.is_ringing());
        assert!(state.remaining().is_none());
    }
    #[test]
    fn ringing_state_is_detected_without_audio_entity_identity() {
        let state = AlarmState {
            phase: AlarmPhase::Ringing {
                started_at: Instant::now(),
            },
        };

        assert!(state.is_ringing());
        assert!(state.remaining().is_none());
    }


    #[cfg(target_os = "windows")]
    #[test]
    fn custom_one_time_alarm_matches_only_exact_local_minute_and_date() {
        let alarm = AlarmConfig {
            id: 7,
            enabled: true,
            label: "Study".into(),
            hour: 7,
            minute: 30,
            mode: 0,
            year: 2026,
            month: 9,
            day: 23,
            repeat_days: 0,
        };

        let exact = LocalDateTime {
            year: 2026,
            month: 9,
            day: 23,
            day_of_week: 3,
            hour: 7,
            minute: 30,
        };
        assert!(custom_alarm_matches(&alarm, exact));
        assert!(!custom_alarm_matches(&alarm, LocalDateTime { day: 24, ..exact }));
        assert!(!custom_alarm_matches(&alarm, LocalDateTime { minute: 31, ..exact }));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn custom_weekly_alarm_uses_monday_to_sunday_bit_order() {
        let alarm = AlarmConfig {
            id: 8,
            enabled: true,
            label: "Work".into(),
            hour: 9,
            minute: 0,
            mode: 1,
            year: 2026,
            month: 9,
            day: 21,
            repeat_days: (1 << 0) | (1 << 4),
        };

        let monday = LocalDateTime {
            year: 2026,
            month: 9,
            day: 21,
            day_of_week: 1,
            hour: 9,
            minute: 0,
        };
        assert!(custom_alarm_matches(&alarm, monday));
        assert!(custom_alarm_matches(&alarm, LocalDateTime { day: 25, day_of_week: 5, ..monday }));
        assert!(!custom_alarm_matches(&alarm, LocalDateTime { day: 27, day_of_week: 0, ..monday }));
    }

    #[test]
    fn failed_attention_failsafe_only_applies_while_still_desktop_pet() {
        assert!(!should_abort_failed_attention(
            true,
            ALARM_ATTENTION_FAILSAFE - Duration::from_millis(1),
        ));
        assert!(should_abort_failed_attention(
            true,
            ALARM_ATTENTION_FAILSAFE,
        ));
        assert!(!should_abort_failed_attention(
            false,
            ALARM_ATTENTION_FAILSAFE + Duration::from_secs(30),
        ));
    }
}
