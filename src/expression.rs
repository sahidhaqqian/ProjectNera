//! Project Nera — Phase 5.8 facial expression state.
//!
//! The expression is derived from `NeraAnimationState::visual_state()`, the same
//! authoritative answer that already drives clip selection. Deriving it from raw
//! interaction messages instead would let the face and the body disagree — a
//! smiling Nera playing `Angry.vrma`, or an angry face over a head-pat clip.
//!
//! Facial ownership is explicit:
//!
//! 1. **`Neutral` is not an override.** `SetExpressions` *replaces* every
//!    override, and an `ExpressionOverride` component wins over the VRMA
//!    animation player's value. So `Neutral` emits an empty weight set, which
//!    removes our overrides and hands the face back to the clip. Pushing the
//!    `neutral` preset instead would fight the emotion presets, since
//!    `nera_256.vrm` binds a real morph target to it.
//!
//! 2. Generic reactions override only emotion presets. The protected basic clips
//!    use `basic.rs` for intentional eyebrow/eyelid/mouth poses and intensity.
//!    `AutomaticFace` retains natural blink on detailed rigs; imported models
//!    fall back to available presets, including deliberate closed-eye faces.
//!
//! Manual expression previews intentionally own eyelids and mouth as well.
//! Detailed Nera morphs refine basic-animation faces after the VRM pass while retaining
//! its natural blink; clearing a preview returns ownership to the body state.
//!
//! Overriding is safe here rather than destructive because of a property of the
//! asset set, not of this code: every VRMA Nera actually plays is **body-only**.
//! Scanning `assets/animations/**.vrma` for expression keys (`happy`, `angry`,
//! `sad`, `surprised`, `relaxed`, `neutral`, `blink`, `expressions`) finds
//! exactly one hit — `headpat_with_blink.vrma`, which no system loads. So these
//! overrides *add* a face to clips that never had one instead of pinning a
//! clip's own facial performance at full weight.
//!
//! That is an asset fact and can change: if a future clip carries expression
//! curves, its basic mapping must be removed so the clip keeps its own face.

use std::time::Duration;

pub(crate) mod basic;
pub(crate) mod face;
pub(crate) mod presets;
use presets::preset_weights;
pub(crate) use presets::{NeraExpressionPreset, preview_selection};

use bevy::prelude::*;

use bevy_vrm1::prelude::{ExpressionEntityMap, SetExpressions, VrmSystemSets};

use crate::animation::{IdleMicroAction, NeraAnimationState, VisualState};

// -----------------------------------------------------------------------------
// Tuning
// -----------------------------------------------------------------------------

/// Crossfade length. Deliberately close to `animation::HEAD_STROKE_BLEND` so a
/// face change and the body clip that caused it land together.
const EXPRESSION_BLEND: Duration = Duration::from_millis(180);

/// Weights are quantized to this step so the VRM event only re-fires when the
/// visible result actually changes. At 32 steps a 180 ms crossfade emits roughly
/// a dozen events instead of one per frame.
const WEIGHT_QUANTUM: f32 = 1.0 / 32.0;

/// Below this an override is invisible but still suppresses the VRMA clip's own
/// value for that preset, so it is dropped rather than pushed as a near-zero.
const MIN_VISIBLE_WEIGHT: f32 = 0.01;

// -----------------------------------------------------------------------------
// Vocabulary
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum NeraExpression {
    /// No override at all: the VRMA clip owns the face.
    #[default]
    Neutral,
    Happy,
    Angry,
    Sad,
    Surprised,
    Relaxed,
}

/// Every expression Nera can hold. Used by tests to prove the emitted presets
/// stay inside the emotion allowlist.
const ALL_EXPRESSIONS: [NeraExpression; 6] = [
    NeraExpression::Neutral,
    NeraExpression::Happy,
    NeraExpression::Angry,
    NeraExpression::Sad,
    NeraExpression::Surprised,
    NeraExpression::Relaxed,
];

impl NeraExpression {
    /// VRM preset name as declared in `assets/vrm/nera_256.vrm`.
    ///
    /// `Neutral` has no name on purpose — see the module docs.
    fn preset(self) -> Option<&'static str> {
        match self {
            NeraExpression::Neutral => None,
            NeraExpression::Happy => Some("happy"),
            NeraExpression::Angry => Some("angry"),
            NeraExpression::Sad => Some("sad"),
            NeraExpression::Surprised => Some("surprised"),
            NeraExpression::Relaxed => Some("relaxed"),
        }
    }
}

