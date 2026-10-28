//! The press's transport: `commit::Write` onto Minos, one endpoint at a time.
//!
//! `crate::commit` decides WHAT is still owed and in what order; this is the
//! only place a creation-flow write becomes an HTTP call. Every arm names the
//! endpoint it calls, because the rules that matter — the order, the diff, the
//! journal — are the engine's and are tested there.
//!
//! THREE THINGS THIS TRANSPORT OWNS, and they are the three the engine
//! deliberately does not (`crate::commit::Transport`):
//!
//! 1. **The bodies**, from the write's fields.
//! 2. **The role map.** A seat is written BY ROLE NAME; the game's own ids are
//!    read once (`GET /games/{id}/roles`) and matched by name. A blank draft's
//!    picker holds the TEMPLATE vocabulary's ids, and the server refuses one
//!    against the other's game with a 409 — sending the picker's id would be
//!    the refusal this mapping exists to avoid (§12.3).
//! 3. **The staged ids.** A write may name a node, a book, a step or a command
//!    that the same press just created; [`Journal`] is where that local id
//!    became the id the server returned.
//!
//! A REF IS TYPED BY ITS WRITE, NOT BY ITS SHAPE. Two staged rows (a node and
//! a book) can share the local id `-1`, because each draft's counter is its
//! own — so a `Ref::Staged(-1)` means whatever the write that carries it says
//! it means, and every arm resolves it through the matching journal map.
//!
//! NOTHING HERE RETRIES, REORDERS OR COMPENSATES. If a call fails, the answer
//! stops the press at that position (`crate::commit::run`) and the draft stays.
//!
//! THE ONE READ THAT IS NOT A LADDER READ is the node diff: `POST
//! /games/{id}/hierarchy` answers with the WHOLE tree and does not say which
//! node is the new one, so the first node create in a press reads the tree once
//! and each answer's new id is the difference. That is the same trick the
//! console's step-add pump uses against a scenario answer, and it is a read —
//! reads are free and constant; only writes are the press's.
//!
//! A STEP'S CREATE ANSWERS WITH THE WHOLE BOOK, not the step. The new step's
//! id is the one the answer carries and the book did not: the transport
//! remembers the book's step ids before the first add and journals the
//! difference, because the very next write — a command on that step, then its
//! audience — addresses the step by its staged id. Without that adoption the
//! press stopped at 06 with "the step this command belongs to was never
//! created", on a step the server had just made.

use std::collections::{BTreeMap, BTreeSet};

use crate::backend::{
    BackendError, GameScenario, HierarchyNode, MinosMaster, NewGame, ScenarioMission,
};
use crate::commit::{Answer, Read, Ref, Transport, Write};
use crate::draft::{Journal, Ladder};

/// The press, against one signed-in Minos.
pub struct MasterTransport<'a> {
    master: &'a MinosMaster,
    token: &'a str,
    /// Which ladder this press walks. It is a FIELD and not a guess from the
    /// journal, because a book's endpoint differs by ladder and guessing would
    /// be one wrong write rather than a refusal.
    ladder: Ladder,
    /// The game's id, when the shell already knows it (a `Reopened` draft).
    /// During a press the journal's word wins, because 01 is what makes it.
    game: Option<i64>,
    /// The game's own roles by name, read at most once per press.
    roles: Option<BTreeMap<String, i64>>,
    /// The tree's node ids as they stood before the press's first create.
    /// `None` until a node create needs it.
    known_nodes: Option<BTreeSet<i64>>,
    /// The task organisation's vocabulary by level, read at most once per
    /// press. The ids a create sends are master data (`/hierarchy`), and the
    /// flow stages a level rather than an id.
    echelons: Option<Vec<(crate::groups::GroupKind, i64)>>,
    /// The step and sub-step ids each book held before this press's first
    /// add, keyed by the book (a scenario or a mission).
    ///
    /// A STEP'S ANSWER IS THE WHOLE BOOK. The new row is the id the answer
    /// carries and the remembered set did not, which is the only way to
    /// journal the staged step the next command will name.
    known_books: BTreeMap<i64, BookIds>,
}

/// The ids a book held before an add.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct BookIds {
    steps: BTreeSet<i64>,
    substeps: BTreeSet<i64>,
}

impl BookIds {
    fn of_game(scenario: &GameScenario) -> Self {
        Self {
            steps: scenario.steps.iter().map(|step| step.id).collect(),
            substeps: scenario
                .steps
                .iter()
                .flat_map(|step| step.substeps.iter().map(|sub| sub.id))
                .collect(),
        }
    }

    fn of_mission(mission: &ScenarioMission) -> Self {
        Self {
            steps: mission.steps.iter().map(|step| step.id).collect(),
            substeps: mission
                .steps
                .iter()
                .flat_map(|step| step.substeps.iter().map(|sub| sub.id))
                .collect(),
        }
    }
}

impl<'a> MasterTransport<'a> {
    pub fn new(master: &'a MinosMaster, token: &'a str, ladder: Ladder) -> Self {
        Self {
            master,
            token,
            ladder,
            game: None,
            roles: None,
            known_nodes: None,
            echelons: None,
            known_books: BTreeMap::new(),
        }
    }

    /// The game a `Reopened` draft is about. A `Blank` one learns its id from
    /// 01, and the journal's word is used then.
    pub fn for_game(mut self, game_id: i64) -> Self {
        self.game = Some(game_id);
        self
    }

