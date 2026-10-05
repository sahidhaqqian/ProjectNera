use super::*;
use bevy::{
    ecs::system::RunSystemOnce,
    mesh::{
        VertexAttributeValues,
        skinning::{SkinnedMesh, SkinnedMeshInverseBindposes},
    },
};

fn app_with_skin() -> (App, Entity, Entity, Entity) {
    let (mut app, window, camera, avatar, _, _) = tests::presentation_app();
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<SkinnedMeshInverseBindposes>>();
    let bones: Vec<_> = app
        .world_mut()
        .query::<(Entity, &VrmBone)>()
        .iter(app.world())
        .map(|(entity, bone)| (entity, bone.as_str().to_string()))
        .collect();
    let hips = bones.iter().find(|(_, bone)| bone == "hips").unwrap().0;
    for (entity, _) in &bones {
        app.world_mut()
            .entity_mut(*entity)
            .insert(ChildOf(if *entity == hips { avatar } else { hips }));
    }
    let head = bones.iter().find(|(_, bone)| bone == "head").unwrap().0;
    let upper = app
        .world_mut()
        .spawn((
            VrmBone::from("leftUpperLeg"),
            RestGlobalTransform(GlobalTransform::from_translation(Vec3::new(0.0, -0.1, 0.0))),
            ChildOf(hips),
            NeraLiveBone,
        ))
        .id();
    let lower = app
        .world_mut()
        .spawn((
            VrmBone::from("leftLowerLeg"),
            RestGlobalTransform(GlobalTransform::from_translation(Vec3::new(0.0, -0.4, 0.0))),
            ChildOf(upper),
            NeraLiveBone,
        ))
        .id();
    let foot = app
        .world_mut()
        .spawn((
            VrmBone::from("leftFoot"),
            RestGlobalTransform(GlobalTransform::from_translation(Vec3::new(0.0, -0.6, 0.0))),
            ChildOf(lower),
            NeraLiveBone,
        ))
        .id();
    let mut mesh = Mesh::from(Cuboid::new(0.7, 1.3, 0.2));
    let VertexAttributeValues::Float32x3(points) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
    else {
        panic!()
    };
    let indices: Vec<_> = points
        .iter()
        .map(|p| [if p[1] > 0.0 { 1_u16 } else { 2 }, 0, 0, 0])
        .collect();
    let count = points.len();
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_INDEX,
        VertexAttributeValues::Uint16x4(indices),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_JOINT_WEIGHT,
        vec![[1.0_f32, 0.0, 0.0, 0.0]; count],
    );
    mesh.generate_skinned_mesh_bounds().unwrap();
    // Exercise the same GPU-only state used by the native avatar.
    let _ = mesh.take_gpu_data().unwrap();
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
    let binds = app
        .world_mut()
        .resource_mut::<Assets<SkinnedMeshInverseBindposes>>()
        .add(vec![
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(0.0, -0.45, 0.0)),
            Mat4::from_translation(Vec3::new(0.0, 0.6, 0.0)),
        ]);
    app.world_mut().spawn((
        Mesh3d(mesh),
        SkinnedMesh {
            joints: vec![hips, head, foot],
            inverse_bindposes: binds,
        },
        ChildOf(avatar),
    ));
    (app, window, camera, avatar)
}

#[test]
fn screensaver_repeated_entry_exit_restores_exact_snapshot_without_scaling_avatar() {
    let (mut app, window, camera, avatar) = app_with_skin();
    let root = *app.world().get::<Transform>(avatar).unwrap();
    let camera_pose = *app.world().get::<Transform>(camera).unwrap();
    let level = app.world().get::<Window>(window).unwrap().window_level;
    for _ in 0..20 {
        app.world_mut()
            .write_message(PresentationCommand::EnterAttention(
                AttentionKind::Screensaver,
            ));
        app.update();
        assert!(app.world().resource::<PresentationState>().is_screensaver());
        let snapshot = app
            .world()
            .resource::<PresentationState>()
            .snapshot
            .as_ref()
            .unwrap()
            .clone();
        app.world_mut()
            .write_message(PresentationCommand::EnterAttention(
                AttentionKind::Screensaver,
            ));
        app.update();
        assert_eq!(
            app.world()
                .resource::<PresentationState>()
                .snapshot
                .as_ref()
                .unwrap()
                .camera_transform,
            snapshot.camera_transform
        );
        let large = app.world().get::<Window>(window).unwrap();
        assert_eq!(large.resolution.physical_height(), 1015);
        app.world_mut()
            .write_message(PresentationCommand::ExitScreensaver);
        app.update();
        assert!(app.world().resource::<PresentationState>().is_desktop_pet());
        assert_eq!(*app.world().get::<Transform>(camera).unwrap(), camera_pose);
        assert_eq!(*app.world().get::<Transform>(avatar).unwrap(), root);
        let window = app.world().get::<Window>(window).unwrap();
        assert_eq!(window.position, WindowPosition::At(IVec2::new(40, 60)));
        assert_eq!(window.resolution.physical_width(), 600);
        assert_eq!(window.resolution.physical_height(), 950);
        assert_eq!(window.window_level, level);
        assert!(
            app.world()
                .resource::<PresentationState>()
                .snapshot
                .is_none()
        );
    }
}

