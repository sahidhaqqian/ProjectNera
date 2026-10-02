//! Exercise the actual native Bevy input observer/Unicode editor and the
//! gallery's ownership bridge without starting the renderer or Win32 runner.

use super::*;
use bevy::{
    input::InputPlugin,
    input_focus::{FocusCause, InputDispatchPlugin, InputFocusPlugin},
    text::{FontCx, LayoutCx, RemSize, TextEdit, apply_text_edits},
    ui_widgets::EditableTextInputPlugin,
    window::Ime,
};
use bevy_clipboard::Clipboard;

fn editing_app(field: TextFocus, value: &str) -> (App, Entity, Entity) {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        InputPlugin,
        InputFocusPlugin,
        InputDispatchPlugin,
        EditableTextInputPlugin,
    ))
    .init_resource::<FontCx>()
    .init_resource::<LayoutCx>()
    .init_resource::<Clipboard>()
    .init_resource::<Assets<bevy::text::Font>>()
    .init_resource::<RemSize>()
    .init_resource::<UiScale>()
    .add_message::<Ime>()
    .add_message::<bevy::picking::events::Pointer<bevy::picking::events::Release>>()
    .insert_resource(ManagerState::for_test())
    .add_systems(
        PreUpdate,
        clear_unfocused_text_focus.before(bevy::input_focus::InputFocusSystems::Dispatch),
    )
    .add_systems(
        PostUpdate,
        (
            bevy::ui::widget::update_editable_text_styles,
            apply_text_edits,
        )
            .chain(),
    )
    .add_systems(
        PostUpdate,
        (sync_text_fields, handle_keyboard)
            .chain()
            .after(apply_text_edits),
    );
    // Use the bundled font so these keyboard/layout tests never depend on
    // which Windows system fonts happen to be installed on the test host.
    let font = bevy::text::Font::from_bytes(bevy::text::DEFAULT_FONT_DATA.to_vec());
    let family = {
        let mut context = app.world_mut().resource_mut::<FontCx>();
        let registered = context.collection.register_fonts(font.data, None);
        context
            .collection
            .family_name(registered[0].0)
            .unwrap()
            .to_owned()
    };
    let window = app
        .world_mut()
        .spawn((ManagerWindow, PrimaryWindow, Window::default()))
        .id();
    let editor = app
        .world_mut()
        .spawn((
            ManagerTextField(field),
            crate::utility_window::text_input::single_line(
                EditableText::new(value),
                14.0,
                Color::WHITE,
            ),
        ))
        .id();
    app.world_mut().get_mut::<TextFont>(editor).unwrap().font =
        bevy::text::FontSource::Family(family.into());
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(editor, FocusCause::Navigated);
    app.update();
    (app, window, editor)
}

fn key(app: &mut App, window: Entity, logical_key: Key, text: Option<&str>) {
    let event = KeyboardInput {
        key_code: KeyCode::Unidentified(bevy::input::keyboard::NativeKeyCode::Unidentified),
        logical_key,
        state: ButtonState::Pressed,
        text: text.map(Into::into),
        repeat: false,
        window,
    };
    app.world_mut().write_message(event.clone());
    app.update();
    app.world_mut().write_message(KeyboardInput {
        state: ButtonState::Released,
        ..event
    });
    app.update();
}

fn value(app: &App, field: TextFocus) -> &str {
    let state = app.world().resource::<ManagerState>();
    match field {
        TextFocus::Search => &state.search,
        TextFocus::Rename => &state.rename_value,
        TextFocus::CombineName => &state.combine_name,
        TextFocus::None => unreachable!(),
    }
}

#[test]
fn every_gallery_text_field_corrects_middle_typos_with_native_navigation() {
    for field in [TextFocus::Search, TextFocus::Rename, TextFocus::CombineName] {
        let (mut app, window, _) = editing_app(field, "sytting");
        key(&mut app, window, Key::End, None);
        for _ in 0..5 {
            key(&mut app, window, Key::ArrowLeft, None);
        }
        key(&mut app, window, Key::Backspace, None);
        key(&mut app, window, Key::Character("i".into()), Some("i"));
        assert_eq!(value(&app, field), "sitting");

        key(&mut app, window, Key::Home, None);
        key(&mut app, window, Key::ArrowRight, None);
        key(&mut app, window, Key::Delete, None);
        key(&mut app, window, Key::Character("i".into()), Some("i"));
        assert_eq!(value(&app, field), "sitting");
    }
}

#[test]
fn native_editor_deletes_unicode_characters_and_replaces_selection() {
    let (mut app, window, editor) = editing_app(TextFocus::Search, "aéñz");
    key(&mut app, window, Key::Home, None);
    key(&mut app, window, Key::ArrowRight, None);
    key(&mut app, window, Key::Delete, None);
    assert_eq!(value(&app, TextFocus::Search), "añz");
    key(&mut app, window, Key::End, None);
    key(&mut app, window, Key::ArrowLeft, None);
    key(&mut app, window, Key::Backspace, None);
    assert_eq!(value(&app, TextFocus::Search), "az");

    app.world_mut()
        .get_mut::<EditableText>(editor)
        .unwrap()
        .queue_edit(TextEdit::SelectAll);
    app.update();
    key(
        &mut app,
        window,
        Key::Character("nera".into()),
        Some("nera"),
    );
    assert_eq!(value(&app, TextFocus::Search), "nera");
}

