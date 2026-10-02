# Multi Character Manager Memory Audit

Measured on Windows with the debug build, 2-3 October 2026, on
`codex/animation-manager-redesign`. This audit follows the earlier Phase 7 polish;
it does not change the main pet's 24/30 FPS policy or source VRM quality.

## Current Follow-Up: Cached Cards Only

The selected live rig was the remaining high-impact cost: the seven-card warm
baseline was 452.5 MiB private, and selecting the largest custom VRM reached
1040.8 MiB. The current gallery has **zero live VRM scenes**, including on a cache
miss. Source resolution, meshes, materials and animations are untouched.

Before: cached cards plus one selected live rig; missing thumbnails borrowed that
rig's render slot inside the manager. After: cached animated frame cards only. Selecting
a card only highlights it; **Use Character** explicitly assigns it to the selected
desktop slot. Import registers/highlights but never automatically loads a runtime rig.

The main library owns one serial background job for imports and legacy entries.
A hidden disposable worker records exactly one model, waits for every GPU readback
and cache publication, retires the scene, then exits. Process exit releases driver
and allocator high-water marks. Deadlines scale with duration: twice the recording
length plus 30 seconds internally, plus 60 seconds in the parent, which reaps its
own child on timeout. Failure is optional/nonfatal and never blocks Use Character.
Browsing itself never loads a rig, including cache misses. Cached/embedded static
images or older short atlases remain available while the full cache is prepared.

The initial ten-frame prototype recorded only a small portion of idle. It is now
replaced by **whole idle cycles covering at least 30 seconds**. The supplied idle
lasts 9.9167 seconds: four cycles produce **318 frames / 39.67 seconds** at 8 FPS.
Every visible card has its own looping player. Capture seeks exact sample times,
so readback latency cannot skip the middle/end of idle. Temporary capture runs at
30 FPS; browsing remains capped at 15 FPS, and main Nera remains at 24/30 FPS.

The cache is a version-2 JSON manifest plus concatenated JPEG frames, not video:
`.idle-v2.json` and `.idle-v2.frames`. The source/clip identity invalidates stale
caches. Frame dimensions are 256x320, JPEG quality 88. Visible cards retain compressed
bytes and only current/next decoded frames, reusing a stable Bevy Image/GPU texture.
One decoded RGBA frame costs **0.3125 MiB**; a 318-frame uncompressed atlas would
cost **99.375 MiB per card**, which this implementation explicitly avoids. Offscreen,
removed, hidden or closed cards release buffers, images and pending decode tasks.
Playback never reads files per frame. IO/capture state is bounded, with no video
codec or Bevy feature changes. BLAKE3 is now direct but was already transitive;
Cargo.lock adds no new package.

### Duplicate Identity

`character/library/import.rs` streams BLAKE3 with a 64 KiB buffer and a 128 MiB
input bound. Canonical paths are checked with content verification, so replacing
a file in place does not incorrectly reuse an old character. Renamed/moved copies
match their content fingerprint. New imports store original and migrated-content
hashes plus canonical source path in `catalog.json`. Legacy entries are populated
from their existing owned copies; normalized-content matching handles VRM0 imports
whose original bytes were not saved. Migration preserves existing entries, names
and source files; it does not silently merge earlier duplicate entries.

An existing match is highlighted with **Character already imported**, without a
new entry or copy. Hash/source fields deserialize with defaults for old catalogs.
Registry removal removes the identity, waits for existing safe runtime replacement,
then invalidates all derived cache versions before another import can start. Only
the owner deletes disk caches, avoiding a delayed worker invalidation racing a
reimport. The built-in avatar remains non-removable; original VRMs are never deleted.

### Current Measurements

Same seven-character catalog and warm cache, Windows debug build. Figures are
**private committed memory of the manager worker alone**, not all Nera processes
or GPU residency. Raw log: `%TEMP%/nera-manager-static-warm.log`.

| Stage | Previous MiB | Static Intermediate MiB |
| --- | ---: | ---: |
| Empty manager | 293.4 | 301.2 |
| One card | 449.9 | 303.9 |
| Seven cards, warm | 452.5 | **302.4** |
| Largest model selected | 1040.8 | **302.3** |
| After card removal | 549.8 | 305.8 |
| Hidden | 537.3 | 303.0 |
| Close cleanup, before process exit | not sampled | 303.0 |

