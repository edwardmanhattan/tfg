//! The access token's own lifetime, and the refresh chain behind it.
//!
//! Minos issues a short-lived access token (an hour, by contract — see
//! `docs/minos-api.yaml`, `TokenPair`) and a refresh token that ROTATES on
//! every use: the response to `/auth/refresh` carries a new one and the
//! presented one is invalidated, so a replay is rejected
//! (`docs/minos-backend-postman.json`, "Exchange a refresh token for a new
//! access token").
//!
//! That pair is what makes "sign in once" possible, and what makes it easy to
//! get wrong. Three facts shape this module:
//!
//! 1. **The access token outlives no page it was printed on.** A console that
//!    runs for a whole exercise runs longer than an hour, so the token in
//!    memory is stale long before the operator is done. Before this module
//!    the console simply kept it: the socket was refused with 109, the plot
//!    read answered 401 ("game feed: plot read failed: unauthorized"), and
//!    the only cure was to sign in again. Nothing rotated the chain.
//! 2. **Rotation is destructive.** The old refresh token dies the moment the
//!    new one exists, so two rotations racing each other leave one caller
//!    holding a dead token — and a replay may be read as theft. Therefore
//!    exactly ONE thread ever calls `/auth/refresh`; everyone else asks that
//!    thread to (`nudge`). Single-flight is structural here, not a mutex
//!    around a shared call.
//! 3. **The rotated refresh token must be persisted at once.** The keyring
//!    entry is the next launch's only credential, and a rotation that is not
//!    written back leaves the keyring holding a token the server has already
//!    burned. So the keeper owns the keyring entry for the session: it writes
//!    on every rotation (`install` takes an identifier; the environment's
//!    capture credential passes `None` and stays memory-only).
//!
//! **Two ways a rotation is asked for.** The PROACTIVE one is the clock: the
//! keeper sleeps until a slack before `expires_in` and replaces the token
//! before anything is refused. The REACTIVE one is a refusal: the server is
//! the authority on what a token is worth, so a 109 on the socket or a 401 on
//! a plot read means "now", not "when the clock says so" — the two clocks
//! (ours and the server's) need not agree, and the server's wins. Both land
//! in the same place, which is why there is one schedule (`due`) rather than
//! two timers.
//!
//! **What the keeper is not.** It holds no socket, no session cell and no
//! shell. It reports rotations on a channel ([`TokenEvent`]) and the binary
//! turns them into pushes (the hold cell, the socket's connect data). Keeping
//! that split means this module can be tested against a stub without a
//! window, and the token has exactly one writer.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{BackendError, MinosAuth, TokenPair, keyring_clear, keyring_save};

/// The most a proactive rotation may run ahead of the expiry it replaces.
///
/// Five minutes of an hour-long token: long enough that a slow network, a
/// suspended laptop or an asleep server still lands before the old token
/// dies, short enough that the console does not churn the refresh chain.
const SLACK_CAP_SECS: u64 = 300;

/// Below this many seconds a token is too short-lived to replace early: the
/// rotation fires at expiry instead. Without the floor a `expires_in: 1`
/// answer would mean "refresh immediately, forever" — a rotation loop that
/// burns the refresh chain faster than the server mints it.
const SLACK_FLOOR_SECS: u64 = 60;

/// The wait after a failed rotation, and the floor between two REACTIVE ones.
///
/// The two are the same number on purpose. A refused token asks for a
/// rotation now; a rotation that has just been tried (and failed, or
/// succeeded into a token the socket has not yet picked up) must not be tried
/// again for half a minute, or a socket retrying an old token in a tight loop
/// would rotate the chain with it. Thirty seconds is short enough that a
/// transient failure is retried well inside the access token's life.
const RETRY_WAIT: Duration = Duration::from_secs(30);

/// The longest a failing rotation waits before trying again: the server is
/// asked at most twice a minute while it is unhealthy.
const RETRY_CAP: Duration = Duration::from_secs(60);

