//! The press: two ladders, one write at a time, a journal, and no rollback.
//!
//! STAGES 01–06 WRITE NOTHING. The draft is local (`crate::draft`), and this
//! module is the only place in the creation flow that talks to the server.
//! [`plan`] turns a draft plus what the server already holds into the writes
//! still owed, in ladder order; [`run`] issues them one at a time against a
//! [`Transport`], recording every answer in the draft's [`Journal`]. Nothing
//! here builds an HTTP request: the transport does, from [`Write`], and the
//! seam exists so the whole engine is testable with a fake.
//!
//! TWO LADDERS, AND THEY ARE NOT THE SAME LADDER (§9.2):
//!
//! ```text
//! GAME     01 GAME        POST /games
//!          02 SEATS       POST /games/{id}/participants        [before units]
//!          03 HIERARCHY   POST /games/{id}/hierarchy, top-down  [parent first]
//!          04 UNITS       POST /games/{id}/units, then the tree
//!          05 PLACEMENTS  PUT /games/{id}/units/{u}/placement
//!          06 STEPS       scenarios, steps, sub-steps, audiences
//!          07 COMMANDERS  PUT .../hierarchy/{node}/commander
//!          08 READINESS   GET /games/{id}/readiness — A READ
//!          09 TRANSITION  POST /games/{id}/transitions
//!
//! MISSION  M1 MISSION  POST /scenario-missions
//!          M2 STEPS    POST /scenario-missions/{id}/steps
//!          M3 COMMANDS POST .../steps/{s}/substeps, PUT .../substeps/{s}/related
//! ```
//!
//! The order is not the sheet's, and the corrections are erratum 1: `POST
//! /games/{id}/units` requires `id_commander`, and that commander must
//! already be a participant, so SEATS precedes UNITS; and a hull's tree
//! placement names a node, so HIERARCHY precedes UNITS too. The mission ladder
//! writes no game, no seats, no hierarchy, no placements, no commanders and no
//! transition — which is what `NO MAP · NO SEATS, NO GROUPS` means.
//!
//! EACH POSITION IS BUILT WHEN IT IS REACHED, not all at the start, because
//! every position after 01 is addressed BY THE GAME'S ID and that id does not
//! exist until 01 answers. Before the press the same builder draws the ladder
//! with the reads it has; during the press it draws it with what the press has
//! learned.
//!
//! COMMIT IS A DIFF WITH A JOURNAL, NOT A REPLAY (§9.3):
//!
//! - **Plan against live state.** A hull already in the game is not assigned
//!   again, a placement that matches is not re-sent, a participant whose role
//!   matches is not re-seated.
//! - **Record what landed.** Every answer goes into the [`Journal`], with the
//!   id the server returned, so a staged node, book, step or sub-step can be
//!   named by every later write in the same press.
//! - **On failure, re-read and re-plan.** [`run`] stops at the failed
//!   position; the operator presses again, the caller refreshes [`Live`], and
//!   a second run skips every position the journal says settled.
//!
//! NOTHING ROLLS BACK. There is no compensation and no delete-the-game: the
//! draft stays, because it is the only record of what was intended, and the
//! first failure is the server's.
//!
//! THE LOST FIRST WRITE. `POST /games` has no idempotency key, so a timeout on
//! 01 may have made the game anyway. A bare "retry 01" would then make a
//! duplicate the operator cannot see, so [`run`] re-reads by name instead:
//! exactly one recent match is adopted, none offers a retry, and more than one
//! stops and names them.
//!
//! THREE SEAMS THIS MODULE DOES NOT CLOSE, and refuses rather than fakes:
//!
//! - A step-level `/related` route ALWAYS answers 409 on both ladders, so a
//!   staged step-level relation is a [`Plan::gaps`] entry, not a write.
//! - The book holds no sub-steps yet (stage 04, the handoff's slice 9), so M3
//!   and 06's sub-step and audience writes have nothing to carry.
//! - A write's REQUEST BODY is the transport's: a game audience is
//!   `{related:[{kind:"hull", id}]}` and a template audience is `{units:[…]}`,
//!   and [`Write::SetAudience`] hands over the draft's unit ids and no JSON.

use crate::backend::{GamePace, TimeWindow};
use crate::book::{self, BookWrite, LiveScenario, ScenarioRef, StepRef};
use crate::draft::{BaseState, Blocker, Draft, Journal, Ladder, Landing, Verb};
use crate::force::{self, ForceWrite, LiveHull};
use crate::gamestate::GameMode;
use crate::groups::{self, GroupRef, GroupWrite, LiveGroup};
use crate::roster::{self, LiveSeat, RosterWrite};

/// How recently a game must have been created to be the one a lost 01 made.
///
/// A minute would be too tight — a slow round trip under load is still the
/// same press — and an hour is a window in which an operator can genuinely
/// create two games of the same name. Five minutes is the window §9.3 tells
/// the operator to look inside.
pub const LOST_GAME_WINDOW_MINUTES: i64 = 5;

// ===========================================================================
// What the server holds
// ===========================================================================

/// What the server already holds, as the diff's reference.
///
/// THE FRAME IS THE PRESS'S, NOT THE DRAFT'S. The draft is the operator's
/// intent; this is the record it is compared against, read fresh before the
/// press and read again after a failure. Every field is a type the modules
/// that own the rules already use, so the diff is their diff and this module
/// only decides the order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Live {
    /// The game's id, when the server holds one. A `Reopened` draft has it
    /// before the press; a `Blank` one learns it from 01.
    pub game_id: Option<i64>,
    pub seats: Vec<LiveSeat>,
    pub nodes: Vec<LiveGroup>,
    pub units: Vec<LiveHull>,
    /// The game's books, with their step ids — a step-add's answer is the
    /// whole scenario, so the ids already there are what tells the new one
    /// apart.
    pub scenarios: Vec<LiveScenario>,
}

impl Live {
    /// What the server holds for a `Reopened` draft: nothing staged yet, and
    /// the game's own id.
    pub fn reopened(game_id: i64) -> Self {
        Live {
            game_id: Some(game_id),
            ..Live::default()
        }
    }
}

// ===========================================================================
// The writes
// ===========================================================================

/// A row another write names: one the server already holds, or one the same
/// press is about to create.
///
/// A staged id is negative and never collides with a server id. The transport
/// resolves it from the journal — the only place the answer that made it
/// lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ref {
    Staged(i64),
    Live(i64),
}

impl Ref {
    pub fn of(id: i64) -> Self {
        if id < 0 {
            Ref::Staged(id)
        } else {
            Ref::Live(id)
        }
    }

    /// The id this ref names, staged-negative or server-positive.
    pub fn id(self) -> i64 {
        match self {
            Ref::Staged(id) | Ref::Live(id) => id,
        }
    }

    pub fn is_staged(self) -> bool {
        matches!(self, Ref::Staged(_))
    }
}

impl From<GroupRef> for Ref {
    fn from(r: GroupRef) -> Self {
        match r {
            GroupRef::New(id) => Ref::Staged(id),
            GroupRef::Live(id) => Ref::Live(id),
        }
    }
}

impl From<StepRef> for Ref {
    fn from(r: StepRef) -> Self {
        match r {
            StepRef::New(id) => Ref::Staged(id),
            StepRef::Live(id) => Ref::Live(id),
        }
    }
}

impl From<ScenarioRef> for Ref {
    fn from(r: ScenarioRef) -> Self {
        match r {
            ScenarioRef::New(id) => Ref::Staged(id),
            ScenarioRef::Live(id) => Ref::Live(id),
        }
    }
}

/// The body of 01, as the draft states it.
///
/// `max_echelon_rank` is a RANK NUMBER — `40`, not a hierarchy id (§12.7) —
/// and the two ends carry the values derived from LENGTH, never a typed end
/// (§4.1). `overlay_text` is deliberately absent: the create accepts it and no
/// frame shows it (§12.12).
#[derive(Debug, Clone, PartialEq)]
pub struct NewGameBody {
    pub name: String,
    pub mode: GameMode,
    pub pace: GamePace,
    pub description: String,
    pub purpose: String,
    pub target: String,
    pub area: String,
    pub map_tag: String,
    pub max_echelon_rank: Option<i64>,
    /// All four ends, or none. Half a window is never written.
    pub window: TimeWindow,
    pub time_factor: Option<f64>,
}

