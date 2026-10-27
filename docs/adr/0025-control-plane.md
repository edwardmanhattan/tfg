# ADR-0025: The control plane is a hub, and the desktop is its first host

A second operator on the same exercise had no way to move the map. The console
is one process with one window: the camera, the hulls, the session and the map
all live inside the GPUI shell, and the only way to see what the operator sees
is to stand behind them. A phone in the same room, a `curl` on the next desk, a
second screen on the wall — none of them can ask the console to look somewhere.

This ADR fixes the seam. It does not build the final topology.

## Status

Accepted. Carries forward ADR-0020 (the console is one GPUI shell) and
ADR-0017 (local-first drafts, written through at an advance). Stages 0–7 of the
migration are built: the protocol, the hub, the embedded server, device pairing
with scopes, the toolkit-free reducer, and the headless `tfg-server`. The console
in *client* mode — the desktop following a remote host as a display — is
recorded here as the target and is not built.

## Context

Three things are easy to conflate, and conflating them is how a remote-control
feature becomes a rewrite:

1. **Minos**, the external game backend. It owns the authoritative game:
   positions, orders, readiness, scenarios, permissions. Nothing here changes
   that.
2. **The TFG control plane.** It owns *view* state — where the camera is, what
   the map is showing — and the authority to accept or refuse a request to
   change it.
3. **TFG clients.** The desktop console, a CLI, and a future phone. They should
   all speak one versioned protocol.

The existing code already had the seams this needs, which is why it did not
have to be rewritten:

- `shell_gpui.rs` owns the camera and moves it through `move_camera`, `pan_by`
  and `zoom_by`, then reports it to the binary with `ToBinary::Camera`.
- `main.rs`'s order thread is the one place that camera is written down, and it
  already shares it with the map thread and the fix feed.
- `backend/live.rs` already shows the house pattern for a network actor: own
  thread, own Tokio runtime, `std::sync::mpsc` for commands and events.
- `map_geometry.rs` already has the Mercator arithmetic, toolkit-free.

## Decision

**A toolkit-free control hub, an embedded server that never touches the UI, and
the desktop as its first host and its only writer.**

### The hub is the state, and one thread writes it

`src/control/hub.rs` holds the camera, a monotonic revision, and the recent
command ids. It is written by exactly one thread, and which thread depends on the
deployment:

- **Embedded (the console).** The order thread, in the `ToBinary::Camera` arm,
  because that thread already owns the camera tuple. A command arriving over HTTP
  is never applied to the hub directly; it is validated, handed to the shell, and
  the *resulting camera* comes back through the order thread. So the camera
  inside the hub is always a camera the desktop is actually rendering.
- **Headless (`tfg-server`).** The command thread, which drains the accepted
  commands and resolves each with `ControlHub::apply`. There is no shell to ask,
  so the reducer is the authority — and it imports `map_geometry::pan_center` and
  `view::{ZOOM_MIN, ZOOM_MAX}`, the same code the gesture path uses, so a
  server-side clamp cannot disagree with a renderer's.

That is the single-writer rule, enforced by the API: `report_camera` and `apply`
are the only mutators, each called from one place, and never both in one process.

A revision advances only when the camera actually differs, including its
viewport size. Most ticks report the camera they already had — the shell tells
the binary where it is on every tick — so equality is the coalescing rule, and a
drag that fires sixty ticks a second publishes only the distinct frames. The same
rule makes an optimistic client safe: a display that follows a command and then
reports the camera it followed publishes nothing.

### Commands are HTTP, events are a WebSocket, on one port

`POST /v1/commands` with JSON, and `GET /v1/ws` for the stream, both under the
same bearer token, both on the same listener.

HTTP for commands because a refusal must be a *reply*: a mobile client needs to
know whether its command was refused for a bad token, a bad shape, or a stale
protocol, and a WebSocket write has nowhere to put a 422. WebSocket for events
because a camera that moves twelve times while the operator scrolls should not
cost twelve polls.

The server runs on its own thread with its own runtime (`server.rs`), and the
only things it shares with the desktop are the hub (read) and an
`std::sync::mpsc` channel (write). **No network thread calls GPUI.**

### The protocol is geographic and semantic

A command is an intent on the shared state, never an input event and never a
pixel:

