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
//!
//! The second axis lives here too: the MODE, which is how an exercise
//! produces its positions at all. It is a type for the same reason the
//! state is — a mode CHANGES BEHAVIOUR (a `maneuver` exercise integrates
//! orders over assumed time, a `static` one has its positions put by
//! hand, a `scenario` one holds an order until a step releases it), and a
//! client that carried the mode as a string would have to remember what
//! each spelling means at every place it compared one.

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

/// How an exercise produces its positions, as ONE axis.
///
/// A MODE IS A TYPE AND NOT A STRING BECAUSE IT CHANGES BEHAVIOUR. A
/// `maneuver` exercise integrates a Commando's orders over assumed
/// time; a `static` one has its positions put by hand and refuses
/// orders outright; a `scenario` one is maneuver with a Game Master's
/// plot, and it HOLDS an order until a step releases it. A client that
/// carried the mode as a string would compare it against a spelling in
/// a dozen places, and each of those places would have to remember what
/// the spelling means.
///
/// The three connected variants are the Minos `GameMode` wire values
/// verbatim, for the reason the states are: the backend is
/// authoritative, and a value the client invented would be a mode
/// nothing implements.
///
/// `Unknown` is the one that did not exist as a value. It is the
/// absence of a mode the client could name — a word the server sent
/// that this build has not heard of, or a game read that failed
/// altogether — and an `Option` cannot be matched on alongside the
/// modes it excludes. It FAILS CLOSED, which is the whole point: a
/// mode the client does not know must not silently gain the
/// permission to steer, because the server's own rule names the modes
/// that allow it rather than the ones that do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameMode {
    Maneuver,
    Static,
    Scenario,
    Unknown,
}

