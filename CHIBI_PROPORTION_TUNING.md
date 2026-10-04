# Chibi V1 visual proportion tuning

This pass adds one conservative femur-length refinement inside the existing
reversible proportion overlay. The semantic mode, scheduling, persistence,
support ownership, Cloud, spring reset and Alarm systems remain as V1.

## Final values

| Proportion | Before | After | Reason |
| --- | --- | --- | --- |
| Hips/body uniform scale | 0.62 | **0.62** | Current overall size and torso/dress proportions already worked. |
| Relative head scale | 1.9 | **1.9** | Shorter thighs increase head dominance enough; matched views did not justify enlarging the head. |
| Upper-leg rest segment | 1.0 of V1 Chibi | **0.85** | Subtle shortening on both Nera and Alicia, without visible knee or skirt defects. |
| Lower-leg/foot scales | Existing V1 scales | **Unchanged** | Preserve shin length, shoe size and bending. |

No other artistic proportion value changed. There is no further whole-avatar
shrink, character preset, editor or second body-mode owner.

## How the leg refinement works

Both loaded avatars map lowerLeg directly beneath upperLeg. Nera's imported
knee vector points along local +Y; Alicia's points approximately -Y with small
X/Z components. Nera also attaches skirt spring branches to its thighs.
Scaling the upper-leg bone would affect those dress branches and propagate
scale through a bending knee.

Instead the existing post-pose layer shortens the imported thigh contribution
by moving the knee pivot:

```rust
knee.translation = raw_pose.translation + imported_rest_translation * (0.85 - 1.0);
```

This is an upper-leg segment adjustment: the translated node is its mapped
lower-leg endpoint. Upper-leg, lower-leg and foot scales/rotations stay intact.
Authored VRMA translation changes relative to the imported rest position remain
exactly the same. The existing PreUpdate restoration removes the precise saved
raw knee translation before the next animation evaluation.

Both leg mappings must be unique, direct and finite. Unsupported helper chains
skip this optional refinement on both sides, log once, and retain ordinary V1
Chibi. No character names or axis assumptions drive the adjustment.

Neutral foot correction now measures the lowest rest foot after these knee
offsets through the actual imported parent matrices, then applies the existing
body scale. Existing support/contact solving, sole allowance, Cloud root yield
and Alarm's standing prediction consume that result. No Y offset was tuned by
hand. Knee-parented coat-skirt attachments move with the knee, while their
scale/length and upper-leg sibling branches remain intact.

## Matched neutral screenshots

Both versions use the same actual Idle frame at **0.5 seconds**, paused before
pose evaluation, front-facing root, gaze disabled and identical camera and
projection. Normal, Chibi and restored Normal were captured for each avatar.
The attachment has different poses, so it was used for visual direction rather
than as a numerical height measurement.

| Avatar | Before | After |
| --- | --- | --- |
| Nera | [Neutral before](C:/Users/sahid/Documents/ai/target/chibi-tuning/before/default-chibi.png) | [Neutral after](C:/Users/sahid/Documents/ai/target/chibi-tuning/after/default-chibi.png) |
| Alicia Solid | [Neutral before](C:/Users/sahid/Documents/ai/target/chibi-tuning/before/custom-chibi.png) | [Neutral after](C:/Users/sahid/Documents/ai/target/chibi-tuning/after/custom-chibi.png) |

| Measurement | Nera before → after | Alicia before → after |
| --- | --- | --- |
| Projected hips-to-feet span | 113.39 → 105.16 px | 131.87 → 122.84 px |
| Projected knees-to-feet span | 53.14 → 53.06 px | 58.79 → 58.68 px |
| Rendered silhouette height | 279 → 271 px (-2.9%) | 306 → 297 px (-2.9%) |
| Rendered bottom pixel | 896 → 896 | 894 → 894 |
| Head eye-width proxy | 9.30315 → 9.30326 px | 16.56171 → 16.56192 px |

All measured local scales and camera snapshots match before/after. Evaluated
foot Y changes by at most 0.13 px; the tiny projected shin differences come from
perspective/depth. Geometry tests verify unchanged actual shin and toe lengths.
Nera's dress-to-shoe gap becomes subtly shorter; Alicia's exposed legs read
more compact. Dress hems, knees and shoes remain clean. A smaller head change
or stronger thigh compression was unnecessary.

Metrics: [before](C:/Users/sahid/Documents/ai/target/chibi-tuning/before/metrics.json),
[after](C:/Users/sahid/Documents/ai/target/chibi-tuning/after/metrics.json).
Run this opt-in path through the existing harness with
`--feature-smoke --chibi-smoke --chibi-proportion-compare --chibi-smoke-model=<path>`.
The unchanged baseline executable is preserved at
`target/chibi-tuning/before-chibi-validation.exe` for this local comparison.

## Regression and native validation

All existing regression tests and native-matrix guards were retained.
Six new hierarchy-based tests cover segment length, bent knees, 100 exact
restorations, animation translation/rotation/scale writes, opposite imported
axes, paired helper-chain fallback, character replacement and rest-foot height.

`cargo fmt`, `cargo check`, `cargo test`, and `cargo build --examples` passed:
**433 passed, 0 failed, 6 existing ignored**.

The existing Windows GPU/native matrix passed on bundled Nera and imported
Alicia Solid: **57 fresh screenshots, zero skips, successful exit**, in 69.13 s.
It covers idle, walking, taskbar/window sitting, drag/release, Headpat, Relax,
Death01, Sword_Idle, Rest/Ride Cloud, Alarm, and replacement while Chibi.
Neutral comparisons additionally assert exact restoration of every humanoid
local transform on both loaded avatars.

Visual review of articulated poses found no knee collapse, shoe distortion or
new skirt deformation. Both Cloud faces remain inside the canvas. Alicia's
long ponytails can still extend below the feet or approach the canvas boundary
in some poses, as documented in V1. They settle without stale/exploding geometry.
These are semantic-input tests with real Windows placement and rendering;
physical pointer tests were not added in this tuning pass.

[Native report](C:/Users/sahid/Documents/ai/target/chibi-tuning/native/report.json).

## Files touched by this tuning pass

- `src/avatar/body_mode.rs`: optional paired knee overlay and derived neutral
  foot correction inside the existing ownership/restoration path.
- `src/avatar/body_mode/leg_tests.rs`: six additional regression tests.
- `src/avatar/body_mode/smoke.rs`: opt-in same-frame neutral comparison only;
  the existing full native matrix remains intact.
- `CHIBI_MODE_V1.md`, `CHIBI_PROPORTION_TUNING.md`: proportion/report documentation.

No Cloud, spring adapter, support, Alarm, menu or settings implementation was
rewritten by this tuning pass.
