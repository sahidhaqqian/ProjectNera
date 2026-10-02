//! Keep immutable VRM geometry on the GPU without retaining a second CPU copy.
//! Nera uses bone-based hit testing and never requests first-person mesh splitting.
use bevy::{asset::RenderAssetUsages, prelude::*};

/// Use the same renderer diagnostics policy in development and packaged pets.
/// Native debug layers keep shader/resource tracking data for the process's
/// lifetime. Opt back in with WGPU_DEBUG=1 / WGPU_VALIDATION=1 when diagnosing GPU bugs.
pub(crate) fn renderer_settings() -> bevy::render::settings::WgpuSettings {
    use bevy::render::settings::{InstanceFlags, MemoryHints, WgpuSettings};
    WgpuSettings {
        // Keep indirect-call validation (required by DX12) and wgpu's own
        // validation; only native debug layers and shader debug data are opt-in.
        instance_flags: InstanceFlags::VALIDATION_INDIRECT_CALL.with_env(),
        memory_hints: MemoryHints::MemoryUsage,
        ..default()
    }
}

pub(crate) struct AvatarMemoryPlugin;

impl Plugin for AvatarMemoryPlugin {
    fn build(&self, app: &mut App) {
        // Asset messages are flushed in PostUpdate. Last runs before render
        // extraction, including the very first upload (no extra upload/clone).
        app.add_systems(
            Last,
            (release_avatar_mesh_copies, release_avatar_texture_copies),
        );
    }
}

fn is_vrm_asset(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("vrm"))
        || path.ends_with("vrm/Cloud/nera_cloud_1024.glb")
}

pub(crate) fn release_avatar_mesh_copies(
    mut events: MessageReader<AssetEvent<Mesh>>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for event in events.read() {
        let AssetEvent::Added { id } = event else {
            continue;
        };
        if !server
            .get_path(*id)
            .is_some_and(|path| is_vrm_asset(path.path()))
        {
            continue;
        }
        // The Added event already schedules extraction. Do not emit Modified:
        // that would try to upload the now-extracted geometry again next frame.
        if let Some(mesh) = meshes.get_mut_untracked(*id) {
            mesh.asset_usage = RenderAssetUsages::RENDER_WORLD;
        }
    }
}

