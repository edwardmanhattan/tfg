//! The ring pack: a group dropped on the map, as pure geometry.
//!
//! A HULL DROPPED ON THE MAP IS ONE PLACEMENT; A GROUP IS A PACK. That is the
//! whole of this module. The operator drags a satgas or a gugus onto the map,
//! and its hulls are laid out around the drop point instead of landing on top
//! of each other — one hull at the anchor, the rest on rings around it, and a
//! sub-group given its own centre and packed inside its parent's ring
//! (`docs/creation-flow-handoff.md` §6.4).
//!
//! THE RULES, in the sheet's own words:
//!
//! ```text
//! anchor   = the drop point
//! ring k   holds 6k hulls, at radius k·R
//! packing  = depth-first through the group tree; within a ring, unit-id order
//! spread   = 360° ÷ N, starting at 12 o'clock, for a partial ring
//! R        ≈ 0.02° of latitude; longitude scaled by 1 ÷ cos(lat)
//! writing  = every hull's position flattened, sent one at a time in unit-id order
//! ```
//!
//! TWO ORDERS, AND THEY ARE NOT THE SAME ORDER. Packing is geometry: it walks
//! the tree so a sub-group's hulls land together, and sorts within a ring by
//! unit id so the layout is deterministic. Writing is the wire: there is no
//! batch route, so the commit sends N placements one at a time in UNIT-ID
//! ORDER, which is the only ordering a resumed press can reproduce. [`Pack`]
//! carries both, and [`Pack::writes`] is the one the commit uses — a pack that
//! walked the tree differently must not change the write sequence, and a
//! failed write at position k must not shift the ones after it.
//!
//! NOTHING ABOUT A CLUSTER SURVIVES. The server sees N placements and no group
//! geometry, so nothing here is stored: reopening a game shows the hulls, not
//! a ring. A group has no map marker and no stored anchor — this pack is the
//! only thing a group does to the map.
//!
//! A pure module with its own tests: no network, no draft, no pixel.

use std::collections::BTreeMap;

use crate::force::Start;

/// `R` — the ring radius, in degrees of latitude.
///
/// Degrees of latitude rather than metres because the map is a lat/lon
/// surface and 0.02° is about 2.2 km: a cluster of a dozen hulls is then a
/// dozen distinguishable pins at a planning zoom, which is the whole point of
/// packing instead of stacking.
pub const RING_RADIUS_DEG: f64 = 0.02;

/// How many slots ring `k` holds. The centre is not a ring: 0, 6, 12, 18.
pub fn capacity(ring: usize) -> usize {
    if ring == 0 { 0 } else { 6 * ring }
}

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

/// Where one slot of a body's layout sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    /// 0 is the anchor itself; 1 and up are rings.
    pub ring: usize,
    /// This slot's place within its ring, 0-based, clockwise from 12 o'clock.
    pub index: usize,
    /// How many slots the ring holds IN THIS BODY. For a partial ring this is
    /// smaller than [`capacity`], and it is what the spread divides into.
    pub count: usize,
}

impl Slot {
    /// The radius this slot sits at, in degrees of latitude.
    pub fn radius(self) -> f64 {
        RING_RADIUS_DEG * self.ring as f64
    }

    /// This slot's bearing from the anchor, clockwise from 12 o'clock.
    pub fn bearing_deg(self) -> f64 {
        if self.ring == 0 {
            0.0
        } else {
            self.index as f64 * 360.0 / self.count as f64
        }
    }
}

/// Where slot `slot` of a body holding `slots` things sits.
///
/// The first slot takes the anchor — `5 HULLS · ANCHOR + FOUR AT 90°` — and
/// the rest fill ring 1, then ring 2, and so on. A ring that is not full
/// spreads its own slots evenly: four on ring 1 sit at 90°, six at 60°, and a
/// full ring is the special case rather than a separate rule.
///
/// `slot` at or past `slots` answers the last ring rather than panicking: this
/// is geometry called from a drag, and a pin in the wrong place is a better
/// failure than a crash mid-gesture.
pub fn slot_of(slot: usize, slots: usize) -> Slot {
    if slot == 0 || slots <= 1 {
        return Slot {
            ring: 0,
            index: 0,
            count: 1,
        };
    }
    let mut ring = 1;
    // Slots consumed before this ring: the centre, then every full ring.
    let mut before = 1;
    loop {
        let on_this = capacity(ring).min(slots.saturating_sub(before));
        if slot < before + on_this {
            return Slot {
                ring,
                index: slot - before,
                count: on_this,
            };
        }
        if on_this == 0 {
            // Nothing left to fill: answer the last ring's last place.
            return Slot {
                ring,
                index: 0,
                count: 1,
            };
        }
        before += on_this;
        ring += 1;
    }
}

