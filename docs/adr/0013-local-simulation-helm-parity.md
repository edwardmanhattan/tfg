# Local simulation mirrors HelmOrder without impersonating MinOS

The local simulation accepts the same per-`GameUnit` `HelmOrder` shape as
the connected client, but it remains a sandbox authority. It advances a
ship by heading and speed over game time, emits `Sim` fixes, and never
claims that MinOS accepted or committed the command.

## Rules

- Apply a heading setpoint at the next simulation tick; do not invent a
  turn-rate model.
- Clamp to the local class maximum and report `Clamped`. If no usable local
  maximum exists, refuse the order.
- Keep land collision as an explicit sandbox-only rule: stop at the first
  land intersection and retain the blocked `HelmOrder`.
- Keep legacy waypoint `Order` and multi-unit waypoint behavior for old
  scenarios and replay, but hide them from the new helm surface. New helm
  commands never derive heading from a waypoint.
- Use the same `Grant`/`Authority` model with a `SetHelm` verb. Lower
  authority is refused, never queued.
- Return a local result type with the same operator-facing result states,
  explicitly labeled as local sandbox; do not fabricate MinOS audit fields
  or a server commit.
- Integrate heading and speed over game time on each tick and emit a `Sim`
  `Fix`. A `HelmOrder` has no waypoint arrival or ETA state.
- Keep multi-unit helm fan-out out of this slice. Legacy multi-unit
  waypoint commands remain available only for compatibility.

The local cap, land result, and sandbox result source must remain visibly
distinct from MinOS's authoritative response.
