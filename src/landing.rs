//! The landing: one history and one question (F1a, F1b).
//!
//! After sign-in the creation flow asks exactly one thing — the mode — beside a
//! read-only history of the games this account can see. Answering it opens a
//! local draft (`crate::draft`); `PROCEED` is the only button, and what it does
//! depends on what is chosen (`docs/creation-flow-handoff.md` §3.1).
//!
//! THE HISTORY IS READ, NOT FILTERED BY MODE (§3.1). Picking a mode changes
//! nothing about the list: the tabs are GAME STATES, and the counts come from
//! the UNFILTERED read, because a filtered page cannot count the tabs it
//! filtered out. Five tabs, because there are four states (erratum 14) and
//! `preparation` is where a Play Directly press lands — the tab an operator will
//! look for first.
//!
//! FOUR COLUMNS, NOT FIVE. The sheet's `ORIGIN` column has no API backing
//! (erratum 11), so `WHEN` is `created_at` and provenance is a backend field
//! request rather than a client guess from names.
//!
//! WHAT PROCEED DOES, and every clause is a rule:
//!
//! | Selection | Proceed does |
//! | --- | --- |
//! | a mode, no game row | opens a `Blank` draft at stage 01 |
//! | a `planning` game row | opens a `Reopened` draft seeded from it |
//! | any other game row | opens the console, as a double-click would |
//!
//! A row in `preparation`, `execution` or `closure` is not editable — the terms,
//! the force and the tree are fixed once planning ends, and the server refuses
//! the writes with a 409 — so the row carries a hint saying so rather than
//! opening a draft that cannot commit.
//!
//! THE MODE IS CREATE-ONLY. A reopened draft's mode is the game's, so a chosen
//! mode is IGNORED when a row is selected; the row answers the question.
//!
//! Toolkit-free: no GPUI, no network. The shell draws what this decides.

use crate::backend::{GameRow, ScenarioMission};
use crate::gamestate::GameMode;

/// The sentence the button carries when nothing is chosen.
pub const PROCEED_DISABLED: &str = "NOTHING SELECTED \u{b7} PROCEED IS DISABLED";

/// The sentence a scenario draft adds until a START FROM row is chosen.
///
/// A Saved Scenario is a scenario and nothing else, so `BLANK SCENARIO` is a
/// row rather than an absence — but it is still a row, and the operator picks
/// it (`§3.1`).
pub const START_FROM_UNCHOSEN: &str = "NO START FROM ROW CHOSEN \u{b7} PROCEED IS DISABLED";

/// One tab of the history.
///
/// `All` is not a state — it is every state — so its `state()` is `None` and
/// nothing sends it to the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    All,
    Planning,
    Preparation,
    Execution,
    Closure,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::All,
        Tab::Planning,
        Tab::Preparation,
        Tab::Execution,
        Tab::Closure,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::All => "ALL",
            Tab::Planning => "PLANNING",
            Tab::Preparation => "PREPARATION",
            Tab::Execution => "EXECUTION",
            Tab::Closure => "CLOSURE",
        }
    }

    /// The state this tab filters on, or `None` for the all tab.
    ///
    /// This is the value `GET /games?state=` takes, and `None` is the whole
    /// list — never the empty string, which is not a state.
    pub fn state(self) -> Option<&'static str> {
        match self {
            Tab::All => None,
            Tab::Planning => Some("planning"),
            Tab::Preparation => Some("preparation"),
            Tab::Execution => Some("execution"),
            Tab::Closure => Some("closure"),
        }
    }

    /// Whether a game in `state` belongs on this tab.
    pub fn matches(self, state: &str) -> bool {
        match self.state() {
            None => true,
            Some(s) => s == state,
        }
    }
}

impl Default for Tab {
    fn default() -> Self {
        Tab::All
    }
}

/// One of the three cards the question offers, with the sheet's own line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModeCard {
    pub number: u8,
    pub mode: GameMode,
    pub line: &'static str,
    /// What the mode earns, right-aligned on the card: the sheet's
    /// `NO STAGE 04 · NO SCENARIO` / `STAGES 01–06 · SCENARIOS`. It is a
    /// machine readout, so it is set in the machine face.
    pub readout: &'static str,
}

