//! Project Nera — Phase 5.77 multi-alarm manager.
//!
//! The alarm utility window now behaves like a small alarm app instead of a
//! single configuration form: list, add, edit, enable/disable and delete.
//! Runtime scheduling stays in `alarm.rs`; this module only edits persistent
//! `settings::AlarmConfig` values.

use bevy::{
    camera::RenderTarget,
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
    ui::UiTargetCamera,
    window::{
        WindowLevel,
        WindowPosition,
        WindowRef,
        WindowResizeConstraints,
    },
};

use crate::settings::{
    AlarmConfig,
    NeraSettings,
    save_settings_deferred,
};
use crate::utility_window::{UtilityWindow, spawn_window_controls};

const MAX_ALARMS: usize = 12;

const SHELL: Color = Color::srgba(0.032, 0.042, 0.078, 0.985);
const CARD: Color = Color::srgba(0.055, 0.071, 0.122, 0.98);
const CARD_OUTLINE: Color = Color::srgba(0.18, 0.23, 0.38, 0.78);
const PRIMARY: Color = Color::srgb(0.94, 0.95, 1.0);
const BODY: Color = Color::srgb(0.70, 0.74, 0.84);
const MUTED: Color = Color::srgb(0.46, 0.51, 0.64);
const BLUE: Color = Color::srgb(0.20, 0.40, 0.86);
const BLUE_HOVER: Color = Color::srgb(0.25, 0.46, 0.96);
const BLUE_PRESSED: Color = Color::srgb(0.17, 0.34, 0.76);

#[derive(Message, Default)]
pub(crate) struct OpenAlarmSettingsWindow;

#[derive(Resource, Default)]
pub(crate) struct AlarmSettingsInputGate {
    window_open: bool,
    minimized: bool,
    close_requested: bool,
}

pub(crate) fn nera_input_available(gate: Res<AlarmSettingsInputGate>) -> bool {
    !gate.window_open || gate.minimized
}

pub(crate) fn nera_input_blocked(gate: Res<AlarmSettingsInputGate>) -> bool {
    gate.window_open && !gate.minimized
}

#[derive(Component)]
struct AlarmSettingsWindow;

#[derive(Component)]
struct AlarmSettingsCamera;

#[derive(Component)]
struct AlarmSettingsUiRoot;

#[derive(Component)]
struct AlarmSettingsOwned {
    window: Entity,
}

#[derive(Component)]
struct AlarmSettingsDragRegion;

#[derive(Component)]
struct AlarmSettingsScrollArea;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum AlarmSettingsView {
    #[default]
    List,
    Edit,
    TimePicker,
    DatePicker,
}

#[derive(Resource, Default)]
struct AlarmSettingsUiState {
    view: AlarmSettingsView,
    draft: Option<AlarmConfig>,
    rebuild: bool,
}

#[derive(Component, Clone, Copy, Debug)]
enum AlarmSettingsAction {
    Close,
    Add,
    Edit(u32),
    Toggle(u32),
    Delete(u32),
    Back,
    Save,
    DraftToggleEnabled,
    DraftCycleLabel,
    DraftCycleMode,
    OpenTimePicker,
    OpenDatePicker,
    DraftHourDown,
    DraftHourUp,
    DraftMinuteDown,
    DraftMinuteUp,
    DraftToggleMeridiem,
    DraftYearDown,
    DraftYearUp,
    DraftMonthDown,
    DraftMonthUp,
    DraftDayDown,
    DraftDayUp,
    DraftToggleDay(u8),
}

pub(crate) struct AlarmSettingsPlugin;

impl Plugin for AlarmSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AlarmSettingsInputGate>()
            .init_resource::<AlarmSettingsUiState>()
            .add_message::<OpenAlarmSettingsWindow>()
            .add_systems(PreUpdate, sync_alarm_window_minimized)
            .add_systems(
                Update,
                (
                    open_alarm_settings_window,
                    handle_alarm_settings_actions,
                    rebuild_alarm_settings_ui,
                    drag_alarm_settings_window,
                    scroll_alarm_settings_window,
                    commit_alarm_settings_close_after_release,
                    cleanup_alarm_settings_owned,
                    observe_alarm_settings_window_removed,
                )
                    .chain(),
            );
    }
}

fn open_alarm_settings_window(
    mut open: MessageReader<OpenAlarmSettingsWindow>,
    mut existing: Query<&mut Window, With<AlarmSettingsWindow>>,
    mut gate: ResMut<AlarmSettingsInputGate>,
    mut ui: ResMut<AlarmSettingsUiState>,
    mut commands: Commands,
) {
    if open.read().next().is_none() {
        return;
    }

    if let Ok(mut window) = existing.single_mut() {
        window.set_minimized(false);
        window.focused = true;
        gate.window_open = true;
        gate.minimized = false;
        return;
    }

    gate.window_open = true;
    gate.minimized = false;
    gate.close_requested = false;
    ui.view = AlarmSettingsView::List;
    ui.draft = None;
    ui.rebuild = true;

    let window = commands
        .spawn((
            Name::new("Nera Alarm Settings Window"),
            AlarmSettingsWindow,
            UtilityWindow::new(14.0),
            Window {
                title: "Nera Alarms".into(),
                resolution: (660, 800).into(),
                resizable: true,
                resize_constraints: WindowResizeConstraints {
                    min_width: 560.0,
                    min_height: 650.0,
                    ..default()
                },
                decorations: false,
                transparent: true,
                window_level: WindowLevel::AlwaysOnTop,
                position: WindowPosition::Automatic,
                skip_taskbar: false,
                ..default()
            },
        ))
        .id();

    commands.spawn((
        Name::new("Nera Alarm Settings Camera"),
        AlarmSettingsCamera,
        AlarmSettingsOwned { window },
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Window(WindowRef::Entity(window)),
    ));

    info!("Nera Alarm Settings -> opened (multi-alarm manager)");
}

