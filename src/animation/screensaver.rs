//! Presentation choices request the existing animation owner and lazy rig loaders.
use super::playback::{ClipSlot, LazyNeraVrma, LazyNeraVrmaRequested, request_lazy_vrma};
use super::{NeraAnimationClips, NeraAnimationState, TemporaryAction, VisualState};
use crate::{
    avatar::NeraAvatar,
    pet_feedback::{PetFx, PetFxAnchor, PetFxKind},
    presentation::PresentationState,
    rng::NeraRng,
};
use bevy::prelude::*;
use bevy_vrm1::prelude::{Initialized, LoadedVrma, VrmaDuration};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ScreensaverClip {
    Waving,
    Breathing,
    Holding,
    Dwarf4,
    Briefcase2,
    Dwarf,
    Dwarf3,
    Offensive,
    Dwarf2,
}
impl ScreensaverClip {
    pub(crate) const ALL: [Self; 9] = [
        Self::Waving,
        Self::Breathing,
        Self::Holding,
        Self::Dwarf4,
        Self::Briefcase2,
        Self::Dwarf,
        Self::Dwarf3,
        Self::Offensive,
        Self::Dwarf2,
    ];
    const VARIATIONS: [Self; 7] = [
        Self::Holding,
        Self::Dwarf4,
        Self::Briefcase2,
        Self::Dwarf,
        Self::Dwarf3,
        Self::Offensive,
        Self::Dwarf2,
    ];
    pub(super) fn index(self) -> usize {
        self as usize
    }
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Waving => "animations/animasi_basic/Waving.vrma",
            Self::Breathing => "animations/animasi_basic/Breathing_Idle.vrma",
            Self::Holding => "animations/animasi_basic/Holding_Idle.vrma",
            Self::Dwarf4 => "animations/animasi_basic/Dwarf_Idle_4.vrma",
            Self::Briefcase2 => "animations/animasi_basic/Standing_WBriefcase_Idle_2.vrma",
            Self::Dwarf => "animations/animasi_basic/Dwarf_Idle.vrma",
            Self::Dwarf3 => "animations/animasi_basic/Dwarf_Idle_3.vrma",
            Self::Offensive => "animations/animasi_basic/Offensive_Idle.vrma",
            Self::Dwarf2 => "animations/animasi_basic/Dwarf_Idle_2.vrma",
        }
    }
}

pub(super) fn register(
    event: On<LoadedVrma>,
    loaders: Query<&LazyNeraVrma>,
    durations: Query<&VrmaDuration>,
    mut clips: ResMut<NeraAnimationClips>,
) {
    let Ok(LazyNeraVrma::Screensaver(kind)) = loaders.get(event.vrma) else {
        return;
    };
    let Ok(duration) = durations.get(event.vrma) else {
        return;
    };
    if duration.0.is_zero() {
        return;
    }
    clips.screensaver[kind.index()] = Some(ClipSlot {
        entity: event.vrma,
        duration: duration.0,
    });
    info!(
        "Screensaver VRMA ready: {} duration={:?}",
        kind.path(),
        duration.0
    );
}

#[derive(Resource, Default)]
pub(crate) struct ScreensaverRoutine {
    avatar: Option<Entity>,
    pending: Option<ScreensaverClip>,
    pending_since: Duration,
    next_variation: Duration,
    last_variation: Option<ScreensaverClip>,
    insect_until: Option<Duration>,
    headpat: bool,
    unavailable: [bool; 9],
    rng: NeraRng,
}
impl ScreensaverRoutine {
    fn delay(&mut self) -> Duration {
        Duration::from_secs_f32(self.rng.range_f32(30.0, 60.0))
    }
    fn choose(&mut self) -> Option<ScreensaverClip> {
        let choices: Vec<_> = ScreensaverClip::VARIATIONS
            .into_iter()
            .filter(|clip| !self.unavailable[clip.index()] && Some(*clip) != self.last_variation)
            .collect();
        if choices.is_empty() {
            return None;
        }
        Some(choices[self.rng.range_i32(0, choices.len() as i32 - 1) as usize])
    }
    fn queue(&mut self, clip: ScreensaverClip, now: Duration) {
        self.pending = Some(clip);
        self.pending_since = now;
        self.insect_until = None;
    }
}

