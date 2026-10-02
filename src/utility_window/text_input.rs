//! Shared styling for native Bevy text editing. Bevy owns the buffer, caret,
//! grapheme navigation and selection; utility owners persist committed values.

use bevy::{
    input_focus::InputFocus,
    prelude::*,
    text::{EditableText, EditableTextFilter, TextCursorStyle},
};

pub(crate) fn single_line(editor: EditableText, font_size: f32, color: Color) -> impl Bundle {
    (
        editor,
        EditableTextFilter::new(|character| !character.is_control()),
        TextCursorStyle {
            color,
            selection_color: Color::srgba(0.12, 0.39, 0.94, 0.75),
            unfocused_selection_color: Color::NONE,
            selected_text_color: Some(color),
        },
        TextLayout::no_wrap(),
        TextFont {
            font_size: FontSize::Px(font_size),
            ..default()
        },
        TextColor(color),
        Node {
            flex_grow: 1.0,
            min_width: px(0),
            height: percent(100),
            padding: UiRect::vertical(px(12)),
            overflow: Overflow::clip_x(),
            ..default()
        },
    )
}

/// Raw ButtonInput shortcuts bypass focused-event propagation. Their owners
/// use this gate so typing cannot invoke an unrelated desktop action.
pub(crate) fn keyboard_shortcuts_available(
    focus: Option<Res<InputFocus>>,
    editors: Query<(), With<EditableText>>,
) -> bool {
    !focus
        .as_deref()
        .and_then(InputFocus::get)
        .is_some_and(|entity| editors.contains(entity))
}
