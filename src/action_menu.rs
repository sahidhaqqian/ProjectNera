//! Project Nera — Phase 5.5 radial action wheel.
//!
//! The old TrackPopupMenu prototype proved the command/lifecycle plumbing.
//! This module replaces only the presentation layer with a Mate-Engine-style
//! radial wheel while keeping:
//!
//! - Bevy main thread unblocked.
//! - Existing tray/native UI thread.
//! - Typed `NeraActionMenuCommand` messages.
//! - Praying while the wheel is open and Nera is standing.
//! - Animation / Alarm command contracts.
//!
//! The wheel is a real transparent Win32 popup. Its center is color-key
//! transparent so Nera remains visible inside the ring.

use std::{
    f32::consts::{PI, TAU},
    ffi::c_void,
    mem::size_of,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicIsize, Ordering},
    },
    time::{Duration, Instant},
};

use bevy::prelude::*;

use windows::{
    Win32::{
        Foundation::{
            COLORREF, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
        },
        Graphics::Gdi::{
            BeginPaint, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen,
            CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_GUI_FONT, DeleteObject, Ellipse, EndPaint,
            GetStockObject, GetTextExtentPoint32W, HDC, HGDIOBJ, HOLLOW_BRUSH, InvalidateRect,
            LineTo, MoveToEx, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, Polygon,
            Polyline, RoundRect, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_TOPMOST, KillTimer, LWA_COLORKEY,
            RegisterClassExW, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow,
            SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, WM_DESTROY,
            WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
            WM_RBUTTONUP, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
            WS_POPUP,
        },
    },
    core::{Error, Result, w},
};

