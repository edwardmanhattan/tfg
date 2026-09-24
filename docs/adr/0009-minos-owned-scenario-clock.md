# Scenario clock is Minos-owned; writes are reads

Pause, resume, and the time factor go through Minos (`POST
/games/{id}/pause`, `POST .../resume`, `PATCH .../time-factor`), and
their answers are the only clock reads the client gets — no clock GET
exists. The live engine ratio seeds from the detail's `time_factor`
for connected executions, never from the setup windows; Space pauses
through Minos when a game executes. The local hold follows the
scenario hold so the two displays never disagree.

## Considered Options

Keep the window-derived ratio and local Space pause for connected
games, syncing occasionally (rejected: the display would drift from
the assumed times the server stamps on fixes, and a local pause would
claim a hold Minos never took — orders would still be rejected
server-side while the bar said paused). Poll a clock GET on a timer
(rejected: the endpoint does not exist; the write answers carry the
full GameClock including assumed_now and segments, which is
sufficient for operator actions). Pre-refuse orders locally while the
clock reads held (rejected: the read goes stale between pulls, and a
stale local refusal is worse than the server's loud 409 — the order
path never queues, so paused rejection is already honest).

## Consequences

`time_factor`, `running`, and `accepting_actions` display as three
separate truths (a blackout runs with orders closed). The factor box
caps at 144x for sanity; the backend has no ceiling. Closure does not
freeze the clock backend-side (B4) — out of scope here, recorded on
the map. Continuous assumed-now display wants a poll cadence or a
clock event channel; both are later slices.
