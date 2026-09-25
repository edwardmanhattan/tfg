//! Dev replay sources (backend split, step 1).
//!
//! - [`FileReplay`]: dev default. Replays canned frames from a JSON fixture
//!   (`tests/fixtures/tracks.json`), looping. No network, deterministic.
//! - [`now_ts`]: the serve-time clock replay frames are stamped with.
//! - [`parse_frame`]: wire-JSON frame parsing, shared with the legacy
//!   HTTP poll until that moves out too.

use std::fs;

use serde::Deserialize;

use crate::geo::track::Fix;

use super::{BackendError, PollSource};

#[derive(Debug, Deserialize)]
struct Fixture {
    frames: Vec<Vec<serde_json::Value>>,
}

/// Current UTC time, millis precision, fixed-width (lexicographically ordered).
/// Sources stamp this on serve; the registry compares `ts` as strings.
pub fn now_ts() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// Stamp one poll round with receipt time (UTC, millis).
///
/// Replays loop canned frames, so wire `ts` rewinds every cycle and the
/// registry (rightly) drops it as out-of-order. A live backend emits fresh
/// timestamps; the replay sources model that by stamping on serve.
fn stamp_now(frame: &mut [Fix]) {
    let now = now_ts();
    for fix in frame {
        fix.ts = now.clone();
        // Receipt time too: replayed frames are fresh on serve, so the
        // old-data badge (wire-gated) stays quiet on fixtures.
        fix.received_at = Some(now.clone());
    }
}

/// Parse one poll round from wire JSON values (flat lat/lon per fix).
pub(crate) fn parse_frame(raw_frame: Vec<serde_json::Value>) -> Result<Vec<Fix>, String> {
    let mut frame = Vec::with_capacity(raw_frame.len());
    for raw in raw_frame {
        let text = serde_json::to_string(&raw).map_err(|e| e.to_string())?;
        frame.push(Fix::from_wire_json(&text)?);
    }
    Ok(frame)
}

/// Replays fixture frames in order, looping forever.
pub struct FileReplay {
    frames: Vec<Vec<Fix>>,
    cursor: usize,
}

impl FileReplay {
    pub fn from_file(path: &str) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::from_json(&text)
    }

    /// Parse a fixture already held in memory, including one embedded in
    /// the executable. Installed builds need no scenario sidecar files.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let fixture: Fixture = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut frames = Vec::with_capacity(fixture.frames.len());
        for (i, raw_frame) in fixture.frames.iter().enumerate() {
            frames.push(
                parse_frame(raw_frame.clone()).map_err(|e| format!("frame {i}: {e}"))?,
            );
        }
        if frames.is_empty() {
            return Err("fixture has no frames".into());
        }
        Ok(Self { frames, cursor: 0 })
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

impl PollSource for FileReplay {
    fn poll(&mut self) -> Result<Vec<Fix>, BackendError> {
        let mut frame = self.frames[self.cursor % self.frames.len()].clone();
        self.cursor += 1;
        stamp_now(&mut frame);
        Ok(frame)
    }
}
