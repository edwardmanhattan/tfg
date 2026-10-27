//! The console as a *client*: follow a host's camera instead of hosting one.
//!
//! This is the mirror image of [`super::server`]. The console binds nothing and
//! opens no device table. It connects to a host over one WebSocket, applies the
//! host's camera with the same `apply_control_command` a hosted console uses,
//! and posts its own camera when the operator moves it.
//!
//! Two rules shape this file, and they are the same two the server keeps:
//!
//! - **No GPUI.** This thread only sends [`AcceptedCommand`]s down the same
//!   `std::sync::mpsc` channel the embedded server uses. The shell drains it on
//!   its own tick and moves the camera with the methods a mouse gesture uses.
//! - **The display is optimistic, the host is the record.** A gesture moves the
//!   camera locally at once and the client posts `camera.set` after the fact. A
//!   camera that arrives from the host is applied through the shell's reducer,
//!   and the resulting report from the shell is *never* posted back as a
//!   command: the host already knows the move it asked for. [`ClientHandle::followed`]
//!   carries those reports, and the `local` flag is what keeps them out of the
//!   outbound path.
//!
//! What the client adds on top of the raw protocol is coalescing, in two places:
//!
//! 1. A report equal to the camera the host already has is dropped before it
//!    becomes an HTTP request. The shell reports on every camera message, and
//!    most of those are the camera it already reported.
//! 2. A camera forwarded to the shell installs a *pending* value. Reports from
//!    ticks that predate the shell applying it are dropped, so a slow tick
//!    cannot post the old camera back and undo a remote move. The pending value
//!    clears when the shell reports the camera it was asked to show.
//!
//! Reconnection is the socket's problem, not the shell's: the client retries
//! with backoff, resets its revision on every connection, and only accepts
//! cameras again after a fresh snapshot. Reports made while unsynced are
//! dropped, because until the host has said where its camera is there is
//! nothing sensible to compare them against.

use crate::control::{
    AcceptedCommand, CameraSnapshot, Command, CommandRequest, Event, GeoPoint, MAIN_VIEW,
    PROTOCOL_VERSION, ViewState,
};
use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tungstenite::Message;
use tungstenite::client::IntoClientRequest;

/// How many camera reports may wait for the client thread.
///
/// The shell can produce a camera message faster than a gesture can be posted,
/// and only the newest one matters. A small queue lets the thread keep the
/// latest report while dropping the ones behind it; an unbounded channel would
/// let a stalled HTTP post grow without limit.
const OUTBOUND_QUEUE: usize = 4;

/// How long a failed host stays failed before each retry.
const BACKOFF_START: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(10);

/// How often a repeated connection or HTTP error is written to the log.
const ERROR_LOG_INTERVAL: Duration = Duration::from_secs(10);

/// What a display tells the client about itself.
#[derive(Debug, Clone)]
enum Report {
    /// The camera the shell is showing. `local` is false when a remote command
    /// asked for this camera: the host already knows about that move.
    Camera { camera: CameraSnapshot, local: bool },
    /// The display's size, without a camera. Kept separate for callers that
    /// learn the viewport before the first camera, and for tests.
    Viewport { width: u32, height: u32 },
}

/// Where the console should follow, and with what credential.
///
/// `new` is the operator's common case: a URL from `TFG_CONTROL_SERVER` and the
/// token from `TFG_CONTROL_TOKEN`. The timeout fields are public so a test (or
/// a slow link) can shorten them without another constructor.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub server: String,
    pub token: String,
    /// The name this console reports as. `source` in a `camera.changed` event
    /// is this string, so it is what lets the client recognise its own echo.
    pub client_id: String,
    pub connect_timeout: Duration,
    /// How long one socket read waits before the thread checks for outbound
    /// work and the stop flag. Not a protocol timeout.
    pub read_timeout: Duration,
    pub post_timeout: Duration,
}

impl ClientConfig {
    pub fn new(server: impl Into<String>, token: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            token: token.into(),
            client_id: format!("console-{}-{}", std::process::id(), next_client_number()),
            connect_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_millis(250),
            post_timeout: Duration::from_secs(3),
        }
    }
}