fn sync_alarm_window_minimized(
    windows: Query<&UtilityWindow, With<AlarmSettingsWindow>>,
    mut gate: ResMut<AlarmSettingsInputGate>,
) {
    let minimized = windows.single().ok()
        .and_then(UtilityWindow::native_handle)
        .is_some_and(crate::platform::is_window_minimized);
    if gate.minimized != minimized {
        gate.minimized = minimized;
    }
}

fn rebuild_alarm_settings_ui(
    settings: Res<NeraSettings>,
    windows: Query<Entity, With<AlarmSettingsWindow>>,
    cameras: Query<Entity, With<AlarmSettingsCamera>>,
    roots: Query<Entity, With<AlarmSettingsUiRoot>>,
    mut ui: ResMut<AlarmSettingsUiState>,
    mut commands: Commands,
) {
    if !ui.rebuild {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(camera) = cameras.single() else {
        return;
    };

    for root in &roots {
        commands.entity(root).despawn();
    }

    match ui.view {
        AlarmSettingsView::List => {
            spawn_alarm_list_ui(&mut commands, window, camera, &settings);
        }
        AlarmSettingsView::Edit => {
            if let Some(draft) = ui.draft.as_ref() {
                spawn_alarm_editor_ui(&mut commands, window, camera, draft);
            } else {
                ui.view = AlarmSettingsView::List;
                spawn_alarm_list_ui(&mut commands, window, camera, &settings);
            }
        }
        AlarmSettingsView::TimePicker => {
            if let Some(draft) = ui.draft.as_ref() {
                spawn_time_picker_ui(&mut commands, window, camera, draft);
            } else {
                ui.view = AlarmSettingsView::List;
                spawn_alarm_list_ui(&mut commands, window, camera, &settings);
            }
        }
        AlarmSettingsView::DatePicker => {
            if let Some(draft) = ui.draft.as_ref() {
                spawn_date_picker_ui(&mut commands, window, camera, draft);
            } else {
                ui.view = AlarmSettingsView::List;
                spawn_alarm_list_ui(&mut commands, window, camera, &settings);
            }
        }
    }

    ui.rebuild = false;
}

fn spawn_root(
    commands: &mut Commands,
    window: Entity,
    camera: Entity,
    build: impl FnOnce(&mut ChildSpawnerCommands),
) {
    commands
        .spawn((
            Name::new("Nera Alarm Settings Root"),
            AlarmSettingsUiRoot,
            AlarmSettingsOwned { window },
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
                Node {
                    width: percent(100),
                    height: percent(100),
                    padding: UiRect::all(px(22)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(14),
                    border_radius: BorderRadius::all(px(28)),
                    ..default()
                },
                BackgroundColor(SHELL),
                Outline::new(px(1), Val::ZERO, CARD_OUTLINE),
                BoxShadow::new(
                    Color::srgba(0.0, 0.0, 0.0, 0.55),
                    px(0),
                    px(8),
                    px(0),
                    px(22),
                ),
            ))
            .with_children(build);
        });
}

