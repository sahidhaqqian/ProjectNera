//! Small opt-in desktop-floor walking controller. Animation owns the pose;
//! desktop physics owns native translation. No library scanning or clip cache.
use bevy::{ecs::system::SystemParam, prelude::*, window::PrimaryWindow};

use crate::{
    animation::{NeraAnimationClips, NeraAnimationState, VisualState},
    desktop_physics::DesktopPhysicsState,
    environment::{DesktopEnvironment, PlacementState},
    interaction::{DesktopPetInteraction, InteractionState},
    platform::{self, NativeMonitorHandle, PhysicalRect},
    presentation::PresentationState,
    rng::NeraRng,
    settings::{NeraSettings, SettingsInputGate},
};

const RESUME_DELAY: f32 = 6.0;
const SPEED_LOGICAL: f32 = 90.0;

#[derive(Resource)]
pub(crate) struct WalkingRuntime {
    target: Option<i32>,
    direction: f32,
    cooldown: f32,
    fraction: f32,
    expected_left: Option<i32>,
    monitor: Option<NativeMonitorHandle>,
    bounds: Option<(i32, i32)>,
    rng: NeraRng,
}

impl Default for WalkingRuntime {
    fn default() -> Self {
        Self {
            target: None,
            direction: 0.0,
            cooldown: RESUME_DELAY,
            fraction: 0.0,
            expected_left: None,
            monitor: None,
            bounds: None,
            rng: NeraRng::from_entropy(),
        }
    }
}

impl WalkingRuntime {
    pub(crate) fn direction(&self) -> f32 {
        self.direction
    }

    fn stop(&mut self, animation: &mut NeraAnimationState, delay: f32) {
        animation.set_walking(false);
        self.target = None;
        self.direction = 0.0;
        self.fraction = 0.0;
        self.expected_left = None;
        self.cooldown = delay;
    }

    fn next_step(&mut self, left: i32, speed: f32, dt: f32) -> i32 {
        let Some(target) = self.target else {
            return left;
        };
        self.fraction += speed * dt.clamp(0.0, 0.05);
        let whole = self.fraction.floor() as i32;
        self.fraction -= whole as f32;
        if target > left {
            left.saturating_add(whole).min(target)
        } else {
            left.saturating_sub(whole).max(target)
        }
    }
}

#[derive(SystemParam)]
pub(crate) struct WalkingContext<'w, 's> {
    settings: Res<'w, NeraSettings>,
    interaction: Res<'w, DesktopPetInteraction>,
    presentation: Res<'w, PresentationState>,
    settings_gate: Res<'w, SettingsInputGate>,
    #[cfg(target_os = "windows")]
    alarm_gate: Res<'w, crate::alarm_settings::AlarmSettingsInputGate>,
    primary: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
}

impl WalkingContext<'_, '_> {
    fn allows_walking(&self) -> bool {
        #[cfg(target_os = "windows")]
        if !crate::alarm_settings::nera_input_available(Res::clone(&self.alarm_gate)) {
            return false;
        }
        self.settings.walking_enabled
            && self.presentation.is_desktop_pet()
            && crate::settings::nera_input_available(Res::clone(&self.settings_gate))
            && self
                .primary
                .iter()
                .next()
                .is_some_and(|window| window.visible)
            && self.interaction.state == InteractionState::None
            && self.interaction.right_pressed_body_part.is_none()
    }
}