- `camera.set { center, zoom }` — absolute, idempotent. The desktop client
  sends this after its own gestures, in a later stage.
- `camera.nudge { dx_fraction, dy_fraction }` — relative, resolved against the
  viewport the desktop last reported. Positive `dx` moves the camera west,
  positive `dy` north: the same sense as dragging the map, so an operator who
  has used the console already knows which way is which.
- `camera.zoom { delta }` — center-anchored, clamped by the shell's own limits.
- `view.report { width, height }` — a display's own size. This is the one command
  that exists because of the headless host: a server has no frame to measure, so
  a rendering client tells it one. The console ignores it and keeps its own
  frame authoritative, which is why it is a command rather than a field on the
  camera.

`dx_fraction`/`dy_fraction` are fractions of the viewport, not pixels and not
degrees, because the desktop's viewport changes with the window and a phone
does not know how big that window is.

Zoom stays center-anchored for remote clients even though the desktop's wheel
is cursor-anchored. A remote client has no cursor to anchor to; inventing one
would be a worse lie than a different zoom rule.

### A device is a credential, not a client

The operator's token comes from the environment and belongs to no device. Every
other credential is a **paired device**: its own token, its own scopes, revocable
on its own.

- **Tokens are stored as hashes.** `$XDG_DATA_HOME/tfg/control-devices.json`
  holds a SHA-256 of each token, never the token, so a copy of the file is not a
  copy of the credentials. The file is `0600` and written beside itself then
  renamed, so a crash mid-write leaves the old table rather than half of a new
  one. A file that does not parse is a startup error for the control server and
  never a reason the console fails to open: starting empty would silently
  un-pair every device, and the operator would discover it one refused request
  at a time.
- **Scopes, not roles.** `observe`, `control_view`, `admin`, cumulative. A
  watching wall display cannot steer; a phone cannot pair another phone, because
  one stolen device must not become a foothold that outlives its own revocation.
  The operator token holds all three.
- **Pairing is a one-time code, because the LAN is not a trusted channel.** Eight
  Crockford-base32 characters — forty bits, no `I`/`L`/`O`/`U` — that live for
  120 seconds, pair exactly one device, and close after five wrong guesses. That
  short a code is only safe because of the other two rules; the three together
  are the design. Entropy comes from `/dev/urandom` with **no fallback**: a
  machine that cannot produce a code refuses to open a window and says why,
  rather than issuing a guessable one.
- **The offer is text, and the reference client draws the code.** The offer
  carries a `tfg-control://pair?...` URI and the same thing as JSON. The server
  emits no image; `tfgctl pair` renders the JSON payload as a terminal QR code
  when stdout is a terminal. The encoder is hand-written because no QR crate is
  vendored, and it earns that choice by being compared module-for-module with
  `qrencode` and decoded back with `zbarimg` (see Verified). A piped `pair`
  still prints only the code, URL and payload, so scripts are unaffected.
- **Revocation is deletion.** A revoked device is removed from the file, so a
  restart cannot resurrect it. That is the property an in-memory table would
  quietly lose.

### Refusals are part of the contract

- A malformed body is `400 invalid_json`.
- A command that parses but cannot be applied is `422 invalid_command` with the
  reason (a latitude of 91 is not a place).
- An envelope newer than the server is `409 unsupported_protocol` — refused
  rather than misread.
- A missing or wrong token is `401 unauthorized`.
- A real credential without the scope is `403 forbidden`, which is a different
  answer from `401`: one says "not this token's job", the other "not a token".
- A command id already seen is `200 duplicate` and does not reach the host
  twice.
- Pairing has four refusals of its own — `no_pairing_window`,
  `invalid_pairing_code`, `pairing_expired`, `too_many_attempts` — because a
  client needs to tell "wrong code, try again" from "the window is closed". A
  single `401` for all four would make a phone retype a code that cannot work.

Validation happens at the protocol boundary (`Command::validate`), so the
shell receives values it can trust.

### Attribution rides with the camera

The shell tags the camera message with the command that asked for the move, and
the hub publishes `camera.changed` with that `source` and `command_id`. A
watcher therefore sees its own command's effect named, and a gesture is named
`desktop`. Without this, two operators sharing a map cannot tell whose nudge
moved it.

## Consequences

