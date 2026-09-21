//! Project Nera — persistent application settings foundation.
//!
//! Scope of this first settings slice:
//! - persistent settings resource (Bevy SettingsPlugin)
//! - second Bevy window opened from Nera's radial menu
//! - Always-on-top as one source of truth for Settings + tray
//! - FPS cap via WinitSettings
//! - synchronous settings save before Exit

mod startup;
mod shadow;

use std::time::Duration;

use bevy::{
    app::AppExit,
    camera::RenderTarget,
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
    settings::{
        ReflectSettingsGroup,
        SaveSettingsDeferred,
        SaveSettingsSync,
        SettingsGroup,
    },
    ui::UiTargetCamera,
    window::{
        PrimaryWindow,
        WindowLevel,
        WindowMoved,
        WindowPosition,
        WindowRef,
    },
    winit::{UpdateMode, WinitSettings},
};

// =============================================================================
// Persistent state
// =============================================================================

/// Settings that already have a real runtime consumer.
///
/// Keep unfinished controls out of this resource: a setting should only be
/// persisted once the engine actually knows how to apply it.
#[derive(Resource, SettingsGroup, Reflect, Clone, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "general")]
pub(crate) struct NeraSettings {
    pub(crate) always_on_top: bool,
    /// When enabled, the primary Nera window stays hidden on launch until the
    /// tray's Show Nera command makes it visible.
    pub(crate) start_hidden: bool,
    pub(crate) remember_position: bool,
    pub(crate) window_x: i32,
    pub(crate) window_y: i32,

    /// Uniform VRM scale in percent. Kept proportional; no independent X/Y stretch.
    pub(crate) character_size: u16,
    /// Standing support offset in logical pixels. Positive values keep the visible
    /// feet slightly above the desktop support line; negative values sink them lower.
    pub(crate) ground_offset: i16,

    /// Master switch for gaze behavior driven by the global mouse cursor.
    pub(crate) follow_mouse: bool,
    /// Enables VRM LookAt eye tracking while Follow Mouse is active.
    pub(crate) follow_eyes: bool,
    /// Extends LookAt into bevy_vrm1 upper-body tracking.
    pub(crate) follow_head: bool,
    /// Cursor-to-gaze range in percent. UI cycles 25 / 50 / 75 / 100.
    pub(crate) follow_intensity: u8,

    /// Master switch for the desktop-friendly shadow system.
    pub(crate) shadow_enabled: bool,
    /// 0 = Simple blob, 1 = real animated VRM Character shadow.
    pub(crate) shadow_type: u8,
    /// Overall alpha in percent. UI cycles 25 / 40 / 55 / 70.
    pub(crate) shadow_opacity: u8,
    /// Controls how far the layered translucent rings spread.
    /// UI cycles 25 / 50 / 75 / 100.
    pub(crate) shadow_softness: u8,
    /// Key-light intensity percentage. UI cycles 75 / 100 / 125 / 150.
    pub(crate) brightness: u8,

    /// 0 = uncapped update loop. Rendering can still be limited by VSync
    /// because the primary Nera window currently uses AutoVsync.
    pub(crate) fps_limit: u16,
}

impl Default for NeraSettings {
    fn default() -> Self {
        Self {
            always_on_top: true,
            start_hidden: false,
            remember_position: true,
            window_x: 200,
            window_y: 200,
            character_size: 100,
            ground_offset: 0,
            follow_mouse: true,
            follow_eyes: true,
            follow_head: false,
            follow_intensity: 75,
            shadow_enabled: true,
            shadow_type: 0,
            shadow_opacity: 40,
            shadow_softness: 75,
            brightness: 100,
            fps_limit: 60,
        }
    }
}

impl NeraSettings {
    pub(crate) fn toggle_always_on_top(&mut self) {
        self.always_on_top = !self.always_on_top;
    }

    pub(crate) fn character_scale(&self) -> f32 {
        self.character_size.clamp(50, 150) as f32 / 100.0
    }

    pub(crate) fn ground_offset_logical(&self) -> f32 {
        self.ground_offset.clamp(-40, 40) as f32
    }

    fn cycle_character_size(&mut self) {
        self.character_size = match self.character_size {
            50 => 75,
            75 => 100,
            100 => 125,
            125 => 150,
            _ => 50,
        };
    }

