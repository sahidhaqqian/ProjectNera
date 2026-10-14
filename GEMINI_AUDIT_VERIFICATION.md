# Gemini audit verification — 14 October 2026

Reviewed the supplied 22 claims against `445f1c0` and the installed Bevy 0.19.1 / bevy_vrm1 0.9.2 source. This is a focused verification, not a new architecture audit. Severity in the supplied report is not automatically accepted: a missing guard or unnecessary update does not establish an observed crash or measured heavy stutter.

| # | Verdict | Evidence / action |
|---|---|---|
| 1 | Confirmed; fixed | Preview attached `VrmaHandle` before validation. Imports now preflight self-contained binary glTF, humanoid hips mapping and actual keyframe times; preview waits for asset/dependencies and shares runtime validation before attaching the handle. Invalid previews are skipped with one log. |
| 2 | Partly confirmed; fixed | HeadStroke was omitted from the desktop seat overlay. Added it and protected seat-anchor sampling for HeadStroke, Treat and FocusCelebrate. The proposed Study-start bug is already prevented by `FocusContext::seat_ready`; no standing reading pose is captured as a seat. |
| 3 | Confirmed; fixed | Cloud froze every non-seated visual state. It now preserves only upper-body reactions and the existing seated return blend, leaving Dodge/full-body owners intact. Its exact previous transforms are restored in Update **after** the Chibi PreUpdate restore and before animation, matching the desktop-seat layer's ordering. |
| 4 | Partly confirmed; tightened | Focus already waits for the lazy Typing slot before requesting Work; a multi-second lazy-load fallback is not its current flow. Props now additionally require Typing to be the actual current clip before starting their settling delay. |
| 5 | Confirmed; fixed | Hidden/unchanged cheeks mutated material assets each frame. Visibility uses `set_if_neq`; material writes occur only when visible alpha actually changes. No quantitative stutter improvement is claimed. |
| 6 | Confirmed; fixed | A cache miss fully decoded previously validated audio on the main thread. Retained content-hash and cheap container checks; removed repeated full decoding. File reading/hashing still occurs on a trigger, bounded to 8 MiB. |
| 7 | Confirmed; fixed | Checkpoint return assumed desktop rect and RestTransform still existed. Missing data now releases the session instead of unwrapping. |
| 8 | Confirmed; fixed narrowly | Higher-priority persistent/one-shot actions could leave a manual library reaction underneath. Both paths now also remove LibraryAnimation alongside their existing Touch/Treat/Focus cleanup. Head/chest/limb resume semantics are preserved: the existing `floating_overrides_reaction_then_reaction_returns` regression explicitly requires them. The suggestion to cancel all Reaction actions was too broad. |
| 9 | Partly fixed; one limitation retained | Missing cached entities now invalidate cache accounting and allow reloading. LRU protects all active library/AutoDance entities. In-place external replacement of a cached path is still not hot-reloaded; supported Manager imports use unique names. Automatic reload needs asset/graph coordination, so a speculative replacement cache was not added. Restart clears that cache. |
| 10 | Confirmed; fixed | Matching rejected requests now clear automatic ownership flags; accepted dispatch clears them too. Automatic failures log without issuing a manual playback UI error. |
| 11 | Confirmed; fixed | Walking coordinates were relative to the physics surface, while Edge expected-position checks added work-area origin. All three consumers now use the captured surface origin. Offset/negative monitor and left-taskbar geometry have regression coverage; physical multi-monitor QA is not claimed. |
| 12 | Confirmed; fixed | Empty WASAPI polls fed silence at 8 kHz into a 44.1/48 kHz stream. Silence now uses the capture rate, preserving the decimator/filter state. Real packet absence still contributes silence. |
| 13 | Confirmed; fixed | A late decoded JPEG could repeatedly miss the exact desired frame. A completed frame is displayed when advancing, then current timeline decoding resumes; one pending task remains the bound. |
| 14 | Rejected; unchanged | Upstream `init_spring_joint_states` computes `bone_length` from **local** `tail_tf.translation.length()`, not scaled GlobalTransform. Chibi removes its local overlay in PreUpdate before that Update initializer. Dividing this normal local baseline by 0.62 would corrupt Normal restoration. Existing exact spring-restoration coverage remains. |
| 15 | Confirmed; fixed | Moves now compensate file/metadata changes on save failure and always refresh if rollback fails. Dance remapping restores its in-memory snapshot on save error. Irreversible deletion updates both live metadata sets and refreshes even if persistence fails. Errors remain visible. |
| 16 | Partly confirmed; fixed selectively | Constant rest translation/scale tracks are omitted; genuinely animated unsupported tracks still fail safely. Animation-only output rebuilds its buffer/accessor tables, avoiding inherited source payload accumulation. Files with meshes, skins, images or foreign extensions preserve their referenced data. Hips conversion intentionally retains the pinned loader's normalized global-height convention, verified by existing retarget tests; changing only Combine to a different convention was not justified by a failing supported fixture. Unusual offset rigs remain a compatibility limitation. |
| 17 | Confirmed; fixed | Taskbar z-order refresh now checks the current primary window's AlwaysOnTop preference, matching ordinary-window support. No topmost assertion is issued for OFF. |
| 18 | Rejected for production flow; unchanged | `CloudCommand::Summon` destroys the old visual and spawns a new one when switching modes. Rest does not turn into Ride on the same visual; the disappearance fade deliberately retains the old contact. Resetting it on fade would move the cloud. |
| 19 | Confirmed; fixed | Bevy's iterator advances its cursor per consumed item; it has no consume-on-drop behavior. Drag/release and manager-open readers now consume the full batch rather than leaving old events for another frame. |
| 20 | Confirmed; fixed | WAV preflight now rejects zero/non-byte-aligned sample widths before the shared decoder. Existing positive channel/rate/block-alignment checks remain. |
| 21 | Confirmed; fixed | Unavailable Breathing no longer prevents available Screensaver variations from running above Idle after their existing random delay. |
| 22 | Already fixed; unchanged | `apply_light_settings` already queries `DirectionalLight` with `With<NeraMainLight>`. The supplied finding describes code that is no longer present. |

