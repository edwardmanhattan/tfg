//! Live socket actor (backend split, step 5, last one).
//!
//! - [`LiveWire`]: tick-batched bridge — socket pushes queue, polls drain.
//! - [`LiveEvent`] / [`LiveCmd`]: the letters between actor and UI.
//! - `run_actor`: own thread + runtime, Centrifugo client, §7 refusals.
//!
//! History: the subscribe-await deadlock and the swallowed handshake
//! refusal both lived here; see the comments at the handshake.

use crate::geo::track::Fix;

use super::{BackendError, FeedEvent, GameMsg, GameOrderEvent, GamePositionUpdate, MinosRest, PollSource, parse_message_event, parse_order_event, parse_positions_event_for_game};

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

/// Reconnect pacing for the SDK's own mid-session retries (H12):
/// exponential 1 s → 30 s cap with full jitter, matching the custom
/// path's constants so the two halves never disagree about patience.
/// The stock `BackoffReconnect` has no jitter — a fleet reconnecting
/// in lockstep is exactly what jitter is for.
#[derive(Debug, Clone, Copy)]
struct JitteredBackoff;

impl tokio_centrifuge::config::ReconnectStrategy for JitteredBackoff {
    fn time_before_next_attempt(&self, attempt: u32) -> std::time::Duration {
        let exp = LIVE_BACKOFF_BASE_SECS
            .saturating_mul(1u64 << attempt.min(6))
            .min(LIVE_BACKOFF_CAP_SECS);
        std::time::Duration::from_secs(exp) + std::time::Duration::from_secs(full_jitter_secs(5))
    }
}

/// Connect-data envelope (§2, literal): the API access token rides
/// inside the connect command's `data` as `{"token": …}` — NOT in the
/// `token` field (Centrifugo core would 3500 it) and NOT in a header.
/// Verified live: absent → 107, garbage → 101, so the server reads this.
fn connect_data(token: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "token": token })).unwrap_or_default()
}

/// One-shot handshake diagnosis: token SHAPE only, never the secret.
/// Tells a no-expiry refusal (§3: refused, never eternal) apart from a
/// server-side secret/grpc mismatch. Tagged for one-shot diagnosis.
fn log_token_shape(tag: &str, token: &str) {
    use base64::Engine as _;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        eprintln!(
            "[LIVE-dbg] {tag}: opaque token (len={}, dots={})",
            token.len(),
            parts.len().saturating_sub(1)
        );
        return;
    }
    let payload = match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(p) => p,
        Err(_) => {
            eprintln!("[LIVE-dbg] {tag}: jwt-shaped but payload not base64url");
            return;
        }
    };
    let v: serde_json::Value = match serde_json::from_slice(&payload) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("[LIVE-dbg] {tag}: jwt payload not JSON");
            return;
        }
    };
    let now = chrono::Utc::now().timestamp();
    match v.get("exp").and_then(|e| e.as_i64()) {
        Some(exp) => eprintln!("[LIVE-dbg] {tag}: jwt with exp in {}s", exp - now),
        None => eprintln!("[LIVE-dbg] {tag}: jwt with NO exp claim (refused by the §3 rule)"),
    }
}

/// Actor -> UI reports. The UI owns refresh (keyring) and sign-out; the
/// actor never touches either.
#[derive(Debug, Clone)]
pub enum LiveEvent {
    Connected {
        client_id: String,
    },
    Announced(Vec<(String, Option<String>, Option<String>)>),
    Reconnecting {
        attempt: u32,
        wait_secs: u64,
    },
    /// Fatal refusal (101/103/107): the actor stops itself. The UI signs
    /// out (101) or shows the error; the wire goes quiet and stale flags
    /// read the outage as feed-down.
    Refused {
        code: u32,
        reason: String,
    },
    /// Token expiry (109): the UI refreshes silently and pushes the new
    /// token back down.
    RefreshDue,
    /// A game message arrived on a watched channel (H11): broadcast on
    /// `game:<id>`, addressed on `personal:<user>`. Notification with
    /// body, not the inbox — the UI draws and files it.
    Message(GameMsg),
    /// Best-effort committed-order publication. The HTTP 201 remains
    /// authoritative; this can only reconcile an existing Unknown result.
    OrderIssued(GameOrderEvent),
    /// One authoritative MinOS game-position publication. The game id
    /// travels with it so a late event cannot enter another exercise.
    GamePositions(GamePositionUpdate),
    /// The reconnect picture no longer names these operational vessels.
    /// The UI removes them instead of leaving a permanently stale marker.
    Vanished(Vec<String>),
    SocketError(String),
}

