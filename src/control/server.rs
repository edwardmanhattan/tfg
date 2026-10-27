//! The embedded control server: HTTP JSON commands, one WebSocket event stream.
//!
//! The server has its own thread and its own Tokio runtime, because the UI
//! thread must never wait on a socket and the network thread must never touch
//! a toolkit object. Everything it knows about the application arrives as a
//! [`ControlHub`] (read) or leaves through the `std::sync::mpsc` command
//! channel (write) — the desktop drains that on its own UI tick.
//!
//! Why hyper and not axum: hyper, hyper-util and tungstenite are already in
//! the dependency tree through the socket SDK, so this adds no crate to the
//! build. The handlers below are the same shape axum would give them.

use super::auth;
use super::devices::{Credentials, Devices, PairError, Scope};
use super::hub::ControlHub;
use super::protocol::{
    API_VERSION, AcceptedCommand, COMMAND_CENTER_ROLE, CameraSource, Command, CommandRequest,
    DisplayHello, DisplayState, Event, MAIN_VIEW, PROTOCOL_VERSION, Readiness, ReadinessReason,
    SessionState, error_code,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{CONNECTION, UPGRADE};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::{Message, Role};

/// The largest command body the server will read. A camera command is a few
/// hundred bytes; anything near this is a mistake or an attack.
pub const MAX_BODY_BYTES: usize = 64 * 1024;

/// How many WebSocket clients may watch at once. Each one is a task and a
/// 64-event buffer, so the bound is about memory honesty, not policy.
pub const MAX_WS_CLIENTS: usize = 8;

/// How long startup waits for the listener to bind before reporting failure.
const BIND_TIMEOUT: Duration = Duration::from_secs(5);

/// How often sessions and displays are checked against their deadlines.
///
/// Fine enough that a phone notices a dead console within a couple of seconds
/// of the deadline, coarse enough to be nothing on the runtime.
const EXPIRY_TICK: Duration = Duration::from_secs(2);

/// Where the server listens and what it accepts.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// A socket address, e.g. `127.0.0.1:8787`. Loopback unless the operator
    /// says otherwise.
    pub listen: String,
    /// The bearer token. Must be at least [`auth::MIN_TOKEN_LEN`] characters.
    pub token: String,
    /// The paired-device table. `None` gives a server with no pairing: the
    /// operator token is then the only credential, which is what the tests
    /// and a throwaway run want.
    pub devices: Option<std::path::PathBuf>,
}

impl ServerConfig {
    /// The common case: a listen address, a token, and the device table at the
    /// user's data directory.
    pub fn new(listen: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            listen: listen.into(),
            token: token.into(),
            devices: Devices::default_path(),
        }
    }
}

/// The address a client on another machine should put in front of `/v1`.
///
/// A `0.0.0.0` bind is not an address anybody can connect to, so the pairing
/// payload names the first non-loopback IPv4 the host has instead. Getting
/// this wrong is the difference between a QR code that works and one that
/// sends a phone to `0.0.0.0`.
fn advertised_url(addr: SocketAddr) -> String {
    if !addr.ip().is_unspecified() {
        return format!("http://{addr}");
    }
    let port = addr.port();
    let fallback = format!("http://127.0.0.1:{port}");
    let Ok(list) = std::net::UdpSocket::bind("0.0.0.0:0").and_then(|socket| {
        // No packet is sent: connect() on a UDP socket only picks the route,
        // which is enough to learn this machine's address on the LAN.
        socket.connect("192.0.2.1:9")?;
        socket.local_addr()
    }) else {
        return fallback;
    };
    match list.ip() {
        std::net::IpAddr::V4(ip) if !ip.is_loopback() => format!("http://{ip}:{port}"),
        _ => fallback,
    }
}

/// A running server: where it bound, and the thread it runs on.
pub struct ServerHandle {
    addr: SocketAddr,
    _thread: JoinHandle<()>,
}

impl ServerHandle {
    /// The address actually bound. With port 0 in the config this is the port
    /// the operating system chose, which is what tests and log lines want.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The base URL a client should use on the same machine.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The base URL to put in a pairing payload: reachable from the LAN when
    /// the bind is, and loopback when it is not.
    pub fn advertised_url(&self) -> String {
        advertised_url(self.addr)
    }
}

struct ServerState {
    credentials: Arc<Credentials>,
    hub: ControlHub,
    command_tx: Sender<AcceptedCommand>,
    clients: Arc<AtomicUsize>,
    generated: AtomicU64,
    advertised: String,
}

/// Start the server on its own thread.
///
/// Returns once the listener is bound, so a bind failure is reported to the
/// caller instead of scrolling past on a background thread.
pub fn spawn_server(
    config: ServerConfig,
    command_tx: Sender<AcceptedCommand>,
    hub: ControlHub,
) -> Result<ServerHandle, String> {
    if config.token.len() < auth::MIN_TOKEN_LEN {
        return Err(format!(
            "the control token must be at least {} characters (try `openssl rand -hex 32`)",
            auth::MIN_TOKEN_LEN
        ));
    }
    if config.token.trim().is_empty() {
        return Err("the control token is empty".to_string());
    }
    // Opened BEFORE the thread starts, so a devices file that does not parse is
    // reported to the caller rather than discovered by the first client.
    let devices = Devices::open(config.devices.clone())?;
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<SocketAddr, String>>();
    let thread = std::thread::Builder::new()
        .name("tfg-control".to_string())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready_tx.send(Err(format!("control runtime: {error}")));
                    return;
                }
            };
            runtime.block_on(async move {
                let listener = match TcpListener::bind(&config.listen).await {
                    Ok(listener) => listener,
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!(
                            "control server bind {}: {error}",
                            config.listen
                        )));
                        return;
                    }
                };
                let addr = match listener.local_addr() {
                    Ok(addr) => addr,
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!("control server address: {error}")));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(addr));
                let advertised = advertised_url(addr);
                let state = Arc::new(ServerState {
                    credentials: Arc::new(Credentials::new(config.token.clone(), devices)),
                    hub: hub.clone(),
                    command_tx,
                    clients: Arc::new(AtomicUsize::new(0)),
                    generated: AtomicU64::new(0),
                    advertised,
                });
                // The readiness verdict is computed from timestamps, so it is
                // already right the moment a deadline passes. This task exists
                // to *say so*: a phone sitting on a waiting screen has to be
                // told that the console went quiet, and a display that closed
                // its laptop must stop counting as attached.
                tokio::spawn(async move {
                    let mut tick = tokio::time::interval(EXPIRY_TICK);
                    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                    loop {
                        tick.tick().await;
                        hub.expire();
                    }
                });
                accept_loop(listener, state).await;
            });
        })
        .map_err(|error| format!("control server thread: {error}"))?;
    let addr = ready_rx
        .recv_timeout(BIND_TIMEOUT)
        .map_err(|_| "control server did not report a bound address".to_string())??;
    Ok(ServerHandle {
        addr,
        _thread: thread,
    })
}

async fn accept_loop(listener: TcpListener, state: Arc<ServerState>) {
    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => {
                // A failed accept is per-connection (file descriptors, a reset
                // before the handshake); the listener is still good.
                eprintln!("control server accept: {error}");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };
        let state = state.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |request: Request<Incoming>| {
                let state = state.clone();
                async move { handle(request, state).await }
            });
            if let Err(error) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .with_upgrades()
                .await
            {
                // Clients disconnect mid-body all the time; that is not news.
                let _ = error;
            }
        });
    }
}

