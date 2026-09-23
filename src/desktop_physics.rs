//! Project Nera — Phase 5.7 desktop-space gravity / support physics.
//!
//! Physics is intentionally expressed in global *physical desktop pixels*.
//! The compact transparent HWND is only translated; its width/height are not
//! used as collision geometry. The avatar's projected standing feet anchor is
//! the support point that collides with desktop, taskbar, and window tops.

use bevy::prelude::*;

use crate::{
    animation::{NeraAnimationClips, NeraAnimationState},
    environment::{DesktopEnvironment, PlacementState, TaskbarEdge},
    interaction::InteractionState,
    platform::{self, NativeMonitorHandle, NativeWindowHandle, PhysicalPoint, PhysicalRect},
    DesktopPetInteraction,
};

// -----------------------------------------------------------------------------
// Tuning
// -----------------------------------------------------------------------------

/// Downward acceleration in physical desktop pixels / second².
const GRAVITY_PX_PER_SEC2: f32 = 2_350.0;

/// Prevent giant one-frame jumps after stalls/debugger breaks.
const TERMINAL_VELOCITY_PX_PER_SEC: f32 = 2_800.0;

/// A support this close to the feet is considered already grounded. This also
/// absorbs integer rounding and tiny animation-anchor noise.
const SUPPORT_EPSILON_PX: i32 = 4;

/// Clamp simulation delta so an OS hitch cannot tunnel through narrow surfaces.
const MAX_PHYSICS_DT: f32 = 1.0 / 20.0;

// -----------------------------------------------------------------------------
// Runtime state
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PhysicsSupport {
    Desktop {
        monitor: NativeMonitorHandle,
    },
    Window {
        target: NativeWindowHandle,
        /// Preserve the compact HWND's horizontal placement relative to the
        /// supporting window while that window is moved.
        nera_left_offset: i32,
    },
    Taskbar {
        monitor: NativeMonitorHandle,
        edge: TaskbarEdge,
    },
}

#[derive(Resource, Debug)]
pub(crate) struct DesktopPhysicsState {
    /// Positive Y is downward in Win32 desktop coordinates.
    pub vertical_velocity: f32,
    pub falling: bool,
    pub support: Option<PhysicsSupport>,

    /// Standing feet anchor captured before physics takes ownership.
    ///
    /// This is intentionally frozen while falling / supported. The Falling and
    /// Falling_To_Landing clips are allowed to move the visual feet without
    /// changing the rigid-body collision point underneath the avatar.
    support_anchor_logical: Option<Vec2>,

    /// Fractional physical pixels accumulated between native MoveWindow calls.
    subpixel_y: f32,

    /// Set after a fall has committed its single Landing transition. It stays
    /// latched while support is stable and is cleared only when a genuinely new
    /// unsupported fall begins. This prevents edge/corner support jitter from
    /// replaying Falling_To_Landing.
    landing_latched: bool,
}

impl Default for DesktopPhysicsState {
    fn default() -> Self {
        Self {
            vertical_velocity: 0.0,
            falling: false,
            support: None,
            support_anchor_logical: None,
            subpixel_y: 0.0,
            landing_latched: false,
        }
    }
}

impl DesktopPhysicsState {
    /// Refresh the rigid-body standing anchor after a deliberate geometry
    /// change such as Character Size / Ground Offset. Normal animation must
    /// not rewrite this anchor while physics owns it.
    pub(crate) fn refresh_support_anchor(&mut self, anchor: Option<Vec2>) {
        if anchor.is_some() {
            self.support_anchor_logical = anchor;
        }
    }

    /// Reset every cached physics value after an external/native position
    /// recovery (for example restoring Nera's remembered position from tray).
    ///
    /// The next physics frame will capture a fresh standing-feet anchor and
    /// rediscover support from the recovered HWND position instead of reusing
    /// stale support/landing state from the old desktop location.
    pub(crate) fn reset_for_position_recovery(&mut self) {
        self.vertical_velocity = 0.0;
        self.falling = false;
        self.support = None;
        self.support_anchor_logical = None;
        self.subpixel_y = 0.0;
        self.landing_latched = false;
    }

}

#[derive(Clone, Copy, Debug)]
struct SurfaceHit {
    y: i32,
    support: PhysicsSupport,
}

// -----------------------------------------------------------------------------
// Main simulation
// -----------------------------------------------------------------------------

