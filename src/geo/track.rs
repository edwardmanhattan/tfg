//! Presentation movement model (see CONTEXT.md): Fix / Ship / Track / Trail.
//!
//! Rules (from the v0 + geo tickets):
//! - Fix instants order fixes (nanos, parsed — never string-compared:
//!   the wire mixes `Z` with numeric offsets); an older-or-equal instant
//!   is dropped.
//! - A ship with no fix for 3 consecutive polls is `stale` (marker kept).
//! - Displayed position lerps previous -> latest by wall-clock fraction,
//!   with a small-jump guard (sub-8 m jumps hold, killing GPS jitter).
//! - Each track is bounded (default: last 60 fixes); the trail renders from it.

use std::collections::{HashMap, VecDeque};

use chrono::DateTime;
use serde::Deserialize;

use super::coordinates::GeoPosition;

/// Provenance of a Fix: backend report, synthetic sim emission, or
/// authoritative Minos game position (C3: computed exercise truth —
/// noiseless like sim, scenario-stamped like nothing else, so it ages
/// by neither wall clock nor jitter guard).
/// Decided in ADR-0003: the jitter guard and displays key off this.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FixSource {
    #[default]
    Wire,
    Sim,
    Game,
}

/// One accepted position report for a ship. `ts` is the recorded time
/// (orders fixes); `received_at` is the receipt time (ages the fix at
/// render). Labels ride each fix so the journal stays faithful without
/// a separate label table.
#[derive(Debug, Clone, PartialEq)]
pub struct Fix {
    /// Identity of the ship. Wire format uses `ship_id`; Minos wire ships
    /// key as decimal strings of their unit id (see `Registry::announce`).
    pub ship_id: String,
    pub position: GeoPosition,
    /// RFC3339 UTC, e.g. `2026-09-12T00:00:02Z`. Orders fixes. Minos
    /// semantics: recorded_at (re-stamped live, kept on backfill).
    pub ts: String,
    /// Minos received_at: when the server accepted the fix. Ages live
    /// fixes at render; absent on sim/replay emissions.
    pub received_at: Option<String>,
    pub heading_deg: Option<f32>,
    pub speed_kn: Option<f32>,
    pub accuracy_m: Option<f32>,
    /// Human labels stamped per fix (Minos name/hull_number).
    pub name: Option<String>,
    pub hull_number: Option<String>,
    /// True when reconstructed from an offline backlog: joins the trail
    /// but never moves the marker (see `Registry::blend`).
    pub backfilled: bool,
    /// Server-stated age in seconds (M4): Minos measures the fix
    /// against its own clock at response time and says so on REST
    /// snapshots. Socket events carry none — the client computes those
    /// from receipt. Present beats computed everywhere it is used:
    /// the server knows both clocks, the client only trusts its own
    /// against server timestamps.
    pub age_secs: Option<f64>,
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
    #[serde(default)]
    accuracy_m: Option<f32>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    hull_number: Option<String>,
    #[serde(default)]
    received_at: Option<String>,
    #[serde(default)]
    backfilled: bool,
}

impl From<WireFix> for Fix {
    fn from(w: WireFix) -> Self {
        Self {
            ship_id: w.ship_id,
            position: GeoPosition { latitude: w.lat, longitude: w.lon },
            ts: w.ts,
            received_at: w.received_at,
            heading_deg: w.heading_deg,
            speed_kn: w.speed_kn,
            accuracy_m: w.accuracy_m,
            name: w.name,
            hull_number: w.hull_number,
            backfilled: w.backfilled,
            source: FixSource::Wire,
            age_secs: None,
            seq: 0,
        }
    }
}

impl Fix {
    pub fn epoch_secs(&self) -> i64 {
        parse_epoch(&self.ts).unwrap_or(0)
    }

    /// Instant ordering for ingest guards, nanos precision. String order
    /// only matches time order within one format, and the wire mixes `Z`
    /// with numeric offsets (`+07:00`) — compare instants, never strings.
    pub fn epoch_nanos(&self) -> i64 {
        self.ts
            .parse::<DateTime<chrono::Utc>>()
            .ok()
            .and_then(|dt| dt.timestamp_nanos_opt())
            .unwrap_or(0)
    }