pub const MODE_CARDS: [ModeCard; 3] = [
    ModeCard {
        number: 1,
        mode: GameMode::Static,
        line: "Positions set by hand; orders are refused.",
        readout: "NO STAGE 04 \u{b7} NO SCENARIO",
    },
    ModeCard {
        number: 2,
        mode: GameMode::Maneuver,
        line: "The exercise integrates helm orders.",
        readout: "NO STAGE 04 \u{b7} NO SCENARIO",
    },
    ModeCard {
        number: 3,
        mode: GameMode::Scenario,
        line: "Authored steps, sub-steps, audiences.",
        readout: "STAGES 01\u{2013}06 \u{b7} SCENARIOS",
    },
];

/// The line under the cards, which is the whole shape of the flow.
pub const EVERYTHING_ELSE_LATER: &str = "Everything else is asked inside the draft.";

/// What a scenario draft may start from: nothing chosen yet, an empty book, or
/// a Saved Scenario.
///
/// `Unchosen` exists because `BLANK SCENARIO` is a ROW: a scenario draft whose
/// START FROM list has been drawn but not touched is not a draft starting from
/// blank, it is a question nobody has answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartFrom {
    Unchosen,
    Blank,
    Mission(i64),
}

impl Default for StartFrom {
    fn default() -> Self {
        StartFrom::Unchosen
    }
}

/// How the START FROM read is going.
///
/// A FAILED READ IS A STATE, not an empty list: the list would read as "this
/// account has no Saved Scenarios", which is a different and possibly false
/// statement. The builder of the landing sets this, because only it knows
/// whether the read was attempted.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Load {
    #[default]
    NotAsked,
    Reading,
    Ready,
    /// The server refused or did not answer, in its own words.
    Failed(String),
}

/// One row of the START FROM list, ready to draw.
///
/// The blank row and a mission row are the same shape on purpose: they are the
/// same list, and a blank one that drew differently would be a different kind
/// of thing.
#[derive(Debug, Clone, PartialEq)]
pub struct StartRow {
    pub choice: StartFrom,
    pub title: String,
    /// The mission's code (`Surface`, `Escort`, …), empty for the blank row.
    pub code: String,
    /// `9 STEPS`, or `0 STEPS` for the blank row.
    pub steps: String,
    /// The mission's date, as the history's `WHEN` renders it.
    pub when: String,
    /// The blank row's sentence; empty for a mission.
    pub note: String,
}

/// What pressing `PROCEED` does.
#[derive(Debug, Clone, PartialEq)]
pub enum Proceed {
    /// Nothing chosen: the button is disabled and says why.
    Disabled(&'static str),
    /// A mode and nothing else: a `Blank` draft at stage 01.
    Blank(GameMode),
    /// A START FROM row: a draft seeded from that Saved Scenario.
    SavedScenario(i64),
    /// A `planning` row: a draft seeded from the game, whose press is a diff.
    Reopen(i64),
    /// A row past planning: open the console. There is nothing left to draft,
    /// and `why` is what the row's hint says.
    Open { game_id: i64, why: &'static str },
}

impl Proceed {
    /// Whether the button is live.
    pub fn enabled(&self) -> bool {
        !matches!(self, Proceed::Disabled(_))
    }

    /// The sentence a disabled button carries.
    pub fn why(&self) -> Option<&'static str> {
        match self {
            Proceed::Disabled(why) => Some(why),
            Proceed::Open { why, .. } => Some(why),
            _ => None,
        }
    }
}

/// The landing's own state: the read-only history, the one question, and the
/// selection on each.
///
/// THE GAMES ARE THE UNFILTERED READ and stay that way: the tabs filter what is
/// DRAWN, and every count is counted over this list. A caller that handed this
/// a filtered page would get counts that silently disagree with the tabs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Landing {
    games: Vec<GameRow>,
    missions: Vec<ScenarioMission>,
    pub missions_load: Load,
    pub tab: Tab,
    mode: Option<GameMode>,
    start_from: StartFrom,
    selected: Option<i64>,
}

impl Landing {
    pub fn new() -> Self {
        Self::default()
    }

    // -- the reads -------------------------------------------------------

    /// The whole history, as the unfiltered read gives it.
    ///
    /// A selection about a game that is no longer listed is dropped: the row it
    /// was about has gone, and a Proceed that reopened a game the list no longer
    /// shows would be a button acting on something the operator cannot see.
    pub fn set_games(&mut self, games: Vec<GameRow>) {
        self.games = games;
        if let Some(selected) = self.selected {
            if !self.games.iter().any(|g| g.id == selected) {
                self.selected = None;
            }
        }
    }

