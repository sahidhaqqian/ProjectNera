#!/usr/bin/env python3
"""
Patch VRMA GLB files that declare VRMC_vrm_animation in extensionsRequired.

Some glTF validators reject vendor extensions listed in extensionsRequired
before bevy_vrm1 gets a chance to parse VRMC_vrm_animation. This script keeps
the actual VRMC_vrm_animation payload and extensionsUsed entry intact, but
removes that one name from extensionsRequired.

A .bak copy is created before modifying each file.
"""

from __future__ import annotations

import argparse
import json
import shutil
import struct
from pathlib import Path

GLB_MAGIC = b"glTF"
GLB_VERSION = 2
JSON_CHUNK = 0x4E4F534A

def pad4(data: bytes, fill: bytes = b" ") -> bytes:
    return data + fill * ((4 - len(data) % 4) % 4)

def read_glb(path: Path):
    raw = path.read_bytes()
    if len(raw) < 12:
        raise ValueError("file too small to be a GLB/VRMA")

    magic, version, declared_length = struct.unpack_from("<4sII", raw, 0)
    if magic != GLB_MAGIC:
        raise ValueError("not a binary glTF (GLB) file")
    if version != GLB_VERSION:
        raise ValueError(f"unsupported GLB version: {version}")
    if declared_length != len(raw):
        raise ValueError(
            f"GLB length mismatch: header={declared_length}, actual={len(raw)}"
        )

    chunks = []
    offset = 12
    while offset < len(raw):
        if offset + 8 > len(raw):
            raise ValueError("truncated GLB chunk header")
        chunk_len, chunk_type = struct.unpack_from("<II", raw, offset)
        offset += 8
        end = offset + chunk_len
        if end > len(raw):
            raise ValueError("truncated GLB chunk")
        chunks.append((chunk_type, raw[offset:end]))
        offset = end

    if not chunks or chunks[0][0] != JSON_CHUNK:
        raise ValueError("first GLB chunk is not JSON")

    json_bytes = chunks[0][1].rstrip(b" \t\r\n\0")
    document = json.loads(json_bytes.decode("utf-8"))
    return document, chunks[1:]

def write_glb(path: Path, document, other_chunks):
    json_raw = json.dumps(
        document,
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")
    json_raw = pad4(json_raw, b" ")

    encoded_chunks = [
        struct.pack("<II", len(json_raw), JSON_CHUNK) + json_raw
    ]

    for chunk_type, chunk_data in other_chunks:
        # Preserve existing non-JSON chunk bytes exactly.
        encoded_chunks.append(
            struct.pack("<II", len(chunk_data), chunk_type) + chunk_data
        )

    body = b"".join(encoded_chunks)
    header = struct.pack("<4sII", GLB_MAGIC, GLB_VERSION, 12 + len(body))
    path.write_bytes(header + body)

def patch(path: Path) -> bool:
    document, other_chunks = read_glb(path)

    used = document.get("extensionsUsed", [])
    required = document.get("extensionsRequired", [])

    print(f"\n{path}")
    print(f"  extensionsUsed     = {used}")
    print(f"  extensionsRequired = {required}")

    target = "VRMC_vrm_animation"
    if target not in required:
        print("  no patch needed")
        return False

    new_required = [name for name in required if name != target]
    if new_required:
        document["extensionsRequired"] = new_required
    else:
        document.pop("extensionsRequired", None)

    backup = path.with_suffix(path.suffix + ".bak")
    if not backup.exists():
        shutil.copy2(path, backup)
        print(f"  backup              = {backup}")

    write_glb(path, document, other_chunks)
    print(f"  patched: removed {target!r} from extensionsRequired")
    return True

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("files", nargs="+", type=Path)
    args = parser.parse_args()

    failed = False
    for path in args.files:
        try:
            patch(path)
        except Exception as exc:
            failed = True
            print(f"\n{path}\n  ERROR: {exc}")

    raise SystemExit(1 if failed else 0)

if __name__ == "__main__":
    main()
