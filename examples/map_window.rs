//! Spike step 2: single window proving the composition path.
//!
//! A GPUI window owns the command-center chrome (title bar + roster
//! placeholder) and shows the maplibre-rendered harbor frame as an `img`
//! (CPU-image path: PNG bytes -> `gpui::Image` -> `ImageSource::Image`).
//! Render-on-demand: the map is rendered once at startup; camera moves and
//! ship ticks will call for re-render in the ships-layer ticket.
//!
//! Run: `cargo run --example map_window`
//! Needs: real OS (no sandbox), network for tiles, GPU for maplibre.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Application, Context, Image, ImageFormat, IntoElement, Render, Window, WindowOptions, div, img, prelude::*};
use maplibre_native::{CameraUpdate, ImageRendererBuilder, LatLng};

/// Render one static harbor frame to PNG bytes (Hamburg, zoom 11).
fn render_map_png() -> Vec<u8> {
    let mut renderer = ImageRendererBuilder::new()
        .with_size(
            NonZeroU32::new(800).unwrap(),
            NonZeroU32::new(600).unwrap(),
        )
        .build_static_renderer();
    renderer.load_style_from_url(
        &"https://demotiles.maplibre.org/style.json"
            .parse()
            .unwrap(),
    );
    let camera = CameraUpdate::new()
        .center(LatLng {
            lat: 53.5413,
            lng: 9.9842,
        })
        .zoom(11.0);
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        match renderer.render_static(&camera) {
            Ok(image) => {
                let buf = image.as_image();
                let mut png = Vec::new();
                buf.write_to(
                    &mut std::io::Cursor::new(&mut png),
                    image::ImageFormat::Png,
                )
                .unwrap();
                return png;
            }
            Err(e) => {
                if std::time::Instant::now() > deadline {
                    panic!("map render kept failing: {e:?}");
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}

struct MapView {
    map: Arc<Image>,
}

impl Render for MapView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                div()
                    .px_2()
                    .py_1()
                    .child("Command center (spike) — roster: 0 ships"),
            )
            .child(img(self.map.clone()).flex_1().w_full())
    }
}

fn main() {
    let png = render_map_png();
    std::fs::write("target/map_spike.png", &png).unwrap();
    let map = Arc::new(Image::from_bytes(ImageFormat::Png, png));

    Application::new().run(|cx: &mut App| {
        cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|_| MapView { map })
        })
        .unwrap();
        cx.activate(true);
    });
}
