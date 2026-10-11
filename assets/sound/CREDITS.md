# Alarm Sound Sources

Pet interaction/startup/cloud cues have their own source table and original
licenses in [pet/CREDITS.md](pet/CREDITS.md).

The nine alternatives to Classic are distributed under Creative Commons Zero (CC0 1.0).
They may be used and redistributed, including commercially, without attribution.
Credits are retained here for provenance.

| File | Source | Creator | License |
| --- | --- | --- | --- |
| `gentle.ogg` | [Interface Sounds](https://kenney.nl/assets/interface-sounds), `confirmation_001.ogg` | Kenney | CC0 1.0 |
| `chime.ogg` | [Interface Sounds](https://kenney.nl/assets/interface-sounds), `confirmation_004.ogg` | Kenney | CC0 1.0 |
| `clock_beeps.wav` | [alarm beep electronic.wav](https://freesound.org/people/reecord2/sounds/96063/), public HQ MP3 preview | reecord2 | CC0 1.0 |
| `soft_bell.ogg` | [UI Sound Effects](https://opengameart.org/content/ui-sound-effects-button-clicks-user-feedback-notifications), `Ding.wav` | Robin Lamb / VCSL / VSCO 2 CE | CC0 1.0 |
| `wind_chimes.ogg` | Same UI Sound Effects pack, `chimes.wav` | Robin Lamb / VCSL / VSCO 2 CE | CC0 1.0 |
| `deep_bell.ogg` | Same UI Sound Effects pack, `ding_deep.wav` | Robin Lamb / VCSL / VSCO 2 CE | CC0 1.0 |
| `steel_melody.ogg` | [Music Jingles](https://kenney.nl/assets/music-jingles), `jingles_STEEL00.ogg` | Kenney | CC0 1.0 |
| `pizzicato.ogg` | Same Music Jingles pack, `jingles_PIZZI04.ogg` | Kenney | CC0 1.0 |
| `8bit_tune.ogg` | Same Music Jingles pack, `jingles_NES00.ogg` | Kenney | CC0 1.0 |

Kenney's original licenses are included in `KENNEY-LICENSE.txt` (Interface Sounds)
and `KENNEY-JINGLES-LICENSE.txt` (Music Jingles).
The seven new alternatives were converted to WAV/OGG and adjusted toward a
0.50 peak (maximum 4x gain for quiet recordings). Gentle and Chime are unchanged.
Nera uses the existing master volume/mute and repeats finished alarm sounds
with a start interval of at least 3 seconds.

Digital and Siren were removed. Their persisted IDs (3 and 4) now select Clock
Beeps and Soft Bell. Added built-ins use separate reserved IDs, preserving
the imported library's existing IDs starting at 5.

Download provenance:

- https://cdn.freesound.org/previews/96/96063_704006-hq.mp3
- https://opengameart.org/sites/default/files/ui_wav.zip
- https://kenney.nl/media/pages/assets/music-jingles/f37e530b9e-1677590399/kenney_music-jingles.zip

`alarm.mp3` is the pre-existing Classic sound. Its provenance was not established
by this change; it is not represented as one of the newly verified CC0 assets.
