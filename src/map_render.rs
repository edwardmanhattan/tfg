//! Shared map plumbing for the egui shell.
//!
//! One persistent [`LiveMap`] (a `Continuous` renderer) per map thread:
//! the GL context, tile cache, glyph atlas, and shaders stay hot, so a
//! re-render is camera-update + a few pumped frames (milliseconds), not a
//! pipeline rebuild (seconds — the old static path).
//!
//! [`project_mercator`]: WebMercator projection for overlay markers
//! (prototype duplication of the map engine's projection, ADR-0001).

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use maplibre_native::{
    CameraUpdate, Continuous, ImageRenderer, ImageRendererBuilder, LatLng, ResourceOptions,
};

/// Pixel layout of [`LiveMap::frame_rgba`]. Flip if the map renders washed.
const PREMULTIPLIED: bool = true;

/// Repo-committed seed cache (written by `examples/seed_cache.rs`).
/// Never opened writable by the app: runs copy it to [`runtime_cache_path`]
/// first, so everyday map use never dirties the working tree.
pub fn seed_cache_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/tiles-cache.seed.sqlite")
}

/// Writable runtime copy of the tile cache. Restored from the seed when
/// missing (fresh clone, `cargo clean`), then refreshed by use.
pub fn runtime_cache_path() -> PathBuf {
    let runtime =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/tfg-tiles-cache.sqlite");
    if !runtime.is_file() {
        if let Some(parent) = runtime.parent() {
            std::fs::create_dir_all(parent).expect("target dir writable");
        }
        std::fs::copy(seed_cache_path(), &runtime).expect("seed cache present; run seed_cache");
    }
    runtime
}

/// Repo-committed ambient tile cache (seeded by `examples/seed_cache.rs`).
/// The app boots from this without network; misses re-fetch and refresh it.
pub fn repo_cache_path() -> PathBuf {
    runtime_cache_path()
}

/// A persistent map scene: build once, re-render for the process lifetime.
pub struct LiveMap {
    renderer: ImageRenderer<Continuous>,
    w: u32,
    h: u32,
}

impl LiveMap {
    /// Build the scene and pump until the style is loaded.
    pub fn new(
        center: (f64, f64),
        zoom: f64,
        w: u32,
        h: u32,
        style_url: &str,
        cache_path: PathBuf,
    ) -> Self {
        let mut renderer = ImageRendererBuilder::new()
            .with_size(NonZeroU32::new(w).unwrap(), NonZeroU32::new(h).unwrap())
            .with_resource_options(
                ResourceOptions::default()
                .with_cache_path(cache_path)
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

/// World-size base in px at zoom 0 for the overlay math. Calibrated
/// against the renderer (task #42, `examples/calibrate_projection.rs`):
/// a +0.02° latitude pan at zoom 11 moves content 59.0px on the frame
/// vs 29.3px under a 256 base (ratio 2.014) — the MapLibre world is
/// 512-based, so the overlay must be too, or units drift with zoom.
const WORLD_BASE_PX: f64 = 512.0;

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
        let scale = WORLD_BASE_PX * 2f64.powf(zoom);
        let x = (lon + 180.0) / 360.0 * scale;
        let s = (lat.to_radians().tan() + 1.0 / lat.to_radians().cos()).ln();
        let y = (1.0 - s / std::f64::consts::PI) / 2.0 * scale;
        (x, y)
    }
    let (x, y) = world(lat, lon, zoom);
    let (cx, cy) = world(center.0, center.1, zoom);
    (x - cx + w / 2.0, y - cy + h / 2.0)
}

/// Keep the point under the cursor fixed across a zoom step (task #43):
/// the new center puts the cursor's geographic point back under it.
/// Exact inverse of the forward path by construction (see test).
#[allow(clippy::too_many_arguments)]
pub fn anchor_center(
    px: f64,
    py: f64,
    center: (f64, f64),
    zoom: f64,
    new_zoom: f64,
    w: f64,
    h: f64,
) -> (f64, f64) {
    use std::f64::consts::PI;
    let (gla, glo) = unproject_mercator(px, py, center, zoom, w, h);
    let scale = WORLD_BASE_PX * 2f64.powf(new_zoom);
    let gx = (glo + 180.0) / 360.0 * scale;
    let s = (gla.to_radians().tan() + 1.0 / gla.to_radians().cos()).ln();
    let gy = (1.0 - s / PI) / 2.0 * scale;
    let cx = gx - (px - w / 2.0);
    let cy = gy - (py - h / 2.0);
    let lon = cx / scale * 360.0 - 180.0;
    let s2 = (1.0 - 2.0 * cy / scale) * PI;
    let lat = s2.sinh().atan().to_degrees();
    (lat, lon)
}

/// Inverse of [`project_mercator`]: window pixels back to lat/lon.
/// Used for click-to-order waypoints; exact round-trip of the forward path.
#[allow(clippy::too_many_arguments)]
pub fn unproject_mercator(
    px: f64,
    py: f64,
    center: (f64, f64),
    zoom: f64,
    w: f64,
    h: f64,
) -> (f64, f64) {
    use std::f64::consts::PI;
    fn world(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
        let scale = WORLD_BASE_PX * 2f64.powf(zoom);
        let x = (lon + 180.0) / 360.0 * scale;
        let s = (lat.to_radians().tan() + 1.0 / lat.to_radians().cos()).ln();
        let y = (1.0 - s / PI) / 2.0 * scale;
        (x, y)
    }
    let scale = WORLD_BASE_PX * 2f64.powf(zoom);
    let (cx, cy) = world(center.0, center.1, zoom);
    let x = px - w / 2.0 + cx;
    let y = py - h / 2.0 + cy;
    let lon = x / scale * 360.0 - 180.0;
    let s = (1.0 - 2.0 * y / scale) * PI;
    let lat = s.sinh().atan().to_degrees();
    (lat, lon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_keeps_cursor_geo_fixed() {
        let center = (-6.108, 106.910);
        for (px, py) in [(400.0, 300.0), (100.0, 500.0), (700.0, 120.0)] {
            let (gla, glo) = unproject_mercator(px, py, center, 11.0, 800.0, 600.0);
            for new_zoom in [10.0, 11.5, 13.0] {
                let nc = anchor_center(px, py, center, 11.0, new_zoom, 800.0, 600.0);
                let (gla2, glo2) = unproject_mercator(px, py, nc, new_zoom, 800.0, 600.0);
                assert!((gla - gla2).abs() < 1e-9, "lat stable at {new_zoom}");
                assert!((glo - glo2).abs() < 1e-9, "lon stable at {new_zoom}");
            }
        }
    }

    #[test]
    fn unproject_round_trips_project() {
        let center = (-6.108, 106.910);
        for (lat, lon) in [(-6.095, 106.85), (-6.14, 106.82), (-6.0, 107.1)] {
            let (px, py) = project_mercator(lat, lon, center, 11.0, 800.0, 600.0);
            let (la, lo) = unproject_mercator(px, py, center, 11.0, 800.0, 600.0);
            assert!((la - lat).abs() < 1e-9, "lat {la} vs {lat}");
            assert!((lo - lon).abs() < 1e-9, "lon {lo} vs {lon}");
        }
    }
}
