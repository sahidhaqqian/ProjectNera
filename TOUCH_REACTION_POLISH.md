# Touch Reaction Polish

Hand and Arm clicks now select authored VRMA reactions through the existing animation owner. Head, Torso, Leg, Foot, drag, headpat, support, cloud, and presentation routing retain their existing responsibilities. Chibi proportions and its authoritative body-mode resource are unchanged.

## Runtime ownership and admission

The existing hit-testing pipeline emits `NeraClicked` with one semantic `BodyPart`. `sync_interaction_animation_actions` routes Hand/Arm events to `TouchReactionRuntime`, which owns selection history, the accepted Hand streak, and cooldowns. A usable loaded clip and an immediate admission from `NeraAnimationState` are required before committing any count, random-selection history, cooldown, or `TouchReactionAccepted` message.

`TemporaryAction::TouchReaction(TouchReactionClip)` uses the existing one-shot state, VRMA slots, playback graph, and reaction blend. Animation playback remains the pose owner; there is no separate Hand/Arm pose player. Procedural Hand/Arm offsets are removed from their reaction path; existing Leg/Foot reactions remain procedural. A later accepted touch can replace a gesture after its cooldown, even if that authored clip has not finished. Otherwise its loaded duration determines completion and return to the base pose.

A higher-priority owner, support transition, dragging, loading/replacement, or Alarm Presentation prevents a touch from being queued underneath it. Missing/zero-duration clips do not consume a streak step. Rest Cloud consumes Hand/Arm clicks without creating reactions or history; Ride Cloud requires its existing calibrated seat to be ready.

## Selection and feedback

The streak timeout is **4.5 seconds since the last accepted Hand touch**. Hand and Arm have separate **0.65-second acceptance cooldowns**. Rejected clicks do not advance or refresh the streak. Another admitted body region, accepted headpat/Arm interaction, dragging, incompatible ownership, or character replacement breaks the consecutive Hand streak.

| Accepted Hand step | Authored reaction | Face | Extra feedback |
|---|---|---|---|
| 1–2 | Random ready wave1/wave2/salute/thankful | Clip mapping below | No AngryMark |
| 3 | Dismissive Gesture | Angry 0.9 | No AngryMark |
| 4 | Dismissive Gesture | Angry 0.9 | AngryMark |
| 5–6 | Shrugging | Angry 0.9 | No AngryMark |
| 7 | Shrugging | Angry 0.9 | AngryMark; reset streak |

Normal Hand selection avoids the previous normal clip when another loaded candidate is ready. Arm independently selects Arm Stretching or Shoulder Rubbing, avoiding its previous choice when both are ready. Arm never borrows Hand escalation clips.

`TouchReactionAccepted` drives the existing `PetFx`/`PetSfx` architecture. AngryMark is emitted only for accepted Hand steps 4 and 7. Generic visual-change feedback skips TouchReaction, preventing an additional AngryMark on every angry clip. Basic animation previews and desktop playback use the same face mapping.

## Assets and faces

Provenance was resolved from `%APPDATA%/ProjectNera/animation_bookmarks.txt`. The real selected Thankful and Shrugging sources are the bookmarked Clawatar `156_Thankful` and `145_Shrugging` clips; the other six are the bookmarked Mixamo-VRMA clips. Canonical runtime names are stable and do not rename the user's source files.

| Exact source path | Exact runtime copy path | Face/intensity | Authored duration |
|---|---|---|---:|
| `assets/animations/Bookmarks/Wave.vrma` | `assets/animations/animasi_basic/wave1.vrma` | Joy 0.8 | 0.500s |
| `assets/animations/Bookmarks/Wave2.vrma` | `assets/animations/animasi_basic/wave2.vrma` | Joy 0.8 | 1.458s |
| `assets/animations/Bookmarks/Salute.vrma` | `assets/animations/animasi_basic/salute.vrma` | Default 1.0 | 2.833s |
| `assets/animations/Bookmarks/156_Thankful.vrma` | `assets/animations/animasi_basic/thankful.vrma` | Timid 0.65 | 2.833s |
| `assets/animations/Bookmarks/Dismissing_Gesture.vrma` | `assets/animations/animasi_basic/Dismissive Gesture.vrma` | Angry 0.9 | 2.208s |
| `assets/animations/Bookmarks/145_Shrugging.vrma` | `assets/animations/animasi_basic/Shrugging.vrma` | Angry 0.9 | 1.875s |
| `assets/animations/Bookmarks/Arm_Stretching.vrma` | `assets/animations/animasi_basic/Arm Stretching.vrma` | Default 1.0 | 8.833s |
| `assets/animations/Bookmarks/Shoulder_Rubbing.vrma` | `assets/animations/animasi_basic/Shoulder Rubbing.vrma` | Timid 0.65 | 5.833s |

