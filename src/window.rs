//! The exercise's planned window, as the operator has it.
//!
//! WHY THE WINDOW IS A DRAFT AND NOT A WRITE. Minos accepts a change to the
//! exercise's terms only while the game is in `planning` — a game in
//! preparation answers "A game's terms may only be changed while it is in
//! `planning`" — and the window is one of those terms. A window written
//! after the advance has moved the exercise on is therefore refused, so the
//! window is staged with the rest of the plan and written BY the advance,
//! before the transition.
//!
//! THE RULE IS ONE RULE, AND THE SCENARIO COMPOSER ALREADY HAD IT. A step's
//! window and the exercise's window are the same shape: both ends or
//! neither, and the end after the start. The composer's copy of that rule
//! lived in `ComposerDraft::problem`; it lives HERE now, and both call it,
//! so the console cannot grow two answers to "is this a window".
//!
//! The composer's window is two military times and the exercise's is a date
//! and two military times, so the exercise's real clock carries one extra
//! fact — WHICH DAY — and nothing else. That is why the rule is written
//! once and the date is checked beside it rather than inside it.

use std::collections::BTreeMap;

/// What is wrong with a window whose two ends are both `HHMM`, or neither.
///
/// `label` names the window in the sentence, so the composer and the
/// exercise each speak about themselves: "a window" and "the real clock".
/// The rule is the same for both, and the sentences differ only in the
/// name — a refusal that says "a window's end must be after its start" to
/// somebody who typed the EXERCISE's ends sends them to the wrong field.
///
/// The cases, in the order they are met:
/// - neither end: no window, and no complaint. An unwritten window is a
///   real state — the backend supplies no default for either end.
/// - one end: the half-window the backend refuses. Named as a half, because
///   "1000 is not a military time" would send the author to fix the wrong
///   field.
/// - a value that is not a military time: the offending value is quoted,
///   because "not a military time" without it leaves the author hunting.
/// - the end before the start: the window runs backwards.
pub fn window_problem(label: &str, start: &str, end: &str) -> Option<String> {
    match (start.is_empty(), end.is_empty()) {
        (false, true) => Some(format!("{label} needs an end as well as a start")),
        (true, false) => Some(format!("{label} needs a start as well as an end")),
        (false, false) => {
            if !crate::view::hhmm_ok(start) {
                return Some(format!("{start} is not a military time like 1000"));
            }
            if !crate::view::hhmm_ok(end) {
                return Some(format!("{end} is not a military time like 1030"));
            }
            if !crate::view::hhmm_window_ok(start, end) {
                return Some(format!("{label}'s end must be after its start"));
            }
            None
        }
        (true, true) => None,
    }
}

/// What is wrong with the REAL clock's window: a date and two `HHMM` ends.
///
/// The date is the operator's to look up — the real clock is the wall clock
/// of the exercise, and a guessed date would put the exercise on the wrong
/// day while every other check passed. So the date is typed, and a date
/// that is not a real calendar day is refused here rather than arriving as a
/// 400 from the field.
///
/// The date sits BESIDE the window rule rather than inside it: both ends
/// carry the same date, so "the end is after the start" is still a question
/// about the two military times.
pub fn actual_problem(date: &str, start: &str, end: &str) -> Option<String> {
    if start.is_empty() && end.is_empty() {
        return None;
    }
    if !crate::backend::date_ok(date) {
        return Some(format!("{date} is not a day like 2026-11-01"));
    }
    window_problem("the real clock", start, end)
}

/// The exercise clock's window: two `HHMM` ends and no date.
///
/// The exercise clock has no date to speak of — Minos compares it only with
/// itself — so there is nothing here to check but the window rule.
pub fn assumed_problem(start: &str, end: &str) -> Option<String> {
    window_problem("the exercise clock", start, end)
}

/// One clock as the server states it: a date and a military time.
///
/// The server speaks RFC3339 for every clock, and the console speaks
/// `HHMM` because that is what an operator types. This is the read that
/// turns one into the other, and it is lossy in one direction only: a
/// server value with no `T` in it is not a clock this console can show,
/// and says so rather than guessing a date.
pub fn read_clock(value: &str) -> Option<(String, String)> {
    let (date, rest) = value.split_once('T')?;
    let time: String = rest.chars().filter(char::is_ascii_digit).take(4).collect();
    if time.len() != 4 {
        return None;
    }
    Some((date.to_string(), time))
}