fn release_avatar_texture_copies(
    mut events: MessageReader<AssetEvent<Image>>,
    server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    for event in events.read() {
        let AssetEvent::Added { id } = event else {
            continue;
        };
        if server
            .get_path(*id)
            .is_some_and(|path| is_vrm_asset(path.path()))
        {
            // Keep dimensions/sampler metadata; let Bevy move only pixel data
            // to the renderer. Render targets and dynamic UI images are excluded.
            if let Some(image) = images.get_mut_untracked(*id) {
                image.asset_usage = RenderAssetUsages::RENDER_WORLD;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    fn report_process_memory() {
        // PROCESS_MEMORY_COUNTERS_EX; kept test-only to avoid runtime polling.
        #[repr(C)]
        #[derive(Default)]
        struct Counters {
            size: u32,
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
            fn GetProcessMemoryInfo(process: isize, counters: *mut Counters, size: u32) -> i32;
        }
        let mut counters = Counters::default();
        counters.size = std::mem::size_of::<Counters>() as u32;
        assert_ne!(
            unsafe {
                GetProcessMemoryInfo(-1, &mut counters, std::mem::size_of::<Counters>() as u32)
            },
            0
        );
        eprintln!(
            "MEMORY working_set_mib={} private_mib={}",
            counters.ws / 1048576,
            counters.private / 1048576
        );
    }

    #[test]
    fn only_vrm_geometry_is_eligible() {
        for path in ["vrm/nera_256.vrm", "imported/Character.VRM"] {
            assert!(is_vrm_asset(std::path::Path::new(path)));
        }
        for path in ["shadow", "model.glb", "clip.vrma", "image.png"] {
            assert!(!is_vrm_asset(std::path::Path::new(path)));
        }
    }

    #[test]
    #[ignore = "loads local VRM and initializes the native GPU"]
    fn avatar_geometry_survives_gpu_only_upload_and_respawn() {
        use bevy::{
            app::PluginsState, camera::RenderTarget, render::render_resource::TextureFormat,
            winit::WinitPlugin,
        };
        use bevy_vrm1::prelude::*;
        let baseline = std::env::var_os("NERA_MEMORY_BASELINE").is_some();
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::render::RenderPlugin {
                    render_creation: if baseline {
                        bevy::render::settings::WgpuSettings {
                            memory_hints: bevy::render::settings::MemoryHints::MemoryUsage,
                            ..default()
                        }
                    } else {
                        renderer_settings()
                    }
                    .into(),
                    ..default()
                })
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
        app.add_plugins((VrmPlugin, VrmaPlugin));
        if !baseline {
            app.add_plugins(AvatarMemoryPlugin);
        }
        while app.plugins_state() != PluginsState::Ready {
            bevy::tasks::tick_global_task_pools_on_main_thread();
        }
        app.finish();
        app.cleanup();
        let handle = app
            .world()
            .resource::<AssetServer>()
            .load(crate::character::DEFAULT_CHARACTER);
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
            Msaa::Off,
            RenderTarget::Image(target.into()),
            Transform::from_xyz(0.0, 0.8, 4.5).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
        ));
        for _ in 0..2 {
            let root = app
                .world_mut()
                .spawn((VrmHandle(handle.clone()), Transform::default()))
                .id();
            let mut frame_times = Vec::new();
            for frame in 0..180 {
                let start = std::time::Instant::now();
                app.update();
                if frame >= 120 {
                    frame_times.push(start.elapsed());
                }
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
            frame_times.sort();
            eprintln!(
                "MEMORY median_update_us={}",
                frame_times[frame_times.len() / 2].as_micros()
            );
            assert!(
                app.world().get::<Vrm>(root).is_some(),
                "avatar must initialize, including on respawn"
            );
            let world = app.world_mut();
            let meshes = world.resource::<Assets<Mesh>>();
            let mut count = 0;
            let mut morph_bytes = 0;
            for (id, mesh) in meshes.iter() {
                if world
                    .resource::<AssetServer>()
                    .get_path(id)
                    .is_some_and(|p| is_vrm_asset(p.path()))
                {
                    count += 1;
                    if baseline {
                        morph_bytes += mesh
                            .morph_targets()
                            .map_or(0, |m| std::mem::size_of_val(m.as_slice()));
                    } else {
                        assert_eq!(mesh.asset_usage, RenderAssetUsages::RENDER_WORLD);
                        assert!(mesh.try_attribute(Mesh::ATTRIBUTE_POSITION).is_err());
                    }
                }
            }
            assert!(count > 0);
            let mut pixel_bytes = 0;
            let mut texture_count = 0;
            for (id, image) in world.resource::<Assets<Image>>().iter() {
                if world
                    .resource::<AssetServer>()
                    .get_path(id)
                    .is_some_and(|p| is_vrm_asset(p.path()))
                {
                    texture_count += 1;
                    pixel_bytes += image.data.as_ref().map_or(0, Vec::len);
                    if !baseline {
                        assert!(
                            image.data.is_none(),
                            "avatar texture pixels must leave CPU RAM"
                        );
                        assert!(image.width() > 0 && image.height() > 0);
                    }
                }
            }
            assert!(texture_count > 0);
            eprintln!("MEMORY avatar_textures={texture_count} retained_pixel_bytes={pixel_bytes}");
            eprintln!(
                "MEMORY baseline={baseline} avatar_meshes={count} retained_morph_bytes={morph_bytes}"
            );
            #[cfg(windows)]
            report_process_memory();
            world.despawn(root);
            let gpu_meshes = app
                .sub_app(bevy::render::RenderApp)
                .world()
                .resource::<bevy::render::render_asset::RenderAssets<
                bevy::render::mesh::RenderMesh,
            >>();
            assert!(
                gpu_meshes
                    .iter()
                    .any(|(_, mesh)| mesh.has_morph_targets() && mesh.vertex_count > 0),
                "GPU must retain expression geometry after CPU data is released"
            );
        }
    }
}
