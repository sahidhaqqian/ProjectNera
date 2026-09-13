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
    },
};

const ALARM_SOUND_PATH: &str = "sound/alarm.mp3";
const ALARM_JUMP_VRMA_PATH: &str =
    "animations/animasi_basic/jump_happy.vrma";
const ALARM_WAVE_VRMA_PATH: &str =
    "animations/animasi_basic/161_Waving.vrma";

#[derive(Clone, Copy, Debug)]
enum AlarmPhase {
    Idle,

    Armed {
        deadline: Instant,
        duration: Duration,
    },

    Ringing,
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
        matches!(self.phase, AlarmPhase::Ringing)
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
            .add_systems(
                Startup,
                load_alarm_audio,
            )
            .add_systems(
                Update,
                (
                    ensure_alarm_vrmas_under_nera,
                    consume_alarm_menu_commands,
                    debug_alarm_now,
                    tick_alarm_deadline,
                    cleanup_after_presentation_exit,
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
        AlarmMenuChoice::ThirtyMinutes => {
            Some(Duration::from_secs(30 * 60))
        }

        // Phase 5.6.2 will attach a proper input panel.
        AlarmMenuChoice::Custom
        | AlarmMenuChoice::Cancel => None,
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

                    AlarmPhase::Ringing => {
                        // Cleanup happens from PresentationExited so all alarm
                        // exit paths have one single source of truth.
                        presentation.write(
                            PresentationCommand::ExitAttention,
                        );
                    }
                }
            }

            AlarmMenuChoice::Custom => {
                warn!(
                    "Custom alarm duration UI is not wired yet; \
                     use 5 / 10 / 30 min in Phase 5.6.1"
                );
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

/// Development-only shortcut for testing without waiting five minutes.
///
/// Ctrl + Alt + A -> ring immediately.
///
/// Remove this once the Alarm panel has a dedicated "Test Alarm" action.
fn debug_alarm_now(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<AlarmState>,
) {
    let ctrl = keyboard.pressed(KeyCode::ControlLeft)
        || keyboard.pressed(KeyCode::ControlRight);
    let alt = keyboard.pressed(KeyCode::AltLeft)
        || keyboard.pressed(KeyCode::AltRight);

    if ctrl && alt && keyboard.just_pressed(KeyCode::KeyA) {
        if state.is_ringing() {
            warn!(
                "DEBUG alarm trigger ignored: alarm is already ringing"
            );
            return;
        }

        state.phase = AlarmPhase::Armed {
            deadline: Instant::now(),
            duration: Duration::ZERO,
        };

        info!("DEBUG: Nera alarm deadline forced to now");
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

    state.phase = AlarmPhase::Ringing;

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

        let AlarmPhase::Ringing = state.phase else {
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
            preset_duration(AlarmMenuChoice::ThirtyMinutes),
            Some(Duration::from_secs(1800)),
        );
    }

    #[test]
    fn custom_and_cancel_are_not_timer_presets() {
        assert_eq!(
            preset_duration(AlarmMenuChoice::Custom),
            None,
        );
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
            phase: AlarmPhase::Ringing,
        };

        assert!(state.is_ringing());
        assert!(state.remaining().is_none());
    }

}