/// The override set to push to `bevy_vrm1`.
///
/// Two slots are enough: at most one outgoing and one incoming preset are live
/// during a crossfade. Fixed-size and `PartialEq` so change detection is a
/// plain comparison instead of hashing a map every frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ExpressionWeights {
    slots: [(Option<&'static str>, f32); 2],
}

impl ExpressionWeights {
    /// Live preset/weight pairs, ready for `SetExpressions::from_iter`.
    fn live(self) -> impl Iterator<Item = (&'static str, f32)> {
        self.slots
            .into_iter()
            .filter_map(|(preset, weight)| preset.map(|name| (name, weight)))
    }

    fn is_empty(self) -> bool {
        self.slots.iter().all(|(preset, _)| preset.is_none())
    }
}

// -----------------------------------------------------------------------------
// Pure mapping
// -----------------------------------------------------------------------------

/// Face implied by the body. Exhaustive on purpose: adding a `VisualState`
/// variant must force a decision about its expression.
fn expression_for_visual(visual: VisualState) -> NeraExpression {
    use NeraExpression as Face;
    use VisualState as Body;

    match visual {
        // Petting is affection; poking is not. `HitHead` feeds the three-hit
        // damage combo that ends in Death, so it reads as surprise, not joy.
        Body::HeadStroke | Body::StartupGreeting | Body::Treat | Body::FocusCelebrate => {
            Face::Happy
        }
        Body::HitHead | Body::DodgeWhileSitting => Face::Surprised,
        Body::HitChest => Face::Angry,
        Body::LimbReaction(part) => match part {
            crate::interaction::BodyPart::LeftArm
            | crate::interaction::BodyPart::RightArm
            | crate::interaction::BodyPart::LeftHand
            | crate::interaction::BodyPart::RightHand => Face::Happy,
            _ => Face::Surprised,
        },
        Body::Death => Face::Sad,

        // Airborne, and the alarm presentation.
        Body::Floating | Body::Falling => Face::Surprised,
        Body::AlarmWave | Body::AlarmJump => Face::Surprised,

        // Settled, calm states.
        Body::SittingLoop | Body::SittingEnter | Body::Praying => Face::Relaxed,

        #[cfg(target_os = "windows")]
        Body::MenuAnimation(choice) => expression_for_menu(choice),

        Body::IdleMicro(action) => expression_for_idle_micro(action),

        // Neutral means "no override". Idle has no facial intent of its own,
        // and the short transitional clips are moments where a held emotion
        // would read as stuck rather than as expression.
        Body::Idle
        | Body::FocusActivity(_)
        | Body::Screensaver(_)
        | Body::TouchReaction(_)
        | Body::Walking
        | Body::SittingExit
        | Body::Landing
        | Body::LibraryAnimation(_) => Face::Neutral,
    }
}

/// Fidgets supply a face only when their body language calls for one.
///
/// The micro-action clips are body-only like everything else Nera plays, so
/// these overrides add the face rather than fight one.
fn expression_for_idle_micro(action: IdleMicroAction) -> NeraExpression {
    use IdleMicroAction as Fidget;
    use NeraExpression as Face;

    match action {
        // Scanning the room is attention, not emotion.
        Fidget::LookAround | Fidget::CatwalkIdleTwistR => Face::Neutral,
        Fidget::Relax | Fidget::Sleepy | Fidget::Bored | Fidget::SittingRubbingArm => Face::Relaxed,
        Fidget::AngryGesture => Face::Angry,
        // `nera_256.vrm` declares no blush preset, so the shy clip gets the
        // nearest available emotion instead of an override that would fight it.
        Fidget::Blush => Face::Happy,
        Fidget::CursorNotice => Face::Surprised,
        Fidget::CursorReach | Fidget::CursorCatch | Fidget::EdgePeek => Face::Happy,
        Fidget::CursorEscape => Face::Relaxed,
    }
}

/// Menu clips are body-only performances of an emotion, so the face supplies
/// what the clip leaves out. Matching the clip's own intent is what keeps the
/// two layers reading as one gesture.
#[cfg(target_os = "windows")]
fn expression_for_menu(choice: crate::action_menu::AnimationMenuChoice) -> NeraExpression {
    use crate::action_menu::AnimationMenuChoice as Menu;
    use NeraExpression as Face;

    match choice {
        Menu::Wave => Face::Happy,
        Menu::Angry => Face::Angry,
        Menu::Sad => Face::Sad,
        Menu::Surprised => Face::Surprised,
        // Body-only clips.
        Menu::Thinking | Menu::ResetBasePose => Face::Neutral,
    }
}

fn quantize(weight: f32) -> f32 {
    (weight / WEIGHT_QUANTUM).round() * WEIGHT_QUANTUM
}

/// Crossfade weights between two expressions.
///
/// Pure so it can be tested without a Bevy `World`, matching how
/// `alarm::preset_duration` and `presentation::compute_attention_layout` are
/// tested.
fn compute_weights(
    previous: NeraExpression,
    current: NeraExpression,
    blend: f32,
) -> ExpressionWeights {
    let t = blend.clamp(0.0, 1.0);

    // Quantize the incoming weight and *derive* the outgoing one from it, so a
    // crossfade always sums to exactly 1.0. Rounding both sides independently
    // can overshoot, and two presets above half weight double-deform the face.
    let incoming = quantize(t).clamp(0.0, 1.0);
    let outgoing = 1.0 - incoming;

    let raw = if previous == current {
        [(current.preset(), 1.0), (None, 0.0)]
    } else {
        [(previous.preset(), outgoing), (current.preset(), incoming)]
    };

    let slots = raw.map(|(preset, weight)| {
        // Dropping invisible weights matters: a zero-weight override still
        // suppresses the VRMA clip's value for that preset.
        if preset.is_none() || weight < MIN_VISIBLE_WEIGHT {
            (None, 0.0)
        } else {
            (preset, weight.clamp(0.0, 1.0))
        }
    });

    ExpressionWeights { slots }
}

// -----------------------------------------------------------------------------
// Runtime state
// -----------------------------------------------------------------------------

#[derive(Resource, Debug)]
pub(crate) struct NeraExpressionState {
    /// Expression being faded towards.
    current: NeraExpression,
    /// Expression being faded away from.
    previous: NeraExpression,
    /// Crossfade progress, 0..1.
    blend: f32,
    /// VRM entity that received `last_pushed`. If the avatar is reloaded or
    /// replaced, the same facial state must still be re-sent to the new entity.
    bound_entity: Option<Entity>,
    /// Change detection, so an unchanged face emits no VRM event at all.
    last_pushed: Option<Vec<(String, f32)>>,
}

#[derive(Resource, Default, Debug)]
pub(crate) struct ExpressionPreviewOverride {
    selection: Option<(NeraExpressionPreset, f32)>,
}

#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub(crate) enum ExpressionPreviewRequest {
    Set(NeraExpressionPreset, f32),
    Clear,
}

