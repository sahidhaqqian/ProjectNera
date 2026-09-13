from pathlib import Path

path = Path("src/environment/sitting.rs")
s = path.read_text(encoding="utf-8")

# ------------------------------------------------------------------
# 1. Import PresentationState
# ------------------------------------------------------------------
import_line = "use crate::presentation::PresentationState;\n"

if import_line not in s:
    marker = "use super::{\n"
    if marker not in s:
        raise SystemExit(
            "Could not find `use super::{` in src/environment/sitting.rs"
        )

    s = s.replace(
        marker,
        import_line + "\n" + marker,
        1,
    )

# ------------------------------------------------------------------
# 2. Give follow_sitting_attachment access to presentation state
# ------------------------------------------------------------------
old_sig = """pub(super) fn follow_sitting_attachment(
    time: Res<Time>,
    mut timer: ResMut<SittingTimer>,
"""

new_sig = """pub(super) fn follow_sitting_attachment(
    presentation: Option<Res<PresentationState>>,
    time: Res<Time>,
    mut timer: ResMut<SittingTimer>,
"""

if new_sig not in s:
    if old_sig not in s:
        raise SystemExit(
            "Could not find follow_sitting_attachment signature. "
            "Your sitting.rs differs from the expected Phase 4 version."
        )

    s = s.replace(old_sig, new_sig, 1)

# ------------------------------------------------------------------
# 3. Presentation owns HWND positioning while Attention is active.
#
# IMPORTANT:
# - Do NOT mutate PlacementState.
# - Do NOT detach from taskbar/window.
# - Do NOT clear target_window.
#
# This preserves the exact sitting attachment for restore.
# ------------------------------------------------------------------
function_marker = """    mut window_query: Query<&mut Window, With<PrimaryWindow>>,
) {
"""

guard = """    // Phase 5.55: Presentation Mode has exclusive ownership of Nera's
    // native window position while Attention is active.
    //
    // Keep PlacementState untouched so a sitting Nera can return to the exact
    // same taskbar/window attachment when Attention ends.
    if presentation
        .as_ref()
        .is_some_and(|state| state.is_attention())
    {
        return;
    }

"""

if guard not in s:
    fn_pos = s.find("pub(super) fn follow_sitting_attachment(")
    if fn_pos == -1:
        raise SystemExit("follow_sitting_attachment not found")

    marker_pos = s.find(function_marker, fn_pos)
    if marker_pos == -1:
        raise SystemExit(
            "Could not find end of follow_sitting_attachment signature"
        )

    insert_at = marker_pos + len(function_marker)
    s = s[:insert_at] + guard + s[insert_at:]

path.write_text(s, encoding="utf-8")

print("Patched:", path)
print("Presentation import:", import_line.strip() in s)
print(
    "Attention guard:",
    "state.is_attention()" in s,
)
print(
    "Placement preserved:",
    "PlacementState::Free" not in guard,
)
