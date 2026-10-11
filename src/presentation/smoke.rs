//! Opt-in native renderer/HWND validation using production command channels.
use super::*;
use crate::{
    action_menu::{AlarmMenuChoice, NeraActionMenuCommand},
    avatar::body_mode::{AvatarBodyMode, ToggleBodyMode},
    tray::{TrayCommand, send_action_menu_result, send_tray_command},
};
use bevy::render::view::screenshot::ScreenshotCaptured;
use bevy::{
    input::{
        ButtonState,
        mouse::{MouseButtonInput, MouseScrollUnit, MouseWheel},
        touch::TouchPhase,
    },
    window::CursorMoved,
};

#[derive(Resource, Default)]
struct NativeSmoke {
    stage: u8,
    at: f32,
    original: Option<PresentationSnapshot>,
    captures: Vec<serde_json::Value>,
    checks: Vec<String>,
    capture_label: String,
    variant: usize,
    variant_started: bool,
    saw_greeting: bool,
    saw_firefly: bool,
    swat_started: f32,
    swat_capture: usize,
    utility_requested: bool,
    settings_checked: bool,
    alarm_editor_checked: bool,
}

pub(crate) fn install(app: &mut App) {
    // A failed run must never leave a previous success report looking current.
    let _ = std::fs::remove_file(output().join("report.json"));
    app.init_resource::<NativeSmoke>()
        .add_observer(observe_capture)
        .add_systems(
            Last,
            (activity::exit_on_avatar_click, run)
                .chain()
                .after(maintain_attention_presentation),
        );
}

fn output() -> std::path::PathBuf {
    std::path::PathBuf::from("target/presentation-native")
}

fn observe_capture(
    event: On<ScreenshotCaptured>,
    mut smoke: ResMut<NativeSmoke>,
    environment: Res<DesktopEnvironment>,
) {
    let Ok(image) = event.image.clone().try_into_dynamic() else {
        return;
    };
    let image = image.to_rgba8();
    let (width, height) = image.dimensions();
    let mut min = UVec2::new(width, height);
    let mut max = UVec2::ZERO;
    let mut transparent = 0_u64;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel[3] == 0 {
            transparent += 1;
        }
        // Measure visible character pixels, excluding the black alpha-only
        // compositor layer. The saved RGBA render target stays untouched.
        if pixel[3] > 20 && pixel.0[..3].iter().any(|channel| *channel > 2) {
            min = min.min(UVec2::new(x, y));
            max = max.max(UVec2::new(x, y));
        }
    }
    let occupancy = environment.monitor.map_or(0.0, |monitor| {
        (max.y.saturating_sub(min.y) + 1) as f32 / monitor.bounds.height() as f32
    });
    let _ = std::fs::create_dir_all(output());
    let path = output().join(format!("{}.png", smoke.capture_label));
    image.save(&path).expect("native diagnostic PNG");
    let label = smoke.capture_label.clone();
    smoke.captures.push(serde_json::json!({"label":label,"width":width,"height":height,"alpha_zero_pixels":transparent,"bounds":[min.x,min.y,max.x,max.y],"visible_monitor_height":occupancy,"path":path}));
    info!(
        "PRESENTATION_NATIVE capture={} occupancy={:.3} alpha_zero={} bounds={:?}..{:?}",
        label, occupancy, transparent, min, max
    );
}

fn capture(world: &mut World, label: &str) {
    world.resource_mut::<NativeSmoke>().capture_label = label.into();
    send_tray_command(TrayCommand::Screenshot);
}

fn enter() {
    send_action_menu_result(Some(NeraActionMenuCommand::Screensaver));
}

fn script_non_click_input(world: &mut World) {
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    world.write_message(CursorMoved {
        window,
        position: Vec2::splat(900.0),
        delta: Some(Vec2::splat(500.0)),
    });
    world.write_message(MouseWheel {
        window,
        unit: MouseScrollUnit::Line,
        x: 0.0,
        y: -10.0,
        phase: TouchPhase::Moved,
    });
    world
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Space);
}

