//! Reproducible memory ladder for Project Nera.
//!
//! Run with: cargo run --bin mem_stages -- <B|C|D0|D1|D2|D3|D4|E|F0|F3|F4|G0|G3|G4>
use std::num::NonZeroU32;

use bevy::{
    app::AppExit,
    asset::AssetPlugin,
    prelude::*,
    render::{
        RenderPlugin,
        settings::{InstanceFlags, MemoryHints, WgpuSettings},
    },
    window::{PresentMode, WindowPlugin},
};
use bevy_vrm1::prelude::*;

const SAMPLE_FRAME: u32 = 300;

#[derive(Resource)]
struct Measurement {
    stage: &'static str,
    asset_stage: char,
    label: &'static str,
    frame: u32,
}

#[repr(C)]
#[derive(Default)]
struct ProcessMemory {
    cb: u32,
    faults: u32,
    peak_working_set: usize,
    working_set: usize,
    peak_paged_pool: usize,
    paged_pool: usize,
    peak_nonpaged_pool: usize,
    nonpaged_pool: usize,
    pagefile: usize,
    peak_pagefile: usize,
    private: usize,
}

#[link(name = "psapi")]
unsafe extern "system" {
    fn GetProcessMemoryInfo(process: isize, counters: *mut ProcessMemory, size: u32) -> i32;
}

fn sample_memory(
    mut measurement: ResMut<Measurement>,
    vrms: Query<(), With<Vrm>>,
    vrmas: Query<(), With<Vrma>>,
    mut exit: MessageWriter<AppExit>,
) {
    let assets_ready = match measurement.asset_stage {
        'F' => !vrms.is_empty(),
        'G' => !vrms.is_empty() && !vrmas.is_empty(),
        _ => true,
    };
    if !assets_ready {
        return;
    }
    measurement.frame += 1;
    if measurement.frame != SAMPLE_FRAME {
        return;
    }
    let mut memory = ProcessMemory::default();
    memory.cb = std::mem::size_of::<ProcessMemory>() as u32;
    assert_ne!(
        unsafe {
            GetProcessMemoryInfo(-1, &mut memory, std::mem::size_of::<ProcessMemory>() as u32)
        },
        0,
    );
    println!(
        "MEM_RESULT stage={} label=\"{}\" private_mib={:.1} working_set_mib={:.1} peak_working_set_mib={:.1} vrms={} vrmas={}",
        measurement.stage,
        measurement.label,
        memory.private as f64 / 1_048_576.0,
        memory.working_set as f64 / 1_048_576.0,
        memory.peak_working_set as f64 / 1_048_576.0,
        vrms.iter().count(),
        vrmas.iter().count(),
    );
    exit.write(AppExit::Success);
}

fn window_plugin() -> WindowPlugin {
    WindowPlugin {
        primary_window: Some(Window {
            title: "Nera memory ladder".into(),
            resolution: (600, 950).into(),
            present_mode: PresentMode::AutoVsync,
            desired_maximum_frame_latency: NonZeroU32::new(1),
            ..default()
        }),
        ..default()
    }
}

fn renderer_plugin() -> RenderPlugin {
    RenderPlugin {
        render_creation: WgpuSettings {
            instance_flags: InstanceFlags::VALIDATION_INDIRECT_CALL.with_env(),
            memory_hints: MemoryHints::MemoryUsage,
            ..default()
        }
        .into(),
        ..default()
    }
}

fn add_window_stack(app: &mut App) {
    app.add_plugins(MinimalPlugins)
        .add_plugins(bevy::log::LogPlugin::default())
        .add_plugins(bevy::input::InputPlugin)
        .add_plugins(window_plugin())
        .add_plugins(bevy::a11y::AccessibilityPlugin)
        .add_plugins(AssetPlugin {
            file_path: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets")
                .to_string_lossy()
                .into_owned(),
            ..default()
        })
        .add_plugins(bevy::winit::WinitPlugin::default());
}

fn add_renderer_stack(app: &mut App) {
    app.add_plugins(renderer_plugin())
        .add_plugins(bevy::image::ImagePlugin::default())
        .add_plugins(bevy::mesh::MeshPlugin)
        .add_plugins(bevy::camera::CameraPlugin)
        .add_plugins(bevy::light::LightPlugin)
        .add_plugins(bevy::gizmos::GizmoPlugin)
        .add_plugins(bevy::render::pipelined_rendering::PipelinedRenderingPlugin)
        .add_plugins(bevy::core_pipeline::CorePipelinePlugin);
}

#[derive(Copy, Clone, Debug)]
enum PbrVariant {
    Default,
    NoPrepass,
    NoDeferredLighting,
    NoGpuInstanceBuilder,
    Trimmed,
}

