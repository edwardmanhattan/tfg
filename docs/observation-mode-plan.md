# Observation mode: the plan

The phone is not a second operator. It is a viewer for the **execution phase**
of one exercise, holding a camera that belongs to a **command-center display**,
not to the game master's console. This document is the build plan for that.

Everything here is between TFG processes. The TFG control server never calls
Minos, and no mobile client ever sees a Minos credential. The main console is
the only process that reads Minos; it publishes a projection of what it already
holds.

---

## 1. The shape

```text
                    Minos (game backend)
                          │
                          │  only the main console reads it
                          ▼
  ┌───────────────────────────────────┐
  │ main tfg (operator)               │
  │  • holds the session              │
  │  • owns the `main` camera         │
  │  • publishes session + units      │
  └──────────────┬────────────────────┘
                 │  PUT /v1/session  (or the in-process hub)
                 ▼
  ┌───────────────────────────────────┐
  │ TFG control server                │
  │  • session, units, readiness      │
  │  • `main` and `observer` cameras  │
  │  • command-center presence        │
  └───┬───────────────┬───────────────┴───────────────┐
      │               │                               │
      ▼               ▼                               ▼
 command center   phone (Flutter)                 tfgctl
 (same tfg binary,  observer remote              reference client
  --mode            (observe +                  and test harness
  command-center)    control_view)
```

The server is either the one embedded in the main console
(`TFG_CONTROL_LISTEN`) or a headless `tfg-server`. A command center always
follows; it never hosts.

## 2. Decisions

**D1 — The observer camera is its own view.** The hub already models views as a
map. `main` stays the operator's camera; `observer` is the one the phone moves
and the command center renders. The phone can pan the observation without
dragging the game master's screen around. `views` is a map, so per-display
cameras are a later additive change.

**D2 — The session is shared state, published by the console.** The console
already holds the game id, phase, scenario, hulls, commanders, embarkations and
placements. It publishes a projection. It does not make the server read Minos,
and it does not send Minos tokens anywhere.

**D3 — Readiness is computed, never asserted.** A phone is ready only when all
of these hold:

1. a session exists,
2. its phase is `execution`,
3. at least one command-center display is attached,
4. the session was refreshed within 15 seconds.

The server sends the verdict **and the reason**, so every waiting screen can say
what it is waiting for.

**D4 — A command center is an authenticated display.** It connects to `/v1/ws`,
sends `display.hello` with `role: "command_center"`, and heartbeats. The message
requires the `admin` scope, which a paired phone never has. That is the
enforcement behind "only command-center mode can be remoted".

**D5 — Presence is the socket; liveness is a heartbeat.** A display is present
for as long as its socket lives, refreshed every 10 s and reaped after 30 s. A
session expires after 15 s without a refresh. A crashed console stops being
ready instead of freezing the phone on a dead map.

**D6 — One observation, many command centers.** Every command center mirrors the
single `observer` view. The phone does not choose a target in this version. If
two command centers have different window sizes, the last `view.report` wins and
that is what nudges resolve against; the plan is to require the same size or add
per-display views later.

**D7 — Protocol envelope 2, routes stay `/v1/`.** The event set changes shape
and grows. Bump `PROTOCOL_VERSION` to 2; `GET /v1/hello` reports it. No
compatibility shim: all clients in this repo are updated together. The route
version and the envelope version were separate from the start for exactly this.

**D8 — `camera.focus` is resolved by the host.** The phone sends a unit id, not
coordinates. The host looks the unit up in the session it owns and centres the
observer camera on the latest position. A stale position on the phone cannot
focus the wrong place, and the command is one small message.

**D9 — Command center mode is a separate, small GPUI surface.** The console
shell is 45k lines of panels that command-center mode must hide. Hiding them all
is more work and more risk than a new module with the map, a session strip, the
unit markers and a waiting card. Same binary, different branch taken before
Minos or the keyring is touched.

**D10 — The OK button has no protocol yet.** It renders inert until there is a
verb for it. Do not invent one to fill the wireframe.

## 3. The wire

### 3.1 `GET /v1/hello`

```json
{
  "protocol_version": 2,
  "api_version": "v1",
  "service": "tfg-control",
  "revision": 9,
  "pairing_open": false
}
```

### 3.2 Snapshot

