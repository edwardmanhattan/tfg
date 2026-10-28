//! The drop formation: a group dropped on the map, as pure geometry.
//!
//! A HULL DROPPED ON THE MAP IS ONE PLACEMENT; A GROUP IS A FORMATION. That is
//! the whole of this module. The operator drags a satuan tugas or a gugus onto
//! the map, and its hulls are laid out as a **wedge** around the drop point
//! instead of landing on top of each other — or on a line, which is what the
//! old ring's partial spread degenerated to for two or three hulls.
//!
//! THE SHAPE, and why this one:
//!
//! ```text
//! anchor   = the drop point, at the FORMATION'S CENTRE
//! shape    = one wedge for the whole body: a lead, then pairs alternating
//!            port and starboard, each rank one gap further aft and outboard
//!            (half-angle 30°)
//! gap      = 100 m between neighbouring hulls, along an arm and across a rank
//! packing  = depth-first through the group tree; within a body, unit-id order
//! writing  = every hull's position flattened, sent one at a time in unit-id order
//! ```
//!
//! A WEDGE IS A FORMATION, NOT A DECORATION. It is what a screen ahead of a
//! guide looks like, it is symmetric when the count is odd and a diamond when
//! it is even, and it never collapses to a straight line: the smallest groups
//! are a pair abreast (two), a triangle (three) and a diamond (four). The old
//! ring spread `360° ÷ N` from 12 o'clock, so three hulls sat at 0°, 90° and
//! 180° — a lead and two on a line through it — and two hulls were a line by
//! definition.
//!
//! THE CENTRE IS THE DROP. The first hull used to take the anchor exactly,
//! which put the formation's mass ahead of the pointer; the whole wedge is
//! translated so its centroid sits on the drop instead.
//!
//! 100 METRES IS THE GAP, not the ring radius. The old pack spaced ships at
//! about 2.2 km, which at a planning zoom is a screen apart and at a working
//! zoom is off the water the operator is looking at. A hundred metres is about
//! a hull's length: close enough to read as one body, far enough that the pins
//! do not overlap when the chart is zoomed to the formation.
//!
//! TWO ORDERS, AND THEY ARE NOT THE SAME ORDER. Packing is geometry: it walks
//! the tree so a sub-group's hulls land together, and sorts within a body by
//! unit id so the layout is deterministic. Writing is the wire: there is no
//! batch route, so the commit sends N placements one at a time in UNIT-ID
//! ORDER, which is the only ordering a resumed press can reproduce. [`Pack`]
//! carries both, and [`Pack::writes`] is the one the commit uses — a pack that
//! walked the tree differently must not change the write sequence, and a
//! failed write at position k must not shift the ones after it.
//!
//! NOTHING ABOUT A FORMATION SURVIVES. The server sees N placements and no
//! group geometry, so nothing here is stored: reopening a game shows the hulls,
//! not a wedge. A group has no map marker and no stored anchor — this pack is
//! the only thing a group does to the map.
//!
//! A pure module with its own tests: no network, no draft, no pixel.

use std::collections::BTreeMap;

use crate::force::Start;

/// One gap: 100 metres, in degrees of latitude.
///
/// A degree of latitude is 111.32 km by the 1984 ellipsoid's mean, and the
/// pack is a planning gesture rather than a survey: a gap a few centimetres
/// long is still a hundred metres on the water. Longitude is scaled by
/// 1 ÷ cos(lat) at the anchor, because a degree of longitude is shorter away
/// from the equator.
pub const GAP_M: f64 = 100.0;
pub const GAP_DEG: f64 = GAP_M / 111_320.0;

/// The half-angle of the wedge's arms, in degrees from the axis.
///
/// Thirty degrees makes the distance between neighbouring hulls — lead to
/// first rank, and along an arm — exactly one gap:
/// `sqrt(sin²30 + cos²30) = 1`.
pub const HALF_ANGLE_DEG: f64 = 30.0;

