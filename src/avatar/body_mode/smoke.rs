//! Bounded Windows render/placement regression. Inputs are synthetic semantic
//! messages; screenshots still come from the real rendered VRM and native HWND.
use super::{AvatarBodyMode, BodyProportions, ToggleBodyMode};
use crate::{
    animation::{
        AlarmAnimationCommand, IdleMicroAction, LibraryPlayback, NeraAnimationClips,
        NeraAnimationPlayback, NeraAnimationState, PlayLibraryAnimation, VisualState,
    },
    avatar::NeraAvatar,
    cloud::{CloudCommand, CloudMode, CloudRuntime, CloudSeatPose},
    desktop_physics::{DesktopPhysicsState, PhysicsSupport},
    environment::{AvatarAnchor, DesktopEnvironment, PlacementState, TaskbarEdge},
    interaction::{
        BodyPart, DesktopPetInteraction, InteractionState, NeraDragStarted, NeraHeadPatted,
        NeraLiveBone, NeraPointerPosition, NeraReleased,
    },
    platform::{NativeWindowHandle, PhysicalPoint, PhysicalRect},
    presentation::{AttentionKind, PresentationCommand, PresentationState},
    settings::NeraSettings,
    walking::{WalkIntent, WalkingRuntime},
};
use bevy::{
    ecs::system::RunSystemOnce,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};
use bevy_vrm1::{prelude::*, vrm::body_tracking::BodyTracking};
use std::path::PathBuf;

#[derive(Resource)]
struct Smoke {
    stage: u8,
    at: f32,
    custom: bool,
    interactive: bool,
    authored_root: Option<Vec3>,
    model: PathBuf,
    original_scales: Option<(Vec3, Vec3)>,
    original_span: f32,
    support: Option<PhysicsSupport>,
    support_y: Option<f32>,
    walking_start: Option<PhysicalRect>,
    placement: PlacementState,
    seat: Option<IVec2>,
    revision: u64,
    avatar: Option<Entity>,
    window: Option<NativeWindowHandle>,
    captures: Vec<String>,
    skips: Vec<String>,
}

pub(crate) fn install(app: &mut App) {
    let model = std::env::args()
        .find_map(|arg| arg.strip_prefix("--chibi-smoke-model=").map(PathBuf::from))
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
                .join("io.github.sahidhaqqi.projectnera/characters/model-07665e5f50f3b8ac.vrm")
        });
    assert!(
        model.is_file(),
        "Chibi smoke requires an imported custom VRM: {}",
        model.display()
    );
    if std::env::args().any(|arg| arg == "--chibi-proportion-compare") {
        install_comparison(app, model);
        return;
    }
    app.insert_resource(Smoke {
        stage: 0,
        at: 0.0,
        custom: false,
        interactive: std::env::args().any(|arg| arg == "--chibi-smoke-interactive"),
        authored_root: None,
        model,
        original_scales: None,
        original_span: 0.0,
        support: None,
        support_y: None,
        walking_start: None,
        placement: PlacementState::Free,
        seat: None,
        revision: 0,
        avatar: None,
        window: None,
        captures: Vec::new(),
        skips: Vec::new(),
    })
    .add_systems(Last, run.after(crate::pet_feedback::update_particles));
}

fn advance(world: &mut World, stage: u8) {
    let at = world.resource::<Time<Real>>().elapsed_secs();
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.stage = stage;
    smoke.at = at;
    info!(
        "Chibi native smoke {} stage {stage}",
        if smoke.custom { "custom" } else { "default" }
    );
}

fn authored_root_base(world: &mut World) -> Vec3 {
    let entity = avatar(world);
    let root = *world.get::<Transform>(entity).unwrap();
    let mut translation = root.translation;
    // Check the reversible proportion layer below the checkpoint contact
    // correction. That correction legitimately changes with evaluated feet.
    if let Some(y) = world
        .resource::<crate::animation::LibraryMotion>()
        .raw_root_y_for_validation(entity)
    {
        translation.y = y;
    }
    let offset = world.resource::<BodyProportions>().root_offset_y();
    translation - root.rotation * (root.scale * Vec3::Y * offset)
}

fn capture(world: &mut World, state: &str) {
    let name = format!(
        "chibi-{}-{state}.png",
        if world.resource::<Smoke>().custom {
            "custom"
        } else {
            "default"
        }
    );
    let directory = std::env::temp_dir().join("nera-chibi-smoke");
    std::fs::create_dir_all(&directory).unwrap();
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(directory.join(&name)));
    world.resource_mut::<Smoke>().captures.push(name);
}

fn pointer() -> NeraPointerPosition {
    NeraPointerPosition {
        screen: IVec2::ZERO,
        local: Vec2::ZERO,
    }
}

fn avatar(world: &mut World) -> Entity {
    world
        .query_filtered::<Entity, (With<NeraAvatar>, With<Initialized>)>()
        .single(world)
        .unwrap()
}

fn scales_and_span(world: &mut World) -> (Vec3, Vec3, f32) {
    let mut hips = None;
    let mut head = None;
    for (bone, local, global) in world
        .query_filtered::<(&VrmBone, &Transform, &GlobalTransform), With<NeraLiveBone>>()
        .iter(world)
    {
        assert!(
            local.translation.is_finite() && local.rotation.is_finite() && local.scale.is_finite()
        );
        assert!(global.translation().is_finite());
        match bone.as_str() {
            "hips" => hips = Some((local.scale, global.translation())),
            "head" => head = Some((local.scale, global.translation())),
            _ => {}
        }
    }
    let hips = hips.unwrap();
    let head = head.unwrap();
    (hips.0, head.0, head.1.distance(hips.1))
}

