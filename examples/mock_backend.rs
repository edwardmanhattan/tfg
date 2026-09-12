//! Mock v0 backend: serves fixture frames over HTTP, one per poll.
//!
//! `GET /v0/positions` returns the next frame as a JSON array of wire
//! fixes, advancing round-robin (mirrors the real backend's poll
//! semantics against canned data). Anything else is 404.
//!
//! Run: `cargo run --example mock_backend`
//! Then: `TFG_BACKEND_URL=http://127.0.0.1:3000 scripts/run-egui-window.sh`

use std::sync::{Arc, Mutex};

fn main() {
    let text =
        std::fs::read_to_string("tests/fixtures/tracks.json").expect("fixture present");
    let fixture: serde_json::Value = serde_json::from_str(&text).expect("fixture parses");
    let frames = fixture["frames"].as_array().expect("frames array").clone();
    let state = Arc::new(Mutex::new(0usize));
    let n = frames.len();
    println!("mock v0 backend: {n} frames on http://127.0.0.1:3000/v0/positions");

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
        let body = serde_json::to_string(&frames[i]).expect("frame serializes");
        eprintln!("served frame {i} ({} ship(s))", frames[i].as_array().map(|a| a.len()).unwrap_or(0));
        let _ = rq.respond(
            tiny_http::Response::from_string(body)
                .with_header("Content-Type: application/json".parse::<tiny_http::Header>().expect("header")),
        );
    }
}
