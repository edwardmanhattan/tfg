//! The scenario book as the operator has it, before Minos has heard of it.
//!
//! AUTHORING A SCENARIO AND ITS STEPS ARE LOCAL ACTS. They used to be one
//! backend write each, fired the moment the operator clicked, so composing
//! against a game the operator's roles could not yet write answered 403 in
//! the middle of the draft — with no session built yet to explain it.
//!
//! What Minos still owns is the record. The draft is what the operator is
//! looking at; a stage advance is the moment the draft is written through,
//! and [`plan`] is the diff that says which writes that is. Nothing here
//! talks to the network, which is the point: the rules about what a legal
//! draft is are pure functions over plain data, so they are testable
//! without a session.
//!
//! THE ORDER OF [`plan`] IS CREATES, THEN STEPS. A step belongs to a
//! scenario the server has to hold first, so a staged scenario is created
//! before any of its steps are posted, and steps of one scenario stay in
//! the order they were authored.

use std::collections::BTreeMap;

/// One step being authored, before the server has given it an id.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedStep {
    /// Negative while staged: never collides with a server id, so the
    /// composer can select and display staged rows with the same key.
    pub local_id: i64,
    pub content: String,
    /// `HHMM`, both ends or neither — validated on the way in, same as
    /// the step form, so the flush never posts a half window.
    pub start: Option<String>,
    pub end: Option<String>,
}

/// One scenario being authored, before the server has given it an id.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedScenario {
    /// Negative while staged: never collides with a server id.
    pub local_id: i64,
    pub title: String,
    pub steps: Vec<StagedStep>,
}

/// The operator's scenario draft: brand-new scenarios with their steps,
/// plus step adds and step cuts against scenarios the server holds.
///
/// Step edits to a staged-new scenario live on the scenario itself; only
/// edits to server-held scenarios need the side maps.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BookDraft {
    news: BTreeMap<i64, StagedScenario>,
    step_adds: BTreeMap<i64, Vec<StagedStep>>,
    step_cuts: Vec<(i64, i64)>,
    next_local: i64,
}

impl BookDraft {
    pub fn new() -> Self {
        Self::default()
    }

    fn fresh_local(&mut self) -> i64 {
        self.next_local -= 1;
        self.next_local
    }

    /// Stage a new scenario. Answers its local id (negative) so the
    /// composer can open it like any other row in the book.
    pub fn stage_scenario(&mut self, title: &str) -> i64 {
        let local_id = self.fresh_local();
        self.news.insert(
            local_id,
            StagedScenario { local_id, title: title.to_string(), steps: Vec::new() },
        );
        local_id
    }

    /// Abandon a staged-new scenario outright. Server-held scenarios are
    /// never deleted from here — the composer has no delete-scenario
    /// control, so there is no write to plan for one.
    pub fn drop_scenario(&mut self, local_id: i64) -> bool {
        self.news.remove(&local_id).is_some()
    }

    /// Stage a step onto a staged-new scenario.
    pub fn stage_step_new(
        &mut self,
        local_id: i64,
        content: &str,
        window: Option<(String, String)>,
    ) -> Option<i64> {
        let step_local = self.fresh_local();
        let slot = self.news.get_mut(&local_id)?;
        slot.steps.push(StagedStep {
            local_id: step_local,
            content: content.to_string(),
            start: window.clone().map(|(a, _)| a),
            end: window.map(|(_, b)| b),
        });
        Some(step_local)
    }

    /// Drop a step from a staged-new scenario.
    pub fn drop_new_step(&mut self, local_id: i64, step_local: i64) -> bool {
        let Some(slot) = self.news.get_mut(&local_id) else {
            return false;
        };
        let before = slot.steps.len();
        slot.steps.retain(|s| s.local_id != step_local);
        slot.steps.len() != before
    }

    /// Stage a step onto a server-held scenario.
    pub fn stage_step_add(
        &mut self,
        scenario_id: i64,
        content: &str,
        window: Option<(String, String)>,
    ) -> i64 {
        let step_local = self.fresh_local();
        self.step_adds.entry(scenario_id).or_default().push(StagedStep {
            local_id: step_local,
            content: content.to_string(),
            start: window.clone().map(|(a, _)| a),
            end: window.map(|(_, b)| b),
        });
        step_local
    }

