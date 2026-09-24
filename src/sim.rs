//! Simulation source (prototype, map #10 ticket #16).
//!
//! The sim IS a poll source (ADR-0003): each round it advances owned ships
//! from their orders and emits synthetic fixes. The registry never sees
//! orders. UI talks to the sim over channels: [`SimCommand`] down,
//! [`SimEvent`] up. [`MergeSource`] joins wire + sim rounds, suppressing
//! wire fixes for sim-owned ships.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

use crate::backend::{BackendError, PollSource, now_ts};
use crate::catalog::{Catalog, Class};
use crate::clock::GameClock;
use crate::command::{Authority, Grant, GrantDenial, MoveCommand, Verb};
use crate::log::{Journal, LogKind};
use crate::geo::coordinates::GeoPosition;
use crate::geo::track::{Fix, FixSource};
use crate::land::{Land, PATH_SAMPLE_M};

/// Real seconds between sim rounds (the poll cadence). Motion itself
/// advances by GAME time: game_dt = real_dt × ratio (grill #17, ADR-0004).
pub const SIM_TICK_SECS: f64 = 2.0;
/// Distance to a waypoint that counts as arrived.
pub const ARRIVAL_M: f64 = 50.0;

/// Stop at the first sampled land crossing instead of leaving a local
/// helm at the previous tick position. The land model is polygonal, so
/// the final short bisection is sandbox geometry, not a MinOS fact.
fn stop_at_first_land(
    land: &Land,
    from: GeoPosition,
    to: GeoPosition,
    heading_deg: f32,
    speed_kn: f32,
    game_dt: f64,
) -> GeoPosition {
    if !land.is_water(&from) {
        return from;
    }
    if land.path_is_water(&from, &to) {
        return to;
    }
    let steps = (from.distance_m(&to) / PATH_SAMPLE_M).ceil().max(1.0) as usize;
    for step in 1..=steps {
        let fraction = step as f64 / steps as f64;
        let probe = from.dead_reckon(heading_deg, speed_kn, game_dt * fraction);
        if !land.is_water(&probe) {
            let mut water = (step - 1) as f64 / steps as f64;
            let mut landward = fraction;
            for _ in 0..12 {
                let middle = (water + landward) / 2.0;
                let candidate = from.dead_reckon(heading_deg, speed_kn, game_dt * middle);
                if land.is_water(&candidate) {
                    water = middle;
                } else {
                    landward = middle;
                }
            }
            return from.dead_reckon(heading_deg, speed_kn, game_dt * landward);
        }
    }
    to
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderState {
    EnRoute,
    /// Path crossed land: ship stopped at the coast, order preserved.
    Blocked,
    Arrived,
    Holding,
}

#[derive(Debug, Clone)]
pub struct Order {
    pub waypoint: GeoPosition,
    pub speed_kn: f32,
    pub state: OrderState,
}

/// A local sandbox HelmOrder. It is deliberately separate from the
/// legacy waypoint Order so the sandbox can preserve old scenarios while
/// exposing the new persistent heading/speed contract.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HelmOrder {
    pub heading_deg: f32,
    pub speed_kn: f32,
    pub blocked: bool,
}

/// UI -> sim.
#[derive(Debug, Clone)]
pub enum SimCommand {
    /// Take control with the class whose stats will drive the unit
    /// (grill #18: class holds abilities; chosen at takeover).
    /// Unknown ids are refused, never fallen back (H10): inventing a
    /// ship's abilities is worse than standing it down.
    TakeControl { ship_id: String, pos: GeoPosition, class_id: String },
    /// Push Minos-synced figures into the sim's catalog (H10): the sim
    /// owns a catalog of its own, and UI-side runtime classes would
    /// otherwise resolve to nothing here. Same namespaced shape as
    /// [`Catalog::upsert_runtime_class`](crate::catalog::Catalog::upsert_runtime_class).
    UpsertClass {
        minos_class_id: i64,
        name: String,
        version: i64,
        speed_kn: f64,
        cruise_kn: f64,
        range_nm: f64,
    },
    Release { ship_id: String },
    SetOrder { ship_id: String, waypoint: GeoPosition, speed_kn: f32 },
    /// Persistent local helm setpoint. Unlike legacy SetOrder, this has
    /// no waypoint and remains in force until replaced.
    SetHelm {
        ship_id: String,
        heading_deg: f32,
        speed_kn: f32,
        authority: Authority,
        grant: Grant,
    },
    CancelOrder { ship_id: String },
    /// Multi-unit move (precedence grill, #19): fanned out to per-ship
    /// orders under grant + authority checks. The sim never sees commands
    /// except through this variant.
    OrderMove { command: MoveCommand },
    /// Pause is a full hold: motion + game time freeze, wall clock runs on.
    SetPaused { paused: bool },
    /// Session clock pace (session flow): fixed-ratio mapping real to
    /// game seconds (ADR-0004). The organizer sets it at session start.
    SetClockRatio { ratio: f64 },
    /// Scenario epoch seed (participant-correct map): the Minos
    /// assumed_start, so the display readout anchors on scenario time
    /// instead of the wall-clock first tick. Idempotent — re-seeding
    /// the same epoch changes nothing; local elapsed keeps
    /// accumulating on top, which is exactly the projection's
    /// contract (never the server's integral).
    SeedClockStart { ts: String },
    /// Ingest ack (Log grill, #20): the UI reports back each fix's
    /// Registry-stamped seq after ingest, so journal entries can cite
    /// fix seqs. Reuses the command channel; no new plumbing.
    FixAck { ship_id: String, seq: u64 },
    /// Rotate the journal to a fresh per-session file (state-machine
    /// grill, #26): Start opens a new file, the old one stays on disk.
    RotateJournal { path: PathBuf },
}

/// Read view of one owned ship for the orders UI.
/// `eta_secs` is GAME seconds until arrival (ADR-0004: motion integrates
/// over game time, so ETA is quoted in game time too).
#[derive(Debug, Clone)]
pub struct OrderView {
    pub ship_id: String,
    pub waypoint: Option<GeoPosition>,
    pub ordered_speed_kn: Option<f32>,
    pub state: OrderState,
    pub eta_secs: Option<u64>,
    /// Taxonomy readout (grill #18): class id + player-facing type label.
    pub class_id: String,
    pub type_label: String,
    /// Class capability: the max speed this unit can sail at.
    pub max_speed_kn: f32,
}

/// UI <- sim: why a SetOrder was refused (ticket #22).
#[derive(Debug, Clone)]
pub enum OrderRefusal {
    /// Waypoint is on land.
    LandWaypoint,
    /// Straight path to the waypoint crosses land.
    LandBetween,
    /// Takeover named a class the sim does not know (H10): no stats,
    /// no ship. Sync specs first — the sim invents no abilities.
    UnknownClass,
    /// A HelmOrder was requested for a class with no usable local speed
    /// bound. The sandbox fails closed rather than inventing a maximum.
    NoSpeedLimit,
    /// The local command did not satisfy the public HelmOrder range.
    InvalidHelm,
}

