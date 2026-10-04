#!/usr/bin/env bash
# What the zone costs the operator at the default 1040x640, and where the
# wheel goes.
#
#   bash scripts/ui-shoot/verify-scroll.sh
#
# Fails loudly. Every case reads a number the app printed or a pixel it
# rendered. None of them reads a screenshot and forms an opinion.
#
# THE INVARIANT, and why the checks are shaped the way they are.
#
# One wheel gesture must move exactly one thing: the island body under the
# pointer if that body has room in the direction being scrolled, otherwise the
# column. The defect this verifies is that neither was reachable — Essentials
# wanted ~760pt of content against a 380pt island, and the column moves whole
# islands, so nothing could reach the composer.
#
# Asserting that by counting wheel notches does not work, and cost three runs
# to find out why. A gesture is smooth: one notch of the wheel is many frames,
# the body absorbs them until it runs out mid-gesture, and the column picks up
# the remainder. So "the body absorbed N notches" measures where the boundary
# happened to fall, not whether the boundary was respected.
#
# What is checkable is the pair, per frame: the offset changed AND no body under
# the pointer had room. The app prints both on one line, so the check is a
# filter over the log rather than a timing experiment.
#
# NO CLICK TARGET AND NO STARTING SCROLL IS ASSUMED. Positions come from
# find-buttons.py, which reads a button's fill out of the frame, and from the
# app's own island report. Both were learned the hard way: the first version of
# this script hard-coded (101,520) for the composer's button, the app was in a
# different state by then, that coordinate was "declare ready", and the script
# POSTed the session from planning into preparation while measuring it.
set -uo pipefail

DISPLAY_NUM=${X11_DISPLAY_NUM:-:99}
export DISPLAY="$DISPLAY_NUM"
export XDRV_DISPLAY="$DISPLAY_NUM"
X=${X:-/tmp/opencode/xdrv}
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SHOTS=${SHOTS:-/tmp/opencode/shots}
LOG=/tmp/opencode/tfg.log
FULL="$SHOTS/full.png"
NOTCH=3
SETTLE=1.2

mkdir -p "$SHOTS"
fails=0

mark() { wc -l < "$LOG"; }

scroll_now() {
  local v
  v=$(grep 'zone: ' "$LOG" | tail -1 | grep -o 'scroll=[0-9.]*' | cut -d= -f2)
  printf '%s' "${v:-0}"
}

# An island's on-screen geometry, as the app reports it: x y width height.
#
# Anchored on `pos=` because the app logs more than one line per island, and a
# bare match on the id returns the wrong one — which fed fields cut out of a
# sentence into arithmetic.
#
# ALL FOUR from the app. Reading the height here and the width from
# tokens::ZONE_W is how the crop came out `380x+212+320`: the height and the y
# were the width. A geometry this script depends on belongs in one place, and
# the app already prints it.
#
# UNANCHORED at the end, on purpose. The line continues `body=Some(BodyFit
# {...})`, so a `$` anchor made the substitution silently not fire, `island`
# returned nothing, and every caller arithmetic'd on an empty string: the gap
# strip came out at y=-8, the pointer parked off-window at 0, and two cases
# reported a dead column. A parser that returns nothing instead of erroring is
# the failure mode worth writing down.
#
# The y is SCREEN space, not the app's logged `pos.y`. The zone applies its
# scroll as a view transform at draw time (chrome::island_owned takes `scroll`
# as a parameter and subtracts it), so the logged position is where the island
# LIVES in the column, not where it is drawn. Reading it as screen space put
# Essentials' crop 500pt below the island on screen — a crop that caught the
# Fleet picker button instead, and a click that opened the Fleet picker.
island() {
  local scrolled
  scrolled=$(scroll_now)
  grep "Id::new(\"$1\") pos=" "$LOG" | tail -1 |
    sed -E 's/.*pos=\(([0-9.]+),([0-9.]+)\) size=([0-9.]+)x([0-9.]+).*/\1 \2 \3 \4/' |
    awk -v s="$scrolled" '{print int($1), int($2 - s), int($3), int($4)}'
}

