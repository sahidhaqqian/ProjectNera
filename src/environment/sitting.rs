use bevy::{
    prelude::*,
    window::{PrimaryWindow, WindowLevel},
};

use crate::platform::{
    self,
    NativeMonitorHandle,
    NativeWindowHandle,
    PhysicalPoint,
    PhysicalRect,
};

use super::{
    DesktopEnvironment,
    TaskbarEdge,
    TaskbarInfo,
};

// -----------------------------------------------------------------------------
// Phase 4 tuning
// -----------------------------------------------------------------------------

/// Maximum distance between the landing anchor and a support edge before snap.
/// This value is expressed at 96 DPI and scaled to Nera's current DPI.
const SNAP_DISTANCE_AT_96_DPI: f64 = 64.0;

/// Keeps the support point slightly away from a window's extreme corners when
/// the target is resized smaller than the original landing position.
const WINDOW_EDGE_MARGIN_AT_96_DPI: f64 = 12.0;

/// Used only while the VRM foot bones are not ready yet.
const FALLBACK_SUPPORT_X_RATIO: f32 = 0.50;
const FALLBACK_SUPPORT_Y_RATIO: f32 = 0.88;

/// Re-assert target-relative z-order at a low cadence. Geometry following still
/// happens immediately whenever the target moves/resizes.
const Z_ORDER_REFRESH_INTERVAL: f32 = 0.10;

