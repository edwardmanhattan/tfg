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
    /// Displayed frame: swapped ONLY after its successor decodes
    /// (hold-until-ready). Never black after first paint.
    frame: Option<Arc<Image>>,
    frame_no: u64,
    encode_ms: f64,
    /// Newest-wins successor; promoted on decode-ready, never displayed
    /// raw (displaying it raw is the churn-black bug).
    pending: Option<(u64, Arc<Image>, f64)>,
    promoted: u64,
    retired_tiles: u64,
}

impl Render for MapDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Hold-until-ready gate. `use_` starts the successor's async
        // decode (if not running) and notifies this view on completion;
        // until it returns Some the STABLE frame stays up. Promotion then
        // retires the old frame on both axes the stock path leaks:
        // GPU atlas tile (`drop_image`) and CPU asset entry
        // (`remove_asset` — `fetch_asset` never evicts).
        if let Some((n, arc, ms)) = self.pending.clone() {
            if arc.clone().use_render_image(window, cx).is_some() {
                if let Some(old) = self.frame.take() {
                    if let Some(old_ri) = old.clone().get_render_image(window, cx) {
                        cx.drop_image(old_ri, Some(window));
                        self.retired_tiles += 1;
                        eprintln!("retire frame={} -> tile dropped", self.frame_no);
                    }
                    old.remove_asset(cx);
                }
                eprintln!("promote frame={n}");
                self.frame = Some(arc);
                self.frame_no = n;
                self.encode_ms = ms;
                self.pending = None;
                self.promoted += 1;
            }
        }
        let mut root = div().relative().size_full().bg(gpui::black());
        if let Some(frame) = self.frame.clone() {
            root = root.child(div().absolute().size_full().child(img(frame)));
        }
        let pend = self.pending.as_ref().map(|(n, _, _)| n.to_string()).unwrap_or_else(|| "-".into());
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
                    "frame {} enc {:.1}ms pend {} up {} ret {}",
                    self.frame_no, self.encode_ms, pend, self.promoted, self.retired_tiles
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
                // Default encoder = Fast compression + ADAPTIVE filtering
                // (5 filters tried per row). FASTFILTER=1 pins NoFilter to
                // show how much of the encode is filter search.
                if std::env::var("FASTFILTER").as_deref() == Ok("1") {
                    image::codecs::png::PngEncoder::new_with_quality(
                        Cursor::new(&mut png),
                        image::codecs::png::CompressionType::Fast,
                        image::codecs::png::FilterType::NoFilter,
                    )
                    .write_image(
                        &rgba,
                        FRAME_W,
                        FRAME_H,
                        image::ExtendedColorType::Rgba8,
                    )
                    .expect("png encode");
                } else {
                    image::codecs::png::PngEncoder::new(Cursor::new(&mut png))
                        .write_image(
                            &rgba,
                            FRAME_W,
                            FRAME_H,
                            image::ExtendedColorType::Rgba8,
                        )
                        .expect("png encode");
                }
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
        let entity = cx.new(|_| MapDemo {
            frame: None,
            frame_no: 0,
            encode_ms: 0.0,
            pending: None,
            promoted: 0,
            retired_tiles: 0,
        });
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
                            let retired = drain.update(app, |root: &mut MapDemo, cx| {
                                let r = root.pending.take().map(|(_, a, _)| a);
                                root.pending = Some((n, arc, ms));
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
