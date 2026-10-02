//! Bundled sound IDs are stable because settings and alarm records persist them.
use bevy::{
    audio::{AudioPlayer, PlaybackSettings},
    prelude::*,
};

pub(crate) const SOUNDS: [(&str, &str); 5] = [
    ("Classic", "sound/alarm.mp3"),
    ("Gentle", "sound/gentle.ogg"),
    ("Chime", "sound/chime.ogg"),
    ("Digital", "sound/digital.wav"),
    ("Siren", "sound/siren.wav"),
];

pub(crate) fn name(id: u8) -> &'static str {
    SOUNDS.get(id as usize).unwrap_or(&SOUNDS[0]).0
}
pub(crate) fn path(id: u8) -> &'static str {
    SOUNDS.get(id as usize).unwrap_or(&SOUNDS[0]).1
}

#[derive(Message)]
pub(crate) struct PreviewAlarmSound(pub(crate) u8);
#[derive(Component)]
pub(crate) struct AlarmSoundPreview;

pub(super) fn preview_sound(
    mut requests: MessageReader<PreviewAlarmSound>,
    assets: Res<super::AlarmAssets>,
    mut commands: Commands,
    previews: Query<Entity, With<AlarmSoundPreview>>,
) {
    let Some(request) = requests.read().last() else {
        return;
    };
    for entity in &previews {
        commands.entity(entity).despawn();
    }
    commands.spawn((
        AlarmSoundPreview,
        AudioPlayer::new(
            assets.sounds[if usize::from(request.0) < SOUNDS.len() {
                usize::from(request.0)
            } else {
                0
            }]
            .clone(),
        ),
        PlaybackSettings::DESPAWN,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_previews_reuse_audio_assets_and_keep_only_the_latest_player() {
        let mut app = App::new();
        app.init_resource::<Assets<bevy::audio::AudioSource>>()
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
            app.world_mut().write_message(PreviewAlarmSound(u8::MAX));
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
