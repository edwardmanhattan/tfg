//! Presentation movement model (see CONTEXT.md): Fix / Ship / Track / Trail.
//!
//! Rules (from the v0 + geo tickets):
//! - Backend `ts` (RFC3339 UTC) orders fixes; an older-or-equal `ts` is dropped.
//! - A ship with no fix for 3 consecutive polls is `stale` (marker kept).
//! - Displayed position lerps previous -> latest by wall-clock fraction,
//!   with a small-jump guard (sub-8 m jumps hold, killing GPS jitter).
//! - Each track is bounded (default: last 60 fixes); the trail renders from it.

use std::collections::{HashMap, VecDeque};

use chrono::DateTime;
use serde::Deserialize;

use super::coordinates::GeoPosition;

/// Provenance of a Fix: backend report or synthetic sim emission.
/// Decided in ADR-0003: the jitter guard and displays key off this.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FixSource {
    #[default]
    Wire,
    Sim,
}

/// One accepted position report for a ship.
#[derive(Debug, Clone, PartialEq)]
pub struct Fix {
    /// Identity of the ship. Wire format uses `ship_id`.
    pub ship_id: String,
    pub position: GeoPosition,
    /// RFC3339 UTC, e.g. `2026-09-12T00:00:02Z`. Orders fixes.
    pub ts: String,
    pub heading_deg: Option<f32>,
    pub speed_kn: Option<f32>,
    pub source: FixSource,
    /// Ingest sequence stamped by the Registry (Log grill, #20): total
    /// ingest order, gaps where out-of-order fixes were dropped.
    pub seq: u64,
}

/// Wire shape of a fix: flat lat/lon (serde) -> [`Fix`].
#[derive(Debug, Clone, Deserialize)]
struct WireFix {
    ship_id: String,
    lat: f64,
    lon: f64,
    ts: String,
    heading_deg: Option<f32>,
    speed_kn: Option<f32>,
}

impl From<WireFix> for Fix {
    fn from(w: WireFix) -> Self {
        Self {
            ship_id: w.ship_id,
            position: GeoPosition { latitude: w.lat, longitude: w.lon },
            ts: w.ts,
            heading_deg: w.heading_deg,
            speed_kn: w.speed_kn,
            source: FixSource::Wire,
            seq: 0,
        }
    }
}

impl Fix {
    pub fn epoch_secs(&self) -> i64 {
        self.ts
            .parse::<DateTime<chrono::Utc>>()
            .map(|dt| dt.timestamp())
            .unwrap_or(0)
    }

    /// Parse the wire shape used by fixtures and (later) the HTTP backend.
    pub fn from_wire_json(s: &str) -> Result<Self, String> {
        serde_json::from_str::<WireFix>(s)
            .map(Fix::from)
            .map_err(|e| e.to_string())
    }
}

/// How much history a track keeps.
#[derive(Debug, Clone, Copy)]
pub struct TrailBound {
    /// Maximum fixes retained per ship.
    pub max_fixes: usize,
}

impl Default for TrailBound {
    fn default() -> Self {
        Self { max_fixes: 60 }
    }
}

/// Jumps smaller than this hold position (GPS jitter guard).
pub const JITTER_GUARD_M: f64 = 8.0;

/// Followed-ship displacement from viewport center that earns a tracking
/// re-render. Hysteresis against refetch storms while chasing.
pub const TRACK_MIN_MOVE_M: f64 = 150.0;

/// Whether follow-tracking should request a new frame for a ship at
/// `ship` while the viewport sits on `center`.
pub fn should_track(center: GeoPosition, ship: GeoPosition) -> bool {
    center.distance_m(&ship) >= TRACK_MIN_MOVE_M
}

/// Missed polls before a ship is flagged stale.
pub const STALE_AFTER_MISSED: u32 = 3;

#[derive(Debug)]
struct ShipState {
    latest: Fix,
    previous: Option<Fix>,
    track: VecDeque<Fix>,
    missed: u32,
    stale: bool,
}

/// Read view of one ship for renderers / roster UI.
#[derive(Debug, Clone, PartialEq)]
pub struct ShipView {
    pub ship_id: String,
    pub latest: Fix,
    pub stale: bool,
    pub trail: Vec<GeoPosition>,
    pub source: FixSource,
}

/// All tracked ships, advanced one backend poll at a time.
#[derive(Debug, Default)]
pub struct Registry {
    ships: HashMap<String, ShipState>,
    bound: TrailBound,
    next_seq: u64,
}