fn assert_mode(world: &mut World, mode: AvatarBodyMode) {
    assert_eq!(*world.resource::<AvatarBodyMode>(), mode);
    let body = world.resource::<BodyProportions>();
    let factor = if mode == AvatarBodyMode::Chibi {
        0.62
    } else {
        1.0
    };
    assert!((body.body_scale() - factor).abs() < 0.0001);
    let rig = body
        .rig
        .as_ref()
        .expect("loaded avatar must have safe mapped proportions");
    let hips = world.get::<Transform>(rig.hips).unwrap().scale;
    let head = world.get::<Transform>(rig.head).unwrap().scale;
    if let Some((raw_hips, raw_head)) = rig.pose_scales {
        assert!(hips.abs_diff_eq(raw_hips * body.body_scale(), 0.00001));
        assert!(head.abs_diff_eq(raw_head * body.head_scale(), 0.00001));
    }
    let _ = scales_and_span(world);
}

fn remember_support(world: &mut World) {
    let physics = world.resource::<DesktopPhysicsState>();
    let support = physics.support;
    let y = physics.support_anchor_logical().map(|point| point.y);
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.support = support;
    smoke.support_y = y;
}

fn assert_support(world: &mut World) {
    let physics = world.resource::<DesktopPhysicsState>();
    let smoke = world.resource::<Smoke>();
    assert!(!physics.falling && physics.support.is_some());
    assert_eq!(
        physics.support, smoke.support,
        "body mode must retain native surface identity"
    );
    assert!(
        (physics.support_anchor_logical().unwrap().y - smoke.support_y.unwrap()).abs() < 2.0,
        "body mode must preserve the existing rigid support plane"
    );
    crate::avatar::assert_smoke_ground_contact(world);
}

fn begin_walk(world: &mut World) {
    world
        .run_system_once(
            |physics: Res<DesktopPhysicsState>,
             environment: Res<DesktopEnvironment>,
             clips: Res<NeraAnimationClips>,
             mut walking: ResMut<WalkingRuntime>,
             mut animation: ResMut<NeraAnimationState>| {
                let surface = physics
                    .walking_surface(&environment)
                    .expect("smoke needs a real walking support");
                let monitor = environment.monitor.unwrap();
                let current = environment.nera_rect.unwrap().left - surface.origin_x;
                let bounds = (
                    surface.bounds.0 - surface.origin_x,
                    surface.bounds.1 - surface.origin_x,
                );
                let target = if current + 180 < bounds.1 {
                    current + 180
                } else {
                    current - 180
                }
                .clamp(bounds.0, bounds.1);
                assert!(current.abs_diff(target) > 40);
                // The smoke owns this explicit journey. Cancel any previously
                // active runtime and use the real loaded LookAround lifetime,
                // which the controller requires before it starts the gait.
                walking.cancel(&mut animation);
                animation.cancel_startup_greeting();
                animation.cancel_idle_variation();
                let lookaround_duration = clips
                    .idle_micro_duration(IdleMicroAction::LookAround)
                    .filter(|duration| !duration.is_zero())
                    .expect("smoke needs the loaded LookAround clip duration");
                assert!(walking.begin(
                    WalkIntent {
                        surface: surface.key,
                        monitor: monitor.handle,
                        target_x: target,
                        bounds,
                        speed: 50.0,
                        scale: 1.0,
                        return_trip: false,
                        edge_peek: false,
                    },
                    current,
                    0.2,
                    &mut animation,
                    lookaround_duration
                ));
            },
        )
        .unwrap();
    world.resource_mut::<Smoke>().walking_start = world.resource::<DesktopEnvironment>().nera_rect;
}

fn stop_walk(world: &mut World) {
    world.resource_mut::<NeraSettings>().walking_enabled = false;
    world
        .run_system_once(
            |mut walking: ResMut<WalkingRuntime>, mut animation: ResMut<NeraAnimationState>| {
                walking.cancel(&mut animation);
            },
        )
        .unwrap();
}

fn reset_animation(world: &mut World) {
    world.write_message(crate::action_menu::NeraActionMenuCommand::Animation(
        crate::action_menu::AnimationMenuChoice::ResetBasePose,
    ));
    world
        .resource_mut::<NeraAnimationState>()
        .cancel_idle_variation();
}

fn library_is(world: &World, path: &str) -> bool {
    if let VisualState::LibraryAnimation(entity) =
        world.resource::<NeraAnimationState>().visual_state()
    {
        world.resource::<LibraryPlayback>().path_for(entity) == Some(path)
    } else {
        false
    }
}

fn begin_drag(world: &mut World) {
    world
        .run_system_once(
            |mut cloud: ResMut<CloudRuntime>,
             mut environment: ResMut<DesktopEnvironment>,
             mut physics: ResMut<DesktopPhysicsState>,
             mut animation: ResMut<NeraAnimationState>,
             mut sfx: MessageWriter<crate::pet_feedback::PetSfx>,
             mut fx: MessageWriter<crate::pet_feedback::PetFx>| {
                crate::cloud::begin_cloud_drag(
                    &mut cloud,
                    &mut environment,
                    &mut physics,
                    &mut animation,
                    &mut sfx,
                    &mut fx,
                );
            },
        )
        .unwrap();
    world.resource_mut::<DesktopPetInteraction>().state = InteractionState::Dragging;
    world.write_message(NeraDragStarted {
        screen: IVec2::ZERO,
        body_part: BodyPart::Torso,
    });
}

fn release(world: &mut World) {
    world
        .resource_mut::<DesktopPetInteraction>()
        .reset_gesture_state();
    world.write_message(NeraReleased {
        pointer: pointer(),
        body_part: BodyPart::Torso,
        was_dragging: true,
    });
}

fn move_seat(world: &mut World, point: IVec2) {
    let environment = world.resource::<DesktopEnvironment>();
    let rect = environment.nera_rect.unwrap();
    let seat = environment.seat_probe_global().unwrap();
    let hwnd = environment.nera_hwnd.unwrap();
    crate::platform::move_window(
        hwnd,
        PhysicalPoint::new(rect.left + point.x - seat.x, rect.top + point.y - seat.y),
    )
    .unwrap();
    world.resource_mut::<DesktopEnvironment>().nera_rect = crate::platform::window_rect(hwnd);
}

