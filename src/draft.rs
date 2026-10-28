//! The creation flow's draft: one local object from the landing to the press.
//!
//! THE DRAFT IS LOCAL UNTIL THE PRESS. After sign-in the console asks one
//! question — the mode — and answering it opens a draft. Every stage edits
//! memory: the terms, the force, the task organisation, the book and the
//! roster are all staged, and the map's pins are the draft's own. Nothing is
//! written until one of the three verbs is pressed in stage 06, and there is
//! no save: closing the window loses the draft
//! (`docs/creation-flow-handoff.md` §1, §3.6).
//!
//! This module is the SHAPE of that object and the few rules that are about
//! the draft as a whole rather than about one collection:
//!
//! - the [`Origin`], which decides what is prefilled and what a press does;
//! - the mode, which is create-only and fixes how many stages the rail has;
//! - the [`Terms`] and their derived [`TimeBase`] — the operator types two
//!   starts, a factor and a length, and NO END IS EVER TYPED (§4.1);
//! - the [`Verb`]s the mode earns, and the [`Blocker`]s that stop them;
//! - the [`Journal`]: what a previous press already wrote, so a resumed
//!   press never re-sends a write the server answered.
//!
//! The collections themselves are the modules that already own them —
//! [`crate::force`], [`crate::groups`], [`crate::book`], [`crate::roster`] —
//! and they are EXTENDED here, not duplicated. Each of those is already a
//! diff against a live mirror, with its own ordered write plan; the draft
//! is what holds the four of them together under one mode, one origin and
//! one window.
//!
//! WHY THE CHECKS ARE HERE AND NOT IN THE STAGES. A stage that refuses a
//! half-set window is a stage the operator can leave; the verb is the thing
//! that cannot run, so [`Draft::blockers`] is the answer to "what is still
//! missing" for the whole draft, and REVIEW renders it. The rules are the
//! server's — they name the field or the hull the server would name, and
//! they are checked before the write rather than after a 400 (§9.3).
//!
//! Nothing here talks to the network and nothing here knows what a pixel is.

use std::collections::{BTreeMap, BTreeSet};

use crate::backend::{
    GameDetail, GamePace, TimeWindow, actual_to_rfc3339, assumed_hhmm_to_rfc3339, date_ok,
};
use crate::book::BookDraft;
use crate::force::{ForceDraft, GapReason, Start};
use crate::gamestate::GameMode;
use crate::groups::{GroupKind, GroupsDraft};
use crate::roster::RosterDraft;

// ===========================================================================
// The origin, the stages and the verbs
// ===========================================================================

/// Where a draft came from. Exactly one, and it decides what is prefilled and
/// what the press will do (§3.2).
///
/// `Blank` writes every stage; `SavedScenario` starts from a server-side
/// detached mission and its hulls enter the force HOLLOW — in the game, no
/// position — because a Saved Scenario has no positions at all; `Reopened`
/// starts from a game in `planning` and a press is a DIFF against what the
/// server already holds rather than a create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The landing's mode, with nothing behind it.
    Blank { mode: GameMode },
    /// A START FROM row: a detached mission, which is what a Saved Scenario
    /// is on the server.
    SavedScenario { mission_id: i64 },
    /// A `planning` row on the landing, reopened.
    Reopened { game_id: i64 },
}

impl Origin {
    /// The line the draft's header carries beside `DRAFT`.
    pub fn note(self) -> String {
        match self {
            Origin::Blank { .. } => "NO GAME RECORDED".to_string(),
            Origin::SavedScenario { .. } => "FROM A SAVED SCENARIO".to_string(),
            Origin::Reopened { game_id } => format!("GAME {game_id}"),
        }
    }

    /// Whether the press is a create rather than a diff.
    pub fn is_create(self) -> bool {
        !matches!(self, Origin::Reopened { .. })
    }

    pub fn game_id(self) -> Option<i64> {
        match self {
            Origin::Reopened { game_id } => Some(game_id),
            _ => None,
        }
    }
}

/// One stage of the rail.
///
/// THE RAIL NAVIGATES: a stage is reached by clicking it, not by finishing
/// the one before it. There is no auto-advance and no forward lock; the only
/// gated things are the verbs (§1). So a stage is not a step in a wizard and
/// carries no "next".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Essentials,
    Units,
    Groups,
    /// Scenario mode only. A static or maneuver draft has five stages.
    Scenario,
    Players,
    Review,
}

const PLAIN_STAGES: [Stage; 5] = [
    Stage::Essentials,
    Stage::Units,
    Stage::Groups,
    Stage::Players,
    Stage::Review,
];

const SCENARIO_STAGES: [Stage; 6] = [
    Stage::Essentials,
    Stage::Units,
    Stage::Groups,
    Stage::Scenario,
    Stage::Players,
    Stage::Review,
];

impl Stage {
    /// The stages this mode's rail draws, in order.
    ///
    /// The NUMBER on the rail is the position in this list plus one, which is
    /// why the number is not stored anywhere: a stored number is a number
    /// that can disagree with the list, and the flow's stage numbers move with
    /// the mode while the WRITE LADDER's numbers do not (§1).
    pub fn all(mode: GameMode) -> &'static [Stage] {
        if mode.is_scenario() {
            &SCENARIO_STAGES
        } else {
            &PLAIN_STAGES
        }
    }

    /// This stage's number on a rail for `mode`, or `None` when the mode has
    /// no such stage.
    pub fn number_in(self, mode: GameMode) -> Option<u8> {
        Stage::all(mode)
            .iter()
            .position(|s| *s == self)
            .map(|at| at as u8 + 1)
    }

    /// The rail's label.
    pub fn label(self) -> &'static str {
        match self {
            Stage::Essentials => "ESSENTIALS",
            Stage::Units => "UNITS",
            Stage::Groups => "GROUPS",
            Stage::Scenario => "SCENARIO",
            Stage::Players => "PLAYERS",
            Stage::Review => "REVIEW",
        }
    }
}

/// One of the presses in stage 06. Each stops at a different place (§1).
///
/// A static or maneuver draft earns two of them: `Save as Scenario` is not
/// offered at all, because the server refuses a scenario on a non-scenario
/// game with a 409 and a Saved Scenario has no mode to belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    SaveAsGameSession,
    SaveAsScenario,
    PlayDirectly,
}

const PLAIN_VERBS: [Verb; 2] = [Verb::SaveAsGameSession, Verb::PlayDirectly];
const SCENARIO_VERBS: [Verb; 3] = [
    Verb::SaveAsGameSession,
    Verb::SaveAsScenario,
    Verb::PlayDirectly,
];

impl Verb {
    /// The verbs this mode earns, in the order stage 06 draws them.
    pub fn all(mode: GameMode) -> &'static [Verb] {
        if mode.is_scenario() {
            &SCENARIO_VERBS
        } else {
            &PLAIN_VERBS
        }
    }

    /// The verb's name. Title case, because it is a press and not a label.
    pub fn label(self) -> &'static str {
        match self {
            Verb::SaveAsGameSession => "Save as Game Session",
            Verb::SaveAsScenario => "Save as Scenario",
            Verb::PlayDirectly => "Play Directly",
        }
    }

    /// Where the press leaves the console (§10).
    pub fn lands(self) -> &'static str {
        match self {
            Verb::SaveAsGameSession => "LANDS IN PLANNING",
            Verb::SaveAsScenario => "LANDS IN SAVED SCENARIOS",
            Verb::PlayDirectly => "LANDS IN PREPARATION",
        }
    }

    /// Which ladder this verb walks.
    pub fn ladder(self) -> Ladder {
        match self {
            Verb::SaveAsScenario => Ladder::Mission,
            Verb::SaveAsGameSession | Verb::PlayDirectly => Ladder::Game,
        }
    }

    /// The last ladder position this verb writes. The mission ladder's three
    /// positions are its whole length.
    pub fn stops_at(self) -> u8 {
        match self {
            Verb::SaveAsScenario => 3,
            Verb::SaveAsGameSession => 7,
            Verb::PlayDirectly => 9,
        }
    }
}

/// The two write orders (§9.2). The numbers are a different series from the
/// rail's, and they do not move with the mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Ladder {
    /// 01 GAME, 02 SEATS, 03 HIERARCHY, 04 UNITS, 05 PLACEMENTS, 06 STEPS,
    /// 07 COMMANDERS, 08 READINESS (a read), 09 TRANSITION.
    Game,
    /// M1 MISSION, M2 STEPS, M3 COMMANDS + AUDIENCES.
    Mission,
}

impl Ladder {
    /// How many positions the ladder has, read included.
    pub fn length(self) -> u8 {
        match self {
            Ladder::Game => 9,
            Ladder::Mission => 3,
        }
    }
}

// ===========================================================================
// The time base
// ===========================================================================

/// The ten fields of stage 01, in the order the form draws them.
///
/// The order is here and not in the renderer for the reason every field list
/// in this console is: the keystrokes, the draft and the scroll counter have
/// to agree about which field is which, and two lists of one order are two
/// lists waiting to disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermField {
    Name,
    Description,
    Purpose,
    Target,
    Area,
    MapTag,
    ActualStart,
    AssumedStart,
    TimeFactor,
    Length,
}