/// UI <- sim: why an OrderMove leg was refused (precedence grill, #19).
/// Loud by design: refusals are Log entries, never silent drops.
#[derive(Debug, Clone)]
pub enum CommandRefusal {
    /// Issuer ranks below the ship's current holder.
    LowerAuthority { held_rank: u8, by_rank: u8 },
    /// The grant itself fails: expired, out of scope, or verb denied.
    Grant(GrantDenial),
}

/// Sim -> UI, drained per frame.
#[derive(Debug, Clone)]
pub enum SimEvent {
    Orders(Vec<OrderView>),
    Arrival { ship_id: String },
    /// A SetOrder was rejected (land waypoint / land between).
    OrderRefused { ship_id: String, reason: OrderRefusal },
    /// A local HelmOrder was applied after the command was accepted by
    /// the sandbox. The UI uses this to leave Pending without guessing
    /// that a queued channel send was an authoritative commit.
    HelmApplied {
        ship_id: String,
        heading_deg: f32,
        requested_speed_kn: f32,
        accepted_speed_kn: f32,
    },
    /// An OrderMove leg was refused (precedence grill, #19): lower
    /// authority or a failing grant. The future Log's entry kind.
    CommandRefused { ship_id: String, reason: CommandRefusal },
    /// A higher authority took a ship from its holder (reassertion).
    /// Loud by design: the detach is recorded, never silent.
    CommandOverridden { ship_id: String, prev_rank: u8, by_rank: u8 },
    /// An en-route ship stopped at the coast (order kept, state Blocked).
    ShipBlocked { ship_id: String },
    /// Game-clock readout, sent every round (ADR-0004: game_ts is derived
    /// from ts via this clock, never stored on fixes).
    Clock { game_elapsed_secs: u64, ratio: f64, paused: bool },
    /// Real + derived game clock readings, humane format, per round.
    ClockReadout { real_ts: String, game_ts: Option<String>, paused: bool },
}

#[derive(Debug)]
struct SimShip {
    pos: GeoPosition,
    heading_deg: f32,
    order: Option<Order>,
    helm_order: Option<HelmOrder>,
    /// Catalog class backing this unit (grill #18): holds its abilities.
    class: Class,
    /// Who currently holds the ship (precedence grill, #19): only an
    /// equal or higher authority may overwrite its order.
    held_by: Authority,
}

pub struct SimSource {
    ships: HashMap<String, SimShip>,
    cmd_rx: Receiver<SimCommand>,
    evt_tx: Sender<SimEvent>,
    clock: GameClock,
    last_tick: Option<Instant>,
    land: Option<Land>,
    catalog: Catalog,
    /// Append-only action journal (Log grill, #20): the sim is the
    /// single writer; every command outcome lands here as well as on
    /// the event channel.
    journal: Journal,
    last_marker_min: u64,
    /// Latest acked ingest seq per ship (Log grill, #20): what journal
    /// entries cite. Updated by FixAck; read when events journal.
    last_seq: HashMap<String, u64>,
}

impl SimSource {
    pub fn new(cmd_rx: Receiver<SimCommand>, evt_tx: Sender<SimEvent>) -> Self {
        Self::new_with_journal(cmd_rx, evt_tx, Journal::disabled())
    }

    pub fn new_with_journal(
        cmd_rx: Receiver<SimCommand>,
        evt_tx: Sender<SimEvent>,
        journal: Journal,
    ) -> Self {
        // Land data is optional: if the asset is missing the sim still
        // runs (no collision), so a broken checkout never blocks dev.
        let land = match Land::from_default_asset() {
            Ok(l) => Some(l),
            Err(e) => {
                eprintln!("land collision disabled: {e}");
                None
            }
        };
        // Taxonomy is required (grill #18): the catalog defines what a
        // unit IS; without it take-control cannot work.
        let catalog = Catalog::from_default_asset().expect("catalog asset valid");
        Self {
            ships: HashMap::new(),
            cmd_rx,
            evt_tx,
            clock: GameClock::default(),
            last_tick: None,
            land,
            catalog,
            journal,
            last_marker_min: 0,
            last_seq: HashMap::new(),
        }
    }

    pub fn owned_ids(&self) -> Vec<String> {
        self.ships.keys().cloned().collect()
    }

    /// Shared commit path (ticket #22 + grill #18): land rejection, then
    /// class-capped order install. Returns a land refusal, if any.
    fn apply_leg(
        &mut self,
        ship_id: &str,
        waypoint: GeoPosition,
        speed_kn: f32,
    ) -> Option<OrderRefusal> {
        let refusal = self.land.as_ref().and_then(|land| {
            if !land.is_water(&waypoint) {
                Some(OrderRefusal::LandWaypoint)
            } else if let Some(s) = self.ships.get(ship_id) {
                (!land.path_is_water(&s.pos, &waypoint)).then_some(OrderRefusal::LandBetween)
            } else {
                None
            }
        });
        if refusal.is_some() {
            return refusal;
        }
        if let Some(s) = self.ships.get_mut(ship_id) {
            // Class caps the order (grill #18): abilities live on the
            // class; orders cannot exceed capability. A missing cap
            // reads as no speed (H10) — failing closed, never infinite.
            let max = Catalog::stat(&s.class, "speed_kn", 0.0) as f32;
            s.helm_order = None;
            s.order = Some(Order {
                waypoint,
                speed_kn: speed_kn.min(max),
                state: OrderState::EnRoute,
            });
        }
        None
    }

