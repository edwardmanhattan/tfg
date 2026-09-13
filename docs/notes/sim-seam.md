# Synthetic-fix ingestion seam (for #12)

Primary sources: `src/geo/track.rs` (Registry/poll/blend/stale, read in full),
`src/geo/coordinates.rs` (`distance_m`, `lerp`), `src/backend.rs`
(`PollSource`, `stamp_now`), `examples/egui_window.rs` (poll thread owns
`Box<dyn PollSource>`, UI maps elapsed-since-poll onto `blend`).

Question: how does a sim tick feed synthetic fixes into the pipeline without
breaking what presentation relies on?

## Recommended shape: the sim IS a poll source

`SimSource: PollSource`, installed beside the wire source in a `MergeSource`
that concatenates both round vectors. Facts supporting this:

- The poll thread already owns `Box<dyn PollSource>` and swaps impls at
  startup (`egui_window.rs`, `TFG_BACKEND_URL` branch) — a merge source is one
  more impl, not new architecture.
- Everything downstream (`registry.poll` → `ships()` → `blend` → markers,
  trails, stale, inspector) consumes `Vec<Fix>` and never asks where a fix
  came from. Mixed fleets (wire traffic + owned sim ships) fall out for free.
- Mode toggle = arm/disarm: `MergeSource` is installed when sim work lands,
  with the sim disarmed (emits nothing) — zero behavior change until the
  first order. No startup-mode fork, no second thread, no registry locking.

Rejected: a separate fast sim thread writing to the registry directly (two
clocks, `Mutex` around the registry, bypasses the miss/stale accounting that
mode-exit gets for free — see below).

## Cadence: keep 2s rounds, sub-step inside the sim

- At 10 kn a ship moves ~10 m per 2 s round; `blend` already glides between
  fixes, so 2 s emission looks smooth. The sim may integrate kinematics at
  10 Hz internally (arrival detection, turn rates) but emits one fix/round.
- No pipeline change: the UI's elapsed/2 s fraction mapping is untouched.

## Synthetic fixes must be marked: add a source field

`Fix` today has no provenance. Options: (a) new `source: FixSource { Wire,
Sim }` field, (b) `ship_id` prefix convention. (b) pollutes identity and
leaks into every display site; (a) is a small, honest model change (struct
literals in tests + `WireFix` conversion update; `ShipView` exposes it for a
SIM badge). This is the one piece that touches the geo model — flag it for
the data-architecture grill (#14) and CONTEXT.md.

The field earns its keep twice:

1. **Jitter-guard bypass.** `blend` holds sub-8 m steps (`JITTER_GUARD_M`,
   `track.rs`) — tuned for GPS noise. A slow sim ship (< ~8 kn ⇒ < 8 m/round)
   would freeze mid-ocean. Sim fixes are noiseless by construction, so `blend`
   should skip the hold for `Sim` fixes. Without the field, the guard fights
   the sim and there's no clean way to tell them apart.
2. **Stamping ownership.** `stamp_now` (backend.rs) exists because replays
   loop canned `ts` backwards. The sim owns its clock and stamps strictly
   increasing millis-RFC3339 per emit; stamping stays a *source*
   responsibility, never registry logic. (Format must stay fixed-width:
   `poll` compares `ts` lexicographically.)

## What breaks, what comes free

- **Stale logic comes free, correctly.** Sim emits every round for owned ships
  ⇒ never stale. Disarm the sim (mode toggle off) ⇒ misses accumulate ⇒
  stale badges — mode exit reads as signal loss, which is the truth.
- **Trails/bounds unchanged.** 60-fix window is 2 min of sim history, same as
  wire. `displayed_position`/`should_track` are source-agnostic.
- **Missing kinematics helper.** `GeoPosition` has `distance_m` + `lerp` only
  — no dead-reckoning. The sim needs `dead_reckon(pos, heading, speed, dt)`
  (pure, new, unit-tested in `coordinates.rs`). Order application (turn toward
  waypoint, arrival) sits above it and belongs to #13/#14, not the seam.
- **Wire `ts` guard stays.** `fix.ts <= latest.ts ⇒ drop` is the loop-freeze
  fix's counterpart: with sim-owned monotonic clocks it never misfires; mixed
  rounds (wire + sim in one `poll`) are ordered per ship, never compared
  across ships.

## Bottom line for the grills

#13 (verbs) decides orders; #14 (data) owns `FixSource`, the `Order` model,
the CONTEXT.md collision, and whether the merge-source seam wants an ADR (it
is moderately hard to reverse — lean yes). The prototype (#16) then wires
`SimSource` + `MergeSource` + one orderable ship and proves the loop.
