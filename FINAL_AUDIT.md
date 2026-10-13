# Project Nera final audit

Audit: 2026-10-13–14 (Asia/Bangkok). Feature freeze; single agent.
Candidate: 0.1.0, Windows x64, proprietary binary distribution.
Source: `codex/animation-manager-redesign`, HEAD `c421e92` plus the current
uncommitted working tree. Manifest hashes identify the packaged files.
Previous milestone reports were not treated as evidence of passing this audit.

## Verified findings and fixes

| Severity | Demonstrated problem | Fix and validation | Result |
|---|---|---|---|
| P0 | Library traversal/mutations could follow NTFS links outside the owned root and reach original files | Canonical containment on mutation/import/traversal plus visited-directory guard; real junction regression verifies originals cannot be renamed/deleted/imported through the escape | PASSED |
| P1 | Release assets preferred the compiler's repository over an extracted application | Executable-relative release assets; development fallback only in debug; incomplete-package regression | PASSED |
| P1 | Missing core resources could leave an invisible application | Bootstrap VRM/idle/shader check, actionable local log and native dialog; incomplete package exits 1 | PASSED |
| P1 | Initial tray registration failure destroyed its recovery callback and could strand Start Hidden | Retain HWND/message pump and Explorer recovery registration; native injected-failure regression closes the callback cleanly | PASSED |
| P1 | GUI release lacked persistent panic/error diagnostics; settings errors preceded logging | Bounded local logging before settings, retained panic hook; real failing child-process regression and native corrupt-config recovery | PASSED |
| P2 | Manual grounded clips used toes/hands as contact, disagreeing with desktop ankle/sole convention during Chibi transitions | Reuse existing GroundingPolicy: grounded clips use feet; authored crouch/get-up poses retain full-body contact. No new placement owner or Chibi proportions; Relax toggles and Standing Up 3/checkpoint native cases pass | PASSED |
| P2 | Generic/missing executable, tray and utility-window icons | Supplied transparent artwork at seven ICO resolutions; embedded metadata, shared native icons on Bevy windows/tray | PASSED on final native release |
| Release | Classic MP3 provenance unknown | User-authorized CC0 Clock Beeps variation; sound ID 0 retained, prior MP3 excluded | PASSED |
| Release | Missing portable CRT/dependency/font notices | Required signed Microsoft x64 Redist DLL and dependency/asset/font notices added | Final package below |
| Build | Mandatory machine-specific wrapper/linker configuration; severe full-LTO paging on 8 GiB host | Remove local compiler paths; opt-level 3 + Thin LTO, package codegen units 8, dependencies retain their profile | Final build below |

Windows rotation initially failed with append-only handles. It was corrected to
explicit seek positions under the per-process mutex and regression-tested.
Logs are bounded to 2 MiB each; four inactive logs remain alongside active logs.

No known unresolved P0/P1 or reproducible release-blocking P2 defect remains in
completed scenarios. This bounded audit does not certify every rig/driver/PC.

## Architecture and scope

Current owner/exit paths were inspected: bounded VRMA playback/cache; desktop
support/checkpoint motion; Cloud/presentation; Focus/Treat/AutoDance priorities;
replacement, expressions and spring ordering; Settings/Alarm UI; worker IPC and
teardown; persistence/import containment; WASAPI packet/format bounds, COM cleanup,
retry and shutdown. Bevy 0.19.1 already validates glTF cycles before scene spawn;
no redundant validator or loader rewrite was added.

Pre-existing uncommitted Death01, 2D walking-depth, garment-spring and gaze-setting
work is preserved and included. Not all dirty files are audit changes. Chibi's
proportions, reversible layer, springs, Cloud/Alarm integration and owner remain.

Native harness corrections retain assertions/tolerances: Cloud release is sampled
after the production solver's next frame; standing after Alarm/drag is sampled
after authored Landing and idle calibration; authored Chibi root restoration is
checked beneath checkpoint contact correction. Earlier failures remain in
`target/final-audit`; final passing runs are identified below. Tests use semantic
commands, real HWNDs/VRMs and GPU rendering without moving/typing user input.

## Performance and memory

Host: i5-1155G7, eight logical processors, 8 GiB RAM, Intel Iris Xe, Windows x64.
Matched debug idle runs: bundled Nera, 24 FPS, muted, roaming off, sequential,
compiler stopped; 60 seconds each, averages exclude the first 25 seconds.

