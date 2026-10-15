//! The map's marker model: what one hull IS on the map, and the vocabulary
//! that draws it.
//!
//! A MOVE, not a redesign (ADR-0020). `ShipMarker`, `MapSymbolShape`, the
//! affiliation inks and the pick order were `main.rs`'s, spelled against
//! `egui::Color32`. They are here because both shells need them and the GPUI
//! shell cannot name egui: the colours are `view::Rgba` now, and each shell
//! converts at its own boundary.
//!
//! What is NOT here: the painters. `paint_affiliation_frame`, `paint_map_symbol`
//! and the glyph paths stay in `main.rs` because they are egui painter calls,
//! and the GPUI shell writes its own from the same model. That is the seam —
//! one vocabulary, two renderers, the same rule the halo follows.
//!
//! The model is deliberately a VIEW of a fix rather than the fix: it carries
//! the resolved affiliation, the projected position and the drawn extent, so a
//! painter never reaches back into the registry or the resolver mid-frame.

use crate::map_render::UnitLod;
use crate::visuals::UnitVisual;


pub struct ShipMarker {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub stale: bool,
    /// Wire data older than the old-data threshold (backfilled ticket).
    pub old_data: bool,
    pub source: crate::geo::track::FixSource,
    pub trail: Vec<(f64, f64)>,
    /// Course, shortest-arc blended across the poll interval so a
    /// turn does not spin a thumbnail the long way round. None when
    /// the fix carries no course — the marker then draws neutral and
    /// says so, rather than assuming north.
    pub heading_deg: Option<f32>,
    /// Taxonomy-derived far-map glyph. Unknown always draws, so a
    /// missing visual, image, texture, or taxonomy cannot make the
    /// unit disappear.
    pub map_symbol: crate::store::MapSymbol,
    pub latitude: f64,
    /// The blended position's longitude, carried beside the latitude
    /// so a group's ground extent is measured from the same fix the
    /// marker is drawn with rather than a second lookup of the
    /// registry's latest.
    pub longitude: f64,
    /// The marker's own drawn extent in logical pixels — the larger of
    /// its length and beam — carried so a group's Zone can be padded to
    /// enclose what it actually holds rather than to enclose a point.
    /// Zero when nothing measurable is known; the Zone pad floors that.
    pub footprint_px: f64,
    /// Resolved affiliation, so the painter never reaches back into the
    /// resolver mid-frame. Unknown is a real answer, never a missing
    /// one: the resolver always terminates.
    pub affiliation: crate::store::Affiliation,
    /// In the exercise but unassigned in the task organisation, which the
    /// symbology draws as a dashed frame. Independent of `stale`: a
    /// planned unit and a silent one are different facts.
    pub planned: bool,
    pub label: String,
    pub lod: UnitLod,
    pub visual: Option<UnitVisual>,
}

// The shape vocabulary lives in `view`, which the parity proto compiles: the
// shell draws these shapes and must not reach through `markers` (and so
// `store`) to name them. Re-exported here so every existing
// `markers::MapSymbolShape` path keeps resolving.
pub use crate::view::MapSymbolShape;

pub fn map_symbol_shape(symbol: crate::store::MapSymbol) -> MapSymbolShape {
    use crate::store::MapSymbol;
    match symbol {
        MapSymbol::UnknownShip => MapSymbolShape::Dot,
        MapSymbol::Destroyer => MapSymbolShape::Destroyer,
        MapSymbol::Frigate => MapSymbolShape::Frigate,
        MapSymbol::Corvette => MapSymbolShape::Corvette,
        MapSymbol::Auxiliary => MapSymbolShape::Auxiliary,
        MapSymbol::Landing => MapSymbolShape::Landing,
        MapSymbol::Submarine => MapSymbolShape::Submarine,
        MapSymbol::Plane => MapSymbolShape::Plane,
        MapSymbol::GroundUnit => MapSymbolShape::Tank,
        MapSymbol::Port => MapSymbolShape::Port,
    }
}


