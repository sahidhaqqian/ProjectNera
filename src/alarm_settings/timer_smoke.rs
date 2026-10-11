//! Scripted native timer/Cloud handoff validation, using the isolated feature-smoke profile.
use super::*;
use crate::{
    cloud::{CloudCommand, CloudMode, CloudRuntime},
    environment::{AvatarAnchor, DesktopEnvironment},
    presentation::{PresentationCommand, PresentationState},
};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

#[derive(Resource, Default)]
struct Smoke {
    stage: u8,
    next: f64,
    remaining: u64,
    saved_book: String,
}

pub(crate) fn install(app: &mut App) {
    let _ = std::fs::remove_file(output().join("report.json"));
    app.init_resource::<Smoke>().add_systems(Last, run);
}

fn output() -> std::path::PathBuf {
    "target/alarm-timer-native".into()
}

fn capture(world: &mut World, title: &str, file: &str) {
    let window = world
        .query::<(Entity, &Window)>()
        .iter(world)
        .find_map(|(entity, window)| (window.title == title).then_some(entity))
        .unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(output().join(file)));
}

fn remaining(world: &World) -> u64 {
    world
        .resource::<AlarmState>()
        .remaining()
        .expect("accepted timer must be armed")
        .as_secs()
}

fn verify_card(world: &mut World, active: bool) {
    let card = world
        .query_filtered::<&Node, With<ActiveTimerCard>>()
        .single(world)
        .unwrap();
    assert_eq!(
        card.display,
        if active { Display::Flex } else { Display::None }
    );
    if active {
        let remaining = world.resource::<AlarmState>().remaining().unwrap();
        let expected = timer_countdown(remaining);
        let previous_second = timer_countdown(remaining + std::time::Duration::from_secs(1));
        assert!(
            world
                .query::<(&ActiveTimerText, &Text)>()
                .iter(world)
                .any(|(part, text)| matches!(part, ActiveTimerText::Countdown)
                    && (text.0 == expected || text.0 == previous_second))
        );
    }
}

fn enter_screensaver() {
    // Same typed result as the real radial action, without touching the OS pointer.
    crate::tray::send_action_menu_result(Some(NeraActionMenuCommand::Screensaver));
}

fn verify_handoff(world: &mut World) {
    assert!(world.resource::<PresentationState>().is_screensaver());
    assert!(!world.resource::<CloudRuntime>().owns_placement());
    assert_eq!(
        world.resource::<DesktopEnvironment>().attached_anchor,
        AvatarAnchor::Feet
    );
    assert!(
        !world
            .query::<&crate::cloud::CloudVisual>()
            .iter(world)
            .any(|_| true)
    );
}

fn run(world: &mut World) {
    let elapsed = world.resource::<Time<Real>>().elapsed_secs_f64();
    if elapsed < 3.0 || elapsed < world.resource::<Smoke>().next {
        return;
    }
    assert!(elapsed < 70.0, "timer/Cloud native smoke timed out");
    world.resource_scope(|world, mut smoke: Mut<Smoke>| {
        match smoke.stage {
            0 => {
                if !world.query_filtered::<Entity, (With<crate::avatar::NeraAvatar>, With<bevy_vrm1::prelude::Initialized>)>().iter(world).any(|_| true) { return; }
                std::fs::create_dir_all(output()).unwrap();
                world.resource_mut::<NeraSettings>().audio_muted = true;
                smoke.saved_book = world.resource::<NeraSettings>().alarm_book.clone();
                world.write_message(OpenAlarmSettingsWindow);
            }
            1 => {
                if !world.query_filtered::<&UtilityWindow, With<AlarmSettingsWindow>>().iter(world).any(|shell| shell.native_handle().is_some()) { return; }
                smoke_action(world, "quick5");
            }
            2 => {
                assert!((297..=300).contains(&remaining(world)));
                verify_card(world, true);
                smoke.remaining = remaining(world);
                capture(world, "Nera Alarms", "five-minute-active.png");
            }
            3 => {
                assert!(remaining(world) < smoke.remaining);
                verify_card(world, true);
                smoke_action(world, "quick10");
            }
            4 => {
                assert!((597..=600).contains(&remaining(world)));
                verify_card(world, true);
                smoke_action(world, "close");
            }
            5 => { world.write_message(OpenAlarmSettingsWindow); }
            6 => {
                verify_card(world, true);
                assert!((590..=600).contains(&remaining(world)));
                capture(world, "Nera Alarms", "timer-reopened.png");
                smoke_action(world, "cancel_timer");
            }
            7 => {
                verify_card(world, false);
                assert!(world.resource::<AlarmState>().remaining().is_none());
                assert_eq!(world.resource::<NeraSettings>().alarm_book, smoke.saved_book);
                capture(world, "Nera Alarms", "timer-cancelled.png");
                smoke_action(world, "close");
            }
            8 => { world.write_message(CloudCommand::Summon(CloudMode::Ride)); }
            9 => {
                assert_eq!(world.resource::<CloudRuntime>().mode(), Some(CloudMode::Ride));
                enter_screensaver();
            }
            10 => {
                verify_handoff(world);
                capture(world, "Nera", "ride-to-screensaver.png");
                world.write_message(PresentationCommand::ExitScreensaver);
            }
            11 => {
                assert!(world.resource::<PresentationState>().is_desktop_pet());
                assert!(!world.resource::<CloudRuntime>().owns_placement());
                assert_eq!(world.resource::<DesktopEnvironment>().attached_anchor, AvatarAnchor::Feet);
                world.write_message(CloudCommand::Summon(CloudMode::Rest));
            }
            12 => {
                assert_eq!(world.resource::<CloudRuntime>().mode(), Some(CloudMode::Rest));
                enter_screensaver();
            }
            13 => {
                verify_handoff(world);
                capture(world, "Nera", "rest-to-screensaver.png");
                world.write_message(PresentationCommand::ExitScreensaver);
            }
            14 => {
                assert!(world.resource::<PresentationState>().is_desktop_pet());
                assert!(!world.resource::<CloudRuntime>().owns_placement());
                assert_eq!(world.resource::<DesktopEnvironment>().attached_anchor, AvatarAnchor::Feet);
                for name in ["five-minute-active.png", "timer-reopened.png", "timer-cancelled.png", "ride-to-screensaver.png", "rest-to-screensaver.png"] {
                    assert!(output().join(name).is_file(), "missing native capture {name}");
                }
                let report = serde_json::json!({"status":"PASSED", "quick_timer":"accepted 5/10 min; countdown advances; reopening preserves deadline; cancel hides card; saved alarms unchanged", "screensaver":"Ride/Rest release Cloud and enter through radial command; exit uses feet/free support", "physical_input":"NOT PERFORMED; scripted semantic actions only"});
                std::fs::write(output().join("report.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
                info!("ALARM_TIMER_NATIVE PASSED timer card and Rest/Ride -> Screensaver");
                world.write_message(bevy::app::AppExit::Success);
            }
            _ => return,
        }
        smoke.stage += 1;
        smoke.next = elapsed + if smoke.stage == 12 { 4.0 } else { 1.5 };
    });
}
