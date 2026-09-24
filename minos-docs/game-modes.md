# Game modes — design record

Written 2026-09-15, while planning slice 2. This is the reasoning behind the contract, not the
contract. Where this file and `openapi/minos-api.yaml` disagree, the spec wins and this file is
wrong.

Status: **Maneuver is the only game mode in this release. Static is deferred.**

> **STATUS 2026-09-22 — the LIFECYCLE and SETUP are built; EXECUTION is PARTLY built; CLOSURE is not.**
>
> Everything that FRAMES an exercise exists: the four states and the forward-only machine, the room
> key and join, the readiness gate, the roster, the game pieces and who steers them, the task
> organisation with icons, manual placement, and the two-path visibility rule.
>
> **Execution is built IN PART, and this block claimed all of it until 2026-09-22.** The MOVEMENT
> MODEL runs: `game_unit_placements` holds where a hull starts and `game_fixes` is the append-only
> chain of what it has done since, with an origin leg materialised for every placed hull at the
> transition into `execution`. `time_factor_change` is the rate log.
> `POST /games/{id}/units/{unit_id}/order` closes a leg and opens the next along the rhumb line, the
> assumed clock can be paused, resumed and re-rated, and distance and bearing are computed per request
> by `GET /games/{id}/positions?from_unit=<id>`. All of it has been exercised end to end over HTTP by
> `scripts/rehearse_game.py`.
>
> **CONCEPT §3 DEFINES EXECUTION AS EIGHT THINGS AND FOUR OF THEM DO NOT EXIST.** Items 3, 6, 7 and 8
> are *messages* (the GM and the Judges send Telegram and administrative messages to units and
> objects), *judgements* (a judgement against every action a personnel took — turning, adjusting
> speed, the route held), *simulated events* (natural or synthetic disasters, orders from higher up)
> and *warnings* (for a breach — too far from the area, or not responding). No table, no route and no
> contract path exists for any of them.
>
> **Closure does not exist either**, and it is a different and smaller thing: the analysis phase, where
> the GM and the Judges replay the exercise with a playback slider and add their reviews. Concept §3's
> own list is what makes the distinction — messages, judgements and warnings are Execution, and only
> the replay and the reviews are Closure.
>
> **The permission vocabulary is still ahead of the code, and that is now worth a decision rather
> than a note.** `order`, `pause` and `change_factor` have routes, but those routes are
> **authentication-only with the authority check in the service** — a game role is a row, and a
> Casbin object names resource families with no instance id, so no policy could express "the
> commander of THIS hull". `judge`, `send_message` and `warn` have no route at all. The objects
> `/games/messages`, `/games/judgements`, `/games/timeline` and `/games/review` still have nothing
> behind them, and `/games/control` has routes that no policy asks for. So the matrix governs the
> setup and lifecycle objects — `participants`, `units`, `hierarchy` and, in the service rather than
> a route, `transitions` — and the rest of it is vocabulary staged for the parts of EXECUTION that are
> not built (messaging, judgements and warnings) and for Closure's timeline and review reads. **That
> staged vocabulary is no longer grantable** (2026-09-22): the write refuses any (object, action)
> pair that nothing in this build asks for, so an operator can no longer be handed a checkbox that
> governs nothing. The split itself is a scope decision, recorded in `docs/backlog.md` B19.
>
> **What is NOT a gap, deliberately:** Static (deferred, §7 H8), room-key rotation (H11), and the
> `game:<id>` realtime events, which are specified in `docs/realtime-protocol.md` §4a and emit
> nothing — only `live-feed` currently publishes.

