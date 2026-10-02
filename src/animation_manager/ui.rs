use crate::expression::NeraExpressionPreset;
use bevy::{
    input_focus::{FocusCause, InputFocus},
    prelude::*,
    text::EditableText,
    ui::UiTargetCamera,
};

use super::{
    CombineListScroll, Dialog, GalleryScroll, GalleryScrollbarThumb, GalleryScrollbarTrack,
    GalleryTab, ManagerAction, ManagerCamera, ManagerDragRegion, ManagerOwned, ManagerRoot,
    ManagerState, ManagerTextField, ManagerWindow, PreviewSlot, TextFocus,
};

pub(super) const ROW_HEIGHT: f32 = 360.0;
pub(super) const COLUMNS: usize = 3;

#[derive(Component)]
pub(super) struct GalleryContent(pub(super) usize);
#[derive(Component)]
pub(super) struct GalleryRow;
#[derive(Component)]
pub(super) struct GalleryCount(usize);

pub(super) fn sync_gallery_count(
    state: Res<ManagerState>,
    scroll: Query<&ComputedNode, With<GalleryScroll>>,
    mut labels: Query<(&GalleryCount, &mut Text)>,
) {
    let Ok(node) = scroll.single() else {
        return;
    };
    let height = node.size().y * node.inverse_scale_factor();
    for (count, mut text) in &mut labels {
        let first = if count.0 == 0 {
            0
        } else {
            ((state.scroll_y / ROW_HEIGHT).floor() as usize * COLUMNS + 1).min(count.0)
        };
        let last =
            (((state.scroll_y + height) / ROW_HEIGHT).ceil() as usize * COLUMNS).min(count.0);
        text.set_if_neq(Text::new(format!(
            "{first} - {last} of {} animations",
            count.0
        )));
    }
}

pub(super) fn visible_rows(scroll: f32, viewport: f32, count: usize) -> std::ops::Range<usize> {
    let total = count.div_ceil(COLUMNS);
    let first = (scroll.max(0.0) / ROW_HEIGHT).floor() as usize;
    let start = first.saturating_sub(1).min(total);
    start..(first + (viewport / ROW_HEIGHT).ceil() as usize + 2).min(total)
}

pub(super) fn sync_gallery_rows(
    mut commands: Commands,
    state: Res<ManagerState>,
    scroll: Query<(Entity, &ComputedNode), With<GalleryScroll>>,
    content: Query<(Entity, &GalleryContent)>,
    rows: Query<Entity, With<GalleryRow>>,
    mut previous: Local<Option<(Entity, std::ops::Range<usize>)>>,
) {
    let (Ok((scroll_entity, computed)), Ok((content, count))) = (scroll.single(), content.single())
    else {
        *previous = None;
        return;
    };
    let range = visible_rows(
        state.scroll_y,
        computed.size().y * computed.inverse_scale_factor(),
        count.0,
    );
    if previous.as_ref() == Some(&(scroll_entity, range.clone())) {
        return;
    }
    let entries = state.filtered();
    for row in &rows {
        commands.entity(row).despawn();
    }
    commands.entity(content).with_children(|gallery| {
        for row_number in range.clone() {
            gallery
                .spawn((
                    GalleryRow,
                    Node {
                        position_type: PositionType::Absolute,
                        top: px(row_number as f32 * ROW_HEIGHT),
                        width: percent(100),
                        height: px(ROW_HEIGHT - 14.0),
                        flex_direction: FlexDirection::Row,
                        column_gap: px(12),
                        ..default()
                    },
                ))
                .with_children(|row| {
                    for index in row_number * COLUMNS
                        ..((row_number + 1) * COLUMNS).min(if state.tab == GalleryTab::Characters {
                            state.characters.models.len()
                        } else {
                            entries.len()
                        })
                    {
                        if state.tab == GalleryTab::Characters {
                            super::characters::card(row, index, &state);
                        } else {
                            spawn_card(row, entries[index], index, &state);
                        }
                    }
                });
        }
    });
    *previous = Some((scroll_entity, range));
}
const SHELL: Color = Color::srgba(0.032, 0.042, 0.078, 0.985);
pub(super) const CARD: Color = Color::srgba(0.055, 0.071, 0.122, 0.98);
const OUTLINE: Color = Color::srgba(0.20, 0.27, 0.43, 0.85);
pub(super) const PRIMARY: Color = Color::srgb(0.95, 0.96, 1.0);
pub(super) const MUTED: Color = Color::srgb(0.60, 0.67, 0.82);
pub(super) const BLUE: Color = Color::srgb(0.12, 0.39, 0.94);
pub(super) const BLUE_HOVER: Color = Color::srgb(0.19, 0.46, 1.0);
pub(super) const BLUE_PRESSED: Color = Color::srgb(0.10, 0.32, 0.78);
pub(super) const BUTTON: Color = Color::srgb(0.095, 0.12, 0.21);

