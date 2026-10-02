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

/// The zoom at which the ground resolution is `meters_per_px`.
///
/// The inverse of [`meters_per_pixel`], and what replaced the old
/// `ZONE_ZOOM` constant: framing a group now aims at the zoom where its
/// OWN extent earns a Zone, rather than at one number that decided the
/// same thing for every fleet on earth.
pub fn zoom_for_ground_resolution(latitude: f64, meters_per_px: f64) -> f64 {
    if meters_per_px <= 0.0 {
        return 0.0;
    }
    let lat = latitude.clamp(-MAX_MERCATOR_LAT, MAX_MERCATOR_LAT);
    let world_m = EARTH_EQUATOR_M * lat.to_radians().cos();
    (world_m / (WORLD_BASE_PX * meters_per_px)).log2()
}

/// Whether a unit is PLANNED rather than present, which the symbology
/// draws as a dashed frame.
///
/// "Regardless of affiliation, present status is indicated by a solid
/// line and planned status by a dashed line" — a statement about INTENT:
/// the object will reside at this location. The fact behind it here is a
/// hull that is in the exercise but unassigned in the task organisation
/// (Minos: `id_hierarchy_node` absent), which is a declaration a human
/// made.
///
/// Deliberately NOT "has no fix yet". That is absence of evidence, and
/// it already has a channel — a stale hull greys its glyph and takes the
/// old-data ring. Dashing a stale unit would read as *planned* where the
/// truth is *we have not heard from it*, which is the more dangerous of
/// the two mistakes.
pub fn unit_is_planned(unit_id: Option<i64>, unassigned_units: &std::collections::HashSet<i64>) -> bool {
    unit_id.is_some_and(|id| unassigned_units.contains(&id))
}

/// Whether a unit's name is painted at this Representation.
///
/// Near only, plus focus. The rule needs no density cap to be safe,
/// because the footprint ladder makes Near inherently sparse: a 100 m
/// boat needs about 0.6 m/px to reach Near, where a 1920 px viewport
/// spans roughly a kilometre. "Label at Near" is self-limiting.
///
/// A text width is deliberately NOT an input. A unit marker is a point,
/// so "the text fits inside the marker" is arithmetically dead — a 48 px
/// marker's own extent can never be wider than the name beside it — and
/// a parameter that cannot change the answer is a lie in the signature.
pub fn should_paint_unit_label(lod: UnitLod, focused: bool) -> bool {
    focused || lod == UnitLod::Near
}

/// Whether a group's text is painted, measured against the group's OWN
/// ground extent — the hull's width for a Zone, `2 x cover radius` for a
/// far symbol.
///
/// One rule for both representations, on the quantity the ladder already
/// measures, so a group's two forms cannot drift apart. It degrades
/// honestly: the more ground a group holds, the more room it has to be
/// named. Focus always wins and is uncapped, because an operator can
/// only hold one or two things in focus.
pub fn should_paint_group_text(extent_px: f64, text_px: f32, focused: bool) -> bool {
    focused || text_px as f64 <= extent_px
}

/// Which way a Group is drawn at the current zoom.
///
/// A Group has no hull to size itself by, so this ladder answers a
/// different question from the unit ladder: not "can I see this one
/// object" but "is this formation's ground big enough to be a shape".
/// The two ladders are deliberately independent — a Zone around Far
/// symbols is a true statement, not a contradiction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupRepresentation {
    /// A ground-anchored polygon around the member positions.
    Zone,
    /// A framed symbol at the member centroid.
    Far,
}

/// Screen extent at which a Group earns a Zone, in logical pixels.
///
/// Twice `LOD_NEAR_MIN_PX`, because a Zone has to hold its members,
/// carry its own padding AND stay distinct from the Zone beside it —
/// none of which an image has to do.
pub const GROUP_ZONE_MIN_PX: f64 = 96.0;

/// Screen extent at which a Group drops back off Zone. Below the entry
/// line by the SAME gap the unit ladder keeps (48 -> 44 is 4 px, so
/// 96 -> 92), so a formation sitting on the boundary does not oscillate
/// as the view moves, and the two ladders behave alike.
pub const GROUP_ZONE_EXIT_PX: f64 = 92.0;

/// A Group's ground, measured once per frame.
///
/// `centroid_lat`/`centroid_lon` are the ONE centroid: the ladder
/// measures from it and the far symbol is drawn on it, so the extent is
/// always taken about the point the operator actually sees. With no
/// members there is no centroid and no resolution — `known_members` is
/// zero and the caller draws nothing at all.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ProjectedGroupExtent {
    pub centroid_lat: f64,
    pub centroid_lon: f64,
    pub cover_radius_m: f64,
    pub radius_px: f64,
    pub known_members: usize,
    pub meters_per_pixel: f64,
}

impl ProjectedGroupExtent {
    /// The extent of members that are all hidden or positionless. Every
    /// measurement is zero rather than a stale carry-over, so a caller
    /// that forgets to check `known_members` draws nothing instead of
    /// drawing the last thing it measured.
    fn empty() -> Self {
        ProjectedGroupExtent::default()
    }
}

/// Metres per degree of latitude, taken from the projection's own
/// equator constant, so a group's ground extent and its pixel extent
/// come from one number rather than two conventions.
const METRES_PER_DEGREE: f64 = EARTH_EQUATOR_M / 360.0;

/// Longitudes are a circle: a formation spanning the antimeridian has a
/// naive mean near 0° and a spread near 360°, which would report every
/// such group as theatre-wide. Offsets from the first member are wrapped
/// into ±180°, so the mean is the mean of the members — and the mean is
/// wrapped back too, so the centroid projects at the position the
/// operator sees rather than 0.2° off it.
fn wrap_lon(lon: f64) -> f64 {
    (lon + 180.0).rem_euclid(360.0) - 180.0
}

