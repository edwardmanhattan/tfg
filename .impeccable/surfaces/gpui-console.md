# Surface brief — the GPUI console

<!-- impeccable:surface-brief 1 -->

## Scope and visitor mode

- **Surface:** `src/shell_gpui.rs` (the GPUI console) with `src/view/mod.rs` as its
  toolkit-free kernel and `src/bin/tfg_gpui.rs` as the binary.
- **Primary target:** `src/view/mod.rs`
- **Related targets:** `src/shell_gpui.rs`, `src/bin/tfg_gpui.rs`, `assets/fonts/`
- **Mode:** **Operate.** The visitor is an operator completing a task — running a
  live wargaming exercise. Scanability, consistency and the real usage scene
  (exercise room, glare, field laptops) outrank expression.
- **Why this brief exists:** a replacement visual world for an existing console
  mid-cutover. `docs/ui-overhaul-status.md` and `docs/gpui-migration-handoff.md`
  stay the build log; this file owns the direction.

## Audience, job, and constraints

An operator at their own machine in an exercise room or the field, holding a seat
(Organizer / Commander / Helm / Observer), reading a live tile map and acting
within their authority. The job is: know where every unit is, know what state
the exercise is in, and issue or refuse the action the model permits.

Constraints that bind every slice:

- The palette is **pinned by the user** and is unchanged: Console Night, Deep
  Well, Panel Slate, Hairline Slate, Cut Grey, Body Silver, Map Ink, Radar Cyan
  as the one signal, plus the status inks and the rank colours barred from
  chrome. The reference image's grey/orange scheme was explicitly rejected.
- Layout composition is **free** — the user's words: *"the layout composition
  will vary as needed, I just want to imitate the feels."* The three-zone rule
  is therefore not sacred, but no slice may retire it without a stated reason.
- Product truth, `CONTEXT.md` vocabulary, and every backend contract stay
  untouched.
- Air-gapped: fonts ship in-repo as compiled-in bytes.

## Direction contract

<!-- impeccable:direction-contract 1 -->

Seed key `21929d2e` (`--scope direction --mode operate`). The roll assigned
candidate 5 (Saville catalog sleeve); the user's reference image **pins the
world**, and a brief-pinned direction beats the roll. The roll's challengers
were weighed and each left a named raise, listed under the contract.

**THESIS.** An instrument panel, not a panel of islands. Every surface the
operator reads is a hairline-framed field carrying tracked caps labels, dense
key/value rows, and quantities set in a tabular condensed face that never
reflows as the number changes. The category-default arrangement it refuses:
the floating chamfered "island" with a lit rim — chrome that reads as
furniture. Here chrome reads as an instrument the operator is looking *into*.

**OWN-WORLD.** Saira Condensed in three weights does all human work; PT Mono
does all machine output; every numeral is set with `tnum`. Panels are square,
flat, and separated by one 1px `CUT_GREY` edge — no chamfer, no cast shadow,
no bevel, no rim, no gradient. (The edge is Cut Grey rather than Hairline
Slate, and the reason is the tile map: the console floats over bright land and
dark water in the same frame, and a Hairline Slate hairline all but disappears
over pale land. Cut Grey holds on both, which is the job the old four-element
casing was doing.) A panel's title is a tracked-caps strip on Panel Slate
carrying the label left and one right-aligned machine readout, under a rule of
the same 1px. Depth is tonal only: Console Night → Panel Slate → Deep Well.
Radar Cyan is still the only accent and still means *live or yours*; it is the
edge of the one panel that owns input, at full alpha, and it is nowhere else.

**STORY.** The operator opens the console and reads one thing first: what state
the exercise is in and what is outstanding. Then the map, with the selected
unit called out. Then the instrument row. Every quantity they might want to
compare across panels — counts, ETAs, latencies, scores — lines up in columns
because the figures are tabular, so the eye tracks a column instead of
re-reading each row.

**FIRST VIEWPORT.** A full-width command band across the top, one row: exercise
identity and phase on the left, the clock and the link state as the two things
read at a glance, then the readouts, then the settings and sign-out verbs at
the right. Below it the map owns the whole window, with a reticle and tick
scale at its centre and the selected unit's callout drawn off its corner. The
working column docks left, each panel a hairline-framed field. At the bottom,
the instrument row: four panels — the readiness gate, the fleet register, the
live log, and the selected unit's inspector — each with a tracked-caps title
and dense tabular rows.

**FORM.** Chosen from the user's pinned reference image (a maritime
chokepoint-watch console), with three raises donated by the roll's challengers.
Its position on my ordered list: 1 of 7, which is why the reference beat the
roll rather than the roll beating it.

The three raises, each named for its donor:

- **From the split-flap concourse (challenger 1, verdict: wins):** *rows are
  live entities ranked by time, and columns never move.* Every list in the
  console becomes a ruled grid of fixed columns in one face, and a state change
  restyles the row without breaking the grid.
- **From the dark-first developer console (challenger 4, verdict: competitive):**
  *monospaced figures everywhere, panel headers in small caps, and panels
  persist so navigation changes their contents rather than the layout.* The
  console's frame stops rebuilding itself per state.
- **From the midnight transit diagram (challenger 2, verdict: declined on
  clarity — a real-world tile map cannot become a schematic):** *the chosen
  line burns brightest.* The selected unit's affiliation is the only one at
  full ink; every other unit recedes.

Declined challengers and what the direction kept from each: the Labanotation
score (challenger 6) donated *quantity as ink extent* — a readiness count is
also a filled bar, not only a number. The Saville catalog sleeve (the assigned
candidate) donated *nothing is labeled twice*. The acetate tab board
(challenger 3) donated the stepped fore-edge rail idea, held in reserve for the
five game states, not used yet.

**FINISH.** unreviewed and undocumented is unfinished; this build ends with the
finish review, the verdict, DESIGN.md, and every shipping raster carrying its
provenance.

## Memorable moment

The readiness gate read as one instrument: the outstanding count in a large
tabular figure, a filled bar whose extent is the same number, and the server's
own blocker sentences in PT Mono beneath it. One number, three encodings, no
label repeated.

## Slice order

Each slice is verifiable on its own and shrinks the risk of the next, the way
`docs/gpui-migration-handoff.md` already works.

1. ~~**Type and tokens.**~~ **DONE 2026-10-13.** Saira Condensed (four
   weights) and PT Mono vendored at `assets/fonts/` with both OFL licences,
   registered in `src/shell_gpui.rs`. The kernel owns the new scale
   (`FACE_HUMAN`, `FACE_MACHINE`, `LABEL_PT`, `BODY_PT`, `READOUT_SM_PT`,
   `READOUT_LG_PT`, `MACHINE_PT`, `LABEL_TRACKING`, `ROW_TRACKING`,
   `TABULAR_FIGURES`) with five tests pinning the ladder. The top band's link
   word and phase are tracked-caps stamps in their status ink, its readouts are
   machine rows, and every island title now goes through `panel_title()`.
   Verified by capture over the real Jakarta map: `.impeccable/review/`.
   `cargo test --lib view::` 33 pass. Four pre-existing failures elsewhere,
   unchanged.
