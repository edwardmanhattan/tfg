#!/usr/bin/env bash
# The headless control plane's acceptance test: `tfg-server` owns a camera with
# no window, `tfgctl` moves it, devices pair, rotate and are revoked, and the
# table survives a restart.
#
#   bash scripts/control-server-smoke.sh
#
# The sibling of scripts/control-smoke.sh, and deliberately the opposite shape:
# that one proves the DESKTOP moved its own map, this one proves a machine with
# no display can hold the state at all. Nothing here opens a window or needs a
# display server, so it runs over ssh.
#
# What it cannot prove: that anything was drawn. A headless host has no tiles
# and no picture; two clients agree on a camera, not on pixels.
#
# Exit code 0 means every check passed; 1 means at least one did not, and the
# failing checks are printed with the numbers they saw.
set -uo pipefail
cd "$(dirname "$0")/.."

PORT="${TFG_SERVER_SMOKE_PORT:-8792}"
TOKEN="${TFG_SERVER_SMOKE_TOKEN:-efefefefefefefefefefefefefefefefefefefefefefefefefefefefefefefef}"
OUT="${TFG_SERVER_SMOKE_OUT:-/tmp/opencode/control-server-smoke}"
SERVER=./target/debug/tfg-server
CLI=./target/debug/tfgctl
URL="http://127.0.0.1:$PORT"

for binary in "$SERVER" "$CLI"; do
    if [ ! -x "$binary" ]; then
        echo "REFUSING: $binary is not built. Run: cargo build --bin tfg-server --bin tfgctl" >&2
        exit 3
    fi
done

rm -rf "$OUT"
mkdir -p "$OUT/devices"