## Validation

A first cached test run was explicitly discarded: Windows clock/timestamps made Cargo consider an older binary fresh. Only Nera's dev build artifacts were cleaned; source, settings, assets and release artifacts were preserved. All results below use rebuilt changed sources.

| Validation | Actual result |
|---|---|
| `cargo fmt`, `cargo fmt --check`, `git diff --check` | PASSED |
| `cargo check --locked -j1` | PASSED; 31.34 s |
| `cargo test --locked --bin nera -j1` | PASSED; 646 passed, 0 failed, 11 ignored |
| `cargo test --locked -j1` | PASSED; 646 passed, 0 failed, 11 ignored; example validation binaries also compiled; 5m23s build, 7.54s main tests |
| `cargo build --locked --bin nera -j1` | PASSED; 2m31s; fresh debug executable |
| Native Chibi | PASSED; 73.12s; bundled Nera and imported Alicia Solid; reversible transforms, walking/headpat, library VRMA, taskbar, Cloud, drag, Alarm and spring checks |
| Native Cloud | PASSED; 43.20s; existing Rest/Ride/drag/support harness |
| Native Focus | PASSED; 69.49s; 16 checks, including seated Study, headpat/resume, actual Typing/laptop, Chibi recalibration, Alarm priority, completion and cleanup |
| Native Animation Manager host | PASSED; 98.71s; worker open/close cycles and process cleanup |

Twenty focused regression tests were added for VRMA import/preview rejection, missing hips and invalid times, original-byte preservation, stale clip cache recovery, automatic failure flags/UI isolation, higher-priority manual interruption, missing checkpoint data, seated headpat, reversible Cloud overlay/Dodge ownership, actual Typing readiness, unchanged blush material events, packet-gap filter continuity, late JPEG decoding, Edge coordinates, metadata failure rollback/delete refresh, repeated Combine buffer bounds, constant rest channels, WAV sample widths, Screensaver fallback and batched pointer messages. Existing assertions were retained; seat-anchor coverage was expanded. The initial overly broad Reaction cancellation was corrected when the existing Head/Floating regression demonstrated intentional resume behavior.

Native testing used semantic messages, real HWNDs, actual VRMA/skeletons and GPU rendering, without moving the OS pointer or sending keyboard input. Study-taskbar and Work-laptop captures were visually inspected. External-window sitting was SKIPPED because no visible support edge had enough headroom. Physical pointer/AlwaysOnTop-off targeting, multiple physical monitors/left taskbar, perceptual audio/real WASAPI packet-gap playback, 25/50 wall-clock-minute waits, Do Not Disturb notification visibility, release/abort-build execution and hosted CI were NOT PERFORMED in this patch. No measured performance improvement is claimed. Raw logs/results are under `target/gemini-verification/`; current native Focus screenshots/report are under `target/focus-v2-native/`.

## Changed files

- Animation: `src/animation.rs`, `src/animation/{library,library_motion,playback,screensaver,state,systems}.rs`, `src/animation/touch/seated.rs`.
- Manager: `src/animation_manager.rs`, `src/animation_manager/{combine,idle_cache,preview,process,registry}.rs`.
- Pose/support/activity: `src/avatar/anchors.rs`, `src/cloud.rs`, `src/cloud/pose.rs`, `src/environment/sitting.rs`, `src/focus/activity.rs`.
- Face/audio/autonomy: `src/expression/face.rs`, `src/alarm/sounds/library.rs`, `src/music/{dsp,native}.rs`, `src/pet_behavior/living.rs`, `src/pet_behavior/living/tests.rs`.

No Chibi proportion, spring solver, DancePool, asset, timer or animation-owner architecture was replaced. The report's three rejected claims (#14, #18, #22) received no speculative patch. Remaining compatibility boundaries are documented in #9 and #16.

The existing release ZIP is historical and does not yet contain this verification patch. No commit, push or new release publication is part of this request.
