---
name: Tactical Floor Game
description: Night ops console for a live wargaming floor game — navy chrome floating over dark tiles, one radar-cyan signal, and a lit rim that says which surface owns input.
colors:
  radar-cyan: "#22D3EE"
  console-night: "#0F172A"
  deep-well: "#020617"
  panel-slate: "#1E293B"
  hairline-slate: "#334155"
  cut-grey: "#8C9BAE"
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
shapes:
  island-chamfer: "20px top-right"
  control-radius: "6px"
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
    padding: "16px"
  seam:
    restColor: "{colors.cut-grey}"
    restAlpha: 1.0
    activeColor: "{colors.radar-cyan}"
    activeAlpha: 0.5
    width: "3px"
    inset: "1px"
    outerBorder: "2px {colors.cut-grey}"
  band-scanline:
    pitch: "4px"
    alpha: 0.067
    scope: "title band only"
  button:
    backgroundColor: "{colors.button-graphite}"
    textColor: "{colors.button-label}"
    typography: "{typography.button}"
    rounded: "{shapes.control-radius}"
    padding: "8px 12px"
    border: "1px {colors.hairline-slate}"
  button-hover:
    backgroundColor: "rgba(34, 211, 238, 0.16)"
    textColor: "#F0F0F0"
    rounded: "{shapes.control-radius}"
    padding: "8px 12px"
    border: "1px {colors.cut-grey}"
  button-selected:
    backgroundColor: "{colors.radar-cyan}"
    textColor: "{colors.deep-well}"
    typography: "{typography.button}"
    rounded: "{shapes.control-radius}"
    padding: "8px 12px"
  text-field:
    backgroundColor: "{colors.deep-well}"
    textColor: "{colors.body-silver}"
    typography: "{typography.body}"
    rounded: "{shapes.control-radius}"
    padding: "8px 12px"
    border: "1px {colors.hairline-slate}"
    width: "280px"
  top-zone:
    backgroundColor: "{colors.console-night}"
    textColor: "{colors.body-silver}"
  side-zone:
    backgroundColor: "{colors.console-night}"
    textColor: "{colors.body-silver}"
---

# Design System: Tactical Floor Game

## Overview

**Creative North Star: "The Lit Console"**

Calm, exact, mission-grade. A dark navy instrument console floating over a
night tile map: the map owns the whole window, the chrome is a thin band at
the top plus a column of islands down one side, and the one piece of
ornament in the system is a lit rim on the island you are touching.

The register is *tactical with heat*. Sharp chamfers and machined edges
read as built equipment rather than as web cards, and a surface carries a
faint emissive rim so an operator can see at a glance which panel owns the
pointer and which one does not. What the heat is spent on is the one channel
it can pay for: **attention**. Everything else stays off.

The map is the room's only light source. Every panel is navy and quiet until
it has something to say, and when it does it says it in colored ink rather
than in motion. The product's doctrine is *present, never predict*, and the
aesthetic doctrine is the same: the interface reports accepted state and does
not perform.

**Key Characteristics:**
- One radar-cyan signal, and it means "live or yours". Nothing else is cyan.
- The heat lives on the island rim and the title band, never across a body.
- Depth is tonal (chrome → slate → well) plus a bold cut-grey border: a 2px
  outer edge and a 3px inset rim.
- Ubuntu Light for people, Hack for machines; two type roles, no more.
- Circles are map objects, rectangles are controls. The chamfer means a
  surface that moves.

## The three zones

The window is divided into three zones and nothing else. There is no fourth
place for a panel to live, which is what keeps the console readable.

### Map stage — the background

The tile canvas owns every pixel the panel layout offers. UI never insets
it, and a zone never takes space from it: a docked zone floats over the map
and the map's texture stays full-window beneath.

The map is painted in egui's `Background` layer. Zones and islands paint in
`Order::Middle`, above it, and they are `interactable`, so a zone steals the
pointer from the map and a drag that starts over chrome is not a camera pan.

**The one consequence, stated because it is easy to miss.** Because the side
zone overlays rather than insets, the window's centre is not the map's
*visible* centre. Any camera action that centres a hull must offset its goal
by half the zone's width toward the open side, or the unit parks under the
chrome. This is a rule about camera goals, not a layout fix.

### Top zone — a fixed band

A full-width unrounded band in Console Night, one row, holding the things an
operator needs without hunting: connection state, how many are online, the
clock block, and logout at the right.