/// One write the ladder owes. Each is one endpoint, and the field names are
/// the body's.
#[derive(Debug, Clone, PartialEq)]
pub enum Write {
    // -- the game ladder -------------------------------------------------
    /// 01 — `POST /games`.
    CreateGame(NewGameBody),
    /// 02 — `POST /games/{id}/participants`.
    ///
    /// BY ROLE NAME, NEVER BY ID. A blank draft's picker offers the template
    /// vocabulary (`GET /game-roles`), and the server refuses a template id
    /// with a 409 because the two vocabularies' ids do not overlap (§12.3).
    /// The transport reads the game's own roles and maps the name.
    Seat {
        user_id: i64,
        role_name: String,
        call_sign: String,
    },
    /// 02 — `PATCH /games/{id}/participants/{user_id}`. The server clears the
    /// seat's readiness, so the caller re-reads the roster after one.
    Reseat { user_id: i64, role_name: String },
    /// 02 — `DELETE /games/{id}/participants/{user_id}`. A hard delete, and
    /// idempotent.
    Unseat { user_id: i64 },
    /// 03 — `POST /games/{id}/hierarchy`.
    CreateNode {
        local_id: i64,
        name: String,
        /// The echelon's `id_unit_hierarchy`. The rank is checked against
        /// `max_echelon_rank` before the press, not here. `0` when the staged
        /// row has none — the flow's rung is a local kind — and the transport
        /// resolves it from the level before sending.
        echelon_id: i64,
        /// The level itself, for a write whose id is `0`.
        kind: crate::groups::GroupKind,
        parent: Option<Ref>,
    },
    /// 03 — `PATCH /games/{id}/hierarchy/{node_id}`.
    RenameNode { node: Ref, name: String },
    /// 03 — `PUT /games/{id}/hierarchy/{node_id}/parent`.
    MoveNode { node: Ref, parent: Ref },
    /// 03 — `DELETE /games/{id}/hierarchy/{node_id}/parent`.
    UnparentNode { node: Ref },
    /// 03 — `DELETE /games/{id}/hierarchy/{node_id}` — the node and its
    /// subtree.
    DeleteNode { node: Ref },
    /// 04 — `POST /games/{id}/units`. Minos requires a commander, and that
    /// commander must already be a participant: this is why SEATS is 02.
    AssignUnit { unit_id: i64, commander_id: i64 },
    /// 04 — `PUT /games/{id}/units/{unit_id}/hierarchy`.
    PlaceUnitInNode { unit_id: i64, node: Ref },
    /// 04 — `DELETE /games/{id}/units/{unit_id}/hierarchy`.
    UnplaceUnit { unit_id: i64 },
    /// 04 — `POST /games/{id}/units/{unit_id}/embarkations`.
    Embark { host: i64, child: i64 },
    /// 04 — `POST /games/{id}/units/{unit_id}/embarkations/detach`.
    Disembark { host: i64, child: i64 },
    /// 04 — `DELETE /games/{id}/units/{unit_id}`.
    RemoveUnit { unit_id: i64 },
    /// 05 — `PUT /games/{id}/units/{unit_id}/placement`. One write per hull,
    /// in unit-id order; there is no batch route.
    Place { unit_id: i64, lat: f64, lon: f64 },
    /// 05 — `DELETE /games/{id}/units/{unit_id}/placement`. The hull stays in
    /// the game.
    Lift { unit_id: i64 },
    /// 06 / M1 — `POST /games/{id}/scenarios` or `POST /scenario-missions`.
    ///
    /// A game's scenario takes the title and the description too (the body
    /// the collection shows), and `id_mission` is NEVER sent: a draft seeded
    /// from a Saved Scenario may have been edited, and replaying the mission
    /// would discard the edits (§9.3).
    CreateBook {
        local_id: i64,
        title: String,
        description: String,
    },
    /// 06 / M2 — `POST .../steps`.
    ///
    /// `kind` is `None` today: the book's model does not carry a step's kind
    /// yet, and a step created without one takes the server's own default —
    /// which is what the console's composer already sends. Stage 04's
    /// `+ STEP` / `+ BEAT` is what fills it in.
    CreateStep {
        book: Ref,
        local_id: i64,
        kind: Option<String>,
        content: String,
        window: Option<(String, String)>,
    },
    /// 06 — `DELETE /games/{id}/scenarios/{sid}/steps/{step_id}`.
    DeleteStep { book: Ref, step_id: i64 },
    /// M3 — `POST .../steps/{step_id}/substeps`.
    ///
    /// A `beat` refuses this with a 409, so the composer refuses a sub-step on
    /// a beat locally rather than letting the server say it. The book now holds
    /// sub-steps (stage 04), so this is emitted by the step position of both
    /// ladders — a step-level relation is still a [`gaps`] entry, never a
    /// write.
    CreateSubstep {
        book: Ref,
        step: Ref,
        local_id: i64,
        content: String,
    },
    /// M3 — `PUT .../substeps/{substep_id}/related`. THE WHOLE LIST, not a
    /// delta; empty clears it. Template ladders take register hull ids under
    /// `units`, game ladders take `related:[{kind:"hull", id}]`.
    SetAudience {
        book: Ref,
        step: Ref,
        substep: Ref,
        hulls: Vec<i64>,
        /// THE NODES THE AUDIENCE CAME FROM — provenance, not a row. A group
        /// dropped on a sub-step resolves to its leaf hulls at drop time; the
        /// nodes are shown so the operator can see where they came from, and
        /// the wire never hears about them.
        nodes: Vec<i64>,
    },
    /// 07 — `PUT /games/{id}/hierarchy/{node_id}/commander`. A participant,
    /// and not on the judge side.
    CommandNode { node: Ref, user_id: i64 },
    /// 07 — `DELETE /games/{id}/hierarchy/{node_id}/commander`.
    ClearNodeCommander { node: Ref },
    /// 07 — `PATCH /games/{id}/units/{unit_id}`: a hull's commander where 04
    /// did not create the hull.
    ReassignUnit { unit_id: i64, commander_id: i64 },
    /// 09 — `POST /games/{id}/transitions`. Play Directly only.
    Transition { to: &'static str },
}

impl Write {
    /// What this write is, for the failure block's line.
    pub fn label(&self) -> String {
        match self {
            Write::CreateGame(body) => format!("create game {:?}", body.name),
            Write::Seat { user_id, .. } => format!("seat user {user_id}"),
            Write::Reseat { user_id, .. } => format!("re-role user {user_id}"),
            Write::Unseat { user_id } => format!("unseat user {user_id}"),
            Write::CreateNode { name, .. } => format!("create node {name:?}"),
            Write::RenameNode { node, .. } => format!("rename node {}", node.id()),
            Write::MoveNode { node, .. } => format!("move node {}", node.id()),
            Write::UnparentNode { node } => format!("unparent node {}", node.id()),
            Write::DeleteNode { node } => format!("delete node {}", node.id()),
            Write::AssignUnit { unit_id, .. } => format!("assign unit {unit_id}"),
            Write::PlaceUnitInNode { unit_id, node } => {
                format!("place unit {unit_id} in node {}", node.id())
            }
            Write::UnplaceUnit { unit_id } => format!("unplace unit {unit_id}"),
            Write::Embark { child, host } => format!("embark unit {child} on {host}"),
            Write::Disembark { child, host } => format!("detach unit {child} from {host}"),
            Write::RemoveUnit { unit_id } => format!("remove unit {unit_id}"),
            Write::Place { unit_id, .. } => format!("place unit {unit_id}"),
            Write::Lift { unit_id } => format!("lift unit {unit_id}"),
            Write::CreateBook { title, .. } => format!("create book {title:?}"),
            Write::CreateStep { content, .. } => {
                let short: String = content.chars().take(40).collect();
                format!("create step {short:?}")
            }
            Write::DeleteStep { step_id, .. } => format!("delete step {step_id}"),
            Write::CreateSubstep { content, .. } => {
                let short: String = content.chars().take(40).collect();
                format!("create sub-step {short:?}")
            }
            Write::SetAudience { hulls, .. } => format!("relate {} hull(s)", hulls.len()),
            Write::CommandNode { node, user_id } => {
                format!("command node {} with user {user_id}", node.id())
            }
            Write::ClearNodeCommander { node } => {
                format!("clear commander on node {}", node.id())
            }
            Write::ReassignUnit { unit_id, .. } => format!("re-command unit {unit_id}"),
            Write::Transition { to } => format!("transition to {to}"),
        }
    }

    /// The staged row this write creates, when it creates one. The journal
    /// records the id the answer carried against it.
    fn created(&self) -> Option<Created> {
        match self {
            Write::CreateNode { local_id, .. } => Some(Created::Node(*local_id)),
            Write::CreateBook { local_id, .. } => Some(Created::Book(*local_id)),
            Write::CreateStep { local_id, .. } => Some(Created::Step(*local_id)),
            Write::CreateSubstep { local_id, .. } => Some(Created::Substep(*local_id)),
            _ => None,
        }
    }
}

/// A staged row a write creates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Created {
    Node(i64),
    Book(i64),
    Step(i64),
    Substep(i64),
}

// ===========================================================================
// The plan
// ===========================================================================

/// One position on a ladder: what the sheet's right-hand column draws as a row.
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub number: u8,
    /// The row's name: `GAME`, `SEATS`, `M1 MISSION`…
    pub name: &'static str,
    /// The row's one-line description.
    pub note: &'static str,
    pub writes: Vec<Write>,
    /// 08 is the gate: a READ, so it is never "written", and a counter that
    /// said it was would be lying about the one position whose whole point is
    /// that nobody writes it.
    pub read: Option<Read>,
}

/// What a position has done so far, as F7's status column draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Written,
    /// The gate, when the read answered.
    Read,
    /// The server refused or did not answer.
    Failed,
    Pending,
}

impl Position {
    /// This position's status against what a previous press recorded.
    pub fn status(&self, journal: &Journal, ladder: Ladder) -> Status {
        match journal.landing(ladder, self.number) {
            Some(Landing::Written { .. }) => Status::Written,
            Some(Landing::Read) => Status::Read,
            Some(Landing::Failed { .. }) => Status::Failed,
            None => Status::Pending,
        }
    }
}

/// The writes still owed, and the reasons the ladder cannot be built.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub positions: Vec<Position>,
    /// Places the draft asks for something the ladder has no honest write
    /// for. A gap is not a failure — it is the plan refusing to send a write
    /// it knows will be refused, so the operator hears it before the press
    /// rather than as a 409 in the middle of one.
    pub gaps: Vec<String>,
}

impl Plan {
    /// The positions still owed: everything the journal has not settled.
    pub fn owed(&self, journal: &Journal, ladder: Ladder) -> Vec<&Position> {
        self.positions
            .iter()
            .filter(|p| !matches!(p.status(journal, ladder), Status::Written | Status::Read))
            .collect()
    }
}

/// The whole ladder, as the draft and this read stand.
///
/// `offset` is the frame the window's instants are written in — the server's
/// own when it stated one, the client's otherwise. It is a parameter because a
/// plan is a pure function of the draft, the live read and that frame.
pub fn plan(draft: &Draft, live: &Live, verb: Verb, offset: &str) -> Plan {
    Plan {
        positions: (1..=verb.stops_at())
            .map(|number| position(draft, live, verb, number, offset))
            .collect(),
        gaps: gaps(draft, live, verb),
    }
}