    /// Data age at render, in seconds (backfilled ticket): live fixes age
    /// from received_at, backfilled ones from recorded_at (`ts`) — the
    /// receipt time of a flush would read age-zero on days-old data.
    /// A server-stated age (M4, REST snapshots) wins over both: it was
    /// measured at response time, so it understates by transit, which
    /// is noted rather than corrected. Falls back to `ts` when receipt
    /// is unknown (sim/replay).
    pub fn data_age_secs(&self, now_epoch: i64) -> Option<i64> {
        if let Some(a) = self.age_secs {
            return Some(a as i64);
        }
        if self.backfilled {
            return parse_epoch(&self.ts).map(|t| now_epoch - t);
        }
        match self.received_at.as_deref().and_then(parse_epoch) {
            Some(t) => Some(now_epoch - t),
            None => parse_epoch(&self.ts).map(|t| now_epoch - t),
        }
    }

    /// Old-data indication (backfilled ticket): data older than the
    /// threshold. The caller decides which sources badge (wire only:
    /// sim clocks are game time, not wall time).
    pub fn is_old_data(&self, now_epoch: i64) -> bool {
        self.data_age_secs(now_epoch).is_some_and(|a| a > OLD_DATA_AFTER_SECS)
    }

    /// Parse the wire shape used by fixtures and (later) the HTTP backend.
    pub fn from_wire_json(s: &str) -> Result<Self, String> {
        serde_json::from_str::<WireFix>(s)
            .map(Fix::from)
            .map_err(|e| e.to_string())
    }
}

/// Parse one RFC3339 timestamp to epoch seconds.
fn parse_epoch(s: &str) -> Option<i64> {
    s.parse::<DateTime<chrono::Utc>>().ok().map(|dt| dt.timestamp())
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

/// Data older than this reads as old data (backfilled ticket): fixed
/// default, clears the 3-minute moored-report trap (moored beacons
/// report every 3 minutes, well under this).
pub const OLD_DATA_AFTER_SECS: i64 = 600;

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
    /// Announced but never tracked (silent vessel): no position exists.
    /// Renderers skip these; the roster lists them without follow.
    pub silent: bool,
    pub trail: Vec<GeoPosition>,
    pub source: FixSource,
}

/// A vessel known from the REST snapshot that has never reported:
/// listed so silence and non-existence stay distinct.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnouncedShip {
    pub ship_id: String,
    pub name: Option<String>,
    pub hull_number: Option<String>,
}

/// All tracked ships, advanced one backend poll at a time.
#[derive(Debug, Default)]
pub struct Registry {
    ships: HashMap<String, ShipState>,
    /// Snapshot-known vessels (REST mapping ticket): ids with labels that
    /// have no fix yet. Never counted as missed/stale; a first fix
    /// promotes the id into `ships` and drops it from here on read.
    known: HashMap<String, (Option<String>, Option<String>)>,
    bound: TrailBound,
    next_seq: u64,
}

impl Registry {
    pub fn new(bound: TrailBound) -> Self {
        Self { ships: HashMap::new(), known: HashMap::new(), bound, next_seq: 0 }
    }

    /// Announce a snapshot-known vessel (REST mapping ticket): listed as
    /// silent until its first fix arrives. Re-announcing refreshes labels.
    pub fn announce(
        &mut self,
        ship_id: String,
        name: Option<String>,
        hull_number: Option<String>,
    ) {
        if !self.ships.contains_key(&ship_id) {
            self.known.insert(ship_id, (name, hull_number));
        }
    }

    /// Silent vessels: announced but never fixed. Sorted by id. Drops ids
    /// that have since reported (promotion is one-way).
    pub fn announced(&self) -> Vec<AnnouncedShip> {
        let mut out: Vec<AnnouncedShip> = self
            .known
            .iter()
            .filter(|(id, _)| !self.ships.contains_key(*id))
            .map(|(id, (name, hull))| AnnouncedShip {
                ship_id: id.clone(),
                name: name.clone(),
                hull_number: hull.clone(),
            })
            .collect();
        out.sort_by(|a, b| a.ship_id.cmp(&b.ship_id));
        out
    }

