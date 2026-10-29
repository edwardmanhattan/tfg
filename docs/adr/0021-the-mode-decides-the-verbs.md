# ADR-0021: The mode decides the verbs

The console's mode was a string in one field, compared at the places that
needed it, and the mode it read was the SERVER's mode: the mode that says
whether the wire will take an order. That was the wrong question, and the
console was answering it in the wrong places.

## Status

Accepted. Amended 2026-10-29: a scenario exercise's helm is drawn
(the step's READY is the commit), and the step reminder strip was replaced by
the scenario card in the map's body. Carries forward ADR-0008 (the backend is authoritative over the
phase) to the mode: the server's word is the mode, and the mode is what
decides which controls the console may draw.

## Context

Minos has three modes, and they are the server's own (`models.GameMode`):

- `maneuver` — orders apply at once, the server clamps and answers.
- `scenario` — the server ACCEPTS an order and HOLDS it (HTTP 202) until a
  played step releases the hull.
- `static` — the server refuses every order with a 409; the positions are
  put there by hand.

The server's own predicate is `GameMode.AllowsSteering`, and it answers
"may an ORDER move a hull". It says yes for `maneuver` and `scenario`,
because in a scenario exercise the order is held rather than refused.

The console drew a helm — a compass, a speed scale and a SET HELM button —
wherever it had a commanded hull in an executing exercise, in every mode.
So in a scenario exercise it offered a free-standing instrument whose every
answer was "held", and in a static exercise it offered an instrument whose
every answer was a 409. The operator had to learn both facts from the
server, at the moment of the press.

The console's rules are not the server's rules. What the console offers is
a different question: what does this station DO in this exercise?

- **maneuver** — the commander gets the helm. An order posts at once.
- **scenario** — the commander gets the helm, and the STEP's READY is the
  commit: the instruments set the course and speed, the server holds the
  order while no step relating the hull is playing, and the step that plays
  releases it. The Game Master plays, skips, ends and replays steps. The
  clock runs inside a step: a scenario exercise opens stopped and the server
  refuses a resume between the beats.
- **static** — no helm either. The positions are set BY HAND: the commander
  drags a hull on the map in execution and the hull jumps there. The
  commit route (`PUT /games/{id}/units/{unit_id}/position`) takes a
  latitude and a longitude and nothing else; the preview route
  (`POST .../position/preview`) publishes the frames of the drag to the
  other participants' boards and records nothing.

An exercise whose mode the server did not say is `Unknown`, and it fails
closed: no helm, no scenario surfaces, no clock verb.

## Decision

**The mode is read once, where the session is read, into a `GameMode`, and
every verb is a predicate on that one value. The console's predicates live
in `src/mode_rules.rs` and are NOT the server's `allows_steering`.**

- The mode reaches the shell as `SessionView.facts.mode: GameMode`, parsed
  in `main.rs::fetch_session` — the one place the session is read. A rule
  that compared the mode's spelling at each site would be a rule that could
  be spelled two ways.
- The shell holds one predicate per rule (`helm_offered`,
  `scenario_exercise`, `step_declare_offered`, `step_verbs_offered`,
  `clock_may_resume`), each with a doc comment naming the rule it enforces.
  The pure rules live in `mode_rules` with their own unit tests; the shell
  is a thin caller, so a test exercises exactly what the click does.
- Where there is no helm, the instruments are NOT drawn greyed out — they
  are not drawn at all, and the mode's sentence stands where they were. A
  disabled-looking control reads as a control that failed; an empty channel
  reads as nothing at all.
- The writes refuse as well as the drawings refuse. `send_helm` refuses
  outside a maneuver or scenario exercise, `send_step_declare` refuses
  outside a scenario exercise, and `send_step_verb` refuses outside one.
- The scenario surfaces — the board's step verbs, the scenario widget, the
  step's declaration — belong to a scenario exercise alone.
- The clock's verb is ONE predicate, read by the button's label and by the
  write it sends. In a scenario exercise it is a pause and only a pause.
- The game-level readiness ("ready to start") and the step-level readiness
  are two different facts. The step's verb is named for the step
  ("READY" / "WITHDRAW") and lives on the scenario card in the map's body,
  so the commander does not have to open the board to answer the question
  the card is asking. The old strip under the band is gone.

## Consequences

- The console's copy of the modes (`SESSION_MODES`) had to change: it
  described `scenario` as "personnel steer like maneuver", which was the
  server's rule and is now the console's opposite.
- The helm is drawn in ONE place in execution (the commander's rail) and
  one place outside it (the Command island). Both copies obey the same
  rule; a copy that answers a different rule is a copy that lies.
- A capture could not photograph a mode without three live exercises, so
  there is a `TFG_MODE` seed, read at boot, that forces the session's mode
  the way `TFG_PHASE` forces the phase. A seed naming a mode this build
  does not know is no seed at all.
- The console's rules and the server's agree about the helm now: both
  modes where `allows_steering` is true also draw the instruments. The
  difference that remains is about the COMMIT — a scenario's READY posts
  the held order and the declaration together, where a maneuver's SET HELM
  posts the order alone. The comment on each predicate says so. A reader who
  finds the disagreement has to read the two comments, which is the point:
  the disagreement is the decision, not an oversight.
- The static exercise's drag is a NEW gesture: the rail's helm channel
  carries the commander's hulls as things to pick up, the frames go out on
  the preview route while the drag is in the air, and the commit is the
  release — a commit per pointer movement would put a row in the chain for
  every frame, which is exactly why the preview route exists.
  - The gesture is offered for the caller's OWN hulls only, and the server
    would also let the Game Master correct any hull. That is a deliberate
    narrowing: the console offers what a commander may do, and the Game
    Master's corrections are the Game Master's own surface.

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
