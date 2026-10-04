# Chibi Mode V1

The top-level radial **Chibi** item toggles the `AvatarBodyMode` resource.
`NeraSettings.body_mode` is only its persisted encoding; the native menu caches
label text, and rig caches record which geometry has actually been applied.
The same loaded VRM, skin, expressions and VRMA library are used in both modes.

## Proportion architecture

`avatar::body_mode` resolves the rendered skeleton from `HipsBoneEntity`.
It validates humanoid transforms, a descendant mapped head and foot, and spring
geometry. Source animation skeletons are excluded. It preserves imported local
transforms and restores only its own previous pose overlay before evaluation.
After Bevy animation, walking/seated corrections and the cloud pose layer, it
composes uniform hips scale `0.62` and relative head scale `1.9`, before VRM
constraints, propagation, gaze, expressions and spring simulation. Limbs and
torso inherit the hips scale; eyes, hair and head children inherit the head
scale. Authored rotations, animation targets and translation deltas remain intact.

The visual tuning pass additionally shortens the imported upper-leg rest
segment to `0.85` by a reversible additive knee-pivot offset. Authored animation
translation deltas, rotations, shin/foot scales and torso scaling remain intact.
Neutral foot correction includes the resulting rest geometry. See
[Chibi proportion tuning](C:/Users/sahid/Documents/ai/CHIBI_PROPORTION_TUNING.md)
for matched neutral before/after screenshots and the updated validation report.

A neutral root-height correction comes from the original rest-pose hips/feet,
so authored animation, dragging and falling retain the compact body's support
baseline. This is separate from the existing transient pose-grounding fix.
Both layers remove their own correction in a defined order each frame.
While Cloud owns placement this standing correction yields to its evaluated
pelvis contact, preventing lying custom avatars from being pushed out of the
transparent canvas. Alarm predicts the standing correction after Cloud dismissal.

Animation cannot overwrite the completed layer: its writes run first. Scales
compose with that frame's evaluated pose, so animation scale channels are also
supported. Normal restores exact original/evaluated scales without division or
cumulative multiplication. Character replacement captures the new rig while
retaining the selected semantic mode.

## Grounding and state ownership

The existing perspective contact solver aligns evaluated feet with the same
cached support plane. Sole allowance scales with the compact body; the user's
ground offset remains their setting. Footprint refresh retains support identity
and vertical contact, including walking. Standing refresh remains pending while
seated/airborne. Window/taskbar seat realignment reuses existing placement math.

Rest/Ride keep `CloudRuntime` as placement owner. Raw cloud pose caches are
sampled before proportions; hips-derived contact offsets and dynamic skinned
bounds follow the final body. Rest's existing gaze suppression stays active.
Head/body hit areas use the effective applied proportions. Alarm uses canonical
rest landmarks with the same body/root mapping and enlarged-head allowance.
Mode changes during dragging or Attention are queued semantically and applied
after release/exit; they never replace those states' ownership.

Transitions reuse `PetFx::CloudPuff` and `PetSfx::Happy`. Settings use the existing
deferred save mechanism. Saved Chibi applies only after initialized humanoid
data is available.

## Spring and custom VRM safety

`bevy_vrm1` 0.9.2 exposes chains but keeps solver state private. A small adapter
uses its registered reflection schema to reset inertia and scaled chain lengths
after final propagation. Normal restores the original solver baseline. Tests
check the actual dependency's schema. An incompatible schema or invalid spring
geometry disables Chibi safely and logs the reason instead of duplicating the
spring solver. Dependency upgrades must keep this contract test passing.

V1 requires mapped hips, one descendant head and at least one descendant foot,
finite nondegenerate humanoid transforms and supported spring chains. Unusual
skin weights, accessories outside the humanoid hierarchy and extreme existing
proportions may produce imperfect deformation. V1 supplies one fixed proportion
style rather than character-specific adjustments.

Head-attached accessories and spring chains inherit the larger head. Alicia's
very long ponytails consequently extend below the feet and can meet the fixed
canvas boundary. This is a known visual limitation of V1; spring transforms
remain finite and their original solver state is restored when returning Normal.

## Files and regression coverage

- `src/avatar/body_mode.rs`: semantic mode, settings encoding, hierarchy
  validation, reversible pose/root layer and transition feedback.
- `src/avatar/body_mode/springs.rs`: dependency reflection adapter, scaled
  spring geometry and exact solver restoration.