fn remember_seat(world: &mut World) {
    let environment = world.resource::<DesktopEnvironment>();
    let placement = environment.placement;
    let seat = environment.seat_probe_global().unwrap();
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.placement = placement;
    smoke.seat = Some(IVec2::new(seat.x, seat.y));
}

fn assert_seat(world: &World, placement: bool) {
    let environment = world.resource::<DesktopEnvironment>();
    let smoke = world.resource::<Smoke>();
    if placement {
        assert_eq!(environment.placement, smoke.placement);
    }
    let seat = environment.seat_probe_global().unwrap();
    let delta = IVec2::new(seat.x, seat.y) - smoke.seat.unwrap();
    assert!(
        delta.abs().max_element() <= 2,
        "resized avatar detached from seat: {delta:?}"
    );
}

fn taskbar_sit(world: &mut World) -> bool {
    let environment = world.resource::<DesktopEnvironment>();
    let taskbar = environment
        .taskbar
        .taskbars
        .iter()
        .find(|bar| bar.edge == TaskbarEdge::Bottom)
        .copied();
    let Some(taskbar) = taskbar else {
        world
            .resource_mut::<Smoke>()
            .skips
            .push("No bottom taskbar exposed; sitting taskbar check skipped".into());
        return false;
    };
    move_seat(
        world,
        IVec2::new(
            (taskbar.rect.left + taskbar.rect.right) / 2,
            taskbar.rect.top,
        ),
    );
    let mut environment = world.resource_mut::<DesktopEnvironment>();
    environment.set_attached_anchor(AvatarAnchor::Seat);
    environment.placement = PlacementState::PendingTaskbar {
        monitor: taskbar.monitor,
        edge: taskbar.edge,
        settle_frames: 1,
    };
    true
}

fn window_sit(world: &mut World) -> bool {
    let environment = world.resource::<DesktopEnvironment>();
    let monitor = environment.monitor.unwrap();
    let hwnd = environment.nera_hwnd.unwrap();
    let candidate = environment
        .discovered_windows
        .iter()
        .find(|window| {
            let x = (window.rect.left + window.rect.right) / 2;
            window.rect.top > monitor.work_area.top + 160
                && window.rect.top < monitor.work_area.bottom - 80
                && crate::platform::window_at_point(
                    PhysicalPoint::new(x, window.rect.top + 1),
                    hwnd,
                    &environment.discovered_windows,
                ) == Some(window.handle)
        })
        .cloned();
    let Some(candidate) = candidate else {
        world.resource_mut::<Smoke>().skips.push(
            "No visible external window edge with sitting headroom; window sitting skipped".into(),
        );
        return false;
    };
    move_seat(
        world,
        IVec2::new(
            (candidate.rect.left + candidate.rect.right) / 2,
            candidate.rect.top,
        ),
    );
    world.resource_mut::<Smoke>().window = Some(candidate.handle);
    let mut environment = world.resource_mut::<DesktopEnvironment>();
    environment.set_attached_anchor(AvatarAnchor::Seat);
    environment.placement = PlacementState::PendingWindow {
        target: candidate.handle,
        settle_frames: 1,
    };
    true
}

fn floating_seat(world: &mut World) {
    world.resource_mut::<DesktopEnvironment>().detach_nera();
    let bounds = world
        .resource::<DesktopEnvironment>()
        .monitor
        .unwrap()
        .work_area;
    move_seat(
        world,
        IVec2::new(
            (bounds.left + bounds.right) / 2,
            bounds.top + bounds.height() * 55 / 100,
        ),
    );
}

fn assert_rest(world: &mut World) {
    assert_eq!(
        world.resource::<CloudRuntime>().mode(),
        Some(CloudMode::Rest)
    );
    assert_eq!(
        world.resource::<NeraAnimationState>().visual_state(),
        VisualState::SittingLoop
    );
    let entity = avatar(world);
    assert!(world.get::<LookAt>(entity).is_none());
    assert!(world.get::<BodyTracking>(entity).is_none());
    assert!(!world.resource::<DesktopPhysicsState>().falling);
    assert_cloud_head_inside_canvas(world);
}

/// Native support alignment can pass while the rendered face is clipped by
/// the transparent window. Check actual evaluated humanoid landmarks as well
/// as the native seat; unusual long accessories remain a documented V1 limit.
fn assert_cloud_head_inside_canvas(world: &mut World) {
    world
        .run_system_once(
            |camera: Query<(&Camera, &GlobalTransform), With<crate::avatar::NeraMainCamera>>,
             bones: Query<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>| {
                let (camera, view) = camera.single().unwrap();
                let viewport = camera.logical_viewport_size().unwrap();
                let mut head_checked = false;
                for (bone, global) in &bones {
                    if !matches!(
                        bone.as_str(),
                        "head" | "leftEye" | "rightEye" | "jaw" | "neck"
                    ) {
                        continue;
                    }
                    let pixel = camera
                        .world_to_viewport(view, global.translation())
                        .unwrap();
                    assert!(
                        pixel.is_finite()
                            && pixel.x >= 0.0
                            && pixel.y >= 0.0
                            && pixel.x < viewport.x
                            && pixel.y < viewport.y,
                        "Cloud face landmark {} escaped render canvas: {pixel:?} / {viewport:?}",
                        bone.as_str(),
                    );
                    head_checked |= bone.as_str() == "head";
                }
                assert!(head_checked, "native face check requires the mapped head");
            },
        )
        .unwrap();
}

/// The dependency owns the solver. Check its actual live chain transforms for
/// finite values after repeated transitions and keep a rendered hair/clothing
/// screenshot for visual inspection of the settled result.
fn assert_springs_finite(world: &mut World) {
    let entity = avatar(world);
    let root = world.get::<HipsBoneEntity>(entity).unwrap().0;
    let mut stack = vec![root];
    let mut joints = Vec::new();
    while let Some(entity) = stack.pop() {
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
        if let Some(spring) = world.get::<SpringRoot>(entity) {
            joints.extend(spring.joints.iter().copied());
        }
    }
    for entity in &joints {
        let local = world
            .get::<Transform>(*entity)
            .expect("live spring joint needs its local transform");
        let global = world
            .get::<GlobalTransform>(*entity)
            .expect("live spring joint needs its propagated transform");
        assert!(
            local.translation.is_finite() && local.rotation.is_finite() && local.scale.is_finite()
        );
        assert!(
            global.to_matrix().is_finite(),
            "spring transform became stale or non-finite"
        );
    }
    info!(
        "Chibi native smoke checked {} live spring joints",
        joints.len()
    );
}

