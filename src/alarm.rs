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
//! selected bundled alarm sound, repeated until dismissed
//!      ↓
//! JumpHappy once
//!      ↓
//! Wave loop
//!      ↓
//! presentation dismissed
//!      ↓
//! stop sound + stop alarm animation + restore DesktopPet

use std::time::{Duration, Instant};
pub(crate) mod sounds;

use bevy::{
    audio::{AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings},
    prelude::*,
};

use crate::{
    action_menu::{AlarmMenuChoice, NeraActionMenuCommand},
    animation::AlarmAnimationCommand,
    presentation::{AttentionKind, PresentationCommand, PresentationExited, PresentationState},
    settings::{AlarmConfig, NeraSettings, save_settings_deferred},
};

#[cfg(target_os = "windows")]
use crate::platform::{LocalDateTime, local_datetime};

const ALARM_SOUND_PATH: &str = "sound/classic.wav";
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
    pub(crate) sound: u16,
}

impl Default for AlarmDisplayInfo {
    fn default() -> Self {
        Self {
            label: "Alarm".to_string(),
            hour: 7,
            minute: 30,
            sound: 1,
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

/// Read-only projection for UI; deadlines remain owned by `AlarmState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlarmStatus {
    Idle,
    Armed {
        remaining: Duration,
        duration: Duration,
    },
    Ringing,
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct AlarmRuntimeSet;

#[derive(Resource, Debug)]
pub(crate) struct AlarmState {
    phase: AlarmPhase,
    pending: Vec<PendingAlarm>,
}

#[derive(Debug)]
struct PendingAlarm {
    deadline: Instant,
    duration: Duration,
    display: AlarmDisplayInfo,
}

impl Default for AlarmState {
    fn default() -> Self {
        Self {
            phase: AlarmPhase::Idle,
            pending: Vec::new(),
        }
    }
}

impl AlarmState {
    #[cfg(test)]
    pub(crate) fn for_test(status: AlarmStatus) -> Self {
        let phase = match status {
            AlarmStatus::Idle => AlarmPhase::Idle,
            AlarmStatus::Armed {
                remaining,
                duration,
            } => AlarmPhase::Armed {
                deadline: Instant::now() + remaining,
                duration,
            },
            AlarmStatus::Ringing => AlarmPhase::Ringing {
                started_at: Instant::now(),
            },
        };
        Self {
            phase,
            pending: Vec::new(),
        }
    }

    pub(crate) fn is_ringing(&self) -> bool {
        matches!(self.phase, AlarmPhase::Ringing { .. })
    }

    pub(crate) fn remaining(&self) -> Option<Duration> {
        let AlarmStatus::Armed { remaining, .. } = self.status_at(Instant::now()) else {
            return None;
        };
        Some(remaining)
    }

    pub(crate) fn status_at(&self, now: Instant) -> AlarmStatus {
        match self.phase {
            AlarmPhase::Idle => AlarmStatus::Idle,
            AlarmPhase::Armed { deadline, duration } => AlarmStatus::Armed {
                remaining: deadline.saturating_duration_since(now),
                duration,
            },
            AlarmPhase::Ringing { .. } => AlarmStatus::Ringing,
        }
    }

    /// Scheduled alarms can interrupt a future quick/snooze timer, but never
    /// another ringing alarm. Preserve that timer's deadline and label while
    /// delivering pending alarms in deadline order.
    fn advance_deadline(
        &mut self,
        now: Instant,
        display: &mut AlarmDisplayInfo,
    ) -> Option<Duration> {
        if self.is_ringing() {
            return None;
        }

        if let Some(index) = self
            .pending
            .iter()
            .enumerate()
            .min_by_key(|(_, alarm)| alarm.deadline)
            .map(|(index, _)| index)
        {
            let promote = match self.phase {
                AlarmPhase::Idle => true,
                AlarmPhase::Armed { deadline, .. } => self.pending[index].deadline < deadline,
                AlarmPhase::Ringing { .. } => false,
            };
            if promote {
                let next = self.pending.remove(index);
                if let AlarmPhase::Armed { deadline, duration } = self.phase {
                    self.pending.push(PendingAlarm {
                        deadline,
                        duration,
                        display: display.clone(),
                    });
                }
                *display = next.display;
                self.phase = AlarmPhase::Armed {
                    deadline: next.deadline,
                    duration: next.duration,
                };
            }
        }

        let AlarmPhase::Armed { deadline, duration } = self.phase else {
            return None;
        };
        if now < deadline {
            return None;
        }
        self.phase = AlarmPhase::Ringing { started_at: now };
        Some(duration)
    }
}

#[derive(Resource)]
struct AlarmAssets {
    sounds: [Handle<AudioSource>; sounds::SOUNDS.len()],
}

/// Every alarm audio entity carries this marker.
///
/// Cleanup always stops/despawns *all* marked entities. This makes the alarm
/// robust even if a future bug accidentally creates more than one player.
#[derive(Component)]
struct AlarmAudioPlayback;

#[derive(Component)]
struct AlarmRepeatAfter(Instant);

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
            .init_resource::<CustomAlarmRuntime>()
            .init_resource::<AlarmDisplayInfo>()
            .init_resource::<sounds::AlarmSoundLibrary>()
            .add_message::<AlarmControlCommand>()
            .add_message::<sounds::PreviewAlarmSound>()
            .add_systems(Update, (sounds::preview_sound, repeat_alarm_sound))
            .add_systems(Startup, load_alarm_audio)
            .add_systems(
                Update,
                (
                    consume_alarm_menu_commands,
                    consume_alarm_control_commands,
                    tick_custom_alarm_schedule,
                )
                    .chain(),
            )
            // All previous presentation/panel cleanup runs in Update. A queued
            // alarm must not reuse Attention while the old panel is closing.
            .add_systems(
                PostUpdate,
                (
                    cleanup_after_presentation_exit,
                    abort_ringing_if_attention_never_started,
                    tick_alarm_deadline,
                )
                    .chain()
                    .in_set(AlarmRuntimeSet),
            );
    }
}

fn load_alarm_audio(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(AlarmAssets {
        sounds: std::array::from_fn(|index| {
            asset_server.load(sounds::path(sounds::BUNDLED_SOUND_IDS[index]))
        }),
    });

    info!("Nera alarm audio requested | {}", ALARM_SOUND_PATH,);
}

#[cfg(all(debug_assertions, target_os = "windows"))]
pub(crate) fn memory_audit_counts(world: &mut World) -> (usize, usize, usize) {
    (
        world.query::<&AlarmAudioPlayback>().iter(world).count(),
        world
            .query::<&sounds::AlarmSoundPreview>()
            .iter(world)
            .count(),
        world
            .resource::<Assets<AudioSource>>()
            .iter()
            .map(|(_, audio)| audio.bytes.len())
            .sum(),
    )
}

#[cfg(all(debug_assertions, target_os = "windows"))]
pub(crate) fn memory_audit_ring_source(world: &mut World) -> Option<AssetId<AudioSource>> {
    world
        .query_filtered::<&AudioPlayer, With<AlarmAudioPlayback>>()
        .iter(world)
        .next()
        .map(|player| player.0.id())
}

pub(crate) fn preset_duration(choice: AlarmMenuChoice) -> Option<Duration> {
    match choice {
        AlarmMenuChoice::FiveMinutes => Some(Duration::from_secs(5 * 60)),
        AlarmMenuChoice::TenMinutes => Some(Duration::from_secs(10 * 60)),
        AlarmMenuChoice::FifteenMinutes => Some(Duration::from_secs(15 * 60)),
        AlarmMenuChoice::ThirtyMinutes => Some(Duration::from_secs(30 * 60)),
        AlarmMenuChoice::FortyFiveMinutes => Some(Duration::from_secs(45 * 60)),
        AlarmMenuChoice::OneHour => Some(Duration::from_secs(60 * 60)),
        AlarmMenuChoice::TwoHours => Some(Duration::from_secs(2 * 60 * 60)),

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
    settings: Res<NeraSettings>,
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
                        presentation.write(PresentationCommand::ExitAttention);
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
                display.sound = settings.alarm_sound;
                // Presets are whole minutes. Record their actual local due
                // time when accepted, so the editor can show it immediately.
                #[cfg(target_os = "windows")]
                {
                    set_timer_display_time(&mut display, duration);
                }

                info!("Nera alarm armed -> {:?} ({:?})", preset, duration,);
            }
        }
    }
}