- `src/avatar.rs`, `src/main.rs`: module/plugin wiring.
- `src/avatar/anchors.rs`, `src/environment/sitting.rs`: resized standing/seat
  anchors with preserved support ownership.
- `src/cloud.rs`, `src/cloud/tests.rs`: pose ordering and Rest/Ride regressions.
- `src/presentation.rs`: canonical Chibi landmarks and stable Alarm framing.
- `src/interaction.rs`: effective head/body hit areas.
- `src/action_menu.rs`: direct top-level Chibi command and resource-derived label.
- `src/settings/model.rs`, `src/settings/runtime.rs`: persisted encoding and
  legacy settings compatibility.
- `src/feature_smoke.rs`, `src/avatar/body_mode/smoke.rs`,
  `examples/chibi_validation.rs`: opt-in native regression executable.
- `examples/chibi_support_fixture.rs`: optional bounded, separate-process real
  Windows support surface for the native window-sitting test.

Unit/regression coverage includes 100 repeated toggles, exact non-unit imported
local/root restoration, animation pose/scale writes, character replacement,
saved startup readiness, malformed/missing bones, dragging deferral, support
plane/contact retention, all taskbar-edge and window seat geometry, Rest/Ride
ownership, Alarm landmarks and the actual pinned spring reflection contract.

## Repeatable Windows validation

Use a separate executable so the user's running pet does not need replacement:

```powershell
cargo run --example chibi_validation -- --feature-smoke --chibi-smoke --chibi-smoke-model=<absolute-imported-vrm-path>
```

Run with isolated child-process `APPDATA` to keep the user's character catalog
unchanged. The smoke settings profile also has its own process-specific ID.
The harness uses synthetic semantic inputs with real Windows placement, VRMA
playback, humanoid transforms and GPU screenshots. Missing desktop surfaces
are reported as explicit skips. `--chibi-smoke-interactive` retains the successful
test window for subsequent physical UI checks. Output is in
`%TEMP%\nera-chibi-smoke`, including `report.json`.

If every normal application window is maximized, start the optional external
support fixture in a separate terminal before the regression:

```powershell
cargo build --examples
.\target\debug\examples\chibi_support_fixture.exe --lifetime-secs=300
```

The fixture exposes a real captioned window edge without activating user apps
and closes automatically. The regression must report `skips: []` to count the
window-sitting check as covered.

## Validation result

`cargo fmt`, `cargo check`, `cargo test` and `cargo build --examples` passed.
The final suite has **427 passed, 0 failed, 6 ignored**. Existing dead-code
warnings remain. `git diff --check` passed.

Windows GPU/native validation used the bundled Nera VRM and the imported
**Alicia Solid** VRM (`model-07665e5f50f3b8ac.vrm`) on Intel Iris Xe/Vulkan.
The same production entrypoint ran in the separate validation executable with
isolated settings/catalog. Both models completed every state below with no
skipped support surface:

| State | Native validation |
| --- | --- |
| Idle, Normal → Chibi → Normal | Exact scales restored; support plane retained |
| Walking | Native movement occurred; mode changes preserved ground contact |
| Taskbar sitting / normal window sitting | Same support identity and seat point retained |
| Drag / release | Mode geometry deferred until release; grounding recovered |
| VRMA changes | Relax, Death01 and imported Sword_Idle used the same rig |
| Headpat | Existing animation/expression continued in both body modes |
| Rest Cloud | Seat retained; gaze/headpat suppression retained; face stayed inside canvas |
| Ride Cloud | Seat retained; gaze/headpat remained enabled |
| Alarm Presentation | Canonical framing retained; mode change deferred until exit |
| Character replacement while Chibi | New imported rig inherited selection with its own original proportions |
| Springs | 147 default / 66 custom live joints stayed finite across transitions |

The native report records **57 screenshots**, no skips, and successful exit.
The final run completed in 69.37 seconds; all 57 image files were verified fresh,
including the final custom settled-springs capture before native window teardown.
Its report is preserved at `target/chibi-validation-report.json`.
Screenshots were inspected for silhouette, expressions, cloud contact/framing,
hair/clothing and Alarm framing. The original custom Rest clipping was found
visually and corrected by yielding the standing root correction to Cloud.

These checks use synthetic semantic inputs and real native placement/rendering.
They are not physical pointer-driven menu/drag tests. The user elected native
validation only after the Windows control permission timed out. The direct
radial command/label also has regression coverage; physical pointer gestures
remain unverified. The only remaining observed visual limitation is the
unusually long head-attached Alicia ponytail described above.
