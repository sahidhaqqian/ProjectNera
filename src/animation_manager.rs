//! Animation gallery utility window. Preview instances never carry `NeraAvatar`
//! and never enter the production animation state resolver.
mod preview;
pub(crate) mod process;
mod registry;
mod ui;

use std::path::PathBuf;

use bevy::{
    camera::RenderTarget,
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        mouse::{MouseScrollUnit, MouseWheel},
    },
    prelude::*,
    window::{WindowLevel, WindowPosition, WindowRef, WindowResizeConstraints},
};
use registry::AnimationRegistry;

#[derive(Message, Default)]
pub(crate) struct OpenAnimationManager;

#[derive(Resource, Default)]
pub(crate) struct AnimationManagerInputGate {
    pub window_open: bool,
    /// ButtonInput is application-global, so block Nera only while the pointer
    /// is physically inside the manager window. Once the cursor leaves the
    /// manager, the exposed desktop pet can become hittable and draggable even
    /// while the utility window stays open.
    pub pointer_over_window: bool,
}

#[derive(Resource)]
struct CaretBlink {
    timer: Timer,
    visible: bool,
}

impl Default for CaretBlink {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(0.52, TimerMode::Repeating),
            visible: true,
        }
    }
}

impl CaretBlink {
    fn reset_visible(&mut self) {
        self.visible = true;
        self.timer.reset();
    }
}

#[derive(Component)]
struct ManagerWindow;
#[derive(Component)]
struct ManagerOwned;
#[derive(Component)]
struct ManagerCamera;
#[derive(Component)]
struct ManagerRoot;
#[derive(Component)]
struct ManagerDragRegion;
#[derive(Component)]
struct GalleryScroll;
#[derive(Component)]
struct GalleryScrollbarThumb;
#[derive(Component)]
struct GalleryScrollbarTrack;
#[derive(Resource, Default)]
struct ScrollbarDrag(Option<f32>);
#[derive(Component)]
struct TextCaret;
#[derive(Component, Clone)]
struct PreviewSlot {
    key: String,
    index: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GalleryTab {
    All,
    Bookmarks,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextFocus {
    None,
    Search,
    Rename,
}
#[derive(Clone)]
enum Dialog {
    Rename(String),
    Delete(String),
}

#[derive(Component, Clone)]
enum ManagerAction {
    Close,
    Back,
    All,
    Bookmarks,
    Search,
    Import,
    Reset,
    OpenMenu(String),
    Rename(String),
    FocusRename,
    Delete(String),
    Bookmark(String),
    ConfirmRename,
    ConfirmDelete,
    CancelDialog,
    Restart(String),
}

#[derive(Resource)]
struct ManagerState {
    registry: AnimationRegistry,
    tab: GalleryTab,
    search: String,
    focus: TextFocus,
    menu_key: Option<String>,
    dialog: Option<Dialog>,
    rename_value: String,
    error: Option<String>,
    dirty: bool,
    scroll_y: f32,
    close_requested: bool,
    return_to_menu: bool,
}

impl ManagerState {
    fn new() -> Self {
        let root = PathBuf::from(crate::nera_asset_path()).join("animations");
        let metadata = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("ProjectNera/animation_bookmarks.txt");
        Self {
            registry: AnimationRegistry::new(root, metadata),
            tab: GalleryTab::All,
            search: String::new(),
            focus: TextFocus::None,
            menu_key: None,
            dialog: None,
            rename_value: String::new(),
            error: None,
            dirty: true,
            scroll_y: 0.0,
            close_requested: false,
            return_to_menu: false,
        }
    }

    fn filtered(&self) -> Vec<&registry::AnimationEntry> {
        let needle = self.search.to_lowercase();
        self.registry
            .entries
            .iter()
            .filter(|entry| {
                (self.tab == GalleryTab::All || self.registry.bookmarks.contains(&entry.key))
                    && (needle.is_empty()
                        || entry.display_name.to_lowercase().contains(&needle)
                        || entry.key.to_lowercase().contains(&needle))
            })
            .collect()
    }
}

pub(crate) struct AnimationManagerPlugin;
impl Plugin for AnimationManagerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ManagerState::new())
            .init_resource::<AnimationManagerInputGate>()
            .init_resource::<CaretBlink>()
            .init_resource::<ScrollbarDrag>()
            .init_resource::<preview::PreviewRuntime>()
            .add_message::<OpenAnimationManager>()
            .add_systems(PreUpdate, sync_manager_pointer_gate)
            .add_systems(
                Update,
                (
                    open_window,
                    handle_keyboard,
                    handle_scroll_keys,
                    handle_actions,
                    ui::rebuild_ui,
                    ui::blink_text_caret,
                    drag_window,
                    scroll_gallery,
                    drag_gallery_scrollbar,
                    sync_gallery_scrollbar,
                    ui::sync_gallery_rows,
                    ui::sync_gallery_count,
                    preview::sync_visible_previews,
                    preview::prepare_preview_rigs,
                    preview::start_ready_previews,
                    close_after_release,
                    cleanup_closed_window,
                )
                    .chain(),
            );
    }
}