/// The symbol a hull's class NAME reads as, most specific first.
///
/// A BRIDGE, not the console's rule, and named as one so it is not mistaken
/// for it. The console resolves a symbol from the taxonomy's STABLE IDS
/// (`store::SymbolResolver`: type, then category, then domain), which needs
/// the synced mirror and the operator's own assignments behind it. The
/// register the GPUI shell fetches carries class NAMES instead, and no ids
/// to resolve against, so the prototype reads the names — in one place,
/// where the seam is visible, and tested, rather than scattered into the
/// shell. When the store is wired into the binary this is what the ID
/// resolver replaces; the two agree on the vocabulary, which is why the
/// return type is the same enum and not a shape.
///
/// Ordered most-specific first: a class that says "submarine" is a submarine
/// whatever else it says, and only an unreadable name falls through to the
/// honest generic ship.
pub fn symbol_from_class_name(name: &str) -> crate::store::MapSymbol {
    use crate::store::MapSymbol;
    let n = name.to_ascii_lowercase();
    let has = |needle: &str| n.contains(needle);
    if has("submarine") || has("ssk") || has("ssn") || has("submersible") {
        return MapSymbol::Submarine;
    }
    if has("destroyer") || has("ddg") {
        return MapSymbol::Destroyer;
    }
    if has("frigate") || has("ffg") {
        return MapSymbol::Frigate;
    }
    if has("corvette")
        || has("fast attack")
        || has("patrol")
        || has("missile boat")
        || has("fac")
    {
        return MapSymbol::Corvette;
    }
    if has("landing") || has("lpd") || has("lhd") || has("lst") || has("amphibious") {
        return MapSymbol::Landing;
    }
    if has("auxiliary")
        || has("tanker")
        || has("oiler")
        || has("supply")
        || has("replenishment")
        || has("hospital")
    {
        return MapSymbol::Auxiliary;
    }
    if has("port") || has("harbour") || has("harbor") || has("base") {
        return MapSymbol::Port;
    }
    if has("drone")
        || has("uav")
        || has("aircraft")
        || has("plane")
        || has("helicopter")
        || has("male")
    {
        return MapSymbol::Plane;
    }
    MapSymbol::UnknownShip
}

// The glyph's colour vocabulary lives in `view` for the same reason the
// shape enum does: the shell paints it and must not reach through `markers`.
// Re-exported so existing `markers::` paths keep resolving.
pub use crate::view::{map_symbol_accent, map_symbol_fill};

// The affiliation and frame colour vocabulary lives in `view` for the same
// reason the glyph's does: the shell paints it. Re-exported so existing
// `markers::` paths keep resolving.
pub use crate::view::{
    FRAME_DASH_GAP_PX, FRAME_DASH_PX, FRAME_KEYLINE, FRAME_KEYLINE_OFFSET_PX,
    FRAME_KEYLINE_OUT_PX, FRAME_KEYLINE_PX, FRAME_KEYLINE_STALE, FRAME_STROKE_PX, MAP_INK_DARK,
    affiliation_fill, affiliation_ink, darkened_ink, offset_run_outward,
};

/// naming a wrong kind.
pub fn plurality_icon(symbols: &[crate::store::MapSymbol]) -> Option<crate::store::MapSymbol> {
    if symbols.is_empty() {
        return None;
    }
    // Tallied by ordinal: `MapSymbol` is a plain enum with no Hash/Ord,
    // and the ordinal is its stable identity, so the tie-break below is
    // deterministic rather than dependent on iteration order.
    let mut tally: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
    for symbol in symbols {
        *tally.entry(*symbol as i32).or_insert(0) += 1;
    }
    tally
        .into_iter()
        .max_by_key(|(ordinal, count)| (*count, *ordinal))
        .and_then(|(ordinal, count)| {
            (count * 2 > symbols.len())
                .then(|| crate::store::MapSymbol::from_ordinal(ordinal))
        })
}

