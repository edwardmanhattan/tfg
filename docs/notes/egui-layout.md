# egui 0.36 layout capabilities (for #11)

Primary source: vendored `egui-0.36.2` crate source (exact version in `Cargo.lock`),
paths below relative to the crate root. Findings answer: what can fixed
multi-pane ops layouts do for the command center (map + roster + inspector +
orders/event-log panel)?

## Panels: left/right/top/bottom/center all exist

- One `Panel` type with constructors per edge — `Panel::left/right/top/bottom`
  (`src/containers/panel.rs:249-274`) — exposed as the familiar aliases
  `SidePanel::left(..)`, `TopBottomPanel::bottom(..)`, `CentralPanel::default()`.
  Caller-declared order decides layout: side/top/bottom panels claim their edge
  first, central takes the rest.
- Side panels are user-resizable (`Panel::resizable`, `panel.rs:322`) and
  skinnable per panel (`Panel::frame`, `panel.rs:413`); animated show/hide
  exists (`show_animated_inside`, `panel.rs:497`). So: roster left (resizable),
  orders + event log bottom (collapsible), inspector right or stacked under the
  roster — all without any docking framework.
- Panel contents are plain immediate-mode `Ui`, so the existing roster code
  moves verbatim into whichever pane owns it.

## Orders + event-log widgets are stock parts

- `ScrollArea::stick_to_bottom` (`src/containers/scroll_area.rs:655`) pins an
  event log to the newest line until the user scrolls up (then it unsticks;
  dragging back to the bottom re-sticks) — exactly ops-console behavior, no
  custom code. `show_rows` (`scroll_area.rs:983`) virtualizes long logs.
- Speed/heading entry: `DragValue` (`src/widgets/drag_value.rs:55`) and
  `Slider` (`src/widgets/slider.rs:98`) — drag-to-set numbers beat typing for
  helm orders. Buttons, checkboxes, `selectable_value` already in use.
- Read-only log text: `TextEdit::multiline` over a `String` buffer (or plain
  `Label`s inside the scroll area; labels are cheaper per frame for
  append-mostly logs).

## Marker interaction covers click now, drag later

- `Response::clicked / secondary_clicked` (`src/response.rs:184,211`) plus
  `interact_pointer_pos` (`response.rs:546`) — the current map-click select is
  the full extent of what's needed for selecting ships and picking waypoint
  targets (click ship → click map = ordered waypoint).
- Dragging is available when orders want it: `drag_started / dragged`
  (`response.rs:393,433`). Caveat: drag on the map image needs a deliberate
  `sense` (click-and-drag vs click-to-select disambiguation is our logic, not
  egui's).
- `Painter::arrow / line_segment / circle / text`
  (`src/painter.rs:417,318,341,469`) draw heading vectors, waypoint legs, and
  order badges as overlay shapes — same painter the markers already use.

## Repaint budgeting fits the ~10Hz overlay model

- `ctx.request_repaint_after(Duration)` (`src/context.rs:1872`) schedules the
  next frame during idle; the shell already uses 100ms. Overlays re-emit every
  frame but panels only re-layout changed regions — egui's tessellation is the
  per-frame cost, and at our shape counts (tens of markers, one log panel)
  that's noise next to the map thread.
- Rule of thumb from the docs on `request_repaint_after`: the duration starts
  at the *next* frame, so cadence is "at least every N ms while idle", not a
  hard timer — fine for marker glide (wall-clock `blend` already absorbs
  jitter), and orders/event-log updates ride free on the same repaints.

## Bottom line for the layout build

Fixed panes (left roster, right inspector, bottom orders + log, central map)
need no new dependencies and no custom layout code: panels + scroll areas +
stock widgets cover it. The only custom geometry stays in our overlay seam
(`project_mercator` + `hit_test`), extended with waypoint-leg shapes.
