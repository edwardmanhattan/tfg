# MEMORY

Session handoff. Read this first, then `/tmp/opencode/arena-nato/SYNTHESIS.md`.

## The task

The user asked for NATO military map symbology to be implemented properly in
`tfg`, pointing at the Wikimedia gallery, and said they were fine with a
complete overhaul if the current system was insufficient. It was.

## Repo state

**The tree is dirty and the user's own work is uncommitted.** `src/main.rs`,
`src/store.rs`, `src/map_render.rs`, `src/groups*`, `src/backend.rs`,
`src/sim.rs`, `src/log.rs`, `CONTEXT.md`, `examples/calibrate_projection.rs`
carry ~3180 lines of the user's pre-existing work. **Do not revert, stash, or
reformat them.** The symbology work adds on top and touches none of it.

Also untracked and the user's, not mine: `MEMORY.md` (this file) and
`docs/research/nato-symbology-page.md`.

Three commits of mine exist on `main`, all scoped to `src/symbology/`,
`assets/symbology/`, `licenses/` and `proto/p5-epaint/src/main.rs`:

- `21a325f` the APP-6C symbology core, a generator and a proof loop
- `fc2d5ec` the icon geometry ported from the MIT tables
- `8dffad1` FINDINGS §6: the canonical frame geometry, and the one shape not
  to copy
- **(uncommitted) Unit 3** — the frame moved into `symbology/frame.rs`, with
  the canonical outlines, the cubic quatrefoil and the one-rule icon radius.
  Two defects found on the way, §7 and §8 below.

Nothing of the user's is committed. If you commit, stage by path.

## What is done and independently verified

### Unit 1, the layering move. Done, verified.

`Affiliation` and `BattleDimension` live in `src/symbology/mod.rs` and
nothing else. `src/symbology/` imports only `std` and `super::`.

### Unit 2, the generator. Done, verified, and then REWRITTEN by Unit 2b.

```
src/symbology/svgpath.rs        889   real SVG path parser, lexer, flattening
src/symbology/generate.rs     ~1500   join, invariants, emitter, tests
src/symbology/icons.rs          190   Point, IconMark, Fit, thresholds
src/symbology/icons_generated.rs       GENERATED, 25 icons, do not hand-edit
assets/symbology/icons.tsv              the SELECTION: 25 rows, hand-authored
assets/symbology/milsymbol.tsv          the GEOMETRY: 687 keys, GENERATED
assets/symbology/extract-mjs.mjs        the extractor, pinned commit
assets/symbology/render-icons.py        renders the generated table to SVG
assets/symbology/FINDINGS.md            what the port found, with numbers
licenses/milsymbol-LICENSE.md
```

Verified by running, not by self-report:

```
cd proto/p5-epaint && cargo test              # 63 passed, 0 failed
cd proto/p5-epaint && cargo run -- --symbology-generate   # wrote the table
cd proto/p5-epaint && cargo run -- --symbology-check      # agree, exit 0
```

And the flattened output was rendered to SVG, rasterised and LOOKED AT: all 25
icons are correct, including the anchor, the infantry saltire, and `supply`'s
full-width bar. `assets/symbology/render-icons.py` is the tool, and
`rsvg-convert -z 2 out.svg -o out.png` is the rasteriser. **Do not skip looking
for a geometry change; a parser that is wrong still compiles and still passes
its own tests, and two such bugs got through Units 1 and 2 that way.**

### Unit 3, the frame. Done, verified.

`src/symbology/frame.rs` is new and owns `SymbolFrame`, `frame_for`,
`frame_extent`, `frame_polygon`, `frame_strokes`, `frame_icon_radius`,
`SYMBOL_FRAME_ASPECT` and `symbol_box_px`. `map_render.rs` keeps `pub use`
re-exports of all of them so no call site moved, and `battle_dimension`
stays put until Unit 6.

Five of the six frame tests moved with the geometry; the sixth,
`an_aircraft_is_not_drawn_as_a_land_unit`, **stayed in `map_render.rs` on
purpose** — what it is really about is `battle_dimension` reading a
`MapSymbol`, and that function is a guess about a store enum until Unit 6
replaces it. Four new tests came with the move: the canonical extents against
the canonical outlines, no frame wider than the box it is fitted to, the
tolerance conversion, and the stroke gap inside the icon box.

