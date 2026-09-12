//! Shared map plumbing for the egui shell.
//!
//! One persistent [`LiveMap`] (a `Continuous` renderer) per map thread:
//! the GL context, tile cache, glyph atlas, and shaders stay hot, so a
//! re-render is camera-update + a few pumped frames (milliseconds), not a
//! pipeline rebuild (seconds — the old static path).
//!
//! [`project_mercator`]: WebMercator projection for overlay markers
//! (prototype duplication of the map engine's projection, ADR-0001).
//!
//! NOTE: on systems with libuv >= 1.51 the precompiled core aborts without
//! an older libuv preloaded (see scripts/build-libuv-workaround.sh).

use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use maplibre_native::{
    CameraUpdate, Continuous, ImageRenderer, ImageRendererBuilder, LatLng, ResourceOptions,
};

/// Pixel layout of [`LiveMap::frame_rgba`]. Flip if the map renders washed.
const PREMULTIPLIED: bool = true;

/// A persistent map scene: build once, re-render for the process lifetime.
pub struct LiveMap {
    renderer: ImageRenderer<Continuous>,
    w: u32,
    h: u32,
}

impl LiveMap {
    /// Build the scene and pump until the style is loaded.
    pub fn new(center: (f64, f64), zoom: f64, w: u32, h: u32, style_url: &str) -> Self {
        let mut renderer = ImageRendererBuilder::new()
            .with_size(NonZeroU32::new(w).unwrap(), NonZeroU32::new(h).unwrap())
            .with_resource_options(
                ResourceOptions::default()
                    .with_cache_path(std::env::temp_dir().join("tfg-maplibre-cache"))
                    .with_maximum_cache_size(256 * 1024 * 1024),
            )
            .build_continuous_renderer();
        let loaded = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(AtomicBool::new(false));
        let observer = renderer.map_observer();
        observer.set_did_finish_loading_style_callback({
            let loaded = loaded.clone();
            move || loaded.store(true, Ordering::SeqCst)
        });
        observer.set_did_fail_loading_map_callback({
            let failed = failed.clone();
            move |e| {
                eprintln!("map failed to load: {}", e.message);
                failed.store(true, Ordering::SeqCst);
            }
        });
        renderer.update_camera(
            &CameraUpdate::new().center(LatLng { lat: center.0, lng: center.1 }).zoom(zoom),
        );
        renderer.load_style_from_url(&style_url.parse().unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while !loaded.load(Ordering::SeqCst) {
            if failed.load(Ordering::SeqCst) {
                panic!("map style failed to load");
            }
            if std::time::Instant::now() > deadline {
                panic!("map style never loaded");
            }
            renderer.render_once();
            std::thread::sleep(Duration::from_millis(50));
        }
        let mut map = Self { renderer, w, h };
        // Let the first tiles arrive before anyone reads a frame.
        map.pump(8);
        map
    }

    /// Move the camera; the next [`Self::frame_rgba`] shows the new view.
    pub fn set_center(&mut self, center: (f64, f64), zoom: f64) {
        self.renderer.update_camera(
            &CameraUpdate::new().center(LatLng { lat: center.0, lng: center.1 }).zoom(zoom),
        );
    }

    /// Pump the runloop N frames so async work (tiles) can land.
    pub fn pump(&mut self, n: u32) {
        for _ in 0..n {
            self.renderer.render_once();
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Render one frame; raw RGBA bytes, row-major, `w` x `h`.
    pub fn frame_rgba(&mut self) -> Vec<u8> {
        self.renderer.render_once();
        self.renderer.read_still_image().buffer().to_vec()
    }

    pub fn is_premultiplied() -> bool {
        PREMULTIPLIED
    }

    pub fn dims(&self) -> (u32, u32) {
        (self.w, self.h)
    }
}

/// Project lat/lon to window pixels for a `w` x `h` viewport centered on
/// `center` at `zoom`.
pub fn project_mercator(
    lat: f64,
    lon: f64,
    center: (f64, f64),
    zoom: f64,
    w: f64,
    h: f64,
) -> (f64, f64) {
    fn world(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
        let scale = 256.0 * 2f64.powf(zoom);
        let x = (lon + 180.0) / 360.0 * scale;
        let s = (lat.to_radians().tan() + 1.0 / lat.to_radians().cos()).ln();
        let y = (1.0 - s / std::f64::consts::PI) / 2.0 * scale;
        (x, y)
    }
    let (x, y) = world(lat, lon, zoom);
    let (cx, cy) = world(center.0, center.1, zoom);
    (x - cx + w / 2.0, y - cy + h / 2.0)
}
