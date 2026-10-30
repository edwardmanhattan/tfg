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
/// The route version and the envelope version are separate on purpose: this
/// version can serve commands on the `/v1/` routes while the snapshot says
/// which shape its events have. Version 2 adds the observation state
/// (`session`, `readiness`, `displays`) and the observer view.
pub const PROTOCOL_VERSION: u64 = 2;

/// The camera the operator's console renders. A remote command may name it,
/// but only the console itself is the authority about it.
pub const MAIN_VIEW: &str = "main";

/// The camera a command center renders and a phone moves.
///
/// It is a view of its own rather than a second meaning for `main` so an
/// observer can pan without dragging the game master's screen.
pub const OBSERVER_VIEW: &str = "observer";

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

/// The exercise an observation is about, as the console projects it.
///
/// This is deliberately a *projection*: the console holds the Minos session,
/// and the control plane holds only what a viewer needs. No Minos credential,
/// no roster beyond the hulls that move, no message traffic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionState {
    pub game_id: i64,
    pub name: String,
    /// The server's own word: `planning`, `preparation`, `execution`,
    /// `closure`. Kept as text so a phase this build has not heard of is
    /// shown rather than guessed at; [`Self::is_execution`] is the one place
    /// that decides whether observation is live.
    pub phase: String,
    #[serde(default)]
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scenario: Option<ScenarioState>,
    #[serde(default)]
    pub units: Vec<UnitState>,
    /// The task organisation the hulls sit in, so a wall can name a hull's
    /// group and draw the group card the console draws.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<GroupState>,
    /// The scenario's book while the exercise runs. `None` outside execution
    /// and for an exercise with no script.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<BoardState>,
    /// The exercise's seats, as the console sees them: the Crew panel's rows.
    #[serde(default)]
    pub participants: Vec<ParticipantState>,
    /// The traffic the console may read, newest first, bounded. Published so
    /// an observer's Comms panel says what the exercise is saying; the
    /// console is still the only Minos reader.
    #[serde(default)]
    pub messages: Vec<MessageState>,
}

/// One seat in the exercise, for the Crew panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticipantState {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub judge: bool,
    #[serde(default)]
    pub ready: bool,
}

/// One message the exercise holds, as an observer may see it.
///
/// `assumed` keeps the console's rule: a message sent as a scenario role has
/// no author, and the projection never invents one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessageState {
    pub id: i64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub class_label: String,
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub assumed: bool,
    #[serde(default)]
    pub broadcast: bool,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub at: String,
    #[serde(default)]
    pub degree: String,
}

impl SessionState {
    pub fn is_execution(&self) -> bool {
        self.phase == "execution"
    }

    /// Validate what a host published before it enters the shared state.
    pub fn validate(&self) -> Result<(), String> {
        if self.game_id <= 0 {
            return Err("a session needs a positive game id".to_string());
        }
        if self.phase.trim().is_empty() {
            return Err("a session needs a phase".to_string());
        }
        let mut seen = std::collections::BTreeSet::new();
        for unit in &self.units {
            if unit.id <= 0 {
                return Err(format!("unit {} has no id", unit.index));
            }
            if !seen.insert(unit.id) {
                return Err(format!("unit {} appears twice", unit.id));
            }
            if let Some(position) = unit.position {
                if !position.latitude.is_finite() || !(-90.0..=90.0).contains(&position.latitude) {
                    return Err(format!("unit {} has an impossible latitude", unit.id));
                }
                if !position.longitude.is_finite()
                    || !(-180.0..=180.0).contains(&position.longitude)
                {
                    return Err(format!("unit {} has an impossible longitude", unit.id));
                }
            }
        }
        Ok(())
    }
}

/// The scenario being played and the step it is on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioState {
    pub id: i64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
}

/// One task-organisation node, as a wall reads it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroupState {
    pub id: i64,
    pub name: String,
    /// The echelon word the console knows (`Gugus`, `SatuanTugas`, …).
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commander: Option<CommanderState>,
    /// Hulls mustered directly into this node.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub units: Vec<i64>,
}

