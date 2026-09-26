use bevy::{prelude::*, ui::UiTargetCamera};

use super::{
    CaretBlink, CombineListScroll, Dialog, GalleryScroll, GalleryScrollbarThumb,
    GalleryScrollbarTrack, GalleryTab, ManagerAction, ManagerCamera, ManagerDragRegion,
    ManagerOwned, ManagerRoot, ManagerState, ManagerWindow, PreviewSlot, TextCaret, TextFocus,
};

pub(super) const ROW_HEIGHT: f32 = 360.0;
pub(super) const COLUMNS: usize = 3;

#[derive(Component)]
pub(super) struct GalleryContent(usize);
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
                    for index in
                        row_number * COLUMNS..((row_number + 1) * COLUMNS).min(entries.len())
                    {
                        spawn_card(row, entries[index], index, &state);
                    }
                });
        }
    });
    *previous = Some((scroll_entity, range));
}
const SHELL: Color = Color::srgba(0.032, 0.042, 0.078, 0.985);
const CARD: Color = Color::srgba(0.055, 0.071, 0.122, 0.98);
const OUTLINE: Color = Color::srgba(0.20, 0.27, 0.43, 0.85);
const PRIMARY: Color = Color::srgb(0.95, 0.96, 1.0);
const MUTED: Color = Color::srgb(0.60, 0.67, 0.82);
const BLUE: Color = Color::srgb(0.12, 0.39, 0.94);
pub(super) const BLUE_HOVER: Color = Color::srgb(0.19, 0.46, 1.0);
pub(super) const BLUE_PRESSED: Color = Color::srgb(0.10, 0.32, 0.78);
pub(super) const BUTTON: Color = Color::srgb(0.095, 0.12, 0.21);

pub(super) fn button_color(action: &ManagerAction, state: &ManagerState) -> Color {
    match action {
        ManagerAction::Import
        | ManagerAction::Reset
        | ManagerAction::ConfirmRename
        | ManagerAction::ConfirmCombine => BLUE,
        ManagerAction::Base if state.tab == GalleryTab::Base => BLUE,
        ManagerAction::All if state.tab == GalleryTab::All => BLUE,
        ManagerAction::Bookmarks if state.tab == GalleryTab::Bookmarks => BLUE,
        ManagerAction::ConfirmDelete => Color::srgb(0.58, 0.13, 0.18),
        _ => BUTTON,
    }
}

fn label(parent: &mut ChildSpawnerCommands, value: impl Into<String>, size: f32, color: Color) {
    parent.spawn((
        Text::new(value),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    ));
}

fn caret(parent: &mut ChildSpawnerCommands, visible: bool) {
    parent.spawn((
        TextCaret,
        Text::new("|"),
        TextFont {
            font_size: FontSize::Px(14.0),
            ..default()
        },
        TextColor(PRIMARY),
        if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        },
    ));
}

