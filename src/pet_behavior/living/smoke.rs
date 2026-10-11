use super::*;
use bevy::{
    ecs::system::RunSystemOnce,
    render::view::screenshot::{Screenshot, save_to_disk},
};

#[derive(Resource, Default)]
struct Smoke {
    stage: u8,
    at: f32,
    home: i32,
    hand: Option<Entity>,
    original: Quat,
    changed: bool,
    saved_position: IVec2,
}
pub(crate) fn install(app: &mut App) {
    app.world_mut().resource_mut::<LivingPointer>().scripted = Some(None);
    let _ = std::fs::remove_file(output().join("report.json"));
    app.init_resource::<Smoke>().add_systems(Last, run);
}
fn output() -> std::path::PathBuf {
    "target/living-native".into()
}
fn advance(world: &mut World, stage: u8) {
    let at = world.resource::<Time<Real>>().elapsed_secs();
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.stage = stage;
    smoke.at = at;
    info!("LIVING_NATIVE stage={stage}");
}
fn capture(world: &mut World, file: &str) {
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(output().join(file)));
}
fn aim(world: &mut World) {
    let (arm, target) = world
        .run_system_once(|context: LivingContext, env: Res<DesktopEnvironment>| {
            let arm = context
                .arms(&env)
                .into_iter()
                .next()
                .expect("native humanoid needs a usable arm");
            (
                arm,
                arm.shoulder + Vec2::new(arm.reach * 0.4, -arm.reach * 0.25),
            )
        })
        .unwrap();
    world.resource_mut::<LivingPointer>().scripted = Some(Some(target));
    world.resource_mut::<NeraSettings>().cursor_play = true;
    world.resource_mut::<PetBehavior>().living.cursor_in = 0.0;
    let original = world.get::<Transform>(arm.upper).unwrap().rotation;
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.hand = Some(arm.upper);
    smoke.original = original;
    smoke.changed = false;
}
fn escaped(world: &mut World) {
    world.resource_mut::<LivingPointer>().scripted = Some(None);
}
fn floor(world: &mut World) {
    let env = world.resource::<DesktopEnvironment>();
    let (hwnd, rect, monitor, feet) = (
        env.nera_hwnd.unwrap(),
        env.nera_rect.unwrap(),
        env.monitor.unwrap(),
        env.feet_anchor_logical.unwrap(),
    );
    let x = monitor.work_area.left + 160;
    let y = monitor.work_area.bottom - (feet.y * env.nera_scale_factor as f32).round() as i32;
    platform::move_window(hwnd, platform::PhysicalPoint::new(x, y)).unwrap();
    world
        .resource_mut::<DesktopPhysicsState>()
        .reset_for_position_recovery();
    world.resource_mut::<DesktopEnvironment>().nera_rect = platform::window_rect(hwnd);
    world.resource_mut::<Smoke>().home = x;
    assert_eq!(
        world
            .resource::<DesktopEnvironment>()
            .nera_rect
            .unwrap()
            .width(),
        rect.width()
    );
}
fn run(world: &mut World) {
    let elapsed = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<Smoke>().stage;
    let since = elapsed - world.resource::<Smoke>().at;
    assert!(
        elapsed < 150.0,
        "Living native validation timed out at stage {stage}"
    );
    world.resource_mut::<PetBehavior>().decision_in = 100.0;
    if let Some(bone) = world.resource::<Smoke>().hand
        && let Some(pose) = world.get::<Transform>(bone)
        && pose
            .rotation
            .angle_between(world.resource::<Smoke>().original)
            > 0.15
    {
        world.resource_mut::<Smoke>().changed = true;
    }
    match stage {
        0 => {
            let mut settings = world.resource_mut::<NeraSettings>();
            settings.audio_muted = true;
            settings.audio_reactive_dance = false;
            settings.walking_enabled = false;
            settings.cursor_play = false;
            settings.edge_peek = false;
            settings.pause_roaming_fullscreen = false;
            if elapsed < 4.0
                || world.resource::<NeraAnimationState>().visual_state() != VisualState::Idle
                || world.resource::<DesktopPhysicsState>().falling
            {
                return;
            }
            std::fs::create_dir_all(output()).unwrap();
            floor(world);
            advance(world, 1);
        }
        1 if since > 1.0 && world.resource::<DesktopPhysicsState>().support.is_some() => {
            aim(world);
            advance(world, 2);
        }
        2 if world.resource::<NeraAnimationState>().visual_state()
            == VisualState::IdleMicro(IdleMicroAction::CursorCatch) =>
        {
            assert!(
                world.resource::<Smoke>().changed,
                "real arm must move during Cursor Reach"
            );
            crate::avatar::assert_smoke_ground_contact(world);
            capture(world, "cursor-catch.png");
            advance(world, 3);
        }
        3 if since > 1.5 && world.resource::<PetBehavior>().living.cursor.is_none() => {
            aim(world);
            advance(world, 4);
        }
        4 if world
            .resource::<PetBehavior>()
            .living
            .cursor
            .is_some_and(|c| c.phase == CursorPhase::Reach && c.elapsed > 0.6) =>
        {
            escaped(world);
            advance(world, 5);
        }
        5 if since > 0.1 => {
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::IdleMicro(IdleMicroAction::CursorEscape)
            );
            capture(world, "cursor-escaped.png");
            advance(world, 6);
        }
        6 if since > 1.1 => {
            assert!(world.resource::<PetBehavior>().living.cursor.is_none());
            world.write_message(crate::avatar::body_mode::ToggleBodyMode);
            advance(world, 7);
        }
        7 if since > 1.0 => {
            assert_eq!(
                *world.resource::<crate::avatar::body_mode::AvatarBodyMode>(),
                crate::avatar::body_mode::AvatarBodyMode::Chibi
            );
            aim(world);
            advance(world, 8);
        }
        8 if world.resource::<NeraAnimationState>().visual_state()
            == VisualState::IdleMicro(IdleMicroAction::CursorCatch) =>
        {
            assert!(world.resource::<Smoke>().changed);
            crate::avatar::assert_smoke_ground_contact(world);
            capture(world, "chibi-cursor-catch.png");
            escaped(world);
            advance(world, 9);
        }
        9 if since > 1.5 => {
            let settings = world.resource::<NeraSettings>();
            let saved = IVec2::new(settings.window_x, settings.window_y);
            world.resource_mut::<Smoke>().saved_position = saved;
            world.resource_mut::<NeraSettings>().cursor_play = false;
            world.resource_mut::<NeraSettings>().walking_enabled = true;
            world.resource_mut::<NeraSettings>().edge_peek = true;
            world.resource_mut::<PetBehavior>().living.edge_in = 0.0;
            advance(world, 10);
        }
        10 if world
            .resource::<PetBehavior>()
            .living
            .edge
            .is_some_and(|e| e.phase == EdgePhase::Hide) =>
        {
            let env = world.resource::<DesktopEnvironment>();
            let rect = env.nera_rect.unwrap();
            let work = env.monitor.unwrap().work_area;
            assert!(
                rect.left < work.left || rect.right > work.right,
                "real HWND must be partly outside the monitor"
            );
            let foot = world
                .resource::<DesktopPhysicsState>()
                .support_anchor_logical()
                .unwrap()
                .x
                * env.nera_scale_factor as f32
                + rect.left as f32;
            assert!(foot > work.left as f32 && foot < work.right as f32);
            let settings = world.resource::<NeraSettings>();
            assert_eq!(
                IVec2::new(settings.window_x, settings.window_y),
                world.resource::<Smoke>().saved_position,
                "temporary hide position must not be persisted"
            );
            crate::avatar::assert_smoke_ground_contact(world);
            capture(world, "edge-hidden-render.png");
            advance(world, 11);
        }
        11 if world.resource::<PetBehavior>().living.edge.is_none() => {
            assert_eq!(
                world
                    .resource::<DesktopEnvironment>()
                    .nera_rect
                    .unwrap()
                    .left,
                world.resource::<Smoke>().home
            );
            capture(world, "edge-returned.png");
            world.resource_mut::<PetBehavior>().living.edge_in = 0.0;
            advance(world, 12);
        }
        12 if world
            .resource::<PetBehavior>()
            .living
            .edge
            .is_some_and(|e| e.phase == EdgePhase::Hide) =>
        {
            world.write_message(crate::interaction::NeraClicked {
                body_part: crate::interaction::BodyPart::LeftHand,
                pointer: crate::interaction::NeraPointerPosition {
                    screen: IVec2::ZERO,
                    local: Vec2::ZERO,
                },
            });
            advance(world, 13);
        }
        13 if since > 0.25 => {
            assert!(world.resource::<PetBehavior>().living.edge.is_none());
            assert_eq!(
                world
                    .resource::<DesktopEnvironment>()
                    .nera_rect
                    .unwrap()
                    .left,
                world.resource::<Smoke>().home
            );
            assert!(matches!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::TouchReaction(_)
            ));
            clear_for_character_change(world);
            let model = std::path::PathBuf::from(std::env::var_os("APPDATA").unwrap())
                .join("io.github.sahidhaqqi.projectnera/characters/model-07665e5f50f3b8ac.vrm");
            assert!(model.is_file());
            world.write_message(crate::character::ImportCharacter(model));
            advance(world, 14);
        }
        14 if since > 2.0
            && !world
                .resource::<crate::character::CharacterImportStatus>()
                .loading
            && world.resource::<NeraAnimationState>().visual_state() == VisualState::Idle =>
        {
            assert_ne!(
                world
                    .resource::<crate::character::ActiveCharacter>()
                    .asset_path,
                crate::character::DEFAULT_CHARACTER
            );
            aim(world);
            advance(world, 15);
        }
        15 if world.resource::<NeraAnimationState>().visual_state()
            == VisualState::IdleMicro(IdleMicroAction::CursorCatch) =>
        {
            assert!(world.resource::<Smoke>().changed);
            capture(world, "custom-cursor-catch.png");
            escaped(world);
            advance(world, 16);
        }
        16 if since > 1.5 => {
            aim(world);
            world.write_message(crate::cloud::CloudCommand::Summon(
                crate::cloud::CloudMode::Rest,
            ));
            advance(world, 17);
        }
        17 if since > 1.0 => {
            assert!(
                world
                    .resource::<crate::cloud::CloudRuntime>()
                    .owns_placement()
            );
            assert!(world.resource::<PetBehavior>().cursor_pose().is_none());
            assert!(world.resource::<PetBehavior>().living.edge.is_none());
            world.write_message(crate::cloud::CloudCommand::Dismiss);
            advance(world, 18);
        }
        18 if since > 1.5
            && world.resource::<NeraAnimationState>().visual_state() == VisualState::Idle =>
        {
            aim(world);
            advance(world, 19);
        }
        19 if world
            .resource::<PetBehavior>()
            .living
            .cursor
            .is_some_and(|c| c.phase == CursorPhase::Reach && c.elapsed > 0.3) =>
        {
            world.write_message(crate::animation::AlarmAnimationCommand::Start);
            world.write_message(crate::presentation::PresentationCommand::EnterAttention(
                crate::presentation::AttentionKind::Alarm,
            ));
            advance(world, 20);
        }
        20 if since > 0.5 => {
            assert!(world.resource::<PresentationState>().is_attention());
            assert!(world.resource::<PetBehavior>().cursor_pose().is_none());
            world.write_message(crate::animation::AlarmAnimationCommand::Stop);
            world.write_message(crate::presentation::PresentationCommand::ExitAttention);
            advance(world, 21);
        }
        21 if since > 1.5
            && world.resource::<NeraAnimationState>().visual_state() == VisualState::Idle =>
        {
            aim(world);
            advance(world, 22);
        }
        22 if world
            .resource::<PetBehavior>()
            .living
            .cursor
            .is_some_and(|c| c.phase == CursorPhase::Reach && c.elapsed > 0.3) =>
        {
            world.resource_mut::<DesktopPetInteraction>().state = InteractionState::Dragging;
            advance(world, 23);
        }
        23 if since > 0.3 => {
            assert!(world.resource::<PetBehavior>().cursor_pose().is_none());
            world
                .resource_mut::<DesktopPetInteraction>()
                .reset_gesture_state();
            let report = serde_json::json!({"status":"PASSED", "pointer":"synthetic facts, no OS mouse/keyboard control", "bundled_cursor_catch_escape":true, "chibi_reach":true, "native_floor_edge_hide_reveal_home":true, "touch_interrupt_recovers":true, "imported_alicia_reach":true, "rest_cloud_suppresses":true, "manual_input":false});
            std::fs::write(
                output().join("report.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            info!("LIVING_NATIVE PASSED");
            world.write_message(bevy::app::AppExit::Success);
            advance(world, 24);
        }
        _ => {}
    }
}