impl ExpressionPreviewRequest {
    pub(crate) fn for_selection(preset: NeraExpressionPreset, intensity: f32) -> Self {
        preview_selection(preset, intensity).map_or(Self::Clear, |(preset, intensity)| {
            Self::Set(preset, intensity)
        })
    }
}

impl ExpressionPreviewOverride {
    fn apply(&mut self, request: ExpressionPreviewRequest) {
        self.selection = match request {
            ExpressionPreviewRequest::Set(preset, intensity) => {
                preview_selection(preset, intensity)
            }
            ExpressionPreviewRequest::Clear => None,
        };
    }
}

impl Default for NeraExpressionState {
    fn default() -> Self {
        Self {
            current: NeraExpression::default(),
            previous: NeraExpression::default(),
            // Start settled: there is nothing to fade from before the first
            // real expression arrives.
            blend: 1.0,
            bound_entity: None,
            last_pushed: None,
        }
    }
}

impl NeraExpressionState {
    fn advance(&mut self, target: NeraExpression, delta: Duration) {
        if target != self.current {
            // An interruption mid-fade restarts from the outgoing target
            // rather than from the blended midpoint. That can pop slightly, but
            // it keeps the state to three fields and never leaves a stale
            // third preset on the model.
            self.previous = self.current;
            self.current = target;
            self.blend = 0.0;

            info!("Nera expression -> {target:?}");
        }

        if self.blend >= 1.0 {
            return;
        }

        let total = EXPRESSION_BLEND.as_secs_f32();
        if total <= 0.0 {
            self.blend = 1.0;
            return;
        }

        self.blend = (self.blend + delta.as_secs_f32() / total).min(1.0);
    }

    fn weights(&self) -> ExpressionWeights {
        compute_weights(self.previous, self.current, self.blend)
    }
}

pub(crate) struct NeraExpressionPlugin;

impl Plugin for NeraExpressionPlugin {
    fn build(&self, app: &mut App) {
        face::install(app);
        app.init_resource::<NeraExpressionState>()
            .init_resource::<ExpressionPreviewOverride>()
            .add_message::<ExpressionPreviewRequest>()
            .add_systems(
                PostUpdate,
                drive_nera_expressions.before(VrmSystemSets::Expressions),
            );
    }
}

