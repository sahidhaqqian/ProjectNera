# Phase 7 Cloud / SFX / FX Release Validation

## Optional Pet Audio

No new sounds are bundled or downloaded in this milestone. Existing alarm sounds
and their licenses are unchanged. Supply original/licensed short Ogg cues at:

| Semantic event | Asset path under `assets/` | Debounce |
|---|---|---|
| HeadPat | `sound/pet/headpat.ogg` | 1000 ms |
| Happy | `sound/pet/happy.ogg` | 1500 ms |
| Embarrassed | `sound/pet/embarrassed.ogg` | 1500 ms |
| Angry | `sound/pet/angry.ogg` | 1500 ms |
| Landing | `sound/pet/landing.ogg` | 350 ms |
| CloudAppear | `sound/pet/cloud_appear.ogg` | 300 ms |
| CloudDisappear | `sound/pet/cloud_disappear.ogg` | 300 ms |
| Sleep | `sound/pet/sleep.ogg` | 1500 ms |
| Wake | `sound/pet/wake.ogg` | 1500 ms |

Restart after adding files: discovery occurs once at startup. Missing/not-yet-loaded
cues are silently dropped; gameplay never waits for them. Playback shares master
volume/mute with alarms. At most four cues overlap and every player has a five-second
safety expiry, so use short cues rather than music/long loops.

## Automated / Bounded Native Checks

```powershell
cargo fmt --all -- --check
cargo check --all-targets --offline
cargo test --offline
cargo build --bin nera --offline
.\target\debug\nera.exe --feature-smoke --cloud-smoke
```

The Cloud smoke uses an isolated PID-specific settings profile and synthetic
interaction state through production helpers. It captures lying/seated Rest,
Rest after all-body touches, Ride, moved Ride, and a replaced VRM 0.x character.
It checks actual HWND
freefall after dismissal and support after touchdown; attachment cleanup during
replacement and Attention; and bounded/expired particles. Its lifetime is bounded.
Screenshots are in `%TEMP%\nera-feature-smoke`.

Earlier Cloud/Sleep verification on 2026-10-02:

- `cargo fmt --all -- --check`: PASS.
- `cargo check --all-targets --offline`: PASS (existing dead-code warnings).
- `cargo test --offline`: 320 passed, 0 failed, 6 ignored; 19 milestone tests
  plus eleven sleeping-loop/pose regression tests.
- `cargo build --bin nera --offline`: PASS.
- Bounded Cloud Windows smoke: PASS, exit 0. Real HWND Rest/Ride placement,
  synthetic drag/release, no-support floating, ignored ride throw, dismissal
  freefall/landing, legacy character replacement, Attention cleanup, and
  particle expiry passed. It checks real Sleeping Idle
  playback after a complete loop, head-to-right geometry and both hands away
  from the face on VRM1 and migrated VRM0. All-body click/headpat bursts must
  leave both Rest variants in the base loop; seated sleep verifies real closed
  eyelids and no lying arm layer. Ride uses the front rim with legs in front;
  lying Rest uses the inner cushion with the face unobscured and hands lowered
  onto/alongside the torso. The previously inspected bundled legacy model has
  no facial morphs and its eyes remain open.
  The updated native run visually verified open-eyed ordinary Ride sitting,
  closed-eyed seated Rest (`cloud-rest-seated.png`), and unchanged right-facing
  lying sleep after all-body touches (`cloud-rest-touch.png`).
- Existing bounded Motion Windows smoke: PASS, exit 0. Ordinary throw/landing,
  Gravity-OFF motion, and the migrated VRM0 Death01 shadow remain functional.
- Bounded Cloud smoke with the separate Alicia Solid VRM 0.x reference: PASS,
  exit 0. Real Blink intent reached rendered morph weights; both closed eyelids
  were visually verified in `cloud-rest-vrm0-blink.png`. Sleeping arms and
  head-to-right placement also passed on that replacement rig.

Implementation adds `src/cloud.rs` plus `cloud/{visual,pose,sleep_arms,tests,smoke}.rs`, and
`src/pet_feedback.rs` plus `pet_feedback/{sfx,particles}.rs`. Integration is in
main/action-menu/interaction, animation/anchors, physics/behavior, expression,
character/presentation, shadow/memory, tray recovery, and debug smoke dispatch.
The user-authored Cloud GLB is reused without modification. No Cargo features or
dependencies changed. Workspace `cargo fmt --all` also mechanically formatted
pre-existing files; it did not refactor those unrelated systems.

