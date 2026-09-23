//! Project Nera — Phase 5.5 radial action wheel.
//!
//! The old TrackPopupMenu prototype proved the command/lifecycle plumbing.
//! This module replaces only the presentation layer with a Mate-Engine-style
//! radial wheel while keeping:
//!
//! - Bevy main thread unblocked.
//! - Existing tray/native UI thread.
//! - Typed `NeraActionMenuCommand` messages.
//! - Praying while the wheel is open.
//! - Animation / Alarm command contracts.
//!
//! The wheel is a real transparent Win32 popup. Its center is color-key
//! transparent so Nera remains visible inside the ring.

use std::{
    ffi::c_void,
    f32::consts::{PI, TAU},
    mem::size_of,
    sync::{
        atomic::{AtomicIsize, Ordering},
        Mutex,
        OnceLock,
    },
};

use bevy::prelude::*;

use windows::{
    core::{w, Error, Result},
    Win32::{
        Foundation::{
            COLORREF,
            GetLastError,
            HINSTANCE,
            HWND,
            LPARAM,
            LRESULT,
            RECT,
            WPARAM,
        },
        Graphics::Gdi::{
            BeginPaint,
            CreatePen,
            CreateSolidBrush,
            DeleteObject,
            Ellipse,
            EndPaint,
            GetStockObject,
            HGDIOBJ,
            InvalidateRect,
            LineTo,
            MoveToEx,
            PAINTSTRUCT,
            PS_SOLID,
            SelectObject,
            SetBkMode,
            SetTextColor,
            TextOutW,
            DEFAULT_GUI_FONT,
            NULL_PEN,
            TRANSPARENT,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW,
            DefWindowProcW,
            DestroyWindow,
            RegisterClassExW,
            SetForegroundWindow,
            SetLayeredWindowAttributes,
            ShowWindow,
            LWA_COLORKEY,
            SW_SHOW,
            WM_DESTROY,
            WM_ERASEBKGND,
            WM_KEYDOWN,
            WM_KILLFOCUS,
            WM_LBUTTONUP,
            WM_MOUSEMOVE,
            WM_PAINT,
            WM_RBUTTONUP,
            WNDCLASSEXW,
            WS_EX_LAYERED,
            WS_EX_TOOLWINDOW,
            WS_EX_TOPMOST,
            WS_POPUP,
        },
    },
};

use crate::{
    alarm_settings::OpenAlarmSettingsWindow,
    animation::NeraAnimationState,
    interaction::NeraRightClicked,
    settings::{OpenSettingsWindow, ReturnToActionMenu},
    tray::{
        send_action_menu_result,
        TrayRuntime,
    },
};

