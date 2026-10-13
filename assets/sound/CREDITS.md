# Alarm Sound Sources

Pet interaction/startup/cloud cues have their own source table and original
licenses in [pet/CREDITS.md](pet/CREDITS.md).

All ten built-in alarm sounds are distributed under Creative Commons Zero (CC0 1.0).
They may be used and redistributed, including commercially, without attribution.
Credits are retained here for provenance.

| File | Source | Creator | License |
| --- | --- | --- | --- |
| `classic.wav` | Derived from the same CC0 electronic alarm below; slower/lower pitch, reduced gain and short fades via `tools/make_classic_alarm.py` | reecord2; variation for Project Nera | CC0 1.0 |
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

The former `alarm.mp3` had unverified provenance and is excluded from the release.
Classic keeps sound ID 0, so saved alarms retain their selection. The replacement
uses existing CC0 audio; no download or playback-system change is required.
