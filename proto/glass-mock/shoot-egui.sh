#!/usr/bin/env bash
# Capture the four prototype shots in one go and report their dimensions.
#
#   bash proto/glass-mock/shoot-egui.sh
#
# The dimension printout matters: the console's panels are positioned from
# `screen.right()`, so a capture that is not the mock's viewBox silently
# overlaps the Settings panel onto the Session panel. THAT HAPPENED: a 480-wide
# capture put Settings on top of Session, and it read as a layout bug when it
# was a harness bug.
#
# The window manager is not running, and egui's wgpu backend cannot present
# without DRI3, which this Xvfb does not have. `--shoot` therefore succeeds
# intermittently: when the Vulkan context comes up it takes 3 seconds, and when
# it does not the binary sleeps in epoll_wait until the timeout kills it. The
# 200-second timeout below is sized for the good case and long enough to fail
# cleanly in the bad one.
set -uo pipefail

BIN=proto/p5-epaint/target/debug/p5-epaint
SHOTS=proto/glass-mock/shots

export LIBGL_ALWAYS_SOFTWARE=1
export DISPLAY=:99

pgrep -x Xvfb >/dev/null || bash scripts/ui-shoot/xvfb-up.sh | tail -1
for p in $(pgrep -x p5-epaint); do kill -9 "$p" 2>/dev/null; done
sleep 1

shoot() {
  local tag="$1"; shift
  rm -f "$SHOTS/glass-$tag.png"
  timeout 200 "$BIN" "$@" --shoot --name "$tag" >/tmp/opencode/shoot-$tag.log 2>&1
  local rc=$?
  python3 - "$SHOTS/glass-$tag.png" "$tag" "$rc" <<'PY'
import os, struct, sys
p, tag, rc = sys.argv[1], sys.argv[2], int(sys.argv[3])
log = open('/tmp/opencode/shoot-' + tag + '.log').read()
if not os.path.exists(p):
    why = "wgpu could not present (no DRI3)" if "DRI3" in log else f"timed out, rc={rc}"
    print(f"  {tag:<16} MISSING  ({why})")
else:
    d = open(p, "rb").read()
    w, h = struct.unpack(">II", d[16:24])
    note = "" if "components" in tag else ("" if (w, h) == (1440, 900) else f"  (console wants 1440x900)")
    print(f"  {tag:<16} {w}x{h}{note}")
PY
}

echo "console:"
shoot frame-rest --glass-b
shoot frame-polygon --glass-b --glass-b-shadow
echo "component sheet:"
shoot comp-rest --glass-b --components
shoot comp-focus --glass-b --components --field-focus
