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

Verified on 2026-10-02:

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
