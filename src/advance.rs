//! The advance, as a plan: what it writes, in what order, and what refuses
//! it whole.
//!
//! LEAVE PLANNING IS ONE DECISION, AND THE DECISION HAS AN ORDER. The force,
//! the roster, the task organisation and the exercise's own terms are
//! written through in one flush and the game moves afterwards, so the ORDER
//! of the writes is load-bearing rather than tidy. This module is where
//! that order is written down, as data, so it can be read and tested
//! without a session — the binary executes the steps this module lays out.
//!
//! THE ORDER, AND WHY EACH ONE IS WHERE IT IS.
//!
//! - THE WINDOW FIRST. The planned window is the exercise's own terms, and
//!   Minos accepts a change to the terms only while the game is in
//!   `planning` — so it has to be written before the transition, and it is
//!   written before everything else because it is the one write that needs a
//!   grant the others do not: `PATCH /games/{id}` is the administrator's
//!   ticket. A seat without that ticket learns it here, before the seats,
//!   the force and the groups have been spent.
//! - SEATS BEFORE THE FORCE. A commander write names a participant, and the
//!   console's flow is local-first: a player staged a minute ago commands a
//!   hull staged a minute ago, and neither exists on the server until this
//!   moment.
//! - THE FORCE BEFORE THE ROLE CHANGES. A role change onto the judge side
//!   RELEASES the hulls that seat commands — the server takes the commands
//!   away rather than leaving them pointing at somebody who may not hold
//!   them. So the role change has to land after the force has given those
//!   commands to whoever the operator said should have them.
//! - UNSEATS AFTER THE FORCE, for the same reason the force comes first:
//!   nobody is briefly seatless while their hull is still being handed
//!   over.
//! - THE BOOK LAST. A step's related list names hulls and task-organisation
//!   nodes, so the force and the task organisation have to exist before the
//!   book is posted. A new scenario's steps come after the scenario, and a
//!   removed step comes after the scenario it is removed from.
//! - THE TRANSITION LAST. A refused flush refuses the advance, and the goal
//!   is forgotten rather than retried implicitly: the exercise is still in
//!   `planning` and the operator can fix the plan and go again.
//!
//! EVERY RULE IS CHECKED BEFORE THE FIRST WRITE. The rules are the force's
//! (`force::plan`), the room's (a judge-side seat commands nothing) and the
//! roster's (a role change onto the judge side releases the hulls that seat
//! commands). A plan that breaks one of them refuses WHOLE, because
//! issuing the rest would half-commit a force the operator cannot see.

use std::collections::BTreeMap;

use crate::backend::{GameUpdate, TimeWindow};
use crate::force::{ForceDraft, ForceGap, ForceWrite, LiveHull};
use crate::groups::GroupWrite;
use crate::roster::{JudgeSideHazard, LiveSeat, RosterDraft, RosterWrite};
use crate::window::WindowDraft;

/// One thing the advance does. Each step is one endpoint, and the order the
/// steps come in is the order the server accepts them in.
///
/// NOT `PartialEq`: the window step carries a `GameUpdate`, and that is the
/// backend's type to derive or not to derive. The steps are compared by
/// their KIND, which is what the order is about.
#[derive(Debug, Clone)]
pub enum Step {
    /// The exercise's terms: the planned window. One `PATCH /games/{id}`.
    Window(GameUpdate),
    /// Seat somebody new.
    Seat(RosterWrite),
    /// One force write: an assignment, a handover, a placement, an
    /// embarkation, a removal.
    Force(ForceWrite),
    /// Move a seated person to another game role.
    Role(RosterWrite),
    /// Take somebody off the roster.
    Unseat(RosterWrite),
    /// One task-organisation write.
    ///
    /// The whole task organisation is ONE step, not one step per write: the
    /// writes name each other (a node's parent, the hull in it), and a node
    /// this flush creates only has a server id once it has been created. One
    /// step keeps that map inside the flush that builds it.
    Groups(Vec<GroupWrite>),
    /// The scenario book, whole.
    ///
    /// LAST, because a step's related list names hulls AND task-organisation
    /// nodes: the force and the groups have to exist before the book is
    /// posted. A new scenario's steps come after the scenario, and a removed
    /// step comes after the scenario it is removed from.
    Book(Vec<crate::book::BookWrite>),
    /// Move the exercise to preparation. The last thing, and the only one
    /// that changes what every participant may see.
    Transition,
}

