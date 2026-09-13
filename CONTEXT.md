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
_Avoid_: command (reserved for the broader game model), instruction

**Owned**:
A ship driven by the sim from a player's orders.
_Avoid_: ownship, friendly

**Traffic**:
A ship driven by wire fixes, never ordered.
_Avoid_: background, NPC

**Waypoint**:
The ordered position an owned ship is steering toward.
_Avoid_: destination, target
