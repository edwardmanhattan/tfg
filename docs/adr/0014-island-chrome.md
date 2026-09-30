# ADR-0014: Islands are an owned `Area`, not `egui::Window`

## Status

Accepted. Applies to the Roster island. The other five islands still run on
`egui::Window` and are expected to migrate once this has been lived with.

## Context

DESIGN.md now specifies a chamfered island: a 20px cut on the top-right
corner, stroked 2px. The cut is not decoration — an angled edge is localised
faster than a fade, because an eye can track an edge and cannot track a
dissolve, and the operator's attention is on the map while islands sit
peripheral.

egui cannot draw it. `egui::Frame` carries `fill`, `stroke`, `corner_radius`,
`inner_margin`, `outer_margin` and `shadow` — a rounded rect and nothing else.
`egui::Window::frame()` accepts a `Frame`, and there is no custom-shape hook
anywhere on `Window`. So a chamfered body is impossible on a native `Window`,
full stop. The prototype confirmed the geometry is otherwise trivial:
`Shape::convex_polygon` over five points, painted *around* an axis-aligned
content rect, which is why the cut costs paint and not layout.

That leaves three options, and two of them are bad:

1. **Keep the Window, notch the body.** Impossible, per above.
2. **Keep the Window at `corner_radius` 0 and paint a body-coloured triangle
   over the corner.** The body hides the artifact but the *shadow* still
   shows the rounded silhouette, and it looks wrong exactly where the eye
   goes.
3. **Own the container.** Build the island on `egui::Area` and write the
   chrome: title band, drag, close, constraining, fixed footprint.

## Decision

Islands are an owned `Area` — `tfg::chrome::island` / `island_scrolled` — not
`egui::Window`.

`Area` supplies position memory, viewport constraining, layer ordering and
sizing, so what is actually written by hand is the title band, the drag
region, the close button, and the footprint. The previous frame's drag is
read with `Context::read_response` and applied *before* the area opens, the
same ordering `egui::Window` uses internally, so a drag does not lag a frame.

## Consequences

- **The silhouette is ours.** The body is a polygon, so the cut can be a
  deliberate 2px edge rather than a radius compromise.
- **The footprint is fixed.** An island does not resize to fit its content;
  the body scrolls. This is a behaviour change for the Roster: its marker
  list no longer carries its own 300px `ScrollArea` cap, and the Legend and
  Keys sections scroll with it instead of extending the window. The old cap
  existed to stop a `Window` growing; a fixed island does not grow.
- **We own the close button and the drag region.** Adding an island is cheap.
  Changing how islands behave is ours to maintain.
- **Keyboard focus is unsolved.** A chamfered island is a painted panel, not a
  focusable `Widget`, so it cannot take keyboard focus the way a `Window`
  could. Roster's content is reachable (it is a list of rows inside a `Ui`),
  but the *title band* is not focusable and there is no focus ring. If the
  other five islands migrate and any of them turns out to need focus-driven
  interaction, that is the thing to build next — a real `Widget` with
  `Sense::focusable`, not a painted panel.
- **Hit testing is rectangle-based.** egui hit-tests the allocated rect, not
  the painted polygon, so the chamfer's cut corner is clickable-but-empty.
  For the Roster this is 20px of dead triangle in a corner that holds nothing.
  It is not worth a vertex test yet; if an island ever needs its corner to be
  inert, the app already has a point-in-polygon test to reuse
  (`src/overlay.rs` over `convex_hull`).
- **A sheared panel was rejected.** A shear insets the top-left corner by the
  lean, so every fixed content inset pokes through the diagonal and the
  maintenance cost lands on each widget rather than once. The chamfer is the
  whole shape language. See DESIGN.md, Shapes.
- **The cut edge is a cool grey, not Radar Cyan.** DESIGN.md reserves cyan
  for live state. A cyan cut would spend the one accent that carries meaning
  on decoration, in a console where "is this live?" must decode at a glance.
  Promoting it is a one-constant change and is only defensible if the cut
  means "this surface owns input" and is lit only on the focused island.

## Notes

`src/chrome.rs` has no tfg types, so it is type-checked and screenshot-verified
from `proto/p5-epaint` (`--island`), which compiles the real file via
`#[path]`. tfg itself is not built by that harness — it pulls the MapLibre C++
core, and `AGENTS.md` asks that it not be compiled on the user's behalf.
