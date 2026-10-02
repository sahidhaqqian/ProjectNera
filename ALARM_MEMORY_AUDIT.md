# Alarm Memory And Framing Audit

Measured on 2026-10-03 on Windows with debug `nera.exe`, the built-in Nera,
isolated temporary profiles, muted audio, and the unchanged 24 FPS setting.
These are **whole pet-process** private bytes and working set, not the alarm
panel alone, the separate Character Manager, or dedicated VRAM. Units are MiB.

## Findings

- The existing editor/dismiss handlers already destroyed their window, camera,
  and owned UI trees. No surviving alarm UI tree was found after close.
- Both alarm UI cameras inherited Bevy's default 4x MSAA. Multisampled targets
  are unnecessary for these text/control panels and add renderer allocations.
- The global `FontAtlasSet` retained 12 glyph atlases after opening the editor's
  list, edit, time, and date screens. Ringing added three more retained atlases.
  They are 512x512 RGBA images: 12-15 MiB CPU pixels plus GPU copies. Despawning
  UI alone does not release this cache.
- A long sound preview continued playing/loading after its editor closed.
  Runtime ringing already had its own audio cleanup path.
- Five startup sound handles retain about 1.008 MiB compressed source data.
  This is shared, stable baseline data, not the large memory bottleneck.
  AssetServer already shares loads for identical paths; the old preview resolver
  did not create independent copies of the sound source on every click.
- Attention enlarges the primary transparent 3D window to 1690x842 on this
  1920x1080 monitor. Its renderer allocation high-water mark is significantly
  larger than opening only the small editor. The main window restores to
  600x950 after dismissal; it does not remain accidentally enlarged.

## Focused Changes

Alarm editor and dismiss cameras use `Msaa::Off`, without changing the main
camera, animation, source VRM, audio backend, or runtime FPS policy.

Settings and both alarm windows share one scoped font-cache owner in
`utility_window/font_cache.rs`. It captures the original keys once and waits
until every tracked window/camera/root has disappeared, including delayed OS
close cleanup. It removes new, unreferenced glyph textures/atlases only;
pre-existing keys and atlases referenced by any remaining text or ImageNode,
even hidden UI, survive. Overlapping utility windows cannot independently
capture each other's temporary fonts as permanent baseline data.

Closing the editor despawns its preview players, including late/loading
requests, without touching runtime ringing. Preview resolution explicitly
reuses the five startup handles. Last request wins, and only one preview player
is retained. No global AssetServer unload, renderer reset, working-set trim,
extra audio stack, or dependency was added.

Cleanup checks are trivial while idle. Atlas/reference enumeration happens
only after the final scoped owner disappears. The native measurement hooks are
debug-only and installed only when explicitly requested.

## Measurements

The comparable runs below have no screenshot readback allocations. Each ran
three editor cycles, followed by three ringing/dismiss cycles. Close samples
were taken six seconds after requesting closure, including real OS close paths.

| Stage | Before Private | After Private | Before Working Set | After Working Set |
| --- | ---: | ---: | ---: | ---: |
| Cold pet baseline | 390.10 | 390.11 | 493.07 | 493.28 |
| Editor list, cycle 1 | 482.40 | 418.25 | 596.64 | 532.77 |
| Editor pickers, cycle 1 | 487.29 | 423.03 | 601.68 | 537.77 |
| Editor closed, cycle 1 | 483.25 | 406.77 | 588.13 | 511.80 |
| Editor closed, cycle 3 | 485.15 | 409.55 | 590.63 | 515.34 |
| Ringing, cycle 1 | 508.11 | 497.01 | 638.58 | 628.61 |
| Ring dismissed, cycle 1 | 509.48 | 490.38 | 618.61 | 600.42 |
| Ring dismissed, cycle 3 | 510.54 | 492.32 | 621.08 | 603.12 |

Both first editor cycles had 17 loaded animation clips. Later autonomous clip
timing differed (19 clips before ringing versus 18 after), so ringing rows
are indicative lifecycle samples, not an isolated measurement of this patch.

First editor-close excess private memory fell from 93.15 to 16.66 MiB, about
82% less retention. Every after-close sample has zero alarm UI nodes, editor
and dismiss owners/windows, sound preview players, ring players, and glyph
atlases; image count returns to 64, with only the main camera remaining.

The final screenshot-enabled recheck also passed all six cycles. Editor close
was 407.84 MiB on cycle 1 and 416.25 on cycle 3; final ring close was 501.11.
That run loaded an additional autonomous clip and used screenshot readbacks,
so it is not an apples-to-apples memory improvement measurement. It confirmed
`attention=false`, `main_window_physical=[600,950]`, fonts=0, UI=0, players=0
at each close. The shared-cache Settings recheck passed six cycles as well:
389.92 baseline, 416.76 first close, 425.16 final close, no retained Settings UI
or atlases. Original Settings measurements are in `SETTINGS_MEMORY_AUDIT.md`.

## Remaining Memory Limitation

Ringing still leaves roughly 100 MiB above the cold baseline in the ordinary
after-run, despite owned UI/audio/font cleanup. The samples remain around
490-492 MiB over three dismissals, not an accumulating alarm entity leak.
Global render pipelines, driver/allocator reserves, and lazily loaded VRMAs
can survive panel closure. The precise allocation split has not been measured
with a heap/GPU profiler; the renderer explanation is an inference, not proof
that all residual memory is harmless. Dedicated VRAM was not measured.

