mod window_discovery;
mod window_tracking;
mod z_order;

pub use window_tracking::TrackedWindow;

pub use z_order::ZOrderState;


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
    // Window Discovery
    // -------------------------------------------------------------------------

    pub discovered_windows: Vec<DesktopWindow>,
    pub window_under_cursor: Option<NativeWindowHandle>,

    // -------------------------------------------------------------------------
    // Window Tracking
    // -------------------------------------------------------------------------

    pub target_window: Option<TrackedWindow>,

    // Z order
    pub z_order: ZOrderState,
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
            target_window: None,
            z_order: ZOrderState::default(),
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
            .init_resource::<window_discovery::WindowDiscoveryTimer>()
            .init_resource::<z_order::ZOrderTimer>()
            .add_systems(First,sync_desktop_environment,)
            .add_systems(
                PreUpdate,
                (
                    window_discovery::refresh_window_snapshot,
                    window_discovery::update_window_under_cursor,

                    window_tracking::acquire_target_from_cursor,
                    window_tracking::track_target_window,
                    z_order::update_z_order,
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