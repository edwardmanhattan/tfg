//! egui command-center shell, live edition.
//!
//! A background thread polls the [`FileReplay`] mock every [`POLL_SECS`]
//! (the v0 cadence) and ships each round over a channel to the UI thread,
//! which ingests it into the [`Registry`]. Markers glide previous -> latest
//! by wall-clock fraction of the poll interval (`Registry::blend`); the map
//! frame itself stays static (overlay approach, ADR-0001) and repaints run
//! at ~10 Hz. Follow highlights only; re-centering needs continuous
//! re-render (deferred).
//!
//! Run: `scripts/run-egui-window.sh`

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui;
use tfg::backend::{FileReplay, PollSource};
use tfg::geo::track::{Fix, Registry, TrailBound};
use tfg::map_render::{project_mercator, render_static_png};

const MAP_W: f64 = 800.0;
const MAP_H: f64 = 600.0;
const CENTER: (f64, f64) = (53.5413, 9.9842);
const ZOOM: f64 = 11.0;
const STYLE: &str = "https://tiles.openfreemap.org/styles/liberty";
/// v0 poll cadence, in seconds.
const POLL_SECS: f64 = 2.0;

struct ShipMarker {
    id: String,
    x: f64,
    y: f64,
    stale: bool,
    trail: Vec<(f64, f64)>,
}

struct ShipApp {
    map_png: Vec<u8>,
    /// Bumped every map swap: egui caches images by URI.
    map_version: u64,
    /// Viewport center shared by projection and (on swap) the frame.
    center: (f64, f64),
    registry: Registry,
    poll_rx: Receiver<Vec<Fix>>,
    recenter_tx: Sender<(u64, (f64, f64), Vec<u8>)>,
    recenter_rx: Receiver<(u64, (f64, f64), Vec<u8>)>,
    recenter_seq: u64,
    recentering: Option<String>,
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
    /// Apply a finished background re-render (last-writer-wins by generation).
    fn drain_recenter(&mut self) {
        for (seq, center, png) in self.recenter_rx.try_iter() {
            if seq == self.recenter_seq {
                self.map_png = png;
                self.center = center;
                self.map_version += 1;
                if self.recentering.is_some() {
                    eprintln!("recentered");
                }
                self.recentering = None;
            }
        }
    }

    /// Start a background map re-render centered on `at` for `ship`.
    fn start_recenter(&mut self, ship: &str, at: (f64, f64)) {
        self.recenter_seq += 1;
        let seq = self.recenter_seq;
        self.recentering = Some(ship.to_string());
        let tx = self.recenter_tx.clone();
        eprintln!("recentering on {ship}…");
        std::thread::spawn(move || {
            let png = render_static_png(at, ZOOM, MAP_W as u32, MAP_H as u32, STYLE);
            std::fs::write("target/map_spike.png", &png).unwrap_or(());
            let _ = tx.send((seq, at, png));
        });
    }
}

impl eframe::App for ShipApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_recenter();
        let markers = self.markers();
        // Overlay glide needs continuous repaints; the map frame is static.
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
                self.start_recenter(&ship, at);
            }
            if let Some(ship) = self.recentering.clone() {
                ui.label(format!("centering on {ship}…"));
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let uri = format!("bytes://map-{}.png", self.map_version);
            let img = egui::Image::from_bytes(uri, self.map_png.clone())
                .fit_to_exact_size(egui::vec2(MAP_W as f32, MAP_H as f32));
            let response = ui.add(img);
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
        });
    }
}

fn main() -> eframe::Result<()> {
    // Map frame is still rendered once at startup (static harbor).
    let png = render_static_png(CENTER, ZOOM, MAP_W as u32, MAP_H as u32, STYLE);
    std::fs::write("target/map_spike.png", &png).unwrap();

    // Poll thread owns the replay; the UI owns the registry.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
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
                    if tx.send(fixes).is_err() {
                        return; // UI gone
                    }
                }
                Err(e) => eprintln!("poll failed (ships keep misses): {e}"),
            }
            std::thread::sleep(Duration::from_secs_f64(POLL_SECS));
        }
    });

    let (recenter_tx, recenter_rx) = mpsc::channel();
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
                map_png: png,
                map_version: 0,
                center: CENTER,
                registry: Registry::new(TrailBound::default()),
                poll_rx: rx,
                recenter_tx,
                recenter_rx,
                recenter_seq: 0,
                recentering: None,
                last_poll: Instant::now(),
                hidden: HashSet::new(),
                following: None,
                show_trail: true,
            }))
        }),
    )
}
