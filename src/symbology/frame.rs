//! The frame: the four outlines that carry affiliation, the icon box that
//! sits inside them, and the openness that carries battle dimension.
//!
//! Pure geometry, in pixels about a centre. No colour, no hit test, no
//! renderer — which is the point of it being here rather than in
//! `map_render`: a painter, a hit test and the headless contact sheet all
//! read one outline, so none of them can disagree with another about where
//! the frame's edge is.
//!
//! The outlines are the CANONICAL ones, taken from the frame table of
//! `spatialillusions/milsymbol` (MIT) at commit
//! `f5134380157f475cbf5a9bdec69b6c33cf66e0e7`, file `src/ms/symbolgeometries.js`,
//! keys `GroundFriend`, `GroundHostile`, `GroundNeutral`, `GroundUnknown`.
//! APP-6C draws land and sea surface in the same frames, so the `Ground*` rows
//! are the right four for both. Licence in `licenses/milsymbol-LICENSE.md`,
//! and what the port found — including the one shape that must NOT be copied
//! — is in `assets/symbology/FINDINGS.md` §6.

use super::icons::{BOX_PX, EM_BOX, MIN_ARC_SAGITTA_EM, MIN_ARC_SAGITTA_PX, STROKE_RATIO};
use super::svgpath;
use super::{Affiliation, BattleDimension};

/// The side of the reference space the canonical outlines are authored in.
///
/// Upstream draws into a 200-unit box centred on (100, 100). Every number in
/// this module that is not a pixel is in those units until
/// [`frame_scale`] converts it, which happens in exactly one place.
pub const REFERENCE_SIDE: f64 = 200.0;

/// The centre of the reference space, which every path in it is written
/// around and which has to come off before a path can be used about an
/// arbitrary point.
const REFERENCE_CENTRE: f64 = REFERENCE_SIDE / 2.0;

/// The canonical quatrefoil, verbatim.
///
/// Four CUBIC lobes springing from the corners of an inner square of side 74,
/// which puts each apex 32.25 out from that square and the whole outline 138.5
/// across. An earlier version of this file built the same shape from four
/// SEMICIRCULAR lobes of radius `extent / 4` on an inner square of side
/// `extent / 2`, which is the same silhouette with a different curve and a
/// narrower core: the cubic lobe's corner sits 52.33 from the centre and the
/// circular one's 48.9, so the two are distinguishable and only one of them is
/// the standard's.
const QUATREFOIL_PATH: &str = "M63,63 C63,20 137,20 137,63 C180,63 180,137 137,137 \
     C137,180 63,180 63,137 C20,137 20,63 63,63 Z";

/// The flattening tolerance in REFERENCE units.
///
/// [`MIN_ARC_SAGITTA_EM`] is stated in em and these outlines are authored in a
/// 200-unit space, so the figure has to be converted rather than reused: a
/// quarter of a device pixel is `0.25 x 200 / 22` units here. Handing the em
/// number to a reference-space path would flatten the lobes five times too
/// coarsely and nothing downstream would notice — the same class of mistake as
/// the fill threshold in `FINDINGS.md` §5, and pinned by a test below.
///
/// The consequence is that the outline is a FIXED polyline, six segments per
/// lobe, chosen for [`BOX_PX`]. That is invisible at the 22 px every frame is
/// drawn at — a quarter-pixel chord error cannot be seen — and visibly faceted
/// at four times that, so a caller that paints a frame larger has to flatten
/// the path again rather than scale this one. The icon table has exactly the
/// same contract and says so.
const FRAME_TOLERANCE_UNITS: f64 = MIN_ARC_SAGITTA_PX * REFERENCE_SIDE / BOX_PX;

/// The frame aspect: the friendly rectangle is 1.5 wide to 1 tall in the
/// reference space, and every other frame is square.
pub const SYMBOL_FRAME_ASPECT: f64 = 1.5;

/// Half the reference box: the icon's own drawing area.
///
/// APP-6A puts a standard octagon inside every frame, and upstream agrees
/// about where it is: `src/symbolfunctions/icon.js:5` defaults `gbbox` to
/// `x1:50, y2:150`, which is a 100 x 100 box in this 200-unit space. So the
/// icon's reach is not a per-frame fudge factor, it is one number, and it
/// happens to be exactly what fits inside the hostile diamond's inscribed
/// square — 144 / √2 is 101.8, and 100 is what the standard drew.
const ICON_BOX_HALF: f64 = REFERENCE_SIDE / 4.0;