> **CHANGED 2026-09-15 — Operational is no longer a game mode.**
>
> It described real vessels reporting their own positions, and that is a **live feed**: no game, no
> participants, no state machine, no orders, no assumed clock. Shipping it as a value in
> `games.mode` would have meant a game whose "game" does nothing, so it moved OUT rather than being
> deferred — see migration `000014_narrow_game_mode`.
>
> What that means for the rest of this document, which is otherwise kept as written:
>
> * **§1's axis still holds** — where positions come from. It is now the reason for the *split*
>   rather than a difference between two modes. Operational was never a variant of Maneuver; it was
>   the same question answered differently, which is exactly why it became its own resource.
> * **§4 describes the live feed.** Read it as that. The reasoning survived the move intact because
>   it is about positions that come from a device, not about games.
> * **§4.4 (manual placement) and §4.6 (the teleport) are GONE.** Both existed to stop a GM authoring
>   a position inside a live device track. Nobody authors a real vessel's position, so the conflict
>   they resolved cannot arise.
> * **§6.2's Operational carve-out is obsolete** — there is no mode left to make an exception for.
> * **H10 is moot and H12 is answered** — see §7.

---

## 1. The axis that actually varies

The modes differ in **where positions come from**, and almost every "rule difference" is a
consequence of that rather than an independent decision:

| Mode | Position source | Status |
|------|-----------------|--------|
| Static | a person drags it on a board | deferred |
| Maneuver | the server integrates orders over assumed time | in scope |
| ~~Operational~~ | *devices report them* | **moved out — now the live feed, see §4** |

Framing it this way matters because it stops the rules from being enumerated per mode, where they
would drift apart. The questions "may this be clamped?", "does it continue when silent?" and "who
may author it?" all have the same answer within a mode, and the answer follows from the source.

---

## 2. The shared spine

Identical in both modes, and worth stating so it does not get re-litigated:

- A game has participants, a **game-scoped hierarchy** (F1: the task organisation only — the
  peacetime administrative tree is out of scope), and units assigned to it.
- **Planning → Preparation → Execution → Closure.** What Preparation *means* differs slightly
  (see below); the machine does not.
- Positions are **derived for display** from an append-only event log. The log is the record;
  where a ship is drawn is a rendering of it.
- Authorization is game-scoped through the existing Casbin domain (`GameDomain(gameID)`), with
  per-object `own` rules plus a service-level instance check where a policy cannot reach.
- Warnings are a lookup table the client populates (B7). Judgements never reach unit or node
  Commandos (B8).

---

## 3. Maneuver

Steering is **heading in degrees and speed in knots** (concept `[1.x]`). There is no coordinate
channel, and the concept never mentions one.

**ALL OF THE TABLE BELOW IS IMPLEMENTED** (as of 2026-09-22) — see the status block at the top. The
rules were decided and recorded here before the code existed, which is why this table reads as a
specification; it is now a description of running behaviour too. The order endpoint is
`POST /games/{id}/units/{unit_id}/order`, the movement model is `internal/models/game_movement.go`,
and the chain is `game_fixes` — an append-only log of LEGS rather than an `game_orders` table, as an
earlier version of this section guessed. The order is the act; the leg it opens is the record, and
storing the leg is what makes a position a pure function of the chain.

| Rule | Behaviour | Source |
|------|-----------|--------|
| Who steers | only the unit's own Commando; a node commander is advisory | B1 |
| Speed limit | server clamps to the hull's specification maximum | B3 |
| Turning | **instant, in this release** — see the decision below | B4, revised 2026-09-21 |
| Silence | the unit continues on its **last order** until a new one arrives | B6 |

Two consequences that the code must not soften:

- **Clamping is mandatory here, and forbidden in Operational.** A 100-knot order is a bug (B3);
  a 35-knot *measurement* is a fact. Same number, opposite handling, decided by whether the value
  came from a person commanding or a device reporting.
- **A hull with no published specification cannot move.** Not zero, not a default — the movement
  model refuses and says why. This is why `current_specification` is absent rather than zeroed,
  and why `null` means unknown throughout the specification payload.

#### DECIDED 2026-09-21: turns are INSTANT, and the reason is the deadline

This document said "turn at max rate toward the ordered heading" while `README.md` §5 said turns are
`instant` — a genuine contradiction, and the two describe different systems. **Instant turns win.**