/// Project a Group's members onto the map and measure their ground.
///
/// The cover radius is the max distance from the member centroid — the
/// radius of the smallest circle holding them — in metres, converted to
/// pixels through the resolution at the CENTROID's latitude rather than
/// the viewport's, mirroring `projected_unit_geometry`: a fleet
/// spanning latitudes is not sized from wherever the operator happens to
/// be looking.
///
/// Positions must be the blended ones the markers are drawn with.
/// Taking raw Fixes here instead would let a Zone describe ground its
/// members are not in yet.
pub fn projected_group_extent(positions: &[(f64, f64)], zoom: f64) -> ProjectedGroupExtent {
    let Some(&(_, first_lon)) = positions.first() else {
        return ProjectedGroupExtent::empty();
    };
    let n = positions.len() as f64;
    let centroid_lat = positions.iter().map(|p| p.0).sum::<f64>() / n;
    let deltas: Vec<f64> = positions
        .iter()
        .map(|p| wrap_lon(p.1 - first_lon))
        .collect();
    let mean_delta = deltas.iter().sum::<f64>() / n;
    let mpp = meters_per_pixel(centroid_lat, zoom);
    let m_per_deg_lon = METRES_PER_DEGREE * centroid_lat.to_radians().cos();
    let cover_radius_m = deltas
        .iter()
        .zip(positions)
        .fold(0.0f64, |worst, (delta, p)| {
            let dy = (p.0 - centroid_lat) * METRES_PER_DEGREE;
            let dx = (delta - mean_delta) * m_per_deg_lon;
            worst.max((dx * dx + dy * dy).sqrt())
        });
    ProjectedGroupExtent {
        centroid_lat,
        centroid_lon: wrap_lon(first_lon + mean_delta),
        cover_radius_m,
        radius_px: if mpp > 0.0 { cover_radius_m / mpp } else { 0.0 },
        known_members: positions.len(),
        meters_per_pixel: mpp,
    }
}

/// Choose how a Group is drawn at the current zoom.
///
/// `current` is the representation drawn last frame for THIS group, and
/// is what makes the selection hysteretic: a Group already at Zone stays
/// there until its cover radius falls below `GROUP_ZONE_EXIT_PX`, so
/// zooming across the boundary cannot make it flicker. Pass `None` on
/// the first frame.
///
/// A Group with fewer than two known members is ALWAYS Far. One member
/// has a cover radius of zero and so would never draw a Zone at any zoom;
/// and a disc drawn around a single ship is decoration, because that
/// ship already carries a marker, a frame and its Amplifier. The extent
/// is deliberately NOT floored by the fattest member's footprint to
/// rescue that case.
pub fn select_group_representation(
    extent: &ProjectedGroupExtent,
    current: Option<GroupRepresentation>,
) -> GroupRepresentation {
    if extent.known_members < 2 {
        return GroupRepresentation::Far;
    }
    match current {
        Some(GroupRepresentation::Zone) if extent.radius_px >= GROUP_ZONE_EXIT_PX => {
            GroupRepresentation::Zone
        }
        _ if extent.radius_px >= GROUP_ZONE_MIN_PX => GroupRepresentation::Zone,
        _ => GroupRepresentation::Far,
    }
}

/// How many points each member contributes to a Zone's dilation.
///
/// The dilation is the convex hull of every member point expanded into
/// a polygon of this many sides, which is exactly the Minkowski sum of
/// the hull with a disc — the true offset, not an approximation of one.
/// Sixteen sides make the guaranteed-inside radius `cos(π/16)` of the
/// nominal pad, and that shortfall is sub-pixel at any pad that matters.
pub const ZONE_CAP_SAMPLES: usize = 16;

/// Margin beyond the members' own drawn size, in logical pixels.
pub const ZONE_MARGIN_PX: f64 = 8.0;

/// The radius a Far unit actually paints at: the base circle every
/// marker carries, under whichever glyph sits on it.
pub const SYMBOL_FOOTPRINT_RADIUS_PX: f64 = 8.0;

/// The radius genuinely inside a Zone polygon for a given pad.
///
/// A `ZONE_CAP_SAMPLES`-gon disc inscribes a circle of `cos(π/N)` times
/// its nominal radius, and every containment claim is written against
/// THIS number rather than against the nominal pad.
pub fn zone_inscribed_radius(pad_px: f64) -> f64 {
    pad_px * (std::f64::consts::PI / ZONE_CAP_SAMPLES as f64).cos()
}

/// How far a Zone is dilated from its member positions.
///
/// A Zone's job is to enclose what it holds, and what it holds are
/// MARKERS, not points: at Far a hull paints as a footprint circle
/// extending well past its position, so a pad sized for the zoom you are
/// looking at lets symbols poke through the hull at the next zoom out.
/// Anchoring the pad to the members' own drawn size keeps the margin
/// constant-looking at every zoom and makes "no marker pokes out" a
/// testable invariant instead of an aesthetic judgement.
pub fn zone_pad_px(fattest_member_footprint_px: f64) -> f64 {
    SYMBOL_FOOTPRINT_RADIUS_PX.max(fattest_member_footprint_px / 2.0) + ZONE_MARGIN_PX
}

