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

/// An `HHMM` stamp as minutes since midnight. `None` for anything that is not
/// one, so arithmetic never runs on a half-typed field.
fn hhmm_minutes(text: &str) -> Option<i64> {
    if !crate::view::hhmm_ok(text) {
        return None;
    }
    let hours: i64 = text[0..2].parse().ok()?;
    let minutes: i64 = text[2..4].parse().ok()?;
    Some(hours * 60 + minutes)
}

/// Minutes since midnight back as `HHMM`, wrapping at the day's end — a step
/// that runs past midnight is still a legal stamp, and the wrap is the only
/// honest reading of "half an hour later" at 23:45.
fn hhmm_at(minutes: i64) -> String {
    let at = minutes.rem_euclid(24 * 60);
    format!("{:02}{:02}", at / 60, at % 60)
}

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

/// One edit to a step the server already holds.
///
/// A saved step is rewritten with `PATCH .../steps/{id}`, so the edit is a
/// side map rather than a second copy of the step: only the fields the
/// operator touched are carried, and a field left `None` is left alone on the
/// server. A WINDOW IS BOTH ENDS — the picker edits one end, but a window is
/// one value, and sending half of it would be a state Minos has no reading
/// for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StepEdit {
    pub kind: Option<StepKind>,
    pub start: Option<String>,
    pub end: Option<String>,
}

impl StepEdit {
    /// The window this edit carries, if it touched one.
    pub fn window(&self) -> Option<(String, String)> {
        self.start.clone().zip(self.end.clone())
    }
}

/// One saved step's window, as the session read gave it: the chain input the
/// live edit cascades over.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveStepWindow {
    pub step_id: i64,
    pub start: Option<String>,
    pub end: Option<String>,
}

