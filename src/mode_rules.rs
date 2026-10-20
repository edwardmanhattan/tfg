//! The console's own rules about a mode: which verbs each mode earns.
//!
//! THESE ARE NOT THE SERVER'S RULES, and the difference is the whole
//! reason this module exists. Minos answers "may an ORDER move a hull" with
//! `GameMode.AllowsSteering`, and it answers YES for a `scenario` exercise —
//! because the server will hold the order and release it when the step
//! says so. The CONSOLE does not offer a helm there at all: a scenario
//! exercise is played through its steps, and the commander's verb on the
//! current step is to declare readiness for it. The server's rule is about
//! what the wire will accept; this one is about what the console offers.
//! A console that offered a free-standing helm in a scenario exercise would
//! be offering a verb whose answer is always "held", and the operator would
//! learn what that means from the server rather than from the screen.
//!
//! The mode is READ ONCE, where the session is read, and every rule below
//! is a function of that one value. A rule that compared the mode's SPELLING
//! at each site would be a rule that could be spelled two ways, and the
//! second spelling would be the one nobody tested.
//!
//! `Unknown` is a mode, and it FAILS CLOSED: a mode this build has not
//! heard of offers no helm, no scenario surfaces and no clock verb, because
//! the safe direction for an exercise the console cannot describe is the
//! one where it offers nothing.

use crate::gamestate::GameMode;

/// Whether the helm is offered in this mode.
///
/// ADR-0021: the helm belongs to a `maneuver` exercise and to nothing else.
/// A `static` exercise has no order at all (the server refuses every one
/// with a 409), and a `scenario` exercise is played through its steps, so
/// the helm's instruments have nothing to steer in either.
///
/// NOT `GameMode::allows_steering`, which is the server's rule and says
/// yes for `scenario`: the server will accept and HOLD an order there, and
/// the console's answer to "is this a free-standing helm?" is no.
pub fn helm_offered(mode: GameMode) -> bool {
    matches!(mode, GameMode::Maneuver)
}

/// Whether this is a scenario exercise: one played from a Game Master's
/// book, where a step is what the exercise runs on.
///
/// The scenario surfaces — the board, the step, the step's verbs — belong
/// to this mode alone. A `maneuver` exercise has no book to play, and a
/// `static` one has no step to declare readiness for.
pub fn scenario_exercise(mode: GameMode) -> bool {
    mode.is_scenario()
}

/// Whether the commander's step verb is offered: the declaration that
/// says this station is ready for the step the exercise asks.
///
/// ADR-0021: in a scenario exercise the step IS the commander's verb, and
/// the declaration is the only way to answer it. Outside one there is no
/// step to declare for, so the verb is not drawn at all.
pub fn step_declare_offered(mode: GameMode) -> bool {
    mode.is_scenario()
}

/// Whether the Game Master's step verbs are offered: play, skip, end,
/// replay.
///
/// ADR-0021: these drive a step, and only a scenario exercise has steps.
pub fn step_verbs_offered(mode: GameMode) -> bool {
    mode.is_scenario()
}

/// Whether the clock may be resumed from this console.
///
/// ADR-0021: in a scenario exercise the clock runs INSIDE a step — the
/// server refuses a resume between beats with a 409, and the exercise
/// opens stopped. So the console offers PAUSE and never a resume there. In
/// the other two modes the pair stays as it is, and the verb follows the
/// last clock answer as it always did.
///
/// `Unknown` says no: a mode the console cannot name is not a mode it may
/// move the clock of.
pub fn clock_may_resume(mode: GameMode) -> bool {
    matches!(mode, GameMode::Maneuver | GameMode::Static)
}

/// The sentence the helm's place carries when there is no helm.
///
/// ADR-0021: the instruments are not drawn greyed out in a mode that has
/// no helm — they are not drawn at all, and the MODE'S SENTENCE stands
/// where they were, because an empty channel reads as a control that
/// failed to draw. Each sentence names the remedy the mode actually has:
/// the step for a scenario exercise, the hand for a static one.
pub fn helm_sentence(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Maneuver => {
            "An order posts at once; the server clamps and answers."
        }
        GameMode::Scenario => {
            "No helm in a scenario exercise \u{2014} the step is the commander's verb: \
             declare yourself ready for the step the exercise asks."
        }
        GameMode::Static => {
            "No helm in a static exercise \u{2014} the positions are set by hand: \
             drag the hull onto the map and it jumps there."
        }
        GameMode::Unknown => {
            "This build does not know the exercise's mode, so it offers no helm."
        }
    }
}

/// The sentence the scenario surfaces carry when there are none.
///
/// The scenario widget is not drawn outside a scenario exercise, and this
/// is what the board says for itself when it is opened in one that has no
/// script — a board with no steps is a book the Game Master has not
/// written, not a broken surface.
pub fn no_script_sentence() -> &'static str {
    "This exercise has no script: there are no steps to play."
}