# The same island, CLIPPED to the band, and the clip is what makes the crop
# mean anything.
#
# An island scrolled half off the top of the band is drawn clipped: its title
# band is not on screen and its lower half is. Its own rect still says 380pt
# tall at its column position, so cropping to the rect and calling the result
# "Essentials" captures whatever island happens to occupy that part of the
# window — which is how the composer button turned into the Fleet picker's.
#
# chrome::island_on_band already decides what is visible; this is the same rule
# applied to geometry rather than to drawing.
island_on_band() {
  local _ y h top bot
  read -r _ y _ h <<<"$(island "$1")"
  top=$ZY
  bot=$ZY1
  # A rect that does not reach into the band is not on it.
  [ $((y + h)) -lt "$top" ] && return 1
  [ "$y" -gt "$bot" ] && return 1
  local top_y=$y bottom_y=$((y + h))
  [ "$top_y" -lt "$top" ] && top_y=$top
  [ "$bottom_y" -gt "$bot" ] && bottom_y=$bot
  printf '%s %s' "$top_y" "$((bottom_y - top_y))"
}

# The zone's own rect, from chrome::zone_band, as the app logs it.
zone() {
  grep 'zone: ' "$LOG" | tail -1 |
    sed -E 's/.*band=\[\[([0-9.]+) ([0-9.]+)\] - \[([0-9.]+) ([0-9.]+)\]\].*/\1 \2 \3 \4/' |
    awk '{print int($1), int($2), int($3), int($4)}'
}

grab() { import -window root -crop "${2:-1040x640+0+0}" +repage "$1"; }

# The hex colour at a point. magick prints srgb(...) and the comparison wants a
# bare triple, so the wrapper is stripped here.
probe() { magick "$FULL" -format "%[pixel:p{$1,$2}]" info: | sed 's/^srgb(\(.*\))$/\1/'; }

# Whether a modal is up, by the colour of its title band.
#
# PROBED AT y=60, NOT AT THE BAND'S CENTRE. The band's own centre at y=55 runs
# through the tracked label "Scenario composer", so the probe returned the
# label's ink (196,202,212) and the check for Panel Slate (30,41,59) failed
# against a modal that was plainly open. Four points down is below the cap
# height of every title in the set, so it reads the fill whatever the modal is
# called.
modal_up() { [ "$(probe 200 60)" = "30,41,59" ]; }

ok()  { printf 'ok   %-50s %s\n' "$1" "$2"; }
bad() { printf 'FAIL %-50s %s\n' "$1" "$2"; fails=$((fails + 1)); }

check_eq() {
  if [ "$3" = "$2" ]; then ok "$1" "$3"; else bad "$1" "want $2, got $3"; fi
}

check_gt() {
  if awk "BEGIN{exit !((\"$3\") > (\"$2\"))}"; then ok "$1" "$3 > $2"; else bad "$1" "want > $2, got $3"; fi
}

# The band's four corners, as four NAMED variables. `zone` emits no leading
# space, so `read -r _ zx zy zx1 zy1` bound the first corner to `_` and shifted
# the rest into the crop geometry — `584x-344+40+344`. A negative height was the
# loud version; had the numbers stayed positive it would have cropped the wrong
# region and clicked something else.
read -r ZX ZY ZX1 ZY1 <<<"$(zone)"
ZONE_W=$((ZX1 - ZX))

# An island body's current scroll offset, as the app reports it.
#
# Space before the closing brace, which the app's Debug formatting puts there:
# `offset: 240.0 })`. Anchoring on `})` with no space matched nothing and the
# value came back as the whole log line, which then failed the numeric
# comparison as a syntax error rather than as a false.
body_offset() {
  grep "Id::new(\"$1\") pos=" "$LOG" | tail -1 |
    sed -E 's/.*offset: ([0-9.]+) *\}\) *$/\1/'
}