```json
{
  "type": "snapshot",
  "protocol_version": 2,
  "revision": 9,
  "views": {
    "main":     { "id": "main",     "camera": { "center": { "latitude": -6.108, "longitude": 106.91 }, "zoom": 11.0, "width": 1280, "height": 720 } },
    "observer": { "id": "observer", "camera": { "center": { "latitude": -6.108, "longitude": 106.91 }, "zoom": 11.0, "width": 1920, "height": 1080 } }
  },
  "session": {
    "game_id": 42,
    "name": "Exercise North",
    "phase": "execution",
    "mode": "maneuver",
    "scenario": { "id": 7, "title": "Phase 2", "step": "Secure the strait" },
    "units": [
      {
        "index": 1,
        "id": 123,
        "name": "Evertsen",
        "hull_number": "F805",
        "class_name": "Frigate",
        "commander": { "id": 9, "name": "CDR Aldana" },
        "embarked": [ { "id": 321, "name": "NH90", "hull_number": "N-110" } ],
        "embarked_in": null,
        "position": { "latitude": -6.2, "longitude": 106.8 },
        "heading_deg": 92.0,
        "speed_kn": 14.0,
        "stale": false
      }
    ]
  },
  "readiness": { "ready": true, "reason": null },
  "displays": [
    { "id": "wall-1", "label": "wall-1", "role": "command_center", "viewport": { "width": 1920, "height": 1080 } }
  ]
}
```

`session` is `null` when no exercise is held. `observer` is absent until a
session is ready; clients wait rather than invent a camera.

`readiness.reason` is one of `no_session`, `not_execution`, `no_display`,
`stale`, or `null` when ready.

### 3.3 Events

```json
{ "type": "camera.changed", "revision": 10, "source": "phone-1",
  "command_id": "phone-1-7",
  "view": { "id": "observer", "camera": { "...": "..." } } }
```

```json
{ "type": "session.changed", "revision": 11,
  "session": { "...": "..." },
  "readiness": { "ready": false, "reason": "no_display" },
  "displays": [] }
```

`session.changed` covers session data, phase, unit list and presence. One event
for the whole observation state means one handler in the phone and one place to
look when the screen is wrong.

`display.attached` and `display.rejected` answer a `display.hello`. A live
display sends `display.heartbeat` every 10 s:

```json
{ "type": "display.hello", "id": "wall-1", "label": "wall-1", "role": "command_center" }
{ "type": "display.heartbeat" }
{ "type": "display.attached", "id": "wall-1" }
{ "type": "display.rejected", "code": "forbidden", "message": "display attachment needs the admin scope" }
```

The first attached display creates the `observer` view. That creation broadcasts
a `camera.changed` for `observer` (so every already-connected phone learns the
viewport) and sends the attaching display a fresh snapshot (so it has the view it
is about to render). A second `snapshot` after `attached` is ordinary, not an
error.

### 3.4 Writing the session

`PUT /v1/session` — **admin**. The body is the `session` object above:

```json
{
  "game_id": 42,
  "name": "Exercise North",
  "phase": "execution",
  "mode": "maneuver",
  "scenario": { "id": 7, "title": "Phase 2", "step": "Secure the strait" },
  "units": [ { "index": 1, "id": 123, "...": "..." } ]
}
```

`DELETE /v1/session` clears it. `GET /v1/session` is the same object for a CLI
that does not want the whole state. `GET /v1/displays` lists the attached
command centers.

The host PUTs at most once a second while anything changed, and at least every
5 s as a heartbeat. The hub coalesces by equality; a heartbeat updates
`last_seen` without advancing the revision.

The hub assigns `index` from array order, so the phone's grid numbers and the
command center's markers agree. The array order is the session's unit order.

### 3.5 Commands

The envelope gains `view`, defaulting to `"main"`:

```json
{
  "protocol_version": 2,
  "view": "observer",
  "client_id": "phone-7",
  "command_id": "phone-7-42",
  "type": "camera.focus",
  "payload": { "unit_id": 123, "zoom": 14.0 }
}
```

`zoom` is optional and clamped to the rails. The existing `camera.nudge`,
`camera.zoom`, `camera.set` and `view.report` all take `view` the same way.

- An unknown view is `422 invalid_command`.
- A focus on an unknown or unpositioned unit is `422`, naming the unit.
- A phone never sends `view.report`: the observer viewport belongs to the
  command center, not to the phone's screen.
- A phone's drag is normalised by the **observer** view's width/height, so a
  quarter-drag is a quarter of the wall display.

### 3.6 Auth

| Route / message | Scope |
| --- | --- |
| `GET /hello`, `POST /pair` | none |
| `GET /state`, `/session`, `/displays`, `/ws` | `observe` |
| `POST /commands` (any view) | `control_view` |
| `PUT`/`DELETE /session` | `admin` |
| `display.hello` | `admin` |

The console and command centers use the operator token. A phone is paired with
`observe,control_view` and cannot publish a session or register a display.

