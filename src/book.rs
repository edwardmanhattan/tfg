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

/// A step being written, before the server has given it an id.
///
/// Held as a draft rather than written straight through because the backend
/// assigns ids and answers with the whole scenario: a form that only exists
/// after a round trip is a form the author cannot see their cursor in.
#[derive(Default, Clone)]
pub struct ComposerDraft {
    pub content: String,
    /// `HHMM`, validated by `crate::view::hhmm_ok` on the way out.
    pub start_hour: String,
    pub end_hour: String,
}

impl ComposerDraft {
    pub fn window(&self) -> Option<(&str, &str)> {
        (!self.start_hour.is_empty() && !self.end_hour.is_empty())
            .then_some((self.start_hour.as_str(), self.end_hour.as_str()))
    }

    /// What is wrong with the draft, if anything.
    ///
    /// Half a window is the case that matters: the backend refuses it, and
    /// the refusal arrives after the author has finished typing rather than
    /// while they are still in the field.
    ///
    /// THE RULE IS NOT HERE. It is `crate::window::window_problem` — one
    /// rule for every window in this console, because the exercise's own
    /// window is the same shape and the two answers could otherwise drift
    /// apart. This method names the window ("a window") and asks.
    pub fn problem(&self) -> Option<String> {
        crate::window::window_problem("a window", &self.start_hour, &self.end_hour)
    }
}

/// What a step IS (`§7.1`): an orders step, or a beat.
///
/// The kind is not decoration: a BEAT carries no sub-steps, and the server
/// answers 409 if one is sent. The composer refuses it locally with the same
/// sentence, so the refusal arrives before the press rather than from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StepKind {
    #[default]
    Orders,
    Beat,
}

impl StepKind {
    pub fn wire(self) -> &'static str {
        match self {
            StepKind::Orders => "orders",
            StepKind::Beat => "beat",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            StepKind::Orders => "ORDERS",
            StepKind::Beat => "BEAT",
        }
    }

    /// The sentence the server would give, said here instead.
    pub const BEAT_REFUSAL: &'static str =
        "A BEAT CARRIES NO SUB-STEPS \u{2014} THE SERVER ANSWERS 409";
}

/// One sub-step: a command, its audience, and the nodes the audience came
/// from.
///
/// A SUB-STEP SHARES ITS STEP'S WINDOW (`§7.1`) — it has no window of its own,
/// which is why there is no start or end here. `nodes` is PROVENANCE, not a
/// write: a group dropped on a sub-step resolves to its hulls at drop time, and
/// the nodes are shown as where the hulls came from.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedSubStep {
    /// Negative while staged: never collides with a server id.
    pub local_id: i64,
    /// The step it belongs to (either half staged).
    pub step: i64,
    pub content: String,
    pub units: Vec<i64>,
    pub nodes: Vec<i64>,
}

