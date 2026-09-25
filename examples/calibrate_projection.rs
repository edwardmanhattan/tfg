//! Projection calibrator (task #42): measures the renderer's true
//! pixels-per-degree against the hand-rolled WebMercator overlay math.
//!
//! Method: render two frames at the same zoom with centers panned by a
//! known delta. A fixed geographic edge (the Jakarta Bay coastline)
//! shifts by a measurable pixel count; the shift is independent of the
//! edge's own position, so `measured / predicted` reads out the scale
//! mismatch directly: ~1.0 = overlay matches, ~2.0 = 512-vs-256 base,
//! anything else = a deeper camera disagreement.
//!
//! Run: `cargo run --example calibrate_projection`
//! Paste the whole output back. Needs the seeded tile cache (default
//! path); network only if tiles miss.

use tfg::map_render::{LiveMap, prepare_runtime_cache, project_mercator};
use tfg::paths::AppPaths;

const STYLE: &str = "https://tiles.openfreemap.org/styles/liberty";
const W: u32 = 800;
const H: u32 = 600;
const ZOOM: f64 = 11.0;

fn luma(r: u8, g: u8, b: u8) -> f32 {
    0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32
}

/// Strongest vertical edge on the middle column (N-S scan for the E-W
/// coast). Returns (y, strength); strength under ~20 means no edge found
/// (open water or solid land).
fn coast_y(frame: &[u8]) -> (i32, f32) {
    let col = (W / 2) as usize;
    let at = |y: usize| {
        let i = (y * W as usize + col) * 4;
        luma(frame[i], frame[i + 1], frame[i + 2])
    };
    let mut best = (0i32, 0f32);
    for y in 20..(H as usize - 20) {
        let g = (at(y + 1) - at(y - 1)).abs();
        if g > best.1 {
            best = (y as i32, g);
        }
    }
    best
}

/// Strongest horizontal edge on the middle row (E-W scan for N-S edges).
fn edge_x(frame: &[u8]) -> (i32, f32) {
    let row = (H / 2) as usize;
    let at = |x: usize| {
        let i = (row * W as usize + x) * 4;
        luma(frame[i], frame[i + 1], frame[i + 2])
    };
    let mut best = (0i32, 0f32);
    for x in 20..(W as usize - 20) {
        let g = (at(x + 1) - at(x - 1)).abs();
        if g > best.1 {
            best = (x as i32, g);
        }
    }
    best
}

fn render(center: (f64, f64), cache_path: &std::path::Path) -> Vec<u8> {
    let mut scene = LiveMap::new(center, ZOOM, W, H, STYLE, cache_path.to_path_buf());
    scene.pump(12);
    scene.frame_rgba()
}

/// Predicted pixel shift of a fixed feature when the center moves from
/// `a` to `b`, under the overlay math. Independent of the feature.
fn predicted_shift(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let (_, ya) = project_mercator(a.0, a.1, a, ZOOM, W as f64, H as f64);
    let (_, yb) = project_mercator(a.0, a.1, b, ZOOM, W as f64, H as f64);
    let (xa, _) = project_mercator(a.0, a.1, a, ZOOM, W as f64, H as f64);
    let (xb, _) = project_mercator(a.0, a.1, b, ZOOM, W as f64, H as f64);
    (xb - xa, yb - ya)
}

fn main() {
    // Latitude pan across the bay coast: middle column should cross it.
    let south = (-6.1300, 106.8700);
    let north = (-6.1100, 106.8700);
    let paths = AppPaths::discover().expect("runtime paths");
    let cache = prepare_runtime_cache(&paths.map_cache).expect("map cache");
    let fa = render(south, &cache);
    let fb = render(north, &cache);
    let (ya, sa) = coast_y(&fa);
    let (yb, sb) = coast_y(&fb);
    let measured_lat = (yb - ya) as f64;
    let (_, pred_lat) = predicted_shift(south, north);
    println!("--- latitude pan (+0.02 deg, zoom {ZOOM}) ---");
    println!("coast y: south frame {ya} (strength {sa:.0}), north frame {yb} (strength {sb:.0})");
    println!("measured shift: {measured_lat:.1}px, overlay predicts: {pred_lat:.1}px");
    println!("ratio measured/predicted: {:.3}", measured_lat / pred_lat);

    // Longitude pan: middle row hunts a north-south edge (port, river).
    let west = (-6.1080, 106.8300);
    let east = (-6.1080, 106.8700);
    let fw = render(west);
    let fe = render(east);
    let (xa, sxa) = edge_x(&fw);
    let (xb, sxb) = edge_x(&fe);
    let measured_lon = (xb - xa) as f64;
    let (pred_lon, _) = predicted_shift(west, east);
    println!("--- longitude pan (+0.04 deg, zoom {ZOOM}) ---");
    println!("edge x: west frame {xa} (strength {sxa:.0}), east frame {xb} (strength {sxb:.0})");
    println!("measured shift: {measured_lon:.1}px, overlay predicts: {pred_lon:.1}px");
    if pred_lon.abs() > 1.0 {
        println!("ratio measured/predicted: {:.3}", measured_lon / pred_lon);
    }
    println!("--- verdict guide: ratio ~1 = match, ~2 = 512-vs-256 base, else deeper mismatch ---");
}
