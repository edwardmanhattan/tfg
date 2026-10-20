# Visual integrity check — the GPUI console

Date: 2026-10-20. Tree: `dd03e34` plus the working-set changes.
Subject: `src/shell_gpui.rs` (25,936 lines), `src/view/mod.rs` (the token layer).

**Verdict.** The console does not have "a few inconsistencies". It has **two
different design systems living side by side in the same window**, and the
drift is not in the tokens — the token layer is disciplined — it is in the
26,000 lines that draw with it. Three findings are structural and are visible
in any screenshot; the rest are a long tail of one-off values that never
route through the token layer.

| | count |
|---|---|
| Surfaces captured and reviewed | 26 |
| Surfaces with at least one visible inconsistency | 19 |
| Token-layer values defined in `view` and never used by the shell | 9 of 47 |
| Distinct text sizes in the shell | 14 (token scale has 4) |
| Distinct corner radii | 10 (tokens have 2) |
| Distinct island body paddings | 3 (tokens have 2) |
| Literal colour constants that duplicate a token | 6 |
| Unregistered font families in use | 2 (`Inter`, `monospace`) |

---

# What was fixed, and what was left

The fix order in this document was worked through in slices, each verified
by capture against the real binary. The map basemap was not touched: it is
the one item on the list that is a product answer rather than a patch.

## Fixed

| # | finding | what changed |
|---|---|---|
| 1 | two panel languages | the modal silhouette is now a flat rectangle with a 1px `CUT_GREY` edge (the chamfer is gone, and `view::CUT` with it); the execution widgets are opaque `CONSOLE_NIGHT` with the same edge and no radius; the sign-in card is square with the same edge; the Inspector dock's radius is `view::PANEL_RADIUS` |
| 2 | two island bodies with no padding | every body root now uses `view::PANEL_PAD` and `view::PANEL_GAP`, including the Log and the Assessment |
| 3 | two surfaces clipping their own content | the Essentials island is 470pt (was 380) and the Settings card is 380pt (was 320); both now fit their content exactly (`content == viewport`, `down=false`) |
| 4 | two typefaces the console does not have | every `font_family("Inter")` and `"monospace"` is gone: the Log's feed is `FACE_MACHINE`, the Assessment's labels are `FACE_HUMAN` |
| 5 | fourteen text sizes | one scale, every size a token: `LABEL_PT` 10, `DENSE_PT` 11, `MACHINE_PT` 12, `BODY_PT` 13, `READOUT_SM_PT` 18, `READOUT_LG_PT` 30. `DENSE_PT` is new and documented — the console's column is 320pt wide and 13pt does not fit a row of four columns in it. The old egui-era `TEXT_*_PT` set is deleted |
| 6 | state drawn in the wrong ink | a status line is now a `Status` — the words **and** the kind — and the kind decides the ink: `FAULT_RED` for a refusal, `SIGNAL_GREEN` for a write that landed, `WARNING_SAND` for one in flight, body silver for a fact. 90-odd assignments converted; the untokenised `STALE_SAND` is deleted; `DATA_AMBER` now carries staleness |
| 7 | one state, three colours | "ready" is `SIGNAL_GREEN` everywhere; "not arrived" is `WARNING_SAND`; "unplaced" is `DATA_AMBER` |
| 8 | the accent spent on twelve things | out of "SEATED", the role labels, the row wash, "new group" (now a verb pill), the wordmark, the sign-in button, the drill tab's fill and the step card's bar. One selection treatment in the console: a 1px cyan edge and the bright ink, never a filled slab |
| 9 | ten corner radii | the panel radius is 0, the control radius is `CONTROL_RADIUS`, and the sign-in card and the dock follow |
| 10 | the daylight map | **not fixed** — a product answer, see below |
| 11 | the band was a 47% scrim | the top band and the step-reminder strip are opaque `CONSOLE_NIGHT` |
| 12 | four label languages | the eight modal titles go through `panel_title()`, the same unit the island bands use; the Assessment's facts go through `kv()`; the Inspector's title is a name in the human face at the label size |
| 13 | two leading figures in the rail | the course keeps the 30pt, the speed drops to the readout that sits inside a row; "kn" is in the human face, because a unit is a word a human wrote |
| 14 | "(s)" in twenty places | `plural(n, noun)`; "2 scenarios, 3 steps", "1 hull", "6 hulls" |
| 15 | two steppers, two designs | one `stepper()` component, in the Settings card and the Command island — and the Command island's two buttons were **dead** (they pointed at ids nothing listened to), so they now write the same draft the rail's compass writes |
| 16 | the zone width was a number of its own | the shell's default is `view::ZONE_W` (320, was 360) and the Settings stepper clamps to `ZONE_MIN_W`/`ZONE_MAX_W` (240–600, was 240–520) |
| 17 | `DESIGN.md` was the wrong document | it now says so, and points at the surface brief and the token layer |
| 18 | the Fleet picker's rows ran off the card | a row was one flex row, and a flex row does not wrap: a room with eight seats ran the handover pills past the card's edge and they were cut in half. The row is a column now — the hull, its state and its commander on the first line, the seats on a wrapped second |

