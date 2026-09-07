mod window_discovery;
mod window_tracking;
mod taskbar;
mod sitting;

pub use window_tracking::TrackedWindow;

pub use taskbar::{
    TaskbarEdge,
    TaskbarInfo,
    TaskbarState,
};

pub use sitting::{AvatarAnchor, PlacementState};

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

    /// Standing support point projected from leftFoot/rightFoot.
    pub feet_anchor_logical: Option<Vec2>,

    /// Sitting support point projected from the hips + tuned seat offset.
    pub seat_anchor_logical: Option<Vec2>,

    /// Which anchor an active attachment should use.
    /// Keep `Feet` until the sitting animation is wired; the animation state
    /// can switch this to `Seat` without changing snap/follow math.
    pub attached_anchor: AvatarAnchor,

    // -------------------------------------------------------------------------
    // Monitor
    // -------------------------------------------------------------------------

    pub monitor: Option<MonitorInfo>,

    // -------------------------------------------------------------------------
    // Window Discovery
    // -------------------------------------------------------------------------

    pub discovered_windows: Vec<DesktopWindow>,

    // -------------------------------------------------------------------------
    // Window Tracking
    // -------------------------------------------------------------------------

    pub target_window: Option<TrackedWindow>,

    // -------------------------------------------------------------------------
    // Taskbar
    // -------------------------------------------------------------------------

    pub taskbar: TaskbarState,

    // -------------------------------------------------------------------------
    // Phase 4 - Sitting / attachment
    // -------------------------------------------------------------------------

    pub placement: PlacementState,

    /// True while Phase 4 hides Nera because the attached target is temporarily
    /// unavailable (minimized or DWM-cloaked on another virtual desktop).
    pub hidden_for_target_suspend: bool,
}

impl Default for DesktopEnvironment {
    fn default() -> Self {
        Self {
            nera_hwnd: None,
            nera_rect: None,
            nera_dpi: platform::BASE_DPI,
            nera_scale_factor: 1.0,
            feet_anchor_logical: None,
            seat_anchor_logical: None,
            attached_anchor: AvatarAnchor::Feet,
            monitor: None,

            discovered_windows: Vec::new(),
            target_window: None,
            taskbar: TaskbarState::default(),
            placement: PlacementState::default(),
            hidden_for_target_suspend: false,
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
            .init_resource::<sitting::SittingTimer>()
            .add_systems(First, sync_desktop_environment)
            .init_resource::<taskbar::TaskbarTimer>()
            .add_systems(
                PreUpdate,
                (
                    window_discovery::refresh_window_snapshot,
                    window_tracking::track_target_window,
                    taskbar::refresh_taskbars,
                    sitting::follow_sitting_attachment,
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