/// The scenario's steps, as the wall's Scenario window reads them.
///
/// The console's board is derived from three sources — the book, the pointer
/// and the gate — and this is the part of it a wall needs to draw the same
/// window: the book's order, where the exercise stands, each step's commands
/// and the hulls those commands address.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BoardState {
    #[serde(default)]
    pub total: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<usize>,
    #[serde(default)]
    pub steps: Vec<BoardStepState>,
    /// The related hulls of the step the board reads, with their commanders
    /// and declarations. Empty when the step asks nobody.
    #[serde(default)]
    pub units: Vec<BoardUnitState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardStepState {
    pub scenario: String,
    pub step_id: i64,
    /// 1-based position in the whole book.
    pub number: usize,
    #[serde(default)]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<(String, String)>,
    /// `playing`, `asking`, `played` or `later`.
    #[serde(default)]
    pub read: String,
    /// `idle`, `played`, `current`, `unit_ready`, `playing`, `halted` or
    /// `done`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub substeps: Vec<BoardSubstepState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardSubstepState {
    pub id: i64,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub units: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardUnitState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub name: String,
    #[serde(default)]
    pub commander: String,
    /// `None` when the gate does not ask this unit; `Some(false)` is a
    /// declaration still owed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready: Option<bool>,
}

/// One hull a viewer may look at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UnitState {
    /// The number the phone's grid and the command center's marker share.
    /// The hub assigns it from the host's array order, so a publisher leaves
    /// it out and a client reads it back.
    #[serde(default)]
    pub index: u32,
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub hull_number: String,
    #[serde(default)]
    pub class_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commander: Option<CommanderState>,
    /// The hulls aboard this one, direct children only.
    #[serde(default)]
    pub embarked: Vec<UnitRef>,
    /// The hull this one is aboard, when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embarked_in: Option<UnitRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<GeoPoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_deg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_kn: Option<f64>,
    /// No live fix within the registry's own staleness window.
    #[serde(default)]
    pub stale: bool,
    /// Published length and beam, when Minos has them: the command center
    /// sizes a hull's photograph by the ground its hull covers, exactly as
    /// the console does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loa_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beam_m: Option<f64>,
    /// The affiliation the console resolved for this hull (`friendly`,
    /// `neutral`, `hostile`, `unknown`). The wall has no store to resolve it.
    #[serde(default)]
    pub affiliation: String,
    /// The taxonomy symbol, as a wire word (`frigate`, `destroyer`, …). A
    /// client that does not share the console's store draws the same mark
    /// from this.
    #[serde(default)]
    pub symbol: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommanderState {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitRef {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub hull_number: String,
}

/// Why observation is not open yet. The phone's waiting screen names these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessReason {
    NoSession,
    NotExecution,
    NoDisplay,
    Stale,
}

impl ReadinessReason {
    /// The wire word, for a log line or an error message.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoSession => "no_session",
            Self::NotExecution => "not_execution",
            Self::NoDisplay => "no_display",
            Self::Stale => "stale",
        }
    }

    /// What a waiting screen says to a person. The phone renders the wire word
    /// and its own copy; this is for the CLI and the console's readiness bar.
    pub fn sentence(self) -> &'static str {
        match self {
            Self::NoSession => "no exercise is open on the console",
            Self::NotExecution => "the exercise is not in the execution phase",
            Self::NoDisplay => "no command center is attached",
            Self::Stale => "the console has stopped refreshing the exercise",
        }
    }
}

/// The server's verdict about whether a phone may steer the observer view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Readiness {
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ReadinessReason>,
}

impl Readiness {
    pub fn ready() -> Self {
        Self {
            ready: true,
            reason: None,
        }
    }

    pub fn waiting(reason: ReadinessReason) -> Self {
        Self {
            ready: false,
            reason: Some(reason),
        }
    }
}

/// A command center currently attached to the event stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayState {
    pub id: String,
    pub label: String,
    /// `command_center` today; a role rather than a bool so another display
    /// kind can be added without a new field.
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<DisplayViewport>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayViewport {
    pub width: u32,
    pub height: u32,
}

/// Which observer windows the phone has asked the wall to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PanelSet {
    #[serde(default)]
    pub crew: bool,
    #[serde(default)]
    pub scenario: bool,
    #[serde(default)]
    pub comms: bool,
}