fn next_client_number() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A handle the shell-side runtime keeps and hands to the order thread.
///
/// Cloning is cheap and shares one connection; the client stops when the last
/// clone is dropped. Dropping joins the thread, so tests can start and stop
/// clients without leaving sockets behind.
#[derive(Clone)]
pub struct ClientHandle {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    outbound: SyncSender<Report>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl ClientHandle {
    /// The operator moved the camera, or the window resized. This is the only
    /// kind of report that may become a command.
    pub fn camera(&self, camera: CameraSnapshot) {
        self.send(Report::Camera {
            camera,
            local: true,
        });
    }

    /// The shell finished applying a camera the host asked for.
    ///
    /// The report clears the pending gate and may update the host's idea of the
    /// viewport, but it is never posted as a camera command: the host is where
    /// the camera came from.
    pub fn followed(&self, camera: CameraSnapshot) {
        self.send(Report::Camera {
            camera,
            local: false,
        });
    }

    /// The display's size, when a caller knows it separately from a camera.
    pub fn viewport(&self, width: u32, height: u32) {
        self.send(Report::Viewport { width, height });
    }

    fn send(&self, report: Report) {
        match self.inner.outbound.try_send(report) {
            // Full means the client is behind on HTTP. The next camera report
            // carries the current state, so dropping this one costs nothing
            // that lasts.
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let thread = self.thread.lock().ok().and_then(|mut slot| slot.take());
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

/// Start following `config.server`.
///
/// A malformed URL or an empty token is refused here, because those are launch
/// mistakes a log line should name before the window opens. A host that is not
/// up yet is not a launch mistake: the thread retries, and the console shows
/// its own camera until the first snapshot arrives.
pub fn spawn_client(
    config: ClientConfig,
    commands: Sender<AcceptedCommand>,
) -> Result<ClientHandle, String> {
    let endpoint = Endpoint::parse(&config.server)?;
    if config.token.trim().is_empty() {
        return Err("the control token is empty".to_string());
    }
    if config
        .token
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err("the control token contains whitespace or a control character".to_string());
    }
    let (outbound, reports) = std::sync::mpsc::sync_channel(OUTBOUND_QUEUE);
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let thread = std::thread::Builder::new()
        .name("tfg-control-client".to_string())
        .spawn(move || run(config, endpoint, reports, commands, thread_stop))
        .map_err(|error| format!("control client thread: {error}"))?;
    Ok(ClientHandle {
        inner: Arc::new(ClientInner {
            outbound,
            stop,
            thread: Mutex::new(Some(thread)),
        }),
    })
}

/// A server address split into the parts both transports need.
#[derive(Debug, Clone)]
struct Endpoint {
    host: String,
    port: u16,
    /// `http://host:port`, with no trailing slash.
    base: String,
}

impl Endpoint {
    fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim().trim_end_matches('/');
        let rest = if let Some(rest) = raw.strip_prefix("https://") {
            return Err(format!(
                "wss is not supported yet; point {rest} at an http server on the LAN"
            ));
        } else if let Some(rest) = raw.strip_prefix("http://") {
            rest
        } else {
            return Err(format!(
                "the control server must start with http://, got {raw}"
            ));
        };
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if !path.is_empty() {
            // The routes are appended to the base, so a path here would be
            // silently dropped and the client would talk to the wrong place.
            return Err(format!(
                "the control server URL should not carry a path, got /{path}"
            ));
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (
                host.to_string(),
                port.parse::<u16>()
                    .map_err(|_| format!("{port} is not a port"))?,
            ),
            None => (authority.to_string(), 80),
        };
        let host = host.trim_matches(['[', ']']).to_string();
        if host.is_empty() {
            return Err(format!("the control server URL has no host: {raw}"));
        }
        let authority = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        Ok(Self {
            base: format!("http://{authority}"),
            host,
            port,
        })
    }

    fn websocket_url(&self) -> String {
        format!("ws://{}/v1/ws", self.authority())
    }

    fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    fn commands_url(&self) -> String {
        format!("{}/v1/commands", self.base)
    }
}

/// Rate-limited stderr: a dead host should not write a line every 250 ms.
#[derive(Default)]
struct FailureLog {
    last: Option<Instant>,
    suppressed: u32,
}

impl FailureLog {
    fn log(&mut self, message: &str) {
        let now = Instant::now();
        let due = self
            .last
            .is_none_or(|at| now.duration_since(at) >= ERROR_LOG_INTERVAL);
        if !due {
            self.suppressed += 1;
            return;
        }
        if self.suppressed > 0 {
            eprintln!(
                "control client: {message} ({} similar messages suppressed)",
                self.suppressed
            );
        } else {
            eprintln!("control client: {message}");
        }
        self.last = Some(now);
        self.suppressed = 0;
    }

