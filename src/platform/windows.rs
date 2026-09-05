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

use windows::core::BOOL;

use windows::Win32::{
    Foundation::{
        HWND,
        LPARAM,
        POINT,
        RECT,
    },

    Graphics::{
        Dwm::{
            DwmGetWindowAttribute,
            DWMWA_CLOAKED,
        },

        Gdi::{
            EnumDisplayMonitors,
            HDC,
            HMONITOR,
            GetMonitorInfoW,
            MonitorFromWindow,
            MONITORINFO,
            MONITOR_DEFAULTTONEAREST,
        },
    },

    UI::{
        HiDpi::GetDpiForWindow,

        WindowsAndMessaging::{
            GetWindow,
            GW_HWNDPREV,
            GW_HWNDNEXT,
            EnumWindows,
            GA_ROOT,
            GetAncestor,
            GetClassNameW,
            GetCursorPos,
            GetShellWindow,
            GetWindowLongPtrW,
            GetWindowRect,
            GetWindowTextW,
            GWL_EXSTYLE,
            IsIconic,
            IsWindow,
            IsWindowVisible,
            SetWindowPos,
            WindowFromPoint,
            SWP_NOACTIVATE,
            SWP_NOSIZE,
            SWP_NOZORDER,
            WS_EX_TOOLWINDOW,
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


#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeMonitorHandle(isize);

impl NativeMonitorHandle {
    pub const fn from_raw(raw: isize) -> Self {
        Self(raw)
    }

    pub const fn as_raw(self) -> isize {
        self.0
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

    pub const fn intersects(
    self,
    other: PhysicalRect,
    ) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.top < other.bottom
            && self.bottom > other.top
    }

    pub const fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub const fn contains(self, point: PhysicalPoint) -> bool {
        point.x >= self.left
            && point.x < self.right
            && point.y >= self.top
            && point.y < self.bottom
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
    pub handle: NativeMonitorHandle,
    pub bounds: PhysicalRect,
    pub work_area: PhysicalRect,
    pub is_primary: bool,
}


// -----------------------------------------------------------------------------
// Desktop Windows
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct DesktopWindow {
    pub handle: NativeWindowHandle,
    pub rect: PhysicalRect,
    pub title: String,
    pub class_name: String,
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZOrderWindow {
    pub handle: NativeWindowHandle,
    pub rect: PhysicalRect,
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


// -----------------------------------------------------------------------------
// Window Discovery
// -----------------------------------------------------------------------------

struct EnumWindowsContext {
    nera_hwnd: NativeWindowHandle,
    windows: Vec<DesktopWindow>,
}

fn native_handle_from_hwnd(hwnd: HWND) -> NativeWindowHandle {
    NativeWindowHandle::from_raw(hwnd.0 as isize)
}

fn window_title(hwnd: HWND) -> String {
    let mut buffer = [0u16; 512];

    let length = unsafe {
        GetWindowTextW(
            hwnd,
            &mut buffer,
        )
    };

    if length <= 0 {
        return String::new();
    }

    String::from_utf16_lossy(
        &buffer[..length as usize],
    )
}

fn window_class_name(hwnd: HWND) -> String {
    let mut buffer = [0u16; 256];

    let length = unsafe {
        GetClassNameW(
            hwnd,
            &mut buffer,
        )
    };

    if length <= 0 {
        return String::new();
    }

    String::from_utf16_lossy(
        &buffer[..length as usize],
    )
}

fn is_window_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;

    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            size_of::<u32>() as u32,
        )
    };

    result.is_ok() && cloaked != 0
}

fn is_system_window_class(class_name: &str) -> bool {
    matches!(
        class_name,
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "DV2ControlHost"
            | "SysShadow"
            | "tooltips_class32"
    )
}

fn is_window_candidate(
    hwnd: HWND,
    nera_hwnd: NativeWindowHandle,
) -> bool {
    // Null / invalid HWND.
    if hwnd.0.is_null() {
        return false;
    }

    // Jangan pernah anggap Nera sendiri sebagai target.
    if hwnd == nera_hwnd.hwnd() {
        return false;
    }

    // HWND masih hidup?
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return false;
    }

    // Window harus visible.
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return false;
    }

    // Jangan ambil shell desktop.
    let shell_window = unsafe {
        GetShellWindow()
    };

    if hwnd == shell_window {
        return false;
    }

    // Kita cuma mau root top-level window.
    let root = unsafe {
        GetAncestor(
            hwnd,
            GA_ROOT,
        )
    };

    if root != hwnd {
        return false;
    }

    // Skip UWP/internal windows yang sedang cloaked.
    if is_window_cloaked(hwnd) {
        return false;
    }

    // Skip utility/tool windows.
    let ex_style = unsafe {
        GetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
        )
    } as u32;

    if (ex_style & WS_EX_TOOLWINDOW.0) != 0 {
        return false;
    }

    let class_name =
        window_class_name(hwnd);

    if is_system_window_class(&class_name) {
        return false;
    }

    let handle =
        native_handle_from_hwnd(hwnd);

    let Some(rect) =
        window_rect(handle)
    else {
        return false;
    };

    // Minimized/invalid/internal windows kadang menghasilkan
    // geometry yang tidak berguna.
    if rect.width() <= 1
        || rect.height() <= 1
    {
        return false;
    }

    true
}

