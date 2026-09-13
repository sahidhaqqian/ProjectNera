from pathlib import Path
import re

# ================================================================
# Cargo.toml — enable Bevy audio + MP3 only
# ================================================================
cargo_path = Path("Cargo.toml")
cargo = cargo_path.read_text(encoding="utf-8")

m = re.search(
    r'(bevy\s*=\s*\{.*?features\s*=\s*\[)(.*?)(\]\s*\})',
    cargo,
    flags=re.S,
)

if not m:
    raise SystemExit(
        "Could not find Bevy features array in Cargo.toml"
    )

features = m.group(2)

extra = []
if '"bevy_audio"' not in features:
    extra.append('    "bevy_audio",')
if '"mp3"' not in features:
    extra.append('    "mp3",')

if extra:
    new_features = features.rstrip() + "\n" + "\n".join(extra) + "\n"
    cargo = (
        cargo[:m.start(2)]
        + new_features
        + cargo[m.end(2):]
    )
    cargo_path.write_text(cargo, encoding="utf-8")
    print("Cargo.toml: enabled bevy_audio + mp3")
else:
    print("Cargo.toml: audio features already present")

# ================================================================
# src/main.rs — module + plugin registration
# ================================================================
main_path = Path("src/main.rs")
s = main_path.read_text(encoding="utf-8")

if "mod alarm;" not in s:
    marker = "mod animation;\n"
    if marker not in s:
        raise SystemExit("Could not find `mod animation;` in main.rs")
    s = s.replace(
        marker,
        marker + "mod alarm;\n",
        1,
    )

# Prefer PresentationPlugin position; fallback to NeraAnimationPlugin.
if "        alarm::AlarmPlugin,\n" not in s:
    if "        PresentationPlugin,\n" in s:
        s = s.replace(
            "        PresentationPlugin,\n",
            "        PresentationPlugin,\n"
            "        alarm::AlarmPlugin,\n",
            1,
        )
    elif "        NeraAnimationPlugin,\n" in s:
        s = s.replace(
            "        NeraAnimationPlugin,\n",
            "        NeraAnimationPlugin,\n"
            "        alarm::AlarmPlugin,\n",
            1,
        )
    else:
        raise SystemExit(
            "Could not find PresentationPlugin or NeraAnimationPlugin "
            "inside the plugin tuple"
        )

main_path.write_text(s, encoding="utf-8")
print("main.rs: AlarmPlugin integrated")
