#!/usr/bin/env bash
# Capture the GPUI window.
#
# Hardened after a grim hang ate a whole session: on 2026-10-13 a bare `grim`
# blocked at the Wayland screencopy layer and wrote nothing, so every later
# step — including the `pkill` that was supposed to clean up — never ran, the
# app stayed alive, and the command only ended when the harness aborted it.
#
# Four rules, and each one is the fix for a piece of that:
#
#   1. EVERY external call has a `timeout`. A blocked tool is a fast, named
#      error here, never an unbounded wait.
#   2. The app is killed by a TRAP, not by a statement at the end. A failure
#      anywhere still reaps it.
#   3. grim is PROBED before the app is even started. If screencopy is stuck
#      there is nothing to capture, and the honest thing is to say so in eight
#      seconds rather than to launch, wait, and hang.
#   4. The window is found by CLASS (`tfg-gpui`) and only then by PID. Hyprland
#      places this window at a different spot each launch — (768,26) and
#      (0,445) a minute apart — so a rect read from a stale listing, or a
#      second window sharing a pid, crops the WRONG window and the capture
#      looks like a plausible but wrong picture. That cost a whole debugging
#      pass: the shell was drawing the sign-in card while the crops showed the
#      console.
#   5. The window must be on the ACTIVE workspace. `grim -o eDP-1` screencopies
#      the composited output, which is the active workspace only — a window on
#      another workspace is not merely mis-cropped, it is absent, and whatever
#      is on top at those coordinates gets written instead. This bit on
#      2026-10-13: three captures of a fullscreen video while the app drew
#      perfectly on `special:scratchpad`. It cannot be worked around —
#      `hyprctl dispatch` is broken in this build (its Lua wrapper cannot parse
#      a multi-word argument, over the CLI or the raw socket), so the workspace
#      cannot be switched to. REFUSE instead: a named error is recoverable, a
#      wrong PNG is not.
#
# Usage: gpui-capture.sh [RUN_SECS] [OUT] [extra tfg_gpui env, e.g. "TFG_COMMS=1"]
set -euo pipefail
cd "$(dirname "$0")/.."

export TFG_API_USER="${TFG_API_USER:-superuser}"
export TFG_API_PASS="${TFG_API_PASS:-nasipadangudawandi}"
RUN_SECS="${1:-14}"
OUT="${2:-/tmp/gpui_shot.png}"
EXTRA_ENV="${3:-}"

T=8   # seconds any single external call may take

# 3. PREFLIGHT: is screencopy alive at all?
if ! timeout "$T" grim -o eDP-1 /tmp/gpui_preflight.png 2>/dev/null; then
    echo "REFUSING: grim cannot capture (timeout ${T}s, no file written)." >&2
    echo "Wayland screencopy looks stuck; try \`hyprctl reload\` before a capture." >&2
    echo "Nothing was launched and nothing is left running." >&2
    exit 3
fi
rm -f /tmp/gpui_preflight.png

pkill -x tfg_gpui 2>/dev/null || true
sleep 0.5

# 2. The trap: however this script ends, the app does too.
PID=""
cleanup() {
    if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        kill "$PID" 2>/dev/null || true
        sleep 0.3
        kill -9 "$PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

# shellcheck disable=SC2086
env $EXTRA_ENV ./target/debug/tfg_gpui > /tmp/gpui_run.log 2>&1 &
PID=$!
echo "launched pid $PID  (env: ${EXTRA_ENV:-none})"

sleep "$RUN_SECS"

# 1. Timeout on the compositor query too: it is an IPC round trip. CLASS
# FIRST: this window's class is `tfg-gpui` (the app_id mystery resolved), and
# matching it is immune to the pid reuse and the per-launch placement that
# made a pid+rect match crop someone else's window.
RECT=$(timeout "$T" hyprctl -j clients | python3 -c "
import json,sys,os
pid=int(os.environ.get('PID') or 0)
rows=json.load(sys.stdin)
for c in rows:
    if c.get('class','')=='tfg-gpui':
        a,s=c['at'],c['size']
        print(a[0],a[1],s[0],s[1],c.get('workspace',{}).get('name','')); break
else:
    for c in rows:
        if pid and c.get('pid')==pid:
            a,s=c['at'],c['size']
            print(a[0],a[1],s[0],s[1],c.get('workspace',{}).get('name','')); break
" || true)
if [ -z "$RECT" ]; then
    echo "no tfg-gpui window (did it exit? see /tmp/gpui_run.log)" >&2
    exit 4
fi
read -r X Y W H WWS <<< "$RECT"
echo "window rect (logical): $X $Y $W $H  on workspace '$WWS'"

# 5. The window has to be on the workspace grim will screencopy. Being on
# another one means it is not in the image at all, and the pixels at its rect
# belong to whatever is on top there.
ACTIVE=$(timeout "$T" hyprctl -j activeworkspace | python3 -c 'import json,sys; print(json.load(sys.stdin)["name"])' || true)
if [ -n "$WWS" ] && [ "$WWS" != "$ACTIVE" ]; then
    echo "REFUSING: tfg-gpui is on workspace '$WWS' but the active workspace is '$ACTIVE'." >&2
    echo "grim screencopies the active workspace only, so this capture would show" >&2
    echo "whatever is on top there instead of the app. Switch to '$WWS' and re-run." >&2
    echo "(This build's \`hyprctl dispatch\` cannot switch workspaces — its Lua wrapper" >&2
    echo "rejects a multi-word argument, over the CLI and the raw IPC socket alike.)" >&2
    exit 5
fi

# Physical = logical x 1.25 on this monitor.
PX=$(python3 -c "print(round($X*1.25))"); PY=$(python3 -c "print(round($Y*1.25))")
PW=$(python3 -c "print(round($W*1.25))"); PH=$(python3 -c "print(round($H*1.25))")

timeout "$T" grim -o eDP-1 /tmp/gpui_full.png
timeout "$T" magick /tmp/gpui_full.png -crop "${PW}x${PH}+${PX}+${PY}" +repage "$OUT"
echo "captured $OUT  ($(stat -c%s "$OUT") bytes)"
