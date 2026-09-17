//! Backend polling (v0 contract, see resolution on the backend ticket).
//!
//! - [`PollSource`]: one poll round -> the fixes seen this round.
//! - [`FileReplay`]: dev default. Replays canned frames from a JSON fixture
//!   (`tests/fixtures/tracks.json`), looping. No network, deterministic.
//! - [`HttpPoll`]: real backend. Not wired yet — returns an error until the
//!   backend exists; swapping impls is one line at the call site.

use std::fs;

use serde::{Deserialize, Serialize};

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
/// redeem. Same contract style as [`HttpPoll`]: base URL + blocking client.
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
        Ok(Self { base_url: base_url.trim_end_matches('/').to_string(), client })
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

/// Minos token pair (docs/minos-api.yaml): short-lived access token plus
/// the desktop refresh path. `refresh_token` is present only when the
/// login asked for it (`return_refresh_token`), which native clients must.
#[derive(Debug, Clone)]
pub struct TokenPair {
    pub access_token: String,
    pub expires_in: u64,
    pub refresh_token: Option<String>,
}

/// Minos auth (auth grill resolution): identifier login, rotation refresh,
/// password-change gate probe. Blocking client like the other backends.
/// Tokens live with the caller — access in memory, refresh in the keyring.
pub struct MinosAuth {
    base_url: String,
    client: reqwest::blocking::Client,
}

/// Unwrap the Minos envelope `{status_code, message, data}`: non-2xx
/// becomes the server's message, success yields `data`.
fn unwrap_envelope(resp: reqwest::blocking::Response) -> Result<serde_json::Value, String> {
    let status = resp.status();
    let body: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!(
            "HTTP {status}: {}",
            body["message"].as_str().unwrap_or("request failed")
        ));
    }
    Ok(body["data"].clone())
}

impl MinosAuth {
    pub fn new(base_url: &str) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { base_url: base_url.trim_end_matches('/').to_string(), client })
    }

    /// Sign in. Desktop path: `return_refresh_token` true (no cookie jar).
    /// Unknown user and wrong password are an identical 401 by design.
    pub fn login(&self, identifier: &str, password: &str) -> Result<TokenPair, String> {
        let data = unwrap_envelope(
            self.client
                .post(&format!("{}/auth/login", self.base_url))
                .json(&serde_json::json!({
                    "identifier": identifier,
                    "password": password,
                    "return_refresh_token": true,
                }))
                .send()
                .map_err(|e| e.to_string())?,
        )?;
        Ok(TokenPair {
            access_token: data["access_token"].as_str().unwrap_or("").to_string(),
            expires_in: data["expires_in"].as_u64().unwrap_or(0),
            refresh_token: data["refresh_token"].as_str().map(|s| s.to_string()),
        })
    }

    /// Rotate: presents the refresh token, returns a new pair. The old
    /// refresh token is invalidated server-side (replay is rejected).
    pub fn refresh(&self, refresh_token: &str) -> Result<TokenPair, String> {
        let data = unwrap_envelope(
            self.client
                .post(&format!("{}/auth/refresh", self.base_url))
                .json(&serde_json::json!({ "refresh_token": refresh_token }))
                .send()
                .map_err(|e| e.to_string())?,
        )?;
        Ok(TokenPair {
            access_token: data["access_token"].as_str().unwrap_or("").to_string(),
            expires_in: data["expires_in"].as_u64().unwrap_or(0),
            refresh_token: data["refresh_token"].as_str().map(|s| s.to_string()),
        })
    }

    /// Change the account password (the must_change_password gate). 204 on
    /// success; every other refresh token for the account is invalidated,
    /// so the caller re-logins afterwards. Passwords go byte for byte.
    pub fn change_password(
        &self,
        access_token: &str,
        current_password: &str,
        new_password: &str,
    ) -> Result<(), String> {
        self.client
            .put(&format!("{}/users/me/password", self.base_url))
            .bearer_auth(access_token)
            .json(&serde_json::json!({
                "current_password": current_password,
                "new_password": new_password,
            }))
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Gate probe: who am I with this token. Ok means the token works and
    /// no gate stands in the way; a 403 with valid credentials means the
    /// account is not Active (or must_change_password still shuts doors).
    pub fn me(&self, access_token: &str) -> Result<(), String> {
        self.client
            .get(&format!("{}/users/me", self.base_url))
            .bearer_auth(access_token)
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Refresh-token store (keyring resolution): OS keyring entry per
/// identifier, single writer on the UI thread. `None` on load means no
/// entry (fresh login); any other store failure degrades to memory with
/// a visible degraded status — never silently.
const KEYRING_SERVICE: &str = "tfg-minos";

/// Save (overwrite) the refresh token for this identifier.
pub fn keyring_save(user: &str, token: &str) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, user)
        .map_err(|e| e.to_string())?
        .set_password(token)
        .map_err(|e| e.to_string())
}

/// Load the stored refresh token, if any.
pub fn keyring_load(user: &str) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, user).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(pw) => Ok(Some(pw)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Forget the stored refresh token (sign out). Missing entry is fine.
pub fn keyring_clear(user: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, user).map_err(|e| e.to_string())?;
    match entry.delete_password() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
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
                    use std::io::Read;
                    rq.as_reader().read_to_string(&mut text).expect("body reads");
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
                    use std::io::Read;
                    rq.as_reader().read_to_string(&mut text).expect("body reads");
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
        let rec = client.issue("ani", "helm kri-a", "TFG-0007").expect("issue succeeds");
        assert_eq!(rec.code, "TFG-0007");
        assert!(!rec.redeemed);
        let all = client.list().expect("list succeeds");
        assert_eq!(all.len(), 1);
        let done = client.redeem("TFG-0007").expect("redeem succeeds");
        assert!(done.redeemed);
        assert!(client.redeem("TFG-9999").is_err(), "unknown code fails loud");
    }

    #[test]
    fn minos_login_refresh_and_gate_against_stub() {
        // Stub speaks the Minos auth contract: login demands identifier +
        // return_refresh_token, refresh rotates, me gates on the token.
        let server = tiny_http::Server::http("127.0.0.1:18082").expect("bind test port");
        std::thread::spawn(move || {
            for mut rq in server.incoming_requests().take(4) {
                let mut text = String::new();
                use std::io::Read;
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
                        r#"{"status_code":200,"message":"Successfull","data":{"id":7,"username":"operator1"}}"#,
                    )
                } else {
                    tiny_http::Response::empty(404)
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
        auth.me("AT2").expect("gate open");
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
        assert!(err.contains("401"), "{err}");
        let err = auth.me("STALE").unwrap_err();
        assert!(err.contains("403"), "{err}");
    }

    #[test]
    fn minos_change_password_204() {
        let server = tiny_http::Server::http("127.0.0.1:18084").expect("bind test port");
        std::thread::spawn(move || {
            for mut rq in server.incoming_requests().take(1) {
                assert_eq!(rq.url(), "/api/v1/users/me/password");
                let mut text = String::new();
                use std::io::Read;
                rq.as_reader().read_to_string(&mut text).expect("body reads");
                let v: serde_json::Value = serde_json::from_str(&text).expect("json body");
                assert!(v["current_password"].is_string());
                assert!(v["new_password"].is_string());
                let _ = rq.respond(tiny_http::Response::empty(204));
            }
        });
        let auth = MinosAuth::new("http://127.0.0.1:18084/api/v1").expect("client builds");
        auth.change_password("AT", "old-pw", "new-pw-12-chars").expect("change succeeds");
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
