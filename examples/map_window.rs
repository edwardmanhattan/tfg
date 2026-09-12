//! Ships-layer prototype: registry + mock replay wired into the map window.
//!
//! - Drives [`tfg::geo::track::Registry`] from [`tfg::backend::FileReplay`]
//!   (all fixture frames at startup; deterministic).
//! - Markers + trails are GPUI overlay divs positioned by a hand-rolled
//!   WebMercator projection of `displayed_position`. PROTOTYPE CAVEAT: this
//!   duplicates projection (maplibre owns truth per the geo decision);
//!   per ADR-0001 overlays are the sanctioned home (own renderer later).
//!   Overlay keeps this ticket about UI shape, not layer plumbing.
//! - Roster: per-ship show/hide, click-to-follow (highlight only;
//!   re-centering needs continuous re-render), trail toggle, stale badges.
//!
//! Run: `scripts/run-map-window.sh`

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{
    App, Application, Bounds, ClickEvent, Context, Image, ImageFormat, IntoElement, Render,
    Window, WindowBounds, WindowOptions, div, img, prelude::*, px, rgb, size,
};
use tfg::backend::{FileReplay, PollSource};
use tfg::geo::track::{Registry, TrailBound};
use tfg::map_render::{project_mercator, render_static_png};

const PANEL_BG: u32 = 0x1f2937;
const ROW_HOVER_BG: u32 = 0x374151;
const TEXT: u32 = 0xf9fafb;
const DIM_TEXT: u32 = 0x9ca3af;
const MAP_W: f64 = 800.0;
const MAP_H: f64 = 600.0;
const CENTER_LAT: f64 = 53.5413;
const CENTER_LON: f64 = 9.9842;
const ZOOM: f64 = 11.0;

struct ShipMarker {
    id: String,
    x: f64,
    y: f64,
    stale: bool,
    trail: Vec<(f64, f64)>,
}

struct MapView {
    map: Arc<Image>,
    markers: Vec<ShipMarker>,
    hidden: HashSet<String>,
    following: Option<String>,
    show_trail: bool,
}

impl MapView {
    fn ship_color(id: &str) -> u32 {
        match id {
            "nordwind" => 0x2563eb,
            "ostsee" => 0xdc2626,
            _ => 0x16a34a,
        }
    }
}