2. ~~**Panel grammar.**~~ **DONE 2026-10-13.** The island silhouette is gone.
   Nine elements per panel — cast shadow, three-ring steel casing, top light,
   bottom shade, 2px rim at 3px inset, 12px radius, band scanline texture —
   replaced by one 1px edge. `PANEL_RADIUS` 0, `PANEL_EDGE` 1,
   `PANEL_EDGE_REST` Cut Grey, `PANEL_EDGE_OWNED` Radar Cyan at full alpha,
   `PANEL_BAND_RULE` 1, `PANEL_PAD` 10, `PANEL_GAP` 6, and `panel_edge(bool)`
   keeping ADR-0016's one-boolean single-owner discipline. Two kernel tests.
   Verified by pixels, not by the test alone:
   `.impeccable/review/tfg-slice2-crop.png` (resting — 0 cyan px on either
   edge) and `.impeccable/review/tfg-slice2-lit-edge.png` (pointer seeded over
   STEP — 1371 cyan px on STEP's edge, 0 on OPERATOR's). Exactly one lit edge.

   Also in this slice: `TFG_POINTER=x,y` was named in two comments and read by
   nothing, so the lit state had no capture path at all. It is wired now, next
   to the other capture seeds. A malformed or absent value is `None`, which is
   the real resting state.

   The egui console is untouched and still ships the old world — `chrome.rs`
   and `tokens.rs` keep every constant this slice stopped using in GPUI. That
   is correct until ADR-0020 step 5 deletes egui.
3. ~~**Tabular readouts.**~~ **DONE 2026-10-13.** `human_figures()` builds the
   `gpui::Font` carrying `tnum` (GPUI 0.2.2 has no `font_features` builder on
   `Styled` — the feature list only reaches a `TextStyle` through `.font(Font
   { .. })`, whose fields are public, so the wrapper is built at the shell
   boundary and the kernel keeps its plain `(&str, u32)`). `figure(value,
   leads_panel, ink)` is the component, sized through `view::readout_size`.
   The band's zoom readout is the first figure in the console, and `TFG_ZOOM`
   seeds it so the property is measurable rather than asserted.

   **Proven by difference images, not by the kernel test.** With the pointer
   and zoom seeded, `TFG_ZOOM=8` vs `9` differ in 200 px, all inside the
   digit's own 10px advance at x 266–275 — nothing in the band's control
   layout moved, so two different digits at the same count share an advance.
   `TFG_ZOOM=9` vs `11` differ in 505 px, 402 of them in the band spanning
   x 266–326: the second digit plus the `+` control pushed right by exactly
   one advance. Captures: `.impeccable/review/tfg-slice3-tnum-z9.png` and
   `-z11.png`.

   **Not yet placed: the 30pt leading figure.** `figure(_, true, _)` and
   `readout_size(true)` are tested and wired, but no panel body leads with one
   yet. `zone_islands` is a pure function of `GameState` and Review — the
   readiness gate, the one quantity that earns 30pt — appears only in
   `Planning`, which no capture seed reaches; the live session is in
   execution. Shipping an unphotographed panel body is exactly what this
   project's discipline forbids, so the composition is owed rather than
   guessed.
4. ~~**Panel-body grammar, proven on Step.**~~ **DONE 2026-10-13.** Three named
   units and nothing else: `note()` (prose, human face, body size),
   `section()` (tracked caps plus a hairline filling the rest of the row — a
   ruled break, not a heading with air under it), and `kv()` (label left in
   tracked caps, machine value right-aligned, hairline under). The body this
   replaced was eleven `text_size(px(11.0))` divs in a column with 4px gaps —
   a stack of paragraphs where nothing lined up, so the eye re-read every row.
   `kv` puts the values in a column the eye can track.

   The Step island leads with `0 of 1 READY` at `READOUT_LG_PT` — the 30pt
   figure `figure(_, true, _)` was built for. **Its ink is Map Ink, not a
   status colour.** A status colour at 30pt is the accent-spending failure this
   project already recorded once: in this environment the readiness count is
   outstanding on every frame, so a yellow figure would be the panel's default
   face and a default-state accent is an accent spent on nothing. Size carries
   the authority, the `READY` word and the `OUTSTANDING` row carry the state.

   **The height arithmetic, and the trap in it.** The Step island went
   380 → 500, and the intermediate values are recorded because each one failed
   differently:

   | height | viewport | measured | result |
   |---|---|---|---|
   | 380 | 350 | fits exactly, as shipped | — |
   | 460 | 430 | content 438 | **8pt clipped, `down=false` — unreachable** |
   | 480 | 450 | content 450 | fits exactly |
   | 500 | 470 | content 470 | fits, verbs are the tail |

   The 460 row is the trap and it is worth knowing: `content_h` is
   `max_offset + viewport`, so 438 = 8 + 430. An 8pt overflow sits *inside*
   `BODY_SCROLL_SLACK` (12pt), so `can_scroll_down` reports false — a clipped
   line with no scroll available. That is the silent-clip defect
   `docs/ui-overhaul-status.md` records, produced by arithmetic rather than by
   a layout mistake. The fix is the box, not the content: the body's 6pt gap is
   the console's own spacing rhythm, so trimming it to buy 22pt would trade a
   rhythm for a fit.

   One line was removed rather than fitted: the note under the Game Master's
   verbs. At ~511pt of body content against a 450pt viewport its last line was
   painting below the panel's own box — "a `max_rect` tells the layout where to
   stop; it does not stop the painter" — and the GAME MASTER label plus four
   disabled verbs already said what the note said.

   **Residual, stated rather than resolved:** the exact box fit is within about
   10pt of what I can measure cleanly here. The fit log reports
   `content=470 viewport=470 max_offset=0`, which is the toolkit's own
   measurement and the one I trust; my pixel arithmetic on where the panel's
   bottom edge falls is less reliable, because the column's bottom border does
   not register as a detectable edge in these captures. Everything an operator
   acts on — the figure, every row, DECLARE READY, the GM verbs — is fully
   visible in `.impeccable/review/tfg-slice4-step-body.png`.
