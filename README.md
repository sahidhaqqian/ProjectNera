# Project Nera

Project Nera is a 3D desktop pet for Windows built with Rust and Bevy. It renders a VRM avatar in a transparent native window and combines avatar interaction, VRMA animation, desktop-aware movement, utility windows, alarms, and Win32 integration.

> Application version: **0.1.0** · Current milestone: **Phase 5.9 — Refactor / Cleanup / Robustness complete**

## Overview

Nera lives in a frameless transparent window above the Windows desktop. The visible avatar receives pointer input while transparent areas pass clicks through to applications underneath. Nera can be dragged, react to body-part interactions, sit on taskbars and supported windows, fall onto desktop surfaces, and switch into an attention presentation for alarms.

The application uses Bevy resources, components, messages, and systems for behavior. Native Windows APIs provide global cursor coordinates, HWND movement and tracking, monitor/taskbar information, system tray support, and the radial action menu.

## Current Features

### Desktop pet and interaction

- Transparent, frameless primary window with configurable always-on-top behavior.
- Native click-through outside the avatar's current hit regions.
- DPI-aware click-versus-drag classification with captured pointer state.
- Native desktop-window dragging with semantic drag start, movement, and release messages.
- Animated body-part hit detection for the head, torso, arms, hands, legs, and feet.
- Left-click reactions and right-click action-menu activation.
- Button-free headpat recognition from deliberate back-and-forth cursor strokes.
- Input isolation while Settings or Alarm Settings owns application mouse input.

### Desktop environment and placement

- Native HWND, monitor, DPI, work-area, taskbar, and external-window discovery.
- External support-window tracking across movement, resizing, minimizing, cloaking, restoration, and closure.
- Sitting on supported native-window top edges and taskbar edges using a pose-derived Seat anchor.
- Attachment following, target-relative z-order, temporary suspension, and drag-to-unsnap behavior.
- Desktop-space gravity, falling, landing, and support detection in global physical pixels.
- Standing support on monitor work-area floors, taskbars, and visible native-window top edges.
- Support revalidation when a window moves or disappears, plus tray-based position recovery.
- Multi-monitor and non-zero/negative desktop coordinate handling in native geometry paths.

### Character behavior

- VRM 1.0 avatar loading through `bevy_vrm1`.
- VRMA clip loading with real clip durations and explicit looping/one-shot behavior.
- Prioritized animation state for idle, sitting, dragging/floating, physics, reactions, death, menu, and alarm behavior.
- Sitting entry and seated loop animation tied to pose-first Seat-anchor calibration.
- Falling and landing animation integrated with desktop support physics.
- Head/chest hit reactions, head-stroke reaction, and a three-hit death combo.
- Random idle micro-actions: Look Around, Relax, Sleepy, and Blush.
- Menu-triggered Wave, Thinking, Angry, Sad, and Surprised animations.
- Facial expression crossfades derived from the resolved body-animation state.
- Cursor-driven eye/head gaze with animation-aware temporary ownership.
- VRM spring-bone simulation and configurable simple or character shadows.

### Utilities

- Native Windows radial action menu opened by right-clicking Nera.
- Animation Manager gallery, quick alarms, custom alarm editor, and Settings access from the menu.
- Animation Manager with live per-card VRM previews, search, persistent bookmarks, and physical VRMA import/rename/delete.
- Quick alarm presets for 5, 10, 15, 30, and 45 minutes, 1 hour, and 2 hours, plus Test Now and Cancel.
- Persistent one-time and weekly alarms, with enable/disable, edit, and delete controls for up to 12 entries.
- Alarm attention mode with a reframed upper-body presentation, looping sound, intro/loop animation, dismiss, and five-minute snooze.
- Persistent Settings window with controls for:
  - always on top, Start with Windows, Start Hidden, and remembered position;
  - character size and ground offset;
  - **Aktifkan jalan** checkbox (Yes/No, off by default): short automatic walks
    along the current monitor's desktop floor, using `animasi_basic/Walking.vrma`;
  - mouse, eye, and upper-body gaze behavior;
  - shadow type, opacity, softness, and scene brightness;
  - master volume, mute, and FPS limit;
  - reset defaults, return to the action menu, exit, and application information.
- System tray commands for Show, Hide, Reset Position, Always On Top, and Exit.
- Saved settings, position recovery, deferred persistence during editing, and synchronous save on controlled exit paths.

The radial menu currently shows **Chat**, **Productivity**, and **Games** as disabled placeholders. They are not implemented features.

Walking starts after about six idle seconds once Settings and the radial menu
are closed. Nera pauses between walks and yields to hovering, clicking, dragging,
sitting, falls, selected animations, and alarms. Uncheck **Aktifkan jalan** to
stop; the choice is saved. This first version does not walk along window tops.
The protected `assets/animations/animasi_basic/` folder is included in Git; the
rest of the local animation collection remains ignored.

## Tech Stack

Versions are taken from `Cargo.toml`.

