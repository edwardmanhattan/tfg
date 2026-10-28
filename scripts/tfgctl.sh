#!/usr/bin/env bash
# Run the TFG control CLI (tfgctl) against the local control host.
#
#   TFG_CONTROL_URL   host base URL (tfgctl reads this for its --server default)
#   TFG_CONTROL_TOKEN bearer token
#
# Usage: scripts/tfgctl.sh <tfgctl-args...>
#   scripts/tfgctl.sh status
#   scripts/tfgctl.sh panel scenario on
#   scripts/tfgctl.sh hello --json
set -euo pipefail

# Detect primary local IP at call time (falling back to 127.0.0.1 if disconnected)
_HOST_IP="$(ip route get 1.1.1.1 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="src") print $(i+1)}' || true)"
[ -z "$_HOST_IP" ] && _HOST_IP="$(ip -4 -o addr show scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | head -n1 || true)"
[ -z "$_HOST_IP" ] && _HOST_IP="127.0.0.1"

: "${TFG_CONTROL_URL:=http://${_HOST_IP}:8787}"
: "${TFG_CONTROL_TOKEN:=ff6a090b0e022424aa8439bf154b67c096fd79ceff6e2489f36f0f8a1e0ef452}"

export TFG_CONTROL_URL TFG_CONTROL_TOKEN

exec ~/Work/Crossnet/tfg/target/debug/tfgctl "$@"