5. **Map chrome — kernel and plumbing DONE, drawing GATED OFF.** 2026-10-14.
   Three kernel functions, pure and tested (40 `view::` tests, five new):
   `metres_per_pixel(lat, zoom)`, `scale_bar(mpp, min, max)`, `scale_label(m)`.
   At zoom 11 over Jakarta it returns `10 km at 131.6 px`, which is right.
   Three bugs were found by the tests: the ladder walked `20 -> 40 -> 80`
   because `20/5` divides exactly; "nearest the band's middle" oscillated
   (200km at z6, 100km at z7) because the nearer-to-middle rung was the
   smaller one; and one test asserted the wrong invariant — that a bar's
   distance grows with zoom, when it must shrink. It now asserts what matters:
   `metres / px` equals the map's true resolution at every zoom.

   **The camera is retained rather than discarded.** `MapResp` has always
   carried the centre and zoom; the binary's drain read them as `_at`/`_zoom`
   and threw them away, which is why this console never had a scale bar — the
   number was arriving every frame and nobody kept it. Verified arriving:
   `center=(-6.108, 106.91) zoom=11`. `FrameFeed` was widened to carry it. A
   replay reports an absent camera honestly (`ZOOM_MIN` is 3.0, so zoom 0 is
   unambiguously "no camera") and draws no bar.

   `paint_reticle` replaces the selection ring: four corner brackets, drawn as
   four paths because GPUI 0.2.2 has no per-corner border. Unverified by
   capture — this mirror holds `units 0`, so no marker renders.

   **`map_chrome` is gated off and returns `Vec::new()`**, with the bisect in
   its own comment. Everything about it is correct except getting the element
   on screen. It is gated rather than left half-working because the strips'
   backgrounds DO paint (measured, a 562-unit delta where the scrim lands), so
   an enabled version leaves two unexplained dark bands over the map with
   nothing in them — worse than no chrome. Verified no regression against the
   slice-4 baseline: worst per-point delta 0.

   ### The GPUI layout finding, and the pre-existing defect inside it

   An absolutely positioned element with an explicit `left` plus a `width` and
   **no opposing edge does not paint at all**. Evidence: a 200x40 solid red box
   at (400,700) rendered as bare map, across a build where `map_chrome` had run
   832 frames and provably pushed it.

   The **stretch idiom** — `.absolute().left(0).right(0)` plus a `top` and an
   `h` — paints the element's own background. It does **not** paint the
   element's children, which is the open problem. `top_zone` is built the same
   way and lays its children out fine.

   Tried and rejected, each recorded rather than guessed at again: `pl()`
   padding-left killed a strip outright; `pt`/`pr` killed another; `py`/`px`
   changed nothing; a flex spacer changed nothing; a nested `flex_col` inside a
   `flex_1` child changed nothing; flattening every child to strip level made it
   worse and stopped the scrims painting too.

   **The reproduction was built, and it PASSES — all ten variants.**
   `proto/gpui-spike/src/bin/layout_repro.rs` is a 760x420 window of labelled
   bands, each isolating one property. Every band paints both its label and a
   solid amber probe box:

   | | variant | result |
   |---|---|---|
   | A | stretch + `px`/`py` + flat children | paints |
   | B | stretch, no padding | paints |
   | C | stretch + `px` + `flex_1` child (`top_zone`'s exact shape) | paints |
   | D | stretch + `pl` only | paints |
   | E | in-flow, not absolute (control) | paints |
   | F | full-bleed absolute sibling painted first | paints |
   | G | full-bleed first, strip added last | paints |
   | H | `Vec<AnyElement>` from a fn, added in a loop | paints |
   | I | `py(26)` on `h(44)` — a negative content box | paints |
   | J | root with `on_mouse_move`, `render` returns `into_any_element()` | paints |

   So none of the properties blamed in the console bisect is actually at fault.
   The `pl`/`pt`/`pr` conclusions were wrong: those methods work fine here. What
   that leaves is an interaction with something in the console's root that
   these variants do not capture, and the strongest lead is that `band_el` —
   also an absolutely positioned direct child of `root`, added with
   `.child()` — paints its children perfectly, while a strip built the same way
   and added the same way does not.

   **Next attempt, and it is a different kind of job:** instrument the console
   rather than the shape. `proto/gpui-spike/src/bin/layout_repro.rs` is the
   scaffold for it — add the console's real `root` composition to it one
   sibling at a time until the strip's children stop painting. The sibling that
   does it is the answer. Do not re-run the shape bisect; it is settled.

   **A PRE-EXISTING DEFECT the bisect found: the zone grip does not paint.** It
   is built exactly like the red box — `.absolute().left(w).top(y).w(n).h(m)`,
   no opposing edge — and measured on a capture it puts 5–6 grey pixels per
   column at x 357–363, all at the panel seam around y 285–356, against a rule
   that should run the band's full height. **The console's dock-resize handle
   is invisible and its gesture is undiscoverable.** Same class of defect the
   console's own status doc keeps recording. Fixing that positioning idiom
   fixes the grip and this chrome at once.
6. **Instrument row.** Four panels across the bottom. **SUPERSEDED 2026-10-19
   by the commander's rail** — see "The commander's rail" below.

### What slice 1 did not prove

Two things are established by source and by cmap table scan but not by pixels,
and both are owed by a later slice rather than claimed here:

- **`tnum` reaches the shaper.** Read off gpui 0.2.2's Linux path (`FontKey`
  keyed on `(family, features)` → `load_font` stores it → `layout_line`
  applies it as shaping attrs). Nothing in the top band needs it — PT Mono is
  tabular by construction — so no surface exercises it yet. Slice 3.
- **The arrow glyph.** Saira Condensed covers U+2192 by cmap scan, and no
  surface in this capture draws one. The 32 `→` call sites are the thing that
  shipped as `.notdef` boxes last time, so they get a pixel check before
  slice 2 is called done.

### Capture hygiene, worth recording

