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
use crate::utility_window::{UtilityWindow, spawn_window_controls};

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
    minimized: bool,
    close_requested: bool,
    return_to_menu_requested: bool,
}

pub(crate) fn nera_input_available(gate: Res<SettingsInputGate>) -> bool {
    !gate.window_open || gate.minimized
}

pub(crate) fn nera_input_blocked(gate: Res<SettingsInputGate>) -> bool {
    gate.window_open && !gate.minimized
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
pub(super) struct ActivityLevelValue;

#[derive(Component)]
pub(super) struct RoamingRadiusValue;

#[derive(Component)]
pub(super) struct PauseFullscreenValue;

#[derive(Component)]
pub(super) struct RoamingRadiusTrack;

#[derive(Component)]
pub(super) struct RoamingRadiusThumb;

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

#[derive(Component)]
struct SettingsFooter;

#[derive(Component)]
pub(super) struct CharacterImportStatus;

#[derive(Component)]
pub(super) struct GravityValue;

#[derive(Component)]
pub(super) struct ThrowValue;
#[derive(Component)]
pub(super) struct ThrowCheckmark;
#[derive(Component)]
pub(super) struct ThrowIntensityValue;
#[derive(Component)]
pub(super) struct ThrowIntensityTrack;
#[derive(Component)]
pub(super) struct ThrowIntensityThumb;

#[derive(Component, Clone, Copy)]
pub(super) enum SettingsButtonAction {
    ToggleAlwaysOnTop,
    ToggleStartWithWindows,
    ToggleStartHidden,
    ToggleRememberPosition,
    CycleCharacterSize,
    CycleGroundOffset,
    ToggleWalking,
    ToggleGravity,
    ToggleThrow,
    OpenCharacterManager,
    CycleActivityLevel,
    TogglePauseFullscreen,
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
    ImportCharacter,
}

pub(super) fn open_settings_window(
    mut open: MessageReader<OpenSettingsWindow>,
    mut existing: Query<&mut Window, With<SettingsWindow>>,
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
        for mut window in &mut existing {
            window.set_minimized(false);
            window.focused = true;
        }
        input_gate.window_open = true;
        input_gate.minimized = false;
        return;
    }

    // While Settings exists, it exclusively owns application mouse input.
    // Closing is deferred until the close-button mouse gesture has released,
    // so winit never loses the corresponding mouse-up event.
    input_gate.window_open = true;
    input_gate.minimized = false;
    input_gate.close_requested = false;
    input_gate.return_to_menu_requested = false;

    let window_entity = commands
        .spawn((
            Name::new("Nera Settings Window"),
            SettingsWindow,
            crate::utility_window::font_cache::FontCacheOwner,
            UtilityWindow::new(14.0),
            Window {
                title: "Nera Settings".into(),
                resolution: (580, 820).into(),
                resizable: true,
                resize_constraints: WindowResizeConstraints {
                    min_width: 560.0,
                    min_height: 520.0,
                    ..default()
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
            crate::utility_window::font_cache::FontCacheOwner,
            SettingsOwned {
                window: window_entity,
            },
            Camera2d,
            // UI uses analytic edge/glyph smoothing; a separate 4x target is unnecessary.
            Msaa::Off,
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
            crate::utility_window::font_cache::FontCacheOwner,
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
                spawn_settings_header(panel, window);

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
                            settings_row(card, "Gravity", if settings.gravity_enabled { "ON" } else { "OFF" }.into(), GravityValue, SettingsButtonAction::ToggleGravity);
                            checkbox_row(card, "Throw Physics", if settings.throw_enabled { "ON" } else { "OFF" }, settings.throw_enabled, SettingsButtonAction::ToggleThrow, (ThrowValue, ThrowCheckmark));
                            settings_slider(card, "Throw Intensity", settings.throw_intensity_label(), f32::from(settings.throw_intensity.min(100)) / 100.0, (ThrowIntensityValue, ThrowIntensityTrack, ThrowIntensityThumb));
                            walking_checkbox(card, settings.walking_enabled);
                            settings_row(
                                card,
                                "Activity",
                                settings.activity_level().label().into(),
                                ActivityLevelValue,
                                SettingsButtonAction::CycleActivityLevel,
                            );
                            roaming_radius_slider(card, settings.roaming_radius_px());
                            settings_row(
                                card,
                                "Pause in Fullscreen",
                                if settings.pause_roaming_fullscreen { "ON" } else { "OFF" }.into(),
                                PauseFullscreenValue,
                                SettingsButtonAction::TogglePauseFullscreen,
                            );
                            card.spawn((
                                Text::new("Drag Nera to set home · Roaming stays on the current monitor"),
                                TextFont { font_size: FontSize::Px(11.0), ..default() },
                                TextColor(Color::srgb(0.45, 0.5, 0.64)),
                            ));
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

                    });

                // A sibling of the scroll viewport, so these actions stay
                // visible even at the minimum window height.
                panel.spawn((
                    Name::new("Nera Settings Fixed Navigation"),
                    SettingsFooter,
                    Node {
                        width: percent(100),
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: px(8),
                        padding: UiRect::top(px(8)),
                        ..default()
                    },
                )).with_children(|footer| {
                    action_button(footer, "Multi Character", SettingsButtonAction::OpenCharacterManager);
                    action_button(footer, "Reset to defaults", SettingsButtonAction::ResetDefaults);
                    action_button(footer, "Exit Nera", SettingsButtonAction::Exit);
                    action_button(footer, "Import Character (.vrm)", SettingsButtonAction::ImportCharacter);
                    footer.spawn((
                        CharacterImportStatus,
                        Text::new("Choose a VRM character (0.x or 1.0)"),
                        TextFont { font_size: FontSize::Px(12.0), ..default() },
                        TextColor(Color::srgb(0.60, 0.67, 0.82)),
                        Node { max_height: px(48), overflow: Overflow::clip(), ..default() },
                    ));
                });
            });
        });
}