impl Render for MapView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let roster = div()
            .flex_col()
            .w(px(220.0))
            .p(px(8.0))
            .gap(px(4.0))
            .bg(rgb(PANEL_BG))
            .text_color(rgb(TEXT))
            .child(div().text_sm().child("Command center".to_string()))
            .child(
                div().text_sm().text_color(rgb(DIM_TEXT)).child(format!(
                    "{} ships — click a name to follow",
                    self.markers.len()
                )),
            )
            .child(
                div()
                    .id("trails-toggle")
                    .p(px(4.0))
                    .text_sm()
                    .hover(|s| s.bg(rgb(ROW_HOVER_BG)))
                    .on_click(cx.listener(|view, _e: &ClickEvent, _w, cx| {
                        view.show_trail = !view.show_trail;
                        eprintln!("trails: {}", if view.show_trail { "on" } else { "off" });
                        cx.notify();
                    }))
                    .child(format!("{} trails", if self.show_trail { "[x]" } else { "[ ]" })),
            );

        let roster = self.markers.iter().enumerate().fold(roster, |roster, (i, m)| {
            let id = m.id.clone();
            let id2 = m.id.clone();
            let hidden = self.hidden.contains(&m.id);
            let following = self.following.as_deref() == Some(&m.id);
            roster.child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .p(px(4.0))
                    .text_sm()
                    .hover(|s| s.bg(rgb(ROW_HOVER_BG)))
                    .child(
                        div()
                            .id(("ship-hide", i))
                            .on_click(cx.listener(move |view, _e: &ClickEvent, _w, cx| {
                                if view.hidden.remove(&id) {
                                    eprintln!("show {id}");
                                } else {
                                    view.hidden.insert(id.clone());
                                    eprintln!("hide {id}");
                                }
                                cx.notify();
                            }))
                            .child(format!("{}", if hidden { "[ ]" } else { "[x]" })),
                    )
                    .child(
                        div()
                            .id(("ship-follow", i))
                            .flex_1()
                            .on_click(cx.listener(move |view, _e: &ClickEvent, _w, cx| {
                                view.following = if view.following.as_deref() == Some(&id2) {
                                    eprintln!("unfollow {id2}");
                                    None
                                } else {
                                    eprintln!("follow {id2}");
                                    Some(id2.clone())
                                };
                                cx.notify();
                            }))
                            .child(format!(
                                "{}{}",
                                m.id,
                                if following {
                                    " (following)"
                                } else if m.stale {
                                    " (stale)"
                                } else {
                                    ""
                                }
                            )),
                    ),
            )
        });

        let mut layer = div().relative().w(px(MAP_W as f32)).h(px(MAP_H as f32)).child(
            img(self.map.clone()).w(px(MAP_W as f32)).h(px(MAP_H as f32)),
        );
        for m in &self.markers {
            if self.hidden.contains(&m.id) {
                continue;
            }
            if self.show_trail {
                for (tx, ty) in &m.trail {
                    layer = layer.child(
                        div()
                            .absolute()
                            .left(px(*tx as f32 - 2.0))
                            .top(px(*ty as f32 - 2.0))
                            .w(px(4.0))
                            .h(px(4.0))
                            .rounded_full()
                            .bg(rgb(Self::ship_color(&m.id)))
                            .opacity(0.55),
                    );
                }
            }
            layer = layer
                .child(
                    div()
                        .absolute()
                        .left(px(m.x as f32 - 8.0))
                        .top(px(m.y as f32 - 8.0))
                        .w(px(16.0))
                        .h(px(16.0))
                        .rounded_full()
                        .bg(rgb(Self::ship_color(&m.id)))
                        .border_2()
                        .border_color(rgb(0xffffff))
                        .opacity(if m.stale { 0.45 } else { 1.0 }),
                )
                .child(
                    div()
                        .absolute()
                        .left(px(m.x as f32 + 10.0))
                        .top(px(m.y as f32 - 10.0))
                        .px(px(4.0))
                        .text_xs()
                        .bg(rgb(0x111827))
                        .text_color(rgb(TEXT))
                        .rounded_md()
                        .child(m.id.clone()),
                );
        }

        div()
            .flex()
            .flex_row()
            .size_full()
            .bg(rgb(0x111827))
            .text_color(rgb(TEXT))
            .child(roster)
            .child(layer)
    }
}

fn main() {
    // Drive the registry from the mock replay (deterministic startup state).
    let fixture = format!("{}/tests/fixtures/tracks.json", env!("CARGO_MANIFEST_DIR"));
    let mut replay = FileReplay::from_file(&fixture).expect("fixture loads");
    let mut registry = Registry::new(TrailBound::default());
    let n = replay.frame_count();
    for _ in 0..n {
        let fixes = replay.poll().expect("replay polls");
        registry.poll(fixes);
    }
    let now = registry.ships().iter().map(|s| s.latest.epoch_secs()).max().unwrap_or(0);
    let markers: Vec<ShipMarker> = registry
        .ships()
        .iter()
        .map(|s| {
            let pos = registry.displayed_position(&s.ship_id, now).unwrap_or(s.latest.position);
            let (x, y) = project_mercator(pos.latitude, pos.longitude, (CENTER_LAT, CENTER_LON), ZOOM, MAP_W, MAP_H);
            let trail = s
                .trail
                .iter()
                .map(|p| project_mercator(p.latitude, p.longitude, (CENTER_LAT, CENTER_LON), ZOOM, MAP_W, MAP_H))
                .collect();
            ShipMarker { id: s.ship_id.clone(), x, y, stale: s.stale, trail }
        })
        .collect();
    println!(
        "registry: {} ships, ostsee stale={}",
        markers.len(),
        markers.iter().find(|m| m.id == "ostsee").map(|m| m.stale).unwrap_or(false)
    );

    // Map image + GPUI window (same composition as the spike).
    let png = render_static_png((CENTER_LAT, CENTER_LON), ZOOM, MAP_W as u32, MAP_H as u32, "https://tiles.openfreemap.org/styles/liberty");
    std::fs::write("target/map_spike.png", &png).unwrap();
    let map = Arc::new(Image::from_bytes(ImageFormat::Png, png));

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1040.), px(640.)), cx);
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |_, cx| {
                cx.new(|_| MapView {
                    map: map.clone(),
                    markers,
                    hidden: HashSet::new(),
                    following: None,
                    show_trail: true,
                })
            },
        )
        .unwrap();
        cx.activate(true);
    });
}
