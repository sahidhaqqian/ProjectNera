//! Opt-in native smoke run with a separate settings profile and bounded lifetime.
use crate::{
    character::library::{CharacterInstances, CharacterLibraryRequest},
    interaction::{BodyPart, NeraClicked, NeraPointerPosition},
};
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::{PrimaryWindow, WindowCloseRequested},
};

#[derive(Resource, Default)]
struct Smoke {
    stage: u8,
    processes: Vec<u32>,
}

pub(crate) fn install(app: &mut App) {
    if std::env::args().any(|arg| arg == "--library-motion-smoke") {
        crate::animation::install_library_motion_smoke(app);
    } else if std::env::args().any(|arg| arg == "--focus-activity-smoke") {
        crate::focus::smoke::install(app);
    } else if std::env::args().any(|arg| arg == "--treat-focus-smoke") {
        crate::treat::smoke::install(app);
    } else if std::env::args().any(|arg| arg == "--living-smoke") {
        crate::pet_behavior::living::smoke::install(app);
    } else if std::env::args().any(|arg| arg == "--alarm-timer-smoke") {
        crate::alarm_settings::install_timer_smoke(app);
    } else if std::env::args().any(|arg| arg == "--presentation-smoke") {
        crate::presentation::install_smoke(app);
    } else if std::env::args().any(|arg| arg == "--touch-smoke") {
        crate::animation::touch::smoke::install(app);
    } else if std::env::args().any(|arg| arg == "--chibi-smoke") {
        crate::avatar::body_mode::install_smoke(app);
    } else if std::env::args().any(|arg| arg == "--cloud-smoke") {
        crate::cloud::install_smoke(app);
    } else if std::env::args().any(|arg| arg == "--motion-smoke") {
        app.init_resource::<MotionSmoke>()
            .add_systems(Last, motion_run);
    } else if std::env::args().any(|arg| arg == "--throw-settings-smoke") {
        app.init_resource::<ThrowSettingsSmoke>()
            .add_systems(Last, throw_settings_run);
    } else {
        app.init_resource::<Smoke>().add_systems(Last, run);
    }
}

#[derive(Resource, Default)]
struct ThrowSettingsSmoke(u8);

fn throw_settings_run(world: &mut World) {
    let elapsed = world.resource::<Time<Real>>().elapsed_secs();
    let directory = std::env::temp_dir().join("nera-feature-smoke");
    let _ = std::fs::create_dir_all(&directory);
    match world.resource::<ThrowSettingsSmoke>().0 {
        0 if elapsed > 2.0 => {
            let settings = world.resource::<crate::settings::NeraSettings>();
            assert!(settings.throw_enabled && settings.gravity_enabled);
            assert_eq!(settings.throw_intensity, 50);
            world.write_message(crate::settings::OpenSettingsWindow);
            world.resource_mut::<ThrowSettingsSmoke>().0 = 1;
        }
        1 if elapsed > 4.0 => {
            scroll_throw_settings(world, 425.0);
            world.resource_mut::<ThrowSettingsSmoke>().0 = 2;
        }
        2 if elapsed > 5.0 => {
            screenshot_throw_settings(world, "settings-throw-default.png");
            world.resource_mut::<ThrowSettingsSmoke>().0 = 3;
        }
        3 if elapsed > 6.0 => {
            crate::settings::smoke_throw_controls(world, 25);
            let settings = world.resource::<crate::settings::NeraSettings>();
            assert!(!settings.throw_enabled && settings.gravity_enabled);
            assert_eq!(settings.throw_intensity, 25);
            assert_eq!(settings.throw_strength(), 0.0);
            world.resource_mut::<ThrowSettingsSmoke>().0 = 4;
        }
        4 if elapsed > 7.0 => {
            screenshot_throw_settings(world, "settings-throw-off.png");
            world.resource_mut::<ThrowSettingsSmoke>().0 = 5;
        }
        5 if elapsed > 8.0 => {
            crate::settings::smoke_throw_controls(world, 100);
            let settings = world.resource::<crate::settings::NeraSettings>();
            assert!(settings.throw_enabled && settings.gravity_enabled);
            assert_eq!(settings.throw_intensity, 100);
            let window = throw_settings_window(world);
            world
                .get_mut::<Window>(window)
                .unwrap()
                .resolution
                .set(560.0, 520.0);
            scroll_throw_settings(world, 500.0);
            world.resource_mut::<ThrowSettingsSmoke>().0 = 6;
        }
        6 if elapsed > 10.0 => {
            screenshot_throw_settings(world, "settings-throw-compact.png");
            world.resource_mut::<ThrowSettingsSmoke>().0 = 7;
        }
        7 if elapsed > 12.0 => {
            world.write_message(AppExit::Success);
            world.resource_mut::<ThrowSettingsSmoke>().0 = 8;
        }
        _ => {}
    }
    assert!(elapsed < 25.0, "throw settings smoke timed out");
}