fn spawn_alarm_list_ui(
    commands: &mut Commands,
    window: Entity,
    camera: Entity,
    settings: &NeraSettings,
) {
    let alarms = settings.alarm_configs();
    let enabled = alarms.iter().filter(|alarm| alarm.enabled).count();

    spawn_root(commands, window, camera, |panel| {
        spawn_header(panel, window, "Alarms", Some("Manage your Nera alarms"), false);

        panel.spawn((
            Text::new(format!(
                "{} alarm{} · {} enabled",
                alarms.len(),
                if alarms.len() == 1 { "" } else { "s" },
                enabled,
            )),
            TextFont { font_size: FontSize::Px(14.0), ..default() },
            TextColor(MUTED),
        ));

        spawn_action_button(
            panel,
            if alarms.len() >= MAX_ALARMS {
                "Alarm limit reached"
            } else {
                "+  Add Alarm"
            },
            AlarmSettingsAction::Add,
            BLUE,
        );

        panel
            .spawn((
                AlarmSettingsScrollArea,
                Node {
                    width: percent(100),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(10),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                ScrollPosition::default(),
            ))
            .with_children(|list| {
                if alarms.is_empty() {
                    list.spawn((
                        Node {
                            width: percent(100),
                            height: px(170),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(px(18)),
                            ..default()
                        },
                        BackgroundColor(CARD),
                        Outline::new(px(1), Val::ZERO, CARD_OUTLINE),
                    ))
                    .with_children(|empty| {
                        empty.spawn((
                            Text::new("No alarms yet.\nCreate one with + Add Alarm."),
                            TextFont { font_size: FontSize::Px(16.0), ..default() },
                            TextColor(BODY),
                        ));
                    });
                }

                for alarm in &alarms {
                    spawn_alarm_card(list, alarm);
                }
            });

        panel.spawn((
            Text::new("Project Nera · alarms are saved automatically"),
            TextFont { font_size: FontSize::Px(12.0), ..default() },
            TextColor(MUTED),
        ));
    });
}

fn spawn_alarm_card(parent: &mut ChildSpawnerCommands, alarm: &AlarmConfig) {
    parent
        .spawn((
            Node {
                width: percent(100),
                padding: UiRect::all(px(16)),
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
                border_radius: BorderRadius::all(px(18)),
                ..default()
            },
            BackgroundColor(CARD),
            Outline::new(px(1), Val::ZERO, CARD_OUTLINE),
        ))
        .with_children(|card| {
            card.spawn(Node {
                width: percent(100),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|top| {
                top.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(3),
                    ..default()
                })
                .with_children(|copy| {
                    copy.spawn((
                        Text::new(format_alarm_time(alarm)),
                        TextFont { font_size: FontSize::Px(30.0), ..default() },
                        TextColor(if alarm.enabled { PRIMARY } else { MUTED }),
                    ));
                    copy.spawn((
                        Text::new(alarm.label.clone()),
                        TextFont { font_size: FontSize::Px(15.0), ..default() },
                        TextColor(BODY),
                    ));
                });

                spawn_small_button(
                    top,
                    if alarm.enabled { "ON" } else { "OFF" },
                    AlarmSettingsAction::Toggle(alarm.id),
                    if alarm.enabled { BLUE } else { Color::srgb(0.10, 0.12, 0.20) },
                );
            });

            card.spawn((
                Text::new(schedule_summary(alarm)),
                TextFont { font_size: FontSize::Px(13.0), ..default() },
                TextColor(MUTED),
            ));

            card.spawn(Node {
                width: percent(100),
                column_gap: px(8),
                justify_content: JustifyContent::FlexEnd,
                ..default()
            })
            .with_children(|actions| {
                spawn_small_button(
                    actions,
                    "Edit",
                    AlarmSettingsAction::Edit(alarm.id),
                    Color::srgb(0.10, 0.13, 0.22),
                );
                spawn_small_button(
                    actions,
                    "Delete",
                    AlarmSettingsAction::Delete(alarm.id),
                    Color::srgb(0.28, 0.08, 0.11),
                );
            });
        });
}

