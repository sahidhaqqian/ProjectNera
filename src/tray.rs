use std::{
    ffi::c_void,
    mem::size_of,
    sync::{
        atomic::{
            AtomicIsize,
            AtomicU32,
            Ordering,
        },
        mpsc::{
            self,
            Receiver,
            Sender,
        },
        Arc,
        Mutex,
        OnceLock,
    },
    thread,
};

use bevy::{
    app::AppExit,
    prelude::*,
    window::{
        PrimaryWindow,
        Window,
        WindowLevel,
    },
};

use windows::{
    core::{
        w,
        Error,
        HRESULT,
        PCWSTR,
        Result,
    },
    Win32::{
        Foundation::{
            GetLastError,
            HINSTANCE,
            HWND,
            LPARAM,
            LRESULT,
            POINT,
            WPARAM,
        },
        System::LibraryLoader::{
            GetModuleHandleW,
        },
        UI::{
            Shell::{
                NIF_ICON,
                NIF_MESSAGE,
                NIF_TIP,
                NIM_ADD,
                NIM_DELETE,
                NOTIFYICONDATAW,
                Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW,
                CreatePopupMenu,
                CreateWindowExW,
                DefWindowProcW,
                DestroyMenu,
                DestroyWindow,
                DispatchMessageW,
                GetCursorPos,
                GetMessageW,
                IDI_APPLICATION,
                LoadIconW,
                MF_SEPARATOR,
                MF_STRING,
                MSG,
                PostMessageW,
                PostQuitMessage,
                RegisterClassExW,
                RegisterWindowMessageW,
                SetForegroundWindow,
                TPM_BOTTOMALIGN,
                TPM_LEFTALIGN,
                TPM_RETURNCMD,
                TPM_RIGHTBUTTON,
                TrackPopupMenu,
                TranslateMessage,
                WM_CLOSE,
                WM_DESTROY,
                WM_LBUTTONUP,
                WM_NULL,
                WM_RBUTTONUP,
                WNDCLASSEXW,
                WS_EX_TOOLWINDOW,
                WS_OVERLAPPED,
            },
        },
    },
};

// =============================================================================
// Plugin
// =============================================================================

pub struct TrayPlugin;

impl Plugin for TrayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            start_system_tray,
        )
        .add_systems(
            Update,
            process_tray_commands,
        );
    }
}

// =============================================================================
// Tray -> Bevy commands
// =============================================================================

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
)]
enum TrayCommand {
    Show,
    Hide,
    ToggleAlwaysOnTop,
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

// Registered Windows message yang dikirim ketika Explorer/taskbar dibuat ulang.
static TASKBAR_CREATED_MSG: AtomicU32 =
    AtomicU32::new(0);

// =============================================================================
// Command channel
// =============================================================================

static TRAY_SENDER:
    OnceLock<
        Mutex<
            Option<
                Sender<TrayCommand>,
            >,
        >,
    > = OnceLock::new();

fn send_tray_command(
    command: TrayCommand,
) {
    let Some(sender_slot) =
        TRAY_SENDER.get()
    else {
        return;
    };

    let Ok(sender_guard) =
        sender_slot.lock()
    else {
        return;
    };

    let Some(sender) =
        sender_guard.as_ref()
    else {
        return;
    };

    let _ =
        sender.send(command);
}

// =============================================================================
// Bevy-side runtime
// =============================================================================

#[derive(Resource)]
struct TrayRuntime {
    receiver:
        Mutex<
            Receiver<TrayCommand>,
        >,

