mod alarm;
mod animation;
mod avatar;
mod benchmark;
mod character;
mod memory;

mod desktop_physics;

#[cfg(target_os = "windows")]
mod action_menu;
#[cfg(target_os = "windows")]
mod alarm_dismiss;
#[cfg(target_os = "windows")]
mod alarm_settings;
#[cfg(target_os = "windows")]
mod animation_manager;
mod environment;
mod expression;
mod interaction;
mod pet_behavior;
mod platform;
mod presentation;
mod rng;
mod settings;
mod startup_entrance;
mod utility_window;
mod vrm0_migration;
mod walking;

#[cfg(target_os = "windows")]
mod tray;

use animation::{
    NeraAnimationPlugin, apply_animation_state, sync_environment_animation_state,
    sync_interaction_animation_actions, tick_animation_actions,
};
use avatar::{
    DeathVisualGrounding, setup_camera, setup_lighting, spawn_nera, sync_look_at_for_animation,
    update_avatar_anchors, update_look_at_target,
};

use environment::DesktopEnvironmentPlugin;

use expression::NeraExpressionPlugin;

use interaction::{
    DesktopPetInteraction, HeadPatGesture, HoveredBodyPart, InteractionPlugin,
    attach_bone_colliders, detect_head_pat, drag_nera_window, mark_nera_live_bones,
    nera_utility_input_available, nera_utility_input_blocked, settle_pending_sit,
    suspend_nera_interaction_for_settings, update_click_through,
};

use std::num::NonZeroU32;

use bevy::{
    asset::AssetPlugin,
    pbr::PbrPlugin,
    prelude::*,
    render::RenderPlugin,
    settings::SettingsPlugin,
    window::{CursorOptions, PresentMode, WindowLevel, WindowPlugin, WindowPosition},
};

use bevy_vrm1::prelude::*;

use presentation::PresentationPlugin;
use settings::{NeraSettings, NeraSettingsPlugin};

#[cfg(target_os = "windows")]
use action_menu::ActionMenuPlugin;
#[cfg(target_os = "windows")]
use animation_manager::process::AnimationManagerHostPlugin;

#[cfg(target_os = "windows")]
use alarm_dismiss::AlarmDismissPlugin;

#[cfg(target_os = "windows")]
use alarm_settings::AlarmSettingsPlugin;

#[cfg(target_os = "windows")]
use tray::TrayPlugin;

/// Bevy 0.19 resolves file assets relative to the executable directory when
/// BEVY_ASSET_ROOT / CARGO_MANIFEST_DIR are not present at runtime. `cargo run`
/// therefore points at `target/debug/assets`, while Project Nera keeps assets
/// in the repository root. Prefer the repository assets while developing, and
/// fall back to a normal `assets` folder beside the executable for packaged builds.
fn nera_asset_path() -> String {
    let development_assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");

    if development_assets.is_dir() {
        development_assets.to_string_lossy().into_owned()
    } else {
        "assets".to_string()
    }
}

