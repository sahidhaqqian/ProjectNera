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

pub(crate) struct AlarmPlugin;

impl Plugin for AlarmPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AlarmState>()
            .init_resource::<AlarmVrmaBootstrap>()
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

fn tick_alarm_deadline(
    mut commands: Commands,
    assets: Res<AlarmAssets>,
    mut state: ResMut<AlarmState>,
    mut presentation: MessageWriter<PresentationCommand>,
    mut animation: MessageWriter<AlarmAnimationCommand>,
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
