# Presentation map client

The client-side view of live ships on a real-world tile map. Presentation mode shows where ships are; it never simulates where they go. Simulation is a fix source: the registry ingests fixes regardless of provenance, so presentation displays sim-sourced fixes alongside wire ones without simulating anything itself.

## Language

**Fix**:
One accepted position report for a ship: latitude, longitude, and timestamp, with optional heading and speed.
_Avoid_: ping, update, coordinate

**Ship**:
A tracked identity with its latest accepted Fix and a stale flag.
_Avoid_: vessel, target, contact

**Track**:
The ordered history of accepted Fixes for one ship, newest last.
_Avoid_: log, history

**Trail**:
The rendered recent portion of a Track, bounded to the last 60 fixes (about 2 minutes at a 2-second poll).
_Avoid_: path, route, breadcrumbs

## Simulation

**Source**:
The provenance of a Fix: wire (backend report) or sim (synthetic, emitted from orders).
_Avoid_: origin, type

**Order**:
One active directive on an owned ship: a waypoint plus a speed. New orders overwrite; cancel clears; arrival holds position.
_Avoid_: command (that's the multi-unit directive, see Command), instruction

**Owned**:
A ship driven by the sim from a player's orders.
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

**Pause**:
A full hold of game time: motion, game_now, and the log clock freeze while the real wall clock runs on. Enforced tick-wise by the sim; session datetimes are never edited.
_Avoid_: freeze (that is the effect, not the verb), stop

**Category**:
What a unit is in the world (Ship, Plane, Tank, Port). Determines which stat keys exist — the schema, not the values.
_Avoid_: kind, sort, domain

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
One directive from a commander over units in their jurisdiction: it fans out into one Order per named unit. The sim never sees commands, only orders.
_Avoid_: order (that's the per-ship directive)

**Jurisdiction**:
The set of units a commander may command: full orders inside it, view-only outside.
_Avoid_: scope

**Commander**:
The role commanding a unit, a Satgas, or a Gugus. Higher jurisdictions override lower ones; the organizer commands all.
_Avoid_: player (that's who holds the role, not the role)

**Organizer**:
The role that sets up a session: players, units with placements, groups with commanders, and the real and game time windows. Commands all units.
_Avoid_: admin, host

**Satgas**:
A group of units under one commander.
_Avoid_: squadron, fleet

**Gugus**:
A group of Satgas under one commander.
_Avoid_: flotilla

**Log**:
The append-only action journal of a session: one entry per action or event, replayable from genesis and retraceable per game minute. Entries name a commander seat as actor; fixes are cited by sequence, never duplicated.
_Avoid_: track (that's the position record, see Track)
