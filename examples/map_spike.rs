//! Headless tile-path check: render the test harbor and save the PNG.
//!
//! No UI, no backend, no ships — just tiles in, PNG out. Uses the shared
//! [`tfg::map_render::render_static_png`] (same code the window shells run).
//!
//! NOTE: needs the libuv workaround on systems with libuv >= 1.51, e.g.
//! `LD_PRELOAD=<repo>/target/libuv-1.44.2/build/libuv.so.1`.

fn main() {
    let t0 = std::time::Instant::now();
    let png = tfg::map_render::render_static_png(
        (53.5413, 9.9842),
        11.0,
        800,
        600,
        "https://tiles.openfreemap.org/styles/liberty",
    );
    std::fs::write("target/map_spike.png", &png).unwrap();
    println!("saved target/map_spike.png ({} bytes in {:.1}s)", png.len(), t0.elapsed().as_secs_f64());
}
