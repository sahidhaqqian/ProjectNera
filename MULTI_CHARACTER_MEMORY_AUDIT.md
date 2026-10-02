# Multi Character Manager Memory Audit

Measured on Windows with the debug build, 2-3 October 2026, on
`codex/animation-manager-redesign`. This audit follows the earlier Phase 7 polish;
it does not change the main pet's 24/30 FPS policy or source VRM quality.

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
