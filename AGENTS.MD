# Project Nera Agent Instructions

## Purpose

This file contains stable instructions for coding agents working on Project Nera.
Keep it short, practical, and resistant to becoming stale.

For architecture/ownership questions, use `ARCHITECTURE.md` as the main reference.
For feature-specific details, use the focused docs already in the repository.

Do not treat this file as a roadmap or changelog.

---

## Project

Project Nera is a Windows 3D desktop pet built with:

- Rust 2024
- Bevy 0.19
- `bevy_vrm1` 0.9.2
- Win32/native Windows integration
- VRM avatars and VRMA animations

The repository is mature and already contains substantial regression coverage.
Existing subsystem boundaries and state ownership are intentional unless code or tests prove otherwise.

---

## Agent / Token Budget

Use a **single-agent workflow by default**.

For ordinary implementation, bug fixing, polish, or focused feature work:

- do not spawn multiple subagents;
- use 0 subagents by default;
- use at most 1 subagent only for a genuinely independent investigation;
- never ask several agents to inspect the same subsystem;
- do not run broad repository audits unless explicitly requested;
- inspect the smallest relevant set of files first;
- avoid repeatedly rediscovering architecture already documented here or in `ARCHITECTURE.md`.

Prefer this workflow:

1. inspect the relevant implementation;
2. identify the current owner/state path;
3. make a short implementation plan;
4. patch directly;
5. run targeted tests;
6. run the full required validation once near completion.

Do not repeatedly run the entire test suite after every small edit unless failures require it.

---

## Working Style

- Prefer focused patches over broad refactors.
- Fix root causes instead of visual/hardcoded workarounds.
- Preserve existing behavior unless the task explicitly changes it.
- Reuse existing resources, messages, state owners, and helper infrastructure.
- Do not create duplicate state ownership.
- Do not rewrite a working subsystem merely because another design is possible.
- Do not perform unrelated cleanup while implementing a feature or bug fix.
- Do not merge/split files only to optimize file count.
- Small files are acceptable when they have a real, independent responsibility.
- When V1 acceptance criteria pass, stop. Do not silently expand scope.

Before an architectural change, read the relevant section of `ARCHITECTURE.md`.
For small local changes, do not consume context by rereading the entire architecture document unnecessarily.

---

## Core Ownership Rules

One concern should have one authoritative owner.

### Desktop/native environment

- Native desktop/taskbar/window facts: `DesktopEnvironment`.
- Do not duplicate taskbar/work-area/window geometry state elsewhere.

### Physics and placement

- Free-standing gravity, throw, and support: `DesktopPhysicsState` / desktop physics subsystem.
- Sitting/window attachment: existing environment/sitting placement path.
- Rest/Ride placement: `CloudRuntime` while Cloud owns the avatar.
- Do not solve placement bugs with arbitrary pixel offsets when existing geometry/support data can provide the answer.

### Interaction

- Pointer capture and click/drag classification: `DesktopPetInteraction`.
- Headpat, touch, drag, and release should remain semantic interaction paths.
- Do not turn raw pointer handlers into animation/audio/FX owners.

### Animation

- Animation intent/state: existing animation state owner.
- Runtime VRMA loading/playback: existing playback/clip registry.
- Touch reactions request animation through the current animation path.
- Do not introduce a competing direct `AnimationPlayer` ownership path.
- Protected runtime animations in `assets/animations/animasi_basic/` must remain protected from destructive rename/delete operations.

### Facial expressions

- Use the existing expression system.
- Do not directly fight expression ownership from unrelated systems every frame.

### Chibi/body mode

- Semantic Normal/Chibi selection belongs to `AvatarBodyMode`.
- Applied proportion overlay belongs to the existing body-mode implementation.
- Chibi must remain reversible and use the same VRM/VRMA pipeline.
- Do not add Chibi-specific avatar files or a second body-mode implementation.

### Cloud

- Rest and Ride retain placement ownership while active.
- Rest may suppress gaze/interaction according to existing behavior.
- Do not reintroduce gravity/support ownership underneath active Cloud placement.

### Alarm

- Alarm runtime owns scheduling and real ringing/playback lifecycle.
- Alarm Settings owns editor UI and editor-only preview state.
- Closing the editor must not stop a real active alarm.
- Presentation ownership must be released cleanly after alarm dismissal.

### Character management