fn drag_toggle(world: &mut World) {
    reset_animation(world);
    begin_drag(world);
    world.resource_mut::<Smoke>().revision = world.resource::<BodyProportions>().revision();
    world.write_message(ToggleBodyMode);
    advance(world, 11);
}

fn run(world: &mut World) {
    let elapsed = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<Smoke>().stage;
    // Successful interactive runs stay available for real pointer/menu checks.
    // All bounded regression work has already completed and produced its report.
    if stage == 30 && world.resource::<Smoke>().interactive {
        return;
    }
    let since = elapsed - world.resource::<Smoke>().at;
    assert!(
        elapsed < 180.0,
        "Chibi native smoke timed out at stage {stage}"
    );
    assert!(
        stage == 0 || since < 25.0,
        "Chibi stage {stage} did not reach its expected state"
    );
    match stage {
        0 if elapsed > 3.0
            && !world
                .resource::<crate::startup_entrance::StartupEntrance>()
                .blocks_position_save()
            && world.resource::<DesktopPhysicsState>().support.is_some()
            && world.resource::<BodyProportions>().rig.is_some() =>
        {
            let mut settings = world.resource_mut::<NeraSettings>();
            settings.audio_muted = true;
            settings.pause_roaming_fullscreen = false;
            settings.walking_enabled = false;
            settings.follow_mouse = true;
            settings.follow_head = true;
            world
                .resource_mut::<NeraAnimationState>()
                .cancel_startup_greeting();
            world
                .resource_mut::<NeraAnimationState>()
                .cancel_idle_variation();
            assert_mode(world, AvatarBodyMode::Normal);
            let (hips, head, span) = scales_and_span(world);
            let entity = avatar(world);
            let mut smoke = world.resource_mut::<Smoke>();
            smoke.original_scales = Some((hips, head));
            smoke.original_span = span;
            smoke.avatar = Some(entity);
            remember_support(world);
            capture(world, "normal-idle");
            world.write_message(ToggleBodyMode);
            advance(world, 1);
        }
        1 if since > 0.7 => {
            assert_mode(world, AvatarBodyMode::Chibi);
            assert_support(world);
            let (_, _, span) = scales_and_span(world);
            assert!(
                span < world.resource::<Smoke>().original_span * 0.8,
                "body must visibly shorten on the evaluated hierarchy"
            );
            capture(world, "chibi-idle");
            world.write_message(ToggleBodyMode);
            advance(world, 2);
        }
        2 if since > 0.6 => {
            assert_mode(world, AvatarBodyMode::Normal);
            assert_support(world);
            let (hips, head, _) = scales_and_span(world);
            let original = world.resource::<Smoke>().original_scales.unwrap();
            assert_eq!(
                (hips, head),
                original,
                "Normal must exactly restore imported scales"
            );
            capture(world, "restored-idle");
            world.write_message(ToggleBodyMode);
            advance(world, 3);
        }
        3 if since > 0.6
            && world.resource::<NeraAnimationClips>().walking_ready()
            && world
                .resource::<NeraAnimationClips>()
                .idle_micro_duration(IdleMicroAction::LookAround)
                .is_some_and(|duration| !duration.is_zero()) =>
        {
            assert_mode(world, AvatarBodyMode::Chibi);
            // Let pet_behavior observe the changed preference before creating
            // our intent; its normal preference update cancels old journeys.
            world.resource_mut::<NeraSettings>().walking_enabled = true;
            advance(world, 34);
        }
        34 if since > 0.4 => {
            begin_walk(world);
            advance(world, 4);
        }
        4 if since > 0.8
            && world.resource::<NeraAnimationState>().visual_state() == VisualState::Walking
            && world
                .resource::<DesktopEnvironment>()
                .nera_rect
                .unwrap()
                .left
                .abs_diff(world.resource::<Smoke>().walking_start.unwrap().left)
                > 3 =>
        {
            assert_mode(world, AvatarBodyMode::Chibi);
            assert_support(world);
            let start = world.resource::<Smoke>().walking_start.unwrap();
            assert!(
                world
                    .resource::<DesktopEnvironment>()
                    .nera_rect
                    .unwrap()
                    .left
                    .abs_diff(start.left)
                    > 3
            );
            capture(world, "walking-chibi");
            world.write_message(ToggleBodyMode);
            advance(world, 5);
        }
        5 if since > 0.5 => {
            assert_mode(world, AvatarBodyMode::Normal);
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::Walking
            );
            assert_support(world);
            capture(world, "walking-normal");
            world.write_message(ToggleBodyMode);
            stop_walk(world);
            world.write_message(NeraHeadPatted { pointer: pointer() });
            advance(world, 6);
        }
        6 if since > 0.35 => {
            assert_mode(world, AvatarBodyMode::Chibi);
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::HeadStroke
            );
            assert!(
                world
                    .resource::<NeraAnimationClips>()
                    .head_stroke_playing(world.resource::<NeraAnimationPlayback>())
            );
            assert_support(world);
            capture(world, "headpat-chibi");
            world.write_message(ToggleBodyMode);
            advance(world, 7);
        }
        7 if since > 0.35 => {
            assert_mode(world, AvatarBodyMode::Normal);
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::HeadStroke
            );
            capture(world, "headpat-normal");
            reset_animation(world);
            world.write_message(PlayLibraryAnimation {
                path: "animations/animasi_basic/Relax.vrma".into(),
            });
            advance(world, 8);
        }
        8 if since > 0.4 && library_is(world, "animations/animasi_basic/Relax.vrma") => {
            assert_mode(world, AvatarBodyMode::Normal);
            capture(world, "library-normal");
            world.write_message(ToggleBodyMode);
            advance(world, 9);
        }
        9 if since > 0.5 => {
            assert_mode(world, AvatarBodyMode::Chibi);
            assert!(library_is(world, "animations/animasi_basic/Relax.vrma"));
            assert_support(world);
            capture(world, "library-chibi");
            world.write_message(PlayLibraryAnimation {
                path: "animations/animasi_basic/Death01.vrma".into(),
            });
            advance(world, 10);
        }
        10 if since > 1.0 && library_is(world, "animations/animasi_basic/Death01.vrma") => {
            assert_mode(world, AvatarBodyMode::Chibi);
            capture(world, "library-death-chibi");
            let path = "animations/UAL1_converted_43_vrma/Sword_Idle.vrma";
            if crate::nera_asset_file(path).is_file() {
                world.write_message(ToggleBodyMode);
                world.write_message(PlayLibraryAnimation { path: path.into() });
                advance(world, 32);
            } else {
                world
                    .resource_mut::<Smoke>()
                    .skips
                    .push("Optional authored Sword_Idle VRMA unavailable".into());
                drag_toggle(world);
            }
        }
        32 if since > 0.5
            && library_is(world, "animations/UAL1_converted_43_vrma/Sword_Idle.vrma") =>
        {
            assert_mode(world, AvatarBodyMode::Normal);
            assert_eq!(world.resource::<BodyProportions>().root_offset_y(), 0.0);
            let root = authored_root_base(world);
            world.resource_mut::<Smoke>().authored_root = Some(root);
            capture(world, "authored-sword-normal");
            world.write_message(ToggleBodyMode);
            advance(world, 33);
        }
        33 if since > 0.7 => {
            assert_mode(world, AvatarBodyMode::Chibi);
            assert!(library_is(
                world,
                "animations/UAL1_converted_43_vrma/Sword_Idle.vrma"
            ));
            let offset = world.resource::<BodyProportions>().root_offset_y();
            assert!(
                offset.is_finite() && offset < 0.0,
                "Chibi needs its measured neutral-root correction"
            );
            let normal_root = world.resource::<Smoke>().authored_root.unwrap();
            let base = authored_root_base(world);
            assert!(
                base.abs_diff_eq(normal_root, 0.0001),
                "authored VRMA must retain the same avatar-root base"
            );
            assert_springs_finite(world);
            capture(world, "authored-sword-chibi");
            drag_toggle(world);
        }
        11 if since > 0.5 => {
            assert_eq!(*world.resource::<AvatarBodyMode>(), AvatarBodyMode::Normal);
            assert!((world.resource::<BodyProportions>().body_scale() - 0.62).abs() < 0.0001);
            assert_eq!(
                world.resource::<BodyProportions>().revision(),
                world.resource::<Smoke>().revision
            );
            assert_eq!(
                world.resource::<DesktopPetInteraction>().state,
                InteractionState::Dragging
            );
            capture(world, "drag-deferred");
            release(world);
            advance(world, 12);
        }
        12 if since > 1.0
            && !world.resource::<DesktopPhysicsState>().falling
            && world.resource::<DesktopPhysicsState>().support.is_some()
            && world.resource::<NeraAnimationState>().visual_state() == VisualState::Idle =>
        {
            assert_mode(world, AvatarBodyMode::Normal);
            crate::avatar::assert_smoke_ground_contact(world);
            capture(world, "drag-released");
            if taskbar_sit(world) {
                advance(world, 13);
            } else if window_sit(world) {
                advance(world, 16);
            } else {
                floating_seat(world);
                world.write_message(CloudCommand::Summon(CloudMode::Rest));
                advance(world, 19);
            }
        }
        13 if since > 2.5
            && world.resource::<NeraAnimationState>().visual_state()
                == VisualState::SittingLoop =>
        {
            assert!(matches!(
                world.resource::<DesktopEnvironment>().placement,
                PlacementState::Taskbar(_)
            ));
            remember_seat(world);
            capture(world, "taskbar-normal");
            world.write_message(ToggleBodyMode);
            advance(world, 14);
        }
        14 if since > 0.7 => {
            assert_mode(world, AvatarBodyMode::Chibi);
            assert_seat(world, true);
            capture(world, "taskbar-chibi");
            world.write_message(ToggleBodyMode);
            advance(world, 15);
        }
        15 if since > 0.7 => {
            assert_mode(world, AvatarBodyMode::Normal);
            assert_seat(world, true);
            world.resource_mut::<DesktopEnvironment>().detach_nera();
            if window_sit(world) {
                advance(world, 16);
            } else {
                floating_seat(world);
                world.write_message(CloudCommand::Summon(CloudMode::Rest));
                advance(world, 19);
            }
        }
        16 if since > 3.0
            && world.resource::<NeraAnimationState>().visual_state()
                == VisualState::SittingLoop =>
        {
            assert!(matches!(
                world.resource::<DesktopEnvironment>().placement,
                PlacementState::Window(_)
            ));
            remember_seat(world);
            capture(world, "window-normal");
            world.write_message(ToggleBodyMode);
            advance(world, 17);
        }
        17 if since > 0.7 => {
            assert_mode(world, AvatarBodyMode::Chibi);
            assert_seat(world, true);
            capture(world, "window-chibi");
            world.write_message(ToggleBodyMode);
            advance(world, 18);
        }
        18 if since > 0.7 => {
            assert_mode(world, AvatarBodyMode::Normal);
            assert_seat(world, true);
            floating_seat(world);
            world.write_message(CloudCommand::Summon(CloudMode::Rest));
            advance(world, 19);
        }
        19 if since > 3.0
            && world.resource::<NeraAnimationState>().visual_state()
                == VisualState::SittingLoop =>
        {
            assert_rest(world);
            remember_seat(world);
            capture(world, "rest-normal");
            world.write_message(ToggleBodyMode);
            advance(world, 20);
        }
        20 if since > 0.7 => {
            assert_mode(world, AvatarBodyMode::Chibi);
            assert_rest(world);
            assert_seat(world, false);
            capture(world, "rest-chibi");
            world.write_message(NeraHeadPatted { pointer: pointer() });
            advance(world, 21);
        }
        21 if since > 0.35 => {
            assert_rest(world);
            assert_mode(world, AvatarBodyMode::Chibi);
            capture(world, "rest-touch-suppressed");
            world.write_message(CloudCommand::Summon(CloudMode::Ride));
            advance(world, 22);
        }
        22 if since > 0.9 => {
            assert!(world.resource::<CloudRuntime>().is_riding());
            assert!(
                world
                    .resource::<CloudSeatPose>()
                    .is_ready_for(world.resource::<CloudRuntime>())
            );
            let entity = avatar(world);
            assert!(world.get::<LookAt>(entity).is_some());
            remember_seat(world);
            capture(world, "ride-chibi");
            world.write_message(ToggleBodyMode);
            advance(world, 23);
        }
        23 if since > 0.7 => {
            assert_mode(world, AvatarBodyMode::Normal);
            assert!(world.resource::<CloudRuntime>().is_riding());
            assert_seat(world, false);
            capture(world, "ride-normal");
            world.write_message(ToggleBodyMode);
            world.write_message(NeraHeadPatted { pointer: pointer() });
            advance(world, 24);
        }
        24 if since > 0.35 => {
            assert_mode(world, AvatarBodyMode::Chibi);
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::HeadStroke
            );
            assert!(world.resource::<CloudRuntime>().is_riding());
            capture(world, "ride-headpat-chibi");
            world.write_message(PresentationCommand::EnterAttention(AttentionKind::Alarm));
            world.write_message(AlarmAnimationCommand::Start);
            advance(world, 25);
        }
        25 if since > 0.8
            && matches!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::AlarmJump | VisualState::AlarmWave
            ) =>
        {
            assert!(world.resource::<PresentationState>().is_attention());
            assert!(!world.resource::<CloudRuntime>().owns_placement());
            assert_mode(world, AvatarBodyMode::Chibi);
            capture(world, "alarm-chibi");
            world.resource_mut::<Smoke>().revision = world.resource::<BodyProportions>().revision();
            world.write_message(ToggleBodyMode);
            advance(world, 26);
        }
        26 if since > 0.5 => {
            assert_eq!(*world.resource::<AvatarBodyMode>(), AvatarBodyMode::Normal);
            assert!((world.resource::<BodyProportions>().body_scale() - 0.62).abs() < 0.0001);
            assert_eq!(
                world.resource::<BodyProportions>().revision(),
                world.resource::<Smoke>().revision
            );
            assert!(world.resource::<PresentationState>().is_attention());
            capture(world, "alarm-toggle-deferred");
            world.write_message(AlarmAnimationCommand::Stop);
            world.write_message(PresentationCommand::ExitAttention);
            advance(world, 27);
        }
        // Native support can be acquired before the authored Landing clip
        // finishes. Check settled standing, after its idle blend/calibration.
        27 if since > 2.5
            && world.resource::<DesktopPhysicsState>().support.is_some()
            && world.resource::<NeraAnimationState>().visual_state() == VisualState::Idle =>
        {
            assert!(world.resource::<PresentationState>().is_desktop_pet());
            assert_mode(world, AvatarBodyMode::Normal);
            crate::avatar::assert_smoke_ground_contact(world);
            capture(world, "alarm-restored-normal");
            world.write_message(ToggleBodyMode);
            advance(world, 28);
        }
        28 if since > 0.7 => {
            assert_mode(world, AvatarBodyMode::Chibi);
            assert_springs_finite(world);
            capture(world, "settled-springs-chibi");
            if !world.resource::<Smoke>().custom {
                let path = world.resource::<Smoke>().model.clone();
                world.write_message(crate::character::ImportCharacter(path));
                advance(world, 29);
            } else {
                let smoke = world.resource::<Smoke>();
                let report = serde_json::json!({
                    "default_and_imported_custom": true,
                    "inputs": "synthetic semantic messages; real HWND, VRMA, bones and renderer",
                    "manual_pointer_validation": false,
                    "interactive_after_pass": smoke.interactive,
                    "captures": smoke.captures, "skips": smoke.skips,
                    "custom_model": smoke.model,
                    "elapsed_seconds": elapsed,
                });
                let directory = std::env::temp_dir().join("nera-chibi-smoke");
                std::fs::write(
                    directory.join("report.json"),
                    serde_json::to_vec_pretty(&report).unwrap(),
                )
                .unwrap();
                info!(
                    "CHIBI_SMOKE_PASS default/custom toggle/restore/walking/VRMA/headpat/drag/sitting/cloud/alarm; skips={:?}",
                    smoke.skips
                );
                if world.resource::<Smoke>().interactive {
                    stop_walk(world);
                    world.resource_mut::<NeraSettings>().walking_enabled = false;
                    info!(
                        "CHIBI_SMOKE_INTERACTIVE_READY custom Chibi avatar is ready for actual UI validation"
                    );
                }
                advance(world, 30);
            }
        }
        29 if since > 2.0
            && !world
                .resource::<crate::character::CharacterImportStatus>()
                .loading
            && world.resource::<DesktopPhysicsState>().support.is_some() =>
        {
            let entity = avatar(world);
            assert_ne!(
                Some(entity),
                world.resource::<Smoke>().avatar,
                "custom model must replace the actual live hierarchy"
            );
            assert_mode(world, AvatarBodyMode::Chibi);
            world.resource_mut::<Smoke>().custom = true;
            capture(world, "replacement-inherits-chibi");
            world.write_message(ToggleBodyMode);
            advance(world, 31);
        }
        31 if since > 0.7 => {
            assert_mode(world, AvatarBodyMode::Normal);
            advance(world, 0);
        }
        // Let the renderer finish the final settled-springs capture before
        // destroying the native HWND on a noninteractive successful run.
        30 if since > 0.8 => {
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
}

