//! Screensaver exits only for a mouse press on Nera's existing body hit route.
use super::{PresentationCommand, PresentationState};
use crate::interaction::HoveredBodyPart;
use bevy::{
    input::{ButtonState, mouse::MouseButtonInput},
    prelude::*,
    window::PrimaryWindow,
};

const ARM_SECONDS: f32 = 0.35;

#[derive(Debug, Default)]
pub(super) struct ClickGuard {
    elapsed: f32,
}

impl ClickGuard {
    fn observe(&mut self, dt: f32, clicked_avatar: bool) -> bool {
        self.elapsed += dt;
        self.elapsed >= ARM_SECONDS && clicked_avatar
    }
}

pub(super) fn exit_on_avatar_click(
    time: Res<Time>,
    mut state: ResMut<PresentationState>,
    mut mouse: MessageReader<MouseButtonInput>,
    primary: Query<Entity, With<PrimaryWindow>>,
    hovered: Res<HoveredBodyPart>,
    mut commands: MessageWriter<PresentationCommand>,
) {
    let primary = primary.single().ok();
    // Drain all events even outside Screensaver, so an old desktop/settings
    // click cannot dismiss the next presentation. Releases and wheel buttons
    // do not count; movement, scroll and keyboard never enter this path.
    let pressed_nera = mouse.read().fold(false, |pressed, event| {
        pressed
            | (Some(event.window) == primary
                && event.state == ButtonState::Pressed
                && matches!(event.button, MouseButton::Left | MouseButton::Right))
    });
    if !state.is_screensaver() {
        return;
    }
    let Some(input) = state.screensaver_input.as_mut() else {
        return;
    };
    if input.observe(time.delta_secs(), pressed_nera && hovered.part.is_some()) {
        info!("Screensaver exit: Nera clicked");
        commands.write(PresentationCommand::ExitScreensaver);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::BodyPart;

    #[test]
    fn entry_grace_and_no_click_never_dismiss() {
        let mut input = ClickGuard::default();
        assert!(!input.observe(0.1, true));
        assert!(!input.observe(0.1, true));
        assert!(!input.observe(1.0, false));
        assert!(input.observe(0.1, true));
    }

    #[test]
    fn only_primary_window_body_presses_exit_and_alarm_is_untouched() {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<PresentationState>()
            .init_resource::<HoveredBodyPart>()
            .add_message::<MouseButtonInput>()
            .add_message::<PresentationCommand>()
            .add_systems(Update, exit_on_avatar_click);
        let primary = app.world_mut().spawn(PrimaryWindow).id();
        let other = app.world_mut().spawn_empty().id();
        let mut state = app.world_mut().resource_mut::<PresentationState>();
        state.mode =
            super::super::PresentationMode::Attention(super::super::AttentionKind::Screensaver);
        state.screensaver_input = Some(ClickGuard { elapsed: 1.0 });
        app.world_mut().resource_mut::<HoveredBodyPart>().part = Some(BodyPart::Torso);
        for (window, button, state, hit, accepted) in [
            (other, MouseButton::Left, ButtonState::Pressed, true, false),
            (
                primary,
                MouseButton::Left,
                ButtonState::Released,
                true,
                false,
            ),
            (
                primary,
                MouseButton::Middle,
                ButtonState::Pressed,
                true,
                false,
            ),
            (
                primary,
                MouseButton::Left,
                ButtonState::Pressed,
                false,
                false,
            ),
            (primary, MouseButton::Left, ButtonState::Pressed, true, true),
            (
                primary,
                MouseButton::Right,
                ButtonState::Pressed,
                true,
                true,
            ),
        ] {
            app.world_mut().resource_mut::<HoveredBodyPart>().part = hit.then_some(BodyPart::Torso);
            app.world_mut().write_message(MouseButtonInput {
                window,
                button,
                state,
            });
            app.update();
            let mut commands = app
                .world_mut()
                .resource_mut::<Messages<PresentationCommand>>();
            assert_eq!(commands.len(), usize::from(accepted));
            commands.clear();
        }
        app.world_mut().resource_mut::<PresentationState>().mode =
            super::super::PresentationMode::Attention(super::super::AttentionKind::Alarm);
        app.world_mut().write_message(MouseButtonInput {
            window: primary,
            button: MouseButton::Left,
            state: ButtonState::Pressed,
        });
        app.update();
        assert!(
            app.world()
                .resource::<Messages<PresentationCommand>>()
                .is_empty()
        );
    }

    #[test]
    fn cursor_scroll_and_keyboard_activity_do_not_exit() {
        use bevy::{
            input::{
                mouse::{MouseScrollUnit, MouseWheel},
                touch::TouchPhase,
            },
            window::CursorMoved,
        };
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<PresentationState>()
            .insert_resource(HoveredBodyPart {
                part: Some(BodyPart::Head),
            })
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<MouseButtonInput>()
            .add_message::<MouseWheel>()
            .add_message::<CursorMoved>()
            .add_message::<PresentationCommand>()
            .add_systems(Update, exit_on_avatar_click);
        let primary = app.world_mut().spawn(PrimaryWindow).id();
        let mut state = app.world_mut().resource_mut::<PresentationState>();
        state.mode =
            super::super::PresentationMode::Attention(super::super::AttentionKind::Screensaver);
        state.screensaver_input = Some(ClickGuard { elapsed: 1.0 });
        for _ in 0..5 {
            app.world_mut().write_message(CursorMoved {
                window: primary,
                position: Vec2::splat(900.0),
                delta: Some(Vec2::splat(500.0)),
            });
            app.world_mut().write_message(MouseWheel {
                window: primary,
                unit: MouseScrollUnit::Line,
                x: 0.0,
                y: -10.0,
                phase: TouchPhase::Moved,
            });
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::Space);
            app.update();
            assert!(app.world().resource::<PresentationState>().is_screensaver());
            assert!(
                app.world()
                    .resource::<Messages<PresentationCommand>>()
                    .is_empty()
            );
        }
    }
}
