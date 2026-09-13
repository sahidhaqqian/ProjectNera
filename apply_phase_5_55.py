from pathlib import Path

path = Path("src/main.rs")
s = path.read_text(encoding="utf-8")

# ------------------------------------------------------------------
# 1. module
# ------------------------------------------------------------------
if "mod presentation;" not in s:
    marker = "mod platform;\n"
    if marker not in s:
        raise SystemExit("Could not find `mod platform;` in src/main.rs")

    s = s.replace(
        marker,
        marker + "mod presentation;\n",
        1,
    )

# ------------------------------------------------------------------
# 2. import plugin
# ------------------------------------------------------------------
if "use presentation::PresentationPlugin;" not in s:
    marker = "use bevy_vrm1::prelude::*;\n"
    if marker not in s:
        raise SystemExit(
            "Could not find `use bevy_vrm1::prelude::*;` in src/main.rs"
        )

    s = s.replace(
        marker,
        marker + "\nuse presentation::PresentationPlugin;\n",
        1,
    )

# ------------------------------------------------------------------
# 3. plugin registration
# ------------------------------------------------------------------
if "        PresentationPlugin,\n" not in s:
    marker = "        NeraAnimationPlugin,\n"
    if marker not in s:
        raise SystemExit(
            "Could not find NeraAnimationPlugin in the main plugin tuple"
        )

    s = s.replace(
        marker,
        marker + "        PresentationPlugin,\n",
        1,
    )

# ------------------------------------------------------------------
# 4. Disable desktop-only gestures during Attention Mode.
#
# Keep update_click_through alive: alarm dismissal will later need avatar
# clicking even while the transparent HWND covers the monitor.
# ------------------------------------------------------------------
replacements = [
    (
        "                drag_nera_window,\n",
        "                drag_nera_window"
        ".run_if(presentation::desktop_pet_active),\n",
    ),
    (
        "            detect_head_pat.after(update_click_through),\n",
        "            detect_head_pat"
        ".after(update_click_through)"
        ".run_if(presentation::desktop_pet_active),\n",
    ),
    (
        "                settle_pending_sit,\n",
        "                settle_pending_sit"
        ".run_if(presentation::desktop_pet_active),\n",
    ),
]

for old, new in replacements:
    if new in s:
        continue

    if old not in s:
        print(
            "warning: integration marker not found, skipped:",
            old.strip(),
        )
        continue

    s = s.replace(old, new, 1)

path.write_text(s, encoding="utf-8")

print("Phase 5.55 Presentation Mode integrated into src/main.rs.")
print("Debug: press F9 to enter/exit Alarm Attention preview.")
