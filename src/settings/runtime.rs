//! Apply persisted settings to the running application and manage position persistence.
use std::time::Duration;

use bevy::{
    audio::{AudioSink, AudioSinkPlayback, GlobalVolume, Volume},
    prelude::*,
    settings::{SaveSettingsDeferred, SaveSettingsSync},
    window::{PrimaryWindow, WindowLevel, WindowMoved, WindowPosition},
    winit::{UpdateMode, WinitSettings},
};

use super::{NeraSettings, startup};

#[derive(Resource, Default)]
pub(super) struct StartupRuntime {
    pub(super) enabled: bool,
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
}

impl Default for PositionRuntime {
    fn default() -> Self {
        Self {
            restore_applied: false,
            last_known: IVec2::new(200, 200),
            dirty: false,
            save_timer: Timer::from_seconds(0.65, TimerMode::Once),
        }
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
) {
    let Ok(primary_entity) = primary.single() else {
        return;
    };

    for event in moved.read() {
        if event.window != primary_entity {
            continue;
        }

        runtime.last_known = event.position;

        if !settings.remember_position {
            continue;
        }

        if settings.window_x == event.position.x && settings.window_y == event.position.y {
            continue;
        }

        settings.window_x = event.position.x;
        settings.window_y = event.position.y;
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
    mut global_volume: ResMut<GlobalVolume>,
    mut active_sinks: Query<&mut AudioSink>,
) {
    if !settings.is_changed() {
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

pub(super) fn apply_fps_limit(settings: Res<NeraSettings>, mut winit: ResMut<WinitSettings>) {
    if !settings.is_changed() {
        return;
    }

    let mode = if settings.fps_limit == 0 {
        UpdateMode::Continuous
    } else {
        let seconds = 1.0 / f64::from(settings.fps_limit.max(30));
        UpdateMode::reactive_low_power(Duration::from_secs_f64(seconds))
    };

    // A desktop pet is very often *not* the focused window. Limiting only
    // focused_mode would therefore make the setting mostly ineffective.
    winit.focused_mode = mode.clone();
    winit.unfocused_mode = mode;

    info!("Settings: FPS limit -> {}", settings.fps_label());
}

/// Shared helper for any UI / native shell surface that mutates settings.
pub(crate) fn save_settings_deferred(commands: &mut Commands) {
    commands.queue(SaveSettingsDeferred(Duration::from_millis(150)));
}

pub(crate) fn save_settings_before_exit(commands: &mut Commands) {
    commands.queue(SaveSettingsSync::IfChanged);
}