// -----------------------------------------------------------------------------
// Placement state
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowAttachment {
    pub target: NativeWindowHandle,

    /// Horizontal position of the support point relative to target.left.
    /// It remains stable while the target moves and is clamped when resized.
    pub support_offset_x: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskbarAttachment {
    pub monitor: NativeMonitorHandle,
    pub edge: TaskbarEdge,

    /// Offset along the taskbar's inner support edge.
    /// X for top/bottom taskbars, Y for left/right taskbars.
    pub along_offset: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlacementState {
    #[default]
    Free,
    Window(WindowAttachment),
    Taskbar(TaskbarAttachment),
}

#[derive(Resource)]
pub(super) struct SittingTimer {
    z_order: Timer,
    z_order_initialized: bool,
}

impl Default for SittingTimer {
    fn default() -> Self {
        Self {
            z_order: Timer::from_seconds(
                Z_ORDER_REFRESH_INTERVAL,
                TimerMode::Repeating,
            ),
            z_order_initialized: false,
        }
    }
}

// -----------------------------------------------------------------------------
// Support anchor
// -----------------------------------------------------------------------------

impl DesktopEnvironment {
    /// Called by the avatar/camera system after projecting Nera's feet into the
    /// viewport. Logical pixels are kept so the anchor survives DPI changes.
    pub fn set_support_anchor(&mut self, local_logical: Vec2) {
        self.support_anchor_logical = Some(local_logical);
    }

    fn support_anchor_for_rect(&self, rect: PhysicalRect) -> PhysicalPoint {
        let width = rect.width().max(1);
        let height = rect.height().max(1);

        if let Some(anchor) = self.support_anchor_logical {
            let scale = self.nera_scale_factor as f32;

            return PhysicalPoint::new(
                ((anchor.x * scale).round() as i32).clamp(0, width),
                ((anchor.y * scale).round() as i32).clamp(0, height),
            );
        }

        PhysicalPoint::new(
            ((width as f32) * FALLBACK_SUPPORT_X_RATIO).round() as i32,
            ((height as f32) * FALLBACK_SUPPORT_Y_RATIO).round() as i32,
        )
    }

    fn taskbar_anchor_for_rect(
        &self,
        rect: PhysicalRect,
        edge: TaskbarEdge,
    ) -> PhysicalPoint {
        let support = self.support_anchor_for_rect(rect);

        match edge {
            // Feet sit on the upper edge of a bottom taskbar.
            TaskbarEdge::Bottom => support,

            // For the other orientations we attach the nearest side of Nera's
            // native window to the taskbar's inner edge. This is the foundation
            // for vertical/top taskbar layouts while bottom remains the natural
            // "sitting" pose.
            TaskbarEdge::Top => PhysicalPoint::new(support.x, 0),
            TaskbarEdge::Left => PhysicalPoint::new(0, support.y),
            TaskbarEdge::Right => PhysicalPoint::new(rect.width(), support.y),
        }
    }

    fn snap_distance(&self) -> i32 {
        (SNAP_DISTANCE_AT_96_DPI * self.nera_scale_factor)
            .round()
            .max(1.0) as i32
    }

    fn window_edge_margin(&self) -> i32 {
        (WINDOW_EDGE_MARGIN_AT_96_DPI * self.nera_scale_factor)
            .round()
            .max(0.0) as i32
    }
}

// -----------------------------------------------------------------------------
// Drag lifecycle API
// -----------------------------------------------------------------------------

impl DesktopEnvironment {
    /// Dragging Nera always breaks the previous physical attachment first.
    pub fn begin_nera_drag(&mut self) {
        if self.placement == PlacementState::Free {
            return;
        }

        info!("Nera detached by drag: {:?}", self.placement);
        self.detach_nera();
    }

    /// Called exactly when the left mouse button is released after a drag.
    /// The selected target comes from Nera's landing/support point, never from
    /// the cursor position.
    pub fn finish_nera_drag(&mut self) -> bool {
        let Some(nera_hwnd) = self.nera_hwnd else {
            return false;
        };

        let Some(nera_rect) = platform::window_rect(nera_hwnd) else {
            return false;
        };

        self.nera_rect = Some(nera_rect);

        // The final drag movement may have crossed onto another monitor after
        // the First-stage sync already ran. Refresh DPI/monitor now so logical
        // foot coordinates and snap tolerances use the landing monitor, not the
        // monitor from the beginning of this frame.
        self.nera_dpi = platform::dpi_for_window(nera_hwnd);
        self.nera_scale_factor = platform::dpi_scale_factor(self.nera_dpi);
        self.monitor = platform::monitor_info(nera_hwnd);

        // Release is rare, so prefer a fresh desktop snapshot instead of a
        // potentially 0.2 s-old discovery sample. This makes the landing
        // decision match windows that were moved while Nera was being dragged.
        if let Ok(windows) = platform::enumerate_windows(nera_hwnd) {
            self.discovered_windows = windows;
        }

        let snap_distance = self.snap_distance();
        let support_anchor = self.support_anchor_for_rect(nera_rect);
        let support_global = PhysicalPoint::new(
            nera_rect.left + support_anchor.x,
            nera_rect.top + support_anchor.y,
        );

        let window_candidate = self
            .discovered_windows
            .iter()
            .filter_map(|window| {
                if platform::is_window_minimized(window.handle) {
                    return None;
                }

                // Support point must land over the target's top edge span.
                if support_global.x < window.rect.left
                    || support_global.x > window.rect.right
                {
                    return None;
                }

                let distance = (support_global.y - window.rect.top).abs();
                if distance > snap_distance {
                    return None;
                }

                // Do not attach to a covered/background window merely because
                // its top edge happens to be numerically closer. Probe one pixel
                // inside the candidate and require it to be the topmost visible
                // desktop window at the landing X coordinate.
                let edge_probe = PhysicalPoint::new(
                    support_global.x,
                    window.rect.top.saturating_add(1),
                );

                let visible_at_edge = platform::window_at_point(
                    edge_probe,
                    nera_hwnd,
                    &self.discovered_windows,
                ) == Some(window.handle);

                visible_at_edge.then_some((distance, window.handle, window.rect))
            })
            .min_by_key(|candidate| candidate.0);

        let taskbar_candidate = self
            .taskbar
            .taskbars
            .iter()
            .filter_map(|taskbar| {
                let anchor = self.taskbar_anchor_for_rect(nera_rect, taskbar.edge);
                let global = PhysicalPoint::new(
                    nera_rect.left + anchor.x,
                    nera_rect.top + anchor.y,
                );

                taskbar_snap_distance(global, *taskbar)
                    .filter(|distance| *distance <= snap_distance)
                    .map(|distance| (distance, *taskbar))
            })
            .min_by_key(|candidate| candidate.0);

        let choose_taskbar = match (window_candidate, taskbar_candidate) {
            (None, None) => {
                self.detach_nera();
                return false;
            }
            (None, Some(_)) => true,
            (Some(_), None) => false,
            (Some(window), Some(taskbar)) => taskbar.0 < window.0,
        };

        if choose_taskbar {
            let Some((_, taskbar)) = taskbar_candidate else {
                self.detach_nera();
                return false;
            };

            return self.attach_to_taskbar(taskbar, nera_rect);
        }

        let Some((_, handle, target_rect)) = window_candidate else {
            self.detach_nera();
            return false;
        };

        self.attach_to_window(handle, target_rect, nera_rect)
    }

    pub fn detach_nera(&mut self) {
        self.placement = PlacementState::Free;
        self.target_window = None;
        self.hidden_for_target_suspend = false;
    }

    fn attach_to_window(
        &mut self,
        handle: NativeWindowHandle,
        target_rect: PhysicalRect,
        nera_rect: PhysicalRect,
    ) -> bool {
        let Some(nera_hwnd) = self.nera_hwnd else {
            return false;
        };

        if !self.track_window(handle) {
            return false;
        }

        // `track_window` samples native geometry again, so prefer that newest
        // rect over the discovery snapshot used during candidate selection.
        let target_rect = self
            .target_window
            .as_ref()
            .map(|target| target.rect)
            .unwrap_or(target_rect);

        let anchor = self.support_anchor_for_rect(nera_rect);
        let support_x = nera_rect.left + anchor.x;
        let requested_offset = support_x - target_rect.left;
        let (desired, support_offset_x) = position_on_window(
            target_rect,
            anchor,
            requested_offset,
            self.window_edge_margin(),
        );

        // Store the already-clamped offset immediately. This avoids one frame
        // where placement state and the physical snapped position disagree.
        self.placement = PlacementState::Window(WindowAttachment {
            target: handle,
            support_offset_x,
        });

        if platform::move_window(nera_hwnd, desired).is_ok() {
            self.nera_rect = platform::window_rect(nera_hwnd);
        }

        info!(
            "Nera snapped to window: {:?} | support_offset_x={}",
            handle,
            support_offset_x,
        );

        true
    }

    fn attach_to_taskbar(
        &mut self,
        taskbar: TaskbarInfo,
        nera_rect: PhysicalRect,
    ) -> bool {
        let anchor = self.taskbar_anchor_for_rect(nera_rect, taskbar.edge);
        let global = PhysicalPoint::new(
            nera_rect.left + anchor.x,
            nera_rect.top + anchor.y,
        );

        let along_offset = match taskbar.edge {
            TaskbarEdge::Top | TaskbarEdge::Bottom => {
                global.x - taskbar.rect.left
            }
            TaskbarEdge::Left | TaskbarEdge::Right => {
                global.y - taskbar.rect.top
            }
        };

        self.target_window = None;
        self.placement = PlacementState::Taskbar(TaskbarAttachment {
            monitor: taskbar.monitor,
            edge: taskbar.edge,
            along_offset,
        });

        let desired = position_on_taskbar(
            taskbar,
            anchor,
            along_offset,
        )
        .0;

        let Some(nera_hwnd) = self.nera_hwnd else {
            self.detach_nera();
            return false;
        };

        if platform::move_window(nera_hwnd, desired).is_ok() {
            self.nera_rect = platform::window_rect(nera_hwnd);
        }

        info!(
            "Nera snapped to taskbar: monitor=0x{:X} edge={:?} along_offset={}",
            taskbar.monitor.as_raw(),
            taskbar.edge,
            along_offset,
        );

        true
    }
}

// -----------------------------------------------------------------------------
// Snap math
// -----------------------------------------------------------------------------

fn taskbar_snap_distance(
    point: PhysicalPoint,
    taskbar: TaskbarInfo,
) -> Option<i32> {
    match taskbar.edge {
        TaskbarEdge::Bottom => {
            (point.x >= taskbar.rect.left && point.x <= taskbar.rect.right)
                .then_some((point.y - taskbar.rect.top).abs())
        }
        TaskbarEdge::Top => {
            (point.x >= taskbar.rect.left && point.x <= taskbar.rect.right)
                .then_some((point.y - taskbar.rect.bottom).abs())
        }
        TaskbarEdge::Left => {
            (point.y >= taskbar.rect.top && point.y <= taskbar.rect.bottom)
                .then_some((point.x - taskbar.rect.right).abs())
        }
        TaskbarEdge::Right => {
            (point.y >= taskbar.rect.top && point.y <= taskbar.rect.bottom)
                .then_some((point.x - taskbar.rect.left).abs())
        }
    }
}

fn clamp_support_on_window(
    target: PhysicalRect,
    requested_x: i32,
    margin: i32,
) -> i32 {
    let min = target.left + margin.min(target.width().max(0) / 2);
    let max = target.right - margin.min(target.width().max(0) / 2);

    requested_x.clamp(min.min(max), max.max(min))
}

fn position_on_window(
    target: PhysicalRect,
    anchor: PhysicalPoint,
    support_offset_x: i32,
    margin: i32,
) -> (PhysicalPoint, i32) {
    let requested_support_x = target.left + support_offset_x;
    let support_x = clamp_support_on_window(target, requested_support_x, margin);

    (
        PhysicalPoint::new(
            support_x - anchor.x,
            target.top - anchor.y,
        ),
        support_x - target.left,
    )
}

fn position_on_taskbar(
    taskbar: TaskbarInfo,
    anchor: PhysicalPoint,
    along_offset: i32,
) -> (PhysicalPoint, i32) {
    match taskbar.edge {
        TaskbarEdge::Bottom | TaskbarEdge::Top => {
            let support_x = (taskbar.rect.left + along_offset)
                .clamp(taskbar.rect.left, taskbar.rect.right);

            let y = match taskbar.edge {
                TaskbarEdge::Bottom => taskbar.rect.top - anchor.y,
                TaskbarEdge::Top => taskbar.rect.bottom - anchor.y,
                _ => unreachable!(),
            };

            (
                PhysicalPoint::new(support_x - anchor.x, y),
                support_x - taskbar.rect.left,
            )
        }
        TaskbarEdge::Left | TaskbarEdge::Right => {
            let support_y = (taskbar.rect.top + along_offset)
                .clamp(taskbar.rect.top, taskbar.rect.bottom);

            let x = match taskbar.edge {
                TaskbarEdge::Left => taskbar.rect.right - anchor.x,
                TaskbarEdge::Right => taskbar.rect.left - anchor.x,
                _ => unreachable!(),
            };

            // Preserve the current local Y anchor; only the taskbar-normal axis
            // is forced. This keeps the behavior symmetrical with horizontal
            // taskbars and provides the multi-edge foundation.
            (
                PhysicalPoint::new(x, support_y - anchor.y),
                support_y - taskbar.rect.top,
            )
        }
    }
}

// -----------------------------------------------------------------------------
// Follow + lifecycle + z-order
// -----------------------------------------------------------------------------

pub(super) fn follow_sitting_attachment(
    time: Res<Time>,
    mut timer: ResMut<SittingTimer>,
    mut environment: ResMut<DesktopEnvironment>,
    mut window_query: Query<&mut Window, With<PrimaryWindow>>,
) {
    let refresh_z_order = if !timer.z_order_initialized {
        timer.z_order_initialized = true;
        true
    } else {
        timer.z_order.tick(time.delta()).just_finished()
    };

    let placement = environment.placement;

    match placement {
        PlacementState::Free => {
            return;
        }

        PlacementState::Window(attachment) => {
            follow_window_attachment(
                &mut environment,
                &mut window_query,
                attachment,
                refresh_z_order,
            );
        }

        PlacementState::Taskbar(attachment) => {
            follow_taskbar_attachment(
                &mut environment,
                attachment,
            );
        }
    }
}

fn follow_window_attachment(
    environment: &mut DesktopEnvironment,
    window_query: &mut Query<&mut Window, With<PrimaryWindow>>,
    attachment: WindowAttachment,
    refresh_z_order: bool,
) {
    let Some(target) = environment.target_window.clone() else {
        environment.detach_nera();
        return;
    };

    if target.handle != attachment.target || !target.alive {
        let should_restore_visibility = environment.hidden_for_target_suspend;
        environment.detach_nera();

        if should_restore_visibility {
            if let Ok(mut window) = window_query.single_mut() {
                window.visible = true;
            }
        }

        info!("Nera detached: target window closed or changed");
        return;
    }

    // Minimize or DWM cloaking suspends the attachment without destroying it.
    // This covers normal minimize/restore plus switching virtual desktops, where
    // the target HWND remains alive but is temporarily hidden by DWM.
    if target.minimized || target.cloaked {
        if !environment.hidden_for_target_suspend {
            if let Ok(mut window) = window_query.single_mut() {
                if window.visible {
                    window.visible = false;
                    environment.hidden_for_target_suspend = true;
                    info!(
                        "Nera suspended because target is unavailable: minimized={} cloaked={}",
                        target.minimized,
                        target.cloaked,
                    );
                }
            }
        }

        return;
    }

    if environment.hidden_for_target_suspend {
        if let Ok(mut window) = window_query.single_mut() {
            window.visible = true;
        }

        environment.hidden_for_target_suspend = false;
        info!("Nera resumed after target became visible again");
    }

    let Some(nera_hwnd) = environment.nera_hwnd else {
        return;
    };

    let Some(nera_rect) = environment.nera_rect else {
        return;
    };

    let anchor = environment.support_anchor_for_rect(nera_rect);
    let (desired, clamped_offset) = position_on_window(
        target.rect,
        anchor,
        attachment.support_offset_x,
        environment.window_edge_margin(),
    );

    if desired.x != nera_rect.left || desired.y != nera_rect.top {
        if platform::move_window(nera_hwnd, desired).is_ok() {
            environment.nera_rect = platform::window_rect(nera_hwnd);
        }
    }

    if clamped_offset != attachment.support_offset_x {
        environment.placement = PlacementState::Window(WindowAttachment {
            support_offset_x: clamped_offset,
            ..attachment
        });
    }

    if refresh_z_order {
        let always_on_top = window_query
            .single()
            .map(|window| matches!(window.window_level, WindowLevel::AlwaysOnTop))
            .unwrap_or(true);

        if !always_on_top {
            if let Err(error) = platform::place_window_directly_above(
                nera_hwnd,
                attachment.target,
            ) {
                debug!("Failed to refresh target-relative z-order: {error}");
            }
        }
    }
}

fn follow_taskbar_attachment(
    environment: &mut DesktopEnvironment,
    attachment: TaskbarAttachment,
) {
    let Some(taskbar) = environment
        .taskbar
        .for_monitor(attachment.monitor)
        .copied()
        .filter(|taskbar| taskbar.edge == attachment.edge)
    else {
        // Taskbar disappeared (auto-hide/layout changed). Do not keep a stale
        // attachment to old geometry.
        environment.detach_nera();
        info!("Nera detached: attached taskbar is no longer available");
        return;
    };

    let Some(nera_hwnd) = environment.nera_hwnd else {
        return;
    };

    let Some(nera_rect) = environment.nera_rect else {
        return;
    };

    let anchor = environment.taskbar_anchor_for_rect(nera_rect, taskbar.edge);
    let (desired, clamped_offset) = position_on_taskbar(
        taskbar,
        anchor,
        attachment.along_offset,
    );

    if desired.x != nera_rect.left || desired.y != nera_rect.top {
        if platform::move_window(nera_hwnd, desired).is_ok() {
            environment.nera_rect = platform::window_rect(nera_hwnd);
        }
    }

    if clamped_offset != attachment.along_offset {
        environment.placement = PlacementState::Taskbar(TaskbarAttachment {
            along_offset: clamped_offset,
            ..attachment
        });
    }
}
