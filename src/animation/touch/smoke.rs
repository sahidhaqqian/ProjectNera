//! Bounded native validation using production semantic messages and real VRM/VRMA rendering.
use super::{TouchReactionAccepted, TouchReactionRuntime, seated::TouchSeatPose};
use crate::{
    animation::{
        AlarmAnimationCommand, IdleMicroAction, NeraAnimationClips, NeraAnimationPlayback,
        NeraAnimationState, TouchReactionClip, VisualState,
    },
    avatar::{
        NeraAvatar, NeraMainCamera,
        body_mode::{AvatarBodyMode, BodyProportions, ToggleBodyMode},
    },
    cloud::{CloudCommand, CloudMode, CloudRuntime, CloudSeatPose},
    desktop_physics::DesktopPhysicsState,
    environment::{AvatarAnchor, DesktopEnvironment, PlacementState, TaskbarEdge},
    interaction::{
        BodyPart, DesktopPetInteraction, InteractionState, NeraClicked, NeraDragStarted,
        NeraHeadPatted, NeraLiveBone, NeraPointerPosition, NeraReleased,
    },
    pet_feedback::{PetFx, PetFxKind},
    platform::PhysicalPoint,
    presentation::{AttentionKind, PresentationCommand, PresentationState},
    settings::NeraSettings,
    walking::{WalkIntent, WalkingRuntime},
};
use bevy::{
    ecs::{message::MessageCursor, system::RunSystemOnce},
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::PrimaryWindow,
};
use bevy_vrm1::prelude::{Initialized, VrmBone};
use std::path::PathBuf;

#[derive(Resource)]
struct Smoke {
    stage: u8,
    at: f32,
    initialized: bool,
    custom: bool,
    model: PathBuf,
    cursor: MessageCursor<TouchReactionAccepted>,
    fx_cursor: MessageCursor<PetFx>,
    expected: Option<(BodyPart, u8, String)>,
    collect_step: usize,
    seen: [bool; 8],
    last_send: f32,
    hold_until: f32,
    captures: Vec<String>,
    records: Vec<serde_json::Value>,
    probes: Vec<serde_json::Value>,
    skips: Vec<String>,
    pending_captures: Vec<(f32, String, TouchReactionClip)>,
    avatar: Option<Entity>,
    seat: Option<IVec2>,
    placement: PlacementState,
    lower_body: Vec<(Entity, Transform)>,
}

