# HelmOrder is the persistent per-unit helm directive

The current movement contract uses a persistent `HelmOrder` for one
`GameUnit`: compass heading in degrees and speed in knots. It has no
waypoint and does not describe a route. An accepted order replaces the
previous setpoint; speed `0` is the explicit `Hold position` action and
retains the last accepted heading.

## Considered Options

Keeping waypoint `Order` as the primary directive was rejected because it
makes the commander choose a destination when the backend contract is
already heading plus speed. A shared multi-unit order was rejected for
this slice because each ship needs an unambiguous authoritative setpoint.
A client-generated destination or movement path was rejected because the
client presents `Fix`es and must not invent game state.

## Authority and lifecycle

- `SetHelm` is the domain grant verb; the transport may continue to call
  the MinOS operation an order.
- One `HelmOrder` belongs to one `GameUnit`. A future multi-unit `Command`
  may fan out into multiple `HelmOrder`s.
- Only one submission is in flight. A newer intent replaces the local
  pending draft; pending commands are never queued.
- A disconnect while a submission is in flight makes its outcome
  **unknown**, not accepted or refused. It is not automatically retried.
- An accepted MinOS order is authoritative across client restart and
  reconnect. A local draft is only an operator intention.
- `Cancel` clears an unapplied local draft only. Once an order is accepted,
  stopping is the explicit `Hold position` action.
- Higher jurisdiction replaces on acceptance; a lower-authority order is
  refused and never queued.
- The existing waypoint `Order` remains only for legacy replay data and
  possible future navigation work; it is not exposed by the new helm
  surface.

## Consequences

The client can render accepted `Fix` movement without predicting a
position, while the local simulation can expose the same per-unit helm
contract for sandbox development. Command result states, interpolation
details, and the final interface remain separate decisions.
