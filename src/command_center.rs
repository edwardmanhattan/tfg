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
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Application, Bounds, Context, IntoElement, ParentElement, Render, Styled,
    StyledImage, TitlebarOptions, Window, WindowBounds, WindowOptions, div, img, px, size,
};

use crate::control::{
    AcceptedCommand, CameraSnapshot, ClientConfig, ClientHandle, Command, DisplayHello, Event,
    GeoPoint, OBSERVER_VIEW, Readiness, ReadinessReason, SessionState, UnitState, ViewState,
    spawn_client,
};
use crate::map_gate::FrameSlot;
use crate::map_geometry;
use crate::shell_threads::{MapReq, MapResp};
use crate::view;

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
            CommandCenter::new(server, client, events_rx, commands_rx, map_tx, map_rx)
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
    fn new(
        server: String,
        client: ClientHandle,
        events: Receiver<Event>,
        commands: Receiver<AcceptedCommand>,
        map_tx: Sender<MapReq>,
        map_rx: Receiver<MapResp>,
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
            glide: HashMap::new(),
        }
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
                    ..
                } => {
                    self.note_positions(session.as_ref());
                    self.session = session;
                    self.readiness = readiness;
                    if let Some(view) = views
                        .get(OBSERVER_VIEW)
                        .or_else(|| views.get(crate::control::MAIN_VIEW))
                    {
                        self.follow(view);
                    }
                }
                Event::SessionChanged {
                    session, readiness, ..
                } => {
                    self.note_positions(session.as_ref());
                    self.session = session;
                    self.readiness = readiness;
                }
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

        // The markers: numbered, projected at the picture's own camera, and
        // scaled from the picture's pixel space to the window.
        if let Some(session) = &self.session {
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
                if !(0.0..=1.0).contains(&(x / f64::from(self.map_dims.0))) {
                    continue;
                }
                if !(0.0..=1.0).contains(&(y / f64::from(self.map_dims.1))) {
                    continue;
                }
                let left = win_w * (x as f32 / self.map_dims.0);
                let top = win_h * (y as f32 / self.map_dims.1);
                let ink = if unit.stale {
                    crate::ink::rgba(view::BODY_SILVER)
                } else {
                    crate::ink::rgba(view::RADAR_CYAN)
                };
                root = root.child(
                    div()
                        .absolute()
                        .left(px(left - 13.0))
                        .top(px(top - 13.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .w(px(26.0))
                                .h(px(26.0))
                                .rounded_full()
                                .border_1()
                                .border_color(ink)
                                .bg(crate::ink::rgba(view::CONSOLE_NIGHT))
                                .flex()
                                .items_center()
                                .justify_center()
                                .font_family("PT Mono")
                                .text_size(px(12.0))
                                .text_color(ink)
                                .child(unit.index.to_string()),
                        )
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(2.0))
                                .bg(crate::ink::rgba(view::CONSOLE_NIGHT.with_alpha(220)))
                                .font_family("Saira Condensed")
                                .text_size(px(12.0))
                                .text_color(crate::ink::rgba(view::MAP_INK))
                                .child(unit.name.clone()),
                        ),
                );
            }
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
                            .w(px(520.0))
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
                                    .text_size(px(12.0))
                                    .text_color(crate::ink::rgba(view::RADAR_CYAN))
                                    .child(line),
                            )
                            .child(
                                div()
                                    .p(px(16.0))
                                    .font_family("Saira Condensed")
                                    .text_size(px(16.0))
                                    .text_color(crate::ink::rgba(view::MAP_INK))
                                    .child(sentence),
                            )
                            .child(
                                div()
                                    .px(px(16.0))
                                    .pb(px(12.0))
                                    .font_family("PT Mono")
                                    .text_size(px(11.0))
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
                .h(px(64.0))
                .px(px(16.0))
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
                                .text_size(px(20.0))
                                .text_color(crate::ink::rgba(view::MAP_INK))
                                .child(title),
                        )
                        .child(
                            div()
                                .font_family("Saira Condensed")
                                .text_size(px(12.0))
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
                                .text_size(px(11.0))
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
                                .text_size(px(11.0))
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
