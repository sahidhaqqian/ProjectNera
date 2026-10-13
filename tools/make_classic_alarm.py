"""Derive Classic from the existing CC0 electronic clock cue (no new download)."""
from array import array
from pathlib import Path
import wave

source = Path("assets/sound/clock_beeps.wav")
destination = Path("assets/sound/classic.wav")
with wave.open(str(source), "rb") as wav:
    assert wav.getsampwidth() == 2 and wav.getnchannels() == 2
    rate = wav.getframerate()
    data = array("h", wav.readframes(wav.getnframes()))
frames = len(data) // 2
output_frames = int(frames / 0.82)
output = array("h")
for frame in range(output_frames):
    position = min(frame * 0.82, frames - 1)
    left = int(position)
    right = min(left + 1, frames - 1)
    mix = position - left
    gain = 0.70 * min(1.0, frame / (rate*0.02), (output_frames-1-frame) / (rate*0.06))
    for channel in range(2):
        output.append(round((data[left*2+channel]*(1-mix) + data[right*2+channel]*mix)*gain))
with wave.open(str(destination), "wb") as wav:
    wav.setparams((2, 2, rate, 0, "NONE", "not compressed"))
    wav.writeframes(output.tobytes())
print(f"Classic CC0 clock variation: {destination}, {output_frames/rate:.2f}s")
