//! Temporary seat ownership, independent of native desktop attachments.
mod bounds;
mod pose;
mod sleep_arms;
#[cfg(debug_assertions)]
mod smoke;
mod visual;
pub(crate) use pose::CloudSeatPose;
#[cfg(debug_assertions)]
pub(crate) use smoke::install as install_smoke;
#[cfg(test)]
mod tests;

use crate::{
    animation::{NeraAnimationState, SittingPose, VisualState},
    avatar::NeraAvatar,
    desktop_physics::DesktopPhysicsState,
    environment::{AvatarAnchor, DesktopEnvironment},
    interaction::{DesktopPetInteraction, InteractionState},
    pet_feedback::{PetFx, PetFxAnchor, PetFxKind, PetSfx, PetSfxKind},
    presentation::PresentationState,
};
use bevy::{prelude::*, window::PrimaryWindow};
use bevy_vrm1::prelude::Initialized;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloudMode {
    Rest,
    Ride,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestPose {
    Lying,
    Seated,
}

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloudCommand {
    Summon(CloudMode),
    Dismiss,
}

#[derive(Clone, Copy, Debug)]
struct CloudAttachment {
    mode: CloudMode,
    avatar: Entity,
    visual: Entity,
    /// Physical desktop pixels. Visual bobbing never changes this point.
    seat: Vec2,
}

#[derive(Clone, Copy, Debug, Default)]
enum CloudState {
    #[default]
    Hidden,
    Appearing {
        attachment: CloudAttachment,
        elapsed: f32,
    },
    Attached(CloudAttachment),
    Disappearing {
        visual: Entity,
        seat: Vec2,
        remaining: f32,
    },
}

const APPEAR_SECONDS: f32 = 0.24;
const DISAPPEAR_SECONDS: f32 = 0.25;

#[derive(Resource, Default, Debug)]
pub(crate) struct CloudRuntime {
    state: CloudState,
    sleep_in: f32,
    last_rest_pose: Option<RestPose>,
}

impl CloudRuntime {
    #[cfg(test)]
    pub(crate) fn for_test(mode: CloudMode, avatar: Entity, visual: Entity) -> Self {
        let mut cloud = Self::default();
        cloud.summon(mode, avatar, visual, Vec2::new(300.0, 400.0));
        cloud
    }

    fn attachment(&self) -> Option<CloudAttachment> {
        match self.state {
            CloudState::Appearing { attachment, .. } | CloudState::Attached(attachment) => {
                Some(attachment)
            }
            _ => None,
        }
    }
    pub(crate) fn mode(&self) -> Option<CloudMode> {
        self.attachment().map(|a| a.mode)
    }
    pub(crate) fn owns_placement(&self) -> bool {
        self.attachment().is_some()
    }
    pub(crate) fn is_riding(&self) -> bool {
        self.mode() == Some(CloudMode::Ride)
    }
    pub(crate) fn rest_pose(&self) -> Option<RestPose> {
        self.last_rest_pose
            .filter(|_| self.mode() == Some(CloudMode::Rest))
    }
    pub(crate) fn is_lying_rest(&self) -> bool {
        self.rest_pose() == Some(RestPose::Lying)
    }
    pub(crate) fn contact_world(&self, hips: &GlobalTransform, sleeping: bool) -> Vec3 {
        if self.mode() == Some(CloudMode::Rest) && sleeping {
            let scale = hips.to_scale_rotation_translation().0.abs().max_element();
            hips.translation() - Vec3::Y * 0.10 * scale
        } else {
            crate::avatar::seat_contact_world(hips)
        }
    }
    fn visual(&self) -> Option<Entity> {
        match self.state {
            CloudState::Disappearing { visual, .. } => Some(visual),
            _ => self.attachment().map(|a| a.visual),
        }
    }
    fn summon(&mut self, mode: CloudMode, avatar: Entity, visual: Entity, seat: Vec2) {
        if mode == CloudMode::Rest {
            // Like desktop sitting, choose once per attachment and avoid repeats.
            self.last_rest_pose = Some(match self.last_rest_pose {
                Some(RestPose::Lying) => RestPose::Seated,
                _ => RestPose::Lying,
            });
        }
        self.state = CloudState::Appearing {
            attachment: CloudAttachment {
                mode,
                avatar,
                visual,
                seat,
            },
            elapsed: 0.0,
        };
        self.sleep_in = 3.0;
    }
    fn dismiss(&mut self) -> Option<CloudAttachment> {
        let attachment = self.attachment()?;
        // Attachment ownership ends immediately, not at the end of the visual fade.
        self.state = CloudState::Disappearing {
            visual: attachment.visual,
            seat: attachment.seat,
            remaining: DISAPPEAR_SECONDS,
        };
        Some(attachment)
    }
    fn begin_drag(&mut self) -> Option<CloudAttachment> {
        if self.mode() == Some(CloudMode::Rest) {
            self.dismiss()
        } else {
            None
        }
    }
    fn advance(&mut self, dt: f32) -> Option<Entity> {
        match self.state {
            CloudState::Appearing {
                attachment,
                elapsed,
            } => {
                let elapsed = elapsed + dt;
                self.state = if elapsed >= APPEAR_SECONDS {
                    CloudState::Attached(attachment)
                } else {
                    CloudState::Appearing {
                        attachment,
                        elapsed,
                    }
                };
            }
            CloudState::Disappearing {
                visual,
                seat,
                remaining,
            } => {
                let remaining = remaining - dt;
                if remaining <= 0.0 {
                    self.state = CloudState::Hidden;
                    return Some(visual);
                }
                self.state = CloudState::Disappearing {
                    visual,
                    seat,
                    remaining,
                };
            }
            _ => {}
        }
        None
    }
}

pub(crate) struct CloudPlugin;
impl Plugin for CloudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CloudRuntime>()
            .init_resource::<visual::CloudVisualAssets>()
            .init_resource::<pose::CloudSeatPose>()
            .init_resource::<pose::SleepDirection>()
            .init_resource::<sleep_arms::SleepingArms>()
            .add_message::<CloudCommand>();
        app.add_systems(
            PreUpdate,
            (pose::restore_sleep_direction, sleep_arms::restore),
        )
        // Chibi removes its contribution in PreUpdate. Restore the cloud layer
        // afterward, before any new animation evaluation, including sparse clips.
        .add_systems(
            Update,
            pose::restore_raw_seat_pose
                .before(crate::animation::sync_interaction_animation_actions),
        )
        .add_systems(
            PostUpdate,
            pose::orient_sleeping_hips
                .after(bevy::app::AnimationSystems)
                .after(crate::avatar::preserve_seated_hips)
                .before(pose::stabilize_cloud_pose),
        );
        app.add_systems(
            PostUpdate,
            sleep_arms::apply
                .after(bevy::app::AnimationSystems)
                .after(pose::orient_sleeping_hips)
                .before(bevy_vrm1::prelude::VrmSystemSets::Constraints),
        );
        app.add_systems(
            PostUpdate,
            pose::stabilize_cloud_pose
                // The cached cloud seat is an animation pose. Proportions are
                // applied afterward, so toggling cannot rescale a cached pose.
                .before(crate::avatar::body_mode::BodyProportionSet::Apply)
                .after(bevy::app::AnimationSystems)
                .after(crate::avatar::preserve_seated_hips)
                .before(bevy_vrm1::prelude::VrmSystemSets::Constraints),
        );
        #[cfg(target_os = "windows")]
        app.add_systems(
            Update,
            menu_commands
                .after(crate::action_menu::poll_action_menu_result)
                .before(update_cloud),
        );
    }
}

