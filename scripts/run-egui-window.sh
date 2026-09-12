#!/usr/bin/env bash
# Run the egui command-center prototype with the spike's required env.
# Usage: scripts/run-egui-window.sh [cargo-args...]
set -euo pipefail
exec cargo run --example egui_window "$@"
