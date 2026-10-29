# TFG control server — API list

A flat, list-shaped reference for the TFG control plane (protocol v2). This is the document to hand to the remote mobile app's developer: what exists, what it is called, and what it is for.

The full narrative contract lives in [`control-api-v1.md`](control-api-v1.md); this file is the list, and where the two disagree, the list should win for scoping purposes and the narrative should win for meaning. Update both together.

The illustrated developer reference — Mermaid diagrams, field tables, and copyable JSON examples — is [`control-api.html`](control-api.html).

## Base

- Service: `tfg-control`, API version `v1`, protocol version `2`.
- Base path: `/v1/`.
- Host: the console in client mode, or the headless host (`tfg-server`). Default listen `127.0.0.1:8787`.
- Client: base URL + `Authorization: Bearer <token>`.

## Autentikasi

Every route except `GET /v1/hello` and `POST /v1/pair` requires a bearer token.

| Credential | Grant | Scopes |
| --- | --- | --- |
| `TFG_CONTROL_TOKEN` | `operator` | `observe`, `control_view`, `admin` |
| paired device | `dev-…` | granted at pairing |

- `observe` — reads: `GET /v1/state`, `/v1/views*`, `/v1/ws`.
- `control_view` — camera commands + `view.report`; implies `observe`.
- `admin` — pairing window, list/revoke devices; implies everything.

Errors: `401 unauthorized`, `403 forbidden`; wrong envelope version: `409 unsupported_protocol`.

## Endpoints

### Reads

| Method | Path | Auth | Purpose | In | Out | Example |
| --- | --- | --- | --- | --- | --- | --- |
| GET | `/v1/hello` | none | protocol/revision/pairing_open | — | JSON | `{"protocol_version":2,"revision":7,"pairing_open":false}` |
| GET | `/v1/whoami` | bearer | grant + scopes | — | JSON | `{"client":"dev-4b2379bfa714","scopes":["observe","control_view"]}` |
| GET | `/v1/state` | observe | whole shared state | — | JSON | `{"type":"snapshot","revision":7,"session":{"game_id":45},"readiness":{"ready":true}}` |
| GET | `/v1/views` | observe | views | — | JSON | `{"views":{"main":{"id":"main"},"observer":{"id":"observer"}}}` |
| GET | `/v1/views/{view_id}/camera` | observe | camera one view | — | JSON; 404 if unknown | `{"id":"main","camera":{"center":{"latitude":-6.108,"longitude":106.91},"zoom":11.0,"width":1280,"height":720}}` |
| GET | `/v1/session` | observe | exercise + readiness + displays | — | JSON | `{"game_id":45,"phase":"execution"}` |
| GET | `/v1/units/{unit_id}/picture` | observe | hull photo | — | image/png | image/png |
| GET | `/v1/displays` | observe | displays + readiness | — | JSON | `{"displays":[{"id":"wall-1"}],"readiness":{"ready":true}}` |

### Commands

| Method | Path | Auth | Purpose | In | Out | Example |
| --- | --- | --- | --- | --- | --- | --- |
| POST | `/v1/commands` | control_view | send one command | JSON command — `{"protocol_version":2,"type":"camera.nudge","payload":{"dx_fraction":0.25,"dy_fraction":0.0}}` | 202 accepted / 200 duplicate JSON | `{"command_id":"01J8Z6Q0K3","status":"accepted","revision":7}` |

### Pairing and devices

| Method | Path | Auth | Purpose | In | Out | Example |
| --- | --- | --- | --- | --- | --- | --- |
| POST | `/v1/pairing` | admin | open pairing window | JSON `{scopes?, ttl_seconds?}` — e.g. `{"scopes":["observe","control_view"],"ttl_seconds":120}` | JSON `{code, expires_in, uri, payload, url, scopes}` — e.g. `{"code":"K7QM-3XPD","expires_in":120,...}` | `{"code":"K7QM-3XPD","expires_in":120,...}` |
| POST | `/v1/pair` | none (code) | spend code | JSON `{code, label}` — e.g. `{"code":"k7qm 3xpd","label":"phone"}` | JSON `{device, token, url}` | `{"device":{...},"token":"9f2c…","url":"http://192.168.1.20:8787"}` |
| GET | `/v1/devices` | admin | list devices | — | JSON `{devices:[…]}` | `{"devices":[{…}]}` |
| DELETE | `/v1/devices/{id}` | admin | revoke device | — (path) | JSON `{device, status}` | `{"device":"dev-…","status":"revoked"}` |
| POST | `/v1/auth/rotate` | device | rotate token | JSON `{device, token}` — e.g. `{"device":"dev-4b2379bfa714","token":"…"}` | JSON `{device, token}` | `{"device":"dev-…","token":"…"}` |

### Realtime

| Method | Path | Auth | Purpose | In | Out | Example |
| --- | --- | --- | --- | --- | --- | --- |
| GET | `/v1/ws` | observe | realtime stream | WS upgrade; `request.snapshot` — e.g. `{"type":"request.snapshot"}` | events | `{"type":"camera.changed","revision":8,...}` |

## Examples

### Reads — every endpoint

