//! The map handoff: newest-wins queue, hold-until-ready promotion, and the
//! retirement that keeps GPUI's sprite atlas and asset map bounded.
//!
//! Extracted from the U6 spike so `gpui-map` (the texture-path proof) and
//! `gpui-island` (the console surface, map included) drive the SAME code.
//! U8 then copies one implementation into the shell, not two.
//!
//! Two facts about pinned gpui 0.2.2 shape this, both measured rather than
//! assumed (see `docs/gpui-migration-handoff.md`, U6):
//!
//! - `ImageSource::Image` decodes asynchronously and returns `None` until
//!   it lands, and the asset id is the hash of the bytes, so every new
//!   frame starts a fresh load. Displaying a frame the moment it is queued
//!   therefore paints nothing — permanently, under churn. The displayed
//!   frame is held until its successor resolves.
//! - Neither the asset map nor the sprite atlas evicts on replace.
//!   `drop_image` frees the atlas tile, `remove_asset` the CPU entry; a
//!   superseded pending frame has a CPU entry but no tile.
//!
//! Cost of the gate is one frame of decode lag (p50 39ms on real 800x600
//! map frames) against egui's synchronous `TextureHandle::set` — invisible
//! at the map thread's own pump cadence, which is an order of magnitude
//! slower.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use gpui::{App, Image, ImageFormat, RenderImage, Window};
use image::{Frame, ImageBuffer, Rgba};

/// What the HUD and the verification log read.
#[derive(Clone, Copy, Debug, Default)]
pub struct GateStats {
    /// Sequence number of the frame on screen.
    pub displayed: u64,
    /// Producer-side encode cost for it (0 when replayed off disk).
    pub encode_ms: f64,
    /// Decode + notify lag the user would feel, in ms.
    pub lag_ms: f64,
    /// Sequence number still decoding, if any.
    pub pending: Option<u64>,
    /// Promotions so far.
    pub promoted: u64,
    /// Atlas tiles dropped so far. Tracks `promoted` strictly apart from
    /// the first, which has nothing to retire — the invariant the memory
    /// test asserts.
    pub retired: u64,
}

/// A frame that has been queued but has not yet decoded.
struct Pending {
    no: u64,
    image: Arc<Image>,
    encode_ms: f64,
    queued: Instant,
}

#[derive(Default)]
pub struct FrameGate {
    displayed: Option<Arc<Image>>,
    pending: Option<Pending>,
    promoted: u64,
    retired: u64,
    last_no: u64,
    last_encode_ms: f64,
    last_lag_ms: f64,
}

impl FrameGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Newest-wins on the QUEUE, never on the display. A superseded pending
    /// frame is handed back so the caller can `remove_asset` it — it never
    /// painted, so it has no atlas tile, but its decoded entry would
    /// otherwise stay in the asset map for the life of the window. The
    /// removal is the caller's because this runs inside
    /// `Entity::update`, which already holds the app.
    pub fn queue(&mut self, no: u64, bytes: Vec<u8>, encode_ms: f64) -> Option<Arc<Image>> {
        let stale = self.pending.take().map(|p| p.image);
        self.pending = Some(Pending {
            no,
            image: Arc::new(Image::from_bytes(ImageFormat::Png, bytes)),
            encode_ms,
            queued: Instant::now(),
        });
        stale
    }

    /// Hold-until-ready: promote only once the successor decodes, so the
    /// surface never paints an empty frame; then retire the outgoing frame
    /// on both axes. Returns true when a promotion happened.
    pub fn tick(&mut self, window: &mut Window, app: &mut App) -> bool {
        let Some(pending) = &self.pending else {
            return false;
        };
        if pending.image.clone().use_render_image(window, app).is_none() {
            return false;
        }
        let pending = self.pending.take().expect("checked just above");
        let lag = pending.queued.elapsed().as_secs_f64() * 1000.0;

        if let Some(old) = self.displayed.take() {
            if let Some(tile) = old.clone().get_render_image(window, app) {
                app.drop_image(tile, Some(window));
                self.retired += 1;
            }
            old.remove_asset(app);
        }

        eprintln!(
            "gate promote frame={} lag={lag:.1}ms promoted={} retired={}",
            pending.no,
            self.promoted + 1,
            self.retired
        );
        self.displayed = Some(pending.image);
        self.last_no = pending.no;
        self.last_encode_ms = pending.encode_ms;
        self.last_lag_ms = lag;
        self.promoted += 1;
        true
    }

    /// The frame to paint. Nothing until the first promotion lands.
    pub fn image(&self) -> Option<Arc<Image>> {
        self.displayed.clone()
    }

    pub fn stats(&self) -> GateStats {
        GateStats {
            displayed: self.last_no,
            encode_ms: self.last_encode_ms,
            lag_ms: self.last_lag_ms,
            pending: self.pending.as_ref().map(|p| p.no),
            promoted: self.promoted,
            retired: self.retired,
        }
    }
}