impl Registry {
    pub fn new(bound: TrailBound) -> Self {
        Self { ships: HashMap::new(), bound, next_seq: 0 }
    }

    /// Ingest one poll round. Unknown ids appear as new pending ships;
    /// ships with no fix this round accumulate `missed` and go stale at 3.
    /// Returns the accepted fixes as (ship, seq) pairs for ingest acks
    /// (Log grill, #20); dropped out-of-order fixes consume seqs silently.
    pub fn poll(&mut self, fixes: Vec<Fix>) -> Vec<(String, u64)> {
        let mut seen = std::collections::HashSet::new();
        let mut acked = Vec::with_capacity(fixes.len());
        for mut fix in fixes {
            fix.seq = self.next_seq;
            self.next_seq += 1;
            let pair = (fix.ship_id.clone(), fix.seq);
            seen.insert(fix.ship_id.clone());
            match self.ships.get_mut(&fix.ship_id) {
                Some(s) => {
                    if fix.ts <= s.latest.ts {
                        continue; // out-of-order or duplicate: drop
                    }
                    s.previous = Some(std::mem::replace(&mut s.latest, fix.clone()));
                    s.track.push_back(fix);
                    while s.track.len() > self.bound.max_fixes {
                        s.track.pop_front();
                    }
                    s.missed = 0;
                    s.stale = false;
                    acked.push(pair);
                }
                None => {
                    let mut track = VecDeque::new();
                    track.push_back(fix.clone());
                    self.ships.insert(
                        fix.ship_id.clone(),
                        ShipState { latest: fix, previous: None, track, missed: 0, stale: false },
                    );
                    acked.push(pair);
                }
            }
        }
        for (id, s) in self.ships.iter_mut() {
            if !seen.contains(id) {
                s.missed += 1;
                if s.missed >= STALE_AFTER_MISSED {
                    s.stale = true;
                }
            }
        }
        acked
    }

    pub fn ships(&self) -> Vec<ShipView> {
        let mut out: Vec<ShipView> = self
            .ships
            .values()
            .map(|s| ShipView {
                ship_id: s.latest.ship_id.clone(),
                latest: s.latest.clone(),
                stale: s.stale,
                trail: s.track.iter().map(|f| f.position).collect(),
                source: s.latest.source,
            })
            .collect();
        out.sort_by(|a, b| a.ship_id.cmp(&b.ship_id));
        out
    }

    /// Where to draw the marker at wall-clock `now_epoch`: lerp
    /// previous -> latest, holding on sub-guard jumps. No prediction
    /// past the latest fix.
    pub fn displayed_position(&self, ship_id: &str, now_epoch: i64) -> Option<GeoPosition> {
        let s = self.ships.get(ship_id)?;
        let prev = s.previous.as_ref().unwrap_or(&s.latest);
        let t0 = prev.epoch_secs();
        let t1 = s.latest.epoch_secs();
        if t1 <= t0 {
            return Some(s.latest.position);
        }
        let frac = (now_epoch - t0) as f64 / (t1 - t0) as f64;
        self.blend(ship_id, frac)
    }

