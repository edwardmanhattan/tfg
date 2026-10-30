//! The shared observation state: one revision, one fan-out, one writer per
//! fact.
//!
//! [`ControlHub`] owns everything a remote client may see:
//!
//! - the camera the console renders (`main`),
//! - the camera a command center renders and a phone moves (`observer`),
//! - the exercise the console projected (`session`), with its units,
//! - which command centers are attached,
//! - the readiness verdict computed from those facts.
//!
//! Each fact has exactly one writer, and the mutex is where they meet:
//!
//! - **Embedded**: the desktop's order thread reports `main`; the session
//!   publisher writes the session; the server thread applies observer
//!   commands and records displays.
//! - **Headless** (`tfg-server`): the command thread applies `main`, the HTTP
//!   route writes the session, the server thread applies observer commands.
//!
//! No writer is a toolkit object and no server thread touches GPUI. The camera
//! in the hub is always one somebody is rendering, and the session is always
//! one the console actually holds.
//!
//! Idempotency lives here too: the hub remembers recent command ids, so a
//! client that retries a POST after a timeout does not move the camera twice.

use super::protocol::{
    CameraSnapshot, CameraSource, Command, DisplayState, Event, GeoPoint, MAIN_VIEW,
    OBSERVER_VIEW, ObserverUi, PROTOCOL_VERSION, Panel, Readiness, ReadinessReason, SessionState,
    SettingsState, ViewState,
};
use crate::map_geometry::pan_center;
use crate::view::{ZOOM_MAX, ZOOM_MIN};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// How many events a slow subscriber may fall behind before it is told to
/// resynchronise. Small on purpose: the recovery is a fresh snapshot, and a
/// stale camera is worse than a re-read.
pub const EVENT_BUFFER: usize = 64;

/// Remembered command ids: enough for a retry storm, bounded so a misbehaving
/// client cannot grow the table without limit.
pub const SEEN_COMMAND_CAPACITY: usize = 512;

/// How long a command id stays remembered. A retry after this is a new
/// command; the operator's hand has moved on.
pub const SEEN_COMMAND_TTL: Duration = Duration::from_secs(300);

/// A session that has not been refreshed for this long is no longer live.
///
/// The console refreshes every five seconds. Fifteen seconds tolerates two
/// missed heartbeats and still notices a crash before a phone has been staring
/// at a frozen map for long.
pub const SESSION_TTL: Duration = Duration::from_secs(15);

/// A command center that has not been heard from for this long is gone.
///
/// The display heartbeats every ten seconds; thirty seconds tolerates two
/// missed beats plus a reconnect.
pub const DISPLAY_TTL: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct SessionRecord {
    state: SessionState,
    last_seen: Instant,
}

#[derive(Debug)]
struct DisplayRecord {
    state: DisplayState,
    last_seen: Instant,
}

#[derive(Debug)]
struct Shared {
    views: BTreeMap<String, CameraSnapshot>,
    session: Option<SessionRecord>,
    displays: BTreeMap<String, DisplayRecord>,
    readiness: Readiness,
    /// The observer's windows and popup, as the phone set them.
    ui: ObserverUi,
    /// The console's text size, as a step of its four-step ladder.
    text_scale_step: usize,
    /// Hull pictures the console published, PNG-encoded, keyed by unit id.
    ///
    /// Not part of the revision: a picture is fetched by a client that wants
    /// it, not fanned out to every watcher, and the session has already said
    /// which hulls exist.
    pictures: HashMap<i64, Arc<Vec<u8>>>,
    revision: u64,
    /// Recent `(command_id, first seen)` pairs, oldest first.
    seen: VecDeque<(String, Instant)>,
}

impl Shared {
    /// True when this id was already accepted. Also records it when new.
    fn remember(&mut self, command_id: &str, now: Instant) -> bool {
        while let Some((_, at)) = self.seen.front() {
            if now.duration_since(*at) < SEEN_COMMAND_TTL && self.seen.len() < SEEN_COMMAND_CAPACITY
            {
                break;
            }
            self.seen.pop_front();
        }
        if self.seen.iter().any(|(id, _)| id == command_id) {
            return true;
        }
        self.seen.push_back((command_id.to_string(), now));
        false
    }
}

/// Why a camera command could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    /// No view has that name. `observer` is created when a display attaches;
    /// anything else is a client bug.
    UnknownView(String),
    /// The focus command named a unit the session does not hold.
    UnknownUnit(i64),
    /// The unit exists but has neither a live fix nor a recorded placement.
    UnitHasNoPosition(i64),
    /// The focus command named a task group the session does not hold.
    UnknownGroup(i64),
    /// The group exists but none of its hulls has a ground to centre on.
    GroupHasNoPosition(i64),
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownView(view) => write!(f, "there is no view named `{view}`"),
            Self::UnknownUnit(id) => write!(f, "the session holds no unit {id}"),
            Self::UnitHasNoPosition(id) => {
                write!(f, "unit {id} has no position to focus on")
            }
            Self::UnknownGroup(id) => write!(f, "the session holds no task group {id}"),
            Self::GroupHasNoPosition(id) => {
                write!(f, "task group {id} has no placed hull to focus on")
            }
        }
    }
}

impl std::error::Error for ApplyError {}

/// The handle shared between the desktop host and the server threads.
///
/// Cheap to clone: clones share the same state and the same event fan-out.
#[derive(Clone)]
pub struct ControlHub {
    shared: Arc<Mutex<Shared>>,
    events: broadcast::Sender<Event>,
}

impl ControlHub {
    /// A hub at revision 0, looking where the desktop is looking.
    pub fn new(camera: CameraSnapshot) -> Self {
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        Self {
            shared: Arc::new(Mutex::new(Shared {
                views: BTreeMap::from([(MAIN_VIEW.to_string(), camera)]),
                session: None,
                displays: BTreeMap::new(),
                readiness: Readiness::waiting(ReadinessReason::NoSession),
                ui: ObserverUi::default(),
                text_scale_step: crate::view::TEXT_SCALE_DEFAULT,
                pictures: HashMap::new(),
                revision: 0,
                seen: VecDeque::new(),
            })),
            events,
        }
    }

    /// A poisoned lock here holds whole values and a revision, not a
    /// half-applied transaction, so recovering is honest.
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The camera the desktop currently renders. The main view is never
    /// removed, so this cannot be absent.
    pub fn camera(&self) -> CameraSnapshot {
        self.lock()
            .views
            .get(MAIN_VIEW)
            .copied()
            .expect("the main view is created with the hub")
    }

    /// One named camera, if it exists.
    pub fn camera_of(&self, view: &str) -> Option<CameraSnapshot> {
        self.lock().views.get(view).copied()
    }

    /// The revision of the shared state. Monotonic; 0 before the first change.
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// The exercise the console published, if any. A stale session is still
    /// returned: the map can keep showing the last known units while the
    /// readiness verdict says nobody is refreshing them.
    pub fn session(&self) -> Option<SessionState> {
        self.lock().session.as_ref().map(|record| record.state.clone())
    }

    /// The console's text size, as a step of its ladder.
    pub fn text_scale_step(&self) -> usize {
        self.lock().text_scale_step
    }