    /// Ingest one poll round. Unknown ids appear as new pending ships;
    /// ships with no fix this round accumulate `missed` and go stale at 3.
    /// Returns the accepted fixes as (ship, seq) pairs for ingest acks
    /// (Log grill, #20); dropped out-of-order fixes consume seqs silently.
    pub fn poll(&mut self, fixes: Vec<Fix>) -> Vec<(String, u64)> {
        self.poll_inner(fixes, true)
    }

    /// Ingest an authoritative game-position publication without letting
    /// its independent stream affect Wire/Sim miss counters.
    pub fn poll_game(&mut self, fixes: Vec<Fix>) -> Vec<(String, u64)> {
        self.poll_inner(fixes, false)
    }

    fn poll_inner(&mut self, fixes: Vec<Fix>, count_misses: bool) -> Vec<(String, u64)> {
        let mut seen = std::collections::HashSet::new();
        let mut acked = Vec::with_capacity(fixes.len());
        for mut fix in fixes {
            // An unparsable timestamp is not a position instant. Reject
            // it before it can become epoch zero and poison ordering.
            if fix.epoch_nanos() <= 0 {
                continue;
            }
            fix.seq = self.next_seq;
            self.next_seq += 1;
            let pair = (fix.ship_id.clone(), fix.seq);
            seen.insert(fix.ship_id.clone());
            match self.ships.get_mut(&fix.ship_id) {
                Some(s) => {
                    // A connected exercise's authoritative Game source
                    // owns the identity; a later wire measurement must not
                    // overwrite it. Conversely, a Game publication is
                    // authoritative even when its scenario timestamp is
                    // earlier than a wall-clock measurement that happened
                    // to share the unit id.
                    if s.latest.source == FixSource::Game && fix.source != FixSource::Game {
                        continue;
                    }
                    let game_takes_authority = fix.source == FixSource::Game
                        && s.latest.source != FixSource::Game;
                    // Instant order, not string order (mixed `Z`/offsets).
                    if !game_takes_authority
                        && fix.epoch_nanos() <= s.latest.epoch_nanos()
                    {
                        continue; // out-of-order or duplicate: drop
                    }
                    let previous = std::mem::replace(&mut s.latest, fix.clone());
                    s.previous = Some(previous);
                    // A newer fix at the same coordinate may update the
                    // heading, but it must not add another visible Trail dot.
                    let same_position = s
                        .track
                        .back()
                        .is_some_and(|last| last.position == fix.position);
                    if !same_position {
                        s.track.push_back(fix);
                        while s.track.len() > self.bound.max_fixes {
                            s.track.pop_front();
                        }
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
        if count_misses {
            for (id, s) in self.ships.iter_mut() {
                // Game fixes arrive on their own MinOS position stream,
                // not through this poll's empty Wire/Sim rounds. Do not
                // let an unrelated two-second poll mark a live game ship
                // stale.
                if s.latest.source == FixSource::Game {
                    continue;
                }
                if !seen.contains(id) {
                    s.missed += 1;
                    if s.missed >= STALE_AFTER_MISSED {
                        s.stale = true;
                    }
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
                silent: false,
                trail: s.track.iter().map(|f| f.position).collect(),
                source: s.latest.source,
            })
            .collect();
        out.sort_by(|a, b| a.ship_id.cmp(&b.ship_id));
        out
    }

    /// Whether the latest two accepted fixes form a normal display
    /// interval. A reconnect/long gap must snap rather than animate an
    /// unobserved route.
    pub fn has_large_gap(&self, ship_id: &str, max_gap_secs: i64) -> bool {
        let Some(s) = self.ships.get(ship_id) else { return true; };
        let Some(previous) = s.previous.as_ref() else { return false; };
        let gap = s.latest.epoch_secs() - previous.epoch_secs();
        gap <= 0 || gap > max_gap_secs
    }

    /// Remove an operational wire ship after its authoritative snapshot
    /// says it vanished. Sim/Game identities have separate authorities and
    /// must not be reaped by a live-feed picture.
    pub fn remove_ship(&mut self, ship_id: &str) {
        if self
            .ships
            .get(ship_id)
            .is_some_and(|ship| ship.latest.source == FixSource::Wire)
        {
            self.ships.remove(ship_id);
        }
        self.known.remove(ship_id);
    }

    /// Where to draw the marker at wall-clock `now_epoch`: lerp
    /// previous -> latest, holding on sub-guard jumps. A backfilled
    /// latest never moves the marker while a previous live fix exists
    /// (backfilled ticket) — the flush joins the trail, not the position.
    /// No prediction past the latest fix.
    pub fn displayed_position(&self, ship_id: &str, now_epoch: i64) -> Option<GeoPosition> {
        let s = self.ships.get(ship_id)?;
        if s.latest.backfilled && s.previous.is_some() {
            return s.previous.as_ref().map(|p| p.position);
        }
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
    /// Holds on sub-guard jumps; a backfilled latest holds previous while
    /// one exists (no lerp into the past); unknown ships yield None.
    pub fn blend(&self, ship_id: &str, frac: f64) -> Option<GeoPosition> {
        let s = self.ships.get(ship_id)?;
        let prev = s.previous.as_ref().unwrap_or(&s.latest);
        if s.latest.backfilled && s.previous.is_some() {
            return Some(prev.position);
        }
        // Sim and Game fixes are noiseless by construction: never hold
        // them. Wire alone gets the jitter guard.
        if s.latest.source == FixSource::Wire
            && prev.position.distance_m(&s.latest.position) < JITTER_GUARD_M
        {
            return Some(prev.position);
        }
        Some(prev.position.lerp(&s.latest.position, frac))
    }

    /// Heading interpolated across the same poll fraction as position,
    /// along the SHORTEST ARC.
    ///
    /// Naive `a + (b - a) * frac` takes the long way round whenever the
    /// reports straddle north: 350° to 10° is a 20° turn, but the
    /// arithmetic produces a 340° swing, and a rotated image would spin
    /// almost a full circle between two fixes that meant a gentle
    /// starboard nudge. Normalising the delta into (-180, 180] first
    /// makes the turn direction the shorter one.
    ///
    /// `None` when the latest fix has no heading, or when there is
    /// nothing to interpolate from — the caller draws neutral rather
    /// than inventing a course. A fix that HAS a heading but whose
    /// predecessor does not uses the latest one outright: there is no
    /// arc to follow, but the known course is still known.
    pub fn blend_heading(&self, ship_id: &str, frac: f64) -> Option<f32> {
        let s = self.ships.get(ship_id)?;
        let to = s.latest.heading_deg?;
        let Some(from) = s.previous.as_ref().and_then(|p| p.heading_deg) else {
            return Some(to);
        };
        let frac = frac.clamp(0.0, 1.0) as f32;
        // Wrapping the delta into (-180, 180] is the whole trick: it
        // makes the interpolation take whichever direction is shorter.
        let mut delta = (to - from).rem_euclid(360.0);
        if delta > 180.0 {
            delta -= 360.0;
        }
        Some((from + delta * frac).rem_euclid(360.0))
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
            received_at: None,
            heading_deg: None,
            speed_kn: None,
            accuracy_m: None,
            name: None,
            hull_number: None,
            backfilled: false,
            source: FixSource::Wire,
            age_secs: None,
            seq: 0,
        }
    }

    #[test]
    fn ordered_rounds_keep_every_point_on_the_trail() {
        // M5: intermediate publications must survive to the Track — one
        // round carrying three increasing fixes keeps all three.
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![
            fix("a", 53.5, 9.9, "2026-09-12T00:00:00Z"),
            fix("a", 53.6, 9.9, "2026-09-12T00:00:01Z"),
            fix("a", 53.7, 9.9, "2026-09-12T00:00:02Z"),
        ]);
        let ships = r.ships();
        assert_eq!(ships.len(), 1);
        assert_eq!(ships[0].trail.len(), 3, "no intermediate dropped");
        assert!((ships[0].latest.position.latitude - 53.7).abs() < 1e-9);
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
    fn game_poll_does_not_increment_wire_misses() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![fix("wire", 53.5, 9.9, "2026-09-12T00:00:00Z")]);
        for _ in 0..4 {
            r.poll_game(vec![]);
        }
        assert!(!r.ships()[0].stale, "independent Game stream must not stale Wire");
    }

    #[test]
    fn duplicate_coordinates_do_not_duplicate_trail() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![fix("a", 53.5, 9.9, "2026-09-12T00:00:00Z")]);
        r.poll(vec![fix("a", 53.5, 9.9, "2026-09-12T00:00:02Z")]);
        let ships = r.ships();
        assert_eq!(ships[0].trail.len(), 1);
        assert_eq!(ships[0].latest.ts, "2026-09-12T00:00:02Z");
    }

    #[test]
    fn game_authority_is_not_overwritten_by_wire_timestamp() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![fix("a", 53.5, 9.9, "2026-09-12T00:00:10Z")]);
        let mut game = fix("a", -6.0, 106.0, "2026-09-12T00:00:00Z");
        game.source = FixSource::Game;
        r.poll_game(vec![game]);
        let mut wire = fix("a", 54.0, 10.0, "2026-09-12T00:00:20Z");
        wire.source = FixSource::Wire;
        r.poll(vec![wire]);
        assert_eq!(r.ships()[0].latest.source, FixSource::Game);
    }

    #[test]
    fn invalid_timestamps_are_rejected() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![fix("bad", 53.5, 9.9, "not-a-time")]);
        assert!(r.ships().is_empty());
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

    fn headed(id: &str, lat: f64, lon: f64, ts: &str, heading: Option<f32>) -> Fix {
        let mut f = fix(id, lat, lon, ts);
        f.heading_deg = heading;
        f
    }

    /// 350° to 10° is a 20° starboard nudge, and the blend must take
    /// that short way. The naive `a + (b-a)*frac` would swing 340°
    /// the other way and spin the image almost a full circle between
    /// two fixes that meant a gentle turn.
    #[test]
    fn heading_blend_takes_the_shortest_arc() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![headed("h", 0.0, 0.0, "2026-09-12T00:00:00Z", Some(350.0))]);
        r.poll(vec![headed("h", 0.0, 0.1, "2026-09-12T00:00:10Z", Some(10.0))]);
        let mid = r.blend_heading("h", 0.5).expect("heading blends");
        // Halfway between 350 and 10 the short way is 0/360, NOT 180.
        assert!(mid < 1.0 || mid > 359.0, "mid {mid} should be near north");
        // Full fraction lands exactly on the reported course.
        assert!((r.blend_heading("h", 1.0).expect("end") - 10.0).abs() < 1e-3);
        // Quarter and three-quarter walk the short arc in order.
        let q = r.blend_heading("h", 0.25).expect("q");
        let tq = r.blend_heading("h", 0.75).expect("tq");
        assert!(q > 350.0, "quarter {q} still west of north");
        assert!(tq < 10.0, "three-quarter {tq} past north");
    }

    /// The long way round is only ever chosen when it genuinely is
    /// shorter: 0 to 270 is 90° anticlockwise, not 270° clockwise.
    #[test]
    fn heading_blend_picks_shorter_direction_both_ways() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![headed("h", 0.0, 0.0, "2026-09-12T00:00:00Z", Some(0.0))]);
        r.poll(vec![headed("h", 0.0, 0.1, "2026-09-12T00:00:10Z", Some(270.0))]);
        let mid = r.blend_heading("h", 0.5).expect("blend");
        // Half of a 90° anticlockwise turn: 315, not 135.
        assert!((mid - 315.0).abs() < 1e-3, "mid {mid}, wanted 315");
    }

    /// No course means no rotation, never a zero. A fix with a
    /// heading but no headed predecessor uses its own course; an
    /// unheaded latest stays unheaded.
    #[test]
    fn heading_blend_preserves_unknown() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![headed("h", 0.0, 0.0, "2026-09-12T00:00:00Z", Some(90.0))]);
        r.poll(vec![headed("h", 0.0, 0.1, "2026-09-12T00:00:10Z", None)]);
        assert_eq!(r.blend_heading("h", 0.5), None, "latest has no course");

        // A headed latest over an unheaded predecessor still knows its
        // own course: there is no arc, but the fact is real.
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![headed("h", 0.0, 0.0, "2026-09-12T00:00:00Z", None)]);
        r.poll(vec![headed("h", 0.0, 0.1, "2026-09-12T00:00:10Z", Some(45.0))]);
        assert_eq!(r.blend_heading("h", 0.5), Some(45.0));

        // A ship nobody has heard of has no heading, and that is not
        // an error.
        assert_eq!(r.blend_heading("ghost", 0.5), None);
    }

    /// Fractions outside 0..1 clamp rather than extrapolating a
    /// course past what was reported.
    #[test]
    fn heading_blend_clamps_fraction() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![headed("h", 0.0, 0.0, "2026-09-12T00:00:00Z", Some(10.0))]);
        r.poll(vec![headed("h", 0.0, 0.1, "2026-09-12T00:00:10Z", Some(50.0))]);
        assert!((r.blend_heading("h", -5.0).expect("lo") - 10.0).abs() < 1e-3);
        assert!((r.blend_heading("h", 9.0).expect("hi") - 50.0).abs() < 1e-3);
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
    fn announce_lists_silent_until_first_fix() {
        let mut r = Registry::new(TrailBound::default());
        r.announce("13".into(), Some("KRI Ahmad Yani".into()), Some("KRI-AH-YN".into()));
        assert!(r.ships().is_empty(), "silent ships are not tracked");
        let silent = r.announced();
        assert_eq!(silent.len(), 1);
        assert_eq!(silent[0].name.as_deref(), Some("KRI Ahmad Yani"));
        // Empty rounds never stale the announced id (it was never seen).
        r.poll(vec![]);
        r.poll(vec![]);
        r.poll(vec![]);
        assert_eq!(r.announced().len(), 1);
        // First fix promotes: tracked once, announced never again.
        r.poll(vec![fix("13", -6.0, 106.0, "2026-09-12T00:00:00Z")]);
        assert_eq!(r.ships().len(), 1);
        assert!(!r.ships()[0].silent);
        assert!(r.announced().is_empty());
    }

    #[test]
    fn backfilled_latest_holds_marker_but_joins_trail() {
        let mut r = Registry::new(TrailBound::default());
        r.poll(vec![fix("a", 53.5, 9.9, "2026-09-12T00:00:00Z")]);
        let mut flush = fix("a", 54.5, 10.9, "2026-09-12T00:00:10Z");
        flush.backfilled = true;
        flush.received_at = Some("2026-09-12T00:05:00Z".into());
        r.poll(vec![flush]);
        // Marker holds previous live position at every fraction.
        assert_eq!(
            r.blend("a", 1.0).unwrap(),
            GeoPosition { latitude: 53.5, longitude: 9.9 }
        );
        assert_eq!(
            r.displayed_position("a", 1789171210).unwrap(),
            GeoPosition { latitude: 53.5, longitude: 9.9 }
        );
        // The flush still joins the trail.
        assert_eq!(r.ships()[0].trail.len(), 2);
    }

    #[test]
    fn data_age_uses_received_for_live_and_recorded_for_backfill() {
        let now = 1789171800; // 2026-09-12T00:10:00Z
        let mut live = fix("a", 53.5, 9.9, "2026-09-12T00:09:00Z");
        live.received_at = Some("2026-09-12T00:09:05Z".into());
        assert_eq!(live.data_age_secs(now), Some(55));
        assert!(!live.is_old_data(now));
        let mut old = fix("b", 53.5, 9.9, "2026-09-12T00:09:00Z");
        old.received_at = Some("2026-09-12T00:09:05Z".into());
        assert!(old.is_old_data(now + 3600));
        // Backfill ages from recorded_at even with a fresh receipt.
        let mut flush = fix("c", 53.5, 9.9, "2026-09-10T00:00:00Z");
        flush.backfilled = true;
        flush.received_at = Some("2026-09-12T00:10:00Z".into());
        assert!(flush.data_age_secs(now).unwrap() > OLD_DATA_AFTER_SECS);
        assert!(flush.is_old_data(now));
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