/// The bounding box every frame is inscribed in: (width, height).
///
/// The frame is fitted to the box's HEIGHT and takes its width from its own
/// proportions, which is what keeps a hull the same size on screen whichever
/// way its allegiance changes: the shape differs, the size does not.
///
/// That rule is lossless only because no canonical frame is relatively wider
/// than the friendly rectangle, and a test says so. Were one to be, it would
/// have to be fitted by width instead, and the ladder — which sizes against
/// the box's height — would stop being the thing that bounds a symbol.
pub fn symbol_box_px() -> (f64, f64) {
    (BOX_PX * SYMBOL_FRAME_ASPECT, BOX_PX)
}

/// The four frame shapes, which in this symbology carry the thing's standard
/// identity. Colour is a redundant second cue, never the only one: a
/// colour-blind operator, and a greyscale screenshot, still read the shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolFrame {
    /// Friendly: an axis-aligned rectangle. The one frame that is not square.
    Rectangle,
    /// Hostile: a diamond, the one shape no other identity uses.
    Diamond,
    /// Neutral: a square.
    Square,
    /// Unknown: the quatrefoil, because "nobody has said" must not look like a
    /// confident claim in any of the other three shapes.
    Quatrefoil,
}

/// The frame an affiliation draws in.
pub fn frame_for(affiliation: Affiliation) -> SymbolFrame {
    match affiliation {
        Affiliation::Friendly => SymbolFrame::Rectangle,
        Affiliation::Hostile => SymbolFrame::Diamond,
        Affiliation::Neutral => SymbolFrame::Square,
        // An unresolved affiliation arrives here too: the resolver answers
        // Unknown rather than failing, and Unknown is the honest shape for it.
        Affiliation::Unknown => SymbolFrame::Quatrefoil,
    }
}

/// A frame's canonical extents in reference units, as (width, height).
///
/// Read off the four `Ground*` rows of the table named in the module header:
/// the friendly rectangle 150 x 100, the hostile diamond 144 x 144, the
/// neutral square 110 x 110, the unknown quatrefoil 138.5 x 138.5. Only the
/// rectangle is not square, and it is 1.5 : 1, which is where
/// [`SYMBOL_FRAME_ASPECT`] comes from.
///
/// A table rather than a measurement of the outlines, because the three
/// straight-edged shapes are exact and a sampled curve is not: the quatrefoil
/// is flattened from a path, so measuring its own bounding box would fold the
/// flattening tolerance into the number everything else is derived from. A test
/// holds the two to each other.
fn canonical_extent(frame: SymbolFrame) -> (f64, f64) {
    match frame {
        SymbolFrame::Rectangle => (150.0, 100.0),
        SymbolFrame::Diamond => (144.0, 144.0),
        SymbolFrame::Square => (110.0, 110.0),
        SymbolFrame::Quatrefoil => (138.5, 138.5),
    }
}

/// Reference units to pixels at `box_px`, for one frame.
///
/// The single conversion in this module. Every other size question is
/// [`canonical_extent`] in reference units multiplied by this.
fn frame_scale(frame: SymbolFrame, box_px: f64) -> f64 {
    box_px / canonical_extent(frame).1
}

/// A frame's own extents as (width, height) for a `box_px`-tall box.
///
/// Fitted by height, width from the frame's own proportions — see
/// [`symbol_box_px`] for why that is the rule and not an accident.
///
/// The neutral square is where this departs from the canonical artwork, and
/// the departure is deliberate. Its canonical 110 is LARGER than the friendly
/// rectangle's 100, so inscribing it faithfully would make it the largest
/// thing on the map; the box is what the LOD ladder and the picker size
/// against, and a symbol that overflows its own box jumps when a hull changes
/// allegiance. So it is inscribed like the rest. A house decision, recorded
/// here rather than dressed up as a measurement.
pub fn frame_extent(frame: SymbolFrame, box_px: f64) -> (f64, f64) {
    let (w, h) = canonical_extent(frame);
    (box_px * w / h, box_px)
}

