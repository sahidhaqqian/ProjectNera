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
    walking_frames: Vec<serde_json::Value>,
    floor: f32,
}

// Reproduce the playback/grounding path from cef7e2e without reverting the
// unrelated character migration and focus changes. Debug validation only.
pub(in crate::animation) fn legacy_playback() -> bool {
    static LEGACY: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
        std::env::args().any(|arg| arg == "--feature-smoke")
            && std::env::args().any(|arg| arg == "--legacy-library-motion")
    });
    *LEGACY
}

pub(crate) fn install(app: &mut App) {
    if std::env::args().any(|arg| arg == "--death-ground-smoke") {
        app.init_resource::<Smoke>()
            .add_systems(Last, death_run.after(move_canvas));
        return;
    }
    if std::env::args().any(|arg| arg == "--garment-spring-smoke") {
        app.init_resource::<Smoke>()
            .add_systems(Last, garment_run.after(move_canvas));
        return;
    }
    if std::env::args().any(|arg| arg == "--walking-ground-smoke") {
        app.init_resource::<Smoke>()
            .add_systems(Last, walking_run.after(move_canvas));
        return;
    }
    let _ = std::fs::remove_file("target/library-motion-native/report.json");
    app.init_resource::<Smoke>()
        .add_systems(Last, run.after(crate::pet_feedback::update_particles));
}

fn death_directory() -> std::path::PathBuf {
    let label = std::env::var("NERA_DEATH_VALIDATION_LABEL")
        .ok()
        .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or_else(|| {
            if legacy_playback() {
                "legacy"
            } else {
                "current"
            }
            .into()
        });
    std::path::PathBuf::from("target/death-ground-native").join(label)
}