/// The line the planning column carries in place of the scenario book's
/// door, when the exercise is not a scenario one.
///
/// A SCRIPT IS A SCENARIO EXERCISE'S MECHANISM, and it is the server's:
/// the step's gate, the step's clock and the step ceiling are scenario-only
/// (`minos/internal/services/game_step_ceiling.go`), so a book written in a
/// maneuver exercise is a set of pages nothing will ever play. The server
/// tolerates the write; the console does not offer the door, because a door
/// that opens onto pages that do nothing is a lie about what the exercise
/// is.
pub fn book_sentence(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Maneuver => {
            "A script belongs to a scenario exercise \u{2014} this one is played by orders."
        }
        GameMode::Static => {
            "A script belongs to a scenario exercise \u{2014} this one's positions \
             are set by hand."
        }
        GameMode::Scenario => "",
        GameMode::Unknown => {
            "This build does not know the exercise's mode, so it cannot tell a \
             scenario exercise from another \u{2014} and it offers no script."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The helm belongs to a maneuver exercise and to nothing else. A
    /// scenario exercise's order would be HELD by the server, and a static
    /// exercise's is refused outright, so a helm in either would be a
    /// control whose answer is never "applied".
    #[test]
    fn the_helm_is_offered_in_a_maneuver_exercise_alone() {
        assert!(helm_offered(GameMode::Maneuver));
        assert!(!helm_offered(GameMode::Scenario));
        assert!(!helm_offered(GameMode::Static));
        assert!(
            !helm_offered(GameMode::Unknown),
            "a mode this build does not know must not gain a helm"
        );
    }

    /// The console's helm rule is NOT the server's steering rule, and the
    /// difference is the point: the server allows an order in a scenario
    /// exercise because it HOLDS it, and the console offers no helm there
    /// at all. A console that copied the server's rule would offer an
    /// instrument whose every answer is "held".
    #[test]
    fn the_console_s_helm_is_not_the_servers_steering_rule() {
        assert!(GameMode::Scenario.allows_steering());
        assert!(
            !helm_offered(GameMode::Scenario),
            "the server holds the order; the console does not offer the helm"
        );
    }

    /// The scenario surfaces belong to a scenario exercise alone. A
    /// maneuver exercise has no book and a static one has no step, so a
    /// board drawn in either is a surface with nothing in it.
    #[test]
    fn the_scenario_surfaces_belong_to_a_scenario_exercise_alone() {
        for mode in [GameMode::Maneuver, GameMode::Static, GameMode::Unknown] {
            assert!(!scenario_exercise(mode), "{mode:?} is not a scenario");
            assert!(!step_declare_offered(mode), "{mode:?} has no step to declare for");
            assert!(!step_verbs_offered(mode), "{mode:?} has no step to play");
        }
        assert!(scenario_exercise(GameMode::Scenario));
        assert!(step_declare_offered(GameMode::Scenario));
        assert!(step_verbs_offered(GameMode::Scenario));
    }

    /// A scenario exercise's clock runs inside a step, and the server
    /// refuses a resume between the beats with a 409 — so the console
    /// offers a pause and never a resume. The other two modes keep the
    /// pair they always had.
    #[test]
    fn a_scenario_exercise_offers_no_resume_between_beats() {
        assert!(!clock_may_resume(GameMode::Scenario));
        assert!(clock_may_resume(GameMode::Maneuver));
        assert!(clock_may_resume(GameMode::Static));
        assert!(
            !clock_may_resume(GameMode::Unknown),
            "an unknown mode may not move the clock"
        );
    }

    /// Every mode has a sentence to stand where the helm was, and the two
    /// that have a remedy name it: the step for a scenario exercise, the
    /// hand for a static one. A sentence that named neither would leave the
    /// operator with the fact and no way forward.
    #[test]
    fn every_mode_says_what_it_has_instead_of_a_helm() {
        for mode in [
            GameMode::Maneuver,
            GameMode::Scenario,
            GameMode::Static,
            GameMode::Unknown,
        ] {
            let sentence = helm_sentence(mode);
            assert!(!sentence.is_empty(), "{mode:?} has no sentence");
            assert!(sentence.ends_with('.'), "{sentence:?} is not a sentence");
        }
        assert!(helm_sentence(GameMode::Scenario).contains("step"));
        assert!(helm_sentence(GameMode::Static).contains("by hand"));
    }

    /// The book's door belongs to a scenario exercise, and the line that
    /// stands in its place names why. A door that opened onto pages nothing
    /// would ever play would be a lie about the exercise.
    #[test]
    fn the_scripts_door_belongs_to_a_scenario_exercise_alone() {
        assert_eq!(book_sentence(GameMode::Scenario), "", "a scenario exercise has the door");
        for mode in [GameMode::Maneuver, GameMode::Static, GameMode::Unknown] {
            let sentence = book_sentence(mode);
            assert!(!sentence.is_empty(), "{mode:?} needs a reason");
            assert!(sentence.ends_with('.'), "{sentence:?} is not a sentence");
            assert!(sentence.contains("scenario"), "{sentence:?} does not say which");
        }
    }
}