    pub fn games(&self) -> &[GameRow] {
        &self.games
    }

    /// The Saved Scenarios, newest first as the server lists them.
    pub fn set_missions(&mut self, missions: Vec<ScenarioMission>) {
        self.missions = missions;
        self.missions_load = Load::Ready;
        // A START FROM row about a mission that is no longer listed is the same
        // case as a vanished game: the answer goes with the row.
        if let StartFrom::Mission(id) = self.start_from {
            if !self.missions.iter().any(|m| m.id == id) {
                self.start_from = StartFrom::Unchosen;
            }
        }
    }

    pub fn missions_reading(&mut self) {
        self.missions_load = Load::Reading;
    }

    pub fn missions_failed(&mut self, why: &str) {
        self.missions = Vec::new();
        self.missions_load = Load::Failed(why.to_string());
    }

    pub fn missions(&self) -> &[ScenarioMission] {
        &self.missions
    }

    // -- the history -----------------------------------------------------

    /// Every tab with its count, counted over the unfiltered read.
    pub fn counts(&self) -> [(Tab, usize); 5] {
        Tab::ALL.map(|tab| {
            let count = self.games.iter().filter(|g| tab.matches(&g.state)).count();
            (tab, count)
        })
    }

    /// The rows this tab draws, in the server's order.
    pub fn visible(&self) -> Vec<&GameRow> {
        self.games
            .iter()
            .filter(|g| self.tab.matches(&g.state))
            .collect()
    }

    /// The row a single click selected.
    pub fn selected(&self) -> Option<i64> {
        self.selected
    }