A leg is therefore a straight rhumb line with a **corner** at each fix, so a fix needs no turn-in arc
and there is no `turn_rate_max_deg_s` term in the position formula at all. One row per order, one
straight run.

**The trade is fidelity, and it is reversible WITHOUT a schema change.** A real vessel leaving 090 for
180 traces an arc, and the map will draw a corner instead. If max-rate turns are wanted later the fix
chain does not change — the turn simply gains derived intermediate points, and only the position
function grows a term. That is what makes taking the cheap option now safe: it defers *geometry*, not
the data model.

`unit_spec_versions.turn_rate_max_deg_s` stays where it is and stays populated. This release does not
use it; nothing deletes it, and `turn_rate_is_derived` already records whether the value was derived.

---

## 4. Operational — now the live feed, not a mode

> **This section describes a feature OUTSIDE the game.** It was written as a game mode, and the
> reasoning below survived the move unchanged because it is all about *positions that come from a
> device* rather than about games. Read "in Operational mode" as "in the live feed" throughout.
>
> Two subsections did NOT survive and are marked in place: **§4.4** and **§4.6**, both of which
> existed to keep a GM from authoring a position inside a live device track.

Real vessels — currently **military assets**, so the existing taxonomy describes them and no
civilian branch is needed. A mobile app reports the vessel's position.

### 4.1 Positions are measurements

They are **never clamped and never extrapolated**.

- Clamping a measurement is falsifying it. A vessel doing 35 knots against a 30-knot spec is
  probably the most interesting fact in the exercise.
- A silent device leaves an **unknown** position. Deadline-reckoning one would draw a fabricated
  track with the same confidence as a real fix — the map would lie. "Missing" is the honest
  representation of "we do not know", not a nicety.

### 4.2 There is no physics loop

Execution in Operational mode is a **track store plus a timeline**. The assumed clock may still
advance for the exercise, but nothing moves except real vessels. This makes it the second mode
with no movement model, after Static — so "is there a simulator?" is a property of the mode, not
an assumption baked into Execution.

### 4.3 Who may report

**H1 answered 2026-09-15: the unit's own Commando.** The binding is therefore **user → unit** —
the same assignment Maneuver uses — and the two modes share one assignment model. The mode
decides what an assignment *does*, not what it *is*.

Three things follow:

- **The device is an attribute of the assignment**, not a separate identity. The app is signed in
  as the Commando, so it uses the ordinary session and needs no device credential. The device
  identifier is still recorded (§4.8) — to audit a swap and explain a discontinuity — but it
  grants nothing.
