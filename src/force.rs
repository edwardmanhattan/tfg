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
    /// THE HULL THAT BROUGHT THIS ONE IN, when the register's pre-embarked
    /// units are what put it here.
    ///
    /// Deploying a parent brings its register contents with it in the same
    /// write (`with_contents`), so these hulls need no write of their own:
    /// no assignment, no embarkation, nothing. The mark is what tells [`plan`]
    /// so, and it is the client's half of a mechanism the server performs
    /// atomically — the operator sees the boats arrive beside the ship
    /// instead of finding them in the next read.
    ///
    /// It is cleared the moment the operator moves the hull off its bringer,
    /// because then it is a hull the operator is placing and the plan owes it
    /// an assignment like any other.
    pub brought_by: Option<i64>,
    /// WHETHER DEPLOYING THIS HULL SHOULD BRING WHAT THE REGISTER SAYS IT
    /// CARRIES — the picker's toggle, carried on the hull it applies to.
    ///
    /// The choice is per hull because the deployment is: one hull arrives with
    /// her boats and the next arrives bare, and the difference is the force the
    /// exercise will have. `false` for a hull the operator has not asked about,
    /// and the only thing that makes [`plan`] name `with_contents` at all. It
    /// is named the wire's word on purpose: one name for one decision.
    pub with_contents: bool,
}