/// One body of the task organisation as the pack sees it.
///
/// `hulls` are the hulls the body holds DIRECTLY, and `children` are the
/// bodies under it. An `unsur` is a hull and never a container, so there is no
/// Unsur node: a hull is a leaf, and only a body can hold anything.
///
/// The caller's tree is a forest with one hull in one body — [`crate::groups`]
/// refuses anything else at stage time, which is where the drag is. A hull
/// listed twice would be written twice, so [`Pack::writes`] keeps one position
/// per hull: the last one the walk gave it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Body {
    /// The group's id, carried so a caller can build a tree and read it back.
    pub id: i64,
    pub hulls: Vec<i64>,
    pub children: Vec<Body>,
}

impl Body {
    pub fn new(id: i64, hulls: Vec<i64>, children: Vec<Body>) -> Self {
        Self {
            id,
            hulls,
            children,
        }
    }

    /// A hull leaf's body: a body that holds exactly one hull and nothing
    /// else. The map's ordinary drop.
    pub fn leaf(id: i64, hull: i64) -> Self {
        Self::new(id, vec![hull], Vec::new())
    }

    /// Every hull under this body, in walk order.
    pub fn hulls_below(&self) -> Vec<i64> {
        let mut out: Vec<i64> = self.hulls.clone();
        for child in &self.children {
            out.extend(child.hulls_below());
        }
        out
    }
}

/// The slots of a formation of `count` things, in slot order, as
/// `(north, east)` offsets in GAP units, translated so their centroid is
/// `(0, 0)`.
///
/// THE ORDER IS THE SHAPE'S. Slot 0 is the lead of an odd formation and the
/// port hand of the first rank of an even one; after that the slots alternate
/// port and starboard, one rank at a time. A body's hulls and its child bodies
/// share the same slots, so a sub-group's centre is a position in its parent's
/// wedge exactly as a hull is.
///
/// A count of zero answers nothing rather than panicking: this is geometry
/// called from a drag, and an empty body is a refusal the caller makes, not a
/// crash mid-gesture.
pub fn slots(count: usize) -> Vec<(f64, f64)> {
    if count == 0 {
        return Vec::new();
    }
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(count);
    // ODD: a lead, then pairs. EVEN: pairs alone, which is a diamond rather
    // than a lopsided V with one arm longer than the other.
    if count % 2 == 1 {
        points.push((0.0, 0.0));
    }
    let half = HALF_ANGLE_DEG.to_radians();
    let (outboard, aft) = (half.sin(), half.cos());
    let mut rank = 1usize;
    while points.len() < count {
        let r = rank as f64;
        // Port is negative east. The pair is placed port first so the slot
        // order reads left-to-right on the first rank and then aft.
        points.push((-aft * r, -outboard * r));
        if points.len() < count {
            points.push((-aft * r, outboard * r));
        }
        rank += 1;
    }
    // CENTRE THE WEDGE ON THE DROP. Without this the lead is the anchor and
    // every other hull is behind it, so the formation lands ahead of the
    // pointer that asked for it.
    let n = points.len() as f64;
    let north: f64 = points.iter().map(|(north, _)| *north).sum::<f64>() / n;
    let east: f64 = points.iter().map(|(_, east)| *east).sum::<f64>() / n;
    points
        .into_iter()
        .map(|(north_at, east_at)| (north_at - north, east_at - east))
        .collect()
}

/// The point `offset` GAP units away from `anchor`.
///
/// `offset` is `(north, east)`; the east component is divided by the cosine of
/// the anchor's latitude, because a degree of longitude is shorter away from
/// the equator. Near the poles the cosine goes to zero and the scale to
/// infinity, so it is floored: a pin that lands a little short of its slot is
/// still a pin on the map.
pub fn place(anchor: Start, offset: (f64, f64)) -> Start {
    let d_lat = offset.0 * GAP_DEG;
    let cos_lat = anchor.lat.to_radians().cos().abs().max(1e-6);
    Start {
        lat: (anchor.lat + d_lat).clamp(-90.0, 90.0),
        lon: wrap_lon(anchor.lon + offset.1 * GAP_DEG / cos_lat),
    }
}

