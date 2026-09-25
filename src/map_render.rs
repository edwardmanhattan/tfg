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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rusqlite::{params, Connection};

use maplibre_native::{
    CameraUpdate, Continuous, ImageRenderer, ImageRendererBuilder, LatLng, ResourceOptions,
};

/// Pixel layout of [`LiveMap::frame_rgba`]. Flip if the map renders washed.
const PREMULTIPLIED: bool = true;

/// The seed contains a complete offline style/resource set, but the source
/// snapshot was captured with ordinary HTTP expiry timestamps. Pin the seed
/// rows before MapLibre opens the cache: otherwise a fresh Windows install
/// tries to revalidate the style over the network and can never report a
/// style-loaded callback in an offline/CI environment.
const OFFLINE_SEED_EXPIRY: i64 = 4_102_444_800; // 2100-01-01T00:00:00Z

fn pin_offline_seed(path: &Path) -> Result<(), String> {
    let connection = Connection::open(path)
        .map_err(|e| format!("map cache expiry update open failed: {e}"))?;
    connection
        .execute(
            "UPDATE resources SET expires = ?1",
            params![OFFLINE_SEED_EXPIRY],
        )
        .map_err(|e| format!("map resource expiry update failed: {e}"))?;
    connection
        .execute(
            "UPDATE tiles SET expires = ?1",
            params![OFFLINE_SEED_EXPIRY],
        )
        .map_err(|e| format!("map tile expiry update failed: {e}"))?;
    Ok(())
}

/// Validate a MapLibre cache as a self-contained SQLite seed/cache.
pub fn validate_cache(path: &Path) -> Result<(), String> {
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("map cache is not SQLite ({}): {e}", path.display()))?;
    let quick_check: String = connection
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|e| format!("map cache integrity check failed: {e}"))?;
    if quick_check != "ok" {
        return Err(format!("map cache integrity check: {quick_check}"));
    }
    for table in ["tiles", "resources"] {
        let count: i64 = connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .map_err(|e| format!("map cache table {table} missing: {e}"))?;
        if count == 0 {
            return Err(format!("map cache table {table} is empty"));
        }
    }
    Ok(())
}

fn cache_is_usable(path: &Path) -> bool {
    validate_cache(path).is_ok()
}

