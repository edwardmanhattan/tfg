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
//! The adopted answer is `gate.rs`: hold-until-ready promotion plus
//! explicit retirement, which this binary drives. Frames come from
//! `MAP_FRAMES_DIR` (default `/tmp/opencode/mapframes`, written by
//! `examples/map_burst` from the real map engine) and fall back to a
//! synthetic gradient when no PNGs are present — the fallback exists so
//! the crate is still useful with no data, and its numbers are kept only
//! to show how much they misled (a gradient is near-worst-case entropy;
//! real tiles encode 5x cheaper).
//!
//! `LOOP=1` recycles the burst forever for the memory test; a plain run
//! replays it once and holds the last frame. `BURST_MS` sets the cadence.
//! Verdict + numbers live in `docs/gpui-migration-handoff.md`.

#[path = "../../../src/map_gate.rs"]
mod gate;

use std::io::Cursor;
use std::time::Duration;

use gpui::{
    App, AppContext, Application, AsyncApp, Bounds, Context, IntoElement, ParentElement, Render,
    Styled, Window, WindowBounds, WindowOptions, div, img, px, size,
};
use image::{ImageBuffer, Rgba, RgbaImage};

use gate::{FrameGate, FrameReplay, FrameSlot};

const FRAME_W: u32 = 800;
const FRAME_H: u32 = 600;

/// The rail that makes a channel-order mistake obvious: cyan is emitted, so
/// a wrong swizzle paints it orange. The PNG path's frames carry the same
/// bar, so the two captures can be compared directly.
fn swizzle_rgba_to_bgra(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

/// `MODE=render`: the raw path. No encode, no decode, nothing held.
struct RawDemo {
    slot: FrameSlot,
    incoming: Option<(u64, (u32, u32), Vec<u8>)>,
    produced: u64,
}

impl Render for RawDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Nothing to wait for: `ImageSource::Render` is already resolved, so
        // the frame that arrived this tick is the frame that paints.
        if let Some((no, size, bgra)) = self.incoming.take() {
            self.slot.show(window, cx, no, size, bgra);
        }
        let mut root = div().relative().size_full().bg(gpui::black());
        if let Some(frame) = self.slot.image() {
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
                .child(format!(
                    "raw frame {} shown {} ret {} produced {}",
                    self.slot.shown(),
                    self.slot.shown(),
                    self.slot.retired(),
                    self.produced
                )),
        )
    }
}

struct MapDemo {
    gate: FrameGate,
}

impl Render for MapDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.gate.tick(window, cx);

        let mut root = div().relative().size_full().bg(gpui::black());
        if let Some(frame) = self.gate.image() {
            root = root.child(div().absolute().size_full().child(img(frame)));
        }
        let s = self.gate.stats();
        let pend = s.pending.map(|n| n.to_string()).unwrap_or_else(|| "-".into());
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
                    s.displayed, s.encode_ms, s.lag_ms, pend, s.promoted, s.retired
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
    // Producer: with real frames on disk the replay stands in for the map
    // thread (the encode cost was measured offline by `map_burst`); with
    // none, generate and encode here so the whole round trip is timed.
    let dir = std::env::var("MAP_FRAMES_DIR")
        .unwrap_or_else(|_| "/tmp/opencode/mapframes".to_string());
    let burst_ms: u64 = std::env::var("BURST_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(80);
    let looping = std::env::var("LOOP").as_deref() == Ok("1");
    let raw = std::env::var("MODE").as_deref() == Ok("render");
    let replay = FrameReplay::start(&dir, burst_ms, looping);
    let synthetic = replay.frames() == 0;
    let (syn_tx, syn_rx) = std::sync::mpsc::channel::<(u64, Vec<u8>, f64)>();
    if synthetic {
        std::thread::spawn(move || {
            let mut n: u64 = 0;
            loop {
                n += 1;
                let rgba = render_frame(n);
                let t0 = std::time::Instant::now();
                let mut png = Vec::new();
                {
                    use image::ImageEncoder as _;
                    image::codecs::png::PngEncoder::new(Cursor::new(&mut png))
                        .write_image(&rgba, FRAME_W, FRAME_H, image::ExtendedColorType::Rgba8)
                        .expect("png encode");
                }
                let ms = t0.elapsed().as_secs_f64() * 1000.0;
                if syn_tx.send((n, png, ms)).is_err() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(burst_ms));
            }
        });
    }

    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(860.0), px(680.0)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        if raw {
            let entity = cx.new(|_| RawDemo {
                slot: FrameSlot::new(),
                incoming: None,
                produced: 0,
            });
            let drain = entity.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                let mut n: u64 = 0;
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(burst_ms))
                        .await;
                    n += 1;
                    let mut rgba = render_frame(n)
                        .into_raw();
                    // The map thread's own work: swizzle once, off the UI
                    // thread, exactly where egui's unmul swap lived.
                    swizzle_rgba_to_bgra(&mut rgba);
                    let no = n;
                    let _ = cx.update(|app| {
                        drain.update(app, |root: &mut RawDemo, cx| {
                            root.incoming = Some((no, (FRAME_W, FRAME_H), rgba));
                            root.produced = no;
                            cx.notify();
                        })
                    });
                    if n % 25 == 0 {
                        eprintln!("raw produced={n}");
                    }
                }
            })
            .detach();
            cx.open_window(opts, |_window, _cx| entity.clone()).unwrap();
            cx.activate(true);
            return;
        }

        let entity = cx.new(|_| MapDemo {
            gate: FrameGate::new(),
        });
        let drain = entity.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_millis(20)).await;
                let frame = if synthetic {
                    let mut latest = None;
                    for f in syn_rx.try_iter() {
                        latest = Some(f);
                    }
                    latest
                } else {
                    replay.latest()
                };
                if let Some((no, bytes, encode_ms)) = frame {
                    let _ = cx.update(|app| {
                        let stale = drain.update(app, |root: &mut MapDemo, cx| {
                            // Newest-wins on the queue; the superseded
                            // frame is removed below, outside this borrow.
                            let stale = root.gate.queue(no, bytes, encode_ms);
                            cx.notify();
                            stale
                        });
                        if let Some(stale) = stale {
                            stale.remove_asset(app);
                        }
                    });
                }
            }
        })
        .detach();
        cx.open_window(opts, |_window, _cx| entity.clone()).unwrap();
        cx.activate(true);
    });
}
