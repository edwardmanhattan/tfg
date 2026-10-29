//! The scenario library: the operator's own catalog of saved scenarios.
//!
//! WHY THE CATALOG IS OURS AND NOT MINOS'S. A game has exactly one book, and
//! the book is that game's ordered scenarios — there is no template of a
//! scenario anywhere in the Minos contract, and no catalog either. So
//! "compose a scenario now, choose it in another game's planning" is a
//! request the server cannot answer, and the answer has to live on this
//! machine: one JSON file beside the local store, holding each plan whole —
//! name, description, window, steps, and each step's place and its related
//! hulls.
//!
//! THE RULE THIS MODULE ENFORCES: THE LIBRARY IS THE SOURCE OF TRUTH FOR A
//! PLAN, AND THE BOOK IS A DEPLOYMENT OF IT. A saved scenario pushed into a
//! game becomes that game's scenario; the game's copy is what the server
//! holds, and the library keeps its own. Nothing here writes to the network:
//! the only thing that reaches Minos is a stage advance, and the staging
//! lives in `crate::book`.
//!
//! THE FILE IS THE WHOLE LIBRARY: ONE JSON OBJECT, {"scenarios": [...]}. A
//! machine that has never saved a scenario has no library, and that is not a
//! failure — a missing file reads as an empty library. A file that cannot be
//! read or cannot be understood reads as an empty library TOO, but it says so
//! through `last_read_failed`: a corrupt file must not take the console down
//! with it, and it must not be mistaken for a library the operator never
//! had. The two cases look the same on purpose — a half-written file and an
//! absent file leave the operator with no saved scenarios either way — and
//! the flag is what tells them apart.
//!
//! A STEP'S WINDOW IS BOTH ENDS OR NEITHER, the same shape
//! `crate::window`'s rule answers to, because the exercise's own window and
//! the composer's step row are one shape and two answers could drift apart.
//! The library does not re-answer that rule — the console asks `crate::window`
//! before a step is ever saved — but it does keep the SHAPE: a half window
//! is repaired to none on the way in, because the server would refuse it and
//! the operator would be holding a plan that cannot be deployed. A file can
//! break that shape, being hand-edited or written by a version that did not
//! know it, and a half window is dropped rather than posted.
//!
//! WRITING IS ATOMIC. A temp file beside the target, renamed over it, so a
//! crash mid-write leaves the old library intact rather than half a file.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::book::BookDraft;

/// Where a step points, when the author dropped a pin.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub lat: f64,
    pub lon: f64,
}

/// One step of a saved scenario.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedStep {
    pub content: String,
    /// `HHMM` on the assumed clock. Both ends or neither.
    pub start_hour: Option<String>,
    pub end_hour: Option<String>,
    /// The place the step points at, when the author dropped a pin.
    pub location: Option<Place>,
    /// The hulls (and task-organisation nodes) the step obliges, in the
    /// book's own shape: `(kind, id)`, kind being "hull" or "node".
    pub related: Vec<(String, i64)>,
}

impl SavedStep {
    /// The step's window as the pair the book takes, or nothing at all.
    ///
    /// Both ends or neither: a step with one end is not a window the server
    /// would take, so it is not a window here either.
    pub fn window(&self) -> Option<(String, String)> {
        self.start_hour.clone().zip(self.end_hour.clone())
    }

    /// Drop half a window, keeping a whole one exactly as it was written.
    fn repair(&mut self) {
        if self.start_hour.is_some() != self.end_hour.is_some() {
            self.start_hour = None;
            self.end_hour = None;
        }
    }
}

/// One saved scenario: the plan, whole.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedScenario {
    /// The library's own key. Never a server id: a saved scenario outlives
    /// every game it was ever pushed into.
    pub id: String,
    pub title: String,
    pub description: String,
    /// The plan's window, as a LOCAL datetime: `YYYY-MM-DDTHH:MM`, no zone.
    pub start: Option<String>,
    pub end: Option<String>,
    pub steps: Vec<SavedStep>,
}

/// The whole library as it is held on disk.
///
/// The read-failure flag rides along with the value rather than beside it:
/// a caller that has just read the library wants to know whether the file it
/// read was the file it asked for, and the answer belongs to the library it
/// read, not to some global the next read would overwrite.
///
/// The flag never reaches the file: the file is the SCENARIOS, and a report
/// about one read is not part of what the operator keeps.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub scenarios: Vec<SavedScenario>,
    /// Whether the last read found a file it could not read.
    #[serde(skip)]
    read_failed: bool,
}

