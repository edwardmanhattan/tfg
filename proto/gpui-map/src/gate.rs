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

use gpui::{App, Image, ImageFormat, Window};

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

/// Replays PNG frames from a directory the way the map thread sends them:
/// in order, newest-wins to the consumer, `burst_ms` apart. Frames come
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
