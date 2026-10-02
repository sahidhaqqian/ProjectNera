//! Central low-priority pet brain. One decision owns idle, fidgets and walking;
//! user placement and desktop support exclusively own sitting/standing changes.
use crate::{
    animation::{
        BasePose, IdleMicroAction, LazyNeraVrma, LazyNeraVrmaRequested, NeraAnimationClips,
        NeraAnimationState, VisualState, request_lazy_vrma,
    },
    desktop_physics::{DesktopPhysicsState, WalkingSurface, WalkingSurfaceKey},
    environment::{DesktopEnvironment, PlacementState},
    interaction::{DesktopPetInteraction, InteractionState, NeraReleased},
    platform::{self, NativeMonitorHandle, PhysicalRect},
    presentation::PresentationState,
    rng::NeraRng,
    settings::{ActivityLevel, NeraSettings, SettingsInputGate},
    startup_entrance::StartupEntrance,
    walking::{WalkIntent, WalkOutcome, WalkingRuntime},
};
use bevy::{ecs::system::SystemParam, prelude::*, window::PrimaryWindow};
use std::collections::VecDeque;

const RESUME_DELAY: f32 = 6.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Behavior {
    StayIdle,
    Variation(IdleMicroAction),
    Walk,
}

#[derive(Clone, Copy, Debug)]
struct HomeAnchor {
    surface: WalkingSurfaceKey,
    monitor: NativeMonitorHandle,
    x: i32,
    expected_x: i32,
    width: i32,
    scale: f32,
}

#[derive(Resource)]
pub(crate) struct PetBehavior {
    home: Option<HomeAnchor>,
    history: VecDeque<Behavior>,
    decision_in: f32,
    walk_in: f32,
    blocked: bool,
    edge_return: bool,
    preferences: Option<(bool, ActivityLevel, u16)>,
    fullscreen: bool,
    fullscreen_in: f32,
    fullscreen_monitor: Option<NativeMonitorHandle>,
    rng: NeraRng,
}

impl Default for PetBehavior {
    fn default() -> Self {
        Self {
            home: None,
            history: VecDeque::with_capacity(3),
            decision_in: RESUME_DELAY,
            walk_in: 0.0,
            blocked: true,
            edge_return: false,
            preferences: None,
            fullscreen: false,
            fullscreen_in: 0.0,
            fullscreen_monitor: None,
            rng: NeraRng::from_entropy(),
        }
    }
}

impl PetBehavior {
    fn observe_surface(
        &mut self,
        surface: WalkingSurface,
        rect: PhysicalRect,
        monitor: NativeMonitorHandle,
        scale: f32,
        walking: &WalkingRuntime,
    ) -> bool {
        let x = rect.left.saturating_sub(surface.origin_x);
        let expected = walking.expected_x().or(self.home.map(|h| h.expected_x));
        let changed = self.home.is_none_or(|h| {
            h.surface != surface.key
                || h.monitor != monitor
                || h.width != rect.width()
                || (h.scale - scale).abs() > 0.001
                || expected != Some(x)
        });
        if changed {
            self.edge_return = false;
            self.home = Some(HomeAnchor {
                surface: surface.key,
                monitor,
                x,
                expected_x: x,
                width: rect.width(),
                scale,
            });
            self.decision_in = RESUME_DELAY;
        } else if let Some(home) = &mut self.home {
            home.expected_x = x;
        }
        changed
    }
    fn remember(&mut self, behavior: Behavior) {
        if self.history.len() == 3 {
            self.history.pop_front();
        }
        self.history.push_back(behavior);
    }
    fn walk_delay(&mut self, activity: ActivityLevel) -> f32 {
        let (min, max) = activity.walk_interval_secs();
        self.rng.range_f32(min, max)
    }
    fn interrupt(&mut self, walking: &mut WalkingRuntime, animation: &mut NeraAnimationState) {
        walking.cancel(animation);
        animation.cancel_idle_variation();
        self.edge_return = false;
        self.decision_in = RESUME_DELAY;
        self.blocked = true;
    }
    fn choose(&mut self, candidates: &[(Behavior, u32)]) -> Behavior {
        // Remove recent choices if alternatives exist; never spin rerolling a
        // small/missing asset set. StayIdle is always a safe fallback.
        let fresh: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|(b, _)| !self.history.contains(b))
            .collect();
        let pool = if fresh.is_empty() { candidates } else { &fresh };
        let total: u32 = pool.iter().map(|(_, w)| w).sum();
        if total == 0 {
            return Behavior::StayIdle;
        }
        let mut roll = (self.rng.next_f32() * total as f32) as u32;
        for &(behavior, weight) in pool {
            if roll < weight {
                return behavior;
            }
            roll -= weight;
        }
        Behavior::StayIdle
    }

    fn tick_seated_idle(
        &mut self,
        dt: f32,
        clips: &NeraAnimationClips,
        animation: &mut NeraAnimationState,
    ) {
        if animation.visual_state() != VisualState::SittingLoop {
            return;
        }
        self.decision_in = (self.decision_in - dt).max(0.0);
        if self.decision_in > 0.0 {
            return;
        }
        let behavior = self.choose(&idle_candidates(
            IdleMicroAction::for_pose(BasePose::Sitting),
            clips,
        ));
        if let Behavior::Variation(action) = behavior
            && let Some(duration) = clips.idle_micro_duration(action)
        {
            animation.request_idle_variation(action, duration);
        }
        self.decision_in = self.rng.range_f32(8.0, 30.0);
        self.remember(behavior);
    }
}

