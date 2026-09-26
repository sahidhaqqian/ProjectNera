//! Settings utility-window lifecycle, construction, controls, and scrolling.
use bevy::{
    app::AppExit,
    camera::RenderTarget,
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
    ui::UiTargetCamera,
    window::{PrimaryWindow, WindowLevel, WindowPosition, WindowRef, WindowResizeConstraints},
};

use super::runtime::{
    PositionRuntime, StartupRuntime, save_settings_before_exit, save_settings_deferred,
};
use super::{NeraSettings, startup};

#[derive(Message, Default)]
pub(crate) struct OpenSettingsWindow;

/// Emitted only after the Settings mouse gesture has fully released and the
/// utility window is being closed. ActionMenuPlugin uses this to reopen the
/// root radial menu at the same center where Settings was entered.
#[derive(Message, Default)]
pub(crate) struct ReturnToActionMenu;

/// Owns input routing while the secondary Settings window exists.
///
/// Bevy's mouse ButtonInput is application-global: clicks on the Settings
/// window are visible to systems that also drive Nera's PrimaryWindow.  This
/// gate prevents those clicks from becoming Nera body hits / drags.
///
/// Closing is requested on button press but the Settings HWND stays alive
/// until that mouse gesture releases, so the release cannot fall through to Nera.
#[derive(Resource, Default)]
pub(crate) struct SettingsInputGate {
    window_open: bool,
    close_requested: bool,
    return_to_menu_requested: bool,
}

pub(crate) fn nera_input_available(gate: Res<SettingsInputGate>) -> bool {
    !gate.window_open
}

pub(crate) fn nera_input_blocked(gate: Res<SettingsInputGate>) -> bool {
    gate.window_open
}
// =============================================================================
// Settings window
// =============================================================================

#[derive(Component)]
pub(super) struct SettingsWindow;

#[derive(Component)]
pub(super) struct SettingsOwned {
    window: Entity,
}

#[derive(Component)]
pub(super) struct AlwaysOnTopValue;

#[derive(Component)]
pub(super) struct StartWithWindowsValue;

#[derive(Component)]
pub(super) struct StartHiddenValue;

#[derive(Component)]
pub(super) struct RememberPositionValue;

#[derive(Component)]
pub(super) struct CharacterSizeValue;

#[derive(Component)]
pub(super) struct GroundOffsetValue;

#[derive(Component)]
pub(super) struct WalkingEnabledValue;

#[derive(Component)]
pub(super) struct WalkingCheckmark;

#[derive(Component)]
pub(super) struct FollowMouseValue;

#[derive(Component)]
pub(super) struct FollowEyesValue;

#[derive(Component)]
pub(super) struct FollowHeadValue;

#[derive(Component)]
pub(super) struct FollowIntensityValue;

#[derive(Component)]
pub(super) struct ShadowEnabledValue;

#[derive(Component)]
pub(super) struct ShadowTypeValue;

#[derive(Component)]
pub(super) struct ShadowOpacityValue;

#[derive(Component)]
pub(super) struct ShadowSoftnessValue;

#[derive(Component)]
pub(super) struct BrightnessValue;

#[derive(Component)]
pub(super) struct MasterVolumeValue;

#[derive(Component)]
pub(super) struct AudioMutedValue;

#[derive(Component)]
pub(super) struct FpsValue;

#[derive(Component)]
pub(super) struct SettingsDragRegion;

/// Scrollable body of the Settings utility window. The title bar and footer
/// intentionally stay outside this node so they remain fixed while the cards
/// move underneath the viewport.
#[derive(Component)]
pub(super) struct SettingsScrollArea;

#[derive(Component, Clone, Copy)]
pub(super) enum SettingsButtonAction {
    ToggleAlwaysOnTop,
    ToggleStartWithWindows,
    ToggleStartHidden,
    ToggleRememberPosition,
    CycleCharacterSize,
    CycleGroundOffset,
    ToggleWalking,
    ToggleFollowMouse,
    ToggleFollowEyes,
    ToggleFollowHead,
    CycleFollowIntensity,
    ToggleShadow,
    CycleShadowType,
    CycleShadowOpacity,
    CycleShadowSoftness,
    CycleBrightness,
    CycleMasterVolume,
    ToggleAudioMute,
    CycleFps,
    BackToMenu,
    ResetDefaults,
    CloseWindow,
    Exit,
}

