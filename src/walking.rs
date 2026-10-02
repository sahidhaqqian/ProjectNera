//! Destination-based locomotion. Behavior schedules; physics moves the HWND.
use crate::{
    animation::{IdleMicroAction, NeraAnimationState, VisualState},
    desktop_physics::{DesktopPhysicsState, WalkingSurface, WalkingSurfaceKey},
    environment::DesktopEnvironment,
    platform::NativeMonitorHandle,
};
use bevy::prelude::*;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub(crate) struct WalkIntent {
    pub surface: WalkingSurfaceKey,
    pub monitor: NativeMonitorHandle,
    /// Relative to support origin: moving windows carry the destination.
    pub target_x: i32,
    pub bounds: (i32, i32),
    pub speed: f32,
    pub scale: f32,
    pub return_trip: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WalkOutcome {
    Arrived,
    Edge,
    Interrupted,
}

#[derive(Resource, Default)]
pub(crate) struct WalkingRuntime {
    intent: Option<WalkIntent>,
    direction: f32,
    velocity: f32,
    fraction: f32,
    turn_remaining: f32,
    looking_around: bool,
    elapsed: f32,
    expected_x: Option<i32>,
    pub outcome: Option<WalkOutcome>,
}

impl WalkingRuntime {
    pub(crate) fn direction(&self) -> f32 {
        if self.looking_around {
            0.0
        } else {
            self.direction
        }
    }
    pub(crate) fn active(&self) -> bool {
        self.intent.is_some()
    }
    pub(crate) fn expected_x(&self) -> Option<i32> {
        self.expected_x
    }
    pub(crate) fn take_outcome(&mut self) -> Option<WalkOutcome> {
        let outcome = self.outcome.take();
        if outcome.is_some() {
            self.expected_x = None;
        }
        outcome
    }
    pub(crate) fn begin(
        &mut self,
        intent: WalkIntent,
        current_x: i32,
        turn_delay: f32,
        animation: &mut NeraAnimationState,
        lookaround_duration: Duration,
    ) -> bool {
        if self.active()
            || !animation.request_idle_variation(IdleMicroAction::LookAround, lookaround_duration)
        {
            return false;
        }
        self.intent = Some(intent);
        self.looking_around = true;
        self.direction = if intent.target_x > current_x {
            1.0
        } else {
            -1.0
        };
        self.velocity = 0.0;
        self.fraction = 0.0;
        self.elapsed = 0.0;
        self.turn_remaining = turn_delay.clamp(0.2, 0.4);
        self.expected_x = Some(current_x);
        self.outcome = None;
        true
    }
    pub(crate) fn cancel(&mut self, animation: &mut NeraAnimationState) {
        // An interruption also invalidates a completion the brain has not consumed.
        if self.active() || self.outcome.is_some() {
            self.finish(animation, WalkOutcome::Interrupted);
        }
    }
    fn finish(&mut self, animation: &mut NeraAnimationState, outcome: WalkOutcome) {
        if self.looking_around
            && animation.visual_state() == VisualState::IdleMicro(IdleMicroAction::LookAround)
        {
            animation.cancel_idle_variation();
        }
        self.looking_around = false;
        animation.set_walking(false);
        self.intent = None;
        self.direction = 0.0;
        self.velocity = 0.0;
        self.fraction = 0.0;
        // Keep the final committed X until the brain consumes the outcome.
        // Otherwise the last moving frame would look like a user relocation.
        self.outcome = Some(outcome);
    }

    fn ready_to_turn(&mut self, animation: &mut NeraAnimationState) -> bool {
        match animation.visual_state() {
            // The animation controller owns the clip's actual lifetime; no second timer.
            VisualState::IdleMicro(IdleMicroAction::LookAround) if self.looking_around => false,
            VisualState::Idle => {
                self.looking_around = false;
                true
            }
            VisualState::Walking if !self.looking_around => true,
            _ => {
                self.cancel(animation);
                false
            }
        }
    }
    fn next_step(&mut self, left: i32, target: i32, speed: f32, scale: f32, dt: f32) -> i32 {
        let dt = dt.clamp(0.0, 0.05);
        let acceleration = 240.0 * scale;
        let remaining = left.abs_diff(target) as f32;
        // Brake using v²/(2a), independent of frame rate and journey length.
        let desired = speed.min((2.0 * acceleration * remaining).sqrt());
        self.velocity += (desired - self.velocity).clamp(-acceleration * dt, acceleration * dt);
        self.fraction += self.velocity * dt;
        let whole = self.fraction.floor() as i32;
        self.fraction -= whole as f32;
        if target > left {
            left.saturating_add(whole).min(target)
        } else {
            left.saturating_sub(whole).max(target)
        }
    }
}

fn valid_destination(intent: WalkIntent, surface: WalkingSurface) -> bool {
    let target = surface.origin_x.saturating_add(intent.target_x);
    surface.key == intent.surface
        && (surface.bounds.0..=surface.bounds.1).contains(&target)
        && (intent.bounds.0..=intent.bounds.1).contains(&intent.target_x)
}

pub(crate) fn update_walking(
    time: Res<Time>,
    real_time: Res<Time<Real>>,
    mut physics: ResMut<DesktopPhysicsState>,
    mut environment: ResMut<DesktopEnvironment>,
    mut animation: ResMut<NeraAnimationState>,
    mut runtime: ResMut<WalkingRuntime>,
) {
    let Some(intent) = runtime.intent else { return };
    let (Some(surface), Some(rect)) =
        (physics.walking_surface(&environment), environment.nera_rect)
    else {
        runtime.cancel(&mut animation);
        return;
    };
    let local_x = rect.left.saturating_sub(surface.origin_x);
    if real_time.delta_secs() >= 0.5
        || environment.monitor.map(|m| m.handle) != Some(intent.monitor)
        || !valid_destination(intent, surface)
        || runtime.expected_x != Some(local_x)
    {
        runtime.cancel(&mut animation);
        return;
    }
    if !runtime.ready_to_turn(&mut animation) {
        return;
    }
    let dt = time.delta_secs().min(0.05);
    runtime.elapsed += dt;
    if runtime.elapsed > 15.0 {
        runtime.cancel(&mut animation);
        return;
    }
    // Face while idle, then begin the gait.
    if runtime.turn_remaining > 0.0 {
        runtime.turn_remaining = (runtime.turn_remaining - dt).max(0.0);
        return;
    }
    if !animation.set_walking(true) {
        runtime.cancel(&mut animation);
        return;
    }
    let next_x = runtime.next_step(local_x, intent.target_x, intent.speed, intent.scale, dt);
    if !physics.walk_horizontal(surface.origin_x.saturating_add(next_x), &mut environment) {
        runtime.cancel(&mut animation);
        return;
    }
    runtime.expected_x = Some(next_x);
    if next_x == intent.target_x {
        let edge = !intent.return_trip && (next_x == intent.bounds.0 || next_x == intent.bounds.1);
        runtime.finish(
            &mut animation,
            if edge {
                WalkOutcome::Edge
            } else {
                WalkOutcome::Arrived
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    fn intent() -> WalkIntent {
        WalkIntent {
            surface: WalkingSurfaceKey::Monitor(NativeMonitorHandle::from_raw(1)),
            monitor: NativeMonitorHandle::from_raw(1),
            target_x: 200,
            bounds: (0, 600),
            speed: 90.0,
            scale: 1.0,
            return_trip: false,
        }
    }
    #[test]
    fn arrival_preserves_last_committed_position_until_the_brain_observes_it() {
        let mut runtime = WalkingRuntime::default();
        let mut animation = NeraAnimationState::default();
        runtime.begin(intent(), 100, 0.3, &mut animation, Duration::from_secs(4));
        runtime.expected_x = Some(200);
        runtime.finish(&mut animation, WalkOutcome::Arrived);
        assert_eq!(runtime.expected_x(), Some(200));
        assert_eq!(runtime.take_outcome(), Some(WalkOutcome::Arrived));
        assert_eq!(runtime.expected_x(), None);
    }
    #[test]
    fn cancellation_invalidates_completed_walks_without_losing_the_final_position() {
        let mut runtime = WalkingRuntime::default();
        let mut animation = NeraAnimationState::default();
        runtime.cancel(&mut animation);
        assert_eq!(runtime.take_outcome(), None);
        for outcome in [WalkOutcome::Edge, WalkOutcome::Arrived] {
            runtime.begin(intent(), 100, 0.3, &mut animation, Duration::from_secs(4));
            runtime.expected_x = Some(200);
            runtime.finish(&mut animation, outcome);
            assert!(!runtime.active());

            // Hover/drag/hide may arrive before the brain consumes completion.
            // Repeated blocked frames must never uncover the old edge return.
            runtime.cancel(&mut animation);
            runtime.cancel(&mut animation);
            assert_eq!(runtime.expected_x(), Some(200));
            assert_eq!(runtime.take_outcome(), Some(WalkOutcome::Interrupted));
            assert_eq!(runtime.take_outcome(), None);
            assert_eq!(runtime.expected_x(), None);
            assert_eq!(animation.visual_state(), VisualState::Idle);
        }
    }
    #[test]
    fn lookaround_finishes_before_turn_and_cancels_with_the_walk() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<NeraAnimationState>()
            .add_systems(Update, crate::animation::tick_animation_actions);
        let mut runtime = WalkingRuntime::default();
        assert!(runtime.begin(
            intent(),
            100,
            0.3,
            &mut app.world_mut().resource_mut::<NeraAnimationState>(),
            Duration::from_secs(4)
        ));
        assert_eq!(runtime.direction(), 0.0);
        assert!(!runtime.ready_to_turn(&mut app.world_mut().resource_mut::<NeraAnimationState>()));
        assert_eq!(runtime.turn_remaining, 0.3);
        assert_eq!(runtime.expected_x(), Some(100));
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs(3));
        app.update();
        assert!(!runtime.ready_to_turn(&mut app.world_mut().resource_mut::<NeraAnimationState>()));
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs(1));
        app.update();
        assert!(runtime.ready_to_turn(&mut app.world_mut().resource_mut::<NeraAnimationState>()));
        assert_eq!(runtime.direction(), 1.0);
        assert_eq!(runtime.turn_remaining, 0.3);
        runtime.cancel(&mut app.world_mut().resource_mut::<NeraAnimationState>());

        // Every new trip, including the return from an edge, starts with its own look.
        let return_trip = WalkIntent {
            return_trip: true,
            ..intent()
        };
        assert!(runtime.begin(
            return_trip,
            100,
            0.3,
            &mut app.world_mut().resource_mut::<NeraAnimationState>(),
            Duration::from_secs(4)
        ));
        runtime.cancel(&mut app.world_mut().resource_mut::<NeraAnimationState>());
        assert!(!runtime.active());
        assert_eq!(runtime.direction(), 0.0);
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::Idle
        );
        assert_eq!(runtime.take_outcome(), Some(WalkOutcome::Interrupted));
    }
    #[test]
    fn walking_waits_for_a_usable_lookaround_and_drops_it_on_other_animations() {
        let mut runtime = WalkingRuntime::default();
        let mut animation = NeraAnimationState::default();
        assert!(!runtime.begin(intent(), 100, 0.3, &mut animation, Duration::ZERO));
        assert!(!runtime.active());
        assert!(runtime.begin(intent(), 100, 0.3, &mut animation, Duration::from_secs(4)));
        animation.set_physics_falling(true);
        assert!(!runtime.ready_to_turn(&mut animation));
        assert!(!runtime.active());
        assert_eq!(animation.visual_state(), VisualState::Falling);
        assert_eq!(runtime.take_outcome(), Some(WalkOutcome::Interrupted));
    }
    #[test]
    fn moved_surface_carries_destination_but_resize_or_new_monitor_rejects_it() {
        let intent = intent();
        let surface = WalkingSurface {
            key: intent.surface,
            origin_x: -1920,
            bounds: (-1912, -608),
        };
        assert!(valid_destination(intent, surface));
        assert!(valid_destination(
            intent,
            WalkingSurface {
                origin_x: -1800,
                bounds: (-1792, -488),
                ..surface
            }
        ));
        assert!(!valid_destination(
            intent,
            WalkingSurface {
                bounds: (-1912, -1800),
                ..surface
            }
        ));
        assert!(!valid_destination(
            intent,
            WalkingSurface {
                key: WalkingSurfaceKey::Monitor(NativeMonitorHandle::from_raw(2)),
                ..surface
            }
        ));
    }
    #[test]
    fn turn_faces_travel_direction_before_gait_and_restores_original_rotation() {
        use crate::avatar::{NeraAvatar, apply_walking_pose};
        use bevy_vrm1::prelude::{HipsBoneEntity, RestTransform};
        let mut app = App::new();
        app.init_resource::<NeraAnimationState>()
            .init_resource::<WalkingRuntime>()
            .add_systems(Update, apply_walking_pose);
        let hips = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.1, 0.94, 1.88),
                RestTransform(Transform::from_xyz(0.0, 0.9, 0.0)),
            ))
            .id();
        let avatar = app
            .world_mut()
            .spawn((NeraAvatar, Transform::IDENTITY, HipsBoneEntity(hips)))
            .id();
        let mut animation = NeraAnimationState::default();
        app.world_mut().resource_mut::<WalkingRuntime>().begin(
            intent(),
            100,
            0.3,
            &mut animation,
            Duration::from_secs(4),
        );
        app.insert_resource(animation);
        app.update();
        assert_eq!(
            app.world().get::<Transform>(avatar).unwrap().rotation,
            Quat::IDENTITY
        );
        // Advance the real animation lifetime before facing the destination.
        app.init_resource::<Time>();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs(4));
        app.world_mut()
            .run_system_once(crate::animation::tick_animation_actions)
            .unwrap();
        app.world_mut()
            .run_system_once(
                |mut runtime: ResMut<WalkingRuntime>, mut animation: ResMut<NeraAnimationState>| {
                    assert!(runtime.ready_to_turn(&mut animation));
                },
            )
            .unwrap();
        app.update();
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::Idle
        );
        assert!(
            (app.world().get::<Transform>(avatar).unwrap().rotation * Vec3::Z - Vec3::X).length()
                < 0.0001
        );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .set_walking(true);
        app.update();
        assert_eq!(
            app.world().get::<Transform>(hips).unwrap().translation,
            Vec3::new(0.0, 0.94, 0.0)
        );
        app.world_mut().resource_mut::<WalkingRuntime>().direction = -1.0;
        app.update();
        assert!(
            (app.world().get::<Transform>(avatar).unwrap().rotation * Vec3::Z + Vec3::X).length()
                < 0.0001
        );
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .set_walking(false);
        app.world_mut().resource_mut::<WalkingRuntime>().direction = 0.0;
        app.update();
        assert_eq!(
            app.world().get::<Transform>(avatar).unwrap().rotation,
            Quat::IDENTITY
        );
    }
    #[test]
    fn acceleration_braking_and_subpixels_reach_both_targets_without_overshoot() {
        for target in [-317, 317] {
            for dt in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 144.0] {
                let mut runtime = WalkingRuntime::default();
                let mut left = 0;
                let mut peak: f32 = 0.0;
                for _ in 0..1800 {
                    let next = runtime.next_step(left, target, 90.0, 1.0, dt);
                    assert!(next.abs_diff(target) <= left.abs_diff(target));
                    peak = peak.max(runtime.velocity);
                    left = next;
                    if left == target {
                        break;
                    }
                }
                assert_eq!(left, target);
                assert!(peak >= 89.0);
                assert!(runtime.velocity < peak / 2.0);
            }
        }
    }
    #[test]
    fn stalls_cannot_jump_and_interruption_drops_motion() {
        let mut runtime = WalkingRuntime::default();
        assert!(runtime.next_step(0, 400, 90.0, 1.0, 60.0) <= 1);
        let mut animation = NeraAnimationState::default();
        animation.set_walking(true);
        runtime.finish(&mut animation, WalkOutcome::Interrupted);
        assert!(!runtime.active());
        assert_eq!(runtime.direction(), 0.0);
        assert_eq!(runtime.velocity, 0.0);
        assert_eq!(animation.visual_state(), VisualState::Idle);
    }
}
