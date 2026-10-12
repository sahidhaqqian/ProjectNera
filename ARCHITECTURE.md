# Project Nera Architecture

> Current architecture reference for the Phase 7 codebase.
>
> **Source of truth:** the code. This document records stable ownership, process boundaries, state flow, scheduling constraints, and where features belong. Feature-specific tuning/results stay in focused docs such as `CHIBI_MODE_V1.md`, `CHIBI_PROPORTION_TUNING.md`, `TOUCH_REACTION_POLISH.md`, and the memory-audit documents.

---

## 1. Architecture at a glance

Project Nera is a Windows 3D desktop pet built with Rust 2024, Bevy 0.19, and `bevy_vrm1` 0.9.2. The primary Bevy window is transparent, undecorated, compact, and backed by native Win32 desktop integration.

Nera is organized around **explicit state owners** instead of one giant state machine:

| Concern | Authoritative owner |
|---|---|
| Desktop/native snapshot + attachment facts | `DesktopEnvironment` |
| Free standing gravity/throw/support | `DesktopPhysicsState` |
| Pointer capture and click/drag classification | `DesktopPetInteraction` |
| Body animation state | `NeraAnimationState` |
| Runtime animation playback/clip registry | `NeraAnimationPlayback` / `NeraAnimationClips` |
| Normal/Chibi selection | `AvatarBodyMode` |
| Applied Chibi rig overlay | `BodyProportions` |
| Rest/Ride placement | `CloudRuntime` while attached |
| DesktopPet/Attention presentation | `PresentationState` |
| Automatic face | `NeraExpressionState` |
| Low-priority autonomous choices | `PetBehavior` |
| Local Windows music evidence / hysteresis | `MusicDetectionState` (background Media Session + WASAPI observers) |
| Curated AutoDance clip references | `DancePool` / Animation Manager registry metadata |
| Active destination walk | `WalkingRuntime` |
| Alarm deadlines/ringing | `AlarmState` |
| Persistent app configuration | `NeraSettings` |
| Live character selection | `ActiveCharacter` |
| Character catalog/companions | `CharacterCatalog` / `CharacterInstances` |
| Semantic audio/visual feedback | `PetSfx` / `PetFx` pipelines |

Core rule:

> A feature may request or temporarily suspend another subsystem, but it must not create a second owner for the same state.

Examples:

- Cloud may suspend physics placement, but does not become a second gravity system.
- Alarm may enter Attention presentation, but does not permanently rewrite normal desktop placement.
- Chibi may overlay evaluated skeleton proportions, but does not replace the VRM/VRMA pipeline.
- Touch reactions request animation; they do not directly own a competing `AnimationPlayer`.

---

## 2. Runtime/process topology

```text
Main Nera process
├─ primary transparent desktop-pet Bevy window
├─ Settings Bevy utility window                [when open]
├─ Alarm Settings Bevy utility window          [when open]
├─ Alarm dismiss surface                       [while ringing]
├─ native tray/message-loop thread
├─ native radial popup
├─ optional companion Nera processes
└─ Animation Manager host
   └─ separate Animation Manager worker process
      ├─ gallery renderer/UI
      ├─ animation/expression previews
      └─ character-card presentation

Preview generation
└─ disposable Animation Manager worker process
   └─ renders full idle preview cache and exits
```

### Animation Manager isolation

The gallery runs in a worker launched with `--animation-manager-worker`. This is intentional: renderer/driver allocations can retain high-water memory even after assets are despawned. Closing the worker lets the OS reclaim that renderer lifetime instead of leaving it attached to the main pet.

### Multi-character isolation

Additional desktop characters are separate instances of the same executable launched with `--character-instance`. Each companion gets its own renderer, window, settings identity, animation state, physics, gaze, and live model. The main instance owns the roster and lifecycle.

---

## 3. Bootstrap and main composition

`src/main.rs` is the composition root. It should contain plugin registration and **cross-subsystem ordering**, not feature implementation.

Startup order:

1. Detect special worker/process modes.
2. Create the Bevy `App`.
3. Load `NeraSettings` before primary-window creation.
4. Apply companion-instance settings identity if relevant.
5. Resolve Start Hidden before creating the HWND.
6. Register the private character asset source.
7. Install Bevy, VRM, VRMA, Nera domain, and Windows-only plugins.
8. Initialize cross-domain resources.
9. Register the explicit main Update/PostUpdate/Last schedule.
10. Optionally install smoke/benchmark instrumentation.
11. Run the app.

Primary-window defaults currently include transparent/undecorated rendering, `AlwaysOnTop`, taskbar skip, fixed size, and disabled native cursor hit-test until Nera's interaction pipeline enables it.

---

## 4. Module map

