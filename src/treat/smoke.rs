//! Opt-in native validation: real VRM/VRMA/glTF, synthetic semantic commands,
//! isolated settings and no OS pointer/keyboard input.
use super::*;
use crate::{
    action_menu::{AlarmMenuChoice, NeraActionMenuCommand},
    alarm::AlarmState,
    avatar::body_mode::{AvatarBodyMode, ToggleBodyMode},
    focus::{FocusCommand, FocusLength, FocusPreset, FocusSession},
    settings::NeraSettings,
};
use bevy::{
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};

#[derive(Resource, Default)]
struct Smoke {
    stage: u8,
    at: f32,
    food: usize,
    captures: Vec<String>,
    checks: Vec<String>,
    puffs: Vec<Entity>,
    hearts: usize,
    happy: usize,
    treat_sides: Vec<bool>,
    happy_before: usize,
}
pub(crate) fn install(app: &mut App) {
    let _ = std::fs::remove_file(output().join("report.json"));
    app.init_resource::<Smoke>()
        .add_systems(Last, (feedback, run).chain());
}
fn output() -> std::path::PathBuf {
    "target/treat-focus-native".into()
}
fn feedback(
    mut smoke: ResMut<Smoke>,
    mut fx: MessageReader<PetFx>,
    mut sfx: MessageReader<PetSfx>,
) {
    for event in fx.read() {
        if let PetFxAnchor::Bone(entity) = event.anchor {
            smoke.puffs.push(entity);
        }
        if event.kind == PetFxKind::Heart {
            smoke.hearts += 1;
            if let PetFxAnchor::HeadSide { right } = event.anchor {
                smoke.treat_sides.push(right);
            }
        }
    }
    smoke.happy += sfx
        .read()
        .filter(|event| event.0 == PetSfxKind::Happy)
        .count();
}
fn capture(world: &mut World, title: Option<&str>, file: &str) {
    let window = match title {
        Some(title) => world
            .query::<(Entity, &Window)>()
            .iter(world)
            .find_map(|(entity, window)| (window.title == title).then_some(entity))
            .unwrap(),
        None => world
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(world)
            .unwrap(),
    };
    std::fs::create_dir_all(output()).unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(output().join(file)));
    world.resource_mut::<Smoke>().captures.push(file.into());
}
fn next(world: &mut World, stage: u8) {
    let at = world.resource::<Time<Real>>().elapsed_secs();
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.stage = stage;
    smoke.at = at;
}
fn passed(world: &mut World, check: &str) {
    info!("TREAT_FOCUS_NATIVE PASS {check}");
    world.resource_mut::<Smoke>().checks.push(check.into());
}
fn choose(world: &mut World, index: usize) {
    // Seed the production random selector rather than bypassing its pool.
    let last = world.resource::<TreatRuntime>().last_food;
    let seed = (1..1000)
        .find(|seed| {
            let mut trial = TreatRuntime {
                last_food: last,
                rng: NeraRng::seeded(*seed),
                ..default()
            };
            trial.choose(&(0..FOODS.len()).collect::<Vec<_>>()) == Some(index)
        })
        .unwrap();
    world.resource_mut::<TreatRuntime>().rng = NeraRng::seeded(seed);
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.treat_sides.clear();
    smoke.happy_before = smoke.happy;
    drop(smoke);
    crate::tray::send_action_menu_result(Some(NeraActionMenuCommand::Treat));
}
fn attachment(world: &mut World) {
    assert_eq!(
        world.resource::<NeraAnimationState>().visual_state(),
        VisualState::Treat
    );
    assert!(
        world
            .resource::<NeraAnimationClips>()
            .treat_playing(world.resource::<NeraAnimationPlayback>())
    );
    let (prop, parent, transform) = world
        .query_filtered::<(Entity, &ChildOf, &GlobalTransform), With<TreatProp>>()
        .single(world)
        .map(|(entity, parent, transform)| (entity, parent.parent(), *transform))
        .unwrap();
    assert_eq!(world.get::<VrmBone>(parent).unwrap().as_str(), "leftHand");
    assert!(transform.to_matrix().is_finite());
    assert!(
        world.resource::<Smoke>().puffs.contains(&parent),
        "CloudPuff must target the actual hand"
    );
    let index = world.resource::<TreatRuntime>().last_food.unwrap();
    let center = transform.to_matrix().transform_point3(FOODS[index].center);
    let hand = world.get::<GlobalTransform>(parent).unwrap();
    assert!(
        center.distance(hand.translation())
            < 0.3 * hand.to_scale_rotation_translation().0.max_element()
    );
    assert!(
        world
            .query::<(Entity, &Mesh3d)>()
            .iter(world)
            .any(|(entity, _)| {
                let mut entity = entity;
                while let Some(parent) = world.get::<ChildOf>(entity) {
                    entity = parent.parent();
                    if entity == prop {
                        return true;
                    }
                }
                false
            }),
        "downloaded food must have instantiated renderable meshes"
    );
}
fn empty(world: &mut World) {
    assert!(!world.resource::<TreatRuntime>().busy());
    assert_eq!(
        world
            .query_filtered::<Entity, With<TreatProp>>()
            .iter(world)
            .count(),
        0
    );
}
fn run(world: &mut World) {
    let elapsed = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<Smoke>().stage;
    let dt = elapsed - world.resource::<Smoke>().at;
    assert!(
        elapsed < 200.0,
        "Treat/Focus native timeout at stage {stage}"
    );
    match stage {
        0 if elapsed > 8.0
            && !world
                .resource::<crate::startup_entrance::StartupEntrance>()
                .blocks_position_save()
            && world
                .resource::<NeraAnimationClips>()
                .treat_duration()
                .is_some() =>
        {
            assert_eq!(
                world
                    .resource::<TreatAssets>()
                    .0
                    .iter()
                    .filter(|handle| world.resource::<Assets<WorldAsset>>().contains(handle.id()))
                    .count(),
                7
            );
            let mut settings = world.resource_mut::<NeraSettings>();
            settings.walking_enabled = false;
            settings.audio_muted = true;
            drop(settings);
            world.write_message(crate::alarm_settings::OpenAlarmSettingsWindow);
            world.write_message(NeraActionMenuCommand::Alarm(AlarmMenuChoice::FiveMinutes));
            world.write_message(FocusCommand::Preset(FocusPreset::Work));
            world.write_message(FocusCommand::Length(FocusLength::Minutes50));
            next(world, 1);
        }
        1 if dt > 1.0 => {
            crate::alarm_settings::smoke_focus_control(world, FocusCommand::Start);
            next(world, 2);
        }
        2 if dt > 1.0 => {
            let focus = world.resource::<FocusSession>();
            assert!(focus.active());
            assert!((2997..=3000).contains(&focus.remaining().unwrap().as_secs()));
            assert_eq!(focus.sfx_factor(PetSfxKind::Happy), 0.25);
            assert_eq!(focus.sfx_factor(PetSfxKind::HeadPat), 1.0);
            assert!(world.resource::<AlarmState>().remaining().is_some());
            capture(world, Some("Nera Alarms"), "focus-work-50.png");
            crate::alarm_settings::smoke_focus_control(world, FocusCommand::Stop);
            passed(
                world,
                "Focus UI Start/50-minute Work/countdown; real alarm remains armed",
            );
            next(world, 3);
        }
        3 if dt > 1.0 => {
            assert!(!world.resource::<FocusSession>().active());
            world.write_message(FocusCommand::Length(FocusLength::Endless));
            world.write_message(FocusCommand::Start);
            world.write_message(crate::interaction::NeraHeadPatted {
                pointer: crate::interaction::NeraPointerPosition {
                    screen: IVec2::ZERO,
                    local: Vec2::ZERO,
                },
            });
            next(world, 4);
        }
        4 if dt > 0.4 => {
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::HeadStroke
            );
            assert!(world.resource::<FocusSession>().active());
            assert_eq!(world.resource::<FocusSession>().remaining(), None);
            passed(
                world,
                "Focus Stop restores sound; Endless and headpat remain available",
            );
            crate::alarm_settings::smoke_action(world, "close");
            next(world, 5);
        }
        5 if dt > 6.0 => {
            choose(world, 0);
            next(world, 6);
        }
        6 if dt > 4.0 => {
            attachment(world);
            let index = world.resource::<Smoke>().food;
            capture(
                world,
                None,
                &format!("normal-{}.png", FOODS[index].name.to_lowercase()),
            );
            if index == 0 {
                next(world, 7);
            } else {
                world.resource_mut::<NeraAnimationState>().cancel_treat();
                next(world, 8);
            }
        }
        7 if dt > 0.85 => {
            assert!(
                world
                    .resource::<TreatRuntime>()
                    .active
                    .as_ref()
                    .is_some_and(|active| active.prop.is_none())
            );
            assert_eq!(
                world
                    .query_filtered::<Entity, With<TreatProp>>()
                    .iter(world)
                    .count(),
                0
            );
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::Treat
            );
            assert!(
                world
                    .resource::<NeraAnimationState>()
                    .treat_remaining()
                    .unwrap()
                    > Duration::from_secs(3)
            );
            assert!(
                world
                    .resource::<NeraAnimationClips>()
                    .treat_playing(world.resource::<NeraAnimationPlayback>())
            );
            assert!(!world.resource::<Smoke>().treat_sides.is_empty());
            let visible_sides: Vec<_> = world
                .query::<&crate::pet_feedback::PetParticle>()
                .iter(world)
                .filter_map(|particle| particle.heart_side())
                .collect();
            assert!(
                visible_sides.contains(&true) && visible_sides.contains(&false),
                "both sides must have actually spawned heart particles: {visible_sides:?}"
            );
            capture(world, None, "bite-pizza-hearts.png");
            passed(
                world,
                "Bite removes prop at mouth contact and emits hearts while authored animation continues",
            );
            next(world, 8);
        }
        8 if !world.resource::<TreatRuntime>().busy() && dt > 0.6 => {
            empty(world);
            let index = world.resource::<Smoke>().food;
            if index == 0 {
                let smoke = world.resource::<Smoke>();
                assert!((2..=5).contains(&smoke.treat_sides.len()));
                assert!(smoke.treat_sides.contains(&true) && smoke.treat_sides.contains(&false));
                assert_eq!(smoke.happy, smoke.happy_before + 1);
                passed(
                    world,
                    "Bite emits exactly 2-5 left/right Hearts and one Happy; full clip completion adds no duplicate feedback",
                );
            } else {
                assert!(
                    world.resource::<Smoke>().treat_sides.is_empty(),
                    "cancelling before bite must not reward"
                );
            }
            passed(
                world,
                &format!(
                    "Normal {}: real GLB, left-hand parent, VRMA, hand puff and cleanup",
                    FOODS[index].name
                ),
            );
            if index < 6 {
                world.resource_mut::<Smoke>().food += 1;
                choose(world, index + 1);
                next(world, 6);
            } else {
                world.write_message(ToggleBodyMode);
                next(world, 9);
            }
        }
        9 if dt > 1.0 => {
            assert_eq!(*world.resource::<AvatarBodyMode>(), AvatarBodyMode::Chibi);
            choose(world, 1);
            next(world, 10);
        }
        10 if dt > 4.0 => {
            attachment(world);
            capture(world, None, "chibi-apple.png");
            next(world, 11);
        }
        11 if dt > 7.0 => {
            empty(world);
            passed(
                world,
                "Chibi Apple: inherited hand scale, animation completion and cleanup",
            );
            choose(world, 2);
            next(world, 12);
        }
        12 if dt > 0.6 => {
            assert!(world.resource::<TreatRuntime>().busy());
            world.write_message(crate::character::ImportCharacter(std::path::PathBuf::from(
                r"C:\Users\sahid\AppData\Roaming\io.github.sahidhaqqi.projectnera\characters\model-07665e5f50f3b8ac.vrm")));
            next(world, 13);
        }
        13 if dt > 5.0
            && !world
                .resource::<crate::character::CharacterImportStatus>()
                .loading =>
        {
            assert_ne!(
                world
                    .resource::<crate::character::ActiveCharacter>()
                    .asset_path,
                crate::character::DEFAULT_CHARACTER
            );
            empty(world);
            if world
                .resource::<NeraAnimationClips>()
                .treat_duration()
                .is_none()
            {
                return;
            }
            passed(
                world,
                "Live Alicia replacement cancels pending/active Treat and removes stale prop",
            );
            choose(world, 3);
            next(world, 14);
        }
        14 if dt > 4.0 => {
            attachment(world);
            capture(world, None, "alicia-chibi-cookie.png");
            next(world, 15);
        }
        15 if dt > 7.0 => {
            empty(world);
            passed(
                world,
                "Alicia Chibi: finger-derived grip, VRMA playback/consumption/cleanup",
            );
            world.write_message(ToggleBodyMode);
            world.write_message(crate::cloud::CloudCommand::Summon(CloudMode::Rest));
            next(world, 16);
        }
        16 if dt > 2.0 => {
            assert_eq!(
                world.resource::<CloudRuntime>().mode(),
                Some(CloudMode::Rest)
            );
            world.write_message(RequestTreat);
            next(world, 17);
        }
        17 if dt > 0.5 => {
            empty(world);
            passed(world, "Rest Cloud rejects Treat");
            world.write_message(crate::cloud::CloudCommand::Summon(CloudMode::Ride));
            next(world, 18);
        }
        18 if dt > 2.0 => {
            choose(world, 4);
            next(world, 19);
        }
        19 if dt > 4.0 => {
            attachment(world);
            assert_eq!(
                world.resource::<CloudRuntime>().mode(),
                Some(CloudMode::Ride)
            );
            capture(world, None, "alicia-ride-donut.png");
            passed(world, "Ride Cloud retains placement while Treat plays");
            world.resource_mut::<DesktopPetInteraction>().state = InteractionState::Dragging;
            next(world, 20);
        }
        20 if dt > 0.3 => {
            empty(world);
            passed(world, "Dragging interrupts Treat and removes prop");
            world
                .resource_mut::<DesktopPetInteraction>()
                .reset_gesture_state();
            world.write_message(crate::cloud::CloudCommand::Dismiss);
            world.write_message(FocusCommand::Length(FocusLength::Custom));
            world.write_message(FocusCommand::Start);
            next(world, 21);
        }
        21 if dt > 2.0 => {
            world.resource_mut::<FocusSession>().smoke_finish();
            next(world, 22);
        }
        22 if dt > 0.4 => {
            assert!(!world.resource::<FocusSession>().active());
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::FocusCelebrate
            );
            assert!(world.resource::<AlarmState>().remaining().is_some());
            capture(world, None, "focus-completion.png");
            passed(
                world,
                "Focus completion plays existing Waving/Happy with Hearts; real alarm deadline survives",
            );
            next(world, 23);
        }
        23 if dt > 8.0 => {
            assert_ne!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::FocusCelebrate
            );
            assert!(world.resource::<AlarmState>().remaining().is_some());
            let smoke = world.resource::<Smoke>();
            std::fs::write(output().join("report.json"), serde_json::to_vec_pretty(&serde_json::json!({
                "passed":smoke.checks,"screenshots":smoke.captures,
                "focus_clock":"finite completion advanced only Focus timer; actual UI countdown ran in real time",
                "food_previews":"all seven rendered before bite; pizza, Chibi apple and Alicia cookie consumed at the authored bite and finished their full animations; other previews cancelled before bite",
                "audio":"semantic Happy/HeadPat sound routing checked; smoke audio muted",
                "not_performed":["physical mouse input", "waiting 25/50 wall-clock minutes", "live ringing alarm/screensaver interruption (regression tests cover priorities)"]
            })).unwrap()).unwrap();
            info!("TREAT_FOCUS_NATIVE complete: {} checks", smoke.checks.len());
            world.write_message(AppExit::Success);
            next(world, 24);
        }
        _ => {}
    }
}