/// UI -> actor commands.
#[derive(Debug)]
pub enum LiveCmd {
    SetToken(String),
    /// Watch a game's broadcast channel (H11): `game:<id>`, or
    /// unsubscribe with None. Re-sending the held id is a no-op.
    WatchGame(Option<i64>),
    /// Watch the authoritative per-game position stream:
    /// `game:<id>:positions`.
    WatchGamePositions(Option<i64>),
    /// Watch the caller's personal channel (H11): `personal:<user>`,
    /// or unsubscribe with None.
    WatchPersonal(Option<i64>),
    Shutdown,
}

/// Subscribe one live channel: the live feed, game event/position
/// channels, or a personal addressed channel. The callback normalizes
/// the publication into the appropriate LiveEvent. The declaration
/// rides the (re)handshake, so watching works across reconnects with no
/// extra re-arm: the SDK resubscribes every non-unsubscribed slot itself.
fn watch_subscription(
    client: &tokio_centrifuge::client::Client,
    event_tx: &std::sync::mpsc::Sender<LiveEvent>,
    channel: &str,
) -> tokio_centrifuge::subscription::Subscription {
    let sub = client.new_subscription(channel);
    {
        let event_tx = event_tx.clone();
        let channel = channel.to_string();
        sub.on_publication(move |p: tokio_centrifuge::protocol::Publication| {
            let position_game_id = channel
                .strip_prefix("game:")
                .and_then(|rest| rest.strip_suffix(":positions"))
                .and_then(|id| id.parse::<i64>().ok());
            if let Some(game_id) = position_game_id
                && let Some(plot) = parse_positions_event_for_game(&p.data, game_id)
            {
                let _ = event_tx.send(LiveEvent::GamePositions(plot));
            } else if let Some(event) = parse_order_event(&p.data) {
                let _ = event_tx.send(LiveEvent::OrderIssued(event));
            } else {
                match parse_message_event(&p.data) {
                    Some(msg) => {
                        let _ = event_tx.send(LiveEvent::Message(msg));
                    }
                    None => eprintln!(
                        "{channel}: ignoring non-message publication ({} bytes)",
                        p.data.len()
                    ),
                }
            }
        });
    }
    {
        let channel = channel.to_string();
        sub.on_error(move |e| {
            eprintln!("{channel} subscription error: {e:?}");
        });
    }
    let _ = sub.subscribe();
    sub
}

struct LiveShared {
    queue: std::sync::Mutex<std::collections::VecDeque<FeedEvent>>,
    /// Last-known picture per ship (snapshot seed + socket overlay).
    /// The poll side replays it every tick. Every write merges by
    /// timestamp+id — a stale picture never overwrites a newer socket
    /// fix — and only a resync removes ships the picture no longer
    /// names (see `resync_snapshot`).
    known: std::sync::Mutex<std::collections::HashMap<String, Fix>>,
    live: std::sync::atomic::AtomicBool,
    /// One-shot picture emission (H6): armed by the connect-time merge
    /// and every resync, consumed by the next poll. Polls otherwise
    /// carry fresh fixes only, so a silent ship accumulates Registry
    /// misses and goes stale instead of being replayed fresh forever.
    seed: std::sync::atomic::AtomicBool,
}