fn throw_settings_window(world: &mut World) -> Entity {
    world
        .query::<(Entity, &Window)>()
        .iter(world)
        .find_map(|(entity, window)| (window.title == "Nera Settings").then_some(entity))
        .unwrap()
}

fn scroll_throw_settings(world: &mut World, y: f32) {
    for (name, mut scroll) in world
        .query::<(&Name, &mut ScrollPosition)>()
        .iter_mut(world)
    {
        if name.as_str() == "Nera Settings Scroll Area" {
            scroll.0.y = y;
        }
    }
}

fn screenshot_throw_settings(world: &mut World, name: &str) {
    let window = throw_settings_window(world);
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(
            std::env::temp_dir().join("nera-feature-smoke").join(name),
        ));
}

#[derive(Resource, Default)]
struct MotionSmoke {
    stage: u8,
    at: f32,
    origin: Option<crate::platform::PhysicalRect>,
}

fn motion_run(world: &mut World) {
    use crate::{
        animation::{NeraAnimationState, VisualState},
        desktop_physics::DesktopPhysicsState,
        environment::DesktopEnvironment,
        settings::NeraSettings,
    };
    let elapsed = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<MotionSmoke>().stage;
    let dt = elapsed - world.resource::<MotionSmoke>().at;
    let directory = std::env::temp_dir().join("nera-feature-smoke");
    let _ = std::fs::create_dir_all(&directory);
    match stage {
        0 if elapsed > 8.0 && world.resource::<DesktopPhysicsState>().support.is_some() => {
            world.resource_mut::<NeraSettings>().audio_muted = true;
            world.resource_mut::<NeraSettings>().throw_enabled = true;
            world.resource_mut::<NeraSettings>().throw_intensity = 100;
            world
                .resource_mut::<NeraAnimationState>()
                .cancel_startup_greeting();
            world
                .resource_mut::<NeraAnimationState>()
                .cancel_idle_variation();
            motion_raise(world, 220);
            world.resource_mut::<MotionSmoke>().origin =
                world.resource::<DesktopEnvironment>().nera_rect;
            world.write_message(crate::interaction::NeraThrow {
                velocity: Vec2::new(1200.0, -650.0),
            });
            world.resource_mut::<MotionSmoke>().stage = 1;
            world.resource_mut::<MotionSmoke>().at = elapsed;
        }
        1 if dt > 0.3 => {
            let old = world.resource::<MotionSmoke>().origin.unwrap();
            let now = world.resource::<DesktopEnvironment>().nera_rect.unwrap();
            assert!(
                now.left > old.left + 40 && now.top < old.top,
                "throw must move sideways and upward"
            );
            assert!(world.resource::<DesktopPhysicsState>().throwing);
            motion_screenshot(world, "throw-arc.png");
            world.resource_mut::<MotionSmoke>().stage = 2;
        }
        2 if dt > 4.0 => {
            assert!(
                !world.resource::<DesktopPhysicsState>().falling
                    && world.resource::<DesktopPhysicsState>().support.is_some(),
                "throw must land"
            );
            world.resource_mut::<NeraSettings>().gravity_enabled = false;
            motion_raise(world, 180);
            world.resource_mut::<MotionSmoke>().origin =
                world.resource::<DesktopEnvironment>().nera_rect;
            world.write_message(crate::interaction::NeraThrow {
                velocity: Vec2::new(600.0, -250.0),
            });
            world.resource_mut::<MotionSmoke>().stage = 3;
            world.resource_mut::<MotionSmoke>().at = elapsed;
        }
        3 if dt > 0.3 => {
            let old = world.resource::<MotionSmoke>().origin.unwrap();
            let now = world.resource::<DesktopEnvironment>().nera_rect.unwrap();
            assert!(
                now.left > old.left + 25 && now.top < old.top,
                "gravity off must still retain launch momentum"
            );
            world.resource_mut::<MotionSmoke>().stage = 4;
        }
        4 if dt > 3.2 => {
            assert!(
                !world.resource::<DesktopPhysicsState>().throwing
                    && !world.resource::<DesktopPhysicsState>().falling
            );
            world.resource_mut::<MotionSmoke>().origin =
                world.resource::<DesktopEnvironment>().nera_rect;
            motion_screenshot(world, "gravity-off.png");
            world.resource_mut::<MotionSmoke>().stage = 5;
            world.resource_mut::<MotionSmoke>().at = elapsed;
        }
        5 if dt > 1.0 => {
            assert_eq!(
                world.resource::<MotionSmoke>().origin,
                world.resource::<DesktopEnvironment>().nera_rect,
                "gravity off must keep the unsupported placement"
            );
            world.resource_mut::<NeraSettings>().gravity_enabled = true;
            world.write_message(crate::character::ImportCharacter(
                std::path::Path::new(&crate::nera_asset_path()).join("vrm/nera0.0.vrm"),
            ));
            world.resource_mut::<MotionSmoke>().stage = 6;
            world.resource_mut::<MotionSmoke>().at = elapsed;
        }
        6 if dt > 3.0
            && !world
                .resource::<crate::character::CharacterImportStatus>()
                .loading
            && world.resource::<DesktopPhysicsState>().support.is_some() =>
        {
            assert!(
                world
                    .resource::<crate::character::ActiveCharacter>()
                    .migrated_from_vrm0
            );
            for mut camera in world
                .query_filtered::<&mut Camera, With<crate::avatar::NeraMainCamera>>()
                .iter_mut(world)
            {
                camera.clear_color = ClearColorConfig::Custom(Color::WHITE);
            }
            world.write_message(crate::animation::PlayLibraryAnimation {
                path: "animations/animasi_basic/Death01.vrma".into(),
            });
            world.resource_mut::<MotionSmoke>().stage = 7;
            world.resource_mut::<MotionSmoke>().at = elapsed;
        }
        7 if matches!(
            world.resource::<NeraAnimationState>().visual_state(),
            VisualState::LibraryAnimation(_)
        ) =>
        {
            world.resource_mut::<MotionSmoke>().stage = 8;
            world.resource_mut::<MotionSmoke>().at = elapsed;
        }
        8 if dt > 1.7 => {
            assert!(matches!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::LibraryAnimation(_)
            ));
            crate::settings::assert_smoke_shadow_ground(world);
            motion_screenshot(world, "legacy-death-shadow.png");
            std::fs::write(directory.join("motion.json"), br#"{"throw_arc":true,"landing":true,"gravity_off_coast":true,"gravity_off_stable":true,"legacy_death_ground_shadow":true}"#).unwrap();
            world.resource_mut::<MotionSmoke>().stage = 9;
            world.resource_mut::<MotionSmoke>().at = elapsed;
        }
        9 if dt > 1.0 => {
            world.write_message(AppExit::Success);
            world.resource_mut::<MotionSmoke>().stage = 10;
        }
        _ => {}
    }
    assert!(elapsed < 60.0, "motion smoke timed out at stage {stage}");
}

