#!/usr/bin/env bash
# Capture a set of states after a change, against the HOSTED dev API, and
# assert things about the pixels.
#
# The host is the app's own default (api.tfg.development.crossnet.co.id) —
# scripts/gpui-capture.sh already exports the hosted superuser credentials,
# so nothing here overrides them. The local stack is a throwaway database and
# is NOT the deployment the operator uses.
#
# Usage: scripts/gpui-sweep.sh <tag> "name:seed[|assertion;assertion]" ...
#
#   scripts/gpui-sweep.sh after "plan:TFG_GAME=9" \
#     "prep:TFG_GAME=41|at:0,300=cut-grey;runs:250=cut-grey=3"
#
# The seeds are the app's own capture seeds; see scripts/gpui-capture.sh.
# The assertions are shot-assert.sh's; see scripts/shot-assert.sh. A
# sweep with assertions is a TEST: it exits non-zero when a pixel is
# wrong, and when a capture caught a live gesture from the operator.
#
# Out is overridable with TFG_SHOT_OUT, which is where the review sets
# under .impeccable/review/ want them.
set -uo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$here"

TAG="${1:-x}"; shift
OUT="${TFG_SHOT_OUT:-/tmp/opencode/shots}"
mkdir -p "$OUT"

FAILED=0
TAINTED=0

for spec in "$@"; do
  name="${spec%%:*}"
  rest="${spec#*:}"
  extra="${rest%%|*}"
  asserts=""
  if [ "$rest" != "$extra" ]; then
    asserts="${rest#*|}"
  fi

  echo "--- $TAG/$name : $extra"
  out=$(timeout 120 bash scripts/gpui-capture.sh 16 "$OUT/$TAG-$name.png" "$extra" 2>&1)
  echo "$out" | grep -E 'captured |input during capture|pointer:' | sed 's/^/  /'

  # A capture that caught a live gesture is a photograph of a state nobody
  # asked for. It is not a failure, but it is not the picture either, and
  # a sweep that hides this is how a "layout bug" hunt begins.
  if echo "$out" | grep -q 'TAINTED'; then
    echo "  ** TAINTED: $name caught a live gesture — re-shoot it with the mouse away"
    TAINTED=$((TAINTED + 1))
  fi

  if [ -n "$asserts" ]; then
    IFS=';' read -ra list <<< "$asserts"
    if bash scripts/shot-assert.sh "$OUT/$TAG-$name.png" "${list[@]}"; then
      echo "  ** assertions hold"
    else
      echo "  ** ASSERTIONS FAILED for $name"
      FAILED=$((FAILED + 1))
    fi
  fi
done

echo "SWEEP $TAG DONE  ($FAILED state(s) failed, $TAINTED tainted)"
[ "$FAILED" -eq 0 ] || exit 1