fn spawn_settings_header(parent: &mut ChildSpawnerCommands, window: Entity) {
    parent
        .spawn(Node {
            width: percent(100),
            height: px(58),
            flex_shrink: 0.0,
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
                        flex_shrink: 0.0,
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
                            font_size: FontSize::Px(23.0),
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

            spawn_window_controls(header, window);

            header
                .spawn((
                    Button,
                    SettingsButtonAction::CloseWindow,
                    Node {
                        width: px(42),
                        height: px(42),
                        flex_shrink: 0.0,
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
    checkbox_row(
        parent,
        "Allow Roaming",
        if enabled { "Yes" } else { "No" },
        enabled,
        SettingsButtonAction::ToggleWalking,
        (WalkingEnabledValue, WalkingCheckmark),
    );
}

fn checkbox_row<V: Component, C: Component>(
    parent: &mut ChildSpawnerCommands,
    label: &'static str,
    value: &'static str,
    enabled: bool,
    action: SettingsButtonAction,
    markers: (V, C),
) {
    let (value_marker, checkmark_marker) = markers;
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
                Name::new(format!("{label} checkbox")),
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
                                checkmark_marker,
                                Node {
                                    width: px(18),
                                    height: px(18),
                                    display: if enabled {
                                        Display::Flex
                                    } else {
                                        Display::None
                                    },
                                    ..default()
                                },
                            ))
                            .with_children(|tick| {
                                // Draw the tick directly: the bundled font is a
                                // subset and need not contain a checkmark glyph.
                                for (left, top, width, angle) in
                                    [(2.0, 9.5, 6.0, 45.0), (5.5, 7.5, 11.0, -45.0)]
                                {
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
                    value_marker,
                    Text::new(value),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.88, 0.9, 0.98)),
                ));
            });
        });
}

fn roaming_radius_slider(parent: &mut ChildSpawnerCommands, radius: f32) {
    settings_slider(
        parent,
        "Roaming Distance",
        format!("{radius:.0} px"),
        (radius - 300.0) / 300.0,
        (RoamingRadiusValue, RoamingRadiusTrack, RoamingRadiusThumb),
    );
}

