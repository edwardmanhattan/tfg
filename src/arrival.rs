//! ARRIVAL (F8) — what the press left behind, and the one gate (`§10`).
//!
//! THE PRESS ENDS SOMEWHERE, and where it ends is not one place: a game session
//! lands `PLANNING · NOT STARTED`, a Saved Scenario lands in the library with no
//! map at all, and Play Directly lands `PREPARATION · LIVE` with the gate that
//! the whole exercise waits behind. Three presses, three states, one gate.
//!
//! This module is the panel's MODEL and nothing else: what each arrival says,
//! and how the gate's strip is read. It is pure, so the sentences can be tested
//! against the sheet, and the shell draws it.
//!
//! TWO RULES THAT ARE EASY TO GET WRONG, and are the reason this is not just a
//! match in the shell:
//!
//! - **The verdict is the server's** (`can_execute`). The strip's ticks are the
//!   client's own reading, for legibility; when the two disagree the server
//!   wins and the panel says so (`§10`).
//! - **`READY n OF m` counts the EXERCISE SIDE** — seated participants who are
//!   not on the judge side (erratum 10). The sheet's frames count the referee
//!   and are left as drawn, so the difference stays visible.

use crate::backend::GameReadinessView;
use crate::draft::Verb;

/// Where the press landed, and what the panel says about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landing {
    /// Save as Game Session: a game in planning, nothing started.
    Planning,
    /// Save as Scenario: a Saved Scenario, and no map at all.
    Library,
    /// Play Directly: a game in preparation, live.
    Preparation,
}

impl Landing {
    pub fn of(verb: Verb) -> Landing {
        match verb {
            Verb::SaveAsGameSession => Landing::Planning,
            Verb::SaveAsScenario => Landing::Library,
            Verb::PlayDirectly => Landing::Preparation,
        }
    }

    /// `LANDS IN`, the first row of the table.
    pub fn lands(self) -> &'static str {
        match self {
            Landing::Planning => "PLANNING \u{b7} NOT STARTED",
            Landing::Library => "SAVED SCENARIOS \u{b7} 1 NEW",
            Landing::Preparation => "PREPARATION \u{b7} LIVE",
        }
    }

    /// Whether this arrival has a map on screen. A Saved Scenario is the only
    /// one that does not.
    pub fn has_map(self) -> bool {
        !matches!(self, Landing::Library)
    }
}

/// One row of the arrival table: what the press carried, and what comes next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: &'static str,
    pub value: String,
}

/// The arrival panel's whole content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arrival {
    pub landing: Landing,
    /// The game the press made, when it made one. A Saved Scenario has none.
    pub game_id: Option<i64>,
    pub rows: Vec<Row>,
}

impl Arrival {
    /// The sheet's table for this verb (`§10`), with the draft's own numbers
    /// where the sheet draws its example.
    pub fn of(
        verb: Verb,
        game_id: Option<i64>,
        placed: (usize, usize),
        units: usize,
        steps: usize,
        substeps: usize,
        seated: (usize, usize),
    ) -> Arrival {
        let landing = Landing::of(verb);
        let rows = match landing {
            Landing::Planning => vec![
                Row {
                    label: "FORCE",
                    value: format!("{} OF {} PLACED", placed.0, placed.0 + placed.1),
                },
                Row {
                    label: "PLAN",
                    value: format!(
                        "{units} UNITS \u{b7} {steps} STEPS \u{b7} {} SEATED",
                        seated.0
                    ),
                },
                Row {
                    label: "NEXT",
                    value: "OPENS AT 01 ESSENTIALS".to_string(),
                },
            ],
            Landing::Library => vec![
                Row {
                    label: "FORCE",
                    value: format!("{units} UNITS IN THE BASKET"),
                },
                Row {
                    label: "PLAN",
                    value: format!("{steps} STEPS \u{b7} {substeps} SUB-STEPS"),
                },
                Row {
                    label: "NEXT",
                    value: "OPEN IT TO START A NEW GAME".to_string(),
                },
            ],
            Landing::Preparation => vec![
                Row {
                    label: "FORCE",
                    value: format!(
                        "{units} UNITS \u{b7} {} OF {} PLACED",
                        placed.0,
                        placed.0 + placed.1
                    ),
                },
                Row {
                    label: "PLAN",
                    value: format!("{} SEATED \u{b7} 0 OF {} READY", seated.0, seated.0),
                },
                Row {
                    label: "NEXT",
                    value: "THE GAME MASTER PRESSES START".to_string(),
                },
            ],
        };
        Arrival {
            landing,
            game_id,
            rows,
        }
    }
}

/// THE ONE GATE: `preparation \u{2192} execution`, and its five conditions.
///
/// THE VERDICT IS THE SERVER'S. The ticks are the client's reading of what it
/// can see, and they exist so the operator can tell *which* ingredients are
/// missing at a glance — not to overrule `can_execute`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gate {
    /// `(condition, met)` in the sheet's order.
    pub conditions: Vec<(&'static str, bool)>,
    /// The server's own sentences, verbatim. Never reworded.
    pub blockers: Vec<String>,
    /// What the server said: the verdict, and not this struct's arithmetic.
    pub open: bool,
    /// `fast` waives readiness and NOTHING else.
    pub waived: bool,
    /// `(ready, exercise side)`: the denominator excludes the judge side.
    pub ready: (usize, usize),
}

