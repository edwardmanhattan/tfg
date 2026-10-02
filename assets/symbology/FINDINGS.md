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
  superstructure box that is absent or identical. At the 22 px the map draws,
  they are one icon.
- Surface combatant, patrol, amphibious warfare ship, carrier and mine warfare
  vessel are **the same downward arrow**, differing by one or two steps in the
  tail. Mine warfare adds three bumps and is the only one that reads.
- Most `GR.M1.*` modifiers are text-only. The `GR.M2.*` slot carries the real
  geometry for airborne, airmobile, motorized, mountain and amphibious, so the
  M1/M2 distinction is about table slot, not about the glyph.

So the constraint on the vocabulary is not "which icons exist" but **"which
icons survive being 22 pixels wide"**. A port that took all 689 would ship 689
icons of which most are indistinguishable, and the invariants would pass all
of them because every one of them is individually well-formed.

The curated selection in `icons.tsv` is therefore a judgement about
distinguishability, and the contact sheet is what it is judged on.

## 3. Our stroke is five times heavier than upstream's, and it costs two icons

Upstream strokes at 3 units in a 200 box: 1.5 percent, which is **0.33 px** at
the 22 px this map draws. Anything that thin vanishes, so this project strokes
at `STROKE_RATIO` = 0.072 of the box, 1.58 px. That five-fold difference is
what closes fine gaps, and the gap invariant caught exactly two icons:

    SE.IC.SEA SURFACE DECOY   three chevrons 1.50 px apart
    SE.IC.DISTRESSED VESSEL   features 1.25 px apart

Both were dropped. This is the price of being legible at theatre zoom, and it
is worth stating plainly rather than widening the invariant: a 22 px symbol
cannot carry detail finer than a pixel, and the standard has detail finer than
a pixel. Neither role is in the client's domain.

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
