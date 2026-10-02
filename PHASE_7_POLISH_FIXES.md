# Phase 7 polish fixes

Base: `060d059be5743b9ced6558e15e0ce33419c51210` (`phase 7: memory optimizer part 2`).
Work branch: `codex/phase7-polish`, in the Codex worktree. Integration target:
`codex/animation-manager-redesign` in the primary checkout, from the same base commit.

## Root causes and focused changes

| Request | Root cause | Change |
| --- | --- | --- |
| Alarm radial | Preset durations belonged to a second radial page instead of the existing alarm editor. | One Alarm entry opens Alarm Settings directly. All seven quick durations, Test Now and Cancel live in its editor. |
| Headpat | The reaction used Joy and the generic plus-shaped HeadPatSparkle particle. | Keep `ShakeHeadNo1.vrma` and the existing HeadPat sound event; use Timid and semantic Heart feedback. Three hearts, 750 ms lifetime, existing cooldown and global particle cap. |
| Floating feet | Retargeted hips translation changes visual foot contact independently of the physics support anchor; only Death previously reconciled that contact. | `AvatarVisualGrounding` restores its temporary root Y before evaluation and solves grounded foot contact at the actual animated depth after constraints. Standing support geometry is calibrated from settled Idle. No per-animation Y offsets. |
| Caret | Raw keyboard handling appended/popped the end of a String; the drawn caret had no cursor or selection state. | Reuse Bevy's native EditableText and InputFocus for Search, Rename and Combine Name. Preserve caret/selection across required gallery rebuilds and release text ownership on blur/close. |
| Ride Cloud/taskbar | Native drag updates bypassed cloud alignment, and bounds were an anchor rather than the whole rendered mount. | Clamp CloudRuntime's logical seat using the existing native monitor work area and projected avatar + full-sized cloud mesh bounds. Include bob extremes and refresh the monitor during dragging. |
| Custom alarm sounds | Alarm configuration supported only five fixed startup handles, with no imported catalog or Random selection. | Extend the existing alarm/audio stack with an AppData catalog, stable IDs, content hashing, decoder validation, bounded imported-handle cache, Random resolution once per ringing event and editor-only preview ownership. |
| Sleeping head follows mouse | LookAt and BodyTracking admission did not consider CloudRuntime's Rest owner. | Derive gaze eligibility from the active Rest attachment. Neutralize cursor target and remove procedural head/body tracking during Rest; normal settings resume when that owner releases. |
| Alarm A/B size | Live pose fitting continually increased camera distance, while authored depth movement changed perspective magnification. | Presentation owns one rest-rig-derived orthographic portrait for the whole alarm sequence. Snapshot/restore the original camera transform/projection; reframe once for an initialized replacement character. No continuous scale forcing. |
| VRM 0.x replacement hangs | With Bevy's `multi_threaded` feature absent, IoTaskPool eagerly runs the optional thumbnail job on the caller. Its native renderer-process wait blocked the UI for about 45 seconds. | Keep the existing serial PreviewJobs owner, but execute the blocking wait on one named OS thread. Poll completion before joining; report worker failure once. Restore the smoke test's original 65-second bound. |

## Ownership and transitions

- AlarmState still owns deadlines/ringing. AlarmSoundLibrary owns only catalog,
  content validation and the bounded two-handle imported cache. Existing Bevy
  AudioPlayer/AudioSink remain the sole playback stack. Random chooses a handle
  once; repeats clone that selected handle. Closing the editor stops only
  AlarmSoundPreview, including pending playback, and never the real ring.
- NeraAnimationState remains the animation owner. Grounding is a derived pose
  correction, with a single AvatarVisualGrounding replacing the former
  Death-only correction resource. Physics/environment retain support placement.
  Falling, jumping, dragging, seated/cloud placement, Attention and unknown
  imported clips keep authored motion. Future Dance/Music clips can opt into the
  shared GroundingPolicy.
- CloudRuntime remains the attachment/seat owner. Bounds are derived geometry;
  they neither persist position nor introduce another native-window mover.
  Rest gaze suppression stores no second preference or restoration flag.