impl Gate {
    /// Read the strip off what the console knows.
    ///
    /// `ready` is `(n, m)` counted over the EXERCISE SIDE only (erratum 10);
    /// `window_ok`, `units`, `placed` and `commanders` are the four the panel
    /// can see for itself. The verdict is still `view.can_execute`.
    pub fn of(
        view: &GameReadinessView,
        window_ok: bool,
        units: usize,
        placed: (usize, usize),
        commanders_missing: usize,
        ready: (usize, usize),
    ) -> Gate {
        let readiness_met = ready.0 >= ready.1 || view.fast;
        Gate {
            conditions: vec![
                ("WINDOW COMPLETE", window_ok),
                ("FORCE", units > 0),
                ("PLACED", placed.1 == 0),
                ("COMMANDER GUS-1", commanders_missing == 0),
                ("READY", readiness_met),
            ],
            blockers: view.blockers.clone(),
            open: view.can_execute,
            waived: view.fast,
            ready,
        }
    }

    /// Whether the client's own reading agrees with the server's verdict.
    ///
    /// When it does not, the panel says the server wins (`§10`) — which is the
    /// whole reason this is a method instead of a silent `if`.
    pub fn agrees(&self) -> bool {
        let all_met = self.conditions.iter().all(|(_, met)| *met);
        all_met == self.open
    }

    /// `READY 2 OF 4`.
    pub fn ready_label(&self) -> String {
        format!("READY {} OF {}", self.ready.0, self.ready.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(can_execute: bool, blockers: &[&str], fast: bool) -> GameReadinessView {
        GameReadinessView {
            can_execute,
            blockers: blockers.iter().map(|s| s.to_string()).collect(),
            fast,
        }
    }

    /// THREE PRESSES, THREE STATES — and only one of them has a map.
    #[test]
    fn each_verb_lands_somewhere_different() {
        assert_eq!(Landing::of(Verb::SaveAsGameSession), Landing::Planning);
        assert_eq!(Landing::of(Verb::SaveAsScenario), Landing::Library);
        assert_eq!(Landing::of(Verb::PlayDirectly), Landing::Preparation);
        assert!(!Landing::Library.has_map(), "a Saved Scenario has no map");
        assert!(Landing::Preparation.has_map());
    }

    /// The panel's rows are the sheet's own, with the draft's numbers.
    #[test]
    fn the_panel_says_what_the_press_left() {
        let arrival = Arrival::of(Verb::SaveAsGameSession, Some(42), (9, 0), 9, 5, 4, (4, 9));
        assert_eq!(arrival.landing, Landing::Planning);
        assert_eq!(arrival.game_id, Some(42));
        assert_eq!(arrival.rows[0].value, "9 OF 9 PLACED");
        assert!(arrival.rows[1].value.contains("5 STEPS"));
        assert_eq!(arrival.rows[2].value, "OPENS AT 01 ESSENTIALS");
        // And a Saved Scenario says where IT goes instead.
        let library = Arrival::of(Verb::SaveAsScenario, None, (0, 0), 9, 5, 4, (0, 0));
        assert_eq!(library.rows[2].value, "OPEN IT TO START A NEW GAME");
        assert_eq!(library.game_id, None);
    }

    /// THE VERDICT IS THE SERVER'S, and the strip is the client's reading.
    #[test]
    fn the_verdict_is_the_servers_and_the_strip_is_ours() {
        // The server says no, and names readiness.
        let refused = view(
            false,
            &["2 participants have not declared themselves ready"],
            false,
        );
        let gate = Gate::of(&refused, true, 9, (9, 0), 0, (2, 4));
        assert!(!gate.open, "the verdict is can_execute");
        assert!(!gate.conditions.iter().all(|(_, met)| *met));
        assert!(gate.agrees(), "the strip and the server agree");
        assert_eq!(gate.ready_label(), "READY 2 OF 4");
        assert_eq!(
            gate.blockers,
            vec!["2 participants have not declared themselves ready".to_string()],
            "the sentence is the server's, verbatim"
        );
    }

    /// `fast` WAIVES READINESS AND NOTHING ELSE.
    #[test]
    fn fast_waives_readiness_alone() {
        let fast = view(true, &[], true);
        let gate = Gate::of(&fast, true, 9, (9, 0), 0, (0, 4));
        assert!(gate.waived);
        assert!(
            gate.conditions.iter().all(|(_, met)| *met),
            "readiness is waived"
        );
        // A missing window is still missing.
        let gate = Gate::of(&fast, false, 9, (9, 0), 0, (0, 4));
        assert!(!gate.conditions.iter().all(|(_, met)| *met));
        assert!(!gate.agrees(), "the strip disagrees with the verdict");
    }

    /// THE STRIP DISAGREEING WITH THE SERVER IS SHOWN, not resolved: the panel
    /// must say the server wins.
    #[test]
    fn a_disagreement_is_visible_rather_than_settled() {
        let server_no = view(false, &["something the client cannot see"], false);
        let gate = Gate::of(&server_no, true, 9, (9, 0), 0, (4, 4));
        assert!(
            gate.conditions.iter().all(|(_, met)| *met),
            "the client sees no gap"
        );
        assert!(!gate.open);
        assert!(!gate.agrees(), "so the panel says the server wins");
    }

    /// THE DENOMINATOR EXCLUDES THE JUDGE SIDE (erratum 10), so the panel's
    /// count is the exercise's, not the sheet's frame count.
    #[test]
    fn the_ready_count_is_the_exercise_side() {
        let ready = view(
            false,
            &["1 participant has not declared themselves ready"],
            false,
        );
        // Four seated, one of them the referee: three on the exercise side.
        let gate = Gate::of(&ready, true, 9, (9, 0), 0, (2, 3));
        assert_eq!(gate.ready_label(), "READY 2 OF 3");
    }
}
