//! Desktop window snapshot, target tracking, and support surfaces.
mod sitting;
mod taskbar;

pub use taskbar::{TaskbarEdge, TaskbarInfo, TaskbarState};

pub use sitting::{AvatarAnchor, PlacementState};
#[cfg(test)]
pub(crate) use sitting::{TaskbarAttachment, WindowAttachment};

use bevy::{prelude::*, window::PrimaryWindow};

use crate::platform::{
    self, DesktopWindow, MonitorInfo, NativeMonitorHandle, NativeWindowHandle, PhysicalRect,
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
        app.init_resource::<DesktopEnvironment>()
            .init_resource::<WindowDiscoveryTimer>()
            .init_resource::<sitting::SittingTimer>()
            .add_systems(First, sync_desktop_environment)
            .init_resource::<taskbar::TaskbarTimer>()
            .add_systems(
                PreUpdate,
                (
                    refresh_window_snapshot,
                    track_target_window,
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
        let mut query = world.query_filtered::<Entity, With<PrimaryWindow>>();

        query.iter(world).next()
    };

    let Some(primary_window_entity) = primary_window_entity else {
        return;
    };

    let current_handle = world.resource::<DesktopEnvironment>().nera_hwnd;

    let handle = if let Some(handle) = current_handle {
        handle
    } else {
        let Some(handle) = platform::native_window_handle(primary_window_entity) else {
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

    let mut environment = world.resource_mut::<DesktopEnvironment>();

    environment.nera_hwnd = Some(handle);
    environment.nera_rect = rect;
    environment.nera_dpi = dpi;
    environment.nera_scale_factor = scale_factor;
    environment.monitor = monitor;
}

// -----------------------------------------------------------------------------
// Discovery cadence
// -----------------------------------------------------------------------------

const WINDOW_ENUM_INTERVAL: f32 = 0.2;

// 0.2 s = 5 scans / second.
//
// Kita TIDAK EnumWindows setiap frame.
#[derive(Resource)]
struct WindowDiscoveryTimer {
    timer: Timer,
    initialized: bool,
}

impl Default for WindowDiscoveryTimer {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(WINDOW_ENUM_INTERVAL, TimerMode::Repeating),
            initialized: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Desktop snapshot
// -----------------------------------------------------------------------------

fn refresh_window_snapshot(
    time: Res<Time>,
    mut timer: ResMut<WindowDiscoveryTimer>,
    mut environment: ResMut<DesktopEnvironment>,
) {
    let Some(nera_hwnd) = environment.nera_hwnd else {
        return;
    };

    let should_refresh = if !timer.initialized {
        timer.initialized = true;
        true
    } else {
        timer.timer.tick(time.delta()).just_finished()
    };

    if !should_refresh {
        return;
    }

    match platform::enumerate_windows(nera_hwnd) {
        Ok(windows) => {
            debug!("Desktop window snapshot: {} candidates", windows.len());

            environment.discovered_windows = windows;
        }

        Err(error) => {
            warn!("EnumWindows failed: {error}");
        }
    }
}

// -----------------------------------------------------------------------------
// Tracked Window
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct TrackedWindow {
    /// Persistent HWND target.
    pub handle: NativeWindowHandle,

    /// Rect terakhir yang valid ketika window tidak minimized.
    pub rect: PhysicalRect,

    /// Rect frame/update sebelumnya.
    pub previous_rect: PhysicalRect,

    /// HWND masih valid.
    pub alive: bool,

    /// Window sedang minimized.
    pub minimized: bool,

    /// DWM currently hides the HWND (for example another virtual desktop).
    pub cloaked: bool,

    /// Posisi berubah pada update ini.
    pub moved: bool,

    /// Ukuran berubah pada update ini.
    pub resized: bool,

    /// Monitor target saat ini.
    pub monitor: Option<NativeMonitorHandle>,

    /// Monitor sebelum update terakhir.
    pub previous_monitor: Option<NativeMonitorHandle>,

    /// Target berpindah monitor pada update ini.
    pub monitor_changed: bool,
}

impl TrackedWindow {
    fn new(
        handle: NativeWindowHandle,
        rect: PhysicalRect,
        monitor: Option<NativeMonitorHandle>,
    ) -> Self {
        Self {
            handle,

            rect,
            previous_rect: rect,

            alive: true,
            minimized: false,
            cloaked: platform::is_window_cloaked(handle),

            moved: false,
            resized: false,

            monitor,
            previous_monitor: monitor,
            monitor_changed: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Target Selection API
// -----------------------------------------------------------------------------

impl DesktopEnvironment {
    /// Mulai track HWND tertentu.
    ///
    /// Phase 4 nanti bisa memakai method yang sama ketika Nera
    /// landing / snap ke sebuah window.
    pub fn track_window(&mut self, handle: NativeWindowHandle) -> bool {
        if !platform::is_window_alive(handle) {
            return false;
        }

        // Jangan acquire window yang sudah minimized.
        //
        // Window yang SUDAH menjadi target boleh kemudian minimized;
        // itu ditangani oleh tracking system.
        if platform::is_window_minimized(handle) {
            return false;
        }

        let Some(rect) = platform::window_rect(handle) else {
            return false;
        };

        let monitor = platform::monitor_info(handle).map(|info| info.handle);

        self.target_window = Some(TrackedWindow::new(handle, rect, monitor));

        true
    }
}

// -----------------------------------------------------------------------------
// Persistent Tracking
// -----------------------------------------------------------------------------

fn track_target_window(mut environment: ResMut<DesktopEnvironment>) {
    let Some(target) = environment.target_window.as_mut() else {
        return;
    };

    // Flags ini hanya berlaku untuk update sekarang.
    target.moved = false;
    target.resized = false;
    target.monitor_changed = false;

    // Begitu HWND sudah dinyatakan dead,
    // jangan mencoba "menghidupkan" handle lama lagi.
    //
    // HWND Windows dapat direcycle oleh OS.
    if !target.alive {
        return;
    }

    // -------------------------------------------------------------------------
    // Alive
    // -------------------------------------------------------------------------

    if !platform::is_window_alive(target.handle) {
        target.alive = false;
        target.minimized = false;
        target.cloaked = false;

        info!("Tracked window closed: {:?}", target.handle,);

        return;
    }

    // -------------------------------------------------------------------------
    // Minimized
    // -------------------------------------------------------------------------

    let was_minimized = target.minimized;

    let minimized = platform::is_window_minimized(target.handle);

    target.minimized = minimized;

    if minimized != was_minimized {
        if minimized {
            info!("Tracked window minimized: {:?}", target.handle,);
        } else {
            info!("Tracked window restored: {:?}", target.handle,);
        }
    }

    let was_cloaked = target.cloaked;
    target.cloaked = platform::is_window_cloaked(target.handle);

    if target.cloaked != was_cloaked {
        debug!(
            "Tracked window DWM cloak changed: {:?} -> {}",
            target.handle, target.cloaked,
        );
    }

    // Keep the last normal rect while Windows has no meaningful visible
    // geometry. Minimize is handled as support loss by sitting.rs; DWM cloak
    // remains a temporary virtual-desktop suspension.
    if minimized || target.cloaked {
        return;
    }

    // -------------------------------------------------------------------------
    // Geometry
    // -------------------------------------------------------------------------

    let Some(new_rect) = platform::window_rect(target.handle) else {
        return;
    };

    let old_rect = target.rect;

    target.previous_rect = old_rect;

    target.rect = new_rect;

    target.moved = old_rect.left != new_rect.left || old_rect.top != new_rect.top;

    target.resized = old_rect.width() != new_rect.width() || old_rect.height() != new_rect.height();

    // -------------------------------------------------------------------------
    // Monitor
    // -------------------------------------------------------------------------

    let old_monitor = target.monitor;

    let new_monitor = platform::monitor_info(target.handle).map(|info| info.handle);

    target.previous_monitor = old_monitor;

    target.monitor = new_monitor;

    target.monitor_changed = old_monitor != new_monitor;

    // -------------------------------------------------------------------------
    // Diagnostics
    // -------------------------------------------------------------------------

    if target.monitor_changed {
        info!(
            "Tracked window changed monitor: {:?} -> {:?}",
            old_monitor.map(|monitor| monitor.as_raw()),
            new_monitor.map(|monitor| monitor.as_raw()),
        );
    }

    if target.moved || target.resized {
        debug!(
            "Tracked window changed: {:?} | moved={} resized={} | rect={:?}",
            target.handle, target.moved, target.resized, target.rect,
        );
    }
}
