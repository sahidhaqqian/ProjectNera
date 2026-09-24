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
const MAX_ACTIVE_PREVIEWS: usize = 15;
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

#[derive(Resource, Default)]
pub(super) struct PreviewRuntime {
    active: HashMap<String, PreviewScene>,
}

impl PreviewRuntime {
    pub fn remove(&mut self, key: &str, commands: &mut Commands, images: &mut Assets<Image>) {
        if let Some(scene) = self.active.remove(key) {
            commands.entity(scene.root).despawn();
            commands.entity(scene.camera).despawn();
            images.remove(&scene.image);
        }
    }

    pub fn clear(&mut self, commands: &mut Commands, images: &mut Assets<Image>) {
        let keys: Vec<_> = self.active.keys().cloned().collect();
        for key in keys {
            self.remove(&key, commands, images);
        }
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
    let visible_rows = (viewport_height / super::ui::ROW_HEIGHT).ceil().max(1.0) as usize;
    let total_rows = state.filtered().len().div_ceil(PREVIEW_COLUMNS);
    let active_rows = visible_rows
        .saturating_add(2)
        .min(MAX_ACTIVE_ROWS)
        .min(total_rows);
    let mut first_active_row = first_visible_row.saturating_sub(1);
    first_active_row = first_active_row.min(total_rows.saturating_sub(active_rows));
    let last_active_row = first_active_row.saturating_add(active_rows);

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

    let mut occupied: HashSet<_> = runtime.active.values().map(|scene| scene.slot).collect();
    for (slot, mut image_node) in slots {
        if !needed.contains(&slot.key) {
            continue;
        }
        if let Some(scene) = runtime.active.get(&slot.key) {
            image_node.image = scene.image.clone();
            continue;
        }
        let Some(index) = (0..MAX_ACTIVE_PREVIEWS).find(|index| !occupied.contains(index)) else {
            break;
        };
        occupied.insert(index);

        // Render layers isolate unit-scale rigs without offsetting their rest pose.
        let layer = RenderLayers::layer(index + 1);
        let image = images.add(Image::new_target_texture(
            512,
            228,
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
                Transform::from_xyz(0.0, 0.85, 3.4).looking_at(Vec3::new(0.0, 0.85, 0.0), Vec3::Y),
                layer.clone(),
                Msaa::Off,
            ))
            .with_children(|parent| {
                parent.spawn((
                    DirectionalLight {
                        illuminance: 10_000.0,
                        shadow_maps_enabled: false,
                        contact_shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.7, -0.5, 0.0)),
                    layer.clone(),
                ));
            })
            .id();
        let mut clip = Entity::PLACEHOLDER;
        let root = commands
            .spawn((
                Name::new("Nera Gallery Preview Avatar"),
                PreviewAvatarRoot,
                VrmHandle(assets.load("vrm/nera_256.vrm")),
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
                path: "animations/VRMA/Relax.vrma".into(),
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
    }
}