/// Restore the embedded map seed at `runtime` when it is missing or
/// corrupt, then return the writable cache path. The write is atomic so an
/// interrupted first launch cannot leave a half-seeded SQLite file behind.
pub fn prepare_runtime_cache(runtime: &Path) -> Result<PathBuf, String> {
    if runtime.is_file() && !cache_is_usable(runtime) {
        let backup = runtime.with_extension(format!(
            "sqlite.corrupt-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::rename(runtime, backup).map_err(|e| {
            format!("corrupt map cache could not be moved aside ({}): {e}", runtime.display())
        })?;
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = runtime.as_os_str().to_os_string();
            sidecar.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(sidecar));
        }
    }
    if runtime.is_file() {
        // A cache from an earlier launch may contain the same seed rows with
        // the original HTTP expiry. Keep those rows offline-usable on every
        // launch, not only on a first install.
        pin_offline_seed(runtime)?;
    }
    if !runtime.is_file() {
        let parent = runtime
            .parent()
            .ok_or_else(|| format!("map cache has no parent: {}", runtime.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("map cache directory unavailable ({}): {e}", parent.display()))?;
        let temporary = runtime.with_extension(format!(
            "sqlite.tmp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::write(&temporary, crate::assets::MAP_SEED)
            .map_err(|e| format!("map seed write failed ({}): {e}", temporary.display()))?;
        pin_offline_seed(&temporary)?;
        if let Err(e) = std::fs::rename(&temporary, runtime) {
            // Another first launch won the race. Its complete seed is the
            // desired result; discard this process's temporary copy.
            if runtime.is_file() {
                let _ = std::fs::remove_file(&temporary);
            } else {
                let _ = std::fs::remove_file(&temporary);
                return Err(format!("map seed install failed ({}): {e}", runtime.display()));
            }
        }
    }
    Ok(runtime.to_path_buf())
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

/// Earth's equatorial circumference, metres. The numerator of the
/// ground resolution below, kept as a named constant because a second
/// copy of this number is how a scale drifts by a factor of two.
const EARTH_EQUATOR_M: f64 = 40_075_016.686;

/// Ground resolution at a latitude and zoom, metres per pixel.
///
/// The inverse of `project_mercator`'s scale: the world is
/// `WORLD_BASE_PX * 2^zoom` pixels wide and one full turn of
/// longitude is `EARTH_EQUATOR_M` metres, with the Mercator
/// stretch's `cos(latitude)` on top. Both constants are the ones the
/// projection above already uses, so the two cannot disagree.
///
/// This is the ONLY conversion from real-world metres to map pixels in
/// the client. Nothing else may scale a hull: an image's aspect ratio
/// is a property of a photograph, not a measurement of a ship.
pub fn meters_per_pixel(latitude: f64, zoom: f64) -> f64 {
    // Mercator diverges at the poles; clamping to the Web Mercator
    // limit keeps a hull at 89.999° finite rather than dividing by a
    // vanishing cosine.
    let lat = latitude.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
    EARTH_EQUATOR_M * lat.to_radians().cos() / (WORLD_BASE_PX * 2f64.powf(zoom))
}

/// The Web Mercator cut-off latitude. Mercator is defined to ±85.0511°;
/// past it the projection is clamped by every renderer, and this
/// client follows rather than producing infinities.
const MAX_MERCATOR_LAT: f64 = 85.051_128_78;

/// A hull's on-screen footprint, in pixels, from Minos' measurements.
///
/// `has_scale` is the honesty flag: it is true only when BOTH
/// dimensions were published, because a length without a beam cannot
/// produce a true-to-scale quad. A false `has_scale` leaves the
/// values at zero rather than guessing one from the image — the
/// caller draws a symbol instead.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ProjectedUnitGeometry {
    pub length_px: f64,
    pub beam_px: f64,
    pub has_scale: bool,
    /// Ground resolution this was computed at, kept so a caller can
    /// label a non-scale-aware rendering without recomputing.
    pub meters_per_pixel: f64,
}

/// How a unit is drawn at the current zoom.
///
/// The three levels answer "how much can the operator usefully see",
/// and they are chosen from the unit's PROJECTED FOOTPRINT rather
/// than from the raw zoom: a 180 m barge and a 12 m boat cross the
/// same thresholds at different zooms, because that is when each
/// becomes legible. Selecting on zoom alone would draw every hull on
/// the exercise at one size, which is the uniform-dot map this
/// replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitLod {
    /// A type/category symbol. The image is not loaded or drawn.
    Far,
    /// Overall silhouette — the image at reduced size, or a shape.
    Middle,
    /// The full image, mapped to the unit's real length and beam.
    Near,
}

/// Footprint below which a hull is a symbol, in logical pixels.
const LOD_FAR_MAX_PX: f64 = 16.0;
/// Footprint at which a hull earns its full image.
const LOD_NEAR_MIN_PX: f64 = 48.0;
/// Footprint at which a hull drops back off Near. Below the entry
/// threshold, so a hull sitting exactly on the boundary does not
/// oscillate as the zoom jitters around it.
const LOD_NEAR_EXIT_PX: f64 = 44.0;

/// The largest projected dimension — the one that decides when a hull
/// is big enough to read. Length dominates for every hull, but the
/// maximum is taken so an unusually beamy unit is not judged on a
/// dimension that says nothing about its footprint.
fn footprint_px(g: &ProjectedUnitGeometry) -> f64 {
    g.length_px.max(g.beam_px)
}

/// Choose a level of detail for a unit's projected geometry.
///
/// `current` is the level drawn last frame for THIS unit, and is what
/// makes the selection hysteretic: a hull already at Near stays there
/// until its footprint falls below `LOD_NEAR_EXIT_PX`, so zooming in
/// and out across the boundary cannot make it flicker between two
/// representations. Pass `None` on the first frame.
///
/// A hull with no published size (`has_scale == false`) never reaches
/// Near: there is no true-to-scale image to show, and drawing a
/// photo at an invented size would be a lie about the world. It still
/// gets Middle once it is big enough, because a silhouette is
/// legible without knowing the real dimensions.
pub fn select_unit_lod(
    geometry: &ProjectedUnitGeometry,
    current: Option<UnitLod>,
) -> UnitLod {
    let footprint = footprint_px(geometry);
    if !geometry.has_scale {
        return if footprint >= LOD_FAR_MAX_PX {
            UnitLod::Middle
        } else {
            UnitLod::Far
        };
    }
    match current {
        // Already showing the full image: hold it until clearly too
        // small, not merely below the entry line.
        Some(UnitLod::Near) if footprint >= LOD_NEAR_EXIT_PX => UnitLod::Near,
        _ if footprint >= LOD_NEAR_MIN_PX => UnitLod::Near,
        _ if footprint >= LOD_FAR_MAX_PX => UnitLod::Middle,
        _ => UnitLod::Far,
    }
}

/// Project a hull's real-world measurements onto the map at its own
/// latitude (not the viewport center — a fleet spans latitudes, and
/// using the center would mis-size every hull outside it).
///
/// Returns `has_scale = false` whenever either dimension is missing.
/// Never substitute a default, and never derive a dimension from an
/// image's aspect ratio.
pub fn projected_unit_geometry(
    latitude: f64,
    zoom: f64,
    loa_m: Option<f64>,
    beam_m: Option<f64>,
) -> ProjectedUnitGeometry {
    let mpp = meters_per_pixel(latitude, zoom);
    match (loa_m, beam_m) {
        // A published dimension is a real length; a nonsensical one is
        // treated as unpublished rather than drawn.
        (Some(loa), Some(beam)) if loa > 0.0 && beam > 0.0 => ProjectedUnitGeometry {
            length_px: loa / mpp,
            beam_px: beam / mpp,
            has_scale: true,
            meters_per_pixel: mpp,
        },
        _ => ProjectedUnitGeometry {
            length_px: 0.0,
            beam_px: 0.0,
            has_scale: false,
            meters_per_pixel: mpp,
        },
    }
}

/// Which way a Minos unit image points, in compass degrees.
///
/// The example assets are top-down photographs with the BOW toward the
/// RIGHT edge of the image, so the image's own forward axis reads as
/// east — heading 90°. This is a NAMED CONSTANT, not a convention
/// inferred from pixels or filenames: orientation is a property of
/// how the pictures were drawn, and if Minos ever publishes it the
/// metadata overrides this.
pub const IMAGE_FORWARD_HEADING_DEG: f32 = 90.0;

/// A deliberately round geographic interval for the map's quiet reference
/// grid. It changes at zoom thresholds so the operator gets useful spacing
/// without a dense thicket of lines or a grid that disappears when zoomed in.
pub fn grid_spacing_deg(zoom: f64) -> f64 {
    match zoom {
        z if z < 8.0 => 1.0,
        z if z < 10.0 => 0.5,
        z if z < 12.0 => 0.1,
        z if z < 14.0 => 0.05,
        z if z < 16.0 => 0.02,
        _ => 0.01,
    }
}

/// The compass heading a unit image is drawn at, from the unit's own
/// course and the image's forward axis.
///
/// Subtracting the forward axis is the whole transform: a ship pointing
/// north (0°) is drawn rotated a quarter turn anti-clockwise so the
/// image's right-hand bow points up the screen. `None` heading draws
/// NEUTRAL (no rotation), which is a real state — an un-turned image
/// is not a north-pointing one.
pub fn image_heading(heading_deg: Option<f32>) -> Option<f32> {
    heading_deg.map(|h| (h - IMAGE_FORWARD_HEADING_DEG).rem_euclid(360.0))
}

/// A rotated unit image as four screen-space vertices.
///
/// The quad is built long-axis-first around its projected centre and
/// rotated IN PLACE: the geographic position is never rotated, only
/// the visual object drawn on top of it, so a thumbnail cannot drift
/// off the hull it belongs to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotatedQuad {
    /// Corner 0 is the BOW end of the long axis, then clockwise.
    pub corners: [(f64, f64); 4],
    pub center: (f64, f64),
}