    /// The host's text size, as the state every client reads.
    pub fn settings(&self) -> SettingsState {
        let step = self.lock().text_scale_step;
        SettingsState {
            step,
            text_scale: crate::view::text_scale_at(step),
        }
    }

    /// Record the text size a client asked for (or the console reported) and
    /// TELL EVERY WALL.
    ///
    /// A SETTING NOBODY IS TOLD ABOUT is a setting that only exists in the
    /// process that applied it. The walls used to draw at their own compile-
    /// time default no matter what `GET /v1/settings` answered, so a phone's
    /// `settings.font` moved the console and nothing else. The change spends a
    /// revision because it changes what every attached display shows.
    ///
    /// Returns the revision when something changed, `None` when the step was
    /// already in force — a command that would change nothing is still
    /// accepted, it simply does not move anything.
    pub fn set_text_scale_step(&self, step: usize) -> Option<u64> {
        let step = step.min(crate::view::TEXT_SCALES.len() - 1);
        let mut shared = self.lock();
        if shared.text_scale_step == step {
            return None;
        }
        shared.text_scale_step = step;
        let revision = bump(&mut shared);
        drop(shared);
        let _ = self.events.send(Event::SettingsChanged {
            revision,
            settings: SettingsState {
                step,
                text_scale: crate::view::text_scale_at(step),
            },
        });
        Some(revision)
    }

    /// The observer's windows and popup.
    pub fn ui(&self) -> ObserverUi {
        self.lock().ui
    }

    /// A hull picture the console published, PNG-encoded.
    pub fn picture(&self, unit_id: i64) -> Option<Arc<Vec<u8>>> {
        self.lock().pictures.get(&unit_id).cloned()
    }

    /// Store a hull picture. Not a revision: a picture is fetched by the
    /// client that wants it, not fanned out with the session.
    pub fn set_picture(&self, unit_id: i64, png: Vec<u8>) {
        self.lock().pictures.insert(unit_id, Arc::new(png));
    }

    /// The command centers attached and live right now.
    pub fn displays(&self) -> Vec<DisplayState> {
        let shared = self.lock();
        live_displays(&shared, Instant::now())
    }

    /// The readiness verdict, computed against the clock rather than trusted
    /// from the last mutation.
    pub fn readiness(&self) -> Readiness {
        let shared = self.lock();
        readiness_at(&shared, Instant::now())
    }

    /// The whole shared state, as the WebSocket snapshot and `/v1/state`.
    pub fn snapshot_event(&self) -> Event {
        let shared = self.lock();
        let now = Instant::now();
        Event::Snapshot {
            protocol_version: PROTOCOL_VERSION,
            revision: shared.revision,
            views: shared
                .views
                .iter()
                .map(|(id, camera)| {
                    (
                        id.clone(),
                        ViewState {
                            id: id.clone(),
                            camera: *camera,
                        },
                    )
                })
                .collect(),
            session: shared.session.as_ref().map(|record| record.state.clone()),
            readiness: readiness_at(&shared, now),
            displays: live_displays(&shared, now),
            ui: shared.ui,
            settings: SettingsState {
                step: shared.text_scale_step,
                text_scale: crate::view::text_scale_at(shared.text_scale_step),
            },
        }
    }

    /// Every event fans out from here, including a behind-the-back publish
    /// from the host. A closed set of subscribers is not an error.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// A new camera from the desktop, with the client (or hand) that moved it.
    /// The main-view wrapper for [`Self::report_view`].
    pub fn report_camera(&self, camera: CameraSnapshot, source: CameraSource) -> Option<u64> {
        self.report_view(MAIN_VIEW, camera, source)
    }

    /// A camera report for one named view.
    ///
    /// Returns the new revision when the camera actually changed, and `None`
    /// when the tick reported the camera it already had. A resize counts: the
    /// viewport is part of the camera, and a remote nudge resolves against it.
    pub fn report_view(
        &self,
        view: &str,
        camera: CameraSnapshot,
        source: CameraSource,
    ) -> Option<u64> {
        let mut shared = self.lock();
        if !shared.views.contains_key(view) && !create_observer(&mut shared, view) {
            return None;
        }
        if shared.views.get(view).is_some_and(|current| *current == camera) {
            return None;
        }
        shared.views.insert(view.to_string(), camera);
        let revision = bump(&mut shared);
        let event = camera_event(
            revision,
            view,
            camera,
            Some(source.client_id),
            source.command_id,
        );
        drop(shared);
        let _ = self.events.send(event);
        Some(revision)
    }

    /// Resolve a camera command against the main view.
    pub fn apply(&self, command: &Command, source: CameraSource) -> Option<u64> {
        self.apply_view(MAIN_VIEW, command, source).ok().flatten()
    }

