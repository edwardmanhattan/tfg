#!/usr/bin/env bash
# A private display for tfg, with no root and no compositor.
#
# The real screen cannot be photographed unattended: grim blocks on an idle
# window, a fresh window is only on top for seconds, and a lock screen or the
# user's own terminal will happily be what gets captured. A virtual X server
# has none of those problems — `import -window root` always gets the app — and
# XTEST drives it without needing the compositor at all.
#
# xorg-server and libglvnd are not installed and there is no root, so they are
# fetched by URL and unpacked into a local prefix, the same trick stack.sh uses
# for Postgres. The server's own deps (libXfont2, libpixman, libxshmfence,
# libxkbfile) are already present because XWayland needs them.
set -euo pipefail

ROOT=${X11_ROOT:-/tmp/opencode/x11}
PKGS=${X11_PKGS:-/tmp/opencode/pkgs}
DISP=${X11_DISPLAY_NUM:-:99}
GEOM=${X11_GEOM:-1920x1200x24}
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

mkdir -p "$PKGS" "$ROOT"
cd "$PKGS"

# Xvfb is its own package on Arch; xorg-server alone ships Xorg.
for p in xorg-server-xvfb xorg-server-common libglvnd; do
  url=$(pacman -Sp "$p" | tail -1)
  f=$(basename "$url")
  [ -f "$f" ] || curl -sSLo "$f" "$url"
  tar --zstd -xf "$f" -C "$ROOT"
done

pkill -f "Xvfb $DISP" 2>/dev/null || true
sleep 1
# xkbdir must NOT be overridden: the unpacked xorg-server-common has no keymaps
# and the server dies with "Failed to activate virtual core keyboard".
LD_LIBRARY_PATH="$ROOT/usr/lib" setsid "$ROOT/usr/bin/Xvfb" "$DISP" \
  -screen 0 "$GEOM" -nolisten tcp >/tmp/opencode/xvfb.log 2>&1 < /dev/null &
sleep 3
pgrep -f "Xvfb $DISP" >/dev/null || { tail -5 /tmp/opencode/xvfb.log; exit 1; }

cc -O2 -g -o /tmp/opencode/xdrv "$(dirname "${BASH_SOURCE[0]}")/xdrv.c" \
  -lXtst -lX11 -lXext

echo "display $DISP up, driver at /tmp/opencode/xdrv"
echo "  DISPLAY=$DISP /tmp/opencode/xdrv place 0 0 1600 1100"
echo "  DISPLAY=$DISP import -window root shot.png"