fn idle_candidates(
    actions: &[IdleMicroAction],
    clips: &NeraAnimationClips,
) -> Vec<(Behavior, u32)> {
    let mut candidates = vec![(Behavior::StayIdle, 3)];
    for &action in actions {
        if clips.idle_micro_duration(action).is_some() {
            candidates.push((Behavior::Variation(action), 2));
        }
    }
    candidates
}

#[derive(SystemParam)]
pub(crate) struct BehaviorContext<'w, 's> {
    settings: Res<'w, NeraSettings>,
    interaction: Res<'w, DesktopPetInteraction>,
    presentation: Res<'w, PresentationState>,
    entrance: Res<'w, StartupEntrance>,
    settings_gate: Res<'w, SettingsInputGate>,
    #[cfg(target_os = "windows")]
    alarm_gate: Res<'w, crate::alarm_settings::AlarmSettingsInputGate>,
    #[cfg(target_os = "windows")]
    manager_gate: Res<'w, crate::animation_manager::AnimationManagerInputGate>,
    #[cfg(target_os = "windows")]
    menu: Option<Res<'w, crate::action_menu::ActionMenuState>>,
    primary: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
}

impl BehaviorContext<'_, '_> {
    fn available(&self) -> bool {
        #[cfg(target_os = "windows")]
        if !crate::alarm_settings::nera_input_available(Res::clone(&self.alarm_gate))
            || self.manager_gate.window_open
            || self.menu.as_ref().is_some_and(|menu| menu.is_open())
        {
            return false;
        }
        self.presentation.is_desktop_pet()
            && !self.entrance.blocks_position_save()
            && crate::settings::nera_input_available(Res::clone(&self.settings_gate))
            && self.primary.iter().any(|w| w.visible)
            && self.interaction.state == InteractionState::None
            && self.interaction.right_pressed_body_part.is_none()
    }
}

fn roaming_bounds(home: i32, radius: i32, surface: WalkingSurface) -> Option<(i32, i32)> {
    // A dragged-offscreen home or resized monitor still needs a reachable
    // recovery destination. Clamp the planning anchor, never the actual HWND.
    let home = home.clamp(
        surface.bounds.0.saturating_sub(surface.origin_x),
        surface.bounds.1.saturating_sub(surface.origin_x),
    );
    let min = surface
        .bounds
        .0
        .saturating_sub(surface.origin_x)
        .max(home.saturating_sub(radius));
    let max = surface
        .bounds
        .1
        .saturating_sub(surface.origin_x)
        .min(home.saturating_add(radius));
    (min < max).then_some((min, max))
}