fn spawn_alarm_editor_ui(
    commands: &mut Commands,
    window: Entity,
    camera: Entity,
    alarm: &AlarmConfig,
) {
    spawn_root(commands, window, camera, |panel| {
        spawn_header(
            panel,
            window,
            "Edit Alarm",
            Some("Schedule, repeat days, and label"),
            true,
        );

        panel
            .spawn((
                Node {
                    width: percent(100),
                    padding: UiRect::all(px(18)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: px(7),
                    border_radius: BorderRadius::all(px(20)),
                    ..default()
                },
                BackgroundColor(CARD),
                Outline::new(px(1), Val::ZERO, CARD_OUTLINE),
            ))
            .with_children(|preview| {
                preview.spawn((
                    Text::new(format_alarm_time(alarm)),
                    TextFont {
                        font_size: FontSize::Px(46.0),
                        ..default()
                    },
                    TextColor(PRIMARY),
                ));
                preview.spawn((
                    Text::new(alarm.label.clone()),
                    TextFont {
                        font_size: FontSize::Px(17.0),
                        ..default()
                    },
                    TextColor(BODY),
                ));
                preview.spawn((
                    Text::new(schedule_summary(alarm)),
                    TextFont {
                        font_size: FontSize::Px(13.0),
                        ..default()
                    },
                    TextColor(MUTED),
                ));
            });

        panel
            .spawn((
                AlarmSettingsScrollArea,
                Node {
                    width: percent(100),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    row_gap: px(10),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                ScrollPosition::default(),
            ))
            .with_children(|form| {
                spawn_setting_row(
                    form,
                    "Enabled",
                    if alarm.enabled { "ON" } else { "OFF" },
                    AlarmSettingsAction::DraftToggleEnabled,
                );
                spawn_setting_row(
                    form,
                    "Label",
                    &alarm.label,
                    AlarmSettingsAction::DraftCycleLabel,
                );
                spawn_setting_row(
                    form,
                    "Type",
                    alarm.mode_label(),
                    AlarmSettingsAction::DraftCycleMode,
                );

                // Keep the editor clean: time is one setting. Detailed controls
                // live on a dedicated picker page instead of three separate rows.
                spawn_setting_row(
                    form,
                    "Time",
                    &format_alarm_time(alarm),
                    AlarmSettingsAction::OpenTimePicker,
                );

                if alarm.mode == 0 {
                    spawn_setting_row(
                        form,
                        "Date",
                        &alarm.date_label(),
                        AlarmSettingsAction::OpenDatePicker,
                    );
                } else {
                    spawn_section_title(form, "Repeat on");
                    spawn_day_picker(form, alarm);
                    form.spawn((
                        Text::new(
                            "All days start ON. Click a day to turn it OFF. If every day is OFF, the alarm is turned OFF too.",
                        ),
                        TextFont {
                            font_size: FontSize::Px(12.0),
                            ..default()
                        },
                        TextColor(MUTED),
                    ));
                }

                form.spawn((
                    Text::new(
                        "Label selection uses presets for now; free-text labels can be added later without changing the scheduler.",
                    ),
                    TextFont {
                        font_size: FontSize::Px(12.0),
                        ..default()
                    },
                    TextColor(MUTED),
                ));
            });

        spawn_action_button(panel, "Save Alarm", AlarmSettingsAction::Save, BLUE);
    });
}

fn spawn_time_picker_ui(
    commands: &mut Commands,
    window: Entity,
    camera: Entity,
    alarm: &AlarmConfig,
) {
    spawn_root(commands, window, camera, |panel| {
        spawn_header(panel, window, "Set Time", Some("Choose when Nera should ring"), true);

        panel
            .spawn((
                Node {
                    width: percent(100),
                    padding: UiRect::all(px(24)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: px(8),
                    border_radius: BorderRadius::all(px(20)),
                    ..default()
                },
                BackgroundColor(CARD),
                Outline::new(px(1), Val::ZERO, CARD_OUTLINE),
            ))
            .with_children(|preview| {
                preview.spawn((
                    Text::new(format_alarm_time(alarm)),
                    TextFont {
                        font_size: FontSize::Px(54.0),
                        ..default()
                    },
                    TextColor(PRIMARY),
                ));
                preview.spawn((
                    Text::new("Time preview"),
                    TextFont {
                        font_size: FontSize::Px(12.0),
                        ..default()
                    },
                    TextColor(MUTED),
                ));
            });

        panel
            .spawn(Node {
                width: percent(100),
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                row_gap: px(12),
                ..default()
            })
            .with_children(|body| {
                let hour12 = match alarm.hour % 12 {
                    0 => 12,
                    value => value,
                };
                spawn_picker_stepper(
                    body,
                    "Hour",
                    &format!("{hour12:02}"),
                    AlarmSettingsAction::DraftHourDown,
                    AlarmSettingsAction::DraftHourUp,
                );
                spawn_picker_stepper(
                    body,
                    "Minute",
                    &format!("{:02}", alarm.minute),
                    AlarmSettingsAction::DraftMinuteDown,
                    AlarmSettingsAction::DraftMinuteUp,
                );
                spawn_setting_row(
                    body,
                    "AM / PM",
                    if alarm.hour < 12 { "AM" } else { "PM" },
                    AlarmSettingsAction::DraftToggleMeridiem,
                );
            });

        spawn_action_button(panel, "Done", AlarmSettingsAction::Back, BLUE);
    });
}

fn spawn_date_picker_ui(
    commands: &mut Commands,
    window: Entity,
    camera: Entity,
    alarm: &AlarmConfig,
) {
    spawn_root(commands, window, camera, |panel| {
        spawn_header(panel, window, "Set Date", Some("Choose the one-time alarm date"), true);

        panel
            .spawn((
                Node {
                    width: percent(100),
                    padding: UiRect::all(px(24)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: px(8),
                    border_radius: BorderRadius::all(px(20)),
                    ..default()
                },
                BackgroundColor(CARD),
                Outline::new(px(1), Val::ZERO, CARD_OUTLINE),
            ))
            .with_children(|preview| {
                preview.spawn((
                    Text::new(alarm.date_label()),
                    TextFont {
                        font_size: FontSize::Px(38.0),
                        ..default()
                    },
                    TextColor(PRIMARY),
                ));
                preview.spawn((
                    Text::new("Date preview"),
                    TextFont {
                        font_size: FontSize::Px(12.0),
                        ..default()
                    },
                    TextColor(MUTED),
                ));
            });

        panel
            .spawn(Node {
                width: percent(100),
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                row_gap: px(12),
                ..default()
            })
            .with_children(|body| {
                spawn_picker_stepper(
                    body,
                    "Year",
                    &alarm.year.to_string(),
                    AlarmSettingsAction::DraftYearDown,
                    AlarmSettingsAction::DraftYearUp,
                );
                spawn_picker_stepper(
                    body,
                    "Month",
                    &format!("{:02}", alarm.month),
                    AlarmSettingsAction::DraftMonthDown,
                    AlarmSettingsAction::DraftMonthUp,
                );
                spawn_picker_stepper(
                    body,
                    "Day",
                    &format!("{:02}", alarm.day),
                    AlarmSettingsAction::DraftDayDown,
                    AlarmSettingsAction::DraftDayUp,
                );
            });

        spawn_action_button(panel, "Done", AlarmSettingsAction::Back, BLUE);
    });
}

fn spawn_picker_stepper(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    value: &str,
    down: AlarmSettingsAction,
    up: AlarmSettingsAction,
) {
    parent
        .spawn((
            Node {
                width: percent(100),
                min_height: px(64),
                padding: UiRect::axes(px(15), px(10)),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(px(15)),
                ..default()
            },
            BackgroundColor(CARD),
            Outline::new(px(1), Val::ZERO, CARD_OUTLINE),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(BODY),
            ));

            row.spawn(Node {
                align_items: AlignItems::Center,
                column_gap: px(8),
                ..default()
            })
            .with_children(|controls| {
                spawn_small_button(
                    controls,
                    "-",
                    down,
                    Color::srgb(0.095, 0.12, 0.21),
                );
                controls.spawn((
                    Node {
                        min_width: px(76),
                        height: px(36),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(px(11)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.075, 0.095, 0.17)),
                ))
                .with_children(|value_box| {
                    value_box.spawn((
                        Text::new(value),
                        TextFont {
                            font_size: FontSize::Px(15.0),
                            ..default()
                        },
                        TextColor(PRIMARY),
                    ));
                });
                spawn_small_button(
                    controls,
                    "+",
                    up,
                    Color::srgb(0.095, 0.12, 0.21),
                );
            });
        });
}

fn spawn_header(
    parent: &mut ChildSpawnerCommands,
    window: Entity,
    title: &str,
    subtitle: Option<&str>,
    show_back: bool,
) {
    parent
        .spawn(Node {
            width: percent(100),
            height: px(64),
            flex_shrink: 0.0,
            align_items: AlignItems::Center,
            column_gap: px(10),
            ..default()
        })
        .with_children(|header| {
            if show_back {
                spawn_small_button(
                    header,
                    "< Back",
                    AlarmSettingsAction::Back,
                    Color::srgb(0.09, 0.11, 0.19),
                );
            }

            header
                .spawn((
                    Button,
                    AlarmSettingsDragRegion,
                    Node {
                        flex_grow: 1.0,
                        min_width: px(0),
                        height: percent(100),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::Center,
                        row_gap: px(2),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                ))
                .with_children(|drag| {
                    drag.spawn((
                        Text::new(title),
                        TextFont { font_size: FontSize::Px(25.0), ..default() },
                        TextColor(PRIMARY),
                    ));
                    if let Some(subtitle) = subtitle {
                        drag.spawn((
                            Text::new(subtitle),
                            TextFont { font_size: FontSize::Px(12.0), ..default() },
                            TextColor(MUTED),
                        ));
                    }
                });

            spawn_window_controls(header, window);
            spawn_small_button(
                header,
                "X",
                AlarmSettingsAction::Close,
                Color::srgb(0.09, 0.11, 0.19),
            );
        });
}

fn spawn_section_title(parent: &mut ChildSpawnerCommands, title: &str) {
    parent.spawn((
        Text::new(title),
        TextFont { font_size: FontSize::Px(13.0), ..default() },
        TextColor(Color::srgb(0.50, 0.58, 0.78)),
    ));
}

fn spawn_setting_row(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    value: &str,
    action: AlarmSettingsAction,
) {
    parent
        .spawn((
            Node {
                width: percent(100),
                min_height: px(56),
                padding: UiRect::axes(px(15), px(10)),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(px(15)),
                ..default()
            },
            BackgroundColor(CARD),
            Outline::new(px(1), Val::ZERO, CARD_OUTLINE),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(label),
                TextFont { font_size: FontSize::Px(15.0), ..default() },
                TextColor(BODY),
            ));
            spawn_small_button(
                row,
                value,
                action,
                Color::srgb(0.095, 0.12, 0.21),
            );
        });
}

fn spawn_day_picker(parent: &mut ChildSpawnerCommands, alarm: &AlarmConfig) {
    const DAYS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];

    parent
        .spawn(Node {
            width: percent(100),
            column_gap: px(6),
            justify_content: JustifyContent::SpaceBetween,
            ..default()
        })
        .with_children(|days| {
            for (bit, label) in DAYS.into_iter().enumerate() {
                let enabled = alarm.day_enabled(bit as u8);
                days.spawn((
                    Button,
                    AlarmSettingsAction::DraftToggleDay(bit as u8),
                    Node {
                        width: px(58),
                        height: px(42),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(px(13)),
                        ..default()
                    },
                    BackgroundColor(if enabled {
                        BLUE
                    } else {
                        Color::srgb(0.075, 0.09, 0.15)
                    }),
                    Outline::new(
                        px(1),
                        Val::ZERO,
                        if enabled {
                            Color::srgba(0.42, 0.60, 1.0, 0.72)
                        } else {
                            CARD_OUTLINE
                        },
                    ),
                ))
                .with_children(|button| {
                    button.spawn((
                        Text::new(label),
                        TextFont {
                            font_size: FontSize::Px(15.0),
                            ..default()
                        },
                        TextColor(if enabled { PRIMARY } else { MUTED }),
                    ));
                });
            }
        });
}