# A button's centre, chosen by position inside one island's own rect.
#
# find-buttons.py emits one row per button: y0 y1 x0 x1 w h cx cy.
#
# LOWEST inside the island, not widest. Two earlier attempts at "the composer
# button" were wrong in ways that only surfaced as a write:
#
#   * Widest over the whole window is the toolbar's "Sign out" at (991,53),
#     which signs the operator out mid-verification.
#   * Widest inside the zone is the SESSION COMBO, 271pt across, against the
#     composer's 134. Clicking it opened the edit form, and the run went on to
#     measure a layout that was no longer the one it meant to.
#
# Lowest inside Essentials' own rect is the composer: it is the last control in
# setup_game_ui, and the islands below Essentials fall outside that rect.
island_button() {
  local ix _ _ _ iy ih
  # FOUR fields, four names. `island` emits x y width height and nothing else,
  # so a leading `_` in the read bound x and shifted the rest: the crop came out
  # `380x+212+320`, which found no button in the frame and reported the zone as
  # having no composer in it. The island was fine. The crop was rotated.
  read -r ix _ _ _ <<<"$(island "$1")"
  read -r iy ih <<<"$(island_on_band "$1")" || return 0
  grab "$FULL" "${ZONE_W}x${ih}+${ix}+${iy}"
  # The crop's origin goes INTO find-buttons, so the coordinates it reports are
  # window coordinates — which is what xdrv takes.
  #
  # LOWEST, because the composer is the last control in setup_game_ui. Widest
  # is the session combo at 271pt against the composer's 154, and clicking that
  # opens the edit form instead.
  python3 "$HERE/find-buttons.py" "$FULL" "$ix" "$iy" |
    sort -n -k8 | tail -1 | awk '{print $7, $8}'
}

# The session's state, read straight from the API.
#
# The guard that turns a mis-aimed click from a mutation into a report. Every
# button in this zone is either inert or writes to the server, and a harness
# that cannot say which it just pressed should not be pressing them.
session_state() {
  curl -s -X POST http://127.0.0.1:8099/api/api/v1/auth/login \
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
print(max(named, key=lambda g: g["id"])["state"] if named else "")
'
}

wheel() { "$X" wheel "$1" >/dev/null; sleep "$SETTLE"; }

# The invariant check, reported with the frame count it was taken over. A check
# that passed over zero frames has proved nothing, and a bare 0 reads the same
# as one that passed over four hundred.
report_chain() {
  local got
  got=$(chain_violations "$2")
  if [ "${got%%/*}" = "0" ]; then
    ok "$1" "$got frames"
  else
    bad "$1" "$got frames"
  fi
}

# The band strip above the first island: the operator can aim there and no body
# can claim it. Taken from the app's report of where the first island is.
gap_y() {
  local _ y
  read -r _ y _ _ <<<"$(island z.user)"
  printf '%s' "$((y - 8))"
}

# The column back at the top, by aiming at that strip and wheeling up until it
# stops changing. Not a fixed notch count: the column's range depends on the
# state, and 60 notches of scroll-up is a saturating gesture rather than a
# precise one.
# The gap strip is 16pt tall and the wheel gesture is smooth, so a notch aimed
# at it can overshoot into the band above the first island and read as a miss.
# Aim at the band's own top edge minus a point instead, and wheel in single
# notches until the offset stops changing.
reset_column() {
  "$X" mousemove 184 "$((ZY + 2))"; sleep 0.4
  # Loop until the offset is ZERO, not until it stops changing. "Stops changing"
  # is the wrong exit: with the column partway down, the strip above the first
  # island is where Essentials now sits, so the wheel is legitimately absorbed
  # by Essentials' body, the offset does not move, and the loop exited at 261
  # believing it had reset. The next case then parked its pointer on a body
  # that was already at its top, the body handed the gesture to the column, and
  # the case reported the column moving when the body should have taken it.
  for _ in $(seq 1 200); do
    [ "$(scroll_now)" = "0" ] && break
    "$X" wheel "-1" >/dev/null
    sleep 0.3
  done
  sleep 1
}