    // HWND tidak disimpan langsung sebagai Resource,
    // karena HWND dari windows-rs bukan Send/Sync.
    hidden_hwnd:
        Arc<AtomicIsize>,
}

impl Drop for TrayRuntime {
    fn drop(&mut self) {
        let raw =
            self.hidden_hwnd.load(
                Ordering::Acquire,
            );

        if raw == 0 {
            return;
        }

        let hwnd =
            HWND(
                raw as *mut c_void,
            );

        // Pastikan tray thread ikut ditutup
        // kalau aplikasi berhenti lewat jalur lain.
        let _ = unsafe {
            PostMessageW(
                Some(hwnd),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
    }
}

// =============================================================================
// Startup
// =============================================================================

fn start_system_tray(
    mut commands: Commands,
) {
    let (
        sender,
        receiver,
    ) = mpsc::channel();

    let sender_slot =
        TRAY_SENDER.get_or_init(
            || Mutex::new(None),
        );

    if let Ok(mut slot) =
        sender_slot.lock()
    {
        *slot = Some(sender);
    }

    let hidden_hwnd =
        Arc::new(
            AtomicIsize::new(0),
        );

    let thread_hwnd =
        Arc::clone(
            &hidden_hwnd,
        );

    let spawn_result =
        thread::Builder::new()
            .name(
                "nera-tray".into(),
            )
            .spawn(move || {
                if let Err(error) =
                    run_tray_thread(
                        thread_hwnd,
                    )
                {
                    eprintln!(
                        "Nera tray error: {error:?}"
                    );
                }
            });

    if let Err(error) =
        spawn_result
    {
        error!(
            "Failed to start Nera tray thread: {}",
            error
        );

        return;
    }

    commands.insert_resource(
        TrayRuntime {
            receiver:
                Mutex::new(
                    receiver,
                ),

            hidden_hwnd,
        },
    );

    info!(
        "Nera system tray started"
    );
}

// =============================================================================
// Bevy command handling
// =============================================================================

fn process_tray_commands(
    runtime:
        Option<Res<TrayRuntime>>,

    mut windows:
        Query<
            &mut Window,
            With<PrimaryWindow>,
        >,

    mut exit:
        MessageWriter<AppExit>,
) {
    let Some(runtime) =
        runtime
    else {
        return;
    };

    let pending_commands = {
        let Ok(receiver) =
            runtime
                .receiver
                .lock()
        else {
            return;
        };

        receiver
            .try_iter()
            .collect::<Vec<_>>()
    };

    for command in
        pending_commands
    {
        match command {
            // =============================================================
            // SHOW
            // =============================================================

            TrayCommand::Show => {
                if let Some(
                    mut window,
                ) = windows
                    .iter_mut()
                    .next()
                {
                    window.visible =
                        true;

                    info!(
                        "Tray: Show Nera"
                    );
                }
            }

            // =============================================================
            // HIDE
            // =============================================================

            TrayCommand::Hide => {
                if let Some(
                    mut window,
                ) = windows
                    .iter_mut()
                    .next()
                {
                    window.visible =
                        false;

                    info!(
                        "Tray: Hide Nera"
                    );
                }
            }

            // =============================================================
            // ALWAYS ON TOP
            // =============================================================

            TrayCommand::ToggleAlwaysOnTop => {
                if let Some(
                    mut window,
                ) = windows
                    .iter_mut()
                    .next()
                {
                    let enabled =
                        matches!(
                            window.window_level,
                            WindowLevel::AlwaysOnTop
                        );

                    if enabled {
                        window.window_level =
                            WindowLevel::Normal;

                        info!(
                            "Tray: Always On Top OFF"
                        );
                    } else {
                        window.window_level =
                            WindowLevel::AlwaysOnTop;

                        info!(
                            "Tray: Always On Top ON"
                        );
                    }
                }
            }

            // =============================================================
            // EXIT
            // =============================================================

            TrayCommand::Exit => {
                info!(
                    "Tray: Exit Nera"
                );

                exit.write(
                    AppExit::Success,
                );
            }
        }
    }
}

// =============================================================================
// UTF-16 helper
// =============================================================================

fn write_utf16(
    destination: &mut [u16],
    value: &str,
) {
    destination.fill(0);

    let max_length =
        destination
            .len()
            .saturating_sub(1);

    for (
        output,
        input,
    ) in destination
        .iter_mut()
        .take(max_length)
        .zip(
            value.encode_utf16(),
        )
    {
        *output = input;
    }
}

// =============================================================================
// Win32 error helper
// =============================================================================

fn last_win32_error() -> Error {
    let error =
        unsafe {
            GetLastError()
        };

    Error::from_hresult(
        HRESULT::from_win32(
            error.0,
        ),
    )
}

// =============================================================================
// Tray icon
// =============================================================================

unsafe fn add_tray_icon(
    hwnd: HWND,
) -> Result<()> {
    let icon =
        unsafe {
            LoadIconW(
                None,
                IDI_APPLICATION,
            )?
        };

    let mut data =
        NOTIFYICONDATAW {
            cbSize:
                size_of::<
                    NOTIFYICONDATAW,
                >() as u32,

            hWnd:
                hwnd,

            uID:
                TRAY_ICON_ID,

            uFlags:
                NIF_MESSAGE
                    | NIF_ICON
                    | NIF_TIP,

            uCallbackMessage:
                WM_NERA_TRAY,

            hIcon:
                icon,

            ..Default::default()
        };

    write_utf16(
        &mut data.szTip,
        "Nera",
    );

    let added =
        unsafe {
            Shell_NotifyIconW(
                NIM_ADD,
                &data,
            )
        };

    if !added.as_bool() {
        return Err(
            last_win32_error(),
        );
    }

    info!(
        "Tray native icon registered | hwnd=0x{:X} callback={}",
        hwnd.0 as usize,
        WM_NERA_TRAY,
    );

    Ok(())
}

unsafe fn remove_tray_icon(
    hwnd: HWND,
) {
    let data =
        NOTIFYICONDATAW {
            cbSize:
                size_of::<
                    NOTIFYICONDATAW,
                >() as u32,

            hWnd:
                hwnd,

            uID:
                TRAY_ICON_ID,

            ..Default::default()
        };

    let _ =
        unsafe {
            Shell_NotifyIconW(
                NIM_DELETE,
                &data,
            )
        };
}

// =============================================================================
// Native safety menu
// =============================================================================

unsafe fn show_safety_menu(
    hwnd: HWND,
) {
    let Ok(menu) =
        (unsafe {
            CreatePopupMenu()
        })
    else {
        error!(
            "Tray: failed to create popup menu"
        );

        return;
    };

    // -------------------------------------------------------------------------
    // Show
    // -------------------------------------------------------------------------

    let _ =
        unsafe {
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_SHOW,
                w!("Show Nera"),
            )
        };

    // -------------------------------------------------------------------------
    // Hide
    // -------------------------------------------------------------------------

    let _ =
        unsafe {
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_HIDE,
                w!("Hide Nera"),
            )
        };