fn spawn_action_button(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: AlarmSettingsAction,
    color: Color,
) {
    parent
        .spawn((
            Button,
            action,
            Node {
                width: percent(100),
                height: px(52),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(px(16)),
                ..default()
            },
            BackgroundColor(color),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(label),
                TextFont { font_size: FontSize::Px(16.0), ..default() },
                TextColor(PRIMARY),
            ));
        });
}

fn spawn_small_button(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: AlarmSettingsAction,
    color: Color,
) {
    parent
        .spawn((
            Button,
            action,
            Node {
                min_width: px(42),
                min_height: px(36),
                padding: UiRect::axes(px(12), px(7)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(px(12)),
                ..default()
            },
            BackgroundColor(color),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(label),
                TextFont { font_size: FontSize::Px(13.0), ..default() },
                TextColor(PRIMARY),
            ));
        });
}

fn handle_alarm_settings_actions(
    mut interactions: Query<
        (&Interaction, &AlarmSettingsAction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>),
    >,
    mut settings: ResMut<NeraSettings>,
    mut gate: ResMut<AlarmSettingsInputGate>,
    mut ui: ResMut<AlarmSettingsUiState>,
    mut commands: Commands,
) {
    for (interaction, action, mut background) in &mut interactions {
        *background = BackgroundColor(action_color(
            *action,
            *interaction,
            ui.draft.as_ref(),
        ));

        if *interaction != Interaction::Pressed {
            continue;
        }

        match *action {
            AlarmSettingsAction::Close => {
                gate.close_requested = true;
            }
            AlarmSettingsAction::Add => {
                let alarms = settings.alarm_configs();
                if alarms.len() >= MAX_ALARMS {
                    continue;
                }
                ui.draft = Some(AlarmConfig::default_new(settings.next_alarm_id()));
                ui.view = AlarmSettingsView::Edit;
                ui.rebuild = true;
            }
            AlarmSettingsAction::Edit(id) => {
                ui.draft = settings
                    .alarm_configs()
                    .into_iter()
                    .find(|alarm| alarm.id == id);
                if ui.draft.is_some() {
                    ui.view = AlarmSettingsView::Edit;
                    ui.rebuild = true;
                }
            }
            AlarmSettingsAction::Toggle(id) => {
                let mut alarms = settings.alarm_configs();
                if let Some(alarm) = alarms.iter_mut().find(|alarm| alarm.id == id) {
                    alarm.enabled = !alarm.enabled;
                    settings.store_alarm_configs(&alarms);
                    save_settings_deferred(&mut commands);
                    ui.rebuild = true;
                }
            }
            AlarmSettingsAction::Delete(id) => {
                let mut alarms = settings.alarm_configs();
                let before = alarms.len();
                alarms.retain(|alarm| alarm.id != id);
                if alarms.len() != before {
                    settings.store_alarm_configs(&alarms);
                    save_settings_deferred(&mut commands);
                    ui.rebuild = true;
                }
            }
            AlarmSettingsAction::Back => {
                match ui.view {
                    AlarmSettingsView::TimePicker | AlarmSettingsView::DatePicker => {
                        // Picker changes are kept in the in-memory draft. Return
                        // to the editor without committing to persistent settings.
                        ui.view = AlarmSettingsView::Edit;
                    }
                    AlarmSettingsView::Edit => {
                        ui.view = AlarmSettingsView::List;
                        ui.draft = None;
                    }
                    AlarmSettingsView::List => {}
                }
                ui.rebuild = true;
            }
            AlarmSettingsAction::Save => {
                let Some(draft) = ui.draft.take() else {
                    continue;
                };
                let draft = draft.normalized();
                let mut alarms = settings.alarm_configs();
                if let Some(existing) = alarms.iter_mut().find(|alarm| alarm.id == draft.id) {
                    *existing = draft;
                } else if alarms.len() < MAX_ALARMS {
                    alarms.push(draft);
                }
                settings.store_alarm_configs(&alarms);
                save_settings_deferred(&mut commands);
                ui.view = AlarmSettingsView::List;
                ui.rebuild = true;
            }
            AlarmSettingsAction::OpenTimePicker => {
                if ui.draft.is_some() {
                    ui.view = AlarmSettingsView::TimePicker;
                    ui.rebuild = true;
                }
            }
            AlarmSettingsAction::OpenDatePicker => {
                if ui.draft.is_some() {
                    ui.view = AlarmSettingsView::DatePicker;
                    ui.rebuild = true;
                }
            }
            action => {
                let Some(draft) = ui.draft.as_mut() else {
                    continue;
                };
                mutate_draft(draft, action);
                ui.rebuild = true;
            }
        }
    }
}