Groups are split by separators, not gaps. Actions are all small buttons at
one height; secondary readouts (headcount, zoom) are small weak mono, so the
link state and the clock stay the two things the band reads at a glance.

It is the frame, not a card, so it takes no corner radius. It floats over the
map like everything else; it never pushes the map down.

**The mode toggle is not here.** Presentation and Simulation is a startup
decision, not an operating control, so it moved out of the toolbar and into
the settings modal. Simulation mode does not announce itself: the console
simply *is* the simulation, and the only hint is a word in the zone.

### Side zone — a column of islands

The side zone is where the operator works. It is a vertical column of
islands down the left or right edge, toggled as a unit, and each island
inside it carries one job.

The zone **floats over the map**. This is deliberate and it is the opposite
of what a dock normally does, so the reason is stated: an operator reads the
map, and a map that resizes as a panel opens reflows the thing they are
reading. The camera's visible centre moves instead, by the offset rule above.

The zone docks **left or right** and remembers which. A commander who steers
with the right hand and an organiser scanning rows with the left should not
have to argue about it. Its inner edge drags to resize (240–600px); the
column, the band and the camera offset all read the live width, and Settings
holds a slider twin plus the reset to default.

### Modal zone — above everything

Modal surfaces sit in `Order::Top`, centred, and they dim what is behind them
so a drag cannot escape onto the map. Three exist: Scenario Composer, Fleet
Picker, and Player Picker. Modals are for authoring; islands are for
operating. A form with fields in it belongs in a modal, not in a column.

## The five game states

The side zone's contents are a function of one value: where the exercise is.
There are five, and the zone is never empty.

| State | Source | The side zone holds |
| --- | --- | --- |
| **No session** | no held game | The user island, then one island whose only verb is *start a new session*. |
| **Planning** | `planning` | Essentials, Control, Fleet, Players, Review. |
| **Preparation** | `preparation` | Readiness: who is in, what is placed, what is outstanding. |
| **Execution** | `execution` | Orders, Roster, Log. No setup, no pickers. |
| **Closure** | `closure` | The assessment surface: summary, timeline, judgements. |

Two rules make this a spine rather than a pile of conditionals.

**The user island is always first.** Identity does not change with the state,
so the one thing that does not move is pinned at the top of the column. It
carries the avatar and the app role from the sign-in, and it is where logout
lives when the top zone is collapsed.

**A state never shows a verb that the backend would refuse.** `static` mode
refuses ordering, so an Execution zone in a static exercise has no Orders
island. Planning mode offers the Scenario Composer only when the mode is
`scenario`. Hiding a verb is better than showing one that 409s.

### Planning, in order

Planning is the only state with a defined island order, and the order is the
order of dependencies, not the order of importance.

1. **Essentials.** Name, description, the planned real and assumed windows,
   and the mode. Mode is fixed at creation and cannot be revised, so once a
   session exists the control is read-only rather than hidden.
2. **Control.** The time factor. The only field in Planning that stays live
   after the exercise begins.
3. **Fleet.** What is deployed, with a button that opens the Fleet Picker.
4. **Players.** Who is in, with a button that opens the Player Picker.
5. **Review.** The gate checklist and the advance into preparation. Its own
   island: sharing Players buried the gate below the roster.

## Islands

An island is a floating panel with a fixed footprint, a chamfered
top-right corner, and a lit rim.

- **Silhouette.** The top-right corner is chamfered 20px and the cut is
  stroked 2px in Cut Grey. Every other corner is square. It is a polygon
  painted around an axis-aligned content rect, so the cut costs paint and not
  layout, which is why an island's contents never have to know about it.
- **Fixed footprint.** An island does not resize to fit its content. The body
  scrolls. A panel that grows under the cursor while someone is reading a map
  is worse than one that scrolls, and a stable silhouette is what makes the
  chamfer read as a shape.
- **Title band.** 30px, Panel Slate, carrying one tracked monospace label in
  short caps and, right-aligned, a count or a status. This is the band's
  whole job. It is also where the only permitted texture lives.
- **Body.** Console Night, scrolling, at a 420px cap.

### The rim, and why it is neutral at rest

This is the system's load-bearing decision and it was made by rendering, so
the reasoning is recorded rather than asserted.

