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

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui;
use tfg::backend::{FileReplay, PollSource};
use tfg::geo::track::{Fix, Registry, TrailBound, should_track};
use tfg::geo::GeoPosition;
use tfg::map_render::{LiveMap, project_mercator};

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
            self.registry.poll(fixes);
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
                ShipMarker { id: s.ship_id.clone(), x, y, stale: s.stale, trail }
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

        egui::Panel::left("roster").show(ui, |ui| {
            ui.heading("Command center");
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
                    let label = if Some(&m.id) == self.following.as_ref() {
                        format!("{} (following)", m.id)
                    } else if m.stale {
                        format!("{} (stale)", m.id)
                    } else {
                        m.id.clone()
                    };
                    if ui.selectable_value(&mut self.following, Some(m.id.clone()), label).clicked()
                    {
                        eprintln!("follow {:?}", self.following);
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
                    egui::Image::new(tex).fit_to_exact_size(egui::vec2(MAP_W as f32, MAP_H as f32)),
                );
                let rect = response.rect;
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
                    painter.text(
                        c + egui::vec2(10.0, -10.0),
                        egui::Align2::LEFT_TOP,
                        &m.id,
                        egui::FontId::proportional(12.0),
                        egui::Color32::BLACK,
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
    // Poll thread owns the replay; the UI owns the registry.
    let shutdown = std::sync::Arc::new(AtomicBool::new(false));
    let (poll_tx, poll_rx) = mpsc::channel();
    let poll_shutdown = shutdown.clone();
    let poll_handle = std::thread::spawn(move || {
        let fixture = format!("{}/tests/fixtures/tracks.json", env!("CARGO_MANIFEST_DIR"));
        let mut replay = match FileReplay::from_file(&fixture) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("fixture failed to load: {e}");
                return;
            }
        };
        loop {
            match replay.poll() {
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
        let mut scene = LiveMap::new(CENTER, ZOOM, MAP_W as u32, MAP_H as u32, STYLE);
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
            }))
        }),
    )
}
