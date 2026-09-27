//! A bounded pool of independent VRM worlds rendered into card textures.
//! Each avatar has its own render layer AND animation-target namespace.
//! bevy_vrm1's bake pass searches targets globally, so duplicate skeleton IDs
//! would otherwise select another avatar's retarget table.
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use bevy::{
    animation::{AnimationTargetId, RepeatAnimation},
    camera::{RenderTarget, visibility::RenderLayers},
    prelude::*,
    render::render_resource::TextureFormat,
};
use bevy_vrm1::prelude::*;

use super::{GalleryScroll, ManagerState, PreviewSlot};

const PREVIEW_COLUMNS: usize = 3;
const MAX_ACTIVE_PREVIEWS: usize = 12;
const MAX_ACTIVE_ROWS: usize = MAX_ACTIVE_PREVIEWS / PREVIEW_COLUMNS;

// Allow the dependency's deferred animation-graph bake to complete.
const PREVIEW_PLAYBACK_WARMUP_FRAMES: u8 = 2;

#[derive(Component)]
pub(super) struct PreviewAvatarRoot;

/// LoadedVrma can arrive before the parent preview VRM has finished building its
/// humanoid skeleton. Mark the clip as ready, then start it only after the VRM
/// exposes HipsBoneEntity. This avoids a load-order race when many previews are
/// created together.
#[derive(Component)]
pub(super) struct PreviewClipReady;

struct PreviewScene {
    root: Entity,
    camera: Entity,
    clip: Entity,
    image: Handle<Image>,
    slot: usize,
    prepared: bool,
    started: bool,
    warmup_frames: u8,
}

#[derive(Resource)]
pub(super) struct PreviewRuntime {
    character_path: String,
    active: HashMap<String, PreviewScene>,
    light: Option<Entity>,
    requested: HashSet<String>,
    changed_at: Option<std::time::Instant>,
}

impl Default for PreviewRuntime {
    fn default() -> Self {
        Self::for_character(crate::character::DEFAULT_CHARACTER.to_owned())
    }
}

impl PreviewRuntime {
    pub(super) fn for_character(character_path: String) -> Self {
        Self {
            character_path,
            active: HashMap::new(),
            light: None,
            requested: HashSet::new(),
            changed_at: None,
        }
    }