/// A longitude kept on the map: the server refuses ±180 and beyond.
fn wrap_lon(lon: f64) -> f64 {
    let wrapped = ((lon + 180.0).rem_euclid(360.0)) - 180.0;
    if wrapped == -180.0 { 180.0 } else { wrapped }
}

/// The pack, as the two orders it has.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pack {
    /// The walk: depth-first through the tree, so a sub-group's hulls land
    /// together. Duplicates are kept here, because this is what happened.
    pub walk: Vec<(i64, Start)>,
    /// The wire: one position per hull, in UNIT-ID ORDER. This is the order
    /// the commit sends them in, one placement write at a time.
    pub writes: Vec<(i64, Start)>,
}

impl Pack {
    /// Where a hull was packed, or `None` when it was not in the tree.
    pub fn get(&self, unit_id: i64) -> Option<Start> {
        self.writes
            .binary_search_by_key(&unit_id, |(id, _)| *id)
            .ok()
            .map(|at| self.writes[at].1)
    }

    /// How many hulls the pack places.
    pub fn len(&self) -> usize {
        self.writes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }
}

/// Pack a body's hulls around `anchor`.
///
/// A group dropped on a group's row packs THAT group's hulls; the caller
/// chooses the root, so a satgas dragged out of a gugus packs its own hulls
/// and not its parent's.
pub fn pack(body: &Body, anchor: Start) -> Pack {
    let mut walk = Vec::new();
    place_body(body, anchor, &mut walk);
    // One position per hull, in unit-id order: the last slot a hull was given
    // wins, which keeps a tree that repeats a hull from writing it twice.
    let mut by_unit: BTreeMap<i64, Start> = BTreeMap::new();
    for (unit, at) in &walk {
        by_unit.insert(*unit, *at);
    }
    Pack {
        walk,
        writes: by_unit.into_iter().collect(),
    }
}

/// Place the body's whole force as ONE wedge around the anchor.
///
/// A SUB-GROUP DOES NOT GET A WEDGE OF ITS OWN. It used to: each child took a
/// slot in its parent's wedge and packed its own hulls around that point — and
/// a child's outboard hull then landed inside its sibling's arm, 29 m from it
/// in a five-hull gugus. Two formations nested inside each other cannot both
/// keep a hundred-metre gap unless the parent's slots grow with each child's
/// footprint, which makes the parent's shape depend on every subtree's size.
/// One wedge for the whole body is what keeps the gap true, and the tree is
/// not lost: the map draws its links (`hierarchy_links`), which is what the
/// organisation looks like on the water anyway.
///
/// The walk is still depth-first — a body's own hulls, then each child's —
/// so a sub-group's hulls take consecutive slots even though they are not a
/// cluster of their own.
fn place_body(body: &Body, anchor: Start, out: &mut Vec<(i64, Start)>) {
    let mut hulls: Vec<i64> = Vec::new();
    walk_hulls(body, &mut hulls);
    let slots = slots(hulls.len());
    for (at, unit) in hulls.iter().enumerate() {
        out.push((*unit, place(anchor, slots[at])));
    }
}

