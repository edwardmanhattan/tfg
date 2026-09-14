//! egui command-center shell, live edition over a persistent map scene.
//!
//! - A background poll thread feeds mock fixes into the [`Registry`] at the
//!   v0 cadence; markers glide by wall-clock fraction (`Registry::blend`).
//! - A background map thread owns one persistent [`LiveMap`]: recenter is a
//!   camera update + a few pumped frames (milliseconds), and follow-tracking
//!   is now just repeated camera updates, not pipeline rebuilds.
//! - Frames arrive as raw RGBA into versioned egui textures; overlay
//!   markers/trails/roster are immediate-mode (ADR-0001/0002).
//!
//! Run: `scripts/run-egui-window.sh`

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui;
use tfg::backend::{FileReplay, HttpPoll, PollSource};
use tfg::catalog::Catalog;
use tfg::geo::track::{Fix, FixSource, Registry, TrailBound, should_track};
use tfg::geo::GeoPosition;
use tfg::map_render::LiveMap;
use tfg::map_render::{project_mercator, unproject_mercator};
use tfg::overlay::hit_test;
use tfg::land::Land;
use tfg::sim::{
    MergeSource, OrderRefusal, OrderState, OrderView, SimCommand, SimEvent, SimSource,
};

const MAP_W: f64 = 800.0;
const MAP_H: f64 = 600.0;
const CENTER: (f64, f64) = (-6.108, 106.910);
const ZOOM: f64 = 11.0;
const STYLE: &str = "https://tiles.openfreemap.org/styles/liberty";
/// v0 poll cadence, in seconds.
const POLL_SECS: f64 = 2.0;
/// Pumped frames per recenter on the hot scene.
const RECENTER_PUMP: u32 = 6;

struct ShipMarker {
    id: String,
    x: f64,
    y: f64,
    stale: bool,
    source: FixSource,
    trail: Vec<(f64, f64)>,
}

struct ShipApp {
    map_tex: Option<egui::TextureHandle>,
    map_version: u64,
    /// Viewport center shared by projection and (on swap) the frame.
    center: (f64, f64),
    registry: Registry,
    poll_rx: Receiver<Vec<Fix>>,
    map_req_tx: Option<Sender<(u64, (f64, f64))>>,
    map_resp_rx: Receiver<(u64, (f64, f64), Vec<u8>)>,
    map_seq: u64,
    recentering: Option<String>,
    shutdown: std::sync::Arc<AtomicBool>,
    poll_handle: Option<JoinHandle<()>>,
    map_handle: Option<JoinHandle<()>>,
    last_poll: Instant,
    hidden: HashSet<String>,
    following: Option<String>,
    show_trail: bool,
    selected: Option<String>,
    /// Wall-clock last-seen + fix counts per ship (inspector readout;
    /// stamped as poll rounds arrive, so the geo model stays time-free).
    last_seen: HashMap<String, Instant>,
    fix_count: HashMap<String, usize>,
    /// Sim orders channel + read views (prototype sim loop).
    sim_cmd_tx: Option<Sender<SimCommand>>,
    sim_evt_rx: Receiver<SimEvent>,
    order_views: HashMap<String, OrderView>,
    controlled: HashSet<String>,
    pending_waypoint: Option<(f64, f64)>,
    placing: bool,
    order_speed: f32,
    /// Land test for order validation (ticket #22): land waypoints are
    /// rejected in the UI before they ever reach the sim.
    land: Option<Land>,
    /// Last order refusal from the sim, shown until the next attempt.
    order_warning: Option<String>,
    /// Unit taxonomy (grill #18): class chosen at take-control.
    catalog: Catalog,
    selected_class: usize,
    /// Game clock readout from the sim (ADR-0004: game time is derived
    /// and reported per round; the UI never computes it itself).
    game_elapsed_secs: Option<u64>,
    game_ratio: f64,
    game_paused: bool,
    /// Real + derived game clock readings, humane format, per round.
    real_ts: Option<String>,
    game_ts: Option<String>,
}