async fn handle(
    request: Request<Incoming>,
    state: Arc<ServerState>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = request.uri().path().to_string();
    let method = request.method().clone();

    // The two unauthenticated routes. A client that cannot reach hello cannot
    // learn which protocol version to speak; a client that cannot reach pair
    // cannot spend a code it was shown.
    if path == format!("/{API_VERSION}/hello") && method == Method::GET {
        return Ok(json_response(
            StatusCode::OK,
            &serde_json::json!({
                "protocol_version": PROTOCOL_VERSION,
                "api_version": API_VERSION,
                "service": "tfg-control",
                "revision": state.hub.revision(),
                "pairing_open": state.credentials.devices().pairing_open(),
            }),
        ));
    }
    if path == format!("/{API_VERSION}/pair") && method == Method::POST {
        return Ok(pair(request, &state).await);
    }

    let grant = match state.credentials.authorise(request.headers()) {
        Some(grant) => grant,
        None => {
            let mut response = error_response(
                StatusCode::UNAUTHORIZED,
                error_code::UNAUTHORIZED,
                "missing or wrong bearer token",
            );
            response.headers_mut().insert(
                hyper::header::WWW_AUTHENTICATE,
                hyper::header::HeaderValue::from_static("Bearer"),
            );
            return Ok(response);
        }
    };

    match (&method, path.as_str()) {
        (&Method::GET, "/v1/state") => {
            if let Err(response) = require(&grant, Scope::Observe) {
                return Ok(response);
            }
            Ok(json_response(
                StatusCode::OK,
                &serde_json::to_value(state.hub.snapshot_event())
                    .unwrap_or_else(|_| serde_json::json!({})),
            ))
        }
        (&Method::GET, "/v1/whoami") => Ok(json_response(
            StatusCode::OK,
            &serde_json::json!({
                "client": grant.id,
                "label": grant.label,
                "scopes": grant.scopes,
            }),
        )),
        (&Method::GET, "/v1/views") => {
            if let Err(response) = require(&grant, Scope::Observe) {
                return Ok(response);
            }
            let Event::Snapshot { views, .. } = state.hub.snapshot_event() else {
                return Ok(error_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    error_code::SERVER_ERROR,
                    "snapshot is not a snapshot",
                ));
            };
            Ok(json_response(
                StatusCode::OK,
                &serde_json::json!({ "views": views }),
            ))
        }
        (&Method::GET, path) if path.starts_with("/v1/views/") => {
            if let Err(response) = require(&grant, Scope::Observe) {
                return Ok(response);
            }
            let mut parts = path.trim_start_matches("/v1/views/").split('/');
            let view_id = parts.next().unwrap_or_default();
            let tail = parts.next();
            if tail != Some("camera") || parts.next().is_some() {
                return Ok(not_found());
            }
            let Some(camera) = state.hub.camera_of(view_id) else {
                return Ok(not_found());
            };
            Ok(json_response(
                StatusCode::OK,
                &serde_json::json!({ "id": view_id, "camera": camera }),
            ))
        }
        (&Method::GET, "/v1/session") => {
            if let Err(response) = require(&grant, Scope::Observe) {
                return Ok(response);
            }
            Ok(json_response(
                StatusCode::OK,
                &serde_json::json!({
                    "session": state.hub.session(),
                    "readiness": state.hub.readiness(),
                    "displays": state.hub.displays(),
                    "ui": state.hub.ui(),
                }),
            ))
        }
        (&Method::PUT, "/v1/session") => {
            // Only the console publishes: it is the one process that holds a
            // Minos session, and a projection nobody can vouch for is worse
            // than none.
            if let Err(response) = require(&grant, Scope::Admin) {
                return Ok(response);
            }
            let body = match read_body(request).await {
                Ok(body) => body,
                Err(response) => return Ok(response),
            };
            let session: SessionState = match serde_json::from_slice(&body) {
                Ok(session) => session,
                Err(error) => {
                    return Ok(error_response(
                        StatusCode::BAD_REQUEST,
                        error_code::INVALID_JSON,
                        &format!("invalid session JSON: {error}"),
                    ));
                }
            };
            if let Err(message) = session.validate() {
                return Ok(error_response(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    error_code::INVALID_COMMAND,
                    &message,
                ));
            }
            let changed = state.hub.set_session(session).is_some();
            Ok(json_response(
                StatusCode::OK,
                &serde_json::json!({
                    "status": if changed { "changed" } else { "unchanged" },
                    "revision": state.hub.revision(),
                    "readiness": state.hub.readiness(),
                }),
            ))
        }
        (&Method::DELETE, "/v1/session") => {
            if let Err(response) = require(&grant, Scope::Admin) {
                return Ok(response);
            }
            state.hub.clear_session();
            Ok(json_response(
                StatusCode::OK,
                &serde_json::json!({
                    "status": "cleared",
                    "revision": state.hub.revision(),
                    "readiness": state.hub.readiness(),
                }),
            ))
        }
        (&Method::GET, path) if path.starts_with("/v1/units/") => {
            if let Err(response) = require(&grant, Scope::Observe) {
                return Ok(response);
            }
            let mut parts = path.trim_start_matches("/v1/units/").split('/');
            let unit_id = parts.next().unwrap_or_default();
            if parts.next() != Some("picture") || parts.next().is_some() {
                return Ok(not_found());
            }
            let Ok(unit_id) = unit_id.parse::<i64>() else {
                return Ok(not_found());
            };
            let Some(png) = state.hub.picture(unit_id) else {
                return Ok(not_found());
            };
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header(hyper::header::CONTENT_TYPE, "image/png")
                .body(Full::new(Bytes::from(png.to_vec())))
                .unwrap_or_else(|_| Response::new(Full::new(Bytes::from_static(b"")))))
        }
        (&Method::PUT, path) if path.starts_with("/v1/units/") => {
            // The console is the only reader of Minos, so it is the only
            // publisher of hull pictures. A wall fetches them from here.
            if let Err(response) = require(&grant, Scope::Admin) {
                return Ok(response);
            }
            let mut parts = path.trim_start_matches("/v1/units/").split('/');
            let unit_id = parts.next().unwrap_or_default();
            if parts.next() != Some("picture") || parts.next().is_some() {
                return Ok(not_found());
            }
            let Ok(unit_id) = unit_id.parse::<i64>() else {
                return Ok(not_found());
            };
            let body = match read_body_limited(request, MAX_PICTURE_BYTES).await {
                Ok(body) => body,
                Err(response) => return Ok(response),
            };
            if body.len() < 8 || &body[..8] != b"\x89PNG\r\n\x1a\n" {
                return Ok(error_response(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    error_code::INVALID_COMMAND,
                    "a hull picture must be a PNG",
                ));
            }
            state.hub.set_picture(unit_id, body.to_vec());
            Ok(json_response(
                StatusCode::OK,
                &serde_json::json!({ "status": "stored", "unit_id": unit_id, "bytes": body.len() }),
            ))
        }
        (&Method::GET, "/v1/displays") => {
            if let Err(response) = require(&grant, Scope::Observe) {
                return Ok(response);
            }
            Ok(json_response(
                StatusCode::OK,
                &serde_json::json!({
                    "displays": state.hub.displays(),
                    "readiness": state.hub.readiness(),
                }),
            ))
        }
        (&Method::POST, "/v1/commands") => Ok(submit_command(request, &state, &grant).await),
        (&Method::POST, "/v1/pairing") => Ok(open_pairing(&state, &grant, request).await),
        (&Method::GET, "/v1/devices") => {
            if let Err(response) = require(&grant, Scope::Admin) {
                return Ok(response);
            }
            Ok(json_response(
                StatusCode::OK,
                &serde_json::json!({ "devices": state.credentials.devices().list() }),
            ))
        }
        (&Method::DELETE, path) if path.starts_with("/v1/devices/") => {
            if let Err(response) = require(&grant, Scope::Admin) {
                return Ok(response);
            }
            let id = path.trim_start_matches("/v1/devices/");
            if id.is_empty() || id.contains('/') {
                return Ok(not_found());
            }
            if state.credentials.devices().revoke(id) {
                Ok(json_response(
                    StatusCode::OK,
                    &serde_json::json!({ "device": id, "status": "revoked" }),
                ))
            } else {
                Ok(error_response(
                    StatusCode::NOT_FOUND,
                    error_code::UNKNOWN_DEVICE,
                    "no device has that id",
                ))
            }
        }
        (&Method::POST, "/v1/auth/rotate") => Ok(rotate(&state, &grant)),
        (&Method::GET, "/v1/ws") => {
            if let Err(response) = require(&grant, Scope::Observe) {
                return Ok(response);
            }
            Ok(upgrade_websocket(request, state, grant).await)
        }
        _ => Ok(not_found()),
    }
}

/// A refusal a client can branch on: the credential is real, it may not do
/// this.
fn require(grant: &super::devices::Grant, needed: Scope) -> Result<(), Response<Full<Bytes>>> {
    if grant.allows(needed) {
        Ok(())
    } else {
        Err(error_response(
            StatusCode::FORBIDDEN,
            error_code::FORBIDDEN,
            &format!(
                "this credential carries {:?} and this needs a {} scope",
                grant.scopes, needed
            ),
        ))
    }
}

