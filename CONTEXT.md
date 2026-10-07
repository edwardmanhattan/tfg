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

**Command**:
One directive from a commander over units in their jurisdiction: it fans out into one per-unit directive, currently a HelmOrder for helm control or a legacy Order for waypoint navigation. The sim never sees commands, only per-unit directives.
_Avoid_: order (that's the per-ship directive)

**Jurisdiction**:
The set of units a commander may command: full orders inside it, view-only outside.
_Avoid_: scope

**Commander**:
The role commanding a unit or a group. Higher ranks override lower ones; the organizer commands all.
_Avoid_: player (that's who holds the role, not the role)

**Organizer**:
The role that sets up a session: players, units with placements, groups with commanders, and the real and game time windows. Commands all units.
_Avoid_: admin, host

**Group**:
One node of the session's task organisation: id, name, level, commander, mustered units and child groups. Every child sits at a strictly lower level than its parent.
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

**Roster draft**:
The roster as the operator has it on the client, before Minos has been told any of it: who is seated, as what, under which call sign, and who is leaving. Staging a seat, changing a role and unseating are all draft edits and none of them is a backend write. A Roster draft is written through in one ordered diff at a stage advance — seats before force assigns, because a piece IS a hull commanded by a participant, and unseats after the force, so nobody goes seatless while their hull is still being handed over.
_Avoid_: staged roster, local roster, unsaved roster_

**Book draft**:
The scenario book as the operator has it on the client, before Minos has been told any of it: brand-new scenarios with their steps, plus step adds and step cuts against scenarios Minos already holds. Authoring a scenario or a step is a draft edit, never a backend write. A Book draft is written through in one ordered diff at a stage advance — creates before their steps, because a step belongs to a scenario the server has to hold first — and always before the transition, because the server only takes pages in planning.
_Avoid_: staged book, local book, unsaved book_

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
The engine switch: simulation (the sim ticks) or presentation (wire-only, owned ships frozen). Orthogonal to phase.
_Avoid_: state (that's the phase axis)

**Island**:
One floating panel (Session, Roster, Inspector, Orders, Log) over the fullscreen map. Closable and reopenable; replaces dock menus.
_Avoid_: dock, pane, window (that's the OS frame)

**Wizard**:
The stepped onboarding (Welcome → Session → Fleet → Review → Live) that IS the Setup phase: a centered island, skippable, dismissed on going Live.
_Avoid_: tour, guide

**Log**:
The append-only action journal of a session: one entry per action or event, replayable from genesis and retraceable per game minute. Entries name a commander seat as actor; fixes are cited by sequence, never duplicated.
_Avoid_: track (that's the position record, see Track)