#[test]
fn native_pointer_click_places_caret_and_drag_selects_without_rebuilding() {
    use bevy::{
        camera::RenderTarget,
        picking::{
            backend::HitData,
            events::{Drag, Pointer, Press},
            pointer::{Location, PointerButton, PointerId},
        },
    };
    let (mut app, window, editor) = editing_app(TextFocus::Search, "nera");
    let target = RenderTarget::Window(WindowRef::Entity(window))
        .normalize(Some(window))
        .unwrap();
    app.world_mut().resource_mut::<ManagerState>().dirty = false;
    app.world_mut().trigger(Pointer::new(
        PointerId::Mouse,
        Location {
            target: target.clone(),
            position: Vec2::ZERO,
        },
        Press {
            button: PointerButton::Primary,
            hit: HitData::new(window, 0.0, None, None),
            count: 1,
        },
        editor,
    ));
    app.update();
    assert!(!app.world().resource::<ManagerState>().dirty);
    key(&mut app, window, Key::Character("X".into()), Some("X"));
    assert_eq!(value(&app, TextFocus::Search), "Xnera");

    app.world_mut().trigger(Pointer::new(
        PointerId::Mouse,
        Location {
            target: target.clone(),
            position: Vec2::ZERO,
        },
        Press {
            button: PointerButton::Primary,
            hit: HitData::new(window, 0.0, None, None),
            count: 1,
        },
        editor,
    ));
    app.update();
    let drag = Vec2::new(10_000.0, 0.0);
    app.world_mut().trigger(Pointer::new(
        PointerId::Mouse,
        Location {
            target,
            position: drag,
        },
        Drag {
            button: PointerButton::Primary,
            distance: drag,
            delta: drag,
        },
        editor,
    ));
    app.update();
    key(
        &mut app,
        window,
        Key::Character("love".into()),
        Some("love"),
    );
    assert_eq!(value(&app, TextFocus::Search), "love");
}

#[test]
fn gallery_rebuild_transfers_cursor_and_selection_instead_of_resetting_to_end() {
    let (mut app, window, editor) = editing_app(TextFocus::Search, "sytting");
    key(&mut app, window, Key::Home, None);
    key(&mut app, window, Key::ArrowRight, None);
    app.add_systems(Update, capture_text_editor);
    app.world_mut().resource_mut::<ManagerState>().dirty = true;
    app.update();
    let snapshot = app
        .world()
        .resource::<ManagerState>()
        .editor_snapshot
        .as_ref()
        .unwrap()
        .1
        .clone();
    app.world_mut().despawn(editor);
    let replacement = app
        .world_mut()
        .spawn((
            ManagerTextField(TextFocus::Search),
            crate::utility_window::text_input::single_line(snapshot, 14.0, Color::WHITE),
        ))
        .id();
    app.world_mut()
        .resource_mut::<InputFocus>()
        .set(replacement, FocusCause::Navigated);
    key(&mut app, window, Key::Delete, None);
    key(&mut app, window, Key::Character("i".into()), Some("i"));
    assert_eq!(value(&app, TextFocus::Search), "sitting");
}

#[test]
fn escape_and_window_blur_release_text_keyboard_ownership() {
    let (mut app, window, _) = editing_app(TextFocus::Search, "nera");
    key(&mut app, window, Key::Escape, None);
    assert!(app.world().resource::<InputFocus>().get().is_none());
    assert!(app.world().resource::<ManagerState>().focus == TextFocus::None);

    let (mut app, window, editor) = editing_app(TextFocus::Search, "nera");
    assert_eq!(app.world().resource::<InputFocus>().get(), Some(editor));
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    key(&mut app, window, Key::Character("x".into()), Some("x"));
    assert_eq!(value(&app, TextFocus::Search), "nera");
    assert!(app.world().resource::<InputFocus>().get().is_none());
}

#[test]
fn home_and_end_inside_text_do_not_scroll_the_gallery() {
    let (mut app, window, _) = editing_app(TextFocus::Search, "nera");
    let scroll = app
        .world_mut()
        .spawn((
            GalleryScroll,
            ScrollPosition(Vec2::new(0.0, 80.0)),
            ComputedNode::default(),
        ))
        .id();
    app.add_systems(Update, handle_scroll_keys);
    key(&mut app, window, Key::Home, None);
    key(&mut app, window, Key::End, None);
    assert_eq!(app.world().get::<ScrollPosition>(scroll).unwrap().0.y, 80.0);
}

#[test]
fn combining_releases_native_focus_and_keeps_the_name_locked() {
    let (mut app, window, _) = editing_app(TextFocus::CombineName, "nera");
    app.world_mut().resource_mut::<ManagerState>().combine_job = Some(Arc::new(Mutex::new(None)));
    key(&mut app, window, Key::Character("x".into()), Some("x"));
    assert_eq!(value(&app, TextFocus::CombineName), "nera");
    assert!(app.world().resource::<InputFocus>().get().is_none());
}

#[test]
fn closing_the_manager_releases_native_editor_and_transient_snapshot() {
    let (mut app, window, editor) = editing_app(TextFocus::Search, "nera");
    let root = app.world_mut().spawn(ManagerOwned).id();
    app.world_mut().entity_mut(root).add_child(editor);
    app.init_resource::<AnimationManagerInputGate>()
        .init_resource::<preview::PreviewRuntime>()
        .init_resource::<Assets<Image>>()
        .add_systems(Update, cleanup_closed_window);
    app.world_mut()
        .resource_mut::<AnimationManagerInputGate>()
        .window_open = true;
    app.world_mut().despawn(window);
    app.update();
    assert!(app.world().get_entity(editor).is_err());
    assert!(app.world().resource::<InputFocus>().get().is_none());
    assert!(
        app.world()
            .resource::<ManagerState>()
            .editor_snapshot
            .is_none()
    );
    assert!(app.world().resource::<ManagerState>().focus == TextFocus::None);
}