fn consume_alarm_control_commands(
    mut commands: Commands,
    mut controls: MessageReader<AlarmControlCommand>,
    mut state: ResMut<AlarmState>,
    mut display: ResMut<AlarmDisplayInfo>,
    mut presentation: MessageWriter<PresentationCommand>,
    mut animation: MessageWriter<AlarmAnimationCommand>,
    alarm_audio: Query<(Entity, Option<&AudioSink>), With<AlarmAudioPlayback>>,
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
        #[cfg(target_os = "windows")]
        set_timer_display_time(&mut display, duration);
        presentation.write(PresentationCommand::ExitAttention);

        info!(
            "Nera alarm snoozed for {:?} | {} audio player(s) stopped",
            duration, stopped,
        );
    }
}

#[cfg(target_os = "windows")]
fn set_timer_display_time(display: &mut AlarmDisplayInfo, duration: Duration) {
    let now = local_datetime();
    let minutes =
        (u64::from(now.hour) * 60 + u64::from(now.minute) + duration.as_secs() / 60) % (24 * 60);
    display.hour = (minutes / 60) as u8;
    display.minute = (minutes % 60) as u8;
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
    if !alarm.enabled || now.hour != alarm.hour as u16 || now.minute != alarm.minute as u16 {
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
) {
    // Reading the alarm book every frame must not mark the entire settings
    // resource changed. That used to wake unrelated audio/FPS systems and spam
    // their INFO logs at the render rate.
    let settings_changed = collect_due_alarms(
        settings.bypass_change_detection(),
        &mut state,
        &mut runtime,
        local_datetime(),
        Instant::now(),
    );
    if settings_changed {
        settings.set_changed();
        save_settings_deferred(&mut commands);
    }
}

#[cfg(target_os = "windows")]
fn collect_due_alarms(
    settings: &mut NeraSettings,
    state: &mut AlarmState,
    runtime: &mut CustomAlarmRuntime,
    now: LocalDateTime,
    deadline: Instant,
) -> bool {
    let alarm_data_changed = runtime.cached_book != settings.alarm_book
        || runtime.cached_initialized != settings.alarm_book_initialized;

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
        return false;
    }
    runtime.last_checked = Some(key);
    runtime.cached_book.clone_from(&settings.alarm_book);
    runtime.cached_initialized = settings.alarm_book_initialized;

    let mut alarms = settings.alarm_configs();
    runtime.retain_ids(&alarms);

    let mut settings_changed = false;
    for due in &mut alarms {
        if !custom_alarm_matches(due, now) || runtime.fired_this_minute(due.id, key) {
            continue;
        }
        // Mark on enqueue, not presentation: queued alarms may wait across
        // minutes while the user dismisses or snoozes an earlier alarm.
        runtime.mark_fired(due.id, key);
        state.pending.push(PendingAlarm {
            deadline,
            duration: Duration::ZERO,
            display: AlarmDisplayInfo {
                label: due.label.clone(),
                hour: due.hour,
                minute: due.minute,
                sound: due.sound,
            },
        });
        if due.mode == 0 {
            due.enabled = false;
            settings_changed = true;
        }
        info!(
            "Nera custom alarm queued -> id={} label={} {:02}:{:02}",
            due.id, due.label, due.hour, due.minute
        );
    }
    if settings_changed {
        settings.store_alarm_configs(&alarms);
        runtime.cached_book.clone_from(&settings.alarm_book);
        runtime.cached_initialized = settings.alarm_book_initialized;
    }
    settings_changed
}