#[cfg(target_os = "windows")]
fn menu_commands(
    mut menu: MessageReader<crate::action_menu::NeraActionMenuCommand>,
    mut commands: MessageWriter<CloudCommand>,
) {
    for command in menu.read() {
        if let crate::action_menu::NeraActionMenuCommand::Cloud(command) = command {
            commands.write(*command);
        }
    }
}

fn release_placement(
    environment: &mut DesktopEnvironment,
    physics: &mut DesktopPhysicsState,
    animation: &mut NeraAnimationState,
) {
    environment.begin_nera_drag();
    environment.set_attached_anchor(AvatarAnchor::Feet);
    physics.reset_for_position_recovery();
    animation.cancel_physics_motion();
    animation.restore_support_after_attention(false);
}

fn disappearance(
    attachment: CloudAttachment,
    sfx: &mut MessageWriter<PetSfx>,
    fx: &mut MessageWriter<PetFx>,
) {
    sfx.write(PetSfx(PetSfxKind::CloudDisappear));
    if attachment.mode == CloudMode::Rest {
        sfx.write(PetSfx(PetSfxKind::Wake));
    }
    fx.write(PetFx {
        kind: PetFxKind::CloudPuff,
        anchor: PetFxAnchor::Desktop(attachment.seat),
    });
}

