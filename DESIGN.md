---
name: Tactical Floor Game
description: Night ops console for a live wargaming floor game — navy chrome over dark tiles, one radar-cyan signal.
colors:
  radar-cyan: "#22D3EE"
  console-night: "#0F172A"
  deep-well: "#020617"
  panel-slate: "#1E293B"
  hairline-slate: "#334155"
  map-ink: "#E2E8F0"
  body-silver: "#8C8C8C"
  button-graphite: "#3C3C3C"
  button-label: "#B4B4B4"
  status-success: "#4ADE80"
  status-warning: "#FABF69"
  status-error: "#F87171"
  status-idle: "#808080"
  data-amber: "#F59E0B"
  alert-yellow: "#FFFF00"
  rank-unsur: "#16A34A"
  rank-satgas: "#2563EB"
  rank-gugus: "#9333EA"
  rank-opsgab: "#EA580C"
typography:
  heading:
    fontFamily: "Ubuntu Light, system-ui, sans-serif"
    fontSize: "18px"
    fontWeight: 300
  body:
    fontFamily: "Ubuntu Light, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 300
  button:
    fontFamily: "Ubuntu Light, system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 300
  small:
    fontFamily: "Ubuntu Light, system-ui, sans-serif"
    fontSize: "9px"
    fontWeight: 300
  map-label:
    fontFamily: "Ubuntu Light, system-ui, sans-serif"
    fontSize: "12px"
    fontWeight: 300
  mono:
    fontFamily: "Hack, Ubuntu Mono, monospace"
    fontSize: "13px"
    fontWeight: 400
rounded:
  island: "8px"
  control: "6px"
spacing:
  sm: "6px"
  md: "8px"
  lg: "10px"
  xl: "20px"
components:
  island:
    backgroundColor: "{colors.console-night}"
    textColor: "{colors.body-silver}"
    typography: "{typography.body}"
    rounded: "{rounded.island}"
    padding: "6px"
  button:
    backgroundColor: "{colors.button-graphite}"
    textColor: "{colors.button-label}"
    typography: "{typography.button}"
    rounded: "{rounded.control}"
    padding: "6px 10px"
  button-hover:
    backgroundColor: "rgba(34, 211, 238, 0.16)"
    textColor: "#F0F0F0"
    rounded: "{rounded.control}"
    padding: "6px 10px"
  button-active:
    backgroundColor: "rgba(34, 211, 238, 0.27)"
    textColor: "#FFFFFF"
    rounded: "{rounded.control}"
    padding: "6px 10px"
  button-selected:
    backgroundColor: "{colors.radar-cyan}"
    textColor: "{colors.deep-well}"
    typography: "{typography.button}"
    rounded: "{rounded.control}"
    padding: "6px 10px"
  text-field:
    backgroundColor: "{colors.deep-well}"
    textColor: "{colors.body-silver}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "6px 10px"
    width: "280px"
  toolbar:
    backgroundColor: "{colors.console-night}"
    textColor: "{colors.body-silver}"
    typography: "{typography.body}"
---

# Design System: Tactical Floor Game

## Overview

**Creative North Star: "The Night Ops Console"**

Calm, exact, mission-grade. The app is a dark navy instrument console laid
over a night tile map: the map owns the whole window, the chrome is a thin
band at the top plus movable islands floating over it, and every pixel of
ornament that isn't carrying state has been taken out. The reference feeling
is a darkened exercise room — legible under glare on a field laptop and
equally legible on a desk monitor at 02:00 — not a consumer dashboard and
definitely not a neon HUD.

Density is deliberate. This is an Operate surface: an organizer or commander
scans rows, toggles islands, and reads a clock under time pressure, so the
layout favours compact rows, hairline separation and short labels over airy
cards. Expression lives in a very small number of precise details — the
single cyan state signal, the ringed map symbology, the two-font split —
and nowhere else. Confirmed visual anti-reference: decorative cyberpunk /
gamer-chrome glow, and friendly rounded SaaS card stacks.

The map is the room's only light source. Everything else is a navy panel
that stays quiet until it has something to say, and when it does it says it
in colored ink (green/amber/red/yellow) rather than in motion, badges, or
animation. The product's own doctrine — *present, never predict* — is the
aesthetic doctrine too: the interface reports accepted state and does not
perform.

**Key Characteristics:**
- One radar-cyan signal on a navy console; no second chrome accent.
- Depth is tonal (chrome → slate → well) plus a 1px hairline, not shadow.
- Ubuntu Light for people, Hack for machines; two type roles, no more.
- Circles are map objects, rectangles are controls — never the reverse.
- Status is always colored ink; idle is the only gray state.

## Colors

The palette is a dark, low-chroma console field with one high-chroma signal
and a small set of strictly-scoped semantic hues.