pub(crate) fn update_screensaver_animation(
    presentation: Res<PresentationState>,
    time: Res<Time<Real>>,
    avatars: Query<Entity, (With<NeraAvatar>, With<Initialized>)>,
    clips: Res<NeraAnimationClips>,
    mut state: ResMut<NeraAnimationState>,
    mut routine: ResMut<ScreensaverRoutine>,
    assets: Res<AssetServer>,
    loaders: Query<(Entity, &LazyNeraVrma, Option<&LazyNeraVrmaRequested>)>,
    mut commands: Commands,
    mut fx: MessageWriter<PetFx>,
) {
    let now = time.elapsed();
    if !presentation.is_screensaver() {
        if routine.avatar.is_some() {
            state.cancel_screensaver_animation();
            state.finish_action(TemporaryAction::HeadStroke);
            *routine = ScreensaverRoutine::default();
        }
        return;
    }
    let Ok(avatar) = avatars.single() else { return };
    if routine.avatar != Some(avatar) {
        state.cancel_screensaver_animation();
        if routine.avatar.is_some() {
            state.finish_action(TemporaryAction::HeadStroke);
        }
        *routine = ScreensaverRoutine {
            avatar: Some(avatar),
            ..default()
        };
        routine.queue(ScreensaverClip::Waving, now);
    }
    if state.visual_state() == VisualState::HeadStroke {
        if !routine.headpat {
            state.cancel_screensaver_animation();
            routine.pending = None;
            routine.insect_until = None;
            routine.next_variation = now + routine.delay();
        }
        routine.headpat = true;
        return;
    }
    routine.headpat = false;
    if let VisualState::Screensaver(kind) = state.visual_state() {
        if kind != ScreensaverClip::Breathing {
            // Keep the full authored one-shot; prewarm the default while the
            // greeting/variation plays so its return doesn't wait for I/O.
            if clips.screensaver[ScreensaverClip::Breathing.index()].is_none() {
                request_lazy_vrma(
                    LazyNeraVrma::Screensaver(ScreensaverClip::Breathing),
                    &mut commands,
                    &assets,
                    &loaders,
                );
            }
            return;
        }
    }
    // Alarm/critical/manual reactions keep their existing priority and cannot
    // be restarted or cancelled by this low-priority presentation scheduler.
    if !matches!(
        state.visual_state(),
        VisualState::Idle | VisualState::Screensaver(_)
    ) {
        return;
    }
    if routine.pending.is_none() {
        if state.visual_state() == VisualState::Screensaver(ScreensaverClip::Breathing) {
            if now < routine.next_variation {
                return;
            }
            if let Some(clip) = routine.choose() {
                routine.queue(clip, now);
            } else {
                routine.next_variation = now + routine.delay();
                return;
            }
        } else {
            if routine.unavailable[ScreensaverClip::Breathing.index()] {
                return;
            }
            routine.queue(ScreensaverClip::Breathing, now);
        }
    }
    let Some(kind) = routine.pending else { return };
    let slot = clips.screensaver[kind.index()];
    if slot.is_none() {
        if now.saturating_sub(routine.pending_since) >= Duration::from_secs(20) {
            warn!(
                "Screensaver animation unavailable, skipping: {}",
                kind.path()
            );
            routine.unavailable[kind.index()] = true;
            routine.pending = None;
            routine.next_variation = now + routine.delay();
        } else {
            request_lazy_vrma(
                LazyNeraVrma::Screensaver(kind),
                &mut commands,
                &assets,
                &loaders,
            );
        }
        return;
    }
    let slot = slot.unwrap();
    if kind == ScreensaverClip::Dwarf2 {
        let until = if let Some(until) = routine.insect_until {
            until
        } else {
            fx.write(PetFx {
                kind: PetFxKind::Firefly,
                anchor: PetFxAnchor::Head,
            });
            let until = now + Duration::from_secs(2);
            routine.insect_until = Some(until);
            until
        };
        if now < until {
            return;
        }
    }
    state.cancel_screensaver_animation();
    let action = TemporaryAction::Screensaver(kind);
    if kind == ScreensaverClip::Breathing {
        state.start_persistent_action(action);
    } else {
        state.request_one_shot(action, slot.duration);
    }
    if kind != ScreensaverClip::Breathing {
        if kind != ScreensaverClip::Waving {
            routine.last_variation = Some(kind);
        }
        routine.next_variation = now + slot.duration + routine.delay();
    } else if routine.next_variation.is_zero() {
        routine.next_variation = now + routine.delay();
    }
    routine.pending = None;
    routine.insect_until = None;
    info!(
        "Screensaver playing {} next variation in {:.1}s",
        kind.path(),
        routine.next_variation.saturating_sub(now).as_secs_f32()
    );
}

