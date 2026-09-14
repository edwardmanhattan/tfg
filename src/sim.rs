//! Simulation source (prototype, map #10 ticket #16).
//!
//! The sim IS a poll source (ADR-0003): each round it advances owned ships
//! from their orders and emits synthetic fixes. The registry never sees
//! orders. UI talks to the sim over channels: [`SimCommand`] down,
//! [`SimEvent`] up. [`MergeSource`] joins wire + sim rounds, suppressing
//! wire fixes for sim-owned ships.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Instant;

use crate::backend::{PollSource, now_ts};
use crate::catalog::{Catalog, Class};
use crate::clock::GameClock;
use crate::geo::coordinates::GeoPosition;
use crate::geo::track::{Fix, FixSource};
use crate::land::Land;

/// Real seconds between sim rounds (the poll cadence). Motion itself
/// advances by GAME time: game_dt = real_dt × ratio (grill #17, ADR-0004).
pub const SIM_TICK_SECS: f64 = 2.0;
/// Distance to a waypoint that counts as arrived.
pub const ARRIVAL_M: f64 = 50.0;

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

/// UI -> sim.
#[derive(Debug, Clone)]
pub enum SimCommand {
    /// Take control with the class whose stats will drive the unit
    /// (grill #18: class holds abilities; chosen at takeover).
    TakeControl { ship_id: String, pos: GeoPosition, class_id: String },
    Release { ship_id: String },
    SetOrder { ship_id: String, waypoint: GeoPosition, speed_kn: f32 },
    CancelOrder { ship_id: String },
    /// Pause is a full hold: motion + game time freeze, wall clock runs on.
    SetPaused { paused: bool },
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
}

/// Sim -> UI, drained per frame.
#[derive(Debug, Clone)]
pub enum SimEvent {
    Orders(Vec<OrderView>),
    Arrival { ship_id: String },
    /// A SetOrder was rejected (land waypoint / land between).
    OrderRefused { ship_id: String, reason: OrderRefusal },
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
    /// Catalog class backing this unit (grill #18): holds its abilities.
    class: Class,
}

pub struct SimSource {
    ships: HashMap<String, SimShip>,
    cmd_rx: Receiver<SimCommand>,
    evt_tx: Sender<SimEvent>,
    clock: GameClock,
    last_tick: Option<Instant>,
    land: Option<Land>,
    catalog: Catalog,
}

impl SimSource {
    pub fn new(cmd_rx: Receiver<SimCommand>, evt_tx: Sender<SimEvent>) -> Self {
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
        Self { ships: HashMap::new(), cmd_rx, evt_tx, clock: GameClock::default(), last_tick: None, land, catalog }
    }

    pub fn owned_ids(&self) -> Vec<String> {
        self.ships.keys().cloned().collect()
    }

    fn drain_commands(&mut self) {
        for cmd in self.cmd_rx.try_iter() {
            match cmd {
                SimCommand::TakeControl { ship_id, pos, class_id } => {
                    // Class chosen at takeover (grill #18); unknown ids
                    // fall back to the first ship class so a bad selector
                    // value can never wedge the sim.
                    let class = self
                        .catalog
                        .class(&class_id)
                        .cloned()
                        .or_else(|| self.catalog.ship_classes().into_iter().next().cloned())
                        .expect("catalog has ship classes");
                    self.ships.entry(ship_id).or_insert_with(|| SimShip {
                        pos,
                        heading_deg: 0.0,
                        order: None,
                        class,
                    });
                }
                SimCommand::Release { ship_id } => {
                    self.ships.remove(&ship_id);
                }
                SimCommand::SetOrder { ship_id, waypoint, speed_kn } => {
                    // Reject at commit (ticket #22): land waypoint, or a
                    // straight path that crosses land.
                    let refusal = self.land.as_ref().and_then(|land| {
                        if !land.is_water(&waypoint) {
                            Some(OrderRefusal::LandWaypoint)
                        } else if let Some(s) = self.ships.get(&ship_id) {
                            (!land.path_is_water(&s.pos, &waypoint))
                                .then_some(OrderRefusal::LandBetween)
                        } else {
                            None
                        }
                    });
                    if let Some(reason) = refusal {
                        let _ = self.evt_tx.send(SimEvent::OrderRefused {
                            ship_id: ship_id.clone(),
                            reason,
                        });
                    } else if let Some(s) = self.ships.get_mut(&ship_id) {
                        // Class caps the order (grill #18): abilities live
                        // on the class; orders cannot exceed capability.
                        let max = Catalog::stat(&s.class, "speed_kn", f64::MAX) as f32;
                        s.order = Some(Order {
                            waypoint,
                            speed_kn: speed_kn.min(max),
                            state: OrderState::EnRoute,
                        });
                    }
                }
                SimCommand::CancelOrder { ship_id } => {
                    if let Some(s) = self.ships.get_mut(&ship_id) {
                        s.order = None;
                    }
                }
                SimCommand::SetPaused { paused } => {
                    self.clock.set_paused(paused);
                }
            }
        }
    }