# An island's body back at its top, with the pointer parked over it so the
# column cannot take the gesture.
#
# Body offsets live in egui's memory under the island id. They survive every
# frame and every run against a running app, so without this a case inherits
# the previous case's scroll and then asserts against it.
rewind_body() {
  local _ y _ h
  read -r _ y _ h <<<"$(island "$1")"
  "$X" mousemove 184 "$((y + h / 2))"; sleep 0.4
  # To the offset being ZERO, not for a fixed number of notches. A fixed count
  # either under-scrolls (the case then starts partway down) or over-scrolls,
  # and when the body is at its top the extra notches go to the column, which
  # is how a "rewind" ended up moving the very thing it was clearing.
  for _ in $(seq 1 200); do
    [ "$(body_offset "$1")" = "0.0" ] && break
    "$X" wheel "-$NOTCH" >/dev/null
    sleep 0.3
  done
  sleep 1.2
}

# The invariant, over the frames after line $1: no frame moved the column
# while the island body under the POINTER still had room to scroll.
#
# Scoped to the body under the pointer, which is the whole difference between
# a check that means something and one that does not. "Some island had room" is
# almost always true — there are five islands and only one is under the cursor
# — so a checker phrased that way reported 15 violations per gesture against a
# chain that was behaving exactly as designed.
#
# `wheel_owner` on the zone line names the island whose body was under the
# pointer, and `body=` on that island's line carries its measurement. The walk
# is a state machine over the two because the app prints them together within a
# frame; correlating two greps across frames reported frames where the offset
# moved while an UNRELATED island had room.
chain_violations() {
  tail -n +"$1" "$LOG" | python3 "$HERE/check-chain.py"
}

# Reported with the frame count it was taken over. A check that passed over
# zero frames has proved nothing, and a bare 0 reads the same as one that passed
# over four hundred.
report_chain() {
  local got
  got=$(chain_violations "$2")
  if [ "${got%%/*}" = "0" ]; then
    ok "$1" "$got frames"
  else
    bad "$1" "$got frames"
  fi
}

# --- reach the known state ---------------------------------------------------
bash "$HERE/drive-to-session.sh" >/dev/null

# --- 0. a known starting point ----------------------------------------------
# Not cosmetic. Every case below reads the column offset, and an offset left
# over from a previous run makes the first assertion report a defect that is
# only the state it started in.
reset_column
check_eq "column starts at the top" 0 "$(scroll_now)"

# --- 1. a scrolling body takes the wheel, and the column stays put ----------
rewind_body z.essentials
m=$(mark); wheel "$NOTCH"
check_eq "wheel over a scrolling body leaves the column" 0 "$(scroll_now)"
grab "$SHOTS/scroll-01-body-scrolled.png" 400x640+0+0

# --- 2. the composer is reachable at the default window size ----------------
# Scroll the body out with the pointer parked on it, so the composer button is
# on screen with the column still at the top and Essentials still the island
# under the pointer.
rewind_body z.essentials
check_eq "the column is at the top before the body is scrolled" 0 "$(scroll_now)"
m=$(mark)

