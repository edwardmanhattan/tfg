//! Camera glide: the eased move when the operator recentres on a hull.
//!
//! `request_frame` used to assign `self.center = at` outright. The view
//! teleported and the stale tile was drawn translated underneath while the
//! map thread caught up (the "buttery canvas" pass at `main.rs`, which
//! unprojects each screen corner through the live camera and reprojects it
//! into the texture's own space). That read as a stutter rather than as a
//! move, because nothing said "the camera is travelling" — only that the
//! picture had changed underneath.
//!
//! So the camera moves on a curve instead of jumping. The tile pipeline is
//! unchanged and still lags; now the lag reads as the settle.
//!
//! Easing comes from egui, not from here: `animate_bool_with_time_and_easing`
//! supplies a wall-clock ramp against `emath::easing::cubic_out`, so the
//! clock, the reduced-motion path (`animation_time = 0` collapses the ramp),
//! and the repaint request while in flight are all egui's problem. What this
//! module owns is the part egui cannot: a move toward a target that changes
//! mid-flight.
//!
//! Interruption is the whole design constraint. An operator who clicks a
//! second hull 120 ms into a 300 ms glide must not get a jump, so a new goal
//! snapshots the *current* position and starts a fresh ramp from it. The
//! animation id is keyed on a generation counter precisely so that a new goal
//! gets a new animation rather than resuming the old one from a fraction
//! that no longer matches any stored `from`.

use eframe::egui;
use egui::emath::easing;

/// DESIGN.md, Motion — "camera recentre on a hull". Zoom is deliberately
/// not eased here: `zoom_by` owns the stepper, and the wheel gesture is
/// continuous and must never be tweened.
pub const GLIDE_SECS: f32 = 0.30;

/// A camera position and the goal it is travelling toward.
///
/// Lon/lat, boxed as `f64` like every other coordinate in this client. The
/// eased value is the *live* camera — the one hit-testing, the UV remap and
/// the overlay projection all read — so everything on screen agrees with
/// everything clickable at every frame of the move, including the frames
/// where the tile underneath has not caught up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glide {
    pub at: (f64, f64),
    pub goal: (f64, f64),
    /// Where the current ramp started. Paired with `epoch`, never read
    /// without it.
    from: (f64, f64),
    /// Bumped on every retarget. Keys the egui animation id, so a new goal
    /// is a new animation starting at 0 instead of a resumption whose
    /// fraction no longer matches `from`.
    ///
    /// Named `epoch` rather than `gen` because `gen` is a reserved keyword
    /// in edition 2024.
    epoch: u64,
}

impl Glide {
    pub fn new(at: (f64, f64)) -> Self {
        Self {
            at,
            goal: at,
            from: at,
            epoch: 0,
        }
    }

    /// True once the camera has arrived and nothing is in flight. Used to
    /// skip the per-frame easing work entirely on an idle camera.
    pub fn settled(&self) -> bool {
        self.at == self.goal
    }

    /// Retarget. Returns `true` if this actually moved the goal, which is
    /// the caller's cue to bump nothing else — the generation bump and the
    /// `from` snapshot both happen here.
    pub fn retarget(&mut self, goal: (f64, f64)) -> bool {
        if self.goal == goal {
            return false;
        }
        // Snapshot where the camera actually is, mid-flight or not.
        self.from = self.at;
        self.goal = goal;
        self.epoch += 1;
        true
    }

    /// Jump with no easing. Used when the camera must be correct this frame
    /// rather than over the next few: a viewport resize, and the boot frame
    /// before anything has been drawn.
    pub fn snap(&mut self, at: (f64, f64)) {
        self.at = at;
        self.goal = at;
        self.from = at;
        self.epoch += 1;
    }

    /// Advance one frame. Returns `true` while the camera is still moving,
    /// so the caller can tell a settled frame from a travelling one.
    pub fn step(&mut self, ctx: &egui::Context, secs: f32) -> bool {
        if self.settled() {
            return false;
        }
        // egui owns the clock, the easing, and the repaint request while an
        // animation is in flight. `secs == 0` collapses to the target
        // immediately, which is the reduced-motion path for free.
        let t = ctx.animate_bool_with_time_and_easing(
            egui::Id::new(("camera.glide", self.epoch)),
            true,
            secs,
            easing::cubic_out,
        ) as f64;
        self.at = (
            self.from.0 + (self.goal.0 - self.from.0) * t,
            self.from.1 + (self.goal.1 - self.from.1) * t,
        );
        !self.settled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The interpolation the camera position is computed from, extracted so
    /// it can be tested without a `Context`. `Glide::step` is this, plus
    /// the egui clock that supplies `t`.
    pub fn lerp(from: (f64, f64), to: (f64, f64), t: f64) -> (f64, f64) {
        (
            from.0 + (to.0 - from.0) * t,
            from.1 + (to.1 - from.1) * t,
        )
    }

    #[test]
    fn a_new_glide_is_settled() {
        let g = Glide::new((1.0, 2.0));
        assert!(g.settled());
        assert_eq!(g.at, (1.0, 2.0));
    }

    #[test]
    fn retargeting_the_current_goal_is_a_no_op() {
        let mut g = Glide::new((0.0, 0.0));
        assert!(!g.retarget((0.0, 0.0)), "a redundant retarget must not bump the generation");
    }

    #[test]
    fn retarget_snapshots_the_current_position_not_the_old_goal() {
        let mut g = Glide::new((0.0, 0.0));
        assert!(g.retarget((10.0, 0.0)));
        // Halfway along the first move, the operator clicks a second hull.
        g.at = lerp((0.0, 0.0), (10.0, 0.0), 0.5);
        assert!(g.retarget((20.0, 0.0)));
        assert_eq!(g.from, (5.0, 0.0), "the new ramp starts where the camera is");
        assert_eq!(g.goal, (20.0, 0.0));
    }

    #[test]
    fn a_retarget_never_jumps_the_position() {
        let mut g = Glide::new((0.0, 0.0));
        g.retarget((10.0, 0.0));
        g.at = lerp((0.0, 0.0), (10.0, 0.0), 0.12);
        let before = g.at;
        g.retarget((-4.0, 7.0));
        // A fresh generation restarts the ramp at t=0, which is `from` —
        // and `from` is where the camera already was. Zero discontinuity.
        assert_eq!(g.from, before);
        assert_eq!(lerp(g.from, g.goal, 0.0), before);
    }

    #[test]
    fn snap_clears_the_ramp() {
        let mut g = Glide::new((0.0, 0.0));
        g.retarget((10.0, 0.0));
        g.snap((3.0, 4.0));
        assert!(g.settled());
        assert_eq!(g.at, (3.0, 4.0));
    }

    #[test]
    fn cubic_out_is_decelerating_and_never_overshoots() {
        let mut last = 0.0;
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            let v = easing::cubic_out(t);
            assert!((0.0..=1.0).contains(&v), "cubic_out left 0..=1 at t={t}: {v}");
            assert!(v >= last, "cubic_out must be monotonic, fell at t={t}");
            last = v;
        }
        assert!((easing::cubic_out(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_glide_reaches_the_goal_on_its_final_frame() {
        let mut g = Glide::new((0.0, 0.0));
        g.retarget((8.0, -2.0));
        // t == 1 is the last value the ramp produces; at that point the
        // camera must equal the goal, or it never "settles".
        let arrived = lerp(g.from, g.goal, 1.0);
        assert_eq!(arrived, (8.0, -2.0));
    }
}
