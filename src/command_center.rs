//! The command center: a display, not a console.
//!
//! This is the surface a phone remotes. It renders one view — `observer` — and
//! nothing else: no roster, no composer, no clock, no Minos credential. The
//! binary branches here before it reads the keyring or signs in, which is what
//! makes that claim true by construction rather than by discipline.
//!
//! Three facts arrive from three places, and this module joins them:
//!
//! - the **session** and its readiness come up the control socket (the console
//!   that holds the exercise publishes them; this module is a follower);
//! - the **picture** comes from the same map thread the console uses, asked for
//!   at this window's size and the observer camera;
//! - the **markers** are the session's units projected at that camera, so a
//!   phone's `camera.focus` moves the hulls on this wall without this module
//!   knowing what a focus command is.
//!
//! What it deliberately does not do: take a camera gesture (it is a display,
//! not a controller), read Minos, or invent an OK verb before the protocol has
//! one.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Application, Bounds, Context, IntoElement, ParentElement, Render, Styled,
    StatefulInteractiveElement, StyledImage, TitlebarOptions, Window, WindowBounds, WindowOptions,
    canvas, div, img, point, prelude::*, px, size,
};

use crate::control::{
    AcceptedCommand, CameraSnapshot, ClientConfig, ClientHandle, Command, DisplayHello, Event,
    BoardState, GeoPoint, OBSERVER_VIEW, ObserverUi, Readiness, ReadinessReason, SessionState,
    SettingsState, UnitState, ViewState, spawn_client,
};
use crate::map_gate::FrameSlot;
use crate::map_geometry;
use crate::shell_gpui::{PHOTO_ZOOM, paint_unit_mark, plural};
use crate::shell_threads::{MapReq, MapResp};
use crate::view;
use crate::view::MapSymbolShape;

/// Where this display follows, and what it calls itself.
#[derive(Clone, Debug)]
pub struct CommandCenterConfig {
    pub server: String,
    pub token: String,
    /// Stable across reconnects: the server replaces the record rather than
    /// stacking a second display.
    pub display_id: String,
    pub label: String,
}

impl CommandCenterConfig {
    /// Read the endpoint from the same two variables the console uses.
    ///
    /// A command center must be pointed at a host; there is no sensible
    /// default, and a wall that silently watched `127.0.0.1` would be a wall
    /// showing nothing with no explanation.
    pub fn from_env() -> Result<Self, String> {
        let server = std::env::var("TFG_CONTROL_SERVER")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                "TFG_CONTROL_SERVER is not set; a command center must be told which console to follow"
                    .to_string()
            })?;
        let token = std::env::var("TFG_CONTROL_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "TFG_CONTROL_TOKEN is not set".to_string())?;
        let label = std::env::var("TFG_DISPLAY_LABEL")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "command center".to_string());
        let display_id = std::env::var("TFG_DISPLAY_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "command-center".to_string());
        Ok(Self {
            server,
            token,
            display_id,
            label,
        })
    }
}

/// Run the command center until its window closes.
/// One hull's photograph, decoded and in the byte order the atlas takes.
struct Picture {
    width: u32,
    height: u32,
    /// BGRA, the order `RenderImage` expects (the console swizzles the same
    /// way, and `turn_to_size` carries the four channels untouched).
    bgra: Vec<u8>,
}

/// Fetch hull photographs from the control server, once each.
///
/// The wall has no Minos credential by design; the console publishes what it
/// already fetched, and this thread asks for the hulls the session names. One
/// request per hull, ever: a picture does not change under an exercise.
fn spawn_picture_fetch(
    server: String,
    token: String,
    requests: Receiver<Vec<i64>>,
    pictures: Sender<(i64, Picture)>,
) {
    std::thread::spawn(move || {
        let Ok(http) = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
        else {
            return;
        };
        for batch in requests {
            for unit_id in batch {
                let url = format!("{server}/v1/units/{unit_id}/picture");
                let Ok(response) = http.get(&url).bearer_auth(&token).send() else {
                    // The console publishes a picture when it fetches one;
                    // a hull without a photograph has no route at all, and a
                    // 404 is the ordinary answer.
                    continue;
                };
                if !response.status().is_success() {
                    continue;
                }
                let Ok(bytes) = response.bytes() else { continue };
                let Ok(image) = image::load_from_memory(&bytes) else {
                    continue;
                };
                let rgba = image.to_rgba8();
                let (width, height) = rgba.dimensions();
                let mut bgra = rgba.into_raw();
                crate::map_gate::swizzle_rgba_to_bgra(&mut bgra);
                let _ = pictures.send((
                    unit_id,
                    Picture {
                        width,
                        height,
                        bgra,
                    },
                ));
            }
        }
    });
}

