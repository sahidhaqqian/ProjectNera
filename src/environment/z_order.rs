use bevy::prelude::*;

use crate::platform::{
    self,
    NativeWindowHandle,
    PhysicalPoint,
    PhysicalRect,
};

use super::DesktopEnvironment;

const Z_ORDER_INTERVAL: f32 = 0.1;

// -----------------------------------------------------------------------------
// Timer
// -----------------------------------------------------------------------------

#[derive(Resource)]
pub(super) struct ZOrderTimer {
    timer: Timer,
    initialized: bool,
}

impl Default for ZOrderTimer {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(
                Z_ORDER_INTERVAL,
                TimerMode::Repeating,
            ),
            initialized: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Occlusion
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OccludingWindow {
    pub handle: NativeWindowHandle,
    pub rect: PhysicalRect,
}

// -----------------------------------------------------------------------------
// Z-Order State
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZOrderState {
    /// Candidate valid terdekat di atas target.
    pub window_above_target:
        Option<NativeWindowHandle>,

    /// Candidate valid terdekat di bawah target.
    pub window_below_target:
        Option<NativeWindowHandle>,

    /// Semua window di atas target yang overlap
    /// dengan area target.
    pub occluders:
        Vec<OccludingWindow>,

    /// Shortcut:
    /// apakah target setidaknya tertutup sebagian?
    pub target_occluded: bool,
}

impl ZOrderState {
    /// Phase 4 akan memakai ini untuk mengecek:
    ///
    /// "posisi tempat Nera duduk sedang tertutup
    /// window lain atau tidak?"
    #[allow(dead_code)]
    pub fn is_point_occluded(
        &self,
        point: PhysicalPoint,
    ) -> bool {
        self.occluders
            .iter()
            .any(|window| {
                window.rect.contains(point)
            })
    }
}

// -----------------------------------------------------------------------------
// System
// -----------------------------------------------------------------------------

pub(super) fn update_z_order(
    time: Res<Time>,
    mut timer: ResMut<ZOrderTimer>,
    mut environment: ResMut<DesktopEnvironment>,
) {
    let should_refresh =
        if !timer.initialized {
            timer.initialized = true;
            true
        } else {
            timer
                .timer
                .tick(time.delta())
                .just_finished()
        };

    if !should_refresh {
        return;
    }

    let Some(nera_hwnd) =
        environment.nera_hwnd
    else {
        environment.z_order =
            ZOrderState::default();

        return;
    };

    let Some(target) =
        environment.target_window.as_ref()
    else {
        environment.z_order =
            ZOrderState::default();

        return;
    };

    // Jangan hitung occlusion kalau target sudah mati
    // atau sedang minimize.
    if !target.alive
        || target.minimized
    {
        environment.z_order =
            ZOrderState::default();

        return;
    }

    let target_handle =
        target.handle;

    let target_rect =
        target.rect;

    // -------------------------------------------------------------------------
    // Above
    // -------------------------------------------------------------------------

    let above =
        platform::windows_above_target(
            target_handle,
            nera_hwnd,
        );

    let window_above_target =
        above
            .first()
            .map(|window| {
                window.handle
            });

    // -------------------------------------------------------------------------
    // Occlusion
    // -------------------------------------------------------------------------

    let occluders:
        Vec<OccludingWindow> =
        above
            .iter()
            .filter(|window| {
                window
                    .rect
                    .intersects(
                        target_rect,
                    )
            })
            .map(|window| {
                OccludingWindow {
                    handle:
                        window.handle,

                    rect:
                        window.rect,
                }
            })
            .collect();

    // -------------------------------------------------------------------------
    // Below
    // -------------------------------------------------------------------------

    let window_below_target =
        platform::window_below_target(
            target_handle,
            nera_hwnd,
        )
        .map(|window| {
            window.handle
        });

    // -------------------------------------------------------------------------
    // Result
    // -------------------------------------------------------------------------

    let next_state =
        ZOrderState {
            window_above_target,
            window_below_target,

            target_occluded:
                !occluders.is_empty(),

            occluders,
        };

    // Jangan spam log tiap 100 ms.
    if next_state
        != environment.z_order
    {
        debug!(
            "Z-order changed | above={:?} below={:?} occluded={} occluders={}",
            next_state.window_above_target,
            next_state.window_below_target,
            next_state.target_occluded,
            next_state.occluders.len(),
        );

        environment.z_order =
            next_state;
    }
}