fn main() {
    #[cfg(target_os = "windows")]
    if std::env::args().any(|arg| arg == "--animation-manager-worker") {
        animation_manager::process::run_worker();
        return;
    }
    let memory_benchmark_d3 = std::env::args().any(|arg| arg == "--memory-benchmark-d3");
    let memory_benchmark_default = std::env::args().any(|arg| arg == "--memory-benchmark");
    let benchmark = memory_benchmark_d3 || memory_benchmark_default;
    // Nera draws one animated character, so the CPU MeshUniform path uses less
    // memory than Bevy's GPU instance-buffer builder. `--memory-benchmark`
    // deliberately restores Bevy's default to keep the A/B baseline runnable.
    let use_gpu_instance_buffer_builder = memory_benchmark_default && !memory_benchmark_d3;
    let mut app = App::new();

    // Load persistent Nera settings before the native window is created.
    //
    // Bevy SettingsPlugin performs its load immediately when the plugin is
    // added, so `NeraSettings` is already available here. This lets the HWND
    // be born in the correct visibility state instead of relying on a later
    // hidden -> visible transition that can race native window creation.
    app.add_plugins(SettingsPlugin::new("io.github.sahidhaqqi.projectnera"));

    let launch_visible = !app.world().resource::<NeraSettings>().start_hidden;
    character::register_asset_source(&mut app);

    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: nera_asset_path(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Nera".into(),
                    transparent: true,
                    decorations: false,
                    window_level: WindowLevel::AlwaysOnTop,
                    resolution: (600, 950).into(),
                    resizable: false,
                    position: WindowPosition::At(IVec2::new(200, 200)),
                    present_mode: PresentMode::AutoVsync,
                    desired_maximum_frame_latency: NonZeroU32::new(1),
                    has_shadow: false,
                    skip_taskbar: true,
                    // Native startup visibility comes directly from the
                    // already-loaded persistent setting. There is no later
                    // reveal/hide transition, so:
                    //
                    // Start Hidden OFF -> HWND is created visible.
                    // Start Hidden ON  -> HWND is created hidden.
                    visible: launch_visible,
                    ..default()
                }),
                primary_cursor_options: Some(CursorOptions {
                    hit_test: false,
                    ..default()
                }),
                ..default()
            })
            .set(RenderPlugin {
                render_creation: memory::renderer_settings().into(),
                ..default()
            })
            .set(PbrPlugin {
                use_gpu_instance_buffer_builder,
                ..default()
            }),
    );

    app.add_plugins((
        VrmPlugin,
        VrmaPlugin,
        memory::AvatarMemoryPlugin,
        DesktopEnvironmentPlugin,
        InteractionPlugin,
        NeraAnimationPlugin,
        NeraExpressionPlugin,
        PresentationPlugin,
        alarm::AlarmPlugin,
        NeraSettingsPlugin,
        character::CharacterPlugin,
    ));

    #[cfg(target_os = "windows")]
    app.add_plugins((
        TrayPlugin,
        ActionMenuPlugin,
        AnimationManagerHostPlugin,
        AlarmDismissPlugin,
        AlarmSettingsPlugin,
        utility_window::UtilityWindowPlugin,
    ));

    app.insert_resource(ClearColor(Color::NONE))
        .init_resource::<DesktopPetInteraction>()
        .init_resource::<HoveredBodyPart>()
        .init_resource::<HeadPatGesture>()
        .init_resource::<desktop_physics::DesktopPhysicsState>()
        .init_resource::<walking::WalkingRuntime>()
        .init_resource::<pet_behavior::PetBehavior>()
        .init_resource::<startup_entrance::StartupEntrance>()
        .init_resource::<DeathVisualGrounding>()
        .add_systems(Startup, (setup_camera, setup_lighting, spawn_nera))
        .add_systems(
            Update,
            (
                mark_nera_live_bones,
                attach_bone_colliders,
                update_look_at_target,
                // A secondary Settings window owns application mouse input while
                // it is open. Reset Nera capture state first so Settings clicks
                // cannot become body hits / drags through global ButtonInput.
                suspend_nera_interaction_for_settings.run_if(nera_utility_input_blocked),
                // Native placement changes first, then Phase 5.3 derives the
                // base pose, consumes temporary interaction actions, resolves
                // priority, and finally applies the correct VRMA/anchor.
                drag_nera_window
                    .run_if(presentation::desktop_pet_active)
                    .run_if(startup_entrance::pet_is_revealed)
                    .run_if(nera_utility_input_available),
                startup_entrance::prepare_startup_entrance,
                desktop_physics::update_desktop_physics
                    .run_if(presentation::desktop_pet_active)
                    .run_if(startup_entrance::physics_allowed),
                sync_interaction_animation_actions,
                // One scheduler consumes current support/interaction facts.
                // Always run cancellation, including hidden/attention frames.
                pet_behavior::update_pet_behavior,
                sync_environment_animation_state,
                walking::update_walking,
                tick_animation_actions,
                // Start the wave after aging existing actions, so its first
                // frame retains the clip's full playback duration.
                startup_entrance::advance_startup_entrance,
                sync_look_at_for_animation,
                apply_animation_state,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            (avatar::apply_walking_pose, avatar::preserve_seated_hips)
                .after(bevy::app::AnimationSystems)
                .after(animation::smooth_walking_cycle)
                .before(VrmSystemSets::Constraints),
        )
        .add_systems(
            PostUpdate,
            update_click_through
                .after(VrmSystemSets::SpringBone)
                .run_if(startup_entrance::pet_is_revealed)
                .run_if(presentation::desktop_pet_active)
                .run_if(nera_utility_input_available),
        )
        .add_systems(
            PostUpdate,
            detect_head_pat
                .after(update_click_through)
                .run_if(startup_entrance::pet_is_revealed)
                .run_if(presentation::desktop_pet_active)
                .run_if(nera_utility_input_available),
        )
        // Pose-first support calibration: Sitting_Enter starts in Update,
        // then Last samples its animated Seat and commits the attachment.
        // The attachment keeps Seat aligned for the rest of the entry clip.
        .add_systems(
            Last,
            (
                update_avatar_anchors,
                settle_pending_sit.run_if(presentation::desktop_pet_active),
            )
                .chain(),
        );

    if benchmark {
        app.add_plugins(benchmark::FullBenchmarkPlugin {
            d3: !use_gpu_instance_buffer_builder,
        });
    }

    app.run();
}