#[test]
fn alarm_preempts_screensaver_preserves_desktop_snapshot_and_ignores_stale_input_exit() {
    let (mut app, window, camera, _) = app_with_skin();
    let original = *app.world().get::<Transform>(camera).unwrap();
    app.world_mut()
        .write_message(PresentationCommand::EnterAttention(
            AttentionKind::Screensaver,
        ));
    app.update();
    app.world_mut()
        .write_message(PresentationCommand::EnterAttention(AttentionKind::Alarm));
    app.world_mut()
        .write_message(PresentationCommand::ExitScreensaver);
    app.update();
    let state = app.world().resource::<PresentationState>();
    assert_eq!(
        state.mode,
        PresentationMode::Attention(AttentionKind::Alarm)
    );
    assert_eq!(state.snapshot.as_ref().unwrap().camera_transform, original);
    assert_eq!(
        app.world()
            .get::<Window>(window)
            .unwrap()
            .resolution
            .physical_height(),
        842
    );
    app.world_mut()
        .write_message(PresentationCommand::EnterAttention(
            AttentionKind::Screensaver,
        ));
    app.update();
    assert_eq!(
        app.world().resource::<PresentationState>().mode,
        PresentationMode::Attention(AttentionKind::Alarm)
    );
    app.world_mut()
        .write_message(PresentationCommand::ExitAttention);
    app.update();
    assert!(app.world().resource::<PresentationState>().is_desktop_pet());
    assert_eq!(*app.world().get::<Transform>(camera).unwrap(), original);
}