The numbers, all measured rather than asserted. **The box has since gone to
32 px** — the left-hand figures are what the frame module was measured at when
this was written, and are kept because they are what the arithmetic was checked
against; the right-hand pair is what it measures now:

| frame | canonical (of 200) | at 22 px | icon radius | clearance | at 32 px | icon radius | clearance |
| --- | --- | --- | --- | --- | --- | --- | --- |
| friendly | 150 x 100 | 33 x 22 | 9.416 | 11.000 | 48 x 32 | 13.696 | 16.000 |
| hostile | 144 x 144 | 22 x 22 | 6.055 | 7.778 | 32 x 32 | 8.807 | **11.314** |
| neutral | 110 x 110 | 22 x 22 | 8.416 | 11.000 | 32 x 32 | 12.241 | 16.000 |
| unknown | 138.5 x 138.5 | 22 x 22 | 6.358 | 8.312 | 32 x 32 | 9.248 | 12.090 |

Every one of those radii is within 7 percent of the four hand-tuned
multipliers it replaces, which is the point: the sizes did not move, the
arithmetic did. The diamond is still the tight case and the quatrefoil's core
is the cubic lobe's corner at 52.33 reference units, not the circular lobe's
48.9.

**The diamond being tight is now load-bearing rather than incidental.** The
base disc the map paints under every glyph is sized from it
(`tightest_clearance_px`), because one marker is drawn once for whichever of
the four affiliations it resolves to and the disc must clear all four. Two
things had to be right to measure it, and both were wrong first: the quatrefoil
comes back as several flattened runs concatenated into one vector, so
`i -> i + 1 mod len` bridges a chord across the middle of the shape; and each
run repeats its first point as its last, so the trailing pair is zero-length.
Neither is an edge, and treating either as one reports a clearance of **zero**
— a disc of no size. That is also what caught the literal it replaced: the disc
was 8.0 while the diamond cleared only 7.78, so the hostile frame was already
being painted over by a fifth of a pixel.

### Three findings better than the brief

**The grammar had to become upstream's.** The old generator used a ten-command
DSL of `M`/`L`/`A` on the reasoning that a narrow grammar lets it PROVE the
table sits inside its primitive set. Sound while the table was hand-authored;
wrong now that the geometry has a source, because upstream writes cubics.
`svgpath.rs` parses the real grammar. The number lexer is hand-rolled because
`10-20` is two numbers and `.5.5` is two more, and whitespace-splitting — what
most hand-rolled parsers do — silently mis-reads both. Upstream writes both.

**The fill decision must come from the geometry, not upstream's flag.**
Upstream defaults a path to fill AND stroke. For the infantry saltire —
`M25,50 L175,150 M25,150 L175,50`, two single-segment runs — filling either
encloses no area, so a faithful "filled" deletes the icon. The rule shipped:
a fill covering less than one device pixel is a stroke's job.

**The coverage floor had to change metric, not value.** It measured ink
bounding-box AREA against the em box, which rejects APP-6C's own `supply` —
upstream's path is a bare horizontal line, zero em tall, not a dot. Area is the
wrong question for a uniform fit, which scales by the longer axis. Now the
longer axis, floor 0.15, tightest real icon 0.24.

### Two more, found in Unit 3

**The cubic flattening bound was FALSE, not loose** (`FINDINGS.md` §7).
`segs_for` used `0.25 * second_difference` for cubics, which is the constant
for QUADRATICS. The canonical quatrefoil lobe stands 32.25 units off its own
chord against a claimed bound of 21.4, and the test
`a_straight_cubic_needs_one_segment` was asserting one segment for a path that
deviates 17.5 em against a tolerance of 11.36. Three quarters is the
correction; the test is now split in two. Four of the twenty-five icons
changed on regeneration and every invariant still holds.

**A zero-height icon was emitted in the wrong coordinate space, and a clamp
hid it** (`FINDINGS.md` §8). `fit_for` bailed to the IDENTITY fit on
`w <= 0 || h <= 0`, leaving `supply` in raw em coordinates inside a table
documented as being in the unit square — and `q` saturated at plus or minus 8
while formatting, which is a value no normalised coordinate reaches and an em
coordinate reaches trivially, so the clamp rewrote one wrong number into a
different wrong number. Both removed; two invariants added that run against
the **checked-in table**, not a fresh `generate`. Both failed on the old
table, which is the only evidence they are worth anything.