fn choose_target(x: i32, bounds: (i32, i32), distance: i32, prefer_right: bool) -> i32 {
    let (min, max) = bounds;
    if x <= min {
        return x.saturating_add(distance).clamp(min, max);
    }
    if x >= max {
        return x.saturating_sub(distance).clamp(min, max);
    }
    let right_room = max.saturating_sub(x);
    let left_room = x.saturating_sub(min);
    let right = if prefer_right {
        right_room >= distance || right_room >= left_room
    } else {
        left_room < distance && right_room > left_room
    };
    if right {
        x.saturating_add(distance).min(max)
    } else {
        x.saturating_sub(distance).max(min)
    }
}

pub(crate) fn update_pet_behavior(
    time: Res<Time>,
    real_time: Res<Time<Real>>,
    context: BehaviorContext,
    clips: Res<NeraAnimationClips>,
    physics: Res<DesktopPhysicsState>,
    mut released: MessageReader<NeraReleased>,
    environment: Res<DesktopEnvironment>,
    mut animation: ResMut<NeraAnimationState>,
    mut walking: ResMut<WalkingRuntime>,
    mut brain: ResMut<PetBehavior>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    lazy_loaders: Query<(Entity, &LazyNeraVrma, Option<&LazyNeraVrmaRequested>)>,
) {
    let dt = time.delta_secs().min(0.05);
    let settings = &context.settings;
    if released.read().any(|r| r.was_dragging)
        || context.interaction.state == InteractionState::Dragging
    {
        brain.home = None;
    }
    let prefs = (
        settings.walking_enabled,
        settings.activity_level(),
        settings.roaming_radius_px,
    );
    if brain.preferences != Some(prefs) {
        walking.cancel(&mut animation);
        brain.edge_return = false;
        brain.walk_in = brain.walk_delay(prefs.1);
        brain.decision_in = RESUME_DELAY;
        brain.preferences = Some(prefs);
    }
    // Native fullscreen detection is sampled at 4 Hz, independent of rendering.
    let monitor = environment.monitor.map(|m| m.handle);
    brain.fullscreen_in -= real_time.delta_secs().min(0.5);
    if monitor != brain.fullscreen_monitor || brain.fullscreen_in <= 0.0 {
        brain.fullscreen_monitor = monitor;
        brain.fullscreen_in = 0.25;
        brain.fullscreen = match (environment.nera_hwnd, environment.monitor) {
            (Some(hwnd), Some(monitor)) if settings.pause_roaming_fullscreen => {
                platform::foreground_fullscreen_on_monitor(hwnd, monitor)
            }
            _ => false,
        };
    }
    let native_visible = environment.nera_hwnd.is_some_and(|h| {
        platform::window_is_visible(h)
            && !platform::is_window_minimized(h)
            && !platform::is_window_cloaked(h)
    });
    if !context.available()
        || !native_visible
        || environment.hidden_for_target_suspend
        || real_time.delta_secs() >= 0.5
        || (settings.pause_roaming_fullscreen && brain.fullscreen)
    {
        brain.interrupt(&mut walking, &mut animation);
        return;
    }
    if brain.blocked {
        brain.blocked = false;
        brain.walk_in = brain.walk_delay(settings.activity_level());
    }
    if physics.falling {
        brain.interrupt(&mut walking, &mut animation);
        return;
    }
    if environment.placement != PlacementState::Free {
        walking.cancel(&mut animation);
        // Roaming never detaches a seat. Only pose-matched fidgets are eligible,
        // including when Stay Here is enabled; native placement remains untouched.
        match animation.visual_state() {
            VisualState::SittingLoop => brain.tick_seated_idle(dt, &clips, &mut animation),
            VisualState::IdleMicro(action) if action.base_pose() == BasePose::Sitting => {}
            _ => brain.interrupt(&mut walking, &mut animation),
        }
        return;
    }
    if !matches!(
        animation.visual_state(),
        VisualState::Idle | VisualState::Walking | VisualState::IdleMicro(_)
    ) {
        brain.interrupt(&mut walking, &mut animation);
        return;
    }
    let surface = physics.walking_surface(&environment);
    let scale = environment.nera_scale_factor.clamp(0.5, 4.0) as f32 * settings.character_scale();
    if let (Some(surface), Some(rect), Some(monitor)) = (surface, environment.nera_rect, monitor) {
        if brain.observe_surface(surface, rect, monitor, scale, &walking) {
            walking.cancel(&mut animation);
        }
    } else {
        walking.cancel(&mut animation);
    }
    if let Some(outcome) = walking.take_outcome() {
        brain.edge_return = outcome == WalkOutcome::Edge && settings.walking_enabled;
        brain.decision_in = if brain.edge_return {
            brain.rng.range_f32(0.6, 1.1)
        } else {
            brain.rng.range_f32(8.0, 30.0)
        };
        brain.walk_in = brain.walk_delay(settings.activity_level());
    }
    if walking.active() {
        return;
    }
    brain.walk_in = (brain.walk_in - dt).max(0.0);
    if animation.visual_state() != VisualState::Idle {
        return;
    }
    brain.decision_in = (brain.decision_in - dt).max(0.0);
    if brain.decision_in > 0.0 {
        return;
    }
    let bounds = brain.home.zip(surface).and_then(|(h, s)| {
        roaming_bounds(
            h.x,
            (settings.roaming_radius_px() * environment.nera_scale_factor as f32).round() as i32,
            s,
        )
    });
    let can_walk = settings.walking_enabled
        && clips.walking_ready()
        && clips
            .idle_micro_duration(IdleMicroAction::LookAround)
            .is_some()
        && bounds.is_some()
        && surface.is_some()
        && (brain.walk_in <= 0.0 || brain.edge_return);
    let mut candidates = idle_candidates(IdleMicroAction::for_pose(BasePose::Idle), &clips);
    // Cold idle fidgets are still eligible, but selecting one for the first
    // time merely requests its VRMA. The pet stays in Idle for that decision
    // and the clip joins the normal candidate pool once retargeting completes.
    for action in IdleMicroAction::for_pose(BasePose::Idle) {
        if clips.idle_micro_duration(*action).is_none()
            && LazyNeraVrma::for_idle_micro(*action).is_some()
        {
            candidates.push((Behavior::Variation(*action), 1));
        }
    }
    if can_walk {
        candidates.push((Behavior::Walk, 8));
    }
    let behavior = if brain.edge_return && can_walk {
        Behavior::Walk
    } else {
        brain.choose(&candidates)
    };
    brain.decision_in = brain.rng.range_f32(8.0, 30.0);
    match behavior {
        Behavior::Walk => {
            if let (Some(surface), Some(bounds), Some(home), Some(rect)) =
                (surface, bounds, brain.home, environment.nera_rect)
            {
                let x = rect.left.saturating_sub(surface.origin_x);
                let distance = (brain.rng.range_f32(120.0, 500.0) * scale).round() as i32;
                let prefer_right = brain.rng.next_f32() >= 0.5;
                let target = choose_target(x, bounds, distance, prefer_right);
                if target != x {
                    let speed = brain.rng.range_f32(70.0, 110.0) * scale;
                    let turn = brain.rng.range_f32(0.2, 0.4);
                    walking.begin(
                        WalkIntent {
                            surface: surface.key,
                            monitor: home.monitor,
                            target_x: target,
                            bounds,
                            speed,
                            scale,
                            return_trip: brain.edge_return,
                        },
                        x,
                        turn,
                        &mut animation,
                        clips
                            .idle_micro_duration(IdleMicroAction::LookAround)
                            .unwrap_or_default(),
                    );
                }
            }
            brain.edge_return = false;
        }
        Behavior::Variation(action) => {
            if let Some(duration) = clips.idle_micro_duration(action) {
                animation.request_idle_variation(action, duration);
            } else if let Some(kind) = LazyNeraVrma::for_idle_micro(action) {
                request_lazy_vrma(kind, &mut commands, &asset_server, &lazy_loaders);
            }
        }
        Behavior::StayIdle => {}
    }
    brain.remember(behavior);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::{AvatarAnchor, TaskbarEdge};
    fn surface(origin: i32) -> WalkingSurface {
        WalkingSurface {
            key: WalkingSurfaceKey::Window(crate::platform::NativeWindowHandle::from_raw(7)),
            bounds: (origin + 8, origin + 1000),
            origin_x: origin,
        }
    }
    fn rect(left: i32) -> PhysicalRect {
        PhysicalRect {
            left,
            top: 10,
            right: left + 600,
            bottom: 960,
        }
    }
    #[test]
    fn home_follows_support_origin_but_changes_for_user_moves_monitor_and_dpi() {
        let monitor = NativeMonitorHandle::from_raw(1);
        let mut brain = PetBehavior::default();
        let walking = WalkingRuntime::default();
        assert!(brain.observe_surface(surface(-1900), rect(-1500), monitor, 1.0, &walking));
        assert_eq!(brain.home.unwrap().x, 400);
        assert!(!brain.observe_surface(surface(-1700), rect(-1300), monitor, 1.0, &walking));
        assert_eq!(brain.home.unwrap().x, 400);
        assert!(brain.observe_surface(surface(-1700), rect(-1200), monitor, 1.0, &walking));
        assert_eq!(brain.home.unwrap().x, 500);
        let second = NativeMonitorHandle::from_raw(2);
        assert!(brain.observe_surface(surface(100), rect(600), second, 1.25, &walking));
        assert_eq!(brain.home.unwrap().monitor, second);
        assert_eq!(brain.home.unwrap().scale, 1.25);
    }
    #[test]
    fn radius_clips_to_surface_and_offscreen_home_still_has_inward_destination() {
        let surface = surface(-1920);
        assert_eq!(roaming_bounds(500, 400, surface), Some((100, 900)));
        assert_eq!(roaming_bounds(500, 600, surface), Some((8, 1000)));
        let recovery = roaming_bounds(-500, 400, surface).unwrap();
        assert_eq!(recovery, (8, 408));
        assert!(choose_target(-500, recovery, 120, false) > -500);
    }
    fn behavior_test_app() -> App {
        let mut app = App::new();
        app.add_plugins((bevy::app::TaskPoolPlugin::default(), bevy::asset::AssetPlugin::default()));
        app.init_resource::<Time>()
            .init_resource::<Time<Real>>()
            .init_resource::<NeraSettings>()
            .init_resource::<DesktopPetInteraction>()
            .init_resource::<PresentationState>()
            .init_resource::<StartupEntrance>()
            .init_resource::<SettingsInputGate>()
            .init_resource::<NeraAnimationClips>()
            .init_resource::<DesktopPhysicsState>()
            .init_resource::<DesktopEnvironment>()
            .init_resource::<NeraAnimationState>()
            .init_resource::<WalkingRuntime>()
            .init_resource::<PetBehavior>()
            .add_message::<NeraReleased>()
            .add_systems(
                Update,
                (
                    update_pet_behavior,
                    crate::animation::sync_environment_animation_state,
                    crate::walking::update_walking,
                )
                    .chain(),
            );
        #[cfg(target_os = "windows")]
        app.init_resource::<crate::alarm_settings::AlarmSettingsInputGate>()
            .init_resource::<crate::animation_manager::AnimationManagerInputGate>();
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app
    }
    #[test]
    fn roaming_preference_changes_leave_user_seat_attached_even_when_context_is_blocked() {
        let mut app = behavior_test_app();
        let monitor = NativeMonitorHandle::from_raw(1);
        let placement = PlacementState::PendingTaskbar {
            monitor,
            edge: TaskbarEdge::Bottom,
            settle_frames: 1,
        };
        {
            let mut environment = app.world_mut().resource_mut::<DesktopEnvironment>();
            environment.placement = placement;
            environment.attached_anchor = AvatarAnchor::Seat;
        }
        app.world_mut().resource_mut::<PetBehavior>().preferences =
            Some((true, ActivityLevel::Normal, 400));
        {
            let mut settings = app.world_mut().resource_mut::<NeraSettings>();
            settings.walking_enabled = true;
            settings.roaming_radius_px = 500;
        }
        app.update();
        assert_eq!(
            app.world().resource::<DesktopEnvironment>().placement,
            placement
        );

        // Stay Here and the Settings checkbox both turn this preference off.
        app.world_mut()
            .resource_mut::<NeraSettings>()
            .walking_enabled = false;
        app.update();
        assert_eq!(
            app.world().resource::<DesktopEnvironment>().placement,
            placement
        );
        assert_eq!(
            app.world().resource::<DesktopEnvironment>().attached_anchor,
            AvatarAnchor::Seat
        );
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::SittingLoop
        );

        // Re-enabling roaming must leave an existing seat alone too.
        app.world_mut()
            .resource_mut::<NeraSettings>()
            .walking_enabled = true;
        app.update();
        assert_eq!(
            app.world().resource::<DesktopEnvironment>().placement,
            placement
        );
    }
    #[test]
    fn blocked_context_invalidates_an_unconsumed_edge_result() {
        let mut app = behavior_test_app();
        app.world_mut()
            .resource_mut::<NeraSettings>()
            .walking_enabled = true;
        app.world_mut().resource_mut::<PetBehavior>().preferences =
            Some((true, ActivityLevel::Normal, 400));
        app.world_mut().resource_mut::<WalkingRuntime>().outcome = Some(WalkOutcome::Edge);
        app.update();
        app.update();
        let brain = app.world().resource::<PetBehavior>();
        assert!(!brain.edge_return);
        assert_eq!(brain.decision_in, RESUME_DELAY);
        assert_eq!(
            app.world_mut()
                .resource_mut::<WalkingRuntime>()
                .take_outcome(),
            Some(WalkOutcome::Interrupted)
        );
    }
    #[test]
    fn unified_schedule_cancels_a_stale_walk_even_during_startup_or_hidden_context() {
        let mut app = behavior_test_app();
        let mut animation = NeraAnimationState::default();
        app.world_mut().resource_mut::<WalkingRuntime>().begin(
            WalkIntent {
                surface: surface(0).key,
                monitor: NativeMonitorHandle::from_raw(1),
                target_x: 200,
                bounds: (0, 600),
                speed: 90.0,
                scale: 1.0,
                return_trip: false,
            },
            100,
            0.3,
            &mut animation,
            std::time::Duration::from_secs(4),
        );
        app.insert_resource(animation);
        app.update();
        assert!(!app.world().resource::<WalkingRuntime>().active());
        assert_eq!(
            app.world().resource::<NeraAnimationState>().visual_state(),
            VisualState::Idle
        );
    }
    #[test]
    fn seeded_decisions_avoid_recent_behaviors_and_missing_assets_do_not_stall() {
        let mut a = PetBehavior {
            rng: NeraRng::seeded(44),
            ..default()
        };
        let mut b = PetBehavior {
            rng: NeraRng::seeded(44),
            ..default()
        };
        let candidates = [
            (Behavior::StayIdle, 3),
            (Behavior::Walk, 8),
            (Behavior::Variation(IdleMicroAction::Sleepy), 2),
            (Behavior::Variation(IdleMicroAction::Relax), 2),
        ];
        for _ in 0..100 {
            let chosen = a.choose(&candidates);
            assert_eq!(chosen, b.choose(&candidates));
            assert!(!a.history.contains(&chosen));
            a.remember(chosen);
            b.remember(chosen);
        }
        assert_eq!(a.choose(&[(Behavior::StayIdle, 1)]), Behavior::StayIdle);
        assert_eq!(a.choose(&[]), Behavior::StayIdle);
    }
    #[test]
    fn activity_cooldowns_respect_profiles() {
        let mut brain = PetBehavior {
            rng: NeraRng::seeded(9),
            ..default()
        };
        for activity in [
            ActivityLevel::Calm,
            ActivityLevel::Normal,
            ActivityLevel::Active,
        ] {
            let (min, max) = activity.walk_interval_secs();
            for _ in 0..200 {
                assert!((min..max).contains(&brain.walk_delay(activity)));
            }
        }
    }
    #[test]
    fn destinations_turn_inward_and_never_escape_negative_monitor_or_home_bounds() {
        for x in [-1700, -1500, -1100] {
            for right in [false, true] {
                let target = choose_target(x, (-1700, -1100), 240, right);
                assert!((-1700..=-1100).contains(&target));
                assert_ne!(target, x);
            }
        }
        assert!(choose_target(-1700, (-1700, -1100), 240, false) > -1700);
        assert!(choose_target(-1100, (-1700, -1100), 240, true) < -1100);
    }
    #[test]
    fn interruptions_cancel_fidgets_and_reset_decision_without_offline_catchup() {
        let mut brain = PetBehavior::default();
        let mut walking = WalkingRuntime::default();
        let mut animation = NeraAnimationState::default();
        animation
            .request_idle_variation(IdleMicroAction::Sleepy, std::time::Duration::from_secs(3));
        brain.edge_return = true;
        brain.interrupt(&mut walking, &mut animation);
        assert_eq!(animation.visual_state(), VisualState::Idle);
        assert!(brain.blocked);
        assert!(!brain.edge_return);
        assert_eq!(brain.decision_in, RESUME_DELAY);
    }

    #[test]
    fn seated_idle_keeps_its_pose_across_many_decisions_and_interruption() {
        use bevy_vrm1::prelude::{LoadedVrma, VrmaDuration};
        use std::time::Duration;
        let mut world = World::new();
        world.init_resource::<NeraAnimationClips>();
        let vrm = world.spawn_empty().id();
        let seated = world
            .spawn(VrmaDuration(Duration::from_secs(2)))
            .observe(crate::animation::register_idle_sitting_rubbing_arm_animation)
            .id();
        let standing = world
            .spawn(VrmaDuration(Duration::from_secs(10)))
            .observe(crate::animation::register_idle_bored_animation)
            .id();
        world.trigger(LoadedVrma { vrm, vrma: seated });
        world.trigger(LoadedVrma {
            vrm,
            vrma: standing,
        });
        let clips = world.resource::<NeraAnimationClips>();
        let mut brain = PetBehavior::default();
        let mut animation = NeraAnimationState::default();
        animation.restore_support_after_attention(true);
        brain.remember(Behavior::StayIdle);
        brain.decision_in = 0.0;
        brain.tick_seated_idle(0.05, clips, &mut animation);
        assert_eq!(
            animation.visual_state(),
            VisualState::IdleMicro(IdleMicroAction::SittingRubbingArm)
        );
        let cooldown = brain.decision_in;
        brain.tick_seated_idle(0.05, clips, &mut animation);
        assert_eq!(brain.decision_in, cooldown);
        brain.interrupt(&mut WalkingRuntime::default(), &mut animation);
        assert_eq!(animation.visual_state(), VisualState::SittingLoop);

        // Run the actual seated-decision and animation-lifetime code well past
        // the former 12–30 second automatic stand-up interval, without a HWND.
        let mut app = App::new();
        app.init_resource::<Time>()
            .insert_resource(animation)
            .add_systems(Update, crate::animation::tick_animation_actions);
        for _ in 0..2400 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(Duration::from_millis(50));
            brain.tick_seated_idle(
                0.05,
                clips,
                &mut app.world_mut().resource_mut::<NeraAnimationState>(),
            );
            app.update();
            let state = app.world().resource::<NeraAnimationState>();
            assert_eq!(state.base_pose(), BasePose::Sitting);
            assert!(matches!(
                state.visual_state(),
                VisualState::SittingLoop
                    | VisualState::IdleMicro(IdleMicroAction::SittingRubbingArm)
            ));
        }
        let mut animation = app
            .world_mut()
            .remove_resource::<NeraAnimationState>()
            .unwrap();
        animation.cancel_idle_variation();

        let missing = NeraAnimationClips::default();
        brain.decision_in = 0.0;
        brain.tick_seated_idle(0.05, &missing, &mut animation);
        assert_eq!(animation.visual_state(), VisualState::SittingLoop);
        assert!(brain.decision_in >= 8.0);
    }
}