/// One position, built from the draft and the read AS THEY ARE NOW.
///
/// This is the unit [`run`] uses, because position 02 onwards is addressed by
/// the game's id and that id arrives with 01's answer — a ladder built in one
/// pass before the press cannot name a game it has not made.
pub fn position(draft: &Draft, live: &Live, verb: Verb, number: u8, offset: &str) -> Position {
    if verb.ladder() == Ladder::Mission {
        return mission_position(draft, live, number);
    }
    match number {
        1 => {
            let mut writes = Vec::new();
            if game_id(draft, live).is_none() && draft.terms.time.state() != BaseState::Partial {
                writes.push(Write::CreateGame(NewGameBody {
                    name: draft.terms.name.trim().to_string(),
                    mode: draft.mode,
                    pace: draft.pace,
                    description: draft.terms.description.clone(),
                    purpose: draft.terms.purpose.clone(),
                    target: draft.terms.target.clone(),
                    area: draft.terms.area.clone(),
                    map_tag: draft.terms.map_tag.clone(),
                    max_echelon_rank: draft.ceiling,
                    window: draft.terms.time.window(offset).unwrap_or_default(),
                    time_factor: draft.terms.time.factor(),
                }));
            }
            Position {
                number,
                name: "GAME",
                note: "the game record",
                writes,
                read: None,
            }
        }
        // Before units, because a hull's commander must already be a
        // participant.
        2 => Position {
            number,
            name: "SEATS",
            note: "who sits in it",
            writes: roster::plan(&draft.roster, &live.seats)
                .into_iter()
                .map(seat_write)
                .collect(),
            read: None,
        },
        3 => Position {
            number,
            name: "HIERARCHY",
            note: "the task organisation, a parent before its child",
            writes: tree(draft, live)
                .into_iter()
                .filter_map(node_write)
                .collect(),
            read: None,
        },
        4 => {
            // The order inside 04 is load-bearing: a hull before the node that
            // names it, embarkation after both ends exist, removals last.
            let mut writes: Vec<Write> = force(draft, live)
                .into_iter()
                .filter_map(|w| match w {
                    ForceWrite::Assign {
                        unit_id,
                        commander_id,
                        ..
                    } => Some(Write::AssignUnit {
                        unit_id,
                        commander_id,
                    }),
                    _ => None,
                })
                .collect();
            writes.extend(
                tree(draft, live)
                    .into_iter()
                    .filter_map(unit_placement_write),
            );
            writes.extend(force(draft, live).into_iter().filter_map(|w| match w {
                ForceWrite::Embark { host, child, .. } => Some(Write::Embark { host, child }),
                ForceWrite::Disembark { host, child, .. } => Some(Write::Disembark { host, child }),
                _ => None,
            }));
            writes.extend(force(draft, live).into_iter().filter_map(|w| match w {
                ForceWrite::Remove { unit_id, .. } => Some(Write::RemoveUnit { unit_id }),
                _ => None,
            }));
            Position {
                number,
                name: "UNITS",
                note: "each hull, its place in the tree, then who is aboard whom",
                writes,
                read: None,
            }
        }
        5 => Position {
            number,
            name: "PLACEMENTS",
            note: "where each hull starts",
            writes: force(draft, live)
                .into_iter()
                .filter_map(|w| match w {
                    ForceWrite::Place {
                        unit_id, lat, lon, ..
                    } => Some(Write::Place { unit_id, lat, lon }),
                    ForceWrite::Lift { unit_id, .. } => Some(Write::Lift { unit_id }),
                    _ => None,
                })
                .collect(),
            read: None,
        },
        6 => Position {
            number,
            name: "STEPS",
            note: "the book: scenarios, steps, sub-steps, audiences",
            writes: if draft.book.is_empty() || draft.mode != GameMode::Scenario {
                Vec::new()
            } else {
                book::plan(&draft.book, &live.scenarios)
                    .into_iter()
                    // A RESUME MUST NOT CREATE THE SAME PAGE TWICE. A staged
                    // book is not in `live` — it was not on the server when
                    // the draft was seeded — so the plan's create is the only
                    // thing that would run again after a press made it and
                    // failed later in the same position. The journal's
                    // adopted ids are the record that it landed.
                    .filter(|write| !already_booked(draft, write))
                    .filter_map(step_write)
                    .collect()
            },
            read: None,
        },
        7 => {
            let mut writes: Vec<Write> = tree(draft, live)
                .into_iter()
                .filter_map(commander_write)
                .collect();
            // A commander change on a hull the server already holds is 07's,
            // not 04's: 04 creates, 07 re-commands.
            writes.extend(force(draft, live).into_iter().filter_map(|w| match w {
                ForceWrite::HandOver {
                    unit_id,
                    commander_id,
                    ..
                } => Some(Write::ReassignUnit {
                    unit_id,
                    commander_id,
                }),
                _ => None,
            }));
            Position {
                number,
                name: "COMMANDERS",
                note: "who commands what",
                writes,
                read: None,
            }
        }
        8 => Position {
            number,
            name: "READINESS",
            note: "who has declared \u{2014} a read, never a write",
            writes: Vec::new(),
            read: game_id(draft, live).map(|game_id| Read::Readiness { game_id }),
        },
        9 => Position {
            number,
            name: "TRANSITION",
            note: "into preparation",
            writes: vec![Write::Transition { to: "preparation" }],
            read: None,
        },
        other => Position {
            number: other,
            name: "UNKNOWN",
            note: "not a position on this ladder",
            writes: Vec::new(),
            read: None,
        },
    }
}

fn mission_position(draft: &Draft, live: &Live, number: u8) -> Position {
    match number {
        1 => Position {
            number,
            name: "M1 MISSION",
            note: "the saved scenario's own name and description",
            writes: if draft.journal.mission_id().is_some() {
                Vec::new()
            } else {
                vec![Write::CreateBook {
                    local_id: draft
                        .book
                        .staged_news()
                        .next()
                        .map(|b| b.local_id)
                        .unwrap_or(0),
                    title: draft.saved_title().trim().to_string(),
                    description: draft.saved_description().trim().to_string(),
                }]
            },
            read: None,
        },
        2 => Position {
            number,
            name: "M2 STEPS",
            note: "its steps, in order",
            writes: mission_steps(draft),
            read: None,
        },
        // M3 is the commands: a mission ladder's audience is written on a
        // sub-step and nowhere else, so this position is exactly the book's
        // sub-steps and their audiences — in the order the composer authored
        // them, each command's audience straight after it.
        other => Position {
            number: other,
            name: "M3 COMMANDS",
            note: "each command, then who it obliges",
            writes: if draft.book.is_empty() {
                Vec::new()
            } else {
                book::plan(&draft.book, &live.scenarios)
                    .into_iter()
                    .filter_map(step_write)
                    .filter(|w| {
                        matches!(w, Write::CreateSubstep { .. } | Write::SetAudience { .. })
                    })
                    .collect()
            },
            read: None,
        },
    }
}

/// The steps of the one book a Saved Scenario is.
fn mission_steps(draft: &Draft) -> Vec<Write> {
    let Some(local) = draft.book.staged_news().next().map(|b| b.local_id) else {
        return Vec::new();
    };
    book::plan(&draft.book, &[])
        .into_iter()
        .filter_map(step_write)
        .filter(|w| matches!(w, Write::CreateStep { book, .. } if *book == Ref::Staged(local)))
        .collect()
}

// -- the write builders ----------------------------------------------------

fn seat_write(w: RosterWrite) -> Write {
    match w {
        RosterWrite::Seat {
            user_id,
            role_name,
            call_sign,
            ..
        } => Write::Seat {
            user_id,
            role_name,
            call_sign,
        },
        RosterWrite::Role {
            user_id, role_name, ..
        } => Write::Reseat { user_id, role_name },
        RosterWrite::Unseat { user_id, .. } => Write::Unseat { user_id },
    }
}

/// The tree's node writes: creates, renames, re-parents and removals. The
/// unit placements and the commanders are the same plan, split across 04 and
/// 07 because that is where the ladder puts them.
fn node_write(w: GroupWrite) -> Option<Write> {
    match w {
        GroupWrite::CreateNode {
            local_id,
            name,
            echelon_id,
            kind,
            parent,
        } => Some(Write::CreateNode {
            local_id,
            name,
            echelon_id,
            // The level, for a write whose id is `0`: the flow stages a rung,
            // not a master-data id, and the transport resolves the name
            // against `GET /hierarchy` like it resolves a seat's role.
            kind,
            parent: parent.map(Ref::from),
        }),
        GroupWrite::Rename { group_id, name } => Some(Write::RenameNode {
            node: Ref::Live(group_id),
            name,
        }),
        GroupWrite::MoveNode { group_id, parent } => Some(Write::MoveNode {
            node: Ref::Live(group_id),
            parent: Ref::from(parent),
        }),
        GroupWrite::ClearParent { group_id } => Some(Write::UnparentNode {
            node: Ref::Live(group_id),
        }),
        GroupWrite::DeleteNode { group_id } => Some(Write::DeleteNode {
            node: Ref::Live(group_id),
        }),
        _ => None,
    }
}

fn commander_write(w: GroupWrite) -> Option<Write> {
    match w {
        GroupWrite::SetCommander { group, user_id } => Some(Write::CommandNode {
            node: Ref::from(group),
            user_id,
        }),
        GroupWrite::ClearCommander { group_id } => Some(Write::ClearNodeCommander {
            node: Ref::Live(group_id),
        }),
        _ => None,
    }
}

/// A hull's place in the tree. Unplaces come before places, which is the
/// order `groups::plan` promises, so a hull moving between bodies is never in
/// two at once.
fn unit_placement_write(w: GroupWrite) -> Option<Write> {
    match w {
        GroupWrite::UnplaceUnit { unit_id } => Some(Write::UnplaceUnit { unit_id }),
        GroupWrite::PlaceUnit { group, unit_id } => Some(Write::PlaceUnitInNode {
            unit_id,
            node: Ref::from(group),
        }),
        _ => None,
    }
}

/// Whether a previous press already created this book row.
///
/// The book ladder's answers carry whole pages, and each one is adopted into
/// the journal as it lands; a resume re-plans the position from scratch, so
/// without this it would post the scenario, the step and the command again.
fn already_booked(draft: &Draft, write: &BookWrite) -> bool {
    match write {
        BookWrite::Create { local_id, .. } => draft.journal.book(*local_id).is_some(),
        BookWrite::AddStep { step, .. } => match step {
            StepRef::New(local) => draft.journal.step(*local).is_some(),
            StepRef::Live(_) => false,
        },
        BookWrite::AddSubstep { substep, .. } => draft.journal.substep(*substep).is_some(),
        // A delete, a step-level related list and an audience are all
        // idempotent statements about a row that exists; re-sending one is
        // the diff working, not a duplicate.
        _ => false,
    }
}

fn step_write(w: BookWrite) -> Option<Write> {
    match w {
        BookWrite::Create {
            local_id,
            title,
            description,
        } => Some(Write::CreateBook {
            local_id,
            title,
            description,
        }),
        BookWrite::AddStep {
            scenario,
            step,
            content,
            window,
        } => Some(Write::CreateStep {
            book: Ref::from(scenario),
            local_id: step_ref_id(step),
            kind: None,
            content,
            window,
        }),
        BookWrite::RemoveStep {
            scenario_id,
            step_id,
        } => Some(Write::DeleteStep {
            book: Ref::Live(scenario_id),
            step_id,
        }),
        BookWrite::AddSubstep {
            scenario,
            step,
            substep,
            content,
        } => Some(Write::CreateSubstep {
            book: Ref::from(scenario),
            step: Ref::from(step),
            local_id: substep,
            content,
        }),
        BookWrite::SetAudience {
            scenario,
            step,
            substep,
            hulls,
            nodes,
        } => Some(Write::SetAudience {
            book: Ref::from(scenario),
            step: Ref::from(step),
            substep: Ref::Staged(substep),
            hulls,
            nodes,
        }),
        // The step-level audience route answers 409 always, on both ladders:
        // it is a gap, never a write.
        BookWrite::SetRelated { .. } => None,
    }
}

