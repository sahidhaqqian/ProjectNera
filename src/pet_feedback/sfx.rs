//! Optional semantic cues share Bevy Audio and existing master volume/mute.
use bevy::{
    audio::{AudioPlayer, AudioSource, PlaybackSettings},
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
}
impl PetSfxKind {
    const ALL: [Self; 9] = [
        Self::HeadPat,
        Self::Happy,
        Self::Embarrassed,
        Self::Angry,
        Self::Landing,
        Self::CloudAppear,
        Self::CloudDisappear,
        Self::Sleep,
        Self::Wake,
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
        }
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
    sounds: [Option<Handle<AudioSource>>; 9],
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
    played_at: [Option<Duration>; 9],
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
        if settings.audio_muted || !presentation.is_desktop_pet() {
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
                PlaybackSettings::DESPAWN,
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
        if player.remaining <= 0.0 || !presentation.is_desktop_pet() {
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