    fn drain_commands(&mut self) {
        // Drain first: the channel iterator borrows self, which would
        // forbid the mutable borrows the command arms need below.
        let cmds: Vec<SimCommand> = self.cmd_rx.try_iter().collect();
        for cmd in cmds {
            match cmd {
                SimCommand::TakeControl { ship_id, pos, class_id } => {
                    // Journaled so session logs replay placement (#41).
                    self.journal.append(
                        self.clock.game_now_ts(),
                        "organizer",
                        LogKind::Command,
                        serde_json::json!({"event": "take-control", "ship": ship_id, "lat": pos.latitude, "lon": pos.longitude, "class": class_id}),
                    );
                    // H10: fail closed. An unknown class id means no
                    // published figures reached this sim — driving the
                    // hull on another class's abilities would be an
                    // invention, so the takeover is refused loudly and
                    // no ship is stood up.
                    let Some(class) = self.catalog.class(&class_id).cloned() else {
                        let reason = OrderRefusal::UnknownClass;
                        let _ = self.evt_tx.send(SimEvent::OrderRefused {
                            ship_id: ship_id.clone(),
                            reason: reason.clone(),
                        });
                        self.journal.append(
                            self.clock.game_now_ts(),
                            "sim",
                            LogKind::OrderRefused,
                            serde_json::json!({"ship": ship_id, "reason": format!("{reason:?}")}),
                        );
                        continue;
                    };
                    self.ships.entry(ship_id).or_insert_with(|| SimShip {
                        pos,
                        heading_deg: 0.0,
                        order: None,
                        helm_order: None,
                        class,
                        held_by: Authority::UNIT,
                    });
                }
                SimCommand::UpsertClass {
                    minos_class_id,
                    name,
                    version,
                    speed_kn,
                    cruise_kn,
                    range_nm,
                } => {
                    self.catalog.upsert_runtime_class(
                        minos_class_id,
                        name,
                        version,
                        speed_kn,
                        cruise_kn,
                        range_nm,
                    );
                }
                SimCommand::Release { ship_id } => {
                    self.journal.append(
                        self.clock.game_now_ts(),
                        "organizer",
                        LogKind::Command,
                        serde_json::json!({"event": "release", "ship": ship_id}),
                    );
                    self.ships.remove(&ship_id);
                }
                SimCommand::SetOrder { ship_id, waypoint, speed_kn } => {
                    if let Some(reason) = self.apply_leg(&ship_id, waypoint, speed_kn) {
                        let _ = self.evt_tx.send(SimEvent::OrderRefused {
                            ship_id: ship_id.clone(),
                            reason: reason.clone(),
                        });
                        self.journal.append(
                            self.clock.game_now_ts(),
                            "sim",
                            LogKind::OrderRefused,
                            serde_json::json!({"ship": ship_id, "reason": format!("{reason:?}")}),
                        );
                    }
                }
                SimCommand::OrderMove { command } => {
                    let now = self.clock.game_elapsed_secs();
                    let game_ts = self.clock.game_now_ts();
                    let actor = format!("authority:{}", command.authority.rank());
                    self.journal.append(
                        game_ts.clone(),
                        actor.clone(),
                        LogKind::Command,
                        serde_json::json!({
                            "ships": command.fan_out().iter().map(|l| l.ship_id.clone()).collect::<Vec<_>>(),
                        }),
                    );
                    for leg in command.fan_out() {
                        let id = leg.ship_id.clone();
                        if let Err(denial) =
                            command.grant.covers(&id, Verb::Move, now)
                        {
                            let reason = CommandRefusal::Grant(denial);
                            let _ = self.evt_tx.send(SimEvent::CommandRefused {
                                ship_id: id.clone(),
                                reason: reason.clone(),
                            });
                            self.journal.append(
                                game_ts.clone(),
                                actor.clone(),
                                LogKind::CommandRefused,
                                serde_json::json!({"ship": id, "reason": format!("{reason:?}")}),
                            );
                            continue;
                        }
                        let held =
                            self.ships.get(&id).map(|s| s.held_by).unwrap_or(Authority::UNIT);
                        if command.authority < held {
                            let reason = CommandRefusal::LowerAuthority {
                                held_rank: held.rank(),
                                by_rank: command.authority.rank(),
                            };
                            let _ = self.evt_tx.send(SimEvent::CommandRefused {
                                ship_id: id.clone(),
                                reason: reason.clone(),
                            });
                            self.journal.append(
                                game_ts.clone(),
                                actor.clone(),
                                LogKind::CommandRefused,
                                serde_json::json!({"ship": id, "reason": format!("{reason:?}")}),
                            );
                            continue;
                        }
                        if let Some(reason) =
                            self.apply_leg(&id, leg.waypoint, leg.speed_kn)
                        {
                            let _ = self.evt_tx.send(SimEvent::OrderRefused {
                                ship_id: id.clone(),
                                reason: reason.clone(),
                            });
                            self.journal.append(
                                game_ts.clone(),
                                actor.clone(),
                                LogKind::OrderRefused,
                                serde_json::json!({"ship": id, "reason": format!("{reason:?}")}),
                            );
                            continue;
                        }
                        if command.authority > held {
                            let _ = self.evt_tx.send(SimEvent::CommandOverridden {
                                ship_id: id.clone(),
                                prev_rank: held.rank(),
                                by_rank: command.authority.rank(),
                            });
                            self.journal.append(
                                game_ts.clone(),
                                actor.clone(),
                                LogKind::CommandOverridden,
                                serde_json::json!({"ship": id, "prev_rank": held.rank(), "by_rank": command.authority.rank()}),
                            );
                        }
                        if let Some(s) = self.ships.get_mut(&id) {
                            s.held_by = command.authority;
                        }
                    }
                }
                SimCommand::SetHelm {
                    ship_id,
                    heading_deg,
                    speed_kn,
                    authority,
                    grant,
                } => {
                    let now = self.clock.game_elapsed_secs();
                    let game_ts = self.clock.game_now_ts();
                    let actor = format!("authority:{}", authority.rank());
                    if let Err(denial) = grant.covers(&ship_id, Verb::SetHelm, now) {
                        let reason = CommandRefusal::Grant(denial);
                        let _ = self.evt_tx.send(SimEvent::CommandRefused {
                            ship_id: ship_id.clone(),
                            reason: reason.clone(),
                        });
                        self.journal.append(
                            game_ts,
                            actor,
                            LogKind::CommandRefused,
                            serde_json::json!({"ship": ship_id, "reason": format!("{reason:?}")}),
                        );
                        continue;
                    }
                    let Some(ship) = self.ships.get(&ship_id) else {
                        continue;
                    };
                    let held = ship.held_by;
                    if authority < held {
                        let reason = CommandRefusal::LowerAuthority {
                            held_rank: held.rank(),
                            by_rank: authority.rank(),
                        };
                        let _ = self.evt_tx.send(SimEvent::CommandRefused {
                            ship_id: ship_id.clone(),
                            reason: reason.clone(),
                        });
                        self.journal.append(
                            game_ts,
                            actor,
                            LogKind::CommandRefused,
                            serde_json::json!({"ship": ship_id, "reason": format!("{reason:?}")}),
                        );
                        continue;
                    }
                    if !heading_deg.is_finite()
                        || !(0.0..360.0).contains(&heading_deg)
                        || !speed_kn.is_finite()
                        || speed_kn < 0.0
                    {
                        let reason = OrderRefusal::InvalidHelm;
                        let _ = self.evt_tx.send(SimEvent::OrderRefused {
                            ship_id: ship_id.clone(),
                            reason,
                        });
                        self.journal.append(
                            game_ts,
                            actor,
                            LogKind::OrderRefused,
                            serde_json::json!({"ship": ship_id, "reason": "InvalidHelm"}),
                        );
                        continue;
                    }
                    let max = Catalog::stat(&ship.class, "speed_kn", 0.0) as f32;
                    if max <= 0.0 {
                        let reason = OrderRefusal::NoSpeedLimit;
                        let _ = self.evt_tx.send(SimEvent::OrderRefused {
                            ship_id: ship_id.clone(),
                            reason,
                        });
                        self.journal.append(
                            game_ts,
                            actor,
                            LogKind::OrderRefused,
                            serde_json::json!({"ship": ship_id, "reason": "NoSpeedLimit"}),
                        );
                        continue;
                    }
                    let heading = heading_deg.rem_euclid(360.0);
                    let accepted_speed = speed_kn.clamp(0.0, max);
                    let applied_heading = if accepted_speed == 0.0 {
                        ship.heading_deg
                    } else {
                        heading
                    };
                    self.journal.append(
                        game_ts.clone(),
                        actor.clone(),
                        LogKind::Command,
                        serde_json::json!({
                            "event": "set-helm",
                            "ship": ship_id.clone(),
                            "heading_deg": heading,
                            "requested_speed_kn": speed_kn,
                            "accepted_speed_kn": accepted_speed,
                        }),
                    );
                    if let Some(ship) = self.ships.get_mut(&ship_id) {
                        ship.order = None;
                        ship.heading_deg = applied_heading;
                        ship.helm_order = Some(HelmOrder {
                            heading_deg: applied_heading,
                            speed_kn: accepted_speed,
                            blocked: false,
                        });
                        ship.held_by = authority;
                    }
                    if authority > held {
                        let _ = self.evt_tx.send(SimEvent::CommandOverridden {
                            ship_id: ship_id.clone(),
                            prev_rank: held.rank(),
                            by_rank: authority.rank(),
                        });
                        self.journal.append(
                            game_ts,
                            actor,
                            LogKind::CommandOverridden,
                            serde_json::json!({"ship": ship_id, "prev_rank": held.rank(), "by_rank": authority.rank()}),
                        );
                    }
                    let _ = self.evt_tx.send(SimEvent::HelmApplied {
                        ship_id,
                        heading_deg: applied_heading,
                        requested_speed_kn: speed_kn,
                        accepted_speed_kn: accepted_speed,
                    });
                }
                SimCommand::CancelOrder { ship_id } => {
                    if let Some(s) = self.ships.get_mut(&ship_id) {
                        s.order = None;
                        s.helm_order = None;
                    }
                }
                SimCommand::SetPaused { paused } => {
                    self.clock.set_paused(paused);
                }
                SimCommand::SetClockRatio { ratio } => {
                    self.clock.set_ratio(ratio);
                }
                SimCommand::SeedClockStart { ts } => {
                    self.clock.begin(&ts);
                }
                SimCommand::FixAck { ship_id, seq } => {
                    let slot = self.last_seq.entry(ship_id).or_insert(seq);
                    *slot = (*slot).max(seq);
                }
                SimCommand::RotateJournal { path } => {
                    self.journal = Journal::open_append(path).unwrap_or_else(|e| {
                        eprintln!("journal rotation failed: {e}");
                        Journal::disabled()
                    });
                    self.last_seq.clear();
                    self.last_marker_min = 0;
                }
            }
        }
    }