/// took the FIRST painted marker, which is the one they cannot see when
/// near-zoom thumbnails overlap.
pub fn paint_order(markers: &[ShipMarker]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..markers.len()).collect();
    // Stable, so markers of equal Representation keep their list order.
    order.sort_by_key(|&i| match markers[i].lod {
        crate::map_render::UnitLod::Far => 0,
        crate::map_render::UnitLod::Middle => 1,
        crate::map_render::UnitLod::Near => 2,
    });
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_render::{GLYPH_AUTHORING_RADIUS_PX, SYMBOL_BOX_PX, symbol_footprint_radius_px};
    use crate::store::Affiliation;
    use crate::symbology::frame::{frame_for, frame_icon_radius};
    use crate::view::Rgba;
    use super::*;

    /// The glyph fill is the shape's accent brightened toward white, and
    /// stale greys it — the console's rule, and the reason a hull that
    /// stopped reporting stops advertising which kind it is.
    #[test]
    fn the_glyph_fill_is_the_accent_and_grey_when_stale() {
        assert_eq!(
            map_symbol_fill(MapSymbolShape::Frigate, true),
            Rgba::rgb(160, 160, 160)
        );
        let accent = map_symbol_accent(MapSymbolShape::Frigate);
        let fresh = map_symbol_fill(MapSymbolShape::Frigate, false);
        assert_ne!(fresh, Rgba::rgb(160, 160, 160), "fresh is not the stale grey");
        assert!(
            fresh.r >= accent.r && fresh.g >= accent.g && fresh.b >= accent.b,
            "the fill brightens the accent toward white"
        );
    }

    /// The bridge reads the register's real class names, which is the whole
    /// point of it existing: a name it cannot read must fall through to the
    /// generic ship, never to a wrong shape.
    #[test]
    fn the_class_name_bridge_reads_the_register() {
        use crate::store::MapSymbol;
        for (name, want) in [
            ("KRI Ahmad Yani / Van Speijk · Frigate", MapSymbol::Frigate),
            ("Nagapasa / DSME 209-1400 · Submarine", MapSymbol::Submarine),
            ("Mandau / PSM Mk-III · Fast Attack Craft", MapSymbol::Corvette),
            ("Makassar / LPD · Amphibious", MapSymbol::Landing),
            ("Arun · Auxiliary", MapSymbol::Auxiliary),
            ("Anka-S (MALE)", MapSymbol::Plane),
            ("A hull nobody classified", MapSymbol::UnknownShip),
        ] {
            assert_eq!(symbol_from_class_name(name), want, "reading {name:?}");
        }
    }

    /// WCAG relative-luminance contrast between two opaque colours.
    fn contrast_ratio(a: Rgba, b: Rgba) -> f32 {
        fn channel(v: u8) -> f32 {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        let lum = |c: Rgba| {
            0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
        };
        let (hi, lo) = {
            let (x, y) = (lum(a), lum(b));
            if x > y {
                (x, y)
            } else {
                (y, x)
            }
        };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn every_map_symbol_has_a_distinct_shape_and_fill() {
        let identities: std::collections::HashSet<_> = crate::store::MapSymbol::ALL
            .into_iter()
            .map(|symbol| {
                let shape = map_symbol_shape(symbol);
                (shape, map_symbol_accent(shape))
            })
            .collect();
        assert_eq!(identities.len(), crate::store::MapSymbol::ALL.len());
        assert_eq!(map_symbol_shape(crate::store::MapSymbol::UnknownShip), MapSymbolShape::Dot);
    }

    #[test]
    fn the_frame_outline_reads_on_both_of_its_edges() {
        // An outline has two edges and is visible in proportion to the WEAKER
        // of them. This test exists because only one of them was ever checked:
        // the hue clears 3.10:1 against its own fill and 1.03:1 against the
        // sea, so the frame's real legibility was the sea — and the fill is
        // what a reviewer looks at when they ask whether a symbol is clear.
        //
        // The casing is what fixes the outer edge, so the property to hold is
        // the FULL stack: hue against keyline (is the hue readable at all),
        // keyline against both terrains (is the hue findable at all), and the
        // glyph against the fill (did we spend the fill's contrast to get
        // here). 3.0 is the floor the map's own tokens use for ink on a panel.
        let sea = Rgba::rgb(0xa8, 0xd0, 0xf0);
        let land = Rgba::rgb(0xe8, 0xe0, 0xd0);
        for affiliation in crate::store::Affiliation::ALL {
            let hue = affiliation_ink(affiliation);
            let fill = affiliation_fill(affiliation, false);
            assert!(
                contrast_ratio(hue, FRAME_KEYLINE) >= 3.0,
                "{affiliation:?} hue is {:.2}:1 on its keyline",
                contrast_ratio(hue, FRAME_KEYLINE)
            );
            for (what, terrain) in [("sea", sea), ("land", land)] {
                assert!(
                    contrast_ratio(FRAME_KEYLINE, terrain) >= 3.0,
                    "{affiliation:?} keyline is {:.2}:1 against {what}",
                    contrast_ratio(FRAME_KEYLINE, terrain)
                );
            }
            assert!(
                contrast_ratio(hue, fill) >= 3.0,
                "{affiliation:?} hue is {:.2}:1 on its own fill",
                contrast_ratio(hue, fill)
            );
        }
        // The casing must actually show outside the hue, or the whole arrangement is
        // a wider single line with extra steps.
        assert!(
            FRAME_KEYLINE_OUT_PX > 0.0,
            "a casing with no outer sliver protects nothing"
        );
        assert_eq!(FRAME_KEYLINE_PX, FRAME_STROKE_PX + FRAME_KEYLINE_OUT_PX);
        // AND it must not be drawn as a wider CENTRED stroke, which is the
        // version that works in the abstract and eats the glyph on screen:
        // centred, a casing this wide extends FRAME_KEYLINE_OUT_PX / 2 into
        // the interior, which measured 55 percent of the glyph's pixels gone.
        // The offset must therefore equal half the difference between the two
        // widths, which is what makes the casing sit wholly outside.
        assert_eq!(
            FRAME_KEYLINE_OFFSET_PX,
            (FRAME_KEYLINE_PX - FRAME_STROKE_PX) / 2.0
        );
        // The run offset is what actually protects the interior: push a point
        // outward and it must move, and only outward.
        let center = (0.0_f32, 0.0_f32);
        let run = vec![(10.0_f32, 0.0_f32), (0.0_f32, 10.0_f32)];
        let cased = offset_run_outward(&run, center, 2.0);
        for (original, moved) in run.iter().zip(&cased) {
            let before = (original.0 - center.0).hypot(original.1 - center.1);
            let after = (moved.0 - center.0).hypot(moved.1 - center.1);
            assert!(
                after > before,
                "the casing moved a point inward: {before} -> {after}"
            );
        }
        // A degenerate point at the centre has no outward direction and must be
        // left alone rather than producing a NaN coordinate.
        let at_centre = offset_run_outward(&[center], center, 2.0);
        assert_eq!(at_centre[0], center, "a point on the centre moved");
        // A stale hull greys its casing but must keep an edge that reads,
        // because "this hull stopped reporting" is still a thing to see.
        for (what, terrain) in [("sea", sea), ("land", land)] {
            assert!(
                contrast_ratio(FRAME_KEYLINE_STALE, terrain) >= 3.0,
                "the stale keyline is {:.2}:1 against {what}",
                contrast_ratio(FRAME_KEYLINE_STALE, terrain)
            );
        }
    }

    #[test]
    fn every_glyph_ink_reads_against_every_affiliation_fill() {
        for affiliation in crate::store::Affiliation::ALL {
            let fill = affiliation_fill(affiliation, false);
            for symbol in crate::store::MapSymbol::ALL {
                let ink = map_symbol_fill(map_symbol_shape(symbol), false);
                let ratio = contrast_ratio(ink, fill);
                assert!(
                    ratio >= 3.0,
                    "{symbol:?} on {affiliation:?} fill is {ratio:.2}:1, under 3:1"
                );
            }
        }
        // And the fill itself must survive the map underneath it, which is the
        // other half: a symbol that reads against its own glyph but vanishes
        // into the sea has not been fixed. Both terrains, because the map is
        // pale in both and the numbers are close enough to be worth pinning.
        let sea = Rgba::rgb(0xa8, 0xd0, 0xf0);
        let land = Rgba::rgb(0xe8, 0xe0, 0xd0);
        for affiliation in crate::store::Affiliation::ALL {
            let fill = affiliation_fill(affiliation, false);
            for (what, terrain) in [("sea", sea), ("land", land)] {
                let ratio = contrast_ratio(fill, terrain);
                assert!(
                    ratio >= 3.0,
                    "{affiliation:?} fill is {ratio:.2}:1 against {what}, under 3:1"
                );
            }
        }
        // Stale greys the fill AND the glyph, and they must still separate —
        // a stale hull is exactly the one an operator needs to pick out.
        let stale_fill = affiliation_fill(crate::store::Affiliation::Friendly, true);
        for symbol in crate::store::MapSymbol::ALL {
            let ink = map_symbol_fill(map_symbol_shape(symbol), true);
            assert!(
                contrast_ratio(ink, stale_fill) >= 3.0,
                "stale {symbol:?} is under 3:1 against the stale fill"
            );
        }
    }

    #[test]
    fn every_affiliation_has_its_own_frame_and_ink() {
        let frames: std::collections::HashSet<_> = crate::store::Affiliation::ALL
            .into_iter()
            .map(|a| frame_for(a))
            .collect();
        assert_eq!(frames.len(), crate::store::Affiliation::ALL.len());
        let inks: std::collections::HashSet<_> = crate::store::Affiliation::ALL
            .into_iter()
            .map(|a| affiliation_ink(a))
            .collect();
        assert_eq!(inks.len(), crate::store::Affiliation::ALL.len());
        // Hostile is red, friendly is blue, unknown is yellow: the three
        // readings an operator already has in their head.
        let ink = |a| affiliation_ink(a);
        assert!(ink(crate::store::Affiliation::Hostile).r > ink(crate::store::Affiliation::Hostile).b);
        assert!(ink(crate::store::Affiliation::Friendly).b > ink(crate::store::Affiliation::Friendly).r);
    }

    #[test]
    fn the_painter_scales_glyphs_to_fill_their_frame() {
        for affiliation in crate::store::Affiliation::ALL {
            let frame = frame_for(affiliation);
            let radius = frame_icon_radius(frame, SYMBOL_BOX_PX);
            let scale = radius / GLYPH_AUTHORING_RADIUS_PX;
            assert!(scale > 0.0 && scale < 2.0, "{affiliation:?} scale {scale}");
            // A glyph never grows past the disc it sits on.
            assert!(5.5 * scale <= symbol_footprint_radius_px() + 1e-9);
        }
        // The widest glyph FILLS its frame, and that is a property of the
        // authored literals rather than of the size or the affiliation:
        //
        //     reach == literal * (icon_radius / GLYPH_AUTHORING_RADIUS_PX)
        //
        // so `reach / icon_radius` is `literal / 8` for every frame at every
        // box, and asserting it against the measured radius measures a
        // constant. That is what the 0.70 floor did, and it failed the tight
        // frames at 0.6875 while telling you it had measured something.
        //
        // So the honest statements are the two that CAN move: the literal has
        // to be most of the authoring radius, or the vocabulary is drawn as a
        // speck in a border; and the result at the box the map actually uses
        // has to be above what the renderer can show as a shape at all.
        assert!(
            5.5 >= 0.65 * GLYPH_AUTHORING_RADIUS_PX,
            "the widest glyph fills only {} of its frame",
            5.5 / GLYPH_AUTHORING_RADIUS_PX
        );
        assert!(
            3.0 >= 0.35 * GLYPH_AUTHORING_RADIUS_PX,
            "the smallest glyph fills only {} of its frame",
            3.0 / GLYPH_AUTHORING_RADIUS_PX
        );
    }

    #[test]
    fn glyphs_grow_with_the_symbol_box() {
        let reach_at = |box_px: f64| {
            frame_icon_radius(frame_for(crate::store::Affiliation::Friendly), box_px)
                / GLYPH_AUTHORING_RADIUS_PX
                * 5.5
        };
        let small = reach_at(SYMBOL_BOX_PX / 2.0);
        let full = reach_at(SYMBOL_BOX_PX);
        assert!(
            full > small * 1.9,
            "doubling the box moved a glyph from {small} px to only {full} px"
        );
        // And the box the map actually draws at is one an operator can read,
        // which the ratio above cannot tell you: a ratio is equally satisfied
        // by a legible symbol and by a 4 px one.
        assert!(
            reach_at(SYMBOL_BOX_PX) >= 6.0,
            "at {SYMBOL_BOX_PX} px the widest glyph reaches only {} px, which is \
             below the ~6 px the renderer can show as a shape",
            reach_at(SYMBOL_BOX_PX)
        );
    }

    /// A marker with nothing but an identity and a Representation. The
    /// picking tests care about geometry and order, not about fixes.
    fn marker_stub() -> ShipMarker {
        ShipMarker {
            id: String::new(),
            x: 0.0,
            y: 0.0,
            stale: false,
            old_data: false,
            source: crate::geo::track::FixSource::Wire,
            trail: Vec::new(),
            heading_deg: None,
            map_symbol: crate::store::MapSymbol::UnknownShip,
            latitude: 0.0,
            longitude: 0.0,
            footprint_px: 0.0,
            affiliation: crate::store::Affiliation::Unknown,
            planned: false,
            label: String::new(),
            lod: crate::map_render::UnitLod::Far,
            visual: None,
        }
    }

    #[test]
    fn pick_order_is_the_reverse_of_paint_order() {
        let mut markers: Vec<ShipMarker> = Vec::new();
        // Deliberately jumbled: Middle, Near, Far, Near, Far.
        for (i, lod) in [
            UnitLod::Middle,
            UnitLod::Near,
            UnitLod::Far,
            UnitLod::Near,
            UnitLod::Far,
        ]
        .into_iter()
        .enumerate()
        {
            markers.push(ShipMarker {
                id: format!("m{i}"),
                lod,
                x: i as f64 * 100.0,
                y: 0.0,
                ..marker_stub()
            });
        }
        let order = paint_order(&markers);
        let ranks: Vec<u8> = order.iter().map(|&i| match markers[i].lod {
            UnitLod::Far => 0,
            UnitLod::Middle => 1,
            UnitLod::Near => 2,
        }).collect();
        assert_eq!(ranks, vec![0, 0, 1, 2, 2], "paint order is Far, Middle, Near");
        // Same Representation keeps its original relative order.
        assert!(order[0] < order[1], "Far markers keep list order");
        assert!(order[3] < order[4], "Near markers keep list order");
        // And picking walks that list backwards, so the marker painted
        // LAST is picked first — here the last Near in list order.
        assert_eq!(*order.last().unwrap(), 3);
        assert_eq!(*order.first().unwrap(), 2);
    }

    #[test]
    fn a_group_draws_a_kind_only_on_a_strict_majority() {
        use crate::store::MapSymbol::{Corvette, Destroyer, Plane, Submarine};
        assert_eq!(
            plurality_icon(&[Corvette, Corvette, Corvette, Corvette, Corvette, Corvette, Destroyer]),
            Some(Corvette),
            "six corvettes beat one destroyer"
        );
        // Mixed, and an exact tie: no icon, so the frame stays empty
        // rather than naming a wrong kind.
        assert_eq!(plurality_icon(&[Corvette, Corvette, Plane, Plane]), None);
        // Exactly half is still not a majority.
        assert_eq!(plurality_icon(&[Corvette, Plane]), None);
        // One member alone IS a majority of one — the rule is about
        // agreement, not about being a crowd.
        assert_eq!(plurality_icon(&[Submarine]), Some(Submarine));
        assert_eq!(plurality_icon(&[]), None);
    }
}