SERVER_PID=""
WATCH_PID=""
DISPLAY_PID=""
cleanup() {
    for pid in $DISPLAY_PID $WATCH_PID $SERVER_PID; do
        [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
    done
}
trap cleanup EXIT INT TERM

start_server() {
    "$SERVER" --listen "127.0.0.1:$PORT" --token "$TOKEN" \
        --devices "$OUT/devices/control-devices.json" >>"$OUT/server.log" 2>&1 &
    SERVER_PID=$!
    for _ in $(seq 1 30); do
        if "$CLI" --server "$URL" hello >/dev/null 2>&1; then
            return 0
        fi
        sleep 1
    done
    echo "REFUSING: the headless server did not answer on $URL in 30s." >&2
    tail -20 "$OUT/server.log" >&2
    exit 3
}
stop_server() {
    [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
    SERVER_PID=""
}

start_server
echo "== the headless server is up on $URL"

# The reference client's own verbs, exactly as a person would run them.
"$CLI" --server "$URL" --token "$TOKEN" camera get --json >"$OUT/camera-before.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" watch --json >"$OUT/watch.log" 2>&1 &
WATCH_PID=$!
sleep 1
"$CLI" --server "$URL" --token "$TOKEN" camera left --json >"$OUT/left.json" 2>&1 || true
"$CLI" --server "$URL" --token "$TOKEN" camera zoom-in --json >"$OUT/zoom.json" 2>&1 || true
sleep 1
"$CLI" --server "$URL" --token "$TOKEN" camera get --json >"$OUT/camera-after.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" camera set --lat 51.5 --lon -0.12 --zoom 9 >"$OUT/set.json" 2>&1 || true
sleep 0.6
"$CLI" --server "$URL" --token "$TOKEN" camera get --json >"$OUT/set-once.json" 2>&1
REVISION_AFTER_SET=$("$CLI" --server "$URL" --token "$TOKEN" --json status 2>/dev/null \
    | python3 -c 'import json,sys;print(json.load(sys.stdin)["revision"])' 2>/dev/null || echo 0)
# The same absolute camera again: idempotent, so no revision is spent on it.
"$CLI" --server "$URL" --token "$TOKEN" camera set --lat 51.5 --lon -0.12 --zoom 9 >/dev/null 2>&1 || true
sleep 0.6
REVISION_AFTER_REPEAT=$("$CLI" --server "$URL" --token "$TOKEN" --json status 2>/dev/null \
    | python3 -c 'import json,sys;print(json.load(sys.stdin)["revision"])' 2>/dev/null || echo 0)
"$CLI" --server "$URL" --token "$TOKEN" whoami --json >"$OUT/whoami-operator.json" 2>&1

# Pairing, scopes, revocation and the viewport report need an HTTP client; the
# CLI deliberately has no "spend somebody else's code" verb, because that is
# the device's job and not the reference client's.
DEVICE_TOKEN=$(python3 - "$URL" "$TOKEN" "$OUT" <<'PY'
import json, sys, time, urllib.error, urllib.request

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

# A window opens, a device spends the code, and the code is spent for good.
status, offer = call("POST", "/v1/pairing", operator, {"scopes": ["observe", "control_view"]})
save("pairing-offer.json", {"status": status, "offer": offer})
status, paired = call("POST", "/v1/pair", None, {"code": offer.get("code", "").lower(), "label": "smoke phone"})
save("paired.json", {"status": status, "paired": paired})
device_token = paired.get("token", "")
status, replay = call("POST", "/v1/pair", None, {"code": offer.get("code", ""), "label": "replay"})
save("pairing-replay.json", {"status": status, "body": replay})

# What that device is, and what it may not do.
whoami_status, whoami_body = call("GET", "/v1/whoami", device_token)
save("whoami-device.json", {"status": whoami_status, "body": whoami_body})
save("device-pair.json", {"status": call("POST", "/v1/pairing", device_token, {})[0]})
save("device-none.json", {"status": call("GET", "/v1/state", None)[0]})
save("pairing-admin.json", {"status": call("POST", "/v1/pairing", operator, {"scopes": ["observe", "admin"]})[0]})

# A watching-only device can see the map and cannot move it.
_, watcher = call("POST", "/v1/pairing", operator, {"scopes": ["observe"]})
_, watched = call("POST", "/v1/pair", None, {"code": watcher["code"], "label": "smoke display"})
watch_token = watched.get("token", "")
save("watch-steer.json", {
    "status": call("POST", "/v1/commands", watch_token,
                   {"type": "camera.zoom", "payload": {"delta": 1.0}})[0],
})
save("watch-read.json", {"status": call("GET", "/v1/state", watch_token)[0]})

# The viewport is the display's own fact: the same fraction of a wider
# viewport is a bigger move, which is what makes a phone's "left" mean the
# same thing on a 4K monitor.
def nudge_west():
    before = call("GET", "/v1/views/main/camera", operator)[1]["camera"]["center"]["longitude"]
    call("POST", "/v1/commands", operator,
         {"type": "camera.nudge", "payload": {"dx_fraction": 0.25, "dy_fraction": 0.0},
          "client_id": "smoke"})
    for _ in range(50):
        time.sleep(0.05)
        after = call("GET", "/v1/views/main/camera", operator)[1]["camera"]["center"]["longitude"]
        if after != before:
            return before - after
    return 0.0

narrow = nudge_west()
call("POST", "/v1/commands", operator,
     {"type": "view.report", "payload": {"width": 1920, "height": 1200}, "client_id": "smoke"})
time.sleep(0.3)
wide = nudge_west()
save("viewport.json", {"narrow": narrow, "wide": wide,
                       "camera": call("GET", "/v1/views/main/camera", operator)[1]["camera"]})

# Revocation, while the file is the only record of it.
_, devices = call("GET", "/v1/devices", operator)
save("devices.json", {"status": 200, "body": devices})
save("revoked.json", {"status": call("DELETE", "/v1/devices/" + paired.get("device", {}).get("id", "?"),
                                    operator)[0]})
save("revoked-token.json", {"status": call("GET", "/v1/state", device_token)[0]})
save("revoke-twice.json", {"status": call("DELETE", "/v1/devices/" + paired.get("device", {}).get("id", "?"),
                                          operator)[0]})

print(device_token)
PY
)
DEVICE_ID=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["paired"]["device"]["id"])' "$OUT/paired.json" 2>/dev/null || echo "?")

echo "== the paired device was $DEVICE_ID"

# Observation: an exercise is published, a command center attaches, the phone's
# focus moves the observer view and never the game master's camera.
#
# This is the point of the control plane's second half, and none of it is
# visible to a unit test: the readiness verdict crosses HTTP, the display
# handshake crosses the WebSocket, and the focus is resolved against the session
# the server holds.
echo "== observation mode"
"$CLI" --server "$URL" --token "$TOKEN" session set --file fixtures/observation-session.json \
    >"$OUT/session-published.json" 2>&1 || true
"$CLI" --server "$URL" --token "$TOKEN" --json session status >"$OUT/session-no-display.json" 2>&1
# A phone token cannot publish the exercise: only the console holds one.
python3 - "$URL" "$TOKEN" "$OUT" <<'DEVICE_PUT'
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

# A fresh phone, because the device this script paired earlier has been
# revoked by the time it gets here and would be refused for the wrong reason.
_, offer = call("POST", "/v1/pairing", operator, {"scopes": ["observe", "control_view"]})
_, paired = call("POST", "/v1/pair", None,
                 {"code": offer.get("code", ""), "label": "observation phone"})
phone = paired.get("token", "")
body = json.loads(open("fixtures/observation-session.json").read())
status, reply = call("PUT", "/v1/session", phone, body)
with open(f"{out}/session-put-device.json", "w") as handle:
    json.dump({"status": status, "body": reply}, handle)
DEVICE_PUT
"$CLI" --server "$URL" --token "$TOKEN" display attach --id smoke-wall --label "smoke wall" \
    --width 1920 --height 1080 >"$OUT/display.log" 2>&1 &
DISPLAY_PID=$!
for _ in $(seq 1 20); do
    if "$CLI" --server "$URL" --token "$TOKEN" --json session status 2>/dev/null \
        | python3 -c 'import json,sys; sys.exit(0 if json.load(sys.stdin)["readiness"]["ready"] else 1)' 2>/dev/null; then
        break
    fi
    sleep 0.25
done
"$CLI" --server "$URL" --token "$TOKEN" --json session status >"$OUT/session-ready.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" --json display list >"$OUT/display-list.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" --json units >"$OUT/units.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" --json camera get >"$OUT/main-before-focus.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" --json camera get --view observer >"$OUT/observer-before-focus.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" --json focus 3 --zoom 13 >"$OUT/focus.json" 2>&1 || true
sleep 0.4
"$CLI" --server "$URL" --token "$TOKEN" --json camera get --view observer >"$OUT/observer-after-focus.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" --json camera get >"$OUT/main-after-focus.json" 2>&1
# The console's one remembered setting: a host-wide preference with a read.
"$CLI" --server "$URL" --token "$TOKEN" --json settings >"$OUT/settings-before.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" settings font 4 >"$OUT/settings-set.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" --json settings >"$OUT/settings-after.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" settings font 2 >/dev/null 2>&1

# The phone's windows and the hull pictures: both are server state, and both
# are what the wall draws from. The picture is a real 1x1 PNG so the GET is
# checked against bytes, not against a content type alone.
"$CLI" --server "$URL" --token "$TOKEN" --json panel >"$OUT/panels-closed.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" panel crew on >"$OUT/panel-crew-on.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" --json session status >"$OUT/session-panel-open.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" --json panel >"$OUT/panels-open.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" panel crew off >"$OUT/panel-crew-off.json" 2>&1
printf '%s' 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==' \
    | base64 -d >"$OUT/hull.png"
curl -sS -X PUT "$URL/v1/units/9001/picture" \
    -H "Authorization: Bearer $TOKEN" -H 'content-type: image/png' \
    --data-binary @"$OUT/hull.png" >"$OUT/picture-put.json"
curl -sS -o "$OUT/picture-got.png" -w '%{http_code} %{content_type}' \
    "$URL/v1/units/9001/picture" -H "Authorization: Bearer $TOKEN" >"$OUT/picture-get.txt"

# The console goes quiet: the session outlives its heartbeat, the verdict flips
# to `stale`, and steering is refused while it is.
sleep 17
"$CLI" --server "$URL" --token "$TOKEN" --json session status >"$OUT/session-stale.json" 2>&1
"$CLI" --server "$URL" --token "$TOKEN" --json focus 1 >"$OUT/focus-stale.json" 2>&1 || true
# The command center closes its window and stops counting at once.
kill "$DISPLAY_PID" 2>/dev/null || true
DISPLAY_PID=""
for _ in $(seq 1 20); do
    if "$CLI" --server "$URL" --token "$TOKEN" --json display list 2>/dev/null \
        | python3 -c 'import json,sys; sys.exit(0 if not json.load(sys.stdin)["displays"] else 1)' 2>/dev/null; then
        break
    fi
    sleep 0.25
done
"$CLI" --server "$URL" --token "$TOKEN" --json session status >"$OUT/session-detached.json" 2>&1
echo "== observation mode released its display"

# A restart is the only test that a revoked device stays revoked: an in-memory
# table would pass every check above and forget everything here.
stop_server
sleep 0.5
start_server
echo "== restarted with the same devices file"

python3 - "$URL" "$OUT" "$DEVICE_TOKEN" "${REVISION_AFTER_SET:-0}" "${REVISION_AFTER_REPEAT:-0}" <<'PY'
import json
import pathlib
import sys
import urllib.error
import urllib.request

url, out_dir, revoked_token, rev_set, rev_repeat = sys.argv[1], pathlib.Path(sys.argv[2]), sys.argv[3], sys.argv[4], sys.argv[5]
checks = []

def check(name, ok, detail=""):
    checks.append((name, ok, detail))

def read(name):
    try:
        return json.loads((out_dir / name).read_text())
    except Exception as error:  # noqa: BLE001 - report, do not crash
        return {"_error": str(error)}

def get(path, token=None):
    request = urllib.request.Request(url + path)
    if token:
        request.add_header("Authorization", "Bearer " + token)
    try:
        with urllib.request.urlopen(request) as response:
            return response.status, json.loads(response.read() or b"{}")
    except urllib.error.HTTPError as error:
        return error.code, {}

before = read("camera-before.json")
after = read("camera-after.json")
once = read("set-once.json")
left = read("left.json")
zoom = read("zoom.json")
whoami = read("whoami-operator.json")
offer = read("pairing-offer.json")
paired = read("paired.json")
replay = read("pairing-replay.json")
device_me = read("whoami-device.json")
watch_steer = read("watch-steer.json")
watch_read = read("watch-read.json")
devices = read("devices.json")
revoked = read("revoked.json")
revoke_twice = read("revoke-twice.json")
viewport = read("viewport.json")
session_published = read("session-published.json")
session_no_display = read("session-no-display.json")
session_ready = read("session-ready.json")
display_list = read("display-list.json")
units = read("units.json")
focus = read("focus.json")
observer_before = read("observer-before-focus.json")
observer_after = read("observer-after-focus.json")
main_before = read("main-before-focus.json")
main_after = read("main-after-focus.json")
session_stale = read("session-stale.json")
session_detached = read("session-detached.json")
session_put_device = read("session-put-device.json")

check("the reference client's nudge was accepted", left.get("status") == "accepted", json.dumps(left))
check("the reference client's zoom was accepted", zoom.get("status") == "accepted", json.dumps(zoom))

try:
    check(
        "a nudge moves the headless camera west",
        after["center"]["longitude"] < before["center"]["longitude"],
        f'{before["center"]["longitude"]} -> {after["center"]["longitude"]}',
    )
except (KeyError, TypeError) as error:
    check("a nudge moves the headless camera west", False, f"unreadable camera: {error}")

try:
    check(
        "a zoom step adds exactly one",
        abs(after["zoom"] - (before["zoom"] + 1.0)) < 1e-9,
        f'{before["zoom"]} -> {after["zoom"]}',
    )
except (KeyError, TypeError):
    check("a zoom step adds exactly one", False, "unreadable camera")

try:
    check(
        "camera.set is absolute",
        abs(once["center"]["latitude"] - 51.5) < 1e-9
        and abs(once["center"]["longitude"] + 0.12) < 1e-9
        and abs(once["zoom"] - 9.0) < 1e-9,
        json.dumps(once),
    )
except (KeyError, TypeError):
    check("camera.set is absolute", False, "unreadable camera")

check(
    "the same camera set twice spends no revision",
    rev_set == rev_repeat and rev_set != "0",
    f"revision {rev_set} then {rev_repeat}",
)

check("the operator token carries admin", "admin" in whoami.get("scopes", []), json.dumps(whoami.get("scopes")))

check("a pairing window opens", offer.get("status") == 200 and offer["offer"].get("code"), json.dumps(offer)[:200])
check(
    "the offer carries a URI and a JSON payload for a QR code",
    offer.get("offer", {}).get("uri", "").startswith("tfg-control://pair?")
    and json.loads(offer["offer"]["payload"]).get("pairing_code") == offer["offer"]["code"],
    offer.get("offer", {}).get("uri", "")[:60],
)
check("a code pairs a device", paired.get("status") == 200 and paired["paired"].get("token"), json.dumps(paired)[:120])
check("a spent code cannot pair a second device", replay.get("status") == 404, json.dumps(replay.get("body"))[:120])
check(
    "the paired device resolves to itself with its scopes",
    device_me.get("status") == 200 and device_me["body"].get("label") == "smoke phone",
    json.dumps(device_me.get("body"))[:120],
)
check("a paired device cannot open a pairing window", read("device-pair.json").get("status") == 403)
check("a request with no token is refused", read("device-none.json").get("status") == 401)
check("pairing will not mint an admin device", read("pairing-admin.json").get("status") == 422)
check("a watching device can read the map", watch_read.get("status") == 200, json.dumps(watch_read))
check("a watching device cannot steer it", watch_steer.get("status") == 403, json.dumps(watch_steer))
check(
    "a wider viewport makes the same fraction a bigger move",
    viewport.get("wide", 0) > viewport.get("narrow", 0) > 0,
    f'narrow {viewport.get("narrow")} wide {viewport.get("wide")}',
)
check(
    "the device list names the device and never its hash",
    devices.get("body", {}).get("devices", [{}])[0].get("id") and "token_hash" not in json.dumps(devices),
    json.dumps(devices.get("body"))[:160],
)
check("a device can be revoked", revoked.get("status") == 200)
check("revoking twice is a 404", revoke_twice.get("status") == 404)
check("a revoked token is refused at once", read("revoked-token.json").get("status") == 401)

check(
    "the console's projection was accepted",
    session_published.get("status") == "changed"
    and session_published.get("readiness", {}).get("reason") == "no_display",
    json.dumps(session_published)[:160],
)
check(
    "a session with no command center waits",
    session_no_display.get("readiness", {}).get("reason") == "no_display",
    json.dumps(session_no_display.get("readiness")),
)
check(
    "a paired phone cannot publish the console's session",
    session_put_device.get("status") == 403,
    json.dumps(session_put_device)[:120],
)
check(
    "an attached command center opens observation",
    session_ready.get("readiness", {}).get("ready") is True,
    json.dumps(session_ready.get("readiness")),
)
displays = display_list.get("displays", [])
check(
    "the display list names the attached command center",
    len(displays) == 1 and displays[0].get("id") == "smoke-wall"
    and displays[0].get("viewport", {}).get("width") == 1920,
    json.dumps(displays)[:160],
)
listed_units = units if isinstance(units, list) else units.get("session", {}).get("units")
check(
    "the units are numbered the way the phone's grid numbers them",
    isinstance(listed_units, list)
    and len(listed_units) == 6
    and [unit.get("index") for unit in listed_units] == [1, 2, 3, 4, 5, 6],
    f"{len(listed_units) if isinstance(listed_units, list) else '?'} units",
)
check(
    "focus is accepted against the observer view",
    focus.get("status") == "accepted" and focus.get("view") == "observer",
    json.dumps(focus)[:120],
)


def centre(camera):
    try:
        return (round(camera["center"]["latitude"], 6), round(camera["center"]["longitude"], 6))
    except (KeyError, TypeError):
        return None


check(
    "the observer view centred on unit 3 at the requested zoom",
    centre(observer_after) == (-6.0211, 105.7733)
    and abs(observer_after.get("zoom", 0) - 13.0) < 1e-9,
    f'{centre(observer_after)} zoom {observer_after.get("zoom")}',
)
check(
    "the observer command moved the observer view",
    centre(observer_before) != centre(observer_after),
    f"{centre(observer_before)} -> {centre(observer_after)}",
)
check(
    "the game master's camera did not move",
    centre(main_before) == centre(main_after)
    and main_before.get("zoom") == main_after.get("zoom"),
    f"{centre(main_before)} zoom {main_before.get('zoom')} -> "
    f"{centre(main_after)} zoom {main_after.get('zoom')}",
)
check(
    "a console that stopped refreshing turns the verdict stale",
    session_stale.get("readiness", {}).get("reason") == "stale",
    json.dumps(session_stale.get("readiness")),
)
# The CLI prints a refusal as one line on stderr, and that line is the
# contract a person reads: the code and the reason.
stale_text = (out_dir / "focus-stale.json").read_text() if (out_dir / "focus-stale.json").exists() else ""
check(
    "steering is refused while the session is stale",
    "not_ready" in stale_text and "stale" in stale_text,
    stale_text.strip()[:120],
)
settings_before = json.loads((out_dir / "settings-before.json").read_text())
check(
    "the console's text size is readable",
    "step" in settings_before and settings_before.get("names", [None] * 4)[1] == "Normal",
    json.dumps(settings_before),
)
settings_set = json.loads((out_dir / "settings-set.json").read_text())
check(
    "a font step is accepted",
    settings_set.get("status") == "accepted",
    json.dumps(settings_set),
)
settings_after = json.loads((out_dir / "settings-after.json").read_text())
check(
    "the new size is what the host reports",
    settings_after.get("step") == 3,
    json.dumps(settings_after),
)
panels_closed = json.loads((out_dir / "panels-closed.json").read_text())
check(
    "no observer window is open until the phone asks",
    panels_closed.get("panels", {}).get("crew") is not True,
    json.dumps(panels_closed),
)
panel_open = json.loads((out_dir / "session-panel-open.json").read_text())
check(
    "the phone's window reaches the shared state",
    panel_open.get("ui", {}).get("panels", {}).get("crew") is True,
    json.dumps(panel_open.get("ui")),
)
panel_read = json.loads((out_dir / "panels-open.json").read_text())
check(
    "the panel read names the open window",
    panel_read.get("panels", {}).get("crew") is True,
    json.dumps(panel_read),
)
panel_off = json.loads((out_dir / "panel-crew-off.json").read_text())
check(
    "the window closes again",
    panel_off.get("status") == "accepted",
    json.dumps(panel_off),
)
picture_put = json.loads((out_dir / "picture-put.json").read_text())
check(
    "the console publishes a hull picture",
    picture_put.get("status") == "stored",
    json.dumps(picture_put),
)
got_status, got_type = (out_dir / "picture-get.txt").read_text().split()
served = (out_dir / "picture-got.png").read_bytes()
check(
    "the wall can fetch the same bytes",
    got_status == "200" and got_type == "image/png"
    and served[:8] == b"\x89PNG\r\n\x1a\n" and len(served) > 40,
    f"{got_status} {got_type} {len(served)} bytes",
)
check(
    "closing the command center releases observation",
    session_detached.get("readiness", {}).get("reason") in ("no_display", "no_session", "stale"),
    json.dumps(session_detached.get("readiness")),
)

# After the restart.
check(
    "the revoked device stayed revoked across a restart",
    get("/v1/state", revoked_token)[0] == 401,
)

devices_file = out_dir / "devices" / "control-devices.json"
mode = oct(devices_file.stat().st_mode & 0o777) if devices_file.exists() else "missing"
check("the devices file is 0600", mode == "0o600", mode)
text = devices_file.read_text() if devices_file.exists() else ""
check(
    "the devices file holds no plaintext token",
    "token" not in text.replace("token_hash", ""),
    f"{devices_file.name}, {len(text)} bytes",
)

events = []
for line in (out_dir / "watch.log").read_text().splitlines():
    line = line.strip()
    if line.startswith("{"):
        try:
            events.append(json.loads(line))
        except ValueError:
            pass
check("the watcher got a snapshot", any(event.get("type") == "snapshot" for event in events), f"{len(events)} events")
changed = [event for event in events if event.get("type") == "camera.changed"]
check("the watcher saw the headless camera change", bool(changed), f"{len(changed)} camera events")
check(
    "the change names the client that asked",
    any(event.get("source") == "tfgctl" for event in changed),
    ", ".join(sorted({str(event.get("source")) for event in changed})),
)

failed = 0
for name, ok, detail in checks:
    if not ok:
        failed += 1
    print(f'{"PASS" if ok else "FAIL"}  {name}' + (f'  ({detail})' if detail else ""))
print()
print(f"{len(checks) - failed}/{len(checks)} checks passed; artifacts in {out_dir}")
sys.exit(1 if failed else 0)
PY
STATUS=$?

echo
echo "== server log (applied commands)"
grep -E "^r[0-9]+|^        " "$OUT/server.log" | tail -12 || true
exit "$STATUS"
