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

/// Stamp one poll round with receipt time (UTC, millis).
///
/// Replays loop canned frames, so wire `ts` rewinds every cycle and the
/// registry (rightly) drops it as out-of-order. A live backend emits fresh
/// timestamps; the replay sources model that by stamping on serve.
fn stamp_now(frame: &mut [Fix]) {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    for fix in frame {
        fix.ts = now.clone();
    }
}

/// Parse one poll round from wire JSON values (flat lat/lon per fix).
fn parse_frame(raw_frame: Vec<serde_json::Value>) -> Result<Vec<Fix>, String> {
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
        let fixture: Fixture = serde_json::from_str(&text).map_err(|e| e.to_string())?;
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
    fn poll(&mut self) -> Result<Vec<Fix>, String> {
        let mut frame = self.frames[self.cursor % self.frames.len()].clone();
        self.cursor += 1;
        stamp_now(&mut frame);
        Ok(frame)
    }
}

/// Real HTTP backend: `GET {base_url}/v0/positions` returning a JSON array
/// of wire fixes. Against `examples/mock_backend.rs` today, the real
/// backend tomorrow: same contract, different URL.
pub struct HttpPoll {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl HttpPoll {
    pub fn new(base_url: &str) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { base_url: base_url.trim_end_matches('/').to_string(), client })
    }
}

impl PollSource for HttpPoll {
    fn poll(&mut self) -> Result<Vec<Fix>, String> {
        let url = format!("{}/v0/positions", self.base_url);
        let fixes: Vec<serde_json::Value> = self
            .client
            .get(&url)
            .send()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())?;
        parse_frame(fixes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_poll_fetches_mock_backend() {
        let body = r#"[{"ship_id":"a","lat":53.5,"lon":9.9,"ts":"2026-09-12T00:00:00Z"}]"#;
        let server = tiny_http::Server::http("127.0.0.1:18080").expect("bind test port");
        std::thread::spawn(move || {
            for rq in server.incoming_requests().take(1) {
                assert_eq!(rq.url(), "/v0/positions");
                let _ = rq.respond(tiny_http::Response::from_string(body));
            }
        });
        let mut src = HttpPoll::new("http://127.0.0.1:18080").expect("client builds");
        let fixes = src.poll().expect("poll succeeds");
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].ship_id, "a");
        assert_eq!(fixes[0].position.latitude, 53.5);
    }

    #[test]
    fn replay_loop_stays_fresh_past_wrap() {
        use crate::geo::track::Registry;
        let mut src =
            FileReplay::from_file("tests/fixtures/tracks.json").expect("fixture loads");
        let mut reg = Registry::default();
        let n = src.frame_count();
        for _ in 0..n {
            let frame = src.poll().unwrap();
            reg.poll(frame);
        }
        let before = reg
            .ships()
            .iter()
            .find(|s| s.ship_id == "nordwind")
            .expect("nordwind tracked")
            .latest
            .ts
            .clone();
        // Wrap: frame 0 re-served. Without receipt stamping the registry
        // would drop it as out-of-order and freeze the inspector.
        std::thread::sleep(std::time::Duration::from_millis(5));
        let frame = src.poll().unwrap();
        reg.poll(frame);
        let after = reg
            .ships()
            .iter()
            .find(|s| s.ship_id == "nordwind")
            .expect("nordwind tracked")
            .latest
            .ts
            .clone();
        assert!(after > before, "wrapped frame accepted: {after} > {before}");
    }

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
