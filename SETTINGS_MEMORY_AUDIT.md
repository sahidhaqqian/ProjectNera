# Settings Memory Audit

Measured on 2026-10-03, Windows, debug `nera.exe`, built-in Nera, the existing
24 FPS runtime setting. This concerns the Settings window in the **pet process**,
not the separate Multi Character Manager. All samples use MiB (1024 * 1024 bytes).

## Findings

The existing close handlers already destroyed the Settings window, its camera,
and its recursively owned UI tree. No growing Settings entity tree was found.
However, Bevy's shared `FontAtlasSet` retained eight glyph atlases after closure:
eight 512x512 RGBA images, 8 MiB of CPU pixels plus GPU textures. Dropping UI
entities alone does not remove these cached atlases.

The Settings Camera2d also inherited Bevy's default 4x MSAA, unlike Nera's main
camera. Its extra multisampled target and pipeline/native allocation footprint
were unnecessary for this UI. The combined change below substantially reduced
process memory; the contribution of each individual allocator/GPU allocation
was not measured with a heap profiler.

## Focused Fix

The subsequent alarm audit extracted this same cleanup into
`utility_window/font_cache.rs`, shared by Settings and the two alarm windows.
`settings/memory.rs` now joins that scope before opening. Cleanup waits for the
last `FontCacheOwner` window/camera/UI root, so overlapping utility windows do
not retain each other's temporary glyphs permanently. The measurements below
are the original Settings-only run; see [alarm memory audit](ALARM_MEMORY_AUDIT.md)
for the follow-up rather than treating those historical numbers as a new sample.

- Only the Settings camera now uses `Msaa::Off`. Bevy UI's analytic edge/glyph
  smoothing remains enabled. Main-camera and 24/30 FPS policies are untouched.
- Before a new Settings window opens, `settings/memory.rs` snapshots existing
  font-atlas keys. Duplicate opens/minimize restores do not replace the snapshot.
- After both the window and `SettingsOwned` camera/UI root disappear, remove
  newly introduced atlas keys that no surviving text or ImageNode references.
  Remove their CPU Image assets as well; normal Bevy asset-removal handling then
  releases render assets and image bind groups.
- Preserve all pre-existing atlases and atlases referenced by other-window or
  hidden text/images. No avatar, alarm, radial-menu or shared asset purge occurs.
- OS close can remove the window in Last before owned UI cleanup in the next
  Update. Reclamation waits for both, rather than mistaking closing Settings text
  for a permanent shared-font owner.
- No dependency, new UI framework, filesystem polling or forced working-set
  trimming was added. Closed/idle checks allocate nothing; atlas/reference scans
  happen only once per completed close.

## Native Measurements

The debug-only `--settings-memory-audit` hook waits 10 seconds for startup,
opens Settings for 4 seconds, requests closure, waits 6 seconds for cleanup,
and repeats six times. Cycle four uses the real `WindowCloseRequested` OS path;
others use the existing close-button handler and mouse-release gate. The
successful native run exited automatically with code 0.

Both before/after runs used isolated temporary APPDATA profiles and the same
warm LOCALAPPDATA cache fixture. The user's configuration and running pet were
not modified. These are single-machine measurements, not release-wide budgets.

| Stage | Before Private | After Private | Before Working Set | After Working Set |
| --- | ---: | ---: | ---: | ---: |
| Before first open | 390.05 | 390.45 | 492.76 | 493.54 |
| First open | 493.54 | 429.33 | 605.75 | 542.35 |
| First close | 489.34 | 417.07 | 593.22 | 521.42 |
| Second close | 490.46 | 418.38 | 594.46 | 523.29 |
| Third close | 493.32 | 423.03 | 596.84 | 527.27 |
| Fourth close (OS request) | 494.18 | 424.13 | 598.31 | 528.97 |
| Fifth close | 494.39 | 424.61 | 598.61 | 529.57 |
| Sixth close | 498.59 | 426.07 | 602.03 | 530.99 |

