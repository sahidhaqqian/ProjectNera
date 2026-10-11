//! Two interruptible routines of the existing low-priority pet brain.
mod pose;
#[cfg(all(debug_assertions, target_os = "windows"))]
pub(crate) mod smoke;
#[cfg(test)]
mod tests;

use super::*;
use crate::{
    avatar::NeraMainCamera,
    interaction::NeraLiveBone,
    pet_feedback::{PetFx, PetFxAnchor, PetFxKind, PetSfx, PetSfxKind},
};
use bevy_vrm1::prelude::VrmBone;
use std::time::Duration;

/// Read-only cursor fact, shared with gaze. Debug injection never moves the OS pointer.
#[derive(Resource, Default)]
pub(crate) struct LivingPointer {
    pub point: Option<Vec2>,
    #[cfg(debug_assertions)]
    pub scripted: Option<Option<Vec2>>,
}

pub(crate) struct LivingPlugin;
impl Plugin for LivingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LivingPointer>();
        pose::install(app);
    }
}

pub(crate) fn clear_for_character_change(world: &mut World) {
    pose::clear(world);
    if !world.contains_resource::<PetBehavior>() {
        return;
    }
    if let Some(edge) = world.resource::<PetBehavior>().living.edge
        && world.contains_resource::<DesktopPhysicsState>()
        && world.contains_resource::<DesktopEnvironment>()
        && world
            .get_resource::<PresentationState>()
            .is_none_or(|p| p.is_desktop_pet())
        && world
            .get_resource::<crate::cloud::CloudRuntime>()
            .is_none_or(|c| !c.owns_placement())
    {
        let expected = world
            .get_resource::<WalkingRuntime>()
            .and_then(|w| w.expected_x())
            .map(|x| x + edge.work.left)
            .unwrap_or(edge.expected);
        world.resource_scope(|world, mut physics: Mut<DesktopPhysicsState>| {
            let mut env = world.resource_mut::<DesktopEnvironment>();
            if env
                .nera_rect
                .is_some_and(|r| r.left == expected && r.width() == edge.width)
                && let Some(surface) = physics.walking_surface(&env)
            {
                physics.walk_horizontal(
                    edge.home.clamp(surface.bounds.0, surface.bounds.1),
                    &mut env,
                );
            }
        });
    }
    world.resource_scope(|world, mut brain: Mut<PetBehavior>| {
        brain
            .living
            .interrupt(&mut world.resource_mut::<NeraAnimationState>());
        brain.living = Default::default();
        brain.home = None;
    });
    if !world.contains_resource::<WalkingRuntime>() {
        return;
    }
    world.resource_scope(|world, mut walking: Mut<WalkingRuntime>| {
        walking.cancel(&mut world.resource_mut::<NeraAnimationState>())
    });
}

pub(crate) fn sample_pointer(mut pointer: ResMut<LivingPointer>) {
    #[cfg(debug_assertions)]
    if let Some(point) = pointer.scripted {
        pointer.point = point;
        return;
    }
    pointer.point = platform::global_cursor_position().map(|p| Vec2::new(p.x as f32, p.y as f32));
}

#[derive(Clone, Copy, Debug)]
struct Arm {
    upper: Entity,
    lower: Entity,
    hand: Entity,
    shoulder: Vec2,
    reach: f32,
}

#[derive(SystemParam)]
pub(crate) struct LivingContext<'w, 's> {
    pointer: Res<'w, LivingPointer>,
    bones: Query<'w, 's, (Entity, &'static VrmBone, &'static GlobalTransform), With<NeraLiveBone>>,
    camera: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<NeraMainCamera>>,
    fx: MessageWriter<'w, PetFx>,
    sfx: MessageWriter<'w, PetSfx>,
}