fn pbr_plugin(variant: PbrVariant) -> bevy::pbr::PbrPlugin {
    let mut plugin = bevy::pbr::PbrPlugin::default();
    match variant {
        PbrVariant::Default => {}
        PbrVariant::NoPrepass => plugin.prepass_enabled = false,
        PbrVariant::NoDeferredLighting => plugin.add_default_deferred_lighting_plugin = false,
        PbrVariant::NoGpuInstanceBuilder => plugin.use_gpu_instance_buffer_builder = false,
        PbrVariant::Trimmed => {
            plugin.prepass_enabled = false;
            plugin.add_default_deferred_lighting_plugin = false;
            plugin.use_gpu_instance_buffer_builder = false;
        }
    }
    plugin
}

fn add_pbr_stack(app: &mut App, variant: PbrVariant) {
    app.add_plugins(bevy::world_serialization::WorldSerializationPlugin)
        .add_plugins(bevy::animation::AnimationPlugin)
        .add_plugins(bevy::gltf::GltfPlugin::default())
        .add_plugins(pbr_plugin(variant));
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Msaa::Off,
        Transform::from_xyz(0.0, 0.8, 4.5).looking_at(Vec3::new(0.0, 0.8, 0.0), Vec3::Y),
    ));
}

fn spawn_pbr_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.3, 0.5, 0.9))),
    ));
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_rotation_x(-0.7)),
    ));
}

fn spawn_vrm(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn((
        VrmHandle(assets.load("vrm/nera_256.vrm")),
        Transform::from_xyz(0.0, -0.85, 0.0),
    ));
}

fn spawn_vrm_with_idle(mut commands: Commands, assets: Res<AssetServer>) {
    commands
        .spawn((
            VrmHandle(assets.load("vrm/nera_256.vrm")),
            Transform::from_xyz(0.0, -0.85, 0.0),
        ))
        .with_child((
            Name::new("Idle VRMA"),
            VrmaHandle(assets.load("animations/animasi_basic/idle.vrma")),
        ));
}

fn main() {
    let requested = std::env::args()
        .nth(1)
        .map(|argument| argument.to_ascii_uppercase())
        .unwrap_or_else(|| panic!("usage: mem_stages <B|C|D0|D1|D2|D3|D4|E|F|G>"));
    let (stage, stage_name, pbr_variant) = match requested.as_str() {
        "B" => ('B', "B", PbrVariant::Default),
        "C" => ('C', "C", PbrVariant::Default),
        "D" | "D0" => ('D', "D0", PbrVariant::Default),
        "D1" => ('D', "D1", PbrVariant::NoPrepass),
        "D2" => ('D', "D2", PbrVariant::NoDeferredLighting),
        "D3" => ('D', "D3", PbrVariant::NoGpuInstanceBuilder),
        "D4" => ('D', "D4", PbrVariant::Trimmed),
        "E" => ('E', "E", PbrVariant::Default),
        "F" | "F0" => ('F', "F0", PbrVariant::Default),
        "F3" => ('F', "F3", PbrVariant::NoGpuInstanceBuilder),
        "F4" => ('F', "F4", PbrVariant::Trimmed),
        "G" | "G0" => ('G', "G0", PbrVariant::Default),
        "G3" => ('G', "G3", PbrVariant::NoGpuInstanceBuilder),
        "G4" => ('G', "G4", PbrVariant::Trimmed),
        _ => panic!("usage: mem_stages <B|C|D0|D1|D2|D3|D4|E|F0|F3|F4|G0|G3|G4>"),
    };
    let label = match requested.as_str() {
        "B" => "Bevy + window, no renderer",
        "C" => "Window + minimal 3D renderer",
        "D" | "D0" => "+ PBR default",
        "D1" => "+ PBR: prepass disabled",
        "D2" => "+ PBR: deferred lighting disabled",
        "D3" => "+ PBR: GPU instance builder disabled",
        "D4" => "+ PBR: all three disabled",
        "E" => "+ VRM plugin, no model",
        "F" | "F0" => "+ nera_256.vrm, PBR default",
        "F3" => "+ nera_256.vrm, PBR D3",
        "F4" => "+ nera_256.vrm, PBR D4",
        "G" | "G0" => "+ idle VRMA, PBR default",
        "G3" => "+ idle VRMA, PBR D3",
        "G4" => "+ idle VRMA, PBR D4",
        _ => unreachable!(),
    };

    let mut app = App::new();
    add_window_stack(&mut app);
    if stage >= 'C' {
        add_renderer_stack(&mut app);
        app.add_systems(Startup, spawn_camera);
    }
    if stage >= 'D' {
        add_pbr_stack(&mut app, pbr_variant);
        app.add_systems(Startup, spawn_pbr_scene);
    }
    if stage >= 'E' {
        app.add_plugins((VrmPlugin, VrmaPlugin));
    }
    if stage == 'F' {
        app.add_systems(Startup, spawn_vrm);
    } else if stage == 'G' {
        app.add_systems(Startup, spawn_vrm_with_idle);
    }
    app.insert_resource(Measurement {
        stage: stage_name,
        asset_stage: stage,
        label,
        frame: 0,
    })
    .add_systems(Update, sample_memory)
    .run();
}