use crate::{
    alarm_settings::OpenAlarmSettingsWindow,
    animation::NeraAnimationState,
    avatar::body_mode::{AvatarBodyMode, ToggleBodyMode},
    environment::{DesktopEnvironment, PlacementState},
    interaction::NeraRightClicked,
    settings::{OpenSettingsWindow, ReturnToActionMenu},
    tray::{TrayRuntime, send_action_menu_result},
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
    Treat,
    Cloud(crate::cloud::CloudCommand),
    Animation(AnimationMenuChoice),
    AnimationManager,
    Chibi,
    Screensaver,
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
const SELECTION_TIMER: usize = 1;
const SELECTION_FEEDBACK_MS: u32 = 150;

/// Rare chroma key. Every pixel painted with this exact RGB becomes invisible.
const KEY_RGB: (u8, u8, u8) = (1, 2, 3);

const RING_RGB: (u8, u8, u8) = (16, 20, 34);
const RING_INNER_RGB: (u8, u8, u8) = (23, 28, 47);
const HOVER_RGB: (u8, u8, u8) = (40, 37, 66);
const SELECTED_RGB: (u8, u8, u8) = (106, 79, 151);
const LINE_RGB: (u8, u8, u8) = (48, 55, 80);
const ICON_RGB: (u8, u8, u8) = (205, 190, 248);
const TEXT_RGB: (u8, u8, u8) = (245, 244, 252);
const MUTED_RGB: (u8, u8, u8) = (108, 116, 145);

fn rgb(value: (u8, u8, u8)) -> COLORREF {
    COLORREF(value.0 as u32 | ((value.1 as u32) << 8) | ((value.2 as u32) << 16))
}

// =============================================================================
// Menu model
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RadialPage {
    Root,
    Cloud,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RadialItemAction {
    Disabled,
    Open(RadialPage),
    Back,
    Command(NeraActionMenuCommand),
}

#[derive(Clone, Copy, Debug)]
enum RadialIcon {
    Treat,
    Cloud,
    Settings,
    Play,
    Alarm,
    Games,
    Sparkle,
    Monitor,
    Moon,
    Ride,
    Close,
    Back,
}

#[derive(Clone, Copy, Debug)]
struct RadialItem {
    icon: RadialIcon,
    label: &'static str,
    action: RadialItemAction,
}

impl RadialItem {
    fn enabled(self) -> bool {
        !matches!(self.action, RadialItemAction::Disabled)
    }
}

const ROOT_ITEMS: [RadialItem; 7] = [
    RadialItem {
        icon: RadialIcon::Cloud,
        label: "Cloud",
        action: RadialItemAction::Open(RadialPage::Cloud),
    },
    RadialItem {
        icon: RadialIcon::Settings,
        label: "Settings",
        action: RadialItemAction::Command(NeraActionMenuCommand::Settings),
    },
    RadialItem {
        icon: RadialIcon::Play,
        label: "Animation",
        action: RadialItemAction::Command(NeraActionMenuCommand::AnimationManager),
    },
    RadialItem {
        icon: RadialIcon::Alarm,
        label: "Alarm",
        action: RadialItemAction::Command(NeraActionMenuCommand::AlarmSettings),
    },
    RadialItem {
        icon: RadialIcon::Games,
        label: "Games",
        action: RadialItemAction::Disabled,
    },
    RadialItem {
        icon: RadialIcon::Sparkle,
        label: "Chibi",
        action: RadialItemAction::Command(NeraActionMenuCommand::Chibi),
    },
    RadialItem {
        icon: RadialIcon::Monitor,
        label: "Screensaver",
        action: RadialItemAction::Command(NeraActionMenuCommand::Screensaver),
    },
];

const CLOUD_ITEMS: [RadialItem; 5] = [
    RadialItem {
        icon: RadialIcon::Moon,
        label: "Rest",
        action: RadialItemAction::Command(NeraActionMenuCommand::Cloud(
            crate::cloud::CloudCommand::Summon(crate::cloud::CloudMode::Rest),
        )),
    },
    RadialItem {
        icon: RadialIcon::Ride,
        label: "Ride",
        action: RadialItemAction::Command(NeraActionMenuCommand::Cloud(
            crate::cloud::CloudCommand::Summon(crate::cloud::CloudMode::Ride),
        )),
    },
    RadialItem {
        icon: RadialIcon::Close,
        label: "Dismiss",
        action: RadialItemAction::Command(NeraActionMenuCommand::Cloud(
            crate::cloud::CloudCommand::Dismiss,
        )),
    },
    RadialItem {
        icon: RadialIcon::Treat,
        label: "Treat",
        action: RadialItemAction::Command(NeraActionMenuCommand::Treat),
    },
    RadialItem {
        icon: RadialIcon::Back,
        label: "Back",
        action: RadialItemAction::Back,
    },
];

fn items_for(page: RadialPage) -> &'static [RadialItem] {
    match page {
        RadialPage::Root => &ROOT_ITEMS,
        RadialPage::Cloud => &CLOUD_ITEMS,
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
        RadialPage::Cloud => Some(RadialPage::Root),
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
    /// Presentation feedback only; the typed command still belongs to Bevy.
    pending: Option<PendingSelection>,
    result_sent: bool,
}

#[derive(Clone, Copy, Debug)]
struct PendingSelection {
    index: usize,
    ready_at: Instant,
}

impl NativeRadialState {
    fn begin_selection(&mut self, index: usize, now: Instant) -> bool {
        if self.result_sent
            || self.pending.is_some()
            || !items_for(self.page)
                .get(index)
                .is_some_and(|item| item.enabled())
        {
            return false;
        }
        self.hover = Some(index);
        self.pending = Some(PendingSelection {
            index,
            ready_at: now + Duration::from_millis(SELECTION_FEEDBACK_MS.into()),
        });
        true
    }

    fn take_ready_selection(&mut self, now: Instant) -> Option<RadialItemAction> {
        let pending = self.pending?;
        if self.result_sent || now < pending.ready_at {
            return None;
        }
        self.pending = None;
        items_for(self.page)
            .get(pending.index)
            .map(|item| item.action)
    }
}

impl Default for NativeRadialState {
    fn default() -> Self {
        Self {
            page: RadialPage::Root,
            hover: None,
            pending: None,
            result_sent: false,
        }
    }
}

static NATIVE_STATE: OnceLock<Mutex<NativeRadialState>> = OnceLock::new();
static RADIAL_HWND: AtomicIsize = AtomicIsize::new(0);
static CLASS_READY: OnceLock<()> = OnceLock::new();
/// Native painting caches only presentation text. `AvatarBodyMode` owns the
/// selection; the menu never changes or resolves the body mode itself.
static CHIBI_LABEL: OnceLock<Mutex<&'static str>> = OnceLock::new();

fn chibi_label(mode: AvatarBodyMode) -> &'static str {
    match mode {
        AvatarBodyMode::Normal => "Chibi: Off",
        AvatarBodyMode::Chibi => "Chibi: On",
    }
}

fn native_chibi_label() -> &'static Mutex<&'static str> {
    CHIBI_LABEL.get_or_init(|| Mutex::new(chibi_label(AvatarBodyMode::Normal)))
}

fn sync_chibi_menu_label(mode: Res<AvatarBodyMode>) {
    let next = chibi_label(*mode);
    let changed = native_chibi_label().lock().is_ok_and(|mut label| {
        if *label == next {
            return false;
        }
        *label = next;
        true
    });
    let raw = RADIAL_HWND.load(Ordering::Acquire);
    if changed && raw != 0 {
        let _ = unsafe { InvalidateRect(Some(HWND(raw as *mut c_void)), None, false) };
    }
}

fn native_state() -> &'static Mutex<NativeRadialState> {
    NATIVE_STATE.get_or_init(|| Mutex::new(NativeRadialState::default()))
}