An earlier pass lit **every** island's rim in Radar Cyan at rest. Rendered on
its own it looked good. Rendered as a console it failed, for a reason that is
only visible once more than one island is on screen: the accent had been
spent on every surface at once, so it stopped being a signal. An operator
reading "cyan" learned nothing, because cyan was everywhere.

So the rim has two states, and this is the rule extension ADR-0014
anticipated and left as a one-line change:

- **At rest**, the rim is Cut Grey at full opacity plus a 2px cut-grey outer
  border, so stacked islands hold a clear edge over a bright tile map. It
  separates the panel from the moving map and says nothing else.
- **Owning input**, the rim is Radar Cyan at 50%. Exactly one island is in
  this state at a time, and it is the one under the pointer.

That is what buys the register. A lit rim on every panel is decoration; a lit
rim on one panel is a focus indicator that happens to look like an instrument.

### Texture, confined

A horizontal rule texture was rendered across the whole body and rejected.
The rules cut straight through the rows, and the first thing to lose was the
smallest text on the panel — the class line under each hull name. Texture
that crosses text does not read as texture; it reads as interference.

The same failure killed a machined-slot pass: at 3px wide on an 8px pitch
across a 340px band, forty slots are not ventilation, they are a dashed rule,
and a dashed rule running through the title reads as a rendering fault.

Confined to the **title band**, at a 4px pitch and 17/255 white, the texture
is free. The band holds one short label and a count, nothing is lost, and the
panel picks up a screen-like texture that supports rather than competes.

This is the general rule and it covers every future surface: **texture and
hardware detail live on the title band, never across a body.**

## Colors

The palette is a dark, low-chroma console field with one high-chroma signal
and a small set of strictly-scoped semantic hues.