pub(super) fn open_settings_window(
    mut open: MessageReader<OpenSettingsWindow>,
    existing: Query<Entity, With<SettingsWindow>>,
    settings: Res<NeraSettings>,
    startup_state: Res<StartupRuntime>,
    mut input_gate: ResMut<SettingsInputGate>,
    mut commands: Commands,
) {
    if open.read().next().is_none() {
        return;
    }

    // One settings window only. If it is already open, do not create a second
    // copy with a separate camera/UI tree.
    if !existing.is_empty() {
        input_gate.window_open = true;
        return;
    }

    // While Settings exists, it exclusively owns application mouse input.
    // Closing is deferred until the close-button mouse gesture has released,
    // so winit never loses the corresponding mouse-up event.
    input_gate.window_open = true;
    input_gate.close_requested = false;
    input_gate.return_to_menu_requested = false;

    let window_entity = commands
        .spawn((
            Name::new("Nera Settings Window"),
            SettingsWindow,
            Window {
                title: "Nera Settings".into(),
                resolution: (580, 820).into(),
                resizable: true,
                resize_constraints: WindowResizeConstraints {
                    min_width: 460.0,
                    min_height: 520.0,
                    max_width: 960.0,
                    max_height: 1200.0,
                },
                // The OS chrome is intentionally removed. The visible rounded
                // shell is Bevy UI drawn on a transparent native window.
                decorations: false,
                transparent: true,
                // Settings must stay in front of the Always-On-Top pet.
                // Nera itself becomes click-through while this window is open.
                window_level: WindowLevel::AlwaysOnTop,
                position: WindowPosition::Automatic,
                skip_taskbar: false,
                ..default()
            },
        ))
        .id();

    let camera_entity = commands
        .spawn((
            Name::new("Nera Settings Camera"),
            SettingsOwned {
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

    spawn_settings_ui(
        &mut commands,
        window_entity,
        camera_entity,
        &settings,
        &startup_state,
    );

    info!("Nera Settings -> opened (custom shell)");
}

fn spawn_settings_ui(
    commands: &mut Commands,
    window: Entity,
    camera: Entity,
    settings: &NeraSettings,
    startup_state: &StartupRuntime,
) {
    commands
        .spawn((
            Name::new("Nera Settings Root"),
            SettingsOwned { window },
            UiTargetCamera(camera),
            Node {
                width: percent(100),
                height: percent(100),
                padding: UiRect::all(px(14)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            // Keep the native window itself fully transparent. Only the custom
            // rounded shell below is visible.
            BackgroundColor(Color::NONE),
        ))
        .with_children(|root| {
            root.spawn((
                Name::new("Nera Settings Shell"),
                Node {
                    width: percent(100),
                    height: percent(100),
                    padding: UiRect::all(px(18)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(9),
                    border_radius: BorderRadius::all(px(28)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.032, 0.042, 0.078, 0.985)),
                Outline::new(
                    px(1),
                    Val::ZERO,
                    Color::srgba(0.18, 0.23, 0.38, 0.85),
                ),
                BoxShadow::new(
                    Color::srgba(0.0, 0.0, 0.0, 0.55),
                    px(0),
                    px(8),
                    px(0),
                    px(22),
                ),
            ))
            .with_children(|panel| {
                spawn_settings_header(panel);

                // The settings list is now a real clipped/scrollable viewport.
                // Keep the draggable header and small footer fixed outside it.
                panel
                    .spawn((
                        Name::new("Nera Settings Scroll Area"),
                        SettingsScrollArea,
                        ScrollPosition::default(),
                        Node {
                            width: percent(100),
                            flex_grow: 1.0,
                            min_height: px(0),
                            flex_direction: FlexDirection::Column,
                            row_gap: px(9),
                            overflow: Overflow::scroll_y(),
                            ..default()
                        },
                    ))
                    .with_children(|scroll| {

                        settings_card(scroll, "GENERAL", |card| {
                            settings_row(
                                card,
                                "Always on top",
                                if settings.always_on_top { "ON" } else { "OFF" }.to_string(),
                                AlwaysOnTopValue,
                                SettingsButtonAction::ToggleAlwaysOnTop,
                            );
                            settings_row(
                                card,
                                "Start with Windows",
                                if startup_state.enabled { "ON" } else { "OFF" }.to_string(),
                                StartWithWindowsValue,
                                SettingsButtonAction::ToggleStartWithWindows,
                            );
                            settings_row(
                                card,
                                "Start hidden",
                                if settings.start_hidden { "ON" } else { "OFF" }.to_string(),
                                StartHiddenValue,
                                SettingsButtonAction::ToggleStartHidden,
                            );
                            settings_row(
                                card,
                                "Remember position",
                                if settings.remember_position { "ON" } else { "OFF" }.to_string(),
                                RememberPositionValue,
                                SettingsButtonAction::ToggleRememberPosition,
                            );
                        });

                        settings_card(scroll, "CHARACTER", |card| {
                            settings_row(
                                card,
                                "Character size",
                                settings.character_size_label(),
                                CharacterSizeValue,
                                SettingsButtonAction::CycleCharacterSize,
                            );
                            settings_row(
                                card,
                                "Ground offset",
                                settings.ground_offset_label(),
                                GroundOffsetValue,
                                SettingsButtonAction::CycleGroundOffset,
                            );

                            card.spawn((
                                Text::new("Size stays proportional · Ground offset changes standing contact only"),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                                TextColor(Color::srgb(0.45, 0.5, 0.64)),
                            ));
                        });

                        settings_card(scroll, "BEHAVIOR", |card| {
                            walking_checkbox(card, settings.walking_enabled);
                            settings_row(
                                card,
                                "Follow mouse",
                                if settings.follow_mouse { "ON" } else { "OFF" }.to_string(),
                                FollowMouseValue,
                                SettingsButtonAction::ToggleFollowMouse,
                            );
                            settings_row(
                                card,
                                "Eyes",
                                if settings.follow_eyes { "ON" } else { "OFF" }.to_string(),
                                FollowEyesValue,
                                SettingsButtonAction::ToggleFollowEyes,
                            );
                            settings_row(
                                card,
                                "Head / upper body",
                                if settings.follow_head { "ON" } else { "OFF" }.to_string(),
                                FollowHeadValue,
                                SettingsButtonAction::ToggleFollowHead,
                            );
                            settings_row(
                                card,
                                "Follow intensity",
                                settings.follow_intensity_label(),
                                FollowIntensityValue,
                                SettingsButtonAction::CycleFollowIntensity,
                            );
                        });

                        settings_card(scroll, "GRAPHICS", |card| {
                            settings_row(
                                card,
                                "Shadow",
                                if settings.shadow_enabled { "ON" } else { "OFF" }.to_string(),
                                ShadowEnabledValue,
                                SettingsButtonAction::ToggleShadow,
                            );
                            settings_row(
                                card,
                                "Shadow type",
                                settings.shadow_type_label().to_string(),
                                ShadowTypeValue,
                                SettingsButtonAction::CycleShadowType,
                            );
                            settings_row(
                                card,
                                "Shadow opacity",
                                settings.shadow_opacity_label(),
                                ShadowOpacityValue,
                                SettingsButtonAction::CycleShadowOpacity,
                            );
                            settings_row(
                                card,
                                "Simple softness",
                                settings.shadow_softness_label(),
                                ShadowSoftnessValue,
                                SettingsButtonAction::CycleShadowSoftness,
                            );
                            settings_row(
                                card,
                                "Brightness",
                                settings.brightness_label(),
                                BrightnessValue,
                                SettingsButtonAction::CycleBrightness,
                            );

                            card.spawn((
                                Text::new("Simple = cheapest blob · Character = real animated VRM shadow"),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                                TextColor(Color::srgb(0.45, 0.5, 0.64)),
                            ));
                        });

                        settings_card(scroll, "AUDIO", |card| {
                            settings_row(
                                card,
                                "Master volume",
                                settings.master_volume_label(),
                                MasterVolumeValue,
                                SettingsButtonAction::CycleMasterVolume,
                            );
                            settings_row(
                                card,
                                "Mute audio",
                                if settings.audio_muted { "ON" } else { "OFF" }.to_string(),
                                AudioMutedValue,
                                SettingsButtonAction::ToggleAudioMute,
                            );

                            card.spawn((
                                Text::new("Applies globally to current and future Nera sounds"),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                                TextColor(Color::srgb(0.45, 0.5, 0.64)),
                            ));
                        });

                        settings_card(scroll, "PERFORMANCE", |card| {
                            settings_row(
                                card,
                                "FPS limit",
                                settings.fps_label(),
                                FpsValue,
                                SettingsButtonAction::CycleFps,
                            );

                            card.spawn((
                                Text::new("30  ·  60  ·  90  ·  120  ·  144  ·  Unlimited (VSync)"),
                                TextFont {
                                    font_size: FontSize::Px(12.0),
                                    ..default()
                                },
                                TextColor(Color::srgb(0.45, 0.5, 0.64)),
                            ));
                        });

                        settings_card(scroll, "ABOUT", |card| {
                            info_row(card, "App", "Project Nera".to_string());
                            info_row(
                                card,
                                "Version",
                                format!("v{}", env!("CARGO_PKG_VERSION")),
                            );
                            info_row(card, "Engine", "Rust · Bevy 0.19 · VRM 1.0".to_string());
                            info_row(
                                card,
                                "Build",
                                if cfg!(debug_assertions) {
                                    "Debug".to_string()
                                } else {
                                    "Release".to_string()
                                },
                            );
                        });

                        settings_card(scroll, "APP", |card| {
                            action_button(
                                card,
                                "Reset to defaults",
                                SettingsButtonAction::ResetDefaults,
                            );

                            card.spawn((
                                Text::new("Reset restores app settings, startup state, audio, size, and saved position"),
                                TextFont {
                                    font_size: FontSize::Px(11.0),
                                    ..default()
                                },
                                TextColor(Color::srgb(0.45, 0.5, 0.64)),
                            ));

                            action_button(
                                card,
                                "Exit Nera",
                                SettingsButtonAction::Exit,
                            );
                        });

                    });

                panel.spawn((
                    Text::new(format!(
                        "Project Nera v{}  ·  Drag window edges to resize  ·  Wheel / touchpad to scroll",
                        env!("CARGO_PKG_VERSION"),
                    )),
                    TextFont {
                        font_size: FontSize::Px(11.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.32, 0.37, 0.5)),
                ));
            });
        });
}

fn spawn_settings_header(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn(Node {
            width: percent(100),
            height: px(58),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(10),
            ..default()
        })
        .with_children(|header| {
            header
                .spawn((
                    Button,
                    SettingsButtonAction::BackToMenu,
                    Node {
                        width: px(68),
                        height: px(42),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(px(13)),
                        ..default()
                    },
                    BackgroundColor(button_color(
                        SettingsButtonAction::BackToMenu,
                        Interaction::None,
                    )),
                    Outline::new(px(1), Val::ZERO, Color::srgba(0.24, 0.29, 0.43, 0.9)),
                ))
                .with_children(|back| {
                    back.spawn((
                        Text::new("Back"),
                        TextFont {
                            font_size: FontSize::Px(14.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.73, 0.77, 0.89)),
                    ));
                });

            // Only the title strip starts a native drag move. Keeping both
            // navigation buttons outside it prevents click-vs-drag ambiguity.
            header
                .spawn((
                    Button,
                    SettingsDragRegion,
                    Node {
                        flex_grow: 1.0,
                        height: percent(100),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::Center,
                        row_gap: px(5),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                ))
                .with_children(|title| {
                    title.spawn((
                        Text::new("NERA SETTINGS"),
                        TextFont {
                            font_size: FontSize::Px(28.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.89, 0.91, 1.0)),
                    ));

                    title.spawn((
                        Text::new("Desktop pet controls"),
                        TextFont {
                            font_size: FontSize::Px(13.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.45, 0.51, 0.68)),
                    ));
                });

            header
                .spawn((
                    Button,
                    SettingsButtonAction::CloseWindow,
                    Node {
                        width: px(42),
                        height: px(42),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border_radius: BorderRadius::all(px(13)),
                        ..default()
                    },
                    BackgroundColor(button_color(
                        SettingsButtonAction::CloseWindow,
                        Interaction::None,
                    )),
                    Outline::new(px(1), Val::ZERO, Color::srgba(0.24, 0.29, 0.43, 0.9)),
                ))
                .with_children(|close| {
                    close.spawn((
                        Text::new("X"),
                        TextFont {
                            font_size: FontSize::Px(24.0),
                            ..default()
                        },
                        TextColor(Color::srgb(0.73, 0.77, 0.89)),
                    ));
                });
        });
}

fn settings_card(
    parent: &mut ChildSpawnerCommands,
    title: &'static str,
    content: impl FnOnce(&mut ChildSpawnerCommands),
) {
    parent
        .spawn((
            Node {
                width: percent(100),
                flex_shrink: 0.0,
                padding: UiRect::all(px(12)),
                flex_direction: FlexDirection::Column,
                row_gap: px(5),
                border_radius: BorderRadius::all(px(18)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.055, 0.069, 0.118, 0.97)),
            Outline::new(px(1), Val::ZERO, Color::srgba(0.12, 0.16, 0.28, 0.9)),
        ))
        .with_children(|card| {
            card.spawn((
                Text::new(title),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::srgb(0.77, 0.8, 0.95)),
            ));

            card.spawn(Node {
                height: px(2),
                ..default()
            });

            content(card);
        });
}

fn settings_row<M: Component>(
    parent: &mut ChildSpawnerCommands,
    label: &'static str,
    initial_value: String,
    marker: M,
    action: SettingsButtonAction,
) {
    parent
        .spawn(Node {
            width: percent(100),
            height: px(42),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            column_gap: px(14),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::srgb(0.82, 0.85, 0.94)),
            ));

            row.spawn((
                Button,
                action,
                Node {
                    min_width: px(178),
                    height: px(40),
                    padding: UiRect::horizontal(px(16)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(12)),
                    ..default()
                },
                BackgroundColor(button_color(action, Interaction::None)),
                Outline::new(px(1), Val::ZERO, Color::srgba(0.18, 0.23, 0.38, 0.9)),
            ))
            .with_children(|button| {
                button.spawn((
                    Text::new(initial_value),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.88, 0.9, 0.98)),
                    marker,
                ));
            });
        });
}

fn walking_checkbox(parent: &mut ChildSpawnerCommands, enabled: bool) {
    let action = SettingsButtonAction::ToggleWalking;
    parent
        .spawn(Node {
            width: percent(100),
            height: px(42),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            column_gap: px(14),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new("Aktifkan jalan"),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::srgb(0.82, 0.85, 0.94)),
            ));
            row.spawn((
                Button,
                action,
                Name::new("Enable walking checkbox"),
                Node {
                    min_width: px(178),
                    height: px(40),
                    padding: UiRect::horizontal(px(16)),
                    column_gap: px(10),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(12)),
                    ..default()
                },
                BackgroundColor(button_color(action, Interaction::None)),
                Outline::new(px(1), Val::ZERO, Color::srgba(0.18, 0.23, 0.38, 0.9)),
            ))
            .with_children(|button| {
                button
                    .spawn((
                        Node {
                            width: px(22),
                            height: px(22),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(px(4)),
                            ..default()
                        },
                        Outline::new(px(1), Val::ZERO, Color::srgb(0.64, 0.73, 0.95)),
                    ))
                    .with_children(|checkbox| {
                        checkbox
                            .spawn((
                                WalkingCheckmark,
                                Node {
                                    width: px(18),
                                    height: px(18),
                                    display: if enabled { Display::Flex } else { Display::None },
                                    ..default()
                                },
                            ))
                            .with_children(|tick| {
                                // Draw the tick directly: the bundled font is a
                                // subset and need not contain a checkmark glyph.
                                for (left, top, width, angle) in [
                                    (2.0, 9.5, 6.0, 45.0),
                                    (5.5, 7.5, 11.0, -45.0),
                                ] {
                                    tick.spawn((
                                        Node {
                                            position_type: PositionType::Absolute,
                                            left: px(left),
                                            top: px(top),
                                            width: px(width),
                                            height: px(3),
                                            border_radius: BorderRadius::all(px(1)),
                                            ..default()
                                        },
                                        UiTransform::from_rotation(Rot2::degrees(angle)),
                                        BackgroundColor(Color::srgb(0.61, 0.88, 0.74)),
                                    ));
                                }
                            });
                    });
                button.spawn((
                    WalkingEnabledValue,
                    Text::new(if enabled { "Yes" } else { "No" }),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.88, 0.9, 0.98)),
                ));
            });
        });
}

