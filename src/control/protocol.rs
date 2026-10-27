//! The versioned wire contract between the desktop host and remote clients.
//!
//! Everything here is plain data: no GPUI, no Tokio, no Minos. A command is a
//! semantic intent ("move the camera a quarter viewport west"), never a raw
//! input event, and a camera is a place on the earth, never a pixel.
//!
//! The contract is documented for operators in `docs/control-api-v1.md` and
//! generated shape-for-shape into `schemas/control-v1.json`. When a field
//! moves, all three move together.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The URL version: the routes live under `/{API_VERSION}/`.
pub const API_VERSION: &str = "v1";

/// The envelope version, carried in bodies and in the WebSocket snapshot.
///
/// The route version and the envelope version are separate on purpose: a
/// future v2 can serve `v1` commands on the old routes while the snapshot
/// says which shape its events have.
pub const PROTOCOL_VERSION: u64 = 1;

/// The one view the desktop renders today. Views are named rather than
/// implied so a second display can be added without a protocol break.
pub const MAIN_VIEW: &str = "main";

/// A geographic point. Latitude is clamped by the map, longitude wraps.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
}

/// The camera, as a place and a scale, plus the viewport it is expressed in.
///
/// The viewport is part of the camera because a fraction-of-viewport nudge is
/// only meaningful against a size: the desktop reports its frame size with
/// every camera it sends, so a remote nudge is resolved by the desktop's own
/// arithmetic instead of by a guess made on the server.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraSnapshot {
    pub center: GeoPoint,
    pub zoom: f64,
    pub width: u32,
    pub height: u32,
}

impl CameraSnapshot {
    /// The desktop runtime already carries its camera as a tuple shared with
    /// the map thread. Converting here keeps that tuple out of the protocol.
    pub fn from_tuple(value: (f64, f64, f64, u32, u32)) -> Self {
        Self {
            center: GeoPoint {
                latitude: value.0,
                longitude: value.1,
            },
            zoom: value.2,
            width: value.3,
            height: value.4,
        }
    }

    /// The reverse, for the host that still keeps the tuple.
    pub fn to_tuple(self) -> (f64, f64, f64, u32, u32) {
        (
            self.center.latitude,
            self.center.longitude,
            self.zoom,
            self.width,
            self.height,
        )
    }
}

/// A named view and the camera it is looking through.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    pub id: String,
    pub camera: CameraSnapshot,
}

/// Commands accepted by the control plane.
///
/// The names are the wire names. `camera.nudge` carries viewport FRACTIONS:
/// positive `dx_fraction` moves the camera west, positive `dy_fraction` moves
/// it north — the same sense as dragging the map, so an operator who has used
/// the console already knows which way is which.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum Command {
    #[serde(rename = "camera.set")]
    CameraSet { center: GeoPoint, zoom: f64 },
    #[serde(rename = "camera.nudge")]
    CameraNudge { dx_fraction: f64, dy_fraction: f64 },
    #[serde(rename = "camera.zoom")]
    CameraZoom { delta: f64 },
    /// The viewport the display is rendering at, in device pixels.
    ///
    /// This is a *display's* fact rather than a view's: it is sent by a client
    /// that renders (the desktop in client mode) so that a fraction-of-viewport
    /// nudge from a phone resolves against the right size. An embedded desktop
    /// reports its size with its camera and never needs this.
    #[serde(rename = "view.report")]
    ViewReport { width: u32, height: u32 },
}

/// How far a single command may move the camera. Deliberately generous: a
/// remote control may want a whole screen, but no command may teleport the
/// map to another hemisphere or hand the desktop a NaN.
const MAX_FRACTION: f64 = 2.0;
const MAX_ZOOM_DELTA: f64 = 10.0;
/// An 8K display on each side, per axis. Anything larger is a client bug.
const MAX_VIEWPORT: u32 = 8192;

