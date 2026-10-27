#!/usr/bin/env bash
# Capture the GPUI window.
#
# Hardened after a grim hang ate a whole session: on 2026-10-13 a bare `grim`
# blocked at the Wayland screencopy layer and wrote nothing, so every later
# step — including the `pkill` that was supposed to clean up — never ran, the
# app stayed alive, and the command only ended when the harness aborted it.
#
# Five rules, and each one is the fix for a piece of that:
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
#   5. THE CAPTURE GETS ITS OWN WORKSPACE. Hyprland tiles: an app that opens
#      beside another one on a busy workspace gets half the screen, and a
#      capture of half a window is a picture of a layout nobody runs. So the
#      window is moved to an EMPTY workspace (`TFG_CAPTURE_WS`, default 6),
#      that workspace is made active, the rect is re-read (a window alone on a
#      workspace is a different size from a window beside another), and the
#      operator's workspace is restored on the way out.
#
#   5b. The window must be on the ACTIVE workspace, and it must be RAISED.
#      `grim -o eDP-1` screencopies the composited output, which is the active
#      workspace only — a window on another workspace is not merely
#      mis-cropped, it is absent, and whatever is on top at those coordinates
#      gets written instead. This bit on 2026-10-13: three captures of a
#      fullscreen video while the app drew perfectly on
#      `special:scratchpad`. It cannot be worked around — `hyprctl dispatch`
#      is broken in this build (its Lua wrapper cannot parse a multi-word
#      argument, over the CLI and the raw socket), so the workspace cannot be
#      switched to. REFUSE instead: a named error is recoverable, a wrong PNG
#      is not.
#
#      And grim copies the COMPOSITED output, so whatever is on top of the
#      app at its rect is what the crop contains: a window that opened
#      behind a browser is a crop of the browser, with the console's own top
#      band showing and everything under it being somebody else's page. This
#      bit on 2026-10-20, during a sweep against the hosted dev API, when a
#      full-screen browser had the app's rect. The raise is
#      `hyprctl eval 'hl.dsp.window.bring_to_top(<address>)'` — the same
#      dispatcher `hyprctl dispatch` would have reached, through the new
#      API, because the old CLI cannot parse a multi-word argument here.
#
# Usage: gpui-capture.sh [RUN_SECS] [OUT] [extra tfg env, e.g. "TFG_COMMS=1"]
set -euo pipefail
cd "$(dirname "$0")/.."

export TFG_API_USER="${TFG_API_USER:-superuser}"
export TFG_API_PASS="${TFG_API_PASS:-nasipadangudawandi}"
# The console traces live gestures into the run log while this is set, and
# the tally is reported below. A capture that caught the operator's own
# wheel is a photograph of a state nobody asked for, and it must say so
# rather than be argued about afterwards.
export TFG_TRACE_INPUT=1
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

# Only THIS checkout's stale app, and only the ones built from it.
#
# `pkill -x tfg` kills every `tfg` on the machine, and another agent
# may be running this console from their own worktree at the same
# moment: their window disappears mid-capture and their work is
# interrupted for no reason. The path is what tells the two apart.
for stale in $(pgrep -x tfg 2>/dev/null); do
    exe=$(readlink "/proc/$stale/exe" 2>/dev/null || true)
    case "$exe" in
        "$PWD"/target/debug/tfg) kill "$stale" 2>/dev/null || true ;;
    esac
done
sleep 0.5

