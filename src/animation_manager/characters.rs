use super::{
    GalleryScroll, ManagerAction, ManagerState, PreviewSlot,
    ui::{self, label},
};
use bevy::prelude::*;

#[derive(Component)]
pub(super) struct CharacterRail;

fn button(
    parent: &mut ChildSpawnerCommands,
    value: &str,
    action: ManagerAction,
    width: Val,
    state: &ManagerState,
) {
    let color = ui::button_color(&action, state);
    parent
        .spawn((
            Button,
            action,
            Node {
                width,
                height: px(40),
                min_width: px(0),
                flex_shrink: 0.0,
                padding: UiRect::horizontal(px(8)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            BackgroundColor(color),
        ))
        .with_children(|button| label(button, value, 13.0, ui::PRIMARY));
}

pub(super) fn panel(parent: &mut ChildSpawnerCommands, state: &ManagerState) {
    parent
        .spawn(Node {
            width: percent(100),
            flex_grow: 1.0,
            min_height: px(0),
            column_gap: px(18),
            ..default()
        })
        .with_children(|content| {
            content
                .spawn((
                    CharacterRail,
                    bevy::ui::RelativeCursorPosition::default(),
                    ScrollPosition(Vec2::new(0.0, state.character_scroll_y)),
                    Node {
                        width: px(184),
                        height: percent(100),
                        min_height: px(0),
                        overflow: Overflow::scroll_y(),
                        flex_shrink: 0.0,
                        flex_direction: FlexDirection::Column,
                        row_gap: px(3),
                        ..default()
                    },
                ))
                .with_children(|rail| {
                    label(rail, "Multi Character", 18.0, ui::PRIMARY);
                    for (index, slot) in state.characters.slots.iter().enumerate() {
                        rail.spawn(Node {
                            width: percent(100),
                            column_gap: px(8),
                            ..default()
                        })
                        .with_children(|row| {
                            button(
                                row,
                                &format!(
                                    "Character {}\n{}",
                                    index + 1,
                                    if slot.used { "Used" } else { "Not Used" }
                                ),
                                ManagerAction::CharacterSlot(index),
                                px(140),
                                state,
                            );
                            button(
                                row,
                                "X",
                                ManagerAction::ClearCharacter(index),
                                px(36),
                                state,
                            );
                        });
                    }
                    if state.characters.can_add {
                        button(
                            rail,
                            "Add More Character",
                            ManagerAction::AddCharacterSlot,
                            percent(100),
                            state,
                        );
                    }
                    button(
                        rail,
                        "Import Character",
                        ManagerAction::ImportCharacter,
                        percent(100),
                        state,
                    );
                });
            content
                .spawn((
                    GalleryScroll,
                    ScrollPosition(Vec2::new(0.0, state.scroll_y)),
                    Node {
                        flex_grow: 1.0,
                        min_width: px(0),
                        min_height: px(0),
                        height: percent(100),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                ))
                .with_children(|gallery| {
                    gallery.spawn((
                        ui::GalleryContent(state.characters.models.len()),
                        Node {
                            width: percent(100),
                            flex_shrink: 0.0,
                            height: px(state.characters.models.len().div_ceil(ui::COLUMNS) as f32
                                * ui::ROW_HEIGHT),
                            ..default()
                        },
                    ));
                });
        });
}

pub(super) fn card(parent: &mut ChildSpawnerCommands, index: usize, state: &ManagerState) {
    let model = &state.characters.models[index];
    let selected = state
        .characters
        .slots
        .get(state.character_slot)
        .and_then(|slot| slot.path.as_ref())
        == Some(&model.path);
    parent
        .spawn((
            Button,
            ManagerAction::AssignCharacter(model.path.clone()),
            Node {
                width: percent(32),
                min_width: px(0),
                height: percent(100),
                padding: UiRect::all(px(8)),
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
                border_radius: BorderRadius::all(px(8)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(if selected { ui::BLUE } else { ui::CARD }),
        ))
        .with_children(|card| {
            card.spawn((
                PreviewSlot {
                    key: format!("model/{index}"),
                    index,
                },
                ImageNode::default(),
                Node {
                    width: percent(100),
                    height: px(236),
                    flex_shrink: 0.0,
                    ..default()
                },
            ));
            label(card, &model.name, 14.0, ui::PRIMARY);
            if model.facial_targets == 0 {
                label(card, "No facial blendshapes", 12.0, ui::MUTED);
            }
        });
}
