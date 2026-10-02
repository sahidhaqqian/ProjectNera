//! Opt-in, read-only catalog audit. No character files/configuration are changed.
use bevy::{mesh::skinning::SkinnedMesh, prelude::*, world_serialization::WorldAsset};
use bevy_vrm1::prelude::*;

use super::{GalleryTab, ManagerState, ManagerWindow, preview::PreviewRuntime};

#[derive(Resource, Default)]
struct Audit {
    models: Vec<crate::character::library::CharacterEntry>,
    stage: usize,
    next: f64,
    label: String,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<Audit>()
        .add_systems(Update, advance.before(super::ui::rebuild_ui))
        .add_systems(Last, closed_snapshot);
}

fn snapshot(world: &mut World, label: &str) {
    let (cached, live) = world.resource::<PreviewRuntime>().model_counts();
    let entities = world.query::<Entity>().iter(world).count();
    let roots = world
        .query_filtered::<Entity, With<Vrm>>()
        .iter(world)
        .count();
    let cameras = world
        .query_filtered::<Entity, With<Camera3d>>()
        .iter(world)
        .count();
    assert_eq!(live, 0, "gallery must never retain live rigs");
    assert_eq!(roots, 0, "gallery must not load VRM scenes");
    assert_eq!(cameras, 0, "cached cards do not need 3D cameras");
    let players = world
        .query_filtered::<Entity, With<AnimationPlayer>>()
        .iter(world)
        .count();
    let skins = world.query::<&SkinnedMesh>().iter(world).count();
    let joints: usize = world
        .query::<&SkinnedMesh>()
        .iter(world)
        .map(|s| s.joints.len())
        .sum();
    let images = world.resource::<Assets<Image>>();
    let server = world.resource::<AssetServer>();
    let model_pixels: usize = images
        .iter()
        .filter(|(id, _)| {
            server
                .get_path(*id)
                .is_some_and(|path| path.path().extension().is_some_and(|ext| ext == "vrm"))
        })
        .map(|(_, image)| image.data.as_ref().map_or(0, Vec::len))
        .sum();
    let model_geometry: usize = world
        .resource::<Assets<Mesh>>()
        .iter()
        .filter(|(id, _)| {
            server
                .get_path(*id)
                .is_some_and(|path| path.path().extension().is_some_and(|ext| ext == "vrm"))
        })
        .map(|(_, mesh)| {
            mesh.try_attributes().map_or(0, |attributes| {
                attributes
                    .map(|(_, values)| values.get_bytes().len())
                    .sum::<usize>()
            }) + mesh
                .try_morph_targets()
                .map_or(0, |morphs| std::mem::size_of_val(morphs.as_slice()))
        })
        .sum();
    let cpu_pixels: usize = images
        .iter()
        .map(|(_, i)| i.data.as_ref().map_or(0, Vec::len))
        .sum();
    let texture_bytes: u64 = images
        .iter()
        .map(|(_, i)| {
            let d = &i.texture_descriptor;
            let (block_width, block_height) = d.format.block_dimensions();
            u64::from(d.size.width.div_ceil(block_width))
                * u64::from(d.size.height.div_ceil(block_height))
                * u64::from(d.size.depth_or_array_layers)
                * u64::from(d.format.block_copy_size(None).unwrap_or(0))
        })
        .sum();
    let (private, working_set, peak_commit) = super::process::memory_usage();
    let record = serde_json::json!({
        "stage": label, "pid": std::process::id(),
        "private_mib": private as f64 / 1048576.0,
        "working_set_mib": working_set as f64 / 1048576.0,
        "peak_commit_mib": peak_commit as f64 / 1048576.0,
        "model_cpu_pixels_mib": model_pixels as f64 / 1048576.0,
        "model_cpu_vertices_morphs_mib": model_geometry as f64 / 1048576.0,
        "cards": cached, "live": live, "entities": entities, "vrm_roots": roots,
        "animated_cards": world.resource::<PreviewRuntime>().animated_count(),
        "compressed_frames_mib": world.resource::<PreviewRuntime>().compressed_bytes() as f64 / 1048576.0,
        "frame_counts": world.resource::<PreviewRuntime>().idle_samples().iter().map(|(_, count)| *count).collect::<Vec<_>>(),
        "cameras_3d": cameras, "skins": skins, "joints": joints, "players": players,
        "images": images.len(), "cpu_pixels_mib": cpu_pixels as f64 / 1048576.0,
        "texture_base_mib_estimate": texture_bytes as f64 / 1048576.0,
        "meshes": world.resource::<Assets<Mesh>>().len(),
        "standard_materials": world.resource::<Assets<StandardMaterial>>().len(),
        "mtoon_materials": world.resource::<Assets<MToonMaterial>>().len(),
        "world_assets": world.resource::<Assets<WorldAsset>>().len(),
        "vrm_assets": world.resource::<Assets<VrmAsset>>().len(),
        "vrma_assets": world.resource::<Assets<VrmaAsset>>().len(),
        "graphs": world.resource::<Assets<AnimationGraph>>().len(),
        "clips": world.resource::<Assets<AnimationClip>>().len(),
    });
    eprintln!("NERA_MANAGER_MEMORY {record}");
}

