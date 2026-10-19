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

## Unresolved

- Whether the bottom instrument row survives contact with a 1040x640 window,
  which is the app's hardcoded launch size. Measured, not assumed.
- `FontFeatures` reaches the shaper on Linux through
  `load_family → load_font → LoadedFont.features → layout_line`. Verified by
  source. Not yet verified by pixels — that is slice 3.
- The `tnum` path is keyed on `(family, features)`, so a face registered twice
  with different features gets two `LoadedFont` entries. Harmless at this scale,
  worth a glance if the type scale grows.
