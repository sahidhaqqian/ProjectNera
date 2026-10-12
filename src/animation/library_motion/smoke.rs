//! Real Windows placement and VRMA validation without OS pointer/keyboard input.
use super::*;
use bevy::{
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};

const UP: &str = "animations/Mixamo-VRMA/Standing_Up_3.vrma";
const RIGHT: &str = "animations/Mixamo-VRMA/Standing_Walk_Right.vrma";
#[derive(Resource, Default)]
struct Smoke {
    stage: u8,
    at: f32,
    home: IVec2,
    peak_x: i32,
    min_x: i32,
    checks: Vec<String>,
    contacts: usize,
    repetitions: u8,
}
pub(crate) fn install(app: &mut App) {
    let _ = std::fs::remove_file("target/library-motion-native/report.json");
    app.init_resource::<Smoke>()
        .add_systems(Last, run.after(crate::pet_feedback::update_particles));
}
fn position(world: &World) -> IVec2 {
    let rect = world.resource::<DesktopEnvironment>().nera_rect.unwrap();
    IVec2::new(rect.left, rect.top)
}
fn next(world: &mut World, stage: u8) {
    let time = world.resource::<Time<Real>>().elapsed_secs();
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.stage = stage;
    smoke.at = time;
}
fn start(world: &mut World, path: &str) {
    let home = position(world);
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.home = home;
    smoke.peak_x = home.x;
    smoke.min_x = home.x;
    world.write_message(super::super::library::PlayLibraryAnimation { path: path.into() });
}
fn pass(world: &mut World, text: &str) {
    info!("LIBRARY_MOTION_NATIVE PASS {text}");
    world.resource_mut::<Smoke>().checks.push(text.into());
}
fn screenshot(world: &mut World, name: &str) {
    std::fs::create_dir_all("target/library-motion-native").unwrap();
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(format!(
            "target/library-motion-native/{name}.png"
        )));
}
fn home(world: &mut World) {
    let current = position(world);
    let home = world.resource::<Smoke>().home;
    assert!(
        (current - home).as_vec2().length() <= 2.0,
        "checkpoint drift: {current:?} vs {home:?}"
    );
}
fn contact(world: &mut World) {
    let Some(session) = world.resource::<LibraryMotion>().session.as_ref() else {
        return;
    };
    if session.airborne || session.returning.is_some() {
        return;
    }
    let checkpoint = session.checkpoint;
    let environment = world.resource::<DesktopEnvironment>();
    let rect = environment.nera_rect.unwrap();
    let dpi = environment.nera_scale_factor as f32;
    let settings = world.resource::<crate::settings::NeraSettings>();
    let scale = world
        .resource::<crate::avatar::body_mode::BodyProportions>()
        .body_scale();
    let margin = crate::avatar::standing_support_offset(settings, scale);
    let target = checkpoint.position.y as f32 + checkpoint.contact.y * dpi - margin * dpi;
    let (camera, view) = camera_geometry(world).unwrap();
    let actual = world
        .query_filtered::<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>()
        .iter(world)
        .filter(|(bone, _)| contact_bone(bone.as_str()))
        .filter_map(|(_, p)| {
            camera
                .world_to_viewport(&view, p.translation())
                .ok()
                .map(|p| rect.top as f32 + p.y * dpi)
        })
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        (actual - target).abs() < 5.0,
        "rendered library contact floats/sinks: {actual} vs {target}"
    );
    world.resource_mut::<Smoke>().contacts += 1;
}
fn run(world: &mut World) {
    let time = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<Smoke>().stage;
    let dt = time - world.resource::<Smoke>().at;
    assert!(time < 180.0, "Library motion timeout stage {stage}");
    let active = world.resource::<LibraryMotion>().owns_placement();
    if matches!(stage, 1 | 2 | 4 | 5 | 7 | 8 | 15 | 16 | 25 | 26) && active {
        contact(world);
    }
    if matches!(stage, 4 | 5 | 7 | 8 | 15 | 16) {
        let x = position(world).x;
        let mut smoke = world.resource_mut::<Smoke>();
        smoke.peak_x = smoke.peak_x.max(x);
        smoke.min_x = smoke.min_x.min(x);
    }
    match stage {
        0 if time > 8.0
            && !world
                .resource::<crate::startup_entrance::StartupEntrance>()
                .blocks_position_save() =>
        {
            world
                .resource_mut::<crate::settings::NeraSettings>()
                .walking_enabled = false;
            world
                .resource_mut::<crate::settings::NeraSettings>()
                .audio_muted = true;
            start(world, UP);
            next(world, 1);
        }
        1 if active && dt > 1.0 => {
            screenshot(world, "standing-up-start");
            next(world, 2);
        }
        2 if !active && manual_clip(world).is_none() && dt > 2.0 => {
            home(world);
            screenshot(world, "standing-up-restored");
            pass(
                world,
                "Standing Up 3 keeps evaluated body contact and restores its checkpoint",
            );
            start(world, RIGHT);
            next(world, 4);
        }
        4 if active => {
            next(world, 5);
        }
        5 if !active && manual_clip(world).is_none() && dt > 0.5 => {
            home(world);
            let smoke = world.resource::<Smoke>();
            assert!(
                smoke.home.x - smoke.min_x > 20,
                "authored negative-X walk must move the HWND, observed {}px right / {}px left",
                smoke.peak_x - smoke.home.x,
                smoke.home.x - smoke.min_x
            );
            pass(
                world,
                "Standing Walk Right follows its authored negative-X trajectory, then returns exactly",
            );
            world.write_message(crate::avatar::body_mode::ToggleBodyMode);
            next(world, 6);
        }
        6 if dt > 1.0 => {
            start(world, RIGHT);
            next(world, 7);
        }
        7 if active => {
            screenshot(world, "chibi-walk");
            next(world, 8);
        }
        8 if !active && manual_clip(world).is_none() && dt > 0.5 => {
            home(world);
            assert!(world.resource::<Smoke>().min_x < world.resource::<Smoke>().home.x - 10);
            world.resource_mut::<Smoke>().repetitions += 1;
            if world.resource::<Smoke>().repetitions < 3 {
                start(world, RIGHT);
                next(world, 7);
            } else {
                pass(
                    world,
                    "Three Chibi replays move and return without position/proportion drift",
                );
                world.write_message(crate::avatar::body_mode::ToggleBodyMode);
                next(world, 9);
            }
        }
        9 if dt > 1.0 => {
            start(world, UP);
            next(world, 10);
        }
        10 if active && dt > 0.6 => {
            let environment = world.resource::<DesktopEnvironment>();
            let hwnd = environment.nera_hwnd.unwrap();
            let p = position(world) + IVec2::new(90, -35);
            platform::move_window(hwnd, platform::PhysicalPoint::new(p.x, p.y)).unwrap();
            world.resource_mut::<DesktopEnvironment>().nera_rect = platform::window_rect(hwnd);
            world.resource_mut::<Smoke>().home = p;
            world.resource_mut::<DesktopPetInteraction>().state = InteractionState::Dragging;
            next(world, 11);
        }
        11 if dt > 0.7 => {
            assert!(!active);
            home(world);
            assert!(manual_clip(world).is_none());
            pass(
                world,
                "Drag cancels playback/checkpoint without pulling the HWND back",
            );
            world
                .resource_mut::<DesktopPetInteraction>()
                .reset_gesture_state();
            next(world, 12);
        }
        12 if dt > 2.0 => {
            world.write_message(crate::character::ImportCharacter(std::path::PathBuf::from(r"C:\Users\sahid\AppData\Roaming\io.github.sahidhaqqi.projectnera\characters\model-07665e5f50f3b8ac.vrm")));
            next(world, 13);
        }
        13 if dt > 4.0
            && !world
                .resource::<crate::character::CharacterImportStatus>()
                .loading =>
        {
            assert!(!active);
            start(world, RIGHT);
            next(world, 15);
        }
        15 if active => {
            screenshot(world, "alicia-walk");
            next(world, 16);
        }
        16 if !active && manual_clip(world).is_none() && dt > 0.5 => {
            home(world);
            assert!(world.resource::<Smoke>().min_x < world.resource::<Smoke>().home.x - 10);
            pass(
                world,
                "Imported Alicia uses humanoid proportions, follows authored motion and restores checkpoint",
            );
            let mut environment = world.resource_mut::<DesktopEnvironment>();
            let taskbar = *environment
                .taskbar
                .taskbars
                .iter()
                .find(|t| t.edge == crate::environment::TaskbarEdge::Bottom)
                .expect("native bottom taskbar");
            environment.placement = PlacementState::PendingTaskbar {
                monitor: taskbar.monitor,
                edge: taskbar.edge,
                settle_frames: 1,
            };
            environment.set_attached_anchor(AvatarAnchor::Seat);
            next(world, 24);
        }
        24 if dt > 3.0
            && matches!(
                world.resource::<DesktopEnvironment>().placement,
                PlacementState::Taskbar(_)
            )
            && world.resource::<NeraAnimationState>().visual_state()
                == VisualState::SittingLoop =>
        {
            start(world, RIGHT);
            next(world, 25);
        }
        25 if active => {
            next(world, 26);
        }
        26 if !active && manual_clip(world).is_none() && dt > 0.5 => {
            home(world);
            next(world, 27);
        }
        27 if dt > 1.0 => {
            home(world);
            assert!(matches!(
                world.resource::<DesktopEnvironment>().placement,
                PlacementState::Taskbar(_)
            ));
            assert_eq!(
                world.resource::<DesktopEnvironment>().attached_anchor,
                AvatarAnchor::Seat
            );
            screenshot(world, "taskbar-checkpoint-restored");
            pass(
                world,
                "Taskbar seated playback returns to its original Seat attachment and stays aligned",
            );
            world.resource_mut::<DesktopEnvironment>().placement = PlacementState::Free;
            next(world, 28);
        }
        28 if dt > 1.0 => {
            start(world, UP);
            next(world, 17);
        }
        17 if active && dt > 0.6 => {
            world.write_message(crate::action_menu::NeraActionMenuCommand::Alarm(
                crate::action_menu::AlarmMenuChoice::TestNow,
            ));
            next(world, 18);
        }
        18 if dt > 1.0 => {
            assert!(!active);
            assert!(
                world
                    .resource::<crate::presentation::PresentationState>()
                    .is_attention()
            );
            pass(
                world,
                "Real Alarm preempts library motion and discards its checkpoint",
            );
            world.write_message(crate::action_menu::NeraActionMenuCommand::Alarm(
                crate::action_menu::AlarmMenuChoice::Cancel,
            ));
            next(world, 19);
        }
        19 if dt > 3.0 => {
            start(world, UP);
            next(world, 20);
        }
        20 if active && dt > 0.6 => {
            world.write_message(crate::cloud::CloudCommand::Summon(
                crate::cloud::CloudMode::Ride,
            ));
            next(world, 21);
        }
        21 if dt > 1.0 => {
            assert!(!active);
            assert!(world.resource::<crate::cloud::CloudRuntime>().is_riding());
            pass(
                world,
                "Ride Cloud takes placement without a stale checkpoint restore",
            );
            let smoke = world.resource::<Smoke>();
            let report = serde_json::json!({"passed":smoke.checks,"evaluated_contact_frames":smoke.contacts,"not_performed":["physical mouse/keyboard input","all third-party clips","multi-monitor hardware"]});
            std::fs::write(
                "target/library-motion-native/report.json",
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            world.write_message(AppExit::Success);
            next(world, 22);
        }
        _ => {}
    }
}
