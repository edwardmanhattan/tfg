# The unit details panel (2026-10-20)

The Game Master's question — *what is that ship?* — had no answer in the
console. The Inspector dock existed, with readouts, but only a capture could
open it, and there was no way to select a hull from the map at all: the
markers are painted into a canvas, so they have no hitbox, and a press on a
ship landed on the map surface and panned the chart.

## The direction

**A click on a hull brings up its details.** The press resolves the unit
under it; the release opens a dock with three ladders — what it is, where it
is, and the figures the register published for it.

## What the panel says

| ladder | rows |
|---|---|
| identity | type, class, commander, embarkation |
| the fix | position, course, speed, state, trail |
| the figures | length, beam, draft, displacement, speed max, cruise, range, turn rate |

Every figure is a fact the console already holds, and **every fact it does
not hold is a dash**. The console never guesses:

- a hull with no published specification says so;
- a hull whose embark read has not been made says `—` for its embarkation
  rather than claiming it is on the water — the embark read is the server's
  per-host passenger list, and a host that was never read has no answer to
  give;
- a hull with no wire fix says `no fix` rather than drawing a position.

## The decisions that took the trouble

**The pick is decided at the PRESS.** GPUI delivers a mouse release to
whatever is under the pointer, so a release that lands on a panel never
reaches the map surface at all. A release-time pick would therefore be a
pick nobody made — the panel would open on a hull the operator had not
pointed at, or on one they had pointed at a second ago. So the press decides,
and a press that turns into a pan UNDOES the pick: a drag that starts on a
ship must leave the selection exactly where it was, or every drag across the
chart would move the target under the operator's hands.

**The radius is 30 WINDOW points,** converted through the frame's own scale,
so a hull is as easy to point at at one zoom as at another.

**The selection is by the unit's OWN id,** not by an index into the roster:
the roster changes under the panel, and an index would hand the next hull's
details to whichever row moved into that slot.

**The dock holds the pointer.** The map surface is a full-window hitbox, so
a press on CENTRE would also reach the map — panning the chart and picking
whatever hull was under the button. The dock truncates the hover path below
it, the way the column's islands do.

**The backend stopped dropping the class.** The server publishes
`unit_class_name` on every piece row and the parser threw it away, so the
panel had to say "no class" for a hull whose class was right there.

**One order loop bug fixed on the way.** The embark-spec read returned from
the order loop when the server could not be reached — which dropped every
LATER order on the floor. A console that stops listening looks exactly like
a console that has nothing to say.

## The pictures

- `unit-details.png` — the whole window, hosted API, game 9.
- `unit-details-crop.png` — the panel.

Captured with `TFG_GAME=9 TFG_UNIT=5`: `TFG_UNIT` names the unit the panel
opens on, the same "pin the state, never the gesture" precedent as
`TFG_SEL` — the compositor makes a synthetic click unreliable here.

## How it was verified

**Three tests.** The hit test on its own (`a_click_on_a_hull_picks_the_hull`),
the click END TO END through GPUI's own event path
(`a_click_on_a_hull_opens_its_details`), and the drag that starts on a hull
and selects nothing while still panning the chart
(`a_drag_that_starts_on_a_hull_selects_nothing`). The middle one exists
because a kernel test of the hit-test alone passes while the WIRING is dead
— which is exactly how the last wheel defect survived a green suite.

`cargo test --lib`: **622 pass**, the same four pre-existing failures.

## The reads the panel makes for itself

**The figures are asked for, not remembered.** They used to come from the
local store, and the store is synced only at BOOT — so an operator who
signed in through the console's own card saw a dash for every figure until
the next launch. When a unit is selected the shell asks the binary for that
hull's published specification (`ToBinary::ReadSpec { unit }`), the binary
calls `GET /units/{id}`, and the answer comes back on its own feed. The
panel fills in when it lands. The store's mirror is still read at boot, so a
launch with credentials on the command line shows the figures immediately,
and the API's answer is authoritative and overwrites it. A hull with no
published specification still says so.

**The type comes off the wire.** The register's own read (`GET /units`)
publishes each hull's class summary, and that summary carries the class's
TYPE (`unit_class.unit_type.name`) — the backend used to drop it, so the
console could only guess the type from the local taxonomy. The roster row
carries the type name now, and the panel reads it from there when the drill
tree does not know the unit.

**And the embark read is asked for — of every host.** The embark read is per
HOST: it answers "who is inside X". So the panel asks for every host in the
exercise, not only for the hull that was pointed at, and both directions
follow — "carrying N" is the hull's own read, "aboard X" is the read of the
host carrying it.

**"On the water" is a claim, and the claim waits.** A hull that is aboard a
carrier the console never read is not a hull on the water. The console says
"on the water" only once every host in the composition has been read, and
says a dash before that. A Game Master has no seat and commands no group, so
the commander's own read never runs for one: the panel's read is the panel's
claim.

## What is still owed

- The figures come from the API's CURRENT specification version, so a
  specification version pinned by a game being judged is not what the panel
  shows — and the panel does not say which version it read, because the
  dock has no room for it.
- The panel is a dock under the map, and the dock covers the column's
  bottom island. The column's scroll reaches it; the dock does not move.
