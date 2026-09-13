//! Mock v0 backend: serves scenario frames over HTTP, one per poll.
//!
//! `GET /v0/positions` returns the next frame as a JSON array of wire
//! fixes, advancing round-robin (mirrors the real backend's poll
//! semantics against canned data). Anything else is 404.
//!
//! Scenarios (dev tool):
//! - default (no args): the presentation loop (`tests/fixtures/tracks.json`)
//! - `surge`: 5 ships converging (traffic-surge demo)
//! - `ghost`: a ship that vanishes after 2 frames (stale demo)
//! - `dark`: backend alive, nothing reporting (all-stale demo)
//!
//! Run: `cargo run --example mock_backend -- [scenario]`
//! Then: `TFG_BACKEND_URL=http://127.0.0.1:3000 scripts/run-egui-window.sh`

use std::sync::{Arc, Mutex};

fn main() {
    let scenario = std::env::args().nth(1);
    let path = match scenario.as_deref() {
        None => "tests/fixtures/tracks.json".to_string(),
        Some(name) => format!("scenarios/{name}.json"),
    };
    let text = std::fs::read_to_string(&path).expect("scenario present");
    let fixture: serde_json::Value = serde_json::from_str(&text).expect("fixture parses");
    let frames = fixture["frames"].as_array().expect("frames array").clone();
    let state = Arc::new(Mutex::new(0usize));
    let n = frames.len();
    println!("mock v0 backend [{path}]: {n} frames on http://127.0.0.1:3000/v0/positions");

    let server = tiny_http::Server::http("127.0.0.1:3000").expect("bind 3000");
    for rq in server.incoming_requests() {
        if rq.url() != "/v0/positions" {
            let _ = rq.respond(tiny_http::Response::empty(404));
            continue;
        }
        let i = {
            let mut cursor = state.lock().expect("cursor lock");
            let i = *cursor % n;
            *cursor += 1;
            i
        };
        let mut frame = frames[i].clone();
        // Receipt stamp: the replay loops, so canned `ts` rewinds every
        // cycle; a live backend emits fresh timestamps (see backend.rs).
        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
        if let Some(arr) = frame.as_array_mut() {
            for fix in arr {
                fix["ts"] = serde_json::Value::String(now.clone());
            }
        }
        let body = serde_json::to_string(&frame).expect("frame serializes");
        eprintln!("served frame {i} ({} ship(s))", frames[i].as_array().map(|a| a.len()).unwrap_or(0));
        let _ = rq.respond(
            tiny_http::Response::from_string(body)
                .with_header("Content-Type: application/json".parse::<tiny_http::Header>().expect("header")),
        );
    }
}
