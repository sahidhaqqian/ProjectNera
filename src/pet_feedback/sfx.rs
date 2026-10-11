//! Optional semantic cues share Bevy Audio and existing master volume/mute.
use bevy::{
    audio::{AudioPlayer, AudioSource, PlaybackSettings, Volume},
    prelude::*,
};
use std::{path::Path, time::Duration};

const MAX_PLAYERS: usize = 4;
const MAX_PLAY_SECONDS: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(crate) enum PetSfxKind {
    HeadPat,
    Happy,
    Embarrassed,
    Angry,
    Landing,
    CloudAppear,
    CloudDisappear,
    Sleep,
    Wake,
    Greeting,
}
impl PetSfxKind {
    const ALL: [Self; 10] = [
        Self::HeadPat,
        Self::Happy,
        Self::Embarrassed,
        Self::Angry,
        Self::Landing,
        Self::CloudAppear,
        Self::CloudDisappear,
        Self::Sleep,
        Self::Wake,
        Self::Greeting,
    ];
    fn path(self) -> &'static str {
        match self {
            Self::HeadPat => "sound/pet/headpat.ogg",
            Self::Happy => "sound/pet/happy.ogg",
            Self::Embarrassed => "sound/pet/embarrassed.ogg",
            Self::Angry => "sound/pet/angry.ogg",
            Self::Landing => "sound/pet/landing.ogg",
            Self::CloudAppear => "sound/pet/cloud_appear.ogg",
            Self::CloudDisappear => "sound/pet/cloud_disappear.ogg",
            Self::Sleep => "sound/pet/sleep.ogg",
            Self::Wake => "sound/pet/wake.ogg",
            Self::Greeting => "sound/pet/greeting.ogg",
        }
    }
    /// Pet cues sit below the alarm/music. Settings owns the master multiplier.
    fn gain(self) -> f32 {
        match self {
            Self::Greeting => 0.38,
            Self::Happy => 0.30,
            Self::HeadPat | Self::Embarrassed => 0.24,
            Self::Angry => 0.22,
            Self::Landing => 0.28,
            Self::CloudAppear | Self::CloudDisappear => 0.24,
            Self::Sleep => 0.18,
            Self::Wake => 0.24,
        }
    }
    fn allowed_in(self, presentation: &crate::presentation::PresentationState) -> bool {
        presentation.is_desktop_pet()
            || (presentation.is_screensaver() && matches!(self, Self::HeadPat | Self::Greeting))
    }
    fn cooldown(self) -> Duration {
        Duration::from_millis(match self {
            Self::HeadPat => 1000,
            Self::Landing => 350,
            Self::CloudAppear | Self::CloudDisappear => 300,
            _ => 1500,
        })
    }
}
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct PetSfx(pub(crate) PetSfxKind);