#[cfg(not(target_os = "windows"))]
fn tick_custom_alarm_schedule() {}

fn tick_alarm_deadline(
    mut commands: Commands,
    assets: Res<AlarmAssets>,
    mut library: ResMut<sounds::AlarmSoundLibrary>,
    mut audio: ResMut<Assets<AudioSource>>,
    mut state: ResMut<AlarmState>,
    mut presentation: MessageWriter<PresentationCommand>,
    mut animation: MessageWriter<AlarmAnimationCommand>,
    mut display: ResMut<AlarmDisplayInfo>,
    presentation_state: Res<PresentationState>,
    #[cfg(target_os = "windows")] dismiss_runtime: Res<crate::alarm_dismiss::AlarmDismissRuntime>,
    #[cfg(target_os = "windows")] dismiss_panels: Query<
        (),
        With<crate::alarm_dismiss::AlarmDismissWindow>,
    >,
    existing_audio: Query<(Entity, Option<&AudioSink>), With<AlarmAudioPlayback>>,
) {
    if !presentation_state.is_desktop_pet() && !presentation_state.is_screensaver() {
        return;
    }
    #[cfg(target_os = "windows")]
    if !dismiss_runtime.is_closed() || !dismiss_panels.is_empty() {
        // Native close can remove the entity before Update consumes the old
        // presentation exit and resets the panel's ownership. Starting here
        // would let stale runtime.window suppress the next panel's spawn.
        return;
    }
    let Some(duration) = state.advance_deadline(Instant::now(), &mut display) else {
        return;
    };

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

    let sound = library.resolve(display.sound, &assets.sounds, &mut audio);
    commands.spawn((
        Name::new("Nera Alarm Audio"),
        AlarmAudioPlayback,
        AudioPlayer::new(sound),
        PlaybackSettings::ONCE,
        AlarmRepeatAfter(Instant::now() + Duration::from_secs(3)),
    ));

    presentation.write(PresentationCommand::EnterAttention(AttentionKind::Alarm));

    animation.write(AlarmAnimationCommand::Start);

    info!(
        "Nera alarm TRIGGERED | original_duration={:?} | sound=loop",
        duration
    );
}