## 4. The hub

`src/control/hub.rs` grows from "a camera and a revision" to "the observation
state".

```rust
pub struct SessionState {
    pub game_id: i64,
    pub name: String,
    pub phase: String,          // the server's own word; `is_execution()` is a method
    pub mode: String,
    pub scenario: Option<ScenarioState>,
    pub units: Vec<UnitState>,
}

pub struct UnitState {
    pub index: u32,
    pub id: i64,
    pub name: String,
    pub hull_number: String,
    pub class_name: String,
    pub commander: Option<CommanderState>,
    pub embarked: Vec<UnitRef>,
    pub embarked_in: Option<UnitRef>,
    pub position: Option<GeoPoint>,
    pub heading_deg: Option<f64>,
    pub speed_kn: Option<f64>,
    pub stale: bool,
}

pub struct Readiness {
    pub ready: bool,
    pub reason: Option<ReadinessReason>,   // no_session | not_execution | no_display | stale
}

pub struct DisplayState {
    pub id: String,
    pub label: String,
    pub role: String,
    pub viewport: Option<(u32, u32)>,
}
```

```rust
impl ControlHub {
    pub fn set_session(&self, session: SessionState) -> Option<u64>;
    pub fn clear_session(&self) -> Option<u64>;
    pub fn touch_session(&self) -> Option<u64>;      // heartbeat; no revision bump
    pub fn expire(&self) -> bool;                     // reaps stale sessions/displays
    pub fn attach_display(&self, display: DisplayState) -> Option<u64>;
    pub fn touch_display(&self, id: &str);
    pub fn detach_display(&self, id: &str) -> Option<u64>;
    pub fn readiness(&self) -> Readiness;
    pub fn report_camera(&self, view: &str, camera: CameraSnapshot, source: CameraSource) -> Option<u64>;
    pub fn apply(&self, view: &str, command: &Command, source: CameraSource) -> Result<Option<u64>, ApplyError>;
}
```

`ApplyError` is `UnknownView`, `UnknownUnit`, or `UnitHasNoPosition`; the server
maps all three to `422 invalid_command` with a specific message.

Rules to test:

- Setting an equal session (same data) does not bump the revision.
- A heartbeat never bumps it.
- A session going stale bumps once and flips `readiness`; a fresh PUT bumps back.
- Attaching the first command center flips `no_display` to `ready` and creates
  the `observer` view from `main` if it does not exist.
- Detaching the last command center flips back to `no_display` but keeps the
  observer camera so a reconnect returns to the same place.
- A new `game_id` resets the observer camera to `main`'s centre and zoom.
- Focus on a unit with a position moves the observer camera; focus on a unit
  without one is an error, not a no-op.
- `view.report` on `observer` changes the observer viewport and every later
  nudge resolves against it.

## 5. The console's publisher

The shell already knows the session; the registry already knows positions. They
live on different threads today, so the publisher is the seam.

```rust
// src/control/publish.rs
pub enum SessionUpdate {
    Roster(Option<SessionState>),      // no positions; the publisher fills them
    Positions(Vec<(i64, UnitPosition)>),
}

pub enum SessionSink {
    Hub(ControlHub),                   // the console is the host
    Http { server: String, token: String },  // the console follows tfg-server
    Off,
}

pub fn spawn_session_publisher(sink: SessionSink, rx: Receiver<SessionUpdate>);
```

- **Shell → binary.** A new `ToBinary::Session(Box<SessionState>)` sent at most
  once a second when a cheap key changes: `(game_id, state, step, pieces'
  ids/commanders/hosts)`. It is built from `SessionView` and `PieceView`, which
  the shell already holds. `index` comes from the array order.
- **Fix thread → publisher.** Once a second, the map feed sends the latest
  position for every unit id in the roster, with `stale` from the registry and a
  fallback to the recorded placement before the first fix.
- **Publisher.** Composes roster + positions, compares with the last sent, and
  writes the hub or PUTs. Heartbeat every 5 s. A `Roster(None)` clears once.
- **On session change.** A different `game_id`, or leaving execution, is a
  normal update; readiness handles the rest. No special "stop" call.

The session's scenario is the playing step when the clock has one:
`scenario.id`, the scenario's title from the book, and the step content. If no
step is playing, send the scenario alone with `step: null`.

## 6. Command-center mode

```bash
tfg --mode command-center \
  --server http://192.168.1.15:8787 \
  --token "$TFG_CONTROL_TOKEN" \
  --label wall-1
```

`--server`/`--token` fall back to `TFG_CONTROL_SERVER`/`TFG_CONTROL_TOKEN`.
The branch is taken in `main` **before** `AppPaths`, the keyring, the roster read
or `fetch_session`. A command center with no Minos access must work.

