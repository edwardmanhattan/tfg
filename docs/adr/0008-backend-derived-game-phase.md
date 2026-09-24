# Backend-derived game phase

The held Minos game's local stage (Planning / Ready / Live / Eval)
derives from a `GET /games/{id}` detail read, never from a local flag
alone. Every select, refresh, and transition funnels through one
resync that projects the authoritative state onto the local machine;
Minos is forward-only, so there is deliberately no projection back.

## Considered Options

Keep the local `sim_ready` flag as the stage driver, syncing it
opportunistically after transitions (rejected: any transition another
client makes — or any game selected mid-exercise — leaves the client
claiming Planning while Minos runs Execution, and the local Back /
End Session / Replan verbs then pretend the backend moved with it).
Drive the engine start/stop from socket events instead of the detail
read (rejected: game lifecycle events are not currently published —
map #80 notes — so polling the authoritative read on operator
actions is the only honest signal today).

## Consequences

Selecting a preparation game shows Ready; selecting an execution
game enters Live (or says loudly why the engine refused); selecting
a closed game shows Eval. "Akhiri sesi" transitions the backend to
closure first and only ends locally on success (or when the resync
proves another client closed it). "Rencanakan ulang" releases the
closed hold and returns to the picker — a replan is a new exercise,
not a rewound backend. The `← Kembali` verb is gone, replaced by an
explicit resync. Steady-state refreshes are guarded no-ops: only a
mismatch moves the machine.
