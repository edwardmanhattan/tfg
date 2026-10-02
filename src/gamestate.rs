//! Where an exercise is in its lifecycle, as one value.
//!
//! This is the axis the whole console's layout hangs off. Before it existed
//! the same fact was carried by three overlapping representations in
//! `main.rs` — `Phase` (Setup / Live / Closed), `SimStage` (Planning /
//! Ready / Live / Eval) and `Option<(i64, String)>` for "no held game" —
//! which is why the shell's contents were spread across a phase bar, a
//! wizard and a dozen string comparisons against `"execution"`.
//!
//! A leaf module with no tfg dependencies, so `proto/p5-epaint` compiles it
//! by `#[path]` and runs its tests without building the MapLibre core. That
//! is the same reason `symbology` is a leaf: a vocabulary that can only be
//! checked by building a 2GB C++ dependency is a vocabulary nobody re-reads.
//!
//! See ADR-0008 for why the backend is authoritative over this value.

/// Where the exercise is, as ONE axis.
///
/// This replaces the three overlapping representations the shell used to
/// carry over the same fact: `Phase` (Setup / Live / Closed), `SimStage`
/// (Planning / Ready / Live / Eval) and `Option<(i64, String)>`
/// (`users_game`, where `None` meant "no session"). Three names for one
/// value is why the zone's contents were spread across a phase bar, a
/// wizard and a dozen `users_game_state == Some("execution")` comparisons.
///
/// `NoSession` is the one that did not exist as a value. It is the absence
/// of a held game, which the old shape could only express as an `Option`,
/// and an `Option` cannot be matched on alongside the states it excludes.
///
/// The four connected variants are the Minos `GameState` wire values
/// verbatim, because ADR-0008 makes the backend authoritative and a resync
/// projects them here. They are NOT the concept document's prose names:
/// the backend's constant for the steering mode is `maneuver` and its own
/// test lists `"dynamic"` as an invalid mode, so a client that echoes the
/// prose back gets a 400 on a value the specification published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameState {
    NoSession,
    Planning,
    Preparation,
    Execution,
    Closure,
}

impl GameState {
    /// The Minos wire value, or `None` for the client-only state.
    ///
    /// `None` is honest rather than a sentinel string: `NoSession` has no
    /// server counterpart, and sending one would be a request for a game
    /// that does not exist.
    pub fn wire(self) -> Option<&'static str> {
        match self {
            GameState::NoSession => None,
            GameState::Planning => Some("planning"),
            GameState::Preparation => Some("preparation"),
            GameState::Execution => Some("execution"),
            GameState::Closure => Some("closure"),
        }
    }

    /// Parse a Minos `GameState`. An unrecognised value reads as
    /// `NoSession`, never as a guess: a state the client invents would
    /// render a zone full of verbs the backend has not agreed to.
    pub fn from_wire(s: &str) -> Self {
        match s {
            "planning" => GameState::Planning,
            "preparation" => GameState::Preparation,
            "execution" => GameState::Execution,
            "closure" => GameState::Closure,
            _ => GameState::NoSession,
        }
    }

    /// Short caps, for an island's trailing note.
    pub fn label(self) -> &'static str {
        match self {
            GameState::NoSession => "NO SESSION",
            GameState::Planning => "PLANNING",
            GameState::Preparation => "PREPARATION",
            GameState::Execution => "EXECUTION",
            GameState::Closure => "CLOSURE",
        }
    }

    /// The roster and the force are editable only here. Mirrors Minos
    /// `GameRosterIsOpen`, so the picker buttons and the server agree about
    /// when a fleet may be edited.
    pub fn is_planning(self) -> bool {
        matches!(self, GameState::Planning | GameState::Preparation)
    }

    /// Whether this state shows the map's live clock and the orders
    /// surface.
    pub fn is_running(self) -> bool {
        matches!(self, GameState::Execution)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four connected states must round-trip through the wire values.
    /// This is the test that would have caught the client echoing the
    /// concept document's prose back at the server: `dynamic` is not a mode
    /// Minos accepts, and its own test lists it as invalid.
    #[test]
    fn every_connected_state_round_trips() {
        for s in [
            GameState::Planning,
            GameState::Preparation,
            GameState::Execution,
            GameState::Closure,
        ] {
            let wire = s.wire().expect("a connected state has a wire value");
            assert_eq!(GameState::from_wire(wire), s);
        }
    }

    /// The wire values are the backend's, not this module's invention. If
    /// Minos renames a state this fails here rather than as a 400 in the
    /// field, which is the entire reason the strings are spelled out.
    #[test]
    fn wire_values_are_the_contract() {
        assert_eq!(GameState::Planning.wire(), Some("planning"));
        assert_eq!(GameState::Preparation.wire(), Some("preparation"));
        assert_eq!(GameState::Execution.wire(), Some("execution"));
        assert_eq!(GameState::Closure.wire(), Some("closure"));
        assert_eq!(GameState::NoSession.wire(), None);
    }

    /// An unrecognised state must read as `NoSession`, never as a guess.
    /// A client that invented a fifth state would render a zone full of
    /// verbs the backend has not agreed to, and the operator would find out
    /// from a 409.
    #[test]
    fn an_unknown_state_is_no_session() {
        for junk in ["", "draft", "lobby", "Dynamic", "EXECUTION", "live"] {
            assert_eq!(
                GameState::from_wire(junk),
                GameState::NoSession,
                "{junk:?} was read as a real state"
            );
        }
    }

    /// Planning and Preparation are the only states that may edit the
    /// roster, mirroring Minos `GameRosterIsOpen`. Closure in particular
    /// must not: the backend refuses, and a picker that offered the verb
    /// would be offering a 409.
    #[test]
    fn only_the_two_open_states_may_edit_the_roster() {
        assert!(GameState::Planning.is_planning());
        assert!(GameState::Preparation.is_planning());
        assert!(!GameState::Execution.is_planning());
        assert!(!GameState::Closure.is_planning());
        assert!(!GameState::NoSession.is_planning());
    }

    /// Only Execution is running. The clock and the orders surface hang off
    /// this, so a state that answered true here would show an orders pane
    /// over a game nobody is playing.
    #[test]
    fn only_execution_is_running() {
        assert!(GameState::Execution.is_running());
        for s in [
            GameState::NoSession,
            GameState::Planning,
            GameState::Preparation,
            GameState::Closure,
        ] {
            assert!(!s.is_running(), "{s:?} claimed to be running");
        }
    }

    /// Every state carries a label, because the zone's trailing note must
    /// never be blank. A blank note reads as "no state", which is a state
    /// the console does not have. The labels are upper-case ASCII because
    /// the band renders them tracked and in short caps; a mixed-case label
    /// there would look like a sentence that lost its sentence.
    #[test]
    fn every_state_has_a_label() {
        for s in [
            GameState::NoSession,
            GameState::Planning,
            GameState::Preparation,
            GameState::Execution,
            GameState::Closure,
        ] {
            let label = s.label();
            assert!(!label.is_empty());
            assert!(
                label.chars().all(|c| c.is_ascii_uppercase() || c == ' '),
                "{label:?} is not short caps"
            );
        }
    }
}
