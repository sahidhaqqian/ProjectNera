//! Pure support-anchor projection plus desktop/window/taskbar surface discovery.
use bevy::prelude::*;

use crate::{
    environment::{DesktopEnvironment, TaskbarEdge},
    platform::{self, NativeWindowHandle, PhysicalPoint, PhysicalRect},
};

use super::{PhysicsSupport, SUPPORT_EPSILON_PX};

#[derive(Clone, Copy, Debug)]
pub(super) struct SurfaceHit {
    pub(super) y: i32,
    pub(super) support: PhysicsSupport,
}

/// Keep the whole transparent canvas within this monitor's work area. These
/// bounds constrain autonomous targets, never an immediate position clamp.
pub(super) fn canvas_walking_bounds(work: PhysicalRect, width: i32) -> Option<(i32, i32)> {
    let left = work.left.saturating_add(8);
    let right = work.right.saturating_sub(width).saturating_sub(8);
    (width > 0 && work.height() > 0 && right > left).then_some((left, right))
}

pub(super) fn window_top_on_monitor(window: PhysicalRect, work: PhysicalRect) -> bool {
    window.top >= work.top
        && window.top < work.bottom
        && window.left < work.right
        && window.right > work.left
}

/// Convert support-foot coordinates to HWND-left coordinates before intersecting
/// them with monitor bounds. Window-top walking needs a small foot safety margin,
/// but does not require the entire transparent canvas to fit on the title bar.
pub(super) fn window_walking_bounds(
    monitor_bounds: (i32, i32),
    window: PhysicalRect,
    feet_offset_x: i32,
    scale: f64,
) -> Option<(i32, i32)> {
    if !scale.is_finite() || scale <= 0.0 || window.width() <= 0 || window.height() <= 0 {
        return None;
    }
    let margin = (16.0 * scale.clamp(0.5, 4.0)).round() as i32;
    let left = monitor_bounds.0.max(
        window
            .left
            .saturating_add(margin)
            .saturating_sub(feet_offset_x),
    );
    let right = monitor_bounds.1.min(
        window
            .right
            .saturating_sub(margin)
            .saturating_sub(feet_offset_x),
    );
    (right > left).then_some((left, right))
}

/// External dragging/resizing may leave the current canvas outside safe bounds.
/// Permit gradual recovery toward them without allowing further outward travel.
pub(super) fn step_stays_safe_or_returns_to_bounds(
    current: i32,
    next: i32,
    (min, max): (i32, i32),
) -> bool {
    if min >= max {
        return false;
    }
    if current < min {
        next >= current && next <= max
    } else if current > max {
        next <= current && next >= min
    } else {
        (min..=max).contains(&next)
    }
}

pub(super) fn live_window_support_rect(
    target: NativeWindowHandle,
    environment: &DesktopEnvironment,
) -> Option<PhysicalRect> {
    if !platform::is_window_alive(target)
        || !platform::window_is_visible(target)
        || platform::is_window_minimized(target)
        || platform::is_window_cloaked(target)
    {
        return None;
    }
    let rect = platform::window_rect(target)?;
    let monitor = platform::monitor_info(target)?;
    let top_guard = (24.0 * environment.nera_scale_factor.max(0.5)).round() as i32;
    // A maximized/fullscreen app has removed its usable top standing surface.
    if rect.width() <= 0
        || rect.height() <= 0
        || rect.top <= monitor.bounds.top.saturating_add(top_guard)
    {
        return None;
    }
    Some(rect)
}

pub(super) fn global_feet_anchor(
    local_logical: Option<Vec2>,
    scale_factor: f64,
    rect: PhysicalRect,
) -> Option<PhysicalPoint> {
    let local = local_logical?;
    if !local.x.is_finite() || !local.y.is_finite() {
        return None;
    }

    let scale = scale_factor.max(0.25) as f32;

    Some(PhysicalPoint::new(
        rect.left.saturating_add((local.x * scale).round() as i32),
        rect.top.saturating_add((local.y * scale).round() as i32),
    ))
}