/// One of the observer's three windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Crew,
    Scenario,
    Comms,
}

impl Panel {
    pub fn parse(raw: &str) -> Option<Panel> {
        match raw {
            "crew" => Some(Panel::Crew),
            "scenario" => Some(Panel::Scenario),
            "comms" => Some(Panel::Comms),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Panel::Crew => "crew",
            Panel::Scenario => "scenario",
            Panel::Comms => "comms",
        }
    }
}

impl PanelSet {
    /// Set one window, leaving the others as they were.
    pub fn set(&mut self, panel: Panel, open: bool) {
        match panel {
            Panel::Crew => self.crew = open,
            Panel::Scenario => self.scenario = open,
            Panel::Comms => self.comms = open,
        }
    }

    pub fn any(self) -> bool {
        self.crew || self.scenario || self.comms
    }
}

/// What the observer's screen looks like: the windows the phone opened and
/// the hull its popup is about.
///
/// This is UI state, not a game fact — but it lives on the host so every
/// attached wall agrees with the phone, and so a reconnect restores the same
/// screen instead of a blank one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ObserverUi {
    #[serde(default)]
    pub panels: PanelSet,
    /// The unit whose popup is open. Set by `camera.focus`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<i64>,
    /// The task group whose popup is open. Set by `camera.focus_group`;
    /// mutually exclusive with [`Self::focus`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus_group: Option<i64>,
}

/// The host's text size: one setting, host-wide, that every surface draws at.
///
/// A SETTING RATHER THAN A CONSOLE PREFERENCE. The console applies it to its
/// own window and every attached wall applies it to theirs, which is why it
/// travels on the stream: a command that changes one screen and cannot reach
/// the wall the phone is actually looking at is a command with a lie in it.
///
/// `step` is the index into the ladder `GET /v1/settings` publishes, and
/// `text_scale` is the multiplier it resolves to. Both travel so a client
/// that has never read the ladder can size its type from this alone.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SettingsState {
    #[serde(default = "default_text_step")]
    pub step: usize,
    #[serde(default = "default_text_scale")]
    pub text_scale: f32,
}

fn default_text_step() -> usize {
    crate::view::TEXT_SCALE_DEFAULT
}

fn default_text_scale() -> f32 {
    crate::view::text_scale_at(crate::view::TEXT_SCALE_DEFAULT)
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            step: crate::view::TEXT_SCALE_DEFAULT,
            text_scale: crate::view::text_scale_at(crate::view::TEXT_SCALE_DEFAULT),
        }
    }
}

/// The role a display announces. Only a command center may be remoted.
pub const COMMAND_CENTER_ROLE: &str = "command_center";

/// A display's first message on the event stream.
///
/// Sent *up* the WebSocket, not over HTTP: the attachment dies with the socket,
/// which is the one fact the rest of the system needs. A display that is not
/// attached is not showing anything, and a socket that is open is showing
/// something.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayHello {
    /// Stable for the life of a display. Reusing it after a reconnect replaces
    /// the old record rather than stacking a second one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub role: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewport: Option<DisplayViewport>,
}

impl DisplayHello {
    /// The display's id, or a generated one. Bounded and printable, because it
    /// is broadcast to every client and drawn on the console's readiness bar.
    pub fn display_id(&self) -> String {
        let raw = self.id.as_deref().unwrap_or_default().trim();
        let cleaned: String = raw
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
            .take(48)
            .collect();
        if cleaned.is_empty() {
            "display".to_string()
        } else {
            cleaned
        }
    }