    /// Single click selects (`§3.1`); a double click opens, and that is the
    /// shell's gesture, not this model's.
    ///
    /// THE LAST ANSWER WINS, in both directions: picking a row withdraws the
    /// wizard's answer, because the one verb below the wizard now belongs to
    /// this row (reopen or open) and a mode left chosen beside it would be an
    /// answer the press does not use.
    pub fn select(&mut self, game_id: i64) {
        self.selected = Some(game_id);
        self.mode = None;
        self.start_from = StartFrom::Unchosen;
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    pub fn selected_row(&self) -> Option<&GameRow> {
        let id = self.selected?;
        self.games.iter().find(|g| g.id == id)
    }

    // -- the question ----------------------------------------------------

    pub fn mode(&self) -> Option<GameMode> {
        self.mode
    }

    /// Choose a mode. The mode is create-only and the server never changes it,
    /// so this is the one place it is set — and a mode this build cannot name
    /// is not a choice.
    ///
    /// Leaving `SCENARIO` withdraws a START FROM answer: the list belongs to
    /// that mode, and keeping it would leave a draft seeded from a Saved
    /// Scenario the operator can no longer see.
    ///
    /// IT ALSO WITHDRAWS A SELECTED ROW, and that is the other half of "the
    /// last answer wins". A row clicked earlier stays selected and would
    /// otherwise retarget the one verb: the operator answers the wizard, presses
    /// `PROCEED`, and lands in that game's console instead of the draft they
    /// just described.
    pub fn choose_mode(&mut self, mode: GameMode) {
        if mode == GameMode::Unknown {
            return;
        }
        self.mode = Some(mode);
        self.selected = None;
        if mode != GameMode::Scenario {
            self.start_from = StartFrom::Unchosen;
        }
    }

    pub fn start_from(&self) -> StartFrom {
        self.start_from
    }

    /// Answer the START FROM question. Only a scenario draft asks it, so a
    /// choice made in another mode is refused rather than stored.
    pub fn choose_start_from(&mut self, choice: StartFrom) {
        if self.mode != Some(GameMode::Scenario) {
            return;
        }
        self.start_from = choice;
    }

    /// The START FROM rows, or none when the mode does not ask.
    ///
    /// `STATIC · MANEUVER HAVE NO SUCH LIST — ALWAYS BLANK`: a mission is a
    /// scenario and neither of those modes can hold one, so the list is not
    /// drawn at all rather than drawn empty.
    pub fn start_rows(&self) -> Vec<StartRow> {
        if self.mode != Some(GameMode::Scenario) {
            return Vec::new();
        }
        let mut rows = vec![StartRow {
            choice: StartFrom::Blank,
            title: "BLANK SCENARIO".to_string(),
            code: String::new(),
            steps: "0 STEPS".to_string(),
            when: String::new(),
            note: "No steps. The composer starts empty.".to_string(),
        }];
        for mission in &self.missions {
            rows.push(StartRow {
                choice: StartFrom::Mission(mission.id),
                title: mission.title.clone(),
                code: mission.code.clone(),
                steps: step_label(mission.step_count()),
                when: mission.date_stamp().map(when_label).unwrap_or_default(),
                note: String::new(),
            });
        }
        rows
    }

    /// What the one button does.
    ///
    /// A ROW AND A MODE ARE NEVER BOTH ANSWERED: `select` and `choose_mode` each
    /// withdraw the other, so this verb is always about the last thing the
    /// operator picked (`§3.1`). A row is therefore checked before the mode only
    /// as an order, not as a precedence.
    pub fn proceed(&self) -> Proceed {
        if let Some(row) = self.selected_row() {
            return if row.state == "planning" {
                Proceed::Reopen(row.id)
            } else {
                Proceed::Open {
                    game_id: row.id,
                    why: row_hint(&row.state),
                }
            };
        }
        let Some(mode) = self.mode else {
            return Proceed::Disabled(PROCEED_DISABLED);
        };
        if mode == GameMode::Scenario {
            return match self.start_from {
                StartFrom::Unchosen => Proceed::Disabled(START_FROM_UNCHOSEN),
                StartFrom::Blank => Proceed::Blank(mode),
                StartFrom::Mission(id) => Proceed::SavedScenario(id),
            };
        }
        Proceed::Blank(mode)
    }
}

/// What a row's hint says: whether PROCEED may draft it, and why not.
///
/// A row past planning is not editable — the terms, the force and the tree are
/// fixed once planning ends and the server refuses the writes with a 409 — so
/// the hint says so rather than letting the operator open a draft that cannot
/// commit (`§3.1`).
pub fn row_hint(state: &str) -> &'static str {
    match state {
        "planning" => "PLANNING \u{b7} PROCEED REOPENS IT AS A DRAFT",
        "preparation" => "PREPARATION \u{b7} NOT EDITABLE \u{b7} PROCEED OPENS THE CONSOLE",
        "execution" => "EXECUTION \u{b7} NOT EDITABLE \u{b7} PROCEED OPENS THE CONSOLE",
        "closure" => "CLOSURE \u{b7} NOT EDITABLE \u{b7} PROCEED OPENS THE CONSOLE",
        _ => "A STATE THIS BUILD DOES NOT KNOW \u{b7} PROCEED OPENS THE CONSOLE",
    }
}

/// `9 STEPS`, or `1 STEP`.
pub fn step_label(steps: usize) -> String {
    if steps == 1 {
        "1 STEP".to_string()
    } else {
        format!("{steps} STEPS")
    }
}