Cards two through seven stayed between 302.1 and 302.4 MiB. Seven-card working set
was 422.5 MiB. Every stage reported zero VRM roots/assets, WorldAssets, skins/joints,
animation players/graphs/clips, VRMA assets, MToon materials and 3D cameras. The
gallery's native renderer/font/UI baseline remains around 300 MiB; reducing that
substantially would require changes beyond this focused patch. Closing the worker
releases it. Active desktop pets/companions still load their full-quality models
in separate processes. Import-only generation can still have a model-dependent
temporary memory peak; it is not a retained browsing cost.

The static intermediate measurements above are historical within this follow-up.
The final **full-duration animated-cache** run used the same seven models:

| Stage | Full Idle Private MiB | Compressed Frames MiB | Live Rigs |
| --- | ---: | ---: | ---: |
| Empty manager | 293.76 | 0.00 | 0 |
| One card | 298.94 | 2.54 | 0 |
| Two cards | 304.80 | 6.12 | 0 |
| Three cards | 309.36 | 9.70 | 0 |
| Four cards | 314.54 | 13.28 | 0 |
| Five cards | 318.70 | 16.17 | 0 |
| Six cards | 323.57 | 19.75 | 0 |
| Seven cards | **328.38** | 23.27 | 0 |
| Largest model selected | 328.25 | 23.27 | 0 |
| After removing six cards | 304.32 | 2.54 | 0 |
| Hidden | 301.77 | 0.00 | 0 |
| Close cleanup, before process exit | 297.78 | 0.00 | 0 |

Raw log: `%TEMP%/nera-manager-full-idle-warm.err.log`. Seven-card working set was
436.23 MiB; manager peak private commit was 328.41 MiB. This is about 27% below the
452.5 MiB baseline while adding full animated previews. Per-card compressed storage
was 2.54-3.58 MiB, plus at most two decoded 0.3125 MiB frames and one GPU texture.
All stages had zero VRM assets/roots, skeleton skins/joints, animation players,
graphs/clips, VRMA assets, MToon materials and 3D cameras. Source texture/mesh memory
was zero. The remaining roughly 294 MiB is native renderer/UI/font/thread overhead,
not retained avatar scenes. It is released when the manager process exits.

**The sub-500 MiB result is manager-only, warm-cache private memory on this machine.**
It is not a universal total-Nera or GPU budget. First-time background generation
still loads one full-quality source in a separate process: measured temporary peaks
were 454.7-1120.4 MiB, with 44.5-47.3 seconds per model. Each generator exited and was
reaped before the next started. Large models can still exceed 1 GiB during that
one-time operation or when applied as real desktop avatars. Their quality was not
silently reduced. A main-process exit may leave an already-running generator until
its bounded deadline; it cannot run indefinitely or become a browsing rig.

Native generation validated 318 frame images per model at 256x320, covering
39.67 seconds. Six sequences had 318 distinct pixel hashes, one had 317 (one
visually identical sampled pose). The compressed total for all seven was 23.27 MiB.
Verified matching full caches were also installed in the user's derived cache,
without changing source models or their current registry; removed entries were not
restored. A new import or changed idle automatically prepares a new cache.

### Changed Files

- `Cargo.toml`, `Cargo.lock`
- `src/animation_manager.rs`
- `src/animation_manager/characters.rs`, `ui.rs`
- `src/animation_manager/preview.rs`, `thumbnails.rs`, `process.rs`, `memory_audit.rs`
- `src/animation_manager/generation.rs` (new)
- `src/animation_manager/idle_cache.rs` (new)
- `src/character.rs`, `src/character/roster.rs`
- `src/character/library.rs`, `src/character/library/import.rs` (new)
- `src/character/library/previews.rs` (new)
- `src/character/library/removal.rs`, `release.rs` (release changes are test defaults only)
- `README.md`, `ARCHITECTURE.md`, `MULTI_CHARACTER_MEMORY_AUDIT.md`

### Validation And Remaining Checks

Regression tests cover browsing/cache misses without any rig, cached selection
without rebuilding, explicit Use, unchanged 24/30 FPS runtime policy, GPU readback
lifetime through all 240+ frames, complete playback/wrap and bounded decode state,
compressed-cache round trip/corruption rejection, whole-cycle duration selection,
renamed/canonical duplicate reuse without copying, stable content hashes,
legacy hash persistence, repeated highlight/removal selection safety, owned-copy
removal/reimport, cache-version invalidation, and hidden/closed preview cleanup.

