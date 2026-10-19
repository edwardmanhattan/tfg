//! The authoritative movement of a Minos exercise.
//!
//! An ordered unit MOVES because the server says where it is. The console
//! never simulates a Minos-driven hull: it reports the position the exercise
//! holds, and the marker follows.
//!
//! **Three sources, one seam.** The server can say where a hull is in three
//! ways, and this module turns all three into the same thing — a
//! [`Fix`] with [`FixSource::Game`]:
//!
//! 1. `GET /games/{id}/positions` — the exercise's derived plot, advanced
//!    along the fix chain to the scenario's now. This is the WORKING source:
//!    it answers continuously, and it is the read the realtime protocol
//!    names as the interim answer (`minos-docs/realtime-protocol.md`,
//!    §`game:<id>:positions`).
//! 2. `game.order_issued` on the `game:<id>` channel — the order's own
//!    position, published the moment the order lands. It is the first word
//!    about where the hull went, and it arrives before any plot read.
//! 3. `game:<id>:positions` — the per-exercise position stream. It is
//!    authorised and specified, and **nothing publishes to it yet** (the
//!    protocol says so, re-checked 2026-09-22). The subscription is declared
//!    anyway: when the server starts publishing, the console is already
//!    listening, and the payload is the agreed shape.
//!
//! **Why the fixes are `Game` and not `Wire`.** The instant on a game fix is
//! the SCENARIO clock, which is routinely hours or days from wall time
//! (`GET /games/{id}/positions?at=`). A wall-clock age computed from it would
//! read as "days old" on a hull that is steaming at this instant. The
//! registry already knows the difference: game fixes are never wall-aged and
//! never counted as missed by the wire's silence.
//!
//! **What is not here.** The console's own simulation ([`crate::sim`]) is a
//! different thing: it drives the ships the console owns in a session of its
//! own, with no game behind them. A Minos exercise is the server's, and this
//! module only reports it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::backend::{GameOrderEvent, GamePositionUpdate, LiveCmd, MinosMaster};
use crate::geo::track::{Fix, FixSource};
use crate::geo::GeoPosition;

/// What the game feed needs to know about the session, and what the binary
/// already knows.
///
/// The cell is shared, not copied: the binary writes it when the session
/// changes (a sign-in, a hold, an advance) and the reader reads it every
/// tick. A copy would be a second answer to "which game is held", and the
/// two would disagree.
#[derive(Debug, Clone, Default)]
pub struct GameHold {
    /// The access token the reads are made with. Empty when signed out.
    pub token: String,
    /// The held game's id. Zero when no game is held.
    pub game_id: i64,
    /// The held game's state as the server last read it: `planning`,
    /// `preparation`, `execution`, `closure`. The positions channel is
    /// watched only in `execution` — before the transition there is no fix
    /// chain, so there is nothing to publish.
    pub state: String,
    /// Bumped to ask for a read NOW. A write that moves a hull (a helm
    /// order) bumps it, so the marker follows the order without waiting out
    /// the poll.
    pub kick: u64,
}

impl GameHold {
    /// Is the exercise running? The plot exists only once the transition
    /// into `execution` has materialised an origin for every placed hull.
    pub fn executing(&self) -> bool {
        self.state == "execution"
    }
}

/// The shared hold cell.
pub type GameHoldCell = Arc<Mutex<GameHold>>;

/// The console's handle on the socket's command channel.
///
/// The channel belongs to the wire that is up NOW: the actor owns the
/// receiving end, so a wire that is swapped for a new one takes a new
/// channel with it. The handle is therefore a cell, and the watches are
/// re-declared on whatever wire is current.
pub type LiveCtl = Arc<Mutex<Option<Sender<LiveCmd>>>>;