Do not claim that all alarm-related RAM returns immediately to Windows or that
six cycles establish an hours-long leak-free soak. Resetting the main renderer
to force its high-water footprint down would be a much more invasive change.

An existing scheduling diagnostic also occurs in both before/after runs:
`Attention failed to start within 2s` is logged on normal dismissal because
the Update watchdog observes DesktopPet before PostUpdate exit cleanup handles
PresentationExited. The native checks confirm actual restoration/audio stop.
This unrelated watchdog scheduling behavior was not changed in this patch.

## Cropped Head Fix

The original camera fitted the animated head **joint**, with only 18% of the
head-to-hips span above it, and calculated perspective distance at z=0. The
joint is not the crown; hair/headwear and forward JumpHappy motion enlarged
the visible head beyond that fitted viewport. The native before screenshot
reproduced the head intersecting row 0.

Attention now includes a 40% crown/headwear allowance and fits at the closest
animated upper-body depth, including a small mesh-thickness margin. The locked
composition center and monotonic zoom-out policy remain; normal desktop camera
settings and window/FPS policy are unchanged. This adds only a scalar maximum
to the existing bone scan, with no mesh scanning or per-frame allocation.

Native after screenshots at 3 seconds (JumpHappy), 9 seconds and 29 seconds
(Wave) show the whole head. Their first visible rows on the black screenshot
background were 107, 230 and 199 of 842, respectively; the before 3-second
capture intersected row 0. Raised hands still use the existing pose-fitting
behavior. On dismissal the original presentation snapshot is restored.

The bounds are a humanoid-rig approximation, not arbitrary mesh/headwear
measurements. Custom oversized hats, extreme poses and other character scales
still require manual checks.

## Validation And Reproduction

- `cargo fmt --all` and formatting check: pass.
- `cargo check --quiet`: pass, existing dead-code warnings only.
- `cargo test --quiet`: **367 passed, 0 failed, 6 ignored**.
- Eight added regressions cover shared overlapping font ownership, repeated
  editor/dismiss cleanup without changing main-camera/FPS settings, early/late
  preview stop, bounded preview resolution using shared assets, head crown fit,
  depth translation, and raised-hand/jump fit at a locked camera center.
- Native six-cycle alarm audit: pass, exit 0.
- Native six-cycle Settings regression: pass, exit 0.
- Native alarm framing audit: pass, exit 0; before/after screenshots inspected.
- Editor list, date picker and dismiss panel screenshots inspected with MSAA off.

Build `cargo build --bin nera`, then run the debug executable with a temporary
APPDATA/LOCALAPPDATA profile to avoid altering normal user configuration:

```text
target/debug/nera.exe --alarm-memory-audit
target/debug/nera.exe --alarm-memory-audit --alarm-memory-screenshots
target/debug/nera.exe --alarm-framing-audit
```

The first emits `NERA_ALARM_MEMORY` JSON and ends with
`ALARM_MEMORY_AUDIT_PASS editor_cycles=3 ring_cycles=3`. The framing variant
captures the primary portrait at 3/9/29 seconds, dismisses normally, then
checks cleanup and exits with `ALARM_FRAMING_AUDIT_PASS captures=3`. Allow an
external 190-second timeout, including renderer startup. A preliminary
80-second before-capture run hit its external timeout after capturing all
three poses; it is used only for visual reproduction, not cleanup validation.

Raw local logs are in `%TEMP%/nera-alarm-memory-{before,after,final}.err.log`,
`nera-alarm-framing-{before,after}.err.log` and
`nera-settings-memory-shared.err.log`. Screenshots are
`%TEMP%/nera-alarm-portrait-{3s,9s,29s}.png` plus the preserved
`nera-alarm-portrait-before-3s.png`, and `nera-alarm-memory-{list,date,dismiss}.png`.

Still manually validate audible preview/ringing/snooze, overlapping windows,
125%/150% DPI, custom VRMs/headwear, character-size extremes, multiple monitors,
fullscreen transitions, minimize/restore, and an extended memory soak. Native
tests muted audio and exercised the real lifecycle, not human pointer input.

## Files In This Follow-Up

- `src/utility_window/font_cache.rs` (shared cache owner/reclamation).
- `src/utility_window.rs` (shared cleanup scheduling).
- `src/settings/memory.rs`, `src/settings/mod.rs`, `src/settings/ui.rs`
  (join shared ownership; preserve earlier Settings cleanup behavior/tests).
- `src/alarm.rs`, `src/alarm/sounds.rs` (reuse source handles, bounded resolver
  regression and debug audio counters).
- `src/alarm_settings.rs` (editor MSAA, ownership, preview cleanup/tests).
- `src/alarm_settings/memory_audit.rs` (opt-in native memory/framing checks).
- `src/alarm_dismiss.rs` (dismiss MSAA, ownership, lifecycle test/counters).
- `src/presentation.rs` (alarm-only headroom/depth fit and three regressions).
- `README.md`, `ARCHITECTURE.md`, `SETTINGS_MEMORY_AUDIT.md`, this report.

Earlier uncommitted Character Manager changes are preserved and are not part
of this alarm follow-up. No sound, model or animation asset was changed.