/// Returns the hard desktop floor for the monitor currently owning Nera.
///
/// A bottom taskbar is preferred when its top edge is the work-area floor; for
/// other taskbar orientations the monitor work-area bottom remains the gravity
/// floor. Using the environment monitor also avoids mixing floors from vertically
/// stacked monitors that happen to share the same X range.
pub(super) fn floor_surface_for_feet(
    feet: PhysicalPoint,
    environment: &DesktopEnvironment,
) -> Option<SurfaceHit> {
    // The desktop floor belongs to the monitor containing/nearest the Nera
    // HWND, not to the projected foot X. At screen corners an animation or
    // projection can put feet.x a few pixels outside monitor.bounds even while
    // the actual HWND is still on that monitor. Requiring x_inside() here can
    // therefore delete the only hard floor and create an infinite fall.
    let monitor = environment.monitor.or_else(|| {
        let nera_rect = environment.nera_rect?;
        let center_x = nera_rect.left.saturating_add(nera_rect.width() / 2);
        let center_y = nera_rect.top.saturating_add(nera_rect.height() / 2);

        platform::enumerate_monitors()
            .into_iter()
            .min_by_key(|candidate| {
                let dx = if center_x < candidate.bounds.left {
                    candidate.bounds.left - center_x
                } else if center_x > candidate.bounds.right {
                    center_x - candidate.bounds.right
                } else {
                    0
                };

                let dy = if center_y < candidate.bounds.top {
                    candidate.bounds.top - center_y
                } else if center_y > candidate.bounds.bottom {
                    center_y - candidate.bounds.bottom
                } else {
                    0
                };

                i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy)
            })
    })?;

    // Prefer an explicit bottom taskbar only when the projected foot is over
    // its horizontal span. Otherwise the monitor work-area bottom remains a
    // valid hard floor. This is what prevents corner/off-edge infinite fall.
    if let Some(taskbar) = environment
        .taskbar
        .for_monitor(monitor.handle)
        .copied()
        .filter(|taskbar| taskbar.edge == TaskbarEdge::Bottom && x_inside(feet.x, taskbar.rect))
    {
        return Some(SurfaceHit {
            y: taskbar.rect.top,
            support: PhysicsSupport::Taskbar {
                monitor: taskbar.monitor,
                edge: taskbar.edge,
            },
        });
    }

    Some(SurfaceHit {
        y: monitor.work_area.bottom,
        support: PhysicsSupport::Desktop {
            monitor: monitor.handle,
        },
    })
}

pub(super) fn resolve_support_surface(
    support: PhysicsSupport,
    feet_x: i32,
    current_left: i32,
    environment: &DesktopEnvironment,
) -> Option<(i32, i32)> {
    match support {
        PhysicsSupport::Desktop { monitor } => platform::enumerate_monitors()
            .into_iter()
            .find(|info| info.handle == monitor)
            .map(|info| (info.work_area.bottom, current_left)),

        PhysicsSupport::Window {
            target,
            nera_left_offset,
        } => {
            let rect = live_window_support_rect(target, environment)?;
            let desired_left = rect.left.saturating_add(nera_left_offset);
            // Evaluate the feet after following the moving support. Testing the
            // old world X first makes a fast lateral window drag lose support.
            let desired_feet_x = feet_x.saturating_add(desired_left.saturating_sub(current_left));
            if !x_inside(desired_feet_x, rect) {
                return None;
            }

            // Alt-Tab / clicking another app does not move or minimize the
            // supporting window, but another normal window can now cover it
            // in z-order at the exact point Nera stands. Geometry alone would
            // keep reporting the (now hidden) top edge as valid support,
            // which is what made Nera look like she was floating in place
            // when the app she stood on disappeared behind another window.
            if !window_surface_visible(environment, desired_feet_x, rect, target) {
                return None;
            }

            Some((rect.top, desired_left))
        }

        PhysicsSupport::Taskbar { monitor, edge } => environment
            .taskbar
            .for_monitor(monitor)
            .copied()
            .filter(|taskbar| taskbar.edge == edge && edge == TaskbarEdge::Bottom)
            .map(|taskbar| (taskbar.rect.top, current_left)),
    }
}

