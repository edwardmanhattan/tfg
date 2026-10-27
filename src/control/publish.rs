//! The console's projection of the exercise, published to the control plane.
//!
//! The shell knows the roster; the binary's fix registry knows where the hulls
//! are. Neither is the whole of what a viewer needs, and they live on different
//! threads, so this module is the seam that joins them.
//!
//! Two writers, one thread:
//!
//! - the shell sends a [`SessionUpdate::Roster`] at most once a second, when the
//!   exercise it holds actually changed;
//! - the fix feed sends [`SessionUpdate::Positions`] as often as it likes, and
//!   the publisher keeps only the newest fix per unit.
//!
//! The publisher composes them, compares with what the sink already has, and
//! writes only on a change — plus a heartbeat every [`HEARTBEAT`], which is what
//! keeps the session from going stale while an exercise sits still.
//!
//! Two sinks, because a console can be either end of the control plane:
//!
//! - [`SessionSink::Hub`] when this console *is* the host: the projection goes
//!   straight into the shared state, no socket involved;
//! - [`SessionSink::Http`] when it follows another host: the projection is a
//!   `PUT /v1/session`, the same request `tfgctl` makes from a fixture.
//!
//! The rule that shapes all of it: **this module never reads Minos.** The shell
//! hands it a `SessionState`, which is what the console already had; a second
//! Minos reader in the control plane is the thing this design exists to avoid.

use super::hub::ControlHub;
use super::protocol::{GeoPoint, SessionState, UnitState};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// How often the projection is re-sent even when nothing changed.
///
/// The server's session TTL is fifteen seconds; five gives two missed beats of
/// margin before a phone is told the console went quiet.
pub const HEARTBEAT: Duration = Duration::from_secs(5);

/// How long the publisher waits for work before checking the clock.
///
/// Small enough that a heartbeat lands on time, large enough that an idle
/// console does nothing four times a second.
const TICK: Duration = Duration::from_millis(250);

/// How often a repeated sink error is written to the log.
const ERROR_LOG_INTERVAL: Duration = Duration::from_secs(10);

/// Where one hull is, as the console's fix registry holds it.
///
/// Not [`super::protocol::UnitState`]: the wire type carries the whole picture
/// of a hull, and a fix is only the part that changes once a second.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitPosition {
    pub latitude: f64,
    pub longitude: f64,
    pub heading_deg: Option<f64>,
    pub speed_kn: Option<f64>,
    pub stale: bool,
}

/// One thing the publisher learned.
pub enum SessionUpdate {
    /// The exercise the console holds, or `None` when it holds none.
    ///
    /// Positions in this projection are the exercise's recorded placements —
    /// where a hull starts — and the publisher replaces them with live fixes
    /// as soon as the registry has one.
    Roster(Box<Option<SessionState>>),
    /// The newest fix for each hull named, keyed by unit id.
    Positions(Vec<(i64, UnitPosition)>),
}

/// Where a projection goes.
pub enum SessionSink {
    /// This console is the host: write the shared state directly.
    Hub(ControlHub),
    /// This console follows a host: `PUT /v1/session`.
    Http { server: String, token: String },
    /// No control plane is configured. Publishing would be a thread writing to
    /// nobody, so there is no thread.
    Off,
}

impl SessionSink {
    pub fn http(server: impl Into<String>, token: impl Into<String>) -> Self {
        Self::Http {
            server: server.into().trim_end_matches('/').to_string(),
            token: token.into(),
        }
    }

    fn is_off(&self) -> bool {
        matches!(self, Self::Off)
    }
}