### The docked sidebar

| # | finding | what changed |
|---|---|---|
| 19 | the column floated over the map instead of being docked | it now starts at the window's left edge and runs to its bottom edge — flush the way the top band is flush. It was inset 24pt on the outer edge and 22pt under the band, and the 22pt was there in the three phases with no strip to put there, which is the tell: it was a leftover, not a margin |
| 20 | the fields were separate cards, not a column | the fields touch and share a rule. Every field draws the rule BELOW it and only the first draws the one above, so the rule between two fields is one line. The 2pt gap it used to carry put the MAP between two fields of the same panel |
| 20a | the field outlines were drawn UNDER the field's own content, and the content painted over them | the measured result was 40% of the edge's strength on the right and nothing at all along the bottom, where the next field's band covered it — a 1px rule that is 40% grey is not a rule. The column now paints the fields first and the outlines after, over the whole stack, so every edge is at full strength and the rule between two fields is drawn once, by the field above. What remains under the content is the field's hitbox, which is a shield and not meant to be seen |
| 21 | the owned field's perimeter did not close | in a joined column a field's top edge belongs to the field above, so an owned field that is not first now draws its own 1px cyan line over that rule and its perimeter closes |
| 22 | a wheel over the console zoomed the map | the map's surface is a full-window hitbox and GPUI's scroll hit test collects EVERY hitbox under the pointer rather than the path to it, so a wheel over the column, the top band, the rail, the widgets or a modal reached the map's handler. Every piece of chrome now stops the propagation, and the column's handler claims the gesture. The regression test is `a_wheel_over_the_column_does_not_zoom_the_map`, and it fails without the fix (the map zoomed to 10.8) |

### Two more, found only against the hosted deployment

These did not exist in the local fixture's data, and are recorded here
because they are the reason the audit was re-run against the real server.

| # | finding | what changed |
|---|---|---|
| 23 | the rail's picture channel printed the hull number the next channel already states | the register's label is "KRI Abdul Halim Perdanakusuma (KRI-355)" — name *and* hull number — and the identity channel one to the right already says the hull number on its own HULL row. The picture channel now prints the name alone, and the name is a function of the label rather than the whole label |
| 24 | the Command island's HULLS row cut a real hull name in half | the row was one line — name, course, speed, state — in a fixed 318pt budget, which was the column's old 360pt width minus a guess. The register's names run to 31 characters, so the name was cut mid-word, and this toolkit's `truncate` clips a div-wrapped string without an ellipsis (measured, not assumed: a 60pt-wide probe clipped "KRI Abdul Halim Perdanakusuma" at "KRI Abdul Ha" and drew no "…"). The name now has a line of its own at the panel's full width, and the three readings keep their columns beneath it |

Both were verified by capture against the hosted API. Finding 24 was
introduced by this audit's own change of the column's default width from
360pt to the token's 320pt, and finding 23 was a pre-existing duplication
that only the real register's long names made visible.

Two things were found on the way and are worth naming separately:

- **The helm steppers were dead controls.** The Command island drew a `−`
  and a `+` for the course and the speed, and nothing listened to them: the
  only way to set a course was to drag the rail's compass, and the rail is
  execution's, so in preparation the row looked like the way to steer and was
  not. They now write the same draft the rail writes and the same `SET HELM`
  sends.
- **Two tests carried a second copy of the zone width.** The camera-offset
  tests asserted literals (580, 220, 593.75) that were half the old
  default width. They now derive the expectation from `view::ZONE_W`, which
  is what they were always about.

## Left, deliberately