impl TermField {
    pub const ALL: [TermField; 10] = [
        TermField::Name,
        TermField::Description,
        TermField::Purpose,
        TermField::Target,
        TermField::Area,
        TermField::MapTag,
        TermField::ActualStart,
        TermField::AssumedStart,
        TermField::TimeFactor,
        TermField::Length,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TermField::Name => "NAME",
            TermField::Description => "DESCRIPTION",
            TermField::Purpose => "PURPOSE",
            TermField::Target => "TARGET",
            TermField::Area => "AREA",
            TermField::MapTag => "MAP TAG",
            TermField::ActualStart => "ACTUAL START",
            TermField::AssumedStart => "ASSUMED START",
            TermField::TimeFactor => "TIME FACTOR",
            TermField::Length => "LENGTH",
        }
    }

    /// The right-aligned hint beside the well.
    ///
    /// F2a's `FIELD INVENTORY` panel is a drawing aid, not a UI panel: the
    /// same facts belong on each field, which is where this goes (§4.2).
    pub fn hint(self) -> &'static str {
        match self {
            TermField::Name => "REQUIRED \u{b7} 255 MAX",
            TermField::Description | TermField::Purpose | TermField::Target => {
                "OPTIONAL \u{b7} 4000"
            }
            TermField::Area | TermField::MapTag => "OPT \u{b7} HIDDEN",
            TermField::ActualStart
            | TermField::AssumedStart
            | TermField::TimeFactor
            | TermField::Length => "OPT \u{b7} EXEC",
        }
    }

    /// The server's own limit on the field, or `None` for a field that is not
    /// free text.
    pub fn max_len(self) -> Option<usize> {
        match self {
            TermField::Name => Some(255),
            TermField::Description | TermField::Purpose | TermField::Target => Some(4000),
            TermField::Area => Some(1000),
            TermField::MapTag => Some(255),
            _ => None,
        }
    }
}

/// The four numbers that make the exercise's window — the operator's, as
/// typed.
///
/// THE ENDS ARE NEVER TYPED (§4.1). An operator types two starts, a factor
/// and a length; both ends are derived, and a factor change moves the
/// ASSUMED end and the assumed clock and never the real window. F2b's
/// `ELAPSED · DERIVED, NOT TYPED from the two starts and the factor` is true
/// as a display and would be one input short as a rule — something has to say
/// how long the exercise runs, and no frame has that field. `length` (real
/// hours) is that input: the smallest addition that keeps the invariant.
///
/// The assumed clock carries a fixed base date in the wire (only its TIME is
/// meaningful — nothing compares the two clocks), so the assumption is kept
/// here as a time of day and the date stays the backend's business.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TimeBase {
    /// The real clock's day, `YYYY-MM-DD`.
    pub actual_date: String,
    /// The real clock's start, `HHMM`.
    pub actual_start: String,
    /// The exercise clock's start, `HHMM`.
    pub assumed_start: String,
    /// Assumed time ÷ real time. Above zero.
    pub factor: String,
    /// Real hours the exercise runs for. Above zero.
    pub length: String,
}

/// What a time base is, as one value: EMPTY, or COMPLETE, or being typed.
///
/// "The time base is one group: empty, or complete." A half-set base is a
/// draft state the form may hold while the operator types, and REVIEW names
/// it; it is never sent half-set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseState {
    Empty,
    Partial,
    Complete,
}

impl TimeBase {
    /// Whether nothing has been typed.
    pub fn is_empty(&self) -> bool {
        self.actual_date.trim().is_empty()
            && self.actual_start.trim().is_empty()
            && self.assumed_start.trim().is_empty()
            && self.factor.trim().is_empty()
            && self.length.trim().is_empty()
    }

    /// The two ends, as numbers, or `None` when either is not a number above
    /// zero.
    pub fn factor(&self) -> Option<f64> {
        positive(&self.factor)
    }

    pub fn length_hours(&self) -> Option<f64> {
        positive(&self.length)
    }

    /// Assumed time ÷ real time, for the derived ends and for display.
    pub fn elapsed_hours(&self) -> Option<f64> {
        Some(self.length_hours()? * self.factor()?)
    }

    /// Whether the two starts name what they have to name.
    pub fn starts_are_real(&self) -> bool {
        date_ok(self.actual_date.trim())
            && crate::view::hhmm_ok(self.actual_start.trim())
            && crate::view::hhmm_ok(self.assumed_start.trim())
    }

    pub fn state(&self) -> BaseState {
        if self.is_empty() {
            return BaseState::Empty;
        }
        if self.starts_are_real() && self.length_hours().is_some() && self.factor().is_some() {
            BaseState::Complete
        } else {
            BaseState::Partial
        }
    }

    /// What is wrong with a half-set base, in the operator's own words.
    ///
    /// `None` for both of the legal states: an empty base is an ordinary
    /// planning state (the server allows it and the execution gate refuses
    /// without it) and a complete one has nothing to say.
    ///
    /// The checks run in the order the form draws the fields, so the sentence
    /// names the first well the operator would look at.
    pub fn problem(&self) -> Option<String> {
        if self.state() != BaseState::Partial {
            return None;
        }
        let date = self.actual_date.trim();
        let real = self.actual_start.trim();
        let assumed = self.assumed_start.trim();
        if date.is_empty() && real.is_empty() {
            return Some(
                "The real clock has no start \u{2014} both ends of the window are derived from it."
                    .to_string(),
            );
        }
        if date.is_empty() != real.is_empty() {
            return Some("The real clock needs a day and a time together.".to_string());
        }
        if !date_ok(date) {
            return Some(format!("{date} is not a day like 2026-11-01."));
        }
        if !crate::view::hhmm_ok(real) {
            return Some(format!("{real} is not a military time like 0930."));
        }
        if assumed.is_empty() {
            return Some(
                "The exercise clock has no start \u{2014} it cannot be derived from the real one."
                    .to_string(),
            );
        }
        if !crate::view::hhmm_ok(assumed) {
            return Some(format!("{assumed} is not a military time like 0930."));
        }
        if self.length_hours().is_none() {
            return Some(
                "LENGTH must be a number of hours above zero \u{2014} both ends are derived from it."
                    .to_string(),
            );
        }
        if self.factor().is_none() {
            return Some(
                "TIME FACTOR must be a number above zero \u{2014} it is assumed time over real time."
                    .to_string(),
            );
        }
        Some("The time base is half set.".to_string())
    }

    /// Both ends, derived. `None` unless the base is complete.
    ///
    /// `offset` is the frame the instants are written in — the server's own
    /// when it stated one, the client's otherwise (`backend::local_offset`).
    /// It is a parameter rather than a lookup so this module stays a pure
    /// function of its input.
    pub fn window(&self, offset: &str) -> Option<TimeWindow> {
        if self.state() != BaseState::Complete {
            return None;
        }
        let actual_start =
            actual_to_rfc3339(self.actual_date.trim(), self.actual_start.trim(), offset)?;
        let assumed_start = assumed_hhmm_to_rfc3339(self.assumed_start.trim(), offset)?;
        let real_seconds = self.length_hours()? * 3600.0;
        let assumed_seconds = self.elapsed_hours()? * 3600.0;
        Some(TimeWindow {
            actual_end: after_seconds(&actual_start, real_seconds),
            assumed_end: after_seconds(&assumed_start, assumed_seconds),
            actual_start: Some(actual_start),
            assumed_start: Some(assumed_start),
        })
    }

    /// The base as a game's detail read states it.
    ///
    /// The length is not on the wire, so it is DERIVED FROM THE TWO REAL ENDS
    /// — the inverse of [`TimeBase::window`], and the one direction in which
    /// the ends are read rather than computed. A window the server holds
    /// incompletely leaves the length blank, which is the honest reading: the
    /// draft cannot invent how long an exercise runs.
    pub fn from_detail(detail: &GameDetail) -> Self {
        let clock = |value: &Option<String>| value.as_deref().and_then(crate::window::read_clock);
        let actual = clock(&detail.window.actual_start);
        let assumed = clock(&detail.window.assumed_start);
        let length = match (
            detail.window.actual_start.as_deref(),
            detail.window.actual_end.as_deref(),
        ) {
            (Some(start), Some(end)) => hours_between(start, end),
            _ => None,
        };
        TimeBase {
            actual_date: actual.as_ref().map(|(d, _)| d.clone()).unwrap_or_default(),
            actual_start: actual.map(|(_, t)| t).unwrap_or_default(),
            assumed_start: assumed.map(|(_, t)| t).unwrap_or_default(),
            factor: if detail.time_factor > 0.0 {
                format!("{}", detail.time_factor)
            } else {
                String::new()
            },
            length: length.map(|h| format!("{h}")).unwrap_or_default(),
        }
    }
}

/// A number above zero, or `None`.
fn positive(text: &str) -> Option<f64> {
    let value: f64 = text.trim().parse().ok()?;
    (value.is_finite() && value > 0.0).then_some(value)
}

/// An RFC3339 instant moved forward by `seconds`, in the same frame.
fn after_seconds(instant: &str, seconds: f64) -> Option<String> {
    let at = chrono::DateTime::parse_from_rfc3339(instant).ok()?;
    let moved = at + chrono::Duration::seconds(seconds.round() as i64);
    Some(moved.to_rfc3339_opts(chrono::SecondsFormat::Secs, false))
}

/// The hours between two RFC3339 instants, or `None` when either is not one.
fn hours_between(start: &str, end: &str) -> Option<f64> {
    let a = chrono::DateTime::parse_from_rfc3339(start).ok()?;
    let b = chrono::DateTime::parse_from_rfc3339(end).ok()?;
    let seconds = (b - a).num_seconds();
    (seconds > 0).then(|| seconds as f64 / 3600.0)
}

/// The terms as the operator edits them — the six pieces of prose and the
/// time base.
///
/// The mode is not here: it is the draft's, fixed at creation, and shown as a
/// stamp the form never offers to change.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Terms {
    pub name: String,
    pub description: String,
    pub purpose: String,
    pub target: String,
    /// Hidden from participants until execution.
    pub area: String,
    /// Hidden until execution.
    pub map_tag: String,
    pub time: TimeBase,
}

impl Terms {
    pub fn blank() -> Self {
        Self::default()
    }

    /// The terms a `Reopened` draft opens on: the server's own words, so the
    /// operator edits a plan rather than retyping one.
    pub fn from_detail(detail: &GameDetail) -> Self {
        Terms {
            name: detail.name.clone(),
            description: detail.description.clone(),
            purpose: detail.purpose.clone(),
            target: detail.target.clone(),
            area: detail.area.clone().unwrap_or_default(),
            map_tag: detail.map_tag.clone().unwrap_or_default(),
            time: TimeBase::from_detail(detail),
        }
    }