fn motion_raise(world: &mut World, pixels: i32) {
    let environment = world.resource::<crate::environment::DesktopEnvironment>();
    let hwnd = environment.nera_hwnd.unwrap();
    let rect = environment.nera_rect.unwrap();
    crate::platform::move_window(
        hwnd,
        crate::platform::PhysicalPoint::new(rect.left, rect.top - pixels),
    )
    .unwrap();
    world
        .resource_mut::<crate::environment::DesktopEnvironment>()
        .nera_rect = crate::platform::window_rect(hwnd);
    world
        .resource_mut::<crate::desktop_physics::DesktopPhysicsState>()
        .reset_for_position_recovery();
}

fn motion_screenshot(world: &mut World, name: &str) {
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(
            std::env::temp_dir().join("nera-feature-smoke").join(name),
        ));
}

fn run(world: &mut World) {
    let elapsed = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<Smoke>().stage;
    let directory = std::env::temp_dir().join("nera-feature-smoke");
    let _ = std::fs::create_dir_all(&directory);
    match stage {
        0 if elapsed > 2.0 => {
            world
                .resource_mut::<crate::settings::NeraSettings>()
                .audio_muted = true;
            for _ in 0..2 {
                world.write_message(CharacterLibraryRequest::Spawn(
                    crate::character::DEFAULT_CHARACTER.into(),
                ));
            }
            world.resource_mut::<Smoke>().stage = 1;
        }
        1 if elapsed > 12.0 => {
            let instances = world.resource::<CharacterInstances>();
            assert_eq!(
                instances.companions.len(),
                2,
                "both companion processes must remain alive"
            );
            let processes = instances.process_ids();
            for pid in &processes {
                let report: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(directory.join(format!("companion-{pid}.json")))
                        .expect("companion must initialize and reveal its avatar"),
                )
                .unwrap();
                assert!(report["live_bones"].as_u64().unwrap() > 20);
                assert_eq!(report["alarm_count"], 0);
            }
            world.resource_mut::<Smoke>().processes = processes;
            world.write_message(crate::settings::OpenSettingsWindow);
            world.resource_mut::<Smoke>().stage = 2;
        }
        2 if elapsed > 15.0 => {
            let window = world
                .query_filtered::<(Entity, &Window), Without<PrimaryWindow>>()
                .iter(world)
                .find(|(_, window)| window.title.contains("Settings"))
                .map(|(entity, _)| entity)
                .unwrap();
            world
                .commands()
                .spawn(Screenshot::window(window))
                .observe(save_to_disk(directory.join("character-settings.png")));
            world.resource_mut::<Smoke>().stage = 3;
        }
        3 if elapsed > 17.0 => {
            let windows: Vec<_> = world
                .query_filtered::<Entity, Without<PrimaryWindow>>()
                .iter(world)
                .filter(|entity| world.get::<Window>(*entity).is_some())
                .collect();
            for window in windows {
                world.write_message(WindowCloseRequested { window });
            }
            world.write_message(NeraClicked {
                body_part: BodyPart::LeftHand,
                pointer: NeraPointerPosition {
                    screen: IVec2::ZERO,
                    local: Vec2::ZERO,
                },
            });
            world.resource_mut::<Smoke>().stage = 4;
        }
        4 if elapsed > 17.45 => {
            assert!(matches!(
                world
                    .resource::<crate::animation::NeraAnimationState>()
                    .visual_state(),
                crate::animation::VisualState::TouchReaction(_)
            ));
            let window = world
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(world)
                .unwrap();
            world
                .commands()
                .spawn(Screenshot::window(window))
                .observe(save_to_disk(directory.join("left-hand.png")));
            world.resource_mut::<Smoke>().stage = 5;
        }
        5 if elapsed > 19.0 => {
            world.write_message(NeraClicked {
                body_part: BodyPart::RightFoot,
                pointer: NeraPointerPosition {
                    screen: IVec2::ZERO,
                    local: Vec2::ZERO,
                },
            });
            world.resource_mut::<Smoke>().stage = 6;
        }
        6 if elapsed > 19.45 => {
            assert_eq!(
                world
                    .resource::<crate::animation::NeraAnimationState>()
                    .visual_state(),
                crate::animation::VisualState::LimbReaction(BodyPart::RightFoot)
            );
            let window = world
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(world)
                .unwrap();
            world
                .commands()
                .spawn(Screenshot::window(window))
                .observe(save_to_disk(directory.join("right-foot.png")));
            let id = world.resource::<CharacterInstances>().companions[0].id;
            world.write_message(CharacterLibraryRequest::Close(id));
            world.resource_mut::<Smoke>().stage = 7;
        }
        7 if elapsed > 24.0 => {
            assert_eq!(
                world.resource::<CharacterInstances>().companions.len(),
                1,
                "Close must terminate only the selected companion"
            );
            assert!(!matches!(
                world
                    .resource::<crate::animation::NeraAnimationState>()
                    .visual_state(),
                crate::animation::VisualState::LimbReaction(_)
            ));
            let report = serde_json::json!({"processes":world.resource::<Smoke>().processes,"spawned":2,"remaining_before_owner_exit":1,"limb_reactions":true});
            std::fs::write(
                directory.join("companions.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            world.write_message(NeraClicked {
                body_part: BodyPart::Torso,
                pointer: NeraPointerPosition {
                    screen: IVec2::ZERO,
                    local: Vec2::ZERO,
                },
            });
            world.write_message(crate::alarm_settings::OpenAlarmSettingsWindow);
            world.resource_mut::<Smoke>().stage = 8;
        }
        8 => {
            let strength = world
                .query::<&crate::expression::face::AutomaticAngry>()
                .iter(world)
                .next()
                .unwrap()
                .0;
            if strength <= 0.9 {
                assert!(
                    elapsed < 25.0,
                    "angry face did not reach its peak: {:?}",
                    world
                        .resource::<crate::animation::NeraAnimationState>()
                        .visual_state()
                );
                return;
            }
            let window = world
                .query_filtered::<Entity, With<PrimaryWindow>>()
                .single(world)
                .unwrap();
            world
                .commands()
                .spawn(Screenshot::window(window))
                .observe(save_to_disk(directory.join("automatic-angry.png")));
            screenshot_alarm(world, "alarms-list.png");
            crate::alarm_settings::smoke_action(world, "add");
            world.resource_mut::<Smoke>().stage = 9;
        }
        9 if elapsed > 28.0 => {
            for _ in 0..3 {
                crate::alarm_settings::smoke_action(world, "sound");
            }
            crate::alarm_settings::smoke_action(world, "preview");
            world.resource_mut::<Smoke>().stage = 10;
        }
        10 if elapsed > 30.0 => {
            screenshot_alarm(world, "alarms-editor.png");
            crate::alarm_settings::smoke_action(world, "save");
            let alarms = world
                .resource::<crate::settings::NeraSettings>()
                .alarm_configs();
            assert_eq!(
                alarms.last().unwrap().sound,
                4,
                "sound selection must survive Save"
            );
            world.resource_mut::<Smoke>().stage = 11;
        }
        11 if elapsed > 32.0 => {
            screenshot_alarm(world, "alarms-saved.png");
            world.resource_mut::<Smoke>().stage = 12;
        }
        12 if elapsed > 34.0 => {
            world.write_message(AppExit::Success);
            world.resource_mut::<Smoke>().stage = 13;
        }
        _ => {}
    }
    if elapsed > 45.0 {
        panic!("native feature smoke timed out");
    }
}

fn screenshot_alarm(world: &mut World, name: &str) {
    let window = world
        .query::<(Entity, &Window)>()
        .iter(world)
        .find(|(_, window)| window.title == "Nera Alarms")
        .map(|(entity, _)| entity)
        .unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(
            std::env::temp_dir().join("nera-feature-smoke").join(name),
        ));
}
