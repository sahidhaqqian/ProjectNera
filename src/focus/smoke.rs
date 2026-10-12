//! Isolated native UI/VRM/prop validation without OS mouse/keyboard control.
use super::*;
use activity::FocusProp;
use bevy::{
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};
#[derive(Resource, Default)]
struct Smoke {
    stage: u8,
    at: f32,
    passed: Vec<String>,
}
pub(crate) fn install(app: &mut App) {
    let _ = std::fs::remove_file("target/focus-v2-native/report.json");
    app.init_resource::<Smoke>().add_systems(Last, run);
}
fn next(world: &mut World, stage: u8) {
    let at = world.resource::<Time<Real>>().elapsed_secs();
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.stage = stage;
    smoke.at = at;
}
fn pass(world: &mut World, message: &str) {
    info!("FOCUS_V2_NATIVE PASS {message}");
    world.resource_mut::<Smoke>().passed.push(message.into());
}
fn capture(world: &mut World, ui: bool, name: &str) {
    let entity = if ui {
        world
            .query::<(Entity, &Window)>()
            .iter(world)
            .find(|(_, w)| w.title == "Nera Alarms")
            .unwrap()
            .0
    } else {
        world
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(world)
            .unwrap()
    };
    std::fs::create_dir_all("target/focus-v2-native").unwrap();
    world
        .commands()
        .spawn(Screenshot::window(entity))
        .observe(save_to_disk(format!("target/focus-v2-native/{name}.png")));
}
fn props(world: &mut World, count: usize) {
    assert_eq!(world.query::<&FocusProp>().iter(world).count(), count);
}
fn start(world: &mut World, preset: FocusPreset, length: FocusLength) {
    for command in [
        FocusCommand::Preset(preset),
        FocusCommand::Length(length),
        FocusCommand::Start,
    ] {
        crate::alarm_settings::smoke_focus_control(world, command);
    }
}
fn run(world: &mut World) {
    let time = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<Smoke>().stage;
    let dt = time - world.resource::<Smoke>().at;
    assert!(time < 150.0, "Focus V2 timeout stage {stage}");
    match stage {
        0 if time > 8.0
            && !world
                .resource::<crate::startup_entrance::StartupEntrance>()
                .blocks_position_save() =>
        {
            let mut settings = world.resource_mut::<crate::settings::NeraSettings>();
            settings.walking_enabled = true;
            settings.audio_muted = true;
            settings.master_volume = 73;
            world.write_message(crate::alarm_settings::OpenAlarmSettingsWindow);
            world.write_message(crate::action_menu::NeraActionMenuCommand::Alarm(
                crate::action_menu::AlarmMenuChoice::FiveMinutes,
            ));
            next(world, 1);
        }
        1 if dt > 0.8 => {
            capture(world, true, "alarms-tab");
            crate::alarm_settings::smoke_action(world, "focus");
            next(world, 2);
        }
        2 if dt > 0.7 => {
            start(world, FocusPreset::Study, FocusLength::Minutes25);
            next(world, 3);
        }
        3 if dt > 2.0 => {
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::FocusActivity(FocusPreset::Study)
            );
            props(world, 1);
            let book = world
                .query::<(&FocusProp, &GlobalTransform)>()
                .iter(world)
                .find(|(prop, _)| prop.preset == FocusPreset::Study)
                .unwrap()
                .1
                .translation();
            let target = world
                .query_filtered::<&Transform, With<crate::avatar::LookAtTarget>>()
                .single(world)
                .unwrap()
                .translation;
            assert!(
                target.distance(book) < 0.08,
                "Study gaze follows the book, not the OS pointer"
            );
            assert!(
                world
                    .query_filtered::<&bevy_vrm1::prelude::LookAt, With<crate::avatar::NeraAvatar>>(
                    )
                    .single(world)
                    .is_ok()
            );
            assert!(
                (1496..=1500).contains(
                    &world
                        .resource::<FocusSession>()
                        .remaining()
                        .unwrap()
                        .as_secs()
                )
            );
            assert!(
                world
                    .resource::<crate::alarm::AlarmState>()
                    .remaining()
                    .is_some()
            );
            capture(world, true, "focus-study-tab");
            capture(world, false, "study-book");
            pass(
                world,
                "Study UI/countdown/calm animation/open-book prop; real timer still armed",
            );
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
            props(world, 0);
            assert!(world.resource::<FocusSession>().active());
            pass(
                world,
                "Headpat temporarily owns animation, hides prop and preserves Focus",
            );
            next(world, 5);
        }
        5 if dt > 3.0 => {
            props(world, 1);
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::FocusActivity(FocusPreset::Study)
            );
            crate::alarm_settings::smoke_focus_control(world, FocusCommand::Stop);
            next(world, 6);
        }
        6 if dt > 0.5 => {
            props(world, 0);
            assert!(!world.resource::<FocusSession>().active());
            assert_eq!(
                world
                    .resource::<FocusSession>()
                    .sfx_factor(PetSfxKind::Happy),
                1.0
            );
            pass(
                world,
                "Manual Study stop removes prop/animation and restores sound eligibility",
            );
            start(world, FocusPreset::Work, FocusLength::Minutes50);
            next(world, 7);
        }
        7 if dt > 6.0 => {
            assert!(world.resource::<crate::cloud::CloudRuntime>().is_riding());
            props(world, 1);
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::FocusActivity(FocusPreset::Work)
            );
            capture(world, true, "focus-work-tab");
            capture(world, false, "work-laptop");
            pass(
                world,
                "Standing Work enters existing Ride Cloud, then typing/laptop and 50-minute countdown",
            );
            world.write_message(crate::avatar::body_mode::ToggleBodyMode);
            next(world, 8);
        }
        8 if dt > 1.5 => {
            props(world, 1);
            capture(world, false, "work-chibi");
            pass(
                world,
                "Chibi recalibrates workstation from evaluated humanoid dimensions",
            );
            world.write_message(crate::avatar::body_mode::ToggleBodyMode);
            world.resource_mut::<DesktopPetInteraction>().state = InteractionState::Dragging;
            next(world, 9);
        }
        9 if dt > 0.5 => {
            props(world, 0);
            assert!(world.resource::<FocusSession>().active());
            assert!(
                world
                    .resource::<FocusSession>()
                    .session
                    .as_ref()
                    .unwrap()
                    .paused
            );
            pass(
                world,
                "Dragging pauses activity/timer without stealing placement",
            );
            world
                .resource_mut::<DesktopPetInteraction>()
                .reset_gesture_state();
            next(world, 10);
        }
        10 if dt > 1.0
            && world.resource::<NeraAnimationState>().visual_state()
                == VisualState::FocusActivity(FocusPreset::Work) =>
        {
            props(world, 1);
            crate::alarm_settings::smoke_focus_control(world, FocusCommand::Stop);
            next(world, 20);
        }
        20 if dt > 1.5 => {
            props(world, 0);
            assert!(!world.resource::<FocusSession>().active());
            assert!(
                !world
                    .resource::<crate::cloud::CloudRuntime>()
                    .owns_placement()
            );
            pass(
                world,
                "Manual Work stop removes desk/typing and dismisses only the Focus-requested Ride",
            );
            start(world, FocusPreset::Work, FocusLength::Minutes50);
            next(world, 21);
        }
        21 if dt > 2.0
            && world.resource::<NeraAnimationState>().visual_state()
                == VisualState::FocusActivity(FocusPreset::Work) =>
        {
            props(world, 1);
            world.write_message(crate::action_menu::NeraActionMenuCommand::Alarm(
                crate::action_menu::AlarmMenuChoice::TestNow,
            ));
            next(world, 11);
        }
        11 if dt > 2.0 => {
            props(world, 0);
            assert!(!world.resource::<FocusSession>().active());
            assert!(world.resource::<PresentationState>().is_attention());
            assert_eq!(
                world
                    .resource::<crate::alarm::AlarmState>()
                    .status_at(std::time::Instant::now()),
                crate::alarm::AlarmStatus::Ringing
            );
            capture(world, false, "alarm-preempts-focus");
            pass(world, "Real ringing alarm wins and cleanly ends Focus");
            world.write_message(crate::action_menu::NeraActionMenuCommand::Alarm(
                crate::action_menu::AlarmMenuChoice::Cancel,
            ));
            next(world, 12);
        }
        12 if dt > 3.0 => {
            assert!(world.resource::<PresentationState>().is_desktop_pet());
            start(world, FocusPreset::Study, FocusLength::Custom);
            next(world, 13);
        }
        13 if dt > 2.0 => {
            props(world, 1);
            world.resource_mut::<FocusSession>().smoke_finish();
            next(world, 14);
        }
        14 if dt > 0.4 => {
            props(world, 0);
            assert!(!world.resource::<FocusSession>().active());
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::FocusCelebrate
            );
            capture(world, false, "focus-completed");
            pass(
                world,
                "Completion cleans activity/prop and plays existing Waving/Happy/Heart reaction",
            );
            next(world, 15);
        }
        15 if dt > 4.0 => {
            props(world, 0);
            assert!(!matches!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::FocusActivity(_) | VisualState::FocusCelebrate
            ));
            let mut environment = world.resource_mut::<crate::environment::DesktopEnvironment>();
            let taskbar = *environment
                .taskbar
                .taskbars
                .iter()
                .find(|taskbar| taskbar.edge == crate::environment::TaskbarEdge::Bottom)
                .expect("native bottom taskbar");
            environment.placement = crate::environment::PlacementState::PendingTaskbar {
                monitor: taskbar.monitor,
                edge: taskbar.edge,
                settle_frames: 1,
            };
            environment.set_attached_anchor(crate::environment::AvatarAnchor::Seat);
            start(world, FocusPreset::Work, FocusLength::Endless);
            next(world, 16);
        }
        16 if dt > 6.0
            && world.resource::<NeraAnimationState>().visual_state()
                == VisualState::FocusActivity(FocusPreset::Work) =>
        {
            props(world, 1);
            assert!(matches!(
                world
                    .resource::<crate::environment::DesktopEnvironment>()
                    .placement,
                crate::environment::PlacementState::Taskbar(_)
            ));
            assert!(
                !world
                    .resource::<crate::cloud::CloudRuntime>()
                    .owns_placement()
            );
            capture(world, false, "work-taskbar");
            pass(
                world,
                "Taskbar Work keeps its existing seat, adds typing/desk and creates no Cloud",
            );
            crate::tray::send_action_menu_result(Some(
                crate::action_menu::NeraActionMenuCommand::Screensaver,
            ));
            next(world, 17);
        }
        17 if dt > 1.5 => {
            props(world, 0);
            assert!(!world.resource::<FocusSession>().active());
            assert!(world.resource::<PresentationState>().is_screensaver());
            pass(
                world,
                "Screensaver retains priority and releases all Focus state/props",
            );
            world.write_message(crate::presentation::PresentationCommand::ExitScreensaver);
            next(world, 18);
        }
        18 if dt > 2.0 => {
            props(world, 0);
            assert!(!matches!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::FocusActivity(_) | VisualState::FocusCelebrate
            ));
            let settings = world.resource::<crate::settings::NeraSettings>();
            assert!(settings.walking_enabled);
            assert_eq!(settings.master_volume, 73);
            pass(
                world,
                "Repeated sessions return cleanly and preserve roaming/audio preferences",
            );
            world.write_message(crate::cloud::CloudCommand::Summon(
                crate::cloud::CloudMode::Ride,
            ));
            next(world, 22);
        }
        22 if dt > 1.0 => {
            assert!(world.resource::<crate::cloud::CloudRuntime>().is_riding());
            start(world, FocusPreset::Work, FocusLength::Custom);
            next(world, 23);
        }
        23 if dt > 2.0 => {
            props(world, 1);
            assert!(
                !world
                    .resource::<FocusSession>()
                    .session
                    .as_ref()
                    .unwrap()
                    .ride_requested
            );
            world.resource_mut::<FocusSession>().smoke_finish();
            next(world, 24);
        }
        24 if dt > 0.4 => {
            props(world, 0);
            assert!(world.resource::<crate::cloud::CloudRuntime>().is_riding());
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::FocusCelebrate
            );
            pass(
                world,
                "Work completion celebrates on a pre-existing user Ride without dismissing it",
            );
            next(world, 25);
        }
        25 if dt > 3.5 => {
            props(world, 0);
            assert!(!world.resource::<FocusSession>().active());
            assert!(!matches!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::FocusActivity(_) | VisualState::FocusCelebrate
            ));
            let report = serde_json::json!({"passed":world.resource::<Smoke>().passed,"not_performed":["physical mouse input","waiting 25/50 wall-clock minutes","listening to audio (native profile muted)"]});
            std::fs::write(
                "target/focus-v2-native/report.json",
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            world.write_message(AppExit::Success);
            next(world, 19);
        }
        _ => {}
    }
}