/// The publisher thread's handle. Cheap to clone the sender out of; the thread
/// stops when every handle is dropped.
pub struct SessionHandle {
    /// `None` when the sink is [`SessionSink::Off`]: there is nothing to send
    /// to and nothing to start.
    tx: Option<Sender<SessionUpdate>>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl SessionHandle {
    /// A handle that publishes nowhere. Used by `tfg-server`, which reads
    /// sessions from HTTP rather than holding one, and by tests.
    pub fn off() -> Self {
        Self {
            tx: None,
            stop: Arc::new(AtomicBool::new(true)),
            thread: Mutex::new(None),
        }
    }

    /// The exercise the console holds changed (or it stopped holding one).
    pub fn roster(&self, session: Option<SessionState>) {
        self.send(SessionUpdate::Roster(Box::new(session)));
    }

    /// New fixes, keyed by unit id. The publisher keeps the newest per hull.
    pub fn positions(&self, positions: Vec<(i64, UnitPosition)>) {
        if positions.is_empty() {
            return;
        }
        self.send(SessionUpdate::Positions(positions));
    }

    fn send(&self, update: SessionUpdate) {
        if let Some(tx) = &self.tx {
            // A closed channel means the thread is gone, which is what a sink
            // that failed to start looks like. Publishing is best-effort by
            // design: a viewer that misses an update gets the next heartbeat.
            let _ = tx.send(update);
        }
    }

    pub fn is_publishing(&self) -> bool {
        self.tx.is_some()
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.lock().ok().and_then(|mut slot| slot.take()) {
            let _ = thread.join();
        }
    }
}

/// Start the publisher. `Off` returns a handle with no thread at all.
pub fn spawn_session_publisher(sink: SessionSink) -> Result<SessionHandle, String> {
    if sink.is_off() {
        return Ok(SessionHandle::off());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("tfg-session-publish".to_string())
        .spawn(move || Publisher::new(sink).run(rx, &thread_stop))
        .map_err(|error| format!("session publisher thread: {error}"))?;
    Ok(SessionHandle {
        tx: Some(tx),
        stop,
        thread: Mutex::new(Some(thread)),
    })
}

/// The composed state, and what the sink already has of it.
struct Publisher {
    sink: Sink,
    roster: Option<SessionState>,
    positions: HashMap<i64, UnitPosition>,
    /// Whether the shell has ever told this publisher what it holds.
    ///
    /// A publisher starts knowing nothing, and knowing nothing is not the
    /// claim "there is no exercise": clearing on start-up would delete the
    /// session another console published. Only a roster message — including
    /// the `None` that says this console has let go — authorises a write.
    told: bool,
    /// What the sink was last told. `None` means the sink has no session.
    sent: Option<Option<SessionState>>,
    sent_at: Option<Instant>,
    log: FailureLog,
}

impl Publisher {
    fn new(sink: SessionSink) -> Self {
        let sink = match sink {
            SessionSink::Hub(hub) => Sink::Hub(hub),
            SessionSink::Http { server, token } => match Sink::http(&server, &token) {
                Ok(sink) => sink,
                Err(error) => {
                    eprintln!("session publisher disabled: {error}");
                    Sink::Dead(error)
                }
            },
            SessionSink::Off => Sink::Dead("no control plane configured".to_string()),
        };
        Self {
            sink,
            roster: None,
            positions: HashMap::new(),
            told: false,
            sent: None,
            sent_at: None,
            log: FailureLog::default(),
        }
    }

    fn run(mut self, updates: Receiver<SessionUpdate>, stop: &AtomicBool) {
        while !stop.load(Ordering::SeqCst) {
            match updates.recv_timeout(TICK) {
                Ok(update) => {
                    self.apply(update);
                    // A burst becomes one write: everything that arrived while
                    // this thread was awake is already in `self`.
                    while let Ok(update) = updates.try_recv() {
                        self.apply(update);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                // The channel closed under us. `Drop` on the handle is the
                // usual way, and it also sets `stop` — the clean-up after the
                // loop is what both paths share, so neither can skip it.
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            self.publish_if_due();
        }
        // The console is closing. Say so, once, rather than leaving a phone
        // waiting on a session that went stale fifteen seconds later. Best
        // effort: a host that has already gone away has nothing to clear.
        if self.sent.as_ref().is_some_and(Option::is_some) {
            if let Err(error) = self.sink.clear() {
                self.log.log(&error);
            }
        }
    }

    fn apply(&mut self, update: SessionUpdate) {
        match update {
            SessionUpdate::Roster(session) => {
                self.roster = *session;
                self.told = true;
                if self.roster.is_none() {
                    // Stale fixes for an exercise that ended would be a picture
                    // of the last one.
                    self.positions.clear();
                }
            }
            SessionUpdate::Positions(positions) => {
                for (id, fix) in positions {
                    self.positions.insert(id, fix);
                }
            }
        }
    }

    fn publish_if_due(&mut self) {
        if !self.told {
            // The shell has said nothing yet. Saying "no session" on its behalf
            // would be a claim about an exercise this console never held.
            return;
        }
        let composed = self.roster.as_ref().map(|roster| compose(roster, &self.positions));
        // Only a live session needs a heartbeat. Repeating a clear would be a
        // DELETE every five seconds — and after this console let go, deleting
        // again could take down the session another console has since
        // published.
        let due = composed.is_some() && self.sent_at.is_none_or(|at| at.elapsed() >= HEARTBEAT);
        let changed = self.sent.as_ref() != Some(&composed);
        if !changed && !due {
            return;
        }
        let result = match &composed {
            Some(session) => self.sink.publish(session),
            None => self.sink.clear(),
        };
        match result {
            Ok(()) => {
                self.sent = Some(composed);
                self.sent_at = Some(Instant::now());
                self.log.recovered();
            }
            Err(error) => {
                // The state is NOT recorded as sent: the next tick retries,
                // because a console that cannot reach its viewer should keep
                // trying rather than settle into being wrong.
                self.log.log(&error);
            }
        }
    }
}

/// Roster + fixes, with the numbers the viewer draws.
///
/// This is the one place that decides what a hull's position is: a live fix
/// when the registry has one, the recorded placement otherwise. A placement is
/// not a live fix, so a hull that has never been plotted is marked `stale` —
/// "this is where the exercise put it, nobody has seen it move".
fn compose(roster: &SessionState, positions: &HashMap<i64, UnitPosition>) -> SessionState {
    let mut session = roster.clone();
    for (index, unit) in session.units.iter_mut().enumerate() {
        unit.index = u32::try_from(index + 1).unwrap_or(u32::MAX);
        live(unit, positions.get(&unit.id).copied());
    }
    session
}

fn live(unit: &mut UnitState, fix: Option<UnitPosition>) {
    match fix {
        Some(fix) => {
            unit.position = Some(GeoPoint {
                latitude: fix.latitude,
                longitude: fix.longitude,
            });
            unit.heading_deg = fix.heading_deg;
            unit.speed_kn = fix.speed_kn;
            unit.stale = fix.stale;
        }
        None => {
            // No fix at all. The placement the shell sent stands, and it is
            // not a live picture of the hull.
            unit.heading_deg = None;
            unit.speed_kn = None;
            unit.stale = unit.position.is_some();
        }
    }
}

enum Sink {
    Hub(ControlHub),
    Http {
        http: reqwest::blocking::Client,
        url: String,
        token: String,
    },
    /// The sink could not be built. Every attempt reports the reason rather
    /// than pretending to publish.
    Dead(String),
}

impl Sink {
    fn http(server: &str, token: &str) -> Result<Self, String> {
        if token.trim().is_empty() {
            return Err("the control token is empty".to_string());
        }
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|error| format!("HTTP client: {error}"))?;
        Ok(Self::Http {
            http,
            url: format!("{server}/v1/session"),
            token: token.to_string(),
        })
    }

    fn publish(&self, session: &SessionState) -> Result<(), String> {
        match self {
            Self::Hub(hub) => {
                // The hub owns the heartbeat: a publish equal to what it holds
                // refreshes the session's clock and publishes no event, which is
                // exactly what a heartbeat should be.
                hub.set_session(session.clone());
                Ok(())
            }
            Self::Http { http, url, token } => {
                let response = http
                    .put(url)
                    .bearer_auth(token)
                    .json(session)
                    .send()
                    .map_err(|error| format!("publishing the session: {error}"))?;
                let status = response.status();
                if status.is_success() {
                    return Ok(());
                }
                let body = response.text().unwrap_or_default();
                Err(format!(
                    "publishing the session: HTTP {status} {}",
                    body.chars().take(160).collect::<String>()
                ))
            }
            Self::Dead(reason) => Err(reason.clone()),
        }
    }

    fn clear(&self) -> Result<(), String> {
        match self {
            Self::Hub(hub) => {
                hub.clear_session();
                Ok(())
            }
            Self::Http { http, url, token } => {
                let response = http
                    .delete(url)
                    .bearer_auth(token)
                    .send()
                    .map_err(|error| format!("clearing the session: {error}"))?;
                let status = response.status();
                if status.is_success() {
                    return Ok(());
                }
                Err(format!("clearing the session: HTTP {status}"))
            }
            Self::Dead(reason) => Err(reason.clone()),
        }
    }
}

/// Rate-limited stderr: a viewer that is down should not write a line four
/// times a second.
#[derive(Default)]
struct FailureLog {
    last: Option<Instant>,
    suppressed: u32,
}

impl FailureLog {
    fn log(&mut self, message: &str) {
        let due = self
            .last
            .is_none_or(|at| at.elapsed() >= ERROR_LOG_INTERVAL);
        if !due {
            self.suppressed += 1;
            return;
        }
        if self.suppressed > 0 {
            eprintln!(
                "session publisher: {message} ({} similar messages suppressed)",
                self.suppressed
            );
        } else {
            eprintln!("session publisher: {message}");
        }
        self.last = Some(Instant::now());
        self.suppressed = 0;
    }

    fn recovered(&mut self) {
        self.last = None;
        self.suppressed = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::protocol::{ScenarioState, CommanderState};

    fn unit(id: i64, name: &str, latitude: Option<f64>) -> UnitState {
        UnitState {
            index: 0,
            id,
            name: name.to_string(),
            hull_number: format!("H{id}"),
            class_name: "Frigate".to_string(),
            commander: None,
            embarked: Vec::new(),
            embarked_in: None,
            position: latitude.map(|latitude| GeoPoint {
                latitude,
                longitude: 106.8,
            }),
            heading_deg: None,
            speed_kn: None,
            stale: false,
        }
    }

    fn roster() -> SessionState {
        SessionState {
            game_id: 42,
            name: "Exercise".to_string(),
            phase: "execution".to_string(),
            mode: "maneuver".to_string(),
            scenario: Some(ScenarioState {
                id: 7,
                title: "Sunda Strait".to_string(),
                step: Some("Step 3".to_string()),
            }),
            units: vec![unit(9, "Evertsen", Some(-6.2)), unit(4, "Tromp", None)],
        }
    }

    #[test]
    fn a_live_fix_replaces_the_recorded_placement() {
        let mut positions = HashMap::new();
        positions.insert(
            9,
            UnitPosition {
                latitude: -6.1,
                longitude: 106.7,
                heading_deg: Some(72.0),
                speed_kn: Some(18.0),
                stale: false,
            },
        );
        let composed = compose(&roster(), &positions);
        let evertsen = &composed.units[0];
        assert_eq!(evertsen.position.expect("a position").latitude, -6.1);
        assert_eq!(evertsen.heading_deg, Some(72.0));
        assert_eq!(evertsen.speed_kn, Some(18.0));
        assert!(!evertsen.stale);
    }

    #[test]
    fn a_placement_without_a_fix_is_stale_and_a_hull_with_neither_is_not() {
        let composed = compose(&roster(), &HashMap::new());
        // Evertsen has a placement: where the exercise put it, not where
        // anybody has seen it.
        assert!(composed.units[0].stale);
        assert!(composed.units[0].position.is_some());
        // Tromp has neither. `stale` would be a claim about a fix that was
        // never expected.
        assert!(!composed.units[1].stale);
        assert!(composed.units[1].position.is_none());
    }

    #[test]
    fn unit_numbers_come_from_the_roster_order() {
        let composed = compose(&roster(), &HashMap::new());
        assert_eq!(composed.units[0].index, 1);
        assert_eq!(composed.units[1].index, 2);
        assert_eq!(composed.units[0].id, 9);
    }

    #[test]
    fn a_commander_survives_the_round_trip() {
        let mut roster = roster();
        roster.units[0].commander = Some(CommanderState {
            id: 5,
            name: "Cdr Okafor".to_string(),
        });
        let composed = compose(&roster, &HashMap::new());
        assert_eq!(
            composed.units[0].commander.as_ref().map(|c| c.name.as_str()),
            Some("Cdr Okafor")
        );
    }

    /// The composition the publisher actually writes, without a network: a hub
    /// is a sink like any other.
    #[test]
    fn a_publisher_writes_the_roster_and_its_fixes_into_the_hub() {
        let hub = ControlHub::new(crate::control::CameraSnapshot::from_tuple((
            -6.2, 106.8, 11.0, 800, 600,
        )));
        let handle = spawn_session_publisher(SessionSink::Hub(hub.clone())).expect("publisher");
        assert!(handle.is_publishing());
        handle.roster(Some(roster()));
        handle.positions(vec![(
            9,
            UnitPosition {
                latitude: -6.05,
                longitude: 106.75,
                heading_deg: Some(70.0),
                speed_kn: Some(17.0),
                stale: false,
            },
        )]);
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(3) {
            if hub.session().is_some_and(|session| {
                session
                    .units
                    .first()
                    .and_then(|unit| unit.position)
                    .is_some_and(|position| (position.latitude + 6.05).abs() < 1e-9)
            }) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the hub never received the composition");
    }

    /// A publisher starts knowing nothing, and "knowing nothing" must not be
    /// sent as "there is no exercise": another console may have published
    /// one, and a console that opens beside it would delete it.
    #[test]
    fn a_publisher_that_was_never_told_anything_clears_nothing() {
        let hub = ControlHub::new(crate::control::CameraSnapshot::from_tuple((
            -6.2, 106.8, 11.0, 800, 600,
        )));
        hub.set_session(roster());
        let handle = spawn_session_publisher(SessionSink::Hub(hub.clone())).expect("publisher");
        // Fixes arriving before any roster are still nothing to publish.
        handle.positions(vec![(
            9,
            UnitPosition {
                latitude: -6.05,
                longitude: 106.75,
                heading_deg: None,
                speed_kn: None,
                stale: false,
            },
        )]);
        std::thread::sleep(Duration::from_millis(600));
        assert!(
            hub.session().is_some(),
            "a publisher that was never told anything deleted another console's session"
        );
    }

    #[test]
    fn an_off_sink_starts_no_thread() {
        let handle = spawn_session_publisher(SessionSink::Off).expect("no sink");
        assert!(!handle.is_publishing());
        // And the calls are cheap no-ops rather than panics.
        handle.roster(Some(roster()));
        handle.positions(vec![(
            1,
            UnitPosition {
                latitude: 0.0,
                longitude: 0.0,
                heading_deg: None,
                speed_kn: None,
                stale: false,
            },
        )]);
    }

    /// The sink a console that FOLLOWS another console uses: a real HTTP host,
    /// a real `PUT /v1/session`, and the same hub a hosting console would fill
    /// directly. What this catches is everything the unit tests above cannot:
    /// the URL, the bearer token, the route, and the shape the server parses.
    #[test]
    fn an_http_sink_publishes_to_a_host_and_clears_on_the_way_out() {
        const TOKEN: &str = "0123456789abcdef0123456789abcdef";
        let hub = ControlHub::new(crate::control::CameraSnapshot::from_tuple((
            -6.2, 106.8, 11.0, 800, 600,
        )));
        let (commands, _command_rx) = std::sync::mpsc::channel();
        let server = crate::control::spawn_server(
            crate::control::ServerConfig::new("127.0.0.1:0", TOKEN),
            commands,
            hub.clone(),
        )
        .expect("server starts");
        let handle = spawn_session_publisher(SessionSink::http(server.url(), TOKEN))
            .expect("publisher starts");
        handle.roster(Some(roster()));

        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) && hub.session().is_none() {
            std::thread::sleep(Duration::from_millis(20));
        }
        let published = hub.session().expect("the host received the session");
        assert_eq!(published.game_id, 42);
        assert_eq!(published.units.len(), 2);

        // Dropping the last handle is the console closing: the publisher says
        // so rather than leaving a phone waiting on a session that will only
        // time out fifteen seconds later.
        drop(handle);
        assert!(hub.session().is_none(), "the publisher did not clear the session");
    }
}
