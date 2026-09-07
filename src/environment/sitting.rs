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

/// Hip/seat probe radius used while dragging. Mate Engine X3 uses a compact
/// probe around the hips rather than waiting for the feet to reach the edge.
const SIT_PROBE_RADIUS_AT_96_DPI: f64 = 56.0;

/// Keeps the support point slightly away from a window's extreme corners when
/// the target is resized smaller than the original landing position.
const WINDOW_EDGE_MARGIN_AT_96_DPI: f64 = 12.0;

/// Used only while the requested VRM anchor is not ready yet.
const FALLBACK_SUPPORT_X_RATIO: f32 = 0.50;
const FALLBACK_FEET_Y_RATIO: f32 = 0.88;
const FALLBACK_SEAT_Y_RATIO: f32 = 0.55;

/// Re-assert target-relative z-order at a low cadence. Geometry following still
/// happens immediately whenever the target moves/resizes.
const Z_ORDER_REFRESH_INTERVAL: f32 = 0.10;

// -----------------------------------------------------------------------------
// Avatar support anchors
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AvatarAnchor {
    #[default]
    Feet,
    Seat,
}

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

    /// Candidate accepted by the hips probe. Sitting animation is activated
    /// first; native placement is intentionally delayed until Seat is sampled.
    PendingWindow {
        target: NativeWindowHandle,
        settle_frames: u8,
    },
    PendingTaskbar {
        monitor: NativeMonitorHandle,
        edge: TaskbarEdge,
        settle_frames: u8,
    },

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
    /// Called after projecting leftFoot/rightFoot into Nera's viewport.
    /// Logical pixels are stored so the anchor survives DPI changes.
    pub fn set_feet_anchor(&mut self, local_logical: Vec2) {
        self.feet_anchor_logical = Some(local_logical);
    }

    /// Called after projecting the hips-derived seat contact point.
    pub fn set_seat_anchor(&mut self, local_logical: Vec2) {
        self.seat_anchor_logical = Some(local_logical);
    }

    /// Animation/pose code owns this switch. Phase 4 geometry only consumes it.
    /// Use `Feet` for idle/standing/drag and `Seat` while a sitting pose is active.
    pub fn set_attached_anchor(&mut self, anchor: AvatarAnchor) {
        if self.attached_anchor != anchor {
            self.attached_anchor = anchor;
            info!("Nera attachment anchor changed to {:?}", anchor);
        }
    }

    fn anchor_for_rect(
        &self,
        rect: PhysicalRect,
        mode: AvatarAnchor,
    ) -> PhysicalPoint {
        let width = rect.width().max(1);
        let height = rect.height().max(1);

        let logical_anchor = match mode {
            AvatarAnchor::Feet => self.feet_anchor_logical,
            AvatarAnchor::Seat => self.seat_anchor_logical,
        };

        if let Some(anchor) = logical_anchor {
            let scale = self.nera_scale_factor as f32;

            return PhysicalPoint::new(
                ((anchor.x * scale).round() as i32).clamp(0, width),
                ((anchor.y * scale).round() as i32).clamp(0, height),
            );
        }

        let fallback_y = match mode {
            AvatarAnchor::Feet => FALLBACK_FEET_Y_RATIO,
            AvatarAnchor::Seat => FALLBACK_SEAT_Y_RATIO,
        };

        PhysicalPoint::new(
            ((width as f32) * FALLBACK_SUPPORT_X_RATIO).round() as i32,
            ((height as f32) * fallback_y).round() as i32,
        )
    }

    fn attachment_anchor_for_rect(&self, rect: PhysicalRect) -> PhysicalPoint {
        self.anchor_for_rect(rect, self.attached_anchor)
    }

    fn taskbar_anchor_for_rect(
        &self,
        rect: PhysicalRect,
        edge: TaskbarEdge,
        mode: AvatarAnchor,
    ) -> PhysicalPoint {
        let support = self.anchor_for_rect(rect, mode);

        match edge {
            // Bottom taskbar behaves like a horizontal sitting/standing surface.
            TaskbarEdge::Bottom => support,

            // For the other orientations we attach the nearest side of Nera's
            // native window to the taskbar's inner edge.
            TaskbarEdge::Top => PhysicalPoint::new(support.x, 0),
            TaskbarEdge::Left => PhysicalPoint::new(0, support.y),
            TaskbarEdge::Right => PhysicalPoint::new(rect.width(), support.y),
        }
    }

    fn sit_probe_radius(&self) -> i32 {
        (SIT_PROBE_RADIUS_AT_96_DPI * self.nera_scale_factor)
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
// Mate-style sitting state machine API
// -----------------------------------------------------------------------------

impl DesktopEnvironment {
    /// User-driven unsnap. Unlike `detach_nera`, this deliberately does NOT
    /// perform Seat -> Feet HWND rebasing: once the user pulls away, the cursor
    /// owns native window movement and rebasing is what caused the visible jump.
    pub fn begin_nera_drag(&mut self) {
        match self.placement {
            PlacementState::Free => return,
            PlacementState::PendingWindow { .. } | PlacementState::PendingTaskbar { .. } => {
                info!("Nera pending sit cancelled by drag");
            }
            PlacementState::Window(_) | PlacementState::Taskbar(_) => {
                info!("Nera unsnapped by drag: {:?}", self.placement);
            }
        }

        self.placement = PlacementState::Free;
        self.target_window = None;
        self.hidden_for_target_suspend = false;
        self.set_attached_anchor(AvatarAnchor::Feet);
    }

    /// Global hips/seat probe in physical desktop pixels.
    pub fn seat_probe_global(&self) -> Option<PhysicalPoint> {
        let rect = self.nera_rect?;
        let anchor = self.anchor_for_rect(rect, AvatarAnchor::Seat);

        Some(PhysicalPoint::new(
            rect.left + anchor.x,
            rect.top + anchor.y,
        ))
    }

    /// Horizontal support Y used only for the current-drag unsnap separation
    /// check. Vertical taskbars return None because their normal axis is X.
    pub fn current_attachment_surface_y(&self) -> Option<i32> {
        match self.placement {
            PlacementState::Window(attachment) => self
                .target_window
                .as_ref()
                .filter(|target| target.handle == attachment.target)
                .map(|target| target.rect.top),

            PlacementState::Taskbar(attachment) => self
                .taskbar
                .for_monitor(attachment.monitor)
                .filter(|taskbar| taskbar.edge == attachment.edge)
                .and_then(|taskbar| match taskbar.edge {
                    TaskbarEdge::Bottom => Some(taskbar.rect.top),
                    TaskbarEdge::Top => Some(taskbar.rect.bottom),
                    TaskbarEdge::Left | TaskbarEdge::Right => None,
                }),

            _ => None,
        }
    }

    /// Called repeatedly while a deliberate drag is armed. This is only probe
    /// detection: no native HWND movement and no target tracking begins yet.
    /// The probe is based on hips/Seat, matching Mate Engine's X3 approach.
    pub fn try_begin_sit_from_probe(&mut self) -> bool {
        if self.placement != PlacementState::Free {
            return false;
        }

        let Some(nera_hwnd) = self.nera_hwnd else {
            return false;
        };

        let Some(nera_rect) = platform::window_rect(nera_hwnd) else {
            return false;
        };

        self.nera_rect = Some(nera_rect);
        self.nera_dpi = platform::dpi_for_window(nera_hwnd);
        self.nera_scale_factor = platform::dpi_scale_factor(self.nera_dpi);
        self.monitor = platform::monitor_info(nera_hwnd);

        let probe_radius = self.sit_probe_radius();
        let seat_anchor = self.anchor_for_rect(nera_rect, AvatarAnchor::Seat);
        let probe = PhysicalPoint::new(
            nera_rect.left + seat_anchor.x,
            nera_rect.top + seat_anchor.y,
        );

        let window_candidate = self
            .discovered_windows
            .iter()
            .filter_map(|window| {
                if platform::is_window_minimized(window.handle) {
                    return None;
                }

                if probe.x < window.rect.left || probe.x > window.rect.right {
                    return None;
                }

                let distance = (probe.y - window.rect.top).abs();
                if distance > probe_radius {
                    return None;
                }

                let edge_probe = PhysicalPoint::new(
                    probe.x,
                    window.rect.top.saturating_add(1),
                );

                let visible_at_edge = platform::window_at_point(
                    edge_probe,
                    nera_hwnd,
                    &self.discovered_windows,
                ) == Some(window.handle);

                visible_at_edge.then_some((distance, window.handle))
            })
            .min_by_key(|candidate| candidate.0);

        let taskbar_candidate = self
            .taskbar
            .taskbars
            .iter()
            .filter_map(|taskbar| {
                taskbar_snap_distance(probe, *taskbar)
                    .filter(|distance| *distance <= probe_radius)
                    .map(|distance| (distance, *taskbar))
            })
            .min_by_key(|candidate| candidate.0);

        let choose_taskbar = match (window_candidate, taskbar_candidate) {
            (None, None) => return false,
            (None, Some(_)) => true,
            (Some(_), None) => false,
            (Some(window), Some(taskbar)) => taskbar.0 < window.0,
        };

        self.set_attached_anchor(AvatarAnchor::Seat);

        if choose_taskbar {
            let Some((distance, taskbar)) = taskbar_candidate else {
                return false;
            };

            self.placement = PlacementState::PendingTaskbar {
                monitor: taskbar.monitor,
                edge: taskbar.edge,
                // One complete frame lets sitting.vrma evaluate before Seat
                // becomes the native support anchor.
                settle_frames: 1,
            };

            info!(
                "Nera sit candidate: taskbar monitor=0x{:X} edge={:?} probe_distance={}px | pose first",
                taskbar.monitor.as_raw(),
                taskbar.edge,
                distance,
            );

            return true;
        }

        let Some((distance, target)) = window_candidate else {
            return false;
        };

        self.placement = PlacementState::PendingWindow {
            target,
            settle_frames: 1,
        };

        info!(
            "Nera sit candidate: window {:?} probe_distance={}px | pose first",
            target,
            distance,
        );

        true
    }

    /// Runs after animated bones have been sampled in `Last`. Returns true only
    /// on the frame a Pending* state is successfully committed to attachment.
    pub fn settle_pending_sit(&mut self) -> bool {
        match self.placement {
            PlacementState::PendingWindow {
                target,
                mut settle_frames,
            } => {
                if settle_frames > 0 {
                    settle_frames -= 1;
                    self.placement = PlacementState::PendingWindow {
                        target,
                        settle_frames,
                    };
                    return false;
                }

                self.commit_window_sit(target)
            }

            PlacementState::PendingTaskbar {
                monitor,
                edge,
                mut settle_frames,
            } => {
                if settle_frames > 0 {
                    settle_frames -= 1;
                    self.placement = PlacementState::PendingTaskbar {
                        monitor,
                        edge,
                        settle_frames,
                    };
                    return false;
                }

                self.commit_taskbar_sit(monitor, edge)
            }

            _ => false,
        }
    }

    pub fn detach_nera(&mut self) {
        // System/lifecycle detach keeps the existing stable handoff so a target
        // closing/minimizing does not leave standing feet below the surface.
        self.rebase_attachment_to_feet();

        self.placement = PlacementState::Free;
        self.target_window = None;
        self.hidden_for_target_suspend = false;
        self.set_attached_anchor(AvatarAnchor::Feet);
    }

    fn rebase_attachment_to_feet(&mut self) {
        if self.attached_anchor == AvatarAnchor::Feet {
            return;
        }

        let Some(nera_hwnd) = self.nera_hwnd else {
            return;
        };
        let Some(nera_rect) = self.nera_rect else {
            return;
        };

        let from_anchor = match self.placement {
            PlacementState::Window(_) => {
                self.anchor_for_rect(nera_rect, self.attached_anchor)
            }
            PlacementState::Taskbar(attachment) => self.taskbar_anchor_for_rect(
                nera_rect,
                attachment.edge,
                self.attached_anchor,
            ),
            PlacementState::Free
            | PlacementState::PendingWindow { .. }
            | PlacementState::PendingTaskbar { .. } => return,
        };

        let feet_anchor = match self.placement {
            PlacementState::Window(_) => {
                self.anchor_for_rect(nera_rect, AvatarAnchor::Feet)
            }
            PlacementState::Taskbar(attachment) => self.taskbar_anchor_for_rect(
                nera_rect,
                attachment.edge,
                AvatarAnchor::Feet,
            ),
            PlacementState::Free
            | PlacementState::PendingWindow { .. }
            | PlacementState::PendingTaskbar { .. } => return,
        };

        let desired = match self.placement {
            PlacementState::Window(_) => PhysicalPoint::new(
                nera_rect.left,
                nera_rect.top + from_anchor.y - feet_anchor.y,
            ),
            PlacementState::Taskbar(attachment) => match attachment.edge {
                TaskbarEdge::Top | TaskbarEdge::Bottom => PhysicalPoint::new(
                    nera_rect.left,
                    nera_rect.top + from_anchor.y - feet_anchor.y,
                ),
                TaskbarEdge::Left | TaskbarEdge::Right => PhysicalPoint::new(
                    nera_rect.left + from_anchor.x - feet_anchor.x,
                    nera_rect.top,
                ),
            },
            PlacementState::Free
            | PlacementState::PendingWindow { .. }
            | PlacementState::PendingTaskbar { .. } => return,
        };

        if desired.x == nera_rect.left && desired.y == nera_rect.top {
            return;
        }

        if platform::move_window(nera_hwnd, desired).is_ok() {
            self.nera_rect = platform::window_rect(nera_hwnd);
        }
    }

    fn commit_window_sit(&mut self, handle: NativeWindowHandle) -> bool {
        let Some(nera_hwnd) = self.nera_hwnd else {
            self.placement = PlacementState::Free;
            return false;
        };
        let Some(nera_rect) = platform::window_rect(nera_hwnd) else {
            self.placement = PlacementState::Free;
            return false;
        };

        if !self.track_window(handle) {
            self.placement = PlacementState::Free;
            self.set_attached_anchor(AvatarAnchor::Feet);
            return false;
        }

        let Some(target_rect) = self
            .target_window
            .as_ref()
            .map(|target| target.rect)
        else {
            self.placement = PlacementState::Free;
            return false;
        };

        self.set_attached_anchor(AvatarAnchor::Seat);
        let anchor = self.anchor_for_rect(nera_rect, AvatarAnchor::Seat);
        let support_x = nera_rect.left + anchor.x;
        let requested_offset = support_x - target_rect.left;
        let (desired, support_offset_x) = position_on_window(
            target_rect,
            anchor,
            requested_offset,
            self.window_edge_margin(),
        );

        self.placement = PlacementState::Window(WindowAttachment {
            target: handle,
            support_offset_x,
        });

        if platform::move_window(nera_hwnd, desired).is_ok() {
            self.nera_rect = platform::window_rect(nera_hwnd);
        }

        info!(
            "Nera sitting committed to window: {:?} | Seat calibrated | support_offset_x={}",
            handle,
            support_offset_x,
        );

        true
    }

    fn commit_taskbar_sit(
        &mut self,
        monitor: NativeMonitorHandle,
        edge: TaskbarEdge,
    ) -> bool {
        let Some(taskbar) = self
            .taskbar
            .for_monitor(monitor)
            .copied()
            .filter(|taskbar| taskbar.edge == edge)
        else {
            self.placement = PlacementState::Free;
            self.set_attached_anchor(AvatarAnchor::Feet);
            return false;
        };

        let Some(nera_hwnd) = self.nera_hwnd else {
            self.placement = PlacementState::Free;
            return false;
        };
        let Some(nera_rect) = platform::window_rect(nera_hwnd) else {
            self.placement = PlacementState::Free;
            return false;
        };

        self.set_attached_anchor(AvatarAnchor::Seat);
        let anchor = self.taskbar_anchor_for_rect(
            nera_rect,
            taskbar.edge,
            AvatarAnchor::Seat,
        );
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

        let desired = position_on_taskbar(taskbar, anchor, along_offset).0;

        if platform::move_window(nera_hwnd, desired).is_ok() {
            self.nera_rect = platform::window_rect(nera_hwnd);
        }

        info!(
            "Nera sitting committed to taskbar: monitor=0x{:X} edge={:?} | Seat calibrated | along_offset={}",
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
        PlacementState::Free
        | PlacementState::PendingWindow { .. }
        | PlacementState::PendingTaskbar { .. } => {
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

    let anchor = environment.attachment_anchor_for_rect(nera_rect);
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

    let anchor = environment.taskbar_anchor_for_rect(
        nera_rect,
        taskbar.edge,
        environment.attached_anchor,
    );
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