`cargo fmt --all`, `cargo fmt --all -- --check`, `cargo check --quiet` and
`cargo test --quiet -- --test-threads=1` pass: **354 passed, 0 failed, 6 ignored**.
Ignored renderer/native asset tests remain opt-in. Existing dead-code warnings were
not refactored. Native build and seven-card staged memory audit also pass.

Native `--roster-smoke` ran for a full loop and observed **seven animated cards,
318 distinct displayed frame indices, 15.0 FPS, zero live rigs, 327.6 MiB private**.
Minimize cleared preview state; restore reloaded only caches; close cleanup passed
before exit. Desktop and 720x640 screenshots were inspected. Raw log:
`%TEMP%/nera-manager-full-idle-roster.err.log`. This validates cached animation
playback in the actual Windows UI, not just the cache files or unit-test clock.

A real VRM generated 318 usable 256x320 frames in the disposable worker and exited
successfully; sampled frames were visually inspected. The existing parent
cycle harness exited before returning its two-cycle completion markers on this
run; it is not counted as a successful parent soak test. Remaining manual checks:

1. Drag the manager title, select cards, and press Use in each desktop slot.
2. Import the same model from its original path and a renamed copy; confirm the
   message, highlight, unchanged registry count, and unchanged desktop avatar.
3. Delete the active imported character, verify safe default replacement, and
   reimport; inspect cache deletion and regenerated preview.
4. Repeat open/close and import/remove cycles; run a longer multi-pet soak.
5. Test 125%/150% DPI and other GPUs/release builds. Manager selection must remain
   zero-rig even for unusually large models.

## Historical Audit

The sections below describe the earlier one-live-rig implementation and its
measurements, not the current cached-only browsing architecture.

## Findings

The manager is a separate Bevy/wgpu process. Its renderer and GPU/CPU assets are
not shared with the desktop pet or companions. Within that process AssetServer
deduplicates live handles to the same path, but different imported paths remain
different assets, including imports with visually identical content.

The previous character gallery allowed two full live VRM previews while generating
card thumbnails. Once warmed, only the selected rig remained. Every uncached card
still passed through the full VRM/glTF loader, texture decoding/upload, cloned
WorldAsset, skeleton/skin initialization, VRMA rig, animation graph and retarget bake.
Offscreen/remove/hidden cleanup already removed those entities. Merely reducing
the simultaneous count from two to one would not prevent loading every large model.

The clearest bottleneck was catalog entry five, a roughly 24 MB source VRM. Creating
its thumbnail raised sampled private memory from 523.1 to 1038.6 MiB. At the next
settled sample the assets were back to one default Nera: one 3D camera, two
WorldAssets (VRM and idle VRMA templates), one animation graph and one clip.
The large allocation therefore outlived the discarded model rather than being
multiple permanently live avatars. Allocator/renderer/driver retained capacity is
the remaining inference; this audit does not claim a native heap allocation trace.

The existing AvatarMemoryPlugin already releases CPU VRM pixels and vertex/morph
arrays after upload. Final instrumentation measures **0 bytes** for both in settled
scenes. VrmAsset/VrmaAsset counts also return to zero: their temporary load handles
are removed by bevy_vrm1. The live WorldAsset instances and material/mesh/image
handles own the necessary render assets, not an accumulating VrmAsset cache.

Default Nera has 27 skinned mesh instances and 20 AnimationPlayer components in its
loaded hierarchy, sharing one graph/clip; these do not mean 20 independent avatars.
Joint counts in the log sum references across skins, not unique bones. Materials,
skins and players disappear with the rig. No new renderer, physics or audio stack
was introduced.

## Chosen Change

Keep the existing worker, UI, renderer and lifecycle. The character gallery now
has **one live VRM render slot** and static cached card images:

1. Read a valid, source-stamped PNG cache when available.
2. Otherwise read only the VRM JSON and its embedded thumbnail byte range.
3. If neither is usable, temporarily use the one render slot to generate a thumbnail.
4. Retain its camera through GPU screenshot readback, save a small derived PNG, and
   release the rig. After pending visible thumbnails finish, resume the selected model.