impl LivingContext<'_, '_> {
    fn arms(&self, environment: &DesktopEnvironment) -> Vec<Arm> {
        let (Some(rect), Ok((camera, view))) = (environment.nera_rect, self.camera.single()) else {
            return vec![];
        };
        let screen = |p| {
            camera.world_to_viewport(view, p).ok().map(|p| {
                Vec2::new(rect.left as f32, rect.top as f32)
                    + p * environment.nera_scale_factor as f32
            })
        };
        let mut arms = vec![];
        for side in ["left", "right"] {
            let find = |suffix| {
                self.bones
                    .iter()
                    .find(|(_, bone, _)| bone.as_str() == format!("{side}{suffix}"))
            };
            let (Some((upper, _, u)), Some((lower, _, l)), Some((hand, _, h))) =
                (find("UpperArm"), find("LowerArm"), find("Hand"))
            else {
                continue;
            };
            let (Some(shoulder), Some(elbow), Some(wrist)) = (
                screen(u.translation()),
                screen(l.translation()),
                screen(h.translation()),
            ) else {
                continue;
            };
            let reach = shoulder.distance(elbow) + elbow.distance(wrist);
            if reach.is_finite() && reach > 15.0 && reach < rect.height() as f32 * 0.65 {
                arms.push(Arm {
                    upper,
                    lower,
                    hand,
                    shoulder,
                    reach,
                });
            }
        }
        arms
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CursorPhase {
    Notice,
    Reach,
    Catch,
    Escape,
    Release,
}
impl CursorPhase {
    fn action(self) -> IdleMicroAction {
        match self {
            Self::Notice => IdleMicroAction::CursorNotice,
            Self::Reach | Self::Release => IdleMicroAction::CursorReach,
            Self::Catch => IdleMicroAction::CursorCatch,
            Self::Escape => IdleMicroAction::CursorEscape,
        }
    }
    fn duration(self) -> f32 {
        match self {
            Self::Notice => 0.75,
            Self::Reach => 1.1,
            Self::Catch => 0.55,
            Self::Escape => 0.85,
            Self::Release => 0.65,
        }
    }
}

#[derive(Clone, Copy)]
struct CursorRoutine {
    arm: Arm,
    phase: CursorPhase,
    elapsed: f32,
    target: Vec2,
    release_from: f32,
}

#[derive(Clone, Copy)]
pub(crate) struct CursorPose {
    pub upper: Entity,
    pub lower: Entity,
    pub hand: Entity,
    pub target: Vec2,
    pub amount: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EdgePhase {
    Approach,
    Hide,
    Reveal,
    Return,
}
#[derive(Clone, Copy)]
struct EdgeRoutine {
    monitor: NativeMonitorHandle,
    home: i32,
    hidden: i32,
    visible: i32,
    phase: EdgePhase,
    wait: f32,
    expected: i32,
    recovery: bool,
    width: i32,
    work: PhysicalRect,
    check_in: f32,
}

pub(crate) struct LivingBehavior {
    cursor: Option<CursorRoutine>,
    edge: Option<EdgeRoutine>,
    dwell: f32,
    cursor_in: f32,
    edge_in: f32,
    rng: NeraRng,
}
impl Default for LivingBehavior {
    fn default() -> Self {
        Self {
            cursor: None,
            edge: None,
            dwell: 0.0,
            cursor_in: 8.0,
            edge_in: 90.0,
            rng: NeraRng::from_entropy(),
        }
    }
}

fn is_living(action: IdleMicroAction) -> bool {
    IdleMicroAction::LIVING.contains(&action)
}
fn phase_animation(animation: &mut NeraAnimationState, phase: CursorPhase) -> bool {
    if matches!(animation.visual_state(), VisualState::IdleMicro(action) if is_living(action)) {
        animation.cancel_idle_variation();
    }
    animation.request_idle_variation(
        phase.action(),
        Duration::from_secs_f32(phase.duration() + 0.15),
    )
}

impl LivingBehavior {
    pub(super) fn blocks_position_save(&self) -> bool {
        self.edge.is_some()
    }
    pub(super) fn gaze(&self) -> Option<Vec2> {
        self.cursor.map(|c| c.target)
    }
    pub(super) fn cursor_pose(&self) -> Option<CursorPose> {
        let c = self.cursor?;
        let amount = match c.phase {
            CursorPhase::Notice => 0.0,
            CursorPhase::Reach => smooth(c.elapsed / 0.65),
            CursorPhase::Catch => 1.0 - 0.12 * smooth(c.elapsed / c.phase.duration()),
            CursorPhase::Escape | CursorPhase::Release => {
                c.release_from * (1.0 - smooth(c.elapsed / c.phase.duration()))
            }
        };
        Some(CursorPose {
            upper: c.arm.upper,
            lower: c.arm.lower,
            hand: c.arm.hand,
            target: c.target,
            amount,
        })
    }
    pub(super) fn interrupt(&mut self, animation: &mut NeraAnimationState) {
        self.cursor = None;
        self.dwell = 0.0;
        self.cursor_in = self.cursor_in.max(8.0);
        if let Some(edge) = &mut self.edge {
            edge.recovery = true;
        }
        if matches!(animation.visual_state(), VisualState::IdleMicro(action) if is_living(action)) {
            animation.cancel_idle_variation();
        }
    }

    /// Attention restores its own rect first. Cloud/user relocation invalidates
    /// the journey; a normal interruption at our last position safely reveals it.
    pub(super) fn recover(
        &mut self,
        context: &BehaviorContext,
        physics: &mut DesktopPhysicsState,
        env: &mut DesktopEnvironment,
        walking: &mut WalkingRuntime,
        animation: &mut NeraAnimationState,
    ) {
        let Some(mut edge) = self.edge else {
            return;
        };
        if !context.settings.edge_peek || !context.settings.walking_enabled {
            edge.recovery = true;
        }
        if !edge.recovery {
            self.edge = Some(edge);
            return;
        }
        if !context.presentation.is_desktop_pet() {
            return;
        }
        // Keep the grab point stable while click-vs-drag discrimination is
        // unresolved. Release may reveal; a real drag invalidates recovery.
        if context.interaction.state == InteractionState::Pressed
            || context.interaction.right_pressed_body_part.is_some()
        {
            return;
        }
        let Some(rect) = env.nera_rect else {
            return;
        };
        let expected = walking
            .expected_x()
            .map(|x| x + edge.work.left)
            .unwrap_or(edge.expected);
        walking.cancel(animation);
        if context.interaction.state != InteractionState::Dragging
            && env.placement == PlacementState::Free
            && !physics.falling
            && !context.cloud.as_ref().is_some_and(|c| c.owns_placement())
            && env.monitor.is_some_and(|m| m.handle == edge.monitor)
            && rect.width() == edge.width
            && rect.left == expected
            && let Some(surface) = physics.walking_surface(env)
        {
            // A single inward recovery uses the same checked physics movement.
            physics.walk_horizontal(edge.home.clamp(surface.bounds.0, surface.bounds.1), env);
        }
        self.edge = None;
        self.edge_in = self.rng.range_f32(90.0, 180.0);
    }

    pub(super) fn tick(
        &mut self,
        dt: f32,
        settings: &NeraSettings,
        clips: &NeraAnimationClips,
        context: &mut LivingContext,
        physics: &mut DesktopPhysicsState,
        env: &mut DesktopEnvironment,
        walking: &mut WalkingRuntime,
        animation: &mut NeraAnimationState,
    ) -> bool {
        self.cursor_in = (self.cursor_in - dt).max(0.0);
        self.edge_in = (self.edge_in - dt).max(0.0);
        if self.cursor.is_some() {
            if !settings.cursor_play || !settings.follow_mouse {
                self.interrupt(animation);
                return false;
            }
            return self.tick_cursor(dt, context, animation);
        }
        if self.edge.is_some() {
            return self.tick_edge(dt, clips, physics, env, walking, animation);
        }
        if animation.visual_state() != VisualState::Idle || walking.active() {
            self.dwell = 0.0;
            return false;
        }
        if settings.cursor_play && settings.follow_mouse && self.cursor_in <= 0.0 {
            let arm = context.pointer.point.and_then(|point| {
                context
                    .arms(env)
                    .into_iter()
                    .filter(|a| point.distance(a.shoulder) < a.reach * 1.15)
                    .min_by(|a, b| {
                        a.shoulder
                            .distance_squared(point)
                            .total_cmp(&b.shoulder.distance_squared(point))
                    })
            });
            if let (Some(arm), Some(target)) = (arm, context.pointer.point) {
                self.dwell += dt;
                if self.dwell >= 0.8 && phase_animation(animation, CursorPhase::Notice) {
                    self.cursor = Some(CursorRoutine {
                        arm,
                        phase: CursorPhase::Notice,
                        elapsed: 0.0,
                        target,
                        release_from: 0.0,
                    });
                    self.cursor_in = self.rng.range_f32(22.0, 40.0);
                    info!("Living Cursor Play: noticed nearby pointer");
                    return true;
                }
            } else {
                self.dwell = 0.0;
            }
        }
        if settings.edge_peek
            && settings.walking_enabled
            && self.edge_in <= 0.0
            && clips.walking_ready()
        {
            self.edge_in = self.rng.range_f32(90.0, 180.0);
            if let Some(edge) = plan_edge(physics, env, clips)
                && begin_edge_walk(edge.hidden, edge, clips, physics, env, walking, animation)
            {
                self.edge = Some(edge);
                info!("Living Edge Peek: walking to exposed monitor edge");
                return true;
            }
        }
        false
    }

    fn tick_cursor(
        &mut self,
        dt: f32,
        context: &mut LivingContext,
        animation: &mut NeraAnimationState,
    ) -> bool {
        let mut c = self.cursor.unwrap();
        if animation.visual_state() != VisualState::IdleMicro(c.phase.action()) {
            self.interrupt(animation);
            return false;
        }
        c.elapsed += dt;
        let escaped = context.pointer.point.is_none_or(|p| {
            p.distance(c.arm.shoulder) > c.arm.reach * 1.45
                || p.distance(c.target) > c.arm.reach * 0.7
        });
        if matches!(c.phase, CursorPhase::Notice | CursorPhase::Reach) && !escaped {
            c.target = c
                .target
                .lerp(context.pointer.point.unwrap(), 1.0 - (-8.0 * dt).exp());
        }
        let next = if matches!(
            c.phase,
            CursorPhase::Notice | CursorPhase::Reach | CursorPhase::Catch
        ) && escaped
        {
            Some(CursorPhase::Escape)
        } else if c.elapsed >= c.phase.duration() {
            match c.phase {
                CursorPhase::Notice => Some(CursorPhase::Reach),
                CursorPhase::Reach => Some(CursorPhase::Catch),
                CursorPhase::Catch => Some(CursorPhase::Release),
                _ => None,
            }
        } else {
            self.cursor = Some(c);
            return true;
        };
        if let Some(next) = next {
            if !phase_animation(animation, next) {
                self.interrupt(animation);
                return false;
            }
            c.release_from = self.cursor_pose().map_or(0.0, |p| p.amount);
            c.phase = next;
            c.elapsed = 0.0;
            self.cursor = Some(c);
            if next == CursorPhase::Catch {
                context.fx.write(PetFx {
                    kind: PetFxKind::Heart,
                    anchor: PetFxAnchor::Head,
                });
                context.sfx.write(PetSfx(PetSfxKind::Happy));
            }
            info!("Living Cursor Play: {next:?}");
        } else {
            animation.cancel_idle_variation();
            self.cursor = None;
            self.dwell = 0.0;
        }
        true
    }

    fn tick_edge(
        &mut self,
        dt: f32,
        clips: &NeraAnimationClips,
        physics: &mut DesktopPhysicsState,
        env: &mut DesktopEnvironment,
        walking: &mut WalkingRuntime,
        animation: &mut NeraAnimationState,
    ) -> bool {
        let mut edge = self.edge.unwrap();
        if edge.recovery {
            return false;
        }
        let Some(rect) = env.nera_rect else {
            self.interrupt(animation);
            return false;
        };
        let expected = walking
            .expected_x()
            .map(|x| edge.work.left + x)
            .unwrap_or(edge.expected);
        if env
            .monitor
            .is_none_or(|m| m.handle != edge.monitor || m.work_area != edge.work)
            || rect.width() != edge.width
            || rect.left != expected
            || physics.peek_surface(env).is_none()
        {
            self.interrupt(animation);
            return false;
        }
        edge.expected = rect.left;
        edge.check_in -= dt;
        if edge.check_in <= 0.0 {
            edge.check_in = 0.25;
            if !exposed_edge(
                edge.work,
                rect,
                &platform::enumerate_monitors(),
                edge.monitor,
                edge.hidden < edge.visible,
            ) {
                self.interrupt(animation);
                return false;
            }
        }
        if walking.active() {
            self.edge = Some(edge);
            return true;
        }
        if let Some(outcome) = walking.take_outcome() {
            if outcome == WalkOutcome::Interrupted {
                self.interrupt(animation);
                return false;
            }
            match edge.phase {
                EdgePhase::Approach => {
                    edge.phase = EdgePhase::Hide;
                    edge.wait = 2.5;
                    animation.request_idle_variation(
                        IdleMicroAction::EdgePeek,
                        Duration::from_secs_f32(2.7),
                    );
                }
                EdgePhase::Reveal => {
                    edge.wait = 1.1;
                    animation.request_idle_variation(
                        IdleMicroAction::EdgePeek,
                        Duration::from_secs_f32(1.3),
                    );
                }
                EdgePhase::Return => {
                    self.edge = None;
                    info!("Living Edge Peek: returned home");
                    return true;
                }
                _ => {}
            }
        }
        edge.wait -= dt;
        if edge.wait <= 0.0 && matches!(edge.phase, EdgePhase::Hide | EdgePhase::Reveal) {
            animation.cancel_idle_variation();
            let target = if edge.phase == EdgePhase::Hide {
                edge.visible
            } else {
                edge.home
            };
            edge.phase = if edge.phase == EdgePhase::Hide {
                EdgePhase::Reveal
            } else {
                EdgePhase::Return
            };
            if !begin_edge_walk(target, edge, clips, physics, env, walking, animation) {
                self.interrupt(animation);
                return false;
            }
        }
        self.edge = Some(edge);
        true
    }
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn exposed_edge(
    work: PhysicalRect,
    rect: PhysicalRect,
    monitors: &[platform::MonitorInfo],
    handle: NativeMonitorHandle,
    left: bool,
) -> bool {
    let edge = if left { work.left } else { work.right };
    !monitors.iter().any(|m| {
        m.handle != handle
            && m.bounds.top < rect.bottom
            && m.bounds.bottom > rect.top
            && if left {
                m.bounds.left < edge && m.bounds.right >= edge
            } else {
                m.bounds.right > edge && m.bounds.left <= edge
            }
    })
}

fn plan_edge(
    physics: &DesktopPhysicsState,
    env: &DesktopEnvironment,
    clips: &NeraAnimationClips,
) -> Option<EdgeRoutine> {
    let surface = physics.peek_surface(env)?;
    let normal = physics.walking_surface(env)?;
    let monitor = env.monitor?;
    let rect = env.nera_rect?;
    let look = clips
        .idle_micro_duration(IdleMicroAction::LookAround)?
        .as_secs_f32();
    let speed = 120.0 * env.nera_scale_factor as f32;
    let max_distance = speed * (13.0 - look).max(0.0);
    let monitors = platform::enumerate_monitors();
    let (hidden, visible) = [
        (surface.bounds.0, normal.bounds.0, true),
        (surface.bounds.1, normal.bounds.1, false),
    ]
    .into_iter()
    .filter(|(x, _, left)| {
        (rect.left - x).unsigned_abs() as f32 <= max_distance
            && (if *left {
                monitor.work_area.left == monitor.bounds.left
            } else {
                monitor.work_area.right == monitor.bounds.right
            })
            && exposed_edge(monitor.work_area, rect, &monitors, monitor.handle, *left)
    })
    .min_by_key(|(x, _, _)| (rect.left - x).unsigned_abs())
    .map(|(hidden, visible, _)| (hidden, visible))?;
    Some(EdgeRoutine {
        monitor: monitor.handle,
        home: rect.left.clamp(normal.bounds.0, normal.bounds.1),
        hidden,
        visible,
        phase: EdgePhase::Approach,
        wait: 0.0,
        expected: rect.left,
        recovery: false,
        width: rect.width(),
        work: monitor.work_area,
        check_in: 0.25,
    })
}

fn begin_edge_walk(
    target: i32,
    edge: EdgeRoutine,
    clips: &NeraAnimationClips,
    physics: &DesktopPhysicsState,
    env: &DesktopEnvironment,
    walking: &mut WalkingRuntime,
    animation: &mut NeraAnimationState,
) -> bool {
    let (Some(surface), Some(rect), Some(duration)) = (
        physics.peek_surface(env),
        env.nera_rect,
        clips.idle_micro_duration(IdleMicroAction::LookAround),
    ) else {
        return false;
    };
    if rect.left == target {
        return false;
    }
    walking.begin(
        WalkIntent {
            surface: surface.key,
            monitor: edge.monitor,
            target_x: target - surface.origin_x,
            bounds: (
                surface.bounds.0 - surface.origin_x,
                surface.bounds.1 - surface.origin_x,
            ),
            speed: 120.0 * env.nera_scale_factor as f32,
            scale: env.nera_scale_factor as f32,
            return_trip: edge.phase != EdgePhase::Approach,
            edge_peek: true,
        },
        rect.left - surface.origin_x,
        0.25,
        animation,
        duration,
    )
}
