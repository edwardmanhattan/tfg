//! Backend polling (v0 contract, see resolution on the backend ticket).
//!
//! - [`PollSource`]: one poll round -> the fixes seen this round.
//! - [`FileReplay`]: dev default. Replays canned frames from a JSON fixture
//!   (`tests/fixtures/tracks.json`), looping. No network, deterministic.
//! - [`HttpPoll`]: real backend. Not wired yet — returns an error until the
//!   backend exists; swapping impls is one line at the call site.

use std::fs;

use serde::{Deserialize, Serialize};

use crate::geo::track::{Fix, FixSource};

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
        // Receipt time too: replayed frames are fresh on serve, so the
        // old-data badge (wire-gated) stays quiet on fixtures.
        fix.received_at = Some(now.clone());
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
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Minos standing picture (REST mapping ticket): the initial picture the
/// socket then keeps current. Vessels that never reported carry labels
/// but no position — announced as silent, never zero-filled.
#[derive(Debug, Clone, Deserialize)]
struct FeedPosition {
    id_unit: u64,
    name: String,
    hull_number: Option<String>,
    position: Option<FeedFix>,
}

/// One stored fix as the server holds it.
#[derive(Debug, Clone, Deserialize)]
struct FeedFix {
    latitude: f64,
    longitude: f64,
    speed_kn: Option<f32>,
    course_deg: Option<f32>,
    accuracy_m: Option<f32>,
    recorded_at: String,
    received_at: String,
    #[serde(default)]
    backfilled: bool,
}

impl FeedFix {
    fn to_fix(&self, ship_id: String, name: String, hull_number: Option<String>) -> Fix {
        Fix {
            ship_id,
            position: crate::geo::GeoPosition {
                latitude: self.latitude,
                longitude: self.longitude,
            },
            ts: self.recorded_at.clone(),
            received_at: Some(self.received_at.clone()),
            heading_deg: self.course_deg,
            speed_kn: self.speed_kn,
            accuracy_m: self.accuracy_m,
            name: Some(name),
            hull_number,
            backfilled: self.backfilled,
            source: FixSource::Wire,
            seq: 0,
        }
    }
}

/// Snapshot out of one REST read: silent announcements plus latest fixes.
pub struct Snapshot {
    pub announced: Vec<(String, Option<String>, Option<String>)>,
    pub fixes: Vec<Fix>,
}

/// Minos REST reads (REST mapping ticket): snapshot fetch beside the
/// auth client. Same blocking style; the poll thread calls it, the
/// socket actor re-reads it after every reconnect.
pub struct MinosRest {
    base_url: String,
    client: reqwest::blocking::Client,
}

impl MinosRest {
    pub fn new(base_url: &str) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { base_url: base_url.trim_end_matches('/').to_string(), client })
    }

    /// Standing picture: every vessel, ordered by id. Not paginated by
    /// design (a page of a picture is a wrong picture).
    pub fn snapshot(&self, access_token: &str) -> Result<Snapshot, String> {
        let positions: Vec<FeedPosition> = unwrap_envelope(
            self.client
                .get(&format!("{}/live-feed/positions", self.base_url))
                .bearer_auth(access_token)
                .send()
                .map_err(|e| e.to_string())?,
        )
        .and_then(|data| serde_json::from_value(data).map_err(|e| e.to_string()))?;
        let mut announced = Vec::with_capacity(positions.len());
        let mut fixes = Vec::new();
        for p in positions {
            let id = p.id_unit.to_string();
            announced.push((id.clone(), Some(p.name.clone()), p.hull_number.clone()));
            if let Some(pos) = p.position {
                fixes.push(pos.to_fix(id, p.name, p.hull_number));
            }
        }
        Ok(Snapshot { announced, fixes })
    }
}

/// One live-feed publication (§5): one message per beacon batch, per
/// vessel. Optional numerics are absent-never-zero on the wire.
#[derive(Debug, Clone, Deserialize)]
struct FeedEvent {
    id_unit: u64,
    name: Option<String>,
    hull_number: Option<String>,
    latitude: f64,
    longitude: f64,
    speed_kn: Option<f32>,
    course_deg: Option<f32>,
    accuracy_m: Option<f32>,
    recorded_at: String,
    received_at: String,
    #[serde(default)]
    backfilled: bool,
}

