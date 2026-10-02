//! Native window operations for Nera's transparent, custom utility shells.
//! The OS state stays authoritative, including after taskbar restore or snap.

use bevy::{
    math::CompassOctant,
    prelude::*,
    window::{CursorIcon, SystemCursorIcon},
};

use crate::platform::{self, NativeWindowHandle};

pub(crate) mod font_cache;
pub(crate) mod text_input;

#[derive(Component)]
#[require(CursorIcon)]
pub(crate) struct UtilityWindow {
    shell_margin: f32,
    native_handle: Option<NativeWindowHandle>,
}

impl UtilityWindow {
    pub(crate) fn new(shell_margin: f32) -> Self {
        Self {
            shell_margin,
            native_handle: None,
        }
    }

    pub(crate) fn native_handle(&self) -> Option<NativeWindowHandle> {
        self.native_handle
    }
}

#[derive(Clone, Copy)]
pub(crate) enum WindowControlAction {
    Minimize,
    ToggleMaximize,
}

#[derive(Component)]
pub(crate) struct WindowControl {
    pub window: Entity,
    pub action: WindowControlAction,
}

pub(crate) struct UtilityWindowPlugin;

impl Plugin for UtilityWindowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<font_cache::UtilityFontCache>()
            .add_systems(First, acquire_native_handles)
            .add_systems(Update, (handle_window_controls, resize_windows).chain())
            .add_systems(Last, font_cache::release);
    }
}

/// HWND lookup touches winit's main-thread-only map. Cache it on the entity so
/// closing/reopening a utility window cannot accidentally reuse a stale handle.
fn acquire_native_handles(world: &mut World) {
    let mut windows = world.query::<(Entity, &mut UtilityWindow)>();
    for (entity, mut utility) in windows.iter_mut(world) {
        if utility.native_handle.is_none() {
            if let Some(handle) = platform::native_window_handle(entity) {
                utility.native_handle = Some(handle);
            }
        }
    }
}

pub(crate) fn spawn_window_controls(parent: &mut ChildSpawnerCommands, window: Entity) {
    for (label, action) in [
        ("-", WindowControlAction::Minimize),
        ("[]", WindowControlAction::ToggleMaximize),
    ] {
        parent
            .spawn((
                Button,
                WindowControl { window, action },
                Node {
                    width: px(36),
                    height: px(42),
                    flex_shrink: 0.0,
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(14)),
                    ..default()
                },
                BackgroundColor(control_color(Interaction::None)),
                Outline::new(px(1), Val::ZERO, Color::srgba(0.22, 0.28, 0.43, 0.95)),
            ))
            .with_children(|button| {
                button.spawn((
                    Text::new(label),
                    TextFont {
                        font_size: FontSize::Px(18.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.89, 0.91, 1.0)),
                ));
            });
    }
}

fn control_color(interaction: Interaction) -> Color {
    match interaction {
        Interaction::None => Color::srgb(0.09, 0.11, 0.19),
        Interaction::Hovered => Color::srgb(0.13, 0.16, 0.26),
        Interaction::Pressed => Color::srgb(0.07, 0.085, 0.15),
    }
}

fn handle_window_controls(
    mouse: Res<ButtonInput<MouseButton>>,
    mut buttons: Query<(&Interaction, &WindowControl, &mut BackgroundColor), Changed<Interaction>>,
    mut windows: Query<(&mut Window, &UtilityWindow)>,
) {
    for (interaction, control, mut background) in &mut buttons {
        background.0 = control_color(*interaction);
        if *interaction != Interaction::Pressed || !mouse.just_pressed(MouseButton::Left) {
            continue;
        }
        let Ok((mut window, utility)) = windows.get_mut(control.window) else {
            continue;
        };
        if !window.focused {
            continue;
        }
        match control.action {
            WindowControlAction::Minimize => window.set_minimized(true),
            WindowControlAction::ToggleMaximize => {
                if let Some(handle) = utility.native_handle {
                    window.set_maximized(!platform::is_window_maximized(handle));
                }
            }
        }
    }
}

