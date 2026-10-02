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

/// Stable surface identity. Desktop and bottom-taskbar support on one monitor
/// deliberately share an identity, so taskbar changes do not move the home.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WalkingSurfaceKey {
    Monitor(NativeMonitorHandle),
    Window(NativeWindowHandle),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WalkingSurface {
    pub key: WalkingSurfaceKey,
    /// Inclusive physical X range for the left edge of Nera's HWND.
    pub bounds: (i32, i32),
    /// Store the home/destination relative to this origin to follow a moving
    /// window (or a monitor whose position changes in Display Settings).
    pub origin_x: i32,
}

#[derive(Resource, Debug)]
pub(crate) struct DesktopPhysicsState {
    /// Positive Y is downward in Win32 desktop coordinates.
    pub vertical_velocity: f32,
    pub horizontal_velocity: f32,
    pub throwing: bool,
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
    subpixel_x: f32,

    /// Set after a fall has committed its single Landing transition. It stays
    /// latched while support is stable and is cleared only when a genuinely new
    /// unsupported fall begins. This prevents edge/corner support jitter from
    /// replaying Falling_To_Landing.
    landing_latched: bool,
    /// During the startup entrance, land on the chosen monitor's floor instead
    /// of an unrelated application window crossed on the way down.
    startup_monitor: Option<NativeMonitorHandle>,
}

impl Default for DesktopPhysicsState {
    fn default() -> Self {
        Self {
            vertical_velocity: 0.0,
            horizontal_velocity: 0.0,
            throwing: false,
            falling: false,
            support: None,
            support_anchor_logical: None,
            subpixel_y: 0.0,
            subpixel_x: 0.0,
            landing_latched: false,
            startup_monitor: None,
        }
    }
}

impl DesktopPhysicsState {
    /// Read the rigid contact plane without letting the visual pose recalibrate it.
    pub(crate) fn support_anchor_logical(&self) -> Option<Vec2> {
        self.support_anchor_logical
    }

    pub(crate) fn begin_startup_drop(
        &mut self,
        environment: &mut DesktopEnvironment,
        animation: &mut NeraAnimationState,
    ) -> bool {
        let (Some(hwnd), Some(rect), Some(monitor), Some(anchor)) = (
            environment.nera_hwnd,
            environment.nera_rect,
            environment.monitor,
            environment.feet_anchor_logical,
        ) else {
            return false;
        };
        let Some(origin) = startup_drop_origin(
            rect,
            monitor.work_area,
            anchor,
            environment.nera_scale_factor,
        ) else {
            return false;
        };
        if platform::move_window(hwnd, origin).is_err() {
            return false;
        }
        environment.nera_rect = platform::window_rect(hwnd);
        if environment.nera_rect.is_none() {
            return false;
        }
        self.reset_for_position_recovery();
        self.support_anchor_logical = Some(anchor);
        self.startup_monitor = Some(monitor.handle);
        self.falling = true;
        animation.set_physics_falling(true);
        true
    }

    pub(crate) fn cancel_startup_drop(&mut self) {
        self.startup_monitor = None;
    }

    /// Standing support remains owned by physics; sitting attachments never
    /// become walking surfaces. Refresh native facts before granting movement.
    pub(crate) fn walking_surface(
        &self,
        environment: &DesktopEnvironment,
    ) -> Option<WalkingSurface> {
        if self.falling || environment.placement != PlacementState::Free {
            return None;
        }
        let rect = environment.nera_rect?;
        // First refreshes this native monitor snapshot every frame. Reuse it
        // across brain, locomotion, and final-step validation rather than
        // enumerating the complete monitor topology in every consumer.
        let monitor = environment.monitor?;
        let monitor_bounds = canvas_walking_bounds(monitor.work_area, rect.width())?;
        match self.support? {
            PhysicsSupport::Desktop {
                monitor: support_monitor,
            }
            | PhysicsSupport::Taskbar {
                monitor: support_monitor,
                edge: TaskbarEdge::Bottom,
            } if support_monitor == monitor.handle => Some(WalkingSurface {
                key: WalkingSurfaceKey::Monitor(monitor.handle),
                bounds: monitor_bounds,
                origin_x: monitor.work_area.left,
            }),
            PhysicsSupport::Window { target, .. } => {
                let target_rect = live_window_support_rect(target, environment)?;
                // A spanning window can have its dominant monitor elsewhere.
                // Only the part of its top edge on Nera's current monitor is
                // eligible; horizontal bounds below keep travel on that part.
                if !window_top_on_monitor(target_rect, monitor.work_area) {
                    return None;
                }
                let feet = global_feet_anchor(
                    self.support_anchor_logical,
                    environment.nera_scale_factor,
                    rect,
                )?;
                if feet.x < target_rect.left
                    || feet.x >= target_rect.right
                    || !window_surface_visible(environment, feet.x, target_rect, target)
                {
                    return None;
                }
                Some(WalkingSurface {
                    key: WalkingSurfaceKey::Window(target),
                    bounds: window_walking_bounds(
                        monitor_bounds,
                        target_rect,
                        feet.x.saturating_sub(rect.left),
                        environment.nera_scale_factor,
                    )?,
                    origin_x: target_rect.left,
                })
            }
            _ => None,
        }
    }

