use super::*;
use bevy::ecs::system::RunSystemOnce;

fn app() -> App {
    let mut app = App::new();
    app.init_resource::<LivingPointer>()
        .add_message::<PetFx>()
        .add_message::<PetSfx>()
        .init_resource::<PetBehavior>()
        .init_resource::<NeraAnimationState>();
    app
}
fn arm() -> Arm {
    Arm {
        upper: Entity::from_raw_u32(1).unwrap(),
        lower: Entity::from_raw_u32(2).unwrap(),
        hand: Entity::from_raw_u32(3).unwrap(),
        shoulder: Vec2::splat(100.0),
        reach: 100.0,
    }
}
fn start(app: &mut App) {
    app.world_mut().resource_mut::<LivingPointer>().point = Some(Vec2::splat(140.0));
    let brain = &mut app.world_mut().resource_mut::<PetBehavior>().living;
    brain.cursor = Some(CursorRoutine {
        arm: arm(),
        phase: CursorPhase::Notice,
        elapsed: 0.0,
        target: Vec2::splat(140.0),
        release_from: 0.0,
    });
    phase_animation(
        &mut app.world_mut().resource_mut::<NeraAnimationState>(),
        CursorPhase::Notice,
    );
}
fn tick(app: &mut App, dt: f32) {
    app.world_mut()
        .run_system_once(
            move |mut brain: ResMut<PetBehavior>,
                  mut animation: ResMut<NeraAnimationState>,
                  mut context: LivingContext| {
                brain.living.tick_cursor(dt, &mut context, &mut animation);
            },
        )
        .unwrap();
}
#[test]
fn cursor_notice_reach_catch_release_returns_to_idle() {
    let mut app = app();
    start(&mut app);
    tick(&mut app, 0.76);
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::IdleMicro(IdleMicroAction::CursorReach)
    );
    tick(&mut app, 1.11);
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::IdleMicro(IdleMicroAction::CursorCatch)
    );
    assert_eq!(app.world().resource::<Messages<PetFx>>().len(), 1);
    tick(&mut app, 0.56);
    tick(&mut app, 0.66);
    assert!(
        app.world()
            .resource::<PetBehavior>()
            .cursor_pose()
            .is_none()
    );
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::Idle
    );
}
#[test]
fn escaping_pointer_retracts_from_current_reach_without_a_pose_jump() {
    let mut app = app();
    start(&mut app);
    tick(&mut app, 0.76);
    tick(&mut app, 0.25);
    let amount = app
        .world()
        .resource::<PetBehavior>()
        .cursor_pose()
        .unwrap()
        .amount;
    app.world_mut().resource_mut::<LivingPointer>().point = Some(Vec2::splat(900.0));
    tick(&mut app, 0.01);
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::IdleMicro(IdleMicroAction::CursorEscape)
    );
    assert_eq!(
        app.world()
            .resource::<PetBehavior>()
            .cursor_pose()
            .unwrap()
            .amount,
        amount
    );
    tick(&mut app, 0.86);
    assert!(
        app.world()
            .resource::<PetBehavior>()
            .cursor_pose()
            .is_none()
    );
}
#[test]
fn higher_priority_reaction_cancels_cursor_without_cancelling_user_reaction() {
    let mut app = app();
    start(&mut app);
    app.world_mut()
        .resource_mut::<NeraAnimationState>()
        .request_touch_reaction(
            crate::animation::TouchReactionClip::Wave1,
            Duration::from_secs(2),
        );
    tick(&mut app, 0.05);
    assert!(
        app.world()
            .resource::<PetBehavior>()
            .cursor_pose()
            .is_none()
    );
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::TouchReaction(crate::animation::TouchReactionClip::Wave1)
    );
}
#[test]
fn monitor_seams_are_not_hiding_edges_including_negative_coordinates() {
    let handle = NativeMonitorHandle::from_raw(1);
    let work = PhysicalRect {
        left: -1920,
        top: 0,
        right: 0,
        bottom: 1040,
    };
    let rect = PhysicalRect {
        left: -800,
        top: 100,
        right: -200,
        bottom: 1040,
    };
    let neighbor = platform::MonitorInfo {
        handle: NativeMonitorHandle::from_raw(2),
        bounds: PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        },
        work_area: work,
        is_primary: false,
    };
    assert!(exposed_edge(work, rect, &[neighbor], handle, true));
    assert!(!exposed_edge(work, rect, &[neighbor], handle, false));
    let mut stacked = neighbor;
    stacked.bounds.top = 1080;
    assert!(exposed_edge(work, rect, &[stacked], handle, false));
}
#[test]
fn clear_character_discards_routines_and_animation_tail() {
    let mut app = app();
    start(&mut app);
    app.init_resource::<WalkingRuntime>();
    clear_for_character_change(app.world_mut());
    assert!(
        app.world()
            .resource::<PetBehavior>()
            .cursor_pose()
            .is_none()
    );
    assert!(app.world().resource::<PetBehavior>().living.edge.is_none());
    assert_eq!(
        app.world().resource::<NeraAnimationState>().visual_state(),
        VisualState::Idle
    );
}

#[test]
fn drag_alarm_and_library_playback_preempt_and_never_resume_cursor_tail() {
    use crate::animation::TemporaryAction;
    for action in [
        TemporaryAction::Floating,
        TemporaryAction::AlarmWave,
        TemporaryAction::LibraryAnimation(Entity::from_raw_u32(44).unwrap()),
    ] {
        let mut app = app();
        start(&mut app);
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .request_action_for_test(action, Duration::from_secs(5));
        let expected = app.world().resource::<NeraAnimationState>().visual_state();
        tick(&mut app, 0.05);
        assert!(
            app.world()
                .resource::<PetBehavior>()
                .cursor_pose()
                .is_none()
        );
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            expected
        );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .advance(Duration::from_secs(6));
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::Idle
        );
    }
}

