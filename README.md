# Project Nera

Project Nera 0.1.0 is a native Windows 3D desktop companion. A transparent VRM
avatar lives above the desktop, reacts to touch, sits on supported surfaces, and
uses local VRMA animations. This is proprietary software; see LICENSE.
Feature freeze is in effect. Release evidence is in FINAL_AUDIT.md and
RELEASE_CHECKLIST.md.

## Install and run

1. Extract the complete Windows x64 ZIP to a writable folder.
2. Keep `nera.exe` and `assets` together. Double-click `nera.exe`.
3. Optionally run `Create-Desktop-Shortcut.cmd` to create a shortcut using the
   official embedded Nera icon.

Rust, Cargo and development tools are not required to run the package. The
application needs Windows 10/11 x64, a compatible GPU/driver and its native Windows
graphics/audio services. The binary is unsigned; Windows reputation warnings may
appear. Do not run the executable from inside the ZIP.

## Controls and activities

- Drag Nera to move her. Transparent areas pass clicks through to other apps.
  Deliberate head strokes trigger headpat; body touches trigger authored or
  procedural reactions according to region. Hands and arms use separate VRMA pools.
- Right-click Nera for the radial menu: Settings, Animation Manager, Alarm,
  Screensaver, Chibi and Cloud. Cloud contains Rest, Ride, Treat and Dismiss.
  Games remains unavailable; no online AI/chat service is included.
- Chibi changes the same loaded rig reversibly and retains existing animations.
- Rest sleeps on a cloud. Ride moves with dragging and retains Cloud placement.
  Treat attaches a lightweight random prop and plays the supplied drinking
  animation, then cleans up the prop and shows heart feedback.
- Screensaver frames the upper body, faces the viewer and alternates calm
  animations. Headpat remains available; a body click exits. Scroll does not exit.
- Alarm Settings has separate **Alarms** and **Focus** tabs. Real alarms and quick
  timers are independent of Study/Work sessions. Focus supports 25/50 minutes,
  custom duration and Endless. Study reads a book; Work types at a workstation,
  using taskbar sitting or Ride Cloud as appropriate. Stop releases props/state;
  timed completion gives a reaction and Windows notification.
- Settings controls scale, rendering/FPS, gaze, sound, roaming, physics, startup
  and characters. Optional Audio Reactive Dance combines Windows media sessions
  and loopback audio evidence; it plays only the user's configured Dance Pool.
- Animation Manager handles imports, bookmarks, previews, expressions and the
  Dance tab. Protected Animasi Dasar clips cannot be renamed/deleted. Companion
  characters and the manager run in separate processes for memory isolation.
- The tray offers Show, Hide, Reset Position, Screenshot and Exit. Screenshot
  exports a transparent PNG. If Nera is hidden/off-screen, use the tray to recover.

Imported models/animations/audio stay local. Audio Reactive Dance is opt-in:
loopback analysis stays on-device and does not upload or save audio recordings.
Bundled runtime assets do not require internet access. Imported assets require
their own appropriate permissions.

## Settings, recovery and diagnostics

Preferences/catalogues use the existing per-user Windows settings folders.
Running Focus sessions and companion lineups are transient. Missing/corrupt
settings load defaults with an error log; active alarms require Nera to remain
running. Windows notifications depend on system notification/Do Not Disturb rules.

Logs are under `%LOCALAPPDATA%/ProjectNera/logs/`: one file per process, bounded
to 2 MiB, with old inactive files pruned. Errors and ordinary Rust panics are
recorded even when release has no console. Logs can contain local asset paths
and opted-in media metadata; inspect them before sharing. A missing packaged
default avatar/idle/shader produces an actionable startup dialog. Restore the
complete assets folder if that occurs.

Quit via the tray before replacing a package. Keep a backup of imported content
and settings. Disabling Start with Windows in Settings removes the startup entry;
deleting the extracted application folder removes the portable application.
User settings are retained separately.
After moving the folder, toggle Start with Windows OFF and ON to update its path.
The public ZIP contains protected runtime clips, not personal animation
collections. When upgrading to another folder, keep/copy your private animation
folders under `assets/animations` or re-import their VRMAs in Animation Manager.
Bookmark/Dance Pool metadata alone does not contain the animation files. Imported
character files remain in the per-user character catalogue.

## Build and validate (authorized developers)

Use Windows x64 with Rust 1.98.0, the MSVC C++ toolchain and Windows SDK. No
machine-specific linker path or mandatory sccache wrapper is committed.
Optional local compiler caching may be configured outside the repository.
Release uses opt-level 3 and Thin LTO to keep build memory practical.

```powershell
cargo fmt --check
cargo check --locked
cargo test --locked
cargo build --release --locked
cargo build --examples --locked
cargo metadata --locked --format-version 1 | Set-Content -Encoding utf8 target/metadata.json
python tools/generate_dependency_notices.py target/metadata.json
./tools/package_windows.ps1 -OutputRoot target/distribution
```

Pillow is required only for `tools/make_icon.py`, which reproduces the ICO from
the supplied PNG. Release `nera.exe` uses the Windows GUI subsystem; debug keeps
terminal logging. CI runs the format/check/test/release gates, checks the PE
subsystem and uses the same package script. A SHA-256 sidecar and manifest record
the built package's files, toolchain, source revision and dirty status.

Native smoke harnesses are opt-in debug tools. For example:
`target/debug/nera.exe --feature-smoke --chibi-smoke`.
They inject semantic events, use isolated settings and never move the physical
mouse. Automated native results do not imply physical UI clicks or clean-machine
certification. Consult the release checklist for actual performed/skipped tests.

## Architecture and credits

ARCHITECTURE.md describes state ownership, schedules and process boundaries;
AGENTS.md contains repository working rules. Feature docs hold detailed behavior.
Third-party dependency notices and asset credits ship with the package.
The workstation mesh/textures and supplied artwork retain their original quality.
No separate Chibi avatar or animation set is bundled.