| Technology | Version | Role |
|---|---:|---|
| Rust | Edition 2024 | Application language |
| Bevy | 0.19 | ECS, rendering, windows, UI, audio, input, and application runtime |
| `bevy_vrm1` | 0.9.2 | VRM 1.0 avatars, VRMA playback, LookAt, expressions, and spring bones |
| `windows` | 0.62.2 | Win32 windowing, shell/tray, registry, monitor, DPI, GDI, and DWM integration |
| `raw-window-handle` | 0.6.2 | Native window-handle interoperability on Windows |
| VRM / VRMA | Repository assets | Avatar model and animation formats |

Bevy default features are disabled; `Cargo.toml` enables the specific 3D rendering, winit, UI, picking, settings, audio, MP3, PNG, and JPEG features used by Nera.

## Requirements

- **Windows.** The desktop, tray, utility-window, and platform implementations use Win32 APIs and Windows-only modules.
- A current stable Rust toolchain capable of compiling Rust 2024 edition code.
- The `x86_64-pc-windows-msvc` Rust target used by the checked-in Cargo configuration.
- The repository's `assets/` directory at runtime. It contains the active VRM, VRMA, MP3, and shader assets.

The checked-in `.cargo/config.toml` configures `sccache` as `rustc-wrapper` and points the Windows MSVC target at a user-specific `rust-lld.exe` path. On another machine, install/configure `sccache` and adjust that linker path if it does not match the local Rust toolchain.

No minimum hardware specification is defined by the repository.

## Getting Started

### Clone

The configured Git remote uses SSH:

```powershell
git clone git@github.com:sahidhaqqian/ProjectNera.git
cd ProjectNera
```

### Build

Debug build:

```powershell
cargo build
```

Optimized release build:

```powershell
cargo build --release
```

Keep the `assets/` directory available with packaged builds. During repository development, the application resolves assets from the manifest directory; packaged execution falls back to an `assets` directory beside the executable.

GitHub Actions artifacts include `nera.exe`, `assets/`, and `START-HERE.txt`.
Download the debug or release artifact ZIP and **extract the entire ZIP** into
one folder, then run the extracted `nera.exe`. Keep `assets/` beside it; moving
only the executable causes `Path not found` errors for the model and animations.
The workflow checks that required runtime assets were committed and packaged.

### Run

```powershell
cargo run
```

To run the optimized profile:

```powershell
cargo run --release
```

### Tests

Run the complete automated suite:

```powershell
cargo test
```

For a fast compile-only validation:

```powershell
cargo check
```

Subsystem tests can be filtered with standard Cargo test filters, for example:

```powershell
cargo test animation::tests
cargo test settings::
```

## Project Structure

The tree below shows the important tracked runtime and documentation paths. Large animation libraries are grouped rather than listed asset-by-asset.

```text
ProjectNera/
├── .cargo/
│   └── config.toml
├── assets/
│   ├── animations/
│   │   ├── Falling.vrma
│   │   ├── animasi_basic/                 # 17 active, protected Nera animations
│   │   ├── Clawatar_animations/
│   │   ├── FreeMotionPack1_converted_21_vrma/
│   │   ├── UAL1_converted_43_vrma/
│   │   ├── Unused/                        # Former basic clips, editable stock
│   │   └── VRMA/
│   ├── shaders/
│   │   └── nera_shadow_only.wgsl
│   ├── sound/
│   │   └── alarm.mp3
│   └── vrm/
│       ├── nera_256.vrm
│       ├── nera.vrm
│       └── nera0.0.vrm
├── src/
│   ├── main.rs
│   ├── avatar.rs
│   ├── avatar/
│   │   └── anchors.rs
│   ├── interaction.rs
│   ├── interaction/
│   │   ├── headpat.rs
│   │   └── hit_testing.rs
│   ├── animation.rs
│   ├── animation/
│   │   ├── state.rs
│   │   ├── systems.rs
│   │   └── playback.rs
│   ├── environment.rs
│   ├── environment/
│   │   ├── sitting.rs
│   │   └── taskbar.rs
│   ├── desktop_physics.rs
│   ├── desktop_physics/
│   │   └── geometry.rs
│   ├── expression.rs
│   ├── presentation.rs
│   ├── alarm.rs
│   ├── alarm_settings.rs
│   ├── alarm_dismiss.rs
│   ├── action_menu.rs
│   ├── animation_manager.rs
│   ├── animation_manager/
│   │   ├── preview.rs
│   │   ├── registry.rs
│   │   └── ui.rs
│   ├── settings/
│   │   ├── mod.rs
│   │   ├── model.rs
│   │   ├── runtime.rs
│   │   ├── shadow.rs
│   │   ├── startup.rs
│   │   └── ui.rs
│   ├── tray.rs
│   ├── tray/
│   │   └── native.rs
│   ├── platform/
│   │   ├── mod.rs
│   │   └── windows.rs
│   └── rng.rs
├── ARCHITECTURE.md
├── Cargo.toml
├── Cargo.lock
└── README.md
```

Major ownership boundaries:

