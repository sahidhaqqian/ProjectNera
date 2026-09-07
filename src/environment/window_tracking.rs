use bevy::prelude::*;

use crate::platform::{
    self,
    NativeMonitorHandle,
    NativeWindowHandle,
    PhysicalRect,
};

use super::DesktopEnvironment;

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

    /// Window sedang DWM-cloaked, biasanya karena berada di virtual desktop lain.
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
            cloaked: false,

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
    pub fn track_window(
        &mut self,
        handle: NativeWindowHandle,
    ) -> bool {
        if !platform::is_window_alive(handle) {
            return false;
        }

        // Jangan acquire window yang sudah minimized.
        //
        // Window yang SUDAH menjadi target boleh kemudian minimized;
        // itu ditangani oleh tracking system.
        if platform::is_window_minimized(handle)
            || platform::is_window_cloaked(handle)
        {
            return false;
        }

        let Some(rect) =
            platform::window_rect(handle)
        else {
            return false;
        };

        let monitor =
            platform::monitor_info(handle)
                .map(|info| info.handle);

        self.target_window =
            Some(
                TrackedWindow::new(
                    handle,
                    rect,
                    monitor,
                ),
            );

        true
    }
}

// -----------------------------------------------------------------------------
// Persistent Tracking
// -----------------------------------------------------------------------------

pub(super) fn track_target_window(
    mut environment: ResMut<DesktopEnvironment>,
) {
    let Some(target) =
        environment.target_window.as_mut()
    else {
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

    if !platform::is_window_alive(
        target.handle,
    ) {
        target.alive = false;
        target.minimized = false;
        target.cloaked = false;

        info!(
            "Tracked window closed: {:?}",
            target.handle,
        );

        return;
    }

    // -------------------------------------------------------------------------
    // Minimized
    // -------------------------------------------------------------------------

    let was_minimized =
        target.minimized;

    let minimized =
        platform::is_window_minimized(
            target.handle,
        );

    target.minimized = minimized;

    if minimized != was_minimized {
        if minimized {
            info!(
                "Tracked window minimized: {:?}",
                target.handle,
            );
        } else {
            info!(
                "Tracked window restored: {:?}",
                target.handle,
            );
        }
    }

    // -------------------------------------------------------------------------
    // DWM cloaking / Virtual Desktop
    // -------------------------------------------------------------------------

    let was_cloaked =
        target.cloaked;

    let cloaked =
        platform::is_window_cloaked(
            target.handle,
        );

    target.cloaked = cloaked;

    if cloaked != was_cloaked {
        if cloaked {
            info!(
                "Tracked window cloaked (likely another virtual desktop): {:?}",
                target.handle,
            );
        } else {
            info!(
                "Tracked window uncloaked: {:?}",
                target.handle,
            );
        }
    }

    // Jangan update rect ketika minimized.
    //
    // Kita pertahankan rect normal terakhir supaya Nera
    // tidak tiba-tiba bergerak ke koordinat minimized/icon.
    if minimized || cloaked {
        return;
    }

    // -------------------------------------------------------------------------
    // Geometry
    // -------------------------------------------------------------------------

    let Some(new_rect) =
        platform::window_rect(
            target.handle,
        )
    else {
        return;
    };

    let old_rect =
        target.rect;

    target.previous_rect =
        old_rect;

    target.rect =
        new_rect;

    target.moved =
        old_rect.left != new_rect.left
            || old_rect.top != new_rect.top;

    target.resized =
        old_rect.width() != new_rect.width()
            || old_rect.height() != new_rect.height();

    // -------------------------------------------------------------------------
    // Monitor
    // -------------------------------------------------------------------------

    let old_monitor =
        target.monitor;

    let new_monitor =
        platform::monitor_info(
            target.handle,
        )
        .map(|info| info.handle);

    target.previous_monitor =
        old_monitor;

    target.monitor =
        new_monitor;

    target.monitor_changed =
        old_monitor != new_monitor;

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
            target.handle,
            target.moved,
            target.resized,
            target.rect,
        );
    }
}
