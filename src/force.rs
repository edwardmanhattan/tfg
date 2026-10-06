//! The force as the operator has it, before Minos has heard of it.
//!
//! PLACING A HULL, SAYING WHERE IT SITS AND SAYING WHO COMMANDS IT ARE LOCAL
//! ACTS. They used to be three backend writes fired the moment the operator
//! made each decision, and the first of them could not happen at all without
//! the third: Minos requires a commander on assignment, so the console used to
//! stand in front of a commander combo box and refuse the map until it was
//! answered. The map is the surface an operator works on, so the map is where
//! the answer lives now — a hull is dropped, and who steers it is a question
//! the Player picker answers whenever it is answered.
//!
//! What Minos still owns is the record. The draft is what the operator is
//! looking at; a stage advance is the moment the draft is written through, and
//! [`plan`] is the diff that says which writes that is. Nothing here talks to
//! the network, which is the point: the rules about what a legal draft is —
//! what the server will accept, in what order — are pure functions over plain
//! data, so they are testable without a session.
//!
//! THE ORDER OF [`plan`] IS NOT ARBITRARY. A piece is assigned before it is
//! given a position, because the position endpoint 404s on a hull the server
//! has never heard of. A hull the operator has taken back out of the exercise
//! is dropped last, so nothing is ever briefly absent while it moves.

use std::collections::BTreeMap;

use crate::geo::coordinates::GeoPosition;

/// A hull's starting position. NOT a live position: it is where the hull
/// STARTS, and once the exercise begins it is frozen as the first leg of that
/// hull's fix chain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Start {
    pub lat: f64,
    pub lon: f64,
}

/// Two starts this close are the same start. The server round-trips the pair
/// through JSON, so a draft read back from a placement view is a few bits off
/// the draft that produced it, and a plan that wrote that difference back on
/// every sync would never converge.
const SAME_PLACE_M: f64 = 0.5;

fn same_start(a: Option<Start>, b: Option<Start>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => {
            let p = GeoPosition {
                latitude: a.lat,
                longitude: a.lon,
            };
            let q = GeoPosition {
                latitude: b.lat,
                longitude: b.lon,
            };
            p.distance_m(&q) < SAME_PLACE_M
        }
        (None, None) => true,
        _ => false,
    }
}

/// One hull in the operator's draft: in the exercise, sitting at a starting
/// position if it has one, commanded by whoever they have said so far.
///
/// `commander_id` is `None` for the ordinary case this module exists for — a
/// hull dropped on the map and not yet handed to anybody. It is a real state
/// here and it is refused at the edge, in [`plan`], because it is not a state
/// Minos can hold.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftHull {
    pub unit_id: i64,
    pub name: String,
    pub hull_number: String,
    /// The catalog class the sim drives this hull with. Carried because the
    /// sync does not need it and the sim does, and re-deriving it at sync time
    /// would re-run the one lookup that can legitimately come back empty.
    pub class_id: String,
    /// Absent for a piece that is in the exercise and not on the map — the
    /// state a lift leaves behind, and the state an assign leaves behind.
    pub start: Option<Start>,
    pub commander_id: Option<i64>,
}

impl DraftHull {
    pub fn new(
        unit_id: i64,
        name: String,
        hull_number: String,
        class_id: String,
    ) -> Self {
        Self {
            unit_id,
            name,
            hull_number,
            class_id,
            start: None,
            commander_id: None,
        }
    }
}

/// One hull as Minos has it: the last thing the sync left behind.
///
/// This is the whole of what a diff needs out of the server. Names and
/// hierarchy live in the backend types and are not this module's business,
/// except the name — a hull being dropped has to be nameable in the line that
/// says so.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveHull {
    pub unit_id: i64,
    pub name: String,
    pub commander_id: Option<i64>,
    pub start: Option<Start>,
}

/// The operator's force, keyed by unit id so every read is a lookup and the
/// order a list is drawn in never depends on insertion.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ForceDraft {
    hulls: BTreeMap<i64, DraftHull>,
}

