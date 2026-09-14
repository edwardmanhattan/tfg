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
use crate::clock::GameClock;
use crate::geo::coordinates::GeoPosition;
use crate::geo::track::{Fix, FixSource};

/// Real seconds between sim rounds (the poll cadence). Motion itself
/// advances by GAME time: game_dt = real_dt × ratio (grill #17, ADR-0004).
pub const SIM_TICK_SECS: f64 = 2.0;
/// Distance to a waypoint that counts as arrived.
pub const ARRIVAL_M: f64 = 50.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderState {
    EnRoute,
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
    TakeControl { ship_id: String, pos: GeoPosition },
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
}    /// Sim -> UI, drained per frame.
#[derive(Debug, Clone)]
pub enum SimEvent {
    Orders(Vec<OrderView>),
    Arrival { ship_id: String },
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
}

pub struct SimSource {
    ships: HashMap<String, SimShip>,
    cmd_rx: Receiver<SimCommand>,
    evt_tx: Sender<SimEvent>,
    clock: GameClock,
    last_tick: Option<Instant>,
}

impl SimSource {
    pub fn new(cmd_rx: Receiver<SimCommand>, evt_tx: Sender<SimEvent>) -> Self {
        Self { ships: HashMap::new(), cmd_rx, evt_tx, clock: GameClock::default(), last_tick: None }
    }

    pub fn owned_ids(&self) -> Vec<String> {
        self.ships.keys().cloned().collect()
    }

    fn drain_commands(&mut self) {
        for cmd in self.cmd_rx.try_iter() {
            match cmd {
                SimCommand::TakeControl { ship_id, pos } => {
                    self.ships.entry(ship_id).or_insert(SimShip {
                        pos,
                        heading_deg: 0.0,
                        order: None,
                    });
                }
                SimCommand::Release { ship_id } => {
                    self.ships.remove(&ship_id);
                }
                SimCommand::SetOrder { ship_id, waypoint, speed_kn } => {
                    if let Some(s) = self.ships.get_mut(&ship_id) {
                        s.order = Some(Order {
                            waypoint,
                            speed_kn,
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
                    }
                }
                None => OrderView {
                    ship_id: id.clone(),
                    waypoint: None,
                    ordered_speed_kn: None,
                    state: OrderState::Holding,
                    eta_secs: None,
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
                        // Motion integrates over GAME elapsed seconds.
                        s.pos = s.pos.dead_reckon(s.heading_deg, o.speed_kn, game_dt);
                    }
                }
            }
            fixes.push(Fix {
                ship_id: id.clone(),
                position: s.pos,
                ts: now_ts(),
                heading_deg: Some(s.heading_deg),
                speed_kn: s.order.as_ref().map(|o| o.speed_kn),
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
        let start = ship_at(-6.10, 106.86);
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: start }).unwrap();
        // ~330 m east at ludicrous speed: several 2 s ticks to arrive.
        let wp = ship_at(-6.10, 106.863);
        cmd.send(SimCommand::SetOrder { ship_id: "t".into(), waypoint: wp, speed_kn: 120.0 }).unwrap();
        let first = sim.poll_round(SIM_TICK_SECS).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].source, FixSource::Sim);
        // First round stamps the session start (ADR-0004): emits the ship
        // but moves nothing yet.
        assert_eq!(first[0].position, start);
        // Run until arrival.
        let mut arrived = false;
        for _ in 0..40 {
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
        let start = ship_at(-6.10, 106.86);
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: start }).unwrap();
        cmd.send(SimCommand::SetOrder {
            ship_id: "t".into(),
            waypoint: ship_at(-6.10, 106.90),
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
        let start = ship_at(-6.10, 106.86);
        cmd.send(SimCommand::TakeControl { ship_id: "t".into(), pos: start }).unwrap();
        cmd.send(SimCommand::SetOrder {
            ship_id: "t".into(),
            waypoint: ship_at(-6.10, 106.90),
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
    fn merge_suppresses_wire_for_owned_ships() {
        use crate::backend::FileReplay;
        let (sim, _, cmd) = harness();
        cmd.send(SimCommand::TakeControl { ship_id: "nordwind".into(), pos: ship_at(-6.1, 106.86) })
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
