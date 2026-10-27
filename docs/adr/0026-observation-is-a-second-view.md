# ADR-0026: Observation is a second view, and readiness is computed

The phone remote from ADR-0025 could move one camera: the operator's. That is
the wrong surface for an observer. An observer is not a second game master —
they watch a running exercise from the wall, and the wall is not the operator's
desk. Handing them `main` would mean every nudge fights the person running the
game.

This ADR fixes the shape of observation mode. It builds on ADR-0025 (the control
plane is a hub) and supersedes one sentence of it: *"one view, named `main`."*

## Status

Accepted. Protocol v2. Phases 1–5 of
[`docs/observation-mode-plan.md`](../observation-mode-plan.md) are built: the
wire types, the hub's session/readiness/displays, the server routes and the
readiness gate, the console's publisher, the command-center mode, and the
mobile handoff document. The Flutter client itself is not written.

## Context

An exercise has a **phase** (planning, preparation, execution, closure), and
observation is only meaningful during execution. The console already knows all
of it: the game id, the scenario, the hulls, their commanders, their fixes. The
server knows none of it and must not learn it by asking Minos — the console is
the only process that holds a Minos credential, and a second reader of the game
backend would be a second authority over it.

Meanwhile the phone cannot draw the map. The protocol carries a camera and a
place, never tiles. So the picture has to exist somewhere the observer can see
it: a screen in the room, showing what the phone is steering.

Three questions had to be answered together:

1. **Which camera does the phone move?** If it is the operator's, observation is
   a hostile takeover of the console.
2. **When may it move it?** A phone that steers before the exercise starts is
   steering nothing, and a wall that says "live" over a planning screen is
   lying.
3. **Who counts as a wall?** "Only a command center may be remoted" is a
   property of the system, not a convention the operator remembers.

## Decision

### The observer camera is its own view

`observer` is a view like `main`, with its own camera. A phone command carries
`"view": "observer"`; the host applies it there, and `main` is unreachable from
a phone (`camera.focus` on `main` is `422`). The wall renders `observer`.

The two cameras start equal — the observer opens on the operator's shot — and
then diverge the first time anybody steers. That is the point: the operator's
map is a private workspace, and the wall is the shared one.

One `observer` view is shared by every attached command center. Per-display
cameras are deferred; a room with two walls showing different places is a
different feature with a different routing question.

### The session is a projection, published by the console

`SessionState` is what a viewer needs — game id, name, phase, scenario, units
with commanders, embarked hulls and fixes — and nothing else. The console
composes it from what it already holds and `PUT`s it; a heartbeat every 5 s keeps
it fresh. The server never calls Minos.

The console publishes **only on change**, and its publisher has a `told` gate:
a console that starts beside another must not delete a session it never
published. A publisher that has not been told anything writes nothing, and a
clear is sent only when the console lets go of an exercise it held.

A session that stops being refreshed is kept and reported `stale`, not deleted.
A stale map beats an empty one, and `no_session` must mean "the console holds
nothing", not "the console paused".

### Readiness is computed from four facts

Observation is open only when all of these hold:

1. a session exists,
2. its phase is `execution`,
3. at least one `command_center` display is attached,
4. the session was refreshed within 15 seconds.

The verdict travels as `{ready, reason}` with `no_session | not_execution |
no_display | stale`. Every waiting surface — the CLI's `session watch`, the
command center's card, the phone — renders the same four reasons. No surface
asserts readiness of its own.

### A command center is a display, and display presence needs `admin`

A command center attaches on `/v1/ws` and sends `display.hello` with
`role: "command_center"`, then heartbeats every 10 s; it is reaped after 30. The
attachment dies with the socket.

That message requires `admin`, which a paired phone never has. This is the whole
enforcement of "only a command center may be remoted": the device being remoted
cannot be the device that makes itself eligible.

### The gate is server-side

Observer commands are refused `409 not_ready` until readiness holds. `view.report`
is exempt (a waiting display still has a size); `main` is untouched.

Observer commands are resolved inside the hub — `camera.focus` is looked up
against the session's units, not passed through — so they never reach the
console's command channel. The console's own gestures still flow the old way.
The host is the authority about its own units, and a phone never sends
coordinates.

### Protocol v2, routes unchanged

The snapshot carries `session`, `readiness` and `displays`; `session.changed`
joins the event stream; `camera.focus` joins the commands. The envelope version
is `2`. The routes stay under `/v1/`.

This is a clean break. The project is in development, every in-repo client moved
in the same wave, and a v1 body is refused rather than half-read.

### The command center is a mode of the same binary

`tfg --mode command-center` branches before the keyring and the Minos sign-in, so
a wall cannot hold a game session even by accident. It reuses the map thread and
the frame slot, renders the observer view with numbered markers, and takes no
camera gesture. Its waiting card is a pure function with tests, because the copy
is part of the contract.

## Consequences

- **`main` and `observer` can disagree, by design.** An operator who moves their
  own map while a phone moves the wall sees two different places. That is the
  separation working, and the wall's strip names the exercise so nobody confuses
  the two.
- **The wall needs the operator token today.** `display.hello` requires `admin`.
  Distributing that token to every wall is a real cost; a `display` scope minted
  by pairing is the obvious next step and is recorded as an open question rather
  than guessed at.
- **Readiness can flap on a slow console.** A console that cannot refresh within
  15 s makes every waiting screen say `stale`. The heartbeat is 5 s for that
  reason; the cost of a wrong `stale` is a card, and the cost of a wrong `ready`
  is a viewer steering an exercise that is not running.
- **A crashed console leaves a stale session, not an empty one.** Units and
  fixes stay visible. The phone greys them out; it does not empty the grid.
- **One observer camera for every wall.** Two command centers attached show the
  same place. Choosing a display is not in the protocol.
- **Units at the same position overlap on the wall.** An embarked helicopter
  shares its parent's fix, so the markers stack and the later one wins. The grid
  is the complete list; the map is the glance.
- **The OK button does nothing.** There is no verb for it yet. Shipping an inert
  button is honest; inventing a command so the button feels alive is not.
- **Comms publishes nothing in this phase.** The panel exists on the phone as a
  placeholder. Whether observers may see message traffic is a policy decision,
  not a protocol one, and it is still open.

## Verified

- `scripts/control-server-smoke.sh` — 41 checks, including attach/detach, expiry,
  `403` for a phone on `PUT /v1/session`, a non-admin `display.hello`, the
  readiness gate, and `camera.focus` moving only `observer`.
- `cargo test --lib control::` — protocol round-trips, hub rules (stale-not-gone,
  headless gating, focus resolution), server routes and scopes, publisher sinks
  and its `told` gate, client attach/heartbeat/viewport.
- Driven: a headless host plus `fixtures/observation-session.json` and a real
  command-center window captured in `/tmp` during development — waiting card,
  ready map with the session strip and markers, and a live `focus 1 --zoom 12`
  that moved `observer` to `-5.9123, 105.8412 z12` while `main` stayed at
  `-6.2, 106.8 z11`.
- Not verified against a live Minos execution exercise: no local Minos was
  available during the build, so the ready state was proven with the published
  fixture on a headless host.