// A short opt-in capture path for assessing proportion constants. It uses the
// existing Idle clip, renderer and body-mode owner, with no production controls.
const COMPARISON_IDLE_TIME: f32 = 0.5;

#[derive(Resource)]
struct Comparison {
    stage: u8,
    at: f32,
    model: PathBuf,
    custom: bool,
    configured: bool,
    freeze: bool,
    camera: Option<(Entity, Transform, Projection)>,
    original_pose: Vec<(String, Transform)>,
    samples: Vec<serde_json::Value>,
}

fn install_comparison(app: &mut App, model: PathBuf) {
    app.insert_resource(Comparison {
        stage: 0,
        at: 0.0,
        model,
        custom: false,
        configured: false,
        freeze: false,
        camera: None,
        original_pose: Vec::new(),
        samples: Vec::new(),
    })
    .add_systems(
        Update,
        comparison_controls
            .after(crate::pet_behavior::update_pet_behavior)
            .before(crate::animation::apply_animation_state),
    )
    .add_systems(
        Update,
        freeze_comparison_pose.after(crate::animation::apply_animation_state),
    )
    .add_systems(
        Last,
        comparison_run.after(crate::pet_feedback::update_particles),
    );
}

fn comparison_controls(world: &mut World) {
    if !world.resource::<Comparison>().configured {
        return;
    }
    let mut settings = world.resource_mut::<NeraSettings>();
    settings.walking_enabled = false;
    settings.audio_muted = true;
    settings.follow_mouse = false;
    settings.follow_eyes = false;
    settings.follow_head = false;
    world
        .run_system_once(
            |mut walking: ResMut<WalkingRuntime>, mut animation: ResMut<NeraAnimationState>| {
                walking.cancel(&mut animation);
                animation.cancel_startup_greeting();
                animation.cancel_idle_variation();
            },
        )
        .unwrap();
    for mut transform in world
        .query_filtered::<&mut Transform, With<NeraAvatar>>()
        .iter_mut(world)
    {
        transform.rotation = Quat::IDENTITY;
    }
    let camera = world.resource::<Comparison>().camera.clone();
    if let Some((entity, transform, projection)) = camera {
        *world.get_mut::<Transform>(entity).unwrap() = transform;
        *world.get_mut::<Projection>(entity).unwrap() = projection;
    }
}