| Measurement | Cached before debug | Current debug |
|---|---:|---:|
| Private memory | 440.4 MiB | 444.6 MiB |
| Working set | 542.2 MiB | 546.7 MiB |
| CPU, one-core percentage | 30.10% | 27.10% |
| CPU, normalized across eight threads | 3.76% | 3.39% |
| First visible HWND | 2.97 s | 0.57 s |

Single-host samples include cache/order/background-work variation; HWND timing is
not avatar-ready time. No statistically established speed-up is claimed. About
4 MiB private-memory difference is not evidence of a persistent leak.
Raw reports: `target/final-audit/idle-before` and `idle-after`.

| Native lifecycle | Measurement and cleanup |
|---|---|
| Settings, six cycles | Private 439.4 MiB before, 531.8 after first close, 539.5 after sixth; closes 2–6 rise about 2.1 MiB. Closed UI nodes/windows, font keys/textures/CPU bytes all zero; images 68 and one camera |
| Alarm, three editor + three ringing cycles | Private 439.7 MiB before, 557.3 after final close. Retained audio cache about 2.13 MiB; no editor/ringing/preview entities or windows; font/UI counts return to zero |
| Manager, two worker cycles | Main private 447.5 MiB before, 445.1 after both closes. Workers exit; Windows reclaims their renderer lifetimes |
| Focus/Work, repeated sessions | Full activity/UI lifecycle peaks at 611.6 MiB private, ends at 587.7 MiB. First Work about 559.5 MiB. Props/owners clean up. Includes UI, Cloud, animation, screenshot and driver allocations; does not isolate GLB cost |
| Nera → Alicia replacement during Treat | Private memory about 618.4 MiB before transition, peak 645.8, then 621.9 after settling and 624.9 at run end. Old Treat props/ownership removed; custom-rig animation/cleanup passes. One transition, not a long-running replacement stress test |

Work GLB stays original: 11,612,308 bytes, 267,920 triangles, three original
1024 textures. No mesh/texture reduction. UI high-water RAM partly remains after
font/entity cleanup; later measured cycles plateau rather than demonstrating
ongoing application-resource growth. Hours-long soak/GPU allocation trace skipped.
Replacement samples are in `target/final-audit/replacement-memory`. Compilation
was active for that additional lifecycle run, so no CPU comparison or isolated
working-set performance claim is made from it.

Final release: 180-second native run, 90 samples, normal close and no crash.
Steady averages: 444.6 MiB private, 513.1 MiB working set, 21.21% of one core
(2.65% across eight threads). Per-process 3D-engine snapshot: 7.14%; not a
long GPU trace. Warm HWND appeared in 0.62 s. First/last steady 30-second private
averages: 439.4/446.6 MiB (+7.2). Growth coincides with lazy Blush/Catwalk/Sleepy
clip loads; final 30 seconds span only 446.45–446.85 MiB. This is a short plateau,
not proof of hours-long leak freedom. Raw data: `release-soak/report.json`.

## Validation

| Gate/scenario | Result | Current-run evidence |
|---|---|---|
| cargo fmt --check | PASSED | fmt-check.log |
| cargo check --locked -j1 | PASSED | check-final.log; final refresh 7.37 s |
| cargo test --locked -j1 | PASSED: 626 passed, 0 failed, 11 ignored | tests.log; 4.67 s test execution, plus compilation |
| cargo build --release --locked -j1 | PASSED | release-final.log; 42 min 08 s, all release binaries |
| Native Chibi, Nera + Alicia Solid | PASSED | smokes-contact-complete: restoration, walking, headpat, VRMA changes, drag/release, taskbar, Rest/Ride, Alarm deferral, replacement, finite springs; 54 captures |
| Native manual/checkpoint animation | PASSED | smokes-contact-complete: 434 contact frames; Standing Up 3, directional walk, repeated Chibi, Alicia, seated return, Alarm/Cloud/drag priority |
| Cloud | PASSED | smokes-rerun: Rest/Ride, release/bounds and ownership |
| Presentation/Screensaver/Alarm | PASSED | smokes: restoration, real Alarm preemption, viewer gaze, Normal/Chibi headpat, seven routine clips/firefly |
| Focus | PASSED | smokes-final: 16 checks, countdown, Study/Work prop/gaze, headpat, drag, taskbar/Ride, stop/completion, real Alarm priority/repetition |
| Treat | PASSED | smokes: seven props, hand/bite/mouth/hearts/cleanup, Normal/Chibi/Alicia, replacement/Rest/Ride/drag |
| Touch | PASSED | smokes: eight clips, seven-touch streak/FX, both sides, cooldown, drag, mixed regions, walking/taskbar, Cloud/Alarm/replacement |
| Companions/Manager/Settings/Alarm lifecycle | PASSED | smokes; metrics above |
| Corrupt settings | PASSED | corrupt-config: default recovery and diagnostic; original test profile restored |
| Incomplete package | PASSED | Native dialog/log/exit 1; missing-package.log |
| Official icon/GUI subsystem | PASSED on final release | native-release-final: Shell default-open, PE subsystem 2, main/Settings/Alarms/Manager icons, no console (Win32 error 6), local log, tray registration |
| PNG transparency/visual inspection | PASSED | Native presentation alpha counts; Study/Work/Screensaver and both Chibi rigs inspected |