/// Build the quad for a unit image of `length_px` by `beam_px` centred
/// on `center`, rotated to `heading_deg`.
///
/// The bow direction is the difference between the midpoint of the bow
/// edge (corners 0 and 1) and the stern edge, so the long axis is
/// unambiguous even though the corners are not on it.
///
/// Degenerate geometry yields `None` rather than a zero-area quad:
/// the caller draws a symbol instead. An unknown heading is NOT
/// degenerate — the image draws on its native axis.
pub fn rotated_unit_quad(
    center: (f64, f64),
    length_px: f64,
    beam_px: f64,
    heading_deg: Option<f32>,
) -> Option<RotatedQuad> {
    rotated_unit_quad_with_forward_heading(
        center,
        length_px,
        beam_px,
        heading_deg,
        IMAGE_FORWARD_HEADING_DEG,
    )
}

/// Variant used by a versioned `UnitVisual`: the asset's own forward
/// axis is data, not a process-wide assumption. The default wrapper
/// above keeps the public rotation ticket API convenient.
pub fn rotated_unit_quad_with_forward_heading(
    center: (f64, f64),
    length_px: f64,
    beam_px: f64,
    heading_deg: Option<f32>,
    forward_heading_deg: f32,
) -> Option<RotatedQuad> {
    if !(length_px > 0.0) || !(beam_px > 0.0) {
        return None;
    }
    let (cx, cy) = center;
    // No course known: draw on the asset's own forward axis (east),
    // and let the Inspector say the heading is unknown. An un-turned
    // image is a real state, not a north-pointing one.
    let rot = heading_deg
        .map(|heading| (heading - forward_heading_deg).rem_euclid(360.0))
        .unwrap_or(0.0);
    let rad = rot.to_radians() as f64;
    let (sin, cos) = rad.sin_cos();
    let half_l = length_px / 2.0;
    let half_b = beam_px / 2.0;
    // The image's own long axis points +X (east) before rotation, and
    // `rot` is how far the ship has turned from that. Applying the
    // usual screen-space rotation — Y down, so positive is clockwise —
    // to that +X axis gives the direction the bow must end up on.
    let (dir_x, dir_y) = (cos, sin);
    // The short axis is the long one turned a quarter turn.
    let (perp_x, perp_y) = (-sin, cos);

    let corner = |along: f64, across: f64| {
        (
            cx + dir_x * along + perp_x * across,
            cy + dir_y * along + perp_y * across,
        )
    };
    Some(RotatedQuad {
        // Bow end first (bow corners 0 and 1), then clockwise.
        corners: [
            corner(half_l, -half_b),
            corner(half_l, half_b),
            corner(-half_l, half_b),
            corner(-half_l, -half_b),
        ],
        center: (cx, cy),
    })
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
    fn offline_seed_pins_resource_expiry_for_maplibre() {
        let path = std::env::temp_dir().join(format!(
            "tfg-map-cache-expiry-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let connection = Connection::open(&path).expect("cache");
        connection
            .execute_batch(
                "CREATE TABLE resources (expires INTEGER);
                 CREATE TABLE tiles (expires INTEGER);
                 INSERT INTO resources VALUES (0);
                 INSERT INTO tiles VALUES (0);",
            )
            .expect("schema");
        drop(connection);

        pin_offline_seed(&path).expect("pin seed");
        let connection = Connection::open(&path).expect("reopen");
        let resource_expiry: i64 = connection
            .query_row("SELECT expires FROM resources", [], |row| row.get(0))
            .expect("resource expiry");
        let tile_expiry: i64 = connection
            .query_row("SELECT expires FROM tiles", [], |row| row.get(0))
            .expect("tile expiry");
        assert_eq!(resource_expiry, OFFLINE_SEED_EXPIRY);
        assert_eq!(tile_expiry, OFFLINE_SEED_EXPIRY);
        drop(connection);
        std::fs::remove_file(path).ok();
    }

    /// Each zoom level halves the ground a pixel covers, so four
    /// levels make it 16x finer. The LOD thresholds rest on this
    /// being exact rather than approximate.
    #[test]
    fn resolution_halves_with_each_zoom_level() {
        let lat = -6.108; // Jawa
        let z10 = meters_per_pixel(lat, 10.0);
        let z14 = meters_per_pixel(lat, 14.0);
        let z18 = meters_per_pixel(lat, 18.0);
        assert!((z10 / z14 - 16.0).abs() < 1e-9, "four zoom levels is 16x");
        assert!((z14 / z18 - 16.0).abs() < 1e-9);
        assert!(z10 > z14 && z14 > z18, "more zoom, finer ground");
    }

    /// The stretch runs the way cos does: metres-per-pixel is
    /// proportional to cos(latitude), so it FALLS from the equator
    /// toward the poles and a pixel covers less ground up there.
    /// Java is the case that matters for this client.
    #[test]
    fn resolution_shrinks_toward_the_poles() {
        let z = 14.0;
        let equator = meters_per_pixel(0.0, z);
        let jawa = meters_per_pixel(-6.108, z);
        let scotland = meters_per_pixel(57.0, z);
        assert!(equator > jawa, "a pixel covers less ground at the equator");
        assert!(jawa > scotland, "57 deg is further from the equator than 6");
        // Exactly cos, in both directions.
        let by_cos = equator * jawa.to_radians().cos();
        assert!((jawa - by_cos).abs() < 1e-9, "{jawa} vs {by_cos}");
        let by_cos_2 = equator * scotland.to_radians().cos();
        assert!((scotland - by_cos_2).abs() < 1e-9, "{scotland} vs {by_cos_2}");
        // The equator value is the 512-base Web Mercator resolution.
        let expected = EARTH_EQUATOR_M / (WORLD_BASE_PX * 2f64.powf(z));
        assert!((equator - expected).abs() < 1e-6);
        // 512 base, not 256: the equator resolution is half the
        // classic 256-tile figure, exactly as WORLD_BASE_PX's
        // calibration comment claims.
        assert!(
            (equator - 156_543.033_928_04 / 2f64.powi(z + 1)).abs() < 1e-6,
            "equator at z14 is {equator}, expected half of the 256-base figure"
        );
    }

    /// Dimensions project to pixels by dividing by the local
    /// resolution, and the same hull at two zooms differs by exactly
    /// the zoom factor.
    #[test]
    fn geometry_scales_with_zoom_and_latitude() {
        let loa = Some(120.0);
        let beam = Some(16.0);
        let g14 = projected_unit_geometry(-6.108, 14.0, loa, beam);
        let g18 = projected_unit_geometry(-6.108, 18.0, loa, beam);
        assert!(g14.has_scale && g18.has_scale);
        // Zooming in 4 levels multiplies the pixel footprint by 2^4, so
        // the COARSER level is the larger divisor: g18 is 16x g14.
        assert!((g18.length_px / g14.length_px - 16.0).abs() < 1e-9);
        assert!((g18.beam_px / g14.beam_px - 16.0).abs() < 1e-9);
        // Long axis is the long axis, at any zoom.
        assert!(g14.length_px > g14.beam_px);

        // Dividing by a resolution that shrinks poleward means the same
        // hull covers MORE pixels up there — the stretch, stated in
        // the unit the LOD actually reads. Asserted explicitly because
        // this sign is the one most easily written backwards.
        let eq = projected_unit_geometry(0.0, 14.0, loa, beam);
        let jawa = projected_unit_geometry(-6.108, 14.0, loa, beam);
        let scotland = projected_unit_geometry(57.0, 14.0, loa, beam);
        assert!(jawa.length_px > eq.length_px, "6 deg of stretch beats 0");
        assert!(scotland.length_px > jawa.length_px, "57 deg stretches further");
        // And the ratio tracks cos exactly, inversely.
        let ratio = scotland.length_px / eq.length_px;
        let expected = 1.0 / scotland.to_radians().cos();
        assert!((ratio - expected).abs() < 1e-9, "{ratio} vs {expected}");
    }

    /// A hull is either measured or it is not. Half a measurement is
    /// not a measurement, and a zero or a negative is not a length —
    /// either would draw a ship nobody sized.
    #[test]
    fn missing_dimensions_are_unscaled_not_defaulted() {
        let mpp = meters_per_pixel(-6.108, 14.0);
        for (loa, beam) in [
            (None, Some(16.0)),
            (Some(120.0), None),
            (None, None),
            (Some(0.0), Some(16.0)),
            (Some(120.0), Some(-1.0)),
        ] {
            let g = projected_unit_geometry(-6.108, 14.0, loa, beam);
            assert!(!g.has_scale, "{loa:?}/{beam:?} must not claim scale");
            assert_eq!(g.length_px, 0.0);
            assert_eq!(g.beam_px, 0.0);
            // The resolution is still reported so a caller can label a
            // non-scale-aware draw without recomputing.
            assert!((g.meters_per_pixel - mpp).abs() < 1e-12);
        }
    }

    /// Mercator is undefined at the poles; the clamp keeps a hull at
    /// the ice edge finite instead of dividing by ~0.
    #[test]
    fn polar_latitudes_stay_finite() {
        for lat in [90.0, -90.0, 89.999, 180.0] {
            let mpp = meters_per_pixel(lat, 12.0);
            assert!(mpp.is_finite() && mpp > 0.0, "lat {lat} gave {mpp}");
        }
    }

    /// Compass direction the quad's bow points, recovered the way a
    /// reader would: the bow edge is the midpoint of corners 0 and 1.
    /// The test reads the geometry back rather than restating the
    /// implementation, so a sign error cannot hide behind itself.
    fn bow_compass_deg(quad: &RotatedQuad) -> f64 {
        let mid = |a: (f64, f64), b: (f64, f64)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let bow = mid(quad.corners[0], quad.corners[1]);
        let stern = mid(quad.corners[2], quad.corners[3]);
        // Screen Y is down, so north is -Y: atan2(dx, -dy) is compass.
        ((bow.0 - stern.0).atan2(-(bow.1 - stern.1)).to_degrees()).rem_euclid(360.0)
    }

    /// LOD from a footprint in pixels, with no projection involved —
    /// the thresholds are the thing under test.
    fn lod_at(footprint_px: f64, current: Option<UnitLod>) -> UnitLod {
        select_unit_lod(
            &ProjectedUnitGeometry {
                length_px: footprint_px,
                beam_px: 0.0,
                has_scale: true,
                meters_per_pixel: 1.0,
            },
            current,
        )
    }

    /// The three bands, checked at points clearly inside each so the
    /// test is about the bands rather than their edges.
    #[test]
    fn footprint_selects_far_middle_near() {
        assert_eq!(lod_at(4.0, None), UnitLod::Far, "a speck");
        assert_eq!(lod_at(15.0, None), UnitLod::Far, "just under the line");
        assert_eq!(lod_at(20.0, None), UnitLod::Middle, "silhouette");
        assert_eq!(lod_at(47.0, None), UnitLod::Middle, "just under Near");
        assert_eq!(lod_at(60.0, None), UnitLod::Near, "full image");
        assert_eq!(lod_at(400.0, None), UnitLod::Near, "close in");
    }

    /// The band edges themselves, so a threshold change is a
    /// deliberate edit rather than an accident.
    #[test]
    fn thresholds_land_where_documented() {
        assert_eq!(lod_at(LOD_FAR_MAX_PX, None), UnitLod::Middle, "16 enters Middle");
        assert_eq!(lod_at(LOD_NEAR_MIN_PX, None), UnitLod::Near, "48 enters Near");
    }

    /// The hysteresis band: a hull between the exit and entry lines
    /// STAYS at Near when it is already there, and enters Near when it
    /// is not. Without this, a hull parked at 46 px would flip on
    /// every frame of zoom jitter.
    #[test]
    fn near_holds_through_the_jitter_band() {
        let jitter = 46.0;
        assert!(
            exit_line_below_entry_line(),
            "exit line must sit below the entry line or hysteresis inverts"
        );
        // Already Near: stays Near inside the band.
        assert_eq!(lod_at(jitter, Some(UnitLod::Near)), UnitLod::Near);
        // Not yet Near: still Middle inside the band.
        assert_eq!(lod_at(jitter, Some(UnitLod::Middle)), UnitLod::Middle);
        // Below the exit line it drops, hysteresis or not.
        assert_eq!(lod_at(LOD_NEAR_EXIT_PX - 0.1, Some(UnitLod::Near)), UnitLod::Middle);
    }

    /// The invariant the hysteresis depends on, stated as a test so a
    /// future threshold edit cannot invert it unnoticed: the exit line
    /// must sit BELOW the entry line, or a hull would have to shrink to
    /// stay at Near and could never leave.
    fn exit_line_below_entry_line() -> bool {
        LOD_NEAR_EXIT_PX < LOD_NEAR_MIN_PX
    }

    /// The distinct levels a walk passed through, in order, collapsing
    /// repeats. A level that persists across many samples is one
    /// VISIT, not many.
    fn runs(levels: &[UnitLod]) -> Vec<UnitLod> {
        let mut out: Vec<UnitLod> = Vec::new();
        for l in levels {
            if out.last() != Some(l) {
                out.push(*l);
            }
        }
        out
    }

    /// Zooming in and back out must not flicker.
    ///
    /// The failure to guard is an OSCILLATION — a level left and then
    /// re-entered while the footprint moves one way. It is NOT a
    /// repeated sample: a stable band repeats legitimately (Far, Far,
    /// Far is a hull that is simply small), so "no two adjacent
    /// samples match" would reject correct behaviour. What is asserted
    /// is that the sequence of DISTINCT levels is monotonic, which is
    /// the definition of no flicker.
    #[test]
    fn zoom_round_trip_does_not_flicker() {
        // A hull walked from a speck up past Near and back down,
        // sampling the whole range densely.
        let walk: Vec<f64> = (0..=240).map(|i| 5.0 + i as f64 * 0.5).collect();

        let mut level: Option<UnitLod> = None;
        let mut up = Vec::new();
        for fp in &walk {
            level = Some(lod_at(*fp, level));
            up.push(level.expect("level"));
        }
        assert_eq!(runs(&up), vec![UnitLod::Far, UnitLod::Middle, UnitLod::Near]);
        assert_eq!(*up.last().expect("peak"), UnitLod::Near);

        let mut level = Some(*up.last().expect("peak"));
        let mut down = Vec::new();
        for fp in walk.iter().rev() {
            level = Some(lod_at(*fp, level));
            down.push(level.expect("level"));
        }
        assert_eq!(runs(&down), vec![UnitLod::Near, UnitLod::Middle, UnitLod::Far]);
        assert_eq!(*down.last().expect("end"), UnitLod::Far, "back to a speck");
    }

    /// The specific oscillation hysteresis exists to prevent: a hull
    /// inside the band between the exit and entry lines holds whatever
    /// it already had, so jitter cannot bounce it.
    #[test]
    fn hull_in_the_band_keeps_its_level_under_jitter() {
        // Already Near, jittering inside and around the band: stays
        // Near, including above the entry line it would otherwise
        // have needed to climb through.
        let mut near = Some(UnitLod::Near);
        for fp in [48.0_f64, 47.5, 46.0, 45.0, 44.0, 47.0, 45.5, 46.5] {
            near = Some(lod_at(fp, near));
            assert_eq!(near, Some(UnitLod::Near), "stayed Near at {fp}");
        }
        // Already Middle, jittering below the entry line: stays
        // Middle even at the top of the band.
        let mut mid = Some(UnitLod::Middle);
        for fp in [44.0_f64, 44.5, 45.0, 46.0, 47.0, 47.5] {
            mid = Some(lod_at(fp, mid));
            assert_eq!(mid, Some(UnitLod::Middle), "stayed Middle at {fp}");
        }
        // Reaching the entry line from Middle IS an upgrade: the band
        // is for holding a level, never for refusing a real one.
        assert_eq!(lod_at(LOD_NEAR_MIN_PX, Some(UnitLod::Middle)), UnitLod::Near);
    }

    /// A hull with no published size never gets a true-to-scale image,
    /// however close the operator zooms. Middle is allowed because a
    /// silhouette reads without dimensions; Near is not, because it
    /// would draw a photo at a size nobody published.
    #[test]
    fn unscaled_units_never_reach_near() {
        let huge_but_unmeasured = ProjectedUnitGeometry {
            length_px: 0.0,
            beam_px: 0.0,
            has_scale: false,
            meters_per_pixel: 1.0,
        };
        assert_eq!(
            select_unit_lod(&huge_but_unmeasured, None),
            UnitLod::Far,
            "no size, no footprint"
        );
        // Even if a caller hands it a large footprint, has_scale wins.
        let lying = ProjectedUnitGeometry {
            length_px: 500.0,
            beam_px: 80.0,
            has_scale: false,
            meters_per_pixel: 1.0,
        };
        assert_eq!(select_unit_lod(&lying, None), UnitLod::Middle);
        assert_eq!(select_unit_lod(&lying, Some(UnitLod::Near)), UnitLod::Middle);
        assert_eq!(select_unit_lod(&lying, Some(UnitLod::Middle)), UnitLod::Middle);
    }

    /// A hull with only a beam is still a footprint for LOD purposes
    /// only if it is measured — but a lone beam cannot scale an image,
    /// so has_scale stays false and Near stays out of reach. Confirms
    /// the band split is driven by has_scale, not by pixel size.
    #[test]
    fn beam_dominated_footprint_uses_the_maximum() {
        // Length below Far, beam above it: the larger dimension decides.
        let beamy = ProjectedUnitGeometry {
            length_px: 10.0,
            beam_px: 60.0,
            has_scale: true,
            meters_per_pixel: 1.0,
        };
        assert_eq!(select_unit_lod(&beamy, None), UnitLod::Near, "beam dominates");
    }

    /// The four cardinal headings, checked one at a time by name. The
    /// tick ships bow-right, so each quarter turn moves it a quarter
    /// turn on screen; a flipped Y sign or a wrong offset shows up here
    /// as a specific named direction rather than a vague "rotated".
    #[test]
    fn quad_bow_follows_the_compass() {
        let c = (100.0, 100.0);
        for (heading, want, name) in [
            (0.0_f32, 0.0, "north"),
            (90.0, 90.0, "east"),
            (180.0, 180.0, "south"),
            (270.0, 270.0, "west"),
        ] {
            let q = rotated_unit_quad(c, 40.0, 10.0, Some(heading)).expect("quad");
            let got = bow_compass_deg(&q);
            assert!(
                (got - want).abs() < 1e-6,
                "heading {heading} ({name}) pointed {got}, wanted {want}"
            );
        }
    }

    /// The offset is the image's own orientation: a ship heading east
    /// (90°) is the asset's native bow-right, so it must draw
    /// unrotated. If this fails, the constant and the formula
    /// disagree about what "forward" means.
    #[test]
    fn forward_axis_constant_puts_east_on_the_native_axis() {
        let c = (10.0, 10.0);
        let east = rotated_unit_quad(c, 40.0, 10.0, Some(90.0)).expect("quad");
        let neutral = rotated_unit_quad(c, 40.0, 10.0, None).expect("quad");
        for (a, b) in east.corners.iter().zip(neutral.corners.iter()) {
            assert!((a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9, "east is native");
        }
        assert_eq!(image_heading(Some(90.0)), Some(0.0));
        // And the constant is the one the tick ships with.
        assert_eq!(IMAGE_FORWARD_HEADING_DEG, 90.0);
    }

    /// The per-visual forward axis is data, not just the process-wide
    /// default: changing it changes the asset rotation used for the
    /// quad, rather than silently reusing the client constant.
    #[test]
    fn per_visual_forward_axis_changes_only_the_asset_rotation() {
        let default = rotated_unit_quad((0.0, 0.0), 40.0, 10.0, Some(0.0)).expect("default");
        let native = rotated_unit_quad_with_forward_heading(
            (0.0, 0.0),
            40.0,
            10.0,
            Some(0.0),
            0.0,
        )
        .expect("native");
        assert!((bow_compass_deg(&default) - 0.0).abs() < 1e-6);
        assert!((bow_compass_deg(&native) - 90.0).abs() < 1e-6);
    }

    /// An unknown course draws neutral, which is NOT north: the image
    /// keeps its own forward axis and the Inspector reports the
    /// heading as unknown. Silently drawing a bow-up ship would be a
    /// confident lie.
    #[test]
    fn missing_heading_draws_neutral_not_north() {
        let q = rotated_unit_quad((0.0, 0.0), 40.0, 10.0, None).expect("quad");
        assert!((bow_compass_deg(&q) - 90.0).abs() < 1e-6, "native east, not north");
        assert_eq!(image_heading(None), None);
    }

    /// The quad rotates about its centre and never moves the hull:
    /// the mean of the corners is the projected position at every
    /// heading, which is what keeps a thumbnail on its ship.
    #[test]
    fn quad_rotates_in_place_around_its_centre() {
        let c = (640.0, 360.0);
        for heading in [0.0_f32, 37.0, 90.0, 145.0, 180.0, 270.0, 359.0] {
            let q = rotated_unit_quad(c, 120.0, 16.0, Some(heading)).expect("quad");
            assert_eq!(q.center, c, "centre is reported unchanged");
            let mx = q.corners.iter().map(|p| p.0).sum::<f64>() / 4.0;
            let my = q.corners.iter().map(|p| p.1).sum::<f64>() / 4.0;
            assert!((mx - c.0).abs() < 1e-9, "mean x held at {heading}");
            assert!((my - c.1).abs() < 1e-9, "mean y held at {heading}");
        }
    }

    /// The long edge is loa and the short edge is beam, and rotation
    /// never changes either.
    #[test]
    fn quad_preserves_length_and_beam() {
        let (l, b) = (120.0, 16.0);
        for heading in [0.0_f32, 33.0, 90.0, 200.0] {
            let q = rotated_unit_quad((0.0, 0.0), l, b, Some(heading)).expect("quad");
            let bow = ((q.corners[0].0 + q.corners[1].0) / 2.0, (q.corners[0].1 + q.corners[1].1) / 2.0);
            let stern =
                ((q.corners[2].0 + q.corners[3].0) / 2.0, (q.corners[2].1 + q.corners[3].1) / 2.0);
            let len = (bow.0 - stern.0).hypot(bow.1 - stern.1);
            let beam = (q.corners[1].0 - q.corners[0].0).hypot(q.corners[1].1 - q.corners[0].1);
            assert!((len - l).abs() < 1e-9, "long edge {len} at {heading}");
            assert!((beam - b).abs() < 1e-9, "short edge {beam} at {heading}");
        }
    }

    /// A unit with no published size has no true-to-scale quad; the
    /// caller gets None and draws a symbol instead.
    #[test]
    fn degenerate_geometry_yields_no_quad() {
        for (l, b) in [(0.0, 10.0), (10.0, 0.0), (0.0, 0.0), (-1.0, 10.0), (10.0, -1.0)] {
            assert!(
                rotated_unit_quad((0.0, 0.0), l, b, Some(90.0)).is_none(),
                "{l}x{b} must not draw"
            );
        }
    }

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
