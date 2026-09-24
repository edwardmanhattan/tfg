//! Standing picture + publication shapes (backend split, step 4).
//!
//! - [`MinosRest::snapshot`]: `GET /live-feed/positions` — announcements
//!   for silent vessels plus seed fixes for the rest.
//! - [`Snapshot`]: that picture, handed to the socket actor.
//! - [`FeedEvent`]: one §5 publication (shared with the actor, hence
//!   `pub(crate)` — still not public API).

use serde::Deserialize;

use crate::geo::track::{Fix, FixSource};

use super::{BackendError, GameFix, unwrap_envelope};

/// Minos standing picture (REST mapping ticket): the initial picture the
/// socket then keeps current. Vessels that never reported carry labels
/// but no position — announced as silent, never zero-filled.
#[derive(Debug, Clone, Deserialize)]
struct FeedPosition {
    id_unit: u64,
    name: String,
    hull_number: Option<String>,
    position: Option<FeedFix>,
}

/// One stored fix as the server holds it.
#[derive(Debug, Clone, Deserialize)]
struct FeedFix {
    latitude: f64,
    longitude: f64,
    speed_kn: Option<f32>,
    course_deg: Option<f32>,
    accuracy_m: Option<f32>,
    recorded_at: String,
    received_at: String,
    /// Server-computed age at response time (M4): retained on the fix
    /// so freshness never depends on client-clock agreement.
    #[serde(default)]
    age_seconds: Option<f64>,
    #[serde(default)]
    backfilled: bool,
}

impl FeedFix {
    fn to_fix(&self, ship_id: String, name: String, hull_number: Option<String>) -> Fix {
        Fix {
            ship_id,
            position: crate::geo::GeoPosition {
                latitude: self.latitude,
                longitude: self.longitude,
            },
            ts: self.recorded_at.clone(),
            received_at: Some(self.received_at.clone()),
            heading_deg: self.course_deg,
            speed_kn: self.speed_kn,
            accuracy_m: self.accuracy_m,
            name: Some(name),
            hull_number,
            backfilled: self.backfilled,
            source: FixSource::Wire,
            // M4: the server-stated age rides the snapshot fix; socket
            // events carry none and age from receipt instead.
            age_secs: self.age_seconds,
            seq: 0,
        }
    }
}

/// Snapshot out of one REST read: silent announcements plus latest fixes.
pub struct Snapshot {
    pub announced: Vec<(String, Option<String>, Option<String>)>,
    pub fixes: Vec<Fix>,
}

/// One game message as a socket event carries it (H11): the event
/// envelope (type/game/occurred) plus the drawable subset of the
/// message payload. The assumed identity is what ordinary
/// participants see; the real author rides only where entitled.
/// Full thread/inbox reads stay HTTP (later slice) — the event is a
/// notification with enough body to draw, not the inbox.
#[derive(Debug, Clone)]
pub struct GameMsg {
    /// `game.message_sent` (broadcast, `game:<id>`) or
    /// `personal.message_sent` (addressed, `personal:<user>`).
    pub event: String,
    pub id: i64,
    pub game_id: i64,
    pub kind: String,
    pub class_code: String,
    pub class_label: String,
    /// Assumed role name, else real author name, else unknown.
    pub sender: String,
    pub broadcast: bool,
    pub content: String,
    pub callsign: String,
    pub created_at: String,
    pub assumed_at: String,
    pub occurred_at: String,
}

/// Parse a message-event publication (§9). None when the bytes are
/// not a message event — the channel may carry future event types
/// this client does not draw yet. Callers log and drop, never crash
/// the feed.
pub(crate) fn parse_message_event(data: &[u8]) -> Option<GameMsg> {
    let v: serde_json::Value = serde_json::from_slice(data).ok()?;
    let event = v["type"].as_str()?;
    if event != "game.message_sent" && event != "personal.message_sent" {
        return None;
    }
    let d = &v["data"];
    let sender = d["sender"]["assumed_role"]["name"]
        .as_str()
        .map(|s| s.to_string())
        .or_else(|| {
            d["sender"]["author"]["name"]
                .as_str()
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown sender".to_string());
    Some(GameMsg {
        event: event.to_string(),
        id: d["id"].as_i64()?,
        game_id: d["id_game"].as_i64()?,
        kind: d["kind"].as_str().unwrap_or("").to_string(),
        class_code: d["classification"]["code"].as_str().unwrap_or("").to_string(),
        class_label: d["classification"]["label"].as_str().unwrap_or("").to_string(),
        sender,
        broadcast: d["broadcast"].as_bool().unwrap_or(false),
        content: d["content"].as_str().unwrap_or("").to_string(),
        callsign: d["callsign"].as_str().unwrap_or("").to_string(),
        created_at: d["created_at"].as_str().unwrap_or("").to_string(),
        assumed_at: d["assumed_at"].as_str().unwrap_or("").to_string(),
        occurred_at: v["occurred_at"].as_str().unwrap_or("").to_string(),
    })
}

/// Minos REST reads (REST mapping ticket): snapshot fetch beside the
/// auth client. Same blocking style; the poll thread calls it, the
/// socket actor re-reads it after every reconnect.
pub struct MinosRest {
    /// Read by the socket actor to rebuild its re-read client after a
    /// reconnect — crate-visible, not public API.
    pub(crate) base_url: String,
    client: reqwest::blocking::Client,
}

impl MinosRest {
    pub fn new(base_url: &str) -> Result<Self, BackendError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }

    /// Standing picture: every vessel, ordered by id. Not paginated by
    /// design (a page of a picture is a wrong picture).
    pub fn snapshot(&self, access_token: &str) -> Result<Snapshot, BackendError> {
        let data = unwrap_envelope(
            self.client
                .get(&format!("{}/live-feed/positions", self.base_url))
                .bearer_auth(access_token)
                .send()?,
        )?;
        let positions: Vec<FeedPosition> = serde_json::from_value(data)?;
        let mut announced = Vec::with_capacity(positions.len());
        let mut fixes = Vec::new();
        for p in positions {
            let id = p.id_unit.to_string();
            announced.push((id.clone(), Some(p.name.clone()), p.hull_number.clone()));
            if let Some(pos) = p.position {
                fixes.push(pos.to_fix(id, p.name, p.hull_number));
            }
        }
        Ok(Snapshot { announced, fixes })
    }
}

/// One live-feed publication (§5): one message per beacon batch, per
/// vessel. Optional numerics are absent-never-zero on the wire.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FeedEvent {
    pub(crate) id_unit: u64,
    pub(crate) name: Option<String>,
    pub(crate) hull_number: Option<String>,
    pub(crate) latitude: f64,
    pub(crate) longitude: f64,
    pub(crate) speed_kn: Option<f32>,
    pub(crate) course_deg: Option<f32>,
    pub(crate) accuracy_m: Option<f32>,
    pub(crate) recorded_at: String,
    pub(crate) received_at: String,
    #[serde(default)]
    pub(crate) backfilled: bool,
}