impl FeedEvent {
    fn to_fix(&self) -> Fix {
        Fix {
            ship_id: self.id_unit.to_string(),
            position: crate::geo::GeoPosition {
                latitude: self.latitude,
                longitude: self.longitude,
            },
            ts: self.recorded_at.clone(),
            received_at: Some(self.received_at.clone()),
            heading_deg: self.course_deg,
            speed_kn: self.speed_kn,
            accuracy_m: self.accuracy_m,
            name: self.name.clone(),
            hull_number: self.hull_number.clone(),
            backfilled: self.backfilled,
            source: FixSource::Wire,
            seq: 0,
        }
    }
}

/// Backoff for socket retries (transport ticket): base 1 s, cap 30 s,
/// full jitter. Tunable here, applied in the actor loop.
pub const LIVE_BACKOFF_BASE_SECS: u64 = 1;
pub const LIVE_BACKOFF_CAP_SECS: u64 = 30;

/// Full jitter in whole seconds over [0, bound].
fn full_jitter_secs(bound: u64) -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    nanos % (bound + 1)
}

/// Actor -> UI reports. The UI owns refresh (keyring) and sign-out; the
/// actor never touches either.
#[derive(Debug, Clone)]
pub enum LiveEvent {
    Connected { client_id: String },
    Announced(Vec<(String, Option<String>, Option<String>)>),
    Reconnecting { attempt: u32, wait_secs: u64 },
    /// Fatal refusal (101/103/107): the actor stops itself. The UI signs
    /// out (101) or shows the error; the wire goes quiet and stale flags
    /// read the outage as feed-down.
    Refused { code: u32, reason: String },
    /// Token expiry (109): the UI refreshes silently and pushes the new
    /// token back down.
    RefreshDue,
    SocketError(String),
}

/// UI -> actor commands.
#[derive(Debug)]
pub enum LiveCmd {
    SetToken(String),
    Shutdown,
}

struct LiveShared {
    queue: std::sync::Mutex<std::collections::VecDeque<FeedEvent>>,
    /// Last-known picture per ship (snapshot seed + socket overlay).
    /// The poll side replays it every tick; reconnect re-reads reseed it.
    known: std::sync::Mutex<std::collections::HashMap<String, Fix>>,
    live: std::sync::atomic::AtomicBool,
}

/// Tick-batched live wire (transport ticket): socket publications queue
/// on the actor thread; each poll drains newest-per-ship and replays the
/// last-known picture for every known vessel (snapshot + socket overlay),
/// so silence counts as Registry misses and disconnects read as stale.
/// Disconnect yields empty rounds.
pub struct LiveWire {
    shared: std::sync::Arc<LiveShared>,
    cmd_tx: std::sync::mpsc::Sender<LiveCmd>,
}

impl LiveWire {
    /// Snapshot first (announcements + seed picture), then stand up the
    /// actor. Snapshot failure fails the whole connect — the caller falls
    /// back to an empty wire with the reason in its description.
    pub fn connect(
        ws_url: &str,
        rest: &MinosRest,
        token: &str,
        cmd_tx: std::sync::mpsc::Sender<LiveCmd>,
        cmd_rx: std::sync::mpsc::Receiver<LiveCmd>,
        event_tx: std::sync::mpsc::Sender<LiveEvent>,
    ) -> Result<Self, String> {
        let snap = rest.snapshot(token)?;
        let _ = event_tx.send(LiveEvent::Announced(snap.announced));
        let mut known = std::collections::HashMap::new();
        for f in snap.fixes {
            known.insert(f.ship_id.clone(), f);
        }
        let shared = std::sync::Arc::new(LiveShared {
            queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
            known: std::sync::Mutex::new(known),
            live: std::sync::atomic::AtomicBool::new(true),
        });
        let token = token.to_string();
        let ws_url = ws_url.to_string();
        let rest_base = rest.base_url.clone();
        let actor_shared = shared.clone();
        std::thread::spawn(move || {
            run_actor(ws_url, rest_base, token, actor_shared, cmd_rx, event_tx);
        });
        Ok(Self { shared, cmd_tx })
    }

