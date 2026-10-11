//! Apply persisted settings to the running application and manage position persistence.
use std::time::Duration;

use bevy::{
    audio::{AudioSink, AudioSinkPlayback, GlobalVolume, Volume},
    prelude::*,
    settings::{SaveSettingsDeferred, SaveSettingsSync},
    window::{PrimaryWindow, WindowLevel, WindowMoved, WindowPosition},
    winit::{UpdateMode, WinitSettings},
};
use bevy_framepace::{FramepaceSettings, Limiter};

use super::{NeraSettings, startup};

#[derive(Resource, Default)]
pub(super) struct StartupRuntime {
    pub(super) enabled: bool,
}

/// Last values applied to global runtime controls. `NeraSettings` contains many
/// unrelated fields, so its Bevy change flag alone is too broad: moving Nera or
/// editing an alarm must not reapply and log audio/FPS settings.
#[derive(Resource, Default)]
pub(super) struct AppliedRuntimeSettings {
    audio: Option<(u8, bool)>,
    fps_limit: Option<u16>,
}

impl AppliedRuntimeSettings {
    fn take_audio_change(&mut self, volume: u8, muted: bool) -> bool {
        let next = (volume, muted);
        if self.audio == Some(next) {
            return false;
        }
        self.audio = Some(next);
        true
    }

    fn take_fps_change(&mut self, fps_limit: u16) -> bool {
        if self.fps_limit == Some(fps_limit) {
            return false;
        }
        self.fps_limit = Some(fps_limit);
        true
    }
}

// Persisted WindowMoved coordinates are desktop physical pixels. These values
// match Nera's normal 600x950 launch canvas closely enough to detect stale
// positions after monitor/DPI/layout changes.
const STARTUP_WINDOW_WIDTH_PX: i32 = 600;
const STARTUP_WINDOW_HEIGHT_PX: i32 = 950;
const STARTUP_MIN_VISIBLE_PX: i32 = 64;

#[derive(Resource)]
pub(super) struct PositionRuntime {
    restore_applied: bool,
    pub(super) last_known: IVec2,
    pub(super) dirty: bool,
    save_timer: Timer,
    entrance_settling: bool,
    entrance_quiet_updates: u8,
}

