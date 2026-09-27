use std::num::NonZeroU32;

use bevy::{
    prelude::*,
    render::{
        RenderPlugin,
        settings::{InstanceFlags, MemoryHints, WgpuSettings},
    },
    window::{PresentMode, WindowLevel, WindowPlugin},
};

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Nera Memory Baseline".into(),
                        resolution: (600, 950).into(),
                        transparent: true,
                        decorations: false,
                        window_level: WindowLevel::AlwaysOnTop,
                        present_mode: PresentMode::AutoVsync,
                        desired_maximum_frame_latency: NonZeroU32::new(1),
                        ..default()
                    }),
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: WgpuSettings {
                        instance_flags: InstanceFlags::VALIDATION_INDIRECT_CALL.with_env(),
                        memory_hints: MemoryHints::MemoryUsage,
                        ..default()
                    }
                    .into(),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(Color::NONE))
        .run();
}