/// The raw path, and the correction that came with it.
///
/// `ImageSource::Render` was written off in the first U6 survey as
/// unreachable, on the reading that `RenderImage::new` needs a `Frame` that
/// is gpui-private. It is not: `Frame` is `image::Frame`, re-exported from
/// the `image` crate the shader already depends on, and `RenderImage::new`
/// takes `impl Into<SmallVec<[Frame; 1]>>` — so an array of one frame
/// converts. An external crate CAN build a `RenderImage` from raw bytes.
///
/// That matters because it removes every moving part the PNG path needed:
/// no encode, no asset-map entry, no async decode, and therefore nothing to
/// hold — `ImageSource::Render` resolves on the spot, so the frame is
/// paintable in the frame it arrives. What remains is the atlas tile, which
/// is still minted per `ImageId` and still has to be retired by hand.
///
/// It also brings the SMEAR: a picture that has fallen behind the camera
/// does not reach the whole window, and the band it misses is filled with
/// the frame's own edge, stretched. GPUI paints an image into a rectangle
/// and cannot sample a piece of one, so each edge is its own small image.
pub struct FrameSlot {
    displayed: Option<Arc<RenderImage>>,
    /// The frame's edges, in the order [`FrameEdge`] numbers them: the
    /// smear for the band the picture does not reach.
    edges: [Option<Arc<RenderImage>>; 4],
    shown: u64,
    retired: u64,
}

/// Which edge of the frame a smear belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameEdge {
    Left = 0,
    Right = 1,
    Top = 2,
    Bottom = 3,
}

impl FrameSlot {
    pub fn new() -> Self {
        Self {
            displayed: None,
            edges: [None, None, None, None],
            shown: 0,
            retired: 0,
        }
    }

    /// Hand over one frame of BGRA bytes (the sprite atlas is
    /// `Bgra8Unorm`; the swizzle belongs on the producer's thread), retiring
    /// the tile of the frame it replaces.
    pub fn show(
        &mut self,
        window: &mut Window,
        app: &mut App,
        no: u64,
        size: (u32, u32),
        bgra: Vec<u8>,
    ) {
        // The edges are cut from the same bytes, before they are handed to
        // the frame: the smear has to be the picture's own edge, not a
        // colour guessed from it.
        let edges = edge_strips(size.0, size.1, &bgra);
        let Some(buffer) = ImageBuffer::<Rgba<u8>, _>::from_raw(size.0, size.1, bgra) else {
            return; // wrong byte count; nothing to show
        };
        let image = Arc::new(RenderImage::new([Frame::new(buffer)]));
        if let Some(old) = self.displayed.replace(image) {
            app.drop_image(old, Some(window));
            self.retired += 1;
        }
        for (slot, edge) in self.edges.iter_mut().zip(edges) {
            if let Some(old) = std::mem::replace(slot, edge) {
                app.drop_image(old, Some(window));
                self.retired += 1;
            }
        }
        self.shown = no;
    }

    pub fn image(&self) -> Option<Arc<RenderImage>> {
        self.displayed.clone()
    }

    /// The smear for one edge: the frame's own border, stretched across the
    /// band the picture does not reach.
    pub fn edge(&self, edge: FrameEdge) -> Option<Arc<RenderImage>> {
        self.edges[edge as usize].clone()
    }

    pub fn shown(&self) -> u64 {
        self.shown
    }

    pub fn retired(&self) -> u64 {
        self.retired
    }
}

