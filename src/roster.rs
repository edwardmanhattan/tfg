//! The roster as the operator has it, before Minos has heard of it.
//!
//! STAGING A SEAT, CHANGING A ROLE AND UNSEATING ARE LOCAL ACTS. They used
//! to be one backend write each, fired the moment the operator clicked, so
//! a picker click against a user the server already held answered 409 and
//! a call sign the server had seen answered 400 — in the middle of the
//! operator's authoring, with no session built yet to explain it.
//!
//! What Minos still owns is the record. The draft is what the operator is
//! looking at; a stage advance is the moment the draft is written through,
//! and [`plan`] is the diff that says which writes that is. Nothing here
//! talks to the network, which is the point: the rules about what a legal
//! draft is are pure functions over plain data, so they are testable
//! without a session.
//!
//! THE ORDER OF [`plan`] IS SEATS, THEN ROLES, THEN UNSEATS. A force assign
//! names a commander who must already be a participant, so seats go first;
//! an unseat lands last, so nobody is briefly seatless while their hull is
//! still being handed over.

use std::collections::{BTreeMap, BTreeSet};

/// One seat the operator has staged: who, as what, under which sign.
///
/// `judge` is captured from the game-role mirror at stage time and carried
/// here only so the merged roster can badge the row before any write.
#[derive(Debug, Clone, PartialEq)]
pub struct StagedSeat {
    pub user_id: i64,
    pub user_name: String,
    pub role_id: i64,
    pub role_name: String,
    pub call_sign: String,
    pub judge: bool,
}

/// The operator's roster draft: staged seats plus staged removals of
/// server-held seats.
///
/// Keyed by user id: one seat per person per game, the same rule the
/// server enforces, so re-staging somebody updates their row rather than
/// doubling it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RosterDraft {
    seats: BTreeMap<i64, StagedSeat>,
    removed: BTreeSet<i64>,
}

impl RosterDraft {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stage somebody (or re-stage them with a new role): a removal mark
    /// for them is withdrawn, because the operator's latest word is that
    /// they are in.
    pub fn stage(&mut self, seat: StagedSeat) {
        self.removed.remove(&seat.user_id);
        self.seats.insert(seat.user_id, seat);
    }

    /// Take somebody out of the draft. A staged-new seat simply vanishes
    /// (no write); a server-held seat is marked for unseating at the
    /// advance, so `on_server` is the mirror's word, not a guess.
    pub fn remove(&mut self, user_id: i64, on_server: bool) {
        self.seats.remove(&user_id);
        if on_server {
            self.removed.insert(user_id);
        } else {
            self.removed.remove(&user_id);
        }
    }

    /// Withdraw a staged removal: the operator changed their mind and the
    /// server row stands.
    pub fn restore(&mut self, user_id: i64) -> bool {
        self.removed.remove(&user_id)
    }

    /// Retarget a staged seat's call sign (recovery for a taken sign).
    pub fn set_call_sign(&mut self, user_id: i64, call_sign: &str) -> bool {
        match self.seats.get_mut(&user_id) {
            Some(seat) => {
                seat.call_sign = call_sign.to_string();
                true
            }
            None => false,
        }
    }

    pub fn get(&self, user_id: i64) -> Option<&StagedSeat> {
        self.seats.get(&user_id)
    }

    pub fn contains(&self, user_id: i64) -> bool {
        self.seats.contains_key(&user_id)
    }

    pub fn is_removed(&self, user_id: i64) -> bool {
        self.removed.contains(&user_id)
    }

    pub fn is_empty(&self) -> bool {
        self.seats.is_empty() && self.removed.is_empty()
    }

    pub fn len(&self) -> usize {
        self.seats.len() + self.removed.len()
    }

    /// Every staged seat, in user-id order.
    pub fn staged(&self) -> impl Iterator<Item = &StagedSeat> {
        self.seats.values()
    }

    /// Every staged removal, in user-id order.
    pub fn removals(&self) -> impl Iterator<Item = &i64> {
        self.removed.iter()
    }

    /// Drop the whole draft: a released hold, a closed session, a hold
    /// that moved to another game.
    pub fn clear(&mut self) {
        self.seats.clear();
        self.removed.clear();
    }

