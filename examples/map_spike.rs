//! Headless tile-path check: build the persistent scene, save one PNG.
//!
//! No UI, no backend, no ships — proves the continuous renderer path that
//! all shells run. Uses [`tfg::map_render::LiveMap`].

use image::{ImageBuffer, Rgba};

fn main() {
    let t0 = std::time::Instant::now();
    let mut map = tfg::map_render::LiveMap::new(
        (-6.108, 106.910),
        11.0,
        800,
        600,
        "https://tiles.openfreemap.org/styles/liberty",
        tfg::map_render::prepare_runtime_cache(&tfg::paths::AppPaths::discover().unwrap().map_cache)
            .expect("map cache"),
    );
    println!("scene ready in {:.1}s", t0.elapsed().as_secs_f64());
    map.pump(10);
    let (w, h) = map.dims();
    let raw = map.frame_rgba();
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(w, h, raw).expect("frame size matches dims");
    img.save("target/map_spike.png").unwrap();
    println!(
        "saved target/map_spike.png in {:.1}s total",
        t0.elapsed().as_secs_f64()
    );
}