    fn recovered(&mut self) {
        if self.last.is_some() {
            eprintln!("control client: connected");
            self.last = None;
            self.suppressed = 0;
        }
    }
}

fn run(
    config: ClientConfig,
    endpoint: Endpoint,
    reports: Receiver<Report>,
    commands: Sender<AcceptedCommand>,
    stop: Arc<AtomicBool>,
) {
    let http = match reqwest::blocking::Client::builder()
        .timeout(config.post_timeout)
        .build()
    {
        Ok(http) => http,
        Err(error) => {
            eprintln!("control client disabled: building the HTTP client: {error}");
            return;
        }
    };
    let mut failures = FailureLog::default();
    let mut backoff = BACKOFF_START;
    while !stop.load(Ordering::SeqCst) {
        let started = Instant::now();
        let connection = connect(&config, &endpoint);
        let socket = match connection {
            Ok(socket) => {
                failures.recovered();
                socket
            }
            Err(error) => {
                failures.log(&format!("following {}: {error}", endpoint.base));
                if sleep_unless_stopped(&stop, backoff) {
                    return;
                }
                backoff = (backoff * 2).min(BACKOFF_MAX);
                continue;
            }
        };
        let mut session = Session {
            socket,
            endpoint: endpoint.clone(),
            token: config.token.clone(),
            client_id: config.client_id.clone(),
            synced: false,
            revision: 0,
            last_known: None,
            displayed: None,
            display_size: None,
            pending: None,
            counter: 0,
            log: FailureLog::default(),
        };
        match session.run(&http, &reports, &commands, &stop) {
            SessionEnd::Stopped => return,
            SessionEnd::Dropped(reason) => {
                failures.log(&format!("the event stream stopped: {reason}"));
            }
        }
        // A connection that lived long enough to carry a camera is healthy;
        // one that closed at once grows the backoff instead.
        if started.elapsed() >= Duration::from_secs(5) {
            backoff = BACKOFF_START;
        } else {
            backoff = (backoff * 2).min(BACKOFF_MAX);
        }
        if sleep_unless_stopped(&stop, backoff) {
            return;
        }
    }
}

/// Connect, authenticate, and leave the socket on a short read timeout.
///
/// The timeout is what lets one thread both watch the event stream and drain
/// the shell's reports: a read with nothing to read is a wake-up, not a block.
fn connect(
    config: &ClientConfig,
    endpoint: &Endpoint,
) -> Result<tungstenite::WebSocket<TcpStream>, String> {
    let mut request = endpoint
        .websocket_url()
        .into_client_request()
        .map_err(|error| format!("websocket request: {error}"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {}", config.token)
            .parse()
            .map_err(|_| "the token is not a valid header value".to_string())?,
    );
    let address = endpoint.authority();
    let mut last = None;
    for candidate in address
        .to_socket_addrs()
        .map_err(|error| format!("resolving {address}: {error}"))?
    {
        match TcpStream::connect_timeout(&candidate, config.connect_timeout) {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                // Handshake reads use the connect budget; the event loop
                // shortens this once the socket is open.
                let _ = stream.set_read_timeout(Some(config.connect_timeout));
                let socket = match tungstenite::client(request.clone(), stream) {
                    Ok((socket, _response)) => socket,
                    Err(error) => return Err(format!("websocket handshake: {error}")),
                };
                let _ = socket.get_ref().set_read_timeout(Some(config.read_timeout));
                return Ok(socket);
            }
            Err(error) => last = Some(error),
        }
    }
    Err(format!(
        "connecting to {address}: {}",
        last.map(|error| error.to_string())
            .unwrap_or_else(|| "no address resolved".to_string())
    ))
}

/// Sleep in slices so a drop is never waiting out a ten-second backoff.
fn sleep_unless_stopped(stop: &AtomicBool, total: Duration) -> bool {
    let step = Duration::from_millis(50);
    let started = Instant::now();
    while started.elapsed() < total {
        if stop.load(Ordering::SeqCst) {
            return true;
        }
        let left = total.saturating_sub(started.elapsed());
        std::thread::sleep(left.min(step));
    }
    stop.load(Ordering::SeqCst)
}

enum SessionEnd {
    Stopped,
    Dropped(String),
}

struct Session {
    socket: tungstenite::WebSocket<TcpStream>,
    endpoint: Endpoint,
    token: String,
    client_id: String,
    /// True once a snapshot has been read on THIS connection. Revisions and
    /// cameras are not carried across a reconnect.
    synced: bool,
    revision: u64,
    /// The camera the host is believed to have: what a local report is compared
    /// against before it becomes an HTTP request.
    last_known: Option<CameraSnapshot>,
    /// The camera last sent to the shell, so an event that asks for a camera
    /// the display already shows does not become another command.
    displayed: Option<(GeoPoint, f64)>,
    /// The last viewport a display reported, kept even while unsynced: the
    /// size is known before the host has said where its camera is.
    display_size: Option<(u32, u32)>,
    /// A camera forwarded to the shell that the shell has not reported back
    /// yet. Reports that do not match it are ticks from before the move.
    pending: Option<(GeoPoint, f64)>,
    counter: u64,
    log: FailureLog,
}

impl Session {
    fn run(
        &mut self,
        http: &reqwest::blocking::Client,
        reports: &Receiver<Report>,
        commands: &Sender<AcceptedCommand>,
        stop: &AtomicBool,
    ) -> SessionEnd {
        loop {
            if stop.load(Ordering::SeqCst) {
                return SessionEnd::Stopped;
            }
            loop {
                match reports.try_recv() {
                    Ok(report) => self.report(report, http),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return SessionEnd::Stopped,
                }
            }
            match self.read() {
                Ok(Some(event)) => self.event(event, commands, http),
                Ok(None) => {}
                Err(error) => return SessionEnd::Dropped(error),
            }
        }
    }

