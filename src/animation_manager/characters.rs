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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remove_button_is_import_only_and_never_nested_in_assign_button() {
        let mut app = App::new();
        let mut state = ManagerState::for_test();
        state.characters.models.truncate(1);
        state
            .characters
            .models
            .push(crate::character::library::CharacterEntry {
                path: "characters://model-card-test.vrm".into(),
                name: "Test".into(),
                facial_targets: 0,
                ..default()
            });
        app.insert_resource(state).add_systems(
            Update,
            |mut commands: Commands, state: Res<ManagerState>| {
                commands.spawn(Node::default()).with_children(|parent| {
                    card(parent, 0, &state);
                    card(parent, 1, &state);
                });
            },
        );
        app.update();
        let world = app.world_mut();
        let removes: Vec<_> = world
            .query::<(&ManagerAction, &ChildOf)>()
            .iter(world)
            .filter_map(|(action, parent)| {
                matches!(action, ManagerAction::RemoveCharacter(_)).then_some(parent.parent())
            })
            .collect();
        assert_eq!(removes.len(), 1);
        assert!(world.get::<Button>(removes[0]).is_none());
        let keys: Vec<_> = world
            .query::<&PreviewSlot>()
            .iter(world)
            .map(|slot| slot.key.clone())
            .collect();
        assert!(keys.contains(&"model/characters://model-card-test.vrm".into()));
        assert!(keys.contains(&format!("model/{}", crate::character::DEFAULT_CHARACTER)));
    }
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
                    button(
                        rail,
                        "Use Character",
                        ManagerAction::UseCharacter,
                        percent(100),
                        state,
                    );
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
                    if !state.characters.status.is_empty() {
                        label(rail, &state.characters.status, 12.0, ui::MUTED);
                    }
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
    let selected = state.selected_character == model.path;
    parent
        .spawn((
            Node {
                width: percent(32),
                min_width: px(0),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                border_radius: BorderRadius::all(px(8)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(if selected { ui::BLUE } else { ui::CARD }),
        ))
        .with_children(|card| {
            card.spawn((
                Button,
                ManagerAction::AssignCharacter(model.path.clone()),
                Node {
                    width: percent(100),
                    height: percent(100),
                    padding: UiRect::all(px(8)),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(8),
                    ..default()
                },
                BackgroundColor(if selected { ui::BLUE } else { ui::CARD }),
            ))
            .with_children(|body| {
                body.spawn((
                    PreviewSlot {
                        key: format!("model/{}", model.path),
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
                label(body, &model.name, 14.0, ui::PRIMARY);
                if model.facial_targets == 0 {
                    label(body, "No facial blendshapes", 12.0, ui::MUTED);
                }
            });
            if model.removable() {
                card.spawn((
                    Button,
                    ManagerAction::RemoveCharacter(model.path.clone()),
                    Node {
                        position_type: PositionType::Absolute,
                        top: px(8),
                        right: px(8),
                        width: px(32),
                        height: px(32),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border_radius: BorderRadius::all(px(6)),
                        ..default()
                    },
                    BackgroundColor(ui::BUTTON),
                    ZIndex(1),
                ))
                .with_children(|close| label(close, "X", 14.0, ui::PRIMARY));
            }
        });
}