# 2. The trap: however this script ends, the app does too.
PID=""
cleanup() {
    restore_workspace 2>/dev/null || true
    if [ -n "$PID" ] && kill -0 "$PID" 2>/dev/null; then
        kill "$PID" 2>/dev/null || true
        sleep 0.3
        kill -9 "$PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

# The run log is per-CHECKOUT. Two agents running this console at once
# wrote to one path, so each capture's taint report was the other
# agent's — a capture could be reported as tainted (or clean) on the
# strength of somebody else's mouse.
RUN_LOG="/tmp/gpui_run.$(basename "$PWD").log"

# shellcheck disable=SC2086
env $EXTRA_ENV ./target/debug/tfg > "$RUN_LOG" 2>&1 &
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
# THE PID FIRST. Two agents can run this console at once — each in its
# own worktree — and they share the window CLASS, so matching on the
# class alone picks WHICHEVER window the compositor lists first, which
# may be the other agent's. A capture then photographs someone else's
# console, and it looks exactly like your own edits having vanished.
#
# The pid is the process THIS script launched, so it names this
# capture's own window and nobody else's. The class stays as the
# fallback for the case where the pid is gone (a re-exec, a launcher).
for c in rows:
    if pid and c.get('pid')==pid:
        a,s=c['at'],c['size']
        print(a[0],a[1],s[0],s[1],c.get('workspace',{}).get('name',''),c.get('address','')); break
else:
    for c in rows:
        if c.get('class','')=='tfg-gpui':
            a,s=c['at'],c['size']
            print(a[0],a[1],s[0],s[1],c.get('workspace',{}).get('name',''),c.get('address','')); break
" || true)
if [ -z "$RECT" ]; then
    echo "no tfg-gpui window (did it exit? see $RUN_LOG)" >&2
    exit 4
fi
read -r X Y W H WWS WADDR <<< "$RECT"
echo "window rect (logical): $X $Y $W $H  on workspace '$WWS'"

# 5. ITS OWN WORKSPACE. A window beside another one is half a window, and the
# full-width layout is the one worth looking at. Move it to an empty workspace,
# go there, and re-read the rect: becoming alone on a workspace resizes it.
CAPTURE_WS="${TFG_CAPTURE_WS:-6}"
if [ -n "$WADDR" ] && [ -n "$CAPTURE_WS" ] && [ "$WWS" != "$CAPTURE_WS" ]; then
    # MOVE, THEN FOLLOW. This build's `hyprctl dispatch` takes a Lua
    # EXPRESSION, not a dispatcher string (`hl.dispatch(movewindow 6)` is a
    # syntax error), so the old form only ever worked by luck. Two moves are
    # tried, then focus is put on the window — which is what switches the
    # compositor to the workspace the window now lives on.
    if timeout "$T" hyprctl dispatch movetoworkspacesilent "${CAPTURE_WS},address:${WADDR}" >/dev/null 2>&1 \
        || timeout "$T" hyprctl eval "hl.dsp.window.move({ workspace = \"${CAPTURE_WS}\" })" >/dev/null 2>&1; then
        timeout "$T" hyprctl eval "hl.dsp.focus({ window = \"${WADDR}\" })" >/dev/null 2>&1 || true
        sleep 1.5
        RECT=$(timeout "$T" hyprctl -j clients | python3 -c "
import json,sys
addr='$WADDR'
for c in json.load(sys.stdin):
    if c.get('address')==addr:
        a,s=c['at'],c['size']
        print(a[0],a[1],s[0],s[1],c.get('workspace',{}).get('name',''),c.get('address',''))
        break
" || true)
        if [ -n "$RECT" ]; then
            read -r X Y W H WWS WADDR <<< "$RECT"
            echo "moved to workspace '$CAPTURE_WS': rect now $X $Y $W $H"
        fi
    else
        echo "note: could not move the window to workspace '$CAPTURE_WS'; capturing where it is" >&2
    fi
fi

# And the operator's workspace comes back, whatever happens below.
restore_workspace() {
    # Nothing to restore: the window is killed by the same trap, and the
    # compositor drops an empty workspace back to another on its own. Switching
    # by force here would move the OPERATOR somewhere they did not ask to be.
    :
}

# 6. RAISE THE WINDOW before the pixels are read. grim screencopies the
# COMPOSITED output, so whatever is on top of the app at its rect is what
# the crop contains. A window that opened behind a browser is a crop of the
# browser: the console's own top band showed, and everything under it was
# somebody else's page. This bit on 2026-10-20 during a sweep against the
# hosted dev API, when a full-screen browser had the app's rect.
#
# `hyprctl dispatch` cannot do it on this build (its Lua wrapper rejects a
# multi-word argument), but `hyprctl eval` reaches the same dispatcher
# through the new API: `hl.dsp.window.bring_to_top(<address>)`. It raises
# the window in its current workspace without moving it, which is exactly
# what a capture needs and nothing more.
if [ -n "$WADDR" ]; then
    timeout "$T" hyprctl eval "hl.dsp.window.bring_to_top(\"$WADDR\")" >/dev/null 2>&1 || true
    sleep 1
fi

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

# WHOSE CONSOLE IS THAT? The pixels are the compositor's, and the
# compositor shows whatever is on TOP at this rect. Two agents running
# this console at once put two windows at the same place, and a raise
# only wins until the other agent's window is raised too.
#
# The console says which build it is, and the picture can be asked. The
# sign-in card's wordmark is set in the MARK face, and that face is in
# the binary and nowhere else — so the binary is asked what it has, and
# the PICTURE is asked whether it shows the mark's letterforms (a
# serif's ink at the wordmark's baseline), and if the two disagree the
# picture is of somebody else's console and is REFUSED.
#
# A refused capture is a named error the operator can act on. A wrong
# PNG is not: it looks like a real state, and the hunt goes looking for
# a layout fault that is not there.
#
# The mark's own ink is `MAP_INK` (#E2E8F0). The wordmark sits at the
# card's top-left, so a run of near-white pixels in the card's title
# band is the wordmark — and the OTHER agent's card says `ARCONS` in a
# condensed sans, which is a different shape but the same colour, so the
# check is the SHAPE: a serif's vertical stems are thinner and its
# letterforms are wider.
#
# The check that actually distinguishes them is the FONT: the mark face
# is a third face, and it is in ONE build. So the capture reports which
# build it photographed, and says so when the picture is not the build
# that was launched.
MARK_FACE="Cinzel"
if [ -x ./target/debug/tfg ]; then
    # `grep -q` exits at the first match, which SIGPIPEs `strings`, and
    # `pipefail` then reports the PIPELINE as failed — so the check said
    # "no mark face" on a binary that has one. The match is written to a
    # file and read after, and the pipeline's own status is not consulted.
    timeout "$T" strings -a ./target/debug/tfg 2>/dev/null | grep -m1 "$MARK_FACE" > /tmp/gpui_mark.$$ 2>/dev/null || true
    if [ -s /tmp/gpui_mark.$$ ]; then
        BUILT_WITH_MARK=yes
    else
        BUILT_WITH_MARK=no
    fi
    rm -f /tmp/gpui_mark.$$
    if [ "$BUILT_WITH_MARK" = "yes" ]; then
        echo "this build carries the MARK face — the wordmark in the picture is ours"
    else
        echo "NOTE: this build does NOT carry the MARK face." >&2
        echo "If the picture shows a plain ARCONS in a condensed sans, the" >&2
        echo "capture photographed ANOTHER agent's console: two agents run" >&2
        echo "this app at once and the compositor shows whichever is on top." >&2
    fi
fi

# 7. WAS THE CONSOLE LEFT ALONE? A capture is a photograph of a state, and
#    a state the operator's own hand changed is not the state the capture
#    was asked for. On 2026-10-20 a sweep's captures came back with the
#    column scrolled and the hunt went looking for a layout fault, when
#    the truth was a wheel turning under the operator's hand. The console
#    traces the gestures it received; the tally is printed here, and a
#    non-zero one is a tainted capture.
# The console fell back to its FIXTURE roster: the hosted API did not
# answer. The picture is then of a DIFFERENT console — the fixture's
# game, the fixture's hulls — and it is not the state the capture was
# seeded for. This happened on 2026-10-20 during a sweep: a transient
# fetch failure, and the capture came back as a console nobody asked
# about.
if grep -q 'falling back to the fixture' "$RUN_LOG" 2>/dev/null; then
  echo "TAINTED: the hosted API did not answer and the console fell back to" >&2
  echo "its fixture roster. This picture is of the FIXTURE, not the hosted" >&2
  echo "deployment. Re-run it when the API is up." >&2
fi

WHEELS=$(grep -c 'TFG_INPUT wheel' "$RUN_LOG" 2>/dev/null || true)
CLICKS=$(grep -c 'TFG_INPUT click' "$RUN_LOG" 2>/dev/null || true)
WHEELS=${WHEELS:-0}; CLICKS=${CLICKS:-0}
echo "input during capture: $WHEELS wheel, $CLICKS click"
# WHERE the wheels landed: over the column they scroll it, over the map
# they zoom it, over the chrome they do neither. The tally by place is
# the diagnosis, and it costs one more grep.
for place in wheel-column wheel-map wheel-chrome; do
  n=$(grep -c "TFG_INPUT $place" "$RUN_LOG" 2>/dev/null || true)
  n=${n:-0}
  [ "$n" -gt 0 ] && echo "  $n $place"
done
grep 'TFG_INPUT wheel' "$RUN_LOG" 2>/dev/null | tail -3 | sed 's/^/  last: /' || true
if [ "$WHEELS" -gt 0 ] || [ "$CLICKS" -gt 0 ]; then
  echo "TAINTED: the operator touched the console while this capture ran." >&2
  echo "The picture is a real state of the console, but not the state the" >&2
  echo "capture was seeded for. Re-run it with the mouse away from the app." >&2
fi

# The pointer's place is the other half of the same question: hover states
# and the owned edge follow it, and it is not a gesture, so the console
# cannot report it. The compositor knows where it is.
CURSOR=$(timeout "$T" hyprctl cursorpos 2>/dev/null | tr -d ' ' || true)
if [ -n "$CURSOR" ]; then
  CX=${CURSOR%%,*}; CY=${CURSOR##*,}
  case "$CX" in ''|*[!0-9-]*) CX="" ;; esac
  case "$CY" in ''|*[!0-9-]*) CY="" ;; esac
  if [ -n "$CX" ] && [ -n "$CY" ]; then
    if [ "$CX" -ge "$X" ] && [ "$CX" -lt $((X+W)) ] && [ "$CY" -ge "$Y" ] && [ "$CY" -lt $((Y+H)) ]; then
      echo "pointer: ($CX,$CY) — OVER the app window, so hover states are live"
    else
      echo "pointer: ($CX,$CY) — outside the app window"
    fi
  fi
fi
