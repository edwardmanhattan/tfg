//! Game clock (grill #17, ADR-0004): maps real time to game time for the sim.
//!
//! Decisions:
//! - **Fixed ratio**: `game_now = game_start + elapsed_real × ratio`, with
//!   `ratio` fixed for the session. 1:1 until a real `Game` supplies bounds.
//! - **Stretch past the scheduled end**: the ratio keeps applying past
//!   `real_end`; sessions end explicitly, never by the clock.
//! - **Pause is a full hold**: motion, `game_now`, and the log clock freeze
//!   while the wall clock runs on. The sim is tick-driven, so the hold is
//!   enforced tick-wise — paused ticks advance nothing — equivalent to a
//!   pause accumulator and never edits stored session bounds.
//! - **Motion integrates over game elapsed** (`distance = speed × game_dt`).
//! - **`game_ts` is derived, never stored** on fixes (ADR-0004).

use chrono::{DateTime, Utc};

/// Game seconds per real second. 1:1 until a real `Game` model supplies
/// session bounds (real_start/end, game_start/end) to derive a ratio from.
#[derive(Debug, Clone)]
pub struct GameClock {
    ratio: f64,
    started: bool,
    game_elapsed: f64,
    paused: bool,
    /// Real-world instant the session started (first tick), UTC. The
    /// anchor `game_now` is derived from: game_now = game_start + elapsed.
    game_start: Option<DateTime<Utc>>,
}

impl Default for GameClock {
    fn default() -> Self {
        Self::new(1.0)
    }
}

impl GameClock {
    pub fn new(ratio: f64) -> Self {
        Self { ratio, started: false, game_elapsed: 0.0, paused: false, game_start: None }
    }

    /// Feed one real-time tick of `real_dt_secs`; returns game seconds
    /// elapsed this tick. The first tick stamps the session start
    /// (`real_start`) and moves nothing; paused ticks move nothing.
    pub fn tick(&mut self, real_dt_secs: f64) -> f64 {
        if !self.started {
            self.started = true;
            return 0.0;
        }
        let game_dt = if self.paused { 0.0 } else { real_dt_secs * self.ratio };
        self.game_elapsed += game_dt;
        game_dt
    }

    /// Stamp the session's real start (call once, at the first tick).
    /// `real_start_ts` is an RFC3339 UTC string, as `now_ts()` produces.
    pub fn begin(&mut self, real_start_ts: &str) {
        self.game_start = real_start_ts.parse::<DateTime<Utc>>().ok();
    }

    /// Derived game-now clock reading, humane format (UTC). None until the
    /// session has begun and the start stamp parsed.
    pub fn game_now_ts(&self) -> Option<String> {
        self.game_start.map(|s| {
            (s + chrono::Duration::seconds(self.game_elapsed as i64))
                .format("%Y-%m-%d %H:%M:%SZ")
                .to_string()
        })
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn ratio(&self) -> f64 {
        self.ratio
    }

    /// Session pace (session flow): the organizer fixes the real-to-game
    /// ratio at session start; mid-session changes just re-aim the slope.
    pub fn set_ratio(&mut self, ratio: f64) {
        self.ratio = ratio;
    }

    pub fn started(&self) -> bool {
        self.started
    }

    /// Game seconds since the session's game_start (floored).
    pub fn game_elapsed_secs(&self) -> u64 {
        self.game_elapsed as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_tick_stamps_start_and_moves_nothing() {
        let mut c = GameClock::default();
        assert!(!c.started());
        assert_eq!(c.tick(2.0), 0.0);
        assert!(c.started());
        assert_eq!(c.game_elapsed_secs(), 0);
    }

    #[test]
    fn one_to_one_default_maps_real_to_game() {
        let mut c = GameClock::default();
        c.tick(0.0);
        assert!((c.tick(2.0) - 2.0).abs() < 1e-9);
        assert_eq!(c.game_elapsed_secs(), 2);
    }

    #[test]
    fn ratio_compresses_real_into_game() {
        let mut c = GameClock::new(60.0); // 1 real min = 1 game hour
        c.tick(0.0);
        assert!((c.tick(1.0) - 60.0).abs() < 1e-9);
        assert_eq!(c.game_elapsed_secs(), 60);
    }

    #[test]
    fn game_now_derives_from_real_start_plus_elapsed() {
        let mut c = GameClock::default();
        assert_eq!(c.game_now_ts(), None, "no reading before the session begins");
        c.begin("2026-09-14T10:00:00.000Z");
        c.tick(0.0);
        c.tick(90.0); // 1.5 game minutes
        assert_eq!(c.game_now_ts().as_deref(), Some("2026-09-14 10:01:30Z"));
    }

    #[test]
    fn pause_holds_game_time_while_wall_clock_runs() {
        let mut c = GameClock::default();
        c.tick(0.0);
        c.tick(2.0); // +2 game secs
        c.set_paused(true);
        assert_eq!(c.tick(28.0), 0.0); // 28 real secs pass, game holds
        assert_eq!(c.game_elapsed_secs(), 2);
        c.set_paused(false);
        assert!((c.tick(2.0) - 2.0).abs() < 1e-9); // resumes from the hold
        assert_eq!(c.game_elapsed_secs(), 4);
    }
}