impl DraftHull {
    pub fn new(unit_id: i64, name: String, hull_number: String, class_id: String) -> Self {
        Self {
            unit_id,
            name,
            hull_number,
            class_id,
            start: None,
            commander_id: None,
            host: None,
            course_deg: None,
            brought_by: None,
            with_contents: false,
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
    ///
    /// THE BOATS A HULL BROUGHT STAY IN THE GAME, ON THE WATER. Removing a
    /// parent does not remove the register's pre-embarked units — Minos has
    /// no such cascade, and the boats were deployed pieces in their own right
    /// the moment the parent landed. They are un-hosted here so the plan
    /// never tries to put one aboard a hull that is about to leave.
    pub fn remove(&mut self, unit_id: i64) -> Option<DraftHull> {
        for hull in self.hulls.values_mut() {
            if hull.brought_by == Some(unit_id) {
                hull.brought_by = None;
                hull.host = None;
            }
        }
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
                    // A hull the server already holds was not brought in by
                    // anybody: the mechanism has already happened, and the
                    // frame is the record of it. Nor does it have a choice
                    // left about bringing contents: an assignment already ran.
                    brought_by: None,
                    with_contents: false,
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
            // A HULL THE SERVER HOLDS IS NOT "BROUGHT" ANY MORE: the
            // mechanism already happened, and the draft's mark is what kept
            // the plan from writing it. Keeping the mark past the write would
            // leave a hull the server created looking like one it is about to.
            let mut hull = hull;
            if frame.contains_key(&hull.unit_id) {
                hull.brought_by = None;
            }
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

    /// MOVE A CLUSTER (`§5.6`): shift every named hull's staged position by the
    /// same delta, and answer how many moved.
    ///
    /// A CLUSTER IS A GESTURE, NOT A STORED OBJECT — a group's hulls, or a
    /// selection. It is staged here exactly like one hull's move, so the commit
    /// writes ONE PLACEMENT PER HULL in unit-id order, with no batch route:
    /// `MOVE A CLUSTER → N WRITES, UNIT-ID ORDER, NO BATCH ROUTE`. A hull with
    /// no ground is not moved (there is nothing to shift), and a delta of
    /// nothing moves nothing.
    pub fn move_cluster(&mut self, units: &[i64], d_lat: f64, d_lon: f64) -> usize {
        if d_lat == 0.0 && d_lon == 0.0 {
            return 0;
        }
        let mut moved = 0usize;
        for unit_id in units {
            if let Some(hull) = self.hulls.get_mut(unit_id) {
                if let Some(start) = &mut hull.start {
                    start.lat = (start.lat + d_lat).clamp(-90.0, 90.0);
                    start.lon = (start.lon + d_lon).clamp(-180.0, 180.0);
                    moved += 1;
                }
            }
        }
        moved
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
    ///
    /// MOVING A BROUGHT HULL MAKES IT THE OPERATOR'S. A hull the register put
    /// aboard its parent needs no write while it stays there — the parent's
    /// assignment brings it — but a hull dragged onto another host, or onto
    /// open water, is a statement of the operator's own and the plan owes it
    /// an assignment like any other.
    pub fn set_host(&mut self, unit_id: i64, host: Option<i64>) -> bool {
        match self.hulls.get_mut(&unit_id) {
            Some(hull) => {
                if hull.host != host {
                    hull.brought_by = None;
                }
                hull.host = host;
                true
            }
            None => false,
        }
    }

    /// Stage the register's pre-embarked units beside the hull that brings
    /// them.
    ///
    /// A DEPLOYMENT IS ONE ACT. The server puts a parent's register contents
    /// into the exercise aboard her, in the same transaction as the
    /// assignment — so the console draws them now, in the draft, rather than
    /// showing a ship that will be larger in the next read. Each child is
    /// marked [`DraftHull::brought_by`], which is what keeps [`plan`] from
    /// writing hulls the server is about to create for itself.
    ///
    /// A CHILD THE OPERATOR HAS ALREADY PUT ABOARD IS CLAIMED, not skipped.
    /// The register's copy often arrives after the drag that put the boat on
    /// the ship, and a list that only ever added new rows would leave that
    /// boat looking like an ordinary piece — so the press would assign it and
    /// then attach it to a hull whose own assignment is about to put it
    /// there, which the server refuses as `already aboard this hull`. The
    /// claim only happens when the child is aboard THIS parent and the parent
    /// is bringing her contents; a boat the operator put somewhere else is
    /// theirs, and this list is a statement about the world, never an edit to
    /// the plan.
    ///
    /// Answers how many children the draft now owes to this parent's
    /// assignment.
    pub fn bring_contents(
        &mut self,
        parent: i64,
        children: &[(i64, String, String)],
    ) -> usize {
        let Some(parent_hull) = self.hulls.get(&parent) else {
            return 0;
        };
        if !parent_hull.with_contents {
            return 0;
        }
        let mut claimed = 0usize;
        for (id, name, hull_number) in children {
            if *id == parent || self.removed.contains(id) {
                continue;
            }
            match self.hulls.get_mut(id) {
                Some(child) if child.host == Some(parent) => {
                    if child.brought_by != Some(parent) {
                        child.brought_by = Some(parent);
                        claimed += 1;
                    }
                }
                Some(_) => {}
                None => {
                    self.hulls.insert(
                        *id,
                        DraftHull {
                            unit_id: *id,
                            name: name.clone(),
                            hull_number: hull_number.clone(),
                            class_id: String::new(),
                            start: None,
                            commander_id: None,
                            host: Some(parent),
                            course_deg: None,
                            brought_by: Some(parent),
                            // The boats are already the parent's to bring: a
                            // child is not a deployment of its own.
                            with_contents: false,
                        },
                    );
                    claimed += 1;
                }
            }
        }
        claimed
    }

    /// The hulls standing in the draft only because `parent` brought them.
    pub fn brought_by(&self, parent: i64) -> Vec<&DraftHull> {
        self.hulls
            .values()
            .filter(|hull| hull.brought_by == Some(parent))
            .collect()
    }

    /// Claim ONE child for its parent's assignment, when the register's copy
    /// is already read and the operator's drag put the boat aboard anyway.
    ///
    /// The same rule [`Self::bring_contents`] applies to a whole list, for the
    /// gesture that lands after the list was read: the press must not attach a
    /// unit the parent's own assignment is about to put there. Answers whether
    /// the child is now the parent's to bring.
    pub fn mark_brought(&mut self, parent: i64, child: i64) -> bool {
        let brings = self
            .get(parent)
            .is_some_and(|hull| hull.with_contents && hull.unit_id != child);
        if !brings {
            return false;
        }
        match self.hulls.get_mut(&child) {
            Some(hull) if hull.host == Some(parent) => {
                hull.brought_by = Some(parent);
                true
            }
            _ => false,
        }
    }

    /// SAY WHETHER A HULL BRINGS WHAT THE REGISTER SAYS SHE CARRIES, and stage
    /// or take back exactly what that decision is about.
    ///
    /// One call because the two halves cannot disagree: a hull that will not
    /// bring her boats cannot be standing for them in the draft, and a hull
    /// that will must have them beside her — otherwise the plan either creates
    /// hulls nobody asked for or leaves the ones it shows unwritten. `children`
    /// is the register's copy; it is ignored when the answer is `false`.
    ///
    /// Answers how many children changed hands.
    pub fn set_bring_contents(
        &mut self,
        unit_id: i64,
        want: bool,
        children: &[(i64, String, String)],
    ) -> usize {
        let Some(hull) = self.hulls.get_mut(&unit_id) else {
            return 0;
        };
        hull.with_contents = want;
        if want {
            self.bring_contents(unit_id, children)
        } else {
            self.drop_contents(unit_id)
        }
    }

    /// Take back the hulls the register's copy staged beside `parent`.
    pub fn drop_contents(&mut self, parent: i64) -> usize {
        let ids: Vec<i64> = self
            .hulls
            .values()
            .filter(|hull| hull.brought_by == Some(parent))
            .map(|hull| hull.unit_id)
            .collect();
        for id in &ids {
            self.hulls.remove(id);
        }
        ids.len()
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
        /// Named when the operator has named one. A planning save may hold
        /// hulls whose commanders come later.
        commander_id: Option<i64>,
        /// The picker's toggle, read from the hull being deployed: ask the
        /// server to bring the register's pre-embarked units aboard her.
        with_contents: bool,
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
/// Three states refuse, and all three are Minos's rules rather than this
/// module's. One person commanding two hulls is the first: the server says
/// "That person already commands the unit ... One person commands one
/// thing", and a second hull for the same commander is refused as a conflict
/// rather than as a duplicate. A hull told to ride itself, and a ring of hulls
/// that carry each other, are the next two: the attach endpoint refuses both
/// outright ("putting the group inside the carrier is a ring"), and they are
/// the states a drag can build, so catching them here is the point of this
/// module rather than a courtesy.
///
/// AN UNCOMMANDED HULL IS NOT A REFUSAL. `POST /games/{id}/units` takes the
/// hull alone, and planning a TFG can take months, so a save may leave the
/// commanders for later. The start gate is where a force has to be complete,
/// not the save.
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
///
/// `pub` for the creation flow's REVIEW stage, which asks the same question
/// one stage earlier and must give the same answer: it passes the empty
/// `live` a draft has not written anything to, so every drafted hull is
/// judged as the new one it is (`crate::draft`). Two answers to "is this
/// force writable" is the drift this module exists to prevent.
pub fn gap(draft: &ForceDraft, live: &[LiveHull]) -> Option<ForceGap> {
    // A HULL MAY BE UNCOMMANDED. `POST /games/{id}/units` takes the hull
    // alone: Minos's "a piece always has a commander" is a rule for the START
    // GATE, not for planning, and a plan that takes months begins with hulls
    // whose commanders are not named yet. The console refused the whole press
    // over it, which made an early save impossible.

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
pub fn judge_side_gap(draft: &ForceDraft, judges: &BTreeMap<i64, String>) -> Option<ForceGap> {
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

/// Whether this hull's presence is the PARENT'S ASSIGNMENT'S doing.
///
/// True only when all three hold, because the plan is a statement about what
/// THIS press writes and nothing more:
///
/// - the draft says the hull was brought by a parent (`brought_by`);
/// - the parent is being assigned by this plan (`with_contents` set, and the
///   server does not hold the parent yet) — that is the write the server acts
///   on;
/// - the server does not hold the child yet.
///
/// A hull the server already has is nobody's rider: it exists, and no plan
/// will create it again. The distinction is what keeps a press from asking the
/// server to attach a unit its own assignment is about to put aboard —
/// `unit_ids[0]: already aboard this hull`.
fn brought_in_this_press(draft: &ForceDraft, live: &[LiveHull], hull: &DraftHull) -> bool {
    let Some(parent) = hull.brought_by else {
        return false;
    };
    if live.iter().any(|l| l.unit_id == hull.unit_id) {
        return false;
    }
    let Some(parent_hull) = draft.get(parent) else {
        return false;
    };
    if !parent_hull.with_contents {
        return false;
    }
    !live.iter().any(|l| l.unit_id == parent)
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
        // A HULL THE REGISTER BROUGHT NEEDS NO WRITE OF ITS OWN: the parent's
        // assignment creates it aboard her in the same transaction. See
        // `brought_in_this_press` for the three conditions.
        if brought_in_this_press(draft, live, hull) {
            continue;
        }
        let Some(here) = live.iter().find(|l| l.unit_id == hull.unit_id) else {
            writes.push(ForceWrite::Assign {
                unit_id: hull.unit_id,
                name: hull.name.clone(),
                commander_id: hull.commander_id,
                with_contents: hull.with_contents,
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
        // A HULL THE REGISTER PUT ABOARD IS ALREADY THERE: its host is the
        // bringer, and the bringer's assignment is what puts it there. See
        // pass 1.
        if brought_in_this_press(draft, live, hull) {
            continue;
        }
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

    fn live(
        unit_id: i64,
        name: &str,
        commander_id: Option<i64>,
        start: Option<(f64, f64)>,
    ) -> LiveHull {
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
                    commander_id: Some(3),
                    with_contents: false,
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

    /// A HULL MAY BE SAVED WITHOUT A COMMANDER. Planning a TFG takes months,
    /// and the commanders are named later; `POST /games/{id}/units` takes the
    /// hull alone, so the assignment is the write and the commander is an
    /// optional field on it.
    #[test]
    fn an_uncommanded_hull_is_assigned_without_a_commander() {
        let mut draft = ForceDraft::new();
        draft.upsert(hull(7, "KRI Nanggala", Some((-5.4, 106.2))));
        draft.upsert(hull(8, "KRI Rangsat", Some((-5.1, 106.9))));
        let writes = plan(&draft, &[]).expect("a plan without commanders");
        assert_eq!(
            writes
                .iter()
                .filter(|w| matches!(w, ForceWrite::Assign { .. }))
                .count(),
            2,
            "both hulls are assigned"
        );
        assert!(writes.iter().all(|w| match w {
            ForceWrite::Assign { commander_id, .. } => commander_id.is_none(),
            _ => true,
        }));
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
                ForceWrite::Assign {
                    unit_id: 7,
                    name: "KRI Nanggala".into(),
                    commander_id: Some(3),
                    with_contents: false,
                },
                ForceWrite::Place {
                    unit_id: 7,
                    name: "KRI Nanggala".into(),
                    lat: -5.4,
                    lon: 106.2
                },
                ForceWrite::Assign {
                    unit_id: 8,
                    name: "KRI Rangsat".into(),
                    commander_id: Some(4),
                    with_contents: false,
                },
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
        let server = vec![live(7, "KRI Nanggala", Some(3), Some((-5.40001, 106.2)))];
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
        let names: Vec<&str> = draft
            .uncommanded()
            .iter()
            .map(|h| h.name.as_str())
            .collect();
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

    /// THE REGISTER'S PRE-EMBARKED UNITS NEED NO WRITE OF THEIR OWN.
    ///
    /// Deploying a parent puts them in the exercise aboard her, in the same
    /// assignment (`with_contents`), so a plan that also assigned and embarked
    /// them would ask the server to create a hull it is creating and to embark
    /// one it is embarking. The draft shows them; the plan writes the parent.
    #[test]
    fn a_register_brought_hull_is_planned_with_its_parent_alone() {
        let mut draft = ForceDraft::new();
        let mut parent = hull(5, "KRI Ahmad Yani", None);
        parent.commander_id = Some(3);
        parent.with_contents = true;
        draft.upsert(parent);
        let added = draft.bring_contents(5, &[(7, "KRI Nanggala".into(), "h7".into())]);
        assert_eq!(added, 1, "the boat is staged");
        assert_eq!(draft.get(7).map(|h| h.host), Some(Some(5)), "aboard her");
        assert_eq!(
            plan(&draft, &[]),
            Ok(vec![ForceWrite::Assign {
                unit_id: 5,
                name: "KRI Ahmad Yani".into(),
                commander_id: Some(3),
                with_contents: true,
            }]),
            "the parent is assigned; the boat arrives with her"
        );
    }

    /// MOVED OFF ITS BRINGER, A HULL IS THE OPERATOR'S OWN. The mark is the
    /// client's half of a mechanism the server performed; a hull dragged onto
    /// open water is a hull the plan owes an assignment like any other.
    #[test]
    fn a_hull_moved_off_its_bringer_is_assigned_again() {
        let mut draft = ForceDraft::new();
        let mut parent = hull(5, "KRI Ahmad Yani", None);
        parent.commander_id = Some(3);
        parent.with_contents = true;
        draft.upsert(parent);
        draft.bring_contents(5, &[(7, "KRI Nanggala".into(), "h7".into())]);
        assert!(draft.set_host(7, None));
        let writes = plan(&draft, &[]).expect("plans");
        assert!(
            writes.contains(&ForceWrite::Assign {
                unit_id: 7,
                name: "KRI Nanggala".into(),
                commander_id: None,
                with_contents: false,
            }),
            "the hull is on the water and needs an assignment: {writes:?}"
        );
    }

    /// A BOAT OUTLIVES ITS SHIP IN THE DRAFT TOO: removing the parent leaves
    /// the register's units in the exercise, on the water, because Minos has
    /// no cascade and a hull deployed once is a piece of its own.
    #[test]
    fn removing_a_bringer_orphans_its_boats_on_the_water() {
        let mut draft = ForceDraft::new();
        let mut parent = hull(5, "KRI Ahmad Yani", None);
        parent.with_contents = true;
        draft.upsert(parent);
        draft.bring_contents(5, &[(7, "KRI Nanggala".into(), "h7".into())]);
        draft.remove(5);
        let boat = draft.get(7).expect("the boat stays");
        assert_eq!(boat.host, None, "on the water");
        assert_eq!(boat.brought_by, None, "and nobody's rider");
    }

    /// THE TOGGLE IS THE OPERATOR'S, and the plan says what they chose: a hull
    /// deployed bare is assigned bare, and one asked to bring her boats says
    /// so — the same draft, two different forces.
    #[test]
    fn the_assignment_carries_the_operator_s_choice() {
        let mut draft = ForceDraft::new();
        let mut bare = hull(5, "KRI Ahmad Yani", None);
        bare.with_contents = false;
        draft.upsert(bare);
        let mut bringing = hull(6, "KRI Diponegoro", None);
        bringing.with_contents = true;
        draft.upsert(bringing);
        let writes = plan(&draft, &[]).expect("plans");
        let asked: Vec<(i64, bool)> = writes
            .iter()
            .filter_map(|w| match w {
                ForceWrite::Assign {
                    unit_id,
                    with_contents,
                    ..
                } => Some((*unit_id, *with_contents)),
                _ => None,
            })
            .collect();
        assert_eq!(asked, vec![(5, false), (6, true)]);
    }

    /// SAYING "NO" TAKES BACK WHAT SAID "YES" STAGED — and saying "yes" again
    /// puts them back, because the toggle is a decision and not a one-way
    /// door.
    #[test]
    fn the_toggle_stages_and_unstages_a_hulls_boats() {
        let mut draft = ForceDraft::new();
        let mut parent = hull(5, "KRI Ahmad Yani", None);
        parent.with_contents = true;
        draft.upsert(parent);
        let boats = [(7, "KRI Nanggala".to_string(), "h7".to_string())];
        assert_eq!(draft.set_bring_contents(5, true, &boats), 1, "brought");
        assert!(draft.contains(7));
        assert_eq!(draft.set_bring_contents(5, false, &boats), 1, "taken back");
        assert!(!draft.contains(7), "and gone from the draft");
        assert_eq!(draft.set_bring_contents(5, true, &boats), 1, "brought again");
        assert!(draft.contains(7));
    }

    /// THE REPORTED 400: a boat the operator dragged aboard a hull that is
    /// itself being deployed with `with_contents`.
    ///
    /// The register's copy lands after the drag, and the boat is already in the
    /// draft with that host. The press must write the PARENT alone — the
    /// server creates the boat aboard her — or the attach that follows is
    /// refused with `unit_ids[0]: that unit is already aboard this hull`.
    #[test]
    fn a_boat_dragged_aboard_before_the_register_copy_is_not_embarked_twice() {
        let mut draft = ForceDraft::new();
        let mut parent = hull(590, "KRI Makassar", None);
        parent.with_contents = true;
        draft.upsert(parent);
        // The drag: the boat is aboard, and nothing marks it yet.
        let mut boat = hull(800, "Leopard 2A4", None);
        boat.host = Some(590);
        draft.upsert(boat);
        // The register's copy lands.
        assert_eq!(
            draft.bring_contents(590, &[(800, "Leopard 2A4".into(), "h800".into())]),
            1,
            "the existing boat is claimed, not skipped"
        );
        assert_eq!(
            plan(&draft, &[]),
            Ok(vec![ForceWrite::Assign {
                unit_id: 590,
                name: "KRI Makassar".into(),
                commander_id: None,
                with_contents: true,
            }]),
            "the parent's assignment creates the boat: no second write"
        );
    }

    /// THE OTHER SIDE OF THAT RULE: when the parent is ALREADY in the exercise
    /// her assignment does not run, so her boats are ordinary deployments and
    /// must be written — the skip is about this press, not about the mark.
    #[test]
    fn a_boat_of_a_hull_the_server_already_has_is_written_itself() {
        let mut draft = ForceDraft::new();
        let mut parent = hull(590, "KRI Makassar", None);
        parent.with_contents = true;
        draft.upsert(parent);
        let mut boat = hull(800, "Leopard 2A4", None);
        boat.host = Some(590);
        boat.brought_by = Some(590);
        draft.upsert(boat);
        let writes = plan(&draft, &[live(590, "KRI Makassar", None, None)]).expect("plans");
        assert!(
            writes.iter().any(|w| matches!(
                w,
                ForceWrite::Assign { unit_id: 800, .. }
            )),
            "the boat is assigned by this press: {writes:?}"
        );
        assert!(
            writes
                .iter()
                .any(|w| matches!(w, ForceWrite::Embark { child: 800, .. })),
            "and put aboard her: {writes:?}"
        );
    }

    /// The embarkation endpoint 404s on a hull — or a host — the server has
    /// never heard of, so a new host and a new child are both assigned
    /// before the link that names them is posted.
    #[test]
    fn an_embark_is_ordered_after_both_hulls_assignments() {        let mut draft = ForceDraft::new();
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
                ForceWrite::Assign {
                    unit_id: 5,
                    name: "KRI Ahmad Yani".into(),
                    commander_id: Some(3),
                    with_contents: false,
                },
                ForceWrite::Assign {
                    unit_id: 7,
                    name: "KRI Nanggala".into(),
                    commander_id: Some(4),
                    with_contents: false,
                },
                ForceWrite::Embark {
                    host: 5,
                    child: 7,
                    name: "KRI Nanggala".into()
                },
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
        draft.set_start(
            7,
            Some(Start {
                lat: -5.0,
                lon: 107.0,
            }),
        );
        draft.seed(&[live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))]);
        assert_eq!(
            draft.get(7).and_then(|h| h.start),
            Some(Start {
                lat: -5.0,
                lon: 107.0
            }),
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
            plan(
                &draft,
                &[live(7, "KRI Nanggala", Some(3), Some((-5.4, 106.2)))]
            ),
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
            Some(Start {
                lat: -6.0,
                lon: 108.0
            }),
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

#[cfg(test)]
mod cluster_tests {
    use super::*;

    fn force() -> ForceDraft {
        let mut force = ForceDraft::new();
        for (unit_id, start) in [
            (
                351,
                Some(Start {
                    lat: -3.0,
                    lon: 108.0,
                }),
            ),
            (
                352,
                Some(Start {
                    lat: -3.1,
                    lon: 108.1,
                }),
            ),
            (353, None),
        ] {
            force.upsert(DraftHull {
                unit_id,
                name: format!("KRI {unit_id}"),
                hull_number: format!("{unit_id}"),
                class_id: "AHMAD YANI".into(),
                start,
                commander_id: None,
                host: None,
                course_deg: None,
                brought_by: None,
                with_contents: false,
            });
        }
        force
    }

    /// A CLUSTER MOVES EVERY HULL THAT HAS GROUND, and leaves the rest alone.
    #[test]
    fn a_cluster_moves_what_is_placed() {
        let mut force = force();
        let moved = force.move_cluster(&[351, 352, 353], 0.5, -0.25);
        assert_eq!(moved, 2, "the unplaced hull has nothing to shift");
        let a = force.get(351).unwrap().start.clone().unwrap();
        let b = force.get(352).unwrap().start.clone().unwrap();
        assert!((a.lat + 2.5).abs() < 1e-9 && (a.lon - 107.75).abs() < 1e-9);
        assert!((b.lat + 2.6).abs() < 1e-9 && (b.lon - 107.85).abs() < 1e-9);
        assert!(force.get(353).unwrap().start.is_none());
    }

    /// A DELTA OF NOTHING MOVES NOTHING — the verb is not a no-op write.
    #[test]
    fn a_empty_move_stages_nothing() {
        let mut force = force();
        assert_eq!(force.move_cluster(&[351, 352], 0.0, 0.0), 0);
    }
}