/// One authoritative publication, as game fixes.
///
/// The whole round carries ONE instant — the publication's `assumed_time` —
/// because a round is a picture of the exercise at an instant, and stamping
/// its hulls with different instants would make the ordering between them
/// meaningless.
///
/// **The round's instant, and NOT the hull's own.** A position's own
/// `assumed_time` is the instant its LEG began, and a leg begins once: a
/// hull ordered at 13:00 keeps reporting 13:00 for as long as it steers on
/// that leg. Stamping the fixes with it would make every read of a moving
/// hull carry the SAME instant, and the registry — which orders by instant —
/// would accept the first and refuse every one after it. The hull would
/// stand still on the map while the server moved it, which is the one
/// symptom this whole path exists to remove.
pub fn game_fixes(update: &GamePositionUpdate) -> Vec<Fix> {
    update
        .positions
        .iter()
        .map(|p| Fix {
            ship_id: p.unit_id.to_string(),
            position: GeoPosition {
                latitude: p.latitude,
                longitude: p.longitude,
            },
            // The scenario instant of the ROUND — the moment the exercise
            // was at when this picture was taken. The hull's own stamp is
            // the fallback, for a publication that carries only that.
            ts: if update.assumed_time.is_empty() {
                p.assumed_time.clone()
            } else {
                update.assumed_time.clone()
            },
            // The server measured nothing: it DERIVED this position from
            // the leg in force. There is no receipt time, so nothing here is
            // aged against the wall clock.
            received_at: None,
            heading_deg: p.heading_deg.map(|h| h as f32),
            speed_kn: p.speed_kn.map(|s| s as f32),
            accuracy_m: None,
            name: None,
            hull_number: None,
            backfilled: false,
            source: FixSource::Game,
            age_secs: None,
            seq: 0,
        })
        .collect()
}

/// The position an order publication carries, as a game fix.
///
/// The order's answer is a position at the order's own instant — the origin
/// of the new leg. It is authoritative and it is the FIRST word about where
/// the hull went, so it is ingested on the same path as the plot: one
/// vocabulary, one marker.
pub fn order_fix(event: &GameOrderEvent) -> Fix {
    let fix = &event.fix;
    Fix {
        ship_id: fix.unit_id.to_string(),
        position: GeoPosition {
            latitude: fix.latitude,
            longitude: fix.longitude,
        },
        ts: fix.assumed_time.clone(),
        received_at: None,
        heading_deg: Some(fix.heading as f32),
        speed_kn: Some(fix.speed as f32),
        accuracy_m: None,
        name: None,
        hull_number: None,
        backfilled: false,
        source: FixSource::Game,
        age_secs: None,
        seq: 0,
    }
}

/// Write the hold, and say what changed.
///
/// The kick rides along because a session change is a reason to read the
/// plot NOW: a game that has just entered execution has a plot for the
/// first time, and waiting out the poll would show the exercise's hulls
/// arriving late.
pub fn set_hold(hold: &GameHoldCell, token: &str, game_id: i64, state: &str) {
    if let Ok(mut h) = hold.lock() {
        h.token = token.to_string();
        h.game_id = game_id;
        h.state = state.to_string();
        h.kick = h.kick.wrapping_add(1);
    }
}

/// Ask for a plot read now, without changing the session.
pub fn kick(hold: &GameHoldCell) {
    if let Ok(mut h) = hold.lock() {
        h.kick = h.kick.wrapping_add(1);
    }
}

/// Record the state alone, without touching the token or the game.
///
/// The reader uses it: a plot with hulls in it IS the server's word that the
/// exercise is running — the fix chain is materialised at the transition
/// into `execution`, and there is no plot before it.
pub fn set_state(hold: &GameHoldCell, state: &str) {
    if let Ok(mut h) = hold.lock() {
        h.state = state.to_string();
    }
}

/// Declare the socket's game channels for the held session.
///
/// The broadcast channel carries the exercise's messages and its order
/// events; the positions channel carries the authoritative movement stream.
/// The positions channel is declared only in `execution`, because that is
/// when the fix chain exists and there is anything to publish — and because
/// a subscription the server refuses is a log line nobody can act on.
///
/// Re-sending the same declaration is free: the actor compares it with what
/// it holds and does nothing.
pub fn sync_watch(live: &LiveCtl, hold: &GameHoldCell) {
    let (game_id, executing, signed_in) = match hold.lock() {
        Ok(h) => (h.game_id, h.executing(), !h.token.is_empty()),
        Err(_) => return,
    };
    let cmd_tx = match live.lock() {
        Ok(slot) => slot.clone(),
        Err(_) => return,
    };
    let Some(cmd_tx) = cmd_tx else { return };
    let game = if signed_in && game_id != 0 {
        Some(game_id)
    } else {
        None
    };
    let positions = if executing { game } else { None };
    let _ = cmd_tx.send(LiveCmd::WatchGame(game));
    let _ = cmd_tx.send(LiveCmd::WatchGamePositions(positions));
}

