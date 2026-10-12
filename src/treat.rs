//! One hand-attached prop interaction; animation, placement and feedback keep
//! their existing owners. The supplied Drinking VRMA uses the LEFT hand.
use std::time::Duration;

use bevy::world_serialization::{WorldAsset, WorldAssetRoot};
use bevy::{ecs::system::SystemParam, prelude::*};
use bevy_vrm1::prelude::{RestGlobalTransform, VrmBone};

use crate::{
    animation::{NeraAnimationClips, NeraAnimationPlayback, NeraAnimationState, VisualState},
    avatar::NeraAvatar,
    cloud::{CloudMode, CloudRuntime},
    environment::{DesktopEnvironment, PlacementState},
    interaction::{DesktopPetInteraction, InteractionState, NeraLiveBone},
    pet_feedback::{PetFx, PetFxAnchor, PetFxKind, PetSfx, PetSfxKind},
    presentation::PresentationState,
    rng::NeraRng,
    walking::WalkingRuntime,
};

pub(crate) const DRINKING_PATH: &str = "animations/animasi_basic/Drinking.vrma";
#[cfg(debug_assertions)]
pub(crate) mod smoke;
const COOLDOWN: Duration = Duration::from_secs(3);
const LOAD_TIMEOUT: Duration = Duration::from_secs(10);
// Corrected Drinking.vrma: keep the item through the approach and wrist turn;
// consume at the completed sip/bite pose at the mouth, before hand recovery.
const CONSUME_AT: Duration = Duration::from_millis(4200);
// Stay above PetFx's existing 250ms admission cooldown.
const LOVE_INTERVAL: Duration = Duration::from_millis(350);

pub(crate) struct Food {
    pub(crate) name: &'static str,
    pub(crate) path: &'static str,
    center: Vec3,
    half_size: Vec3,
}

// Actual evaluated glTF scene bounds (including multi-part pizza/burger).
// These are asset geometry, never avatar-specific scale or placement offsets.
pub(crate) const FOODS: [Food; 7] = [
    Food {
        name: "Pizza",
        path: "props/treats/pizza.glb",
        center: Vec3::new(0.0, 0.021735, 0.0),
        half_size: Vec3::new(0.4209, 0.021735, 0.4209),
    },
    Food {
        name: "Apple",
        path: "props/treats/apple.glb",
        center: Vec3::new(0.0, 0.095377, 0.0),
        half_size: Vec3::new(0.0981475, 0.0953765, 0.0981475),
    },
    Food {
        name: "Hamburger",
        path: "props/treats/burger.glb",
        center: Vec3::new(0.0, 0.145, 0.0),
        half_size: Vec3::new(0.202048, 0.145, 0.202048),
    },
    Food {
        name: "Cookie",
        path: "props/treats/cookie.glb",
        center: Vec3::new(0.0, 0.024885, 0.0),
        half_size: Vec3::new(0.109872, 0.024885, 0.095152),
    },
    Food {
        name: "Donut",
        path: "props/treats/donut.glb",
        center: Vec3::new(0.0, 0.044446, 0.0),
        half_size: Vec3::new(0.1225295, 0.044446, 0.1414845),
    },
    Food {
        name: "Cupcake",
        path: "props/treats/cupcake.glb",
        center: Vec3::new(0.0, 0.2119, 0.0),
        half_size: Vec3::new(0.1775815, 0.2119, 0.15379),
    },
    Food {
        name: "Soda",
        path: "props/treats/soda-can.glb",
        center: Vec3::new(0.0, 0.175298, 0.0),
        half_size: Vec3::new(0.11232, 0.1752985, 0.11232),
    },
];

#[derive(Message, Default)]
pub(crate) struct RequestTreat;

#[derive(Resource, Default)]
struct TreatAssets(Vec<Handle<WorldAsset>>);

#[derive(Component)]
pub(crate) struct TreatProp;

struct Pending {
    avatar: Entity,
    at: Duration,
}
struct Active {
    avatar: Entity,
    prop: Option<Entity>,
    duration: Duration,
    last_remaining: Duration,
    love: Option<TreatLove>,
}