Thus the first visit to a model without a thumbnail can briefly pause the selected
live preview. It never creates a second simultaneous rig. Switching selection
retires the previous rig; a three-update gap allows dropped references to propagate.
At most twelve visible card images are resident, each at most 256x320. Thumbnails
and character render targets release their CPU pixel copies after upload.

The largest gain comes from avoiding full model loads for ordinary cards, especially
the large model that already has an embedded thumbnail. The disk cache then avoids
repeating full loads for models without one. A 256x320 RGBA image is only 0.3125 MiB
before compression; the generated example PNGs are approximately 21-24 KiB each.
Lower FPS reduces work frequency; it is not the main reason memory falls.

IO is bounded to one thumbnail read and two small PNG writes. It is triggered by
visible-card changes, not idle filesystem polling. Decoding allows PNG/JPEG with
2048x2048 dimensions and a 32 MiB decoder allocation limit; encoded thumbnails are
limited to 8 MiB and JSON to 4 MiB. Invalid or uniform/empty images fall back safely.
The direct `image` dependency exposes bounded decoding from the same version and
PNG/JPEG codecs already used transitively by Bevy. Cargo.lock adds no new packages,
and Bevy features are unchanged.

## Matched Measurements

These are **private bytes / MiB of the manager worker**, not total Nera memory,
physical RAM, or VRAM. Each stage adds one catalog entry while default Nera remains
selected. The final cold run used a new isolated cache; the warm run reused its
visually checked thumbnails. Samples are taken seven seconds after each addition.

| Stage | Before | After, cold cache | After, warm cache |
|---|---:|---:|---:|
| Empty manager | 293.5 | 293.5 | 293.4 |
| 1 character | 449.1 | 449.9 | 449.9 |
| 2 characters | 517.7 | 529.8 | 451.2 |
| 3 characters | 521.5 | 574.9 | 451.6 |
| 4 characters | 523.1 | 535.7 | 451.9 |
| 5 characters | 1038.6 | 503.7 | 451.9 |
| 6 characters | 821.3 | 539.3 | 452.2 |
| 7 characters | 829.4 | 507.3 | 452.5 |

At seven cards this is about **39% lower cold** and **45% lower warm** than the
matched baseline. Adding six cached cards costs about 2.6 MiB instead of loading
six models. Cold regeneration can still retain allocation capacity, so a one-rig
limit alone is not sufficient. Cold peak commit before selecting other live models
was 628.2 MiB; warm peak at that point was 453.6 MiB.

The warm seven-card worker's **total working set was 564.5 MiB**, including shared
resident pages. Private bytes are committed private memory and can include paged-out
memory; working set is resident memory and includes shared DLL/driver pages. Neither
is a measurement of dedicated GPU memory. Texture size logs estimate base-mip image
storage; they exclude mip chains, renderer buffers, shader caches and driver overhead.

## Limits Of The 500 MB Goal

Below 500 MiB private memory is realistic and measured for this warm gallery with
default Nera selected: **452.5 MiB, approximately 475 MB decimal**. It is not a hard
limit for physical working set, cold generation, every custom model, or the combined
pet/companions/manager processes.

Explicitly selecting the largest imported VRM still loads its full-quality assets:
the warm audit measured **1040.8 MiB private**, **1280.5 MiB peak commit**, and about
**227.3 MiB estimated base-mip image storage**. After removing the other cards and
returning to default, private memory was 549.8 MiB; after hiding, 537.3 MiB. At that
point there were zero VRM roots, 3D cameras, skins, players, graphs, clips and
WorldAssets. Closing the worker ends that process and releases its retained capacity.

A large model without an embedded/cached thumbnail can still incur a similar first
generation peak. A universal sub-500 MB limit would require an additional policy,
such as preview-specific reduced textures/geometry or an on-demand static-only
fallback for large models. Those are separate quality/UX decisions and were not
silently applied to the source or runtime avatar in this focused change.

## Resource Ownership

- Visible thumbnails are owned by PreviewRuntime, released offscreen, on removal,
  on hide/minimize and on close. No hidden rig continues animating.
- Generated PNGs persist under LocalAppData
  `io.github.sahidhaqqi.projectnera/thumbnails-v1`. They are a disk cache, not retained
  live scenes. Keys include the asset path, source length and modification time.