    /// One field as the form shows it.
    ///
    /// The two casts are worth it: ACTUAL START reads as its day and its time
    /// together, because that is one row on the form, while the model keeps
    /// them apart because they are one day and one time.
    pub fn text(&self, field: TermField) -> String {
        match field {
            TermField::Name => self.name.clone(),
            TermField::Description => self.description.clone(),
            TermField::Purpose => self.purpose.clone(),
            TermField::Target => self.target.clone(),
            TermField::Area => self.area.clone(),
            TermField::MapTag => self.map_tag.clone(),
            TermField::ActualStart => {
                match (self.time.actual_date.trim(), self.time.actual_start.trim()) {
                    ("", "") => String::new(),
                    (day, time) => format!("{day} {time}").trim().to_string(),
                }
            }
            TermField::AssumedStart => self.time.assumed_start.clone(),
            TermField::TimeFactor => self.time.factor.clone(),
            TermField::Length => self.time.length.clone(),
        }
    }

    /// Put one field.
    ///
    /// The form drives ACTUAL START through the calendar and the time grid
    /// (`terms.time.actual_date` and `terms.time.actual_start`), and through
    /// this only when a whole day-and-time arrives at once — a paste. A value
    /// that is a bare time sets the time and leaves the day alone, because
    /// that is what a time well does.
    pub fn set_text(&mut self, field: TermField, value: String) {
        match field {
            TermField::Name => self.name = value,
            TermField::Description => self.description = value,
            TermField::Purpose => self.purpose = value,
            TermField::Target => self.target = value,
            TermField::Area => self.area = value,
            TermField::MapTag => self.map_tag = value,
            TermField::ActualStart => match value.split_once(|c: char| c == 'T' || c == ' ') {
                Some((day, time)) => {
                    self.time.actual_date = day.trim().to_string();
                    self.time.actual_start = time.trim().to_string();
                }
                None => self.time.actual_start = value.trim().to_string(),
            },
            TermField::AssumedStart => self.time.assumed_start = value,
            TermField::TimeFactor => self.time.factor = value,
            TermField::Length => self.time.length = value,
        }
    }

    /// The server refuses a nameless game, so the draft does too.
    pub fn name_ok(&self) -> bool {
        !self.name.trim().is_empty()
    }

    /// Whether every required field is filled, and no field is over its
    /// limit. Length is the server's, not a matter of taste: a 256-character
    /// name is a 400 the operator cannot see coming.
    pub fn too_long(&self) -> Option<(TermField, usize)> {
        TermField::ALL.iter().find_map(|f| {
            let limit = f.max_len()?;
            (self.text(*f).chars().count() > limit).then_some((*f, limit))
        })
    }
}

// ===========================================================================
// The journal
// ===========================================================================

/// What one ladder position left behind.
#[derive(Debug, Clone, PartialEq)]
pub enum Landing {
    /// The server answered and gave an id, where the position makes one.
    Written { id: Option<i64> },
    /// Position 08 of the game ladder: the readiness gate. A READ, so it is
    /// never "written" — a counter that said it was would be lying about the
    /// one position whose whole point is that nobody writes it (§9.1).
    Read,
    /// The server refused or did not answer. `502` has no envelope, so the
    /// status and the `X-Request-ID` are all there is (§9.1).
    Failed {
        status: u16,
        request_id: Option<String>,
    },
}

impl Landing {
    /// Whether the position is settled: written, or read.
    pub fn settled(&self) -> bool {
        matches!(self, Landing::Written { .. } | Landing::Read)
    }
}

/// What a previous press already wrote (§9.3).
///
/// A write the journal says succeeded is NEVER re-sent, and a resumed press
/// reads the journal rather than the draft's optimism: the draft says what
/// the operator intends, the journal says what the server agreed to. The
/// resolved ids are here for the same reason — a staged group's node id and a
/// staged step's id exist only after the answer that made them, and every
/// later write in the same press names them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Journal {
    landings: BTreeMap<(Ladder, u8), Landing>,
    game_id: Option<i64>,
    mission_id: Option<i64>,
    scenario_id: Option<i64>,
    /// Staged group (negative local id) → the hierarchy node the server made.
    nodes: BTreeMap<i64, i64>,
    /// Staged book (negative local id) → the scenario or mission the server
    /// made. A book's own id is what its steps name, and a draft may hold more
    /// than one (`Ref::Staged` in `crate::commit`).
    books: BTreeMap<i64, i64>,
    /// Staged step (negative local id) → the step the server made.
    steps: BTreeMap<i64, i64>,
    /// Staged sub-step (negative local id) → the sub-step the server made.
    substeps: BTreeMap<i64, i64>,
}

impl Journal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn landing(&self, ladder: Ladder, position: u8) -> Option<&Landing> {
        self.landings.get(&(ladder, position))
    }

    pub fn note_written(&mut self, ladder: Ladder, position: u8, id: Option<i64>) {
        self.landings
            .insert((ladder, position), Landing::Written { id });
    }

    pub fn note_read(&mut self, ladder: Ladder, position: u8) {
        self.landings.insert((ladder, position), Landing::Read);
    }

    pub fn note_failed(
        &mut self,
        ladder: Ladder,
        position: u8,
        status: u16,
        request_id: Option<String>,
    ) {
        self.landings
            .insert((ladder, position), Landing::Failed { status, request_id });
    }

    /// How many positions are settled, read included. The `6 OF 9` of F7.
    pub fn settled(&self, ladder: Ladder) -> u8 {
        self.landings
            .iter()
            .filter(|((l, _), landing)| *l == ladder && landing.settled())
            .count() as u8
    }

    /// The first position a resumed press owes: the lowest one that is not
    /// settled, or one past the end when the ladder is done.
    ///
    /// A FAILED position is owed again — resume re-plans from the journal and
    /// re-issues from there, and it never re-sends the positions below (§9.1).
    pub fn next_owed(&self, ladder: Ladder) -> u8 {
        (1..=ladder.length())
            .find(|position| {
                !self
                    .landings
                    .get(&(ladder, *position))
                    .is_some_and(Landing::settled)
            })
            .unwrap_or(ladder.length() + 1)
    }

    pub fn is_empty(&self) -> bool {
        self.landings.is_empty()
            && self.game_id.is_none()
            && self.mission_id.is_none()
            && self.scenario_id.is_none()
            && self.nodes.is_empty()
            && self.books.is_empty()
            && self.steps.is_empty()
            && self.substeps.is_empty()
    }

    pub fn clear(&mut self) {
        *self = Journal::default();
    }

    /// Adopt a game that a lost 01 created anyway.
    ///
    /// `POST /games` has no idempotency key, so a timeout on 01 may have made
    /// the game regardless. The rule is to re-read by name and, on exactly one
    /// recent match, adopt it and continue at 02 — never to offer a bare
    /// "retry 01", and never to guess between two (§9.3).
    pub fn adopt_game(&mut self, game_id: i64) {
        self.game_id = Some(game_id);
        self.note_written(Ladder::Game, 1, Some(game_id));
    }

    pub fn game_id(&self) -> Option<i64> {
        self.game_id
    }

    pub fn set_game_id(&mut self, game_id: Option<i64>) {
        self.game_id = game_id;
    }

    pub fn mission_id(&self) -> Option<i64> {
        self.mission_id
    }

    pub fn set_mission_id(&mut self, mission_id: Option<i64>) {
        self.mission_id = mission_id;
    }

    pub fn scenario_id(&self) -> Option<i64> {
        self.scenario_id
    }

    pub fn set_scenario_id(&mut self, scenario_id: Option<i64>) {
        self.scenario_id = scenario_id;
    }

    /// Record the node a staged group became.
    pub fn resolve_node(&mut self, local_id: i64, node_id: i64) {
        self.nodes.insert(local_id, node_id);
    }

    pub fn node(&self, local_id: i64) -> Option<i64> {
        self.nodes.get(&local_id).copied()
    }

    /// Record the scenario or mission a staged book became.
    pub fn resolve_book(&mut self, local_id: i64, book_id: i64) {
        self.books.insert(local_id, book_id);
    }

    pub fn book(&self, local_id: i64) -> Option<i64> {
        self.books.get(&local_id).copied()
    }

    pub fn resolve_step(&mut self, local_id: i64, step_id: i64) {
        self.steps.insert(local_id, step_id);
    }

    pub fn step(&self, local_id: i64) -> Option<i64> {
        self.steps.get(&local_id).copied()
    }

    pub fn resolve_substep(&mut self, local_id: i64, substep_id: i64) {
        self.substeps.insert(local_id, substep_id);
    }

    pub fn substep(&self, local_id: i64) -> Option<i64> {
        self.substeps.get(&local_id).copied()
    }
}

// ===========================================================================
// The draft
// ===========================================================================

/// Why a verb cannot run. The stage the operator can fix it in, and the
/// sentence they are told — the sentence names a field, a hull or a body, so
/// it can be acted on rather than looked up.
#[derive(Debug, Clone, PartialEq)]
pub struct Blocker {
    pub stage: Stage,
    pub sentence: String,
}