// =============================================================================
// Bevy-side lifecycle
// =============================================================================

#[derive(Resource, Default)]
pub(crate) struct ActionMenuState {
    open: bool,
    /// Last root-menu center in desktop physical pixels. Settings uses this to
    /// return to the same radial menu location instead of reopening under the
    /// Settings-window cursor.
    last_center: Option<IVec2>,
}

impl ActionMenuState {
    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    fn open_at(
        &mut self,
        screen: IVec2,
        placement: PlacementState,
        animation: &mut NeraAnimationState,
    ) {
        self.open = true;
        self.last_center = Some(screen);
        // Attachment may already be pending before animation's support sync.
        // The wheel still opens; only its standing-only pose is gated.
        animation.set_praying_menu(placement == PlacementState::Free);
    }
}

pub(crate) struct ActionMenuPlugin;

impl Plugin for ActionMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActionMenuState>()
            .add_message::<NeraActionMenuCommand>()
            .add_systems(
                Update,
                (
                    sync_chibi_menu_label,
                    open_action_menu,
                    reopen_action_menu_from_settings,
                    poll_action_menu_result,
                    keep_action_menu_front,
                )
                    .chain(),
            );
    }
}

fn post_action_menu_request(
    screen: IVec2,
    menu: &mut ActionMenuState,
    animation: &mut NeraAnimationState,
    placement: PlacementState,
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

    menu.open_at(screen, placement, animation);

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
    environment: Res<DesktopEnvironment>,
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
            environment.placement,
            &tray,
        );
    }
}

fn reopen_action_menu_from_settings(
    mut requests: MessageReader<ReturnToActionMenu>,
    mut menu: ResMut<ActionMenuState>,
    mut animation: ResMut<NeraAnimationState>,
    environment: Res<DesktopEnvironment>,
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

    post_action_menu_request(
        screen,
        &mut menu,
        &mut animation,
        environment.placement,
        &tray,
    );
    info!(
        "Nera Settings -> radial root reopened at ({}, {})",
        screen.x, screen.y,
    );
}

fn keep_action_menu_front(menu: Res<ActionMenuState>) {
    if !menu.open {
        return;
    }

    let raw = RADIAL_HWND.load(Ordering::Acquire);
    if raw == 0 {
        return;
    }

    let hwnd = HWND(raw as *mut c_void);

    // Environment/taskbar maintenance can legitimately reassert Nera's own
    // TOPMOST HWND. Run this in Update after those PreUpdate systems so the
    // radial wheel remains the front-most topmost window for the whole time it
    // is open, including Settings -> Back.
    let _ = unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
}