- PresentationState remains the snapshot/window/camera owner. A/B transitions
  retain the same projection, placement and canonical root scale. Dismissal
  restores desktop state and releases ownership even with a missing window or
  camera. Character replacement uses the same snapshot.
  Native validation also exposed a dismissal ordering race: the alarm-start
  failsafe could read DesktopPet before the exit cleanup consumed its message.
  Exit cleanup now precedes that failsafe and the next deadline in PostUpdate.
- Bevy owns native editing state; ManagerState owns committed gallery strings.
  Raw presentation shortcuts respect editable focus. Gallery navigation does
  not consume Home/End while text is focused. Closing a worker releases focus.

## Sound-library behavior and limits

- MP3, OGG and WAV are supported through the decoder already used by Bevy audio.
- Imports use BLAKE3 byte fingerprints, so renamed copies deduplicate. Reimport
  repairs a missing/corrupted registered file.
- Imported files live under
  `%APPDATA%/io.github.sahidhaqqi.projectnera/alarm-sounds`, with a staged catalog
  write. IDs are not reused after deletion; prior five built-in IDs and alarm
  book configuration remain compatible.
- Built-ins cannot be removed. Deletion clears stored alarm references, quick
  selection and editor drafts to Classic. A currently ringing handle remains
  valid until its normal alarm lifecycle stops it.
- Missing or corrupted imports quietly resolve to Classic. Validation occurs on
  trigger/preview rather than every frame/repeat. Random filters invalid imports.
- Limits: 32 imported sounds, 8 MiB each, five minutes, mono/stereo, at most
  192 kHz. The imported decoded-source handle cache holds at most two entries.

## Files changed

- Alarm/menu/audio/config: `src/action_menu.rs`, `src/alarm.rs`,
  `src/alarm/sounds.rs`, new `src/alarm/sounds/library.rs`,
  `src/alarm_settings.rs`, `src/alarm_settings/memory_audit.rs`,
  `src/settings/model.rs`, `Cargo.toml`, `Cargo.lock`.
- Headpat: `src/expression/basic.rs`, `src/pet_feedback.rs`,
  `src/pet_feedback/particles.rs`.
- Cloud/gaze: `src/cloud.rs`, new `src/cloud/bounds.rs`,
  `src/cloud/visual.rs`, `src/cloud/tests.rs`, `src/cloud/smoke.rs`, `src/avatar.rs`.
- Grounding/presentation: `src/avatar/anchors.rs`, `src/animation/playback.rs`,
  `src/animation/state.rs` (test-only action fixture), `src/desktop_physics.rs`,
  `src/presentation.rs`, `src/main.rs`.
- Reusable editing: `src/utility_window.rs`, new
  `src/utility_window/text_input.rs`, `src/animation_manager.rs`,
  `src/animation_manager/ui.rs`, `src/animation_manager/preview.rs`,
  `src/animation_manager/process.rs`, new
  `src/animation_manager/text_input_tests.rs`.
- Compact dynamic skin-bounds regression: `src/memory.rs`.
- Character replacement responsiveness: `src/character/library/previews.rs`.
- Documentation: `ARCHITECTURE.md`, this report.

## Regression coverage

- Direct radial Alarm command, all quick presets, backward-compatible imported
  IDs and Random configuration.
- Sound-byte dedup/restart/delete/reimport; invalid extension/size/metadata,
  truncation; missing/corrupt/repair; reference cleanup and path safety; bounded
  handle cache; ring/preview marker separation and explicit stop/close behavior.
- Timid headpat and semantic Heart/SFX dispatch, existing debounce/entity caps.
- Contact at different foot depths over repeated restore/evaluate cycles;
  grounded-policy admission, airborne/drag exclusion and replacement geometry.
- Native Bevy editing on all three fields: Left/Right/Home/End/Backspace/Delete,
  accented Unicode characters, selection replacement, pointer caret/drag selection, caret
  transfer through rebuild, blur/Escape, gallery-scroll gate, busy combine lock,
  close cleanup.
- Whole mounted geometry at all four taskbar edges, negative monitor coordinates,
  DPI 1/1.25/1.5/2, invalid/oversized bounds; live skin metadata survives GPU-only
  vertex-buffer release.
