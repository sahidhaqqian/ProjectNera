use std::{
    ffi::c_void,
    mem::size_of,
};

use bevy::{
    prelude::Entity,
    winit::WINIT_WINDOWS,
};

use raw_window_handle::{
    HasWindowHandle,
    RawWindowHandle,
};

use windows::Win32::{
    Foundation::{
        HWND,
        POINT,
        RECT,
    },
    Graphics::Gdi::{
        GetMonitorInfoW,
        MonitorFromWindow,
        MONITORINFO,
        MONITOR_DEFAULTTONEAREST,
    },
    UI::{
        HiDpi::GetDpiForWindow,
        WindowsAndMessaging::{
            GetCursorPos,
            GetWindowRect,
            SetWindowPos,
            SWP_NOACTIVATE,
            SWP_NOSIZE,
            SWP_NOZORDER,
        },
    },
};

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

pub const BASE_DPI: u32 = 96;
const MONITOR_PRIMARY_FLAG: u32 = 1;
// -----------------------------------------------------------------------------
// Native Handle
// -----------------------------------------------------------------------------

/// HWND versi Nera.
///
/// Kita sengaja tidak menyimpan `windows::Win32::Foundation::HWND`
/// langsung di Bevy Resource.
///
/// Yang disimpan hanya nilai pointer/handle native-nya.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeWindowHandle(isize);

impl NativeWindowHandle {
    pub const fn from_raw(raw: isize) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> isize {
        self.0
    }

    fn hwnd(self) -> HWND {
        HWND(self.0 as *mut c_void)
    }
}

// -----------------------------------------------------------------------------
// Native Geometry
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhysicalPoint {
    pub x: i32,
    pub y: i32,
}

impl PhysicalPoint {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhysicalRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PhysicalRect {
    pub const fn width(self) -> i32 {
        self.right - self.left
    }

    pub const fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub const fn origin(self) -> PhysicalPoint {
        PhysicalPoint {
            x: self.left,
            y: self.top,
        }
    }
}

impl From<RECT> for PhysicalRect {
    fn from(rect: RECT) -> Self {
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

// -----------------------------------------------------------------------------
// Monitor
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorInfo {
    /// Seluruh area monitor.
    pub bounds: PhysicalRect,

    /// Area usable setelah taskbar/system UI dikurangi.
    pub work_area: PhysicalRect,

    pub is_primary: bool,
}

// -----------------------------------------------------------------------------
// HWND Acquisition
// -----------------------------------------------------------------------------

/// Mengambil HWND asli dari window Bevy/winit.
///
/// Fungsi ini hanya perlu dipakai ketika mendapatkan handle pertama kali.
/// Setelah itu Nera menyimpan `NativeWindowHandle`.
pub fn native_window_handle(entity: Entity) -> Option<NativeWindowHandle> {
    WINIT_WINDOWS.with(|windows| {
        let windows = windows.borrow();

        let window = windows.get_window(entity)?;

        let window_handle = window.window_handle().ok()?;

        match window_handle.as_raw() {
            RawWindowHandle::Win32(handle) => {
                Some(NativeWindowHandle::from_raw(handle.hwnd.get()))
            }

            _ => None,
        }
    })
}

// -----------------------------------------------------------------------------
// Cursor
// -----------------------------------------------------------------------------

pub fn global_cursor_position() -> Option<PhysicalPoint> {
    let mut point = POINT::default();

    unsafe {
        GetCursorPos(&mut point).ok()?;
    }

    Some(PhysicalPoint {
        x: point.x,
        y: point.y,
    })
}

// -----------------------------------------------------------------------------
// Window Geometry
// -----------------------------------------------------------------------------

pub fn window_rect(handle: NativeWindowHandle) -> Option<PhysicalRect> {
    let mut rect = RECT::default();

    unsafe {
        GetWindowRect(handle.hwnd(), &mut rect).ok()?;
    }

    Some(rect.into())
}

/// Pindahkan window tanpa:
///
/// - resize
/// - mengubah z-order
/// - mengambil fokus
///
/// Ini nantinya juga akan dipakai Phase 4 ketika Nera mengikuti target window.
pub fn move_window(
    handle: NativeWindowHandle,
    position: PhysicalPoint,
) -> windows::core::Result<()> {
    unsafe {
        SetWindowPos(
            handle.hwnd(),
            None,
            position.x,
            position.y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
}

// -----------------------------------------------------------------------------
// DPI
// -----------------------------------------------------------------------------

pub fn dpi_for_window(handle: NativeWindowHandle) -> u32 {
    let dpi = unsafe {
        GetDpiForWindow(handle.hwnd())
    };

    if dpi == 0 {
        BASE_DPI
    } else {
        dpi
    }
}

pub fn dpi_scale_factor(dpi: u32) -> f64 {
    dpi as f64 / BASE_DPI as f64
}

pub fn physical_to_logical(value: f64, dpi: u32) -> f64 {
    value / dpi_scale_factor(dpi)
}

pub fn logical_to_physical(value: f64, dpi: u32) -> f64 {
    value * dpi_scale_factor(dpi)
}

// -----------------------------------------------------------------------------
// Monitor Information
// -----------------------------------------------------------------------------

pub fn monitor_info(handle: NativeWindowHandle) -> Option<MonitorInfo> {
    let monitor = unsafe {
        MonitorFromWindow(
            handle.hwnd(),
            MONITOR_DEFAULTTONEAREST,
        )
    };

    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };

    let success = unsafe {
        GetMonitorInfoW(
            monitor,
            &mut info,
        )
    };

    if !success.as_bool() {
        return None;
    }

    Some(MonitorInfo {
        bounds: info.rcMonitor.into(),
        work_area: info.rcWork.into(),
        is_primary: (info.dwFlags & MONITOR_PRIMARY_FLAG) != 0,
    })
}
