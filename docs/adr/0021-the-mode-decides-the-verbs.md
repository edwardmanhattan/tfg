# ADR-0021: The mode decides the verbs

## Status

Accepted. Carries forward ADR-0008 (the backend is authoritative over the
phase) to the mode: the server's word is the mode, and the mode is what
decides which controls the console may draw.

## Context

The console read the exercise's mode exactly once, to print it in the
Essentials island. Every control was drawn for every exercise. That was not a
cosmetic gap — it was three wrong controls in two of the three modes.

The server's rules, read from `minos/internal/models/game.go` and confirmed
against the deployed dev API:

- `maneuver` — the server integrates orders over assumed time. An order posts
  at once and is clamped.
- `scenario` — the exercise is played through its STEPS. An order is HELD
  (HTTP 202, a staged order with no position) until a played step releases
  the hull. A step's gate waits for the readiness the related participants
  declare for that step. The clock runs inside a step: the exercise opens
  stopped, and a resume between beats is refused.
- `static` — positions are set rather than simulated. An order is refused
  (HTTP 409). A hull is moved by hand, with
  `PUT /games/{id}/units/{unit_id}/position` — a jump, with no heading and no
  speed — and the drag's frames go to
  `POST /games/{id}/units/{unit_id}/position/preview`, which publishes without
  recording.

The product decision (owner, 2026-10-20):

- **maneuver** — the commander gets the helm.
- **scenario** — **no seat gets a helm.** A scenario exercise is played
  through its steps; the commander's only verb on the current step is to
  declare readiness for that step.
- **static** — no helm either; a hull is set by hand on the map.

## Decision

**The mode is a type, and each mode has its own set of verbs.**

- `src/gamestate.rs` carries `GameMode { Maneuver, Static, Scenario, Unknown }`
  beside `GameState`. A mode the server did not say is `Unknown`, never a
  guess, and `Unknown` fails closed: no helm, no scenario surfaces.
- The console's rules are its own, and they are NOT the server's
  `AllowsSteering()`. The server can hold an order in a scenario exercise;
  the console does not offer a free-standing helm there, because a scenario
  exercise is played through its steps. The two rules answer different
  questions and each is named for what it is.
- A control that the server would refuse is not drawn. Not greyed out, not
  drawn: a compass that cannot be used is a lie about the state, and the
  console's own rule has always been that a disabled look that still posts is
  the one thing it must not ship.
- Where a mode has no control, the console draws the mode's sentence in the
  place the control was, naming the fact and the remedy.

## Consequences

- The server's `allows_steering` stays on `GameMode` because the client must
  not be the only place that knows it — but the helm predicate is a different
  one, and the two must not be confused.
- A scenario exercise's commander answers the step, not the helm. The step's
  readiness and the exercise's readiness are two different facts with two
  different gates (a step's gate, and the preparation→execution gate), and
  the console must not draw them with the same word.
- A static exercise is steerable only if the console offers the hand-set
  position gesture. Without it, static is a dead end: a refused helm and
  nothing else.
- Captures need one exercise per mode, or a mode seed. `TFG_MODE` forces the
  mode at boot, the way `TFG_PHASE` forces the phase.
- The mode is fixed at creation (the server has no route that changes it) and
  is re-read on every session read, so a joined exercise is mode-correct
  without the console ever having created it.

## Verified

Against the deployed dev API, 2026-10-20:

- The three modes are the legal three. `POST /games` with `mode: "dynamic"`
  answers 400 — "This field must be one of maneuver static scenario" — and
  the three legal values are accepted.
- A scenario order is **held**: `POST /games/{id}/units/{uid}/order` answers
  **202** with `staged: true` and the server's own `released_by` ("the next
  played step that releases this hull"), and no position. A maneuver order
  answers **201** with the fix. A static order is refused **409**: "This
  exercise's positions are set rather than simulated".
- A static position is set by hand: `PUT /games/{id}/units/{uid}/position`
  answers **201** with a jump record (`kind: "jump"`, heading 0, speed 0),
  and the same route in a scenario exercise answers **409**.
- A scenario exercise's clock runs inside a step: `POST /games/{id}/resume`
  with no step playing answers **409** — "This exercise's clock runs inside a
  step, and no step is playing".
- A step's gate waits for the related commander's readiness: the play is
  refused with "1 of 1 participant(s) related to this step have not declared
  themselves ready", and the declaration (`PUT …/steps/{tid}/readiness`)
  answers it.
- The whole lifecycle, in the advance's write set and order — the window,
  the seats, the force, the task organisation, the book, and the transition —
  lands on the server, and the exercise then reaches `execution`. Before the
  window rode the advance, a console-run exercise could reach `preparation`
  and never leave it: the terms are only writable while `planning`.

