# Simulation is a fix source, not a parallel pipeline

The sim advances owned ships from orders and emits synthetic fixes through a
`SimSource: PollSource`, merged with the wire source. The registry never
learns about orders — it ingests `Vec<Fix>` exactly as before.

## Considered Options

A dedicated sim thread writing to the registry directly (rejected: two clocks,
locking, and it bypasses the miss/stale accounting that mode-exit gets for
free), and unmarked synthetic fixes (rejected: the jitter guard would freeze
slow sim ships with no clean way to tell them apart — hence the `FixSource`
field on `Fix`).

## Consequences

`Fix` carries provenance; `blend` skips the jitter hold for sim fixes; the
sim owns its monotonic clock. Mixed fleets (wire traffic + owned ships) work
with no downstream changes.
