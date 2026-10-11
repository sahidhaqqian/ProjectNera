//! Project Nera — persistent application settings foundation.
//!
//! Scope of this first settings slice:
//! - persistent settings resource (Bevy SettingsPlugin)
//! - second Bevy window opened from Nera's radial menu
//! - Always-on-top as one source of truth for Settings + tray
//! - FPS cap via WinitSettings
//! - synchronous settings save before Exit

mod memory;
#[cfg(all(debug_assertions, target_os = "windows"))]
mod memory_audit;
mod model;
mod runtime;
mod shadow;
mod startup;
mod ui;
#[cfg(debug_assertions)]
pub(crate) use shadow::assert_smoke_presentation_shadow_hidden;
#[cfg(debug_assertions)]
pub(crate) use shadow::assert_smoke_shadow_ground;
#[cfg(debug_assertions)]
pub(crate) use ui::smoke_pointer_routing;
#[cfg(debug_assertions)]
pub(crate) use ui::smoke_throw_controls;

pub(crate) use model::{ActivityLevel, AlarmConfig, NeraSettings};
pub(crate) use runtime::{save_settings_before_exit, save_settings_deferred};
pub(crate) use ui::{
    OpenSettingsWindow, ReturnToActionMenu, SettingsInputGate, nera_input_available,
    nera_input_blocked, nera_pointer_input_blocked,
};

use bevy::prelude::*;
use runtime::*;
use ui::*;
// =============================================================================
// Plugin
// =============================================================================

pub(crate) struct NeraSettingsPlugin;

impl Plugin for NeraSettingsPlugin {
    fn build(&self, app: &mut App) {
        #[cfg(all(debug_assertions, target_os = "windows"))]
        if std::env::args().any(|arg| arg == "--settings-memory-audit") {
            memory_audit::install(app);
        }
        // Keep the frame-pacing plugin next to the FPS system that depends on
        // FramepaceSettings. This guarantees the resource exists before
        // apply_fps_limit is initialized/run.
        app.add_plugins(bevy_framepace::FramepacePlugin)
            .add_plugins(shadow::NeraShadowPlugin)
            .init_resource::<SettingsInputGate>()
            .init_resource::<memory::SettingsFontCache>()
            .init_resource::<StartupRuntime>()
            .init_resource::<AppliedRuntimeSettings>()
            .init_resource::<PositionRuntime>()
            .add_message::<OpenSettingsWindow>()
            .add_message::<crate::animation_manager::OpenCharacterManager>()
            .add_message::<ReturnToActionMenu>()
            .add_systems(Startup, initialize_startup_state)
            .add_systems(PreUpdate, sync_settings_minimized)
            .add_systems(
                Update,
                (
                    apply_remembered_position
                        .before(crate::startup_entrance::prepare_startup_entrance),
                    record_primary_window_position,
                    flush_position_save,
                    (memory::begin, open_settings_window).chain(),
                    (handle_settings_buttons, import_character_from_settings).chain(),
                    handle_roaming_radius_slider,
                    handle_throw_intensity_slider,
                    drag_settings_window,
                    scroll_settings_window,
                    close_settings_after_mouse_release,
                    update_settings_labels,
                    (
                        update_character_settings_labels,
                        update_character_import_status,
                        update_gravity_label,
                        update_throw_settings_labels,
                    ),
                    update_audio_settings_labels,
                    (update_roaming_settings_labels, update_living_labels).chain(),
                    apply_always_on_top,
                    apply_character_size.before(crate::startup_entrance::prepare_startup_entrance),
                    apply_audio_settings,
                    apply_fps_limit,
                    cleanup_settings_window,
                    observe_settings_window_removed,
                )
                    .chain(),
            );
    }
}