fn freeze_comparison_pose(world: &mut World) {
    if !world.resource::<Comparison>().freeze {
        return;
    }
    // Bevy's paused player still evaluates its seek time. Setting that exact
    // time before PostUpdate means every capture uses the same evaluated pose.
    for mut player in world.query::<&mut AnimationPlayer>().iter_mut(world) {
        for (_, active) in player.playing_animations_mut() {
            active.set_seek_time(COMPARISON_IDLE_TIME).pause();
        }
    }
}

fn comparison_advance(world: &mut World, stage: u8) {
    let now = world.resource::<Time<Real>>().elapsed_secs();
    let mut comparison = world.resource_mut::<Comparison>();
    comparison.stage = stage;
    comparison.at = now;
    info!(
        "Chibi neutral Idle comparison {} stage {stage}",
        if comparison.custom {
            "custom"
        } else {
            "default"
        }
    );
}

fn comparison_pose(world: &mut World) -> Vec<(String, Transform)> {
    let mut pose: Vec<_> = world
        .query_filtered::<(&VrmBone, &Transform), With<NeraLiveBone>>()
        .iter(world)
        .map(|(bone, pose)| (bone.as_str().to_string(), *pose))
        .collect();
    pose.sort_by(|left, right| left.0.cmp(&right.0));
    pose
}

