//! The shared control state: one writer, one monotonic revision, one fan-out.
//!
//! [`ControlHub`] is the single owner of the camera that clients read, and it
//! has exactly one writer per deployment:
//!
//! - **Embedded**: the desktop's order thread (`report_camera`), because it is
//!   the thread that already owns the camera tuple and the map request. The
//!   camera in the hub is therefore always one the shell is rendering.
//! - **Headless** (`tfg-server`): the command thread (`apply`), which resolves
//!   a semantic command with the same arithmetic the shell uses.
//!
//! Either way the server threads only read (`camera`, `snapshot_event`) and
//! subscribe (`subscribe`). That is the single-writer rule from the
//! control-plane ADR, enforced by the API rather than by convention.
//!
//! Idempotency lives here too: the hub remembers recent command ids, so a
//! client that retries a POST after a timeout does not move the camera twice.

use super::protocol::{
    CameraSnapshot, CameraSource, Command, Event, GeoPoint, MAIN_VIEW, PROTOCOL_VERSION, ViewState,
};
use crate::map_geometry::pan_center;
use crate::view::{ZOOM_MAX, ZOOM_MIN};
use std::collections::{BTreeMap, VecDeque};
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

#[derive(Debug)]
struct Shared {
    camera: CameraSnapshot,
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
                camera,
                revision: 0,
                seen: VecDeque::new(),
            })),
            events,
        }
    }

    /// A poisoned lock here holds a camera and a revision, not a half-applied
    /// transaction, so recovering is honest: the state is still whole.
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The camera the desktop currently renders.
    pub fn camera(&self) -> CameraSnapshot {
        self.lock().camera
    }

    /// The revision of the shared state. Monotonic; 0 before the first change.
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// The whole shared state, as the WebSocket snapshot and `/v1/state`.
    pub fn snapshot_event(&self) -> Event {
        let shared = self.lock();
        let view = ViewState {
            id: MAIN_VIEW.to_string(),
            camera: shared.camera,
        };
        Event::Snapshot {
            protocol_version: PROTOCOL_VERSION,
            revision: shared.revision,
            views: BTreeMap::from([(MAIN_VIEW.to_string(), view)]),
        }
    }

    /// Every event fans out from here, including a behind-the-back publish
    /// from the host. A closed set of subscribers is not an error.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    /// A new camera from the desktop, with the client (or hand) that moved it.
    ///
    /// Returns the new revision when the camera actually changed, and `None`
    /// when the tick reported the camera it already had — which is most ticks,
    /// because the shell tells the binary where the camera is on every one. A
    /// resize counts: the viewport is part of the camera, and a remote nudge
    /// resolves against it.
    pub fn report_camera(&self, camera: CameraSnapshot, source: CameraSource) -> Option<u64> {
        self.commit(camera, source)
    }

    /// Resolve a camera command against the shared state.
    ///
    /// The desktop does not need this: it moves its own camera and reports the
    /// result, so the camera in the hub is always one it is rendering. A
    /// headless server has no camera to ask, so the command is applied here —
    /// with the same Mercator arithmetic and the same zoom rails the shell
    /// uses, because a reducer that clamped differently from the renderer
    /// would publish a camera nobody draws.
    ///
    /// Returns the revision when something changed, `None` when the command was
    /// a no-op (already at the zoom rail, a nudge of zero, a viewport that is
    /// the one already reported). An `AcceptedCommand` whose command was a
    /// no-op is still *accepted*; it simply does not move anything.
    pub fn apply(&self, command: &Command, source: CameraSource) -> Option<u64> {
        let current = self.camera();
        let next = match command {
            Command::CameraSet { center, zoom } => CameraSnapshot {
                center: *center,
                zoom: zoom.clamp(ZOOM_MIN, ZOOM_MAX),
                ..current
            },
            Command::CameraNudge {
                dx_fraction,
                dy_fraction,
            } => {
                let (width, height) = (f64::from(current.width), f64::from(current.height));
                if width <= 0.0 || height <= 0.0 {
                    // No display has said how big it is: a fraction of a
                    // viewport nobody has named has no meaning.
                    return None;
                }
                let (latitude, longitude) = pan_center(
                    (current.center.latitude, current.center.longitude),
                    current.zoom,
                    dx_fraction * width,
                    dy_fraction * height,
                    width,
                    height,
                );
                CameraSnapshot {
                    center: GeoPoint {
                        latitude,
                        longitude,
                    },
                    ..current
                }
            }
            Command::CameraZoom { delta } => CameraSnapshot {
                zoom: (current.zoom + delta).clamp(ZOOM_MIN, ZOOM_MAX),
                ..current
            },
            Command::ViewReport { width, height } => CameraSnapshot {
                width: *width,
                height: *height,
                ..current
            },
        };
        self.commit(next, source)
    }

    /// The one place the shared camera changes. Equality is the coalescing
    /// rule: a report that matches costs nothing, which is what makes an
    /// optimistic client safe — a display that follows a command and then
    /// reports the camera it followed publishes nothing.
    fn commit(&self, camera: CameraSnapshot, source: CameraSource) -> Option<u64> {
        let revision = {
            let mut shared = self.lock();
            if shared.camera == camera {
                return None;
            }
            shared.camera = camera;
            shared.revision += 1;
            shared.revision
        };
        let _ = self.events.send(Event::CameraChanged {
            revision,
            source: Some(source.client_id),
            command_id: source.command_id,
            view: ViewState {
                id: MAIN_VIEW.to_string(),
                camera,
            },
        });
        Some(revision)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::protocol::GeoPoint;

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
            } => {
                assert_eq!(protocol_version, PROTOCOL_VERSION);
                assert_eq!(revision, 1);
                assert_eq!(views[MAIN_VIEW].camera, camera(-6.1));
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
}
