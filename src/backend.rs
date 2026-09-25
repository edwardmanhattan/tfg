//! Backend clients, split by responsibility:
//!
//! - [`PollSource`]: one poll round -> the fixes seen this round.
//! - [`replay`]: dev fixture replay ([`FileReplay`]) and the serve clock.
//! - [`auth`]: login, refresh, gate probe, keyring.
//! - [`master`]: master-data reads, session-users rows, hull specs.
//! - [`feed`]: snapshot REST, publication shapes.
//! - [`live`]: socket actor, bridge, refusal policy.
//!
//! The parent keeps the trait, the legacy mock clients, the shared
//! envelope helper, and the tests. Everything else lives below.

mod auth;
mod error;
mod feed;
mod live;
mod master;
mod replay;

pub use error::BackendError;

pub use auth::{AuthenticatedUser, MinosAuth, TokenPair, keyring_clear, keyring_load, keyring_save, last_user_clear, last_user_load, last_user_save};
pub use feed::{GameMsg, MinosRest, Snapshot};
pub use live::{LIVE_BACKOFF_BASE_SECS, LIVE_BACKOFF_CAP_SECS, LiveCmd, LiveEvent, LiveWire};
pub use master::{BackendUser, GameClock, GameClockSegment, GameDetail, GameFix, GameHullPos, GameOrderEvent, GamePlacement, GamePositionFix, GamePositionUpdate, GameRow, GameUnit, GameUpdate, HierarchyNode, HullSpec, ImageManifest, InboxMsg, InboxPage, JoinResult, Judgement, MinosMaster, MsgDraft, MsgRecipient, Participant, PlacementList, PositionList, Review, ScenarioRole, TableData, TimelineEvent, TimelinePage, UnitImageEntry};
pub use replay::{FileReplay, now_ts};
// Shared with live.rs and the tests below; not public API.
pub(crate) use feed::FeedEvent;
pub(crate) use feed::{parse_message_event, parse_order_event, parse_positions_event, parse_positions_event_for_game};

use serde::{Deserialize, Serialize};

use crate::geo::track::Fix;

pub trait PollSource {
    fn poll(&mut self) -> Result<Vec<Fix>, BackendError>;
}

/// Mock HTTP source (M10): `GET {base_url}/v0/positions` returning a
/// JSON array of wire fixes, served by `examples/mock_backend.rs`.
/// Explicit mock/replay compatibility only — this is NOT the Minos
/// integration (Minos is `GET /api/v1/live-feed/positions` with a
/// bearer envelope via [`MinosRest`], plus the Centrifugo socket via
/// [`LiveWire`]). Nothing here may be mistaken for a backend route.
pub struct MockPoll {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl MockPoll {
    pub fn new(base_url: &str) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }
}

impl PollSource for MockPoll {
    fn poll(&mut self) -> Result<Vec<Fix>, BackendError> {
        let url = format!("{}/v0/positions", self.base_url);
        let fixes: Vec<serde_json::Value> = self.client.get(&url).send()?.json()?;
        replay::parse_frame(fixes).map_err(BackendError::Other)
    }
}

/// Invite record (slice v, grill #30): a code binding a roster user to a
/// seat label. Issued locally by the organizer (source of truth) and
/// mirrored to the mock backend when connected; redemption over the
/// network is a later slice — the mock validates codes today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub code: String,
    pub user: String,
    pub seat: String,
    pub redeemed: bool,
}

/// Identity endpoints against the mock (later real) backend: issue, list,
/// redeem. Same contract style as [`MockPoll`]: base URL + blocking client.
pub struct InviteClient {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl InviteClient {
    pub fn new(base_url: &str) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }

    /// Mirror a locally issued record: the mock honors the client code
    /// when unused and mints `TFG-XXXX` otherwise.
    pub fn issue(&self, user: &str, seat: &str, code: &str) -> Result<Invite, String> {
        self.client
            .post(&format!("{}/v0/invites", self.base_url))
            .json(&serde_json::json!({"user": user, "seat": seat, "code": code}))
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())
    }

    /// All records the backend holds.
    pub fn list(&self) -> Result<Vec<Invite>, String> {
        self.client
            .get(&format!("{}/v0/invites", self.base_url))
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())
    }

    /// Validate a code (the future login path): marks it redeemed.
    /// Unknown codes fail loud via the backend's 404.
    pub fn redeem(&self, code: &str) -> Result<Invite, String> {
        self.client
            .post(&format!("{}/v0/invites/redeem", self.base_url))
            .json(&serde_json::json!({"code": code}))
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())
    }
}

/// Unwrap the Minos envelope `{status_code, message, data}`: non-2xx
/// becomes a classified [`BackendError`] (401/403 by variant, the rest
/// with status), success yields `data`. Shared with the submodules.
pub(crate) fn unwrap_envelope(
    resp: reqwest::blocking::Response,
) -> Result<serde_json::Value, BackendError> {
    let status = resp.status();
    let body: serde_json::Value = resp.json()?;
    if !status.is_success() {
        return Err(BackendError::http_error(status.as_u16(), &body));
    }
    Ok(body["data"].clone())
}