/// Parse the best-effort order publication. Unknown event types and
/// incomplete GameFix records return `None`; neither is allowed to
/// reconcile an Unknown command result.
pub(crate) fn parse_order_event(bytes: &[u8]) -> Option<GameFix> {
    let envelope: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    if envelope["type"].as_str() != Some("game.order_issued") {
        return None;
    }
    let fix: GameFix = serde_json::from_value(envelope["data"].clone()).ok()?;
    if fix.assumed_time.is_empty()
        || !fix.heading.is_finite()
        || !(0.0..360.0).contains(&fix.heading)
        || !fix.speed.is_finite()
        || fix.speed < 0.0
        || !fix.latitude.is_finite()
        || !(-90.0..=90.0).contains(&fix.latitude)
        || !fix.longitude.is_finite()
        || !(-180.0..=180.0).contains(&fix.longitude)
        || fix.requested_speed.is_some_and(|requested| !requested.is_finite() || requested < 0.0)
        || fix.clamped != fix.requested_speed.is_some()
        || (fix.clamped && fix.requested_speed.is_some_and(|requested| requested <= fix.speed))
    {
        return None;
    }
    Some(fix)
}

impl FeedEvent {
    /// One-line terminal summary for the coordinate-arrival log.
    /// Shape only — no secret material rides these events.
    pub(crate) fn summary(&self) -> String {
        format!(
            "unit={} lat={:.6} lon={:.6} ts={} backfilled={}",
            self.id_unit, self.latitude, self.longitude, self.recorded_at, self.backfilled
        )
    }

    pub(crate) fn to_fix(&self) -> Fix {
        Fix {
            ship_id: self.id_unit.to_string(),
            position: crate::geo::GeoPosition {
                latitude: self.latitude,
                longitude: self.longitude,
            },
            ts: self.recorded_at.clone(),
            received_at: Some(self.received_at.clone()),
            heading_deg: self.course_deg,
            speed_kn: self.speed_kn,
            accuracy_m: self.accuracy_m,
            name: self.name.clone(),
            hull_number: self.hull_number.clone(),
            backfilled: self.backfilled,
            source: FixSource::Wire,
            // Socket events intentionally carry no age: freshness
            // computes from receipt (see data_age_secs).
            age_secs: None,
            seq: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_events_parse_broadcast_and_addressed() {
        // H11: broadcast on game:<id>, addressed on personal:<user> —
        // both draw from the same envelope with the sender resolved.
        let broadcast = parse_message_event(
            br#"{"type":"game.message_sent","id_game":3,"occurred_at":"2026-11-01T02:00:00Z","data":{"id":11,"id_game":3,"kind":"telegram","classification":{"code":"TERBUKA","label":"Terbuka"},"sender":{"assumed_role":{"id":2,"name":"Panglima"}},"recipients":[],"broadcast":true,"content":"Mulai latihan","callsign":"","created_at":"2026-11-01T02:00:00Z","assumed_at":"2026-11-01T02:00:00Z"}}"#,
        )
        .expect("broadcast parses");
        assert_eq!(broadcast.sender, "Panglima", "assumed identity shown");
        assert!(broadcast.broadcast);
        assert_eq!(broadcast.class_code, "TERBUKA");
        let addressed = parse_message_event(
            br#"{"type":"personal.message_sent","id_game":3,"occurred_at":"2026-11-01T02:01:00Z","data":{"id":12,"id_game":3,"kind":"administrative","classification":{"code":"RAHASIA","label":"Rahasia"},"sender":{"author":{"id":6,"username":"budi","name":"Budi Santoso"}},"recipients":[{"id_user":7,"kind":"to","deleted":false}],"broadcast":false,"content":"Rapat jam 3","callsign":"KRI-381","created_at":"2026-11-01T02:01:00Z","assumed_at":"2026-11-01T02:01:00Z"}}"#,
        )
        .expect("addressed parses");
        assert_eq!(addressed.sender, "Budi Santoso", "real author where entitled");
        assert!(!addressed.broadcast);
        assert_eq!(addressed.callsign, "KRI-381");
        // Unknown future event types are ignored, never fatal.
        assert!(parse_message_event(br#"{"type":"game.judged","id_game":3}"#).is_none());
    }
}
