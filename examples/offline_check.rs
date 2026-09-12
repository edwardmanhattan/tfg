//! Offline proof: render the theater with the network denied.
//!
//! Registers a file source that fails every *network* request, so the frame
//! can only come from the repo-committed ambient cache. A full map here
//! proves the app boots offline. Saves `target/offline-check.png`.
//!
//! `cargo run --example offline_check`

use image::{ImageBuffer, Rgba};
use maplibre_native::{
    FileSourceType, RequestHandle, Responder, ResourceRequest,
    file_source::{ErrorReason, FileSource, Response, register_file_source},
};
use tfg::map_render::{LiveMap, repo_cache_path};

/// Fails everything the cache can't serve: no packets leave the host.
struct DenyNetwork;

impl FileSource for DenyNetwork {
    fn can_request(&self, _request: &ResourceRequest) -> bool {
        true
    }

    fn request(&self, request: ResourceRequest, responder: Responder) -> RequestHandle {
        eprintln!("denied network: {}", request.url);
        responder.complete(denied());
        RequestHandle::Done
    }
}

fn denied() -> Response {
    Response::error(ErrorReason::Connection, "offline proof: network denied")
}

fn main() {
    // Must register before the scene exists: this replaces the network path.
    register_file_source(FileSourceType::Network, DenyNetwork);
    let t0 = std::time::Instant::now();
    let mut scene = LiveMap::new(
        (-6.108, 106.910),
        11.0,
        800,
        600,
        "https://tiles.openfreemap.org/styles/liberty",
        repo_cache_path(),
    );
    scene.pump(10);
    let (w, h) = scene.dims();
    let raw = scene.frame_rgba();
    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(w, h, raw).expect("frame size matches dims");
    img.save("target/offline-check.png").unwrap();
    println!("saved target/offline-check.png in {:.1}s", t0.elapsed().as_secs_f64());
}