struct TreatLove {
    sides: [bool; 5],
    count: usize,
    emitted: usize,
    next_at: Duration,
}
impl TreatLove {
    fn new(at: Duration, rng: &mut NeraRng) -> Self {
        let count = rng.range_i32(2, 5) as usize;
        let mut sides = [false; 5];
        sides[0] = rng.range_i32(0, 1) != 0;
        sides[1] = !sides[0]; // Every burst shows both sides, in random order.
        for side in &mut sides[2..count] {
            *side = rng.range_i32(0, 1) != 0;
        }
        Self {
            sides,
            count,
            emitted: 0,
            next_at: at,
        }
    }
    fn pulse(&mut self, at: Duration) -> Option<bool> {
        if self.emitted == self.count || at < self.next_at {
            return None;
        }
        let side = self.sides[self.emitted];
        self.emitted += 1;
        // One pulse per update, with no catch-up burst after a stalled frame.
        self.next_at = at + LOVE_INTERVAL;
        Some(side)
    }
}
impl Active {
    fn consume_and_pulse(
        &mut self,
        remaining: Duration,
        playing: bool,
        rng: &mut NeraRng,
    ) -> (Option<Entity>, Option<bool>) {
        if !playing {
            return (None, None);
        }
        let elapsed = self.duration.saturating_sub(remaining);
        let consumed = if elapsed >= CONSUME_AT {
            self.prop.take()
        } else {
            None
        };
        if consumed.is_some() {
            self.love = Some(TreatLove::new(elapsed, rng));
        }
        let heart = self.love.as_mut().and_then(|love| love.pulse(elapsed));
        (consumed, heart)
    }
}

#[derive(Resource, Default)]
pub(crate) struct TreatRuntime {
    pending: Option<Pending>,
    active: Option<Active>,
    cooldown_until: Duration,
    last_food: Option<usize>,
    rng: NeraRng,
}
impl TreatRuntime {
    pub(crate) fn busy(&self) -> bool {
        self.pending.is_some() || self.active.is_some()
    }
    fn request(&mut self, avatar: Option<Entity>, now: Duration, allowed: bool) {
        if allowed && !self.busy() && now >= self.cooldown_until {
            if let Some(avatar) = avatar {
                self.pending = Some(Pending { avatar, at: now });
            }
        }
    }
    fn choose(&mut self, available: &[usize]) -> Option<usize> {
        let fresh: Vec<_> = available
            .iter()
            .copied()
            .filter(|index| available.len() == 1 || Some(*index) != self.last_food)
            .collect();
        let chosen = fresh.get(self.rng.pick_index(fresh.len())?).copied()?;
        self.last_food = Some(chosen);
        Some(chosen)
    }
}

pub(crate) struct TreatPlugin;
impl Plugin for TreatPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TreatRuntime>()
            .init_resource::<TreatAssets>()
            .add_message::<RequestTreat>()
            .add_systems(Startup, load_props)
            .add_systems(
                Update,
                update_treat
                    .after(crate::animation::tick_animation_actions)
                    .before(crate::animation::apply_animation_state),
            );
    }
}
fn load_props(server: Res<AssetServer>, mut assets: ResMut<TreatAssets>) {
    assets.0 = FOODS
        .iter()
        .map(|food| server.load(format!("{}#Scene0", food.path)))
        .collect();
}

#[derive(SystemParam)]
struct TreatContext<'w, 's> {
    presentation: Res<'w, PresentationState>,
    interaction: Res<'w, DesktopPetInteraction>,
    cloud: Res<'w, CloudRuntime>,
    entrance: Res<'w, crate::startup_entrance::StartupEntrance>,
    loading: Option<Res<'w, crate::character::CharacterImportStatus>>,
    environment: Res<'w, DesktopEnvironment>,
    seat: Option<Res<'w, crate::animation::touch::seated::TouchSeatPose>>,
    cloud_seat: Option<Res<'w, crate::cloud::CloudSeatPose>>,
    avatars: Query<'w, 's, Entity, With<NeraAvatar>>,
}
impl TreatContext<'_, '_> {
    fn allowed(&self, animation: &NeraAnimationState) -> bool {
        self.presentation.is_desktop_pet()
            && !self.entrance.blocks_position_save()
            && !self.loading.as_ref().is_some_and(|status| status.loading)
            && !self.environment.hidden_for_target_suspend
            && self.interaction.state != InteractionState::Dragging
            && self.interaction.drag_offset.is_none()
            && self.cloud.mode() != Some(CloudMode::Rest)
            && animation.can_request_touch_reaction()
            && if self.cloud.owns_placement() {
                self.cloud_seat
                    .as_ref()
                    .is_some_and(|seat| seat.is_ready_for(&self.cloud))
            } else if self.environment.placement != PlacementState::Free {
                self.seat.as_ref().is_some_and(|seat| {
                    seat.is_ready_for(
                        self.environment.placement,
                        animation.selected_sitting_pose(),
                    )
                })
            } else {
                true
            }
    }
}

