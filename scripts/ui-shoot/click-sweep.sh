#!/usr/bin/env bash
# Click-sweep a region and stop at the first point that changes the app's state,
# detected by one probe pixel rather than by reading a screenshot.
#
# A screenshot's on-screen scale is not knowable from the image, so every
# coordinate estimated by eye is suspect. Clicking is not: if the click lands,
# the state changes, and a single probe pixel is an unambiguous oracle.
#
#   click-sweep.sh x0 y0 x1 y1 step PROBE_X PROBE_Y REF_X REF_Y
set -euo pipefail
X0=$1; Y0=$2; X1=$3; Y1=$4; STEP=$5
PX=$6; PY=$7; RX=$8; RY=$9
X=/tmp/opencode/xdrv

sample() { DISPLAY=:99 import -window root -crop "1x1+$1+$2" +repage txt:- 2>/dev/null | tail -1 | grep -o '#[0-9A-F]*' | head -1; }

ref=$(sample "$RX" "$RY")
echo "reference pixel at $RX,$RY is $ref"

y=$Y0
while [ "$y" -le "$Y1" ]; do
  x=$X0
  while [ "$x" -le "$X1" ]; do
    $X mousemove "$x" "$y" >/dev/null
    $X click "$x" "$y" >/dev/null
    sleep 0.35
    v=$(sample "$PX" "$PY")
    if [ "$v" != "$ref" ]; then
      echo "CHANGED at $x,$y -> probe $v (was $ref)"
      exit 0
    fi
    x=$((x + STEP))
  done
  y=$((y + STEP))
done
echo "no change anywhere in $X0,$Y0 - $X1,$Y1"