/// The point `slot` of `slots` away from `anchor`.
pub fn place(anchor: Start, slot: Slot) -> Start {
    if slot.ring == 0 {
        return anchor;
    }
    let radius = slot.radius();
    let theta = slot.bearing_deg().to_radians();
    // 1 ÷ cos(lat) converts an angular north-distance into the equivalent
    // east-distance. Near the poles the cosine goes to zero and the scale to
    // infinity, so it is floored: a pin that lands a little short of the ring
    // is still a pin on the map.
    let cos_lat = anchor.lat.to_radians().cos().abs().max(1e-6);
    Start {
        lat: (anchor.lat + radius * theta.cos()).clamp(-90.0, 90.0),
        lon: wrap_lon(anchor.lon + radius * theta.sin() / cos_lat),
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

/// Place one body around its own anchor, then recurse into its children with
/// the anchors their slots gave them.
fn place_body(body: &Body, anchor: Start, out: &mut Vec<(i64, Start)>) {
    let mut hulls = body.hulls.clone();
    hulls.sort_unstable();
    hulls.dedup();
    let slots = hulls.len() + body.children.len();
    for (at, unit) in hulls.iter().enumerate() {
        out.push((*unit, place(anchor, slot_of(at, slots))));
    }
    for (at, child) in body.children.iter().enumerate() {
        let slot = slot_of(hulls.len() + at, slots);
        // A sub-group keeps its own centre: it is an anchor on its parent's
        // ring, and it packs its own children around that point, recursively.
        place_body(child, place(anchor, slot), out);
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

    fn distance(a: Start, b: Start) -> f64 {
        ((a.lat - b.lat).powi(2) + (a.lon - b.lon).powi(2)).sqrt()
    }

    /// `0 AT THE CENTRE, 6 ON R, 12 ON 2R, 18 ON 3R`.
    #[test]
    fn capacity_is_six_per_ring() {
        assert_eq!(capacity(0), 0);
        assert_eq!(capacity(1), 6);
        assert_eq!(capacity(2), 12);
        assert_eq!(capacity(3), 18);
    }

    /// One hull needs no ring: it is the drop point, exactly.
    #[test]
    fn one_hull_is_the_anchor() {
        let anchor = at(-6.1, 106.8);
        let pack = pack(&Body::leaf(1, 351), anchor);
        assert_eq!(pack.len(), 1);
        assert_eq!(pack.get(351), Some(anchor));
        assert!(near(pack.writes[0].1, anchor));
    }

    /// `5 HULLS · ANCHOR + FOUR AT 90°` — and the cardinals are what give the
    /// cluster its vertical and its horizontal axis.
    #[test]
    fn five_hulls_are_the_anchor_and_four_cardinals() {
        let anchor = at(0.0, 0.0);
        let body = Body::new(1, vec![355, 351, 354, 352, 353], Vec::new());
        let pack = pack(&body, anchor);
        assert_eq!(pack.len(), 5);
        // Slot 0 is the lowest unit id: the walk sorts within a body.
        assert_eq!(pack.get(351), Some(anchor));
        let east = RING_RADIUS_DEG;
        assert!(
            near(pack.get(352).unwrap(), at(RING_RADIUS_DEG, 0.0)),
            "slot 1 is 12 o'clock: {:?}",
            pack.get(352)
        );
        assert!(near(pack.get(353).unwrap(), at(0.0, east)), "3 o'clock");
        assert!(
            near(pack.get(354).unwrap(), at(-RING_RADIUS_DEG, 0.0)),
            "6 o'clock"
        );
        assert!(near(pack.get(355).unwrap(), at(0.0, -east)), "9 o'clock");
    }

    /// `A FULL RING IS THE SPECIAL CASE — THE SAME RULE, 360° ÷ 6 = 60°`.
    #[test]
    fn a_full_ring_spreads_at_sixty_degrees() {
        let anchor = at(0.0, 0.0);
        let body = Body::new(1, (1..=7).collect(), Vec::new());
        let pack = pack(&body, anchor);
        assert_eq!(pack.len(), 7);
        let sixty = 60f64.to_radians();
        // Slot 1 is 12 o'clock and slot 2 is 60° clockwise of it.
        assert!(near(pack.get(2).unwrap(), at(RING_RADIUS_DEG, 0.0)));
        assert!(
            near(
                pack.get(3).unwrap(),
                at(RING_RADIUS_DEG * sixty.cos(), RING_RADIUS_DEG * sixty.sin())
            ),
            "60° clockwise: {:?}",
            pack.get(3)
        );
        // And every one of them is on the ring.
        for unit in 2..=7 {
            let p = pack.get(unit).unwrap();
            assert!(
                (distance(p, anchor) - RING_RADIUS_DEG).abs() < EPS,
                "hull {unit} is on R: {p:?}"
            );
        }
    }

    /// Capacity, read as rings: the eighth hull is the first on 2R, alone on
    /// its ring, so it sits at 12 o'clock.
    #[test]
    fn the_seventh_hull_opens_the_second_ring() {
        assert_eq!(slot_of(6, 7).ring, 1);
        assert_eq!(slot_of(7, 8).ring, 2);
        assert_eq!(slot_of(7, 8).radius(), 2.0 * RING_RADIUS_DEG);
        assert_eq!(slot_of(7, 8).bearing_deg(), 0.0);
        assert_eq!(slot_of(19, 20).ring, 3, "the twentieth hull opens 3R");
    }

    /// Longitude is scaled by 1 ÷ cos(lat): the same radius is a wider step
    /// in degrees of longitude away from the equator.
    #[test]
    fn longitude_is_scaled_by_the_cosine() {
        let anchor = at(60.0, 10.0);
        // Five hulls put four on ring 1 at 90°, so the second of them is due
        // east of the anchor.
        let body = Body::new(1, (1..=5).collect(), Vec::new());
        let pack = pack(&body, anchor);
        let east = pack.get(3).unwrap();
        assert!(
            (east.lat - 60.0).abs() < 1e-8,
            "no latitude on a due-east step"
        );
        assert!(
            (east.lon - (10.0 + RING_RADIUS_DEG / 0.5)).abs() < 1e-8,
            "1 ÷ cos(60°) doubles the longitude step: {}",
            east.lon
        );
    }

    /// The walk is the tree's order and the writes are the wire's: a sub-group
    /// keeps its hulls together, and the commit still sends by unit id.
    #[test]
    fn the_walk_is_the_trees_and_the_writes_are_by_unit_id() {
        let anchor = at(-6.1, 106.8);
        let body = Body::new(
            1,
            vec![40, 10],
            vec![Body::new(2, vec![30, 20], Vec::new())],
        );
        let pack = pack(&body, anchor);
        assert_eq!(
            pack.walk.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![10, 40, 20, 30],
            "the body's own hulls, then the subtree"
        );
        assert_eq!(
            pack.writes.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            vec![10, 20, 30, 40],
            "the wire is unit-id order"
        );
        assert_eq!(
            pack.get(10),
            Some(anchor),
            "the pack's placement is the write's placement"
        );
    }

    /// `SAME RULE AT EVERY DEPTH — A SUB-GROUP PACKED INSIDE A RING KEEPS ITS
    /// OWN CENTRE`: its first hull is its anchor, and its second is R from
    /// that point, not from the root's.
    #[test]
    fn a_sub_group_keeps_its_own_centre() {
        let anchor = at(0.0, 0.0);
        let body = Body::new(1, vec![1], vec![Body::new(2, vec![2, 3], Vec::new())]);
        let pack = pack(&body, anchor);
        let child_anchor = pack.get(2).unwrap();
        assert!(
            near(child_anchor, at(RING_RADIUS_DEG, 0.0)),
            "the subgroup's centre is a slot on the parent's ring: {child_anchor:?}"
        );
        let grandchild = pack.get(3).unwrap();
        assert!(
            near(grandchild, at(2.0 * RING_RADIUS_DEG, 0.0)),
            "and its own hull is R from ITS centre: {grandchild:?}"
        );
        assert!((distance(child_anchor, grandchild) - RING_RADIUS_DEG).abs() < EPS);
    }

    /// Nothing in, nothing out: a body with no hulls and no children places
    /// no pins, which is what an empty group dropped on the map must do.
    #[test]
    fn an_empty_body_places_nothing() {
        let pack = pack(&Body::new(1, Vec::new(), Vec::new()), at(0.0, 0.0));
        assert!(pack.is_empty());
        assert!(pack.walk.is_empty());
    }

    /// A hull named twice is one pin, because there is no batch route and two
    /// writes for one hull would move it twice.
    #[test]
    fn a_repeated_hull_is_one_write() {
        let body = Body::new(1, vec![351], vec![Body::new(2, vec![351, 352], Vec::new())]);
        let pack = pack(&body, at(0.0, 0.0));
        assert_eq!(pack.writes.len(), 2, "{:?}", pack.writes);
        assert_eq!(pack.walk.len(), 3, "the walk still shows what happened");
    }

    /// The pack is deterministic: the same tree and anchor are the same pins,
    /// whatever order the caller listed the hulls in.
    #[test]
    fn the_pack_is_deterministic() {
        let a = Body::new(1, vec![353, 351, 352], Vec::new());
        let b = Body::new(1, vec![351, 352, 353], Vec::new());
        let anchor = at(-3.2841, 108.9214);
        assert_eq!(pack(&a, anchor), pack(&b, anchor));
    }

    /// Near the pole the longitude scale is floored rather than infinite, and
    /// every pin stays on the map the placement route accepts.
    #[test]
    fn a_pack_at_the_pole_stays_on_the_map() {
        let body = Body::new(1, (1..=8).collect(), Vec::new());
        let pack = pack(&body, at(89.99, 179.99));
        for (unit, at) in &pack.writes {
            assert!((-90.0..=90.0).contains(&at.lat), "hull {unit}: {at:?}");
            assert!((-180.0..=180.0).contains(&at.lon), "hull {unit}: {at:?}");
        }
    }
}
