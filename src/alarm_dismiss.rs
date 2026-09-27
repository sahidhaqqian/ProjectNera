//! Project Nera — alarm dismiss panel.
//!
//! Phase 5.75 originally used a small red box that drifted around the monitor.
//! That worked as a lifecycle proof, but it looked like a debug affordance rather
//! than a real alarm UI. The dismiss surface is now a compact utility panel that
//! deliberately shares the same visual language as Nera Settings:
//!
//! - transparent native window + rounded Bevy shell
//! - dark navy cards and outlines
//! - custom title strip
//! - large primary action
//! - always-on-top without stealing keyboard focus on spawn
//!
//! `Turn off the Alarm` exits Attention through the normal presentation lifecycle.
//! `Snooze 5 min` stops the current ring, arms one new deadline, then restores
//! DesktopPet through that same presentation exit path.

use std::time::{Duration, Instant};

use bevy::{
    camera::RenderTarget,
    prelude::*,
    ui::UiTargetCamera,
    window::{WindowCloseRequested, WindowLevel, WindowPosition, WindowRef},
};

use crate::{
    alarm::{AlarmControlCommand, AlarmDisplayInfo, DEFAULT_SNOOZE_DURATION},
    environment::DesktopEnvironment,
    platform::{self, LocalDateTime, NativeWindowHandle, PhysicalPoint, PhysicalRect},
    presentation::{
        AttentionKind, PresentationCommand, PresentationEntered,
        PresentationExited, PresentationState,
    },
};

// -----------------------------------------------------------------------------
// Tuning
// -----------------------------------------------------------------------------

const PANEL_WIDTH_LOGICAL: u32 = 520;
const PANEL_HEIGHT_LOGICAL: u32 = 520;

/// Native placement margin in physical desktop pixels. The panel is parked on
/// the right side of the active monitor, visually beside Nera's Attention pose.
const PANEL_EDGE_MARGIN_PHYSICAL: i32 = 56;

/// If the panel never becomes reachable, dismiss the alarm instead of trapping
/// the user behind looping audio with no usable control.
const SPAWN_FAILSAFE: Duration = Duration::from_secs(2);

// Settings-derived visual language.
const SHELL_COLOR: Color = Color::srgba(0.032, 0.042, 0.078, 0.985);
const SHELL_OUTLINE: Color = Color::srgba(0.18, 0.23, 0.38, 0.85);
const CARD_COLOR: Color = Color::srgba(0.055, 0.069, 0.118, 0.97);
const CARD_OUTLINE: Color = Color::srgba(0.12, 0.16, 0.28, 0.9);
const TITLE_COLOR: Color = Color::srgb(0.89, 0.91, 1.0);
const MUTED_TEXT: Color = Color::srgb(0.45, 0.51, 0.68);
const BODY_TEXT: Color = Color::srgb(0.82, 0.85, 0.94);
const SUBTLE_TEXT: Color = Color::srgb(0.62, 0.67, 0.8);

// Primary button follows the blue accent from the approved mockup rather than
// Settings' neutral buttons. Everything around it still matches Settings.
const PRIMARY_COLOR: Color = Color::srgb(0.22, 0.43, 0.92);
const PRIMARY_HOVER: Color = Color::srgb(0.29, 0.51, 1.0);
const PRIMARY_PRESSED: Color = Color::srgb(0.16, 0.34, 0.76);
const PRIMARY_OUTLINE: Color = Color::srgba(0.52, 0.68, 1.0, 0.72);
const PRIMARY_TEXT: Color = Color::srgb(0.97, 0.98, 1.0);

const SECONDARY_COLOR: Color = Color::srgb(0.095, 0.112, 0.17);
const SECONDARY_HOVER: Color = Color::srgb(0.13, 0.15, 0.22);
const SECONDARY_PRESSED: Color = Color::srgb(0.07, 0.085, 0.135);
const SECONDARY_OUTLINE: Color = Color::srgba(0.24, 0.28, 0.39, 0.9);
const SECONDARY_TEXT: Color = Color::srgb(0.88, 0.9, 0.96);

// -----------------------------------------------------------------------------
// Markers & runtime
// -----------------------------------------------------------------------------

#[derive(Component)]
pub(crate) struct AlarmDismissWindow;