impl Default for PositionRuntime {
    fn default() -> Self {
        Self {
            restore_applied: false,
            last_known: IVec2::new(200, 200),
            dirty: false,
            save_timer: Timer::from_seconds(0.65, TimerMode::Once),
            entrance_settling: false,
            entrance_quiet_updates: 0,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum PositionRecording {
    WindowEvents,
    Suppress,
    Settled(IVec2),
}

impl PositionRuntime {
    fn recording_mode(
        &mut self,
        entrance_blocks_save: bool,
        has_window_event: bool,
        settled_position: Option<IVec2>,
    ) -> PositionRecording {
        if entrance_blocks_save {
            self.entrance_settling = true;
            self.entrance_quiet_updates = 0;
            return PositionRecording::Suppress;
        }
        if !self.entrance_settling {
            return PositionRecording::WindowEvents;
        }

        let Some(position) = settled_position else {
            self.entrance_quiet_updates = 0;
            return PositionRecording::Suppress;
        };

        // Native WindowMoved messages can arrive after the intro has ended.
        // Drain them while sampling the authoritative settled rect, then give
        // ordinary position recording back ownership once the queue is quiet.
        self.entrance_quiet_updates = if has_window_event {
            0
        } else {
            self.entrance_quiet_updates.saturating_add(1)
        };
        if self.entrance_quiet_updates >= 2 {
            self.entrance_settling = false;
        }
        PositionRecording::Settled(position)
    }
}

pub(super) fn initialize_startup_state(mut state: ResMut<StartupRuntime>) {
    match startup::is_enabled() {
        Ok(enabled) => {
            state.enabled = enabled;
            info!(
                "Settings: Start with Windows {}",
                if enabled { "ON" } else { "OFF" }
            );
        }
        Err(error) => {
            state.enabled = false;
            warn!("Settings: failed to read Windows startup state: {error}");
        }
    }
}

#[cfg(target_os = "windows")]
fn safe_startup_position(requested: IVec2) -> (IVec2, bool) {
    let monitors = crate::platform::enumerate_monitors();

    // Do not rewrite persistent state if Windows has not exposed monitor
    // information yet. Visibility retry will keep running independently.
    if monitors.is_empty() {
        return (requested, true);
    }

    let requested_right = requested.x.saturating_add(STARTUP_WINDOW_WIDTH_PX);
    let requested_bottom = requested.y.saturating_add(STARTUP_WINDOW_HEIGHT_PX);

    let reachable = monitors.iter().any(|monitor| {
        let bounds = monitor.bounds;

        let overlap_width = requested_right
            .min(bounds.right)
            .saturating_sub(requested.x.max(bounds.left));

        let overlap_height = requested_bottom
            .min(bounds.bottom)
            .saturating_sub(requested.y.max(bounds.top));

        overlap_width >= STARTUP_MIN_VISIBLE_PX && overlap_height >= STARTUP_MIN_VISIBLE_PX
    });

    if reachable {
        return (requested, true);
    }

    // A remembered coordinate can become invalid after unplugging a monitor,
    // changing resolution/DPI, or a previous native presentation transition.
    // Recover to the primary monitor work area automatically.
    let monitor = monitors
        .iter()
        .find(|monitor| monitor.is_primary)
        .or_else(|| monitors.first())
        .copied()
        .expect("monitor list is known to be non-empty");

    let work = monitor.work_area;
    let width = STARTUP_WINDOW_WIDTH_PX.min(work.width().max(1));
    let height = STARTUP_WINDOW_HEIGHT_PX.min(work.height().max(1));

    let x = work
        .left
        .saturating_add(work.width().saturating_sub(width) / 2);
    let y = work
        .top
        .saturating_add(work.height().saturating_sub(height) / 2);

    (IVec2::new(x, y), false)
}

#[cfg(not(target_os = "windows"))]
fn safe_startup_position(requested: IVec2) -> (IVec2, bool) {
    (requested, true)
}

pub(super) fn apply_remembered_position(
    mut settings: ResMut<NeraSettings>,
    mut runtime: ResMut<PositionRuntime>,
    mut primary: Query<&mut Window, With<PrimaryWindow>>,
    mut commands: Commands,
) {
    if runtime.restore_applied {
        return;
    }

    let Ok(mut window) = primary.single_mut() else {
        return;
    };

    let requested = IVec2::new(settings.window_x, settings.window_y);
    let (position, remembered_position_valid) = safe_startup_position(requested);

    runtime.last_known = position;

    if settings.remember_position {
        window.position = WindowPosition::At(position);

        if remembered_position_valid {
            info!(
                "Settings: restored Nera position -> ({}, {})",
                position.x, position.y
            );
        } else {
            warn!(
                "Settings: remembered position ({}, {}) was off-screen; recovered -> ({}, {})",
                requested.x, requested.y, position.x, position.y
            );

            // Heal the saved coordinates so the invalid position does not
            // repeat on every launch.
            settings.window_x = position.x;
            settings.window_y = position.y;
            save_settings_deferred(&mut commands);
        }
    }

    runtime.restore_applied = true;
}

pub(super) fn record_primary_window_position(
    mut moved: MessageReader<WindowMoved>,
    primary: Query<Entity, With<PrimaryWindow>>,
    mut settings: ResMut<NeraSettings>,
    mut runtime: ResMut<PositionRuntime>,
    entrance: Option<Res<crate::startup_entrance::StartupEntrance>>,
    environment: Option<Res<crate::environment::DesktopEnvironment>>,
    physics: Option<Res<crate::desktop_physics::DesktopPhysicsState>>,
    presentation: Option<Res<crate::presentation::PresentationState>>,
    brain: Option<Res<crate::pet_behavior::PetBehavior>>,
) {
    let Ok(primary_entity) = primary.single() else {
        moved.clear();
        return;
    };

    // Always consume messages, including while the entrance owns placement.
    // Updating settings first and merely delaying disk writes would still save
    // an airborne position if Exit or another settings action flushes early.
    let last_event = moved
        .read()
        .filter(|event| event.window == primary_entity)
        .map(|event| event.position)
        .last();
    // A partly hidden edge position is temporary; keep the previous saved home.
    if brain.is_some_and(|brain| brain.living_blocks_position_save()) {
        return;
    }
    let entrance_blocks_save = entrance
        .as_ref()
        .is_some_and(|entrance| entrance.blocks_position_save());
    let settled_position = match (
        environment.as_deref(),
        physics.as_deref(),
        presentation.as_deref(),
    ) {
        (Some(environment), Some(physics), Some(presentation))
            if !physics.falling && physics.support.is_some() && presentation.is_desktop_pet() =>
        {
            environment
                .nera_rect
                .map(|rect| IVec2::new(rect.left, rect.top))
        }
        _ => None,
    };
    let position = match runtime.recording_mode(
        entrance_blocks_save,
        last_event.is_some(),
        settled_position,
    ) {
        PositionRecording::WindowEvents => last_event,
        PositionRecording::Suppress => None,
        PositionRecording::Settled(position) => Some(position),
    };
    let Some(position) = position else {
        return;
    };
    runtime.last_known = position;
    if settings.remember_position
        && (settings.window_x != position.x || settings.window_y != position.y)
    {
        settings.window_x = position.x;
        settings.window_y = position.y;
        runtime.dirty = true;
        runtime.save_timer.reset();
    }
}

pub(super) fn flush_position_save(
    time: Res<Time>,
    mut runtime: ResMut<PositionRuntime>,
    mut commands: Commands,
) {
    if !runtime.dirty {
        return;
    }

    runtime.save_timer.tick(time.delta());
    if !runtime.save_timer.is_finished() {
        return;
    }

    runtime.dirty = false;
    save_settings_deferred(&mut commands);
}

pub(super) fn apply_always_on_top(
    settings: Res<NeraSettings>,
    mut primary: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !settings.is_changed() {
        return;
    }

    let Ok(mut window) = primary.single_mut() else {
        return;
    };

    let desired = if settings.always_on_top {
        WindowLevel::AlwaysOnTop
    } else {
        WindowLevel::Normal
    };

    if window.window_level != desired {
        window.window_level = desired;
        info!(
            "Settings: Always On Top {}",
            if settings.always_on_top { "ON" } else { "OFF" }
        );
    }
}

pub(super) fn apply_character_size(
    settings: Res<NeraSettings>,
    mut avatar: Query<&mut Transform, With<crate::avatar::NeraAvatar>>,
) {
    if !settings.is_changed() {
        return;
    }

    let Ok(mut transform) = avatar.single_mut() else {
        return;
    };

    let desired = Vec3::splat(settings.character_scale());
    if (transform.scale - desired).length_squared() <= 0.000_001 {
        return;
    }

    transform.scale = desired;
    info!(
        "Settings: Character size applied -> {}",
        settings.character_size_label()
    );
}

pub(super) fn apply_audio_settings(
    settings: Res<NeraSettings>,
    mut applied: ResMut<AppliedRuntimeSettings>,
    mut global_volume: ResMut<GlobalVolume>,
    mut active_sinks: Query<&mut AudioSink>,
) {
    if !settings.is_changed()
        || !applied.take_audio_change(settings.master_volume, settings.audio_muted)
    {
        return;
    }

    let desired = settings.audio_volume();
    global_volume.volume = desired.clone();

    // GlobalVolume is used when future sounds are created. Bevy intentionally
    // does not retroactively change already-playing sinks, so mirror the new
    // value into live Nera audio too (currently the looping alarm).
    for mut sink in &mut active_sinks {
        sink.set_volume(Volume::Linear(
            settings.master_volume.clamp(0, 100) as f32 / 100.0,
        ));

        if settings.audio_muted {
            sink.mute();
        } else {
            sink.unmute();
        }
    }

    info!(
        "Settings: Audio -> volume={} mute={}",
        settings.master_volume_label(),
        settings.audio_muted,
    );
}

pub(super) fn apply_fps_limit(
    settings: Res<NeraSettings>,
    mut applied: ResMut<AppliedRuntimeSettings>,
    mut framepace: ResMut<FramepaceSettings>,
    mut winit: ResMut<WinitSettings>,
) {
    if !settings.is_changed() || !applied.take_fps_change(settings.fps_limit) {
        return;
    }

    // Winit's reactive modes are not strict frame caps: window/redraw events
    // are allowed to wake the app before the configured wait has elapsed.
    // Keep the event loop continuous and let bevy_framepace enforce the real
    // frametime for both focused and unfocused desktop-pet operation.
    winit.focused_mode = UpdateMode::Continuous;
    winit.unfocused_mode = UpdateMode::Continuous;

    let effective_fps = match settings.fps_limit {
        0 => 0,
        // Migrate the retired 15 FPS choice without breaking old settings files.
        15 => 24,
        fps => fps.max(24),
    };

    framepace.limiter = if effective_fps == 0 {
        Limiter::Off
    } else {
        Limiter::from_framerate(f64::from(effective_fps))
    };

    info!("Settings: FPS limit -> {}", settings.fps_label());
}

/// Shared helper for any UI / native shell surface that mutates settings.
pub(crate) fn save_settings_deferred(commands: &mut Commands) {
    commands.queue(SaveSettingsDeferred(Duration::from_millis(150)));
}

pub(crate) fn save_settings_before_exit(commands: &mut Commands) {
    commands.queue(SaveSettingsSync::IfChanged);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_setting_cache_only_accepts_relevant_value_changes() {
        let mut applied = AppliedRuntimeSettings::default();

        assert!(applied.take_audio_change(100, false));
        assert!(!applied.take_audio_change(100, false));
        assert!(applied.take_audio_change(75, false));
        assert!(applied.take_audio_change(75, true));
        assert!(!applied.take_audio_change(75, true));

        assert!(applied.take_fps_change(0));
        assert!(!applied.take_fps_change(0));
        assert!(applied.take_fps_change(60));
        assert!(!applied.take_fps_change(60));
    }

    #[test]
    fn entrance_positions_wait_for_support_and_ignore_delayed_native_events() {
        let mut runtime = PositionRuntime::default();
        let previous = runtime.last_known;
        let settled = IVec2::new(320, 175);

        // Exit during entrance must still see the old safe position. Even a
        // grounded pose cannot be saved while the greeting is still active.
        assert_eq!(
            runtime.recording_mode(true, true, None),
            PositionRecording::Suppress,
        );
        assert_eq!(
            runtime.recording_mode(true, true, Some(settled)),
            PositionRecording::Suppress,
        );
        assert_eq!(runtime.last_known, previous);

        // Cancellation in midair or into Attention has no settled position.
        assert_eq!(
            runtime.recording_mode(false, true, None),
            PositionRecording::Suppress,
        );
        assert_eq!(
            runtime.recording_mode(false, true, Some(settled)),
            PositionRecording::Settled(settled),
        );
        assert!(runtime.entrance_settling);

        // An event delayed past the first quiet update must still use current
        // geometry, not its stale airborne coordinates.
        assert_eq!(
            runtime.recording_mode(false, false, Some(settled)),
            PositionRecording::Settled(settled),
        );
        assert_eq!(
            runtime.recording_mode(false, true, Some(settled)),
            PositionRecording::Settled(settled),
        );
        for _ in 0..2 {
            assert_eq!(
                runtime.recording_mode(false, false, Some(settled)),
                PositionRecording::Settled(settled),
            );
        }
        assert!(!runtime.entrance_settling);
        assert_eq!(
            runtime.recording_mode(false, true, None),
            PositionRecording::WindowEvents,
        );
    }

    #[test]
    fn ordinary_window_movement_keeps_its_existing_recording_behavior() {
        let mut runtime = PositionRuntime::default();
        assert_eq!(
            runtime.recording_mode(false, true, None),
            PositionRecording::WindowEvents,
        );
    }

    #[test]
    fn position_recorder_preserves_saved_coordinates_until_intro_settles() {
        // Spell out the resource so this headless regression never calls the
        // default settings' native local-date lookup or any windowing APIs.
        let settings = NeraSettings {
            audio_reactive_dance: false,
            always_on_top: true,
            start_hidden: false,
            remember_position: true,
            window_x: 100,
            window_y: 150,
            character_size: 100,
            character_asset: String::new(),
            body_mode: "normal".to_string(),
            ground_offset: 0,
            walking_enabled: false,
            cursor_play: false,
            edge_peek: false,
            gravity_enabled: true,
            throw_enabled: true,
            throw_intensity: 50,
            activity_level: 1,
            roaming_radius_px: 400,
            pause_roaming_fullscreen: true,
            follow_mouse: false,
            follow_eyes: false,
            follow_head: false,
            follow_intensity: 75,
            shadow_enabled: false,
            shadow_type: 0,
            shadow_opacity: 40,
            shadow_softness: 75,
            brightness: 100,
            master_volume: 100,
            audio_muted: true,
            fps_limit: 60,
            alarm_enabled: false,
            alarm_hour: 0,
            alarm_minute: 0,
            alarm_mode: 0,
            alarm_year: 2026,
            alarm_month: 9,
            alarm_day: 26,
            alarm_repeat_days: 0,
            alarm_book: String::new(),
            alarm_book_initialized: true,
            alarm_sound: 0,
        };
        let mut app = App::new();
        app.insert_resource(settings)
            .init_resource::<PositionRuntime>()
            .init_resource::<crate::startup_entrance::StartupEntrance>()
            .init_resource::<crate::environment::DesktopEnvironment>()
            .init_resource::<crate::desktop_physics::DesktopPhysicsState>()
            .init_resource::<crate::presentation::PresentationState>()
            .add_message::<WindowMoved>()
            .add_systems(Update, record_primary_window_position);
        let window = app.world_mut().spawn(PrimaryWindow).id();
        app.world_mut().write_message(WindowMoved {
            window,
            position: IVec2::new(100, -900),
        });
        app.update();
        let settings = app.world().resource::<NeraSettings>();
        assert_eq!((settings.window_x, settings.window_y), (100, 150));
        assert!(!app.world().resource::<PositionRuntime>().dirty);

        // Removing the optional entrance resource simulates its releasing
        // ownership without installing the native entrance/physics systems.
        app.world_mut()
            .remove_resource::<crate::startup_entrance::StartupEntrance>();
        app.world_mut().write_message(WindowMoved {
            window,
            position: IVec2::new(100, -500),
        });
        app.update();
        assert_eq!(app.world().resource::<NeraSettings>().window_y, 150);

        app.world_mut()
            .resource_mut::<crate::environment::DesktopEnvironment>()
            .nera_rect = Some(crate::platform::PhysicalRect {
            left: 300,
            top: 200,
            right: 900,
            bottom: 1150,
        });
        app.world_mut()
            .resource_mut::<crate::desktop_physics::DesktopPhysicsState>()
            .support = Some(crate::desktop_physics::PhysicsSupport::Desktop {
            monitor: crate::platform::NativeMonitorHandle::from_raw(1),
        });
        app.world_mut().write_message(WindowMoved {
            window,
            position: IVec2::new(100, -300),
        });
        app.update();
        let settings = app.world().resource::<NeraSettings>();
        assert_eq!((settings.window_x, settings.window_y), (300, 200));
        assert!(app.world().resource::<PositionRuntime>().dirty);

        app.update();
        app.update();
        app.world_mut().write_message(WindowMoved {
            window,
            position: IVec2::new(400, 250),
        });
        app.update();
        let settings = app.world().resource::<NeraSettings>();
        assert_eq!((settings.window_x, settings.window_y), (400, 250));
    }
}
