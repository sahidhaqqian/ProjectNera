mod alarm;
mod animation;
mod application_assets;
#[cfg(target_os = "windows")]
mod application_icon;
mod avatar;
mod benchmark;
mod character;
mod cloud;
mod dance_pool;
mod diagnostics;
mod focus;
mod memory;
mod music;
mod treat;

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
#[cfg(debug_assertions)]
mod feature_smoke;
mod interaction;
mod pet_behavior;
mod pet_feedback;
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
    AvatarVisualGrounding, setup_camera, setup_lighting, spawn_nera, sync_look_at_for_animation,
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

/// Packaged assets always take precedence; the repository fallback is debug-only.
fn nera_asset_path() -> String {
    application_assets::directory()
        .to_string_lossy()
        .into_owned()
}

fn nera_asset_directory() -> std::path::PathBuf {
    let path = std::path::PathBuf::from(nera_asset_path());
    if path.is_absolute() {
        path
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.join(&path)))
            .unwrap_or(path)
    }
}

fn nera_asset_file(path: &str) -> std::path::PathBuf {
    nera_asset_directory().join(path)
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
    // Keep Bevy's default GPU instance-buffer path in production. The D3
    // variant saves some memory, but it has not passed Nera's visual smoke
    // test and can make the VRM disappear. Restrict it to the explicit
    // benchmark flag so normal launches remain render-correct.
    let use_gpu_instance_buffer_builder = !memory_benchmark_d3;
    let mut app = App::new();
    // Settings load before windows are created; install diagnostics first so
    // malformed/missing configuration is reported instead of silently lost.
    app.add_plugins(bevy::log::LogPlugin {
        filter: "warn,nera=info,chibi_validation=info,touch_validation=info".into(),
        custom_layer: diagnostics::layer,
        ..default()
    });
    let missing = application_assets::missing_core(&nera_asset_directory());
    if !missing.is_empty() {
        diagnostics::startup_failure(&format!(
            "Required runtime files are missing: {}. Extract the complete Nera ZIP and keep nera.exe beside its assets folder.",
            missing.join(", ")
        ));
        std::process::exit(1);
    }
    #[cfg(debug_assertions)]
    let feature_smoke = std::env::args().any(|arg| arg == "--feature-smoke");
    #[cfg(debug_assertions)]
    // Scripted harnesses inject semantic/window events; never capture the
    // user's physical pointer while they work in another application.
    let scripted_pointer = feature_smoke
        || std::env::args().any(|arg| {
            matches!(
                arg.as_str(),
                "--settings-memory-audit" | "--alarm-memory-audit"
            )
        })
        || std::env::var_os("NERA_GALLERY_HOST_SMOKE_TEST").is_some();
    #[cfg(not(debug_assertions))]
    let scripted_pointer = false;
    #[cfg(debug_assertions)]
    let garment_gravity =
        !feature_smoke || !std::env::args().any(|arg| arg == "--garment-spring-baseline");
    #[cfg(not(debug_assertions))]
    let garment_gravity = true;

    // Load persistent Nera settings before the native window is created.
    //
    // Bevy SettingsPlugin performs its load immediately when the plugin is
    // added, so `NeraSettings` is already available here. This lets the HWND
    // be born in the correct visibility state instead of relying on a later
    // hidden -> visible transition that can race native window creation.
    let settings_id = character::library::instance_id().map_or_else(
        || "io.github.sahidhaqqi.projectnera".to_owned(),
        |id| format!("io.github.sahidhaqqi.projectnera.instance.{id}"),
    );
    #[cfg(debug_assertions)]
    let settings_id = if feature_smoke {
        format!(
            "io.github.sahidhaqqi.projectnera.smoke.{}",
            std::process::id()
        )
    } else {
        settings_id
    };
    app.add_plugins(SettingsPlugin::new(&settings_id));
    character::library::configure_instance(&mut app.world_mut().resource_mut::<NeraSettings>());

    let launch_visible = !app.world().resource::<NeraSettings>().start_hidden;
    character::register_asset_source(&mut app);

    app.add_plugins(
        DefaultPlugins
            .build()
            .disable::<bevy::log::LogPlugin>()
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

    #[cfg(target_os = "windows")]
    app.add_plugins(application_icon::ApplicationIconPlugin);

    app.add_plugins((
        VrmPlugin,
        VrmaPlugin,
        memory::AvatarMemoryPlugin,
        DesktopEnvironmentPlugin,
        InteractionPlugin,
        NeraAnimationPlugin,
        music::MusicPlugin,
        NeraExpressionPlugin,
        PresentationPlugin,
        alarm::AlarmPlugin,
        NeraSettingsPlugin,
        character::CharacterPlugin,
        cloud::CloudPlugin,
        pet_feedback::PetFeedbackPlugin,
        avatar::body_mode::AvatarBodyModePlugin,
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
        .add_plugins((focus::FocusPlugin, treat::TreatPlugin))
        .init_resource::<DesktopPetInteraction>()
        .init_resource::<HoveredBodyPart>()
        .init_resource::<HeadPatGesture>()
        .init_resource::<desktop_physics::DesktopPhysicsState>()
        .init_resource::<walking::WalkingRuntime>()
        .init_resource::<pet_behavior::PetBehavior>()
        .add_plugins(pet_behavior::living::LivingPlugin)
        .insert_resource(dance_pool::DancePool::load(&dance_pool::DancePool::file()))
        .init_resource::<startup_entrance::StartupEntrance>()
        .init_resource::<AvatarVisualGrounding>()
        .add_systems(PreUpdate, avatar::restore_visual_grounding)
        .add_systems(Startup, (setup_camera, setup_lighting, spawn_nera))
        .add_systems(
            Update,
            (
                mark_nera_live_bones,
                attach_bone_colliders,
                pet_behavior::living::sample_pointer,
                update_look_at_target,
                // Utility hover owns new gestures. A captured pet drag keeps
                // its release; mouse-down provenance stays PrimaryWindow-only.
                suspend_nera_interaction_for_settings.run_if(nera_utility_input_blocked),
                cloud::update_cloud,
                // Native placement changes first, then Phase 5.3 derives the
                // base pose, consumes temporary interaction actions, resolves
                // priority, and finally applies the correct VRMA/anchor.
                drag_nera_window
                    .run_if(move || !scripted_pointer)
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
                animation::screensaver::update_screensaver_animation,
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
            avatar::preserve_grounded_pose
                .after(VrmSystemSets::PropagateAfterConstraints)
                .after(bevy::camera::CameraUpdateSystems)
                .before(VrmSystemSets::GazeControl),
        )
        .add_systems(
            PostUpdate,
            avatar::apply_garment_spring_gravity
                .run_if(move || garment_gravity)
                .before(VrmSystemSets::SpringBone),
        )
        .add_systems(
            PostUpdate,
            update_click_through
                .run_if(move || !scripted_pointer)
                .after(VrmSystemSets::SpringBone)
                .run_if(startup_entrance::pet_is_revealed)
                .run_if(presentation::avatar_hit_testing_active)
                .run_if(nera_utility_input_available),
        )
        .add_systems(
            PostUpdate,
            detect_head_pat
                .run_if(move || !scripted_pointer)
                .after(update_click_through)
                .run_if(startup_entrance::pet_is_revealed)
                .run_if(presentation::avatar_hit_testing_active)
                .run_if(nera_utility_input_available),
        )
        // Pose-first support calibration: Sitting_Enter starts in Update,
        // then Last samples its animated Seat and commits the attachment.
        // The attachment keeps Seat aligned for the rest of the entry clip.
        .add_systems(
            Last,
            (
                // Attention camera pixels are presentation geometry, not
                // desktop collision/seat calibration.
                update_avatar_anchors.run_if(presentation::desktop_pet_active),
                settle_pending_sit.run_if(presentation::desktop_pet_active),
                cloud::align_cloud_seat,
                cloud::animate_cloud_visual,
                pet_feedback::update_particles,
            )
                .chain(),
        );

    if benchmark {
        app.add_plugins(benchmark::FullBenchmarkPlugin {
            d3: !use_gpu_instance_buffer_builder,
        });
    }
    #[cfg(debug_assertions)]
    if feature_smoke {
        feature_smoke::install(&mut app);
    }

    app.run();
}