/// Garbage-collection tag mirroring `settings::SettingsOwned`.
#[derive(Component)]
struct AlarmDismissOwned {
    window: Entity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AlarmPanelAction {
    Dismiss,
    SnoozeFiveMinutes,
}

#[derive(Component, Clone, Copy)]
struct AlarmActionButton {
    action: AlarmPanelAction,
    normal: Color,
    hovered: Color,
    pressed: Color,
}

#[derive(Component)]
struct AlarmDismissDragRegion;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AlarmPanelSnapshot {
    time: String,
    meridiem: &'static str,
    date: String,
    label: String,
}

impl AlarmPanelSnapshot {
    fn from_alarm(local: LocalDateTime, alarm: &AlarmDisplayInfo) -> Self {
        let (time, meridiem) =
            format_alarm_time(alarm.hour as u16, alarm.minute as u16);

        Self {
            time,
            meridiem,
            date: format_alarm_date(local),
            label: alarm.label.clone(),
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct AlarmDismissRuntime {
    window: Option<Entity>,
    hwnd: Option<NativeWindowHandle>,
    requested_at: Option<Instant>,
    failsafe_done: bool,
    revealed: bool,
    pending_action: Option<AlarmPanelAction>,
}

pub(crate) struct AlarmDismissPlugin;

impl Plugin for AlarmDismissPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AlarmDismissRuntime>()
            // HWND lookup goes through winit's thread-local window map, so it
            // must run on the main thread.
            .add_systems(First, acquire_dismiss_hwnd)
            .add_systems(
                Update,
                (
                    spawn_dismiss_panel,
                    place_dismiss_panel,
                    drag_dismiss_panel,
                    update_alarm_action_buttons,
                    handle_native_close,
                    commit_alarm_action_after_release,
                    spawn_failsafe,
                    cleanup_dismiss_panel,
                    collect_orphaned_dismiss_entities,
                )
                    .chain(),
            );
    }
}

// -----------------------------------------------------------------------------
// Placement
// -----------------------------------------------------------------------------

/// Park the panel on the right side of the active monitor and vertically center
/// it. Works with negative desktop coordinates used by left/top secondary
/// monitors and degrades to the monitor origin if the panel is larger than the
/// usable bounds.
fn panel_target(
    bounds: PhysicalRect,
    panel_width: i32,
    panel_height: i32,
) -> PhysicalPoint {
    let panel_width = panel_width.max(1);
    let panel_height = panel_height.max(1);

    let min_x = bounds.left;
    let min_y = bounds.top;
    let max_x = (bounds.right - panel_width).max(min_x);
    let max_y = (bounds.bottom - panel_height).max(min_y);

    let preferred_x = bounds
        .right
        .saturating_sub(panel_width)
        .saturating_sub(PANEL_EDGE_MARGIN_PHYSICAL);
    let centered_y = bounds
        .top
        .saturating_add((bounds.height().saturating_sub(panel_height)) / 2);

    PhysicalPoint::new(
        preferred_x.clamp(min_x, max_x),
        centered_y.clamp(min_y, max_y),
    )
}

// -----------------------------------------------------------------------------
// Spawn / HWND
// -----------------------------------------------------------------------------

fn spawn_dismiss_panel(
    mut entered: MessageReader<PresentationEntered>,
    mut runtime: ResMut<AlarmDismissRuntime>,
    windows: Query<Entity, With<AlarmDismissWindow>>,
    alarm: Res<AlarmDisplayInfo>,
    mut commands: Commands,
) {
    if !entered
        .read()
        .any(|event| event.kind == AttentionKind::Alarm)
    {
        return;
    }

    // Never stack two alarm panels.
    if runtime.window.is_some() || !windows.is_empty() {
        return;
    }

    runtime.reset();
    runtime.requested_at = Some(Instant::now());

    let window_entity = commands
        .spawn((
            Name::new("Nera Alarm Dismiss Window"),
            AlarmDismissWindow,
            Window {
                title: "Nera Alarm".into(),
                resolution: (PANEL_WIDTH_LOGICAL, PANEL_HEIGHT_LOGICAL).into(),
                resizable: false,
                decorations: false,
                transparent: true,
                window_level: WindowLevel::AlwaysOnTop,
                position: WindowPosition::Automatic,
                skip_taskbar: true,
                // Alarm presentation must not steal keyboard focus from the app
                // the user was using when it fired.
                focused: false,
                // Reveal only after native placement succeeds, preventing a
                // one-frame flash at winit's automatic position.
                visible: false,
                ..default()
            },
        ))
        .id();

    let camera_entity = commands
        .spawn((
            Name::new("Nera Alarm Dismiss Camera"),
            AlarmDismissOwned {
                window: window_entity,
            },
            Camera2d,
            Camera {
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Window(WindowRef::Entity(window_entity)),
        ))
        .id();

    let snapshot = AlarmPanelSnapshot::from_alarm(
        platform::local_datetime(),
        &alarm,
    );

    spawn_alarm_panel_ui(
        &mut commands,
        window_entity,
        camera_entity,
        snapshot,
    );

    runtime.window = Some(window_entity);

    info!(
        "Nera alarm dismiss panel spawned | {}x{} logical",
        PANEL_WIDTH_LOGICAL, PANEL_HEIGHT_LOGICAL,
    );
}

fn spawn_alarm_panel_ui(
    commands: &mut Commands,
    window: Entity,
    camera: Entity,
    snapshot: AlarmPanelSnapshot,
) {
    commands
        .spawn((
            Name::new("Nera Alarm Dismiss Root"),
            AlarmDismissOwned { window },
            UiTargetCamera(camera),
            Node {
                width: percent(100),
                height: percent(100),
                padding: UiRect::all(px(14)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::NONE),
        ))
        .with_children(|root| {
            root.spawn((
                Name::new("Nera Alarm Shell"),
                Node {
                    width: percent(100),
                    height: percent(100),
                    padding: UiRect::all(px(20)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(12),
                    border_radius: BorderRadius::all(px(28)),
                    ..default()
                },
                BackgroundColor(SHELL_COLOR),
                Outline::new(px(1), Val::ZERO, SHELL_OUTLINE),
                BoxShadow::new(
                    Color::srgba(0.0, 0.0, 0.0, 0.55),
                    px(0),
                    px(8),
                    px(0),
                    px(22),
                ),
            ))
            .with_children(|panel| {
                spawn_alarm_header(panel);
                spawn_alarm_card(panel, &snapshot);

                // Actions live outside the information card. Keeping them in a
                // dedicated footer prevents the old layout from overflowing the
                // rounded shell on smaller DPI/font combinations.
                spawn_alarm_action_button(
                    panel,
                    "Nera Turn Off Alarm Button",
                    "Turn off the Alarm",
                    AlarmPanelAction::Dismiss,
                    PRIMARY_COLOR,
                    PRIMARY_HOVER,
                    PRIMARY_PRESSED,
                    PRIMARY_OUTLINE,
                    PRIMARY_TEXT,
                    60.0,
                    18.0,
                );

                spawn_alarm_action_button(
                    panel,
                    "Nera Snooze Alarm Button",
                    "Snooze 5 min",
                    AlarmPanelAction::SnoozeFiveMinutes,
                    SECONDARY_COLOR,
                    SECONDARY_HOVER,
                    SECONDARY_PRESSED,
                    SECONDARY_OUTLINE,
                    SECONDARY_TEXT,
                    50.0,
                    15.0,
                );
            });
        });
}

fn spawn_alarm_header(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Name::new("Nera Alarm Header"),
            Button,
            AlarmDismissDragRegion,
            Node {
                width: percent(100),
                height: px(54),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::NONE),
        ))
        .with_children(|header| {
            header
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    ..default()
                })
                .with_children(|copy| {
                    copy.spawn((
                        Text::new("Alarm"),
                        TextFont {
                            font_size: FontSize::Px(24.0),
                            ..default()
                        },
                        TextColor(TITLE_COLOR),
                    ));

                    copy.spawn((
                        Text::new("Project Nera"),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                        TextColor(MUTED_TEXT),
                    ));
                });

            header
                .spawn((
                    Node {
                        padding: UiRect::axes(px(11), px(6)),
                        border_radius: BorderRadius::all(px(999)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.22, 0.43, 0.92, 0.14)),
                    Outline::new(
                        px(1),
                        Val::ZERO,
                        Color::srgba(0.34, 0.52, 1.0, 0.32),
                    ),
                ))
                .with_children(|pill| {
                    pill.spawn((
                        Text::new("RINGING"),
                        TextFont {
                            font_size: FontSize::Px(11.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.58, 0.7, 1.0)),
                    ));
                });
        });
}

fn spawn_alarm_card(
    parent: &mut ChildSpawnerCommands,
    snapshot: &AlarmPanelSnapshot,
) {
    parent
        .spawn((
            Name::new("Nera Alarm Card"),
            Node {
                width: percent(100),
                flex_grow: 1.0,
                min_height: px(250),
                padding: UiRect::axes(px(22), px(18)),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: px(10),
                border_radius: BorderRadius::all(px(20)),
                ..default()
            },
            BackgroundColor(CARD_COLOR),
            Outline::new(px(1), Val::ZERO, CARD_OUTLINE),
        ))
        .with_children(|card| {
            card.spawn((
                Text::new(snapshot.label.clone()),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(SUBTLE_TEXT),
            ));

            card.spawn(Node {
                width: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Baseline,
                column_gap: px(10),
                ..default()
            })
            .with_children(|time| {
                time.spawn((
                    Text::new(snapshot.time.clone()),
                    TextFont {
                        font_size: FontSize::Px(62.0),
                        ..default()
                    },
                    TextColor(PRIMARY_TEXT),
                ));

                time.spawn((
                    Text::new(snapshot.meridiem),
                    TextFont {
                        font_size: FontSize::Px(25.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.42, 0.61, 1.0)),
                ));
            });

            card.spawn((
                Text::new(snapshot.date.clone()),
                TextFont {
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(BODY_TEXT),
            ));

            card.spawn((
                Node {
                    width: percent(64),
                    height: px(1),
                    margin: UiRect::axes(px(0), px(5)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.23, 0.28, 0.42, 0.52)),
            ));

            card.spawn((
                Text::new("A small reminder from Nera."),
                TextFont {
                    font_size: FontSize::Px(13.0),
                    ..default()
                },
                TextColor(SUBTLE_TEXT),
            ));
        });
}

#[allow(clippy::too_many_arguments)]
fn spawn_alarm_action_button(
    parent: &mut ChildSpawnerCommands,
    name: &'static str,
    label: &'static str,
    action: AlarmPanelAction,
    normal: Color,
    hovered: Color,
    pressed: Color,
    outline: Color,
    text_color: Color,
    height: f32,
    font_size: f32,
) {
    parent
        .spawn((
            Name::new(name),
            AlarmActionButton {
                action,
                normal,
                hovered,
                pressed,
            },
            Button,
            Node {
                width: percent(100),
                height: px(height),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(px(16)),
                ..default()
            },
            BackgroundColor(normal),
            Outline::new(px(1), Val::ZERO, outline),
            BoxShadow::new(
                Color::srgba(0.0, 0.0, 0.0, 0.24),
                px(0),
                px(5),
                px(0),
                px(12),
            ),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(font_size),
                    ..default()
                },
                TextColor(text_color),
            ));
        });
}