### Primary
- **Radar Cyan** (#22D3EE): the only accent. It marks live state on chrome —
  text selection, hyperlink text, the hovered and pressed wash on any
  button, and the solid fill of a toolbar toggle that is switched ON. Its
  rarity is what makes an ON state readable across a room.

### Secondary
The group-rank palette is a categorical scale, not decoration. It is used
for the hull zones, centroid flags and per-rank identity drawn on the map
(and, at the green end, for per-unit identity), and it is barred from chrome.
- **Unsur Green** (#16A34A): rank 1 zone fill (at 70/255 alpha) and stroke;
  also the default identity color for units without a named hull.
- **Satuan Tugas Blue** (#2563EB): rank 2 zone fill and stroke; also the
  named identity of the `nordwind` unit.
- **Gugus Purple** (#9333EA): rank 3 zone fill and stroke.
- **Operasi Gabungan Orange** (#EA580C): rank 4 (highest) zone fill and
  stroke. Zones paint highest rank first so lower ranks layer over them.

### Status & Signal
Semantic ink for state readouts; every status line routes through one
function so state is never gray-on-gray.
- **Signal Green** (#4ADE80): success and connected — "connected", "synced",
  "signed in as", "issued", "redeemed".
- **Warning Sand** (#FABF69): transitional or degraded-but-not-failed —
  "connecting".
- **Fault Red** (#F87171): failure — "fail", "refused", "error",
  "unreachable", "socket error", "sync stopped".
- **Idle Gray** (#808080): neutral/idle status and stale units.
- **Data Amber** (#F59E0B): age-of-data marker — the ring around a unit
  whose fix is old, and the hollow ghost of a replayed unit. Data age, never
  feed state.
- **Alert Yellow** (#FFFF00): attention without fault — `⚠` warning lines,
  the strong `PAUSED` clock label, and the ring around the unit currently
  being followed.

### Neutral
The Console Night family carries all chrome; every step is a depth, not a hue.
- **Console Night** (#0F172A): window fill and panel fill — the chrome every
  island and the top toolbar are made of.
- **Panel Slate** (#1E293B): faint background — the resting tone for
  secondary/inset surfaces inside chrome.
- **Deep Well** (#020617): extreme background — text-field wells, the
  selection text color, and the near-black the map sits against.
- **Hairline Slate** (#334155): every window/panel stroke (1px) — the single
  edge treatment in the system.
- **Body Silver** (#8C8C8C): default label and body text on chrome.
- **Button Label** (#B4B4B4): text on a resting button; lifts to #F0F0F0 on
  hover and white when pressed.
- **Button Graphite** (#3C3C3C): resting button fill — deliberately dull so
  the cyan wash on interaction reads as the event.
- **Map Ink** (#E2E8F0): the only text drawn on tiles — unit ids, flag
  labels, replay ghosts.

### Named Rules
**The One Signal Rule.** Radar Cyan is reserved for live chrome state
(selection, link, hover/press wash, toggle ON). It is never a border, never a
heading, never a background field, and it never appears twice as decoration.
**The Rank Rule.** Green → Blue → Purple → Orange encodes group rank and
nothing else. A rank color on a button, a label, or a panel is a bug.
**The Never Gray-On-Gray Rule.** Any line reporting state goes through the
status ink function. If a message could read as success, failure or idle, its
color must say which.

## Typography

**Display Font:** none — the system has no display role by design.
**Body Font:** Ubuntu Light, with system-ui and sans-serif fallback (egui's
stock proportional family).
**Label/Mono Font:** Hack, with Ubuntu Mono and monospace fallback (egui's
stock monospace family).

**Character:** A single light humanist sans doing all the human work, paired
with a rigid monospace that is only ever allowed to show machine output. The
lightness is what keeps a dense console readable; emphasis comes from color
and position, not from weight.

### Hierarchy
- **Heading** (300, 18px, default leading): island titles — "Session",
  "Roster", "Orders", "Command center setup". One per island, never scaled up.
- **Body** (300, 13px, default leading): labels, rows, values, help lines.
  The workhorse; it sits in the spacing rhythm rather than in a measure —
  islands are capped at 420px of scroll, so long lines wrap inside the island.
- **Button** (300, 13px, default leading): button, toggle and selectable
  labels. Sentence case, lowercase verbs ("connect live", "release").
- **Label / Small** (300, 9px): the `small_button` zoom steppers and other
  tightly-packed chrome affordances.
- **Map Label** (300, 12px): unit ids and flag labels painted directly on
  tiles, always in Map Ink.
- **Mono** (400, 13px): the event feed and session-log lines only — machine
  text, in a machine face.

Emphasis is `.strong()` and it is spent almost nowhere (the `PAUSED` clock
label is the canonical use); `.weak()` renders the same body color at 60%
alpha for empty-state hints.

### Named Rules
**The Two-Face Rule.** If a human wrote it, it's Ubuntu Light. If a machine
produced it, it's Hack. No third family, and no monospace used for effect.

## Layout

There is no grid and no breakpoint set: this is a native desktop window, so
the spatial model is a **stage**, not a page.

- **Top toolbar** — a full-width top panel in Console Night holding two
  horizontal rows: row 1 is mode select, island toggles, and the `− z +`
  zoom stepper; row 2 (Simulation only) is the `act` identity combobox, the
  numbered desktop tabs (`[1] … [9]`), and the clock block (UTC line, then
  the `GAME … · G+mm:ss (n×)` line with `PAUSED` beside it when frozen). The
  toolbar has no corner radius — it is the frame, not a card.
- **Map stage** — a central panel with no frame and no margin, so the tile
  canvas owns every point the panel offers. UI never insets the map.
- **Islands** — floating, movable windows (Session, Users, Roster, Fleet,
  Groups, Orders, Inspector, Log, Connection, Login, Wizard) positioned on a
  loose absolute grid of defaults (8/560/816px across, 64/140/300/478px
  down). Islands are resizable where their content warrants it; the wizard
  refuses to collapse below its 460px minimum and Fleet below 760px.
- **Scroll discipline** — island bodies scroll at a 420px cap so a tall
  island never swallows the map; the Roster and Fleet hull lists get their
  own 300px cap with their headers pinned outside it.
- **Miller columns** — the Fleet picker's taxonomy column is fixed at
  176 × 170px so the column set stays aligned while browsing.
- **Spacing rhythm** — item spacing 10 × 8px (horizontal × vertical), button
  padding 10 × 6px, indent 20px, island inner margin 6px. Rows are built from
  horizontal groups separated by 1px dividers rather than by large gaps.
- **Adaptation** — instead of reflowing, the operator rearranges: islands
  drag, desktops swap on number keys 1–9, and the camera stays put. This is
  the answer to varied screen sizes and glare — fewer, movable, dense surfaces
  rather than stacked responsive cards.

## Elevation & Depth

Layered, not lifted. Depth is a tonal ladder — Console Night chrome sits on
Panel Slate insets, which sit over the Deep Well the map is drawn against —
plus a 1px Hairline Slate stroke on every window and panel edge. The theme
does not author its own shadow vocabulary; it inherits egui's stock shadows,
which stay small and dark and are never the thing that communicates depth.

### Shadow Vocabulary
- **Island cast** (`box-shadow: 10px 20px 15px rgba(0,0,0,0.376)`): the
  default window shadow behind every floating island. Present for separation
  from the moving map, not for drama.
- **Popup cast** (`box-shadow: 6px 10px 8px rgba(0,0,0,0.376)`): combobox
  and menu popups only — one step tighter than an island.
- **No shadow on rules.** Separators and indent lines are a 1px #3C3C3C
  stroke with zero elevation; markers on the map carry depth with a 2px white
  ring, never a glow.

### Named Rules
**The Hairline Rule.** Every panel edge is exactly 1px of Hairline Slate.
If a surface needs to feel raised, it steps a tone lighter — it does not get
a bigger shadow, a glow, or a second border.

## Shapes

Two shapes, two meanings. Rectangles are controls and containers; circles are
objects on the map. Nothing mixes them.

- **Islands and windows** — gently curved corners at an 8px radius; menus
  match at 6px.
- **Controls** — buttons, toggles, checkboxes, selects and text fields all
  sit at a 6px radius. The radius is uniform across the whole widget family,
  so a control is recognisable by silhouette alone at a glance.
- **Non-interactive chrome** — dividers, toolbar and unrounded panels stay
  square (2px where egui rounds a frame internally). Corner radius signals
  "you can touch this".
- **Map symbology is pure circle geometry**: an 8px filled unit marker with
  a 2px white ring; a 12px ring for focus states (yellow = followed,
  light blue = selected, amber = old data); a 10px flag disc with a 2px white
  ring; a 14px ring on zone vertices over a 10px zone spine; 2px trail dots
  at 55% of the unit color.
- **Zones** are translucent fills (70/255 alpha) with an opaque 2px stroke of
  the same rank color — outline is the shape, fill is only a hint.

## Components

### Buttons
Plain, compact and instrument-like: a dull graphite pill that only wakes up
when touched.
- **Shape:** gently curved (6px radius), padding 10px horizontal × 6px
  vertical, sentence-case labels.
- **Primary:** there is no filled brand button — importance is expressed by
  position and by the ON state, never by a loud resting color. Resting fill
  is Button Graphite with Button Label text.
- **Hover / Focus:** fill becomes a Radar Cyan wash at 16% alpha, text lifts
  to #F0F0F0, stroke 1.5px.
- **Active / Pressed:** Radar Cyan wash at 27% alpha, text white, stroke 2px.
- **Disabled:** rendered at 50% alpha (egui's disabled alpha) — greyed in
  place, never hidden.
- **Small:** the `−` / `+` zoom steppers use the 9px label style inside the
  same 6px control shape.

### Toggles & Selectables
- **Style:** the same 6px control shape as buttons; used for the toolbar
  island switches, desktop tabs, mode select (`📡 Presentation` /
  `🎮 Simulation`), the Presentation/Simulation pair in the wizard, and every
  roster row.
- **State:** OFF is Button Graphite; ON is **solid Radar Cyan with Deep Well
  text** — the highest-contrast pairing in the system, which is why an active
  island is findable from across the room. Hover/press use the same cyan wash
  as buttons.
- Selected rows in ComboBoxes use the same cyan-fill rule.

### Cards / Containers (Islands)
- **Character:** quiet navy panes that float over the map and never compete
  with it.
- **Corner Style:** 8px radius.
- **Background:** Console Night; inset/secondary areas step to Panel Slate;
  input wells step to Deep Well.
- **Border:** 1px Hairline Slate on every side — the only edge treatment.
- **Shadow Strategy:** stock island cast (see Elevation & Depth); the hairline
  does the real work.
- **Internal Padding:** 6px island margin; rows separated by 10 × 8px; body
  content scrolls at a 420px cap.
- **Title:** one 18px Heading, sentence case, no eyebrow or kicker line.

### Inputs / Fields
- **Style:** Deep Well background (darker than the island, so a field reads
  as cut into the panel), Body Silver text, 6px radius, 6px × 10px padding,
  280px default width; password fields mask in place.
- **Focus:** the cyan signal — cursor and selection fill are Radar Cyan, and
  the selection text is Deep Well.
- **Error / Disabled:** errors never live inside the field; they print as a
  Fault Red status line beneath the control, with the recovery named on the
  adjacent control. Disabled controls drop to 50% alpha.

### Navigation
- **Toolbar:** full-width, unrounded Console Night band, two rows of
  horizontal groups split by 1px dividers. Island toggles first, zoom and
  clock last — controls the operator touches constantly sit leftmost.
- **Desktop tabs:** selectable labels numbered `[1] … [9]`, mirroring the
  number-key shortcuts; ON = cyan fill. The camera never moves when a tab
  switches.
- **Wizard:** a non-collapsible 460px-min island with a heading, one line of
  guidance, and a `← Back` / `Next →` footer row separated by a divider.
- **Mobile treatment:** none — this is a desktop window; adaptation is
  dragging islands and swapping desktops, not reflowing.

### Signature Components
- **Status line** — one row of state ink (green / sand / red / gray) driven by
  message content, so every backend message lands in a predictable color.
  This is the app's most-used component and it is deliberately text-only.
- **Unit marker** — filled circle + 2px white ring + 12px focus ring + a 12px
  offset Map Ink label. Trails are 2px dots of the same color at 55% alpha.
- **Zone hull / centroid flag** — a translucent rank-colored polygon with an
  opaque rank stroke, and a flag disc carrying lat/lon, drawn only at or
  above zoom 11.

## Do's and Don'ts

### Do:
- **Do** route every state-bearing message through the status ink function
  (green / sand / red / gray) before it reaches a label.
- **Do** keep Radar Cyan to live state: selection, links, hover/press wash,
  and toggle ON. Solid cyan with Deep Well text is the ON treatment.
- **Do** separate panels with a 1px Hairline Slate stroke and step a tone for
  depth (Console Night → Panel Slate → Deep Well).
- **Do** use the 8px island / 6px control radius pair exactly; 8px means
  container, 6px means control.
- **Do** put machine output — event feed, session logs, ids — in Hack, and
  cap island body scroll at 420px (300px for the Roster and Fleet lists).
- **Do** draw map objects as circles with 2px rings and label them in Map Ink
  at 12px; keep zone fills at 70/255 alpha under an opaque 2px rank stroke.
- **Do** keep the map canvas frameless and full-window — UI floats over it,
  never insets it.

### Don't:
- **Don't** introduce a second chrome accent, a gradient, or a glow; the
  palette is one cyan plus navy neutrals plus scoped semantic hues.
- **Don't** put a rank color (green/blue/purple/orange) on chrome — it is
  reserved for group rank on the map.
- **Don't** ship gray-on-gray status, and don't animate to announce state —
  color says it first.
- **Don't** add a third typeface, bold body text for emphasis, or monospace
  for anything a human wrote.
- **Don't** reach for a bigger shadow when a surface needs presence; use the
  hairline and a tonal step.
- **Don't** round the toolbar, and don't add a filled "brand" primary button —
  resting controls stay graphite.
- **Don't** invent breakpoints or responsive stacking: this is a desktop
  window with movable islands.