impl ShipApp {
    fn ship_color(id: &str) -> egui::Color32 {
        match id {
            "nordwind" => egui::Color32::from_rgb(0x25, 0x63, 0xeb),
            "ostsee" => egui::Color32::from_rgb(0xdc, 0x26, 0x26),
            _ => egui::Color32::from_rgb(0x16, 0xa3, 0x4a),
        }
    }

    /// Drain pending poll rounds, then derive marker geometry for this frame.
    fn markers(&mut self) -> Vec<ShipMarker> {
        let mut rounds = 0;
        for fixes in self.poll_rx.try_iter() {
            rounds += 1;
            for f in &fixes {
                self.last_seen.insert(f.ship_id.clone(), Instant::now());
                *self.fix_count.entry(f.ship_id.clone()).or_insert(0) += 1;
            }
            self.registry.poll(fixes);
        }
        for evt in self.sim_evt_rx.try_iter() {
            match evt {
                SimEvent::Orders(views) => {
                    for v in views {
                        self.order_views.insert(v.ship_id.clone(), v);
                    }
                }
                SimEvent::Clock { game_elapsed_secs, ratio, paused } => {
                    self.game_elapsed_secs = Some(game_elapsed_secs);
                    self.game_ratio = ratio;
                    self.game_paused = paused;
                }
                SimEvent::ClockReadout { real_ts, game_ts, paused } => {
                    self.real_ts = Some(real_ts);
                    self.game_ts = game_ts;
                    self.game_paused = paused;
                }
                SimEvent::OrderRefused { ship_id, reason } => {
                    let why = match reason {
                        OrderRefusal::LandWaypoint => "waypoint is on land",
                        OrderRefusal::LandBetween => "path crosses land",
                    };
                    eprintln!("order refused ({ship_id}): {why}");
                    self.order_warning = Some(format!("{ship_id}: {why}"));
                }
                SimEvent::ShipBlocked { ship_id } => {
                    eprintln!("blocked at coast: {ship_id}");
                }
                SimEvent::Arrival { ship_id } => {
                    eprintln!("arrived {ship_id}");
                }
            }
        }
        if rounds > 0 {
            self.last_poll = Instant::now();
            let ships = self.registry.ships();
            eprintln!(
                "poll: {} ship(s){}",
                ships.len(),
                ships
                    .iter()
                    .filter(|s| s.stale)
                    .map(|s| format!(" [stale: {}]", s.ship_id))
                    .collect::<String>()
            );
        }
        let frac = (self.last_poll.elapsed().as_secs_f64() / POLL_SECS).clamp(0.0, 1.0);
        let center = self.center;
        self.registry
            .ships()
            .iter()
            .map(|s| {
                let pos = self.registry.blend(&s.ship_id, frac).unwrap_or(s.latest.position);
                let (x, y) = project_mercator(pos.latitude, pos.longitude, center, ZOOM, MAP_W, MAP_H);
                let trail = s
                    .trail
                    .iter()
                    .map(|p| project_mercator(p.latitude, p.longitude, center, ZOOM, MAP_W, MAP_H))
                    .collect();
                ShipMarker { id: s.ship_id.clone(), x, y, stale: s.stale, source: s.source, trail }
            })
            .collect()
    }

    /// Apply finished map frames (last-writer-wins by sequence).
    fn drain_map(&mut self, ctx: &egui::Context) {
        for (seq, center, rgba) in self.map_resp_rx.try_iter() {
            if seq == self.map_seq {
                let img = if LiveMap::is_premultiplied() {
                    egui::ColorImage::from_rgba_premultiplied(
                        [MAP_W as usize, MAP_H as usize],
                        &rgba,
                    )
                } else {
                    egui::ColorImage::from_rgba_unmultiplied(
                        [MAP_W as usize, MAP_H as usize],
                        &rgba,
                    )
                };
                self.map_tex = Some(ctx.load_texture(
                    format!("map-{}", self.map_version),
                    img,
                    egui::TextureOptions::LINEAR,
                ));
                self.map_version += 1;
                self.center = center;
                if self.recentering.is_some() {
                    eprintln!("recentered");
                }
                self.recentering = None;
            }
        }
    }