    /// How many writes are still owed to the server.
    pub fn pending(&self, live: &[LiveSeat]) -> usize {
        plan(self, live).len()
    }
}

/// One seat as Minos holds it: the last thing the sync left behind.
///
/// This is the whole of what a diff needs out of the server — and, since the
/// judge side does not command, one thing more: whether the seat is on it.
/// A judge-side seat may not hold a command, and the advance has to know
/// that BEFORE it writes the force rather than after the server refuses it.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveSeat {
    pub user_id: i64,
    pub user_name: String,
    pub role_id: i64,
    /// Whether the seat sits on the judge side. Captured from the same read
    /// as the role, because the role id alone does not say which side it is.
    pub judge: bool,
}

/// One roster write the sync owes Minos. Each is one endpoint.
#[derive(Debug, Clone, PartialEq)]
pub enum RosterWrite {
    /// Seat somebody new. Carries the call sign, because Minos requires
    /// one and this is the only write that can set it.
    Seat {
        user_id: i64,
        user_name: String,
        role_id: i64,
        role_name: String,
        call_sign: String,
    },
    /// Move a seated person to another game role.
    Role {
        user_id: i64,
        user_name: String,
        role_id: i64,
        role_name: String,
    },
    /// Take somebody off the roster.
    Unseat { user_id: i64, user_name: String },
}

impl RosterWrite {
    pub fn user_id(&self) -> i64 {
        match self {
            Self::Seat { user_id, .. }
            | Self::Role { user_id, .. }
            | Self::Unseat { user_id, .. } => *user_id,
        }
    }

    /// Who this write is about, for a line that has to say whose seat
    /// held the whole advance up.
    pub fn label(&self) -> String {
        match self {
            Self::Seat { user_name, role_name, .. } => {
                format!("seat {user_name} as {role_name}")
            }
            Self::Role { user_name, role_name, .. } => {
                format!("move {user_name} to {role_name}")
            }
            Self::Unseat { user_name, .. } => format!("unseat {user_name}"),
        }
    }
}

/// The writes that carry `draft` onto a server holding `live`, in the
/// order the server accepts them.
///
/// A staged seat for somebody the server already holds is NOT a seat:
/// same role converges to nothing, a different role becomes a role
/// change. That is what turns a re-picked directory row from a 409 into
/// a no-op instead of a refusal in the middle of authoring.
///
/// THE ORDER IS SEATS, THEN ROLES, THEN UNSEATS, and the three are
/// gathered separately rather than interleaved as the draft is walked. The
/// draft is walked in user-id order, so an interleaved plan would write a
/// role change before a seat that happens to sort earlier — and the order
/// the module promises is the order the server accepts: a commander write
/// names a participant, so the seats go first.
pub fn plan(draft: &RosterDraft, live: &[LiveSeat]) -> Vec<RosterWrite> {
    let mut seats = Vec::new();
    let mut roles = Vec::new();
    for seat in draft.staged() {
        match live.iter().find(|l| l.user_id == seat.user_id) {
            None => seats.push(RosterWrite::Seat {
                user_id: seat.user_id,
                user_name: seat.user_name.clone(),
                role_id: seat.role_id,
                role_name: seat.role_name.clone(),
                call_sign: seat.call_sign.clone(),
            }),
            Some(held) if held.role_id != seat.role_id => roles.push(RosterWrite::Role {
                user_id: seat.user_id,
                user_name: seat.user_name.clone(),
                role_id: seat.role_id,
                role_name: seat.role_name.clone(),
            }),
            Some(_) => {}
        }
    }
    let mut unseats = Vec::new();
    for user_id in draft.removals() {
        if let Some(held) = live.iter().find(|l| l.user_id == *user_id) {
            unseats.push(RosterWrite::Unseat {
                user_id: held.user_id,
                user_name: held.user_name.clone(),
            });
        }
    }
    seats.extend(roles);
    seats.extend(unseats);
    seats
}

/// Whether a game role may need a fleet before the exercise can run.
///
/// TRUE for everything that is not judge-side, because that is all the
/// helpers mirror can say. There is no positive "requires a command" flag on
/// a game role — the schema carries `is_judge_side` and nothing else — so the
/// honest form of this question is "is this the judge side", and the answer
/// for the rest is a warning rather than a block.
///
/// A free function because the rule is pure and takes only the flag: the note
/// appears in two places (the seat form and the roster row) and one answer
/// served from two places is two answers waiting to disagree.
pub fn role_may_need_command(is_judge_side: bool) -> bool {
    !is_judge_side
}

