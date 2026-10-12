use std::{
    ffi::c_void,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicIsize, AtomicU32, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use bevy::{
    app::AppExit,
    prelude::*,
    window::{PrimaryWindow, Window},
};

use crate::{
    animation::NeraAnimationState,
    desktop_physics::DesktopPhysicsState,
    environment::DesktopEnvironment,
    platform,
    settings::{NeraSettings, save_settings_before_exit, save_settings_deferred},
};

use crate::action_menu::NeraActionMenuCommand;

mod native;

use native::run_tray_thread;

use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE},
};

// =============================================================================
// Plugin
// =============================================================================

pub struct TrayPlugin;

impl Plugin for TrayPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TrayNotification>()
            .add_systems(Update, show_notifications)
            .add_systems(Startup, start_system_tray)
            .add_systems(
                Update,
                process_tray_commands.before(crate::startup_entrance::prepare_startup_entrance),
            );
    }
}

/// One-shot user feedback through the existing Windows notification-area icon.
/// It does not arm, cancel or borrow the real alarm runtime.
#[derive(Message)]
pub(crate) struct TrayNotification {
    pub(crate) title: String,
    pub(crate) body: String,
}

fn show_notifications(
    runtime: Option<Res<TrayRuntime>>,
    mut notifications: MessageReader<TrayNotification>,
) {
    for notification in notifications.read() {
        let raw = runtime
            .as_ref()
            .map_or(0, |runtime| runtime.hidden_hwnd_raw());
        if raw == 0 {
            warn!("Nera notification unavailable: tray is not ready");
            continue;
        }
        if let Err(error) = native::show_notification(raw, &notification.title, &notification.body)
        {
            warn!("Nera notification failed: {error}");
        } else {
            info!(
                "Nera notification submitted to Windows | {}",
                notification.title
            );
        }
    }
}

// =============================================================================
// Tray -> Bevy commands
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrayCommand {
    Show,
    Hide,
    ResetPosition,
    ToggleAlwaysOnTop,
    Screenshot,
    Exit,
}

// =============================================================================
// Constants
// =============================================================================

const TRAY_ICON_ID: u32 = 1;

// Private message untuk callback Shell_NotifyIcon.
const WM_NERA_TRAY: u32 = 6002;

// Native context-menu IDs.
const MENU_SHOW: usize = 1001;
const MENU_HIDE: usize = 1002;
const MENU_ALWAYS_ON_TOP: usize = 1003;
const MENU_EXIT: usize = 1004;
const MENU_RESET_POSITION: usize = 1005;
const MENU_SCREENSHOT: usize = 1006;

