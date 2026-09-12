# Presentation map client

The client-side view of live ships on a real-world tile map. Presentation mode shows where ships are; it never simulates where they go.

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