// -- the gaps --------------------------------------------------------------

/// The reasons the ladder cannot be built from this draft, whatever the verb
/// is asked of.
///
/// These are not the draft's blockers (`crate::draft`): a blocker is a rule the
/// server states about the draft, and a gap is a write the LADDER has no honest
/// place for. Both stop the press; the sentence says which is which.
pub fn gaps(draft: &Draft, live: &Live, verb: Verb) -> Vec<String> {
    let mut gaps = Vec::new();
    if verb.ladder() == Ladder::Mission {
        let books = draft.book.staged_news().count();
        if books > 1 {
            gaps.push(format!(
                "A Saved Scenario is one book, and this draft holds {books}. Saving it as one \
                 would drop the rest."
            ));
        }
    } else if game_id(draft, live).is_none() && draft.terms.time.state() == BaseState::Partial {
        gaps.push(format!(
            "The time base is half set, and a window is written whole: {}",
            draft.terms.time.problem().unwrap_or_default()
        ));
    }
    if !draft.book.is_empty() && draft.mode != GameMode::Scenario {
        gaps.push("Steps belong to a scenario exercise, and this draft is not one.".to_string());
    }
    // The force is the game ladder's. The mission ladder writes no units, no
    // commanders and no positions, so its gaps are not its business.
    if verb.ladder() == Ladder::Game {
        if let Err(gap) = force::plan(&draft.force, &live.units) {
            gaps.push(format!("The force cannot be written: {gap}"));
        }
    }
    let audiences = draft.book.relation_count();
    if audiences > 0 {
        gaps.push(format!(
            "A step-level audience is never written: Minos refuses it with a 409, and an audience \
             belongs on a sub-step. {audiences} hull(s) are staged against a step \u{2014} move them \
             onto a sub-step in the composer."
        ));
    }
    // A BEAT CARRIES NO SUB-STEPS, and the server says so with a 409. The
    // composer refuses to author one; a draft that holds one anyway (a seeded
    // read, a hand-built fixture) is a gap rather than a write that fails.
    let mut beat_subs = 0usize;
    for news in draft.book.staged_news() {
        for step in &news.steps {
            if step.kind == crate::book::StepKind::Beat {
                beat_subs += draft.book.substeps_of(step.local_id).len();
            }
        }
    }
    if beat_subs > 0 {
        gaps.push(format!(
            "A beat carries no sub-steps, and this draft holds {beat_subs} on one. The server \
             refuses the write with a 409."
        ));
    }
    gaps
}

// -- the shared reads ------------------------------------------------------

fn tree(draft: &Draft, live: &Live) -> Vec<GroupWrite> {
    groups::plan(&draft.groups, &live.nodes)
}



/// The force's writes. `force::plan` refuses a draft it cannot write at all;
/// that refusal is a gap, raised once by [`gaps`], so here it reads as nothing.
fn force(draft: &Draft, live: &Live) -> Vec<ForceWrite> {
    force::plan(&draft.force, &live.units).unwrap_or_default()
}

/// The game's id: the press's own record first, then the read, then the
/// draft's origin. Never a guess.
pub fn game_id(draft: &Draft, live: &Live) -> Option<i64> {
    draft
        .journal
        .game_id()
        .or(live.game_id)
        .or_else(|| draft.origin.game_id())
}

/// The local id a staged step reference names.
fn step_ref_id(step: StepRef) -> i64 {
    match step {
        StepRef::New(id) | StepRef::Live(id) => id,
    }
}

// ===========================================================================
// The press
// ===========================================================================

/// A read the press owes. Reads are free and constant; only writes are the
/// press's, but two of them are reads the ladder depends on.
#[derive(Debug, Clone, PartialEq)]
pub enum Read {
    /// 08 — the gate. Its verdict is the arrival panel's business; the ladder
    /// records that it answered.
    Readiness { game_id: i64 },
    /// The lost-01 question: every game with this name created in the last
    /// [`LOST_GAME_WINDOW_MINUTES`]. The transport does the filtering, because
    /// only it can read `created_at`.
    GamesNamed { name: String, within_minutes: i64 },
}

/// What the server said.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// A write landed, with the id it made when it made one.
    Done { id: Option<i64> },
    /// A read answered: the ids it listed, in the server's order.
    Found { ids: Vec<i64> },
    /// The server refused with an envelope: `data.errors` for a 4xx, or a
    /// sentence.
    Refused {
        status: u16,
        sentence: String,
        request_id: Option<String>,
    },
    /// The server did not answer — a 502, a timeout, a closed socket. There is
    /// no envelope, so the status and the `X-Request-ID` are all there is.
    NoAnswer {
        status: Option<u16>,
        request_id: Option<String>,
    },
    /// The write was NEVER ISSUED: the transport could not build it — a staged
    /// row whose id is not resolved, a role this game does not have. Nothing
    /// reached the server, so there is no status and no request id, and
    /// nothing to duplicate by pressing again.
    NotSent { sentence: String },
}

/// The seam: one write in, one answer out.
///
/// The real one is [`crate::backend::MasterTransport`]: it maps each write onto
/// one Minos endpoint, reads the game's own roles for a seat, and resolves a
/// staged ref from the journal. Everything below is the contract it satisfies,
/// which is why the engine is testable with a fake and the mapping is testable
/// without a server.
///
/// A TRANSPORT OWNS THREE THINGS THE ENGINE DELIBERATELY DOES NOT:
///
/// 1. **The request bodies**, from [`Write`]'s fields.
/// 2. **The role map.** A [`Write::Seat`] names a role by NAME; the transport
///    reads `GET /games/{id}/roles` and maps it, because a blank draft has no
///    game to read roles from until 01 has landed (§12.3).
/// 3. **The refs.** A [`Write`] may name a node or a book the same press just
///    made (`Ref::Staged`); the journal is where that local id became an id.
///
/// The engine never retries, never reorders and never compensates. If the
/// answer is not `Done`, the press stops there.
pub trait Transport {
    fn send(&mut self, write: &Write, journal: &Journal) -> Answer;
    fn read(&mut self, read: &Read, journal: &Journal) -> Answer;
}

/// Why a position stopped the press.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    /// The status the server gave, when it gave one. `None` for a timeout or a
    /// closed socket.
    pub status: Option<u16>,
    /// The `X-Request-ID` a 5xx carries — the API asks to be quoted that when
    /// a failure is reported.
    pub request_id: Option<String>,
    /// The server's own words, or the engine's sentence for an answer that
    /// carried none.
    pub sentence: String,
}

/// What a press left behind.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Every position the verb writes is settled.
    Done,
    /// The press never started: the draft is not writable yet, and these are
    /// the reasons the button would give.
    Blocked(Vec<Blocker>),
    /// The ladder cannot be built from this draft.
    Unplanned(Vec<String>),
    /// A position stopped the press. Everything before it is written, nothing
    /// is rolled back, and the draft is still here.
    Stopped {
        position: u8,
        name: &'static str,
        failure: Failure,
        /// Where the next press resumes: the same position, re-planned.
        resume_at: u8,
    },
}

/// Press one of the verbs: issue the ladder, one write at a time.
///
/// Call it again to resume. A resume RE-PLANS — so the caller should refresh
/// [`Live`] first, because that is what tells the plan which of a failed
/// position's writes already landed. A position the journal says settled is
/// never issued again.
///
/// `offset` is the frame the window's instants are written in.
pub fn run(
    draft: &mut Draft,
    live: &Live,
    verb: Verb,
    offset: &str,
    transport: &mut impl Transport,
) -> Outcome {
    let blockers = draft.blockers(verb);
    if !blockers.is_empty() {
        return Outcome::Blocked(blockers);
    }
    let gaps = gaps(draft, live, verb);
    if !gaps.is_empty() {
        return Outcome::Unplanned(gaps);
    }
    let ladder = verb.ladder();

    for number in 1..=verb.stops_at() {
        // Built here, not up front: 02 onwards is addressed by the game's id,
        // and 01 is what makes it.
        let position = position(draft, live, verb, number, offset);
        if matches!(
            position.status(&draft.journal, ladder),
            Status::Written | Status::Read
        ) {
            continue;
        }

        if let Some(read) = &position.read {
            match transport.read(read, &draft.journal) {
                Answer::Found { .. } | Answer::Done { .. } => {
                    draft.journal.note_read(ladder, number);
                }
                other => {
                    let failure = failure_of(other);
                    draft.journal.note_failed(
                        ladder,
                        number,
                        failure.status.unwrap_or(0),
                        failure.request_id.clone(),
                    );
                    return stop(&position, failure);
                }
            }
            continue;
        }
        if number == 8 {
            // The gate is a read of a game that must exist by now: reaching
            // this without an id is a plan built from nothing, and saying so
            // is better than settling a position nobody read.
            let failure = Failure {
                status: None,
                request_id: None,
                sentence: "The gate cannot be read before the game exists.".to_string(),
            };
            draft.journal.note_failed(ladder, number, 0, None);
            return stop(&position, failure);
        }

        for write in &position.writes {
            match transport.send(write, &draft.journal) {
                Answer::Done { id } => {
                    if matches!(write, Write::CreateGame(_)) && id.is_none() {
                        let failure = Failure {
                            status: None,
                            request_id: None,
                            sentence: "The server made the game but did not say which one."
                                .to_string(),
                        };
                        draft.journal.note_failed(ladder, number, 0, None);
                        return stop(&position, failure);
                    }
                    record(draft, write, id, ladder);
                }
                Answer::NoAnswer { status, request_id } => {
                    // THE LOST FIRST WRITE. The game may exist anyway, so the
                    // question is not "retry?" but "did it land?".
                    if let Write::CreateGame(body) = write {
                        match recover_game(draft, &body.name, transport) {
                            Ok(Some(id)) => {
                                draft.journal.adopt_game(id);
                                continue;
                            }
                            Ok(None) => {
                                let failure = Failure {
                                    status,
                                    request_id,
                                    sentence: format!(
                                        "The server did not answer, and no game named {:?} was \
                                         created in the last {} minutes \u{2014} pressing again is \
                                         safe.",
                                        body.name, LOST_GAME_WINDOW_MINUTES
                                    ),
                                };
                                draft.journal.note_failed(
                                    ladder,
                                    number,
                                    failure.status.unwrap_or(0),
                                    failure.request_id.clone(),
                                );
                                return stop(&position, failure);
                            }
                            Err(failure) => {
                                draft.journal.note_failed(
                                    ladder,
                                    number,
                                    failure.status.unwrap_or(0),
                                    failure.request_id.clone(),
                                );
                                return stop(&position, failure);
                            }
                        }
                    }
                    let failure = Failure {
                        status,
                        request_id,
                        sentence: "The server did not answer. The first one is on the server. \
                                   Nothing is rolled back, and the draft is still here."
                            .to_string(),
                    };
                    draft.journal.note_failed(
                        ladder,
                        number,
                        failure.status.unwrap_or(0),
                        failure.request_id.clone(),
                    );
                    return stop(&position, failure);
                }
                other => {
                    let failure = failure_of(other);
                    draft.journal.note_failed(
                        ladder,
                        number,
                        failure.status.unwrap_or(0),
                        failure.request_id.clone(),
                    );
                    return stop(&position, failure);
                }
            }
        }
        // A position with no writes at all is settled too: nothing was owed.
        draft.journal.note_written(
            ladder,
            number,
            if number == 1 {
                draft.journal.game_id()
            } else {
                None
            },
        );
    }
    Outcome::Done
}

