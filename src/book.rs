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
/// edits to server-held scenarios need the side maps. Relations ride a
/// side map too, keyed by (scenario, step) with either half staged
/// (negative) or live — authoring links hulls to steps that do not exist
/// yet is the ordinary planning flow, not an edge.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BookDraft {
    news: BTreeMap<i64, StagedScenario>,
    step_adds: BTreeMap<i64, Vec<StagedStep>>,
    step_cuts: Vec<(i64, i64)>,
    step_relations: BTreeMap<(i64, i64), Vec<(String, i64)>>,
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
    /// control, so there is no write to plan for one. Its staged
    /// relations go with it: posting links for a scenario that will
    /// never exist would 404 every one.
    pub fn drop_scenario(&mut self, local_id: i64) -> bool {
        self.step_relations.retain(|(sid, _), _| *sid != local_id);
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
        let dropped = slot.steps.len() != before;
        if dropped {
            // A step that never posts takes its staged links with it.
            self.step_relations.remove(&(local_id, step_local));
        }
        dropped
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
        if dropped {
            self.step_relations.remove(&(scenario_id, step_local));
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

    /// Stage a step's whole related list: hulls of the exercise and nodes
    /// of its task organisation, in authoring order. Either half of the
    /// key may be staged (negative) — linking a hull to a step that does
    /// not exist yet is the planning flow, and the flush posts it after
    /// the step lands. An empty list is a real write (it clears), so
    /// presence of the entry, not its length, is what plans.
    pub fn stage_relations(&mut self, scenario_id: i64, step_id: i64, refs: Vec<(String, i64)>) {
        self.step_relations.insert((scenario_id, step_id), refs);
    }

    /// The staged related list for a step, if the author touched it.
    pub fn related(&self, scenario_id: i64, step_id: i64) -> Option<&Vec<(String, i64)>> {
        self.step_relations.get(&(scenario_id, step_id))
    }

    pub fn is_empty(&self) -> bool {
        self.news.is_empty()
            && self.step_adds.is_empty()
            && self.step_cuts.is_empty()
            && self.step_relations.is_empty()
    }

    /// Drop staged ops against scenarios the server no longer lists, so
    /// a deleted scenario cannot hold orphaned steps in the draft. Cuts
    /// take their staged links with them: posting relations for a step
    /// being deleted would race its own removal.
    pub fn prune(&mut self, live_ids: &[i64]) {
        self.step_adds.retain(|sid, _| live_ids.contains(sid));
        self.step_cuts.retain(|(sid, _)| live_ids.contains(sid));
        // Relations follow the same rule, but posting links for a staged
        // row that no longer exists would 404 — so staged entries are
        // checked against the draft itself, live ones against the server.
        let draft_has = |sid: i64, step: i64| {
            if sid < 0 {
                return self
                    .news
                    .get(&sid)
                    .is_some_and(|sc| sc.steps.iter().any(|s| s.local_id == step));
            }
            if step < 0 {
                return self
                    .step_adds
                    .get(&sid)
                    .is_some_and(|steps| steps.iter().any(|s| s.local_id == step));
            }
            !self.step_cuts.contains(&(sid, step))
        };
        self.step_relations.retain(|(sid, step), _| {
            if *sid < 0 {
                draft_has(*sid, *step)
            } else if *step < 0 {
                live_ids.contains(sid) && draft_has(*sid, *step)
            } else {
                live_ids.contains(sid) && draft_has(*sid, *step)
            }
        });
    }

    /// Drop the whole draft: a released hold, a closed session, a hold
    /// that moved to another game.
    pub fn clear(&mut self) {
        self.news.clear();
        self.step_adds.clear();
        self.step_cuts.clear();
        self.step_relations.clear();
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

/// Which step a staged write addresses: one the server holds, or one
/// the same flush just posted (resolved from the step-add answer's diff
/// at apply time — exactly one unknown step appears per answer, and any
/// other count fails the flush loudly rather than guessing).
#[derive(Debug, Clone, PartialEq)]
pub enum StepRef {
    Live(i64),
    New(i64),
}

/// One book write the sync owes Minos. Each is one endpoint.
#[derive(Debug, Clone, PartialEq)]
pub enum BookWrite {
    /// Append a scenario to the book's end. The answer carries the
    /// server id the staged steps resolve against.
    Create { local_id: i64, title: String },
    /// Append a step to the end of a scenario. `step` is always staged
    /// (live steps are never added) and resolves like the scenario.
    AddStep {
        scenario: ScenarioRef,
        step: StepRef,
        content: String,
        window: Option<(String, String)>,
    },
    /// Delete one step. Positions are never renumbered, so no
    /// sibling write depends on this one.
    RemoveStep { scenario_id: i64, step_id: i64 },
    /// Replace one step's related list: hulls of the exercise and nodes
    /// of its task organisation. Last, because every row it names — the
    /// scenario, the step, the pieces — has to exist first.
    SetRelated {
        scenario: ScenarioRef,
        step: StepRef,
        refs: Vec<(String, i64)>,
    },
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
            Self::SetRelated { refs, .. } => format!("relate {} post(s)", refs.len()),
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
                step: StepRef::New(step.local_id),
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
                step: StepRef::New(step.local_id),
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
    // Relations last: every row they name — the scenario, the step — is
    // posted ahead of them.
    for ((sid, step), refs) in &draft.step_relations {
        let (scenario, step) = if *sid < 0 {
            (ScenarioRef::New(*sid), StepRef::New(*step))
        } else if *step < 0 {
            if !live.iter().any(|l| l.id == *sid) {
                continue;
            }
            (ScenarioRef::Live(*sid), StepRef::New(*step))
        } else {
            if !live.iter().any(|l| l.id == *sid) {
                continue;
            }
            (ScenarioRef::Live(*sid), StepRef::Live(*step))
        };
        writes.push(BookWrite::SetRelated { scenario, step, refs: refs.clone() });
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
                    step: StepRef::New(draft
                        .get_new(local)
                        .and_then(|sc| sc.steps.first().map(|s| s.local_id))
                        .unwrap_or(0)),
                    content: "sweep north".into(),
                    window: Some(("0600".into(), "0700".into())),
                },
                BookWrite::AddStep {
                    scenario: ScenarioRef::New(local),
                    step: StepRef::New(draft
                        .get_new(local)
                        .and_then(|sc| sc.steps.get(1).map(|s| s.local_id))
                        .unwrap_or(0)),
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
                step: StepRef::New(
                    draft
                        .added_steps(11)
                        .next()
                        .map(|s| s.local_id)
                        .unwrap_or(0),
                ),
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

    #[test]
    fn relations_post_after_creates_and_step_adds() {
        // The ordinary planning flow: scenario and steps are staged, so
        // their links are too — the flush must post the rows those links
        // name BEFORE the links themselves.
        let mut draft = BookDraft::new();
        let local = draft.stage_scenario("First light");
        let step = draft.stage_step_new(local, "sweep", None).unwrap();
        draft.stage_relations(local, step, vec![("hull".into(), 13), ("node".into(), 5)]);
        let writes = plan(&draft, &[]);
        let kinds: Vec<&str> = writes
            .iter()
            .map(|w| match w {
                BookWrite::Create { .. } => "create",
                BookWrite::AddStep { .. } => "addstep",
                BookWrite::RemoveStep { .. } => "remove",
                BookWrite::SetRelated { .. } => "relate",
            })
            .collect();
        assert_eq!(kinds, vec!["create", "addstep", "relate"]);
        assert!(matches!(
            writes[2],
            BookWrite::SetRelated {
                scenario: ScenarioRef::New(_),
                step: StepRef::New(_),
                ..
            }
        ));
    }

    #[test]
    fn live_relations_post_last() {
        let mut draft = BookDraft::new();
        draft.stage_step_add(11, "screen the lanes", None);
        // One staged add and one link on a live step: add first, relate
        // last, so the link names a step the server already holds.
        draft.stage_relations(11, 3, vec![("hull".into(), 13)]);
        let writes = plan(&draft, &[live(11)]);
        let kinds: Vec<&str> = writes
            .iter()
            .map(|w| match w {
                BookWrite::AddStep { .. } => "addstep",
                BookWrite::SetRelated { .. } => "relate",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["addstep", "relate"]);
        assert!(matches!(
            writes[1],
            BookWrite::SetRelated {
                scenario: ScenarioRef::Live(11),
                step: StepRef::Live(3),
                ..
            }
        ));
    }

    #[test]
    fn cutting_a_step_drops_its_staged_links_at_prune() {
        // Posting a link for a step being deleted would race its own
        // removal, so the cut wins and the link goes with it.
        let mut draft = BookDraft::new();
        draft.stage_step_cut(11, 3);
        draft.stage_relations(11, 3, vec![("hull".into(), 13)]);
        draft.prune(&[11]);
        let kinds: Vec<&str> = plan(&draft, &[live(11)])
            .iter()
            .map(|w| match w {
                BookWrite::RemoveStep { .. } => "remove",
                BookWrite::SetRelated { .. } => "relate",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["remove"], "the cut stands and its link is gone");
    }

    #[test]
    fn an_empty_staged_list_is_a_real_write() {
        // Clearing a step's relations is an edit: presence of the entry,
        // not its length, is what plans.
        let mut draft = BookDraft::new();
        draft.stage_relations(11, 3, Vec::new());
        assert_eq!(plan(&draft, &[live(11)]).len(), 1);
    }

    #[test]
    fn prune_drops_relations_for_vanished_drafts() {
        let mut draft = BookDraft::new();
        let local = draft.stage_scenario("Temp");
        let step = draft.stage_step_new(local, "x", None).unwrap();
        draft.stage_relations(local, step, vec![("hull".into(), 13)]);
        draft.prune(&[]);
        assert!(draft.step_relations.is_empty());
    }
}
