//! Temporary placement lease for manually selected VRMA clips. The existing
//! controller owns playback; desktop/environment own native facts and support.
use bevy::{animation::animated_field, prelude::*};
use bevy_vrm1::prelude::{HipsBoneEntity, RestTransform, VrmBone, VrmSystemSets};

use super::{NeraAnimationPlayback, NeraAnimationState, TemporaryAction, VisualState};
use crate::{
    avatar::{NeraAvatar, NeraMainCamera},
    desktop_physics::{DesktopPhysicsState, PhysicsSupport},
    environment::{AvatarAnchor, DesktopEnvironment, PlacementState},
    interaction::{DesktopPetInteraction, InteractionState, NeraLiveBone},
    platform::{self, NativeWindowHandle, PhysicalRect},
};

const RETURN_SECONDS: f32 = 0.35;

#[cfg(debug_assertions)]
pub(super) mod smoke;

#[derive(Clone, Copy, Debug)]
struct Checkpoint {
    position: IVec2,
    placement: PlacementState,
    anchor: AvatarAnchor,
    contact: Vec2,
    monitor: PhysicalRect,
    window: Option<(NativeWindowHandle, PhysicalRect)>,
}
impl Checkpoint {
    fn contact_y_at(&self, destination: IVec2, surface_delta: IVec2, dpi: f32) -> f32 {
        (self.position.y + surface_delta.y - destination.y) as f32 / dpi + self.contact.y
    }
    fn capture(environment: &DesktopEnvironment, physics: &DesktopPhysicsState) -> Option<Self> {
        let rect = environment.nera_rect?;
        let contact = if environment.placement != PlacementState::Free {
            environment.seat_anchor_logical?
        } else {
            physics
                .support_anchor_logical()
                .or(environment.feet_anchor_logical)?
        };
        let target = match (environment.placement, physics.support) {
            (PlacementState::Window(attachment), _) => Some(attachment.target),
            (_, Some(PhysicsSupport::Window { target, .. })) => Some(target),
            _ => None,
        };
        Some(Self {
            position: IVec2::new(rect.left, rect.top),
            placement: environment.placement,
            anchor: environment.attached_anchor,
            contact,
            monitor: environment.monitor?.bounds,
            window: target
                .and_then(|target| platform::window_rect(target).map(|rect| (target, rect))),
        })
    }
    fn surface_delta(&self) -> Option<IVec2> {
        match self.window {
            Some((target, original)) => {
                if !platform::is_window_alive(target)
                    || platform::is_window_minimized(target)
                    || platform::is_window_cloaked(target)
                {
                    return None;
                }
                let current = platform::window_rect(target)?;
                Some(IVec2::new(
                    current.left - original.left,
                    current.top - original.top,
                ))
            }
            None => Some(IVec2::ZERO),
        }
    }
}

struct Session {
    avatar: Entity,
    hips: Entity,
    clip: Entity,
    revision: u64,
    checkpoint: Checkpoint,
    clip_origin: IVec2,
    first: Vec3,
    translation: Option<TranslationPose>,
    horizontal: Vec3,
    airborne: bool,
    grounding: super::GroundingPolicy,
    initial_bias: Option<f32>,
    returning: Option<(IVec2, f32)>,
    expected: IVec2,
    frame_target: Option<IVec2>,
}

struct TranslationPose {
    clip: Handle<AnimationClip>,
    target: bevy::animation::AnimationTargetId,
    source_rest: Vec3,
    destination_rest: Vec3,
    basis: Quat,
    ratio: f32,
}
impl TranslationPose {
    fn sample(&self, world: &World, time: f32) -> Option<Vec3> {
        let raw = world
            .resource::<Assets<AnimationClip>>()
            .get(&self.clip)?
            .sample_clamped(animated_field!(Transform::translation), self.target, time)?;
        let pose = self.destination_rest + self.basis * (raw - self.source_rest) * self.ratio;
        pose.is_finite().then_some(pose)
    }
}

#[derive(Resource, Default)]
pub(crate) struct LibraryMotion {
    session: Option<Session>,
    raw_hips: Option<(Entity, Vec3)>,
    raw_root: Option<(Entity, f32)>,
}
impl LibraryMotion {
    #[cfg(debug_assertions)]
    pub(crate) fn raw_root_y_for_validation(&self, avatar: Entity) -> Option<f32> {
        self.raw_root
            .filter(|(owner, _)| *owner == avatar)
            .map(|(_, y)| y)
    }

    #[cfg(debug_assertions)]
    pub(crate) fn smoke_contact_target(
        &self,
        environment: &DesktopEnvironment,
        margin: f32,
    ) -> Option<f32> {
        let session = self.session.as_ref()?;
        let rect = environment.nera_rect?;
        Some(
            session.checkpoint.contact_y_at(
                IVec2::new(rect.left, rect.top),
                session.checkpoint.surface_delta()?,
                environment.nera_scale_factor.max(0.25) as f32,
            ) - margin,
        )
    }
    pub(crate) fn owns_placement(&self) -> bool {
        self.session.is_some()
    }

    pub(crate) fn uses_body_contact(&self) -> bool {
        self.session.as_ref().is_some_and(|session| {
            session.grounding == super::GroundingPolicy::BodyContact && session.returning.is_none()
        })
    }

    pub(crate) fn needs_pose_shadow_contact(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| !session.airborne && session.returning.is_none())
    }
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<LibraryMotion>()
        .add_systems(
            PreUpdate,
            restore_pose.before(crate::avatar::restore_visual_grounding),
        )
        .add_systems(
            Update,
            cancel_interrupted
                .after(crate::interaction::drag_nera_window)
                .before(crate::desktop_physics::update_desktop_physics),
        )
        .add_systems(
            PostUpdate,
            extract_motion
                .after(bevy::app::AnimationSystems)
                .after(crate::avatar::apply_walking_pose)
                .after(crate::avatar::preserve_seated_hips)
                .before(crate::avatar::body_mode::BodyProportionSet::Apply)
                .before(VrmSystemSets::Constraints),
        )
        .add_systems(
            PostUpdate,
            ground_pose
                .after(VrmSystemSets::PropagateAfterConstraints)
                .after(bevy::camera::CameraUpdateSystems)
                .before(VrmSystemSets::GazeControl),
        )
        .add_systems(
            Last,
            move_canvas
                .after(crate::avatar::update_avatar_anchors)
                .before(crate::cloud::align_cloud_seat),
        );
}

