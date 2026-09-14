//! Simulation source (prototype, map #10 ticket #16).
//!
//! The sim IS a poll source (ADR-0003): each round it advances owned ships
//! from their orders and emits synthetic fixes. The registry never sees
//! orders. UI talks to the sim over channels: [`SimCommand`] down,
//! [`SimEvent`] up. [`MergeSource`] joins wire + sim rounds, suppressing
//! wire fixes for sim-owned ships.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use crate::backend::{PollSource, now_ts};
use crate::geo::coordinates::GeoPosition;
use crate::geo::track::{Fix, FixSource};

/// Seconds of simulated motion per emitted fix (one poll round).
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
}

/// Read view of one owned ship for the orders UI.
#[derive(Debug, Clone)]
pub struct OrderView {
    pub ship_id: String,
    pub waypoint: Option<GeoPosition>,
    pub ordered_speed_kn: Option<f32>,
    pub state: OrderState,
    pub eta_secs: Option<u64>,
}

/// Sim -> UI, drained per frame.
#[derive(Debug, Clone)]
pub enum SimEvent {
    Orders(Vec<OrderView>),
    Arrival { ship_id: String },
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
}

impl SimSource {
    pub fn new(cmd_rx: Receiver<SimCommand>, evt_tx: Sender<SimEvent>) -> Self {
        Self { ships: HashMap::new(), cmd_rx, evt_tx }
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
        self.drain_commands();
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
                        s.pos = s.pos.dead_reckon(s.heading_deg, o.speed_kn, SIM_TICK_SECS);
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
        let first = sim.poll().unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].source, FixSource::Sim);
        assert!(first[0].position.distance_m(&start) > 0.0);
        // Run until arrival.
        let mut arrived = false;
        for _ in 0..40 {
            sim.poll().unwrap();
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