pub(super) fn nearest_surface_in_band(
    feet: PhysicalPoint,
    tolerance: i32,
    environment: &DesktopEnvironment,
) -> Option<SurfaceHit> {
    let min_y = feet.y.saturating_sub(tolerance);
    let max_y = feet.y.saturating_add(tolerance);

    collect_surfaces(feet.x, environment)
        .into_iter()
        .filter(|hit| hit.y >= min_y && hit.y <= max_y)
        .min_by_key(|hit| (hit.y - feet.y).abs())
}

pub(super) fn first_surface_crossed(
    feet: PhysicalPoint,
    next_feet_y: i32,
    environment: &DesktopEnvironment,
) -> Option<SurfaceHit> {
    collect_surfaces(feet.x, environment)
        .into_iter()
        // Allow a tiny epsilon above the sampled feet because Falling.vrma can
        // shift the projected sole by a couple of pixels between frames.
        .filter(|hit| {
            hit.y >= feet.y.saturating_sub(SUPPORT_EPSILON_PX)
                && hit.y <= next_feet_y.saturating_add(SUPPORT_EPSILON_PX)
        })
        // Win32 Y grows downward: the smallest crossing Y is hit first.
        .min_by_key(|hit| hit.y)
}

fn collect_surfaces(feet_x: i32, environment: &DesktopEnvironment) -> Vec<SurfaceHit> {
    let mut surfaces = Vec::with_capacity(
        environment.discovered_windows.len() + environment.taskbar.taskbars.len() + 2,
    );

    // Ordinary top-level application windows. Discovery decides *which*
    // HWNDs are candidates, but geometry is refreshed here so a window being
    // dragged/resized cannot leave collision 0.2 s behind the visible edge.
    for window in &environment.discovered_windows {
        let Some(window_rect) = live_window_support_rect(window.handle, environment) else {
            continue;
        };

        if x_inside(feet_x, window_rect)
            && window_surface_visible(environment, feet_x, window_rect, window.handle)
        {
            surfaces.push(SurfaceHit {
                y: window_rect.top,
                support: PhysicsSupport::Window {
                    target: window.handle,
                    nera_left_offset: environment
                        .nera_rect
                        .map(|nera| nera.left - window_rect.left)
                        .unwrap_or_default(),
                },
            });
        }
    }

    // The taskbar's upper edge is also a horizontal collision surface. Prefer
    // this explicit support over Desktop when both share the same Y.
    for taskbar in &environment.taskbar.taskbars {
        if taskbar.edge == TaskbarEdge::Bottom && x_inside(feet_x, taskbar.rect) {
            surfaces.push(SurfaceHit {
                y: taskbar.rect.top,
                support: PhysicsSupport::Taskbar {
                    monitor: taskbar.monitor,
                    edge: taskbar.edge,
                },
            });
        }
    }

    // The monitor floor is a hard boundary and must never disappear merely
    // because the projected feet X drifts a few pixels beyond a screen edge.
    // floor_surface_for_feet() resolves the monitor from the Nera HWND first.
    if let Some(floor) = floor_surface_for_feet(
        PhysicalPoint::new(feet_x, environment.nera_rect.map(|r| r.top).unwrap_or(0)),
        environment,
    ) {
        // If this is an explicit taskbar hit it is already present above; the
        // generic desktop floor is still useful as a hard fallback.
        match floor.support {
            PhysicsSupport::Desktop { .. } => surfaces.push(floor),
            PhysicsSupport::Taskbar { monitor, .. } => {
                if let Some(info) = platform::enumerate_monitors()
                    .into_iter()
                    .find(|info| info.handle == monitor)
                {
                    surfaces.push(SurfaceHit {
                        y: info.work_area.bottom,
                        support: PhysicsSupport::Desktop { monitor },
                    });
                }
            }
            PhysicsSupport::Window { .. } => {}
        }
    }

    // At identical Y, taskbar/window supports are more informative than the
    // generic desktop floor and can be revalidated independently later.
    surfaces.sort_by_key(|hit| {
        let priority = match hit.support {
            PhysicsSupport::Window { .. } => 0,
            PhysicsSupport::Taskbar { .. } => 1,
            PhysicsSupport::Desktop { .. } => 2,
        };
        (hit.y, priority)
    });
    surfaces.dedup_by_key(|hit| hit.y);

    surfaces
}