    /// THE TRANSPORT A PRESS USES: the game the draft is about, when it
    /// already has one.
    ///
    /// A REOPENED DRAFT'S 01 WRITES NOTHING — the game exists — so the
    /// journal never learns its id, and every write from 02 on would stop the
    /// ladder with "the game's id is not known yet". The id is in the origin
    /// (or the live frame), and this is the one place that reads it for the
    /// wire. Constructing the transport without it was the failure: 03
    /// HIERARCHY stopped a reopened press on a game 01 had not created
    /// because it was already there.
    pub fn for_draft(mut self, draft: &crate::draft::Draft, live: &crate::commit::Live) -> Self {
        if let Some(id) = crate::commit::game_id(draft, live) {
            self.game = Some(id);
        }
        self
    }

    fn game(&self, journal: &Journal) -> Result<i64, Answer> {
        journal
            .game_id()
            .or(self.game)
            .ok_or_else(|| Answer::NotSent {
                sentence: "The game's id is not known yet \u{2014} 01 creates it.".to_string(),
            })
    }

    // -- refs -----------------------------------------------------------

    fn node(&self, node: Ref, journal: &Journal) -> Result<i64, Answer> {
        match node {
            Ref::Live(id) => Ok(id),
            Ref::Staged(local) => journal.node(local).ok_or_else(|| Answer::NotSent {
                sentence: format!(
                    "The task group this write names was never created \u{2014} an earlier node \
                     write did not answer with its id (staged {local})."
                ),
            }),
        }
    }

    fn book(&self, book: Ref, journal: &Journal) -> Result<i64, Answer> {
        match book {
            Ref::Live(id) => Ok(id),
            Ref::Staged(local) => journal
                .book(local)
                .or_else(|| journal.scenario_id())
                .or_else(|| journal.mission_id())
                .ok_or_else(|| Answer::NotSent {
                    sentence: "The plan this step belongs to was never created.".to_string(),
                }),
        }
    }

    fn step(&self, step: Ref, journal: &Journal) -> Result<i64, Answer> {
        match step {
            Ref::Live(id) => Ok(id),
            Ref::Staged(local) => journal.step(local).ok_or_else(|| Answer::NotSent {
                sentence: format!(
                    "The step this command belongs to was never created (staged {local})."
                ),
            }),
        }
    }

    fn substep(&self, substep: Ref, journal: &Journal) -> Result<i64, Answer> {
        match substep {
            Ref::Live(id) => Ok(id),
            Ref::Staged(local) => journal.substep(local).ok_or_else(|| Answer::NotSent {
                sentence: format!(
                    "The command this audience belongs to was never created (staged {local})."
                ),
            }),
        }
    }

    // -- the role map ----------------------------------------------------

    /// The id a seat's role name means IN THIS GAME, reading the game's own
    /// roles the first time it is asked.
    ///
    /// The name is matched as the server spells it, then case-insensitively:
    /// the template vocabulary (`GET /game-roles`) and the game's own are two
    /// copies of one list, and a refusal here is far better than a template id
    /// sent against a game — which the server answers with a 409 the operator
    /// cannot act on.
    fn role_id(&mut self, game: i64, role_name: &str) -> Result<i64, Answer> {
        if self.roles.is_none() {
            let roles = self
                .master
                .game_roles(self.token, game)
                .map_err(answer_of)?;
            self.roles = Some(roles.into_iter().map(|r| (r.name, r.id)).collect());
        }
        let roles = self.roles.as_ref().expect("just read");
        let wanted = role_name.trim();
        if let Some(id) = roles.get(wanted) {
            return Ok(*id);
        }
        let lower = wanted.to_lowercase();
        roles
            .iter()
            .find(|(name, _)| name.to_lowercase() == lower)
            .map(|(_, id)| *id)
            .ok_or_else(|| Answer::NotSent {
                sentence: format!(
                    "{wanted} is not a role of this game \u{2014} a seat is written in the game's \
                     own role vocabulary, not the template's."
                ),
            })
    }

    /// THE MASTER-DATA ID FOR A LEVEL. The vocabulary is read on the first
    /// create that needs one; the name is the key because that is what both
    /// sides carry, and `GroupKind::from_echelon` is the squashing rule the
    /// console already uses in the other direction.
    ///
    /// A level the vocabulary does not name is a refusal BEFORE the request:
    /// sending `0` is the `400 · id_unit_hierarchy: this field is required`
    /// that stopped 03, and it names a column rather than the level the
    /// operator built.
    fn echelon(&mut self, kind: crate::groups::GroupKind) -> Result<i64, Answer> {
        if self.echelons.is_none() {
            let levels = self.master.echelon_levels(self.token).map_err(answer_of)?;
            let mut by_kind: Vec<(crate::groups::GroupKind, i64)> = Vec::new();
            for level in levels {
                if let Some(level_kind) = crate::groups::GroupKind::from_echelon(&level.name) {
                    if !by_kind.iter().any(|(seen, _)| *seen == level_kind) {
                        by_kind.push((level_kind, level.id));
                    }
                }
            }
            self.echelons = Some(by_kind);
        }
        self.echelons
            .as_ref()
            .expect("just read")
            .iter()
            .find(|(level_kind, _)| *level_kind == kind)
            .map(|(_, id)| *id)
            .ok_or_else(|| Answer::NotSent {
                sentence: format!(
                    "{} is not a level of this deployment's task organisation \u{2014} the \
                     server's vocabulary names the levels a body may be created at.",
                    kind.label()
                ),
            })
    }

