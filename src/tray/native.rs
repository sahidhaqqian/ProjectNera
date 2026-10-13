//! Win32 tray icon, menu, callback window, and message loop.
use std::{
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicIsize, Ordering},
    },
};

use bevy::prelude::*;
use windows::{
    Win32::{
        Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{
                NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
                NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
                DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, MF_SEPARATOR,
                MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassExW,
                RegisterWindowMessageW, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN,
                TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_CLOSE,
                WM_DESTROY, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSEXW, WS_EX_TOOLWINDOW,
                WS_OVERLAPPED,
            },
        },
    },
    core::{Error, HRESULT, PCWSTR, Result, w},
};

use super::{
    MENU_ALWAYS_ON_TOP, MENU_EXIT, MENU_HIDE, MENU_RESET_POSITION, MENU_SCREENSHOT, MENU_SHOW,
    TASKBAR_CREATED_MSG, TRAY_ICON_ID, TrayCommand, WM_NERA_TRAY, command_for_menu_id,
    send_action_menu_result, send_tray_command,
};
use crate::action_menu::{WM_SHOW_NERA_ACTION_MENU, show_nera_action_menu};

fn write_utf16(destination: &mut [u16], value: &str) {
    destination.fill(0);

    let max_length = destination.len().saturating_sub(1);

    for (output, input) in destination
        .iter_mut()
        .take(max_length)
        .zip(value.encode_utf16())
    {
        *output = input;
    }
}

// =============================================================================
// Win32 error helper
// =============================================================================

fn last_win32_error() -> Error {
    let error = unsafe { GetLastError() };

    Error::from_hresult(HRESULT::from_win32(error.0))
}

// =============================================================================
// Tray icon
// =============================================================================

unsafe fn add_tray_icon(hwnd: HWND) -> Result<()> {
    let icon = crate::application_icon::shared_icon(16)?;

    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,

        hWnd: hwnd,

        uID: TRAY_ICON_ID,

        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,

        uCallbackMessage: WM_NERA_TRAY,

        hIcon: icon,

        ..Default::default()
    };

    write_utf16(&mut data.szTip, "Nera");

    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) };

    if !added.as_bool() {
        return Err(last_win32_error());
    }

    info!(
        "Tray native icon registered | hwnd=0x{:X} callback={}",
        hwnd.0 as usize, WM_NERA_TRAY,
    );

    Ok(())
}

unsafe fn remove_tray_icon(hwnd: HWND) {
    let data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,

        hWnd: hwnd,

        uID: TRAY_ICON_ID,

        ..Default::default()
    };

    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
}

pub(super) fn show_notification(raw: isize, title: &str, body: &str) -> Result<()> {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: HWND(raw as *mut std::ffi::c_void),
        uID: TRAY_ICON_ID,
        uFlags: NIF_INFO,
        dwInfoFlags: NIIF_INFO,
        ..Default::default()
    };
    write_utf16(&mut data.szInfoTitle, title);
    write_utf16(&mut data.szInfo, body);
    if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
        Ok(())
    } else {
        Err(last_win32_error())
    }
}

// =============================================================================
// Native safety menu
// =============================================================================