    /// Ask the map thread for a frame centered on `at` for `ship`.
    fn request_frame(&mut self, ship: &str, at: (f64, f64)) {
        let Some(tx) = self.map_req_tx.clone() else {
            return; // shutting down
        };
        self.map_seq += 1;
        self.recentering = Some(ship.to_string());
        eprintln!("recentering on {ship}…");
        let _ = tx.send((self.map_seq, at));
    }
}

impl eframe::App for ShipApp {
    /// Ordered teardown: stop the poll source, close the map request
    /// channel so the map thread drops its scene on its OWN thread, then
    /// join both before eframe tears down GL. Lets background threads run
    /// past this point and the native map core frees out from under them
    /// (the `double free` on close).
    fn on_exit(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.map_req_tx.take();
        if let Some(h) = self.map_handle.take() {
            let _ = h.join();
        }
        if let Some(h) = self.poll_handle.take() {
            let _ = h.join();
        }
        eprintln!("shutdown: threads joined");
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_map(ui.ctx());
        let markers = self.markers();
        ui.ctx().request_repaint_after(Duration::from_millis(100));

        // Space toggles pause: a full hold (ADR-0004). The sim enforces it
        // tick-wise; the UI just forwards the verb.
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) {
            if let Some(tx) = &self.sim_cmd_tx {
                let paused = !self.game_paused;
                let _ = tx.send(SimCommand::SetPaused { paused });
                eprintln!("{}", if paused { "pause" } else { "resume" });
            }
        }