impl LiveShared {
    /// Drain the queue in arrival order (M5): every publication flows,
    /// not just the newest per ship. The timestamp guard in
    /// `merge_fixes` still drops anything older than known, so
    /// retransmits never emit — but intermediate points reach the
    /// Registry and the Track keeps them.
    fn drain_in_order(&self) -> Vec<Fix> {
        if let Ok(mut q) = self.queue.lock() {
            q.drain(..).map(|ev| ev.to_fix()).collect()
        } else {
            Vec::new()
        }
    }

    /// Timestamp-guarded overlay of fixes onto the known picture: a
    /// fix at or before the known stamp for its ship is dropped
    /// loudly, never applied. Snapshots and socket bursts both funnel
    /// through here, so their arrival order stops mattering. Returns
    /// the accepted fixes — the poll side emits exactly these, so the
    /// Registry only ever sees fresh data (H6).
    fn merge_fixes(&self, fixes: Vec<Fix>, tag: &str) -> Vec<Fix> {
        let mut accepted = Vec::with_capacity(fixes.len());
        if fixes.is_empty() {
            return accepted;
        }
        if let Ok(mut known) = self.known.lock() {
            for fix in fixes {
                let id = fix.ship_id.clone();
                match known.get(&id) {
                    Some(prev) if prev.epoch_nanos() >= fix.epoch_nanos() => {
                        eprintln!(
                            "[LIVE-dbg] drop: unit={id} {tag} ts={} <= known ts={}",
                            fix.ts, prev.ts
                        );
                    }
                    _ => {
                        eprintln!(
                            "[LIVE-dbg] apply: unit={id} {tag} lat={:.6} lon={:.6} ts={}",
                            fix.position.latitude, fix.position.longitude, fix.ts
                        );
                        known.insert(id, fix.clone());
                        accepted.push(fix);
                    }
                }
            }
        }
        accepted
    }

    /// Reconnect recovery (no durable channel history to replay):
    /// apply everything the socket delivered first, merge the fresh
    /// picture over it by timestamp+id, then drop ships the picture
    /// no longer names. Draining before removing is the point — a
    /// ship with a queued fix newer than the picture is live, not
    /// vanished, and the drain puts it into `known` ahead of the
    /// removal pass.
    fn resync_snapshot(&self, picture: Vec<Fix>) -> Vec<String> {
        let queued = self.drain_in_order();
        let queued_accepted = self.merge_fixes(queued, "socket");
        let mut ids: std::collections::HashSet<String> =
            picture.iter().map(|f| f.ship_id.clone()).collect();
        // A queued socket fix is proof of life. It must be included in
        // the survivor set, otherwise the removal pass immediately
        // deletes the very ship the drain just rescued.
        ids.extend(queued_accepted.iter().map(|f| f.ship_id.clone()));
        self.merge_fixes(picture, "snapshot");
        let mut gone = Vec::new();
        if let Ok(mut known) = self.known.lock() {
            gone = known
                .keys()
                .filter(|id| !ids.contains(*id))
                .cloned()
                .collect();
            for id in &gone {
                eprintln!("[LIVE-dbg] resync: unit={id} vanished from the picture");
                known.remove(id);
            }
        }
        // The new picture seeds the Registry once (H6): ships it names
        // get markers on the next poll; silence after that accrues
        // misses normally.
        self.seed.store(true, std::sync::atomic::Ordering::SeqCst);
        gone
    }

    /// Consume a pending one-shot picture emission, if armed.
    fn take_seed(&self) -> bool {
        self.seed.swap(false, std::sync::atomic::Ordering::SeqCst)
    }
}

/// Tick-batched live wire (transport ticket): socket publications queue
/// on the actor thread; each poll emits fresh fixes only, plus a
/// one-shot picture after (re)connects so markers exist. Silence
/// counts as Registry misses and disconnects read as stale. Disconnect
/// yields empty rounds. Subscribe-then-snapshot with merge-by-id is the
/// recovery model throughout: nothing published between the subscribe
/// and the picture is lost, and a stale picture never wins over newer
/// socket data (see `LiveShared`).
pub struct LiveWire {
    shared: std::sync::Arc<LiveShared>,
    cmd_tx: std::sync::mpsc::Sender<LiveCmd>,
}

