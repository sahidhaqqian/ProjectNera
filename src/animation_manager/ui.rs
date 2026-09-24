use bevy::{prelude::*, ui::UiTargetCamera};

use super::{
    Dialog, GalleryScroll, GalleryTab, ManagerAction, ManagerCamera, ManagerDragRegion,
    ManagerOwned, ManagerRoot, ManagerState, ManagerWindow, PreviewSlot, TextFocus,
};

pub(super) const ROW_HEIGHT: f32 = 232.0;
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
        ManagerAction::Import | ManagerAction::Reset | ManagerAction::ConfirmRename => BLUE,
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
    let entries: Vec<_> = state.filtered().into_iter().cloned().collect();
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
                                        "Manage and preview your Nera animations",
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
                            button(bar, "All Animations", ManagerAction::All, px(185), &state);
                            button(
                                bar,
                                "*  Bookmarks",
                                ManagerAction::Bookmarks,
                                px(176),
                                &state,
                            );
                            bar.spawn(Node {
                                flex_grow: 1.0,
                                ..default()
                            });
                            button(bar, "Import VRMA", ManagerAction::Import, px(178), &state);
                        });

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
                            Outline::new(px(1), Val::ZERO, OUTLINE),
                        ))
                        .with_children(|search| {
                            let text = if state.search.is_empty() {
                                "Search animations...".to_owned()
                            } else {
                                format!(
                                    "{}{}",
                                    state.search,
                                    if state.focus == TextFocus::Search {
                                        "|"
                                    } else {
                                        ""
                                    }
                                )
                            };
                            label(
                                search,
                                text,
                                14.0,
                                if state.search.is_empty() {
                                    MUTED
                                } else {
                                    PRIMARY
                                },
                            );
                        });

                    panel
                        .spawn((
                            GalleryScroll,
                            ScrollPosition(Vec2::new(0.0, state.scroll_y)),
                            Node {
                                width: percent(100),
                                flex_grow: 1.0,
                                min_height: px(0),
                                flex_direction: FlexDirection::Column,
                                row_gap: px(14),
                                overflow: Overflow::scroll_y(),
                                scrollbar_width: 8.0,
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
                            for (row_number, chunk) in entries.chunks(3).enumerate() {
                                gallery
                                    .spawn(Node {
                                        width: percent(100),
                                        height: px(218),
                                        flex_shrink: 0.0,
                                        flex_direction: FlexDirection::Row,
                                        column_gap: px(12),
                                        ..default()
                                    })
                                    .with_children(|row| {
                                        for (column, entry) in chunk.iter().enumerate() {
                                            spawn_card(row, entry, row_number * 3 + column, &state);
                                        }
                                    });
                            }
                        });

                    if let Some(error) = &state.error {
                        label(panel, error.clone(), 13.0, Color::srgb(1.0, 0.48, 0.53));
                    }
                    button(
                        panel,
                        "Reset Previews",
                        ManagerAction::Reset,
                        percent(100),
                        &state,
                    );
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
                    height: px(151),
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
                            format!("{} - {}", entry.emotion, entry.source),
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
                        width: px(420),
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
                                    Outline::new(px(1), Val::ZERO, OUTLINE),
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
                                    label(
                                        field,
                                        format!(
                                            "{}{}{}",
                                            state.rename_value,
                                            suffix,
                                            if state.focus == TextFocus::Rename {
                                                "|"
                                            } else {
                                                ""
                                            }
                                        ),
                                        14.0,
                                        PRIMARY,
                                    )
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
