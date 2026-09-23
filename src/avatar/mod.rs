//! Avatar scene, gaze ownership, and pose-derived support anchors.
//! Scheduling remains in main so cross-subsystem ordering stays explicit.
mod anchors;
mod gaze;
mod spawn;

pub(crate) use anchors::{DeathVisualGrounding, update_avatar_anchors};
pub(crate) use gaze::{sync_look_at_for_animation, update_look_at_target};
pub(crate) use spawn::{setup_camera, setup_lighting, spawn_nera};

use bevy::prelude::*;

#[derive(Component)]
pub(crate) struct NeraAvatar;

#[derive(Component)]
pub(crate) struct LookAtTarget;