    fn character_size_label(&self) -> String {
        format!("{}%", self.character_size.clamp(50, 150))
    }

    fn cycle_ground_offset(&mut self) {
        self.ground_offset = match self.ground_offset {
            -40 => -20,
            -20 => 0,
            0 => 20,
            20 => 40,
            _ => -40,
        };
    }

    fn ground_offset_label(&self) -> String {
        format!("{:+} px", self.ground_offset.clamp(-40, 40))
    }

    fn cycle_follow_intensity(&mut self) {
        self.follow_intensity = match self.follow_intensity {
            25 => 50,
            50 => 75,
            75 => 100,
            _ => 25,
        };
    }

    fn follow_intensity_label(&self) -> String {
        format!("{}%", self.follow_intensity.clamp(25, 100))
    }

    fn cycle_shadow_type(&mut self) {
        self.shadow_type = if self.shadow_type == 0 { 1 } else { 0 };
    }

    fn shadow_type_label(&self) -> &'static str {
        if self.shadow_type == 1 {
            "Character"
        } else {
            "Simple"
        }
    }

    pub(crate) fn uses_simple_shadow(&self) -> bool {
        self.shadow_type != 1
    }

    pub(crate) fn uses_character_shadow(&self) -> bool {
        self.shadow_type == 1
    }

    fn cycle_shadow_opacity(&mut self) {
        self.shadow_opacity = match self.shadow_opacity {
            25 => 40,
            40 => 55,
            55 => 70,
            _ => 25,
        };
    }

    fn shadow_opacity_label(&self) -> String {
        format!("{}%", self.shadow_opacity.clamp(0, 100))
    }

    fn cycle_shadow_softness(&mut self) {
        self.shadow_softness = match self.shadow_softness {
            25 => 50,
            50 => 75,
            75 => 100,
            _ => 25,
        };
    }

    fn shadow_softness_label(&self) -> String {
        format!("{}%", self.shadow_softness.clamp(25, 100))
    }

    fn cycle_brightness(&mut self) {
        self.brightness = match self.brightness {
            75 => 100,
            100 => 125,
            125 => 150,
            _ => 75,
        };
    }

    fn brightness_label(&self) -> String {
        format!("{}%", self.brightness.clamp(50, 200))
    }

    fn cycle_fps_limit(&mut self) {
        self.fps_limit = match self.fps_limit {
            30 => 60,
            60 => 90,
            90 => 120,
            120 => 144,
            144 => 0,
            _ => 30,
        };
    }

    fn fps_label(&self) -> String {
        match self.fps_limit {
            0 => "Unlimited (VSync)".to_string(),
            fps => format!("{fps} FPS"),
        }
    }
}

/// Sent by the radial action menu. The Settings module owns what "opening
/// settings" means, so action_menu.rs does not need to know about UI details.
#[derive(Message, Default)]
pub(crate) struct OpenSettingsWindow;

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
}

pub(crate) fn nera_input_available(gate: Res<SettingsInputGate>) -> bool {
    !gate.window_open
}

pub(crate) fn nera_input_blocked(gate: Res<SettingsInputGate>) -> bool {
    gate.window_open
}

#[derive(Resource, Default)]
struct StartupRuntime {
    enabled: bool,
}

/// One-shot gate for the primary native window.
///
/// main.rs creates Nera hidden so Windows never flashes the pet before
/// persistent settings are available. On the first update we either reveal it
/// or intentionally leave it hidden according to `start_hidden`.
#[derive(Resource, Default)]
struct LaunchVisibilityRuntime {
    applied: bool,
}

#[derive(Resource)]
struct PositionRuntime {
    restore_applied: bool,
    last_known: IVec2,
    dirty: bool,
    save_timer: Timer,
}

impl Default for PositionRuntime {
    fn default() -> Self {
        Self {
            restore_applied: false,
            last_known: IVec2::new(200, 200),
            dirty: false,
            save_timer: Timer::from_seconds(0.65, TimerMode::Once),
        }
    }
}

// =============================================================================
// Plugin
// =============================================================================

pub(crate) struct NeraSettingsPlugin;

