# Project Nera release checklist

Candidate: 0.1.0, Windows x64, proprietary. Audit: 2026-10-13–14, Asia/Bangkok.
Statuses below describe this run, not old milestone reports.

| Requirement | Status | Evidence / qualification |
|---|---|---|
| Feature freeze; existing owners preserved | PASSED | Release integration + demonstrated fixes; no product features |
| Official transparent ICO, 16/24/32/48/64/128/256 | PASSED | Original PNG conversion, alpha preserved, small-size inspection |
| Executable name/version/icon resource | PASSED | Native metadata 0.1.0, extracted resource icon |
| Release Windows GUI subsystem | PASSED | PE subsystem 2; Windows Shell default-open outside a command launcher |
| No release console | PASSED | Main/Manager console attach returns Win32 error 6; no console HWND |
| Debug terminal logging | PASSED | Debug PE subsystem 3 and real captured terminal diagnostics |
| Main/Settings/Alarms/Manager icons | PASSED | Native WM_GETICON and extracted PNGs |
| Tray official icon/registration | PASSED (registration/resource) | Shared executable icon; native registration log and failure-recovery test; physical tray visuals skipped |
| Desktop shortcut icon | PASSED | Final generated .lnk targets packaged exe, IconLocation exe,0 |
| Manager/companion shutdown | PASSED | Real worker and companion smoke; no orphan ownership; existing no-console creation flags retained |
| Missing core assets | PASSED | Actionable native dialog/log and exit 1 |
| Missing/corrupt settings recovery | PASSED | Isolated malformed TOML launch loads defaults and preserves error log |
| Bounded logs/ordinary panic diagnostics | PASSED | LocalAppData/ProjectNera/logs; rotation and real failing-child regression |
| Library file containment | PASSED | Native NTFS junction regression protects originals |
| cargo fmt --check | PASSED | Fresh current-source gate |
| cargo check --locked -j1 | PASSED | Final explicit current-source check |
| cargo test --locked -j1 | PASSED | 626 passed, 0 failed, 11 ignored |
| cargo build --release --locked -j1 | PASSED | All release binaries; 42 min 08 s |
| Normal/Chibi/default/Alicia/replacement | PASSED | Final Chibi native harness; exact restoration, deferral and finite springs |
| Rest/Ride ↔ drag and taskbar boundaries | PASSED | Real Cloud/Focus harnesses with semantic drag/release |
| Screensaver ↔ Alarm ↔ DesktopPet | PASSED | Real presentation harness, exact restoration and real alarm preemption |
| Study/Work ↔ headpat/stop/completion | PASSED | 16 native checks, book/laptop visual inspection, timer/notice/cleanup |
| Treat ↔ normal ownership | PASSED | Seven props; authored bite/mouth/FX, custom rig and cleanup |
| Manual animation/checkpoint ↔ Cloud/Alarm/drag | PASSED | 434 native contact frames, default/Alicia, repeated Chibi and seated return |
| Hand/Arm reactions and replacement | PASSED | Real all-eight/seven-streak/FX/cooldown/side/priority smoke |
| Native external-window sitting | NOT PERFORMED | No exposed window edge with headroom; taskbar/native and support regressions passed |
| AutoDance ↔ manual, live media | NOT PERFORMED (native) | Ownership/detector regression suite passed; no real music claim in this audit |
| Screenshot PNG transparency | PASSED | Native alpha-zero counts and rendered capture inspection |
| UI/worker memory lifecycle | PASSED for measured cycles | Font/UI/preview cleanup, worker exits, later cycles plateau |
| Idle before/after measurement | PASSED | Sequential 60 s debug runs, bundled model/24 FPS, compiler stopped |
| Release extended idle run | PASSED (180 s) | 90 samples; 444.6 MiB private, 513.1 MiB working set, 2.65% CPU across eight threads; hours-long soak skipped |
| Original workstation/artwork fidelity | PASSED | Original 11.6 MB mesh/textures and logo retained |
| Required DLL/runtime assets in portable ZIP | PASSED | PE-driven signed Microsoft app-local CRT, runtime-file/LFS checks and manifest |
| Extract-and-run without Rust/Cargo runtime dependency | PASSED on this host | Native packaged Shell launch, Windows import table, VCRUNTIME140 loaded from package; clean-machine certification skipped |
| Proprietary code and asset authority | PASSED (owner confirmation) | LICENSE; supplied-asset authorization; separate third-party/CC0/VRM notices |
| CI build/test/package gates | PASSED (configuration); hosted run NOT PERFORMED | Windows workflow added; no push/dispatch requested |
| Physical double-click/tray Show/Hide/Exit | NOT PERFORMED | Shell default-open/normal WM_CLOSE tested; no user's mouse/keyboard control |
| Taskbar/Alt+Tab physical icon appearance | NOT PERFORMED | Native icon resources/window slots verified; primary pet intentionally skips taskbar |
| Clean Windows 10/11 PC; mixed DPI/multiple monitors | NOT PERFORMED | One native development host |
| Disconnected audio device/actual listening | NOT PERFORMED | Source/fault/format regressions; muted feature harnesses |
| Signing/SmartScreen reputation | NOT PERFORMED | No signing certificate supplied |

Final verdict: **CONDITIONAL GO** for this locally validated Windows x64 candidate.
No known unresolved P0/P1 or release-blocking P2. Remaining qualifications:
unsigned binary, clean-machine/hardware/physical QA and live audio cases above.
Artifact: `target/distribution/Nera-0.1.0-windows-x64.zip`, manifest and SHA-256
sidecar. Consult FINAL_AUDIT.md for evidence and earlier failed runs. No hosted
CI run, publication, commit or push was performed by this audit.
