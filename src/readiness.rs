//! The execution gate: whether the server has said the exercise may run, and
//! what to say while it has not.
//!
//! A MOVE, not a redesign (ADR-0020, step 1). `ReadinessGap` and
//! `unknown_gate_blockers` were `main.rs`'s, and their five tests were pinned
//! there too. They are here because the rules are pure — classify a failed
//! read, and name what is missing without ever inventing "nothing is
//! outstanding" — and a rule that can only be checked by standing up a store
//! connection, a core map and a token is a rule that does not get checked.
//!
//! The server's own verdict (`GameReadinessView`, `blockers`, `can_execute`)
//! stays in `backend`; this module is only the CLIENT's answer for when there
//! is no verdict to read, which is the branch a deleted rule would quietly
//! come back in.

use crate::backend::BackendError;

/// Why the last readiness read produced no verdict.
///
/// THREE states, because "unknown" was two different situations and only one of
/// them was worth retrying. `Absent` is the one that matters: a server that
/// answers 404 does not publish the check at all, and "retry the sync" sends the
/// operator into a loop that cannot succeed. That is not a degraded read, it is
/// a client ahead of its server, and it is a fact about the SERVER rather than
/// about the connection, so it wants a different sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReadinessGap {
    /// Nothing read yet. Ordinary on first paint.
    #[default]
    NotRead,
    /// The read failed for a reason another attempt might fix.
    Unread,
    /// The server answered 404: it has no readiness route on this path.
    Absent,
}

impl ReadinessGap {
    /// Classify a failed read. A 404 is not a transport hiccup, and treating it
    /// as one is what produced impossible advice.
    ///
    /// Verified against the deployed dev API, which 404s on
    /// `/games/{id}/readiness` while every route beside it answers.
    pub fn of(e: &BackendError) -> Self {
        match e {
            BackendError::Api { status: 404, .. } => Self::Absent,
            _ => Self::Unread,
        }
    }
}

/// What to say when the gate is unknown.
///
/// NEVER an empty list. Empty means "nothing is outstanding", and that is the
/// one answer that must never be invented — this runs before the first bundle
/// lands and after a failed read.
///
/// The three states are three different facts, not one fact with three phrasings:
/// nothing has run yet, something failed and may work next time, or this server
/// does not have the endpoint and never will without a deploy. Only the middle
/// one earns the word "retry".
///
/// A free function because the rule is pure and the app is not, and because
/// this is the branch where a deleted rule would quietly come back: it is the
/// one place tempted to work the gate out from what this client happens to hold.
pub fn unknown_gate_blockers(gap: ReadinessGap) -> Vec<String> {
    vec![match gap {
        ReadinessGap::NotRead => "the readiness check has not run yet".to_string(),
        ReadinessGap::Unread => "the readiness check could not be read — retry the sync".to_string(),
        ReadinessGap::Absent => {
            "this server does not publish the readiness check".to_string()
        }
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The checklist is the SERVER'S WORDS. A client that rebuilt them would be
    /// a second copy of the gate, which is the thing this change exists to end
    /// — and the copy was already wrong once.
    #[test]
    fn the_checklist_passes_the_servers_words_through() {
        let view = crate::backend::GameReadinessView {
            can_execute: false,
            blockers: vec![
                "the planned window is incomplete (both actual_end and assumed_end are required)"
                    .to_string(),
                "2 of 3 unit(s) have not been placed on the map".to_string(),
            ],
            fast: false,
        };
        assert_eq!(view.blockers.len(), 2);
        assert!(!view.names_readiness(), "neither blocker is about readiness");
    }

    /// An unread or unrun gate must NEVER read as clear.
    ///
    /// Empty means "nothing is outstanding", and that is the one answer that
    /// must never be invented. This branch runs before the first bundle lands
    /// and after a failed read, so an empty list here would offer the advance
    /// on a game nobody has checked.
    #[test]
    fn an_unknown_gate_refuses_rather_than_reads_as_clear() {
        assert!(!unknown_gate_blockers(ReadinessGap::NotRead).is_empty(), "never run yet");
        assert!(!unknown_gate_blockers(ReadinessGap::Unread).is_empty(), "read failed");
    }

    /// A server that answers 404 does not publish the check, so the message
    /// must NOT tell the operator to retry. Verified against the deployed dev
    /// API, which 404s on `/games/{id}/readiness` while every route beside it
    /// answers: the client is ahead of its server, and a retry cannot fix that.
    #[test]
    fn an_absent_route_is_not_advice_to_retry() {
        let absent = crate::backend::BackendError::Api {
            status: 404,
            message: "Not Found".to_string(),
            detail: None,
        };
        let gap = ReadinessGap::of(&absent);
        assert_eq!(gap, ReadinessGap::Absent, "a 404 is not a transport hiccup");
        let msg = &unknown_gate_blockers(gap)[0];
        assert!(!msg.contains("retry"), "a retry cannot conjure the route: {msg}");
        assert!(msg.contains("does not publish"), "{msg}");
    }

    /// The other failures keep their advice, because another attempt can fix
    /// them. Only the 404 loses the word.
    #[test]
    fn a_transient_failure_still_earns_the_retry() {
        let other = crate::backend::BackendError::Api {
            status: 503,
            message: "unavailable".to_string(),
            detail: None,
        };
        assert_eq!(ReadinessGap::of(&other), ReadinessGap::Unread);
        assert!(unknown_gate_blockers(ReadinessGap::Unread)[0].contains("retry"));
    }

    /// The two unknown states say DIFFERENT things, because they are different
    /// events. "Has not run yet" is normal on first paint; "could not be read"
    /// is a degraded sync the operator may need to retry. Folding them into one
    /// string loses the only actionable half.
    #[test]
    fn an_unread_gate_says_so_and_names_the_remedy() {
        assert!(unknown_gate_blockers(ReadinessGap::Unread)[0].contains("retry"));
        assert!(!unknown_gate_blockers(ReadinessGap::NotRead)[0].contains("retry"));
    }
}