/// The scope a command needs. Everything that moves a map is
/// `control_view`; a device that only watches cannot steer.
fn needed_scope(command: &Command) -> Scope {
    match command {
        Command::CameraSet { .. }
        | Command::CameraNudge { .. }
        | Command::CameraZoom { .. }
        | Command::CameraFocus { .. } => Scope::ControlView,
        // The viewport is something a display says about itself. A phone that
        // lies about it could change what "a quarter of a viewport" means, so
        // it needs the same scope that steering does.
        Command::ViewReport { .. } => Scope::ControlView,
        // Opening a window on the wall is the same authority a nudge needs:
        // it is observer UI, and a watch-only device does not steer it.
        Command::UiSet { .. } => Scope::ControlView,
    }
}

/// Whether a command may move a camera right now.
///
/// The readiness verdict is not advice: it is the answer to "is anyone there to
/// see this move". A phone steering a view nobody renders is a gesture into a
/// hole, so the server refuses it and says which condition is missing. The
/// viewport report is exempt — a display says how big it is the moment it
/// attaches, before the session is open.
fn readiness_refusal(command: &Command, readiness: &Readiness) -> Option<(String, String)> {
    if matches!(command, Command::ViewReport { .. }) || readiness.ready {
        return None;
    }
    let reason = readiness.reason.map(ReadinessReason::as_str).unwrap_or("not_ready");
    Some((
        error_code::NOT_READY.to_string(),
        format!("observation is not open yet: {reason}"),
    ))
}

/// `POST /v1/pairing` — the operator opens a window and reads the code out.
async fn open_pairing(
    state: &Arc<ServerState>,
    grant: &super::devices::Grant,
    request: Request<Incoming>,
) -> Response<Full<Bytes>> {
    if let Err(response) = require(grant, Scope::Admin) {
        return response;
    }
    let body = match read_body(request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    #[derive(serde::Deserialize, Default)]
    struct Request_ {
        #[serde(default)]
        scopes: Vec<Scope>,
        #[serde(default)]
        ttl_seconds: Option<u64>,
    }
    let parsed: Request_ = if body.is_empty() {
        Request_::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(parsed) => parsed,
            Err(error) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    error_code::INVALID_JSON,
                    &format!("invalid pairing JSON: {error}"),
                );
            }
        }
    };
    // Default: everything a remote control needs and nothing it does not.
    let scopes = if parsed.scopes.is_empty() {
        vec![Scope::Observe, Scope::ControlView]
    } else {
        parsed.scopes
    };
    if scopes.contains(&Scope::Admin) {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            error_code::INVALID_COMMAND,
            "a paired device may not carry the admin scope; pair it as the operator or not at all",
        );
    }
    let ttl = parsed
        .ttl_seconds
        .map(|seconds| Duration::from_secs(seconds.clamp(30, 600)))
        .unwrap_or(super::devices::PAIRING_TTL);
    match state
        .credentials
        .devices()
        .open_pairing(scopes, ttl, &state.advertised)
    {
        Ok(offer) => json_response(
            StatusCode::OK,
            &serde_json::to_value(offer).unwrap_or_default(),
        ),
        Err(message) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            error_code::SERVER_ERROR,
            &message,
        ),
    }
}

/// `POST /v1/pair` — the unauthenticated exchange of a code for a device token.
async fn pair(request: Request<Incoming>, state: &Arc<ServerState>) -> Response<Full<Bytes>> {
    let body = match read_body(request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    #[derive(serde::Deserialize)]
    struct Request_ {
        code: String,
        #[serde(default)]
        label: Option<String>,
        #[serde(default)]
        client_id: Option<String>,
    }
    let parsed: Request_ = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                error_code::INVALID_JSON,
                &format!("invalid pairing JSON: {error}"),
            );
        }
    };
    let label = parsed
        .label
        .as_deref()
        .or(parsed.client_id.as_deref())
        .unwrap_or("paired device");
    match state.credentials.devices().pair(&parsed.code, label) {
        Ok(paired) => json_response(
            StatusCode::OK,
            &serde_json::json!({
                "device": paired.device,
                "token": paired.token,
                "url": state.advertised,
            }),
        ),
        Err(error) => pair_error(error),
    }
}

fn pair_error(error: PairError) -> Response<Full<Bytes>> {
    let status = StatusCode::from_u16(error.status()).unwrap_or(StatusCode::BAD_REQUEST);
    error_response(status, error.code(), error.message())
}

/// `POST /v1/auth/rotate` — a device retires its own token.
fn rotate(state: &Arc<ServerState>, grant: &super::devices::Grant) -> Response<Full<Bytes>> {
    if grant.id == "operator" {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            error_code::FORBIDDEN,
            "the operator token comes from TFG_CONTROL_TOKEN; change it in the environment and restart",
        );
    }
    match state.credentials.devices().rotate(&grant.id) {
        Some(token) => json_response(
            StatusCode::OK,
            &serde_json::json!({ "device": grant.id, "token": token }),
        ),
        None => error_response(
            StatusCode::NOT_FOUND,
            error_code::UNKNOWN_DEVICE,
            "this device is no longer in the table; pair again",
        ),
    }
}

/// Read a bounded JSON body, or the refusal that says why not.
/// A hull photograph is not a command body: a 512 px PNG is a few hundred
/// kilobytes, and the console already caps the picture it keeps.
const MAX_PICTURE_BYTES: usize = 4 * 1024 * 1024;

async fn read_body(request: Request<Incoming>) -> Result<Bytes, Response<Full<Bytes>>> {
    read_body_limited(request, MAX_BODY_BYTES).await
}

async fn read_body_limited(
    request: Request<Incoming>,
    limit: usize,
) -> Result<Bytes, Response<Full<Bytes>>> {
    match Limited::new(request.into_body(), limit)
        .collect()
        .await
    {
        Ok(collected) => Ok(collected.to_bytes()),
        Err(_) => Err(error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            error_code::BODY_TOO_LARGE,
            "the body is larger than the server reads",
        )),
    }
}

async fn submit_command(
    request: Request<Incoming>,
    state: &Arc<ServerState>,
    grant: &super::devices::Grant,
) -> Response<Full<Bytes>> {
    let body = match read_body(request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let parsed: CommandRequest = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(error) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                error_code::INVALID_JSON,
                &format!("invalid command JSON: {error}"),
            );
        }
    };
    let client_id = parsed.client_id();
    let view = parsed.view_id();
    let command_id = parsed
        .command_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| next_command_id(state));

    if let Some(version) = parsed.protocol_version
        && version != PROTOCOL_VERSION
    {
        state.hub.command_rejected(
            &command_id,
            error_code::UNSUPPORTED_PROTOCOL,
            "the server speaks protocol version 2",
        );
        return error_response(
            StatusCode::CONFLICT,
            error_code::UNSUPPORTED_PROTOCOL,
            &format!(
                "protocol version {version} is not supported; this server speaks {PROTOCOL_VERSION}"
            ),
        );
    }

    if let Err(message) = parsed.command.validate() {
        state
            .hub
            .command_rejected(&command_id, error_code::INVALID_COMMAND, &message);
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            error_code::INVALID_COMMAND,
            &message,
        );
    }

    // Scope check AFTER validation: a malformed command is a client bug and a
    // forbidden one is a credential fact, and the bug is the more useful thing
    // to report first.
    let needed = needed_scope(&parsed.command);
    if !grant.allows(needed) {
        let message = format!("this credential may not send {} commands", needed);
        state
            .hub
            .command_rejected(&command_id, error_code::FORBIDDEN, &message);
        return error_response(StatusCode::FORBIDDEN, error_code::FORBIDDEN, &message);
    }

    if view != MAIN_VIEW {
        // A view the console does not render is resolved here, on the server
        // thread, against the state the server owns. It never reaches the
        // desktop: a phone must not be able to move the game master's screen,
        // and the command center is a display, not a second writer.
        if let Some((code, message)) =
            readiness_refusal(&parsed.command, &state.hub.readiness())
        {
            state.hub.command_rejected(&command_id, &code, &message);
            return error_response(StatusCode::CONFLICT, &code, &message);
        }
        if state.hub.remember_command(&command_id) {
            return json_response(
                StatusCode::OK,
                &serde_json::json!({
                    "command_id": command_id,
                    "status": "duplicate",
                    "view": view,
                    "revision": state.hub.revision(),
                }),
            );
        }
        let source = CameraSource {
            client_id: client_id.clone(),
            command_id: Some(command_id.clone()),
        };
        match state.hub.apply_view(&view, &parsed.command, source) {
            Ok(_) => {
                state.hub.command_accepted(&command_id);
                return json_response(
                    StatusCode::ACCEPTED,
                    &serde_json::json!({
                        "command_id": command_id,
                        "status": "accepted",
                        "view": view,
                        "revision": state.hub.revision(),
                    }),
                );
            }
            Err(error) => {
                let message = error.to_string();
                state.hub.command_rejected(
                    &command_id,
                    error_code::INVALID_COMMAND,
                    &message,
                );
                return error_response(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    error_code::INVALID_COMMAND,
                    &message,
                );
            }
        }
    }

    // Idempotency: a retry of a command already accepted answers with the
    // duplicate status and does not reach the desktop a second time.
    if state.hub.remember_command(&command_id) {
        return json_response(
            StatusCode::OK,
            &serde_json::json!({
                "command_id": command_id,
                "status": "duplicate",
                "revision": state.hub.revision(),
            }),
        );
    }

    let accepted = AcceptedCommand {
        client_id,
        command_id: command_id.clone(),
        view,
        command: parsed.command,
    };
    if state.command_tx.send(accepted).is_err() {
        state.hub.command_rejected(
            &command_id,
            error_code::HOST_GONE,
            "the desktop is no longer listening",
        );
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            error_code::HOST_GONE,
            "the desktop's command channel is closed",
        );
    }
    state.hub.command_accepted(&command_id);
    json_response(
        StatusCode::ACCEPTED,
        &serde_json::json!({
            "command_id": command_id,
            "status": "accepted",
            "revision": state.hub.revision(),
        }),
    )
}

