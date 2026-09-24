#!/usr/bin/env bash
# Run the egui command-center application.
# Usage: scripts/run-egui-window.sh [cargo-args...]
set -euo pipefail
exec cargo run "$@"