// =============================================================================
// Typed commands — unchanged from the proven Phase 5.5 backend
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AnimationMenuChoice {
    Wave,
    Thinking,
    Angry,
    Sad,
    Surprised,
    ResetBasePose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AlarmMenuChoice {
    FiveMinutes,
    TenMinutes,
    ThirtyMinutes,
    FifteenMinutes,
    FortyFiveMinutes,
    OneHour,
    TwoHours,
    /// Rings immediately. Replaces the old Ctrl+Alt+A debug shortcut so the
    /// alarm can be auditioned without a keyboard hack.
    TestNow,
    Cancel,
}

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NeraActionMenuCommand {
    Animation(AnimationMenuChoice),
    Alarm(AlarmMenuChoice),
    AlarmSettings,
    Settings,
}

/// Posted to the existing tray HWND. The tray thread creates the radial popup,
/// then its normal GetMessage loop services both the tray HWND and wheel HWND.
pub(crate) const WM_SHOW_NERA_ACTION_MENU: u32 = 6003;

// =============================================================================
// Wheel geometry / palette
// =============================================================================

const VK_ESCAPE_RAW: u32 = 0x1B;

const WHEEL_SIZE: i32 = 390;
const WHEEL_CENTER: f32 = WHEEL_SIZE as f32 * 0.5;
const OUTER_RADIUS: f32 = 184.0;
const INNER_RADIUS: f32 = 94.0;
const ITEM_RADIUS: f32 = 139.0;
const HOVER_BUBBLE_RADIUS: i32 = 31;

/// Rare chroma key. Every pixel painted with this exact RGB becomes invisible.
const KEY_RGB: (u8, u8, u8) = (1, 2, 3);

const RING_RGB: (u8, u8, u8) = (16, 20, 34);
const RING_INNER_RGB: (u8, u8, u8) = (23, 28, 47);
const HOVER_RGB: (u8, u8, u8) = (46, 52, 79);
const LINE_RGB: (u8, u8, u8) = (48, 55, 80);
const ACCENT_RGB: (u8, u8, u8) = (156, 105, 255);
const TEXT_RGB: (u8, u8, u8) = (245, 244, 252);
const MUTED_RGB: (u8, u8, u8) = (108, 116, 145);

fn rgb(value: (u8, u8, u8)) -> COLORREF {
    COLORREF(
        value.0 as u32
            | ((value.1 as u32) << 8)
            | ((value.2 as u32) << 16),
    )
}

// =============================================================================
// Menu model
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RadialPage {
    Root,
    Animation,
    Alarm,
    /// Longer presets + "Test Now", reached from the Alarm page. Replaces the
    /// old `Custom` slot, which was an enabled menu item that did nothing but
    /// write a `warn!` to the log.
    AlarmMore,
}

#[derive(Clone, Copy, Debug)]
enum RadialItemAction {
    Disabled,
    Open(RadialPage),
    Back,
    Command(NeraActionMenuCommand),
}

#[derive(Clone, Copy, Debug)]
struct RadialItem {
    glyph: &'static str,
    label: &'static str,
    action: RadialItemAction,
}

impl RadialItem {
    fn enabled(self) -> bool {
        !matches!(self.action, RadialItemAction::Disabled)
    }
}

const ROOT_ITEMS: [RadialItem; 6] = [
    RadialItem {
        glyph: "●",
        label: "Chat",
        action: RadialItemAction::Disabled,
    },
    RadialItem {
        glyph: "⚙",
        label: "Settings",
        action: RadialItemAction::Command(NeraActionMenuCommand::Settings),
    },
    RadialItem {
        glyph: "▶",
        label: "Animation",
        action: RadialItemAction::Open(RadialPage::Animation),
    },
    RadialItem {
        glyph: "◷",
        label: "Alarm",
        action: RadialItemAction::Open(RadialPage::Alarm),
    },
    RadialItem {
        glyph: "✓",
        label: "Productivity",
        action: RadialItemAction::Disabled,
    },
    RadialItem {
        glyph: "◆",
        label: "Games",
        action: RadialItemAction::Disabled,
    },
];

const ANIMATION_ITEMS: [RadialItem; 7] = [
    RadialItem {
        glyph: "↑",
        label: "Wave",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Animation(AnimationMenuChoice::Wave),
        ),
    },
    RadialItem {
        glyph: "…",
        label: "Thinking",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Animation(AnimationMenuChoice::Thinking),
        ),
    },
    RadialItem {
        glyph: "!",
        label: "Angry",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Animation(AnimationMenuChoice::Angry),
        ),
    },
    RadialItem {
        glyph: "↓",
        label: "Sad",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Animation(AnimationMenuChoice::Sad),
        ),
    },
    RadialItem {
        glyph: "○",
        label: "Surprised",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Animation(AnimationMenuChoice::Surprised),
        ),
    },
    RadialItem {
        glyph: "↺",
        label: "Reset",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Animation(AnimationMenuChoice::ResetBasePose),
        ),
    },
    RadialItem {
        glyph: "‹",
        label: "Back",
        action: RadialItemAction::Back,
    },
];

const ALARM_ITEMS: [RadialItem; 6] = [
    RadialItem {
        glyph: "5",
        label: "5 min",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Alarm(AlarmMenuChoice::FiveMinutes),
        ),
    },
    RadialItem {
        glyph: "10",
        label: "10 min",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Alarm(AlarmMenuChoice::TenMinutes),
        ),
    },
    RadialItem {
        glyph: "30",
        label: "30 min",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Alarm(AlarmMenuChoice::ThirtyMinutes),
        ),
    },
    RadialItem {
        glyph: "›",
        label: "More",
        action: RadialItemAction::Open(RadialPage::AlarmMore),
    },
    RadialItem {
        glyph: "×",
        label: "Cancel",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Alarm(AlarmMenuChoice::Cancel),
        ),
    },
    RadialItem {
        glyph: "‹",
        label: "Back",
        action: RadialItemAction::Back,
    },
];

