# TFG control API v2

A TFG host can expose a small control endpoint so another client — the `tfgctl`
CLI today, a phone or a command-center wall later — can move the map, watch the
shared state, and observe a live exercise. It is off unless the operator turns
it on, and it binds to loopback unless they say otherwise.

The developer-facing HTML reference — diagrams, field tables, and copyable JSON
request/response examples — is [`control-api.html`](control-api.html).

The routes kept their `/v1/` prefix; only the envelope version moved to `2`.
Version 2 adds the observation surfaces: a shared **session**, a computed
**readiness** verdict, display presence, the `observer` view, and
`camera.focus`. A v1 client is refused, not misread: see [Versions](#versions).

There are two hosts, one protocol:

- **The console** (`tfg`) binds the endpoint to a window that is already open.
  The camera in the protocol is the camera the operator is looking at, and a
  command moves that window's map.
- **`tfg-server`** is headless. It owns the camera itself, with the same
  arithmetic the console uses, and has no picture to draw.

A client cannot tell them apart, and should not try: the protocol is about a
camera and a place on the earth, not about a renderer.

The design is recorded in [`docs/adr/0025-control-plane.md`](adr/0025-control-plane.md).
This document is the contract: if a client works against it, the client works.

## Turning it on

```bash
TFG_CONTROL_LISTEN=127.0.0.1:8787 \
TFG_CONTROL_TOKEN="$(openssl rand -hex 32)" \
cargo run
```

or, with no window at all:

```bash
cargo run --bin tfg-server -- --listen 127.0.0.1:8787
```

| Variable | Meaning |
| --- | --- |
| `TFG_CONTROL_LISTEN` | Address to bind. Loopback for a client on this machine; `0.0.0.0:8787` exposes it to the LAN, which is an explicit choice. |
| `TFG_CONTROL_SERVER` | An `http://host:port` to follow instead of hosting. The console becomes a display for that host (see below). |
| `TFG_CONTROL_TOKEN` | The operator's bearer token. At least 32 characters, or the server refuses to start. |
| `TFG_CONTROL_DEVICES` | Where the paired-device table lives. Default `$XDG_DATA_HOME/tfg/control-devices.json`. |

The two directions are exclusive. `TFG_CONTROL_SERVER` wins when both names are
set, and the console says that it ignored `TFG_CONTROL_LISTEN`: two authorities
over one camera would each believe they are the writer.

With neither listen nor token set there is no server. With only
`TFG_CONTROL_LISTEN` set the console says `TFG_CONTROL_TOKEN is missing` and
starts without one. A bind failure, or a devices file that does not parse, is
reported and the console starts anyway: remote control is a convenience, never a
reason for the map not to open. The control endpoint is the only thing that
refuses to start in that case, and it says which file and why.

`tfg-server` is the opposite: the endpoint *is* the program, so every startup
failure is fatal and named.

**Credentials are secrets.** They are compared in constant time, stored as
SHA-256 hashes when they are on disk at all, never logged, and never returned by
any route. The one exception is deliberate and one-shot: the token a pairing
exchange mints is returned to the device that spent the code, once, and never
again. The Minos credentials are not part of this API and never will be:
clients talk to the host, and the host talks to Minos.

## Versions

- The routes are under `/v1/`. The prefix did not move when the envelope did:
  a path is a place, a version is a language.
- Bodies carry `protocol_version`: `2`.
- `GET /v1/hello` reports the envelope version and the route version. It is
  unauthenticated, because a client that cannot read `hello` cannot learn which
  version to speak.
- A command whose `protocol_version` is not `2` is refused with
  `409 unsupported_protocol`. Being misread is worse than being refused.
- v2 is a clean break: the snapshot carries `session`, `readiness`,
  `displays` and the host's `settings`, the wire has two more events
  (`session.changed`, `settings.changed`), and `observer` exists as a second
  view. Every client in this repository moved together, and
  an old client is expected to fail loudly rather than half-work.

## Authentication and scopes

Every route except `GET /v1/hello` and `POST /v1/pair` needs:

```http
Authorization: Bearer <token>
```

A missing or wrong token is `401 unauthorized` with `WWW-Authenticate: Bearer`.

Two kinds of credential resolve to a **grant**:

| Credential | Grant id | Scopes |
| --- | --- | --- |
| `TFG_CONTROL_TOKEN` | `operator` | `observe`, `control_view`, `admin` |
| A paired device | `dev-…` | whatever the pairing window granted |

Scopes are cumulative:

| Scope | Allows | Implies |
| --- | --- | --- |
| `observe` | `GET /v1/state`, `/v1/views*`, `/v1/ws` | — |
| `control_view` | every camera command and `view.report` | `observe` |
| `admin` | open a pairing window, list and revoke devices | everything |

A credential that is real but insufficient is `403 forbidden`, which is a
different answer from `401`: the first says "not this token's job", the second
says "not a token".

The operator token is never written to disk and cannot be rotated over HTTP:
it comes from the environment, so replacing it means changing the environment
and restarting. A paired device rotates its own token instead
(`POST /v1/auth/rotate`).

`GET /v1/whoami` says which grant a token resolved to, which is the first thing
to run when a command is refused.

## Reads

### `GET /v1/hello`

```json
{
  "protocol_version": 2,
  "api_version": "v1",
  "service": "tfg-control",
  "revision": 7,
  "pairing_open": false
}
```

`pairing_open` is `true` while a one-time code is live, so a client can tell
"nobody is pairing" from "the code you were given has expired".

### `GET /v1/whoami`

```json
{ "client": "dev-4b2379bfa714", "label": "phone", "scopes": ["observe", "control_view"] }
```

### `GET /v1/state`

The whole shared state, and the shape the WebSocket sends on connect.

```json
{
  "type": "snapshot",
  "protocol_version": 2,
  "revision": 7,
  "views": {
    "main": {
      "id": "main",
      "camera": {
        "center": { "latitude": -6.108, "longitude": 106.91 },
        "zoom": 11.0,
        "width": 1280,
        "height": 720
      }
    },
    "observer": {
      "id": "observer",
      "camera": {
        "center": { "latitude": -5.9123, "longitude": 105.8412 },
        "zoom": 12.0,
        "width": 1280,
        "height": 720
      }
    }
  },
  "session": {
    "game_id": 45,
    "name": "Sunda Strait Watch",
    "phase": "execution",
    "mode": "exercise",
    "units": []
  },
  "readiness": { "ready": true },
  "displays": [
    {
      "id": "wall-1",
      "label": "wall-1",
      "role": "command_center",
      "viewport": { "width": 1280, "height": 720 }
    }
  ]
}
```

`revision` is monotonic over shared-state changes. `width`/`height` are the
rendering surface's viewport in device pixels, and they ride with the camera
because a fraction-of-viewport command is only meaningful against a size. On the
console they are the window's frame; on `tfg-server` they are whatever a display
last reported with `view.report`, or the console's base geometry until one does.

### `GET /v1/views`

```json
{ "views": { "main": { "id": "main", "camera": { … } }, "observer": { … } } }
```

`main` is the operator's camera. `observer` is the camera a phone may move: it
is a **separate** view, so a remote command can never drag the game master's map.

### `GET /v1/views/{view_id}/camera`

```json
{
  "id": "main",
  "camera": {
    "center": { "latitude": -6.108, "longitude": 106.91 },
    "zoom": 11.0,
    "width": 1280,
    "height": 720
  }
}
```

An unknown view is `404 not_found`. There are two views today, `main` and
`observer`; they are named rather than implied so a third display is additive.
Every command may carry `"view": "observer"` to address the second one; absent
means `main`.

### `GET /v1/session`

The exercise the console projected, whether observation is open, and which
command centers are attached. This is the read a phone's waiting screen polls.

```json
{
  "session": {
    "game_id": 45,
    "name": "Sunda Strait Watch",
    "phase": "execution",
    "mode": "exercise",
    "scenario": { "id": 7, "title": "Sunda Strait, first light", "step": "Step 3 of 7: shadow the contact" },
    "units": [
      {
        "index": 1,
        "id": 9001,
        "name": "Evertsen",
        "hull_number": "F805",
        "class_name": "Karel Doorman frigate",
        "commander": { "id": 3, "name": "Lt Cdr Halvorsen" },
        "embarked": [{ "id": 9101, "name": "NH90 N-110", "hull_number": "" }],
        "position": { "latitude": -5.9123, "longitude": 105.8412 },
        "heading_deg": 84.0,
        "speed_kn": 14.0,
        "stale": false
      }
    ]
  },
  "readiness": { "ready": true },
  "displays": [
    { "id": "wall-1", "label": "wall-1", "role": "command_center",
      "viewport": { "width": 1280, "height": 720 } }
  ],
  "ui": {
    "panels": { "crew": true, "scenario": false, "comms": false },
    "focus": 9001
  }
}
```

The session also carries `participants` (the exercise's seats, for the Crew
window) and `messages` (the traffic the console may read, newest first, bounded
to the most recent 40) — see [the session body](#get-v1session) above for the
fields. `units[]` carries `loa_m`, `beam_m`, `affiliation` and `symbol` so a wall
with no store can draw the console's own mark and photograph.

`session` is `null` when the console holds no exercise. `units[].index` is the
number the phone's grid and the wall's marker share; the server assigns it from
the publisher's array order.

**Readiness is computed, never asserted.** It is `ready` only when all four hold:

1. a session exists,
2. its phase is `execution`,
3. at least one `command_center` display is attached,
4. the session was refreshed within the last 15 seconds.

When it is not ready, `reason` names the one thing to wait for:

| `reason` | Meaning | What a waiting screen says |
| --- | --- | --- |
| `no_session` | Nothing has been published. | "No exercise open." |
| `not_execution` | A session exists in another phase. | "«name» is in planning. Observation opens at execution." |
| `no_display` | Execution, but no command center is attached. | "No command center is attached." |
| `stale` | The console stopped refreshing (crashed or lost the network). | "The host stopped reporting. Reconnecting…" |

An expired session is kept and reported `stale` rather than deleted: a stale map
beats an empty one.

`ui` is the observer's screen: the three windows the phone opened and the hull
its popup is about. It is UI state, but it lives on the host so every attached
wall agrees with the phone and a reconnect restores the same screen instead of a
blank one. A new exercise (a different game id), or a clear, resets it.

### `PUT /v1/session` and `DELETE /v1/session`

Needs `admin`. These are the console's routes, not a phone's: the console is the
only process that holds a Minos session, and it publishes a projection. A client
that is not the console has nothing to put here.

A `PUT` replaces the projection and refreshes its clock; the same body sent
again is a heartbeat, not a change. `DELETE` says the console holds no exercise
again. Both answer

```json
{ "status": "changed", "revision": 12, "readiness": { "ready": true } }
```

or `{ "status": "unchanged", … }`.

### `GET /v1/units/{unit_id}/picture`

The hull's photograph, as `image/png`, from the console's own cache. Needs
`observe`.

The console is the only process that may read Minos, so it is the only thing
that fetches pictures; a wall has no Minos credential and asks here instead.
`PUT` the same route publishes one (needs `admin`, a PNG body up to 4 MiB); the
console does that once per hull, when its thumb fetcher lands the picture.
`404` is the ordinary answer for a hull with no photograph, and a wall that gets
one draws the symbol instead.

`loa_m`/`beam_m` on the session's units are what size the photograph: the console
draws a hull at its published length and beam, not at whatever pixels the upload
happened to have.

### `GET /v1/displays`

```json
{ "displays": [ { "id": "wall-1", "label": "wall-1", "role": "command_center",
                  "viewport": { "width": 1280, "height": 720 } } ],
  "readiness": { "ready": true } }
```

A display is present for as long as its WebSocket lives, refreshed by a
heartbeat every 10 s and reaped after 30. `viewport` is `null` until the display
reports a size.

## Commands

`POST /v1/commands` takes one command. The reply says whether it was accepted;
the WebSocket says when it took effect.

```json
{
  "protocol_version": 2,
  "client_id": "phone-7",
  "command_id": "01J8Z6Q0K3",
  "type": "camera.nudge",
  "payload": { "dx_fraction": 0.25, "dy_fraction": 0.0 }
}
```

| Field | Required | Meaning |
| --- | --- | --- |
| `type` | yes | Command name. |
| `payload` | yes | Command arguments; the shape belongs to the type. |
| `command_id` | no | Unique per command. A retry with the same id is answered `duplicate` and does not act twice. Generated by the server when omitted. |
| `client_id` | no | Attribution only. Not authorization. Bounded to 64 printable characters; `remote` when absent. |
| `protocol_version` | no | Envelope version. Absent means 1, which this server refuses. |
| `view` | no | `main` (the operator's camera, default) or `observer` (the phone's). |

Accepted reply — `202 Accepted`:

```json
{ "command_id": "01J8Z6Q0K3", "status": "accepted", "revision": 7 }
```

The revision is the state at acceptance, before the command has been applied.
The `camera.changed` that follows carries the revision of the applied camera,
which is the one to believe.

Retry of a known id — `200 OK`:

```json
{ "command_id": "01J8Z6Q0K3", "status": "duplicate", "revision": 7 }
```

The server remembers recent ids for five minutes, up to 512 of them. An id older
than that is a new command, which is the right reading: the operator's hand has
moved on.

A command that would change nothing — a zoom already at its limit, a nudge of
zero, the camera it already has — is still *accepted* and spends no revision.
Coalescing is by equality, so "accepted" means "understood", and the event
stream is where "and it moved" lives.

### `camera.nudge`

Move the camera by a fraction of the viewport it last reported.

```json
{ "type": "camera.nudge", "payload": { "dx_fraction": 0.25, "dy_fraction": -0.5 } }
```

- `dx_fraction` positive moves the camera **west** (the picture moves east);
  negative moves it east.
- `dy_fraction` positive moves the camera **north**; negative moves it south.
- The sense matches dragging the map, so `0.25` is the same move as a hand
  pulling the map a quarter of a frame to the right.
- Each is bounded to ±2. The host resolves the fraction against its own
  viewport and moves the camera with the same Mercator arithmetic a drag uses,
  so a remote nudge and a gesture cannot disagree about where the same move
  ends. Over many nudges the centre can pass the antimeridian and the longitude
  grows past ±180, exactly as a drag's does; latitude stays inside ±90 because
  the inverse Mercator is bounded.
- A nudge before any viewport is known is a no-op rather than a guess: on
  `tfg-server`, send `view.report` first.

`tfgctl camera left` is `{ "dx_fraction": 0.25, "dy_fraction": 0.0 }`.

### `camera.zoom`

```json
{ "type": "camera.zoom", "payload": { "delta": 1.0 } }
```

Center-anchored: the point at the middle of the view stays in the middle. The
host clamps to its own zoom limits (3.0 to 18.0, the console's rails). `delta` is
bounded to ±10 per command.

The console's own wheel zoom is cursor-anchored, which is a different rule on
purpose: a remote client has no cursor to anchor to.

### `camera.set`

Absolute and idempotent: sending it twice leaves the camera in the same place,
and the second one publishes nothing.

```json
{
  "type": "camera.set",
  "payload": {
    "center": { "latitude": -6.2, "longitude": 106.8 },
    "zoom": 12.0
  }
}
```

Latitude is `-90..=90`, longitude `-180..=180`, zoom `-30..=30`, all finite. A
value outside those bounds is refused, not clamped: a latitude of 91 is a bug in
the client, and a server that quietly fixes it hides the bug.

### `camera.focus`

Centre the observer view on a unit the session published. This is what the
phone's numbered grid sends; it is resolved **host-side** against the session,
so a phone never needs to know where a hull is.

```json
{ "type": "camera.focus", "payload": { "unit_id": 9001, "zoom": 12.0 } }
```

- `unit_id` is the session's `units[].id` (not the grid number — the grid number
  is `index`, and the phone sends `id`).
- `zoom` is optional; omitted keeps the observer view's current zoom.
- An id the session does not carry is `422 invalid_command`; a unit with no
  position is the same, with a message saying so.
- `camera.focus` on `main` is `422`: the operator's camera is not a remote
  surface, and accepting it on `main` would let a phone drag the game master's
  map.

### `camera.focus_group`

Centre the observer view on a task group. A group has no position of its own,
so the host resolves it against the session and centres on the mean of the
placed hulls beneath it — every direct hull and every child group's hulls.

```json
{ "type": "camera.focus_group", "payload": { "group_id": 18, "zoom": 12.0 } }
```

- `group_id` is `session.groups[].id`.
- `zoom` is optional; omitted keeps the observer view's current zoom.
- An unknown group is `422 invalid_command`; a group whose hulls are all
  unplaced is the same, with a message saying so.
- It sets `ui.focus_group` and clears `ui.focus`, so the wall opens the group
  card instead of the hull card.

### `ui.set`

Open or close one of the observer's windows. The hull or group the popup is
about rides `camera.focus` / `camera.focus_group`.

```json
{ "type": "ui.set", "payload": { "panel": "crew", "open": true } }
```

- `panel` is `crew`, `scenario` or `comms`. There is no `units` window: the
  phone's unit grid is a control, not a panel, and Crew is the people and hulls
  the exercise holds.
- The state is idempotent: the same `open` twice changes nothing and advances no
  revision.
- It needs `control_view` and the readiness gate, like a nudge — opening a
  window on a wall that is not observing is a gesture into a hole.
- It is carried to every wall in `session.changed` (and in the snapshot), which
  is why it is one event with the session rather than a camera move.

### `settings.font`

The console's text size, as a step of its four-step ladder. Host-wide: it is not
addressed to a view and needs no readiness.

```json
{ "type": "settings.font", "payload": { "step": 1 } }
```

- `step` is `0..=3` (`Small`, `Normal`, `Large`, `Largest`); anything else is
  `422`.
- The console applies it and remembers it in `settings.json`. **Every attached
  command center applies it too**, from the `settings.changed` event, so a
  phone's command moves the wall it is watching and not only the operator's
  window. A headless host records it for `GET /v1/settings`.
- It is state, not a local preference: the change spends a revision, rides the
  snapshot, and is published to every watcher. Re-sending the step already in
  force is accepted and changes nothing.
- It needs `control_view`.

```json
GET /v1/settings
{ "step": 1, "text_scale": 1.15,
  "steps": [1.0, 1.15, 1.3, 1.45],
  "names": ["Small", "Normal", "Large", "Largest"] }
```

### `view.report`

A display says how big its viewport is, in device pixels.

```json
{ "type": "view.report", "payload": { "width": 1920, "height": 1200 } }
```

This exists for the headless host. A console measures its own frame and reports
the size with the camera, so on a console this command is accepted and ignored —
the window is the authority about the window. A headless server has no frame, so
a rendering client tells it one; every later `camera.nudge` resolves against the
size that was reported last.

- `width` and `height` are `1..=8192`. Zero is the state before a first frame,
  not a report.
- It needs `control_view`, because it changes what "a quarter of a viewport"
  means for everybody.
- It is a change like any other: it advances `revision` once, and a repeat of the
  same size does not.
- On the `observer` view it is exempt from the readiness gate: a waiting display
  still has to say how big it is.

### The readiness gate

Commands against `observer` are refused until observation is open:

```json
{ "error": { "code": "not_ready",
             "message": "observation is not open yet: no_display" } }
```

`409 not_ready`, with the same four reasons the read uses. The gate is
server-side because a phone that has not read `GET /v1/session` yet must not be
able to steer; `view.report` is the one exemption, and `main` is untouched (the
operator's own gestures are not a remote client).

## Pairing and devices

A pairing flow that only worked on a machine you already trusted would be
pointless, so this one is built for the LAN and for a phone:

1. The operator, holding the operator token, opens a window:
   `POST /v1/pairing`.
2. The server answers with a short code that lives for two minutes.
3. The device spends the code: `POST /v1/pair` — unauthenticated, because the
   code *is* the credential.
4. The server answers with a device token, which that device keeps.

### `POST /v1/pairing`

Needs `admin`.

```json
{ "scopes": ["observe", "control_view"], "ttl_seconds": 120 }
```

```json
{
  "code": "K7QM-3XPD",
  "expires_in": 120,
  "uri": "tfg-control://pair?url=http://192.168.1.20:8787&code=K7QM-3XPD",
  "payload": "{\"protocol\":\"tfg-control\",\"version\":1,\"url\":\"http://192.168.1.20:8787\",\"pairing_code\":\"K7QM-3XPD\"}",
  "url": "http://192.168.1.20:8787",
  "scopes": ["observe", "control_view"]
}
```

- `scopes` defaults to `["observe", "control_view"]`.
- `ttl_seconds` is clamped to `30..=600`; the default is 120.
- `admin` is refused with `422 invalid_command`. A paired device must not be able
  to pair another, or one stolen phone becomes a foothold that outlives its own
  revocation.
- Opening a window replaces any window already open: there is one code on screen
  at a time.
- `url` is the direct host base URL the offer points at; `uri` and `payload` are
  the same offer in the two shapes a QR code or a deep link would carry. **The
  server does not render a QR image**; the offer is
  text. The reference client does draw one: `tfgctl pair` prints a terminal QR
  code when its stdout is a terminal, and only the code, URL and payload lines
  when it is piped. The rendered code is byte-mode level M, versions 1–10, and
  is drawn with half-height blocks so it scans from a phone pointed at the
  screen. `--no-color` or `NO_COLOR` prints the same glyphs without the
  black-on-white SGR wrapper, which is the right choice on a light terminal.

The advertised `url` is resolved at startup: a `0.0.0.0` bind names the machine's
first non-loopback IPv4, because a payload that says `0.0.0.0` sends a phone
nowhere.

### `POST /v1/pair`

Unauthenticated. The body is the code, however it was typed, plus a name for the
device list.

```json
{ "code": "k7qm 3xpd", "label": "phone" }
```

```json
{
  "device": {
    "id": "dev-4b2379bfa714",
    "label": "phone",
    "scopes": ["observe", "control_view"],
    "created_at": 1793084168,
    "last_seen": 1793084168
  },
  "token": "9f2c…",
  "url": "http://192.168.1.20:8787"
}
```

- Case, spaces and the dash do not matter, and `I`/`L`/`O` fold to `1`/`1`/`0`
  the way Crockford's alphabet intends, because a code is read off a screen and
  typed on a phone.
- The code is single-use: it is spent by the device that used it.
- Five wrong guesses close the window (`429 too_many_attempts`), so a photograph
  of a screen is useless five tries later.
- The token is 64 hex characters and is returned **once**. It is stored only as
  a SHA-256 hash, so a copy of the devices file is not a copy of the
  credentials.

### `GET /v1/devices`

Needs `admin`.

```json
{ "devices": [ { "id": "dev-…", "label": "phone", "scopes": ["observe"], "created_at": 1793084168, "last_seen": 1793084175 } ] }
```

The stored hash is never in this answer.

### `DELETE /v1/devices/{id}`

Needs `admin`. `{ "device": "dev-…", "status": "revoked" }`. The device's token
stops working on its next request, and the deletion is written to disk, so a
restart cannot resurrect it. Revoking an id that is not there is
`404 unknown_device`.

### `POST /v1/auth/rotate`

A device retires its own token and gets a new one. Its id, label and scopes stay.

```json
{ "device": "dev-4b2379bfa714", "token": "…" }
```

Called with the operator token this is `422 forbidden`, with a message saying to
change `TFG_CONTROL_TOKEN` instead: an environment variable cannot be rotated by
a request, and pretending otherwise would leave two live credentials.

## The event stream

`GET /v1/ws` — a WebSocket, same bearer token on the upgrade request. Needs
`observe`.

On connect the server sends the whole state, then one event per change:

```json
{
  "type": "camera.changed",
  "revision": 8,
  "source": "phone-7",
  "command_id": "01J8Z6Q0K3",
  "view": {
    "id": "main",
    "camera": {
      "center": { "latitude": -6.2, "longitude": 106.8 },
      "zoom": 11.5,
      "width": 1280,
      "height": 720
    }
  }
}
```

| Event | Meaning |
| --- | --- |
| `snapshot` | The whole state at `revision`: views, session, readiness, displays. Sent on connect, and again after a gap. |
| `camera.changed` | A camera moved. `view.id` says which one. `source` names the client, or `desktop` for the operator's own gesture. `command_id` is present when a command caused it. |
| `session.changed` | The exercise, its phase, its units, the readiness verdict, the attached displays, or the observer's windows changed. One event, because they all answer the same question: may I steer, and what am I looking at? `ui` carries the open windows and the focused hull. |
| `settings.changed` | The host's text size changed. Carries `settings.step` and the resolved `text_scale`; every surface that draws type redraws at it. The same object rides `snapshot`. |
| `display.attached` | This socket's `display.hello` was accepted. Carries the id the host recorded. |
| `display.rejected` | This socket's `display.hello` was refused (a phone without `admin`, a bad role). The socket stays open. |
| `command.accepted` | A command reached the host. The evidence that it *worked* is the `camera.changed` that follows. |
| `command.rejected` | A command was refused at the boundary. The HTTP reply carried the same code. |

Only state changes advance `revision`. An acknowledgement repeats the revision
it was accepted at.

Camera changes are coalesced by equality: the console reports its camera on every
tick, and only a camera that actually differs — including a resize — publishes. A
drag that produces sixty ticks a second publishes the distinct frames, not the
idle ones. The same rule is what makes an optimistic client safe — one that
applies a command locally and then reports the result publishes nothing.

A client that falls more than 64 events behind is sent a fresh `snapshot` instead
of a guessed patch. A client can also ask at any time:

```json
{ "type": "request.snapshot" }
```

Camera commands are not accepted on the WebSocket. They go over HTTP, where a
refusal can be a reply.

### A command center is a display

A command center attaches over the same socket and says what it is:

```json
{ "type": "display.hello",
  "payload": { "id": "wall-1", "role": "command_center", "label": "wall-1",
               "viewport": { "width": 1920, "height": 1080 } } }
```

It then beats every 10 s:

```json
{ "type": "display.heartbeat", "payload": { "id": "wall-1" } }
```

Rules:

- `display.hello` needs `admin`. A paired phone is refused, which is the
  enforcement behind "only command-center mode can be remoted": the presence
  that satisfies readiness cannot be faked by the device being remoted.
- Presence is the socket. A reconnect announces again; the attachment dies with
  the socket, and a display that stops beating is reaped after 30 s.
- Re-announcing with the same id replaces the record rather than stacking a
  second one. That is also how a display reports a resize: `view.report` says
  what to resolve a nudge against, the record says what the wall can show.
- One observer view is shared by every command center. Per-display cameras are
  deferred.

At most 8 clients may watch at once; the ninth is refused with
`503 too_many_clients`. A client that disconnects does not disturb the host or
the other clients.

## Errors

```json
{ "error": { "code": "invalid_command", "message": "latitude must be finite and between -90 and 90" } }
```

| HTTP | `code` | Cause |
| --- | --- | --- |
| 401 | `unauthorized` | Missing or wrong bearer token. |
| 400 | `invalid_json` | Body is not JSON, or not a command. |
| 413 | `body_too_large` | Body over 64 KiB. |
| 422 | `invalid_command` | Parsed, but the values cannot be applied. |
| 409 | `unsupported_protocol` | `protocol_version` is not 2. |
| 409 | `not_ready` | An `observer` command before observation is open. The message names the reason. |
| 404 | `not_found` | No such route or view. |
| 503 | `host_gone` | The host's command channel is closed (the window is gone). |
| 503 | `too_many_clients` | Too many WebSocket clients. |
| 403 | `forbidden` | The credential is real; this is not its scope. |
| 401 | `invalid_pairing_code` | Wrong code. The window is still open. |
| 404 | `no_pairing_window` | No code is live on this host. |
| 410 | `pairing_expired` | The code lived out its two minutes. |
| 429 | `too_many_attempts` | Five wrong guesses closed the window. |
| 404 | `unknown_device` | No device has that id. |

Treat the code as the contract and the message as prose for a human.

## `tfgctl`

```bash
cargo run --bin tfgctl -- --server http://127.0.0.1:8787 --token "$TFG_CONTROL_TOKEN" status
cargo run --bin tfgctl -- … camera get
cargo run --bin tfgctl -- … camera left
cargo run --bin tfgctl -- … camera zoom-in
cargo run --bin tfgctl -- … camera set --lat -6.2 --lon 106.8 --zoom 12
cargo run --bin tfgctl -- … watch
cargo run --bin tfgctl -- … whoami
cargo run --bin tfgctl -- … pair --scopes observe,control_view
cargo run --bin tfgctl -- … devices
cargo run --bin tfgctl -- … revoke dev-4b2379bfa714
cargo run --bin tfgctl -- … rotate --save

# observation
cargo run --bin tfgctl -- … session status
cargo run --bin tfgctl -- … session watch [--until ready]
cargo run --bin tfgctl -- … session set --file session.json
cargo run --bin tfgctl -- … session clear
cargo run --bin tfgctl -- … units
cargo run --bin tfgctl -- … focus 3 [--zoom 12]
cargo run --bin tfgctl -- … display list
cargo run --bin tfgctl -- … display attach --id wall-1 --label wall-1 --width 1920 --height 1080
cargo run --bin tfgctl -- … panel crew on      # also scenario, comms; `panel` lists
cargo run --bin tfgctl -- … settings           # the console's text size
cargo run --bin tfgctl -- … settings font 3    # 1-4, or Small/Normal/Large/Largest
```

`session watch` polls `GET /v1/session` once a second and prints one line per
**transition**, not one per tick:

```text
waiting  no session
waiting  session 45 "Sunda Strait Watch" is planning
waiting  session 45 "Sunda Strait Watch" in execution, no command center attached
ready    session 45 · observer 1280x720 · 6 unit(s)
```

`--until ready` exits `0` at the first ready line, which is what a script or a
waiting UI wants. `--interval 500ms` changes the poll.

`focus 3` sends `camera.focus` for grid number 3 on the `observer` view;
`--view main` addresses the operator's view explicitly (and is refused by the
readiness gate and the focus rule). `display attach` opens a WebSocket, sends
`display.hello`, reports a viewport, heartbeats, and prints readiness
transitions — it simulates a command center, so the whole server-side feature is
testable before a wall exists.

Flags: `--server`, `--token`, `--json` (stable JSON on stdout, one object per
line), `--timeout 3s`. Configuration falls back to `TFG_CONTROL_URL` and
`TFG_CONTROL_TOKEN`, then to `~/.config/tfgctl/config.json`:

```json
{ "server": "http://127.0.0.1:8787", "token": "…" }
```

A token in that file is a convenience for a loopback bind; on a LAN pass it in
the environment instead. `rotate --save` writes the new token there with mode
`0600`.

`watch` prints one line per event — `camera.changed r8 -6.20000, 106.80000 at
zoom 11.50 from phone-7 (01J8Z6Q0K3)` — or the raw event with `--json`.

`pair` opens a window, prints the code, and waits for a device to use it, so an
operator running `tfg-server` in a terminal has one command for the whole
exchange. It cannot show the token it hands out: that goes to the device.

The CLI is a reference client, not an API the mobile app shells out to. The
phone speaks this protocol directly; [`mobile-client-quickstart.md`](mobile-client-quickstart.md)
is the build brief for that client.

## `tfg-server`

```bash
cargo run --bin tfg-server -- --listen 127.0.0.1:8787
cargo run --bin tfg-server -- --listen 0.0.0.0:8787 --token "$(openssl rand -hex 32)"
cargo run --bin tfg-server -- --ephemeral --devices /tmp/devices.json
cargo run --bin tfg-server -- --lat 51.5 --lon -0.12 --zoom 9
```

| Flag | Meaning |
| --- | --- |
| `--listen` | Default `127.0.0.1:8787`. |
| `--token` | The operator token. **Generated and printed when absent on loopback**; required when binding anything else, because a generated credential on the LAN is a credential printed to a terminal. |
| `--devices` | The device table. Default `$XDG_DATA_HOME/tfg/control-devices.json`. |
| `--ephemeral` | Keep the table in memory; nothing survives exit. |
| `--lat` / `--lon` / `--zoom` | Where to start. Defaults to the console's opening camera. |

It prints the listen URL, the address a client on the LAN should use, the
devices file, the starting camera, and one line per applied command:

```
r6  camera.nudge +0.25,+0.00 51.50000, -1.05384 zoom 9.00  from tfgctl (cli-…)
    camera.set 51.5000,-0.1200@9.00 no change                from tfgctl (cli-…)
```

What it is: a headless owner of the camera's *state*, for clients that want the
map to have one place everybody agrees on. What it is not: a renderer. It has no
tiles, no marks and no picture, so `camera.changed` from `tfg-server` means
"the shared camera moved", not "somebody redrew".

One host per devices file. Two processes sharing it each hold their own copy in
memory and the last writer wins, so a console and a server should not both run
against the same path at the same time. `--ephemeral` exists partly for that.

## A console in client mode

A console can be the display instead of the host:

```bash
TFG_CONTROL_SERVER=http://127.0.0.1:8787 \
TFG_CONTROL_TOKEN="$(openssl rand -hex 32)" \
cargo run
```

### The command center

`--mode command-center` turns the same binary into the wall a phone remotes:

```bash
TFG_CONTROL_SERVER=http://192.168.1.15:8787 \
TFG_CONTROL_TOKEN="$OPERATOR_TOKEN" \
TFG_DISPLAY_ID=wall-1 TFG_DISPLAY_LABEL=wall-1 \
cargo run -- --mode command-center
```

It renders the `observer` view and nothing else: no roster, no composer, no
Minos credential. It branches before the keyring and the sign-in, so that claim
is structural. It announces itself as a `command_center` display (which needs
`admin`), beats every 10 s, and re-announces when its window resizes.

What it shows: a top strip with the exercise, the scenario step, the unit count
and the observer zoom; the map at the observer camera; numbered markers for the
units that fall on screen; and, while observation is not open, a card naming the
one thing it is waiting for. It takes no camera gesture of its own — it is a
display, not a controller.

It opens no listener and reads no device table. On launch it connects to
`/v1/ws` with the same bearer token, takes the host's snapshot, and moves the
map to the camera in it. After that, every `camera.changed` the host publishes
is applied through the shell's own `move_camera`, so following a remote nudge
and dragging the map are the same code path.

The console posts only its **own** camera:

- A gesture (or a resize) becomes `camera.set` with the camera the shell is
  already showing, and `view.report` when the viewport changed. A report equal
  to the camera the host already has is dropped before it becomes a request, so
  the host's equality coalescing is not asked to absorb a stream of no-ops.
- A camera the host asked for is never posted back. The shell's report of that
  move is `followed` rather than `camera` on the client side; it clears the
  client's pending state and may correct the host's viewport, but it cannot
  become a command.
- Reports from a tick that predates the shell applying a remote camera are
  discarded until the shell catches up. Otherwise a slow tick could post the old
  camera over the move it just followed.
- Until the first snapshot of a connection arrives, camera reports are dropped:
  there is nothing to compare them against and no authority to tell. A snapshot
  resets the revision; a stale `camera.changed` from before it is ignored.

The client reconnects with backoff (250 ms doubling to 10 s) and resets its
revision on every connection. A host that is not up yet is not a launch error:
the console opens on its own camera and joins when the host answers. A malformed
URL, or an `https://` URL (there is no TLS yet), disables the client with a
message naming the reason; the console still opens locally, because a remote
host is a convenience and never a reason for the map not to open.

A paired device can also be a console. Following needs only `observe`;
reporting the display's viewport and its gestures needs `control_view`, so a
watch-only device follows the map and is refused with `403` if it tries to
steer — the correct answer for a wall display nobody can move.

## What the desktop keeps to itself

The protocol carries state, not widgets. These are never serialized and never
remote-controlled: focus, hover, drag gestures, caret blink, open pickers, panel
layout, the map texture, and transient animation. A remote client asks for a
camera; it does not click a button.

## What is not here yet

- **QR image files.** The offer carries a URI and a JSON payload, and
  `tfgctl pair` draws the payload as a terminal QR code for a phone to scan.
  Writing an image file (PNG/SVG) is left to a client.
- `marker.changed`. Markers ride `session.changed`; a separate event would say
  the same thing twice.
- Per-display cameras. One `observer` view is shared by every attached command
  center.
- A verb for the phone's OK button. The button is inert until the protocol has
  something for it to mean.
- The OK button's verb. The phone's button is inert until the protocol has
  something for it to mean.
- `GET /v1/commands/{id}` for polling a command's fate. The event stream is the
  only way to see one applied.
- TLS. `wss` and `https` are refused by `tfgctl watch` with a message rather
  than a silent downgrade.
- mDNS discovery. Pairing needs an address; the operator supplies one.