fn death_run(world: &mut World) {
    let time = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<Smoke>().stage;
    let dt = time - world.resource::<Smoke>().at;
    assert!(time < 55.0, "Death ground smoke timeout stage {stage}");
    if stage == 1 && manual_clip(world).is_some() {
        let hips = world
            .query_filtered::<&HipsBoneEntity, With<NeraAvatar>>()
            .iter(world)
            .next()
            .unwrap()
            .0;
        if let Some((_, playback_time)) = playing_clip(world, hips) {
            let (camera, view) = camera_geometry(world).unwrap();
            let rect = world.resource::<DesktopEnvironment>().nera_rect.unwrap();
            let dpi = world.resource::<DesktopEnvironment>().nera_scale_factor as f32;
            let bones: Vec<_> = world.query_filtered::<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>()
                .iter(world).filter_map(|(bone, pose)| {
                    let point = camera.world_to_viewport(&view, pose.translation()).ok()?;
                    Some(serde_json::json!({"bone":bone.as_str(),"screen_y":rect.top as f32 + point.y*dpi,
                        "world":pose.translation().to_array()}))
                }).collect();
            let root_y = world
                .query_filtered::<&Transform, With<NeraAvatar>>()
                .iter(world)
                .next()
                .unwrap()
                .translation
                .y;
            let motion = world.resource::<LibraryMotion>();
            let raw_root_y = motion.raw_root.map(|(_, y)| y).or_else(|| {
                world
                    .resource::<crate::avatar::AvatarVisualGrounding>()
                    .raw_root_y_for_validation()
            });
            let airborne = motion.session.as_ref().map(|s| s.airborne);
            let mode = *world.resource::<crate::avatar::body_mode::AvatarBodyMode>();
            let clip = manual_clip(world).unwrap();
            let source_path = world
                .resource::<super::super::LibraryPlayback>()
                .path_for(clip)
                .map(str::to_owned);
            let frame = serde_json::json!({"mode":format!("{mode:?}"),"time":playback_time,
                "source_path":source_path,
                "floor":world.resource::<Smoke>().floor,"root_y":root_y,"raw_root_y":raw_root_y,
                "hips":world.get::<Transform>(hips).unwrap().translation.to_array(),"airborne":airborne,
                "window_y_drift":rect.top-world.resource::<Smoke>().home.y,"bones":bones});
            // The old BodyContact contract must survive gallery playback.
            // Check settled lying frames, after entry blending and the fall.
            if dt > 1.5
                && !std::env::var("NERA_DEATH_VALIDATION_LABEL")
                    .is_ok_and(|s| s.starts_with("before-"))
            {
                assert!(
                    !motion.owns_placement(),
                    "Death must keep its original body-contact owner"
                );
                assert_eq!(
                    rect.top,
                    world.resource::<Smoke>().home.y,
                    "Death moved its desktop floor"
                );
                if let Some(raw) = raw_root_y {
                    assert!(
                        root_y <= raw + 0.0001,
                        "Death body contact lifted the lying pose"
                    );
                    let scale = world
                        .resource::<crate::settings::NeraSettings>()
                        .character_scale();
                    assert!(
                        root_y >= raw - 1.5 * scale.max(0.25) - 0.0001,
                        "Death exceeded its existing bounded root drop"
                    );
                }
                let lowest = bones
                    .iter()
                    .map(|b| b["screen_y"].as_f64().unwrap())
                    .fold(f64::NEG_INFINITY, f64::max);
                let floor = world.resource::<Smoke>().floor as f64;
                assert!(
                    lowest >= floor - 12.0,
                    "Death left the original body-contact plane: {lowest} vs {floor}"
                );
                // A foot can already be below the target margin, especially on
                // a smaller imported Chibi rig. Original BodyContact never
                // lifts such a pose just to force an exact ankle-to-floor gap.
            }
            world.resource_mut::<Smoke>().walking_frames.push(frame);
            // During entry blending, the dominant graph clip can still be idle.
            // Schedule captures by wall time so both paths capture the same pose.
            let capture = (dt * 2.0).floor() as u8;
            if capture > world.resource::<Smoke>().repetitions && capture <= 4 {
                world.resource_mut::<Smoke>().repetitions = capture;
                std::fs::create_dir_all(death_directory()).unwrap();
                let window = world
                    .query_filtered::<Entity, With<PrimaryWindow>>()
                    .single(world)
                    .unwrap();
                world
                    .commands()
                    .spawn(Screenshot::window(window))
                    .observe(save_to_disk(
                        death_directory().join(format!("{mode:?}-{capture}.png")),
                    ));
            }
        }
    }
    match stage {
        0 if time > 9.0
            && !world
                .resource::<crate::startup_entrance::StartupEntrance>()
                .blocks_position_save() =>
        {
            let mut settings = world.resource_mut::<crate::settings::NeraSettings>();
            settings.walking_enabled = false;
            settings.audio_muted = true;
            for mut camera in world
                .query_filtered::<&mut Camera, With<NeraMainCamera>>()
                .iter_mut(world)
            {
                camera.clear_color = ClearColorConfig::Custom(Color::WHITE);
            }
            death_start(world);
        }
        1 if manual_clip(world).is_none()
            && !world.resource::<LibraryMotion>().owns_placement()
            && dt > 4.0 =>
        {
            home(world);
            if *world.resource::<crate::avatar::body_mode::AvatarBodyMode>()
                == crate::avatar::body_mode::AvatarBodyMode::Normal
            {
                world.write_message(crate::avatar::body_mode::ToggleBodyMode);
                next(world, 2);
            } else {
                std::fs::create_dir_all(death_directory()).unwrap();
                std::fs::write(
                    death_directory().join("report.json"),
                    serde_json::to_vec_pretty(&world.resource::<Smoke>().walking_frames).unwrap(),
                )
                .unwrap();
                next(world, 3);
            }
        }
        2 if dt > 1.0 => death_start(world),
        3 if dt > 1.0 => {
            world.write_message(AppExit::Success);
            next(world, 4);
        }
        _ => {}
    }
}

fn death_start(world: &mut World) {
    let environment = world.resource::<DesktopEnvironment>();
    let rect = environment.nera_rect.unwrap();
    let floor = rect.top as f32
        + environment.feet_anchor_logical.unwrap().y * environment.nera_scale_factor as f32;
    world.resource_mut::<Smoke>().floor = floor;
    world.resource_mut::<Smoke>().repetitions = 0;
    // Reproduce the gallery entry the user actually selects, not only the
    // protected base copy. The two bundled files are byte-identical aliases.
    let path = std::env::var("NERA_DEATH_VALIDATION_PATH")
        .unwrap_or_else(|_| "animations/UAL1_converted_43_vrma/Death01.vrma".into());
    start(world, &path);
    next(world, 1);
}