    fn read(&mut self) -> Result<Option<Event>, String> {
        match self.socket.read() {
            Ok(Message::Text(text)) => serde_json::from_str::<Event>(text.as_str())
                .map(Some)
                .map_err(|error| format!("an event was not the protocol's JSON: {error}")),
            Ok(Message::Close(_)) => Err("the host closed the event stream".to_string()),
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn event(
        &mut self,
        event: Event,
        commands: &Sender<AcceptedCommand>,
        http: &reqwest::blocking::Client,
    ) {
        match event {
            Event::Snapshot {
                revision, views, ..
            } => {
                self.revision = revision;
                self.synced = true;
                self.pending = None;
                if let Some(view) = views.get(MAIN_VIEW) {
                    self.follow(view, commands, "server", None);
                    // The display's own size is a fact the host needs even
                    // when the camera arrives at the one the display shows.
                    self.sync_viewport(http);
                }
            }
            Event::CameraChanged {
                revision,
                source,
                command_id,
                view,
            } => {
                if !self.synced || revision <= self.revision {
                    // A replay from before the snapshot, or an event that
                    // predates one already applied.
                    return;
                }
                self.revision = revision;
                self.last_known = Some(view.camera);
                if source.as_deref() == Some(self.client_id.as_str()) {
                    // The host accepted our own report. The display is
                    // already showing it; following would pull the camera
                    // back from a gesture that has moved on since.
                    return;
                }
                if view.id == MAIN_VIEW {
                    self.follow(
                        &view,
                        commands,
                        source.as_deref().unwrap_or("server"),
                        command_id.as_deref(),
                    );
                }
            }
            Event::CommandAccepted { .. } => {}
            Event::CommandRejected {
                command_id,
                code,
                message,
            } => {
                self.log.log(&format!(
                    "the host refused {command_id}: {message} ({code})"
                ));
            }
        }
    }

    /// Ask the shell to show a camera the host published.
    fn follow(
        &mut self,
        view: &ViewState,
        commands: &Sender<AcceptedCommand>,
        source: &str,
        command_id: Option<&str>,
    ) {
        let camera = view.camera;
        self.last_known = Some(camera);
        let shown = self
            .displayed
            .is_some_and(|(center, zoom)| center == camera.center && zoom == camera.zoom);
        if shown {
            // Nothing will be sent to the shell, so there is nothing to wait
            // for. The display may still need to hear about the viewport; the
            // caller handles that on a snapshot.
            self.pending = None;
            return;
        }
        self.displayed = Some((camera.center, camera.zoom));
        self.pending = Some((camera.center, camera.zoom));
        let accepted = AcceptedCommand {
            client_id: source.to_string(),
            command_id: command_id
                .map(str::to_string)
                .unwrap_or_else(|| format!("follow-r{}", self.revision)),
            command: Command::CameraSet {
                center: camera.center,
                zoom: camera.zoom,
            },
        };
        let _ = commands.send(accepted);
    }

    fn report(&mut self, report: Report, http: &reqwest::blocking::Client) {
        match report {
            Report::Camera { camera, local } => {
                self.display_size = Some((camera.width, camera.height));
                if !self.synced {
                    return;
                }
                if let Some((center, zoom)) = self.pending {
                    if camera.center == center && camera.zoom == zoom {
                        self.pending = None;
                    } else {
                        // A tick from before the shell applied the host's
                        // camera. Posting it would undo the host's move.
                        return;
                    }
                }
                self.displayed = Some((camera.center, camera.zoom));
                let Some(known) = self.last_known else {
                    return;
                };
                if local && (known.center != camera.center || known.zoom != camera.zoom) {
                    self.post(
                        Command::CameraSet {
                            center: camera.center,
                            zoom: camera.zoom,
                        },
                        http,
                    );
                }
                if known.width != camera.width || known.height != camera.height {
                    self.post(
                        Command::ViewReport {
                            width: camera.width,
                            height: camera.height,
                        },
                        http,
                    );
                }
            }
            Report::Viewport { width, height } => {
                self.display_size = Some((width, height));
                if !self.synced {
                    return;
                }
                self.report_viewport(width, height, http);
            }
        }
    }

    fn report_viewport(&mut self, width: u32, height: u32, http: &reqwest::blocking::Client) {
        let Some(known) = self.last_known else {
            return;
        };
        if known.width != width || known.height != height {
            self.post(Command::ViewReport { width, height }, http);
        }
    }

    /// Tell a fresh host how big this display is, once, on the snapshot.
    ///
    /// Not on every event: a viewport belongs to a display, and two displays
    /// answering each other's viewport would trade `view.report`s forever.
    fn sync_viewport(&mut self, http: &reqwest::blocking::Client) {
        if let Some((width, height)) = self.display_size {
            self.report_viewport(width, height, http);
        }
    }

    /// Post one command and absorb its success.
    ///
    /// The known camera advances only on success, so a refused or unreachable
    /// post is retried by the next report rather than being assumed applied.
    fn post(&mut self, command: Command, http: &reqwest::blocking::Client) {
        let command_id = format!("{}-c{}", self.client_id, self.counter);
        self.counter += 1;
        let request = CommandRequest {
            protocol_version: Some(PROTOCOL_VERSION),
            client_id: Some(self.client_id.clone()),
            command_id: Some(command_id.clone()),
            command,
        };
        let response = match http
            .post(self.endpoint.commands_url())
            .bearer_auth(&self.token)
            .json(&request)
            .send()
        {
            Ok(response) => response,
            Err(error) => {
                self.log.log(&format!("posting {command_id}: {error}"));
                return;
            }
        };
        let status = response.status();
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            let body = body.trim();
            if body.is_empty() {
                self.log
                    .log(&format!("posting {command_id}: HTTP {status}"));
            } else {
                self.log.log(&format!(
                    "posting {command_id}: HTTP {status} \u{2014} {body}"
                ));
            }
            return;
        }
        match &request.command {
            Command::CameraSet { center, zoom } => {
                if let Some(known) = &mut self.last_known {
                    known.center = *center;
                    known.zoom = *zoom;
                }
            }
            Command::ViewReport { width, height } => {
                if let Some(known) = &mut self.last_known {
                    known.width = *width;
                    known.height = *height;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{CameraSource, ControlHub, ServerConfig, ServerHandle, spawn_server};
    use std::sync::mpsc::channel;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn camera(latitude: f64, longitude: f64, zoom: f64, width: u32, height: u32) -> CameraSnapshot {
        CameraSnapshot {
            center: GeoPoint {
                latitude,
                longitude,
            },
            zoom,
            width,
            height,
        }
    }

    fn place(camera: &CameraSnapshot) -> Command {
        Command::CameraSet {
            center: camera.center,
            zoom: camera.zoom,
        }
    }

    /// A host with no GPUI: the hub, a server on a free port, and the channel
    /// a real host drains. The test is the drain, so it can apply a command
    /// when it wants to see the resulting event.
    struct Host {
        handle: ServerHandle,
        hub: ControlHub,
        accepted: Receiver<AcceptedCommand>,
    }

    impl Host {
        fn start(camera: CameraSnapshot) -> Self {
            let (tx, accepted) = channel();
            let hub = ControlHub::new(camera);
            let handle = spawn_server(
                ServerConfig {
                    listen: "127.0.0.1:0".to_string(),
                    token: TOKEN.to_string(),
                    devices: None,
                },
                tx,
                hub.clone(),
            )
            .expect("server starts");
            Self {
                handle,
                hub,
                accepted,
            }
        }

        fn config(&self) -> ClientConfig {
            let mut config = ClientConfig::new(self.handle.url(), TOKEN);
            config.connect_timeout = Duration::from_secs(2);
            config.read_timeout = Duration::from_millis(40);
            config.post_timeout = Duration::from_secs(2);
            config
        }

        /// Apply a command as `tfg-server`'s drain would.
        fn apply(&self, command: &AcceptedCommand) {
            let source = CameraSource {
                client_id: command.client_id.clone(),
                command_id: Some(command.command_id.clone()),
            };
            self.hub.apply(&command.command, source);
        }
    }

    /// Start a client and wait for the first snapshot to reach its channel.
    fn followed_host(host: &Host) -> (ClientHandle, Receiver<AcceptedCommand>, CameraSnapshot) {
        let (commands, forwarded) = channel();
        let client = spawn_client(host.config(), commands).expect("client starts");
        let accepted = forwarded
            .recv_timeout(Duration::from_secs(3))
            .expect("the snapshot reaches the shell");
        assert!(matches!(accepted.command, Command::CameraSet { .. }));
        let camera = host.hub.camera();
        // The shell always answers a camera it applied; without this the
        // client's pending gate would hold the next report.
        client.followed(camera);
        (client, forwarded, camera)
    }

    #[test]
    fn a_console_follows_the_hosts_camera() {
        let start = camera(-6.2, 106.8, 11.0, 1280, 720);
        let host = Host::start(start);
        let (commands, forwarded) = channel();
        let _client = spawn_client(host.config(), commands).expect("client starts");
        let accepted = forwarded
            .recv_timeout(Duration::from_secs(3))
            .expect("the snapshot reaches the shell");
        assert_eq!(accepted.command, place(&start));
        assert_eq!(accepted.client_id, "server");
    }

    #[test]
    fn a_remote_camera_change_moves_the_console() {
        let start = camera(-6.2, 106.8, 11.0, 1280, 720);
        let host = Host::start(start);
        let (_client, forwarded, _) = followed_host(&host);
        let moved = camera(-6.4, 107.1, 9.5, 1280, 720);
        host.hub.apply(
            &place(&moved),
            CameraSource {
                client_id: "phone".to_string(),
                command_id: Some("phone-1".to_string()),
            },
        );
        let accepted = forwarded
            .recv_timeout(Duration::from_secs(3))
            .expect("the remote camera reaches the shell");
        assert_eq!(accepted.command, place(&moved));
        assert_eq!(accepted.client_id, "phone");
        assert_eq!(accepted.command_id, "phone-1");
    }

    #[test]
    fn a_local_camera_is_posted_once_and_its_echo_is_not() {
        let start = camera(-6.2, 106.8, 11.0, 1280, 720);
        let host = Host::start(start);
        let (client, forwarded, _) = followed_host(&host);
        let moved = camera(-6.5, 106.9, 10.0, 1280, 720);
        client.camera(moved);
        let posted = host
            .accepted
            .recv_timeout(Duration::from_secs(3))
            .expect("the local camera reaches the host");
        assert_eq!(posted.command, place(&moved));
        assert!(posted.client_id.starts_with("console-"));
        // The host applies it, as a headless host would, and broadcasts the
        // echo. It must not come back to the shell as another command.
        host.apply(&posted);
        assert_eq!(host.hub.camera(), moved);
        assert!(
            forwarded.recv_timeout(Duration::from_millis(400)).is_err(),
            "the console forwarded its own move back to the shell"
        );
        // And the same camera again is not a new command.
        client.camera(moved);
        assert!(
            host.accepted
                .recv_timeout(Duration::from_millis(400))
                .is_err(),
            "an equal camera was posted twice"
        );
    }

    #[test]
    fn a_resize_is_a_view_report_not_a_camera_set() {
        let start = camera(-6.2, 106.8, 11.0, 1280, 720);
        let host = Host::start(start);
        let (client, _forwarded, _) = followed_host(&host);
        let resized = camera(-6.2, 106.8, 11.0, 1600, 900);
        client.camera(resized);
        let posted = host
            .accepted
            .recv_timeout(Duration::from_secs(3))
            .expect("the viewport reaches the host");
        assert_eq!(
            posted.command,
            Command::ViewReport {
                width: 1600,
                height: 900
            }
        );
    }

    #[test]
    fn a_stale_tick_cannot_undo_a_remote_move() {
        let start = camera(-6.2, 106.8, 11.0, 1280, 720);
        let host = Host::start(start);
        let (client, forwarded, _) = followed_host(&host);
        let moved = camera(-6.4, 107.1, 9.5, 1280, 720);
        host.hub.apply(
            &place(&moved),
            CameraSource {
                client_id: "phone".to_string(),
                command_id: Some("phone-2".to_string()),
            },
        );
        forwarded
            .recv_timeout(Duration::from_secs(3))
            .expect("the remote camera reaches the shell");
        // A tick generated before the shell applied the remote camera still
        // carries the old one. It must not be posted, or the host would move
        // back to it.
        client.camera(start);
        assert!(
            host.accepted
                .recv_timeout(Duration::from_millis(400))
                .is_err(),
            "a stale camera was posted over the remote move"
        );
        // Once the shell reports the camera it was asked for, local reports
        // are accepted again.
        client.followed(moved);
        let next = camera(-6.0, 106.5, 12.0, 1280, 720);
        client.camera(next);
        let posted = host
            .accepted
            .recv_timeout(Duration::from_secs(3))
            .expect("a fresh local camera still posts");
        assert_eq!(posted.command, place(&next));
    }

    #[test]
    fn the_server_url_is_http_and_pathless() {
        let endpoint = Endpoint::parse("http://192.168.1.5:9000").expect("plain http");
        assert_eq!(endpoint.base, "http://192.168.1.5:9000");
        assert_eq!(endpoint.websocket_url(), "ws://192.168.1.5:9000/v1/ws");
        assert_eq!(
            Endpoint::parse("http://example.test")
                .expect("the default port")
                .port,
            80
        );
        assert!(
            Endpoint::parse("https://example.test")
                .expect_err("wss is refused")
                .contains("not supported")
        );
        assert!(
            Endpoint::parse("http://example.test/lan/v1")
                .expect_err("a path is refused")
                .contains("should not carry a path")
        );
        assert!(Endpoint::parse("example.test:9000").is_err());
        assert_eq!(
            Endpoint::parse("http://[::1]:9000").expect("ipv6").base,
            "http://[::1]:9000"
        );
    }
}