What it does:

1. Starts the map thread and a GPUI window.
2. Connects with the existing control client, configured for
   `view: "observer"` and `display: { role: "command_center", label }`.
3. Sends `display.hello` on every connection, heartbeats every 10 s.
4. Reports its viewport with `view.report` when the window resizes.
5. Renders the map, numbered unit markers, and a session strip.

What it never does: read Minos, sign in, show the roster, the composer, the
clock, the inbox or any other console panel. No camera gestures: it is a display.
A mouse drag on it does nothing.

Waiting is a first-class screen. Before `readiness.ready`, the window shows the
map dimmed and a card naming the reason:

| reason | copy |
| --- | --- |
| `no_session` | "Waiting for the game master to start an exercise." |
| `not_execution` | "Session 42 is in planning. Observation opens at execution." |
| `no_display` | "Connecting this command center…" (normally a blink) |
| `stale` | "The host stopped reporting. Reconnecting…" |
| unreachable | "Cannot reach the control server at …" |

The session strip after ready: exercise name, scenario/step, unit count, live
state, and the observer camera's scale.

## 7. `tfgctl`

The CLI is the test surface until the phone exists.

```bash
tfgctl session status                 # session, phase, readiness, displays
tfgctl session watch [--until ready]  # transitions, exits 0 when ready
tfgctl session set --file session.json
tfgctl session clear
tfgctl units                          # index, hull, commander, position, embarked
tfgctl focus 3                        # camera.focus on the observer view
tfgctl focus dev? --view main         # explicit view
tfgctl display                        # list attached command centers
tfgctl display attach --label wall-1  # simulate one, stay attached
```

`display attach` is the important one: it opens a WS, sends `display.hello`,
reports a viewport, heartbeats, and prints readiness transitions. That makes the
whole server-side feature testable before the command center shell exists, and
it doubles as the CLI's waiting interface.

`session watch` output, piped or on a TTY:

```text
waiting  no session
waiting  session 42 "Exercise North" is planning
waiting  execution, no command center attached
ready    session 42 · observer 1920x1080 · 6 units
```

`session set --file` takes the same JSON as `PUT /v1/session`, so fixtures are
shared between the CLI, the smoke script and the mobile guide.

`camera` commands gain `--view main|observer`, default `main`; `focus` defaults
to `observer` because that is what it is for.

## 8. The phone's side (notes for the Flutter build)

The quickstart gets a v2 section. The rules the app must follow:

- Parse `protocol_version == 2` from `/hello`; refuse anything else with a
  readable message.
- Read `session`, `readiness`, `displays` from the snapshot, then handle
  `session.changed` and `camera.changed` for `view.id == "observer"`.
- **Gate every control on `readiness.ready`.** When it is false, show the reason
  and the waiting copy; do not send commands into a `403`/`422` loop.
- The unit grid is `session.units` in order, labelled by `unit.index`. Key rows
  by `unit.id`, not by index, because the list can change.
- Tapping a unit sends `camera.focus` with `view: "observer"` and opens the info
  window from the same `UnitState` (commander, embarked, position, course/speed).
- A drag sends `camera.nudge` with fractions of the **observer** viewport from
  the snapshot, not of the phone's screen. Positive `dx` is west, positive `dy`
  is north.
- When `readiness.ready` flips false, freeze controls and keep the device token;
  pairing is not the remedy for "waiting".
- `GET /v1/state` and the snapshot carry everything; the phone never needs
  Minos, `tfgctl`, or SSH.

Scenario and Comms panels: the session carries the scenario title and current
step, so Scenario can render real content immediately. Comms has no published
content in this phase; the panel should say "not published" rather than show an
empty inbox that reads as "no traffic".

## 9. Build order

Each phase ends in something observable. Do not start the next one before the
previous one's check passes.

**Phase 0 — contract.**
Write this plan into `docs/adr/0026-observation-mode.md` (or an ADR that carries
the decisions) and update `docs/control-api-v1.md` with the v2 shapes. No code.
Done when the event names, field names, readiness reasons and auth table are
fixed.

**Phase 1 — protocol + hub.**
`SessionState`, `UnitState`, `Readiness`, `DisplayState`, `Command::CameraFocus`,
`CommandRequest.view`, `PROTOCOL_VERSION = 2`, `OBSERVER_VIEW`, the hub methods
and their tests. Done when `cargo test --lib control::` passes and the new tests
cover every rule in §4.