    fn views(&self) -> Vec<OrderView> {
        let mut out: Vec<OrderView> = self
            .ships
            .iter()
            .map(|(id, s)| {
                if let Some(helm) = &s.helm_order {
                    return OrderView {
                        ship_id: id.clone(),
                        waypoint: None,
                        ordered_speed_kn: Some(helm.speed_kn),
                        state: OrderState::Holding,
                        eta_secs: None,
                        class_id: s.class.id.clone(),
                        type_label: s.class.display_type().to_string(),
                        max_speed_kn: Catalog::stat(&s.class, "speed_kn", 0.0) as f32,
                    };
                }
                match &s.order {
                    Some(o) => {
                        let dist = s.pos.distance_m(&o.waypoint);
                        // Game seconds to waypoint: motion covers dist at
                        // speed over GAME time (ADR-0004).
                        let eta = (dist / (o.speed_kn as f64 * 0.514_444)) as u64;
                        OrderView {
                            ship_id: id.clone(),
                            waypoint: Some(o.waypoint),
                            ordered_speed_kn: Some(o.speed_kn),
                            state: o.state,
                            eta_secs: Some(eta),
                            class_id: s.class.id.clone(),
                            type_label: s.class.display_type().to_string(),
                            max_speed_kn: Catalog::stat(&s.class, "speed_kn", 0.0) as f32,
                        }
                    }
                    None => OrderView {
                        ship_id: id.clone(),
                        waypoint: None,
                        ordered_speed_kn: None,
                        state: OrderState::Holding,
                        eta_secs: None,
                        class_id: s.class.id.clone(),
                        type_label: s.class.display_type().to_string(),
                        max_speed_kn: Catalog::stat(&s.class, "speed_kn", 0.0) as f32,
                    },
                }
            })
            .collect();
        out.sort_by(|a, b| a.ship_id.cmp(&b.ship_id));
        out
    }
}

impl PollSource for SimSource {
    fn poll(&mut self) -> Result<Vec<Fix>, BackendError> {
        let now = Instant::now();
        let real_dt = self.last_tick.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
        self.last_tick = Some(now);
        self.poll_round(real_dt)
    }
}