        egui::Panel::left("roster").show(ui, |ui| {
            ui.heading("Command center");
            // Clock block: real + derived game time, humane format
            // (grill #17, ADR-0004). Both readings come from the sim.
            ui.horizontal(|ui| {
                ui.label(format!("UTC {}", self.real_ts.as_deref().unwrap_or("—")));
                if self.game_paused {
                    ui.label(
                        egui::RichText::new("PAUSED")
                            .strong()
                            .color(egui::Color32::YELLOW),
                    );
                }
            });
            ui.horizontal(|ui| {
                let elapsed = self.game_elapsed_secs.unwrap_or(0);
                let g = self.game_ts.as_deref().unwrap_or("—");
                ui.label(format!(
                    "GAME {g} · G+{:02}:{:02} ({:.0}×)",
                    elapsed / 60,
                    elapsed % 60,
                    self.game_ratio
                ));
            });
            ui.label(format!("{} ships — click a name to follow", markers.len()));
            ui.separator();
            ui.checkbox(&mut self.show_trail, "trails");
            ui.separator();
            let mut follow_req: Option<(String, (f64, f64))> = None;
            for m in &markers {
                ui.horizontal(|ui| {
                    let mut shown = !self.hidden.contains(&m.id);
                    if ui.checkbox(&mut shown, "").changed() {
                        if shown {
                            self.hidden.remove(&m.id);
                        } else {
                            self.hidden.insert(m.id.clone());
                        }
                        eprintln!("{}", if shown { "show " } else { "hide " }.to_string() + &m.id);
                    }
                    let mut label = m.id.clone();
                    if Some(&m.id) == self.following.as_ref() {
                        label += " (following)";
                    }
                    if m.stale {
                        label += " (stale)";
                    }
                    if m.source == FixSource::Sim {
                        label += " (sim)";
                    }
                    if ui.selectable_value(&mut self.following, Some(m.id.clone()), label).clicked()
                    {
                        eprintln!("follow {:?}", self.following);
                        self.selected = Some(m.id.clone());
                        if self.following.as_deref() == Some(&m.id) {
                            if let Some(s) =
                                self.registry.ships().iter().find(|s| s.ship_id == m.id)
                            {
                                follow_req = Some((
                                    m.id.clone(),
                                    (s.latest.position.latitude, s.latest.position.longitude),
                                ));
                            }
                        }
                    }
                });
            }
            if ui.small_button("unfollow").clicked() {
                self.following = None;
            }
            ui.separator();
            // Inspector: live readout for the selected ship.
            ui.heading("Inspector");
            let mut close_inspector = false;
            let mut follow_selected: Option<(String, (f64, f64))> = None;
            match self.selected.clone().and_then(|id| {
                self.registry.ships().iter().find(|s| s.ship_id == id).map(|s| (id, s.latest.clone(), s.stale, s.trail.len()))
            }) {
                Some((id, fix, stale, trail_len)) => {
                    let sim_badge = if fix.source == FixSource::Sim { " (sim)" } else { "" };
                    ui.label(format!("ship: {id}{sim_badge}{}", if stale { " (stale)" } else { "" }));
                    ui.label(format!(
                        "pos: {:.6} {:.6}",
                        fix.position.latitude, fix.position.longitude
                    ));
                    ui.label(format!(
                        "hdg/spd: {} / {}",
                        fix.heading_deg.map(|h| format!("{h:.0}°")).as_deref().unwrap_or("—"),
                        fix.speed_kn.map(|s| format!("{s:.0} kn")).as_deref().unwrap_or("—")
                    ));
                    let age = self.last_seen.get(&id).map(|t| t.elapsed().as_secs()).unwrap_or(999);
                    let n = self.fix_count.get(&id).copied().unwrap_or(0);
                    ui.label(format!("last update: {age}s ago · {n} fixes · trail {trail_len}"));
                    ui.label(format!("wire ts: {}", fix.ts));
                    ui.horizontal(|ui| {
                        if ui.small_button("follow").clicked() {
                            follow_selected = Some((
                                id.clone(),
                                (fix.position.latitude, fix.position.longitude),
                            ));
                        }
                        if ui.small_button("close").clicked() {
                            close_inspector = true;
                        }
                    });
                }
                None => {
                    ui.label("click a ship on the map or roster");
                }
            }
            if close_inspector {
                self.selected = None;
            }
            if let Some((ship, at)) = follow_selected {
                self.following = Some(ship.clone());
                self.request_frame(&ship, at);
            }
            ui.separator();
            // Orders (prototype sim loop): take control, place waypoint,
            // commit speed order; sim advances the ship, inspector shows it.
            ui.heading("Orders");
            if let Some(id) = self.selected.clone() {
                if self.controlled.contains(&id) {
                    if let Some(v) = self.order_views.get(&id) {
                        let state = match v.state {
                            OrderState::EnRoute => "en route",
                            OrderState::Blocked => "BLOCKED (land)",
                            OrderState::Arrived => "arrived",
                            OrderState::Holding => "holding",
                        };
                        ui.label(format!("order: {state}"));
                        if let Some(eta) = v.eta_secs {
                            // Game seconds (ADR-0004): motion covers the
                            // distance over game time, so ETA quotes game time.
                            ui.label(format!("eta: G+{}:{:02}", eta / 60, eta % 60));
                        }
                    }
                    // Taxonomy readout (grill #18) + class-capped speed.
                    ui.label(format!(
                        "{} · {} · max {:.0} kn",
                        v.class_id,
                        v.type_label,
                        v.max_speed_kn
                    ));
                    self.order_speed = self.order_speed.min(v.max_speed_kn);
                    ui.add(
                        egui::DragValue::new(&mut self.order_speed)
                            .speed(1.0)
                            .range(0.0..=v.max_speed_kn as f64)
                            .suffix(" kn"),
                    );
                    if ui.small_button(if self.placing { "click map…" } else { "place waypoint" }).clicked() {
                        self.placing = !self.placing;
                    }
                    let can_commit = self
                        .pending_waypoint
                        .is_some_and(|(la, lo)| {
                            self.land
                                .as_ref()
                                .map(|l| l.is_water(&GeoPosition { latitude: la, longitude: lo }))
                                .unwrap_or(true)
                        });
                    if let Some((la, lo)) = self.pending_waypoint {
                        if !can_commit {
                            ui.label(
                                egui::RichText::new("⚠ waypoint on land")
                                    .color(egui::Color32::YELLOW),
                            );
                        }
                    }
                    if let Some(w) = &self.order_warning {
                        ui.label(egui::RichText::new(format!("⚠ {w}")).color(egui::Color32::YELLOW));
                    }
                    if ui.add_enabled(can_commit, egui::Button::new("order")).clicked() {
                        self.order_warning = None;
                        if let Some((la, lo)) = self.pending_waypoint {
                            if let Some(tx) = &self.sim_cmd_tx {
                                let _ = tx.send(SimCommand::SetOrder {
                                    ship_id: id.clone(),
                                    waypoint: GeoPosition { latitude: la, longitude: lo },
                                    speed_kn: self.order_speed,
                                });
                                eprintln!("order {id} -> ({la:.4}, {lo:.4}) @ {} kn", self.order_speed);
                            }
                            self.pending_waypoint = None;
                            self.placing = false;
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("cancel").clicked() {
                            if let Some(tx) = &self.sim_cmd_tx {
                                let _ = tx.send(SimCommand::CancelOrder { ship_id: id.clone() });
                            }
                        }
                        if ui.small_button("release").clicked() {
                            if let Some(tx) = &self.sim_cmd_tx {
                                let _ = tx.send(SimCommand::Release { ship_id: id.clone() });
                            }
                            self.controlled.remove(&id);
                            self.order_views.remove(&id);
                            self.pending_waypoint = None;
                            self.placing = false;
                        }
                    });
                } else {
                    // Take-control with a class selector (grill #18): the
                    // chosen class's stats drive the unit from then on.
                    let ships = self.catalog.ship_classes();
                    let names: Vec<&str> = ships.iter().map(|c| c.name.as_str()).collect();
                    egui::ComboBox::from_label("")
                        .selected_text(
                            names.get(self.selected_class).copied().unwrap_or("—"),
                        )
                        .show_ui(ui, |ui| {
                            for (i, name) in names.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_class, i, *name);
                            }
                        });
                    if ui.small_button("take control").clicked() {
                        if let Some(s) = self.registry.ships().iter().find(|s| s.ship_id == id) {
                            if let Some(tx) = &self.sim_cmd_tx {
                                let class_id = ships
                                    .get(self.selected_class)
                                    .map(|c| c.id.clone())
                                    .unwrap_or_default();
                                let _ = tx.send(SimCommand::TakeControl {
                                    ship_id: id.clone(),
                                    pos: s.latest.position,
                                    class_id,
                                });
                                eprintln!("take control {id}");
                            }
                            self.controlled.insert(id);
                        }
                    }
                }
            } else {
                ui.label("select a ship first");
            }
            if let Some((ship, at)) = follow_req {
                self.request_frame(&ship, at);
            }
            // Follow-tracking: chase the followed ship when it drifts from
            // center. Gated on no re-render in flight, so frames can't pile.
            if self.recentering.is_none() {
                if let Some(id) = self.following.clone() {
                    if let Some(s) = self.registry.ships().iter().find(|s| s.ship_id == id) {
                        let ship_pos = s.latest.position;
                        let center = GeoPosition {
                            latitude: self.center.0,
                            longitude: self.center.1,
                        };
                        if should_track(center, ship_pos) {
                            self.request_frame(&id, (ship_pos.latitude, ship_pos.longitude));
                        }
                    }
                }
            }
            if let Some(ship) = self.recentering.clone() {
                ui.label(format!("centering on {ship}…"));
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(tex) = &self.map_tex {
                let response = ui.add(
                    egui::Image::new(tex)
                        .fit_to_exact_size(egui::vec2(MAP_W as f32, MAP_H as f32))
                        .sense(egui::Sense::click()),
                );
                let rect = response.rect;
                // Map click: place a pending waypoint when arming, else
                // select the nearest visible marker (12px).
                if response.clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let px = (pos.x - rect.min.x) as f64;
                        let py = (pos.y - rect.min.y) as f64;
                        if self.placing {
                            let (la, lo) = unproject_mercator(
                                px, py, self.center, ZOOM, MAP_W, MAP_H,
                            );
                            eprintln!("waypoint preview ({la:.4}, {lo:.4})");
                            self.pending_waypoint = Some((la, lo));
                        } else {
                            let visible: Vec<(String, f64, f64)> = markers
                                .iter()
                                .filter(|m| !self.hidden.contains(&m.id))
                                .map(|m| (m.id.clone(), m.x, m.y))
                                .collect();
                            if let Some(id) = hit_test(&visible, px, py, 12.0) {
                                eprintln!("select {id}");
                                self.selected = Some(id);
                            }
                        }
                    }
                }
                let painter = ui.painter_at(rect);
                for m in &markers {
                    if self.hidden.contains(&m.id) {
                        continue;
                    }
                    let color = if m.stale { egui::Color32::GRAY } else { Self::ship_color(&m.id) };
                    if self.show_trail {
                        for (tx, ty) in &m.trail {
                            painter.circle_filled(
                                rect.min + egui::vec2(*tx as f32, *ty as f32),
                                2.0,
                                color.linear_multiply(0.55),
                            );
                        }
                    }
                    let c = rect.min + egui::vec2(m.x as f32, m.y as f32);
                    painter.circle_filled(c, 8.0, color);
                    painter.circle_stroke(c, 8.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
                    if Some(&m.id) == self.following.as_ref() {
                        painter.circle_stroke(c, 12.0, egui::Stroke::new(2.0, egui::Color32::YELLOW));
                    }
                    if Some(&m.id) == self.selected.as_ref() {
                        painter.circle_stroke(
                            c,
                            12.0,
                            egui::Stroke::new(2.0, egui::Color32::LIGHT_BLUE),
                        );
                    }
                    painter.text(
                        c + egui::vec2(10.0, -10.0),
                        egui::Align2::LEFT_TOP,
                        &m.id,
                        egui::FontId::proportional(12.0),
                        egui::Color32::BLACK,
                    );
                }
                // Waypoint legs: pending preview (white) + committed per
                // owned ship (light blue), drawn from the ship marker.
                let mut legs: Vec<((f64, f64), (f64, f64), egui::Color32)> = Vec::new();
                for m in &markers {
                    if self.hidden.contains(&m.id) {
                        continue;
                    }
                    if let Some(v) = self.order_views.get(&m.id) {
                        if let Some(wp) = v.waypoint {
                            if v.state == OrderState::EnRoute {
                                let (wx, wy) = project_mercator(
                                    wp.latitude, wp.longitude, self.center, ZOOM, MAP_W, MAP_H,
                                );
                                legs.push(((m.x, m.y), (wx, wy), egui::Color32::LIGHT_BLUE));
                            }
                        }
                    }
                }
                if let Some((la, lo)) = self.pending_waypoint {
                    if let Some(id) = self.selected.clone() {
                        if let Some(m) = markers.iter().find(|m| m.id == id) {
                            let (wx, wy) = project_mercator(la, lo, self.center, ZOOM, MAP_W, MAP_H);
                            legs.push(((m.x, m.y), (wx, wy), egui::Color32::WHITE));
                        }
                    }
                }
                for ((x1, y1), (x2, y2), color) in legs {
                    painter.line_segment(
                        [rect.min + egui::vec2(x1 as f32, y1 as f32),
                         rect.min + egui::vec2(x2 as f32, y2 as f32)],
                        egui::Stroke::new(2.0, color),
                    );
                    painter.circle_filled(
                        rect.min + egui::vec2(x2 as f32, y2 as f32),
                        5.0,
                        color,
                    );
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("loading map scene…");
                });
            }
        });
    }
}