fn edge() -> EdgeRoutine {
    EdgeRoutine {
        monitor: NativeMonitorHandle::from_raw(1),
        home: 160,
        hidden: -288,
        visible: 8,
        phase: EdgePhase::Hide,
        wait: 2.5,
        expected: -288,
        recovery: false,
        width: 600,
        work: PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        },
        check_in: 0.25,
    }
}
fn recovery_app() -> App {
    let mut app = app();
    app.init_resource::<DesktopPhysicsState>()
        .init_resource::<DesktopEnvironment>()
        .init_resource::<NeraSettings>()
        .init_resource::<DesktopPetInteraction>()
        .init_resource::<PresentationState>()
        .init_resource::<StartupEntrance>()
        .init_resource::<SettingsInputGate>()
        .init_resource::<WalkingRuntime>();
    #[cfg(target_os = "windows")]
    app.init_resource::<crate::alarm_settings::AlarmSettingsInputGate>()
        .init_resource::<crate::animation_manager::AnimationManagerInputGate>();
    app.world_mut().resource_mut::<PetBehavior>().living.edge = Some(edge());
    app
}
fn recover(app: &mut App) {
    app.world_mut()
        .run_system_once(
            |mut brain: ResMut<PetBehavior>,
             context: BehaviorContext,
             mut physics: ResMut<DesktopPhysicsState>,
             mut env: ResMut<DesktopEnvironment>,
             mut walking: ResMut<WalkingRuntime>,
             mut animation: ResMut<NeraAnimationState>| {
                brain.living.recover(
                    &context,
                    &mut physics,
                    &mut env,
                    &mut walking,
                    &mut animation,
                );
            },
        )
        .unwrap();
}
#[test]
fn attention_defers_edge_recovery_until_desktop_snapshot_returns() {
    let mut app = recovery_app();
    app.world_mut()
        .resource_mut::<PetBehavior>()
        .living
        .interrupt(&mut NeraAnimationState::default());
    app.insert_resource(PresentationState::for_test(
        crate::presentation::PresentationMode::Attention(crate::presentation::AttentionKind::Alarm),
    ));
    recover(&mut app);
    assert!(
        app.world()
            .resource::<PetBehavior>()
            .living
            .edge
            .unwrap()
            .recovery
    );
    assert!(
        app.world()
            .resource::<PetBehavior>()
            .living_blocks_position_save()
    );
}

#[test]
fn unresolved_press_defers_recovery_without_changing_drag_grab_point() {
    let mut app = recovery_app();
    app.world_mut()
        .resource_mut::<PetBehavior>()
        .living
        .interrupt(&mut NeraAnimationState::default());
    app.world_mut()
        .resource_mut::<DesktopPetInteraction>()
        .state = InteractionState::Pressed;
    let grab = IVec2::new(350, 250);
    app.world_mut()
        .resource_mut::<DesktopPetInteraction>()
        .drag_offset = Some(grab);
    recover(&mut app);
    assert!(app.world().resource::<PetBehavior>().living.edge.is_some());
    assert_eq!(
        app.world().resource::<DesktopPetInteraction>().drag_offset,
        Some(grab)
    );
}
#[test]
fn dragging_or_new_cloud_placement_invalidates_pending_edge_recovery() {
    for cloud in [false, true] {
        let mut app = recovery_app();
        app.world_mut()
            .resource_mut::<PetBehavior>()
            .living
            .interrupt(&mut NeraAnimationState::default());
        let rect = PhysicalRect {
            left: 900,
            top: 20,
            right: 1500,
            bottom: 970,
        };
        app.world_mut()
            .resource_mut::<DesktopEnvironment>()
            .nera_rect = Some(rect);
        if cloud {
            let avatar = app.world_mut().spawn_empty().id();
            let visual = app.world_mut().spawn_empty().id();
            app.insert_resource(crate::cloud::CloudRuntime::for_test(
                crate::cloud::CloudMode::Ride,
                avatar,
                visual,
            ));
        } else {
            app.world_mut()
                .resource_mut::<DesktopPetInteraction>()
                .state = InteractionState::Dragging;
        }
        recover(&mut app);
        assert!(app.world().resource::<PetBehavior>().living.edge.is_none());
        assert_eq!(
            app.world().resource::<DesktopEnvironment>().nera_rect,
            Some(rect)
        );
    }
}
#[test]
fn rest_and_ride_cloud_block_living_admission() {
    for mode in [crate::cloud::CloudMode::Rest, crate::cloud::CloudMode::Ride] {
        let mut app = recovery_app();
        let avatar = app.world_mut().spawn_empty().id();
        let visual = app.world_mut().spawn_empty().id();
        app.insert_resource(crate::cloud::CloudRuntime::for_test(mode, avatar, visual));
        let allowed = app
            .world_mut()
            .run_system_once(|context: BehaviorContext| context.available())
            .unwrap();
        assert!(!allowed);
    }
}

#[test]
fn debug_pointer_injection_is_read_only_and_missing_arm_mapping_is_safe() {
    let mut app = app();
    app.world_mut().resource_mut::<LivingPointer>().scripted = Some(Some(Vec2::new(-10.0, 20.0)));
    app.world_mut().run_system_once(sample_pointer).unwrap();
    assert_eq!(
        app.world().resource::<LivingPointer>().point,
        Some(Vec2::new(-10.0, 20.0))
    );
    app.init_resource::<DesktopEnvironment>();
    let count = app
        .world_mut()
        .run_system_once(|context: LivingContext, env: Res<DesktopEnvironment>| {
            context.arms(&env).len()
        })
        .unwrap();
    assert_eq!(count, 0);
}