/// PostUpdate so the animation state has already been ticked and applied for
/// this frame, and ahead of `bevy_vrm1`'s own expression pass so the override
/// lands in the same frame instead of a frame late.
fn drive_nera_expressions(
    time: Res<Time>,
    animation: Res<NeraAnimationState>,
    cloud: Option<Res<crate::cloud::CloudRuntime>>,
    clips: Option<Res<crate::animation::NeraAnimationClips>>,
    library: Option<Res<crate::animation::LibraryPlayback>>,
    mut automatic: Local<face::AutomaticFace>,
    mut state: ResMut<NeraExpressionState>,
    mut preview: ResMut<ExpressionPreviewOverride>,
    mut requests: MessageReader<ExpressionPreviewRequest>,
    vrms: Query<
        (
            Entity,
            Ref<ExpressionEntityMap>,
            Has<face::NamedTimidEyelids>,
        ),
        With<crate::avatar::NeraAvatar>,
    >,
    mut commands: Commands,
) {
    for request in requests.read() {
        preview.apply(*request);
    }
    let visual = animation.visual_state();
    let path = if let VisualState::LibraryAnimation(entity) = visual {
        library
            .as_ref()
            .and_then(|library| library.path_for(entity))
    } else {
        None
    };
    let selection = if visual == VisualState::SittingLoop
        && cloud
            .as_ref()
            .is_some_and(|cloud| cloud.mode() == Some(crate::cloud::CloudMode::Rest))
    {
        if !cloud.as_ref().is_some_and(|cloud| cloud.is_lying_rest())
            || clips
                .as_ref()
                .is_some_and(|clips| clips.sleeping_idle_entity().is_some())
        {
            basic::for_clip("animasi_basic/Sleeping_Idle.vrma")
        } else {
            Some((NeraExpressionPreset::Exhausted, 0.6))
        }
    } else if visual == VisualState::SittingLoop
        && animation.selected_sitting_pose() == crate::animation::SittingPose::Sitting4
    {
        basic::for_clip("animasi_basic/Sitting_4.vrma")
    } else {
        basic::for_visual(visual, path)
    };
    automatic.advance_headpat(
        selection,
        time.delta_secs(),
        clips
            .as_ref()
            .and_then(|clips| clips.head_stroke_progress(&animation)),
    );
    let target = expression_for_visual(visual);
    state.advance(target, time.delta());

    // `iter().next()` rather than `single()`: while the VRM is still loading
    // there is no expression map yet, and `single()` would log an error every
    // frame. `bevy_vrm1` also warns if the event targets an uninitialized VRM.
    let Some((vrm_entity, map, named_eyelids)) = vrms.iter().next() else {
        // A later VRM may be a freshly spawned entity. Forget which entity
        // received the previous override so its current face is re-applied.
        state.bound_entity = None;
        state.last_pushed = None;
        return;
    };

    if state.bound_entity != Some(vrm_entity) || map.is_changed() {
        state.bound_entity = Some(vrm_entity);
        state.last_pushed = None;
    }

    let weights: Vec<(String, f32)> = match preview.selection {
        Some((preset, intensity)) => {
            preset_weights(preset, intensity, map.0.keys().map(|name| name.0.as_str()))
        }
        None if automatic.current.is_some() || automatic.previous.is_some() => {
            automatic.weights_for_rig(map.0.keys().map(|name| name.0.as_str()), named_eyelids)
        }
        None => state
            .weights()
            .live()
            .filter_map(|(name, weight)| {
                map.0
                    .keys()
                    .find(|available| available.0 == name)
                    .map(|_| (name.to_owned(), weight))
            })
            .collect(),
    };

    let angry = state
        .weights()
        .live()
        .find(|(name, _)| *name == "angry")
        .map_or(0.0, |(_, weight)| weight);
    commands.entity(vrm_entity).insert((
        face::FacePose(preview.selection),
        *automatic,
        face::AutomaticAngry(angry),
    ));

    if state.last_pushed.as_ref() == Some(&weights) {
        return;
    }

    state.last_pushed = Some(weights.clone());

    // An empty set is meaningful, not a no-op: `apply_set_expressions` removes
    // every override absent from the map, which is how Neutral hands the face
    // back to the VRMA clip.
    commands.trigger(SetExpressions::from_iter(
        vrm_entity,
        weights
            .iter()
            .map(|(name, weight)| (name.as_str(), *weight)),
    ));

    if weights.is_empty() {
        trace!("Nera face -> VRMA control");
    }
}