    /// Blend previous -> latest by explicit fraction in [0, 1] (clamped).
    /// Wall-clock driven: the UI maps elapsed-since-poll onto the poll
    /// interval, so markers glide between fixes instead of jumping.
    /// Holds on sub-guard jumps; unknown ships yield None.
    pub fn blend(&self, ship_id: &str, frac: f64) -> Option<GeoPosition> {
        let s = self.ships.get(ship_id)?;
        let prev = s.previous.as_ref().unwrap_or(&s.latest);
        // Sim fixes are noiseless by construction: never hold them.
        if s.latest.source == FixSource::Wire
            && prev.position.distance_m(&s.latest.position) < JITTER_GUARD_M
        {
            return Some(prev.position);
        }
        Some(prev.position.lerp(&s.latest.position, frac))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(id: &str, lat: f64, lon: f64, ts: &str) -> Fix {
        Fix {
            ship_id: id.into(),
            position: GeoPosition { latitude: lat, longitude: lon },
            ts: ts.into(),
            heading_deg: None,
            speed_kn: None,
            source: FixSource::Wire,
            seq: 0,
        }
    }

    #[test]
    fn out_of_order_and_duplicates_are_dropped() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![fix("a", 53.5, 9.9, "2026-09-12T00:00:02Z")]);
        r.poll(vec![fix("a", 53.6, 9.9, "2026-09-12T00:00:00Z")]); // older
        r.poll(vec![fix("a", 53.6, 9.9, "2026-09-12T00:00:02Z")]); // duplicate ts
        let ships = r.ships();
        assert_eq!(ships.len(), 1);
        assert_eq!(ships[0].latest.position.latitude, 53.5);
        assert_eq!(ships[0].trail.len(), 1);
    }

    #[test]
    fn ingest_stamps_total_order_seq() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![
            fix("a", 53.5, 9.9, "2026-09-12T00:00:02Z"),
            fix("b", 53.5, 9.9, "2026-09-12T00:00:02Z"),
        ]);
        r.poll(vec![fix("a", 53.6, 9.9, "2026-09-12T00:00:04Z")]);
        let ships = r.ships();
        let seq = |id: &str| ships.iter().find(|s| s.ship_id == id).unwrap().latest.seq;
        assert_eq!(seq("a"), 2);
        assert_eq!(seq("b"), 1);
    }

    #[test]
    fn unknown_ids_appear_and_stale_after_three_misses() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![fix("a", 53.5, 9.9, "2026-09-12T00:00:00Z")]);
        assert!(!r.ships()[0].stale);
        r.poll(vec![]);
        r.poll(vec![]);
        assert!(!r.ships()[0].stale); // 2 misses: not yet
        r.poll(vec![]);
        assert!(r.ships()[0].stale); // 3rd miss: stale, marker kept
        assert_eq!(r.ships().len(), 1);
    }

    #[test]
    fn track_evicts_oldest_beyond_bound() {
        let mut r = Registry::new(TrailBound { max_fixes: 3 });
        for i in 0..5 {
            r.poll(vec![fix(
                "a",
                53.5 + i as f64 * 0.01,
                9.9,
                &format!("2026-09-12T00:00:{:02}Z", i * 2),
            )]);
        }
        assert_eq!(r.ships()[0].trail.len(), 3);
    }

    #[test]
    fn displayed_position_interpolates_and_holds_jitter() {
        let mut r = Registry::new(TrailBound::default());
        // Big jump: interpolates halfway.
        r.poll(vec![fix("big", 53.0, 9.0, "2026-09-12T00:00:00Z")]);
        r.poll(vec![fix("big", 54.0, 10.0, "2026-09-12T00:00:10Z")]);
        let mid = r.displayed_position("big", 1789171205).unwrap();
        assert!((mid.latitude - 53.5).abs() < 1e-9);
        // Sub-guard jump: holds previous.
        r.poll(vec![fix("jit", 53.5, 9.9, "2026-09-12T00:00:00Z")]);
        r.poll(vec![fix("jit", 53.50001, 9.90001, "2026-09-12T00:00:10Z")]);
        let held = r.displayed_position("jit", 1789171205).unwrap();
        assert_eq!(held.latitude, 53.5);
    }

    #[test]
    fn blend_interpolates_by_fraction_holds_jitter_and_misses_unknown() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![fix("big", 53.0, 9.0, "2026-09-12T00:00:00Z")]);
        r.poll(vec![fix("big", 54.0, 10.0, "2026-09-12T00:00:10Z")]);
        assert_eq!(
            r.blend("big", 0.0).unwrap(),
            GeoPosition { latitude: 53.0, longitude: 9.0 }
        );
        let mid = r.blend("big", 0.5).unwrap();
        assert!((mid.latitude - 53.5).abs() < 1e-9);
        assert_eq!(
            r.blend("big", 1.0).unwrap(),
            GeoPosition { latitude: 54.0, longitude: 10.0 }
        );
        assert_eq!(r.blend("ghost", 0.5), None);
        // Single fix (no previous): holds latest at any fraction.
        r.poll(vec![fix("solo", 53.5, 9.9, "2026-09-12T00:00:00Z")]);
        assert_eq!(
            r.blend("solo", 0.3).unwrap(),
            GeoPosition { latitude: 53.5, longitude: 9.9 }
        );
    }

    #[test]
    fn tracking_fires_beyond_threshold_holds_inside() {
        let center = GeoPosition { latitude: 53.5413, longitude: 9.9842 };
        let near = GeoPosition { latitude: 53.5414, longitude: 9.9843 };
        let far = GeoPosition { latitude: 53.55, longitude: 10.0 };
        assert!(!should_track(center, near));
        assert!(should_track(center, far));
    }

    #[test]
    fn wire_json_parses_flat_lat_lon() {
        let f = Fix::from_wire_json(
            r#"{"ship_id":"a","lat":53.5,"lon":9.9,"ts":"2026-09-12T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(f.position.latitude, 53.5);
        assert_eq!(f.epoch_secs(), 1789171200);
    }
}