```text
src/
├─ main.rs                              bootstrap + cross-subsystem schedule
│
├─ avatar.rs                            avatar/camera/light/gaze
│  ├─ avatar/anchors.rs                 feet/seat projection + visual grounding
│  └─ avatar/body_mode.rs               Normal/Chibi owner + reversible overlay
│     ├─ body_mode/springs.rs           spring runtime adapter
│     ├─ body_mode/leg_tests.rs         leg-proportion regressions
│     └─ body_mode/smoke.rs             native Chibi smoke support
│
├─ interaction.rs                       pointer capture + semantic messages
│  ├─ interaction/hit_testing.rs        animated body colliders/raycasting
│  ├─ interaction/headpat.rs            button-free head-stroke recognizer
│  └─ interaction/momentum.rs           drag velocity / throw sampling
│
├─ animation.rs                         animation plugin boundary
│  ├─ animation/state.rs                vocabulary, priorities, authoritative state
│  ├─ animation/systems.rs              events/environment -> requests
│  ├─ animation/playback.rs             VRMA registry/loading/playback mapping
│  ├─ animation/library.rs              gallery-selected desktop playback
│  ├─ animation/walking_loop.rs         steady-stride loop handling
│  ├─ animation/limbs.rs                procedural Leg/Foot reactions
│  └─ animation/touch.rs                authored Hand/Arm resolver
│     ├─ touch/seated.rs                seated lower-body preservation
│     ├─ touch/tests.rs                 touch regression tests
│     └─ touch/smoke.rs                 native touch smoke support
│
├─ expression.rs                        facial ownership/crossfade
│  ├─ expression/basic.rs               protected VRMA -> face intent
│  ├─ expression/presets.rs             semantic face recipes
│  └─ expression/face.rs                detailed morph-target layer
│
├─ environment.rs                       native desktop snapshot
│  ├─ environment/taskbar.rs            taskbar discovery/geometry
│  └─ environment/sitting.rs            attached placement/following
├─ desktop_physics.rs                   gravity/throw/standing support
│  └─ desktop_physics/geometry.rs       physical-pixel support geometry
├─ pet_behavior.rs                      autonomous decision owner
├─ walking.rs                           destination locomotion executor
├─ startup_entrance.rs                  one entrance per process
│
├─ cloud.rs                             Rest/Ride placement owner
│  ├─ cloud/bounds.rs                   visible bounds
│  ├─ cloud/pose.rs                     cloud seat/sleep stabilization
│  ├─ cloud/sleep_arms.rs               lying-sleep arm overlay
│  ├─ cloud/visual.rs                   GLB/procedural cloud visual
│  ├─ cloud/tests.rs                    regressions
│  └─ cloud/smoke.rs                    native smoke support
│
├─ pet_feedback.rs                      semantic feedback bridge
│  ├─ pet_feedback/sfx.rs               bounded semantic audio
│  └─ pet_feedback/particles.rs         bounded short-lived FX
│
├─ presentation.rs                      DesktopPet <-> Attention
├─ alarm.rs                             scheduler/ringing lifecycle
│  ├─ alarm/sounds.rs                   sound IDs/playback/preview boundary
│  └─ alarm/sounds/library.rs           imported alarm-sound catalog
├─ alarm_settings.rs                    alarm manager/editor UI
│  └─ alarm_settings/memory_audit.rs    opt-in memory audit
├─ alarm_dismiss.rs                     ringing dismiss/snooze surface
│
├─ character.rs                         validated live model replacement
│  ├─ character/roster.rs               roster protocol vocabulary
│  └─ character/library.rs              catalog + companion owner
│     ├─ library/import.rs               import/fingerprint work
│     ├─ library/previews.rs             preview-cache preparation
│     ├─ library/removal.rs              safe model removal
│     └─ library/release.rs              companion release handshake
├─ vrm0_migration.rs                    in-memory VRM 0.x -> 1.0 migration
│
├─ animation_manager.rs                 worker-side gallery application
│  ├─ animation_manager/process.rs       host/worker IPC/process lifecycle
│  ├─ animation_manager/registry.rs      animation catalog/bookmarks
│  ├─ animation_manager/base.rs          protected runtime animation contract
│  ├─ animation_manager/preview.rs       bounded independent preview rigs
│  ├─ animation_manager/thumbnails.rs    thumbnail/idle-preview cache front end
│  ├─ animation_manager/idle_cache.rs    compressed idle-card sequence cache
│  ├─ animation_manager/generation.rs    disposable preview renderer
│  ├─ animation_manager/characters.rs    character gallery/roster UI bridge
│  ├─ animation_manager/combine.rs       VRMA trim/combine/export
│  ├─ animation_manager/ui.rs            gallery UI
│  ├─ animation_manager/memory_audit.rs  worker memory audit
│  ├─ combine_loader_tests.rs            real exported-VRMA load checks
│  └─ text_input_tests.rs                editable-text regression tests
│
├─ settings/
│  ├─ mod.rs                             plugin/public boundary
│  ├─ model.rs                           persistent schema/defaults
│  ├─ runtime.rs                         runtime sync + persistence
│  ├─ ui.rs                              Settings window
│  ├─ shadow.rs                          simple/character shadows
│  ├─ startup.rs                         Windows startup registry
│  ├─ memory.rs                          font-cache lifecycle bridge
│  └─ memory_audit.rs                    opt-in Settings audit
│
├─ utility_window.rs                     shared utility-window shell
│  ├─ utility_window/font_cache.rs       scoped glyph-cache ownership
│  └─ utility_window/text_input.rs       shared editable-text behavior/style
│
├─ action_menu.rs                        Win32 radial wheel
├─ tray.rs                               Bevy-side tray bridge
│  └─ tray/native.rs                     native icon/menu/message loop
├─ platform/mod.rs                       native facade
│  └─ platform/windows.rs                Win32 implementation
├─ memory.rs                             VRM CPU/GPU retention tuning
├─ benchmark.rs                          opt-in full-app memory benchmark
├─ feature_smoke.rs                      debug integration smoke harness
├─ rng.rs                                deterministic non-crypto PRNG
└─ bin/{mem_minimal,mem_bevy,mem_stages}.rs

examples/
├─ chibi_validation.rs
├─ chibi_support_fixture.rs
└─ touch_validation.rs
```

---

## 5. Frame scheduling and pose ordering

Ordering is part of Nera's architecture because several systems modify native placement or evaluated VRM pose.

### First / early native facts

- `environment::sync_desktop_environment` obtains/updates the primary HWND, rect, DPI, and monitor.
- `utility_window` acquires HWNDs for temporary utility windows.

### PreUpdate

Before normal behavior/animation requests:

- refresh native-window snapshot;
- track current support window;
- refresh taskbars;
- follow sitting attachments;
- restore temporary previous-frame pose overlays before animation writes a new pose.

Examples of reversible overlays restored here include Chibi proportions, procedural Leg/Foot offsets, visual grounding, and Cloud sleep pose layers.

### Main Update chain

`main.rs` keeps this cross-domain sequence explicit:

```text
mark live VRM bones
→ attach hit colliders
→ update look-at target
→ suspend pet input if utility UI owns input
→ update Cloud semantic state
→ native drag/movement
→ startup-entrance preparation
→ desktop physics/support
→ interaction -> animation requests
→ autonomous pet behavior
→ support/environment -> animation state
→ walking executor
→ age animation actions
→ startup-entrance progression
→ look-at ownership
→ apply animation state
```

Plugin-local systems are ordered around the chain when necessary, e.g. library playback, alarm requests, action-menu requests, and lazy clip admission.

### PostUpdate evaluated-pose pipeline

Conceptually:

```text
Bevy AnimationSystems
→ walking loop / authored pose
→ walking + seated corrections
→ Cloud sleeping/seat pose corrections
→ Chibi proportion overlay
→ VRM constraints / propagation
→ visual grounding
→ gaze + expression passes
→ Chibi spring-state adjustment
→ SpringBone
→ click-through
→ headpat detection
```

Read actual `.before(...)` / `.after(...)` constraints before changing any pose layer.

### Last

Late consumers include:

- live feet/seat anchor projection;
- pending sitting commit after the animated Seat exists;
- cloud logical-seat alignment and visual bobbing;
- particle update/expiry;
- Attention presentation maintenance.

### Overlay invariant