/// One access token and the instant it stops being accepted.
///
/// `expires_at` is the MONOTONIC clock, not the wall clock: the wall clock can
/// step (an NTP correction, a timezone change) and a lifetime measured with it
/// would jump with the step. What it does NOT measure is time spent suspended:
/// the monotonic clock stops while the machine sleeps, so a laptop that slept
/// through the expiry wakes believing the token is still good. That is the
/// reactive path's job — the first refusal rotates it — and the reason a
/// refusal is never treated as "the clock will catch up".
#[derive(Debug, Clone)]
struct Access {
    token: String,
    expires_at: Instant,
}

/// The session's credential state. One lock, because every field is read and
/// written together by "is this token still the session's".
#[derive(Default)]
struct Chain {
    /// The token every request carries. `None` when signed out.
    access: Option<Access>,
    /// The refresh token: rotated by every rotation, dead the moment the next
    /// one works. `None` when the login did not return one.
    refresh: Option<String>,
    /// The identifier the keyring entry is keyed by. `None` for a credential
    /// that came from the environment: a capture's account is not the
    /// machine's own, so nothing is written to the keyring for it.
    identifier: Option<String>,
    /// When the refresher should next try. `None` when signed out — there is
    /// no chain to keep alive, and the thread sleeps until an install.
    due: Option<Instant>,
    /// When the last rotation was attempted, for [`RETRY_WAIT`].
    tried_at: Option<Instant>,
    /// Consecutive failed rotations: the retry ladder's rung.
    fails: u32,
}

/// A rotation, as the rest of the console sees it.
///
/// Two events and no more, because those are the two things every holder of
/// the token must do: swap it, or stop using it.
#[derive(Debug)]
pub enum TokenEvent {
    /// A new access token. Every holder swaps to it — the plot reader's hold
    /// cell, the socket's connect data, the sign-in state writes post with.
    Rotated { access_token: String },
    /// The refresh token was refused (401): the chain is dead and only a
    /// sign-in can start another. The keyring entry is already cleared, so a
    /// relaunch shows the card instead of rotating a burned token.
    Expired,
}

/// The session's credential, kept alive.
///
/// No `Debug`: it holds a bearer token and a refresh token, and a `Debug` impl
/// is a print statement waiting to put one in a log.
pub struct TokenKeeper {
    host: String,
    inner: Mutex<Chain>,
    /// Wakes the one rotation thread. A send carries no meaning beyond
    /// "look at the plan again" — the plan lives in `inner`.
    nudge_tx: Sender<()>,
    events: Sender<TokenEvent>,
}

impl TokenKeeper {
    /// Start the keeper and its one rotation thread.
    ///
    /// Returns the receiver of [`TokenEvent`]s: the binary drains it on a
    /// thread of its own and pushes the new token to whoever holds one. The
    /// channel is unbounded, so a rotation that lands while nobody is
    /// listening is not lost — it waits.
    pub fn start(host: &str) -> (Arc<Self>, Receiver<TokenEvent>) {
        let (nudge_tx, nudge_rx) = std::sync::mpsc::channel::<()>();
        let (events, event_rx) = std::sync::mpsc::channel::<TokenEvent>();
        let keeper = Arc::new(Self {
            host: host.trim_end_matches('/').to_string(),
            inner: Mutex::new(Chain::default()),
            nudge_tx,
            events,
        });
        let thread_keeper = keeper.clone();
        std::thread::spawn(move || thread_keeper.run(nudge_rx));
        (keeper, event_rx)
    }

