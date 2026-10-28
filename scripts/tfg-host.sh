#!/usr/bin/env bash
# Run the console itself as the TFG control-plane host (publisher of camera state).
#
# Console-as-host: the console owns the shared camera and publishes state.
# The endpoint comes from the same env vars the headless host uses
# (src/main.rs: control_endpoint_from_env -> ControlEndpoint::Host).
#
#   TFG_CONTROL_LISTEN  host bind (Host arm)
#   TFG_CONTROL_TOKEN   bearer token for that host
#   TFG_MINOS_HOST      Minos backend
#   TFG_SIGNED_OUT      force the signed-out card (1 = yes)
#   TFG_API_USER/PASS   Minos backend credentials
#
# Usage: scripts/tfg-host.sh
set -euo pipefail

: "${TFG_CONTROL_LISTEN:=0.0.0.0:8787}"
: "${TFG_CONTROL_TOKEN:=ff6a090b0e022424aa8439bf154b67c096fd79ceff6e2489f36f0f8a1e0ef452}"
: "${TFG_MINOS_HOST:=https://api.tfg.development.crossnet.co.id/api/v1}"
: "${TFG_SIGNED_OUT:=0}"
: "${TFG_API_USER:=superuser}"
: "${TFG_API_PASS:=nasipadangudawandi}"

export TFG_CONTROL_LISTEN TFG_CONTROL_TOKEN TFG_MINOS_HOST TFG_SIGNED_OUT TFG_API_USER TFG_API_PASS

exec cargo run --bin tfg