- `main.rs` — app bootstrap, resources, plugin registration, and explicit cross-subsystem scheduling.
- `avatar.rs` + `avatar/anchors.rs` — avatar spawning and gaze, then pose-derived Feet/Seat anchors and death visual grounding.
- `interaction.rs` + `interaction/` — pointer capture, click/drag classification, semantic messages, hit testing, and headpat recognition.
- `animation.rs` + `animation/` — request scheduling, animation state and priorities, clip library, and playback.
- `environment.rs` + `environment/` — native desktop and external-window state, taskbar data, and sitting attachments.
- `desktop_physics*` — gravity, falling/landing, standing support, and pure support geometry.
- `presentation.rs` — attention-mode ownership of the main window and camera.
- `alarm*`, `action_menu.rs`, and `animation_manager*` — alarms and Windows utility surfaces.
- `settings/` — persistent model/defaults, runtime application, settings UI, startup integration, and shadows.
- `tray*` and `platform/` — tray-thread bridge and general Win32 integration.

## Architecture

Phase 5.9 separated the main architectural hotspots into subsystem-focused modules. The follow-up consolidation keeps each behavior's closely related code together: pointer messages and capture in `interaction.rs`, desktop-window discovery and tracking in `environment.rs`, avatar spawning and gaze in `avatar.rs`, idle scheduling in `animation/systems.rs`, and clip registration plus playback in `animation/playback.rs`. `main.rs` remains focused on bootstrap and cross-subsystem ordering.

A simplified runtime flow is:

```text
Windows / Bevy input and desktop facts
                 ↓
       Interaction + Environment
                 ↓
       Placement / Physics / Alarm
                 ↓
        Animation state resolution
                 ↓
     Playback + Expression + Avatar
```

This diagram is intentionally simplified. Sitting, presentation, physics, and avatar anchors share carefully ordered Bevy state, and native attention mode temporarily transfers ownership of the main window.

Read [ARCHITECTURE.md](ARCHITECTURE.md) before making cross-subsystem changes. It is the authoritative guide for:

- module and state ownership;
- meaningful dependency directions;
- interaction, sitting, alarm, settings, and tray message flows;
- Bevy startup/update/post-update/last-stage ordering;
- behavior-sensitive areas and architecture guardrails;
- the practical “Where do I change X?” lookup table.

## Current Development Status

Phase 5.9 completed the architecture-focused refactor without intentionally changing application behavior.

A follow-up consolidation reduced the Rust source layout from 39 to 32 files without changing the intended runtime behavior. Avatar spawning now calls `spawn_animation_library()`; the avatar's VRMA paths and load observers live beside playback in `animation/playback.rs`.

| Checkpoint | Status |
|---|---|
| 5.9.1 — `main.rs` decomposition | Complete |
| 5.9.2 — animation architecture | Complete |
| 5.9.3 — settings architecture | Complete |
| 5.9.4 — physics/sitting/window robustness | Complete |
| 5.9.5 — platform/tray/native integration | Complete |
| 5.9.6 — final architectural cleanup | Complete |

Validation recorded at Phase 5.9 completion:

```text
cargo check: PASS
cargo test: 116 passed, 0 failed, 0 ignored
```

The suite covers state transitions and pure decision logic, including animation priority, interaction hit testing, presentation, settings persistence, desktop support geometry, and native helper boundaries. It does not replace manual Windows GUI testing.

## Known Limitations and Notes

- Project Nera currently targets Windows; native desktop functionality is not implemented for other operating systems.
- Automated tests do not fully exercise real HWND focus, click-through, z-order, tray callbacks, Explorer restarts, multi-monitor movement, or mixed-DPI behavior.
- The native application should be smoke-tested on Windows after changes to interaction, sitting, presentation, settings windows, tray, or platform code.
- `environment/sitting.rs`, `platform/windows.rs`, and `settings/ui.rs` remain relatively large because their current responsibilities are cohesive or behavior-sensitive. File size alone is not a reason to split them.
- The gallery lists stock VRMA files and uses independent VRM instances with unique animation target IDs for previews. Clicking a card restarts its preview and plays that file once on the desktop pet through the animation controller, then returns to the current base pose.
- `cargo check` currently passes with non-fatal dead-code warnings for retained helpers and state variants.
- The checked-in Cargo linker configuration may require a local path adjustment, as described under Requirements.

## Development

Before changing behavior across module boundaries:

1. Read [ARCHITECTURE.md](ARCHITECTURE.md).
2. Identify the authoritative resource or message owner.
3. Preserve existing Bevy ordering and run conditions.
4. Add focused regression coverage for changed state transitions or pure decisions.
5. Run `cargo check` and relevant targeted tests before the full suite.
6. Manually exercise affected native Windows behavior when HWND, input, positioning, tray, or utility-window code changes.

In particular, preserve the `Pressed -> Dragging -> Released` lifecycle, sitting/unsnapping handoff, animation priority and completion semantics, attention snapshot restoration, settings compatibility, and native thread boundaries.
