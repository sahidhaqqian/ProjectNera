//! Bundled sound IDs are stable because settings and alarm records persist them.
use bevy::{
    audio::{AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings},
    prelude::*,
};
mod library;
pub(crate) use library::{AlarmSoundLibrary, clear_deleted_references};
pub(crate) const DEFAULT_SOUND: u16 = 0;
pub(crate) const RANDOM_SOUND: u16 = u16::MAX;
pub(crate) const STOP_PREVIEW: u16 = u16::MAX - 1;

pub(crate) const SOUNDS: [(&str, &str); 5] = [
    ("Classic", "sound/alarm.mp3"),
    ("Gentle", "sound/gentle.ogg"),
    ("Chime", "sound/chime.ogg"),
    ("Digital", "sound/digital.wav"),
    ("Siren", "sound/siren.wav"),
];

pub(crate) fn name(id: u16) -> &'static str {
    if id == RANDOM_SOUND {
        return "Random";
    }
    SOUNDS.get(id as usize).unwrap_or(&SOUNDS[0]).0
}
pub(crate) fn path(id: u16) -> &'static str {
    SOUNDS.get(id as usize).unwrap_or(&SOUNDS[0]).1
}

#[derive(Message)]
pub(crate) struct PreviewAlarmSound(pub(crate) u16);
#[derive(Component)]
pub(crate) struct AlarmSoundPreview;

pub(super) fn preview_sound(
    mut requests: MessageReader<PreviewAlarmSound>,
    assets: Res<super::AlarmAssets>,
    mut library: ResMut<AlarmSoundLibrary>,
    mut audio: ResMut<Assets<AudioSource>>,
    mut commands: Commands,
    previews: Query<(Entity, Option<&AudioSink>), With<AlarmSoundPreview>>,
) {
    let Some(request) = requests.read().last() else {
        return;
    };
    for (entity, sink) in &previews {
        if let Some(sink) = sink {
            sink.stop();
        }
        commands.entity(entity).despawn();
    }
    if request.0 == STOP_PREVIEW {
        return;
    }
    let sound = library.resolve(request.0, &assets.sounds, &mut audio);
    commands.spawn((
        AlarmSoundPreview,
        AudioPlayer::new(sound),
        PlaybackSettings::DESPAWN,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_preview_only_releases_editor_players_while_a_real_alarm_keeps_its_source() {
        let mut app = App::new();
        app.init_resource::<Assets<AudioSource>>()
            .init_resource::<AlarmSoundLibrary>()
            .add_message::<PreviewAlarmSound>()
            .add_systems(Update, preview_sound);
        let sounds = std::array::from_fn(|_| {
            app.world_mut()
                .resource_mut::<Assets<AudioSource>>()
                .add(AudioSource {
                    bytes: vec![0].into(),
                })
        });
        let ringing_source = sounds[0].clone();
        app.insert_resource(super::super::AlarmAssets { sounds });
        let ringing = app
            .world_mut()
            .spawn((
                super::super::AlarmAudioPlayback,
                AudioPlayer::new(ringing_source.clone()),
            ))
            .id();
        for id in [1, 2, 3] {
            app.world_mut().write_message(PreviewAlarmSound(id));
            app.update();
            let world = app.world_mut();
            assert_eq!(
                world
                    .query_filtered::<Entity, With<AlarmSoundPreview>>()
                    .iter(world)
                    .count(),
                1
            );
        }
        app.world_mut()
            .write_message(PreviewAlarmSound(STOP_PREVIEW));
        app.update();
        let world = app.world_mut();
        assert_eq!(
            world
                .query_filtered::<Entity, With<AlarmSoundPreview>>()
                .iter(world)
                .count(),
            0
        );
        assert_eq!(
            world.get::<AudioPlayer>(ringing).unwrap().0.id(),
            ringing_source.id()
        );
    }

    #[test]
    fn repeated_previews_reuse_audio_assets_and_keep_only_the_latest_player() {
        let mut app = App::new();
        app.init_resource::<Assets<bevy::audio::AudioSource>>()
            .init_resource::<AlarmSoundLibrary>()
            .add_message::<PreviewAlarmSound>()
            .add_systems(Update, preview_sound);
        let sounds = std::array::from_fn(|_| {
            app.world_mut()
                .resource_mut::<Assets<bevy::audio::AudioSource>>()
                .add(bevy::audio::AudioSource {
                    bytes: vec![0u8].into(),
                })
        });
        let default_sound = sounds[0].id();
        app.insert_resource(super::super::AlarmAssets { sounds });
        for _ in 0..6 {
            for id in 0..100 {
                app.world_mut().write_message(PreviewAlarmSound(id));
            }
            app.world_mut().write_message(PreviewAlarmSound(1000));
            app.update();
            let world = app.world_mut();
            let player = world
                .query_filtered::<&AudioPlayer, With<AlarmSoundPreview>>()
                .single(world)
                .unwrap();
            assert_eq!(player.0.id(), default_sound);
            assert_eq!(
                world.resource::<Assets<bevy::audio::AudioSource>>().len(),
                SOUNDS.len()
            );
        }
    }

    #[test]
    fn every_bundled_alarm_sound_decodes_to_non_silent_audio() {
        use bevy::audio::{AudioSource, Decodable};
        for (_, path) in SOUNDS {
            let bytes = std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets")
                    .join(path),
            )
            .unwrap();
            let source = AudioSource {
                bytes: bytes.into(),
            };
            assert!(
                source
                    .decoder()
                    .take(100_000)
                    .any(|sample| sample.abs() > 0.0001),
                "silent or invalid sound: {path}"
            );
        }
    }
}
