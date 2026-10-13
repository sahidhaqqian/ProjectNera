"""Retain resolved dependency notices without shipping Project Nera source."""
import json
import sys
from pathlib import Path

metadata = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8-sig"))
output = Path(sys.argv[2] if len(sys.argv) > 2 else "assets/licenses")
output.mkdir(parents=True, exist_ok=True)
components = []
texts = []
for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
    if package["name"] == "nera":
        continue
    root = Path(package["manifest_path"]).parent
    components.append({k: package.get(k) for k in ("name", "version", "license", "repository", "source")})
    files = sorted(set(root.glob("LICENSE*")) | set(root.glob("LICENCE*")) | set(root.glob("COPYING*")) | set(root.glob("NOTICE*")))
    for file in files:
        if file.is_file():
            texts.append(f"\n{'='*72}\n{package['name']} {package['version']} — {file.name}\n\n{file.read_text(encoding='utf-8', errors='replace')}")
    if package["name"] == "bevy_text":
        for file in root.rglob("*LICENSE*"):
            if file not in files and file.is_file():
                texts.append(f"\nEmbedded font / {package['name']} {package['version']} — {file.name}\n\n{file.read_text(encoding='utf-8', errors='replace')}")
font_license = output / "FiraMono-LICENSE.txt"
if not font_license.is_file():
    raise SystemExit("Missing bundled Fira Mono license; preserve assets/licenses/FiraMono-LICENSE.txt")
texts.append("\nEmbedded Bevy default font — Fira Mono (SIL OFL 1.1)\n\n"+font_license.read_text(encoding="utf-8"))
(output / "COMPONENTS.json").write_text(json.dumps(components, indent=2)+"\n", encoding="utf-8")
(output / "DEPENDENCY_LICENSES.txt").write_text("Resolved build graph (including build/test and platform dependencies).\n"+"".join(texts), encoding="utf-8")
print(f"{len(components)} components; {len(texts)} retained license/notice texts")
