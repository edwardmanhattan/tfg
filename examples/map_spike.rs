//! Spike step 1: prove the maplibre-native tile path works headless.
//!
//! Renders one static frame of a test harbor (Hamburg, zoom 11) from the
//! public demotiles style and saves it to `target/map_spike.png`.
//! No GPUI, no backend, no ships — just tiles in, PNG out.
//!
//! NOTE: on systems with libuv >= 1.51 (e.g. current Arch), the precompiled
//! core's bundled libuv collides with the system one and the runloop aborts
//! (`io_uring_enter ... EBADF`). Workaround for the spike: preload an older
//! libuv, e.g.
//! `LD_PRELOAD=/tmp/libuv-1.44.2/build/libuv.so.1 cargo run --example map_spike`
//! The real fix (from-source core build) is a follow-up, not spike scope.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use maplibre_native::{CameraUpdate, ImageRendererBuilder, LatLng};

fn main() {
    let mut renderer = ImageRendererBuilder::new()
        .with_size(
            NonZeroU32::new(800).unwrap(),
            NonZeroU32::new(600).unwrap(),
        )
        .build_static_renderer();

    let style_loaded = Arc::new(AtomicBool::new(false));
    let idle = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let observer = renderer.map_observer();
    observer.set_did_finish_loading_style_callback({
        let style_loaded = style_loaded.clone();
        move || style_loaded.store(true, Ordering::SeqCst)
    });
    observer.set_did_become_idle_callback({
        let idle = idle.clone();
        move || idle.store(true, Ordering::SeqCst)
    });
    observer.set_did_fail_loading_map_callback({
        let failed = failed.clone();
        move |e| {
            eprintln!("map failed to load: {}", e.message);
            failed.store(true, Ordering::SeqCst);
        }
    });

    renderer.load_style_from_url(
        &"https://tiles.openfreemap.org/styles/liberty"
            .parse()
            .unwrap(),
    );

    // Static renderer needs its runloop pumped for async resources to arrive.
    // Pump a fixed number of frames and save the LAST one (idle callbacks
    // don't fire for Static). If tiles are flowing, the last frame shows
    // the harbor.
    let camera = CameraUpdate::new()
        .center(LatLng {
            lat: 53.5413,
            lng: 9.9842,
        })
        .zoom(11.0);
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    let mut last = None;
    for frame in 1..=40 {
        if failed.load(Ordering::SeqCst) {
            eprintln!("style/tiles failed to load, aborting");
            std::process::exit(1);
        }
        match renderer.render_static(&camera) {
            Ok(image) => {
                println!(
                    "frame {frame}: {}x{}",
                    image.as_image().width(),
                    image.as_image().height()
                );
                last = Some(image);
            }
            Err(e) => eprintln!("frame {frame} failed (still loading?): {e:?}"),
        }
        if std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    match last {
        Some(image) => {
            image.as_image().save("target/map_spike.png").unwrap();
            println!("saved target/map_spike.png");
        }
        None => {
            eprintln!("no frame rendered at all, aborting");
            std::process::exit(1);
        }
    }
}