fn stop(position: &Position, failure: Failure) -> Outcome {
    Outcome::Stopped {
        position: position.number,
        name: position.name,
        failure,
        resume_at: position.number,
    }
}

/// Record what one answered write created, so later writes can name it.
fn record(draft: &mut Draft, write: &Write, id: Option<i64>, ladder: Ladder) {
    if let Write::CreateGame(_) = write {
        draft.journal.set_game_id(id);
        return;
    }
    let Some(id) = id else { return };
    match write.created() {
        Some(Created::Node(local)) => draft.journal.resolve_node(local, id),
        Some(Created::Book(local)) => {
            draft.journal.resolve_book(local, id);
            match ladder {
                Ladder::Game => draft.journal.set_scenario_id(Some(id)),
                Ladder::Mission => draft.journal.set_mission_id(Some(id)),
            }
        }
        Some(Created::Step(local)) => draft.journal.resolve_step(local, id),
        Some(Created::Substep(local)) => draft.journal.resolve_substep(local, id),
        None => {}
    }
}

fn failure_of(answer: Answer) -> Failure {
    match answer {
        Answer::Refused {
            status,
            sentence,
            request_id,
        } => Failure {
            status: Some(status),
            request_id,
            sentence,
        },
        Answer::NoAnswer { status, request_id } => Failure {
            status,
            request_id,
            sentence: "The server did not answer.".to_string(),
        },
        Answer::NotSent { sentence } => Failure {
            status: None,
            request_id: None,
            sentence,
        },
        // A write answered with a list, or a read answered with a write's
        // shape: the transport is confused, and that is worth saying rather
        // than settling a position on it.
        Answer::Done { .. } | Answer::Found { .. } => Failure {
            status: None,
            request_id: None,
            sentence: "The transport answered a write with a list.".to_string(),
        },
    }
}

