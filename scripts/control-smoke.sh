#!/usr/bin/env bash
# The control plane's real-binary smoke test: a CLI command moves the map the
# operator is looking at, and the event stream names the client that asked.
#
#   bash scripts/control-smoke.sh
#
# This is an acceptance test and not a unit test, because "the camera moved" is
# only interesting if the SHELL moved it: the hub is written by the order
# thread from the camera the shell renders, so a changed longitude here means
# the desktop's own `move_camera` ran, not that a server accepted a request.
#
# It opens a console window on the current display for about fifteen seconds.
# Anything already running from THIS checkout is killed first (the same rule as
# scripts/gpui-capture.sh: never another worktree's console).
#
# Exit code 0 means every check passed; 1 means at least one did not, and the
# failing checks are printed with the numbers they saw.
set -uo pipefail
cd "$(dirname "$0")/.."

PORT="${TFG_SMOKE_PORT:-8791}"
TOKEN="${TFG_SMOKE_TOKEN:-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef}"
OUT="${TFG_SMOKE_OUT:-/tmp/opencode/control-smoke}"
APP=./target/debug/tfg
CLI=./target/debug/tfgctl
SERVER="http://127.0.0.1:$PORT"
# A throwaway device table: pairing against the operator's own would leave a
# device behind on the machine that ran the smoke.
DEVICES="$OUT/devices/control-devices.json"

mkdir -p "$OUT"
for binary in "$APP" "$CLI"; do
    if [ ! -x "$binary" ]; then
        echo "REFUSING: $binary is not built. Run: cargo build --bin tfg --bin tfgctl" >&2
        exit 3
    fi
done

# Only this checkout's stale app. Another agent's window is not ours to close.
for stale in $(pgrep -x tfg 2>/dev/null); do
    exe=$(readlink "/proc/$stale/exe" 2>/dev/null || true)
    case "$exe" in
        "$PWD"/target/debug/tfg) kill "$stale" 2>/dev/null || true ;;
    esac
done
sleep 0.5

APP_PID=""
WATCH_PID=""
cleanup() {
    for pid in $WATCH_PID $APP_PID; do
        [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
    done
}
trap cleanup EXIT INT TERM

TFG_CONTROL_LISTEN="127.0.0.1:$PORT" TFG_CONTROL_TOKEN="$TOKEN" \
    TFG_CONTROL_DEVICES="$DEVICES" \
    "$APP" >"$OUT/app.log" 2>&1 &
APP_PID=$!
export TFG_CONTROL_TOKEN="$TOKEN"

# The server binds before the window draws, so wait for the route rather than
# for a fixed number of seconds.
UP=0
for _ in $(seq 1 30); do
    if "$CLI" --server "$SERVER" hello >/dev/null 2>&1; then
        UP=1
        break
    fi
    sleep 1
done
if [ "$UP" -ne 1 ]; then
    echo "REFUSING: the control server did not answer on $SERVER in 30s." >&2
    tail -20 "$OUT/app.log" >&2
    exit 3
fi

"$CLI" --server "$SERVER" camera get --json >"$OUT/camera-before.json" 2>"$OUT/get-before.err"
"$CLI" --server "$SERVER" watch --json >"$OUT/watch.log" 2>&1 &
WATCH_PID=$!
sleep 1

"$CLI" --server "$SERVER" camera left --json >"$OUT/left.json" 2>&1 || true
"$CLI" --server "$SERVER" camera zoom-in --json >"$OUT/zoom.json" 2>&1 || true
sleep 2
"$CLI" --server "$SERVER" camera get --json >"$OUT/camera-after.json" 2>"$OUT/get-after.err"
"$CLI" --server "$SERVER" status >"$OUT/status.txt" 2>&1 || true

# Pairing against the CONSOLE, not the headless host: the desktop is where an
# operator will actually read a code out loud, and the token it mints has to
# work against the same grant table the operator token is checked by.
python3 - "$SERVER" "$TOKEN" "$OUT" <<'PY'
import json, sys, urllib.error, urllib.request

url, operator, out = sys.argv[1], sys.argv[2], sys.argv[3]

def call(method, path, token=None, body=None):
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(url + path, data=data, method=method)
    if token:
        request.add_header("Authorization", "Bearer " + token)
    if data:
        request.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(request) as response:
            return response.status, json.loads(response.read() or b"{}")
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read() or b"{}")