    /// Keep native translation in the physics owner. The behavior controller
    /// supplies a bounded horizontal step without changing the support anchor.
    pub(crate) fn walk_horizontal(
        &mut self,
        left: i32,
        environment: &mut DesktopEnvironment,
    ) -> bool {
        let Some(surface) = self.walking_surface(environment) else {
            return false;
        };
        let (Some(hwnd), Some(rect)) = (environment.nera_hwnd, environment.nera_rect) else {
            return false;
        };
        if !step_stays_safe_or_returns_to_bounds(rect.left, left, surface.bounds) {
            return false;
        }
        // Probe the future foot position as well as the current one. A higher
        // window can cover only part of an otherwise valid standing surface.
        if let WalkingSurfaceKey::Window(target) = surface.key {
            let Some(target_rect) = live_window_support_rect(target, environment) else {
                return false;
            };
            if target_rect.left != surface.origin_x {
                return false;
            }
            let Some(feet) = global_feet_anchor(
                self.support_anchor_logical,
                environment.nera_scale_factor,
                rect,
            ) else {
                return false;
            };
            let next_feet_x = feet.x.saturating_add(left.saturating_sub(rect.left));
            if next_feet_x < target_rect.left
                || next_feet_x >= target_rect.right
                || !window_surface_visible(environment, next_feet_x, target_rect, target)
            {
                return false;
            }
        }
        if left == rect.left {
            return true;
        }
        if platform::move_window(hwnd, PhysicalPoint::new(left, rect.top)).is_err() {
            return false;
        }
        environment.nera_rect = platform::window_rect(hwnd);
        let Some(updated) = environment.nera_rect else {
            return false;
        };
        if let WalkingSurfaceKey::Window(target) = surface.key {
            // Physics follows this offset on the next frame. Leaving the old
            // offset here would undo each autonomous step immediately.
            self.support = Some(PhysicsSupport::Window {
                target,
                nera_left_offset: updated.left.saturating_sub(surface.origin_x),
            });
        }
        updated.left == left
    }

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
        self.horizontal_velocity = 0.0;
        self.subpixel_x = 0.0;
        self.throwing = false;
        self.vertical_velocity = 0.0;
        self.falling = false;
        self.support = None;
        self.support_anchor_logical = None;
        self.subpixel_y = 0.0;
        self.landing_latched = false;
        self.startup_monitor = None;
    }
}

// -----------------------------------------------------------------------------
// Main simulation
// -----------------------------------------------------------------------------

fn throw_launch_velocity(
    velocity: Vec2,
    scale: f32,
    settings: &crate::settings::NeraSettings,
) -> Option<Vec2> {
    if !velocity.is_finite() || !scale.is_finite() || settings.throw_strength() <= 0.0 {
        return None;
    }
    let velocity = velocity.clamp_length_max(2200.0 * scale.max(0.25)) * settings.throw_strength();
    (velocity.length_squared() > 0.0).then_some(velocity)
}