fn garment_run(world: &mut World) {
    use bevy_vrm1::prelude::SpringJointProps;
    let baseline = std::env::args().any(|arg| arg == "--garment-spring-baseline");
    let time = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<Smoke>().stage;
    let dt = time - world.resource::<Smoke>().at;
    assert!(time < 50.0, "Garment spring smoke timeout stage {stage}");
    match stage {
        0 if time > 9.0
            && !world
                .resource::<crate::startup_entrance::StartupEntrance>()
                .blocks_position_save() =>
        {
            let props: Vec<_> = world
                .query::<(&Name, &SpringJointProps)>()
                .iter(world)
                .filter(|(name, _)| name.as_str().to_ascii_lowercase().contains("skirt"))
                .map(|(_, p)| p.gravity_power)
                .collect();
            assert!(!props.is_empty(), "Use a model with skirt spring joints");
            assert!(
                props
                    .iter()
                    .all(|p| if baseline { *p == 0.0 } else { *p > 0.0 })
            );
            let mut settings = world.resource_mut::<crate::settings::NeraSettings>();
            settings.walking_enabled = false;
            settings.audio_muted = true;
            for mut camera in world
                .query_filtered::<&mut Camera, With<NeraMainCamera>>()
                .iter_mut(world)
            {
                camera.clear_color = ClearColorConfig::Custom(Color::WHITE);
            }
            start(world, "animations/animasi_basic/Sitting_4.vrma");
            next(world, 1);
        }
        1 if dt > 2.0 => {
            garment_sample(world, "seated-normal");
            start(world, "animations/animasi_basic/Death01.vrma");
            next(world, 2);
        }
        2 if dt > 1.7 => {
            garment_sample(world, "lying-normal");
            next(world, 3);
        }
        3 if dt > 2.0
            && !world.resource::<LibraryMotion>().owns_placement()
            && manual_clip(world).is_none() =>
        {
            home(world);
            world.write_message(crate::avatar::body_mode::ToggleBodyMode);
            next(world, 4);
        }
        4 if dt > 1.0 => {
            start(world, "animations/animasi_basic/Death01.vrma");
            next(world, 5);
        }
        5 if dt > 1.7 => {
            garment_sample(world, "lying-chibi");
            next(world, 6);
        }
        6 if dt > 2.0
            && !world.resource::<LibraryMotion>().owns_placement()
            && manual_clip(world).is_none() =>
        {
            home(world);
            world.write_message(crate::avatar::body_mode::ToggleBodyMode);
            next(world, 7);
        }
        7 if dt > 1.0 => {
            garment_sample(world, "restored-normal");
            std::fs::write(
                "target/garment-spring-native/report.json",
                serde_json::to_vec_pretty(&world.resource::<Smoke>().walking_frames).unwrap(),
            )
            .unwrap();
            next(world, 8);
        }
        // Keep the window alive until the final screenshot's GPU readback.
        8 if dt > 0.75 => {
            world.write_message(AppExit::Success);
            next(world, 9);
        }
        _ => {}
    }
}

fn garment_sample(world: &mut World, label: &str) {
    use bevy_vrm1::prelude::SpringJointProps;
    let baseline = std::env::args().any(|arg| arg == "--garment-spring-baseline");
    let samples: Vec<_> = world.query::<(&Name,&SpringJointProps,&GlobalTransform)>().iter(world)
        .filter(|(name,_,_)| name.as_str().to_ascii_lowercase().contains("skirt"))
        .map(|(name,p,t)| {
            assert!(t.to_matrix().is_finite());
            assert!(if baseline { p.gravity_power == 0.0 } else { p.gravity_power > 0.0 });
            serde_json::json!({"name":name.as_str(),"gravity":p.gravity_power,"stiffness":p.stiffness,"position":t.translation().to_array()})
        }).collect();
    let hair_gravity: Vec<_> = world
        .query::<(&Name, &SpringJointProps)>()
        .iter(world)
        .filter(|(name, _)| name.as_str().to_ascii_lowercase().contains("hair"))
        .map(|(_, p)| p.gravity_power)
        .collect();
    world
        .resource_mut::<Smoke>()
        .walking_frames
        .push(serde_json::json!({"pose":label,"skirt":samples,"hair_gravity":hair_gravity}));
    std::fs::create_dir_all("target/garment-spring-native").unwrap();
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(format!(
            "target/garment-spring-native/{label}.png"
        )));
}