fn format_alarm_time(hour_24: u16, minute: u16) -> (String, &'static str) {
    let hour_24 = hour_24.min(23);
    let minute = minute.min(59);
    let meridiem = if hour_24 < 12 { "AM" } else { "PM" };
    let hour_12 = match hour_24 % 12 {
        0 => 12,
        hour => hour,
    };

    (format!("{hour_12:02}:{minute:02}"), meridiem)
}

fn format_alarm_date(local: LocalDateTime) -> String {
    const DAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];

    let day_name = DAYS
        .get(local.day_of_week as usize)
        .copied()
        .unwrap_or("Today");
    let month_name = local
        .month
        .checked_sub(1)
        .and_then(|month| MONTHS.get(month as usize))
        .copied()
        .unwrap_or("Unknown");

    format!("{day_name}, {} {month_name}", local.day)
}

/// Exclusive because `platform::native_window_handle` reads winit's
/// thread-local window map, which is only valid on the main thread.
fn acquire_dismiss_hwnd(world: &mut World) {
    if world.resource::<AlarmDismissRuntime>().hwnd.is_some() {
        return;
    }

    let Some(window_entity) = world.resource::<AlarmDismissRuntime>().window
    else {
        return;
    };

    let Some(handle) = platform::native_window_handle(window_entity) else {
        return;
    };

    info!(
        "Nera alarm dismiss HWND acquired: 0x{:X}",
        handle.as_raw() as usize,
    );

    world.resource_mut::<AlarmDismissRuntime>().hwnd = Some(handle);
}

