#!/usr/bin/env bash
# Drive tfg to a known state: signed in, session SHOOT-01 held.
#
#   bash scripts/ui-shoot/drive-to-session.sh
#
# NEVER inherits DISPLAY. An earlier version read ${DISPLAY:-:99}, and inside
# an agent shell that is the OPERATOR'S REAL DESKTOP, so the app opened a
# window on the screen they were using and the driver then found no egui window
# on :99. The display is named here and only here, the same way xvfb-up.sh
# takes X11_DISPLAY_NUM. `unset DISPLAY` is not enough on its own because winit
# falls back to $DISPLAY from the environment it inherits.
set -euo pipefail

DISPLAY_NUM=${X11_DISPLAY_NUM:-:99}
export DISPLAY="$DISPLAY_NUM"
export XDRV_DISPLAY="$DISPLAY_NUM"
X=${X:-/tmp/opencode/xdrv}

ROOT=/tmp/opencode
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# The session is put back into `planning` before the app is driven.
#
# Not housekeeping. The zone's contents are a function of the session's state,
# and an earlier run of verify-scroll.sh clicked "Enter preparation", which
# moved the column from Essentials to Readiness. The next run then measured
# Readiness while every assertion still named Essentials, and reported the
# differences as defects. The console's own state machine is doing exactly what
# it should; the harness has to put it back.
#
# Written straight to the database rather than through the API because the
# transition is not a PATCH: the state machine refuses a planning->planning
# move, so there is no request that undoes it.
reset_state() {
  local pw=${PGPASSWORD:-local-owner-pw}
  PGPASSWORD="$pw" psql -h 127.0.0.1 -p 5433 -U minos -d minos -tAc \
    "update games set state='planning' where id = $1" >/dev/null
}
GAME_ID=$(curl -s -X POST http://127.0.0.1:8099/api/api/v1/auth/login \
  -H 'Content-Type: application/json' \
  -d '{"identifier":"supersuser","password":"tfg-dev-password"}' \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["data"]["access_token"])' \
  | xargs -I{} curl -s -H "Authorization: Bearer {}" \
      'http://127.0.0.1:8099/api/api/v1/games?page_size=100&page_number=1' \
  | python3 -c '
import sys, json
rows = json.load(sys.stdin)["data"]
rows = rows if isinstance(rows, list) else rows.get("games", [])
named = [g for g in rows if g.get("name") == "SHOOT-01"]
print(max(named, key=lambda g: g["id"])["id"] if named else "")
')
[ -n "$GAME_ID" ] && reset_state "$GAME_ID"

# Always restarted. The app reads the session's state when it logs in and holds
# it, so resetting the database under a running instance changes nothing on
# screen — which is how a run ended up photographing Readiness while believing
# it was looking at Essentials.
if pgrep -x tfg >/dev/null; then
  pkill -x tfg
  sleep 2
fi
if ! pgrep -x tfg >/dev/null; then
  cd "$REPO"
  setsid env -u WAYLAND_DISPLAY DISPLAY="$DISPLAY_NUM" \
    TFG_MINOS_HOST=http://127.0.0.1:8099/api/api/v1 TFG_ZONE_DEBUG=1 \
    LIBGL_ALWAYS_SOFTWARE=1 \
    ./target/debug/tfg > "$ROOT/tfg.log" 2>&1 < /dev/null &
  # Software rendering plus map tiles drops input for the first seconds, which
  # reads as a dead control rather than a busy one.
  sleep 30
fi

"$X" focus >/dev/null
sleep 0.5
"$X" click 519 340; sleep 0.4
"$X" type supersuser; sleep 0.3
"$X" click 519 384; sleep 0.4
"$X" type tfg-dev-password; sleep 0.3
"$X" click 519 426; sleep 7

# The session picker, then the newest SHOOT-01 in it.
"$X" click 95 264; sleep 1.2
"$X" click 112 297; sleep 6

import -window root -crop 1040x640+0+0 +repage "$ROOT/shots/state.png"
echo "session $GAME_ID, state.png written"
grep 'zone:' "$ROOT/tfg.log" | tail -1
grep 'z\.' "$ROOT/tfg.log" | tail -6