pub(crate) fn update_walking(
    time: Res<Time>,
    real_time: Res<Time<Real>>,
    context: WalkingContext,
    clips: Res<NeraAnimationClips>,
    physics: Res<DesktopPhysicsState>,
    mut environment: ResMut<DesktopEnvironment>,
    mut animation: ResMut<NeraAnimationState>,
    mut runtime: ResMut<WalkingRuntime>,
) {
    // Always cancel on interruptions, including sleep/resume. A run condition
    // would leave the old target and facing alive while the system is skipped.
    let ready = context.allows_walking()
        && clips.walking_ready()
        && environment.placement == PlacementState::Free
        && !environment.hidden_for_target_suspend
        && physics.walking_floor().is_some()
        && real_time.delta_secs() < 0.5
        && matches!(
            animation.visual_state(),
            VisualState::Idle | VisualState::Walking
        );
    if !ready {
        runtime.stop(&mut animation, RESUME_DELAY);
        return;
    }
    let (Some(rect), Some(monitor), Some(hwnd)) = (
        environment.nera_rect,
        environment.monitor,
        environment.nera_hwnd,
    ) else {
        runtime.stop(&mut animation, RESUME_DELAY);
        return;
    };
    let Some(bounds) = walking_bounds(monitor.work_area, rect.width()) else {
        runtime.stop(&mut animation, RESUME_DELAY);
        return;
    };
    if physics.walking_floor() != Some(monitor.handle)
        || !platform::window_is_visible(hwnd)
        || platform::is_window_minimized(hwnd)
        || platform::is_window_cloaked(hwnd)
    {
        runtime.stop(&mut animation, RESUME_DELAY);
        return;
    }
    if runtime.monitor != Some(monitor.handle)
        || runtime.bounds != Some(bounds)
        || runtime.expected_left.is_some_and(|left| left != rect.left)
        || (runtime.target.is_some() && animation.visual_state() != VisualState::Walking)
    {
        runtime.stop(&mut animation, RESUME_DELAY);
        runtime.monitor = Some(monitor.handle);
        runtime.bounds = Some(bounds);
    }

    let scale =
        environment.nera_scale_factor.clamp(0.5, 4.0) as f32 * context.settings.character_scale();
    if runtime.target.is_none() {
        runtime.cooldown = (runtime.cooldown - time.delta_secs().min(0.05)).max(0.0);
        if runtime.cooldown > 0.0 {
            return;
        }
        let distance = runtime.rng.range_f32(140.0, 240.0) * scale;
        let right = runtime.rng.next_f32() >= 0.5;
        let target = choose_target(rect.left, bounds, distance.round() as i32, right);
        if target == rect.left || !animation.set_walking(true) {
            runtime.stop(&mut animation, RESUME_DELAY);
            return;
        }
        runtime.target = Some(target);
        runtime.direction = if target > rect.left { 1.0 } else { -1.0 };
        // Let the walking pose and facing settle before translating the HWND.
        runtime.expected_left = Some(rect.left);
        return;
    }

    let left = runtime.next_step(rect.left, SPEED_LOGICAL * scale, time.delta_secs());
    if !physics.walk_horizontal(left, &mut environment) {
        runtime.stop(&mut animation, RESUME_DELAY);
        return;
    }
    runtime.expected_left = Some(left);
    if runtime.target == Some(left) {
        let delay = runtime.rng.range_f32(12.0, 25.0);
        runtime.stop(&mut animation, delay);
    }
}

// Use the full transparent canvas as a conservative visible-body bound. Never
// clamp the current position: a pet partly off-screen must walk back, not jump.
fn walking_bounds(work: PhysicalRect, width: i32) -> Option<(i32, i32)> {
    let left = work.left.saturating_add(8);
    let right = work.right.saturating_sub(width).saturating_sub(8);
    (width > 0 && right > left).then_some((left, right))
}