fn mutate_draft(draft: &mut AlarmConfig, action: AlarmSettingsAction) {
    match action {
        AlarmSettingsAction::DraftToggleEnabled => draft.enabled = !draft.enabled,
        AlarmSettingsAction::DraftCycleLabel => draft.cycle_label(),
        AlarmSettingsAction::DraftCycleMode => {
            draft.cycle_mode();
            // Weekly alarms should be immediately useful. If an older alarm
            // enters weekly mode with no selected days, restore the intuitive
            // default instead of showing seven OFF buttons.
            if draft.mode == 1 && draft.repeat_days == 0 {
                draft.repeat_days = 0b0111_1111;
            }
        }
        AlarmSettingsAction::DraftHourDown => draft.decrement_hour(),
        AlarmSettingsAction::DraftHourUp => draft.cycle_hour(),
        AlarmSettingsAction::DraftMinuteDown => draft.decrement_minute(),
        AlarmSettingsAction::DraftMinuteUp => draft.cycle_minute(),
        AlarmSettingsAction::DraftToggleMeridiem => {
            draft.hour = if draft.hour < 12 {
                draft.hour.saturating_add(12)
            } else {
                draft.hour.saturating_sub(12)
            };
        }
        AlarmSettingsAction::DraftYearDown => draft.decrement_year(),
        AlarmSettingsAction::DraftYearUp => draft.cycle_year(),
        AlarmSettingsAction::DraftMonthDown => draft.decrement_month(),
        AlarmSettingsAction::DraftMonthUp => draft.cycle_month(),
        AlarmSettingsAction::DraftDayDown => draft.decrement_day(),
        AlarmSettingsAction::DraftDayUp => draft.cycle_day(),
        AlarmSettingsAction::DraftToggleDay(bit) => {
            draft.toggle_day(bit);
            // A weekly alarm with zero active days cannot fire. Reflect that
            // truth in the master state instead of leaving a misleading ON.
            if draft.repeat_days == 0 {
                draft.enabled = false;
            } else {
                // Selecting any weekday means the user is actively scheduling
                // this alarm, so turn it back on as part of the same gesture.
                draft.enabled = true;
            }
        }
        _ => {}
    }
}

