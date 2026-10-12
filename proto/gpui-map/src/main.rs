//! U6 spike: the map texture decision, measured rather than theorized.
//!
//! Question: how do 800x600 raw RGBA map frames reach a GPUI surface at
//! interaction cadence? Surveyed against pinned gpui 0.2.2 source:
//!
//! - `ImageSource::Image(Arc<Image>)`: `Image` holds ENCODED bytes
//!   (PNG/JPEG/…) decoded async off the UI thread. Needs a PNG encode per
//!   frame. The asset map does NOT evict on replace and the sprite atlas
//!   only frees via `drop_image`, so BOTH sides are the caller's job.
//! - `ImageSource::Render(Arc<RenderImage>)`: `RenderImage::new` needs a
//!   `Frame` that is `pub(crate)` — external crates cannot construct one.
//!   Dead path for raw frames.
//! - `Surface`: macOS-only (CoreVideo). Dead on Linux.
//! - Zero-copy external texture: absent, as ADR-0002 already observed.
//!
//! So the stock path is PNG-encode → `Image` → async decode → upload.
//! Two things make it correct, both found by measurement:
//!
//! 1. **Hold-until-ready.** `use_asset` returns `None` until the decode
//!    lands, and every new `Arc<Image>` (id = hash of bytes) restarts the
//!    load — swapping the display immediately blacks the surface out
//!    permanently under churn. The WINDOWED frame is held until its
//!    successor resolves, which costs ~1 frame of lag against egui's
//!    synchronous `TextureHandle::set` and never paints nothing.
//! 2. **Explicit retire.** Promotions `drop_image` the old tile and
//!    `remove_asset` the old entry; superseded *pending* frames are
//!    removed too. Without it the atlas and the asset map grow per frame.
//!
//! Frames come from `MAP_FRAMES_DIR` (default `/tmp/opencode/mapframes`,
//! written by `examples/map_burst` from the real map engine) and fall back
//! to a synthetic gradient when no PNGs are present. `LOOP=1` recycles the
//! burst forever for the memory test; plain run replays it once and holds
//! the last frame so a capture is stable. Verdict + numbers live in
//! `docs/gpui-migration-handoff.md`; a custom upload element stays the
//! follow-up only if those numbers demand it — they do not.

use std::io::Cursor;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use gpui::{
    App, AppContext, Application, AsyncApp, Bounds, Context, Image, ImageFormat, IntoElement,
    ParentElement, Render, Styled, Window, WindowBounds, WindowOptions, div, img, px, size,
};
use image::{ImageBuffer, Rgba, RgbaImage};

const FRAME_W: u32 = 800;
const FRAME_H: u32 = 600;

struct MapDemo {
    /// Displayed frame: swapped ONLY after its successor decodes
    /// (hold-until-ready). Never black after first paint.
    frame: Option<Arc<Image>>,
    frame_no: u64,
    encode_ms: f64,
    /// Newest-wins successor; promoted on decode-ready, never displayed
    /// raw (displaying it raw is the churn-black bug). The instant is
    /// when the bytes were queued, so promotion time minus it is the
    /// real decode lag the user would see as response latency.
    pending: Option<(u64, Arc<Image>, f64, Instant)>,
    promoted: u64,
    retired_tiles: u64,
    last_lag_ms: f64,
}

impl Render for MapDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Hold-until-ready gate. `use_` starts the successor's async
        // decode (if not running) and notifies this view on completion;
        // until it returns Some the STABLE frame stays up. Promotion then
        // retires the old frame on both axes the stock path leaks:
        // GPU atlas tile (`drop_image`) and CPU asset entry
        // (`remove_asset` — `fetch_asset` never evicts).
        if let Some((n, arc, ms, queued)) = self.pending.clone() {
            if arc.clone().use_render_image(window, cx).is_some() {
                let lag = queued.elapsed().as_secs_f64() * 1000.0;
                if let Some(old) = self.frame.take() {
                    if let Some(old_ri) = old.clone().get_render_image(window, cx) {
                        cx.drop_image(old_ri, Some(window));
                        self.retired_tiles += 1;
                        eprintln!("retire frame={} -> tile dropped", self.frame_no);
                    }
                    old.remove_asset(cx);
                }
                eprintln!("promote frame={n} lag={lag:.1}ms");
                self.frame = Some(arc);
                self.frame_no = n;
                self.encode_ms = ms;
                self.last_lag_ms = lag;
                self.pending = None;
                self.promoted += 1;
            }
        }
        let mut root = div().relative().size_full().bg(gpui::black());
        if let Some(frame) = self.frame.clone() {
            root = root.child(div().absolute().size_full().child(img(frame)));
        }
        let pend = self.pending.as_ref().map(|(n, _, _, _)| n.to_string()).unwrap_or_else(|| "-".into());
        root.child(
            div()
                .absolute()
                .top(px(8.0))
                .left(px(8.0))
                .px(px(10.0))
                .py(px(4.0))
                .bg(gpui::black())
                .font_family("monospace")
                .text_size(px(13.0))
                .text_color(gpui::white())
                .child(format!(
                    "frame {} enc {:.1}ms lag {:.0}ms pend {} up {} ret {}",
                    self.frame_no, self.encode_ms, self.last_lag_ms, pend, self.promoted, self.retired_tiles
                )),
        )
    }
}