// -----------------------------------------------------------------------------
// Pure state tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_owns_face_until_cleared_and_rebinds_after_avatar_change() {
        use bevy_vrm1::prelude::VrmExpression;
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<NeraAnimationState>()
            .init_resource::<NeraExpressionState>()
            .init_resource::<ExpressionPreviewOverride>()
            .add_message::<ExpressionPreviewRequest>()
            .add_systems(PostUpdate, drive_nera_expressions);
        let expression = app.world_mut().spawn_empty().id();
        let avatar = app
            .world_mut()
            .spawn((
                crate::avatar::NeraAvatar,
                ExpressionEntityMap([(VrmExpression::from("happy"), expression)].into()),
            ))
            .id();

        app.world_mut().write_message(ExpressionPreviewRequest::Set(
            NeraExpressionPreset::Joy,
            1.0,
        ));
        app.update();
        assert_eq!(
            app.world().resource::<NeraExpressionState>().last_pushed,
            Some(vec![("happy".into(), 1.0)])
        );

        app.world_mut().write_message(ExpressionPreviewRequest::Set(
            NeraExpressionPreset::Angry,
            1.0,
        ));
        app.update();
        assert_eq!(
            app.world().resource::<NeraExpressionState>().last_pushed,
            Some(vec![])
        );

        app.world_mut().despawn(avatar);
        let replacement = app
            .world_mut()
            .spawn((
                crate::avatar::NeraAvatar,
                ExpressionEntityMap([(VrmExpression::from("happy"), expression)].into()),
            ))
            .id();
        app.world_mut().write_message(ExpressionPreviewRequest::Set(
            NeraExpressionPreset::Joy,
            1.0,
        ));
        app.update();
        assert_eq!(
            app.world().resource::<NeraExpressionState>().bound_entity,
            Some(replacement)
        );
        assert_eq!(
            app.world().resource::<NeraExpressionState>().last_pushed,
            Some(vec![("happy".into(), 1.0)])
        );

        app.world_mut()
            .write_message(ExpressionPreviewRequest::Clear);
        app.update();
        assert!(
            app.world()
                .resource::<ExpressionPreviewOverride>()
                .selection
                .is_none()
        );
        assert_eq!(
            app.world().resource::<NeraExpressionState>().last_pushed,
            Some(vec![])
        );
    }

    #[test]
    fn preview_intensity_is_clamped() {
        let mut preview = ExpressionPreviewOverride::default();
        preview.apply(ExpressionPreviewRequest::Set(
            NeraExpressionPreset::Joy,
            2.0,
        ));
        assert_eq!(preview.selection, Some((NeraExpressionPreset::Joy, 1.0)));
        preview.apply(ExpressionPreviewRequest::Set(
            NeraExpressionPreset::Joy,
            -1.0,
        ));
        assert!(preview.selection.is_none());
        preview.apply(ExpressionPreviewRequest::Clear);
        assert!(preview.selection.is_none());
    }

    #[test]
    fn default_and_zero_intensity_release_preview_ownership() {
        for request in [
            ExpressionPreviewRequest::Set(NeraExpressionPreset::Default, 1.0),
            ExpressionPreviewRequest::Set(NeraExpressionPreset::Default, 0.75),
            ExpressionPreviewRequest::Set(NeraExpressionPreset::Angry, 0.0),
            ExpressionPreviewRequest::Set(NeraExpressionPreset::Timid, f32::NAN),
            ExpressionPreviewRequest::Set(NeraExpressionPreset::Exhausted, f32::INFINITY),
        ] {
            let mut preview = ExpressionPreviewOverride::default();
            preview.apply(ExpressionPreviewRequest::Set(
                NeraExpressionPreset::Angry,
                0.9,
            ));
            assert!(preview.selection.is_some());
            preview.apply(request);
            assert!(preview.selection.is_none(), "{request:?}");
            let ExpressionPreviewRequest::Set(preset, intensity) = request else {
                unreachable!()
            };
            assert_eq!(
                ExpressionPreviewRequest::for_selection(preset, intensity),
                ExpressionPreviewRequest::Clear
            );
        }
    }

    #[test]
    fn driver_releases_manual_pose_and_deduplicates_unchanged_expression_events() {
        use bevy_vrm1::prelude::VrmExpression;
        #[derive(Resource, Default)]
        struct Pushes(usize);

        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<NeraAnimationState>()
            .init_resource::<NeraExpressionState>()
            .init_resource::<ExpressionPreviewOverride>()
            .init_resource::<Pushes>()
            .add_message::<ExpressionPreviewRequest>()
            .add_systems(PostUpdate, drive_nera_expressions)
            .add_observer(|_: On<SetExpressions>, mut pushes: ResMut<Pushes>| {
                pushes.0 += 1;
            });
        let angry = app.world_mut().spawn_empty().id();
        let avatar = app
            .world_mut()
            .spawn((
                crate::avatar::NeraAvatar,
                ExpressionEntityMap([(VrmExpression::from("angry"), angry)].into()),
            ))
            .id();

        for release in [
            ExpressionPreviewRequest::Set(NeraExpressionPreset::Default, 1.0),
            ExpressionPreviewRequest::Set(NeraExpressionPreset::Angry, 0.0),
        ] {
            app.world_mut().write_message(ExpressionPreviewRequest::Set(
                NeraExpressionPreset::Angry,
                0.9,
            ));
            app.update();
            assert_eq!(
                app.world().get::<face::FacePose>(avatar).unwrap().0,
                Some((NeraExpressionPreset::Angry, 0.9))
            );
            let pushes = app.world().resource::<Pushes>().0;
            for _ in 0..3 {
                app.world_mut().write_message(ExpressionPreviewRequest::Set(
                    NeraExpressionPreset::Angry,
                    0.9,
                ));
                app.update();
            }
            assert_eq!(app.world().resource::<Pushes>().0, pushes);

            app.world_mut().write_message(release);
            app.update();
            assert!(
                app.world()
                    .resource::<ExpressionPreviewOverride>()
                    .selection
                    .is_none()
            );
            assert!(
                app.world()
                    .get::<face::FacePose>(avatar)
                    .unwrap()
                    .0
                    .is_none()
            );
            assert_eq!(
                app.world().resource::<NeraExpressionState>().last_pushed,
                Some(vec![])
            );
            assert_eq!(app.world().resource::<Pushes>().0, pushes + 1);
            app.update();
            assert_eq!(app.world().resource::<Pushes>().0, pushes + 1);
        }
    }

    #[test]
    fn neutral_emits_no_override_at_all() {
        let weights = compute_weights(NeraExpression::Neutral, NeraExpression::Neutral, 1.0);

        assert!(weights.is_empty());
        assert_eq!(weights.live().count(), 0);
    }

    #[test]
    fn a_settled_expression_emits_exactly_one_full_override() {
        for expression in ALL_EXPRESSIONS {
            let Some(preset) = expression.preset() else {
                continue;
            };

            let weights = compute_weights(expression, expression, 1.0);

            let live: Vec<_> = weights.live().collect();
            assert_eq!(live, vec![(preset, 1.0)], "{expression:?}");
        }
    }

    /// Blink and the mouth/gaze presets belong to the VRMA clips and to
    /// bevy_vrm1's own systems. Overriding one would freeze Nera's blinking,
    /// which is far more noticeable than a wrong emotion.
    #[test]
    fn only_emotion_presets_can_ever_be_pushed() {
        const ALLOWED: [&str; 5] = ["happy", "angry", "sad", "surprised", "relaxed"];

        for previous in ALL_EXPRESSIONS {
            for current in ALL_EXPRESSIONS {
                for step in 0..=10 {
                    let weights = compute_weights(previous, current, step as f32 / 10.0);

                    for (preset, weight) in weights.live() {
                        assert!(
                            ALLOWED.contains(&preset),
                            "{preset} is not an emotion preset",
                        );
                        assert!(
                            (MIN_VISIBLE_WEIGHT..=1.0).contains(&weight),
                            "{preset} pushed at {weight}",
                        );
                    }
                }
            }
        }
    }

    /// Two full-weight presets at once would double the deformation and look
    /// broken, so a crossfade must hand weight from one to the other — exactly,
    /// not approximately, since `compute_weights` derives the outgoing weight
    /// from the incoming one.
    #[test]
    fn crossfade_weight_always_sums_to_exactly_one() {
        let previous = NeraExpression::Angry;
        let current = NeraExpression::Happy;

        for step in 0..=1000 {
            let blend = step as f32 / 1000.0;
            let weights = compute_weights(previous, current, blend);

            let live: Vec<_> = weights.live().collect();

            // Both ends of the fade legitimately hold a single preset.
            if live.len() == 2 {
                let total: f32 = live.iter().map(|(_, w)| *w).sum();
                assert_eq!(total, 1.0, "blend {blend} summed to {total}: {live:?}",);
            } else {
                assert_eq!(live.len(), 1, "blend {blend}: {live:?}");
                assert_eq!(live[0].1, 1.0, "blend {blend}: {live:?}",);
            }
        }
    }

    #[test]
    fn crossfade_midpoint_holds_both_presets() {
        let weights = compute_weights(NeraExpression::Angry, NeraExpression::Happy, 0.5);

        let live: Vec<_> = weights.live().collect();

        assert_eq!(live.len(), 2, "{live:?}");
        assert!(live.iter().any(|(name, _)| *name == "angry"));
        assert!(live.iter().any(|(name, _)| *name == "happy"));
    }

    #[test]
    fn a_finished_crossfade_drops_the_outgoing_preset() {
        let weights = compute_weights(NeraExpression::Angry, NeraExpression::Happy, 1.0);

        let live: Vec<_> = weights.live().collect();

        assert_eq!(live, vec![("happy", 1.0)]);
    }

    /// Change detection is what keeps a settled face from re-triggering the VRM
    /// event 60 times a second.
    #[test]
    fn quantized_weights_are_stable_while_settled() {
        let first = compute_weights(NeraExpression::Happy, NeraExpression::Happy, 1.0);
        let second = compute_weights(NeraExpression::Happy, NeraExpression::Happy, 1.0);

        assert_eq!(first, second);
    }

    #[test]
    fn out_of_range_blend_is_clamped_not_extrapolated() {
        let settled = compute_weights(NeraExpression::Sad, NeraExpression::Happy, 1.0);

        assert_eq!(
            compute_weights(NeraExpression::Sad, NeraExpression::Happy, 42.0,),
            settled,
        );
        assert_eq!(
            compute_weights(NeraExpression::Sad, NeraExpression::Happy, -7.0,),
            compute_weights(NeraExpression::Sad, NeraExpression::Happy, 0.0,),
        );
    }

    // ---------------------------------------------------------------------
    // Body -> face mapping
    // ---------------------------------------------------------------------

    #[test]
    fn petting_is_happy_but_poking_is_not() {
        assert_eq!(
            expression_for_visual(VisualState::HeadStroke),
            NeraExpression::Happy,
        );

        // `HitHead` feeds the three-hit damage combo that ends in Death.
        assert_ne!(
            expression_for_visual(VisualState::HitHead),
            NeraExpression::Happy,
        );
        assert_eq!(
            expression_for_visual(VisualState::HitHead),
            NeraExpression::Surprised,
        );
    }

    #[test]
    fn the_five_required_emotions_are_all_reachable() {
        let reached: Vec<_> = [
            VisualState::Idle,
            VisualState::HeadStroke,
            VisualState::HitChest,
            VisualState::Death,
            VisualState::Falling,
        ]
        .iter()
        .map(|visual| expression_for_visual(*visual))
        .collect();

        assert_eq!(
            reached,
            vec![
                NeraExpression::Neutral,
                NeraExpression::Happy,
                NeraExpression::Angry,
                NeraExpression::Sad,
                NeraExpression::Surprised,
            ],
        );
    }

    #[test]
    fn alarm_presentation_reads_as_surprised() {
        assert_eq!(
            expression_for_visual(VisualState::AlarmWave),
            NeraExpression::Surprised,
        );
        assert_eq!(
            expression_for_visual(VisualState::AlarmJump),
            NeraExpression::Surprised,
        );
    }

    #[test]
    fn startup_wave_greets_happily_without_using_the_alarm_face() {
        assert_eq!(
            expression_for_visual(VisualState::StartupGreeting),
            NeraExpression::Happy,
        );
        assert_ne!(
            expression_for_visual(VisualState::StartupGreeting),
            expression_for_visual(VisualState::AlarmWave),
        );
    }

    #[test]
    fn sitting_and_praying_settle_into_relaxed() {
        assert_eq!(
            expression_for_visual(VisualState::SittingLoop),
            NeraExpression::Relaxed,
        );
        assert_eq!(
            expression_for_visual(VisualState::SittingEnter),
            NeraExpression::Relaxed,
        );
        assert_eq!(
            expression_for_visual(VisualState::Praying),
            NeraExpression::Relaxed,
        );
    }

    #[test]
    fn transitional_states_hand_the_face_back_to_the_clip() {
        for visual in [
            VisualState::Idle,
            VisualState::SittingExit,
            VisualState::Landing,
        ] {
            assert_eq!(
                expression_for_visual(visual),
                NeraExpression::Neutral,
                "{visual:?}",
            );
        }
    }

    /// These menu clips are body-only. The expression mapping supplies the
    /// facial intent that the VRMA itself does not contain.
    #[cfg(target_os = "windows")]
    #[test]
    fn menu_expressions_agree_with_their_own_clips() {
        use crate::action_menu::AnimationMenuChoice as Menu;

        assert_eq!(expression_for_menu(Menu::Angry), NeraExpression::Angry,);
        assert_eq!(expression_for_menu(Menu::Sad), NeraExpression::Sad,);
        assert_eq!(
            expression_for_menu(Menu::Surprised),
            NeraExpression::Surprised,
        );
        assert_eq!(expression_for_menu(Menu::Wave), NeraExpression::Happy,);
    }

    #[test]
    fn idle_micro_actions_get_a_calm_face() {
        use crate::animation::IdleMicroAction as Fidget;

        assert_eq!(
            expression_for_idle_micro(Fidget::LookAround),
            NeraExpression::Neutral,
            "scanning the room is attention, not emotion",
        );
        assert_eq!(
            expression_for_idle_micro(Fidget::Relax),
            NeraExpression::Relaxed,
        );
        assert_eq!(
            expression_for_idle_micro(Fidget::Sleepy),
            NeraExpression::Relaxed,
        );
        assert_eq!(
            expression_for_idle_micro(Fidget::Blush),
            NeraExpression::Happy,
        );
    }

    #[test]
    fn a_fidget_shows_through_the_visual_state_mapping() {
        use crate::animation::IdleMicroAction as Fidget;

        assert_eq!(
            expression_for_visual(VisualState::IdleMicro(Fidget::Sleepy)),
            NeraExpression::Relaxed,
        );
        assert_eq!(
            expression_for_visual(VisualState::IdleMicro(Fidget::LookAround)),
            NeraExpression::Neutral,
        );
    }

    #[test]
    fn seated_dodge_releases_its_surprise_back_to_the_relaxed_seated_face() {
        let mut state = NeraExpressionState::default();
        let dodge = expression_for_visual(VisualState::DodgeWhileSitting);
        assert_eq!(dodge, NeraExpression::Surprised);
        state.advance(dodge, Duration::from_secs(1));
        let sitting = expression_for_visual(VisualState::SittingLoop);
        state.advance(sitting, Duration::ZERO);
        state.advance(sitting, Duration::from_secs(1));
        assert_eq!(
            state.weights().live().collect::<Vec<_>>(),
            vec![("relaxed", 1.0)],
        );

        for action in IdleMicroAction::SITTING {
            assert_eq!(
                expression_for_visual(VisualState::IdleMicro(action)),
                sitting,
            );
        }
    }

    #[test]
    fn new_standing_fidgets_keep_body_and_face_intent_together() {
        for (action, face) in [
            (IdleMicroAction::CatwalkIdleTwistR, NeraExpression::Neutral),
            (IdleMicroAction::Bored, NeraExpression::Relaxed),
            (IdleMicroAction::AngryGesture, NeraExpression::Angry),
        ] {
            assert_eq!(expression_for_visual(VisualState::IdleMicro(action)), face,);
        }
    }

    // ---------------------------------------------------------------------
    // Crossfade progression
    // ---------------------------------------------------------------------

    #[test]
    fn advance_fades_and_then_settles() {
        let mut state = NeraExpressionState::default();

        state.advance(NeraExpression::Happy, Duration::from_millis(60));
        assert_eq!(state.current, NeraExpression::Happy);
        assert_eq!(state.previous, NeraExpression::Neutral);
        assert!(state.blend > 0.0 && state.blend < 1.0);

        // Neutral has no preset, so early in the fade only `happy` is live,
        // and only at partial weight.
        let live: Vec<_> = state.weights().live().collect();
        assert_eq!(live.len(), 1, "{live:?}");
        assert_eq!(live[0].0, "happy");
        assert!(
            live[0].1 > 0.0 && live[0].1 < 1.0,
            "expected a partial weight, got {:?}",
            live,
        );

        for _ in 0..10 {
            state.advance(NeraExpression::Happy, Duration::from_millis(60));
        }

        assert_eq!(state.blend, 1.0);
        assert_eq!(
            state.weights().live().collect::<Vec<_>>(),
            vec![("happy", 1.0)],
        );
    }

    #[test]
    fn a_settled_state_stops_advancing() {
        let mut state = NeraExpressionState::default();

        state.advance(NeraExpression::Angry, Duration::from_secs(1));
        assert_eq!(state.blend, 1.0);

        for _ in 0..100 {
            state.advance(NeraExpression::Angry, Duration::from_secs(1));
        }

        assert_eq!(state.blend, 1.0);
        assert_eq!(state.current, NeraExpression::Angry);
    }

    #[test]
    fn changing_target_restarts_the_fade_from_the_outgoing_expression() {
        let mut state = NeraExpressionState::default();

        state.advance(NeraExpression::Happy, Duration::from_secs(1));
        assert_eq!(state.blend, 1.0);

        state.advance(NeraExpression::Sad, Duration::from_millis(0));

        assert_eq!(state.current, NeraExpression::Sad);
        assert_eq!(state.previous, NeraExpression::Happy);
        assert_eq!(state.blend, 0.0);
    }

    /// A zero or negative delta must not rewind or stall the fade.
    #[test]
    fn zero_delta_does_not_rewind_a_crossfade() {
        let mut state = NeraExpressionState::default();

        state.advance(NeraExpression::Happy, Duration::from_millis(60));
        let before = state.blend;

        state.advance(NeraExpression::Happy, Duration::ZERO);

        assert_eq!(state.blend, before);
    }
}