> A pose layer must remove **only its own previous-frame delta**, then reapply after the pose owners it depends on. Never restore an entire stale transform and erase another owner.

---

## 6. Platform, environment, sitting, and physics

### `platform/*`

Raw Win32 work belongs behind the platform facade. It covers native window handles/rects, global cursor coordinates, monitor/DPI/work-area information, external-window enumeration, taskbar/native shell behavior, z-order, local date/time, and related Windows operations.

Domain code should prefer `platform::*` helpers to scattered direct Win32 calls.

### `DesktopEnvironment`

`DesktopEnvironment` contains the current desktop/native snapshot:

- Nera HWND/rect/DPI/scale factor;
- monitor;
- discovered external windows;
- tracked support window;
- taskbar state;
- feet/seat projected anchors;
- `PlacementState` and current attached anchor;
- temporary hiding when an attached target is unavailable.

Window enumeration is periodic rather than per-frame; support following and other placement-sensitive facts update as needed.

### Sitting

`environment::sitting` owns native attachment/follow behavior for supported windows/taskbar edges. Animation owns the seated visual state. Neither should replace the other.

Walking/autonomy must not implicitly stand Nera up. Sitting/standing changes come from user placement or support validity.

### Desktop physics

`DesktopPhysicsState` owns free standing motion in global physical desktop pixels:

- gravity;
- vertical velocity;
- bounded throw impulse;
- desktop/work-area floors;
- taskbar/window-top standing support;
- support loss/recovery;
- horizontal walking movement through the same native placement path.

Physics yields while another explicit placement owner is active: sitting attachment, Cloud, or Attention.

---

## 7. Interaction pipeline

### Semantic body vocabulary

```text
Head
Torso
LeftArm / RightArm
LeftHand / RightHand
LeftLeg / RightLeg
LeftFoot / RightFoot
```

Hit testing follows animated live VRM bones. Side-specific semantic values are retained even when higher-level behavior currently treats both sides alike.

### `DesktopPetInteraction`

This is the single pointer-gesture owner. It tracks hover, press, drag threshold/capture, release, right-click, throw momentum, and sitting unsnap safeguards.

The body part is captured at press time, so animation/cursor motion cannot change the meaning of an in-progress gesture.

Semantic messages include:

- `NeraClicked`
- `NeraRightClicked`
- `NeraDragStarted`
- `NeraDragged`
- `NeraReleased`
- `NeraThrow`
- `NeraHeadPatted`

New feature code should consume these instead of reimplementing raw mouse classification.

### Headpat

`interaction/headpat.rs` recognizes deliberate back-and-forth cursor movement over the head without requiring LMB. It is intentionally separate from click/drag capture.

### Input ownership

Settings and Alarm Settings block new pet gestures only while the pointer is
over their utility window or a close gesture awaits release. Opening either
editor leaves the pet draggable. A captured pet gesture retains its release
across utility hover; new presses must originate from the primary pet window,
since Bevy's global mouse button resource includes utility clicks. Autonomy
still yields while the editors are open. Animation Manager and Attention retain
their existing input ownership. Blocked input and presentation entry/exit reset
in-flight pointer state so missed releases cannot leave the pet swallowing clicks.

---

## 8. Animation architecture

### Authoritative state

`NeraAnimationState` owns the desktop avatar's body state. It separates:

- base pose (`Idle` / `Sitting`);
- support transitions;
- temporary/persistent actions;
- sitting-position choice;
- action priority;
- real one-shot duration;
- damage combo state;
- a revision used to intentionally restart repeated requests.

`VisualState` is the resolved answer consumed by playback, expressions, gaze, feedback, and other pose-aware systems.

### Conceptual priority

```text
Walking
< Idle variation
< Startup greeting
< Reactions / library animation
< Menu pose
< Floating
< Physics
< Critical / death
< Alarm
< Alarm intro
```

Do not bypass this by starting a parallel `AnimationPlayer` from feature code.

### Playback ownership

`animation/playback.rs` owns:

- runtime/protected VRMA paths;
- clip registration/load observers;
- measured durations;
- loop/one-shot behavior;
- visual-state -> clip mapping;
- playback/blend application;
- lazy clip admission where applicable.

One-shot lifetime should come from loaded VRMA duration, not duplicated guessed timers.

### Animation library playback

`animation/library.rs` turns gallery selection into a normal desktop animation request. The manager is not a second desktop playback owner.

For manually selected clips, `animation/library_motion.rs` holds a temporary placement lease and a checkpoint of the native position, support and attachment. Its reversible hips translation overlay samples the original VRMA at the existing player's playback time, transfers horizontal displacement to the native window once, and preserves authored vertical motion. Translation retargeting uses rest hips-to-feet distances rather than absolute scene Y, so desktop root placement cannot collapse the motion. Cached clips remain unchanged for AutoDance and other uses. Grounded poses use evaluated humanoid contact; clips with an authored rise retain their airborne trajectory.

Physics, sitting followers, anchor recalibration, autonomous behavior and position persistence yield during this lease. Normal completion eases back to the original checkpoint, following its supporting window if it moved. Drag, Cloud, presentation, loading/replacement or an external relocation discard the checkpoint instead of pulling Nera away from the new owner. AutoDance remains on its existing placement path. This layer reads the current animation graph; it does not control another player or alter the Chibi proportion overlay.

### Walking animation

`animation/walking_loop.rs` loops the steady gait portion of the walking VRMA instead of repeatedly replaying authored lead-in/slowdown frames.

---

## 9. Touch reaction architecture

Touch is now split by semantic type, while `NeraAnimationState` remains the body-pose owner.

### Hand/Arm: authored VRMA

`animation/touch.rs` owns admission, cooldown/history, and selection for Hand/Arm reactions.

Protected runtime clips:

```text
Hand normal
├─ wave1.vrma
├─ wave2.vrma
├─ salute.vrma
└─ thankful.vrma

Hand escalation
├─ Dismissive Gesture.vrma
└─ Shrugging.vrma

Arm
├─ Arm Stretching.vrma
└─ Shoulder Rubbing.vrma
```

The semantic repeat sequence is tracked by `TouchReactionRuntime`. Only an **accepted** reaction advances history/cooldown. Blocked states, drag, missing/zero clip duration, or cooldown rejection do not consume a step.

`TouchReactionAccepted` is emitted after successful admission so PetFx/PetSfx can react to actual semantic outcomes rather than raw clicks.

### Seated authored reactions