    fn views(&self) -> Vec<OrderView> {
        let mut out: Vec<OrderView> = self
            .ships
            .iter()
            .map(|(id, s)| match &s.order {
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
                        max_speed_kn: Catalog::stat(&s.class, "speed_kn", f64::MAX) as f32,
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
                    max_speed_kn: Catalog::stat(&s.class, "speed_kn", f64::MAX) as f32,
                },
            })
            .collect();
        out.sort_by(|a, b| a.ship_id.cmp(&b.ship_id));
        out
    }
}

impl PollSource for SimSource {
    fn poll(&mut self) -> Result<Vec<Fix>, String> {
        let now = Instant::now();
        let real_dt = self.last_tick.map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
        self.last_tick = Some(now);
        self.poll_round(real_dt)
    }
}

impl SimSource {
    /// One sim round given `real_dt` real seconds elapsed since the last
    /// round. Split from `poll` so tests drive time deterministically.
    fn poll_round(&mut self, real_dt: f64) -> Result<Vec<Fix>, String> {
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
            if let Some(o) = s.order.as_mut() {
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
                heading_deg: Some(s.heading_deg),
                speed_kn: s
                    .order
                    .as_ref()
                    .map(|o| o.speed_kn)
                    .or(Some(Catalog::stat(&s.class, "speed_kn", 0.0) as f32)),
                source: FixSource::Sim,
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
            let _ = self.evt_tx.send(SimEvent::Arrival { ship_id });
        }
        for ship_id in blocked {
            let _ = self.evt_tx.send(SimEvent::ShipBlocked { ship_id });
        }
        Ok(fixes)
    }
}

/// Joins a wire source with the sim: wire fixes for sim-owned ships are
/// suppressed (owned vs traffic). A wire error yields sim-only output.
pub struct MergeSource {
    wire: Box<dyn PollSource>,
    sim: SimSource,
}

impl MergeSource {
    pub fn new(wire: Box<dyn PollSource>, sim: SimSource) -> Self {
        Self { wire, sim }
    }
}

impl PollSource for MergeSource {
    fn poll(&mut self) -> Result<Vec<Fix>, String> {
        let sim_fixes = self.sim.poll()?;
        let owned = self.sim.owned_ids();
        let mut out = sim_fixes;
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
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: start, class_id: "container".into() }).unwrap();
        // ~3.2 km east across open water at ludicrous speed.
        let wp = ship_at(-5.92, 106.955);
        cmd.send(SimCommand::SetOrder { ship_id: "t".into(), waypoint: wp, speed_kn: 120.0 }).unwrap();
        let first = sim.poll_round(SIM_TICK_SECS).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].source, FixSource::Sim);
        // First round stamps the session start (ADR-0004): emits the ship
        // but moves nothing yet.
        assert_eq!(first[0].position, start);
        // Run until arrival. Order clamps to the container class's
        // 20 kn, so ~3.2 km takes ~160 two-second game ticks.
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
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: start, class_id: "container".into() }).unwrap();
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
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: start, class_id: "container".into() }).unwrap();
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
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: ship_at(-5.8, 106.7), class_id: "container".into() })
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
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: ship_at(-5.9, 106.9), class_id: "container".into() })
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
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: ship_at(-5.8, 106.9), class_id: "destroyer".into() })
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
            class_id: "tanker".into(), // 16 kn class
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
        assert_eq!(v.ordered_speed_kn, Some(16.0), "order clamped to class");
        assert_eq!(v.class_id, "tanker");
        assert_eq!(v.type_label, "Very Large Crude Carrier");
        assert_eq!(v.max_speed_kn, 16.0);
        // Unknown class id falls back to the first ship class.
        let (mut sim, _, cmd) = harness();
        cmd.send(SimCommand::TakeControl {
            ship_id: "t".into(),
            pos: ship_at(-5.92, 106.92),
            class_id: "nonexistent".into(),
        })
        .unwrap();
        sim.poll_round(0.0).unwrap();
        assert_eq!(sim.views()[0].class_id, "tanker", "fallback to first ship class");
    }

    #[test]
    fn merge_suppresses_wire_for_owned_ships() {
        use crate::backend::FileReplay;
        let (sim, _, cmd) = harness();
        cmd.send(SimCommand::TakeControl { ship_id: "nordwind".into(), pos: ship_at(-6.1, 106.86), class_id: "container".into() })
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
}
