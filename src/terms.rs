//! The exercise's terms as the operator edits them, while planning.
//!
//! THE WIZARD AND THIS PANEL ASK THE SAME QUESTIONS. A session is created
//! with a name, a mode, a pace and four pieces of prose, and the planning
//! phase's ESSENTIALS panel is the same form afterwards: what the exercise
//! is, how much ceremony it carries, and what the Game Master wrote about
//! it. The mode is the one field that is not editable, because the server
//! fixes it at creation and answers "mode may only be set on create" — so
//! the panel shows it and never offers it.
//!
//! WHY IT IS A DRAFT AND NOT A WRITE PER KEYSTROKE. The server takes a
//! PARTIAL edit (`PATCH /games/{id}`): absent leaves a field alone, and an
//! empty string clears it. A field that wrote on every character would be
//! one PATCH per keystroke and would clear a description the moment its
//! author selected all and started retyping. So the form is drafted, the
//! operator's Apply sends the DIFFERENCE, and only the difference — a
//! panel that restates what the server already holds is a panel that can
//! be refused for a reason the operator cannot see, which is the same rule
//! `advance::window_update` follows for the clock.
//!
//! THE WINDOW IS NOT HERE. It has its own draft (`crate::window`) because
//! it existed before this panel and because the advance writes it; this
//! module owns the prose and the pace, and [`combine`] joins the two
//! halves into the one request the server takes.

use crate::backend::{GamePace, GameUpdate, TimeWindow};

/// The text fields the panel edits, in the order it draws them.
///
/// The order is here and not in the renderer for the reason `WindowDraft`'s
/// is: the keystrokes and the draft have to agree about which field is
/// which, and two lists of one order are two lists waiting to disagree.
pub const TEXT_FIELDS: [&str; 4] = ["name", "description", "purpose", "target"];

/// The terms as the server states them, for the diff.
///
/// `pace` is the WIRE WORD and may be empty: a pace this build has not
/// heard of is not a pace, and a form that defaulted one would silently
/// re-pace an exercise every time somebody renamed it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Live {
    pub name: String,
    pub description: String,
    pub purpose: String,
    pub target: String,
    pub pace: String,
}

/// The operator's terms, staged.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TermsDraft {
    pub name: String,
    pub description: String,
    pub purpose: String,
    pub target: String,
    /// The pace as the server's wire word, empty when it named none this
    /// build knows.
    pub pace: String,
}

impl TermsDraft {
    /// The draft as the session's read leaves it: what the exercise already
    /// holds is what the operator edits, so the fields open on the server's
    /// own words rather than on five blanks.
    pub fn from_read(
        name: &str,
        pace: &str,
        description: &str,
        purpose: &str,
        target: &str,
    ) -> Self {
        TermsDraft {
            name: name.to_string(),
            description: description.to_string(),
            purpose: purpose.to_string(),
            target: target.to_string(),
            pace: pace.to_string(),
        }
    }

    /// The four text fields, in the panel's order.
    pub fn fields(&self) -> [&str; 4] {
        [&self.name, &self.description, &self.purpose, &self.target]
    }

    /// One field, by its place in [`TermsDraft::fields`].
    pub fn field(&self, idx: usize) -> &str {
        self.fields()[idx]
    }

    /// Put one field, by its place in [`TermsDraft::fields`].
    pub fn set_field(&mut self, idx: usize, value: String) {
        match idx {
            0 => self.name = value,
            1 => self.description = value,
            2 => self.purpose = value,
            3 => self.target = value,
            _ => {}
        }
    }

    /// The server refuses a nameless session, so the panel does too — the
    /// wizard's own rule, asked of the same field.
    pub fn name_ok(&self) -> bool {
        !self.name.trim().is_empty()
    }