    /// Adopt a token pair as the session's: the login answer, the keyring's
    /// rotation at launch, or the environment's capture credential.
    ///
    /// `identifier` names the account the keyring entry belongs to; `None`
    /// means "keep this in memory only" (see [`Chain::identifier`]).
    ///
    /// The pair's refresh token is written to the keyring HERE, not by the
    /// caller, because from this moment the keeper is the only thing that
    /// rotates it — and a rotation the keyring does not know about is a
    /// launch that cannot sign in.
    pub fn install(&self, pair: &TokenPair, identifier: Option<&str>) {
        let lifetime = Duration::from_secs(pair.expires_in);
        if let Ok(mut chain) = self.inner.lock() {
            let access = Access {
                token: pair.access_token.clone(),
                expires_at: Instant::now() + lifetime,
            };
            // The plan replaces the token a slack before it dies, measured
            // from the instant its life started at — the token's own clock,
            // not a second reading of `now` a lock and a write later.
            chain.due = Some(access.expires_at.saturating_sub(slack_for(pair.expires_in)));
            chain.access = Some(access);
            // An answer without a refresh token keeps the one already held:
            // the chain is a chain, and a refresh that answers with no new
            // link does not break it.
            if pair.refresh_token.is_some() {
                chain.refresh = pair.refresh_token.clone();
            }
            chain.identifier = identifier.map(str::to_string);
            // A fresh install is a fresh clock: the reactive floor is for a
            // token that has been refused, and this one has not been used.
            chain.tried_at = None;
            chain.fails = 0;
        }
        if let (Some(id), Some(rt)) = (identifier, pair.refresh_token.as_deref()) {
            store(id, rt);
        }
        let _ = self.nudge_tx.send(());
    }

    /// The token to send now. Empty when signed out — the same convention the
    /// hold cell and every write path already use.
    pub fn current(&self) -> String {
        self.inner
            .lock()
            .ok()
            .and_then(|chain| chain.access.as_ref().map(|a| a.token.clone()))
            .unwrap_or_default()
    }

    /// Sign out: forget the token and the chain. The caller clears the
    /// keyring entry it owns (`keyring_clear`) — a deliberate sign-out is the
    /// operator's, and it is the caller who knows the identifier.
    pub fn clear(&self) {
        if let Ok(mut chain) = self.inner.lock() {
            *chain = Chain::default();
        }
        let _ = self.nudge_tx.send(());
    }

    /// "The server refused the token you gave us" — rotate now.
    ///
    /// Called from a 109 disconnect and from a 401 on a read. A no-op when
    /// signed out (there is nothing to rotate) and paced by [`RETRY_WAIT`]
    /// when the last attempt is still fresh (a refusal storm is one problem,
    /// and each rotation would burn a refresh token to prove it).
    pub fn nudge(&self) {
        if let Ok(mut chain) = self.inner.lock() {
            if chain.access.is_none() {
                return;
            }
            // Now, or as soon as the floor allows — the thread reads `due`
            // and the floor together, so this only has to say "look again".
            chain.due = Some(Instant::now());
        }
        let _ = self.nudge_tx.send(());
    }

