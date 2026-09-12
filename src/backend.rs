//! Backend polling (v0 contract, see resolution on the backend ticket).
//!
//! - [`PollSource`]: one poll round -> the fixes seen this round.
//! - [`FileReplay`]: dev default. Replays canned frames from a JSON fixture
//!   (`tests/fixtures/tracks.json`), looping. No network, deterministic.
//! - [`HttpPoll`]: real backend. Not wired yet — returns an error until the
//!   backend exists; swapping impls is one line at the call site.

use std::fs;

use serde::Deserialize;

use crate::geo::track::Fix;

/// One round of polling. Errors are strings; the registry treats a failed
/// round as "no fixes" (ships accumulate misses toward stale).
pub trait PollSource {
    fn poll(&mut self) -> Result<Vec<Fix>, String>;
}

#[derive(Debug, Deserialize)]
struct Fixture {
    frames: Vec<Vec<serde_json::Value>>,
}

/// Replays fixture frames in order, looping forever.
pub struct FileReplay {
    frames: Vec<Vec<Fix>>,
    cursor: usize,
}

impl FileReplay {
    pub fn from_file(path: &str) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let fixture: Fixture = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let mut frames = Vec::with_capacity(fixture.frames.len());
        for (i, raw_frame) in fixture.frames.iter().enumerate() {
            let mut frame = Vec::with_capacity(raw_frame.len());
            for raw in raw_frame {
                let wire: serde_json::Value = raw.clone();
                let text = serde_json::to_string(&wire).map_err(|e| e.to_string())?;
                frame.push(
                    Fix::from_wire_json(&text)
                        .map_err(|e| format!("frame {i}: {e}"))?,
                );
            }
            frames.push(frame);
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
    fn poll(&mut self) -> Result<Vec<Fix>, String> {
        let frame = self.frames[self.cursor % self.frames.len()].clone();
        self.cursor += 1;
        Ok(frame)
    }
}

/// Real HTTP backend (`GET /v0/positions`). Unwired until the backend exists.
pub struct HttpPoll {
    pub base_url: String,
}

impl PollSource for HttpPoll {
    fn poll(&mut self) -> Result<Vec<Fix>, String> {
        Err(format!(
            "HTTP backend not wired yet (base {}) — use FileReplay",
            self.base_url
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_serves_frames_in_order_and_loops() {
        let mut src =
            FileReplay::from_file("tests/fixtures/tracks.json").expect("fixture loads");
        assert!(src.frame_count() >= 2);
        let first = src.poll().unwrap();
        let ids: Vec<&str> = first.iter().map(|f| f.ship_id.as_str()).collect();
        assert!(ids.contains(&"nordwind") && ids.contains(&"ostsee"));
        let n = src.frame_count();
        for _ in 0..n - 1 {
            src.poll().unwrap();
        }
        let again = src.poll().unwrap();
        assert_eq!(again.len(), first.len());
        assert_eq!(again[0].ship_id, first[0].ship_id);
    }

}