fn next_command_id(state: &Arc<ServerState>) -> String {
    let counter = state.generated.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("srv-{now}-{counter}")
}

/// Answer a WebSocket upgrade and hand the connection to its own task.
///
/// The subscription is created BEFORE the 101 goes out, so an event published
/// between the snapshot and the first read is not lost. A late reader sees the
/// snapshot (the state at one revision) plus everything after it.
async fn upgrade_websocket(
    mut request: Request<Incoming>,
    state: Arc<ServerState>,
    grant: super::devices::Grant,
) -> Response<Full<Bytes>> {
    if !is_websocket(&request) {
        return error_response(
            StatusCode::BAD_REQUEST,
            error_code::INVALID_JSON,
            "this route is a WebSocket; send Upgrade: websocket",
        );
    }
    if state.clients.fetch_add(1, Ordering::SeqCst) >= MAX_WS_CLIENTS {
        state.clients.fetch_sub(1, Ordering::SeqCst);
        return error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            error_code::TOO_MANY_CLIENTS,
            "too many control clients are watching",
        );
    }
    let guard = ClientGuard {
        clients: state.clients.clone(),
    };

    // tungstenite builds the accept key; hyper performs the upgrade. Both use
    // http 1.x, so the request types are the same types.
    let mut handshake = Request::builder()
        .method(request.method().clone())
        .uri(request.uri().clone())
        .version(request.version());
    for (name, value) in request.headers() {
        handshake = handshake.header(name, value);
    }
    let handshake = match handshake.body(()) {
        Ok(handshake) => handshake,
        Err(_) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                error_code::INVALID_JSON,
                "bad upgrade request",
            );
        }
    };
    let accept =
        match tokio_tungstenite::tungstenite::handshake::server::create_response(&handshake) {
            Ok(accept) => accept,
            Err(error) => {
                return error_response(
                    StatusCode::BAD_REQUEST,
                    error_code::INVALID_JSON,
                    &format!("the WebSocket handshake was refused: {error}"),
                );
            }
        };

    let events = state.hub.subscribe();
    let hub = state.hub.clone();
    let upgrade = hyper::upgrade::on(&mut request);
    tokio::spawn(async move {
        let _guard = guard;
        let upgraded = match upgrade.await {
            Ok(upgraded) => upgraded,
            Err(error) => {
                eprintln!("control websocket upgrade: {error}");
                return;
            }
        };
        let stream =
            WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
        serve_websocket(stream, hub, events, grant).await;
    });

    let mut builder = Response::builder().status(StatusCode::SWITCHING_PROTOCOLS);
    for (name, value) in accept.headers() {
        builder = builder.header(name, value);
    }
    builder.body(Full::new(Bytes::new())).unwrap_or_else(|_| {
        error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            error_code::SERVER_ERROR,
            "upgrade response",
        )
    })
}

/// The number of watching clients, released however the task ends.
struct ClientGuard {
    clients: Arc<AtomicUsize>,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.clients.fetch_sub(1, Ordering::SeqCst);
    }
}

/// One client verb, as it arrives *up* the event stream.
///
/// These are not commands: a command is an HTTP POST, where the reply can say
/// why it was refused. These are facts about the socket itself — a display
/// announcing that it is on, and a client asking for a fresh snapshot.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", content = "payload")]
enum ClientMessage {
    #[serde(rename = "request.snapshot")]
    RequestSnapshot,
    #[serde(rename = "display.hello")]
    DisplayHello(DisplayHello),
    #[serde(rename = "display.heartbeat")]
    DisplayHeartbeat(DisplayHeartbeat),
    /// Anything this build does not know. Ignored rather than closing the
    /// socket: a newer client's extra verb is not a reason to stop showing the
    /// map.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct DisplayHeartbeat {
    id: String,
}

async fn serve_websocket(
    mut socket: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    hub: ControlHub,
    mut events: broadcast::Receiver<Event>,
    grant: super::devices::Grant,
) {
    if send_event(&mut socket, hub.snapshot_event()).await.is_err() {
        return;
    }
    let mut attached: Option<String> = None;
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => {
                    if send_event(&mut socket, event).await.is_err() {
                        break;
                    }
                }
                // A subscriber that fell behind is resynchronised with a whole
                // snapshot rather than a guessed patch. The revision in it is
                // the truth; anything older is advice.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if send_event(&mut socket, hub.snapshot_event()).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    let Ok(message) = serde_json::from_str::<ClientMessage>(text.as_str()) else {
                        continue;
                    };
                    match message {
                        ClientMessage::RequestSnapshot => {
                            if send_event(&mut socket, hub.snapshot_event()).await.is_err() {
                                break;
                            }
                        }
                        ClientMessage::DisplayHello(hello) => {
                            let reply = attach_display(&hub, &grant, hello);
                            if send_event(&mut socket, reply.clone()).await.is_err() {
                                break;
                            }
                            attached = match reply {
                                Event::DisplayAttached { id } => Some(id),
                                _ => attached,
                            };
                            // The snapshot carries the new display list, so a
                            // client that just attached does not have to guess
                            // whether it was counted.
                            if send_event(&mut socket, hub.snapshot_event()).await.is_err() {
                                break;
                            }
                        }
                        ClientMessage::DisplayHeartbeat(beat) => {
                            if attached.as_deref() == Some(beat.id.as_str()) {
                                hub.touch_display(&beat.id);
                            }
                        }
                        ClientMessage::Unknown => {}
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(_)) => break,
            },
        }
    }
    // The attachment dies with the socket. That is the whole point of putting
    // it here: a closed window cannot keep claiming to be showing something.
    if let Some(id) = attached {
        hub.detach_display(&id);
    }
}

/// Answer a display's handshake. A command center is a display the server can
/// see; only the operator's own credential may claim to be one.
fn attach_display(
    hub: &ControlHub,
    grant: &super::devices::Grant,
    hello: DisplayHello,
) -> Event {
    if !grant.allows(Scope::Admin) {
        return Event::DisplayRejected {
            code: error_code::FORBIDDEN.to_string(),
            message: format!(
                "a {} display needs the admin scope; this credential carries {:?}",
                hello.role, grant.scopes
            ),
        };
    }
    if let Err(message) = hello.validate() {
        return Event::DisplayRejected {
            code: error_code::INVALID_COMMAND.to_string(),
            message,
        };
    }
    let id = hello.display_id();
    hub.attach_display(DisplayState {
        label: hello.label_or_id(),
        role: COMMAND_CENTER_ROLE.to_string(),
        viewport: hello.viewport,
        id: id.clone(),
    });
    Event::DisplayAttached { id }
}

async fn send_event(
    socket: &mut WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
    event: Event,
) -> Result<(), ()> {
    let text = serde_json::to_string(&event).map_err(|_| ())?;
    socket.send(Message::text(text)).await.map_err(|_| ())
}