Logs/reports above are under `target/final-audit`. The 11 ignored tests require
opt-in assets/GPU/gallery or quiet audio output. Corresponding smoke runs do not
mean ignored test functions ran. Long Focus durations are explicitly accelerated;
no claim of waiting 25/50 real minutes or listening to muted smoke audio.

## Distribution and remaining qualifications

Proprietary LICENSE reflects the user's choice. Owner confirmed distribution
authority for supplied avatars, Cloud/workstation and animations. Default VRM
metadata also permits redistribution/corporate commercial use, author
siroihakumai. CC0 props/audio, MIT migration attribution and dependency/font
notices remain separate. Prior unverified Classic MP3 is removed.

Packager checks runtime assets/LFS pointers, includes required signed Microsoft
x64 CRT from Visual Studio Redist, and records manifest/ZIP SHA-256. Release
lookup never uses a developer repository. No Rust/Cargo runtime process required.
CI now runs fmt/check/test/release and identical package/PE gates; hosted workflow
**NOT PERFORMED** because no push/dispatch was requested.

**NOT PERFORMED:** physical double-click/tray clicks; taskbar/Alt+Tab visual QA;
clean Windows 10/11 PC without developer tools; mixed-DPI/multiple monitors;
disconnected audio device; live YouTube/AutoDance-to-manual transition; audio
listening; hours-long soak. No exposed external-window edge with headroom existed
in Chibi/touch smoke, so native window-sitting checks skipped; taskbar/regressions
passed. Media/endpoint handling was reviewed/regression-tested without claiming
live music playback. Persisted local DancePool has four user entries, unchanged
and excluded from distribution.

Final release executable: 64,973,824 bytes, version 0.1.0, x64 Windows GUI.
SHA-256: `BA5EB6AF3CF14160AEA2A90E7FA8FE05B56557FFFDD205E309CEFC2284E85913`.
Native Shell launch produced its visible HWND in 2.80 s. The manager launched
without a console and exited cleanly; main/Settings/Alarms/manager icon slots
were verified. Loaded `VCRUNTIME140.dll` comes from the package directory, not
the development installation; other dependencies are Windows system DLLs.
The optional shortcut was generated in the audit folder and verified to target
this package's executable/icon. No shortcut was written to the user's Desktop.

Unsigned executable/signing reputation and clean-machine QA remain distribution
qualifications. Ordinary Rust panics are logged; native/driver faults are not
claimed to produce Rust backtraces or automatic dumps.
Observed P3 log noise remains: startup can report idle fallback while async clips
load, and Bevy can report an already-destroyed window after utility close. Native
load/close/resource cleanup passed; these warnings were not treated as crashes.

Final verdict: **CONDITIONAL GO** for this tested Windows x64 portable candidate.
No known P0/P1 or release-blocking P2 remains. Signing and clean-machine/hardware
QA above remain qualifications; this is not blanket certification of all PCs.
Artifact: `target/distribution/Nera-0.1.0-windows-x64.zip`, with SHA-256 sidecar
and file manifest. It contains the same natively verified executable/assets,
app-local CRT and notices. Personal animation collections are excluded; retain
their folders or re-import when upgrading. Hosted CI and publication were not
performed, and no commit/tag/push was created by this audit.

## Files changed

- Release/bootstrap: Cargo.toml/lock, build.rs, local .cargo config removal,
  main.rs, application_assets/icon.rs, diagnostics.rs, worker logging, tray.
- Defects/regressions: animation_manager registry and UI test fixture,
  library_motion, avatar-anchor validation, native Chibi/Cloud/checkpoint sampling.
  Existing unrelated dirty files preserved.
- Distribution: branding, classic.wav/paths/credits, LICENSE/third-party/font/
  dependency notices, release/validation/shortcut tools, Windows CI, README,
  ARCHITECTURE, FINAL_AUDIT and RELEASE_CHECKLIST.

No product feature, preset, animation stack or economy/simulation added.
Release work stops after these gates/deliverables.