Also fixed, and found by the same look: the frame test's
`min_distance_to_edges` measured the distance to the infinite LINE through each
edge, which under-reports by 12 percent at the quatrefoil's inner corners
because the perpendicular foot falls outside the segment. Clamped in
`frame.rs`'s copy. `map_render.rs`'s copy is untouched and still unclamped;
its remaining users are convex, where it does not bite.

## What is decided and must not be relitigated

Full reasoning in `/tmp/opencode/arena-nato/SYNTHESIS.md`. Load-bearing:

1. **Base architecture** is candidate C's from the arena: a pure egui-free
   plan from an identity to a primitive list, a generated icon table, and a
   contact sheet as the proof surface.

2. **Hull class leaves the icon vocabulary.** APP-6C has no destroyer icon; it
   has `Naval`, an anchor. Class goes to the label.

3. **Affiliation is never stored.** Resolved by `AffiliationResolver` and
   stamped in at paint time. The stored per-type row is global; affiliation is
   a fact about one exercise by one operator. The `None`-deletes /
   declared-`Unknown`-blocks distinction and `store.rs:1864-1964` stay
   untouched.

4. **Storage is a pinned, self-validating integer, not a SIDC.** APP-6C,
   APP-6D and 2525D use a 20-DIGIT NUMERIC code; the 15-character alphanumeric
   form is 2525B/C and APP-6B. All three arena candidates built on the wrong
   generation.

5. **The palette must never grey the frame on stale.** `main.rs:739-745`:
   the glyph greys, the frame keeps its hue, because "who stopped reporting"
   and "whose side" are two different questions.

6. **Geometry is ported from `spatialillusions/milsymbol` (MIT).** Licence
   verified directly: "Copyright (c) 2017 Måns Beckman". `MIT` is already in
   `about.toml`. Pinned at `f5134380157f475cbf5a9bdec69b6c33cf66e0e7`,
   v3.0.5.

7. **Nothing is extracted from Wikimedia.** Two independent grounds: CC
   BY-SA 4.0 by a private contributor, which `about.toml` accepts no entry
   for; and 57 percent of files are `<text>` letter codes with no geometry.

8. **Echelon is a real field, `None` for a unit.** A `Unit` here is one named
   hull with a hull number — a platform, not a formation. Three dots over
   KRI Cakra would be a falsehood. For groups it derives from `GroupKind`,
   monotone across four steps.

9. **Rejected, do not revisit.** `milsymbol` the Rust crate (AGPL-3.0).
   `milsymbol-rs` (needs a JS runtime). `ntds-icons` (NTDS naval outlines, not
   2525/APP-6). MapSymbs fonts (no OSI licence, stops at APP-6A).
   `mil-sym-java`/`mil-sym-ts` (Apache-2.0, right reference, wrong language).
   `spatialillusions/stanag-app6` (good SIDC schema, not needed yet).

10. **`frame_extent`'s "MEASURED from the standard's own artwork" is gone**,
    along with every number it propped up. What replaced it cites the pinned
    file, the commit and the row. **Never repeat that phrasing.**

## What is left, in order

Units form a chain. One owner each. Each ends verifiable.

### Unit 4. The paint plan.

`Slot` carrying a semantic role and no RGB field, so "hue never carries type"
is a compile error. `Mark`, `Placement`, echelon and modifier geometry, and
`palette_for` with two inks. Echelon glyphs are in upstream's
`src/symbolfunctions/modifier-echelon-json.js`; the `GR.M2.*` table carries
the real geometry for airborne, airmobile, motorized, mountain and amphibious
where `GR.M1.*` is text-only.

Note the frame's polyline density is fixed for `BOX_PX` (six segments per
quatrefoil lobe, a quarter-pixel chord error). A frame painted LARGER has to
re-flatten the path, not scale the polygon. `frame.rs` says so where a caller
will read it.

### Unit 5. Store.

Pinned-integer column, one-shot DROP and reseed (the table is client-owned,
never populated in production, and Minos publishes nothing into it), and an
insert-only seed keyed on the 41 mirror role strings, returning an unmatched
worklist.

### Unit 6. `main.rs`.

`paint_plan`; delete the six parallel matches (`MapSymbolShape` `:534`,
`map_symbol_shape` `:547`, `map_symbol_label` `:577`, `map_symbol_accent`
`:593`, `map_symbol_fill` `:611`, `battle_dimension` `map_render.rs:800`); move
the editor off the single-hull Inspector (`:8974`) onto the type row in the
Setup drill (`:6256`); delete the re-exports in `map_render.rs:739-779`.