`animation/touch/seated.rs` protects the calibrated seated lower body when authored Hand/Arm clips contain lower-body motion. Upper-body touch reactions therefore do not detach Nera from a seat/cloud visually.

### Leg/Foot: procedural overlay

`animation/limbs.rs` now owns only Leg/Foot additive motion. Arm/Hand procedural offsets intentionally evaluate to zero and are regression-tested.

The filename is historical; renaming it later is hygiene, not a behavior refactor.

---

## 10. Expressions and gaze

### Automatic expression ownership

`expression.rs` derives facial state from the **resolved visual body state**, not directly from raw clicks. This prevents body/face disagreement when an animation request is interrupted or reprioritized.

`expression/basic.rs` gives protected runtime clips intentional facial presets/intensities. `expression/face.rs` handles detailed Nera-specific named morph targets around the VRM expression pass, while imported avatars fall back to available standard VRM expressions.

### Manual preview

Animation Manager expression preview is an explicit temporary owner. Default/reset/0% releases that ownership; it should not force a permanent neutral override. Body VRMA playback does not implicitly clear the user's selected preview expression.

### Gaze

`avatar.rs` owns look-at target and state-aware gaze enabling. High-priority animation/presentation can suppress gaze. Rest Cloud explicitly disables cursor head-follow while sleeping and restores normal ownership on exit.

---

## 11. Avatar anchors, grounding, and Chibi

### Anchors

`avatar/anchors.rs` projects final live pose into desktop support information:

- feet contact for standing;
- hips/seat contact for sitting.

Support code should consume evaluated avatar geometry rather than HWND-bottom guesses.

`AvatarVisualGrounding` is a reversible visual alignment layer, not a second physics owner.

### Chibi semantic state

`AvatarBodyMode` is the one user-visible owner:

```text
Normal
Chibi
```

`NeraSettings.body_mode` only persists it; radial UI only requests/toggles it.

### Chibi rig overlay

`BodyProportions` applies a reversible runtime overlay to the **same VRM and same VRMA pose**:

1. capture/validate the current humanoid rig;
2. restore only Nera's previous Chibi overlay in PreUpdate;
3. let animation/walking/sitting/cloud evaluate normally;
4. apply body/head/upper-leg proportion changes;
5. derive support correction from real rig geometry rather than hardcoded pixels;
6. let constraints/gaze/expressions/springs see final proportions;
7. restore exactly on Normal or character replacement.

Spring handling is isolated in `body_mode/springs.rs` because `bevy_vrm1` keeps solver runtime details private. Dependency-schema compatibility is regression-tested.

Detailed Chibi proportions and native validation belong in `CHIBI_MODE_V1.md` / `CHIBI_PROPORTION_TUNING.md`.

---

## 12. Cloud architecture

`CloudRuntime` is a temporary placement owner with:

```text
CloudMode::Rest
CloudMode::Ride

Hidden → Appearing → Attached → Disappearing → Hidden
```

### Logical versus visual placement

Cloud stores one logical desktop seat point. Visual bobbing never changes that logical support point.

### Rest

Rest is a temporary bed/sleep state:

- chooses/holds a Rest pose per attachment;
- applies sleep orientation/arm overlays when needed;
- suppresses normal touch/headpat and gaze while asleep;
- dragging detaches the Rest cloud;
- alarms, character replacement, explicit dismissal, and other defined transitions release ownership through the normal lifecycle.

### Ride

Ride is a floating seat/mount:

- avatar + cloud move together;
- release remains floating;
- throw/gravity/native support snap are suspended while Ride owns placement;
- while dragging, native movement follows the pointer without a competing Cloud correction;
- on release, monitor bounds constrain the head/sides and logical seat. Feet/cloud may overlap the taskbar, so animated feet do not become a moving bottom barrier.

### Ordering

Cloud pose correction runs after animation and before Chibi proportions/VRM constraints. Chibi therefore scales the final cloud-compatible pose rather than scaling cached cloud geometry twice.

---

## 13. Semantic PetFx and PetSfx

Features emit semantic feedback messages instead of creating private effect engines.

```text
PetSfx(PetSfxKind::...)
PetFx { kind, anchor }
```

`pet_feedback/sfx.rs` reuses Bevy Audio, master volume/mute, bounded concurrent cues, and safe missing-file behavior. Bundled CC0 cues and their provenance live in `assets/sound/pet/`. Cue gain is stored in `PlaybackSettings` and retained when Settings updates active audio sinks. Greeting is emitted once on entry to startup/screensaver waving; screensaver admits only greeting and headpat cues.

`pet_feedback/particles.rs` owns shared meshes/materials, bounded particle count, short lifetimes, and anchors such as head/desktop point.

Current semantic uses include headpat hearts, anger marks, landing/cloud puffs, sleep Zs, and reaction cues.

A new feature should normally add/reuse a semantic kind, not create a second audio/particle stack.

---

## 14. Autonomous behavior and walking

`PetBehavior` decides low-priority actions; `WalkingRuntime` executes a chosen destination.

Living Behavior V1 adds two routines inside that same brain. Cursor Play consumes read-only desktop cursor facts shared with gaze; its Notice/Reach/Catch/Escape stages use the existing idle-action priority, expression mapping and PetFx/PetSfx. A reversible humanoid two-bone reach composes after animation and body proportions, before VRM constraints, and restores its original rotations in PreUpdate. It never captures or moves the operating-system pointer. Missing/unusable arm chains leave normal gaze intact.

Edge Peek is a monitor-floor-only WalkingRuntime journey: approach an exposed edge, briefly hide part of the canvas, emerge, then return to the pre-journey position. Physics grants bounded partial-canvas movement while keeping the measured foot contact inside the support plane; ordinary roaming/window-top bounds remain unchanged. Monitor seams, support changes, drag/user relocation, loading, Cloud and Attention interrupt the routine. Attention restores its own placement before pending edge recovery; a user/Cloud relocation invalidates that recovery. Temporary edge positions are excluded from position persistence. Cursor Play and Edge Peek preferences live in NeraSettings; Edge Peek also requires Allow Roaming.

### `PetBehavior`

Owns:

- idle/fidget/walk choice;
- recent-action history;
- home/support identity;
- activity-dependent cooldowns;
- roaming bounds and eligibility;
- interruption gates.

It does **not** move HWNDs directly and does not decide sitting/standing.

### `WalkingRuntime`

Owns:

- support-relative destination intent;
- look-around preparation;
- direction/turn delay;
- acceleration, velocity, sub-pixel accumulation, braking;
- arrival/interruption outcome.