- **The basemap is a daylight map** under a night console. `MAP_STYLE` is
  `https://tiles.openfreemap.org/styles/liberty`. The palette was designed
  against a dark map — "the map is the room's only light source", the edge
  that holds on "bright land and dark water in the same frame", the tonal
  depth that goes nowhere on a cream basemap. Changing the style is one
  constant, but choosing between "a dark style" and "a console that admits
  it is a daytime chart" is the owner's call, not a patch.
- **The rail's picture channel is empty** where the hull has no photograph: a
  1px frame with a dot in it. The local mirror has no images, so this is
  what a missing asset looks like, not a bug in the drawing.

## After

`cargo test --lib`: **617 pass**, with the same four pre-existing failures
(`book::prune_drops_relations_for_vanished_drafts`,
`map_render::quad_bow_follows_the_compass`,
`roster::seats_come_before_roles_before_unseats`,
`sim::order_move_applies_then_higher_overrides_loudly`) that the GPUI
handoff records as untouched by migration work. `cargo test --lib view::`:
41 pass. The console was captured in 26 states before and after, and
re-captured against the hosted dev API after the two data-driven fixes;
the three sets are in `.impeccable/review/integrity-2026-10-20/`.

---


Two passes, and they agree.

**Static.** Every value the shell paints was extracted from `src/shell_gpui.rs`
and compared against `src/view/mod.rs`. Counts are `grep` counts against the
source, not estimates. The shell glob-imports the token layer
(`use crate::view::{self as view, *};`, `shell_gpui.rs:54`), so unqualified
token names count as uses.

**Visual.** The real binary was run against the **hosted dev API**
(`https://api.tfg.development.crossnet.co.id/api/v1`, the deployment the
operator actually uses) and captured on the Wayland compositor with
`scripts/gpui-capture.sh` — the console is a GPUI/Vulkan app and does not
render on Xvfb. 26 launches, one per state, each opened by a `TFG_*` seed.
Captures are in `/tmp/opencode/shots/`.

> **The first pass was run against the local Minos stack, and that was
> wrong.** The local stack is a throwaway database seeded with a fixture:
> short hull names, one exercise, one room. It made the console look
> tidier than it is. Re-running the same sweep against the hosted
> deployment found two defects the fixture could not produce — a rail
> channel that overran its own edge, and a ledger row that cut a real hull
> name in half — because the register's real names run to 31 characters and
> the fixture's ran to 20. The harness now points at the hosted API by
> default, which is where it always pointed: `scripts/gpui-capture.sh`
> already exported the hosted superuser credentials, and the local host was
> an override in the sweep, not the script's default.

The source of truth for the console is **not** `DESIGN.md`. That file describes
the egui era: chamfered floating islands, Ubuntu Light + Hack, a 20px cut,
cast shadows. The GPUI console replaced that with the "instrument panel"
direction in `.impeccable/surfaces/gpui-console.md`: *square, flat, one 1px
`CUT_GREY` edge, no chamfer, no cast shadow, no bevel, no rim, no gradient;
Saira Condensed for humans, PT Mono for machines; Radar Cyan is the only
accent and means live-or-yours.* `view/mod.rs` implements that direction.
**`DESIGN.md` has not been updated to match and is now misleading** — it is
the document a reader would check a component against, and it is wrong about
the chamfer, the typeface, the cast, the radius and the panel language.

---

## P0 — The three structural breaks

### 1. The console draws two panel languages, and the phase decides which

The island column and every modal draw one way; the execution widgets and the
sign-in card draw the old way.

| surface | fill | edge | radius | code |
|---|---|---|---|---|
| island (column) | `CONSOLE_NIGHT`, opaque | 1px `CUT_GREY` | 0 | `shell_gpui.rs:1990-1996` |
| modal | `CONSOLE_NIGHT`, opaque | **2px** `CUT_GREY`, **20px top-right chamfer** | chamfer | `shell_gpui.rs:1094-1122` |
| execution widget | **`PANEL_SLATE` at alpha 214/255** | 1px `HAIRLINE_SLATE` | **6px** | `shell_gpui.rs:10108-10109` |
| sign-in card | `DEEP_WELL` | 1px `HAIRLINE_SLATE` | **8px** | `shell_gpui.rs:12633-12636` |