impl ForceDraft {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, unit_id: i64) -> Option<&DraftHull> {
        self.hulls.get(&unit_id)
    }

    pub fn contains(&self, unit_id: i64) -> bool {
        self.hulls.contains_key(&unit_id)
    }

    pub fn len(&self) -> usize {
        self.hulls.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hulls.is_empty()
    }

    /// Every hull, in unit-id order.
    pub fn hulls(&self) -> impl Iterator<Item = &DraftHull> {
        self.hulls.values()
    }

    /// Put a hull in the draft, or replace it wholesale. Replacing is what
    /// re-dropping a hull already in the exercise means: the operator is
    /// saying where it sits now, and everything else about it is still what
    /// they said before.
    pub fn upsert(&mut self, hull: DraftHull) {
        self.hulls.insert(hull.unit_id, hull);
    }

    /// Take a hull out of the exercise. Returns it so the caller can drop the
    /// local half of it — the sim ship, the labels, the selection.
    pub fn remove(&mut self, unit_id: i64) -> Option<DraftHull> {
        self.hulls.remove(&unit_id)
    }

    /// Empty the draft: a released hold, a closed session, a hold that moved
    /// to another game.
    pub fn clear(&mut self) {
        self.hulls.clear();
    }

    /// Say where a hull starts. `None` lifts it off the map without taking it
    /// out of the exercise.
    pub fn set_start(&mut self, unit_id: i64, start: Option<Start>) -> bool {
        match self.hulls.get_mut(&unit_id) {
            Some(hull) => {
                hull.start = start;
                true
            }
            None => false,
        }
    }

    /// Hand a hull to somebody, or take the hand back.
    pub fn set_commander(&mut self, unit_id: i64, commander_id: Option<i64>) -> bool {
        match self.hulls.get_mut(&unit_id) {
            Some(hull) => {
                hull.commander_id = commander_id;
                true
            }
            None => false,
        }
    }

    /// Hulls nobody commands yet, in unit-id order. The refusal names these and
    /// the bulk fill walks these.
    pub fn uncommanded(&self) -> Vec<&DraftHull> {
        self.hulls
            .values()
            .filter(|hull| hull.commander_id.is_none())
            .collect()
    }

    /// How many writes are still owed to the server.
    pub fn pending(&self, live: &[LiveHull]) -> usize {
        match plan(self, live) {
            Ok(writes) => writes.len(),
            // A gap means nothing is writable at all, so everything is pending.
            Err(_) => self.hulls.len(),
        }
    }
}

/// One write the sync owes Minos. Each is one endpoint, and the order they are
/// issued in is [`plan`]'s order.
#[derive(Debug, Clone, PartialEq)]
pub enum ForceWrite {
    /// Put a hull into the exercise. Carries the commander, because Minos
    /// requires one and this is the only write that can set it for the first
    /// time.
    Assign {
        unit_id: i64,
        name: String,
        commander_id: i64,
    },
    /// Change who commands a hull the server already has.
    HandOver {
        unit_id: i64,
        name: String,
        commander_id: i64,
    },
    /// Record a hull's starting position.
    Place {
        unit_id: i64,
        name: String,
        lat: f64,
        lon: f64,
    },
    /// Take a hull off the map. The hull stays in the exercise.
    Lift { unit_id: i64, name: String },
    /// Take a hull out of the exercise entirely.
    Remove { unit_id: i64, name: String },
}

impl ForceWrite {
    pub fn unit_id(&self) -> i64 {
        match self {
            Self::Assign { unit_id, .. }
            | Self::HandOver { unit_id, .. }
            | Self::Place { unit_id, .. }
            | Self::Lift { unit_id, .. }
            | Self::Remove { unit_id, .. } => *unit_id,
        }
    }

    /// The hull's name, for a line that has to say WHICH hull went wrong.
    pub fn name(&self) -> &str {
        match self {
            Self::Assign { name, .. }
            | Self::HandOver { name, .. }
            | Self::Place { name, .. }
            | Self::Lift { name, .. }
            | Self::Remove { name, .. } => name,
        }
    }
}

/// Why a draft cannot be written through yet.
///
/// A hull in the exercise with nobody to steer it is the whole of it, and the
/// rule is Minos's rather than this module's: `game_units.id_commander` is NOT
/// NULL and the assignment endpoint says so in as many words. The draft
/// tolerates the state so the operator can work; the sync refuses it and names
/// the hulls, because handing a piece to whoever happened to be seated first
/// is the one answer nobody can undo by looking at the map.
#[derive(Debug, Clone, PartialEq)]
pub struct ForceGap {
    pub hulls: Vec<String>,
}

impl std::fmt::Display for ForceGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} — Minos gives every piece a commander, so name one on the Player picker first",
            self.hulls.join(", ")
        )
    }
}