/// The body's hulls, depth-first, each body's own list in unit-id order.
fn walk_hulls(body: &Body, out: &mut Vec<i64>) {
    let mut own = body.hulls.clone();
    own.sort_unstable();
    own.dedup();
    out.extend(own);
    for child in &body.children {
        walk_hulls(child, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tolerance for a degree, which is a few metres at these scales.
    const EPS: f64 = 1e-9;

    fn at(lat: f64, lon: f64) -> Start {
        Start { lat, lon }
    }

    fn near(a: Start, b: Start) -> bool {
        (a.lat - b.lat).abs() < EPS && (a.lon - b.lon).abs() < EPS
    }

    /// Distance in metres on a flat patch at the equator, where a degree of
    /// latitude and a degree of longitude are both 111.32 km.
    fn metres(a: Start, b: Start) -> f64 {
        let d_lat = (a.lat - b.lat) * 111_320.0;
        let d_lon = (a.lon - b.lon) * 111_320.0;
        (d_lat * d_lat + d_lon * d_lon).sqrt()
    }

    /// THE SMALLEST FORMATIONS ARE NOT LINES. This is the regression the
    /// module was rewritten for: two hulls abreast, three a triangle, four a
    /// diamond — never three points on one line.
    #[test]
    fn the_small_formations_are_not_lines() {
        let anchor = at(0.0, 0.0);
        // Two: abreast, one gap apart.
        let pair = pack(&Body::new(1, vec![351, 352], Vec::new()), anchor);
        let a = pair.get(351).expect("a");
        let b = pair.get(352).expect("b");
        assert!((metres(a, b) - GAP_M).abs() < 0.5, "two abreast");
        assert!((a.lat - b.lat).abs() < EPS, "on the same parallel");

        // Three: a triangle, so the area of the three is not zero.
        let three = pack(&Body::new(1, vec![351, 352, 353], Vec::new()), anchor);
        let p: Vec<(f64, f64)> = [351, 352, 353]
            .iter()
            .map(|id| {
                let at = three.get(*id).expect("packed");
                (at.lat, at.lon)
            })
            .collect();
        let cross = (p[1].0 - p[0].0) * (p[2].1 - p[0].1)
            - (p[1].1 - p[0].1) * (p[2].0 - p[0].0);
        assert!(cross.abs() > 1e-12, "three hulls, not a line: {p:?}");

        // Four: a diamond — no two ranks on one line, as a lopsided V would be.
        let four = pack(&Body::new(1, (1..=4).collect(), Vec::new()), anchor);
        let ranks: Vec<f64> = [1, 2, 3, 4]
            .iter()
            .map(|id| four.get(*id).expect("packed").lat)
            .collect();
        assert!(
            ranks.iter().any(|lat| (lat - ranks[0]).abs() > EPS),
            "the ranks differ"
        );
    }

    /// EVERY GAP IS A HUNDRED METRES: lead to first rank, and neighbour to
    /// neighbour along an arm.
    #[test]
    fn the_gap_is_one_hundred_metres() {
        let anchor = at(0.0, 0.0);
        let body = Body::new(1, vec![1, 2, 3, 4, 5, 6, 7], Vec::new());
        let pack = pack(&body, anchor);
        let mut placed: Vec<Start> = (1..=7)
            .filter_map(|id| pack.get(id))
            .collect();
        for a in 0..placed.len() {
            for b in (a + 1)..placed.len() {
                let gap = metres(placed[a], placed[b]);
                assert!(
                    gap > GAP_M * 0.99,
                    "hulls {a} and {b} are {} m apart, inside the gap",
                    gap
                );
            }
        }
        // The closest pair is exactly one gap, not more: a wedge that spreads
        // wider than its gap is a ring with extra steps.
        placed.sort_by(|a, b| a.lat.partial_cmp(&b.lat).unwrap());
        let closest = (0..placed.len() - 1)
            .map(|i| metres(placed[i], placed[i + 1]))
            .fold(f64::INFINITY, f64::min);
        assert!((closest - GAP_M).abs() < 0.5, "closest pair: {closest} m");
    }

    /// THE WEDGE IS CENTRED ON THE DROP, so the formation's mass lands under
    /// the pointer rather than ahead of it.
    #[test]
    fn the_formation_is_centred_on_the_drop() {
        let anchor = at(-6.1, 106.8);
        for count in 1..=9 {
            let body = Body::new(1, (1..=count as i64).collect(), Vec::new());
            let pack = pack(&body, anchor);
            let n = pack.len() as f64;
            let lat: f64 = pack.writes.iter().map(|(_, at)| at.lat).sum::<f64>() / n;
            let lon: f64 = pack.writes.iter().map(|(_, at)| at.lon).sum::<f64>() / n;
            assert!(
                (lat - anchor.lat).abs() < 1e-9 && (lon - anchor.lon).abs() < 1e-9,
                "{count} hulls centred on the drop: {lat}, {lon}"
            );
        }
    }

    /// ONE HULL NEEDS NO FORMATION: it is the drop point, exactly.
    #[test]
    fn one_hull_is_the_anchor() {
        let anchor = at(-6.1, 106.8);
        let pack = pack(&Body::leaf(1, 351), anchor);
        assert_eq!(pack.len(), 1);
        assert!(
            near(pack.get(351).expect("the hull"), anchor),
            "one hull lands on the drop"
        );
        assert!(near(pack.writes[0].1, anchor));
    }

    /// LONGITUDE IS SCALED BY THE LATITUDE, so a wedge at 60° north is as
    /// wide on the ground as one at the equator.
    #[test]
    fn longitude_is_scaled_by_the_latitude() {
        let anchor = at(60.0, 10.0);
        let body = Body::new(1, vec![1, 2], Vec::new());
        let pack = pack(&body, anchor);
        let a = pack.get(1).expect("a");
        let b = pack.get(2).expect("b");
        // Two abreast: the east-west separation is one gap on the ground.
        let d_lon = (a.lon - b.lon).abs() * 111_320.0 * 60f64.to_radians().cos();
        assert!((d_lon - GAP_M).abs() < 0.5, "{d_lon} m apart");
    }

    /// THE WALK IS DEPTH-FIRST and the write is by unit id — two different
    /// orders, and the commit uses the second.
    #[test]
    fn the_walk_is_the_tree_and_the_write_is_the_id() {
        let anchor = at(0.0, 0.0);
        let body = Body::new(
            1,
            vec![9, 7],
            vec![Body::new(2, vec![5], Vec::new()), Body::leaf(3, 3)],
        );
        let pack = pack(&body, anchor);
        assert_eq!(
            pack.walk.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![7, 9, 5, 3],
            "the body's hulls in id order, then its children"
        );
        assert_eq!(
            pack.writes.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![3, 5, 7, 9],
            "one write per hull, in unit-id order"
        );
    }

    /// A SUB-GROUP IS NOT A CLUSTER OF ITS OWN: its hulls take consecutive
    /// slots in the body's single wedge, and the whole pack keeps the gap.
    ///
    /// The nested version of this put a child's outboard hull inside its
    /// sibling's arm — 29 m apart in a five-hull gugus — which is exactly the
    /// overlap the gap exists to prevent.
    #[test]
    fn a_sub_groups_hulls_share_the_one_wedge() {
        let anchor = at(0.0, 0.0);
        let children = vec![
            Body::new(2, vec![10, 11, 12], Vec::new()),
            Body::new(3, vec![20, 21], Vec::new()),
        ];
        let body = Body::new(1, vec![9], children);
        let pack = pack(&body, anchor);
        let placed: Vec<Start> = [9, 10, 11, 12, 20, 21]
            .iter()
            .map(|id| pack.get(*id).expect("packed"))
            .collect();
        for a in 0..placed.len() {
            for b in (a + 1)..placed.len() {
                let gap = metres(placed[a], placed[b]);
                assert!(gap > GAP_M * 0.99, "{} m apart: {gap}", gap);
            }
        }
        // And the walk is depth-first: the parent's hull, then the first
        // child's three, then the second's two.
        assert_eq!(
            pack.walk.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![9, 10, 11, 12, 20, 21]
        );
    }

    /// The pack is deterministic: the same tree and the same drop answer the
    /// same positions, which is what a resumed press needs.
    #[test]
    fn the_same_drop_is_the_same_formation() {
        let body = Body::new(1, vec![3, 1, 2], vec![Body::new(2, vec![5, 4], Vec::new())]);
        let one = pack(&body, at(-6.1, 106.8));
        let two = pack(&body, at(-6.1, 106.8));
        assert_eq!(one, two);
    }
}
