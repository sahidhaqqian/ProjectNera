mod alarm;
mod animation;
mod avatar;

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
mod platform;
mod presentation;
mod rng;
mod settings;

#[cfg(target_os = "windows")]
mod tray;

use animation::{
    NeraAnimationPlugin, apply_animation_state, schedule_idle_variation,
    sync_environment_animation_state, sync_interaction_animation_actions, tick_animation_actions,
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
    prelude::*,
    render::{
        RenderPlugin,
        settings::{MemoryHints, WgpuSettings},
    },
    settings::SettingsPlugin,
    window::{CursorOptions, PresentMode, WindowLevel, WindowPlugin, WindowPosition},
};

use bevy_vrm1::prelude::*;

use presentation::PresentationPlugin;
use settings::{NeraSettings, NeraSettingsPlugin};

#[cfg(target_os = "windows")]
use action_menu::ActionMenuPlugin;
#[cfg(target_os = "windows")]
use animation_manager::AnimationManagerPlugin;

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
    let mut app = App::new();

    // Load persistent Nera settings before the native window is created.
    //
    // Bevy SettingsPlugin performs its load immediately when the plugin is
    // added, so `NeraSettings` is already available here. This lets the HWND
    // be born in the correct visibility state instead of relying on a later
    // hidden -> visible transition that can race native window creation.
    app.add_plugins(SettingsPlugin::new("io.github.sahidhaqqi.projectnera"));

    let launch_visible = !app.world().resource::<NeraSettings>().start_hidden;

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
                render_creation: WgpuSettings {
                    memory_hints: MemoryHints::MemoryUsage,
                    ..default()
                }
                .into(),
                ..default()
            }),
    );

    app.add_plugins((
        VrmPlugin,
        VrmaPlugin,
        DesktopEnvironmentPlugin,
        InteractionPlugin,
        NeraAnimationPlugin,
        NeraExpressionPlugin,
        PresentationPlugin,
        alarm::AlarmPlugin,
        NeraSettingsPlugin,
    ));

    #[cfg(target_os = "windows")]
    app.add_plugins((
        TrayPlugin,
        ActionMenuPlugin,
        AnimationManagerPlugin,
        AlarmDismissPlugin,
        AlarmSettingsPlugin,
    ));

    app.insert_resource(ClearColor(Color::NONE))
        .init_resource::<DesktopPetInteraction>()
        .init_resource::<HoveredBodyPart>()
        .init_resource::<HeadPatGesture>()
        .init_resource::<desktop_physics::DesktopPhysicsState>()
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
                    .run_if(nera_utility_input_available),
                desktop_physics::update_desktop_physics.run_if(presentation::desktop_pet_active),
                sync_environment_animation_state,
                sync_interaction_animation_actions,
                // Phase 5.8 idle variation. Runs before the tick so a fidget
                // requested this frame ages like every other one-shot, and
                // before `apply_animation_state` so it plays without a frame of
                // latency. It is the lowest priority layer, so it can never
                // displace something the systems above just requested.
                schedule_idle_variation.run_if(presentation::desktop_pet_active),
                tick_animation_actions,
                sync_look_at_for_animation,
                apply_animation_state,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            update_click_through
                .after(VrmSystemSets::SpringBone)
                .run_if(presentation::desktop_pet_active)
                .run_if(nera_utility_input_available),
        )
        .add_systems(
            PostUpdate,
            detect_head_pat
                .after(update_click_through)
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
        )
        .run();
}