fn desktop_window_from_hwnd(
    hwnd: HWND,
    nera_hwnd: NativeWindowHandle,
) -> Option<DesktopWindow> {
    if !is_window_candidate(
        hwnd,
        nera_hwnd,
    ) {
        return None;
    }

    let handle =
        native_handle_from_hwnd(hwnd);

    let rect =
        window_rect(handle)?;

    Some(DesktopWindow {
        handle,
        rect,
        title: window_title(hwnd),
        class_name: window_class_name(hwnd),
    })
}

unsafe extern "system" fn enum_windows_proc(
    hwnd: HWND,
    lparam: LPARAM,
) -> BOOL {
    let context = unsafe {
        &mut *(lparam.0 as *mut EnumWindowsContext)
    };

    if let Some(window) =
        desktop_window_from_hwnd(
            hwnd,
            context.nera_hwnd,
        )
    {
        context.windows.push(window);
    }

    BOOL::from(true)
}

pub fn enumerate_windows(
    nera_hwnd: NativeWindowHandle,
) -> windows::core::Result<Vec<DesktopWindow>> {
    let mut context =
        EnumWindowsContext {
            nera_hwnd,
            windows: Vec::new(),
        };

    unsafe {
        EnumWindows(
            Some(enum_windows_proc),
            LPARAM(
                (&mut context as *mut EnumWindowsContext)
                    as isize,
            ),
        )?;
    }

    Ok(context.windows)
}

fn direct_window_at_point(
    point: PhysicalPoint,
    nera_hwnd: NativeWindowHandle,
) -> Option<NativeWindowHandle> {
    let hwnd = unsafe {
        WindowFromPoint(
            POINT {
                x: point.x,
                y: point.y,
            },
        )
    };

    if hwnd.0.is_null() {
        return None;
    }

    let root = unsafe {
        GetAncestor(
            hwnd,
            GA_ROOT,
        )
    };

    if root.0.is_null() {
        return None;
    }

    if !is_window_candidate(
        root,
        nera_hwnd,
    ) {
        return None;
    }

    Some(native_handle_from_hwnd(root))
}

pub fn window_at_point(
    point: PhysicalPoint,
    nera_hwnd: NativeWindowHandle,
    windows: &[DesktopWindow],
) -> Option<NativeWindowHandle> {
    // Fast/native path.
    if let Some(handle) =
        direct_window_at_point(
            point,
            nera_hwnd,
        )
    {
        return Some(handle);
    }

    // Fallback:
    //
    // Kalau WindowFromPoint menemukan Nera sendiri,
    // cari candidate berikutnya dari snapshot desktop.
    windows
        .iter()
        .find(|window| {
            window.handle != nera_hwnd
                && window.rect.contains(point)
        })
        .map(|window| window.handle)
}


// -----------------------------------------------------------------------------
// Z-Order
// -----------------------------------------------------------------------------