fn action_color(
    action: AlarmSettingsAction,
    interaction: Interaction,
    draft: Option<&AlarmConfig>,
) -> Color {
    let normal = match action {
        AlarmSettingsAction::Save | AlarmSettingsAction::Add => BLUE,
        AlarmSettingsAction::Delete(_) => Color::srgb(0.28, 0.08, 0.11),
        AlarmSettingsAction::DraftToggleDay(bit) => {
            if draft.is_some_and(|alarm| alarm.day_enabled(bit)) {
                BLUE
            } else {
                Color::srgb(0.075, 0.09, 0.15)
            }
        }
        _ => Color::srgb(0.095, 0.12, 0.21),
    };

    match interaction {
        Interaction::Pressed => match action {
            AlarmSettingsAction::Save | AlarmSettingsAction::Add => BLUE_PRESSED,
            AlarmSettingsAction::Delete(_) => Color::srgb(0.38, 0.10, 0.14),
            AlarmSettingsAction::DraftToggleDay(bit)
                if draft.is_some_and(|alarm| alarm.day_enabled(bit)) =>
            {
                BLUE_PRESSED
            }
            _ => Color::srgb(0.18, 0.22, 0.36),
        },
        Interaction::Hovered => match action {
            AlarmSettingsAction::Save | AlarmSettingsAction::Add => BLUE_HOVER,
            AlarmSettingsAction::Delete(_) => Color::srgb(0.34, 0.09, 0.13),
            AlarmSettingsAction::DraftToggleDay(bit)
                if draft.is_some_and(|alarm| alarm.day_enabled(bit)) =>
            {
                BLUE_HOVER
            }
            _ => Color::srgb(0.135, 0.17, 0.29),
        },
        Interaction::None => normal,
    }
}

fn drag_alarm_settings_window(
    mouse: Res<ButtonInput<MouseButton>>,
    drag_regions: Query<
        &Interaction,
        (Changed<Interaction>, With<AlarmSettingsDragRegion>),
    >,
    mut windows: Query<&mut Window, With<AlarmSettingsWindow>>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    if !drag_regions
        .iter()
        .any(|interaction| *interaction == Interaction::Pressed)
    {
        return;
    }

    if let Ok(mut window) = windows.single_mut() {
        window.start_drag_move();
    }
}

fn scroll_alarm_settings_window(
    mut wheel: MessageReader<MouseWheel>,
    windows: Query<Entity, With<AlarmSettingsWindow>>,
    mut scroll_areas: Query<(&mut ScrollPosition, &ComputedNode), With<AlarmSettingsScrollArea>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok((mut position, computed)) = scroll_areas.single_mut() else {
        return;
    };

    let max_y = ((computed.content_size().y - computed.size().y)
        * computed.inverse_scale_factor())
        .max(0.0);

    for event in wheel.read() {
        if event.window != window {
            continue;
        }
        let delta = match event.unit {
            MouseScrollUnit::Line => event.y * 42.0,
            MouseScrollUnit::Pixel => event.y,
        };
        position.0.y = (position.0.y - delta).clamp(0.0, max_y);
    }
}