fn settings_slider<V: Component, T: Component, H: Component>(
    parent: &mut ChildSpawnerCommands,
    label: &'static str,
    value: String,
    position: f32,
    markers: (V, T, H),
) {
    let (value_marker, track_marker, thumb_marker) = markers;
    parent
        .spawn(Node {
            width: percent(100),
            min_height: px(66),
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            ..default()
        })
        .with_children(|row| {
            row.spawn(Node {
                width: percent(100),
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            })
            .with_children(|labels| {
                labels.spawn((
                    Text::new(label),
                    TextFont {
                        font_size: FontSize::Px(16.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.82, 0.85, 0.94)),
                ));
                labels.spawn((
                    value_marker,
                    Text::new(value),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.88, 0.9, 0.98)),
                ));
            });
            row.spawn((
                Button,
                track_marker,
                bevy::ui::RelativeCursorPosition::default(),
                Node {
                    width: percent(100),
                    height: px(28),
                    border_radius: BorderRadius::all(px(10)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.095, 0.12, 0.21)),
            ))
            .with_children(|track| {
                track.spawn((
                    thumb_marker,
                    bevy::picking::Pickable::IGNORE,
                    Node {
                        position_type: PositionType::Absolute,
                        left: percent(position.clamp(0.0, 1.0) * 100.0),
                        margin: UiRect::left(px(-7)),
                        top: px(5),
                        width: px(14),
                        height: px(18),
                        border_radius: BorderRadius::all(px(7)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.61, 0.73, 0.98)),
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
                flex_shrink: 0.0,
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
        SettingsButtonAction::ImportCharacter => match interaction {
            Interaction::Pressed => Color::srgb(0.10, 0.27, 0.72),
            Interaction::Hovered => Color::srgb(0.16, 0.39, 0.94),
            Interaction::None => Color::srgb(0.12, 0.32, 0.82),
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
        | SettingsButtonAction::ToggleGravity
        | SettingsButtonAction::ToggleThrow
        | SettingsButtonAction::OpenCharacterManager
        | SettingsButtonAction::CycleActivityLevel
        | SettingsButtonAction::TogglePauseFullscreen
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
    mut restore_character: MessageWriter<crate::character::RestoreDefaultCharacter>,
    mut characters: MessageWriter<crate::animation_manager::OpenCharacterManager>,
) {
    for (interaction, action, mut background) in &mut interactions {
        *background = BackgroundColor(button_color(*action, *interaction));

        if *interaction != Interaction::Pressed {
            continue;
        }

        match action {
            SettingsButtonAction::OpenCharacterManager => {
                characters.write_default();
                input_gate.close_requested = true;
            }
            SettingsButtonAction::ToggleGravity => {
                settings.gravity_enabled = !settings.gravity_enabled;
                save_settings_deferred(&mut commands);
            }
            SettingsButtonAction::ToggleThrow => {
                settings.throw_enabled = !settings.throw_enabled;
                save_settings_deferred(&mut commands);
            }
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
                    "Settings: Allow Roaming {}",
                    if settings.walking_enabled {
                        "ON"
                    } else {
                        "OFF"
                    }
                );
            }
            SettingsButtonAction::CycleActivityLevel => {
                settings.cycle_activity_level();
                save_settings_deferred(&mut commands);
            }
            SettingsButtonAction::TogglePauseFullscreen => {
                settings.pause_roaming_fullscreen = !settings.pause_roaming_fullscreen;
                save_settings_deferred(&mut commands);
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
                restore_character.write(crate::character::RestoreDefaultCharacter);

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
            // The picker has its own system so it can use the utility HWND
            // without borrowing the primary pet Window.
            SettingsButtonAction::ImportCharacter => {}
        }
    }
}

pub(super) fn import_character_from_settings(
    actions: Query<(&Interaction, &SettingsButtonAction), Changed<Interaction>>,
    windows: Query<&UtilityWindow, With<SettingsWindow>>,
    mut status: ResMut<crate::character::CharacterImportStatus>,
    mut imports: MessageWriter<crate::character::ImportCharacter>,
) {
    if status.loading
        || !actions.iter().any(|(interaction, action)| {
            *interaction == Interaction::Pressed
                && matches!(action, SettingsButtonAction::ImportCharacter)
        })
    {
        return;
    }
    let Some(owner) = windows.single().ok().and_then(UtilityWindow::native_handle) else {
        status.message = "Window is still opening; try Import again.".into();
        return;
    };
    match crate::platform::pick_asset_file(
        owner,
        "Import Character",
        "VRM character (*.vrm)\0*.vrm\0\0",
    ) {
        Ok(Some(path)) => {
            imports.write(crate::character::ImportCharacter(path));
        }
        Ok(None) => {}
        Err(error) => status.message = error,
    }
}

pub(super) fn update_gravity_label(
    settings: Res<NeraSettings>,
    mut labels: Query<&mut Text, With<GravityValue>>,
) {
    for mut label in &mut labels {
        label.set_if_neq(Text::new(if settings.gravity_enabled {
            "ON"
        } else {
            "OFF"
        }));
    }
}

fn throw_percent_from_cursor(x: f32) -> u8 {
    ((x + 0.5).clamp(0.0, 1.0) * 100.0).round() as u8
}

#[cfg(debug_assertions)]
pub(crate) fn smoke_throw_controls(world: &mut World, intensity: u8) {
    use bevy::ecs::system::RunSystemOnce;
    let toggle = world
        .query::<(Entity, &SettingsButtonAction)>()
        .iter(world)
        .find_map(|(entity, action)| {
            matches!(action, SettingsButtonAction::ToggleThrow).then_some(entity)
        })
        .unwrap();
    *world.get_mut::<Interaction>(toggle).unwrap() = Interaction::Pressed;
    world.run_system_once(handle_settings_buttons).unwrap();
    *world.get_mut::<Interaction>(toggle).unwrap() = Interaction::None;

    let track = world
        .query_filtered::<Entity, With<ThrowIntensityTrack>>()
        .single(world)
        .unwrap();
    *world.get_mut::<Interaction>(track).unwrap() = Interaction::Pressed;
    world
        .get_mut::<bevy::ui::RelativeCursorPosition>(track)
        .unwrap()
        .normalized = Some(Vec2::new(f32::from(intensity) / 100.0 - 0.5, 0.0));
    world
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    world
        .run_system_cached(handle_throw_intensity_slider)
        .unwrap();
    world
        .resource_mut::<ButtonInput<MouseButton>>()
        .release(MouseButton::Left);
    world
        .run_system_cached(handle_throw_intensity_slider)
        .unwrap();
    world.resource_mut::<ButtonInput<MouseButton>>().clear();
    *world.get_mut::<Interaction>(track).unwrap() = Interaction::None;
}

pub(super) fn handle_throw_intensity_slider(
    mouse: Res<ButtonInput<MouseButton>>,
    tracks: Query<(&Interaction, &bevy::ui::RelativeCursorPosition), With<ThrowIntensityTrack>>,
    mut dragging: Local<bool>,
    mut settings: ResMut<NeraSettings>,
    mut commands: Commands,
) {
    if !mouse.pressed(MouseButton::Left) {
        if *dragging {
            save_settings_deferred(&mut commands);
        }
        *dragging = false;
        return;
    }
    let Ok((interaction, cursor)) = tracks.single() else {
        *dragging = false;
        return;
    };
    if mouse.just_pressed(MouseButton::Left) && *interaction == Interaction::Pressed {
        *dragging = true;
    }
    if *dragging && let Some(cursor) = cursor.normalized {
        let percent = throw_percent_from_cursor(cursor.x);
        if settings.throw_intensity != percent {
            settings.throw_intensity = percent;
        }
    }
}

pub(super) fn update_throw_settings_labels(
    settings: Res<NeraSettings>,
    mut labels: Query<(&mut Text, Option<&ThrowValue>, Option<&ThrowIntensityValue>)>,
    mut checkmarks: Query<&mut Node, With<ThrowCheckmark>>,
    mut thumbs: Query<&mut Node, (With<ThrowIntensityThumb>, Without<ThrowCheckmark>)>,
) {
    if !settings.is_changed() {
        return;
    }
    for (mut text, enabled, intensity) in &mut labels {
        if enabled.is_some() {
            text.0 = if settings.throw_enabled { "ON" } else { "OFF" }.into();
        } else if intensity.is_some() {
            text.0 = settings.throw_intensity_label();
        }
    }
    for mut mark in &mut checkmarks {
        mark.display = if settings.throw_enabled {
            Display::Flex
        } else {
            Display::None
        };
    }
    for mut thumb in &mut thumbs {
        thumb.left = percent(f32::from(settings.throw_intensity.min(100)));
    }
}

pub(super) fn update_character_import_status(
    status: Res<crate::character::CharacterImportStatus>,
    mut labels: Query<&mut Text, With<CharacterImportStatus>>,
) {
    let message = if status.message.is_empty() {
        "Choose a VRM character (0.x or 1.0)"
    } else {
        &status.message
    };
    for mut text in &mut labels {
        if text.0 != message {
            text.0 = message.to_owned();
        }
    }
}

pub(super) fn sync_settings_minimized(
    windows: Query<&UtilityWindow, With<SettingsWindow>>,
    mut gate: ResMut<SettingsInputGate>,
) {
    let minimized = windows.iter().any(|window| {
        window
            .native_handle()
            .is_some_and(crate::platform::is_window_minimized)
    });
    if gate.minimized != minimized {
        gate.minimized = minimized;
    }
}

pub(super) fn handle_roaming_radius_slider(
    mouse: Res<ButtonInput<MouseButton>>,
    tracks: Query<(&Interaction, &bevy::ui::RelativeCursorPosition), With<RoamingRadiusTrack>>,
    mut dragging: Local<bool>,
    mut settings: ResMut<NeraSettings>,
    mut commands: Commands,
) {
    if !mouse.pressed(MouseButton::Left) {
        if *dragging {
            save_settings_deferred(&mut commands);
        }
        *dragging = false;
        return;
    }
    let Ok((interaction, cursor)) = tracks.single() else {
        *dragging = false;
        return;
    };
    if mouse.just_pressed(MouseButton::Left) && *interaction == Interaction::Pressed {
        *dragging = true;
    }
    if *dragging {
        if let Some(cursor) = cursor.normalized {
            // Bevy 0.19 UI coordinates are normalized around the node center.
            let radius = (300.0 + (cursor.x + 0.5).clamp(0.0, 1.0) * 300.0).round() as u16;
            if settings.roaming_radius_px != radius {
                settings.roaming_radius_px = radius;
            }
        }
    }
}

pub(super) fn update_roaming_settings_labels(
    settings: Res<NeraSettings>,
    mut labels: Query<(
        &mut Text,
        Option<&ActivityLevelValue>,
        Option<&RoamingRadiusValue>,
        Option<&PauseFullscreenValue>,
    )>,
    mut thumbs: Query<&mut Node, With<RoamingRadiusThumb>>,
) {
    if !settings.is_changed() {
        return;
    }
    for (mut text, activity, radius, fullscreen) in &mut labels {
        if activity.is_some() {
            text.0 = settings.activity_level().label().into();
        } else if radius.is_some() {
            text.0 = format!("{:.0} px", settings.roaming_radius_px());
        } else if fullscreen.is_some() {
            text.0 = if settings.pause_roaming_fullscreen {
                "ON"
            } else {
                "OFF"
            }
            .into();
        }
    }
    for mut thumb in &mut thumbs {
        thumb.left = percent((settings.roaming_radius_px() - 300.0) / 300.0 * 100.0);
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
            text.0 = if settings.walking_enabled {
                "Yes"
            } else {
                "No"
            }
            .into();
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
    gate.minimized = false;
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
    gate.minimized = false;
    gate.close_requested = false;
    gate.return_to_menu_requested = false;
}

#[cfg(test)]
mod walking_checkbox_tests {
    use super::*;

    #[test]
    fn settings_camera_and_tree_are_released_without_touching_main_camera() {
        let mut app = App::new();
        app.init_resource::<NeraSettings>()
            .init_resource::<StartupRuntime>()
            .init_resource::<SettingsInputGate>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<OpenSettingsWindow>()
            .add_message::<ReturnToActionMenu>()
            .add_systems(
                Update,
                (
                    open_settings_window,
                    close_settings_after_mouse_release,
                    cleanup_settings_window,
                    observe_settings_window_removed,
                )
                    .chain(),
            );
        let main_camera = app
            .world_mut()
            .spawn((Camera3d::default(), Msaa::Sample4))
            .id();
        let baseline = app.world_mut().query::<Entity>().iter(app.world()).count();
        for cycle in 0..6 {
            app.world_mut().write_message(OpenSettingsWindow);
            app.update();
            let world = app.world_mut();
            let camera = world
                .query_filtered::<&Msaa, (With<SettingsOwned>, With<Camera>)>()
                .single(world)
                .unwrap();
            assert_eq!(*camera, Msaa::Off);
            assert!(world.query::<&Node>().iter(world).count() > 0);
            if cycle == 3 {
                let window = world
                    .query_filtered::<Entity, With<SettingsWindow>>()
                    .single(world)
                    .unwrap();
                world.despawn(window);
            } else {
                world.resource_mut::<SettingsInputGate>().close_requested = true;
            }
            app.update();
            let world = app.world_mut();
            assert_eq!(world.query::<&SettingsOwned>().iter(world).count(), 0);
            assert_eq!(world.query::<&SettingsWindow>().iter(world).count(), 0);
            assert_eq!(world.query::<&Node>().iter(world).count(), 0);
            assert_eq!(world.query::<Entity>().iter(world).count(), baseline);
            assert_eq!(world.get::<Msaa>(main_camera), Some(&Msaa::Sample4));
            assert!(!world.resource::<SettingsInputGate>().window_open);
            assert_eq!(world.resource::<NeraSettings>().fps_limit, 24);
        }
    }

    #[test]
    fn allow_roaming_checkbox_changes_the_existing_preference_immediately_without_touching_fps() {
        let mut app = App::new();
        app.init_resource::<NeraSettings>()
            .init_resource::<StartupRuntime>()
            .init_resource::<PositionRuntime>()
            .init_resource::<SettingsInputGate>()
            .add_message::<AppExit>()
            .add_message::<crate::character::RestoreDefaultCharacter>()
            .add_message::<crate::animation_manager::OpenCharacterManager>()
            .add_systems(Update, handle_settings_buttons);
        let button = app
            .world_mut()
            .spawn((
                Button,
                Interaction::Pressed,
                SettingsButtonAction::ToggleWalking,
                BackgroundColor::default(),
            ))
            .id();
        app.update();
        assert!(app.world().resource::<NeraSettings>().walking_enabled);
        app.update();
        assert!(app.world().resource::<NeraSettings>().walking_enabled);
        *app.world_mut().get_mut::<Interaction>(button).unwrap() = Interaction::None;
        app.update();
        *app.world_mut().get_mut::<Interaction>(button).unwrap() = Interaction::Pressed;
        app.update();
        let settings = app.world().resource::<NeraSettings>();
        assert!(!settings.walking_enabled);
        assert_eq!(settings.fps_limit, 24);
        assert_eq!(settings.roaming_radius_px, 400);
    }

    #[test]
    fn throw_controls_refresh_and_reset_without_changing_gravity() {
        let mut app = App::new();
        app.init_resource::<NeraSettings>()
            .add_systems(Update, update_throw_settings_labels);
        let enabled = app.world_mut().spawn((Text::new(""), ThrowValue)).id();
        let intensity = app
            .world_mut()
            .spawn((Text::new(""), ThrowIntensityValue))
            .id();
        let mark = app
            .world_mut()
            .spawn((Node::default(), ThrowCheckmark))
            .id();
        let thumb = app
            .world_mut()
            .spawn((Node::default(), ThrowIntensityThumb))
            .id();
        app.update();
        assert_eq!(app.world().get::<Text>(enabled).unwrap().0, "ON");
        assert_eq!(app.world().get::<Text>(intensity).unwrap().0, "50%");
        assert_eq!(
            app.world().get::<Node>(mark).unwrap().display,
            Display::Flex
        );
        assert_eq!(app.world().get::<Node>(thumb).unwrap().left, percent(50));
        {
            let mut settings = app.world_mut().resource_mut::<NeraSettings>();
            settings.throw_enabled = false;
            settings.throw_intensity = 30;
        }
        app.update();
        assert_eq!(app.world().get::<Text>(enabled).unwrap().0, "OFF");
        assert_eq!(app.world().get::<Text>(intensity).unwrap().0, "30%");
        assert_eq!(
            app.world().get::<Node>(mark).unwrap().display,
            Display::None
        );
        assert_eq!(app.world().get::<Node>(thumb).unwrap().left, percent(30));
        assert!(app.world().resource::<NeraSettings>().gravity_enabled);
        *app.world_mut().resource_mut::<NeraSettings>() = NeraSettings::default();
        app.update();
        assert_eq!(app.world().get::<Text>(enabled).unwrap().0, "ON");
        assert_eq!(app.world().get::<Node>(thumb).unwrap().left, percent(50));
    }

    #[test]
    fn throw_slider_tracks_cursor_from_zero_to_one_hundred_without_changing_roaming() {
        let mut app = App::new();
        app.init_resource::<NeraSettings>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, handle_throw_intensity_slider);
        let track = app
            .world_mut()
            .spawn((
                ThrowIntensityTrack,
                Interaction::Pressed,
                bevy::ui::RelativeCursorPosition {
                    cursor_over: true,
                    normalized: Some(Vec2::new(-0.17, 0.0)),
                },
            ))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert_eq!(app.world().resource::<NeraSettings>().throw_intensity, 33);
        app.world_mut()
            .get_mut::<bevy::ui::RelativeCursorPosition>(track)
            .unwrap()
            .normalized = Some(Vec2::new(1.0, 0.0));
        app.update();
        assert_eq!(app.world().resource::<NeraSettings>().throw_intensity, 100);
        app.world_mut()
            .get_mut::<bevy::ui::RelativeCursorPosition>(track)
            .unwrap()
            .normalized = Some(Vec2::new(-1.0, 0.0));
        app.update();
        assert_eq!(app.world().resource::<NeraSettings>().throw_intensity, 0);
        assert_eq!(
            app.world().resource::<NeraSettings>().roaming_radius_px,
            400
        );
        assert!(app.world().resource::<NeraSettings>().gravity_enabled);
    }

    #[test]
    fn reopening_minimized_settings_restores_existing_window_and_input_gate() {
        use bevy::ecs::system::RunSystemOnce;

        let mut app = App::new();
        app.init_resource::<NeraSettings>()
            .init_resource::<StartupRuntime>()
            .insert_resource(SettingsInputGate {
                window_open: true,
                minimized: true,
                ..default()
            })
            .add_message::<OpenSettingsWindow>()
            .add_systems(Update, open_settings_window);
        let settings_window = app
            .world_mut()
            .spawn((
                SettingsWindow,
                Window {
                    focused: false,
                    ..default()
                },
            ))
            .id();

        assert!(
            app.world_mut()
                .run_system_once(nera_input_available)
                .unwrap()
        );
        assert!(!app.world_mut().run_system_once(nera_input_blocked).unwrap());

        app.world_mut().write_message(OpenSettingsWindow);
        app.update();

        let world = app.world_mut();
        assert_eq!(
            world
                .query_filtered::<Entity, With<SettingsWindow>>()
                .iter(world)
                .count(),
            1
        );
        let mut window = world.get_mut::<Window>(settings_window).unwrap();
        assert!(window.focused);
        assert_eq!(window.internal.take_minimize_request(), Some(false));
        assert!(!world.run_system_once(nera_input_available).unwrap());
        assert!(world.run_system_once(nera_input_blocked).unwrap());
    }

    #[test]
    fn navigation_actions_stay_outside_the_scrolling_settings_body() {
        let mut app = App::new();
        app.add_systems(Startup, |mut commands: Commands| {
            let window = commands.spawn_empty().id();
            let camera = commands.spawn_empty().id();
            spawn_settings_ui(
                &mut commands,
                window,
                camera,
                &NeraSettings::default(),
                &StartupRuntime::default(),
            );
        });
        app.update();
        let world = app.world_mut();
        let footer = world
            .query_filtered::<Entity, With<SettingsFooter>>()
            .single(world)
            .unwrap();
        let scroll = world
            .query_filtered::<Entity, With<SettingsScrollArea>>()
            .single(world)
            .unwrap();
        assert_eq!(
            world.get::<ChildOf>(footer).unwrap().parent(),
            world.get::<ChildOf>(scroll).unwrap().parent()
        );
        assert_eq!(world.get::<Node>(footer).unwrap().flex_shrink, 0.0);
        let actions: Vec<_> = world
            .get::<Children>(footer)
            .unwrap()
            .iter()
            .filter_map(|entity| world.get::<SettingsButtonAction>(entity))
            .collect();
        assert!(matches!(
            actions.as_slice(),
            [
                SettingsButtonAction::OpenCharacterManager,
                SettingsButtonAction::ResetDefaults,
                SettingsButtonAction::Exit,
                SettingsButtonAction::ImportCharacter,
            ]
        ));
        let character_manager_count = world
            .query::<&SettingsButtonAction>()
            .iter(world)
            .filter(|action| matches!(action, SettingsButtonAction::OpenCharacterManager))
            .count();
        assert_eq!(character_manager_count, 1);
        let reset_count = world
            .query::<&SettingsButtonAction>()
            .iter(world)
            .filter(|action| matches!(action, SettingsButtonAction::ResetDefaults))
            .count();
        assert_eq!(reset_count, 1);
        let exit_count = world
            .query::<&SettingsButtonAction>()
            .iter(world)
            .filter(|action| matches!(action, SettingsButtonAction::Exit))
            .count();
        assert_eq!(exit_count, 1);
    }

    #[test]
    fn labels_and_checkbox_refresh_when_walking_is_enabled_and_reset() {
        let mut app = App::new();
        app.init_resource::<NeraSettings>()
            .init_resource::<StartupRuntime>()
            .add_systems(Update, update_settings_labels);
        let value = app
            .world_mut()
            .spawn((Text::new(""), WalkingEnabledValue))
            .id();
        let checkmark = app
            .world_mut()
            .spawn((Node::default(), WalkingCheckmark))
            .id();
        let follow = app
            .world_mut()
            .spawn((Text::new(""), FollowMouseValue))
            .id();

        app.update();
        assert_eq!(app.world().get::<Text>(value).unwrap().0, "No");
        assert_eq!(
            app.world().get::<Node>(checkmark).unwrap().display,
            Display::None
        );
        assert_eq!(app.world().get::<Text>(follow).unwrap().0, "ON");

        app.world_mut()
            .resource_mut::<NeraSettings>()
            .walking_enabled = true;
        app.update();
        assert_eq!(app.world().get::<Text>(value).unwrap().0, "Yes");
        assert_eq!(
            app.world().get::<Node>(checkmark).unwrap().display,
            Display::Flex
        );

        *app.world_mut().resource_mut::<NeraSettings>() = NeraSettings::default();
        app.update();
        assert_eq!(app.world().get::<Text>(value).unwrap().0, "No");
        assert_eq!(
            app.world().get::<Node>(checkmark).unwrap().display,
            Display::None
        );
    }
}