- Rest gaze release on explicit dismissal, dragging, presentation/hide,
  replacement and mode switch, with both enabled and disabled user settings.
- A/B root/camera/projection invariance and exact normal-state restoration,
  degraded exits and one-time initialized character replacement reframe.
- Thumbnail generation begins on a different thread and can remain pending
  without blocking the caller, even with Bevy's single-threaded task pools.

## Validation results

`cargo fmt` passed. `cargo check` passed with existing/legacy dead-code warnings.
`cargo test` passed: **395 passed, 0 failed, 6 ignored**, plus three auxiliary
binaries with zero tests. Tests use isolated APPDATA/LOCALAPPDATA directories
under `target/` so filesystem/cache tests do not touch the user's catalogs.
The headless native-editor harness initializes the same clipboard buffer and
font/style pipeline as the native UI; `bevy_clipboard` is already transitive and
is exposed only as a dev dependency for this fixture.

Native Windows checks run outside the filesystem sandbox so HWNDs/GPU output
reach the interactive desktop, with separate AppData profiles. The observed
monitor is 1920 x 1080 with a 48-pixel bottom taskbar; other edges/DPI cases are
covered by regression tests, not by physical monitor reconfiguration.

- `--alarm-framing-audit`: PASS; captured real Jump at 3 seconds and Wave at
  9/29 seconds. Inspected rendered portraits; model scale is consistent, hands
  fit the canonical portrait, and dismissal returns to the 600 x 950 pet window.
  No false Attention-start failure remains after the lifecycle-order correction.
- `--feature-smoke --cloud-smoke`: PASS with a cold isolated character/cache
  profile, in about 38 seconds with the original 65-second deadline. Validated
  Rest tracking suppression, all detach paths exercised by the audit, dragging
  Ride below the bottom work area, release staying mounted, three Timid headpat
  hearts, evaluated grounded foot contact, VRM 0.x replacement, Rest on the new
  rig, Attention interruption and particle cleanup. The reported unresponsive
  replacement was reproduced before fixing the blocking thumbnail wait; it did
  not recur in this run.
- `--alarm-memory-audit --alarm-memory-screenshots`: PASS. Import, renamed-byte
  duplicate, selection/save, Random, deletion/reference cleanup, preview
  replacement/stop and editor-close/real-ring isolation all passed. Across three
  editor and three ring cycles, closed states return to zero owned UI entities,
  utility windows, preview/ring players and font CPU bytes; one main camera and
  64 images remain. Editor-close private memory was 521.7, 517.5 and 515.1 MiB.
  This establishes bounded ownership in the short audit, not a long-duration
  driver-memory guarantee.
- Inspected real Windows rendered captures of headpat/grounding, cloud poses
  and alarm A/B framing. A direct pointer run opened the native radial wheel
  and confirmed one Alarm entry. The subsequent Alarm click and live keyboard
  editing were not established: the user stopped Computer Use with Escape and
  explicitly asked to skip the remaining tests. Native editor regression tests
  and alarm audits above passed, but remaining manual checks are unperformed.

## Remaining boundaries

- Unknown custom VRMA imports retain authored motion until explicitly classified
  as Grounded. Automatically grounding an unclassified custom jump would erase
  intentional airborne behavior.
- A mount larger than the entire monitor work area cannot fit completely; the
  clamp centers its bounds to maximize visible area.
- Optional HeadPat SFX files are absent from this checkout. The existing semantic
  dispatch remains intact; audible validation requires installing that asset.
- The bundled Nera VRM has no bound eyelid morph, as documented in the existing
  architecture. Rest still suppresses procedural gaze completely; no substitute
  eyelid geometry is introduced.
- A short repeated-open audit detects ownership/entity/cache leaks but does not
  establish long-duration process-memory or GPU-driver stability.
- Backspace behavior for decomposed combining sequences and ZWJ emoji follows
  the existing Bevy/Parley editor and font shaping; it can remove one code point
  at a time. ASCII navigation/deletion, accented Unicode editing and native
  selection are covered without introducing a second text-editing implementation.