fn script_avatar_click(world: &mut World) {
    // Probe the real evaluated rig and production colliders without moving the
    // desktop cursor. Only Bevy window events/semantic hover are injected.
    let head = world
        .query_filtered::<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>()
        .iter(world)
        .find(|(bone, _)| bone.as_str() == "head")
        .unwrap()
        .1
        .translation();
    let (camera, view) = world
        .query_filtered::<(&Camera, &GlobalTransform), With<crate::avatar::NeraMainCamera>>()
        .single(world)
        .unwrap();
    let cursor = camera.world_to_viewport(view, head).unwrap();
    let part = crate::interaction::smoke_parts_at_cursor(world, vec![cursor])[0];
    // The head landmark sits at the neck on some rigs/Chibi poses, where the
    // production hit route can correctly resolve Torso instead of Head. Any
    // actual semantic body hit is sufficient for click-on-Nera dismissal.
    assert!(part.is_some(), "scripted click missed the evaluated avatar");
    world.resource_mut::<HoveredBodyPart>().part = part;
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    world.write_message(MouseButtonInput {
        window,
        button: MouseButton::Left,
        state: ButtonState::Pressed,
    });
}

fn script_headpat(world: &mut World) {
    let eyes: Vec<_> = world
        .query_filtered::<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>()
        .iter(world)
        .filter(|(bone, _)| matches!(bone.as_str(), "leftEye" | "rightEye"))
        .map(|(_, pose)| pose.translation())
        .collect();
    assert!(!eyes.is_empty());
    let eye = eyes.iter().copied().sum::<Vec3>() / eyes.len() as f32;
    let (camera, view) = world
        .query_filtered::<(&Camera, &GlobalTransform), With<crate::avatar::NeraMainCamera>>()
        .single(world)
        .unwrap();
    let cursor = camera.world_to_viewport(view, eye).unwrap();
    assert_eq!(
        crate::interaction::smoke_parts_at_cursor(world, vec![cursor])[0],
        Some(crate::interaction::BodyPart::Head)
    );
    world.write_message(crate::interaction::NeraHeadPatted {
        pointer: crate::interaction::NeraPointerPosition {
            screen: IVec2::ZERO,
            local: cursor,
        },
    });
}

fn check_screensaver_pose_ownership(world: &mut World) {
    crate::settings::assert_smoke_presentation_shadow_hidden(world);
    use bevy_vrm1::{prelude::LookAt, vrm::body_tracking::BodyTracking};
    let (look, tracking) = world.query_filtered::<(Option<&LookAt>, Option<&BodyTracking>), With<crate::avatar::NeraAvatar>>()
        .single(world).unwrap();
    assert!(
        look.is_some() && tracking.is_none(),
        "Screensaver needs viewer eye gaze without head/spine cursor tracking"
    );
    assert!(
        world.query::<&AnimationPlayer>().iter(world).any(|player| {
            player
                .playing_animations()
                .any(|(_, active)| !active.is_paused())
        }),
        "Screensaver must retain authored animation playback"
    );
    let landmarks: Vec<_> = world
        .query_filtered::<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>()
        .iter(world)
        .filter(|(bone, _)| matches!(bone.as_str(), "head" | "leftFoot" | "rightFoot"))
        .map(|(bone, transform)| (bone.as_str().to_string(), transform.translation()))
        .collect();
    let (camera, view) = world
        .query_filtered::<(&Camera, &GlobalTransform), With<crate::avatar::NeraMainCamera>>()
        .single(world)
        .unwrap();
    let size = camera.logical_viewport_size().unwrap();
    for (bone, position) in landmarks {
        let pixel = camera.world_to_viewport(view, position).unwrap();
        if bone == "head" {
            assert!(
                pixel.x > 0.0 && pixel.x < size.x && pixel.y > 0.0 && pixel.y < size.y,
                "close-up head must remain visible: {pixel:?} in {size:?}"
            );
        } else {
            assert!(
                pixel.y > size.y,
                "close-up must crop legs: {bone} at {pixel:?} in {size:?}"
            );
        }
    }
}