unsafe fn show_safety_menu(hwnd: HWND) {
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
        error!("Tray: failed to create popup menu");

        return;
    };

    // -------------------------------------------------------------------------
    // Show
    // -------------------------------------------------------------------------

    let _ = unsafe { AppendMenuW(menu, MF_STRING, MENU_SHOW, w!("Show Nera")) };

    // -------------------------------------------------------------------------
    // Hide
    // -------------------------------------------------------------------------

    let _ = unsafe { AppendMenuW(menu, MF_STRING, MENU_HIDE, w!("Hide Nera")) };

    // -------------------------------------------------------------------------
    // Reset Position
    // -------------------------------------------------------------------------

    let _ = unsafe { AppendMenuW(menu, MF_STRING, MENU_RESET_POSITION, w!("Reset Position")) };

    // -------------------------------------------------------------------------
    // Always On Top
    // -------------------------------------------------------------------------

    let _ = unsafe { AppendMenuW(menu, MF_STRING, MENU_ALWAYS_ON_TOP, w!("Always On Top")) };

    // -------------------------------------------------------------------------
    // Separator
    // -------------------------------------------------------------------------

    let _ = unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) };
    let _ = unsafe { AppendMenuW(menu, MF_STRING, MENU_SCREENSHOT, w!("Screenshot")) };
    let _ = unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) };

    // -------------------------------------------------------------------------
    // Exit
    // -------------------------------------------------------------------------

    let _ = unsafe { AppendMenuW(menu, MF_STRING, MENU_EXIT, w!("Exit")) };

    let mut cursor = POINT::default();

    if unsafe { GetCursorPos(&mut cursor) }.is_err() {
        let _ = unsafe { DestroyMenu(menu) };

        return;
    }

    info!("Tray: opening safety menu at {}, {}", cursor.x, cursor.y,);

    // Diperlukan agar popup menu notification-area
    // mendapatkan foreground behavior yang benar.
    let _ = unsafe { SetForegroundWindow(hwnd) };

    let selected = unsafe {
        TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
            cursor.x,
            cursor.y,
            None,
            hwnd,
            None,
        )
    };

    let command = selected.0 as usize;

    // Recommended Win32 tray-menu cleanup behavior.
    let _ = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };

    let _ = unsafe { DestroyMenu(menu) };

    // -------------------------------------------------------------------------
    // Handle menu command
    // -------------------------------------------------------------------------

    if let Some(command) = command_for_menu_id(command) {
        send_tray_command(command);
        if command == TrayCommand::Exit {
            let _ = unsafe { DestroyWindow(hwnd) };
        }
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
    // Phase 5.5: Nera Action Menu
    // =========================================================================
    if message == WM_SHOW_NERA_ACTION_MENU {
        // The coordinates originated as i32 screen pixels. The cast through
        // WPARAM/LPARAM preserves the low 32-bit two's-complement value, so
        // negative coordinates on left/top secondary monitors survive.
        let x = wparam.0 as i32;
        let y = lparam.0 as i32;

        if let Err(error) = unsafe { show_nera_action_menu(hwnd, x, y) } {
            error!("Nera radial action menu error: {error:?}");
            send_action_menu_result(None);
        }

        // The radial HWND is non-blocking. Its own WndProc sends the eventual
        // typed result/cancel event through ACTION_MENU_SENDER.
        return LRESULT(0);
    }
    // =========================================================================
    // Explorer.exe restarted
    // =========================================================================

    let taskbar_msg = TASKBAR_CREATED_MSG.load(Ordering::Relaxed);

    if taskbar_msg != 0 && message == taskbar_msg {
        info!("Explorer restarted; re-registering Nera tray icon");

        if let Err(error) = unsafe { add_tray_icon(hwnd) } {
            error!("Failed to re-register Nera tray icon: {:?}", error);
        }

        return LRESULT(0);
    }

    // =========================================================================
    // Shell notification-area callback
    // =========================================================================

    if message == WM_NERA_TRAY {
        let event = lparam.0 as u32;

        match event {
            // -----------------------------------------------------------------
            // Left click
            // -----------------------------------------------------------------
            WM_LBUTTONUP => {
                info!("Tray left click");

                send_tray_command(TrayCommand::Show);
            }

            // -----------------------------------------------------------------
            // Right click
            // -----------------------------------------------------------------
            WM_RBUTTONUP => {
                info!("Tray right click");

                unsafe {
                    show_safety_menu(hwnd);
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
            let _ = unsafe { DestroyWindow(hwnd) };

            LRESULT(0)
        }

        WM_DESTROY => {
            info!("Destroying Nera tray");

            unsafe {
                remove_tray_icon(hwnd);

                PostQuitMessage(0);
            }

            LRESULT(0)
        }

        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

// =============================================================================
// Win32 tray thread
// =============================================================================

pub(super) fn run_tray_thread(hidden_hwnd: Arc<AtomicIsize>) -> Result<()> {
    run_with_initial_registration(hidden_hwnd, |hwnd| unsafe { add_tray_icon(hwnd) })
}

fn run_with_initial_registration(
    hidden_hwnd: Arc<AtomicIsize>,
    register: impl FnOnce(HWND) -> Result<()>,
) -> Result<()> {
    let module = unsafe { GetModuleHandleW(None)? };

    let instance = HINSTANCE(module.0);

    let class_name = w!("NeraTrayWindowClass");

    // =========================================================================
    // Register hidden window class
    // =========================================================================

    let window_class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,

        hInstance: instance,

        lpszClassName: class_name,

        lpfnWndProc: Some(tray_window_proc),

        ..Default::default()
    };

    let atom = unsafe { RegisterClassExW(&window_class) };

    if atom == 0 {
        return Err(last_win32_error());
    }

    // =========================================================================
    // Create hidden Win32 window
    // =========================================================================

    let hwnd = unsafe {
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

    hidden_hwnd.store(hwnd.0 as isize, Ordering::Release);

    // =========================================================================
    // Register shell recovery before attempting the icon. Explorer can still
    // be starting when Windows launches a Start Hidden instance.
    // =========================================================================

    let taskbar_msg = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    TASKBAR_CREATED_MSG.store(taskbar_msg, Ordering::Relaxed);

    if let Err(error) = register(hwnd) {
        warn!(
            "Initial tray registration failed: {error}; keeping callback alive for Explorer recovery"
        );
    }

    // =========================================================================
    // Win32 message loop
    // =========================================================================

    let mut message = MSG::default();

    loop {
        let result = unsafe { GetMessageW(&mut message, None, 0, 0) };

        // GetMessage:
        //
        // > 0 = normal message
        //   0 = WM_QUIT
        //  -1 = failure

        if result.0 == -1 {
            let error = last_win32_error();

            hidden_hwnd.store(0, Ordering::Release);

            return Err(error);
        }

        if result.0 == 0 {
            break;
        }

        unsafe {
            let _ = TranslateMessage(&message);

            DispatchMessageW(&message);
        }
    }

    hidden_hwnd.store(0, Ordering::Release);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_utf16;

    #[test]
    fn initial_shell_failure_keeps_native_callback_alive_and_closes_cleanly() {
        use super::*;
        use std::time::{Duration, Instant};
        use windows::Win32::UI::WindowsAndMessaging::IsWindow;
        let handle = Arc::new(AtomicIsize::new(0));
        let shared = handle.clone();
        let thread = std::thread::spawn(move || {
            run_with_initial_registration(shared, |_| {
                Err(Error::from_hresult(HRESULT(0x80004005_u32 as i32)))
            })
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        while handle.load(Ordering::Acquire) == 0
            && !thread.is_finished()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        // Give the injected registration failure time to reach the real pump.
        std::thread::sleep(Duration::from_millis(50));
        let raw = handle.load(Ordering::Acquire);
        let alive = raw != 0 && unsafe { IsWindow(Some(HWND(raw as *mut _))) }.as_bool();
        if raw != 0 {
            unsafe { PostMessageW(Some(HWND(raw as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)) }
                .unwrap();
        }
        let result = thread.join().unwrap();
        assert!(
            alive,
            "initial shell failure must not destroy its recovery callback"
        );
        result.unwrap();
        assert_eq!(handle.load(Ordering::Acquire), 0);
    }

    #[test]
    fn utf16_buffer_is_truncated_and_null_terminated() {
        let mut destination = [u16::MAX; 5];
        write_utf16(&mut destination, "Nerabyte");
        assert_eq!(
            &destination[..4],
            &[b'N' as u16, b'e' as u16, b'r' as u16, b'a' as u16]
        );
        assert_eq!(destination[4], 0);
    }

    #[test]
    fn utf16_helper_clears_unused_buffer_space() {
        let mut destination = [u16::MAX; 6];
        write_utf16(&mut destination, "N");
        assert_eq!(destination, [b'N' as u16, 0, 0, 0, 0, 0]);
    }
}
