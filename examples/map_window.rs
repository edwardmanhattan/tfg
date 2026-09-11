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
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui::{App, Application, Context, Image, ImageFormat, IntoElement, Render, Window, WindowOptions, div, img, prelude::*};
use maplibre_native::{CameraUpdate, ImageRendererBuilder, LatLng};

/// Render one static harbor frame to PNG bytes (Hamburg, zoom 11).
/// Pumps a fixed number of frames and returns the LAST one (idle callbacks
/// don't fire for Static).
fn render_map_png() -> Vec<u8> {
    let mut renderer = ImageRendererBuilder::new()
        .with_size(
            NonZeroU32::new(800).unwrap(),
            NonZeroU32::new(600).unwrap(),
        )
        .build_static_renderer();
    let style_loaded = Arc::new(AtomicBool::new(false));
    let idle = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let observer = renderer.map_observer();
    observer.set_did_finish_loading_style_callback({
        let style_loaded = style_loaded.clone();
        move || style_loaded.store(true, Ordering::SeqCst)
    });
    observer.set_did_become_idle_callback({
        let idle = idle.clone();
        move || idle.store(true, Ordering::SeqCst)
    });
    observer.set_did_fail_loading_map_callback({
        let failed = failed.clone();
        move |e| {
            eprintln!("map failed to load: {}", e.message);
            failed.store(true, Ordering::SeqCst);
        }
    });
    renderer.load_style_from_url(
        &"https://tiles.openfreemap.org/styles/liberty"
            .parse()
            .unwrap(),
    );
    let camera = CameraUpdate::new()
        .center(LatLng {
            lat: 53.5413,
            lng: 9.9842,
        })
        .zoom(11.0);
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    let mut last = None;
    for _ in 1..=40 {
        if failed.load(Ordering::SeqCst) {
            panic!("map failed to load");
        }
        match renderer.render_static(&camera) {
            Ok(image) => last = Some(image),
            Err(e) => eprintln!("render attempt failed (still loading?): {e:?}"),
        }
        if std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let image = last.expect("no frame rendered at all");
    let buf = image.as_image();
    let mut png = Vec::new();
    buf.write_to(
        &mut std::io::Cursor::new(&mut png),
        image::ImageFormat::Png,
    )
    .unwrap();
    png
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