#[test]
fn full_body_visible_height_is_85_percent_and_normal_chibi_geometry_is_finite() {
    let (mut app, _, _, _) = app_with_skin();
    for (body, head, offset) in [(1.0, 1.0, 0.0), (0.62, 1.9, -0.19)] {
        let bounds = app
            .world_mut()
            .run_system_once(
                move |geometry: full_body::FullBodyGeometry, avatars: PortraitAvatars| {
                    geometry.bounds(&avatars, body, head, offset, true)
                },
            )
            .unwrap()
            .unwrap();
        let monitor = crate::platform::PhysicalRect {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        let layout = compute_screensaver_layout(monitor, bounds);
        let visible = (bounds.top_y - bounds.bottom_y) / layout.viewport_height
            * layout.window_height as f32
            / 1080.0;
        assert!((visible - 0.85).abs() < 0.001);
        assert!(layout.camera_z.is_finite() && layout.viewport_height > 0.0);
        assert!(layout.window_width < 1920);
    }
    assert_eq!(ATTENTION_WINDOW_MONITOR_HEIGHT_RATIO, 0.78);
    assert_eq!(ATTENTION_UPPER_BODY_VIEW_FRACTION, 0.80);
    assert_eq!(ATTENTION_ENTER_SECONDS, 0.38);
}

#[test]
fn compact_gpu_skin_bounds_derive_thigh_compression_from_the_existing_ground_correction() {
    let (mut app, _, _, _) = app_with_skin();
    let (body_only, shortened) = app
        .world_mut()
        .run_system_once(
            |geometry: full_body::FullBodyGeometry, avatars: PortraitAvatars| {
                (
                    geometry.bounds(&avatars, 0.62, 1.9, -0.19, true).unwrap(),
                    geometry
                        .bounds(&avatars, 0.62, 1.9, -0.21325, true)
                        .unwrap(),
                )
            },
        )
        .unwrap();
    assert!((body_only.bottom_y - shortened.bottom_y).abs() < 0.00001);
    assert!((body_only.top_y - shortened.top_y - 0.0279).abs() < 0.00001);
}

#[test]
fn vrma_source_rigs_under_the_avatar_never_supply_humanoid_landmarks() {
    let (mut app, _, _, avatar) = app_with_skin();
    let measured = |geometry: full_body::FullBodyGeometry, avatars: PortraitAvatars| {
        geometry.bounds(&avatars, 0.62, 1.9, -0.21325, true)
    };
    let original = app.world_mut().run_system_once(measured).unwrap().unwrap();
    app.world_mut().spawn((
        VrmBone::from("head"),
        RestGlobalTransform(GlobalTransform::from_translation(Vec3::new(0.0, 20.0, 0.0))),
        ChildOf(avatar),
    ));
    let with_source = app.world_mut().run_system_once(measured).unwrap().unwrap();
    assert_eq!(original.top_y, with_source.top_y);
    assert_eq!(original.bottom_y, with_source.bottom_y);
    let live_head = app
        .world_mut()
        .query_filtered::<(Entity, &VrmBone), With<NeraLiveBone>>()
        .iter(app.world())
        .find(|(_, bone)| bone.as_str() == "head")
        .unwrap()
        .0;
    app.world_mut().entity_mut(live_head).remove::<VrmBone>();
    assert!(app.world_mut().run_system_once(measured).unwrap().is_none());
}

#[test]
fn missing_geometry_camera_monitor_or_unready_avatar_rejects_without_snapshot() {
    for case in 0..5 {
        let (mut app, _, camera, avatar) = app_with_skin();
        match case {
            0 => {
                app.world_mut().remove_resource::<Assets<Mesh>>();
            }
            1 => {
                app.world_mut().despawn(camera);
            }
            2 => app.world_mut().resource_mut::<DesktopEnvironment>().monitor = None,
            3 => {
                app.world_mut().entity_mut(avatar).remove::<Initialized>();
            }
            _ => {
                app.world_mut()
                    .resource_mut::<DesktopEnvironment>()
                    .monitor
                    .as_mut()
                    .unwrap()
                    .bounds
                    .bottom = 0;
            }
        }
        app.world_mut()
            .write_message(PresentationCommand::EnterAttention(
                AttentionKind::Screensaver,
            ));
        app.update();
        let state = app.world().resource::<PresentationState>();
        assert!(state.is_desktop_pet() && state.snapshot.is_none());
    }
}

#[test]
fn screensaver_entry_and_exit_clear_drag_hover_and_headpat() {
    use crate::interaction::{BodyPart, InteractionState};
    let (mut app, _, _, _) = app_with_skin();
    for command in [
        PresentationCommand::EnterAttention(AttentionKind::Screensaver),
        PresentationCommand::ExitScreensaver,
    ] {
        let mut interaction = app.world_mut().resource_mut::<DesktopPetInteraction>();
        interaction.state = InteractionState::Dragging;
        interaction.pressed_body_part = Some(BodyPart::LeftHand);
        interaction.drag_offset = Some(IVec2::new(20, 30));
        app.world_mut().resource_mut::<HoveredBodyPart>().part = Some(BodyPart::Head);
        app.world_mut()
            .resource_mut::<HeadPatGesture>()
            .total_travel = 50.0;
        app.world_mut().write_message(command);
        app.update();
        let interaction = app.world().resource::<DesktopPetInteraction>();
        assert_eq!(interaction.state, InteractionState::None);
        assert!(interaction.pressed_body_part.is_none() && interaction.drag_offset.is_none());
        assert!(app.world().resource::<HoveredBodyPart>().part.is_none());
        assert_eq!(app.world().resource::<HeadPatGesture>().total_travel, 0.0);
    }
}

#[test]
fn screensaver_restores_window_and_taskbar_seat_without_replaying_entry() {
    use crate::{
        environment::{TaskbarAttachment, TaskbarEdge, WindowAttachment},
        platform::{NativeMonitorHandle, NativeWindowHandle},
    };
    for placement in [
        PlacementState::Window(WindowAttachment {
            target: NativeWindowHandle::from_raw(9),
            support_offset_x: 37,
        }),
        PlacementState::Taskbar(TaskbarAttachment {
            monitor: NativeMonitorHandle::from_raw(1),
            edge: TaskbarEdge::Bottom,
            along_offset: 51,
        }),
    ] {
        let (mut app, _, _, _) = app_with_skin();
        let mut environment = app.world_mut().resource_mut::<DesktopEnvironment>();
        environment.placement = placement;
        environment.set_attached_anchor(AvatarAnchor::Seat);
        environment.hidden_for_target_suspend = true;
        app.world_mut()
            .write_message(PresentationCommand::EnterAttention(
                AttentionKind::Screensaver,
            ));
        app.update();
        assert_eq!(
            app.world().resource::<DesktopEnvironment>().placement,
            PlacementState::Free
        );
        app.world_mut()
            .write_message(PresentationCommand::ExitScreensaver);
        app.update();
        let environment = app.world().resource::<DesktopEnvironment>();
        assert_eq!(environment.placement, placement);
        assert_eq!(environment.attached_anchor, AvatarAnchor::Seat);
        assert!(environment.hidden_for_target_suspend);
        assert_eq!(
            app.world().resource::<NeraAnimationState>().base_pose(),
            crate::animation::BasePose::Sitting
        );
    }
}

#[test]
fn screensaver_releases_cursor_gaze_preserves_authored_pose_and_restores_preferences() {
    use crate::{
        avatar::{LookAtTarget, sync_look_at_for_animation},
        settings::NeraSettings,
    };
    use bevy_vrm1::{prelude::LookAt, vrm::body_tracking::BodyTracking};
    let (mut app, _, _, avatar) = app_with_skin();
    app.insert_resource(NeraSettings {
        follow_mouse: true,
        follow_eyes: true,
        follow_head: true,
        ..default()
    })
    .add_systems(
        Update,
        sync_look_at_for_animation.after(handle_presentation_commands),
    );
    app.world_mut().spawn(LookAtTarget);
    let authored_pose = *app.world().get::<Transform>(avatar).unwrap();
    app.update();
    assert!(app.world().get::<LookAt>(avatar).is_some());
    assert!(app.world().get::<BodyTracking>(avatar).is_some());
    app.world_mut()
        .write_message(PresentationCommand::EnterAttention(
            AttentionKind::Screensaver,
        ));
    app.update();
    assert!(app.world().get::<LookAt>(avatar).is_none());
    assert!(app.world().get::<BodyTracking>(avatar).is_none());
    assert_eq!(
        *app.world().get::<Transform>(avatar).unwrap(),
        authored_pose
    );
    assert!(
        app.world_mut()
            .run_system_once(avatar_hit_testing_active)
            .unwrap()
    );
    app.world_mut()
        .write_message(PresentationCommand::ExitScreensaver);
    app.update();
    assert!(app.world().get::<LookAt>(avatar).is_some());
    assert!(app.world().get::<BodyTracking>(avatar).is_some());
    let settings = app.world().resource::<NeraSettings>();
    assert!(settings.follow_mouse && settings.follow_eyes && settings.follow_head);
    app.world_mut()
        .write_message(PresentationCommand::EnterAttention(AttentionKind::Alarm));
    app.update();
    assert!(
        !app.world_mut()
            .run_system_once(avatar_hit_testing_active)
            .unwrap()
    );
}

#[test]
fn desktop_physics_waits_for_normal_native_geometry_after_screensaver_exit() {
    let (mut app, window, _, _) = app_with_skin();
    let original_rect = app.world().resource::<DesktopEnvironment>().nera_rect;
    app.world_mut()
        .write_message(PresentationCommand::EnterAttention(
            AttentionKind::Screensaver,
        ));
    app.update();
    let canvas = app.world().get::<Window>(window).unwrap();
    let WindowPosition::At(position) = canvas.position else {
        panic!()
    };
    let attention_rect = crate::platform::PhysicalRect {
        left: position.x,
        top: position.y,
        right: position.x + canvas.resolution.physical_width() as i32,
        bottom: position.y + canvas.resolution.physical_height() as i32,
    };
    app.world_mut()
        .resource_mut::<DesktopEnvironment>()
        .nera_rect = Some(attention_rect);
    app.world_mut()
        .write_message(PresentationCommand::ExitScreensaver);
    app.update();
    assert!(app.world().resource::<PresentationState>().is_desktop_pet());
    assert!(!app.world_mut().run_system_once(desktop_pet_active).unwrap());
    assert!(
        !app.world_mut()
            .run_system_once(avatar_hit_testing_active)
            .unwrap()
    );
    // First's real native poll supplies the restored HWND on the next frame.
    app.world_mut()
        .resource_mut::<DesktopEnvironment>()
        .nera_rect = original_rect;
    app.update();
    assert!(app.world_mut().run_system_once(desktop_pet_active).unwrap());
    assert!(
        app.world_mut()
            .run_system_once(avatar_hit_testing_active)
            .unwrap()
    );
}