pub fn run(
    config: CommandCenterConfig,
    map_tx: Sender<MapReq>,
    map_rx: Receiver<MapResp>,
) -> Result<(), String> {
    let (commands_tx, commands_rx) = std::sync::mpsc::channel::<AcceptedCommand>();
    let (events_tx, events_rx) = std::sync::mpsc::channel::<Event>();
    let mut client_config = ClientConfig::new(config.server.clone(), config.token.clone());
    // The observer view is the one a phone may move. A command center that
    // followed `main` would show the game master's camera and fight it.
    client_config.view = OBSERVER_VIEW.to_string();
    client_config.display = Some(DisplayHello {
        id: Some(config.display_id.clone()),
        role: crate::control::COMMAND_CENTER_ROLE.to_string(),
        label: config.label.clone(),
        viewport: None,
    });
    client_config.events = Some(events_tx);
    let client = spawn_client(client_config, commands_tx)?;
    let server = config.server.clone();
    let (picture_request_tx, picture_request_rx) = std::sync::mpsc::channel::<Vec<i64>>();
    let (picture_tx, picture_rx) = std::sync::mpsc::channel::<(i64, Picture)>();
    spawn_picture_fetch(
        server.clone(),
        config.token.clone(),
        picture_request_rx,
        picture_tx,
    );

    Application::new().run(move |cx: &mut App| {
        register_fonts(cx);
        let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("tfg command center".into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            // The capture harness finds this window by class, the same way it
            // finds the console; the title is what tells a person which one it
            // is.
            app_id: Some("tfg-gpui".to_string()),
            ..Default::default()
        };
        let entity = cx.new(|_| {
            CommandCenter::new(
                server,
                client,
                events_rx,
                commands_rx,
                map_tx,
                map_rx,
                picture_request_tx,
                picture_rx,
            )
        });
        let window = cx.open_window(opts, |_window, _cx| entity.clone());
        if window.is_err() {
            eprintln!("command center: the window could not be opened");
            cx.quit();
            return;
        }
        let tick = entity.clone();
        cx.spawn(async move |cx: &mut gpui::AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                let alive = cx.update(|app| {
                    tick.update(app, |center: &mut CommandCenter, cx| {
                        center.poll();
                        cx.notify();
                    })
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
    });
    Ok(())
}

fn register_fonts(cx: &mut App) {
    use std::borrow::Cow;
    // The console's two faces, registered here rather than shared: this module
    // must not reach into the shell, and two registrations of the same bytes
    // is a few hundred kilobytes for a display that must look like the same
    // instrument.
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(
                include_bytes!("../assets/fonts/SairaCondensed-Regular.ttf").as_slice(),
            ),
            Cow::Borrowed(
                include_bytes!("../assets/fonts/SairaCondensed-SemiBold.ttf").as_slice(),
            ),
            Cow::Borrowed(include_bytes!("../assets/fonts/PTMono-Regular.ttf").as_slice()),
        ])
        .expect("bundled fonts register");
}

struct CommandCenter {
    server: String,
    client: ClientHandle,
    events: Receiver<Event>,
    commands: Receiver<AcceptedCommand>,
    map_tx: Sender<MapReq>,
    map_rx: Receiver<MapResp>,

    /// What the host published. `None` until the first snapshot: "not heard
    /// from" and "no exercise" are different waiting screens.
    session: Option<SessionState>,
    readiness: Readiness,
    /// The phone's windows and the hull its popup is about.
    ui: ObserverUi,
    heard_from_host: bool,

    /// The observer camera, as the host has it.
    camera: CameraSnapshot,
    /// The size of the picture, in device pixels, and the camera it was asked
    /// at. Markers are projected into this space and scaled to the window.
    map_px: (u32, u32),
    map_dims: (f32, f32),
    asked: Option<((f64, f64), f64, (u32, u32))>,
    seq: u64,

    frame: FrameSlot,
    incoming: Option<(u64, u32, u32, Vec<u8>, (f64, f64), f64)>,
    /// Hull photographs, as fetched from the control server.
    pictures: HashMap<i64, Picture>,
    /// When each hull's picture was last asked for. A wall can start before
    /// the console has fetched a picture, so a missing one is asked again —
    /// slowly, not once per tick.
    picture_asked: HashMap<i64, Instant>,
    picture_request: Sender<Vec<i64>>,
    picture_rx: Receiver<(i64, Picture)>,
    /// Turned pictures, keyed by hull, raster size and whole degree. The
    /// console's cache, with the same reason: a turn is expensive and a hull
    /// on one course asks for the same raster every frame.
    turned: HashMap<(i64, u32, u32, u16), Arc<gpui::RenderImage>>,
    turned_bytes: usize,
    /// Where each hull is being drawn, and where it is going.
    ///
    /// The session's positions arrive once a second; the wall draws at fifty
    /// frames. Without this a marker jumps a whole second's leg at once, which
    /// at a close zoom reads as a hull teleporting. The blend is the same rule
    /// the console uses (ADR-0011): display-only, between two accepted
    /// positions, never a new fact.
    glide: HashMap<i64, Glide>,
}

#[derive(Debug, Clone, Copy)]
struct Glide {
    from: GeoPoint,
    to: GeoPoint,
    at: Instant,
    /// The interval the last two positions actually arrived at, so the blend
    /// finishes as the next one lands.
    gap: f64,
}

impl CommandCenter {
    #[allow(clippy::too_many_arguments)]
    fn new(
        server: String,
        client: ClientHandle,
        events: Receiver<Event>,
        commands: Receiver<AcceptedCommand>,
        map_tx: Sender<MapReq>,
        map_rx: Receiver<MapResp>,
        picture_request: Sender<Vec<i64>>,
        picture_rx: Receiver<(i64, Picture)>,
    ) -> Self {
        Self {
            server,
            client,
            events,
            commands,
            map_tx,
            map_rx,
            session: None,
            readiness: Readiness::waiting(ReadinessReason::NoSession),
            ui: ObserverUi::default(),
            heard_from_host: false,
            camera: CameraSnapshot::from_tuple((
                map_geometry::MAP_CENTER.0,
                map_geometry::MAP_CENTER.1,
                map_geometry::MAP_ZOOM,
                map_geometry::MAP_W as u32,
                map_geometry::MAP_H as u32,
            )),
            map_px: (0, 0),
            map_dims: (map_geometry::MAP_W as f32, map_geometry::MAP_H as f32),
            asked: None,
            seq: 0,
            frame: FrameSlot::new(),
            incoming: None,
            pictures: HashMap::new(),
            picture_asked: HashMap::new(),
            picture_request,
            picture_rx,
            turned: HashMap::new(),
            turned_bytes: 0,
            glide: HashMap::new(),
        }
    }

    /// THE HOST'S TEXT SIZE, applied before the next frame.
    ///
    /// Every size the wall draws goes through [`ts`], so one step moves the
    /// strip, the three windows and the unit card together — the same rule
    /// the console follows for its own window. The scale is a process-wide
    /// atomic because that is what the console's ladder is; this display is
    /// its own process, so nothing else reads it.
    fn note_settings(&mut self, settings: SettingsState) {
        let scale = view::text_scale_at(settings.step);
        if (view::text_scale() - scale).abs() < f32::EPSILON {
            return;
        }
        view::set_text_scale(scale);
    }

    /// Everything that can change without a gesture: the host's events, the
    /// camera a phone moved, and the newest picture. Called on a timer, which
    /// is why it drains rather than waits.
    fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.heard_from_host = true;
            match event {
                Event::Snapshot {
                    views,
                    session,
                    readiness,
                    ui,
                    settings,
                    ..
                } => {
                    self.note_positions(session.as_ref());
                    self.session = session;
                    self.readiness = readiness;
                    self.ui = ui;
                    self.note_settings(settings);
                    if let Some(view) = views
                        .get(OBSERVER_VIEW)
                        .or_else(|| views.get(crate::control::MAIN_VIEW))
                    {
                        self.follow(view);
                    }
                }
                Event::SessionChanged {
                    session,
                    readiness,
                    ui,
                    ..
                } => {
                    self.note_positions(session.as_ref());
                    self.session = session;
                    self.readiness = readiness;
                    self.ui = ui;
                }
                Event::SettingsChanged { settings, .. } => self.note_settings(settings),
                Event::CameraChanged { view, .. } => {
                    if view.id == OBSERVER_VIEW {
                        self.follow(&view);
                    }
                }
                _ => {}
            }
        }
        while let Ok(command) = self.commands.try_recv() {
            // A camera the host asked for. This display has no gestures of its
            // own, so every accepted camera is the host's and there is no
            // report to send back.
            if command.view == OBSERVER_VIEW
                && let Command::CameraSet { center, zoom } = command.command
            {
                self.camera.center = center;
                self.camera.zoom = zoom;
                self.asked = None;
                self.client.followed(self.camera);
            }
        }
        // Pictures the fetcher landed: kept for the mark's photo LOD.
        while let Ok((unit_id, picture)) = self.picture_rx.try_recv() {
            self.pictures.insert(unit_id, picture);
        }
        // Ask for the pictures of every hull the session names, once.
        if let Some(session) = &self.session {
            let now = Instant::now();
            let wanted: Vec<i64> = session
                .units
                .iter()
                .filter(|unit| unit.position.is_some())
                .filter(|unit| !self.pictures.contains_key(&unit.id))
                .filter(|unit| {
                    self.picture_asked
                        .get(&unit.id)
                        .is_none_or(|at| now.saturating_duration_since(*at) >= Duration::from_secs(5))
                })
                .map(|unit| unit.id)
                .collect();
            if !wanted.is_empty() {
                for id in &wanted {
                    self.picture_asked.insert(*id, now);
                }
                let _ = self.picture_request.send(wanted);
            }
        }
        while let Ok((no, at, zoom, (w, h), mut rgba)) = self.map_rx.try_recv() {
            crate::map_gate::swizzle_rgba_to_bgra(&mut rgba);
            self.incoming = Some((no, w, h, rgba, at, zoom));
        }
    }

    /// Start a new glide for every hull whose position moved.
    ///
    /// Only a change starts one: a `session.changed` that carries a readiness
    /// flip leaves the markers where they are, and a hull standing still must
    /// not shimmer just because events keep arriving.
    fn note_positions(&mut self, session: Option<&SessionState>) {
        let now = Instant::now();
        let Some(session) = session else {
            self.glide.clear();
            return;
        };
        let live: std::collections::HashSet<i64> = session.units.iter().map(|u| u.id).collect();
        self.glide.retain(|id, _| live.contains(id));
        for unit in &session.units {
            let Some(position) = unit.position else {
                self.glide.remove(&unit.id);
                continue;
            };
            match self.glide.get_mut(&unit.id) {
                Some(glide) if glide.to == position => {}
                Some(glide) => {
                    let gap = now
                        .saturating_duration_since(glide.at)
                        .as_secs_f64()
                        .clamp(0.2, 3.0);
                    // The old destination becomes the start: the blend stays
                    // between two accepted positions, and the marker is drawn
                    // where it already was rather than snapping to the last one.
                    glide.from = glide.to;
                    glide.to = position;
                    glide.at = now;
                    glide.gap = gap;
                }
                None => {
                    self.glide.insert(
                        unit.id,
                        Glide {
                            from: position,
                            to: position,
                            at: now,
                            gap: 1.0,
                        },
                    );
                }
            }
        }
    }

    /// Where a hull is drawn this frame: the blend through its glide.
    fn placed(&self, unit: &UnitState) -> Option<GeoPoint> {
        let Some(glide) = self.glide.get(&unit.id) else {
            return unit.position;
        };
        let frac = if glide.gap <= 0.0 {
            1.0
        } else {
            (glide.at.elapsed().as_secs_f64() / glide.gap).clamp(0.0, 1.0)
        };
        let lat = glide.from.latitude + (glide.to.latitude - glide.from.latitude) * frac;
        let lon = glide.from.longitude + (glide.to.longitude - glide.from.longitude) * frac;
        Some(GeoPoint {
            latitude: lat,
            longitude: lon,
        })
    }

    fn follow(&mut self, view: &ViewState) {
        // Only the place. The view's `width`/`height` belong to whichever
        // display last reported them; this window's size is its own, and
        // copying theirs here would report somebody else's wall as ours.
        self.camera.center = view.camera.center;
        self.camera.zoom = view.camera.zoom;
        // The picture on screen is now from an older camera; the next render
        // asks for one at this one.
        self.asked = None;
    }

    /// Ask the map thread for a picture at the current camera and window size.
    fn request_map(&mut self, window: &mut Window) {
        let vp = window.viewport_size();
        let scale = f64::from(window.scale_factor());
        let want = (
            (f64::from(vp.width) * scale).round().max(1.0) as u32,
            (f64::from(vp.height) * scale).round().max(1.0) as u32,
        );
        if want != self.map_px {
            self.map_px = want;
            self.camera.width = want.0;
            self.camera.height = want.1;
            self.asked = None;
            // The host resolves a phone's nudge against the size the display
            // reports, so a resized wall must say so rather than let the phone
            // nudge against the size it used to be.
            self.client.viewport(want.0, want.1);
        }
        let key = (
            (self.camera.center.latitude, self.camera.center.longitude),
            self.camera.zoom,
            self.map_px,
        );
        if self.asked == Some(key) {
            return;
        }
        self.asked = Some(key);
        self.seq += 1;
        let _ = self.map_tx.send((
            self.seq,
            (self.camera.center.latitude, self.camera.center.longitude),
            self.camera.zoom,
            self.map_px,
            map_geometry::JUMP_PUMP,
        ));
    }

    fn ready(&self) -> bool {
        self.readiness.ready && self.heard_from_host
    }

}

