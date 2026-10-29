//! UI sounds: synthesized clicks, dings and buzzes, no audio assets.
//!
//! Everything is generated in memory as short mono sine bursts and played
//! on a background thread through rodio, so the UI thread never blocks on
//! a device. `play()` is fire-and-forget: if there is no audio device
//! (a daemon-less delivery machine, a headless harness) the player thread
//! logs once and exits, and every later `play()` is a no-op send into a
//! dead channel — the console stays silent, never broken.
//!
//! The contract the UI relies on:
//! - any primary click → `Sfx::Click`, from the click hook every ROOT carries
//!   (the console, the composer, the sign-in card) and from every card that
//!   is its own mouse boundary (the basket, the pickers),
//! - an interaction that landed → `Sfx::Ding`, from the console's `Status`
//!   vocabulary and from the composer's `settle_note`,
//! - an interaction the client refused → `Sfx::Buzz`, the same two hooks.
//!
//! ONE INTERACTION IS ONE SOUND. A drop reaches the verdict seam and the
//! status vocabulary both, so `play` collapses the same kind fired twice
//! inside [`SAME_EVENT_MS`].
//!
//! Server refusals (a 4xx landing through the REST pump) stay loud on the
//! status line and do NOT buzz: the pump's error path owns that meaning.

use std::num::{NonZeroU16, NonZeroU32};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// What happened, as the operator's ears hear it.
#[derive(Debug, Clone, Copy)]
pub enum Sfx {
    /// Any primary click. Short and quiet — it fires often.
    Click,
    /// An interaction landed. Two-tone, up.
    Ding,
    /// The client refused an interaction. Low, blunt.
    Buzz,
    /// A TELEGRAM arrived on the wire.
    ///
    /// A short carrier tick, distinct from the click and from the ding:
    /// the operator must be able to tell "something arrived" from "you
    /// clicked something" without looking at the screen. Two quick
    /// high bursts, like a radio acknowledging a call.
    Telegram,
    /// An ADMINISTRATIVE message arrived on the wire.
    ///
    /// The staff counterpart of the telegram tick: lower and longer,
    /// because the traffic between the Game Master and the Judges is
    /// rarer and reads as an interruption rather than as chatter.
    Admin,
}

const RATE: u32 = 22_050;

impl Sfx {
    /// The family this sound belongs to, for the one-event-one-sound rule.
    /// Every kind is its own family today; the method is what a future
    /// variant with two renderings would override.
    fn same_wave(self, other: Sfx) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&other)
    }
}

/// One sine burst: `freq` Hz for `ms` milliseconds with an exponential
/// decay so it ends in silence rather than a click of its own.
fn burst(freq: f32, ms: u64, vol: f32) -> Vec<f32> {
    let n = (RATE as u64 * ms / 1000) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let env = (-4.0 * i as f32 / n as f32).exp();
            (2.0 * std::f32::consts::PI * freq * t).sin() * env * vol
        })
        .collect()
}

/// A square-ish blunt tone for the buzz: sign-of-sine, softened by the
/// same decay so it reads as a refusal rather than a fault.
fn blunt(freq: f32, ms: u64, vol: f32) -> Vec<f32> {
    let n = (RATE as u64 * ms / 1000) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let env = (-2.5 * i as f32 / n as f32).exp();
            (2.0 * std::f32::consts::PI * freq * t).sin().signum() * env * vol * 0.6
        })
        .collect()
}

fn samples(kind: Sfx) -> Vec<f32> {
    match kind {
        Sfx::Click => burst(1500.0, 25, 0.22),
        Sfx::Ding => {
            let mut v = burst(880.0, 90, 0.28);
            v.extend(burst(1318.0, 140, 0.28));
            v
        }
        Sfx::Buzz => blunt(140.0, 220, 0.30),
        Sfx::Telegram => {
            // Two quick high bursts: a carrier acknowledging a call.
            let mut v = burst(1760.0, 40, 0.20);
            v.extend(burst(1760.0, 40, 0.20));
            v
        }
        Sfx::Admin => {
            // Lower and longer: an interruption, not chatter.
            let mut v = burst(587.0, 90, 0.24);
            v.extend(burst(440.0, 160, 0.24));
            v
        }
    }
}

fn player() -> &'static std::sync::mpsc::Sender<Sfx> {
    static PLAYER: OnceLock<std::sync::mpsc::Sender<Sfx>> = OnceLock::new();
    PLAYER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Sfx>();
        // A thread that cannot spawn is a console without sounds, not a
        // console that fails to start — the sender survives either way
        // and a dead channel drops silently in `play()`.
        let _ = std::thread::Builder::new()
            .name("sfx".to_string())
            .spawn(move || {
                // rodio 0.22: no OutputStream/Sink pair — a device sink
                // owns the mixer, and a Player on it queues our bursts.
                // Both live here, on this thread, for the app's lifetime.
                let device_sink =
                    match rodio::stream::DeviceSinkBuilder::open_default_sink() {
                        Ok(sink) => sink,
                        Err(e) => {
                            eprintln!(
                                "sfx: no audio device ({e:?}) — console stays silent"
                            );
                            return;
                        }
                    };
                let player = rodio::Player::connect_new(device_sink.mixer());
                let channels = NonZeroU16::new(1).unwrap();
                let rate = NonZeroU32::new(RATE).unwrap();
                for kind in rx {
                    player.append(rodio::buffer::SamplesBuffer::new(
                        channels,
                        rate,
                        samples(kind),
                    ));
                }
            });
        tx
    })
}

/// How close two identical sounds must be to be the same event.
///
/// ONE INTERACTION CAN REACH TWO OWNERS. A drop's staging sets a status line
/// (the vocabulary hook) and the verdict seam plays the same sound, so without
/// this the operator hears the landing twice. Thirty milliseconds is shorter
/// than any hand moves between two gestures and longer than a frame's worth of
/// dispatch.
const SAME_EVENT_MS: u128 = 30;

/// Play one sound, never blocking. A dead player (no device, hung-up
/// thread) drops the send silently — audio is garnish, and garnish must
/// not fail a console.
///
/// The last kind and instant are remembered so the same sound fired twice for
/// one event is heard once. Different sounds never collapse: a refusal after a
/// landing is two answers and the ear should hear both.
pub fn play(kind: Sfx) {
    static LAST: OnceLock<Mutex<Option<(Sfx, Instant)>>> = OnceLock::new();
    let gate = LAST.get_or_init(|| Mutex::new(None));
    if let Ok(mut last) = gate.lock() {
        if let Some((previous, at)) = *last {
            if previous.same_wave(kind) && at.elapsed().as_millis() < SAME_EVENT_MS {
                return;
            }
        }
        *last = Some((kind, Instant::now()));
    }
    let _ = player().send(kind);
}