    // -------------------------------------------------------------------------
    // Always On Top
    // -------------------------------------------------------------------------

    let _ =
        unsafe {
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_ALWAYS_ON_TOP,
                w!("Always On Top"),
            )
        };

    // -------------------------------------------------------------------------
    // Separator
    // -------------------------------------------------------------------------

    let _ =
        unsafe {
            AppendMenuW(
                menu,
                MF_SEPARATOR,
                0,
                PCWSTR::null(),
            )
        };

    // -------------------------------------------------------------------------
    // Exit
    // -------------------------------------------------------------------------

    let _ =
        unsafe {
            AppendMenuW(
                menu,
                MF_STRING,
                MENU_EXIT,
                w!("Exit"),
            )
        };

    let mut cursor =
        POINT::default();

    if unsafe {
        GetCursorPos(
            &mut cursor,
        )
    }
    .is_err()
    {
        let _ =
            unsafe {
                DestroyMenu(
                    menu,
                )
            };

        return;
    }

    info!(
        "Tray: opening safety menu at {}, {}",
        cursor.x,
        cursor.y,
    );

    // Diperlukan agar popup menu notification-area
    // mendapatkan foreground behavior yang benar.
    let _ =
        unsafe {
            SetForegroundWindow(
                hwnd,
            )
        };

    let selected =
        unsafe {
            TrackPopupMenu(
                menu,

                TPM_LEFTALIGN
                    | TPM_BOTTOMALIGN
                    | TPM_RIGHTBUTTON
                    | TPM_RETURNCMD,

                cursor.x,
                cursor.y,

                None,
                hwnd,
                None,
            )
        };

    let command =
        selected.0 as usize;

    // Recommended Win32 tray-menu cleanup behavior.
    let _ =
        unsafe {
            PostMessageW(
                Some(hwnd),
                WM_NULL,
                WPARAM(0),
                LPARAM(0),
            )
        };

    let _ =
        unsafe {
            DestroyMenu(
                menu,
            )
        };

    // -------------------------------------------------------------------------
    // Handle menu command
    // -------------------------------------------------------------------------

    match command {
        MENU_SHOW => {
            send_tray_command(
                TrayCommand::Show,
            );
        }

        MENU_HIDE => {
            send_tray_command(
                TrayCommand::Hide,
            );
        }

        MENU_ALWAYS_ON_TOP => {
            send_tray_command(
                TrayCommand::ToggleAlwaysOnTop,
            );
        }

        MENU_EXIT => {
            send_tray_command(
                TrayCommand::Exit,
            );

            // Tutup hidden tray window.
            //
            // Bevy sendiri akan keluar lewat AppExit.
            let _ =
                unsafe {
                    DestroyWindow(
                        hwnd,
                    )
                };
        }

        // 0 berarti menu dibatalkan / click outside.
        _ => {}
    }
}

// =============================================================================
// Hidden tray window procedure
// =============================================================================

