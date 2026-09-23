//! Project Nera — persistent application settings foundation.
//!
//! Scope of this first settings slice:
//! - persistent settings resource (Bevy SettingsPlugin)
//! - second Bevy window opened from Nera's radial menu
//! - Always-on-top as one source of truth for Settings + tray
//! - FPS cap via WinitSettings
//! - synchronous settings save before Exit

mod model;
mod runtime;
mod shadow;
mod startup;
mod ui;

pub(crate) use model::{AlarmConfig, NeraSettings};
pub(crate) use runtime::{save_settings_before_exit, save_settings_deferred};
pub(crate) use ui::{
    OpenSettingsWindow, ReturnToActionMenu, SettingsInputGate, nera_input_available,
    nera_input_blocked,
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
        app.add_plugins(shadow::NeraShadowPlugin)
            .init_resource::<SettingsInputGate>()
            .init_resource::<StartupRuntime>()
            .init_resource::<PositionRuntime>()
            .add_message::<OpenSettingsWindow>()
            .add_message::<ReturnToActionMenu>()
            .add_systems(Startup, initialize_startup_state)
            .add_systems(
                Update,
                (
                    apply_remembered_position,
                    record_primary_window_position,
                    flush_position_save,
                    open_settings_window,
                    handle_settings_buttons,
                    drag_settings_window,
                    scroll_settings_window,
                    close_settings_after_mouse_release,
                    update_settings_labels,
                    update_character_settings_labels,
                    update_audio_settings_labels,
                    apply_always_on_top,
                    apply_character_size,
                    apply_audio_settings,
                    apply_fps_limit,
                    cleanup_settings_window,
                    observe_settings_window_removed,
                )
                    .chain(),
            );
    }
}