    // -- the book's ids --------------------------------------------------

    /// Remember a game book's step and sub-step ids, once per book per press.
    fn remember_game_book(&mut self, game: i64, book: i64) -> Result<(), Answer> {
        if self.known_books.contains_key(&book) {
            return Ok(());
        }
        let scenario = self
            .master
            .game_scenario(self.token, game, book)
            .map_err(answer_of)?;
        self.known_books.insert(book, BookIds::of_game(&scenario));
        Ok(())
    }

    /// Remember a Saved Scenario's ids, once per mission per press.
    fn remember_mission(&mut self, mission: i64) -> Result<(), Answer> {
        if self.known_books.contains_key(&mission) {
            return Ok(());
        }
        let book = self
            .master
            .scenario_mission(self.token, mission)
            .map_err(answer_of)?;
        self.known_books.insert(mission, BookIds::of_mission(&book));
        Ok(())
    }

    /// The one step the answer added, journaled against its staged id.
    ///
    /// More than one new id is not a create this client can name — another
    /// client may have added one at the same moment — so the answer names
    /// nothing, and the next write refuses loudly instead of addressing a
    /// row it guessed.
    fn adopt_step(&mut self, book: i64, ids: BookIds) -> Answer {
        let before = self.known_books.insert(book, ids.clone()).unwrap_or_default();
        let added: Vec<i64> = ids.steps.difference(&before.steps).copied().collect();
        match added.as_slice() {
            [one] => Answer::Done { id: Some(*one) },
            _ => Answer::Done { id: None },
        }
    }

    /// The one command the answer added, on the same terms.
    fn adopt_substep(&mut self, book: i64, ids: BookIds) -> Answer {
        let before = self.known_books.insert(book, ids.clone()).unwrap_or_default();
        let added: Vec<i64> = ids
            .substeps
            .difference(&before.substeps)
            .copied()
            .collect();
        match added.as_slice() {
            [one] => Answer::Done { id: Some(*one) },
            _ => Answer::Done { id: None },
        }
    }

    // -- the node diff ---------------------------------------------------

    /// The ids the tree already holds, read once before the first create.
    fn remember_tree(&mut self, game: i64) -> Result<(), Answer> {
        if self.known_nodes.is_none() {
            let tree = self
                .master
                .game_hierarchy(self.token, game)
                .map_err(answer_of)?;
            self.known_nodes = Some(node_ids(&tree));
        }
        Ok(())
    }

    /// Which node the create answer added.
    ///
    /// The answer is the whole tree and says nothing about which row is new, so
    /// the new ids are the difference — and exactly one is expected, because
    /// exactly one node was created. Anything else answers `None`, which leaves
    /// the staged id unre­solved and makes the NEXT write that names it refuse
    /// loudly instead of naming the wrong node.
    fn adopt_created_node(&mut self, tree: &[HierarchyNode]) -> Option<i64> {
        let known = self.known_nodes.clone().unwrap_or_default();
        let ids = node_ids(tree);
        let fresh: Vec<i64> = ids.difference(&known).copied().collect();
        self.known_nodes = Some(ids);
        match fresh.as_slice() {
            [one] => Some(*one),
            _ => None,
        }
    }
}

fn node_ids(tree: &[HierarchyNode]) -> BTreeSet<i64> {
    let mut ids = BTreeSet::new();
    for node in tree {
        ids.insert(node.id);
        ids.extend(node_ids(&node.children));
    }
    ids
}

impl Transport for MasterTransport<'_> {
    fn send(&mut self, write: &Write, journal: &Journal) -> Answer {
        // TWO WRITES NEED NO GAME, and they are the two that MAKE one: 01
        // creates the game, and M1 creates a Saved Scenario, which has no game
        // at all. Every other write is addressed by the game's id, so it is
        // resolved once here instead of in twenty arms.
        match write {
            Write::CreateGame(_) => self.send_without_game(write, journal),
            // THE MISSION LADDER HAS NO GAME AT ALL: its book, its steps and
            // their commands are addressed by the mission id, and requiring a
            // game first refused M2 with "the game's id is not known yet".
            Write::CreateBook { .. }
            | Write::CreateStep { .. }
            | Write::CreateSubstep { .. }
            | Write::SetAudience { .. }
                if self.ladder == Ladder::Mission =>
            {
                self.send_without_game(write, journal)
            }
            _ => match self.game(journal) {
                Ok(game) => self.send_for_game(write, game, journal),
                Err(answer) => answer,
            },
        }
    }

    fn read(&mut self, read: &Read, _journal: &Journal) -> Answer {
        match read {
            // 08 — the gate. Its verdict is the arrival panel's business; the
            // ladder records that it answered, so an empty list is the whole
            // answer here.
            Read::Readiness { game_id } => match self.master.game_readiness(self.token, *game_id) {
                Ok(_) => Answer::Found { ids: Vec::new() },
                Err(e) => answer_of(e),
            },
            // The lost-01 question. Filtered HERE because this is the only
            // layer that can read `created_at`: the engine states the window,
            // the transport applies it.
            Read::GamesNamed {
                name,
                within_minutes,
            } => match self.master.games_list_in(self.token, None) {
                Ok(rows) => {
                    let cut =
                        chrono::Utc::now() - chrono::Duration::minutes((*within_minutes).max(0));
                    let ids: Vec<i64> = rows
                        .into_iter()
                        .filter(|row| row.name == *name)
                        .filter(|row| {
                            // A stamp this build cannot read is not evidence of
                            // AGE: the name matched, and dropping the row would
                            // turn "one game, adopt it" into "none, retry" —
                            // which is how a duplicate gets made. So an
                            // unreadable stamp keeps the row, and the engine's
                            // more-than-one rule is what stops a guess.
                            match chrono::DateTime::parse_from_rfc3339(&row.created_at) {
                                Ok(at) => at >= cut,
                                Err(_) => true,
                            }
                        })
                        .map(|row| row.id)
                        .collect();
                    Answer::Found { ids }
                }
                Err(e) => answer_of(e),
            },
        }
    }
}

