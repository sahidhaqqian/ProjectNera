//! Focus controls use the existing alarm utility window; session state lives
//! exclusively in FocusSession, including while this window is closed.
use super::*;
use crate::focus::{FocusCommand, FocusLength, FocusPreset, FocusSession};

#[derive(Component, Clone, Copy)]
struct FocusControl(FocusCommand);
#[derive(Component)]
struct FocusStatus;
#[derive(Component)]
struct CustomMinutes;
#[derive(Component)]
struct FocusDescription;

pub(super) fn install(app: &mut App) {
    // Standalone editor fixtures also need the same semantic resource/channel;
    // FocusPlugin alone owns advancing the session in the production app.
    app.init_resource::<FocusSession>()
        .add_message::<FocusCommand>()
        .add_systems(Update, handle.before(crate::focus::update_focus))
        .add_systems(PostUpdate, sync);
}

fn button(parent: &mut ChildSpawnerCommands, label: &str, command: FocusCommand) {
    parent
        .spawn((
            Button,
            FocusControl(command),
            Node {
                padding: UiRect::axes(px(10), px(7)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::all(px(10)),
                ..default()
            },
            BackgroundColor(CARD),
        ))
        .with_children(|button| {
            let mut label = button.spawn((
                Text::new(label),
                TextFont {
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(PRIMARY),
            ));
            if command == FocusCommand::Length(FocusLength::Custom) {
                label.insert(CustomMinutes);
            }
        });
}

pub(super) fn spawn(panel: &mut ChildSpawnerCommands) {
    panel
        .spawn((
            Name::new("Focus section"),
            Node {
                width: percent(100),
                flex_shrink: 0.0,
                padding: UiRect::all(px(20)),
                flex_direction: FlexDirection::Column,
                row_gap: px(20),
                border_radius: BorderRadius::all(px(14)),
                ..default()
            },
            BackgroundColor(CARD),
            Outline::new(px(1), Val::ZERO, CARD_OUTLINE),
        ))
        .with_children(|card| {
            spawn_section_title(card, "Choose an activity");
            card.spawn(Node {
                column_gap: px(5),
                row_gap: px(5),
                flex_wrap: FlexWrap::Wrap,
                ..default()
            })
            .with_children(|row| {
                button(row, "Study", FocusCommand::Preset(FocusPreset::Study));
                button(row, "Work", FocusCommand::Preset(FocusPreset::Work));
            });
            card.spawn((FocusDescription, Text::new(FocusPreset::Study.description()), TextFont { font_size: FontSize::Px(14.0), ..default() }, TextColor(BODY)));
            spawn_section_title(card, "Session duration");
            card.spawn(Node { column_gap: px(8), row_gap: px(8), flex_wrap: FlexWrap::Wrap, ..default() }).with_children(|row| {
                button(row, "25 min", FocusCommand::Length(FocusLength::Minutes25));
                button(row, "50 min", FocusCommand::Length(FocusLength::Minutes50));
                button(row, "Custom", FocusCommand::Length(FocusLength::Custom));
                button(row, "Endless", FocusCommand::Length(FocusLength::Endless));
            });
            card.spawn(Node {
                column_gap: px(5),
                row_gap: px(5),
                flex_wrap: FlexWrap::Wrap,
                ..default()
            })
            .with_children(|row| {
                button(row, "-5", FocusCommand::CustomDelta(-5));
                button(row, "-1", FocusCommand::CustomDelta(-1));
                button(row, "+1", FocusCommand::CustomDelta(1));
                button(row, "+5", FocusCommand::CustomDelta(5));
            });
            card.spawn(Node { column_gap: px(12), ..default() }).with_children(|row| {
                button(row, "Start", FocusCommand::Start);
                button(row, "Stop", FocusCommand::Stop);
            });
            card.spawn((
                FocusStatus,
                Text::new("Ready"),
                TextFont {
                    font_size: FontSize::Px(17.0),
                    ..default()
                },
                TextColor(BODY),
            ));
            card.spawn((Text::new("Headpat is welcome. Roaming and pet sounds stay quiet. Alarms always take priority."), TextFont { font_size: FontSize::Px(12.0), ..default() }, TextColor(MUTED)));
        });
}

fn handle(
    buttons: Query<(&Interaction, &FocusControl), Changed<Interaction>>,
    mut requests: MessageWriter<FocusCommand>,
) {
    for (interaction, control) in &buttons {
        if *interaction == Interaction::Pressed {
            requests.write(control.0);
        }
    }
}
fn sync(
    focus: Res<FocusSession>,
    mut status: Query<
        &mut Text,
        (
            With<FocusStatus>,
            Without<CustomMinutes>,
            Without<FocusDescription>,
        ),
    >,
    mut custom: Query<
        &mut Text,
        (
            With<CustomMinutes>,
            Without<FocusStatus>,
            Without<FocusDescription>,
        ),
    >,
    mut description: Query<
        &mut Text,
        (
            With<FocusDescription>,
            Without<FocusStatus>,
            Without<CustomMinutes>,
        ),
    >,
    mut controls: Query<(&FocusControl, &Interaction, &mut BackgroundColor)>,
) {
    for mut text in &mut description {
        let next = focus.preset.description();
        if text.0 != next {
            text.0 = next.into();
        }
    }
    let next = focus.status();
    for mut text in &mut status {
        if text.0 != next {
            text.0.clone_from(&next);
        }
    }
    for mut text in &mut custom {
        let next = format!("Custom: {}m", focus.custom_minutes);
        if text.0 != next {
            text.0 = next;
        }
    }
    for (control, interaction, mut background) in &mut controls {
        let selected = match control.0 {
            FocusCommand::Preset(preset) => focus.preset == preset,
            FocusCommand::Length(length) => focus.length == length,
            FocusCommand::Start => focus.active(),
            _ => false,
        };
        let next = match interaction {
            Interaction::Pressed => BLUE_PRESSED,
            Interaction::Hovered => BLUE_HOVER,
            _ if selected => BLUE,
            _ => SHELL,
        };
        if background.0 != next {
            background.0 = next;
        }
    }
}

#[cfg(debug_assertions)]
pub(super) fn has_controls(world: &mut World) -> bool {
    world
        .query_filtered::<Entity, With<FocusControl>>()
        .iter(world)
        .next()
        .is_some()
}
#[cfg(debug_assertions)]
pub(super) fn smoke_control(world: &mut World, command: FocusCommand) {
    use bevy::ecs::system::RunSystemOnce;
    for mut interaction in world
        .query_filtered::<&mut Interaction, With<FocusControl>>()
        .iter_mut(world)
    {
        *interaction = Interaction::None;
    }
    let entity = world
        .query::<(Entity, &FocusControl)>()
        .iter(world)
        .find_map(|(entity, control)| (control.0 == command).then_some(entity))
        .expect("native Focus button");
    *world.get_mut::<Interaction>(entity).unwrap() = Interaction::Pressed;
    // Process through the real button handler before Bevy's next UI hit-test
    // replaces synthetic Interaction with the user's untouched OS pointer.
    world.run_system_once(handle).unwrap();
    *world.get_mut::<Interaction>(entity).unwrap() = Interaction::None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    #[test]
    fn focus_controls_select_both_modes_and_expose_all_session_choices() {
        let mut app = App::new();
        install(&mut app);
        app.world_mut()
            .run_system_once(|mut commands: Commands| {
                commands.spawn(Node::default()).with_children(spawn);
            })
            .unwrap();
        let controls: Vec<_> = app
            .world_mut()
            .query::<&FocusControl>()
            .iter(app.world())
            .map(|control| control.0)
            .collect();
        for command in [
            FocusCommand::Preset(FocusPreset::Study),
            FocusCommand::Preset(FocusPreset::Work),
            FocusCommand::Length(FocusLength::Minutes25),
            FocusCommand::Length(FocusLength::Minutes50),
            FocusCommand::Length(FocusLength::Custom),
            FocusCommand::Length(FocusLength::Endless),
            FocusCommand::Start,
            FocusCommand::Stop,
        ] {
            assert!(controls.contains(&command));
        }
        smoke_control(app.world_mut(), FocusCommand::Preset(FocusPreset::Work));
        assert!(
            app.world_mut()
                .resource_mut::<Messages<FocusCommand>>()
                .drain()
                .any(|command| command == FocusCommand::Preset(FocusPreset::Work))
        );
    }
}