/// The frame polygon about `center` — closed, whatever the dimension.
///
/// Openness is applied by [`frame_strokes`], which is what the painter draws;
/// this stays the whole outline so the icon's clearance and the hit geometry
/// can reason about one shape.
pub fn frame_polygon(frame: SymbolFrame, center: (f64, f64), box_px: f64) -> Vec<(f64, f64)> {
    let (cx, cy) = center;
    let (w, h) = frame_extent(frame, box_px);
    let (hw, hh) = (w / 2.0, h / 2.0);
    match frame {
        SymbolFrame::Rectangle => {
            vec![
                (cx - hw, cy - hh),
                (cx + hw, cy - hh),
                (cx + hw, cy + hh),
                (cx - hw, cy + hh),
            ]
        }
        // A rhombus on the box's mid-edges: its bounding box IS its height, so
        // it fills the same extent as the rectangle is wide.
        SymbolFrame::Diamond => vec![(cx, cy - hh), (cx + hw, cy), (cx, cy + hh), (cx - hw, cy)],
        SymbolFrame::Square => {
            vec![
                (cx - hh, cy - hh),
                (cx + hh, cy - hh),
                (cx + hh, cy + hh),
                (cx - hh, cy + hh),
            ]
        }
        SymbolFrame::Quatrefoil => quatrefoil_polygon(cx, cy, box_px),
    }
}

/// The polylines to stroke for one frame in one battle dimension.
///
/// A closed frame is ONE polyline whose last point repeats its first, so the
/// painter never has to know which is which. An open frame is the same ring
/// with the edges in the open band removed, split into the runs that remain —
/// generic over the shape, because "which edges are the bottom ones" differs
/// between a rectangle's single edge and a quatrefoil's arc.
pub fn frame_strokes(
    dimension: BattleDimension,
    frame: SymbolFrame,
    center: (f64, f64),
    box_px: f64,
) -> Vec<Vec<(f64, f64)>> {
    let poly = frame_polygon(frame, center, box_px);
    if dimension == BattleDimension::LandAndSeaSurface {
        let mut closed = poly;
        if let Some(first) = closed.first().copied() {
            closed.push(first);
        }
        return vec![closed];
    }
    let (_, cy) = center;
    let (_, h) = frame_extent(frame, box_px);
    let open_up = dimension == BattleDimension::Subsurface;
    let band = h * 0.12;
    let in_band = |p: (f64, f64)| {
        if open_up {
            p.1 <= cy - h / 2.0 + band
        } else {
            p.1 >= cy + h / 2.0 - band
        }
    };
    // Points in the open band are DROPPED, rather than whole edges being
    // skipped: on a smooth outline like the quatrefoil, skipping only the edges
    // whose both ends are in the band leaves the tips of the bottom arc
    // hanging in the air, which does not read as open at all. Runs of the
    // surviving points are what get stroked.
    let keep: Vec<bool> = poly.iter().map(|p| !in_band(*p)).collect();
    let n = poly.len();
    let start = (0..n)
        .find(|&i| keep[i] && !keep[(i + n - 1) % n])
        .unwrap_or(0);
    let mut runs: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    for step in 0..=n {
        let i = (start + step) % n;
        if keep[i] {
            current.push(poly[i]);
        } else if current.len() >= 2 {
            runs.push(std::mem::take(&mut current));
        } else {
            current.clear();
        }
    }
    if current.len() >= 2 {
        runs.push(current);
    }
    if runs.is_empty() {
        // A shape entirely inside the open band cannot happen at these
        // proportions, but a stroke that draws nothing would be a silently
        // missing frame, so fall back to the closed ring.
        let mut closed = poly;
        if let Some(first) = closed.first().copied() {
            closed.push(first);
        }
        return vec![closed];
    }
    runs
}

/// The unknown quatrefoil, flattened from the canonical path and scaled.
///
/// Two things follow from the cubic lobe's construction, and both were wrong
/// in the polar blend this replaced and in the semicircular version after it.
/// The narrowest points are the inner square's CORNERS, not the axes — so an
/// icon has a core to sit in — and the shape is solid through the middle,
/// because the four chords bound the interior rather than cutting it. It is
/// not a union of four discs either, which leaves a hole at the centre, since
/// no full disc centred on a side's midpoint reaches it.
fn quatrefoil_polygon(cx: f64, cy: f64, box_px: f64) -> Vec<(f64, f64)> {
    let runs = svgpath::parse_path_at(QUATREFOIL_PATH, FRAME_TOLERANCE_UNITS)
        .expect("the canonical quatrefoil path parses");
    let scale = frame_scale(SymbolFrame::Quatrefoil, box_px);
    let mut out = Vec::new();
    for run in &runs {
        for &(x, y) in &run.points {
            out.push((
                cx + (x - REFERENCE_CENTRE) * scale,
                cy + (y - REFERENCE_CENTRE) * scale,
            ));
        }
    }
    out
}