    /// Withdraw a staged step-add before it posts.
    pub fn drop_added_step(&mut self, scenario_id: i64, step_local: i64) -> bool {
        let Some(steps) = self.step_adds.get_mut(&scenario_id) else {
            return false;
        };
        let before = steps.len();
        steps.retain(|s| s.local_id != step_local);
        let dropped = steps.len() != before;
        if steps.is_empty() {
            self.step_adds.remove(&scenario_id);
        }
        dropped
    }

    /// Stage the removal of a server-held step.
    pub fn stage_step_cut(&mut self, scenario_id: i64, step_id: i64) {
        if !self.step_cuts.contains(&(scenario_id, step_id)) {
            self.step_cuts.push((scenario_id, step_id));
        }
    }

    /// Withdraw a staged removal: the step stands after all.
    pub fn lift_cut(&mut self, scenario_id: i64, step_id: i64) -> bool {
        let before = self.step_cuts.len();
        self.step_cuts.retain(|cut| *cut != (scenario_id, step_id));
        self.step_cuts.len() != before
    }

    pub fn get_new(&self, local_id: i64) -> Option<&StagedScenario> {
        self.news.get(&local_id)
    }

    pub fn staged_news(&self) -> impl Iterator<Item = &StagedScenario> {
        self.news.values()
    }

    /// Staged step-adds for one server-held scenario, in authoring order.
    pub fn added_steps(&self, scenario_id: i64) -> impl Iterator<Item = &StagedStep> {
        self.step_adds.get(&scenario_id).into_iter().flatten()
    }

    /// Whether a server-held step is staged for removal.
    pub fn is_cut(&self, scenario_id: i64, step_id: i64) -> bool {
        self.step_cuts.contains(&(scenario_id, step_id))
    }

    pub fn is_empty(&self) -> bool {
        self.news.is_empty() && self.step_adds.is_empty() && self.step_cuts.is_empty()
    }

    /// Drop staged ops against scenarios the server no longer lists, so
    /// a deleted scenario cannot hold orphaned steps in the draft.
    pub fn prune(&mut self, live_ids: &[i64]) {
        self.step_adds.retain(|sid, _| live_ids.contains(sid));
        self.step_cuts.retain(|(sid, _)| live_ids.contains(sid));
    }

    /// Drop the whole draft: a released hold, a closed session, a hold
    /// that moved to another game.
    pub fn clear(&mut self) {
        self.news.clear();
        self.step_adds.clear();
        self.step_cuts.clear();
        self.next_local = 0;
    }

    /// How many writes are still owed to the server.
    pub fn pending(&self, live: &[LiveScenario]) -> usize {
        plan(self, live).len()
    }
}

/// One scenario as Minos holds it: the last thing the sync left behind.
///
/// The plan only needs ids — steps of a live scenario are addressed
/// directly, and titles are never rewritten by the flush.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveScenario {
    pub id: i64,
}

/// Which scenario a staged step is posted to: one the server holds, or
/// one the same flush is about to create (resolved from the create
/// answer at pump time, never at plan time).
#[derive(Debug, Clone, PartialEq)]
pub enum ScenarioRef {
    Live(i64),
    New(i64),
}

/// One book write the sync owes Minos. Each is one endpoint.
#[derive(Debug, Clone, PartialEq)]
pub enum BookWrite {
    /// Append a scenario to the book's end. The answer carries the
    /// server id the staged steps resolve against.
    Create { local_id: i64, title: String },
    /// Append a step to the end of a scenario.
    AddStep {
        scenario: ScenarioRef,
        content: String,
        window: Option<(String, String)>,
    },
    /// Delete one step. Positions are never renumbered, so no
    /// sibling write depends on this one.
    RemoveStep { scenario_id: i64, step_id: i64 },
}

impl BookWrite {
    /// What this write is about, for a line that has to say which page
    /// held the whole advance up.
    pub fn label(&self) -> String {
        match self {
            Self::Create { title, .. } => format!("create scenario {title:?}"),
            Self::AddStep { content, .. } => {
                let short: String = content.chars().take(40).collect();
                format!("add step {short:?}")
            }
            Self::RemoveStep { step_id, .. } => format!("remove step {step_id}"),
        }
    }
}