Smoke inputs are synthetic, not a completed manual pointer/DPI/soak run. Initial
rig loading/replacement emitted existing unavailable-Idle fallback warnings;
Winit emitted its existing unknown-Destroyed-window warning during shutdown.

## Multi Character / Headpat / Roaming Polish

The measurements below record the earlier two-rig implementation. The subsequent
[Multi Character memory audit](MULTI_CHARACTER_MEMORY_AUDIT.md) supersedes its
preview architecture and memory figures with one render slot and cached thumbnails.

Verified on 2026-10-02 after the focused follow-up:

- `cargo fmt --all` and `cargo fmt --all -- --check`: PASS.
- `cargo check`: PASS, with existing dead-code warnings.
- `cargo test`: **337 passed, 0 failed, 6 ignored**. Seventeen new regressions
  cover delayed title press/one-shot drag requests, import-card removal isolation
  and default protection, confirmation/IPC validation, two-rig preview budgeting,
  thumbnail freezing/thawing, hidden/remove/close cleanup, layout-gap cache reuse,
  worker-only 15 FPS and unchanged main 24/30 FPS settings, source-preserving
  remove/reimport, atomic-save rollback, active replacement safety, companion
  release/load/generation checks, and immediate use of the existing roaming option.
  The headpat feedback regression additionally verifies the retained semantic SFX.
- `cargo build --bin nera`: PASS.
- Native `--animation-manager-worker --roster-smoke`: PASS, exit 0. Desktop
  and 720x640 screenshots have nonblank model thumbnails and import-only X buttons.
  Seven cached cards settle to one live selected rig; update rate measured **15.0 FPS**.
  Real minimize clears previews, restore recreates bounded state, and closing
  leaves zero preview roots/window/preview state before worker exit.
- Native `--feature-smoke --cloud-smoke`: PASS, exit 0, with isolated AppData.
  Actual HeadStroke playback uses ShakeHeadNo1, the independent automatic face
  selects Joy 100%, and all three sparkles render while Ride remains attached.
  Rest touch suppression, dragging, freefall/landing, model replacement, Attention,
  and particle expiry remain functional. Optional pet SFX files are absent, so
  audible playback was not manually verified; semantic dispatch and missing-file
  safety are covered by tests. `headpat-ride.png` was visually inspected.
- Native companion stdin/stdout release checks: PASS for used and unrelated
  models, each exit 0. The used copy exits safely; the unrelated default copy
  stays alive and acknowledges its real model, rejects selection during the
  release guard, resumes after RELEASE_DONE, and can subsequently switch models.
  These tests use the private temporary import, not the user's character catalog.

### Drag and Memory Findings

Title dragging formerly required `MouseButton::Left.just_pressed` and a changed
UI `Interaction::Pressed` on the same update. Delayed UI input could miss that
coincidence. The new held-button check still requires a changed UI press, avoiding
repeated drag requests. Title labels pass focus and the drag region fills the header.
Physical-pointer title dragging remains a manual check below.

Previously up to twelve model cards each retained an animated VRM hierarchy,
retarget data, camera and 384x512 target. The follow-up admits at most two live
model rigs, retains only visible-card thumbnails after warmup, and uses 256x320
targets (58% fewer target pixels). Stable path keys and the UI-layout-gap guard
avoid accidental re-creation or misbinding when the registry/selection changes.

Same catalog, debug renderer, process samples at 12 seconds after launch:

| Worker memory | Before | Final |
|---|---:|---:|
| Private bytes | 2052.7 MiB | 1166.5 MiB |
| Working set | 1466.2 MiB | 1275.5 MiB |

Private bytes declined about **43%**; working set declined about **13%**.
After warmup the native worker reported 918.1 MiB private bytes. These samples
include loading/renderer allocations, are machine/model dependent, and are not
GPU-memory measurements. The 15 FPS cap reduces update/render frequency (half as
many opportunities as 30 FPS); CPU percentages were not profiled. Most memory
improvement comes from fewer live scenes/cameras and smaller targets, not FPS alone.
Main pet/companion FPS application is unchanged, and no dependencies/features changed.

Remaining limitation: large custom VRMs, shared source data, morph/retarget caches,
and renderer/driver high-water allocations can still dominate RAM until worker
exit. No aggressive asset purge was added. Each live desktop companion still has
its own renderer. The removal handshake adds one blocking pipe reader per companion
and a bounded 16-reply channel; it does not poll the filesystem each frame.

### Changed Files

Modified:

- `src/action_menu.rs`
- `src/animation/playback.rs`
- `src/animation_manager.rs`
- `src/animation_manager/characters.rs`
- `src/animation_manager/preview.rs`
- `src/animation_manager/process.rs`
- `src/animation_manager/ui.rs`
- `src/character.rs`
- `src/character/library.rs`
- `src/character/roster.rs`
- `src/cloud/smoke.rs` (debug native headpat regression only)
- `src/expression/basic.rs`
- `src/pet_feedback.rs` (retained SFX assertion only)
- `src/settings/ui.rs` (existing roaming-control regression only)
- `README.md`
- `ARCHITECTURE.md`
- `PHASE_7_POLISH_VALIDATION.md`

Added:

- `src/character/library/removal.rs`
- `src/character/library/release.rs`

### Remaining Manual Checks

1. Drag the Multi Character title using a real pointer, including first click on
   an unfocused window, and resize/move across monitors at 125%/150% DPI.
2. Use card X/Cancel/Remove on inactive, primary-active and companion-active
   imports; default Nera must have no card X. Verify the original source remains.
3. Reimport through the real file picker, restart, and verify library/selected
   configuration; also test failure/cancellation during a slow model load.
4. Toggle Allow Roaming on/off and restart. Off must retain idle/headpat behavior
   and cancel locomotion, matching the former Stay Here action.
5. Compare main Nera at 24/30 FPS with the 15 FPS manager open; test normal headpat
   with no manual face override, and audible HeadPat only when an optional cue exists.
6. Repeatedly open/scroll/minimize/restore/remove/close and perform a longer soak
   with custom large models. Live rigs must remain bounded and disappear on close.

## Private Eyelid Reference Follow-up

At the user's subsequent request, an official UniVRM reference model was
downloaded outside the repository as a private backup. No new third-party
model or sound is bundled in the release.