- **The embedded server is not the final architecture.** The console still owns
  Minos, the session, the markers and the map, and hosts the server because that
  is where the camera is. `tfg-server` owns only view state. Extracting the rest
  is stage 8: the protocol and the command ownership are proven before the
  runtime is moved, and a protocol that has never carried a real command is not a
  protocol.
- **The desktop is not yet a network client.** It hosts, and `tfg-server` hosts;
  neither follows. A console in client mode would open a WebSocket, feed the
  shell through a channel, and post its own gestures as `camera.set`, with echo
  suppressed by the hub's equality coalescing — the seam exists for it, and the
  local gestures already produce exactly the value that command carries. Not
  built.
- **A headless host has state and no picture.** `tfg-server` is a headless owner
  of the camera, not a renderer: no tiles, no marks, no style. A client that
  reads `camera.changed` from it learns that a camera moved, not that anything
  was drawn. That is a smaller promise than a map server, and stating it is the
  point.
- **One host per devices file.** Two processes sharing the table each hold their
  own copy in memory and the last writer wins. `--ephemeral` exists partly for
  that, and the API doc says so.
- **mDNS and TLS are not built.** Pairing needs an address and the operator
  supplies one; `tfgctl` refuses `https://` rather than downgrading silently. On
  a network the operator does not trust, the honest answer today is: put it
  behind something that terminates TLS.
- **Longitude and latitude are the shell's arithmetic, not the server's.** The
  protocol refuses a place that is not a place (`camera.set` bounds); a relative
  nudge moves with the same Mercator inverse a drag uses, so a remote command and
  a gesture cannot drift apart. A nudge past the antimeridian leaves the longitude
  outside ±180, exactly as a drag does. The headless reducer shares those
  functions by import rather than by copy, which is the only way the two hosts
  can be relied on to agree.
- **A camera move is optimistic and unreconciled.** The host applies the command
  and publishes the result; there is no round trip that blocks the gesture. The
  revision in the event is the host's, which is the only revision that matters.
- **hyper and tungstenite, not axum.** Both are already in the dependency tree
  through the socket SDK; axum would have been a new crate for a wrapper around
  handlers this size. The handlers are the same shape.
- **One view, named `main`.** Views are named in the protocol rather than
  implied, so a second display is additive rather than a break.
- **The device table is JSON, not SQLite.** It is a handful of rows written on
  pairing and revocation; a store crate would be more moving parts than the
  problem has. The hash is what makes the format safe to copy, not the container.

## Verified

- The toolkit-free core is unit-tested: invalid latitudes and longitudes,
  non-finite numbers, zoom bounds, revision increments, resize-as-change,
  duplicate command ids, a bounded seen-table, and lag recovery.
- The server is tested end to end against a fake host (no GPUI): `hello` is
  public, `state` needs the token, a command reaches the host exactly once, a
  retry is a duplicate, malformed and impossible commands carry their codes, a
  camera change is pushed to a watching client, and a client can ask for a
  fresh snapshot.
- The shell's adapter is unit-tested: a remote nudge moves the camera west
  through the same path a gesture uses, the camera message it sends names the
  client and command id, a horizontal nudge does not move the latitude, and a
  nudge that arrives before the first picture still moves the camera.
- `tfgctl` is the reference client: `status`, `camera get`, the four directions,
  the two zoom steps, `camera set`, and `watch`.
- `scripts/control-smoke.sh` drives the real binary and asserts the acceptance
  criterion: with the console running, `tfgctl camera left` moved the centre
  from `106.91` to `106.8413` (west) and `camera zoom-in` took the zoom from
  `11.0` to `12.0`; a watching client received the snapshot, both
  acknowledgements and the attributed `camera.changed` events. Twelve checks,
  all passing, on 2026-10-27.

The first run of that smoke also found a defect the unit tests could not: a
nudge that arrived before the map's first picture was dropped, because the
gesture path refuses to pan a zero-sized frame. A fractional nudge now resolves
against the frame the shell has asked for, so a command sent while the map is
warming up moves the camera that comes up.

The pairing and headless stages were verified the same way: unit tests for the
store, the routes and the reducer, then the real binaries.

