//! Opt-in native HWND/render validation using an isolated debug settings profile.
use super::*;
use crate::interaction::{BodyPart, NeraDragStarted, NeraPointerPosition, NeraReleased};
use bevy::{
    ecs::system::RunSystemOnce,
    render::view::screenshot::{Screenshot, save_to_disk},
};

#[derive(Resource, Default)]
struct Smoke {
    stage: u8,
    at: f32,
    released_seat: Option<Vec2>,
    replaced: bool,
    blink_reference: Option<std::path::PathBuf>,
}
pub(crate) fn install(app: &mut App) {
    let blink_reference = std::env::args().find_map(|arg| {
        arg.strip_prefix("--cloud-smoke-blink-model=")
            .map(std::path::PathBuf::from)
    });
    app.insert_resource(Smoke {
        blink_reference,
        ..default()
    })
    .add_systems(Last, run.after(crate::pet_feedback::update_particles));
}
fn advance(world: &mut World, stage: u8) {
    let now = world.resource::<Time<Real>>().elapsed_secs();
    let mut smoke = world.resource_mut::<Smoke>();
    smoke.stage = stage;
    smoke.at = now;
}
fn screenshot(world: &mut World, name: &str) {
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    let directory = std::env::temp_dir().join("nera-feature-smoke");
    std::fs::create_dir_all(&directory).unwrap();
    world
        .commands()
        .spawn(Screenshot::window(window))
        .observe(save_to_disk(directory.join(name)));
}
fn position(world: &mut World, point: Vec2) {
    let environment = world.resource::<DesktopEnvironment>();
    let rect = environment.nera_rect.unwrap();
    let seat = environment.seat_probe_global().unwrap();
    let hwnd = environment.nera_hwnd.unwrap();
    crate::platform::move_window(
        hwnd,
        crate::platform::PhysicalPoint::new(
            rect.left + point.x.round() as i32 - seat.x,
            rect.top + point.y.round() as i32 - seat.y,
        ),
    )
    .unwrap();
    world.resource_mut::<DesktopEnvironment>().nera_rect = crate::platform::window_rect(hwnd);
}
fn begin_drag(world: &mut World) {
    world
        .run_system_once(
            |mut cloud: ResMut<CloudRuntime>,
             mut environment: ResMut<DesktopEnvironment>,
             mut physics: ResMut<DesktopPhysicsState>,
             mut animation: ResMut<NeraAnimationState>,
             mut sfx: MessageWriter<PetSfx>,
             mut fx: MessageWriter<PetFx>| {
                begin_cloud_drag(
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
        pointer: NeraPointerPosition {
            screen: IVec2::ZERO,
            local: Vec2::ZERO,
        },
        body_part: BodyPart::Torso,
        was_dragging: true,
    });
}
fn assert_head_right(world: &mut World) {
    let mut head = None;
    let mut hips = None;
    for (bone, global) in world.query_filtered::<(&bevy_vrm1::prelude::VrmBone, &GlobalTransform), With<crate::interaction::NeraLiveBone>>().iter(world) {
        match bone.as_str() {
            "head" => head = Some(global.translation()),
            "hips" => hips = Some(global.translation()),
            _ => {}
        }
    }
    assert!(
        head.unwrap().x > hips.unwrap().x + 0.1,
        "sleeping head must lie to the right of the pelvis"
    );
}
fn assert_sleeping_arms(world: &mut World) {
    let mut head = None;
    let mut hands = Vec::new();
    for (bone, global) in world.query_filtered::<(&bevy_vrm1::prelude::VrmBone, &GlobalTransform), With<crate::interaction::NeraLiveBone>>().iter(world) {
        match bone.as_str() {
            "head" => head = Some(global.translation()),
            "leftHand" | "rightHand" => hands.push(global.translation()),
            _ => {}
        }
    }
    assert_eq!(hands.len(), 2);
    for hand in hands {
        assert!(
            hand.x < head.unwrap().x - 0.12,
            "sleeping hand must stay away from the face"
        );
    }
    assert!(world.resource::<sleep_arms::SleepingArms>().applied_count() <= 8);
    assert!(world.resource::<sleep_arms::SleepingArms>().applied_count() >= 6);
}
fn assert_closed_eyelid_morph(world: &mut World) {
    use bevy_vrm1::prelude::{ExpressionEntityMap, ExpressionOverride};
    let map = world
        .query_filtered::<&ExpressionEntityMap, With<NeraAvatar>>()
        .single(world)
        .unwrap();
    let blink = map
        .0
        .iter()
        .find(|(name, _)| name.0 == "blink")
        .map(|(_, entity)| *entity)
        .expect("reference model must supply a real Blink");
    assert!(
        world
            .get::<ExpressionOverride>(blink)
            .is_some_and(|weight| weight.0 > 0.99),
        "rest sleep must close the eyelid expression completely"
    );
    assert!(
        world
            .query::<&bevy::mesh::morph::MorphWeights>()
            .iter(world)
            .any(|weights| weights.weights().iter().any(|weight| *weight > 0.99)),
        "closed eyelid intent must reach rendered morph weights"
    );
}
fn touch_sleeping_pet(world: &mut World) {
    let pointer = NeraPointerPosition {
        screen: IVec2::ZERO,
        local: Vec2::ZERO,
    };
    for body_part in [
        BodyPart::Head,
        BodyPart::Torso,
        BodyPart::LeftArm,
        BodyPart::RightArm,
        BodyPart::LeftHand,
        BodyPart::RightHand,
        BodyPart::LeftLeg,
        BodyPart::RightLeg,
        BodyPart::LeftFoot,
        BodyPart::RightFoot,
    ] {
        world.write_message(crate::interaction::NeraClicked { pointer, body_part });
    }
    for _ in 0..100 {
        world.write_message(crate::interaction::NeraHeadPatted { pointer });
    }
}
fn run(world: &mut World) {
    let elapsed = world.resource::<Time<Real>>().elapsed_secs();
    let stage = world.resource::<Smoke>().stage;
    let since = elapsed - world.resource::<Smoke>().at;
    let count = world
        .query_filtered::<Entity, With<crate::pet_feedback::PetParticle>>()
        .iter(world)
        .count();
    assert!(count <= crate::pet_feedback::MAX_PARTICLES);
    match stage {
        0 if elapsed > 3.0
            && !world
                .resource::<crate::startup_entrance::StartupEntrance>()
                .blocks_position_save() =>
        {
            world
                .resource_mut::<crate::settings::NeraSettings>()
                .follow_mouse = false;
            let bounds = world
                .resource::<DesktopEnvironment>()
                .monitor
                .unwrap()
                .work_area;
            position(
                world,
                Vec2::new(
                    (bounds.left + bounds.right) as f32 * 0.5,
                    bounds.top as f32 + bounds.height() as f32 * 0.55,
                ),
            );
            world.write_message(crate::action_menu::NeraActionMenuCommand::Cloud(
                CloudCommand::Summon(CloudMode::Rest),
            ));
            advance(world, 1);
        }
        1 if since > 8.0 => {
            assert_eq!(
                world.resource::<CloudRuntime>().mode(),
                Some(CloudMode::Rest)
            );
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::SittingLoop
            );
            assert!(!world.resource::<DesktopPhysicsState>().falling);
            let avatar = world
                .query_filtered::<Entity, With<NeraAvatar>>()
                .single(world)
                .unwrap();
            assert!(world.get::<bevy_vrm1::prelude::LookAt>(avatar).is_none());
            assert!(
                world
                    .get::<bevy_vrm1::vrm::body_tracking::BodyTracking>(avatar)
                    .is_none()
            );
            assert!(
                world
                    .resource::<crate::animation::NeraAnimationClips>()
                    .sleeping_idle_playing(
                        world.resource::<crate::animation::NeraAnimationPlayback>()
                    ),
                "Rest must play the real Sleeping Idle after a complete loop"
            );
            assert_head_right(world);
            assert_sleeping_arms(world);
            assert!(
                world
                    .resource::<CloudSeatPose>()
                    .is_ready_for(world.resource::<CloudRuntime>())
            );
            screenshot(world, "cloud-rest.png");
            touch_sleeping_pet(world);
            advance(world, 2);
        }
        2 if since > 0.15 => {
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::SittingLoop,
                "sleep must ignore headpat and every body-part click"
            );
            assert_sleeping_arms(world);
            screenshot(world, "cloud-rest-touch.png");
            advance(world, 3);
        }
        3 if since > 0.8 => {
            world.write_message(CloudCommand::Dismiss);
            advance(world, 22);
        }
        22 if since > 0.4 => {
            assert!(!world.resource::<CloudRuntime>().owns_placement());
            world.write_message(CloudCommand::Summon(CloudMode::Rest));
            advance(world, 20);
        }
        20 if since > 1.0 => {
            assert_eq!(
                world.resource::<CloudRuntime>().rest_pose(),
                Some(RestPose::Seated)
            );
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::SittingLoop
            );
            assert!(
                !world
                    .resource::<crate::animation::NeraAnimationClips>()
                    .sleeping_idle_playing(
                        world.resource::<crate::animation::NeraAnimationPlayback>()
                    )
            );
            assert_closed_eyelid_morph(world);
            assert_eq!(
                world.resource::<sleep_arms::SleepingArms>().applied_count(),
                0
            );
            screenshot(world, "cloud-rest-seated.png");
            touch_sleeping_pet(world);
            advance(world, 21);
        }
        21 if since > 0.15 => {
            assert_eq!(
                world.resource::<CloudRuntime>().rest_pose(),
                Some(RestPose::Seated)
            );
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::SittingLoop
            );
            begin_drag(world);
            assert!(!world.resource::<CloudRuntime>().owns_placement());
            advance(world, 4);
        }
        4 if since > 0.6 => {
            release(world);
            advance(world, 5);
        }
        5 if since > 3.0 => {
            assert!(!world.resource::<CloudRuntime>().owns_placement());
            assert_eq!(
                world
                    .query_filtered::<Entity, With<visual::CloudVisual>>()
                    .iter(world)
                    .count(),
                0
            );
            world.write_message(CloudCommand::Summon(CloudMode::Ride));
            advance(world, 6);
        }
        6 if since > 0.6 => {
            assert!(world.resource::<CloudRuntime>().is_riding());
            begin_drag(world);
            let bounds = world
                .resource::<DesktopEnvironment>()
                .monitor
                .unwrap()
                .work_area;
            position(
                world,
                Vec2::new(
                    (bounds.left + bounds.right) as f32 * 0.5,
                    bounds.top as f32 + bounds.height() as f32 * 0.40,
                ),
            );
            advance(world, 7);
        }
        7 if since > 0.3 => {
            release(world);
            let seat = world.resource::<CloudRuntime>().attachment().unwrap().seat;
            world.resource_mut::<Smoke>().released_seat = Some(seat);
            world.write_message(crate::interaction::NeraThrow {
                velocity: Vec2::splat(2000.0),
            });
            advance(world, 8);
        }
        8 if since > 1.5 => {
            let cloud = world.resource::<CloudRuntime>();
            assert!(cloud.is_riding());
            assert_eq!(
                cloud.attachment().unwrap().seat,
                world.resource::<Smoke>().released_seat.unwrap()
            );
            let physics = world.resource::<DesktopPhysicsState>();
            assert!(!physics.falling && !physics.throwing && physics.support.is_none());
            screenshot(world, "cloud-ride.png");
            begin_drag(world);
            let work = world
                .resource::<DesktopEnvironment>()
                .monitor
                .unwrap()
                .work_area;
            position(
                world,
                Vec2::new(
                    (work.left + work.right) as f32 * 0.5,
                    work.bottom as f32 + 120.0,
                ),
            );
            advance(world, 9);
        }
        9 if since > 0.3 => {
            release(world);
            world
                .run_system_once(
                    |cloud: Res<CloudRuntime>,
                     environment: Res<DesktopEnvironment>,
                     settings: Res<crate::settings::NeraSettings>,
                     camera: Query<
                        (&Camera, &GlobalTransform),
                        With<crate::avatar::NeraMainCamera>,
                    >,
                     children: Query<&Children>,
                     meshes: Query<
                        (&bevy::camera::primitives::Aabb, &GlobalTransform),
                        With<Mesh3d>,
                    >,
                     visuals: Query<(&visual::CloudVisual, &GlobalTransform)>| {
                        let a = cloud.attachment().unwrap();
                        let (camera, view) = camera.single().unwrap();
                        let visible = bounds::project_mounted_bounds(
                            a.avatar,
                            a.visual,
                            a.seat,
                            &environment,
                            camera,
                            view,
                            settings.character_scale(),
                            &children,
                            &meshes,
                            &visuals,
                        )
                        .unwrap();
                        let screen = environment.monitor.unwrap().bounds;
                        assert!(
                            a.seat.y <= screen.bottom as f32 + 1.0,
                            "ride seat below monitor: {a:?}"
                        );
                        assert!(visible.min.y >= screen.top as f32 - 1.0);
                        assert!(visible.min.x >= screen.left as f32 - 1.0);
                        assert!(visible.max.x <= screen.right as f32 + 1.0);
                    },
                )
                .unwrap();
            advance(world, 10);
        }
        10 if since > 1.0 => {
            assert!(world.resource::<CloudRuntime>().is_riding());
            let avatar = world
                .query_filtered::<Entity, With<NeraAvatar>>()
                .single(world)
                .unwrap();
            assert!(world.get::<bevy_vrm1::prelude::LookAt>(avatar).is_some());
            screenshot(world, "cloud-ride-moved.png");
            world.write_message(crate::interaction::NeraHeadPatted {
                pointer: NeraPointerPosition {
                    screen: IVec2::ZERO,
                    local: Vec2::ZERO,
                },
            });
            advance(world, 18);
        }
        18 if since > 0.35 => {
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                crate::animation::VisualState::HeadStroke
            );
            assert!(
                world
                    .resource::<crate::animation::NeraAnimationClips>()
                    .head_stroke_playing(
                        world.resource::<crate::animation::NeraAnimationPlayback>()
                    )
            );
            let face = world
                .query_filtered::<&crate::expression::face::AutomaticFace, With<NeraAvatar>>()
                .single(world)
                .unwrap();
            assert_eq!(
                face.current,
                Some((crate::expression::NeraExpressionPreset::Timid, 1.0))
            );
            assert_eq!(
                count, 1,
                "headpat begins with one heart; later pulses alternate"
            );
            assert!(world.resource::<CloudRuntime>().is_riding());
            screenshot(world, "headpat-ride.png");
            eprintln!(
                "HEADPAT_SMOKE_PASS ShakeHeadNo1/Timid/hearts (optional SFX dispatch covered by unit tests)"
            );
            world.write_message(CloudCommand::Dismiss);
            advance(world, 11);
        }
        11 if since > 0.15 => {
            assert!(!world.resource::<CloudRuntime>().owns_placement());
            let physics = world.resource::<DesktopPhysicsState>();
            assert!(
                physics.falling || physics.support.is_some(),
                "dismissed ride must resume native gravity/support; an edge-clamped mount may already land"
            );
            advance(world, 12);
        }
        12 if since > 3.0 => {
            let physics = world.resource::<DesktopPhysicsState>();
            assert!(!physics.falling && physics.support.is_some());
            assert!(
                world
                    .resource::<crate::settings::NeraSettings>()
                    .gravity_enabled
            );
            assert_eq!(count, 0, "short-lived particles must not accumulate");
            world.write_message(crate::interaction::NeraHeadPatted {
                pointer: NeraPointerPosition {
                    screen: IVec2::ZERO,
                    local: Vec2::ZERO,
                },
            });
            advance(world, 23);
        }
        23 if since > 0.35 => {
            assert_eq!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::HeadStroke
            );
            crate::avatar::assert_smoke_ground_contact(world);
            screenshot(world, "headpat-grounded.png");
            eprintln!("GROUNDING_SMOKE_PASS evaluated_headpat_foot_matches_native_support=true");
            world.write_message(CloudCommand::Summon(CloudMode::Ride));
            advance(world, 13);
        }
        13 if since > 0.7 => {
            assert!(world.resource::<CloudRuntime>().is_riding());
            let path = world
                .resource::<Smoke>()
                .blink_reference
                .clone()
                .unwrap_or_else(|| crate::nera_asset_file("vrm/nera0.0.vrm"));
            world.write_message(crate::character::ImportCharacter(path));
            advance(world, 14);
        }
        14 if since > 3.0
            && !world
                .resource::<crate::character::CharacterImportStatus>()
                .loading
            && !world.resource::<CloudRuntime>().owns_placement() =>
        {
            assert!(
                !world.resource::<CloudRuntime>().owns_placement(),
                "replacement must clean the old attachment"
            );
            world.resource_mut::<Smoke>().replaced = true;
            world.write_message(CloudCommand::Summon(CloudMode::Rest));
            advance(world, 15);
        }
        15 if since > 2.0 => {
            assert!(world.resource::<Smoke>().replaced);
            assert_eq!(
                world.resource::<CloudRuntime>().mode(),
                Some(CloudMode::Rest)
            );
            assert!(
                world
                    .resource::<crate::animation::NeraAnimationClips>()
                    .sleeping_idle_playing(
                        world.resource::<crate::animation::NeraAnimationPlayback>()
                    ),
                "replacement rig must lazily retarget Sleeping Idle"
            );
            assert_head_right(world);
            assert_sleeping_arms(world);
            if world.resource::<Smoke>().blink_reference.is_some() {
                assert_closed_eyelid_morph(world);
                screenshot(world, "cloud-rest-vrm0-blink.png");
            } else {
                screenshot(world, "cloud-rest-vrm0.png");
            }
            world.write_message(crate::presentation::PresentationCommand::EnterAttention(
                crate::presentation::AttentionKind::Alarm,
            ));
            advance(world, 16);
        }
        16 if since > 1.0 => {
            assert!(!world.resource::<CloudRuntime>().owns_placement());
            world.write_message(crate::presentation::PresentationCommand::ExitAttention);
            advance(world, 17);
        }
        17 if since > 2.0 => {
            assert!(world.resource::<PresentationState>().is_desktop_pet());
            assert!(!world.resource::<CloudRuntime>().owns_placement());
            assert_eq!(
                world.resource::<NeraAnimationState>().base_pose(),
                crate::animation::BasePose::Idle
            );
            assert_eq!(count, 0);
            info!(
                "CLOUD_SMOKE_PASS rest/ride/drag/freefall/replacement/attention/particle_cleanup"
            );
            world.write_message(AppExit::Success);
            advance(world, 18);
        }
        _ => {}
    }
    assert!(
        elapsed < 65.0,
        "Cloud native smoke timed out at stage {stage}"
    );
}
