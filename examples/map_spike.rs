//! Spike step 1: prove the maplibre-native tile path works headless.
//!
//! Renders one static frame of a test harbor (Hamburg, zoom 11) from the
//! public demotiles style and saves it to `target/map_spike.png`.
//! No GPUI, no backend, no ships — just tiles in, PNG out.

use std::num::NonZeroU32;
use std::time::Duration;

use maplibre_native::{CameraUpdate, ImageRendererBuilder, LatLng};

fn main() {
    let mut renderer = ImageRendererBuilder::new()
        .with_size(
            NonZeroU32::new(800).unwrap(),
            NonZeroU32::new(600).unwrap(),
        )
        .build_static_renderer();

    renderer.load_style_from_url(
        &"https://demotiles.maplibre.org/style.json"
            .parse()
            .unwrap(),
    );

    // Static renderer needs a pump window for async resources to arrive.
    // Poll until a frame carries pixels or we time out.
    let camera = CameraUpdate::new()
        .center(LatLng {
            lat: 53.5413,
            lng: 9.9842,
        })
        .zoom(11.0);
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut attempts = 0;
    loop {
        attempts += 1;
        match renderer.render_static(&camera) {
            Ok(image) => {
                let buf = image.as_image();
                println!(
                    "rendered {}x{} after {attempts} attempt(s)",
                    buf.width(),
                    buf.height()
                );
                buf.save("target/map_spike.png").unwrap();
                println!("saved target/map_spike.png");
                return;
            }
            Err(e) => {
                if std::time::Instant::now() > deadline {
                    eprintln!("still failing after {attempts} attempts: {e:?}");
                    std::process::exit(1);
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }
}