/// Derive the palm basis and size from humanoid finger geometry, so a custom
/// rig's local bone axes are not assumed when placing the held prop.
fn grip(middle: Vec3, across: Vec3) -> Option<(Vec3, Quat, f32)> {
    let length = middle.length();
    let forward = middle.try_normalize()?;
    let up = forward.cross(across).try_normalize()?;
    let side = up.cross(forward).try_normalize()?;
    if !length.is_finite() || length < 1e-6 {
        return None;
    }
    let rotation = Quat::from_mat3(&Mat3::from_cols(side, up, forward)).normalize();
    Some((forward * length * 0.55, rotation, length * 2.0))
}

fn prop_transform(food: &Food, palm: Vec3, basis: Quat, size: f32) -> Transform {
    let scale = size / (food.half_size.max_element() * 2.0);
    // The corrected Drinking clip grips the SIDE of the item. Its knuckle
    // span (little -> index finger), not the palm normal, is the item's up
    // axis. This rests upright before the authored wrist tilts for a sip.
    let rotation = basis * Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
    let outward = basis * Vec3::Y;
    // Keep the nearest actual model surface beyond the palm. Clearance is
    // derived from palm length and inherits the evaluated avatar/Chibi scale.
    let center = palm + outward * (food.half_size.x * scale + size * 0.175);
    Transform {
        translation: center - rotation * food.center * scale,
        rotation,
        scale: Vec3::splat(scale),
    }
}

fn finished_naturally(
    active: &Active,
    remaining: Option<Duration>,
    visual: VisualState,
    dt: Duration,
) -> bool {
    remaining.is_none()
        && active.last_remaining <= dt
        && matches!(visual, VisualState::Idle | VisualState::SittingLoop)
}

type Bones<'w, 's> =
    Query<'w, 's, (Entity, &'static VrmBone, &'static RestGlobalTransform), With<NeraLiveBone>>;

fn hand_grip(bones: &Bones) -> Option<(Entity, Vec3, Quat, f32)> {
    let find = |name| bones.iter().find(|(_, bone, _)| bone.as_str() == name);
    let (hand, _, rest) = find("leftHand")?;
    let inverse = rest.0.to_matrix().inverse();
    if !inverse.is_finite() {
        return None;
    }
    let local =
        |name| find(name).map(|(_, _, rest)| inverse.transform_point3(rest.0.translation()));
    let middle = local("leftMiddleProximal")?;
    let across = local("leftIndexProximal")? - local("leftLittleProximal")?;
    let (position, rotation, size) = grip(middle, across)?;
    Some((hand, position, rotation, size))
}