/// What the waiting card says, and why. A free function because this copy is
/// a contract with the plan's table, not a fact about a particular window: it
/// is tested without a GPUI context, and the UI is a caller like any other.
fn waiting_copy(
    heard_from_host: bool,
    readiness: &Readiness,
    session: Option<&SessionState>,
    server: &str,
) -> (String, String) {
    if !heard_from_host {
        return (
            "CANNOT REACH THE CONTROL SERVER".to_string(),
            format!("{server} \u{2014} retrying."),
        );
    }
    let Some(reason) = readiness.reason else {
        return (
            "WAITING FOR THE HOST".to_string(),
            "The host has not said whether observation is open.".to_string(),
        );
    };
    match reason {
        ReadinessReason::NoSession => (
            "NO EXERCISE OPEN".to_string(),
            "Waiting for the game master to start an exercise.".to_string(),
        ),
        ReadinessReason::NotExecution => {
            let (name, phase) = session
                .map(|session| (session.name.clone(), session.phase.clone()))
                .unwrap_or_else(|| ("the exercise".to_string(), "another phase".to_string()));
            (
                "OBSERVATION NOT OPEN".to_string(),
                format!("{name} is in {phase}. Observation opens at execution."),
            )
        }
        ReadinessReason::NoDisplay => (
            "CONNECTING THIS COMMAND CENTER".to_string(),
            "The host has not counted this display yet.".to_string(),
        ),
        ReadinessReason::Stale => (
            "THE HOST STOPPED REPORTING".to_string(),
            "Reconnecting\u{2026}".to_string(),
        ),
    }
}

/// One hull as this frame draws it.
struct Mark {
    x: f32,
    y: f32,
    symbol: MapSymbolShape,
    affiliation: crate::symbology::Affiliation,
    stale: bool,
    focused: bool,
    /// The turned photograph and the box it occupies, when this zoom is
    /// close enough and the hull has a picture.
    photo: Option<(Arc<gpui::RenderImage>, f32, f32)>,
}

impl CommandCenter {
    /// Every hull on screen, projected at the live camera, with the mark the
    /// console would draw at this zoom.
    fn prepare_marks(&mut self, win_w: f32, win_h: f32) -> Vec<Mark> {
        let Some(session) = self.session.clone() else {
            return Vec::new();
        };
        // Gathered first, turned second: the turned-picture cache needs
        // `&mut self`, and a hull's placement is a read.
        let mut pending = Vec::new();
        for unit in &session.units {
            let Some(position) = self.placed(unit) else {
                continue;
            };
            let (x, y) = map_geometry::project_mercator(
                position.latitude,
                position.longitude,
                (self.camera.center.latitude, self.camera.center.longitude),
                self.camera.zoom,
                f64::from(self.map_dims.0),
                f64::from(self.map_dims.1),
            );
            if !(0.0..=1.0).contains(&(x / f64::from(self.map_dims.0)))
                || !(0.0..=1.0).contains(&(y / f64::from(self.map_dims.1)))
            {
                continue;
            }
            pending.push((
                unit.id,
                win_w * (x as f32 / self.map_dims.0),
                win_h * (y as f32 / self.map_dims.1),
                MapSymbolShape::from_wire(&unit.symbol),
                crate::symbology::Affiliation::from_str(&unit.affiliation),
                unit.stale,
                self.ui.focus == Some(unit.id),
                position.latitude,
                unit.loa_m,
                unit.beam_m,
                unit.heading_deg,
            ));
        }
        // The map picture is drawn to fill the window, so one axis's scale is
        // the other's: the window is the frame's aspect.
        let s = if self.map_dims.0 > 0.0 {
            win_w / self.map_dims.0
        } else {
            1.0
        };
        let mut out = Vec::with_capacity(pending.len());
        for (id, x, y, symbol, affiliation, stale, focused, latitude, loa, beam, heading) in pending {
            let photo = match (
                self.camera.zoom >= PHOTO_ZOOM,
                hull_extent(latitude, loa, beam, heading, self.camera.zoom, s),
            ) {
                (true, Some((length_px, beam_px, degree))) => {
                    self.turned_for(id, length_px, beam_px, degree)
                }
                _ => None,
            };
            out.push(Mark {
                x,
                y,
                symbol,
                affiliation,
                stale,
                focused,
                photo,
            });
        }
        out
    }

    /// One hull's photograph, turned into the box it is seen in.
    fn turned_for(
        &mut self,
        unit_id: i64,
        length_px: f32,
        beam_px: f32,
        degree: u16,
    ) -> Option<(Arc<gpui::RenderImage>, f32, f32)> {
        let picture = self.pictures.get(&unit_id)?;
        let (w, h) = (picture.width, picture.height);
        let theta = degree as f32 - 90.0;
        let (tw, th) = crate::thumb_rotate::turned_size(length_px, beam_px, theta);
        // Capped, as the console caps it: a hull a kilometre long does not
        // cost a megabyte of pixels.
        let fit = (512.0 / tw.max(th)).min(1.0);
        let (rw, rh) = (
            (tw * fit).round().max(1.0) as u32,
            (th * fit).round().max(1.0) as u32,
        );
        let key = (unit_id, rw, rh, degree);
        if let Some(image) = self.turned.get(&key) {
            return Some((image.clone(), tw, th));
        }
        let kx = w as f32 / length_px.max(1.0);
        let ky = h as f32 / beam_px.max(1.0);
        let pixels =
            crate::thumb_rotate::turn_to_size(&picture.bgra, w, h, theta, rw, rh, kx, ky)?;
        let buffer = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(rw, rh, pixels)?;
        let image = Arc::new(gpui::RenderImage::new([image::Frame::new(buffer)]));
        self.turned_bytes += (rw as usize) * (rh as usize) * 4;
        if self.turned_bytes > 4 * 1024 * 1024 {
            // A cache, not a ledger: emptied when it grows past its budget,
            // exactly as the console's is.
            self.turned.clear();
            self.turned_bytes = 0;
        }
        self.turned.insert(key, image.clone());
        Some((image, tw, th))
    }

    /// The hull the phone focused, and where its mark is this frame.
    fn focused_unit(&self, win_w: f32, win_h: f32) -> Option<((f32, f32), UnitState)> {
        let id = self.ui.focus?;
        let unit = self
            .session
            .as_ref()?
            .units
            .iter()
            .find(|unit| unit.id == id)?
            .clone();
        let position = self.placed(&unit)?;
        let (x, y) = map_geometry::project_mercator(
            position.latitude,
            position.longitude,
            (self.camera.center.latitude, self.camera.center.longitude),
            self.camera.zoom,
            f64::from(self.map_dims.0),
            f64::from(self.map_dims.1),
        );
        Some((
            (
                win_w * (x as f32 / self.map_dims.0),
                win_h * (y as f32 / self.map_dims.1),
            ),
            unit,
        ))
    }