    /// Resolve a camera command against one named view.
    ///
    /// The desktop does not need this for `main`: it moves its own camera and
    /// reports the result, so the camera in the hub is always one it is
    /// rendering. A headless server has no camera to ask, so the command is
    /// applied here — with the same Mercator arithmetic and the same zoom rails
    /// the shell uses, because a reducer that clamped differently from the
    /// renderer would publish a camera nobody draws. The observer view is
    /// always applied here, on both deployments, because only the command
    /// center renders it.
    ///
    /// Returns the revision when something changed, `None` when the command was
    /// a no-op. An `AcceptedCommand` whose command was a no-op is still
    /// *accepted*; it simply does not move anything.
    pub fn apply_view(
        &self,
        view: &str,
        command: &Command,
        source: CameraSource,
    ) -> Result<Option<u64>, ApplyError> {
        let mut shared = self.lock();
        if !shared.views.contains_key(view) && !create_observer(&mut shared, view) {
            return Err(ApplyError::UnknownView(view.to_string()));
        }
        // A panel toggle is observer UI, not a camera. It has its own event
        // (the session one) because every attached wall draws the same
        // windows, and a revision bump is how they learn.
        if let Command::UiSet { panel, open } = command {
            let Some(panel) = Panel::parse(panel) else {
                return Ok(None);
            };
            let mut panels = shared.ui.panels;
            panels.set(panel, *open);
            if panels == shared.ui.panels {
                return Ok(None);
            }
            shared.ui.panels = panels;
            let revision = bump(&mut shared);
            let readiness = readiness_at(&shared, Instant::now());
            let event = session_event(&shared, revision, readiness);
            drop(shared);
            let _ = self.events.send(event);
            return Ok(Some(revision));
        }
        let current = *shared
            .views
            .get(view)
            .expect("the view exists after creation");
        let ui_before = shared.ui;
        let next = match command {
            Command::CameraSet { center, zoom } => Some(CameraSnapshot {
                center: *center,
                zoom: zoom.clamp(ZOOM_MIN, ZOOM_MAX),
                ..current
            }),
            Command::CameraNudge {
                dx_fraction,
                dy_fraction,
            } => {
                let (width, height) = (f64::from(current.width), f64::from(current.height));
                if width <= 0.0 || height <= 0.0 {
                    // No display has said how big it is: a fraction of a
                    // viewport nobody has named has no meaning.
                    return Ok(None);
                }
                let (latitude, longitude) = pan_center(
                    (current.center.latitude, current.center.longitude),
                    current.zoom,
                    dx_fraction * width,
                    dy_fraction * height,
                    width,
                    height,
                );
                Some(CameraSnapshot {
                    center: GeoPoint {
                        latitude,
                        longitude,
                    },
                    ..current
                })
            }
            Command::CameraZoom { delta } => Some(CameraSnapshot {
                zoom: (current.zoom + delta).clamp(ZOOM_MIN, ZOOM_MAX),
                ..current
            }),
            Command::CameraFocus { unit_id, zoom } => {
                shared.ui.focus = Some(*unit_id);
                shared.ui.focus_group = None;
                let position = shared
                    .session
                    .as_ref()
                    .and_then(|record| {
                        record
                            .state
                            .units
                            .iter()
                            .find(|unit| unit.id == *unit_id)
                    })
                    .ok_or(ApplyError::UnknownUnit(*unit_id))?
                    .position
                    .ok_or(ApplyError::UnitHasNoPosition(*unit_id))?;
                Some(CameraSnapshot {
                    center: position,
                    zoom: zoom
                        .map(|zoom| zoom.clamp(ZOOM_MIN, ZOOM_MAX))
                        .unwrap_or(current.zoom),
                    ..current
                })
            }
            Command::CameraFocusGroup { group_id, zoom } => {
                let position = shared
                    .session
                    .as_ref()
                    .and_then(|record| group_anchor(&record.state, *group_id))
                    .ok_or_else(|| {
                        if shared
                            .session
                            .as_ref()
                            .is_some_and(|record| record.state.groups.iter().any(|g| g.id == *group_id))
                        {
                            ApplyError::GroupHasNoPosition(*group_id)
                        } else {
                            ApplyError::UnknownGroup(*group_id)
                        }
                    })?;
                shared.ui.focus_group = Some(*group_id);
                shared.ui.focus = None;
                Some(CameraSnapshot {
                    center: position,
                    zoom: zoom
                        .map(|zoom| zoom.clamp(ZOOM_MIN, ZOOM_MAX))
                        .unwrap_or(current.zoom),
                    ..current
                })
            }
            Command::ViewReport { width, height } => Some(CameraSnapshot {
                width: *width,
                height: *height,
                ..current
            }),
            Command::UiSet { .. } => unreachable!("handled above"),
            // Host-wide, recorded by the server on the way in; not a view.
            Command::SettingsFont { .. } => return Ok(None),
        };
        let Some(camera) = next else {
            return Ok(None);
        };
        if camera == current {
            return Ok(None);
        }
        shared.views.insert(view.to_string(), camera);
        let revision = bump(&mut shared);
        let camera = camera_event(
            revision,
            view,
            camera,
            Some(source.client_id),
            source.command_id,
        );
        // A focus is BOTH a camera move and an observer-UI change: the wall
        // draws the popup from `ui`, so a camera event alone would move the
        // map and leave the card shut until the next roster heartbeat.
        let mut events = vec![camera];
        if shared.ui != ui_before {
            let readiness = readiness_at(&shared, Instant::now());
            events.push(session_event(&shared, revision, readiness));
        }
        drop(shared);
        publish(&self.events, events);
        Ok(Some(revision))
    }

    /// Publish the console's projection of the exercise it holds.
    ///
    /// The hub owns the unit numbers: it assigns `index` from array order, so
    /// the phone's grid and the command center's markers cannot disagree. A
    /// publish equal to the current state is a heartbeat and publishes
    /// nothing.
    pub fn set_session(&self, mut session: SessionState) -> Option<u64> {
        for (index, unit) in session.units.iter_mut().enumerate() {
            unit.index = u32::try_from(index + 1).unwrap_or(u32::MAX);
        }
        let now = Instant::now();
        let mut shared = self.lock();
        let previous_game = shared.session.as_ref().map(|record| record.state.game_id);
        let changed = shared
            .session
            .as_ref()
            .map(|record| &record.state)
            != Some(&session);
        // Computed rather than read from the cached verdict: the clock moved
        // since the last mutation, and a session that went quiet is exactly
        // the case this publish revives.
        let was_stale = readiness_at(&shared, now).reason == Some(ReadinessReason::Stale);
        shared.session = Some(SessionRecord {
            state: session.clone(),
            last_seen: now,
        });
        if !changed && !was_stale {
            shared.readiness = readiness_at(&shared, now);
            return None;
        }
        // A new exercise opens where the operator is looking; an update to the
        // same exercise leaves the observer where the phone put it.
        let reset = previous_game != Some(session.game_id);
        if reset
            && let Some(main) = shared.views.get(MAIN_VIEW).copied()
            && shared.views.contains_key(OBSERVER_VIEW)
        {
            shared.views.insert(OBSERVER_VIEW.to_string(), main);
        }
        if reset {
            // A different exercise is a different screen: the phone's windows
            // and popup belonged to the last one.
            shared.ui = ObserverUi::default();
        }
        let revision = bump(&mut shared);
        let readiness = readiness_at(&shared, now);
        shared.readiness = readiness.clone();
        let mut events = vec![session_event(&shared, revision, readiness)];
        if reset
            && let Some(observer) = shared.views.get(OBSERVER_VIEW).copied()
        {
            events.push(camera_event(
                revision,
                OBSERVER_VIEW,
                observer,
                Some("server".to_string()),
                None,
            ));
        }
        drop(shared);
        publish(&self.events, events);
        Some(revision)
    }

    /// The console no longer holds an exercise. The observer camera is kept
    /// where it was, so a command center that reconnects before the next
    /// session is not thrown back to a default place.
    pub fn clear_session(&self) -> Option<u64> {
        let now = Instant::now();
        let mut shared = self.lock();
        shared.session.take()?;
        shared.ui = ObserverUi::default();
        let revision = bump(&mut shared);
        let readiness = readiness_at(&shared, now);
        shared.readiness = readiness.clone();
        let event = session_event(&shared, revision, readiness);
        drop(shared);
        let _ = self.events.send(event);
        Some(revision)
    }

    /// Refresh a session without republishing it.
    ///
    /// A heartbeat that arrives while the session is healthy changes nothing.
    /// One that revives a stale session is a real change: readiness flips back.
    pub fn touch_session(&self) -> Option<u64> {
        let now = Instant::now();
        let mut shared = self.lock();
        shared.session.as_ref()?;
        // Read the verdict *before* refreshing: a session that was stale is
        // exactly the one this heartbeat brings back.
        let was_stale = readiness_at(&shared, now).reason == Some(ReadinessReason::Stale);
        if let Some(record) = shared.session.as_mut() {
            record.last_seen = now;
        }
        if !was_stale {
            return None;
        }
        let revision = bump(&mut shared);
        let readiness = readiness_at(&shared, now);
        shared.readiness = readiness.clone();
        let event = session_event(&shared, revision, readiness);
        drop(shared);
        let _ = self.events.send(event);
        Some(revision)
    }

