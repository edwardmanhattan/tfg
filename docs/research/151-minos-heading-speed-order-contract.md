# Research note: MinOS heading-and-speed order / `GameFix` contract (tfg #151, part of map #147)

**Status:** complete; no product code changed. Research date: 2026-09-24.

**Primary sources:** the local MinOS checkout at
`/home/edward/Work/Crossnet/minos`, commit
[`49cd4c2c`](https://github.com/Voxtmault/minos/commit/49cd4c2c559674a3ccc5eefa93de3d75072774b2)
(2026-09-23), plus the TFG client on local `main` at `8a95138`. The current
MinOS source and `docs/openapi/minos-api.yaml` win over the older copied
contract under `minos-docs/`.

## 1. Exact write contract

The deployed path is **`POST /api/v1/games/{id}/units/{unit_id}/order`**.
The OpenAPI path calls the game parameter `id` (the issue's `game_id` is its
semantic name); `unit_id` is the hull/unit's own id, not a `game_units` row
id. Both are positive `int64` path values. The route is behind bearer
authentication only; there is no Casbin permission middleware on this write.
Sources: [route/OpenAPI](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/docs/openapi/minos-api.yaml#L4463-L4559),
[router](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/internal/routers/game.go#L302-L313),
and [controller](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/internal/controllers/game_controller.go#L122-L148).

Send `Content-Type: application/json` and an `Authorization: Bearer <access
token>` header. The JSON body has **exactly two required fields**:

```json
{
  "heading_deg": 45,
  "speed_kn": 20
}
```

- `heading_deg` is degrees clockwise from true north, with `0 <= heading_deg < 360`.
  Zero is valid due north; an absent heading is not silently north.
- `speed_kn` is the requested speed in knots, with `speed_kn >= 0`. Zero is an
  explicit stop/hold order, not a missing value.
- No latitude, longitude, waypoint, destination, or timestamp belongs in the
  request. The server derives position from the previous leg and derives
  `assumed_time` from the scenario clock.

The Go DTO makes both fields pointers with `required` validation, precisely so
zero remains distinct from absent: [DTO](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/internal/dto/game_order.go#L9-L28).
A malformed body is 422; a well-formed body with a failed field constraint is
400 (`data.errors` is keyed by the wire field name).

## 2. Successful response: `GameFixRecord`

A committed order returns HTTP **201** in the normal envelope
`{status_code, message, data}`. The `data` object is the server's
`GameFixRecord`:

| field | meaning | contract presence |
|---|---|---|
| `id_unit` | hull id | required |
| `assumed_time` | **scenario-clock** instant at which the new leg starts | required |
| `latitude`, `longitude` | where the hull **had reached** when the order arrived; these become the new leg's origin, not a destination | required |
| `heading_deg` | heading set for the new leg, from `assumed_time` until the next order | required |
| `speed_kn` | speed that will actually run, after any server clamp | required |
| `requested_speed_kn` | what the caller asked for, present only when reduced | optional/nullable; Go emits the key only when clamped |
| `clamped` | whether the requested speed was reduced | required; derived from the two speed facts |
| `created_at` | server write/audit time (real clock) | emitted by the Go DTO, but not in OpenAPI's required list |
| `created_by` | author user id; for this POST, the commanding user | emitted by the Go DTO, but not in OpenAPI's required list |

Sources: [API schema](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/docs/openapi/minos-api.yaml#L8551-L8604)
and [DTO/projection](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/internal/dto/game_order.go#L30-L73).
The stored model also has an internal row `id` and `id_game`; neither is part
of the `GameFixRecord` response. The route/path supplies the game identity;
the response supplies the unit and the leg's scenario identity.

Important distinctions:

- `assumed_time` is not `created_at` and not the HTTP publication time.
  `created_at` is audit metadata; `assumed_time` is the movement timeline.
- `GameFixRecord.clamped` means **speed reduction**. The separate
  `GameHullPosition.clamped` on `GET /games/{id}/positions` means the derived
  geometry was pinned at a pole. They are not the same flag and must not be
  conflated.
- A missing `requested_speed_kn` is the normal un-clamped encoding, not zero.
  A valid response should keep `clamped` and the optional requested value
  consistent; an inconsistent/malformed 2xx is not safe to treat as an order.
- The new heading/speed are a persistent setpoint: the leg remains in force
  until another order. There is no destination or route in this contract.
  A zero-speed order creates a stationary leg at the server-derived point.
  The order service has no separate cancel operation in this write path: a
  subsequent order, including a zero-speed order, is what replaces the leg.

## 3. Server speed clamp

The order service reads the hull's movement domain and the **current**
specification maxima on every order. `Surface` uses
`speed_max_surface_kn`; `Submerged` uses `speed_max_submerged_kn`.
The repository deliberately returns both candidate columns and the model
selects by domain: [repository](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/internal/repositories/game_movement_repo.go#L33-L64),
[domain/limit rules](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/internal/models/game_movement.go#L324-L394).

| requested speed | recorded `speed_kn` | `requested_speed_kn` | `clamped` |
|---|---:|---:|---:|
| `<= limit` (including exactly the limit) | request | absent | `false` |
| `> limit` | `limit` | request | `true` |

This is downward-only and preserves the heading when the speed is reduced.
A missing, zero, or non-positive applicable maximum is **not** an unbounded
order: `Air`, `Land`, an unknown domain, a missing current specification, and
a missing domain-specific maximum all fail closed with 409 and write no leg.
The current fleet's submarines have surfaced maxima but no submerged maximum,
so a submerged order is refused until that data is published. Sources:
[clamp implementation](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/internal/models/game_movement.go#L396-L421),
[service order path](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/internal/services/game_order_service.go#L96-L199),
and [model tests](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/internal/models/game_movement_test.go#L365-L523).

The client must not substitute a local class cap, the local simulation's cap,
or a slider range for this server limit. The response's `speed_kn` is the
accepted/applied value; `requested_speed_kn` is the operator-facing ask when it
was reduced.

## 4. Authority and refusal behavior

The service checks authority **before** game state. A caller is authorized only
when the authenticated user id equals `game_units.id_commander` for the piece
in this game. This is an in-game instance comparison, not a live-feed
`unit_commanders` assignment and not a general `order` permission. There is no
node-commander or Game Master override; a node commander's direction is
advisory unless that person is also this hull's commander. Source:
[order service authority](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/internal/services/game_order_service.go#L330-L423).

The service's check order and resulting outcomes are:

0. missing/invalid authentication -> **401**; malformed JSON/type -> **422**;
   a valid-shaped request with invalid fields or path ids -> **400** (all before
   the service's game/authority checks);
1. game missing -> **404**;
2. piece missing from this game -> **404**;
3. caller is not the piece commander -> **403**, with `data: null`, before the
   game phase is examined (a non-commander does not learn whether the game is
   running from a 409);
4. game is not `execution` (planning, preparation, or closure) -> **409**;
5. `accepting_actions` is false (pause or communications blackout) -> **409**;
   the order is not queued;
6. no applicable speed bound -> **409**, with no leg;
7. an existing leg for the same `(game, unit, assumed_time)` -> **409**; the
   service tells the caller to wait for scenario time to move;
8. an unanchored clock or missing origin is also reported as **409** as a
   repairable exercise invariant failure;
9. repository/transaction/internal failures -> **500**.

The public contract lists 400, 401, 403, 404, 409, and 500; the generic error
shape is `{status_code, message, data: {errors: ...}}`, with `_request` used
for a non-field reason. A 403 can also be an account/authentication gate
failure, so the client can safely treat it as “not accepted/no authority,” but
should not infer the more specific commander diagnosis from a null body.

`accepting_actions` and `running` are separate facts. A game may keep advancing
scenario time while orders are closed (blackout); the order service checks
`accepting_actions`, not merely the clock rate. The migration explicitly
forbids queuing refused orders: [clock/fix migration](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/pkg/migration/migrations/000030_game_execution_clock_and_fixes.up.sql#L20-L39).

The order is committed in a transaction before the optional realtime publish.
A missing/failed `game.order_issued` publication does not undo a committed
order, so the 201/reconciliation answer remains the authority. The event is
best effort and is published after commit: [publish path](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/internal/services/game_order_service.go#L247-L293).

## 5. What the Rust client may currently treat as authoritative

### Already aligned

- `MinosMaster::order_unit` posts the correct path/body and unwraps the MinOS
  envelope: [client seam](../../src/backend/master.rs#L804-L833). It already
  sends no position or timestamp.
- `apply_fix_batch` logs the returned fix and then refreshes
  `GET /games/{id}/positions`; it does not install a second local sim order
  for a connected piece: [order application](../../src/main.rs#L4137-L4242).
- The plot path maps MinOS positions to `FixSource::Game`, keeps the
  per-position scenario stamp, and bypasses the wire-only jitter guard through
  the existing registry: [plot mapping](../../src/main.rs#L4244-L4331),
  [registry blend](../../src/geo/track.rs#L345-L384).
- Execution releases identifiable local game pieces before the MinOS plot is
  pulled, so the connected path does not deliberately run two movement
  authorities: [release path](../../src/main.rs#L4333-L4350).

### Seams that constrain later implementation

1. **Strict decoding is not yet present.** `order_unit` defaults a missing
   `id_unit` to the path id, missing coordinates to `(0,0)`, missing
   heading/speed to the request, and missing `clamped` to `false`. `unwrap_envelope`
   accepts any 2xx. Those fallbacks are convenient projections, but they are
   not safe authoritative values if a successful response is malformed; a
   client must not turn a defaulted field into a committed fix.
   The Rust `GameFix` also drops the Go response's `created_at` and
   `created_by`: [projection/type](../../src/backend/master.rs#L1776-L1791).
2. **Position timestamps are currently conflated.** In
   `GET /games/{id}/positions`, the top-level `assumed_time` is the instant
   the plot was computed; each `positions[].assumed_time` is the stamp of the
   leg in force, not the query instant. The API documents this explicitly:
   [position schema](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/docs/openapi/minos-api.yaml#L8606-L8634).
   `apply_plot` currently puts the per-position leg stamp into `Fix.ts` and
   ignores the list-level instant: [plot mapping](../../src/main.rs#L4275-L4315).
   Because `Registry` drops equal/older timestamps, repeated snapshots of one
   leg are not distinct animation samples under the current mapping. This is
   a factual seam to carry into the animation decision, not a change made by
   this research ticket.
3. **The plot projection drops position `clamped`.** `GameHullPos` has no
   field for it, while the API always supplies it. It must not be confused
   with order-response speed clamping: [position parser/type](../../src/backend/master.rs#L836-L879).
4. **The game event is currently ignored.** MinOS publishes
   `game.order_issued` with the same `GameFixRecord` as the 201, but the TFG
   `LiveWire` parser accepts only message event types and logs all other game
   publications as ignored: [event contract](https://github.com/Voxtmault/minos/blob/49cd4c2c559674a3ccc5eefa93de3d75072774b2/docs/realtime-protocol.md#L312-L328),
   [TFG parser](../../src/backend/feed.rs#L101-L136), [subscription](../../src/backend/live.rs#L131-L155).
5. **The local spec projection is not the MinOS clamp.** `parse_hull_spec`
   exposes only `speed_max_surface_kn` as `HullSpec.speed_kn`; it has no
   movement-domain or submerged-max field. The local `SimSource` also clamps
   waypoint orders against its class catalog. Neither is authoritative for a
   MinOS `HelmOrder`: [spec parser](../../src/backend/master.rs#L1984-L2017),
   [sim clamp](../../src/sim.rs#L226-L257).
6. **The existing UI still derives a heading from a waypoint before posting**
   (`minos_leg`), while the backend itself accepts heading directly. That is
   current legacy UI shape, not a statement that MinOS needs a waypoint: [UI
   seam](../../src/main.rs#L4152-L4175).
7. **Typed refusal information is lost in the order worker.** The worker maps
   `BackendError` to `String` before returning `OrderOut`; later result-state
   work cannot distinguish 403/409 from transport failures without restoring
   the typed seam. The typed transport/error mapping itself is already
   present: [order batch](../../src/main.rs#L4197-L4208),
   [error mapping](../../src/backend/error.rs#L56-L96).

## Bottom line for the map's later tickets

- The backend write is heading + speed only. A 201 `GameFixRecord` is the
  committed, server-derived reconciliation point; its applied speed and
  coordinates are authoritative, while `requested_speed_kn` preserves the
  pre-clamp ask.
- A received non-2xx answer is not an accepted order and is never queued by
  MinOS. A transport failure after the server committed can still leave the
  client's outcome unknown; do not promote a local draft, local role list,
  local class cap, or local position to accepted state on the strength of a
  refusal or an ambiguous transport result.
- Keep the two `assumed_time` meanings distinct: an order/leg response carries
  the leg stamp, while a positions response also carries the top-level plot
  instant. Coordinate animation must consume accepted authoritative samples,
  not client-predicted destinations.
- The current client already has the right write seam, but its permissive
  response defaults, leg-stamp mapping, dropped position clamp, ignored order
  event, and stringified worker errors are the concrete seams that later
  implementation/regression tickets must account for. This note makes no UI or
  lifecycle decision.