/// The seats that may NOT command, by user id and by name.
///
/// The STAGED role wins over the live one, because the staged role is what
/// the advance is about to write: a person the server holds as a player and
/// whom the operator is moving to the judge side is judge-side the moment
/// the advance lands, and a person held as a judge and moved back to a
/// player role is a commander the moment the advance lands.
///
/// The name rides with the id because the refusal has to say WHICH person
/// cannot hold the command — "a judge-side seat may not command" is a rule
/// the operator cannot act on.
pub fn judge_seats(draft: &RosterDraft, live: &[LiveSeat]) -> BTreeMap<i64, String> {
    let mut judges: BTreeMap<i64, String> = BTreeMap::new();
    for seat in live {
        if seat.judge {
            judges.insert(seat.user_id, seat.user_name.clone());
        }
    }
    for seat in draft.staged() {
        if seat.judge {
            judges.insert(seat.user_id, seat.user_name.clone());
        } else {
            judges.remove(&seat.user_id);
        }
    }
    judges
}

/// A staged role change that moves a seat onto the judge side.
///
/// THE SERVER RELEASES THE HULLS. A judge-side participant may not command,
/// so the server takes the commands away rather than leaving them pointing
/// at somebody who may not hold them — the hulls go, and their placements
/// with them. The console has to know which staged changes are like that,
/// because the force it is about to write is the thing that goes.
#[derive(Debug, Clone, PartialEq)]
pub struct JudgeSideMove {
    pub user_id: i64,
    pub user_name: String,
    pub role_name: String,
}

/// The staged role changes that move a seat onto the judge side.
///
/// A staged seat for somebody the server already holds is a role change
/// when the role differs, and the judge's side is what the change says — so
/// the staged seat's own flag is the whole of the answer. A staged NEWCOMER
/// on the judge side is not a move: nobody holds anything yet, and there is
/// nothing to release.
pub fn judge_side_moves(draft: &RosterDraft, live: &[LiveSeat]) -> Vec<JudgeSideMove> {
    let mut moves = Vec::new();
    for seat in draft.staged() {
        if !seat.judge {
            continue;
        }
        let Some(held) = live.iter().find(|l| l.user_id == seat.user_id) else {
            continue;
        };
        if held.role_id == seat.role_id {
            continue;
        }
        moves.push(JudgeSideMove {
            user_id: seat.user_id,
            user_name: seat.user_name.clone(),
            role_name: seat.role_name.clone(),
        });
    }
    moves
}

/// A staged role change that would take a commander's hulls away.
///
/// The refusal is WHOLE: the console is about to write a force, and this
/// change would delete part of it on the server. Issuing the rest would
/// half-commit a plan the operator cannot see.
#[derive(Debug, Clone, PartialEq)]
pub struct JudgeSideHazard {
    /// Who commands the hulls.
    pub user_name: String,
    /// The role they are being moved to.
    pub role_name: String,
    /// The hulls they command, in the draft's order.
    pub hulls: Vec<String>,
}

impl std::fmt::Display for JudgeSideHazard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} commands {} \u{2014} moving them to {} would release the \
             hull from the exercise, because a judge-side seat directs, it \
             does not command",
            self.user_name,
            self.hulls.join(", "),
            self.role_name
        )
    }
}

