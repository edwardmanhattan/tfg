# ADR-0023: The map turns the staged course

The Game Master could put a hull on the water while planning, and then the
map had nothing more to say about it: the hull could be pointed at, but not
moved, and nothing on the map could say which way it pointed. The egui
console had both — a body that dragged the hull, and a compass drawn around
the selected thumbnail whose needle was dragged to set the heading.

## Status

Accepted. Carries forward ADR-0017 (the force draft is local and written
through at an advance) and ADR-0021 (the mode decides the verbs).

## Context

MinOS's placement write (`PUT /games/{id}/units/{unit_id}/placement`) takes
a latitude and a longitude and nothing else. There is no endpoint for a
hull's starting COURSE, and the helm order
(`POST /games/{id}/units/{unit_id}/order`) is refused with a 409 outside
execution — and is refused with a 403 for a hull the caller does not
command, which is most of them in a planning room, where the Game Master
commands no ship at all.

So a course the Game Master states while planning has nowhere to go: it is
either a value the console keeps for itself, or it is posted later as
something else.

The egui console's answer was the honest-looking one and was not: it drew a
needle over the thumbnail and wrote the result into a helm draft, which
looked like an order and was not one until the operator pressed SET HELM.

The GPUI cutover brought neither affordance. The map's press picks a unit
and opens the Inspector; the map is the PAN handle everywhere; and the only
compass in the console is in the commander's rail, in execution, on a hull
the operator commands.

## Decision

**The map is where a staged hull is MOVED and TURNED, and the map says which
hull the hand is on.**

- **A press on the map answers three questions, in order.** What is under
  the pointer (the hover ink), what the hand is holding (the press ink), and
  what the gesture does: the rotator's knob turns a course, a hull's body
  moves the hull, and anything else pans the chart. The knob is tested
  FIRST, because the knob stands clear of the hull's own body and a press on
  the handle is not a press on the hull.
- **A course is a staged value in the Force draft** (`DraftHull.course_deg`),
  and it is deliberately NOT part of the diff `force::plan` takes. A course
  changed and a placement changed are different edits, and the plan's writes
  are the ones the server accepts. The draft's merge keeps it: an entry that
  no longer matches the frame is the operator's, and the server holds no
  course to match.
- **A staged course goes out as a helm order at the start of play** — the
  advance into execution, which is the first moment the server will take
  one. It is posted for the hulls the OPERATOR'S OWN SEAT commands, with the
  hull's published speed, because a helm order is a heading AND a speed and
  a speed of zero is Hold position: it keeps the last accepted heading
  rather than taking the one asked for. A hull the operator does not command
  stays in the plan, and the console NAMES it: a course the server will not
  take is a plan, and a plan the operator cannot see is a plan they will
  believe is an order.
- **The rotator is the console's own compass**, at the scale of a map mark:
  the same rim, the same north wedge, the same needle, the same knob at the
  needle's tip as the rail's dial. An operator who has learned to read one
  can read the other without being told which is which. No needle means no
  course: an unknown heading is not north, and a needle pointing up would say
  it was.
- **A drag on a staged hull is a LOCAL act** while planning, like every
  other draft edit (ADR-0017): the hull's staged position moves in the
  draft and nothing is posted. In a static exercise the same gesture
  previews the position to the other boards and commits it on the release,
  which is where a position is set by hand (ADR-0021).
- **A drag on a hull keeps the selection; a pan undoes it.** The hull is the
  subject of the gesture, and a drag that turns out to be a pan of the chart
  must not change what the operator has selected.
- **The map's pointer claims are the shell's own geometry.** The marks are
  painted into one canvas, so there are no per-hull elements to hover: the
  console answers "what is under the pointer" with the same hit test the
  painter used, and the press ink, the hover ink and the rotator all read
  that one answer.

## Consequences

- The Force draft carries a value Minos cannot hold, and the panel says so:
  a staged position and a staged course are marked "(staged)", because a
  plan the panel reads as a fact is a plan the operator will believe the
  exercise carries.
- A course that cannot be posted — a hull nobody commands, a mode that
  takes no helm orders — is named rather than dropped, and the sentence that
  says it is the console's own.
- The map is no longer only a pan handle: a press on a hull starts a drag
  of that hull, so a chart cannot be panned by starting the drag on a
  hull. It is panned from the water, which is where the chart is.
- GPUI 0.2.2 cannot rotate a picture, so the hull's own image is still
  drawn on the asset's axis and the NEEDLE carries the course. This is the
  same limitation the egui console's thumbnails had, and the reason the
  needle is an instrument and not a decoration.
- A capture cannot photograph a pointer claim without a live gesture, so
  there are three seeds: `TFG_MAP_HOVER` names the hull the pointer is over,
  `TFG_MAP_PRESS` the hull it is holding, and `TFG_COURSE` ("<unit
  id>:<degrees>") stages a course. The same precedent as `TFG_POINTER` and
  `TFG_SEL`: pin the state, never the gesture.

## Verified

Against the deployed dev API, 2026-10-21, on a planning exercise with one
hull placed:

- The staged position is in the draft and in the Inspector: "POSITION
  -6.0800, 106.9000 (staged)".
- The rotator is drawn around the selected staged hull, with the north
  wedge, the cardinal graduations, the needle and the knob at its tip; the
  knob sits at the rim's top when no course is staged, because the knob is
  the handle and a handle nobody can see is a handle nobody grabs.
- The hover ink is a dim ring on the hull under the pointer, the press ink
  a bright one on the hull the hand is holding, and the rotator's rim
  stands clear of both, so the three never read as one thing said twice.