def save(name, value):
    with open(f"{out}/{name}", "w") as handle:
        json.dump(value, handle)

_, offer = call("POST", "/v1/pairing", operator, {"scopes": ["observe", "control_view"]})
status, paired = call("POST", "/v1/pair", None, {"code": offer.get("code", ""), "label": "smoke phone"})
save("console-pair.json", {"status": status, "token": paired.get("token", ""),
                           "device": paired.get("device", {})})
token = paired.get("token", "")
save("console-read.json", {"status": call("GET", "/v1/views/main/camera", token)[0],
                           "body": call("GET", "/v1/views/main/camera", token)[1]})
call("DELETE", "/v1/devices/" + paired.get("device", {}).get("id", "?"), operator)
save("console-revoked.json", {"status": call("GET", "/v1/state", token)[0]})
PY

python3 - "$OUT" "$DEVICES" <<'PY'
import json
import pathlib
import sys

out = pathlib.Path(sys.argv[1])
devices_path = pathlib.Path(sys.argv[2])
checks = []

def check(name, ok, detail=""):
    checks.append((name, ok, detail))

def read(name):
    try:
        return json.loads((out / name).read_text())
    except Exception as error:  # noqa: BLE001 - report, do not crash
        return {"_error": str(error)}

before = read("camera-before.json")
after = read("camera-after.json")
left = read("left.json")
zoom = read("zoom.json")

check("the CLI was accepted", left.get("status") == "accepted", json.dumps(left))
check("zoom was accepted", zoom.get("status") == "accepted", json.dumps(zoom))

try:
    check(
        "a nudge moves the camera west",
        after["center"]["longitude"] < before["center"]["longitude"],
        f'{before["center"]["longitude"]} -> {after["center"]["longitude"]}',
    )
except (KeyError, TypeError) as error:
    check("a nudge moves the camera west", False, f"unreadable camera: {error}")

try:
    check(
        "a zoom step raises the zoom",
        abs((after["zoom"] - before["zoom"]) - 1.0) < 1e-6,
        f'{before["zoom"]} -> {after["zoom"]}',
    )
except (KeyError, TypeError) as error:
    check("a zoom step raises the zoom", False, f"unreadable camera: {error}")

events = []
for line in (out / "watch.log").read_text().splitlines():
    line = line.strip()
    if line.startswith("{"):
        try:
            events.append(json.loads(line))
        except ValueError:
            pass

check("the watcher got a snapshot", any(e.get("type") == "snapshot" for e in events),
      f"{len(events)} events")

changed = [e for e in events if e.get("type") == "camera.changed"]
check("the watcher saw the camera change", bool(changed), f"{len(changed)} camera events")
check(
    "the change names the client that asked",
    any(e.get("source") == "tfgctl" for e in changed),
    ", ".join(sorted({str(e.get("source")) for e in changed})),
)
check(
    "the change carries the command id",
    any(e.get("command_id") for e in changed),
    "",
)

console_pair = read("console-pair.json")
console_read = read("console-read.json")
console_revoked = read("console-revoked.json")
check(
    "a phone can pair with the console",
    console_pair.get("status") == 200 and console_pair.get("device", {}).get("id"),
    json.dumps(console_pair.get("device", {}))[:120],
)
check(
    "the paired device reads the console's camera",
    console_read.get("status") == 200 and "camera" in console_read.get("body", {}),
    json.dumps(console_read.get("body", {}))[:120],
)
check("a revoked device is refused by the console", console_revoked.get("status") == 401)
mode = oct(devices_path.stat().st_mode & 0o777) if devices_path.exists() else "missing"
text = devices_path.read_text() if devices_path.exists() else ""
check(
    "the console wrote its table where TFG_CONTROL_DEVICES said, at 0600, with no token in it",
    mode == "0o600" and "token" not in text.replace("token_hash", ""),
    mode,
)

failed = 0
for name, ok, detail in checks:
    if not ok:
        failed += 1
    print(f'{"PASS" if ok else "FAIL"}  {name}' + (f'  ({detail})' if detail else ""))
print()
print(f'{len(checks) - failed}/{len(checks)} checks passed; artifacts in {out}')
sys.exit(1 if failed else 0)
PY
STATUS=$?

echo
echo "== app log (control lines)"
grep -i control "$OUT/app.log" || true
exit "$STATUS"