/// The writes that carry `draft` onto a server holding `live`, in the
/// order the server accepts them.
///
/// A new scenario is created before any of its steps are posted, and a
/// new scenario's steps stay in authoring order behind it. Ops against a
/// scenario the server no longer lists are skipped rather than posted
/// at a row that is not there.
pub fn plan(draft: &BookDraft, live: &[LiveScenario]) -> Vec<BookWrite> {
    let mut writes = Vec::new();
    // `news` iterates local-id-ascending: -3, -2, -1 — oldest staged
    // first, so the book order matches the authoring order.
    for news in draft.staged_news() {
        writes.push(BookWrite::Create { local_id: news.local_id, title: news.title.clone() });
        for step in &news.steps {
            writes.push(BookWrite::AddStep {
                scenario: ScenarioRef::New(news.local_id),
                content: step.content.clone(),
                window: step.start.clone().zip(step.end.clone()),
            });
        }
    }
    let mut live_adds: Vec<(&i64, &Vec<StagedStep>)> = draft.step_adds.iter().collect();
    live_adds.sort_by_key(|(sid, _)| **sid);
    for (sid, steps) in live_adds {
        if !live.iter().any(|l| l.id == *sid) {
            continue;
        }
        for step in steps {
            writes.push(BookWrite::AddStep {
                scenario: ScenarioRef::Live(*sid),
                content: step.content.clone(),
                window: step.start.clone().zip(step.end.clone()),
            });
        }
    }
    for (sid, step_id) in &draft.step_cuts {
        if !live.iter().any(|l| l.id == *sid) {
            continue;
        }
        writes.push(BookWrite::RemoveStep { scenario_id: *sid, step_id: *step_id });
    }
    writes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(id: i64) -> LiveScenario {
        LiveScenario { id }
    }

    #[test]
    fn an_empty_draft_plans_nothing() {
        assert_eq!(plan(&BookDraft::new(), &[]), Vec::new());
    }

    #[test]
    fn a_new_scenario_is_created_before_its_steps() {
        let mut draft = BookDraft::new();
        let local = draft.stage_scenario("First light");
        draft.stage_step_new(local, "sweep north", Some(("0600".into(), "0700".into())));
        draft.stage_step_new(local, "hold", None);
        assert_eq!(
            plan(&draft, &[]),
            vec![
                BookWrite::Create { local_id: local, title: "First light".into() },
                BookWrite::AddStep {
                    scenario: ScenarioRef::New(local),
                    content: "sweep north".into(),
                    window: Some(("0600".into(), "0700".into())),
                },
                BookWrite::AddStep {
                    scenario: ScenarioRef::New(local),
                    content: "hold".into(),
                    window: None,
                },
            ]
        );
    }

    #[test]
    fn local_ids_are_negative_and_unique() {
        let mut draft = BookDraft::new();
        let a = draft.stage_scenario("A");
        let b = draft.stage_scenario("B");
        assert!(a < 0 && b < 0 && a != b, "never collide with server ids");
    }

    #[test]
    fn a_step_add_to_a_live_scenario_is_one_write() {
        let mut draft = BookDraft::new();
        draft.stage_step_add(11, "screen the lanes", None);
        assert_eq!(
            plan(&draft, &[live(11)]),
            vec![BookWrite::AddStep {
                scenario: ScenarioRef::Live(11),
                content: "screen the lanes".into(),
                window: None,
            }]
        );
    }

    #[test]
    fn ops_against_a_vanished_scenario_are_skipped() {
        let mut draft = BookDraft::new();
        draft.stage_step_add(11, "screen the lanes", None);
        draft.stage_step_cut(11, 3);
        assert_eq!(plan(&draft, &[]), Vec::new());
    }

    #[test]
    fn dropping_a_staged_new_scenario_leaves_no_write() {
        let mut draft = BookDraft::new();
        let local = draft.stage_scenario("Abandoned");
        draft.stage_step_new(local, "never mind", None);
        assert!(draft.drop_scenario(local));
        assert_eq!(plan(&draft, &[]), Vec::new());
    }

    #[test]
    fn withdrawing_a_staged_add_leaves_no_write() {
        let mut draft = BookDraft::new();
        let step = draft.stage_step_add(11, "screen the lanes", None);
        assert!(draft.drop_added_step(11, step));
        assert_eq!(plan(&draft, &[live(11)]), Vec::new());
    }
}