fn comparison_capture(world: &mut World, label: &str, expected: AvatarBodyMode) {
    assert_eq!(*world.resource::<AvatarBodyMode>(), expected);
    assert_eq!(
        world.resource::<NeraAnimationState>().visual_state(),
        VisualState::Idle
    );
    assert!(
        crate::animation::character_animation_ready(world),
        "comparison requires actual Idle playback"
    );
    crate::avatar::assert_smoke_ground_contact(world);
    let entity = avatar(world);
    assert!(world.get::<LookAt>(entity).is_none());
    assert!(world.get::<BodyTracking>(entity).is_none());
    assert!(
        world
            .get::<Transform>(entity)
            .unwrap()
            .rotation
            .abs_diff_eq(Quat::IDENTITY, 0.00001)
    );

    let camera_query = world
        .query_filtered::<(&Camera, &GlobalTransform), With<crate::avatar::NeraMainCamera>>()
        .single(world)
        .unwrap();
    let camera = (camera_query.0.clone(), *camera_query.1);
    let mut projected = std::collections::BTreeMap::new();
    let mut local_scales = std::collections::BTreeMap::new();
    for (bone, pose, global) in world
        .query_filtered::<(&VrmBone, &Transform, &GlobalTransform), With<NeraLiveBone>>()
        .iter(world)
    {
        if matches!(
            bone.as_str(),
            "head"
                | "neck"
                | "hips"
                | "leftLowerLeg"
                | "rightLowerLeg"
                | "leftFoot"
                | "rightFoot"
                | "leftEye"
                | "rightEye"
                | "leftUpperLeg"
                | "rightUpperLeg"
        ) {
            let point = camera
                .0
                .world_to_viewport(&camera.1, global.translation())
                .unwrap();
            projected.insert(bone.as_str().to_string(), point);
            local_scales.insert(bone.as_str().to_string(), pose.scale.to_array());
        }
    }
    let midpoint = |left: &str, right: &str| -> Option<Vec2> {
        Some((*projected.get(left)? + *projected.get(right)?) * 0.5)
    };
    let feet = midpoint("leftFoot", "rightFoot").unwrap();
    let knees = midpoint("leftLowerLeg", "rightLowerLeg").unwrap();
    let hips = *projected.get("hips").unwrap();
    let head = *projected.get("head").unwrap();
    let eye_width = projected
        .get("leftEye")
        .zip(projected.get("rightEye"))
        .map(|(left, right)| left.distance(*right));
    let head_neck = projected.get("neck").map(|neck| head.distance(*neck));
    let players: Vec<_> = world.query::<(Entity, &AnimationPlayer)>().iter(world)
        .flat_map(|(entity, player)| player.playing_animations().map(move |(node, active)| {
            assert!(active.is_paused() && (active.seek_time() - COMPARISON_IDLE_TIME).abs() < 0.00001);
            serde_json::json!({"entity": format!("{entity:?}"), "node": format!("{node:?}"),
                "seek_time": active.seek_time(), "elapsed": active.elapsed(), "paused": active.is_paused(), "weight": active.weight()})
        })).collect();
    assert!(
        !players.is_empty(),
        "comparison needs a paused real animation player"
    );
    let prefix = if world.resource::<Comparison>().custom {
        "custom"
    } else {
        "default"
    };
    let name = format!("{prefix}-{label}.png");
    let body = world.resource::<BodyProportions>();
    let camera_snapshot = world.resource::<Comparison>().camera.as_ref().unwrap();
    let sample = serde_json::json!({
        "avatar": prefix, "mode": format!("{expected:?}"), "screenshot": name,
        "animation": "animations/animasi_basic/idle.vrma", "pose_time_seconds": COMPARISON_IDLE_TIME,
        "camera_transform": {"translation": camera_snapshot.1.translation.to_array(), "rotation": camera_snapshot.1.rotation.to_array(), "scale": camera_snapshot.1.scale.to_array()},
        "camera_projection": format!("{:?}", camera_snapshot.2),
        "root_yaw": 0.0, "gaze_enabled": false,
        "body_scale": body.body_scale(), "head_scale": body.head_scale(), "root_offset_y": body.root_offset_y(),
        "projected_bones_pixels": projected.iter().map(|(name, point)| (name.clone(), point.to_array())).collect::<std::collections::BTreeMap<_, _>>(),
        "local_scales": local_scales,
        "hips_to_feet_pixels": hips.distance(feet), "knees_to_feet_pixels": knees.distance(feet),
        "head_to_hips_pixels": head.distance(hips), "head_to_neck_pixels": head_neck,
        "head_width_eye_proxy_pixels": eye_width,
        "knee_fraction_of_leg_span": knees.distance(feet) / hips.distance(feet),
        "players": players,
    });
    let directory = std::env::temp_dir().join("nera-chibi-proportion-compare");
    std::fs::create_dir_all(&directory).unwrap();
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(directory.join(&name)));
    world.resource_mut::<Comparison>().samples.push(sample);
}