/// Why the advance refuses whole, in the console's words.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// The force breaks a rule the server enforces.
    Force(ForceGap),
    /// A staged role change would take a commander's hulls away.
    Stripped(JudgeSideHazard),
    /// The window is not a window.
    Window(String),
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Force(gap) => write!(f, "{gap}"),
            Self::Stripped(hazard) => write!(f, "{hazard}"),
            Self::Window(why) => write!(f, "{why}"),
        }
    }
}

/// Who commands what, as the force draft has it.
///
/// The state the force writes leave the server in: a hull handed over in
/// this advance is its new commander's, and a hull dropped from the
/// exercise commands nobody.
fn commanders(draft: &ForceDraft) -> BTreeMap<i64, Vec<String>> {
    let mut map: BTreeMap<i64, Vec<String>> = BTreeMap::new();
    for hull in draft.hulls() {
        if let Some(commander) = hull.commander_id {
            map.entry(commander).or_default().push(hull.name.clone());
        }
    }
    map
}

/// The window write the advance owes, from the operator's draft against what
/// the server holds.
///
/// `Err` is the sentence for a draft that is not a window. `Ok(None)` is a
/// draft that says nothing, or a window the server already holds — and
/// NOTHING is written in that case, which is the point: the server merges
/// the clock keys one at a time and checks each against the start already
/// on the game, so a write that restates a window the server holds is a
/// write that can be refused for a reason the operator cannot see.
///
/// ONLY THE CHANGED KEYS GO. The four clock keys are independent on the
/// wire, and an operator who fixes one end of the exercise clock must not
/// have to restate the real clock to do it.
///
/// PACE IS NOT CARRIED ALONG, and the reason is the server's own: `fast`
/// waives the readiness DECLARATION and nothing else — the window, the
/// pieces, the placements and the fighters all still apply under it. So it
/// is not an answer to a window the console cannot satisfy, and setting it
/// quietly would take the exercise's ceremony away from an operator who
/// never asked for it. The pace is the Game Master's to choose, and it is
/// chosen in Control, where the console can show what it does.
pub fn window_update(
    draft: &WindowDraft,
    live: &TimeWindow,
) -> Result<Option<GameUpdate>, String> {
    if let Some(why) = draft.problem() {
        return Err(why);
    }
    let mut window = TimeWindow::default();
    let (actual, assumed) = draft.pairs();
    if let Some((date, start, end)) = actual {
        // The date is the same on both ends, so "the end is after the
        // start" is still a question about the two military times — and the
        // rule that says so is `crate::window`'s.
        //
        // The FRAME is the draft's: the offset the server stated when the
        // console read the window, or the client's own when it stated
        // none. Writing in any other frame writes a DIFFERENT instant.
        let start = crate::backend::actual_to_rfc3339(date, start, &draft.offset);
        let end = crate::backend::actual_to_rfc3339(date, end, &draft.offset);
        let (Some(start), Some(end)) = (start, end) else {
            return Err(format!("{date} is not a day like 2026-11-01"));
        };
        // COMPARED AS INSTANTS, not as strings. The server renders in its
        // own zone and the console writes in the frame it read, so two
        // spellings of one instant can differ as strings while being the
        // same moment. Comparing the strings would report a change the
        // operator never made, and the write that followed would move the
        // exercise by the difference between the two spellings.
        if !crate::backend::same_instant(live.actual_start.as_deref().unwrap_or(""), &start) {
            window.actual_start = Some(start);
        }
        if !crate::backend::same_instant(live.actual_end.as_deref().unwrap_or(""), &end) {
            window.actual_end = Some(end);
        }
    }
    if let Some((start, end)) = assumed {
        let start = crate::backend::assumed_hhmm_to_rfc3339(start, &draft.offset);
        let end = crate::backend::assumed_hhmm_to_rfc3339(end, &draft.offset);
        let (Some(start), Some(end)) = (start, end) else {
            return Err("the exercise clock is not a pair of military times".to_string());
        };
        if !crate::backend::same_instant(live.assumed_start.as_deref().unwrap_or(""), &start) {
            window.assumed_start = Some(start);
        }
        if !crate::backend::same_instant(live.assumed_end.as_deref().unwrap_or(""), &end) {
            window.assumed_end = Some(end);
        }
    }
    if window.is_empty() {
        return Ok(None);
    }
    Ok(Some(GameUpdate { window, ..Default::default() }))
}