/// The lost-01 lookup: adopt exactly one recent game of that name, refuse to
/// guess between two.
///
/// `Ok(None)` is "the game was not made, retry is safe"; `Err` is "more than
/// one, and the operator has to look".
fn recover_game(
    draft: &Draft,
    name: &str,
    transport: &mut impl Transport,
) -> Result<Option<i64>, Failure> {
    let read = Read::GamesNamed {
        name: name.to_string(),
        within_minutes: LOST_GAME_WINDOW_MINUTES,
    };
    match transport.read(&read, &draft.journal) {
        Answer::Found { ids } => match ids.as_slice() {
            [] => Ok(None),
            [one] => Ok(Some(*one)),
            many => Err(Failure {
                status: None,
                request_id: None,
                sentence: format!(
                    "The server did not answer, and {} games named {:?} were created in the last \
                     {} minutes ({}). Guessing here would make a duplicate game you cannot see, so \
                     open the list and pick.",
                    many.len(),
                    name,
                    LOST_GAME_WINDOW_MINUTES,
                    many.iter()
                        .map(|id| id.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }),
        },
        other => Err(failure_of(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::GameDetail;
    use crate::draft::{Draft, Origin};
    use crate::force::{DraftHull, Start};
    use crate::groups::GroupKind;
    use crate::roster::StagedSeat;

    /// A transport that answers from a script and remembers what it saw.
    #[derive(Default)]
    struct Fake {
        /// A create's id, by write label.
        ids: Vec<(String, i64)>,
        /// The lost-01 answer, when the create does not answer.
        lost: Option<Vec<i64>>,
        /// Whether `CreateGame` goes unanswered at all.
        lose_create: bool,
        /// Write labels this fake refuses.
        refuse: Vec<String>,
        /// The send call (0-based) this fake refuses, for a ladder-wide sweep.
        fail_on_send: Option<usize>,
        /// Whether the gate read fails.
        fail_read: bool,
        calls: usize,
        seen: Vec<String>,
        reads: Vec<String>,
    }

    impl Fake {
        fn healthy() -> Self {
            Fake::default()
        }

        fn with_id(label: &str, id: i64) -> Self {
            Fake {
                ids: vec![(label.to_string(), id)],
                ..Fake::healthy()
            }
        }

        fn losing_the_create(ids: Vec<i64>) -> Self {
            Fake {
                lose_create: true,
                lost: Some(ids),
                ..Fake::healthy()
            }
        }

        fn refusing(prefix: &str) -> Self {
            Fake {
                refuse: vec![prefix.to_string()],
                ..Fake::healthy()
            }
        }
    }

    impl Transport for Fake {
        fn send(&mut self, write: &Write, _journal: &Journal) -> Answer {
            let label = write.label();
            let call = self.calls;
            self.calls += 1;
            self.seen.push(label.clone());
            if self.fail_on_send == Some(call) {
                return Answer::Refused {
                    status: 502,
                    sentence: "THE SERVER DID NOT ANSWER".into(),
                    request_id: Some(format!("req-{call}")),
                };
            }
            if self.refuse.iter().any(|r| label.starts_with(r.as_str())) {
                return Answer::Refused {
                    status: 502,
                    sentence: "THE SERVER DID NOT ANSWER".into(),
                    request_id: Some("req-refused".into()),
                };
            }
            if self.lose_create && matches!(write, Write::CreateGame(_)) {
                return Answer::NoAnswer {
                    status: Some(502),
                    request_id: Some("req-lost".into()),
                };
            }
            if let Some((_, id)) = self.ids.iter().find(|(l, _)| l == &label) {
                return Answer::Done { id: Some(*id) };
            }
            Answer::Done { id: None }
        }

        fn read(&mut self, read: &Read, _journal: &Journal) -> Answer {
            self.reads.push(match read {
                Read::Readiness { .. } => "readiness".to_string(),
                Read::GamesNamed { .. } => "games-named".to_string(),
            });
            if self.fail_read && matches!(read, Read::Readiness { .. }) {
                return Answer::NoAnswer {
                    status: Some(502),
                    request_id: Some("req-gate".into()),
                };
            }
            match read {
                Read::GamesNamed { .. } => Answer::Found {
                    ids: self.lost.clone().unwrap_or_default(),
                },
                Read::Readiness { .. } => Answer::Found { ids: Vec::new() },
            }
        }
    }

    fn hull(
        unit_id: i64,
        name: &str,
        commander: Option<i64>,
        start: Option<(f64, f64)>,
    ) -> DraftHull {
        let mut h = DraftHull::new(unit_id, name.into(), format!("{unit_id}"), "frigate".into());
        h.commander_id = commander;
        h.start = start.map(|(lat, lon)| Start { lat, lon });
        h
    }

    fn seat(user_id: i64, name: &str, role: &str) -> StagedSeat {
        StagedSeat {
            user_id,
            user_name: name.into(),
            // A TEMPLATE id, which is what the blank draft's picker holds. The
            // write carries the NAME, so this id never reaches the wire.
            role_id: 1,
            role_name: role.into(),
            call_sign: name.to_uppercase(),
            judge: false,
        }
    }

    fn live_hull(
        unit_id: i64,
        name: &str,
        commander: Option<i64>,
        start: Option<(f64, f64)>,
    ) -> LiveHull {
        LiveHull {
            unit_id,
            name: name.into(),
            commander_id: commander,
            start: start.map(|(lat, lon)| Start { lat, lon }),
            host: None,
        }
    }

    /// A draft with a name, one seated commander and one placed hull.
    ///
    /// A scenario draft also has a step, because an empty book is a stage 04
    /// the console refuses to write.
    fn ready_draft(mode: GameMode) -> Draft {
        let mut draft = Draft::blank(mode).unwrap();
        draft.terms.name = "Baltic Shield 26".into();
        draft.roster.stage(seat(4, "Lt. Fitri", "Commando"));
        draft
            .force
            .upsert(hull(351, "KRI AHMAD YANI", Some(4), Some((-3.2, 108.9))));
        if mode.is_scenario() {
            let book = draft.book.stage_scenario("The sweep");
            draft
                .book
                .stage_step_new(book, "TAKE STATION XYZ", None)
                .expect("a step");
        }
        draft
    }

    fn detail(state: &str) -> GameDetail {
        GameDetail {
            id: 42,
            name: "Baltic Shield 26".into(),
            mode: "maneuver".into(),
            state: state.into(),
            description: String::new(),
            purpose: String::new(),
            target: String::new(),
            area: None,
            map_tag: None,
            overlay_text: None,
            window: TimeWindow::default(),
            pace: Some(GamePace::Standard),
            time_factor: 1.0,
            max_echelon_rank: None,
            room_key: None,
        }
    }

    // -- the ladder ------------------------------------------------------

    /// The ladder is the corrected one, in the corrected order (§9.2).
    #[test]
    fn a_game_draft_plans_the_side_of_the_ladder() {
        let draft = ready_draft(GameMode::Scenario);
        let plan = plan(&draft, &Live::default(), Verb::PlayDirectly, "+00:00");
        assert!(plan.gaps.is_empty(), "{:?}", plan.gaps);
        let names: Vec<&str> = plan.positions.iter().map(|p| p.name).collect();
        assert_eq!(
            names,
            vec![
                "GAME",
                "SEATS",
                "HIERARCHY",
                "UNITS",
                "PLACEMENTS",
                "STEPS",
                "COMMANDERS",
                "READINESS",
                "TRANSITION"
            ]
        );
        // Seats precede units, and the tree precedes the hulls that name it.
        assert!(matches!(plan.positions[0].writes[0], Write::CreateGame(_)));
        assert!(matches!(plan.positions[1].writes[0], Write::Seat { .. }));
        assert!(matches!(
            plan.positions[3].writes[0],
            Write::AssignUnit { unit_id: 351, .. }
        ));
        assert!(matches!(
            plan.positions[4].writes[0],
            Write::Place { unit_id: 351, .. }
        ));
        // 08 is a read, and it is the only position that is not a write. It
        // has no game to read before the press, so it draws empty here and is
        // built again once 01 has made one (the press test proves that).
        assert!(plan.positions[7].writes.is_empty());
        assert!(plan.positions[7].read.is_none());
    }

    /// Save as Game Session stops before readiness and never writes 08 or 09.
    #[test]
    fn a_verb_writes_only_up_to_its_own_position() {
        let draft = ready_draft(GameMode::Maneuver);
        let save = plan(&draft, &Live::default(), Verb::SaveAsGameSession, "+00:00");
        assert_eq!(save.positions.len(), 7);
        assert!(save.positions.iter().all(|p| p.number <= 7));

        let play = plan(&draft, &Live::default(), Verb::PlayDirectly, "+00:00");
        assert_eq!(play.positions.len(), 9);
        assert_eq!(
            play.positions[8].writes,
            vec![Write::Transition { to: "preparation" }]
        );
    }

    /// The mission ladder is three positions and writes no game, no seats, no
    /// hierarchy, no placements, no commanders and no transition.
    #[test]
    fn a_saved_scenario_plans_the_mission_ladder() {
        let mut draft = Draft::blank(GameMode::Scenario).unwrap();
        draft.terms.name = "Harbour Sweep".into();
        let book = draft.book.stage_scenario("The sweep");
        draft
            .book
            .stage_step_new(
                book,
                "TAKE STATION XYZ",
                Some(("0800".into(), "0830".into())),
            )
            .expect("a step");
        let plan = plan(&draft, &Live::default(), Verb::SaveAsScenario, "+00:00");
        assert!(plan.gaps.is_empty(), "{:?}", plan.gaps);
        assert_eq!(plan.positions.len(), 3, "M1, M2, M3");
        assert_eq!(plan.positions[0].name, "M1 MISSION");
        assert!(matches!(
            plan.positions[0].writes[0],
            Write::CreateBook { .. }
        ));
        assert_eq!(plan.positions[1].name, "M2 STEPS");
        assert_eq!(plan.positions[1].writes.len(), 1);
        assert!(matches!(
            plan.positions[1].writes[0],
            Write::CreateStep { .. }
        ));
        assert!(plan.positions[2].writes.is_empty(), "no sub-steps yet");
    }

    /// THE SAVED SCENARIO IS NAMED FROM THE GAME, AND THE OPERATOR'S WORDS
    /// WIN. M1 posts the mission's own title and description, not the game's.
    #[test]
    fn the_saved_scenario_writes_its_own_name_and_description() {
        let mut draft = Draft::blank(GameMode::Scenario).unwrap();
        draft.terms.name = "Baltic Shield 26".into();
        draft.terms.description = "the game's description".into();
        let book = draft.book.stage_scenario("The sweep");
        draft
            .book
            .stage_step_new(book, "TAKE STATION XYZ", None)
            .expect("a step");

        let first = plan(&draft, &Live::default(), Verb::SaveAsScenario, "+00:00");
        match &first.positions[0].writes[0] {
            Write::CreateBook {
                title, description, ..
            } => {
                assert_eq!(title, "Baltic Shield 26", "prefilled from the game");
                assert_eq!(description, "the game's description");
            }
            other => panic!("expected the book's create, got {other:?}"),
        }

        draft.set_saved_title("Sweep of the Sound".into());
        draft.set_saved_description("one sweep, two hulls".into());
        let edited = plan(&draft, &Live::default(), Verb::SaveAsScenario, "+00:00");
        match &edited.positions[0].writes[0] {
            Write::CreateBook {
                title, description, ..
            } => {
                assert_eq!(title, "Sweep of the Sound");
                assert_eq!(description, "one sweep, two hulls");
            }
            other => panic!("expected the book's create, got {other:?}"),
        }
    }

    /// COMMIT IS A DIFF: a hull the server already holds is not assigned again,
    /// a placement that matches is not re-sent, and a seat in the same role is
    /// not re-seated.
    #[test]
    fn a_hull_the_server_already_holds_is_not_assigned_again() {
        let mut draft = Draft::blank(GameMode::Maneuver).unwrap();
        draft.terms.name = "Baltic Shield 26".into();
        draft.roster.stage(seat(4, "Lt. Fitri", "Commando"));
        draft
            .force
            .upsert(hull(351, "KRI AHMAD YANI", Some(4), Some((-3.2, 108.9))));
        let mut live = Live::reopened(42);
        live.seats.push(LiveSeat {
            user_id: 4,
            user_name: "Lt. Fitri".into(),
            role_id: 7,
            role_name: "Commando".into(),
            call_sign: "FITRI".into(),
            judge: false,
        });
        live.units.push(live_hull(
            351,
            "KRI AHMAD YANI",
            Some(4),
            Some((-3.2, 108.9)),
        ));
        let plan = plan(&draft, &live, Verb::SaveAsGameSession, "+00:00");
        assert!(plan.gaps.is_empty(), "{:?}", plan.gaps);
        assert!(plan.positions[0].writes.is_empty(), "no game to create");
        assert!(plan.positions[1].writes.is_empty(), "the seat matches");
        assert!(
            plan.positions[3].writes.is_empty(),
            "no assignment: {:?}",
            plan.positions[3].writes
        );
        assert!(
            plan.positions[4].writes.is_empty(),
            "the placement matches: {:?}",
            plan.positions[4].writes
        );
    }

    /// A commander change on a hull the server holds is 07's write, not 04's.
    #[test]
    fn a_handover_on_a_held_hull_lands_at_seven() {
        let mut draft = Draft::blank(GameMode::Maneuver).unwrap();
        draft.terms.name = "Baltic Shield 26".into();
        draft.roster.stage(seat(4, "Lt. Fitri", "Commando"));
        draft.roster.stage(seat(5, "Rina Wijaya", "Commando"));
        draft
            .force
            .upsert(hull(351, "KRI AHMAD YANI", Some(5), None));
        let mut live = Live::reopened(42);
        live.units
            .push(live_hull(351, "KRI AHMAD YANI", Some(4), None));
        let plan = plan(&draft, &live, Verb::SaveAsGameSession, "+00:00");
        assert!(
            plan.positions[3]
                .writes
                .iter()
                .all(|w| !matches!(w, Write::ReassignUnit { .. })),
            "04 does not carry the handover"
        );
        assert_eq!(
            plan.positions[6].writes,
            vec![Write::ReassignUnit {
                unit_id: 351,
                commander_id: 5
            }]
        );
    }

    /// A seat is written by ROLE NAME: the picker's template id is never sent,
    /// because the server refuses an id from the other vocabulary.
    #[test]
    fn a_seat_carries_a_role_name_not_a_template_id() {
        let draft = ready_draft(GameMode::Maneuver);
        let plan = plan(&draft, &Live::default(), Verb::SaveAsGameSession, "+00:00");
        match &plan.positions[1].writes[..] {
            [
                Write::Seat {
                    user_id, role_name, ..
                },
            ] => {
                assert_eq!(*user_id, 4);
                assert_eq!(role_name, "Commando");
            }
            other => panic!("expected one seat, got {other:?}"),
        }
    }

    /// A RESUME DOES NOT POST A PAGE TWICE. A staged book is not in `live`,
    /// so the plan's create would run again after a press made it and failed
    /// later in the same position; the journal's adopted ids are what says it
    /// landed.
    #[test]
    fn a_resume_does_not_create_the_page_the_journal_adopted() {
        let mut draft = ready_draft(GameMode::Scenario);
        let (book, step) = draft
            .book
            .staged_news()
            .find_map(|news| {
                news.steps
                    .first()
                    .map(|step| (news.local_id, step.local_id))
            })
            .expect("the fixture's book and step");
        let _sub = draft
            .book
            .stage_substep(step, "ONE COMMAND")
            .expect("a command");

        let before = plan(&draft, &Live::default(), Verb::SaveAsGameSession, "+00:00");
        let makes = |plan: &Plan, test: fn(&Write) -> bool| {
            plan.positions[5].writes.iter().any(test)
        };
        assert!(makes(&before, |w| matches!(w, Write::CreateBook { .. })));
        assert!(makes(&before, |w| matches!(w, Write::CreateStep { .. })));

        // The press made the book and the step, then stopped on the command.
        draft.journal.resolve_book(book, 55);
        draft.journal.resolve_step(step, 66);
        let after = plan(&draft, &Live::default(), Verb::SaveAsGameSession, "+00:00");
        assert!(
            !makes(&after, |w| matches!(w, Write::CreateBook { .. })),
            "the book is not made again"
        );
        assert!(
            !makes(&after, |w| matches!(w, Write::CreateStep { .. })),
            "nor the step"
        );
        assert!(
            makes(&after, |w| matches!(w, Write::CreateSubstep { .. })),
            "the command still is"
        );
    }

    /// A reopened draft's plan is a diff from the start, so its ladder has no
    /// game to create.
    #[test]
    fn a_reopened_draft_does_not_create_the_game_it_came_from() {
        let draft = Draft::reopened(&detail("planning")).unwrap();
        assert_eq!(draft.origin, Origin::Reopened { game_id: 42 });
        let plan = plan(&draft, &Live::default(), Verb::SaveAsGameSession, "+00:00");
        assert!(plan.positions[0].writes.is_empty());
        assert_eq!(game_id(&draft, &Live::default()), Some(42));
    }

    /// A game in any other state is not reopened at all.
    #[test]
    fn a_game_past_planning_has_no_draft() {
        assert!(Draft::reopened(&detail("execution")).is_none());
    }

    // -- the press -------------------------------------------------------

    /// The press runs the ladder in order and records every position.
    #[test]
    fn a_press_runs_the_ladder_and_settles_every_position() {
        let mut draft = ready_draft(GameMode::Scenario);
        let mut fake = Fake::with_id("create game \"Baltic Shield 26\"", 42);
        let outcome = run(
            &mut draft,
            &Live::default(),
            Verb::PlayDirectly,
            "+00:00",
            &mut fake,
        );
        assert_eq!(outcome, Outcome::Done, "{outcome:?}");
        assert_eq!(draft.journal.game_id(), Some(42));
        assert_eq!(draft.journal.settled(Ladder::Game), 9);
        // The gate was READ, not written.
        assert_eq!(fake.reads, vec!["readiness"]);
        assert_eq!(draft.journal.landing(Ladder::Game, 8), Some(&Landing::Read));
    }

    /// A failure stops the press at its position, and nothing before it is
    /// rolled back. Resuming re-plans and never re-sends what the journal
    /// settled.
    #[test]
    fn a_failure_stops_the_ladder_and_a_resume_skips_what_landed() {
        let mut draft = ready_draft(GameMode::Scenario);
        draft
            .groups
            .stage_group(0, "GUS-1", GroupKind::SatuanTugas, None, None, vec![351])
            .expect("a group");

        let mut first = Fake::refusing("create node");
        first
            .ids
            .push(("create game \"Baltic Shield 26\"".into(), 42));
        let outcome = run(
            &mut draft,
            &Live::default(),
            Verb::SaveAsGameSession,
            "+00:00",
            &mut first,
        );
        match outcome {
            Outcome::Stopped {
                position,
                name,
                resume_at,
                failure,
            } => {
                assert_eq!(position, 3);
                assert_eq!(name, "HIERARCHY");
                assert_eq!(resume_at, 3);
                assert_eq!(failure.status, Some(502));
                assert_eq!(failure.request_id.as_deref(), Some("req-refused"));
            }
            other => panic!("expected a stop at 03, got {other:?}"),
        }
        // 01 and 02 are settled; the draft is still here.
        assert_eq!(draft.journal.settled(Ladder::Game), 2);
        assert!(first.seen.iter().any(|l| l.starts_with("create game")));
        assert!(first.seen.iter().any(|l| l.starts_with("create node")));
        assert!(!first.seen.iter().any(|l| l.starts_with("assign")));

        // A resume: the two settled positions are not issued again, and the
        // node's answer resolves the staged id.
        let mut second = Fake::with_id("create node \"GUS-1\"", 900);
        let outcome = run(
            &mut draft,
            &Live::default(),
            Verb::SaveAsGameSession,
            "+00:00",
            &mut second,
        );
        assert_eq!(outcome, Outcome::Done, "{outcome:?}");
        assert!(
            !second.seen.iter().any(|l| l.starts_with("create game")),
            "01 is not re-sent: {:?}",
            second.seen
        );
        assert!(
            !second.seen.iter().any(|l| l.starts_with("seat")),
            "02 is not re-sent: {:?}",
            second.seen
        );
        assert_eq!(draft.journal.node(-1), Some(900), "the node resolved");
        assert!(
            second
                .seen
                .iter()
                .any(|l| l.starts_with("place unit 351 in node -1"))
        );
    }

    /// A FAILURE AT ANY POSITION IS RESUME-ABLE FROM IT, and every position
    /// below it stays settled: the ladder is one write at a time, and nothing
    /// rolls back.
    #[test]
    fn a_failure_at_any_position_resumes_from_that_position() {
        fn full_draft() -> Draft {
            let mut draft = ready_draft(GameMode::Scenario);
            draft
                .groups
                .stage_group(0, "GUS-1", GroupKind::SatuanTugas, None, Some(4), vec![351])
                .expect("a group");
            let book = draft.book.stage_scenario("The sweep");
            draft
                .book
                .stage_step_new(
                    book,
                    "TAKE STATION XYZ",
                    Some(("0800".into(), "0830".into())),
                )
                .expect("a step");
            draft
        }

        // How many writes each position owes, from the plan itself.
        let counts: Vec<usize> = plan(
            &full_draft(),
            &Live::default(),
            Verb::PlayDirectly,
            "+00:00",
        )
        .positions
        .iter()
        .map(|p| p.writes.len())
        .collect();
        assert_eq!(counts.len(), 9);
        assert!(counts.iter().sum::<usize>() >= 9, "{counts:?}");

        for position in 1..=9u8 {
            let before: usize = counts[..position as usize - 1].iter().sum();
            let writes_here = counts[position as usize - 1];

            let mut draft = full_draft();
            let mut fake = Fake::with_id("create game \"Baltic Shield 26\"", 42);
            if writes_here == 0 {
                if position != 8 {
                    continue; // nothing to fail at
                }
                fake.fail_read = true;
            } else {
                fake.fail_on_send = Some(before);
            }
            let outcome = run(
                &mut draft,
                &Live::default(),
                Verb::PlayDirectly,
                "+00:00",
                &mut fake,
            );
            match &outcome {
                Outcome::Stopped {
                    position: at,
                    resume_at,
                    failure,
                    ..
                } => {
                    assert_eq!(*at, position, "{outcome:?}");
                    assert_eq!(*resume_at, position);
                    assert!(failure.status.is_some(), "{outcome:?}");
                }
                other => panic!("expected a stop at {position}, got {other:?}"),
            }
            assert_eq!(
                draft.journal.settled(Ladder::Game),
                position - 1,
                "everything below {position} is written and nothing above it is"
            );

            // The position that failed is owed again; everything below it is
            // not. So the resume issues the failed position's writes and the
            // ones above it, and starts at the write that failed.
            let failed_label = fake.seen.get(before).cloned();
            let mut again = Fake::with_id("create game \"Baltic Shield 26\"", 42);
            let outcome = run(
                &mut draft,
                &Live::default(),
                Verb::PlayDirectly,
                "+00:00",
                &mut again,
            );
            assert_eq!(outcome, Outcome::Done, "resume at {position}: {outcome:?}");
            assert_eq!(draft.journal.settled(Ladder::Game), 9);
            let owed: usize = counts[position as usize - 1..].iter().sum();
            assert_eq!(
                again.seen.len(),
                owed,
                "position {position}: only it and the positions above it are owed: {:?}",
                again.seen
            );
            if let Some(label) = failed_label {
                assert_eq!(
                    again.seen.first(),
                    Some(&label),
                    "the resume starts at {position}'s failed write"
                );
            }
        }
    }

    /// A lost 01 is not retried blindly: exactly one recent game of that name
    /// is adopted, and the press continues at 02.
    #[test]
    fn a_lost_create_is_adopted_when_exactly_one_game_wears_the_name() {
        let mut draft = ready_draft(GameMode::Maneuver);
        let mut fake = Fake::losing_the_create(vec![77]);
        let outcome = run(
            &mut draft,
            &Live::default(),
            Verb::SaveAsGameSession,
            "+00:00",
            &mut fake,
        );
        assert_eq!(outcome, Outcome::Done, "{outcome:?}");
        assert_eq!(draft.journal.game_id(), Some(77));
        assert_eq!(fake.reads, vec!["games-named"], "the lookup ran");
        assert!(
            matches!(
                draft.journal.landing(Ladder::Game, 1),
                Some(Landing::Written { .. })
            ),
            "01 is journaled from the adoption"
        );
    }

    /// None wearing the name leaves a retry, and the sentence says so.
    #[test]
    fn a_lost_create_with_no_match_offers_a_retry() {
        let mut draft = ready_draft(GameMode::Maneuver);
        let mut fake = Fake::losing_the_create(Vec::new());
        let outcome = run(
            &mut draft,
            &Live::default(),
            Verb::SaveAsGameSession,
            "+00:00",
            &mut fake,
        );
        match outcome {
            Outcome::Stopped {
                position, failure, ..
            } => {
                assert_eq!(position, 1);
                assert!(
                    failure.sentence.contains("pressing again is safe"),
                    "{failure:?}"
                );
            }
            other => panic!("expected a stop at 01, got {other:?}"),
        }
        assert!(matches!(
            draft.journal.landing(Ladder::Game, 1),
            Some(Landing::Failed { status: 502, .. })
        ));
    }

    /// More than one match STOPS and names them: guessing would create a
    /// duplicate the operator cannot see.
    #[test]
    fn a_lost_create_with_two_matches_stops_and_names_them() {
        let mut draft = ready_draft(GameMode::Maneuver);
        let mut fake = Fake::losing_the_create(vec![41, 42]);
        let outcome = run(
            &mut draft,
            &Live::default(),
            Verb::SaveAsGameSession,
            "+00:00",
            &mut fake,
        );
        match outcome {
            Outcome::Stopped {
                position, failure, ..
            } => {
                assert_eq!(position, 1);
                assert!(failure.sentence.contains("41, 42"), "{failure:?}");
                assert!(!fake.seen.iter().any(|l| l.starts_with("seat")));
            }
            other => panic!("expected a stop at 01, got {other:?}"),
        }
        assert_eq!(draft.journal.game_id(), None, "nothing was adopted");
    }

    /// A draft that cannot be written never starts: the press says why, and
    /// the transport sees nothing.
    #[test]
    fn a_blocked_draft_writes_nothing() {
        let mut draft = Draft::blank(GameMode::Maneuver).unwrap();
        let mut fake = Fake::healthy();
        let outcome = run(
            &mut draft,
            &Live::default(),
            Verb::SaveAsGameSession,
            "+00:00",
            &mut fake,
        );
        match outcome {
            Outcome::Blocked(blockers) => {
                assert!(blockers[0].sentence.starts_with("Name is required"));
            }
            other => panic!("expected a block, got {other:?}"),
        }
        assert!(fake.seen.is_empty(), "nothing reached the wire");
    }

    /// A step-level audience is a gap, not a 409 in the middle of a press.
    #[test]
    fn a_step_level_audience_is_refused_before_the_press() {
        let mut draft = Draft::blank(GameMode::Scenario).unwrap();
        draft.terms.name = "North Sea Patrol".into();
        let book = draft.book.stage_scenario("The sweep");
        let step = draft
            .book
            .stage_step_new(book, "TAKE STATION XYZ", None)
            .expect("a step");
        draft
            .book
            .stage_relations(book, step, vec![("hull".into(), 351)]);
        let plan = plan(&draft, &Live::default(), Verb::SaveAsScenario, "+00:00");
        assert!(
            plan.gaps.iter().any(|g| g.contains("step-level audience")),
            "{:?}",
            plan.gaps
        );
        let mut fake = Fake::healthy();
        let outcome = run(
            &mut draft,
            &Live::default(),
            Verb::SaveAsScenario,
            "+00:00",
            &mut fake,
        );
        assert!(matches!(outcome, Outcome::Unplanned(_)), "{outcome:?}");
        assert!(fake.seen.is_empty());
    }

    /// A half-set time base is a gap in the plan as well as a blocker, so a
    /// create is never sent with two typed starts and no length.
    #[test]
    fn a_half_set_base_never_becomes_a_create() {
        let mut draft = ready_draft(GameMode::Maneuver);
        draft.terms.time.actual_date = "2026-11-01".into();
        draft.terms.time.actual_start = "0800".into();
        let plan = plan(&draft, &Live::default(), Verb::SaveAsGameSession, "+00:00");
        assert!(!plan.gaps.is_empty(), "{:?}", plan.gaps);
        assert!(plan.positions[0].writes.is_empty());
    }

    /// The create carries the draft's terms, the rank number, and the ends the
    /// time base derived.
    #[test]
    fn a_blank_drafts_create_carries_the_drafts_terms() {
        let mut draft = ready_draft(GameMode::Scenario);
        draft.ceiling = Some(40);
        draft.pace = GamePace::Fast;
        draft.terms.time.actual_date = "2026-11-01".into();
        draft.terms.time.actual_start = "0800".into();
        draft.terms.time.assumed_start = "0600".into();
        draft.terms.time.factor = "2".into();
        draft.terms.time.length = "1.5".into();
        let plan = plan(&draft, &Live::default(), Verb::SaveAsGameSession, "+07:00");
        let body = match &plan.positions[0].writes[0] {
            Write::CreateGame(body) => body,
            other => panic!("expected a create, got {other:?}"),
        };
        assert_eq!(body.name, "Baltic Shield 26");
        assert_eq!(body.mode, GameMode::Scenario);
        assert_eq!(body.pace, GamePace::Fast);
        assert_eq!(body.max_echelon_rank, Some(40), "a rank number");
        assert_eq!(
            body.window.actual_end.as_deref(),
            Some("2026-11-01T09:30:00+07:00"),
            "the end is derived, never typed"
        );
        assert_eq!(body.time_factor, Some(2.0));
    }

    /// The origin is what a `SavedScenario` press is: a mission, not a game.
    #[test]
    fn a_saved_scenario_press_creates_a_mission_and_journals_it() {
        let mut draft = Draft::saved_scenario(9, "Harbour Sweep", "the sweep");
        // The shell seeds the mission's steps into the book; the press writes
        // them after M1. A mission with no steps is refused before this.
        let book = draft.book.stage_scenario("Harbour Sweep");
        draft
            .book
            .stage_step_new(book, "TAKE STATION XYZ", None)
            .expect("a step");
        let mut fake = Fake::with_id("create book \"Harbour Sweep\"", 500);
        let outcome = run(
            &mut draft,
            &Live::default(),
            Verb::SaveAsScenario,
            "+00:00",
            &mut fake,
        );
        assert_eq!(outcome, Outcome::Done, "{outcome:?}");
        assert_eq!(draft.journal.mission_id(), Some(500));
        assert_eq!(draft.journal.game_id(), None);
        assert_eq!(draft.journal.settled(Ladder::Mission), 3);
        assert_eq!(draft.origin, Origin::SavedScenario { mission_id: 9 });
    }

    /// A staged book's id is recorded, so the steps written after it can name
    /// the book they belong to.
    #[test]
    fn a_created_book_is_resolved_for_the_steps_that_name_it() {
        let mut draft = Draft::blank(GameMode::Scenario).unwrap();
        draft.terms.name = "North Sea Patrol".into();
        draft.roster.stage(seat(4, "Lt. Fitri", "Commando"));
        draft
            .force
            .upsert(hull(351, "KRI AHMAD YANI", Some(4), None));
        let book = draft.book.stage_scenario("The sweep");
        draft
            .book
            .stage_step_new(book, "TAKE STATION XYZ", None)
            .expect("a step");
        let mut fake = Fake::with_id("create book \"The sweep\"", 600);
        fake.ids
            .push(("create game \"North Sea Patrol\"".into(), 42));
        let outcome = run(
            &mut draft,
            &Live::default(),
            Verb::SaveAsGameSession,
            "+00:00",
            &mut fake,
        );
        assert_eq!(outcome, Outcome::Done, "{outcome:?}");
        assert_eq!(draft.journal.book(book), Some(600));
        assert_eq!(draft.journal.scenario_id(), Some(600));
    }

    /// The status column is read from the journal, so a row cannot disagree
    /// with the counter.
    #[test]
    fn the_status_column_is_the_journals() {
        let draft = ready_draft(GameMode::Maneuver);
        let plan = plan(&draft, &Live::default(), Verb::SaveAsGameSession, "+00:00");
        let mut journal = Journal::new();
        journal.note_written(Ladder::Game, 1, Some(42));
        journal.note_failed(Ladder::Game, 2, 502, None);
        assert_eq!(
            plan.positions[0].status(&journal, Ladder::Game),
            Status::Written
        );
        assert_eq!(
            plan.positions[1].status(&journal, Ladder::Game),
            Status::Failed
        );
        assert_eq!(
            plan.positions[2].status(&journal, Ladder::Game),
            Status::Pending
        );
        // And the owed list is the rows still to issue: the failed 02 (its
        // seat is still owed), and 03 to 07.
        let owed = plan.owed(&journal, Ladder::Game);
        assert_eq!(owed.len(), 6, "{:?}", owed);
    }
}

#[cfg(test)]
mod substep_ladder_tests {
    use super::*;

    /// A MISSION LADDER'S M3 IS THE COMMANDS: the sub-steps and their
    /// audiences, in the composer's order. Before this slice that position was
    /// empty and the audience had nowhere to go but a 409.
    #[test]
    fn m3_emits_the_substeps_and_their_audiences() {
        let mut draft = Draft::blank(GameMode::Scenario).expect("a draft");
        let book = draft.book.stage_scenario_named("Harbour sweep", "");
        let step = draft
            .book
            .stage_step_new(book, "SWEEP", Some(("0930".into(), "1015".into())))
            .expect("a step");
        let sub = draft
            .book
            .stage_substep(step, "TAKE STATION XYZ")
            .expect("a sub-step");
        draft.book.set_substep_units(step, sub, vec![351, 352]);

        let plan = plan(&draft, &Live::default(), Verb::SaveAsScenario, "UTC");
        let m3 = plan.positions.iter().find(|p| p.number == 3).expect("M3");
        assert_eq!(m3.name, "M3 COMMANDS");
        let kinds: Vec<&str> = m3
            .writes
            .iter()
            .map(|w| match w {
                Write::CreateSubstep { .. } => "substep",
                Write::SetAudience { .. } => "audience",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["substep", "audience"]);
        assert!(matches!(
            &m3.writes[1],
            Write::SetAudience { hulls, substep: Ref::Staged(_), .. } if hulls == &vec![351, 352]
        ));
    }

    /// THE SAME WRITES RIDE THE GAME LADDER'S 06, because the step position is
    /// where a sub-step's step exists.
    #[test]
    fn the_games_step_position_carries_the_commands() {
        let mut draft = review_draft_for_test();
        let book = draft
            .book
            .staged_news()
            .next()
            .map(|b| b.local_id)
            .expect("a book");
        let step = draft
            .book
            .stage_step_new(book, "SWEEP", Some(("0930".into(), "1015".into())))
            .expect("a step");
        let sub = draft
            .book
            .stage_substep(step, "TAKE STATION")
            .expect("a sub");
        draft.book.set_substep_units(step, sub, vec![351]);
        let plan = plan(&draft, &Live::default(), Verb::SaveAsGameSession, "UTC");
        let steps = plan
            .positions
            .iter()
            .find(|p| p.name == "STEPS")
            .expect("the step position");
        assert!(
            steps
                .writes
                .iter()
                .any(|w| matches!(w, Write::CreateSubstep { .. })),
            "06 creates the sub-step"
        );
        assert!(
            steps
                .writes
                .iter()
                .any(|w| matches!(w, Write::SetAudience { .. })),
            "and its audience"
        );
    }

    /// A BEAT WITH A SUB-STEP IS A GAP, not a write that fails on the wire.
    #[test]
    fn a_beat_holding_a_substep_is_a_gap() {
        let mut draft = Draft::blank(GameMode::Scenario).expect("a draft");
        let book = draft.book.stage_scenario_named("Harbour sweep", "");
        let step = draft
            .book
            .stage_step_new(book, "CONTACT", None)
            .expect("a step");
        let sub = draft
            .book
            .stage_substep(step, "TAKE STATION")
            .expect("a sub");
        draft.book.set_substep_units(step, sub, vec![351]);
        // Turn it into a beat AFTER staging the sub-step — the composer refuses
        // this order, and a seeded read can still hold it.
        draft.book.set_kind(step, crate::book::StepKind::Beat);
        let gaps = gaps(&draft, &Live::default(), Verb::SaveAsScenario);
        assert!(
            gaps.iter()
                .any(|g| g.contains("A beat carries no sub-steps")),
            "the gap names the 409: {gaps:?}"
        );
    }

    fn review_draft_for_test() -> Draft {
        crate::fixtures::review_draft().expect("the review fixture")
    }
}

#[cfg(test)]
mod cluster_write_tests {
    use super::*;

    /// A CLUSTER IS N WRITES, UNIT-ID ORDER, NO BATCH ROUTE (`§5.6`): the
    /// placement position emits one `Place` per hull, sorted by unit id, so a
    /// partial failure leaves the ones before it written and the journal says
    /// which.
    #[test]
    fn a_cluster_is_n_writes_in_unit_id_order() {
        let mut draft = Draft::blank(GameMode::Maneuver).expect("a draft");
        // Staged out of order on purpose.
        for unit_id in [367_i64, 351, 354] {
            draft.force.upsert(crate::force::DraftHull {
                unit_id,
                name: format!("KRI {unit_id}"),
                hull_number: format!("{unit_id}"),
                class_id: "AHMAD YANI".into(),
                start: Some(crate::force::Start {
                    lat: -3.0,
                    lon: 108.0,
                }),
                // A hull with no commander is a gap, and a gap stops the whole
                // force write — so the cluster's order is only observable once
                // the force can be written at all.
                commander_id: Some(unit_id),
                host: None,
                course_deg: None,
            });
        }
        let plan = plan(&draft, &Live::default(), Verb::PlayDirectly, "UTC");
        let placements = plan
            .positions
            .iter()
            .find(|p| p.name == "PLACEMENTS")
            .expect("the placement position");
        let order: Vec<i64> = placements
            .writes
            .iter()
            .filter_map(|w| match w {
                Write::Place { unit_id, .. } => Some(*unit_id),
                _ => None,
            })
            .collect();
        assert_eq!(order, vec![351, 354, 367], "unit-id order, one write each");
    }
}
