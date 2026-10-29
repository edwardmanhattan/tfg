# Presentation map client

The client-side view of live ships on a real-world tile map. Presentation mode shows where ships are; it never simulates where they go. Simulation is a fix source: the registry ingests fixes regardless of provenance, so presentation displays sim-sourced fixes alongside wire ones without simulating anything itself.

## Language

**Fix**:
One accepted position report for a ship: latitude, longitude, and timestamp, with optional heading and speed. The client may interpolate between accepted Fixes for display only; interpolation never creates a Fix or a trail sample.
_Avoid_: ping, update, coordinate

**Ship**:
A tracked identity with its latest accepted Fix and a stale flag.
_Avoid_: vessel, target, contact

**Track**:
The ordered history of accepted Fixes for one ship, newest last.
_Avoid_: log, history

**Trail**:
The rendered recent portion of a Track, bounded to the last 60 accepted Fixes (about 2 minutes at a 2-second poll). It contains observed Fix positions, never interpolated samples.
_Avoid_: path, route, breadcrumbs

## Simulation

**Source**:
The provenance of a Fix: wire (backend report), sim (synthetic, emitted from orders), or game (authoritative Minos exercise position, computed by the server — noiseless like sim, scenario-stamped, never wall-aged).
_Avoid_: origin, type