### Primary
- **Radar Cyan** (#22D3EE). The only accent. It marks live state on chrome
  and the surface that owns input: hover and press washes, the ON fill of a
  toggle, selection, and a focused island's rim. Its rarity is what makes an
  ON state readable across a room.

### Secondary
The group-rank palette is categorical and belongs to the map. It is barred
from chrome.
- **Unsur Green** (#16A34A): rank 1 zone.
- **Satuan Tugas Blue** (#2563EB): rank 2 zone.
- **Gugus Purple** (#9333EA): rank 3 zone.
- **Operasi Gabungan Orange** (#EA580C): rank 4 zone. Zones paint highest
  rank first so lower ranks layer over them.

### Status & Signal
Every state line routes through one function, so state is never gray on gray.
- **Signal Green** (#4ADE80): connected, synced, signed in, issued.
- **Warning Sand** (#FABF69): connecting, transitional.
- **Fault Red** (#F87171): failed, refused, unreachable.
- **Idle Gray** (#808080): neutral and stale.
- **Data Amber** (#F59E0B): age of data, never feed state.
- **Alert Yellow** (#FFFF00): attention without fault, and the strong
  `PAUSED` label.

### Neutral
The Console Night family carries all chrome; every step is a depth, not a hue.
- **Console Night** (#0F172A): island and zone fill.
- **Panel Slate** (#1E293B): the title band and inset surfaces.
- **Deep Well** (#020617): input wells, and the near-black the map sits against.
- **Hairline Slate** (#334155): every 1px stroke.
- **Cut Grey** (#8C9BAE): the chamfer's 2px cut edge, and an island's resting rim.
- **Body Silver** (#8C8C8C): body text.
- **Button Label** (#B4B4B4): resting button text, lifting to #F0F0F0 on hover.
- **Button Graphite** (#3C3C3C): resting button fill, dull so the cyan wake-up
  reads as the event.
- **Map Ink** (#E2E8F0): the only text drawn on tiles.

### Named Rules
**The One Signal Rule.** Radar Cyan is live or yours, and never anything else.
It is not a border, not a heading, not a background field, and not an
everywhere-lit rim. **The Rank Rule.** Green → Blue → Purple → Orange is
group rank and nothing else; a rank colour on a panel is a bug. **The Never
Gray-On-Gray Rule.** Any line reporting state goes through the status ink
function.

## Typography

**Display Font:** none, by design.
**Body Font:** Ubuntu Light, with system-ui fallback.
**Label/Mono Font:** Hack, with Ubuntu Mono and monospace fallback.

A single light humanist sans does the human work, paired with a rigid
monospace only ever allowed to show machine output. Lightness is what keeps
a dense console readable; emphasis comes from colour and position.

- **Heading** (300, 18px): one per zone-level surface. Never scaled up.
- **Body** (300, 13px): the workhorse, in the spacing rhythm.
- **Button** (300, 13px): sentence case, lowercase verbs.
- **Small** (300, 9px): the zoom steppers and tight chrome.
- **Map Label** (300, 12px): painted on tiles, always Map Ink.
- **Mono** (400, 13px): event feed, session logs, ids.

**The Two-Face Rule.** If a human wrote it, it is Ubuntu Light. If a machine
produced it, it is Hack. No third family, and no monospace for effect.

## Layout and density

There is no grid and no breakpoint set. This is a native desktop window, so
the spatial model is a **stage**, not a page.

- **Spacing rhythm** — item spacing 8 × 6px, button padding 12 × 8px, island
  inner margin 16px on all four sides. Rows are horizontal groups separated
  by 1px dividers rather than by large gaps. Island verbs are full-width
  32px rows.
- **Zone rhythm** — 2px between islands in the side zone, 24px between a
  zone and the window edge. The zone is resizable by its inner edge
  (240–600px, default 320); islands stretch to the live width and the camera
  offset follows it, so a resize never covers a framed hull.
- **Scroll discipline** — island bodies scroll at a 420px cap so a tall
  island never swallows the map. Dense inner lists get their own tighter cap
  with their header pinned outside it.
- **Miller columns** — the Fleet Picker's taxonomy columns are fixed so the
  column set stays aligned while browsing.
- **Adaptation** — instead of reflowing, the operator rearranges: the side
  zone docks left or right, and islands are dragged within it.

## Elevation and depth

Layered, not lifted. Console Night chrome sits on Panel Slate insets, which
sit over the Deep Well the map is drawn against, plus a 1px hairline on every
edge.

- **Island cast** — a hard offset, flat black at 120/255, offset 5 × 6px.
  Present for separation from a moving map, not for drama. A blurred shadow
  is a fill-rate cost proportional to kernel radius and is the first thing
  that stops being affordable on a software rasteriser.
- **Popup cast** — combobox and menu popups only, one step tighter.
- **No shadow on rules.** A divider is a 1px stroke with zero elevation.

**The Hairline Rule.** Every panel edge is 1px of Hairline Slate. A surface
that needs presence steps a tone lighter; it does not get a bigger shadow or
a second border.

**The One Exception.** An island's rim is 2px, and it is the only edge in the
application that is not 1px. It says the surface is chamfered and therefore
draggable, and it is the only place the heat register is allowed to glow.

## Shapes

Two shapes, two meanings. Rectangles are controls and containers; circles are
objects on the map. Nothing mixes them.

- **Islands and zones** — top-right corner chamfered 20px, cut stroked 2px in
  Cut Grey. Every other corner square.
- **Controls** — buttons, toggles, selects and fields all sit at 6px radius,
  uniform across the family, so a control is recognisable by silhouette.
- **Non-interactive chrome** — dividers, the top zone, and unrounded panels
  stay square. Radius signals "you can touch this"; the chamfer signals "this
  whole surface moves".

**The cut edge is never cyan at rest.** It brightens, and it turns cyan, only
on the island that owns input.

- **Map symbology** — a 32px symbol box. The affiliation frame is an outline
  AND a fill: the shape carries allegiance, the fill repeats it redundantly in
  the same hue, darkened toward map ink so it survives being drawn over water
  and land. The glyph sits inside that fill in a lightened type accent, because
  an accent drawn straight onto the map lands near 1:1 contrast and vanishes.
  The outline is CASED — a 1.25px dark keyline drawn just outside the hue,
  never a wider stroke centred on it, which would grow inward and eat the
  glyph. A frame reads by the weaker of its two edges, and the outer one, hue
  against water, was 1.03:1; the keyline puts it at 9:1 on both water and
  land, which is why the hue does not have to be darkened out of recognition.
  Dashed frame for planned, solid for present; a 2px trail dot at 55% of the
  affiliation ink; 2px zone strokes over 70/255 fills. State rings sit at 1.5,
  1.75 and 2.0 disc radii, so they stay outside the symbol rather than closing
  around its middle.

## Components

### Buttons
Plain, compact, instrument-like. A dull graphite pill that only wakes up when
touched. 6px radius, 10 × 6px padding, sentence-case labels. There is no
filled brand button; importance is expressed by position and by the ON state.
Hover is a cyan wash at 16%, pressed at 27% with white text, disabled at 50%
alpha in place.

### Toggles and selects
The same 6px shape. OFF is Button Graphite; ON is solid Radar Cyan with Deep
Well text, the highest-contrast pairing in the system, which is why an active
control is findable from across a room.

### Zones
A zone is a column or a band of islands and nothing else. A zone has no
background of its own; the islands inside it are the visible surface, and the
map shows through the gaps. A zone that grew a panel of its own would be a
fourth kind of thing.

### Inputs
Deep Well background so a field reads as cut into the panel, Body Silver text,
6px radius, 280px default. Focus is the cyan signal: cursor and selection fill
are Radar Cyan and the selection text is Deep Well. Errors never live inside
the field; they print as a Fault Red status line beneath it with the recovery
named on the adjacent control.

### Signature components
- **Status line** — one row of state ink, text only. The most-used component
  in the app.
- **Unit marker** — affiliation frame (shape + fill, stroke cased in dark)
  with the type glyph inside it, the name offset off the box's corner, and
  state rings at 1.5/1.75/2 disc radii.
- **User island** — a default person glyph at 32px, the display name, and the
  app role as a small mono tag. It is the only surface in the app that is
  about a person rather than about state.

## Motion

The governing idea: **motion tells the operator the interface heard them,
keeps a state change legible, and makes a jump in space intelligible.**
Anything else is decoration, and decoration on information-dense chrome
hinders.

**The frequency rule.** The more often something happens, the less it may
move. A 100×/day action gets no animation. A keyboard shortcut never gets
animation. A once-per-exercise transition may be generous.

egui's animation is linear in elapsed time with the curve chosen by the
caller, and it retargets an in-flight animation from its current value, so an
eased tween is correct and a hand-rolled spring is a reinvention.

| Curve | Use for |
| --- | --- |
| `cubic_out` | Everything that enters or moves. A surface settles, never accelerates away. |
| `quadratic_in_out` | Two-ended transitions where both ends are visible. |
| `linear` | Nothing. It is a default, not a choice. |

| Moment | Duration | Note |
| --- | --- | --- |
| Press feedback | 0 | Colour state only. |
| Island / control hover, rim lighting | 0 | Instantaneous. A focus indicator that fades in is late. |
| Island open, modal step, tab change | 150–200ms | `cubic_out` |
| Side zone show / hide, dock flip | 180ms | `cubic_out`, width and opacity together |
| Camera recentre on a hull | 300ms | `cubic_out`, on centre only, never on zoom |
| Zoom step (`−` / `+`) | 200ms | `cubic_out`. The wheel is a gesture and is never tweened. |
| Game state change | 250ms | `cubic_out`. Once per state, so it may be generous. |

**What may animate.** Camera centre on a deliberate operator action, island
and modal entrances from the direction the operator came from, an indicator
travelling between fixed positions, and a surface's own state.

**What may not animate.** Unit markers, which glide because that is display
interpolation and never for style. Anything that pulses: the wire drops in
bulk and a stale marker does not need to flap. Lists, which repopulate
continuously and would re-fire a stagger forever. Anything keyboard-initiated.

**Island open and close is explicitly allowed to animate here**, which the
previous system forbade. The earlier rule existed because a close button was
already the feedback, and with islands now arriving in a column a 150ms
settle is what makes a column read as a stack rather than as a pop. The
duration stays at the low end for that reason.

**Reduced motion** is one switch, not per-component: setting
`style.animation_time = 0` collapses every animation to its target.

## Do's and Don'ts

### Do:
- Route every state-bearing message through the status ink function.
- Keep Radar Cyan to live state and the focused surface.
- Separate panels with a bold cut-grey border and step a tone for depth.
- Light a rim only on the island that owns input.
- Keep texture and hardware detail on a title band, never across a body.
- Put machine output in Hack and give every island a fixed footprint.
- Draw map objects as circles with 2px rings and label them in Map Ink.
- Hide a verb the backend would refuse rather than showing one that 409s.

### Don't:
- Introduce a second chrome accent, a gradient, or a glow on a surface that
  is not reporting state.
- Put a rank colour on chrome.
- Ship gray-on-gray status, or animate to announce state.
- Add a third typeface, bold body text, or monospace for anything a human wrote.
- Reach for a bigger shadow when a surface needs presence.
- Round the top zone, or invent breakpoints and responsive stacking.
- Put a panel anywhere but one of the three zones.