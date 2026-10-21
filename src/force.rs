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
//! has never heard of. Putting one hull aboard another comes after that, for
//! the same reason and one more: the embarkation endpoint 404s on a hull — or
//! a host — the server has never heard of. A hull the operator has taken back
//! out of the exercise is dropped last, so nothing is ever briefly absent
//! while it moves.

use std::collections::{BTreeMap, BTreeSet};

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
    /// The hull this piece is aboard, or `None` for a piece on the water.
    ///
    /// This is the operator's whole statement about embarkation: a drop on a
    /// host sets it, a drop on bare water clears it. The chain it starts is
    /// checked for rings in [`plan`], because Minos refuses one and the
    /// operator can build one by dragging.
    pub host: Option<i64>,
    /// The course this hull starts on, in degrees, as the operator has said
    /// it — `None` for a hull whose course nobody has stated.
    ///
    /// NOT A PLACEMENT. Minos's placement write takes a latitude and a
    /// longitude and nothing else, so a course is not a thing the advance can
    /// write through: the hull's first course is whatever the server's own
    /// materialisation of the placement makes it. This is therefore a STAGED
    /// course — the console's own statement of the course the hull should
    /// start on, held in the draft and posted as a helm order when the
    /// exercise starts playing and the server will take one.
    ///
    /// It is deliberately NOT part of the diff [`plan`] takes: a course
    /// changed and a placement changed are different edits, and the plan's
    /// writes are the ones the server accepts.
    pub course_deg: Option<f32>,
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
            host: None,
            course_deg: None,
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
    /// What the server last said carries this piece, or `None` for one on
    /// the water. The whole of what a diff needs to plan an embarkation.
    pub host: Option<i64>,
}

