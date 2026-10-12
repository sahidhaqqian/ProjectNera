# Treat & Focus V1

Focus is a section of Alarm Settings. Study/Work can run for 25/50 minutes,
custom 1–480 minutes, or Endless. The session keeps its original preset and
duration even if the next session's controls are edited. Start replaces only
Focus; Stop clears only Focus. Closing Alarm Settings does not stop a session.

`FocusSession` owns the real-time session timer. It never changes alarm
deadlines, ringing, or saved user preferences. `PetBehavior` suspends autonomous
roaming/living routines/AutoDance during Focus; `PetSfx` reduces ordinary pet
feedback to 25% of its usual gain. Headpat retains its normal gain and normal
user interactions remain available. Stop/completion removes these modifiers.
Completion requests the existing Waving clip through `NeraAnimationState`,
Happy expression and Heart/Happy feedback. Celebration waits for safe desktop
idle/sitting if higher-priority activity is still running.

Cloud's submenu contains Treat alongside Rest/Ride/Dismiss. Treat is not a
Cloud mode. `TreatRuntime` owns only pending/active interaction and prop
lifetime. The existing animation state and clip registry own Drinking playback;
the expression system applies gentle Joy with natural blink during the clip.
The same expression owner eases a standard VRM `aa`/`a` mouth vowel open during
the bite approach/contact and closes it after consumption. Named Nera morphs
use `Fcl_MTH_A` while reducing the conflicting closed smile. Manual face previews
retain priority; interruption/end releases the mouth along with Treat.
The corrected supplied
8.83-second `Drinking (1).vrma` is bundled under the stable `Drinking.vrma`
filename and uses the left hand. The user approved it for food as well because
the stated Eating clip was absent. It is protected as a base animation.

Seven locally bundled, small CC0 Kenney props use one shared texture. Source,
license, filenames and measured sizes are in `assets/props/treats/CREDITS.md`.
Selection avoids immediate repeats among ready models. Scene loading is bounded
to ten seconds and accepted interactions have a three-second cooldown.

Props are children of the evaluated live humanoid leftHand. Grip axes and size
come from hand/finger rest geometry, not character-specific offsets. Each
asset's evaluated bounds center and half-extents normalize its appearance.
The corrected clip grips from the side: the knuckle span supplies item-up,
and actual model bounds plus palm-derived clearance keep the item outside the
hand. Parent
transforms naturally carry animation, Normal/Chibi scale and custom rig scale.
Missing/degenerate hand/finger data cancels safely with one warning. Food
disappears at the corrected clip's completed sip/bite pose at the mouth
(4.2 seconds), followed by
2–5 existing Heart pulses spaced 350 ms apart. Their count and left/right order
are randomized, with both sides represented, and Happy sound dispatches once.
The full authored animation, including hand recovery, continues afterward.
An interruption before the bite cleans up without affection feedback; an
interruption afterward cancels the remaining pulses with the Treat runtime.

Dragging, loading/replacement, Rest Cloud and attention presentations block or
cancel Treat. Other accepted user reactions can replace it. Cleanup releases
only Treat and its prop. Ride Cloud and native sitting retain existing Cloud
or seated-pose support ownership; no grounding or Chibi overlay was changed.

Native validation: `cargo run -- --feature-smoke --treat-focus-smoke` uses an
isolated settings profile, real VRM/VRMA/glTF rendering and semantic commands.
It never controls the physical mouse. Diagnostics/screenshots are saved under
`target/treat-focus-native/`; `report.json` is written only on successful
completion and states skipped checks. Finite Focus completion advances only
its session timer; real alarm deadlines retain real time. The custom-model
fixture is the existing Alicia Solid import.

The mouth follow-up passed `cargo fmt`, `cargo check`, and the full Rust suite
(590 passed, 0 failed, 11 ignored). Native Treat validation completed 19 checks,
including standard-vowel/rendered-mouth opening, release after completion or
cancellation, Normal/Chibi Nera and Alicia Solid, existing bite/Heart timing,
all seven prop models, replacement cleanup, and Rest/Ride/drag priorities.
It used semantic commands and GPU screenshots without OS mouse/keyboard control.
Native audio was muted; no listening test was performed.