const ALARM_MORE_ITEMS: [RadialItem; 7] = [
    RadialItem {
        glyph: "15",
        label: "15 min",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Alarm(AlarmMenuChoice::FifteenMinutes),
        ),
    },
    RadialItem {
        glyph: "45",
        label: "45 min",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Alarm(AlarmMenuChoice::FortyFiveMinutes),
        ),
    },
    RadialItem {
        glyph: "1h",
        label: "1 hour",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Alarm(AlarmMenuChoice::OneHour),
        ),
    },
    RadialItem {
        glyph: "2h",
        label: "2 hours",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Alarm(AlarmMenuChoice::TwoHours),
        ),
    },
    RadialItem {
        glyph: "⚙",
        label: "Custom",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::AlarmSettings,
        ),
    },
    RadialItem {
        glyph: "!",
        label: "Test Now",
        action: RadialItemAction::Command(
            NeraActionMenuCommand::Alarm(AlarmMenuChoice::TestNow),
        ),
    },
    RadialItem {
        glyph: "‹",
        label: "Back",
        action: RadialItemAction::Back,
    },
];

fn items_for(page: RadialPage) -> &'static [RadialItem] {
    match page {
        RadialPage::Root => &ROOT_ITEMS,
        RadialPage::Animation => &ANIMATION_ITEMS,
        RadialPage::Alarm => &ALARM_ITEMS,
        RadialPage::AlarmMore => &ALARM_MORE_ITEMS,
    }
}

/// Page hierarchy for "go up one level".
///
/// `None` means the page has no parent, so going up closes the wheel instead.
/// Single source of truth: center-click, the Back item, right-click and Escape
/// all navigate through this, so a deeper page cannot accidentally dump the
/// user straight back to Root.
fn parent_page(page: RadialPage) -> Option<RadialPage> {
    match page {
        RadialPage::Root => None,
        RadialPage::Animation | RadialPage::Alarm => {
            Some(RadialPage::Root)
        }
        RadialPage::AlarmMore => Some(RadialPage::Alarm),
    }
}

fn navigate_back(hwnd: HWND, page: RadialPage) {
    match parent_page(page) {
        Some(parent) => set_page(hwnd, parent),
        None => finish_menu(hwnd, None),
    }
}

// =============================================================================
// Native wheel state
// =============================================================================

#[derive(Clone, Copy, Debug)]
struct NativeRadialState {
    page: RadialPage,
    hover: Option<usize>,
    result_sent: bool,
}

impl Default for NativeRadialState {
    fn default() -> Self {
        Self {
            page: RadialPage::Root,
            hover: None,
            result_sent: false,
        }
    }
}

static NATIVE_STATE: OnceLock<Mutex<NativeRadialState>> = OnceLock::new();
static RADIAL_HWND: AtomicIsize = AtomicIsize::new(0);
static CLASS_READY: OnceLock<()> = OnceLock::new();

fn native_state() -> &'static Mutex<NativeRadialState> {
    NATIVE_STATE.get_or_init(|| Mutex::new(NativeRadialState::default()))
}

// =============================================================================
// Bevy-side lifecycle
// =============================================================================

#[derive(Resource, Default)]
struct ActionMenuState {
    open: bool,
    /// Last root-menu center in desktop physical pixels. Settings uses this to
    /// return to the same radial menu location instead of reopening under the
    /// Settings-window cursor.
    last_center: Option<IVec2>,
}

pub(crate) struct ActionMenuPlugin;

impl Plugin for ActionMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActionMenuState>()
            .add_message::<NeraActionMenuCommand>()
            .add_systems(
                Update,
                (
                    open_action_menu,
                    reopen_action_menu_from_settings,
                    poll_action_menu_result,
                )
                    .chain(),
            );
    }
}