impl Plugin for NeraSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(shadow::NeraShadowPlugin)
            .init_resource::<SettingsInputGate>()
            .init_resource::<StartupRuntime>()
            .init_resource::<LaunchVisibilityRuntime>()
            .init_resource::<PositionRuntime>()
            .add_message::<OpenSettingsWindow>()
            .add_systems(Startup, initialize_startup_state)
            .add_systems(
                Update,
                (
                    apply_launch_visibility,
                    apply_remembered_position,
                    record_primary_window_position,
                    flush_position_save,
                    open_settings_window,
                    handle_settings_buttons,
                    drag_settings_window,
                    scroll_settings_window,
                    close_settings_after_mouse_release,
                    update_settings_labels,
                    update_character_settings_labels,
                    apply_always_on_top,
                    apply_character_size,
                    apply_fps_limit,
                    cleanup_settings_window,
                    observe_settings_window_removed,
                )
                    .chain(),
            );
    }
}

// =============================================================================
// Runtime application
// =============================================================================

fn apply_launch_visibility(
    settings: Res<NeraSettings>,
    mut runtime: ResMut<LaunchVisibilityRuntime>,
    mut primary: Query<&mut Window, With<PrimaryWindow>>,
) {
    if runtime.applied {
        return;
    }

    let Ok(mut window) = primary.single_mut() else {
        return;
    };

    window.visible = !settings.start_hidden;
    runtime.applied = true;

    info!(
        "Settings: startup visibility -> {}",
        if settings.start_hidden { "HIDDEN" } else { "VISIBLE" }
    );
}

fn initialize_startup_state(mut state: ResMut<StartupRuntime>) {
    match startup::is_enabled() {
        Ok(enabled) => {
            state.enabled = enabled;
            info!("Settings: Start with Windows {}", if enabled { "ON" } else { "OFF" });
        }
        Err(error) => {
            state.enabled = false;
            warn!("Settings: failed to read Windows startup state: {error}");
        }
    }
}

fn apply_remembered_position(
    settings: Res<NeraSettings>,
    mut runtime: ResMut<PositionRuntime>,
    mut primary: Query<&mut Window, With<PrimaryWindow>>,
) {
    if runtime.restore_applied {
        return;
    }

    let Ok(mut window) = primary.single_mut() else {
        return;
    };

    let position = IVec2::new(settings.window_x, settings.window_y);
    runtime.last_known = position;

    if settings.remember_position {
        window.position = WindowPosition::At(position);
        info!("Settings: restored Nera position -> ({}, {})", position.x, position.y);
    }

    runtime.restore_applied = true;
}

fn record_primary_window_position(
    mut moved: MessageReader<WindowMoved>,
    primary: Query<Entity, With<PrimaryWindow>>,
    mut settings: ResMut<NeraSettings>,
    mut runtime: ResMut<PositionRuntime>,
) {
    let Ok(primary_entity) = primary.single() else {
        return;
    };

    for event in moved.read() {
        if event.window != primary_entity {
            continue;
        }

        runtime.last_known = event.position;

        if !settings.remember_position {
            continue;
        }

        if settings.window_x == event.position.x && settings.window_y == event.position.y {
            continue;
        }

        settings.window_x = event.position.x;
        settings.window_y = event.position.y;
        runtime.dirty = true;
        runtime.save_timer.reset();
    }
}

fn flush_position_save(
    time: Res<Time>,
    mut runtime: ResMut<PositionRuntime>,
    mut commands: Commands,
) {
    if !runtime.dirty {
        return;
    }

    runtime.save_timer.tick(time.delta());
    if !runtime.save_timer.is_finished() {
        return;
    }

    runtime.dirty = false;
    save_settings_deferred(&mut commands);
}

fn apply_always_on_top(
    settings: Res<NeraSettings>,
    mut primary: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !settings.is_changed() {
        return;
    }

    let Ok(mut window) = primary.single_mut() else {
        return;
    };

    let desired = if settings.always_on_top {
        WindowLevel::AlwaysOnTop
    } else {
        WindowLevel::Normal
    };

    if window.window_level != desired {
        window.window_level = desired;
        info!(
            "Settings: Always On Top {}",
            if settings.always_on_top { "ON" } else { "OFF" }
        );
    }
}

