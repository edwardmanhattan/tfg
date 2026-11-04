#!/usr/bin/env bash
# Run the console from its build.
#
# The name is historical: the console cut over to GPUI (ADR-0020) and this
# script's job no longer knows anything about the toolkit. What it changes is
# that it no longer runs `cargo run` — the build is the operator's call
# (`cargo build --bin tfg`), and what starts is the binary that build made.
#
# Usage: scripts/run-egui-window.sh [console-args...]
#   scripts/run-egui-window.sh
#   scripts/run-egui-window.sh --mode command-center
set -euo pipefail

_SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tfg-bin.sh
. "$_SCRIPT_DIR/tfg-bin.sh"

BIN="$(tfg_bin tfg)"
exec "$BIN" "$@"
