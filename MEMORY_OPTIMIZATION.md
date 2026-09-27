# Avatar memory investigation — 2026-09-27

The existing desktop process used 696 MiB working set and 600 MiB private
memory when sampled. CPU throttling does not release resident assets, compiled
shaders, graphics-driver allocations, or renderer caches. This sample alone
does not establish a leak.

## Changes

- VRM mesh assets now use `RenderAssetUsages::RENDER_WORLD` before their first
  render extraction. Bevy moves vertex/index/morph data to the renderer instead
  of keeping a duplicate on the CPU. Mesh bounds remain available.
- VRM texture pixels follow the same policy. Texture dimensions and samplers
  remain available; resolution and model geometry are unchanged.
- Both the desktop pet and gallery worker use this policy. Generated shadow
  meshes, UI images, and render targets are excluded.
- Native GPU debug layers and shader debug information are opt-in even in the
  debug executable. Enable `WGPU_DEBUG=1` and `WGPU_VALIDATION=1` for graphics
  debugging. wgpu validation and DX12 indirect-call validation remain enabled.
- Existing frame pacing, animation scheduling, and GPU mesh preprocessing stay
  in place. No working-set trimming or forced garbage-collection timer is used.

## Measurement

Separate test processes loaded the bundled avatar, rendered to a 600 × 950
offscreen target, then despawned and recreated the avatar using the same asset.
The baseline uses the previous renderer settings and retains CPU asset data.

| Sample | Baseline private / working set | Optimized private / working set |
| --- | --- | --- |
| First avatar | 405 / 470 MiB | 377 / 446 MiB |
| Recreated avatar | 377 / 443 MiB | 350 / 419 MiB |

This is about **27–28 MiB less private memory (7%)** in this workload. The
baseline retained 11,805,840 bytes of dense morph data and 8,454,912 bytes of
texture pixels, plus vertex/index data. The optimized run retained no CPU
geometry/pixel payloads for the 27 VRM meshes and 54 texture assets.

These are renderer/asset tests, not a before/after measurement of the complete
desktop application. They exclude the desktop animation library, utility
windows, and user interactions. The remaining renderer/driver footprint is
substantial; do not interpret the test's 419 MiB as the new desktop RAM total.
Allocator reuse and shader warm-up also affect process-level samples.

## Validation and reproduction

`cargo check` and `cargo test` passed. The GPU test passed with both policies,
including a second instance of the cached avatar and verification that GPU
morph geometry remains present. Native visual inspection of desktop interaction,
expressions, shadows, character import, and gallery scrolling remains necessary.

```powershell
cargo test memory::tests::avatar_geometry_survives_gpu_only_upload_and_respawn -- --ignored --nocapture
$env:NERA_MEMORY_BASELINE = '1'
cargo test memory::tests::avatar_geometry_survives_gpu_only_upload_and_respawn -- --ignored --nocapture
Remove-Item Env:NERA_MEMORY_BASELINE
```

The optimization relies on Nera's bone-based hit testing and immutable avatar
geometry. A future CPU mesh editor or VRM first-person mesh-splitting feature
must retain CPU data for the meshes it modifies.

A separate executable was built at `target/debug/nera-memory.exe` so the running
desktop process need not be stopped or overwritten. Exit the old Nera before
launching this executable. Normal subsequent builds include the same changes.