/// The plot reader's configuration.
pub struct PlotThreadConfig {
    /// Minos' REST base.
    pub host: String,
    /// The session the reader follows.
    pub hold: GameHoldCell,
    /// Where the game fixes go. The consumer is the registry, which ingests
    /// them on the GAME path — never the wire's.
    pub fix_tx: Sender<Vec<Fix>>,
    /// The socket's command channel, when the socket is up. The reader
    /// declares the positions channel on it the moment the exercise proves
    /// it is running, because a plot with hulls in it is that proof.
    pub live: Option<LiveCtl>,
    /// The console's shutdown flag.
    pub shutdown: Arc<AtomicBool>,
}

/// How often the plot is read while the exercise is running. The plot
/// advances with the scenario clock, so a second is the finest motion an
/// operator can read on a map.
const LIVE_READ: Duration = Duration::from_secs(1);

/// How often it is read while the exercise is not running. Slow enough not
/// to hammer a planning room, fast enough that the transition into
/// execution is noticed within a few seconds.
const IDLE_READ: Duration = Duration::from_secs(5);

/// The longest a failing read waits before trying again.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// How often the reader says what it is reading. A line a second is noise;
/// a line every ten seconds is the answer to "is the exercise moving?".
const LOG_EVERY: Duration = Duration::from_secs(10);