fn manual_clip(world: &World) -> Option<Entity> {
    let state = world.resource::<NeraAnimationState>();
    let VisualState::LibraryAnimation(clip) = state.visual_state() else {
        return None;
    };
    (!state.auto_dance_active()).then_some(clip)
}

pub(super) fn uses_checkpoint_motion(visual: VisualState, path: Option<&str>) -> bool {
    matches!(visual, VisualState::LibraryAnimation(_))
        && super::grounding_policy(visual, path) != super::GroundingPolicy::BodyContact
}
fn blocked(world: &World) -> bool {
    world
        .get_resource::<crate::presentation::PresentationState>()
        .is_some_and(|p| !p.is_desktop_pet())
        || world
            .get_resource::<crate::cloud::CloudRuntime>()
            .is_some_and(|c| c.owns_placement())
        || world
            .get_resource::<crate::character::CharacterImportStatus>()
            .is_some_and(|s| s.loading)
        || world
            .get_resource::<DesktopPetInteraction>()
            .is_some_and(|i| i.state == InteractionState::Dragging || i.drag_offset.is_some())
        || world
            .get_resource::<DesktopEnvironment>()
            .is_some_and(|e| e.hidden_for_target_suspend)
}

fn restore_pose(world: &mut World) {
    world.resource_scope(|world, mut motion: Mut<LibraryMotion>| {
        if let Some((entity, translation)) = motion.raw_hips.take()
            && let Some(mut pose) = world.get_mut::<Transform>(entity)
        {
            pose.translation = translation;
        }
        if let Some((entity, y)) = motion.raw_root.take()
            && let Some(mut pose) = world.get_mut::<Transform>(entity)
        {
            pose.translation.y = y;
        }
    });
}
pub(super) fn clear(world: &mut World) {
    if world.contains_resource::<LibraryMotion>() {
        restore_pose(world);
        world.resource_mut::<LibraryMotion>().session = None;
    }
}

fn cancel_interrupted(world: &mut World) {
    let interrupted = blocked(world)
        || world
            .resource::<LibraryMotion>()
            .session
            .as_ref()
            .is_some_and(|session| {
                let environment = world.resource::<DesktopEnvironment>();
                environment.nera_rect.is_some_and(|rect| {
                    IVec2::new(rect.left, rect.top)
                        .as_vec2()
                        .distance(session.expected.as_vec2())
                        > 2.0
                }) || environment.placement != session.checkpoint.placement
                    || session.checkpoint.surface_delta().is_none()
            });
    if interrupted && let Some(session) = world.resource_mut::<LibraryMotion>().session.take() {
        world
            .resource_mut::<NeraAnimationState>()
            .finish_action(TemporaryAction::LibraryAnimation(session.clip));
        info!("Library motion cancelled by a new placement owner; checkpoint discarded");
    }
}

fn extract_motion(world: &mut World) {
    #[cfg(debug_assertions)]
    if smoke::legacy_playback() {
        return;
    }
    if blocked(world) {
        return;
    }
    let selected = manual_clip(world);
    world.resource_scope(|world, mut motion: Mut<LibraryMotion>| {
        if let Some(clip) = selected {
            if world.resource::<NeraAnimationPlayback>().current_clip != Some(clip) {
                return;
            }
            let visual = VisualState::LibraryAnimation(clip);
            let path = world.get_resource::<super::LibraryPlayback>()
                .and_then(|library| library.grounding_path_for(clip));
            if !uses_checkpoint_motion(visual, path) {
                // Death already has a reversible, bounded body-contact layer.
                // Retargeting its hips again and applying standing contact can
                // lift the lying pose and move its shadow away from the floor.
                // Leave playback, physics and grounding on their original path.
                motion.session = None;
                return;
            }
            let revision = world.resource::<NeraAnimationState>().revision;
            let changed = motion
                .session
                .as_ref()
                .is_none_or(|session| session.clip != clip || session.revision != revision);
            if changed {
                let Some((avatar, hips)) = world
                    .query_filtered::<(Entity, &HipsBoneEntity), With<NeraAvatar>>()
                    .iter(world)
                    .next()
                    .map(|(e, h)| (e, h.0))
                else {
                    return;
                };
                let Some(rest) = world.get::<RestTransform>(hips).map(|p| p.0) else {
                    return;
                };
                let Some(target) = world
                    .get::<bevy::animation::AnimationTargetId>(hips)
                    .copied()
                else {
                    return;
                };
                let Some((handle, _)) = playing_clip(world, hips) else {
                    return;
                };
                let translation = source_translation(world, clip, hips);
                let Some(animation_clip) = world.resource::<Assets<AnimationClip>>().get(&handle)
                else {
                    return;
                };
                let first = translation.as_ref().and_then(|p| p.sample(world, 0.0))
                    .or_else(|| animation_clip.sample_clamped(animated_field!(Transform::translation), target, 0.0))
                    .unwrap_or(rest.translation);
                let last = translation.as_ref().and_then(|p| p.sample(world, animation_clip.duration()))
                    .or_else(|| animation_clip.sample_clamped(animated_field!(Transform::translation), target, animation_clip.duration()));
                if !first.is_finite() {
                    return;
                }
                let airborne = if let Some(pose) = translation.as_ref() {
                    airborne_profile(animation_clip.duration(), rest.translation, |time| pose.sample(world, time))
                } else {
                    allows_airborne(animation_clip, target, rest.translation)
                };
                let grounding = super::grounding_policy(
                    VisualState::LibraryAnimation(clip),
                    world.get_resource::<super::LibraryPlayback>().and_then(|library| library.path_for(clip)),
                );
                let environment = world.resource::<DesktopEnvironment>();
                let Some(checkpoint) = motion
                    .session
                    .as_ref()
                    .filter(|s| s.avatar == avatar)
                    .map(|s| s.checkpoint)
                    .or_else(|| {
                        Checkpoint::capture(environment, world.resource::<DesktopPhysicsState>())
                    })
                else {
                    return;
                };
                let Some(rect) = environment.nera_rect else {
                    return;
                };
                let position = IVec2::new(rect.left, rect.top);
                motion.session = Some(Session {
                    avatar,
                    hips,
                    clip,
                    revision,
                    checkpoint,
                    clip_origin: position,
                    first,
                    translation,
                    horizontal: Vec3::ZERO,
                    airborne,
                    grounding,
                    initial_bias: None,
                    returning: None,
                    expected: position,
                    frame_target: None,
                });
                info!("Library checkpoint captured | position={position:?} airborne={airborne} hips_first={first:?} hips_last={last:?}");
            }
        } else if let Some(session) = motion.session.as_mut() {
            if !matches!(
                world.resource::<NeraAnimationState>().visual_state(),
                VisualState::Idle | VisualState::SittingLoop
            ) {
                motion.session = None;
                return;
            }
            if session.returning.is_none() {
                let Some(rect) = world.resource::<DesktopEnvironment>().nera_rect else {
                    motion.session = None;
                    return;
                };
                session.returning = Some((IVec2::new(rect.left, rect.top), 0.0));
                info!("Library animation ended; returning to checkpoint");
            }
        }
        let Some(session) = motion.session.as_mut() else {
            return;
        };
        let Some(hips) = world.get::<Transform>(session.hips) else {
            motion.session = None;
            return;
        };
        let raw = hips.translation;
        let entity = session.hips;
        let returning = session.returning.is_some();
        let first = session.first;
        // Sample only this scoped translation overlay at the existing player's
        // time. Never mutate a cached clip shared with AutoDance or later uses.
        let corrected = if !returning {
            playing_clip(world, entity).and_then(|(_, time)| session.translation.as_ref()?.sample(world, time)).unwrap_or(raw)
        } else { raw };
        // Desktop X moves the HWND once. Depth is part of the 3D pose: replacing
        // it with a screen Y offset changes the view/contact of a backward fall.
        let Some(rest) = world.get::<RestTransform>(entity).map(|rest| rest.0.translation) else {
            motion.session = None;
            return;
        };
        let Some(mut hips) = world.get_mut::<Transform>(entity) else {
            return;
        };
        hips.translation.x = rest.x;
        hips.translation.y = corrected.y;
        hips.translation.z = corrected.z;
        if !returning {
            session.horizontal = Vec3::new(corrected.x - first.x, 0.0, 0.0);
        }
        motion.raw_hips = Some((entity, raw));
    });
}