/// The operator's planned window, staged the way the force and the roster
/// are staged: it is what the advance writes, and it is refused whole if it
/// is not a window.
///
/// The empty draft is not a window and not a complaint: an exercise whose
/// window was never written is an ordinary state, and the advance leaves
/// the server's own window alone rather than blanking it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WindowDraft {
    /// The real clock's day, `YYYY-MM-DD`.
    pub actual_date: String,
    /// The real clock's start, `HHMM`.
    pub actual_start: String,
    /// The real clock's end, `HHMM`.
    pub actual_end: String,
    /// The exercise clock's start, `HHMM`.
    pub assumed_start: String,
    /// The exercise clock's end, `HHMM`.
    pub assumed_end: String,
    /// THE FRAME the window is written in: the UTC offset the server
    /// stated, empty when it has stated none.
    ///
    /// This is not a display detail — it is what makes the plan a plan.
    /// The server renders every instant in ITS zone, so the digits the
    /// console reads back are the server's zone's digits; writing them in
    /// any other frame is writing a DIFFERENT instant, and a window that
    /// round-trips through the server moves by the difference every time
    /// it is saved. Keep the frame the server stated, and the plan is the
    /// plan the operator typed.
    pub offset: String,
}

impl WindowDraft {
    /// A window nobody has written.
    pub fn blank() -> Self {
        Self::default()
    }

    /// The draft as the server's read leaves it: the window the exercise
    /// already holds, in the fields the operator edits.
    ///
    /// A clock the server states in a shape this console cannot read is
    /// left BLANK rather than half-read: a date the console invented would
    /// be a wrong day, and a start with no end is a window the advance
    /// would refuse.
    pub fn from_read(
        actual_start: &str,
        actual_end: &str,
        assumed_start: &str,
        assumed_end: &str,
    ) -> Self {
        let mut draft = Self::blank();
        if let (Some((date, start)), Some((_, end))) = (
            read_clock(actual_start),
            read_clock(actual_end),
        ) {
            draft.actual_date = date;
            draft.actual_start = start;
            draft.actual_end = end;
        }
        if let (Some((_, start)), Some((_, end))) = (
            read_clock(assumed_start),
            read_clock(assumed_end),
        ) {
            draft.assumed_start = start;
            draft.assumed_end = end;
        }
        // THE FRAME, learned from the same strings the digits came from.
        // The server renders every instant in its own zone, and that zone
        // is the frame the plan is written in — so the offset is read here
        // and carried, rather than guessed at the moment of the write.
        draft.offset = [actual_start, actual_end, assumed_start, assumed_end]
            .iter()
            .find_map(|v| crate::backend::clock_offset(v))
            .unwrap_or_default();
        draft
    }

    /// What is wrong with the window, if anything. Both clocks are checked:
    /// a real window with a broken exercise window is still a broken plan.
    pub fn problem(&self) -> Option<String> {
        actual_problem(&self.actual_date, &self.actual_start, &self.actual_end)
            .or_else(|| assumed_problem(&self.assumed_start, &self.assumed_end))
    }

    /// Whether the draft says nothing at all — which is not the same as
    /// saying a half window.
    pub fn is_blank(&self) -> bool {
        self.actual_start.is_empty()
            && self.actual_end.is_empty()
            && self.assumed_start.is_empty()
            && self.assumed_end.is_empty()
    }

    /// The five fields, in the order the editor draws them: the real
    /// clock's day, its two ends, and the exercise clock's two ends.
    ///
    /// The order is HERE and not in the editor, because the editor's
    /// keystrokes and the draft's own fields have to agree about which
    /// field is which — and two lists of the same order are two lists
    /// waiting to disagree.
    pub fn fields(&self) -> [&str; 5] {
        [
            &self.actual_date,
            &self.actual_start,
            &self.actual_end,
            &self.assumed_start,
            &self.assumed_end,
        ]
    }

    /// One field, by its place in [`WindowDraft::fields`].
    pub fn field(&self, idx: usize) -> &str {
        self.fields()[idx]
    }

    /// Put one field, by its place in [`WindowDraft::fields`].
    pub fn set_field(&mut self, idx: usize, value: String) {
        match idx {
            0 => self.actual_date = value,
            1 => self.actual_start = value,
            2 => self.actual_end = value,
            3 => self.assumed_start = value,
            4 => self.assumed_end = value,
            _ => {}
        }
    }

    /// The draft's two clocks, as the pairs the advance sends: the real
    /// clock with its date, the exercise clock without one.
    ///
    /// A clock with nothing in it is `None` rather than a half pair: the
    /// caller refuses the draft before it gets here, and a half window sent
    /// to the server would be a window the server merges one key at a time.
    pub fn pairs(&self) -> (Option<(&str, &str, &str)>, Option<(&str, &str)>) {
        let actual = if self.actual_start.is_empty() && self.actual_end.is_empty() {
            None
        } else {
            Some((
                self.actual_date.as_str(),
                self.actual_start.as_str(),
                self.actual_end.as_str(),
            ))
        };
        let assumed = if self.assumed_start.is_empty() && self.assumed_end.is_empty() {
            None
        } else {
            Some((self.assumed_start.as_str(), self.assumed_end.as_str()))
        };
        (actual, assumed)
    }
}

/// Who commands what, as the force draft has it: a person and the hulls
/// they steer.
///
/// Plain data on purpose. The roster's rules and the force's rules are
/// separate modules, and the one fact they share — a person, and the hulls
/// under them — crosses as a map rather than as a dependency.
pub type Commanders = BTreeMap<i64, Vec<String>>;