impl Library {
    /// Read the library. A missing file is an EMPTY library, never an error:
    /// a machine that has never saved a scenario has no library, and that is
    /// not a failure. A malformed file is also empty — a corrupt file must
    /// not take the console down with it — and the caller is told by
    /// `last_read_failed`.
    pub fn load(path: &Path) -> Library {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Library::default();
            }
            Err(_) => {
                return Library {
                    scenarios: Vec::new(),
                    read_failed: true,
                };
            }
        };
        match serde_json::from_str::<Library>(&text) {
            Ok(mut library) => {
                library.repair();
                library.read_failed = false;
                library
            }
            Err(_) => Library {
                scenarios: Vec::new(),
                read_failed: true,
            },
        }
    }

    /// Write the library. Atomic: a temp file beside the target, renamed
    /// over it, so a crash mid-write leaves the old library intact.
    ///
    /// The library does not make its own directory: the data dir is the
    /// app's business, and a save that cannot find it says so rather than
    /// quietly growing a tree of its own.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| format!("the library could not be written as JSON: {e}"))?;
        let temporary = temporary_path(path);
        std::fs::write(&temporary, text.as_bytes()).map_err(|e| {
            format!(
                "the library could not be written ({}): {e}",
                temporary.display()
            )
        })?;
        if let Err(e) = std::fs::rename(&temporary, path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(format!(
                "the library could not be put in place ({}): {e}",
                path.display()
            ));
        }
        Ok(())
    }

    /// One saved scenario by id.
    pub fn get(&self, id: &str) -> Option<&SavedScenario> {
        self.scenarios.iter().find(|scenario| scenario.id == id)
    }

    /// Put a scenario into the library, replacing the one with the same id.
    /// A scenario with a blank id is GIVEN one. Answers the id.
    ///
    /// A half window is dropped on the way in, and this is the ONE place the
    /// library is allowed to change what it is given: a saved step's window
    /// is both ends or neither, and a library that held a half would be
    /// holding a plan it could not deploy. The operator's WORDS are never
    /// touched — only the window, and only when it is half.
    pub fn upsert(&mut self, mut scenario: SavedScenario) -> String {
        if scenario.id.trim().is_empty() {
            scenario.id = new_id();
        }
        for step in &mut scenario.steps {
            step.repair();
        }
        let id = scenario.id.clone();
        match self.scenarios.iter().position(|held| held.id == id) {
            Some(index) => self.scenarios[index] = scenario,
            None => self.scenarios.push(scenario),
        }
        id
    }

    /// Take a scenario out of the library. Answers whether it was there.
    pub fn remove(&mut self, id: &str) -> bool {
        match self.scenarios.iter().position(|held| held.id == id) {
            Some(index) => {
                self.scenarios.remove(index);
                true
            }
            None => false,
        }
    }

    /// The library's scenarios in a stable order: by title, case-blind, then
    /// by id, so the list never reshuffles between two frames.
    pub fn sorted(&self) -> Vec<&SavedScenario> {
        let mut sorted: Vec<&SavedScenario> = self.scenarios.iter().collect();
        sorted.sort_by(|a, b| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        sorted
    }

    /// Whether the last `load` found a file it could not read.
    pub fn last_read_failed(&self) -> bool {
        self.read_failed
    }

    /// Repair every step's window, so the library holds only whole ones.
    ///
    /// The repair happens on the way IN, on the file's own terms: a plan
    /// the operator can deploy is a plan whose steps are windows or nothing.
    fn repair(&mut self) {
        for scenario in &mut self.scenarios {
            for step in &mut scenario.steps {
                step.repair();
            }
        }
    }
}