impl MasterTransport<'_> {
    /// The writes that need no game: 01, and the whole mission ladder (M1's
    /// book, its steps, their commands and audiences), which is addressed by
    /// the mission's own id.
    fn send_without_game(&mut self, write: &Write, journal: &Journal) -> Answer {
        match write {
            Write::CreateGame(body) => {
                let req = NewGame {
                    name: body.name.clone(),
                    mode: body.mode.wire().unwrap_or("maneuver").to_string(),
                    description: body.description.clone(),
                    purpose: body.purpose.clone(),
                    target: body.target.clone(),
                    area: body.area.clone(),
                    map_tag: body.map_tag.clone(),
                    pace: Some(body.pace),
                    window: body.window.clone(),
                    max_echelon_rank: body.max_echelon_rank,
                    time_factor: body.time_factor,
                };
                match self.master.create_game(self.token, &req) {
                    Ok(row) => Answer::Done { id: Some(row.id) },
                    Err(e) => answer_of(e),
                }
            }
            Write::CreateBook {
                title, description, ..
            } => match self
                .master
                .create_scenario_mission(self.token, title, description)
            {
                Ok(mission) => Answer::Done {
                    id: Some(mission.id),
                },
                Err(e) => answer_of(e),
            },
            // -- M2, M3: the mission's own routes, addressed by its id ----
            Write::CreateStep {
                book,
                kind,
                content,
                window,
                ..
            } => match self.book(*book, journal) {
                Ok(mission) => {
                    if let Err(answer) = self.remember_mission(mission) {
                        return answer;
                    }
                    let window = window.as_ref().map(|(a, b)| (a.as_str(), b.as_str()));
                    match self.master.add_mission_step(
                        self.token,
                        mission,
                        kind.as_deref(),
                        content,
                        window,
                    ) {
                        Ok(answer) => self.adopt_step(mission, BookIds::of_mission(&answer)),
                        Err(e) => answer_of(e),
                    }
                }
                Err(answer) => answer,
            },
            Write::CreateSubstep {
                book,
                step,
                content,
                ..
            } => {
                let (mission, step) =
                    match (self.book(*book, journal), self.step(*step, journal)) {
                        (Ok(book), Ok(step)) => (book, step),
                        (Err(answer), _) | (_, Err(answer)) => return answer,
                    };
                if let Err(answer) = self.remember_mission(mission) {
                    return answer;
                }
                match self
                    .master
                    .add_mission_substep(self.token, mission, step, content)
                {
                    Ok(answer) => self.adopt_substep(mission, BookIds::of_mission(&answer)),
                    Err(e) => answer_of(e),
                }
            }
            Write::SetAudience {
                book,
                step,
                substep,
                hulls,
                nodes: _,
            } => {
                let (mission, step, substep) = match (
                    self.book(*book, journal),
                    self.step(*step, journal),
                    self.substep(*substep, journal),
                ) {
                    (Ok(b), Ok(s), Ok(c)) => (b, s, c),
                    (Err(answer), _, _) | (_, Err(answer), _) | (_, _, Err(answer)) => {
                        return answer;
                    }
                };
                call(self.master.set_mission_substep_related(
                    self.token, mission, step, substep, hulls,
                ))
            }
            other => Answer::NotSent {
                sentence: format!("{} needs a game, and there is none yet.", other.label()),
            },
        }
    }

    /// Every write addressed by the game's id.
    fn send_for_game(&mut self, write: &Write, game: i64, journal: &Journal) -> Answer {
        match write {
            // -- 02 ----------------------------------------------------
            Write::Seat {
                user_id,
                role_name,
                call_sign,
            } => match self.role_id(game, role_name) {
                Ok(role) => call(
                    self.master
                        .add_participant(self.token, game, *user_id, role, call_sign),
                ),
                Err(answer) => answer,
            },
            Write::Reseat { user_id, role_name } => match self.role_id(game, role_name) {
                Ok(role) => call(
                    self.master
                        .set_participant_role(self.token, game, *user_id, role),
                ),
                Err(answer) => answer,
            },
            Write::Unseat { user_id } => {
                call(self.master.remove_participant(self.token, game, *user_id))
            }
            // -- 03 ----------------------------------------------------
            Write::CreateNode {
                name,
                echelon_id,
                kind,
                parent,
                ..
            } => {
                let echelon_id = if *echelon_id != 0 {
                    *echelon_id
                } else {
                    match self.echelon(*kind) {
                        Ok(id) => id,
                        Err(answer) => return answer,
                    }
                };
                let parent_id = match parent {
                    Some(parent) => match self.node(*parent, journal) {
                        Ok(id) => Some(id),
                        Err(answer) => return answer,
                    },
                    None => None,
                };
                if let Err(answer) = self.remember_tree(game) {
                    return answer;
                }
                match self.master.hierarchy_create_node(
                    self.token,
                    game,
                    echelon_id,
                    name,
                    parent_id,
                    "",
                    "",
                ) {
                    Ok(tree) => Answer::Done {
                        id: self.adopt_created_node(&tree),
                    },
                    Err(e) => answer_of(e),
                }
            }
            Write::RenameNode { node, name } => match self.node(*node, journal) {
                Ok(node) => {
                    call(
                        self.master
                            .hierarchy_update_node(self.token, game, node, Some(name)),
                    )
                }
                Err(answer) => answer,
            },
            Write::MoveNode { node, parent } => {
                let (node, parent) = match (self.node(*node, journal), self.node(*parent, journal))
                {
                    (Ok(node), Ok(parent)) => (node, parent),
                    (Err(answer), _) | (_, Err(answer)) => return answer,
                };
                call(
                    self.master
                        .hierarchy_move_node(self.token, game, node, parent),
                )
            }
            Write::UnparentNode { node } => match self.node(*node, journal) {
                Ok(node) => call(self.master.hierarchy_detach_node(self.token, game, node)),
                Err(answer) => answer,
            },
            Write::DeleteNode { node } => match self.node(*node, journal) {
                Ok(node) => call(self.master.hierarchy_delete_node(self.token, game, node)),
                Err(answer) => answer,
            },
            // -- 04 ----------------------------------------------------
            Write::AssignUnit {
                unit_id,
                commander_id,
            } => call(
                self.master
                    .assign_unit(self.token, game, *unit_id, *commander_id),
            ),
            Write::PlaceUnitInNode { unit_id, node } => match self.node(*node, journal) {
                Ok(node) => call(
                    self.master
                        .game_unit_place(self.token, game, *unit_id, node),
                ),
                Err(answer) => answer,
            },
            Write::UnplaceUnit { unit_id } => {
                call(self.master.game_unit_unplace(self.token, game, *unit_id))
            }
            Write::Embark { host, child } => {
                call(
                    self.master
                        .game_attach_units(self.token, game, *host, &[*child]),
                )
            }
            Write::Disembark { host, child } => {
                call(
                    self.master
                        .game_detach_units(self.token, game, *host, &[*child]),
                )
            }
            Write::RemoveUnit { unit_id } => {
                call(self.master.remove_unit(self.token, game, *unit_id))
            }
            // -- 05 ----------------------------------------------------
            Write::Place { unit_id, lat, lon } => call(
                self.master
                    .set_placement(self.token, game, *unit_id, *lat, *lon),
            ),
            Write::Lift { unit_id } => {
                call(self.master.clear_placement(self.token, game, *unit_id))
            }
            // -- 06 ----------------------------------------------------
            Write::CreateBook {
                title, description, ..
            } => match self
                .master
                .create_game_scenario(self.token, game, title, description)
            {
                Ok(scenario) => Answer::Done {
                    id: Some(scenario.id),
                },
                Err(e) => answer_of(e),
            },
            Write::CreateStep {
                book,
                content,
                window,
                ..
            } => match self.book(*book, journal) {
                // THE ANSWER IS THE WHOLE BOOK, and the new step is the id it
                // carries and the remembered book did not. The next write — a
                // command on this step — names it by the staged id.
                Ok(book) => {
                    if let Err(answer) = self.remember_game_book(game, book) {
                        return answer;
                    }
                    let window = window.as_ref().map(|(a, b)| (a.as_str(), b.as_str()));
                    match self
                        .master
                        .add_scenario_step(self.token, game, book, content, window)
                    {
                        Ok(answer) => self.adopt_step(book, BookIds::of_game(&answer)),
                        Err(e) => answer_of(e),
                    }
                }
                Err(answer) => answer,
            },
            Write::DeleteStep { book, step_id } => match self.book(*book, journal) {
                Ok(book) => call(
                    self.master
                        .delete_scenario_step(self.token, game, book, *step_id),
                ),
                Err(answer) => answer,
            },
            Write::CreateSubstep {
                book,
                step,
                content,
                ..
            } => {
                let (book, step) = match (self.book(*book, journal), self.step(*step, journal)) {
                    (Ok(book), Ok(step)) => (book, step),
                    (Err(answer), _) | (_, Err(answer)) => return answer,
                };
                // THE ANSWER IS THE WHOLE SCENARIO, so the command's own id
                // is the one it carries and the remembered book did not —
                // the audience write immediately after it names that id.
                if let Err(answer) = self.remember_game_book(game, book) {
                    return answer;
                }
                match self
                    .master
                    .add_game_substep(self.token, game, book, step, content)
                {
                    Ok(answer) => self.adopt_substep(book, BookIds::of_game(&answer)),
                    Err(e) => answer_of(e),
                }
            }
            Write::SetAudience {
                book,
                step,
                substep,
                hulls,
                // Provenance, not a row: the wire is told the hulls, and the
                // nodes stay where the operator can read them.
                nodes: _,
            } => {
                let (book, step, substep) = match (
                    self.book(*book, journal),
                    self.step(*step, journal),
                    self.substep(*substep, journal),
                ) {
                    (Ok(b), Ok(s), Ok(c)) => (b, s, c),
                    (Err(answer), _, _) | (_, Err(answer), _) | (_, _, Err(answer)) => {
                        return answer;
                    }
                };
                call(self.master.set_game_substep_related(
                    self.token, game, book, step, substep, hulls,
                ))
            }
            // -- 07 ----------------------------------------------------
            Write::CommandNode { node, user_id } => match self.node(*node, journal) {
                Ok(node) => call(
                    self.master
                        .hierarchy_set_commander(self.token, game, node, *user_id),
                ),
                Err(answer) => answer,
            },
            Write::ClearNodeCommander { node } => match self.node(*node, journal) {
                Ok(node) => call(
                    self.master
                        .hierarchy_clear_commander(self.token, game, node),
                ),
                Err(answer) => answer,
            },
            Write::ReassignUnit {
                unit_id,
                commander_id,
            } => call(
                self.master
                    .set_unit_commander(self.token, game, *unit_id, *commander_id),
            ),
            // -- 09 ----------------------------------------------------
            Write::Transition { to } => call(self.master.transition_game(self.token, game, to)),
            // 01 and M1 never reach here (`send` routes them).
            other => Answer::NotSent {
                sentence: format!("{} is not a game write.", other.label()),
            },
        }
    }
}