### The contact sheet ships before the editor.

In `proto/p5-epaint`, rendering the SAME geometry the map draws: every icon,
four affiliations, the size the map actually draws (**32 px**, and that figure
should be read off `BOX_PX` rather than typed, for the reason the disc radius is
derived), plus a nearest-neighbour blow-up of that same raster. Dark background,
because the map is dark. `render-icons.py` is the throwaway that stood in for it
and is not it.

### Docs.

An ADR for the symbology decision, CONTEXT.md language, `DESIGN.md:324` which
still describes an old 8px marker.

## The one open product question, for the user

If hull class leaves the icon, then at **Far** zoom a destroyer and a corvette
are genuinely indistinguishable, because Far is the only zoom where the symbol
draws and `should_paint_unit_label` is `focused || Near` so no label paints
there. Hull class is a Near-zoom and Inspector fact only.

**Partly answered, and not by the answer anyone expected.** The box went from
22 px to 32 px, and the question was read as "the symbols are too small to tell
apart". That is true but it is the *second* reason. The first is that the map
still paints `MapSymbolShape` — ten glyphs hand-drawn against a radius-8 disc,
with reaches of 3.0 to 5.5 that differ from each other by about a pixel — while
the curated `icons.tsv` vocabulary that exists precisely to fix this is
generated, checked in, and still never painted. Raising the box made those ten
glyphs 45 percent larger and legible as shapes without changing which one you
are looking at. **Enlarging is necessary and not sufficient**; retiring
`MapSymbolShape` for the curated table is the half that actually answers the
question, and it is the Unit 6 work already on the list.

### The disc under the symbol was a fixture wearing a channel's clothes

`ship_color` returned blue for the id `"nordwind"` and red for `"ostsee"` —
the two vessel names in `tests/fixtures/tracks.json` — and **one green for
every other id**, which is every real hull. So the disc under every symbol
carried no information at all, and all it did was cover the glyph. It is gone,
and `ship_color` with it.

What replaced it is not "nothing", and that was the part worth measuring. The
glyph alone, drawn straight onto the map, is **1.07:1** against the sea — the
Accent hues were never a channel; they were decorative tints blended 32 percent
into a mid-tone base, and all ten sit between 1.02 and 2.11 against water and
land. The frame's INTERIOR is the fill now, in the affiliation's own hue
darkened toward map ink, which is the standard's own arrangement ("fill colour
is a redundant indication") and gives the shape and the fill the same meaning.
Measured off a capture afterwards: glyph on fill 3.88:1, fill on map 3.64:1.

The generalisable part: **a colour that never varied is not a channel, and
deleting it is a design change rather than a cleanup.** The four affiliation
inks had the same defect in milder form — Unknown was 1.02:1, the sea drawn on
the sea — and it survived because the disc was covering for it.

Three answers: accept it (Far reads allegiance, dimension and role, which is
what a real chart does); paint a compact hull number at Far (fixes it, at the
cost of a textier theatre view); or invent a non-standard third channel
(APP-6C does not sanction one).

**The contact sheet is the evidence and it ships before the editor does.** Its
Far band shows all three answers side by side. Do not ask the user to decide
this in the abstract.

## Fences for any delegate

- Do not build the root crate. `maplibre_native` is a multi-GB C++ build and
  `AGENTS.md` forbids it. `cd proto/p5-epaint && cargo check` is the fast loop
  and takes about two seconds. **It also means `map_render.rs` and `main.rs`
  cannot be compile-checked here** — changes there are verified by reading.
- Do not touch `assets/catalog.json`, `assets/fleet.json`, `scripts/`,
  `vendor/`, or the user's uncommitted work.
- No `build.rs`. The generator is a CLI mode of a binary the user already runs.
- No dependency additions.
- No comments that narrate. A comment states a non-obvious why or an invariant.
- Never claim a number was "measured from the standard" unless you measured it
  yourself off a source you can name and a commit you can pin.
- `src/symbology/{generate,svgpath}.rs` are not rustfmt-clean and were left
  that way on purpose: formatting them adds unrelated hunks to a symbology
  diff. `frame.rs` is clean. A repo-wide `cargo fmt` is its own decision.
- The `edit` tool has been seen to fail to match a multi-line `oldString`
  containing an em dash. Use a small unique anchor, or `python3` with an
  explicit `assert s.count(old) == 1`.