impl LiveWire {
    /// Subscribe first, snapshot second (H4): the actor declares the
    /// subscription before the picture is taken, so a fix published in
    /// between is queued rather than missed — Centrifugo keeps no
    /// history to replay it from. The picture then merges by
    /// timestamp+id, so anything the socket already delivered newer
    /// wins either way round. Snapshot failure fails the whole connect
    /// — the caller falls back to an empty wire with the reason in its
    /// description. `wake_tx` is pinged per publication so the poll
    /// thread can cut its 2 s sleep short; the sim cadence is untouched.
    pub fn connect(
        ws_url: &str,
        rest: &MinosRest,
        token: &str,
        cmd_tx: std::sync::mpsc::Sender<LiveCmd>,
        cmd_rx: std::sync::mpsc::Receiver<LiveCmd>,
        event_tx: std::sync::mpsc::Sender<LiveEvent>,
        wake_tx: std::sync::mpsc::Sender<()>,
    ) -> Result<Self, BackendError> {
        log_token_shape("connect", token);
        let shared = std::sync::Arc::new(LiveShared {
            queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
            known: std::sync::Mutex::new(std::collections::HashMap::new()),
            // H5: not live until the handshake says so — the actor is
            // born down, and the first on_connected raises it. Polls
            // read as feed-down until then, never as a live picture.
            live: std::sync::atomic::AtomicBool::new(false),
            seed: std::sync::atomic::AtomicBool::new(false),
        });
        let token = token.to_string();
        let ws_url = ws_url.to_string();
        let rest_base = rest.base_url.clone();
        let actor_shared = shared.clone();
        let actor_token = token.clone();
        let actor_events = event_tx.clone();
        std::thread::spawn(move || {
            run_actor(ws_url, rest_base, actor_token, actor_shared, cmd_rx, actor_events, wake_tx);
        });
        let snap = rest.snapshot(&token)?;
        let _ = event_tx.send(LiveEvent::Announced(snap.announced));
        shared.merge_fixes(snap.fixes, "snapshot");
        // H6: the connect-time picture seeds the Registry once — every
        // poll after this carries fresh fixes only.
        shared.seed.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(Self { shared, cmd_tx })
    }
}