    /// The card beside the focused hull.
    fn unit_card(
        &self,
        unit: &UnitState,
        at: (f32, f32),
        win_w: f32,
        win_h: f32,
    ) -> gpui::AnyElement {
        // THE CARD IS SIZED TO ITS TYPE, as the console's column is: a bigger
        // step needs a wider card, or the rows wrap and the value column
        // breaks. The offsets around it follow the same number.
        let (w, h) = (ts(300.0), ts(208.0));
        // Beside the mark, flipped when that side has no room, and clamped so
        // a small window can never push the card past its edge.
        let left = (if at.0 + 24.0 + w > win_w {
            at.0 - 24.0 - w
        } else {
            at.0 + 24.0
        })
        .clamp(8.0, (win_w - w - 8.0).max(8.0));
        let top = (at.1 - h / 2.0).clamp(72.0, (win_h - h - 8.0).max(72.0));
        let step = self
            .session
            .as_ref()
            .and_then(|session| session.scenario.as_ref())
            .map(|scenario| match &scenario.step {
                Some(step) if !step.trim().is_empty() => format!("{} · {step}", scenario.title),
                _ => scenario.title.clone(),
            })
            .unwrap_or_else(|| "—".to_string());
        let commander = unit
            .commander
            .as_ref()
            .map(|commander| commander.name.clone())
            .unwrap_or_else(|| "uncommanded".to_string());
        let speed = unit
            .speed_kn
            .map(|speed| format!("{speed:.0} kn"))
            .unwrap_or_else(|| "—".to_string());
        let course = unit
            .heading_deg
            .map(|heading| format!("{heading:.0}°"))
            .unwrap_or_else(|| "—".to_string());
        let embarked = if unit.embarked.is_empty() {
            "—".to_string()
        } else {
            unit.embarked
                .iter()
                .map(|child| child.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let row = |label: &'static str, value: String| {
            div()
                .flex()
                .flex_row()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .font_family("PT Mono")
                        .text_size(px(ts(10.0)))
                        .text_color(crate::ink::rgba(view::BODY_SILVER))
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .text_right()
                        .font_family("Saira Condensed")
                        .text_size(px(ts(13.0)))
                        .text_color(crate::ink::rgba(view::MAP_INK))
                        .child(value),
                )
        };
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(w))
            .border_1()
            .border_color(crate::ink::rgba(view::RADAR_CYAN.with_alpha(160)))
            .bg(crate::ink::rgba(view::PANEL_SLATE))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(12.0))
                    .py(px(6.0))
                    .border_b_1()
                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                    .flex()
                    .flex_row()
                    .justify_between()
                    .child(
                        div()
                            .font_family("Saira Condensed")
                            .text_size(px(ts(11.0)))
                            .text_color(crate::ink::rgba(view::RADAR_CYAN))
                            .child(format!("UNIT {}", unit.index)),
                    )
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(10.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(unit.hull_number.clone()),
                    ),
            )
            .child(
                div()
                    .p(px(12.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .font_family("Saira Condensed")
                            .text_size(px(ts(17.0)))
                            .text_color(crate::ink::rgba(view::MAP_INK))
                            .child(unit.name.clone()),
                    )
                    .child(
                        div()
                            .font_family("Saira Condensed")
                            .text_size(px(ts(11.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(unit.class_name.clone()),
                    )
                    .child(row("COMMANDER", commander))
                    .child(row("SPEED", speed))
                    .child(row("COURSE", course))
                    .child(row("EMBARKED", embarked))
                    .child(row("STEP", step)),
            )
            .into_any_element()
    }

    /// The two readings the Game Master's execution rail carries: the players
    /// in the room and the traffic the exercise holds.
    fn execution_rail(&self, _win_w: f32) -> gpui::AnyElement {
        let Some(session) = &self.session else {
            return div().into_any_element();
        };
        let players = self.players_count();
        let hulls = session.units.len();
        let unseen = 0usize;
        let mut rail = div()
            .absolute()
            .top(px(ts(78.0)))
            .right(px(ts(10.0)))
            .w(px(ts(196.0)))
            .flex()
            .flex_col()
            .gap(px(ts(6.0)));
        // THE SCENARIO LEADS, as it does on the Game Master's rail. The wall
        // does not carry the ready verb — it observes — but the reading is the
        // same one.
        if session.mode == "scenario" {
            let board = session.board.as_ref();
            let value = match board {
                Some(board) if board.total == 0 => "NO SCRIPT".to_string(),
                Some(board) => match board.at {
                    Some(at) => format!("STEP {at}"),
                    None => "NOT STARTED".to_string(),
                },
                None => session
                    .scenario
                    .as_ref()
                    .map(|scenario| scenario.title.clone())
                    .unwrap_or_else(|| "NO SCRIPT".to_string()),
            };
            let status = board
                .and_then(|board| {
                    board
                        .steps
                        .iter()
                        .find(|step| step.read == "playing")
                        .or_else(|| board.steps.iter().find(|step| step.read == "asking"))
                })
                .map(|step| step.status.to_uppercase());
            let hint = board
                .map(|board| {
                    let ready = board
                        .units
                        .iter()
                        .filter(|unit| unit.ready == Some(true))
                        .count();
                    let asked = board
                        .units
                        .iter()
                        .filter(|unit| unit.ready.is_some())
                        .count();
                    if asked > 0 {
                        format!("{ready} of {asked} ready")
                    } else {
                        plural(board.total, "step")
                    }
                })
                .unwrap_or_else(|| {
                    session
                        .scenario
                        .as_ref()
                        .and_then(|scenario| scenario.step.clone())
                        .unwrap_or_else(|| "nothing is playing".to_string())
                });
            rail = rail.child(self.exec_card("SCENARIO", value, hint, status));
        }
        rail = rail.child(self.exec_card(
            "PLAYERS",
            format!("{players}"),
            format!("{} \u{00b7} {}", plural(players, "player"), plural(hulls, "hull")),
            None,
        ));
        rail = rail.child(self.exec_card(
            "COMMS",
            format!("{}", session.messages.len()),
            if unseen > 0 {
                format!(
                    "{} unseen \u{00b7} {} in this exercise",
                    plural(unseen, "message"),
                    plural(session.messages.len(), "message")
                )
            } else {
                format!(
                    "{} in this exercise",
                    plural(session.messages.len(), "message")
                )
            },
            None,
        ));
        rail.into_any_element()
    }

    fn exec_card(
        &self,
        label: &'static str,
        value: String,
        hint: String,
        status: Option<String>,
    ) -> gpui::AnyElement {
        div()
            .w_full()
            .rounded(px(ts(2.0)))
            .bg(crate::ink::rgba(view::DEEP_WELL))
            .border_1()
            .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .h(px(ts(26.0)))
                    .px(px(ts(10.0)))
                    .bg(crate::ink::rgba(view::PANEL_SLATE))
                    .border_b_1()
                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(10.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(label),
                    )
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(view::RADAR_CYAN))
                            .child(status.unwrap_or_default()),
                    ),
            )
            .child(
                div()
                    .px(px(ts(10.0)))
                    .pt(px(ts(4.0)))
                    .font_family("Saira Condensed")
                    .text_size(px(ts(18.0)))
                    .text_color(crate::ink::rgba(view::MAP_INK))
                    .child(value),
            )
            .child(
                div()
                    .px(px(ts(10.0)))
                    .pb(px(ts(6.0)))
                    .font_family("PT Mono")
                    .text_size(px(ts(9.0)))
                    .text_color(crate::ink::rgba(view::BODY_SILVER))
                    .child(hint),
            )
            .into_any_element()
    }

    /// The scenario's own reading, in the body of the map.
    fn scenario_card(&self, _win_h: f32) -> gpui::AnyElement {
        let Some(session) = &self.session else {
            return div().into_any_element();
        };
        let board = session.board.as_ref();
        let value = match board {
            Some(board) if board.total == 0 => "NO SCRIPT".to_string(),
            Some(board) => match board.at {
                Some(at) => format!("STEP {at}"),
                None => "NOT STARTED".to_string(),
            },
            None => session
                .scenario
                .as_ref()
                .map(|scenario| scenario.title.clone())
                .unwrap_or_else(|| "NO SCRIPT".to_string()),
        };
        let (ready, asked) = board
            .map(|board| {
                (
                    board.units.iter().filter(|unit| unit.ready == Some(true)).count(),
                    board.units.iter().filter(|unit| unit.ready.is_some()).count(),
                )
            })
            .unwrap_or((0, 0));
        let hint = if asked > 0 {
            format!("{ready} of {asked} ready")
        } else {
            session
                .scenario
                .as_ref()
                .and_then(|scenario| scenario.step.clone())
                .unwrap_or_else(|| "nothing is playing".to_string())
        };
        div()
            .absolute()
            .left(px(ts(12.0)))
            .bottom(px(ts(12.0)))
            .w(px(ts(292.0)))
            .flex()
            .flex_col()
            .rounded(px(ts(2.0)))
            .bg(crate::ink::rgba(view::CONSOLE_NIGHT))
            .border_1()
            .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .w_full()
                    .h(px(ts(26.0)))
                    .px(px(ts(10.0)))
                    .bg(crate::ink::rgba(view::PANEL_SLATE))
                    .border_b_1()
                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(10.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child("SCENARIO"),
                    )
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(view::RADAR_CYAN))
                            .child(
                                board
                                    .and_then(|board| {
                                        board
                                            .steps
                                            .iter()
                                            .find(|step| step.read == "playing")
                                            .or_else(|| {
                                                board.steps.iter().find(|step| step.read == "asking")
                                            })
                                    })
                                    .map(|step| step.status.to_uppercase())
                                    .unwrap_or_else(|| "IDLE".to_string()),
                            ),
                    ),
            )
            .child(
                div()
                    .px(px(ts(10.0)))
                    .pt(px(ts(5.0)))
                    .font_family("Saira Condensed")
                    .text_size(px(ts(16.0)))
                    .text_color(crate::ink::rgba(view::MAP_INK))
                    .child(value),
            )
            .child(
                div()
                    .px(px(ts(10.0)))
                    .pb(px(ts(6.0)))
                    .font_family("PT Mono")
                    .text_size(px(ts(9.0)))
                    .text_color(crate::ink::rgba(view::BODY_SILVER))
                    .child(hint),
            )
            .into_any_element()
    }

    /// The larger surfaces the phone opens with `ui.set`.
    fn panel_overlays(&self, mut root: gpui::Div, win_w: f32, win_h: f32) -> gpui::Div {
        let open = self.ui.panels;
        if open.comms {
            root = root.child(self.comms_island(win_h));
        }
        if open.crew {
            root = root.child(self.crew_modal(win_w, win_h));
        }
        if open.scenario {
            root = root.child(self.scenario_window(win_w, win_h));
        }
        root
    }

    /// The Crew window, not the widget: a large centred card, as the console's
    /// crew surface is.
    fn crew_modal(&self, win_w: f32, win_h: f32) -> gpui::AnyElement {
        let Some(session) = &self.session else {
            return div().into_any_element();
        };
        let width = ts(1120.0).min((win_w - 32.0).max(420.0));
        let height = ts(620.0).min((win_h - 32.0).max(360.0));
        let ready = session.participants.iter().filter(|p| p.ready).count();
        let mut body = div().flex().flex_col().gap(px(ts(8.0))).p(px(ts(14.0)));
        if session.participants.is_empty() {
            body = body.child(self.board_note("No seat is published for this exercise."));
        }
        for person in &session.participants {
            let hulls: Vec<String> = session
                .units
                .iter()
                .filter(|unit| unit.commander.as_ref().is_some_and(|c| c.id == person.id))
                .map(|unit| format!("{} {}", unit.index, unit.name))
                .collect();
            let mut row = div()
                .w_full()
                .flex()
                .flex_col()
                .gap(px(ts(2.0)))
                .p(px(ts(8.0)))
                .border_1()
                .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                .bg(crate::ink::rgba(view::DEEP_WELL))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_family("Saira Condensed")
                                .text_size(px(ts(16.0)))
                                .text_color(crate::ink::rgba(view::MAP_INK))
                                .child(if person.name.trim().is_empty() {
                                    person.role.clone()
                                } else {
                                    person.name.clone()
                                }),
                        )
                        .child(self.ready_chip(if person.judge {
                            None
                        } else {
                            Some(person.ready)
                        })),
                )
                .child(
                    div()
                        .font_family("PT Mono")
                        .text_size(px(ts(10.0)))
                        .text_color(crate::ink::rgba(view::BODY_SILVER))
                        .child(person.role.clone()),
                );
            for hull in hulls {
                row = row.child(
                    div()
                        .font_family("PT Mono")
                        .text_size(px(ts(10.0)))
                        .text_color(crate::ink::rgba(view::MAP_INK))
                        .child(hull),
                );
            }
            body = body.child(row);
        }
        div()
            .absolute()
            .size_full()
            .bg(crate::ink::rgba(view::CONSOLE_NIGHT.with_alpha(150)))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(width))
                    .h(px(height))
                    .bg(crate::ink::rgba(view::CONSOLE_NIGHT))
                    .border_1()
                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .h(px(ts(38.0)))
                            .px(px(ts(14.0)))
                            .border_b_1()
                            .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                            .child(
                                div()
                                    .font_family("Saira Condensed")
                                    .text_size(px(ts(18.0)))
                                    .text_color(crate::ink::rgba(view::MAP_INK))
                                    .child("CREW"),
                            )
                            .child(
                                div()
                                    .font_family("PT Mono")
                                    .text_size(px(ts(10.0)))
                                    .text_color(crate::ink::rgba(view::BODY_SILVER))
                                    .child(format!(
                                        "{} \u{00b7} {} READY",
                                        plural(session.participants.len(), "seat"),
                                        ready
                                    )),
                            ),
                    )
                    .child(div().id("wall-crew-scroll").flex_1().min_h_0().overflow_y_scroll().child(body)),
            )
            .into_any_element()
    }

    /// The Comms pane, not the widget: the exercise's traffic, beside the map.
    fn comms_island(&self, win_h: f32) -> gpui::AnyElement {
        let Some(session) = &self.session else {
            return div().into_any_element();
        };
        let mut body = div().flex().flex_col().gap(px(ts(8.0))).p(px(ts(12.0)));
        if session.messages.is_empty() {
            body = body.child(self.board_note("No messages in this exercise."));
        }
        for message in &session.messages {
            let time = message
                .at
                .split('T')
                .nth(1)
                .map(|rest| rest.chars().take(5).collect::<String>())
                .unwrap_or_default();
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(ts(2.0)))
                    .pb(px(ts(6.0)))
                    .border_b_1()
                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap(px(ts(8.0)))
                            .child(
                                div()
                                    .font_family("PT Mono")
                                    .text_size(px(ts(9.5)))
                                    .text_color(crate::ink::rgba(view::RADAR_CYAN))
                                    .child(message.sender.clone()),
                            )
                            .child(
                                div()
                                    .font_family("PT Mono")
                                    .text_size(px(ts(9.0)))
                                    .text_color(crate::ink::rgba(view::BODY_SILVER))
                                    .child(format!("{} {time}", message.degree)),
                            ),
                    )
                    .child(
                        div()
                            .font_family("Saira Condensed")
                            .text_size(px(ts(13.0)))
                            .text_color(crate::ink::rgba(view::MAP_INK))
                            .child(message.content.clone()),
                    ),
            );
        }
        div()
            .absolute()
            .top(px(ts(292.0)))
            .right(px(ts(10.0)))
            .w(px(ts(410.0)))
            .max_h(px((win_h - ts(220.0)).max(ts(180.0))))
            .bg(crate::ink::rgba(view::PANEL_SLATE))
            .border_1()
            .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .h(px(ts(32.0)))
                    .px(px(ts(12.0)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .bg(crate::ink::rgba(view::CONSOLE_NIGHT))
                    .border_b_1()
                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(11.0)))
                            .text_color(crate::ink::rgba(view::RADAR_CYAN))
                            .child("COMMS"),
                    )
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(plural(session.messages.len(), "message")),
                    ),
            )
            .child(div().id("wall-comms-scroll").flex_1().min_h_0().overflow_y_scroll().child(body))
            .into_any_element()
    }

    /// The Scenario window, not the widget: the whole frame under the strip,
    /// with the book on the left and the command channels on the right.
    fn scenario_window(&self, win_w: f32, win_h: f32) -> gpui::AnyElement {
        let Some(session) = &self.session else {
            return div().into_any_element();
        };
        let width = win_w;
        let mut col = div()
            .absolute()
            .top(px(ts(64.0)))
            .left(px(0.0))
            .right(px(0.0))
            .bottom(px(0.0))
            .bg(crate::ink::rgba(view::CONSOLE_NIGHT))
            .flex()
            .flex_col();
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(ts(12.0)))
                .h(px(ts(38.0)))
                .px(px(ts(14.0)))
                .border_b_1()
                .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                .child(
                    div()
                        .font_family("Saira Condensed")
                        .text_size(px(ts(18.0)))
                        .text_color(crate::ink::rgba(view::MAP_INK))
                        .child("SCENARIO"),
                )
                .child(
                    div()
                        .font_family("PT Mono")
                        .text_size(px(ts(10.0)))
                        .text_color(crate::ink::rgba(view::BODY_SILVER))
                        .child("OBSERVER"),
                )
                .child(
                    div()
                        .font_family("Saira Condensed")
                        .text_size(px(ts(14.0)))
                        .text_color(crate::ink::rgba(view::MAP_INK))
                        .child(session.name.clone()),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .font_family("PT Mono")
                        .text_size(px(ts(10.0)))
                        .text_color(crate::ink::rgba(view::BODY_SILVER))
                        .child(
                            session
                                .board
                                .as_ref()
                                .map(|board| {
                                    let ready = board
                                        .units
                                        .iter()
                                        .filter(|unit| unit.ready == Some(true))
                                        .count();
                                    let asked = board
                                        .units
                                        .iter()
                                        .filter(|unit| unit.ready.is_some())
                                        .count();
                                    let where_ = match board.at {
                                        Some(at) => format!("step {at} of {}", board.total),
                                        None => format!("{} \u{00b7} not started", plural(board.total, "step")),
                                    };
                                    if asked > 0 {
                                        format!("{where_} \u{00b7} {ready}/{asked} ready")
                                    } else {
                                        where_
                                    }
                                })
                                .unwrap_or_else(|| "no script".to_string()),
                        ),
                ),
        );
        let Some(board) = session.board.as_ref() else {
            return col
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(self.board_note("The console has not published the book for this exercise.")),
                )
                .into_any_element();
        };
        let mut left = div()
            .w(px(ts(430.0)))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap(px(ts(6.0)))
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(self.caps("the book \u{2014} the steps, in order"))
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(plural(board.total, "step")),
                    ),
            );
        let mut list = div()
            .id("wall-board")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(ts(6.0)));
        if board.steps.is_empty() {
            list = list.child(self.board_note("No steps to read."));
        }
        for step in &board.steps {
            list = list.child(self.board_step_row(step));
        }
        left = left.child(list);
        let mut right = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(ts(6.0)))
            .overflow_hidden()
            .child(self.caps("the commands \u{2014} each channel and the hulls it addresses"));
        let mut channels = div()
            .id("wall-channels")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(ts(8.0)));
        let pointer = board
            .at
            .and_then(|at| board.steps.get(at.saturating_sub(1)))
            .or_else(|| board.steps.iter().find(|step| step.read == "playing"))
            .or_else(|| board.steps.iter().find(|step| step.read == "asking"));
        if let Some(step) = pointer {
            let mut tasked: Vec<i64> = Vec::new();
            if step.substeps.is_empty() {
                channels = channels.child(self.board_note("This step has no commands."));
            }
            for (at, sub) in step.substeps.iter().enumerate() {
                let mut channel = div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(ts(5.0)))
                    .p(px(ts(8.0)))
                    .bg(crate::ink::rgba(view::DEEP_WELL))
                    .border_1()
                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(ts(8.0)))
                            .child(
                                div()
                                    .font_family("PT Mono")
                                    .text_size(px(ts(9.0)))
                                    .text_color(crate::ink::rgba(view::BODY_SILVER))
                                    .child(format!("{:02}", at + 1)),
                            )
                            .child(
                                div()
                                    .font_family("Saira Condensed")
                                    .text_size(px(ts(13.0)))
                                    .text_color(crate::ink::rgba(view::MAP_INK))
                                    .child(sub.content.clone()),
                            ),
                    );
                let mut any = false;
                for id in &sub.units {
                    if let Some(unit) = board.units.iter().find(|unit| unit.id == Some(*id)) {
                        tasked.push(*id);
                        any = true;
                        channel = channel.child(self.board_hull_card(unit));
                    }
                }
                if !any {
                    channel = channel.child(self.board_note("No hull is assigned to this command."));
                }
                channels = channels.child(channel);
            }
            let idle: Vec<_> = board
                .units
                .iter()
                .filter(|unit| unit.id.is_none_or(|id| !tasked.contains(&id)))
                .collect();
            if !idle.is_empty() {
                let mut channel = div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(ts(5.0)))
                    .p(px(ts(8.0)))
                    .bg(crate::ink::rgba(view::DEEP_WELL))
                    .border_1()
                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE.with_alpha(150)))
                    .child(self.caps("NOT TASKED \u{2014} no command names these"));
                for unit in idle {
                    channel = channel.child(self.board_hull_card(unit));
                }
                channels = channels.child(channel);
            }
        } else {
            channels = channels.child(self.board_note("Select a step to read its commands."));
        }
        right = right.child(channels);
        col = col.child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_row()
                .gap(px(ts(12.0)))
                .p(px(ts(12.0)))
                .child(left)
                .child(right),
        );
        let _ = (width, win_h);
        col.into_any_element()
    }

    fn caps(&self, text: &str) -> gpui::AnyElement {
        div()
            .font_family("PT Mono")
            .text_size(px(ts(9.0)))
            .text_color(crate::ink::rgba(view::BODY_SILVER))
            .child(text.to_uppercase())
            .into_any_element()
    }

    fn board_note(&self, text: &str) -> gpui::AnyElement {
        div()
            .font_family("Saira Condensed")
            .text_size(px(ts(12.0)))
            .text_color(crate::ink::rgba(view::BODY_SILVER))
            .child(text.to_string())
            .into_any_element()
    }

    fn board_step_row(&self, step: &crate::control::BoardStepState) -> gpui::AnyElement {
        let ink = match step.status.as_str() {
            "playing" | "current" | "unit_ready" => view::RADAR_CYAN,
            "halted" | "done" => view::WARNING_SAND,
            "played" => view::SIGNAL_GREEN,
            _ => view::BODY_SILVER,
        };
        let active = matches!(step.status.as_str(), "playing" | "current" | "unit_ready");
        let mut card = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(ts(4.0)))
            .p(px(ts(8.0)))
            .bg(crate::ink::rgba(if active {
                view::PANEL_SLATE
            } else {
                view::DEEP_WELL
            }))
            .border_1()
            .border_color(crate::ink::rgba(if active {
                view::RADAR_CYAN
            } else {
                view::HAIRLINE_SLATE
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(ts(8.0)))
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(format!("{:>2}", step.number)),
                    )
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(ink))
                            .child(step.status.to_uppercase()),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(
                                step.window
                                    .as_ref()
                                    .map(|(a, b)| format!("{a}\u{2013}{b}"))
                                    .unwrap_or_else(|| "\u{2014}".to_string()),
                            ),
                    ),
            )
            .child(
                div()
                    .font_family("Saira Condensed")
                    .text_size(px(ts(13.0)))
                    .text_color(crate::ink::rgba(view::MAP_INK))
                    .child(if step.content.trim().is_empty() {
                        "(not written yet)".to_string()
                    } else {
                        step.content.clone()
                    }),
            );
        for (at, sub) in step.substeps.iter().enumerate() {
            card = card.child(
                div()
                    .pl(px(ts(18.0)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(ts(6.0)))
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(8.5)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(format!("{:02}", at + 1)),
                    )
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(view::MAP_INK))
                            .child(sub.content.clone()),
                    ),
            );
        }
        card.into_any_element()
    }

    fn board_hull_card(&self, unit: &crate::control::BoardUnitState) -> gpui::AnyElement {
        div()
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(ts(8.0)))
            .p(px(ts(6.0)))
            .bg(crate::ink::rgba(view::PANEL_SLATE))
            .border_1()
            .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(ts(1.0)))
                    .child(
                        div()
                            .font_family("Saira Condensed")
                            .text_size(px(ts(13.0)))
                            .text_color(crate::ink::rgba(view::MAP_INK))
                            .child(unit.name.clone()),
                    )
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(unit.commander.clone()),
                    ),
            )
            .child(self.ready_chip(unit.ready))
            .into_any_element()
    }

    fn ready_chip(&self, ready: Option<bool>) -> gpui::AnyElement {
        let (label, ink) = match ready {
            Some(true) => ("READY", view::SIGNAL_GREEN),
            Some(false) => ("NOT READY", view::WARNING_SAND),
            None => ("\u{2014}", view::BODY_SILVER),
        };
        div()
            .flex_shrink_0()
            .font_family("PT Mono")
            .text_size(px(ts(9.0)))
            .text_color(crate::ink::rgba(ink))
            .child(label)
            .into_any_element()
    }

    fn players_count(&self) -> usize {
        let Some(session) = &self.session else {
            return 0;
        };
        let seated = session
            .participants
            .iter()
            .filter(|person| !person.judge)
            .count();
        if seated > 0 {
            return seated;
        }
        let mut commanders: Vec<i64> = session
            .units
            .iter()
            .filter_map(|unit| unit.commander.as_ref().map(|c| c.id))
            .collect();
        commanders.sort_unstable();
        commanders.dedup();
        commanders.len()
    }

    /// The phone's group focus, resolved to the group's mean ground.
    fn focused_group(&self, win_w: f32, win_h: f32) -> Option<((f32, f32), crate::control::GroupState)> {
        let id = self.ui.focus_group?;
        let session = self.session.as_ref()?;
        let group = session.groups.iter().find(|group| group.id == id)?.clone();
        let anchor = self.group_anchor(session, id)?;
        let (x, y) = map_geometry::project_mercator(
            anchor.latitude,
            anchor.longitude,
            (self.camera.center.latitude, self.camera.center.longitude),
            self.camera.zoom,
            f64::from(self.map_dims.0),
            f64::from(self.map_dims.1),
        );
        Some((
            (
                win_w * (x as f32 / self.map_dims.0),
                win_h * (y as f32 / self.map_dims.1),
            ),
            group,
        ))
    }

    fn group_anchor(&self, session: &SessionState, group_id: i64) -> Option<GeoPoint> {
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
                    .and_then(|unit| self.placed(unit))
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

    fn group_card(
        &self,
        group: &crate::control::GroupState,
        at: (f32, f32),
        win_w: f32,
        win_h: f32,
    ) -> gpui::AnyElement {
        let Some(session) = &self.session else {
            return div().into_any_element();
        };
        let mut subtree = vec![group.id];
        let mut cursor = 0usize;
        while cursor < subtree.len() {
            let parent = subtree[cursor];
            for child in &session.groups {
                if child.parent == Some(parent) && !subtree.contains(&child.id) {
                    subtree.push(child.id);
                }
            }
            cursor += 1;
        }
        let children: Vec<&crate::control::GroupState> = session
            .groups
            .iter()
            .filter(|g| g.parent == Some(group.id))
            .collect();
        let hulls: Vec<String> = session
            .groups
            .iter()
            .filter(|g| subtree.contains(&g.id))
            .flat_map(|g| g.units.iter())
            .filter_map(|id| session.units.iter().find(|unit| unit.id == *id))
            .map(|unit| unit.name.clone())
            .collect();
        let width = ts(316.0);
        let left = (if at.0 + 24.0 + width > win_w {
            at.0 - 24.0 - width
        } else {
            at.0 + 24.0
        })
        .clamp(8.0, (win_w - width - 8.0).max(8.0));
        let top = (at.1 - 120.0).clamp(ts(70.0), (win_h - 360.0).max(ts(70.0)));
        let mut card = div()
            .id("wall-group-card")
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .max_h(px(ts(420.0)))
            .overflow_y_scroll()
            .bg(crate::ink::rgba(view::PANEL_SLATE))
            .border_1()
            .border_color(crate::ink::rgba(group_ink(&group.kind)))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(ts(12.0)))
                    .py(px(ts(6.0)))
                    .border_b_1()
                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(10.0)))
                            .text_color(crate::ink::rgba(group_ink(&group.kind)))
                            .child(group.kind.to_uppercase()),
                    )
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(format!(
                                "{} \u{00b7} {}",
                                plural(hulls.len(), "hull"),
                                plural(children.len(), "group")
                            )),
                    ),
            )
            .child(
                div()
                    .px(px(ts(12.0)))
                    .py(px(ts(8.0)))
                    .flex()
                    .flex_col()
                    .gap(px(ts(4.0)))
                    .child(
                        div()
                            .font_family("Saira Condensed")
                            .text_size(px(ts(17.0)))
                            .text_color(crate::ink::rgba(view::MAP_INK))
                            .child(group.name.clone()),
                    )
                    .child(
                        div()
                            .font_family("Saira Condensed")
                            .text_size(px(ts(13.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(format!(
                                "commander \u{00b7} {}",
                                group
                                    .commander
                                    .as_ref()
                                    .map(|c| c.name.clone())
                                    .unwrap_or_else(|| "uncommanded".to_string())
                            )),
                    ),
            );
        for child in children {
            card = card.child(
                div()
                    .px(px(ts(12.0)))
                    .py(px(ts(3.0)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(ts(6.0)))
                    .border_t_1()
                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(8.5)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child("GROUP"),
                    )
                    .child(
                        div()
                            .font_family("Saira Condensed")
                            .text_size(px(ts(13.0)))
                            .text_color(crate::ink::rgba(view::MAP_INK))
                            .child(child.name.clone()),
                    ),
            );
        }
        for hull in hulls {
            card = card.child(
                div()
                    .px(px(ts(12.0)))
                    .py(px(ts(3.0)))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(ts(6.0)))
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(8.5)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child("HULL"),
                    )
                    .child(
                        div()
                            .font_family("Saira Condensed")
                            .text_size(px(ts(13.0)))
                            .text_color(crate::ink::rgba(view::MAP_INK))
                            .child(hull),
                    ),
            );
        }
        card.into_any_element()
    }

    /// The focused hull's carried pieces, beside its card.
    fn embark_card(
        &self,
        unit: &UnitState,
        at: (f32, f32),
        win_w: f32,
        win_h: f32,
    ) -> gpui::AnyElement {
        if unit.embarked.is_empty() {
            return div().into_any_element();
        }
        let width = ts(300.0);
        let left = (if at.0 + 24.0 + ts(300.0) + 12.0 + width > win_w {
            at.0 - 24.0 - ts(300.0) - 12.0 - width
        } else {
            at.0 + 24.0 + ts(300.0) + 12.0
        })
        .clamp(8.0, (win_w - width - 8.0).max(8.0));
        let top = (at.1 - 70.0).clamp(ts(70.0), (win_h - 220.0).max(ts(70.0)));
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .w(px(width))
            .bg(crate::ink::rgba(view::PANEL_SLATE))
            .border_1()
            .border_color(crate::ink::rgba(view::RADAR_CYAN.with_alpha(160)))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(ts(12.0)))
                    .py(px(ts(6.0)))
                    .border_b_1()
                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .font_family("Saira Condensed")
                            .text_size(px(ts(13.0)))
                            .text_color(crate::ink::rgba(view::MAP_INK))
                            .child(unit.name.clone()),
                    )
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child(plural(unit.embarked.len(), "aboard")),
                    ),
            )
            .child(
                div()
                    .px(px(ts(12.0)))
                    .py(px(ts(8.0)))
                    .flex()
                    .flex_col()
                    .gap(px(ts(4.0)))
                    .child(
                        div()
                            .font_family("PT Mono")
                            .text_size(px(ts(9.0)))
                            .text_color(crate::ink::rgba(view::BODY_SILVER))
                            .child("ABOARD"),
                    )
                    .children(unit.embarked.iter().map(|child| {
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap(px(ts(8.0)))
                            .child(
                                div()
                                    .font_family("Saira Condensed")
                                    .text_size(px(ts(13.0)))
                                    .text_color(crate::ink::rgba(view::MAP_INK))
                                    .child(child.name.clone()),
                            )
                            .child(
                                div()
                                    .font_family("PT Mono")
                                    .text_size(px(ts(9.0)))
                                    .text_color(crate::ink::rgba(view::BODY_SILVER))
                                    .child(child.hull_number.clone()),
                            )
                            .into_any_element()
                    })),
            )
            .into_any_element()
    }
}