#[derive(Resource, Default)]
pub(super) struct SfxCatalog {
    sounds: [Option<Handle<AudioSource>>; PetSfxKind::ALL.len()],
}
impl SfxCatalog {
    fn discover(root: &Path, mut load: impl FnMut(&'static str) -> Handle<AudioSource>) -> Self {
        Self {
            sounds: std::array::from_fn(|index| {
                let path = PetSfxKind::ALL[index].path();
                root.join(path).is_file().then(|| load(path))
            }),
        }
    }
    fn resolve(&self, kind: PetSfxKind) -> Option<&Handle<AudioSource>> {
        self.sounds[kind as usize].as_ref()
    }
}
#[derive(Resource, Default)]
pub(super) struct SfxCooldown {
    played_at: [Option<Duration>; PetSfxKind::ALL.len()],
}
impl SfxCooldown {
    fn admit(&mut self, kind: PetSfxKind, now: Duration, available: bool, active: usize) -> bool {
        let last = &mut self.played_at[kind as usize];
        if !available
            || active >= MAX_PLAYERS
            || last.is_some_and(|last| now.saturating_sub(last) < kind.cooldown())
        {
            return false;
        }
        *last = Some(now);
        true
    }
}
#[derive(Component)]
pub(super) struct PetSfxPlayer {
    remaining: f32,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<SfxCatalog>()
        .init_resource::<SfxCooldown>()
        .add_message::<PetSfx>()
        .add_systems(Startup, load_optional_sfx)
        .add_systems(Update, (expire_players, play_sfx).chain());
}
fn load_optional_sfx(assets: Res<AssetServer>, mut catalog: ResMut<SfxCatalog>) {
    *catalog = SfxCatalog::discover(&crate::nera_asset_directory(), |path| assets.load(path));
    info!(
        "PetSfx: discovered {}/{} bundled cues",
        catalog.sounds.iter().flatten().count(),
        PetSfxKind::ALL.len()
    );
}
pub(super) fn play_sfx(
    mut requests: MessageReader<PetSfx>,
    time: Res<Time<Real>>,
    catalog: Res<SfxCatalog>,
    sources: Res<Assets<AudioSource>>,
    settings: Res<crate::settings::NeraSettings>,
    presentation: Res<crate::presentation::PresentationState>,
    mut cooldown: ResMut<SfxCooldown>,
    players: Query<(), With<PetSfxPlayer>>,
    mut commands: Commands,
) {
    let mut active = players.iter().count();
    for request in requests.read() {
        if settings.audio_muted || !request.0.allowed_in(&presentation) {
            continue;
        }
        let sound = catalog
            .resolve(request.0)
            .filter(|sound| sources.contains(sound.id()));
        if !cooldown.admit(request.0, time.elapsed(), sound.is_some(), active) {
            continue;
        }
        if let Some(sound) = sound {
            commands.spawn((
                PetSfxPlayer {
                    remaining: MAX_PLAY_SECONDS,
                },
                AudioPlayer::new(sound.clone()),
                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(request.0.gain())),
            ));
            active += 1;
        }
    }
}
fn expire_players(
    time: Res<Time<Real>>,
    presentation: Res<crate::presentation::PresentationState>,
    mut players: Query<(Entity, &mut PetSfxPlayer)>,
    mut commands: Commands,
) {
    for (entity, mut player) in &mut players {
        player.remaining -= time.delta_secs();
        if player.remaining <= 0.0
            || !(presentation.is_desktop_pet() || presentation.is_screensaver())
        {
            commands.entity(entity).despawn();
        }
    }
}
pub(super) fn clear(world: &mut World) {
    let players: Vec<_> = world
        .query_filtered::<Entity, With<PetSfxPlayer>>()
        .iter(world)
        .collect();
    for player in players {
        world.despawn(player);
    }
    if let Some(mut requests) = world.get_resource_mut::<Messages<PetSfx>>() {
        requests.clear();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source;

    #[test]
    fn every_bundled_cue_decodes_to_short_non_silent_audio_at_a_safe_mix_level() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let mut fingerprints = std::collections::HashSet::new();
        for kind in PetSfxKind::ALL {
            let bytes = std::fs::read(root.join(kind.path())).unwrap();
            assert!(
                fingerprints.insert(blake3::hash(&bytes)),
                "duplicate {kind:?}"
            );
            let decoder = rodio::Decoder::try_from(std::io::Cursor::new(bytes)).unwrap();
            let samples_per_second =
                decoder.sample_rate().get() as usize * decoder.channels().get() as usize;
            let samples: Vec<f32> = decoder.collect();
            assert!(!samples.is_empty(), "{kind:?}");
            assert!(samples.len() < samples_per_second, "{kind:?} is too long");
            let peak = samples.iter().copied().map(f32::abs).fold(0.0, f32::max);
            assert!(peak > 0.01 && peak * kind.gain() < 0.5, "{kind:?}");
            assert!(samples.iter().all(|sample| sample.is_finite()), "{kind:?}");
        }
    }

    #[test]
    fn mute_and_presentation_gate_cues_without_consuming_their_cooldown() {
        use crate::presentation::{AttentionKind, PresentationMode, PresentationState};
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .init_resource::<SfxCatalog>()
            .init_resource::<SfxCooldown>()
            .init_resource::<Assets<AudioSource>>()
            .init_resource::<crate::settings::NeraSettings>()
            .init_resource::<PresentationState>()
            .add_message::<PetSfx>()
            .add_systems(Update, play_sfx);
        let source = app
            .world_mut()
            .resource_mut::<Assets<AudioSource>>()
            .add(AudioSource {
                bytes: Vec::new().into(),
            });
        app.world_mut().resource_mut::<SfxCatalog>().sounds =
            std::array::from_fn(|_| Some(source.clone()));

        app.world_mut()
            .resource_mut::<crate::settings::NeraSettings>()
            .audio_muted = true;
        app.world_mut().write_message(PetSfx(PetSfxKind::Greeting));
        app.update();
        assert!(
            app.world()
                .resource::<SfxCooldown>()
                .played_at
                .iter()
                .all(Option::is_none)
        );
        app.world_mut()
            .resource_mut::<crate::settings::NeraSettings>()
            .audio_muted = false;
        app.insert_resource(PresentationState::for_test(PresentationMode::Attention(
            AttentionKind::Alarm,
        )));
        app.world_mut().write_message(PetSfx(PetSfxKind::Greeting));
        app.update();
        assert!(
            app.world()
                .resource::<SfxCooldown>()
                .played_at
                .iter()
                .all(Option::is_none)
        );

        app.insert_resource(PresentationState::for_test(PresentationMode::Attention(
            AttentionKind::Screensaver,
        )));
        for kind in PetSfxKind::ALL {
            app.world_mut().write_message(PetSfx(kind));
        }
        app.update();
        let volumes: Vec<f32> = app
            .world_mut()
            .query_filtered::<&PlaybackSettings, With<PetSfxPlayer>>()
            .iter(app.world())
            .map(|settings| settings.volume.to_linear())
            .collect();
        assert_eq!(volumes.len(), 2);
        assert!(volumes.contains(&PetSfxKind::Greeting.gain()));
        assert!(volumes.contains(&PetSfxKind::HeadPat.gain()));
        assert_eq!(
            app.world()
                .resource::<SfxCooldown>()
                .played_at
                .iter()
                .flatten()
                .count(),
            2
        );
    }

    /// Runs real Bevy Audio on the default Windows output, without a window,
    /// persistent settings, or mouse/keyboard control. Opt-in to keep CI quiet.
    #[cfg(target_os = "windows")]
    #[test]
    #[ignore = "requires a real Windows audio output; plays ten quiet pet cues"]
    fn native_bundled_pet_cues_create_audio_sinks_and_finish() {
        use bevy::{
            asset::AssetPlugin,
            audio::{AudioPlugin, AudioSink, AudioSinkPlayback, GlobalVolume},
        };
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin {
                file_path: Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets")
                    .to_string_lossy()
                    .into_owned(),
                ..default()
            },
            AudioPlugin::default(),
        ))
        .init_resource::<crate::settings::NeraSettings>()
        .init_resource::<crate::presentation::PresentationState>();
        install(&mut app);
        app.finish();
        app.cleanup();
        app.world_mut().resource_mut::<GlobalVolume>().volume = Volume::Linear(0.5);
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        loop {
            app.update();
            let catalog = app.world().resource::<SfxCatalog>();
            let sources = app.world().resource::<Assets<AudioSource>>();
            if PetSfxKind::ALL.into_iter().all(|kind| {
                catalog
                    .resolve(kind)
                    .is_some_and(|handle| sources.contains(handle.id()))
            }) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "bundled cues failed to load"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        for kind in PetSfxKind::ALL {
            app.world_mut().write_message(PetSfx(kind));
            app.update();
            let (entity, volume) = app
                .world_mut()
                .query_filtered::<(Entity, &AudioSink), With<PetSfxPlayer>>()
                .iter(app.world())
                .map(|(entity, sink)| (entity, sink.volume().to_linear()))
                .next()
                .expect("real output did not create an AudioSink");
            assert!((volume - kind.gain() * 0.5).abs() < 0.0001);
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while app.world().get_entity(entity).is_ok() {
                std::thread::sleep(Duration::from_millis(10));
                app.update();
                assert!(
                    std::time::Instant::now() < deadline,
                    "{kind:?} did not finish/despawn"
                );
            }
            println!(
                "PetSfx native PASSED: {kind:?} loaded, output sink gain={volume:.3}, completed/despawned"
            );
        }
    }

    #[test]
    fn real_dispatch_is_silent_when_missing_and_limits_available_voices() {
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .init_resource::<SfxCatalog>()
            .init_resource::<SfxCooldown>()
            .init_resource::<Assets<AudioSource>>()
            .init_resource::<crate::settings::NeraSettings>()
            .init_resource::<crate::presentation::PresentationState>()
            .add_message::<PetSfx>()
            .add_systems(Update, (expire_players, play_sfx).chain());
        for _ in 0..1000 {
            app.world_mut().write_message(PetSfx(PetSfxKind::HeadPat));
        }
        app.update();
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<PetSfxPlayer>>()
                .iter(app.world())
                .count(),
            0
        );
        let sound = app
            .world_mut()
            .resource_mut::<Assets<AudioSource>>()
            .add(AudioSource {
                bytes: Vec::new().into(),
            });
        app.world_mut().resource_mut::<SfxCatalog>().sounds =
            std::array::from_fn(|_| Some(sound.clone()));
        for _ in 0..1000 {
            for kind in PetSfxKind::ALL {
                app.world_mut().write_message(PetSfx(kind));
            }
        }
        app.update();
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<PetSfxPlayer>>()
                .iter(app.world())
                .count(),
            MAX_PLAYERS
        );
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_secs(6));
        app.update();
        assert_eq!(
            app.world_mut()
                .query_filtered::<Entity, With<PetSfxPlayer>>()
                .iter(app.world())
                .count(),
            0
        );
    }
    #[test]
    fn semantic_resolver_distinguishes_available_and_missing_assets() {
        let mut sources = Assets::<AudioSource>::default();
        let handle = sources.add(AudioSource {
            bytes: Vec::new().into(),
        });
        let mut catalog = SfxCatalog::default();
        catalog.sounds[PetSfxKind::Landing as usize] = Some(handle.clone());
        assert_eq!(catalog.resolve(PetSfxKind::Landing), Some(&handle));
        assert_eq!(catalog.resolve(PetSfxKind::Angry), None);
        let mut calls = 0;
        let missing = SfxCatalog::discover(Path::new("__nera_missing_optional_audio__"), |_| {
            calls += 1;
            handle.clone()
        });
        assert_eq!(calls, 0);
        assert!(
            PetSfxKind::ALL
                .into_iter()
                .all(|kind| missing.resolve(kind).is_none())
        );
    }
    #[test]
    fn cooldown_and_voice_budget_bound_repeated_events_without_growing_state() {
        let mut cooldown = SfxCooldown::default();
        let now = Duration::from_secs(1);
        assert!(!cooldown.admit(PetSfxKind::HeadPat, now, false, 0));
        assert!(!cooldown.admit(PetSfxKind::HeadPat, now, true, MAX_PLAYERS));
        assert!(cooldown.admit(PetSfxKind::HeadPat, now, true, 0));
        for _ in 0..10_000 {
            assert!(!cooldown.admit(PetSfxKind::HeadPat, now, true, 0));
        }
        assert!(cooldown.admit(PetSfxKind::Landing, now, true, 1));
        assert!(!cooldown.admit(
            PetSfxKind::HeadPat,
            now + Duration::from_millis(999),
            true,
            2
        ));
        assert!(cooldown.admit(PetSfxKind::HeadPat, now + Duration::from_secs(1), true, 0));
        assert_eq!(cooldown.played_at.iter().flatten().count(), 2);
    }
    #[test]
    fn stalled_or_failed_audio_player_still_expires() {
        let mut app = App::new();
        app.init_resource::<Time<Real>>()
            .init_resource::<crate::presentation::PresentationState>()
            .add_systems(Update, expire_players);
        let entity = app
            .world_mut()
            .spawn(PetSfxPlayer {
                remaining: MAX_PLAY_SECONDS,
            })
            .id();
        app.world_mut()
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_secs(6));
        app.update();
        assert!(app.world().get_entity(entity).is_err());
    }
}