fn allows_airborne(
    clip: &AnimationClip,
    target: bevy::animation::AnimationTargetId,
    rest: Vec3,
) -> bool {
    airborne_profile(clip.duration(), rest, |time| {
        clip.sample_clamped(animated_field!(Transform::translation), target, time)
    })
}
fn airborne_profile(duration: f32, rest: Vec3, sample: impl Fn(f32) -> Option<Vec3>) -> bool {
    // Analyze actual authored height, not a filename. Low/crouched/get-up clips
    // retain ground contact; a deliberate rise above rest retains its trajectory.
    let samples = (duration * 30.0).ceil().clamp(2.0, 4096.0) as u32;
    (0..=samples).any(|i| {
        sample(duration * i as f32 / samples as f32)
            .is_some_and(|pose| pose.y > rest.y + rest.length().max(0.01) * 0.03)
    })
}

fn descendant_named(world: &World, root: Entity, name: &str) -> Option<Entity> {
    let mut pending = vec![root];
    while let Some(entity) = pending.pop() {
        if world
            .get::<Name>(entity)
            .is_some_and(|n| n.as_str() == name)
        {
            return Some(entity);
        }
        if let Some(children) = world.get::<Children>(entity) {
            pending.extend(children.iter());
        }
    }
    None
}

fn source_translation(world: &mut World, entity: Entity, hips: Entity) -> Option<TranslationPose> {
    use bevy_vrm1::prelude::{RestGlobalTransform, VrmaAsset};
    let Some(source) = world.get::<super::library::LibrarySource>(entity) else {
        return None;
    };
    let source_handle = source.asset.clone();
    let Some(asset) = world.resource::<Assets<VrmaAsset>>().get(&source_handle) else {
        return None;
    };
    let Some(document) = asset.gltf.source.as_ref().map(|source| &source.document) else {
        return None;
    };
    let Some(mapping) = document
        .extension_value("VRMC_vrm_animation")
        .and_then(|v| v.pointer("/humanoid/humanBones"))
    else {
        return None;
    };
    let source_bone = |name: &str| -> Option<Entity> {
        let index = mapping.get(name)?.get("node")?.as_u64()? as usize;
        descendant_named(world, entity, document.nodes().nth(index)?.name()?)
    };
    let Some(source_hips) = source_bone("hips") else {
        return None;
    };
    let Some(source_target) = world
        .get::<bevy::animation::AnimationTargetId>(source_hips)
        .copied()
    else {
        return None;
    };
    let Some(source_rest) = world
        .get::<RestTransform>(source_hips)
        .map(|p| p.0.translation)
    else {
        return None;
    };
    let Some(destination_rest) = world.get::<RestTransform>(hips).map(|p| p.0.translation) else {
        return None;
    };
    let position = |bone| {
        world
            .get::<RestGlobalTransform>(bone)
            .map(|p| p.0.translation())
    };
    let source_feet = ["leftFoot", "rightFoot"].map(|name| source_bone(name).and_then(position));
    let source_toes = ["leftToes", "rightToes"].map(|name| source_bone(name).and_then(position));
    let Some(source_hip_position) = position(source_hips) else {
        return None;
    };
    let Some(destination_hip_position) = position(hips) else {
        return None;
    };
    let Some(original_handle) = asset.gltf.animations.first().cloned() else {
        return None;
    };
    let mut destination_feet = [None; 2];
    let mut destination_toes = [None; 2];
    for (bone, rest) in world
        .query_filtered::<(&VrmBone, &RestGlobalTransform), With<NeraLiveBone>>()
        .iter(world)
    {
        match bone.as_str() {
            "leftFoot" => destination_feet[0] = Some(rest.0.translation()),
            "rightFoot" => destination_feet[1] = Some(rest.0.translation()),
            "leftToes" => destination_toes[0] = Some(rest.0.translation()),
            "rightToes" => destination_toes[1] = Some(rest.0.translation()),
            _ => {}
        }
    }
    let mean = |points: [Option<Vec3>; 2]| {
        let valid: Vec<_> = points.into_iter().flatten().collect();
        (!valid.is_empty()).then(|| valid.iter().copied().sum::<Vec3>() / valid.len() as f32)
    };
    let (Some(source_foot), Some(destination_foot)) = (mean(source_feet), mean(destination_feet))
    else {
        return None;
    };
    // Rest-leg height differences cancel the scene's desktop Y translation.
    // World-space hips Y alone is almost zero for Nera at root Y=-0.85.
    let Some(ratio) = height_ratio(
        source_hip_position,
        source_foot,
        destination_hip_position,
        destination_foot,
    ) else {
        return None;
    };
    let forward = |feet: Vec3, toes: [Option<Vec3>; 2]| {
        mean(toes).and_then(|toe| {
            let v = toe - feet;
            Vec3::new(v.x, 0.0, v.z).try_normalize()
        })
    };
    let basis = match (
        forward(source_foot, source_toes),
        forward(destination_foot, destination_toes),
    ) {
        (Some(source), Some(destination)) => Quat::from_rotation_arc(source, destination),
        _ => Quat::IDENTITY,
    };
    let parent_rotation = |bone| {
        world
            .get::<ChildOf>(bone)
            .and_then(|p| world.get::<RestGlobalTransform>(p.parent()))
            .map_or(Quat::IDENTITY, |rest| {
                rest.0.to_scale_rotation_translation().1
            })
    };
    let basis = parent_rotation(hips).inverse() * basis * parent_rotation(source_hips);
    let pose = TranslationPose {
        clip: original_handle,
        target: source_target,
        source_rest,
        destination_rest,
        basis,
        ratio,
    };
    pose.sample(world, 0.0)?;
    info!("Library hips translation overlay | rest-leg ratio={ratio:.3} basis={basis:?}");
    Some(pose)
}

