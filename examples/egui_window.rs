//! egui command-center shell at parity with the GPUI `map_window`.
//!
//! Same fixture driving the same `Registry`, same openfreemap harbor frame,
//! same projection, same roster shape (show/hide, follow highlight, trail
//! toggle, stale badges). Markers + trails are immediate-mode painter
//! circles over the map image — the overlay approach, egui edition.
//!
//! Run: `scripts/run-egui-window.sh`

use std::collections::HashSet;

use eframe::egui;
use tfg::backend::{FileReplay, PollSource};
use tfg::geo::track::{Registry, TrailBound};
use tfg::map_render::{project_mercator, render_static_png};

const MAP_W: f64 = 800.0;
const MAP_H: f64 = 600.0;
const CENTER: (f64, f64) = (53.5413, 9.9842);
const ZOOM: f64 = 11.0;
const STYLE: &str = "https://tiles.openfreemap.org/styles/liberty";

struct ShipMarker {
    id: String,
    x: f64,
    y: f64,
    stale: bool,
    trail: Vec<(f64, f64)>,
}

struct ShipApp {
    map_png: Vec<u8>,
    markers: Vec<ShipMarker>,
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
}

impl eframe::App for ShipApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("roster").show(ui, |ui| {
            ui.heading("Command center");
            ui.label(format!("{} ships — click a name to follow", self.markers.len()));
            ui.separator();
            ui.checkbox(&mut self.show_trail, "trails");
            ui.separator();
            for m in &self.markers {
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
                    }
                });
            }
            if ui.small_button("unfollow").clicked() {
                self.following = None;
            }
        });

        egui::CentralPanel::default().show(ui, |ui| {
            let img = egui::Image::from_bytes("bytes://map.png", self.map_png.clone())
                .fit_to_exact_size(egui::vec2(MAP_W as f32, MAP_H as f32));
            let response = ui.add(img);
            let rect = response.rect;
            let painter = ui.painter_at(rect);
            for m in &self.markers {
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
    // Same deterministic startup as the GPUI shell.
    let fixture = format!("{}/tests/fixtures/tracks.json", env!("CARGO_MANIFEST_DIR"));
    let mut replay = FileReplay::from_file(&fixture).expect("fixture loads");
    let mut registry = Registry::new(TrailBound::default());
    for _ in 0..replay.frame_count() {
        registry.poll(replay.poll().expect("replay polls"));
    }
    let now = registry.ships().iter().map(|s| s.latest.epoch_secs()).max().unwrap_or(0);
    let markers: Vec<ShipMarker> = registry
        .ships()
        .iter()
        .map(|s| {
            let pos = registry.displayed_position(&s.ship_id, now).unwrap_or(s.latest.position);
            let (x, y) = project_mercator(pos.latitude, pos.longitude, CENTER, ZOOM, MAP_W, MAP_H);
            let trail = s
                .trail
                .iter()
                .map(|p| project_mercator(p.latitude, p.longitude, CENTER, ZOOM, MAP_W, MAP_H))
                .collect();
            ShipMarker { id: s.ship_id.clone(), x, y, stale: s.stale, trail }
        })
        .collect();
    println!(
        "registry: {} ships, ostsee stale={}",
        markers.len(),
        markers.iter().find(|m| m.id == "ostsee").map(|m| m.stale).unwrap_or(false)
    );

    let png = render_static_png(CENTER, ZOOM, MAP_W as u32, MAP_H as u32, STYLE);
    std::fs::write("target/map_spike.png", &png).unwrap();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1040.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "tfg command center (egui)",
        options,
        Box::new(|_cc| {
            Ok(Box::new(ShipApp {
                map_png: png,
                markers,
                hidden: HashSet::new(),
                following: None,
                show_trail: true,
            }))
        }),
    )
}