impl Command {
    /// Validate external data before it enters the desktop's command channel.
    ///
    /// This is the protocol boundary (type-system rule: parse, don't trust).
    /// The desktop trusts what arrives here because this is where it was
    /// checked.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::CameraSet { center, zoom } => {
                if !center.latitude.is_finite() || !(-90.0..=90.0).contains(&center.latitude) {
                    return Err("latitude must be finite and between -90 and 90".to_string());
                }
                if !center.longitude.is_finite() || !(-180.0..=180.0).contains(&center.longitude) {
                    return Err("longitude must be finite and between -180 and 180".to_string());
                }
                if !zoom.is_finite() {
                    return Err("zoom must be finite".to_string());
                }
                if !(-30.0..=30.0).contains(zoom) {
                    return Err("zoom must be between -30 and 30".to_string());
                }
            }
            Self::CameraNudge {
                dx_fraction,
                dy_fraction,
            } => {
                if !dx_fraction.is_finite() || !dy_fraction.is_finite() {
                    return Err("camera nudge values must be finite".to_string());
                }
                if dx_fraction.abs() > MAX_FRACTION || dy_fraction.abs() > MAX_FRACTION {
                    return Err(format!(
                        "camera nudge values must be between -{MAX_FRACTION} and {MAX_FRACTION}"
                    ));
                }
            }
            Self::CameraZoom { delta } => {
                if !delta.is_finite() || delta.abs() > MAX_ZOOM_DELTA {
                    return Err(format!(
                        "zoom delta must be finite and between -{MAX_ZOOM_DELTA} and {MAX_ZOOM_DELTA}"
                    ));
                }
            }
            Self::ViewReport { width, height } => {
                // A viewport is a display's own size. Zero is the state before
                // the first frame and is not a report; something enormous is a
                // client that has confused pixels with something else.
                if *width == 0 || *height == 0 {
                    return Err("a viewport must have a width and a height".to_string());
                }
                if *width > MAX_VIEWPORT || *height > MAX_VIEWPORT {
                    return Err(format!(
                        "a viewport may be at most {MAX_VIEWPORT} pixels on a side"
                    ));
                }
            }
        }
        Ok(())
    }
}

/// The HTTP body of `POST /v1/commands`.
///
/// `command` is flattened, so the body is one object: the envelope fields sit
/// beside `type` and `payload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRequest {
    /// Envelope version. Absent means v1; a future version is refused rather
    /// than misread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u64>,
    /// Who is asking. Used for attribution in events, not for authorization:
    /// the bearer token is what authorizes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Unique per command. Retrying with the same id is safe: the host
    /// remembers recent ids and answers the retry without acting twice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    #[serde(flatten)]
    pub command: Command,
}

impl CommandRequest {
    /// The client's name for itself, bounded and printable.
    pub fn client_id(&self) -> String {
        normalise_client_id(self.client_id.as_deref())
    }
}

/// Bound a client-supplied name so a log line cannot be forged across lines
/// and a seen-id table cannot be filled with megabytes.
pub fn normalise_client_id(raw: Option<&str>) -> String {
    let trimmed = raw.map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return "remote".to_string();
    }
    let cleaned: String = trimmed
        .chars()
        .filter(|ch| !ch.is_control())
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "remote".to_string()
    } else {
        cleaned
    }
}

/// A command that passed validation and entered the desktop's channel.
#[derive(Debug, Clone, PartialEq)]
pub struct AcceptedCommand {
    pub client_id: String,
    pub command_id: String,
    pub command: Command,
}

impl AcceptedCommand {
    /// The attribution an event carries when this command moves the camera.
    pub fn source(&self) -> CameraSource {
        CameraSource {
            client_id: self.client_id.clone(),
            command_id: Some(self.command_id.clone()),
        }
    }
}

/// Who moved the camera: a client, and the command it sent when there was one.
///
/// A local gesture on the desktop has no command id — it is the operator's
/// hand, and the event says so with `"source": "desktop"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraSource {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
}