**Phase 2 — server + CLI.**
Routes, `display.hello`/heartbeat, presence guard, expiry task, `session`,
`units`, `focus`, `display attach`, `session watch`. Extend
`scripts/control-server-smoke.sh`. Done with a transcript against `tfg-server`:
publish a fixture, see `no_display`, attach a display, see `ready`, focus unit 3,
watch the observer camera move, detach, see `no_display`.

**Phase 3 — console publisher.**
`ToBinary::Session`, the shell projection, the fix-thread positions, the
publisher thread, host and HTTP sinks, heartbeats. Run the console against a real
execution session and a `tfgctl display attach`; `tfgctl units` shows the real
hulls, `focus` moves only the observer view (`camera get --view main` unchanged),
and closing the console flips the phone to `stale` then `no_session`.

**Phase 4 — command center.**
The new GPUI module, client `view`/`display` support, waiting card, map,
numbered markers, viewport report. Done with a driven binary and a capture of
both states (waiting and ready) per `docs/agents/verification.md`.

**Phase 5 — mobile handoff.**
Update `docs/mobile-client-quickstart.md` to v2, add
`fixtures/observation-session.json` and a curl/`tfgctl` transcript that the
friend can run without the command-center UI.

**Phase 6 — deferred.**
Comms content, full scenario book, per-display cameras, the OK button. Only
after the first five phases are in use.

## 10. Verification plan

- **Unit:** hub rules in §4; protocol round-trips; validation of focus and view.
- **Server:** smoke script with a fake session, attach/detach, expiry, scope
  refusals (`403` for a phone token on `PUT /v1/session`), `display.hello` as a
  non-admin.
- **Console:** driven host with a real session; `tfgctl` sees it; operator camera
  unaffected by observer commands.
- **Command center:** capture the waiting card and the ready map; assert the
  session strip says the right exercise and that markers land on the unit
  positions.
- **Phone:** the quickstart transcript as an acceptance test, then the Flutter
  checklist.

## 11. Open questions

1. **Comms.** Observers seeing the exercise's message traffic is a policy
   decision, not a protocol one. Phase 1 publishes none; should the console
   relay the inbox, or is Comms a local-only panel for now?
2. **Scenario depth.** Current scenario + step is cheap. Does the panel need the
   whole book (all scenarios and steps), or only what is playing?
3. **Multiple command centers.** One mirrored `observer` view is the simple
   answer. Is there a case for two walls showing different places, which would
   make the phone choose a display?
4. **Observer start.** On readiness, the observer camera copies `main`'s centre
   and zoom. Is that the right opening shot, or should it open on the session's
   first scenario or a fleet centroid?
5. **Command-center credential.** Command centers use the operator token today.
   Is distributing that token to every wall acceptable, or should there be a
   `display` scope minted by pairing?

## 12. Status

Phases 1–5 are built and uncommitted in the working tree. The design they
implemented is recorded in
[`docs/adr/0026-observation-is-a-second-view.md`](adr/0026-observation-is-a-second-view.md);
three details came out of building it and are worth writing down here.

- **The publisher has a `told` gate.** A console that starts while another
  holds the session must not clear it. It publishes nothing until the shell has
  told it what this console holds, and a clear is only ever sent for a session
  it published.
- **A display re-announces when it resizes.** `view.report` is what a nudge
  resolves against; the display's *record* is what `display list` and the
  waiting screen read. Only `display.hello` carries a viewport, so a resize is a
  second hello with the same id, and the host replaces the record.
- **The wall's camera size is the window's, not the view's.** A view's
  `width`/`height` are whichever display last reported them; the command center
  copies only centre and zoom from the observer view and keeps its own size.

Verification done: `control-server-smoke.sh` 47/47; `cargo test --lib control::*`
(protocol, hub, server, client, publisher) and `cargo test --bin tfgctl` green;
`cargo check --all-targets` clean. Driven captures of the command center in its
waiting and ready states, plus a live `focus` that moved `observer` and left
`main` alone. Ready state was proven with `fixtures/observation-session.json` on
a headless host; no live Minos execution exercise was available locally.

The phone's windows, the hull pictures and the unit card were added after the
first pass, against a live Minos exercise: `ui.set` (crew/scenario/comms) is host
state, `GET/PUT /v1/units/{id}/picture` carries the console's photographs, and
the projection now includes participants, messages, and the hull facts a mark
needs (`symbol`, `affiliation`, `loa_m`, `beam_m`).

Two lib tests fail on this tree and are unrelated to observation:
`map_render::tests::quad_bow_follows_the_compass` (a float tolerance: 180.0000025
against 180) and `sim::tests::order_move_applies_then_higher_overrides_loudly`.
Both files are untouched by this work.