fn update_treat(
    time: Res<Time<Real>>,
    animation_time: Res<Time>,
    mut requests: MessageReader<RequestTreat>,
    mut runtime: ResMut<TreatRuntime>,
    context: TreatContext,
    bones: Bones,
    assets: Res<TreatAssets>,
    scenes: Res<Assets<WorldAsset>>,
    props: Query<(), With<TreatProp>>,
    clips: Res<NeraAnimationClips>,
    playback: Res<NeraAnimationPlayback>,
    mut animation: ResMut<NeraAnimationState>,
    mut walking: ResMut<WalkingRuntime>,
    mut commands: Commands,
    mut fx: MessageWriter<PetFx>,
    mut sfx: MessageWriter<PetSfx>,
) {
    let requested = requests.read().count() > 0;
    let now = time.elapsed();
    let avatar = context.avatars.single().ok();
    let allowed = context.allowed(&animation);
    let TreatRuntime {
        active: active_slot,
        rng,
        ..
    } = &mut *runtime;
    if let Some(active) = active_slot.as_mut() {
        if !allowed
            || avatar != Some(active.avatar)
            || animation.visual_state() != VisualState::Treat
            || active.prop.is_some_and(|prop| !props.contains(prop))
        {
            if allowed
                && avatar == Some(active.avatar)
                && clips.treat_playing(&playback)
                && finished_naturally(
                    active,
                    animation.treat_remaining(),
                    animation.visual_state(),
                    animation_time.delta(),
                )
            {
                info!("Treat finished its full authored clip; normal behavior resumes");
            }
            if let Some(prop) = active.prop.take().filter(|prop| props.contains(*prop)) {
                commands.entity(prop).despawn();
            }
            animation.cancel_treat();
            *active_slot = None;
        } else if let Some(remaining) = animation.treat_remaining() {
            let (consumed, heart) =
                active.consume_and_pulse(remaining, clips.treat_playing(&playback), rng);
            if let Some(prop) = consumed {
                commands.entity(prop).despawn();
                sfx.write(PetSfx(PetSfxKind::Happy));
                info!(
                    "Treat bite: prop consumed; {} heart pulses",
                    active.love.as_ref().map_or(0, |love| love.count)
                );
            }
            if let Some(right) = heart {
                fx.write(PetFx {
                    kind: PetFxKind::Heart,
                    anchor: PetFxAnchor::HeadSide { right },
                });
            }
            active.last_remaining = remaining;
        }
    }
    if requested {
        runtime.request(avatar, now, allowed);
    }
    let Some(pending) = runtime.pending.as_ref() else {
        return;
    };
    let timed_out = now.saturating_sub(pending.at) > LOAD_TIMEOUT;
    if !allowed || avatar != Some(pending.avatar) || timed_out {
        if timed_out {
            warn!("Treat cancelled: prop/animation assets did not become ready");
        }
        runtime.pending = None;
        return;
    }
    let active_avatar = pending.avatar;
    let Some(duration) = clips.treat_duration() else {
        return;
    };
    let available: Vec<_> = assets
        .0
        .iter()
        .enumerate()
        .filter(|(_, handle)| scenes.contains(handle.id()))
        .map(|(index, _)| index)
        .collect();
    if available.is_empty() {
        return;
    }
    let Some((hand, position, rotation, size)) = hand_grip(&bones) else {
        warn!("Treat unavailable: missing/degenerate left-hand or finger humanoid data");
        runtime.pending = None;
        return;
    };
    if !animation.request_treat(duration) {
        runtime.pending = None;
        return;
    }
    let Some(index) = runtime.choose(&available) else {
        return;
    };
    let food = &FOODS[index];
    let prop = commands
        .spawn((
            Name::new(format!("Treat/{}", food.name)),
            TreatProp,
            WorldAssetRoot(assets.0[index].clone()),
            prop_transform(food, position, rotation, size),
            ChildOf(hand),
        ))
        .id();
    fx.write(PetFx {
        kind: PetFxKind::CloudPuff,
        anchor: PetFxAnchor::Bone(hand),
    });
    walking.cancel(&mut animation);
    runtime.active = Some(Active {
        avatar: active_avatar,
        prop: Some(prop),
        duration,
        last_remaining: duration,
        love: None,
    });
    runtime.pending = None;
    runtime.cooldown_until = now + COOLDOWN;
    info!(
        "Treat started | food={} leftHand clip={DRINKING_PATH}",
        food.name
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_food_bounds_stay_outside_the_palm_for_all_rig_axes_and_body_scales() {
        for orientation in [
            Quat::IDENTITY,
            Quat::from_rotation_x(1.2),
            Quat::from_rotation_z(2.1),
        ] {
            let (palm, basis, size) =
                grip(orientation * Vec3::Y * 0.07, orientation * Vec3::X * 0.04).unwrap();
            let outward = basis * Vec3::Y;
            for food in FOODS {
                let prop = prop_transform(&food, palm, basis, size).to_matrix();
                for x in [-1.0, 1.0] {
                    for y in [-1.0, 1.0] {
                        for z in [-1.0, 1.0] {
                            let point = prop.transform_point3(
                                food.center + food.half_size * Vec3::new(x, y, z),
                            );
                            assert!(
                                (point - palm).dot(outward) >= size * 0.175 - 0.00001,
                                "{} crosses the palm clearance",
                                food.name
                            );
                            for body_scale in [0.62, 1.0, 1.7] {
                                assert!(
                                    ((point - palm) * body_scale).dot(outward)
                                        >= size * 0.175 * body_scale - 0.00001
                                );
                            }
                        }
                    }
                }
                // The item stays upright along the knuckle span; wrist tilting
                // remains entirely authored by the shared Drinking animation.
                assert!(
                    (prop.transform_vector3(Vec3::Y).normalize() - basis * Vec3::X).length()
                        < 0.00001
                );
            }
        }
    }

    #[test]
    fn animation_finishes_normally_or_interrupts_independently_of_prop_consumption() {
        let mut active = Active {
            avatar: Entity::from_bits(1),
            prop: None,
            duration: Duration::from_secs(9),
            last_remaining: Duration::from_secs(3),
            love: None,
        };
        let frame = Duration::from_millis(16);
        assert!(!finished_naturally(
            &active,
            Some(Duration::from_secs(3)),
            VisualState::Treat,
            frame
        ));
        assert!(
            !finished_naturally(&active, None, VisualState::Idle, frame),
            "an early user cancellation is not consumption"
        );
        active.last_remaining = Duration::from_millis(10);
        assert!(finished_naturally(&active, None, VisualState::Idle, frame));
        assert!(finished_naturally(
            &active,
            None,
            VisualState::SittingLoop,
            frame
        ));
        for interrupted in [
            VisualState::HeadStroke,
            VisualState::Floating,
            VisualState::AlarmJump,
        ] {
            assert!(!finished_naturally(&active, None, interrupted, frame));
        }
    }

    #[test]
    fn bite_consumes_once_at_authored_marker_and_keeps_recovery_animation_alive() {
        let duration = Duration::from_secs_f32(8.833333);
        let prop = Entity::from_bits(2);
        let mut active = Active {
            avatar: Entity::from_bits(1),
            prop: Some(prop),
            duration,
            last_remaining: duration,
            love: None,
        };
        let mut rng = NeraRng::seeded(81);
        assert_eq!(
            active.consume_and_pulse(
                duration - CONSUME_AT + Duration::from_millis(1),
                true,
                &mut rng
            ),
            (None, None)
        );
        assert_eq!(active.prop, Some(prop));
        // A pending/failed/other animation must never consume or emit affection.
        assert_eq!(
            active.consume_and_pulse(duration - CONSUME_AT, false, &mut rng),
            (None, None)
        );
        assert_eq!(active.prop, Some(prop));
        let (consumed, first_heart) =
            active.consume_and_pulse(duration - CONSUME_AT, true, &mut rng);
        assert_eq!(consumed, Some(prop));
        assert!(first_heart.is_some());
        assert_eq!(active.prop, None);
        assert_eq!(
            active.consume_and_pulse(duration - CONSUME_AT, true, &mut rng),
            (None, None)
        );
        let expected = active.love.as_ref().unwrap().count;
        let mut emitted = 1;
        for pulse in 1..=8 {
            let (consumed, heart) = active.consume_and_pulse(
                duration - CONSUME_AT - LOVE_INTERVAL * pulse,
                true,
                &mut rng,
            );
            assert_eq!(consumed, None, "consumption/Happy feedback cannot repeat");
            emitted += usize::from(heart.is_some());
        }
        assert_eq!(emitted, expected);
        assert!((2..=5).contains(&emitted));
        assert_eq!(active.duration, duration);
        assert!(!finished_naturally(
            &active,
            Some(Duration::from_secs(2)),
            VisualState::Treat,
            Duration::from_millis(16)
        ));
    }

    #[test]
    fn bite_hearts_randomize_two_to_five_pulses_on_both_sides_above_fx_cooldown() {
        let mut counts = std::collections::HashSet::new();
        let mut first_sides = std::collections::HashSet::new();
        for seed in 1..=100 {
            let mut rng = NeraRng::seeded(seed);
            let mut love = TreatLove::new(Duration::ZERO, &mut rng);
            let first = love.pulse(Duration::ZERO).unwrap();
            first_sides.insert(first);
            assert_eq!(love.pulse(LOVE_INTERVAL - Duration::from_millis(1)), None);
            let mut sides = vec![first];
            for pulse in 1..=8 {
                if let Some(side) = love.pulse(LOVE_INTERVAL * pulse) {
                    sides.push(side);
                }
            }
            assert!((2..=5).contains(&sides.len()));
            assert!(sides.contains(&true) && sides.contains(&false));
            counts.insert(sides.len());
        }
        assert_eq!(counts.len(), 4);
        assert_eq!(first_sides.len(), 2);
        let mut love = TreatLove::new(Duration::ZERO, &mut NeraRng::seeded(37));
        assert!(love.pulse(Duration::from_secs(2)).is_some());
        assert_eq!(
            love.pulse(Duration::from_secs(2)),
            None,
            "no stalled-frame burst"
        );
    }

    #[test]
    fn cooldown_priority_loading_and_pending_requests_do_not_restart_an_interaction() {
        let avatar = Entity::from_bits(1);
        let mut runtime = TreatRuntime {
            cooldown_until: Duration::from_secs(3),
            ..default()
        };
        runtime.request(Some(avatar), Duration::from_secs(2), true);
        assert!(!runtime.busy());
        runtime.request(None, Duration::from_secs(3), true);
        assert!(!runtime.busy());
        runtime.request(Some(avatar), Duration::from_secs(3), false);
        assert!(!runtime.busy());
        runtime.request(Some(avatar), Duration::from_secs(3), true);
        assert!(runtime.busy());
        runtime.request(Some(Entity::from_bits(2)), Duration::from_secs(4), true);
        assert_eq!(runtime.pending.as_ref().unwrap().avatar, avatar);
        assert_eq!(runtime.pending.as_ref().unwrap().at, Duration::from_secs(3));
        assert_eq!(runtime.last_food, None);
    }

    #[test]
    fn food_pool_does_not_repeat_and_empty_pool_is_safe() {
        let mut runtime = TreatRuntime {
            rng: NeraRng::seeded(87),
            ..default()
        };
        assert_eq!(runtime.choose(&[]), None);
        let mut seen = std::collections::HashSet::new();
        let mut previous = None;
        for _ in 0..100 {
            let selected = runtime
                .choose(&(0..FOODS.len()).collect::<Vec<_>>())
                .unwrap();
            assert_ne!(previous, Some(selected));
            previous = Some(selected);
            seen.insert(selected);
        }
        assert_eq!(seen.len(), FOODS.len());
        assert_eq!(runtime.choose(&[3]), Some(3));
    }

    #[test]
    fn palm_geometry_is_rig_axis_independent_and_inherits_body_scaling() {
        for orientation in [
            Quat::IDENTITY,
            Quat::from_rotation_x(1.2),
            Quat::from_rotation_z(2.1),
        ] {
            let middle = orientation * Vec3::Y * 0.07;
            let across = orientation * Vec3::X * 0.04;
            let (position, rotation, size) = grip(middle, across).unwrap();
            assert!((size - 0.14).abs() < 0.00001);
            assert!((rotation * Vec3::Y - orientation * -Vec3::Z).length() < 0.00001);
            let parent = Mat4::from_scale(Vec3::splat(0.62));
            assert!(
                (parent.transform_point3(position).length() - position.length() * 0.62).abs()
                    < 0.00001
            );
        }
        assert!(grip(Vec3::ZERO, Vec3::X).is_none());
        assert!(grip(Vec3::Y, Vec3::Y).is_none());
        assert!(grip(Vec3::NAN, Vec3::X).is_none());
    }

    #[test]
    fn bundled_props_are_small_gltf_with_local_texture_and_no_runtime_urls() {
        let mut total = 0;
        for food in FOODS {
            let bytes = std::fs::read(crate::nera_asset_file(food.path)).unwrap();
            assert_eq!(&bytes[..4], b"glTF");
            assert!(bytes.len() < 120_000);
            total += bytes.len();
            let length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
            let json: serde_json::Value = serde_json::from_slice(&bytes[20..20 + length]).unwrap();
            assert!(!json["meshes"].as_array().unwrap().is_empty());
            assert_eq!(json["images"][0]["uri"], "Textures/colormap.png");
            assert!(
                food.center.is_finite()
                    && food.half_size.is_finite()
                    && food.half_size.min_element() > 0.0
            );
        }
        assert!(total < 250_000);
        assert!(crate::nera_asset_file("props/treats/Textures/colormap.png").exists());
        assert!(crate::nera_asset_file(DRINKING_PATH).exists());
    }
}