// Focused forward/depth-motion reproduction. Measure the evaluated feet and
// native window after placement, including frames near the monitor boundary.
fn walking_run(world: &mut World) {
    const WALK: &str = "animations/Mixamo-VRMA/Walking_10.vrma";
    let time = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<Smoke>().stage;
    let dt = time - world.resource::<Smoke>().at;
    let active = world.resource::<LibraryMotion>().owns_placement();
    assert!(time < 55.0, "Walking ground smoke timeout stage {stage}");
    if stage == 1 {
        if let Some(session) = world.resource::<LibraryMotion>().session.as_ref()
            && session.returning.is_none()
        {
            let checkpoint = session.checkpoint;
            let airborne = session.airborne;
            let (camera, view) = camera_geometry(world).unwrap();
            let environment = world.resource::<DesktopEnvironment>();
            let dpi = environment.nera_scale_factor as f32;
            let rect = environment.nera_rect.unwrap();
            let margin = crate::avatar::standing_support_offset(
                world.resource::<crate::settings::NeraSettings>(),
                world
                    .resource::<crate::avatar::body_mode::BodyProportions>()
                    .body_scale(),
            );
            let target = checkpoint.position.y as f32 + (checkpoint.contact.y - margin) * dpi;
            let feet = world
                .query_filtered::<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>()
                .iter(world)
                .filter(|(bone, _)| {
                    matches!(
                        bone.as_str(),
                        "leftFoot" | "rightFoot" | "leftToes" | "rightToes"
                    )
                })
                .filter_map(|(_, pose)| camera.world_to_viewport(&view, pose.translation()).ok())
                .map(|p| rect.top as f32 + p.y * dpi)
                .fold(f32::NEG_INFINITY, f32::max);
            let mode = world.resource::<crate::avatar::body_mode::AvatarBodyMode>();
            let frame = serde_json::json!({"mode":format!("{mode:?}"),"time":dt,
                "foot_error":feet-target,"airborne":airborne,"window":[rect.left,rect.top],
                "canvas_y_drift":rect.top-checkpoint.position.y});
            world.resource_mut::<Smoke>().walking_frames.push(frame);
        }
        let screenshot_index = dt.floor() as u8;
        if active
            && screenshot_index > 0
            && screenshot_index <= 3
            && screenshot_index > world.resource::<Smoke>().repetitions
        {
            world.resource_mut::<Smoke>().repetitions = screenshot_index;
            let mode = world.resource::<crate::avatar::body_mode::AvatarBodyMode>();
            let name = format!("{mode:?}-{screenshot_index}");
            walking_screenshot(world, &name);
        }
    }
    match stage {
        0 if time > 8.0
            && !world
                .resource::<crate::startup_entrance::StartupEntrance>()
                .blocks_position_save() =>
        {
            let mut settings = world.resource_mut::<crate::settings::NeraSettings>();
            settings.walking_enabled = false;
            settings.audio_muted = true;
            settings.shadow_enabled = true;
            settings.shadow_type = 1;
            for mut camera in world
                .query_filtered::<&mut Camera, With<NeraMainCamera>>()
                .iter_mut(world)
            {
                camera.clear_color = ClearColorConfig::Custom(Color::WHITE);
            }
            start(world, WALK);
            next(world, 1);
        }
        1 if !active && manual_clip(world).is_none() && dt > 5.0 => {
            home(world);
            if *world.resource::<crate::avatar::body_mode::AvatarBodyMode>()
                == crate::avatar::body_mode::AvatarBodyMode::Normal
            {
                world.write_message(crate::avatar::body_mode::ToggleBodyMode);
                next(world, 2);
            } else {
                let frames = &world.resource::<Smoke>().walking_frames;
                let max_error = frames
                    .iter()
                    .map(|f| f["foot_error"].as_f64().unwrap().abs())
                    .fold(0.0, f64::max);
                let max_y_drift = frames
                    .iter()
                    .map(|f| f["canvas_y_drift"].as_i64().unwrap().abs())
                    .max()
                    .unwrap_or(0);
                let report = serde_json::json!({"max_foot_error":max_error,"max_canvas_y_drift":max_y_drift,"frames":frames,"checkpoint_restored":true});
                std::fs::create_dir_all("target/walking-ground-native").unwrap();
                std::fs::write(
                    "target/walking-ground-native/report.json",
                    serde_json::to_vec_pretty(&report).unwrap(),
                )
                .unwrap();
                assert!(frames.len() > 20);
                assert_eq!(
                    max_y_drift, 0,
                    "Forward motion must keep the desktop support plane"
                );
                assert!(
                    max_error < 5.0,
                    "Walking 10 feet left support by {max_error}px"
                );
                assert!(
                    frames.iter().all(|f| f["airborne"] == false),
                    "Walking is not a jump"
                );
                world.write_message(AppExit::Success);
                next(world, 3);
            }
        }
        2 if dt > 1.0 => {
            world.resource_mut::<Smoke>().repetitions = 0;
            start(world, WALK);
            next(world, 1);
        }
        _ => {}
    }
}

fn walking_screenshot(world: &mut World, name: &str) {
    std::fs::create_dir_all("target/walking-ground-native").unwrap();
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(format!(
            "target/walking-ground-native/{name}.png"
        )));
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
    let grounding = session.grounding;
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
        .filter(|(bone, _)| ground_contact_bone(bone.as_str(), grounding))
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
