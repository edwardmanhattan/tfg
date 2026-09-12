//! Seed the repo-committed ambient tile cache (`assets/tiles-cache.sqlite`).
//!
//! Renders a grid over the mock theater (Jakarta Bay) at zooms 10-12 so the
//! app boots with full tiles and no network. Run with network when the
//! theater moves or tile versions roll:
//!
//! `MLN_PRECOMPILE=1 LD_PRELOAD=<libuv> cargo run --example seed_cache`
//!
//! NOTE: same libuv workaround as everything maplibre-shaped.

use tfg::map_render::{LiveMap, repo_cache_path};

const STYLE: &str = "https://tiles.openfreemap.org/styles/liberty";

fn main() {
    let t0 = std::time::Instant::now();
    // MapLibre silently skips caching when the parent dir is missing.
    std::fs::create_dir_all(
        repo_cache_path().parent().expect("cache path has a parent"),
    )
    .expect("cache dir writable");
    // Theater corners + center, each at three zooms.
    let spots = [
        (-6.0950, 106.8500),
        (-6.1080, 106.9100),
        (-6.1200, 106.9900),
        (-6.1400, 106.8200),
    ];
    for zoom in [10.0, 11.0, 12.0] {
        for (i, at) in spots.iter().enumerate() {
            let mut scene = LiveMap::new(*at, zoom, 800, 600, STYLE, repo_cache_path());
            scene.pump(8);
            let _ = scene.frame_rgba();
            println!("seeded z{zoom} spot {i} at {:.1}s", t0.elapsed().as_secs_f64());
        }
    }
    let size = std::fs::metadata(repo_cache_path()).map(|m| m.len()).unwrap_or(0);
    println!("cache: {} ({} bytes, {:.1}s)", repo_cache_path().display(), size, t0.elapsed().as_secs_f64());
}