fn open_window(
    mut open: MessageReader<OpenAnimationManager>,
    mut state: ResMut<ManagerState>,
    mut gate: ResMut<AnimationManagerInputGate>,
    existing: Query<&Window, With<ManagerWindow>>,
    mut commands: Commands,
) {
    if open.read().next().is_none() {
        return;
    }
    if !existing.is_empty() {
        gate.window_open = true;
        gate.pointer_over_window = existing
            .iter()
            .any(|window| window.cursor_position().is_some());
        return;
    }
    state.registry.refresh();
    // A newly opened manager should always start on a clean viewport. Keeping
    // the previous ScrollPosition made the first row reopen half-clipped.
    state.scroll_y = 0.0;
    state.menu_key = None;
    state.dialog = None;
    state.focus = TextFocus::None;
    state.dirty = true;
    state.close_requested = false;
    state.return_to_menu = false;
    state.error = None;
    gate.window_open = true;
    // Conservatively block for the creation frame; PreUpdate switches to the
    // actual pointer-in-window state as soon as Bevy has cursor information.
    gate.pointer_over_window = true;
    let window = commands
        .spawn((
            Name::new("Nera Animation Manager Window"),
            ManagerWindow,
            Window {
                title: "Nera Animations".into(),
                resolution: (1120, 980).into(),
                resizable: true,
                resize_constraints: WindowResizeConstraints {
                    min_width: 860.0,
                    min_height: 680.0,
                    max_width: 1500.0,
                    max_height: 1200.0,
                },
                decorations: false,
                transparent: true,
                window_level: WindowLevel::AlwaysOnTop,
                position: WindowPosition::Automatic,
                ..default()
            },
        ))
        .id();
    commands.spawn((
        Name::new("Nera Animation Manager Camera"),
        ManagerOwned,
        ManagerCamera,
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Window(WindowRef::Entity(window)),
    ));
}

fn handle_keyboard(
    mut keys: MessageReader<KeyboardInput>,
    windows: Query<Entity, With<ManagerWindow>>,
    mut state: ResMut<ManagerState>,
    mut caret: ResMut<CaretBlink>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    for event in keys.read() {
        if event.window != window || event.state != ButtonState::Pressed {
            continue;
        }
        if event.logical_key == Key::Escape {
            state.focus = TextFocus::None;
            state.dialog = None;
            state.menu_key = None;
            state.dirty = true;
            continue;
        }
        let focus = state.focus;
        if focus == TextFocus::None {
            continue;
        }
        caret.reset_visible();
        if event.logical_key == Key::Enter {
            if focus == TextFocus::Rename {
                state.apply_rename();
            } else {
                state.focus = TextFocus::None;
                state.dirty = true;
            }
            continue;
        }
        let target = if focus == TextFocus::Search {
            &mut state.search
        } else {
            &mut state.rename_value
        };
        if event.logical_key == Key::Backspace {
            target.pop();
        } else if let Some(text) = &event.text {
            if !text.chars().any(char::is_control) {
                target.push_str(text);
            }
        }
        if focus == TextFocus::Search {
            state.scroll_y = 0.0;
        }
        state.dirty = true;
    }
}