/// Envelope with its cursor page block (timeline): data plus the
/// metadata the load-more control binds to. Absent metadata reads as
/// a single final page, never an error.
pub(crate) fn unwrap_envelope_page(
    resp: reqwest::blocking::Response,
) -> Result<(serde_json::Value, serde_json::Value), BackendError> {
    let status = resp.status();
    let body: serde_json::Value = resp.json()?;
    if !status.is_success() {
        return Err(BackendError::http_error(status.as_u16(), &body));
    }
    Ok((body["data"].clone(), body["metadata"].clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::track::FixSource;

    /// Manifest parsing is pure JSON work, so it is tested against
    /// the envelope shape without a server: dimensions present,
    /// dimensions absent, and the coverage counter.
    #[test]
    fn image_manifest_reads_pixel_dimensions_and_coverage() {
        let data = serde_json::json!({
            "content_changed_at": "2026-09-24T10:00:00Z",
            "etag": "\"asset-v1\"",
            "entry_count": 2,
            "units_without_image_count": 7,
            "entries": [
                {
                    "asset_kind": "unit_image",
                    "id_unit": 6006,
                    "name": "KRI Ahmad Yani",
                    "hull_number": "381",
                    "file_name": "images/6006.png",
                    "content_type": "image/png",
                    "size_bytes": 4096,
                    "width_px": 512,
                    "height_px": 256,
                    "loa_m": 120.5,
                    "beam_m": 16.2
                },
                {
                    "asset_kind": "unit_image",
                    "id_unit": 6020,
                    "name": "KRI 상담",
                    "file_name": "images/6020.jpg",
                    "content_type": "image/jpeg",
                    "size_bytes": 64
                }
            ]
        });
        let m = master::parse_image_manifest(&data);
        assert_eq!(m.version, "\"asset-v1\"", "ETag is the content identity");
        assert_eq!(m.entry_count, 2);
        assert_eq!(m.units_without_image, 7);
        assert_eq!(m.entries.len(), 2);
        let sized = m.entries.iter().find(|e| e.unit_id == 6006).expect("6006");
        assert_eq!(sized.asset_kind, "unit_image");
        assert_eq!(sized.width_px, Some(512));
        assert_eq!(sized.height_px, Some(256));
        assert_eq!(sized.loa_m, Some(120.5));
        assert_eq!(sized.beam_m, Some(16.2));
        // Absent is preserved as absent — never read as zero, which
        // would make a 0×0 image look measured rather than unknown.
        let bare = m.entries.iter().find(|e| e.unit_id == 6020).expect("6020");
        assert_eq!(bare.width_px, None);
        assert_eq!(bare.height_px, None);
        assert_eq!(bare.loa_m, None);
        assert_eq!(bare.beam_m, None);
        // A hull with no image is absent from entries, not a null row.
        assert!(m.entries.iter().all(|e| e.unit_id != 0));
    }

    /// The spec parser is the boundary where an unpublished
    /// measurement must stay unpublished.
    #[test]
    fn hull_spec_keeps_measurements_and_preserves_none() {
        let full = serde_json::json!({
            "unit_class": { "id": 5, "name": "Sigma" },
            "current_specification": {
                "version": 2,
                "speed_max_surface_kn": 30.0,
                "speed_cruise_kn": 18.0,
                "range_nm": 3200.0,
                "loa_m": 120.5,
                "beam_m": 16.2,
                "draft_m": 4.1,
                "displacement_standard_t": 3900.0,
                "displacement_full_t": 5200.0,
                "turn_rate_max_deg_s": 2.5
            }
        });
        let s = master::parse_hull_spec(&full, 13).expect("full spec");
        assert_eq!(s.loa_m, Some(120.5));
        assert_eq!(s.beam_m, Some(16.2));
        assert_eq!(s.draft_m, Some(4.1));
        assert_eq!(s.displacement_standard_t, Some(3900.0));
        assert_eq!(s.displacement_full_t, Some(5200.0));
        assert_eq!(s.turn_rate_max_deg_s, Some(2.5));
        // The sim figures are untouched by the map's.
        assert_eq!(s.speed_kn, Some(30.0));

        // A published spec that omits every physical measurement.
        let bare = serde_json::json!({
            "unit_class": { "id": 5, "name": "Sigma" },
            "current_specification": { "version": 1, "speed_max_surface_kn": 12.0 }
        });
        let s = master::parse_hull_spec(&bare, 13).expect("bare spec");
        assert_eq!(s.loa_m, None, "an absent loa_m is unknown, not zero");
        assert_eq!(s.beam_m, None);
        assert_eq!(s.draft_m, None);
        assert_eq!(s.displacement_standard_t, None);
        assert_eq!(s.turn_rate_max_deg_s, None);
        assert_eq!(s.speed_kn, Some(12.0), "published speed still parses");

        // No published spec at all is the NoSpec refusal, as before.
        let none = serde_json::json!({ "unit_class": { "id": 5, "name": "Sigma" } });
        assert!(master::parse_hull_spec(&none, 13).is_err());
    }

    #[test]
    fn mock_poll_fetches_mock_backend() {
        let body = r#"[{"ship_id":"a","lat":53.5,"lon":9.9,"ts":"2026-09-12T00:00:00Z"}]"#;
        let server = tiny_http::Server::http("127.0.0.1:18080").expect("bind test port");
        std::thread::spawn(move || {
            for rq in server.incoming_requests().take(1) {
                assert_eq!(rq.url(), "/v0/positions");
                let _ = rq.respond(tiny_http::Response::from_string(body));
            }
        });
        let mut src = MockPoll::new("http://127.0.0.1:18080").expect("client builds");
        let fixes = src.poll().expect("poll succeeds");
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].ship_id, "a");
        assert_eq!(fixes[0].position.latitude, 53.5);
    }

    #[test]
    fn invite_round_trip_against_stub() {
        // Stub speaks the mock identity contract: client codes honored,
        // list mirrors the store, redeem flips the flag, unknowns 404.
        let server = tiny_http::Server::http("127.0.0.1:18081").expect("bind test port");
        std::thread::spawn(move || {
            let mut store: Vec<Invite> = Vec::new();
            for mut rq in server.incoming_requests().take(4) {
                let post = matches!(rq.method(), tiny_http::Method::Post);
                let url = rq.url().to_string();
                if !post && url == "/v0/invites" {
                    let body = serde_json::to_string(&store).expect("serializes");
                    let _ = rq.respond(tiny_http::Response::from_string(body));
                } else if post && url == "/v0/invites" {
                    let mut text = String::new();
                    rq.as_reader()
                        .read_to_string(&mut text)
                        .expect("body reads");
                    let v: serde_json::Value = serde_json::from_str(&text).expect("json body");
                    let rec = Invite {
                        code: v["code"].as_str().unwrap_or("TFG-0000").to_string(),
                        user: v["user"].as_str().unwrap_or("").to_string(),
                        seat: v["seat"].as_str().unwrap_or("").to_string(),
                        redeemed: false,
                    };
                    store.push(rec.clone());
                    let _ = rq.respond(tiny_http::Response::from_string(
                        serde_json::to_string(&rec).expect("serializes"),
                    ));
                } else if post && url == "/v0/invites/redeem" {
                    let mut text = String::new();
                    rq.as_reader()
                        .read_to_string(&mut text)
                        .expect("body reads");
                    let v: serde_json::Value = serde_json::from_str(&text).expect("json body");
                    let code = v["code"].as_str().unwrap_or("");
                    match store.iter_mut().find(|r| r.code == code) {
                        Some(rec) => {
                            rec.redeemed = true;
                            let _ = rq.respond(tiny_http::Response::from_string(
                                serde_json::to_string(rec).expect("serializes"),
                            ));
                        }
                        None => {
                            let _ = rq.respond(tiny_http::Response::empty(404));
                        }
                    }
                } else {
                    let _ = rq.respond(tiny_http::Response::empty(404));
                }
            }
        });
        let client = InviteClient::new("http://127.0.0.1:18081").expect("client builds");
        let rec = client
            .issue("ani", "helm kri-a", "TFG-0007")
            .expect("issue succeeds");
        assert_eq!(rec.code, "TFG-0007");
        assert!(!rec.redeemed);
        let all = client.list().expect("list succeeds");
        assert_eq!(all.len(), 1);
        let done = client.redeem("TFG-0007").expect("redeem succeeds");
        assert!(done.redeemed);
        assert!(
            client.redeem("TFG-9999").is_err(),
            "unknown code fails loud"
        );
    }

    #[test]
    fn minos_login_refresh_and_gate_against_stub() {
        // Stub speaks the Minos auth contract: login demands identifier +
        // return_refresh_token, refresh rotates, me gates on the token.
        let server = tiny_http::Server::http("127.0.0.1:18082").expect("bind test port");
        std::thread::spawn(move || {
            for mut rq in server.incoming_requests().take(4) {
                let mut text = String::new();
                rq.as_reader().read_to_string(&mut text).unwrap_or(0);
                let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let url = rq.url().to_string();
                let resp = if url == "/api/v1/auth/login" {
                    assert_eq!(v["identifier"].as_str(), Some("operator1"));
                    assert_eq!(v["return_refresh_token"].as_bool(), Some(true));
                    tiny_http::Response::from_string(
                        r#"{"status_code":200,"message":"Successfull","data":{"access_token":"AT1","token_type":"Bearer","expires_in":3600,"refresh_token":"RT1"}}"#,
                    )
                } else if url == "/api/v1/auth/refresh" {
                    assert_eq!(v["refresh_token"].as_str(), Some("RT1"));
                    tiny_http::Response::from_string(
                        r#"{"status_code":200,"message":"Successfull","data":{"access_token":"AT2","token_type":"Bearer","expires_in":3600,"refresh_token":"RT2"}}"#,
                    )
                } else if url == "/api/v1/users/me" {
                    tiny_http::Response::from_string(
                        r#"{"status_code":200,"message":"Successfull","data":{"id":7,"username":"operator1","roles":[{"id":2,"name":"Operator"}]}}"#,
                    )
                } else {
                    tiny_http::Response::from_string("").with_status_code(404)
                };
                let _ = rq.respond(resp);
            }
        });
        let auth = MinosAuth::new("http://127.0.0.1:18082/api/v1").expect("client builds");
        let pair = auth.login("operator1", "s3cret!").expect("login succeeds");
        assert_eq!(pair.access_token, "AT1");
        assert_eq!(pair.expires_in, 3600);
        assert_eq!(pair.refresh_token.as_deref(), Some("RT1"));
        let pair2 = auth.refresh("RT1").expect("refresh rotates");
        assert_eq!(pair2.access_token, "AT2");
        assert_eq!(pair2.refresh_token.as_deref(), Some("RT2"));
        let identity = auth.me("AT2").expect("gate open");
        assert_eq!(identity.id, 7, "probe carries the caller id for readiness matching");
        assert_eq!(identity.app_role_ids, vec![2], "probe carries application roles");
    }

    #[test]
    fn refusal_detail_surfaces_gate_reasons() {
        // M1: data.errors rides the error — status and top message stay,
        // the gate's own words join them instead of a bare 409.
        let server = tiny_http::Server::http("127.0.0.1:18093").expect("bind test port");
        std::thread::spawn(move || {
            for rq in server.incoming_requests().take(1) {
                assert!(rq.url().contains("/transitions"));
                let _ = rq.respond(
                    tiny_http::Response::from_string(
                        r#"{"status_code":409,"message":"Conflict occurred","data":{"errors":{"_request":"2 hull(s) still need a starting position; Commando Rina Wijaya is not ready"}}}"#,
                    )
                    .with_status_code(409),
                );
            }
        });
        let master = MinosMaster::new("http://127.0.0.1:18093/api/v1").expect("client builds");
        let err = master.transition_game("AT", 3, "execution").unwrap_err();
        let text = err.to_string();
        assert!(text.contains("409"), "status kept: {text}");
        assert!(text.contains("starting position"), "gate detail surfaces: {text}");
        assert!(text.contains("not ready"), "every reason joins: {text}");
        assert!(
            err.detail().is_some_and(|d| d.contains("Rina Wijaya")),
            "structured access preserved"
        );
    }

    #[test]
    fn messages_send_list_read_against_stub() {
        // #99: send answers the stored message, the inbox pages it,
        // receipts come back on the row, roles list for sending-as.
        let server = tiny_http::Server::http("127.0.0.1:18095").expect("bind test port");
        std::thread::spawn(move || {
            for mut rq in server.incoming_requests().take(4) {
                let (method, url) = (rq.method().as_str().to_string(), rq.url().to_string());
                let mut text = String::new();
                rq.as_reader().read_to_string(&mut text).unwrap_or(0);
                let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let (status, body) = if method == "POST" && url.ends_with("/games/3/messages") {
                    assert_eq!(v["kind"].as_str(), Some("telegram"));
                    assert_eq!(v["classification"].as_str(), Some("TERBUKA"));
                    assert!(v.get("to").is_none(), "no audience sent means broadcast");
                    (201, r#"{"status_code":201,"message":"Created","data":{"id":11,"id_game":3,"kind":"telegram","classification":{"code":"TERBUKA","label":"Terbuka"},"sender":{"assumed_role":{"id":2,"name":"Panglima"}},"recipients":[],"broadcast":true,"content":"Mulai latihan","callsign":"","created_at":"2026-11-01T02:00:00Z","assumed_at":"2026-11-01T02:00:00Z"}}"#)
                } else if method == "GET" && url.contains("/games/3/messages") {
                    (200, r#"{"status_code":200,"message":"Successfull","data":[{"id":11,"id_game":3,"kind":"telegram","classification":{"code":"TERBUKA","label":"Terbuka"},"sender":{"assumed_role":{"id":2,"name":"Panglima"}},"recipients":[],"broadcast":true,"content":"Mulai latihan","callsign":"","created_at":"2026-11-01T02:00:00Z","assumed_at":"2026-11-01T02:00:00Z"},{"id":12,"id_game":3,"kind":"administrative","classification":{"code":"RAHASIA","label":"Rahasia"},"sender":{"author":{"id":6,"username":"budi","name":"Budi Santoso"}},"recipients":[{"id_user":7,"kind":"to","read_at":null}],"broadcast":false,"content":"Rapat jam 3","callsign":"KRI-381","created_at":"2026-11-01T02:01:00Z","assumed_at":"2026-11-01T02:01:00Z"}]}"#)
                } else if method == "POST" && url.ends_with("/messages/12/read") {
                    (200, r#"{"status_code":200,"message":"Successfull","data":{"id":12,"id_game":3,"kind":"administrative","classification":{"code":"RAHASIA","label":"Rahasia"},"sender":{"author":{"id":6,"username":"budi","name":"Budi Santoso"}},"recipients":[{"id_user":7,"kind":"to","read_at":"2026-11-01T02:05:00Z"}],"broadcast":false,"content":"Rapat jam 3","callsign":"KRI-381","created_at":"2026-11-01T02:01:00Z","assumed_at":"2026-11-01T02:01:00Z"}}"#)
                } else if method == "GET" && url.ends_with("/scenario-roles") {
                    (200, r#"{"status_code":200,"message":"Successfull","data":[{"id":2,"id_game":3,"name":"Panglima","created_at":"2026-11-01T01:00:00Z"}]}"#)
                } else {
                    (404, r#"{"status_code":404,"message":"Not Found","data":null}"#)
                };
                let resp = if status == 404 {
                    tiny_http::Response::from_string(body).with_status_code(404)
                } else {
                    tiny_http::Response::from_string(body).with_status_code(status)
                };
                let _ = rq.respond(resp);
            }
        });
        let master = MinosMaster::new("http://127.0.0.1:18095/api/v1").expect("client builds");
        let draft = MsgDraft {
            kind: "telegram".into(),
            classification: "TERBUKA".into(),
            content: "Mulai latihan".into(),
            degree: 1,
            ..Default::default()
        };
        let sent = master.send_message("AT", 3, &draft).expect("send");
        assert_eq!(sent.sender, "Panglima");
        assert!(sent.broadcast);
        let inbox = master.inbox_list("AT", 3, None, false).expect("inbox");
        assert_eq!(inbox.len(), 2);
        assert_eq!(inbox[1].sender, "Budi Santoso");
        assert!(inbox[1].recipients.iter().all(|r| r.read_at.is_none()));
        let read = master.mark_read("AT", 3, 12).expect("receipt");
        assert!(read.recipients.iter().all(|r| r.read_at.is_some()));
        let roles = master.scenario_roles("AT", 3).expect("roles");
        assert_eq!(roles, vec![ScenarioRole { id: 2, name: "Panglima".into() }]);
    }

    #[test]
    fn token_pair_parses_strictly() {
        // M2: token_type and lifetime are enforced, not defaulted; the
        // courtesy flag parses; a missing token is a decode error.
        let server = tiny_http::Server::http("127.0.0.1:18094").expect("bind test port");
        std::thread::spawn(move || {
            for mut rq in server.incoming_requests().take(4) {
                let mut text = String::new();
                rq.as_reader().read_to_string(&mut text).unwrap_or(0);
                let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let body = match v["identifier"].as_str() {
                    Some("full") => r#"{"status_code":200,"message":"Successfull","data":{"access_token":"AT","token_type":"Bearer","expires_in":3600,"refresh_token":"RT","must_change_password":true}}"#,
                    Some("notoken") => r#"{"status_code":200,"message":"Successfull","data":{"token_type":"Bearer","expires_in":3600}}"#,
                    Some("badscheme") => r#"{"status_code":200,"message":"Successfull","data":{"access_token":"AT","token_type":"MAC","expires_in":3600}}"#,
                    _ => r#"{"status_code":200,"message":"Successfull","data":{"access_token":"AT","token_type":"Bearer"}}"#,
                };
                let _ = rq.respond(tiny_http::Response::from_string(body));
            }
        });
        let auth = MinosAuth::new("http://127.0.0.1:18094/api/v1").expect("client builds");
        let pair = auth.login("full", "s3cret!").expect("full pair parses");
        assert_eq!(pair.token_type, "Bearer");
        assert!(pair.must_change_password, "courtesy flag routes to change-password");
        assert!(auth.login("notoken", "x").is_err(), "missing token is a decode error");
        assert!(auth.login("badscheme", "x").is_err(), "non-Bearer scheme refused");
        assert!(auth.login("notime", "x").is_err(), "missing lifetime refused, never zero");
    }

    #[test]
    fn minos_logout_revokes_204() {
        // H9: logout presents the refresh token as JSON and answers 204
        // with no body — whatever happened, so it cannot probe tokens.
        let server = tiny_http::Server::http("127.0.0.1:18092").expect("bind test port");
        std::thread::spawn(move || {
            for mut rq in server.incoming_requests().take(1) {
                assert_eq!(rq.url(), "/api/v1/auth/logout");
                let mut text = String::new();
                rq.as_reader().read_to_string(&mut text).unwrap_or(0);
                let v: serde_json::Value = serde_json::from_str(&text).expect("json body");
                assert_eq!(v["refresh_token"].as_str(), Some("RT1"));
                let _ = rq.respond(tiny_http::Response::empty(204));
            }
        });
        let auth = MinosAuth::new("http://127.0.0.1:18092/api/v1").expect("client builds");
        auth.logout(Some("RT1")).expect("logout succeeds");
    }

    #[test]
    fn minos_login_refusal_and_gate_closed() {
        // 401 envelope on bad credentials; 403 from me() means the gate
        // stands (not Active / must_change_password).
        let server = tiny_http::Server::http("127.0.0.1:18083").expect("bind test port");
        std::thread::spawn(move || {
            for rq in server.incoming_requests().take(2) {
                let url = rq.url().to_string();
                let resp = if url == "/api/v1/auth/login" {
                    tiny_http::Response::from_string(
                        r#"{"status_code":401,"message":"Unauthorized","data":null}"#,
                    )
                    .with_status_code(401)
                } else {
                    tiny_http::Response::from_string(
                        r#"{"status_code":403,"message":"Forbidden","data":null}"#,
                    )
                    .with_status_code(403)
                };
                let _ = rq.respond(resp);
            }
        });
        let auth = MinosAuth::new("http://127.0.0.1:18083/api/v1").expect("client builds");
        let err = auth.login("nobody", "wrong").unwrap_err();
        assert!(matches!(err, BackendError::Unauthorized { .. }), "{err}");
        let err = auth.me("STALE").unwrap_err();
        assert!(matches!(err, BackendError::Forbidden { .. }), "{err}");
    }

    #[test]
    fn minos_change_password_204() {
        let server = tiny_http::Server::http("127.0.0.1:18084").expect("bind test port");
        std::thread::spawn(move || {
            for mut rq in server.incoming_requests().take(1) {
                assert_eq!(rq.url(), "/api/v1/users/me/password");
                let mut text = String::new();
                rq.as_reader()
                    .read_to_string(&mut text)
                    .expect("body reads");
                let v: serde_json::Value = serde_json::from_str(&text).expect("json body");
                assert!(v["current_password"].is_string());
                assert!(v["new_password"].is_string());
                let _ = rq.respond(tiny_http::Response::empty(204));
            }
        });
        let auth = MinosAuth::new("http://127.0.0.1:18084/api/v1").expect("client builds");
        auth.change_password("AT", "old-pw", "new-pw-12-chars")
            .expect("change succeeds");
    }

    #[test]
    fn hull_spec_parses_detail_and_refuses_unpublished() {
        let server = tiny_http::Server::http("127.0.0.1:18086").expect("bind test port");
        std::thread::spawn(move || {
            for rq in server.incoming_requests().take(2) {
                let url = rq.url().to_string();
                let body = if url == "/api/v1/units/13" {
                    r#"{"status_code":200,"message":"Successfull","data":{"id":13,"name":"KRI Ahmad Yani","unit_class":{"id":5,"name":"Sigma"},"current_specification":{"version":3,"is_current":true,"speed_max_surface_kn":28.5,"speed_cruise_kn":18.0,"range_nm":4000.0}}}"#
                } else {
                    r#"{"status_code":200,"message":"Successfull","data":{"id":14,"name":"Bare Hull"}}"#
                };
                let _ = rq.respond(tiny_http::Response::from_string(body));
            }
        });
        let master = MinosMaster::new("http://127.0.0.1:18086/api/v1").expect("client builds");
        let spec = master.hull_spec("AT", 13).expect("spec parses");
        assert_eq!(spec.version, 3);
        assert_eq!(spec.class_id, 5);
        assert_eq!(spec.class_name, "Sigma");
        assert_eq!(spec.speed_kn, Some(28.5));
        let err = master.hull_spec("AT", 14).unwrap_err();
        assert!(matches!(err, BackendError::NoSpec { unit_id: 14 }), "{err}");
    }

    #[test]
    fn branch_categories_return_ids_and_empty_is_real() {
        let server = tiny_http::Server::http("127.0.0.1:18087").expect("bind test port");
        std::thread::spawn(move || {
            for rq in server.incoming_requests().take(2) {
                let url = rq.url().to_string();
                let body = if url.contains("id_service_branch=1") {
                    r#"{"status_code":200,"message":"Successfull","data":[{"id":2,"name":"Frigate","type_count":2},{"id":0,"name":"Ghost"}]}"#
                } else {
                    r#"{"status_code":200,"message":"Successfull","data":[]}"#
                };
                let _ = rq.respond(tiny_http::Response::from_string(body));
            }
        });
        let master = MinosMaster::new("http://127.0.0.1:18087/api/v1").expect("client builds");
        assert_eq!(
            master.category_ids_for_branch("AT", 1).expect("ids"),
            vec![2]
        );
        assert!(
            master
                .category_ids_for_branch("AT", 3)
                .expect("empty")
                .is_empty()
        );
    }

    #[test]
    fn session_users_list_search_add_and_command() {
        let server = tiny_http::Server::http("127.0.0.1:18088").expect("bind test port");
        std::thread::spawn(move || {
            for rq in server.incoming_requests().take(8) {
                let (method, url) = (rq.method().as_str().to_string(), rq.url().to_string());
                let body = if method == "GET"
                    && url.contains("/users")
                    && url.contains("page_number=1")
                    && url.contains("search=bud")
                    && url.contains("id_user_status=9")
                    && url.contains("id_app_role=7")
                {
                    r#"{"status_code":200,"message":"Successfull","data":[{"id":6,"username":"budi","name":"Budi Santoso","pangkat":"Serda","satuan":"KRI Sigma","jabatan":"Nakhoda","status":{"id":9,"name":"Active","id_name":"Aktif"}}]}"#
                } else if method == "GET" && url.ends_with("/games/3") {
                    r#"{"status_code":200,"message":"Successfull","data":{"id":3,"name":"Operasi Batu Malang","mode":"maneuver","state":"preparation","time_factor":2.0}}"#
                } else if method == "GET" {
                    r#"{"status_code":200,"message":"Successfull","data":[]}"#
                } else if url.contains("/transitions") {
                    r#"{"status_code":200,"message":"Successfull","data":{"id":3,"name":"Operasi Batu Malang","mode":"maneuver","state":"preparation"}}"#
                } else if method == "POST" && url.ends_with("/games") {
                    r#"{"status_code":201,"message":"Created","data":{"id":3,"name":"Operasi Batu Malang","mode":"maneuver","state":"planning"}}"#
                } else if method == "DELETE" {
                    r#"{"status_code":200,"message":"Successfull","data":[]}"#
                } else if method == "POST" && url.contains("/units") {
                    r#"{"status_code":200,"message":"Successfull","data":[{"id_unit":13,"unit_name":"KRI Ahmad Yani","hull_number":"KRI-AH-YN","id_commander":6,"commander_name":"Budi Santoso"}]}"#
                } else if method == "POST" {
                    r#"{"status_code":201,"message":"Created","data":[{"id_user":6,"user_name":"Budi Santoso","id_game_role":1,"role_name":"Commando","is_judge_side":false,"is_ready":false},{"id_user":7,"user_name":"Rina Wijaya","id_game_role":4,"role_name":"Referee","is_judge_side":true,"is_ready":false}]}"#
                } else {
                    r#"{"status_code":200,"message":"Successfull","data":[{"id_unit":13,"unit_name":"KRI Ahmad Yani","hull_number":"KRI-AH-YN","id_commander":6,"commander_name":"Budi Santoso"}]}"#
                };
                let _ = rq.respond(tiny_http::Response::from_string(body));
            }
        });
        let master = MinosMaster::new("http://127.0.0.1:18088/api/v1").expect("client builds");
        // The filters reach the wire: the row only parses when every
        // query param above matched.
        let users = master.users_list("AT", "bud", Some(9), Some(7)).expect("directory");
        assert_eq!(
            users,
            vec![BackendUser {
                id: 6,
                username: "budi".into(),
                name: "Budi Santoso".into(),
                nrp: String::new(),
                pangkat: "Serda".into(),
                satuan: "KRI Sigma".into(),
                jabatan: "Nakhoda".into(),
                status_id: Some(9),
                status_name: "Active".into(),
            }]
        );
        let seated = master.add_participant("AT", 3, 6, 1).expect("seat");
        assert_eq!(seated.len(), 2, "seat answers with the whole roster");
        assert!(!seated[0].ready, "fresh seat is never ready");
        // The judge flag has to parse: the UI's commander exclusions
        // hang off it, and a silent false would let a Referee in.
        assert!(!seated[0].judge, "exercise side reads as non-judge");
        assert!(seated[1].judge, "judge-side row parses as judge");
        assert_eq!(seated[1].role_name, "Referee");
        let units = master.set_unit_commander("AT", 3, 13, 6).expect("command");
        assert_eq!(units[0].commander_id, Some(6), "command answers with units");
        assert_eq!(units[0].hull_number, "KRI-AH-YN");
        // Game lifecycle for the setup flow: create, assign, remove,
        // advance — every write answers with the collection it changed.
        let game = master
            .create_game("AT", "Operasi Batu Malang", "", "", "", "", "")
            .expect("create");
        assert_eq!(game.id, 3);
        assert_eq!(game.state, "planning", "games are born in planning");
        let pieces = master.assign_unit("AT", 3, 13, 6).expect("assign");
        assert_eq!(pieces[0].commander_id, Some(6), "assign answers with units");
        let pieces = master.remove_unit("AT", 3, 13).expect("remove");
        assert!(pieces.is_empty(), "remove answers with the emptied units");
        let moved = master.transition_game("AT", 3, "preparation").expect("advance");
        assert_eq!(moved.state, "preparation");
        // H1: the held game's stage derives from the detail read, so a
        // transition another client made is visible on select/refresh.
        let detail = master.game_detail("AT", 3).expect("detail");
        assert_eq!(detail.state, "preparation");
        assert_eq!(detail.mode, "maneuver");
        assert_eq!(detail.name, "Operasi Batu Malang");
        // H2: the anchor rides the detail, so mid-exercise selects learn
        // the clock without moving it.
        assert_eq!(detail.time_factor, 2.0);
        assert!(detail.actual_start.is_none(), "preparation has no anchor yet");
    }

    #[test]
    fn execution_gate_writes_against_stub() {
        // C2: placements, readiness, and join speak the game contract —
        // PUT placement answers the setup view, readiness/join answer
        // the game plus the caller's own row.
        let server = tiny_http::Server::http("127.0.0.1:18089").expect("bind test port");
        std::thread::spawn(move || {
            for mut rq in server.incoming_requests().take(4) {
                let (method, url) = (rq.method().as_str().to_string(), rq.url().to_string());
                let mut text = String::new();
                rq.as_reader().read_to_string(&mut text).unwrap_or(0);
                let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let body = if method == "PUT" && url.ends_with("/units/13/placement") {
                    assert_eq!(v["latitude"].as_f64(), Some(-6.0888));
                    assert_eq!(v["longitude"].as_f64(), Some(106.9111));
                    r#"{"status_code":200,"message":"Successfull","data":{"placements":[{"id_unit":13,"latitude":-6.0888,"longitude":106.9111}],"placed":1,"unplaced":2,"ready":false}}"#
                } else if method == "GET" && url.ends_with("/placements") {
                    r#"{"status_code":200,"message":"Successfull","data":{"placements":[{"id_unit":13,"latitude":-6.0888,"longitude":106.9111}],"placed":1,"unplaced":0,"ready":true}}"#
                } else if method == "PUT" && url.ends_with("/readiness") {
                    r#"{"status_code":200,"message":"Successfull","data":{"game":{"id":3,"name":"Operasi Batu Malang","mode":"maneuver","state":"preparation"},"participant":{"id_user":6,"user_name":"Budi Santoso","id_game_role":1,"role_name":"Commando","is_judge_side":false,"is_ready":true},"commanded_units":[]}}"#
                } else if method == "POST" && url.ends_with("/games/join") {
                    assert_eq!(v["room_key"].as_str(), Some("RUNG-7X2"));
                    r#"{"status_code":200,"message":"Successfull","data":{"game":{"id":3,"name":"Operasi Batu Malang","mode":"maneuver","state":"preparation"},"participant":{"id_user":6,"user_name":"Budi Santoso","id_game_role":1,"role_name":"Commando","is_judge_side":false,"is_ready":false},"commanded_units":[]}}"#
                } else {
                    r#"{"status_code":404,"message":"Not Found","data":null}"#
                };
                let resp = if body.contains("\"status_code\":404") {
                    tiny_http::Response::from_string(body).with_status_code(404)
                } else {
                    tiny_http::Response::from_string(body)
                };
                let _ = rq.respond(resp);
            }
        });
        let master = MinosMaster::new("http://127.0.0.1:18089/api/v1").expect("client builds");
        let view = master
            .set_placement("AT", 3, 13, -6.0888, 106.9111)
            .expect("place");
        assert_eq!(view.placed, 1);
        assert_eq!(view.unplaced, 2);
        assert!(!view.ready, "two hulls still need a position");
        assert_eq!(view.placements[0].unit_id, 13);
        let view = master.placements_list("AT", 3).expect("list");
        assert!(view.ready, "server arithmetic decides readiness, never the client");
        assert_eq!(view.unplaced, 0);
        let j = master.set_readiness("AT", 3, true).expect("declare");
        assert_eq!(j.game_id, 3);
        assert_eq!(j.game_state, "preparation");
        assert_eq!(j.user_id, 6);
        assert!(j.ready, "answer carries the row the database decided");
        assert!(!j.judge);
        let j = master.join_game("AT", "RUNG-7X2").expect("join");
        assert_eq!(j.game_name, "Operasi Batu Malang");
        assert!(!j.ready, "fresh join has not declared yet");
    }

    #[test]
    fn authoritative_orders_and_plot_against_stub() {
        // C3: orders carry heading + speed only (no position, no time —
        // both are the server's); the answer is the fix the order closed
        // at, and the plot echoes the instant it answered for.
        let server = tiny_http::Server::http("127.0.0.1:18090").expect("bind test port");
        std::thread::spawn(move || {
            for mut rq in server.incoming_requests().take(2) {
                let (method, url) = (rq.method().as_str().to_string(), rq.url().to_string());
                let mut text = String::new();
                rq.as_reader().read_to_string(&mut text).unwrap_or(0);
                let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let (status, body) = if method == "POST" && url.ends_with("/units/13/order") {
                    assert_eq!(v["heading_deg"].as_f64(), Some(45.0));
                    assert_eq!(v["speed_kn"].as_f64(), Some(20.0));
                    assert!(v.get("latitude").is_none(), "orders carry no position");
                    (201, r#"{"status_code":201,"message":"Created","data":{"id_unit":13,"assumed_time":"2026-11-01T01:00:00Z","latitude":-6.0888,"longitude":106.9111,"heading_deg":45.0,"speed_kn":12.0,"requested_speed_kn":20.0,"clamped":true,"created_at":"2026-11-01T01:00:00Z","created_by":6}}"#)
                } else if method == "GET" && url.ends_with("/positions") {
                    (200, r#"{"status_code":200,"message":"Successfull","data":{"assumed_time":"2026-11-01T01:05:00Z","positions":[{"id_unit":13,"latitude":-6.08,"longitude":106.92,"heading_deg":45.0,"speed_kn":12.0,"clamped":false,"assumed_time":"2026-11-01T01:00:00Z"}]}}"#)
                } else {
                    (404, r#"{"status_code":404,"message":"Not Found","data":null}"#)
                };
                let resp = if status == 200 || status == 201 {
                    tiny_http::Response::from_string(body).with_status_code(status)
                } else {
                    tiny_http::Response::from_string(body).with_status_code(404)
                };
                let _ = rq.respond(resp);
            }
        });
        let master = MinosMaster::new("http://127.0.0.1:18090/api/v1").expect("client builds");
        let fix = master.order_unit("AT", 3, 13, 45.0, 20.0).expect("order");
        assert_eq!(fix.unit_id, 13);
        assert!(fix.clamped, "20 kn exceeded the hull's published max");
        assert_eq!(fix.requested_speed, Some(20.0), "ask preserved beside the clamp");
        assert_eq!(fix.speed, 12.0, "applied speed is the clamped one");
        assert_eq!(fix.assumed_time, "2026-11-01T01:00:00Z");
        let plot = master.positions("AT", 3, None, None).expect("plot");
        assert_eq!(plot.assumed_time, "2026-11-01T01:05:00Z", "instant echoed back");
        assert_eq!(plot.positions.len(), 1);
        assert_eq!(plot.positions[0].heading, 45.0);
    }

    #[test]
    fn scenario_clock_writes_against_stub() {
        // H2: pause appends the zero segment and closes orders while the
        // chosen factor is kept; resume reopens at that rate; factor
        // writes the rate (no upper bound) and answers the whole clock.
        // Every write IS the read — there is no clock GET.
        let server = tiny_http::Server::http("127.0.0.1:18091").expect("bind test port");
        std::thread::spawn(move || {
            for mut rq in server.incoming_requests().take(3) {
                let (method, url) = (rq.method().as_str().to_string(), rq.url().to_string());
                let mut text = String::new();
                rq.as_reader().read_to_string(&mut text).unwrap_or(0);
                let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let body = if method == "POST" && url.ends_with("/games/3/pause") {
                    assert!(v.is_null(), "pause is bodiless");
                    r#"{"status_code":200,"message":"Successfull","data":{"state":"execution","actual_start":"2026-11-01T00:00:00Z","assumed_start":"2026-11-01T00:00:00Z","assumed_now":"2026-11-01T02:00:00Z","time_factor":2.0,"accepting_actions":false,"running":false,"segments":[{"factor":2.0,"effective_from":"2026-11-01T00:00:00Z","source":"execution"},{"factor":0.0,"effective_from":"2026-11-01T02:00:00Z","source":"pause"}]}}"#
                } else if method == "POST" && url.ends_with("/games/3/resume") {
                    r#"{"status_code":200,"message":"Successfull","data":{"state":"execution","actual_start":"2026-11-01T00:00:00Z","assumed_start":"2026-11-01T00:00:00Z","assumed_now":"2026-11-01T02:30:00Z","time_factor":2.0,"accepting_actions":true,"running":true,"segments":[{"factor":2.0,"effective_from":"2026-11-01T00:00:00Z","source":"execution"},{"factor":0.0,"effective_from":"2026-11-01T02:00:00Z","source":"pause"},{"factor":2.0,"effective_from":"2026-11-01T02:30:00Z","source":"resume"}]}}"#
                } else if method == "PATCH" && url.ends_with("/games/3/time-factor") {
                    assert_eq!(v["time_factor"].as_f64(), Some(6.0));
                    r#"{"status_code":200,"message":"Successfull","data":{"state":"execution","actual_start":"2026-11-01T00:00:00Z","assumed_start":"2026-11-01T00:00:00Z","assumed_now":"2026-11-01T03:00:00Z","time_factor":6.0,"accepting_actions":true,"running":true,"segments":[{"factor":6.0,"effective_from":"2026-11-01T03:00:00Z","source":"gm"}]}}"#
                } else {
                    r#"{"status_code":404,"message":"Not Found","data":null}"#
                };
                let resp = if body.contains("\"status_code\":404") {
                    tiny_http::Response::from_string(body).with_status_code(404)
                } else {
                    tiny_http::Response::from_string(body)
                };
                let _ = rq.respond(resp);
            }
        });
        let master = MinosMaster::new("http://127.0.0.1:18091/api/v1").expect("client builds");
        let held = master.pause_game("AT", 3).expect("pause");
        assert!(!held.running, "last segment is the zero one");
        assert!(!held.accepting_actions, "pause closes the exercise to orders");
        assert_eq!(held.time_factor, 2.0, "chosen rate kept for resume");
        assert_eq!(held.segments.last().map(|s| s.source.as_str()), Some("pause"));
        let live = master.resume_game("AT", 3).expect("resume");
        assert!(live.running && live.accepting_actions);
        assert_eq!(live.time_factor, 2.0, "resume restarts at the chosen rate");
        let fast = master.set_time_factor("AT", 3, 6.0).expect("factor");
        assert_eq!(fast.time_factor, 6.0);
        assert!(fast.assumed_now.is_some(), "writes echo the instant they built for");
    }

    #[test]
    fn feed_event_maps_section_five_to_fix() {
        // §5 publication: decimal unit id, labels, recorded vs received
        // time, omitted-never-zero optionals, backfilled flag.
        let ev: FeedEvent = serde_json::from_str(
            r#"{"id_unit":13,"name":"KRI Ahmad Yani","hull_number":"KRI-AH-YN","latitude":-6.0888,"longitude":106.9111,"speed_kn":14.2,"course_deg":87.5,"recorded_at":"2026-09-15T09:56:28Z","received_at":"2026-09-15T09:56:29Z","backfilled":false}"#,
        )
        .expect("§5 parses");
        assert!(ev.accuracy_m.is_none(), "absent stays absent, never zero");
        let f = ev.to_fix();
        assert_eq!(f.ship_id, "13");
        assert_eq!(f.ts, "2026-09-15T09:56:28Z");
        assert_eq!(f.received_at.as_deref(), Some("2026-09-15T09:56:29Z"));
        assert_eq!(f.heading_deg, Some(87.5));
        assert_eq!(f.speed_kn, Some(14.2));
        assert_eq!(f.name.as_deref(), Some("KRI Ahmad Yani"));
        assert!(!f.backfilled);
        assert_eq!(f.source, FixSource::Wire);
    }

    #[test]
    fn snapshot_splits_silent_and_fixes() {
        // Standing picture: one reporting vessel (backfilled flush) and
        // one silent vessel (labels, no position key at all).
        let server = tiny_http::Server::http("127.0.0.1:18085").expect("bind test port");
        std::thread::spawn(move || {
            for rq in server.incoming_requests().take(1) {
                assert_eq!(rq.url(), "/api/v1/live-feed/positions");
                let _ = rq.respond(tiny_http::Response::from_string(
                    r#"{"status_code":200,"message":"Successfull","data":[
                        {"id_unit":13,"name":"KRI Ahmad Yani","hull_number":"KRI-AH-YN","position":{"latitude":-6.08,"longitude":106.91,"recorded_at":"2026-09-10T00:00:00Z","received_at":"2026-09-15T09:56:29Z","age_seconds":492301.0,"backfilled":true}},
                        {"id_unit":14,"name":"KRI Ahmad Yani II","hull_number":null}
                    ]}"#,
                ));
            }
        });
        let rest = MinosRest::new("http://127.0.0.1:18085/api/v1").expect("client builds");
        let snap = rest.snapshot("AT").expect("snapshot succeeds");
        assert_eq!(snap.announced.len(), 2);
        assert_eq!(snap.fixes.len(), 1);
        let f = &snap.fixes[0];
        assert_eq!(f.ship_id, "13");
        assert!(f.backfilled);
        assert_eq!(f.received_at.as_deref(), Some("2026-09-15T09:56:29Z"));
        // M4: the server-stated age rides the snapshot fix.
        assert_eq!(f.age_secs, Some(492301.0));
        assert_eq!(f.data_age_secs(1_700_000_000), Some(492301));
    }

    #[test]
    fn replay_loop_stays_fresh_past_wrap() {
        use crate::geo::track::Registry;
        let mut src = FileReplay::from_file("tests/fixtures/tracks.json").expect("fixture loads");
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
        let mut src = FileReplay::from_file("tests/fixtures/tracks.json").expect("fixture loads");
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