    pub(super) fn set_character(
        &mut self,
        path: String,
        commands: &mut Commands,
        images: &mut Assets<Image>,
    ) {
        if self.character_path != path {
            self.clear(commands, images);
            self.character_path = path;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    pub fn remove(&mut self, key: &str, commands: &mut Commands, images: &mut Assets<Image>) {
        if let Some(scene) = self.active.remove(key) {
            commands.entity(scene.root).try_despawn();
            commands.entity(scene.camera).try_despawn();
            images.remove(&scene.image);
        }
    }

    pub fn clear(&mut self, commands: &mut Commands, images: &mut Assets<Image>) {
        let keys: Vec<_> = self.active.keys().cloned().collect();
        for key in keys {
            self.remove(&key, commands, images);
        }
        self.active.shrink_to_fit();
        if let Some(light) = self.light.take() {
            commands.entity(light).try_despawn();
        }
        self.requested = HashSet::new();
        self.changed_at = None;
    }

    pub fn restart(
        &self,
        key: &str,
        commands: &mut Commands,
        players: &mut Query<&mut AnimationPlayer>,
        children: &Query<&Children>,
    ) {
        if let Some(scene) = self.active.get(key) {
            stop_preview_players(scene.root, players, children);
            play_preview(commands, scene.clip, true);
        }
    }

    pub fn restart_all(
        &self,
        commands: &mut Commands,
        players: &mut Query<&mut AnimationPlayer>,
        children: &Query<&Children>,
    ) {
        for scene in self.active.values() {
            stop_preview_players(scene.root, players, children);
            play_preview(commands, scene.clip, true);
        }
    }
}

fn stop_preview_players(
    root: Entity,
    players: &mut Query<&mut AnimationPlayer>,
    children: &Query<&Children>,
) {
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Ok(mut player) = players.get_mut(entity) {
            player.stop_all();
        }
        if let Ok(entity_children) = children.get(entity) {
            for child in entity_children.into_iter().copied() {
                stack.push(child);
            }
        }
    }
}

fn play_preview(commands: &mut Commands, clip: Entity, reset_spring_bones: bool) {
    commands.trigger(PlayVrma {
        vrma: clip,
        repeat: RepeatAnimation::Forever,
        transition_duration: Duration::ZERO,
        reset_spring_bones,
    });
}

fn on_preview_loaded(trigger: On<LoadedVrma>, mut commands: Commands) {
    commands.entity(trigger.vrma).insert(PreviewClipReady);
}

fn instance_target(root: Entity, original: AnimationTargetId) -> AnimationTargetId {
    AnimationTargetId::from_iter([
        format!("nera-preview:{}", root.to_bits()),
        original.0.to_string(),
    ])
}

/// Prepare the completed live rig before attaching ANY VRMA. Waiting until
/// LoadedVrma is too late: the library has already built the retarget tables.
pub(super) fn prepare_preview_rigs(
    mut runtime: ResMut<PreviewRuntime>,
    initialized: Query<(), With<Initialized>>,
    children: Query<&Children>,
    mut targets: Query<&mut AnimationTargetId>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    for (key, scene) in &mut runtime.active {
        if scene.prepared || !initialized.contains(scene.root) {
            continue;
        }
        let layer = RenderLayers::layer(scene.slot + 1);
        let mut stack = vec![scene.root];
        while let Some(entity) = stack.pop() {
            if let Ok(mut target) = targets.get_mut(entity) {
                *target = instance_target(scene.root, *target);
            }
            commands.entity(entity).insert(layer.clone());
            if let Ok(descendants) = children.get(entity) {
                stack.extend(descendants.iter());
            }
        }
        commands.entity(scene.root).insert(Visibility::Inherited);
        commands
            .entity(scene.clip)
            .insert(VrmaHandle(assets.load(format!("animations/{key}"))));
        scene.prepared = true;
    }
}

/// LoadedVrma is emitted while bevy_vrm1 is still completing its animation-graph
/// update/bake work in Update. Starting playback immediately can therefore sample
/// the pre-baked retargeted clip for the first frame. With many previews created
/// together that showed up as flipped or otherwise broken poses.
///
/// Wait two full manager updates after LoadedVrma + live hips are present before
/// triggering playback. The tiny delay is visually negligible but gives the
/// upstream bake pass time to settle without depending on its private markers.
pub(super) fn start_ready_previews(
    mut runtime: ResMut<PreviewRuntime>,
    ready_clips: Query<(), With<PreviewClipReady>>,
    ready_avatars: Query<(), With<HipsBoneEntity>>,
    mut commands: Commands,
) {
    for scene in runtime.active.values_mut() {
        if scene.started || !ready_clips.contains(scene.clip) || !ready_avatars.contains(scene.root)
        {
            continue;
        }

        if scene.warmup_frames < PREVIEW_PLAYBACK_WARMUP_FRAMES {
            scene.warmup_frames += 1;
            continue;
        }

        play_preview(&mut commands, scene.clip, false);
        scene.started = true;
    }
}

pub(super) fn sync_visible_previews(
    state: Res<ManagerState>,
    scroll: Query<&ComputedNode, With<GalleryScroll>>,
    slots: Query<(&PreviewSlot, &mut ImageNode)>,
    mut runtime: ResMut<PreviewRuntime>,
    mut images: ResMut<Assets<Image>>,
    assets: Res<AssetServer>,
    mut commands: Commands,
) {
    let Ok(computed) = scroll.single() else {
        return;
    };
    let viewport_height = computed.size().y * computed.inverse_scale_factor();
    let first_visible_row = (state.scroll_y / super::ui::ROW_HEIGHT).floor().max(0.0) as usize;
    let total_rows = (computed.content_size().y * computed.inverse_scale_factor()
        / super::ui::ROW_HEIGHT)
        .ceil() as usize;
    // Overscan cards are inexpensive UI; don't load whole VRM worlds for them.
    let first_active_row = first_visible_row.min(total_rows);
    let last_active_row =
        ((state.scroll_y + viewport_height) / super::ui::ROW_HEIGHT).ceil() as usize;
    let last_active_row = last_active_row
        .min(total_rows)
        .min(first_active_row + MAX_ACTIVE_ROWS);

    let mut needed = HashSet::new();
    for (slot, _) in &slots {
        let row = slot.index / PREVIEW_COLUMNS;
        if row >= first_active_row && row < last_active_row {
            needed.insert(slot.key.clone());
        }
    }

    let stale: Vec<_> = runtime
        .active
        .keys()
        .filter(|key| !needed.contains(*key))
        .cloned()
        .collect();
    for key in stale {
        runtime.remove(&key, &mut commands, &mut images);
    }

    if runtime.requested != needed {
        runtime.requested = needed.clone();
        runtime.changed_at = Some(std::time::Instant::now());
    }
    let settled = runtime
        .changed_at
        .is_none_or(|at| at.elapsed() >= Duration::from_millis(120));

    let mut occupied: HashSet<_> = runtime.active.values().map(|scene| scene.slot).collect();
    for (slot, mut image_node) in slots {
        if !needed.contains(&slot.key) {
            continue;
        }
        if let Some(scene) = runtime.active.get(&slot.key) {
            image_node.image = scene.image.clone();
            continue;
        }
        if !settled {
            continue;
        }
        let Some(index) = (0..MAX_ACTIVE_PREVIEWS).find(|index| !occupied.contains(index)) else {
            break;
        };
        occupied.insert(index);

        if runtime.light.is_none() {
            runtime.light = Some(
                commands
                    .spawn((
                        Name::new("Nera Gallery Shared Light"),
                        DirectionalLight {
                            illuminance: 10_000.0,
                            shadow_maps_enabled: false,
                            contact_shadows_enabled: false,
                            ..default()
                        },
                        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.7, -0.5, 0.0)),
                        RenderLayers::from_layers(&(1..=MAX_ACTIVE_PREVIEWS).collect::<Vec<_>>()),
                    ))
                    .id(),
            );
        }

