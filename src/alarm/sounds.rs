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
pub(super) struct AlarmSoundPreview;

pub(super) fn preview_sound(
    mut requests: MessageReader<PreviewAlarmSound>,
    assets: Res<AssetServer>,
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
        AudioPlayer::new(assets.load(path(request.0))),
        PlaybackSettings::DESPAWN,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
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
