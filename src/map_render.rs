//! Shared map plumbing for the egui shell (GPUI shell removed, see ADR-0002).
//!
//! - [`render_static_png`]: one maplibre static frame to PNG bytes. Pumps a
//!   fixed number of frames and returns the LAST one (idle callbacks don't
//!   fire for Static renderers).
//! - [`project_mercator`]: WebMercator (slippy) projection of lat/lon to
//!   window pixels for a viewport center. Prototype duplication of the map
//!   engine's projection (see ADR-0001): overlay markers consume this.
//!
//! NOTE: on systems with libuv >= 1.51 the precompiled core aborts without
//! an older libuv preloaded (see scripts/build-libuv-workaround.sh).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use maplibre_native::{CameraUpdate, ImageRendererBuilder, LatLng};

/// Render one static frame of `style_url` to PNG bytes.
/// Pumps frames until pixels converge (3 identical in a row past a 6-frame
/// minimum, 25 max, 150 ms apart) and returns the LAST one. Idle callbacks
/// don't fire for Static renderers, and early frames are background-only
/// while tiles stream in. Tiles persist in a temp-dir ambient cache, so
/// repeat renders skip the network.
pub fn render_static_png(
    center: (f64, f64),
    zoom: f64,
    w: u32,
    h: u32,
    style_url: &str,
) -> Vec<u8> {
    let mut renderer = ImageRendererBuilder::new()
        .with_size(NonZeroU32::new(w).unwrap(), NonZeroU32::new(h).unwrap())
        .with_resource_options(
            maplibre_native::ResourceOptions::default()
                .with_cache_path(std::env::temp_dir().join("tfg-maplibre-cache"))
                .with_maximum_cache_size(256 * 1024 * 1024),
        )
        .build_static_renderer();
    let failed = Arc::new(AtomicBool::new(false));
    let observer = renderer.map_observer();
    observer.set_did_fail_loading_map_callback({
        let failed = failed.clone();
        move |e| {
            eprintln!("map failed to load: {}", e.message);
            failed.store(true, Ordering::SeqCst);
        }
    });
    renderer.load_style_from_url(&style_url.parse().unwrap());
    let camera = CameraUpdate::new()
        .center(LatLng { lat: center.0, lng: center.1 })
        .zoom(zoom);
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    let mut last = None;
    let (mut stable, mut prev_hash) = (0u32, 0u64);
    for frame in 1..=25 {
        if failed.load(Ordering::SeqCst) {
            panic!("map failed to load");
        }
        match renderer.render_static(&camera) {
            Ok(image) => {
                let mut h = DefaultHasher::new();
                image.as_image().as_raw().hash(&mut h);
                let hash = h.finish();
                stable = if hash == prev_hash { stable + 1 } else { 0 };
                prev_hash = hash;
                last = Some(image);
                if frame >= 6 && stable >= 3 {
                    break; // pixels converged: tiles are in
                }
            }
            Err(e) => eprintln!("render attempt failed (still loading?): {e:?}"),
        }
        if std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    let image = last.expect("no frame rendered at all");
    let buf = image.as_image();
    let mut png = Vec::new();
    buf.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png
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