fn check_restore(world: &mut World) {
    assert!(world.resource::<PresentationState>().is_desktop_pet());
    assert!(
        !world
            .resource::<PresentationState>()
            .desktop_restore_pending
    );
    let original = world.resource::<NativeSmoke>().original.clone().unwrap();
    let (projection, camera) = world
        .query_filtered::<(&Projection, &Transform), With<crate::avatar::NeraMainCamera>>()
        .single(world)
        .unwrap();
    assert_eq!(*camera, original.camera_transform);
    assert_eq!(
        format!("{projection:?}"),
        format!("{:?}", original.camera_projection)
    );
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    assert_eq!(window.resolution.physical_width(), original.window_width);
    assert_eq!(window.resolution.physical_height(), original.window_height);
    assert_eq!(
        window.position,
        WindowPosition::At(IVec2::new(original.window_left, original.window_top))
    );
    assert_eq!(window.window_level, original.window_level);
    assert_eq!(
        world.resource::<DesktopEnvironment>().placement,
        original.placement
    );
    world
        .resource_mut::<NativeSmoke>()
        .checks
        .push("exact desktop window/camera/placement restoration".into());
}

fn run(world: &mut World) {
    let now = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<NativeSmoke>().stage;
    let dt = now - world.resource::<NativeSmoke>().at;
    if now > 180.0 {
        panic!("presentation native smoke timed out at stage {stage}");
    }
    let next = match stage {
        0 if now > 7.0 => {
            if !world.resource::<NativeSmoke>().utility_requested {
                world.write_message(crate::settings::OpenSettingsWindow);
                world.write_message(crate::alarm_settings::OpenAlarmSettingsWindow);
                world.resource_mut::<NativeSmoke>().utility_requested = true;
                return;
            }
            if !world.resource::<NativeSmoke>().settings_checked {
                let checked = crate::settings::smoke_pointer_routing(world);
                world.resource_mut::<NativeSmoke>().settings_checked = checked;
            }
            if !world.resource::<NativeSmoke>().alarm_editor_checked {
                let checked = crate::alarm_settings::smoke_pointer_routing(world);
                world.resource_mut::<NativeSmoke>().alarm_editor_checked = checked;
            }
            let smoke = world.resource::<NativeSmoke>();
            if !smoke.settings_checked || !smoke.alarm_editor_checked {
                return;
            }
            world.resource_mut::<NativeSmoke>().checks.push("real Settings/Alarm editor HWNDs: scripted pointer gate allows pet input outside panel and blocks panel hover".into());
            capture(world, "desktop-normal");
            true
        }
        1 if dt > 1.2 => {
            enter();
            true
        }
        2 if dt > 0.20 => {
            assert!(
                world.resource::<PresentationState>().is_screensaver(),
                "Screensaver did not remain active (native input may have dismissed it)"
            );
            let original = world.resource::<PresentationState>().snapshot.clone();
            world.resource_mut::<NativeSmoke>().original = original;
            check_screensaver_pose_ownership(world);
            capture(world, "screensaver-normal");
            script_non_click_input(world);
            true
        }
        3 if dt > 0.45 => {
            assert!(world.resource::<PresentationState>().is_screensaver());
            check_screensaver_pose_ownership(world);
            script_avatar_click(world);
            world.resource_mut::<NativeSmoke>().checks.push("Normal: scroll/cursor/keyboard ignored, viewer eye gaze active, authored head pose active, scripted body click exits".into());
            true
        }
        4 if dt > 0.8 => {
            check_restore(world);
            enter();
            true
        }
        5 if dt > 0.20 => {
            assert!(world.resource::<PresentationState>().is_screensaver());
            send_action_menu_result(Some(NeraActionMenuCommand::Alarm(AlarmMenuChoice::TestNow)));
            true
        }
        6 if dt > 1.2 => {
            assert!(world.resource::<crate::alarm::AlarmState>().is_ringing());
            assert_eq!(
                world.resource::<PresentationState>().mode,
                PresentationMode::Attention(AttentionKind::Alarm)
            );
            let original = world.resource::<NativeSmoke>().original.as_ref().unwrap();
            assert_eq!(
                world
                    .resource::<PresentationState>()
                    .snapshot
                    .as_ref()
                    .unwrap()
                    .camera_transform,
                original.camera_transform
            );
            capture(world, "alarm-preemption");
            world
                .resource_mut::<NativeSmoke>()
                .checks
                .push("real TestNow Alarm preemption retains original desktop snapshot".into());
            true
        }
        7 if dt > 1.0 => {
            send_action_menu_result(Some(NeraActionMenuCommand::Alarm(AlarmMenuChoice::Cancel)));
            true
        }
        8 if dt > 1.0 => {
            check_restore(world);
            world.write_message(ToggleBodyMode);
            true
        }
        9 if dt > 2.5 => {
            assert_eq!(*world.resource::<AvatarBodyMode>(), AvatarBodyMode::Chibi);
            enter();
            true
        }
        10 if dt > 0.20 => {
            assert!(world.resource::<PresentationState>().is_screensaver());
            check_screensaver_pose_ownership(world);
            capture(world, "screensaver-chibi");
            script_non_click_input(world);
            script_headpat(world);
            true
        }
        11 if dt > 0.45 => {
            assert!(world.resource::<PresentationState>().is_screensaver());
            check_screensaver_pose_ownership(world);
            assert_eq!(
                world
                    .resource::<crate::animation::NeraAnimationState>()
                    .visual_state(),
                crate::animation::VisualState::HeadStroke
            );
            script_avatar_click(world);
            world.resource_mut::<NativeSmoke>().checks.push("Chibi: headpat accepted without exit, viewer eye gaze active, scroll ignored, scripted body click exits".into());
            true
        }
        12 if dt > 0.8 => {
            check_restore(world);
            assert_eq!(*world.resource::<AvatarBodyMode>(), AvatarBodyMode::Chibi);
            world.write_message(ToggleBodyMode);
            true
        }
        13 if dt > 1.0 => {
            enter();
            true
        }
        14 => {
            use crate::animation::screensaver::{ScreensaverClip as Clip, smoke_playing};
            if smoke_playing(world, Clip::Waving) {
                world.resource_mut::<NativeSmoke>().saw_greeting = true;
            }
            if smoke_playing(world, Clip::Breathing) {
                assert!(
                    world.resource::<NativeSmoke>().saw_greeting,
                    "entry Waving never actually played"
                );
                capture(world, "routine-breathing");
                script_headpat(world);
                true
            } else {
                false
            }
        }
        15 if dt > 0.4 => {
            assert!(world.resource::<PresentationState>().is_screensaver());
            assert_eq!(
                world
                    .resource::<crate::animation::NeraAnimationState>()
                    .visual_state(),
                crate::animation::VisualState::HeadStroke
            );
            assert!(
                world
                    .resource::<crate::animation::NeraAnimationClips>()
                    .head_stroke_playing(
                        world.resource::<crate::animation::NeraAnimationPlayback>()
                    )
            );
            check_screensaver_pose_ownership(world);
            capture(world, "routine-headpat");
            true
        }
        16 => {
            use crate::animation::screensaver::{
                ScreensaverClip as Clip, smoke_playing, smoke_select,
            };
            if smoke_playing(world, Clip::Breathing) {
                smoke_select(world, Clip::Holding);
                true
            } else {
                false
            }
        }
        17 => {
            use crate::animation::screensaver::{
                ScreensaverClip as Clip, smoke_playing, smoke_select,
            };
            let variants = [
                Clip::Holding,
                Clip::Dwarf4,
                Clip::Briefcase2,
                Clip::Dwarf,
                Clip::Dwarf3,
                Clip::Offensive,
                Clip::Dwarf2,
            ];
            let index = world.resource::<NativeSmoke>().variant;
            let clip = variants[index];
            if clip == Clip::Dwarf2
                && !world.resource::<NativeSmoke>().saw_firefly
                && world
                    .query::<&crate::pet_feedback::PetParticle>()
                    .iter(world)
                    .any(|particle| particle.is_firefly())
            {
                world.resource_mut::<NativeSmoke>().saw_firefly = true;
                capture(world, "routine-firefly");
            }
            if smoke_playing(world, clip) && !world.resource::<NativeSmoke>().variant_started {
                world.resource_mut::<NativeSmoke>().variant_started = true;
                world.resource_mut::<NativeSmoke>().swat_started = now;
                world
                    .resource_mut::<NativeSmoke>()
                    .checks
                    .push(format!("actual VRMA played: {}", clip.path()));
                info!("PRESENTATION_NATIVE actual routine clip: {}", clip.path());
            }
            if clip == Clip::Dwarf2 && smoke_playing(world, clip) {
                let smoke = world.resource::<NativeSmoke>();
                let frame = smoke.swat_capture;
                if frame < 4 && now - smoke.swat_started >= 0.25 + frame as f32 * 0.4 {
                    world.resource_mut::<NativeSmoke>().swat_capture += 1;
                    capture(world, &format!("dwarf2-pose-{frame}"));
                }
            }
            if world.resource::<NativeSmoke>().variant_started
                && smoke_playing(world, Clip::Breathing)
            {
                if index + 1 < variants.len() {
                    world.resource_mut::<NativeSmoke>().variant += 1;
                    world.resource_mut::<NativeSmoke>().variant_started = false;
                    smoke_select(world, variants[index + 1]);
                    false
                } else {
                    assert!(world.resource::<NativeSmoke>().saw_firefly);
                    world.resource_mut::<NativeSmoke>().checks.push("Waving -> looping Breathing; Normal/Chibi headpat; seven real variations -> Breathing; real firefly precedes Dwarf Idle 2".into());
                    script_avatar_click(world);
                    true
                }
            } else {
                false
            }
        }
        18 if dt > 0.8 => {
            check_restore(world);
            true
        }
        19 if dt > 0.5 => {
            let smoke = world.resource::<NativeSmoke>();
            assert_eq!(smoke.captures.len(), 11);
            for capture in &smoke.captures {
                assert!(capture["alpha_zero_pixels"].as_u64().unwrap() > 0);
                if capture["label"]
                    .as_str()
                    .unwrap()
                    .starts_with("screensaver-")
                {
                    let visible = capture["visible_monitor_height"].as_f64().unwrap();
                    assert!(
                        (0.85..=0.95).contains(&visible),
                        "visible occupancy {visible}"
                    );
                    let bounds = capture["bounds"].as_array().unwrap();
                    assert!(
                        bounds[1].as_u64().unwrap() > 0,
                        "crown must not clip at top"
                    );
                    assert!(
                        bounds[3].as_u64().unwrap() + 2 >= capture["height"].as_u64().unwrap(),
                        "lower body must intentionally continue below the portrait viewport"
                    );
                }
            }
            let report = serde_json::json!({"status":"PASSED","framing":"upper-body close-up; head visible, legs cropped","checks":smoke.checks,"captures":smoke.captures,"pictures":screenshot::pictures_folder(),"physical_radial_tray_clicks":"NOT PERFORMED","physical_body_click":"NOT PERFORMED","physical_headpat":"NOT PERFORMED","scripted_headpat":"PASSED","scripted_body_click":"PASSED","variation_selection":"scripted acceleration; production 30-60 second intervals covered by unit tests","physical_pointer_capture_suppressed_for_harness":true});
            std::fs::write(
                output().join("report.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            info!("PRESENTATION_NATIVE PASSED: {:?}", smoke.checks);
            world.write_message(bevy::app::AppExit::Success);
            false
        }
        _ => false,
    };
    if next {
        let mut smoke = world.resource_mut::<NativeSmoke>();
        smoke.stage += 1;
        smoke.at = now;
    }
}