A stale `Process crashed: tfg_gpui` notification sat composited over the
right end of the band in two consecutive captures. It came from an earlier
manual run being SIGTERM'd, not from the app: the binary runs clean
(`coredumpctl` shows the only recent `tfg_gpui` dump is `si_code: SI_TKILL`,
which is the capture script's own `pkill`). The trap it sets for anyone
capturing this console: a layer-shell notification cannot be dismissed
without clicking into the user's browser, so verify the band's left group and
crop around the right rather than chasing it.

## Command and Crew bodies

Converted to `kv` / `section` / `note`, plus two new units:

- `ruled(Vec<AnyElement>)` — a row under a hairline. The unit a LIST needs; `kv`
  is one label and one value, a roster row is a name plus two or three machine
  columns, and building that out of `kv` would be a table read sideways.
- `machine_col(value, w, ink)` — a fixed-width right-aligned machine column, so
  a column of them lines up down the panel.

Crew's state column now routes through the status palette (three inks) instead
of leaving a judge and an arrived-and-ready person the same grey. Command's hull
row lost its per-row cyan wash — that spent the accent on every row at once,
which is the same failure the island rim had; the state is now the last column's
ink and nothing else is coloured.

**A defect this introduced, found and fixed by capture.** The hull name sat in
`flex_1`, and a flex item will not shrink below its min-content width — so
`KRI Abdul Halim Perdanakusuma (KRI-355)` pushed the machine columns off the
panel's right edge: `0.1 kn` lost its `kn` and the state column was gone
entirely. `min_w_0` and `truncate` do NOT fix it; a definite `.w()` does. Worth
remembering for every future ledger row.

**Verified:** all four islands report `content == viewport`, `down=false` —
operator 142, step 470, command 390, crew 210. No body is clipped inside its own
panel, which is the failure slice 4 hit twice.

**Not pixel-verified:** the Crew person rows. The band is 373pt tall and the full
stack is ~1320, so Crew is culled unless scrolled — and `TFG_SCROLL` also feeds
the island scrolls, which distorts the measurements. Closing Step did not bring
Crew in either. The code is written, the island fits, and the `IN ROOM` header
renders; the rows themselves are still a gap.

**`TFG_ORGANIZER=1`** was added as a capture seed. `player` is the seat's
property, so a seated non-judge gets the player column and Command/Crew cannot
be photographed at all in this environment. The seed forces the organizer
column; absent, the real seat decides.

## Review body

Converted, and it introduced the readout unit Step had been building by hand:

- `verdict(figure, label, ink)` — a figure at `readout_size(true)` with its
  tracked label beside it. Step's `0 of 1 / READY` and Review's `0 / OUTSTANDING`
  are the same shape, so the second one got the name.

`TFG_PHASE=planning|preparation|execution|closure` was added as a capture seed,
for the same reason as `TFG_ORGANIZER`: the phase is the session's, and Review
only exists in Planning, so an executing session hid the panel entirely.

**Verified by pixel:** the panel renders with `OUTSTANDING` (the unpublished
branch), `ROOM KEY` / `KEY` / the key in PT Mono at 12pt inside a 300pt panel,
the share note, and `ADVANCE` with the `Enter execution →` button.

**Not pixel-verified:** the `verdict()` readout itself. The API is not running,
so the held session publishes no readiness checklist and only the
`ReadinessGap::Absent` branch is reachable. The `0 / READY` and `N / OUTSTANDING`
readouts are written, compile, and are unexercised — the same class of gap as
Crew's person rows.

One thing the capture shows that is NOT this slice's work and should be looked
at next: `Enter execution →` is the panel's only verb and it is a full-width
grey slab several lines deep. Every other surface here is a 22pt pill.

## The verb pill

Fixed in `button()`, so all 33 call sites at once. It had two halves and the
capture found both:

1. **Full-width.** The element lands as a child of a `flex_col`, whose
   `align-items` is `stretch` by default, so an unstyled button fills the
   column's width. `Enter execution →` was a slab across a 300pt panel. GPUI
   0.2.2 has no `align_self`, so the stretch can only be removed from outside —
   a row does not stretch its children. Measured: 100% of the content column
   before, **49% after** (`Sign out`, 138pt wide).
2. **Twice too deep.** With egui's `BUTTON_PADDING` (12 × 8) and no line height
   of its own, a rendered pill measured **48pt** against the reference's **22pt**
   — 2.2× too deep, which is what made the verb read as a slab rather than a
   verb. Padding to 4 and a line height tied to the face size (`size * 1.25`)
   rather than left to the shaper's default gives **22pt exactly**, measured.

No call site wanted a full-width button — all 33 are labelled verbs.

**Still open, and it is a grammar call rather than a bug:** the reference labels
its pills in tracked caps (`SIGN OUT`, `APPLY`); this console uses sentence case
(`Sign out`, `Apply`). `label_case()` already exists and the tracked-caps labels
read as instruments, but changing 33 labels at once is a decision about the
whole surface, not a fix.

## The zone grip — attempted, NOT verified

`zone_grip_rect` specifies a 10pt-wide, full-band-height rule straddling the
zone's inner edge. It does not appear. That makes the dock-resize gesture work
with no affordance — a pre-existing defect, not one this overhaul caused.

**Fixed, but not confirmed:** the rule was `h_full()` *and* `mt(8)` *and*
`mb(8)` — 100% of the grip's height plus 16pt of margin, which overflows the
flex cross-axis and resolves to a few stray pixels. It is now an explicit
`h(box_h(grip_box) - 16)` with `flex_shrink_0()`. That removes a real bug; it is
**not** evidence the rule is visible, because the follow-up capture did not find
a solid vertical rule anywhere near the expected x.

**Why it stalled:** the grip's x comes from `band.2` in `zone_band_at`, and three
attempts to locate the band's inner edge by luminance scan disagreed — the third
put the panel/map boundary at x≈479, which contradicts what the captures show.
The measurement method is not reliable against this scene, and guessing at crop
windows is what has consumed the budget.

**Next attempt, and it should start differently:** derive the expected x from the
source (`zone_w` at the time of the capture, logged) rather than inferring it
from pixels, then crop a 20pt window at that x for the full band height. If the
rule is absent there, the element's placement is the problem — it is a sibling of
`band_el` on the same parent, using the same `left + top + w + h` idiom that
`band_el` proves works, so placement should be ruled out by construction before
anything else is changed.

## Verb labels: tracked caps

Done, on the reasoning that a lowercase label on a graphite pill reads as a form
control rather than an instrument. Applied in `button()` so all 33 call sites move
together rather than one at a time. Verified: `SIGN OUT`, `+ NEW SESSION`,
`COMMS`.

**The zoom readout stays lowercase, and that is correct, not an oversight.** `z11`
goes through `figure()` (tabular figures), not `button()`, so the caps rule does
not reach it. The distinction is the point: a VERB is a label and takes caps, a
READING is a figure and takes tabular numerals. Two different kinds of thing.

**Tracking is still owed.** `tracked()` buys letter spacing by splitting a label
into one div per character, which is right for a section header and would put
hundreds of nodes on a row of pills. Uppercase carries most of the effect; the
spacing needs either a real `letter_spacing` on `Styled` (GPUI 0.2.2 has none) or
a per-character span reused deliberately at this scale.

## The instrument row has no egui counterpart

Checked `src/main.rs` and `src/chrome.rs` rather than assuming. The egui console
has **no bottom instrument row**: its layout is a top band, a scrolling side-zone
column (`zone_island_origins`, one stack), and floating modals over the map
(`chrome::modal`, `spec.rect_in(viewport)`). That is the same three-zone
composition the GPUI shell already mirrors — which is why the GPUI port tracked it
island for island.

So the reference's four-panel bottom row is a NEW composition, not a port. There
is nothing in the egui version to copy it from, and it still needs the "which
four, holding what" decision before any code. It is also the one slice that
touches the map's lower band, where `map_chrome` is still gated — so it inherits
that defect unless the strip problem is solved first.

## The commander's rail

Slice 6 above planned FOUR panels across the bottom: the readiness gate, the
fleet register, the live log, and the selected unit's inspector. The
execution phase is not that room. It is a commander's room, and the console
knows one thing about it: the seat holds a hull. So the bottom band is one
instrument for that seat, and it carries what that seat's job needs —
which hull, where it sits, and the helm that moves it.

**What the four panels were for, and what replaced each.** The readiness gate
and the fleet register are the ORGANISER's and the PLANNER's readings, and
the execution phase's own widgets already carry them: the top band states
the phase and the room's readiness, the right column states the crew, the
scenario and the comms. The live log is the console's own record, and it is
a panel of its own rather than a channel of a rail. The selected unit's
inspector is the one that survived, and it became the rail's first channel.
**So the four panels are not retired: three of them were never the
execution phase's bottom band, and the fourth is the rail.**

**The rail is a RAIL.** Opaque, square, flush with the window's edges, one
1px `CUT_GREY` rule along the top. The first version of this was a rounded,
translucent panel floating eight points off the bottom, and the tile map's
own labels came through the type that sat on them. A console read across an
exercise room, under glare, cannot put its readings on top of a map and
call it legible — and the world this brief already committed to says it
plainly: *"Panels are square, flat, and separated by one 1px CUT_GREY edge
— no chamfer, no cast shadow, no bevel, no rim, no gradient."* The rail is
that rule applied to the bottom of the window.

**Three channels, and the slack goes to the control.** The picture and the
name; the identity rows (hull number, group, embark) as a right-aligned
column; and the helm. A course is a circle, so it is a compass; a speed is a
range, so it is a graduated scale. The scale is the one element that gets
BETTER with length, so it takes the window's slack, and the compass and the
scale are the last things to shrink on a narrow window.

**The instruments settle.** A needle that teleports reads as a glitch, so
the compass eases onto a new draft and the scale's fill eases with it — the
short way round the compass, and never during a pointer drag, because while
the operator is holding the control the instrument IS the hand.

Verified by capture against the operator's own deployment at 1920 × 1150 and
760 × 620, with the drags driven: the compass's needle and its readout agree
(they did not before — the canvas was measured at zero height, so the face
collapsed to a smudge and pointed at a bearing the readout denied), and the
scale's fill, thumb, graduations and figure all follow the pointer. The write
itself was verified against the local stack, where `SET HELM` drew a real
refusal from the server and the rail showed it. The build log is
`docs/ui-overhaul-status.md`, "The commander's rail, and the compass that
pointed the wrong way".

**Open, and not this slice's work:** a seat that is not an administrator
cannot sign in — the sign-in reads the whole unit register, which the server
answers 403, and reports the refusal as a wrong password. The commander
seat this rail is built for is the seat that cannot get in. The answer is a
product decision about what a non-administrator's sign-in shows, so it is
recorded rather than changed.

## The visual integrity pass (2026-10-20)

The direction above was written and unit-tested and never audited against
the running console. An audit of all 26 surfaces — by capture, against the
real binary on the compositor, with the pixel values measured rather than
eyeballed — found that the console had drifted away from its own contract
in two directions at once, and that the token layer was the only thing
holding.

**The finding, stated once.** The console drew **two design systems in one
window**: the islands and the modals in the instrument language, and the
execution widgets, the sign-in card, the top band and the two island bodies
that had no padding in the old floating-card language. Everything else —
fourteen text sizes, ten corner radii, three body paddings, a status line
in body silver where the comment above it said "this is exactly the line
that must never be grey on grey" — was drift on top of that.

**The audit** is `docs/visual-integrity-audit.md`. **The fixes** are its
"what was fixed" table. The parts of it that are worth carrying into the
next slice:

- **The token layer was sound; the drawing was not.** Nine of `view`'s
  values were defined and never used (`PANEL_GAP`, `DATA_AMBER`,
  `ROW_TRACKING`, `ZONE_W`, `ISLAND_PAD`, …), and the shell hardcoded its
  own versions of the rest. So the fix was never "add a token" — it was
  "draw with the tokens".
- **A type scale needs a role for the dense tier.** The scale is now
  `LABEL_PT` 10, `DENSE_PT` 11, `MACHINE_PT` 12, `BODY_PT` 13,
  `READOUT_SM_PT` 18, `READOUT_LG_PT` 30. `DENSE_PT` is new: the side zone
  is 320pt wide, and a row that holds a call sign, a name, a role and a
  state does not fit at 13pt. A console that has no dense tier invents
  one, and this one had invented nine.
- **A status line needs to carry its KIND, not its words.** `Status` is the
  words plus a kind, and the kind decides the ink: red for a refusal,
  green for a write that landed, sand for one in flight, silver for a fact.
  The kind is set where the write happens, because that is where the truth
  is. Guessing it from the words at the far end of the file is what left
  every refusal in this console drawn in body silver.
- **One selection treatment.** A chosen item is a 1px cyan edge and the
  bright ink. Never a filled slab: the drill tabs were a full cyan pill and
  the role pills were a cyan outline, and a console with two selection
  languages is a console where the operator cannot predict what a thing
  does.
- **The accent is one thing.** It is live-or-yours and nothing else: the
  link state, the focused field, the panel that owns input, the operator's
  own pick, the live needle on the dial. It is out of "SEATED", the role
  labels, the row wash, the wordmark, the sign-in button and the step
  card's bar.
- **A panel is a panel.** The modal lost the chamfer, the execution
  widgets lost the translucency (the map's place names were legible
  through them), the sign-in card lost its radius, and the top band is
  opaque `CONSOLE_NIGHT` instead of a 47% black scrim over the map.
- **The dead control.** The Command island's course and speed steppers
  were drawn and pointed at ids nothing listened to. They now write the
  same draft the rail's compass writes, and the same `SET HELM` sends. A
  control that looks live and is not is a lie about the console.

Two items were left, both named in the audit: the **basemap** (a daylight
`liberty` style under a night console — a product answer, not a patch) and
the rail's **empty picture channel** where a hull has no photograph.

**And one correction to the method, which is worth more than the fixes.**
The first pass of the audit ran against the LOCAL Minos stack, and the
local stack is a throwaway database with a fixture in it: hull names of 20
characters, one exercise, one room. It made the console look tidier than
it is. Re-run against the **hosted dev API** — the deployment the operator
actually uses — two more defects appeared, both invisible in the fixture's
data and plain in the real one:

- the **rail's picture channel** printed the register's whole label, name
  AND hull number, while the identity channel one to the right already
  states the hull number on its own row. With a 31-character name the
  channel overran its own edge. It now prints the name alone.
- the **Command island's HULLS row** was one line — name, course, speed,
  state — in a fixed 318pt budget that was the column's old 360pt width
  minus a guess, so a real hull name was cut in half. And this toolkit's
  `truncate` clips a div-wrapped string **without an ellipsis** (measured:
  a 60pt probe clipped "KRI Abdul Halim Perdanakusuma" at "KRI Abdul Ha"
  and drew no "…"), so a name can never be shrunk into a fixed column
  here. The name has a line of its own now, at the panel's full width, and
  the three readings keep their columns beneath it.

The lesson is the harness's: `scripts/gpui-capture.sh` already exported the
hosted superuser's credentials, so the hosted API was always the default
and the local stack was an override in the sweep. A fixture is a fixture —
it answers "does this code draw" and not "does this console survive the
data it will actually be given".

`DESIGN.md` is now marked superseded at the top, with a pointer here and to
`src/view/mod.rs`: it is the egui-era document, and it disagrees with the
console about the chamfer, the typeface, the radius, the panel language and
the meaning of the accent.

## The docked sidebar (2026-10-20)

The column was a stack of cards floating over the map, and the operator
said the thing it was: *make the left islands sidebar docked on the left,
and connected with each other.* Three changes, and each one is the
direction above taken literally rather than a new idea.

**DOCKED, not floating.** The column now starts at the window's left edge
and runs to its bottom edge — flush the way the top band is flush, with no
margin of map above it or under it. It was inset 24pt on the outer edge
and 22pt under the band, and it was inset under the band in the three
phases with no strip to put there, which is the tell: that gap was not a
margin, it was a leftover. A sidebar that stops 24pt short of the window's
bottom is a card that happens to be tall. The right dock mirrors it.

**CONNECTED, not stacked.** The fields touch and share a rule. Every field
draws the rule BELOW it and only the first draws the one above, so the
rule between two fields is one line and the column reads as one surface
with fields in it. The 2pt gap it used to carry put the MAP between two
fields of the same panel, and a map-coloured hairline under a field is not
a divider, it is a mistake.

**The outlines are drawn in a second pass, over the whole stack.** The
first version of this drew the rule with the field, under its own
content, and the pixels said otherwise: the edge measured 40% of its
strength on the right and NOTHING along the bottom, because the next
field's band painted over it. A 1px rule that is 40% grey is not a rule,
and a rule that is invisible is not the state language either. So the
column paints the fields first and the outlines after, over the whole
stack, and the rule between two fields is drawn once, by the field above.
What is left under the content is the field's hitbox — the shield that
stops the map behind the panel taking the pointer — and a shield is not
supposed to be seen.

The token layer's own note on `PANEL_GAP` had argued the other way — that
flat panels 2px apart read as ONE panel with a seam in it — and it was
right about the seam and wrong about the fix. The seam was never the gap;
the seam was two 1px edges with map between them. Close the gap and the
two edges become ONE rule.

**The owned field still closes.** In a joined column a field's top edge is
not its own — the rule above it belongs to the field above, and that field
is not the one that owns the input. So an owned field that is not first
draws its own 1px cyan line over that rule, and its perimeter closes
instead of reading as a "U". The lid is painted in the same second pass,
after the rule it covers: the field that owns the pointer is always BELOW
the field whose rule it takes, so a lid painted in the field's own pass
would be covered by the field above.

**And the wheel belongs to the chrome.** The map's surface is a
full-window hitbox, and GPUI's scroll hit test collects EVERY hitbox under
the pointer rather than the path to it — so a wheel over the column, the
top band, the rail, the widgets or a modal reached the map's handler and
ZOOMED the map while the operator was scrolling a panel. The map answers
gestures made over the map; the chrome swallows its own. Every piece of
chrome now stops the propagation, and the column's handler claims the
gesture for the column.

That one is a regression test, and it is the kind that had to be: the
console's own test suite passed while the map was zooming under the
operator's hand, because every test of it called `zoom_by_wheel` directly
and never went through the dispatch that decides WHO gets the wheel.

**And the pixels are asserted, not looked at.** `scripts/shot-assert.sh`
answers "is there one rule between these fields" with a number, reading
the console's palette out of the token layer so an ink that moves cannot
leave the assertion quietly asserting about the old colour.
`scripts/gpui-sweep.sh` runs those assertions after each capture and
exits non-zero when one fails, so a sweep is a test rather than a folder
of pictures. The same harness reports what reached the console while the
picture was being taken, because a capture that caught the operator's own
wheel is a photograph of a state nobody asked for — and, on 2026-10-20,
that is exactly what one of them was.

## The sign-in, overdriven (2026-10-20)

The signed-out surface was the one thing in this console that was not an
instrument: a rounded card inset 24pt on two sides, floating on a flat
`CONSOLE_NIGHT` void, with the old world's furniture still on it (a radius,
a shadow-less bevel, a web form's hint words inside its boxes). It was also
the FIRST thing every operator saw. Everything below is the same decision
taken twice — the theatre, and the instrument — and the two are one change.

**THE THEATRE. The console's own map, live, under a flat night scrim.** The
map thread runs from the launch, so the picture is already on screen when
the operator arrives: the sign-in happens over the water the exercise will
be fought on. The scrim is flat `CONSOLE_NIGHT` at 0.86 — no gradient, no
blur, no glass — because the map is a GHOST under the console's own night,
and a ghost that competes with the panel in front of it is noise wearing a
costume. Measured: the ghosted land reads `(47,53,69)`, the panel's own
`CONSOLE_NIGHT` body reads `(15,23,42)` — the chrome is DARKER than the
map under it, which is the same relationship the console's panels have over
the live map.

**And the theatre ARRIVES.** The console opens on a flat night field and the
coastline resolves out of it: the scrim lifts from opaque night to the
ghost over 0.9s, on an exponential ease-out (the same curve the helm's
instruments use). It is tied to the PICTURE and not to a timer — the clock
starts when the first frame lands — so the theatre never fades into a map
that is not there. Verified by the log, not by the eye: the alpha walks
255 → 219 over 93 frames and settles. The scrim only ever LIFTS; a scrim
that fell back would be the map going dark under the operator's hands, and
that is now a test (`the_theatre_arrives_from_night_and_settles_on_the_ghost`).

**THE INSTRUMENT. The sign-in is a MODAL of this console, centred on the
window, over the theatre.** Square, flat, one 1px `CUT_GREY` edge, a
tracked-caps title on a `PANEL_SLATE` band over a body on `CONSOLE_NIGHT`,
and the console's own form grammar inside it: two credential fields, the
verb, a status line. It was a panel docked to the window's top-left corner,
and the operator called it too much, and they were right — a door is a door,
and a door is in the middle of the wall it is in. A card in the corner is a
card that happens to be tall.

**The credential fields are the console's own form order**: a tracked-caps
label over a well (the composer's fields are label-above-input too), the
value in the MACHINE face because a credential is an id and an id is a
machine value in this console, and an empty field carrying the console's
dash for an absent value rather than a hint word — the label above already
says what the field wants. The submit stays a full-width graphite slab (the
one place a console control is allowed to be a slab) but is SQUARE now, and
it BRIGHTENS to `MAP_INK` when both credentials are there: the verb is ready
to be sent, and the console says so with ink rather than with the accent,
because the one cyan edge on this surface is the field that owns the
keyboard.

**THE SCOPE. The space the readouts took is decoration now.** The
PRE-FLIGHT is gone — a clock, a frame count, a camera and a picture size are
a DIAGNOSTICS panel, and an operator who has to type a username does not
need a stopwatch, a camera bearing and a frame size before the door opens.
In its place, under the form, is a **scope**: the console's own compass face
(rim, two rings inside it, twelve graduations, a north wedge, a needle) drawn
in greys, because the one accent on this surface belongs to the field that
owns the keyboard. There is no figure on it and nothing to read off it. It
is decoration, and it is decoration of the console's own kind: a chart
room's door has a scope on it, and the water the exercise will be fought on
is right there under the card. The title row's far end carries the other
ornament — nine hairlines of graded length, the way a chart's margin is
graduated.

**And the surface got smaller in every other way.** The clock is gone, and
with it the sign-in's own repaint: the map renders on a gesture, so a clock
driven by the map's frames would have needed its own drain to keep ticking
past the theatre's settling. A surface with no clock has no reason to ask
for frames, and the drain loop is gone with it.

**And the console says one thing once.** The scope's caption carries the
STATE (`signing in…`) and the line under the fields carries the EVENT, and
only one of them is ever talking: the busy status line is suppressed, because
the caption already says it. The status slot is RESERVED rather than
conditional — a card that grows a line when the server answers moves the
scope under the operator's hands, and decoration that moves is a bug in the
decoration.

## The sign-in, again (2026-10-20)

Three changes to the same card, and the reason for them is one thing: the
card was carrying a NAME, a CURSOR and a VERB, and it was carrying all three
as afterthoughts.

**THE WORDMARK.** The name is now the largest text on the console, and the
only text allowed to be large. It is set in **Cinzel** — a Roman-capital
inscriptional serif, the letterforms of an engraved plate — tracked at a
sixth of its size, which is the gap a stone-cutter leaves between letters and
the thing that makes an inscription read as an inscription rather than as a
word in a bigger size. Cinzel is a THIRD face and the design system's rule
says two faces and no third; the rule stands for the console's TEXT, and
this is the one place where the text is a name rather than a reading, so the
face is spent on six letters and on nothing else.

**THE CARET.** The credential fields are hand-rolled — GPUI 0.2.2 ships no
text field, only the input plumbing — so the console draws the cursor too.
It stands after the value, in the focused field only, in the accent, and it
BLINKS: a steady bar reads as a character the operator typed, and a blinking
one reads as a cursor. The blink is the console's own tick, not the
system's, so it is the same beat on every machine, and the caret is lit the
moment a field takes the keyboard, because that is the moment a cursor
which happened to be dark would most easily be mistaken for a field that has
stopped accepting input.

**THE SCOPE IS THE VERB.** The plain SIGN IN slab is gone. The scope is the
control: the operator presses the radar, and the radar is the only thing on
the card that can be pressed. It says everything the slab said and more. It
BRIGHTENS when the door is armed — both credentials are there — and it LITS
while the round trip is out, and both are tweened, because a radar that jumps
from grey to lit reads as a different picture rather than as the same
instrument changing state.

Those are two facts, so they are two channels, not one colour spent twice:
`lit` is the link being out, and `bright` is the door being armed. A console
that spends one colour on two states spends the colour on neither, and an
operator who cannot tell "armed" from "waiting" cannot tell whether the
press will do anything. The caption under the scope is a CAPTION and not a
button — tracked caps in the console's own label style, with no slab and no
box — and it is part of the scope's pressable group, so the two are one
control. A verb nobody can read is a verb nobody will press.

**The card's own left edge is one margin.** The wordmark stands on the same
left edge as the field labels under it, because a name that hangs over the
edge of its own card is a name that was pasted on rather than engraved.

**`TFG_SIGNIN=busy|refused`** stages the card's own state for a headless
capture, the same seed-the-state precedent as `TFG_SETTINGS`. Both are
states the operator sees and neither can be reached by a launch that never
presses the verb.

Verified by capture over the real binary on the compositor, against the
hosted map:
`.impeccable/review/signin-2026-10-20/signin-resting.png`,
`-credentials-ready.png`, `-busy.png`, `-refused.png`, `-signed-in-console.png`.
`cargo test --lib`: 619 pass, the same four pre-existing failures. The
mechanical detector over the changed file: no findings.

**The signed-in console, photographed afterwards.** The hosted API did not
answer during the pass above, so every launch fell back to the fixture
roster and the console never left the card. That was transient: with the
API up, the signed-in console is
`.impeccable/review/signin-2026-10-20/signed-in-console.png`, and the
whole signed-in surface was re-shot — the five phases and the three
modals — with `scripts/gpui-sweep.sh` running the pixel assertions over
each one. The column's edges, the owned field's cyan perimeter and the
one-rule-between-fields all still hold, so the rewrite of the signed-OUT
surface did not disturb the console that comes after it. The one thing
this pass learned about the harness: a capture whose log says the console
fell back to its fixture is a picture of the FIXTURE, and the harness now
says so out loud instead of letting it pass as a picture of the
deployment.

## The unit details panel (2026-10-20)

**THE ASK.** As a Game Master and an observer, point at a hull and see what
it is. The console had an Inspector dock — a panel of readouts that only a
capture could open, showing the selected row's class, id, position, course,
speed, trail and state, and nothing else. There was no way to select a
hull from the MAP: the markers are painted into a canvas, so they have no
hitbox, and a press on a ship landed on the map surface and panned the
chart.

**THE CLICK.** A press on the map resolves the hull under it. The pick is
decided at the PRESS and undone if the press turns into a pan, because GPUI
delivers the release to whatever is under the pointer: a release that lands
on a panel never reaches the map, so a release-time pick would be a pick
nobody made. The radius is 30 WINDOW points, converted through the frame's
own scale, so a hull is as easy to point at at one zoom as at another. A
press on open water picks nothing; a press on a hull selects it by its own
id — an id and not a roster index, because the roster changes under the
panel — and the release opens the panel on it.

**THE PANEL.** Three ladders in one dock: what it is (type, class,
commander, embarkation), where it is (position, course, speed, state,
trail), and the figures the register published for it (length, beam, draft,
displacement, speed max, cruise, range, turn rate). Every figure is a fact
the console already holds and every fact it does not hold is a DASH. The
console never guesses: a hull with no published specification says so, a
hull nobody asked about says `—` for its embarkation rather than claiming
it is on the water, and a hull with no wire fix says `no fix` rather than
drawing a position.

**WHERE THE FACTS COME FROM.** The register names the hull and its class;
the exercise's piece says who commands it and what class the server
published for it (the backend used to DROP that field, and a dropped class
is a class the console can only guess at); the wire says where it is and
how fast; the store's spec mirror holds the published figures, read once at
boot — and the figures are asked for again, on demand, in the section below.
The embark read names a HOST's passengers, so the link is read the
other way round — the panel walks the hosts it knows and finds the one
carrying this hull.

**THE DOCK HOLDS THE POINTER.** The map surface is a full-window hitbox,
so a press on the panel's CENTRE would also reach the map: it would pan
the chart and pick whatever hull was under the button. The dock truncates
the hover path below it, the same way the column's islands do.

Verified by capture over the real binary against the hosted API:
`.impeccable/review/unit-details-2026-10-20/unit-details.png`.
`cargo test --lib`: 622 pass, the same four pre-existing failures. Three
new tests: the hit test, the click end to end through GPUI's own event
path, and the drag that starts on a hull and selects nothing.

## The figures come off the wire (2026-10-20)

**THE GAP.** The panel's figures and its TYPE came from the LOCAL STORE, and
the store is synced only at BOOT. So an operator who signs in through the
console's own card — rather than with credentials on the command line — saw
a dash for every figure and no type at all, until the next launch. The
class, the commander and the embarkation were never affected: those come off
the wire.

**THE FIGURES ARE ASKED FOR.** When a unit is selected the shell asks the
binary for that hull's published specification (`ToBinary::ReadSpec
{ unit }`), the binary calls `GET /units/{id}`, and the answer comes back on
its own feed. The panel fills in when it lands. The store's mirror is still
read at boot, so a launch with credentials on the command line shows the
figures immediately, and the API's answer is authoritative and overwrites
it. The console still never invents a figure: a hull with no published
specification says so.

**THE TYPE COMES OFF THE WIRE TOO.** The register's own read (`GET /units`)
publishes each hull's class summary, and that summary carries the class's
TYPE (`unit_class.unit_type.name`). The backend used to drop it, so the
console could only guess the type from the local taxonomy. The roster row
carries the type name now, and the panel reads it from there when the drill
tree does not know the unit.

**AND THE EMBARK READ IS ASKED FOR — OF EVERY HOST.** The embark read is per
HOST: it answers "who is inside X". So the panel asks for every host in the
exercise, not only for the hull that was pointed at, and both directions
follow: "carrying N" is the hull's own read, and "aboard X" is the read of
the host carrying it.

**"ON THE WATER" IS A CLAIM, AND THE CLAIM WAITS.** A hull that is aboard a
carrier the console never read is not a hull on the water. The console says
"on the water" only once every host in the composition has been read, and
says a dash before that. A Game Master has no seat and commands no group, so
the commander's own read never runs for one: the panel's read is the panel's
claim, and the claim is the console's to keep honest.

## The scenario composer, in the console's own grammar (2026-10-21)

The composer was a modal that read like a web form: a box that said
"Scenario name" INSIDE it, a label beside the date field with the calendar
button jammed against the input, a sentence under the pin explaining how to
drag it, and a card 250pt taller than anything in it. None of that was the
console's grammar, and the console's grammar was already written down.

**THE FIELDS ARE LABEL OVER WELL.** The sign-in's rule, applied to the
composer's fields: a tracked-caps label over the well, and an empty well
carrying the console's dash rather than a hint word. The label above
already says what the field wants, and a hint inside the box repeats it —
so "Scenario name" inside the box and nothing above it was a form the
operator had to read twice. The dash is `IDLE_GREY`, the sign-in's own
absent-value ink.

**THE VERB RIDES THE LABEL ROW.** The calendar and the time grid open from
the right of their label, across a hairline — the console puts a verb there
(the STEPS section carries CLOSE on its rule), and a verb beside the well
crowds the value it fills. The pin rides the step maker's label row for the
same reason.

**THE PROSE WENT.** "A pin quotes the map: DRAG it onto the water…", "HHMM,
or leave both empty" and "posts at once — the composer's own scenarios are
staged" are hints about controls, and the controls now SAY what they are:
the pin is a pin, the window's label carries its format, and the row that
posts at once says so in its own title. What stayed is the one sentence
that carries a RULE rather than a hint — the window's landing on the first
step's start and the last step's end — and it lost the echo of the times
the operator had just typed.

**THE CARD IS AS TALL AS ITS CONTENT, AND THE COLUMNS FILL IT.** The card
was 720pt with 470pt of content and a hole under it. Now the card's body
is a row that fills: the left column scrolls the form and the scenario
list, and the right column's step list scrolls while the maker stays pinned
at the bottom. The card's height is still a number, because a card that
clips its own tail is a form whose last verb cannot be reached — but the
number is the FORM's height now, not a number that suits one state and
wastes the rest.

**THE LISTS ARE RULED.** A scenario row is its name on top and its figures
under it — where it came from, how many steps it has, its window — with a
hairline under the pair. A step is the same shape. The rule is what makes a
list a list; without it the rows are a stack of lines.

**AND THE TAG STOPPED SPENDING AN ACCENT.** "this game", "saved" and
"draft" were the console's yellow — an alert ink on a label, beside a
scenario name that was already cyan. They are the label ink now, and the
cyan is on the name alone.

**THE PICKER LANDS IN ONE PLACE.** It opens under the window's four fields,
whichever field it fills, so the operator never hunts for the grid they
just opened. It is inline because GPUI 0.2.2 has no popover, and inline
because a floating layer over the map is a layer the map's own gestures
would fight with.

Verified by capture over the real binary: `scenario-list.png` (the form and
the list), `scenario-game.png` (a scenario this game holds, open in the
steps column), `scenario-date.png` and `scenario-time.png` (the two grids
inline), `scenario-empty.png` (nothing open). `cargo test --lib`: 780 pass,
the same two pre-existing failures.

## Unresolved

- The details panel's figures and its type no longer wait on the boot-time
  store sync: both come off the wire, and the panel asks for them itself.
  One gap remains. The figures come from the API's CURRENT specification
  version, so a version pinned by a game being judged is not what the panel
  shows — and the panel says which version it read nowhere, because the
  dock has no room for it.
- Whether the bottom instrument row survives contact with a 1040x640 window,
  which is the app's hardcoded launch size. Measured, not assumed.
- The sign-in's theatre is the basemap at the console's DEFAULT camera (the
  map's own `MAP_CENTER`, at `MAP_ZOOM`), not at a session's camera — there
  is no session before the sign-in, so there is nothing to aim it at. An
  operator who signs in somewhere else finds the theatre was Jakarta all
  along. Whether the sign-in should aim at the last session's ground is a
  product answer, not a drawing one.
- The basemap under the theatre is still the DAYLIGHT `liberty` style the
  visual-integrity audit named as a product answer rather than a patch. The
  scrim makes it a night map at 0.86, which is a surface answer, and the
  audit's answer is still owed.
- `FontFeatures` reaches the shaper on Linux through
  `load_family → load_font → LoadedFont.features → layout_line`. Verified by
  source. Not yet verified by pixels — that is slice 3.
- The `tnum` path is keyed on `(family, features)`, so a face registered twice
  with different features gets two `LoadedFont` entries. Harmless at this scale,
  worth a glance if the type scale grows.