impl CameraSource {
    /// The desktop's own gestures.
    pub fn desktop() -> Self {
        Self {
            client_id: "desktop".to_string(),
            command_id: None,
        }
    }
}

/// Everything the server pushes down a WebSocket, and everything `/v1/state`
/// answers with.
///
/// The tag is `type`, so a client reads one discriminant and one shape per
/// arm. `revision` is monotonic over shared-state changes, not over events:
/// a command acknowledgement repeats the revision it was accepted at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Event {
    /// The whole shared state. Sent on connect, and again after any gap the
    /// client could not be handed (a lagged subscriber resynchronises rather
    /// than silently missing a camera move).
    #[serde(rename = "snapshot")]
    Snapshot {
        protocol_version: u64,
        revision: u64,
        views: BTreeMap<String, ViewState>,
    },
    /// A camera that moved. `revision` is the revision of the new state.
    #[serde(rename = "camera.changed")]
    CameraChanged {
        revision: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command_id: Option<String>,
        view: ViewState,
    },
    /// The reader's command reached the desktop's channel. The camera event
    /// that follows is what proves it was applied.
    #[serde(rename = "command.accepted")]
    CommandAccepted { command_id: String, revision: u64 },
    /// The reader's command was refused before it reached the desktop.
    #[serde(rename = "command.rejected")]
    CommandRejected {
        command_id: String,
        code: String,
        message: String,
    },
}

/// Error codes a client can branch on. The prose beside them is for a human.
pub mod error_code {
    /// No bearer token, or the wrong one.
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// The body was not JSON, or not this shape.
    pub const INVALID_JSON: &str = "invalid_json";
    /// The command parsed but its values are not usable.
    pub const INVALID_COMMAND: &str = "invalid_command";
    /// The envelope version is newer than this server speaks.
    pub const UNSUPPORTED_PROTOCOL: &str = "unsupported_protocol";
    /// The command body was larger than the server reads.
    pub const BODY_TOO_LARGE: &str = "body_too_large";
    /// The route does not exist.
    pub const NOT_FOUND: &str = "not_found";
    /// The desktop's command channel is gone (the window closed).
    pub const HOST_GONE: &str = "host_gone";
    /// Too many WebSocket clients already.
    pub const TOO_MANY_CLIENTS: &str = "too_many_clients";
    /// The request was fine; the server could not do it.
    pub const SERVER_ERROR: &str = "server_error";
    /// The credential is real but may not do this. A watching client cannot
    /// steer, and a paired device cannot pair another.
    pub const FORBIDDEN: &str = "forbidden";
    /// The pairing code was wrong. The window is still open.
    pub const INVALID_PAIRING_CODE: &str = "invalid_pairing_code";
    /// No pairing window is open on this desktop.
    pub const NO_PAIRING_WINDOW: &str = "no_pairing_window";
    /// The pairing code lived out its two minutes.
    pub const PAIRING_EXPIRED: &str = "pairing_expired";
    /// Five wrong guesses closed the window.
    pub const TOO_MANY_ATTEMPTS: &str = "too_many_attempts";
    /// No device has that id.
    pub const UNKNOWN_DEVICE: &str = "unknown_device";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(body: &str) -> CommandRequest {
        serde_json::from_str(body).expect("command parses")
    }

    #[test]
    fn command_request_uses_the_wire_type_tag() {
        let request = parse(r#"{"command_id":"c-1","type":"camera.zoom","payload":{"delta":1.0}}"#);
        assert_eq!(request.command_id.as_deref(), Some("c-1"));
        assert_eq!(request.command, Command::CameraZoom { delta: 1.0 });
        assert_eq!(request.client_id(), "remote");
    }

    #[test]
    fn command_request_carries_its_envelope_fields() {
        let request = parse(
            r#"{"protocol_version":1,"client_id":"phone","command_id":"c-9",
                "type":"camera.nudge","payload":{"dx_fraction":0.25,"dy_fraction":-0.5}}"#,
        );
        assert_eq!(request.protocol_version, Some(1));
        assert_eq!(request.client_id(), "phone");
        assert_eq!(
            request.command,
            Command::CameraNudge {
                dx_fraction: 0.25,
                dy_fraction: -0.5
            }
        );
    }