impl GameMode {
    /// The Minos wire value, or `None` for the client-only mode.
    ///
    /// `None` is honest rather than a sentinel string: `Unknown` has no
    /// server counterpart, and sending one would be a request for a
    /// mode the server has not agreed to.
    pub fn wire(self) -> Option<&'static str> {
        match self {
            GameMode::Maneuver => Some("maneuver"),
            GameMode::Static => Some("static"),
            GameMode::Scenario => Some("scenario"),
            GameMode::Unknown => None,
        }
    }

    /// Parse a Minos `GameMode`. An unrecognised value reads as
    /// `Unknown`, never as a guess: a mode the client invents would
    /// offer an operator the verbs of an exercise the server runs
    /// differently, and the refusal would arrive as a 409.
    pub fn from_wire(s: &str) -> Self {
        match s {
            "maneuver" => GameMode::Maneuver,
            "static" => GameMode::Static,
            "scenario" => GameMode::Scenario,
            _ => GameMode::Unknown,
        }
    }

    /// Short caps, for an island's trailing note.
    pub fn label(self) -> &'static str {
        match self {
            GameMode::Maneuver => "MANEUVER",
            GameMode::Static => "STATIC",
            GameMode::Scenario => "SCENARIO",
            GameMode::Unknown => "NO MODE",
        }
    }

    /// Whether a Commando may move a hull by ORDER in this mode.
    ///
    /// Mirrors Minos `GameMode.AllowsSteering`, which is an explicit
    /// list rather than the exclusion of the one mode that refuses:
    /// `maneuver` and `scenario` steer, `static` does not, and a mode
    /// this client does not know steers not — a list of what is
    /// allowed fails closed on everything it does not name.
    pub fn allows_steering(self) -> bool {
        matches!(self, GameMode::Maneuver | GameMode::Scenario)
    }

    /// Whether the exercise's positions are PUT THERE BY HAND rather
    /// than simulated by orders. Mirrors Minos
    /// `GameMode.PositionsAreSet`, which is true for `static` alone.
    pub fn positions_are_set(self) -> bool {
        matches!(self, GameMode::Static)
    }

    /// Whether this is a scenario exercise: one played from a Game
    /// Master's book, where an order is held until a step releases it.
    pub fn is_scenario(self) -> bool {
        matches!(self, GameMode::Scenario)
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

    /// The three connected modes must round-trip through the wire
    /// values. This is the test that would have caught a client echoing
    /// the concept document's prose back at the server: `dynamic` is
    /// not a mode Minos accepts, and its own test lists it as invalid.
    #[test]
    fn every_connected_mode_round_trips() {
        for m in [
            GameMode::Maneuver,
            GameMode::Static,
            GameMode::Scenario,
        ] {
            let wire = m.wire().expect("a connected mode has a wire value");
            assert_eq!(GameMode::from_wire(wire), m);
        }
    }

    /// The wire values are the backend's, not this module's invention.
    /// If Minos renames a mode this fails here rather than as a 400 in
    /// the field, which is the entire reason the strings are spelled
    /// out.
    #[test]
    fn mode_wire_values_are_the_contract() {
        assert_eq!(GameMode::Maneuver.wire(), Some("maneuver"));
        assert_eq!(GameMode::Static.wire(), Some("static"));
        assert_eq!(GameMode::Scenario.wire(), Some("scenario"));
        assert_eq!(GameMode::Unknown.wire(), None);
    }

    /// An unrecognised mode must read as `Unknown`, never as a guess.
    /// A failed read of a game leaves the mode empty, so the empty
    /// string is a real answer and not an oversight: it must not read
    /// as a mode that permits anything.
    #[test]
    fn an_unknown_mode_is_not_a_mode() {
        for junk in ["", "dynamic", "live", "Maneuver", "operational", "static "] {
            assert_eq!(
                GameMode::from_wire(junk),
                GameMode::Unknown,
                "{junk:?} was read as a real mode"
            );
        }
    }

    /// Steering is allowed in `maneuver` and `scenario` and nowhere
    /// else. Mirrors Minos `GameMode.AllowsSteering`, whose list is
    /// explicit so that a mode the server has yet to publish fails
    /// closed rather than gaining the permission by not being named.
    #[test]
    fn only_maneuver_and_scenario_allow_steering() {
        assert!(GameMode::Maneuver.allows_steering());
        assert!(GameMode::Scenario.allows_steering());
        assert!(!GameMode::Static.allows_steering());
        assert!(
            !GameMode::Unknown.allows_steering(),
            "a mode this client does not know must not be trusted to steer"
        );
    }

    /// Positions are set by hand in a `static` exercise alone. Mirrors
    /// Minos `GameMode.PositionsAreSet`, and the order matters: the
    /// drag route asks the opposite question, and a mode that answered
    /// both ways would let a drag be committed where the positions are
    /// simulated.
    #[test]
    fn only_static_sets_its_positions_by_hand() {
        assert!(GameMode::Static.positions_are_set());
        assert!(!GameMode::Maneuver.positions_are_set());
        assert!(!GameMode::Scenario.positions_are_set());
        assert!(!GameMode::Unknown.positions_are_set());
    }

    /// Only a scenario exercise is played from a book. The rule is
    /// narrow because it is the one that changes what an order MEANS:
    /// a scenario order is held rather than applied.
    #[test]
    fn only_scenario_is_a_scenario() {
        assert!(GameMode::Scenario.is_scenario());
        assert!(!GameMode::Maneuver.is_scenario());
        assert!(!GameMode::Static.is_scenario());
        assert!(!GameMode::Unknown.is_scenario());
    }

    /// Every mode carries a label, because the zone's trailing note
    /// must never be blank. A blank note reads as "no mode", which is
    /// what the unknown mode is — and saying so by saying nothing would
    /// be indistinguishable from a mode the client forgot to print.
    /// The labels are upper-case ASCII because the band renders them
    /// tracked and in short caps.
    #[test]
    fn every_mode_has_a_label() {
        for m in [
            GameMode::Maneuver,
            GameMode::Static,
            GameMode::Scenario,
            GameMode::Unknown,
        ] {
            let label = m.label();
            assert!(!label.is_empty());
            assert!(
                label.chars().all(|c| c.is_ascii_uppercase() || c == ' '),
                "{label:?} is not short caps"
            );
        }
    }
}
