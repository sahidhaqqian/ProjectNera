use std::{
    ffi::c_void,
    mem::size_of,
    sync::atomic::{AtomicU32, Ordering},
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
        SYSTEMTIME,
    },

    Graphics::{
        Dwm::{
            DwmGetWindowAttribute,
            DWMWA_CLOAKED,
            DWMWA_EXTENDED_FRAME_BOUNDS,
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

    System::SystemInformation::GetLocalTime,

    UI::{
        HiDpi::GetDpiForWindow,

        WindowsAndMessaging::{
            GetWindow,
            GW_HWNDFIRST,
            GW_HWNDNEXT,
            GW_HWNDPREV,
            EnumWindows,
            GA_ROOT,
            GetAncestor,
            GetClassNameW,
            GetCursorPos,
            GetForegroundWindow,
            GetShellWindow,
            GetWindowLongPtrW,
            GetWindowRect,
            GetWindowThreadProcessId,
            GWL_EXSTYLE,
            GWL_STYLE,
            IsIconic,
            IsWindow,
            IsWindowVisible,
            SetWindowPos,
            WindowFromPoint,
            SWP_NOACTIVATE,
            SWP_NOMOVE,
            SWP_NOSIZE,
            SWP_NOZORDER,
            HWND_NOTOPMOST,
            HWND_TOPMOST,
            WS_EX_TOOLWINDOW,
            WS_EX_TOPMOST,
            WS_CAPTION,
            WS_MAXIMIZE,
        },
    },
};

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

pub const BASE_DPI: u32 = 96;
const MONITOR_PRIMARY_FLAG: u32 = 1;

// The gallery has its own renderer process. Track only the child we own so its
// windows cannot become pet supports or trigger fullscreen DND either.
static ANIMATION_MANAGER_PROCESS_ID: AtomicU32 = AtomicU32::new(0);

pub(crate) fn set_animation_manager_process_id(process_id: Option<u32>) {
    ANIMATION_MANAGER_PROCESS_ID.store(process_id.unwrap_or(0), Ordering::Relaxed);
}

fn is_external_application_process(
    process_id: u32,
    own_process_id: u32,
    manager_process_id: u32,
) -> bool {
    process_id != 0
        && own_process_id != 0
        && process_id != own_process_id
        && process_id != manager_process_id
}
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

/// Small platform-neutral projection of Windows' local wall clock.
///
/// Alarm UI needs wall-clock text, but pulling a date/time crate only to render
/// one utility panel would be unnecessary. `GetLocalTime` already applies the
/// user's Windows timezone and daylight-saving rules for us.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalDateTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    /// Windows convention: Sunday = 0 ... Saturday = 6.
    pub day_of_week: u16,
    pub hour: u16,
    pub minute: u16,
}

