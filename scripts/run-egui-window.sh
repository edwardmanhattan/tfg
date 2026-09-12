#!/usr/bin/env bash
# Run the egui command-center prototype with the spike's required env.
# Usage: scripts/run-egui-window.sh [cargo-args...]
set -euo pipefail
SCRIPTS="$(cd "$(dirname "$0")" && pwd)"
"$SCRIPTS/build-libuv-workaround.sh"
REPO="$(cd "$SCRIPTS/.." && pwd)"
export MLN_PRECOMPILE=1
export LD_PRELOAD="$REPO/target/libuv-1.44.2/build/libuv.so.1"
exec cargo run --example egui_window "$@"