fn x_inside(x: i32, rect: PhysicalRect) -> bool {
    x >= rect.left && x <= rect.right
}

/// True when `handle` is actually the topmost/visible window at the point
/// where Nera would stand on it, instead of merely being alive and untouched
/// geometrically. Alt-Tabbing to another app, or any other window being
/// raised above it, changes nothing about a window's rect or minimized/
/// cloaked flags, so those checks alone cannot detect it. Uses the same
/// native hit-test approach as the initial sit probe in `sitting.rs`.
pub(super) fn window_surface_visible(
    environment: &DesktopEnvironment,
    feet_x: i32,
    rect: PhysicalRect,
    handle: NativeWindowHandle,
) -> bool {
    let Some(nera_hwnd) = environment.nera_hwnd else {
        // No Nera HWND to exclude from the hit test yet; do not treat this
        // as a valid surface until it can be verified.
        return false;
    };

    let probe = PhysicalPoint::new(
        feet_x.clamp(rect.left, rect.right.saturating_sub(1)),
        rect.top.saturating_add(1),
    );

    platform::window_at_point(probe, nera_hwnd, &environment.discovered_windows) == Some(handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        environment::TaskbarInfo,
        platform::{MonitorInfo, NativeMonitorHandle},
    };

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> PhysicalRect {
        PhysicalRect {
            left,
            top,
            right,
            bottom,
        }
    }

    fn environment_with_monitor() -> DesktopEnvironment {
        let monitor = NativeMonitorHandle::from_raw(7);
        DesktopEnvironment {
            nera_rect: Some(rect(100, 50, 700, 1000)),
            monitor: Some(MonitorInfo {
                handle: monitor,
                bounds: rect(0, 0, 1920, 1080),
                work_area: rect(0, 0, 1920, 1040),
                is_primary: true,
            }),
            ..default()
        }
    }

    #[test]
    fn canvas_bounds_stay_on_negative_and_offset_monitors() {
        let bounds = canvas_walking_bounds(rect(-1920, 1080, 0, 2120), 600).unwrap();
        assert_eq!(bounds, (-1912, -608));
        assert!(canvas_walking_bounds(rect(-1920, 1080, 0, 2120), 2000).is_none());
        assert!(canvas_walking_bounds(rect(0, 0, 1920, 1040), 0).is_none());
    }

    #[test]
    fn window_bounds_intersect_canvas_and_dpi_scaled_foot_margins() {
        let monitor = (-1912, -608);
        let window = rect(-1500, 1200, -700, 1800);
        let bounds = window_walking_bounds(monitor, window, 300, 1.5).unwrap();
        assert_eq!(bounds, (-1776, -1024));
        for left in [bounds.0, bounds.1] {
            let feet = left + 300;
            assert!((-1476..=-724).contains(&feet));
            assert!((monitor.0..=monitor.1).contains(&left));
        }
        let clipped =
            window_walking_bounds(monitor, rect(-2000, 1200, 100, 1800), 300, 1.0).unwrap();
        assert_eq!(clipped, monitor);
        assert!(window_walking_bounds(monitor, rect(0, 100, 100, 200), 300, 1.0).is_none());
        assert!(window_walking_bounds(monitor, window, 300, f64::NAN).is_none());
        assert!(window_walking_bounds(monitor, rect(-1500, 100, -1490, 200), 300, 1.0).is_none());
    }

    #[test]
    fn moving_window_preserves_local_home_and_destination_geometry() {
        let monitor = (8, 1312);
        let first = rect(400, 200, 900, 700);
        let moved = rect(500, 200, 1000, 700);
        let first_bounds = window_walking_bounds(monitor, first, 300, 1.0).unwrap();
        let moved_bounds = window_walking_bounds(monitor, moved, 300, 1.0).unwrap();
        assert_eq!(moved_bounds, (first_bounds.0 + 100, first_bounds.1 + 100));
        assert_eq!(first_bounds.0 - first.left, moved_bounds.0 - moved.left);
        assert_eq!(first_bounds.1 - first.left, moved_bounds.1 - moved.left);
    }

    #[test]
    fn spanning_window_has_independent_safe_ranges_on_each_monitor() {
        let left_monitor = rect(-1920, 0, 0, 1040);
        let right_monitor = rect(0, 0, 1920, 1040);
        let window = rect(-1800, 300, 1000, 900);
        for monitor in [left_monitor, right_monitor] {
            assert!(window_top_on_monitor(window, monitor));
            let canvas = canvas_walking_bounds(monitor, 600).unwrap();
            let (min, max) = window_walking_bounds(canvas, window, 300, 1.0).unwrap();
            assert!(min >= monitor.left && max + 600 <= monitor.right);
            assert!(min + 300 >= window.left + 16);
            assert!(max + 300 <= window.right - 16);
        }
        // A horizontally overlapping, vertically offset monitor cannot host
        // walking on a top edge that is actually above its visible screen.
        assert!(!window_top_on_monitor(window, rect(0, 500, 1920, 1540)));
        assert!(!window_top_on_monitor(window, rect(1920, 0, 3840, 1040)));
    }

    #[test]
    fn resize_recovery_permits_inward_steps_without_edge_teleports() {
        let bounds = (8, 1312);
        assert!(step_stays_safe_or_returns_to_bounds(-500, -496, bounds));
        assert!(!step_stays_safe_or_returns_to_bounds(-500, -504, bounds));
        assert!(step_stays_safe_or_returns_to_bounds(1500, 1496, bounds));
        assert!(!step_stays_safe_or_returns_to_bounds(1500, 1504, bounds));
        assert!(!step_stays_safe_or_returns_to_bounds(10, 7, bounds));
        assert!(!step_stays_safe_or_returns_to_bounds(1310, 1313, bounds));
        assert!(!step_stays_safe_or_returns_to_bounds(-500, 1400, bounds));
        assert!(!step_stays_safe_or_returns_to_bounds(1500, -1, bounds));
    }

    #[test]
    fn projected_feet_use_dpi_scale_and_reject_non_finite_anchors() {
        let window = rect(-200, 100, 400, 1050);
        assert_eq!(
            global_feet_anchor(Some(Vec2::new(10.0, 20.0)), 1.5, window),
            Some(PhysicalPoint::new(-185, 130))
        );
        assert_eq!(
            global_feet_anchor(Some(Vec2::new(f32::NAN, 1.0)), 1.0, window),
            None
        );
        assert_eq!(global_feet_anchor(None, 1.0, window), None);
    }

    #[test]
    fn bottom_taskbar_is_floor_only_inside_its_horizontal_span() {
        let mut environment = environment_with_monitor();
        let monitor = environment.monitor.unwrap().handle;
        environment.taskbar.taskbars.push(TaskbarInfo {
            monitor,
            rect: rect(0, 1040, 1920, 1080),
            edge: TaskbarEdge::Bottom,
            is_primary: true,
        });

        let taskbar_floor =
            floor_surface_for_feet(PhysicalPoint::new(500, 900), &environment).unwrap();
        assert_eq!(taskbar_floor.y, 1040);
        assert!(matches!(
            taskbar_floor.support,
            PhysicsSupport::Taskbar { .. }
        ));

        let desktop_floor =
            floor_surface_for_feet(PhysicalPoint::new(2500, 900), &environment).unwrap();
        assert_eq!(desktop_floor.y, 1040);
        assert!(matches!(
            desktop_floor.support,
            PhysicsSupport::Desktop { .. }
        ));
    }

    #[test]
    fn crossing_selects_the_first_surface_in_desktop_y_order() {
        let environment = environment_with_monitor();
        let feet = PhysicalPoint::new(500, 1000);
        let hit = first_surface_crossed(feet, 1060, &environment).unwrap();
        assert_eq!(hit.y, 1040);
        assert!(matches!(hit.support, PhysicsSupport::Desktop { .. }));
    }
}
