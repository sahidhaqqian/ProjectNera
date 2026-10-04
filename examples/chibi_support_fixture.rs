//! Optional real external-process HWND for native Chibi window-sitting checks.
//! Build alongside chibi_validation; it closes itself after a bounded lifetime.
#[cfg(target_os = "windows")]
mod native {
    use std::{ffi::c_void, mem::size_of};
    use windows::{
        Win32::{
            Foundation::{COLORREF, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
            Graphics::Gdi::{
                BeginPaint, CreateSolidBrush, DEFAULT_GUI_FONT, DeleteObject, EndPaint,
                GetMonitorInfoW, GetStockObject, HGDIOBJ, MONITOR_DEFAULTTOPRIMARY, MONITORINFO,
                MonitorFromWindow, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
                TextOutW,
            },
            System::LibraryLoader::GetModuleHandleW,
            UI::WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
                HWND_TOPMOST, IDC_ARROW, KillTimer, LoadCursorW, MSG, PostQuitMessage,
                RegisterClassExW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetTimer, SetWindowPos,
                TranslateMessage, WM_CLOSE, WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSEXW, WS_BORDER,
                WS_CAPTION, WS_EX_TOPMOST, WS_SYSMENU,
            },
        },
        core::{Error, HRESULT, Result, w},
    };

    const CLOSE_TIMER: usize = 1;

    fn last_error() -> Error {
        Error::from_hresult(HRESULT::from_win32(unsafe { GetLastError() }.0))
    }

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_TIMER if wparam.0 == CLOSE_TIMER => {
                println!("CHIBI_SUPPORT_FIXTURE_TIMEOUT pid={}", std::process::id());
                let _ = unsafe { DestroyWindow(hwnd) };
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = unsafe { DestroyWindow(hwnd) };
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = unsafe { KillTimer(Some(hwnd), CLOSE_TIMER) };
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = unsafe { BeginPaint(hwnd, &mut paint) };
                let old = unsafe { SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT)) };
                let _ = unsafe { SetBkMode(dc, TRANSPARENT) };
                let _ = unsafe { SetTextColor(dc, COLORREF(0x00202020)) };
                for (line, text) in [
                    "Project Nera: Chibi validation support",
                    "Separate test process with a real visible window edge.",
                    "This fixture closes automatically; no user app is modified.",
                ]
                .iter()
                .enumerate()
                {
                    let text: Vec<u16> = text.encode_utf16().collect();
                    let _ = unsafe { TextOutW(dc, 20, 22 + line as i32 * 28, &text) };
                }
                let _ = unsafe { SelectObject(dc, old) };
                let _ = unsafe { EndPaint(hwnd, &paint) };
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    pub fn run() -> Result<()> {
        let lifetime = std::env::args()
            .find_map(|arg| {
                arg.strip_prefix("--lifetime-secs=")
                    .and_then(|value| value.parse::<u32>().ok())
            })
            .unwrap_or(180)
            .clamp(10, 600);
        let owner = std::env::args()
            .find_map(|arg| {
                arg.strip_prefix("--support-for-hwnd=")
                    .and_then(|value| value.parse::<isize>().ok())
            })
            .map_or(HWND::default(), |raw| HWND(raw as *mut c_void));
        let monitor = unsafe { MonitorFromWindow(owner, MONITOR_DEFAULTTOPRIMARY) };
        let mut monitor_info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
            return Err(last_error());
        }
        let work = monitor_info.rcWork;
        let work_width = (work.right - work.left).max(1);
        let work_height = (work.bottom - work.top).max(1);
        let width = (work_width * 55 / 100).clamp(420, 900).min(work_width);
        let height = (work_height * 24 / 100).clamp(160, 320).min(work_height);
        let x = work.left + (work_width - width) / 2;
        let y = (work.top + work_height * 60 / 100).min(work.bottom - height);
        let module = unsafe { GetModuleHandleW(None)? };
        let instance = HINSTANCE(module.0);
        let background = unsafe { CreateSolidBrush(COLORREF(0x00F2F2F2)) };
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            hInstance: instance,
            lpfnWndProc: Some(window_proc),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
            hbrBackground: background,
            lpszClassName: w!("NeraChibiSupportValidationClass"),
            ..Default::default()
        };
        if unsafe { RegisterClassExW(&class) } == 0 {
            return Err(last_error());
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOPMOST,
                w!("NeraChibiSupportValidationClass"),
                w!("Nera Chibi support validation fixture"),
                WS_CAPTION | WS_SYSMENU | WS_BORDER,
                x,
                y,
                width,
                height,
                None,
                None,
                Some(instance),
                None,
            )?
        };
        if unsafe { SetTimer(Some(hwnd), CLOSE_TIMER, lifetime * 1000, None) } == 0 {
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(last_error());
        }
        // The fixture owns this window only. NOACTIVATE keeps the foreground
        // app intact; topmost makes its edge visible above maximized apps.
        unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                width,
                height,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            )?
        };
        println!(
            "CHIBI_SUPPORT_FIXTURE_READY pid={} hwnd={} path={} work=({},{},{},{}) window=({},{},{},{}) lifetime={}s",
            std::process::id(),
            hwnd.0 as isize,
            std::env::current_exe()
                .map_or_else(|_| "unknown".into(), |path| path.display().to_string()),
            work.left,
            work.top,
            work.right,
            work.bottom,
            x,
            y,
            width,
            height,
            lifetime,
        );
        let mut message = MSG::default();
        loop {
            let status = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
            if status == 0 {
                break;
            }
            if status < 0 {
                return Err(last_error());
            }
            let _ = unsafe { TranslateMessage(&message) };
            unsafe { DispatchMessageW(&message) };
        }
        let _ = unsafe { DeleteObject(HGDIOBJ(background.0)) };
        println!("CHIBI_SUPPORT_FIXTURE_EXIT pid={}", std::process::id());
        Ok(())
    }
}

#[cfg(target_os = "windows")]
fn main() {
    if let Err(error) = native::run() {
        eprintln!("Chibi support fixture failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("The Chibi support validation fixture requires Windows.");
}
