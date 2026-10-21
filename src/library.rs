//! STUB — the real module arrives from the library branch. Kept only so the
//! console compiles while that branch is merged; it is replaced wholesale.

use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedStep {
    pub content: String,
    pub start_hour: Option<String>,
    pub end_hour: Option<String>,
    pub location: Option<Place>,
    pub related: Vec<(String, i64)>,
}

impl SavedStep {
    pub fn window(&self) -> Option<(String, String)> {
        self.start_hour.clone().zip(self.end_hour.clone())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SavedScenario {
    pub id: String,
    pub title: String,
    pub description: String,
    pub start: Option<String>,
    pub end: Option<String>,
    pub steps: Vec<SavedStep>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    pub scenarios: Vec<SavedScenario>,
    failed: bool,
}

impl Library {
    pub fn load(_path: &Path) -> Library {
        Library::default()
    }
    pub fn save(&self, _path: &Path) -> Result<(), String> {
        Ok(())
    }
    pub fn get(&self, _id: &str) -> Option<&SavedScenario> {
        None
    }
    pub fn upsert(&mut self, scenario: SavedScenario) -> String {
        let id = if scenario.id.is_empty() {
            new_id()
        } else {
            scenario.id.clone()
        };
        self.scenarios.push(scenario);
        id
    }
    pub fn remove(&mut self, _id: &str) -> bool {
        false
    }
    pub fn sorted(&self) -> Vec<&SavedScenario> {
        self.scenarios.iter().collect()
    }
    pub fn last_read_failed(&self) -> bool {
        self.failed
    }
}

pub fn new_id() -> String {
    String::new()
}

pub fn place_text(place: &Place) -> String {
    format!("at {:.4}, {:.4}", place.lat, place.lon)
}

pub fn stage_into(draft: &mut crate::book::BookDraft, saved: &SavedScenario) -> i64 {
    let local = draft.stage_scenario(&saved.title);
    for step in &saved.steps {
        if let Some(step_local) = draft.stage_step_new(local, &step.content, step.window()) {
            if !step.related.is_empty() {
                draft.stage_relations(local, step_local, step.related.clone());
            }
        }
    }
    local
}