pub(crate) fn dismiss_cloud(
    cloud: &mut CloudRuntime,
    environment: &mut DesktopEnvironment,
    physics: &mut DesktopPhysicsState,
    animation: &mut NeraAnimationState,
    sfx: &mut MessageWriter<PetSfx>,
    fx: &mut MessageWriter<PetFx>,
) {
    if let Some(attachment) = cloud.dismiss() {
        release_placement(environment, physics, animation);
        disappearance(attachment, sfx, fx);
    }
}

/// Called at the actual Pressed -> Dragging edge, before native movement or sitting probes.
pub(crate) fn begin_cloud_drag(
    cloud: &mut CloudRuntime,
    environment: &mut DesktopEnvironment,
    physics: &mut DesktopPhysicsState,
    animation: &mut NeraAnimationState,
    sfx: &mut MessageWriter<PetSfx>,
    fx: &mut MessageWriter<PetFx>,
) {
    if let Some(attachment) = cloud.begin_drag() {
        release_placement(environment, physics, animation);
        disappearance(attachment, sfx, fx);
    }
}

pub(crate) fn update_cloud(
    time: Res<Time<Real>>,
    presentation: Res<PresentationState>,
    mut requests: MessageReader<CloudCommand>,
    mut cloud: ResMut<CloudRuntime>,
    mut environment: ResMut<DesktopEnvironment>,
    mut physics: ResMut<DesktopPhysicsState>,
    mut animation: ResMut<NeraAnimationState>,
    interaction: Res<DesktopPetInteraction>,
    avatars: Query<Entity, (With<NeraAvatar>, With<Initialized>)>,
    windows: Query<&Window, With<PrimaryWindow>>,
    visuals: Query<(), With<visual::CloudVisual>>,
    mut assets: visual::CloudAssets,
    mut commands: Commands,
    mut sfx: MessageWriter<PetSfx>,
    mut fx: MessageWriter<PetFx>,
) {
    let visible = windows.iter().any(|w| w.visible);
    let interrupted = !presentation.is_desktop_pet()
        || !visible
        || cloud
            .attachment()
            .is_some_and(|a| avatars.get(a.avatar).is_err() || visuals.get(a.visual).is_err())
        || animation.death_active();
    let playback_exit = cloud.owns_placement()
        && matches!(
            animation.visual_state(),
            VisualState::LibraryAnimation(_) | VisualState::MenuAnimation(_)
        );
    if interrupted || playback_exit {
        if let Some(attachment) = cloud.dismiss() {
            release_placement(&mut environment, &mut physics, &mut animation);
            disappearance(attachment, &mut sfx, &mut fx);
        }
    }
    // Only the final command in one update matters, bounding native/menu bursts.
    for request in requests.read().last().into_iter() {
        match *request {
            CloudCommand::Dismiss => {
                if let Some(attachment) = cloud.dismiss() {
                    release_placement(&mut environment, &mut physics, &mut animation);
                    disappearance(attachment, &mut sfx, &mut fx);
                }
            }
            CloudCommand::Summon(mode)
                if !interrupted && interaction.state != InteractionState::Dragging =>
            {
                let Ok(avatar) = avatars.single() else {
                    continue;
                };
                if cloud.mode() == Some(mode) {
                    continue;
                }
                let Some(seat) = environment
                    .seat_probe_global()
                    .map(|p| Vec2::new(p.x as f32, p.y as f32))
                else {
                    continue;
                };
                if let Some(old) = cloud.visual() {
                    commands.entity(old).try_despawn();
                }
                let visual = assets.visuals.spawn(
                    &assets.server,
                    &mut assets.meshes,
                    &mut assets.materials,
                    &mut commands,
                );
                environment.begin_nera_drag();
                physics.reset_for_position_recovery();
                animation.enter_cloud_seat(match mode {
                    CloudMode::Rest => SittingPose::Sitting4,
                    CloudMode::Ride => SittingPose::Default,
                });
                environment.set_attached_anchor(AvatarAnchor::Seat);
                cloud.summon(mode, avatar, visual, seat);
                sfx.write(PetSfx(PetSfxKind::CloudAppear));
                if mode == CloudMode::Rest {
                    sfx.write(PetSfx(PetSfxKind::Sleep));
                }
                fx.write(PetFx {
                    kind: PetFxKind::CloudPuff,
                    anchor: PetFxAnchor::Desktop(seat),
                });
            }
            _ => {}
        }
    }
    if let Some(expired) = cloud.advance(time.delta_secs()) {
        commands.entity(expired).try_despawn();
    }
    if cloud.mode() == Some(CloudMode::Rest) && animation.visual_state() == VisualState::SittingLoop
    {
        cloud.sleep_in -= time.delta_secs();
        if cloud.sleep_in <= 0.0 {
            cloud.sleep_in = 6.0;
            fx.write(PetFx {
                kind: PetFxKind::SleepZ,
                anchor: PetFxAnchor::Head,
            });
        }
    }
}