# Scroll the body to its END, and stop when the app says the offset has stopped
# moving. The composer is the last control in setup_game_ui, so at the bottom of
# the body is where it is, and "the body is fully scrolled" is a number the app
# prints rather than a judgement about a screenshot.
#
# Three earlier versions of this case were each wrong in a way that produced a
# confident answer:
#
#   * A fixed notch count. The body absorbs about five and hands the rest to the
#     column, so twelve carried Essentials off the top of the band.
#   * "Stop as soon as a button appears in Essentials' rect." "copy" is a button
#     inside that rect and sits higher than the composer, so the loop stopped on
#     notch one and clicked `copy`.
#   * "The lowest button in the island." Same answer for the same reason — at a
#     partial scroll the lowest button on screen is not the last one in the body.
#
# So the scroll is driven by the offset and the button is read afterwards, at a
# position where the whole body is on screen.
#
# `chain_violations` over the same frames is the other half of the claim: the
# body took these notches, and at no frame did the column take one as well.
notches=0
prev_offset=$(body_offset z.essentials)
for _ in $(seq 1 40); do
  wheel "$NOTCH"
  notches=$((notches + 1))
  now=$(body_offset z.essentials)
  [ "$now" = "$prev_offset" ] && break
  prev_offset=$now
done
check_gt "the body scrolled under its own wheel" 0 "$prev_offset"
grab "$SHOTS/scroll-02-body-scrolled-to-end.png" 400x640+0+0

coords=$(island_button z.essentials)
if [ -z "$coords" ]; then
  bad "the composer button is on screen at the body's end" "no button in Essentials"
else
  ok "the composer button is on screen at the body's end" "$coords after $notches notches"
fi
report_chain "no frame moved both while reaching it" "$m"

# --- 3. and it opens, and its ✕ closes --------------------------------------
if [ -n "$coords" ]; then
  state_before=$(session_state)
  # shellcheck disable=SC2086
  set -- $coords
  "$X" click "$1" "$2"; sleep 4
  grab "$FULL"
  if modal_up; then
    ok "composer opened" "$(probe 200 60)"
  else
    bad "composer opened" "$(probe 200 60)"
  fi
  grab "$SHOTS/scroll-03-composer-open.png"
  check_eq "opening the composer changed no session state" "$state_before" "$(session_state)"

  # The ✕ by its own position, read off the open modal's own title row rather
  # than remembered: the modal is 880 wide and centred in 1040, so its right
  # edge is at 960 and the ✕ sits 21pt inside that.
  "$X" click 937 54; sleep 2
  grab "$FULL"
  if modal_up; then
    bad "composer closes" "title band still lit"
  else
    ok "composer closes" "$(probe 200 60)"
  fi
  grab "$SHOTS/scroll-04-composer-closed.png"
  check_eq "closing the composer changed no session state" "$state_before" "$(session_state)"
fi

# --- 4. the column scrolls when the pointer is over the band, not a body ----
reset_column
"$X" mousemove 184 "$(gap_y)"; sleep 0.4
m=$(mark); wheel "$NOTCH"
check_gt "wheel over a gap moves the column" 100 "$(scroll_now)"
report_chain "no frame moved both over a gap" "$m"

# --- 5. a body with no room chains to the column ----------------------------
# Operator is 140pt holding about 98pt of content, so it cannot scroll and
# must not swallow the gesture.
reset_column
m=$(mark)
read -r _ user_y _ user_h <<<"$(island z.user)"
"$X" mousemove 184 "$((user_y + user_h / 2))"; sleep 0.5
wheel "$NOTCH"
check_gt "wheel over a fitting body moves the column" 100 "$(scroll_now)"
report_chain "no frame moved both over a fitting body" "$m"

# --- 6. and the map never moves the column ----------------------------------
after=$(scroll_now)
"$X" mousemove 700 400; sleep 0.5
m=$(mark); wheel "$NOTCH"
check_eq "wheel over the map leaves the column" "$after" "$(scroll_now)"
report_chain "no frame moved both over the map" "$m"

# --- 7. and the body takes the wheel again once the column is at the top ----
reset_column
rewind_body z.essentials
m=$(mark); wheel "$NOTCH"
check_eq "body takes the wheel again at the top" 0 "$(scroll_now)"
report_chain "no frame moved both at the top" "$m"

grab "$SHOTS/scroll-05-final.png"
echo
[ "$fails" -eq 0 ] && echo "all checks passed" || echo "$fails check(s) failed"
exit "$fails"