/// The staged role change that would take a commander's hulls away, if there
/// is one.
///
/// `commanders` is who commands what as the FORCE draft has it — the state
/// the force writes leave the server in. A hull handed over in the same
/// advance is not this person's any more, and a hull dropped from the
/// exercise is not either; so the rule asks about the draft, which is the
/// truth about both.
pub fn judge_side_hazard(
    draft: &RosterDraft,
    live: &[LiveSeat],
    commanders: &BTreeMap<i64, Vec<String>>,
) -> Option<JudgeSideHazard> {
    for move_ in judge_side_moves(draft, live) {
        let Some(hulls) = commanders.get(&move_.user_id) else {
            continue;
        };
        if hulls.is_empty() {
            continue;
        }
        return Some(JudgeSideHazard {
            user_name: move_.user_name,
            role_name: move_.role_name,
            hulls: hulls.clone(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seat(user_id: i64, name: &str, role_id: i64) -> StagedSeat {
        StagedSeat {
            user_id,
            user_name: name.into(),
            role_id,
            role_name: format!("role-{role_id}"),
            call_sign: name.to_uppercase(),
            judge: false,
        }
    }

    fn live(user_id: i64, name: &str, role_id: i64) -> LiveSeat {
        LiveSeat { user_id, user_name: name.into(), role_id, judge: false }
    }

    /// The same live seat, but the server holds it on the judge side.
    fn judge_live(user_id: i64, name: &str, role_id: i64) -> LiveSeat {
        LiveSeat { user_id, user_name: name.into(), role_id, judge: true }
    }

    /// A staged seat, on whichever side the staged role sits.
    fn staged(user_id: i64, name: &str, role_id: i64, judge: bool) -> StagedSeat {
        StagedSeat {
            user_id,
            user_name: name.into(),
            role_id,
            role_name: format!("role-{role_id}"),
            call_sign: name.to_uppercase(),
            judge,
        }
    }

    #[test]
    fn a_staged_newcomer_is_one_seat() {
        let mut draft = RosterDraft::new();
        draft.stage(seat(6, "Budi", 1));
        assert_eq!(
            plan(&draft, &[]),
            vec![RosterWrite::Seat {
                user_id: 6,
                user_name: "Budi".into(),
                role_id: 1,
                role_name: "role-1".into(),
                call_sign: "BUDI".into(),
            }]
        );
    }

    #[test]
    fn a_repicked_seat_with_the_same_role_is_nothing() {
        // The 409 the picker used to produce: the server already holds
        // this person in this role, so the advance owes no write.
        let mut draft = RosterDraft::new();
        draft.stage(seat(6, "Budi", 1));
        assert_eq!(plan(&draft, &[live(6, "Budi", 1)]), Vec::new());
    }

    #[test]
    fn a_repicked_seat_with_another_role_is_a_role_change() {
        let mut draft = RosterDraft::new();
        draft.stage(seat(6, "Budi", 2));
        assert_eq!(
            plan(&draft, &[live(6, "Budi", 1)]),
            vec![RosterWrite::Role {
                user_id: 6,
                user_name: "Budi".into(),
                role_id: 2,
                role_name: "role-2".into(),
            }]
        );
    }

    #[test]
    fn a_staged_removal_of_a_held_seat_is_one_unseat() {
        let mut draft = RosterDraft::new();
        draft.remove(6, true);
        assert_eq!(
            plan(&draft, &[live(6, "Budi", 1)]),
            vec![RosterWrite::Unseat { user_id: 6, user_name: "Budi".into() }]
        );
    }

    #[test]
    fn a_removal_of_nobody_is_nothing() {
        let mut draft = RosterDraft::new();
        draft.remove(6, true);
        assert_eq!(plan(&draft, &[]), Vec::new());
    }

    #[test]
    fn removing_a_staged_newcomer_leaves_no_write() {
        let mut draft = RosterDraft::new();
        draft.stage(seat(6, "Budi", 1));
        draft.remove(6, false);
        assert_eq!(plan(&draft, &[]), Vec::new());
    }

    #[test]
    fn staging_withdraws_a_removal() {
        let mut draft = RosterDraft::new();
        draft.remove(6, true);
        draft.stage(seat(6, "Budi", 1));
        assert_eq!(
            plan(&draft, &[live(6, "Budi", 1)]),
            Vec::new(),
            "back in the same role: converged"
        );
    }

    #[test]
    fn seats_come_before_roles_before_unseats() {
        let mut draft = RosterDraft::new();
        draft.remove(9, true);
        draft.stage(seat(7, "Caca", 2));
        draft.stage(seat(6, "Budi", 1));
        let live = vec![live(6, "Budi", 9), live(9, "Dedi", 1)];
        let kinds: Vec<&str> = plan(&draft, &live)
            .iter()
            .map(|w| match w {
                RosterWrite::Seat { .. } => "seat",
                RosterWrite::Role { .. } => "role",
                RosterWrite::Unseat { .. } => "unseat",
            })
            .collect();
        assert_eq!(kinds, vec!["seat", "role", "unseat"]);
    }

    /// The only positive fact the helpers mirror carries about a game role
    /// is `is_judge_side`, so "may need a command" is exactly its negation.
    /// A test that says anything more would be asserting a flag the schema
    /// does not have.
    #[test]
    fn only_the_judge_side_is_known_not_to_need_a_command() {
        assert!(!role_may_need_command(true), "judge side never commands");
        assert!(role_may_need_command(false), "everything else may");
    }

    // -- the judge side ----------------------------------------------------
    //
    // A judge-side seat directs, it does not command, and the server takes
    // the commands away when a seat crosses onto that side. These are the
    // rules that keep the console from destroying the force it is about to
    // write.

    /// The staged role wins over the live one: the staged role is what the
    /// advance is about to write.
    #[test]
    fn a_staged_role_decides_who_is_on_the_judge_side() {
        let mut draft = RosterDraft::new();
        draft.stage(staged(6, "Budi", 2, true));
        let judges = judge_seats(&draft, &[live(6, "Budi", 1)]);
        assert!(judges.contains_key(&6), "the staged judge side is the one that lands");
        assert_eq!(judges.get(&6).map(String::as_str), Some("Budi"));

        let mut back = RosterDraft::new();
        back.stage(staged(6, "Budi", 1, false));
        let judges = judge_seats(&back, &[judge_live(6, "Budi", 2)]);
        assert!(
            !judges.contains_key(&6),
            "a seat moved back to the player side may command again"
        );
    }

    /// A staged role change onto the judge side is the move the server
    /// answers by releasing the seat's hulls.
    #[test]
    fn a_staged_role_change_onto_the_judge_side_is_a_judge_side_move() {
        let mut draft = RosterDraft::new();
        draft.stage(staged(6, "Budi", 2, true));
        let moves = judge_side_moves(&draft, &[live(6, "Budi", 1)]);
        assert_eq!(
            moves,
            vec![JudgeSideMove {
                user_id: 6,
                user_name: "Budi".into(),
                role_name: "role-2".into(),
            }]
        );
    }

    /// A newcomer staged on the judge side holds nothing, so there is
    /// nothing to release and nothing to refuse.
    #[test]
    fn a_newcomer_on_the_judge_side_is_not_a_move() {
        let mut draft = RosterDraft::new();
        draft.stage(staged(6, "Budi", 2, true));
        assert!(judge_side_moves(&draft, &[]).is_empty());
    }

    /// A seat the server already holds as a judge and is staged as the same
    /// role is not a move either: nothing changes.
    #[test]
    fn a_seat_staged_as_the_role_it_already_holds_is_not_a_move() {
        let mut draft = RosterDraft::new();
        draft.stage(staged(6, "Budi", 1, true));
        assert!(judge_side_moves(&draft, &[judge_live(6, "Budi", 1)]).is_empty());
    }

    /// The hazard: a seat moved onto the judge side while it still commands
    /// a hull. The server would release the hull, so the console refuses
    /// the whole advance and names the hull.
    #[test]
    fn a_commander_moved_onto_the_judge_side_is_refused_whole() {
        let mut draft = RosterDraft::new();
        draft.stage(staged(6, "Budi", 2, true));
        let commanders = BTreeMap::from([(6, vec!["KRI Nanggala".to_string()])]);
        let hazard = judge_side_hazard(&draft, &[live(6, "Budi", 1)], &commanders)
            .expect("the move would release the hull");
        assert_eq!(hazard.hulls, vec!["KRI Nanggala".to_string()]);
        assert!(
            hazard.to_string().contains("would release the hull"),
            "the sentence names the fact: {hazard}"
        );
    }

    /// A hull handed over in the same advance is not the commander's any
    /// more, so the role change releases nothing and the advance may go.
    #[test]
    fn a_hull_handed_over_in_the_same_advance_is_not_a_hazard() {
        let mut draft = RosterDraft::new();
        draft.stage(staged(6, "Budi", 2, true));
        let commanders = BTreeMap::from([(9, vec!["KRI Nanggala".to_string()])]);
        assert!(judge_side_hazard(&draft, &[live(6, "Budi", 1)], &commanders).is_none());
    }
}