fn info_row(parent: &mut ChildSpawnerCommands, label: &'static str, value: String) {
    parent
        .spawn(Node {
            width: percent(100),
            min_height: px(32),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            column_gap: px(14),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgb(0.62, 0.67, 0.8)),
            ));

            row.spawn((
                Text::new(value),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgb(0.88, 0.9, 0.98)),
            ));
        });
}

fn action_button(
    parent: &mut ChildSpawnerCommands,
    label: &'static str,
    action: SettingsButtonAction,
) {
    parent
        .spawn((
            Button,
            action,
            Node {
                width: percent(100),
                height: px(46),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(px(13)),
                ..default()
            },
            BackgroundColor(button_color(action, Interaction::None)),
            Outline::new(
                px(1),
                Val::ZERO,
                if matches!(action, SettingsButtonAction::Exit) {
                    Color::srgba(0.58, 0.18, 0.25, 0.75)
                } else {
                    Color::srgba(0.18, 0.23, 0.38, 0.9)
                },
            ),
        ))
        .with_children(|button| {
            button.spawn((
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(if matches!(action, SettingsButtonAction::Exit) {
                    Color::srgb(1.0, 0.84, 0.87)
                } else {
                    Color::srgb(0.88, 0.9, 0.98)
                }),
            ));
        });
}