        // Render layers isolate unit-scale rigs without offsetting their rest pose.
        let layer = RenderLayers::layer(index + 1);
        let image = images.add(Image::new_target_texture(
            512,
            420,
            TextureFormat::Rgba8Unorm,
            Some(TextureFormat::Rgba8UnormSrgb),
        ));
        let camera = commands
            .spawn((
                Name::new("Nera Gallery Preview Camera"),
                Camera3d::default(),
                Camera {
                    order: -1,
                    clear_color: ClearColorConfig::Custom(Color::srgb(0.055, 0.071, 0.122)),
                    ..default()
                },
                RenderTarget::Image(image.clone().into()),
                Projection::Perspective(PerspectiveProjection {
                    fov: 40.0_f32.to_radians(),
                    far: 45.0,
                    ..default()
                }),
                Transform::from_xyz(0.0, 0.85, 2.9).looking_at(Vec3::new(0.0, 0.85, 0.0), Vec3::Y),
                layer.clone(),
                Msaa::Off,
            ))
            .id();
        let mut clip = Entity::PLACEHOLDER;
        let root = commands
            .spawn((
                Name::new("Nera Gallery Preview Avatar"),
                PreviewAvatarRoot,
                VrmHandle(assets.load(runtime.character_path.clone())),
                Transform::default(),
                Visibility::Hidden,
                layer,
            ))
            .with_children(|parent| {
                clip = parent
                    .spawn((Name::new("Nera Gallery Preview VRMA"),))
                    .observe(on_preview_loaded)
                    .id();
            })
            .id();
        image_node.image = image.clone();
        runtime.active.insert(
            slot.key.clone(),
            PreviewScene {
                root,
                camera,
                clip,
                image,
                slot: index,
                prepared: false,
                started: false,
                warmup_frames: 0,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_change_releases_old_preview_assets_once() {
        #[derive(Resource)]
        struct NextCharacter(String);

        fn change_character(
            character: Res<NextCharacter>,
            mut previews: ResMut<PreviewRuntime>,
            mut images: ResMut<Assets<Image>>,
            mut commands: Commands,
        ) {
            previews.set_character(character.0.clone(), &mut commands, &mut images);
        }

        let mut app = App::new();
        app.init_resource::<PreviewRuntime>()
            .init_resource::<Assets<Image>>()
            .insert_resource(NextCharacter(crate::character::DEFAULT_CHARACTER.into()))
            .add_systems(Update, change_character);
        let root = app.world_mut().spawn_empty().id();
        let clip = app.world_mut().spawn(ChildOf(root)).id();
        let camera = app.world_mut().spawn_empty().id();
        let light = app.world_mut().spawn_empty().id();
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(Image::default());
        let image_id = image.id();
        {
            let mut previews = app.world_mut().resource_mut::<PreviewRuntime>();
            previews.light = Some(light);
            previews.requested.insert("idle.vrma".into());
            previews.active.insert("idle.vrma".into(), PreviewScene {
                root,
                camera,
                clip,
                image,
                slot: 0,
                prepared: true,
                started: true,
                warmup_frames: PREVIEW_PLAYBACK_WARMUP_FRAMES,
            });
        }

        app.update();
        assert!(app.world().get_entity(root).is_ok(), "unchanged selection keeps previews");
        assert!(app.world().resource::<Assets<Image>>().contains(image_id));

        app.world_mut().resource_mut::<NextCharacter>().0 = "characters://custom.vrm".into();
        app.update();
        for entity in [root, clip, camera, light] {
            assert!(app.world().get_entity(entity).is_err());
        }
        assert!(!app.world().resource::<Assets<Image>>().contains(image_id));
        let previews = app.world().resource::<PreviewRuntime>();
        assert!(previews.active.is_empty());
        assert!(previews.requested.is_empty());
        assert_eq!(previews.character_path, "characters://custom.vrm");
    }

    #[test]
    #[ignore = "renders the full gallery and measures process memory"]
    fn full_gallery_scroll_and_memory_regression() {
        use super::super::*;
        use bevy::{app::PluginsState, winit::WinitPlugin, world_serialization::WorldAsset};
        #[repr(C)]
        #[derive(Default)]
        struct Memory {
            cb: u32,
            faults: u32,
            peak_ws: usize,
            ws: usize,
            peak_paged: usize,
            paged: usize,
            peak_nonpaged: usize,
            nonpaged: usize,
            pagefile: usize,
            peak_pagefile: usize,
            private: usize,
        }
        #[link(name = "psapi")]
        unsafe extern "system" {
            fn GetProcessMemoryInfo(process: isize, info: *mut Memory, size: u32) -> i32;
        }
        fn sample(app: &mut App, label: &str) {
            let mut memory = Memory::default();
            memory.cb = std::mem::size_of::<Memory>() as u32;
            assert_ne!(
                unsafe {
                    GetProcessMemoryInfo(-1, &mut memory, std::mem::size_of::<Memory>() as u32)
                },
                0
            );
            let world = app.world();
            eprintln!(
                "MEM {label}: working={} MiB private={} MiB entities={} images={} meshes={} worlds={} clips={} graphs={} previews={}",
                memory.ws / 1048576,
                memory.private / 1048576,
                world.entities().len(),
                world.resource::<Assets<Image>>().len(),
                world.resource::<Assets<Mesh>>().len(),
                world.resource::<Assets<WorldAsset>>().len(),
                world.resource::<Assets<AnimationClip>>().len(),
                world.resource::<Assets<AnimationGraph>>().len(),
                world.resource::<PreviewRuntime>().active.len()
            );
        }
        fn frames(app: &mut App, count: usize) {
            for _ in 0..count {
                app.update();
                std::thread::sleep(Duration::from_millis(16));
            }
        }
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: crate::nera_asset_path(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        );
        app.add_plugins((VrmPlugin, VrmaPlugin, AnimationManagerPlugin));
        app.add_message::<crate::settings::ReturnToActionMenu>();
        app.add_message::<crate::animation::PlayLibraryAnimation>();
        while app.plugins_state() != PluginsState::Ready {
            bevy::tasks::tick_global_task_pools_on_main_thread();
        }
        app.finish();
        app.cleanup();
        let model = app
            .world()
            .resource::<AssetServer>()
            .load("vrm/nera_256.vrm");
        app.world_mut()
            .spawn((VrmHandle(model), Transform::default()));
        let target =
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(Image::new_target_texture(
                    600,
                    950,
                    TextureFormat::Rgba8Unorm,
                    Some(TextureFormat::Rgba8UnormSrgb),
                ));
        app.world_mut().spawn((
            Camera3d::default(),
            RenderTarget::Image(target.into()),
            Transform::from_xyz(0.0, 0.85, 3.4).looking_at(Vec3::new(0.0, 0.85, 0.0), Vec3::Y),
        ));
        frames(&mut app, 120);
        sample(&mut app, "baseline");
        for cycle in 0..2 {
            app.world_mut().write_message(OpenAnimationManager);
            app.update();
            let (window, camera) = {
                let world = app.world_mut();
                (
                    world
                        .query_filtered::<Entity, With<ManagerWindow>>()
                        .single(world)
                        .unwrap(),
                    world
                        .query_filtered::<Entity, With<ManagerCamera>>()
                        .single(world)
                        .unwrap(),
                )
            };
            let target =
                app.world_mut()
                    .resource_mut::<Assets<Image>>()
                    .add(Image::new_target_texture(
                        1120,
                        980,
                        TextureFormat::Rgba8Unorm,
                        Some(TextureFormat::Rgba8UnormSrgb),
                    ));
            app.world_mut()
                .entity_mut(camera)
                .insert(RenderTarget::Image(target.into()));
            frames(&mut app, 120);
            sample(&mut app, &format!("open {cycle}"));
            let count = app.world().resource::<ManagerState>().filtered().len();
            for step in 0..15 {
                app.world_mut().write_message(MouseWheel {
                    unit: MouseScrollUnit::Line,
                    x: 0.0,
                    y: -1.0,
                    window,
                    phase: bevy::input::touch::TouchPhase::Moved,
                });
                frames(&mut app, 12);
                let world = app.world_mut();
                let (position, node) = world
                    .query_filtered::<(&ScrollPosition, &ComputedNode), With<GalleryScroll>>()
                    .single(world)
                    .unwrap();
                eprintln!(
                    "SCROLL cycle={cycle} step={step} count={count} requested={} actual={} max={} viewport={}",
                    position.0.y,
                    node.scroll_position.y,
                    scroll_metrics(node).0,
                    scroll_metrics(node).1
                );
            }
            {
                let world = app.world_mut();
                let mut slots = world.query::<&PreviewSlot>();
                assert!(
                    slots.iter(world).any(|slot| slot.index == count - 1),
                    "wheel must reach the last registered animation"
                );
            }
            sample(&mut app, &format!("scrolled {cycle}"));
            app.world_mut()
                .resource_mut::<ManagerState>()
                .close_requested = true;
            frames(&mut app, 180);
            sample(&mut app, &format!("closed {cycle}"));
        }
    }

    #[test]
    fn native_close_releases_preview_entities_and_render_targets() {
        use super::super::{
            AnimationManagerInputGate, ManagerOwned, ManagerWindow,
            cleanup_closed_window, sync_manager_pointer_gate,
        };
        let mut app = App::new();
        app.init_resource::<Assets<Image>>()
            .init_resource::<PreviewRuntime>()
            .init_resource::<AnimationManagerInputGate>()
            .add_systems(PreUpdate, sync_manager_pointer_gate)
            .add_systems(Update, cleanup_closed_window);
        for _ in 0..3 {
            let window = app
                .world_mut()
                .spawn((ManagerWindow, Window::default()))
                .id();
            let owned = app.world_mut().spawn(ManagerOwned).id();
            let root = app.world_mut().spawn_empty().id();
            let clip = app.world_mut().spawn(ChildOf(root)).id();
            let camera = app.world_mut().spawn_empty().id();
            let image = app
                .world_mut()
                .resource_mut::<Assets<Image>>()
                .add(Image::default());
            let image_id = image.id();
            app.world_mut()
                .resource_mut::<PreviewRuntime>()
                .active
                .insert(
                    "test".into(),
                    PreviewScene {
                        root,
                        camera,
                        clip,
                        image,
                        slot: 0,
                        prepared: false,
                        started: false,
                        warmup_frames: 0,
                    },
                );
            app.world_mut()
                .resource_mut::<AnimationManagerInputGate>()
                .window_open = true;
            app.update();
            assert!(app.world().get_entity(root).is_ok());
            app.world_mut().despawn(window);
            app.update();
            for entity in [owned, root, clip, camera] {
                assert!(app.world().get_entity(entity).is_err());
            }
            assert!(!app.world().resource::<Assets<Image>>().contains(image_id));
            assert!(app.world().resource::<PreviewRuntime>().is_empty());
            assert!(
                !app.world()
                    .resource::<AnimationManagerInputGate>()
                    .window_open
            );
        }
    }
    #[test]
    fn identical_rigs_have_disjoint_bake_targets() {
        let mut world = World::new();
        let a = world.spawn_empty().id();
        let b = world.spawn_empty().id();
        let hips = AnimationTargetId::from_iter(["hips"]);
        assert_ne!(instance_target(a, hips), instance_target(b, hips));
        assert_ne!(instance_target(a, hips), hips);
        assert_eq!(instance_target(a, hips), instance_target(a, hips));
    }

    // Run explicitly: this regression loads the actual model/VRMA assets.
    #[test]
    #[ignore = "loads real VRM assets and initializes the headless renderer"]
    fn real_assets_keep_their_pose_and_card_click_plays_on_desktop() {
        use super::super::{ManagerAction, handle_actions};
        use crate::animation::{NeraAnimationState, VisualState};
        use bevy::{app::PluginsState, time::TimeUpdateStrategy, winit::WinitPlugin};

        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: crate::nera_asset_path(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: None,
                    ..default()
                })
                .disable::<WinitPlugin>(),
        );
        app.add_plugins((VrmPlugin, VrmaPlugin));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            16,
        )))
        .init_resource::<PreviewRuntime>()
        .init_resource::<super::super::CaretBlink>()
        .insert_resource(ManagerState::new())
        .add_systems(
            Update,
            (prepare_preview_rigs, start_ready_previews, handle_actions).chain(),
        );
        crate::animation::install_library_test_systems(&mut app);
        while app.plugins_state() != PluginsState::Ready {
            bevy::tasks::tick_global_task_pools_on_main_thread();
        }
        app.finish();
        app.cleanup();

        fn spawn(app: &mut App, key: &str, slot: usize) -> (Entity, Entity) {
            let handle = app
                .world()
                .resource::<AssetServer>()
                .load("vrm/nera_256.vrm");
            let root = app
                .world_mut()
                .spawn((VrmHandle(handle), Transform::default()))
                .id();
            let clip = app
                .world_mut()
                .spawn(ChildOf(root))
                .observe(on_preview_loaded)
                .id();
            app.world_mut()
                .resource_mut::<PreviewRuntime>()
                .active
                .insert(
                    key.into(),
                    PreviewScene {
                        root,
                        clip,
                        camera: Entity::PLACEHOLDER,
                        image: default(),
                        slot,
                        prepared: false,
                        started: false,
                        warmup_frames: 0,
                    },
                );
            (root, clip)
        }
        fn wait(app: &mut App, condition: impl Fn(&World) -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(45);
            loop {
                app.update();
                if condition(app.world()) {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "real asset load/playback timed out"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        fn pose(app: &mut App, root: Entity) -> HashMap<String, Transform> {
            // Sample a fixed timestamp, independent of asynchronous loading time.
            let mut stack = vec![root];
            let mut animated_targets = HashSet::new();
            let mut sampled_players = Vec::new();
            while let Some(entity) = stack.pop() {
                if let Some(children) = app.world().get::<Children>(entity) {
                    stack.extend(children.iter());
                }
                if let Some(mut player) = app.world_mut().get_mut::<AnimationPlayer>(entity) {
                    sampled_players.push(entity);
                    for (_, animation) in player.playing_animations_mut() {
                        animation.set_seek_time(0.6).pause();
                    }
                }
                if let (Some(transitions), Some(graph_handle)) = (
                    app.world().get::<AnimationTransitions>(entity),
                    app.world().get::<AnimationGraphHandle>(entity),
                ) {
                    let graphs = app.world().resource::<Assets<AnimationGraph>>();
                    let node = transitions.get_main_animation().and_then(|index| {
                        graphs.get(graph_handle).and_then(|graph| graph.get(index))
                    });
                    if let Some(bevy::animation::graph::AnimationGraphNode {
                        node_type: bevy::animation::graph::AnimationNodeType::Clip(handle),
                        ..
                    }) = node
                    {
                        let clip = app
                            .world()
                            .resource::<Assets<AnimationClip>>()
                            .get(handle)
                            .unwrap();
                        animated_targets.extend(clip.curves().keys().copied());
                    }
                }
            }
            app.update();
            let hips = app.world().get::<HipsBoneEntity>(root).unwrap().0;
            let mut stack = vec![hips];
            let mut result = HashMap::new();
            while let Some(entity) = stack.pop() {
                if let Some(children) = app.world().get::<Children>(entity) {
                    stack.extend(children.iter());
                }
                if let (Some(bone), Some(transform)) = (
                    app.world().get::<VrmBone>(entity),
                    app.world().get::<Transform>(entity),
                ) {
                    // Omitted bones retain the preceding pose; they are not
                    // outputs of this clip (Angry omits upperChest, for example).
                    if app
                        .world()
                        .get::<AnimationTargetId>(entity)
                        .is_some_and(|id| animated_targets.contains(id))
                    {
                        result.insert(bone.0.clone(), *transform);
                    }
                }
            }
            assert!(result.len() > 15, "must sample the live humanoid skeleton");
            // Resume after sampling: Bevy intentionally does not fade out a
            // paused outgoing action when the next clip is requested.
            for entity in sampled_players {
                let mut player = app.world_mut().get_mut::<AnimationPlayer>(entity).unwrap();
                for (_, animation) in player.playing_animations_mut() {
                    animation.resume();
                }
            }
            result
        }
        let key = "VRMA/Angry.vrma";
        let (reference, _) = spawn(&mut app, key, 0);
        wait(&mut app, |w| {
            w.resource::<PreviewRuntime>().active[key].started
        });
        let reference_pose = pose(&mut app, reference);
        app.world_mut()
            .resource_mut::<PreviewRuntime>()
            .active
            .clear();
        app.world_mut().entity_mut(reference).despawn();
        app.update();

        // Occupy node 1 with a different VRMA on the production rig. Without
        // per-instance IDs, the preview bake can select this rig's rest table.
        let handle = app
            .world()
            .resource::<AssetServer>()
            .load("vrm/nera_256.vrm");
        let desktop = app
            .world_mut()
            .spawn((
                crate::avatar::NeraAvatar,
                VrmHandle(handle),
                Transform::default(),
            ))
            .id();
        let idle = app
            .world()
            .resource::<AssetServer>()
            .load("animations/animasi_basic/idle.vrma");
        let idle_clip = app
            .world_mut()
            .spawn((VrmaHandle(idle), ChildOf(desktop)))
            .observe(on_preview_loaded)
            .observe(crate::animation::register_idle_animation)
            .id();
        wait(&mut app, |w| w.get::<PreviewClipReady>(idle_clip).is_some());
        let (preview, _) = spawn(&mut app, key, 0);
        let (other, _) = spawn(&mut app, "Clawatar_animations/161_Waving.vrma", 1);
        wait(&mut app, |w| {
            w.resource::<PreviewRuntime>()
                .active
                .values()
                .all(|s| s.started)
        });
        let preview_pose = pose(&mut app, preview);
        for (name, expected) in &reference_pose {
            let actual = preview_pose.get(name).unwrap();
            assert!(
                actual.rotation.angle_between(expected.rotation).abs() < 0.002,
                "rotation corrupted: {name}"
            );
            assert!(
                actual.translation.distance(expected.translation) < 0.002,
                "translation corrupted: {name}"
            );
        }
        let other_pose = pose(&mut app, other);
        assert!(
            preview_pose
                .iter()
                .any(|(name, tf)| other_pose.get(name).is_some_and(|other| tf
                    .rotation
                    .angle_between(other.rotation)
                    .abs()
                    > 0.05)),
            "different clips must have different poses"
        );

        app.world_mut().spawn((
            Interaction::Pressed,
            ManagerAction::Restart(key.into()),
            BackgroundColor::default(),
        ));
        wait(&mut app, |w| {
            matches!(
                w.resource::<NeraAnimationState>().visual_state(),
                VisualState::LibraryAnimation(_)
            )
        });
        // Allow PlayVrma to reach its target AnimationPlayer before sampling.
        for _ in 0..12 {
            app.update();
        }
        let desktop_pose = pose(&mut app, desktop);
        for (name, expected) in &reference_pose {
            let actual = desktop_pose.get(name).unwrap();
            assert!(
                actual.rotation.angle_between(expected.rotation).abs() < 0.002,
                "desktop did not play clicked clip: {name}"
            );
        }
        eprintln!(
            "Verified {} humanoid bones: isolated == concurrent preview == clicked desktop",
            reference_pose.len()
        );

        // Change selection while the first new file is loading, then check that
        // the final selection remains bound to its own graph node.
        app.world_mut()
            .write_message(crate::animation::PlayLibraryAnimation {
                path: "animations/animasi_basic/Relax.vrma".into(),
            });
        app.update();
        app.world_mut()
            .write_message(crate::animation::PlayLibraryAnimation {
                path: "animations/Clawatar_animations/161_Waving.vrma".into(),
            });
        wait(&mut app, |w| {
            let VisualState::LibraryAnimation(entity) =
                w.resource::<NeraAnimationState>().visual_state()
            else {
                return false;
            };
            w.get::<VrmaPath>(entity)
                .is_some_and(|p| p.0.ends_with("161_Waving.vrma"))
        });
        for _ in 0..12 {
            app.update();
        }
        let selected_pose = pose(&mut app, desktop);
        for (name, expected) in &other_pose {
            assert!(
                selected_pose[name]
                    .rotation
                    .angle_between(expected.rotation)
                    .abs()
                    < 0.002,
                "rapid click played the wrong clip: {name}"
            );
        }
        eprintln!("Verified rapid selection: the last clicked clip owns desktop playback");

        // Closing the manager must release cloned worlds/graphs/clips as well as
        // entities, while the desktop rig and its selected animation stay alive.
        use bevy::world_serialization::{WorldAsset, WorldAssetRoot};
        let roots = [preview, other];
        let mut scenes = Vec::new();
        let mut graphs = Vec::new();
        let mut clips = Vec::new();
        for root in roots {
            let mut stack = vec![root];
            while let Some(entity) = stack.pop() {
                if let Some(children) = app.world().get::<Children>(entity) {
                    stack.extend(children.iter());
                }
                if let Some(scene) = app.world().get::<WorldAssetRoot>(entity) {
                    scenes.push(scene.0.id());
                }
                if let Some(graph) = app.world().get::<AnimationGraphHandle>(entity) {
                    graphs.push(graph.0.id());
                    if let Some(graph) = app
                        .world()
                        .resource::<Assets<AnimationGraph>>()
                        .get(&graph.0)
                    {
                        for node in graph.graph.node_weights() {
                            if let bevy::animation::graph::AnimationNodeType::Clip(handle) =
                                &node.node_type
                            {
                                clips.push(handle.id());
                            }
                        }
                    }
                }
            }
        }
        assert!(!scenes.is_empty() && !graphs.is_empty() && !clips.is_empty());
        app.init_resource::<super::super::AnimationManagerInputGate>()
            .add_systems(PreUpdate, super::super::sync_manager_pointer_gate)
            .add_systems(Update, super::super::cleanup_closed_window);
        app.world_mut()
            .resource_mut::<super::super::AnimationManagerInputGate>()
            .window_open = true;
        wait(&mut app, |world| {
            world.resource::<PreviewRuntime>().is_empty()
                && scenes
                    .iter()
                    .all(|id| !world.resource::<Assets<WorldAsset>>().contains(*id))
                && graphs
                    .iter()
                    .all(|id| !world.resource::<Assets<AnimationGraph>>().contains(*id))
                && clips
                    .iter()
                    .all(|id| !world.resource::<Assets<AnimationClip>>().contains(*id))
        });
        assert!(app.world().get_entity(desktop).is_ok());
        eprintln!(
            "Verified manager cleanup: preview worlds, graphs and clips released; desktop preserved"
        );

        // Exercise actual Bevy layout with a large library, including the final
        // partial row. The scroll extent must stay constant as rows are recycled.
        use super::super::{ManagerCamera, ManagerWindow, ui};
        let target =
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(Image::new_target_texture(
                    1120,
                    980,
                    TextureFormat::Rgba8Unorm,
                    Some(TextureFormat::Rgba8UnormSrgb),
                ));
        app.world_mut().spawn((ManagerWindow, Window::default()));
        app.world_mut()
            .spawn((ManagerCamera, Camera2d, RenderTarget::Image(target.into())));
        {
            let mut state = app.world_mut().resource_mut::<ManagerState>();
            state.registry.entries = (0..10_000)
                .map(|i| super::super::registry::AnimationEntry {
                    key: format!("{i}.vrma"),
                    display_name: format!("Animation {i}"),
                    source: "Test".into(),
                    emotion: "Neutral",
                })
                .collect();
            state.dirty = true;
        }
        app.add_systems(Update, (ui::rebuild_ui, ui::sync_gallery_rows).chain());
        for _ in 0..5 {
            app.update();
        }
        let (scroll_entity, metrics) = {
            let world = app.world_mut();
            let mut query = world.query_filtered::<(Entity, &ComputedNode), With<GalleryScroll>>();
            let (entity, node) = query.single(world).unwrap();
            (entity, super::super::scroll_metrics(node))
        };
        assert!(
            metrics.1 > 400.0 && metrics.1 < 980.0,
            "gallery viewport must stay inside window: {metrics:?}"
        );
        assert!(
            metrics.0 > 1_000_000.0,
            "scroll extent must represent the full library"
        );
        app.world_mut()
            .get_mut::<ScrollPosition>(scroll_entity)
            .unwrap()
            .0
            .y = metrics.0;
        app.world_mut().resource_mut::<ManagerState>().scroll_y = metrics.0;
        for _ in 0..3 {
            app.update();
        }
        let world = app.world_mut();
        let mut query =
            world.query::<(&PreviewSlot, &ComputedNode, &bevy::ui::UiGlobalTransform)>();
        let cards: Vec<_> = query.iter(world).collect();
        assert!(cards.len() <= 15);
        let (_, node, transform) = cards
            .iter()
            .find(|(slot, _, _)| slot.index == 9999)
            .unwrap();
        assert_eq!(node.size().y, 280.0);
        let center = **transform * Vec2::ZERO;
        assert!(
            center.y > 0.0 && center.y < 980.0,
            "last card must actually be visible: {center:?}"
        );
    }
}