/// The largest radius an icon may occupy inside a frame.
///
/// 2525 builds a symbol by filling its frame rather than floating a small glyph
/// inside a big border, so the icon's reach depends on the frame it sits in,
/// and this is what turns one glyph table into four differently-fitted
/// symbols.
///
/// ONE rule, not four constants: the icon's reach is the canonical icon box
/// ([`ICON_BOX_HALF`]) scaled to this frame, less one stroke. The four tuned
/// multipliers this replaces included a `0.55` for the quatrefoil that was a
/// hidden coupling to the arc-sampling constant in `quatrefoil_polygon` —
/// change the sampling and the icon radius was silently wrong. The stroke is
/// subtracted because a gap narrower than one stroke closes at [`BOX_PX`], the
/// same threshold the icon generator rejects features under: an icon that
/// touches its frame is one blob, not a symbol in a frame.
///
/// The result still clears every frame, and a test measures the clearance
/// against the polygon rather than trusting this arithmetic.
pub fn frame_icon_radius(frame: SymbolFrame, box_px: f64) -> f64 {
    let (_, canonical_h) = canonical_extent(frame);
    ICON_BOX_HALF * box_px / canonical_h - box_px * STROKE_RATIO
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Width of a ring's bounding box, which is how the shape tests compare a
    /// frame against the box it has to be inscribed in.
    fn span(poly: &[(f64, f64)]) -> (f64, f64) {
        let mut min = (f64::MAX, f64::MAX);
        let mut max = (f64::MIN, f64::MIN);
        for &(x, y) in poly {
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
        }
        (max.0 - min.0, max.1 - min.1)
    }

    /// Distance from a point to the nearest edge of a ring, clamped to the
    /// segment.
    ///
    /// Clamped because the quatrefoil turns sharply at each inner corner: the
    /// perpendicular foot from the centre onto the segment leaving that corner
    /// falls OUTSIDE it, so the distance to the infinite line under-reports the
    /// real clearance by 12 percent and the icon-fit test would be measuring a
    /// smaller frame than the one drawn.
    fn min_distance_to_edges(point: (f64, f64), poly: &[(f64, f64)]) -> f64 {
        let mut worst = f64::INFINITY;
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len2 = dx * dx + dy * dy;
            if len2 < 1e-18 {
                continue;
            }
            let t = (((point.0 - a.0) * dx + (point.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
            worst = worst.min((point.0 - (a.0 + t * dx)).hypot(point.1 - (a.1 + t * dy)));
        }
        worst
    }

    fn contains(poly: &[(f64, f64)], point: (f64, f64)) -> bool {
        let mut inside = false;
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            if (a.1 > point.1) != (b.1 > point.1)
                && point.0 < (b.0 - a.0) * (point.1 - a.1) / (b.1 - a.1) + a.0
            {
                inside = !inside;
            }
        }
        inside
    }

    #[test]
    fn every_frame_is_inscribed_in_the_symbol_box() {
        // Painted frames differ in SHAPE, and therefore in bounding box:
        // an inscribed square is genuinely smaller than the box the rectangle
        // fills, and that difference is the visual point. What must hold is
        // that none of them escapes the box — so switching a hull's allegiance
        // can never make its symbol bigger or jump.
        let center = (100.0, 50.0);
        let mut seen: std::collections::HashSet<SymbolFrame> = std::collections::HashSet::new();
        for affiliation in Affiliation::ALL {
            let frame = frame_for(affiliation);
            assert!(seen.insert(frame), "two affiliations share a frame");
            let poly = frame_polygon(frame, center, BOX_PX);
            assert!(poly.len() >= 4, "every frame is a polygon");
            let (w, h) = span(&poly);
            let (bw, bh) = symbol_box_px();
            assert!(
                w <= bw + 1e-9 && h <= bh + 1e-9,
                "{affiliation:?} escapes the box"
            );
        }
        // The friendly rectangle FILLS the box; the neutral square is
        // inscribed in its height, which is what keeps friendly and neutral two
        // different SHAPES rather than two rectangles that differ only in size.
        let (bw, bh) = symbol_box_px();
        let rect = frame_polygon(SymbolFrame::Rectangle, center, BOX_PX);
        assert_eq!(span(&rect).0, bw);
        let square = frame_polygon(SymbolFrame::Square, center, BOX_PX);
        assert_eq!(span(&square).0, bh);
        assert!(span(&square).0 < span(&rect).0);
        // And the standard's aspect: friendly is 1.5 wide to 1 tall.
        assert_eq!(frame_extent(SymbolFrame::Rectangle, 10.0), (15.0, 10.0));
    }

    #[test]
    fn the_four_shapes_are_the_2525_ones() {
        assert_eq!(frame_for(Affiliation::Friendly), SymbolFrame::Rectangle);
        assert_eq!(frame_for(Affiliation::Hostile), SymbolFrame::Diamond);
        assert_eq!(frame_for(Affiliation::Neutral), SymbolFrame::Square);
        assert_eq!(frame_for(Affiliation::Unknown), SymbolFrame::Quatrefoil);
        // A diamond is a diamond, not a rotated square: its vertices sit on the
        // box's mid-edges, so it FILLS the same height as the rectangle.
        let diamond = frame_polygon(SymbolFrame::Diamond, (0.0, 0.0), 10.0);
        assert_eq!(diamond.len(), 4);
        assert!(
            diamond
                .iter()
                .all(|p| (p.0.abs() - 5.0).abs() < 1e-9 || (p.1.abs() - 5.0).abs() < 1e-9)
        );
        // The neutral square is square, and narrower than the rectangle.
        let square = frame_polygon(SymbolFrame::Square, (0.0, 0.0), 10.0);
        assert_eq!(span(&square).0, 10.0);
        // And the quatrefoil fills a square box, not the wide one — to within
        // the flattening tolerance, since the sampled arcs fall just inside the
        // true curve rather than through its apex.
        let quatrefoil = frame_polygon(SymbolFrame::Quatrefoil, (0.0, 0.0), 10.0);
        let tolerance_px = FRAME_TOLERANCE_UNITS * 10.0 / 138.5;
        assert!(
            (span(&quatrefoil).0 - 10.0).abs() <= tolerance_px,
            "quatrefoil spans {:?}, {} px outside the canonical width",
            span(&quatrefoil).0,
            (span(&quatrefoil).0 - 10.0).abs()
        );
    }

    #[test]
    fn battle_dimension_is_carried_by_the_frame_opening() {
        let center = (0.0, 0.0);
        for affiliation in Affiliation::ALL {
            let frame = frame_for(affiliation);

            // Land and sea surface: closed on all four sides.
            let closed = frame_strokes(BattleDimension::LandAndSeaSurface, frame, center, BOX_PX);
            assert_eq!(closed.len(), 1, "{affiliation:?} must be one closed run");
            assert_eq!(
                closed[0].first(),
                closed[0].last(),
                "{affiliation:?} must close on itself"
            );

            // Air and space: the bottom edge is gone. Measured, not assumed —
            // no point of the stroke may sit in the bottom band.
            for run in frame_strokes(BattleDimension::AirAndSpace, frame, center, BOX_PX) {
                assert!(run.len() >= 2, "{affiliation:?} keeps its remaining edges");
                assert!(
                    !run.iter().any(|&(_, y)| y > center.1 + BOX_PX * 0.38),
                    "{affiliation:?} must not draw its bottom edge"
                );
            }

            // Subsurface: the top edge is gone.
            for run in frame_strokes(BattleDimension::Subsurface, frame, center, BOX_PX) {
                assert!(run.len() >= 2);
                assert!(
                    !run.iter().any(|&(_, y)| y < center.1 - BOX_PX * 0.38),
                    "{affiliation:?} must not draw its top edge"
                );
            }
        }
    }

    #[test]
    fn an_icon_always_fits_inside_its_frame() {
        // The icon sits INSIDE the frame rather than floating in it, so its
        // reach must clear the frame's own boundary in every case. Measured
        // against the polygon, not against a formula — the diamond is the tight
        // one (its edge passes at half/√2), and a quatrefoil's inward fillets
        // are not a circle at all.
        for affiliation in Affiliation::ALL {
            let frame = frame_for(affiliation);
            let radius = frame_icon_radius(frame, BOX_PX);
            let poly = frame_polygon(frame, (0.0, 0.0), BOX_PX);
            let clearance = min_distance_to_edges((0.0, 0.0), &poly);
            assert!(
                radius < clearance,
                "{affiliation:?} icon radius {radius} crosses its frame, which closes at {clearance}"
            );
            // And never so tight that a glyph disappears.
            assert!(radius >= 4.0, "{affiliation:?} has no room for an icon");
            // And never so loose that the icon is the frame: the stroke is
            // subtracted, so at least one stroke of daylight survives on every
            // side of the ink.
            assert!(
                radius <= clearance - BOX_PX * STROKE_RATIO,
                "{affiliation:?} leaves no gap for a stroke to be read against"
            );
        }
    }

    #[test]
    fn the_quatrefoil_closes_over_its_centre() {
        // The union of four corner circles has a HOLE at the middle, so an
        // outline built from it would leave the icon's home outside the shape.
        // The fillets bow inward to close it — which is the clearance test
        // above measuring a real core rather than a gap.
        let poly = frame_polygon(SymbolFrame::Quatrefoil, (0.0, 0.0), BOX_PX);
        assert!(
            contains(&poly, (0.0, 0.0)),
            "the centre must be inside the quatrefoil"
        );
        assert!(poly.len() > 20, "a quatrefoil is not a quadrilateral");
        // The core is the inner square's CORNERS, 37 out on each axis in
        // reference units, and those corners are what a semicircular lobe
        // moves: they are 48.9 from the centre there and 52.33 here. Pinning
        // the number is what keeps a plausible-looking approximation of the
        // quatrefoil from coming back.
        let scale = frame_scale(SymbolFrame::Quatrefoil, BOX_PX);
        let corner = 37.0 * 2.0f64.sqrt() * scale;
        assert!(
            (min_distance_to_edges((0.0, 0.0), &poly) - corner).abs()
                <= FRAME_TOLERANCE_UNITS * scale,
            "the core is {} from the centre, not the canonical {corner}",
            min_distance_to_edges((0.0, 0.0), &poly)
        );
    }

    #[test]
    fn the_canonical_extents_are_the_ones_the_outlines_hold() {
        // The extent table is transcribed from the frame table and the outline
        // is drawn from a path, so they are two transcriptions of one upstream
        // row and could drift apart silently. The outlines are the thing every
        // size is measured from, so they are what the table is checked against
        // — within the flattening tolerance for the curve, exactly for the
        // three straight-edged shapes.
        for frame in [
            SymbolFrame::Rectangle,
            SymbolFrame::Diamond,
            SymbolFrame::Square,
            SymbolFrame::Quatrefoil,
        ] {
            let (cw, ch) = canonical_extent(frame);
            // Drawn at its own canonical height, so the polygon comes back in
            // reference units and is comparable with the table.
            let poly = frame_polygon(frame, (0.0, 0.0), ch);
            let (w, h) = span(&poly);
            let tolerance = FRAME_TOLERANCE_UNITS;
            assert!((w - cw).abs() <= tolerance, "{frame:?} spans {w}, not {cw}");
            assert!(
                (h - ch).abs() <= tolerance,
                "{frame:?} spans {h} tall, not {ch}"
            );
        }
    }

    #[test]
    fn no_frame_is_wider_than_the_box_it_is_fitted_to() {
        // `frame_extent` fits by height and takes the width from the frame's
        // own proportions, which is only valid while nothing is relatively
        // wider than the friendly rectangle. This is the check that says so.
        let (bw, _) = symbol_box_px();
        for affiliation in Affiliation::ALL {
            let (w, h) = frame_extent(frame_for(affiliation), BOX_PX);
            assert_eq!(h, BOX_PX, "{affiliation:?} must fill the box's height");
            assert!(w <= bw + 1e-9, "{affiliation:?} is {w} wide in a {bw} box");
        }
        // The friendly rectangle is the widest thing in the set, and it is what
        // fills the box's width exactly — which is where the aspect comes from.
        assert_eq!(frame_extent(SymbolFrame::Rectangle, BOX_PX).0, bw);
    }

    #[test]
    fn the_flattening_tolerance_is_converted_rather_than_reused() {
        // Two routes to the same quarter of a device pixel, and they must
        // agree: through em, or through the reference side. If they ever stop
        // agreeing, one of the two is being computed off the wrong space.
        assert!(
            (FRAME_TOLERANCE_UNITS - MIN_ARC_SAGITTA_EM * REFERENCE_SIDE / EM_BOX).abs() < 1e-12,
            "the frame tolerance is not the icon tolerance in another space"
        );
        // And reusing the em figure unconverted would be five times too coarse,
        // which is the whole reason this is a conversion and not a constant.
        assert!(FRAME_TOLERANCE_UNITS * (EM_BOX / REFERENCE_SIDE) - MIN_ARC_SAGITTA_EM < 1e-9);
    }
}