- File: `C:\Users\sahid\Downloads\Nera-model-backup\AliciaSolid_vrm-0.51.vrm`.
- Source: [UniVRM Alicia Solid fixture](https://github.com/vrm-c/UniVRM/blob/master/Tests/Models/Alicia_vrm-0.51/AliciaSolid_vrm-0.51.vrm).
- Size: 7,878,712 bytes; SHA-256:
  `237BB02EFADF8C13A114AF91DD8E860173081457DEE87017E51011C448D05DC2`.
- Embedded author: copyright DWANGO Co., Ltd.; avatar permission: Everyone.
  The model carries its own [Alicia usage terms](https://3d.nicovideo.jp/alicia/rule.html),
  not the repository's source-code MIT license. Do not bundle/relicense it.
- Legacy facial groups include Blink, Blink_L and Blink_R with real mesh binds.
  The smoke imports a library entry but uses a PID-specific active-character
  profile; it does not replace the user's main active avatar.

```powershell
.\target\debug\nera.exe --feature-smoke --cloud-smoke --cloud-smoke-blink-model=C:\Users\sahid\Downloads\Nera-model-backup\AliciaSolid_vrm-0.51.vrm
```

The requested Charlotte download remains blocked by the in-app browser's
site/download permission. No alternate download route was used to bypass that
block. The bundled Charlotte has no morph targets, expression binds or eyelid
bones; a different model's Blink cannot simply be copied onto its vertex layout.
Closing that specific avatar's eyelids still requires authoring compatible
eyelid geometry/morphs or obtaining a version that includes them.

## Manual Windows Checklist

These checks require real pointer/native desktop interaction. An automated smoke
pass does not establish them. Record OS/GPU, monitor layout, DPI, VRM, FPS cap,
Gravity and Throw settings for each run. Repeat with at least one companion.

1. **Rest on empty desktop:** summon/dismiss Rest repeatedly and verify alternating
   lying and seated sleep, without switching during the attachment or repeated
   Rest commands. Both use Exhausted 100% (requires authored eyelid morphs).
   Check at least two full loops: lying head-to-right, a face unobscured by cloud
   lobes, inner-cushion contact and relaxed hands; seated front-rim contact and
   closed eyes. Click head, torso, both arms/hands/legs/feet and headpat repeatedly:
   neither pose should react or play headpat feedback. Logical position stays
   fixed, bobbing is subtle, and ordinary roaming/falling do not resume.
2. **Rest above a window:** move/resize/minimize/close the underlying window.
   Rest must not attach to it, disappear with it, or chase its top edge.
3. **Drag out of Rest:** drag quickly and slowly. Attachment releases at drag
   start, cloud shrinks away, cursor movement stays continuous, and release
   honors ordinary Gravity/Throw settings. Re-summon/dismiss repeatedly.
4. **Ride in mid-monitor:** Cloud > Ride uses an ordinary seated loop and stays
   suspended, with no native support and no permanent change to Gravity.
5. **Drag Ride:** drag head/torso and visible cloud. The same HWND/mount moves
   as a unit without detaching, seat recalibration jumps, or click-through traps.
6. **Release with no support:** release both fast and slow above empty desktop.
   Ride stays there; no throw, fall, taskbar/window snap, or autonomous walk.
7. **Dismiss Ride:** with Gravity ON, Nera falls/lands normally; with Gravity OFF,
   she stays unsupported. Re-enable gravity and verify ordinary support resumes.
8. **Monitor edges:** move across left/right/top/bottom edges, negative-coordinate
   monitors, and unplug/reconfigure a monitor. Ensure no unexpected support snap.
   Tray Reset Position must dismiss the ride and recover the pet visibly.
9. **125% / 150% DPI:** repeat dragging, release, cloud hit testing, and Settings
   Character Size/Ground Offset changes on same-DPI and mixed-DPI monitors.
10. **Custom VRM:** test VRM 1.0 and migrated VRM 0.x with different body proportions.
    Verify seat height, cloud framing, headpat, dodge, limbs, facial fallback,
    and no floating floor shadow. Test sleep arm placement on long/short arms
    and large sleeves. Models without morphs cannot show new faces or close eyes;
    the bundled `nera0.0.vrm` has zero morph targets/empty blendShapeGroups.
11. **Replacement with cloud:** replace the active character during Rest, Ride,
    appearance/disappearance, or drag. Old cloud/particles must be destroyed and
    the new character must accept normal dragging/physics and a fresh summon.
12. **Fullscreen / alarms / utility lifecycle:** enter/exit fullscreen and Alt-Tab;
    cloud remains usable and no roam starts. Trigger/dismiss/snooze alarms, hide/show
    from tray, and open/close Settings/Animation Manager. Attention/hide intentionally
    dismiss cloud; utility windows alone do not. Play a VRMA: cloud exits but the
    selected manual face remains. Default/0% return to automatic cloud expression.
13. **Headpat SFX/particles:** pet repeatedly without buttons, including on Ride.
    Verify small sparkles, matching face/animation, debounce, volume/mute, and
    unaffected existing alarm audio. Pelvis/legs stay seated throughout the
    reaction and return blend; initial calibration cannot be interrupted by
    immediate clicks. Rest must ignore these reactions/feedback; drag out of Rest
    and verify normal reactions resume. Repeat with all optional files absent.
14. **Landing feedback:** repeat falls/throws/window support loss. Each real
    touchdown has one small puff/cue; drag cancellation and idle support do not.
15. **Soak:** run 60+ minutes, summoning/dismissing both modes and repeating headpat,
    lands, alarms, utility open/close, character replacement, and companion close.
    After bursts settle, FX/player entities return to zero, cloud roots <= 1,
    handles/entities do not grow per cycle, and CPU/RAM show no continuing trend.

## Intentional Limits

- Rest has exactly two stable variations: the promoted `Sleeping_Idle.vrma`
  loop and seated Sleeping using Sitting 4 with Exhausted 100%. Sitting 4 is
  also the lying clip's loading fallback. `Sleeping_Idle_2.vrma` remains in
  Bookmarks. Dedicated SleepEnter/Wake transitions are not added here.
- The reversible sleep arm layer is scoped to Rest Cloud, not a rewritten VRMA.
  It adjusts eight shoulder/arm/wrist rotations with rig rest-axis retargeting;
  fingers and breathing stay in the source clip. Unusual custom proportions
  can still need calibration. The bundled legacy model's eyes cannot close
  without an authored Blink morph/eyelid rig; changing formats alone cannot add it.
- Cloud seat height/scale is calibrated to the supplied model; unusual custom
  proportions still need visual validation. Ride's seated clip hangs its legs
  and uses the measured front rim; lying Rest uses the inner cushion and
  a scale-aware world-down pelvis contact. This is not general collision/IK.
  Cloud has no flying/roaming physics.
- Model data is cached once after first summon; effects use five tiny shared
  meshes/materials. No new audio feature, library, physics engine, UI framework,
  downloadable asset, music, mood, or modding system is introduced.
- Real focus/z-order, pointer/DPI/monitor topology, corrupt asset decoding, audio
  audition, and a long soak remain distinct from bounded automated checks.