fn apply_character_size(
    settings: Res<NeraSettings>,
    mut avatar: Query<&mut Transform, With<crate::NeraAvatar>>,
) {
    if !settings.is_changed() {
        return;
    }

    let Ok(mut transform) = avatar.single_mut() else {
        return;
    };

    let desired = Vec3::splat(settings.character_scale());
    if (transform.scale - desired).length_squared() <= 0.000_001 {
        return;
    }

    transform.scale = desired;
    info!(
        "Settings: Character size applied -> {}",
        settings.character_size_label()
    );
}

fn apply_fps_limit(
    settings: Res<NeraSettings>,
    mut winit: ResMut<WinitSettings>,
) {
    if !settings.is_changed() {
        return;
    }

    let mode = if settings.fps_limit == 0 {
        UpdateMode::Continuous
    } else {
        let seconds = 1.0 / f64::from(settings.fps_limit.max(30));
        UpdateMode::reactive_low_power(Duration::from_secs_f64(seconds))
    };

    // A desktop pet is very often *not* the focused window. Limiting only
    // focused_mode would therefore make the setting mostly ineffective.
    winit.focused_mode = mode.clone();
    winit.unfocused_mode = mode;

    info!("Settings: FPS limit -> {}", settings.fps_label());
}

/// Shared helper for any UI / native shell surface that mutates settings.
pub(crate) fn save_settings_deferred(commands: &mut Commands) {
    commands.queue(SaveSettingsDeferred(Duration::from_millis(150)));
}

pub(crate) fn save_settings_before_exit(commands: &mut Commands) {
    commands.queue(SaveSettingsSync::IfChanged);
}

// =============================================================================
// Settings window
// =============================================================================

#[derive(Component)]
struct SettingsWindow;

#[derive(Component)]
struct SettingsOwned {
    window: Entity,
}

#[derive(Component)]
struct AlwaysOnTopValue;

#[derive(Component)]
struct StartWithWindowsValue;

#[derive(Component)]
struct StartHiddenValue;

#[derive(Component)]
struct RememberPositionValue;

#[derive(Component)]
struct CharacterSizeValue;

#[derive(Component)]
struct GroundOffsetValue;

#[derive(Component)]
struct FollowMouseValue;

#[derive(Component)]
struct FollowEyesValue;

#[derive(Component)]
struct FollowHeadValue;

#[derive(Component)]
struct FollowIntensityValue;

#[derive(Component)]
struct ShadowEnabledValue;

#[derive(Component)]
struct ShadowTypeValue;

#[derive(Component)]
struct ShadowOpacityValue;

#[derive(Component)]
struct ShadowSoftnessValue;

#[derive(Component)]
struct BrightnessValue;

#[derive(Component)]
struct FpsValue;

#[derive(Component)]
struct SettingsDragRegion;

/// Scrollable body of the Settings utility window. The title bar and footer
/// intentionally stay outside this node so they remain fixed while the cards
/// move underneath the viewport.
#[derive(Component)]
struct SettingsScrollArea;

#[derive(Component, Clone, Copy)]
enum SettingsButtonAction {
    ToggleAlwaysOnTop,
    ToggleStartWithWindows,
    ToggleStartHidden,
    ToggleRememberPosition,
    CycleCharacterSize,
    CycleGroundOffset,
    ToggleFollowMouse,
    ToggleFollowEyes,
    ToggleFollowHead,
    CycleFollowIntensity,
    ToggleShadow,
    CycleShadowType,
    CycleShadowOpacity,
    CycleShadowSoftness,
    CycleBrightness,
    CycleFps,
    CloseWindow,
    Exit,
}