fn button_color(action: SettingsButtonAction, interaction: Interaction) -> Color {
    match action {
        SettingsButtonAction::Exit => match interaction {
            Interaction::Pressed => Color::srgb(0.44, 0.10, 0.16),
            Interaction::Hovered => Color::srgb(0.35, 0.095, 0.14),
            Interaction::None => Color::srgb(0.24, 0.075, 0.11),
        },
        SettingsButtonAction::CloseWindow | SettingsButtonAction::BackToMenu => match interaction {
            Interaction::Pressed => Color::srgb(0.20, 0.23, 0.34),
            Interaction::Hovered => Color::srgb(0.16, 0.19, 0.30),
            Interaction::None => Color::srgb(0.095, 0.115, 0.19),
        },
        SettingsButtonAction::ToggleAlwaysOnTop
        | SettingsButtonAction::ToggleStartWithWindows
        | SettingsButtonAction::ToggleStartHidden
        | SettingsButtonAction::ToggleRememberPosition
        | SettingsButtonAction::CycleCharacterSize
        | SettingsButtonAction::CycleGroundOffset
        | SettingsButtonAction::ToggleWalking
        | SettingsButtonAction::ToggleFollowMouse
        | SettingsButtonAction::ToggleFollowEyes
        | SettingsButtonAction::ToggleFollowHead
        | SettingsButtonAction::CycleFollowIntensity
        | SettingsButtonAction::ToggleShadow
        | SettingsButtonAction::CycleShadowType
        | SettingsButtonAction::CycleShadowOpacity
        | SettingsButtonAction::CycleShadowSoftness
        | SettingsButtonAction::CycleBrightness
        | SettingsButtonAction::CycleMasterVolume
        | SettingsButtonAction::ToggleAudioMute
        | SettingsButtonAction::CycleFps
        | SettingsButtonAction::ResetDefaults => match interaction {
            Interaction::Pressed => Color::srgb(0.18, 0.22, 0.36),
            Interaction::Hovered => Color::srgb(0.135, 0.17, 0.29),
            Interaction::None => Color::srgb(0.095, 0.12, 0.21),
        },
    }
}