Movement is committed through `DesktopPhysicsState`. Invalid support/monitor/placement cancels the journey rather than continuing stale intent.

Autonomy yields to user interaction, dragging, physics, alarm/Attention, explicit animations, utility windows/manager ownership, hidden state, and fullscreen-pause rules.

---

## 15. Presentation and alarm

### Presentation

`presentation.rs` separates the normal desktop world from temporary prominent presentation.

```text
PresentationMode::DesktopPet
PresentationMode::Attention(Alarm)
PresentationMode::Attention(Screensaver)
```

Entering Attention snapshots enough window/camera/placement state to restore normal desktop behavior afterward. The same compact transparent Bevy window is repositioned/reframed rather than replaced by a monitor-sized transparent canvas.

Alarm framing uses a canonical layout for the sequence so switching alarm animation A -> B does not suddenly rescale the avatar from live animated bounds.

Presentation priority is `Alarm > Screensaver > DesktopPet`. A due Alarm may
preempt Screensaver directly, retaining the original desktop snapshot; Alarm
dismissal restores DesktopPet instead of resuming Screensaver. Alarm keeps its
existing 0.78 monitor-height canvas, 0.80 upper-body viewport fill, 0.84
horizontal fill, 0.88 width limit and 0.38-second bottom-anchored entrance.

Screensaver is the seventh radial root action (Games remains disabled). It
reuses the primary transparent window, camera, loaded VRM and animation owner.
Its orthographic close-up frames the measured crown/hair through the humanoid
hips, with 0.96 portrait fill inside a centered 0.94-monitor-height canvas and
0.88-monitor-width canvas. Legs intentionally extend below the viewport.
Outstretched arms or wide accessories may crop at the sides; their span does
not shrink the portrait. Read-only compact rest skin/humanoid measurements
include hair/clothing and the current body/head proportions.
Only live humanoid bones supply landmarks; child VRMA source rigs are excluded.
The existing Chibi standing correction supplies the thigh adjustment; framing
does not change avatar scale or the Chibi overlay. Missing/unready geometry
rejects entry with a log. A valid Screensaver request first dismisses Rest/Ride
Cloud, then captures the free desktop state so exit cannot restore a stale Cloud seat.

Screensaver uses the existing presentation snapshot/restore and pointer reset.
Only a left/right press received by Nera's primary window on an existing body
collider exits, after a 0.35-second entry grace. Existing click-through hit
testing and button-free headpat stay active; desktop drag/click-touch reactions
stay suspended. Headpat uses the existing HeadStroke, expression and PetFx/PetSfx
pipeline, interrupts the presentation clip, then returns to Breathing Idle.
Cursor movement, scrolling, keyboard activity and clicks in other windows do
not dismiss Screensaver. Eyes target the viewer at evaluated eye level rather
than the cursor; head/spine tracking stays off so VRMA owns the head/body pose.
Saved follow preferences remain intact and resume on exit.
Both simple and character shadows, including shadow-map rendering, are hidden
during Attention and resume from the saved desktop shadow preference on exit.
`animation/screensaver.rs` schedules a lazy Waving greeting, looping Breathing
Idle, and seven protected one-shot idle variations separated by random 30–60
second breathing intervals without immediate repeats. The selected clip is a
priority-55 action in `NeraAnimationState`, below headpat/reactions and Alarm;
all clips use `NeraAnimationClips` and the same playback controller. Dwarf Idle
2 gets a two-second PetFx firefly lead-in before the clip starts. The tiny
screen-sized yellow insect waits beside the evaluated eyes, then flees along
the authored hand sweep. Exit, Alarm preemption and rig replacement release this routine.
Its conditional exit command cannot dismiss an Alarm that preempted it.
Presentation positions remain temporary and are not saved as normal placement.
Desktop feet/seat calibration is suspended during Attention. After restoration,
physics and pointer gestures resume only once native window facts match the
restored canvas; temporary presentation pixels never replace support anchors.

The opt-in native smoke harness disables physical pointer capture and injects
Bevy window events through the same click handler, so validation can run while
the user works. It checks Normal/Chibi click exit and headpat, viewer eye gaze,
all nine real VRMAs and the firefly lead-in, ignored scroll/cursor/keyboard,
continued VRMA playback and gaze suppression without controlling desktop input.

