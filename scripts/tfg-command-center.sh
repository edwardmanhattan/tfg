#!/usr/bin/env bash
# Run the console as the command center: it follows a control host.
#
# Console-as-command-center (src/main.rs --mode command-center ->
# src/command_center.rs: CommandCenterConfig::from_env).
#
#   TFG_CONTROL_SERVER  host the display follows (Follow arm)
#   TFG_CONTROL_TOKEN   bearer token for that host
#   TFG_DISPLAY_ID      stable display id (default "command-center")
#   TFG_DISPLAY_LABEL   human label (default "command center")
#
# Usage: scripts/tfg-command-center.sh
set -euo pipefail

# Detect primary local IP at call time (falling back to 127.0.0.1 if disconnected)
_HOST_IP="$(ip route get 1.1.1.1 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="src") print $(i+1)}' || true)"
[ -z "$_HOST_IP" ] && _HOST_IP="$(ip -4 -o addr show scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | head -n1 || true)"
[ -z "$_HOST_IP" ] && _HOST_IP="127.0.0.1"

: "${TFG_CONTROL_SERVER:=http://${_HOST_IP}:8787}"
: "${TFG_CONTROL_TOKEN:=ff6a090b0e022424aa8439bf154b67c096fd79ceff6e2489f36f0f8a1e0ef452}"
: "${TFG_DISPLAY_ID:=command-center}"
: "${TFG_DISPLAY_LABEL:=command center}"

export TFG_CONTROL_SERVER TFG_CONTROL_TOKEN TFG_DISPLAY_ID TFG_DISPLAY_LABEL

exec cargo run --bin tfg -- --mode command-center