fn post_action_menu_request(
    screen: IVec2,
    menu: &mut ActionMenuState,
    animation: &mut NeraAnimationState,
    tray: &TrayRuntime,
) {
    if menu.open {
        return;
    }

    let hwnd_raw = tray.hidden_hwnd_raw();
    if hwnd_raw == 0 {
        warn!("Nera radial menu requested before tray HWND was ready");
        return;
    }

    menu.open = true;
    menu.last_center = Some(screen);
    animation.set_praying_menu(true);

    let hwnd = HWND(hwnd_raw as *mut c_void);
    let posted = unsafe {
        windows::Win32::UI::WindowsAndMessaging::PostMessageW(
            Some(hwnd),
            WM_SHOW_NERA_ACTION_MENU,
            WPARAM(screen.x as usize),
            LPARAM(screen.y as isize),
        )
    };

    if posted.is_err() {
        menu.open = false;
        animation.set_praying_menu(false);
        error!("Failed to post Nera radial-menu request");
    }
}

fn open_action_menu(
    mut right_clicked: MessageReader<NeraRightClicked>,
    mut menu: ResMut<ActionMenuState>,
    mut animation: ResMut<NeraAnimationState>,
    tray: Option<Res<TrayRuntime>>,
) {
    let Some(tray) = tray else {
        return;
    };

    for event in right_clicked.read() {
        post_action_menu_request(
            event.pointer.screen,
            &mut menu,
            &mut animation,
            &tray,
        );
    }
}

fn reopen_action_menu_from_settings(
    mut requests: MessageReader<ReturnToActionMenu>,
    mut menu: ResMut<ActionMenuState>,
    mut animation: ResMut<NeraAnimationState>,
    tray: Option<Res<TrayRuntime>>,
) {
    if requests.read().next().is_none() {
        return;
    }

    let Some(tray) = tray else {
        return;
    };

    let Some(screen) = menu.last_center else {
        warn!("Settings requested Back, but no previous radial-menu center was recorded");
        return;
    };

    post_action_menu_request(screen, &mut menu, &mut animation, &tray);
    info!(
        "Nera Settings -> radial root reopened at ({}, {})",
        screen.x,
        screen.y,
    );
}

fn poll_action_menu_result(
    mut menu: ResMut<ActionMenuState>,
    mut animation: ResMut<NeraAnimationState>,
    tray: Option<Res<TrayRuntime>>,
    mut commands: MessageWriter<NeraActionMenuCommand>,
    mut open_settings: MessageWriter<OpenSettingsWindow>,
    mut open_alarm_settings: MessageWriter<OpenAlarmSettingsWindow>,
) {
    let Some(tray) = tray else {
        return;
    };

    for result in tray.drain_action_menu_results() {
        menu.open = false;
        animation.set_praying_menu(false);

        let Some(command) = result else {
            info!("Nera radial menu -> dismissed");
            continue;
        };

        match command {
            NeraActionMenuCommand::Animation(choice) => {
                info!("Nera radial menu -> Animation::{choice:?}");
            }
            NeraActionMenuCommand::Alarm(choice) => {
                info!(
                    "Nera radial menu -> Alarm::{choice:?} (timer logic in Phase 5.6)"
                );
            }
            NeraActionMenuCommand::AlarmSettings => {
                info!("Nera radial menu -> Alarm Settings");
                open_alarm_settings.write(OpenAlarmSettingsWindow);
            }
            NeraActionMenuCommand::Settings => {
                info!("Nera radial menu -> Settings");
                open_settings.write(OpenSettingsWindow);
            }
        }

        commands.write(command);
    }
}

// =============================================================================
// Geometry / hit testing
// =============================================================================

fn point_from_lparam(lparam: LPARAM) -> (i32, i32) {
    let raw = lparam.0 as u32;
    let x = (raw & 0xffff) as u16 as i16 as i32;
    let y = ((raw >> 16) & 0xffff) as u16 as i16 as i32;
    (x, y)
}

fn radius_from_center(x: i32, y: i32) -> f32 {
    let dx = x as f32 - WHEEL_CENTER;
    let dy = y as f32 - WHEEL_CENTER;
    (dx * dx + dy * dy).sqrt()
}