pub(super) fn handle_settings_buttons(
    mut interactions: Query<
        (&Interaction, &SettingsButtonAction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>),
    >,
    mut settings: ResMut<NeraSettings>,
    mut startup_state: ResMut<StartupRuntime>,
    mut position_runtime: ResMut<PositionRuntime>,
    mut input_gate: ResMut<SettingsInputGate>,
    mut primary: Query<&mut Window, With<PrimaryWindow>>,
    mut commands: Commands,
    mut exit: MessageWriter<AppExit>,
) {
    for (interaction, action, mut background) in &mut interactions {
        *background = BackgroundColor(button_color(*action, *interaction));

        if *interaction != Interaction::Pressed {
            continue;
        }

        match action {
            SettingsButtonAction::ToggleAlwaysOnTop => {
                settings.toggle_always_on_top();
                save_settings_deferred(&mut commands);
            }
            SettingsButtonAction::ToggleStartWithWindows => {
                let desired = !startup_state.enabled;
                match startup::set_enabled(desired) {
                    Ok(()) => {
                        startup_state.enabled = desired;
                        if cfg!(debug_assertions) && desired {
                            warn!(
                                "Settings: Start with Windows points to the current debug executable; disable it before deleting target/debug"
                            );
                        }
                        info!(
                            "Settings: Start with Windows {}",
                            if desired { "ON" } else { "OFF" }
                        );
                    }
                    Err(error) => {
                        warn!("Settings: failed to change Start with Windows: {error}");
                    }
                }
            }
            SettingsButtonAction::ToggleStartHidden => {
                settings.start_hidden = !settings.start_hidden;
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Start hidden {} (applies next launch)",
                    if settings.start_hidden { "ON" } else { "OFF" }
                );
            }
            SettingsButtonAction::ToggleRememberPosition => {
                settings.remember_position = !settings.remember_position;
                if settings.remember_position {
                    settings.window_x = position_runtime.last_known.x;
                    settings.window_y = position_runtime.last_known.y;
                }
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Remember position {}",
                    if settings.remember_position {
                        "ON"
                    } else {
                        "OFF"
                    }
                );
            }
            SettingsButtonAction::CycleCharacterSize => {
                settings.cycle_character_size();
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Character size -> {}",
                    settings.character_size_label()
                );
            }
            SettingsButtonAction::CycleGroundOffset => {
                settings.cycle_ground_offset();
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Ground offset -> {}",
                    settings.ground_offset_label()
                );
            }
            SettingsButtonAction::ToggleWalking => {
                settings.walking_enabled = !settings.walking_enabled;
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Walking {}",
                    if settings.walking_enabled { "ON" } else { "OFF" }
                );
            }
            SettingsButtonAction::ToggleFollowMouse => {
                settings.follow_mouse = !settings.follow_mouse;
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Follow mouse {}",
                    if settings.follow_mouse { "ON" } else { "OFF" }
                );
            }
            SettingsButtonAction::ToggleFollowEyes => {
                settings.follow_eyes = !settings.follow_eyes;
                if !settings.follow_eyes {
                    // BodyTracking extends LookAt, so Head cannot stay enabled
                    // after its LookAt/eyes dependency is disabled.
                    settings.follow_head = false;
                }
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Eyes {}",
                    if settings.follow_eyes { "ON" } else { "OFF" }
                );
            }
            SettingsButtonAction::ToggleFollowHead => {
                settings.follow_head = !settings.follow_head;
                if settings.follow_head {
                    // Head follow is implemented through BodyTracking + LookAt.
                    settings.follow_eyes = true;
                }
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Head follow {}",
                    if settings.follow_head { "ON" } else { "OFF" }
                );
            }
            SettingsButtonAction::CycleFollowIntensity => {
                settings.cycle_follow_intensity();
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Follow intensity -> {}",
                    settings.follow_intensity_label()
                );
            }
            SettingsButtonAction::ToggleShadow => {
                settings.shadow_enabled = !settings.shadow_enabled;
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Shadow {}",
                    if settings.shadow_enabled { "ON" } else { "OFF" }
                );
            }
            SettingsButtonAction::CycleShadowType => {
                settings.cycle_shadow_type();
                save_settings_deferred(&mut commands);
                info!("Settings: Shadow type -> {}", settings.shadow_type_label());
            }
            SettingsButtonAction::CycleShadowOpacity => {
                settings.cycle_shadow_opacity();
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Shadow opacity -> {}",
                    settings.shadow_opacity_label()
                );
            }
            SettingsButtonAction::CycleShadowSoftness => {
                settings.cycle_shadow_softness();
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Shadow softness -> {}",
                    settings.shadow_softness_label()
                );
            }
            SettingsButtonAction::CycleBrightness => {
                settings.cycle_brightness();
                save_settings_deferred(&mut commands);
                info!("Settings: Brightness -> {}", settings.brightness_label());
            }
            SettingsButtonAction::CycleMasterVolume => {
                settings.cycle_master_volume();
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Master volume -> {}",
                    settings.master_volume_label()
                );
            }
            SettingsButtonAction::ToggleAudioMute => {
                settings.audio_muted = !settings.audio_muted;
                save_settings_deferred(&mut commands);
                info!(
                    "Settings: Audio mute {}",
                    if settings.audio_muted { "ON" } else { "OFF" }
                );
            }
            SettingsButtonAction::CycleFps => {
                settings.cycle_fps_limit();
                save_settings_deferred(&mut commands);
            }
            SettingsButtonAction::BackToMenu => {
                input_gate.return_to_menu_requested = true;
                input_gate.close_requested = true;
                info!("Nera Settings -> back to action menu requested");
            }
            SettingsButtonAction::ResetDefaults => {
                *settings = NeraSettings::default();

                if let Err(error) = startup::set_enabled(false) {
                    warn!("Settings: reset could not disable Start with Windows: {error}");
                } else {
                    startup_state.enabled = false;
                }

                let default_position = IVec2::new(settings.window_x, settings.window_y);
                position_runtime.last_known = default_position;
                position_runtime.dirty = false;

                if let Ok(mut window) = primary.single_mut() {
                    window.position = WindowPosition::At(default_position);
                }

                save_settings_deferred(&mut commands);
                info!("Settings: restored defaults");
            }
            SettingsButtonAction::CloseWindow => {
                input_gate.return_to_menu_requested = false;
                // Do NOT despawn the native window while this press is still
                // active. If the HWND disappears before winit receives the
                // matching mouse-up, ButtonInput can remain stuck forever.
                input_gate.close_requested = true;
                info!("Nera Settings -> close requested");
            }
            SettingsButtonAction::Exit => {
                save_settings_before_exit(&mut commands);
                exit.write(AppExit::Success);
            }
        }
    }
}

