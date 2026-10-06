# What the milsymbol port found

Notes taken while porting `spatialillusions/milsymbol` (MIT) into
`assets/symbology/milsymbol.tsv`. Kept because both findings changed the
design and neither is visible from the code alone.

Pinned upstream: `f5134380157f475cbf5a9bdec69b6c33cf66e0e7`, v3.0.5,
2026-09-18. Licence in `licenses/milsymbol-LICENSE.md`.

## 1. Most APP-6 icons are text, not geometry

Of 1496 keys across the four `iconparts` tables, 689 carry path geometry and
807 are letter codes (`text("DD")`, `textm1("MS")`) that need a font. Only 59
sea icons have geometry at all.

This is the same defect that disqualified the Wikimedia gallery, measured on
authoritative art instead of a stranger's redrawing: it is not a property of
one bad source, it is a property of the standard. A hull class really is a
letter code, not a glyph.

It vindicates the decision already recorded in `CONTEXT.md`: hull class leaves
the icon vocabulary and goes to the label.

## 2. The icons that DO have geometry are barely distinguishable from each other

Rendered as a contact sheet and looked at, which is the only way to see this:

- Eight sea roles — cargo, tow, tug, fishing, trawler, merchant, container,
  roll-on/roll-off — are **the same trapezoid hull**, differing only in a
  superstructure box that is absent or identical. At the size the map drew when
  this was written (22 px), they are one icon.
- Surface combatant, patrol, amphibious warfare ship, carrier and mine warfare
  vessel are **the same downward arrow**, differing by one or two steps in the
  tail. Mine warfare adds three bumps and is the only one that reads.
- Most `GR.M1.*` modifiers are text-only. The `GR.M2.*` slot carries the real
  geometry for airborne, airmobile, motorized, mountain and amphibious, so the
  M1/M2 distinction is about table slot, not about the glyph.

So the constraint on the vocabulary is not "which icons exist" but **"which
icons survive being as wide as the map draws them"**. A port that took all 689
would ship 689 icons of which most are indistinguishable, and the invariants
would pass all of them because every one of them is individually well-formed.

The curated selection in `icons.tsv` is therefore a judgement about
distinguishability, and the contact sheet is what it is judged on.

**The box has since gone to 32 px**, and that is a real improvement on the
second bullet rather than a change of the criterion: the same downward arrow is
now 45 percent wider, and the fillets the flattening produced gained the
vertices that let them read. What did NOT improve is the third bullet, and
neither would any size — text-only modifiers are a property of the upstream
table, not of how large we draw it. A larger box relaxes this section's
constraint; it does not discharge it, and the selection would want re-judging
against a fresh contact sheet rather than inherited on the strength of a
number.

## 3. Our stroke is five times heavier than upstream's, and it costs two icons

Upstream strokes at 3 units in a 200 box: 1.5 percent, which is **0.33 px** at
the 22 px box this was measured against. Anything that thin vanishes, so this
project strokes at `STROKE_RATIO` = 0.072 of the box — 1.58 px there, and
**2.30 px now** that the box is 32. That five-fold difference is what closes
fine gaps, and the gap invariant caught exactly two icons:

    SE.IC.SEA SURFACE DECOY   three chevrons 1.50 px apart
    SE.IC.DISTRESSED VESSEL   features 1.25 px apart

Both were dropped. This is the price of being legible at theatre zoom, and it
is worth stating plainly rather than widening the invariant: a symbol this size
cannot carry detail finer than a pixel, and the standard has detail finer than
a pixel. Neither role is in the client's domain.

The stroke is a RATIO, so the raise did not rescue either icon — the gap
between their features grew by exactly the same 45 percent the box did, and
they are still below one stroke wide. Regenerating at the new box changed three
icons and dropped none: an arc flattened finer gains vertices, and nothing that
fitted before stopped fitting.

## 4. The coverage floor had to change metric, not value

The first version measured ink bounding-box AREA against the em box, with a
15 percent floor. That rejects APP-6C's own `supply`, whose upstream path is
`M25,120 l150,0` — a bare horizontal line, 750 em wide and **zero em tall**. Its
area is zero. It is not a dot.

