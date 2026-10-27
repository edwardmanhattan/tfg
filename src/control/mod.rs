//! The control plane: a versioned seam between the desktop console and
//! remote clients.
//!
//! The shape is the one the ADR fixes (`docs/adr/0025-control-plane.md`):
//!
//! - [`protocol`] is the wire contract as plain data — no GPUI, no Tokio, no
//!   Minos. A command is a semantic intent, and a camera is a place on the
//!   earth rather than a pixel.
//! - [`hub`] is the shared state: one writer (the desktop's order thread, or a
//!   headless server's command thread), one monotonic revision, one fan-out to
//!   watchers, and the table of recently seen command ids that makes a retry
//!   safe.
//! - [`server`] is the transport: HTTP JSON for commands and reads, one
//!   WebSocket for events, on its own thread with its own runtime.
//! - [`client`] is the other side of that transport: a console that follows a
//!   host instead of hosting, applying the host's cameras and posting its own.
//! - [`auth`] is the bearer-token check.
//! - [`devices`] is pairing: short-lived codes, per-device tokens stored as
//!   hashes, and the scopes that separate watching from steering.
//!
//! The rule the modules exist to keep: **no network thread calls GPUI**. The
//! server and the client send [`AcceptedCommand`]s down a `std::sync::mpsc`
//! channel; the shell drains that on its own tick and moves the camera with the
//! same methods a mouse gesture uses.

mod auth;
mod client;
mod devices;
mod hub;
mod protocol;
mod publish;
mod server;

pub use auth::MIN_TOKEN_LEN;
pub use client::{ClientConfig, ClientHandle, spawn_client};
pub use devices::{
    Credentials, DeviceView, Devices, Grant, PAIRING_ATTEMPTS, PAIRING_TTL, PairError, Paired,
    PairingOffer, Scope,
};
pub use hub::ControlHub;
pub use protocol::{
    API_VERSION, AcceptedCommand, COMMAND_CENTER_ROLE, CameraSnapshot, CameraSource, Command,
    CommandRequest, CommanderState, DisplayHello, DisplayState, DisplayViewport, Event, GeoPoint,
    MAIN_VIEW, OBSERVER_VIEW, PROTOCOL_VERSION, Readiness, ReadinessReason, ScenarioState,
    SessionState, UnitRef, UnitState, ViewState, error_code,
};
pub use publish::{
    SessionHandle, SessionSink, SessionUpdate, UnitPosition, spawn_session_publisher,
};
pub use server::{MAX_BODY_BYTES, MAX_WS_CLIENTS, ServerConfig, ServerHandle, spawn_server};

/// The channel the shell drains once per tick: a command already validated at
/// the protocol boundary, addressed to this desktop.
pub type CommandFeed = Box<dyn FnMut() -> Option<AcceptedCommand> + Send + 'static>;
