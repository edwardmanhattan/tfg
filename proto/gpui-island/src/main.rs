//! The GPUI island, driven from the crate (U8 step 3).
//!
//! The shell moved into the crate for the cutover, so this proto no longer
//! carries its own copy: it drives the same file `src/bin/tfg_gpui.rs` runs.
//! That is the point — the parity proof and the shipped binary cannot drift,
//! which is the pattern `view`, `map_gate` and `halo` already use.
//!
//! The shims below exist because the shell addresses its kernel by `crate::`
//! path, which is right for the crate and meaningless here. Each names the
//! same file the crate compiles, so there is still one implementation — the
//! shims are paths, not copies.
//!
//! `fx` is a re-export rather than an inline module because a `#[path]` inside
//! an inline module resolves against the inline module's own directory, which
//! is a directory that does not exist.
//!
//! NO `map_render` shim, deliberately: the proto feeds the shell from a
//! directory of PNGs (`run_replay`) precisely so this crate never builds
//! MapLibre. Its whole point is a fast, isolated loop for the chrome.

#[path = "../../../src/view/mod.rs"]
mod view;
#[path = "../../../src/gamestate.rs"]
mod gamestate;
#[path = "../../../src/book.rs"]
mod book;
#[path = "../../../src/symbology/mod.rs"]
mod symbology;
#[path = "../../../src/map_gate.rs"]
mod map_gate;
// `force` and `roster` are the console's own staging, toolkit-free, so the
// shell can name them and the proto compiles the same files. `force` reaches
// `geo::coordinates` for `GeoPosition`; the proto shims only that leaf and
// never `geo/track.rs`, which carries chrono + serde and a track registry the
// shell does not draw. The re-export rather than an inline `mod geo` is the
// documented trap: a `#[path]` inside an inline module resolves against the
// inline module's own (nonexistent) directory.
#[path = "../../../src/geo/coordinates.rs"]
pub mod geo_coordinates;
mod geo {
    pub use crate::geo_coordinates as coordinates;
}
#[path = "../../../src/force.rs"]
mod force;
#[path = "../../../src/roster.rs"]
mod roster;
// The drag-and-drop vocabulary is a leaf: no `use`, no toolkit, no kernel — so
// the shell can name `crate::dnd` and the proto compiles the very same file.
// `groups` is deliberately NOT shimmed: it pulls `crate::command` and would
// cascade, so the shell never names it.
#[path = "../../../src/dnd.rs"]
mod dnd;
#[path = "../../../src/halo.rs"]
mod halo;

// `sfx` is the UI's sounds, which only the shipped binary plays: it opens an
// audio device through rodio, and this proto exists to stay clear of that
// whole tree. The shell names `crate::sfx::play` for every click, ding and
// buzz, so the proto carries the same shape as a silent no-op — the console's
// sounds are garnish, and a chrome loop is not a console.
#[allow(dead_code)]
mod sfx {
    #[derive(Debug, Clone, Copy)]
    pub enum Sfx {
        Click,
        Ding,
        Buzz,
    }
    pub fn play(_kind: Sfx) {}
}

#[path = "../../../src/shell_gpui.rs"]
mod shell;

// `backend` is the HTTP stack (reqwest over the local store), which is the
// whole thing this proto exists to stay clear of. The shell names only nine
// of its plain-data read types — the Assessment island's timeline events and
// their page, judgements and reviews, the scenario clock answer, and one
// hull's embarked passengers with the write answers — so the proto carries
// those shapes and nothing else.
//
// This is the ONE shim that is not a `#[path]` to a real file: `master.rs`
// cannot compile without reqwest. The shapes mirror `backend/master.rs`. If
// the shell ever reads a field the shim lacks, this crate fails to compile —
// which is the guard that keeps the two from drifting apart silently.
#[allow(dead_code)]
mod backend {
    use serde_json::Value;

    #[derive(Debug, Clone, PartialEq)]
    pub struct TimelineEvent {
        pub id: String,
        pub etype: String,
        pub assumed_at: String,
        pub data: Value,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct TimelinePage {
        pub events: Vec<TimelineEvent>,
        pub next_cursor: Option<String>,
        pub has_more: bool,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct GameClockSegment {
        pub factor: f64,
        pub effective_from: String,
        pub source: String,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct GameClock {
        pub state: String,
        pub actual_start: Option<String>,
        pub assumed_start: Option<String>,
        pub assumed_now: Option<String>,
        pub time_factor: f64,
        pub accepting_actions: bool,
        pub running: bool,
        pub segments: Vec<GameClockSegment>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct EmbarkedUnit {
        pub id: i64,
        pub name: String,
        pub hull_number: String,
        pub class_name: String,
        pub is_cleared: bool,
        pub cleared_at: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct OverCapacity {
        pub level: String,
        pub id_level: i64,
        pub max_aboard: i64,
        pub would_hold: i64,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct EmbarkationResult {
        pub units: Vec<EmbarkedUnit>,
        pub over_capacity: Vec<OverCapacity>,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct Review {
        pub id: i64,
        pub personnel_id: i64,
        pub personnel_name: String,
        pub author_id: i64,
        pub author_name: String,
        pub body: String,
        pub revised: bool,
        pub created_at: String,
        pub updated_at: String,
    }

    #[derive(Debug, Clone, PartialEq)]
    pub struct Judgement {
        pub id: i64,
        pub personnel_id: i64,
        pub personnel_name: String,
        pub judge_id: i64,
        pub judge_name: String,
        pub cited_unit: Option<String>,
        pub assumed_at: String,
        pub score: String,
        pub created_at: String,
    }
}

fn main() {
    let dir = std::env::var("MAP_FRAMES_DIR")
        .unwrap_or_else(|_| "/tmp/opencode/mapframes".to_string());
    let burst_ms: u64 = std::env::var("BURST_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(80);
    let looping = std::env::var("LOOP").as_deref() == Ok("1");
    shell::run_replay(dir, burst_ms, looping);
}