pub(crate) fn install(app: &mut App) {
    let model = std::env::args()
        .find_map(|arg| arg.strip_prefix("--touch-smoke-model=").map(PathBuf::from))
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default())
                .join("io.github.sahidhaqqi.projectnera/characters/model-07665e5f50f3b8ac.vrm")
        });
    assert!(
        model.is_file(),
        "Native touch validation needs an imported VRM: {}",
        model.display()
    );
    app.insert_resource(Smoke {
        stage: 0,
        at: 0.0,
        initialized: false,
        custom: false,
        model,
        cursor: default(),
        fx_cursor: default(),
        expected: None,
        collect_step: 0,
        seen: [false; 8],
        last_send: 0.0,
        hold_until: 0.0,
        captures: Vec::new(),
        records: Vec::new(),
        probes: Vec::new(),
        skips: Vec::new(),
        pending_captures: Vec::new(),
        avatar: None,
        seat: None,
        placement: PlacementState::Free,
        lower_body: Vec::new(),
    })
    .add_systems(Last, run.after(crate::pet_feedback::update_particles));
}
fn now(world: &World) -> f32 {
    // Production cooldowns and clip timers share the virtual animation clock.
    world.resource::<Time>().elapsed_secs()
}
fn advance(world: &mut World, stage: u8) {
    let at = now(world);
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.stage = stage;
    smoke.at = at;
    info!(
        "Touch native smoke {} stage {stage}",
        if smoke.custom { "custom" } else { "default" }
    );
}
fn name(world: &World, state: &str) -> String {
    format!(
        "touch-{}-{}-{state}.png",
        if world.resource::<Smoke>().custom {
            "custom"
        } else {
            "default"
        },
        if world.resource::<BodyProportions>().body_scale() < 1.0 {
            "chibi"
        } else {
            "normal"
        }
    )
}
fn capture(world: &mut World, filename: String) {
    let directory = std::env::temp_dir().join("nera-touch-smoke");
    std::fs::create_dir_all(&directory).unwrap();
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(directory.join(&filename)));
    world.resource_mut::<Smoke>().captures.push(filename);
}
fn pointer() -> NeraPointerPosition {
    NeraPointerPosition {
        screen: IVec2::ZERO,
        local: Vec2::ZERO,
    }
}
fn click(world: &mut World, part: BodyPart, label: &str, accepted: bool) {
    if accepted {
        let step = if matches!(part, BodyPart::LeftHand | BodyPart::RightHand) {
            world.resource::<TouchReactionRuntime>().hand_streak() + 1
        } else {
            0
        };
        let mut smoke = world.resource_mut::<Smoke>();
        assert!(smoke.expected.is_none(), "Previous touch was not processed");
        smoke.expected = Some((part, step, label.into()));
    }
    world.write_message(NeraClicked {
        pointer: pointer(),
        body_part: part,
    });
}
fn observe(world: &mut World) {
    let (accepted, fx) = world.resource_scope(|world, mut smoke: Mut<Smoke>| {
        let accepted: Vec<_> = smoke
            .cursor
            .read(world.resource::<Messages<TouchReactionAccepted>>())
            .copied()
            .collect();
        let fx: Vec<_> = smoke
            .fx_cursor
            .read(world.resource::<Messages<PetFx>>())
            .copied()
            .collect();
        (accepted, fx)
    });
    assert!(
        accepted.len() <= 1,
        "One input created multiple authored touch reactions"
    );
    for event in accepted {
        let (part, step, label) = world
            .resource_mut::<Smoke>()
            .expected
            .take()
            .expect("Rejected/cooldown input unexpectedly created a reaction");
        assert_eq!(event.hand_step, step);
        assert_eq!(event.angry_mark, matches!(step, 4 | 7));
        if step == 2 {
            let previous = world.resource::<Smoke>().records.last().unwrap();
            assert_ne!(
                previous["clip"].as_str().unwrap(),
                format!("{:?}", event.clip),
                "The first two normal Hand reactions must not repeat immediately"
            );
        }
        let hand = matches!(part, BodyPart::LeftHand | BodyPart::RightHand);
        if hand {
            let pool: &[TouchReactionClip] = match step {
                1 | 2 => &TouchReactionClip::NORMAL_HAND,
                3 | 4 => &[TouchReactionClip::Dismissive],
                _ => &[TouchReactionClip::Shrugging],
            };
            assert!(pool.contains(&event.clip));
        } else {
            assert!(TouchReactionClip::ARM.contains(&event.clip));
        }
        assert_eq!(
            world.resource::<NeraAnimationState>().visual_state(),
            VisualState::TouchReaction(event.clip)
        );
        let clips = world.resource::<NeraAnimationClips>();
        assert!(
            clips.touch_reaction_playing(event.clip, world.resource::<NeraAnimationPlayback>()),
            "Semantic touch must play its actual authored VRMA"
        );
        let duration = clips
            .touch_reaction_duration(event.clip)
            .unwrap()
            .as_secs_f32();
        let angry = fx
            .iter()
            .filter(|fx| fx.kind == PetFxKind::AngryMark)
            .count();
        assert_eq!(
            angry,
            usize::from(event.angry_mark),
            "AngryMark must occur once at steps 4/7 only"
        );
        let index = event.clip as usize;
        let first = !world.resource::<Smoke>().seen[index];
        let delay = if matches!(
            label.as_str(),
            "torso-hand" | "before-alarm" | "before-replacement"
        ) {
            // These contexts intentionally interrupt the touch at 250 ms.
            0.15
        } else if first && event.clip == TouchReactionClip::ArmStretching {
            3.0_f32.min(duration * 0.5)
        } else if first && event.clip == TouchReactionClip::ShoulderRubbing {
            2.0_f32.min(duration * 0.5)
        } else {
            (duration * 0.4).clamp(0.15, 0.30)
        };
        let at = now(world);
        let screenshot = name(world, &format!("{}-{:?}-step{step}", label, event.clip));
        let mode = format!("{:?}", *world.resource::<AvatarBodyMode>());
        let current = format!(
            "{:?}",
            world
                .resource::<NeraAnimationPlayback>()
                .current_clip
                .unwrap()
        );
        let mut smoke = world.resource_mut::<Smoke>();
        smoke.seen[index] = true;
        smoke.hold_until = at + delay + 0.20;
        let custom = smoke.custom;
        let record = serde_json::json!({ "model": if custom { "custom" } else { "default" },
            "mode":mode, "part":format!("{part:?}"), "clip":format!("{:?}", event.clip),
            "path":event.clip.path(), "hand_step":step, "angry_mark":event.angry_mark,
            "angry_mark_count":angry, "duration_seconds":duration, "playing_entity":current });
        smoke.records.push(record);
        if first || event.angry_mark || !label.starts_with("sequence") {
            smoke
                .pending_captures
                .push((at + delay, screenshot, event.clip));
        }
    }
    let at = now(world);
    let due = world.resource_scope(|_, mut smoke: Mut<Smoke>| {
        let mut due = Vec::new();
        smoke.pending_captures.retain(|(when, name, clip)| {
            if *when <= at {
                due.push((name.clone(), *clip));
                false
            } else {
                true
            }
        });
        due
    });
    for (filename, clip) in due {
        assert_eq!(
            world.resource::<NeraAnimationState>().visual_state(),
            VisualState::TouchReaction(clip),
            "Authored screenshot must still render its accepted touch clip: {filename}"
        );
        assert!(
            world
                .resource::<NeraAnimationClips>()
                .touch_reaction_playing(clip, world.resource::<NeraAnimationPlayback>()),
            "Authored screenshot lost its real VRMA playback: {filename}"
        );
        capture(world, filename);
    }
}
fn ready(world: &mut World) -> bool {
    let loaded = TouchReactionClip::ALL.into_iter().all(|clip| {
        world
            .resource::<NeraAnimationClips>()
            .touch_reaction_duration(clip)
            .is_some()
    });
    loaded
        && world
            .query_filtered::<Entity, (
                With<NeraAvatar>,
                With<Initialized>,
                Without<crate::character::CharacterReveal>,
            )>()
            .single(world)
            .is_ok()
        && !world
            .resource::<crate::character::CharacterImportStatus>()
            .loading
}
fn assert_finite(world: &mut World) {
    for (bone, local, global) in world
        .query_filtered::<(&VrmBone, &Transform, &GlobalTransform), With<NeraLiveBone>>()
        .iter(world)
    {
        assert!(
            local.translation.is_finite()
                && local.rotation.is_finite()
                && local.scale.is_finite()
                && global.to_matrix().is_finite(),
            "Non-finite live touch bone: {}",
            bone.as_str()
        );
    }
}
fn reset(world: &mut World) {
    world.write_message(crate::action_menu::NeraActionMenuCommand::Animation(
        crate::action_menu::AnimationMenuChoice::ResetBasePose,
    ));
}
fn start_collect(world: &mut World, stage: u8) {
    world.resource_mut::<TouchReactionRuntime>().rng = crate::rng::NeraRng::seeded(3);
    let at = now(world);
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.collect_step = 0;
    smoke.seen = [false; 8];
    smoke.last_send = at - 1.0;
    smoke.hold_until = at;
    drop(smoke);
    advance(world, stage);
}
fn collect(world: &mut World) -> bool {
    let at = now(world);
    let smoke = world.resource::<Smoke>();
    if smoke.expected.is_some() || at < smoke.hold_until || at - smoke.last_send < 0.78 {
        return false;
    }
    if smoke.collect_step > 0
        && smoke.collect_step.is_multiple_of(9)
        && smoke.seen.iter().all(|seen| *seen)
    {
        assert_eq!(world.resource::<TouchReactionRuntime>().hand_streak(), 0);
        return true;
    }
    assert!(
        smoke.collect_step < 36,
        "Bounded deterministic selector did not render all eight touch clips"
    );
    let step = smoke.collect_step % 9;
    let part = if step < 7 {
        if step % 2 == 0 {
            BodyPart::LeftHand
        } else {
            BodyPart::RightHand
        }
    } else if step == 7 {
        BodyPart::LeftArm
    } else {
        BodyPart::RightArm
    };
    let label = format!("sequence{}", smoke.collect_step / 9 + 1);
    click(world, part, &label, true);
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.collect_step += 1;
    smoke.last_send = at;
    false
}
fn hit_probes(world: &mut World) {
    let (camera, view) = {
        let mut query = world.query_filtered::<(&Camera, &GlobalTransform), With<NeraMainCamera>>();
        let (camera, view) = query.single(world).unwrap();
        (camera.clone(), *view)
    };
    let positions: std::collections::HashMap<_, _> = world
        .query_filtered::<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>()
        .iter(world)
        .filter_map(|(bone, pose)| {
            camera
                .world_to_viewport(&view, pose.translation())
                .ok()
                .map(|point| (bone.as_str().to_owned(), point))
        })
        .collect();
    let mut targets = Vec::new();
    for side in ["left", "right"] {
        if let Some(hand) = positions.get(&format!("{side}Hand")) {
            targets.push((format!("{side}Hand"), *hand));
        }
        if let (Some(lower), Some(hand)) = (
            positions.get(&format!("{side}LowerArm")),
            positions.get(&format!("{side}Hand")),
        ) {
            targets.push((format!("{side}Arm"), lower.lerp(*hand, 0.50)));
        }
    }
    for (part, point) in targets {
        let mut cursors = Vec::new();
        for y in -2..=2 {
            for x in -2..=2 {
                cursors.push(point + Vec2::new(x as f32, y as f32) * 4.0);
            }
        }
        let hits = crate::interaction::smoke_parts_at_cursor(world, cursors);
        let mapped: Vec<_> = hits
            .iter()
            .map(|hit| hit.map(|part| format!("{part:?}")))
            .collect();
        let expected = match part.as_str() {
            "leftHand" => BodyPart::LeftHand,
            "rightHand" => BodyPart::RightHand,
            "leftArm" => BodyPart::LeftArm,
            _ => BodyPart::RightArm,
        };
        let count = hits.iter().filter(|hit| **hit == Some(expected)).count();
        let mode = format!("{:?}", *world.resource::<AvatarBodyMode>());
        let custom = world.resource::<Smoke>().custom;
        info!("Touch native hit probe {mode} {part}: {count}/25 expected region hits");
        world.resource_mut::<Smoke>().probes.push(serde_json::json!({"model":if custom {"custom"} else {"default"},
            "mode":mode,"target":part,"center":[point.x,point.y],"expected_hits":count,"hits":mapped}));
        // Record the entire actual production result; renderer pose/occlusion may
        // legitimately cover a neutral wrist. Do not force semantic priorities here.
    }
}
fn begin_walk(world: &mut World) {
    world
        .run_system_once(
            |physics: Res<DesktopPhysicsState>,
             environment: Res<DesktopEnvironment>,
             clips: Res<NeraAnimationClips>,
             mut walking: ResMut<WalkingRuntime>,
             mut state: ResMut<NeraAnimationState>| {
                let surface = physics
                    .walking_surface(&environment)
                    .expect("Touch smoke needs native walking support");
                let current = environment.nera_rect.unwrap().left - surface.origin_x;
                let bounds = (
                    surface.bounds.0 - surface.origin_x,
                    surface.bounds.1 - surface.origin_x,
                );
                let target = (if current + 160 < bounds.1 {
                    current + 160
                } else {
                    current - 160
                })
                .clamp(bounds.0, bounds.1);
                walking.cancel(&mut state);
                state.cancel_startup_greeting();
                state.cancel_idle_variation();
                assert!(
                    walking.begin(
                        WalkIntent {
                            surface: surface.key,
                            monitor: environment.monitor.unwrap().handle,
                            target_x: target,
                            bounds,
                            speed: 45.0,
                            scale: 1.0,
                            return_trip: false
                        },
                        current,
                        0.2,
                        &mut state,
                        clips
                            .idle_micro_duration(IdleMicroAction::LookAround)
                            .unwrap()
                    )
                );
            },
        )
        .unwrap();
}
fn move_seat(world: &mut World, point: IVec2) {
    let env = world.resource::<DesktopEnvironment>();
    let rect = env.nera_rect.unwrap();
    let seat = env.seat_probe_global().unwrap();
    let hwnd = env.nera_hwnd.unwrap();
    crate::platform::move_window(
        hwnd,
        PhysicalPoint::new(rect.left + point.x - seat.x, rect.top + point.y - seat.y),
    )
    .unwrap();
    world.resource_mut::<DesktopEnvironment>().nera_rect = crate::platform::window_rect(hwnd);
}
fn sit(world: &mut World, window: bool) -> bool {
    let env = world.resource::<DesktopEnvironment>();
    let (point, placement) = if window {
        let monitor = env.monitor.unwrap();
        let hwnd = env.nera_hwnd.unwrap();
        let target = env.discovered_windows.iter().find(|candidate| {
            let x = (candidate.rect.left + candidate.rect.right) / 2;
            candidate.rect.top > monitor.work_area.top + 160
                && candidate.rect.top < monitor.work_area.bottom - 80
                && crate::platform::window_at_point(
                    PhysicalPoint::new(x, candidate.rect.top + 1),
                    hwnd,
                    &env.discovered_windows,
                ) == Some(candidate.handle)
        });
        let Some(target) = target else {
            world
                .resource_mut::<Smoke>()
                .skips
                .push("No external window sitting fixture".into());
            return false;
        };
        (
            IVec2::new((target.rect.left + target.rect.right) / 2, target.rect.top),
            PlacementState::PendingWindow {
                target: target.handle,
                settle_frames: 1,
            },
        )
    } else {
        let Some(bar) = env
            .taskbar
            .taskbars
            .iter()
            .find(|bar| bar.edge == TaskbarEdge::Bottom)
        else {
            world
                .resource_mut::<Smoke>()
                .skips
                .push("No bottom taskbar".into());
            return false;
        };
        (
            IVec2::new((bar.rect.left + bar.rect.right) / 2, bar.rect.top),
            PlacementState::PendingTaskbar {
                monitor: bar.monitor,
                edge: bar.edge,
                settle_frames: 1,
            },
        )
    };
    move_seat(world, point);
    let mut env = world.resource_mut::<DesktopEnvironment>();
    env.set_attached_anchor(AvatarAnchor::Seat);
    env.placement = placement;
    true
}
fn remember_seat(world: &mut World) {
    let env = world.resource::<DesktopEnvironment>();
    let seat = env.seat_probe_global().unwrap();
    let placement = env.placement;
    let lower: Vec<_> = world
        .query_filtered::<(Entity, &VrmBone, &Transform), With<NeraLiveBone>>()
        .iter(world)
        .filter(|(_, bone, _)| {
            matches!(
                bone.as_str(),
                "hips"
                    | "leftUpperLeg"
                    | "rightUpperLeg"
                    | "leftLowerLeg"
                    | "rightLowerLeg"
                    | "leftFoot"
                    | "rightFoot"
                    | "leftToes"
                    | "rightToes"
            )
        })
        .map(|(entity, _, local)| (entity, *local))
        .collect();
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.seat = Some(IVec2::new(seat.x, seat.y));
    smoke.placement = placement;
    smoke.lower_body = lower;
}
fn assert_seat(world: &World, legs: bool) {
    let env = world.resource::<DesktopEnvironment>();
    let smoke = world.resource::<Smoke>();
    let seat = env.seat_probe_global().unwrap();
    assert_eq!(env.placement, smoke.placement);
    assert!(
        (IVec2::new(seat.x, seat.y) - smoke.seat.unwrap())
            .abs()
            .max_element()
            <= 2,
        "Touch detached native seat"
    );
    if legs {
        for (entity, base) in &smoke.lower_body {
            let local = world.get::<Transform>(*entity).unwrap();
            assert!(
                local.translation.abs_diff_eq(base.translation, 0.0001)
                    && local.rotation.abs_diff_eq(base.rotation, 0.0001)
                    && local.scale.abs_diff_eq(base.scale, 0.0001),
                "Touch changed calibrated seated lower body"
            );
        }
    }
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
fn cloud_ready(world: &World, mode: CloudMode) -> bool {
    let cloud = world.resource::<CloudRuntime>();
    cloud.mode() == Some(mode)
        && world.resource::<CloudSeatPose>().is_ready_for(cloud)
        && world.resource::<NeraAnimationState>().visual_state() == VisualState::SittingLoop
}
fn report(world: &mut World) {
    let smoke = world.resource::<Smoke>();
    let report = serde_json::json!({"default_and_imported_custom":true,"default_normal_and_chibi_all_eight":true,
        "custom_chibi_all_eight":true,"elapsed_seconds":world.resource::<Time<Real>>().elapsed_secs(),
        "animation_elapsed_seconds":now(world),"captures":smoke.captures,"reactions":smoke.records,
        "hit_probes":smoke.probes,"skips":smoke.skips,
        "inputs":"production semantic messages; real HWND, VRMA, bones and renderer; deterministic selector seed",
        "manual_pointer_validation":false});
    let directory = std::env::temp_dir().join("nera-touch-smoke");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
    info!(
        "TOUCH_SMOKE_PASS native all-eight/7-streak/FX/left-right/cooldown/drag/head-torso/walk/seat/cloud/alarm/replacement; skips={:?}",
        smoke.skips
    );
}
fn run(world: &mut World) {
    assert!(
        world.resource::<Time<Real>>().elapsed_secs() < 190.0,
        "Native touch smoke exceeded its bounded lifetime"
    );
    observe(world);
    assert_finite(world);
    let smoke = world.resource::<Smoke>();
    let stage = smoke.stage;
    let elapsed = now(world) - smoke.at;
    let free_hold = now(world) >= smoke.hold_until;
    match stage {
        0 if ready(world)
            && world.resource::<DesktopPhysicsState>().support.is_some()
            && !world.resource::<DesktopPhysicsState>().falling =>
        {
            if !world.resource::<Smoke>().initialized {
                let avatar = world
                    .query_filtered::<Entity, With<NeraAvatar>>()
                    .single(world)
                    .unwrap();
                world.resource_mut::<Smoke>().avatar = Some(avatar);
                world.resource_mut::<Smoke>().initialized = true;
                let mut settings = world.resource_mut::<NeraSettings>();
                settings.walking_enabled = false;
                settings.follow_mouse = false;
                settings.follow_eyes = false;
                settings.follow_head = false;
                reset(world);
                advance(world, 0);
                return;
            }
            if elapsed > 0.35
                && world.resource::<NeraAnimationState>().visual_state() == VisualState::Idle
            {
                hit_probes(world);
                start_collect(world, 1);
            }
        }
        1 if collect(world) => {
            world.write_message(ToggleBodyMode);
            reset(world);
            advance(world, 2);
        }
        2 if elapsed > 0.35
            && world.resource::<BodyProportions>().body_scale() < 1.0
            && world.resource::<NeraAnimationState>().visual_state() == VisualState::Idle =>
        {
            hit_probes(world);
            start_collect(world, 3);
        }
        3 | 33 if collect(world) => {
            click(world, BodyPart::Head, "head", false);
            advance(world, 4);
        }
        4 if elapsed > 0.20 => {
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::HitHead
            );
            capture(world, name(world, "head-hit"));
            click(world, BodyPart::LeftHand, "head-hand", true);
            advance(world, 5);
        }
        5 if elapsed > 0.25 => {
            assert_eq!(world.resource::<TouchReactionRuntime>().hand_streak(), 1);
            click(world, BodyPart::RightHand, "cooldown-rejected", false);
            advance(world, 6);
        }
        6 if elapsed > 0.15 => {
            assert_eq!(world.resource::<TouchReactionRuntime>().hand_streak(), 1);
            click(world, BodyPart::Torso, "torso", false);
            advance(world, 7);
        }
        7 if elapsed > 0.15 => {
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::HitChest
            );
            assert_eq!(world.resource::<TouchReactionRuntime>().hand_streak(), 0);
            capture(world, name(world, "torso-hit"));
            advance(world, 71);
        }
        71 if elapsed > 0.25 => {
            // HitChest is only 333 ms; its capture must precede expiry,
            // while the next Hand still waits through its 650 ms cooldown.
            click(world, BodyPart::RightHand, "torso-hand", true);
            advance(world, 8);
        }
        8 if elapsed > 0.25 => {
            world.resource_mut::<DesktopPetInteraction>().state = InteractionState::Dragging;
            world.write_message(NeraDragStarted {
                screen: IVec2::ZERO,
                body_part: BodyPart::RightHand,
            });
            advance(world, 9);
        }
        9 if elapsed > 0.20 => {
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::Floating
            );
            assert_eq!(world.resource::<TouchReactionRuntime>().hand_streak(), 0);
            click(world, BodyPart::RightArm, "drag-rejected", false);
            capture(world, name(world, "drag"));
            advance(world, 10);
        }
        10 if elapsed > 0.20 => {
            world
                .resource_mut::<DesktopPetInteraction>()
                .reset_gesture_state();
            world.write_message(NeraReleased {
                pointer: pointer(),
                body_part: BodyPart::RightHand,
                was_dragging: true,
            });
            advance(world, 11);
        }
        11 if elapsed > 0.30 => {
            assert!(!matches!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::TouchReaction(_)
            ));
            world.write_message(NeraHeadPatted { pointer: pointer() });
            advance(world, 12);
        }
        12 if elapsed > 0.30 => {
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::HeadStroke
            );
            capture(world, name(world, "headpat"));
            reset(world);
            advance(world, 13);
        }
        13 if elapsed > 0.35
            && world.resource::<NeraAnimationState>().visual_state() == VisualState::Idle
            && world.resource::<DesktopPhysicsState>().support.is_some() =>
        {
            begin_walk(world);
            advance(world, 14);
        }
        14 if world.resource::<NeraAnimationState>().visual_state() == VisualState::Walking => {
            capture(world, name(world, "walking-before-touch"));
            click(world, BodyPart::LeftHand, "walking-hand", true);
            advance(world, 15);
        }
        15 if elapsed > 0.70 => {
            assert!(!world.resource::<WalkingRuntime>().active());
            reset(world);
            if sit(world, false) {
                advance(world, 16);
            } else {
                advance(world, 18);
            }
        }
        16 if world.resource::<TouchSeatPose>().is_ready_for(
            world.resource::<DesktopEnvironment>().placement,
            world
                .resource::<NeraAnimationState>()
                .selected_sitting_pose(),
        ) && world.resource::<NeraAnimationState>().visual_state()
            == VisualState::SittingLoop =>
        {
            remember_seat(world);
            click(world, BodyPart::LeftHand, "taskbar-hand", true);
            advance(world, 17);
        }
        17 if elapsed > 0.35 => {
            assert_seat(world, true);
            capture(world, name(world, "taskbar-hand"));
            click(world, BodyPart::RightArm, "taskbar-arm", true);
            advance(world, 18);
        }
        18 if elapsed > 0.40 && free_hold => {
            if world.resource::<Smoke>().seat.is_some() {
                assert_seat(world, true);
            }
            capture(world, name(world, "taskbar-arm"));
            world.resource_mut::<DesktopEnvironment>().detach_nera();
            reset(world);
            if sit(world, true) {
                advance(world, 19);
            } else {
                floating_seat(world);
                world.write_message(CloudCommand::Summon(CloudMode::Ride));
                advance(world, 22);
            }
        }
        19 if world.resource::<TouchSeatPose>().is_ready_for(
            world.resource::<DesktopEnvironment>().placement,
            world
                .resource::<NeraAnimationState>()
                .selected_sitting_pose(),
        ) && world.resource::<NeraAnimationState>().visual_state()
            == VisualState::SittingLoop =>
        {
            remember_seat(world);
            click(world, BodyPart::RightHand, "window-hand", true);
            advance(world, 20);
        }
        20 if elapsed > 0.35 => {
            assert_seat(world, true);
            capture(world, name(world, "window-hand"));
            click(world, BodyPart::Torso, "seated-torso", false);
            advance(world, 21);
        }
        21 if elapsed > 0.25 => {
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::DodgeWhileSitting
            );
            assert_seat(world, false);
            capture(world, name(world, "seated-torso-dodge"));
            floating_seat(world);
            world.write_message(CloudCommand::Summon(CloudMode::Ride));
            advance(world, 22);
        }
        22 if elapsed > 0.40 && cloud_ready(world, CloudMode::Ride) => {
            remember_seat(world);
            click(world, BodyPart::LeftHand, "ride-hand", true);
            advance(world, 23);
        }
        23 if elapsed > 0.35 => {
            assert_seat(world, true);
            assert_eq!(
                world.resource::<CloudRuntime>().mode(),
                Some(CloudMode::Ride)
            );
            capture(world, name(world, "ride-hand"));
            click(world, BodyPart::RightArm, "ride-arm", true);
            advance(world, 24);
        }
        24 if elapsed > 0.40 && free_hold => {
            assert_seat(world, true);
            capture(world, name(world, "ride-arm"));
            world.write_message(CloudCommand::Summon(CloudMode::Rest));
            advance(world, 25);
        }
        25 if elapsed > 0.40 && cloud_ready(world, CloudMode::Rest) => {
            remember_seat(world);
            for part in [
                BodyPart::Head,
                BodyPart::Torso,
                BodyPart::LeftHand,
                BodyPart::RightHand,
                BodyPart::LeftArm,
                BodyPart::RightArm,
            ] {
                click(world, part, "rest-rejected", false);
            }
            world.write_message(NeraHeadPatted { pointer: pointer() });
            advance(world, 26);
        }
        26 if elapsed > 0.40 => {
            assert!(cloud_ready(world, CloudMode::Rest));
            assert_eq!(world.resource::<TouchReactionRuntime>().hand_streak(), 0);
            assert_seat(world, false);
            capture(world, name(world, "rest-touch-suppressed"));
            world.write_message(CloudCommand::Dismiss);
            reset(world);
            advance(world, 27);
        }
        27 if elapsed > 0.40
            && world.resource::<NeraAnimationState>().visual_state() == VisualState::Idle =>
        {
            click(world, BodyPart::LeftHand, "before-alarm", true);
            advance(world, 28);
        }
        28 if elapsed > 0.25 => {
            world.write_message(PresentationCommand::EnterAttention(AttentionKind::Alarm));
            world.write_message(AlarmAnimationCommand::Start);
            advance(world, 29);
        }
        29 if elapsed > 0.35 && world.resource::<PresentationState>().is_attention() => {
            assert!(!matches!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::TouchReaction(_)
            ));
            click(world, BodyPart::RightHand, "alarm-rejected", false);
            click(world, BodyPart::LeftArm, "alarm-rejected", false);
            advance(world, 30);
        }
        30 if elapsed > 0.40 => {
            assert_eq!(world.resource::<TouchReactionRuntime>().hand_streak(), 0);
            capture(world, name(world, "alarm-touch-suppressed"));
            world.write_message(PresentationCommand::ExitAttention);
            world.write_message(AlarmAnimationCommand::Stop);
            advance(world, 31);
        }
        31 if elapsed > 0.40 && world.resource::<PresentationState>().is_desktop_pet() => {
            click(world, BodyPart::LeftHand, "before-replacement", true);
            advance(world, 36);
        }
        36 if elapsed > 0.25 => {
            assert!(world.resource::<Smoke>().expected.is_none());
            assert!(matches!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::TouchReaction(_)
            ));
            let avatar = world
                .query_filtered::<Entity, With<NeraAvatar>>()
                .single(world)
                .unwrap();
            world.resource_mut::<Smoke>().avatar = Some(avatar);
            if world.resource::<Smoke>().custom {
                world.write_message(crate::character::RestoreDefaultCharacter);
            } else {
                let model = world.resource::<Smoke>().model.clone();
                world.write_message(crate::character::ImportCharacter(model));
            }
            advance(world, 32);
        }
        32 if ready(world) && elapsed > 0.35 => {
            let avatar = world
                .query_filtered::<Entity, With<NeraAvatar>>()
                .single(world)
                .unwrap();
            if Some(avatar) == world.resource::<Smoke>().avatar {
                return;
            }
            assert_eq!(world.resource::<TouchReactionRuntime>().hand_streak(), 0);
            assert!(!matches!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::TouchReaction(_)
            ));
            assert_eq!(*world.resource::<AvatarBodyMode>(), AvatarBodyMode::Chibi);
            assert!(world.resource::<BodyProportions>().body_scale() < 1.0);
            capture(world, name(world, "replacement-clears-touch"));
            if !world.resource::<Smoke>().custom {
                world.resource_mut::<Smoke>().custom = true;
                hit_probes(world);
                start_collect(world, 33);
            } else {
                advance(world, 34);
            }
        }
        34 if elapsed > 0.35 => {
            capture(world, name(world, "final-settled"));
            report(world);
            advance(world, 35);
        }
        35 if elapsed > 0.85 => {
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
}