    /// Newest queued event per ship, overlaid on the known picture.
    fn drain_queue(&self) {
        let mut fresh: std::collections::HashMap<String, Fix> = std::collections::HashMap::new();
        if let Ok(mut q) = self.shared.queue.lock() {
            for ev in q.drain(..) {
                let fix = ev.to_fix();
                match fresh.get(&fix.ship_id) {
                    Some(prev) if prev.ts >= fix.ts => {}
                    _ => {
                        fresh.insert(fix.ship_id.clone(), fix);
                    }
                }
            }
        }
        if fresh.is_empty() {
            return;
        }
        if let Ok(mut known) = self.shared.known.lock() {
            for (id, fix) in fresh {
                known.insert(id, fix);
            }
        }
    }
}

impl PollSource for LiveWire {
    fn poll(&mut self) -> Result<Vec<Fix>, String> {
        if !self.shared.live.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(Vec::new()); // actor down: feed-down reads as stale
        }
        self.drain_queue();
        match self.shared.known.lock() {
            Ok(known) => Ok(known.values().cloned().collect()),
            Err(e) => Err(e.to_string()),
        }
    }
}

impl Drop for LiveWire {
    /// Swapping the wire away stops the actor: disconnect, mark down,
    /// end the thread. Best-effort — a dead actor's mailbox is gone.
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(LiveCmd::Shutdown);
    }
}

