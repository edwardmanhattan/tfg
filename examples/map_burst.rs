//! Real map frames for the GPUI texture path (U6), measured and dumped.
//!
//! `map_spike` proves the headless renderer and saves one PNG. This one
//! drives a real camera gesture — the same shape the map thread sees on a
//! scroll-zoom burst — and writes every frame to disk so `proto/gpui-map`
//! can be driven by real pixels instead of synthetic gradients. It also
//! times what the handoff actually costs on real imagery:
//!
//! - `pump` + `frame_rgba` (the map thread's own work),
//! - PNG encode with the `image` defaults (Fast compression, ADAPTIVE
//!   filter search) and with `FilterType::NoFilter`, which the U6 spike
//!   found 2.2x cheaper on synthetic frames — and which turns out to be
//!   the wrong trade on real imagery (see the numbers this prints), so the
//!   adopted encoding is the default one, and that is what gets written,
//! - the encoded size, which is only cache pressure here: bytes travel
//!   memory-to-decoder and never touch a disk or a wire.
//!
//! Env: `MAP_BURST_STEPS` (gesture steps, default 8),
//! `MAP_BURST_DIR` (output dir, default `target/map_burst`),
//! `MAP_BURST_W`/`MAP_BURST_H` (frame size, default 800x600 — the client's
//! `MAP_W`/`MAP_H`).
//!
//! Run: `cargo run --release --example map_burst`

use std::io::Cursor;

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};

/// One step of the gesture, in degrees. Small enough that consecutive
/// frames differ the way a slow drag does, without leaving the seeded
/// cache's coverage.
const STEP_DEG: f64 = 0.004;
const JUMP_PUMP: u32 = 6;

fn env_num(name: &str, default: f64) -> f64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let steps = env_num("MAP_BURST_STEPS", 8.0) as u32;
    let w = env_num("MAP_BURST_W", 800.0) as u32;
    let h = env_num("MAP_BURST_H", 600.0) as u32;
    let dir = std::env::var("MAP_BURST_DIR").unwrap_or_else(|_| "target/map_burst".into());
    std::fs::create_dir_all(&dir).expect("burst output dir");

    // Same entry points the shells use; only the handoff differs.
    let t0 = std::time::Instant::now();
    let mut map = tfg::map_render::LiveMap::new(
        (-6.108, 106.910),
        11.0,
        w,
        h,
        "https://tiles.openfreemap.org/styles/liberty",
        tfg::map_render::prepare_runtime_cache(
            &tfg::paths::AppPaths::discover().unwrap().map_cache,
        )
        .expect("map cache"),
    );
    println!("scene ready in {:.1}s", t0.elapsed().as_secs_f64());

    let (frame_w, frame_h) = map.dims();
    let mut default_ms: Vec<f64> = Vec::new();
    let mut nofilter_ms: Vec<f64> = Vec::new();
    let mut render_ms: Vec<f64> = Vec::new();

    for step in 0..steps {
        // The gesture: a slow north-east drag, camera updated once per
        // step and pumped the way `JUMP_PUMP` does it.
        let lng = 106.910 + STEP_DEG * step as f64;
        let lat = -6.108 + STEP_DEG * 0.5 * step as f64;
        let t_render = std::time::Instant::now();
        map.set_center((lat, lng), 11.0);
        map.pump(JUMP_PUMP);
        let rgba = map.frame_rgba();
        let render = t_render.elapsed().as_secs_f64() * 1000.0;
        render_ms.push(render);

        let mut encode = |filter: Option<FilterType>| -> (f64, usize) {
            let mut png = Vec::new();
            let t = std::time::Instant::now();
            let encoder = match filter {
                Some(f) => PngEncoder::new_with_quality(
                    Cursor::new(&mut png),
                    CompressionType::Fast,
                    f,
                ),
                None => PngEncoder::new(Cursor::new(&mut png)),
            };
            encoder
                .write_image(&rgba, frame_w, frame_h, ExtendedColorType::Rgba8)
                .expect("png encode");
            (t.elapsed().as_secs_f64() * 1000.0, png.len())
        };

        let (d_ms, d_bytes) = encode(None);
        let (nf_ms, nf_bytes) = encode(Some(FilterType::NoFilter));

        // The frames the proto consumes: the DEFAULT encoding, which is
        // what the U6 spike adopted after these numbers — on real imagery
        // the filter search costs 0.8ms and saves 4.5x the bytes, the
        // reverse of the synthetic case that motivated NoFilter.
        let mut png = Vec::new();
        PngEncoder::new(Cursor::new(&mut png))
            .write_image(&rgba, frame_w, frame_h, ExtendedColorType::Rgba8)
            .expect("png encode");

        default_ms.push(d_ms);
        nofilter_ms.push(nf_ms);
        let path = format!("{dir}/frame_{step:03}.png");
        std::fs::write(&path, &png).expect("write frame");
        println!(
            "frame {step}: render {render:.1}ms  encode default {d_ms:.1}ms/{d_bytes}B  \
             nofilter {nf_ms:.1}ms/{nf_bytes}B  -> {path}"
        );
    }

    let mut summarize = |name: &str, mut v: Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = v[v.len() / 2];
        let p95 = v[(v.len() as f64 * 0.95) as usize];
        println!(
            "{name}: min {:.1} p50 {:.1} p95 {:.1} max {:.1} ms",
            v[0],
            p50,
            p95,
            v[v.len() - 1]
        );
    };
    summarize("render+pump", render_ms);
    summarize("encode default(adaptive)", default_ms);
    summarize("encode nofilter", nofilter_ms);
    println!(
        "{} frames of {}x{} in {dir} (total {:.1}s)",
        steps,
        frame_w,
        frame_h,
        t0.elapsed().as_secs_f64()
    );
}