- `GET /v1/hello` (no auth) → out: `{ "protocol_version": 2, "api_version": "v1", "service": "tfg-control", "revision": 7, "pairing_open": false }`.
- `GET /v1/whoami` (bearer) → out: `{ "client": "dev-4b2379bfa714", "label": "phone", "scopes": ["observe", "control_view"] }`.
- `GET /v1/state` (observe) → out: `{ "type": "snapshot", "protocol_version": 2, "revision": 7, "views": { ... }, "session": { ... }, "readiness": { "ready": true }, "displays": [ ... ] }`.
- `GET /v1/views` (observe) → out: `{ "views": { "main": { ... }, "observer": { ... } } }`.
- `GET /v1/views/{view_id}/camera` (observe) → out: `{ "id": "main", "camera": { ... } }`; 404 if unknown.
- `GET /v1/session` (observe) → out: `{ "game_id": 45, "name": "Sunda Strait Watch", "phase": "execution", "mode": "exercise", "units": [] }`.
- `GET /v1/units/{unit_id}/picture` (observe) → out: image/png.
- `GET /v1/displays` (observe) → out: `{ "displays": [ … ], "readiness": { "ready": true } }`.

### Commands — every type

- `camera.nudge`: in `{ "protocol_version": 2, "type": "camera.nudge", "payload": { "dx_fraction": 0.25, "dy_fraction": 0.0 } }` → out `{ "command_id": "01J8Z6Q0K3", "status": "accepted", "revision": 7 }`.
- `camera.zoom`: in `{ "protocol_version": 2, "type": "camera.zoom", "payload": { "delta": 1.0 } }` → out ack.
- `camera.set`: in `{ "protocol_version": 2, "type": "camera.set", "payload": { "center": { "latitude": -6.2, "longitude": 106.8 }, "zoom": 12.0 } }` → out ack.
- `camera.focus`: in `{ "protocol_version": 2, "type": "camera.focus", "payload": { "unit_id": 9001, "zoom": 12.0 } }` → out ack / `422`.
- `ui.set`: in `{ "protocol_version": 2, "type": "ui.set", "payload": { "panel": "scenario", "open": true } }` → out ack.
- `settings.font`: in `{ "protocol_version": 2, "type": "settings.font", "payload": { "step": 1 } }` → out ack.
- `view.report`: in `{ "protocol_version": 2, "type": "view.report", "payload": { "width": 1920, "height": 1200 } }` → out ack.

### Pairing and devices — every endpoint

- `POST /v1/pairing`: in `{ "scopes": ["observe", "control_view"], "ttl_seconds": 120 }` → out `{ "code": "K7QM-3XPD", "expires_in": 120, "uri": "tfg-control://pair?url=http://192.168.1.20:8787&code=K7QM-3XPD", "payload": "...", "url": "http://192.168.1.20:8787", "scopes": ["observe", "control_view"] }`.
- `POST /v1/pair`: in `{ "code": "k7qm 3xpd", "label": "phone" }` → out `{ "device": { "id": "dev-4b2379bfa714", "label": "phone", "scopes": ["observe","control_view"], "created_at": 1793084168, "last_seen": 1793084168 }, "token": "9f2c…", "url": "http://192.168.1.20:8787" }`.
- `GET /v1/devices`: out `{ "devices": [ { "id": "dev-…", "label": "phone", "scopes": ["observe"], "created_at": 1793084168, "last_seen": 1793084168 } ] }`.
- `DELETE /v1/devices/{id}`: in (path), out `{ "device": "dev-…", "status": "revoked" }`.
- `POST /v1/auth/rotate`: in `{ "device": "dev-4b2379bfa714", "token": "…" }` → out `{ "device": "dev-4b2379bfa714", "token": "…" }`.

### Realtime — events

- `snapshot`: whole state at `revision`.
- `camera.changed`: `{ "type": "camera.changed", "revision": 8, "source": "phone-7", "command_id": "01J8Z6Q0K3", "view": { "id": "observer", "camera": { ... } } }`.
- `session.changed`: exercise, phase, units, readiness, displays, ui changed.
- `display.attached` / `display.rejected`: display hello outcome.
- `command.accepted` / `command.rejected`: command outcome.

## Errors

| HTTP | Code | Cause |
| --- | --- | --- |
| 401 | `unauthorized` | missing/wrong bearer |
| 403 | `forbidden` | real token, insufficient scope |
| 400 | `invalid_json` | body not JSON / not a command |
| 413 | `body_too_large` | body > 64 KiB |
| 422 | `invalid_command` | parsed, but cannot be applied |
| 409 | `unsupported_protocol` | protocol_version ≠ 2 |
| 409 | `not_ready` | observer command before observation open |
| 404 | `not_found` | no such route/view/device |
| 503 | `host_gone` | host's command channel closed |
| 503 | `too_many_clients` | too many WS clients |

Pairing errors: `invalid_pairing_code`, `no_pairing_window`, `pairing_expired`, `too_many_attempts`.

## Client quick reference (tfgctl)

```bash
tfgctl hello
tfgctl status
tfgctl session status
tfgctl session watch --until ready
tfgctl session set --file session.json
tfgctl session clear
tfgctl units
tfgctl focus 3
tfgctl display list
tfgctl display attach --label wall-1
tfgctl panel
tfgctl panel scenario on
tfgctl panel scenario off
tfgctl camera get
tfgctl camera zoom-in
tfgctl watch
```

Flags: `--server URL`, `--token TOKEN`, `--json`, `--timeout 3s`, `--no-color`.

## Live state (for testing against your wall right now)

```bash
tfgctl status
# TGF-V3-TREE [execution]  2 unit(s)  1 display(s)  ready
```