/// The actor: own thread, own multi-thread runtime (blocking REST re-reads
/// must not stall callbacks), SDK client with JSON protocol. Reconnects
/// pace on our backoff consts; fatal codes stop the loop and mark down.
fn run_actor(
    ws_url: String,
    rest_base: String,
    token: String,
    shared: std::sync::Arc<LiveShared>,
    cmd_rx: std::sync::mpsc::Receiver<LiveCmd>,
    event_tx: std::sync::mpsc::Sender<LiveEvent>,
) {
    use std::sync::atomic::Ordering;
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            let _ = event_tx.send(LiveEvent::SocketError(format!("runtime: {e}")));
            shared.live.store(false, Ordering::SeqCst);
            return;
        }
    };
    rt.block_on(async move {
        use tokio_centrifuge::client::Client;
        use tokio_centrifuge::config::Config;
        use tokio_centrifuge::events::{ConnectedEvent, DisconnectedEvent};
        use tokio_centrifuge::protocol::Publication;
        let client = Client::new(
            &ws_url,
            Config::new().with_token(token.clone()).use_json(),
        );
        let current_token = std::sync::Arc::new(std::sync::Mutex::new(token));
        // One subscription object: publications attach once, reconnects
        // re-arm the same object (idempotent server-side).
        let sub = client.new_subscription("live-feed");
        // Publications: parse §5, queue newest-wins per tick. Personal
        // channel traffic has no client-subscription path here; when the
        // SDK surfaces it, drain it to the terminal, never to the Registry.
        {
            let shared = shared.clone();
            sub.on_publication(move |p: Publication| {
                match serde_json::from_slice::<FeedEvent>(&p.data) {
                    Ok(ev) => {
                        if let Ok(mut q) = shared.queue.lock() {
                            q.push_back(ev);
                        }
                    }
                    Err(e) => eprintln!("live-feed: unparsable publication: {e}"),
                }
            });
            sub.on_error(|e| {
                eprintln!("live-feed subscription error: {e:?}");
            });
        }
        // Connected: report up, reset backoff, re-read the snapshot
        // (reconcile-by-id: no channel history), re-arm the subscription.
        {
            let event_tx = event_tx.clone();
            let shared = shared.clone();
            let rest_base = rest_base.clone();
            let current_token = current_token.clone();
            let resub = sub.clone();
            client.on_connected(move |e: ConnectedEvent<'_>| {
                let _ = event_tx.send(LiveEvent::Connected {
                    client_id: e.client_id.to_string(),
                });
                shared.live.store(true, Ordering::SeqCst);
                if tokio::runtime::Handle::try_current().is_ok() {
                    tokio::task::spawn(async move {
                        let _ = resub.subscribe().await;
                    });
                    // Snapshot re-read off the callback path.
                    let event_tx = event_tx.clone();
                    let shared = shared.clone();
                    let rest_base = rest_base.clone();
                    let current_token = current_token.clone();
                    tokio::task::spawn_blocking(move || {
                        let tok =
                            current_token.lock().map(|t| t.clone()).unwrap_or_default();
                        let Ok(rest) = MinosRest::new(&rest_base) else { return };
                        let Ok(snap) = rest.snapshot(&tok) else { return };
                        let _ = event_tx.send(LiveEvent::Announced(snap.announced));
                        if let Ok(mut known) = shared.known.lock() {
                            for f in snap.fixes {
                                known.insert(f.ship_id.clone(), f);
                            }
                        }
                    });
                }
            });
        }
        // Disconnects: the refusal policy. Fatal codes stop the loop;
        // 109 asks the UI for a silent refresh; the rest back off.
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        {
            let event_tx = event_tx.clone();
            let stop = stop.clone();
            let attempts = attempts.clone();
            let shared = shared.clone();
            client.on_disconnected(move |e: DisconnectedEvent<'_>| {
                shared.live.store(false, Ordering::SeqCst);
                match e.code {
                    101 | 103 | 107 => {
                        let _ = event_tx.send(LiveEvent::Refused {
                            code: e.code,
                            reason: e.reason.to_string(),
                        });
                        stop.store(true, Ordering::SeqCst);
                    }
                    109 => {
                        let _ = event_tx.send(LiveEvent::RefreshDue);
                    }
                    _ => {
                        let n = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                        let wait = (LIVE_BACKOFF_BASE_SECS
                            .saturating_mul(1u64 << n.min(6))
                            .min(LIVE_BACKOFF_CAP_SECS))
                            + full_jitter_secs(LIVE_BACKOFF_CAP_SECS.min(5));
                        let _ = event_tx.send(LiveEvent::Reconnecting {
                            attempt: n,
                            wait_secs: wait,
                        });
                    }
                }
            });
        }
        client.on_error(|e| {
            eprintln!("live socket error: {e:?}");
        });
        // Initial subscribe attempt (pre-connect declaration; the server
        // arms it on handshake, and on_connected re-arms per reconnect).
        {
            let sub = client.new_subscription("live-feed");
            let _ = sub.subscribe().await;
        }
        let _ = client.connect().await;
        // Command + reconnect loop: token push-down applies live via
        // set_token; shutdown/fatal disconnects the client and ends us.
        let mut backoff_wait: Option<u64> = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            match cmd_rx.try_recv() {
                Ok(LiveCmd::SetToken(t)) => {
                    if let Ok(mut cur) = current_token.lock() {
                        *cur = t.clone();
                    }
                    client.set_token(t);
                }
                Ok(LiveCmd::Shutdown) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
            if stop.load(Ordering::SeqCst) {
                break;
            }
            // Retryable disconnect outstanding: pace our explicit
            // re-handshake on our backoff (harmless if the SDK's own
            // reconnect already won the race — the server dedupes).
            if !shared.live.load(Ordering::SeqCst) && !stop.load(Ordering::SeqCst) {
                let wait = backoff_wait.get_or_insert_with(|| {
                    let n = attempts.load(Ordering::SeqCst).max(1);
                    LIVE_BACKOFF_BASE_SECS
                        .saturating_mul(1u64 << n.min(6))
                        .min(LIVE_BACKOFF_CAP_SECS)
                        + full_jitter_secs(5)
                });
                tokio::time::sleep(std::time::Duration::from_secs(*wait)).await;
                backoff_wait = None;
                let _ = client.connect().await;
            } else {
                backoff_wait = None;
            }
        }
        let _ = client.disconnect().await;
        shared.live.store(false, Ordering::SeqCst);
    });
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
                        {"id_unit":13,"name":"KRI Ahmad Yani","hull_number":"KRI-AH-YN","position":{"latitude":-6.08,"longitude":106.91,"recorded_at":"2026-09-10T00:00:00Z","received_at":"2026-09-15T09:56:29Z","backfilled":true}},
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