fn place_dismiss_panel(
    environment: Res<DesktopEnvironment>,
    mut runtime: ResMut<AlarmDismissRuntime>,
    mut window_query: Query<&mut Window, With<AlarmDismissWindow>>,
) {
    if runtime.revealed {
        return;
    }

    let Some(hwnd) = runtime.hwnd else {
        return;
    };

    let Some(rect) = platform::window_rect(hwnd) else {
        return;
    };

    let Some(bounds) = environment
        .monitor
        .map(|monitor| monitor.bounds)
        .or_else(|| platform::monitor_info(hwnd).map(|monitor| monitor.bounds))
    else {
        trace!("Nera alarm panel has no monitor bounds yet");
        return;
    };

    let target = panel_target(bounds, rect.width(), rect.height());

    if platform::move_window(hwnd, target).is_err() {
        trace!("Nera alarm panel could not be placed this frame");
        return;
    }

    if let Ok(mut window) = window_query.single_mut() {
        if !window.visible {
            window.visible = true;
        }
    }

    // Desired Bevy visibility and actual HWND visibility are not identical.
    // Commit only after Windows confirms the panel is truly reachable.
    if platform::window_is_visible(hwnd) {
        runtime.revealed = true;
        info!(
            "Nera alarm dismiss panel visible | target={},{}",
            target.x, target.y,
        );
    }
}