/// The operator's force, keyed by unit id so every read is a lookup and the
/// order a list is drawn in never depends on insertion.
///
/// THE FRAME IS WHAT THE SERVER LAST SAID, and it is what tells the
/// operator's edit from the server's row. A re-read happens while the
/// operator is still authoring — every write triggers one — so a re-seed
/// that simply overwrote the draft would throw away work the flush has not
/// written. An entry that still matches the frame is the server's, and one
/// that differs is the operator's; the frame is replaced, the edits are
/// kept.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ForceDraft {
    hulls: BTreeMap<i64, DraftHull>,
    /// What the server last said, per hull. The reference the merge reads.
    frame: BTreeMap<i64, DraftHull>,
    /// Hulls the operator has taken out of the exercise. A mark rather than
    /// an absence, because a re-read would otherwise put the hull back.
    removed: BTreeSet<i64>,
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
        self.removed.remove(&hull.unit_id);
        self.hulls.insert(hull.unit_id, hull);
    }

    /// Take a hull out of the exercise. Returns it so the caller can drop the
    /// local half of it — the sim ship, the labels, the selection.
    ///
    /// The removal is MARKED, not merely an absence: a re-read replaces the
    /// draft from the server's frame, and a hull the server still holds
    /// would come back with it.
    pub fn remove(&mut self, unit_id: i64) -> Option<DraftHull> {
        self.removed.insert(unit_id);
        self.hulls.remove(&unit_id)
    }

    /// Set the frame from a read and KEEP the operator's edits.
    ///
    /// The merge is the rule, not a convenience: the session read happens
    /// after every write, so a re-seed that overwrote the draft would
    /// discard an operator's half-finished plan on the strength of a write
    /// they had already made. An entry that no longer matches the frame is
    /// the operator's own statement and survives; one that matches it is
    /// the server's, and the server's newer word wins.
    ///
    /// A hull the operator took out of the exercise stays out while the
    /// server still holds it — that removal is a statement too.
    pub fn seed(&mut self, live: &[LiveHull]) {
        let mut frame: BTreeMap<i64, DraftHull> = BTreeMap::new();
        for l in live {
            frame.insert(
                l.unit_id,
                DraftHull {
                    unit_id: l.unit_id,
                    name: l.name.clone(),
                    hull_number: String::new(),
                    class_id: String::new(),
                    start: l.start,
                    commander_id: l.commander_id,
                    host: l.host,
                    // The server holds no course for a hull, so the frame's
                    // is none — and an operator's staged course differs from
                    // it, which is what keeps the staged course through a
                    // re-seed.
                    course_deg: None,
                },
            );
        }
        let kept: Vec<DraftHull> = self
            .hulls()
            .filter(|hull| self.frame.get(&hull.unit_id) != Some(hull))
            .cloned()
            .collect();
        let removals: BTreeSet<i64> = self
            .removed
            .iter()
            .copied()
            .filter(|id| frame.contains_key(id))
            .collect();
        self.hulls = frame.clone();
        for hull in kept {
            self.hulls.insert(hull.unit_id, hull);
        }
        for id in &removals {
            self.hulls.remove(id);
        }
        self.frame = frame;
        self.removed = removals;
    }

    /// Empty the draft: a released hold, a closed session, a hold that moved
    /// to another game.
    pub fn clear(&mut self) {
        self.hulls.clear();
        self.frame.clear();
        self.removed.clear();
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

    /// Say what course a hull starts on. `None` says nothing about it.
    ///
    /// A STAGED course, and not a write: the placement endpoint takes a
    /// position and nothing else, so the course is held here until the
    /// exercise is playing and a helm order can carry it.
    pub fn set_course(&mut self, unit_id: i64, course_deg: Option<f32>) -> bool {
        match self.hulls.get_mut(&unit_id) {
            Some(hull) => {
                hull.course_deg = course_deg.map(|c| c.rem_euclid(360.0));
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

    /// Put a hull aboard another, or take it off whatever carries it. `None`
    /// puts it back on the water; the host is named, not the piece, because
    /// that is the thing the operator dragged it onto.
    pub fn set_host(&mut self, unit_id: i64, host: Option<i64>) -> bool {
        match self.hulls.get_mut(&unit_id) {
            Some(hull) => {
                hull.host = host;
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
    /// Put a hull aboard another. Names both ends: the endpoint addresses
    /// the host, and the child is the hull that moves.
    Embark { host: i64, child: i64, name: String },
    /// Take a hull off whatever carries it. The host is named because the
    /// detach endpoint addresses the HOST and lists the children leaving it
    /// — and the host it leaves is whatever the server last said, not
    /// whatever the draft now wants.
    Disembark { host: i64, child: i64, name: String },
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
            // The child is the hull this write is about: the host is what
            // it moves to, and a line that has to say which hull went wrong
            // means the one that moved.
            Self::Embark { child, .. } | Self::Disembark { child, .. } => *child,
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
            Self::Embark { name, .. } | Self::Disembark { name, .. } => name,
        }
    }
}

/// Why a draft cannot be written through yet.
///
/// A stage advance is the moment the draft is written through, and a draft
/// that cannot be written refuses WHOLE and names the hulls: issuing the rest
/// would half-commit a force the operator cannot see.
///
/// Four states refuse, and all four are Minos's rules rather than this
/// module's. A hull in the exercise with nobody to steer it is the first:
/// `game_units.id_commander` is NOT NULL and the assignment endpoint says so
/// in as many words. The draft tolerates the state so the operator can work;
/// the sync refuses it, because handing a piece to whoever happened to be
/// seated first is the one answer nobody can undo by looking at the map. One
/// person commanding two hulls is the second: the server says "That person
/// already commands the unit ... One person commands one thing", and a
/// second hull for the same commander is refused as a conflict rather than
/// as a duplicate. A hull told to ride itself, and a ring of hulls that
/// carry each other, are the next two: the attach endpoint refuses both
/// outright ("putting the group inside the carrier is a ring"), and they are
/// the states a drag can build, so catching them here is the point of this
/// module rather than a courtesy.
#[derive(Debug, Clone, PartialEq)]
pub struct ForceGap {
    pub reason: GapReason,
    /// The hulls the sentence names, in unit-id order.
    pub hulls: Vec<String>,
    /// The person the sentence is about, when the rule is about a person.
    /// `None` for the rules that are about hulls alone.
    pub who: Option<String>,
}

/// Which rule refused the draft. The reason is what keeps [`ForceGap`]'s
/// sentence truthful: a ring is not a missing commander, and telling the
/// operator it is would send them to the Player picker to fix a drag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GapReason {
    /// A hull the server does not hold has nobody to command it.
    Uncommanded,
    /// One person is given two hulls.
    TwoCommands,
    /// A hull is given to a seat that may not command.
    JudgeSideCommander,
    /// A hull is its own host.
    SelfHost,
    /// Hulls that carry each other, transitively.
    Ring,
}

impl std::fmt::Display for ForceGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hulls = self.hulls.join(", ");
        match self.reason {
            GapReason::Uncommanded => write!(
                f,
                "{hulls} — Minos gives every piece a commander, so name one on the Player picker first"
            ),
            GapReason::TwoCommands => write!(
                f,
                "{hulls} — one person commands one thing, so the second hull needs a second commander"
            ),
            GapReason::JudgeSideCommander => {
                let who = self.who.as_deref().unwrap_or("that seat");
                write!(
                    f,
                    "{hulls} — {who} is on the judge side, and a judge directs rather \
                     than commands, so give the hull a player-side commander"
                )
            }
            GapReason::SelfHost => write!(
                f,
                "{hulls} — a hull cannot ride itself, so drop it on open water or on another hull"
            ),
            GapReason::Ring => write!(
                f,
                "{hulls} — these hulls carry each other in a ring, so move one back to open water"
            ),
        }
    }
}

/// The first rule the draft breaks, or `None` when it is writable.
///
/// The checks run in the order the operator is likely to meet them, and the
/// first one to fire refuses the whole draft: a plan that writes the commander
/// of a hull it cannot embark would half-commit.
fn gap(draft: &ForceDraft, live: &[LiveHull]) -> Option<ForceGap> {
    let uncommanded: Vec<String> = draft
        .hulls()
        .filter(|hull| hull.commander_id.is_none() && !live_holds(live, hull.unit_id))
        .map(|hull| hull.name.clone())
        .collect();
    if !uncommanded.is_empty() {
        return Some(ForceGap {
            reason: GapReason::Uncommanded,
            hulls: uncommanded,
            who: None,
        });
    }

    // ONE PERSON, ONE THING. The server refuses a second hull for a
    // commander who already has one ("That person already commands the unit
    // ... One person commands one thing — a unit or a task group"), so a
    // draft that hands the same person two hulls is a draft the server will
    // refuse halfway through the write. The count is over the DRAFT, which
    // is the whole force as it will be: a hull the server already holds and
    // the draft leaves alone still counts, because the draft is what the
    // command will be.
    let doubled = doubled_hulls(draft);
    if !doubled.is_empty() {
        return Some(ForceGap {
            reason: GapReason::TwoCommands,
            hulls: doubled,
            who: None,
        });
    }

    let self_hosted: Vec<String> = draft
        .hulls()
        .filter(|hull| hull.host == Some(hull.unit_id))
        .map(|hull| hull.name.clone())
        .collect();
    if !self_hosted.is_empty() {
        return Some(ForceGap {
            reason: GapReason::SelfHost,
            hulls: self_hosted,
            who: None,
        });
    }

    let ring = ring_hulls(draft);
    if !ring.is_empty() {
        return Some(ForceGap {
            reason: GapReason::Ring,
            hulls: ring,
            who: None,
        });
    }

    None
}

/// The hulls a draft gives to a commander who already has one.
///
/// Every hull a commander holds more than one of, in unit-id order. The
/// names are the hulls, not the commander: the hull is what the operator
/// has to change, and the commander is a name the console may not know.
fn doubled_hulls(draft: &ForceDraft) -> Vec<String> {
    let mut by_commander: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for hull in draft.hulls() {
        if let Some(commander) = hull.commander_id {
            by_commander
                .entry(commander)
                .or_default()
                .push(hull.name.clone());
        }
    }
    by_commander
        .values()
        .filter(|hulls| hulls.len() > 1)
        .flatten()
        .cloned()
        .collect()
}

/// The hull a draft gives to a seat that may not command, or `None`.
///
/// THE JUDGE SIDE DOES NOT COMMAND. The server refuses a judge-side
/// commander outright ("a judge-side participant does not command units or
/// task groups"), so the console refuses it before the write rather than
/// after it. `judges` is the room's judge-side seats by id and by name —
/// the staged role wins over the live one, because the staged role is what
/// the advance is about to write.
///
/// This is a rule about the ROOM, so it is not in [`gap`]: the draft alone
/// does not say who is judge-side, and the shell's own door to the plan
/// does not carry the room either. The advance carries both and asks.
pub fn judge_side_gap(
    draft: &ForceDraft,
    judges: &BTreeMap<i64, String>,
) -> Option<ForceGap> {
    for hull in draft.hulls() {
        let Some(commander) = hull.commander_id else {
            continue;
        };
        let Some(who) = judges.get(&commander) else {
            continue;
        };
        return Some(ForceGap {
            reason: GapReason::JudgeSideCommander,
            hulls: vec![hull.name.clone()],
            who: Some(who.clone()),
        });
    }
    None
}

/// Every hull that lies on a ring of `host` links, in unit-id order.
///
/// A walk follows one hull's host chain until it either leaves the draft —
/// the host is not a drafted hull, so the chain ends there — or revisits a
/// hull, at which point everything from the first sighting of that hull
/// onward is the ring. The walk is finite because there are finitely many
/// hulls and it stops on the first repeat.
fn ring_hulls(draft: &ForceDraft) -> Vec<String> {
    let mut on_ring: BTreeSet<i64> = BTreeSet::new();
    for start in draft.hulls().map(|hull| hull.unit_id) {
        let mut path: Vec<i64> = Vec::new();
        let mut cursor = Some(start);
        while let Some(id) = cursor {
            if let Some(at) = path.iter().position(|seen| *seen == id) {
                on_ring.extend(path[at..].iter().copied());
                break;
            }
            path.push(id);
            cursor = draft.get(id).and_then(|hull| hull.host);
        }
    }
    on_ring
        .iter()
        .filter_map(|id| draft.get(*id).map(|hull| hull.name.clone()))
        .collect()
}

/// The writes that carry `draft` onto a server holding `live`, in the order
/// the server accepts them.
///
/// The writes go in three passes, and the order BETWEEN them is load-bearing
/// rather than tidy: presence and command first, with a new hull's placement
/// riding directly behind its assignment; embarkation second, because the
/// embarkation endpoint 404s on a hull — or a host — the server has never
/// heard of, exactly as the position endpoint does; removals last, once
/// nothing else in the plan refers to the hull.
///
/// `Err` means nothing is writable: a hull the server has never heard of has
/// nobody commanding it, or the embarkation chain is one Minos refuses, and
/// issuing the rest of the plan would half-commit a force the operator cannot
/// see. The refusal is at the advance rather than at the drop because that is
/// where the consequence is: the operator can place freely right up until the
/// moment the record matters.
pub fn plan(draft: &ForceDraft, live: &[LiveHull]) -> Result<Vec<ForceWrite>, ForceGap> {
    if let Some(gap) = gap(draft, live) {
        return Err(gap);
    }

    let mut writes = Vec::new();
    // PASS 1 — presence, command and position, one hull at a time in unit-id
    // order. A hull the server does not hold is assigned first, and its
    // placement rides directly behind the assignment, because the position
    // endpoint 404s on a hull the server has never heard of.
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
    // PASS 2 — embarkation, per hull in child unit-id order. AFTER pass 1,
    // because the embarkation endpoint 404s on a hull — or a host — the
    // server has never heard of, the same reason `Assign` precedes `Place`:
    // every hull this pass names has to have been assigned by the pass above.
    // A hull the server does not hold reads as hostless, which is exactly
    // right — it arrives on the water and is put aboard in the same advance.
    for hull in draft.hulls() {
        let live_host = live
            .iter()
            .find(|l| l.unit_id == hull.unit_id)
            .and_then(|l| l.host);
        if hull.host == live_host {
            continue;
        }
        // A hull already inside something must come OUT before it can go into
        // something else: the attach endpoint refuses a unit that is "already
        // inside something", so a re-host is two writes rather than one. The
        // detach addresses the host the server last named — the only place the
        // hull can currently be.
        if let Some(old) = live_host {
            writes.push(ForceWrite::Disembark {
                host: old,
                child: hull.unit_id,
                name: hull.name.clone(),
            });
        }
        if let Some(host) = hull.host {
            writes.push(ForceWrite::Embark {
                host,
                child: hull.unit_id,
                name: hull.name.clone(),
            });
        }
    }
    // PASS 3 — removals last: a hull taken back out of the exercise is
    // dropped once nothing else in the plan refers to it.
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
            host: None,
        }
    }

    /// The same live hull, but the server says it is aboard `host`.
    fn aboard(mut hull: LiveHull, host: i64) -> LiveHull {
        hull.host = Some(host);
        hull
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

    /// A STAGED COURSE SURVIVES A RE-SEED. The session read happens after
    /// every write, and a re-seed that overwrote the draft would throw the
    /// operator's course away on the strength of a write they had already
    /// made. The course is an entry that no longer matches the frame, and
    /// an entry that no longer matches the frame is the operator's.
    #[test]
    fn a_staged_course_survives_a_reseed() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        h.commander_id = Some(3);
        draft.upsert(h);
        draft.set_course(7, Some(90.0));
        // The server re-read: it holds the hull, at the same place, and it
        // holds no course for it — it has no course to hold.
        draft.seed(&[live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))]);
        assert_eq!(
            draft.get(7).and_then(|h| h.course_deg),
            Some(90.0),
            "the staged course is the operator's own statement and it survives"
        );
    }

    /// A COURSE IS NOT A WRITE. Minos's placement takes a position and
    /// nothing else, so a course changed and a placement changed are
    /// different edits — and the plan's writes are the ones the server
    /// accepts. A plan that carried the course would either post a field
    /// the endpoint refuses or, worse, re-post the placement on every
    /// re-seed because the two would never match.
    #[test]
    fn a_course_is_not_a_write() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        h.commander_id = Some(3);
        draft.upsert(h);
        draft.set_course(7, Some(90.0));
        let writes = plan(
            &draft,
            &[live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))],
        )
        .expect("the plan holds");
        assert!(
            writes.is_empty(),
            "the server holds the hull where it is and no course of its own: {writes:?}"
        );
    }

    /// A COURSE IS STATED IN DEGREES, AND A DEGREE IS A DEGREE. The
    /// rotator's needle can point anywhere, and the draft says the same
    /// thing whichever way the operator turned it.
    #[test]
    fn a_course_is_stored_as_a_bearing() {
        let mut draft = ForceDraft::new();
        draft.upsert(hull(7, "KRI Nanggala", Some((-5.4, 106.2))));
        assert!(draft.set_course(7, Some(450.0)));
        assert_eq!(
            draft.get(7).and_then(|h| h.course_deg),
            Some(90.0),
            "450 degrees is 90 degrees, and the draft says so"
        );
        assert!(draft.set_course(7, None));
        assert_eq!(
            draft.get(7).and_then(|h| h.course_deg),
            None,
            "no course is no course"
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
                reason: GapReason::Uncommanded,
                hulls: vec!["KRI Nanggala".into(), "KRI Rangsat".into()],
                who: None,
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
        //
        // TWO COMMANDERS, because one person commands one thing: the two
        // hulls here are two people, and a force that gave them both to the
        // same person would be refused before it was written.
        let mut draft = ForceDraft::new();
        let mut assigned = hull(8, "KRI Rangsat", None);
        assigned.commander_id = Some(4);
        draft.upsert(assigned);
        let mut placed = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        placed.commander_id = Some(3);
        draft.upsert(placed);
        assert_eq!(
            plan(&draft, &[]),
            Ok(vec![
                ForceWrite::Assign { unit_id: 7, name: "KRI Nanggala".into(), commander_id: 3 },
                ForceWrite::Place { unit_id: 7, name: "KRI Nanggala".into(), lat: -5.4, lon: 106.2 },
                ForceWrite::Assign { unit_id: 8, name: "KRI Rangsat".into(), commander_id: 4 },
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

    // -- embarkation ------------------------------------------------------
    //
    // One hull aboard another is one `host` link in the draft and one
    // endpoint at the advance. The link is the whole diff, so it is the
    // whole rule: a change is one write, a synced link is none, and a ring
    // is the shape Minos refuses.

    /// A hull already in the exercise on both sides has nothing to assign,
    /// so putting it aboard names no assignment: the link is the whole edit.
    ///
    /// A CARRIER AND ITS CHILD HAVE TWO COMMANDERS: one person commands one
    /// thing, so the two hulls here are two people. A force that gave them
    /// both to one commander would be refused before it was written.
    #[test]
    fn a_fresh_embark_is_one_write() {
        let mut draft = ForceDraft::new();
        let mut host = hull(5, "KRI Ahmad Yani", None);
        host.commander_id = Some(3);
        draft.upsert(host);
        let mut child = hull(7, "KRI Nanggala", None);
        child.commander_id = Some(4);
        draft.upsert(child);
        assert!(draft.set_host(7, Some(5)), "the child is in the draft");
        let server = vec![
            live(5, "KRI Ahmad Yani", Some(3), None),
            live(7, "KRI Nanggala", Some(4), None),
        ];
        assert_eq!(
            plan(&draft, &server),
            Ok(vec![ForceWrite::Embark {
                host: 5,
                child: 7,
                name: "KRI Nanggala".into(),
            }])
        );
    }

    /// The detach addresses the host the server last named, and lists the
    /// child leaving it: the child is what moves, the host is where from.
    #[test]
    fn a_disembark_is_one_write() {
        let mut draft = ForceDraft::new();
        let mut host = hull(5, "KRI Ahmad Yani", None);
        host.commander_id = Some(3);
        draft.upsert(host);
        let mut child = hull(7, "KRI Nanggala", None);
        child.commander_id = Some(4);
        draft.upsert(child);
        assert!(draft.set_host(7, None), "clearing the link disembarks");
        let server = vec![
            live(5, "KRI Ahmad Yani", Some(3), None),
            aboard(live(7, "KRI Nanggala", Some(4), None), 5),
        ];
        assert_eq!(
            plan(&draft, &server),
            Ok(vec![ForceWrite::Disembark {
                host: 5,
                child: 7,
                name: "KRI Nanggala".into(),
            }])
        );
    }

    /// A hull already aboard one host and re-dropped on another comes out
    /// first: the attach endpoint refuses a unit that is already inside
    /// something, so the plan spends the detach before the attach.
    #[test]
    fn a_re_host_detaches_before_it_attaches() {
        let mut draft = ForceDraft::new();
        // THREE COMMANDERS: one person commands one thing, so three hulls
        // are three people.
        for (id, name, commander) in [
            (5, "KRI Ahmad Yani", 3),
            (7, "KRI Nanggala", 4),
            (9, "KRI Sultan", 5),
        ] {
            let mut h = hull(id, name, None);
            h.commander_id = Some(commander);
            draft.upsert(h);
        }
        assert!(draft.set_host(7, Some(9)), "re-dropped on a new host");
        let server = vec![
            aboard(live(7, "KRI Nanggala", Some(4), None), 5),
            live(5, "KRI Ahmad Yani", Some(3), None),
            live(9, "KRI Sultan", Some(5), None),
        ];
        assert_eq!(
            plan(&draft, &server),
            Ok(vec![
                ForceWrite::Disembark {
                    host: 5,
                    child: 7,
                    name: "KRI Nanggala".into(),
                },
                ForceWrite::Embark {
                    host: 9,
                    child: 7,
                    name: "KRI Nanggala".into(),
                },
            ])
        );
    }

    /// The embarkation endpoint 404s on a hull — or a host — the server has
    /// never heard of, so a new host and a new child are both assigned
    /// before the link that names them is posted.
    #[test]
    fn an_embark_is_ordered_after_both_hulls_assignments() {
        let mut draft = ForceDraft::new();
        let mut host = hull(5, "KRI Ahmad Yani", None);
        host.commander_id = Some(3);
        draft.upsert(host);
        let mut child = hull(7, "KRI Nanggala", None);
        child.commander_id = Some(4);
        draft.upsert(child);
        assert!(draft.set_host(7, Some(5)));
        assert_eq!(
            plan(&draft, &[]),
            Ok(vec![
                ForceWrite::Assign { unit_id: 5, name: "KRI Ahmad Yani".into(), commander_id: 3 },
                ForceWrite::Assign { unit_id: 7, name: "KRI Nanggala".into(), commander_id: 4 },
                ForceWrite::Embark { host: 5, child: 7, name: "KRI Nanggala".into() },
            ])
        );
    }

    /// The server already holds the link, so the draft and the record agree
    /// and the sync must write nothing — the same rule as a position that
    /// round-tripped through JSON.
    #[test]
    fn a_synced_embarkation_plans_nothing() {
        let mut draft = ForceDraft::new();
        let mut host = hull(5, "KRI Ahmad Yani", None);
        host.commander_id = Some(3);
        draft.upsert(host);
        let mut child = hull(7, "KRI Nanggala", None);
        child.commander_id = Some(4);
        child.host = Some(5);
        draft.upsert(child);
        let server = vec![
            live(5, "KRI Ahmad Yani", Some(3), None),
            aboard(live(7, "KRI Nanggala", Some(4), None), 5),
        ];
        assert_eq!(plan(&draft, &server), Ok(Vec::new()));
    }

    /// Minos refuses a self-attach as a ring of one. Catching it here is
    /// what stops the operator spending a stage advance to hear it, and the
    /// hull is named so the sentence points at the piece to drop elsewhere.
    #[test]
    fn a_self_embark_is_refused_and_the_hull_named() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", None);
        h.commander_id = Some(3);
        h.host = Some(7);
        draft.upsert(h);
        assert_eq!(
            plan(&draft, &[]),
            Err(ForceGap {
                reason: GapReason::SelfHost,
                hulls: vec!["KRI Nanggala".into()],
                who: None,
            })
        );
    }

    /// Two hulls that carry each other are the ring Minos refuses. The
    /// refusal names both, because it takes breaking one link to fix it and
    /// the operator should not have to guess which hulls are involved.
    #[test]
    fn an_embarkation_ring_is_refused_and_every_hull_named() {
        let mut draft = ForceDraft::new();
        let mut a = hull(7, "KRI Nanggala", None);
        a.commander_id = Some(3);
        a.host = Some(8);
        draft.upsert(a);
        let mut b = hull(8, "KRI Rangsat", None);
        b.commander_id = Some(4);
        b.host = Some(7);
        draft.upsert(b);
        assert_eq!(
            plan(&draft, &[]),
            Err(ForceGap {
                reason: GapReason::Ring,
                hulls: vec!["KRI Nanggala".into(), "KRI Rangsat".into()],
                who: None,
            })
        );
    }

    /// An embark is a write like any other, so the picker's count includes
    /// it — otherwise a draft whose only edit is a drop on a host would look
    /// synced while the advance still owed the link.
    #[test]
    fn pending_counts_an_embark() {
        let mut draft = ForceDraft::new();
        let mut host = hull(5, "KRI Ahmad Yani", None);
        host.commander_id = Some(3);
        draft.upsert(host);
        let mut child = hull(7, "KRI Nanggala", None);
        child.commander_id = Some(4);
        draft.upsert(child);
        assert!(draft.set_host(7, Some(5)));
        let server = vec![
            live(5, "KRI Ahmad Yani", Some(3), None),
            live(7, "KRI Nanggala", Some(4), None),
        ];
        assert_eq!(draft.pending(&server), 1);
    }

    // -- the commander rules -----------------------------------------------
    //
    // Two rules about WHO holds a command, and both are the server's:
    // one person commands one thing, and a judge-side seat commands
    // nothing. The first is about the draft alone and lives in `gap`; the
    // second is about the room and is asked with the room in hand.

    /// One person, two hulls. The server refuses the second assignment
    /// ("That person already commands the unit ... One person commands one
    /// thing"), so the plan refuses the whole thing and names both hulls.
    #[test]
    fn a_commander_given_two_hulls_refuses_the_whole_plan() {
        let mut draft = ForceDraft::new();
        let mut first = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        first.commander_id = Some(3);
        draft.upsert(first);
        let mut second = hull(8, "KRI Rangsat", Some((-5.1, 106.9)));
        second.commander_id = Some(3);
        draft.upsert(second);
        assert_eq!(
            plan(&draft, &[]),
            Err(ForceGap {
                reason: GapReason::TwoCommands,
                hulls: vec!["KRI Nanggala".into(), "KRI Rangsat".into()],
                who: None,
            })
        );
    }

    /// The same rule about a hull the server already holds: the draft keeps
    /// the commander it has and adds a second hull for them, and the server
    /// would refuse the second write.
    #[test]
    fn a_second_hull_for_a_commander_already_holding_one_refuses() {
        let mut draft = ForceDraft::new();
        let mut held = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        held.commander_id = Some(3);
        draft.upsert(held);
        let mut second = hull(8, "KRI Rangsat", Some((-5.1, 106.9)));
        second.commander_id = Some(3);
        draft.upsert(second);
        let server = vec![live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))];
        assert_eq!(
            plan(&draft, &server),
            Err(ForceGap {
                reason: GapReason::TwoCommands,
                hulls: vec!["KRI Nanggala".into(), "KRI Rangsat".into()],
                who: None,
            })
        );
    }

    /// A judge-side seat may not command. The server refuses the assignment
    /// outright, so the console refuses it before the write and names the
    /// person, because "a judge-side seat may not command" is a rule the
    /// operator cannot act on without knowing WHICH seat.
    #[test]
    fn a_judge_side_commander_refuses_the_whole_plan() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        h.commander_id = Some(3);
        draft.upsert(h);
        let judges = BTreeMap::from([(3, "Budi".to_string())]);
        let gap = judge_side_gap(&draft, &judges).expect("a judge commands nothing");
        assert_eq!(gap.reason, GapReason::JudgeSideCommander);
        assert_eq!(gap.hulls, vec!["KRI Nanggala".to_string()]);
        assert_eq!(gap.who.as_deref(), Some("Budi"));
        assert!(
            gap.to_string().contains("Budi"),
            "the sentence names the seat: {gap}"
        );
    }

    /// A player-side commander is not a gap, and the room's judge seats are
    /// about OTHER people.
    #[test]
    fn a_player_side_commander_is_no_gap() {
        let mut draft = ForceDraft::new();
        let mut h = hull(7, "KRI Nanggala", Some((-5.4, 106.2)));
        h.commander_id = Some(3);
        draft.upsert(h);
        let judges = BTreeMap::from([(9, "Referee".to_string())]);
        assert_eq!(judge_side_gap(&draft, &judges), None);
    }

    // -- the merge --------------------------------------------------------
    //
    // A re-read happens after every write, so the draft is re-seeded while
    // the operator is still authoring. The merge is what keeps that from
    // throwing the operator's work away.

    /// The operator's placement survives a re-read: the frame is replaced
    /// and the edit on top of it is not.
    #[test]
    fn a_re_read_keeps_a_staged_placement() {
        let mut draft = ForceDraft::new();
        draft.seed(&[live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))]);
        draft.set_start(7, Some(Start { lat: -5.0, lon: 107.0 }));
        draft.seed(&[live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))]);
        assert_eq!(
            draft.get(7).and_then(|h| h.start),
            Some(Start { lat: -5.0, lon: 107.0 }),
            "the operator's placement is the operator's"
        );
    }

    /// A hull the operator dropped on the water survives a re-read: the
    /// server does not hold it, so the merge has nothing to adopt over it.
    #[test]
    fn a_re_read_keeps_a_staged_newcomer() {
        let mut draft = ForceDraft::new();
        draft.seed(&[]);
        let mut h = hull(9, "KRI Sultan", Some((-5.0, 107.0)));
        h.commander_id = Some(3);
        draft.upsert(h);
        draft.seed(&[]);
        assert!(draft.contains(9), "the staged hull survives the re-read");
    }

    /// A removal survives a re-read: the server still holds the hull, so
    /// the merge has to keep it out rather than put it back.
    #[test]
    fn a_re_read_keeps_a_staged_removal() {
        let mut draft = ForceDraft::new();
        draft.seed(&[live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))]);
        assert!(draft.remove(7).is_some(), "the hull leaves the exercise");
        draft.seed(&[live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))]);
        assert!(!draft.contains(7), "and it does not come back");
        assert_eq!(
            plan(&draft, &[live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))]),
            Ok(vec![ForceWrite::Remove {
                unit_id: 7,
                name: "KRI Nanggala".into()
            }]),
            "the removal is still owed"
        );
    }

    /// A hull the operator never touched is the server's, so the server's
    /// newer word wins: that is the whole point of a re-read.
    #[test]
    fn a_re_read_adopts_the_servers_newer_placement() {
        let mut draft = ForceDraft::new();
        draft.seed(&[live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))]);
        draft.seed(&[live(7, "KRI Nanggala", Some(3), Some((-6.0, 108.0)))]);
        assert_eq!(
            draft.get(7).and_then(|h| h.start),
            Some(Start { lat: -6.0, lon: 108.0 }),
            "the server's placement is the newer word"
        );
    }

    /// A hull the server dropped is gone: it was not the operator's to keep.
    #[test]
    fn a_re_read_forgets_a_hull_the_server_dropped() {
        let mut draft = ForceDraft::new();
        draft.seed(&[live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))]);
        draft.seed(&[]);
        assert!(!draft.contains(7));
    }
}