fn resize_direction(cursor: Vec2, size: Vec2, shell_margin: f32) -> Option<CompassOctant> {
    if cursor.x < 0.0 || cursor.y < 0.0 || cursor.x >= size.x || cursor.y >= size.y {
        return None;
    }
    // Reach the visible border beyond the transparent margin, including its
    // rounded corners, without covering controls inside the panel padding.
    let edge = shell_margin + 8.0;
    let corner = shell_margin + 16.0;
    let left = cursor.x < corner;
    let right = cursor.x >= size.x - corner;
    let top = cursor.y < corner;
    let bottom = cursor.y >= size.y - corner;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(CompassOctant::NorthWest),
        (_, true, true, _) => Some(CompassOctant::NorthEast),
        (true, _, _, true) => Some(CompassOctant::SouthWest),
        (_, true, _, true) => Some(CompassOctant::SouthEast),
        _ if cursor.x < edge => Some(CompassOctant::West),
        _ if cursor.x >= size.x - edge => Some(CompassOctant::East),
        _ if cursor.y < edge => Some(CompassOctant::North),
        _ if cursor.y >= size.y - edge => Some(CompassOctant::South),
        _ => None,
    }
}

fn resize_windows(
    mouse: Res<ButtonInput<MouseButton>>,
    mut windows: Query<(&mut Window, &mut CursorIcon, &UtilityWindow)>,
) {
    for (mut window, mut cursor_icon, utility) in &mut windows {
        let maximized = utility
            .native_handle
            .is_some_and(platform::is_window_maximized);
        let direction = window
            .cursor_position()
            .filter(|_| !maximized && window.resizable)
            .and_then(|cursor| {
                resize_direction(
                    cursor,
                    Vec2::new(window.width(), window.height()),
                    utility.shell_margin,
                )
            });
        let icon = match direction {
            Some(CompassOctant::North | CompassOctant::South) => SystemCursorIcon::NsResize,
            Some(CompassOctant::East | CompassOctant::West) => SystemCursorIcon::EwResize,
            Some(CompassOctant::NorthEast | CompassOctant::SouthWest) => {
                SystemCursorIcon::NeswResize
            }
            Some(CompassOctant::NorthWest | CompassOctant::SouthEast) => {
                SystemCursorIcon::NwseResize
            }
            None => SystemCursorIcon::Default,
        };
        cursor_icon.set_if_neq(icon.into());
        if mouse.just_pressed(MouseButton::Left) && window.focused {
            if let Some(direction) = direction {
                window.start_drag_resize(direction);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_targets_reach_rounded_shell_but_leave_header_controls_alone() {
        let size = Vec2::new(600.0, 800.0);
        for margin in [14.0, 16.0] {
            assert_eq!(
                resize_direction(Vec2::new(margin + 10.0, margin + 10.0), size, margin),
                Some(CompassOctant::NorthWest)
            );
            assert_eq!(
                resize_direction(Vec2::new(size.x - margin - 4.0, 400.0), size, margin),
                Some(CompassOctant::East)
            );
            assert_eq!(
                resize_direction(Vec2::new(margin + 18.0, margin + 18.0), size, margin),
                None
            );
            assert_eq!(
                resize_direction(Vec2::new(300.0, 400.0), size, margin),
                None
            );
            assert_eq!(resize_direction(Vec2::new(-1.0, 400.0), size, margin), None);
        }
    }

    #[test]
    fn controls_only_minimize_the_target_focused_window() {
        for focused in [false, true] {
            let mut app = App::new();
            app.init_resource::<ButtonInput<MouseButton>>()
                .add_systems(Update, handle_window_controls);
            let target = app
                .world_mut()
                .spawn((
                    Window {
                        focused,
                        ..default()
                    },
                    UtilityWindow::new(14.0),
                ))
                .id();
            let other = app
                .world_mut()
                .spawn((Window::default(), UtilityWindow::new(14.0)))
                .id();
            app.world_mut().spawn((
                Interaction::Pressed,
                WindowControl {
                    window: target,
                    action: WindowControlAction::Minimize,
                },
                BackgroundColor(Color::NONE),
            ));
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(MouseButton::Left);
            app.update();
            assert_eq!(
                app.world_mut()
                    .get_mut::<Window>(target)
                    .unwrap()
                    .internal
                    .take_minimize_request(),
                focused.then_some(true)
            );
            assert_eq!(
                app.world_mut()
                    .get_mut::<Window>(other)
                    .unwrap()
                    .internal
                    .take_minimize_request(),
                None
            );
        }
    }
}