- A removed roster entry invalidates its derived cache. No original or imported VRM
  is changed by thumbnail generation. The existing registry-removal workflow remains
  responsible for owned import files and safe active-character replacement.
- Dropping a rig releases its child hierarchy and references normally. There is no
  global AssetServer purge that could invalidate another live/shared asset.
- Closing the manager terminates its existing isolated worker. Each active desktop
  companion still owns a renderer; this audit does not remove that cross-process cost.

## Reproduction

Build with `cargo build --bin nera`, then run:

```powershell
.\target\debug\nera.exe --animation-manager-worker --manager-memory-audit
```

This debug-only scenario reads the catalog, adds cards one at a time, changes the
selection, selects the largest source, removes cards from its temporary UI state,
hides and closes. It does not remove or change persisted character entries. It emits
`NERA_MANAGER_MEMORY` JSON with private/working-set/peak commit, live/cached previews,
entities, camera/skin/player counts, asset counts and CPU/image byte estimates.

For parent-process baseline and two real open/close cycles, launch the normal debug
binary with `NERA_GALLERY_HOST_SMOKE_TEST=1` and `NERA_GALLERY_HOST_CHARACTERS=1`.
Use an isolated APPDATA for this native smoke because the main pet can save settings.
The worker's existing `--roster-smoke` also captures desktop/compact views, checks
the 15 FPS rate, minimizes/restores, and verifies preview cleanup before process exit.

Raw audit logs from this run are in the local Temp directory:
`nera-manager-audit-before.log`, `nera-manager-final-cold.log`, and
`nera-manager-final-warm.log`. Native renderer memory varies by GPU/driver/build;
these are measurements on this machine, not release-wide budgets.

## Files In This Follow-Up

- `Cargo.toml`, `Cargo.lock`
- `src/animation_manager.rs`
- `src/animation_manager/preview.rs`
- `src/animation_manager/process.rs`
- `src/animation_manager/thumbnails.rs` (new)
- `src/animation_manager/memory_audit.rs` (new, debug-only)
- `src/character.rs` (safe metadata source resolution)
- `README.md`, `ARCHITECTURE.md`, `PHASE_7_POLISH_VALIDATION.md`
- `MULTI_CHARACTER_MEMORY_AUDIT.md` (this report)

Earlier uncommitted Phase 7 changes remain in the branch and are not attributed to
this memory-audit follow-up.

## Final Validation

- `cargo fmt --all` and `cargo fmt --all -- --check`: PASS.
- `cargo check --quiet`: PASS, existing dead-code warnings only.
- `cargo test --quiet -- --test-threads=1`: **341 passed, 0 failed, 6 ignored**.
  An earlier parallel manager run hit an existing timestamp collision between
  temporary-directory fixtures in `base.rs`; that unrelated test helper was not
  changed. The full serial suite includes every non-ignored test.
- `cargo build --bin nera --quiet` and `git diff --check`: PASS.
- Four added regressions cover bounded embedded-thumbnail decoding, invalid/blank
  fallback, source-stamped cache identity and preserved pixels, and keeping the
  camera alive until GPU screenshot readback. Existing preview regressions now
  enforce one live rig, old-selection destruction, frozen-card reuse and cleanup.
- Cold and warm native audits: both exited successfully. Generated VRM 0.x
  thumbnails were visually inspected; the initial black-readback prototype was
  fixed before these final measurements. Its provisional runs are not the reported
  after-values above.
- Two native parent/worker open-close cycles: PASS. Worker samples were **452.2**
  and **452.7 MiB private**, seven cards, one live rig, **15.0 FPS**. Minimize cleared
  previews, restore rebuilt bounded state, and close left zero preview roots/state
  before worker exit. Desktop and 720x640 screenshots were visually inspected.
- Parent private memory before opening: **389.89 MiB**. After worker close one:
  **389.70 MiB**; after close two: **389.71 MiB**. No manager allocation growth was
  observed in the pet process. This run used a private AppData/catalog fixture;
  the user's persisted settings and library were not changed.

The native parent log is `nera-manager-final-host.log` in Temp. Longer soak tests,
other GPUs/drivers, release-build budgets and unusually large/malformed custom VRMs
remain unmeasured. The existing 24/30 FPS isolation regression passes; no main
runtime FPS owner was modified.
