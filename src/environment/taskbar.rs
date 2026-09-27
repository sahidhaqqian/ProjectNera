use bevy::prelude::*;

use crate::platform::{
    self,
    MonitorInfo,
    NativeMonitorHandle,
    PhysicalRect,
};

use super::DesktopEnvironment;

const TASKBAR_REFRESH_INTERVAL: f32 =
    1.0;

// -----------------------------------------------------------------------------
// Taskbar Edge
// -----------------------------------------------------------------------------

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
)]
pub enum TaskbarEdge {
    Left,
    Top,
    Right,
    Bottom,
}

// -----------------------------------------------------------------------------
// Taskbar Info
// -----------------------------------------------------------------------------

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
)]
pub struct TaskbarInfo {
    pub monitor:
        NativeMonitorHandle,

    pub rect:
        PhysicalRect,

    pub edge:
        TaskbarEdge,

    pub is_primary:
        bool,
}

// -----------------------------------------------------------------------------
// State
// -----------------------------------------------------------------------------

#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
)]
pub struct TaskbarState {
    pub taskbars:
        Vec<TaskbarInfo>,

    pub primary:
        Option<TaskbarInfo>,
}

impl TaskbarState {
    pub fn for_monitor(
        &self,
        monitor: NativeMonitorHandle,
    ) -> Option<&TaskbarInfo> {
        self.taskbars
            .iter()
            .find(|taskbar| {
                taskbar.monitor
                    == monitor
            })
    }
}

// -----------------------------------------------------------------------------
// Timer
// -----------------------------------------------------------------------------

#[derive(Resource)]
pub(super) struct TaskbarTimer {
    timer: Timer,
    initialized: bool,
}

impl Default for TaskbarTimer {
    fn default() -> Self {
        Self {
            timer:
                Timer::from_seconds(
                    TASKBAR_REFRESH_INTERVAL,
                    TimerMode::Repeating,
                ),

            initialized: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Detection
// -----------------------------------------------------------------------------

fn taskbar_from_monitor(
    monitor: MonitorInfo,
) -> Option<TaskbarInfo> {
    let bounds =
        monitor.bounds;

    let work =
        monitor.work_area;

    // Berapa pixel area yang "hilang"
    // dari work area pada masing-masing sisi.
    let left =
        (work.left - bounds.left)
            .max(0);

    let top =
        (work.top - bounds.top)
            .max(0);

    let right =
        (bounds.right - work.right)
            .max(0);

    let bottom =
        (bounds.bottom - work.bottom)
            .max(0);

    // Cari reserved edge terbesar.
    let mut thickness = left;
    let mut edge =
        TaskbarEdge::Left;

    if top > thickness {
        thickness = top;
        edge = TaskbarEdge::Top;
    }

    if right > thickness {
        thickness = right;
        edge = TaskbarEdge::Right;
    }

    if bottom > thickness {
        thickness = bottom;
        edge = TaskbarEdge::Bottom;
    }

    // Tidak ada reserved desktop edge.
    //
    // Bisa berarti taskbar auto-hide,
    // atau monitor memang tidak punya
    // taskbar.
    if thickness <= 0 {
        return None;
    }

    let rect =
        match edge {
            TaskbarEdge::Left => {
                PhysicalRect {
                    left:
                        bounds.left,

                    top:
                        bounds.top,

                    right:
                        work.left,

                    bottom:
                        bounds.bottom,
                }
            }

            TaskbarEdge::Top => {
                PhysicalRect {
                    left:
                        bounds.left,

                    top:
                        bounds.top,

                    right:
                        bounds.right,

                    bottom:
                        work.top,
                }
            }

            TaskbarEdge::Right => {
                PhysicalRect {
                    left:
                        work.right,

                    top:
                        bounds.top,

                    right:
                        bounds.right,

                    bottom:
                        bounds.bottom,
                }
            }

            TaskbarEdge::Bottom => {
                PhysicalRect {
                    left:
                        bounds.left,

                    top:
                        work.bottom,

                    right:
                        bounds.right,

                    bottom:
                        bounds.bottom,
                }
            }
        };

    Some(TaskbarInfo {
        monitor:
            monitor.handle,

        rect,

        edge,

        is_primary:
            monitor.is_primary,
    })
}

// -----------------------------------------------------------------------------
// System
// -----------------------------------------------------------------------------

pub(super) fn refresh_taskbars(
    time: Res<Time>,
    mut timer:
        ResMut<TaskbarTimer>,
    mut environment:
        ResMut<DesktopEnvironment>,
) {
    let should_refresh =
        if !timer.initialized {
            timer.initialized = true;
            true
        } else {
            timer.timer
                .tick(time.delta())
                .just_finished()
        };

    if !should_refresh {
        return;
    }

    let monitors =
        platform::enumerate_monitors();

    let mut taskbars:
        Vec<TaskbarInfo> =
        monitors
            .into_iter()
            .filter_map(
                taskbar_from_monitor,
            )
            .collect();

    // Primary dulu supaya output/state
    // konsisten.
    taskbars.sort_by_key(
        |taskbar| {
            !taskbar.is_primary
        },
    );

    let primary =
        taskbars
            .iter()
            .find(|taskbar| {
                taskbar.is_primary
            })
            .copied();

    let next_state =
        TaskbarState {
            taskbars,
            primary,
        };

    if next_state
        == environment.taskbar
    {
        return;
    }

    environment.taskbar =
        next_state;

    for taskbar in
        &environment
            .taskbar
            .taskbars
    {
        info!(
            "Taskbar detected | monitor=0x{:X} primary={} edge={:?} rect={:?}",
            taskbar.monitor.as_raw(),
            taskbar.is_primary,
            taskbar.edge,
            taskbar.rect,
        );
    }
}