/// Pose-first native alignment; the evaluated seat is read after avatar anchor sampling.
pub(crate) fn align_cloud_seat(
    mut cloud: ResMut<CloudRuntime>,
    mut environment: ResMut<DesktopEnvironment>,
    interaction: Res<DesktopPetInteraction>,
    presentation: Res<PresentationState>,
    settings: Res<crate::settings::NeraSettings>,
    camera: Query<(&Camera, &GlobalTransform), With<crate::avatar::NeraMainCamera>>,
    children: Query<&Children>,
    meshes: Query<(&bevy::camera::primitives::Aabb, &GlobalTransform), With<Mesh3d>>,
    visuals: Query<(&visual::CloudVisual, &GlobalTransform)>,
) {
    if !presentation.is_desktop_pet() {
        return;
    }
    let Some(mut attachment) = cloud.attachment() else {
        return;
    };
    let dragging =
        interaction.state == InteractionState::Dragging && attachment.mode == CloudMode::Ride;
    if dragging {
        if let Some(seat) = environment.seat_probe_global() {
            attachment.seat = Vec2::new(seat.x as f32, seat.y as f32);
        }
        // Interaction moved the HWND after the First snapshot. Refresh the
        // existing monitor fact before clamping so a drag can cross monitors.
        if let Some(monitor) = environment
            .nera_hwnd
            .and_then(crate::platform::monitor_info)
        {
            environment.monitor = Some(monitor);
        }
    }
    let current_seat = environment
        .seat_probe_global()
        .map(|p| Vec2::new(p.x as f32, p.y as f32))
        .unwrap_or(attachment.seat);
    // During a drag the pointer owns HWND movement. A second correction here
    // fights it every frame. On release keep the head/sides and logical seat
    // on the monitor, allowing feet/cloud to overlap the taskbar. Animated
    // shoes are not a moving bottom barrier that can push the mount upward.
    if !dragging
        && attachment.mode == CloudMode::Ride
        && let (Some(monitor), Ok((camera, view))) = (environment.monitor, camera.single())
        && let Some(visible) = bounds::project_mounted_bounds(
            attachment.avatar,
            attachment.visual,
            current_seat,
            &environment,
            camera,
            view,
            settings.character_scale(),
            &children,
            &meshes,
            &visuals,
        )
    {
        // Avatar bounds were sampled at the current pose/window, so translate
        // them to the cloud's intended logical placement before clamping.
        let visible = visible.translated(attachment.seat - current_seat);
        attachment.seat += bounds::ride_correction(visible, attachment.seat, monitor.bounds);
    }
    match &mut cloud.state {
        CloudState::Appearing {
            attachment: stored, ..
        }
        | CloudState::Attached(stored) => *stored = attachment,
        _ => {}
    }
    let (Some(hwnd), Some(rect), Some(seat)) = (
        environment.nera_hwnd,
        environment.nera_rect,
        environment.seat_probe_global(),
    ) else {
        return;
    };
    let delta = attachment.seat.round().as_ivec2() - IVec2::new(seat.x, seat.y);
    if delta != IVec2::ZERO
        && crate::platform::move_window(
            hwnd,
            crate::platform::PhysicalPoint::new(
                rect.left.saturating_add(delta.x),
                rect.top.saturating_add(delta.y),
            ),
        )
        .is_ok()
    {
        environment.nera_rect = crate::platform::window_rect(hwnd);
    }
}

pub(crate) use pose::stabilize_cloud_pose;
pub(crate) use visual::CloudVisual;
pub(crate) use visual::animate_cloud_visual;
pub(crate) use visual::cloud_hit;

/// Character commit calls this before destroying the old rig. Also safe in isolated tests.
pub(crate) fn clear_for_character_change(world: &mut World) {
    let Some(cloud) = world.get_resource::<CloudRuntime>() else {
        return;
    };
    let visual = cloud.visual();
    let attached = cloud.owns_placement();
    world.insert_resource(CloudRuntime::default());
    if let Some(visual) = visual {
        world.despawn(visual);
    }
    if attached {
        world.resource_scope(|world, mut environment: Mut<DesktopEnvironment>| {
            world.resource_scope(|world, mut physics: Mut<DesktopPhysicsState>| {
                release_placement(
                    &mut environment,
                    &mut physics,
                    &mut world.resource_mut::<NeraAnimationState>(),
                );
            });
        });
    }
}