#[cfg(test)]
mod tests {
    use super::*;

    /// No window is the default and is always fine. Both ends empty is what
    /// an unauthored window looks like, and it is what the backend takes.
    #[test]
    fn a_draft_with_no_window_is_valid() {
        assert_eq!(window_problem("a window", "", ""), None);
    }

    /// Half a window is the case that bites: the backend refuses it, so the
    /// author is told now rather than after finishing the form.
    #[test]
    fn half_a_window_is_refused_before_the_request() {
        assert_eq!(
            window_problem("a window", "1000", "").as_deref(),
            Some("a window needs an end as well as a start")
        );
        assert_eq!(
            window_problem("a window", "", "1030").as_deref(),
            Some("a window needs a start as well as an end")
        );
    }

    /// Malformed and backwards windows are named as such, and the message
    /// quotes what was typed, because "not a military time" without the
    /// offending value leaves the author hunting.
    #[test]
    fn a_bad_window_says_which_value_is_bad() {
        assert!(window_problem("a window", "900", "1030")
            .unwrap()
            .starts_with("900 is not a military time"));
        assert!(window_problem("a window", "1000", "25:00")
            .unwrap()
            .starts_with("25:00 is not a military time"));
        assert_eq!(
            window_problem("a window", "1030", "1000").as_deref(),
            Some("a window's end must be after its start")
        );
    }

    /// A good window comes through as the pair the request expects, not as
    /// the two fields re-derived at the call site.
    #[test]
    fn a_good_window_becomes_the_pair_the_request_takes() {
        assert_eq!(window_problem("a window", "1000", "1030"), None);
    }

    /// The rule is ONE rule: the exercise's clocks are held to the same
    /// standard as the composer's, and the sentence names the clock so the
    /// operator knows which field to fix.
    #[test]
    fn the_exercises_clocks_are_held_to_the_same_rule() {
        assert_eq!(
            window_problem("the real clock", "0800", "").as_deref(),
            Some("the real clock needs an end as well as a start")
        );
        assert_eq!(
            window_problem("the exercise clock", "1600", "0800").as_deref(),
            Some("the exercise clock's end must be after its start")
        );
    }

    /// The real clock carries a day, and the day is the operator's to look
    /// up: a well-formed time on a day that does not exist is refused here
    /// rather than arriving as a 400 from the field.
    #[test]
    fn the_real_clock_refuses_a_day_that_is_not_a_day() {
        assert_eq!(
            actual_problem("2026-02-30", "0800", "1600").as_deref(),
            Some("2026-02-30 is not a day like 2026-11-01")
        );
        assert_eq!(actual_problem("2026-11-01", "0800", "1600"), None);
    }

    /// A real window with a broken exercise window is still a broken plan,
    /// and the sentence names the exercise clock — the one that is broken.
    #[test]
    fn a_draft_reports_the_clock_that_is_broken() {
        let draft = WindowDraft {
            actual_date: "2026-11-01".into(),
            actual_start: "0800".into(),
            actual_end: "1600".into(),
            assumed_start: "1600".into(),
            assumed_end: "0800".into(),
            offset: "+00:00".into(),
        };
        assert_eq!(
            draft.problem().as_deref(),
            Some("the exercise clock's end must be after its start")
        );
    }

    /// The server's own window, read back into the fields the operator
    /// edits. The real clock's day comes with it, because the day is half
    /// of what the operator has to be able to change.
    #[test]
    fn the_servers_window_seeds_the_draft() {
        let draft = WindowDraft::from_read(
            "2026-11-01T08:00:00Z",
            "2026-11-01T16:00:00Z",
            "2000-01-01T08:00:00Z",
            "2000-01-01T16:00:00Z",
        );
        assert_eq!(draft.actual_date, "2026-11-01");
        assert_eq!(draft.actual_start, "0800");
        assert_eq!(draft.actual_end, "1600");
        assert_eq!(draft.assumed_start, "0800");
        assert_eq!(draft.assumed_end, "1600");
        assert_eq!(draft.problem(), None);
    }

    /// A half window the server holds is NOT half-read into the draft: the
    /// draft would refuse it, and the operator would be shown a window
    /// nobody can write. The server's own shape is left alone instead.
    #[test]
    fn a_half_window_on_the_server_leaves_the_draft_blank() {
        let draft = WindowDraft::from_read("2026-11-01T08:00:00Z", "", "", "");
        assert!(draft.is_blank());
        assert_eq!(draft.problem(), None);
    }

    /// A blank draft is not a complaint. An exercise whose window was never
    /// written is an ordinary state, and the advance leaves the server's own
    /// window alone rather than blanking it.
    #[test]
    fn a_blank_draft_says_nothing() {
        let draft = WindowDraft::blank();
        assert!(draft.is_blank());
        assert_eq!(draft.problem(), None);
    }
}