fn sector_at(page: RadialPage, x: i32, y: i32) -> Option<usize> {
    let radius = radius_from_center(x, y);

    if !(INNER_RADIUS..=OUTER_RADIUS).contains(&radius) {
        return None;
    }

    let items = items_for(page);
    let segment = TAU / items.len() as f32;

    let dx = x as f32 - WHEEL_CENTER;
    let dy = y as f32 - WHEEL_CENTER;

    // Screen Y grows down, so atan2 naturally advances clockwise here.
    // Rotate so index 0 is centered at 12 o'clock.
    let mut angle = dy.atan2(dx) + PI * 0.5;
    if angle < 0.0 {
        angle += TAU;
    }

    let index = ((angle + segment * 0.5) / segment).floor() as usize
        % items.len();

    Some(index)
}

fn item_position(index: usize, count: usize) -> (i32, i32) {
    let angle = -PI * 0.5 + TAU * index as f32 / count as f32;

    (
        (WHEEL_CENTER + angle.cos() * ITEM_RADIUS).round() as i32,
        (WHEEL_CENTER + angle.sin() * ITEM_RADIUS).round() as i32,
    )
}

// =============================================================================
// Painting
// =============================================================================

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

unsafe fn select(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    object: HGDIOBJ,
) -> HGDIOBJ {
    unsafe { SelectObject(hdc, object) }
}