pub(crate) fn update_desktop_physics(
    time: Res<Time>,
    interaction: Res<DesktopPetInteraction>,
    mut environment: ResMut<DesktopEnvironment>,
    clips: Res<NeraAnimationClips>,
    mut animation: ResMut<NeraAnimationState>,
    mut physics: ResMut<DesktopPhysicsState>,
) {
    let Some(hwnd) = environment.nera_hwnd else {
        return;
    };

    // Phase 4 sitting owns native placement completely. Physics support is a
    // standing/free-placement concept and must never reinterpret a sitting
    // attachment as a gravity surface.
    if environment.placement != PlacementState::Free {
        reset_freefall(&mut physics, &mut animation);
        return;
    }

    // A click/hit must keep the current desktop support. Only a real drag owns
    // native HWND movement. Clearing support already on Pressed made every body
    // click temporarily unground Nera and was especially harmful on the third
    // hit that starts Death01.
    if interaction.state == InteractionState::Dragging {
        physics.support = None;
        reset_freefall(&mut physics, &mut animation);
        return;
    }

    let Some(rect) = environment.nera_rect else {
        return;
    };

    // Capture the *standing* feet anchor once, before Falling/Landing can
    // animate those bones away from the support point. The environment keeps
    // sampling the visual skeleton for Phase 4, but desktop physics behaves
    // like a rigid body and therefore owns a stable collision anchor.
    if physics.support_anchor_logical.is_none() {
        physics.support_anchor_logical = environment.feet_anchor_logical;
    }

    let Some(feet) = global_feet_anchor(
        physics.support_anchor_logical,
        environment.nera_scale_factor,
        rect,
    ) else {
        return;
    };

    // Preserve Phase 5.x Death grounding. It already has bespoke body-wide
    // ground stabilization and should not fight another vertical owner.
    if animation.death_active() {
        return;
    }

    // Existing support gets first ownership. Window supports are revalidated
    // every frame, so closing/minimizing/moving a supporting window naturally
    // turns into unsupported freefall without touching Phase 4 tracking.
    if let Some(support) = physics.support {
        if let Some((surface_y, desired_left)) =
            resolve_support_surface(support, feet.x, rect.left, &environment)
        {
            keep_feet_on_surface(
                hwnd,
                rect,
                feet.y,
                surface_y,
                desired_left,
                &mut environment,
            );
            physics.vertical_velocity = 0.0;
            physics.subpixel_y = 0.0;
            physics.falling = false;
            physics.landing_latched = true;
            animation.set_physics_falling(false);
            return;
        }

        info!("Nera physics support lost: {:?}", support);
        physics.support = None;
    }

    // Hard floor recovery. The pre-physics DesktopPet window is allowed to be
    // partially below the monitor work area (for example the historical
    // 600x950 window at y=200). If the standing feet anchor therefore starts
    // *below* the desktop/taskbar floor, a downward-only sweep can never cross
    // that surface and Nera would fall forever. Treat the monitor floor as a
    // hard boundary and depenetrate upward before normal unsupported detection.
    if let Some(floor) = floor_surface_for_feet(feet, &environment) {
        if feet.y > floor.y.saturating_add(SUPPORT_EPSILON_PX) {
            let was_falling = physics.falling;

            keep_feet_on_surface(
                hwnd,
                rect,
                feet.y,
                floor.y,
                rect.left,
                &mut environment,
            );

            physics.vertical_velocity = 0.0;
            physics.subpixel_y = 0.0;
            physics.falling = false;
            physics.support = Some(floor.support);

            if was_falling && !physics.landing_latched {
                animation.land_from_fall(clips.falling_to_landing_duration());
                physics.landing_latched = true;
            } else {
                animation.set_physics_falling(false);
                physics.landing_latched = true;
            }

            info!(
                "Nera physics floor recovery: {:?} | feet_y={} -> floor_y={}",
                floor.support, feet.y, floor.y
            );
            return;
        }
    }

    // If we are already almost touching a valid surface, acquire it directly
    // instead of playing a one-frame fall/landing sequence. This covers startup
    // and exact releases onto a surface.
    if !physics.falling {
        if let Some(hit) = nearest_surface_in_band(
            feet,
            SUPPORT_EPSILON_PX,
            &environment,
        ) {
            keep_feet_on_surface(hwnd, rect, feet.y, hit.y, rect.left, &mut environment);
            physics.support = Some(hit.support);
            physics.vertical_velocity = 0.0;
            physics.subpixel_y = 0.0;
            physics.landing_latched = true;
            animation.set_physics_falling(false);
            debug!("Nera physics support acquired: {:?}", hit.support);
            return;
        }

        physics.falling = true;
        physics.vertical_velocity = 0.0;
        physics.subpixel_y = 0.0;
        physics.landing_latched = false;
        animation.set_physics_falling(true);
        info!(
            "Nera unsupported -> Falling | feet=({}, {}) hwnd_top={} floor_y={:?}",
            feet.x,
            feet.y,
            rect.top,
            floor_surface_for_feet(feet, &environment).map(|floor| floor.y),
        );
    }

    let dt = time.delta_secs().min(MAX_PHYSICS_DT);
    physics.vertical_velocity =
        (physics.vertical_velocity + GRAVITY_PX_PER_SEC2 * dt)
            .min(TERMINAL_VELOCITY_PX_PER_SEC);

    physics.subpixel_y += physics.vertical_velocity * dt;
    let delta_y = physics.subpixel_y.floor() as i32;

    if delta_y <= 0 {
        return;
    }
    physics.subpixel_y -= delta_y as f32;

    let next_feet_y = feet.y.saturating_add(delta_y);

    if let Some(hit) = first_surface_crossed(feet, next_feet_y, &environment) {
        let desired_top = rect.top + (hit.y - feet.y);
        move_nera(hwnd, rect.left, desired_top, &mut environment);

        physics.vertical_velocity = 0.0;
        physics.subpixel_y = 0.0;
        physics.falling = false;
        physics.support = Some(hit.support);

        if !physics.landing_latched {
            animation.land_from_fall(clips.falling_to_landing_duration());
            physics.landing_latched = true;
        } else {
            animation.set_physics_falling(false);
        }

        info!(
            "Nera landed: {:?} | surface_y={}px",
            hit.support,
            hit.y
        );
        return;
    }

    move_nera(
        hwnd,
        rect.left,
        rect.top.saturating_add(delta_y),
        &mut environment,
    );
}

