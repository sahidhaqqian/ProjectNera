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
    interaction::{DesktopPetInteraction, InteractionState},
    platform::{self, NativeMonitorHandle, NativeWindowHandle, PhysicalPoint, PhysicalRect},
};

mod geometry;

use geometry::*;

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

            keep_feet_on_surface(hwnd, rect, feet.y, floor.y, rect.left, &mut environment);

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
        if let Some(hit) = nearest_surface_in_band(feet, SUPPORT_EPSILON_PX, &environment) {
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
        (physics.vertical_velocity + GRAVITY_PX_PER_SEC2 * dt).min(TERMINAL_VELOCITY_PX_PER_SEC);

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

        info!("Nera landed: {:?} | surface_y={}px", hit.support, hit.y);
        return;
    }

    move_nera(
        hwnd,
        rect.left,
        rect.top.saturating_add(delta_y),
        &mut environment,
    );
}

fn reset_freefall(physics: &mut DesktopPhysicsState, animation: &mut NeraAnimationState) {
    physics.vertical_velocity = 0.0;
    physics.subpixel_y = 0.0;
    physics.falling = false;
    physics.support = None;
    physics.support_anchor_logical = None;
    physics.landing_latched = false;
    animation.cancel_physics_motion();
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

fn move_nera(hwnd: NativeWindowHandle, x: i32, y: i32, environment: &mut DesktopEnvironment) {
    if platform::move_window(hwnd, PhysicalPoint::new(x, y)).is_ok() {
        environment.nera_rect = platform::window_rect(hwnd);
    }
}