impl SimSource {
    /// One sim round given `real_dt` real seconds elapsed since the last
    /// round. Split from `poll` so tests drive time deterministically.
    fn poll_round(&mut self, real_dt: f64) -> Result<Vec<Fix>, BackendError> {
        self.drain_commands();
        // Advance the game clock. First round stamps the session start and
        // moves nothing; paused rounds advance nothing while the wall clock
        // runs on (ADR-0004).
        if !self.clock.started() {
            self.clock.begin(&now_ts());
        }
        let game_dt = self.clock.tick(real_dt);
        let mut fixes = Vec::with_capacity(self.ships.len());
        let mut arrivals = Vec::new();
        let mut blocked = Vec::new();
        // Sim speed fix reports the CLASS capability when there is no
        // order (traffic-style readout), else the ordered speed.
        for (id, s) in self.ships.iter_mut() {
            if let Some(helm) = s.helm_order.as_mut() {
                if !helm.blocked && helm.speed_kn > 0.0 {
                    let next = s.pos.dead_reckon(helm.heading_deg, helm.speed_kn, game_dt);
                    let clear = self
                        .land
                        .as_ref()
                        .map(|l| l.path_is_water(&s.pos, &next))
                        .unwrap_or(true);
                    if clear {
                        s.pos = next;
                    } else {
                        s.pos = self
                            .land
                            .as_ref()
                            .map(|land| {
                                stop_at_first_land(
                                    land,
                                    s.pos,
                                    next,
                                    helm.heading_deg,
                                    helm.speed_kn,
                                    game_dt,
                                )
                            })
                            .unwrap_or(s.pos);
                        helm.blocked = true;
                        helm.speed_kn = 0.0;
                        blocked.push(id.clone());
                    }
                }
            } else if let Some(o) = s.order.as_mut() {
                if o.state == OrderState::EnRoute {
                    let dist = s.pos.distance_m(&o.waypoint);
                    if dist <= ARRIVAL_M {
                        s.pos = o.waypoint;
                        o.state = OrderState::Arrived;
                        arrivals.push(id.clone());
                    } else {
                        s.heading_deg = s.pos.bearing_deg_to(&o.waypoint) as f32;
                        // Motion integrates over GAME elapsed seconds,
                        // capped at the remaining distance: without the
                        // cap a fast ship overshoots the waypoint every
                        // tick and ping-pongs around it forever.
                        let speed_mps = o.speed_kn as f64 * 0.514_444;
                        let step_dt = game_dt.min(dist / speed_mps);
                        let next = s.pos.dead_reckon(s.heading_deg, o.speed_kn, step_dt);
                        // Stop en route at the coast (ticket #22): if the
                        // leg this tick would touch land, hold position
                        // and flag blocked; the order survives so a
                        // player-edited waypoint can resume progress.
                        let clear = self
                            .land
                            .as_ref()
                            .map(|l| l.path_is_water(&s.pos, &next))
                            .unwrap_or(true);
                        if clear {
                            s.pos = next;
                        } else {
                            o.state = OrderState::Blocked;
                            blocked.push(id.clone());
                        }
                    }
                }
            }
            fixes.push(Fix {
                ship_id: id.clone(),
                position: s.pos,
                ts: now_ts(),
                received_at: None,
                heading_deg: Some(s.heading_deg),
                speed_kn: s
                    .helm_order
                    .as_ref()
                    .map(|h| h.speed_kn)
                    .or_else(|| s.order.as_ref().map(|o| o.speed_kn))
                    .or(Some(Catalog::stat(&s.class, "speed_kn", 0.0) as f32)),
                accuracy_m: None,
                name: None,
                hull_number: None,
                backfilled: false,
                source: FixSource::Sim,
                // Sim emissions are game-time: no server age to retain.
                age_secs: None,
                // Ingest sequence is stamped by the Registry (Log grill,
                // #20); the sim only fills the placeholder.
                seq: 0,
            });
        }
        let views = self.views();
        let _ = self.evt_tx.send(SimEvent::Orders(views));
        let _ = self.evt_tx.send(SimEvent::Clock {
            game_elapsed_secs: self.clock.game_elapsed_secs(),
            ratio: self.clock.ratio(),
            paused: self.clock.paused(),
        });
        let _ = self.evt_tx.send(SimEvent::ClockReadout {
            real_ts: now_ts(),
            game_ts: self.clock.game_now_ts(),
            paused: self.clock.paused(),
        });
        for ship_id in arrivals {
            let _ = self.evt_tx.send(SimEvent::Arrival { ship_id: ship_id.clone() });
            let fix_seq = self.last_seq.get(&ship_id).copied();
            self.journal.append(
                self.clock.game_now_ts(),
                "sim",
                LogKind::Arrival,
                serde_json::json!({"ship": ship_id, "fix_seq": fix_seq}),
            );
        }
        for ship_id in blocked {
            let _ = self.evt_tx.send(SimEvent::ShipBlocked { ship_id: ship_id.clone() });
            let fix_seq = self.last_seq.get(&ship_id).copied();
            self.journal.append(
                self.clock.game_now_ts(),
                "sim",
                LogKind::ShipBlocked,
                serde_json::json!({"ship": ship_id, "fix_seq": fix_seq}),
            );
        }
        // Game-minute markers (Log grill, #20): derived from the game
        // clock, so pause accrues none. Each marker snapshots the latest
        // acked fix seq per ship: replay positions recoverable by seq.
        let minute = self.clock.game_elapsed_secs() / 60;
        if minute > self.last_marker_min {
            self.last_marker_min = minute;
            let fix_seqs: HashMap<String, u64> =
                self.last_seq.iter().map(|(k, v)| (k.clone(), *v)).collect();
            self.journal.append(
                self.clock.game_now_ts(),
                "sim",
                LogKind::Marker,
                serde_json::json!({"minute": minute, "fix_seqs": fix_seqs}),
            );
        }
        Ok(fixes)
    }
}

/// Joins a wire source with the sim: wire fixes for sim-owned ships are
/// suppressed (owned vs traffic). A wire error yields sim-only output.
/// Disarmed (presentation mode) the sim is not polled at all: owned ships
/// freeze and go stale by the normal miss counter; the clock stops too.
pub struct MergeSource {
    wire: Box<dyn PollSource>,
    sim: SimSource,
    armed: Arc<AtomicBool>,
}

impl MergeSource {
    pub fn new(wire: Box<dyn PollSource>, sim: SimSource) -> Self {
        Self { wire, sim, armed: Arc::new(AtomicBool::new(true)) }
    }

    /// Share the arm flag with the UI thread (session flow): the shell
    /// toggles presentation/simulation mode through it.
    pub fn set_armed_flag(&mut self, flag: Arc<AtomicBool>) {
        self.armed = flag;
    }

    /// Swap the wire source at runtime (mode model, #39): the poll thread
    /// replaces backends without restarting the sim underneath.
    pub fn set_wire(&mut self, wire: Box<dyn PollSource>) {
        self.wire = wire;
    }
}