fn height_ratio(source_hips: Vec3, source_foot: Vec3, hips: Vec3, foot: Vec3) -> Option<f32> {
    let source = (source_hips.y - source_foot.y).abs();
    let destination = (hips.y - foot.y).abs();
    let ratio = destination / source;
    (source > 1e-5 && destination > 1e-5 && ratio.is_finite() && ratio > 0.0).then_some(ratio)
}

fn playing_clip(world: &World, mut entity: Entity) -> Option<(Handle<AnimationClip>, f32)> {
    // Read the existing humanoid player's graph, never a second playback path
    // or private bevy_vrm1 component. Manual entry has a zero-duration blend.
    loop {
        if let Some(player) = world.get::<AnimationPlayer>(entity) {
            let (index, animation) = player
                .playing_animations()
                .max_by(|(_, a), (_, b)| a.weight().total_cmp(&b.weight()))?;
            let graph = world.get::<AnimationGraphHandle>(entity)?;
            let graph = world.resource::<Assets<AnimationGraph>>().get(&graph.0)?;
            return match &graph.get(*index)?.node_type {
                bevy::animation::graph::AnimationNodeType::Clip(handle) => {
                    Some((handle.clone(), animation.seek_time()))
                }
                _ => None,
            };
        }
        entity = world.get::<ChildOf>(entity)?.parent();
    }
}

fn ground_pose(world: &mut World) {
    world.resource_scope(|world, mut motion: Mut<LibraryMotion>| {
        let Some(session) = motion.session.as_mut() else {
            return;
        };
        session.frame_target = None;
        if blocked(world) {
            return;
        }
        let environment = world.resource::<DesktopEnvironment>();
        let Some(surface_delta) = session.checkpoint.surface_delta() else {
            return;
        };
        if environment.nera_rect.is_none() {
            return;
        }
        let dpi = environment.nera_scale_factor.max(0.25) as f32;
        if let Some((_, elapsed)) = session.returning.as_mut() {
            *elapsed += world.resource::<Time>().delta_secs().min(0.05);
        }
        // Solve contact against the HWND destination of this rendered frame,
        // not its old location before root-motion projection/return easing.
        let Some(target) = pose_destination(world, session, surface_delta, dpi) else {
            return;
        };
        session.frame_target = Some(target);
        let floor = session.checkpoint.contact_y_at(target, surface_delta, dpi);
        let Some((camera, view)) = camera_geometry(world) else {
            return;
        };
        let contact = if session.returning.is_some()
            && session.checkpoint.placement != PlacementState::Free
        {
            world
                .get::<GlobalTransform>(session.hips)
                .map(crate::avatar::seat_contact_world)
        } else {
            world
                .query_filtered::<(&VrmBone, &GlobalTransform), With<NeraLiveBone>>()
                .iter(world)
                .filter(|(bone, _)| ground_contact_bone(bone.as_str(), session.grounding))
                .filter_map(|(_, pose)| {
                    camera
                        .world_to_viewport(&view, pose.translation())
                        .ok()
                        .map(|pixel| (pixel.y, pose.translation()))
                })
                .max_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, pose)| pose)
        };
        let Some(contact) = contact else { return };
        let margin = if session.returning.is_some()
            && session.checkpoint.placement != PlacementState::Free
        {
            0.0
        } else {
            let settings = world.resource::<crate::settings::NeraSettings>();
            let scale = world
                .get_resource::<crate::avatar::body_mode::BodyProportions>()
                .map_or(1.0, |b| b.body_scale());
            crate::avatar::standing_support_offset(settings, scale)
        };
        let Some(bias) =
            crate::avatar::contact_y_correction(&camera, &view, contact, floor - margin)
        else {
            return;
        };
        let bias = if session.airborne && session.returning.is_none() {
            *session.initial_bias.get_or_insert(bias)
        } else {
            bias
        };
        if !bias.is_finite() {
            return;
        }
        if let Some(mut root) = world.get_mut::<Transform>(session.avatar) {
            let raw_y = root.translation.y;
            root.translation.y += bias;
            motion.raw_root = Some((session.avatar, raw_y));
        }
    });
}
fn contact_bone(name: &str) -> bool {
    matches!(
        name,
        "head"
            | "hips"
            | "spine"
            | "chest"
            | "leftHand"
            | "rightHand"
            | "leftLowerLeg"
            | "rightLowerLeg"
            | "leftFoot"
            | "rightFoot"
            | "leftToes"
            | "rightToes"
            | "leftLowerArm"
            | "rightLowerArm"
    )
}