fn scroll_metrics(computed: &ComputedNode) -> (f32, f32) {
    let scale = computed.inverse_scale_factor();
    let viewport = (computed.size().y * scale).max(1.0);
    let content = (computed.content_size().y * scale).max(viewport);
    ((content - viewport).max(0.0), viewport)
}

/// With a library containing hundreds or thousands of VRMAs, a traditional
/// 3-line wheel step makes reaching the bottom impractical. Keep pixel/trackpad
/// scrolling precise, but scale discrete wheel notches with the library size.
fn line_scroll_step(max_scroll: f32) -> f32 {
    if max_scroll <= ui::ROW_HEIGHT * 24.0 {
        return ui::ROW_HEIGHT * 1.6;
    }

    (max_scroll * 0.08).max(ui::ROW_HEIGHT * 3.0)
}

fn handle_scroll_keys(
    mut keys: MessageReader<KeyboardInput>,
    windows: Query<Entity, With<ManagerWindow>>,
    mut scroll: Query<(&mut ScrollPosition, &ComputedNode), With<GalleryScroll>>,
    mut state: ResMut<ManagerState>,
) {
    if state.focus != TextFocus::None {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok((mut position, computed)) = scroll.single_mut() else {
        return;
    };
    let (max_scroll, viewport) = scroll_metrics(computed);

    for event in keys.read() {
        if event.window != window || event.state != ButtonState::Pressed {
            continue;
        }
        let next = match event.logical_key {
            Key::Home => Some(0.0),
            Key::End => Some(max_scroll),
            Key::PageDown => Some(position.0.y + viewport * 0.90),
            Key::PageUp => Some(position.0.y - viewport * 0.90),
            _ => None,
        };
        if let Some(next) = next {
            position.0.y = next.clamp(0.0, max_scroll);
            state.scroll_y = position.0.y;
        }
    }
}

fn sync_manager_pointer_gate(
    windows: Query<&Window, With<ManagerWindow>>,
    mut gate: ResMut<AnimationManagerInputGate>,
) {
    // Keep the lifecycle flag until cleanup runs, including native/Alt+F4 close.
    gate.pointer_over_window = windows
        .iter()
        .any(|window| window.cursor_position().is_some());
}

impl ManagerState {
    fn apply_rename(&mut self) {
        let Some(Dialog::Rename(key)) = self.dialog.clone() else {
            return;
        };
        match self.registry.rename(&key, &self.rename_value) {
            Ok(_) => {
                self.dialog = None;
                self.focus = TextFocus::None;
                self.menu_key = None;
                self.error = None;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
        self.dirty = true;
    }
}

fn handle_actions(
    mut state: ResMut<ManagerState>,
    mut caret: ResMut<CaretBlink>,
    mut actions: Query<(&Interaction, &ManagerAction, &mut BackgroundColor), Changed<Interaction>>,
    mut previews: ResMut<preview::PreviewRuntime>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut animation_players: Query<&mut AnimationPlayer>,
    children: Query<&Children>,
    mut desktop_requests: MessageWriter<crate::animation::PlayLibraryAnimation>,
) {
    for (interaction, action, mut color) in &mut actions {
        *color = BackgroundColor(match interaction {
            Interaction::Hovered => ui::BLUE_HOVER,
            Interaction::Pressed => ui::BLUE_PRESSED,
            Interaction::None => ui::button_color(action, &state),
        });
        if *interaction != Interaction::Pressed {
            continue;
        }
        state.error = None;
        match action {
            ManagerAction::Close => state.close_requested = true,
            ManagerAction::Back => {
                state.close_requested = true;
                state.return_to_menu = true;
            }
            ManagerAction::All => {
                state.tab = GalleryTab::All;
                state.scroll_y = 0.0;
                state.dirty = true;
            }
            ManagerAction::Bookmarks => {
                state.tab = GalleryTab::Bookmarks;
                state.scroll_y = 0.0;
                state.dirty = true;
            }
            ManagerAction::Search => {
                state.focus = TextFocus::Search;
                caret.reset_visible();
                state.dirty = true;
            }
            ManagerAction::Import => {
                if let Some(path) = pick_vrma_file() {
                    match state.registry.import(&path) {
                        Ok(_) => {
                            state.tab = GalleryTab::All;
                            state.search.clear();
                            state.scroll_y = 0.0;
                        }
                        Err(error) => state.error = Some(error.to_string()),
                    }
                    state.dirty = true;
                }
            }
            ManagerAction::Reset => {
                previews.restart_all(&mut commands, &mut animation_players, &children)
            }
            ManagerAction::OpenMenu(key) => {
                state.menu_key = (state.menu_key.as_ref() != Some(key)).then(|| key.clone());
                state.dirty = true;
            }
            ManagerAction::Rename(key) => {
                state.rename_value = std::path::Path::new(key)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                state.dialog = Some(Dialog::Rename(key.clone()));
                state.focus = TextFocus::Rename;
                caret.reset_visible();
                state.menu_key = None;
                state.dirty = true;
            }
            ManagerAction::FocusRename => {
                state.focus = TextFocus::Rename;
                caret.reset_visible();
                state.dirty = true;
            }
            ManagerAction::Delete(key) => {
                state.dialog = Some(Dialog::Delete(key.clone()));
                state.menu_key = None;
                state.dirty = true;
            }
            ManagerAction::Bookmark(key) => {
                if let Err(error) = state.registry.toggle_bookmark(key) {
                    state.error = Some(error.to_string());
                }
                state.menu_key = None;
                state.dirty = true;
            }
            ManagerAction::ConfirmRename => state.apply_rename(),
            ManagerAction::ConfirmDelete => {
                if let Some(Dialog::Delete(key)) = state.dialog.clone() {
                    previews.remove(&key, &mut commands, &mut images);
                    match state.registry.delete(&key) {
                        Ok(()) => {
                            state.dialog = None;
                            state.error = None;
                        }
                        Err(error) => state.error = Some(error.to_string()),
                    }
                }
                state.dirty = true;
            }
            ManagerAction::CancelDialog => {
                state.dialog = None;
                state.focus = TextFocus::None;
                state.dirty = true;
            }
            ManagerAction::Restart(key) => {
                previews.restart(key, &mut commands, &mut animation_players, &children);
                desktop_requests.write(crate::animation::PlayLibraryAnimation {
                    path: format!("animations/{key}"),
                });
            }
        }
    }
}

fn scroll_gallery(
    mut wheel: MessageReader<MouseWheel>,
    windows: Query<Entity, With<ManagerWindow>>,
    mut scroll: Query<(&mut ScrollPosition, &ComputedNode), With<GalleryScroll>>,
    mut state: ResMut<ManagerState>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok((mut position, computed)) = scroll.single_mut() else {
        return;
    };
    let (max, _) = scroll_metrics(computed);
    for event in wheel.read() {
        if event.window != window {
            continue;
        }
        let delta = match event.unit {
            MouseScrollUnit::Line => event.y * line_scroll_step(max),
            MouseScrollUnit::Pixel => event.y,
        };
        position.0.y = (position.0.y - delta).clamp(0.0, max);
        state.scroll_y = position.0.y;
    }
    state.scroll_y = position.0.y;
}

fn sync_gallery_scrollbar(
    scroll: Query<(&ScrollPosition, &ComputedNode), With<GalleryScroll>>,
    mut thumbs: Query<&mut Node, With<GalleryScrollbarThumb>>,
) {
    let Ok((position, computed)) = scroll.single() else {
        return;
    };
    let Ok(mut thumb) = thumbs.single_mut() else {
        return;
    };

    let scale = computed.inverse_scale_factor();
    let viewport_height = (computed.size().y * scale).max(1.0);
    let content_height = (computed.content_size().y * scale).max(viewport_height);
    let track_height = (viewport_height - 4.0).max(1.0);
    let thumb_height = (track_height * viewport_height / content_height)
        .clamp(28.0_f32.min(track_height), track_height);
    let max_scroll = (content_height - viewport_height).max(0.0);
    let progress = if max_scroll > 0.0 {
        (position.0.y / max_scroll).clamp(0.0, 1.0)
    } else {
        0.0
    };

    thumb.height = px(thumb_height);
    thumb.top = px(progress * (track_height - thumb_height));
}

fn drag_gallery_scrollbar(
    mouse: Res<ButtonInput<MouseButton>>,
    mut drag: ResMut<ScrollbarDrag>,
    tracks: Query<(&Interaction, &bevy::ui::RelativeCursorPosition), With<GalleryScrollbarTrack>>,
    mut scroll: Query<(&mut ScrollPosition, &ComputedNode), With<GalleryScroll>>,
    mut state: ResMut<ManagerState>,
) {
    if !mouse.pressed(MouseButton::Left) || state.dialog.is_some() {
        drag.0 = None;
        return;
    }
    let (Ok((interaction, cursor)), Ok((mut position, computed))) =
        (tracks.single(), scroll.single_mut())
    else {
        return;
    };
    let Some(cursor) = cursor.normalized else {
        return;
    };
    let (max_scroll, viewport) = scroll_metrics(computed);
    let track = (viewport - 4.0).max(1.0);
    let thumb = (track * viewport / (viewport + max_scroll)).clamp(28.0_f32.min(track), track);
    let travel = track - thumb;
    if max_scroll == 0.0 || travel <= 0.0 {
        return;
    }
    // Bevy 0.19 normalizes UI coordinates around the node's center.
    let pointer = (cursor.y + 0.5) * track;
    if mouse.just_pressed(MouseButton::Left) && *interaction == Interaction::Pressed {
        let top = position.0.y / max_scroll * travel;
        drag.0 = Some(if (top..=top + thumb).contains(&pointer) {
            pointer - top
        } else {
            thumb / 2.0
        });
        state.focus = TextFocus::None;
    }
    if let Some(offset) = drag.0 {
        position.0.y = ((pointer - offset) / travel).clamp(0.0, 1.0) * max_scroll;
        state.scroll_y = position.0.y;
    }
}

fn drag_window(
    mouse: Res<ButtonInput<MouseButton>>,
    regions: Query<&Interaction, (Changed<Interaction>, With<ManagerDragRegion>)>,
    mut windows: Query<&mut Window, With<ManagerWindow>>,
) {
    if !mouse.just_pressed(MouseButton::Left)
        || !regions
            .iter()
            .any(|interaction| *interaction == Interaction::Pressed)
    {
        return;
    }
    if let Ok(mut window) = windows.single_mut() {
        window.start_drag_move();
    }
}

fn close_after_release(
    mouse: Res<ButtonInput<MouseButton>>,
    mut state: ResMut<ManagerState>,
    mut gate: ResMut<AnimationManagerInputGate>,
    windows: Query<Entity, With<ManagerWindow>>,
    owned: Query<Entity, With<ManagerOwned>>,
    mut previews: ResMut<preview::PreviewRuntime>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
    mut return_to_menu: MessageWriter<crate::settings::ReturnToActionMenu>,
) {
    if !state.close_requested || mouse.pressed(MouseButton::Left) {
        return;
    }
    for entity in &windows {
        commands.entity(entity).despawn();
    }
    for entity in &owned {
        commands.entity(entity).despawn();
    }
    previews.clear(&mut commands, &mut images);
    if state.return_to_menu {
        return_to_menu.write(crate::settings::ReturnToActionMenu);
    }
    state.close_requested = false;
    state.return_to_menu = false;
    state.dialog = None;
    state.menu_key = None;
    state.focus = TextFocus::None;
    gate.window_open = false;
    gate.pointer_over_window = false;
}

fn cleanup_closed_window(
    windows: Query<Entity, With<ManagerWindow>>,
    mut gate: ResMut<AnimationManagerInputGate>,
    mut previews: ResMut<preview::PreviewRuntime>,
    mut images: ResMut<Assets<Image>>,
    owned: Query<Entity, With<ManagerOwned>>,
    mut commands: Commands,
) {
    if !windows.is_empty() || (!gate.window_open && owned.is_empty() && previews.is_empty()) {
        return;
    }
    for entity in &owned {
        commands.entity(entity).despawn();
    }
    previews.clear(&mut commands, &mut images);
    gate.window_open = false;
    gate.pointer_over_window = false;
}

#[cfg(target_os = "windows")]
fn pick_vrma_file() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::{
        Win32::UI::Controls::Dialogs::{
            GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_PATHMUSTEXIST, OPENFILENAMEW,
        },
        core::{PCWSTR, PWSTR},
    };
    let mut buffer = [0u16; 32768];
    let filter: Vec<u16> = "VRMA animation (*.vrma)\0*.vrma\0\0"
        .encode_utf16()
        .collect();
    let title: Vec<u16> = "Import VRMA\0".encode_utf16().collect();
    let mut dialog = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        lpstrFile: PWSTR(buffer.as_mut_ptr()),
        nMaxFile: buffer.len() as u32,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrTitle: PCWSTR(title.as_ptr()),
        Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST,
        ..Default::default()
    };
    if !unsafe { GetOpenFileNameW(&mut dialog).as_bool() } {
        return None;
    }
    let len = buffer.iter().position(|unit| *unit == 0)?;
    Some(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrollbar_click_and_drag_reach_both_ends_of_large_library() {
        let mut app = App::new();
        app.insert_resource(ManagerState::new())
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ScrollbarDrag>()
            .add_systems(Update, drag_gallery_scrollbar);
        let track = app
            .world_mut()
            .spawn((
                GalleryScrollbarTrack,
                Interaction::Pressed,
                bevy::ui::RelativeCursorPosition {
                    cursor_over: true,
                    normalized: Some(Vec2::new(0.0, 0.5)),
                },
            ))
            .id();
        let scroll = app
            .world_mut()
            .spawn((
                GalleryScroll,
                ScrollPosition::default(),
                ComputedNode {
                    size: Vec2::new(1000.0, 720.0),
                    content_size: Vec2::new(1000.0, 1_200_000.0),
                    inverse_scale_factor: 1.0,
                    ..default()
                },
            ))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert_eq!(
            app.world().get::<ScrollPosition>(scroll).unwrap().0.y,
            1_199_280.0
        );
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.world_mut()
            .get_mut::<bevy::ui::RelativeCursorPosition>(track)
            .unwrap()
            .normalized = Some(Vec2::new(0.0, -0.5));
        app.update();
        assert_eq!(app.world().get::<ScrollPosition>(scroll).unwrap().0.y, 0.0);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        app.update();
        assert!(app.world().resource::<ScrollbarDrag>().0.is_none());
    }

    #[test]
    fn open_manager_only_blocks_pet_input_while_pointer_is_over_it() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<crate::settings::SettingsInputGate>();
        world.init_resource::<crate::alarm_settings::AlarmSettingsInputGate>();
        world.insert_resource(AnimationManagerInputGate {
            window_open: true,
            pointer_over_window: false,
        });
        assert!(
            world
                .run_system_once(crate::interaction::nera_utility_input_available)
                .unwrap()
        );
        assert!(
            !world
                .run_system_once(crate::interaction::nera_utility_input_blocked)
                .unwrap()
        );
        world
            .resource_mut::<AnimationManagerInputGate>()
            .pointer_over_window = true;
        assert!(
            !world
                .run_system_once(crate::interaction::nera_utility_input_available)
                .unwrap()
        );
        assert!(
            world
                .run_system_once(crate::interaction::nera_utility_input_blocked)
                .unwrap()
        );
    }
}