unsafe fn paint_wheel(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };

    let key_brush = unsafe { CreateSolidBrush(rgb(KEY_RGB)) };
    let ring_brush = unsafe { CreateSolidBrush(rgb(RING_RGB)) };
    let inner_brush = unsafe { CreateSolidBrush(rgb(RING_INNER_RGB)) };
    let hover_brush = unsafe { CreateSolidBrush(rgb(HOVER_RGB)) };
    let accent_brush = unsafe { CreateSolidBrush(rgb(ACCENT_RGB)) };
    let line_pen = unsafe { CreatePen(PS_SOLID, 1, rgb(LINE_RGB)) };

    // Fill entire square with the chroma key.
    let full = RECT {
        left: 0,
        top: 0,
        right: WHEEL_SIZE,
        bottom: WHEEL_SIZE,
    };
    let _ = unsafe {
        windows::Win32::Graphics::Gdi::FillRect(
            hdc,
            &full,
            key_brush,
        )
    };

    let old_pen = unsafe {
        select(hdc, GetStockObject(NULL_PEN))
    };
    let old_brush = unsafe { select(hdc, HGDIOBJ(ring_brush.0)) };

    // Outer disk.
    let outer_margin = (WHEEL_CENTER - OUTER_RADIUS).round() as i32;
    let _ = unsafe {
        Ellipse(
            hdc,
            outer_margin,
            outer_margin,
            WHEEL_SIZE - outer_margin,
            WHEEL_SIZE - outer_margin,
        )
    };

    let state = native_state()
        .lock()
        .map(|state| *state)
        .unwrap_or_default();

    let items = items_for(state.page);

    // Hover bubble.
    if let Some(index) = state.hover {
        if items.get(index).copied().map(RadialItem::enabled).unwrap_or(false) {
            let (cx, cy) = item_position(index, items.len());
            let _ = unsafe { select(hdc, HGDIOBJ(hover_brush.0)) };
            let _ = unsafe {
                Ellipse(
                    hdc,
                    cx - HOVER_BUBBLE_RADIUS,
                    cy - HOVER_BUBBLE_RADIUS,
                    cx + HOVER_BUBBLE_RADIUS,
                    cy + HOVER_BUBBLE_RADIUS,
                )
            };

            // Tiny accent dot toward the outside edge.
            let outer_angle =
                -PI * 0.5 + TAU * index as f32 / items.len() as f32;
            let ax = (WHEEL_CENTER + outer_angle.cos() * 171.0).round() as i32;
            let ay = (WHEEL_CENTER + outer_angle.sin() * 171.0).round() as i32;

            let _ = unsafe { select(hdc, HGDIOBJ(accent_brush.0)) };
            let _ = unsafe { Ellipse(hdc, ax - 3, ay - 3, ax + 3, ay + 3) };
        }
    }

    // A subtly different inner band before punching the transparent hole.
    let _ = unsafe { select(hdc, HGDIOBJ(inner_brush.0)) };
    let band_radius = INNER_RADIUS + 13.0;
    let band_margin = (WHEEL_CENTER - band_radius).round() as i32;
    let _ = unsafe {
        Ellipse(
            hdc,
            band_margin,
            band_margin,
            WHEEL_SIZE - band_margin,
            WHEEL_SIZE - band_margin,
        )
    };

    // Segment separators.
    let _ = unsafe { select(hdc, HGDIOBJ(line_pen.0)) };

    for index in 0..items.len() {
        let segment = TAU / items.len() as f32;
        let angle = -PI * 0.5 - segment * 0.5 + index as f32 * segment;

        let inner_x =
            (WHEEL_CENTER + angle.cos() * (INNER_RADIUS + 14.0)).round() as i32;
        let inner_y =
            (WHEEL_CENTER + angle.sin() * (INNER_RADIUS + 14.0)).round() as i32;
        let outer_x =
            (WHEEL_CENTER + angle.cos() * (OUTER_RADIUS - 3.0)).round() as i32;
        let outer_y =
            (WHEEL_CENTER + angle.sin() * (OUTER_RADIUS - 3.0)).round() as i32;

        let _ = unsafe { MoveToEx(hdc, inner_x, inner_y, None) };
        let _ = unsafe { LineTo(hdc, outer_x, outer_y) };
    }

    // Transparent center: Nera remains visible through the wheel.
    let _ = unsafe { select(hdc, HGDIOBJ(key_brush.0)) };
    let inner_margin = (WHEEL_CENTER - INNER_RADIUS).round() as i32;
    let _ = unsafe {
        Ellipse(
            hdc,
            inner_margin,
            inner_margin,
            WHEEL_SIZE - inner_margin,
            WHEEL_SIZE - inner_margin,
        )
    };

    // Text / glyphs.
    let _ = unsafe { select(hdc, GetStockObject(DEFAULT_GUI_FONT)) };
    let _ = unsafe { SetBkMode(hdc, TRANSPARENT) };

    for (index, item) in items.iter().copied().enumerate() {
        let (cx, cy) = item_position(index, items.len());

        let selected = state.hover == Some(index) && item.enabled();
        let color = if !item.enabled() {
            rgb(MUTED_RGB)
        } else if selected {
            rgb(TEXT_RGB)
        } else {
            rgb(TEXT_RGB)
        };

        let _ = unsafe { SetTextColor(hdc, color) };

        let glyph = wide(item.glyph);
        let label = wide(item.label);

        // Default GUI font is intentionally used: no font asset needs to be
        // shipped and Windows can font-link many Unicode symbols.
        let glyph_x = cx - (item.glyph.chars().count() as i32 * 4);
        let _ = unsafe { TextOutW(hdc, glyph_x, cy - 15, &glyph) };

        let label_x = cx - (item.label.chars().count() as i32 * 3);
        let _ = unsafe { TextOutW(hdc, label_x, cy + 7, &label) };
    }

    // Restore old GDI objects before deleting our owned ones.
    let _ = unsafe { select(hdc, old_brush) };
    let _ = unsafe { select(hdc, old_pen) };

    let _ = unsafe { DeleteObject(HGDIOBJ(key_brush.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(ring_brush.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(inner_brush.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(hover_brush.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(accent_brush.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(line_pen.0)) };

    let _ = unsafe { EndPaint(hwnd, &ps) };
}

// =============================================================================
// Native interaction
// =============================================================================

fn set_page(hwnd: HWND, page: RadialPage) {
    if let Ok(mut state) = native_state().lock() {
        state.page = page;
        state.hover = None;
    }

    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
}

fn finish_menu(hwnd: HWND, result: Option<NeraActionMenuCommand>) {
    if let Ok(mut state) = native_state().lock() {
        if state.result_sent {
            return;
        }
        state.result_sent = true;
    }

    send_action_menu_result(result);
    let _ = unsafe { DestroyWindow(hwnd) };
}

fn activate_item(hwnd: HWND, x: i32, y: i32) {
    let (page, hover) = native_state()
        .lock()
        .map(|state| (state.page, state.hover))
        .unwrap_or((RadialPage::Root, None));

    let radius = radius_from_center(x, y);

    // Center behaves as Close on root and Back on sub-pages. This makes the
    // transparent hole useful without covering Nera with another button.
    if radius < INNER_RADIUS {
        navigate_back(hwnd, page);
        return;
    }

    if radius > OUTER_RADIUS {
        finish_menu(hwnd, None);
        return;
    }

    let Some(index) = hover.or_else(|| sector_at(page, x, y)) else {
        return;
    };

    let Some(item) = items_for(page).get(index).copied() else {
        return;
    };

    match item.action {
        RadialItemAction::Disabled => {}
        RadialItemAction::Open(next) => set_page(hwnd, next),
        RadialItemAction::Back => navigate_back(hwnd, page),
        RadialItemAction::Command(command) => {
            finish_menu(hwnd, Some(command));
        }
    }
}

// =============================================================================
// Radial HWND
// =============================================================================

unsafe extern "system" fn radial_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_ERASEBKGND => LRESULT(1),

        WM_PAINT => {
            unsafe { paint_wheel(hwnd) };
            LRESULT(0)
        }

        WM_MOUSEMOVE => {
            let (x, y) = point_from_lparam(lparam);

            let page = native_state()
                .lock()
                .map(|state| state.page)
                .unwrap_or(RadialPage::Root);

            let next_hover = sector_at(page, x, y);

            let changed = if let Ok(mut state) = native_state().lock() {
                if state.hover != next_hover {
                    state.hover = next_hover;
                    true
                } else {
                    false
                }
            } else {
                false
            };

            if changed {
                let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
            }

            LRESULT(0)
        }

        WM_LBUTTONUP => {
            let (x, y) = point_from_lparam(lparam);
            activate_item(hwnd, x, y);
            LRESULT(0)
        }

        WM_RBUTTONUP => {
            let page = native_state()
                .lock()
                .map(|state| state.page)
                .unwrap_or(RadialPage::Root);

            navigate_back(hwnd, page);

            LRESULT(0)
        }

        WM_KEYDOWN if wparam.0 as u32 == VK_ESCAPE_RAW => {
            let page = native_state()
                .lock()
                .map(|state| state.page)
                .unwrap_or(RadialPage::Root);

            navigate_back(hwnd, page);

            LRESULT(0)
        }

        WM_KILLFOCUS => {
            finish_menu(hwnd, None);
            LRESULT(0)
        }

        WM_DESTROY => {
            RADIAL_HWND.store(0, Ordering::Release);

            let should_send_cancel = if let Ok(mut state) = native_state().lock() {
                if state.result_sent {
                    false
                } else {
                    state.result_sent = true;
                    true
                }
            } else {
                false
            };

            if should_send_cancel {
                send_action_menu_result(None);
            }

            LRESULT(0)
        }

        _ => unsafe {
            DefWindowProcW(hwnd, message, wparam, lparam)
        },
    }
}

fn last_win32_error() -> Error {
    let error = unsafe { GetLastError() };
    Error::from_hresult(windows::core::HRESULT::from_win32(error.0))
}

fn ensure_radial_class() -> Result<HINSTANCE> {
    let module = unsafe { GetModuleHandleW(None)? };
    let instance = HINSTANCE(module.0);

    if CLASS_READY.get().is_some() {
        return Ok(instance);
    }

    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        hInstance: instance,
        lpszClassName: w!("NeraRadialMenuWindowClass"),
        lpfnWndProc: Some(radial_window_proc),
        ..Default::default()
    };

    let atom = unsafe { RegisterClassExW(&class) };
    if atom == 0 {
        return Err(last_win32_error());
    }

    let _ = CLASS_READY.set(());
    Ok(instance)
}

/// Called on the existing native tray thread.
///
/// Unlike TrackPopupMenu, this returns immediately. The tray thread's normal
/// GetMessage loop continues servicing the wheel HWND until it closes.
pub(crate) unsafe fn show_nera_action_menu(
    _owner: HWND,
    screen_x: i32,
    screen_y: i32,
) -> Result<()> {
    if RADIAL_HWND.load(Ordering::Acquire) != 0 {
        return Ok(());
    }

    let instance = ensure_radial_class()?;

    if let Ok(mut state) = native_state().lock() {
        *state = NativeRadialState::default();
    }

    let left = screen_x - WHEEL_SIZE / 2;
    let top = screen_y - WHEEL_SIZE / 2;

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED,
            w!("NeraRadialMenuWindowClass"),
            w!("Nera Actions"),
            WS_POPUP,
            left,
            top,
            WHEEL_SIZE,
            WHEEL_SIZE,
            None,
            None,
            Some(instance),
            None,
        )?
    };

    RADIAL_HWND.store(hwnd.0 as isize, Ordering::Release);

    unsafe {
        SetLayeredWindowAttributes(
            hwnd,
            rgb(KEY_RGB),
            255,
            LWA_COLORKEY,
        )?;
    }

    let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };
    let _ = unsafe { SetForegroundWindow(hwnd) };
    let _ = unsafe { InvalidateRect(Some(hwnd), None, true) };

    info!(
        "Nera radial action wheel opened center=({}, {}) hwnd=0x{:X}",
        screen_x,
        screen_y,
        hwnd.0 as usize,
    );

    Ok(())
}