fn ground_contact_bone(name: &str, policy: super::GroundingPolicy) -> bool {
    // Grounded clips share desktop's ankle/sole convention. Selecting toes or
    // hands instead changes the plane, especially with Chibi proportions.
    if policy == super::GroundingPolicy::Grounded {
        matches!(name, "leftFoot" | "rightFoot")
    } else {
        // Authored crouches/get-up poses still use their evaluated full-body
        // footprint, so planting raised ankles cannot sink the body or trigger
        // a monitor-bound clamp that lifts the whole canvas off its floor.
        contact_bone(name)
    }
}

fn move_canvas(world: &mut World) {
    world.resource_scope(|world, mut motion: Mut<LibraryMotion>| {
        let Some(session) = motion.session.as_mut() else {
            return;
        };
        if blocked(world) {
            return;
        }
        if session.checkpoint.surface_delta().is_none() {
            return;
        }
        let environment = world.resource::<DesktopEnvironment>();
        let (Some(hwnd), Some(rect)) = (environment.nera_hwnd, environment.nera_rect) else {
            return;
        };
        let dpi = environment.nera_scale_factor.max(0.25) as f32;
        let finished = session
            .returning
            .is_some_and(|(_, elapsed)| elapsed >= RETURN_SECONDS);
        let Some(mut target) = session.frame_target else {
            return;
        };
        if session.returning.is_none() {
            let Some((camera, view)) = camera_geometry(world) else {
                return;
            };
            // Bound the evaluated visible humanoid, not its transparent canvas.
            let mut min = Vec2::splat(f32::INFINITY);
            let mut max = Vec2::splat(f32::NEG_INFINITY);
            for pose in world
                .query_filtered::<&GlobalTransform, With<NeraLiveBone>>()
                .iter(world)
            {
                if let Ok(pixel) = camera.world_to_viewport(&view, pose.translation()) {
                    min = min.min(pixel * dpi);
                    max = max.max(pixel * dpi);
                }
            }
            target = clamp_position(target, min, max, session.checkpoint.monitor);
        }
        if IVec2::new(rect.left, rect.top) != target
            && platform::move_window(hwnd, platform::PhysicalPoint::new(target.x, target.y))
                .is_err()
        {
            return;
        }
        let actual = platform::window_rect(hwnd).unwrap_or(rect);
        world.resource_mut::<DesktopEnvironment>().nera_rect = Some(actual);
        session.expected = IVec2::new(actual.left, actual.top);
        if finished {
            let checkpoint = session.checkpoint;
            let mut environment = world.resource_mut::<DesktopEnvironment>();
            environment.placement = checkpoint.placement;
            environment.set_attached_anchor(checkpoint.anchor);
            environment.realign_sitting_attachment();
            info!("Library checkpoint restored | position={target:?}");
            motion.session = None;
        }
    });
}

fn pose_destination(
    world: &mut World,
    session: &Session,
    surface_delta: IVec2,
    dpi: f32,
) -> Option<IVec2> {
    if let Some((from, elapsed)) = session.returning {
        let t = (elapsed / RETURN_SECONDS).clamp(0.0, 1.0);
        return Some(
            from.as_vec2()
                .lerp(
                    (session.checkpoint.position + surface_delta).as_vec2(),
                    t * t * (3.0 - 2.0 * t),
                )
                .round()
                .as_ivec2(),
        );
    }
    let hips = *world.get::<GlobalTransform>(session.hips)?;
    let parent = world
        .get::<ChildOf>(session.hips)
        .and_then(|p| world.get::<GlobalTransform>(p.parent()))?;
    let scale = world
        .get_resource::<crate::avatar::body_mode::BodyProportions>()
        .map_or(1.0, |b| b.body_scale());
    let displacement = parent
        .affine()
        .transform_vector3(session.horizontal * scale);
    let (camera, view) = camera_geometry(world)?;
    let origin = camera.world_to_viewport(&view, hips.translation()).ok()?;
    let moved = camera
        .world_to_viewport(&view, hips.translation() + displacement)
        .ok()?;
    Some(session.clip_origin + surface_delta + desktop_motion_offset(moved - origin, dpi))
}

fn desktop_motion_offset(projected: Vec2, dpi: f32) -> IVec2 {
    // A forward/backward displacement has a vertical perspective component,
    // but the desktop support surface is 2D. Moving the HWND by that component
    // lifts its frozen physics/shadow anchors until placement is released.
    // Keep canvas Y on the checkpoint plane. Authored rises/jumps still live
    // in hips Y and retain the existing airborne-pose handling.
    IVec2::new((projected.x * dpi).round() as i32, 0)
}

fn camera_geometry(world: &mut World) -> Option<(Camera, GlobalTransform)> {
    world
        .query_filtered::<(&Camera, &GlobalTransform), With<NeraMainCamera>>()
        .iter(world)
        .next()
        .map(|(camera, view)| (camera.clone(), *view))
}