/// The frame's outermost two columns and rows, as four small images.
///
/// Two pixels and not one: the atlas packs a tile with no padding, so a
/// one-pixel strip has nothing but its neighbour's tile to sample at the
/// edge, and the seam shows as a line of the wrong colour. Two pixels give
/// the sampler a neighbour inside the tile, and the difference between one
/// and two pixels of smear is a difference nobody can see.
fn edge_strips(w: u32, h: u32, bgra: &[u8]) -> [Option<Arc<RenderImage>>; 4] {
    let (w, h) = (w as usize, h as usize);
    let stride = w * 4;
    if w < 2 || h < 2 || bgra.len() < stride * h {
        return [None, None, None, None];
    }
    // Two columns of every row: the frame's left or right edge.
    let columns = |x0: usize| -> Option<Arc<RenderImage>> {
        let mut buf = Vec::with_capacity(2 * h * 4);
        for y in 0..h {
            let at = y * stride + x0 * 4;
            buf.extend_from_slice(&bgra[at..at + 8]);
        }
        Some(Arc::new(RenderImage::new([Frame::new(
            ImageBuffer::<Rgba<u8>, _>::from_raw(2, h as u32, buf)?,
        )])))
    };
    // Two rows of every column: the frame's top or bottom edge.
    let rows = |y0: usize| -> Option<Arc<RenderImage>> {
        let mut buf = Vec::with_capacity(w * 2 * 4);
        for y in y0..y0 + 2 {
            buf.extend_from_slice(&bgra[y * stride..(y + 1) * stride]);
        }
        Some(Arc::new(RenderImage::new([Frame::new(
            ImageBuffer::<Rgba<u8>, _>::from_raw(w as u32, 2, buf)?,
        )])))
    };
    [
        columns(0),
        columns(w - 2),
        rows(0),
        rows(h - 2),
    ]
}

/// Replays PNG frames from a directory the way the map thread sends them:/// in order, newest-wins to the consumer, `burst_ms` apart. Frames come
/// from `examples/map_burst`, which drives the real map engine — the point
/// being that the gate is exercised by real pixels at a real cadence, not
/// by a generator that flatters it.
pub struct FrameReplay {
    rx: Receiver<(u64, Vec<u8>, f64)>,
    frames: usize,
}

impl FrameReplay {
    /// Start the replay thread. `dir` holding no PNGs leaves the gate
    /// hungry and the surface black, which is the honest state: this never
    /// invents pixels.
    pub fn start(dir: &str, burst_ms: u64, looping: bool) -> Self {
        let mut paths: Vec<PathBuf> = match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "png"))
                .collect(),
            Err(_) => Vec::new(),
        };
        paths.sort();
        let frames: Vec<Vec<u8>> = paths.iter().filter_map(|p| std::fs::read(p).ok()).collect();
        let count = frames.len();
        let (tx, rx) = mpsc::channel();
        if count > 0 {
            std::thread::spawn(move || {
                let mut n: u64 = 0;
                loop {
                    let bytes = frames[(n as usize) % count].clone();
                    n += 1;
                    if tx.send((n, bytes, 0.0)).is_err() {
                        return; // consumer gone
                    }
                    if !looping && n as usize >= count {
                        // Burst spent: hold the last frame so a capture is
                        // of a settled picture. The thread stays parked so
                        // the channel stays open.
                        std::thread::sleep(Duration::from_secs(3600));
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(burst_ms));
                }
            });
        }
        eprintln!("replay: {count} frames from {dir} (burst {burst_ms}ms, loop {looping})");
        Self { rx, frames: count }
    }

    /// Newest frame queued since the last call, dropping any backlog — the
    /// consumer half of newest-wins.
    pub fn latest(&self) -> Option<(u64, Vec<u8>, f64)> {
        let mut latest = None;
        for frame in self.rx.try_iter() {
            latest = Some(frame);
        }
        latest
    }

    pub fn frames(&self) -> usize {
        self.frames
    }
}

/// The same replay for the raw path: PNGs decoded ONCE, up front, into BGRA
/// buffers — which is what the map thread actually produces (it takes
/// `LiveMap::frame_rgba` and swizzles). Decoding here rather than in the UI
/// keeps the comparison honest: the per-frame cost of the raw path is the
/// swizzle, not a decode.
pub struct RawReplay {
    rx: Receiver<(u64, u32, u32, Vec<u8>)>,
    frames: usize,
}

