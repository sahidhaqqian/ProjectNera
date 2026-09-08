//! Phase 5.1 — semantic interaction messages.
//!
//! This module owns the public vocabulary produced by Nera's existing input
//! pipeline. The actual native-window drag/sitting mechanics stay in `main.rs`
//! for Phase 5.1 so there is still only one state machine deciding whether a
//! pointer gesture is a click or a drag.

use bevy::prelude::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InteractionState {
    #[default]
    None,
    Hover,
    Pressed,
    Dragging,
}

/// Pointer coordinates captured at the moment a semantic interaction fires.
///
/// `screen` uses physical desktop pixels (useful for native menus/window work),
/// while `local` uses Nera-window logical coordinates (useful for Phase 5.2
/// raycasts/body-part detection).
#[derive(Clone, Copy, Debug)]
pub(crate) struct NeraPointerPosition {
    pub(crate) screen: IVec2,
    pub(crate) local: Vec2,
}

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraClicked {
    pub(crate) pointer: NeraPointerPosition,
}

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraRightClicked {
    pub(crate) pointer: NeraPointerPosition,
}

/// Extra lifecycle message beyond the minimum roadmap. Consumers that care
/// about drag setup do not need to infer the first `NeraDragged` frame.
#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraDragStarted {
    pub(crate) screen: IVec2,
}

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraDragged {
    pub(crate) screen: IVec2,
    pub(crate) delta: IVec2,
}

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct NeraReleased {
    pub(crate) pointer: NeraPointerPosition,
    pub(crate) was_dragging: bool,
}

pub(crate) struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<NeraClicked>()
            .add_message::<NeraRightClicked>()
            .add_message::<NeraDragStarted>()
            .add_message::<NeraDragged>()
            .add_message::<NeraReleased>()
            // Debug consumer only. Keeping it in PostUpdate guarantees the
            // Update-stage producer has already emitted this frame's messages.
            .add_systems(PostUpdate, log_interaction_messages);
    }
}

fn log_interaction_messages(
    mut clicked: MessageReader<NeraClicked>,
    mut right_clicked: MessageReader<NeraRightClicked>,
    mut drag_started: MessageReader<NeraDragStarted>,
    mut dragged: MessageReader<NeraDragged>,
    mut released: MessageReader<NeraReleased>,
) {
    for event in clicked.read() {
        info!(
            "Nera interaction -> Click screen={:?} local={:?}",
            event.pointer.screen, event.pointer.local
        );
    }

    for event in right_clicked.read() {
        info!(
            "Nera interaction -> RightClick screen={:?} local={:?}",
            event.pointer.screen, event.pointer.local
        );
    }

    for event in drag_started.read() {
        info!("Nera interaction -> DragStarted @ {:?}", event.screen);
    }

    for event in dragged.read() {
        trace!(
            "Nera interaction -> Dragged @ {:?}, delta={:?}",
            event.screen,
            event.delta
        );
    }

    for event in released.read() {
        info!(
            "Nera interaction -> Released screen={:?} local={:?} (was_dragging={})",
            event.pointer.screen,
            event.pointer.local,
            event.was_dragging
        );
    }
}