Screenshot is a tray-only command, between separators before Exit. The existing
tray channel requests Bevy `Screenshot::window` on the main thread without
resizing or changing presentation. `presentation/screenshot.rs` retains RGBA
(Bevy's stock `save_to_disk` discards alpha) and atomically reserves timestamped
PNG filenames with numeric collision suffixes. It captures only Nera's render
target, including current pose, Chibi, Cloud and feedback. Files go to the
Windows Known Folder Pictures / `NeraScreenshots`, with USERPROFILE/Pictures
fallback. Directory, decoding and write failures are logged without panics.

### Alarm runtime

`alarm.rs` owns:

- timer/scheduled deadlines;
- pending alarm ordering;
- ringing state;
- Attention entry/exit;
- alarm animation lifecycle;
- selected sound playback/repeat;
- snooze runtime state.

The editor does not own scheduler state.

### Alarm Settings

`alarm_settings.rs` owns the alarm-manager window, draft/edit state, quick-preset UI, and persistent `AlarmConfig` edits. Alarms and Focus use separate tabs. Focus emits `FocusCommand`; `FocusSession` owns the independent Study/Work clock and activity intent. Its persistent `FocusActivity` requests use the existing animation owner below user reactions, and temporary props follow the resolved activity. Focus modifies `PetBehavior` eligibility and semantic PetSfx gain without rewriting roaming/audio preferences or touching real alarm deadlines. User interactions pause the activity; critical presentations end it safely. Only last mode/duration choices persist. Completion uses the existing animation/expression/feedback owners and submits a one-shot Windows notification through the existing tray icon, independently of real alarm scheduling. See `FOCUS_ACTIVITIES_V2.md`.

The radial Alarm action opens Alarm Settings directly. 5m/10m/15m/30m/45m/1h/2h presets now live in that utility window.

The active timer card and selected preset read `AlarmState` after runtime processing. They show the accepted timer's remaining time, local due time, and ringing state, including after reopening the editor. Quick timers remain runtime deadlines; the saved alarm list represents persistent `AlarmConfig` entries.

### Alarm sounds

`alarm/sounds/library.rs` owns imported alarm-sound storage/catalog, validation, stable IDs, duplicate identity, deletion cleanup, safe missing-file fallback, and Random/default/custom selection compatibility.

Built-in sound IDs are explicitly mapped to asset slots. The original IDs 0–4
remain stable; additional built-ins occupy the reserved IDs immediately below
STOP_PREVIEW/Random. Imported IDs still start at 5 and do not change when the
built-in choices grow.

Editor preview audio is editor-owned. Closing Alarm Settings stops preview only; it must not stop a real ringing alarm.

---

## 16. Settings and shared utility UI

### Persistent schema

`settings/model.rs` is the persistent settings schema. Runtime resources remain authoritative for live transient state; settings encode durable preference.

Do not duplicate OS-owned facts. Windows startup registration is a good example: `settings/startup.rs` treats the registry as source of truth rather than maintaining an unrelated boolean owner.

### Runtime sync

`settings/runtime.rs` applies settings, handles deferred/synchronous save, and persists user placement without recording temporary Alarm/Attention/startup movements as the preferred desktop position.

### Utility windows

`utility_window.rs` provides common native behavior for custom Bevy utility shells: HWND acquisition, minimize/maximize, and native resize-edge handling.

`utility_window/font_cache.rs` provides scoped utility glyph-cache ownership shared by Settings and Alarm Settings.

`utility_window/text_input.rs` centralizes Bevy editable-text ownership/style so caret navigation, selection, Home/End, Backspace/Delete, and keyboard-shortcut blocking do not get reimplemented per search box/editor.

---

## 17. Character import/replacement

`character.rs` owns safe live replacement.

```text
Import/restore request
→ async copy / optional VRM0 migration
→ AssetServer load
→ validate/stage VRM
→ clear old rig-specific caches
→ commit ActiveCharacter + settings
→ reveal when initialized/animation-ready
```

A failed import leaves the previous character intact.

Imported user models are copied into Nera-owned application data. Original source files are not modified/deleted by normal library operations.

`vrm0_migration.rs` performs in-memory VRM 0.x -> 1.0 migration and routes the result through the ordinary compatible import path.

The migrator preserves mesh/bind-pose binary data and child-local frames, rotating scene roots instead of rewriting geometry. Head/collider offsets therefore use the preserved local Z-reflected frame; spring gravity uses the VRM1 world X-reflected frame. MToon migration converts shading intervals, color spaces, outline units, and UV transforms rather than copying legacy shader parameters. An explicit migration marker owns legacy gaze compatibility; the unmarked-file fallback is restricted to the original pre-MToon output. Material/texture expression bindings are retained in the imported JSON, but bevy_vrm1 0.9.2 currently evaluates only morph bindings. Previously converted private copies whose source parameters were discarded require reimporting the original VRM0 to receive corrected material conversion.

### Character-swap invariant

Any state storing old entity/bone/clip references must be reset or rebound: animation registry/playback, touch rig state, expression bindings, feedback anchors, Chibi rig cache, etc. Semantic preferences may persist; stale entity references may not.

---

## 18. Multi-character architecture

`character/library.rs` owns both the persistent catalog and companion process roster.

### Catalog

Entries store model identity/name plus metadata/fingerprints. Streamed content fingerprints prevent renamed identical VRMs from becoming duplicate imports.

Removal rules:

- built-in Nera is not removable;
- only Nera-owned private copies are deleted;
- original user source files remain untouched;
- live uses are released/switched before deletion;
- companion processes participate in a release handshake.

### Companion roster

The main instance allocates slots and launches independent companions. Companion instances cannot recursively create their own companion groups.

Each companion uses an instance-specific settings ID. The model catalog is a shared persistent library; active companion lineup is session/runtime ownership.

---

## 19. Animation Manager

The Animation Manager is a content/preview application isolated from the production desktop rig.

### Host/worker

`animation_manager/process.rs` main-side host starts/stops the worker, forwards roster/character state, receives validated events, and owns the worker lifecycle. The worker owns gallery renderer/UI assets.

### Registry

`registry.rs` owns the filesystem-backed catalog below `assets/animations` and bookmark/origin metadata. Manager operations use registry rules for import/bookmark/move/rename/delete.

### Protected runtime set

`base.rs` defines the production contract for `assets/animations/animasi_basic/`.

Protected runtime clips may be previewed but must not be renamed, deleted, or moved by ordinary manager actions. Any new production behavior depending on a VRMA should promote it into this stable set rather than depend on a mutable bookmark path.

### Preview isolation

Preview rigs never carry `NeraAvatar` and do not enter production physics/animation/gaze/body-mode systems. Independent render layers and animation-target namespaces prevent cross-rig retarget collisions.

### Character cards/cache

Browsing does not retain one full live VRM per card. Embedded/static previews and compressed idle caches are preferred; expensive full-idle generation is done serially in a disposable worker process and exits when finished.

### Combine/export

`combine.rs` trims/combines compatible VRMA sources without changing source files. Loader tests validate exported results using Nera's real VRMA asset path rather than only checking JSON structure.

---

## 20. Radial menu and tray

`action_menu.rs` implements a native transparent Win32 radial popup serviced through the tray/native message-loop infrastructure.

Current root surface:

```text
Cloud
Settings
Animation
Alarm
Games      [disabled placeholder]
Chibi
Screensaver
```

Cloud has Rest/Ride/Dismiss/Treat/Back. Treat emits an interaction request, not a new `CloudMode`: `TreatRuntime` owns only its loading/cooldown/temporary prop lifecycle, while existing animation/expression/PetFx/PetSfx own the reaction. Props attach to a live humanoid hand with rest-derived grip geometry and inherit the evaluated rig (including Chibi). Cloud and seated support retain placement ownership. Alarm opens Alarm Settings directly. Chibi toggles `AvatarBodyMode`; the native menu stores only presentation text, not its own mode state.

The menu emits typed `NeraActionMenuCommand` messages. Domain behavior belongs to the receiving subsystem.

The native wheel draws vector icons and highlights the selected sector briefly before dispatch. Its pending selection is presentation-only: a nonblocking Win32 timer completes it once; page navigation, focus loss, or dismissal cancels it. Disabled entries do not accept selection.

`tray/native.rs` owns Win32 tray icon/menu/callback processing; `tray.rs` bridges those commands into Bevy. Domain behavior should not migrate onto the tray thread.

---

## 21. Memory boundaries

Nera explicitly separates memory lifetimes where renderer/asset high-water behavior matters.

- `memory.rs`: release avoidable CPU-side VRM geometry copies after GPU upload where compatible.
- Settings/Alarm Settings: remain in-process; scoped utility font-cache cleanup releases utility-owned glyph state on close.
- Animation Manager: process-isolated so closing the worker releases its renderer/driver allocations.
- Character gallery: cached previews instead of many full live VRMs.
- Full source VRMs are loaded for actual applied desktop characters, not merely because a gallery card is visible/selected.

Measured results and exact audit procedures belong in the dedicated memory documents.

---

## 22. Persistence/storage ownership

- `NeraSettings`: Bevy SettingsPlugin, instance-aware settings ID.
- Character imports/catalog: Nera-owned application-data character directory.
- Alarm sound imports/catalog: Nera-owned application-data `alarm-sounds` directory.
- Runtime protected animations: `assets/animations/animasi_basic/`.
- General animation library/bookmarks: Animation Manager registry/bookmark ownership.
- Character thumbnail/idle caches: local cache data; safe to regenerate and never equivalent to the user's source VRM.

Persistent configuration must not become a second live state owner. For example, `body_mode = "chibi"` encodes preference, while `AvatarBodyMode` owns runtime selection.

---

## 23. Important end-to-end flows

### Body click -> reaction

```text
cursor/input
→ animated body hit test
→ NeraClicked(BodyPart)
→ animation admission
→ Head/Torso authored reaction OR
   Hand/Arm TouchReactionRuntime OR
   Leg/Foot procedural reaction
→ NeraAnimationState resolves priority
→ VRMA/playback or procedural overlay
→ expression derives from resolved state/clip
→ accepted semantic feedback emits PetFx/PetSfx
→ seated/cloud/grounding layers preserve placement
```

### Drag -> support/throw

```text
press + captured body part
→ movement threshold
→ NeraDragStarted
→ native window follows cursor
→ Rest may detach / Ride may retain cloud ownership
→ momentum sampling
→ release
→ optional NeraThrow
→ DesktopPhysicsState gravity/support
→ Landing/Idle/Sitting animation follows placement result
```

### Sitting

```text
deliberate drag near support
→ sitting candidate
→ SittingEnter
→ evaluated Seat anchor becomes available
→ Last-stage commit
→ environment::sitting follows support
→ seated loop/reaction pose
```

### Chibi toggle

```text
radial command
→ ToggleBodyMode
→ AvatarBodyMode changes/persists
→ previous overlay removed next frame
→ normal animation/cloud/sitting pose evaluates
→ BodyProportions applies Chibi geometry
→ constraints/gaze/expressions/springs consume final rig
→ anchors derive final support contact
```

### Cloud

```text
radial Cloud command
→ CloudRuntime owns placement
→ visual lazily spawns
→ selected seated/sleep animation
→ pose/contact calibration
→ logical seat stable; visual may bob
→ Rest/Ride-specific interaction rules
→ dismiss/interruption releases placement ownership
```

### Alarm

```text
deadline due
→ AlarmState::Ringing
→ enter Attention
→ snapshot desktop presentation
→ canonical portrait framing
→ selected sound + alarm animation sequence
→ dismiss/snooze
→ stop runtime alarm assets
→ restore desktop presentation/support/input
```

### Character replacement

```text
import/select
→ private copy / optional VRM0 migration
→ load + validate
→ clear old rig-specific references
→ commit new ActiveCharacter
→ systems bind to new rig
→ reveal when ready
```

### Animation Manager

```text
open request
→ main host launches worker
→ worker scans registry/caches + renders gallery
→ user action sent over IPC
→ main production owner executes requested action
→ worker closes
→ OS reclaims gallery renderer lifetime
```

---

## 24. State interruption rules

### Audio Reactive Dance

The opt-in `NeraSettings.audio_reactive_dance` preference enables local observation.
`music` runs Media Session and output-loopback/DSP observers off the Bevy thread,
publishing bounded latest evidence; `MusicDetectionState` owns fused hysteresis.
No audio is stored or transmitted. The detector never requests an animation.

Browser Media Session Music type and title/channel metadata are evidence rather
than proof: ordinary videos can expose the same fields. The detector confirms
music from independent metadata/audio evidence and keeps the lower hold policy
with the confirmed media identity. A different item must qualify again; scrolling
videos cannot inherit music ownership indefinitely. Detection tiers, timings and
validation details are documented in `AUDIO_REACTIVE_DANCE.md`.

`PetBehavior` selects explicitly curated `DancePool` registry references and asks
`LibraryPlayback` to load them. `NeraAnimationState::AutoDance` is a low-priority
one-shot intent mapped to existing library playback and its real clip duration.
The same controller, pose layers, grounding and Chibi overlay remain authoritative.
Empty/invalid pools reserve no animation state. Manual actions, loading, Cloud,
dragging and Attention release only autonomous requests/actions. V1 waits while
seated; it never detaches support to dance. The Animation Manager Dance tab edits
reference metadata through its existing worker/host IPC; no VRMA copies are made.

Exact animation priority lives in code, but new features should preserve these conceptual ownership rules:

1. Character loading/replacement may block interactions requiring a stable rig.
2. Attention/Alarm owns presentation while active.
3. Dragging owns pointer/native movement.
4. Cloud owns placement while attached; Rest additionally owns sleep input/gaze suppression.
5. Sitting attachment owns native support following.
6. Desktop physics owns unsupported free placement.
7. User reactions/library animations outrank autonomy.
8. Pet behavior/walking/idle fidgets are low priority and must yield.

Before adding a feature, answer:

- What state does it own?
- Which existing owner must it request/suspend?
- Which state may interrupt it?
- How does it release ownership on **every** exit path?

---

## 25. Validation architecture

### Automated

Use ordinary tests for state machines, geometry, persistence transformations, touch streaks, import logic, and any behavior that does not require a real compositor.

### Real asset/loader

Where file format correctness matters, validate through the same VRM/VRMA loader used by Nera when practical.

### Smoke/native harnesses

Debug smoke modules and `examples/*_validation.rs` run real app/Windows placement/rendering with isolated settings/output. Current dedicated examples include Chibi and touch validation plus a bounded external support-window fixture.

### Memory audits

Settings, Alarm Settings, Animation Manager, and multi-character memory audits are opt-in validation paths, not production behavior.

### Still manual unless specifically automated

- physical pointer targeting/feel;
- Windows focus/z-order quirks;
- auto-hide taskbar / Explorer restart;
- mixed-DPI multi-monitor edge cases;
- unusual third-party HWND/DWM behavior;
- long soak/resource stability;
- arbitrary custom-VRM visual quality.

Never claim these passed solely because `cargo test` passed.

---

## 26. Where to change things

| Goal | Start here | Also inspect |
|---|---|---|
| Add/change body animation | `animation/state.rs`, `playback.rs` | `expression/basic.rs`, `base.rs` |
| Hand/Arm touch behavior | `animation/touch.rs` | `touch/seated.rs`, feedback |
| Leg/Foot touch behavior | `animation/limbs.rs` | animation state/feedback |
| Hit regions | `interaction/hit_testing.rs` | body-mode effective scale |
| Headpat | `interaction/headpat.rs` | animation/expression/feedback |
| Drag/throw | `interaction.rs`, `momentum.rs` | desktop physics |
| Gravity/support | `desktop_physics.rs` | environment geometry/settings |
| Sitting/window follow | `environment/sitting.rs` | anchors/animation |
| Roaming | `pet_behavior.rs` | `walking.rs`, settings |
| Gaze | `avatar.rs` | animation suppression/Cloud Rest |
| Chibi | `avatar/body_mode.rs` | springs/anchors/cloud + Chibi docs |
| Cloud | `cloud.rs` | pose/bounds/animation/physics |
| SFX/particles | `pet_feedback/*` | semantic caller only |
| Alarm scheduler | `alarm.rs` | presentation/dismiss |
| Alarm editor | `alarm_settings.rs` | sounds/utility window |
| Alarm sound import | `alarm/sounds/*` | settings fallback cleanup |
| Attention framing | `presentation.rs` | alarm/body-mode geometry |
| Settings | `settings/model.rs` + relevant UI/runtime | utility window |
| Live character import | `character.rs` | migration/library/reset boundaries |
| Multi-character | `character/library.rs`, `roster.rs` | manager process bridge |
| Gallery content | `animation_manager/registry.rs`, `ui.rs` | `base.rs` |
| Gallery renderer/memory | `process.rs`, `preview.rs`, caches | memory audit |
| Radial menu/tray | `action_menu.rs`, `tray/*` | typed domain commands |
| Native Windows capability | `platform/windows.rs` | facade in `platform/mod.rs` |

---

## 27. Architectural invariants

### Native geometry

- Native support/placement uses physical desktop coordinates where required.
- Multi-monitor coordinates may be negative/non-zero-origin.
- Do not solve taskbar/support issues with arbitrary pixel offsets when geometry already exists.

### Animation

- `NeraAnimationState` owns body state.
- Features request animation; they do not directly establish a second playback owner.
- One-shot duration comes from real loaded clip duration where available.
- Fixed runtime reactions use protected base clips. User-curated AutoDance references
  follow registry rename/bookmark moves and safely skip missing/deleted files.

### Pose overlays

- Restore only your own previous-frame delta.
- Apply after the pose owners you depend on.
- Never accumulate transforms frame-over-frame.
- Never restore transforms captured from a replaced character.

### Placement

- Sitting, Cloud, physics, and Attention have explicit ownership boundaries.
- Lower-priority placement must yield instead of continuously fighting the current owner.

### Interaction

- `DesktopPetInteraction` classifies pointer gestures once.
- Feature logic consumes semantic messages.
- Click, drag, headpat, and menu must not each invent separate capture rules.

### Facial state

- Automatic face derives from resolved body animation/clip intent.
- Manual preview is an explicit temporary owner.
- Clearing preview releases ownership instead of pinning neutral morphs.

### Utility UI

- Utility surfaces may temporarily own application input.
- Closing an editor releases editor-only resources/previews, not unrelated runtime behavior.
- Editable text uses the shared Bevy text-input path.

### Persistence

- Persistent settings encode durable preference; runtime resources own transient state.
- User source files are never deleted as if they were Nera-owned imports.
- Missing/corrupt optional content falls back safely instead of panicking.

---

## 28. Structural hygiene notes

These are observations, not an instruction to refactor immediately.

1. `animation/limbs.rs` is historically named; it now owns only procedural Leg/Foot reactions. A rename/move may improve clarity later, but behavior should not change just for naming.
2. Large cohesive modules such as `settings/ui.rs`, `alarm_settings.rs`, and `animation_manager/preview.rs` should not be split solely because of LOC.
3. Small focused files such as `interaction/momentum.rs`, `utility_window/text_input.rs`, and `font_cache.rs` should not be merged solely to reduce file count.
4. Debug/smoke/audit files are intentionally separated from runtime behavior. They may eventually be grouped under a clearer validation/dev namespace, but should not be folded into production systems.
5. Cross-plugin ordering should remain explicit only where a real producer/consumer dependency exists. Do not globally serialize unrelated Bevy systems.
6. Native Windows QA remains necessary for compositor/focus/taskbar/mixed-DPI cases not covered by a dedicated harness.

---

## 29. Documentation boundaries

Keep stable architecture here:

- ownership;
- module responsibilities;
- process boundaries;
- state/data flow;
- scheduling constraints;
- invariants;
- where-to-change guidance.

Keep volatile detail elsewhere:

- Chibi proportions/screenshots -> Chibi docs;
- touch sequence validation -> `TOUCH_REACTION_POLISH.md`;
- memory measurements -> memory audit docs;
- milestone QA/history -> phase/audit docs.

Keep coding-agent workflow rules in `AGENTS.md` rather than duplicating them here. `AGENTS.md` should point to this document for architecture.

---

## 30. Recommended change workflow

```text
1. Identify the authoritative state owner.
2. Inspect that subsystem and its immediate dependencies.
3. Define acquisition/interruption/release behavior.
4. Reuse existing messages/resources.
5. Make the smallest coherent patch.
6. Add targeted regression tests.
7. Run targeted tests while iterating.
8. Run cargo fmt / cargo check / cargo test near completion.
9. Perform native/manual validation only where required.
10. Stop when acceptance criteria pass.
```

Do not introduce architecture merely because a feature feels large. First explain why the existing owner cannot represent the new behavior.

---

## 31. Architectural north star

```text
Windows / desktop facts
        ↓
semantic environment + interaction
        ↓
explicit state owners
        ↓
behavior requests
        ↓
authoritative animation / placement / presentation
        ↓
reversible pose + face + feedback layers
        ↓
VRM constraints / gaze / expressions / springs
        ↓
transparent desktop presentation
```

The goal is not the smallest possible number of files. The goal is that every feature has a clear answer to:

> **Who owns this state, who may request it, what can interrupt it, and how is ownership released without stale state?**

If those answers stay clear, Nera can keep adding features without turning Windows placement, VRM animation, interaction, and utility UI into competing patches.