pub(super) fn drag_settings_window(
    mouse: Res<ButtonInput<MouseButton>>,
    drag_regions: Query<&Interaction, (Changed<Interaction>, With<SettingsDragRegion>)>,
    mut windows: Query<&mut Window, With<SettingsWindow>>,
) {
    // winit expects start_drag_move() immediately after a real left-button
    // press. Calling it at other times can fail (or panic on some versions).
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

pub(super) fn scroll_settings_window(
    mut wheel: MessageReader<MouseWheel>,
    settings_windows: Query<Entity, With<SettingsWindow>>,
    mut scroll_areas: Query<(&mut ScrollPosition, &ComputedNode), With<SettingsScrollArea>>,
) {
    let Ok(settings_window) = settings_windows.single() else {
        return;
    };

    let Ok((mut scroll_position, computed)) = scroll_areas.single_mut() else {
        return;
    };

    // Computed sizes are physical pixels; convert the scroll range back to the
    // logical-pixel coordinate space used by ScrollPosition.
    let max_y = ((computed.content_size().y - computed.size().y) * computed.inverse_scale_factor())
        .max(0.0);

    for event in wheel.read() {
        // MouseWheel carries the Bevy window that received the event. This is
        // important because Nera has both the transparent pet window and the
        // utility Settings window alive at the same time.
        if event.window != settings_window {
            continue;
        }

        // Positive wheel Y means scrolling upward. ScrollPosition grows down,
        // hence the subtraction. Line wheels move about one settings row while
        // touchpads keep their native pixel delta for smooth scrolling.
        let delta_y = match event.unit {
            MouseScrollUnit::Line => event.y * 42.0,
            MouseScrollUnit::Pixel => event.y,
        };

        scroll_position.0.y = (scroll_position.0.y - delta_y).clamp(0.0, max_y);
    }
}

pub(super) fn update_settings_labels(
    settings: Res<NeraSettings>,
    startup_state: Res<StartupRuntime>,
    mut labels: Query<(
        &mut Text,
        Option<&AlwaysOnTopValue>,
        Option<&StartWithWindowsValue>,
        Option<&StartHiddenValue>,
        Option<&RememberPositionValue>,
        (Option<&WalkingEnabledValue>, Option<&FollowMouseValue>),
        Option<&FollowEyesValue>,
        Option<&FollowHeadValue>,
        Option<&FollowIntensityValue>,
        Option<&ShadowEnabledValue>,
        Option<&ShadowTypeValue>,
        Option<&ShadowOpacityValue>,
        Option<&ShadowSoftnessValue>,
        Option<&BrightnessValue>,
        Option<&FpsValue>,
    )>,
    mut walking_checkmarks: Query<&mut Node, With<WalkingCheckmark>>,
) {
    if !settings.is_changed() && !startup_state.is_changed() {
        return;
    }

    for (
        mut text,
        always_on_top,
        start_with_windows,
        start_hidden,
        remember_position,
        (walking_enabled, follow_mouse),
        follow_eyes,
        follow_head,
        follow_intensity,
        shadow_enabled,
        shadow_type,
        shadow_opacity,
        shadow_softness,
        brightness,
        fps,
    ) in &mut labels
    {
        if always_on_top.is_some() {
            text.0 = if settings.always_on_top { "ON" } else { "OFF" }.into();
        } else if start_with_windows.is_some() {
            text.0 = if startup_state.enabled { "ON" } else { "OFF" }.into();
        } else if start_hidden.is_some() {
            text.0 = if settings.start_hidden { "ON" } else { "OFF" }.into();
        } else if remember_position.is_some() {
            text.0 = if settings.remember_position {
                "ON"
            } else {
                "OFF"
            }
            .into();
        } else if walking_enabled.is_some() {
            text.0 = if settings.walking_enabled { "Yes" } else { "No" }.into();
        } else if follow_mouse.is_some() {
            text.0 = if settings.follow_mouse { "ON" } else { "OFF" }.into();
        } else if follow_eyes.is_some() {
            text.0 = if settings.follow_eyes { "ON" } else { "OFF" }.into();
        } else if follow_head.is_some() {
            text.0 = if settings.follow_head { "ON" } else { "OFF" }.into();
        } else if follow_intensity.is_some() {
            text.0 = settings.follow_intensity_label();
        } else if shadow_enabled.is_some() {
            text.0 = if settings.shadow_enabled { "ON" } else { "OFF" }.into();
        } else if shadow_type.is_some() {
            text.0 = settings.shadow_type_label().into();
        } else if shadow_opacity.is_some() {
            text.0 = settings.shadow_opacity_label();
        } else if shadow_softness.is_some() {
            text.0 = settings.shadow_softness_label();
        } else if brightness.is_some() {
            text.0 = settings.brightness_label();
        } else if fps.is_some() {
            text.0 = settings.fps_label();
        }
    }

    for mut node in &mut walking_checkmarks {
        node.display = if settings.walking_enabled {
            Display::Flex
        } else {
            Display::None
        };
    }
}

pub(super) fn update_character_settings_labels(
    settings: Res<NeraSettings>,
    mut size_labels: Query<&mut Text, With<CharacterSizeValue>>,
    mut ground_labels: Query<&mut Text, (With<GroundOffsetValue>, Without<CharacterSizeValue>)>,
) {
    if !settings.is_changed() {
        return;
    }

    for mut text in &mut size_labels {
        text.0 = settings.character_size_label();
    }

    for mut text in &mut ground_labels {
        text.0 = settings.ground_offset_label();
    }
}

pub(super) fn update_audio_settings_labels(
    settings: Res<NeraSettings>,
    mut volume_labels: Query<&mut Text, With<MasterVolumeValue>>,
    mut mute_labels: Query<&mut Text, (With<AudioMutedValue>, Without<MasterVolumeValue>)>,
) {
    if !settings.is_changed() {
        return;
    }

    for mut text in &mut volume_labels {
        text.0 = settings.master_volume_label();
    }

    for mut text in &mut mute_labels {
        text.0 = if settings.audio_muted { "ON" } else { "OFF" }.into();
    }
}

pub(super) fn cleanup_settings_window(
    settings_windows: Query<(), With<SettingsWindow>>,
    owned: Query<(Entity, &SettingsOwned)>,
    mut commands: Commands,
) {
    for (entity, owner) in &owned {
        if settings_windows.get(owner.window).is_ok() {
            continue;
        }

        commands.entity(entity).despawn();
    }
}

/// Close Settings only after the button gesture that requested the close has
/// fully released while the Settings HWND still exists. This guarantees winit
/// receives the mouse-up event and prevents global ButtonInput from getting
/// stuck in the pressed state.
pub(super) fn close_settings_after_mouse_release(
    mouse: Res<ButtonInput<MouseButton>>,
    settings_windows: Query<Entity, With<SettingsWindow>>,
    mut gate: ResMut<SettingsInputGate>,
    mut return_to_menu: MessageWriter<ReturnToActionMenu>,
    mut commands: Commands,
) {
    if !gate.window_open || !gate.close_requested {
        return;
    }

    let any_pressed = mouse.pressed(MouseButton::Left)
        || mouse.pressed(MouseButton::Right)
        || mouse.pressed(MouseButton::Middle);

    if any_pressed {
        return;
    }

    if let Ok(window) = settings_windows.single() {
        commands.entity(window).despawn();
    }

    let reopen_menu = gate.return_to_menu_requested;

    gate.window_open = false;
    gate.close_requested = false;
    gate.return_to_menu_requested = false;

    if reopen_menu {
        return_to_menu.write(ReturnToActionMenu);
        info!("Nera Settings -> closed, returning to radial menu");
    } else {
        info!("Nera Settings -> closed after mouse release");
    }
}

/// Covers non-button closure paths such as the OS destroying the secondary
/// window. There is no post-despawn release quarantine: the normal X path
/// already waits for release before despawning.
pub(super) fn observe_settings_window_removed(
    mut removed: RemovedComponents<SettingsWindow>,
    mut gate: ResMut<SettingsInputGate>,
) {
    if removed.read().next().is_none() {
        return;
    }

    gate.window_open = false;
    gate.close_requested = false;
    gate.return_to_menu_requested = false;
}

#[cfg(test)]
mod walking_checkbox_tests {
    use super::*;

    #[test]
    fn labels_and_checkbox_refresh_when_walking_is_enabled_and_reset() {
        let mut app = App::new();
        app.init_resource::<NeraSettings>()
            .init_resource::<StartupRuntime>()
            .add_systems(Update, update_settings_labels);
        let value = app.world_mut().spawn((Text::new(""), WalkingEnabledValue)).id();
        let checkmark = app
            .world_mut()
            .spawn((Node::default(), WalkingCheckmark))
            .id();
        let follow = app.world_mut().spawn((Text::new(""), FollowMouseValue)).id();

        app.update();
        assert_eq!(app.world().get::<Text>(value).unwrap().0, "No");
        assert_eq!(app.world().get::<Node>(checkmark).unwrap().display, Display::None);
        assert_eq!(app.world().get::<Text>(follow).unwrap().0, "ON");

        app.world_mut().resource_mut::<NeraSettings>().walking_enabled = true;
        app.update();
        assert_eq!(app.world().get::<Text>(value).unwrap().0, "Yes");
        assert_eq!(app.world().get::<Node>(checkmark).unwrap().display, Display::Flex);

        *app.world_mut().resource_mut::<NeraSettings>() = NeraSettings::default();
        app.update();
        assert_eq!(app.world().get::<Text>(value).unwrap().0, "No");
        assert_eq!(app.world().get::<Node>(checkmark).unwrap().display, Display::None);
    }
}