/// Monotone-chain convex hull, the same construction the Zone overlay
/// used before it moved here with the dilation that needs it.
fn convex_hull(mut pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    if pts.len() <= 1 {
        return pts;
    }
    pts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let mut lower: Vec<(f64, f64)> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2
            && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0
        {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<(f64, f64)> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2
            && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0
        {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The smallest hit region any symbol may have, in logical pixels.
///
/// EQUAL to `SYMBOL_BOX_PX` on purpose — one minimum constant rather
/// than one per representation, and "you can click exactly what you see"
/// with a floor for the symbols too small to aim at. Before this a Far
/// marker's target was an 18 px RADIUS: a 36 px circle around something
/// painting at about 22 px.
pub const MIN_HIT_PX: f64 = SYMBOL_BOX_PX;

/// Half-width of a Zone's clickable outline band, in logical pixels.
pub const ZONE_HIT_BAND_PX: f64 = 6.0;

/// What fraction of the viewport a double-clicked group's ground should
/// fill when the camera frames it.
///
/// A proposal, not a law: the honest statement is "the group's extent,
/// comfortably framed", and this is the knob for it.
pub const FRAME_VIEWPORT_FRACTION: f64 = 0.6;

/// A painted polygon's hit polygon: the same shape, grown only if it is
/// smaller than `min_px` across.
///
/// You can click exactly what you see, with a floor. The dilation reuses
/// the Zone dilation — the hull of the corners expanded into discs — so
/// a 16 px Middle silhouette becomes aimable without a second shape
/// implementation. A polygon already at or above the minimum is returned
/// unchanged: growing a big photograph's hit region past what is painted
/// is the same lie in the other direction.
pub fn hit_polygon(points: &[(f64, f64)], min_px: f64) -> Vec<(f64, f64)> {
    if points.is_empty() {
        return Vec::new();
    }
    let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(x, y) in points {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let extent = (max_x - min_x).max(max_y - min_y);
    let pad = ((min_px - extent) / 2.0).max(0.0);
    if pad <= 0.0 {
        return points.to_vec();
    }
    zone_polygon(points, pad)
}

/// The zoom at which a group's ground fills `fraction` of the viewport.
///
/// The double-click target, replacing the old clamp to a constant zoom:
/// the camera aims at THIS group's extent, framed to a chosen share of
/// the window, so a 50 km Gugus and a 500 m Unsur both end up legible.
pub fn zoom_for_group_frame(
    latitude: f64,
    cover_radius_m: f64,
    viewport_px: f64,
    fraction: f64,
) -> f64 {
    if cover_radius_m <= 0.0 || viewport_px <= 0.0 || fraction <= 0.0 {
        return 3.0;
    }
    // The group's diameter should span `fraction` of the viewport.
    let target_px = (2.0 * cover_radius_m) / (fraction * viewport_px);
    zoom_for_ground_resolution(latitude, target_px).clamp(3.0, 18.0)
}

/// A Zone polygon's on-screen width: the extent its label is measured
/// against.
///
/// The same measure a far symbol uses as twice its cover radius, so both
/// of a group's representations are labelled by one rule. Zero for an
/// empty polygon rather than a negative infinity, so a caller can never
/// paint a label against a width it never measured.
pub fn zone_width_px(points: &[(f64, f64)]) -> f64 {
    let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
    for &(x, _) in points {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
    }
    if !min_x.is_finite() || !max_x.is_finite() {
        return 0.0;
    }
    max_x - min_x
}

/// Dilate a group's member positions into its Zone polygon.
///
/// This is a REAL convex offset, replacing a pad that pushed each hull
/// vertex radially away from the mean: that shape left a member on an
/// edge midpoint with no margin at all, and gave the same group a fat
/// Zone when spread out and a thin one when tight.
///
/// One code path covers every case: three-plus members give the padded
/// hull, exactly two give the capsule (a segment swept by a disc), and a
/// single member gives the disc — though a one-member Group never draws
/// a Zone at all, because its Representation is always Far.
///
/// `points` must be the blended positions the markers are drawn with.
pub fn zone_polygon(points: &[(f64, f64)], pad_px: f64) -> Vec<(f64, f64)> {
    if points.is_empty() {
        return Vec::new();
    }
    let mut dilated: Vec<(f64, f64)> = Vec::with_capacity(points.len() * ZONE_CAP_SAMPLES);
    for &(x, y) in points {
        for i in 0..ZONE_CAP_SAMPLES {
            let angle = i as f64 * std::f64::consts::TAU / ZONE_CAP_SAMPLES as f64;
            dilated.push((x + pad_px * angle.cos(), y + pad_px * angle.sin()));
        }
    }
    convex_hull(dilated)
}

pub use crate::symbology::Affiliation;

/// Where an object operates, which this symbology carries in the frame's
/// OPENNESS rather than in a fourth shape.
pub use crate::symbology::BattleDimension;

// Every re-export below is the layering move, not tidiness.
//
// `Affiliation` was DEFINED in `store`, a persistence module that pulls
// rusqlite; `BattleDimension` was defined here; and the frame geometry was
// here too, in a module that pulls rusqlite AND maplibre_native. All three
// are domain types, so all three now live in `symbology`, which depends on
// nothing, and this crate depends on `symbology` rather than the reverse.
// That is what lets `proto/p5-epaint` compile the symbology core and run its
// icon generator in about two seconds, with no database and no graphics
// context — reviewing a vocabulary should not cost a MapLibre build, and
// before the frame moved, nothing frame-shaped could reach the harness at all.
//
// The re-exports are what keep every existing `store::Affiliation`,
// `map_render::BattleDimension` and `map_render::frame_polygon` call site
// compiling untouched; they are the thing to delete once the callers have
// moved over.
pub use crate::symbology::frame::{
    SYMBOL_FRAME_ASPECT, SymbolFrame, frame_for, frame_extent, frame_icon_radius, frame_polygon,
    frame_strokes, symbol_box_px,
};

/// A symbol frame's HEIGHT, in logical pixels.
///
/// The width follows from the standard's aspect: friendly, hostile and unknown
/// frames are 1.5 to 1, so this is the vertical measure and
/// [`symbol_box_px`] is what everything downstream sizes against. Every frame
/// is inscribed in that box, so all four have identical bounds — which is what
/// lets hit testing answer with one box per representation instead of four
/// shapes per representation.
///
/// Owned by `symbology` because the icon generator's readability invariants are
/// all fractions of this number: two literals would drift, and a drift would
/// silently re-define what "one stroke" means to the generator while leaving
/// the map unchanged.
pub use crate::symbology::BOX_PX as SYMBOL_BOX_PX;

/// The dimension a symbol's icon puts it in.
///
/// Read from the resolved icon rather than from the taxonomy id, because
/// the icon is what the resolver already decided most-specific-first: a
/// type's own assignment beats its category beats its domain. Note that
/// the only consequential distinctions are airborne and submerged —
/// land and sea surface share a closed frame, so an icon we read
/// coarsely is still framed correctly.
///
/// KNOWN LIMITATION, and the standard makes the point deliberately: it
/// decides dimension by ROLE, not by what the vehicle is — "an Army or
/// Marine helicopter unit is a maneuvering unit … and is thus represented
/// in the land dimension", and a landing craft ferrying troops is sea
/// surface while the same airframe fighting ashore is land. So this
/// reads the icon as a proxy, and Minos publishes nothing that could
/// carry the real thing: no role, mission type or manoeuvre flag, and
/// `movement_domains` is operator free text rather than an enumeration.
/// The honest fix is a domain the backend means, not a better guess
/// here. See issue #180.
pub fn battle_dimension(symbol: crate::store::MapSymbol) -> BattleDimension {
    use crate::store::MapSymbol;
    match symbol {
        MapSymbol::Plane => BattleDimension::AirAndSpace,
        MapSymbol::Submarine => BattleDimension::Subsurface,
        // Ships, and the generic ship: sea surface.
        MapSymbol::Destroyer
        | MapSymbol::Frigate
        | MapSymbol::Corvette
        | MapSymbol::Auxiliary
        | MapSymbol::UnknownShip => BattleDimension::LandAndSeaSurface,
        // Ground units, ports and landing zones: land. Which of these is
        // air rather than land would matter, but both draw closed.
        MapSymbol::GroundUnit | MapSymbol::Port | MapSymbol::Landing => {
            BattleDimension::LandAndSeaSurface
        }
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
    use crate::store::MapSymbol;

    /// An extent at an exact pixel radius, so a test can sit ON a
    /// threshold instead of near one.
    fn extent_at(radius_px: f64, members: usize) -> ProjectedGroupExtent {
        ProjectedGroupExtent {
            radius_px,
            known_members: members,
            ..ProjectedGroupExtent::default()
        }
    }

    fn group_at(radius_px: f64, current: Option<GroupRepresentation>) -> GroupRepresentation {
        select_group_representation(&extent_at(radius_px, 4), current)
    }

    /// A zoom sweep, collapsing runs: a stable band legitimately
    /// repeats (Far, Far, Far is a formation that is simply small), so
    /// an assertion of "no two adjacent samples match" rejects correct
    /// behaviour. That trap is #140's; this is the group version.
    fn distinct_levels(radii: &[f64]) -> Vec<GroupRepresentation> {
        let mut current = None;
        let mut levels: Vec<GroupRepresentation> = Vec::new();
        for radius in radii {
            let level = group_at(*radius, current);
            current = Some(level);
            if levels.last() != Some(&level) {
                levels.push(level);
            }
        }
        levels
    }

    #[test]
    fn both_bands_land_where_documented() {
        // Entry: below the line is Far, at the line is Zone.
        assert_eq!(group_at(GROUP_ZONE_MIN_PX - 0.1, None), GroupRepresentation::Far);
        assert_eq!(group_at(GROUP_ZONE_MIN_PX, None), GroupRepresentation::Zone);
        assert_eq!(group_at(GROUP_ZONE_MIN_PX + 0.1, None), GroupRepresentation::Zone);
        // The exit line only speaks to a Group already at Zone.
        assert_eq!(
            group_at(GROUP_ZONE_EXIT_PX - 0.1, Some(GroupRepresentation::Zone)),
            GroupRepresentation::Far
        );
        assert_eq!(
            group_at(GROUP_ZONE_EXIT_PX, Some(GroupRepresentation::Zone)),
            GroupRepresentation::Zone
        );
        // A Group already Far must climb all the way to the entry line:
        // the band never refuses an upgrade, only delays it.
        assert_eq!(
            group_at(GROUP_ZONE_EXIT_PX + 0.1, Some(GroupRepresentation::Far)),
            GroupRepresentation::Far
        );
        assert_eq!(
            group_at(GROUP_ZONE_MIN_PX, Some(GroupRepresentation::Far)),
            GroupRepresentation::Zone
        );
    }

    #[test]
    fn the_band_is_held_from_both_starting_states() {
        for radius in [GROUP_ZONE_EXIT_PX, 94.0, GROUP_ZONE_MIN_PX - 0.1] {
            assert_eq!(
                group_at(radius, Some(GroupRepresentation::Zone)),
                GroupRepresentation::Zone,
                "a Zone must hold through the band at {radius}"
            );
            assert_eq!(
                group_at(radius, Some(GroupRepresentation::Far)),
                GroupRepresentation::Far,
                "a Far must hold through the band at {radius}"
            );
        }
    }

    #[test]
    fn exit_line_sits_below_entry_line() {
        // Inverting the two would make Zone unescapable, which deserves
        // its own assertion rather than a comment.
        assert!(GROUP_ZONE_EXIT_PX < GROUP_ZONE_MIN_PX);
        assert_eq!(
            (GROUP_ZONE_EXIT_PX - GROUP_ZONE_MIN_PX).abs(),
            (LOD_NEAR_EXIT_PX - LOD_NEAR_MIN_PX).abs(),
            "the group band keeps the unit ladder's 4 px jitter gap"
        );
    }

    #[test]
    fn a_zoom_sweep_does_not_flicker() {
        let radii: Vec<f64> = (0..241)
            .map(|i| 20.0 + i as f64 * 0.4)
            .chain((0..241).rev().map(|i| 20.0 + i as f64 * 0.4))
            .collect();
        assert_eq!(
            distinct_levels(&radii),
            vec![
                GroupRepresentation::Far,
                GroupRepresentation::Zone,
                GroupRepresentation::Far,
            ]
        );
    }

    #[test]
    fn one_member_is_never_a_zone_even_on_a_lying_extent() {
        // Handed a radius that would earn a Zone outright, a
        // single-member Group still resolves Far — and cannot be stuck
        // in Zone by a stale `current`.
        assert_eq!(
            select_group_representation(&extent_at(10_000.0, 1), None),
            GroupRepresentation::Far
        );
        assert_eq!(
            select_group_representation(&extent_at(10_000.0, 1), Some(GroupRepresentation::Zone)),
            GroupRepresentation::Far
        );
    }

    #[test]
    fn no_known_members_yields_nothing_to_draw() {
        let empty = projected_group_extent(&[], 12.0);
        assert_eq!(empty.known_members, 0);
        assert_eq!(empty.cover_radius_m, 0.0);
        assert_eq!(empty.radius_px, 0.0);
        // Even handed a lying extent, a memberless Group is Far.
        assert_eq!(
            select_group_representation(&extent_at(1_000.0, 0), None),
            GroupRepresentation::Far
        );
    }

    #[test]
    fn cover_radius_is_measured_in_metres_at_the_centroid() {
        // Two hulls 0.2° of latitude apart: the radius is half that.
        let extent = projected_group_extent(&[(-6.0, 106.9), (-5.8, 106.9)], 12.0);
        let expected_m = 0.1 * METRES_PER_DEGREE;
        assert!(
            (extent.cover_radius_m - expected_m).abs() < 1.0,
            "{} m",
            extent.cover_radius_m
        );
        assert_eq!(extent.known_members, 2);
        // Pixels are that radius over the resolution at the CENTROID's
        // latitude — the same one the pixels-per-point correction uses.
        assert!((extent.radius_px - expected_m / extent.meters_per_pixel).abs() < 1e-6);
        assert!((extent.centroid_lat - -5.9).abs() < 1e-9);
        assert!((extent.centroid_lon - 106.9).abs() < 1e-9);
    }

    #[test]
    fn a_group_spanning_the_antimeridian_is_not_theatre_wide() {
        // Naive longitudes would average 179.9 and -179.9 to 0 with a
        // spread of 359.8°; a naval exercise across the line must still
        // read as the 0.2° of longitude it actually spans.
        let extent = projected_group_extent(&[(0.0, 179.9), (0.0, -179.9)], 12.0);
        let expected_m = 0.1 * METRES_PER_DEGREE;
        assert!(
            (extent.cover_radius_m - expected_m).abs() < 1.0,
            "{} m",
            extent.cover_radius_m
        );
    }

    #[test]
    fn a_groups_pixels_halve_with_each_zoom_level_out() {
        // The same ground spans TWICE the pixels one level IN, which is
        // the direction that matters: the Zone entry line is crossed by
        // zooming in, not out.
        let members = [(-6.0, 106.9), (-5.8, 106.9)];
        let in_one = projected_group_extent(&members, 13.0);
        let out_one = projected_group_extent(&members, 12.0);
        assert!((in_one.radius_px / out_one.radius_px - 2.0).abs() < 1e-9);
        // The ground did not change; only the pixels over it did.
        assert!((in_one.cover_radius_m - out_one.cover_radius_m).abs() < 1e-9);
    }

    #[test]
    fn a_wide_fleet_earns_its_zone_earlier_than_a_tight_one() {
        // The whole point of a footprint-driven ladder: the same
        // thresholds give different zooms for different formations,
        // rather than one constant deciding for every fleet on earth.
        let wide = projected_group_extent(&[(-6.2, 106.8), (-5.9, 107.1)], 10.0);
        let tight = projected_group_extent(&[(-6.1, 106.89), (-6.099, 106.9)], 10.0);
        assert_eq!(
            select_group_representation(&wide, None),
            GroupRepresentation::Zone
        );
        assert_eq!(
            select_group_representation(&tight, None),
            GroupRepresentation::Far
        );
    }

    /// Inside a convex polygon: every edge keeps its interior to the
    /// left of its outward normal. Written as the invariant the Zone
    /// actually promises — every member point at least the inscribed
    /// radius inside — rather than as a spot check on a few corners.
    fn min_distance_to_edges(point: (f64, f64), poly: &[(f64, f64)]) -> f64 {
        let mut worst = f64::INFINITY;
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-9 {
                continue;
            }
            // Absolute, so the answer does not depend on which way round
            // the ring runs: what is being asked is how far the point is
            // from the boundary, not which side of it the point is on.
            let distance = ((dx * (point.1 - a.1) - dy * (point.0 - a.0)) / len).abs();
            worst = worst.min(distance);
        }
        worst
    }

    fn square_members() -> Vec<(f64, f64)> {
        vec![(0.0, 0.0), (400.0, 0.0), (400.0, 300.0), (0.0, 300.0)]
    }

    #[test]
    fn every_member_sits_inside_the_polygon_by_the_whole_pad() {
        let pad = 24.0;
        let poly = zone_polygon(&square_members(), pad);
        let inscribed = zone_inscribed_radius(pad);
        for member in square_members() {
            let clearance = min_distance_to_edges(member, &poly);
            assert!(
                clearance >= inscribed - 1e-6,
                "member {member:?} clears by {clearance}, needs {inscribed}"
            );
        }
    }

    #[test]
    fn the_polygon_actually_reaches_out_by_the_pad() {
        // The other half of the same promise: a Zone that swallowed its
        // members but grew no wider than they would pass the test above.
        let pad = 24.0;
        let poly = zone_polygon(&square_members(), pad);
        let max_x = poly.iter().map(|p| p.0).fold(f64::MIN, f64::max);
        let min_x = poly.iter().map(|p| p.0).fold(f64::MAX, f64::min);
        let max_y = poly.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        let min_y = poly.iter().map(|p| p.1).fold(f64::MAX, f64::min);
        assert!(max_x >= 400.0 + pad - 1e-9, "right edge {max_x}");
        assert!(min_x <= 0.0 - pad + 1e-9, "left edge {min_x}");
        assert!(max_y >= 300.0 + pad - 1e-9, "top edge {max_y}");
        assert!(min_y <= 0.0 - pad + 1e-9, "bottom edge {min_y}");
    }

    #[test]
    fn the_sixteen_gon_shortfall_never_eats_the_pad() {
        // The geometric shortfall of the cap samples is deliberate and
        // must stay a fraction of the pad, never all of it.
        let pad = 12.0;
        let inscribed = zone_inscribed_radius(pad);
        assert!(inscribed > pad * 0.97, "inscribed {inscribed} of {pad}");
        assert!(inscribed < pad);
        let poly = zone_polygon(&square_members(), pad);
        for member in square_members() {
            assert!(min_distance_to_edges(member, &poly) >= inscribed - 1e-6);
        }
    }

    #[test]
    fn two_members_make_a_capsule_and_three_a_hull() {
        // One code path, no degenerate branch: a pair is a segment swept
        // by a disc, a triple is the triangle grown by the same amount.
        let pad = 20.0;
        let inscribed = zone_inscribed_radius(pad);
        let capsule = zone_polygon(&[(0.0, 0.0), (300.0, 0.0)], pad);
        assert!(capsule.len() > 4, "a capsule is a real polygon");
        for member in [(0.0, 0.0), (300.0, 0.0)] {
            assert!(min_distance_to_edges(member, &capsule) >= inscribed - 1e-6);
        }
        // Its extremes are the segment ends pushed by exactly a pad, and
        // it is exactly one pad deep above and below the segment.
        let right = capsule.iter().map(|p| p.0).fold(f64::MIN, f64::max);
        let top = capsule.iter().map(|p| p.1).fold(f64::MIN, f64::max);
        assert!((right - (300.0 + pad)).abs() < 1e-6, "capsule reaches {right}");
        assert!((top - pad).abs() < 1e-6, "capsule depth {top}");

        let hull = zone_polygon(&[(0.0, 0.0), (300.0, 0.0), (150.0, 200.0)], pad);
        for member in [(0.0, 0.0), (300.0, 0.0), (150.0, 200.0)] {
            assert!(min_distance_to_edges(member, &hull) >= inscribed - 1e-6);
        }
        // A grown triangle keeps three corners and grows no further than
        // a pad past its members. Its vertex COUNT is deliberately not
        // asserted: which samples survive the hull depends on the shape,
        // which is exactly why the containment test above is the real one.
        assert!(hull.len() >= 3 && hull.len() <= 3 * ZONE_CAP_SAMPLES);
        let right = hull.iter().map(|p| p.0).fold(f64::MIN, f64::max);
        assert!(right >= 300.0 && right <= 300.0 + pad, "hull reaches {right}");
    }

    #[test]
    fn a_lone_point_dilates_to_a_disc() {
        // Unreachable from the group ladder — a one-member Group is
        // always Far — but the geometry must still be a real shape
        // rather than a point or a panic.
        let poly = zone_polygon(&[(50.0, 60.0)], 16.0);
        assert!(poly.len() >= 3);
        let max_x = poly.iter().map(|p| p.0).fold(f64::MIN, f64::max);
        let min_x = poly.iter().map(|p| p.0).fold(f64::MAX, f64::min);
        assert!((max_x - min_x - 32.0).abs() < 1e-6);
    }

    #[test]
    fn the_pad_never_lets_a_marker_poke_out() {
        // The invariant the derived pad exists for: a Far unit paints a
        // footprint circle of SYMBOL_FOOTPRINT_RADIUS_PX, so the pad is
        // never smaller than it — at any member size.
        for footprint in [0.0, 4.0, 16.0, 48.0, 400.0] {
            let pad = zone_pad_px(footprint);
            assert!(
                pad >= SYMBOL_FOOTPRINT_RADIUS_PX + ZONE_MARGIN_PX,
                "pad {pad} too small for a {footprint} px member"
            );
            assert!(pad >= footprint / 2.0 + ZONE_MARGIN_PX, "pad {pad} for {footprint}");
        }
        // The fattest member decides, not an average: a 400 px member
        // needs more than the bare symbol radius.
        assert!(zone_pad_px(400.0) > zone_pad_px(0.0));
    }

    /// Distance from a point to a triangle, in METRES — zero inside.
    ///
    /// The Zone corners must sit exactly one pad out from the hull, and
    /// the pad is in pixels while the members are in degrees: both sides
    /// are converted to local metres around the triangle's first vertex
    /// first, or the test compares 0.0069 degrees with 760 metres and
    /// fails while everything is actually right.
    fn distance_to_triangle(point: (f64, f64), tri: &[(f64, f64); 3]) -> f64 {
        let (lat0, lon0) = tri[0];
        let m_per_lon = METRES_PER_DEGREE * lat0.to_radians().cos();
        let to_metres = |p: (f64, f64)| ((p.0 - lat0) * METRES_PER_DEGREE, (p.1 - lon0) * m_per_lon);
        let tri_m: Vec<(f64, f64)> = tri.iter().map(|p| to_metres(*p)).collect();
        let tri_arr = [tri_m[0], tri_m[1], tri_m[2]];
        distance_to_triangle_raw(to_metres(point), &tri_arr)
    }

    fn distance_to_triangle_raw(point: (f64, f64), tri: &[(f64, f64); 3]) -> f64 {
        let edge_distance = |a: (f64, f64), b: (f64, f64)| {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len_sq = dx * dx + dy * dy;
            let t = (((point.0 - a.0) * dx + (point.1 - a.1) * dy) / len_sq).clamp(0.0, 1.0);
            let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
            ((point.0 - cx).powi(2) + (point.1 - cy).powi(2)).sqrt()
        };
        let inside = (0..3).all(|i| {
            let a = tri[i];
            let b = tri[(i + 1) % 3];
            (b.0 - a.0) * (point.1 - a.1) - (b.1 - a.1) * (point.0 - a.0) >= 0.0
        });
        if inside {
            0.0
        } else {
            (0..3)
                .map(|i| edge_distance(tri[i], tri[(i + 1) % 3]))
                .fold(f64::INFINITY, f64::min)
        }
    }

    /// Project member positions at a zoom, for the ground-anchored test.
    fn project_triangle(members: [(f64, f64); 3], zoom: f64, w: f64, h: f64) -> Vec<(f64, f64)> {
        members
            .iter()
            .map(|&(lat, lon)| project_mercator(lat, lon, (0.0, 0.0), zoom, w, h))
            .collect()
    }

    #[test]
    fn the_zones_ground_is_the_same_at_every_zoom() {
        // THE invariant: a Zone is a place, not a drawing. Every corner
        // is a sample point one pad from its own member, so unprojected
        // at any zoom it must sit exactly one pad's worth of METRES off
        // the hull — the same ground, whatever the pixel scale is.
        let members = [(-6.0, 106.9), (-5.8, 106.9), (-5.9, 106.95)];
        let (mw, mh) = (1920.0, 1080.0);
        let pad = 20.0;
        for zoom in [11.0, 13.0, 15.0] {
            let poly = zone_polygon(&project_triangle(members, zoom, mw, mh), pad);
            let pad_m = pad * meters_per_pixel(members[0].0, zoom);
            for corner in &poly {
                let (lat, lon) = unproject_mercator(corner.0, corner.1, (0.0, 0.0), zoom, mw, mh);
                let off = distance_to_triangle((lat, lon), &members);
                // Within 1%: the pad is defined in MERCATOR pixels, and
                // unprojecting onto a flat local metre frame introduces
                // the projection's own sub-percent stretch. The property
                // under test — the same ground at every zoom — holds to
                // well within that.
                assert!(
                    (off - pad_m).abs() < pad_m * 0.01,
                    "corner {corner:?} sits {off} m out, expected {pad_m} m at zoom {zoom}"
                );
            }
        }
    }

    #[test]
    fn an_aircraft_is_not_drawn_as_a_land_unit() {
        // The bug this closes: every frame used to be closed, so a
        // helicopter read as a land unit and a submarine as a surface
        // one. The standard puts the difference in the frame's openness.
        //
        // The other five frame tests moved to `symbology/frame.rs` with the
        // geometry. This one stayed, because what it is really about is
        // `battle_dimension` reading a `MapSymbol` — and that function is a
        // guess about a store enum (see its KNOWN LIMITATION), so it stays
        // until Unit 6 replaces the guess with a real identity.
        assert_eq!(battle_dimension(MapSymbol::Plane), BattleDimension::AirAndSpace);
        assert_eq!(battle_dimension(MapSymbol::Submarine), BattleDimension::Subsurface);
        assert_eq!(battle_dimension(MapSymbol::GroundUnit), BattleDimension::LandAndSeaSurface);
        assert_eq!(battle_dimension(MapSymbol::Destroyer), BattleDimension::LandAndSeaSurface);
        // Every icon in the taxonomy resolves to SOME dimension, so no
        // symbol can fall through unpainted.
        for symbol in MapSymbol::ALL {
            let dimension = battle_dimension(symbol);
            let strokes =
                frame_strokes(dimension, frame_for(Affiliation::Friendly), (0.0, 0.0), SYMBOL_BOX_PX);
            assert!(!strokes.is_empty(), "{symbol:?} produced no frame");
        }
    }

        #[test]
    fn a_hit_region_is_never_smaller_than_the_minimum() {
        // A Middle silhouette far below the minimum becomes aimable.
        let tiny = vec![(0.0, 0.0), (16.0, 0.0), (16.0, 6.0), (0.0, 6.0)];
        let grown = hit_polygon(&tiny, MIN_HIT_PX);
        let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
        let (mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
        for &(x, y) in &grown {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        // On its LARGEST dimension: a long thin hull grows along its
        // length, not into a 22 px blob. Making a 120 m destroyer
        // square would be a second lie in the opposite direction.
        assert!(max_x - min_x >= MIN_HIT_PX - 1e-6, "grown width {}", max_x - min_x);
        assert!(
            (max_y - min_y).abs() <= 6.0 + 2.0 * (MIN_HIT_PX - 16.0) / 2.0,
            "a thin hull must not be inflated into a blob: height {}",
            max_y - min_y
        );
        // And the original shape is still inside it: growth never moves
        // the boundary inward.
        for corner in &tiny {
            assert!(grown.contains(corner) || near_any(corner, &grown));
        }
        // A polygon already big enough is returned AS IS: a near-zoom
        // photograph's target is the photograph, not a padded version.
        let big = vec![(0.0, 0.0), (120.0, 0.0), (120.0, 60.0), (0.0, 60.0)];
        assert_eq!(hit_polygon(&big, MIN_HIT_PX), big);
        assert!(hit_polygon(&[], MIN_HIT_PX).is_empty());
    }

    fn near_any(point: &(f64, f64), poly: &[(f64, f64)]) -> bool {
        poly.iter().any(|p| {
            (p.0 - point.0).hypot(p.1 - point.1) <= MIN_HIT_PX / 2.0 + 1e-6
        })
    }

    #[test]
    fn framing_a_group_aims_at_its_own_extent() {
        // Same viewport, very different formations: a 50 km Gugus frames
        // far out, a 500 m Unsur comes in close. This is the whole point
        // of replacing one shared zoom constant.
        let wide = zoom_for_group_frame(-6.1, 50_000.0, 1920.0, FRAME_VIEWPORT_FRACTION);
        let tight = zoom_for_group_frame(-6.1, 500.0, 1920.0, FRAME_VIEWPORT_FRACTION);
        assert!(wide < tight, "a wider formation frames at a lower zoom");
        // And the result really does fill the share it was asked for.
        let viewport_share = 2.0 * 500.0
            / meters_per_pixel(-6.1, tight);
        assert!((viewport_share / 1920.0 - FRAME_VIEWPORT_FRACTION).abs() < 0.01);
        // Nonsense inputs must not produce a nonsense zoom.
        assert_eq!(zoom_for_group_frame(0.0, 0.0, 1920.0, FRAME_VIEWPORT_FRACTION), 3.0);
        assert!((zoom_for_group_frame(-6.1, 500.0, 1920.0, 0.0) - 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_zone_reports_the_width_its_label_is_measured_against() {
        // A capsule from a segment plus a pad: the polygon is wider than
        // the segment by exactly one pad on each side.
        let pad = 20.0;
        let capsule = zone_polygon(&[(0.0, 0.0), (300.0, 0.0)], pad);
        assert_eq!(zone_width_px(&capsule), 300.0 + 2.0 * pad);
        // A grown square is the square plus a pad on each side.
        let square = zone_polygon(&square_members(), 10.0);
        assert_eq!(zone_width_px(&square), 400.0 + 20.0);
        // Nothing to measure is zero, not a negative infinity a label
        // could then be compared against.
        assert_eq!(zone_width_px(&[]), 0.0);
    }

    #[test]
    fn only_an_unassigned_in_exercise_hull_is_planned() {
        let unassigned: std::collections::HashSet<i64> = [7, 9].into_iter().collect();
        // In the exercise but not in the tree: planned, so dashed.
        assert!(unit_is_planned(Some(7), &unassigned));
        assert!(unit_is_planned(Some(9), &unassigned));
        // In the tree: present, so solid.
        assert!(!unit_is_planned(Some(8), &unassigned));
        // Not in the exercise at all, or no mirrored id: nothing to say,
        // so solid — a hull the client cannot identify is never dashed.
        assert!(!unit_is_planned(None, &unassigned));
        assert!(!unit_is_planned(Some(1), &std::collections::HashSet::new()));
    }

    #[test]
    fn a_unit_name_paints_at_near_or_under_focus() {
        assert!(should_paint_unit_label(UnitLod::Near, false));
        // Middle and Far lose the name — this is the whole rule — but a
        // focused one keeps it, because selection is never invisible.
        assert!(!should_paint_unit_label(UnitLod::Middle, false));
        assert!(!should_paint_unit_label(UnitLod::Far, false));
        assert!(should_paint_unit_label(UnitLod::Middle, true));
        assert!(should_paint_unit_label(UnitLod::Far, true));
    }

    #[test]
    fn group_text_paints_when_it_fits_its_own_ground() {
        // The surge-fleet-sized group at zoom 9: 2 x its 92 px cover
        // radius, comfortably wider than `name (4)`.
        assert!(should_paint_group_text(184.0, 85.0, false));
        // A 30 px group cannot carry a name of that width...
        assert!(!should_paint_group_text(60.0, 85.0, false));
        // ...but focus wins, uncapped, whatever the extent.
        assert!(should_paint_group_text(0.0, 400.0, true));
        // Exactly equal fits: the rule is `<=`, not `<`.
        assert!(should_paint_group_text(85.0, 85.0, false));
    }

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
        let by_cos = equator * (-6.108f64).to_radians().cos();
        assert!((jawa - by_cos).abs() < 1e-9, "{jawa} vs {by_cos}");
        let by_cos_2 = equator * 57.0f64.to_radians().cos();
        assert!((scotland - by_cos_2).abs() < 1e-9, "{scotland} vs {by_cos_2}");
        // The equator value is the 512-base Web Mercator resolution.
        let expected = EARTH_EQUATOR_M / (WORLD_BASE_PX * 2f64.powf(z));
        assert!((equator - expected).abs() < 1e-6);
        // 512 base, not 256: the equator resolution is half the
        // classic 256-tile figure, exactly as WORLD_BASE_PX's
        // calibration comment claims.
        assert!(
            (equator - 156_543.033_928_04 / 2f64.powi(z as i32 + 1)).abs() < 1e-6,
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
        let expected = 1.0 / 57.0f64.to_radians().cos();
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

    /// The floating side zone's camera correction, as arithmetic.
    ///
    /// `ShipApp::visible_center` re-centres on the viewport pixel that should
    /// end up in the middle of the *visible* map, which is `w/2 + s` when the
    /// zone covers `s` pixels on the left. The first version passed `s`
    /// itself, which moved the centre left by half the map's width and parked
    /// the framed hull under the chrome — the exact failure the correction
    /// exists to prevent, and one no screenshot would show.
    ///
    /// The property asserted is the SHIFT, not a specific hull's landing
    /// spot: under the corrected centre, the old centre renders `s` pixels
    /// to the left of the new centre's middle, which is the same statement as
    /// "content moved right by `s`, so a centred hull lands on the visible
    /// centre". Asserting a hull's exact pixel would only be true for a hull
    /// that happened to be centred to begin with.
    #[test]
    fn a_left_covered_zone_shifts_the_map_right_by_half_the_zone() {
        let center = (-6.108, 106.910);
        let (w, h, zoom) = (1200.0_f64, 800.0_f64, 11.0_f64);
        let shift = 320.0_f64 / 2.0;

        let goal = unproject_mercator(w / 2.0 + shift, h / 2.0, center, zoom, w, h);
        let (px, py) = project_mercator(center.0, center.1, goal, zoom, w, h);
        assert!(
            (px - (w / 2.0 - shift)).abs() < 0.01,
            "old centre rendered at {px}, wanted {}",
            w / 2.0 - shift
        );
        assert!((py - h / 2.0).abs() < 0.01, "the shift must be horizontal");
    }

    /// A right-docked zone shifts the map the other way. The sign is the
    /// whole reason this is tested rather than eyeballed: a zone on the
    /// right has to move content LEFT, and a sign error here parks the hull
    /// under the chrome on the opposite side, which looks like the zone is
    /// not working at all.
    #[test]
    fn a_right_covered_zone_shifts_the_map_left() {
        let center = (-6.108, 106.910);
        let (w, h, zoom) = (1200.0_f64, 800.0_f64, 11.0_f64);
        let shift = -320.0_f64 / 2.0;

        let goal = unproject_mercator(w / 2.0 + shift, h / 2.0, center, zoom, w, h);
        let (px, _) = project_mercator(center.0, center.1, goal, zoom, w, h);
        assert!(
            (px - (w / 2.0 - shift)).abs() < 0.01,
            "old centre rendered at {px}, wanted {}",
            w / 2.0 - shift
        );
    }
}