fn clamp_position(desired: IVec2, min: Vec2, max: Vec2, monitor: PhysicalRect) -> IVec2 {
    if !min.is_finite() || !max.is_finite() {
        return desired;
    }
    let low = Vec2::new(monitor.left as f32, monitor.top as f32) - min;
    let high = Vec2::new(monitor.right as f32, monitor.bottom as f32) - max;
    let clamp = |value: f32, low: f32, high: f32| {
        if low <= high {
            value.clamp(low, high)
        } else {
            (low + high) * 0.5
        }
    };
    Vec2::new(
        clamp(desired.x as f32, low.x, high.x),
        clamp(desired.y as f32, low.y, high.y),
    )
    .round()
    .as_ivec2()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use std::time::Duration;
    const DEATH_PATH: &str = "animations/animasi_basic/Death01.vrma";

    #[test]
    fn grounded_manual_contact_preserves_desktop_feet_and_body_contact_remains_available() {
        use super::super::GroundingPolicy::*;
        for name in ["leftFoot", "rightFoot"] {
            assert!(ground_contact_bone(name, Grounded));
        }
        for name in ["leftToes", "rightToes", "leftHand", "hips"] {
            assert!(!ground_contact_bone(name, Grounded));
            assert!(ground_contact_bone(name, BodyContact));
            assert!(ground_contact_bone(name, Authored));
        }
    }

    #[test]
    fn checkpoint_contact_rebases_pixels_without_moving_its_support_surface() {
        let checkpoint = Checkpoint {
            position: IVec2::new(230, 152),
            placement: PlacementState::Free,
            anchor: AvatarAnchor::Feet,
            contact: Vec2::new(300.0, 872.0),
            monitor: PhysicalRect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            window: None,
        };
        for dpi in [1.0, 1.25, 2.0] {
            for y in [152, 168, -20] {
                let delta = IVec2::new(0, 17);
                let local = checkpoint.contact_y_at(IVec2::new(230, y), delta, dpi);
                let global = local * dpi + y as f32;
                assert!((global - (152.0 + 17.0 + 872.0 * dpi)).abs() < 0.001);
            }
        }
    }

    fn curve(end: Vec3) -> (AnimationClip, bevy::animation::AnimationTargetId) {
        let target = bevy::animation::AnimationTargetId::from_name(&Name::new("hips"));
        let mut clip = AnimationClip::default();
        clip.add_curve_to_target(
            target,
            AnimatableCurve::new(
                animated_field!(Transform::translation),
                AnimatableKeyframeCurve::new([(0.0, Vec3::Y * 0.9), (2.0, end)]).unwrap(),
            ),
        );
        (clip, target)
    }
    fn fixture() -> (World, Entity, Entity, Entity) {
        let mut world = World::new();
        world.init_resource::<Assets<AnimationClip>>();
        world.init_resource::<Assets<AnimationGraph>>();
        world.init_resource::<NeraAnimationState>();
        world.init_resource::<NeraAnimationPlayback>();
        world.init_resource::<LibraryMotion>();
        world.init_resource::<DesktopPhysicsState>();
        world.init_resource::<DesktopPetInteraction>();
        world.init_resource::<Time>();
        let (clip, target) = curve(Vec3::new(1.0, 0.9, 0.0));
        let handle = world.resource_mut::<Assets<AnimationClip>>().add(clip);
        let (graph, node) = AnimationGraph::from_clip(handle);
        let graph = world.resource_mut::<Assets<AnimationGraph>>().add(graph);
        let mut player = AnimationPlayer::default();
        player.play(node);
        let root = world
            .spawn((
                Transform::from_xyz(0.0, -0.85, 0.0),
                GlobalTransform::IDENTITY,
                player,
                AnimationGraphHandle(graph),
            ))
            .id();
        let hips = world
            .spawn((
                Transform::from_xyz(0.0, 0.9, 0.0),
                RestTransform(Transform::from_xyz(0.0, 0.9, 0.0)),
                target,
                ChildOf(root),
            ))
            .id();
        world
            .entity_mut(root)
            .insert((NeraAvatar, HipsBoneEntity(hips)));
        let entity = world.spawn_empty().id();
        world.resource_mut::<NeraAnimationPlayback>().current_clip = Some(entity);
        world.resource_mut::<NeraAnimationState>().request_one_shot(
            TemporaryAction::LibraryAnimation(entity),
            Duration::from_secs(2),
        );
        world.insert_resource(DesktopEnvironment {
            nera_rect: Some(PhysicalRect {
                left: 300,
                top: 0,
                right: 900,
                bottom: 950,
            }),
            feet_anchor_logical: Some(Vec2::new(300.0, 900.0)),
            monitor: Some(crate::platform::MonitorInfo {
                handle: crate::platform::NativeMonitorHandle::from_raw(1),
                bounds: PhysicalRect {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1080,
                },
                work_area: PhysicalRect {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1040,
                },
                is_primary: true,
            }),
            ..default()
        });
        (world, root, hips, entity)
    }
    #[test]
    fn checkpoint_releases_safely_when_desktop_rect_or_rest_bone_disappears() {
        for missing_rect in [false, true] {
            let (mut world, _, hips, clip) = fixture();
            world.run_system_once(extract_motion).unwrap();
            assert!(world.resource::<LibraryMotion>().session.is_some());
            if missing_rect {
                world
                    .resource_mut::<NeraAnimationState>()
                    .finish_action(TemporaryAction::LibraryAnimation(clip));
                world.resource_mut::<DesktopEnvironment>().nera_rect = None;
            } else {
                world.entity_mut(hips).remove::<RestTransform>();
            }
            world.run_system_once(extract_motion).unwrap();
            assert!(world.resource::<LibraryMotion>().session.is_none());
        }
    }
    #[test]
    fn rest_height_ratio_is_independent_of_world_origin_and_rejects_degenerate_rigs() {
        for offset in [-100.0, -0.85, 0.0, 40.0] {
            let y = Vec3::Y * offset;
            let ratio = height_ratio(y + Vec3::Y, y, y + Vec3::Y * 0.8, y).unwrap();
            assert!((ratio - 0.8).abs() < 0.00001);
        }
        assert!(height_ratio(Vec3::ZERO, Vec3::ZERO, Vec3::Y, Vec3::ZERO).is_none());
        assert!(height_ratio(Vec3::NAN, Vec3::ZERO, Vec3::Y, Vec3::ZERO).is_none());
    }
    #[test]
    fn authored_jump_retains_airborne_motion_while_crouch_and_getup_remain_grounded() {
        let (jump, target) = curve(Vec3::new(0.0, 1.5, 0.0));
        assert!(allows_airborne(&jump, target, Vec3::Y * 0.9));
        let (crouch, target) = curve(Vec3::Y * 0.1);
        assert!(!allows_airborne(&crouch, target, Vec3::Y * 0.9));
    }
    #[test]
    fn projected_depth_motion_does_not_lift_the_desktop_canvas() {
        for dpi in [1.0, 1.25, 2.0] {
            for projected_y in [-300.0, 0.0, 180.0] {
                assert_eq!(
                    desktop_motion_offset(Vec2::new(12.0, projected_y), dpi),
                    IVec2::new((12.0 * dpi).round() as i32, 0),
                );
            }
        }
    }

    #[test]
    fn death_retains_original_retargeted_pose_and_does_not_take_checkpoint_ownership() {
        let (mut world, root, hips, clip) = fixture();
        let mut library = super::super::LibraryPlayback::default();
        library.cache_clip(DEATH_PATH.into(), clip, 0);
        world.insert_resource(library);
        let pose = Transform::from_xyz(0.03, 0.82, -0.02)
            .with_rotation(Quat::from_rotation_x(-1.3))
            .with_scale(Vec3::splat(0.62));
        *world.get_mut::<Transform>(hips).unwrap() = pose;
        let root_pose = *world.get::<Transform>(root).unwrap();
        world.run_system_once(extract_motion).unwrap();
        assert!(!world.resource::<LibraryMotion>().owns_placement());
        assert_eq!(*world.get::<Transform>(hips).unwrap(), pose);
        assert_eq!(*world.get::<Transform>(root).unwrap(), root_pose);
        assert!(world.resource::<LibraryMotion>().raw_hips.is_none());
        assert!(world.resource::<LibraryMotion>().raw_root.is_none());
        assert!(
            !world
                .resource::<LibraryMotion>()
                .needs_pose_shadow_contact()
        );
    }

    #[test]
    fn selecting_death_releases_an_existing_motion_lease_to_body_grounding() {
        let (mut world, _, hips, _) = fixture();
        world.run_system_once(extract_motion).unwrap();
        assert!(world.resource::<LibraryMotion>().owns_placement());
        world.run_system_once(restore_pose).unwrap();
        let death = world.spawn_empty().id();
        let mut library = super::super::LibraryPlayback::default();
        library.cache_clip(DEATH_PATH.into(), death, 0);
        world.insert_resource(library);
        world.resource_mut::<NeraAnimationPlayback>().current_clip = Some(death);
        world.resource_mut::<NeraAnimationState>().request_one_shot(
            TemporaryAction::LibraryAnimation(death),
            Duration::from_secs(2),
        );
        let pose = Transform::from_xyz(0.03, 0.8, 0.02);
        *world.get_mut::<Transform>(hips).unwrap() = pose;
        world.run_system_once(extract_motion).unwrap();
        assert!(!world.resource::<LibraryMotion>().owns_placement());
        assert_eq!(*world.get::<Transform>(hips).unwrap(), pose);
        assert!(!uses_checkpoint_motion(
            VisualState::LibraryAnimation(death),
            Some(DEATH_PATH)
        ));
        assert!(uses_checkpoint_motion(
            VisualState::LibraryAnimation(death),
            Some("animations/Mixamo-VRMA/Walking_10.vrma")
        ));
    }

    #[test]
    fn depth_is_preserved_in_the_rig_instead_of_transferred_to_window_y() {
        let (mut world, _, hips, _) = fixture();
        world.run_system_once(extract_motion).unwrap();
        world.get_mut::<Transform>(hips).unwrap().translation = Vec3::new(0.2, 0.7, -0.5);
        world.run_system_once(extract_motion).unwrap();
        assert_eq!(
            world.get::<Transform>(hips).unwrap().translation,
            Vec3::new(0.0, 0.7, -0.5)
        );
        assert_eq!(
            world
                .resource::<LibraryMotion>()
                .session
                .as_ref()
                .unwrap()
                .horizontal,
            Vec3::new(0.2, 0.0, 0.0)
        );
        assert!(
            world
                .resource::<LibraryMotion>()
                .needs_pose_shadow_contact()
        );
        world
            .resource_mut::<LibraryMotion>()
            .session
            .as_mut()
            .unwrap()
            .airborne = true;
        assert!(
            !world
                .resource::<LibraryMotion>()
                .needs_pose_shadow_contact()
        );
        clear(&mut world);
        assert_eq!(
            world.get::<Transform>(hips).unwrap().translation,
            Vec3::new(0.2, 0.7, -0.5)
        );
    }
    #[test]
    fn translation_overlay_uses_player_time_without_mutating_shared_clips() {
        let (mut world, root, hips, _) = fixture();
        let (handle, _) = playing_clip(&world, hips).unwrap();
        let target = *world
            .get::<bevy::animation::AnimationTargetId>(hips)
            .unwrap();
        world.run_system_once(extract_motion).unwrap();
        let session = world
            .resource_mut::<LibraryMotion>()
            .into_inner()
            .session
            .as_mut()
            .unwrap();
        session.translation = Some(TranslationPose {
            clip: handle.clone(),
            target,
            source_rest: Vec3::Y * 0.9,
            destination_rest: Vec3::Y * 0.5,
            basis: Quat::IDENTITY,
            ratio: 0.8,
        });
        session.first = Vec3::Y * 0.5;
        world
            .get_mut::<AnimationPlayer>(root)
            .unwrap()
            .playing_animations_mut()
            .next()
            .unwrap()
            .1
            .set_seek_time(1.0);
        world.run_system_once(extract_motion).unwrap();
        assert_eq!(
            world.get::<Transform>(hips).unwrap().translation,
            Vec3::Y * 0.5
        );
        assert_eq!(
            world
                .resource::<LibraryMotion>()
                .session
                .as_ref()
                .unwrap()
                .horizontal,
            Vec3::X * 0.4
        );
        assert_eq!(
            world
                .resource::<Assets<AnimationClip>>()
                .get(&handle)
                .unwrap()
                .sample_clamped(animated_field!(Transform::translation), target, 1.0)
                .unwrap(),
            Vec3::new(0.5, 0.9, 0.0)
        );
        clear(&mut world);
        assert_eq!(
            world.get::<Transform>(hips).unwrap().translation,
            Vec3::Y * 0.9
        );
    }
    #[test]
    fn repeated_motion_is_absolute_restores_raw_pose_and_never_scales_or_rotates_bones() {
        let (mut world, root, hips, _) = fixture();
        let raw = Transform::from_xyz(0.7, 0.6, -0.2)
            .with_scale(Vec3::splat(0.62))
            .with_rotation(Quat::from_rotation_z(0.4));
        for _ in 0..100 {
            *world.get_mut::<Transform>(hips).unwrap() = raw;
            world.run_system_once(extract_motion).unwrap();
            let pose = world.get::<Transform>(hips).unwrap();
            assert_eq!(pose.translation, Vec3::new(0.0, 0.6, -0.2));
            assert_eq!(pose.rotation, raw.rotation);
            assert_eq!(pose.scale, raw.scale);
            assert_eq!(
                world
                    .resource::<LibraryMotion>()
                    .session
                    .as_ref()
                    .unwrap()
                    .horizontal,
                Vec3::new(0.7, 0.0, 0.0)
            );
            world.run_system_once(restore_pose).unwrap();
            assert_eq!(*world.get::<Transform>(hips).unwrap(), raw);
            assert_eq!(world.get::<Transform>(root).unwrap().translation.y, -0.85);
        }
    }
    #[test]
    fn completion_returns_but_drag_and_external_relocation_discard_the_checkpoint() {
        let (mut world, _, _, _) = fixture();
        world.run_system_once(extract_motion).unwrap();
        world
            .resource_mut::<NeraAnimationState>()
            .advance(Duration::from_secs(2));
        world.run_system_once(extract_motion).unwrap();
        assert!(
            world
                .resource::<LibraryMotion>()
                .session
                .as_ref()
                .unwrap()
                .returning
                .is_some()
        );
        world.resource_mut::<DesktopPetInteraction>().state = InteractionState::Dragging;
        world.run_system_once(cancel_interrupted).unwrap();
        assert!(!world.resource::<LibraryMotion>().owns_placement());
        let (mut world, _, _, _) = fixture();
        world.run_system_once(extract_motion).unwrap();
        world
            .resource_mut::<DesktopEnvironment>()
            .nera_rect
            .as_mut()
            .unwrap()
            .left += 80;
        world.run_system_once(cancel_interrupted).unwrap();
        assert!(!world.resource::<LibraryMotion>().owns_placement());
        assert!(!matches!(
            world.resource::<NeraAnimationState>().visual_state(),
            VisualState::LibraryAnimation(_)
        ));
    }
    #[test]
    fn seated_checkpoint_and_support_survive_changing_the_manual_clip() {
        let (mut world, _, _, _) = fixture();
        let placement = PlacementState::Taskbar(crate::environment::TaskbarAttachment {
            monitor: crate::platform::NativeMonitorHandle::from_raw(1),
            edge: crate::environment::TaskbarEdge::Bottom,
            along_offset: 600,
        });
        let mut environment = world.resource_mut::<DesktopEnvironment>();
        environment.placement = placement;
        environment.attached_anchor = AvatarAnchor::Seat;
        environment.seat_anchor_logical = Some(Vec2::new(300.0, 700.0));
        world.run_system_once(extract_motion).unwrap();
        world
            .resource_mut::<DesktopEnvironment>()
            .nera_rect
            .as_mut()
            .unwrap()
            .left += 90;
        let another = world.spawn_empty().id();
        world.resource_mut::<NeraAnimationPlayback>().current_clip = Some(another);
        world.resource_mut::<NeraAnimationState>().request_one_shot(
            TemporaryAction::LibraryAnimation(another),
            Duration::from_secs(2),
        );
        world.run_system_once(extract_motion).unwrap();
        let session = world.resource::<LibraryMotion>().session.as_ref().unwrap();
        assert_eq!(session.checkpoint.position, IVec2::new(300, 0));
        assert_eq!(session.clip_origin, IVec2::new(390, 0));
        assert_eq!(session.checkpoint.placement, placement);
        assert_eq!(session.checkpoint.anchor, AvatarAnchor::Seat);
        assert_eq!(session.checkpoint.contact, Vec2::new(300.0, 700.0));
    }
    #[test]
    fn auto_dance_does_not_acquire_manual_motion_ownership_and_replacement_clears_raw_pose() {
        let (mut world, _, hips, entity) = fixture();
        world.run_system_once(extract_motion).unwrap();
        clear(&mut world);
        assert_eq!(
            world.get::<Transform>(hips).unwrap().translation,
            Vec3::Y * 0.9
        );
        world
            .resource_mut::<NeraAnimationState>()
            .finish_action(TemporaryAction::LibraryAnimation(entity));
        assert!(
            world
                .resource_mut::<NeraAnimationState>()
                .request_auto_dance(entity, Duration::from_secs(2))
        );
        world.run_system_once(extract_motion).unwrap();
        assert!(!world.resource::<LibraryMotion>().owns_placement());
    }
    #[test]
    fn visible_bounds_handle_negative_monitors_dpi_and_oversized_rigs() {
        let monitor = PhysicalRect {
            left: -1920,
            top: -100,
            right: 0,
            bottom: 980,
        };
        for dpi in [1.0, 1.25, 2.0] {
            let min = Vec2::new(200.0, 180.0) * dpi;
            let max = Vec2::new(400.0, 450.0) * dpi;
            let result = clamp_position(IVec2::new(2000, 2000), min, max, monitor).as_vec2();
            assert!(result.x + max.x <= monitor.right as f32 + 1.0);
            assert!(result.y + max.y <= monitor.bottom as f32 + 1.0);
        }
        assert_eq!(
            clamp_position(IVec2::ONE, Vec2::NAN, Vec2::ONE, monitor),
            IVec2::ONE
        );
    }
}