**Order**:
A legacy waypoint-and-speed directive retained for old replay data and possible future navigation work. It is not the current helm directive.
_Avoid_: current helm directive, command (that's the multi-unit directive, see Command), instruction

**HelmOrder**:
One persistent directive for a single GameUnit: a compass heading and speed in knots. An accepted HelmOrder replaces the previous one; speed zero is Hold position and retains the last accepted heading. It is not a waypoint, route, or client-invented destination.
_Avoid_: waypoint, move order, navigation plan, instruction

**Command result**:
The reconciliation state of a HelmOrder: Draft, Pending, Accepted, Clamped, Unknown, Refused, or Superseded. A command result is separate from connection freshness; a disconnected client does not downgrade an already accepted result.
_Avoid_: optimistic acceptance, silent retry, raw error string

**Owned**:
A ship driven by the sim from a player's HelmOrders or legacy Orders.
_Avoid_: ownship, friendly

**Traffic**:
A ship driven by wire fixes, never ordered.
_Avoid_: background, NPC

**Waypoint**:
The ordered position an owned ship is steering toward.
_Avoid_: destination, target

**Game time**:
The session clock, derived from real time through a fixed ratio (ADR-0004): `game_now = game_start + elapsed_real × ratio`. Quoted as `game_ts` readings and `G+mm:ss` elapsed; never stored on fixes.
_Avoid_: sim time, virtual time, compressed time
_For a connected execution the ratio seeds from the Minos `time_factor` and the display follows Minos pause/resume (ADR-0009); the scenario integral itself is never recomputed client-side._

**Pause**:
A full hold of game time: motion, game_now, and the log clock freeze while the real wall clock runs on. Enforced tick-wise by the sim; session datetimes are never edited.
_Avoid_: freeze (that is the effect, not the verb), stop
_A connected execution pauses through Minos (`POST /games/{id}/pause`), which also closes the exercise to orders; the local hold follows the scenario hold._

**Category**:
What a unit is in the world (Ship, Plane, Tank, Port). Determines which stat keys exist — the schema, not the values.
_Avoid_: kind, sort, domain
_Minos `unit_categories` are a separate, operator-authored taxonomy, shown as-is in the picker drill; no correspondence with this enum is assumed._

**Class**:
The stat values (abilities) for a family of units: a tanker's 16 kn vs a destroyer's 30 kn. All types under a class behave identically in the sim.
_Avoid_: dev-type (collapsed — was a fuzzy duplicate), model

**Type**:
The player-facing designation of a unit (`Type 052D`, `Arleigh Burke`). Flavor only: identification, no mechanics; many types map to one Class.
_Avoid_: variant, mark

**Unit**:
A catalog entry: taxonomy (Category/Class/Type) plus the class stat row. What a unit IS, defined once.
_Avoid_: blueprint, template

**GameUnit**:
An in-game instance referencing a Unit: position, state, order. What a unit IS DOING in one session.
_Avoid_: unit instance (redundant in context), entity

**Pre-embarked unit**:
A hull the fleet register says another hull carries. Deploying the parent brings them with it — every pre-embarked unit joins the exercise aboard her, uncommanded, in the same write — **when the operator asks for it**: `with_contents` is a choice, per deployment, because it changes the force's size. The register's copy is what the parent CARRIES (`register_contents`, or `GET /units/{id}/embarkations`); the exercise's own composition is what is ACTUALLY aboard (`id_host` on the piece row), and the two are allowed to differ. The console stages the children beside the parent, and marks them as brought so the plan does not write them twice.
_Avoid_: complement (that is the KINDS and counts a hull may carry, not the units), passenger (that is the exercise's word for what is aboard)

**Command**:
One directive from a commander over units in their jurisdiction: it fans out into one per-unit directive, currently a HelmOrder for helm control or a legacy Order for waypoint navigation. The sim never sees commands, only per-unit directives.
_Avoid_: order (that's the per-ship directive)

**Jurisdiction**:
The set of units a commander may command: full orders inside it, view-only outside.
_Avoid_: scope

**Commander**:
The role commanding a unit or a group. Higher ranks override lower ones; the organizer commands all. One person commands ONE thing — a hull or a group, never both and never two — and every group has one: a game does not advance to preparation while a group is nobody's.
_Avoid_: player (that's who holds the role, not the role)

**Organizer**:
The role that sets up a session: players, units with placements, groups with commanders, and the real and game time windows. Commands all units.
_Avoid_: admin, host

**Group**:
One node of the session's task organisation: id, name, level, commander, mustered units and child groups. Every child sits at a strictly lower level than its parent. A group is somebody who orders the force beneath it, so it is never merely a caption: a group with no commander is a gap the advance refuses.
_Avoid_: team, party

**GroupKind**:
The level of a group, lowest first: Unsur, Satuan Tugas, Gugus, Operasi Gabungan. Carries an explicit gapped rank (never a position), so inserting a level in the middle never renumbers.
_Avoid_: echelon (that's the Minos vocabulary term, see Minos docs), tier

**Unsur**:
The lowest group level: individual units under one commander.
_Avoid_: element, squad

**Satuan Tugas**:
A group of units and/or Unsur groups under one commander. `Satgas` is the accepted local alias.
_Avoid_: squadron, fleet

**Gugus**:
A group of lower groups under one commander.
_Avoid_: flotilla

**Operasi Gabungan**:
The highest group level: the whole task organisation under one commander.
_Avoid_: joint operation (that's the exercise, not the group)

**Session**:
One organized play instance: time windows, players, units with placements, groups with commanders. Runs setup → live → closed.
_Avoid_: game (that's the model, not the instance), mission
_When connected, the held game's stage is read from Minos (`GET /games/{id}`) and the local machine follows it; local Back/Replan never move the backend, which is forward-only._

**Seat**:
One command slot in a session: helm of a unit, or commander of a unit or group. Users are bound to seats; one user may hold several.
_Avoid_: slot, position

**Helm**:
The seat that drives one unit: the only binding that makes a unit playable.
_Avoid_: driver, pilot

**Roster**:
The organizer-kept list of named users available to a session: the source of players and seat holders. Local only until a networked backend exists.
_Avoid_: crew, manifest
_When connected, session membership is read live from the backend (user directory, game participants); the local list holds only unposted drafts._

**Invite**:
An organizer-issued code binding a named roster entry to a session seat. Generated locally (mock backend in dev); redemption over the network is a later slice.
_Avoid_: invitation (that's the message, not the code), token

**Group symbol**:
A Group drawn as a symbol rather than a Zone: its affiliation frame, a plurality icon of what its members are, its level colour, and `name (count)` at the member centroid. Fixed size, never rotated — a Group has no heading and no published dimension. A nested Group goes quiet when it is a symbol unless it is selected; a selected one draws beside its parent, and the ladder is never overridden to show it.
_Avoid_: flag (that was a name and a count, not a symbol)

**Zone**:
The rendered ground a Group holds: a live hull around its member positions, dilated outward far enough to enclose those members' markers. In the level colour. Collapses to a group symbol when the group's own extent is too small for a shape to read.
_Avoid_: area (that's organizer-drawn, a different feature), region

**Affiliation**:
What a thing is in relation to the operator watching the map: Friendly, Neutral, Hostile, or Unknown. Stated, never inferred: a unit the operator drives is Friendly unless something is declared about it, and a unit they do not drive is never painted Hostile. Declared per unit, per group, or per branch for one exercise; "this branch is mine" is a standing fact about the operator rather than about a session. Operators read it as **Side**.
_Avoid_: side (that is the operator-facing word, not the canonical one), faction, relation
_2525D calls this standard identity, and adds assumed friend and exercise/pending; those two are deliberately not adopted._

**Representation**:
How one entity is drawn at the current zoom, chosen from its size on screen rather than from the zoom itself: a Unit's photograph, silhouette or symbol; a Group's Zone or its far symbol. A Representation is never stated by the operator and never forced by a tool — it is what the thing actually needs at that zoom.
_Avoid_: LOD, detail level, zoom level (that's the input, not the answer)

**Desktop**:
One scope's command view: the four panes (map, roster, inspector, orders) with action panes filtered to the scope's jurisdiction. Multi-scope players switch between desktops; organizer and observers get a single merged one.
_Avoid_: window (that's the OS frame, not the scope view), screen

**Force draft**:
The force as the operator has it on the client, before Minos has been told any of it: which hulls are in the exercise, where each STARTS, and who commands each. Placing, moving a placement, and handing a hull to a player are all draft edits and none of them is a backend write. Minos holds the record, so a Force draft is written through in one ordered diff at a stage advance — assignment before placement, because a hull Minos has never heard of has no position to set; removals last, so nothing is briefly absent while it moves.
_Avoid_: staged force (that is not a state Minos has either), local force (every force on screen is local until it is not), unsaved force
_A Force draft tolerates a hull with no commander, which Minos does not: `game_units.id_commander` is NOT NULL, so the advance refuses and names the hulls rather than seating whoever happened to be free._

**Staged course**:
The course a hull is to START on, as the operator has said it while planning: the bearing the map's rotator was turned to, held in the Force draft beside the hull's position. Minos has no write for a course — a placement carries a latitude and a longitude and nothing else — so a Staged course is a PLAN, and it goes out as a Helm order when the exercise starts playing, for the hulls the operator's own seat commands. The rest stay in the plan, and the console names them.
_Avoid_: heading (that is a fix's own figure), planned course, draft heading
_A Staged course is not a Helm order: it is the plan of one, and the server takes it only once the exercise is playing._

**Roster draft**:The roster as the operator has it on the client, before Minos has been told any of it: who is seated, as what, under which call sign, and who is leaving. Staging a seat, changing a role and unseating are all draft edits and none of them is a backend write. A Roster draft is written through in one ordered diff at a stage advance — seats before force assigns, because a piece IS a hull commanded by a participant, and unseats after the force, so nobody goes seatless while their hull is still being handed over.
_Avoid_: staged roster, local roster, unsaved roster_

**Helm order**:
A commander's heading-and-speed order for one hull, posted at once with `POST /games/{id}/units/{unit_id}/order` in every mode that takes orders (maneuver and scenario alike; static takes hand-set positions and refuses orders). There is no client-side hold for a step — the server owns timing, and an accepted order applies immediately.
The CONSOLE offers the helm in a maneuver exercise alone (ADR-0021): a scenario exercise is played through its steps, so the commander's verb there is the step's readiness, and the server's holding of an order is a fact about the wire, not a reason to offer an instrument.
_Avoid_: planned helm, queued order, pending order (the server queues nothing, and the client no longer holds anything either)_

**Step ready**:
A commander's statement that their helm for the coming step is set. The server holds one readiness flag per participant and the step gate counts it, so it is the same flag as session readiness; the client separates the two by when each is offered (session readiness in preparation, step ready in execution) and clears it at each step start. Distinct from **Session ready**, which is a player's declaration that the exercise may begin.
_Avoid_: ready (unqualified — two different declarations share the server's one flag)_

**Book draft**:
The scenario book as the operator has it on the client, before Minos has been told any of it: brand-new scenarios with their steps, plus step adds and step cuts against scenarios Minos already holds. Authoring a scenario or a step is a draft edit, never a backend write. A Book draft is written through in one ordered diff at a stage advance — creates before their steps, because a step belongs to a scenario the server has to hold first — and always before the transition, because the server only takes pages in planning.
_Avoid_: staged book, local book, unsaved book_

**Saved scenario**:
A whole scenario the operator authored and kept on this machine: its name, its description, its window, its steps, and each step's place and its related hulls. A saved scenario outlives every game it was ever deployed into, and it is a PLAN — the game's own scenario is a deployment of it, and the two are not the same thing.
_Avoid_: template, preset, scenario library entry, draft scenario_

**Library**:
The file of saved scenarios, kept under the data dir and shared by every game this machine opens. Minos holds no catalog of scenarios — a game has exactly ONE book — so the catalog is the client's. It never writes to the network.
_Avoid_: catalog, store, scenario bank, database_

**Composer**:
The surface where a scenario is authored: its name, its description, its window, and its steps. The composer's scenario is a saved scenario in the making, and adding it to a game stages it into the Book draft.
_Avoid_: editor, builder, form, scenario maker_

**Scenario list**:
The one list of scenarios the composer offers: the draft being written, the scenarios the held game holds, and the ones saved on this machine. Each row says which of the three it is, because a scenario in the game and a scenario in the library are different things. Opening one puts its steps in the composer's steps column, which shows one scenario at a time.
_Avoid_: library list, scenario picker, scenario browser_

**Step place**:
Where a step points, as a number: a latitude and a longitude. A step's words carry the same place in text, and the two are the same digits, because an exercise authored against one position and reviewed against another is a scenario nobody can trust.
_Avoid_: pin, marker, waypoint, target

**Setup**:
The session phase for windows, roster, placements, and seats. Nothing moves, nothing is ordered; booting into Setup is the lobby.
_Avoid_: lobby (that's Setup with defaults), staging

**Live**:
The session phase where the game runs under per-seat authority. Orders flow only when armed as well as live.
_Avoid_: running (that's the engine, see Mode)

**Closed**:
The session phase after End: frozen map plus journal transcript, read-only. Never rewound; reopening starts a new Setup.
_Avoid_: finished (that's the file, not the phase), archive

**Mode**:
The exercise's mode as Minos states it and the console reads it: `maneuver`, `static` or `scenario`. The mode decides the verbs — a maneuver exercise steers by helm order, a scenario exercise is played through its steps (no helm; the commander's verb on the current step is the step ready), and a static exercise sets positions by hand. Fixed at creation, re-read on every session read, and a mode the console does not know is no mode: it fails closed. Orthogonal to phase.
_Avoid_: state (that's the phase axis), engine (that's the retired console's simulation/presentation switch)

**Island**:
One floating panel (Session, Roster, Inspector, Orders, Log) over the fullscreen map. Closable and reopenable; replaces dock menus.
_Avoid_: dock, pane, window (that's the OS frame)

**Wizard**:
The stepped onboarding (Welcome → Session → Fleet → Review → Live) that IS the Setup phase: a centered island, skippable, dismissed on going Live. Being replaced by the draft's stages (see Creation): the creation flow does not keep two ways to create a game, and the wizard's five window fields become stage 01's derived time base.
_Avoid_: tour, guide

**Log**:
The append-only action journal of a session: one entry per action or event, replayable from genesis and retraceable per game minute. Entries name a commander seat as actor; fixes are cited by sequence, never duplicated.
_Avoid_: track (that's the position record, see Track)

## Comms

**Message**:
One communication inside an exercise, as Minos holds it. It has a KIND, a MARKING, a DEGREE, an audience (or none), and content. A Message is the server's record: the console never keeps a message the server does not hold, and the console's copy is refreshed by the socket and by the read.
_Avoid_: mail (that's the staff correspondence only), note, post (that's the book's)

**Telegram**:
The KIND of message that passes between the personnel — radio traffic, an order broadcast. A telegram sent by the Game Master or a Judge must be sent AS a scenario role, and it is a broadcast of orders rather than a conversation: the server refuses a reply on it.
_Avoid_: radio message, dispatch

**Administrative**:
The KIND of message that passes between the Judges and the Game Master. Staff only — the server refuses it from any other seat — and the only kind that can be answered (a reply links to the message it answers).
_Avoid_: staff mail (that's the idea), memo

**Marking**:
The per-message classification: TERBUKA (open), TERBATAS (restricted) or RAHASIA (classified). It gates who may read the message — only TERBUKA may be a broadcast — and it is the message's own border colour. The server publishes the colour; the console maps the CODE to its own ink and never paints with the server's hex.
_Avoid_: classification (that is a hull's category in the taxonomy), priority (that is the degree)

**Degree**:
The urgency a sender marked a message with: BIASA, SEGERA, SANGAT SEGERA or KILAT. A lookup value on the server, read from its own catalogue; it is stored and displayed and drives nothing.
_Avoid_: rank (that's a seat), priority

**Broadcast**:
A message with no recipient list: every participant of the exercise may read it. Only TERBUKA may be one. The server derives broadcast from the empty recipient list — there is no flag to set.
_Avoid_: general, all

**Receipt**:
The instant an addressed recipient read a message, kept per recipient. A broadcast has none: the server refuses a receipt for a message nobody was addressed by. The receipt matrix is what the sender is entitled to see.
_Avoid_: read flag (that's the console's own unread bookkeeping), seen

**Assumed identity**:
The scenario role a sender spoke AS. When a message carries one, the server withholds the real author — from everybody, the Game Master included — so the console shows the assumed identity alone and never guesses the author back.
_Avoid_: alias, pen name, role (that's the seat)

## Creation

The flow that makes a game or a Saved Scenario: the landing, one draft, six
stages, three presses (`docs/creation-flow-handoff.md`).

**Draft**:
One game being composed on this machine: its mode, its terms and time base,
its force, its task organisation, its book, its roster, and the journal of what
a previous press already wrote. Every stage edits memory and nothing is written
until a press in stage 06; closing the window loses it. A draft is a create
(`Blank`, `SavedScenario`) or a diff (`Reopened`), which its origin decides.
_Avoid_: session (that's the held game), wizard state, template

**Origin**:
What a draft was made from — `Blank { mode }`, `SavedScenario { mission_id }`
or `Reopened { game_id }`. It decides what is prefilled and whether a press is
a create or a diff against what the server holds; a `Reopened` draft only
exists for a game in `planning`.
_Avoid_: source (that's a Fix's provenance), kind

**Stage**:
One row of the draft's rail: essentials, units, groups, scenario, players,
review. Six for a scenario draft and five for a static or maneuver one, because
neither of those has a book. The rail NAVIGATES — a stage is reached by
clicking it, not by finishing the one before — and only the three verbs are
gated.
_Avoid_: step (that's the scenario's), wizard step, phase

**Verb**:
One of the three presses in stage 06: Save as Game Session, Save as Scenario,
Play Directly. A verb is disabled and says why, using the same blockers that
would stop it; a static or maneuver draft earns two of them.
_Avoid_: action, mode

**Ladder position**:
A numbered write in one of the two write orders: 01–09 for a game, M1–M3 for a
Saved Scenario. The ladder's numbers are a different series from the rail's and
do not move with the mode.
_Avoid_: stage, step

**Saved Scenario**:
A server-side detached mission: steps, sub-steps and the hulls their audiences
name, with no game, no map, no seats, no groups and no commanders. Made by Save
as Scenario, listed on the landing's START FROM, and a draft may be seeded from
one.
_Avoid_: template (that's the CMS's role vocabulary), mission, library

**Considered**:
A person the operator has made a decision about in the Players stage: seated, or
explicitly `NOT PLAYING`. The stage's denominator is `seated ∪ considered`, so a
person nobody touched does not count. `NOT PLAYING` writes nothing; it deletes
the participant row when there is one.
_Avoid_: selected, ticked, added

**Blocker**:
Why a verb cannot run: the stage the operator can fix it in, and a sentence that
names the field, the hull or the body. The sentence is the server's rule read
one stage earlier, never a client-only nicety.
_Avoid_: error, validation message

**Journal**:
The record of what a press has already written — one landing per ladder
position, plus the ids the server returned for staged rows. A resumed press
reads the journal, never the draft's optimism: a position the journal says
succeeded is not sent again.
_Avoid_: log (that's the session's action journal), history

**Ring pack**:
What a group does to the map when it is dropped on it: its hulls laid out around
the drop point — first hull at the anchor, then rings of `6k` at radius `k·R`,
spread from 12 o'clock — with a sub-group keeping its own centre. Nothing about
the pack is stored; the commit writes N placements in unit-id order.
_Avoid_: cluster (that's the gesture), formation, spread