/// The ink a task group wears, by echelon: the same three colours the
/// console's planning map uses for its organisation lines.
fn group_ink(kind: &str) -> view::Rgba {
    match kind.to_ascii_lowercase().replace([' ', '_'], "").as_str() {
        "gugus" => view::LINK_GUGUS,
        "operasigabungan" | "opgab" => view::LINK_OPERASI_GABUNGAN,
        _ => view::LINK_SATUAN_TUGAS,
    }
}

/// The wall's own design units, at the host's text size.
///
/// THE WALL DRAWS THROUGH THE LADDER. Its literals were tuned at the ladder's
/// DEFAULT step, so that step is the identity: a wall today looks exactly as
/// it did, and a bigger step grows the type — and the boxes sized to it —
/// without a second redesign. Without this the wall rendered at a fixed size
/// no matter what `settings.font` said, which is the bug that made the phone's
/// command look ignored.
fn ts(points: f32) -> f32 {
    scaled(points, view::text_scale())
}

/// [`ts`] with the scale passed in, so the arithmetic can be checked without
/// moving the process-wide setting.
fn scaled(points: f32, scale: f32) -> f32 {
    points * scale / view::text_scale_at(view::TEXT_SCALE_DEFAULT)
}

/// The length, beam and whole-degree course a hull's photograph is drawn at,
/// when it has all three.
fn hull_extent(
    latitude: f64,
    loa_m: Option<f64>,
    beam_m: Option<f64>,
    heading_deg: Option<f64>,
    zoom: f64,
    s: f32,
) -> Option<(f32, f32, u16)> {
    let (loa, beam, heading) = (loa_m?, beam_m?, heading_deg?);
    if loa <= 0.0 || beam <= 0.0 {
        return None;
    }
    let mpp = map_geometry::meters_per_pixel(latitude, zoom);
    if !mpp.is_finite() || mpp <= 0.0 {
        return None;
    }
    Some((
        (loa / mpp) as f32 * s,
        (beam / mpp) as f32 * s,
        heading.round().rem_euclid(360.0) as u16,
    ))
}