pub(super) fn button_color(action: &ManagerAction, state: &ManagerState) -> Color {
    match action {
        ManagerAction::AssignCharacter(path) => {
            if state.selected_character == *path {
                BLUE
            } else {
                CARD
            }
        }
        ManagerAction::CharacterSlot(slot) if state.character_slot == *slot => BLUE,
        ManagerAction::Import
        | ManagerAction::Reset
        | ManagerAction::ConfirmRename
        | ManagerAction::ConfirmCombine => BLUE,
        ManagerAction::Base if state.tab == GalleryTab::Base => BLUE,
        ManagerAction::All if state.tab == GalleryTab::All => BLUE,
        ManagerAction::Bookmarks if state.tab == GalleryTab::Bookmarks => BLUE,
        ManagerAction::Expressions if state.tab == GalleryTab::Expressions => BLUE,
        ManagerAction::Expression(preset) if state.expression == *preset => BLUE,
        ManagerAction::Intensity(value) if state.intensity == *value => BLUE,
        ManagerAction::ConfirmDelete | ManagerAction::ConfirmRemoveCharacter => {
            Color::srgb(0.58, 0.13, 0.18)
        }
        _ => BUTTON,
    }
}

pub(super) fn label(
    parent: &mut ChildSpawnerCommands,
    value: impl Into<String>,
    size: f32,
    color: Color,
) {
    parent.spawn((
        Text::new(value),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    ));
}

#[derive(Component)]
pub(super) struct TextFieldPlaceholder(Entity);