fn choose_target(left: i32, bounds: (i32, i32), distance: i32, prefer_right: bool) -> i32 {
    let (min, max) = bounds;
    let distance = distance.max(1);
    let to_right = if left <= min {
        true
    } else if left >= max {
        false
    } else {
        let right_room = max - left;
        let left_room = left - min;
        if prefer_right {
            right_room >= distance.min(64) || right_room >= left_room
        } else {
            left_room < distance.min(64) && right_room > left_room
        }
    };
    if to_right {
        left.saturating_add(distance).clamp(min, max)
    } else {
        left.saturating_sub(distance).clamp(min, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct Allowed(bool);

    #[test]
    fn walking_requires_opt_in_visibility_and_an_unoccupied_pointer() {
        fn sample(context: WalkingContext, mut allowed: ResMut<Allowed>) {
            allowed.0 = context.allows_walking();
        }
        let mut app = App::new();
        app.init_resource::<NeraSettings>()
            .init_resource::<DesktopPetInteraction>()
            .init_resource::<PresentationState>()
            .init_resource::<SettingsInputGate>()
            .init_resource::<Allowed>()
            .add_systems(Update, sample);
        #[cfg(target_os = "windows")]
        app.init_resource::<crate::alarm_settings::AlarmSettingsInputGate>();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.update();
        assert!(!app.world().resource::<Allowed>().0);
        app.world_mut()
            .resource_mut::<NeraSettings>()
            .walking_enabled = true;
        app.update();
        assert!(app.world().resource::<Allowed>().0);
        for state in [
            InteractionState::Hover,
            InteractionState::Pressed,
            InteractionState::Dragging,
        ] {
            app.world_mut()
                .resource_mut::<DesktopPetInteraction>()
                .state = state;
            app.update();
            assert!(!app.world().resource::<Allowed>().0);
        }
        app.world_mut()
            .resource_mut::<DesktopPetInteraction>()
            .state = InteractionState::None;
        app.world_mut().get_mut::<Window>(window).unwrap().visible = false;
        app.update();
        assert!(!app.world().resource::<Allowed>().0);
        app.world_mut().get_mut::<Window>(window).unwrap().visible = true;
        app.update();
        assert!(app.world().resource::<Allowed>().0);
    }

    #[test]
    fn avatar_faces_travel_direction_and_restores_rotation_on_stop() {
        use crate::avatar::{NeraAvatar, apply_walking_pose};
        use bevy_vrm1::prelude::{HipsBoneEntity, RestTransform};
        let mut app = App::new();
        app.init_resource::<NeraAnimationState>()
            .insert_resource(WalkingRuntime {
                direction: 1.0,
                ..default()
            })
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
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .set_walking(true);
        app.update();
        let transform = app.world().get::<Transform>(avatar).unwrap();
        assert!((transform.rotation * Vec3::Z - Vec3::X).length() < 0.0001);
        assert_eq!(
            app.world().get::<Transform>(hips).unwrap().translation,
            Vec3::new(0.0, 0.94, 0.0)
        );
        app.world_mut().resource_mut::<WalkingRuntime>().direction = -1.0;
        app.update();
        let transform = app.world().get::<Transform>(avatar).unwrap();
        assert!((transform.rotation * Vec3::Z + Vec3::X).length() < 0.0001);
        app.world_mut()
            .resource_mut::<NeraAnimationState>()
            .set_walking(false);
        app.update();
        assert_eq!(
            app.world().get::<Transform>(avatar).unwrap().rotation,
            Quat::IDENTITY
        );
    }

    #[test]
    fn targets_stay_on_negative_coordinate_monitor_and_turn_at_edges() {
        let work = PhysicalRect {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };
        let bounds = walking_bounds(work, 600).unwrap();
        for left in [bounds.0, -1500, -1000, bounds.1] {
            for right in [true, false] {
                let target = choose_target(left, bounds, 240, right);
                assert!((bounds.0..=bounds.1).contains(&target));
                assert_ne!(target, left);
            }
        }
        assert!(choose_target(bounds.0, bounds, 200, false) > bounds.0);
        assert!(choose_target(bounds.1, bounds, 200, true) < bounds.1);
        assert!(walking_bounds(work, 2000).is_none());
    }

    #[test]
    fn movement_preserves_fraction_and_never_overshoots() {
        for target in [-107, 107] {
            let mut runtime = WalkingRuntime {
                target: Some(target),
                ..default()
            };
            let mut left = 0;
            for _ in 0..240 {
                left = runtime.next_step(left, 90.0, 1.0 / 120.0);
            }
            assert_eq!(left, target);
        }
    }

    #[test]
    fn offscreen_recovery_and_stalls_do_not_teleport() {
        let target = choose_target(-500, (8, 1312), 200, false);
        let mut runtime = WalkingRuntime {
            target: Some(target),
            ..default()
        };
        assert_eq!(runtime.next_step(-500, 90.0, 60.0), -496);
    }

    #[test]
    fn interruption_clears_target_fraction_and_walking_pose() {
        let mut animation = NeraAnimationState::default();
        assert!(animation.set_walking(true));
        let mut runtime = WalkingRuntime {
            target: Some(200),
            direction: 1.0,
            fraction: 0.75,
            expected_left: Some(100),
            ..default()
        };
        runtime.stop(&mut animation, RESUME_DELAY);
        assert_eq!(animation.visual_state(), VisualState::Idle);
        assert!(runtime.target.is_none());
        assert!(runtime.expected_left.is_none());
        assert_eq!(runtime.direction(), 0.0);
        assert_eq!(runtime.fraction, 0.0);
        assert_eq!(runtime.cooldown, RESUME_DELAY);
    }
}