fn commit_alarm_settings_close_after_release(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<Entity, With<AlarmSettingsWindow>>,
    mut gate: ResMut<AlarmSettingsInputGate>,
    mut ui: ResMut<AlarmSettingsUiState>,
    mut commands: Commands,
) {
    if !gate.close_requested {
        return;
    }

    if mouse.pressed(MouseButton::Left)
        || mouse.pressed(MouseButton::Right)
        || mouse.pressed(MouseButton::Middle)
    {
        return;
    }

    for window in &windows {
        commands.entity(window).despawn();
    }

    gate.window_open = false;
    gate.minimized = false;
    gate.close_requested = false;
    ui.view = AlarmSettingsView::List;
    ui.draft = None;
    ui.rebuild = false;
    info!("Nera Alarm Settings -> closed");
}

fn cleanup_alarm_settings_owned(
    windows: Query<(), With<AlarmSettingsWindow>>,
    owned: Query<(Entity, &AlarmSettingsOwned)>,
    mut commands: Commands,
) {
    for (entity, owner) in &owned {
        if windows.get(owner.window).is_err() {
            commands.entity(entity).despawn();
        }
    }
}

fn observe_alarm_settings_window_removed(
    mut removed: RemovedComponents<AlarmSettingsWindow>,
    mut gate: ResMut<AlarmSettingsInputGate>,
    mut ui: ResMut<AlarmSettingsUiState>,
) {
    if removed.read().next().is_none() {
        return;
    }

    gate.window_open = false;
    gate.minimized = false;
    gate.close_requested = false;
    ui.view = AlarmSettingsView::List;
    ui.draft = None;
    ui.rebuild = false;
}

fn format_alarm_time(alarm: &AlarmConfig) -> String {
    let hour12 = match alarm.hour.min(23) % 12 {
        0 => 12,
        value => value,
    };
    format!(
        "{:02}:{:02} {}",
        hour12,
        alarm.minute.min(59),
        if alarm.hour < 12 { "AM" } else { "PM" },
    )
}

fn schedule_summary(alarm: &AlarmConfig) -> String {
    if alarm.mode == 0 {
        return format!("One-time · {}", alarm.date_label());
    }

    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let selected: Vec<_> = NAMES
        .into_iter()
        .enumerate()
        .filter_map(|(bit, name)| alarm.day_enabled(bit as u8).then_some(name))
        .collect();

    match selected.len() {
        0 => "Repeat weekly · No days selected".to_string(),
        7 => "Repeat weekly · Every day".to_string(),
        _ => format!("Repeat weekly · {}", selected.join("  ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reopen_restores_existing_alarm_window_without_discarding_draft() {
        let mut app = App::new();
        app.insert_resource(AlarmSettingsInputGate {
            window_open: true,
            minimized: true,
            ..default()
        })
        .insert_resource(AlarmSettingsUiState {
            view: AlarmSettingsView::Edit,
            draft: Some(AlarmConfig::default_new(7)),
            rebuild: false,
        })
        .add_message::<OpenAlarmSettingsWindow>()
        .add_systems(Update, open_alarm_settings_window);
        let window = app.world_mut().spawn((
            AlarmSettingsWindow,
            Window { focused: false, ..default() },
        )).id();
        app.world_mut().write_message(OpenAlarmSettingsWindow);
        app.update();

        let mut window = app.world_mut().get_mut::<Window>(window).unwrap();
        assert_eq!(window.internal.take_minimize_request(), Some(false));
        assert!(window.focused);
        assert!(!app.world().resource::<AlarmSettingsInputGate>().minimized);
        let state = app.world().resource::<AlarmSettingsUiState>();
        assert_eq!(state.view, AlarmSettingsView::Edit);
        assert_eq!(state.draft.as_ref().unwrap().id, 7);
    }

    #[test]
    fn time_preview_handles_midnight_and_pm() {
        let mut alarm = AlarmConfig::default_new(1);
        alarm.hour = 0;
        alarm.minute = 5;
        assert_eq!(format_alarm_time(&alarm), "12:05 AM");

        alarm.hour = 19;
        alarm.minute = 30;
        assert_eq!(format_alarm_time(&alarm), "07:30 PM");
    }

    #[test]
    fn weekly_summary_tracks_selected_days() {
        let mut alarm = AlarmConfig::default_new(1);
        alarm.mode = 1;
        alarm.repeat_days = (1 << 0) | (1 << 4);
        assert_eq!(schedule_summary(&alarm), "Repeat weekly · Mon  Fri");
    }

    #[test]
    fn new_weekly_alarm_starts_with_every_day_on() {
        let alarm = AlarmConfig::default_new(1);
        assert_eq!(alarm.repeat_days, 0b0111_1111);
        assert!((0..7).all(|bit| alarm.day_enabled(bit)));
    }

    #[test]
    fn disabling_every_weekday_turns_alarm_off_and_selecting_one_turns_it_back_on() {
        let mut alarm = AlarmConfig::default_new(1);
        for bit in 0..7 {
            mutate_draft(&mut alarm, AlarmSettingsAction::DraftToggleDay(bit));
        }
        assert_eq!(alarm.repeat_days, 0);
        assert!(!alarm.enabled);

        mutate_draft(&mut alarm, AlarmSettingsAction::DraftToggleDay(0));
        assert!(alarm.day_enabled(0));
        assert!(alarm.enabled);
    }
}
