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
/// This is the whole of what a diff needs out of the server.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveSeat {
    pub user_id: i64,
    pub user_name: String,
    pub role_id: i64,
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
pub fn plan(draft: &RosterDraft, live: &[LiveSeat]) -> Vec<RosterWrite> {
    let mut writes = Vec::new();
    for seat in draft.staged() {
        match live.iter().find(|l| l.user_id == seat.user_id) {
            None => writes.push(RosterWrite::Seat {
                user_id: seat.user_id,
                user_name: seat.user_name.clone(),
                role_id: seat.role_id,
                role_name: seat.role_name.clone(),
                call_sign: seat.call_sign.clone(),
            }),
            Some(held) if held.role_id != seat.role_id => writes.push(RosterWrite::Role {
                user_id: seat.user_id,
                user_name: seat.user_name.clone(),
                role_id: seat.role_id,
                role_name: seat.role_name.clone(),
            }),
            Some(_) => {}
        }
    }
    for user_id in draft.removals() {
        if let Some(held) = live.iter().find(|l| l.user_id == *user_id) {
            writes.push(RosterWrite::Unseat {
                user_id: held.user_id,
                user_name: held.user_name.clone(),
            });
        }
    }
    writes
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
        LiveSeat { user_id, user_name: name.into(), role_id }
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
}