/// A stamp as the history draws it: `23 OCT`.
///
/// A stamp this build cannot read renders as nothing rather than as a guess:
/// the row still identifies itself by its name and its state, and an invented
/// date is the one field an operator would trust and be wrong about.
pub fn when_label(at: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(at) {
        Ok(at) => at.format("%d %b").to_string().to_uppercase(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::TimeWindow;

    fn game(id: i64, name: &str, state: &str) -> GameRow {
        GameRow {
            id,
            name: name.into(),
            state: state.into(),
            mode: "maneuver".into(),
            created_at: "2026-10-23T09:41:00+07:00".into(),
            updated_at: String::new(),
        }
    }

    fn mission(id: i64, title: &str, code: &str, steps: usize, at: &str) -> ScenarioMission {
        let mut m = ScenarioMission {
            id,
            code: code.into(),
            title: title.into(),
            description: String::new(),
            play_count: 0,
            updated_at: Some(at.into()),
            steps: Vec::new(),
        };
        for at in 0..steps {
            m.steps.push(crate::backend::MissionStep {
                id: at as i64 + 1,
                position: at as i64,
                kind: "orders".into(),
                content: format!("step {at}"),
                start_hour: None,
                end_hour: None,
                updated_at: None,
                substeps: Vec::new(),
            });
        }
        m
    }

    /// FIVE TABS, BECAUSE THERE ARE FOUR STATES. The sheet draws four and its
    /// own example contradicts itself; `preparation` is where Play Directly
    /// lands, so it is the tab an operator looks for first (erratum 14).
    #[test]
    fn the_tabs_are_five_and_the_counts_sum_to_the_history() {
        let mut landing = Landing::new();
        landing.set_games(vec![
            game(1, "Baltic Shield 26", "planning"),
            game(2, "North Sea Patrol", "preparation"),
            game(3, "Harbour Defence", "execution"),
            game(4, "Harbour Sweep", "closure"),
            game(5, "Second Wave", "planning"),
            game(6, "Third Wave", "preparation"),
            game(7, "Old One", "closure"),
            game(8, "Older One", "execution"),
        ]);
        let counts = landing.counts();
        assert_eq!(counts.len(), 5);
        assert_eq!(
            counts.map(|(tab, _)| tab.label()),
            ["ALL", "PLANNING", "PREPARATION", "EXECUTION", "CLOSURE"]
        );
        assert_eq!(
            counts.map(|(_, n)| n),
            [8, 2, 2, 2, 2],
            "every count is counted over the whole list"
        );
        // ALL is every game; the four STATE tabs partition the history and sum
        // to the same number. (F1a's own example does not — a row reads
        // PREPARATION while the four counts sum to the total without one,
        // which is what erratum 14 is.)
        assert_eq!(counts[0].1, 8, "ALL is every game");
        assert_eq!(
            counts[1..].iter().map(|(_, n)| n).sum::<usize>(),
            8,
            "the four states partition the history"
        );

        // A tab is a filter on what is DRAWN, and nothing else.
        landing.tab = Tab::Preparation;
        let visible: Vec<i64> = landing.visible().iter().map(|g| g.id).collect();
        assert_eq!(visible, vec![2, 6]);
        landing.tab = Tab::All;
        assert_eq!(landing.visible().len(), 8);
    }

    /// A mode and nothing else opens a blank draft; a mode this build cannot
    /// name is not a choice at all.
    #[test]
    fn a_mode_alone_opens_a_blank_draft() {
        let mut landing = Landing::new();
        assert_eq!(landing.proceed(), Proceed::Disabled(PROCEED_DISABLED));
        landing.choose_mode(GameMode::Maneuver);
        assert_eq!(landing.proceed(), Proceed::Blank(GameMode::Maneuver));
        assert!(landing.proceed().enabled());
        landing.choose_mode(GameMode::Unknown);
        assert_eq!(
            landing.mode(),
            Some(GameMode::Maneuver),
            "an unknown mode is not a choice"
        );
    }

    /// A ROW ALONE IS A COMPLETE ANSWER: a `planning` row reopens, any other
    /// row opens the console, and a row that vanishes takes the answer with it.
    #[test]
    fn a_planning_row_reopens_and_any_other_row_opens_the_console() {
        let mut landing = Landing::new();
        landing.set_games(vec![
            game(1, "Baltic Shield 26", "planning"),
            game(2, "Harbour Defence", "execution"),
        ]);
        landing.choose_mode(GameMode::Scenario);
        landing.select(1);
        assert_eq!(landing.proceed(), Proceed::Reopen(1));

        landing.select(2);
        match landing.proceed() {
            Proceed::Open { game_id, why } => {
                assert_eq!(game_id, 2);
                assert!(why.contains("NOT EDITABLE"), "{why}");
                assert!(why.contains("OPENS THE CONSOLE"), "{why}");
            }
            other => panic!("expected the console, got {other:?}"),
        }
        // A row that vanished from the list takes the selection with it.
        landing.set_games(vec![game(1, "Baltic Shield 26", "planning")]);
        assert_eq!(landing.selected(), None);
        assert_eq!(
            landing.proceed(),
            Proceed::Disabled(PROCEED_DISABLED),
            "the wizard was withdrawn by the row, so the verb is back to nothing chosen"
        );
    }

    /// THE LAST ANSWER WINS, AND THIS IS THE REPORTED DEFECT: a row clicked
    /// earlier stayed selected, so every mode the operator chose afterwards was
    /// ignored and `PROCEED` opened that game's console instead of the draft.
    #[test]
    fn answering_the_wizard_after_a_row_withdraws_the_row() {
        let mut landing = Landing::new();
        landing.set_games(vec![game(1, "Game 23", "planning")]);
        landing.select(1);
        assert_eq!(
            landing.proceed(),
            Proceed::Reopen(1),
            "the row was the answer"
        );

        landing.choose_mode(GameMode::Scenario);
        assert_eq!(landing.selected(), None, "the mode is the newer answer");
        landing.choose_start_from(StartFrom::Blank);
        assert_eq!(
            landing.proceed(),
            Proceed::Blank(GameMode::Scenario),
            "PROCEED must open the draft the operator just described"
        );

        // And the same for a mode that asks no second question.
        landing.select(1);
        landing.choose_mode(GameMode::Maneuver);
        assert_eq!(landing.proceed(), Proceed::Blank(GameMode::Maneuver));
    }

    /// The other direction: a row picked after the wizard withdraws the wizard's
    /// answer, so the one verb can never mean two things at once.
    #[test]
    fn picking_a_row_withdraws_the_wizards_answer() {
        let mut landing = Landing::new();
        landing.set_games(vec![game(1, "Game 23", "planning")]);
        landing.choose_mode(GameMode::Scenario);
        landing.choose_start_from(StartFrom::Mission(7));
        assert_eq!(landing.start_from(), StartFrom::Mission(7));

        landing.select(1);
        assert_eq!(landing.mode(), None, "the row is the newer answer");
        assert_eq!(landing.start_from(), StartFrom::Unchosen);
        assert_eq!(landing.proceed(), Proceed::Reopen(1));
    }

    /// ONLY A SCENARIO DRAFT ASKS START FROM, and it needs an answer even
    /// though `BLANK SCENARIO` is one of the answers.
    #[test]
    fn a_scenario_draft_needs_a_start_from_row() {
        let mut landing = Landing::new();
        landing.choose_mode(GameMode::Scenario);
        assert_eq!(landing.proceed(), Proceed::Disabled(START_FROM_UNCHOSEN));

        landing.choose_start_from(StartFrom::Blank);
        assert_eq!(landing.proceed(), Proceed::Blank(GameMode::Scenario));

        landing.set_missions(vec![mission(
            7,
            "Harbour Sweep",
            "Littoral",
            6,
            "2026-10-11T09:00:00Z",
        )]);
        landing.choose_start_from(StartFrom::Mission(7));
        assert_eq!(landing.proceed(), Proceed::SavedScenario(7));

        // Leaving the mode withdraws the answer: the list belongs to scenario.
        landing.choose_mode(GameMode::Static);
        assert_eq!(landing.start_from(), StartFrom::Unchosen);
        assert_eq!(landing.proceed(), Proceed::Blank(GameMode::Static));
        assert!(
            landing.start_rows().is_empty(),
            "STATIC and MANEUVER have no such list"
        );
    }

    /// A START FROM answer in a mode that never asked is refused rather than
    /// stored, so a static draft cannot be seeded from a Saved Scenario.
    #[test]
    fn a_start_from_answer_in_another_mode_is_refused() {
        let mut landing = Landing::new();
        landing.set_missions(vec![mission(
            7,
            "Harbour Sweep",
            "Littoral",
            6,
            "2026-10-11T09:00:00Z",
        )]);
        landing.choose_mode(GameMode::Maneuver);
        landing.choose_start_from(StartFrom::Mission(7));
        assert_eq!(landing.start_from(), StartFrom::Unchosen);
        assert_eq!(landing.proceed(), Proceed::Blank(GameMode::Maneuver));
    }

    /// The rows the list draws: `BLANK SCENARIO` first, then every Saved
    /// Scenario with its code, its step count and its date.
    #[test]
    fn the_start_from_rows_are_the_blank_one_and_the_saved_scenarios() {
        let mut landing = Landing::new();
        landing.choose_mode(GameMode::Scenario);
        landing.set_missions(vec![
            mission(7, "Baltic Shield 25", "Surface", 9, "2026-10-18T09:00:00Z"),
            mission(8, "North Sea Patrol", "Escort", 14, "2026-10-15T09:00:00Z"),
            mission(9, "Harbour Sweep", "Littoral", 1, "2026-10-11T09:00:00Z"),
        ]);
        let rows = landing.start_rows();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].choice, StartFrom::Blank);
        assert_eq!(rows[0].title, "BLANK SCENARIO");
        assert_eq!(rows[0].steps, "0 STEPS");
        assert!(rows[0].note.starts_with("No steps."));
        assert_eq!(rows[1].title, "Baltic Shield 25");
        assert_eq!(rows[1].code, "Surface");
        assert_eq!(rows[1].steps, "9 STEPS");
        assert_eq!(rows[1].when, "18 OCT");
        assert_eq!(rows[3].steps, "1 STEP", "one is not ones");
        assert!(rows[3].note.is_empty(), "only the blank row has a sentence");
    }

    /// A FAILED READ IS A STATE, not an empty list: drawing nothing would read
    /// as "this account has no Saved Scenarios", which may be false.
    #[test]
    fn a_failed_mission_read_says_so_rather_than_drawing_nothing() {
        let mut landing = Landing::new();
        landing.choose_mode(GameMode::Scenario);
        landing.missions_reading();
        assert_eq!(landing.missions_load, Load::Reading);
        landing.missions_failed("403 no read on /scenario-missions");
        assert!(matches!(landing.missions_load, Load::Failed(_)));
        // The blank row still exists: it needs no read.
        assert_eq!(landing.start_rows().len(), 1);
        assert_eq!(landing.start_rows()[0].choice, StartFrom::Blank);
    }

    /// A mission that vanished from a re-read takes a START FROM answer with
    /// it, like a vanished game takes the selection.
    #[test]
    fn a_vanished_mission_withdraws_the_start_from_answer() {
        let mut landing = Landing::new();
        landing.choose_mode(GameMode::Scenario);
        landing.set_missions(vec![mission(
            7,
            "Harbour Sweep",
            "",
            3,
            "2026-10-11T09:00:00Z",
        )]);
        landing.choose_start_from(StartFrom::Mission(7));
        landing.set_missions(vec![]);
        assert_eq!(landing.start_from(), StartFrom::Unchosen);
    }

    /// The three cards are the sheet's three, with its own lines.
    #[test]
    fn the_cards_are_the_sheets_three() {
        assert_eq!(MODE_CARDS.len(), 3);
        assert_eq!(MODE_CARDS[0].mode, GameMode::Static);
        assert_eq!(MODE_CARDS[1].mode, GameMode::Maneuver);
        assert_eq!(MODE_CARDS[2].mode, GameMode::Scenario);
        assert_eq!(
            MODE_CARDS[0].line,
            "Positions set by hand; orders are refused."
        );
        assert_eq!(MODE_CARDS[1].line, "The exercise integrates helm orders.");
        assert_eq!(MODE_CARDS[2].line, "Authored steps, sub-steps, audiences.");
    }

    /// `WHEN` is `created_at`, drawn as `23 OCT`, and an unreadable stamp draws
    /// nothing rather than a guess.
    #[test]
    fn when_is_the_creation_date_in_the_consoles_own_short_form() {
        assert_eq!(when_label("2026-10-23T09:41:00+07:00"), "23 OCT");
        assert_eq!(when_label("2026-01-02T00:00:00Z"), "02 JAN");
        assert_eq!(when_label("not a stamp"), "");
        assert_eq!(when_label(""), "");
    }

    /// The row's hint is the whole of what a click on a non-planning row means.
    #[test]
    fn every_state_has_a_hint_and_only_planning_reopens() {
        for state in ["planning", "preparation", "execution", "closure", "wargame"] {
            assert!(!row_hint(state).is_empty());
        }
        assert!(row_hint("planning").contains("REOPENS"));
        assert!(row_hint("preparation").contains("NOT EDITABLE"));
        assert!(row_hint("execution").contains("NOT EDITABLE"));
        assert!(row_hint("closure").contains("NOT EDITABLE"));
    }

    /// The landing holds no window and no draft: it is a question and a
    /// history. This is a compile-time fact with a test so the shape does not
    /// quietly grow one.
    #[test]
    fn the_landing_holds_no_draft() {
        let landing = Landing::new();
        assert!(landing.games().is_empty());
        assert!(landing.missions().is_empty());
        assert_eq!(landing.mode(), None);
        assert_eq!(landing.start_from(), StartFrom::Unchosen);
        assert_eq!(landing.selected(), None);
        // And the window type is not part of it: this line would not compile if
        // `TimeWindow` were reachable from the landing's state.
        let _not_a_window: Option<TimeWindow> = None;
    }
}