impl PollSource for LiveWire {
    /// Fresh fixes only (H6): the accepted socket burst, plus the
    /// one-shot picture when a (re)connect armed it. Replaying the
    /// whole known picture every round froze Registry misses (a
    /// duplicate marks `seen` without resetting) and inflated
    /// last-seen/fix counts — a ship that stops publishing now goes
    /// stale after 3 missed 2 s polls instead of never.
    fn poll(&mut self) -> Result<Vec<Fix>, BackendError> {
        if !self.shared.live.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(Vec::new()); // actor down: feed-down reads as stale
        }
        let fresh = self.shared.drain_in_order();
        let mut out = self.shared.merge_fixes(fresh, "socket");
        if self.shared.take_seed() {
            match self.shared.known.lock() {
                Ok(known) => out.extend(known.values().cloned()),
                Err(e) => return Err(BackendError::Other(e.to_string())),
            }
        }
        Ok(out)
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
    wake_tx: std::sync::mpsc::Sender<()>,
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
        // §2, literal: the API token rides in connect `data`, while the
        // core `token` field stays empty (anything there gets 3500'd by
        // Centrifugo core before our backend is even consulted).
        let client = Client::new(
            &ws_url,
            Config::new()
                .with_connect_data(connect_data(&token))
                .with_reconnect_strategy(JitteredBackoff)
                .use_json(),
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
            let wake_tx = wake_tx.clone();
            sub.on_publication(move |p: Publication| {
                match serde_json::from_slice::<FeedEvent>(&p.data) {
                    Ok(ev) => {
                        eprintln!("[LIVE-dbg] pub: {}", ev.summary());
                        if let Ok(mut q) = shared.queue.lock() {
                            q.push_back(ev);
                        }
                        // Fast lane: poke the poll thread so the marker moves
                        // in ~100 ms instead of waiting out the 2 s tick.
                        let _ = wake_tx.send(());
                    }
                    Err(e) => eprintln!("live-feed: unparsable publication: {e}"),
                }
            });
            sub.on_error(|e| {
                eprintln!("live-feed subscription error: {e:?}");
            });
        }
        // Disconnects: the refusal policy. Fatal codes stop the loop;
        // 109 asks the UI for a silent refresh; the rest back off.
        // Declared ahead of the callbacks: a success resets the
        // attempt counter (H12), and every disconnect marks itself so
        // the handshake outcome below never double-reports.
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let disconnect_seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Connected: report up, reset backoff, re-read the snapshot
        // (reconcile-by-id: no channel history), re-arm the subscription.
        {
            let event_tx = event_tx.clone();
            let shared = shared.clone();
            let rest_base = rest_base.clone();
            let current_token = current_token.clone();
            let resub = sub.clone();
            let wake_tx = wake_tx.clone();
            let attempts = attempts.clone();
            client.on_connected(move |e: ConnectedEvent<'_>| {
                let _ = event_tx.send(LiveEvent::Connected {
                    client_id: e.client_id.to_string(),
                });
                shared.live.store(true, Ordering::SeqCst);
                // H12: success resets the backoff — the next outage
                // starts patient-zero, never mid-ladder.
                attempts.store(0, Ordering::SeqCst);
                // Wake the poll thread so the first picture shows
                // immediately once the connect-time merge lands it.
                let _ = wake_tx.send(());
                if tokio::runtime::Handle::try_current().is_ok() {
                    let resub = resub.clone();
                    tokio::task::spawn(async move {
                        let _ = resub.subscribe().await;
                    });
                    // Snapshot re-read off the callback path.
                    let event_tx = event_tx.clone();
                    let shared = shared.clone();
                    let rest_base = rest_base.clone();
                    let current_token = current_token.clone();
                    tokio::task::spawn_blocking(move || {
                        let tok = current_token.lock().map(|t| t.clone()).unwrap_or_default();
                        let Ok(rest) = MinosRest::new(&rest_base) else {
                            return;
                        };
                        let Ok(snap) = rest.snapshot(&tok) else {
                            return;
                        };
                        let _ = event_tx.send(LiveEvent::Announced(snap.announced));
                        // H7/H8: merge by timestamp+id, drop the vanished.
                        // Queued socket fixes drain first inside, so a
                        // ship live on the socket is never reaped by an
                        // older picture.
                        let gone = shared.resync_snapshot(snap.fixes);
                        if !gone.is_empty() {
                            let _ = event_tx.send(LiveEvent::Vanished(gone));
                        }
                    });
                }
            });
        }
        // Disconnects: the refusal policy is below; the counters live
        // above with the connect block.
        {
            let event_tx = event_tx.clone();
            let stop = stop.clone();
            let attempts = attempts.clone();
            let shared = shared.clone();
            let disconnect_seen = disconnect_seen.clone();
            client.on_disconnected(move |e: DisconnectedEvent<'_>| {
                shared.live.store(false, Ordering::SeqCst);
                if stop.load(Ordering::SeqCst) {
                    // Terminal or shutting down: the outcome (Refused,
                    // or the Shutdown itself) is already reported — a
                    // trailing Reconnecting here would only mislead.
                    return;
                }
                disconnect_seen.store(true, Ordering::SeqCst);
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
        // Progress callbacks (transport ticket): the SDK only logs at
        // debug level (no logger here), so handshake attempts would be
        // invisible otherwise. Tagged for one-shot diagnosis.
        client.on_connecting(|e| {
            eprintln!("[LIVE-dbg] connecting (code={} {})", e.code, e.reason);
        });
        // Declare the subscription BEFORE connecting, fire-and-forget (as
        // the SDK's own pubsub example does): awaiting subscribe() while
        // disconnected never resolves — the reply needs the connection
        // that this await is blocking. The server arms the declaration
        // on handshake, and on_connected re-arms per reconnect.
        {
            let _ = sub.subscribe();
        }
        eprintln!("[LIVE-dbg] handshake start: {ws_url}");
        // Never block the actor on a handshake that may never resolve
        // (no reply at all still happens): time out so the outcome is
        // loud and the command loop below stays alive to retry. A
        // refused handshake now fires on_disconnected with its code via
        // the vendored patch, and a later background connect still
        // reports via on_connected.
        match tokio::time::timeout(std::time::Duration::from_secs(15), async {
            client.connect().await
        })
        .await
        {
            Ok(Ok(())) => eprintln!("[LIVE-dbg] handshake resolved: connected"),
            Ok(Err(())) => {
                // H5: with the SDK surfacing refusal codes, the
                // disconnect callback has already sent the real event
                // (Refused with 101/103/107, RefreshDue for 109, or a
                // Reconnecting) ahead of this resolution — stay quiet
                // rather than overwriting it with a generic error. Only
                // a resolution with NO callback at all is still news.
                if disconnect_seen.load(Ordering::SeqCst) {
                    eprintln!("[LIVE-dbg] handshake resolved: refused (event sent)");
                } else {
                    eprintln!("[LIVE-dbg] handshake resolved: refused (no event)");
                    let _ = event_tx.send(LiveEvent::SocketError(
                        "handshake refused with no disconnect event".to_string(),
                    ));
                }
            }
            Err(_) => {
                eprintln!("[LIVE-dbg] handshake timed out after 15s (no reply)");
                let _ = event_tx.send(LiveEvent::SocketError(
                    "handshake timed out after 15s (no connect reply)".to_string(),
                ));
            }
        }
        // Command + reconnect loop: connect-data push-down applies live via
        // set_connect_data; shutdown/fatal disconnects the client and ends us.
        // Message watches (H11) live here too: the game and personal
        // channels join and leave as the hold and the session change.
        let mut backoff_wait: Option<u64> = None;
        let mut game_channel: Option<String> = None;
        let mut game_sub: Option<tokio_centrifuge::subscription::Subscription> = None;
        let mut positions_channel: Option<String> = None;
        let mut positions_sub: Option<tokio_centrifuge::subscription::Subscription> = None;
        let mut personal_channel: Option<String> = None;
        let mut personal_sub: Option<tokio_centrifuge::subscription::Subscription> = None;
        // Swap one watched channel for another (or drop it): unsubscribe
        // the old handle when it names something else, then declare the
        // new one — the declaration rides the (re)handshake.
        let rewatch = |client: &tokio_centrifuge::client::Client,
                           event_tx: &std::sync::mpsc::Sender<LiveEvent>,
                           slot_channel: &mut Option<String>,
                           slot_sub: &mut Option<tokio_centrifuge::subscription::Subscription>,
                           prefix: &str,
                           id: Option<i64>| {
            let want = id.map(|i| format!("{prefix}:{i}"));
            if want == *slot_channel {
                return;
            }
            if let Some(old) = slot_sub.take() {
                let _ = old.unsubscribe();
            }
            *slot_channel = want.clone();
            *slot_sub =
                want.map(|ch| watch_subscription(client, event_tx, &ch));
            match slot_channel {
                Some(ch) => eprintln!("[LIVE-dbg] watching {ch}"),
                None => eprintln!("[LIVE-dbg] unwatching {prefix}"),
            }
        };
        let rewatch_positions = |client: &tokio_centrifuge::client::Client,
                                  event_tx: &std::sync::mpsc::Sender<LiveEvent>,
                                  slot_channel: &mut Option<String>,
                                  slot_sub: &mut Option<tokio_centrifuge::subscription::Subscription>,
                                  id: Option<i64>| {
            let want = id.map(|i| format!("game:{i}:positions"));
            if want == *slot_channel {
                return;
            }
            if let Some(old) = slot_sub.take() {
                let _ = old.unsubscribe();
            }
            *slot_channel = want.clone();
            *slot_sub = want.map(|ch| watch_subscription(client, event_tx, &ch));
            match slot_channel {
                Some(ch) => eprintln!("[LIVE-dbg] watching {ch}"),
                None => eprintln!("[LIVE-dbg] unwatching game positions"),
            }
        };
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            match cmd_rx.try_recv() {
                Ok(LiveCmd::SetToken(t)) => {
                    if let Ok(mut cur) = current_token.lock() {
                        *cur = t.clone();
                    }
                    // Refresh applies to `data`, never the core token field.
                    client.set_connect_data(connect_data(&t));
                }
                Ok(LiveCmd::WatchGame(id)) => {
                    rewatch(&client, &event_tx, &mut game_channel, &mut game_sub, "game", id);
                }
                Ok(LiveCmd::WatchGamePositions(id)) => {
                    rewatch_positions(
                        &client,
                        &event_tx,
                        &mut positions_channel,
                        &mut positions_sub,
                        id,
                    );
                }
                Ok(LiveCmd::WatchPersonal(id)) => {
                    rewatch(
                        &client,
                        &event_tx,
                        &mut personal_channel,
                        &mut personal_sub,
                        "personal",
                        id,
                    );
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
    use crate::geo::track::FixSource;
    use crate::geo::GeoPosition;
    use tokio_centrifuge::config::ReconnectStrategy as _;

    fn fix(id: &str, lat: f64, lon: f64, ts: &str) -> Fix {
        Fix {
            ship_id: id.into(),
            position: GeoPosition { latitude: lat, longitude: lon },
            ts: ts.into(),
            received_at: None,
            heading_deg: None,
            speed_kn: None,
            accuracy_m: None,
            name: None,
            hull_number: None,
            backfilled: false,
            source: FixSource::Wire,
            age_secs: None,
            seq: 0,
        }
    }

    fn shared() -> std::sync::Arc<LiveShared> {
        std::sync::Arc::new(LiveShared {
            queue: std::sync::Mutex::new(std::collections::VecDeque::new()),
            known: std::sync::Mutex::new(std::collections::HashMap::new()),
            live: std::sync::atomic::AtomicBool::new(true),
            seed: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn known_ids(s: &LiveShared) -> Vec<String> {
        let mut ids: Vec<String> = s.known.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }

    #[test]
    fn jittered_backoff_stays_in_band() {
        // H12: exponential 1 s → 30 s cap with up to +5 s of full
        // jitter, matching the custom path's constants.
        for attempt in 0..10u32 {
            let d = JitteredBackoff.time_before_next_attempt(attempt);
            let exp = 1u64.saturating_mul(1u64 << attempt.min(6)).min(30);
            assert!(
                d >= std::time::Duration::from_secs(exp)
                    && d <= std::time::Duration::from_secs(exp + 5),
                "attempt {attempt}: {d:?} outside [{exp}, {}]",
                exp + 5,
            );
        }
    }

    #[test]
    fn snapshot_merge_keeps_newer_socket_fix() {
        // H7: a picture older than the socket burst loses by timestamp,
        // whichever of the two arrived first.
        let s = shared();
        s.merge_fixes(vec![fix("13", -6.0, 106.9, "2026-09-15T10:00:00Z")], "socket");
        s.merge_fixes(vec![fix("13", -6.1, 106.8, "2026-09-15T09:00:00Z")], "snapshot");
        let known = s.known.lock().unwrap();
        assert_eq!(known["13"].position.latitude, -6.0, "stale picture drops");
        // And the reverse order agrees: newer picture still wins.
        drop(known);
        s.merge_fixes(vec![fix("13", -6.2, 106.7, "2026-09-15T11:00:00Z")], "snapshot");
        assert_eq!(
            s.known.lock().unwrap()["13"].position.latitude, -6.2,
            "newer picture applies"
        );
    }

    #[test]
    fn polls_carry_fresh_only_with_one_shot_seed() {
        // H6: the seed emits the picture once; steady polls carry only
        // newly accepted fixes, so silence accrues misses downstream.
        use crate::backend::PollSource;
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let s = shared();
        s.merge_fixes(
            vec![
                fix("13", -6.0, 106.9, "2026-09-15T10:00:00Z"),
                fix("14", -6.2, 107.0, "2026-09-15T10:00:00Z"),
            ],
            "snapshot",
        );
        s.seed.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut wire = LiveWire { shared: s, cmd_tx };
        let first = wire.poll().expect("seed poll");
        assert_eq!(first.len(), 2, "one-shot picture seeds markers");
        let second = wire.poll().expect("steady poll");
        assert!(second.is_empty(), "nothing fresh, nothing emitted");
        // A newer socket fix flows; an older one does not.
        let fresh: FeedEvent = serde_json::from_str(
            r#"{"id_unit":13,"latitude":-6.01,"longitude":106.91,"recorded_at":"2026-09-15T10:05:00Z","received_at":"2026-09-15T10:05:01Z"}"#,
        )
        .expect("§5 parses");
        let stale: FeedEvent = serde_json::from_str(
            r#"{"id_unit":14,"latitude":-6.2,"longitude":107.0,"recorded_at":"2026-09-15T09:00:00Z","received_at":"2026-09-15T09:00:01Z"}"#,
        )
        .expect("§5 parses");
        wire.shared.queue.lock().unwrap().push_back(fresh);
        wire.shared.queue.lock().unwrap().push_back(stale);
        let third = wire.poll().expect("burst poll");
        assert_eq!(third.len(), 1, "only the newer fix is emitted");
        assert_eq!(third[0].ship_id, "13");
    }

    #[test]
    fn burst_keeps_intermediate_points_in_order() {
        // M5: one poll round carries every fresh publication, so the
        // Track keeps intermediate points instead of only the newest.
        use crate::backend::PollSource;
        let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
        let s = shared();
        let mut wire = LiveWire { shared: s, cmd_tx };
        for (i, ts) in [
            "2026-09-15T10:00:00Z",
            "2026-09-15T10:01:00Z",
            "2026-09-15T10:02:00Z",
        ]
        .iter()
        .enumerate()
        {
            let ev: FeedEvent = serde_json::from_str(&format!(
                r#"{{"id_unit":13,"latitude":-6.0,"longitude":{},"recorded_at":"{}","received_at":"{}"}}"#,
                106.9 + i as f64 * 0.01,
                ts,
                ts,
            ))
            .expect("§5 parses");
            wire.shared.queue.lock().unwrap().push_back(ev);
        }
        let round = wire.poll().expect("burst poll");
        assert_eq!(round.len(), 3, "intermediates flow, not just newest");
        assert!(
            round.windows(2).all(|w| w[0].epoch_nanos() < w[1].epoch_nanos()),
            "arrival order preserved"
        );
    }

    #[test]
    fn resync_reaps_vanished_but_spares_queued() {
        // H8: ships the picture no longer names are dropped — unless a
        // queued socket fix proves them live.
        let s = shared();
        s.merge_fixes(
            vec![
                fix("13", -6.0, 106.9, "2026-09-15T10:00:00Z"),
                fix("14", -6.2, 107.0, "2026-09-15T10:00:00Z"),
            ],
            "snapshot",
        );
        let ev: FeedEvent = serde_json::from_str(
            r#"{"id_unit":14,"latitude":-6.21,"longitude":107.01,"recorded_at":"2026-09-15T10:05:00Z","received_at":"2026-09-15T10:05:01Z"}"#,
        )
        .expect("§5 parses");
        s.queue.lock().unwrap().push_back(ev);
        // The picture names only 13 now, but 14 has queued proof of life.
        s.resync_snapshot(vec![fix("13", -6.0, 106.9, "2026-09-15T10:01:00Z")]);
        assert_eq!(known_ids(&s), vec!["13", "14"]);
        // With nothing queued and still unnamed, 14 is reaped.
        s.resync_snapshot(vec![fix("13", -6.0, 106.9, "2026-09-15T10:02:00Z")]);
        assert_eq!(known_ids(&s), vec!["13"]);
    }
}
