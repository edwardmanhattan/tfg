//! Mock v0 backend: serves scenario frames over HTTP, one per poll,
//! plus session identity endpoints for the invite slice (task #35).
//!
//! Positions (unchanged):
//! `GET /v0/positions` returns the next frame as a JSON array of wire
//! fixes, advancing round-robin (mirrors the real backend's poll
//! semantics against canned data).
//!
//! Invites (in-memory; redemption over the network is a later slice):
//! - `POST /v0/invites` {user, seat, code?} -> the record (201). A
//!   client-provided code is honored when unused (the organizer app is
//!   the source of truth and mirrors local records here); otherwise the
//!   mock mints `TFG-XXXX`.
//! - `GET /v0/invites` -> all records.
//! - `POST /v0/invites/redeem` {code} -> the record with redeemed=true,
//!   or 404. Lets dev clients exercise the future login path today.
//!
//! Anything else is 404.
//!
//! Scenarios (dev tool):
//! - default (no args): the presentation loop (`tests/fixtures/tracks.json`)
//! - `surge`: 5 ships converging (traffic-surge demo)
//! - `ghost`: a ship that vanishes after 2 frames (stale demo)
//! - `dark`: backend alive, nothing reporting (all-stale demo)
//!
//! Run: `cargo run --example mock_backend -- [scenario]`
//! Then: `TFG_BACKEND_URL=http://127.0.0.1:3000 scripts/run-egui-window.sh`

use std::io::Read;
use std::sync::{Arc, Mutex};

use tfg::backend::Invite;

fn json_response(status: u16, value: &serde_json::Value) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    tiny_http::Response::from_string(serde_json::to_string(value).expect("json serializes"))
        .with_status_code(status)
        .with_header("Content-Type: application/json".parse::<tiny_http::Header>().expect("header"))
}

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
    let invites: Arc<Mutex<Vec<Invite>>> = Arc::new(Mutex::new(Vec::new()));
    let counter: Arc<Mutex<u32>> = Arc::new(Mutex::new(1));
    let n = frames.len();
    println!("mock v0 backend [{path}]: {n} frames on http://127.0.0.1:3000/v0/positions");

    let server = tiny_http::Server::http("127.0.0.1:3000").expect("bind 3000");
    for mut rq in server.incoming_requests() {
        let post = matches!(rq.method(), tiny_http::Method::Post);
        let url = rq.url().to_string();
        if !post && url == "/v0/positions" {
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
            continue;
        }
        if !post && url == "/v0/invites" {
            let store = invites.lock().expect("invites lock");
            let _ = rq.respond(json_response(200, &serde_json::json!(store.clone())));
            continue;
        }
        if post && url == "/v0/invites" {
            let mut body = String::new();
            if rq.as_reader().read_to_string(&mut body).is_err() {
                let _ = rq.respond(json_response(400, &serde_json::json!({"error": "unreadable body"})));
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(_) => {
                    let _ = rq.respond(json_response(400, &serde_json::json!({"error": "body must be JSON"})));
                    continue;
                }
            };
            let (Some(user), Some(seat)) = (v["user"].as_str(), v["seat"].as_str()) else {
                let _ = rq.respond(json_response(400, &serde_json::json!({"error": "need user + seat"})));
                continue;
            };
            let mut store = invites.lock().expect("invites lock");
            // Client codes win when unused: the organizer app mirrors.
            if let Some(code) = v["code"].as_str() {
                if let Some(rec) = store.iter().find(|r| r.code == code) {
                    let _ = rq.respond(json_response(200, &serde_json::json!(rec.clone())));
                    continue;
                }
                let rec = Invite { code: code.to_string(), user: user.to_string(), seat: seat.to_string(), redeemed: false };
                store.push(rec.clone());
                eprintln!("mirrored invite {} for {user}", rec.code);
                let _ = rq.respond(json_response(201, &serde_json::json!(rec)));
                continue;
            }
            let mut next = counter.lock().expect("counter lock");
            let rec = Invite {
                code: format!("TFG-{:04}", *next),
                user: user.to_string(),
                seat: seat.to_string(),
                redeemed: false,
            };
            *next += 1;
            store.push(rec.clone());
            eprintln!("issued invite {} for {user}", rec.code);
            let _ = rq.respond(json_response(201, &serde_json::json!(rec)));
            continue;
        }
        if post && url == "/v0/invites/redeem" {
            let mut body = String::new();
            if rq.as_reader().read_to_string(&mut body).is_err() {
                let _ = rq.respond(json_response(400, &serde_json::json!({"error": "unreadable body"})));
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(_) => {
                    let _ = rq.respond(json_response(400, &serde_json::json!({"error": "body must be JSON"})));
                    continue;
                }
            };
            let Some(code) = v["code"].as_str() else {
                let _ = rq.respond(json_response(400, &serde_json::json!({"error": "need code"})));
                continue;
            };
            let mut store = invites.lock().expect("invites lock");
            match store.iter_mut().find(|r| r.code == code) {
                Some(rec) => {
                    rec.redeemed = true;
                    eprintln!("redeemed invite {code}");
                    let _ = rq.respond(json_response(200, &serde_json::json!(rec.clone())));
                }
                None => {
                    let _ = rq.respond(json_response(404, &serde_json::json!({"error": "unknown code"})));
                }
            }
            continue;
        }
        let _ = rq.respond(tiny_http::Response::empty(404));
    }
}
