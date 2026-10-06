# The force is composed locally and written through at a stage advance

Placing a hull, moving its starting position, and naming who commands it are
draft edits on the client. None of them is a backend write. A stage advance
(planning → preparation, preparation → execution) syncs the whole force to
MinOS in one ordered diff and only then moves the game.

## Context

MinOS cannot hold a piece without a commander: `game_units.id_commander` is
NOT NULL, and `POST /games/{id}/units` says so in as many words. The console
used to write each decision the moment it was made, which meant the placement
gesture could not happen until the operator had answered a commander combo box
— a player decision front-loaded onto a map decision. It also meant three
round trips for one hull, and a refusal on the second leaving the first
committed.

## Rules

- The map is the working surface. A drop, a drag of a placed hull, and a
  commander change are draft edits with no network call.
- `force::plan` is the whole of the server contract: a pure diff from the
  draft against the live mirror, returning writes in the order the server
  accepts them — assignment before placement (the position endpoint 404s on a
  hull the server has never heard of), removals last.
- A hull in the draft with no commander is a legal draft state and an illegal
  MinOS state. The sync refuses and names the hulls; it never seats whoever
  happened to be free. The operator gets a bulk "fill commanders" verb instead.
- The draft is seeded from the server once per held game, and only then. A
  later bundle re-read refreshes the live mirror the diff is computed against
  and must never clobber the operator's unsaved edits.
- A stage advance flushes first. A refused flush refuses the advance, and the
  goal is forgotten rather than retried implicitly; the operator syncs
  explicitly or advances again.
- Seats are not staged. A commander must be a participant of the game, so
  seating stays an immediate write and the assignment refers to a real
  participant by the time the flush runs.
- The setup sim is frozen, so a placement move must move the displayed
  position itself (`Registry::set_position`) and drop the hull's history.
  Leaving it to the next fix leaves the hull drawn where it used to be.

## Consequences

The Readiness gate reads the server, so it cannot see the draft. That is why
the flush sits in front of the advance rather than behind it, and why the Fleet
island reports "N change(s) not yet on Minos" while any are owed.