/// One command on a saved step, as this draft now has it.
///
/// `id` is the server's id for a command already on the step, or negative for
/// one this draft adds. `units` is the WHOLE audience, because the route
/// replaces the list rather than applying a delta.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveSubstepDraft {
    pub id: i64,
    pub content: String,
    pub units: Vec<i64>,
    /// Whether the operator changed this command. A command seeded from the
    /// session is `false`; it is shown and counted, but not written back.
    pub dirty: bool,
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
    /// Edits to steps the server holds, by `(scenario, step)`. A saved step is
    /// rewritten in place, never re-created.
    step_edits: BTreeMap<(i64, i64), StepEdit>,
    /// Command lists the operator has touched on saved steps, by
    /// `(scenario, step)`. The whole list is held from the first touch, so a
    /// move between commands can keep one hull per command per step.
    live_substeps: BTreeMap<(i64, i64), Vec<LiveSubstepDraft>>,
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
        // A scenario that never posts takes its steps' sub-steps with it.
        if let Some(scenario) = self.news.get(&local_id) {
            for step in &scenario.steps {
                self.substeps.remove(&step.local_id);
            }
        }
        self.news.remove(&local_id).is_some()
    }

    /// HOW LONG A NEW STEP RUNS when it inherits its predecessor's end.
    pub const STEP_MINUTES: i64 = 30;

    /// THE WINDOW AFTER A GIVEN ONE: it begins where the predecessor ended and
    /// runs [`Self::STEP_MINUTES`]. With no predecessor the noon window
    /// `+ STEP` has always opened with stands.
    ///
    /// Public because a REOPENED book's last step is in the session read, not
    /// in this draft: the shell knows the window and asks the same rule.
    pub fn window_after(last: Option<(&str, &str)>) -> (String, String) {
        last.and_then(|(_, end)| hhmm_minutes(end))
            .map(|end| (hhmm_at(end), hhmm_at(end + Self::STEP_MINUTES)))
            .unwrap_or_else(|| ("1200".to_string(), "1230".to_string()))
    }

    /// THE WINDOW A STEP ADDED NOW OPENS WITH.
    ///
    /// A new step follows the last one: it begins where the last step ended and
    /// runs [`Self::STEP_MINUTES`]. The first step of an empty book keeps the
    /// noon window `+ STEP` has always opened with, because there is no
    /// predecessor to inherit from.
    pub fn next_step_window(&self, scenario_id: i64) -> (String, String) {
        let last = self
            .news
            .get(&scenario_id)
            .and_then(|scenario| scenario.steps.last())
            .and_then(|step| step.start.as_deref().zip(step.end.as_deref()));
        Self::window_after(last)
    }

    /// The mutable list holding a staged step, whether it belongs to a
    /// staged-new scenario or to a step-add on a server-held one.
    fn steps_mut_containing(&mut self, step_local: i64) -> Option<&mut Vec<StagedStep>> {
        if let Some(scenario) = self
            .news
            .values_mut()
            .find(|scenario| scenario.steps.iter().any(|step| step.local_id == step_local))
        {
            return Some(&mut scenario.steps);
        }
        self.step_adds
            .values_mut()
            .find(|steps| steps.iter().any(|step| step.local_id == step_local))
    }

    /// SET ONE END OF A STEP'S WINDOW, AND SHIFT THE STEPS BEHIND IT.
    ///
    /// A window edit is a move of the tail, not a second disconnected window:
    /// each following step begins where the one before it ended and keeps its
    /// own duration, all the way down the book. Editing the START moves only
    /// this step's own length — the chain behind it is anchored to its END,
    /// which has not moved.
    ///
    /// Answers whether the step was found. A malformed stamp is ignored rather
    /// than written: the picker only offers `HH:MM`, and a draft is not the
    /// place to discover that one end is not a time.
    pub fn set_step_time(&mut self, step_local: i64, start: bool, value: &str) -> bool {
        let value = value.replace(':', "");
        if !crate::view::hhmm_ok(&value) {
            return false;
        }
        let Some(steps) = self.steps_mut_containing(step_local) else {
            return false;
        };
        let Some(at) = steps
            .iter()
            .position(|step| step.local_id == step_local)
        else {
            return false;
        };
        if start {
            steps[at].start = Some(value);
        } else {
            steps[at].end = Some(value);
        }
        let Some(mut previous_end) = steps[at].end.clone() else {
            return true;
        };
        for step in steps.iter_mut().skip(at + 1) {
            let (Some(from), Some(to)) = (step.start.as_deref(), step.end.as_deref()) else {
                // A step with no window has nothing to keep; the chain picks
                // up again at the next step that has one.
                continue;
            };
            let (Some(from_at), Some(to_at)) = (hhmm_minutes(from), hhmm_minutes(to)) else {
                continue;
            };
            let mut duration = to_at - from_at;
            if duration < 0 {
                duration += 24 * 60;
            }
            let Some(start_at) = hhmm_minutes(&previous_end) else {
                continue;
            };
            let end_at = start_at + duration;
            step.start = Some(hhmm_at(start_at));
            step.end = Some(hhmm_at(end_at));
            previous_end = hhmm_at(end_at);
        }
        true
    }

    /// SET ONE END OF A SAVED STEP'S WINDOW, AND SHIFT THE TAIL AFTER IT.
    ///
    /// The same rule as [`Self::set_step_time`], over a chain that begins with
    /// the server's steps and ends with the steps this draft has appended to
    /// the same scenario. The saved ends are an argument because the book draft
    /// does not hold them — the session read does.
    ///
    /// A saved step with no window has nothing to shift and the UI offers no
    /// picker for it; answered `false` here rather than inventing a window.
    pub fn set_live_step_time(
        &mut self,
        scenario_id: i64,
        step_id: i64,
        start: bool,
        value: &str,
        live: &[LiveStepWindow],
    ) -> bool {
        let value = value.replace(':', "");
        if !crate::view::hhmm_ok(&value) {
            return false;
        }
        // The chain, in book order: the saved steps with earlier edits laid
        // over them, then the additions this draft has staged.
        let mut chain: Vec<(i64, bool, Option<(String, String)>)> = Vec::new();
        for step in live {
            let edit = self.step_edits.get(&(scenario_id, step.step_id));
            let from = edit
                .and_then(|edit| edit.start.clone())
                .or_else(|| step.start.clone());
            let to = edit
                .and_then(|edit| edit.end.clone())
                .or_else(|| step.end.clone());
            chain.push((step.step_id, true, from.zip(to)));
        }
        for step in self
            .step_adds
            .get(&scenario_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            chain.push((
                step.local_id,
                false,
                step.start.clone().zip(step.end.clone()),
            ));
        }
        let Some(at) = chain.iter().position(|(id, _, _)| *id == step_id) else {
            return false;
        };
        let Some((mut from, mut to)) = chain[at].2.clone() else {
            return false;
        };
        if start {
            from = value;
        } else {
            to = value;
        }
        chain[at].2 = Some((from.clone(), to.clone()));
        self.store_live_window(scenario_id, step_id, &from, &to);

        let mut previous_end = to;
        for (id, is_live, window) in chain.iter().skip(at + 1) {
            let Some((from, to)) = window else {
                continue;
            };
            let (Some(from_at), Some(to_at)) = (hhmm_minutes(from), hhmm_minutes(to)) else {
                continue;
            };
            let mut duration = to_at - from_at;
            if duration < 0 {
                duration += 24 * 60;
            }
            let Some(start_at) = hhmm_minutes(&previous_end) else {
                continue;
            };
            let end_at = start_at + duration;
            let (new_start, new_end) = (hhmm_at(start_at), hhmm_at(end_at));
            if *is_live {
                self.store_live_window(scenario_id, *id, &new_start, &new_end);
            } else if let Some(steps) = self.step_adds.get_mut(&scenario_id) {
                if let Some(step) = steps.iter_mut().find(|step| step.local_id == *id) {
                    step.start = Some(new_start.clone());
                    step.end = Some(new_end.clone());
                }
            }
            previous_end = new_end;
        }
        true
    }

    /// Make a saved step a beat, or an orders step again.
    pub fn set_live_step_kind(&mut self, scenario_id: i64, step_id: i64, kind: StepKind) {
        self.step_edits
            .entry((scenario_id, step_id))
            .or_default()
            .kind = Some(kind);
    }

    /// What this draft has staged for one saved step.
    pub fn live_step_edit(&self, scenario_id: i64, step_id: i64) -> Option<&StepEdit> {
        self.step_edits.get(&(scenario_id, step_id))
    }

    fn store_live_window(&mut self, scenario_id: i64, step_id: i64, start: &str, end: &str) {
        let edit = self
            .step_edits
            .entry((scenario_id, step_id))
            .or_default();
        edit.start = Some(start.to_string());
        edit.end = Some(end.to_string());
    }

    /// The saved step's commands as this draft now has them, if touched.
    pub fn live_substeps_of(&self, scenario_id: i64, step_id: i64) -> &[LiveSubstepDraft] {
        self.live_substeps
            .get(&(scenario_id, step_id))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Take the saved step's whole command list into the draft, before the
    /// first edit. The list is the operator's view — saved commands with their
    /// audiences, and any already staged adds — so a move between commands can
    /// enforce one hull per command per step.
    pub fn begin_live_substeps(
        &mut self,
        scenario_id: i64,
        step_id: i64,
        commands: Vec<LiveSubstepDraft>,
    ) {
        let slot = self
            .live_substeps
            .entry((scenario_id, step_id))
            .or_default();
        // MERGE, do not replace: the list may already hold a command this
        // draft added (or an edit to one), and the seed carries the server's
        // commands that are not in it yet.
        for command in commands {
            if !slot.iter().any(|existing| existing.id == command.id) {
                let mut command = command;
                // A SEED IS NOT WORK. Only a method that changes a command
                // marks it, so an untouched command is never written back.
                command.dirty = false;
                slot.push(command);
            }
        }
    }

    /// Add a command to a saved step. Answers its negative local id.
    pub fn stage_live_substep(
        &mut self,
        scenario_id: i64,
        step_id: i64,
        content: &str,
    ) -> i64 {
        let local = self.fresh_local();
        self.live_substeps
            .entry((scenario_id, step_id))
            .or_default()
            .push(LiveSubstepDraft {
                id: local,
                content: content.to_string(),
                units: Vec::new(),
                dirty: true,
            });
        local
    }

    /// Rewrite a saved command's text.
    pub fn set_live_substep_content(
        &mut self,
        scenario_id: i64,
        step_id: i64,
        substep_id: i64,
        content: &str,
    ) -> bool {
        let Some(target) = self
            .live_substeps
            .get_mut(&(scenario_id, step_id))
            .and_then(|list| list.iter_mut().find(|command| command.id == substep_id))
        else {
            return false;
        };
        target.content = content.to_string();
        target.dirty = true;
        true
    }

    /// Replace a saved command's whole audience, and move any hull the drop
    /// named out of the other commands of the SAME step — the server's one
    /// command per hull per step, kept before the press.
    pub fn set_live_substep_units(
        &mut self,
        scenario_id: i64,
        step_id: i64,
        substep_id: i64,
        units: Vec<i64>,
    ) -> bool {
        let Some(list) = self.live_substeps.get_mut(&(scenario_id, step_id)) else {
            return false;
        };
        for other in list.iter_mut().filter(|command| command.id != substep_id) {
            let before = other.units.len();
            other.units.retain(|unit| !units.contains(unit));
            if other.units.len() != before {
                // THE MOVE IS TWO WRITES: the command that gains the hull, and
                // the one it left. A command whose audience changed is work
                // even when the operator dropped somewhere else.
                other.dirty = true;
            }
        }
        let Some(target) = list.iter_mut().find(|command| command.id == substep_id) else {
            return false;
        };
        target.units = units;
        target.dirty = true;
        true
    }

    /// Withdraw a command this draft added to a saved step. A saved command is
    /// not removable here — that is a delete write this ladder does not plan.
    pub fn remove_live_substep(
        &mut self,
        scenario_id: i64,
        step_id: i64,
        substep_id: i64,
    ) -> bool {
        if substep_id > 0 {
            return false;
        }
        let Some(list) = self.live_substeps.get_mut(&(scenario_id, step_id)) else {
            return false;
        };
        let before = list.len();
        list.retain(|command| command.id != substep_id);
        list.len() != before
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
            // A step that never posts takes its staged links with it, and its
            // sub-steps, which cannot exist without the step.
            self.step_relations.remove(&(local_id, step_local));
            self.substeps.remove(&step_local);
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

    /// REMOVE A SUB-STEP the operator staged.
    ///
    /// A sub-step is staged only until the flush posts it, so dropping one is
    /// local and whole: no server row exists to delete, and the next plan
    /// simply does not name it. Answers whether it was there.
    pub fn drop_substep(&mut self, step_local: i64, substep_local: i64) -> bool {
        let Some(list) = self.substeps.get_mut(&step_local) else {
            return false;
        };
        let before = list.len();
        list.retain(|sub| sub.local_id != substep_local);
        let dropped = list.len() != before;
        if dropped && list.is_empty() {
            self.substeps.remove(&step_local);
        }
        dropped
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
            self.substeps.remove(&step_local);
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
            && self.substeps.is_empty()
            && self.step_edits.is_empty()
            && self.live_substeps.is_empty()
    }

    /// Drop staged ops against scenarios the server no longer lists, so
    /// a deleted scenario cannot hold orphaned steps in the draft. Cuts
    /// take their staged links with them: posting relations for a step
    /// being deleted would race its own removal.
    pub fn prune(&mut self, live_ids: &[i64]) {
        self.step_adds.retain(|sid, _| live_ids.contains(sid));
        self.step_cuts.retain(|(sid, _)| live_ids.contains(sid));
        // An edit to a scenario the server no longer lists is moot, and an edit
        // to a step the draft is cutting would race its own removal.
        self.step_edits.retain(|(sid, step), _| {
            live_ids.contains(sid) && !self.step_cuts.contains(&(*sid, *step))
        });
        // A saved command list follows the same rule as the step it belongs
        // to: a vanished scenario or a cut step takes it with it.
        self.live_substeps.retain(|(sid, step), _| {
            live_ids.contains(sid) && !self.step_cuts.contains(&(*sid, *step))
        });
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
        // A sub-step belongs to a step. One whose step the draft no longer
        // holds is an orphan and would never post, so it goes with the step.
        let mut staged_steps: Vec<i64> = self
            .news
            .values()
            .flat_map(|scenario| scenario.steps.iter().map(|step| step.local_id))
            .collect();
        staged_steps.extend(
            self.step_adds
                .values()
                .flatten()
                .map(|step| step.local_id),
        );
        self.substeps
            .retain(|step_local, _| staged_steps.contains(step_local));
    }

    /// Drop the whole draft: a released hold, a closed session, a hold
    /// that moved to another game.
    pub fn clear(&mut self) {
        self.news.clear();
        self.step_adds.clear();
        self.step_cuts.clear();
        self.step_relations.clear();
        self.substeps.clear();
        self.step_edits.clear();
        self.live_substeps.clear();
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
    /// Rewrite a step the server already holds:
    /// `PATCH .../scenarios/{scenario}/steps/{step}`. Absent fields are left
    /// alone — the draft carries only what the operator touched.
    EditStep {
        scenario_id: i64,
        step_id: i64,
        kind: Option<StepKind>,
        window: Option<(String, String)>,
    },
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
    /// Rewrite a command the server already holds:
    /// `PATCH .../steps/{step}/substeps/{substep}`.
    EditSubstep {
        scenario_id: i64,
        step_id: i64,
        substep_id: i64,
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
            Self::EditStep { step_id, .. } => format!("edit step {step_id}"),
            Self::SetRelated { refs, .. } => format!("relate {} post(s)", refs.len()),
            Self::AddSubstep { content, .. } => {
                let short: String = content.chars().take(40).collect();
                format!("add sub-step {short:?}")
            }
            Self::EditSubstep { substep_id, .. } => format!("edit sub-step {substep_id}"),
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
    // Saved steps the operator has rewritten. Before the cuts, so an edit and
    // a cut of the same step settle as the cut.
    for ((sid, step_id), edit) in &draft.step_edits {
        let Some(scenario) = live.iter().find(|scenario| scenario.id == *sid) else {
            continue;
        };
        if !scenario.steps.contains(step_id) || draft.is_cut(*sid, *step_id) {
            continue;
        }
        let window = edit.window();
        if edit.kind.is_none() && window.is_none() {
            continue;
        }
        writes.push(BookWrite::EditStep {
            scenario_id: *sid,
            step_id: *step_id,
            kind: edit.kind,
            window,
        });
    }
    // The commands on saved steps that the operator has touched. After the
    // step edits, because a new command names a step the game already holds;
    // before the cuts, for the same reason as the edits above.
    for ((sid, step_id), commands) in &draft.live_substeps {
        let Some(scenario) = live.iter().find(|scenario| scenario.id == *sid) else {
            continue;
        };
        if !scenario.steps.contains(step_id) || draft.is_cut(*sid, *step_id) {
            continue;
        }
        for command in commands {
            if command.id > 0 && !command.dirty {
                continue;
            }
            if command.id < 0 {
                writes.push(BookWrite::AddSubstep {
                    scenario: ScenarioRef::Live(*sid),
                    step: StepRef::Live(*step_id),
                    substep: command.id,
                    content: command.content.clone(),
                });
            } else {
                writes.push(BookWrite::EditSubstep {
                    scenario_id: *sid,
                    step_id: *step_id,
                    substep_id: command.id,
                    content: command.content.clone(),
                });
            }
            writes.push(BookWrite::SetAudience {
                scenario: ScenarioRef::Live(*sid),
                step: StepRef::Live(*step_id),
                substep: command.id,
                hulls: command.units.clone(),
                nodes: Vec::new(),
            });
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
                BookWrite::EditStep { .. } => "edit",
                BookWrite::SetRelated { .. } => "relate",
                BookWrite::AddSubstep { .. } => "addsub",
                BookWrite::EditSubstep { .. } => "editsub",
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
mod window_tests {
    use super::*;

    fn book() -> (BookDraft, i64) {
        let mut book = BookDraft::new();
        let scenario = book.stage_scenario("Harbour sweep");
        (book, scenario)
    }

    /// A NEW STEP FOLLOWS THE LAST ONE: it opens where the previous step ended
    /// and runs thirty minutes, so a book typed in order stays contiguous.
    #[test]
    fn a_new_step_starts_where_the_last_one_ended() {
        let (mut book, scenario) = book();
        let first = book
            .stage_step_new(scenario, "SWEEP", Some(("1200".into(), "1230".into())))
            .expect("a step");
        assert_eq!(
            book.next_step_window(scenario),
            ("1230".into(), "1300".into())
        );
        // The next step inherits the LAST step's end, not the first's.
        book.stage_step_new(scenario, "RECOVER", Some(book.next_step_window(scenario)))
            .expect("a step");
        assert_eq!(
            book.next_step_window(scenario),
            ("1300".into(), "1330".into())
        );
        assert_eq!(book.steps_mut_containing(first).map(|steps| steps.len()), Some(2));
    }

    /// An empty book still opens on the noon window `+ STEP` has always used.
    #[test]
    fn the_first_step_keeps_the_default_window() {
        let (book, scenario) = book();
        assert_eq!(book.next_step_window(scenario), ("1200".into(), "1230".into()));
    }

    /// EDITING A MIDDLE STEP MOVES THE TAIL: every following step begins where
    /// the one before it ends and keeps its own duration.
    #[test]
    fn editing_an_end_rechains_the_following_steps() {
        let (mut book, scenario) = book();
        let first = book
            .stage_step_new(scenario, "BRIEFING", Some(("0800".into(), "0820".into())))
            .unwrap();
        let second = book
            .stage_step_new(scenario, "TRANSIT", Some(("0820".into(), "0910".into())))
            .unwrap();
        let third = book
            .stage_step_new(scenario, "SWEEP", Some(("0930".into(), "1015".into())))
            .unwrap();
        // The second step now ends at 09:45 instead of 09:10: it grew by 35
        // minutes, and the third step moves with it, keeping its 45 minutes.
        assert!(book.set_step_time(second, false, "09:45"));
        let steps = &book.get_new(scenario).expect("scenario").steps;
        assert_eq!(steps[0].local_id, first);
        assert_eq!(steps[0].end.as_deref(), Some("0820"));
        assert_eq!(
            (steps[1].start.as_deref(), steps[1].end.as_deref()),
            (Some("0820"), Some("0945"))
        );
        assert_eq!(
            (steps[2].start.as_deref(), steps[2].end.as_deref()),
            (Some("0945"), Some("1030")),
            "the tail begins where the edited step ends, same duration"
        );
        assert_eq!(steps[2].local_id, third);
    }

    /// Editing a START changes this step's length only: the tail is anchored to
    /// the END, which has not moved.
    #[test]
    fn editing_a_start_leaves_the_tail_alone() {
        let (mut book, scenario) = book();
        let _first = book
            .stage_step_new(scenario, "BRIEFING", Some(("0800".into(), "0820".into())))
            .unwrap();
        let second = book
            .stage_step_new(scenario, "TRANSIT", Some(("0820".into(), "0910".into())))
            .unwrap();
        let _third = book
            .stage_step_new(scenario, "SWEEP", Some(("0910".into(), "1015".into())))
            .unwrap();
        assert!(book.set_step_time(second, true, "08:30"));
        let steps = &book.get_new(scenario).expect("scenario").steps;
        assert_eq!(steps[1].start.as_deref(), Some("0830"));
        assert_eq!(
            (steps[2].start.as_deref(), steps[2].end.as_deref()),
            (Some("0910"), Some("1015"))
        );
    }

    /// A SUB-STEP CAN BE REMOVED, and removing the last one takes the empty
    /// list with it.
    #[test]
    fn a_staged_substep_can_be_dropped() {
        let (mut book, scenario) = book();
        let step = book.stage_step_new(scenario, "SWEEP", None).unwrap();
        let sub = book.stage_substep(step, "TAKE STATION").unwrap();
        assert_eq!(book.substep_count(), 1);
        assert!(book.drop_substep(step, sub));
        assert_eq!(book.substep_count(), 0);
        assert!(book.substeps_of(step).is_empty());
        assert!(!book.drop_substep(step, sub), "a second removal is a no-op");
    }

    /// A SAVED STEP IS EDITABLE: the window shifts the saved tail, and the
    /// plan carries the change as a `PATCH`, not a re-create.
    #[test]
    fn editing_a_saved_step_moves_the_saved_tail() {
        let mut book = BookDraft::new();
        let live = vec![
            LiveStepWindow {
                step_id: 10,
                start: Some("1000".into()),
                end: Some("1030".into()),
            },
            LiveStepWindow {
                step_id: 11,
                start: Some("1100".into()),
                end: Some("1130".into()),
            },
        ];
        assert!(book.set_live_step_time(7, 10, false, "10:45", &live));
        assert_eq!(
            book.live_step_edit(7, 10).and_then(|edit| edit.window()),
            Some(("1000".into(), "1045".into()))
        );
        assert_eq!(
            book.live_step_edit(7, 11).and_then(|edit| edit.window()),
            Some(("1045".into(), "1115".into())),
            "the next saved step follows the edit, keeping its 30 minutes"
        );
        let writes = plan(
            &book,
            &[LiveScenario {
                id: 7,
                steps: vec![10, 11],
            }],
        );
        assert!(
            matches!(
                &writes[..],
                [BookWrite::EditStep { step_id: 10, .. }, BookWrite::EditStep { step_id: 11, .. }]
            ),
            "{writes:?}"
        );
    }

    /// A SAVED STEP'S KIND IS ONE `PATCH` TOO, and a step the server no longer
    /// lists is skipped rather than posted at a row that is not there.
    #[test]
    fn a_saved_kind_edit_is_one_write() {
        let mut book = BookDraft::new();
        book.set_live_step_kind(7, 10, StepKind::Beat);
        book.set_live_step_kind(7, 99, StepKind::Beat);
        let writes = plan(
            &book,
            &[LiveScenario {
                id: 7,
                steps: vec![10],
            }],
        );
        assert_eq!(
            writes,
            vec![BookWrite::EditStep {
                scenario_id: 7,
                step_id: 10,
                kind: Some(StepKind::Beat),
                window: None,
            }]
        );
    }

    /// A STAGED STEP AFTER A SAVED ONE MOVES WITH THE CHAIN, because the two
    /// are one book on screen and the server appends the staged one to it.
    #[test]
    fn a_saved_edit_moves_the_staged_step_after_it() {
        let mut book = BookDraft::new();
        let added = book.stage_step_add(
            7,
            "RECOVER",
            Some(("1130".into(), "1200".into())),
        );
        let live = vec![LiveStepWindow {
            step_id: 10,
            start: Some("1000".into()),
            end: Some("1030".into()),
        }];
        assert!(book.set_live_step_time(7, 10, false, "10:45", &live));
        let step = book
            .added_steps(7)
            .find(|step| step.local_id == added)
            .expect("the staged step");
        assert_eq!(
            (step.start.as_deref(), step.end.as_deref()),
            (Some("1045"), Some("1115")),
            "the staged tail follows the saved edit"
        );
    }

    /// Dropping a step drops the sub-steps that belonged to it.
    #[test]
    fn dropping_a_step_drops_its_substeps() {
        let (mut book, scenario) = book();
        let step = book.stage_step_new(scenario, "SWEEP", None).unwrap();
        book.stage_substep(step, "TAKE STATION").unwrap();
        assert!(book.drop_new_step(scenario, step));
        assert_eq!(book.substep_count(), 0);
        assert!(
            book.get_new(scenario)
                .is_some_and(|scenario| scenario.steps.is_empty()),
            "nothing of the step is left behind"
        );
    }
}

#[cfg(test)]
mod live_substep_tests {
    use super::*;

    fn live() -> Vec<LiveScenario> {
        vec![LiveScenario {
            id: 7,
            steps: vec![10],
        }]
    }

    /// A SAVED COMMAND IS REWRITTEN IN PLACE, and a new one is added to the
    /// same step: text through `EditSubstep`, audience through `SetAudience`.
    #[test]
    fn a_saved_command_can_be_rewritten_and_added_to() {
        let mut book = BookDraft::new();
        book.begin_live_substeps(
            7,
            10,
            vec![LiveSubstepDraft {
                id: 100,
                content: "OLD".into(),
                units: vec![351],
                dirty: false,
            }],
        );
        assert!(book.set_live_substep_content(7, 10, 100, "NEW"));
        assert!(book.set_live_substep_units(7, 10, 100, vec![351, 352]));
        let added = book.stage_live_substep(7, 10, "ADDED");
        assert!(added < 0);
        assert!(book.set_live_substep_units(7, 10, added, vec![367]));

        let writes = plan(&book, &live());
        assert!(
            writes.iter().any(|write| matches!(
                write,
                BookWrite::EditSubstep { substep_id: 100, content, .. } if content == "NEW"
            )),
            "{writes:?}"
        );
        assert!(
            writes.iter().any(|write| matches!(
                write,
                BookWrite::AddSubstep { substep, content, .. } if *substep == added && content == "ADDED"
            )),
            "{writes:?}"
        );
        assert!(
            writes.iter().any(|write| matches!(
                write,
                BookWrite::SetAudience { substep: 100, hulls, .. } if hulls == &vec![351, 352]
            )),
            "the saved command's whole audience is written"
        );
        assert!(
            writes.iter().any(|write| matches!(
                write,
                BookWrite::SetAudience { substep, hulls, .. } if *substep == added && hulls == &vec![367]
            )),
            "the new command's audience follows it"
        );
    }

    /// ONE COMMAND PER HULL PER STEP, kept locally: a hull moved to one
    /// command leaves the others in the same step.
    #[test]
    fn a_hull_moves_between_saved_commands_of_one_step() {
        let mut book = BookDraft::new();
        book.begin_live_substeps(
            7,
            10,
            vec![
                LiveSubstepDraft {
                    id: 100,
                    content: "A".into(),
                    units: vec![351, 352],
                    dirty: false,
                },
                LiveSubstepDraft {
                    id: 101,
                    content: "B".into(),
                    units: vec![367],
                    dirty: false,
                },
            ],
        );
        assert!(book.set_live_substep_units(7, 10, 101, vec![352, 367]));
        let commands = book.live_substeps_of(7, 10);
        assert_eq!(commands[0].units, vec![351]);
        assert_eq!(commands[1].units, vec![352, 367]);
    }

    /// A COMMAND THIS DRAFT ADDED CAN BE WITHDRAWN; one the server holds is a
    /// delete write this ladder does not plan.
    #[test]
    fn only_a_draft_added_command_is_removable() {
        let mut book = BookDraft::new();
        let added = book.stage_live_substep(7, 10, "NEW");
        assert!(book.remove_live_substep(7, 10, added));
        assert!(book.live_substeps_of(7, 10).is_empty());
        book.begin_live_substeps(
            7,
            10,
            vec![LiveSubstepDraft {
                id: 100,
                content: "OLD".into(),
                units: Vec::new(),
                dirty: false,
            }],
        );
        assert!(!book.remove_live_substep(7, 10, 100));
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
                BookWrite::EditSubstep { .. } => "editsub",
                BookWrite::SetAudience { .. } => "audience",
                BookWrite::RemoveStep { .. } => "remove",
                BookWrite::EditStep { .. } => "edit",
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
