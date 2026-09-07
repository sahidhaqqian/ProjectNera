use bevy::prelude::*;

use crate::platform;

use super::DesktopEnvironment;

// -----------------------------------------------------------------------------
// Discovery cadence
// -----------------------------------------------------------------------------

const WINDOW_ENUM_INTERVAL: f32 = 0.2;

// 0.2 s = 5 scans / second.
//
// Kita TIDAK EnumWindows setiap frame.
#[derive(Resource)]
pub(super) struct WindowDiscoveryTimer {
    timer: Timer,
    initialized: bool,
}

impl Default for WindowDiscoveryTimer {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(
                WINDOW_ENUM_INTERVAL,
                TimerMode::Repeating,
            ),
            initialized: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Desktop snapshot
// -----------------------------------------------------------------------------

pub(super) fn refresh_window_snapshot(
    time: Res<Time>,
    mut timer: ResMut<WindowDiscoveryTimer>,
    mut environment: ResMut<DesktopEnvironment>,
) {
    let Some(nera_hwnd) =
        environment.nera_hwnd
    else {
        return;
    };

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

    match platform::enumerate_windows(
        nera_hwnd,
    ) {
        Ok(windows) => {
            debug!(
                "Desktop window snapshot: {} candidates",
                windows.len()
            );

            environment.discovered_windows =
                windows;
        }

        Err(error) => {
            warn!(
                "EnumWindows failed: {error}"
            );
        }
    }
}