impl Render for CommandCenter {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((no, w, h, bgra, _at, _zoom)) = self.incoming.take() {
            self.frame.show(window, cx, no, (w, h), bgra);
            self.map_dims = (w as f32, h as f32);
        }
        self.request_map(window);

        let window_size = window.viewport_size();
        let (win_w, win_h) = (
            f32::from(window_size.width),
            f32::from(window_size.height),
        );
        let mut root = div()
            .relative()
            .size_full()
            .bg(crate::ink::rgba(view::CONSOLE_NIGHT));

        if let Some(frame) = self.frame.image() {
            root = root.child(
                div()
                    .absolute()
                    .size_full()
                    .child(img(frame).size_full().object_fit(gpui::ObjectFit::Fill)),
            );
        }

        // THE MARKS ARE THE CONSOLE'S MARKS. Below `PHOTO_ZOOM` a hull is its
        // affiliation frame with the taxonomy glyph inside it — the same
        // painting the console does, from the same function — and at or above
        // it the hull's own photograph, sized by its published length and beam
        // and turned on its course. A numbered dot was the wall's invention and
        // it is gone: the grid number belongs on the phone, not on the water.
        let marks = self.prepare_marks(win_w, win_h);
        if !marks.is_empty() {
            root = root.child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .size_full()
                    .child(canvas(
                        |bounds, _, _| bounds,
                        move |_bounds, _, window, _| {
                            for mark in &marks {
                                if let Some((photo, w, h)) = &mark.photo {
                                    let b = Bounds::new(
                                        point(px(mark.x - w / 2.0), px(mark.y - h / 2.0)),
                                        size(px(*w), px(*h)),
                                    );
                                    // The corner radius scales with the
                                    // photograph, as the console's does.
                                    let r = px(w.min(*h).min(8.0) / 2.0);
                                    let _ = window.paint_image(
                                        b,
                                        gpui::Corners::all(r),
                                        photo.clone(),
                                        0,
                                        false,
                                    );
                                } else {
                                    paint_unit_mark(
                                        window,
                                        (mark.x, mark.y),
                                        mark.symbol,
                                        mark.affiliation,
                                        mark.stale,
                                    );
                                }
                                if mark.focused {
                                    crate::shell_gpui::paint_unit_reticle(
                                        window,
                                        (mark.x, mark.y),
                                        view::RADAR_CYAN,
                                    );
                                }
                            }
                        },
                    )),
            );
        }

        // THE EXECUTION CHROME, as the Game Master's console draws it: the
        // two readings on the right (players and comms) and the scenario card
        // in the body. The phone's `ui.panels` does not replace them; it opens
        // the LARGER surface each one is a door to.
        if self.ready() && self.session.as_ref().is_some_and(|s| s.is_execution()) {
            root = root.child(self.execution_rail(win_w));
        }

        // The focused hull or group: the phone tapped it, the camera centred
        // it, and this says who and what it is beside its mark.
        if let Some((mark, unit)) = self.focused_unit(win_w, win_h) {
            root = root.child(self.unit_card(&unit, (mark.0, mark.1), win_w, win_h));
            root = root.child(self.embark_card(&unit, (mark.0, mark.1), win_w, win_h));
        }
        if let Some((mark, group)) = self.focused_group(win_w, win_h) {
            root = root.child(self.group_card(&group, mark, win_w, win_h));
        }

        // The larger surfaces the phone opens. They sit over the chrome, as a
        // modal does over the console's map.
        if self.ready() {
            root = self.panel_overlays(root, win_w, win_h);
        }

        // The waiting scrim: the map stays visible because the place matters,
        // but it is not the subject while the host has not opened observation.
        if !self.ready() {
            let (line, sentence) = waiting_copy(
                self.heard_from_host,
                &self.readiness,
                self.session.as_ref(),
                &self.server,
            );
            root = root.child(
                div()
                    .absolute()
                    .size_full()
                    .bg(crate::ink::rgba(view::CONSOLE_NIGHT.with_alpha(200)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w(px(ts(520.0)))
                            .border_1()
                            .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                            .bg(crate::ink::rgba(view::PANEL_SLATE))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .px(px(16.0))
                                    .py(px(8.0))
                                    .border_b_1()
                                    .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                                    .font_family("Saira Condensed")
                                    .text_size(px(ts(12.0)))
                                    .text_color(crate::ink::rgba(view::RADAR_CYAN))
                                    .child(line),
                            )
                            .child(
                                div()
                                    .p(px(16.0))
                                    .font_family("Saira Condensed")
                                    .text_size(px(ts(16.0)))
                                    .text_color(crate::ink::rgba(view::MAP_INK))
                                    .child(sentence),
                            )
                            .child(
                                div()
                                    .px(px(16.0))
                                    .pb(px(12.0))
                                    .font_family("PT Mono")
                                    .text_size(px(ts(11.0)))
                                    .text_color(crate::ink::rgba(view::BODY_SILVER))
                                    .child(self.server.clone()),
                            ),
                    ),
            );
        }

        // The strip: the exercise while there is one, this display's state
        // while there is not.
        let (title, subtitle, units) = match &self.session {
            Some(session) => {
                let subtitle = match &session.scenario {
                    Some(scenario) => match &scenario.step {
                        Some(step) => format!("{} \u{00b7} {step}", scenario.title),
                        None => scenario.title.clone(),
                    },
                    None => format!("{} \u{00b7} {}", session.phase.to_uppercase(), session.mode.to_uppercase()),
                };
                (
                    session.name.clone(),
                    subtitle,
                    format!("{} UNIT(S)", session.units.len()),
                )
            }
            None => (
                "NO EXERCISE".to_string(),
                "the host has published none".to_string(),
                "0 UNITS".to_string(),
            ),
        };
        root = root.child(
            div()
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .right(px(0.0))
                .h(px(ts(64.0)))
                .px(px(ts(16.0)))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .bg(crate::ink::rgba(view::CONSOLE_NIGHT.with_alpha(232)))
                .border_b_1()
                .border_color(crate::ink::rgba(view::HAIRLINE_SLATE))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .child(
                            div()
                                .font_family("Saira Condensed")
                                .text_size(px(ts(20.0)))
                                .text_color(crate::ink::rgba(view::MAP_INK))
                                .child(title),
                        )
                        .child(
                            div()
                                .font_family("Saira Condensed")
                                .text_size(px(ts(12.0)))
                                .text_color(crate::ink::rgba(view::BODY_SILVER))
                                .child(subtitle),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_end()
                        .gap(px(2.0))
                        .child(
                            div()
                                .font_family("PT Mono")
                                .text_size(px(ts(11.0)))
                                .text_color(crate::ink::rgba(if self.ready() {
                                    view::SIGNAL_GREEN
                                } else {
                                    view::BODY_SILVER
                                }))
                                .child(if self.ready() { "OBSERVER \u{00b7} LIVE" } else { "OBSERVER \u{00b7} WAITING" }),
                        )
                        .child(
                            div()
                                .font_family("PT Mono")
                                .text_size(px(ts(11.0)))
                                .text_color(crate::ink::rgba(view::BODY_SILVER))
                                .child(format!("{units} \u{00b7} ZOOM {:.1}", self.camera.zoom)),
                        ),
                ),
        );

        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(name: &str, phase: &str) -> SessionState {
        SessionState {
            game_id: 42,
            name: name.to_string(),
            phase: phase.to_string(),
            ..SessionState::default()
        }
    }

    /// THE WALL DRAWS THROUGH THE LADDER, and the default step is the
    /// identity: today's wall is unchanged, a bigger step grows the type.
    #[test]
    fn the_wall_scales_its_type_with_the_hosts_ladder() {
        let default = view::text_scale_at(view::TEXT_SCALE_DEFAULT);
        assert!((scaled(20.0, default) - 20.0).abs() < 1e-4);
        let largest = view::text_scale_at(view::TEXT_SCALES.len() - 1);
        assert!(scaled(20.0, largest) > 20.0);
        assert!((scaled(20.0, largest) - 20.0 * largest / default).abs() < 1e-4);
    }

    /// The four reasons the plan's table names, plus the state before the host
    /// has said anything at all. The copy is what a person standing in front of
    /// a wall reads, so it is a contract rather than decoration.
    #[test]
    fn every_waiting_state_says_what_it_is_waiting_for() {
        let no_session = Readiness::waiting(ReadinessReason::NoSession);
        assert_eq!(
            waiting_copy(true, &no_session, None, "http://host:8787"),
            (
                "NO EXERCISE OPEN".to_string(),
                "Waiting for the game master to start an exercise.".to_string()
            )
        );

        let planning = exercise("Exercise North", "planning");
        assert_eq!(
            waiting_copy(
                true,
                &Readiness::waiting(ReadinessReason::NotExecution),
                Some(&planning),
                "http://host:8787"
            ),
            (
                "OBSERVATION NOT OPEN".to_string(),
                "Exercise North is in planning. Observation opens at execution.".to_string()
            )
        );

        assert_eq!(
            waiting_copy(
                true,
                &Readiness::waiting(ReadinessReason::NoDisplay),
                Some(&planning),
                "http://host:8787"
            )
            .0,
            "CONNECTING THIS COMMAND CENTER"
        );
        assert_eq!(
            waiting_copy(
                true,
                &Readiness::waiting(ReadinessReason::Stale),
                Some(&planning),
                "http://host:8787"
            )
            .0,
            "THE HOST STOPPED REPORTING"
        );

        // Nothing heard: the server itself is the thing to name, because the
        // wall's problem is not the exercise.
        let (line, sentence) = waiting_copy(
            false,
            &Readiness::waiting(ReadinessReason::NoSession),
            None,
            "http://192.168.1.15:8787",
        );
        assert_eq!(line, "CANNOT REACH THE CONTROL SERVER");
        assert!(sentence.contains("192.168.1.15:8787"));
    }
}