/// Current Windows-local date/time.
pub fn local_datetime() -> LocalDateTime {
    let value: SYSTEMTIME = unsafe { GetLocalTime() };

    LocalDateTime {
        year: value.wYear,
        month: value.wMonth,
        day: value.wDay,
        day_of_week: value.wDayOfWeek,
        hour: value.wHour,
        minute: value.wMinute,
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
    // Prefer what the user actually sees. GetWindowRect can include invisible
    // resize borders on modern Windows, which makes a desktop pet appear to
    // hover a few pixels above the title bar when snapping to `rect.top`.
    let mut visible_rect = RECT::default();

    let visible_result = unsafe {
        DwmGetWindowAttribute(
            handle.hwnd(),
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut visible_rect as *mut RECT).cast(),
            size_of::<RECT>() as u32,
        )
    };

    if visible_result.is_ok()
        && visible_rect.right > visible_rect.left
        && visible_rect.bottom > visible_rect.top
    {
        return Some(visible_rect.into());
    }

    // DWM attributes are not guaranteed for every HWND. Fall back to the
    // classic API rather than making window discovery/tracking fail.
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

fn is_hwnd_cloaked(hwnd: HWND) -> bool {
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

/// True when DWM currently hides this window while keeping the HWND alive.
/// This commonly happens when the window belongs to another virtual desktop.
pub fn is_window_cloaked(handle: NativeWindowHandle) -> bool {
    is_hwnd_cloaked(handle.hwnd())
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

    let mut process_id = 0;
    let mut own_process_id = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        GetWindowThreadProcessId(nera_hwnd.hwnd(), Some(&mut own_process_id));
    }
    // Settings, alarm panels, and other Nera utilities are not desktop support
    // surfaces even if they are ordinary top-level windows without TOOLWINDOW.
    if !is_external_application_process(
        process_id,
        own_process_id,
        ANIMATION_MANAGER_PROCESS_ID.load(Ordering::Relaxed),
    ) {
        return false;
    }

    // Window harus visible.
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return false;
    }

    if unsafe { IsIconic(hwnd) }.as_bool() {
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
    if is_hwnd_cloaked(hwnd) {
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
    _windows: &[DesktopWindow],
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

    // Nera can obscure the native hit-test. Walk live top-level z-order until
    // the first eligible containing window rather than trusting discovery's
    // older positions/order while a supporting window is dragged or covered.
    let mut candidate = unsafe { GetWindow(nera_hwnd.hwnd(), GW_HWNDFIRST) }.ok()?;
    // HWNDs may disappear or be reordered during traversal. A bounded walk
    // avoids a pathological cycle and conservatively denies support on failure.
    for _ in 0..512 {
        if candidate.0.is_null() {
            break;
        }
        if let Some(window) = desktop_window_from_hwnd(candidate, nera_hwnd) {
            if window.rect.contains(point) {
                return Some(window.handle);
            }
        }
        let Some(next) = unsafe { GetWindow(candidate, GW_HWNDNEXT) }.ok() else {
            break;
        };
        if next == candidate {
            break;
        }
        candidate = next;
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
/// True only once Windows reports the HWND as actually visible.
///
/// Bevy's `Window::visible` is desired state; the native window can lag behind
/// it by a frame while winit applies the change. Lifecycle code that must prove
/// an affordance is reachable should use this native observation instead.
pub fn window_is_visible(handle: NativeWindowHandle) -> bool {
    unsafe { IsWindowVisible(handle.hwnd()) }.as_bool()
}


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


/// Reassert an HWND at the front of the Windows topmost band without moving,
/// resizing, or activating it. Taskbar sitting uses this because Shell_TrayWnd
/// can otherwise visually cover part of an always-on-top pet after the taskbar
/// itself is clicked.
pub fn place_window_topmost_noactivate(
    handle: NativeWindowHandle,
) -> windows::core::Result<()> {
    unsafe {
        SetWindowPos(
            handle.hwnd(),
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    }
}


/// Keep Nera immediately above a normal target window without making Nera
/// globally topmost. Windows that are already above the target remain above
/// Nera, which gives Phase 4 natural desktop occlusion.
pub fn place_window_directly_above(
    nera: NativeWindowHandle,
    target: NativeWindowHandle,
) -> windows::core::Result<()> {
    if nera == target {
        return Ok(());
    }

    let nera_hwnd = nera.hwnd();
    let target_hwnd = target.hwnd();

    // WindowLevel::Normal is the Bevy-side source of truth, but the native
    // style can lag by a frame after toggling Always On Top. Explicitly cross
    // the topmost/non-topmost boundary before doing target-relative ordering.
    let ex_style = unsafe { GetWindowLongPtrW(nera_hwnd, GWL_EXSTYLE) } as u32;

    if (ex_style & WS_EX_TOPMOST.0) != 0 {
        unsafe {
            SetWindowPos(
                nera_hwnd,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )?;
        }
    }

    let above = unsafe { GetWindow(target_hwnd, GW_HWNDPREV) }
        .ok()
        .filter(|hwnd| !hwnd.0.is_null());

    // If Nera is already the immediate predecessor, the relation is correct.
    if above == Some(nera_hwnd) {
        return Ok(());
    }

    unsafe {
        SetWindowPos(
            nera_hwnd,
            above,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
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

/// Whether an external foreground application covers the physical monitor.
/// The monitor work area is deliberately not used: a normally maximized app
/// that still leaves the taskbar visible must not activate do-not-disturb.
pub fn foreground_fullscreen_on_monitor(
    own_hwnd: NativeWindowHandle,
    monitor: MonitorInfo,
) -> bool {
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.0.is_null() || !unsafe { IsWindow(Some(foreground)) }.as_bool() {
        return false;
    }
    let mut foreground_process = 0;
    let mut own_process = 0;
    unsafe {
        GetWindowThreadProcessId(foreground, Some(&mut foreground_process));
        GetWindowThreadProcessId(own_hwnd.hwnd(), Some(&mut own_process));
    }
    // Failure to identify the process cannot establish an external app. This
    // also excludes every Nera utility HWND and our gallery child process.
    if !is_external_application_process(
        foreground_process,
        own_process,
        ANIMATION_MANAGER_PROCESS_ID.load(Ordering::Relaxed),
    ) {
        return false;
    }
    let Some(rect) = window_rect(native_handle_from_hwnd(foreground)) else {
        return false;
    };
    let Some(current_monitor) =
        monitor_info_from_hmonitor(HMONITOR(monitor.handle.as_raw() as *mut c_void))
    else {
        return false;
    };
    let style = unsafe { GetWindowLongPtrW(foreground, GWL_STYLE) } as u32;
    let class_name = window_class_name(foreground);
    fullscreen_candidate(FullscreenCandidate {
        rect,
        monitor_bounds: current_monitor.bounds,
        class_name: &class_name,
        own_process: foreground_process == own_process,
        shell: foreground == unsafe { GetShellWindow() },
        visible: unsafe { IsWindowVisible(foreground) }.as_bool(),
        minimized: unsafe { IsIconic(foreground) }.as_bool(),
        cloaked: is_hwnd_cloaked(foreground),
        // Auto-hidden taskbars can make work_area == monitor.bounds. A normal
        // captioned maximized window still should not count as fullscreen.
        captioned_maximized: (style & WS_MAXIMIZE.0) != 0 && (style & WS_CAPTION.0) == WS_CAPTION.0,
    })
}

struct FullscreenCandidate<'a> {
    rect: PhysicalRect,
    monitor_bounds: PhysicalRect,
    class_name: &'a str,
    own_process: bool,
    shell: bool,
    visible: bool,
    minimized: bool,
    cloaked: bool,
    captioned_maximized: bool,
}

fn fullscreen_candidate(candidate: FullscreenCandidate<'_>) -> bool {
    if candidate.own_process
        || candidate.shell
        || !candidate.visible
        || candidate.minimized
        || candidate.cloaked
        || candidate.captioned_maximized
        || is_system_window_class(candidate.class_name)
    {
        return false;
    }
    let rect = candidate.rect;
    let bounds = candidate.monitor_bounds;
    // Permit a tiny DWM rounding discrepancy, including on mixed-DPI monitors,
    // while requiring coverage of all four physical screen edges.
    const TOLERANCE: i32 = 2;
    bounds.width() > 0
        && bounds.height() > 0
        && rect.width() > 0
        && rect.height() > 0
        && rect.left <= bounds.left.saturating_add(TOLERANCE)
        && rect.top <= bounds.top.saturating_add(TOLERANCE)
        && rect.right >= bounds.right.saturating_sub(TOLERANCE)
        && rect.bottom >= bounds.bottom.saturating_sub(TOLERANCE)
}

#[cfg(test)]
mod fullscreen_tests {
    use super::*;

    #[test]
    fn desktop_windows_exclude_own_utility_and_gallery_processes() {
        assert!(!is_external_application_process(42, 42, 57));
        assert!(!is_external_application_process(57, 42, 57));
        assert!(!is_external_application_process(0, 42, 57));
        assert!(!is_external_application_process(88, 0, 57));
        assert!(is_external_application_process(88, 42, 57));
        // Closing the gallery releases its PID; absence is represented by zero.
        assert!(is_external_application_process(57, 42, 0));
    }

    fn candidate() -> FullscreenCandidate<'static> {
        FullscreenCandidate {
            rect: PhysicalRect {
                left: -1920,
                top: 1080,
                right: 0,
                bottom: 2160,
            },
            monitor_bounds: PhysicalRect {
                left: -1920,
                top: 1080,
                right: 0,
                bottom: 2160,
            },
            class_name: "GameWindow",
            own_process: false,
            shell: false,
            visible: true,
            minimized: false,
            cloaked: false,
            captioned_maximized: false,
        }
    }

    #[test]
    fn fullscreen_uses_physical_bounds_and_current_monitor_only() {
        assert!(fullscreen_candidate(candidate()));
        let mut maximized = candidate();
        maximized.rect.bottom -= 40;
        assert!(!fullscreen_candidate(maximized));
        let mut other_monitor = candidate();
        other_monitor.rect = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        assert!(!fullscreen_candidate(other_monitor));
        let mut rounding = candidate();
        rounding.rect.left += 2;
        rounding.rect.bottom -= 2;
        assert!(fullscreen_candidate(rounding));
    }

    #[test]
    fn own_utility_windows_shell_and_suspended_apps_are_excluded() {
        for flag in 0..6 {
            let mut excluded = candidate();
            match flag {
                0 => excluded.own_process = true,
                1 => excluded.shell = true,
                2 => excluded.visible = false,
                3 => excluded.minimized = true,
                4 => excluded.cloaked = true,
                _ => excluded.captioned_maximized = true,
            }
            assert!(!fullscreen_candidate(excluded));
        }
        for class_name in [
            "Progman",
            "WorkerW",
            "Shell_TrayWnd",
            "Shell_SecondaryTrayWnd",
        ] {
            assert!(!fullscreen_candidate(FullscreenCandidate {
                class_name,
                ..candidate()
            }));
        }
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