- **The gate is an instance check, not a policy.** "May this user report for *this* unit in *this*
  game" cannot be a Casbin object, because objects name resource families with no instance
  dimension — the same reason `/users/me` carries no permission check. So: a service-level check
  that the caller is the assigned Commando of this unit in this game, plus game state is
  Execution. Casbin answers the coarse question (may this participant make per-object edits in
  this game's domain, via the existing `own` action on `/games/units`).
- A failed check is **403, not 404** — the G1 reasoning: reporting a blocked action as "not found"
  is indistinguishable from an absent row and leaves a hole in the audit trail.

### 4.4 Manual placement, and why it must be bounded

The GM places units on the map during Preparation, then devices take over. **Once a device is
reporting for a unit, the GM must not be able to move it** — otherwise whoever runs the exercise
can author a position inside a live device track, and two authors for one fact is exactly what
this design exists to prevent.

Unlocking rule: manual placement becomes available when the unit is **lost** (§4.5) — that is,
when no fix has arrived within the threshold.

**The gate is fix recency alone.** Websocket connection state is *not* part of it:

| Signal | Meaning | What the GM should do |
|--------|---------|----------------------|
| connected, fixes fresh | normal | nothing |
| connected, fixes stale | app alive but not reporting — GPS lock, backgrounded, send-on-change | wait, then investigate |
| disconnected, fixes stale | app is gone — crash, power, or coverage | reposition if needed |

The middle two need different responses, which is why connection state is worth **displaying**.
But it must not *permit* a drag: a device sending fixes normally whose socket is merely flaky
would read as "disconnected", and the GM would be allowed to move a live vessel. `Presence()` is
available on the Centrifugo publisher, so this is a real temptation — the argument against it is
correctness, not capability.

### 4.5 Liveness: three states, not two

At a 5-second cadence:

| State | Age | Behaviour |
|-------|-----|-----------|
| **fresh** | ≤ 15s | draw as live |
| **stale** | 15–60s | draw the last known position **with its age** |
| **lost** | > 60s | stop asserting a position; manual placement unlocks |

The costs are **asymmetric**, and that sets the numbers:

- A too-tight *display* threshold produces a cosmetically alarming "missing" that is not real.
  10s is exactly two missed updates, so ordinary jitter would flap the map; 15s tolerates two
  consecutive losses.
- A too-tight *placement* threshold lets the GM author a position the device is still authoring,
  which corrupts the record permanently. So the gate that **writes** is conservative (60s, twelve
  missed) and the one that only **draws** is liberal.

All three derive from the configured cadence rather than fixed seconds, or a game run at 30s
reporting would show every unit as permanently lost.

### 4.6 The teleport is intended

A GM-placed position followed by the first device fix will look like the vessel jumping to a new
location. This is correct and should be visible in playback — but it must be **legible**, so both
the placement and the fix carry their **author**. Playback can then render that moment as a
handover ("manual placement → first device fix") rather than a glitch. One field, and it is the
same principle as `turn_rate_is_derived`: mark provenance so a reader can interpret what they see.

### 4.7 Time

**Live fixes are timestamped by the server.** At a 5-second cadence over a LAN, round-trip latency
is single-digit milliseconds, so the server's receive time *is* the fix time — more accurate than
the device clock and immune to skew. This removes the client clock from the hot path rather than
correcting it.

**Backfilled fixes are accepted and flagged**, not refused. Two timestamps on every observation:

| Field | Clock | Used for |
|-------|-------|----------|
| `recorded_at` | device (live fixes: server) | placement on the timeline |
| `received_at` | server | telling a judge whether the point was live or reconstructed |

Dropping a backlog instead would make a coverage gap indistinguishable from a silent vessel —
which is precisely the signal §4.5 exists to report. So dropping would poison the feature.

For the offline window the app measures its offset on reconnect and shifts the backlog. Consumer
RTC drift is roughly ±20 ppm — about 72 ms per hour — so one offset applied backwards is sound
against a 5-second cadence.

A `recorded_at` in the future indicates a wrong device clock. **Flag it; do not silently
correct it.** With server timestamping it is a diagnostic, not something on the critical path.

### 4.8 Device swap

Allowed mid-exercise provided **the same user**, the same unit, and the game is still running.
So the stable identity is (game, unit, user) and the device is the variable:

- The device belongs on the **assignment**, not on the user or the unit.
- **Every observation is stamped with the device id.** Otherwise a swap is invisible, and a swap
  often brings a small discontinuity — different receiver, antenna, calibration — that reads as
  the vessel jumping.
- **The swap is an audited event**: who and when. A gap follows it, so a judge will ask.

### 4.9 Orders have no place here

In Operational mode positions come from devices, so an order has nothing to act on. The choice is
between refusing an order and accepting it into a void — and accepting is worse, because a
Commando would be told "order accepted" while their vessel's track is driven entirely by its
receiver.

So the order endpoint should **refuse for a game in Operational mode**, with a message that says
why ("this game reports positions from its vessels") rather than a bare 404, so a client built for
Maneuver gets a diagnosable answer instead of looking like it called the wrong URL.

What remains is the **Telegram message**: B1 makes a node commander advisory, and advice is a
message, not an order. That path is unchanged by mode.

---

## 5. Time authority

The Mini-PC runs `chrony` as the LAN time source, with `local stratum 10` so it serves time
without an upstream. This helps the **Rust desktop** clients and operator machines, which can be
pointed at a LAN server.

It does **not** reach the mobile devices: stock Android and iOS offer no way to point system time
at a custom NTP server. An Android Enterprise device-owner configuration can set `NTP_SERVER`,
but Android's NTP sync is irregular, so nothing may depend on it.

Which is why §4.7 puts the server on the clock instead. The authority provides **consistency, not
absolute correctness**, and consistency is all the design needs: the assumed clock is a fiction,
and a judge needs one clock, not a correct one.

---

## 6. Joining a game, and what each caller may see

### 6.1 The room key

**Confirmed 2026-09-15: one key per game**, system generated. Consequences:

- **It admits; it does not authorise.** The key says "I am in this game". What the caller *is* —
  the Commando of this unit, a Deputy, a Wasit — comes from their assignment. Anyone who overhears
  the key must not thereby acquire a role, and a single shared key is read aloud and written on
  whiteboards by nature.
- **It is transcribed, not verified.** A password is checked by the server and never read back; a
  room key is read aloud or copied off a board and typed into a handset. So it is stored **in the
  clear**, which is a deliberate exception to how every other secret here is handled — a hash cannot
  be read out, so hashing it would make the feature impossible rather than safer.
- **96 bits of uppercase hexadecimal**, 24 characters. The alphabet is `0-9A-F` on purpose: it
  contains no `I`, `L`, `O` or `U`, so every classic transcription confusion (1/I, 0/O) is excluded
  by construction rather than by instructing the reader.
- **It dies at Closure**, by refusing the join rather than by forgetting the key.

Attribution is unaffected by sharing: the caller is already authenticated as themselves, so the
join is attributable even though the key is not.

#### What is built, and what was only planned (2026-09-15)

An earlier version of this section stated all of the above as settled design. Three of them were
plans rather than code, and the difference matters to anyone writing a client against it.

| Claim | Status |
|-------|--------|
| One key per game, system generated | **Built.** `games.room_key`, UNIQUE among live games. |
| Admits, does not authorise | **Built.** The join looks for a participant row; authority comes from the role. |
| Minted at Planning (game creation) | **Changed.** Minted on the transition **into Preparation**, which is where the concept puts the login. A game in `planning` has no key and `room_key` is absent. |
| Usable from Preparation | **Built.** |
| Dies at Closure | **Built, by refusal.** A participant the rules eject at Closure is refused with 403 on `POST /games/join` as well as on `GET /games/{id}` — one refusal is not enough, or the door that is still open is the one that matters. |
| **Rotation is a GM action, and it has to exist** | **NOT BUILT.** There is no rotation endpoint, and the repository refuses to overwrite an existing key. Preparation is entered exactly once, so a second write would mean the key the Game Master had already read out no longer works — a worse outcome than the leak it fixes, silently. A rotation endpoint needs a story for the participants holding the old key, and that story does not exist yet. |
| **Attempts are rate-limited, like `/auth/login`** | **NOT BUILT.** `/games/join` carries authentication and nothing else; the IP limiter is installed on the auth routes only. |

**On the rate limit, the reasoning in the original note was wrong.** It called the room key "the
most guessable credential in the system" — that was true of a short key and is not true of 96 bits,
where guessing is hopeless at any request rate and the limiter would buy nothing. What a limiter
would still buy is against *reconnaissance*: it slows somebody probing the endpoint to learn which
keys exist, which they cannot do any faster by trying harder. Worth adding, not urgent, and it
belongs on its own budget rather than sharing the login limiter — a room of thirty personnel behind
one NAT would otherwise spend the login budget on joining.

**Re-checked 2026-09-22**, after the Execution phase landed: both `NOT BUILT` rows above are still
accurate. There is no room-key rotation endpoint, and `/games/join` still carries authentication and
nothing else. Nothing else in this table has changed. The game-domain work since — the permission
matrix becoming authorable and reachable (B14), the taxonomy retirement rework, and the whole of
Execution — did not touch the room key. Execution introduced no new way INTO a game: the key is still
the only one, and it still dies at Closure by refusing the join.

### 6.2 Visibility: two different mechanisms

The concept imposes both, and conflating them is how you either refuse too much or leak:

| Mechanism | Shape | Example |
|-----------|-------|---------|
| **Field withholding** | same endpoint, 200, fewer fields | the game area before Execution |
| **Access refusal** | 403 | participants on the game during Closure |

**Field withholding** comes from two concept lines: personnel *"does not know where the 'Game Area'
is until it is Executed"*, and the Map Tag *"will be Hidden in other 'Stage'"*. So the game payload
is **caller- and stage-dependent** — one endpoint, one schema, different contents.

| Stage | GM / Judges | Commando / personnel |
|-------|-------------|----------------------|
| Planning | everything | name, state, own assignment |
| Preparation | everything | + readiness |
| Execution | everything | + area, map tag, positions |
| Closure | everything | **403 — ejected to the home menu** |

**Access refusal** is ordinary authorization with one wrinkle: Casbin can key on the game domain,
but "not during Closure" is a *stage* condition a policy cannot see, so the service must add it.

#### As built (2026-09-15)

Both mechanisms above are now code, and two details are worth knowing:

- **`GameVisibility` carries two independent flags**, `Entitled` and `MaySeeRestricted`. They were
  one flag until participants existed, and one flag could not express the table's Preparation row:
  a Commando is entitled to the game and not to its area. The projection requires **both** for
  `area` and `map_tag`, because they are separate fields on a value type and the pair
  "not entitled, but show the area" would otherwise leak exactly what the rule protects.
- **`GET /games/{id}` carries no application permission.** It admits participants and staff, and the
  service decides between them — **participant first**, so an administrator who is also playing a
  Commando is treated as the participant they are. Staff holding `read` on `/system/games` see
  everything, which is what lets the CMS place units during Preparation. A caller who is neither is
  refused with 403.

**Why not refuse instead of withhold?** Because a Commando during Preparation still needs the
game's name and state — their client has to render "Operation X, waiting for readiness". A 403
gives them a blank screen for information they are entitled to.

**Contract consequence:** `area` and `map_tag` must be **optional in the response schema**,
because they are absent for some callers and a generated client has to handle that. Not new:
`current_specification` is already omitted rather than nulled when a hull has no published
specification, and the schema does not require it. Same pattern.

**Confirmed 2026-09-15: hide it.** The rule hides the **game area**, not the participant's own
position.

A note used to sit here carving out an exception for Operational, where the premise — *"does not
know where the 'Game Area' is until it is Executed"* — is false, because the Commando is physically
aboard a real vessel inside that area. Operational is no longer a game mode, so the exception is
gone and the rule applies to every game there is. A mode-conditional rule that was meaningless by
design is not worth keeping for a mode that no longer exists.

The asymmetry that looked like it still needed deciding — whether participants see **other units'**
positions continuously — **was decided on 2026-09-21: they do.** A participant reading the plot sees
every hull, including the other side's and the judge side's, and fog of war is a future release
rather than something being approximated now.

The argument from concept `[3.2]` — that distance and bearing to *selected* units is an explicit
action, which implies positions are not broadcast by default — is an argument about the **live
feed**, where the vessels are real and their positions are measurements. Inside a game the plot is
derived from the fix chain, and there is nothing to leak before play begins because the chain is
empty until the transition materialises the origins. That is a property of the data rather than a
rule somebody has to remember to apply. See `README.md` §1.1.

### 6.3 Preparation: the readiness gate and who may advance

Both rules are from the concept and both became enforceable when participants did.

**The gate has four parts.** `preparation → execution` requires:

1. **At least one unit assigned.** An exercise with no hulls has nothing to manoeuvre, nothing to
   give an order to, and no unit for a Commando to lead. Concept `[2.2]` puts placement here — "the
   'Game Master' will be required to place the Unit in the Map" — and there is no map to place
   anything on.
2. **Every assigned unit placed on the map.** This part is the one that was missing until
   2026-09-21, and it is not pedantry. One hull of twelve satisfies condition 1, and the other
   eleven would reach execution with **no origin leg** — so they would be in the exercise, absent
   from the plot, and unorderable for the rest of the run, because the chain is append-only and an
   origin can only be written at the transition. A game whose force is a third placed is not nearly
   ready; it is a game that will lose two thirds of its pieces silently.
3. **At least one participant on the exercise side.** This is the half that is easy to omit, because
   omitting it makes the gate vacuous: "every non-judge participant is ready" is trivially true for
   a roster holding only Judges.
4. **Every one of them ready.**

*Exercise side* is read from `game_roles.is_judge_side`, not from a role name, because lookup rows
may be renamed (G2) — a name comparison would fail open the first time somebody renamed Wasit. The
Game Master is **not** judge side, so the GM presses Ready too.

The refusal names **every** missing ingredient at once rather than the first one found. A freshly
created game is missing three of the four, and reporting them one per attempt would make the Game
Master press Advance three times to learn three things. The placement sentence is suppressed when
no pieces are assigned at all, because "0 of 0 units have no position" reads as a bug rather than as
an instruction.

- Readiness is **refused** with 409 for a judge-side holder rather than accepted and ignored — a
  Referee who pressed Ready, got a 200, and then watched the game refuse to advance would reasonably
  think the blockage was theirs.
- A **role change clears readiness**. The declaration is about the role held, and letting it survive
  a change would allow declaring under one rule and having it counted under another.

**"At least 1 unit and 1 commando" is ONE condition, not two.** Concept `[1.7]` defines a game piece
as one "commanded by the Commando assigned to it", and `game_units.id_commander` is `NOT NULL` — so
a hull with nobody to steer it is unrepresentable, and counting pieces counts commanders. See §6.4.

**Who may advance.** `POST /games/{id}/transitions` is authorised by being **this game's Game
Master** — an in-game role held by a participant, not an application permission, because a Casbin
object names resource families and carries no instance id. The route itself carries no permission
middleware: any authenticated caller can reach it and only the GM is obeyed. That is a widening of
who may ask and a narrowing of who is answered.

### 6.4 The game pieces: what is in an exercise, and who steers it

Concept `[1.7]`: *"Units — The 'Game Piece' that will be commanded by the Commando assigned to it"*.
`game_units` holds that relationship. Four of its decisions are worth recording, because each one
refuses a state somebody would otherwise expect to be possible.

**A piece is DEFINED by having a commander.** `id_commander` is `NOT NULL`. This is the same point
§6.3 makes from the gate's side: there is no state in which a hull is assigned to an exercise and
nobody commands it, so the rule needs no check of its own.

**The commander must be a participant of THIS game**, enforced by a composite foreign key
`(id_game, id_commander) → game_participants (id_game, id_user)`. The database refuses a stranger
without the service having to remember to check, and that matters more than usual here: a future
endpoint that assigned a commander and forgot the check would put somebody in command of a hull and
nothing would say so. It is also what makes the gate's arithmetic trustworthy, because every piece
is guaranteed to name somebody the roster contains.

**The judge side does not command units.** A Referee "Assesses performance and gives points" — they
run the exercise rather than taking part in it, and a piece commanded by a judge is a player wearing
a referee's shirt. Refused with 400 naming the field, read from `is_judge_side` rather than a role
name, for the reason in §6.3.

**A hull may be in several games at once.** A ship is a real vessel with one hull number; an exercise
is a plan. Uniqueness is `(id_game, id_unit)`: the same hull cannot be listed twice in one exercise,
and may appear in as many exercises as the scenario needs. The alternative — one game per hull — would
make the fleet register a schedule rather than a register.

**The force closes at Execution, on the same window as the roster** — `planning` and `preparation`,
because what Preparation is for is people and hulls arriving. The READS are not bound by that window:
`GET /games/{id}/units` answers at every state, since closure is where the analysis happens.

**Discovery is asymmetric, deliberately.** A Commando learns which hull they steer from
`POST /games/join`, whose `commanded_units` carries **their own pieces and nobody else's**.
`GET /games/{id}/units` is the staff view of the whole order of battle, and handing that to a
participant would leak through the back door exactly what the route's permission refuses at the
front — knowing the other side's order of battle before the exercise starts is most of the exercise.

**Not here yet: where a piece STARTS on the map.** Concept `[2.2]` has the Game Master placing units
during Preparation, which is a different fact from being assigned and belongs with the exercise's
running state rather than its composition.

**UPDATE 2026-09-22 — this is built.** `PUT /games/{id}/units/{unit_id}/placement` writes a hull's
starting position, `DELETE` takes it away, and `GET /games/{id}/placements` is the setup view, with
`placed`, `unplaced` and `ready` so a map screen can tell the Game Master whether pressing Advance
will be refused for a reason they can fix. The reasoning above held: being in the exercise, sitting
in the task organisation, and starting at a coordinate are three independent facts, which is why
`/placement` sits beside `/hierarchy` rather than being a field on either.

Two consequences worth knowing. The **writes close at execution** — the Service, not the route,
enforces that window, because a placement written after play begins would be a second and editable
answer to a question the fix chain has already frozen. The **read does not**: a Game Master
reviewing where the force started needs it after the exercise, not only before.

And the gate depends on it, exactly as §6.3's condition 2 says: one hull placed of twelve is not a
placed force, and the eleven could never be given positions afterwards.

---

## 7. Open decisions

| # | Question | Recommendation |
|---|----------|----------------|
| H1 | ~~Are reporting devices operated by the unit's Commando, or by separate personnel?~~ | **ANSWERED 2026-09-15: the unit's own Commando** — see §4.3. One shared assignment model, so steps 4 and 5 are unblocked |
| H10 | ~~Does Operational mode accept **orders** at all, given positions come from devices?~~ | **MOOT.** There is no order endpoint outside a game. §4.9's reasoning was right — refusing beats accepting an order nothing applies — and the question dissolved with the mode |
| H11 | When may the room key be rotated, and by whom? | **STILL OPEN, and deliberately not built.** §6.1's recommendation — the GM, at any time, old key invalidated immediately — is *not* what shipped: there is no rotation endpoint and the repository refuses to overwrite a key. Preparation is entered once, so an overwrite would silently invalidate a key the Game Master had already distributed. Building it properly needs a decision about the participants holding the old key: are they ejected, or do they keep their place? The code currently has no answer, which is why there is no endpoint |
| H12 | Does a participant's map show **all** units, or only their own plus the ones they select? | **ANSWERED 2026-09-15 for the feed:** the Flutter app is a beacon and displays NOTHING, while the Rust command centre displays every vessel. So there is no per-caller projection to build. Still open for a GAME, where `[3.2]`'s selected-unit distance query suggests positions are not broadcast by default |
| H2 | Confirm cadence 5s, fresh 15s, lost 60s — and per-game configurable | Confirm |
| H3 | Confirm manual placement gates on fix recency only, with connection state display-only | Confirm |
| H5 | Bound the backfill window, or accept anything with a flag? | Accept with a flag; refusing loses real evidence |
| H6 | Does the device supply course and speed, or must the server derive them? | Ask; derived values must be marked derived |
| H8 | Does Static still need to ship in this release? | Confirm deferral is acceptable to the client |
| H9 | Does a unit going **lost** raise a warning automatically, or is that the GM's judgement? | **ANSWERED 2026-09-21: the GM's judgement, always.** The server never raises a warning on its own. So liveness (`fresh`/`stale`/`lost`) is something the server *displays* and the GM *acts* on, which means there is no warning emitter to build and `warn` is a purely GM-authored action |
