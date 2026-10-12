# Focus Activities V2

Alarms and Focus are separate tabs in the existing Alarm Settings window.
Study uses Praying with a torso-attached CC0 open book in front of the hands and gentle Joy.
Work uses the existing Typing VRMA with the supplied laptop workstation
and a neutral face. Typing was moved from Mixamo-VRMA to protected
animasi_basic/Typing.vrma and uses the same lazy loader/playback controller.
When standing, Work summons the existing Ride Cloud and waits for its calibrated
seat before starting the desk/typing. If already sitting on the taskbar, Work
keeps that seat and shows the desk directly, without summoning Cloud.

FocusSession owns the session timer and selected mode. FocusActivity requests
persistent priority-45 intent from NeraAnimationState; the existing clip registry,
lazy VRMA loader and playback controller own playback. No second animation owner.
Props only exist while the resolved activity owns the body. Torso/head/shoulder rest
geometry sizes Study's book, while evaluated hands position it in the torso frame
through the authored pose, with the open pages facing Nera. Standing rest hips/feet
keep Work's medium station size even when seated; evaluated/rest shoulder
ratio adapts the same size to Chibi. The keyboard surface is aligned to evaluated
typing fingers, with the screen facing Nera and the laptop back facing the viewer.
Typing retains its authored seated
legs/hips rotation; the existing seat-contact layer preserves only the calibrated
hip translation, and the existing Cloud/native seat caches can sample Typing.
Chibi
changes rebuild the prop calibration. Unsupported rigs can keep their loop
without a prop, with a bounded warning.

While Study owns the pose, the existing LookAt target follows the book centre
and existing eye/head tracking looks down at it. Activity gaze does not sample
the cursor or rewrite saved follow preferences. Work keeps Typing's authored gaze;
headpat, Rest Cloud and critical presentations retain their suppression rules.

Headpat, authored touches, dragging and support transitions temporarily pause
the activity clock and remove its prop. Focus resumes after the compatible
base pose returns. Alarms, Screensaver, character loading/replacement and Rest Cloud
end the session safely and release its intent/props; they keep their placement
and presentation ownership. Ride keeps placement ownership during Work. Focus
dismisses only a Ride it introduced on stop/completion; an existing user's seat
is preserved. Completion plays the existing Waving/Happy/Heart
reaction; manual Stop does not award completion. Closing the editor keeps the
session running.

Temporary behavior/audio modifiers never change saved roaming or audio values.
Only last Focus mode, duration choice and custom duration are persisted; no
running timer or prop is saved. Focus never edits real alarm deadlines.

Work bundles the exact original 11,612,308-byte supplied GLB: 267,920 triangles and
three 1024x1024 textures, without compression or resizing. Asset sources, licenses
and measured budgets: assets/props/focus/CREDITS.md.
Native validation: cargo run -- --feature-smoke --focus-activity-smoke.
The isolated profile uses semantic commands, never physical mouse/keyboard
input, and saves screenshots/report under target/focus-v2-native. Completion
advances only Focus time; real alarm clocks remain untouched.

## Changed files in the V2 pass

- `src/focus.rs`, `src/focus/activity.rs`, `src/focus/smoke.rs`: session/activity policy, props and native validation.
- `src/alarm_settings.rs`, `src/alarm_settings/focus_ui.rs`: Alarms/Focus tabs and session controls.
- `src/animation/state.rs`, `src/animation/playback.rs`, `src/animation.rs`: Focus intent, protected Typing playback and replacement cleanup.
- `src/animation/touch/seated.rs`, `src/cloud/pose.rs`, `src/avatar/anchors.rs`: sample authored Typing while retaining existing seat/contact ownership.
- `src/avatar.rs`: Study targets the book through the existing gaze pipeline.
- `src/expression.rs`, `src/expression/basic.rs`: Study/Work faces and Typing preview expression.
- `src/settings/model.rs`, `src/settings/runtime.rs`: preference-only persistence and fixture coverage.
- `src/animation_manager/base.rs`: protect `animasi_basic/Typing.vrma`.
- `src/main.rs`, `src/feature_smoke.rs`: isolated validation entry point.
- `assets/animations/animasi_basic/Typing.vrma`, `assets/props/focus/`, `ARCHITECTURE.md`, this document: bundled assets, credits and ownership documentation.

## Validation on Windows, 2026-10-12

- `cargo fmt` and `cargo check`: passed.
- Full `cargo test` passed before the final gaze/ride-completion adjustments (584 passed, 11 ignored).
- Final application suite `cargo test --bin nera`: **585 passed, 0 failed, 11 ignored**. Existing regression assertions remain intact; base counts were extended for Typing.
- Native default-Nera run: **13 passed, 0 failed**, using semantic UI/interaction commands and GPU screenshots without moving the OS pointer or sending keyboard input. Includes Study book/gaze/countdown, headpat/resume, Study/Work stop, standing Work via Ride, taskbar Work without Cloud, Chibi recalibration, drag pause/resume, real alarm priority, completion, Screensaver priority, repeated sessions/preferences, and completion on a pre-existing Ride.
- Native report and screenshots: `target/focus-v2-native/report.json`, `study-book.png`, `work-laptop.png`, `work-taskbar.png`, `work-chibi.png`, and Focus tab screenshots.
- Not performed: physical pointer/keyboard gestures, waiting 25/50 wall-clock minutes, listening to muted native audio, and custom-VRM visual validation. Unit tests cover countdown/completion, degenerate rig geometry and safe restoration.
- Validated executable copied to `target/debug/nera.exe`; the user's ordinary application was not started by the validation script.