fn command_for_menu_id(id: usize) -> Option<TrayCommand> {
    match id {
        MENU_SHOW => Some(TrayCommand::Show),
        MENU_HIDE => Some(TrayCommand::Hide),
        MENU_ALWAYS_ON_TOP => Some(TrayCommand::ToggleAlwaysOnTop),
        MENU_EXIT => Some(TrayCommand::Exit),
        MENU_RESET_POSITION => Some(TrayCommand::ResetPosition),
        MENU_SCREENSHOT => Some(TrayCommand::Screenshot),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshot_menu_id_preserves_all_existing_commands() {
        for (id, command) in [
            (1001, TrayCommand::Show),
            (1002, TrayCommand::Hide),
            (1003, TrayCommand::ToggleAlwaysOnTop),
            (1004, TrayCommand::Exit),
            (1005, TrayCommand::ResetPosition),
            (1006, TrayCommand::Screenshot),
        ] {
            assert_eq!(command_for_menu_id(id), Some(command));
        }
        assert_eq!(command_for_menu_id(0), None);
        assert_eq!(command_for_menu_id(999), None);
    }
}

// Registered Windows message yang dikirim ketika Explorer/taskbar dibuat ulang.
static TASKBAR_CREATED_MSG: AtomicU32 = AtomicU32::new(0);

// =============================================================================
// Command channel
// =============================================================================

static TRAY_SENDER: OnceLock<Mutex<Option<Sender<TrayCommand>>>> = OnceLock::new();

pub(crate) fn send_tray_command(command: TrayCommand) {
    let Some(sender_slot) = TRAY_SENDER.get() else {
        return;
    };

    let Ok(sender_guard) = sender_slot.lock() else {
        return;
    };

    let Some(sender) = sender_guard.as_ref() else {
        return;
    };

    let _ = sender.send(command);
}

// =============================================================================
// Phase 5.5: typed Nera action-menu result channel
// =============================================================================

static ACTION_MENU_SENDER: OnceLock<Mutex<Option<Sender<Option<NeraActionMenuCommand>>>>> =
    OnceLock::new();

pub(crate) fn send_action_menu_result(result: Option<NeraActionMenuCommand>) {
    let Some(sender_slot) = ACTION_MENU_SENDER.get() else {
        return;
    };

    let Ok(sender_guard) = sender_slot.lock() else {
        return;
    };

    let Some(sender) = sender_guard.as_ref() else {
        return;
    };

    let _ = sender.send(result);
}

// =============================================================================
// Bevy-side runtime
// =============================================================================

#[derive(Resource)]
pub(crate) struct TrayRuntime {
    receiver: Mutex<Receiver<TrayCommand>>,

    action_menu_receiver: Mutex<Receiver<Option<NeraActionMenuCommand>>>,

    // HWND is represented as an atomic raw value because windows-rs HWND is
    // not a Send/Sync Bevy resource.
    hidden_hwnd: Arc<AtomicIsize>,
}

impl TrayRuntime {
    pub(crate) fn hidden_hwnd_raw(&self) -> isize {
        self.hidden_hwnd.load(Ordering::Acquire)
    }

    pub(crate) fn drain_action_menu_results(&self) -> Vec<Option<NeraActionMenuCommand>> {
        let Ok(receiver) = self.action_menu_receiver.lock() else {
            return Vec::new();
        };

        receiver.try_iter().collect()
    }
}

impl Drop for TrayRuntime {
    fn drop(&mut self) {
        let raw = self.hidden_hwnd.load(Ordering::Acquire);

        if raw == 0 {
            return;
        }

        let hwnd = HWND(raw as *mut c_void);

        // Pastikan tray thread ikut ditutup
        // kalau aplikasi berhenti lewat jalur lain.
        let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

// =============================================================================
// Startup
// =============================================================================

fn start_system_tray(mut commands: Commands) {
    let (sender, receiver) = mpsc::channel();

    let sender_slot = TRAY_SENDER.get_or_init(|| Mutex::new(None));

    if let Ok(mut slot) = sender_slot.lock() {
        *slot = Some(sender);
    }

    // Phase 5.5: typed Nera action-menu channel.
    let (action_menu_sender, action_menu_receiver) = mpsc::channel();

    let sender_slot = ACTION_MENU_SENDER.get_or_init(|| Mutex::new(None));

    if let Ok(mut slot) = sender_slot.lock() {
        *slot = Some(action_menu_sender);
    }

    let hidden_hwnd = Arc::new(AtomicIsize::new(0));

    let thread_hwnd = Arc::clone(&hidden_hwnd);

    let spawn_result = thread::Builder::new()
        .name("nera-tray".into())
        .spawn(move || {
            if let Err(error) = run_tray_thread(thread_hwnd) {
                eprintln!("Nera tray error: {error:?}");
            }
        });

    if let Err(error) = spawn_result {
        error!("Failed to start Nera tray thread: {}", error);

        return;
    }

    commands.insert_resource(TrayRuntime {
        receiver: Mutex::new(receiver),

        action_menu_receiver: Mutex::new(action_menu_receiver),

        hidden_hwnd,
    });

    info!("Nera system tray started");
}

// =============================================================================
// Bevy command handling
// =============================================================================

pub(crate) fn process_tray_commands(
    runtime: Option<Res<TrayRuntime>>,

    mut windows: Query<&mut Window, With<PrimaryWindow>>,

    mut settings: ResMut<NeraSettings>,

    mut commands: Commands,

    mut exit: MessageWriter<AppExit>,

    mut environment: ResMut<DesktopEnvironment>,

    mut physics: ResMut<DesktopPhysicsState>,

    mut animation: ResMut<NeraAnimationState>,

    mut entrance: ResMut<crate::startup_entrance::StartupEntrance>,
    mut cloud: ResMut<crate::cloud::CloudRuntime>,
    mut sfx: MessageWriter<crate::pet_feedback::PetSfx>,
    mut fx: MessageWriter<crate::pet_feedback::PetFx>,
    mut screenshots: MessageWriter<crate::presentation::screenshot::ScreenshotRequest>,
) {
    let Some(runtime) = runtime else {
        return;
    };

    let pending_commands = {
        let Ok(receiver) = runtime.receiver.lock() else {
            return;
        };

        receiver.try_iter().collect::<Vec<_>>()
    };

    for command in pending_commands {
        match command {
            TrayCommand::Screenshot => {
                screenshots.write(crate::presentation::screenshot::ScreenshotRequest);
            }
            // =============================================================
            // SHOW
            // =============================================================
            TrayCommand::Show => {
                if let Some(mut window) = windows.iter_mut().next() {
                    window.visible = true;

                    info!("Tray: Show Nera");
                }
            }

            // =============================================================
            // HIDE
            // =============================================================
            TrayCommand::Hide => {
                if let Some(mut window) = windows.iter_mut().next() {
                    window.visible = false;

                    info!("Tray: Hide Nera");
                }
            }

            // =============================================================
            // RESET POSITION
            // =============================================================
            TrayCommand::ResetPosition => {
                crate::cloud::dismiss_cloud(
                    &mut cloud,
                    &mut environment,
                    &mut physics,
                    &mut animation,
                    &mut sfx,
                    &mut fx,
                );
                let Some(hwnd) = environment.nera_hwnd else {
                    warn!("Tray: Reset Position ignored because Nera HWND is not ready");
                    continue;
                };

                let monitors = platform::enumerate_monitors();
                let monitor = monitors
                    .iter()
                    .copied()
                    .find(|monitor| monitor.is_primary)
                    .or_else(|| monitors.first().copied());

                let Some(monitor) = monitor else {
                    warn!("Tray: Reset Position failed because no active monitor was found");
                    continue;
                };

                let rect = platform::window_rect(hwnd).or(environment.nera_rect);

                let Some(rect) = rect else {
                    warn!("Tray: Reset Position failed because Nera geometry is unavailable");
                    continue;
                };

                // Recovery is authoritative: any sitting/window attachment or
                // in-progress fall must stop owning the HWND before we move it.
                entrance.request_cancel();
                environment.detach_nera();
                physics.reset_for_position_recovery();
                animation.cancel_physics_motion();

                let work = monitor.work_area;
                let width = rect.width().max(1);
                let height = rect.height().max(1);

                let position = platform::PhysicalPoint::new(
                    work.left + (work.width() - width) / 2,
                    work.top + (work.height() - height) / 2,
                );

                match platform::move_window(hwnd, position) {
                    Ok(()) => {
                        environment.nera_rect = platform::window_rect(hwnd);
                        environment.monitor = Some(monitor);

                        if let Some(mut window) = windows.iter_mut().next() {
                            // Reset Position doubles as an emergency recovery
                            // command, so a previously hidden Nera is shown.
                            window.visible = true;
                        }

                        if settings.remember_position {
                            settings.window_x = position.x;
                            settings.window_y = position.y;
                            save_settings_deferred(&mut commands);
                        }

                        info!(
                            "Tray: Reset Position -> primary work-area center ({}, {})",
                            position.x, position.y,
                        );
                    }
                    Err(error) => {
                        warn!("Tray: Reset Position failed: {error}");
                    }
                }
            }

            // =============================================================
            // ALWAYS ON TOP
            // =============================================================
            TrayCommand::ToggleAlwaysOnTop => {
                settings.toggle_always_on_top();
                save_settings_deferred(&mut commands);

                info!(
                    "Tray: Always On Top {}",
                    if settings.always_on_top { "ON" } else { "OFF" }
                );
            }

            // =============================================================
            // EXIT
            // =============================================================
            TrayCommand::Exit => {
                info!("Tray: Exit Nera");

                save_settings_before_exit(&mut commands);
                exit.write(AppExit::Success);
            }
        }
    }
}