fn is_websocket(request: &Request<Incoming>) -> bool {
    let upgrade = request
        .headers()
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
    let connection = request
        .headers()
        .get(CONNECTION)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        });
    upgrade && connection
}

fn json_response(status: StatusCode, value: &serde_json::Value) -> Response<Full<Bytes>> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(body)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::from_static(b"{}"))))
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response<Full<Bytes>> {
    json_response(
        status,
        &serde_json::json!({ "error": { "code": code, "message": message } }),
    )
}

fn not_found() -> Response<Full<Bytes>> {
    error_response(
        StatusCode::NOT_FOUND,
        error_code::NOT_FOUND,
        "route not found",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::protocol::{CameraSnapshot, GeoPoint};
    use std::time::Instant;
    use tungstenite::client::IntoClientRequest;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn camera(latitude: f64, zoom: f64) -> CameraSnapshot {
        CameraSnapshot {
            center: GeoPoint {
                latitude,
                longitude: 106.8,
            },
            zoom,
            width: 800,
            height: 600,
        }
    }

    /// A host with no GPUI: the hub, a channel, and a server on a free port.
    /// This is the fake host the protocol tests run against.
    struct FakeHost {
        handle: ServerHandle,
        hub: ControlHub,
        commands: std::sync::mpsc::Receiver<AcceptedCommand>,
        http: reqwest::blocking::Client,
    }

    impl FakeHost {
        fn start() -> Self {
            let (command_tx, commands) = std::sync::mpsc::channel();
            let hub = ControlHub::new(camera(-6.2, 11.0));
            let handle = spawn_server(
                ServerConfig {
                    listen: "127.0.0.1:0".to_string(),
                    token: TOKEN.to_string(),
                    devices: None,
                },
                command_tx,
                hub.clone(),
            )
            .expect("server starts");
            Self {
                handle,
                hub,
                commands,
                http: reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .build()
                    .expect("client"),
            }
        }

        fn get(&self, path: &str) -> reqwest::blocking::Response {
            self.http
                .get(format!("{}{path}", self.handle.url()))
                .bearer_auth(TOKEN)
                .send()
                .expect("request")
        }

        fn post(&self, body: serde_json::Value) -> reqwest::blocking::Response {
            self.http
                .post(format!("{}/v1/commands", self.handle.url()))
                .bearer_auth(TOKEN)
                .json(&body)
                .send()
                .expect("request")
        }

        /// A request with a credential other than the operator's: a paired
        /// device, a revoked one, a watching one, or none at all.
        fn call(
            &self,
            method: &str,
            token: Option<&str>,
            path: &str,
            body: Option<serde_json::Value>,
        ) -> reqwest::blocking::Response {
            let url = format!("{}{path}", self.handle.url());
            let request = match method {
                "GET" => self.http.get(url),
                "POST" => self.http.post(url),
                "PUT" => self.http.put(url),
                "DELETE" => self.http.delete(url),
                other => panic!("unsupported method {other}"),
            };
            let request = match token {
                Some(token) => request.bearer_auth(token),
                None => request,
            };
            let request = match body {
                Some(body) => request.json(&body),
                None => request,
            };
            request.send().expect("request")
        }

        /// Open a pairing window as the operator and pair a device with it.
        fn pair_device(&self, label: &str, scopes: &[&str]) -> (String, String) {
            let offer = self.call(
                "POST",
                Some(TOKEN),
                "/v1/pairing",
                Some(serde_json::json!({ "scopes": scopes })),
            );
            assert_eq!(offer.status(), 200, "pairing window opens");
            let offer: serde_json::Value = offer.json().expect("json");
            let paired = self.call(
                "POST",
                None,
                "/v1/pair",
                Some(serde_json::json!({ "code": offer["code"], "label": label })),
            );
            assert_eq!(paired.status(), 200, "the code pairs a device");
            let paired: serde_json::Value = paired.json().expect("json");
            (
                paired["device"]["id"].as_str().expect("id").to_string(),
                paired["token"].as_str().expect("token").to_string(),
            )
        }

        fn connect_ws(
            &self,
        ) -> tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>
        {
            self.connect_ws_as(TOKEN)
        }

        fn connect_ws_as(
            &self,
            token: &str,
        ) -> tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>
        {
            let url = format!("{}/v1/ws", self.handle.url().replace("http://", "ws://"));
            let mut request = url.into_client_request().expect("ws request");
            request.headers_mut().insert(
                hyper::header::AUTHORIZATION,
                hyper::header::HeaderValue::from_str(&format!("Bearer {token}")).expect("header"),
            );
            let (socket, _response) = tungstenite::connect(request).expect("ws connect");
            socket
        }

        /// Publish a projection the way the console does.
        fn put_session(&self, body: &serde_json::Value) -> reqwest::blocking::Response {
            self.call("PUT", Some(TOKEN), "/v1/session", Some(body.clone()))
        }

        /// `/v1/session`, parsed: the readiness verdict and the session.
        fn session(&self) -> serde_json::Value {
            let response = self.get("/v1/session");
            assert_eq!(response.status(), 200);
            response.json().expect("json")
        }
    }

    fn next_event(
        socket: &mut tungstenite::WebSocket<
            tungstenite::stream::MaybeTlsStream<std::net::TcpStream>,
        >,
        deadline: Duration,
    ) -> Event {
        let started = Instant::now();
        loop {
            let message = socket.read().expect("ws read");
            if let tungstenite::Message::Text(text) = message {
                return serde_json::from_str(text.as_str()).expect("event parses");
            }
            assert!(started.elapsed() < deadline, "no event before the deadline");
        }
    }

    #[test]
    fn hello_is_public_and_names_the_protocol() {
        let host = FakeHost::start();
        let response = host
            .http
            .get(format!("{}/v1/hello", host.handle.url()))
            .send()
            .expect("request");
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = response.json().expect("json");
        assert_eq!(body["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(body["api_version"], "v1");
    }

    #[test]
    fn state_requires_the_bearer_token() {
        let host = FakeHost::start();
        let unauthorized = host
            .http
            .get(format!("{}/v1/state", host.handle.url()))
            .send()
            .expect("request");
        assert_eq!(unauthorized.status(), 401);
        let body: serde_json::Value = unauthorized.json().expect("json");
        assert_eq!(body["error"]["code"], "unauthorized");

        let authorized = host.get("/v1/state");
        assert_eq!(authorized.status(), 200);
        let body: serde_json::Value = authorized.json().expect("json");
        assert_eq!(body["type"], "snapshot");
        assert_eq!(body["protocol_version"], PROTOCOL_VERSION);
        assert_eq!(body["views"]["main"]["camera"]["zoom"], 11.0);
    }

    #[test]
    fn a_camera_command_reaches_the_host_once_and_is_acknowledged() {
        let host = FakeHost::start();
        let mut socket = host.connect_ws();
        assert!(matches!(
            next_event(&mut socket, Duration::from_secs(2)),
            Event::Snapshot { .. }
        ));

        let response = host.post(serde_json::json!({
            "protocol_version": PROTOCOL_VERSION,
            "client_id": "test-cli",
            "command_id": "c-1",
            "type": "camera.nudge",
            "payload": { "dx_fraction": 0.25, "dy_fraction": 0.0 }
        }));
        assert_eq!(response.status(), 202);
        let body: serde_json::Value = response.json().expect("json");
        assert_eq!(body["status"], "accepted");

        let accepted = host.commands.try_recv().expect("the host got the command");
        assert_eq!(accepted.client_id, "test-cli");
        assert_eq!(accepted.command_id, "c-1");
        assert_eq!(
            accepted.command,
            crate::control::protocol::Command::CameraNudge {
                dx_fraction: 0.25,
                dy_fraction: 0.0
            }
        );
        assert!(
            host.commands.try_recv().is_err(),
            "the command was delivered twice"
        );

        match next_event(&mut socket, Duration::from_secs(2)) {
            Event::CommandAccepted { command_id, .. } => assert_eq!(command_id, "c-1"),
            other => panic!("expected an acknowledgement, got {other:?}"),
        }
    }

    #[test]
    fn a_retried_command_id_is_answered_without_reaching_the_host_again() {
        let host = FakeHost::start();
        let body = serde_json::json!({
            "command_id": "c-retry",
            "type": "camera.zoom",
            "payload": { "delta": 1.0 }
        });
        assert_eq!(host.post(body.clone()).status(), 202);
        let retry = host.post(body);
        assert_eq!(retry.status(), 200);
        let value: serde_json::Value = retry.json().expect("json");
        assert_eq!(value["status"], "duplicate");
        assert!(host.commands.try_recv().is_ok());
        assert!(
            host.commands.try_recv().is_err(),
            "a retry moved the camera twice"
        );
    }

    #[test]
    fn malformed_and_impossible_commands_are_refused_with_codes() {
        let host = FakeHost::start();

        let malformed = host
            .http
            .post(format!("{}/v1/commands", host.handle.url()))
            .bearer_auth(TOKEN)
            .body("{not json")
            .send()
            .expect("request");
        assert_eq!(malformed.status(), 400);
        let value: serde_json::Value = malformed.json().expect("json");
        assert_eq!(value["error"]["code"], "invalid_json");

        let impossible = host.post(serde_json::json!({
            "command_id": "c-bad",
            "type": "camera.set",
            "payload": { "center": { "latitude": 91.0, "longitude": 0.0 }, "zoom": 4.0 }
        }));
        assert_eq!(impossible.status(), 422);
        let value: serde_json::Value = impossible.json().expect("json");
        assert_eq!(value["error"]["code"], "invalid_command");

        let future = host.post(serde_json::json!({
            "command_id": "c-future",
            "protocol_version": 99,
            "type": "camera.zoom",
            "payload": { "delta": 1.0 }
        }));
        assert_eq!(future.status(), 409);
        assert!(host.commands.try_recv().is_err());
    }

    #[test]
    fn a_camera_change_is_pushed_to_a_watching_client() {
        let host = FakeHost::start();
        let mut socket = host.connect_ws();
        assert!(matches!(
            next_event(&mut socket, Duration::from_secs(2)),
            Event::Snapshot { .. }
        ));

        let revision = host
            .hub
            .report_camera(
                camera(-6.1, 11.5),
                crate::control::protocol::CameraSource {
                    client_id: "phone".to_string(),
                    command_id: Some("c-9".to_string()),
                },
            )
            .expect("a change");
        assert_eq!(revision, 1);

        match next_event(&mut socket, Duration::from_secs(2)) {
            Event::CameraChanged {
                revision,
                source,
                command_id,
                view,
            } => {
                assert_eq!(revision, 1);
                assert_eq!(source.as_deref(), Some("phone"));
                assert_eq!(command_id.as_deref(), Some("c-9"));
                assert_eq!(view.camera.zoom, 11.5);
            }
            other => panic!("expected a camera event, got {other:?}"),
        }
    }

    #[test]
    fn a_watching_client_can_ask_for_a_fresh_snapshot() {
        let host = FakeHost::start();
        let mut socket = host.connect_ws();
        assert!(matches!(
            next_event(&mut socket, Duration::from_secs(2)),
            Event::Snapshot { .. }
        ));
        host.hub.report_camera(
            camera(-6.0, 12.0),
            crate::control::protocol::CameraSource::desktop(),
        );
        socket
            .send(tungstenite::Message::text(r#"{"type":"request.snapshot"}"#))
            .expect("send");
        let mut last = None;
        for _ in 0..3 {
            if let Event::Snapshot {
                revision, views, ..
            } = next_event(&mut socket, Duration::from_secs(2))
            {
                last = Some((revision, views));
                break;
            }
        }
        let (revision, views) = last.expect("a snapshot arrived");
        assert_eq!(revision, 1);
        assert_eq!(views["main"].camera.zoom, 12.0);
    }

    /// The phone opens a window: the observer UI is server state, so every
    /// attached wall draws it and a reconnect restores it.
    #[test]
    fn the_observer_ui_is_a_command_and_a_read() {
        let host = FakeHost::start();
        host.put_session(&session_json("execution"));
        // Readiness needs a display; attach one through the hub.
        host.hub.attach_display(crate::control::DisplayState {
            id: "wall-1".to_string(),
            label: "wall-1".to_string(),
            role: "command_center".to_string(),
            viewport: None,
        });

        let opened = host.post(serde_json::json!({
            "protocol_version": 2,
            "client_id": "phone-7",
            "view": "observer",
            "type": "ui.set",
            "payload": { "panel": "crew", "open": true }
        }));
        assert_eq!(opened.status(), 202, "the window opens");

        let read = host.session();
        assert_eq!(read["ui"]["panels"]["crew"], true);
        assert_eq!(read["ui"]["panels"]["comms"], false);

        // A panel the observer does not have is refused at the boundary.
        let bad = host.post(serde_json::json!({
            "protocol_version": 2,
            "view": "observer",
            "type": "ui.set",
            "payload": { "panel": "units", "open": true }
        }));
        assert_eq!(bad.status(), 422, "there is no `units` window");
    }

    /// The wall fetches the hull picture the console published, and a phone
    /// may read it; writing one needs the operator.
    #[test]
    fn a_hull_picture_is_published_and_served() {
        let host = FakeHost::start();
        // A 1x1 PNG: the smallest real one.
        let png: Vec<u8> = vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ];
        let put = host
            .http
            .put(format!("{}/v1/units/1/picture", host.handle.url()))
            .bearer_auth(TOKEN)
            .header("content-type", "image/png")
            .body(png.clone())
            .send()
            .expect("request");
        assert_eq!(put.status(), 200);
        let got = host.get("/v1/units/1/picture");
        assert_eq!(got.status(), 200);
        assert_eq!(got.headers()["content-type"], "image/png");
        assert_eq!(got.bytes().expect("bytes").as_ref(), png.as_slice());

        // A paired phone may read it and may not replace it.
        let (_, token) = host.pair_device("phone", &["observe", "control_view"]);
        let read = host.call("GET", Some(&token), "/v1/units/1/picture", None);
        assert_eq!(read.status(), 200);
        let write = host
            .http
            .put(format!("{}/v1/units/1/picture", host.handle.url()))
            .bearer_auth(&token)
            .header("content-type", "image/png")
            .body(png)
            .send()
            .expect("request");
        assert_eq!(write.status(), 403, "a phone does not publish pictures");

        // A body that is not a PNG is refused rather than served later.
        let not_png = host
            .http
            .put(format!("{}/v1/units/1/picture", host.handle.url()))
            .bearer_auth(TOKEN)
            .body(b"not a png".to_vec())
            .send()
            .expect("request");
        assert_eq!(not_png.status(), 422);
        assert_eq!(host.get("/v1/units/9/picture").status(), 404);
    }

    fn session_json(phase: &str) -> serde_json::Value {
        serde_json::json!({
            "game_id": 42,
            "name": "Sunda Strait Watch",
            "phase": phase,
            "mode": "maneuver",
            "units": [
                {
                    "id": 1,
                    "name": "Evertsen",
                    "hull_number": "F805",
                    "class_name": "Frigate",
                    "position": { "latitude": -6.2, "longitude": 106.8 }
                },
                {
                    "id": 2,
                    "name": "Tromp",
                    "hull_number": "F801",
                    "class_name": "Frigate",
                    "position": { "latitude": -5.9, "longitude": 106.9 }
                }
            ]
        })
    }

    /// Say hello as a display and read until the server answers the handshake.
    fn hello_as(
        socket: &mut tungstenite::WebSocket<
            tungstenite::stream::MaybeTlsStream<std::net::TcpStream>,
        >,
        id: &str,
    ) -> Event {
        socket
            .send(tungstenite::Message::text(
                serde_json::json!({
                    "type": "display.hello",
                    "payload": {
                        "id": id,
                        "role": "command_center",
                        "label": id,
                        "viewport": { "width": 1920, "height": 1080 }
                    }
                })
                .to_string(),
            ))
            .expect("send hello");
        for _ in 0..8 {
            match next_event(socket, Duration::from_secs(2)) {
                event @ (Event::DisplayAttached { .. } | Event::DisplayRejected { .. }) => {
                    return event;
                }
                _ => {}
            }
        }
        panic!("the server never answered the display handshake");
    }

    fn wait_until(mut done: impl FnMut() -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(5) {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    #[test]
    fn a_session_is_published_numbered_and_read_back() {
        let host = FakeHost::start();
        let response = host.put_session(&session_json("execution"));
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = response.json().expect("json");
        assert_eq!(body["status"], "changed");
        assert_eq!(body["readiness"]["reason"], "no_display");

        let state = host.session();
        assert_eq!(state["session"]["name"], "Sunda Strait Watch");
        assert_eq!(state["session"]["units"][0]["index"], 1);
        assert_eq!(state["session"]["units"][1]["index"], 2);
        assert_eq!(state["session"]["units"][1]["id"], 2);
    }

    #[test]
    fn republishing_the_same_session_is_a_heartbeat() {
        let host = FakeHost::start();
        host.put_session(&session_json("execution"));
        let revision = host.hub.revision();
        let response = host.put_session(&session_json("execution"));
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = response.json().expect("json");
        assert_eq!(body["status"], "unchanged");
        assert_eq!(body["revision"], revision);
    }

    #[test]
    fn a_session_in_planning_does_not_open_observation() {
        let host = FakeHost::start();
        host.put_session(&session_json("planning"));
        assert_eq!(host.session()["readiness"]["reason"], "not_execution");
    }

    #[test]
    fn an_impossible_session_is_refused() {
        let host = FakeHost::start();
        let mut body = session_json("execution");
        body["units"][1]["id"] = serde_json::json!(1);
        let response = host.put_session(&body);
        assert_eq!(response.status(), 422);
        let error: serde_json::Value = response.json().expect("json");
        assert_eq!(error["error"]["code"], "invalid_command");
    }

    #[test]
    fn a_paired_device_cannot_publish_a_session() {
        let host = FakeHost::start();
        let (_, token) = host.pair_device("phone", &["observe", "control_view"]);
        let response = host.call(
            "PUT",
            Some(&token),
            "/v1/session",
            Some(session_json("execution")),
        );
        assert_eq!(response.status(), 403);
        assert_eq!(host.session()["session"], serde_json::Value::Null);
    }

    #[test]
    fn a_session_can_be_cleared() {
        let host = FakeHost::start();
        host.put_session(&session_json("execution"));
        let response = host.call("DELETE", Some(TOKEN), "/v1/session", None);
        assert_eq!(response.status(), 200);
        let state = host.session();
        assert_eq!(state["session"], serde_json::Value::Null);
        assert_eq!(state["readiness"]["reason"], "no_session");
    }

    #[test]
    fn a_display_handshake_opens_observation() {
        let host = FakeHost::start();
        host.put_session(&session_json("execution"));
        assert_eq!(host.session()["readiness"]["reason"], "no_display");

        let mut socket = host.connect_ws();
        // The snapshot, then the handshake answer.
        assert!(matches!(
            next_event(&mut socket, Duration::from_secs(2)),
            Event::Snapshot { .. }
        ));
        match hello_as(&mut socket, "wall-1") {
            Event::DisplayAttached { id } => assert_eq!(id, "wall-1"),
            other => panic!("expected an attachment, got {other:?}"),
        }
        assert!(wait_until(|| host.hub.readiness().ready));
        // The first command center creates the observer view from the
        // operator's camera, and does not move it.
        assert_eq!(host.hub.camera_of(crate::control::OBSERVER_VIEW), Some(camera(-6.2, 11.0)));
        assert_eq!(host.hub.camera(), camera(-6.2, 11.0));

        let displays = host.get("/v1/displays");
        assert_eq!(displays.status(), 200);
        let displays: serde_json::Value = displays.json().expect("json");
        assert_eq!(displays["displays"][0]["id"], "wall-1");
        assert_eq!(displays["displays"][0]["viewport"]["width"], 1920);
    }

    #[test]
    fn a_phone_cannot_claim_to_be_a_command_center() {
        let host = FakeHost::start();
        host.put_session(&session_json("execution"));
        let (_, token) = host.pair_device("phone", &["observe", "control_view"]);
        let mut socket = host.connect_ws_as(&token);
        assert!(matches!(
            next_event(&mut socket, Duration::from_secs(2)),
            Event::Snapshot { .. }
        ));
        match hello_as(&mut socket, "impostor") {
            Event::DisplayRejected { code, .. } => assert_eq!(code, "forbidden"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert_eq!(host.session()["readiness"]["reason"], "no_display");
    }

    #[test]
    fn a_display_that_disconnects_stops_counting() {
        let host = FakeHost::start();
        host.put_session(&session_json("execution"));
        let mut socket = host.connect_ws();
        assert!(matches!(
            next_event(&mut socket, Duration::from_secs(2)),
            Event::Snapshot { .. }
        ));
        hello_as(&mut socket, "wall-1");
        assert!(wait_until(|| host.hub.readiness().ready));

        drop(socket);
        assert!(
            wait_until(|| host.hub.readiness().reason
                == Some(crate::control::ReadinessReason::NoDisplay)),
            "the display stayed attached after its socket closed"
        );
        // The observer camera is kept: a command center that reconnects before
        // the next session is not thrown back to a default place.
        assert!(host.hub.camera_of(crate::control::OBSERVER_VIEW).is_some());
    }

    #[test]
    fn a_focus_moves_the_observer_and_never_the_console() {
        let host = FakeHost::start();
        host.put_session(&session_json("execution"));
        let mut socket = host.connect_ws();
        assert!(matches!(
            next_event(&mut socket, Duration::from_secs(2)),
            Event::Snapshot { .. }
        ));
        hello_as(&mut socket, "wall-1");
        assert!(wait_until(|| host.hub.readiness().ready));

        let response = host.post(serde_json::json!({
            "protocol_version": PROTOCOL_VERSION,
            "view": "observer",
            "client_id": "phone",
            "command_id": "focus-1",
            "type": "camera.focus",
            "payload": { "unit_id": 2, "zoom": 13.0 }
        }));
        assert_eq!(response.status(), 202);
        let body: serde_json::Value = response.json().expect("json");
        assert_eq!(body["view"], "observer");

        let observer = host
            .hub
            .camera_of(crate::control::OBSERVER_VIEW)
            .expect("observer");
        assert!((observer.center.latitude + 5.9).abs() < 1e-9);
        assert_eq!(observer.zoom, 13.0);
        // The game master's screen did not move, and the command never reached
        // the desktop's channel.
        assert_eq!(host.hub.camera(), camera(-6.2, 11.0));
        assert!(
            host.commands.try_recv().is_err(),
            "an observer command reached the console"
        );
    }

    #[test]
    fn an_observer_command_waits_for_readiness() {
        let host = FakeHost::start();
        let response = host.post(serde_json::json!({
            "protocol_version": PROTOCOL_VERSION,
            "view": "observer",
            "client_id": "phone",
            "command_id": "focus-1",
            "type": "camera.focus",
            "payload": { "unit_id": 1 }
        }));
        assert_eq!(response.status(), 409);
        let body: serde_json::Value = response.json().expect("json");
        assert_eq!(body["error"]["code"], "not_ready");
        assert!(host.hub.camera_of(crate::control::OBSERVER_VIEW).is_none());
    }

    #[test]
    fn a_viewport_report_is_allowed_before_observation_opens() {
        // A command center says how big it is the moment it attaches; that
        // cannot wait for a session, and it moves no camera.
        let host = FakeHost::start();
        let response = host.post(serde_json::json!({
            "protocol_version": PROTOCOL_VERSION,
            "view": "observer",
            "client_id": "wall-1",
            "command_id": "viewport-1",
            "type": "view.report",
            "payload": { "width": 1920, "height": 1080 }
        }));
        assert_eq!(response.status(), 202);
        let observer = host
            .hub
            .camera_of(crate::control::OBSERVER_VIEW)
            .expect("observer");
        assert_eq!((observer.width, observer.height), (1920, 1080));
    }

    #[test]
    fn a_focus_on_a_unit_nobody_has_is_a_422() {
        let host = FakeHost::start();
        host.put_session(&session_json("execution"));
        let mut socket = host.connect_ws();
        assert!(matches!(
            next_event(&mut socket, Duration::from_secs(2)),
            Event::Snapshot { .. }
        ));
        hello_as(&mut socket, "wall-1");
        assert!(wait_until(|| host.hub.readiness().ready));

        let response = host.post(serde_json::json!({
            "protocol_version": PROTOCOL_VERSION,
            "view": "observer",
            "client_id": "phone",
            "command_id": "focus-9",
            "type": "camera.focus",
            "payload": { "unit_id": 99 }
        }));
        assert_eq!(response.status(), 422);
        let body: serde_json::Value = response.json().expect("json");
        assert_eq!(body["error"]["code"], "invalid_command");
    }

    #[test]
    fn a_snapshot_carries_the_session_and_the_verdict() {
        let host = FakeHost::start();
        host.put_session(&session_json("execution"));
        let mut socket = host.connect_ws();
        match next_event(&mut socket, Duration::from_secs(2)) {
            Event::Snapshot {
                session,
                readiness,
                displays,
                ..
            } => {
                let session = session.expect("a session");
                assert_eq!(session.game_id, 42);
                assert!(session.is_execution());
                assert_eq!(session.units.len(), 2);
                assert_eq!(readiness.reason, Some(crate::control::ReadinessReason::NoDisplay));
                assert!(displays.is_empty());
            }
            other => panic!("expected a snapshot, got {other:?}"),
        }
    }

    #[test]
    fn a_display_rejection_does_not_close_the_socket() {
        // A client that mislabels itself can retry with a different role; a
        // closed socket would make that a reconnect for a typo.
        let host = FakeHost::start();
        let mut socket = host.connect_ws();
        assert!(matches!(
            next_event(&mut socket, Duration::from_secs(2)),
            Event::Snapshot { .. }
        ));
        socket
            .send(tungstenite::Message::text(
                serde_json::json!({
                    "type": "display.hello",
                    "payload": { "id": "x", "role": "coffee_machine" }
                })
                .to_string(),
            ))
            .expect("send");
        let mut rejected = false;
        for _ in 0..8 {
            match next_event(&mut socket, Duration::from_secs(2)) {
                Event::DisplayRejected { code, .. } => {
                    assert_eq!(code, "invalid_command");
                    rejected = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(rejected, "the bad role was not refused");
        match hello_as(&mut socket, "wall-1") {
            Event::DisplayAttached { .. } => {}
            other => panic!("the socket did not survive the refusal: {other:?}"),
        }
    }

    #[test]
    fn the_camera_route_names_the_view_or_refuses() {
        let host = FakeHost::start();
        let response = host.get("/v1/views/main/camera");
        assert_eq!(response.status(), 200);
        let body: serde_json::Value = response.json().expect("json");
        assert_eq!(body["id"], "main");
        assert_eq!(host.get("/v1/views/other/camera").status(), 404);
        assert_eq!(host.get("/v1/nothing").status(), 404);
    }

    #[test]
    fn a_websocket_without_a_token_is_refused_before_the_upgrade() {
        let host = FakeHost::start();
        let url = format!("{}/v1/ws", host.handle.url().replace("http://", "ws://"));
        let request = url.into_client_request().expect("ws request");
        let error = tungstenite::connect(request).expect_err("the handshake must fail");
        let rendered = format!("{error}");
        assert!(rendered.contains("401"), "unexpected refusal: {rendered}");
    }

    #[test]
    fn pairing_hands_out_a_device_token_that_works_and_is_not_the_operators() {
        let host = FakeHost::start();
        let (device, token) = host.pair_device("phone", &["observe", "control_view"]);
        assert_ne!(token, TOKEN);
        assert_eq!(token.len(), 64);

        let state = host.call("GET", Some(&token), "/v1/state", None);
        assert_eq!(state.status(), 200, "a paired device can read the state");

        let me = host.call("GET", Some(&token), "/v1/whoami", None);
        let me: serde_json::Value = me.json().expect("json");
        assert_eq!(me["client"], device);
        assert_eq!(me["label"], "phone");
        assert_eq!(me["scopes"], serde_json::json!(["observe", "control_view"]));
    }

    #[test]
    fn a_watching_device_can_see_the_map_and_cannot_move_it() {
        let host = FakeHost::start();
        let (_, token) = host.pair_device("wall display", &["observe"]);
        let steer = host.call(
            "POST",
            Some(&token),
            "/v1/commands",
            Some(serde_json::json!({
                "command_id": "c-scope",
                "type": "camera.zoom",
                "payload": { "delta": 1.0 }
            })),
        );
        assert_eq!(steer.status(), 403);
        let body: serde_json::Value = steer.json().expect("json");
        assert_eq!(body["error"]["code"], "forbidden");
        assert!(
            host.commands.try_recv().is_err(),
            "a forbidden command reached the host"
        );
    }

    #[test]
    fn a_paired_device_cannot_open_a_pairing_window() {
        let host = FakeHost::start();
        let (_, token) = host.pair_device("phone", &["observe", "control_view"]);
        let response = host.call(
            "POST",
            Some(&token),
            "/v1/pairing",
            Some(serde_json::json!({})),
        );
        assert_eq!(response.status(), 403);
    }

    #[test]
    fn a_pairing_window_refuses_a_code_that_is_not_the_one_on_screen() {
        let host = FakeHost::start();
        let offer = host.call("POST", Some(TOKEN), "/v1/pairing", None);
        assert_eq!(offer.status(), 200);
        let wrong = host.call(
            "POST",
            None,
            "/v1/pair",
            Some(serde_json::json!({ "code": "ZZZZ-ZZZZ" })),
        );
        assert_eq!(wrong.status(), 401);
        let body: serde_json::Value = wrong.json().expect("json");
        assert_eq!(body["error"]["code"], "invalid_pairing_code");
    }

    #[test]
    fn a_pairing_window_that_was_never_opened_refuses_every_code() {
        let host = FakeHost::start();
        let response = host.call(
            "POST",
            None,
            "/v1/pair",
            Some(serde_json::json!({ "code": "K7QM-3XPD" })),
        );
        assert_eq!(response.status(), 404);
        let body: serde_json::Value = response.json().expect("json");
        assert_eq!(body["error"]["code"], "no_pairing_window");
    }

    #[test]
    fn a_device_can_be_listed_revoked_and_then_it_is_nobody() {
        let host = FakeHost::start();
        let (device, token) = host.pair_device("phone", &["observe"]);

        let list = host.call("GET", Some(TOKEN), "/v1/devices", None);
        let list: serde_json::Value = list.json().expect("json");
        assert_eq!(list["devices"][0]["id"], device);
        assert!(
            list["devices"][0].get("token_hash").is_none(),
            "the device list must not leak a stored hash"
        );

        let revoked = host.call(
            "DELETE",
            Some(TOKEN),
            &format!("/v1/devices/{device}"),
            None,
        );
        assert_eq!(revoked.status(), 200);
        assert_eq!(
            host.call("GET", Some(&token), "/v1/state", None).status(),
            401,
            "a revoked token still worked"
        );
        assert_eq!(
            host.call(
                "DELETE",
                Some(TOKEN),
                &format!("/v1/devices/{device}"),
                None
            )
            .status(),
            404
        );
    }

    #[test]
    fn a_device_rotates_its_own_token_and_the_old_one_dies() {
        let host = FakeHost::start();
        let (_, token) = host.pair_device("laptop", &["observe"]);
        let rotated = host.call("POST", Some(&token), "/v1/auth/rotate", None);
        assert_eq!(rotated.status(), 200);
        let rotated: serde_json::Value = rotated.json().expect("json");
        let fresh = rotated["token"].as_str().expect("token").to_string();
        assert_ne!(fresh, token);
        assert_eq!(
            host.call("GET", Some(&token), "/v1/state", None).status(),
            401
        );
        assert_eq!(
            host.call("GET", Some(&fresh), "/v1/state", None).status(),
            200
        );
    }

    #[test]
    fn the_operator_token_is_rotated_in_the_environment_not_over_http() {
        let host = FakeHost::start();
        let response = host.call("POST", Some(TOKEN), "/v1/auth/rotate", None);
        assert_eq!(response.status(), 422);
        let body: serde_json::Value = response.json().expect("json");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap_or("")
                .contains("TFG_CONTROL_TOKEN")
        );
    }

    #[test]
    fn pairing_will_not_mint_a_device_that_could_pair_others() {
        let host = FakeHost::start();
        let response = host.call(
            "POST",
            Some(TOKEN),
            "/v1/pairing",
            Some(serde_json::json!({ "scopes": ["observe", "admin"] })),
        );
        assert_eq!(response.status(), 422);
    }

    #[test]
    fn hello_says_whether_a_pairing_window_is_open() {
        let host = FakeHost::start();
        let hello: serde_json::Value = host
            .http
            .get(format!("{}/v1/hello", host.handle.url()))
            .send()
            .expect("request")
            .json()
            .expect("json");
        assert_eq!(hello["pairing_open"], false);
        let _ = host.call("POST", Some(TOKEN), "/v1/pairing", None);
        let hello: serde_json::Value = host
            .http
            .get(format!("{}/v1/hello", host.handle.url()))
            .send()
            .expect("request")
            .json()
            .expect("json");
        assert_eq!(hello["pairing_open"], true);
    }
}