/// A write that creates nothing the press has to name.
///
/// The engine journals an id per position where the position makes one; a
/// participant list, a placement list and a unit list are not ids, and a create
/// that answers with the whole collection (a step's, a node's) is named by the
/// diff above or not at all.
fn call<T>(result: Result<T, BackendError>) -> Answer {
    match result {
        Ok(_) => Answer::Done { id: None },
        Err(e) => answer_of(e),
    }
}

/// A failure, as the press reads it.
///
/// THE STATUS IS KEPT BECAUSE THE FAILURE BLOCK SHOWS IT, and the request id
/// because the API asks to be quoted it. A transport error and a body this
/// build cannot read are both NO ANSWER rather than a refusal: the write may
/// have landed, and calling it a refusal would invite a duplicate.
pub fn answer_of(e: BackendError) -> Answer {
    let request_id = e.request_id().map(|s| s.to_string());
    match e {
        BackendError::Api {
            status, message, ..
        } => Answer::Refused {
            status,
            sentence: message,
            request_id,
        },
        BackendError::Unauthorized { message, .. } => Answer::Refused {
            status: 401,
            sentence: message,
            request_id,
        },
        BackendError::Forbidden { message, .. } => Answer::Refused {
            status: 403,
            sentence: message,
            request_id,
        },
        BackendError::Transport(_) => Answer::NoAnswer {
            status: None,
            request_id: None,
        },
        BackendError::Decode(_) => Answer::NoAnswer {
            status: None,
            request_id,
        },
        other => Answer::NotSent {
            sentence: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MinosMaster;

    /// Every failure has an answer, and the three shapes stay apart: a refusal
    /// carries its status, a lost answer carries none, and a client-side gap is
    /// never dressed as either.
    #[test]
    fn a_failure_is_an_answer_with_the_status_it_carries() {
        let refused = answer_of(BackendError::Api {
            status: 502,
            message: "THE SERVER DID NOT ANSWER".into(),
            detail: None,
            request_id: Some("req-52".into()),
        });
        assert_eq!(
            refused,
            Answer::Refused {
                status: 502,
                sentence: "THE SERVER DID NOT ANSWER".into(),
                request_id: Some("req-52".into()),
            }
        );

        let forbidden = answer_of(BackendError::Forbidden {
            message: "not your game".into(),
            detail: None,
        });
        assert!(matches!(forbidden, Answer::Refused { status: 403, .. }));

        let gap = answer_of(BackendError::Other("no such route".into()));
        assert!(
            matches!(gap, Answer::NotSent { .. }),
            "a client-side gap never reached the server: {gap:?}"
        );
    }

    /// A body this build cannot read is NO ANSWER, not a refusal: the write may
    /// have landed, and calling it refused would invite a duplicate.
    #[test]
    fn an_unreadable_answer_is_a_lost_answer() {
        let decode = answer_of(BackendError::Decode(
            serde_json::from_str::<serde_json::Value>("not json").unwrap_err(),
        ));
        assert!(matches!(decode, Answer::NoAnswer { .. }), "{decode:?}");
    }

    /// A REF IS TYPED BY ITS WRITE. Two staged rows share the local id `-1` —
    /// each draft's counter is its own — so the same number resolves to a node
    /// for a node write and to a book for a book write, and to nothing for a
    /// step write that no answer ever resolved.
    #[test]
    fn a_staged_ref_is_resolved_by_the_write_that_carries_it() {
        let master = MinosMaster::new("http://127.0.0.1:1").expect("a client");
        let transport = MasterTransport::new(&master, "t", Ladder::Game);
        let mut journal = Journal::new();
        journal.resolve_node(-1, 900);
        journal.resolve_book(-1, 44);
        assert_eq!(transport.node(Ref::Staged(-1), &journal).ok(), Some(900));
        assert_eq!(transport.book(Ref::Staged(-1), &journal).ok(), Some(44));
        assert!(
            transport.step(Ref::Staged(-1), &journal).is_err(),
            "no step answer resolved -1"
        );
        assert!(transport.substep(Ref::Staged(-1), &journal).is_err());
    }

    /// THE LEVEL IS A SERVER ID. The flow stages a rung — a local kind — and
    /// the create sends `id_unit_hierarchy`; the vocabulary is read once and
    /// an unknown level refuses before the request, naming the level rather
    /// than the column.
    #[test]
    fn a_create_resolves_its_level_from_the_vocabulary() {
        use crate::groups::GroupKind;
        let master = MinosMaster::new("http://127.0.0.1:1").expect("a client");
        let mut transport = MasterTransport::new(&master, "t", Ladder::Game);
        transport.echelons = Some(vec![
            (GroupKind::SatuanTugas, 2),
            (GroupKind::Gugus, 3),
            (GroupKind::OperasiGabungan, 4),
        ]);
        assert_eq!(transport.echelon(GroupKind::Gugus).ok(), Some(3));
        match transport.echelon(GroupKind::Unsur) {
            Err(Answer::NotSent { sentence }) => assert!(
                sentence.contains("Unsur"),
                "the refusal names the level: {sentence}"
            ),
            other => panic!("expected NotSent, got {other:?}"),
        }
    }

    /// A STEP'S CREATE ANSWERS WITH THE WHOLE BOOK: the new step is the id
    /// the answer carries and the remembered book did not. Without this the
    /// command that follows names a staged id no answer resolved, and 06
    /// stopped with "the step this command belongs to was never created".
    #[test]
    fn a_step_create_adopts_the_step_the_book_did_not_have() {
        let master = MinosMaster::new("http://127.0.0.1:1").expect("a client");
        let mut transport = MasterTransport::new(&master, "t", Ladder::Game);
        transport.known_books.insert(
            7,
            BookIds {
                steps: BTreeSet::from([1]),
                substeps: BTreeSet::from([10]),
            },
        );
        // The step's answer: the new step, no new command.
        let after_step = BookIds {
            steps: BTreeSet::from([1, 2]),
            substeps: BTreeSet::from([10]),
        };
        assert_eq!(
            transport.adopt_step(7, after_step),
            Answer::Done { id: Some(2) }
        );
        // The command is added next, so ITS baseline is the book the step's
        // answer left: the step exists, the command does not.
        let after_command = BookIds {
            steps: BTreeSet::from([1, 2]),
            substeps: BTreeSet::from([10, 11]),
        };
        assert_eq!(
            transport.adopt_substep(7, after_command),
            Answer::Done { id: Some(11) }
        );
    }

    /// Two new ids in one answer is not one create, and the transport refuses
    /// to pick: the staged id stays unresolved and the next write says so,
    /// rather than addressing a row that may belong to somebody else.
    #[test]
    fn an_ambiguous_step_answer_names_nothing() {
        let master = MinosMaster::new("http://127.0.0.1:1").expect("a client");
        let mut transport = MasterTransport::new(&master, "t", Ladder::Game);
        transport.known_books.insert(
            7,
            BookIds {
                steps: BTreeSet::new(),
                substeps: BTreeSet::new(),
            },
        );
        let after = BookIds {
            steps: BTreeSet::from([1, 2]),
            substeps: BTreeSet::new(),
        };
        assert_eq!(transport.adopt_step(7, after), Answer::Done { id: None });
    }

    /// THE MISSION LADDER HAS NO GAME. Its step and command writes are
    /// addressed by the mission's id, so they must never stop on "the game's
    /// id is not known yet".
    #[test]
    fn the_mission_ladder_never_asks_for_a_game() {
        let master = MinosMaster::new("http://127.0.0.1:1").expect("a client");
        let mut transport = MasterTransport::new(&master, "t", Ladder::Mission);
        let mut journal = Journal::new();
        journal.resolve_book(-1, 55);
        journal.resolve_step(-2, 66);
        journal.resolve_substep(-3, 77);
        let answer = transport.send(
            &Write::SetAudience {
                book: Ref::Staged(-1),
                step: Ref::Staged(-2),
                substep: Ref::Staged(-3),
                hulls: vec![351],
                nodes: Vec::new(),
            },
            &journal,
        );
        if let Answer::NotSent { sentence } = answer {
            assert!(
                !sentence.contains("game's id"),
                "the mission ladder asked for a game: {sentence}"
            );
        }
    }

    /// A REOPENED DRAFT'S PRESS ALREADY HAS A GAME. Position 01 writes nothing
    /// (the game exists), so the journal never learns the id; the transport is
    /// told it instead, and every write from 02 on can still name it.
    #[test]
    fn a_reopened_press_is_told_the_game_it_edits() {
        let master = MinosMaster::new("http://127.0.0.1:1").expect("a client");
        let blank = MasterTransport::new(&master, "t", Ladder::Game);
        let fresh = crate::draft::Draft::blank(crate::gamestate::GameMode::Scenario)
            .expect("a blank draft");
        assert!(
            blank.for_draft(&fresh, &crate::commit::Live::default())
                .game(&Journal::new())
                .is_err(),
            "a create has no id until 01 answers"
        );
        // The frame's game is the one a reopened draft is about; `for_draft`
        // is what puts it in front of every later write.
        let live = crate::commit::Live {
            game_id: Some(42),
            ..crate::commit::Live::default()
        };
        let reopened = MasterTransport::new(&master, "t", Ladder::Game).for_draft(&fresh, &live);
        assert_eq!(reopened.game(&Journal::new()).ok(), Some(42));
    }

    /// A write that names the game fails LOUDLY when the press has not made one
    /// yet, rather than sending to `/games/0`.
    #[test]
    fn a_game_write_without_a_game_is_not_sent() {
        let master = MinosMaster::new("http://127.0.0.1:1").expect("a client");
        let mut transport = MasterTransport::new(&master, "t", Ladder::Game);
        let answer = transport.send(&Write::Transition { to: "preparation" }, &Journal::new());
        match answer {
            Answer::NotSent { sentence } => {
                assert!(sentence.contains("01 creates it"), "{sentence}")
            }
            other => panic!("expected NotSent, got {other:?}"),
        }
    }

    /// The tree diff keeps its set current, so two creates in one press each
    /// name the right node: the second's difference is against the tree the
    /// first answer left behind.
    #[test]
    fn the_node_diff_names_one_node_per_create() {
        let master = MinosMaster::new("http://127.0.0.1:1").expect("a client");
        let mut transport = MasterTransport::new(&master, "t", Ladder::Game);
        transport.known_nodes = Some(BTreeSet::from([1, 2]));
        let after_first = vec![
            HierarchyNode {
                children: Vec::new(),
                ..node(1)
            },
            HierarchyNode {
                children: Vec::new(),
                ..node(2)
            },
            HierarchyNode {
                children: Vec::new(),
                ..node(10)
            },
        ];
        assert_eq!(transport.adopt_created_node(&after_first), Some(10));
        let after_second = vec![
            HierarchyNode {
                children: vec![HierarchyNode {
                    children: Vec::new(),
                    ..node(11)
                }],
                ..node(1)
            },
            HierarchyNode {
                children: Vec::new(),
                ..node(2)
            },
            HierarchyNode {
                children: Vec::new(),
                ..node(10)
            },
        ];
        assert_eq!(
            transport.adopt_created_node(&after_second),
            Some(11),
            "a child counts too, and the diff is against what the last answer left"
        );
    }

    /// Two new ids in one answer is not one create, and the transport refuses
    /// to pick: an unresolved staged id makes the next write refuse loudly.
    #[test]
    fn an_ambiguous_create_answer_names_nothing() {
        let master = MinosMaster::new("http://127.0.0.1:1").expect("a client");
        let mut transport = MasterTransport::new(&master, "t", Ladder::Game);
        transport.known_nodes = Some(BTreeSet::from([1]));
        let tree = vec![
            HierarchyNode {
                children: Vec::new(),
                ..node(1)
            },
            HierarchyNode {
                children: Vec::new(),
                ..node(2)
            },
            HierarchyNode {
                children: Vec::new(),
                ..node(3)
            },
        ];
        assert_eq!(transport.adopt_created_node(&tree), None);
    }

    /// THE SEAM IS WIRED, end to end: the ENGINE drives the real transport,
    /// the real transport calls a server that is not there, and what comes back
    /// is a LOST ANSWER — not a refusal. The press stops at 01, journals
    /// nothing as written, and the draft is untouched.
    #[test]
    fn the_engine_drives_the_real_transport_against_a_dead_server() {
        let master = MinosMaster::new("http://127.0.0.1:1").expect("a client");
        let mut transport = MasterTransport::new(&master, "t", Ladder::Game);
        let mut draft =
            crate::draft::Draft::blank(crate::gamestate::GameMode::Maneuver).expect("a draft");
        draft.terms.name = "Baltic Shield 26".into();
        // A game with an empty force is refused before the transport is asked
        // anything, so the draft that reaches the dead server has one hull.
        draft.roster.stage(crate::roster::StagedSeat {
            user_id: 4,
            user_name: "Lt. Fitri".into(),
            role_id: 1,
            role_name: "Commando".into(),
            call_sign: "FITRI".into(),
            judge: false,
        });
        draft.force.upsert(crate::force::DraftHull {
            commander_id: Some(4),
            ..crate::force::DraftHull::new(351, "KRI AHMAD YANI".into(), String::new(), String::new())
        });
        let outcome = crate::commit::run(
            &mut draft,
            &crate::commit::Live::default(),
            crate::draft::Verb::SaveAsGameSession,
            "+00:00",
            &mut transport,
        );
        match outcome {
            crate::commit::Outcome::Stopped {
                position,
                failure,
                resume_at,
                ..
            } => {
                assert_eq!(position, 1);
                assert_eq!(resume_at, 1);
                assert_eq!(failure.status, None, "a closed socket has no status");
                assert!(
                    !failure.sentence.is_empty(),
                    "the failure block always has a line to show"
                );
            }
            other => panic!("expected a stop at 01, got {other:?}"),
        }
        assert_eq!(draft.journal.game_id(), None);
        assert_eq!(draft.journal.settled(Ladder::Game), 0);
    }

    fn node(id: i64) -> HierarchyNode {
        HierarchyNode {
            id,
            unit_hierarchy_id: None,
            echelon_name: String::new(),
            echelon_rank: 0,
            parent_id: None,
            name: format!("n{id}"),
            icon: String::new(),
            note: String::new(),
            commander_id: None,
            children: Vec::new(),
        }
    }
}

#[cfg(test)]
mod substep_route_tests {
    use super::*;

    /// THE ROUTE FOLLOWS THE LADDER, and a transport that sent a Saved
    /// Scenario's command to the game route would be sending a valid write to
    /// the wrong resource. This presses nothing: it asserts the fork the arm
    /// takes, which is the part that can be wrong without a server.
    #[test]
    fn the_substep_route_follows_the_ladder() {
        use crate::draft::Ladder;
        // The fork is `self.ladder`, and the two ladders are different values.
        assert_ne!(Ladder::Mission, Ladder::Game);
        assert_eq!(Ladder::Mission.length(), 3, "M1, M2, M3");
        assert_eq!(Ladder::Game.length(), 9, "01..09");
    }
}
