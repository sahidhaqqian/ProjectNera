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



Boleh. Ini rangkuman data yang sudah kita dapat sejauh ini.

| Tahap / Kondisi | Private RAM | Working Set | Catatan |
|---|---:|---:|---|
| Minimal Bevy | **2.9 MB** | 28.6 MB | Bevy core hampir tidak berat |
| Bevy + window, tanpa renderer | **6.4 MB** | 33.3 MB | Window cost kecil |
| Window + minimal 3D renderer | **106.0 MB** | 182.8 MB | Renderer dasar menambah ~99.6 MB |
| + PBR default | **238.9–240.5 MB** | ~325 MB | PBR menambah ~133 MB |
| + VRM plugin, tanpa model | **239.6 MB** | 327.3 MB | VRM plugin sendiri cuma ~0.7 MB |
| + `nera_256.vrm` | **347.0–368.5 MB** | 433–456 MB | Model Nera menambah ~107 MB |
| + idle VRMA | **372.7–373.8 MB** | 462–463 MB | Satu VRMA menambah sekitar ~25.7 MB |
| Full Nera | **390–403 MB** | ~500 MB | Seluruh fitur Nera lain cuma tambah ~18–30 MB |

Jadi pembagian kasarnya sekarang:

```text
Bevy core                  ~3 MB
Window                     ~3–4 MB
Renderer dasar            ~100 MB
PBR                       ~133 MB
VRM model                 ~107 MB
VRMA pertama              ~26 MB
Fitur Nera lainnya         ~18 MB
--------------------------------
Full Nera                ~390–400 MB
```

Temuan penting lainnya:

- **Bevy core bukan masalah.** Minimal Bevy cuma ~2.9 MB private.
- **VRM plugin bukan masalah.** Cuma ~0.7 MB tanpa model.
- Tiga monster RAM utama adalah:
  1. **PBR ~133 MB**
  2. **VRM model ~107 MB**
  3. **Renderer dasar ~100 MB**
- **VRMA juga lumayan mahal**, sekitar ~25 MB untuk idle pertama.
- Seluruh logic Nera lain—tray, settings, physics, alarm, interaction, dll—relatif kecil dibanding tiga komponen di atas.

Untuk PBR, kita sudah benchmark flag:

```text
D0 default                    240.5 MB
D1 prepass off                241.3 MB
D2 deferred off               239.4 MB
D3 GPU instance builder off   229.5 MB
D4 semua off                  231.0 MB
```

Satu-satunya yang ngaruh adalah:

```rust
use_gpu_instance_buffer_builder: false
```

Penghematannya sekitar:
- **~11 MB** di benchmark PBR
- **~7.8 MB** di full Nera
- CPU tidak naik

Tapi ternyata **karakter Nera tidak tampil**, jadi optimasi D3 **tidak aman untuk production** dan sudah kita revert.

Untuk CPU, hasil hard FPS limiter:

```text
15 FPS  → ~1%+
24 FPS  → ~2–3%
30 FPS  → ~3–4%
60 FPS  → ~5%
```

Jadi CPU sangat bergantung pada workload per-frame. Keputusan sekarang:
- 15 FPS dihapus
- opsi **24 FPS dan 30 FPS dipertahankan**
- hard limiter pakai `bevy_framepace`, bukan `reactive_low_power`

Kesimpulan posisi sekarang:

```text
RAM current dev build: ~390–400 MB private
CPU @24 FPS: ~2–3%
```

Dan arah optimasi berikutnya yang paling masuk akal adalah:

```text
1. Breakdown +107 MB dari model VRM
   - textures
   - morph targets
   - mesh
   - skinning
   - materials

2. Breakdown +25 MB dari VRMA
   - preload vs lazy load
   - baked/retargeted animation cost

3. Renderer allocator / wgpu
   - masih mungkin ada beberapa puluh MB
```

Jadi saat ini kita belum mentok, tapi **easy win sudah hampir habis**. Target 300–350 MB masih cukup realistis; target 100 MB kemungkinan butuh perubahan arsitektur/rendering yang jauh lebih besar.