/// One step being authored, before the server has given it an id.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedStep {
    /// Negative while staged: never collides with a server id, so the
    /// composer can select and display staged rows with the same key.
    pub local_id: i64,
    /// Orders or a beat (`§7.1`). A beat carries no sub-steps.
    pub kind: StepKind,
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
    /// The scenario's own text. Minos holds a description beside the title,
    /// so a plan that says more than its title carries it here.
    pub description: String,
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
    /// Sub-steps by the step they belong to (`§7.1`). A sub-step is ONE command
    /// to ONE audience; its window is its step's.
    substeps: BTreeMap<i64, Vec<StagedSubStep>>,
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
        self.stage_scenario_named(title, "")
    }

    /// Stage a new scenario WITH its description. The description is the
    /// scenario's own prose — the thing a plan says beyond its title — and
    /// Minos holds it beside the title, so it travels with the create.
    pub fn stage_scenario_named(&mut self, title: &str, description: &str) -> i64 {
        let local_id = self.fresh_local();
        self.news.insert(
            local_id,
            StagedScenario {
                local_id,
                title: title.to_string(),
                description: description.to_string(),
                steps: Vec::new(),
            },
        );
        local_id
    }

    /// Set a staged scenario's description, after it was staged.
    pub fn set_description(&mut self, local_id: i64, description: &str) -> bool {
        match self.news.get_mut(&local_id) {
            Some(slot) => {
                slot.description = description.to_string();
                true
            }
            None => false,
        }
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
            kind: StepKind::Orders,
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

    /// Make a step a beat, or an orders step again.
    ///
    /// Turning an orders step into a beat KEEPS its sub-steps: deleting them
    /// would be this form destroying work the operator did, and the server
    /// accepts deleting a sub-step from a beat — so the composer refuses new
    /// ones and leaves the old ones visible.
    pub fn set_kind(&mut self, step_local: i64, kind: StepKind) -> bool {
        for scenario in self.news.values_mut() {
            for step in scenario.steps.iter_mut() {
                if step.local_id == step_local {
                    step.kind = kind;
                    return true;
                }
            }
        }
        for steps in self.step_adds.values_mut() {
            for step in steps.iter_mut() {
                if step.local_id == step_local {
                    step.kind = kind;
                    return true;
                }
            }
        }
        false
    }

    /// The kind of a staged step.
    pub fn kind_of(&self, step_local: i64) -> Option<StepKind> {
        for scenario in self.news.values() {
            if let Some(step) = scenario.steps.iter().find(|s| s.local_id == step_local) {
                return Some(step.kind);
            }
        }
        for steps in self.step_adds.values() {
            if let Some(step) = steps.iter().find(|s| s.local_id == step_local) {
                return Some(step.kind);
            }
        }
        None
    }

    /// Stage a sub-step on a step. A BEAT REFUSES, with the server's sentence
    /// (`§7.1`): the refusal is the same one the 409 would carry, said before
    /// the press instead of after it.
    pub fn stage_substep(&mut self, step_local: i64, content: &str) -> Result<i64, String> {
        if self.kind_of(step_local) == Some(StepKind::Beat) {
            return Err(StepKind::BEAT_REFUSAL.to_string());
        }
        let local_id = self.fresh_local();
        self.substeps
            .entry(step_local)
            .or_default()
            .push(StagedSubStep {
                local_id,
                step: step_local,
                content: content.to_string(),
                units: Vec::new(),
                nodes: Vec::new(),
            });
        Ok(local_id)
    }

    /// Set a sub-step's audience.
    ///
    /// ONE SUB-STEP PER HULL PER STEP: a hull that joins this audience leaves
    /// every other sub-step of the SAME step, because the server refuses two
    /// commands to one hull in one step. A hull in another step is untouched —
    /// a hull can be idle in one step and busy in the next.
    pub fn set_substep_units(
        &mut self,
        step_local: i64,
        substep_local: i64,
        units: Vec<i64>,
    ) -> bool {
        let Some(list) = self.substeps.get_mut(&step_local) else {
            return false;
        };
        for other in list.iter_mut().filter(|s| s.local_id != substep_local) {
            other.units.retain(|u| !units.contains(u));
        }
        let Some(target) = list.iter_mut().find(|s| s.local_id == substep_local) else {
            return false;
        };
        target.units = units;
        true
    }

    /// CUT A HULL OUT OF EVERY SUB-STEP THAT NAMED IT.
    ///
    /// A SUB-STEP'S AUDIENCE IS A LIST OF UNIT IDS, not a reference the
    /// force can cascade: taking a hull out of the exercise left the book
    /// still assigning it a task, and the write would then name a hull the
    /// game does not hold. The cut is local and whole — every sub-step, in
    /// every step — because the alternative is an audience that depends on
    /// which step the operator happened to open.
    ///
    /// Answers how many sub-steps lost it.
    pub fn detach_unit(&mut self, unit_id: i64) -> usize {
        let mut cut = 0usize;
        for list in self.substeps.values_mut() {
            for substep in list.iter_mut() {
                let before = substep.units.len();
                substep.units.retain(|unit| *unit != unit_id);
                if substep.units.len() != before {
                    cut += 1;
                }
            }
        }
        cut
    }

    /// Replace a sub-step's command text in the local planning draft.
    pub fn set_substep_content(
        &mut self,
        step_local: i64,
        substep_local: i64,
        content: impl Into<String>,
    ) -> bool {
        let Some(target) = self
            .substeps
            .get_mut(&step_local)
            .and_then(|list| list.iter_mut().find(|s| s.local_id == substep_local))
        else {
            return false;
        };
        target.content = content.into();
        true
    }

    /// ADD hulls to a sub-step's audience — the drop's own verb (`§7.3`).
    ///
    /// Answers `(added, total)`, because the sheet's release reads
    /// `RELEASE ADDS 2 \u{b7} 5 HULLS`: the operator sees what the drop did and
    /// what the command now holds, not merely that something changed.
    ///
    /// A hull already in ANOTHER sub-step of the SAME step is MOVED, which is
    /// what preserves one sub-step per hull per step. A hull in another STEP is
    /// untouched — a hull can be idle in one step and busy in the next.
    pub fn add_substep_units(
        &mut self,
        step_local: i64,
        substep_local: i64,
        units: &[i64],
    ) -> (usize, usize) {
        let Some(list) = self.substeps.get_mut(&step_local) else {
            return (0, 0);
        };
        for other in list.iter_mut().filter(|s| s.local_id != substep_local) {
            other.units.retain(|u| !units.contains(u));
        }
        let Some(target) = list.iter_mut().find(|s| s.local_id == substep_local) else {
            return (0, 0);
        };
        let mut added = 0usize;
        for unit in units {
            if !target.units.contains(unit) {
                target.units.push(*unit);
                added += 1;
            }
        }
        target.units.sort_unstable();
        (added, target.units.len())
    }

    /// How many sub-steps the draft stages. The arrival's library row counts
    /// them, and nothing else needs the whole list.
    pub fn substep_count(&self) -> usize {
        self.substeps.values().map(Vec::len).sum()
    }

    /// The step's sub-steps, in the order they were authored.    /// The step's sub-steps, in the order they were authored.
    pub fn substeps_of(&self, step_local: i64) -> &[StagedSubStep] {
        self.substeps
            .get(&step_local)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Every sub-step the draft holds, in step then authoring order.
    pub fn all_substeps(&self) -> impl Iterator<Item = &StagedSubStep> {
        self.substeps.values().flat_map(|list| list.iter())
    }

    /// Stage a step onto a server-held scenario.
    pub fn stage_step_add(
        &mut self,
        scenario_id: i64,
        content: &str,
        window: Option<(String, String)>,
    ) -> i64 {
        let step_local = self.fresh_local();
        self.step_adds
            .entry(scenario_id)
            .or_default()
            .push(StagedStep {
                local_id: step_local,
                kind: StepKind::Orders,
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

    pub fn staged_news_mut(&mut self) -> impl Iterator<Item = &mut StagedScenario> {
        self.news.values_mut()
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

    /// How many steps the operator has STAGED, across every scenario in the
    /// book.
    ///
    /// The frame's steps are not here, because this draft does not hold the
    /// frame: `plan` takes the live scenarios as an argument, and a
    /// re-read's steps are the read's, not the draft's. The caller that
    /// wants the whole book's count adds the read to this one — the
    /// creation flow's rail does exactly that, and a count that quietly
    /// answered for half a book would be the sort of number nobody can
    /// check.
    pub fn staged_step_count(&self) -> usize {
        self.news.values().map(|s| s.steps.len()).sum::<usize>()
            + self.step_adds.values().map(Vec::len).sum::<usize>()
    }

    /// How many hulls are staged against a step, across the whole book.
    ///
    /// EVERY ONE IS STEP-LEVEL, and that is why the count matters: Minos
    /// refuses a step-level audience with a 409 always, on both ladders, and
    /// an audience belongs on a sub-step. Until this model holds sub-steps,
    /// a non-zero answer here is the creation flow's `plan` gap rather than a
    /// write (`crate::commit`).
    pub fn relation_count(&self) -> usize {
        self.step_relations.values().map(Vec::len).sum()
    }
}

/// THE IDLE BUCKET, DERIVED (`§7.1`): the force minus the union of the step's
/// sub-step audiences.
///
/// Read-only in this slice, and derivable at all only because the union is the
/// whole answer: a hull with no command in this step is idle in it, and a hull
/// that is idle here may be busy in the next — the step's window still covers
/// it. Sorted by unit id, so a row does not reshuffle between renders.
pub fn idle_in_step(force: &[i64], substeps: &[&StagedSubStep]) -> Vec<i64> {
    let mut at: Vec<i64> = force
        .iter()
        .copied()
        .filter(|unit| !substeps.iter().any(|s| s.units.contains(unit)))
        .collect();
    at.sort_unstable();
    at.dedup();
    at
}

/// One scenario as Minos holds it: the last thing the sync left behind.
///
/// The plan only needs ids — steps of a live scenario are addressed
/// directly, and titles are never rewritten by the flush. The step ids ride
/// along because the FLUSH needs them: a step-add answers with the whole
/// scenario, and the only way to tell which of its steps is the new one is
/// to know which ones were already there.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveScenario {
    pub id: i64,
    /// The step ids the server holds, in book order.
    pub steps: Vec<i64>,
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
    Create {
        local_id: i64,
        title: String,
        /// The scenario's prose, empty when the author wrote none.
        description: String,
    },
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
    /// A sub-step: `POST .../steps/{step}/substeps` (`§7.1`).
    ///
    /// It follows its step, because the step's id is what the route takes —
    /// so the ladder's step position creates the step and the command in the
    /// same breath, and the journal resolves each as the answer names it.
    AddSubstep {
        scenario: ScenarioRef,
        step: StepRef,
        substep: i64,
        content: String,
    },
    /// A sub-step's audience: `PUT .../substeps/{s}/related`, the WHOLE list.
    ///
    /// THE ONLY PLACE AN AUDIENCE IS WRITTEN. A step-level relation is refused
    /// with a 409 on both ladders, so that stays a gap and never becomes a
    /// write.
    SetAudience {
        scenario: ScenarioRef,
        step: StepRef,
        substep: i64,
        hulls: Vec<i64>,
        nodes: Vec<i64>,
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
            Self::AddSubstep { content, .. } => {
                let short: String = content.chars().take(40).collect();
                format!("add sub-step {short:?}")
            }
            Self::SetAudience { hulls, .. } => format!("relate {} hull(s)", hulls.len()),
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
        writes.push(BookWrite::Create {
            local_id: news.local_id,
            title: news.title.clone(),
            description: news.description.clone(),
        });
        for step in &news.steps {
            writes.push(BookWrite::AddStep {
                scenario: ScenarioRef::New(news.local_id),
                step: StepRef::New(step.local_id),
                content: step.content.clone(),
                window: step.start.clone().zip(step.end.clone()),
            });
            // A sub-step is a ROW OF ITS STEP (`§7.1`), so it is written right
            // after the step exists and before the next step is posted: the
            // composer's order is the ladder's order.
            for sub in draft.substeps_of(step.local_id) {
                writes.push(BookWrite::AddSubstep {
                    scenario: ScenarioRef::New(news.local_id),
                    step: StepRef::New(step.local_id),
                    substep: sub.local_id,
                    content: sub.content.clone(),
                });
                writes.push(BookWrite::SetAudience {
                    scenario: ScenarioRef::New(news.local_id),
                    step: StepRef::New(step.local_id),
                    substep: sub.local_id,
                    hulls: sub.units.clone(),
                    nodes: sub.nodes.clone(),
                });
            }
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
            for sub in draft.substeps_of(step.local_id) {
                writes.push(BookWrite::AddSubstep {
                    scenario: ScenarioRef::Live(*sid),
                    step: StepRef::New(step.local_id),
                    substep: sub.local_id,
                    content: sub.content.clone(),
                });
                writes.push(BookWrite::SetAudience {
                    scenario: ScenarioRef::Live(*sid),
                    step: StepRef::New(step.local_id),
                    substep: sub.local_id,
                    hulls: sub.units.clone(),
                    nodes: sub.nodes.clone(),
                });
            }
        }
    }
    for (sid, step_id) in &draft.step_cuts {
        if !live.iter().any(|l| l.id == *sid) {
            continue;
        }
        writes.push(BookWrite::RemoveStep {
            scenario_id: *sid,
            step_id: *step_id,
        });
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
        writes.push(BookWrite::SetRelated {
            scenario,
            step,
            refs: refs.clone(),
        });
    }
    writes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live(id: i64) -> LiveScenario {
        LiveScenario {
            id,
            steps: Vec::new(),
        }
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
                BookWrite::Create {
                    local_id: local,
                    title: "First light".into(),
                    description: String::new(),
                },
                BookWrite::AddStep {
                    scenario: ScenarioRef::New(local),
                    step: StepRef::New(
                        draft
                            .get_new(local)
                            .and_then(|sc| sc.steps.first().map(|s| s.local_id))
                            .unwrap_or(0)
                    ),
                    content: "sweep north".into(),
                    window: Some(("0600".into(), "0700".into())),
                },
                BookWrite::AddStep {
                    scenario: ScenarioRef::New(local),
                    step: StepRef::New(
                        draft
                            .get_new(local)
                            .and_then(|sc| sc.steps.get(1).map(|s| s.local_id))
                            .unwrap_or(0)
                    ),
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

    /// The description travels with the CREATE, because Minos holds a
    /// description beside the title and a plan that says more than its title
    /// would lose the rest of itself on the way.
    #[test]
    fn a_staged_scenario_carries_its_description_into_the_create() {
        let mut draft = BookDraft::new();
        let local = draft.stage_scenario_named("First light", "The river crossing, at dawn.");
        assert_eq!(
            plan(&draft, &[]),
            vec![BookWrite::Create {
                local_id: local,
                title: "First light".into(),
                description: "The river crossing, at dawn.".into(),
            }]
        );
    }

    /// A description set AFTER the scenario was staged still travels: the
    /// composer writes the name first and the prose second.
    #[test]
    fn a_description_set_after_staging_still_travels() {
        let mut draft = BookDraft::new();
        let local = draft.stage_scenario("First light");
        assert!(draft.set_description(local, "At dawn."));
        let writes = plan(&draft, &[]);
        assert!(matches!(
            &writes[0],
            BookWrite::Create { description, .. } if description == "At dawn."
        ));
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
                BookWrite::AddSubstep { .. } => "addsub",
                BookWrite::SetAudience { .. } => "audience",
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

    /// A relation against a scenario the server no longer lists is dropped:
    /// posting a link for a row that is not there would 404.
    #[test]
    fn prune_drops_relations_for_vanished_scenarios() {
        let mut draft = BookDraft::new();
        draft.stage_relations(11, 3, vec![("hull".into(), 13)]);
        draft.prune(&[]);
        assert!(draft.step_relations.is_empty());
    }

    /// A relation on a scenario the DRAFT is about to create is kept: the
    /// scenario is not vanished, it is not on the server YET, and the flush
    /// posts the links right behind it.
    #[test]
    fn prune_keeps_relations_for_a_staged_scenario() {
        let mut draft = BookDraft::new();
        let local = draft.stage_scenario("Temp");
        let step = draft.stage_step_new(local, "x", None).unwrap();
        draft.stage_relations(local, step, vec![("hull".into(), 13)]);
        draft.prune(&[]);
        assert_eq!(
            draft.step_relations.len(),
            1,
            "the link rides the scenario the flush is about to create"
        );
    }

    // -- the composer's draft ---------------------------------------------
    //
    // The draft is the only place the author can be told what is wrong with
    // a step BEFORE the round trip, so its rules are worth pinning: a
    // refusal that only arrives as a 400 from the field is a refusal the
    // author has to guess the cause of.
    //
    // The WINDOW's rules are not here: they are `crate::window`'s, because
    // the exercise's own window is the same shape and one rule answers both.
    // What is here is the composer's half of it — the pair the request
    // takes, and the delegation that keeps the two answers from drifting.

    /// An empty step is a legitimate authoring state, not something the
    /// draft refuses. The backend says so explicitly, and a client that
    /// insisted on text would block an author from saving a skeleton.
    #[test]
    fn an_empty_step_is_not_the_drafts_business() {
        let d = ComposerDraft::default();
        assert_eq!(
            d.problem(),
            None,
            "content is the server's rule, not the draft's"
        );
    }

    /// A good window comes through as the pair the request expects, not as
    /// the two fields re-derived at the call site.
    #[test]
    fn a_good_window_becomes_the_pair_the_request_takes() {
        let d = ComposerDraft {
            content: "".into(),
            start_hour: "1000".into(),
            end_hour: "1030".into(),
        };
        assert_eq!(d.problem(), None);
        assert_eq!(d.window(), Some(("1000", "1030")));
    }

    /// A half window is refused by the ONE rule, and the composer's own
    /// sentence is the rule's sentence with the window's name in it.
    #[test]
    fn the_composer_asks_the_shared_window_rule() {
        let half = ComposerDraft {
            start_hour: "1000".into(),
            ..Default::default()
        };
        assert_eq!(
            half.problem().as_deref(),
            crate::window::window_problem("a window", "1000", "").as_deref()
        );
        assert_eq!(
            half.problem().as_deref(),
            Some("a window needs an end as well as a start")
        );
    }
}

#[cfg(test)]
mod substep_tests {
    use super::*;

    fn book() -> (BookDraft, i64, i64) {
        let mut book = BookDraft::new();
        let scenario = book.stage_scenario_named("Harbour sweep", "the sweep north");
        let step = book
            .stage_step_new(
                scenario,
                "SWEEP the strait",
                Some(("0930".into(), "1015".into())),
            )
            .expect("a step");
        (book, scenario, step)
    }

    /// A BEAT CARRIES NO SUB-STEPS, and the refusal is the server's sentence
    /// said before the press instead of after it.
    #[test]
    fn a_beat_refuses_a_substep_with_the_servers_words() {
        let (mut book, _scenario, step) = book();
        assert!(book.set_kind(step, StepKind::Beat));
        let err = book.stage_substep(step, "TAKE STATION").unwrap_err();
        assert_eq!(err, StepKind::BEAT_REFUSAL);
        assert!(err.contains("409"), "the operator reads the answer's shape");
        // And an orders step takes one.
        assert!(book.set_kind(step, StepKind::Orders));
        assert!(book.stage_substep(step, "TAKE STATION").is_ok());
    }

    /// ONE SUB-STEP PER HULL PER STEP: a hull joining one audience leaves the
    /// others in the SAME step.
    #[test]
    fn a_hull_leaves_another_substep_of_the_same_step() {
        let (mut book, _scenario, step) = book();
        let first = book.stage_substep(step, "TAKE STATION XYZ").expect("first");
        let second = book
            .stage_substep(step, "TAKE STATION XYA")
            .expect("second");
        assert!(book.set_substep_units(step, first, vec![351, 352, 353]));
        assert!(book.set_substep_units(step, second, vec![352, 367]));
        let subs = book.substeps_of(step);
        let a = subs.iter().find(|s| s.local_id == first).expect("first");
        let b = subs.iter().find(|s| s.local_id == second).expect("second");
        assert_eq!(a.units, vec![351, 353], "352 moved to the other command");
        assert_eq!(b.units, vec![352, 367]);
    }

    /// THE IDLE BUCKET IS THE FORCE MINUS EVERY AUDIENCE, in unit order.
    #[test]
    fn the_idle_bucket_is_derived() {
        let (mut book, _scenario, step) = book();
        let first = book.stage_substep(step, "TAKE STATION XYZ").expect("first");
        let second = book
            .stage_substep(step, "TAKE STATION XYA")
            .expect("second");
        book.set_substep_units(step, first, vec![351, 352, 353]);
        book.set_substep_units(step, second, vec![354, 367]);
        let subs: Vec<&StagedSubStep> = book.substeps_of(step).iter().collect();
        let force = vec![371, 524, 365, 361, 367, 351, 352, 353, 354];
        assert_eq!(
            idle_in_step(&force, &subs),
            vec![361, 365, 371, 524],
            "sorted, deduped, and nobody from an audience"
        );
    }
}

#[cfg(test)]
mod substep_plan_tests {
    use super::*;

    /// A SUB-STEP IS POSTED AFTER ITS STEP AND ITS AUDIENCE AFTER THAT — the
    /// composer's order is the ladder's order, because the route takes the
    /// step's id.
    #[test]
    fn a_substep_and_its_audience_follow_their_step() {
        let mut book = BookDraft::new();
        let scenario = book.stage_scenario("Harbour sweep");
        let first = book
            .stage_step_new(scenario, "SWEEP", Some(("0930".into(), "1015".into())))
            .expect("a step");
        let sub = book
            .stage_substep(first, "TAKE STATION XYZ")
            .expect("a sub-step");
        book.set_substep_units(first, sub, vec![351, 352]);
        let second = book
            .stage_step_new(scenario, "RECOVER", None)
            .expect("a step");
        let writes = plan(&book, &[]);
        let kinds: Vec<&str> = writes
            .iter()
            .map(|w| match w {
                BookWrite::Create { .. } => "create",
                BookWrite::AddStep { .. } => "addstep",
                BookWrite::AddSubstep { .. } => "addsub",
                BookWrite::SetAudience { .. } => "audience",
                BookWrite::RemoveStep { .. } => "remove",
                BookWrite::SetRelated { .. } => "relate",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["create", "addstep", "addsub", "audience", "addstep"],
            "the command rides its step, and the next step follows"
        );
        assert!(matches!(
            &writes[3],
            BookWrite::SetAudience { substep, hulls, .. } if *substep == sub && hulls == &vec![351, 352]
        ));
        // `second` has no sub-steps, and nothing invents one.
        assert!(
            matches!(&writes[4], BookWrite::AddStep { step: StepRef::New(id), .. } if *id == second)
        );
    }
}

#[cfg(test)]
mod audience_drop_tests {
    use super::*;

    /// THE DROP ADDS, and answers the two numbers the release reads: how many
    /// joined, and how many the command now holds.
    #[test]
    fn adding_units_answers_added_and_total() {
        let mut book = BookDraft::new();
        let scenario = book.stage_scenario("Harbour sweep");
        let step = book
            .stage_step_new(scenario, "SWEEP", None)
            .expect("a step");
        let sub = book.stage_substep(step, "TAKE STATION XYZ").expect("a sub");
        book.set_substep_units(step, sub, vec![351]);
        assert_eq!(book.add_substep_units(step, sub, &[352, 353]), (2, 3));
        // Dropping the same hulls again adds nothing, and says so.
        assert_eq!(book.add_substep_units(step, sub, &[352, 353]), (0, 3));
    }

    /// A HULL ALREADY IN ANOTHER SUB-STEP OF THE SAME STEP IS MOVED — that is
    /// the rule the server states, kept here rather than discovered on the wire.
    #[test]
    fn a_drop_moves_a_hull_between_substeps_of_one_step() {
        let mut book = BookDraft::new();
        let scenario = book.stage_scenario("Harbour sweep");
        let step = book
            .stage_step_new(scenario, "SWEEP", None)
            .expect("a step");
        let a = book.stage_substep(step, "TAKE STATION XYZ").expect("a");
        let b = book.stage_substep(step, "TAKE STATION XYA").expect("b");
        book.set_substep_units(step, a, vec![351, 352, 353]);
        book.set_substep_units(step, b, vec![367]);
        // 352 joins b: moved out of a, added to b.
        assert_eq!(book.add_substep_units(step, b, &[352]), (1, 2));
        let subs = book.substeps_of(step);
        assert_eq!(subs[0].units, vec![351, 353]);
        assert_eq!(subs[1].units, vec![352, 367]);
    }

    /// A HULL IN ANOTHER STEP IS UNTOUCHED.
    #[test]
    fn a_drop_leaves_the_other_steps_alone() {
        let mut book = BookDraft::new();
        let scenario = book.stage_scenario("Harbour sweep");
        let first = book
            .stage_step_new(scenario, "SWEEP", None)
            .expect("a step");
        let second = book
            .stage_step_new(scenario, "RECOVER", None)
            .expect("a step");
        let a = book.stage_substep(first, "TAKE STATION").expect("a");
        let b = book.stage_substep(second, "TAKE STATION").expect("b");
        book.set_substep_units(first, a, vec![351]);
        book.set_substep_units(second, b, vec![351]);
        assert_eq!(
            book.substeps_of(first)[0].units,
            vec![351],
            "still the first step's hull"
        );
        assert_eq!(
            book.substeps_of(second)[0].units,
            vec![351],
            "and the second's"
        );
    }
}