/// A library key that cannot collide with another: the wall clock in
/// milliseconds, then a counter for two saves in the same millisecond.
///
/// The counter is per millisecond, not global: an id reads as a moment, and
/// two scenarios saved in the same moment are told apart by the tail. It is
/// a counter and not a random number because the operator's own eye has to
/// be able to recognise the id of a scenario they saved a moment ago.
pub fn new_id() -> String {
    /// The millisecond the last id was made in, and how many ids that
    /// millisecond has already handed out.
    static SEQUENCE: Mutex<(i64, u64)> = Mutex::new((0, 0));

    let millis = chrono::Utc::now().timestamp_millis();
    let mut sequence = SEQUENCE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (last_millis, count) = *sequence;
    // A clock that steps backwards must not hand out an id that was already
    // handed out, so the counter only ever grows within a millisecond and
    // the millisecond never goes back.
    let (millis, count) = if millis > last_millis {
        (millis, 0)
    } else {
        (last_millis, count + 1)
    };
    *sequence = (millis, count);
    format!("{millis}-{count}")
}

/// The place a step points at, as the step's words carry it: `at {lat:.4},
/// {lon:.4}`. THE SAME DIGITS as the Inspector reads a fix back with — an
/// exercise authored against one position and reviewed against another is a
/// scenario nobody can trust.
pub fn place_text(place: &Place) -> String {
    let Place { lat, lon } = *place;
    format!("at {lat:.4}, {lon:.4}")
}

/// Stage a saved scenario into a book draft: the scenario, then its steps,
/// then each step's related list. Answers the staged scenario's local id.
///
/// THE TITLE GOES WITH THE CREATE; THE PLAN'S WINDOW DOES NOT. Minos holds a
/// title, and a scenario's window in the library is the operator's PLAN —
/// when this scenario is to be run — while the server's timing for it is
/// the step windows on the assumed clock. The composer offers the plan's
/// window as the default for the first and last step, and the operator sees
/// it offered rather than it happening behind them.
///
/// The description is the same scenario's prose and belongs to the same
/// staged scenario, so the local id this answers is the one to set it on.
/// The draft's own shape is not this module's to decide, and the draft that
/// exists today has no place to put a description; what the library can say
/// is WHERE it goes.
///
/// A step that obliges no hull stages no relation write: the step is being
/// created, so it has nothing to clear, and a write that says nothing is
/// noise in the advance the operator is about to read.
pub fn stage_into(draft: &mut BookDraft, saved: &SavedScenario) -> i64 {
    let scenario_local = draft.stage_scenario(&saved.title);
    let mut steps = Vec::new();
    for step in &saved.steps {
        let local = draft.stage_step_new(scenario_local, &step.content, step.window());
        steps.push((step, local));
    }
    for (step, local) in steps {
        let Some(local) = local else { continue };
        if step.related.is_empty() {
            continue;
        }
        draft.stage_relations(scenario_local, local, step.related.clone());
    }
    scenario_local
}

