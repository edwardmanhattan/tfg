#!/usr/bin/env bash
# Capture the three fusion variants from the throwaway mock.
# Served over HTTP because headless Chromium's file:// loader rejects a
# query string, and the mock needs ?v= to pick one variant.
# Usage: bash shoot.sh [outdir]
set -euo pipefail
DIR="$(cd "$(dirname "$0")" && pwd)"
OUT="${1:-$DIR/shots}"
PORT="${PORT:-8731}"
mkdir -p "$OUT"

cd "$DIR"
python3 -m http.server "$PORT" --bind 127.0.0.1 >/tmp/glass-mock-http.log 2>&1 &
SRV=$!
trap 'kill "$SRV" 2>/dev/null || true' EXIT
sleep 1

for i in 0 1 2; do
  chromium --headless --disable-gpu --no-sandbox --hide-scrollbars \
    --force-device-scale-factor=1 \
    --window-size=1440,900 \
    --virtual-time-budget=6000 \
    --screenshot="$OUT/variant-$i.png" \
    "http://127.0.0.1:$PORT/index.html?v=$i" >/dev/null 2>&1
done

ls -l "$OUT"/variant-*.png