    /// Record that a command center is attached. The first one creates the
    /// observer view from the operator's camera.
    pub fn attach_display(&self, display: DisplayState) -> Option<u64> {
        let now = Instant::now();
        let mut shared = self.lock();
        let id = display.id.clone();
        shared.displays.insert(
            id,
            DisplayRecord {
                state: display,
                last_seen: now,
            },
        );
        let created = if shared.views.contains_key(OBSERVER_VIEW) {
            None
        } else {
            shared
                .views
                .get(MAIN_VIEW)
                .copied()
                .map(|main| {
                    shared.views.insert(OBSERVER_VIEW.to_string(), main);
                    main
                })
        };
        let revision = bump(&mut shared);
        let readiness = readiness_at(&shared, now);
        shared.readiness = readiness.clone();
        let mut events = vec![session_event(&shared, revision, readiness)];
        if let Some(observer) = created {
            events.push(camera_event(
                revision,
                OBSERVER_VIEW,
                observer,
                Some("server".to_string()),
                None,
            ));
        }
        drop(shared);
        publish(&self.events, events);
        Some(revision)
    }

    /// A display's heartbeat. No revision: being alive is not a change.
    pub fn touch_display(&self, id: &str) {
        let mut shared = self.lock();
        if let Some(record) = shared.displays.get_mut(id) {
            record.last_seen = Instant::now();
        }
    }

    /// A display's socket ended. Readiness may flip; the observer camera is
    /// deliberately kept.
    pub fn detach_display(&self, id: &str) -> Option<u64> {
        let now = Instant::now();
        let mut shared = self.lock();
        shared.displays.remove(id)?;
        let revision = bump(&mut shared);
        let readiness = readiness_at(&shared, now);
        shared.readiness = readiness.clone();
        let event = session_event(&shared, revision, readiness);
        drop(shared);
        let _ = self.events.send(event);
        Some(revision)
    }

    /// Reap sessions and displays that stopped beating.
    ///
    /// Called on a timer by the server. Emits once per transition rather than
    /// once per tick.
    pub fn expire(&self) -> bool {
        let now = Instant::now();
        let mut shared = self.lock();
        let before = readiness_at(&shared, now);
        let before_displays = shared.displays.len();
        shared
            .displays
            .retain(|_, record| now.saturating_duration_since(record.last_seen) <= DISPLAY_TTL);
        let removed = shared.displays.len() != before_displays;
        let readiness = readiness_at(&shared, now);
        shared.readiness = readiness.clone();
        if !removed && readiness == before {
            return false;
        }
        let revision = bump(&mut shared);
        let event = session_event(&shared, revision, readiness);
        drop(shared);
        let _ = self.events.send(event);
        true
    }

    /// Record a command id. `true` means it was already seen: a retry, and the
    /// server should answer it without acting twice.
    pub fn remember_command(&self, command_id: &str) -> bool {
        self.lock().remember(command_id, Instant::now())
    }

    /// Tell subscribers a command reached the desktop's channel.
    pub fn command_accepted(&self, command_id: &str) {
        let revision = self.revision();
        let _ = self.events.send(Event::CommandAccepted {
            command_id: command_id.to_string(),
            revision,
        });
    }

    /// Tell subscribers a command was refused at the boundary. The HTTP reply
    /// carries the same code; the event exists so a watching operator sees a
    /// second client's mistake without reading its screen.
    pub fn command_rejected(&self, command_id: &str, code: &str, message: &str) {
        let _ = self.events.send(Event::CommandRejected {
            command_id: command_id.to_string(),
            code: code.to_string(),
            message: message.to_string(),
        });
    }

    /// The number of live subscribers. Used by the server's admission check
    /// and by tests; not part of the wire contract.
    pub fn subscribers(&self) -> usize {
        self.events.receiver_count()
    }

    /// Publish a bare event to the fan-out. The host uses it for events it
    /// originates; tests use it to prove lag recovery.
    pub fn publish(&self, event: Event) {
        let _ = self.events.send(event);
    }
}

/// `observer` is the only view that may be created on demand. `main` exists
/// from construction; anything else is a client mistake, not a new display.
fn create_observer(shared: &mut Shared, view: &str) -> bool {
    if view != OBSERVER_VIEW {
        return false;
    }
    let Some(main) = shared.views.get(MAIN_VIEW).copied() else {
        return false;
    };
    shared.views.insert(OBSERVER_VIEW.to_string(), main);
    true
}

fn bump(shared: &mut Shared) -> u64 {
    shared.revision += 1;
    shared.revision
}

fn live_displays(shared: &Shared, now: Instant) -> Vec<DisplayState> {
    shared
        .displays
        .values()
        .filter(|record| now.saturating_duration_since(record.last_seen) <= DISPLAY_TTL)
        .map(|record| record.state.clone())
        .collect()
}

fn readiness_at(shared: &Shared, now: Instant) -> Readiness {
    let Some(session) = &shared.session else {
        return Readiness::waiting(ReadinessReason::NoSession);
    };
    if now.saturating_duration_since(session.last_seen) > SESSION_TTL {
        return Readiness::waiting(ReadinessReason::Stale);
    }
    if !session.state.is_execution() {
        return Readiness::waiting(ReadinessReason::NotExecution);
    }
    let command_center = shared.displays.values().any(|record| {
        record.state.role == "command_center"
            && now.saturating_duration_since(record.last_seen) <= DISPLAY_TTL
    });
    if !command_center {
        return Readiness::waiting(ReadinessReason::NoDisplay);
    }
    Readiness::ready()
}

/// The ground a task group is focused on: the mean of every placed hull in
/// its subtree.
///
/// A group has no position of its own, so this is the only honest answer. A
/// group whose hulls are all unplaced has no anchor; the command is refused
/// rather than the camera being left where it was with a popup that has no
/// mark to sit beside.
fn group_anchor(session: &SessionState, group_id: i64) -> Option<GeoPoint> {
    let mut subtree = vec![group_id];
    let mut cursor = 0usize;
    while cursor < subtree.len() {
        let parent = subtree[cursor];
        for group in &session.groups {
            if group.parent == Some(parent) && !subtree.contains(&group.id) {
                subtree.push(group.id);
            }
        }
        cursor += 1;
    }
    let mut n = 0.0f64;
    let (mut lat, mut lon) = (0.0f64, 0.0f64);
    for group in session.groups.iter().filter(|g| subtree.contains(&g.id)) {
        for id in &group.units {
            if let Some(position) = session
                .units
                .iter()
                .find(|unit| unit.id == *id)
                .and_then(|unit| unit.position)
            {
                lat += position.latitude;
                lon += position.longitude;
                n += 1.0;
            }
        }
    }
    if n == 0.0 {
        return None;
    }
    Some(GeoPoint {
        latitude: lat / n,
        longitude: lon / n,
    })
}

fn session_event(shared: &Shared, revision: u64, readiness: Readiness) -> Event {
    Event::SessionChanged {
        revision,
        session: shared.session.as_ref().map(|record| record.state.clone()),
        readiness,
        displays: live_displays(shared, Instant::now()),
        ui: shared.ui,
    }
}