pub(crate) fn update_desktop_physics(
    time: Res<Time>,
    interaction: Res<DesktopPetInteraction>,
    settings: Res<crate::settings::NeraSettings>,
    mut throws: MessageReader<crate::interaction::NeraThrow>,
    mut environment: ResMut<DesktopEnvironment>,
    clips: Res<NeraAnimationClips>,
    mut animation: ResMut<NeraAnimationState>,
    mut physics: ResMut<DesktopPhysicsState>,
    cloud: Option<Res<crate::cloud::CloudRuntime>>,
) {
    let launch = throws.read().last().and_then(|throw| {
        throw_launch_velocity(
            throw.velocity,
            environment.nera_scale_factor as f32,
            &settings,
        )
    });
    if physics.throwing && settings.throw_strength() <= 0.0 {
        physics.throwing = false;
        physics.horizontal_velocity = 0.0;
        physics.vertical_velocity = 0.0;
        physics.subpixel_x = 0.0;
        physics.subpixel_y = 0.0;
    }
    if cloud.as_ref().is_some_and(|cloud| cloud.owns_placement()) {
        physics.reset_for_position_recovery();
        animation.cancel_physics_motion();
        return;
    }
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
    if let Some(velocity) = launch
        && !animation.death_active()
    {
        physics.reset_for_position_recovery();
        physics.horizontal_velocity = velocity.x;
        physics.vertical_velocity = velocity.y;
        physics.support_anchor_logical = environment.feet_anchor_logical;
        physics.throwing = true;
        physics.falling = true;
        animation.set_physics_falling(true);
    }
    if !settings.gravity_enabled && !physics.throwing {
        reset_freefall(&mut physics, &mut animation);
        return;
    }

    let Some(rect) = environment.nera_rect else {
        return;
    };

    // Moving the transparent canvas above the screen can make Windows report
    // a different nearest monitor. Keep the intended landing monitor until
    // touchdown, while refreshing its geometry if the work area changes.
    if let Some(target) = physics.startup_monitor {
        if let Some(monitor) = platform::enumerate_monitors()
            .into_iter()
            .find(|m| m.handle == target)
        {
            environment.monitor = Some(monitor);
        } else {
            physics.startup_monitor = None;
        }
    }

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
            physics.horizontal_velocity = 0.0;
            physics.subpixel_x = 0.0;
            physics.throwing = false;
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
            physics.horizontal_velocity = 0.0;
            physics.subpixel_x = 0.0;
            physics.throwing = false;
            physics.subpixel_y = 0.0;
            physics.falling = false;
            physics.support = Some(floor.support);
            physics.startup_monitor = None;

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
    let velocity = advance_velocity(
        Vec2::new(physics.horizontal_velocity, physics.vertical_velocity),
        dt,
        settings.gravity_enabled,
    );
    physics.horizontal_velocity = velocity.x;
    physics.vertical_velocity = velocity.y;
    if !settings.gravity_enabled && velocity.length() < 20.0 {
        reset_freefall(&mut physics, &mut animation);
        return;
    }

    physics.subpixel_y += physics.vertical_velocity * dt;
    let mut delta_y = physics.subpixel_y.trunc() as i32;
    physics.subpixel_x += physics.horizontal_velocity * dt;
    let mut delta_x = physics.subpixel_x.trunc() as i32;
    physics.subpixel_x -= delta_x as f32;
    if let Some(monitor) = environment.monitor {
        let margin = (20.0 * environment.nera_scale_factor.max(0.25)) as i32;
        let work = monitor.work_area;
        let next_x = feet.x.saturating_add(delta_x).clamp(
            work.left + margin,
            (work.right - margin).max(work.left + margin),
        );
        if next_x != feet.x.saturating_add(delta_x) {
            physics.horizontal_velocity = 0.0;
            physics.subpixel_x = 0.0;
        }
        delta_x = next_x - feet.x;
        if feet.y.saturating_add(delta_y) < work.top + margin {
            delta_y = work.top + margin - feet.y;
            physics.vertical_velocity = 0.0;
            physics.subpixel_y = 0.0;
        }
    }

    if delta_y == 0 && delta_x == 0 {
        return;
    }
    physics.subpixel_y -= delta_y as f32;

    let next_feet_y = feet.y.saturating_add(delta_y);

    let next_feet = PhysicalPoint::new(feet.x.saturating_add(delta_x), feet.y);
    let hit = if delta_y <= 0 {
        None
    } else if physics.startup_monitor.is_some() {
        floor_surface_for_feet(feet, &environment)
            .filter(|floor| feet.y <= floor.y && next_feet_y >= floor.y)
    } else {
        first_surface_crossed(next_feet, next_feet_y, &environment)
    };
    if let Some(hit) = hit {
        let desired_top = rect.top + (hit.y - feet.y);
        move_nera(
            hwnd,
            rect.left.saturating_add(delta_x),
            desired_top,
            &mut environment,
        );

        physics.vertical_velocity = 0.0;
        physics.horizontal_velocity = 0.0;
        physics.subpixel_x = 0.0;
        physics.throwing = false;
        physics.subpixel_y = 0.0;
        physics.falling = false;
        physics.support = Some(hit.support);
        physics.startup_monitor = None;

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
        rect.left.saturating_add(delta_x),
        rect.top.saturating_add(delta_y),
        &mut environment,
    );
}