fn reset_freefall(
    physics: &mut DesktopPhysicsState,
    animation: &mut NeraAnimationState,
) {
    physics.vertical_velocity = 0.0;
    physics.subpixel_y = 0.0;
    physics.falling = false;
    physics.support = None;
    physics.support_anchor_logical = None;
    physics.landing_latched = false;
    animation.cancel_physics_motion();
}

// -----------------------------------------------------------------------------
// Support geometry
// -----------------------------------------------------------------------------

fn global_feet_anchor(
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
fn floor_surface_for_feet(
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
        .filter(|taskbar| {
            taskbar.edge == TaskbarEdge::Bottom
                && x_inside(feet.x, taskbar.rect)
        })
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

fn resolve_support_surface(
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
            if !platform::is_window_alive(target)
                || platform::is_window_minimized(target)
                || platform::is_window_cloaked(target)
            {
                return None;
            }

            let rect = platform::window_rect(target)?;

            // A maximized/fullscreen app moves its top edge against the monitor
            // ceiling. Treat that as the old standing surface disappearing,
            // rather than teleporting Nera upward with the resized window.
            if let Some(monitor) = platform::monitor_info(target) {
                let top_guard =
                    (24.0 * environment.nera_scale_factor.max(0.5)).round() as i32;

                if rect.top <= monitor.bounds.top.saturating_add(top_guard) {
                    return None;
                }
            }

            x_inside(feet_x, rect)
                .then(|| (rect.top, rect.left.saturating_add(nera_left_offset)))
        }

        PhysicsSupport::Taskbar { monitor, edge } => environment
            .taskbar
            .for_monitor(monitor)
            .copied()
            .filter(|taskbar| taskbar.edge == edge && edge == TaskbarEdge::Bottom)
            .map(|taskbar| (taskbar.rect.top, current_left)),
    }
}

fn nearest_surface_in_band(
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

fn first_surface_crossed(
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

fn collect_surfaces(
    feet_x: i32,
    environment: &DesktopEnvironment,
) -> Vec<SurfaceHit> {
    let mut surfaces = Vec::with_capacity(
        environment.discovered_windows.len() + environment.taskbar.taskbars.len() + 2,
    );

    // Ordinary top-level application windows. Discovery decides *which*
    // HWNDs are candidates, but geometry is refreshed here so a window being
    // dragged/resized cannot leave collision 0.2 s behind the visible edge.
    for window in &environment.discovered_windows {
        if !platform::is_window_alive(window.handle)
            || platform::is_window_minimized(window.handle)
            || platform::is_window_cloaked(window.handle)
        {
            continue;
        }

        let Some(window_rect) = platform::window_rect(window.handle) else {
            continue;
        };

        if x_inside(feet_x, window_rect) {
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
        if taskbar.edge == TaskbarEdge::Bottom
            && x_inside(feet_x, taskbar.rect)
        {
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

fn keep_feet_on_surface(
    hwnd: NativeWindowHandle,
    rect: PhysicalRect,
    feet_y: i32,
    surface_y: i32,
    desired_left: i32,
    environment: &mut DesktopEnvironment,
) {
    let delta_y = surface_y - feet_y;
    if delta_y == 0 && desired_left == rect.left {
        return;
    }

    move_nera(
        hwnd,
        desired_left,
        rect.top.saturating_add(delta_y),
        environment,
    );
}

fn move_nera(
    hwnd: NativeWindowHandle,
    x: i32,
    y: i32,
    environment: &mut DesktopEnvironment,
) {
    if platform::move_window(hwnd, PhysicalPoint::new(x, y)).is_ok() {
        environment.nera_rect = platform::window_rect(hwnd);
    }
}