Measured, not inferred. In `02-exec.png` the execution widget over map water
reads `srgb(52,67,92)`; `PANEL_SLATE` at 214/255 over the same water predicts
`srgb(53,67,90)`. It matches. The panel is 84% opaque and the map's coastline,
its river label and its place names are legible **through** it — the exact
failure the commander's-rail note in `.impeccable/surfaces/gpui-console.md`
already records as unacceptable ("the tile map's own labels came through the
type that sat on them … cannot put its readings on top of a map and call it
legible"). The rail was fixed for this; the widgets were not.

**The modal's chamfer is the bigger tell**: the brief forbids it outright
("no chamfer"), and it is the only chamfered surface left in the app.

### 2. Two island bodies have no padding at all

`shell_gpui.rs:8760` (Log) and `:9186` (Assessment) build their column as
`div().flex().flex_col().w_full()` — no `p()`, no `gap()`. Every other island
body has 10pt (`view::PANEL_PAD`) or 12pt (`PAD`).

In `20-closure.png` the consequence is visible: the Log island's text starts at
the panel's own 1px edge and the leading "N" of "No events yet" is cut by it;
the Assessment island's "SESSION" and "SHOOT-01" are clipped by the edge in
the same way, and the Log's second line runs into the right-hand edge and is
cut mid-word. The rest of the column insets 10–12pt. Same column, same width,
same edge colour, three different margins.

### 3. Two surfaces clip their own content mid-glyph

- **Essentials / "MOVE TO PREPARATION"** — the button is painted half, and the
  Control island's title band is drawn over the other half. Reproduced in
  `00-console.png`, `16-plan-lit.png`, `11-compose.png` and `22-board.png`.
  The Essentials island is a fixed 380pt (`view/mod.rs:1069`) and its content
  is taller; the body scrolls, but the operator sees a sliced button and no
  scrollbar.
- **Settings modal** — `08-settings.png`, last line: *"Geographic grid and
  text size need the map-rig and text-scale slices — not wired yet."* is cut
  horizontally through the middle of the glyphs by the modal's bottom edge.

The project already has this defect class named — `docs/ui-overhaul-status.md`
calls it the silent-clip defect, and the surface brief records the exact
arithmetic trap (an overflow smaller than `BODY_SCROLL_SLACK` reports
`down=false`, so the clipped line is unreachable). It is back, in two places.

---

## P1 — Visible inconsistencies, ranked

### 4. Two typefaces the console does not have

`font_family("Inter")` at `shell_gpui.rs:8755, 9180, 9213, 9264, 9399, 9514`
and `font_family("monospace")` at `:15239`. **Neither is registered** — the
binary registers Saira Condensed (4 weights) and PT Mono only
(`shell_gpui.rs:18605-18640`). GPUI silently falls back to the system face.

The Log island and the whole Assessment island therefore render in a
different typeface from every other panel. It is the single most obvious
break in `20-closure.png`: "Summary / Timeline / Judgements / Reviews" is a
wide grotesque; "OPERATOR" and "ASSESSMENT" are narrow tracked caps. The
Assessment tab strip is a *machine* surface drawn in the *wrong* face — it
should be PT Mono at `MACHINE_PT`, and it is in neither.

### 5. Six sizes for a name, four for prose, fourteen in total

The token scale is four: `LABEL_PT` 10, `BODY_PT` 13, `MACHINE_PT` 12,
`READOUT_LG_PT` 30 (plus `READOUT_SM_PT` 18, unused). The shell draws text at
**9, 9.5, 10, 10.5, 11, 11.5, 12, 12.5, 13, 14, 15, 26, 30** pt.

`.text_size(px(11.0))` alone appears **65 times**, and `note()` — the
sanctioned prose unit — appears 35. So most prose in the console is hand-built
at 11pt rather than routed through the unit that exists for it.

A person's or hull's name is drawn at 15pt (`shell_gpui.rs:3570` operator,
`:3641` session), 13pt (`:5899` crew, `:6537` seat, `:6636` hull), 12pt
(`:12419` board card), 11.5pt (`:4147` player picker) and 11pt (`:5824`
players, `:5420` fleet, `:5211` drill leaf). Five sizes for one role.

`note()` is 13pt. Prose is also 11pt (`:3690` the Game Master's plan — the
most human text in the app), 10.5pt (`:4645`, `:4808`, `:12433`) and 10pt
(`:2584`, `:4978`, `:11110`).

**And the two scales coexist on purpose:** the buttons use the *old* egui
scale (`TEXT_BUTTON_PT` 15, `TEXT_SMALL_PT` 10.5 — `shell_gpui.rs:15632`)
while the panels use the new one (`LABEL_PT` 10). Measured on
`00-console.png`: "SCENARIO COMPOSER" renders 15pt against the 10pt
"PLANNED WINDOW" beside it. A verb is 1.5× its own section header.

### 6. State is drawn in the wrong ink, all over

`FAULT_RED` — the token for a fault — is used **5 times in the whole file**,
all in drag-and-drop, and **never for a write refusal**.

| what | drawn as | code |
|---|---|---|
| server refusal (helm) | untokenised `STALE_SAND` `#F6C56B` | `:10473` |
| server refusal (telegram) | `STALE_SAND` | `:11168` |
| server refusal (command island) | `STALE_SAND` | `:6742` |
| login failure | `STALE_SAND` | `:12685` |
| phase-advance outcome | `note()` → body silver | `:6494` |
| declare-ready outcome | `note()` → body silver | `:6601` |
| step-declare outcome | `note()` → body silver | `:12112` |
| prepare / factor / pause / embark / hold outcomes | `small()` → body silver | `:3761, 3854, 3881, 5528, 5625, 5709` |
| "name required" (visible in `11-compose.png`) | body silver | `:12949` |

`:6494` and `:6601` carry comments that *say* the line goes through the status
palette and then call `note()`, which is body silver. The contract is
documented in the code and broken in the code.

`STALE_SAND` (`#F6C56B`) is a near-twin of the token `WARNING_SAND`
(`#FABF69`) — two sands, one token, and the untokenised one is used for four
different jobs: refusal, staleness, busy-state and login error. `DATA_AMBER`
(`#F59E0B`), the token that owns *age of data*, is used **zero times**; the
Fleet island's "unplaced" dot and the Command island's "stale" column use
`STALE_SAND` instead (`25-scroll2.png` shows "unplaced" in yellow).

Two inversions: `:3988`, `:4646`, `:4809` paint a **success** and a
**refusal** in the same `ALERT_YELLOW`, and a *pending* message in grey.

And the counts that report state are grey: "0 OUTSTANDING" on the Review
band (`:3514`, `:1874`), "{n}/{m} in the room · {k} ready" in the top band
(`:3021`), "0 IN ROOM" (`:3494`).

### 7. One state, three colours

| state | where | colour | code |
|---|---|---|---|
| ready | Players island | **`RADAR_CYAN`** | `:5842` |
| ready | Crew modal | `SIGNAL_GREEN` | `:6050` |
| not arrived | Crew island | `ALERT_YELLOW` | `:5886-5894` |
| unplaced | Fleet island | `ALERT_YELLOW` | `:5414` |
| outstanding | Review band | body silver | `:3514` |

Visible together in `21-prep.png` and `25-scroll2.png`: "not arrived" and
"unplaced" are both alert-yellow, which the token layer reserves for
"attention without fault".

### 8. The accent is spent on eight things that are not "live or yours"

The brief: *"Radar Cyan is still the only accent and still means live or
yours; it is the edge of the one panel that owns input, at full alpha, and it
is nowhere else."*

| use | code |
|---|---|
| "SEATED" on all 8 accounts in the Player picker | `:4065` |
| "Commando" on 3 player rows | `:6005` |
| "ready" in the Players island | `:5842` |
| a cyan wash on every own row | `:5805` |
| "new group" link | `:6387` |
| the Player picker's selected role pills (cyan outline) | `:4156` |
| a 2px cyan bar inside a selected step card | `:11959` |
| a 210-alpha full-width cyan band under the top band | `:15201` |
| the "ARCONS" wordmark, 26pt | `:12646` |
| the **full-cyan Sign-in button** | `:12671` |
| the **full-cyan selected drill tab** | `:4823` |
| the dial needle and the speed fill, at 150/100 alpha | `:1302-1362` |

The last two are the largest spends in the app: a solid cyan slab for the
primary sign-in action, and a solid cyan pill for a selected tab. Both are
*selection* colours, and every other button in the console is graphite.
`06-fleet.png` shows the cyan "Air (0)" tab sitting next to four graphite
siblings; the Player picker shows a **cyan-outlined** selected role next to
graphite siblings. Two different selection treatments in one app.

### 9. Ten corner radii, two of them tokens

`rounded(px(3.0))` ×17, `8.0` ×8, `6.0` ×8, `4.0` ×6, `7.0` ×2, `2.0` ×2,
`1.0` ×2, `5.0` ×1, `2.5` ×1, `14.0` ×1. The token layer owns exactly two:
`CONTROL_RADIUS` 6 and `PANEL_RADIUS` 0. The brief: *"Panels are square,
flat"* and *"Controls — buttons, toggles, selects and fields all sit at 6px
radius, uniform across the family, so a control is recognisable by
silhouette."*

Visible: the sign-in card is 8px, the execution widgets 6px, the taxonomy rows
3px, the radio 7px, the log-in card fields 8px, the board cards 3px, the
Player picker's account rows 3px — while the buttons in the same app are 6px.

### 10. The map is a daylight map under a night console

`src/map_geometry.rs:24` — `MAP_STYLE = "https://tiles.openfreemap.org/styles/liberty"`.
Every capture shows a bright beige/blue daytime basemap under a `#0F172A`
console. The design says *"the map is the room's only light source"*, *"a
dark navy instrument console floating over a night tile map"*, and the whole
edge-and-casing argument is built on *"the map is drawn against Deep Well"*.

The consequence is not aesthetic. The 1px `CUT_GREY` edge is specified to hold
against *"bright land and dark water in the same frame"*; on this basemap the
column's panels are navy-on-cream, the highest-contrast pairing in the app,
and the console's tonal depth system is doing nothing.

### 11. The top band is a 47% scrim, not a band

`shell_gpui.rs:3203` — `.bg(rgba(Rgba::black_alpha(120)))`. Measured on
`02-exec.png` at (200,20): `srgb(83,99,133)`; 0.53 × the water behind it
predicts `srgb(83,100,134)`. The band is the map with a black filter over it,
so the map's roads and labels run through the type that sits on them — which
is the third recorded instance of that failure (the brief lists it for the
rail; the `docs/gpui-migration-handoff.md` "Next" item 3 says the same for
the band: *"Give the top band a fill, or decide it does not need one"*).

The step-reminder strip is the same scrim (`:15159`).

### 12. Section headers, titles and labels come in four languages

| kind | style | where | code |
|---|---|---|---|
| panel title | tracked caps, 10pt, `MAP_INK` | every island band | `:1861` |
| modal title | sentence case, 12.5pt, human face | all 8 modals | `:2470, 3963, 4446, 5942, 11617, 12502, 12844, 13368` |
| section header | `section()` — tracked caps + a rule | 9 uses | `:1614` |
| section header | `small()` — 11pt grey, no rule, no tracking | 11 islands | `:2442, 3607, 3779, 3938, 4421, 5302, 5473, 5642, 5722, 8753, 9178` |
| inspector column labels | lowercase grey | the Inspector dock | `:12419+` |
| form field labels | sentence case 13pt | the New-session form | `:12844+` |

The Control island's "GAME TIME MULTIPLIER" is 11pt grey with no rule; the
Command island's "HULLS" is 10pt tracked caps over a rule. Same column, same
width, same moment.

And the Inspector dock's title (`:12502`) sets **no font family at all** — it
falls to the system default, a third title face in the app.

### 13. The rail carries two leading figures

`view::READOUT_LG_PT`'s own doc: *"The one readout that leads a panel. Never
more than one per panel."* The commander's rail has two 30pt figures —
`figure(&course, true, MAP_INK)` at `:10420` and `figure(&knots, true, MAP_INK)`
at `:10432` — visible side by side in `02-exec.png` as "6°" and "29.9 kn".

The panel that was designed for a leading figure, the Review island, has one
in code (`:6438, 6442`) that is unreachable in this environment (the readiness
checklist is unpublished) — so the only 30pt figures in the console are the two
that break the rule.

The rail's labels are also their own size: `DASH_LABEL_PT = 11.0` (`:854`),
where every other tracked label is `LABEL_PT` 10.

### 14. Copy that a human wrote, in a machine's voice

`"(s)"` pluralisation appears **31 times** in the shell: "2 scenario(s), 3
step(s).", "1 hull(s)", "0 hull(s)", "7 seat(s)", "0 message(s)", "3 step(s)",
"3 step(s) in the book", "message(s) in this exercise", "person(s)". Visible
in `00-console.png`, `22-board.png`, `07-comms.png`, `25-scroll2.png`.

Prose in the machine face: `machine("kn", BODY_SILVER)` (`:10433` — "knots"
is a word), "no session" (`:11660`), "not started · 3 step(s)" (`:11674`),
"person(s)" (`:6007`).

Machine output in the human face: the rail's whole right-hand column —
hull number, group, embark — is `FACE_HUMAN` at 13pt (`:1752-1759`), while
the `kv` unit puts the same class of value in `FACE_MACHINE` at 12pt
(`:1634-1651`).

### 15. Two steppers, two button sizes, one modal

The Settings modal's width stepper is one graphite pill: `− 360 +`
(`08-settings.png`). The Command island's helm steppers are three separate
boxes: a `−` button, a recessed value box, a `+` button (`:6740+`,
`21-prep.png`). Same console, same control, two designs.

Button sizes inside one modal: "PLAY" and "SKIP" on the board's step cards are
small buttons; "PAUSE CLOCK" and "END SESSION →" at the bottom of the same
modal are full-size (`22-board.png`).

---

## P2 — Drift that will bite later

- **Zone width bypasses the token.** The shell's default is `zone_w: 360.0`
  (`:2225`) against `view::ZONE_W = 320.0`, and the Settings steppers clamp
  at 240–520 (`:2541`, `:2558`) against `ZONE_MIN_W`/`ZONE_MAX_W` = 240/**600**.
  The operator can never reach the token's maximum, and the token's default is
  40pt from the shell's. `ZONE_W`, `ZONE_MIN_W` and `ZONE_MAX_W` are used
  **zero times** in the shell.
- **Nine token values are defined and never used**: `PANEL_GAP` (0),
  `DATA_AMBER` (0), `ROW_TRACKING` (0), `ZONE_W`/`ZONE_MIN_W`/`ZONE_MAX_W` (0),
  `ISLAND_PAD` (0), `ISLAND_RADIUS` (0), `READOUT_SM_PT` (0),
  `BAND_SCAN_PITCH`/`BAND_SCAN_ALPHA` (0). The last two mean the title band's
  scanline texture — the design's one permitted ornament — is not implemented
  in the GPUI console at all.
- **The chamfer's geometry survives the chamfer's death.** `view::CUT` is
  still used 22 times, 15 of them to reserve 28pt of right padding inside
  island bodies for a cut that no longer exists (`:2459, 3566, 3619, 3791,
  3952, 4435, 5314, 5654, 5734, 5925, 11047, 11598, 12833, 13357`).
- **Two muted greys for the same job.** `BODY_SILVER.with_alpha(204)` (the
  sanctioned `note()`) and `with_alpha(178)` (14 ad-hoc hint divs) — 80% and
  70% of the same grey, with no rule to say which.
- **`PANEL_SLATE` appears at five alphas** (232, 214, 210, 200, 150, 140) and
  `CUT_GREY` at six. One "panel slate", one "cut grey", eleven different
  greys.
- **The grammar units are barely adopted.** `note()` 35 uses, `kv()` 3,
  `ruled()` 3, `section()` 9, `verdict()` 3, `machine_col()` 6,
  `figure()` 5 — against 65 hand-built 11pt divs. The console mostly does not
  use the vocabulary it defined for itself.
- **`ISLAND_CAST` and the cast shadow.** The token still defines a cast
  (`view/mod.rs:291`) that the new panel language does not use; one reference
  remains in the shell.
- **The rail's picture channel is empty.** `02-exec.png`: a 1px frame with a
  white dot in the middle — the hull's photograph is absent (the local mirror
  has no images) and the channel shows a map symbol instead, so the frame
  reads as a broken image rather than as a missing asset.

---

## What the fixes look like, in order

The order matters: each step makes the next one cheaper, and none of them
needs a new design decision — the design is already written down.

1. **One panel language.** Flatten the modal to the island's treatment
   (`modal_silhouette_at` → a 1px `CUT_GREY` square) and delete `CUT` and the
   15 right-paddings that reserve it. Make the execution widgets and the
   sign-in card opaque and square. *Files: `shell_gpui.rs:1094-1122`,
   `:10103-10109`, `:12630-12636`.*
2. **One padding and one gap.** `PANEL_PAD` + `PANEL_GAP` on every island body,
   including Log (`:8760`) and Assessment (`:9186`). *Kills P0-2 and half of
   the column's raggedness in four lines.*
3. **One type scale.** Map every literal `text_size(px(n))` onto
   `LABEL_PT`/`BODY_PT`/`MACHINE_PT`/`READOUT_LG_PT`; delete the six
   `font_family("Inter")` and the `"monospace"`; move the buttons from
   `TEXT_BUTTON_PT` onto the current scale. *One decision — which size a verb
   is — and it is the one open question already recorded in the surface brief
   ("the reference labels its pills in tracked caps … this console uses
   sentence case").*
4. **Status ink, everywhere.** Delete `STALE_SAND`; route every `*_status`
   line through a status ink, with `FAULT_RED` for refusals and
   `SIGNAL_GREEN` for successes. *Fixes the two comments that currently
   promise what the code does not do.*
5. **One accent.** Cyan = the panel that owns input, plus live state. Remove
   it from "SEATED", "Commando", the row wash, "new group", the wordmark, the
   sign-in button and the selected drill tab.
6. **Fix the two clips.** Essentials 380 → its content's height; Settings
   modal 320 → tall enough for its last line.
7. **One leading figure per panel.** The rail keeps the course; the speed
   drops to `READOUT_SM_PT`.
8. **The map.** Decide on the basemap: a dark style, or a console that admits
   it is a daytime chart. This one is a product answer, not a patch.

Items 1–7 are mechanical and are bounded by the token layer. Item 8 is the
one that needs the owner.

---

## Appendix — the capture set

All from the real binary on the Wayland compositor. The first set was
against the local Minos stack; the set that backs the findings above was
re-taken against the **hosted dev API**, and it is the hosted set that is
copied into `.impeccable/review/integrity-2026-10-20/`.

The captures live in `.impeccable/review/integrity-2026-10-20/`, in three
folders: `before/` (the local stack, the audit's first pass),
`after-local/` (the same server, after the fixes) and `after-hosted/`
(the hosted API, after the fixes). The two "after" sets are the same code
and the difference between them is the DATA — which is the whole point,
because the local fixture's hull names are 20 characters and the real
register's are 31.

The hosted games used, and why each:

| state | game | what it holds |
|---|---|---|
| planning | `9` ("Game 8") | a Commando seat, one hull, a full room of three |
| preparation | `41` ("GPUI helm proof") | a Commando seat and a commanded hull — the Command island's HULLS row with a real name in it |
| execution | `44` ("GPUI step proof") | a Commando seat, one hull, one scenario step, a live picture |
| closure | `11` ("Game 9") | a judge seat, one piece, a scenario with two steps |
| no session | — (`TFG_SESSIONS=1`) | the session picker over the real games list |

The local stack's file names are kept in the table below so the original
sweep can still be read; the hosted sweep used the same `TFG_*` seeds
against the games in the table above.

| local file | state | seed | kept as |
|---|---|---|---|
| `00-console.png` | planning, column at rest | `TFG_GAME=2` | `column-planning.png` |
| `02-exec.png` | execution: widgets + commander's rail | `TFG_GAME=1` | `exec-widgets-rail.png` |
| `03-sessions.png` | no session held | `TFG_SESSIONS=1` | `nosession.png` |
| `04-signedout.png` | sign-in card | `TFG_SIGNED_OUT=1` | `signin.png` |
| `05-player.png` | Player picker | `TFG_PLAYER=1` | `player-picker.png` |
| `06-fleet.png` | Fleet picker | `TFG_FLEET=1` | `fleet-picker.png` |
| `08-settings.png` | Settings | `TFG_SETTINGS=1` | `settings-clip.png` |
| `09-inspect.png` | Inspector dock | `TFG_INSPECT=1` | `inspector.png` |
| `11-compose.png` | New session form | `TFG_COMPOSE=1` | `new-session.png` |
| `20-closure.png` | closure: Assessment + Log | `TFG_PHASE=closure` | `closure-log-assessment.png` |
| `21-prep.png` | preparation: Command + Crew | `TFG_PHASE=preparation` | `prep-command-crew.png` |
| `22-board.png` | Scenario board | `TFG_BOARD=1` | `scenario-board.png` |
| `25-scroll2.png` | Fleet / Players / Groups islands | `TFG_SCROLL=900` | `fleet-players-groups.png` |
| `26-review.png` | Review island | `TFG_SCROLL=1300` | `review.png` |

Not reached by any seed, and therefore **not** checked: the Assessment
island's Timeline/Judgements/Reviews tabs (the phase seed does not reach
closure's populated branch), the Embarked modal's rows, the Crew modal's
person cards, the drop-and-drag states, and the map chrome (which
`map_chrome` gates off, `shell_gpui.rs:1664`).

Two seeds are no-ops and their captures are byte-identical to the baseline:
`TFG_RELATE=1` and `TFG_ORGANIZER=1` (the Relate surface is not reachable
from this seat, and the organiser column is not a seat property).