/// Read the exercise's plot, for as long as the console runs.
///
/// The reader is a thread of its own because the console's other thread is a
/// message loop: a read that takes a second would delay every write behind
/// it, and the writes are the operator's.
///
/// **The cadence is the plot's own.** A plot with hulls in it means the
/// exercise is running, and it is read every second. A plot without hulls
/// means it is not (or that nothing is placed), and it is read every five —
/// which is how the transition into execution is discovered without the
/// console being told. A failing read backs off, because a server that is
/// down should not be asked sixty times a minute.
pub fn spawn_plot_thread(cfg: PlotThreadConfig) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let Ok(master) = MinosMaster::new(&cfg.host) else {
            eprintln!("game feed: no Minos client ({}); the plot is not read", cfg.host);
            return;
        };
        // (game, when, the kick that was seen) — the last read, so the
        // cadence and the kick are both judged against it.
        let mut last: Option<(i64, Instant, u64)> = None;
        let mut fails: u32 = 0;
        // Whether the last plot had hulls in it: the exercise is running.
        let mut live = false;
        // When the reader last said so in the log.
        let mut last_logged: Option<Instant> = None;
        loop {
            if cfg.shutdown.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
            if cfg.shutdown.load(Ordering::SeqCst) {
                return;
            }
            let (token, game_id, kick) = match cfg.hold.lock() {
                Ok(h) => (h.token.clone(), h.game_id, h.kick),
                Err(_) => return,
            };
            if token.is_empty() || game_id == 0 {
                // Signed out, or nothing held: there is nothing to read,
                // and the last read is forgotten so a new session starts
                // with a read of its own.
                last = None;
                live = false;
                fails = 0;
                continue;
            }
            let due = match last {
                None => true,
                Some((gid, at, seen)) => {
                    gid != game_id
                        || seen != kick
                        || at.elapsed() >= if live { LIVE_READ } else { IDLE_READ }
                }
            };
            if !due {
                continue;
            }
            let now = Instant::now();
            match master.positions(&token, game_id, None, None) {
                Ok(plot) => {
                    fails = 0;
                    let update = GamePositionUpdate::from_plot(game_id, plot);
                    let fixes = game_fixes(&update);
                    let running = !fixes.is_empty();
                    if running && !live {
                        // The exercise is running — the server says so by
                        // having a plot at all. The positions channel is
                        // declared now, so the console is listening the
                        // moment the server starts publishing on it, even if
                        // another console did the advancing.
                        set_state(&cfg.hold, "execution");
                        if let Some(live) = &cfg.live {
                            sync_watch(live, &cfg.hold);
                        }
                    }
                    live = running;
                    if !fixes.is_empty() && cfg.fix_tx.send(fixes.clone()).is_err() {
                        return; // the console is gone
                    }
                    // A line every ten seconds: the reader is a background
                    // thread, and what it is doing is the thing an operator
                    // asks about ("is the exercise moving?"). Not every
                    // read — a line a second is noise.
                    if running {
                        let now = Instant::now();
                        if last_logged.map(|at| now.duration_since(at) >= LOG_EVERY).unwrap_or(true) {
                            last_logged = Some(now);
                            let first = &fixes[0];
                            eprintln!(
                                "game feed: {} hull(s) @ {} — {} at {:.6}, {:.6} ({}° {} kn)",
                                fixes.len(),
                                update.assumed_time,
                                first.ship_id,
                                first.position.latitude,
                                first.position.longitude,
                                first.heading_deg.unwrap_or_default(),
                                first.speed_kn.unwrap_or_default(),
                            );
                        }
                    }
                    last = Some((game_id, now, kick));
                }
                Err(e) => {
                    fails = fails.saturating_add(1);
                    eprintln!("game feed: plot read failed: {e}");
                    last = Some((game_id, now, kick));
                }
            }
            // A failing read waits longer and longer, up to a minute. The
            // wait is not the interval: the interval is the cadence of a
            // healthy read, and the backoff is added to it.
            if fails > 0 {
                let wait = Duration::from_secs(1u64 << fails.min(6)).min(MAX_BACKOFF);
                std::thread::sleep(wait.min(Duration::from_secs(4)));
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{GameFix, GamePositionFix};

    fn update() -> GamePositionUpdate {
        GamePositionUpdate {
            game_id: 7,
            assumed_time: "2026-10-19T08:00:00Z".to_string(),
            positions: vec![
                GamePositionFix {
                    unit_id: 42,
                    latitude: -6.1047,
                    longitude: 106.9895,
                    heading_deg: Some(77.0),
                    speed_kn: Some(13.1),
                    assumed_time: "2026-10-19T08:00:00Z".to_string(),
                },
                GamePositionFix {
                    unit_id: 43,
                    latitude: -6.2,
                    longitude: 107.1,
                    heading_deg: None,
                    speed_kn: None,
                    assumed_time: "2026-10-19T08:00:00Z".to_string(),
                },
            ],
        }
    }

    #[test]
    fn a_publication_becomes_game_fixes() {
        let fixes = game_fixes(&update());
        assert_eq!(fixes.len(), 2);
        let first = &fixes[0];
        assert_eq!(first.ship_id, "42", "the unit id is the ship id");
        assert_eq!(first.position.latitude, -6.1047);
        assert_eq!(first.heading_deg, Some(77.0));
        assert_eq!(first.speed_kn, Some(13.1));
        assert_eq!(first.source, FixSource::Game, "a game fix, never a wire one");
        // The scenario instant, and it PARSES: the registry drops a fix
        // whose timestamp is not an instant, and a game fix with an
        // unparsable stamp would vanish instead of moving.
        assert!(first.epoch_nanos() > 0, "the scenario stamp is an instant");
        // No receipt time: a game position is derived, not measured, and
        // nothing about it is aged against the wall clock.
        assert!(first.received_at.is_none());
    }

    #[test]
    fn a_round_carries_the_rounds_instant_not_the_legs() {
        // A position's own `assumed_time` is the instant its LEG began, and
        // a leg begins once. A hull ordered at 13:00 reports 13:00 for as
        // long as it steers on that leg, so stamping the fixes with it
        // would make every read carry the SAME instant — and the registry,
        // which orders by instant, would take the first and refuse the rest.
        // The hull would stand still on the map while the server moved it.
        let mut later = update();
        later.assumed_time = "2026-10-19T08:00:10Z".to_string();
        later.positions[0].assumed_time = "2026-10-19T08:00:00Z".to_string();
        let first = game_fixes(&update());
        let second = game_fixes(&later);
        assert_eq!(first[0].ts, "2026-10-19T08:00:00Z", "the round's instant");
        assert_eq!(second[0].ts, "2026-10-19T08:00:10Z", "and the NEXT round's");
        assert!(
            second[0].epoch_nanos() > first[0].epoch_nanos(),
            "a later round is a later instant, so the registry accepts it"
        );
    }

    #[test]
    fn two_rounds_of_a_moving_hull_both_land() {
        // The whole point of the rule above, asserted through the registry
        // that consumes it: a hull that the server is steering must MOVE on
        // the map. Two rounds, one second apart, and the registry's latest
        // fix must be the second one.
        use crate::geo::track::{Registry, TrailBound};
        let mut registry = Registry::new(TrailBound::default());
        let mut first = update();
        first.assumed_time = "2026-10-19T08:00:00Z".to_string();
        first.positions[0].latitude = -6.1047;
        first.positions[0].assumed_time = "2026-10-19T08:00:00Z".to_string();
        let mut second = update();
        second.assumed_time = "2026-10-19T08:00:01Z".to_string();
        second.positions[0].latitude = -6.1040;
        second.positions[0].assumed_time = "2026-10-19T08:00:00Z".to_string();
        registry.poll_game(game_fixes(&first));
        registry.poll_game(game_fixes(&second));
        let ship = registry.ship("42").expect("the hull is tracked");
        assert_eq!(
            ship.latest.position.latitude, -6.1040,
            "the later round is the hull's position, not the first one's"
        );
    }

    #[test]
    fn an_order_publication_is_a_position_too() {
        let event = GameOrderEvent {
            game_id: 7,
            fix: GameFix {
                unit_id: 42,
                assumed_time: "2026-10-19T08:00:00Z".to_string(),
                latitude: -6.1047,
                longitude: 106.9895,
                heading: 77.0,
                speed: 13.1,
                requested_speed: Some(20.0),
                clamped: true,
                created_at: None,
                created_by: None,
            },
        };
        let fix = order_fix(&event);
        assert_eq!(fix.ship_id, "42");
        assert_eq!(fix.source, FixSource::Game);
        assert_eq!(fix.heading_deg, Some(77.0));
        assert_eq!(fix.speed_kn, Some(13.1));
        assert!(fix.epoch_nanos() > 0);
    }

    #[test]
    fn the_hold_remembers_the_session_and_its_state() {
        let cell: GameHoldCell = Arc::new(Mutex::new(GameHold::default()));
        set_hold(&cell, "tok", 7, "execution");
        let h = cell.lock().unwrap();
        assert_eq!(h.token, "tok");
        assert_eq!(h.game_id, 7);
        assert!(h.executing());
        assert_eq!(h.kick, 1, "a session change asks for a read");
        drop(h);
        kick(&cell);
        assert_eq!(cell.lock().unwrap().kick, 2);
    }

    #[test]
    fn the_watches_follow_the_session() {
        let cell: GameHoldCell = Arc::new(Mutex::new(GameHold::default()));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let live: LiveCtl = Arc::new(Mutex::new(Some(cmd_tx)));
        // Nothing held: nothing is watched.
        sync_watch(&live, &cell);
        assert!(matches!(cmd_rx.try_recv(), Ok(LiveCmd::WatchGame(None))));
        assert!(matches!(cmd_rx.try_recv(), Ok(LiveCmd::WatchGamePositions(None))));
        // A held game: the broadcast channel, and NOT the positions channel
        // before the exercise runs.
        set_hold(&cell, "tok", 7, "planning");
        sync_watch(&live, &cell);
        assert!(matches!(cmd_rx.try_recv(), Ok(LiveCmd::WatchGame(Some(7)))));
        assert!(matches!(cmd_rx.try_recv(), Ok(LiveCmd::WatchGamePositions(None))));
        // In execution: both.
        set_hold(&cell, "tok", 7, "execution");
        sync_watch(&live, &cell);
        assert!(matches!(cmd_rx.try_recv(), Ok(LiveCmd::WatchGame(Some(7)))));
        assert!(matches!(
            cmd_rx.try_recv(),
            Ok(LiveCmd::WatchGamePositions(Some(7)))
        ));
        // A signed-out console watches nothing, even with a game in hand.
        set_hold(&cell, "", 7, "execution");
        sync_watch(&live, &cell);
        assert!(matches!(cmd_rx.try_recv(), Ok(LiveCmd::WatchGame(None))));
        assert!(matches!(cmd_rx.try_recv(), Ok(LiveCmd::WatchGamePositions(None))));
    }
}