    #[test]
    fn a_newer_envelope_version_is_readable_and_refusable() {
        let request =
            parse(r#"{"protocol_version":2,"type":"camera.zoom","payload":{"delta":0.5}}"#);
        assert_eq!(request.protocol_version, Some(2));
    }

    #[test]
    fn an_unknown_command_type_does_not_parse() {
        let error =
            serde_json::from_str::<CommandRequest>(r#"{"type":"camera.teleport","payload":{}}"#);
        assert!(error.is_err());
    }

    #[test]
    fn a_known_type_with_a_missing_field_does_not_parse() {
        let error = serde_json::from_str::<CommandRequest>(r#"{"type":"camera.set","payload":{}}"#);
        assert!(error.is_err());
    }

    #[test]
    fn command_validation_rejects_non_finite_values() {
        assert!(Command::CameraZoom { delta: f64::NAN }.validate().is_err());
        assert!(
            Command::CameraSet {
                center: GeoPoint {
                    latitude: f64::INFINITY,
                    longitude: 0.0
                },
                zoom: 1.0,
            }
            .validate()
            .is_err()
        );
        assert!(
            Command::CameraNudge {
                dx_fraction: f64::NAN,
                dy_fraction: 0.0
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn command_validation_rejects_impossible_places_and_scale() {
        let equator = GeoPoint {
            latitude: 0.0,
            longitude: 0.0,
        };
        assert!(
            Command::CameraSet {
                center: GeoPoint {
                    latitude: 91.0,
                    ..equator
                },
                zoom: 4.0,
            }
            .validate()
            .is_err()
        );
        assert!(
            Command::CameraSet {
                center: GeoPoint {
                    longitude: 180.1,
                    ..equator
                },
                zoom: 4.0,
            }
            .validate()
            .is_err()
        );
        assert!(
            Command::CameraSet {
                center: equator,
                zoom: 40.0
            }
            .validate()
            .is_err()
        );
        assert!(
            Command::CameraSet {
                center: equator,
                zoom: 4.0
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn command_validation_bounds_a_single_gesture() {
        assert!(
            Command::CameraNudge {
                dx_fraction: 2.5,
                dy_fraction: 0.0
            }
            .validate()
            .is_err()
        );
        assert!(
            Command::CameraNudge {
                dx_fraction: 0.25,
                dy_fraction: -0.5
            }
            .validate()
            .is_ok()
        );
        assert!(Command::CameraZoom { delta: 11.0 }.validate().is_err());
        assert!(Command::CameraZoom { delta: -1.0 }.validate().is_ok());
    }

    #[test]
    fn client_names_are_bounded_and_printable() {
        assert_eq!(normalise_client_id(None), "remote");
        assert_eq!(normalise_client_id(Some("   ")), "remote");
        assert_eq!(normalise_client_id(Some("phone\nbreak")), "phonebreak");
        assert_eq!(normalise_client_id(Some(&"x".repeat(100))).len(), 64);
    }

    #[test]
    fn events_are_tagged_by_name_on_the_wire() {
        let event = Event::CommandAccepted {
            command_id: "c-1".to_string(),
            revision: 7,
        };
        let value = serde_json::to_value(&event).expect("serializes");
        assert_eq!(value["type"], "command.accepted");
        assert_eq!(value["revision"], 7);

        let event = Event::CameraChanged {
            revision: 8,
            source: Some("phone".to_string()),
            command_id: Some("c-2".to_string()),
            view: ViewState {
                id: MAIN_VIEW.to_string(),
                camera: CameraSnapshot::from_tuple((-6.2, 106.8, 11.5, 800, 600)),
            },
        };
        let value = serde_json::to_value(&event).expect("serializes");
        assert_eq!(value["type"], "camera.changed");
        assert_eq!(value["view"]["id"], "main");
        assert_eq!(value["view"]["camera"]["center"]["latitude"], -6.2);
    }

    #[test]
    fn a_snapshot_survives_a_round_trip() {
        let event = Event::Snapshot {
            protocol_version: PROTOCOL_VERSION,
            revision: 0,
            views: BTreeMap::from([(
                MAIN_VIEW.to_string(),
                ViewState {
                    id: MAIN_VIEW.to_string(),
                    camera: CameraSnapshot::from_tuple((-6.108, 106.91, 11.0, 800, 600)),
                },
            )]),
        };
        let text = serde_json::to_string(&event).expect("serializes");
        let back: Event = serde_json::from_str(&text).expect("parses");
        assert_eq!(back, event);
    }

    #[test]
    fn a_camera_tuple_round_trips_through_the_protocol() {
        let tuple = (-6.2, 106.8, 12.5, 1280, 720);
        assert_eq!(CameraSnapshot::from_tuple(tuple).to_tuple(), tuple);
    }

    /// The schema is embedded rather than read from disk, so a build in a
    /// different working directory still checks it.
    fn schema() -> serde_json::Value {
        serde_json::from_str(include_str!("../../schemas/control-v1.json"))
            .expect("schemas/control-v1.json is valid JSON")
    }

    #[test]
    fn every_command_in_the_code_is_a_command_in_the_schema() {
        // One per variant. Adding a `Command` arm without a schema entry is a
        // documented command a typed client cannot validate, which is the
        // failure this test exists to catch.
        let commands = [
            Command::CameraSet {
                center: GeoPoint {
                    latitude: -6.2,
                    longitude: 106.8,
                },
                zoom: 12.0,
            },
            Command::CameraNudge {
                dx_fraction: 0.25,
                dy_fraction: 0.0,
            },
            Command::CameraZoom { delta: 1.0 },
            Command::ViewReport {
                width: 1280,
                height: 720,
            },
        ];
        let schema = schema();
        let documented: Vec<&str> = schema["$defs"]["command"]["oneOf"]
            .as_array()
            .expect("the schema lists its commands")
            .iter()
            .map(|entry| {
                let name = entry["$ref"]
                    .as_str()
                    .expect("a $ref")
                    .rsplit('/')
                    .next()
                    .expect("a name");
                schema["$defs"][name]["properties"]["type"]["const"]
                    .as_str()
                    .expect("a const type name")
            })
            .collect();
        for command in &commands {
            let value = serde_json::to_value(command).expect("serialises");
            let tag = value["type"].as_str().expect("a type tag");
            assert!(
                documented.contains(&tag),
                "`{tag}` is a command in the code and not in schemas/control-v1.json"
            );
        }
        assert_eq!(
            documented.len(),
            commands.len(),
            "the schema documents a command the code does not have: {documented:?}"
        );
    }

    #[test]
    fn every_error_code_is_in_the_schema() {
        let schema = schema();
        let documented: Vec<&str> = schema["$defs"]["error_code"]["enum"]
            .as_array()
            .expect("the schema lists its codes")
            .iter()
            .map(|value| value.as_str().expect("a string"))
            .collect();
        for code in [
            error_code::UNAUTHORIZED,
            error_code::FORBIDDEN,
            error_code::INVALID_JSON,
            error_code::INVALID_COMMAND,
            error_code::UNSUPPORTED_PROTOCOL,
            error_code::BODY_TOO_LARGE,
            error_code::NOT_FOUND,
            error_code::HOST_GONE,
            error_code::TOO_MANY_CLIENTS,
            error_code::SERVER_ERROR,
        ] {
            assert!(
                documented.contains(&code),
                "`{code}` is an error code in the code and not in the schema"
            );
        }
    }
}