    /// Whether the draft says anything the server does not already hold.
    ///
    /// Read by the Apply button, so a form nobody has touched is not a
    /// button that posts a no-op the server would have to decide about —
    /// the same rule the clock multiplier's Apply follows.
    pub fn dirty(&self, live: &Live) -> bool {
        self.name.trim() != live.name.trim()
            || self.description != live.description
            || self.purpose != live.purpose
            || self.target != live.target
            || self.pace_changed(live)
    }

    /// Whether the pace is a change the panel may write.
    ///
    /// An unknown pace on the server and no pace in the draft is NOT a
    /// change: nothing was chosen, and the write would replace a pace this
    /// build cannot name with one it guessed.
    fn pace_changed(&self, live: &Live) -> bool {
        !self.pace.is_empty() && self.pace != live.pace
    }

    /// The difference, as the one partial edit the server takes.
    ///
    /// `Err` is a draft the server would refuse (a nameless session), said
    /// here so the operator is told in the field rather than after the
    /// round trip. `Ok(None)` is a draft that changes nothing.
    pub fn plan(&self, live: &Live) -> Result<Option<GameUpdate>, String> {
        if !self.name_ok() {
            return Err(
                "Name is required \u{2014} the server refuses a nameless session.".to_string(),
            );
        }
        if !self.dirty(live) {
            return Ok(None);
        }
        let mut update = GameUpdate::default();
        let name = self.name.trim();
        if name != live.name.trim() {
            update.name = Some(name.to_string());
        }
        if self.description != live.description {
            update.description = Some(self.description.clone());
        }
        if self.purpose != live.purpose {
            update.purpose = Some(self.purpose.clone());
        }
        if self.target != live.target {
            update.target = Some(self.target.clone());
        }
        if self.pace_changed(live) {
            match GamePace::parse(&self.pace) {
                Some(pace) => update.pace = Some(pace),
                // Unreachable from the panel's radios, and refused rather
                // than dropped: a pace this build cannot spell is not a pace
                // the server would take, and a draft that asked for it must
                // hear that in the form and not as a 400.
                None => return Err(format!("{} is not a pace this build knows", self.pace)),
            }
        }
        Ok(Some(update))
    }
}