/// One draft: the landing's answer, the five or six stages' work, and the
/// journal of what a previous press wrote.
///
/// THE MODE IS FIXED. The server takes `mode` on create and never again, so a
/// draft's mode is set by its origin and there is no setter. A mode this
/// build cannot name is not a draft at all — hence the constructors answer
/// `Option`, and `None` is the whole of the refusal.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub origin: Origin,
    /// Never [`GameMode::Unknown`]: see [`Draft::blank`].
    pub mode: GameMode,
    /// Standard by default. `fast` waives exactly one gate condition and
    /// nothing else, so it is on the stamp row where the operator can see why
    /// their gate is shaped differently (§12.2).
    pub pace: GamePace,
    /// `max_echelon_rank` — a RANK NUMBER, e.g. 40, not a hierarchy id
    /// (§12.7). Set in stage 03, where it has meaning, and sent on create.
    pub ceiling: Option<i64>,
    pub terms: Terms,
    pub force: ForceDraft,
    pub groups: GroupsDraft,
    /// The book, as [`crate::book`] owns it.
    ///
    /// That module is STEP-level today and the flow's stage 04 is step and
    /// SUB-STEP; giving the book a frame and a sub-step (§7, the handoff's
    /// slice 9) is an extension of it, not a second model. The rail's `04`
    /// count reads [`BookDraft::staged_step_count`] until then, so it counts
    /// what the draft has staged rather than the whole book.
    pub book: BookDraft,
    pub roster: RosterDraft,
    pub journal: Journal,
    /// The Saved Scenario's own title, once the operator has edited it.
    ///
    /// `None` FOLLOWS THE GAME'S TITLE. Stage 06 draws the field prefilled
    /// from `terms.name`, and the first keystroke makes it the mission's:
    /// asking an operator to type the name they typed in stage 01 again is
    /// the kind of work that produces two slightly different names for one
    /// plan. `None` rather than a copy, so the field follows a name changed
    /// after the field was first drawn.
    pub scenario_title: Option<String>,
    /// The Saved Scenario's own description, on the same terms.
    pub scenario_description: Option<String>,
    /// The steps the SERVER already holds for this game, less the ones this
    /// draft cuts.
    ///
    /// [`BookDraft`] carries the operator's adds and cuts, not the read: a
    /// reopened game's steps live in `SessionView::book`. This is the count
    /// the rail and the blockers need, seeded by whoever holds the read.
    pub held_steps: usize,
}

impl Draft {
    /// A blank draft for a mode the landing offered.
    ///
    /// `None` for [`GameMode::Unknown`]: the landing cannot offer a mode it
    /// could not name, and a draft that held one would be a draft whose every
    /// rule is undefined. `NoSession` is not a mode at all.
    pub fn blank(mode: GameMode) -> Option<Self> {
        if mode == GameMode::Unknown {
            return None;
        }
        Some(Draft {
            origin: Origin::Blank { mode },
            mode,
            pace: GamePace::Standard,
            ceiling: None,
            terms: Terms::blank(),
            force: ForceDraft::new(),
            groups: GroupsDraft::new(),
            book: BookDraft::new(),
            roster: RosterDraft::new(),
            journal: Journal::new(),
            scenario_title: None,
            scenario_description: None,
            held_steps: 0,
        })
    }

    /// A draft seeded from a Saved Scenario: `GET /scenario-missions/{id}`.
    ///
    /// A Saved Scenario is a scenario and nothing else, so the mode is not a
    /// question here. Its hulls enter the force HOLLOW — in the game, no
    /// position — because a Saved Scenario has no positions at all, and its
    /// steps and audiences enter the book. The seeding itself is the caller's
    /// (it holds the read); this is the head of it.
    pub fn saved_scenario(mission_id: i64, title: &str, description: &str) -> Self {
        let terms = Terms {
            name: title.to_string(),
            description: description.to_string(),
            ..Terms::blank()
        };
        Draft {
            origin: Origin::SavedScenario { mission_id },
            mode: GameMode::Scenario,
            pace: GamePace::Standard,
            ceiling: None,
            terms,
            force: ForceDraft::new(),
            groups: GroupsDraft::new(),
            book: BookDraft::new(),
            roster: RosterDraft::new(),
            journal: Journal::new(),
            scenario_title: None,
            scenario_description: None,
            held_steps: 0,
        }
    }

    /// A draft reopened from a game in `planning` (§3.2).
    ///
    /// `None` when the read names no mode this build knows, or a state that
    /// is not `planning`: after planning the terms, the force and the tree are
    /// fixed, the server refuses the writes with a 409, and the landing opens
    /// the console instead of drafting. The origin is what makes the press a
    /// diff, so a draft that cannot diff is not opened at all.
    pub fn reopened(detail: &GameDetail) -> Option<Self> {
        let mode = GameMode::from_wire(&detail.mode);
        if mode == GameMode::Unknown || detail.state != "planning" {
            return None;
        }
        Some(Draft {
            origin: Origin::Reopened { game_id: detail.id },
            mode,
            pace: detail.pace.unwrap_or(GamePace::Standard),
            // Read, not re-picked: the ceiling is a game field, and a draft
            // that reopened the game already has one. `None` is the server
            // stating none, which is "no ceiling" rather than rank zero.
            ceiling: detail.max_echelon_rank,
            terms: Terms::from_detail(detail),
            force: ForceDraft::new(),
            groups: GroupsDraft::new(),
            book: BookDraft::new(),
            roster: RosterDraft::new(),
            journal: Journal::new(),
            scenario_title: None,
            scenario_description: None,
            held_steps: 0,
        })
    }

    /// The Saved Scenario's title: the game's own until the operator edits it.
    pub fn saved_title(&self) -> &str {
        match &self.scenario_title {
            Some(title) => title.as_str(),
            None => self.terms.name.trim(),
        }
    }

    /// The Saved Scenario's description, on the same terms.
    pub fn saved_description(&self) -> &str {
        match &self.scenario_description {
            Some(description) => description.as_str(),
            None => self.terms.description.trim(),
        }
    }

    /// Take the title into the operator's hands.
    pub fn set_saved_title(&mut self, title: String) {
        self.scenario_title = Some(title);
    }

    /// Take the description into the operator's hands.
    pub fn set_saved_description(&mut self, description: String) {
        self.scenario_description = Some(description);
    }

