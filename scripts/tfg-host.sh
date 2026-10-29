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
# This script runs the HOST. The wall a phone remotes is a separate process:
#
#   scripts/tfg-command-center.sh
#
# `scripts/tfg-host.sh --mode command-center` delegates there, because a host
# cannot also be a command-center display of itself.
#
# Usage: scripts/tfg-host.sh [--mode command-center]
set -euo pipefail

# A host and a command center are two roles, not two flags on one process.
# If the operator asked for the wall here, run the wall script instead of
# silently dropping the argument and opening the usual console.
_delegate_command_center() {
  echo "tfg-host.sh: --mode command-center is the wall, not the host; delegating to tfg-command-center.sh" >&2
  local script_dir
  script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
  exec "$script_dir/tfg-command-center.sh"
}

case "${1:-}" in
  --mode)
    if [[ "${2:-}" == "command-center" ]]; then
      _delegate_command_center
    fi
    ;;
  --mode=command-center)
    _delegate_command_center
    ;;
esac

: "${TFG_CONTROL_LISTEN:=0.0.0.0:8787}"
: "${TFG_CONTROL_TOKEN:=ff6a090b0e022424aa8439bf154b67c096fd79ceff6e2489f36f0f8a1e0ef452}"
: "${TFG_MINOS_HOST:=https://api.tfg.development.crossnet.co.id/api/v1}"
: "${TFG_SIGNED_OUT:=0}"
: "${TFG_API_USER:=superuser}"
: "${TFG_API_PASS:=nasipadangudawandi}"

export TFG_CONTROL_LISTEN TFG_CONTROL_TOKEN TFG_MINOS_HOST TFG_SIGNED_OUT TFG_API_USER TFG_API_PASS

exec cargo run --bin tfg -- "$@"