- Live replacement uses the existing character pipeline.
- Companion/multi-character lifecycle uses the existing separate-process model.
- Do not bypass catalog/roster/release ownership with ad hoc process spawning.

### Feedback

- Semantic audio goes through `PetSfx` / the existing audio path.
- Semantic visual feedback goes through `PetFx`.
- Do not create feature-specific audio or particle stacks when the semantic feedback pipeline can represent the behavior.

### Autonomous behavior

- Low-priority autonomous choices belong to `PetBehavior`.
- Destination locomotion belongs to the walking runtime.
- Autonomous behavior must yield to higher-priority user/presentation states.

---

## State Priority and Transitions

Pay special attention to transitions between:

- Normal / idle
- Walking
- Sitting / supported
- Dragging
- Rest Cloud
- Ride Cloud
- Alarm Presentation / Attention
- Animation preview or authored reaction
- Character replacement/loading

A temporary mode must release only the state it owns and restore the correct next owner on exit.

Never fix transition bugs by globally resetting unrelated resources.

When adding a new stateful feature, explicitly answer:

1. who owns it;
2. what it may interrupt;
3. what may interrupt it;
4. how it exits;
5. what state must be restored afterward.

---

## VRM / VRMA Safety

- Custom VRMs must fail gracefully when humanoid data is unusual or incomplete.
- Do not assume imported avatars exactly match the bundled Nera rig.
- Prefer humanoid/rest-pose-derived geometry over hardcoded bone axes or model-specific offsets.
- Keep authored animation transforms intact unless the feature explicitly owns an overlay.
- Avoid cumulative transform multiplication; reversible overlays must restore their previous contribution before reevaluation.
- Missing/invalid assets must not panic or create log spam loops.

---

## Windows / Native Rules

Preserve:

- transparent desktop-pet window behavior;
- click-through behavior;
- always-on-top behavior;
- taskbar/work-area awareness;
- supported-window placement;
- multi-monitor behavior where currently supported;
- native tray/radial-menu lifecycle.

Native Windows behavior cannot always be proven by unit tests.
Do not claim physical/native validation unless it was actually performed.

---

## Process Boundaries

Keep intentional process isolation intact:

- main Nera desktop-pet process;
- Animation Manager worker process;
- disposable preview-generation worker where used;
- companion character processes.

Do not fold renderer-heavy worker functionality back into the main process merely to simplify code.
Memory reclamation through process lifetime is an intentional design choice.

---

## Scope Control

Unless explicitly requested, do not introduce:

- affection/relationship levels;
- hunger/inventory/economy systems;
- quests/achievements;
- AI chat/personality platforms;
- executable/plugin SDK architecture;
- large mod ecosystems;
- unrelated UI redesigns;
- character-specific hardcoded behavior tables.

Prefer a small reusable foundation over an oversized V1.

---

## File / Refactor Guidance

Do not use file count as a refactor target.

Only merge files when responsibilities are genuinely inseparable and the merge improves cohesion.
Only split files when a module has independently meaningful responsibilities or a clear boundary.

Before structural changes, evaluate:

- responsibility/cohesion;
- ownership boundary;
- dependency direction;
- test isolation;
- reuse by other subsystems;
- whether the change reduces or merely relocates complexity.

Avoid cosmetic derefactoring.

---

## Validation

For normal Rust changes, run at minimum near completion:

```powershell
cargo fmt
cargo check
cargo test
```

Use targeted tests first when appropriate.
Build examples when the changed feature relies on validation examples/harnesses.

For native/visual behavior, run the relevant existing smoke/validation harness when available and report exactly what it did and did not verify.

Do not weaken/delete existing regression tests merely to make a patch pass.

---

## Completion Report

At completion, report concisely:

1. root cause or implementation approach;
2. files changed;
3. ownership/state changes, if any;
4. tests/commands run and exact result;
5. native/manual validation actually performed;
6. remaining limitations or skips.

Do not generate large new audit/report documents unless requested or already part of the feature's established workflow.

---

## Documentation

Keep `ARCHITECTURE.md` focused on stable architecture, ownership, process boundaries, and data flow.
Keep feature-specific implementation details in focused docs.
Do not add temporary roadmap status, benchmark snapshots, or short-lived tuning values to this file unless they are part of a stable contract.

When code changes make `ARCHITECTURE.md` materially inaccurate, update the relevant section as part of the same task.