GLB inspection found no expression bindings/channels or scale tracks in these eight assets: they contain humanoid rotation channels and one translation channel. The existing expression layer therefore supplies their faces. No separate Chibi avatar or animation assets were introduced.

All eight runtime copies are registered in `src/animation_manager/base.rs`, using the existing protection against rename, delete, and bookmark movement. The basic catalogue now has 33 clips. Bookmarked source copies retain their existing library/editing ownership.

Asset audit: every runtime copy matches its original SHA256; there were no matching hashes already in the basic directory before copying. The resulting 33 basic files contain zero duplicate hashes. Only the eight intended asset additions appear in Git; no bookmarked original or bookmark metadata was changed.

```text
wave1.vrma           FE48F8B82E9D920D17FDF91F28287CB3696BB5E1A42AC8F2D585DC3C8F854833
wave2.vrma           BBB3BD8C4972DA14CF401B6B4B6BD961340DB1785BE142402A37C5BABAB17B44
salute.vrma          5FFA4D88D4D42F50B8F5C8EC2FC0D93D0A1C144A80794DF261379D8AC9C9E211
thankful.vrma        F13440ED37651D88BF3E50135DA91DF8604D0C7FBCA4577B2F8D1B5F55577E0A
Dismissive Gesture  6EC45E42DEF89F28A8BF9DA5543D9243DC284366D73DBB06A062B39BADEF86B2
Shrugging.vrma       D548D2E9CB59334D5C87B7CD3AF5E2DF39B7222192F7443EA321A1D77EA9FB3C
Arm Stretching      17000DC894B98D90FE20B2793C1DF79B921A98455FE032AA9FC8BFE896A91363
Shoulder Rubbing    9C488AAAAFA7047726E55F13DDE1CBD6C1378D17C7A6AC87882C1A6C95FE375B
```

## Sitting, cloud, and hit testing

Ordinary supported sitting uses `TouchSeatPose` to retain the calibrated hips/legs/feet/toes while the authored upper body reacts. Its cache belongs to the avatar, humanoid hips, support surface, and selected sitting pose. Evaluated transforms are restored before the next pose, then the seat layer runs after animation and before Chibi proportions/VRM constraints. Return blending retains the seat briefly; detach, seat change, or replacement clears ownership. Native support still places the avatar window. No fixed positioning offset was added.

Cloud continues using its existing `CloudSeatPose` and placement owner. Ordinary-seat preservation yields while cloud owns placement. Rest gaze suppression and Chibi transforms remain under their existing owners.

Hitboxes and semantic nearest-intersection selection were not changed. Hand spheres retain radius **0.10**; adjacent forearm capsules retain **0.09**; shoulder/upper-arm capsules retain **0.10**. Overlapping Hand/Arm regions still resolve to the nearest ray intersection, independent of collider spawn order.

## Regression coverage and validation status

Added/extended coverage includes the seven-step Hand sequence, no-repeat selection, timeout boundaries, cooldown bursts, independent Arm selection, other-region/headpat reset, drag rejection, Rest Cloud and Alarm ownership, missing/zero-duration clips, replacement reset without changing Chibi, state priority/return, actual playback dispatch, exact seated-pose restoration and owner clearing, accepted-feedback counts, all protected base files, preserved bookmark originals, and runtime/preview face parity.

Asset SHA256/duplicate audits and `git diff --check` have completed. **Central `cargo fmt`, `cargo check`, and `cargo test` results are pending in this report. Windows native rendered validation is also pending.** No visual-pass or physical pointer-hitbox validation is claimed here. Final command results, screenshots, and any remaining visual issue should be recorded after that validation completes.
