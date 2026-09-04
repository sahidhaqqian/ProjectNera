mod window_discovery;

use bevy::{
    prelude::*,
    window::PrimaryWindow,
};

use crate::platform::{
    self,
    DesktopWindow,
    MonitorInfo,
    NativeWindowHandle,
    PhysicalRect,
};

// -----------------------------------------------------------------------------
// Desktop Environment
// -----------------------------------------------------------------------------

#[derive(Resource, Debug)]
pub struct DesktopEnvironment {
    // -------------------------------------------------------------------------
    // Nera
    // -------------------------------------------------------------------------

    pub nera_hwnd: Option<NativeWindowHandle>,
    pub nera_rect: Option<PhysicalRect>,
    pub nera_dpi: u32,
    pub nera_scale_factor: f64,

    // -------------------------------------------------------------------------
    // Monitor
    // -------------------------------------------------------------------------

    pub monitor: Option<MonitorInfo>,

    // -------------------------------------------------------------------------
    // Desktop Windows
    // -------------------------------------------------------------------------

    /// Snapshot candidate windows hasil EnumWindows.
    pub discovered_windows: Vec<DesktopWindow>,

    /// Window valid yang saat ini berada di bawah cursor.
    pub window_under_cursor: Option<NativeWindowHandle>,
}

impl Default for DesktopEnvironment {
    fn default() -> Self {
        Self {
            nera_hwnd: None,
            nera_rect: None,
            nera_dpi: platform::BASE_DPI,
            nera_scale_factor: 1.0,
            monitor: None,

            discovered_windows: Vec::new(),
            window_under_cursor: None,
        }
    }
}

// -----------------------------------------------------------------------------
// Plugin
// -----------------------------------------------------------------------------

pub struct DesktopEnvironmentPlugin;

impl Plugin for DesktopEnvironmentPlugin {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<DesktopEnvironment>()
            .init_resource::<
                window_discovery::WindowDiscoveryTimer
            >()
            .add_systems(
                First,
                sync_desktop_environment,
            )
            .add_systems(
                PreUpdate,
                (
                    window_discovery::refresh_window_snapshot,
                    window_discovery::update_window_under_cursor,
                )
                    .chain(),
            );
    }
}

// -----------------------------------------------------------------------------
// Synchronization
// -----------------------------------------------------------------------------

fn sync_desktop_environment(world: &mut World) {
    let primary_window_entity = {
        let mut query =
            world.query_filtered::<Entity, With<PrimaryWindow>>();

        query.iter(world).next()
    };

    let Some(primary_window_entity) = primary_window_entity else {
        return;
    };

    let current_handle = world
        .resource::<DesktopEnvironment>()
        .nera_hwnd;

    let handle = if let Some(handle) = current_handle {
        handle
    } else {
        let Some(handle) =
            platform::native_window_handle(primary_window_entity)
        else {
            return;
        };

        info!(
            "Nera native HWND acquired: 0x{:X}",
            handle.as_raw() as usize
        );

        handle
    };

    let rect = platform::window_rect(handle);
    let dpi = platform::dpi_for_window(handle);
    let monitor = platform::monitor_info(handle);
    let scale_factor = platform::dpi_scale_factor(dpi);

    let mut environment =
        world.resource_mut::<DesktopEnvironment>();

    environment.nera_hwnd = Some(handle);
    environment.nera_rect = rect;
    environment.nera_dpi = dpi;
    environment.nera_scale_factor = scale_factor;
    environment.monitor = monitor;
}