impl RawReplay {
    pub fn start(dir: &str, burst_ms: u64, looping: bool) -> Self {
        let mut paths: Vec<PathBuf> = match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "png"))
                .collect(),
            Err(_) => Vec::new(),
        };
        paths.sort();
        let mut frames: Vec<(u32, u32, Vec<u8>)> = Vec::new();
        for path in &paths {
            if let Ok(img) = image::open(path) {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                let mut bytes = rgba.into_raw();
                swizzle_rgba_to_bgra(&mut bytes);
                frames.push((w, h, bytes));
            }
        }
        let count = frames.len();
        let (tx, rx) = mpsc::channel();
        if count > 0 {
            std::thread::spawn(move || {
                let mut n: u64 = 0;
                loop {
                    let (w, h, bytes) = frames[(n as usize) % count].clone();
                    n += 1;
                    if tx.send((n, w, h, bytes)).is_err() {
                        return;
                    }
                    if !looping && n as usize >= count {
                        std::thread::sleep(Duration::from_secs(3600));
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(burst_ms));
                }
            });
        }
        eprintln!("raw replay: {count} frames from {dir} (burst {burst_ms}ms, loop {looping})");
        Self { rx, frames: count }
    }

    pub fn latest(&self) -> Option<(u64, u32, u32, Vec<u8>)> {
        let mut latest = None;
        for frame in self.rx.try_iter() {
            latest = Some(frame);
        }
        latest
    }

    pub fn frames(&self) -> usize {
        self.frames
    }
}

/// RGBA -> BGRA in place. The sprite atlas is `Bgra8Unorm`; gpui's own
/// decoder does the same swap, and it belongs on the producer's thread.
pub fn swizzle_rgba_to_bgra(bytes: &mut [u8]) {
    for px in bytes.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smear is the frame's OWN edge: the first two columns, the last
    /// two columns, the first two rows and the last two rows, cut from the
    /// bytes the map thread sent.
    ///
    /// A smear that was a guess — a colour, a fill, a gradient of nothing —
    /// would put a different map at the edge of the picture, and the edge is
    /// where the operator is looking when they are dragging.
    #[test]
    fn the_smear_is_the_frames_own_edge() {
        // A 4x3 frame, every pixel distinct.
        let (w, h) = (4u32, 3u32);
        let mut bytes = Vec::new();
        for y in 0..h {
            for x in 0..w {
                bytes.extend_from_slice(&[(x * 10) as u8, (y * 10) as u8, 7, 255]);
            }
        }
        let strips = edge_strips(w, h, &bytes);

        // The left edge: the first two columns of every row.
        let left = strips[0].as_ref().expect("the left edge");
        assert_eq!(left.size(0).width.0, 2, "two columns");
        assert_eq!(left.size(0).height.0, 3, "every row");
        let raw = left.as_bytes(0).expect("the strip's bytes");
        assert_eq!(&raw[0..4], &[0, 0, 7, 255], "column 0, row 0");
        assert_eq!(&raw[4..8], &[10, 0, 7, 255], "column 1, row 0");
        assert_eq!(&raw[16..20], &[0, 20, 7, 255], "column 0, row 2");

        // The right edge: the last two columns.
        let right = strips[1].as_ref().expect("the right edge");
        let raw = right.as_bytes(0).expect("the strip's bytes");
        assert_eq!(&raw[0..4], &[20, 0, 7, 255], "column 2, row 0");
        assert_eq!(&raw[4..8], &[30, 0, 7, 255], "column 3, row 0");

        // The top edge: the first two rows, every column.
        let top = strips[2].as_ref().expect("the top edge");
        assert_eq!(top.size(0).width.0, 4, "every column");
        assert_eq!(top.size(0).height.0, 2, "two rows");
        let raw = top.as_bytes(0).expect("the strip's bytes");
        assert_eq!(&raw[0..4], &[0, 0, 7, 255], "column 0, row 0");
        assert_eq!(&raw[12..16], &[30, 0, 7, 255], "column 3, row 0");

        // The bottom edge: the last two rows.
        let bottom = strips[3].as_ref().expect("the bottom edge");
        let raw = bottom.as_bytes(0).expect("the strip's bytes");
        assert_eq!(&raw[0..4], &[0, 10, 7, 255], "column 0, row 1");
        assert_eq!(&raw[12..16], &[30, 10, 7, 255], "column 3, row 1");
        assert_eq!(&raw[28..32], &[30, 20, 7, 255], "column 3, row 2");
    }

    /// A frame too small to have an edge is no smear at all, and a frame
    /// whose bytes do not match its size is nothing to smear.
    #[test]
    fn a_frame_too_small_to_smear_has_no_edges() {
        let bytes = vec![0u8; 4 * 4 * 2 * 2];
        let strips = edge_strips(1, 4, &bytes);
        assert!(strips.iter().all(|s| s.is_none()), "a one-pixel frame has no edge");
        let strips = edge_strips(4, 4, &bytes[..8]);
        assert!(
            strips.iter().all(|s| s.is_none()),
            "a frame whose bytes do not match its size is nothing to smear"
        );
    }
}
