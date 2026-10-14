//! Animation gallery utility window. Preview instances never carry `NeraAvatar`
//! and never enter the production animation state resolver.
mod base;
mod characters;
mod combine;
#[cfg(test)]
mod combine_loader_tests;
mod generation;
mod idle_cache;
#[cfg(debug_assertions)]
mod memory_audit;
mod preview;
pub(crate) mod process;
mod registry;
#[cfg(test)]
mod text_input_tests;
pub(crate) mod thumbnails;
mod ui;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::utility_window::UtilityWindow;
use bevy::{
    camera::RenderTarget,
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
        mouse::{MouseScrollUnit, MouseWheel},
    },
    input_focus::InputFocus,
    prelude::*,
    text::EditableText,
    window::{
        PrimaryWindow, WindowCloseRequested, WindowLevel, WindowPosition, WindowRef,
        WindowResizeConstraints,
    },
};
use registry::AnimationRegistry;

#[derive(Message, Default)]
pub(crate) struct OpenAnimationManager;
#[derive(Message, Default)]
pub(crate) struct OpenCharacterManager;

#[derive(Resource, Default)]
pub(crate) struct AnimationManagerInputGate {
    pub window_open: bool,
    /// ButtonInput is application-global, so block Nera only while the pointer
    /// is physically inside the manager window. Once the cursor leaves the
    /// manager, the exposed desktop pet can become hittable and draggable even
    /// while the utility window stays open.
    pub pointer_over_window: bool,
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
struct CombineListScroll;
#[derive(Component)]
struct GalleryScrollbarThumb;
#[derive(Component)]
struct GalleryScrollbarTrack;
#[derive(Resource, Default)]
struct ScrollbarDrag(Option<f32>);
#[derive(Component)]
struct ManagerTextField(TextFocus);
#[derive(Component, Clone)]
struct PreviewSlot {
    key: String,
    index: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GalleryTab {
    Base,
    All,
    Bookmarks,
    Dance,
    Expressions,
    Characters,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextFocus {
    None,
    Search,
    Rename,
    CombineName,
}
#[derive(Clone)]
enum Dialog {
    Rename(String),
    Delete(String),
    RemoveCharacter(String),
    Combine,
}

struct CombinePart {
    key: String,
    duration: f32,
    start: f32,
    end: f32,
}
type CombineResult = Arc<Mutex<Option<Result<PathBuf, String>>>>;

#[derive(Component, Clone)]
enum ManagerAction {
    Close,
    Back,
    Base,
    All,
    Bookmarks,
    Dance,
    ToggleDance(String),
    Expressions,
    CharacterSlot(usize),
    AssignCharacter(String),
    UseCharacter,
    ClearCharacter(usize),
    AddCharacterSlot,
    ImportCharacter,
    RemoveCharacter(String),
    ConfirmRemoveCharacter,
    Expression(crate::expression::NeraExpressionPreset),
    Intensity(u8),
    Import,
    Reset,
    OpenMenu(String),
    Rename(String),
    Delete(String),
    Bookmark(String),
    ConfirmRename,
    ConfirmDelete,
    CancelDialog,
    Restart(String),
    AddCombine(String),
    OpenCombine,
    RemoveCombine(usize),
    MoveCombine(usize, bool),
    TrimCombine(usize, bool, bool),
    ConfirmCombine,
}

#[derive(Resource)]
struct ManagerState {
    characters: crate::character::roster::CharacterRoster,
    character_slot: usize,
    selected_character: String,
    character_scroll_y: f32,
    character_path: String,
    registry: AnimationRegistry,
    tab: GalleryTab,
    expression: crate::expression::NeraExpressionPreset,
    intensity: u8,
    search: String,
    focus: TextFocus,
    /// Cursor/selection are owned by Bevy's editor. This transient snapshot only
    /// transfers them when the existing gallery rebuild replaces the UI entity.
    editor_snapshot: Option<(TextFocus, EditableText)>,
    menu_key: Option<String>,
    dialog: Option<Dialog>,
    rename_value: String,
    combine_parts: Vec<CombinePart>,
    combine_name: String,
    combine_scroll_y: f32,
    combine_job: Option<CombineResult>,
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
        Self::with_registry(AnimationRegistry::new(root, metadata))
    }

    fn with_registry(registry: AnimationRegistry) -> Self {
        Self {
            characters: Default::default(),
            character_slot: 0,
            selected_character: crate::character::DEFAULT_CHARACTER.into(),
            character_scroll_y: 0.0,
            character_path: std::env::var("NERA_GALLERY_CHARACTER")
                .unwrap_or_else(|_| crate::character::DEFAULT_CHARACTER.into()),
            registry,
            tab: if std::env::var_os("NERA_GALLERY_CHARACTERS").is_some()
                || std::env::args().any(|arg| {
                    matches!(arg.as_str(), "--roster-smoke" | "--manager-memory-audit")
                        || arg.starts_with("--generate-character-preview=")
                }) {
                GalleryTab::Characters
            } else {
                GalleryTab::All
            },
            expression: crate::expression::NeraExpressionPreset::Default,
            intensity: 100,
            search: String::new(),
            focus: TextFocus::None,
            editor_snapshot: None,
            menu_key: None,
            dialog: None,
            rename_value: String::new(),
            combine_parts: Vec::new(),
            combine_name: String::new(),
            combine_scroll_y: 0.0,
            combine_job: None,
            error: None,
            dirty: true,
            scroll_y: 0.0,
            close_requested: false,
            return_to_menu: false,
        }
    }

    #[cfg(test)]
    fn for_test() -> Self {
        let root = std::env::temp_dir().join(format!("nera-manager-empty-{}", std::process::id()));
        Self::with_registry(AnimationRegistry::new(
            root.join("assets"),
            root.join("bookmarks.txt"),
        ))
    }

    fn filtered(&self) -> Vec<&registry::AnimationEntry> {
        let needle = self.search.to_lowercase();
        self.registry
            .entries
            .iter()
            .filter(|entry| {
                (match self.tab {
                    GalleryTab::Base => base::is_base_animation(&entry.key),
                    GalleryTab::All => !self.registry.bookmarks.contains(&entry.key),
                    GalleryTab::Bookmarks => self.registry.bookmarks.contains(&entry.key),
                    GalleryTab::Dance => self.registry.dance_pool.contains(&entry.key),
                    GalleryTab::Expressions | GalleryTab::Characters => false,
                }) && (needle.is_empty()
                    || entry.display_name.to_lowercase().contains(&needle)
                    || entry.key.to_lowercase().contains(&needle)
                    || base::base_label(&entry.key)
                        .is_some_and(|label| label.to_lowercase().contains(&needle)))
            })
            .collect()
    }
}

pub(crate) struct AnimationManagerPlugin;
impl Plugin for AnimationManagerPlugin {
    fn build(&self, app: &mut App) {
        crate::expression::face::install(app);
        app.insert_resource(ManagerState::new())
            .init_resource::<AnimationManagerInputGate>()
            .init_resource::<InputFocus>()
            .init_resource::<ScrollbarDrag>()
            .init_resource::<preview::PreviewRuntime>()
            .add_message::<OpenAnimationManager>()
            .add_message::<crate::expression::ExpressionPreviewRequest>()
            .add_message::<crate::character::roster::RosterRequest>()
            .add_systems(Last, process::forward_dance_pool)
            .add_systems(PreUpdate, sync_manager_pointer_gate)
            .add_systems(
                PreUpdate,
                clear_unfocused_text_focus.before(bevy::input_focus::InputFocusSystems::Dispatch),
            )
            .add_systems(
                Update,
                (
                    open_window,
                    handle_native_close,
                    (sync_text_fields, handle_scroll_keys).chain(),
                    handle_actions,
                    poll_combine_result,
                    capture_text_editor,
                    ui::rebuild_ui,
                    ui::sync_text_field_style,
                    drag_window,
                    scroll_gallery,
                    scroll_combine_list,
                    drag_gallery_scrollbar,
                    sync_gallery_scrollbar,
                    ui::sync_gallery_rows,
                    ui::sync_gallery_count,
                    preview::sync_visible_previews,
                    (preview::prepare_preview_rigs, preview::admit_preview_clips).chain(),
                    (
                        preview::start_ready_previews,
                        preview::sync_expression_previews,
                    )
                        .chain(),
                    close_after_release,
                    cleanup_closed_window,
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                preview::frame_expression_heads.after(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(
                PostUpdate,
                (sync_text_fields, handle_keyboard)
                    .chain()
                    .after(bevy::text::apply_text_edits),
            )
            .add_systems(Last, preview::retire_model_previews);
    }
}

fn open_window(
    mut open: MessageReader<OpenAnimationManager>,
    mut state: ResMut<ManagerState>,
    mut gate: ResMut<AnimationManagerInputGate>,
    existing: Query<&Window, With<ManagerWindow>>,
    primary: Query<Entity, With<PrimaryWindow>>,
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
    state.editor_snapshot = None;
    state.dirty = true;
    state.close_requested = false;
    state.return_to_menu = false;
    state.error = None;
    state.expression = crate::expression::NeraExpressionPreset::Default;
    state.intensity = 100;
    gate.window_open = true;
    // Conservatively block for the creation frame; PreUpdate switches to the
    // actual pointer-in-window state as soon as Bevy has cursor information.
    gate.pointer_over_window = true;
    let window = commands
        .spawn((
            Name::new("Nera Animation Manager Window"),
            ManagerWindow,
            UtilityWindow::new(16.0),
            manager_window_descriptor(),
        ))
        .id();
    // The gallery runs in its own worker and creates no default window. Native
    // Bevy text-input dispatch/IME need that worker's real window as primary.
    if primary.is_empty() {
        commands.entity(window).insert(PrimaryWindow);
    }
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

fn manager_window_descriptor() -> Window {
    Window {
        title: "Nera Animations".into(),
        resolution: (1120, 980).into(),
        resizable: true,
        resize_constraints: WindowResizeConstraints {
            min_width: 860.0,
            min_height: 680.0,
            // Leave the default unbounded maximum so Windows maximize can use
            // the monitor's full work area on any resolution.
            ..default()
        },
        // Keep the transparent rounded shell. Its own header buttons and edge
        // hit testing request native window operations without OS chrome.
        decorations: false,
        transparent: true,
        // Keep the manager above the pet; its owned import dialog inherits the
        // correct native z-order and can no longer appear behind this window.
        window_level: WindowLevel::AlwaysOnTop,
        position: WindowPosition::Automatic,
        skip_taskbar: false,
        ..default()
    }
}

fn sync_text_fields(
    focus: Res<InputFocus>,
    fields: Query<(Entity, &ManagerTextField, &EditableText)>,
    mut state: ResMut<ManagerState>,
) {
    for (entity, field, editor) in &fields {
        let value = editor.value().to_string();
        let target = match field.0 {
            TextFocus::Search => &mut state.search,
            TextFocus::Rename => &mut state.rename_value,
            TextFocus::CombineName => &mut state.combine_name,
            TextFocus::None => continue,
        };
        if *target != value {
            *target = value;
            if field.0 == TextFocus::Search {
                state.scroll_y = 0.0;
            }
            state.dirty = true;
        }
        if focus.get() == Some(entity) && state.focus != field.0 {
            state.focus = field.0;
        }
    }
}

fn capture_text_editor(
    fields: Query<(&ManagerTextField, &EditableText)>,
    mut state: ResMut<ManagerState>,
) {
    if !state.dirty {
        return;
    }
    for (field, editor) in &fields {
        if state.focus == field.0 {
            let value = match field.0 {
                TextFocus::Search => &state.search,
                TextFocus::Rename => &state.rename_value,
                TextFocus::CombineName => &state.combine_name,
                TextFocus::None => continue,
            };
            // Programmatic replacement (new rename/import/combine) must start
            // a fresh editor instead of restoring a previous field's buffer.
            if editor.value().to_string() == *value {
                state.editor_snapshot = Some((field.0, editor.clone()));
            }
        }
    }
}

fn clear_unfocused_text_focus(
    windows: Query<&Window, With<ManagerWindow>>,
    fields: Query<(), With<ManagerTextField>>,
    mut focus: ResMut<InputFocus>,
    mut state: ResMut<ManagerState>,
) {
    if focus.get().is_some_and(|entity| fields.contains(entity))
        && (state.combine_job.is_some() || !windows.iter().any(|window| window.focused))
    {
        focus.clear();
        state.focus = TextFocus::None;
    }
}

fn handle_keyboard(
    mut keys: MessageReader<KeyboardInput>,
    windows: Query<Entity, With<ManagerWindow>>,
    mut state: ResMut<ManagerState>,
    mut input_focus: ResMut<InputFocus>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    if state.combine_job.is_some() {
        keys.clear();
        return;
    }
    for event in keys.read() {
        if event.window != window || event.state != ButtonState::Pressed {
            continue;
        }
        if event.logical_key == Key::Escape {
            state.focus = TextFocus::None;
            state.editor_snapshot = None;
            input_focus.clear();
            state.dialog = None;
            state.menu_key = None;
            state.dirty = true;
            continue;
        }
        let focus = state.focus;
        if focus == TextFocus::None {
            continue;
        }
        if event.logical_key == Key::Enter {
            if focus == TextFocus::Rename {
                state.apply_rename();
            } else {
                state.focus = TextFocus::None;
                state.dirty = true;
            }
            input_focus.clear();
            continue;
        }
        // EditableTextInputPlugin owns editing, grapheme-aware navigation,
        // selection and pointer placement. Do not append/pop this String here.
    }
}

fn scroll_metrics(computed: &ComputedNode) -> (f32, f32) {
    let scale = computed.inverse_scale_factor();
    let viewport = (computed.size().y * scale).max(1.0);
    let content = (computed.content_size().y * scale).max(viewport);
    ((content - viewport).max(0.0), viewport)
}

/// Keep wheel motion predictable regardless of the number of animations.
/// A typical three-line wheel notch moves roughly one card row.
fn line_scroll_step() -> f32 {
    ui::ROW_HEIGHT * 0.30
}

fn handle_scroll_keys(
    mut keys: MessageReader<KeyboardInput>,
    windows: Query<Entity, With<ManagerWindow>>,
    mut scroll: Query<(&mut ScrollPosition, &ComputedNode), With<GalleryScroll>>,
    mut state: ResMut<ManagerState>,
) {
    if state.focus != TextFocus::None || state.dialog.is_some() {
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
    fn apply_combine(&mut self) {
        if self.combine_job.is_some() {
            return;
        }
        let segments: Vec<_> = self
            .combine_parts
            .iter()
            .map(|part| combine::Segment {
                key: part.key.clone(),
                start: part.start,
                end: Some(part.end),
            })
            .collect();
        let root = self.registry.root().to_owned();
        let name = self.combine_name.clone();
        let result: CombineResult = Arc::new(Mutex::new(None));
        let worker_result = result.clone();
        match std::thread::Builder::new()
            .name("nera-combine".into())
            .spawn(move || {
                let outcome =
                    std::panic::catch_unwind(|| combine::combine(&root, &segments, &name))
                        .map_err(|_| "Combining failed unexpectedly".to_owned())
                        .and_then(|outcome| outcome.map_err(|error| error.to_string()));
                *worker_result
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(outcome);
            }) {
            Ok(_) => self.combine_job = Some(result),
            Err(error) => self.error = Some(error.to_string()),
        }
        if self.combine_job.is_some() {
            self.focus = TextFocus::None;
        }
        self.dirty = true;
    }

    fn finish_combine(&mut self, result: Result<PathBuf, String>) {
        match result {
            Ok(path) => {
                self.registry.refresh();
                self.search = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                self.tab = GalleryTab::All;
                self.scroll_y = 0.0;
                self.dialog = None;
                self.focus = TextFocus::None;
                self.combine_parts.clear();
                self.combine_name.clear();
                self.combine_scroll_y = 0.0;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        self.dirty = true;
    }

    fn apply_rename(&mut self) {
        let Some(Dialog::Rename(key)) = self.dialog.clone() else {
            return;
        };
        match self.registry.rename(&key, &self.rename_value) {
            Ok(new_key) => {
                for part in &mut self.combine_parts {
                    if part.key == key {
                        part.key = new_key.clone();
                    }
                }
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

fn poll_combine_result(mut state: ResMut<ManagerState>) {
    let result = state
        .combine_job
        .as_ref()
        .and_then(|job| job.lock().ok()?.take());
    if let Some(result) = result {
        state.combine_job = None;
        state.finish_combine(result);
    }
}

fn handle_actions(
    mut state: ResMut<ManagerState>,
    mut actions: Query<(&Interaction, &ManagerAction, &mut BackgroundColor), Changed<Interaction>>,
    mut previews: ResMut<preview::PreviewRuntime>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut animation_players: Query<&mut AnimationPlayer>,
    children: Query<&Children>,
    mut desktop_requests: MessageWriter<crate::animation::PlayLibraryAnimation>,
    mut expression_requests: MessageWriter<crate::expression::ExpressionPreviewRequest>,
    mut roster: MessageWriter<crate::character::roster::RosterRequest>,
    windows: Query<&UtilityWindow, With<ManagerWindow>>,
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
        if state.combine_job.is_some() {
            continue;
        }
        if state.error.take().is_some() {
            state.dirty = true;
        }
        match action {
            ManagerAction::CharacterSlot(slot) => {
                state.character_slot = *slot;
                if let Some(path) = state
                    .characters
                    .slots
                    .get(*slot)
                    .and_then(|slot| slot.path.clone())
                {
                    state.selected_character = path;
                }
                state.dirty = true;
            }
            ManagerAction::AssignCharacter(path) => {
                state.selected_character = path.clone();
                state.dirty = true;
            }
            ManagerAction::UseCharacter => {
                roster.write(crate::character::roster::RosterRequest::Assign {
                    slot: state.character_slot,
                    path: state.selected_character.clone(),
                });
            }
            ManagerAction::ClearCharacter(slot) => {
                roster.write(crate::character::roster::RosterRequest::Clear { slot: *slot });
            }
            ManagerAction::RemoveCharacter(path) => {
                if state
                    .characters
                    .models
                    .iter()
                    .any(|model| model.path == *path && model.removable())
                {
                    state.dialog = Some(Dialog::RemoveCharacter(path.clone()));
                    state.focus = TextFocus::None;
                    state.dirty = true;
                }
            }
            ManagerAction::ConfirmRemoveCharacter => {
                if let Some(Dialog::RemoveCharacter(path)) = state.dialog.take()
                    && state
                        .characters
                        .models
                        .iter()
                        .any(|model| model.path == path && model.removable())
                {
                    roster.write(crate::character::roster::RosterRequest::Remove { path });
                }
                state.dirty = true;
            }
            ManagerAction::AddCharacterSlot => {
                roster.write(crate::character::roster::RosterRequest::AddSlot);
            }
            ManagerAction::ImportCharacter => {
                if let Some(owner) = windows.single().ok().and_then(UtilityWindow::native_handle) {
                    match crate::platform::pick_asset_file(
                        owner,
                        "Import Character",
                        "VRM character (*.vrm)\0*.vrm\0\0",
                    ) {
                        Ok(Some(path)) => {
                            roster.write(crate::character::roster::RosterRequest::Import {
                                slot: state.character_slot,
                                path,
                            });
                        }
                        Err(error) => {
                            state.error = Some(error);
                            state.dirty = true;
                        }
                        _ => {}
                    }
                }
            }
            ManagerAction::Close => {
                state.close_requested = true;
                state.return_to_menu = state.tab == GalleryTab::Characters;
            }
            ManagerAction::Back => {
                state.close_requested = true;
                state.return_to_menu = true;
            }
            ManagerAction::All => {
                state.tab = GalleryTab::All;
                state.scroll_y = 0.0;
                state.dirty = true;
            }
            ManagerAction::Base => {
                state.tab = GalleryTab::Base;
                state.search.clear();
                state.scroll_y = 0.0;
                state.dirty = true;
            }
            ManagerAction::Bookmarks => {
                state.tab = GalleryTab::Bookmarks;
                state.scroll_y = 0.0;
                state.dirty = true;
            }
            ManagerAction::Dance => {
                state.tab = GalleryTab::Dance;
                state.search.clear();
                state.scroll_y = 0.0;
                state.dirty = true;
            }
            ManagerAction::ToggleDance(key) => {
                if let Err(error) = state.registry.toggle_dance(key) {
                    state.error = Some(error.to_string());
                }
                state.menu_key = None;
                state.dirty = true;
            }
            ManagerAction::Expressions => {
                previews.clear(&mut commands, &mut images);
                state.tab = GalleryTab::Expressions;
                state.scroll_y = 0.0;
                state.focus = TextFocus::None;
                state.dirty = true;
            }
            ManagerAction::Expression(preset) => {
                state.expression = *preset;
                expression_requests.write(
                    crate::expression::ExpressionPreviewRequest::for_selection(
                        *preset,
                        f32::from(state.intensity) / 100.0,
                    ),
                );
                state.dirty = true;
            }
            ManagerAction::Intensity(value) => {
                state.intensity = (*value).min(100);
                expression_requests.write(
                    crate::expression::ExpressionPreviewRequest::for_selection(
                        state.expression,
                        f32::from(state.intensity) / 100.0,
                    ),
                );
                state.dirty = true;
            }
            ManagerAction::Import => {
                let Some(owner) = windows.single().ok().and_then(UtilityWindow::native_handle)
                else {
                    state.error =
                        Some("Animation Manager window is not ready; try Import again.".into());
                    state.dirty = true;
                    continue;
                };
                match crate::platform::pick_asset_file(
                    owner,
                    "Import VRMA",
                    "VRMA animation (*.vrma)\0*.vrma\0\0",
                ) {
                    Ok(Some(path)) => {
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
                    Ok(None) => {}
                    Err(error) => {
                        state.error = Some(error);
                        state.dirty = true;
                    }
                }
            }
            ManagerAction::Reset => {
                previews.restart_all(&mut commands, &mut animation_players, &children);
                state.expression = crate::expression::NeraExpressionPreset::Default;
                expression_requests.write(crate::expression::ExpressionPreviewRequest::Clear);
                state.dirty = true;
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
                state.editor_snapshot = None;
                state.menu_key = None;
                state.dirty = true;
            }
            ManagerAction::Delete(key) => {
                state.dialog = Some(Dialog::Delete(key.clone()));
                state.menu_key = None;
                state.dirty = true;
            }
            ManagerAction::Bookmark(key) => {
                match state.registry.toggle_bookmark(key) {
                    Ok(new_key) => {
                        for part in &mut state.combine_parts {
                            if part.key == *key {
                                part.key = new_key.clone();
                            }
                        }
                        previews.remove(key, &mut commands, &mut images);
                        state.scroll_y = 0.0;
                    }
                    Err(error) => state.error = Some(error.to_string()),
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
                            state.combine_parts.retain(|part| part.key != key);
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
            ManagerAction::AddCombine(key) => {
                if let Some(path) = state.registry.path(key) {
                    match combine::duration(&path) {
                        Ok(duration) if duration > 0.01 => state.combine_parts.push(CombinePart {
                            key: key.clone(),
                            duration,
                            start: 0.0,
                            end: duration,
                        }),
                        Ok(_) => state.error = Some("Animation has no usable duration".into()),
                        Err(error) => state.error = Some(error.to_string()),
                    }
                }
                state.menu_key = None;
                state.dirty = true;
            }
            ManagerAction::OpenCombine => {
                state.dialog = Some(Dialog::Combine);
                state.focus = TextFocus::None;
                state.menu_key = None;
                state.dirty = true;
            }
            ManagerAction::RemoveCombine(index) => {
                if *index < state.combine_parts.len() {
                    state.combine_parts.remove(*index);
                }
                state.dirty = true;
            }
            ManagerAction::MoveCombine(index, up) => {
                let next = if *up {
                    index.saturating_sub(1)
                } else {
                    index + 1
                };
                if *index < state.combine_parts.len() && next < state.combine_parts.len() {
                    state.combine_parts.swap(*index, next);
                }
                state.dirty = true;
            }
            ManagerAction::TrimCombine(index, start, increase) => {
                if let Some(part) = state.combine_parts.get_mut(*index) {
                    let delta = if *increase { 0.25 } else { -0.25 };
                    if *start {
                        part.start = (part.start + delta).clamp(0.0, (part.end - 0.01).max(0.0));
                    } else {
                        part.end = (part.end + delta)
                            .clamp((part.start + 0.01).min(part.duration), part.duration);
                    }
                }
                state.dirty = true;
            }
            ManagerAction::ConfirmCombine => state.apply_combine(),
        }
    }
}

fn scroll_gallery(
    mut wheel: MessageReader<MouseWheel>,
    windows: Query<Entity, With<ManagerWindow>>,
    mut scroll: Query<
        (&mut ScrollPosition, &ComputedNode),
        (With<GalleryScroll>, Without<characters::CharacterRail>),
    >,
    mut rails: Query<
        (
            &bevy::ui::RelativeCursorPosition,
            &mut ScrollPosition,
            &ComputedNode,
        ),
        (With<characters::CharacterRail>, Without<GalleryScroll>),
    >,
    mut state: ResMut<ManagerState>,
) {
    if state.dialog.is_some() {
        wheel.clear();
        return;
    }
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
            MouseScrollUnit::Line => event.y * line_scroll_step(),
            MouseScrollUnit::Pixel => event.y,
        };
        if let Ok((cursor, mut rail, computed)) = rails.single_mut()
            && cursor
                .normalized
                .is_some_and(|p| (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y))
        {
            let (max, _) = scroll_metrics(computed);
            rail.0.y = (rail.0.y - delta).clamp(0.0, max);
            state.character_scroll_y = rail.0.y;
            continue;
        }
        position.0.y = (position.0.y - delta).clamp(0.0, max);
        state.scroll_y = position.0.y;
    }
    state.scroll_y = position.0.y;
}

fn scroll_combine_list(
    mut wheel: MessageReader<MouseWheel>,
    mut state: ResMut<ManagerState>,
    windows: Query<Entity, With<ManagerWindow>>,
    mut scroll: Query<(&mut ScrollPosition, &ComputedNode), With<CombineListScroll>>,
) {
    if !matches!(state.dialog, Some(Dialog::Combine)) {
        wheel.clear();
        return;
    }
    let (Ok(window), Ok((mut position, node))) = (windows.single(), scroll.single_mut()) else {
        return;
    };
    let (max, _) = scroll_metrics(node);
    position.0.y = position.0.y.clamp(0.0, max);
    for event in wheel.read().filter(|event| event.window == window) {
        let delta = match event.unit {
            MouseScrollUnit::Line => event.y * 30.0,
            MouseScrollUnit::Pixel => event.y,
        };
        position.0.y = (position.0.y - delta).clamp(0.0, max);
    }
    state.combine_scroll_y = position.0.y;
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
    if !mouse.pressed(MouseButton::Left)
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
    if !state.close_requested || mouse.pressed(MouseButton::Left) || state.combine_job.is_some() {
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

fn handle_native_close(
    mut requests: MessageReader<WindowCloseRequested>,
    windows: Query<Entity, With<ManagerWindow>>,
    mut state: ResMut<ManagerState>,
) {
    for request in requests.read() {
        if windows.contains(request.window) {
            state.close_requested = true;
            state.return_to_menu = state.tab == GalleryTab::Characters;
        }
    }
}

fn cleanup_closed_window(
    windows: Query<Entity, With<ManagerWindow>>,
    mut gate: ResMut<AnimationManagerInputGate>,
    mut previews: ResMut<preview::PreviewRuntime>,
    mut images: ResMut<Assets<Image>>,
    owned: Query<Entity, With<ManagerOwned>>,
    fields: Query<Entity, With<ManagerTextField>>,
    mut focus: ResMut<InputFocus>,
    mut state: ResMut<ManagerState>,
    mut commands: Commands,
) {
    if !windows.is_empty() || (!gate.window_open && owned.is_empty() && previews.is_empty()) {
        return;
    }
    for entity in &owned {
        commands.entity(entity).despawn();
    }
    if focus.get().is_some_and(|entity| fields.contains(entity)) {
        focus.clear();
    }
    state.focus = TextFocus::None;
    state.editor_snapshot = None;
    previews.clear(&mut commands, &mut images);
    gate.window_open = false;
    gate.pointer_over_window = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dance_tab_actions_filter_pool_and_keep_existing_preview_play_requests() {
        // Selection requires a real, contained source after path safety checks.
        let root =
            std::env::temp_dir().join(format!("nera-manager-dance-actions-{}", std::process::id()));
        std::fs::create_dir_all(root.join("assets/Imported")).unwrap();
        std::fs::write(
            root.join("assets/Imported/user-choice.vrma"),
            b"selection fixture",
        )
        .unwrap();
        let state = ManagerState::with_registry(AnimationRegistry::new(
            root.join("assets"),
            root.join("bookmarks.txt"),
        ));
        let mut app = App::new();
        app.insert_resource(state)
            .init_resource::<preview::PreviewRuntime>()
            .init_resource::<Assets<Image>>()
            .add_message::<crate::animation::PlayLibraryAnimation>()
            .add_message::<crate::expression::ExpressionPreviewRequest>()
            .add_message::<crate::character::roster::RosterRequest>()
            .add_systems(Update, handle_actions);
        let button = app
            .world_mut()
            .spawn((
                Interaction::Pressed,
                ManagerAction::ToggleDance("Imported/user-choice.vrma".into()),
                BackgroundColor::default(),
            ))
            .id();
        app.update();
        assert_eq!(
            app.world()
                .resource::<ManagerState>()
                .registry
                .entries
                .len(),
            1
        );
        let click = |app: &mut App, action: ManagerAction| {
            app.world_mut().entity_mut(button).insert(action);
            *app.world_mut().get_mut::<Interaction>(button).unwrap() = Interaction::Pressed;
            app.update();
        };
        click(&mut app, ManagerAction::Dance);
        assert!(app.world().resource::<ManagerState>().tab == GalleryTab::Dance);
        assert_eq!(app.world().resource::<ManagerState>().filtered().len(), 1);
        click(
            &mut app,
            ManagerAction::Restart("Imported/user-choice.vrma".into()),
        );
        let events: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<crate::animation::PlayLibraryAnimation>>()
            .drain()
            .collect();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].path, "animations/Imported/user-choice.vrma");
        click(
            &mut app,
            ManagerAction::ToggleDance("Imported/user-choice.vrma".into()),
        );
        assert!(app.world().resource::<ManagerState>().filtered().is_empty());
        drop(app);
        std::fs::remove_file(root.join("assets/Imported/user-choice.vrma")).unwrap();
        std::fs::remove_file(root.join("dance_pool.json")).unwrap();
        std::fs::remove_dir(root.join("assets/Imported")).unwrap();
        std::fs::remove_dir(root.join("assets")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn title_drag_accepts_delayed_ui_press_but_does_not_repeat_or_drag_after_release() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<MouseButton>>()
            .add_systems(Update, drag_window);
        let window = app
            .world_mut()
            .spawn((ManagerWindow, Window::default()))
            .id();
        let region = app
            .world_mut()
            .spawn((ManagerDragRegion, Interaction::None))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        *app.world_mut().get_mut::<Interaction>(region).unwrap() = Interaction::Pressed;
        app.update();
        assert!(
            app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .internal
                .take_move_request()
        );
        app.update();
        assert!(
            !app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .internal
                .take_move_request()
        );
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        *app.world_mut().get_mut::<Interaction>(region).unwrap() = Interaction::None;
        app.update();
        *app.world_mut().get_mut::<Interaction>(region).unwrap() = Interaction::Pressed;
        app.update();
        assert!(
            !app.world_mut()
                .get_mut::<Window>(window)
                .unwrap()
                .internal
                .take_move_request()
        );
    }

    #[test]
    fn imported_character_remove_requires_confirmation_and_cannot_remove_default() {
        use crate::character::{DEFAULT_CHARACTER, library::CharacterEntry, roster::RosterRequest};
        let mut app = action_test_app();
        let path = "characters://model-confirm-test.vrm";
        app.world_mut()
            .resource_mut::<ManagerState>()
            .characters
            .models
            .push(CharacterEntry {
                path: path.into(),
                name: "Test".into(),
                facial_targets: 0,
                ..default()
            });
        let button = app
            .world_mut()
            .spawn((
                Interaction::Pressed,
                ManagerAction::RemoveCharacter(path.into()),
                BackgroundColor::default(),
            ))
            .id();
        app.update();
        assert!(matches!(&app.world().resource::<ManagerState>().dialog,
            Some(Dialog::RemoveCharacter(value)) if value == path));
        assert!(app.world().resource::<Messages<RosterRequest>>().is_empty());
        app.world_mut().entity_mut(button).despawn();
        let confirm = app
            .world_mut()
            .spawn((
                Interaction::Pressed,
                ManagerAction::ConfirmRemoveCharacter,
                BackgroundColor::default(),
            ))
            .id();
        app.update();
        let requests: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<RosterRequest>>()
            .drain()
            .collect();
        assert_eq!(requests, vec![RosterRequest::Remove { path: path.into() }]);
        app.world_mut().entity_mut(confirm).despawn();
        app.world_mut().spawn((
            Interaction::Pressed,
            ManagerAction::RemoveCharacter(DEFAULT_CHARACTER.into()),
            BackgroundColor::default(),
        ));
        app.update();
        assert!(app.world().resource::<ManagerState>().dialog.is_none());
        assert!(app.world().resource::<Messages<RosterRequest>>().is_empty());
    }

    fn action_test_app() -> App {
        let mut app = App::new();
        app.insert_resource(ManagerState::for_test())
            .init_resource::<InputFocus>()
            .init_resource::<preview::PreviewRuntime>()
            .init_resource::<Assets<Image>>()
            .add_message::<crate::animation::PlayLibraryAnimation>()
            .add_message::<crate::expression::ExpressionPreviewRequest>()
            .add_message::<crate::character::roster::RosterRequest>()
            .add_systems(Update, handle_actions);
        app
    }

    #[test]
    fn card_selection_only_applies_after_explicit_use() {
        use crate::character::roster::RosterRequest;
        let mut app = action_test_app();
        let path = crate::character::DEFAULT_CHARACTER;
        let button = app
            .world_mut()
            .spawn((
                Interaction::Pressed,
                ManagerAction::AssignCharacter(path.into()),
                BackgroundColor::default(),
            ))
            .id();
        app.update();
        assert_eq!(
            app.world().resource::<ManagerState>().selected_character,
            path
        );
        assert!(app.world().resource::<Messages<RosterRequest>>().is_empty());
        app.world_mut().despawn(button);
        app.world_mut().spawn((
            Interaction::Pressed,
            ManagerAction::UseCharacter,
            BackgroundColor::default(),
        ));
        app.update();
        let requests: Vec<_> = app
            .world_mut()
            .resource_mut::<Messages<RosterRequest>>()
            .drain()
            .collect();
        assert_eq!(
            requests,
            vec![RosterRequest::Assign {
                slot: 0,
                path: path.into()
            }]
        );
    }

    #[test]
    fn default_and_zero_percent_actions_send_clear_preview() {
        use crate::expression::{ExpressionPreviewRequest, NeraExpressionPreset};
        for action in [
            ManagerAction::Expression(NeraExpressionPreset::Default),
            ManagerAction::Intensity(0),
        ] {
            let mut app = action_test_app();
            app.world_mut().resource_mut::<ManagerState>().expression = NeraExpressionPreset::Angry;
            app.world_mut()
                .spawn((Interaction::Pressed, action, BackgroundColor::default()));
            app.update();
            let requests: Vec<_> = app
                .world_mut()
                .resource_mut::<Messages<ExpressionPreviewRequest>>()
                .drain()
                .collect();
            assert_eq!(requests, vec![ExpressionPreviewRequest::Clear]);
        }
    }

    #[test]
    fn playing_vrma_preserves_selected_expression_and_intensity() {
        use crate::expression::{ExpressionPreviewRequest, NeraExpressionPreset};
        for preset in [
            NeraExpressionPreset::Angry,
            NeraExpressionPreset::Timid,
            NeraExpressionPreset::Exhausted,
        ] {
            let mut app = action_test_app();
            {
                let mut state = app.world_mut().resource_mut::<ManagerState>();
                state.expression = preset;
                state.intensity = 75;
            }
            app.world_mut().spawn((
                Interaction::Pressed,
                ManagerAction::Restart("animasi_basic/Sleepy.vrma".into()),
                BackgroundColor::default(),
            ));
            app.update();
            let state = app.world().resource::<ManagerState>();
            assert_eq!(state.expression, preset);
            assert_eq!(state.intensity, 75);
            assert!(
                app.world_mut()
                    .resource_mut::<Messages<ExpressionPreviewRequest>>()
                    .drain()
                    .next()
                    .is_none()
            );
            let playback: Vec<_> = app
                .world_mut()
                .resource_mut::<Messages<crate::animation::PlayLibraryAnimation>>()
                .drain()
                .collect();
            assert_eq!(playback.len(), 1);
            assert_eq!(playback[0].path, "animations/animasi_basic/Sleepy.vrma");
        }
    }

    #[test]
    fn manager_window_keeps_transparent_shell_and_allows_unbounded_maximize() {
        let window = manager_window_descriptor();
        assert!(!window.decorations);
        assert!(window.transparent);
        assert!(window.resizable);
        assert!(!window.resize_constraints.max_width.is_finite());
        assert!(!window.resize_constraints.max_height.is_finite());
        assert!(!window.skip_taskbar);
    }

    #[test]
    fn native_close_waits_until_combine_export_finishes() {
        let mut app = App::new();
        let mut state = ManagerState::for_test();
        state.combine_job = Some(Arc::new(Mutex::new(None)));
        app.insert_resource(state)
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AnimationManagerInputGate>()
            .init_resource::<preview::PreviewRuntime>()
            .init_resource::<Assets<Image>>()
            .add_message::<WindowCloseRequested>()
            .add_message::<crate::settings::ReturnToActionMenu>()
            .add_systems(Update, (handle_native_close, close_after_release).chain());
        let window = app.world_mut().spawn(ManagerWindow).id();
        app.world_mut()
            .write_message(WindowCloseRequested { window });
        app.update();
        assert!(app.world().get_entity(window).is_ok());
        assert!(app.world().resource::<ManagerState>().close_requested);
        app.world_mut().resource_mut::<ManagerState>().combine_job = None;
        app.update();
        assert!(app.world().get_entity(window).is_err());
    }

    #[test]
    fn scrollbar_click_and_drag_reach_both_ends_of_large_library() {
        let mut app = App::new();
        app.insert_resource(ManagerState::for_test())
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
        world.init_resource::<crate::interaction::DesktopPetInteraction>();
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
