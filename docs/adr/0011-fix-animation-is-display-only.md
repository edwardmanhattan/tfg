# Fix animation is display-only and bounded

The client may interpolate a marker between two accepted `Fix` coordinates,
but the interpolation is never a new game fact. The Trail contains only
accepted Fix positions.

## Rules

- Use valid Fix timestamps, bounded by the normal poll cadence; use game time
  for pause and time scaling.
- Do not animate position while game time is paused.
- Backfilled or historical Fixes update the Track/replay but never replay as
  live movement.
- A stale ship holds its last authoritative position; the client does not
  dead-reckon indefinitely.
- Large, time-invalid, or reconnect gaps snap to the newer authoritative Fix
  and make the discontinuity visible through the existing stale/data-age
  treatment.
- A Fix with unchanged coordinates may update heading, but it does not add a
  duplicate visible Trail dot.
- A reconnect gap is not animated because the intervening movement was not
  observed.

## Consequences

The map can look like a ship is moving between normal authoritative samples
without turning display interpolation into simulation. Historical replay and
live presentation can share the Track while remaining visibly distinct.
