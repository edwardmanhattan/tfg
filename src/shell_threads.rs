//! Shell threads shared by both shells.
//!
//! U2 slice: the poll thread moves here verbatim from the egui binary's
//! `fn main`, so the shipped shell and the GPUI shell (`proto/gpui-shell`)
//! call one spawn function instead of keeping two copies. It is toolkit-free
//! — channels of [`Fix`] — which is the property that lets it live in the
//! lib at all.
//!
//! The map thread stays in the egui binary for now, deliberately. `MapResp`
//! carries `egui::ColorImage` (`main.rs`), and moving the spawn would drag
//! egui into the lib through the channel type. U6 replaces the handoff with
//! a raw frame type plus per-shell conversion, and the spawn follows then.
//!
//! Behaviour is unchanged: boot wire from `TFG_BACKEND_URL` / `TFG_SCENARIO`
//! env, runtime swaps over `wire_ctl`, 20x100ms sleep with shutdown check
//! and the live-wire fast lane, `UI gone` exits on dropped receivers.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::assets::scenario_json;
use crate::backend::{
    BackendError, FileReplay, LiveCmd, LiveEvent, LiveWire, MinosRest, MockPoll, PollSource,
};
use crate::geo::track::Fix;
use crate::log::Journal;
use crate::sim::{MergeSource, SimCommand, SimEvent, SimSource};

/// Runtime wire backends for the poll thread (task #39). Replay stays
/// boot-only (env).
pub enum WireKind {
    /// Explicit mock source (M10): examples/mock_backend.rs over
    /// TFG_BACKEND_URL. Never a Minos route — Minos is Live below.
    Mock(String),
    /// A built-in scenario embedded in the executable. The value is the
    /// scenario stem accepted by [`scenario_json`].
    EmbeddedReplay(String),
    Live {
        ws_url: String,
        rest_base: String,
        token: String,
        cmd_tx: Sender<LiveCmd>,
        cmd_rx: Receiver<LiveCmd>,
        event_tx: Sender<LiveEvent>,
        wake_tx: Sender<()>,
    },
    Empty,
}

/// Wire source that yields nothing: a disconnected presentation, or a
/// simulation running on sim data alone.
struct EmptyWire;

impl PollSource for EmptyWire {
    fn poll(&mut self) -> Result<Vec<Fix>, BackendError> {
        Ok(Vec::new())
    }
}

/// Build a wire source plus its status description. Live failures fall
/// back to an empty wire with the reason (feed-down reads as stale).
fn build_wire(kind: WireKind) -> Result<(Box<dyn PollSource>, String), String> {
    match kind {
        WireKind::Mock(url) => MockPoll::new(&url)
            .map(|h| (Box::new(h) as Box<dyn PollSource>, format!("mock {url}")))
            .map_err(|e| e),
        WireKind::EmbeddedReplay(name) => {
            let json = scenario_json(&name)
                .ok_or_else(|| format!("unknown built-in scenario {name}"))?;
            FileReplay::from_json(json)
                .map(|r| (Box::new(r) as Box<dyn PollSource>, format!("replay {name} (embedded)")))
                .map_err(|e| format!("embedded scenario {name}: {e}"))
        }
        WireKind::Live { ws_url, rest_base, token, cmd_tx, cmd_rx, event_tx, wake_tx } => {
            let rest = MinosRest::new(&rest_base).map_err(|e| e)?;
            match LiveWire::connect(&ws_url, &rest, &token, cmd_tx, cmd_rx, event_tx, wake_tx) {
                Ok(w) => Ok((Box::new(w) as Box<dyn PollSource>, format!("live {ws_url}"))),
                Err(e) => Ok((
                    Box::new(EmptyWire) as Box<dyn PollSource>,
                    format!("live failed ({e}); empty"),
                )),
            }
        }
        WireKind::Empty => Ok((Box::new(EmptyWire) as Box<dyn PollSource>, "empty".to_string())),
    }
}

/// Everything the poll thread takes ownership of. Plain channels and flags;
/// the caller keeps the UI-side ends it needs (`poll_rx`, `sim_cmd_tx`,
/// `sim_evt_rx`, the `wire_ctl`/`wake` senders, `shutdown`, `armed`).
pub struct PollThreadConfig {
    pub wire_ctl_rx: Receiver<WireKind>,
    pub wire_wake_rx: Receiver<()>,
    pub sim_cmd_rx: Receiver<SimCommand>,
    pub sim_evt_tx: Sender<SimEvent>,
    pub poll_tx: Sender<Vec<Fix>>,
    pub shutdown: Arc<AtomicBool>,
    pub armed: Arc<AtomicBool>,
    pub initial_log: PathBuf,
}

/// Spawn the backend poll loop. The UI owns the registry; this thread owns
/// the source and feeds mock fixes at the v0 cadence, with the sim joining
/// every round via `MergeSource` (disarmed = wire only).
pub fn spawn_poll_thread(cfg: PollThreadConfig) -> JoinHandle<()> {
    std::thread::spawn(move || {
        // Boot wire (task #39, M10): TFG_BACKEND_URL points at the
        // explicit mock (examples/mock_backend.rs) — never at Minos,
        // which is the Live wire below. Replay data is embedded; an
        // unknown scenario fails rather than reading a local file.
        let boot_kind = match std::env::var("TFG_BACKEND_URL") {
            Ok(url) => WireKind::Mock(url),
            Err(_) => match std::env::var("TFG_SCENARIO") {
                Ok(name) => WireKind::EmbeddedReplay(name),
                Err(_) => WireKind::EmbeddedReplay("empty".to_string()),
            },
        };
        let (wire, desc) = match build_wire(boot_kind) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("backend failed to start: {e}");
                return;
            }
        };
        eprintln!("backend: {desc}");
        let mut source = MergeSource::new(
            wire,
            SimSource::new_with_journal(
                cfg.sim_cmd_rx,
                cfg.sim_evt_tx,
                Journal::open(cfg.initial_log).unwrap_or_else(|e| {
                    eprintln!("session log disabled: {e}");
                    Journal::disabled()
                }),
            ),
        );
        source.set_armed_flag(cfg.armed);
        loop {
            // Runtime wire swaps from the Connection island (task #39).
            while let Ok(kind) = cfg.wire_ctl_rx.try_recv() {
                match build_wire(kind) {
                    Ok((w, desc)) => {
                        eprintln!("backend: {desc}");
                        source.set_wire(w);
                    }
                    Err(e) => eprintln!("wire swap failed: {e}"),
                }
            }
            match source.poll() {
                Ok(fixes) => {
                    if cfg.poll_tx.send(fixes).is_err() {
                        return; // UI gone
                    }
                }
                Err(e) => eprintln!("poll failed (ships keep misses): {e}"),
            }
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(100));
                if cfg.shutdown.load(Ordering::SeqCst) {
                    return;
                }
                // Live-wire fast lane: a queued socket publication cuts the
                // 2 s sleep short, so the marker moves in ~100 ms. The sim
                // cadence is untouched — this only skips idle sleeping.
                if cfg.wire_wake_rx.try_iter().count() > 0 {
                    break;
                }
            }
        }
    })
}