fn closed_snapshot(world: &mut World) {
    if world.resource::<Audit>().label != "closed" {
        return;
    }
    if world
        .query_filtered::<Entity, With<ManagerWindow>>()
        .iter(world)
        .next()
        .is_none()
    {
        snapshot(world, "closed");
        assert!(world.resource::<PreviewRuntime>().is_empty());
        world.resource_mut::<Audit>().label = "finished".into();
        world.write_message(AppExit::Success);
    }
}

fn advance(world: &mut World) {
    let now = world.resource::<Time<Real>>().elapsed_secs_f64();
    if now < world.resource::<Audit>().next {
        return;
    }
    world.resource_scope(|world, mut audit: Mut<Audit>| {
        if audit.stage == 0 {
            let mut state = world.resource_mut::<ManagerState>();
            audit.models = std::mem::take(&mut state.characters.models);
            state.tab = GalleryTab::Characters;
            for slot in &mut state.characters.slots {
                slot.path = None;
            }
            state.dirty = true;
            audit.label = "empty_manager".into();
            audit.stage = 1;
            audit.next = now + 6.0;
            return;
        }
        snapshot(world, &audit.label);
        let count = audit.models.len();
        if audit.stage <= count {
            let index = audit.stage - 1;
            let mut state = world.resource_mut::<ManagerState>();
            state.characters.models.push(audit.models[index].clone());
            if index == 0 {
                state.characters.slots[0].path = Some(audit.models[0].path.clone());
            }
            state.dirty = true;
            audit.label = format!("characters_{}", index + 1);
        } else if audit.stage == count + 1 {
            let mut state = world.resource_mut::<ManagerState>();
            state.characters.slots[0].path = audit.models.last().map(|m| m.path.clone());
            state.selected_character = audit
                .models
                .last()
                .map(|m| m.path.clone())
                .unwrap_or_default();
            state.dirty = true;
            audit.label = "selection_changed".into();
        } else if audit.stage == count + 2 {
            let largest = audit
                .models
                .iter()
                .max_by_key(|model| {
                    crate::character::preview_model_file(&model.path)
                        .and_then(|path| path.metadata().ok())
                        .map_or(0, |m| m.len())
                })
                .map(|model| model.path.clone());
            let mut state = world.resource_mut::<ManagerState>();
            state.selected_character = largest.clone().unwrap_or_default();
            state.characters.slots[0].path = largest;
            state.dirty = true;
            audit.label = "largest_model_selected".into();
        } else if audit.stage == count + 3 {
            let mut state = world.resource_mut::<ManagerState>();
            state.characters.models.truncate(1);
            state.scroll_y = 0.0;
            state.characters.slots[0].path = audit.models.first().map(|m| m.path.clone());
            state.dirty = true;
            audit.label = "cards_removed".into();
        } else if audit.stage == count + 4 {
            for mut window in world
                .query_filtered::<&mut Window, With<ManagerWindow>>()
                .iter_mut(world)
            {
                window.visible = false;
            }
            audit.label = "hidden".into();
        } else if audit.stage == count + 5 {
            world.resource_mut::<ManagerState>().close_requested = true;
            audit.label = "closed".into();
        } else {
            assert!(world.resource::<PreviewRuntime>().is_empty());
            world.write_message(AppExit::Success);
            audit.next = f64::INFINITY;
            return;
        }
        audit.stage += 1;
        audit.next = now + 7.0;
    });
}