/// Synthetic map frame: horizontal gradient, vertical bands, and a
/// counter bar across the top whose width encodes the frame number —
/// movement is visible frame to frame, like tiles pumping.
fn render_frame(n: u64) -> RgbaImage {
    let mut buf: RgbaImage = ImageBuffer::new(FRAME_W, FRAME_H);
    for y in 0..FRAME_H {
        for x in 0..FRAME_W {
            let r = ((x * 255 / FRAME_W) as u8).wrapping_add((n * 7) as u8);
            let g = ((y * 255 / FRAME_H) as u8).wrapping_add((n * 13) as u8);
            let b = (128 + ((x + y + n as u32 * 3) % 128)) as u8;
            buf.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }
    let bar_w = ((n * 37) % FRAME_W as u64) as u32;
    for y in 0..24 {
        for x in 0..bar_w {
            buf.put_pixel(x, y, Rgba([34, 211, 238, 255]));
        }
    }
    buf
}

/// Where frames come from. `Real` replays a burst captured from the map
/// engine by `examples/map_burst` (`MAP_FRAMES_DIR`, default
/// `/tmp/opencode/mapframes`) — real Jakarta tiles from the same
/// `LiveMap::frame_rgba` the shipped map thread sends, so the gate,
/// decode and atlas behaviour are measured against the real thing rather
/// than a gradient. `Synthetic` stays as the fallback that needs no data.
enum Source {
    Real(Vec<Vec<u8>>),
    Synthetic,
}

fn load_source() -> Source {
    let dir = std::env::var("MAP_FRAMES_DIR")
        .unwrap_or_else(|_| "/tmp/opencode/mapframes".to_string());
    let mut paths: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "png"))
            .collect(),
        Err(_) => Vec::new(),
    };
    paths.sort();
    let frames: Vec<Vec<u8>> = paths
        .iter()
        .filter_map(|p| std::fs::read(p).ok())
        .collect();
    if frames.is_empty() {
        eprintln!("source: synthetic (no PNGs in {dir})");
        Source::Synthetic
    } else {
        eprintln!("source: real, {} frames from {dir}", frames.len());
        Source::Real(frames)
    }
}

fn main() {
    // Producer thread: the map thread's shape (newest-wins, convert off
    // the UI thread). PNG encode timed per frame into the log — zero for
    // real frames, whose encode was measured by `map_burst` offline.
    let source = load_source();
    let (png_tx, png_rx) = mpsc::channel::<(u64, Vec<u8>, f64)>();
    std::thread::spawn(move || {
        // BURST_MS: gap between real frames — the gesture cadence. The
        // engine's own pump is slower than this on a cold cache (see the
        // map_burst numbers), so this is the unfavourable case for the
        // gate: frames arriving faster than the UI can settle.
        let burst_ms: u64 = std::env::var("BURST_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(80);
        let hold = std::env::var("LOOP").as_deref() != Ok("1");
        let mut n: u64 = 0;
        // STATIC=1: emit one frame and hold it — isolates asset-decode
        // churn from paint. If a held frame paints, churn is the killer;
        // if not, the Image path itself is broken on this backend.
        let once = std::env::var("STATIC").as_deref() == Ok("1");
        let real = match source {
            Source::Real(frames) => frames,
            Source::Synthetic => Vec::new(),
        };
        loop {
            let (png, ms) = if !real.is_empty() {
                let i = (n as usize) % real.len();
                (real[i].clone(), 0.0)
            } else {
                let rgba = render_frame(n + 1);
                let t0 = Instant::now();
                let mut png = Vec::new();
                {
                    use image::ImageEncoder as _;
                    image::codecs::png::PngEncoder::new(Cursor::new(&mut png))
                        .write_image(&rgba, FRAME_W, FRAME_H, image::ExtendedColorType::Rgba8)
                        .expect("png encode");
                }
                (png, t0.elapsed().as_secs_f64() * 1000.0)
            };
            n += 1;
            // Newest-wins lives on the consumer (drain-all, keep latest);
            // the producer just stamps each frame. Unbounded channel is
            // fine: the drain ticks twice per frame period.
            let bytes = png.len();
            if png_tx.send((n, png, ms)).is_err() {
                return; // UI gone
            }
            eprintln!("enc frame={n} {ms:.1}ms bytes={bytes}");
            if once {
                std::thread::sleep(Duration::from_secs(3600));
                return;
            }
            if hold && n as usize >= real.len().max(1) {
                // Real burst exhausted: hold the last frame forever so a
                // capture shows a settled picture (LOOP=1 keeps churning
                // for the memory test).
                std::thread::sleep(Duration::from_secs(3600));
                return;
            }
            std::thread::sleep(Duration::from_millis(burst_ms));
        }
    });

    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(860.0), px(680.0)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let entity = cx.new(|_| MapDemo {
            frame: None,
            frame_no: 0,
            encode_ms: 0.0,
            pending: None,
            promoted: 0,
            retired_tiles: 0,
            last_lag_ms: 0.0,
        });
        let drain = entity.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let mut prev_no = 0u64;
            loop {
                cx.background_executor().timer(Duration::from_millis(20)).await;
                let mut latest: Option<(u64, Vec<u8>, f64)> = None;
                for m in png_rx.try_iter() {
                    latest = Some(m);
                }
                if let Some((n, png, ms)) = latest {
                    if n != prev_no {
                        prev_no = n;
                        let t0 = Instant::now();
                        let arc = Arc::new(Image::from_bytes(ImageFormat::Png, png));
                        let queue_ms = t0.elapsed().as_secs_f64() * 1000.0;
                        eprintln!("ui frame={n} queue={queue_ms:.1}ms");
                        let _ = cx.update(|app| {
                            let retired = drain.update(app, |root: &mut MapDemo, cx| {
                                let r = root.pending.take().map(|(_, a, _, _)| a);
                                root.pending = Some((n, arc, ms, Instant::now()));
                                cx.notify();
                                r
                            });
                            if let Some(a) = retired {
                                a.remove_asset(app);
                            }
                        });
                    }
                }
            }
        })
        .detach();
        cx.open_window(opts, |_window, _cx| entity.clone()).unwrap();
        cx.activate(true);
    });
}