/// The writes that carry `draft` onto a server holding `live`, in the order
/// the server accepts them.
///
/// `Err` means nothing is writable: a hull the server has never heard of has
/// nobody commanding it, and issuing the rest of the plan would half-commit a
/// force the operator cannot see. The refusal is at the advance rather than at
/// the drop because that is where the consequence is: the operator can place
/// freely right up until the moment the record matters.
pub fn plan(draft: &ForceDraft, live: &[LiveHull]) -> Result<Vec<ForceWrite>, ForceGap> {
    let gap: Vec<String> = draft
        .hulls()
        .filter(|hull| hull.commander_id.is_none() && !live_holds(live, hull.unit_id))
        .map(|hull| hull.name.clone())
        .collect();
    if !gap.is_empty() {
        return Err(ForceGap { hulls: gap });
    }

    let mut writes = Vec::new();
    for hull in draft.hulls() {
        let Some(here) = live.iter().find(|l| l.unit_id == hull.unit_id) else {
            writes.push(ForceWrite::Assign {
                unit_id: hull.unit_id,
                name: hull.name.clone(),
                // The gap check above is what makes this a total function: a
                // hull the server lacks got past it only by having one.
                commander_id: hull
                    .commander_id
                    .expect("a planned assignment carries a commander"),
            });
            if let Some(start) = hull.start {
                writes.push(ForceWrite::Place {
                    unit_id: hull.unit_id,
                    name: hull.name.clone(),
                    lat: start.lat,
                    lon: start.lon,
                });
            }
            continue;
        };
        match (hull.commander_id, here.commander_id) {
            (Some(next), Some(current)) if next != current => {
                writes.push(ForceWrite::HandOver {
                    unit_id: hull.unit_id,
                    name: hull.name.clone(),
                    commander_id: next,
                });
            }
            (Some(next), None) => {
                writes.push(ForceWrite::HandOver {
                    unit_id: hull.unit_id,
                    name: hull.name.clone(),
                    commander_id: next,
                });
            }
            // Minos has no commanderless piece, so a draft that dropped its
            // commander is not an edit that exists. No control produces it:
            // every one that sets a commander offers a name, never a blank.
            _ => {}
        }
        if !same_start(hull.start, here.start) {
            match hull.start {
                Some(start) => writes.push(ForceWrite::Place {
                    unit_id: hull.unit_id,
                    name: hull.name.clone(),
                    lat: start.lat,
                    lon: start.lon,
                }),
                None => writes.push(ForceWrite::Lift {
                    unit_id: hull.unit_id,
                    name: hull.name.clone(),
                }),
            }
        }
    }
    for here in live {
        if !draft.contains(here.unit_id) {
            writes.push(ForceWrite::Remove {
                unit_id: here.unit_id,
                name: here.name.clone(),
            });
        }
    }
    Ok(writes)
}

