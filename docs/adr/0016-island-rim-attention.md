# ADR-0016: An island's rim is neutral at rest and lit only when it owns input

## Status

Accepted. Supersedes nothing; it carries out the rule extension
`docs/adr/0014-island-chrome.md` left as "a one-line change here", and it
extends the One Signal Rule in `DESIGN.md` rather than replacing it.

## Context

The console is being overhauled into three zones and the aesthetic register
was chosen as *tactical with heat*: a lit emissive rim on panels, which is
what separates an instrument from a web card.

The obvious implementation is to paint every island's rim in Radar Cyan. It
looks correct in isolation. It was rendered with two islands on screen and
it fails, for a reason that is only visible at the console's scale rather than
the panel's.

With a rim lit on every island, cyan is on every surface at once. An operator
reading "cyan" learns nothing, because cyan is the resting state of the
chrome. The One Signal Rule (`DESIGN.md`, Colors) exists so that one accent
can mean one thing; lighting every rim spends it on decoration and the signal
stops decoding. The accent is the scarcest resource in the system and this
was the most direct way to waste it.

## Considered Options

1. **Light every rim.** Rejected on the render above. Also leaves nothing for
   the accent to mean.
2. **No rim at all, back to a flat hairline.** This is the previous system and
   it is why the register feels like a web app: a hairline is a document
   edge, and a panel over a moving map needs to read as a physical surface.
3. **Neutral rim at rest, lit on the focused island.** One island is lit at a
   time, and it is the one under the pointer.

## Decision

An island's rim is **Cut Grey (#8C9BAE) at 34%** at rest, and **Radar Cyan
(#22D3EE) at 50%** on the island that currently owns input. Exactly one
island is lit at a time.

The rim is 2px, inset 1px from the island's own polygon outline, and it traces
the chamfer. That last part is why the register reads as *heat* rather than as
a highlighted border: the chamfer is the one edge an eye reads as a deliberate
cut, so lighting it is lighting the shape's most characteristic feature.

This is a rule extension with a reason, which is the condition
ADR-0014 set for touching the cut edge's colour.

## Consequences

- **The heat register has exactly one job: attention.** That is what it is
  for, and it is spent on the surface under the pointer. A console where
  everything glows tells the operator nothing about where they are.
- **It is a focus indicator that reads as hardware.** The previous system had
  no focus ring on a painted panel at all (ADR-0014, Consequences), which is
  why the Roster's title band was unreachable by keyboard. A rim that lights
  on the island under input is the first honest focus affordance the island
  chrome has had.
- **`hovered` is read from the previous frame's response**, as it already is
  for the cut edge in `chrome.rs`. The rim therefore lights one frame after
  the pointer arrives rather than flickering against the current frame's hit
  test. That is deliberate and it is why the transition is instantaneous
  rather than eased: an eased rim reads as a glow, and a glow on lag looks
  like a fault.
- **Keyboard focus is still unsolved** and this does not fix it. A chamfered
  island is a painted panel, not a `Widget`, so it cannot take keyboard focus
  the way a `Window` could. Lighting on the pointer is a pointer affordance.
  If an island ever needs focus-driven interaction, the next thing to build is
  a real `Widget` with `Sense::focusable`, and this ADR should be read as
  widening the gap rather than closing it.
- **Only one island lights**, so the implementation must resolve "which island
  owns input" before any rim is painted rather than each island deciding for
  itself. If two islands both test `hovered`, both light, and the rule is
  broken exactly when the operator is moving quickly.

## Notes

Verified by render, not by assertion. `proto/p5-epaint` gained a `--heat N`
mode and the rim's two states were captured and compared against a no-heat
control. The decision trail is `docs/decisions/ui-overhaul.tsv`.