fn reset_freefall(physics: &mut DesktopPhysicsState, animation: &mut NeraAnimationState) {
    physics.horizontal_velocity = 0.0;
    physics.subpixel_x = 0.0;
    physics.throwing = false;
    physics.vertical_velocity = 0.0;
    physics.subpixel_y = 0.0;
    physics.falling = false;
    physics.support = None;
    physics.support_anchor_logical = None;
    physics.landing_latched = false;
    physics.startup_monitor = None;
    animation.cancel_physics_motion();
}

fn advance_velocity(velocity: Vec2, dt: f32, gravity: bool) -> Vec2 {
    let dt = dt.clamp(0.0, MAX_PHYSICS_DT);
    let damping = (-if gravity { 0.8 } else { 3.4 } * dt).exp();
    Vec2::new(
        velocity.x * damping,
        if gravity {
            (velocity.y + GRAVITY_PX_PER_SEC2 * dt).min(TERMINAL_VELOCITY_PX_PER_SEC)
        } else {
            velocity.y * damping
        },
    )
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

fn startup_drop_origin(
    rect: PhysicalRect,
    work: PhysicalRect,
    feet: Vec2,
    scale: f64,
) -> Option<PhysicalPoint> {
    if !feet.is_finite()
        || !scale.is_finite()
        || scale <= 0.0
        || work.width() <= 0
        || work.height() <= 0
    {
        return None;
    }
    let max_left = work.right.saturating_sub(rect.width()).max(work.left);
    let left = rect.left.clamp(work.left, max_left);
    let top = work
        .top
        .saturating_sub((feet.y * scale as f32).round() as i32)
        .saturating_sub((24.0 * scale).round() as i32);
    Some(PhysicalPoint::new(left, top))
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn throw_preferences_scale_launch_once_and_preserve_gravity() {
        let mut settings = crate::settings::NeraSettings::default();
        let raw = Vec2::new(1000.0, -800.0);
        assert_eq!(throw_launch_velocity(raw, 1.0, &settings), Some(raw * 0.5));
        settings.throw_intensity = 100;
        assert_eq!(throw_launch_velocity(raw, 1.0, &settings), Some(raw));
        settings.throw_intensity = 25;
        assert_eq!(throw_launch_velocity(raw, 1.0, &settings), Some(raw * 0.25));
        assert!(
            throw_launch_velocity(Vec2::splat(10000.0), 2.0, &settings)
                .unwrap()
                .length()
                <= 1100.1
        );
        settings.throw_intensity = 0;
        assert!(throw_launch_velocity(raw, 1.0, &settings).is_none());
        settings.throw_intensity = 50;
        settings.throw_enabled = false;
        assert!(throw_launch_velocity(raw, 1.0, &settings).is_none());
        assert!(settings.gravity_enabled);
        settings.gravity_enabled = false;
        settings.throw_enabled = true;
        assert_eq!(throw_launch_velocity(raw, 1.0, &settings), Some(raw * 0.5));
        assert!(throw_launch_velocity(Vec2::NAN, 1.0, &settings).is_none());
    }

    #[test]
    fn disabling_throws_cancels_momentum_but_does_not_disable_falling() {
        let mut app = App::new();
        let mut settings = crate::settings::NeraSettings::default();
        settings.throw_enabled = false;
        let physics = DesktopPhysicsState {
            throwing: true,
            falling: true,
            horizontal_velocity: 800.0,
            vertical_velocity: -500.0,
            ..default()
        };
        app.insert_resource(settings)
            .insert_resource(physics)
            .init_resource::<Time>()
            .init_resource::<DesktopPetInteraction>()
            .init_resource::<DesktopEnvironment>()
            .init_resource::<NeraAnimationClips>()
            .init_resource::<NeraAnimationState>()
            .add_message::<crate::interaction::NeraThrow>()
            .add_systems(Update, update_desktop_physics);
        app.update();
        let physics = app.world().resource::<DesktopPhysicsState>();
        assert!(!physics.throwing);
        assert_eq!(physics.horizontal_velocity, 0.0);
        assert_eq!(physics.vertical_velocity, 0.0);
        assert!(physics.falling);
        assert!(
            app.world()
                .resource::<crate::settings::NeraSettings>()
                .gravity_enabled
        );
    }

    #[test]
    fn throws_arc_down_with_gravity_and_coast_to_rest_without_it() {
        let mut on = Vec2::new(1200.0, -900.0);
        let mut off = on;
        for _ in 0..30 {
            on = advance_velocity(on, 1.0 / 30.0, true);
            off = advance_velocity(off, 1.0 / 30.0, false);
        }
        assert!(on.y > 1000.0 && on.x > 400.0);
        assert!(off.length() < 60.0 && off.y < 0.0);
        assert_eq!(advance_velocity(Vec2::ZERO, 0.05, false), Vec2::ZERO);
        let mut physics = DesktopPhysicsState {
            throwing: true,
            horizontal_velocity: 900.0,
            vertical_velocity: -300.0,
            ..default()
        };
        physics.reset_for_position_recovery();
        assert!(!physics.throwing);
        assert_eq!(physics.horizontal_velocity, 0.0);
        assert_eq!(physics.vertical_velocity, 0.0);
    }

    #[test]
    fn falling_and_pending_sitting_never_grant_autonomous_movement() {
        let mut environment = DesktopEnvironment::default();
        let mut physics = DesktopPhysicsState {
            falling: true,
            support: Some(PhysicsSupport::Desktop {
                monitor: NativeMonitorHandle::from_raw(7),
            }),
            ..default()
        };
        assert!(physics.walking_surface(&environment).is_none());
        assert!(!physics.walk_horizontal(100, &mut environment));
        physics.falling = false;
        environment.placement = PlacementState::PendingWindow {
            target: NativeWindowHandle::from_raw(8),
            settle_frames: 2,
        };
        assert!(physics.walking_surface(&environment).is_none());
        assert!(!physics.walk_horizontal(100, &mut environment));
        environment.placement = PlacementState::PendingTaskbar {
            monitor: NativeMonitorHandle::from_raw(7),
            edge: TaskbarEdge::Bottom,
            settle_frames: 2,
        };
        assert!(physics.walking_surface(&environment).is_none());
        assert!(!physics.walk_horizontal(100, &mut environment));
    }

    #[test]
    fn entrance_starts_above_target_monitor_at_its_dpi_and_preserves_safe_x() {
        let work = PhysicalRect {
            left: -1920,
            top: 1080,
            right: 0,
            bottom: 2120,
        };
        let rect = PhysicalRect {
            left: -1700,
            top: 1200,
            right: -950,
            bottom: 2387,
        };
        let feet = Vec2::new(300.0, 800.0);
        let origin = startup_drop_origin(rect, work, feet, 1.25).unwrap();
        assert_eq!(origin.x, -1700);
        assert_eq!(origin.y + (feet.y * 1.25) as i32, work.top - 30);
        let offscreen = PhysicalRect {
            left: -4000,
            ..rect
        };
        assert_eq!(
            startup_drop_origin(offscreen, work, feet, 1.25).unwrap().x,
            work.left
        );
        assert!(startup_drop_origin(rect, work, Vec2::NAN, 1.25).is_none());
    }
}