fn text_field(
    parent: &mut ChildSpawnerCommands,
    field: TextFocus,
    state: &ManagerState,
    input_focus: &mut InputFocus,
    size: f32,
    placeholder: &str,
) {
    let value = match field {
        TextFocus::Search => &state.search,
        TextFocus::Rename => &state.rename_value,
        TextFocus::CombineName => &state.combine_name,
        TextFocus::None => return,
    };
    let editor = state
        .editor_snapshot
        .as_ref()
        .filter(|(owner, editor)| *owner == field && editor.value().to_string() == *value)
        .map(|(_, editor)| editor.clone())
        .unwrap_or_else(|| EditableText::new(value));
    let mut field_entity = parent.spawn((
        ManagerTextField(field),
        crate::utility_window::text_input::single_line(editor, size, PRIMARY),
    ));
    if state.combine_job.is_some() {
        field_entity.insert(bevy::picking::Pickable::IGNORE);
    }
    let entity = field_entity.id();
    if state.focus == field && state.combine_job.is_none() {
        input_focus.set(entity, FocusCause::Navigated);
    }
    if !placeholder.is_empty() {
        parent.spawn((
            TextFieldPlaceholder(entity),
            bevy::picking::Pickable::IGNORE,
            Text::new(placeholder),
            TextFont {
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(MUTED),
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            if value.is_empty() && state.focus != field {
                Visibility::Visible
            } else {
                Visibility::Hidden
            },
        ));
    }
}

pub(super) fn sync_text_field_style(
    focus: Res<InputFocus>,
    fields: Query<(Entity, &EditableText, &ChildOf), With<ManagerTextField>>,
    mut outlines: Query<&mut Outline>,
    mut placeholders: Query<(&TextFieldPlaceholder, &mut Visibility)>,
) {
    for (entity, _, parent) in &fields {
        if let Ok(mut outline) = outlines.get_mut(parent.parent()) {
            outline.color = if focus.get() == Some(entity) {
                BLUE
            } else {
                OUTLINE
            };
        }
    }
    for (placeholder, mut visibility) in &mut placeholders {
        let empty = fields
            .get(placeholder.0)
            .is_ok_and(|(_, editor, _)| editor.value().to_string().is_empty());
        *visibility = if empty && focus.get() != Some(placeholder.0) {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

pub(super) fn button(
    parent: &mut ChildSpawnerCommands,
    value: &str,
    action: ManagerAction,
    width: Val,
    state: &ManagerState,
) {
    let background = button_color(&action, state);
    parent
        .spawn((
            Button,
            action,
            Node {
                width,
                height: px(44),
                flex_shrink: 0.0,
                padding: UiRect::horizontal(px(15)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(px(12)),
                ..default()
            },
            BackgroundColor(background),
            Outline::new(px(1), Val::ZERO, OUTLINE),
        ))
        .with_children(|node| label(node, value, 15.0, PRIMARY));
}

fn spawn_expression_panel(parent: &mut ChildSpawnerCommands, state: &ManagerState) {
    parent
        .spawn(Node {
            width: percent(100),
            flex_grow: 1.0,
            min_height: px(0),
            padding: UiRect::all(px(12)),
            flex_direction: FlexDirection::Column,
            row_gap: px(10),
            ..default()
        })
        .with_children(|panel| {
            label(panel, "Facial expressions", 22.0, PRIMARY);
            if state
                .characters
                .models
                .iter()
                .any(|model| model.path == state.character_path && model.facial_targets == 0)
            {
                label(
                    panel,
                    "This model has no facial blendshapes.",
                    14.0,
                    Color::srgb(1.0, 0.68, 0.36),
                );
            }
            for row in NeraExpressionPreset::ALL.chunks(4) {
                panel
                    .spawn(Node {
                        width: percent(100),
                        flex_grow: 1.0,
                        min_height: px(80),
                        max_height: px(190),
                        column_gap: px(16),
                        flex_direction: FlexDirection::Row,
                        ..default()
                    })
                    .with_children(|cards| {
                        for &preset in row {
                            let selected = state.expression == preset;
                            cards
                                .spawn((
                                    Button,
                                    ManagerAction::Expression(preset),
                                    Node {
                                        width: percent(23),
                                        height: percent(100),
                                        border_radius: BorderRadius::all(px(8)),
                                        flex_direction: FlexDirection::Column,
                                        align_items: AlignItems::Center,
                                        justify_content: JustifyContent::Center,
                                        ..default()
                                    },
                                    BackgroundColor(if selected { BLUE } else { CARD }),
                                    Outline::new(
                                        px(if selected { 2 } else { 1 }),
                                        Val::ZERO,
                                        if selected { PRIMARY } else { OUTLINE },
                                    ),
                                ))
                                .with_children(|card| {
                                    card.spawn((
                                        PreviewSlot {
                                            key: format!("face/{}", preset.wire()),
                                            index: 0,
                                        },
                                        ImageNode::default(),
                                        Node {
                                            width: percent(100),
                                            height: percent(78),
                                            min_height: px(0),
                                            ..default()
                                        },
                                    ));
                                    label(card, preset.name(), 16.0, PRIMARY);
                                });
                        }
                    });
            }
            label(
                panel,
                format!("Intensity: {}%", state.intensity),
                17.0,
                PRIMARY,
            );
            panel
                .spawn(Node {
                    width: percent(100),
                    height: px(44),
                    column_gap: px(12),
                    flex_direction: FlexDirection::Row,
                    ..default()
                })
                .with_children(|steps| {
                    for value in [0, 25, 50, 75, 100] {
                        button(
                            steps,
                            &format!("{value}%"),
                            ManagerAction::Intensity(value),
                            percent(18.5),
                            state,
                        );
                    }
                });
        });
}

pub(super) fn rebuild_ui(
    mut commands: Commands,
    mut state: ResMut<ManagerState>,
    windows: Query<Entity, With<ManagerWindow>>,
    cameras: Query<Entity, With<ManagerCamera>>,
    roots: Query<Entity, With<ManagerRoot>>,
    mut input_focus: ResMut<InputFocus>,
) {
    if !state.dirty {
        return;
    }
    let (Ok(window), Ok(camera)) = (windows.single(), cameras.single()) else {
        return;
    };
    for root in &roots {
        commands.entity(root).despawn();
    }
    input_focus.clear();
    let entries = state.filtered();
    commands
        .spawn((
            Name::new("Nera Animation Manager UI"),
            ManagerOwned,
            ManagerRoot,
            UiTargetCamera(camera),
            Node {
                width: percent(100),
                height: percent(100),
                padding: UiRect::all(px(16)),
                ..default()
            },
            BackgroundColor(Color::NONE),
        ))
        .with_children(|outer| {
            outer
                .spawn((
                    Node {
                        width: percent(100),
                        height: percent(100),
                        padding: UiRect::all(px(18)),
                        flex_direction: FlexDirection::Column,
                        row_gap: px(13),
                        border_radius: BorderRadius::all(px(24)),
                        ..default()
                    },
                    BackgroundColor(SHELL),
                    Outline::new(px(1), Val::ZERO, OUTLINE),
                    BoxShadow::new(
                        Color::srgba(0.0, 0.0, 0.0, 0.58),
                        px(0),
                        px(8),
                        px(0),
                        px(22),
                    ),
                ))
                .with_children(|panel| {
                    panel
                        .spawn(Node {
                            width: percent(100),
                            height: px(72),
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::FlexStart,
                            column_gap: px(18),
                            ..default()
                        })
                        .with_children(|header| {
                            button(header, if state.tab == GalleryTab::Characters { "Done" } else { "Back" }, ManagerAction::Back, px(88), &state);
                            header
                                .spawn((
                                    Button,
                                    ManagerDragRegion,
                                    Node {
                                        flex_grow: 1.0,
                                        min_width: px(0),
                                        height: percent(100),
                                        flex_direction: FlexDirection::Column,
                                        row_gap: px(3),
                                        ..default()
                                    },
                                    BackgroundColor(Color::NONE),
                                ))
                                .with_children(|title| {
                                    for (value, size, color) in [
                                        (if state.tab == GalleryTab::Characters { "MULTI CHARACTER" } else { "ANIMATIONS" }, 27.0, PRIMARY),
                                        ("Drag title to move / edges to resize", 13.0, MUTED),
                                    ] {
                                        title.spawn((Text::new(value), TextFont { font_size: FontSize::Px(size), ..default() },
                                            TextColor(color), bevy::ui::FocusPolicy::Pass));
                                    }
                                });
                            header
                                .spawn(Node {
                                    column_gap: px(8),
                                    flex_shrink: 0.0,
                                    ..default()
                                })
                                .with_children(|controls| {
                                    crate::utility_window::spawn_window_controls(controls, window);
                                    button(controls, "X", ManagerAction::Close, px(44), &state);
                                });
                        });

                    if state.tab != GalleryTab::Characters { panel
                        .spawn(Node {
                            width: percent(100),
                            min_height: px(48),
                            flex_shrink: 0.0,
                            flex_wrap: FlexWrap::Wrap,
                            row_gap: px(8),
                            flex_direction: FlexDirection::Row,
                            column_gap: px(10),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|bar| {
                            button(bar, "Animasi Dasar", ManagerAction::Base, px(155), &state);
                            button(
                                bar,
                                "*  Bookmarks",
                                ManagerAction::Bookmarks,
                                px(150),
                                &state,
                            );
                            button(bar, "All Animations", ManagerAction::All, px(155), &state);
                            button(bar, "Expressions", ManagerAction::Expressions, px(145), &state);
                            bar.spawn(Node {
                                flex_grow: 1.0,
                                ..default()
                            });
                            button(bar, "Import VRMA", ManagerAction::Import, px(140), &state);
                        }); }

                    if state.tab == GalleryTab::Base {
                        label(panel, "Nera's core animations — click a preview to replay. Editing is locked.", 13.0, MUTED);
                    }

                    if state.tab == GalleryTab::Characters {
                        super::characters::panel(panel, &state);
                    } else if state.tab == GalleryTab::Expressions {
                        spawn_expression_panel(panel, &state);
                    } else {
                    panel
                        .spawn((
                            Node {
                                width: percent(100),
                                height: px(44),
                                padding: UiRect::horizontal(px(15)),
                                align_items: AlignItems::Center,
                                border_radius: BorderRadius::all(px(12)),
                                ..default()
                            },
                            BackgroundColor(CARD),
                            Outline::new(
                                px(1),
                                Val::ZERO,
                                if state.focus == TextFocus::Search {
                                    BLUE
                                } else {
                                    OUTLINE
                                },
                            ),
                        ))
                        .with_children(|search| {
                            text_field(search, TextFocus::Search, &state, &mut input_focus, 14.0, "Search animations...");
                        });

                    panel
                        .spawn(Node {
                            width: percent(100),
                            flex_grow: 1.0,
                            min_height: px(0),
                            position_type: PositionType::Relative,
                            ..default()
                        })
                        .with_children(|viewport| {
                            viewport
                                .spawn((
                                    GalleryScroll,
                                    ScrollPosition(Vec2::new(0.0, state.scroll_y)),
                                    Node {
                                        width: percent(100),
                                        height: percent(100),
                                        min_height: px(0),
                                        padding: UiRect::right(px(20)),
                                        flex_direction: FlexDirection::Column,
                                        row_gap: px(14),
                                        overflow: Overflow::scroll_y(),
                                        ..default()
                                    },
                                ))
                                .with_children(|gallery| {
                                    if entries.is_empty() {
                                        gallery
                                            .spawn((
                                                Node {
                                                    width: percent(100),
                                                    height: px(160),
                                                    justify_content: JustifyContent::Center,
                                                    align_items: AlignItems::Center,
                                                    ..default()
                                                },
                                                BackgroundColor(CARD),
                                                Outline::new(px(1), Val::ZERO, OUTLINE),
                                            ))
                                            .with_children(|empty| {
                                                label(
                                                    empty,
                                                    if state.tab == GalleryTab::Bookmarks {
                                                        "No bookmarked animations"
                                                    } else {
                                                        "No animations found"
                                                    },
                                                    18.0,
                                                    MUTED,
                                                )
                                            });
                                    }
                                    if !entries.is_empty() {
                                        gallery.spawn((
                                            GalleryContent(entries.len()),
                                            Node {
                                                width: percent(100),
                                                height: px(entries.len().div_ceil(COLUMNS) as f32
                                                    * ROW_HEIGHT
                                                    - 14.0),
                                                flex_shrink: 0.0,
                                                ..default()
                                            },
                                        ));
                                    }
                                });

                            viewport
                                .spawn((
                                    Button,
                                    GalleryScrollbarTrack,
                                    bevy::ui::RelativeCursorPosition::default(),
                                    Node {
                                        position_type: PositionType::Absolute,
                                        right: px(1),
                                        top: px(2),
                                        bottom: px(2),
                                        width: px(14),
                                        border_radius: BorderRadius::all(px(4)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.12, 0.16, 0.27, 0.72)),
                                ))
                                .with_children(|track| {
                                    track.spawn((
                                        GalleryScrollbarThumb,
                                        bevy::picking::Pickable::IGNORE,
                                        Node {
                                            position_type: PositionType::Absolute,
                                            top: px(0),
                                            width: percent(100),
                                            height: px(48),
                                            border_radius: BorderRadius::all(px(4)),
                                            ..default()
                                        },
                                        BackgroundColor(BLUE),
                                    ));
                                });
                        });
                    }

                    if let Some(error) = &state.error {
                        label(panel, error.clone(), 13.0, Color::srgb(1.0, 0.48, 0.53));
                    }
                    panel
                        .spawn(Node {
                            width: percent(100),
                            height: px(44),
                            flex_shrink: 0.0,
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|footer| {
                            if state.tab == GalleryTab::Characters {
                                label(footer, state.characters.status.clone(), 13.0, MUTED);
                            } else if state.tab == GalleryTab::Expressions {
                                label(footer, "8 expressions · preview on desktop pet", 14.0, MUTED);
                            } else {
                                footer.spawn((
                                    GalleryCount(entries.len()),
                                    Text::new(format!("{} animations", entries.len())),
                                    TextFont {
                                        font_size: FontSize::Px(14.0),
                                        ..default()
                                    },
                                    TextColor(MUTED),
                                ));
                            }
                            if state.tab != GalleryTab::Expressions && state.tab != GalleryTab::Characters {
                            button(
                                footer,
                                &format!("Combine ({})", state.combine_parts.len()),
                                ManagerAction::OpenCombine,
                                px(180),
                                &state,
                            );
                            }
                            if state.tab != GalleryTab::Characters { button(
                                footer,
                                if state.tab == GalleryTab::Expressions { "Reset Expression" } else { "Reset Previews" },
                                ManagerAction::Reset,
                                px(180),
                                &state,
                            ); }
                        });
                    if let Some(dialog) = &state.dialog {
                        spawn_dialog(panel, dialog, &state, &mut input_focus);
                    }
                });
        });
    state.dirty = false;
}

fn spawn_card(
    parent: &mut ChildSpawnerCommands,
    entry: &super::registry::AnimationEntry,
    index: usize,
    state: &ManagerState,
) {
    let key = entry.key.clone();
    let is_base = super::base::is_base_animation(&key);
    parent
        .spawn((
            Node {
                width: percent(32.5),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                border_radius: BorderRadius::all(px(15)),
                ..default()
            },
            BackgroundColor(CARD),
            Outline::new(px(1), Val::ZERO, OUTLINE),
        ))
        .with_children(|card| {
            card.spawn((
                Button,
                ManagerAction::Restart(key.clone()),
                PreviewSlot {
                    key: key.clone(),
                    index,
                },
                ImageNode::default(),
                Node {
                    width: percent(100),
                    height: px(280),
                    flex_shrink: 0.0,
                    border_radius: BorderRadius::all(px(14)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.07, 0.09, 0.16)),
            ));
            card.spawn(Node {
                width: percent(100),
                height: px(66),
                padding: UiRect::axes(px(12), px(8)),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|footer| {
                footer
                    .spawn(Node {
                        flex_grow: 1.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: px(2),
                        ..default()
                    })
                    .with_children(|info| {
                        label(info, entry.display_name.clone(), 15.0, PRIMARY);
                        label(
                            info,
                            if let Some(label) = super::base::base_label(&entry.key) {
                                format!("Dasar (locked) - {label}")
                            } else {
                                format!("{} - {}", entry.emotion, entry.source)
                            },
                            12.0,
                            MUTED,
                        );
                    });
                if !is_base {
                    button(
                        footer,
                        "...",
                        ManagerAction::OpenMenu(key.clone()),
                        px(40),
                        state,
                    );
                }
            });
            if !is_base && state.menu_key.as_deref() == Some(&key) {
                card.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        right: px(8),
                        bottom: px(55),
                        width: px(178),
                        padding: UiRect::all(px(7)),
                        row_gap: px(4),
                        flex_direction: FlexDirection::Column,
                        border_radius: BorderRadius::all(px(10)),
                        ..default()
                    },
                    BackgroundColor(SHELL),
                    Outline::new(px(1), Val::ZERO, OUTLINE),
                    ZIndex(10),
                ))
                .with_children(|menu| {
                    button(
                        menu,
                        "Add to Combine",
                        ManagerAction::AddCombine(key.clone()),
                        percent(100),
                        state,
                    );
                    button(
                        menu,
                        "Rename",
                        ManagerAction::Rename(key.clone()),
                        percent(100),
                        state,
                    );
                    button(
                        menu,
                        "Delete",
                        ManagerAction::Delete(key.clone()),
                        percent(100),
                        state,
                    );
                    let bookmarked = state.registry.bookmarks.contains(&key);
                    button(
                        menu,
                        if bookmarked {
                            "Remove Bookmark"
                        } else {
                            "Add Bookmark"
                        },
                        ManagerAction::Bookmark(key),
                        percent(100),
                        state,
                    );
                });
            }
        });
}

fn spawn_combine_editor(
    modal: &mut ChildSpawnerCommands,
    state: &ManagerState,
    input_focus: &mut InputFocus,
) {
    label(modal, "Combine animations", 24.0, PRIMARY);
    label(
        modal,
        "Play clips in order. Trim in 0.25-second steps. Original files stay intact.",
        13.0,
        MUTED,
    );
    modal
        .spawn((
            CombineListScroll,
            ScrollPosition(Vec2::new(0.0, state.combine_scroll_y)),
            Node {
                width: percent(100),
                flex_grow: 1.0,
                min_height: px(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(10),
                overflow: Overflow::scroll_y(),
                ..default()
            },
        ))
        .with_children(|list| {
            if state.combine_parts.is_empty() {
                label(
                    list,
                    "Choose clips with each card's ... menu > Add to Combine, then return here.",
                    16.0,
                    MUTED,
                );
            }
            for (index, part) in state.combine_parts.iter().enumerate() {
                list.spawn((
                    Node {
                        width: percent(100),
                        padding: UiRect::all(px(12)),
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: px(8),
                        border_radius: BorderRadius::all(px(10)),
                        ..default()
                    },
                    BackgroundColor(CARD),
                ))
                .with_children(|clip| {
                    label(clip, format!("{}. {}", index + 1, part.key), 14.0, PRIMARY);
                    clip.spawn(Node {
                        width: percent(100),
                        flex_direction: FlexDirection::Row,
                        column_gap: px(8),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|row| {
                        label(row, format!("Start: {:.2}s", part.start), 14.0, MUTED);
                        button(
                            row,
                            "-",
                            ManagerAction::TrimCombine(index, true, false),
                            px(40),
                            state,
                        );
                        button(
                            row,
                            "+",
                            ManagerAction::TrimCombine(index, true, true),
                            px(40),
                            state,
                        );
                        label(row, format!("End: {:.2}s", part.end), 14.0, MUTED);
                        button(
                            row,
                            "-",
                            ManagerAction::TrimCombine(index, false, false),
                            px(40),
                            state,
                        );
                        button(
                            row,
                            "+",
                            ManagerAction::TrimCombine(index, false, true),
                            px(40),
                            state,
                        );
                        button(
                            row,
                            "Up",
                            ManagerAction::MoveCombine(index, true),
                            px(52),
                            state,
                        );
                        button(
                            row,
                            "Down",
                            ManagerAction::MoveCombine(index, false),
                            px(72),
                            state,
                        );
                        button(row, "X", ManagerAction::RemoveCombine(index), px(40), state);
                    });
                });
            }
        });
    let total: f32 = state
        .combine_parts
        .iter()
        .map(|part| part.end - part.start)
        .sum();
    label(
        modal,
        format!(
            "{} clips · {:.2}s total · Save to animations/Combined",
            state.combine_parts.len(),
            total
        ),
        13.0,
        MUTED,
    );
    if state.combine_job.is_some() {
        label(
            modal,
            "Combining... Please wait while the new VRMA is saved.",
            15.0,
            PRIMARY,
        );
    }
    modal
        .spawn((
            Node {
                width: percent(100),
                height: px(46),
                flex_shrink: 0.0,
                padding: UiRect::horizontal(px(12)),
                align_items: AlignItems::Center,
                border_radius: BorderRadius::all(px(10)),
                ..default()
            },
            BackgroundColor(CARD),
            Outline::new(
                px(1),
                Val::ZERO,
                if state.focus == TextFocus::CombineName {
                    BLUE
                } else {
                    OUTLINE
                },
            ),
        ))
        .with_children(|field| {
            text_field(
                field,
                TextFocus::CombineName,
                state,
                input_focus,
                15.0,
                "New animation name",
            );
            if !state.combine_name.to_ascii_lowercase().ends_with(".vrma") {
                label(field, ".vrma", 15.0, MUTED);
            }
        });
    modal
        .spawn(Node {
            width: percent(100),
            flex_direction: FlexDirection::Row,
            column_gap: px(12),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|row| {
            button(
                row,
                "Add more / Back",
                ManagerAction::CancelDialog,
                px(215),
                state,
            );
            button(
                row,
                "Create VRMA",
                ManagerAction::ConfirmCombine,
                px(200),
                state,
            );
        });
}

fn spawn_dialog(
    parent: &mut ChildSpawnerCommands,
    dialog: &Dialog,
    state: &ManagerState,
    input_focus: &mut InputFocus,
) {
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.01, 0.02, 0.05, 0.84)),
            GlobalZIndex(100),
        ))
        .with_children(|overlay| {
            overlay
                .spawn((
                    Node {
                        width: if matches!(dialog, Dialog::Combine) {
                            px(750)
                        } else {
                            px(420)
                        },
                        height: if matches!(dialog, Dialog::Combine) {
                            percent(88)
                        } else {
                            Val::Auto
                        },
                        max_height: px(820),
                        padding: UiRect::all(px(22)),
                        row_gap: px(14),
                        flex_direction: FlexDirection::Column,
                        border_radius: BorderRadius::all(px(18)),
                        ..default()
                    },
                    BackgroundColor(SHELL),
                    Outline::new(px(1), Val::ZERO, OUTLINE),
                ))
                .with_children(|modal| {
                    if let Some(error) = &state.error {
                        label(modal, error.clone(), 13.0, Color::srgb(1.0, 0.48, 0.53));
                    }
                    match dialog {
                        Dialog::Combine => spawn_combine_editor(modal, state, input_focus),
                        Dialog::Rename(key) => {
                            label(modal, "Rename animation", 22.0, PRIMARY);
                            label(modal, key.rsplit('/').next().unwrap_or(key), 13.0, MUTED);
                            modal
                                .spawn((
                                    Node {
                                        width: percent(100),
                                        height: px(46),
                                        padding: UiRect::horizontal(px(12)),
                                        align_items: AlignItems::Center,
                                        border_radius: BorderRadius::all(px(10)),
                                        ..default()
                                    },
                                    BackgroundColor(CARD),
                                    Outline::new(
                                        px(1),
                                        Val::ZERO,
                                        if state.focus == TextFocus::Rename {
                                            BLUE
                                        } else {
                                            OUTLINE
                                        },
                                    ),
                                ))
                                .with_children(|field| {
                                    let suffix = if state
                                        .rename_value
                                        .to_ascii_lowercase()
                                        .ends_with(".vrma")
                                    {
                                        ""
                                    } else {
                                        ".vrma"
                                    };
                                    text_field(field, TextFocus::Rename, state, input_focus, 14.0, "");
                                    label(field, suffix, 14.0, MUTED);
                                });
                            modal
                                .spawn(Node {
                                    width: percent(100),
                                    flex_direction: FlexDirection::Row,
                                    column_gap: px(10),
                                    ..default()
                                })
                                .with_children(|row| {
                                    button(
                                        row,
                                        "Cancel",
                                        ManagerAction::CancelDialog,
                                        px(180),
                                        state,
                                    );
                                    button(
                                        row,
                                        "Rename",
                                        ManagerAction::ConfirmRename,
                                        px(180),
                                        state,
                                    );
                                });
                        }
                        Dialog::Delete(key) => {
                            label(modal, "Delete animation?", 22.0, PRIMARY);
                            label(modal, key.rsplit('/').next().unwrap_or(key), 15.0, MUTED);
                            label(modal, "This will remove the animation file.", 13.0, MUTED);
                            modal
                                .spawn(Node {
                                    width: percent(100),
                                    flex_direction: FlexDirection::Row,
                                    column_gap: px(10),
                                    ..default()
                                })
                                .with_children(|row| {
                                    button(
                                        row,
                                        "Cancel",
                                        ManagerAction::CancelDialog,
                                        px(180),
                                        state,
                                    );
                                    button(
                                        row,
                                        "Delete",
                                        ManagerAction::ConfirmDelete,
                                        px(180),
                                        state,
                                    );
                                });
                        }
                        Dialog::RemoveCharacter(path) => {
                            label(modal, "Remove character?", 22.0, PRIMARY);
                            if let Some(model) = state.characters.models.iter().find(|model| model.path == *path) {
                                label(modal, &model.name, 15.0, MUTED);
                            }
                            label(modal, "Active copies will switch to Nera or close. The original VRM is kept; you can import it again.", 13.0, MUTED);
                            modal.spawn(Node {
                                width: percent(100),
                                column_gap: px(10),
                                ..default()
                            }).with_children(|row| {
                                button(row, "Cancel", ManagerAction::CancelDialog, px(180), state);
                                button(row, "Remove", ManagerAction::ConfirmRemoveCharacter, px(180), state);
                            });
                        }
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_thousand_entries_only_build_nearby_cards_even_after_jumping_to_end() {
        let mut app = App::new();
        let mut state = ManagerState::for_test();
        state.registry.entries = (0..10_000)
            .map(|i| super::super::registry::AnimationEntry {
                key: format!("{i}.vrma"),
                display_name: format!("Animation {i}"),
                source: "Test".into(),
                emotion: "Neutral",
            })
            .collect();
        app.insert_resource(state)
            .add_systems(Update, sync_gallery_rows);
        app.world_mut().spawn((
            GalleryScroll,
            ComputedNode {
                size: Vec2::new(1000.0, 720.0),
                inverse_scale_factor: 1.0,
                ..default()
            },
        ));
        app.world_mut()
            .spawn((GalleryContent(10_000), Node::default()));
        for row in [0, 1200, 3332, 0] {
            app.world_mut().resource_mut::<ManagerState>().scroll_y = row as f32 * ROW_HEIGHT;
            app.update();
            let world = app.world_mut();
            let slots: Vec<_> = world
                .query::<&PreviewSlot>()
                .iter(world)
                .map(|slot| slot.index)
                .collect();
            assert!(
                slots.len() <= 15,
                "only viewport plus overscan should exist"
            );
            assert!(slots.contains(&(row * COLUMNS)));
            if row == 3332 {
                assert!(slots.contains(&9999));
            }
        }
    }
}