impl PollSource for MergeSource {
    fn poll(&mut self) -> Result<Vec<Fix>, BackendError> {
        let mut out = if self.armed.load(Ordering::SeqCst) {
            self.sim.poll()?
        } else {
            Vec::new()
        };
        let owned = self.sim.owned_ids();
        match self.wire.poll() {
            Ok(wire_fixes) => {
                out.extend(wire_fixes.into_iter().filter(|f| !owned.contains(&f.ship_id)));
            }
            Err(e) => eprintln!("wire poll failed (sim continues): {e}"),
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::Grant;
    use crate::command::Leg;
    use std::sync::mpsc;

    fn harness() -> (SimSource, Receiver<SimEvent>, Sender<SimCommand>) {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (evt_tx, evt_rx) = mpsc::channel();
        (SimSource::new(cmd_rx, evt_tx), evt_rx, cmd_tx)
    }

    fn ship_at(lat: f64, lon: f64) -> GeoPosition {
        GeoPosition { latitude: lat, longitude: lon }
    }

    #[test]
    fn disarmed_sim_emits_nothing() {
        let (mut sim, _, _) = harness();
        assert!(sim.poll().unwrap().is_empty());
    }

    #[test]
    fn order_advances_ship_and_arrives() {
        let (mut sim, evt_rx, cmd) = harness();
        let start = ship_at(-5.92, 106.92);
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: start, class_id: "martadinata-sigma-10514-pkr".into() }).unwrap();
        // ~3.2 km east across open water at ludicrous speed.
        let wp = ship_at(-5.92, 106.955);
        cmd.send(SimCommand::SetOrder { ship_id: "t".into(), waypoint: wp, speed_kn: 120.0 }).unwrap();
        let first = sim.poll_round(SIM_TICK_SECS).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].source, FixSource::Sim);
        // First round stamps the session start (ADR-0004): emits the ship
        // but moves nothing yet.
        assert_eq!(first[0].position, start);
        // Run until arrival. Order clamps to the Martadinata class's
        // 28 kn, so ~3.9 km takes ~135 two-second game ticks.
        let mut arrived = false;
        for _ in 0..200 {
            sim.poll_round(SIM_TICK_SECS).unwrap();
            if evt_rx.try_iter().any(|e| matches!(e, SimEvent::Arrival { .. })) {
                arrived = true;
                break;
            }
        }
        assert!(arrived, "ship arrives at the waypoint");
        let views = sim.views();
        assert_eq!(views[0].state, OrderState::Arrived);
    }

    #[test]
    fn cancel_holds_position() {
        let (mut sim, _, cmd) = harness();
        let start = ship_at(-5.92, 106.92);
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: start, class_id: "martadinata-sigma-10514-pkr".into() }).unwrap();
        cmd.send(SimCommand::SetOrder {
            ship_id: "t".into(),
            waypoint: ship_at(-5.92, 106.95),
            speed_kn: 60.0,
        })
        .unwrap();
        sim.poll().unwrap();
        cmd.send(SimCommand::CancelOrder { ship_id: "t".into() }).unwrap();
        let a = sim.poll().unwrap()[0].position;
        let b = sim.poll().unwrap()[0].position;
        assert_eq!(a, b, "cancelled ship holds");
        assert_eq!(sim.views()[0].state, OrderState::Holding);
    }

    #[test]
    fn game_clock_starts_on_first_tick_and_clock_event_flows() {
        let (mut sim, evt_rx, _) = harness();
        sim.poll_round(SIM_TICK_SECS).unwrap(); // stamps session start
        sim.poll_round(SIM_TICK_SECS).unwrap();
        // One Clock event per round; take the last (rounds drain in order).
        let clock = evt_rx
            .try_iter()
            .filter_map(|e| match e {
                SimEvent::Clock { game_elapsed_secs, ratio, paused } => {
                    Some((game_elapsed_secs, ratio, paused))
                }
                _ => None,
            })
            .last()
            .expect("clock event arrives");
        assert_eq!(clock.0, SIM_TICK_SECS as u64, "one game tick elapsed");
        assert!((clock.1 - 1.0).abs() < 1e-9);
        assert!(!clock.2);
    }

    #[test]
    fn paused_sim_holds_positions_and_game_time() {
        let (mut sim, evt_rx, cmd) = harness();
        let start = ship_at(-5.92, 106.92);
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: start, class_id: "martadinata-sigma-10514-pkr".into() }).unwrap();
        cmd.send(SimCommand::SetOrder {
            ship_id: "t".into(),
            waypoint: ship_at(-5.92, 106.95),
            speed_kn: 60.0,
        })
        .unwrap();
        sim.poll_round(SIM_TICK_SECS).unwrap(); // session start
        cmd.send(SimCommand::SetPaused { paused: true }).unwrap();
        sim.poll_round(SIM_TICK_SECS).unwrap(); // pause takes effect
        let a = sim.poll_round(SIM_TICK_SECS).unwrap()[0].position;
        let b = sim.poll_round(SIM_TICK_SECS).unwrap()[0].position;
        assert_eq!(a, b, "paused ship holds");
        assert_eq!(sim.views()[0].state, OrderState::EnRoute, "order survives pause");
        let clock = evt_rx
            .try_iter()
            .find_map(|e| matches!(e, SimEvent::Clock { paused: true, .. }).then_some(()))
            .is_some();
        assert!(clock, "paused flag reported");
        // Unpause: the order resumes from the held position.
        cmd.send(SimCommand::SetPaused { paused: false }).unwrap();
        sim.poll_round(SIM_TICK_SECS).unwrap();
        let c = sim.poll_round(SIM_TICK_SECS).unwrap()[0].position;
        assert!(c.distance_m(&a) > 0.0, "motion resumes after unpause");
    }

    #[test]
    fn land_waypoint_is_rejected_and_water_accepted() {
        let (mut sim, evt_rx, cmd) = harness();
        // Ship in the Java Sea, waypoint inland on Java.
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: ship_at(-5.8, 106.7), class_id: "martadinata-sigma-10514-pkr".into() })
            .unwrap();
        sim.poll_round(0.0).unwrap(); // arm + session start
        cmd.send(SimCommand::SetOrder {
            ship_id: "t".into(),
            waypoint: ship_at(-6.5, 107.0), // inland Java
            speed_kn: 60.0,
        })
        .unwrap();
        sim.poll_round(0.0).unwrap();
        assert!(evt_rx.try_iter().any(
            |e| matches!(e, SimEvent::OrderRefused { reason: OrderRefusal::LandWaypoint, .. })
        ));
        assert_eq!(sim.views()[0].state, OrderState::Holding, "no order created");
        // A sea waypoint across open water is accepted.
        cmd.send(SimCommand::SetOrder {
            ship_id: "t".into(),
            waypoint: ship_at(-5.6, 107.6),
            speed_kn: 60.0,
        })
        .unwrap();
        sim.poll_round(0.0).unwrap();
        assert_eq!(sim.views()[0].state, OrderState::EnRoute);
    }

    #[test]
    fn path_crossing_land_is_refused() {
        let (mut sim, evt_rx, cmd) = harness();
        // North of Java heading south across the island: water on both
        // ends, land between.
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: ship_at(-5.9, 106.9), class_id: "martadinata-sigma-10514-pkr".into() })
            .unwrap();
        sim.poll_round(0.0).unwrap();
        cmd.send(SimCommand::SetOrder {
            ship_id: "t".into(),
            waypoint: ship_at(-7.5, 106.9), // open Indian Ocean, past Java
            speed_kn: 60.0,
        })
        .unwrap();
        sim.poll_round(0.0).unwrap();
        assert!(evt_rx.try_iter().any(
            |e| matches!(e, SimEvent::OrderRefused { reason: OrderRefusal::LandBetween, .. })
        ));
    }

    #[test]
    fn en_route_ship_stops_at_coast_and_keeps_order() {
        // Defense in depth (ticket #22): the commit check covers the
        // whole straight path, so en-route land only appears if the
        // world changes under the order. Simulate that by swapping in a
        // synthetic island after the order is committed.
        let (mut sim, evt_rx, cmd) = harness();
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: ship_at(-5.8, 106.9), class_id: "martadinata-sigma-10514-pkr".into() })
            .unwrap();
        sim.poll_round(0.0).unwrap();
        cmd.send(SimCommand::SetOrder {
            ship_id: "t".into(),
            waypoint: ship_at(-5.8, 107.3), // open water, clear path
            speed_kn: 600.0,
        })
        .unwrap();
        sim.poll_round(0.0).unwrap(); // order committed
        assert_eq!(sim.views()[0].state, OrderState::EnRoute);
        // The world changes: an island rises athwart the ship's track.
        sim.land = Some(
            Land::from_geojson(
                r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{},"geometry":{"type":"Polygon","coordinates":[[[107.05,-5.85],[107.15,-5.85],[107.15,-5.75],[107.05,-5.75],[107.05,-5.85]]]}}]}"#,
            )
            .expect("synthetic land parses"),
        );
        let mut saw_blocked = false;
        for _ in 0..30 {
            sim.poll_round(120.0).unwrap(); // 2 min game steps, fast ship
            if evt_rx.try_iter().any(|e| matches!(e, SimEvent::ShipBlocked { .. })) {
                saw_blocked = true;
                break;
            }
        }
        assert!(saw_blocked, "ship must stop at the coast");
        let v = &sim.views()[0];
        assert_eq!(v.state, OrderState::Blocked);
        assert!(v.waypoint.is_some(), "order survives blocking");
        assert!(
            sim.land.as_ref().unwrap().is_water(&sim.ships["t"].pos),
            "ship rests on water, not inside the island"
        );
    }

    #[test]
    fn class_caps_order_speed_and_views_expose_taxonomy() {
        let (mut sim, _, cmd) = harness();
        cmd.send(SimCommand::TakeControl {
            ship_id: "t".into(),
            pos: ship_at(-5.92, 106.92),
            class_id: "cakra-type-209-1300".into(), // 11 kn class
        })
        .unwrap();
        sim.poll_round(0.0).unwrap();
        // Order above capability: clamped to the class speed.
        cmd.send(SimCommand::SetOrder {
            ship_id: "t".into(),
            waypoint: ship_at(-5.92, 106.95),
            speed_kn: 999.0,
        })
        .unwrap();
        sim.poll_round(0.0).unwrap();
        let v = &sim.views()[0];
        assert_eq!(v.ordered_speed_kn, Some(11.0), "order clamped to class");
        assert_eq!(v.class_id, "cakra-type-209-1300");
        assert_eq!(v.type_label, "Kapal selam serang diesel-elektrik");
        assert_eq!(v.max_speed_kn, 11.0);
        // Unknown class id fails closed (H10): no ship, loud refusal.
        let (mut sim, evt_rx, cmd) = harness();
        cmd.send(SimCommand::TakeControl {
            ship_id: "t".into(),
            pos: ship_at(-5.92, 106.92),
            class_id: "nonexistent".into(),
        })
        .unwrap();
        sim.poll_round(0.0).unwrap();
        assert!(sim.views().is_empty(), "no ship on unknown class");
        assert!(
            evt_rx.try_iter().any(
                |e| matches!(e, SimEvent::OrderRefused { reason: OrderRefusal::UnknownClass, .. })
            ),
            "takeover refusal names the missing class"
        );
    }

    #[test]
    fn upsert_class_makes_minos_figures_drivable() {
        // H10: synced figures pushed down from the UI drive the sim —
        // the sim's own catalog starts asset-only.
        let (mut sim, _, cmd) = harness();
        cmd.send(SimCommand::UpsertClass {
            minos_class_id: 5,
            name: "Sigma".into(),
            version: 3,
            speed_kn: 28.0,
            cruise_kn: 18.0,
            range_nm: 5000.0,
        })
        .unwrap();
        cmd.send(SimCommand::TakeControl {
            ship_id: "t".into(),
            pos: ship_at(-5.92, 106.92),
            class_id: "minos-5".into(),
        })
        .unwrap();
        sim.poll_round(0.0).unwrap();
        let v = &sim.views()[0];
        assert_eq!(v.class_id, "minos-5");
        assert_eq!(v.max_speed_kn, 28.0, "Minos figures drive, not bundled ones");
    }

    #[test]
    fn merge_suppresses_wire_for_owned_ships() {
        use crate::backend::FileReplay;
        let (sim, _, cmd) = harness();
        cmd.send(SimCommand::TakeControl { ship_id: "nordwind".into(), pos: ship_at(-6.1, 106.86), class_id: "martadinata-sigma-10514-pkr".into() })
            .unwrap();
        let wire = FileReplay::from_file("tests/fixtures/tracks.json").expect("fixture");
        let mut merge = MergeSource::new(Box::new(wire), sim);
        // Drain one round so TakeControl lands (sim polls first inside merge).
        let _ = merge.poll().unwrap();
        let round = merge.poll().unwrap();
        let ids: Vec<&str> = round.iter().map(|f| f.ship_id.as_str()).collect();
        assert!(ids.contains(&"nordwind") && ids.contains(&"ostsee"));
        let nord: Vec<&Fix> = round.iter().filter(|f| f.ship_id == "nordwind").collect();
        assert_eq!(nord.len(), 1, "exactly one nordwind fix per round");
        assert_eq!(nord[0].source, FixSource::Sim);
    }

    fn move_cmd(auth: Authority, lat: f64, lon: f64, expires: u64) -> SimCommand {
        SimCommand::OrderMove {
            command: MoveCommand {
                legs: vec![Leg {
                    ship_id: "t".into(),
                    waypoint: ship_at(lat, lon),
                    speed_kn: 10.0,
                }],
                default_speed_kn: None,
                authority: auth,
                grant: Grant {
                    units: vec!["t".into()],
                    expires_game_secs: expires,
                    verbs: vec![Verb::Move],
                },
            },
        }
    }

    #[test]
    fn order_move_applies_then_higher_overrides_loudly() {
        let (mut sim, evt_rx, cmd) = harness();
        cmd.send(SimCommand::TakeControl {
            ship_id: "t".into(),
            pos: ship_at(-5.92, 106.92),
            class_id: "martadinata-sigma-10514-pkr".into(),
        })
        .unwrap();
        cmd.send(move_cmd(Authority::UNIT, -5.92, 106.95, u64::MAX)).unwrap();
        sim.poll_round(0.0).unwrap();
        let wp = sim.views()[0].waypoint.unwrap();
        assert!((wp.longitude - 106.95).abs() < 1e-9, "unit-level move lands");
        let _ = evt_rx.try_iter().collect::<Vec<_>>();
        // Higher authority overwrites + records the override.
        cmd.send(move_cmd(Authority::SATGAS, -5.92, 106.96, u64::MAX)).unwrap();
        sim.poll_round(0.0).unwrap();
        let wp = sim.views()[0].waypoint.unwrap();
        assert!((wp.longitude - 106.96).abs() < 1e-9, "satgas overwrites");
        assert!(
            evt_rx.try_iter().any(|e| matches!(
                e,
                SimEvent::CommandOverridden { prev_rank: 0, by_rank: 1, .. }
            )),
            "override is loud"
        );
        // Lower authority is refused loudly; the order stands.
        cmd.send(move_cmd(Authority::UNIT, -5.92, 106.95, u64::MAX)).unwrap();
        sim.poll_round(0.0).unwrap();
        let wp = sim.views()[0].waypoint.unwrap();
        assert!((wp.longitude - 106.96).abs() < 1e-9, "refused move changes nothing");
        assert!(
            evt_rx.try_iter().any(|e| matches!(
                e,
                SimEvent::CommandRefused {
                    reason: CommandRefusal::LowerAuthority { .. },
                    ..
                }
            )),
            "refusal is loud"
        );
    }

    #[test]
    fn order_move_honors_grant_bounds() {
        let (mut sim, evt_rx, cmd) = harness();
        cmd.send(SimCommand::TakeControl {
            ship_id: "t".into(),
            pos: ship_at(-5.92, 106.92),
            class_id: "martadinata-sigma-10514-pkr".into(),
        })
        .unwrap();
        // Expired grant (game clock starts at 0, expiry 0 is past).
        cmd.send(move_cmd(Authority::ORGANIZER, -5.92, 106.95, 0)).unwrap();
        sim.poll_round(0.0).unwrap();
        assert!(sim.views()[0].waypoint.is_none(), "expired grant applies nothing");
        assert!(
            evt_rx.try_iter().any(|e| matches!(
                e,
                SimEvent::CommandRefused {
                    reason: CommandRefusal::Grant(GrantDenial::Expired),
                    ..
                }
            )),
            "expiry is loud"
        );
    }

    fn journal_lines(path: &std::path::Path) -> Vec<serde_json::Value> {
        let text = std::fs::read_to_string(path).unwrap();
        text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn arrival_cites_last_acked_fix_seq() {
        let path = std::env::temp_dir().join("tfg-log-test-arrival.jsonl");
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (evt_tx, evt_rx) = mpsc::channel();
        let journal = Journal::open(path.clone()).unwrap();
        let mut sim = SimSource::new_with_journal(cmd_rx, evt_tx, journal);
        let start = ship_at(-5.92, 106.92);
        cmd_tx
            .send(SimCommand::TakeControl { ship_id: "t".into(), pos: start, class_id: "martadinata-sigma-10514-pkr".into() })
            .unwrap();
        cmd_tx
            .send(SimCommand::SetOrder {
                ship_id: "t".into(),
                waypoint: ship_at(-5.92, 106.955),
                speed_kn: 120.0,
            })
            .unwrap();
        sim.poll_round(SIM_TICK_SECS).unwrap();
        // Ingest ack lands before arrival: the citation trails reality
        // by one round, exactly as the live UI feeds it.
        cmd_tx.send(SimCommand::FixAck { ship_id: "t".into(), seq: 7 }).unwrap();
        let mut arrived = false;
        for _ in 0..200 {
            sim.poll_round(SIM_TICK_SECS).unwrap();
            if evt_rx.try_iter().any(|e| matches!(e, SimEvent::Arrival { .. })) {
                arrived = true;
                break;
            }
        }
        assert!(arrived, "ship arrives");
        drop(sim);
        let arrival = journal_lines(&path)
            .into_iter()
            .find(|e| e["kind"] == "Arrival")
            .expect("arrival journaled");
        assert_eq!(arrival["payload"]["fix_seq"], 7);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn minute_marker_snapshots_acked_seqs() {
        let path = std::env::temp_dir().join("tfg-log-test-marker.jsonl");
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (evt_tx, _evt_rx) = mpsc::channel();
        let journal = Journal::open(path.clone()).unwrap();
        let mut sim = SimSource::new_with_journal(cmd_rx, evt_tx, journal);
        cmd_tx
            .send(SimCommand::TakeControl {
                ship_id: "t".into(),
                pos: ship_at(-5.92, 106.92),
                class_id: "martadinata-sigma-10514-pkr".into(),
            })
            .unwrap();
        cmd_tx.send(SimCommand::FixAck { ship_id: "t".into(), seq: 3 }).unwrap();
        cmd_tx.send(SimCommand::FixAck { ship_id: "u".into(), seq: 5 }).unwrap();
        // 35 two-second game ticks cross minute one (the first stamps
        // the start and moves nothing).
        for _ in 0..35 {
            sim.poll_round(SIM_TICK_SECS).unwrap();
        }
        drop(sim);
        let marker = journal_lines(&path)
            .into_iter()
            .find(|e| e["kind"] == "Marker")
            .expect("marker journaled");
        assert_eq!(marker["payload"]["minute"], 1);
        assert_eq!(marker["payload"]["fix_seqs"]["t"], 3);
        assert_eq!(marker["payload"]["fix_seqs"]["u"], 5);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rotate_journal_starts_a_fresh_file() {
        let dir = std::env::temp_dir();
        let first = dir.join("tfg-log-test-rotate-1.jsonl");
        let second = dir.join("tfg-log-test-rotate-2.jsonl");
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (evt_tx, _evt_rx) = mpsc::channel();
        let mut sim =
            SimSource::new_with_journal(cmd_rx, evt_tx, Journal::open(first.clone()).unwrap());
        cmd_tx.send(SimCommand::TakeControl {
            ship_id: "t".into(),
            pos: ship_at(-5.92, 106.92),
            class_id: "martadinata-sigma-10514-pkr".into(),
        }).unwrap();
        sim.poll_round(0.0).unwrap();
        cmd_tx.send(SimCommand::RotateJournal { path: second.clone() }).unwrap();
        sim.poll_round(0.0).unwrap();
        // One marker each would need a minute; instead check the files:
        // the first holds pre-rotation entries (none: no events yet),
        // so force an entry post-rotation via a refused order on land.
        cmd_tx.send(SimCommand::SetOrder {
            ship_id: "t".into(),
            waypoint: ship_at(-6.5, 107.0), // inland Java (proven land)
            speed_kn: 10.0,
        }).unwrap();
        sim.poll_round(0.0).unwrap();
        drop(sim);
        let first_lines = journal_lines(&first);
        let second_lines = journal_lines(&second);
        // TakeControl journals its placement (#41, replay needs it), so the
        // pre-rotation file holds exactly that entry. Rotation is proven by
        // the refusal landing in the NEW file, not by the old file being empty.
        assert_eq!(first_lines.len(), 1, "take-control journaled before rotation");
        assert_eq!(first_lines[0]["payload"]["event"], "take-control");
        assert!(
            second_lines.iter().any(|e| e["kind"] == "OrderRefused"),
            "post-rotation entries land in the new file"
        );
        std::fs::remove_file(&first).ok();
        std::fs::remove_file(&second).ok();
    }
}