fn main() -> eframe::Result<()> {
    // Poll thread owns the backend source; the UI owns the registry.
    // TFG_BACKEND_URL=http://host:port selects HTTP, else file replay.
    // The sim joins every round via MergeSource (disarmed = wire only).
    let shutdown = std::sync::Arc::new(AtomicBool::new(false));
    let (poll_tx, poll_rx) = mpsc::channel();
    let (sim_cmd_tx, sim_cmd_rx) = mpsc::channel::<SimCommand>();
    let (sim_evt_tx, sim_evt_rx) = mpsc::channel::<SimEvent>();
    let ui_sim_cmd_tx = sim_cmd_tx.clone();
    let poll_shutdown = shutdown.clone();
    let poll_handle = std::thread::spawn(move || {
        let fixture = format!("{}/tests/fixtures/tracks.json", env!("CARGO_MANIFEST_DIR"));
        let wire: Box<dyn PollSource> = match std::env::var("TFG_BACKEND_URL") {
            Ok(url) => {
                eprintln!("backend: HTTP {url}");
                match HttpPoll::new(&url) {
                    Ok(h) => Box::new(h),
                    Err(e) => {
                        eprintln!("http backend failed to start: {e}");
                        return;
                    }
                }
            }
            Err(_) => {
                eprintln!("backend: file replay");
                match FileReplay::from_file(&fixture) {
                    Ok(r) => Box::new(r),
                    Err(e) => {
                        eprintln!("fixture failed to load: {e}");
                        return;
                    }
                }
            }
        };
        let mut source = MergeSource::new(wire, SimSource::new(sim_cmd_rx, sim_evt_tx));
        loop {
            match source.poll() {
                Ok(fixes) => {
                    if poll_tx.send(fixes).is_err() {
                        return; // UI gone
                    }
                }
                Err(e) => eprintln!("poll failed (ships keep misses): {e}"),
            }
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(100));
                if poll_shutdown.load(Ordering::SeqCst) {
                    return;
                }
            }
        }
    });

    // Map thread owns the persistent scene; frames come back by channel.
    // It exits when the UI drops its request sender, dropping the scene
    // on this thread (see on_exit) instead of racing process teardown.
    let (map_req_tx, map_req_rx) = mpsc::channel::<(u64, (f64, f64))>();
    let (map_resp_tx, map_resp_rx) = mpsc::channel::<(u64, (f64, f64), Vec<u8>)>();
    let map_handle = std::thread::spawn(move || {
        let mut scene = LiveMap::new(CENTER, ZOOM, MAP_W as u32, MAP_H as u32, STYLE, tfg::map_render::repo_cache_path());
        while let Ok((seq, at)) = map_req_rx.recv() {
            scene.set_center(at, ZOOM);
            scene.pump(RECENTER_PUMP);
            let rgba = scene.frame_rgba();
            if map_resp_tx.send((seq, at, rgba)).is_err() {
                break; // UI gone
            }
        }
    });
    // Initial frame so the window never opens empty-handed for long.
    map_req_tx.send((0, CENTER)).expect("map thread alive");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1040.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "tfg command center (egui)",
        options,
        Box::new(|cc| {
            // Required once: without image loaders, from_bytes fails.
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Ok(Box::new(ShipApp {
                map_tex: None,
                map_version: 0,
                center: CENTER,
                registry: Registry::new(TrailBound::default()),
                poll_rx,
                map_req_tx: Some(map_req_tx),
                map_resp_rx,
                map_seq: 0,
                recentering: None,
                shutdown,
                poll_handle: Some(poll_handle),
                map_handle: Some(map_handle),
                last_poll: Instant::now(),
                hidden: HashSet::new(),
                following: None,
                show_trail: true,
                selected: None,
                last_seen: HashMap::new(),
                fix_count: HashMap::new(),
                sim_cmd_tx: Some(ui_sim_cmd_tx),
                sim_evt_rx,
                order_views: HashMap::new(),
                controlled: HashSet::new(),
                pending_waypoint: None,
                placing: false,
                order_speed: 20.0,
                land: Land::from_default_asset().ok(),
                order_warning: None,
                catalog: Catalog::from_default_asset().expect("catalog asset valid"),
                selected_class: 0,
                game_elapsed_secs: None,
                game_ratio: 1.0,
                game_paused: false,
                real_ts: None,
                game_ts: None,
            }))
        }),
    )
}