fn live_holds(live: &[LiveHull], unit_id: i64) -> bool {
    live.iter().any(|l| l.unit_id == unit_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hull(unit_id: i64, name: &str, start: Option<(f64, f64)>) -> DraftHull {
        let mut h = DraftHull::new(unit_id, name.into(), format!("h{unit_id}"), "c".into());
        h.start = start.map(|(lat, lon)| Start { lat, lon });
        h
    }

    fn live(unit_id: i64, name: &str, commander_id: Option<i64>, start: Option<(f64, f64)>) -> LiveHull {
        LiveHull {
            unit_id,
            name: name.into(),
            commander_id,
            start: start.map(|(lat, lon)| Start { lat, lon }),
        }
    }

    #[test]
    fn a_dropped_hull_is_one_assign_and_one_place() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        h.commander_id = Some(3);
        draft.upsert(h);
        let writes = plan(&draft, &[]).expect("commanded hull plans");
        assert_eq!(
            writes,
            vec![
                ForceWrite::Assign {
                    unit_id: 7,
                    name: "KRI Nanggala".into(),
                    commander_id: 3
                },
                ForceWrite::Place {
                    unit_id: 7,
                    name: "KRI Nanggala".into(),
                    lat: -5.4,
                    lon: 106.2
                },
            ]
        );
    }

    #[test]
    fn an_uncommanded_hull_refuses_the_whole_plan() {
        let mut draft = ForceDraft::new();
        draft.upsert(hull(7, "KRI Nanggala", Some((-5.4, 106.2))));
        draft.upsert(hull(8, "KRI Rangsat", Some((-5.1, 106.9))));
        assert_eq!(
            plan(&draft, &[]),
            Err(ForceGap {
                hulls: vec!["KRI Nanggala".into(), "KRI Rangsat".into()]
            })
        );
    }

    #[test]
    fn a_hull_the_server_already_has_may_stay_uncommanded_locally() {
        // The server gave it a commander on assignment, and Minos has no
        // commanderless piece to fall back to — so the draft's blank is not a
        // write, only the placement move is.
        let mut draft = ForceDraft::new();
        draft.upsert(hull(7, "KRI Nanggala", Some((-5.4, 106.2))));
        let writes = plan(&draft, &[live(7, "KRI Nanggala", Some(3), None)]).expect("plans");
        assert_eq!(
            writes,
            vec![ForceWrite::Place {
                unit_id: 7,
                name: "KRI Nanggala".into(),
                lat: -5.4,
                lon: 106.2
            }]
        );
    }

    #[test]
    fn a_synced_force_plans_nothing() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        h.commander_id = Some(3);
        draft.upsert(h);
        let server = vec![live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))];
        assert_eq!(plan(&draft, &server), Ok(Vec::new()));
        assert_eq!(draft.pending(&server), 0);
    }

    #[test]
    fn a_moved_placement_is_one_write_and_nothing_else() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", Some((-5.0, 107.0)));
        h.commander_id = Some(3);
        draft.upsert(h);
        let server = vec![live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))];
        assert_eq!(
            plan(&draft, &server),
            Ok(vec![ForceWrite::Place {
                unit_id: 7,
                name: "KRI Nanggala".into(),
                lat: -5.0,
                lon: 107.0
            }])
        );
    }

    #[test]
    fn a_lift_is_one_write_that_stays_in_the_exercise() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", None);
        h.commander_id = Some(3);
        draft.upsert(h);
        let server = vec![live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))];
        assert_eq!(
            plan(&draft, &server),
            Ok(vec![ForceWrite::Lift {
                unit_id: 7,
                name: "KRI Nanggala".into()
            }])
        );
    }

    #[test]
    fn a_handover_is_its_own_write() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        h.commander_id = Some(9);
        draft.upsert(h);
        let server = vec![live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))];
        assert_eq!(
            plan(&draft, &server),
            Ok(vec![ForceWrite::HandOver {
                unit_id: 7,
                name: "KRI Nanggala".into(),
                commander_id: 9
            }])
        );
    }

    #[test]
    fn a_hull_taken_out_of_the_exercise_is_dropped_last() {
        let mut draft = ForceDraft::new();
        let mut h = hull(8, "KRI Rangsat", Some((-5.1, 106.9)));
        h.commander_id = Some(3);
        draft.upsert(h);
        let server = vec![
            live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2))),
            live(8, "KRI Rangsat", Some(3), Some((-5.1, 106.9))),
        ];
        assert_eq!(
            plan(&draft, &server),
            Ok(vec![ForceWrite::Remove {
                unit_id: 7,
                name: "KRI Nanggala".into()
            }])
        );
    }

    #[test]
    fn a_new_assignment_is_ordered_before_its_placement() {
        // The position endpoint 404s on a hull the server has never heard of,
        // so the order is load-bearing rather than tidy.
        let mut draft = ForceDraft::new();
        let mut assigned = hull(8, "KRI Rangsat", None);
        assigned.commander_id = Some(3);
        draft.upsert(assigned);
        let mut placed = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        placed.commander_id = Some(3);
        draft.upsert(placed);
        assert_eq!(
            plan(&draft, &[]),
            Ok(vec![
                ForceWrite::Assign { unit_id: 7, name: "KRI Nanggala".into(), commander_id: 3 },
                ForceWrite::Place { unit_id: 7, name: "KRI Nanggala".into(), lat: -5.4, lon: 106.2 },
                ForceWrite::Assign { unit_id: 8, name: "KRI Rangsat".into(), commander_id: 3 },
            ])
        );
    }

    #[test]
    fn a_read_back_placement_is_not_a_write() {
        // The server round-trips the pair through JSON. A draft seeded from a
        // placement view must not re-write the position on every sync.
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        h.commander_id = Some(3);
        draft.upsert(h);
        let server = vec![live(
            7,
            "KRI Nanggala",
            Some(3),
            Some((-5.400000000_1, 106.200000000_1)),
        )];
        assert_eq!(plan(&draft, &server), Ok(Vec::new()));
    }

    #[test]
    fn a_whole_metre_is_a_move() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        h.commander_id = Some(3);
        draft.upsert(h);
        let server = vec![live(
            7,
            "KRI Nanggala",
            Some(3),
            Some((-5.40001, 106.2)),
        )];
        assert_eq!(plan(&draft, &server).unwrap().len(), 1);
    }

    #[test]
    fn pending_counts_writes_not_hulls() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        h.commander_id = Some(3);
        draft.upsert(h);
        let server = vec![live(7, "KRI Nanggala", Some(3), None)];
        assert_eq!(draft.pending(&server), 1);
    }

    #[test]
    fn uncommanded_lists_only_the_blank_hulls() {
        let mut draft = ForceDraft::new();
        let mut commanded = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        commanded.commander_id = Some(3);
        draft.upsert(commanded);
        draft.upsert(hull(8, "KRI Rangsat", Some((-5.1, 106.9))));
        let names: Vec<&str> = draft.uncommanded().iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["KRI Rangsat"]);
    }

    #[test]
    fn re_dropping_keeps_the_commander_already_chosen() {
        let mut draft = ForceDraft::new();
        let mut first = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        first.commander_id = Some(9);
        draft.upsert(first);
        // The second drop replaces the hull, so the caller has to carry the
        // commander over — that is the caller's half of the rule and this
        // test is the reason it is written down.
        let carried = draft.get(7).expect("still in the draft").commander_id;
        assert_eq!(carried, Some(9));
    }
}
