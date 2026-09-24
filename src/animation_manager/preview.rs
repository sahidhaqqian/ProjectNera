//! A bounded pool of independent VRM worlds rendered into card textures.
//! The positions are spaced beyond each camera's far plane, so cameras cannot
//! see one another's avatar or the production desktop avatar.
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use bevy::{
    animation::RepeatAnimation, camera::RenderTarget, prelude::*,
    render::render_resource::TextureFormat,
};
use bevy_vrm1::prelude::*;

use super::{GalleryScroll, ManagerState, PreviewSlot};

const PREVIEW_COLUMNS: usize = 3;
const MAX_ACTIVE_PREVIEWS: usize = 15;
const MAX_ACTIVE_ROWS: usize = MAX_ACTIVE_PREVIEWS / PREVIEW_COLUMNS;

struct PreviewScene {
    root: Entity,
    camera: Entity,
    clip: Entity,
    image: Handle<Image>,
    slot: usize,
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
    play_preview(&mut commands, trigger.vrma, false);
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
    let visible_rows = (viewport_height / super::ui::ROW_HEIGHT)
        .ceil()
        .max(1.0) as usize;
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
        let x = 100.0 + index as f32 * 100.0;
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
                    fov: 35.0_f32.to_radians(),
                    far: 45.0,
                    ..default()
                }),
                Transform::from_xyz(x, 0.72, 2.65)
                    .looking_at(Vec3::new(x, 0.55, 0.0), Vec3::Y),
                Msaa::Off,
            ))
            .id();
        let mut clip = Entity::PLACEHOLDER;
        let root = commands
            .spawn((
                Name::new("Nera Gallery Preview Avatar"),
                VrmHandle(assets.load("vrm/nera_256.vrm")),
                Transform::from_xyz(x, -0.83, 0.0),
            ))
            .with_children(|parent| {
                clip = parent
                    .spawn((
                        Name::new("Nera Gallery Preview VRMA"),
                        VrmaHandle(assets.load(format!("animations/{}", slot.key))),
                    ))
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
            },
        );
    }
}