/// The temp file a save writes first: beside the target, so the rename is
/// on the same filesystem and is therefore a rename rather than a copy.
fn temporary_path(path: &Path) -> PathBuf {
    /// How many temp files this process has named, so two saves in the same
    /// moment do not write to the same scratch file.
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "scenarios.json".to_string());
    let write = WRITES.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!("{name}.tmp-{}-{write}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::{BookWrite, LiveScenario, plan};

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("tfg-library-test-{name}-{}", std::process::id()))
    }

    fn step(content: &str) -> SavedStep {
        SavedStep {
            content: content.to_string(),
            ..Default::default()
        }
    }

    fn scenario(title: &str) -> SavedScenario {
        SavedScenario {
            title: title.to_string(),
            ..Default::default()
        }
    }

    /// A machine that has never saved a scenario has no library. That is
    /// not a failure, and the caller must not be told otherwise.
    #[test]
    fn a_missing_file_is_an_empty_library() {
        let path = scratch("missing");
        let _ = std::fs::remove_file(&path);
        let library = Library::load(&path);
        assert!(library.scenarios.is_empty());
        assert!(!library.last_read_failed());
    }

    /// A corrupt file must not take the console down with it, and it must
    /// not be mistaken for a library the operator never had. Both halves of
    /// that are said here: the library is empty AND the flag is up.
    #[test]
    fn a_malformed_file_is_an_empty_library_and_says_so() {
        let path = scratch("malformed");
        std::fs::write(&path, b"{ not json at all").unwrap();
        let library = Library::load(&path);
        assert!(library.scenarios.is_empty());
        assert!(library.last_read_failed());
        let _ = std::fs::remove_file(&path);
    }

    /// The library is the plan, whole: the steps, their windows, their
    /// places and their related hulls all come back as they went in.
    #[test]
    fn a_saved_library_comes_back_whole() {
        let path = scratch("round-trip");
        let mut library = Library::default();
        let id = library.upsert(SavedScenario {
            title: "First light".into(),
            description: "Sweep the strait before dawn.".into(),
            start: Some("2026-11-01T08:00".into()),
            end: Some("2026-11-01T16:00".into()),
            steps: vec![
                SavedStep {
                    content: "sweep north".into(),
                    start_hour: Some("0600".into()),
                    end_hour: Some("0700".into()),
                    location: Some(Place {
                        lat: -5.1234,
                        lon: 119.5678,
                    }),
                    related: vec![("hull".into(), 13), ("node".into(), 5)],
                },
                step("hold"),
            ],
            ..Default::default()
        });
        library.save(&path).unwrap();

        let read = Library::load(&path);
        assert!(!read.last_read_failed());
        assert_eq!(read.scenarios.len(), 1);
        let held = read.get(&id).unwrap();
        assert_eq!(held.title, "First light");
        assert_eq!(held.description, "Sweep the strait before dawn.");
        assert_eq!(held.start.as_deref(), Some("2026-11-01T08:00"));
        assert_eq!(held.steps.len(), 2);
        assert_eq!(held.steps[0].content, "sweep north");
        assert_eq!(held.steps[0].start_hour.as_deref(), Some("0600"));
        assert_eq!(
            held.steps[0].location,
            Some(Place {
                lat: -5.1234,
                lon: 119.5678
            })
        );
        assert_eq!(
            held.steps[0].related,
            vec![("hull".to_string(), 13), ("node".to_string(), 5)]
        );
        assert!(held.steps[1].start_hour.is_none());

        // The file is the whole library, one object, and nothing of the
        // read-flag rides along into it.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.trim_start().starts_with('{'));
        assert!(raw.contains("\"scenarios\""));
        assert!(!raw.contains("read_failed"));
        let _ = std::fs::remove_file(&path);
    }

    /// An upsert replaces the scenario with the same id and leaves the rest
    /// of the library alone; a blank id is GIVEN one, because a scenario
    /// with no key could never be found again.
    #[test]
    fn an_upsert_replaces_by_id_and_gives_a_blank_one_a_name() {
        let mut library = Library::default();
        library.upsert(SavedScenario {
            id: "kept".into(),
            title: "First".into(),
            ..Default::default()
        });
        library.upsert(SavedScenario {
            id: "other".into(),
            title: "Second".into(),
            ..Default::default()
        });

        let id = library.upsert(SavedScenario {
            id: "kept".into(),
            title: "Revised".into(),
            ..Default::default()
        });
        assert_eq!(id, "kept");
        assert_eq!(
            library.scenarios.len(),
            2,
            "the same id is replaced, not added"
        );
        assert_eq!(library.get("kept").unwrap().title, "Revised");

        let fresh = library.upsert(scenario("Third"));
        assert!(!fresh.trim().is_empty(), "a blank id is given one");
        assert!(library.get(&fresh).is_some());
        assert_eq!(library.scenarios.len(), 3);
    }

    /// A delete answers whether it was there: the console has to be able to
    /// say "already gone" rather than claim a success it did not have.
    #[test]
    fn a_remove_answers_whether_it_was_there() {
        let mut library = Library::default();
        library.upsert(SavedScenario {
            id: "held".into(),
            title: "Held".into(),
            ..Default::default()
        });
        assert!(library.remove("held"));
        assert!(!library.remove("held"));
        assert!(library.get("held").is_none());
    }

    /// The list must not reshuffle between two frames, so the order is by
    /// title CASE-BLIND — "Alpha" and "alpha" are the same word to an
    /// operator — and then by id, which is the one thing that cannot tie.
    #[test]
    fn the_sorted_list_is_case_blind_and_stable() {
        let mut library = Library::default();
        library.upsert(SavedScenario {
            id: "3".into(),
            title: "beta".into(),
            ..Default::default()
        });
        library.upsert(SavedScenario {
            id: "1".into(),
            title: "Alpha".into(),
            ..Default::default()
        });
        library.upsert(SavedScenario {
            id: "2".into(),
            title: "alpha".into(),
            ..Default::default()
        });

        let titles: Vec<&str> = library.sorted().iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["Alpha", "alpha", "beta"]);
        let ids: Vec<&str> = library.sorted().iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["1", "2", "3"],
            "the id breaks the tie the title leaves"
        );
    }

    /// The pin's words and the Inspector's reading of the same place are
    /// the same digits, to four decimals: about eleven metres, which says
    /// "here" and not "this rivet".
    #[test]
    fn the_place_text_carries_four_decimals() {
        let text = place_text(&Place {
            lat: -5.12341,
            lon: 119.98762,
        });
        assert_eq!(text, "at -5.1234, 119.9876");
    }

    /// Staging a saved scenario puts the scenario first, then its steps in
    /// the order they were authored, then the relations — and the plan that
    /// comes out says creates, then step adds, then relations, because a
    /// step belongs to a scenario the server has to hold first.
    #[test]
    fn a_staged_scenario_plans_creates_then_steps_then_relations() {
        let saved = SavedScenario {
            title: "First light".into(),
            steps: vec![
                SavedStep {
                    content: "sweep north".into(),
                    start_hour: Some("0600".into()),
                    end_hour: Some("0700".into()),
                    related: vec![("hull".into(), 13)],
                    ..Default::default()
                },
                step("hold"),
            ],
            ..Default::default()
        };
        let mut draft = BookDraft::new();
        let local = stage_into(&mut draft, &saved);
        assert!(local < 0, "a staged scenario is never a server id");

        let staged = draft.get_new(local).unwrap();
        assert_eq!(staged.title, "First light");
        assert_eq!(staged.steps.len(), 2);
        assert_eq!(staged.steps[0].content, "sweep north");
        assert_eq!(staged.steps[1].content, "hold");
        assert_eq!(staged.steps[0].start.as_deref(), Some("0600"));

        let writes = plan(&draft, &[]);
        let kinds: Vec<&str> = writes
            .iter()
            .map(|write| match write {
                BookWrite::Create { .. } => "create",
                BookWrite::AddStep { .. } => "addstep",
                BookWrite::RemoveStep { .. } => "remove",
                BookWrite::EditStep { .. } => "edit",
                BookWrite::SetRelated { .. } => "relate",
                BookWrite::AddSubstep { .. } => "addsub",
                BookWrite::EditSubstep { .. } => "editsub",
                BookWrite::SetAudience { .. } => "audience",
            })
            .collect();
        assert_eq!(kinds, vec!["create", "addstep", "addstep", "relate"]);
        assert!(matches!(&writes[0], BookWrite::Create { title, .. } if title == "First light"));
        assert!(matches!(
            &writes[1],
            BookWrite::AddStep { content, window: Some((_, _)), .. } if content == "sweep north"
        ));
        assert!(
            matches!(&writes[2], BookWrite::AddStep { content, window: None, .. } if content == "hold")
        );
        assert!(
            matches!(&writes[3], BookWrite::SetRelated { refs, .. } if refs == &vec![("hull".to_string(), 13)])
        );
    }

    /// A step's window is both ends or neither, and the file can break that
    /// rule. A half is dropped rather than posted: the server would refuse
    /// it, and the operator would be holding a plan that cannot be deployed.
    #[test]
    fn a_half_window_in_a_saved_step_is_dropped() {
        let path = scratch("half-window");
        let raw = r#"{"scenarios":[{"id":"s","title":"T","description":"","start":null,"end":null,"steps":[{"content":"hold","start_hour":"0600","end_hour":null,"location":null,"related":[]}]}]}"#;
        std::fs::write(&path, raw.as_bytes()).unwrap();

        let library = Library::load(&path);
        assert!(!library.last_read_failed());
        let held = &library.scenarios[0];
        assert!(held.steps[0].start_hour.is_none(), "the half is dropped");
        assert!(held.steps[0].end_hour.is_none());

        // And it is never posted: the plan carries no window at all.
        let mut draft = BookDraft::new();
        let local = stage_into(&mut draft, held);
        let staged = draft.get_new(local).unwrap();
        assert!(staged.steps[0].start.is_none());
        assert!(staged.steps[0].end.is_none());
        let writes = plan(&draft, &[]);
        assert!(matches!(
            &writes[1],
            BookWrite::AddStep { window: None, .. }
        ));
        let _ = std::fs::remove_file(&path);
    }

    /// A half window is dropped on the way IN too, not only when it comes
    /// from a file: a library that held a half would be holding a plan it
    /// could not deploy, and the file it writes would carry the half on.
    #[test]
    fn a_half_window_is_dropped_on_the_way_in() {
        let mut library = Library::default();
        let id = library.upsert(SavedScenario {
            title: "First light".into(),
            steps: vec![SavedStep {
                content: "hold".into(),
                start_hour: Some("0600".into()),
                end_hour: None,
                ..Default::default()
            }],
            ..Default::default()
        });
        let held = library.get(&id).unwrap();
        assert!(held.steps[0].start_hour.is_none(), "the half is dropped");
        assert!(held.steps[0].end_hour.is_none());
        assert_eq!(held.steps[0].content, "hold", "the words are untouched");
    }

    /// The staged relations must name the step they belong to, or the
    /// server would be told about a hull against a step that is not there.
    #[test]
    fn a_staged_relation_names_its_own_step() {
        let saved = SavedScenario {
            title: "First light".into(),
            steps: vec![
                SavedStep {
                    content: "sweep".into(),
                    related: vec![("hull".into(), 13)],
                    ..Default::default()
                },
                SavedStep {
                    content: "hold".into(),
                    related: vec![("node".into(), 5)],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut draft = BookDraft::new();
        let local = stage_into(&mut draft, &saved);
        let staged = draft.get_new(local).unwrap();
        let first = staged.steps[0].local_id;
        let second = staged.steps[1].local_id;
        assert_eq!(
            draft.related(local, first),
            Some(&vec![("hull".to_string(), 13)])
        );
        assert_eq!(
            draft.related(local, second),
            Some(&vec![("node".to_string(), 5)])
        );
    }

    /// Two ids made in the same moment must not be the same id, or the
    /// second save would overwrite the first.
    #[test]
    fn two_ids_are_never_the_same() {
        let first = new_id();
        let second = new_id();
        assert_ne!(first, second);
        assert!(!first.trim().is_empty());
    }

    /// A save leaves no scratch file behind: the file beside the library is
    /// a temp file, not a second library, and a rename over the target is
    /// what makes the write atomic.
    #[test]
    fn a_save_leaves_no_scratch_file_behind() {
        let path = scratch("atomic");
        let mut library = Library::default();
        library.upsert(scenario("First light"));
        library.save(&path).unwrap();
        assert!(path.exists());
        // The scratch file this test's own save would have written, and
        // nothing else: the other tests keep their own names, so a stray
        // temp file here is this save's own litter.
        let target = path.file_name().unwrap().to_string_lossy().into_owned();
        let stray: Vec<String> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(&target) && name != &target)
            .collect();
        assert!(
            stray.is_empty(),
            "the temp file is renamed over the target, not left beside it"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// The plan a saved scenario produces must not carry a scenario the
    /// server already holds: a saved scenario is deployed, never merged.
    #[test]
    fn a_staged_scenario_is_a_new_one_whatever_the_server_holds() {
        let saved = SavedScenario {
            title: "First light".into(),
            ..Default::default()
        };
        let mut draft = BookDraft::new();
        stage_into(&mut draft, &saved);
        let live = vec![LiveScenario {
            id: 7,
            steps: Vec::new(),
        }];
        let writes = plan(&draft, &live);
        assert!(matches!(&writes[0], BookWrite::Create { .. }));
    }

    /// THE SHAPES ARE FIXED, and the console builds them in full — every
    /// field named, no `..Default::default()` to hide a change. A field
    /// added here, or one renamed, breaks the console's own code, so this
    /// test is the shape contract written down.
    #[test]
    fn the_saved_shapes_are_built_in_full() {
        let place = Place {
            lat: -6.2,
            lon: 106.8,
        };
        let step = SavedStep {
            content: "cross the river".to_string(),
            start_hour: Some("1000".to_string()),
            end_hour: Some("1030".to_string()),
            location: Some(place),
            related: vec![("hull".to_string(), 13)],
        };
        let scenario = SavedScenario {
            id: "s1".to_string(),
            title: "First light".to_string(),
            description: "The river crossing.".to_string(),
            start: Some("2026-11-01T08:00".to_string()),
            end: Some("2026-11-01T16:00".to_string()),
            steps: vec![step],
        };
        assert_eq!(scenario.steps[0].location, Some(place));
        assert_eq!(Library::default().scenarios.len(), 0);
    }
}