First-close private-memory retention fell from **99.29 MiB to 26.63 MiB** above
each process's startup baseline, about a **73% reduction**. Process memory after
close was approximately **72 MiB lower** in both first and sixth samples.

Every after-close sample contained:

- Settings windows / owned roots+cameras / UI nodes / glyphs: **0 / 0 / 0 / 0**.
- Total cameras: **1**, the main pet camera.
- Settings-created font atlases/textures: **0**, compared with eight before.
- Images: **64**, back to baseline instead of 72.
- CPU image pixels: **2.67 MiB**, back to baseline instead of 10.67 MiB.

Every reopen recreated the expected eight atlases and 956 glyphs rather than
accumulating another set. Total animation clips grew from 17 to 19 in both
runs as existing autonomous idle behavior lazily loaded VRMA assets. That is
runtime animation-library ownership, not Settings preview loading. Consequently
the later approximately 9 MiB increase cannot all be attributed to Settings.

## Remaining Retention

Memory does **not** return exactly to the cold startup baseline. About 27 MiB
remained after the first close even with Settings fonts/images/entities gone.
Source inspection indicates shared UI/render pipeline and buffer caches plus
native/driver allocator capacity as likely contributors; their precise split
has not been profiled. These reusable global caches are deliberately not reset,
which could break other UI or cause shader recompilation/resource churn.

Private bytes, working set and dedicated GPU memory are different metrics;
this audit reports the first two, not dedicated VRAM. It does not hide retention
using `EmptyWorkingSet`, force a global AssetServer unload, or claim zero leaks
for every subsystem. Six bounded cycles validate Settings-owned cleanup, not
an hours-long soak or other GPU/DPI/custom-VRM configurations.

## Validation And Reproduction

- `cargo fmt --all` / formatting check: pass.
- `cargo check --quiet`: pass, existing dead-code warnings only.
- `cargo test --quiet`: **359 passed, 0 failed, 6 ignored**.
- Five new regressions cover six repeated font cleanup cycles, preserving
  pre-existing/avatar assets and hidden shared text/images, duplicate-open
  ownership, delayed OS UI cleanup, and camera/UI-tree destruction without
  changing main-camera MSAA or runtime FPS.
- Native six-cycle memory audit: pass, including OS-request closure.
- Existing `--feature-smoke --throw-settings-smoke`: pass, exit code 0.
  Default and 560x520 Settings screenshots were visually inspected; text,
  checkboxes, slider and sticky navigation render correctly with MSAA off.

For an isolated reproduction, build the debug binary and run
`target/debug/nera.exe --settings-memory-audit` with APPDATA/LOCALAPPDATA directed
to temporary fixtures. Without the flag no memory instrumentation is installed.
The hook emits `NERA_SETTINGS_MEMORY` JSON snapshots plus a final
`SETTINGS_MEMORY_AUDIT_PASS cycles=6` marker on stderr. An external 125-second
process timeout is recommended for native event-loop failures.

Raw local logs: `%TEMP%/nera-settings-memory-before.err.log` and
`%TEMP%/nera-settings-memory-final.err.log`. An interim attempt directly despawned
a native window too late in Last and triggered a winit redraw loop; it was killed
by its bounded timeout, the harness was corrected to use the real OS request,
and its partial results are **not** the final after-values above.

Still manually check normal Settings X/Back, repeated minimize/restore, text in
other utility windows after closure, 125%/150% DPI and a longer soak. The user's
older executable must be restarted/rebuilt to pick up these changes.

## Files In This Fix

- `src/settings/memory.rs` (new scoped atlas ownership/reclamation and tests).
- `src/settings/memory_audit.rs` (new debug-only native audit).
- `src/settings/mod.rs` (lifecycle scheduling and opt-in audit registration).
- `src/settings/ui.rs` (Settings-only MSAA and lifecycle regression).
- `src/animation_manager/process.rs` (only widens the existing debug memory
  helper's visibility for reuse; older gallery changes are separate).
- `README.md`, `ARCHITECTURE.md`, `SETTINGS_MEMORY_AUDIT.md`.

All earlier uncommitted character-gallery work remains intact.