/// The advance's steps, in the order the server takes them.
///
/// `window` is the window write, or `None` when the operator wrote no
/// window or the server already holds it. Every rule is checked before the
/// first step, so a plan that breaks one refuses WHOLE.
pub fn plan(
    force: &ForceDraft,
    live_hulls: &[LiveHull],
    roster: &RosterDraft,
    live_seats: &[LiveSeat],
    groups: &[GroupWrite],
    book: &[crate::book::BookWrite],
    window: Option<GameUpdate>,
) -> Result<Vec<Step>, Refusal> {
    // THE FORCE'S OWN RULES: a hull with nobody to steer it, one person
    // given two hulls, a hull riding itself, a ring of hulls.
    let force_writes = crate::force::plan(force, live_hulls).map_err(Refusal::Force)?;
    // THE ROSTER'S RULE, FIRST OF THE TWO THAT ASK ABOUT PEOPLE: a role
    // change onto the judge side releases the hulls that seat commands, so
    // it is refused while the force is still whole.
    //
    // IT IS NAMED BEFORE THE JUDGE-SIDE COMMANDER, because they are the
    // same mistake seen from two sides and the ROLE CHANGE is the one the
    // operator made. "Moving Budi to Referee would release the hull" tells
    // them what to undo; "Budi is judge-side" tells them what they already
    // did.
    let hazard = crate::roster::judge_side_hazard(roster, live_seats, &commanders(force));
    if let Some(hazard) = hazard {
        return Err(Refusal::Stripped(hazard));
    }
    // THE ROOM'S RULE: a judge-side seat commands nothing, and the server
    // refuses the assignment outright. This one catches the seat that was
    // ALREADY judge-side — nobody's edit, just a hand-over to a person who
    // may not steer.
    let judges = crate::roster::judge_seats(roster, live_seats);
    if let Some(gap) = crate::force::judge_side_gap(force, &judges) {
        return Err(Refusal::Force(gap));
    }

    let roster_writes = crate::roster::plan(roster, live_seats);
    let mut steps = Vec::new();
    if let Some(window) = window {
        steps.push(Step::Window(window));
    }
    for write in roster_writes.iter().filter(|w| matches!(w, RosterWrite::Seat { .. })) {
        steps.push(Step::Seat(write.clone()));
    }
    for write in force_writes {
        steps.push(Step::Force(write));
    }
    for write in roster_writes.iter().filter(|w| matches!(w, RosterWrite::Role { .. })) {
        steps.push(Step::Role(write.clone()));
    }
    for write in roster_writes.iter().filter(|w| matches!(w, RosterWrite::Unseat { .. })) {
        steps.push(Step::Unseat(write.clone()));
    }
    if !groups.is_empty() {
        steps.push(Step::Groups(groups.to_vec()));
    }
    if !book.is_empty() {
        steps.push(Step::Book(book.to_vec()));
    }
    steps.push(Step::Transition);
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::force::DraftHull;
    use crate::roster::{LiveSeat, StagedSeat};

    fn hull(unit_id: i64, name: &str, commander: Option<i64>) -> DraftHull {
        let mut h = DraftHull::new(unit_id, name.into(), format!("h{unit_id}"), "c".into());
        h.commander_id = commander;
        h
    }

    fn seat(user_id: i64, name: &str, role_id: i64, judge: bool) -> StagedSeat {
        StagedSeat {
            user_id,
            user_name: name.into(),
            role_id,
            role_name: format!("role-{role_id}"),
            call_sign: name.to_uppercase(),
            judge,
        }
    }

    fn live_seat(user_id: i64, name: &str, role_id: i64, judge: bool) -> LiveSeat {
        LiveSeat {
            user_id,
            user_name: name.into(),
            role_id,
            role_name: format!("role-{role_id}"),
            call_sign: name.to_uppercase(),
            judge,
        }
    }

    fn window() -> GameUpdate {
        GameUpdate {
            window: TimeWindow {
                actual_start: Some("2026-11-01T08:00:00Z".into()),
                actual_end: Some("2026-11-01T16:00:00Z".into()),
                assumed_start: Some("2000-01-01T08:00:00Z".into()),
                assumed_end: Some("2000-01-01T16:00:00Z".into()),
            },
            ..Default::default()
        }
    }

    /// The window is the exercise's own terms, and Minos accepts a change
    /// to the terms only while the game is in `planning` — so it is written
    /// BEFORE the transition, and before everything else, because it is the
    /// one write that needs the administrator's ticket.
    #[test]
    fn the_window_is_written_before_the_transition() {
        let mut draft = ForceDraft::new();
        draft.upsert(hull(7, "KRI Nanggala", Some(3)));
        let steps = plan(
            &draft,
            &[],
            &RosterDraft::new(),
            &[],
            &[],
            &[],
            Some(window()),
        )
        .expect("a plan");
        let kinds: Vec<&str> = steps
            .iter()
            .map(|s| match s {
                Step::Window(_) => "window",
                Step::Seat(_) => "seat",
                Step::Force(_) => "force",
                Step::Role(_) => "role",
                Step::Unseat(_) => "unseat",
                Step::Groups(_) => "group",
                Step::Book(_) => "book",
                Step::Transition => "transition",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["window", "force", "transition"],
            "the window comes first and the transition last"
        );
    }

    /// A seat comes before the force, because a commander write names a
    /// participant; a role change comes after the force, because a role
    /// change onto the judge side releases the hulls that seat commands.
    #[test]
    fn a_seat_comes_before_the_force_and_a_role_change_after_it() {
        let mut force = ForceDraft::new();
        force.upsert(hull(7, "KRI Nanggala", Some(3)));
        let mut roster = RosterDraft::new();
        roster.stage(seat(9, "Caca", 2, false));
        roster.stage(seat(6, "Budi", 3, true));
        let live = vec![live_seat(6, "Budi", 1, false)];
        let steps = plan(&force, &[], &roster, &live, &[], &[], None).expect("a plan");
        let kinds: Vec<&str> = steps
            .iter()
            .map(|s| match s {
                Step::Window(_) => "window",
                Step::Seat(_) => "seat",
                Step::Force(_) => "force",
                Step::Role(_) => "role",
                Step::Unseat(_) => "unseat",
                Step::Groups(_) => "group",
                Step::Book(_) => "book",
                Step::Transition => "transition",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["seat", "force", "role", "transition"],
            "the seat lands before the hull that names it, the role change after"
        );
    }

    /// A staged role change onto the judge side would release the hull that
    /// seat commands, so the advance refuses WHOLE — before any write, so
    /// the force it was about to write is never destroyed.
    #[test]
    fn a_role_change_onto_the_judge_side_refuses_the_whole_advance() {
        let mut force = ForceDraft::new();
        force.upsert(hull(7, "KRI Nanggala", Some(6)));
        let mut roster = RosterDraft::new();
        roster.stage(seat(6, "Budi", 2, true));
        let live = vec![live_seat(6, "Budi", 1, false)];
        let refusal = plan(&force, &[], &roster, &live, &[], &[], None)
            .expect_err("the role change would release the hull");
        match refusal {
            Refusal::Stripped(hazard) => {
                assert_eq!(hazard.hulls, vec!["KRI Nanggala".to_string()]);
            }
            other => panic!("expected the stripped hazard, got {other:?}"),
        }
    }

    /// A commander given two hulls refuses the whole advance, before any
    /// write.
    #[test]
    fn a_commander_given_two_hulls_refuses_the_whole_advance() {
        let mut force = ForceDraft::new();
        force.upsert(hull(7, "KRI Nanggala", Some(3)));
        force.upsert(hull(8, "KRI Rangsat", Some(3)));
        let refusal = plan(&force, &[], &RosterDraft::new(), &[], &[], &[], None)
            .expect_err("one person commands one thing");
        assert!(matches!(refusal, Refusal::Force(_)), "{refusal:?}");
    }

    /// A judge-side seat may not command, so the advance refuses the whole
    /// thing rather than let the server refuse it halfway.
    #[test]
    fn a_judge_side_commander_refuses_the_whole_advance() {
        let mut force = ForceDraft::new();
        force.upsert(hull(7, "KRI Nanggala", Some(6)));
        let roster = RosterDraft::new();
        let live = vec![live_seat(6, "Budi", 1, true)];
        let refusal = plan(&force, &[], &roster, &live, &[], &[], None)
            .expect_err("a judge commands nothing");
        assert!(matches!(refusal, Refusal::Force(_)), "{refusal:?}");
    }

    /// The window write carries only what changed: the server merges the
    /// clock keys one at a time, so an operator who fixes one end must not
    /// have to restate the other clock.
    #[test]
    fn the_window_write_carries_only_the_changed_keys() {
        let mut draft = WindowDraft {
            actual_date: "2026-11-01".into(),
            actual_start: "0800".into(),
            actual_end: "1600".into(),
            assumed_start: "0800".into(),
            assumed_end: "1600".into(),
            offset: "+00:00".into(),
        };
        let live = TimeWindow {
            actual_start: Some("2026-11-01T08:00:00Z".into()),
            actual_end: Some("2026-11-01T16:00:00Z".into()),
            assumed_start: Some("2000-01-01T08:00:00Z".into()),
            assumed_end: Some("2000-01-01T16:00:00Z".into()),
        };
        assert!(
            window_update(&draft, &live).expect("a window").is_none(),
            "a window the server already holds is not written again"
        );
        draft.assumed_end = "1700".into();
        let update = window_update(&draft, &live)
            .expect("a window")
            .expect("the end changed");
        assert_eq!(
            update.window.assumed_end.as_deref(),
            Some("2000-01-01T17:00:00+00:00"),
            "the write is in the frame the draft carries"
        );
        assert_eq!(update.window.actual_start, None, "the real clock is not restated");
        assert_eq!(update.window.assumed_start, None, "the start did not change");
    }

    /// THE WINDOW DOES NOT MOVE WHEN NOTHING CHANGED.
    ///
    /// The server renders in its own zone, so the strings it holds and the
    /// strings the console writes can differ while being the same moment.
    /// Comparing the STRINGS would report a change the operator never made,
    /// and the write that followed would move the exercise — and its clock —
    /// by the difference between the two spellings.
    ///
    /// This is the defect the live API proved: a window read back and
    /// written back moved seven hours, on both clocks, every time.
    #[test]
    fn an_unchanged_window_is_not_written_even_when_the_spelling_differs() {
        // The server's own zone, and the same instant in two spellings.
        let draft = WindowDraft {
            actual_date: "2026-11-01".into(),
            actual_start: "1500".into(),
            actual_end: "2300".into(),
            assumed_start: "1500".into(),
            assumed_end: "2300".into(),
            offset: "+07:00".into(),
        };
        let live = TimeWindow {
            actual_start: Some("2026-11-01T15:00:00+07:00".into()),
            actual_end: Some("2026-11-01T23:00:00+07:00".into()),
            assumed_start: Some("2000-01-01T15:00:00+07:00".into()),
            assumed_end: Some("2000-01-01T23:00:00+07:00".into()),
        };
        assert!(
            window_update(&draft, &live).expect("a window").is_none(),
            "the same instant in another spelling is not a change"
        );
        // And the same window spelled in UTC is the same instant too.
        let live_utc = TimeWindow {
            actual_start: Some("2026-11-01T08:00:00Z".into()),
            actual_end: Some("2026-11-01T16:00:00Z".into()),
            assumed_start: Some("2000-01-01T08:00:00Z".into()),
            assumed_end: Some("2000-01-01T16:00:00Z".into()),
        };
        let draft_utc = WindowDraft {
            actual_date: "2026-11-01".into(),
            actual_start: "0800".into(),
            actual_end: "1600".into(),
            assumed_start: "0800".into(),
            assumed_end: "1600".into(),
            offset: "+00:00".into(),
        };
        assert!(
            window_update(&draft_utc, &live_utc).expect("a window").is_none(),
            "and the same instant in the other zone is not a change either"
        );
    }

    /// A draft that is not a window is refused, and the sentence names the
    /// clock that is broken.
    #[test]
    fn a_broken_window_is_refused_before_anything_is_written() {
        let draft = WindowDraft {
            actual_date: "2026-11-01".into(),
            actual_start: "0800".into(),
            actual_end: "1600".into(),
            assumed_start: "1600".into(),
            assumed_end: "0800".into(),
            offset: "+00:00".into(),
        };
        let refusal = window_update(&draft, &TimeWindow::default())
            .expect_err("the exercise clock runs backwards");
        assert!(
            refusal.contains("the exercise clock"),
            "the sentence names the clock: {refusal}"
        );
    }
}