fn comparison_run(world: &mut World) {
    let elapsed = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<Comparison>().stage;
    let since = elapsed - world.resource::<Comparison>().at;
    assert!(
        elapsed < 90.0,
        "Chibi neutral comparison timed out at stage {stage}"
    );
    match stage {
        0 if elapsed > 3.0
            && !world
                .resource::<crate::startup_entrance::StartupEntrance>()
                .blocks_position_save()
            && world.resource::<DesktopPhysicsState>().support.is_some()
            && world.resource::<BodyProportions>().rig.is_some() =>
        {
            let camera = world.query_filtered::<(Entity, &Transform, &Projection), With<crate::avatar::NeraMainCamera>>().single(world).unwrap();
            let snapshot = (camera.0, *camera.1, camera.2.clone());
            let mut comparison = world.resource_mut::<Comparison>();
            comparison.camera = Some(snapshot);
            comparison.configured = true;
            if *world.resource::<AvatarBodyMode>() != AvatarBodyMode::Normal {
                world.write_message(ToggleBodyMode);
            }
            comparison_advance(world, 1);
        }
        1 if since > 0.6
            && world.resource::<NeraAnimationState>().visual_state() == VisualState::Idle
            && crate::animation::character_animation_ready(world) =>
        {
            world.resource_mut::<Comparison>().freeze = true;
            comparison_advance(world, 2);
        }
        2 if since > 0.7 => {
            comparison_capture(world, "normal", AvatarBodyMode::Normal);
            world.resource_mut::<Comparison>().original_pose = comparison_pose(world);
            world.write_message(ToggleBodyMode);
            comparison_advance(world, 3);
        }
        3 if since > 1.0 => {
            comparison_capture(world, "chibi", AvatarBodyMode::Chibi);
            world.write_message(ToggleBodyMode);
            comparison_advance(world, 4);
        }
        4 if since > 1.0 => {
            comparison_capture(world, "restored-normal", AvatarBodyMode::Normal);
            assert_eq!(
                comparison_pose(world),
                world.resource::<Comparison>().original_pose,
                "the same paused Idle pose must restore every humanoid local transform exactly"
            );
            comparison_advance(world, 5);
        }
        5 if since > 0.7 => {
            if !world.resource::<Comparison>().custom {
                let model = world.resource::<Comparison>().model.clone();
                let mut comparison = world.resource_mut::<Comparison>();
                comparison.freeze = false;
                comparison.custom = true;
                world.write_message(crate::character::ImportCharacter(model));
                comparison_advance(world, 6);
            } else {
                let comparison = world.resource::<Comparison>();
                let report = serde_json::json!({
                    "same_evaluated_idle_frame": true,
                    "pose_time_seconds": COMPARISON_IDLE_TIME,
                    "root_yaw": 0.0, "gaze_enabled": false,
                    "custom_model": comparison.model,
                    "samples": comparison.samples,
                });
                let directory = std::env::temp_dir().join("nera-chibi-proportion-compare");
                std::fs::write(
                    directory.join("metrics.json"),
                    serde_json::to_vec_pretty(&report).unwrap(),
                )
                .unwrap();
                info!(
                    "CHIBI_PROPORTION_COMPARE_PASS six same-frame neutral Idle captures plus projected humanoid metrics"
                );
                world.write_message(AppExit::Success);
                comparison_advance(world, 7);
            }
        }
        6 if since > 2.0
            && !world
                .resource::<crate::character::CharacterImportStatus>()
                .loading
            && world.resource::<DesktopPhysicsState>().support.is_some()
            && crate::animation::character_animation_ready(world) =>
        {
            comparison_advance(world, 1);
        }
        _ => {}
    }
}