    /// The rotation thread: wait out the plan, rotate, wait again.
    fn run(&self, nudges: Receiver<()>) {
        loop {
            let due = match self.inner.lock() {
                Ok(chain) => chain.due,
                Err(_) => return,
            };
            let Some(due) = due else {
                // Signed out: the thread has nothing to keep alive and
                // sleeps on the mailbox until an install or a clear.
                if nudges.recv().is_err() {
                    return; // the keeper is gone; so is the session
                }
                continue;
            };
            let now = Instant::now();
            if due > now {
                match nudges.recv_timeout(due - now) {
                    // A nudge moved the plan: re-read it rather than
                    // rotating on a schedule that has just changed.
                    Ok(()) => continue,
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            // Due, or asked for. `rotate` re-reads the chain under the lock
            // and always moves `due` — forwards, or to None — so this loop
            // cannot spin on a plan it did not advance.
            self.rotate();
        }
    }

    /// The one place `/auth/refresh` is called.
    ///
    /// Every path out of here moves `due` — forwards, or to `None` — because
    /// the thread that called it reads the plan again immediately, and a plan
    /// still in the past would be a rotation loop.
    fn rotate(&self) {
        let (refresh, floor) = {
            let Ok(mut chain) = self.inner.lock() else {
                return;
            };
            if chain.access.is_none() {
                // Signed out between the plan and the attempt: there is
                // nothing to rotate, and the plan is void.
                chain.due = None;
                return;
            }
            if chain.due.is_none() {
                return;
            }
            let floor = chain
                .tried_at
                .and_then(|at| RETRY_WAIT.checked_sub(at.elapsed()));
            (chain.refresh.clone(), floor)
        };
        if let Some(left) = floor {
            // A nudge inside the floor: the attempt is owed, not now. Move
            // the plan to the floor's end and let the loop wait it out.
            if let Ok(mut chain) = self.inner.lock() {
                chain.due = Some(Instant::now() + left);
            }
            return;
        }
        let Some(refresh) = refresh else {
            // A login that returned no refresh token: nothing to rotate.
            // The access token is the session, and the session ends with it
            // — said out loud, because the alternative is a silent 401 an
            // hour from now.
            if let Ok(mut chain) = self.inner.lock() {
                chain.due = None;
            }
            eprintln!("session: no refresh token — this session ends with the access token");
            return;
        };
        let auth = match MinosAuth::new(&self.host) {
            Ok(auth) => auth,
            Err(e) => {
                self.failed(&e.to_string());
                return;
            }
        };
        match auth.refresh(&refresh) {
            Ok(pair) => self.rotated(pair),
            // The one failure that is not worth retrying: the chain is dead
            // server-side, and every further attempt is a replay of a burned
            // token. Sign in again — that is the only cure.
            Err(BackendError::Unauthorized { .. }) => self.expired(),
            Err(e) => self.failed(&e.to_string()),
        }
    }

    /// A rotation landed: adopt the pair, persist the new link, tell everyone.
    fn rotated(&self, pair: TokenPair) {
        let lifetime = Duration::from_secs(pair.expires_in);
        let identifier = self.inner.lock().ok().and_then(|c| c.identifier.clone());
        if let (Some(id), Some(rt)) = (identifier.as_deref(), pair.refresh_token.as_deref()) {
            store(id, rt);
        }
        if let Ok(mut chain) = self.inner.lock() {
            let access = Access {
                token: pair.access_token.clone(),
                expires_at: Instant::now() + lifetime,
            };
            chain.due = Some(access.expires_at.saturating_sub(slack_for(pair.expires_in)));
            chain.access = Some(access);
            if pair.refresh_token.is_some() {
                chain.refresh = pair.refresh_token.clone();
            }
            chain.tried_at = Some(Instant::now());
            chain.fails = 0;
        }
        eprintln!(
            "session: access token rotated (fresh for {}s)",
            pair.expires_in
        );
        let _ = self.events.send(TokenEvent::Rotated {
            access_token: pair.access_token,
        });
    }

    /// A transient failure: keep the token, try again shortly.
    ///
    /// The token is NOT cleared. A server that is down for a minute does not
    /// invalidate a token that is good for an hour, and dropping the session
    /// on a network blip would sign the operator out of a running exercise
    /// for no reason.
    fn failed(&self, why: &str) {
        let wait = match self.inner.lock() {
            Ok(mut chain) => {
                chain.fails = chain.fails.saturating_add(1);
                chain.tried_at = Some(Instant::now());
                let wait = if chain.fails <= 1 {
                    RETRY_WAIT
                } else {
                    RETRY_CAP
                };
                chain.due = Some(Instant::now() + wait);
                wait
            }
            Err(_) => return,
        };
        eprintln!(
            "session: token rotation failed ({why}); retrying in {}s",
            wait.as_secs()
        );
    }

    /// The chain is dead: drop it, forget the keyring entry, report it.
    fn expired(&self) {
        let identifier = match self.inner.lock() {
            Ok(mut chain) => {
                let identifier = chain.identifier.clone();
                chain.access = None;
                chain.refresh = None;
                chain.due = None;
                chain.tried_at = Some(Instant::now());
                chain.fails = 0;
                identifier
            }
            Err(_) => return,
        };
        if let Some(id) = identifier.as_deref() {
            if let Err(e) = keyring_clear(id) {
                eprintln!("session: keyring entry for {id} not cleared ({e})");
            }
        }
        eprintln!("session: refresh token refused — sign in again");
        let _ = self.events.send(TokenEvent::Expired);
    }
}

/// Persist a refresh token, loudly on failure.
///
/// A keyring write that fails is not fatal to the running session — the chain
/// keeps rotating in memory — but it IS fatal to the NEXT launch, which will
/// find a burned token and show the card. So it is said, not swallowed.
fn store(identifier: &str, refresh_token: &str) {
    if let Err(e) = keyring_save(identifier, refresh_token) {
        eprintln!("session: refresh token not stored ({e}); the next launch will ask to sign in");
    }
}

/// How far ahead of expiry the proactive rotation fires.
///
/// A tenth of the token's life, capped at five minutes: an hour-long token is
/// replaced five minutes early, a ten-minute one a minute early. Below
/// [`SLACK_FLOOR_SECS`] it is zero — refresh at expiry — so a very short token
/// cannot turn into a rotation loop.
fn slack_for(expires_in: u64) -> Duration {
    if expires_in < SLACK_FLOOR_SECS {
        Duration::ZERO
    } else {
        Duration::from_secs((expires_in / 10).min(SLACK_CAP_SECS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stub `/auth/refresh` that rotates `RT1 -> AT2/RT2 -> AT3/RT3`,
    /// asserting the chain it is presented with. `answers` is how many
    /// rotations the test expects, so the stub thread ends with the test.
    ///
    /// The server is BOUND BY THE CALLER, on the test's own thread: a stub
    /// that binds inside its thread races the first rotation, and a race on a
    /// port is a test that fails once in twenty runs.
    fn serve_rotations(server: tiny_http::Server, answers: usize) {
        for (round, mut rq) in server.incoming_requests().enumerate().take(answers) {
            assert_eq!(rq.url(), "/auth/refresh");
            let mut text = String::new();
            use std::io::Read as _;
            rq.as_reader()
                .read_to_string(&mut text)
                .expect("body reads");
            let v: serde_json::Value = serde_json::from_str(&text).expect("json body");
            // The chain: the presented token is the one the last answer
            // minted, never the one before it — a replay would kill the
            // family server-side.
            assert_eq!(
                v["refresh_token"].as_str(),
                Some(format!("RT{}", round + 1).as_str())
            );
            let body = format!(
                r#"{{"status_code":200,"message":"Successfull","data":{{"access_token":"AT{}","token_type":"Bearer","expires_in":3600,"refresh_token":"RT{}"}}}}"#,
                round + 2,
                round + 2
            );
            let _ = rq.respond(tiny_http::Response::from_string(body));
        }
    }

    /// Wait for a condition the rotation thread sets, without a sleep long
    /// enough to be a test timeout of its own.
    fn until(what: &str, mut ok: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("timed out waiting for {what}");
    }

    fn pair(access: &str, refresh: Option<&str>, expires_in: u64) -> TokenPair {
        TokenPair {
            access_token: access.to_string(),
            token_type: "Bearer".to_string(),
            expires_in,
            refresh_token: refresh.map(str::to_string),
            must_change_password: false,
        }
    }

    #[test]
    fn a_rotation_replaces_the_token_and_advances_the_chain() {
        let server = tiny_http::Server::http(("127.0.0.1", 18120)).expect("bind test port");
        let stub = std::thread::spawn(move || serve_rotations(server, 2));
        let (keeper, events) = TokenKeeper::start("http://127.0.0.1:18120");
        // No identifier: the keyring is not touched (a headless test box may
        // have no secret service, and this test is not about it).
        keeper.install(&pair("AT1", Some("RT1"), 3600), None);
        assert_eq!(keeper.current(), "AT1");
        // The keeper does not emit at install: the caller already holds the
        // token it installed, so a launch is not a "rotation".
        assert!(events.try_recv().is_err(), "install reports nothing");
        // Rotate twice: the stub asserts the SECOND call presents the token
        // the first one minted, which is the whole reason this module owns
        // the chain. (The reactive floor is stepped over the way a minute of
        // uptime would step it: the test has no minute to spend.)
        keeper.rotate();
        assert_eq!(keeper.current(), "AT2");
        if let Ok(mut chain) = keeper.inner.lock() {
            chain.tried_at = None;
        }
        keeper.rotate();
        until("the keeper to adopt the newest token", || {
            keeper.current() == "AT3"
        });
        let mut seen = Vec::new();
        while let Ok(event) = events.try_recv() {
            if let TokenEvent::Rotated { access_token } = event {
                seen.push(access_token);
            }
        }
        assert_eq!(seen, vec!["AT2".to_string(), "AT3".to_string()]);
        stub.join().expect("stub thread");
    }

    #[test]
    fn a_refused_refresh_ends_the_session() {
        let server = tiny_http::Server::http(("127.0.0.1", 18121)).expect("bind test port");
        std::thread::spawn(move || {
            for mut rq in server.incoming_requests().take(1) {
                let _ = rq.respond(tiny_http::Response::from_string(
                    r#"{"status_code":401,"message":"Unauthorized","data":null}"#,
                ));
            }
        });
        let (keeper, events) = TokenKeeper::start("http://127.0.0.1:18121");
        keeper.install(&pair("AT1", Some("RT1"), 3600), None);
        keeper.nudge();
        let event = events
            .recv_timeout(Duration::from_secs(5))
            .expect("an answer");
        assert!(matches!(event, TokenEvent::Expired), "{event:?}");
        assert_eq!(keeper.current(), "", "a dead chain holds no token");
        // And it stays quiet: a refused chain is not retried into a replay.
        assert!(
            events.recv_timeout(Duration::from_millis(300)).is_err(),
            "no further rotations after a refusal"
        );
    }

    #[test]
    fn a_transient_failure_keeps_the_token() {
        // Nothing listens on this port: the rotation is a transport failure,
        // which is not the token's fault.
        let (keeper, events) = TokenKeeper::start("http://127.0.0.1:18122");
        keeper.install(&pair("AT1", Some("RT1"), 3600), None);
        keeper.nudge();
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(keeper.current(), "AT1", "a blip does not sign anybody out");
        assert!(events.try_recv().is_err(), "no rotation to report");
    }

    #[test]
    fn the_slack_is_a_tenth_capped_at_five_minutes() {
        // The numbers the doc promises, so a token's early-rotation moment is
        // never a guess.
        assert_eq!(slack_for(3600), Duration::from_secs(300));
        assert_eq!(slack_for(1000), Duration::from_secs(100));
        assert_eq!(slack_for(600), Duration::from_secs(60));
        assert_eq!(slack_for(59), Duration::ZERO, "too short to be early");
        assert_eq!(slack_for(0), Duration::ZERO);
    }

    #[test]
    fn a_cleared_session_rotates_nothing() {
        let (keeper, events) = TokenKeeper::start("http://127.0.0.1:18123");
        keeper.install(&pair("AT1", Some("RT1"), 3600), None);
        keeper.clear();
        assert_eq!(keeper.current(), "");
        keeper.nudge();
        std::thread::sleep(Duration::from_millis(300));
        assert!(events.try_recv().is_err(), "signed out: nothing to rotate");
    }
}