const MAX_Z_ORDER_SCAN: usize = 256;

fn z_order_window_from_hwnd(
    hwnd: HWND,
    nera_hwnd: NativeWindowHandle,
) -> Option<ZOrderWindow> {
    if hwnd.0.is_null() {
        return None;
    }

    if hwnd == nera_hwnd.hwnd() {
        return None;
    }

    if !is_window_candidate(
        hwnd,
        nera_hwnd,
    ) {
        return None;
    }

    let handle =
        native_handle_from_hwnd(hwnd);

    // Minimized window tidak mungkin secara visual
    // menutupi target.
    if is_window_minimized(handle) {
        return None;
    }

    let rect =
        window_rect(handle)?;

    Some(ZOrderWindow {
        handle,
        rect,
    })
}


pub fn windows_above_target(
    target: NativeWindowHandle,
    nera_hwnd: NativeWindowHandle,
) -> Vec<ZOrderWindow> {
    let mut result = Vec::new();

    let mut current =
        target.hwnd();

    for _ in 0..MAX_Z_ORDER_SCAN {
        let Ok(next) = (unsafe {
            GetWindow(
                current,
                GW_HWNDPREV,
            )
        }) else {
            break;
        };

        if next.0.is_null()
            || next == current
        {
            break;
        }

        current = next;

        if let Some(window) =
            z_order_window_from_hwnd(
                next,
                nera_hwnd,
            )
        {
            result.push(window);
        }
    }

    result
}

pub fn window_below_target(
    target: NativeWindowHandle,
    nera_hwnd: NativeWindowHandle,
) -> Option<ZOrderWindow> {
    let mut current =
        target.hwnd();

    for _ in 0..MAX_Z_ORDER_SCAN {
        let Ok(next) = (unsafe {
            GetWindow(
                current,
                GW_HWNDNEXT,
            )
        }) else {
            break;
        };

        if next.0.is_null()
            || next == current
        {
            break;
        }

        current = next;

        if let Some(window) =
            z_order_window_from_hwnd(
                next,
                nera_hwnd,
            )
        {
            return Some(window);
        }
    }

    None
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


pub fn is_window_alive(
    handle: NativeWindowHandle,
) -> bool {
    unsafe {
        IsWindow(
            Some(handle.hwnd()),
        )
        .as_bool()
    }
}

pub fn is_window_minimized(
    handle: NativeWindowHandle,
) -> bool {
    unsafe {
        IsIconic(
            handle.hwnd(),
        )
        .as_bool()
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
fn monitor_info_from_hmonitor(
    monitor: HMONITOR,
) -> Option<MonitorInfo> {
    if monitor.0.is_null() {
        return None;
    }

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
        handle: NativeMonitorHandle::from_raw(
            monitor.0 as isize,
        ),

        bounds: info.rcMonitor.into(),

        work_area: info.rcWork.into(),

        is_primary:
            (info.dwFlags & MONITOR_PRIMARY_FLAG) != 0,
    })
}

pub fn monitor_info(
    handle: NativeWindowHandle,
) -> Option<MonitorInfo> {
    let monitor = unsafe {
        MonitorFromWindow(
            handle.hwnd(),
            MONITOR_DEFAULTTONEAREST,
        )
    };

    monitor_info_from_hmonitor(monitor)
}

// -----------------------------------------------------------------------------
// Monitor Enumeration
// -----------------------------------------------------------------------------

struct EnumMonitorsContext {
    monitors: Vec<MonitorInfo>,
}

unsafe extern "system" fn enum_monitors_proc(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let context = unsafe {
        &mut *(
            lparam.0
                as *mut EnumMonitorsContext
        )
    };

    if let Some(info) =
        monitor_info_from_hmonitor(
            monitor,
        )
    {
        context.monitors.push(info);
    }

    BOOL::from(true)
}

pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    let mut context = EnumMonitorsContext {
        monitors: Vec::new(),
    };

    let success = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitors_proc),
            LPARAM(
                (&mut context as *mut EnumMonitorsContext)
                    as isize,
            ),
        )
    };

    if !success.as_bool() {
        return Vec::new();
    }

    context.monitors
}

