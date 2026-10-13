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

pub(crate) const SOUNDS: [(&str, &str); 10] = [
    ("Classic", "sound/classic.wav"),
    ("Gentle", "sound/gentle.ogg"),
    ("Chime", "sound/chime.ogg"),
    ("Clock Beeps", "sound/clock_beeps.wav"),
    ("Soft Bell", "sound/soft_bell.ogg"),
    ("Wind Chimes", "sound/wind_chimes.ogg"),
    ("Deep Bell", "sound/deep_bell.ogg"),
    ("Steel Melody", "sound/steel_melody.ogg"),
    ("Pizzicato", "sound/pizzicato.ogg"),
    ("8-bit Tune", "sound/8bit_tune.ogg"),
];

// Imports historically start at 5. New built-ins use the reserved high end so
// expanding the choices never changes a persisted imported sound's identity.
pub(crate) const FIRST_EXTRA_BUNDLED_SOUND_ID: u16 = STOP_PREVIEW - 5;
pub(crate) const BUNDLED_SOUND_IDS: [u16; SOUNDS.len()] = [
    0,
    1,
    2,
    3,
    4,
    FIRST_EXTRA_BUNDLED_SOUND_ID,
    FIRST_EXTRA_BUNDLED_SOUND_ID + 1,
    FIRST_EXTRA_BUNDLED_SOUND_ID + 2,
    FIRST_EXTRA_BUNDLED_SOUND_ID + 3,
    FIRST_EXTRA_BUNDLED_SOUND_ID + 4,
];

pub(crate) fn bundled_index(id: u16) -> Option<usize> {
    BUNDLED_SOUND_IDS.iter().position(|sound| *sound == id)
}

pub(crate) fn name(id: u16) -> &'static str {
    if id == RANDOM_SOUND {
        return "Random";
    }
    bundled_index(id).map_or(SOUNDS[0].0, |index| SOUNDS[index].0)
}
pub(crate) fn path(id: u16) -> &'static str {
    bundled_index(id).map_or(SOUNDS[0].1, |index| SOUNDS[index].1)
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
    fn ten_alarm_choices_keep_original_ids_and_remove_warning_sirens() {
        assert_eq!(SOUNDS.len(), 10);
        assert_eq!(&BUNDLED_SOUND_IDS[..5], &[0, 1, 2, 3, 4]);
        let unique: std::collections::HashSet<_> = BUNDLED_SOUND_IDS.into_iter().collect();
        assert_eq!(unique.len(), SOUNDS.len());
        for (index, id) in BUNDLED_SOUND_IDS.into_iter().enumerate() {
            assert_eq!(bundled_index(id), Some(index));
            assert_eq!(name(id), SOUNDS[index].0);
            assert_eq!(path(id), SOUNDS[index].1);
            assert_ne!(id, RANDOM_SOUND);
            assert_ne!(id, STOP_PREVIEW);
        }
        assert_eq!(name(3), "Clock Beeps");
        assert_eq!(name(4), "Soft Bell");
        for removed in ["sound/digital.wav", "sound/siren.wav"] {
            assert!(!crate::nera_asset_file(removed).exists());
        }
    }

    /// Exercises the existing editor preview path on real Windows audio without
    /// opening windows or touching the user's mouse/keyboard/settings.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires Windows audio output; plays ten short quiet previews"]
    fn native_bundled_alarm_previews_use_windows_output_and_stop_cleanly() {
        use bevy::{
            asset::AssetPlugin,
            audio::{AudioPlugin, GlobalVolume, Volume},
        };
        use std::time::{Duration, Instant};

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                file_path: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets")
                    .to_string_lossy()
                    .into_owned(),
                ..default()
            },
            AudioPlugin::default(),
        ))
        .init_resource::<AlarmSoundLibrary>()
        .add_message::<PreviewAlarmSound>()
        .add_systems(Update, preview_sound);
        let sounds =
            BUNDLED_SOUND_IDS.map(|id| app.world().resource::<AssetServer>().load(path(id)));
        app.insert_resource(super::super::AlarmAssets { sounds });
        app.finish();
        app.cleanup();
        app.world_mut().resource_mut::<GlobalVolume>().volume = Volume::Linear(0.25);
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            app.update();
            let assets = app.world().resource::<super::super::AlarmAssets>();
            let audio = app.world().resource::<Assets<AudioSource>>();
            if assets.sounds.iter().all(|sound| audio.contains(sound.id())) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "bundled alarm sounds failed to load"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        for id in BUNDLED_SOUND_IDS {
            app.world_mut().write_message(PreviewAlarmSound(id));
            app.update();
            let world = app.world_mut();
            let sink = world
                .query_filtered::<&AudioSink, With<AlarmSoundPreview>>()
                .single(world)
                .expect("preview did not create a real Windows AudioSink");
            assert!(!sink.empty());
            assert!((sink.volume().to_linear() - 0.25).abs() < 0.0001);
            std::thread::sleep(Duration::from_millis(200));
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
            println!(
                "Alarm native PASSED: {} loaded, real output, preview stopped cleanly",
                name(id)
            );
        }
    }

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