Area is the wrong question to ask of a uniform fit, because the fit scales by
the longer axis: a long thin icon is drawn at exactly the same size as a square
one. The floor is now the longer AXIS as a fraction of the box, which still
rejects a dot — the entire point of the rule — and accepts the standard's own
horizontal-bar icons. Measured over the selection the tightest icon is 0.24
(upstream's `engineer`, an 80-unit bracket) and the loosest 0.84.


## 5. Upstream's fill default is "both", and for many icons only the stroke shows

`defaultProperties` in `iconparts-functions.js` sets `fill` and `stroke` to the
icon colour for any path that does not say otherwise, so a plain path is a
filled shape with a 3-unit stroke riding on it. epaint draws a fill and a
stroke as separate `Shape`s and this symbology never wants both on one mark.

The naive mapping — `fill !== false` means Fill — silently deletes icons. The
infantry saltire is `M25,50 L175,150 M25,150 L175,50`: two single-segment runs.
Filling either encloses no area at all, so a faithful "filled" draws nothing
and the icon vanishes. Same for `reconnaissance`'s `M25,150L175,50`, and for
the many `GR.M1.*` tables upstream marks filled but draws as strokes.

The fill decision is therefore made from the geometry: a fill covering less
than one device pixel is not a fill, it is a stroke's job. The threshold lives
in `generate.rs`, in em, and is measured after the conversion to em — upstream's
box is 200 units to tfg's 1000, so the same number in upstream units would be
25 times too small and would let every hairline sliver through as a fill.

## 6. The canonical frame geometry, and one shape that must NOT be copied

`src/ms/symbolgeometries.js` in the same pinned checkout carries the frame
outlines with explicit bounding boxes, in the same 200-unit space centred on
(100,100). Extents measured off the paths:

| frame | bbox | note |
| --- | --- | --- |
| friendly | 150 x 100 | `M25,50 l150,0 0,100 -150,0 z` |
| hostile | 144 x 144 | `M 100,28 L172,100 100,172 28,100 100,28 Z` |
| neutral | 110 x 110 | `M45,45 l110,0 0,110 -110,0 z` |
| unknown | 138.5 x 138.5 | four CUBIC lobes, inner square side 74 |

Three of these replace numbers in the tree that no one could check.
`frame_extent` claims the extents were "MEASURED from the standard's own
artwork, not inferred from a filename" and cites 590x390, 586x586, 430x430,
590x590. The ratios survive (friendly is the only non-square, and 1.5:1 either
way) but the provenance claim is not one this project can make, and the
neutral frame is 110 against a friendly height of 100 — **larger**, where the
tree deliberately inscribes it smaller so the box holds everything. That
reversal is a house decision and needs recording as one, not as a measurement.

The icon box is corroborated rather than assumed: `icon.js:5` defaults `gbbox`
to `x1:50, x2:150, y1:50, y2:150`, a 100x100 box in the 200 space. So the
icon's fit target is that box, and `frame_icon_radius`'s four tuned
multipliers — including the `0.55` for the quatrefoil, a hidden coupling to
`PER_LOBE` in `quatrefoil_polygon` — can go.

**Do not copy `SeaFriend`, which upstream draws as a CIRCLE, `cx 100 cy 100
r 60`.** That is the later-edition sea frame; APP-6C draws the sea surface
friendly frame as the same rectangle as land, and this project implements
APP-6C. Upstream keys its frame table by `dimension + affiliation` with no
edition in the key, so it cannot be read off the table which shape belongs to
which edition — copying it would import a 2525D outline into an APP-6C
implementation. This is the one place in the port where the source is right
about the standard and still wrong for this project.

The four cubic lobes also contradict `quatrefoil_polygon`, which builds four
SEMICIRCULAR ones from `extent/4` arcs on an `extent/2` inner square. Same
shape, different curve: the canonical one has inner side 74 and total extent
138.5, so each lobe rises 32.25 rather than the current 34.6.

Both of these have now landed, in `src/symbology/frame.rs`: the outlines are
the canonical ones, the quatrefoil is flattened from the upstream path rather
than assembled from arcs, and the icon box is one number instead of four tuned
multipliers. The `frame_extent` comment that §6 calls unverifiable is gone.

## 7. The cubic flattening bound was FALSE, not merely loose

Found while porting the quatrefoil, because reusing `svgpath.rs` on a path
authored in the reference space is the first thing that has to survive being
flown in a different unit.

`segs_for` chose a segment count from the second difference of a curve's
control points: `bound = 0.25 * max(|P0-2P1+P2|, |P1-2P3+P2|)`, then
`n = ceil(sqrt(bound / tolerance))`. A quarter is the right constant for a
QUADRATIC and the wrong one for a cubic — a quadratic's worst case is its
apex, half a second difference out; a cubic's comes a third of the way along,
where it has already picked up the whole of it, so it needs three quarters.

The estimate was not merely conservative. The canonical quatrefoil lobe
`M63,63 C63,20 137,20 137,63` stands **32.25 units off its own chord** while a
quarter of its second difference is 21.4. And the cubic that looks straight,
`M0 0 C33 66 66 99 99 99` — the first three control points collinear, which is
what `a_straight_cubic_needs_one_segment` was built on — deviates **17.5 em**
against a tolerance of 11.36. One segment was accepted for a curve that broke
the tolerance by half, and the test that should have caught it was asserting
the wrong thing about the wrong path.

So a "tolerance the curve provably exceeds is not a tolerance", and every
flattened table in this project was up to three times coarser than it claimed.
Three quarters is the correction; the test is now split in two, one genuinely
collinear cubic and one that only looks it.

Two things followed, and both are now in the code rather than in a comment:
`parse_path_at(d, tolerance)` takes the tolerance as a parameter, because the
icon tables are in em and the frame outlines are in reference units and passing
the em figure to the latter would flatten five times too coarsely — the same
class of mistake as §5, in the opposite direction. And the frame's tolerance is
written as the conversion `MIN_ARC_SAGITTA_PX * REFERENCE_SIDE / BOX_PX` with a
test pinning it to the em route, so neither number can drift alone.

Regenerating after the correction changed four of the twenty-five icons —
`AirDefence`, `AmphibiousGround`, `NavalInfantry`, `FixedWing`, the ones with
curves coarse enough for the bound to matter — and every readability invariant
still holds. A stricter flattening can only remove curvature, never open a
gap, so nothing that passed before can fail now.

## 8. A zero-height icon was emitted in the wrong coordinate space, and a clamp hid it

Found by LOOKING at the sheet, which is the whole argument for having one.

`fit_for` had a `if w <= 0.0 || h <= 0.0 { return Ok(Fit::NONE) }` guard, and
`Fit::NONE` is the IDENTITY. So any icon with zero extent on one axis was left
in raw em coordinates inside a table whose own doc comment says "already in the
unit square" — and the painter multiplies that by the box, so `supply` drew as a
16 em line where the standard's is 750 em wide.

The guard was hidden by `q`, which saturated at plus or minus 8 while
formatting. Eight is a value no normalised coordinate can reach and an em
coordinate reaches trivially, so the clamp rewrote one wrong number into a
different wrong number, quietly, in the only place the number was printed. Both
were removed: the guard is gone (`w.max(h)` cannot be zero, because the
coverage floor already refuses anything under 15% of the box, and that includes
the both-zero case) and the clamp is gone.

Two invariants replace them, and both run against the **checked-in table**
rather than a fresh `generate`, because the artifact is what ships and a
generator that is right while its output is stale has still shipped the stale
one:

- `every_icon_sits_in_the_unit_square` — every point of every mark is inside
  `[0, 1]`, to 1e-6.
- `only_an_icon_with_no_ink_has_the_identity_fit` — `FIT[i] == Fit::NONE` if
  and only if `GEOMETRY[i]` is empty, so a caller reasoning about an icon's
  margins from `FIT` is never told an icon with geometry has none.

Both failed on the old table, which is the only evidence they are worth
anything. `supply` now draws as the full-width bar §4 said it was.