    pub fn label_or_id(&self) -> String {
        let label = self.label.trim();
        if label.is_empty() {
            self.display_id()
        } else {
            label.chars().take(64).collect()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.role != COMMAND_CENTER_ROLE {
            return Err(format!(
                "`{}` is not a display role this build serves; the only one is `{COMMAND_CENTER_ROLE}`",
                self.role
            ));
        }
        Ok(())
    }
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
    /// Centre a view on one of the session's hulls.
    ///
    /// The host resolves the unit against the session it owns, so a phone
    /// never has to convert a stale position into a camera it guessed.
    #[serde(rename = "camera.focus")]
    CameraFocus {
        unit_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zoom: Option<f64>,
    },
    /// Centre a view on one of the session's task groups.
    ///
    /// A group has no position of its own; the host resolves it against the
    /// session and centres on the mean of the hulls beneath it, so a phone
    /// never has to compute a group's ground from a stale copy.
    #[serde(rename = "camera.focus_group")]
    CameraFocusGroup {
        group_id: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zoom: Option<f64>,
    },
    /// Open or close one of the observer's windows.
    ///
    /// The phone sends the panel's wire name and the state it wants, so a
    /// retry is idempotent and two toggles cannot fight over "open".
    #[serde(rename = "ui.set")]
    UiSet { panel: String, open: bool },
    /// The console's text size, as a step of its four-step ladder.
    ///
    /// A preference, not a camera: it is host-wide, it needs no readiness,
    /// and the console applies and remembers it. A headless host records it
    /// so a client can read what the wall would use.
    #[serde(rename = "settings.font")]
    SettingsFont { step: u32 },
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
            Self::CameraFocus { unit_id, zoom } => {
                if *unit_id <= 0 {
                    return Err("a focus command needs a positive unit id".to_string());
                }
                if let Some(zoom) = zoom
                    && (!zoom.is_finite() || !(-30.0..=30.0).contains(zoom))
                {
                    return Err("focus zoom must be finite and between -30 and 30".to_string());
                }
            }
            Self::CameraFocusGroup { group_id, zoom } => {
                if *group_id <= 0 {
                    return Err("a group focus needs a positive group id".to_string());
                }
                if let Some(zoom) = zoom
                    && (!zoom.is_finite() || !(-30.0..=30.0).contains(zoom))
                {
                    return Err("focus zoom must be finite and between -30 and 30".to_string());
                }
            }
            Self::SettingsFont { step } => {
                if *step as usize >= crate::view::TEXT_SCALES.len() {
                    return Err(format!(
                        "font step {step} is not one of {}",
                        crate::view::TEXT_SCALES.len()
                    ));
                }
            }
            Self::UiSet { panel, .. } => {
                if Panel::parse(panel).is_none() {
                    return Err(format!(
                        "unknown panel {panel:?}; the observer has crew, scenario and comms"
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
    /// Envelope version. Absent means the current one; a future version is
    /// refused rather than misread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u64>,
    /// Which view the command is about. Absent means `main`, so an old client
    /// that never learned about the observer still talks to the console.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
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

    /// The view this command names. Empty or absent is `main`.
    pub fn view_id(&self) -> String {
        self.view
            .as_deref()
            .map(str::trim)
            .filter(|view| !view.is_empty())
            .unwrap_or(MAIN_VIEW)
            .to_string()
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
    /// The view the command named, already normalised. `main` for a console
    /// command; `observer` for a phone or command-center viewport report.
    pub view: String,
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
        session: Option<SessionState>,
        readiness: Readiness,
        displays: Vec<DisplayState>,
        #[serde(default)]
        ui: ObserverUi,
        /// The host's text size. Sent with the state so a wall that has just
        /// connected draws at it rather than at its own default.
        #[serde(default)]
        settings: SettingsState,
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
    /// The observation state changed: the session, its phase, its units, the
    /// readiness verdict, or which command centers are attached.
    ///
    /// One event rather than four, because every one of them changes the same
    /// decision on the phone: may I steer, and what am I steering?
    #[serde(rename = "session.changed")]
    SessionChanged {
        revision: u64,
        session: Option<SessionState>,
        readiness: Readiness,
        displays: Vec<DisplayState>,
        #[serde(default)]
        ui: ObserverUi,
    },
    /// The host's text size changed. Every surface that draws type redraws at
    /// the new step — that is what makes it host-wide rather than a
    /// preference one client keeps to itself.
    #[serde(rename = "settings.changed")]
    SettingsChanged {
        revision: u64,
        settings: SettingsState,
    },
    /// A display handshake was accepted on this socket.
    #[serde(rename = "display.attached")]
    DisplayAttached { id: String },
    /// A display handshake was refused. The socket stays open, so a client
    /// can retry with a different id or give a readable reason.
    #[serde(rename = "display.rejected")]
    DisplayRejected { code: String, message: String },
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
    /// The observation is not open yet: no session, not execution, no command
    /// center attached, or the console went quiet. The body names which.
    pub const NOT_READY: &str = "not_ready";
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
            session: None,
            readiness: Readiness::waiting(ReadinessReason::NoSession),
            displays: Vec::new(),
            ui: Default::default(),
            settings: Default::default(),
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

    #[test]
    fn a_command_without_a_view_is_about_the_console() {
        let request = parse(r#"{"type":"camera.zoom","payload":{"delta":1.0}}"#);
        assert_eq!(request.view_id(), MAIN_VIEW);
        let request = parse(r#"{"view":"observer","type":"camera.zoom","payload":{"delta":1.0}}"#);
        assert_eq!(request.view_id(), OBSERVER_VIEW);
        let request = parse(r#"{"view":"  ","type":"camera.zoom","payload":{"delta":1.0}}"#);
        assert_eq!(request.view_id(), MAIN_VIEW);
    }

    #[test]
    fn a_focus_needs_a_real_unit_and_a_usable_zoom() {
        assert!(
            Command::CameraFocus {
                unit_id: 0,
                zoom: None
            }
            .validate()
            .is_err()
        );
        assert!(
            Command::CameraFocus {
                unit_id: 1,
                zoom: Some(f64::NAN)
            }
            .validate()
            .is_err()
        );
        assert!(
            Command::CameraFocus {
                unit_id: 1,
                zoom: Some(14.0)
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn a_session_rejects_duplicate_and_impossible_units() {
        let mut session = SessionState {
            game_id: 42,
            name: "Exercise".to_string(),
            phase: "execution".to_string(),
            mode: "maneuver".to_string(),
            scenario: None,
            units: vec![UnitState {
                index: 1,
                id: 7,
                name: "Evertsen".to_string(),
                hull_number: "F805".to_string(),
                class_name: "Frigate".to_string(),
                commander: None,
                embarked: Vec::new(),
                embarked_in: None,
                position: Some(GeoPoint {
                    latitude: -6.2,
                    longitude: 106.8,
                }),
                heading_deg: None,
                speed_kn: None,
                stale: false,
                ..UnitState::default()
            }],
            ..SessionState::default()
        };
        assert!(session.validate().is_ok());
        session.units.push(session.units[0].clone());
        assert!(session.validate().is_err());
    }

    /// A setting travels as state, and a body that predates it lands on the
    /// ladder's default rather than on its first rung.
    #[test]
    fn a_settings_change_round_trips_and_defaults_to_the_ladder() {
        let event = Event::SettingsChanged {
            revision: 4,
            settings: SettingsState {
                step: 3,
                text_scale: crate::view::text_scale_at(3),
            },
        };
        let text = serde_json::to_string(&event).expect("serializes");
        assert!(text.contains("\"settings.changed\""));
        let back: Event = serde_json::from_str(&text).expect("parses");
        assert_eq!(back, event);

        assert_eq!(SettingsState::default().step, crate::view::TEXT_SCALE_DEFAULT);
        let bare: SettingsState = serde_json::from_str("{}").expect("parses");
        assert_eq!(bare, SettingsState::default());
    }

    #[test]
    fn readiness_and_session_events_round_trip() {
        let event = Event::SessionChanged {
            revision: 3,
            session: Some(SessionState {
                game_id: 42,
                name: "Exercise".to_string(),
                phase: "execution".to_string(),
                mode: "maneuver".to_string(),
                scenario: None,
                units: Vec::new(),
                ..SessionState::default()
            }),
            readiness: Readiness::waiting(ReadinessReason::NoDisplay),
            displays: vec![DisplayState {
                id: "wall-1".to_string(),
                label: "wall-1".to_string(),
                role: "command_center".to_string(),
                viewport: None,
            }],
            ui: Default::default(),
        };
        let text = serde_json::to_string(&event).expect("serializes");
        let back: Event = serde_json::from_str(&text).expect("parses");
        assert_eq!(back, event);
        assert!(text.contains("\"no_display\""));
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
            Command::CameraFocus {
                unit_id: 42,
                zoom: Some(12.0),
            },
            Command::ViewReport {
                width: 1280,
                height: 720,
            },
            Command::UiSet {
                panel: "crew".to_string(),
                open: true,
            },
            Command::SettingsFont { step: 2 },
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