/// Join the two halves of one edit into the request the server takes.
///
/// The terms and the window are ONE PATCH: the panel draws them together,
/// the operator presses one verb, and Minos accepts a change to the terms
/// only while the exercise is in `planning` — so two requests would be two
/// chances to half-write a form. `None` on both sides is nothing to write;
/// `None` on one side leaves that half alone, which is the partial edit's
/// own meaning.
pub fn combine(terms: Option<GameUpdate>, window: Option<GameUpdate>) -> Option<GameUpdate> {
    match (terms, window) {
        (None, None) => None,
        (Some(update), None) | (None, Some(update)) => Some(update),
        (Some(mut terms), Some(window)) => {
            terms.name = terms.name.or(window.name);
            terms.description = terms.description.or(window.description);
            terms.purpose = terms.purpose.or(window.purpose);
            terms.target = terms.target.or(window.target);
            terms.area = terms.area.or(window.area);
            terms.map_tag = terms.map_tag.or(window.map_tag);
            terms.pace = terms.pace.or(window.pace);
            terms.window = TimeWindow {
                actual_start: terms.window.actual_start.or(window.window.actual_start),
                actual_end: terms.window.actual_end.or(window.window.actual_end),
                assumed_start: terms.window.assumed_start.or(window.window.assumed_start),
                assumed_end: terms.window.assumed_end.or(window.window.assumed_end),
            };
            Some(terms)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live() -> Live {
        Live {
            name: "GPU step proof".into(),
            description: "exercise game".into(),
            purpose: "prove the declare path".into(),
            target: "the board".into(),
            pace: "standard".into(),
        }
    }

    /// The draft opens on the server's own words: an editor that starts
    /// blank makes the operator retype a plan that is already written.
    #[test]
    fn the_draft_opens_on_what_the_server_holds() {
        let draft = TermsDraft::from_read(
            "GPU step proof",
            "standard",
            "exercise game",
            "prove the declare path",
            "the board",
        );
        assert_eq!(
            draft.fields(),
            [
                "GPU step proof",
                "exercise game",
                "prove the declare path",
                "the board",
            ]
        );
        assert_eq!(draft.pace, "standard");
        assert!(!draft.dirty(&live()), "re-reading is not an edit");
        assert!(draft.plan(&live()).unwrap().is_none(), "nothing to write");
    }

    /// ONLY THE CHANGED FIELD GOES. The server merges a partial edit key by
    /// key, and a write that restates a field can be refused for a reason
    /// the operator never caused.
    #[test]
    fn only_the_changed_field_is_written() {
        let mut draft = TermsDraft::from_read(
            "GPU step proof",
            "standard",
            "exercise game",
            "prove the declare path",
            "the board",
        );
        draft.set_field(2, "prove the OTHER path".into());
        let update = draft.plan(&live()).unwrap().expect("a change");
        assert_eq!(update.purpose.as_deref(), Some("prove the OTHER path"));
        assert!(update.name.is_none());
        assert!(update.description.is_none());
        assert!(update.target.is_none());
        assert!(update.pace.is_none());
        assert!(update.window.is_empty());
    }

    /// A cleared field is a CLEARED field: empty is a value the server
    /// accepts for prose, and a panel that read empty as "leave alone"
    /// could never delete a sentence.
    #[test]
    fn an_emptied_description_clears_it() {
        let mut draft =
            TermsDraft::from_read("GPU step proof", "standard", "exercise game", "", "");
        draft.set_field(1, String::new());
        let update = draft.plan(&live()).unwrap().expect("a change");
        assert_eq!(update.description.as_deref(), Some(""));
    }

    /// A nameless session is refused HERE, in the field, rather than by the
    /// server after a round trip — the wizard's rule, and the same sentence.
    #[test]
    fn a_nameless_session_is_refused_before_the_request() {
        let mut draft = TermsDraft::from_read("GPU step proof", "standard", "", "", "");
        draft.set_field(0, "   ".into());
        let why = draft.plan(&live()).unwrap_err();
        assert!(why.starts_with("Name is required"), "{why}");
    }

    /// The pace is written only when the operator moved it. An unknown pace
    /// on the server is not a pace, and a form that defaulted one would
    /// re-pace an exercise nobody asked to re-pace.
    #[test]
    fn the_pace_is_written_only_when_it_moved() {
        // A session whose prose is unwritten, so the pace is the only
        // question this draft has an opinion about.
        let live = Live {
            name: "GPU step proof".into(),
            pace: "standard".into(),
            ..Default::default()
        };
        let mut draft = TermsDraft::from_read("GPU step proof", "", "", "", "");
        assert!(!draft.dirty(&live), "an unknown pace is not a choice");
        assert!(draft.plan(&live).unwrap().is_none());
        draft.pace = "fast".into();
        let update = draft.plan(&live).unwrap().expect("a change");
        assert_eq!(update.pace, Some(GamePace::Fast));
    }

    /// The two halves join into ONE request: the panel draws the terms and
    /// the window together, and the server takes them as one partial edit.
    #[test]
    fn the_terms_and_the_window_are_one_write() {
        let mut terms = GameUpdate::default();
        terms.name = Some("Renamed".into());
        let window = GameUpdate {
            window: TimeWindow {
                actual_start: Some("2026-11-01T08:00:00+00:00".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let joined = combine(Some(terms), Some(window)).expect("a write");
        assert_eq!(joined.name.as_deref(), Some("Renamed"));
        assert!(joined.window.actual_start.is_some());
        // And half an edit is still that half, not a blank write.
        assert!(combine(None, None).is_none());
        let only_terms = combine(
            Some(GameUpdate {
                name: Some("Renamed".into()),
                ..Default::default()
            }),
            None,
        )
        .expect("a write");
        assert_eq!(only_terms.name.as_deref(), Some("Renamed"));
        assert!(only_terms.window.is_empty());
    }
}