- **Devices** (`devices.rs`, 15 tests): a paired token authorises and carries its
  scopes; the operator token holds all three; a stored token is a 64-character
  hash and not the token; a wrong token authorises nobody; a code is spent by the
  device that used it; a code is readable and forgiving of case, spaces, dashes
  and `I`/`L`/`O`; five wrong guesses close the window even for the right code
  afterwards; an expired window refuses the right code; revocation is immediate
  and idempotent; rotation retires the old token and keeps the device; the table
  survives a restart **including the revocation**; a file that does not parse is
  an error and not an empty table; the offer carries the URI and payload a QR
  code would hold; a label is bounded and never empty; the operator token is
  tried before the device table.
- **Routes** (`server.rs`, 11 new tests): a paired token works and is not the
  operator's; a watching device reads the map and is refused with `403` when it
  steers, and the refused command never reaches the host; a paired device cannot
  open a pairing window; a wrong code is `401 invalid_pairing_code`; a window that
  was never opened is `404 no_pairing_window`; a device lists, revokes, and its
  token is `401` afterwards while revoking twice is `404`; a device rotates its
  own token and the old one dies; the operator token cannot be rotated over HTTP;
  an `admin` device is refused; `hello` reports `pairing_open`.
- **Reducer** (`hub.rs`, 9 tests): a headless nudge moves west by the same
  arithmetic as the shell's pan (asserted against `pan_center` called directly);
  zoom is centre-anchored; both zoom rails hold; `camera.set` keeps the viewport
  and clamps; a zoom already at the rail publishes nothing; a nudge with no
  viewport is a no-op; a viewport report is a change and a repeat is not; a
  nudge resolves against the reported viewport, so a wider one moves further; a
  command is attributed to the client that sent it.
- **`scripts/control-server-smoke.sh`** drives `tfg-server` and `tfgctl` with no
  display: the server starts, `camera left` moves the centre from `106.91` to
  `106.8413` and `zoom-in` from `11.0` to `12.0`, `camera.set` is absolute and
  the identical set again spends no revision, pairing mints a working device
  token, a spent code is `404`, a watching device is `403` at the command
  boundary, `view.report 1920x1200` doubles the same fraction's move, the device
  file is `0600` with no plaintext token, and after a **restart** the revoked
  device is still `401`. 28 checks, all passing, on 2026-10-27.

The four checks beyond the first eight are pairing against the **console**: a
code opened by the operator pairs a device, that device reads the console's
camera through its own token, revocation stops it, and the table lands at the
path `TFG_CONTROL_DEVICES` named, mode `0600`, with no plaintext token in it.
Pairing a phone is the whole point of the credential work, and until this ran,
only the headless host had been proven to do it.

- **Terminal QR** (`qr.rs`, 12 tests plus one ignored dump): every module of all
  eight mask candidates was compared with `qrencode -t ASCII -l M -m 0 -8 -v N`
  for every version 1–10 at exact level-M capacity, plus the offer and a forced
  version-1/7/10 set. qrencode's matrix equals one candidate exactly in every
  case, which pins the byte encoding, Reed–Solomon parity, interleaving,
  placement, format bits and version bits. Four matrices are embedded as
  goldens: two through the public `encode`, two through a fixed mask, because
  qrencode's mask heuristic and the ISO N1–N4 score legitimately disagree on
  some payloads. The chosen matrix for all ten versions, and the code as
  `tfgctl pair` actually printed it, were turned into PBMs and decoded by
  `zbarimg`, which returned the original bytes every time — including the
  213-byte version-10 payload.
- **The display rule** was driven, not assumed: a headless `tfg-server` plus a
  PTY ran `tfgctl pair`, printed 25 QR lines 49 characters wide (version 6, 41
  modules, four-module quiet zone) with 25 black-on-white SGR wrappers, paired a
  device over `POST /v1/pair`, and exited 0. The same command under a pipe
  printed only the code, URL and payload lines, and `--no-color` printed the
  glyphs without SGR.

That smoke found two things unit tests had not: `tfgctl camera get --json` emits
the camera object itself rather than a `{ id, camera }` wrapper (the route wraps,
the CLI does not, and the test asserted the route's shape), and the first
version of the script read the camera *after* `camera.set` while asserting on the
nudge. Both are the kind of mistake a script that never lies about its inputs
would not have made, and both were caught by the script failing rather than by
review.