pub(super) fn blink_text_caret(
    time: Res<Time>,
    mut blink: ResMut<CaretBlink>,
    mut carets: Query<&mut Visibility, With<TextCaret>>,
) {
    blink.timer.tick(time.delta());
    if blink.timer.just_finished() {
        blink.visible = !blink.visible;
    }
    for mut visibility in &mut carets {
        *visibility = if blink.visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn button(
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

pub(super) fn rebuild_ui(
    mut commands: Commands,
    mut state: ResMut<ManagerState>,
    windows: Query<Entity, With<ManagerWindow>>,
    cameras: Query<Entity, With<ManagerCamera>>,
    roots: Query<Entity, With<ManagerRoot>>,
) {
    if !state.dirty {
        return;
    }
    let (Ok(_window), Ok(camera)) = (windows.single(), cameras.single()) else {
        return;
    };
    for root in &roots {
        commands.entity(root).despawn();
    }
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
                            button(header, "Back", ManagerAction::Back, px(88), &state);
                            header
                                .spawn((
                                    Button,
                                    ManagerDragRegion,
                                    Node {
                                        flex_grow: 1.0,
                                        flex_direction: FlexDirection::Column,
                                        row_gap: px(3),
                                        ..default()
                                    },
                                    BackgroundColor(Color::NONE),
                                ))
                                .with_children(|title| {
                                    label(title, "ANIMATIONS", 27.0, PRIMARY);
                                    label(
                                        title,
                                        "Drag the scrollbar or press Home / End to jump",
                                        13.0,
                                        MUTED,
                                    );
                                });
                            button(header, "X", ManagerAction::Close, px(48), &state);
                        });

                    panel
                        .spawn(Node {
                            width: percent(100),
                            height: px(48),
                            flex_direction: FlexDirection::Row,
                            column_gap: px(10),
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|bar| {
                            button(bar, "Animasi Dasar", ManagerAction::Base, px(155), &state);
                            button(bar, "All Animations", ManagerAction::All, px(155), &state);
                            button(
                                bar,
                                "*  Bookmarks",
                                ManagerAction::Bookmarks,
                                px(150),
                                &state,
                            );
                            bar.spawn(Node {
                                flex_grow: 1.0,
                                ..default()
                            });
                            button(bar, "Import VRMA", ManagerAction::Import, px(140), &state);
                        });

                    if state.tab == GalleryTab::Base {
                        label(panel, "Nera's core animations — preview or copy into Combine. Editing is locked.", 13.0, MUTED);
                    }

                    panel
                        .spawn((
                            Button,
                            ManagerAction::Search,
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
                            let focused = state.focus == TextFocus::Search;
                            if state.search.is_empty() && !focused {
                                label(search, "Search animations...", 14.0, MUTED);
                            } else {
                                label(search, state.search.clone(), 14.0, PRIMARY);
                                if focused {
                                    caret(search, true);
                                }
                            }
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
                            footer.spawn((
                                GalleryCount(entries.len()),
                                Text::new(format!("{} animations", entries.len())),
                                TextFont {
                                    font_size: FontSize::Px(14.0),
                                    ..default()
                                },
                                TextColor(MUTED),
                            ));
                            button(
                                footer,
                                &format!("Combine ({})", state.combine_parts.len()),
                                ManagerAction::OpenCombine,
                                px(180),
                                &state,
                            );
                            button(
                                footer,
                                "Reset Previews",
                                ManagerAction::Reset,
                                px(180),
                                &state,
                            );
                        });
                    if let Some(dialog) = &state.dialog {
                        spawn_dialog(panel, dialog, &state);
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
                button(
                    footer,
                    "...",
                    ManagerAction::OpenMenu(key.clone()),
                    px(40),
                    state,
                );
            });
            if state.menu_key.as_deref() == Some(&key) {
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
                    if super::base::is_base_animation(&key) {
                        label(menu, "Nera core - locked", 12.0, MUTED);
                        if let Some(index) = super::base::base_order(&key) {
                            label(
                                menu,
                                super::base::BASE_ANIMATIONS[index].purpose,
                                12.0,
                                MUTED,
                            );
                        }
                        return;
                    }
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

fn spawn_combine_editor(modal: &mut ChildSpawnerCommands, state: &ManagerState) {
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
            Button,
            ManagerAction::FocusCombineName,
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
            if state.combine_name.is_empty() && state.focus != TextFocus::CombineName {
                label(field, "New animation name", 15.0, MUTED);
            } else {
                label(field, &state.combine_name, 15.0, PRIMARY);
            }
            if state.focus == TextFocus::CombineName {
                caret(field, true);
            }
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

fn spawn_dialog(parent: &mut ChildSpawnerCommands, dialog: &Dialog, state: &ManagerState) {
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
                        Dialog::Combine => spawn_combine_editor(modal, state),
                        Dialog::Rename(key) => {
                            label(modal, "Rename animation", 22.0, PRIMARY);
                            label(modal, key.rsplit('/').next().unwrap_or(key), 13.0, MUTED);
                            modal
                                .spawn((
                                    Button,
                                    ManagerAction::FocusRename,
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
                                    label(field, state.rename_value.clone(), 14.0, PRIMARY);
                                    if state.focus == TextFocus::Rename {
                                        caret(field, true);
                                    }
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
