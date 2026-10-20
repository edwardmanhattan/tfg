#!/usr/bin/env bash
# Capture the RUNNING GPUI console's window. Does not launch anything.
#
#   gpu-capture.sh [OUT]
#
# The console is a Vulkan app (blade), so it needs a GPU-backed display:
# Xvfb has no DRI3 and the surface creation fails there. The compositor is
# the display; grim copies the output and the window rect comes from
# hyprctl, so the crop is the app and not the desktop.
set -euo pipefail
OUT=${1:-/tmp/opencode/shots/gpui.png}
T=8

RECT=$(timeout "$T" hyprctl -j clients | python3 -c "
import json,sys
for c in json.load(sys.stdin):
    # An XWayland window reports 'class' as 'instance\0class', so match on
    # the prefix rather than the whole string.
    if c.get('class','').split('\x00')[0].startswith('tfg-gpui'):
        a,s=c['at'],c['size']
        print(a[0],a[1],s[0],s[1],c.get('workspace',{}).get('name','')); break
else:
    print('')
" || true)
if [ -z "$RECT" ]; then
  echo "no tfg-gpui window" >&2
  exit 4
fi
read -r X Y W H WWS <<< "$RECT"
ACTIVE=$(timeout "$T" hyprctl -j activeworkspace | python3 -c 'import json,sys; print(json.load(sys.stdin)["name"])' || true)
if [ -n "$WWS" ] && [ "$WWS" != "$ACTIVE" ]; then
  echo "REFUSING: the window is on workspace '$WWS', the active one is '$ACTIVE'." >&2
  exit 5
fi
echo "window rect (logical): $X $Y $W $H  on workspace '$WWS'"

PX=$(python3 -c "print(round($X*1.25))"); PY=$(python3 -c "print(round($Y*1.25))")
PW=$(python3 -c "print(round($W*1.25))"); PH=$(python3 -c "print(round($H*1.25))")
timeout "$T" grim -o eDP-1 /tmp/opencode/_full.png
timeout "$T" magick /tmp/opencode/_full.png -crop "${PW}x${PH}+${PX}+${PY}" +repage "$OUT"
echo "captured $OUT"