fn open_settings_window(
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

    let window_entity = commands
        .spawn((
            Name::new("Nera Settings Window"),
            SettingsWindow,
            Window {
                title: "Nera Settings".into(),
                resolution: (580, 1020).into(),
                resizable: false,
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
            SettingsOwned { window: window_entity },
            Camera2d,
            Camera {
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Window(WindowRef::Entity(window_entity)),
        ))
        .id();

    spawn_settings_ui(&mut commands, window_entity, camera_entity, &settings, &startup_state);

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

                        settings_card(scroll, "APP", |card| {
                            card.spawn((
                                Button,
                                SettingsButtonAction::Exit,
                                Node {
                                    width: percent(100),
                                    height: px(46),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    border_radius: BorderRadius::all(px(13)),
                                    ..default()
                                },
                                BackgroundColor(button_color(
                                    SettingsButtonAction::Exit,
                                    Interaction::None,
                                )),
                                Outline::new(
                                    px(1),
                                    Val::ZERO,
                                    Color::srgba(0.58, 0.18, 0.25, 0.75),
                                ),
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("Exit Nera"),
                                    TextFont {
                                        font_size: FontSize::Px(16.0),
                                        ..default()
                                    },
                                    TextColor(Color::srgb(1.0, 0.84, 0.87)),
                                ));
                            });
                        });

                    });

                panel.spawn((
                    Text::new("Project Nera  ·  Mouse wheel / touchpad to scroll"),
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
            column_gap: px(12),
            ..default()
        })
        .with_children(|header| {
            // Only the title strip starts a native drag move. Keeping the close
            // button outside this region prevents click-vs-drag ambiguity.
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
                    Outline::new(
                        px(1),
                        Val::ZERO,
                        Color::srgba(0.24, 0.29, 0.43, 0.9),
                    ),
                ))
                .with_children(|close| {
                    close.spawn((
                        Text::new("×"),
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
                padding: UiRect::all(px(12)),
                flex_direction: FlexDirection::Column,
                row_gap: px(5),
                border_radius: BorderRadius::all(px(18)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.055, 0.069, 0.118, 0.97)),
            Outline::new(
                px(1),
                Val::ZERO,
                Color::srgba(0.12, 0.16, 0.28, 0.9),
            ),
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
                Outline::new(
                    px(1),
                    Val::ZERO,
                    Color::srgba(0.18, 0.23, 0.38, 0.9),
                ),
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

fn button_color(action: SettingsButtonAction, interaction: Interaction) -> Color {
    match action {
        SettingsButtonAction::Exit => match interaction {
            Interaction::Pressed => Color::srgb(0.44, 0.10, 0.16),
            Interaction::Hovered => Color::srgb(0.35, 0.095, 0.14),
            Interaction::None => Color::srgb(0.24, 0.075, 0.11),
        },
        SettingsButtonAction::CloseWindow => match interaction {
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
        | SettingsButtonAction::ToggleFollowMouse
        | SettingsButtonAction::ToggleFollowEyes
        | SettingsButtonAction::ToggleFollowHead
        | SettingsButtonAction::CycleFollowIntensity
        | SettingsButtonAction::ToggleShadow
        | SettingsButtonAction::CycleShadowType
        | SettingsButtonAction::CycleShadowOpacity
        | SettingsButtonAction::CycleShadowSoftness
        | SettingsButtonAction::CycleBrightness
        | SettingsButtonAction::CycleFps => {
            match interaction {
                Interaction::Pressed => Color::srgb(0.18, 0.22, 0.36),
                Interaction::Hovered => Color::srgb(0.135, 0.17, 0.29),
                Interaction::None => Color::srgb(0.095, 0.12, 0.21),
            }
        }
    }
}

fn handle_settings_buttons(
    mut interactions: Query<
        (&Interaction, &SettingsButtonAction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>),
    >,
    mut settings: ResMut<NeraSettings>,
    mut startup_state: ResMut<StartupRuntime>,
    position_runtime: Res<PositionRuntime>,
    mut input_gate: ResMut<SettingsInputGate>,
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
                            warn!("Settings: Start with Windows points to the current debug executable; disable it before deleting target/debug");
                        }
                        info!("Settings: Start with Windows {}", if desired { "ON" } else { "OFF" });
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
                info!("Settings: Remember position {}", if settings.remember_position { "ON" } else { "OFF" });
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
            SettingsButtonAction::ToggleFollowMouse => {
                settings.follow_mouse = !settings.follow_mouse;
                save_settings_deferred(&mut commands);
                info!("Settings: Follow mouse {}", if settings.follow_mouse { "ON" } else { "OFF" });
            }
            SettingsButtonAction::ToggleFollowEyes => {
                settings.follow_eyes = !settings.follow_eyes;
                if !settings.follow_eyes {
                    // BodyTracking extends LookAt, so Head cannot stay enabled
                    // after its LookAt/eyes dependency is disabled.
                    settings.follow_head = false;
                }
                save_settings_deferred(&mut commands);
                info!("Settings: Eyes {}", if settings.follow_eyes { "ON" } else { "OFF" });
            }
            SettingsButtonAction::ToggleFollowHead => {
                settings.follow_head = !settings.follow_head;
                if settings.follow_head {
                    // Head follow is implemented through BodyTracking + LookAt.
                    settings.follow_eyes = true;
                }
                save_settings_deferred(&mut commands);
                info!("Settings: Head follow {}", if settings.follow_head { "ON" } else { "OFF" });
            }
            SettingsButtonAction::CycleFollowIntensity => {
                settings.cycle_follow_intensity();
                save_settings_deferred(&mut commands);
                info!("Settings: Follow intensity -> {}", settings.follow_intensity_label());
            }
            SettingsButtonAction::ToggleShadow => {
                settings.shadow_enabled = !settings.shadow_enabled;
                save_settings_deferred(&mut commands);
                info!("Settings: Shadow {}", if settings.shadow_enabled { "ON" } else { "OFF" });
            }
            SettingsButtonAction::CycleShadowType => {
                settings.cycle_shadow_type();
                save_settings_deferred(&mut commands);
                info!("Settings: Shadow type -> {}", settings.shadow_type_label());
            }
            SettingsButtonAction::CycleShadowOpacity => {
                settings.cycle_shadow_opacity();
                save_settings_deferred(&mut commands);
                info!("Settings: Shadow opacity -> {}", settings.shadow_opacity_label());
            }
            SettingsButtonAction::CycleShadowSoftness => {
                settings.cycle_shadow_softness();
                save_settings_deferred(&mut commands);
                info!("Settings: Shadow softness -> {}", settings.shadow_softness_label());
            }
            SettingsButtonAction::CycleBrightness => {
                settings.cycle_brightness();
                save_settings_deferred(&mut commands);
                info!("Settings: Brightness -> {}", settings.brightness_label());
            }
            SettingsButtonAction::CycleFps => {
                settings.cycle_fps_limit();
                save_settings_deferred(&mut commands);
            }
            SettingsButtonAction::CloseWindow => {
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

fn drag_settings_window(
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

fn scroll_settings_window(
    mut wheel: MessageReader<MouseWheel>,
    settings_windows: Query<Entity, With<SettingsWindow>>,
    mut scroll_areas: Query<
        (&mut ScrollPosition, &ComputedNode),
        With<SettingsScrollArea>,
    >,
) {
    let Ok(settings_window) = settings_windows.single() else {
        return;
    };

    let Ok((mut scroll_position, computed)) = scroll_areas.single_mut() else {
        return;
    };

    // Computed sizes are physical pixels; convert the scroll range back to the
    // logical-pixel coordinate space used by ScrollPosition.
    let max_y = ((computed.content_size().y - computed.size().y)
        * computed.inverse_scale_factor())
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

        scroll_position.0.y =
            (scroll_position.0.y - delta_y).clamp(0.0, max_y);
    }
}

fn update_settings_labels(
    settings: Res<NeraSettings>,
    startup_state: Res<StartupRuntime>,
    mut labels: Query<(
        &mut Text,
        Option<&AlwaysOnTopValue>,
        Option<&StartWithWindowsValue>,
        Option<&StartHiddenValue>,
        Option<&RememberPositionValue>,
        Option<&FollowMouseValue>,
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
        follow_mouse,
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
            text.0 = if settings.remember_position { "ON" } else { "OFF" }.into();
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
}

fn update_character_settings_labels(
    settings: Res<NeraSettings>,
    mut size_labels: Query<&mut Text, With<CharacterSizeValue>>,
    mut ground_labels: Query<
        &mut Text,
        (With<GroundOffsetValue>, Without<CharacterSizeValue>),
    >,
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

fn cleanup_settings_window(
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
fn close_settings_after_mouse_release(
    mouse: Res<ButtonInput<MouseButton>>,
    settings_windows: Query<Entity, With<SettingsWindow>>,
    mut gate: ResMut<SettingsInputGate>,
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

    gate.window_open = false;
    gate.close_requested = false;
    info!("Nera Settings -> closed after mouse release");
}

/// Covers non-button closure paths such as the OS destroying the secondary
/// window. There is no post-despawn release quarantine: the normal X path
/// already waits for release before despawning.
fn observe_settings_window_removed(
    mut removed: RemovedComponents<SettingsWindow>,
    mut gate: ResMut<SettingsInputGate>,
) {
    if removed.read().next().is_none() {
        return;
    }

    gate.window_open = false;
    gate.close_requested = false;
}