pub(crate) fn poll_action_menu_result(
    mut menu: ResMut<ActionMenuState>,
    mut animation: ResMut<NeraAnimationState>,
    tray: Option<Res<TrayRuntime>>,
    mut commands: MessageWriter<NeraActionMenuCommand>,
    mut toggle_body_mode: MessageWriter<ToggleBodyMode>,
    mut open_settings: MessageWriter<OpenSettingsWindow>,
    mut open_alarm_settings: MessageWriter<OpenAlarmSettingsWindow>,
    mut open_animation_manager: MessageWriter<crate::animation_manager::OpenAnimationManager>,
    mut presentation: MessageWriter<crate::presentation::PresentationCommand>,
    mut treat: MessageWriter<crate::treat::RequestTreat>,
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
            NeraActionMenuCommand::Treat => {
                treat.write(crate::treat::RequestTreat);
            }
            NeraActionMenuCommand::Cloud(choice) => {
                info!("Nera radial menu -> Cloud::{choice:?}");
            }
            NeraActionMenuCommand::Chibi => {
                info!("Nera radial menu -> Chibi toggle");
                toggle_body_mode.write(ToggleBodyMode);
            }
            NeraActionMenuCommand::Screensaver => {
                presentation.write(crate::presentation::PresentationCommand::EnterAttention(
                    crate::presentation::AttentionKind::Screensaver,
                ));
            }
            NeraActionMenuCommand::AnimationManager => {
                info!("Nera radial menu -> Animation Manager");
                open_animation_manager.write(crate::animation_manager::OpenAnimationManager);
            }
            NeraActionMenuCommand::Animation(choice) => {
                info!("Nera radial menu -> Animation::{choice:?}");
            }
            NeraActionMenuCommand::Alarm(choice) => {
                info!("Nera radial menu -> Alarm::{choice:?} (timer logic in Phase 5.6)");
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

    let index = ((angle + segment * 0.5) / segment).floor() as usize % items.len();

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

unsafe fn select(hdc: HDC, object: HGDIOBJ) -> HGDIOBJ {
    unsafe { SelectObject(hdc, object) }
}

/// Small native vector icons avoid tiny/missing Unicode font glyphs.
unsafe fn paint_icon(hdc: HDC, icon: RadialIcon, cx: i32, cy: i32, color: COLORREF) {
    let pen = unsafe { CreatePen(PS_SOLID, 2, color) };
    let fill = unsafe { CreateSolidBrush(color) };
    let old_pen = unsafe { select(hdc, HGDIOBJ(pen.0)) };
    let old_brush = unsafe { select(hdc, GetStockObject(HOLLOW_BRUSH)) };
    let points = |coords: &[(i32, i32)]| -> Vec<POINT> {
        coords
            .iter()
            .map(|&(x, y)| POINT {
                x: cx + x,
                y: cy + y,
            })
            .collect()
    };
    let line = |coords: &[(i32, i32)]| {
        let _ = unsafe { Polyline(hdc, &points(coords)) };
    };
    let polygon = |coords: &[(i32, i32)]| {
        let _ = unsafe { Polygon(hdc, &points(coords)) };
    };
    let circle = |x: i32, y: i32, radius: i32| {
        let _ = unsafe {
            Ellipse(
                hdc,
                cx + x - radius,
                cy + y - radius,
                cx + x + radius,
                cy + y + radius,
            )
        };
    };

    match icon {
        RadialIcon::Treat => {
            polygon(&[(-11, 4), (-8, -5), (-3, -10), (5, -10), (11, -4), (11, 5)]);
            line(&[(-11, 8), (11, 8)]);
            line(&[(-4, -4), (-3, -3)]);
            line(&[(5, -2), (6, -1)]);
        }
        RadialIcon::Cloud | RadialIcon::Ride => {
            line(&[
                (-12, 5),
                (-12, 1),
                (-10, -2),
                (-6, -3),
                (-5, -7),
                (-2, -10),
                (3, -10),
                (7, -7),
                (8, -3),
                (10, -3),
                (13, 0),
                (13, 5),
                (10, 8),
                (-9, 8),
                (-12, 5),
            ]);
            if matches!(icon, RadialIcon::Ride) {
                line(&[(-5, 2), (6, 2), (3, -1)]);
                line(&[(6, 2), (3, 5)]);
            }
        }
        RadialIcon::Settings => {
            let gear: Vec<_> = (0..32)
                .map(|index| {
                    let angle = TAU * index as f32 / 32.0;
                    let radius = if index % 4 < 2 { 13.0 } else { 10.0 };
                    POINT {
                        x: cx + (angle.cos() * radius).round() as i32,
                        y: cy + (angle.sin() * radius).round() as i32,
                    }
                })
                .collect();
            let _ = unsafe { Polygon(hdc, &gear) };
            circle(0, 0, 5);
        }
        RadialIcon::Play => {
            polygon(&[(-8, -12), (12, 0), (-8, 12)]);
        }
        RadialIcon::Alarm => {
            circle(0, 1, 10);
            line(&[(0, -5), (0, 1), (5, 4)]);
            line(&[(-12, -7), (-7, -12)]);
            line(&[(7, -12), (12, -7)]);
            line(&[(-7, 9), (-10, 13)]);
            line(&[(7, 9), (10, 13)]);
        }
        RadialIcon::Games => {
            let _ = unsafe { RoundRect(hdc, cx - 13, cy - 7, cx + 14, cy + 10, 8, 8) };
            line(&[(-10, 1), (-2, 1)]);
            line(&[(-6, -3), (-6, 5)]);
            circle(5, -1, 2);
            circle(9, 4, 2);
        }
        RadialIcon::Sparkle => {
            let _ = unsafe { select(hdc, HGDIOBJ(fill.0)) };
            polygon(&[
                (0, -14),
                (4, -4),
                (14, 0),
                (4, 4),
                (0, 14),
                (-4, 4),
                (-14, 0),
                (-4, -4),
            ]);
        }
        RadialIcon::Monitor => {
            let _ = unsafe { RoundRect(hdc, cx - 13, cy - 10, cx + 14, cy + 8, 4, 4) };
            line(&[(0, 8), (0, 13)]);
            line(&[(-7, 13), (7, 13)]);
        }
        RadialIcon::Moon => {
            polygon(&[
                (-2, -12),
                (-8, -9),
                (-12, -4),
                (-12, 3),
                (-8, 9),
                (-2, 12),
                (5, 11),
                (10, 7),
                (12, 2),
                (6, 4),
                (1, 2),
                (-2, -3),
                (-3, -8),
            ]);
        }
        RadialIcon::Close => {
            line(&[(-9, -9), (9, 9)]);
            line(&[(9, -9), (-9, 9)]);
        }
        RadialIcon::Back => {
            line(&[(0, -10), (-10, 0), (0, 10)]);
            line(&[(-10, 0), (12, 0)]);
        }
    }
    let _ = unsafe { select(hdc, old_brush) };
    let _ = unsafe { select(hdc, old_pen) };
    let _ = unsafe { DeleteObject(HGDIOBJ(fill.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(pen.0)) };
}

fn sector_polygon(index: usize, count: usize) -> Vec<POINT> {
    let segment = TAU / count as f32;
    let start = -PI * 0.5 + (index as f32 - 0.5) * segment;
    let mut points = vec![POINT {
        x: WHEEL_CENTER as i32,
        y: WHEEL_CENTER as i32,
    }];
    points.extend((0..=32).map(|step| {
        let angle = start + segment * step as f32 / 32.0;
        POINT {
            x: (WHEEL_CENTER + angle.cos() * OUTER_RADIUS).round() as i32,
            y: (WHEEL_CENTER + angle.sin() * OUTER_RADIUS).round() as i32,
        }
    }));
    points
}

unsafe fn paint_wheel(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };
    let state = native_state()
        .lock()
        .map(|state| *state)
        .unwrap_or_default();
    unsafe { render_wheel(hdc, &state) };
    let _ = unsafe { EndPaint(hwnd, &ps) };
}

unsafe fn render_wheel(hdc: HDC, state: &NativeRadialState) {
    let key_brush = unsafe { CreateSolidBrush(rgb(KEY_RGB)) };
    let ring_brush = unsafe { CreateSolidBrush(rgb(RING_RGB)) };
    let inner_brush = unsafe { CreateSolidBrush(rgb(RING_INNER_RGB)) };
    let hover_brush = unsafe { CreateSolidBrush(rgb(HOVER_RGB)) };
    let selected_brush = unsafe { CreateSolidBrush(rgb(SELECTED_RGB)) };
    let line_pen = unsafe { CreatePen(PS_SOLID, 1, rgb(LINE_RGB)) };
    let label_font = unsafe {
        CreateFontW(
            -14,
            0,
            0,
            0,
            500,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        )
    };

    // Fill entire square with the chroma key.
    let full = RECT {
        left: 0,
        top: 0,
        right: WHEEL_SIZE,
        bottom: WHEEL_SIZE,
    };
    let _ = unsafe { windows::Win32::Graphics::Gdi::FillRect(hdc, &full, key_brush) };

    let old_pen = unsafe { select(hdc, GetStockObject(NULL_PEN)) };
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

    let items = items_for(state.page);

    // Fill the whole selected segment before punching out the center.
    if let Some(index) = state.pending.map(|pending| pending.index).or(state.hover) {
        if items
            .get(index)
            .copied()
            .map(RadialItem::enabled)
            .unwrap_or(false)
        {
            let brush = if state.pending.is_some() {
                selected_brush
            } else {
                hover_brush
            };
            let _ = unsafe { select(hdc, HGDIOBJ(brush.0)) };
            let _ = unsafe { Polygon(hdc, &sector_polygon(index, items.len())) };
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

        let inner_x = (WHEEL_CENTER + angle.cos() * (INNER_RADIUS + 14.0)).round() as i32;
        let inner_y = (WHEEL_CENTER + angle.sin() * (INNER_RADIUS + 14.0)).round() as i32;
        let outer_x = (WHEEL_CENTER + angle.cos() * (OUTER_RADIUS - 3.0)).round() as i32;
        let outer_y = (WHEEL_CENTER + angle.sin() * (OUTER_RADIUS - 3.0)).round() as i32;

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

    // Separate 28px vector icons from measured, centered labels.
    let font = if label_font.0.is_null() {
        unsafe { GetStockObject(DEFAULT_GUI_FONT) }
    } else {
        HGDIOBJ(label_font.0)
    };
    let old_font = unsafe { select(hdc, font) };
    let _ = unsafe { SetBkMode(hdc, TRANSPARENT) };

    for (index, item) in items.iter().copied().enumerate() {
        let (cx, cy) = item_position(index, items.len());

        let pressed = state.pending.is_some_and(|pending| pending.index == index);
        let icon_color = if !item.enabled() {
            MUTED_RGB
        } else if pressed {
            TEXT_RGB
        } else {
            ICON_RGB
        };
        unsafe { paint_icon(hdc, item.icon, cx, cy - 8, rgb(icon_color)) };
        let _ =
            unsafe { SetTextColor(hdc, rgb(if item.enabled() { TEXT_RGB } else { MUTED_RGB })) };
        let label_text = if matches!(
            item.action,
            RadialItemAction::Command(NeraActionMenuCommand::Chibi)
        ) {
            native_chibi_label()
                .lock()
                .map(|label| *label)
                .unwrap_or(item.label)
        } else {
            item.label
        };
        let label = wide(label_text);

        let mut extent = SIZE::default();
        let _ = unsafe { GetTextExtentPoint32W(hdc, &label, &mut extent) };
        let _ = unsafe { TextOutW(hdc, cx - extent.cx / 2, cy + 13, &label) };
    }

    // Restore old GDI objects before deleting our owned ones.
    let _ = unsafe { select(hdc, old_brush) };
    let _ = unsafe { select(hdc, old_pen) };
    let _ = unsafe { select(hdc, old_font) };

    let _ = unsafe { DeleteObject(HGDIOBJ(key_brush.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(ring_brush.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(inner_brush.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(hover_brush.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(selected_brush.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(line_pen.0)) };
    let _ = unsafe { DeleteObject(HGDIOBJ(label_font.0)) };
}

// =============================================================================
// Native interaction
// =============================================================================

fn set_page(hwnd: HWND, page: RadialPage) {
    let _ = unsafe { KillTimer(Some(hwnd), SELECTION_TIMER) };
    if let Ok(mut state) = native_state().lock() {
        state.page = page;
        state.hover = None;
        state.pending = None;
    }

    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
}

fn finish_menu(hwnd: HWND, result: Option<NeraActionMenuCommand>) {
    let _ = unsafe { KillTimer(Some(hwnd), SELECTION_TIMER) };
    if let Ok(mut state) = native_state().lock() {
        if state.result_sent {
            return;
        }
        state.result_sent = true;
        state.pending = None;
    }

    send_action_menu_result(result);
    let _ = unsafe { DestroyWindow(hwnd) };
}

fn activate_item(hwnd: HWND, x: i32, y: i32) {
    let state = native_state()
        .lock()
        .map(|state| *state)
        .unwrap_or_default();
    if state.pending.is_some() || state.result_sent {
        return;
    }
    let page = state.page;

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

    // The click coordinates are authoritative, even if hover has gone stale.
    let Some(index) = sector_at(page, x, y) else {
        return;
    };

    let accepted = native_state()
        .lock()
        .is_ok_and(|mut state| state.begin_selection(index, Instant::now()));
    if !accepted {
        return;
    }
    let _ = unsafe { InvalidateRect(Some(hwnd), None, false) };
    // A native timer keeps both the tray and Bevy responsive during feedback.
    if unsafe { SetTimer(Some(hwnd), SELECTION_TIMER, SELECTION_FEEDBACK_MS, None) } == 0 {
        warn!("Radial selection timer unavailable; dispatching selected action immediately");
        complete_selection(
            hwnd,
            Instant::now() + Duration::from_millis(SELECTION_FEEDBACK_MS.into()),
        );
    }
}

fn complete_selection(hwnd: HWND, now: Instant) {
    let ready = native_state().lock().ok().and_then(|mut state| {
        let page = state.page;
        state.take_ready_selection(now).map(|action| (page, action))
    });
    let Some((page, action)) = ready else {
        return;
    };
    let _ = unsafe { KillTimer(Some(hwnd), SELECTION_TIMER) };
    match action {
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
                if state.pending.is_none() && state.hover != next_hover {
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

        WM_TIMER if wparam.0 == SELECTION_TIMER => {
            complete_selection(hwnd, Instant::now());
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
            let _ = unsafe { KillTimer(Some(hwnd), SELECTION_TIMER) };
            RADIAL_HWND.store(0, Ordering::Release);

            let should_send_cancel = if let Ok(mut state) = native_state().lock() {
                state.pending = None;
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

        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
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
        SetLayeredWindowAttributes(hwnd, rgb(KEY_RGB), 255, LWA_COLORKEY)?;
    }

    let _ = unsafe { ShowWindow(hwnd, SW_SHOW) };

    // Nera itself is normally AlwaysOnTop. WS_EX_TOPMOST alone does not
    // guarantee a newly re-opened wheel becomes the front-most member of the
    // topmost band after returning from Settings, so explicitly promote it.
    unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )?;
    }

    let _ = unsafe { SetForegroundWindow(hwnd) };
    let _ = unsafe { InvalidateRect(Some(hwnd), None, true) };

    info!(
        "Nera radial action wheel opened center=({}, {}) hwnd=0x{:X}",
        screen_x, screen_y, hwnd.0 as usize,
    );

    Ok(())
}

// =============================================================================
// Tests — geometry + typed command contracts
// =============================================================================

#[cfg(test)]
mod native_validation;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_waits_for_feedback_and_dispatches_once_despite_repeated_clicks() {
        let now = Instant::now();
        let delay = Duration::from_millis(SELECTION_FEEDBACK_MS.into());
        let mut state = NativeRadialState::default();
        assert!(state.begin_selection(5, now));
        assert!(!state.begin_selection(1, now + delay / 2));
        assert_eq!(state.hover, Some(5));
        assert_eq!(state.take_ready_selection(now), None);
        assert_eq!(
            state.take_ready_selection(now + delay - Duration::from_millis(1)),
            None
        );
        assert_eq!(
            state.take_ready_selection(now + delay),
            Some(RadialItemAction::Command(NeraActionMenuCommand::Chibi))
        );
        assert_eq!(state.take_ready_selection(now + delay * 2), None);
    }

    #[test]
    fn disabled_invalid_or_closed_menu_cannot_schedule_an_action() {
        let now = Instant::now();
        let mut state = NativeRadialState::default();
        assert!(!state.begin_selection(4, now)); // Games remains disabled.
        assert!(!state.begin_selection(ROOT_ITEMS.len(), now));
        assert!(state.pending.is_none());
        state.result_sent = true;
        assert!(!state.begin_selection(1, now));
    }

    #[test]
    fn submenu_and_back_receive_the_same_selection_feedback() {
        let now = Instant::now();
        let ready = now + Duration::from_millis(SELECTION_FEEDBACK_MS.into());
        let mut state = NativeRadialState::default();
        assert!(state.begin_selection(0, now));
        assert_eq!(state.take_ready_selection(now), None);
        assert_eq!(
            state.take_ready_selection(ready),
            Some(RadialItemAction::Open(RadialPage::Cloud))
        );
        state.page = RadialPage::Cloud;
        assert!(state.begin_selection(3, now));
        assert_eq!(state.take_ready_selection(now), None);
        assert_eq!(
            state.take_ready_selection(ready),
            Some(RadialItemAction::Command(NeraActionMenuCommand::Treat))
        );
        assert!(state.begin_selection(4, now));
        assert_eq!(
            state.take_ready_selection(ready),
            Some(RadialItemAction::Back)
        );
    }

    #[test]
    fn wheel_opens_without_praying_before_pending_or_attached_seat_sync() {
        use crate::{
            animation::{BasePose, VisualState},
            environment::{TaskbarAttachment, TaskbarEdge, WindowAttachment},
            platform::{NativeMonitorHandle, NativeWindowHandle},
        };

        let target = NativeWindowHandle::from_raw(1);
        let monitor = NativeMonitorHandle::from_raw(1);
        for placement in [
            PlacementState::PendingWindow {
                target,
                settle_frames: 1,
            },
            PlacementState::PendingTaskbar {
                monitor,
                edge: TaskbarEdge::Bottom,
                settle_frames: 1,
            },
            PlacementState::Window(WindowAttachment {
                target,
                support_offset_x: 100,
            }),
            PlacementState::Taskbar(TaskbarAttachment {
                monitor,
                edge: TaskbarEdge::Bottom,
                along_offset: 100,
            }),
        ] {
            let mut menu = ActionMenuState::default();
            let mut animation = NeraAnimationState::default();
            let screen = IVec2::new(200, 300);
            // Animation still says Idle when a physical attachment arrives
            // earlier in the same frame. The menu must use placement too.
            assert_eq!(animation.base_pose(), BasePose::Idle);
            menu.open_at(screen, placement, &mut animation);
            assert!(menu.open);
            assert_eq!(menu.last_center, Some(screen));
            assert_eq!(animation.visual_state(), VisualState::Idle);
        }

        let mut menu = ActionMenuState::default();
        let mut animation = NeraAnimationState::default();
        menu.open_at(IVec2::ZERO, PlacementState::Free, &mut animation);
        assert!(menu.open);
        assert_eq!(animation.visual_state(), VisualState::Praying);
    }

    #[test]
    fn root_has_seven_wheel_segments() {
        assert_eq!(items_for(RadialPage::Root).len(), 7);
    }

    #[test]
    fn chibi_is_one_direct_root_command_with_a_resource_derived_label() {
        let chibi_entries: Vec<_> = ROOT_ITEMS
            .iter()
            .chain(CLOUD_ITEMS.iter())
            .filter(|item| {
                matches!(
                    item.action,
                    RadialItemAction::Command(NeraActionMenuCommand::Chibi)
                )
            })
            .collect();
        assert_eq!(chibi_entries.len(), 1);
        assert!(ROOT_ITEMS.iter().any(|item| item.label == "Chibi"));
        assert_eq!(chibi_label(AvatarBodyMode::Normal), "Chibi: Off");
        assert_eq!(chibi_label(AvatarBodyMode::Chibi), "Chibi: On");
    }

    #[test]
    fn settings_remains_reachable_without_a_duplicate_roaming_page() {
        assert!(ROOT_ITEMS.iter().any(|item| matches!(
            item.action,
            RadialItemAction::Command(NeraActionMenuCommand::Settings)
        )));
        assert!(!ROOT_ITEMS.iter().any(|item| item.label == "Roaming"));
    }

    #[test]
    fn top_sector_is_first_item() {
        let x = WHEEL_CENTER.round() as i32;
        let y = (WHEEL_CENTER - ITEM_RADIUS).round() as i32;

        assert_eq!(sector_at(RadialPage::Root, x, y), Some(0));
    }

    #[test]
    fn seven_root_sectors_preserve_old_actions_and_add_screensaver() {
        assert_eq!(ROOT_ITEMS.len(), 7);
        assert!(matches!(ROOT_ITEMS[4].action, RadialItemAction::Disabled));
        assert!(matches!(
            ROOT_ITEMS[5].action,
            RadialItemAction::Command(NeraActionMenuCommand::Chibi)
        ));
        assert!(matches!(
            ROOT_ITEMS[6].action,
            RadialItemAction::Command(NeraActionMenuCommand::Screensaver)
        ));
        for index in 0..ROOT_ITEMS.len() {
            let angle = index as f32 * TAU / ROOT_ITEMS.len() as f32 - PI * 0.5;
            let x = (WHEEL_CENTER + ITEM_RADIUS * angle.cos()).round() as i32;
            let y = (WHEEL_CENTER + ITEM_RADIUS * angle.sin()).round() as i32;
            assert_eq!(sector_at(RadialPage::Root, x, y), Some(index));
        }
    }

    #[test]
    fn center_is_not_a_sector() {
        assert_eq!(
            sector_at(RadialPage::Root, WHEEL_CENTER as i32, WHEEL_CENTER as i32,),
            None
        );
    }

    #[test]
    fn animation_manager_and_alarm_keep_typed_commands() {
        assert!(matches!(
            ROOT_ITEMS[2].action,
            RadialItemAction::Command(NeraActionMenuCommand::AnimationManager)
        ));

        assert!(matches!(
            ROOT_ITEMS[3].action,
            RadialItemAction::Command(NeraActionMenuCommand::AlarmSettings)
        ));
    }

    /// Wheel geometry is derived from `items.len()`, so pages may legitimately
    /// differ in segment count. The invariant that matters is navigation: every
    /// page that has a parent must offer a way back, and Root — which has no
    /// parent — must not.
    #[test]
    fn every_sub_page_ends_with_a_back_item() {
        for page in [RadialPage::Root, RadialPage::Cloud] {
            let items = items_for(page);
            assert!(!items.is_empty(), "{page:?}");

            let last_is_back = matches!(items[items.len() - 1].action, RadialItemAction::Back);

            assert_eq!(last_is_back, parent_page(page).is_some(), "{page:?}",);
        }
    }

    #[test]
    fn alarm_has_one_direct_editor_entry_without_preset_pages() {
        let alarm_entries: Vec<_> = ROOT_ITEMS
            .iter()
            .filter(|item| item.label == "Alarm")
            .collect();
        assert_eq!(alarm_entries.len(), 1);
        assert!(matches!(
            alarm_entries[0].action,
            RadialItemAction::Command(NeraActionMenuCommand::AlarmSettings)
        ));
        assert!(
            !ROOT_ITEMS
                .iter()
                .chain(CLOUD_ITEMS.iter())
                .any(|item| matches!(
                    item.action,
                    RadialItemAction::Command(NeraActionMenuCommand::Alarm(_))
                ))
        );
    }
}