// =============================================================================
// Tests — geometry + typed command contracts
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_has_six_wheel_segments() {
        assert_eq!(items_for(RadialPage::Root).len(), 6);
    }

    #[test]
    fn top_sector_is_first_item() {
        let x = WHEEL_CENTER.round() as i32;
        let y = (WHEEL_CENTER - ITEM_RADIUS).round() as i32;

        assert_eq!(
            sector_at(RadialPage::Root, x, y),
            Some(0)
        );
    }

    #[test]
    fn center_is_not_a_sector() {
        assert_eq!(
            sector_at(
                RadialPage::Root,
                WHEEL_CENTER as i32,
                WHEEL_CENTER as i32,
            ),
            None
        );
    }

    #[test]
    fn animation_and_alarm_keep_typed_commands() {
        assert!(matches!(
            ANIMATION_ITEMS[0].action,
            RadialItemAction::Command(
                NeraActionMenuCommand::Animation(AnimationMenuChoice::Wave)
            )
        ));

        assert!(matches!(
            ALARM_ITEMS[1].action,
            RadialItemAction::Command(
                NeraActionMenuCommand::Alarm(AlarmMenuChoice::TenMinutes)
            )
        ));
    }

    /// Wheel geometry is derived from `items.len()`, so pages may legitimately
    /// differ in segment count. The invariant that matters is navigation: every
    /// page that has a parent must offer a way back, and Root — which has no
    /// parent — must not.
    #[test]
    fn every_sub_page_ends_with_a_back_item() {
        for page in [
            RadialPage::Root,
            RadialPage::Animation,
            RadialPage::Alarm,
            RadialPage::AlarmMore,
        ] {
            let items = items_for(page);
            assert!(!items.is_empty(), "{page:?}");

            let last_is_back = matches!(
                items[items.len() - 1].action,
                RadialItemAction::Back
            );

            assert_eq!(
                last_is_back,
                parent_page(page).is_some(),
                "{page:?}",
            );
        }
    }

    /// `AlarmMore` is nested one level below `Alarm`, so going up must not dump
    /// the user straight back to Root.
    #[test]
    fn page_hierarchy_nests_alarm_more_under_alarm() {
        assert_eq!(parent_page(RadialPage::Root), None);
        assert_eq!(
            parent_page(RadialPage::Animation),
            Some(RadialPage::Root)
        );
        assert_eq!(
            parent_page(RadialPage::Alarm),
            Some(RadialPage::Root)
        );
        assert_eq!(
            parent_page(RadialPage::AlarmMore),
            Some(RadialPage::Alarm)
        );
    }

    /// Every alarm slot must perform a real action. `Custom` now opens the
    /// Settings alarm editor instead of looking enabled and only logging a warn.
    #[test]
    fn custom_alarm_opens_the_dedicated_alarm_editor() {
        assert!(matches!(
            ALARM_MORE_ITEMS[4].action,
            RadialItemAction::Command(NeraActionMenuCommand::AlarmSettings)
        ));
        assert_eq!(ALARM_MORE_ITEMS[4].label, "Custom");
    }

    #[test]
    fn alarm_pages_have_no_dead_end_slots() {
        for page in [RadialPage::Alarm, RadialPage::AlarmMore] {
            for item in items_for(page) {
                assert!(item.enabled(), "{page:?} -> {}", item.label);
            }
        }
    }
}