fn camera_event(
    revision: u64,
    view: &str,
    camera: CameraSnapshot,
    source: Option<String>,
    command_id: Option<String>,
) -> Event {
    Event::CameraChanged {
        revision,
        source,
        command_id,
        view: ViewState {
            id: view.to_string(),
            camera,
        },
    }
}

fn publish(events: &broadcast::Sender<Event>, events_to_send: Vec<Event>) {
    for event in events_to_send {
        let _ = events.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::protocol::{GeoPoint, ScenarioState, UnitState};

    fn camera(latitude: f64) -> CameraSnapshot {
        CameraSnapshot {
            center: GeoPoint {
                latitude,
                longitude: 106.8,
            },
            zoom: 11.0,
            width: 800,
            height: 600,
        }
    }

    fn unit(id: i64, name: &str, latitude: Option<f64>) -> UnitState {
        UnitState {
            index: 0,
            id,
            name: name.to_string(),
            hull_number: format!("H{id}"),
            class_name: "Frigate".to_string(),
            commander: None,
            embarked: Vec::new(),
            embarked_in: None,
            position: latitude.map(|latitude| GeoPoint {
                latitude,
                longitude: 106.8,
            }),
            heading_deg: None,
            speed_kn: None,
            stale: false,
                ..UnitState::default()        }
    }

    fn session(phase: &str, units: Vec<UnitState>) -> SessionState {
        SessionState {
            game_id: 42,
            name: "Exercise".to_string(),
            phase: phase.to_string(),
            mode: "maneuver".to_string(),
            scenario: Some(ScenarioState {
                id: 7,
                title: "Sunda Strait".to_string(),
                step: None,
            }),
            units,
            ..SessionState::default()
        }
    }

    fn display(id: &str) -> DisplayState {
        DisplayState {
            id: id.to_string(),
            label: id.to_string(),
            role: "command_center".to_string(),
            viewport: None,
        }
    }

    #[test]
    fn the_first_camera_is_revision_zero() {
        let hub = ControlHub::new(camera(-6.2));
        assert_eq!(hub.revision(), 0);
        assert_eq!(hub.camera(), camera(-6.2));
    }

    #[test]
    fn a_changed_camera_advances_the_revision_once() {
        let hub = ControlHub::new(camera(-6.2));
        assert_eq!(
            hub.report_camera(camera(-6.1), CameraSource::desktop()),
            Some(1)
        );
        assert_eq!(
            hub.report_camera(camera(-6.1), CameraSource::desktop()),
            None
        );
        assert_eq!(
            hub.report_camera(camera(-6.0), CameraSource::desktop()),
            Some(2)
        );
        assert_eq!(hub.revision(), 2);
        assert_eq!(hub.camera(), camera(-6.0));
    }

    #[test]
    fn a_resize_is_a_camera_change() {
        let hub = ControlHub::new(camera(-6.2));
        let mut resized = camera(-6.2);
        resized.width = 1280;
        assert_eq!(hub.report_camera(resized, CameraSource::desktop()), Some(1));
    }

    #[test]
    fn a_camera_change_names_the_client_and_command_that_moved_it() {
        let hub = ControlHub::new(camera(-6.2));
        let mut rx = hub.subscribe();
        let source = CameraSource {
            client_id: "phone".to_string(),
            command_id: Some("c-7".to_string()),
        };
        hub.report_camera(camera(-6.1), source);
        match rx.try_recv().expect("an event") {
            Event::CameraChanged {
                revision,
                source,
                command_id,
                view,
            } => {
                assert_eq!(revision, 1);
                assert_eq!(source.as_deref(), Some("phone"));
                assert_eq!(command_id.as_deref(), Some("c-7"));
                assert_eq!(view.id, MAIN_VIEW);
                assert_eq!(view.camera, camera(-6.1));
            }
            other => panic!("expected a camera event, got {other:?}"),
        }
    }

    #[test]
    fn an_unchanged_camera_publishes_nothing() {
        let hub = ControlHub::new(camera(-6.2));
        let mut rx = hub.subscribe();
        assert_eq!(
            hub.report_camera(camera(-6.2), CameraSource::desktop()),
            None
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_command_id_is_remembered_only_once() {
        let hub = ControlHub::new(camera(-6.2));
        assert!(!hub.remember_command("c-1"));
        assert!(hub.remember_command("c-1"));
        assert!(!hub.remember_command("c-2"));
    }

    #[test]
    fn remembered_command_ids_stay_bounded() {
        let hub = ControlHub::new(camera(-6.2));
        for index in 0..(SEEN_COMMAND_CAPACITY + 32) {
            let _ = hub.remember_command(&format!("c-{index}"));
        }
        let remembered = hub.lock().seen.len();
        assert!(
            remembered <= SEEN_COMMAND_CAPACITY,
            "the seen table grew past its bound: {remembered}"
        );
    }

    #[test]
    fn a_snapshot_carries_the_current_camera_and_revision() {
        let hub = ControlHub::new(camera(-6.2));
        hub.report_camera(camera(-6.1), CameraSource::desktop());
        match hub.snapshot_event() {
            Event::Snapshot {
                protocol_version,
                revision,
                views,
                session,
                readiness,
                displays,
                ..
            } => {
                assert_eq!(protocol_version, PROTOCOL_VERSION);
                assert_eq!(revision, 1);
                assert_eq!(views[MAIN_VIEW].camera, camera(-6.1));
                assert!(session.is_none());
                assert_eq!(readiness, Readiness::waiting(ReadinessReason::NoSession));
                assert!(displays.is_empty());
            }
            other => panic!("expected a snapshot, got {other:?}"),
        }
    }

    #[test]
    fn a_slow_subscriber_is_told_it_lagged_rather_than_missing_events() {
        let hub = ControlHub::new(camera(-6.2));
        let mut rx = hub.subscribe();
        for index in 0..(EVENT_BUFFER + 8) {
            hub.publish(Event::CommandAccepted {
                command_id: format!("c-{index}"),
                revision: index as u64,
            });
        }
        match rx.try_recv() {
            Err(broadcast::error::TryRecvError::Lagged(missed)) => assert!(missed > 0),
            other => panic!("expected a lag, got {other:?}"),
        }
    }

    #[test]
    fn subscribers_are_counted_while_they_are_held() {
        let hub = ControlHub::new(camera(-6.2));
        assert_eq!(hub.subscribers(), 0);
        let rx = hub.subscribe();
        assert_eq!(hub.subscribers(), 1);
        drop(rx);
        assert_eq!(hub.subscribers(), 0);
    }

    fn source(client: &str) -> CameraSource {
        CameraSource {
            client_id: client.to_string(),
            command_id: Some("c-1".to_string()),
        }
    }

    #[test]
    fn a_headless_nudge_moves_the_camera_west_with_the_shells_arithmetic() {
        let hub = ControlHub::new(camera(-6.2));
        let revision = hub.apply(
            &Command::CameraNudge {
                dx_fraction: 0.25,
                dy_fraction: 0.0,
            },
            source("phone"),
        );
        assert_eq!(revision, Some(1));
        let moved = hub.camera();
        assert!(
            moved.center.longitude < 106.8,
            "a positive dx_fraction moves the camera west: {moved:?}"
        );
        // The shell's own pan: the same call, the same numbers.
        let (latitude, longitude) =
            crate::map_geometry::pan_center((-6.2, 106.8), 11.0, 0.25 * 800.0, 0.0, 800.0, 600.0);
        assert!((moved.center.latitude - latitude).abs() < 1e-12);
        assert!((moved.center.longitude - longitude).abs() < 1e-12);
    }

    #[test]
    fn a_headless_zoom_is_centre_anchored() {
        let hub = ControlHub::new(camera(-6.2));
        assert_eq!(
            hub.apply(&Command::CameraZoom { delta: 1.5 }, source("phone")),
            Some(1)
        );
        assert_eq!(hub.camera().zoom, 12.5);
        assert_eq!(hub.camera().center, camera(-6.2).center);
    }

    #[test]
    fn a_headless_zoom_stops_at_the_shells_rails() {
        let hub = ControlHub::new(camera(-6.2));
        hub.apply(&Command::CameraZoom { delta: 100.0 }, source("phone"));
        assert_eq!(hub.camera().zoom, ZOOM_MAX);
        hub.apply(&Command::CameraZoom { delta: -100.0 }, source("phone"));
        assert_eq!(hub.camera().zoom, ZOOM_MIN);
    }

    #[test]
    fn a_headless_set_keeps_the_viewport_and_clamps_the_zoom() {
        let hub = ControlHub::new(camera(-6.2));
        let before = hub.camera();
        hub.apply(
            &Command::CameraSet {
                center: GeoPoint {
                    latitude: 51.5,
                    longitude: -0.12,
                },
                zoom: 99.0,
            },
            source("phone"),
        );
        let after = hub.camera();
        assert_eq!(after.center.latitude, 51.5);
        assert_eq!(after.zoom, ZOOM_MAX);
        assert_eq!((after.width, after.height), (before.width, before.height));
    }

    #[test]
    fn a_zoom_already_at_the_rail_publishes_nothing() {
        let hub = ControlHub::new(CameraSnapshot {
            zoom: ZOOM_MAX,
            ..camera(-6.2)
        });
        let mut rx = hub.subscribe();
        assert_eq!(
            hub.apply(&Command::CameraZoom { delta: 1.0 }, source("phone")),
            None
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_nudge_with_no_viewport_yet_moves_nothing() {
        let hub = ControlHub::new(CameraSnapshot {
            width: 0,
            height: 0,
            ..camera(-6.2)
        });
        assert_eq!(
            hub.apply(
                &Command::CameraNudge {
                    dx_fraction: 0.25,
                    dy_fraction: 0.0,
                },
                source("phone"),
            ),
            None
        );
        assert_eq!(hub.camera().center.longitude, 106.8);
    }

    #[test]
    fn a_viewport_report_is_a_change_and_a_repeat_is_not() {
        let hub = ControlHub::new(camera(-6.2));
        let report = Command::ViewReport {
            width: 1280,
            height: 720,
        };
        assert_eq!(hub.apply(&report, source("desktop")), Some(1));
        assert_eq!((hub.camera().width, hub.camera().height), (1280, 720));
        assert_eq!(hub.apply(&report, source("desktop")), None);
    }

    #[test]
    fn a_nudge_resolves_against_the_viewport_that_was_reported() {
        let hub = ControlHub::new(camera(-6.2));
        hub.apply(
            &Command::ViewReport {
                width: 1600,
                height: 1200,
            },
            source("desktop"),
        );
        let before = hub.camera().center.longitude;
        hub.apply(
            &Command::CameraNudge {
                dx_fraction: 0.25,
                dy_fraction: 0.0,
            },
            source("phone"),
        );
        // A quarter of a wider viewport is a bigger move in degrees, which is
        // exactly what "fraction of the viewport" promises.
        let wide = before - hub.camera().center.longitude;
        let narrow = 106.8 - {
            let narrow_hub = ControlHub::new(camera(-6.2));
            narrow_hub.apply(
                &Command::CameraNudge {
                    dx_fraction: 0.25,
                    dy_fraction: 0.0,
                },
                source("phone"),
            );
            narrow_hub.camera().center.longitude
        };
        assert!(wide > narrow, "wide {wide} should exceed narrow {narrow}");
    }

    #[test]
    fn an_applied_command_is_attributed_to_the_client_that_sent_it() {
        let hub = ControlHub::new(camera(-6.2));
        let mut rx = hub.subscribe();
        hub.apply(&Command::CameraZoom { delta: 1.0 }, source("phone"));
        match rx.try_recv().expect("an event") {
            Event::CameraChanged {
                source: who,
                command_id,
                ..
            } => {
                assert_eq!(who.as_deref(), Some("phone"));
                assert_eq!(command_id.as_deref(), Some("c-1"));
            }
            other => panic!("expected a camera event, got {other:?}"),
        }
    }

    // The observer view -----------------------------------------------------

    #[test]
    fn a_fresh_hub_waits_for_a_session() {
        let hub = ControlHub::new(camera(-6.2));
        assert_eq!(hub.readiness(), Readiness::waiting(ReadinessReason::NoSession));
        assert!(hub.camera_of(OBSERVER_VIEW).is_none());
    }

    #[test]
    fn an_observer_view_appears_when_a_display_attaches_and_copies_the_console() {
        let hub = ControlHub::new(camera(-6.2));
        assert_eq!(hub.attach_display(display("wall-1")), Some(1));
        assert_eq!(hub.camera_of(OBSERVER_VIEW), Some(camera(-6.2)));
        // The console's camera is untouched.
        assert_eq!(hub.camera(), camera(-6.2));
    }

    #[test]
    fn attaching_a_display_says_so_and_names_the_observer_view() {
        let hub = ControlHub::new(camera(-6.2));
        let mut rx = hub.subscribe();
        hub.attach_display(display("wall-1"));
        match rx.try_recv().expect("an event") {
            Event::SessionChanged { displays, .. } => {
                assert_eq!(displays, vec![display("wall-1")]);
            }
            other => panic!("expected a session event, got {other:?}"),
        }
        match rx.try_recv().expect("a camera event") {
            Event::CameraChanged { view, .. } => assert_eq!(view.id, OBSERVER_VIEW),
            other => panic!("expected a camera event, got {other:?}"),
        }
    }

    #[test]
    fn only_the_observer_view_may_be_created_on_demand() {
        let hub = ControlHub::new(camera(-6.2));
        assert_eq!(
            hub.apply_view("sideboard", &Command::CameraZoom { delta: 1.0 }, source("x")),
            Err(ApplyError::UnknownView("sideboard".to_string()))
        );
        assert_eq!(
            hub.apply_view(OBSERVER_VIEW, &Command::CameraZoom { delta: 1.0 }, source("x"))
                .expect("the observer is created on demand"),
            Some(1)
        );
        let observer = hub.camera_of(OBSERVER_VIEW).expect("observer");
        assert_eq!(observer.center, camera(-6.2).center);
    }

    #[test]
    fn a_phone_moves_the_observer_and_the_console_stays_put() {
        let hub = ControlHub::new(camera(-6.2));
        hub.attach_display(display("wall-1"));
        assert_eq!(
            hub.apply_view(OBSERVER_VIEW, &Command::CameraZoom { delta: 1.0 }, source("phone")),
            Ok(Some(2))
        );
        assert_eq!(hub.camera_of(OBSERVER_VIEW).expect("observer").zoom, 12.0);
        assert_eq!(hub.camera(), camera(-6.2));
    }

    #[test]
    fn a_new_exercise_opens_where_the_operator_is_looking() {
        let hub = ControlHub::new(camera(-6.2));
        hub.attach_display(display("wall-1"));
        let _ = hub.apply_view(OBSERVER_VIEW, &Command::CameraZoom { delta: 4.0 }, source("phone"));
        hub.set_session(session("execution", vec![unit(1, "Evertsen", Some(-6.2))]));
        assert_eq!(hub.camera_of(OBSERVER_VIEW), Some(camera(-6.2)));
    }

    #[test]
    fn the_same_exercise_being_updated_leaves_the_observer_alone() {
        let hub = ControlHub::new(camera(-6.2));
        hub.attach_display(display("wall-1"));
        let _ = hub.apply_view(OBSERVER_VIEW, &Command::CameraZoom { delta: 4.0 }, source("phone"));
        hub.set_session(session("execution", vec![unit(1, "Evertsen", Some(-6.2))]));
        let moved = hub.camera_of(OBSERVER_VIEW).expect("observer");
        hub.set_session(session("execution", vec![unit(1, "Evertsen", Some(-6.1))]));
        assert_eq!(hub.camera_of(OBSERVER_VIEW), Some(moved));
    }

    // Settings --------------------------------------------------------------

    /// THE TEXT SIZE IS A BROADCAST FACT, not a local preference. A phone's
    /// `settings.font` used to stop at the console: the hub recorded the step
    /// and told nobody, so every wall kept drawing at its own default.
    #[test]
    fn a_text_size_change_reaches_every_watcher_once() {
        let hub = ControlHub::new(camera(-6.2));
        let mut events = hub.subscribe();
        assert_eq!(hub.settings().step, crate::view::TEXT_SCALE_DEFAULT);

        assert_eq!(hub.set_text_scale_step(3), Some(1), "a change spends a revision");
        match events.try_recv().expect("the stream carries it") {
            Event::SettingsChanged { revision, settings } => {
                assert_eq!(revision, 1);
                assert_eq!(settings.step, 3);
                assert_eq!(settings.text_scale, crate::view::text_scale_at(3));
            }
            other => panic!("expected settings.changed, got {other:?}"),
        }

        // The same step again changes nothing, so it announces nothing.
        assert_eq!(hub.set_text_scale_step(3), None);
        assert!(events.try_recv().is_err(), "an idempotent set is silent");

        // A snapshot carries it, so a wall that has just connected draws at
        // the host's size rather than its own.
        match hub.snapshot_event() {
            Event::Snapshot { settings, .. } => {
                assert_eq!(settings.step, 3);
                assert_eq!(settings.text_scale, crate::view::text_scale_at(3));
            }
            other => panic!("expected a snapshot, got {other:?}"),
        }

        // Off the ladder is the last rung, never a scale nobody can operate.
        assert_eq!(hub.set_text_scale_step(0), Some(2));
        assert_eq!(hub.set_text_scale_step(99), Some(3));
        assert_eq!(hub.settings().step, crate::view::TEXT_SCALES.len() - 1);
    }

    // Readiness -------------------------------------------------------------

    #[test]
    fn readiness_walks_from_no_session_to_ready() {
        let hub = ControlHub::new(camera(-6.2));
        assert_eq!(hub.readiness().reason, Some(ReadinessReason::NoSession));

        hub.set_session(session("planning", vec![unit(1, "Evertsen", None)]));
        assert_eq!(hub.readiness().reason, Some(ReadinessReason::NotExecution));

        hub.set_session(session("execution", vec![unit(1, "Evertsen", None)]));
        assert_eq!(hub.readiness().reason, Some(ReadinessReason::NoDisplay));

        hub.attach_display(display("wall-1"));
        assert_eq!(hub.readiness(), Readiness::ready());

        hub.detach_display("wall-1");
        assert_eq!(hub.readiness().reason, Some(ReadinessReason::NoDisplay));
    }

    #[test]
    fn the_phone_opens_a_window_and_the_hub_remembers_it() {
        let hub = ControlHub::new(camera(-6.2));
        hub.set_session(session("execution", vec![unit(1, "Evertsen", Some(-6.2))]));
        hub.attach_display(display("wall-1"));
        assert!(!hub.ui().panels.crew);

        let mut events = hub.subscribe();
        let revision = hub
            .apply_view(
                OBSERVER_VIEW,
                &Command::UiSet {
                    panel: "crew".to_string(),
                    open: true,
                },
                CameraSource::desktop(),
            )
            .expect("accepted");
        assert!(revision.is_some(), "opening a window is a state change");
        assert!(hub.ui().panels.crew);

        // The event every wall draws from carries the window.
        let event = events.try_recv().expect("an event");
        let Event::SessionChanged { ui, revision, .. } = event else {
            panic!("a panel toggle is a session event");
        };
        assert!(ui.panels.crew);
        assert_eq!(revision, revision);

        // The same toggle again is not a change.
        let again = hub
            .apply_view(
                OBSERVER_VIEW,
                &Command::UiSet {
                    panel: "crew".to_string(),
                    open: true,
                },
                CameraSource::desktop(),
            )
            .expect("accepted");
        assert!(again.is_none());

        // A focus names the hull the popup is about.
        hub.apply_view(
            OBSERVER_VIEW,
            &Command::CameraFocus {
                unit_id: 1,
                zoom: None,
            },
            CameraSource::desktop(),
        )
        .expect("focus resolves");
        assert_eq!(hub.ui().focus, Some(1));

        // A different exercise closes the windows.
        let mut next = session("execution", vec![unit(2, "Tromp", Some(-6.1))]);
        next.game_id = 43;
        hub.set_session(next);
        assert!(!hub.ui().panels.crew);
        assert_eq!(hub.ui().focus, None);
    }

    #[test]
    fn an_expired_session_is_stale_rather_than_gone() {
        let hub = ControlHub::new(camera(-6.2));
        hub.set_session(session("execution", vec![unit(1, "Evertsen", None)]));
        hub.attach_display(display("wall-1"));
        assert!(hub.readiness().ready);

        // Backdate the session rather than sleeping: the clock is the input.
        {
            let mut shared = hub.lock();
            let record = shared.session.as_mut().expect("a session");
            record.last_seen = Instant::now() - (SESSION_TTL + Duration::from_secs(1));
        }
        let readiness = hub.readiness();
        assert_eq!(readiness.reason, Some(ReadinessReason::Stale));
        // The units are still there: a stale map beats an empty one.
        assert_eq!(hub.session().expect("session").units.len(), 1);
    }

    #[test]
    fn a_heartbeat_revives_a_stale_session_once() {
        let hub = ControlHub::new(camera(-6.2));
        hub.set_session(session("execution", vec![unit(1, "Evertsen", None)]));
        hub.attach_display(display("wall-1"));
        {
            let mut shared = hub.lock();
            let record = shared.session.as_mut().expect("a session");
            record.last_seen = Instant::now() - (SESSION_TTL + Duration::from_secs(1));
        }
        assert_eq!(hub.readiness().reason, Some(ReadinessReason::Stale));

        let revision = hub.revision();
        assert_eq!(hub.touch_session(), Some(revision + 1));
        assert!(hub.readiness().ready);
        // A second heartbeat while healthy is not a change.
        assert_eq!(hub.touch_session(), None);
    }

    #[test]
    fn republishing_an_unchanged_session_is_a_heartbeat_not_an_event() {
        let hub = ControlHub::new(camera(-6.2));
        let exercise = session("execution", vec![unit(1, "Evertsen", None)]);
        assert!(hub.set_session(exercise.clone()).is_some());
        let mut rx = hub.subscribe();
        assert_eq!(hub.set_session(exercise), None);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn unit_numbers_come_from_array_order() {
        let hub = ControlHub::new(camera(-6.2));
        hub.set_session(session(
            "execution",
            vec![unit(9, "Evertsen", None), unit(4, "Tromp", None)],
        ));
        let units = hub.session().expect("session").units;
        assert_eq!(units[0].index, 1);
        assert_eq!(units[1].index, 2);
        assert_eq!(units[0].id, 9);
        assert_eq!(units[1].id, 4);
    }

    #[test]
    fn clearing_a_session_keeps_the_observer_camera() {
        let hub = ControlHub::new(camera(-6.2));
        hub.attach_display(display("wall-1"));
        hub.set_session(session("execution", vec![unit(1, "Evertsen", Some(-6.2))]));
        let _ = hub.apply_view(OBSERVER_VIEW, &Command::CameraZoom { delta: 3.0 }, source("phone"));
        let moved = hub.camera_of(OBSERVER_VIEW).expect("observer");

        assert!(hub.clear_session().is_some());
        assert!(hub.session().is_none());
        assert_eq!(hub.readiness().reason, Some(ReadinessReason::NoSession));
        assert_eq!(hub.camera_of(OBSERVER_VIEW), Some(moved));
        // Clearing nothing is not a change.
        assert_eq!(hub.clear_session(), None);
    }

    // Focus -----------------------------------------------------------------

    #[test]
    fn a_focus_puts_the_observer_on_the_unit_without_moving_the_console() {
        let hub = ControlHub::new(camera(-6.2));
        hub.attach_display(display("wall-1"));
        hub.set_session(session(
            "execution",
            vec![unit(1, "Evertsen", Some(-6.2)), unit(2, "Tromp", Some(-5.9))],
        ));
        assert_eq!(
            hub.apply_view(
                OBSERVER_VIEW,
                &Command::CameraFocus {
                    unit_id: 2,
                    zoom: Some(13.0),
                },
                source("phone"),
            )
            .expect("focus resolves")
            .is_some(),
            true
        );
        let observer = hub.camera_of(OBSERVER_VIEW).expect("observer");
        assert_eq!(observer.center.latitude, -5.9);
        assert_eq!(observer.zoom, 13.0);
        // The viewport the command center reported is preserved.
        assert_eq!((observer.width, observer.height), (800, 600));
        assert_eq!(hub.camera(), camera(-6.2));
    }

    #[test]
    fn a_focus_without_a_zoom_leaves_the_scale_alone() {
        let hub = ControlHub::new(camera(-6.2));
        hub.attach_display(display("wall-1"));
        hub.set_session(session("execution", vec![unit(1, "Evertsen", Some(-5.0))]));
        hub.apply_view(
            OBSERVER_VIEW,
            &Command::CameraFocus {
                unit_id: 1,
                zoom: None,
            },
            source("phone"),
        )
        .expect("focus resolves");
        let observer = hub.camera_of(OBSERVER_VIEW).expect("observer");
        assert_eq!(observer.center.latitude, -5.0);
        assert_eq!(observer.zoom, 11.0);
    }

    #[test]
    fn a_focus_names_the_unit_it_cannot_find() {
        let hub = ControlHub::new(camera(-6.2));
        hub.attach_display(display("wall-1"));
        hub.set_session(session("execution", vec![unit(1, "Evertsen", None)]));

        assert_eq!(
            hub.apply_view(
                OBSERVER_VIEW,
                &Command::CameraFocus {
                    unit_id: 99,
                    zoom: None
                },
                source("phone"),
            ),
            Err(ApplyError::UnknownUnit(99))
        );
        assert_eq!(
            hub.apply_view(
                OBSERVER_VIEW,
                &Command::CameraFocus {
                    unit_id: 1,
                    zoom: None
                },
                source("phone"),
            ),
            Err(ApplyError::UnitHasNoPosition(1))
        );
        // A refused focus moved nothing and advanced nothing.
        assert_eq!(hub.camera_of(OBSERVER_VIEW), Some(camera(-6.2)));
        assert_eq!(hub.revision(), 2);
    }

    #[test]
    fn a_focus_on_a_unit_already_centred_is_a_no_op() {
        let hub = ControlHub::new(camera(-6.2));
        hub.attach_display(display("wall-1"));
        hub.set_session(session("execution", vec![unit(1, "Evertsen", Some(-6.2))]));
        let focus = Command::CameraFocus {
            unit_id: 1,
            zoom: Some(11.0),
        };
        assert_eq!(
            hub.apply_view(OBSERVER_VIEW, &focus, source("phone")),
            Ok(None)
        );
    }

    // Expiry ----------------------------------------------------------------

    #[test]
    fn an_expired_display_is_reaped_and_readiness_falls_back() {
        let hub = ControlHub::new(camera(-6.2));
        hub.set_session(session("execution", vec![unit(1, "Evertsen", None)]));
        hub.attach_display(display("wall-1"));
        assert!(hub.readiness().ready);

        {
            let mut shared = hub.lock();
            let record = shared.displays.get_mut("wall-1").expect("a display");
            record.last_seen = Instant::now() - (DISPLAY_TTL + Duration::from_secs(1));
        }
        // The verdict is already wrong without a tick, because it is computed.
        assert_eq!(hub.readiness().reason, Some(ReadinessReason::NoDisplay));
        assert!(hub.displays().is_empty());
    }

    #[test]
    fn expiring_twice_is_one_change() {
        let hub = ControlHub::new(camera(-6.2));
        hub.set_session(session("execution", vec![unit(1, "Evertsen", None)]));
        hub.attach_display(display("wall-1"));
        {
            let mut shared = hub.lock();
            let record = shared.displays.get_mut("wall-1").expect("a display");
            record.last_seen = Instant::now() - (DISPLAY_TTL + Duration::from_secs(1));
        }
        assert!(hub.expire());
        assert!(!hub.expire());
    }

    #[test]
    fn a_live_display_is_not_reaped() {
        let hub = ControlHub::new(camera(-6.2));
        hub.attach_display(display("wall-1"));
        let revision = hub.revision();
        hub.touch_display("wall-1");
        assert!(!hub.expire());
        assert_eq!(hub.revision(), revision);
        assert_eq!(hub.displays().len(), 1);
    }
}