unsafe extern "system" fn tray_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // =========================================================================
    // Explorer.exe restarted
    // =========================================================================

    let taskbar_msg =
        TASKBAR_CREATED_MSG.load(
            Ordering::Relaxed,
        );

    if taskbar_msg != 0
        && message == taskbar_msg
    {
        info!(
            "Explorer restarted; re-registering Nera tray icon"
        );

        if let Err(error) =
            unsafe {
                add_tray_icon(
                    hwnd,
                )
            }
        {
            error!(
                "Failed to re-register Nera tray icon: {:?}",
                error
            );
        }

        return LRESULT(0);
    }

    // =========================================================================
    // Shell notification-area callback
    // =========================================================================

    if message == WM_NERA_TRAY {
        let event =
            lparam.0 as u32;

        match event {
            // -----------------------------------------------------------------
            // Left click
            // -----------------------------------------------------------------

            WM_LBUTTONUP => {
                info!(
                    "Tray left click"
                );

                send_tray_command(
                    TrayCommand::Show,
                );
            }

            // -----------------------------------------------------------------
            // Right click
            // -----------------------------------------------------------------

            WM_RBUTTONUP => {
                info!(
                    "Tray right click"
                );

                unsafe {
                    show_safety_menu(
                        hwnd,
                    );
                }
            }

            // Mouse move / button down / etc.
            _ => {}
        }

        return LRESULT(0);
    }

    // =========================================================================
    // Normal hidden-window messages
    // =========================================================================

    match message {
        WM_CLOSE => {
            let _ =
                unsafe {
                    DestroyWindow(
                        hwnd,
                    )
                };

            LRESULT(0)
        }

        WM_DESTROY => {
            info!(
                "Destroying Nera tray"
            );

            unsafe {
                remove_tray_icon(
                    hwnd,
                );

                PostQuitMessage(
                    0,
                );
            }

            LRESULT(0)
        }

        _ =>
            unsafe {
                DefWindowProcW(
                    hwnd,
                    message,
                    wparam,
                    lparam,
                )
            },
    }
}

// =============================================================================
// Win32 tray thread
// =============================================================================

fn run_tray_thread(
    hidden_hwnd:
        Arc<AtomicIsize>,
) -> Result<()> {
    let module =
        unsafe {
            GetModuleHandleW(
                None,
            )?
        };

    let instance =
        HINSTANCE(
            module.0,
        );

    let class_name =
        w!(
            "NeraTrayWindowClass"
        );

    // =========================================================================
    // Register hidden window class
    // =========================================================================

    let window_class =
        WNDCLASSEXW {
            cbSize:
                size_of::<
                    WNDCLASSEXW,
                >() as u32,

            hInstance:
                instance,

            lpszClassName:
                class_name,

            lpfnWndProc:
                Some(
                    tray_window_proc,
                ),

            ..Default::default()
        };

    let atom =
        unsafe {
            RegisterClassExW(
                &window_class,
            )
        };

    if atom == 0 {
        return Err(
            last_win32_error(),
        );
    }

    // =========================================================================
    // Create hidden Win32 window
    // =========================================================================

    let hwnd =
        unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,

                class_name,

                w!("Nera Tray"),

                WS_OVERLAPPED,

                0,
                0,
                0,
                0,

                None,
                None,

                Some(instance),

                None,
            )?
        };

    hidden_hwnd.store(
        hwnd.0 as isize,
        Ordering::Release,
    );

    // =========================================================================
    // Register tray icon
    // =========================================================================

    if let Err(error) =
        unsafe {
            add_tray_icon(
                hwnd,
            )
        }
    {
        hidden_hwnd.store(
            0,
            Ordering::Release,
        );

        let _ =
            unsafe {
                DestroyWindow(
                    hwnd,
                )
            };

        return Err(
            error,
        );
    }

    // =========================================================================
    // Explorer restart support
    // =========================================================================

    let taskbar_msg =
        unsafe {
            RegisterWindowMessageW(
                w!("TaskbarCreated"),
            )
        };

    TASKBAR_CREATED_MSG.store(
        taskbar_msg,
        Ordering::Relaxed,
    );

    // =========================================================================
    // Win32 message loop
    // =========================================================================

    let mut message =
        MSG::default();

    loop {
        let result =
            unsafe {
                GetMessageW(
                    &mut message,
                    None,
                    0,
                    0,
                )
            };

        // GetMessage:
        //
        // > 0 = normal message
        //   0 = WM_QUIT
        //  -1 = failure

        if result.0 == -1 {
            let error =
                last_win32_error();

            hidden_hwnd.store(
                0,
                Ordering::Release,
            );

            return Err(
                error,
            );
        }

        if result.0 == 0 {
            break;
        }

        unsafe {
            let _ =
                TranslateMessage(
                    &message,
                );

            DispatchMessageW(
                &message,
            );
        }
    }

    hidden_hwnd.store(
        0,
        Ordering::Release,
    );

    Ok(())
}