    /// The rail's stages, in order.
    pub fn stages(&self) -> &'static [Stage] {
        Stage::all(self.mode)
    }

    /// How many stages this draft's rail has: five, or six for a scenario.
    pub fn stage_count(&self) -> usize {
        self.stages().len()
    }

    /// The verbs this draft earns: two, or three for a scenario.
    pub fn verbs(&self) -> &'static [Verb] {
        Verb::all(self.mode)
    }

    /// Whether the draft is worth asking about before the window closes.
    ///
    /// The draft is never persisted (§3.6), so closing with work in it asks
    /// once — `Discard this draft?` — and that question is about whether
    /// anything would be lost.
    pub fn is_empty(&self) -> bool {
        self.terms == Terms::blank()
            && self.ceiling.is_none()
            && self.force.is_empty()
            && self.groups.is_empty()
            && self.book.is_empty()
            && self.roster.is_empty()
            && self.journal.is_empty()
            && self.scenario_title.is_none()
            && self.scenario_description.is_none()
            && self.held_steps == 0
    }

    // -- the counts the rail shows ---------------------------------------

    /// `02 UNITS` — the force's size.
    pub fn unit_count(&self) -> usize {
        self.force.len()
    }

    /// TAKE A HULL OUT OF THE EXERCISE, AND SEVER EVERY TIE IT HAS.
    ///
    /// A HULL IS NAMED IN FOUR PLACES: the force, a body's muster list, a
    /// sub-step's audience, and some other hull's `host`. Removing it from
    /// the force alone left the other three naming an id the draft no longer
    /// held, and the basket drew that id as `UNIT 351` under a body that
    /// still claimed it. The ties are cut here, once, because a control that
    /// removes a hull and forgets one of them is a control that leaves a
    /// ghost.
    ///
    /// RIDERS COME OFF, THEY DO NOT VANISH. A hull taken out cannot carry
    /// anything afterwards — the attach endpoint 404s on a host the exercise
    /// does not hold — so anything aboard it returns to the water rather
    /// than staying attached to a hull nobody can find.
    ///
    /// Answers whether the force held it. A hull that was only in a body (a
    /// state no control builds, but a resumed draft can hold) still has its
    /// ties cut.
    pub fn take_out(&mut self, unit_id: i64) -> bool {
        let held = self.force.remove(unit_id).is_some();
        // Every body that musters it, in one pass over the ids: `set_units`
        // borrows the draft mutably, so the list is collected first.
        let owners: Vec<i64> = self
            .groups
            .staged_groups()
            .filter(|group| group.units.contains(&unit_id))
            .map(|group| group.local_id)
            .collect();
        for owner in owners {
            let rest: Vec<i64> = self
                .groups
                .get(owner)
                .map(|group| {
                    group
                        .units
                        .iter()
                        .copied()
                        .filter(|unit| *unit != unit_id)
                        .collect()
                })
                .unwrap_or_default();
            let _ = self.groups.set_units(owner, rest);
        }
        // Anything it carried returns to the water.
        let riders: Vec<i64> = self
            .force
            .hulls()
            .filter(|hull| hull.host == Some(unit_id))
            .map(|hull| hull.unit_id)
            .collect();
        for rider in riders {
            self.force.set_host(rider, None);
            self.force.set_start(rider, None);
        }
        // A sub-step that named it no longer does.
        self.book.detach_unit(unit_id);
        held
    }

    /// UNGROUP A HULL (`03 GROUPS`): take it out of whatever body musters it
    /// and leave it in the force, loose. Answers the body it left.
    pub fn ungroup(&mut self, unit_id: i64) -> Option<i64> {
        self.groups.remove_unit(unit_id)
    }

    /// How many of the force are on the map, and how many are in the game
    /// without a position. The two legends of F3d and the map mode's header
    /// (`<n> PLACED · <n> UNPLACED`).
    pub fn placement_counts(&self) -> (usize, usize) {
        let placed = self.force.hulls().filter(|h| h.start.is_some()).count();
        (placed, self.force.len() - placed)
    }

    /// `03 GROUPS` — the bodies the tree holds, frame included.
    pub fn group_count(&self) -> usize {
        self.node_ids().len()
    }

    /// `5 UNITS IN THEM` — how many hulls are in some body.
    pub fn units_in_groups(&self) -> usize {
        self.node_ids()
            .into_iter()
            .filter_map(|id| self.groups.get(id))
            .flat_map(|node| node.units.iter().copied())
            .collect::<BTreeSet<i64>>()
            .len()
    }

    /// `05 PLAYERS` — `seated / considered`.
    pub fn considered(&self) -> (usize, usize) {
        (self.roster.staged().count(), self.roster.considered())
    }

    /// `04 SCENARIO` — the steps the draft has STAGED. A `Reopened` draft's
    /// steps are the read's, and the caller adds them (see the `book` field).
    pub fn staged_step_count(&self) -> usize {
        self.book.staged_step_count()
    }

    /// `04 SCENARIO` — every step the book will hold: the server's plus the
    /// ones this draft has staged.
    pub fn book_step_count(&self) -> usize {
        self.held_steps + self.staged_step_count()
    }

    /// Every node id the draft's tree holds: the server's rows that are still
    /// there, and the ones the operator has staged.
    fn node_ids(&self) -> BTreeSet<i64> {
        self.groups
            .frame_groups()
            .chain(self.groups.staged_groups())
            .map(|node| node.local_id)
            .filter(|id| !self.groups.is_removed(*id))
            .collect()
    }

    /// The bodies a group drop can land on, in id order.
    pub fn bodies(&self) -> Vec<i64> {
        self.node_ids().into_iter().collect()
    }

    // -- the verbs -------------------------------------------------------

    /// Whether this draft's mode earns the verb at all.
    pub fn offers(&self, verb: Verb) -> bool {
        self.verbs().contains(&verb)
    }

    /// What stops the verb, in the order the operator will meet it.
    ///
    /// The checks are the server's rules read one stage earlier: a name the
    /// server requires, a commander it insists on, a rank above the ceiling it
    /// accepts, a hull a body holds that is not in the game. A verb that
    /// cannot run is disabled and says WHY, using this list (§9.1).
    pub fn blockers(&self, verb: Verb) -> Vec<Blocker> {
        if !self.offers(verb) {
            return vec![Blocker {
                stage: Stage::Review,
                sentence: format!(
                    "{} is not offered in a {} draft.",
                    verb.label(),
                    self.mode.label().to_lowercase()
                ),
            }];
        }

        let mut blockers = Vec::new();

        // 1. The name, and the length of every field. The server refuses both
        //    with a per-field 400; saying so here is the same answer earlier.
        if !self.terms.name_ok() {
            blockers.push(Blocker {
                stage: Stage::Essentials,
                sentence: "Name is required \u{2014} the server refuses a nameless session."
                    .to_string(),
            });
        }
        if let Some((field, limit)) = self.terms.too_long() {
            blockers.push(Blocker {
                stage: Stage::Essentials,
                sentence: format!("{} is over the server's {limit} characters.", field.label()),
            });
        }

        // 2. The time base: empty is legal while planning, half set is not.
        if let Some(why) = self.terms.time.problem() {
            blockers.push(Blocker {
                stage: Stage::Essentials,
                sentence: why,
            });
        }

        // 3. A STAGE WITH NOTHING IN IT IS NOT A STAGE THAT PASSES. The server
        //    takes an empty game and an empty mission alike, so the console is
        //    the last place that can refuse a save which would open into
        //    nothing. A GROUP IS NOT ONE OF THESE: a flat force is a legal
        //    game (the board says so), and a task organisation is a way of
        //    grouping hulls rather than a stage everyone must fill.
        if verb.ladder() == Ladder::Game && self.unit_count() == 0 {
            blockers.push(Blocker {
                stage: Stage::Units,
                sentence: "The force is empty \u{2014} stage 02 is the hulls the game is fought \
                           with."
                    .to_string(),
            });
        }
        if self.mode.is_scenario() && self.book_step_count() == 0 {
            blockers.push(Blocker {
                stage: Stage::Scenario,
                sentence: "The book is empty \u{2014} stage 04 is the steps the exercise runs."
                    .to_string(),
            });
        }
        if verb == Verb::SaveAsScenario && self.saved_title().trim().is_empty() {
            blockers.push(Blocker {
                stage: Stage::Scenario,
                sentence: "The Saved Scenario has no title \u{2014} a list of saved scenarios is \
                           read by name."
                    .to_string(),
            });
        }

        // 4. The mission ladder writes no game, no seats, no hierarchy, no
        //    placements and no commanders, so none of the rules below are its
        //    business (§9.2). A Saved Scenario's hulls are register ids.
        blockers.extend(self.book_blockers());
        if verb.ladder() == Ladder::Mission {
            return blockers;
        }

        // 5. The force, through the module that owns the rule: one commander
        //    given two hulls, a hull riding itself, a ring of carriers. An
        //    UNCOMMANDED hull is not one of them — a planning save names its
        //    commanders later. The empty `live` is deliberate — a pressed
        //    draft judges every hull as the new piece it is about to write.
        if let Some(gap) = crate::force::gap(&self.force, &[]) {
            let stage = match gap.reason {
                GapReason::TwoCommands | GapReason::JudgeSideCommander => Stage::Players,
                GapReason::SelfHost | GapReason::Ring => Stage::Units,
            };
            blockers.push(Blocker {
                stage,
                sentence: format!("{gap}."),
            });
        }

        // 6. A commander must be a participant of THIS game and not on the
        //    judge side. The judge half is `force`'s rule, asked of the seats
        //    the draft holds; the participant half is the roster's.
        let judges = crate::roster::judge_seats(&self.roster, &[]);
        if let Some(gap) = crate::force::judge_side_gap(&self.force, &judges) {
            blockers.push(Blocker {
                stage: Stage::Players,
                sentence: format!("{gap}."),
            });
        }
        let mut unseated: Vec<String> = Vec::new();
        for hull in self.force.hulls() {
            if let Some(commander) = hull.commander_id {
                if !self.roster.contains(commander) && !unseated.contains(&hull.name) {
                    unseated.push(hull.name.clone());
                }
            }
        }
        if !unseated.is_empty() {
            blockers.push(Blocker {
                stage: Stage::Players,
                sentence: format!(
                    "{} \u{2014} the commander is not seated in this game, and an assignment names \
                     a participant.",
                    unseated.join(", ")
                ),
            });
        }

        // 7. The tree: a body may not hold a hull the game does not, and no
        //    body may sit above the ceiling. The rank BETWEEN bodies is
        //    refused at stage time (`GroupsDraft`), which is where the drag is.
        let mut strays: Vec<String> = Vec::new();
        let mut over_cap: Vec<String> = Vec::new();
        for id in self.node_ids() {
            let Some(node) = self.groups.get(id) else {
                continue;
            };
            for unit in &node.units {
                if !self.force.contains(*unit) && !strays.contains(&node.name) {
                    strays.push(node.name.clone());
                }
            }
            if let Some(cap) = self.ceiling {
                if i64::from(node.kind.rank()) > cap {
                    over_cap.push(format!(
                        "{} is a {} ({})",
                        node.name,
                        kind_name(node.kind),
                        node.kind.rank()
                    ));
                }
            }
        }
        if !strays.is_empty() {
            blockers.push(Blocker {
                stage: Stage::Groups,
                sentence: format!(
                    "{} holds a hull that is not in the game \u{2014} a body holds hulls from the \
                     force.",
                    strays.join(", ")
                ),
            });
        }
        if !over_cap.is_empty() {
            let cap = self.ceiling.unwrap_or_default();
            blockers.push(Blocker {
                stage: Stage::Groups,
                sentence: format!(
                    "{} \u{2014} above this game's ceiling of {cap}.",
                    over_cap.join("; ")
                ),
            });
        }

        // 8. A group's commander is a participant too, on the same terms.
        let mut unseated_bodies: Vec<String> = Vec::new();
        for id in self.node_ids() {
            let Some(node) = self.groups.get(id) else {
                continue;
            };
            if let Some(commander) = node.commander_id {
                if !self.roster.contains(commander) {
                    unseated_bodies.push(node.name.clone());
                }
            }
        }
        if !unseated_bodies.is_empty() {
            blockers.push(Blocker {
                stage: Stage::Players,
                sentence: format!(
                    "{} \u{2014} the task-group commander is not seated in this game.",
                    unseated_bodies.join(", ")
                ),
            });
        }

        blockers
    }

    /// Stage 04's rule, read at stage 06: a step's window is both ends or
    /// neither, the times are military times, and the end is after the start.
    /// The composer refuses these while the author is still in the field; this
    /// is the same rule asked once more of what the draft actually holds.
    fn book_blockers(&self) -> Vec<Blocker> {
        let mut blockers = Vec::new();
        for scenario in self.book.staged_news() {
            for step in &scenario.steps {
                let (start, end) = (
                    step.start.as_deref().unwrap_or_default(),
                    step.end.as_deref().unwrap_or_default(),
                );
                if let Some(why) = crate::window::window_problem("The step", start, end) {
                    blockers.push(Blocker {
                        stage: Stage::Scenario,
                        sentence: format!("{}: {why}.", step.content),
                    });
                }
            }
        }
        blockers
    }

    /// Whether the verb may be pressed. `false` disables it, and
    /// [`Draft::blockers`] is the reason it gives.
    pub fn can_run(&self, verb: Verb) -> bool {
        self.blockers(verb).is_empty()
    }

    /// The settlement counter stage 06 shows: `settled OF the verb's positions`.
    ///
    /// THE DENOMINATOR IS THE VERB'S STOP, not the ladder's length: the panel
    /// draws the ladder of the verb about to be pressed, and Save as Game
    /// Session walks seven of the game ladder's nine positions. A counter that
    /// said `0 OF 9` over a list of seven rows is the sort of number an
    /// operator stops trusting.
    pub fn settled(&self, verb: Verb) -> (u8, u8) {
        (self.journal.settled(verb.ladder()), verb.stops_at())
    }
}

/// A body's level in prose, for a sentence about it.
///
/// `GroupKind::label` is the CODE-facing spelling (`SatuanTugas`), and a
/// sentence that said that would read as a leaked identifier.
fn kind_name(kind: GroupKind) -> &'static str {
    match kind {
        GroupKind::Unsur => "unsur",
        GroupKind::SatuanTugas => "satuan tugas",
        GroupKind::Gugus => "gugus",
        GroupKind::OperasiGabungan => "operasi gabungan",
    }
}