#[cfg(any(debug_assertions, test))]
pub(crate) fn smoke_select(world: &mut World, clip: ScreensaverClip) {
    assert_eq!(
        world.resource::<NeraAnimationState>().visual_state(),
        VisualState::Screensaver(ScreensaverClip::Breathing)
    );
    let now = world.resource::<Time<Real>>().elapsed();
    world.resource_mut::<ScreensaverRoutine>().queue(clip, now);
}

#[cfg(debug_assertions)]
pub(crate) fn smoke_playing(world: &World, clip: ScreensaverClip) -> bool {
    let clips = world.resource::<NeraAnimationClips>();
    world.resource::<NeraAnimationState>().visual_state() == VisualState::Screensaver(clip)
        && clips.screensaver[clip.index()].is_some_and(|slot| {
            Some(slot.entity)
                == world
                    .resource::<super::NeraAnimationPlayback>()
                    .current_clip
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        interaction::{
            NeraClicked, NeraDragStarted, NeraHeadPatted, NeraPointerPosition, NeraReleased,
        },
        presentation::{AttentionKind, PresentationMode},
    };
    use bevy::{asset::AssetPlugin, time::TimeUpdateStrategy};

    fn fixture() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO))
            .init_resource::<NeraAnimationState>()
            .init_resource::<ScreensaverRoutine>()
            .init_resource::<crate::environment::DesktopEnvironment>()
            .insert_resource(PresentationState::for_test(PresentationMode::Attention(
                AttentionKind::Screensaver,
            )))
            .add_message::<PetFx>()
            .add_message::<NeraClicked>()
            .add_message::<NeraDragStarted>()
            .add_message::<NeraHeadPatted>()
            .add_message::<NeraReleased>()
            .add_systems(
                Update,
                (
                    super::super::sync_interaction_animation_actions,
                    super::super::tick_animation_actions,
                    update_screensaver_animation,
                )
                    .chain(),
            );
        app.world_mut()
            .resource_mut::<Time<Virtual>>()
            .set_max_delta(Duration::from_secs(120));
        app.world_mut().spawn((NeraAvatar, Initialized));
        let mut clips = NeraAnimationClips::default();
        for kind in ScreensaverClip::ALL {
            clips.screensaver[kind.index()] = Some(ClipSlot {
                entity: app.world_mut().spawn_empty().id(),
                duration: Duration::from_secs(2),
            });
        }
        clips.head_stroke = Some(ClipSlot {
            entity: app.world_mut().spawn_empty().id(),
            duration: Duration::from_secs(2),
        });
        app.insert_resource(clips);
        app.update();
        app
    }
    fn step(app: &mut App, seconds: f32) {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            seconds,
        )));
        app.update();
    }
    fn visual(app: &App) -> VisualState {
        app.world().resource::<NeraAnimationState>().visual_state()
    }
    fn pat(app: &mut App) {
        app.world_mut().write_message(NeraHeadPatted {
            pointer: NeraPointerPosition {
                screen: IVec2::ZERO,
                local: Vec2::ZERO,
            },
        });
    }

    #[test]
    fn greeting_plays_fully_once_then_breathing_loops_until_the_random_deadline() {
        let mut app = fixture();
        assert_eq!(
            visual(&app),
            VisualState::Screensaver(ScreensaverClip::Waving)
        );
        step(&mut app, 1.0);
        assert_eq!(
            visual(&app),
            VisualState::Screensaver(ScreensaverClip::Waving)
        );
        step(&mut app, 1.0);
        assert_eq!(
            visual(&app),
            VisualState::Screensaver(ScreensaverClip::Breathing)
        );
        let delay = app
            .world()
            .resource::<ScreensaverRoutine>()
            .next_variation
            .as_secs_f32()
            - 2.0;
        assert!((30.0..60.0).contains(&delay));
        step(&mut app, 29.0);
        assert_eq!(
            visual(&app),
            VisualState::Screensaver(ScreensaverClip::Breathing)
        );
        let state = app.world().resource::<NeraAnimationState>();
        let clips = app.world().resource::<NeraAnimationClips>();
        assert_eq!(
            super::super::playback::clip_for_state(state, clips).1,
            bevy::animation::RepeatAnimation::Forever
        );
    }
    #[test]
    fn all_variations_return_to_breathing_and_firefly_precedes_the_swat() {
        for kind in ScreensaverClip::VARIATIONS {
            let mut app = fixture();
            step(&mut app, 2.0);
            smoke_select(app.world_mut(), kind);
            step(&mut app, 0.0);
            if kind == ScreensaverClip::Dwarf2 {
                assert_eq!(
                    visual(&app),
                    VisualState::Screensaver(ScreensaverClip::Breathing)
                );
                let effects: Vec<_> = app
                    .world_mut()
                    .resource_mut::<Messages<PetFx>>()
                    .drain()
                    .collect();
                assert_eq!(effects.len(), 1);
                assert_eq!(effects[0].kind, PetFxKind::Firefly);
                step(&mut app, 1.9);
                assert_eq!(
                    visual(&app),
                    VisualState::Screensaver(ScreensaverClip::Breathing)
                );
                assert!(app.world().resource::<ScreensaverRoutine>().pending == Some(kind));
                step(&mut app, 0.11);
            }
            assert_eq!(visual(&app), VisualState::Screensaver(kind));
            step(&mut app, 2.0);
            assert_eq!(
                visual(&app),
                VisualState::Screensaver(ScreensaverClip::Breathing)
            );
        }
    }
    #[test]
    fn accepted_headpat_interrupts_the_wave_and_returns_to_breathing_without_exiting() {
        let mut app = fixture();
        pat(&mut app);
        step(&mut app, 0.0);
        assert_eq!(visual(&app), VisualState::HeadStroke);
        assert!(app.world().resource::<PresentationState>().is_screensaver());
        step(&mut app, 2.0);
        assert_eq!(
            visual(&app),
            VisualState::Screensaver(ScreensaverClip::Breathing)
        );
    }
    #[test]
    fn alarm_and_loading_reject_headpat_and_screensaver_cleanup_releases_only_its_actions() {
        let mut app = fixture();
        app.insert_resource(PresentationState::for_test(PresentationMode::Attention(
            AttentionKind::Alarm,
        )));
        pat(&mut app);
        step(&mut app, 0.0);
        assert_eq!(visual(&app), VisualState::Idle);
        app.insert_resource(PresentationState::for_test(PresentationMode::Attention(
            AttentionKind::Screensaver,
        )));
        app.insert_resource(crate::character::CharacterImportStatus {
            loading: true,
            ..default()
        });
        pat(&mut app);
        step(&mut app, 0.0);
        assert_ne!(visual(&app), VisualState::HeadStroke);
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .start_persistent_action(TemporaryAction::AlarmWave);
        app.insert_resource(PresentationState::for_test(PresentationMode::DesktopPet));
        step(&mut app, 0.0);
        assert_eq!(visual(&app), VisualState::AlarmWave);
    }
    #[test]
    fn replacement_discards_previous_rig_actions_and_greets_the_new_rig() {
        let mut app = fixture();
        step(&mut app, 2.0);
        pat(&mut app);
        step(&mut app, 0.0);
        let avatar = app
            .world_mut()
            .query_filtered::<Entity, With<NeraAvatar>>()
            .single(app.world())
            .unwrap();
        app.world_mut().despawn(avatar);
        app.world_mut().spawn((NeraAvatar, Initialized));
        step(&mut app, 0.0);
        assert_eq!(
            visual(&app),
            VisualState::Screensaver(ScreensaverClip::Waving)
        );
    }
    #[test]
    fn random_choices_and_delays_are_bounded_and_avoid_immediate_repeat() {
        let mut routine = ScreensaverRoutine {
            rng: NeraRng::seeded(42),
            ..default()
        };
        for _ in 0..1000 {
            let choice = routine.choose().unwrap();
            assert_ne!(Some(choice), routine.last_variation);
            assert!(ScreensaverClip::VARIATIONS.contains(&choice));
            routine.last_variation = Some(choice);
            assert!((30.0..60.0).contains(&routine.delay().as_secs_f32()));
        }
    }
}