fn repeat_alarm_sound(
    mut commands: Commands,
    state: Res<AlarmState>,
    players: Query<(Entity, &AudioSink, &AudioPlayer, &AlarmRepeatAfter), With<AlarmAudioPlayback>>,
) {
    if !state.is_ringing() {
        return;
    }
    for (entity, sink, player, repeat) in &players {
        if sink.empty() && Instant::now() >= repeat.0 {
            commands.entity(entity).despawn();
            commands.spawn((
                AlarmAudioPlayback,
                AlarmRepeatAfter(Instant::now() + Duration::from_secs(3)),
                AudioPlayer::new(player.0.clone()),
                PlaybackSettings::ONCE,
            ));
        }
    }
}

fn cleanup_after_presentation_exit(
    mut commands: Commands,
    mut exited: MessageReader<PresentationExited>,
    mut state: ResMut<AlarmState>,
    mut animation: MessageWriter<AlarmAnimationCommand>,
    alarm_audio: Query<(Entity, Option<&AudioSink>), With<AlarmAudioPlayback>>,
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

fn should_abort_failed_attention(presentation_is_desktop_pet: bool, ringing_for: Duration) -> bool {
    presentation_is_desktop_pet && ringing_for >= ALARM_ATTENTION_FAILSAFE
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
    alarm_audio: Query<(Entity, Option<&AudioSink>), With<AlarmAudioPlayback>>,
) {
    let AlarmPhase::Ringing { started_at } = state.phase else {
        return;
    };

    if !should_abort_failed_attention(presentation_state.is_desktop_pet(), started_at.elapsed()) {
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
        ALARM_ATTENTION_FAILSAFE, stopped,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_quick_preset_updates_status_and_due_time_cancel_clears_it_and_ringing_rejects_it() {
        let mut app = App::new();
        app.init_resource::<AlarmState>()
            .init_resource::<AlarmDisplayInfo>()
            .init_resource::<NeraSettings>()
            .add_message::<NeraActionMenuCommand>()
            .add_message::<PresentationCommand>()
            .add_systems(Update, consume_alarm_menu_commands);
        let saved = app.world().resource::<NeraSettings>().alarm_book.clone();
        app.world_mut()
            .write_message(NeraActionMenuCommand::Alarm(AlarmMenuChoice::FiveMinutes));
        app.update();
        assert!(
            matches!(app.world().resource::<AlarmState>().status_at(Instant::now()),
            AlarmStatus::Armed { remaining, duration }
            if remaining > Duration::from_secs(299) && duration == Duration::from_secs(300))
        );
        #[cfg(target_os = "windows")]
        {
            let mut expected = AlarmDisplayInfo::default();
            set_timer_display_time(&mut expected, Duration::from_secs(300));
            let display = app.world().resource::<AlarmDisplayInfo>();
            assert_eq!(
                (display.hour, display.minute),
                (expected.hour, expected.minute)
            );
        }
        app.world_mut()
            .write_message(NeraActionMenuCommand::Alarm(AlarmMenuChoice::Cancel));
        app.update();
        assert_eq!(
            app.world()
                .resource::<AlarmState>()
                .status_at(Instant::now()),
            AlarmStatus::Idle
        );
        app.insert_resource(AlarmState::for_test(AlarmStatus::Ringing));
        let old = app.world().resource::<AlarmDisplayInfo>().clone();
        app.world_mut()
            .write_message(NeraActionMenuCommand::Alarm(AlarmMenuChoice::TenMinutes));
        app.update();
        assert_eq!(
            app.world()
                .resource::<AlarmState>()
                .status_at(Instant::now()),
            AlarmStatus::Ringing
        );
        let display = app.world().resource::<AlarmDisplayInfo>();
        assert_eq!(
            (display.hour, display.minute, &display.label),
            (old.hour, old.minute, &old.label)
        );
        assert_eq!(app.world().resource::<NeraSettings>().alarm_book, saved);
    }

    fn pending_alarm(label: &str, deadline: Instant) -> PendingAlarm {
        PendingAlarm {
            deadline,
            duration: Duration::ZERO,
            display: AlarmDisplayInfo {
                label: label.into(),
                ..default()
            },
        }
    }

    #[cfg(target_os = "windows")]
    fn scheduled_time() -> LocalDateTime {
        LocalDateTime {
            year: 2026,
            month: 9,
            day: 23,
            day_of_week: 3,
            hour: 7,
            minute: 30,
        }
    }

    #[cfg(target_os = "windows")]
    fn scheduled_alarm(id: u32, mode: u8) -> AlarmConfig {
        AlarmConfig {
            id,
            enabled: true,
            label: format!("Alarm {id}"),
            hour: 7,
            minute: 30,
            mode,
            year: 2026,
            month: 9,
            day: 23,
            repeat_days: 1 << 2,
            sound: 0,
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn all_simultaneous_one_shots_are_queued_and_disabled_even_while_busy() {
        let now = Instant::now();
        for phase in [
            AlarmPhase::Idle,
            AlarmPhase::Armed {
                deadline: now + Duration::from_secs(300),
                duration: Duration::from_secs(300),
            },
            AlarmPhase::Ringing { started_at: now },
        ] {
            let mut settings = NeraSettings::default();
            settings.store_alarm_configs(&[scheduled_alarm(1, 0), scheduled_alarm(2, 0)]);
            let mut runtime = CustomAlarmRuntime::default();
            let mut state = AlarmState { phase, ..default() };
            assert!(collect_due_alarms(
                &mut settings,
                &mut state,
                &mut runtime,
                scheduled_time(),
                now
            ));
            assert_eq!(state.pending.len(), 2);
            assert!(settings.alarm_configs().iter().all(|alarm| !alarm.enabled));
            // A settings edit/re-enable in the same minute must not duplicate a
            // reminder already queued behind another alarm.
            settings.store_alarm_configs(&[scheduled_alarm(1, 0), scheduled_alarm(2, 0)]);
            assert!(!collect_due_alarms(
                &mut settings,
                &mut state,
                &mut runtime,
                scheduled_time(),
                now
            ));
            assert_eq!(state.pending.len(), 2);
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn weekly_alarms_queue_once_per_occurrence_while_another_alarm_rings() {
        let now = Instant::now();
        let mut settings = NeraSettings::default();
        settings.store_alarm_configs(&[scheduled_alarm(1, 1), scheduled_alarm(2, 1)]);
        let mut runtime = CustomAlarmRuntime::default();
        let mut state = AlarmState {
            phase: AlarmPhase::Ringing { started_at: now },
            ..default()
        };
        for _ in 0..2 {
            assert!(!collect_due_alarms(
                &mut settings,
                &mut state,
                &mut runtime,
                scheduled_time(),
                now
            ));
        }
        assert_eq!(state.pending.len(), 2);
        assert!(settings.alarm_configs().iter().all(|alarm| alarm.enabled));
        assert!(!collect_due_alarms(
            &mut settings,
            &mut state,
            &mut runtime,
            LocalDateTime {
                day: 30,
                ..scheduled_time()
            },
            now
        ));
        assert_eq!(state.pending.len(), 4);
    }

    #[test]
    fn scheduled_reminders_preserve_a_future_quick_or_snooze_deadline_and_label() {
        let now = Instant::now();
        let later = now + Duration::from_secs(300);
        for label in ["Quick Alarm", "Snoozed meeting"] {
            let mut display = AlarmDisplayInfo {
                label: label.into(),
                sound: 4,
                ..default()
            };
            let mut state = AlarmState {
                phase: AlarmPhase::Armed {
                    deadline: later,
                    duration: Duration::from_secs(300),
                },
                pending: vec![pending_alarm("First", now), pending_alarm("Second", now)],
            };
            assert_eq!(
                state.advance_deadline(now, &mut display),
                Some(Duration::ZERO)
            );
            assert_eq!(display.label, "First");
            assert_eq!(state.advance_deadline(later, &mut display), None);
            assert_eq!(display.label, "First");
            state.phase = AlarmPhase::Idle;
            assert_eq!(
                state.advance_deadline(now, &mut display),
                Some(Duration::ZERO)
            );
            assert_eq!(display.label, "Second");
            state.phase = AlarmPhase::Idle;
            assert_eq!(state.advance_deadline(now, &mut display), None);
            assert_eq!(display.label, label);
            assert_eq!(
                display.sound, 4,
                "a queued quick/snoozed alarm must retain its sound"
            );
            assert!(matches!(state.phase, AlarmPhase::Armed { deadline, .. } if deadline == later));
            assert_eq!(
                state.advance_deadline(later, &mut display),
                Some(Duration::from_secs(300))
            );
            assert!(state.pending.is_empty());
        }
    }

    #[test]
    fn a_due_quick_alarm_and_same_minute_reminders_all_get_delivered() {
        let now = Instant::now();
        let mut display = AlarmDisplayInfo {
            label: "Quick Alarm".into(),
            ..default()
        };
        let mut state = AlarmState {
            phase: AlarmPhase::Armed {
                deadline: now,
                duration: Duration::from_secs(300),
            },
            pending: vec![pending_alarm("One", now), pending_alarm("Two", now)],
        };
        for label in ["Quick Alarm", "One", "Two"] {
            assert!(state.advance_deadline(now, &mut display).is_some());
            assert_eq!(display.label, label);
            state.phase = AlarmPhase::Idle;
        }
        assert_eq!(state.advance_deadline(now, &mut display), None);
    }

    fn alarm_runtime_app() -> App {
        let mut app = App::new();
        app.init_resource::<AlarmState>()
            .init_resource::<sounds::AlarmSoundLibrary>()
            .init_resource::<Assets<AudioSource>>()
            .init_resource::<AlarmDisplayInfo>()
            .init_resource::<PresentationState>()
            .insert_resource(AlarmAssets {
                sounds: std::array::from_fn(|_| Handle::default()),
            })
            .add_message::<AlarmAnimationCommand>()
            .add_message::<AlarmControlCommand>()
            .add_message::<PresentationCommand>()
            .add_message::<PresentationExited>()
            .add_systems(Update, consume_alarm_control_commands)
            .add_systems(
                PostUpdate,
                (
                    cleanup_after_presentation_exit,
                    abort_ringing_if_attention_never_started,
                    tick_alarm_deadline,
                )
                    .chain(),
            );
        #[cfg(target_os = "windows")]
        app.init_resource::<crate::alarm_dismiss::AlarmDismissRuntime>();
        app
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn queued_alarm_waits_for_runtime_reset_after_native_window_is_gone() {
        use crate::alarm_dismiss::{AlarmDismissRuntime, AlarmDismissWindow};

        let mut app = alarm_runtime_app();
        app.world_mut().resource_mut::<AlarmState>().phase = AlarmPhase::Ringing {
            started_at: Instant::now(),
        };
        app.world_mut()
            .resource_mut::<AlarmState>()
            .pending
            .push(pending_alarm("Next", Instant::now()));
        let panel = app.world_mut().spawn(AlarmDismissWindow).id();
        app.insert_resource(AlarmDismissRuntime::with_window_for_test(panel));
        // Bevy's native-close cleanup has run, but the dismiss system ran
        // before presentation exit this frame and still owns the old window.
        app.world_mut().despawn(panel);
        app.world_mut().write_message(PresentationExited {
            kind: AttentionKind::Alarm,
        });
        app.update();
        assert!(!app.world().resource::<AlarmState>().is_ringing());
        assert_eq!(app.world().resource::<AlarmState>().pending.len(), 1);
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<PresentationCommand>>()
                .drain()
                .count(),
            0
        );

        // Only the completed runtime reset makes the next PresentationEntered
        // safe for spawn_dismiss_panel to consume.
        app.insert_resource(AlarmDismissRuntime::default());
        app.update();
        assert!(app.world().resource::<AlarmState>().is_ringing());
        assert_eq!(app.world().resource::<AlarmDisplayInfo>().label, "Next");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn queued_alarm_waits_for_old_panel_and_audio_teardown() {
        let mut app = alarm_runtime_app();
        app.world_mut().resource_mut::<AlarmState>().phase = AlarmPhase::Ringing {
            started_at: Instant::now(),
        };
        app.world_mut()
            .resource_mut::<AlarmState>()
            .pending
            .push(pending_alarm("Next", Instant::now()));
        let audio = app.world_mut().spawn(AlarmAudioPlayback).id();
        let panel = app
            .world_mut()
            .spawn(crate::alarm_dismiss::AlarmDismissWindow)
            .id();
        app.world_mut().write_message(PresentationExited {
            kind: AttentionKind::Alarm,
        });
        app.update();
        assert!(!app.world().resource::<AlarmState>().is_ringing());
        assert_eq!(app.world().resource::<AlarmState>().pending.len(), 1);
        assert!(app.world().get_entity(audio).is_err());
        app.world_mut().despawn(panel);
        app.update();
        assert!(app.world().resource::<AlarmState>().is_ringing());
        assert_eq!(app.world().resource::<AlarmDisplayInfo>().label, "Next");
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<AlarmAudioPlayback>>()
                .iter(app.world())
                .count(),
            1
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn late_presentation_exit_cannot_stop_the_next_queued_ring() {
        fn late_exit(
            mut sent: Local<bool>,
            mut exited: MessageWriter<PresentationExited>,
            panels: Query<Entity, With<crate::alarm_dismiss::AlarmDismissWindow>>,
            mut commands: Commands,
        ) {
            if *sent {
                return;
            }
            *sent = true;
            exited.write(PresentationExited {
                kind: AttentionKind::Alarm,
            });
            for panel in &panels {
                commands.entity(panel).despawn();
            }
        }
        let mut app = alarm_runtime_app();
        app.add_systems(Update, late_exit.after(consume_alarm_control_commands));
        app.world_mut().resource_mut::<AlarmState>().phase = AlarmPhase::Ringing {
            started_at: Instant::now(),
        };
        app.world_mut()
            .resource_mut::<AlarmState>()
            .pending
            .push(pending_alarm("Due reminder", Instant::now()));
        app.world_mut()
            .spawn(crate::alarm_dismiss::AlarmDismissWindow);
        app.world_mut()
            .write_message(AlarmControlCommand::Snooze(Duration::ZERO));
        app.update();
        assert_eq!(
            app.world().resource::<AlarmDisplayInfo>().label,
            "Due reminder"
        );
        assert!(app.world().resource::<AlarmState>().is_ringing());
        let audio = app
            .world_mut()
            .query_filtered::<Entity, With<AlarmAudioPlayback>>()
            .single(app.world())
            .unwrap();
        // No old PresentationExited may remain for the following frame's
        // cleanup to stop the new alarm or consume its pending snooze.
        app.update();
        assert!(app.world().resource::<AlarmState>().is_ringing());
        assert!(app.world().get_entity(audio).is_ok());
        assert_eq!(app.world().resource::<AlarmState>().pending.len(), 1);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn zero_snooze_survives_exit_without_retriggering_before_panel_closes() {
        let mut app = alarm_runtime_app();
        app.world_mut().resource_mut::<AlarmState>().phase = AlarmPhase::Ringing {
            started_at: Instant::now(),
        };
        let original_quick_deadline = Instant::now() + Duration::from_secs(600);
        app.world_mut()
            .resource_mut::<AlarmState>()
            .pending
            .push(pending_alarm("Quick Alarm", original_quick_deadline));
        let audio = app.world_mut().spawn(AlarmAudioPlayback).id();
        let panel = app
            .world_mut()
            .spawn(crate::alarm_dismiss::AlarmDismissWindow)
            .id();
        app.world_mut()
            .write_message(AlarmControlCommand::Snooze(Duration::ZERO));
        app.world_mut().write_message(PresentationExited {
            kind: AttentionKind::Alarm,
        });
        app.update();
        assert!(matches!(
            app.world().resource::<AlarmState>().phase,
            AlarmPhase::Armed { .. }
        ));
        assert!(app.world().get_entity(audio).is_err());
        app.world_mut().despawn(panel);
        app.update();
        let state = app.world().resource::<AlarmState>();
        assert!(state.is_ringing());
        assert_eq!(state.pending[0].deadline, original_quick_deadline);
    }

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
        let duration =
            preset_duration(AlarmMenuChoice::TestNow).expect("Test Now is a timer preset");

        assert_eq!(duration, Duration::ZERO);

        let deadline = Instant::now() + duration;
        assert!(Instant::now() >= deadline);
    }

    #[test]
    fn default_snooze_is_five_minutes() {
        assert_eq!(DEFAULT_SNOOZE_DURATION, Duration::from_secs(300),);
    }

    #[test]
    fn cancel_is_not_a_timer_preset() {
        assert_eq!(preset_duration(AlarmMenuChoice::Cancel), None,);
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
            ..default()
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
            sound: 0,
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
        assert!(!custom_alarm_matches(
            &alarm,
            LocalDateTime { day: 24, ..exact }
        ));
        assert!(!custom_alarm_matches(
            &alarm,
            LocalDateTime {
                minute: 31,
                ..exact
            }
        ));
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
            sound: 0,
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
        assert!(custom_alarm_matches(
            &alarm,
            LocalDateTime {
                day: 25,
                day_of_week: 5,
                ..monday
            }
        ));
        assert!(!custom_alarm_matches(
            &alarm,
            LocalDateTime {
                day: 27,
                day_of_week: 0,
                ..monday
            }
        ));
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