/// The hulls a drop formation would place, in unit-id order — the order a commit
/// writes them in, whatever order the pack laid them out (§6.4).
///
/// Kept here rather than in `formation` because it is a fact about the WRITE, not
/// about the geometry: the pack's own order is the tree's, and this is the
/// order the server's one-placement-at-a-time route can reproduce.
pub fn write_order(positions: &BTreeMap<i64, Start>) -> Vec<(i64, Start)> {
    positions.iter().map(|(id, at)| (*id, *at)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::force::DraftHull;
    use crate::roster::StagedSeat;

    fn hull(unit_id: i64, name: &str, commander: Option<i64>) -> DraftHull {
        let mut h = DraftHull::new(unit_id, name.into(), format!("{unit_id}"), "frigate".into());
        h.commander_id = commander;
        h
    }

    fn seat(user_id: i64, name: &str) -> StagedSeat {
        StagedSeat {
            user_id,
            user_name: name.into(),
            role_id: 1,
            role_name: "Commando".into(),
            call_sign: name.to_uppercase(),
            judge: false,
        }
    }

    /// A complete time base: two starts, a factor and a length.
    fn complete_time() -> TimeBase {
        TimeBase {
            actual_date: "2026-11-01".into(),
            actual_start: "0800".into(),
            assumed_start: "0600".into(),
            factor: "2".into(),
            length: "1.5".into(),
        }
    }

    fn detail() -> GameDetail {
        GameDetail {
            id: 42,
            name: "Baltic Shield 26".into(),
            mode: "maneuver".into(),
            state: "planning".into(),
            description: "the exercise".into(),
            purpose: "prove the flow".into(),
            target: "the board".into(),
            area: None,
            map_tag: None,
            overlay_text: None,
            window: TimeWindow {
                actual_start: Some("2026-11-01T08:00:00+07:00".into()),
                actual_end: Some("2026-11-01T09:30:00+07:00".into()),
                assumed_start: Some("2000-01-01T06:00:00+07:00".into()),
                assumed_end: Some("2000-01-01T09:00:00+07:00".into()),
            },
            pace: Some(GamePace::Standard),
            time_factor: 2.0,
            max_echelon_rank: Some(40),
            room_key: None,
        }
    }

    // -- the origin and the stages ---------------------------------------

    /// A draft needs a mode, and the one mode the console cannot name is not
    /// one: every rule in the flow is a rule about a mode the server has.
    #[test]
    fn a_draft_needs_a_mode_this_build_knows() {
        assert!(Draft::blank(GameMode::Unknown).is_none());
        for mode in [GameMode::Static, GameMode::Maneuver, GameMode::Scenario] {
            let draft = Draft::blank(mode).expect("a draft");
            assert_eq!(draft.mode, mode);
            assert_eq!(draft.pace, GamePace::Standard, "the default is standard");
            assert!(matches!(draft.origin, Origin::Blank { .. }));
        }
    }

    /// Stage numbering is the mode's: a static draft has no stage 04, and the
    /// rail's numbers are positions in the mode's list rather than a stored
    /// number that could drift from it.
    #[test]
    fn the_stage_count_follows_the_mode() {
        let plain = Draft::blank(GameMode::Maneuver).unwrap();
        assert_eq!(plain.stage_count(), 5);
        assert_eq!(Stage::Essentials.number_in(GameMode::Maneuver), Some(1));
        assert_eq!(Stage::Players.number_in(GameMode::Maneuver), Some(4));
        assert_eq!(Stage::Review.number_in(GameMode::Maneuver), Some(5));
        assert_eq!(Stage::Scenario.number_in(GameMode::Maneuver), None);

        let scenario = Draft::blank(GameMode::Scenario).unwrap();
        assert_eq!(scenario.stage_count(), 6);
        assert_eq!(Stage::Scenario.number_in(GameMode::Scenario), Some(4));
        assert_eq!(Stage::Players.number_in(GameMode::Scenario), Some(5));
        assert_eq!(Stage::Review.number_in(GameMode::Scenario), Some(6));
    }

    /// Two verbs for a static or maneuver draft, three for a scenario: the
    /// server refuses a scenario on a non-scenario game, and a Saved Scenario
    /// has no mode to belong to.
    #[test]
    fn only_a_scenario_draft_earns_save_as_scenario() {
        for mode in [GameMode::Static, GameMode::Maneuver] {
            let draft = Draft::blank(mode).unwrap();
            assert_eq!(
                draft.verbs(),
                &[Verb::SaveAsGameSession, Verb::PlayDirectly]
            );
            assert!(!draft.offers(Verb::SaveAsScenario));
        }
        let scenario = Draft::blank(GameMode::Scenario).unwrap();
        assert_eq!(scenario.verbs().len(), 3);
        assert!(scenario.offers(Verb::SaveAsScenario));
        assert_eq!(Verb::SaveAsScenario.ladder(), Ladder::Mission);
        assert_eq!(Verb::PlayDirectly.stops_at(), 9);
    }

    /// The three origins differ in what a press is: two creates and one diff.
    #[test]
    fn the_origin_decides_what_a_press_is() {
        let blank = Draft::blank(GameMode::Scenario).unwrap();
        assert!(blank.origin.is_create());
        assert_eq!(blank.origin.note(), "NO GAME RECORDED");

        let saved = Draft::saved_scenario(7, "Harbour Sweep", "the sweep");
        assert!(saved.origin.is_create());
        assert_eq!(saved.mode, GameMode::Scenario, "a saved scenario is one");
        assert_eq!(saved.terms.name, "Harbour Sweep");
        assert_eq!(saved.origin, Origin::SavedScenario { mission_id: 7 });

        let reopened = Draft::reopened(&detail()).expect("a planning game");
        assert!(!reopened.origin.is_create(), "a press is a diff");
        assert_eq!(reopened.origin.game_id(), Some(42));
        assert_eq!(reopened.mode, GameMode::Maneuver);
    }

    /// THE CEILING IS A GAME FIELD, so a reopened draft READS it rather than
    /// asking again; a blank one picks it in stage 03, where it has meaning.
    #[test]
    fn a_reopened_draft_reads_the_games_ceiling() {
        let mut with_cap = detail();
        with_cap.max_echelon_rank = Some(40);
        assert_eq!(
            Draft::reopened(&with_cap).unwrap().ceiling,
            Some(40),
            "a rank number, as the server states it"
        );
        let mut without = with_cap.clone();
        without.max_echelon_rank = None;
        assert_eq!(
            Draft::reopened(&without).unwrap().ceiling,
            None,
            "no ceiling, not rank zero"
        );
    }

    /// After planning, the terms, the force and the tree are fixed and the
    /// server refuses the writes — so the landing opens the console instead
    /// of opening a draft that cannot commit.
    #[test]
    fn a_game_past_planning_is_not_reopened() {
        let mut later = detail();
        later.state = "preparation".into();
        assert!(Draft::reopened(&later).is_none());
        let mut unknown = detail();
        unknown.mode = "wargame".into();
        assert!(
            Draft::reopened(&unknown).is_none(),
            "a mode this build cannot name is not a draft"
        );
    }

    // -- the time base ---------------------------------------------------

    /// THE ENDS ARE NEVER TYPED: two starts, a factor and a length derive
    /// both windows, and the elapsed figure is assumed time.
    #[test]
    fn the_ends_are_derived_from_the_starts_the_factor_and_the_length() {
        let window = complete_time().window("+07:00").expect("a window");
        assert_eq!(
            window.actual_start.as_deref(),
            Some("2026-11-01T08:00:00+07:00")
        );
        assert_eq!(
            window.actual_end.as_deref(),
            Some("2026-11-01T09:30:00+07:00"),
            "the real window is the length, and only the length"
        );
        assert_eq!(
            window.assumed_start.as_deref(),
            Some("2000-01-01T06:00:00+07:00")
        );
        assert_eq!(
            window.assumed_end.as_deref(),
            Some("2000-01-01T09:00:00+07:00"),
            "the exercise clock runs at length \u{d7} factor"
        );
        assert_eq!(complete_time().elapsed_hours(), Some(3.0));
    }

    /// A factor change moves the ASSUMED end and never the real window: the
    /// rule §4.1 states and the reason a factor edit must carry its
    /// `assumed_end`.
    #[test]
    fn the_factor_moves_the_exercise_clock_and_not_the_real_one() {
        let mut base = complete_time();
        let before = base.window("+00:00").unwrap();
        base.factor = "1".into();
        let after = base.window("+00:00").unwrap();
        assert_eq!(after.actual_start, before.actual_start);
        assert_eq!(
            after.actual_end, before.actual_end,
            "the real window stands"
        );
        assert_ne!(after.assumed_end, before.assumed_end);
    }

    /// An empty base is a legal planning state: the server allows it and the
    /// execution gate refuses without it. It is not half set, and it says
    /// nothing.
    #[test]
    fn an_empty_time_base_is_legal_and_silent() {
        let base = TimeBase::default();
        assert_eq!(base.state(), BaseState::Empty);
        assert_eq!(base.problem(), None);
        assert!(base.window("+00:00").is_none(), "nothing to send");
    }

    /// A half-set base is a draft state the form may hold and REVIEW names;
    /// it is never sent half-set.
    #[test]
    fn a_half_set_time_base_is_named_and_never_sent() {
        let mut base = complete_time();
        base.length.clear();
        assert_eq!(base.state(), BaseState::Partial);
        let why = base.problem().expect("a reason");
        assert!(why.starts_with("LENGTH must be"), "{why}");
        assert!(base.window("+00:00").is_none(), "never half a window");

        let mut no_day = complete_time();
        no_day.actual_date.clear();
        assert!(
            no_day
                .problem()
                .unwrap()
                .starts_with("The real clock needs"),
            "{:?}",
            no_day.problem()
        );

        let mut no_clock = complete_time();
        no_clock.assumed_start.clear();
        assert!(no_clock.problem().unwrap().contains("exercise clock"));

        // With no real start there is nothing to derive either end from, and
        // the sentence names the clock rather than the whole base.
        let mut no_real = complete_time();
        no_real.actual_date.clear();
        no_real.actual_start.clear();
        assert!(
            no_real
                .problem()
                .unwrap()
                .starts_with("The real clock has no start"),
            "{:?}",
            no_real.problem()
        );
    }

    /// A factor of zero is not a factor: the server divides by it.
    #[test]
    fn a_factor_above_zero_is_the_rule() {
        let mut base = complete_time();
        base.factor = "0".into();
        assert_eq!(base.state(), BaseState::Partial);
        assert!(base.problem().unwrap().contains("TIME FACTOR"));
        base.factor = "-1".into();
        assert!(base.window("+00:00").is_none());
        base.factor = "fast".into();
        assert!(base.window("+00:00").is_none());
    }

    /// A reopened draft's time base reads back into the window the server
    /// holds: the length is derived from the two real ends, which is the one
    /// direction in which an end is read.
    #[test]
    fn a_reopened_draft_derives_its_length_from_the_servers_ends() {
        let reopened = Draft::reopened(&detail()).unwrap();
        assert_eq!(reopened.terms.time.length, "1.5");
        assert_eq!(reopened.terms.time.actual_date, "2026-11-01");
        assert_eq!(reopened.terms.time.actual_start, "0800");
        assert_eq!(reopened.terms.time.assumed_start, "0600");
        assert_eq!(reopened.terms.time.factor, "2");
        let window = reopened.terms.time.window("+07:00").expect("a window");
        assert_eq!(window.actual_start, detail().window.actual_start);
        assert_eq!(window.actual_end, detail().window.actual_end);
        // The assumed clock's date is a fixed base on the wire; its TIME is
        // what carries the plan.
        assert_eq!(
            window.assumed_end.as_deref(),
            Some("2000-01-01T09:00:00+07:00")
        );
    }

    /// The ten fields are in the sheet's order, with the sheet's four hints.
    #[test]
    fn the_ten_fields_are_the_sheets_ten() {
        assert_eq!(TermField::ALL.len(), 10);
        assert_eq!(TermField::Name.label(), "NAME");
        assert_eq!(TermField::MapTag.label(), "MAP TAG");
        assert_eq!(TermField::Length.label(), "LENGTH");
        assert_eq!(TermField::Name.hint(), "REQUIRED \u{b7} 255 MAX");
        assert_eq!(TermField::Purpose.hint(), "OPTIONAL \u{b7} 4000");
        assert_eq!(TermField::Area.hint(), "OPT \u{b7} HIDDEN");
        assert_eq!(TermField::Length.hint(), "OPT \u{b7} EXEC");
        assert_eq!(TermField::Area.max_len(), Some(1000));
        assert_eq!(TermField::AssumedStart.max_len(), None);
    }

    /// ACTUAL START is one row and two facts. A whole day-and-time lands in
    /// both halves; a bare time sets the time and leaves the day alone.
    #[test]
    fn actual_start_is_a_day_and_a_time() {
        let mut terms = Terms::blank();
        terms.set_text(TermField::ActualStart, "2026-11-01 0930".into());
        assert_eq!(terms.time.actual_date, "2026-11-01");
        assert_eq!(terms.time.actual_start, "0930");
        assert_eq!(terms.text(TermField::ActualStart), "2026-11-01 0930");
        terms.set_text(TermField::ActualStart, "1000".into());
        assert_eq!(terms.time.actual_date, "2026-11-01", "the day stands");
        assert_eq!(terms.time.actual_start, "1000");
    }

    // -- completeness ----------------------------------------------------

    /// A nameless draft cannot be written by either verb, on either ladder.
    #[test]
    fn a_nameless_draft_blocks_every_verb() {
        let mut draft = Draft::blank(GameMode::Scenario).unwrap();
        for verb in draft.verbs().to_vec() {
            let blocking = draft.blockers(verb);
            assert_eq!(blocking[0].stage, Stage::Essentials);
            assert!(blocking[0].sentence.starts_with("Name is required"));
            assert!(!draft.can_run(verb));
        }
        draft.terms.name = "Baltic Shield 26".into();
        // A NAME IS NOT A SCENARIO. The mission has a title and no steps, and
        // the verb refuses to save a book that opens into nothing.
        let blocking = draft.blockers(Verb::SaveAsScenario);
        assert_eq!(blocking.len(), 1, "{blocking:?}");
        assert_eq!(blocking[0].stage, Stage::Scenario);
        assert!(blocking[0].sentence.contains("book is empty"), "{blocking:?}");

        let scenario = draft.book.stage_scenario("The sweep");
        draft
            .book
            .stage_step_new(scenario, "TAKE STATION XYZ", None)
            .expect("a step");
        assert!(draft.can_run(Verb::SaveAsScenario), "a name and a step");
    }

    /// THE EMPTY STAGES ARE REFUSED. The server takes a game with no hulls
    /// and a mission with no steps; the console is the last place that can
    /// say no, and it names the stage.
    #[test]
    fn a_stage_with_nothing_in_it_blocks_the_save() {
        let mut draft = Draft::blank(GameMode::Scenario).unwrap();
        draft.terms.name = "North Sea Patrol".into();
        let blocking = draft.blockers(Verb::SaveAsGameSession);
        assert_eq!(blocking[0].stage, Stage::Units, "{blocking:?}");
        assert!(blocking[0].sentence.contains("force is empty"));
        assert!(blocking.iter().any(|b| b.stage == Stage::Scenario));

        // A hull makes stage 02 a stage; the book still has no steps.
        draft.roster.stage(seat(4, "Lt. Fitri"));
        draft.force.upsert(hull(351, "KRI AHMAD YANI", Some(4)));
        let blocking = draft.blockers(Verb::SaveAsGameSession);
        assert_eq!(blocking.len(), 1, "{blocking:?}");
        assert_eq!(blocking[0].stage, Stage::Scenario);

        // And one step settles it.
        let book = draft.book.stage_scenario("The sweep");
        draft
            .book
            .stage_step_new(book, "TAKE STATION XYZ", None)
            .expect("a step");
        assert!(draft.can_run(Verb::SaveAsGameSession));
        assert!(draft.can_run(Verb::SaveAsScenario));
    }

    /// A REOPENED DRAFT'S STEPS ARE THE READ'S. The book draft holds adds and
    /// cuts, so the count that the rail and the blocker use has to add the
    /// steps the server already holds — otherwise a complete game would be
    /// refused for an empty book it does not have.
    #[test]
    fn a_reopened_drafts_steps_are_counted_from_the_read() {
        let mut draft = Draft::reopened(&detail()).expect("a planning game");
        draft.held_steps = 5;
        assert_eq!(draft.staged_step_count(), 0);
        assert_eq!(draft.book_step_count(), 5);
        let blocking = draft.blockers(Verb::SaveAsGameSession);
        assert!(
            !blocking.iter().any(|b| b.stage == Stage::Scenario),
            "{blocking:?}"
        );
    }

    /// THE SAVED SCENARIO IS NAMED FROM THE GAME until the operator says
    /// otherwise: the field opens on the title stage 01 holds, and the first
    /// edit is the mission's own.
    #[test]
    fn the_saved_scenario_follows_the_games_name_until_edited() {
        let mut draft = Draft::blank(GameMode::Scenario).unwrap();
        draft.terms.name = "Baltic Shield 26".into();
        draft.terms.description = "the sweep".into();
        assert_eq!(draft.saved_title(), "Baltic Shield 26");
        assert_eq!(draft.saved_description(), "the sweep");

        // A later rename in stage 01 still reaches the field.
        draft.terms.name = "Baltic Shield 27".into();
        assert_eq!(draft.saved_title(), "Baltic Shield 27");

        draft.set_saved_title("Sweep of the Sound".into());
        draft.set_saved_description(String::new());
        draft.terms.name = "Baltic Shield 28".into();
        assert_eq!(draft.saved_title(), "Sweep of the Sound", "its own name");
        assert_eq!(draft.saved_description(), "", "cleared is cleared");

        // A mission with no title is refused, and the blocker is the field's.
        let book = draft.book.stage_scenario("Sweep of the Sound");
        draft
            .book
            .stage_step_new(book, "TAKE STATION XYZ", None)
            .expect("a step");
        draft.set_saved_title("   ".into());
        let blocking = draft.blockers(Verb::SaveAsScenario);
        assert_eq!(blocking.len(), 1, "{blocking:?}");
        assert!(blocking[0].sentence.contains("no title"), "{blocking:?}");
    }

    /// A PIECE NEEDS A COMMANDER. A hull with none is a legal draft state and
    /// an illegal write, so the verb names the hulls — and never invents a
    /// placeholder commander.
    #[test]
    fn an_uncommanded_hull_does_not_block_a_planning_save() {
        // PLANNING TAKES MONTHS, AND COMMANDERS COME LATER. The server takes
        // a hull on its own, so the save must not demand a seat for every
        // hull first.
        let mut draft = Draft::blank(GameMode::Maneuver).unwrap();
        draft.terms.name = "Baltic Shield 26".into();
        draft.force.upsert(hull(351, "KRI AHMAD YANI", None));
        assert!(
            draft.can_run(Verb::SaveAsGameSession),
            "{:?}",
            draft.blockers(Verb::SaveAsGameSession)
        );
        assert!(
            draft.can_run(Verb::PlayDirectly),
            "and the transition out of planning has no commander precondition"
        );
    }

    /// A commander who is not a participant is a 400 on the assignment write:
    /// the person has to be seated first.
    #[test]
    fn a_commander_who_is_not_seated_blocks_and_names_the_hull() {
        let mut draft = Draft::blank(GameMode::Static).unwrap();
        draft.terms.name = "Harbour Defence".into();
        draft.force.upsert(hull(351, "KRI AHMAD YANI", Some(9)));
        let blocking = draft.blockers(Verb::SaveAsGameSession);
        assert_eq!(blocking.len(), 1, "{blocking:?}");
        assert_eq!(blocking[0].stage, Stage::Players);
        assert!(blocking[0].sentence.contains("KRI AHMAD YANI"));
        assert!(blocking[0].sentence.contains("not seated"));
    }

    /// One scope, one commander: the server refuses a second command for the
    /// same person, and the force module already carries that wording.
    #[test]
    fn one_person_given_two_hulls_blocks_the_verb() {
        let mut draft = Draft::blank(GameMode::Maneuver).unwrap();
        draft.terms.name = "Baltic Shield 26".into();
        draft.roster.stage(seat(4, "Lt. Fitri"));
        draft.force.upsert(hull(351, "KRI AHMAD YANI", Some(4)));
        draft.force.upsert(hull(352, "KRI SLAMET RIYADI", Some(4)));
        let blocking = draft.blockers(Verb::SaveAsGameSession);
        assert!(
            blocking
                .iter()
                .any(|b| b.sentence.contains("one person commands one thing")),
            "{blocking:?}"
        );
    }

    /// A Saved Scenario writes no game, no seats, no hierarchy, no placements
    /// and no commanders — so the force's gaps are not its business.
    #[test]
    fn the_mission_ladder_ignores_the_force_and_the_rosters_rules() {
        let mut draft = Draft::blank(GameMode::Scenario).unwrap();
        draft.terms.name = "Harbour Sweep".into();
        let scenario = draft.book.stage_scenario("The sweep");
        draft
            .book
            .stage_step_new(scenario, "TAKE STATION XYZ", None)
            .expect("a step");
        draft.force.upsert(hull(351, "KRI AHMAD YANI", None));
        assert!(draft.can_run(Verb::SaveAsScenario));
        // The game ladder still has a rule of its own: a judge-side commander
        // may not steer a hull. The mission ladder is not its business.
        let mut judge = seat(9, "Kpt. Sari");
        judge.judge = true;
        draft.roster.stage(judge);
        draft.force.set_commander(351, Some(9));
        assert!(draft.can_run(Verb::SaveAsScenario));
        assert!(!draft.can_run(Verb::SaveAsGameSession));
    }

    /// A body may not hold a hull the game does not have.
    #[test]
    fn a_body_holding_a_hull_outside_the_force_blocks_the_verb() {
        let mut draft = Draft::blank(GameMode::Maneuver).unwrap();
        draft.terms.name = "Baltic Shield 26".into();
        draft
            .groups
            .stage_group(0, "TG-1", GroupKind::SatuanTugas, None, None, vec![351])
            .expect("a group");
        let blocking = draft.blockers(Verb::SaveAsGameSession);
        assert!(
            blocking
                .iter()
                .any(|b| b.stage == Stage::Groups && b.sentence.contains("not in the game")),
            "{blocking:?}"
        );

        draft.force.upsert(hull(351, "KRI AHMAD YANI", None));
        draft.roster.stage(seat(4, "Lt. Fitri"));
        draft.force.set_commander(351, Some(4));
        assert!(draft.can_run(Verb::SaveAsGameSession));
    }

    /// `max_echelon_rank` is a rank NUMBER, so the ceiling check compares
    /// numbers: 40 admits an operasi gabungan and not above it.
    #[test]
    fn the_ceiling_is_a_rank_number() {
        let mut draft = Draft::blank(GameMode::Maneuver).unwrap();
        draft.terms.name = "Baltic Shield 26".into();
        draft.roster.stage(seat(4, "Lt. Fitri"));
        draft.force.upsert(hull(351, "KRI AHMAD YANI", Some(4)));
        draft.ceiling = Some(30);
        draft
            .groups
            .stage_group(
                0,
                "OPGAB JAYA",
                GroupKind::OperasiGabungan,
                None,
                None,
                vec![],
            )
            .expect("a group");
        let blocking = draft.blockers(Verb::SaveAsGameSession);
        assert!(
            blocking
                .iter()
                .any(|b| b.sentence.contains("ceiling of 30")),
            "{blocking:?}"
        );
        draft.ceiling = Some(40);
        assert!(draft.can_run(Verb::SaveAsGameSession));
    }

    /// Stage 04's rule reaches stage 06: a step's window is a military pair
    /// with the end after the start, and the server refuses anything else.
    #[test]
    fn a_badly_windowed_step_blocks_the_scenario_verb() {
        let mut draft = Draft::blank(GameMode::Scenario).unwrap();
        draft.terms.name = "North Sea Patrol".into();
        let scenario = draft.book.stage_scenario("The sweep");
        draft
            .book
            .stage_step_new(
                scenario,
                "TAKE STATION XYZ",
                Some(("1030".into(), "0810".into())),
            )
            .expect("a step");
        let blocking = draft.blockers(Verb::SaveAsScenario);
        assert_eq!(blocking.len(), 1, "{blocking:?}");
        assert_eq!(blocking[0].stage, Stage::Scenario);
        assert!(
            blocking[0].sentence.contains("end must be after its start"),
            "{blocking:?}"
        );
    }

    /// A draft nobody has touched is not worth a confirmation; a draft with a
    /// name in it is (§3.6).
    #[test]
    fn an_untouched_draft_is_empty_and_a_named_one_is_not() {
        let empty = Draft::blank(GameMode::Static).unwrap();
        assert!(empty.is_empty());
        let mut touched = Draft::blank(GameMode::Static).unwrap();
        touched.terms.name = "Harbour Defence".into();
        assert!(!touched.is_empty());
    }

    // -- the counts ------------------------------------------------------

    /// Every number the rail shows is read from the collection it counts.
    #[test]
    fn the_counts_are_read_from_the_collections() {
        let mut draft = Draft::blank(GameMode::Maneuver).unwrap();
        assert_eq!(draft.unit_count(), 0);
        assert_eq!(draft.group_count(), 0);
        assert_eq!(draft.considered(), (0, 0));

        draft.force.upsert(hull(351, "KRI AHMAD YANI", Some(4)));
        let placed = hull(352, "KRI SLAMET RIYADI", Some(5));
        draft.force.upsert(placed);
        draft.force.set_start(
            351,
            Some(Start {
                lat: -3.2,
                lon: 108.9,
            }),
        );
        assert_eq!(draft.unit_count(), 2);
        assert_eq!(draft.placement_counts(), (1, 1), "one pin, one hollow ring");

        let gugus = draft
            .groups
            .stage_group(0, "GUS-1", GroupKind::Gugus, None, None, vec![])
            .expect("a group");
        let satgas = draft
            .groups
            .stage_group(
                0,
                "TG-1",
                GroupKind::SatuanTugas,
                Some(gugus),
                None,
                vec![351, 352],
            )
            .expect("a group");
        assert_eq!(draft.group_count(), 2);
        assert_eq!(draft.units_in_groups(), 2);
        assert_eq!(draft.bodies(), vec![satgas, gugus], "bodies, in id order");

        draft.roster.stage(seat(4, "Lt. Fitri"));
        draft.roster.remove(9, false);
        assert_eq!(draft.considered(), (1, 2), "one seated, one not playing");
    }

    /// The tree's count is a count of bodies, not of staged calls: removing a
    /// body takes it and its subtree out of the number.
    #[test]
    fn removing_a_body_takes_it_out_of_the_count() {
        let mut draft = Draft::blank(GameMode::Maneuver).unwrap();
        let gus = draft
            .groups
            .stage_group(0, "GUS-1", GroupKind::Gugus, None, None, vec![])
            .unwrap();
        let tg = draft
            .groups
            .stage_group(0, "TG-1", GroupKind::SatuanTugas, Some(gus), None, vec![])
            .unwrap();
        assert_eq!(draft.group_count(), 2);
        draft.groups.remove_group(tg);
        assert_eq!(draft.group_count(), 1);
    }

    // -- the journal -----------------------------------------------------

    /// A settled position is never sent again, and resume starts at the first
    /// one that is not settled.
    #[test]
    fn the_journal_says_where_a_resumed_press_starts() {
        let mut journal = Journal::new();
        assert_eq!(journal.next_owed(Ladder::Game), 1);
        journal.note_written(Ladder::Game, 1, Some(42));
        journal.note_written(Ladder::Game, 2, None);
        journal.note_failed(Ladder::Game, 3, 502, None);
        assert_eq!(journal.settled(Ladder::Game), 2);
        assert_eq!(journal.next_owed(Ladder::Game), 3, "the failed one is owed");
        journal.note_written(Ladder::Game, 3, None);
        assert_eq!(journal.next_owed(Ladder::Game), 4);
    }

    /// THE COUNTER'S DENOMINATOR IS THE VERB'S STOP: a Save as Game Session
    /// press walks seven of the nine game positions, and the panel it draws is
    /// that list.
    #[test]
    fn the_settled_counter_counts_the_verbs_own_positions() {
        let draft = Draft::blank(GameMode::Scenario).unwrap();
        assert_eq!(draft.settled(Verb::SaveAsGameSession), (0, 7));
        assert_eq!(draft.settled(Verb::PlayDirectly), (0, 9));
        assert_eq!(draft.settled(Verb::SaveAsScenario), (0, 3));
    }

    /// 08 is a READ, and it settles the position without pretending anything
    /// was written.
    #[test]
    fn the_gate_read_settles_a_position_without_writing_one() {
        let mut journal = Journal::new();
        for position in 1..=7 {
            journal.note_written(Ladder::Game, position, None);
        }
        journal.note_read(Ladder::Game, 8);
        assert_eq!(journal.settled(Ladder::Game), 8);
        assert_eq!(
            journal.landing(Ladder::Game, 8),
            Some(&Landing::Read),
            "not Written"
        );
    }

    /// The two ladders keep their own books: a mission press does not settle
    /// a game position.
    #[test]
    fn the_two_ladders_do_not_share_positions() {
        let mut journal = Journal::new();
        journal.note_written(Ladder::Mission, 1, Some(9));
        assert_eq!(journal.settled(Ladder::Mission), 1);
        assert_eq!(journal.settled(Ladder::Game), 0);
        assert_eq!(journal.next_owed(Ladder::Game), 1);
    }

    /// A lost 01 is adopted, not retried: the game may exist anyway, and
    /// guessing would make a duplicate the operator cannot see.
    #[test]
    fn an_adopted_game_is_journaled_as_written() {
        let mut journal = Journal::new();
        journal.note_failed(Ladder::Game, 1, 502, Some("req-1".into()));
        assert_eq!(journal.next_owed(Ladder::Game), 1);
        journal.adopt_game(77);
        assert_eq!(journal.game_id(), Some(77));
        assert_eq!(journal.next_owed(Ladder::Game), 2, "continue at 02");
    }
}
