//! U6 spike: the map texture decision, measured rather than theorized.
//!
//! Question: how do 800x600 raw RGBA map frames reach a GPUI surface at
//! interaction cadence? Surveyed against pinned gpui 0.2.2 source:
//!
//! - `ImageSource::Image(Arc<Image>)`: `Image` holds ENCODED bytes
//!   (PNG/JPEG/…) decoded async off the UI thread. Old asset auto-removed
//!   on replace (`img.rs`: load/get vs remove on source change), so memory
//!   stays bounded by construction. Needs a PNG encode per frame.
//! - `ImageSource::Render(Arc<RenderImage>)`: `RenderImage::new` needs a
//!   `Frame` that is `pub(crate)` — external crates cannot construct one.
//!   Dead path for raw frames.
//! - `Surface`: macOS-only (CoreVideo). Dead on Linux.
//! - Zero-copy external texture: absent, as ADR-0002 already observed.
//!
//! So the stock path is PNG-encode → `Image` → async decode → upload,
//! with ~1 frame of lag against egui's synchronous `TextureHandle::set`.
//! Map traffic is BURSTY (newest-wins drain on camera moves, `JUMP_PUMP`
//! frames per gesture), not a continuous 50Hz stream — the analysis's
//! churn fear assumes a cadence the pump never produces.
//!
//! This spike times the round trip with synthetic frames (gradient +
//! counter bar, 800x600 @10Hz, newest-wins drain like the map thread) and
//! renders them full-bleed. Verdict + numbers go to the handoff; a custom
//! upload element stays the perf follow-up IF the numbers demand it.

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
    /// Latest decoded-ready frame, swapped in by the drain task.
    frame: Option<Arc<Image>>,
    frame_no: u64,
    encode_ms: f64,
}

impl Render for MapDemo {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mut root = div().relative().size_full().bg(gpui::black());
        if let Some(frame) = self.frame.clone() {
            root = root.child(div().absolute().size_full().child(img(frame)));
        }
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
                .child(format!("frame {} enc {:.1}ms", self.frame_no, self.encode_ms)),
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

fn main() {
    // Producer thread: the map thread's shape (newest-wins, convert off
    // the UI thread). PNG encode timed per frame into the log.
    let (png_tx, png_rx) = mpsc::channel::<(u64, Vec<u8>, f64)>();
    std::thread::spawn(move || {
        let mut n: u64 = 0;
        // STATIC=1: emit one frame and hold it — isolates asset-decode
        // churn from paint. If a held frame paints, churn is the killer;
        // if not, the Image path itself is broken on this backend.
        let once = std::env::var("STATIC").as_deref() == Ok("1");
        loop {
            n += 1;
            let rgba = render_frame(n);
            let t0 = Instant::now();
            let mut png = Vec::new();
            {
                use image::ImageEncoder as _;
                image::codecs::png::PngEncoder::new(Cursor::new(&mut png))
                    .write_image(
                        &rgba,
                        FRAME_W,
                        FRAME_H,
                        image::ExtendedColorType::Rgba8,
                    )
                    .expect("png encode");
            }
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            // Newest-wins lives on the consumer (drain-all, keep latest);
            // the producer just stamps each frame. Unbounded channel is
            // fine: the drain ticks twice per frame period.
            if png_tx.send((n, png, ms)).is_err() {
                return; // UI gone
            }
            eprintln!("enc frame={n} {ms:.1}ms");
            if once {
                // Hold the single frame; keep the thread alive so the
                // channel stays open, but never send again.
                std::thread::sleep(Duration::from_secs(3600));
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });

    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(860.0), px(680.0)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let entity = cx.new(|_| MapDemo { frame: None, frame_no: 0, encode_ms: 0.0 });
        let drain = entity.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            let mut prev_no = 0u64;
            loop {
                cx.background_executor().timer(Duration::from_millis(50)).await;
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
                            drain.update(app, |root: &mut MapDemo, cx| {
                                root.frame = Some(arc);
                                root.frame_no = n;
                                root.encode_ms = ms;
                                cx.notify();
                            })
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