// -----------------------------------------------------------------------------
// Interaction
// -----------------------------------------------------------------------------

fn drag_dismiss_panel(
    mouse: Res<ButtonInput<MouseButton>>,
    drag_regions: Query<&Interaction, (Changed<Interaction>, With<AlarmDismissDragRegion>)>,
    mut windows: Query<&mut Window, With<AlarmDismissWindow>>,
) {
    // Same rule as Settings: winit expects start_drag_move() directly from a
    // real left-button press.
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    let wants_drag = drag_regions
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed);

    if !wants_drag {
        return;
    }

    if let Ok(mut window) = windows.single_mut() {
        window.start_drag_move();
    }
}

fn update_alarm_action_buttons(
    mut buttons: Query<
        (&Interaction, &AlarmActionButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
    mut runtime: ResMut<AlarmDismissRuntime>,
) {
    for (interaction, button, mut background) in &mut buttons {
        let next = match interaction {
            Interaction::Pressed => button.pressed,
            Interaction::Hovered => button.hovered,
            Interaction::None => button.normal,
        };

        if background.0 != next {
            background.0 = next;
        }

        if *interaction == Interaction::Pressed && runtime.pending_action.is_none() {
            runtime.pending_action = Some(button.action);

            info!(
                "Nera alarm action requested: {:?}; waiting for mouse release",
                button.action,
            );
        }
    }
}

/// Alt+F4/native close has the same meaning as Turn off. Bevy removes the
/// native window in Last; request presentation teardown in Update while its
/// identity is still known, so PresentationExited stops audio and animation.
fn handle_native_close(
    mut requests: MessageReader<WindowCloseRequested>,
    windows: Query<(), With<AlarmDismissWindow>>,
    mut runtime: ResMut<AlarmDismissRuntime>,
    mut presentation: MessageWriter<PresentationCommand>,
) {
    let requested = requests
        .read()
        .any(|request| Some(request.window) == runtime.window);
    let disappeared = runtime
        .window
        .is_some_and(|window| !windows.contains(window));
    if requested || disappeared {
        // A close cancels a not-yet-committed button gesture, including snooze.
        runtime.pending_action = None;
        presentation.write(PresentationCommand::ExitAttention);
    }
}

/// True once every mouse button is up.
///
/// Despawning the native window underneath a held button can make winit miss
/// mouse-up and leave global ButtonInput stuck in the pressed state.
fn gesture_released(left: bool, right: bool, middle: bool) -> bool {
    !left && !right && !middle
}

fn commit_alarm_action_after_release(
    mouse: Res<ButtonInput<MouseButton>>,
    mut runtime: ResMut<AlarmDismissRuntime>,
    mut presentation: MessageWriter<PresentationCommand>,
    mut alarm_control: MessageWriter<AlarmControlCommand>,
) {
    let Some(action) = runtime.pending_action else {
        return;
    };

    if !gesture_released(
        mouse.pressed(MouseButton::Left),
        mouse.pressed(MouseButton::Right),
        mouse.pressed(MouseButton::Middle),
    ) {
        return;
    }

    runtime.pending_action = None;

    match action {
        AlarmPanelAction::Dismiss => {
            presentation.write(PresentationCommand::ExitAttention);
        }
        AlarmPanelAction::SnoozeFiveMinutes => {
            alarm_control.write(AlarmControlCommand::Snooze(
                DEFAULT_SNOOZE_DURATION,
            ));
        }
    }
}

// -----------------------------------------------------------------------------
// Failsafe
// -----------------------------------------------------------------------------

fn failsafe_should_fire(
    window_spawned: bool,
    revealed: bool,
    already_fired: bool,
    elapsed: Option<Duration>,
) -> bool {
    if already_fired || !window_spawned || revealed {
        return false;
    }

    matches!(elapsed, Some(elapsed) if elapsed >= SPAWN_FAILSAFE)
}

fn spawn_failsafe(
    mut runtime: ResMut<AlarmDismissRuntime>,
    mut presentation: MessageWriter<PresentationCommand>,
) {
    let fired = failsafe_should_fire(
        runtime.window.is_some(),
        runtime.revealed,
        runtime.failsafe_done,
        runtime.requested_at.map(|at| at.elapsed()),
    );

    if !fired {
        return;
    }

    runtime.failsafe_done = true;

    error!(
        "Nera alarm dismiss panel never reached the screen within {:?}; \
         dismissing the alarm instead of trapping the user",
        SPAWN_FAILSAFE,
    );

    presentation.write(PresentationCommand::ExitAttention);
}

// -----------------------------------------------------------------------------
// Teardown
// -----------------------------------------------------------------------------

fn cleanup_dismiss_panel(
    mut exited: MessageReader<PresentationExited>,
    presentation: Res<PresentationState>,
    mut runtime: ResMut<AlarmDismissRuntime>,
    windows: Query<Entity, With<AlarmDismissWindow>>,
    mut commands: Commands,
) {
    let dismissed = exited
        .read()
        .any(|event| event.kind == AttentionKind::Alarm);

    // Defensive: an always-on-top utility window must never survive after the
    // presentation has already returned to DesktopPet.
    let stranded = runtime.window.is_some() && presentation.is_desktop_pet();

    if !dismissed && !stranded {
        return;
    }

    for window in &windows {
        commands.entity(window).despawn();
    }

    runtime.reset();
}

fn collect_orphaned_dismiss_entities(
    dismiss_windows: Query<(), With<AlarmDismissWindow>>,
    owned: Query<(Entity, &AlarmDismissOwned)>,
    mut commands: Commands,
) {
    for (entity, owner) in &owned {
        if dismiss_windows.get(owner.window).is_ok() {
            continue;
        }

        commands.entity(entity).despawn();
    }
}

impl AlarmDismissRuntime {
    pub(crate) fn is_closed(&self) -> bool {
        self.window.is_none()
    }

    #[cfg(test)]
    pub(crate) fn with_window_for_test(window: Entity) -> Self {
        Self {
            window: Some(window),
            ..default()
        }
    }

    fn reset(&mut self) {
        self.window = None;
        self.hwnd = None;
        self.requested_at = None;
        self.failsafe_done = false;
        self.revealed = false;
        self.pending_action = None;
    }
}

// -----------------------------------------------------------------------------
// Pure tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn dismiss_action_app() -> (App, Entity) {
        let mut app = App::new();
        app.init_resource::<AlarmDismissRuntime>()
            .init_resource::<PresentationState>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<WindowCloseRequested>()
            .add_message::<PresentationCommand>()
            .add_message::<PresentationExited>()
            .add_message::<AlarmControlCommand>()
            .add_systems(
                Update,
                (handle_native_close, commit_alarm_action_after_release).chain(),
            );
        let panel = app.world_mut().spawn(AlarmDismissWindow).id();
        app.world_mut().resource_mut::<AlarmDismissRuntime>().window = Some(panel);
        (app, panel)
    }

    #[test]
    fn native_close_exits_attention_and_cancels_uncommitted_snooze() {
        let (mut app, panel) = dismiss_action_app();
        app.world_mut()
            .resource_mut::<AlarmDismissRuntime>()
            .pending_action = Some(AlarmPanelAction::SnoozeFiveMinutes);
        app.world_mut()
            .write_message(WindowCloseRequested { window: panel });
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<PresentationCommand>>()
                .drain()
                .collect::<Vec<_>>(),
            vec![PresentationCommand::ExitAttention]
        );
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<AlarmControlCommand>>()
                .drain()
                .count(),
            0
        );
        assert!(
            app.world()
                .resource::<AlarmDismissRuntime>()
                .pending_action
                .is_none()
        );
    }

    #[test]
    fn native_close_of_another_window_does_not_dismiss_alarm() {
        let (mut app, _) = dismiss_action_app();
        let other = app.world_mut().spawn_empty().id();
        app.world_mut()
            .write_message(WindowCloseRequested { window: other });
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<PresentationCommand>>()
                .drain()
                .count(),
            0
        );
    }

    #[test]
    fn disappeared_panel_also_exits_attention_after_it_was_revealed() {
        let (mut app, panel) = dismiss_action_app();
        app.world_mut()
            .resource_mut::<AlarmDismissRuntime>()
            .revealed = true;
        app.world_mut().despawn(panel);
        app.update();
        assert_eq!(
            app.world_mut()
                .resource_mut::<Messages<PresentationCommand>>()
                .drain()
                .collect::<Vec<_>>(),
            vec![PresentationCommand::ExitAttention]
        );
    }

    #[test]
    fn panel_sits_on_the_right_and_is_vertically_centered() {
        let bounds = PhysicalRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };

        let target = panel_target(bounds, 500, 470);

        assert_eq!(target.x, 1920 - 500 - PANEL_EDGE_MARGIN_PHYSICAL);
        assert_eq!(target.y, (1080 - 470) / 2);
    }

    #[test]
    fn panel_placement_supports_negative_monitor_origins() {
        let bounds = PhysicalRect {
            left: -1920,
            top: -300,
            right: 0,
            bottom: 780,
        };

        let target = panel_target(bounds, 500, 470);

        assert!(target.x >= bounds.left);
        assert!(target.x + 500 <= bounds.right);
        assert!(target.y >= bounds.top);
        assert!(target.y + 470 <= bounds.bottom);
    }

    #[test]
    fn panel_larger_than_monitor_degrades_to_monitor_origin() {
        let bounds = PhysicalRect {
            left: 100,
            top: 200,
            right: 380,
            bottom: 420,
        };

        let target = panel_target(bounds, 500, 470);

        assert_eq!(target, PhysicalPoint::new(100, 200));
    }

    #[test]
    fn alarm_time_uses_twelve_hour_clock() {
        assert_eq!(format_alarm_time(0, 5), ("12:05".into(), "AM"));
        assert_eq!(format_alarm_time(9, 7), ("09:07".into(), "AM"));
        assert_eq!(format_alarm_time(12, 0), ("12:00".into(), "PM"));
        assert_eq!(format_alarm_time(23, 59), ("11:59".into(), "PM"));
    }

    #[test]
    fn alarm_date_matches_windows_day_and_month_indices() {
        let local = LocalDateTime {
            year: 2026,
            month: 9,
            day: 23,
            day_of_week: 3,
            hour: 7,
            minute: 30,
        };

        assert_eq!(format_alarm_date(local), "Wednesday, 23 September");
    }

    #[test]
    fn failsafe_fires_only_when_panel_never_reached_the_screen() {
        assert!(!failsafe_should_fire(
            true,
            true,
            false,
            Some(Duration::from_secs(30)),
        ));

        assert!(!failsafe_should_fire(
            true,
            false,
            false,
            Some(Duration::from_millis(500)),
        ));

        assert!(failsafe_should_fire(
            true,
            false,
            false,
            Some(SPAWN_FAILSAFE),
        ));

        assert!(!failsafe_should_fire(
            false,
            false,
            false,
            Some(Duration::from_secs(10)),
        ));

        assert!(!failsafe_should_fire(
            true,
            false,
            true,
            Some(Duration::from_secs(10)),
        ));
    }

    #[test]
    fn dismiss_waits_for_every_button_to_release() {
        assert!(!gesture_released(true, false, false));
        assert!(!gesture_released(false, true, false));
        assert!(!gesture_released(false, false, true));
        assert!(!gesture_released(true, true, true));
        assert!(gesture_released(false, false, false));
    }
}
