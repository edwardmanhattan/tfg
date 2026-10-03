#!/usr/bin/env bash
# Capture the app window with a coordinate grid burned in.
#
# Reading a click target off a scaled screenshot costs a round every time, and a
# miss lands on the backdrop instead of the control. The grid is drawn in
# window coordinates and labelled every 100pt, so the numbers in the picture are
# the numbers to pass to xdrv.
#
#   shot.sh out.png                          whole window, grid every 25
#   shot.sh out.png 200 400 300 200 10       crop at x y w h, grid every 10
set -euo pipefail
OUT=$1
ROOT=${X11_DISPLAY_NUM:-:99}

draw_grid() {
  local w=$1 h=$2 step=$3 out=""
  local x
  for ((x = 0; x <= w; x += step)); do out+=" line $x,0 $x,$h"; done
  local y
  for ((y = 0; y <= h; y += step)); do out+=" line 0,$y $w,$y"; done
  for ((x = 0; x <= w; x += step * 4)); do out+=" text $((x + 2)),12 '$x'"; done
  for ((y = 0; y <= h; y += step * 4)); do out+=" text 2,$((y + 11)) '$y'"; done
  printf '%s' "$out"
}

if [ $# -ge 5 ]; then
  X=$2; Y=$3; W=$4; H=$5; STEP=${6:-25}
  convert -size "${W}x${H}" xc:none -stroke '#ff2d9544' -strokewidth 1 \
    -draw "$(draw_grid "$W" "$H" "$STEP")" \
    -fill '#ff2d95dd' -stroke none -pointsize 10 \
    -draw "$(draw_grid "$W" "$H" "$STEP")" png:/tmp/opencode/grid.png
  DISPLAY="$ROOT" import -window root -crop "${W}x${H}+${X}+${Y}" +repage /tmp/opencode/raw.png
  convert /tmp/opencode/raw.png /tmp/opencode/grid.png -compose over -composite "$OUT"
else
  W=${TFG_WIN_W:-1040}; H=${TFG_WIN_H:-640}; STEP=${2:-50}
  convert -size "${W}x${H}" xc:none -stroke '#ff2d9544' -strokewidth 1 \
    -draw "$(draw_grid "$W" "$H" "$STEP")" \
    -fill '#ff2d95dd' -stroke none -pointsize 10 \
    -draw "$(draw_grid "$W" "$H" "$STEP")" png:/tmp/opencode/grid.png
  DISPLAY="$ROOT" import -window root -crop "${W}x${H}+0+0" +repage /tmp/opencode/raw.png
  convert /tmp/opencode/raw.png /tmp/opencode/grid.png -compose over -composite "$OUT"
fi
echo "$OUT"