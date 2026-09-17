//! egui command-center shell, live edition over a persistent map scene.
//!
//! - A background poll thread feeds mock fixes into the [`Registry`] at the
//!   v0 cadence; markers glide by wall-clock fraction (`Registry::blend`).
//! - A background map thread owns one persistent [`LiveMap`]: recenter is a
//!   camera update + a few pumped frames (milliseconds), and follow-tracking
//!   is now just repeated camera updates, not pipeline rebuilds.
//! - Frames arrive as raw RGBA into versioned egui textures; overlay
//!   markers/trails/roster are immediate-mode (ADR-0001/0002).
//!
//! Run: `scripts/run-egui-window.sh`

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui;
use chrono::{TimeZone, Utc};
use tfg::backend::{FileReplay, HttpPoll, Invite, InviteClient, PollSource};
use tfg::catalog::{Catalog, Category};
use tfg::fleet::Fleet;
use tfg::groups::Groups;
use tfg::command::{Authority, Grant, GrantDenial, Leg, MoveCommand, Verb};
use tfg::geo::track::{Fix, FixSource, Registry, TrailBound, should_track};
use tfg::geo::GeoPosition;
use tfg::map_render::LiveMap;
use tfg::map_render::{anchor_center, project_mercator, unproject_mercator};
use tfg::overlay::hit_test;
use tfg::land::Land;
use tfg::sim::{
    CommandRefusal, MergeSource, OrderRefusal, OrderState, OrderView, SimCommand, SimEvent,
    SimSource,
};

const MAP_W: f64 = 800.0;
const MAP_H: f64 = 600.0;

/// Map thread protocol (task #38): every request carries the renderer
/// size so the canvas can fill the window; responses echo it back so the
/// texture is sized right.
type MapReq = (u64, (f64, f64), f64, (u32, u32), u32);
type MapResp = (u64, (f64, f64), f64, (u32, u32), Vec<u8>);
const CENTER: (f64, f64) = (-6.108, 106.910);
const ZOOM: f64 = 11.0;
/// Zone/flag threshold (grill #24, slice iii): zones at or above this
/// zoom, centroid flags below. Zoom controls feed the map thread.
const ZONE_ZOOM: f64 = 11.0;
/// Fixed ground padding around live hulls, in screen px (grill #24).
const ZONE_PAD_PX: f64 = 26.0;
/// Island scroll rule (scrollbar ticket): island bodies whose content can
/// exceed the window scroll vertically instead of clipping. 420px fits a
/// 640px viewport under the toolbar with room for window chrome; unbounded
/// lists inside already-capped islands keep their own tighter cap.
const ISLAND_SCROLL_MAX: f32 = 420.0;
const STYLE: &str = "https://tiles.openfreemap.org/styles/liberty";
/// Session stub pace (session flow): 7 real hours play 7 game days.
/// Full windows UI lands with the organizer flow; the ratio is the load-
/// bearing part (clock.rs derives everything else from it).
const SESSION_RATIO: f64 = 24.0;

/// UI state machine (state-machine grill, #26): phase gates authority,
/// the arm flag gates the engine. Orders flow only in Live + armed.
/// Panels read this; `start()` / `end()` / `reset()` are the only writers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Setup,
    Live,
    Closed,
}

/// Placement pointer tool. Active in Setup (initial fleet) and Live
/// (reinforcements); waypoint arming is a separate Live draft (`placing`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupTool {
    Select,
    Place,
}

/// Unified selection (Inspector-model ticket): one selection, ship-or-group.
/// Selecting either kind clears the other; Inspector visibility IS selection
/// presence — select opens it, deselect closes it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Selection {
    Ship(String),
    Group(String),
}

struct UiMode {
    phase: Phase,
    armed: Arc<AtomicBool>,
    tool: SetupTool,
}

impl UiMode {
    /// Booting into Setup IS the lobby (no Idle state).
    fn new(armed: Arc<AtomicBool>) -> Self {
        Self { phase: Phase::Setup, armed, tool: SetupTool::Select }
    }

    /// Orders flow here and only here.
    fn live(&self) -> bool {
        self.phase == Phase::Live && self.armed.load(Ordering::SeqCst)
    }

    /// Motion, trails, and orders panes belong to Live (any arm).
    fn in_live(&self) -> bool {
        self.phase == Phase::Live
    }

    fn start(&mut self) {
        self.phase = Phase::Live;
        self.armed.store(true, Ordering::SeqCst);
        self.tool = SetupTool::Select;
    }

    fn end(&mut self) {
        self.phase = Phase::Closed;
        self.armed.store(false, Ordering::SeqCst);
        self.tool = SetupTool::Select;
    }

    fn reset(&mut self) {
        self.phase = Phase::Setup;
        self.armed.store(false, Ordering::SeqCst);
        self.tool = SetupTool::Select;
    }
}
/// Top-level mode (task #39): a session exists only in Simulation;
/// Presentation renders live backend data through the same Registry.
/// Switching clears the *view* either way — wire ships and sim ships
/// never share a screen — while the simulation underneath (session,
/// roster, journal, owned units) survives for the return trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppMode {
    Presentation,
    Simulation,
}

/// Runtime wire backends for the poll thread (task #39). Replay stays
/// boot-only (env); the Connection island swaps Http/Empty live.
enum WireKind {
    Http(String),
    Replay(String),
    Empty,
}

/// Wire source that yields nothing: a disconnected presentation, or a
/// simulation running on sim data alone.
struct EmptyWire;

impl PollSource for EmptyWire {
    fn poll(&mut self) -> Result<Vec<Fix>, String> {
        Ok(Vec::new())
    }
}

/// Build a wire source plus its status description.
fn build_wire(kind: &WireKind) -> Result<(Box<dyn PollSource>, String), String> {
    match kind {
        WireKind::Http(url) => HttpPoll::new(url)
            .map(|h| (Box::new(h) as Box<dyn PollSource>, format!("HTTP {url}")))
            .map_err(|e| e),
        WireKind::Replay(path) => FileReplay::from_file(path)
            .map(|r| (Box::new(r) as Box<dyn PollSource>, format!("replay {path}")))
            .map_err(|e| e),
        WireKind::Empty => Ok((Box::new(EmptyWire) as Box<dyn PollSource>, "empty".to_string())),
    }
}
/// One parsed history line: display text plus its actor and ships for
/// the unit/player filters (task #41).
struct LogLine {
    text: String,
    actor: String,
    ships: Vec<String>,
}

/// One placement-affecting journal entry (task #41): take-control adds
/// the ghost, release removes it. Parsed from Command payloads.
struct ReplayEvent {
    game_ts: String,
    ship: String,
    lat: f64,
    lon: f64,
    placed: bool,
}

/// v0 poll cadence, in seconds.
const POLL_SECS: f64 = 2.0;
/// Pumped frames per map request, by kind (task #45): jumps rebuild
/// the view (6), zoom steps split the difference (4), follow tracking
/// turns over fast on near-neighbor tiles (2). Fewer pumps = fresher
/// frames while panning; tile fetch itself stays async either way.
const JUMP_PUMP: u32 = 6;
const ZOOM_PUMP: u32 = 4;
const TRACK_PUMP: u32 = 2;
/// Slowest follow re-request rate: tiles can't arrive faster than the
/// network, so chasing harder only renders stale centers (task #45).
const TRACK_MIN_INTERVAL_MS: u64 = 800;
/// Lead a followed ship by this many seconds of dead reckoning, so tile
/// fetches run ahead of motion instead of behind it (task #45).
const TRACK_LEAD_SECS: f64 = 8.0;

struct ShipMarker {
    id: String,
    x: f64,
    y: f64,
    stale: bool,
    source: FixSource,
    trail: Vec<(f64, f64)>,
}

/// One desktop's order draft (slice iv, grill #25): pending waypoint,
/// waypoint arming, and speed. Drafts persist per desktop across switches.
#[derive(Clone)]
struct Draft {
    waypoint: Option<(f64, f64)>,
    placing: bool,
    speed: f32,
}

impl Default for Draft {
    fn default() -> Self {
        Self { waypoint: None, placing: false, speed: 20.0 }
    }
}

/// Zone polygon for one group, in screen px (slice iii, grill #24).
/// Carries the group id so zones are clickable like flags.
struct ZoneGeom {
    group: String,
    pts: Vec<(f32, f32)>,
    fill: egui::Color32,
    stroke: egui::Color32,
}

/// Point-in-polygon over a zone's screen pts (ray cast). Degenerate
/// hulls (fewer than 3 pts) never hit: click the ship or flag instead.
fn in_poly(px: f64, py: f64, pts: &[(f32, f32)]) -> bool {
    if pts.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = (pts[i].0 as f64, pts[i].1 as f64);
        let (xj, yj) = (pts[j].0 as f64, pts[j].1 as f64);
        if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Collapsed group flag: centroid screen point + lat/lon for click-to-expand.
struct FlagGeom {
    x: f32,
    y: f32,
    lat: f64,
    lon: f64,
    label: String,
    group: String,
    color: egui::Color32,
}

/// Monotone-chain convex hull over screen points (grill #24: live hulls).
fn convex_hull(mut pts: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    if pts.len() <= 1 {
        return pts;
    }
    pts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let mut lower: Vec<(f64, f64)> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2
            && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0
        {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<(f64, f64)> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2
            && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0
        {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Fixed ground padding (grill #24): push hull points outward from the
/// centroid by PAD px.
fn pad_hull(pts: &[(f64, f64)], pad: f64) -> Vec<(f64, f64)> {
    if pts.is_empty() {
        return Vec::new();
    }
    let n = pts.len() as f64;
    let (cx, cy) = (
        pts.iter().map(|p| p.0).sum::<f64>() / n,
        pts.iter().map(|p| p.1).sum::<f64>() / n,
    );
    pts.iter()
        .map(|&(x, y)| {
            let (dx, dy) = (x - cx, y - cy);
            let d = (dx * dx + dy * dy).sqrt();
            if d < 1e-6 {
                (x, y)
            } else {
                (x + dx / d * pad, y + dy / d * pad)
            }
        })
        .collect()
}

struct ShipApp {
    map_tex: Option<egui::TextureHandle>,
    /// What the current texture was rendered for: the canvas translates
    /// (and scales, across zooms) it to the live camera while the fresh
    /// tile is in flight, so gestures glide instead of stepping.
    tex_center: (f64, f64),
    tex_zoom: f64,
    tex_px: (u32, u32),
    /// Viewport center shared by projection and (on swap) the frame.
    center: (f64, f64),
    registry: Registry,
    poll_rx: Receiver<Vec<Fix>>,
    map_req_tx: Option<Sender<MapReq>>,
    map_resp_rx: Receiver<MapResp>,
    map_seq: u64,
    recentering: Option<String>,
    shutdown: std::sync::Arc<AtomicBool>,
    poll_handle: Option<JoinHandle<()>>,
    map_handle: Option<JoinHandle<()>>,
    last_poll: Instant,
    hidden: HashSet<String>,
    following: Option<String>,
    show_trail: bool,
    /// The UI state machine (state-machine grill, #26): the only flow
    /// state panels may read. Replaces session_live / placing_unit.
    mode: UiMode,
    /// Session stub: default windows stamped at start.
    session_windows: Option<(String, String, String, String)>,
    /// Per-session journal path (rotated on Start) + frozen transcript
    /// for the Closed state (the file is the export).
    session_log_path: std::path::PathBuf,
    session_seq: usize,
    transcript: Vec<String>,
    /// Islands (islands grill, #27): floating panels over the fullscreen
    /// map. Run-local visibility; phase decides what may show.
    show_session: bool,
    show_roster: bool,
    show_orders: bool,
    show_log: bool,
    /// Wizard (islands grill, #27): stepped Setup, dismissed on Live.
    wizard_step: usize,
    wizard_done: bool,
    /// Event feed for the Log island: capped human lines drained from
    /// sim events (arrivals, refusals, overrides, blockages).
    event_feed: VecDeque<String>,
    /// Unified selection (Inspector-model ticket): the Inspector shows
    /// whichever is set and shuts when it clears. Helpers below
    /// (select_ship/select_group/deselect) are the only writers.
    selection: Option<Selection>,
    /// Wall-clock last-seen + fix counts per ship (inspector readout;
    /// stamped as poll rounds arrive, so the geo model stays time-free).
    last_seen: HashMap<String, Instant>,
    fix_count: HashMap<String, usize>,
    /// Sim orders channel + read views (prototype sim loop).
    sim_cmd_tx: Option<Sender<SimCommand>>,
    sim_evt_rx: Receiver<SimEvent>,
    order_views: HashMap<String, OrderView>,
    controlled: HashSet<String>,
    pending_waypoint: Option<(f64, f64)>,
    placing: bool,
    order_speed: f32,
    /// Land test for order validation (ticket #22): land waypoints are
    /// rejected in the UI before they ever reach the sim.
    land: Option<Land>,
    /// Last order refusal from the sim, shown until the next attempt.
    order_warning: Option<String>,
    /// Unit taxonomy (grill #18): class chosen at take-control.
    catalog: Catalog,
    selected_class: usize,
    /// Fleet picker (task #29): organizer hull seeds from
    /// assets/fleet.json, filtered by category/class/text, placed by hand
    /// on the map — never automatically.
    fleet: Fleet,
    show_fleet: bool,
    fleet_cat: usize,
    fleet_class: Option<String>,
    fleet_query: String,
    fleet_pick: Option<String>,
    placed_fleet: HashSet<String>,
    /// Setup slice (ii): WIB-entered windows, local roster, seat drafts.
    /// Times are entered in WIB and stored as UTC; the roster names assignees.
    time_real_start: String,
    time_real_end: String,
    time_game_start: String,
    time_game_end: String,
    roster: Vec<String>,
    roster_input: String,
    /// Helm per placed unit (unit_id -> user). Commander seats for
    /// Satgas/Gugus wait for groups (slice iii); unit commanders draft here.
    helm: HashMap<String, String>,
    unit_commander: HashMap<String, String>,
    /// Invites (slice v): local records (source of truth) mirrored to the
    /// mock backend when connected.
    invites: Vec<Invite>,
    invite_seq: usize,
    invite_status: String,
    /// Ratio derived from the entered windows (replaces the 24x stub).
    session_ratio: f64,
    /// Map zoom (slice iii): feeds the map thread per request; the
    /// zone/flag threshold reads it per frame.
    zoom: f64,
    /// Seamless zoom throttle (task #43): overlays track every tick,
    /// the texture follows at most every 250ms; dirty flushes the tail.
    last_zoom_req: Instant,
    /// Slowest follow re-request instant (task #45).
    last_track_req: Instant,
    zoom_dirty: bool,
    /// Top-level mode (task #39): session only in Simulation.
    app_mode: AppMode,
    /// Connection island state (task #39): URL field, current source
    /// description, last connect/disconnect note, control channel.
    backend_url: String,
    wire_desc: String,
    conn_status: String,
    show_connection: bool,
    wire_ctl_tx: Option<Sender<WireKind>>,
    /// Session-log history view (task #40): selected past journal.
    log_view_path: Option<std::path::PathBuf>,
    /// Replay (task #41): placement events, slider position, map ghosts,
    /// unit/player filter.
    log_events: Vec<ReplayEvent>,
    replay_pos: usize,
    show_replay: bool,
    log_filter: String,
    /// Full-window canvas (task #38): renderer pixels, current display
    /// points, and last frame's desired size (resize debounce).
    map_px: (u32, u32),
    map_view: (f64, f64),
    last_desired_px: (u32, u32),
    /// Session groups (slice iii): Satgas/Gugus hierarchy + drafts.
    groups: Groups,
    show_groups: bool,
    group_seq: usize,
    satgas_name: String,
    satgas_commander: Option<String>,
    satgas_members: HashSet<String>,
    gugus_name: String,
    gugus_commander: Option<String>,
    gugus_members: HashSet<String>,
    group_error: Option<String>,
    /// Desktops (slice iv, grill #25): act-as identity + scope tabs.
    /// No identity = organizer with the merged All desktop.
    acting_as: Option<String>,
    desktop: String,
    /// Per-desktop order drafts, saved on every desktop switch.
    drafts: HashMap<String, Draft>,
    /// Game clock readout from the sim (ADR-0004: game time is derived
    /// and reported per round; the UI never computes it itself).
    game_elapsed_secs: Option<u64>,
    game_ratio: f64,
    game_paused: bool,
    /// Real + derived game clock readings, humane format, per round.
    real_ts: Option<String>,
    game_ts: Option<String>,
}

/// Parse WIB "YYYY-MM-DD HH:MM" entry into UTC (slice ii): storage is
/// always UTC; the +7 offset lives only in this parser.
fn parse_wib(s: &str) -> Option<chrono::DateTime<Utc>> {
    let naive = chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M").ok()?;
    chrono::FixedOffset::east_opt(7 * 3600)?
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.with_timezone(&Utc))
}

type Windows = (chrono::DateTime<Utc>, chrono::DateTime<Utc>, chrono::DateTime<Utc>, chrono::DateTime<Utc>);

/// All four window entries parse and both spans run forward.
fn parse_windows(rs: &str, re: &str, gs: &str, ge: &str) -> Option<Windows> {
    let (rs, re, gs, ge) = (parse_wib(rs)?, parse_wib(re)?, parse_wib(gs)?, parse_wib(ge)?);
    (re > rs && ge > gs).then_some((rs, re, gs, ge))
}

impl ShipApp {
    fn ship_color(id: &str) -> egui::Color32 {
        match id {
            "nordwind" => egui::Color32::from_rgb(0x25, 0x63, 0xeb),
            "ostsee" => egui::Color32::from_rgb(0xdc, 0x26, 0x26),
            _ => egui::Color32::from_rgb(0x16, 0xa3, 0x4a),
        }
    }

    /// Drain pending poll rounds, then derive marker geometry for this frame.
    fn markers(&mut self) -> Vec<ShipMarker> {
        let mut rounds = 0;
        for fixes in self.poll_rx.try_iter() {
            rounds += 1;
            for f in &fixes {
                self.last_seen.insert(f.ship_id.clone(), Instant::now());
                *self.fix_count.entry(f.ship_id.clone()).or_insert(0) += 1;
            }
            let acked = self.registry.poll(fixes);
            // Ingest acks (Log grill, #20): report stamped seqs back to
            // the sim AFTER ingest, so journal entries can cite fix seqs.
            if let Some(tx) = &self.sim_cmd_tx {
                for (ship_id, seq) in acked {
                    let _ = tx.send(SimCommand::FixAck { ship_id, seq });
                }
            }
        }
        // Drain events first (same borrow rule as the sim command drain):
        // the channel iterator borrows self, feed() needs it mutably.
        let evts: Vec<SimEvent> = self.sim_evt_rx.try_iter().collect();
        for evt in evts {
            match evt {
                SimEvent::Orders(views) => {
                    for v in views {
                        self.order_views.insert(v.ship_id.clone(), v);
                    }
                }
                SimEvent::Clock { game_elapsed_secs, ratio, paused } => {
                    self.game_elapsed_secs = Some(game_elapsed_secs);
                    self.game_ratio = ratio;
                    self.game_paused = paused;
                }
                SimEvent::ClockReadout { real_ts, game_ts, paused } => {
                    self.real_ts = Some(real_ts);
                    self.game_ts = game_ts;
                    self.game_paused = paused;
                }
                SimEvent::OrderRefused { ship_id, reason } => {
                    let why = match reason {
                        OrderRefusal::LandWaypoint => "waypoint is on land",
                        OrderRefusal::LandBetween => "path crosses land",
                    };
                    self.feed(format!("refused {ship_id}: {why}"));
                    self.order_warning = Some(format!("{ship_id}: {why}"));
                }
                SimEvent::CommandRefused { ship_id, reason } => {
                    let why = match reason {
                        CommandRefusal::LowerAuthority { held_rank, by_rank } => format!(
                            "overruled by higher authority (held {held_rank}, by {by_rank})"
                        ),
                        CommandRefusal::Grant(GrantDenial::Expired) => {
                            "command expired".to_string()
                        }
                        CommandRefusal::Grant(GrantDenial::OutsideScope) => {
                            "ship outside command scope".to_string()
                        }
                        CommandRefusal::Grant(GrantDenial::VerbDenied) => {
                            "verb not granted".to_string()
                        }
                    };
                    self.feed(format!("command refused ({ship_id}): {why}"));
                    self.order_warning = Some(format!("{ship_id}: {why}"));
                }
                SimEvent::CommandOverridden { ship_id, prev_rank, by_rank } => {
                    // Escalation, not a hijack: the command succeeded and
                    // took the ship to a higher authority (e.g. the
                    // organizer's Satgas order lifting UNIT-held ships).
                    // Loud in the feed + journal, but no sticky warning:
                    // warnings are for orders that did NOT happen.
                    self.feed(format!("authority {ship_id}: {prev_rank} -> {by_rank}"));
                }
                SimEvent::ShipBlocked { ship_id } => {
                    self.feed(format!("blocked at coast: {ship_id}"));
                }
                SimEvent::Arrival { ship_id } => {
                    self.feed(format!("arrived {ship_id}"));
                }
            }
        }
        if rounds > 0 {
            self.last_poll = Instant::now();
            let ships = self.registry.ships();
            eprintln!(
                "poll: {} ship(s){}",
                ships.len(),
                ships
                    .iter()
                    .filter(|s| s.stale)
                    .map(|s| format!(" [stale: {}]", s.ship_id))
                    .collect::<String>()
            );
        }
        let frac = (self.last_poll.elapsed().as_secs_f64() / POLL_SECS).clamp(0.0, 1.0);
        let center = self.center;
        let (mw, mh) = self.map_dims();
        self.registry
            .ships()
            .iter()
            .map(|s| {
                let pos = self.registry.blend(&s.ship_id, frac).unwrap_or(s.latest.position);
                let (x, y) = project_mercator(pos.latitude, pos.longitude, center, self.zoom, mw, mh);
                let trail = s
                    .trail
                    .iter()
                    .map(|p| project_mercator(p.latitude, p.longitude, center, self.zoom, mw, mh))
                    .collect();
                ShipMarker { id: s.ship_id.clone(), x, y, stale: s.stale, source: s.source, trail }
            })
            .collect()
    }

    /// Switch top-level modes (task #39): the view clears either way
    /// and Presentation disarms the engine, so watching never stands
    /// anything up. Simulation state underneath is untouched.
    fn set_app_mode(&mut self, mode: AppMode) {
        if self.app_mode == mode {
            return;
        }
        self.app_mode = mode;
        self.registry = Registry::new(TrailBound::default());
        self.deselect();
        self.following = None;
        self.recentering = None;
        if mode == AppMode::Presentation {
            self.mode.armed.store(false, Ordering::SeqCst);
            self.show_session = false;
            self.show_roster = false;
            self.show_fleet = false;
            self.show_orders = false;
            self.show_groups = false;
            self.show_connection = true;
        } else {
            self.show_connection = false;
        }
        eprintln!("mode: {mode:?}");
    }

    /// Connection island (task #39): the wire controls plus status.
    /// Same Registry underneath — this only swaps the source.
    fn connection_island(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connection");
        ui.label(format!("source: {}", self.wire_desc));
        let backend_ships = self
            .registry
            .ships()
            .iter()
            .filter(|s| s.source == FixSource::Wire)
            .count();
        ui.label(format!("backend ships in view: {backend_ships}"));
        ui.horizontal(|ui| {
            ui.label("backend:");
            ui.text_edit_singleline(&mut self.backend_url);
        });
        ui.horizontal(|ui| {
            if ui.button("connect").clicked() {
                let url = self.backend_url.clone();
                if let Some(tx) = &self.wire_ctl_tx {
                    match HttpPoll::new(&url) {
                        Ok(_) => {
                            let _ = tx.send(WireKind::Http(url.clone()));
                            self.wire_desc = format!("HTTP {url}");
                            self.conn_status = format!("connected {url}");
                            eprintln!("wire: HTTP {url}");
                        }
                        Err(e) => {
                            self.conn_status = format!("connect failed: {e}");
                        }
                    }
                }
            }
            if ui.button("disconnect").clicked() {
                if let Some(tx) = &self.wire_ctl_tx {
                    let _ = tx.send(WireKind::Empty);
                    self.wire_desc = "empty".to_string();
                    self.conn_status = "disconnected".to_string();
                    eprintln!("wire: empty");
                }
            }
        });
        ui.label(&self.conn_status);
    }

    /// Display size in points: what overlays project against. The
    /// renderer works in physical pixels; both derive per frame.
    fn map_dims(&self) -> (f64, f64) {
        self.map_view
    }

    /// Apply finished map frames (last-writer-wins by sequence). The
    /// camera stays authoritative: the texture only records what it was
    /// rendered for (center/zoom/size) so the canvas can translate it
    /// while a fresher tile is in flight. One texture, updated in place.
    fn drain_map(&mut self, ctx: &egui::Context) {
        for (seq, center, zoom, size, rgba) in self.map_resp_rx.try_iter() {
            if seq == self.map_seq {
                let img = if LiveMap::is_premultiplied() {
                    egui::ColorImage::from_rgba_premultiplied(
                        [size.0 as usize, size.1 as usize],
                        &rgba,
                    )
                } else {
                    egui::ColorImage::from_rgba_unmultiplied(
                        [size.0 as usize, size.1 as usize],
                        &rgba,
                    )
                };
                match &mut self.map_tex {
                    Some(tex) => tex.set(img, egui::TextureOptions::LINEAR),
                    None => {
                        self.map_tex =
                            Some(ctx.load_texture("map", img, egui::TextureOptions::LINEAR));
                    }
                }
                self.tex_center = center;
                self.tex_zoom = zoom;
                self.tex_px = size;
                if self.recentering.is_some() {
                    eprintln!("recentered");
                }
                self.recentering = None;
            }
        }
    }

    /// Ask the map thread for a frame centered on `at` for `ship`. The
    /// camera jumps now; the texture catches up translated underneath.
    fn request_frame(&mut self, ship: &str, at: (f64, f64)) {
        let Some(tx) = self.map_req_tx.clone() else {
            return; // shutting down
        };
        self.map_seq += 1;
        self.recentering = Some(ship.to_string());
        self.center = at;
        eprintln!("recentering on {ship}…");
        let _ = tx.send((self.map_seq, at, self.zoom, self.map_px, JUMP_PUMP));
    }

    /// Follow-tracking frame (task #45): light pump, no recenter label.
    /// The camera tracks now; the texture follows translated.
    fn track_frame(&mut self, ship: &str, at: (f64, f64)) {
        let Some(tx) = self.map_req_tx.clone() else {
            return; // shutting down
        };
        self.map_seq += 1;
        self.recentering = Some(ship.to_string());
        self.center = at;
        let _ = tx.send((self.map_seq, at, self.zoom, self.map_px, TRACK_PUMP));
    }

    /// Re-render at the current center and zoom without a recenter label.
    fn refresh_map(&mut self) {
        self.refresh_map_with(ZOOM_PUMP);
    }

    /// Gesture preview (smoothness pass): pump 1 keeps the tile pipeline
    /// under ~a throttle tick while the translated stale texture carries
    /// the frame; the full-pump settle lands on the throttle flush.
    fn refresh_map_light(&mut self) {
        self.refresh_map_with(1);
    }

    fn refresh_map_with(&mut self, pump: u32) {
        let Some(tx) = self.map_req_tx.clone() else {
            return; // shutting down
        };
        self.map_seq += 1;
        let _ = tx.send((self.map_seq, self.center, self.zoom, self.map_px, pump));
    }

    /// Zoom step (slice iii): clamps, re-renders, and reports. Zones give
    /// way to flags below ZONE_ZOOM.
    fn zoom_by(&mut self, delta: f64) {
        self.zoom = (self.zoom + delta).clamp(3.0, 18.0);
        eprintln!("zoom {:.0}", self.zoom);
        self.refresh_map();
    }

    /// Select a ship: clears any group selection and opens the Inspector.
    fn select_ship(&mut self, id: String) {
        self.selection = Some(Selection::Ship(id));
    }

    /// Select a group: clears any ship selection and opens the Inspector.
    fn select_group(&mut self, gid: String) {
        self.selection = Some(Selection::Group(gid));
    }

    /// Deselect: the Inspector shuts with the selection. Follow (camera)
    /// is independent and untouched.
    fn deselect(&mut self) {
        self.selection = None;
    }

    /// Group display name + member units for a Satgas or Gugus id.
    fn group_info(&self, gid: &str) -> Option<(String, Vec<String>)> {
        if let Some(s) = self.groups.satgas_list().iter().find(|s| s.id == gid) {
            return Some((s.name.clone(), s.units.clone()));
        }
        if let Some(g) = self.groups.gugus_list().iter().find(|g| g.id == gid) {
            return Some((g.name.clone(), self.groups.gugus_units(&g.id)));
        }
        None
    }

    /// Authority the acting identity holds over these units: the
    /// organizer commands all; others take their highest group level
    /// over the set (Gugus > Satgas > unit), falling back to unit
    /// level where the desktop scope allows (helm). None = view-only.
    fn command_authority(&self, units: &[String]) -> Option<Authority> {
        let Some(user) = self.acting_as.as_ref() else {
            return Some(Authority::ORGANIZER);
        };
        let mut best: Option<Authority> = None;
        for u in units {
            let a = match self.groups.authority(user, u, &self.unit_commander) {
                Some(a) => a,
                None if self.action_allows(u) => Authority::UNIT,
                None => continue,
            };
            best = Some(best.map_or(a, |b: Authority| b.max(a)));
        }
        best
    }

    /// Human label for a command authority level.
    fn authority_label(a: Authority) -> &'static str {
        if a == Authority::ORGANIZER {
            "organizer"
        } else if a == Authority::GUGUS {
            "gugus"
        } else if a == Authority::SATGAS {
            "satgas"
        } else {
            "unit"
        }
    }

    /// Display name for a placed unit: fleet name + hull, or the raw id.
    fn unit_label(&self, id: &str) -> String {
        self.fleet
            .get(id)
            .map(|u| format!("{} ({})", u.name, u.hull))
            .unwrap_or_else(|| id.to_string())
    }

    /// Seat labels for one roster user: helm, unit command, group command.
    fn seat_labels(&self, user: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (u, h) in &self.helm {
            if h == user {
                out.push(format!("helm {}", self.unit_label(u)));
            }
        }
        for (u, c) in &self.unit_commander {
            if c == user {
                out.push(format!("cmdr {}", self.unit_label(u)));
            }
        }
        for s in self.groups.satgas_list() {
            if s.commander.as_deref() == Some(user) {
                out.push(format!("cmdr {}", s.name));
            }
        }
        for g in self.groups.gugus_list() {
            if g.commander.as_deref() == Some(user) {
                out.push(format!("cmdr {}", g.name));
            }
        }
        out.sort();
        out
    }

    /// Backend base URL when the mock/real backend is connected (same env
    /// as the poll source).
    fn backend_base() -> Option<String> {
        std::env::var("TFG_BACKEND_URL").ok()
    }

    /// Push every local invite record to the mock (client codes win).
    fn sync_invites(&mut self) {
        let Some(base) = Self::backend_base() else {
            self.invite_status = "no backend connected".to_string();
            return;
        };
        let client = match InviteClient::new(&base) {
            Ok(c) => c,
            Err(e) => {
                self.invite_status = format!("mock unreachable: {e}");
                return;
            }
        };
        let mut n = 0;
        for inv in &self.invites {
            match client.issue(&inv.user, &inv.seat, &inv.code) {
                Ok(_) => {
                    n += 1;
                }
                Err(e) => {
                    self.invite_status = format!("sync stopped at {}: {e}", inv.code);
                    return;
                }
            }
        }
        self.invite_status = format!("synced {n} invite(s) to mock");
    }

    /// Pull the mock list; local redeemed flags follow by code.
    fn refresh_invites(&mut self) {
        let Some(base) = Self::backend_base() else {
            self.invite_status = "no backend connected".to_string();
            return;
        };
        match InviteClient::new(&base).and_then(|c| c.list()) {
            Ok(remote) => {
                let mut n = 0;
                for inv in &mut self.invites {
                    if !inv.redeemed && remote.iter().any(|r| r.code == inv.code && r.redeemed) {
                        inv.redeemed = true;
                        n += 1;
                    }
                }
                self.invite_status = format!("refreshed from mock ({n} redeemed)");
            }
            Err(e) => {
                self.invite_status = format!("mock unreachable: {e}");
            }
        }
    }

    /// Stash the current draft under the current desktop.
    fn save_draft(&mut self) {
        let cur = self.desktop.clone();
        self.drafts.insert(
            cur,
            Draft { waypoint: self.pending_waypoint, placing: self.placing, speed: self.order_speed },
        );
    }

    /// Restore a desktop's draft (default speed 20 kn when never drafted).
    fn load_draft(&mut self, id: &str) {
        let d = self.drafts.get(id).cloned().unwrap_or_default();
        self.pending_waypoint = d.waypoint;
        self.placing = d.placing;
        self.order_speed = d.speed;
    }

    /// Switch desktop tabs (grill #25): drafts persist per desktop, the
    /// camera stays, observers lose the orders pane.
    fn switch_desktop(&mut self, id: String) {
        self.save_draft();
        self.load_draft(&id);
        self.desktop = id;
        if self.is_observer() {
            self.show_orders = false;
        }
    }

    /// Default desktop for the acting identity: All, or the first scope.
    fn default_desktop(&self) -> String {
        match &self.acting_as {
            None => "all".to_string(),
            Some(user) => self
                .groups
                .scopes_for(user, &self.unit_commander, &self.helm)
                .first()
                .map(|s| s.id.clone())
                .unwrap_or_else(|| "all".to_string()),
        }
    }

    /// Seat-less acting players get the merged view-only desktop.
    fn is_observer(&self) -> bool {
        match &self.acting_as {
            None => false,
            Some(user) => self.groups.scopes_for(user, &self.unit_commander, &self.helm).is_empty(),
        }
    }

    /// Units the current desktop may act on: None = unrestricted
    /// (organizer). Everything still views all.
    fn action_units(&self) -> Option<HashSet<String>> {
        let user = self.acting_as.as_ref()?;
        let mut scopes = self.groups.scopes_for(user, &self.unit_commander, &self.helm);
        if scopes.is_empty() {
            return Some(HashSet::new());
        }
        let pos = scopes.iter().position(|s| s.id == self.desktop).unwrap_or(0);
        Some(scopes.remove(pos).units.into_iter().collect())
    }

    /// Whether the current desktop may command this ship.
    fn action_allows(&self, ship: &str) -> bool {
        match self.action_units() {
            None => true,
            Some(set) => set.contains(ship),
        }
    }

    /// Desktop tabs: (id, label, unit count). Organizer gets the merged
    /// All; observers a single view-only tab.
    fn desktop_tabs(&self) -> Vec<(String, String, usize)> {
        match &self.acting_as {
            None => vec![("all".to_string(), "All".to_string(), self.placed_fleet.len())],
            Some(user) => {
                let scopes = self.groups.scopes_for(user, &self.unit_commander, &self.helm);
                if scopes.is_empty() {
                    return vec![("all".to_string(), "Observer".to_string(), 0)];
                }
                scopes
                    .into_iter()
                    .map(|s| {
                        let n = s.units.len();
                        let label = match s.id.strip_prefix("unit:") {
                            Some(uid) => self
                                .fleet
                                .get(uid)
                                .map(|u| format!("{} ({})", u.name, u.hull))
                                .unwrap_or(s.label),
                            None => s.label,
                        };
                        (s.id, label, n)
                    })
                    .collect()
            }
        }
    }

    /// Push one human line to the Log island feed (capped).
    fn feed(&mut self, msg: String) {
        eprintln!("{msg}");
        self.event_feed.push_back(msg);
        while self.event_feed.len() > 30 {
            self.event_feed.pop_front();
        }
    }

    /// Start action shared by the Session island and the wizard:
    /// default windows, fresh per-session journal, 24:1 clock, armed.
    fn start_session(&mut self) {
        // Going live is organizer-only (slice iv).
        if self.acting_as.is_some() {
            self.feed("start refused: organizer-only".to_string());
            return;
        }
        // Windows were validated in the Session island; re-parse defensively.
        let Some((rs, re, gs, ge)) = parse_windows(
            &self.time_real_start,
            &self.time_real_end,
            &self.time_game_start,
            &self.time_game_end,
        ) else {
            self.feed("start refused: fix the time windows".to_string());
            return;
        };
        let fmt = "%Y-%m-%d %H:%M UTC";
        self.session_windows = Some((
            rs.format(fmt).to_string(),
            re.format(fmt).to_string(),
            gs.format(fmt).to_string(),
            ge.format(fmt).to_string(),
        ));
        // Ratio derived from windows (grill #23): game span over real span.
        let ratio = (ge - gs).num_seconds() as f64 / (re - rs).num_seconds() as f64;
        self.session_ratio = ratio;
        self.session_seq += 1;
        let path = std::path::PathBuf::from(format!(
            "{}/target/tfg-session-log-{}.jsonl",
            env!("CARGO_MANIFEST_DIR"),
            self.session_seq
        ));
        if let Some(tx) = &self.sim_cmd_tx {
            let _ = tx.send(SimCommand::RotateJournal { path: path.clone() });
            let _ = tx.send(SimCommand::SetClockRatio { ratio });
        }
        self.session_log_path = path;
        self.mode.start();
        self.wizard_done = true;
        // Going live needs the working islands; the Fleet picker stays
        // available for Live reinforcements, so it is left as-is. The
        // Inspector opens on first selection, not here.
        self.show_roster = true;
        self.show_orders = true;
        self.show_log = true;
        eprintln!("session live at {ratio:.1}x");
    }

    /// A session exists once started (task #40): working islands unlock
    /// off Live, and lock again when the session ends or resets.
    fn session_live(&self) -> bool {
        self.mode.phase == Phase::Live
    }

    /// Lock the working islands (task #40).
    fn close_working_islands(&mut self) {
        self.show_roster = false;
        self.show_fleet = false;
        self.show_orders = false;
        self.show_log = false;
        self.show_groups = false;
    }

    /// Past session journals on disk, oldest first.
    fn session_log_files() -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        if let Ok(dir) = std::fs::read_dir(format!("{}/target", env!("CARGO_MANIFEST_DIR"))) {
            for e in dir.flatten() {
                let p = e.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("tfg-session-log") && p.extension().and_then(|x| x.to_str()) == Some("jsonl") {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }

    /// History view for one journal: structured lines plus the units and
    /// players seen (ship ids from payloads, non-sim actors).
    fn read_log_view(path: &std::path::PathBuf) -> (Vec<LogLine>, Vec<String>, Vec<String>) {
        let mut entries = Vec::new();
        let mut units = std::collections::BTreeSet::new();
        let mut players = std::collections::BTreeSet::new();
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                let v: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let actor = v["actor"].as_str().unwrap_or("?").to_string();
                if actor != "sim" && !actor.starts_with("authority:") {
                    players.insert(actor.clone());
                }
                let payload = &v["payload"];
                let mut ships = Vec::new();
                for key in ["ship_id", "ship", "ships"] {
                    if let Some(s) = payload[key].as_str() {
                        ships.push(s.to_string());
                        units.insert(s.to_string());
                    }
                    if let Some(arr) = payload[key].as_array() {
                        for s in arr.iter().filter_map(|x| x.as_str()) {
                            ships.push(s.to_string());
                            units.insert(s.to_string());
                        }
                    }
                }
                entries.push(LogLine {
                    text: format!(
                        "{} {} {}: {}",
                        v["game_ts"].as_str().unwrap_or("—"),
                        actor,
                        v["kind"].as_str().unwrap_or("?"),
                        payload,
                    ),
                    actor,
                    ships,
                });
            }
        }
        (entries, units.into_iter().collect(), players.into_iter().collect())
    }

    /// Placement events for the replay slider (task #41): take-control /
    /// release Commands in journal order.
    fn parse_replay(path: &std::path::PathBuf) -> Vec<ReplayEvent> {
        let mut out = Vec::new();
        if let Ok(text) = std::fs::read_to_string(path) {
            for line in text.lines() {
                let v: serde_json::Value = match serde_json::from_str(line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if v["kind"].as_str() != Some("Command") {
                    continue;
                }
                let payload = &v["payload"];
                let event = payload["event"].as_str().unwrap_or("");
                let Some(ship) = payload["ship"].as_str() else {
                    continue;
                };
                let game_ts = v["game_ts"].as_str().unwrap_or("—").to_string();
                match event {
                    "take-control" => {
                        let (Some(la), Some(lo)) =
                            (payload["lat"].as_f64(), payload["lon"].as_f64())
                        else {
                            continue;
                        };
                        out.push(ReplayEvent {
                            game_ts,
                            ship: ship.to_string(),
                            lat: la,
                            lon: lo,
                            placed: true,
                        });
                    }
                    "release" => out.push(ReplayEvent {
                        game_ts,
                        ship: ship.to_string(),
                        lat: 0.0,
                        lon: 0.0,
                        placed: false,
                    }),
                    _ => {}
                }
            }
        }
        out
    }

    /// Ghost units as placed up to the slider: fold take/release events.
    fn replay_state(&self) -> Vec<(String, f64, f64)> {
        let mut map: HashMap<String, (f64, f64)> = HashMap::new();
        for e in self.log_events.iter().take(self.replay_pos) {
            if e.placed {
                map.insert(e.ship.clone(), (e.lat, e.lon));
            } else {
                map.remove(&e.ship);
            }
        }
        let mut out: Vec<(String, f64, f64)> =
            map.into_iter().map(|(s, (la, lo))| (s, la, lo)).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// End action: disarm into Closed, lock working islands, and freeze
    /// the transcript tail.
    fn end_session(&mut self) {
        self.mode.end();
        self.close_working_islands();
        // Closed clears selection: the map freezes under its transcript,
        // clicks go inert, the Inspector shuts.
        self.deselect();
        if let Ok(text) = std::fs::read_to_string(&self.session_log_path) {
            let lines: Vec<String> =
                text.lines().map(|s| s.to_string()).collect();
            let n = lines.len();
            self.transcript =
                lines.into_iter().skip(n.saturating_sub(200)).collect();
        }
        eprintln!("session ended");
    }

    /// Session island: phase, engine toggle, and the per-phase controls.
    /// Reads UiMode; writes only through start()/end()/reset().
    fn session_island(&mut self, ui: &mut egui::Ui) {
        let phase = self.mode.phase;
        ui.label(format!(
            "phase: {phase:?}{}",
            if self.mode.armed.load(Ordering::SeqCst) { " · armed" } else { " · presentation" }
        ));
        if phase != Phase::Closed {
            let mut armed = self.mode.armed.load(Ordering::SeqCst);
            if ui.checkbox(&mut armed, "simulation mode (engine)").changed() {
                self.mode.armed.store(armed, Ordering::SeqCst);
                eprintln!("{}", if armed { "sim armed" } else { "presentation only" });
            }
        }
        match phase {
            Phase::Setup => {
                // Setup editing is organizer-only (slice iv): acting
                // players keep the read-only summary.
                if self.acting_as.is_some() {
                    ui.label("Setup is organizer-only: switch identity to Organizer to edit.");
                    ui.label(format!("placed: {} unit(s) · {} player(s)", self.placed_fleet.len(), self.roster.len()));
                    return;
                }
                ui.heading("Session logs");
                if let Some(path) = self.log_view_path.clone() {
                    let (entries, units, players) = Self::read_log_view(&path);
                    ui.horizontal(|ui| {
                        if ui.small_button("← all logs").clicked() {
                            self.log_view_path = None;
                        }
                        ui.label(
                            path.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("?"),
                        );
                    });
                    ui.label(format!(
                        "units: {} · players: {}",
                        if units.is_empty() { "—".to_string() } else { units.join(", ") },
                        if players.is_empty() { "—".to_string() } else { players.join(", ") },
                    ));
                    egui::ComboBox::from_label("show")
                        .selected_text(&self.log_filter)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.log_filter,
                                "all".to_string(),
                                "all events",
                            );
                            for u in &units {
                                ui.selectable_value(
                                    &mut self.log_filter,
                                    format!("unit:{u}"),
                                    format!("unit {u}"),
                                );
                            }
                            for p in &players {
                                ui.selectable_value(
                                    &mut self.log_filter,
                                    format!("player:{p}"),
                                    format!("player {p}"),
                                );
                            }
                        });
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.show_replay, "show on map");
                        let max = self.log_events.len();
                        if self.replay_pos > max {
                            self.replay_pos = max;
                        }
                        ui.add(egui::Slider::new(&mut self.replay_pos, 0..=max).text("replay"));
                    });
                    if let Some(e) = self.log_events.get(self.replay_pos.saturating_sub(1)) {
                        ui.label(format!("replay @ {}", e.game_ts));
                    } else {
                        ui.label("replay @ start");
                    }
                    let filter = self.log_filter.clone();
                    egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                        for e in entries.iter().rev().take(200) {
                            let show = if filter == "all" {
                                true
                            } else if let Some(u) = filter.strip_prefix("unit:") {
                                e.ships.iter().any(|s| s == u)
                            } else if let Some(p) = filter.strip_prefix("player:") {
                                e.actor == p
                            } else {
                                true
                            };
                            if show {
                                ui.label(&e.text);
                            }
                        }
                    });
                } else {
                    let files = Self::session_log_files();
                    if files.is_empty() {
                        ui.label("no past sessions yet");
                    }
                    for f in files {
                        let name = f
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("?")
                            .to_string();
                        if ui.small_button(&name).clicked() {
                            self.log_events = Self::parse_replay(&f);
                            self.replay_pos = self.log_events.len();
                            self.log_filter = "all".to_string();
                            self.log_view_path = Some(f);
                        }
                    }
                }
                ui.separator();
                ui.heading("Time windows (WIB)");
                ui.horizontal(|ui| {
                    ui.label("real:");
                    ui.text_edit_singleline(&mut self.time_real_start);
                    ui.label("→");
                    ui.text_edit_singleline(&mut self.time_real_end);
                });
                ui.horizontal(|ui| {
                    ui.label("game:");
                    ui.text_edit_singleline(&mut self.time_game_start);
                    ui.label("→");
                    ui.text_edit_singleline(&mut self.time_game_end);
                });
                let windows = parse_windows(
                    &self.time_real_start,
                    &self.time_real_end,
                    &self.time_game_start,
                    &self.time_game_end,
                );
                let windows_ok = windows.is_some();
                match windows {
                    Some((rs, re, gs, ge)) => {
                        let ratio = (ge - gs).num_seconds() as f64 / (re - rs).num_seconds() as f64;
                        ui.label(format!("ratio {ratio:.1}x · stored {}", rs.format("%Y-%m-%d %H:%M UTC")));
                    }
                    None => {
                        ui.label("Windows must be YYYY-MM-DD HH:MM with end after start.");
                    }
                }
                ui.separator();
                ui.heading("Players");
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.roster_input);
                    if ui.small_button("add").clicked() {
                        let name = self.roster_input.trim().to_string();
                        if !name.is_empty() && !self.roster.contains(&name) {
                            self.roster.push(name);
                            self.roster_input.clear();
                        }
                    }
                });
                // Pre-collect: rows mutate seat maps while the roster
                // borrow would still be live (E0502 pattern).
                let roster: Vec<String> = self.roster.clone();
                for name in &roster {
                    ui.horizontal(|ui| {
                        ui.label(name);
                        if ui.small_button("remove").clicked() {
                            self.roster.retain(|n| n != name);
                            self.helm.retain(|_, v| v != name);
                            self.unit_commander.retain(|_, v| v != name);
                        }
                    });
                }
                ui.separator();
                ui.heading("Seats");
                let mut units: Vec<(String, String)> = self
                    .placed_fleet
                    .iter()
                    .map(|id| {
                        let label = self
                            .fleet
                            .get(id)
                            .map(|u| format!("{} ({})", u.name, u.hull))
                            .unwrap_or_else(|| id.clone());
                        (id.clone(), label)
                    })
                    .collect();
                units.sort();
                for (uid, label) in &units {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        let helm_cur = self.helm.get(uid).cloned();
                        egui::ComboBox::from_id_salt(format!("helm-{uid}"))
                            .selected_text(helm_cur.as_deref().unwrap_or("helm: —"))
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(helm_cur.is_none(), "—").clicked() {
                                    self.helm.remove(uid);
                                }
                                for name in &roster {
                                    if ui
                                        .selectable_label(helm_cur.as_deref() == Some(name.as_str()), name)
                                        .clicked()
                                    {
                                        self.helm.insert(uid.clone(), name.clone());
                                    }
                                }
                            });
                        let cmdr_cur = self.unit_commander.get(uid).cloned();
                        egui::ComboBox::from_id_salt(format!("cmdr-{uid}"))
                            .selected_text(cmdr_cur.as_deref().unwrap_or("commander: —"))
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(cmdr_cur.is_none(), "—").clicked() {
                                    self.unit_commander.remove(uid);
                                }
                                for name in &roster {
                                    if ui
                                        .selectable_label(cmdr_cur.as_deref() == Some(name.as_str()), name)
                                        .clicked()
                                    {
                                        self.unit_commander.insert(uid.clone(), name.clone());
                                    }
                                }
                            });
                    });
                }
                ui.separator();
                ui.separator();
                ui.heading("Invites");
                ui.label("Codes bind players to seats. Local records rule; the mock mirrors when connected.");
                for name in &roster {
                    ui.horizontal(|ui| {
                        let seats = self.seat_labels(name);
                        ui.label(format!(
                            "{} — {}",
                            name,
                            if seats.is_empty() { "no seat".to_string() } else { seats.join(", ") }
                        ));
                        if let Some(code) = self
                            .invites
                            .iter()
                            .find(|i| i.user == *name && !i.redeemed)
                            .map(|i| i.code.clone())
                        {
                            ui.label(format!("code: {code}"));
                            if ui.small_button("redeem").clicked() {
                                if let Some(inv) = self.invites.iter_mut().find(|i| i.code == code) {
                                    inv.redeemed = true;
                                }
                                if let Some(base) = Self::backend_base() {
                                    match InviteClient::new(&base).and_then(|c| c.redeem(&code)) {
                                        Ok(_) => {
                                            self.invite_status = format!("redeemed {code} (mock mirrored)");
                                        }
                                        Err(e) => {
                                            self.invite_status = format!("redeemed {code} locally; mock: {e}");
                                        }
                                    }
                                } else {
                                    self.invite_status = format!("redeemed {code} locally");
                                }
                            }
                        } else if ui.small_button("issue code").clicked() {
                            let code = format!("TFG-{:04}", self.invite_seq);
                            self.invite_seq += 1;
                            let seat = seats.join(", ");
                            self.invites.push(Invite {
                                code: code.clone(),
                                user: name.clone(),
                                seat: seat.clone(),
                                redeemed: false,
                            });
                            if let Some(base) = Self::backend_base() {
                                match InviteClient::new(&base).and_then(|c| c.issue(name, &seat, &code)) {
                                    Ok(rec) => {
                                        self.invite_status = format!("issued {} (mock mirrored)", rec.code);
                                    }
                                    Err(e) => {
                                        self.invite_status = format!("issued {code} locally; mock: {e}");
                                    }
                                }
                            } else {
                                self.invite_status = format!("issued {code} locally (no backend)");
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if ui.small_button("sync to mock").clicked() {
                        self.sync_invites();
                    }
                    if ui.small_button("refresh from mock").clicked() {
                        self.refresh_invites();
                    }
                    ui.label(&self.invite_status);
                });
                ui.separator();
                // Warn-not-block go-live (grill #23): gaps are listed,
                // only unparseable windows refuse to start. Unhelmed units
                // stay playable: the organizer commands all.
                let mut warnings = Vec::new();
                if units.is_empty() {
                    warnings.push("no units placed".to_string());
                }
                if roster.is_empty() {
                    warnings.push("roster is empty".to_string());
                }
                for (uid, label) in &units {
                    if !self.helm.contains_key(uid) {
                        warnings.push(format!("{label}: no helm (organizer retains command)"));
                    }
                }
                for w in &warnings {
                    ui.label(egui::RichText::new(format!("⚠ {w}")).color(egui::Color32::YELLOW));
                }
                ui.horizontal(|ui| {
                    if ui.small_button("open fleet picker").clicked() {
                        self.show_fleet = true;
                    }
                    if ui.small_button("open groups").clicked() {
                        self.show_groups = true;
                    }
                    ui.label(format!("placed: {} unit(s) · {} player(s)", units.len(), roster.len()));
                });
                if ui
                    .add_enabled(windows_ok, egui::Button::new("start session (prototype)"))
                    .clicked()
                {
                    self.start_session();
                }
            }
            Phase::Live => {
                if let Some((rs, re, gs, ge)) = &self.session_windows {
                    ui.label(format!("real {rs} → {re}"));
                    ui.label(format!("game {gs} → {ge} ({:.1}x)", self.session_ratio));
                }
                if ui.small_button("end session").clicked() {
                    self.end_session();
                }
            }
            Phase::Closed => {
                ui.label(format!("log: {}", self.session_log_path.display()));
                if ui.small_button("new setup").clicked() {
                    self.mode.reset();
                    self.close_working_islands();
                    self.selection = None;
                    self.placed_fleet.clear();
                    self.fleet_pick = None;
                    self.helm.clear();
                    self.unit_commander.clear();
                    self.groups = Groups::default();
                    self.group_seq = 1;
                    self.satgas_members.clear();
                    self.gugus_members.clear();
                    self.group_error = None;
                    self.invites.clear();
                    self.invite_seq = 1;
                    self.invite_status = "local records".to_string();
                    eprintln!("back to setup");
                }
            }
        }
    }

    /// Zone + flag geometry for this frame (slice iii, grill #24): live
    /// hulls over member markers in the per-level palette, single-unit
    /// circles, centroid flags carrying lat/lon. Empty groups draw nothing.
    fn group_geometry(&self, markers: &[ShipMarker]) -> (Vec<ZoneGeom>, Vec<FlagGeom>) {
        // Gugus first so Satgas zones paint over them.
        let mut work: Vec<(Vec<String>, String, String, egui::Color32, egui::Color32)> = Vec::new();
        for g in self.groups.gugus_list() {
            work.push((
                self.groups.gugus_units(&g.id),
                g.id.clone(),
                g.name.clone(),
                egui::Color32::from_rgba_unmultiplied(0x93, 0x33, 0xea, 70),
                egui::Color32::from_rgb(0x93, 0x33, 0xea),
            ));
        }
        for s in self.groups.satgas_list() {
            work.push((
                s.units.clone(),
                s.id.clone(),
                s.name.clone(),
                egui::Color32::from_rgba_unmultiplied(0x25, 0x63, 0xeb, 70),
                egui::Color32::from_rgb(0x25, 0x63, 0xeb),
            ));
        }
        let mut zones = Vec::new();
        let mut flags = Vec::new();
        for (members, gid, name, fill, stroke) in work {
            let pts: Vec<(f64, f64)> = markers
                .iter()
                .filter(|m| !self.hidden.contains(&m.id) && members.iter().any(|u| u == &m.id))
                .map(|m| (m.x, m.y))
                .collect();
            if pts.is_empty() {
                continue;
            }
            if self.zoom < ZONE_ZOOM {
                let n = pts.len();
                let (cx, cy) = (
                    pts.iter().map(|p| p.0).sum::<f64>() / n as f64,
                    pts.iter().map(|p| p.1).sum::<f64>() / n as f64,
                );
                let mut lat_sum = 0.0;
                let mut lon_sum = 0.0;
                let mut count = 0usize;
                for m in markers.iter().filter(|m| members.iter().any(|u| u == &m.id)) {
                    if let Some(s) = self.registry.ships().iter().find(|s| s.ship_id == m.id) {
                        lat_sum += s.latest.position.latitude;
                        lon_sum += s.latest.position.longitude;
                        count += 1;
                    }
                }
                if count == 0 {
                    continue;
                }
                flags.push(FlagGeom {
                    x: cx as f32,
                    y: cy as f32,
                    lat: lat_sum / count as f64,
                    lon: lon_sum / count as f64,
                    label: format!("{name} ({n})"),
                    group: gid,
                    color: stroke,
                });
                continue;
            }
            let hull = pad_hull(&convex_hull(pts), ZONE_PAD_PX);
            zones.push(ZoneGeom {
                group: gid,
                pts: hull.into_iter().map(|(x, y)| (x as f32, y as f32)).collect(),
                fill,
                stroke,
            });
        }
        (zones, flags)
    }

    /// Groups island (slice iii): the organizer builds Satgas (units +
    /// commander) and Gugus (satgas + commander) from the roster. Editing
    /// runs in Setup and Live (reorganization mid-session); Closed is read-only.
    fn groups_island(&mut self, ui: &mut egui::Ui) {
        ui.heading("Groups");
        ui.label("Satgas of units, Gugus of Satgas. Commanders from the roster.");
        let editable =
            (self.mode.phase == Phase::Setup || self.mode.phase == Phase::Live) && self.acting_as.is_none();
        let roster: Vec<String> = self.roster.clone();
        if editable {
            ui.separator();
            ui.heading("New Satgas");
            ui.horizontal(|ui| {
                ui.label("name:");
                ui.text_edit_singleline(&mut self.satgas_name);
            });
            let cmdr = self.satgas_commander.clone();
            ui.horizontal(|ui| {
                ui.label("commander:");
                egui::ComboBox::from_id_salt("satgas-commander")
                    .selected_text(cmdr.as_deref().unwrap_or("—"))
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(cmdr.is_none(), "—").clicked() {
                            self.satgas_commander = None;
                        }
                        for name in &roster {
                            if ui.selectable_label(cmdr.as_deref() == Some(name.as_str()), name).clicked() {
                                self.satgas_commander = Some(name.clone());
                            }
                        }
                    });
            });
            let mut placed: Vec<(String, String)> = self
                .placed_fleet
                .iter()
                .map(|id| {
                    let label = self
                        .fleet
                        .get(id)
                        .map(|u| format!("{} ({})", u.name, u.hull))
                        .unwrap_or_else(|| id.clone());
                    (id.clone(), label)
                })
                .collect();
            placed.sort();
            for (uid, label) in &placed {
                let owner = self.groups.satgas_of_unit(uid).map(|s| s.name.clone());
                let mut member = self.satgas_members.contains(uid);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut member, "").changed() {
                        if member {
                            self.satgas_members.insert(uid.clone());
                        } else {
                            self.satgas_members.remove(uid);
                        }
                    }
                    match owner {
                        Some(o) => {
                            ui.label(format!("{label} (in {o})"));
                        }
                        None => {
                            ui.label(label);
                        }
                    }
                });
            }
            if ui.small_button("create satgas").clicked() {
                let id = format!("satgas-{}", self.group_seq);
                let members: Vec<String> = self.satgas_members.iter().cloned().collect();
                match self.groups.add_satgas(
                    id.clone(),
                    self.satgas_name.trim().to_string(),
                    members,
                    cmdr,
                ) {
                    Ok(()) => {
                        self.group_seq += 1;
                        self.satgas_name.clear();
                        self.satgas_commander = None;
                        self.satgas_members.clear();
                        self.group_error = None;
                        eprintln!("created satgas {id}");
                    }
                    Err(e) => {
                        self.group_error = Some(e.clone());
                        self.feed(format!("satgas rejected: {e}"));
                    }
                }
            }
        }
        let satgas: Vec<(String, String, Option<String>, usize)> = self
            .groups
            .satgas_list()
            .iter()
            .map(|s| (s.id.clone(), s.name.clone(), s.commander.clone(), s.units.len()))
            .collect();
        for (id, name, commander, n) in &satgas {
            ui.horizontal(|ui| {
                let mark = if self.selection == Some(Selection::Group(id.clone())) { " ●" } else { "" };
                ui.label(format!("{} — {} · {} unit(s){mark}", name, commander.as_deref().unwrap_or("no commander"), n));
                if ui.small_button("select").clicked() {
                    self.select_group(id.clone());
                    self.show_orders = true;
                }
                if editable && ui.small_button("remove").clicked() {
                    self.groups.remove_satgas(id);
                }
            });
        }
        if editable {
            ui.separator();
            ui.heading("New Gugus");
            ui.horizontal(|ui| {
                ui.label("name:");
                ui.text_edit_singleline(&mut self.gugus_name);
            });
            let cmdr = self.gugus_commander.clone();
            ui.horizontal(|ui| {
                ui.label("commander:");
                egui::ComboBox::from_id_salt("gugus-commander")
                    .selected_text(cmdr.as_deref().unwrap_or("—"))
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(cmdr.is_none(), "—").clicked() {
                            self.gugus_commander = None;
                        }
                        for name in &roster {
                            if ui.selectable_label(cmdr.as_deref() == Some(name.as_str()), name).clicked() {
                                self.gugus_commander = Some(name.clone());
                            }
                        }
                    });
            });
            for (id, name, _, n) in &satgas {
                let mut member = self.gugus_members.contains(id);
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut member, "").changed() {
                        if member {
                            self.gugus_members.insert(id.clone());
                        } else {
                            self.gugus_members.remove(id);
                        }
                    }
                    ui.label(format!("{name} · {n} unit(s)"));
                });
            }
            if ui.small_button("create gugus").clicked() {
                let id = format!("gugus-{}", self.group_seq);
                let members: Vec<String> = self.gugus_members.iter().cloned().collect();
                match self.groups.add_gugus(
                    id.clone(),
                    self.gugus_name.trim().to_string(),
                    members,
                    cmdr,
                ) {
                    Ok(()) => {
                        self.group_seq += 1;
                        self.gugus_name.clear();
                        self.gugus_commander = None;
                        self.gugus_members.clear();
                        self.group_error = None;
                        eprintln!("created gugus {id}");
                    }
                    Err(e) => {
                        self.group_error = Some(e.clone());
                        self.feed(format!("gugus rejected: {e}"));
                    }
                }
            }
        }
        let gugus: Vec<(String, String, Option<String>, usize)> = self
            .groups
            .gugus_list()
            .iter()
            .map(|g| (g.id.clone(), g.name.clone(), g.commander.clone(), self.groups.gugus_units(&g.id).len()))
            .collect();
        for (id, name, commander, n) in &gugus {
            ui.horizontal(|ui| {
                let mark = if self.selection == Some(Selection::Group(id.clone())) { " ●" } else { "" };
                ui.label(format!("{} — {} · {} unit(s){mark}", name, commander.as_deref().unwrap_or("no commander"), n));
                if ui.small_button("select").clicked() {
                    self.select_group(id.clone());
                    self.show_orders = true;
                }
                if editable && ui.small_button("remove").clicked() {
                    self.groups.remove_gugus(id);
                }
            });
        }
        if let Some(e) = self.group_error.clone() {
            ui.label(egui::RichText::new(format!("⚠ {e}")).color(egui::Color32::YELLOW));
        }
    }

    /// Fleet island (task #29): organizer hull picker. Filter the 125
    /// seeds by category, class, and type/name/hull text, pick one hull,
    /// then place it by hand with a map click. Nothing places itself.
    fn fleet_island(&mut self, ui: &mut egui::Ui) {
        const CAT_OPTS: [Option<Category>; 5] = [
            None,
            Some(Category::Ship),
            Some(Category::Plane),
            Some(Category::Tank),
            Some(Category::Port),
        ];
        const CAT_LABELS: [&str; 5] = ["All", "Ship", "Plane", "Tank", "Port"];
        ui.heading("Fleet picker");
        ui.label("Organizer: filter, pick one hull, place it on the map by hand.");
        // Collect options first: the combos mutate self while the
        // catalog borrows would still be live (E0502 pattern).
        let class_opts: Vec<(String, String)> = self
            .catalog
            .ship_classes()
            .iter()
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect();
        ui.horizontal(|ui| {
            egui::ComboBox::from_label("category")
                .selected_text(CAT_LABELS[self.fleet_cat])
                .show_ui(ui, |ui| {
                    for (i, label) in CAT_LABELS.iter().enumerate() {
                        ui.selectable_value(&mut self.fleet_cat, i, *label);
                    }
                });
            let class_label = self
                .fleet_class
                .as_ref()
                .and_then(|id| class_opts.iter().find(|(cid, _)| cid == id))
                .map(|(_, name)| name.as_str())
                .unwrap_or("All classes");
            egui::ComboBox::from_label("class")
                .selected_text(class_label)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.fleet_class, None, "All classes");
                    for (id, name) in &class_opts {
                        ui.selectable_value(&mut self.fleet_class, Some(id.clone()), name);
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("type / name / hull:");
            ui.text_edit_singleline(&mut self.fleet_query);
            if ui.small_button("clear").clicked() {
                self.fleet_query.clear();
                self.fleet_class = None;
                self.fleet_cat = 0;
            }
        });
        let query = self.fleet_query.to_lowercase();
        let rows: Vec<tfg::fleet::FleetUnit> = self
            .fleet
            .units()
            .iter()
            .filter(|u| {
                if CAT_OPTS[self.fleet_cat] != self.catalog.class(&u.class_id).map(|c| c.category) {
                    return false;
                }
                if let Some(ref cid) = self.fleet_class {
                    if &u.class_id != cid {
                        return false;
                    }
                }
                if !query.is_empty() {
                    let class_name = self
                        .catalog
                        .class(&u.class_id)
                        .map(|c| c.name.as_str())
                        .unwrap_or("");
                    let hay = format!(
                        "{} {} {} {} {} {}",
                        u.name, u.hull, u.role, class_name, u.satuan, u.pangkalan
                    )
                    .to_lowercase();
                    if !hay.contains(&query) {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect();
        ui.label(format!(
            "{} hulls · {} shown · {} placed",
            self.fleet.len(),
            rows.len(),
            self.placed_fleet.len()
        ));
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            for u in &rows {
                if self.placed_fleet.contains(&u.id) {
                    ui.label(format!("✓ {} ({}) — placed", u.name, u.hull));
                } else {
                    let class_name: String = self
                        .catalog
                        .class(&u.class_id)
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    ui.selectable_value(
                        &mut self.fleet_pick,
                        Some(u.id.clone()),
                        format!("{} ({}) · {}", u.name, u.hull, class_name),
                    );
                }
            }
        });
        let pick_placed = self.fleet_pick.as_ref().map_or(false, |id| self.placed_fleet.contains(id));
        // The sim is not polled while disarmed, so a TakeControl sent
        // with the engine off would sit in the queue invisibly. Gate
        // arming placement on the engine, with a one-click arm here.
        // Placement runs in Setup (initial fleet) and Live
        // (reinforcements); Closed is read-only.
        let armed = self.mode.armed.load(Ordering::SeqCst);
        if self.mode.phase == Phase::Closed {
            ui.label("Placement is unavailable once the session is closed.");
        } else if !armed {
            ui.label("Engine is presentation-only: placed hulls stay invisible until it runs.");
            if ui.small_button("arm engine").clicked() {
                self.mode.armed.store(true, Ordering::SeqCst);
                eprintln!("sim armed");
            }
        } else if self.acting_as.is_some() {
            ui.label("Placement is organizer-only.");
        } else if pick_placed {
            ui.label("Already placed — pick another hull.");
        } else if let Some(id) = self.fleet_pick.clone() {
            let placing = self.mode.tool == SetupTool::Place;
            let name: String = self.fleet.get(&id).map(|u| u.name.clone()).unwrap_or_default();
            if ui
                .small_button(if placing {
                    format!("click the map to place {name}…")
                } else {
                    format!("place {name}")
                })
                .clicked()
            {
                self.mode.tool = if placing { SetupTool::Select } else { SetupTool::Place };
                self.placing = false;
            }
        } else {
            ui.label("Pick a hull above to arm placement.");
        }
    }

    /// Log island: current warning plus the capped sim event feed.
    fn log_island(&mut self, ui: &mut egui::Ui) {
        if let Some(w) = &self.order_warning {
            ui.label(egui::RichText::new(format!("⚠ {w}")).color(egui::Color32::YELLOW));
        }
        egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
            for line in &self.event_feed {
                ui.monospace(line);
            }
        });
    }

    /// Wizard island (islands grill, #27): stepped Setup that dismisses
    /// on Live. Points at the working islands; never duplicates them.
    fn wizard_island(&mut self, ui: &mut egui::Ui) {
        ui.heading("Command center setup");
        ui.label("Four steps to a live game. Skip any time; the islands stay.");
        match self.wizard_step {
            0 => {
                ui.label("Welcome: command simulated ships on a live map. Setup places your fleet, Live plays it.");
                ui.horizontal(|ui| {
                    if ui.button("Begin setup →").clicked() {
                        self.wizard_step = 1;
                        self.show_session = true;
                    }
                    if ui.button("Skip tour").clicked() {
                        self.wizard_done = true;
                    }
                });
            }
            1 => {
                ui.label("Session: arm the engine, then start. Defaults play 7 hours as 7 days.");
                if self.mode.phase == Phase::Setup && ui.button("Start session").clicked() {
                    self.start_session();
                }
                ui.horizontal(|ui| {
                    if ui.button("← Back").clicked() {
                        self.wizard_step = 0;
                    }
                    if ui.button("Next →").clicked() {
                        self.wizard_step = 2;
                        self.show_session = true;
                        self.show_roster = true;
                        self.show_fleet = true;
                        self.show_groups = true;
                    }
                });
            }
            2 => {
                ui.label("Fleet: in the Fleet island, filter by category, class, or type, pick one hull, then click the map to place it by hand.");
                ui.label(format!("placed: {} unit(s)", self.placed_fleet.len()));
                ui.horizontal(|ui| {
                    if ui.button("← Back").clicked() {
                        self.wizard_step = 1;
                    }
                    if ui.button("Next →").clicked() {
                        self.wizard_step = 3;
                        self.show_roster = true;
                    }
                });
            }
            _ => {
                let placed = self.placed_fleet.len();
                ui.label(format!(
                    "Review: {placed} placed, {} owned, {} players.",
                    self.controlled.len(),
                    self.roster.len()
                ));
                ui.label("Gaps are fine: unowned units sail as traffic.");
                ui.horizontal(|ui| {
                    if ui.button("← Back").clicked() {
                        self.wizard_step = 2;
                    }
                    if self.mode.phase == Phase::Setup && ui.button("Go live").clicked() {
                        self.start_session();
                    }
                    if ui.button("Finish").clicked() {
                        self.wizard_done = true;
                    }
                });
            }
        }
    }
}

impl eframe::App for ShipApp {
    /// Ordered teardown: stop the poll source, close the map request
    /// channel so the map thread drops its scene on its OWN thread, then
    /// join both before eframe tears down GL. Lets background threads run
    /// past this point and the native map core frees out from under them
    /// (the `double free` on close).
    fn on_exit(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.map_req_tx.take();
        if let Some(h) = self.map_handle.take() {
            let _ = h.join();
        }
        if let Some(h) = self.poll_handle.take() {
            let _ = h.join();
        }
        eprintln!("shutdown: threads joined");
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_map(ui.ctx());
        // Flush a trailing seamless-zoom step (task #43): the last tick
        // inside the throttle window still gets its frame.
        if self.zoom_dirty && self.last_zoom_req.elapsed() >= Duration::from_millis(250) {
            self.zoom_dirty = false;
            self.last_zoom_req = Instant::now();
            self.refresh_map();
        }
        let markers = self.markers();
        // Group overlays (slice iii, grill #24): recomputed per frame from
        // live marker positions; zones above the zoom threshold, flags below.
        let (zones, flags) = self.group_geometry(&markers);
        ui.ctx().request_repaint_after(Duration::from_millis(100));

        // Space toggles pause: a full hold (ADR-0004). The sim enforces it
        // tick-wise; the UI just forwards the verb.
        if ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) {
            if let Some(tx) = &self.sim_cmd_tx {
                let paused = !self.game_paused;
                let _ = tx.send(SimCommand::SetPaused { paused });
                eprintln!("{}", if paused { "pause" } else { "resume" });
            }
        }
        // Number keys switch desktop tabs (grill #25); the camera stays.
        let num_keys = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        for (i, key) in num_keys.iter().enumerate() {
            if ui.ctx().input(|inp| inp.key_pressed(*key)) {
                let tabs = self.desktop_tabs();
                if let Some((id, _, _)) = tabs.get(i) {
                    let id = id.clone();
                    self.switch_desktop(id);
                }
                break;
            }
        }

        // Toolbar (islands grill, #27): island toggles + the clock block.
        // The dock is dead; every flow below is a floating island.
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                // Top-level mode (task #39): session lives in
                // Simulation; Presentation watches a backend.
                let mut mode = self.app_mode;
                ui.selectable_value(&mut mode, AppMode::Presentation, "📡 Presentation");
                ui.selectable_value(&mut mode, AppMode::Simulation, "🎮 Simulation");
                if mode != self.app_mode {
                    self.set_app_mode(mode);
                }
                ui.separator();
                ui.toggle_value(&mut self.show_connection, "Connection");
                if self.app_mode == AppMode::Presentation {
                    // No Inspector toggle: selection drives it (a marker or
                    // roster click opens it, deselect closes it).
                    ui.toggle_value(&mut self.show_log, "Log");
                }
                if self.app_mode == AppMode::Simulation {
                    ui.toggle_value(&mut self.show_session, "Session");
                    // Fleet spans Setup (initial placement) and Live
                    // (reinforcements); Groups likewise (setup + mid-session
                    // reorganization). The rest unlock once live (task #40).
                    ui.toggle_value(&mut self.show_fleet, "Fleet");
                    ui.toggle_value(&mut self.show_groups, "Groups");
                    // Working islands unlock once a session exists (task
                    // #40): pre-session the lobby is Session + Fleet + Groups + Connection.
                    if self.session_live() {
                        ui.toggle_value(&mut self.show_roster, "Roster");
                        ui.toggle_value(&mut self.show_orders, "Orders");
                        ui.toggle_value(&mut self.show_log, "Log");
                    }
                }
                ui.separator();
                if ui.small_button("−").clicked() {
                    self.zoom_by(-1.0);
                }
                ui.label(format!("z{:.0}", self.zoom));
                if ui.small_button("+").clicked() {
                    self.zoom_by(1.0);
                }
            });
            // Identity + desktops (slice iv, grill #25): simulation-only
            // (task #39) — there is nobody to act as in Presentation.
            if self.app_mode == AppMode::Simulation {
            ui.horizontal(|ui| {
                ui.label("act:");
                let roster: Vec<String> = self.roster.clone();
                let mut act_idx = match &self.acting_as {
                    None => 0,
                    Some(u) => roster.iter().position(|n| n == u).map(|i| i + 1).unwrap_or(0),
                };
                egui::ComboBox::from_label("identity")
                    .selected_text(self.acting_as.as_deref().unwrap_or("Organizer"))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut act_idx, 0, "Organizer");
                        for (i, name) in roster.iter().enumerate() {
                            ui.selectable_value(&mut act_idx, i + 1, name);
                        }
                    });
                let new_acting =
                    if act_idx == 0 { None } else { roster.get(act_idx - 1).cloned() };
                if new_acting != self.acting_as {
                    self.save_draft();
                    self.acting_as = new_acting;
                    let def = self.default_desktop();
                    self.load_draft(&def);
                    self.desktop = def;
                    if self.is_observer() {
                        self.show_orders = false;
                    }
                    eprintln!("acting as {}", self.acting_as.as_deref().unwrap_or("organizer"));
                }
                ui.separator();
                let tabs = self.desktop_tabs();
                for (i, (id, label, n)) in tabs.iter().enumerate() {
                    let mut tab = format!("{label} ({n})");
                    if i < 9 {
                        tab = format!("[{}] {tab}", i + 1);
                    }
                    if ui.selectable_label(&self.desktop == id, tab).clicked() {
                        self.switch_desktop(id.clone());
                    }
                }
            });
            }
            // Clock block: real + derived game time, humane format
            // (grill #17, ADR-0004). Both readings come from the sim.
            ui.horizontal(|ui| {
                ui.label(format!("UTC {}", self.real_ts.as_deref().unwrap_or("—")));
                if self.game_paused {
                    ui.label(
                        egui::RichText::new("PAUSED")
                            .strong()
                            .color(egui::Color32::YELLOW),
                    );
                }
            });
            ui.horizontal(|ui| {
                let elapsed = self.game_elapsed_secs.unwrap_or(0);
                let g = self.game_ts.as_deref().unwrap_or("—");
                ui.label(format!(
                    "GAME {g} · G+{:02}:{:02} ({:.0}×)",
                    elapsed / 60,
                    elapsed % 60,
                    self.game_ratio
                ));
            });
        });
        // Wizard: stepped Setup, dismissed on Live or skip.
        if self.mode.phase == Phase::Setup && !self.wizard_done {
            let mut wiz_open = true;
            egui::Window::new("Command center setup")
                .default_pos(egui::pos2(330.0, 140.0))
                .movable(true)
                .collapsible(false)
                .open(&mut wiz_open)
                .show(ui.ctx(), |ui| {
                    egui::ScrollArea::vertical().max_height(ISLAND_SCROLL_MAX).show(ui, |ui| {
                        self.wizard_island(ui);
                    });
                });
            if !wiz_open {
                self.wizard_done = true;
            }
        }
        if self.show_session {
            let mut open = self.show_session;
            egui::Window::new("Session").movable(true).default_pos(egui::pos2(8.0, 64.0)).open(&mut open).show(ui.ctx(), |ui| {
                egui::ScrollArea::vertical().max_height(ISLAND_SCROLL_MAX).show(ui, |ui| {
                self.session_island(ui);
                });
            });
            self.show_session = open;
        }
        let mut follow_req: Option<(String, (f64, f64))> = None;
        if self.show_roster && self.session_live() {
            let mut open = self.show_roster;
            egui::Window::new("Roster").movable(true).default_pos(egui::pos2(816.0, 64.0)).open(&mut open).show(ui.ctx(), |ui| {
            ui.label(format!("{} ships — click a name to follow", markers.len()));
            ui.separator();
            // State machine: trails belong to Live (nothing moves in
            // Setup; Closed is frozen under its transcript).
            if self.mode.in_live() {
                ui.checkbox(&mut self.show_trail, "trails");
            }
            ui.separator();
            // Scrollbar ticket: the marker list is unbounded, so it gets
            // its own cap (300px, same as the Fleet hull list) while the
            // header, trails toggle, and unfollow stay pinned outside it.
            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            for m in &markers {
                ui.horizontal(|ui| {
                    let mut shown = !self.hidden.contains(&m.id);
                    if ui.checkbox(&mut shown, "").changed() {
                        if shown {
                            self.hidden.remove(&m.id);
                        } else {
                            self.hidden.insert(m.id.clone());
                        }
                        eprintln!("{}", if shown { "show " } else { "hide " }.to_string() + &m.id);
                    }
                    let mut label = m.id.clone();
                    if Some(&m.id) == self.following.as_ref() {
                        label += " (following)";
                    }
                    if m.stale {
                        label += " (stale)";
                    }
                    if m.source == FixSource::Sim {
                        label += " (sim)";
                    }
                    if ui.selectable_value(&mut self.following, Some(m.id.clone()), label).clicked()
                    {
                        eprintln!("follow {:?}", self.following);
                        // Roster click is a select like a marker click.
                        self.select_ship(m.id.clone());
                        if self.following.as_deref() == Some(&m.id) {
                            if let Some(s) =
                                self.registry.ships().iter().find(|s| s.ship_id == m.id)
                            {
                                follow_req = Some((
                                    m.id.clone(),
                                    (s.latest.position.latitude, s.latest.position.longitude),
                                ));
                            }
                        }
                    }
                });
            }
            });
            if ui.small_button("unfollow").clicked() {
                self.following = None;
            }
            ui.separator();
            });
            self.show_roster = open;
        }
        if self.show_fleet && self.mode.phase != Phase::Closed {
            let mut open = self.show_fleet;
            egui::Window::new("Fleet").movable(true).default_pos(egui::pos2(8.0, 300.0)).open(&mut open).show(ui.ctx(), |ui| {
                egui::ScrollArea::vertical().max_height(ISLAND_SCROLL_MAX).show(ui, |ui| {
                self.fleet_island(ui);
                });
            });
            self.show_fleet = open;
        }
        if self.show_groups && self.mode.phase != Phase::Closed {
            let mut open = self.show_groups;
            egui::Window::new("Groups").movable(true).default_pos(egui::pos2(240.0, 64.0)).open(&mut open).show(ui.ctx(), |ui| {
                egui::ScrollArea::vertical().max_height(ISLAND_SCROLL_MAX).show(ui, |ui| {
                self.groups_island(ui);
                });
            });
            self.show_groups = open;
        }
        // Inspector: selection-driven (Inspector-model ticket). The window
        // exists iff a selection exists; closing it deselects. Ships get
        // the live readout, groups get members/commander/authority plus
        // command + focus actions.
        if self.selection.is_some()
            && (self.session_live() || self.app_mode == AppMode::Presentation)
        {
            let mut open = true;
            egui::Window::new("Inspector").movable(true).default_pos(egui::pos2(816.0, 300.0)).open(&mut open).show(ui.ctx(), |ui| {
            ui.heading("Inspector");
            let mut follow_selected: Option<(String, (f64, f64))> = None;
            let mut focus_group: Option<(String, (f64, f64))> = None;
            let mut drill_ship: Option<String> = None;
            let mut drill_group: Option<String> = None;
            let mut deselect = false;
            match self.selection.clone() {
                Some(Selection::Ship(id)) => {
            match self.registry.ships().iter().find(|s| s.ship_id == id).map(|s| (s.latest.clone(), s.stale, s.trail.len())) {
                Some((fix, stale, trail_len)) => {
                    let sim_badge = if fix.source == FixSource::Sim { " (sim)" } else { "" };
                    ui.label(format!("ship: {}{sim_badge}{}", self.unit_label(&id), if stale { " (stale)" } else { "" }));
                    ui.label(format!(
                        "pos: {:.6} {:.6}",
                        fix.position.latitude, fix.position.longitude
                    ));
                    ui.label(format!(
                        "hdg/spd: {} / {}",
                        fix.heading_deg.map(|h| format!("{h:.0}°")).as_deref().unwrap_or("—"),
                        fix.speed_kn.map(|s| format!("{s:.0} kn")).as_deref().unwrap_or("—")
                    ));
                    let age = self.last_seen.get(&id).map(|t| t.elapsed().as_secs()).unwrap_or(999);
                    let n = self.fix_count.get(&id).copied().unwrap_or(0);
                    ui.label(format!("last update: {age}s ago · {n} fixes · trail {trail_len}"));
                    ui.label(format!("wire ts: {}", fix.ts));
                    ui.horizontal(|ui| {
                        if ui.small_button("follow").clicked() {
                            follow_selected = Some((
                                id.clone(),
                                (fix.position.latitude, fix.position.longitude),
                            ));
                        }
                        if ui.small_button("close").clicked() {
                            deselect = true;
                        }
                    });
                }
                None => {
                    ui.label("ship out of view — deselect and pick again");
                    if ui.small_button("clear").clicked() {
                        deselect = true;
                    }
                }
            }
                }
                Some(Selection::Group(gid)) => {
                    match self.group_info(&gid) {
                        Some((name, members)) => {
                            let commander = self.groups.satgas_list().iter().find(|s| s.id == gid).and_then(|s| s.commander.clone()).or_else(|| self.groups.gugus_list().iter().find(|g| g.id == gid).and_then(|g| g.commander.clone()));
                            let allowed: Vec<String> = members.iter().filter(|u| self.action_allows(u)).cloned().collect();
                            let auth = self.command_authority(&allowed).map(Self::authority_label).unwrap_or("view only");
                            ui.label(format!("group: {name} · {} unit(s)", members.len()));
                            ui.label(format!(
                                "commander: {} · you hold: {auth}",
                                commander.as_deref().unwrap_or("—")
                            ));
                            // Gugus lists member Satgas, not a flat unit dump.
                            let satgas_ids: Vec<String> = self
                                .groups
                                .gugus_list()
                                .iter()
                                .find(|g| g.id == gid)
                                .map(|g| g.satgas.clone())
                                .unwrap_or_default();
                            if satgas_ids.is_empty() {
                                for u in &members {
                                    ui.horizontal(|ui| {
                                        ui.label(self.unit_label(u));
                                        if ui.small_button("inspect").clicked() {
                                            drill_ship = Some(u.clone());
                                        }
                                    });
                                }
                            } else {
                                for sid in &satgas_ids {
                                    if let Some(s) = self.groups.satgas_list().iter().find(|s| &s.id == sid) {
                                        ui.horizontal(|ui| {
                                            ui.label(format!("{} · {} unit(s)", s.name, s.units.len()));
                                            if ui.small_button("select").clicked() {
                                                drill_group = Some(s.id.clone());
                                            }
                                        });
                                        for u in &s.units {
                                            ui.horizontal(|ui| {
                                                ui.label(format!("  {}", self.unit_label(u)));
                                                if ui.small_button("inspect").clicked() {
                                                    drill_ship = Some(u.clone());
                                                }
                                            });
                                        }
                                    }
                                }
                            }
                            ui.horizontal(|ui| {
                                if ui.small_button("command").clicked() {
                                    self.show_orders = true;
                                }
                                if ui.small_button("focus").clicked() {
                                    let mut lat_sum = 0.0;
                                    let mut lon_sum = 0.0;
                                    let mut count = 0usize;
                                    for m in &members {
                                        if let Some(s) = self.registry.ships().iter().find(|s| &s.ship_id == m) {
                                            lat_sum += s.latest.position.latitude;
                                            lon_sum += s.latest.position.longitude;
                                            count += 1;
                                        }
                                    }
                                    if count > 0 {
                                        focus_group = Some((
                                            gid.clone(),
                                            (lat_sum / count as f64, lon_sum / count as f64),
                                        ));
                                    }
                                }
                                if ui.small_button("close").clicked() {
                                    deselect = true;
                                }
                            });
                        }
                        None => {
                            ui.label("group removed.");
                            if ui.small_button("clear").clicked() {
                                deselect = true;
                            }
                        }
                    }
                }
                None => {}
            }
            if deselect {
                self.deselect();
            }
            if let Some(id) = drill_ship {
                self.select_ship(id);
            }
            if let Some(gid) = drill_group {
                self.select_group(gid);
            }
            if let Some((ship, at)) = follow_selected {
                self.following = Some(ship.clone());
                self.request_frame(&ship, at);
            }
            if let Some((gid, at)) = focus_group {
                self.center = at;
                self.zoom = self.zoom.max(ZONE_ZOOM);
                self.request_frame(&gid, at);
            }
            ui.separator();
            });
            if !open {
                self.deselect();
            }
        }
        // Orders island: Live-only; observers get no orders pane at all.
        if self.show_orders && self.mode.live() && !self.is_observer() {
            let mut open = self.show_orders;
            egui::Window::new("Orders").movable(true).default_pos(egui::pos2(576.0, 64.0)).open(&mut open).show(ui.ctx(), |ui| {
            // Orders (prototype sim loop): take control, place waypoint,
            // commit speed order; sim advances the ship, inspector shows it.
            ui.heading("Orders");
            // Group command: a zone/flag click (or the Groups island)
            // selects the group; whoever holds authority fans one
            // waypoint out to every unit in jurisdiction. The waypoint
            // draft is shared with single-ship orders.
            if let Some(Selection::Group(gid)) = self.selection.clone() {
                match self.group_info(&gid) {
                    Some((name, members)) => {
                        let allowed: Vec<String> = members
                            .iter()
                            .filter(|u| self.action_allows(u))
                            .cloned()
                            .collect();
                        let auth = self.command_authority(&allowed);
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label(format!("group: {name} · {} unit(s)", members.len()));
                            if ui.small_button("✕").clicked() {
                                self.deselect();
                            }
                        });
                        match auth {
                            Some(a) if !allowed.is_empty() => {
                                ui.label(format!(
                                    "authority: {} · {} in jurisdiction",
                                    Self::authority_label(a),
                                    allowed.len()
                                ));
                                if ui.small_button(if self.placing { "click map…" } else { "place waypoint" }).clicked() {
                                    self.placing = !self.placing;
                                }
                                let can_commit = self
                                    .pending_waypoint
                                    .is_some_and(|(la, lo)| {
                                        self.land
                                            .as_ref()
                                            .map(|l| {
                                                l.is_water(&GeoPosition {
                                                    latitude: la,
                                                    longitude: lo,
                                                })
                                            })
                                            .unwrap_or(true)
                                    });
                                if self.pending_waypoint.is_some() && !can_commit {
                                    ui.label(
                                        egui::RichText::new("⚠ waypoint on land")
                                            .color(egui::Color32::YELLOW),
                                    );
                                }
                                if ui
                                    .add_enabled(
                                        can_commit,
                                        egui::Button::new(format!("order group ({})", allowed.len())),
                                    )
                                    .clicked()
                                {
                                    if let Some((la, lo)) = self.pending_waypoint {
                                        if let Some(tx) = &self.sim_cmd_tx {
                                            let waypoint = GeoPosition {
                                                latitude: la,
                                                longitude: lo,
                                            };
                                            let legs: Vec<Leg> = allowed
                                                .iter()
                                                .map(|ship| {
                                                    let max = self
                                                        .order_views
                                                        .get(ship)
                                                        .map(|v| v.max_speed_kn)
                                                        .unwrap_or(self.order_speed);
                                                    Leg {
                                                        ship_id: ship.clone(),
                                                        waypoint,
                                                        speed_kn: self.order_speed.min(max),
                                                    }
                                                })
                                                .collect();
                                            let _ = tx.send(SimCommand::OrderMove {
                                                command: MoveCommand {
                                                    legs,
                                                    default_speed_kn: Some(self.order_speed),
                                                    authority: a,
                                                    grant: Grant {
                                                        units: allowed.clone(),
                                                        expires_game_secs: u64::MAX,
                                                        verbs: vec![Verb::Move],
                                                    },
                                                },
                                            });
                                            eprintln!(
                                                "order group {name} -> ({la:.4}, {lo:.4})"
                                            );
                                        }
                                        self.pending_waypoint = None;
                                        self.placing = false;
                                    }
                                }
                            }
                            _ => {
                                ui.label("Outside your jurisdiction — view only.");
                            }
                        }
                        ui.separator();
                    }
                    None => {
                        ui.label("group removed.");
                        if ui.small_button("clear").clicked() {
                            self.deselect();
                        }
                    }
                }
            }
            if let Some(Selection::Ship(id)) = self.selection.clone() {
                // Scoped desktop (slice iv): command inside jurisdiction,
                // view everything.
                let allowed = self.action_allows(&id);
                if !allowed {
                    ui.label("Outside your jurisdiction — view only.");
                }
                if self.controlled.contains(&id) {
                    if let Some(v) = self.order_views.get(&id) {
                        let state = match v.state {
                            OrderState::EnRoute => "en route",
                            OrderState::Blocked => "BLOCKED (land)",
                            OrderState::Arrived => "arrived",
                            OrderState::Holding => "holding",
                        };
                        ui.label(format!("order: {state}"));
                        if let Some(eta) = v.eta_secs {
                            // Game seconds (ADR-0004): motion covers the
                            // distance over game time, so ETA quotes game time.
                            ui.label(format!("eta: G+{}:{:02}", eta / 60, eta % 60));
                        }
                    }
                    // Taxonomy readout (grill #18) + class-capped speed.
                    if let Some((class_id, type_label, max_speed)) = self
                        .order_views
                        .get(&id)
                        .map(|v| (v.class_id.clone(), v.type_label.clone(), v.max_speed_kn))
                    {
                        ui.label(format!(
                            "{} · {} · max {:.0} kn",
                            class_id, type_label, max_speed
                        ));
                        self.order_speed = self.order_speed.min(max_speed);
                        ui.add(
                            egui::DragValue::new(&mut self.order_speed)
                                .speed(1.0)
                                .range(0.0..=max_speed as f64)
                                .suffix(" kn"),
                        );
                    } else {
                        ui.label("waiting for sim…");
                        ui.add(
                            egui::DragValue::new(&mut self.order_speed)
                                .speed(1.0)
                                .suffix(" kn"),
                        );
                    }
                    if ui.small_button(if self.placing { "click map…" } else { "place waypoint" }).clicked() {
                        self.placing = !self.placing;
                    }
                    let can_commit = self
                        .pending_waypoint
                        .is_some_and(|(la, lo)| {
                            self.land
                                .as_ref()
                                .map(|l| l.is_water(&GeoPosition { latitude: la, longitude: lo }))
                                .unwrap_or(true)
                        });
                    if self.pending_waypoint.is_some() && !can_commit {
                        ui.label(
                            egui::RichText::new("⚠ waypoint on land")
                                .color(egui::Color32::YELLOW),
                        );
                    }
                    if let Some(w) = &self.order_warning {
                        ui.label(egui::RichText::new(format!("⚠ {w}")).color(egui::Color32::YELLOW));
                    }
                    if ui.add_enabled(can_commit && allowed, egui::Button::new("order")).clicked() {
                        self.order_warning = None;
                        if let Some((la, lo)) = self.pending_waypoint {
                            if let Some(tx) = &self.sim_cmd_tx {
                                let _ = tx.send(SimCommand::SetOrder {
                                    ship_id: id.clone(),
                                    waypoint: GeoPosition { latitude: la, longitude: lo },
                                    speed_kn: self.order_speed,
                                });
                                eprintln!("order {id} -> ({la:.4}, {lo:.4}) @ {} kn", self.order_speed);
                            }
                            self.pending_waypoint = None;
                            self.placing = false;
                        }
                    }
                    // Group order (precedence core): one muster waypoint
                    // fanned out to every controlled ship, each leg capped
                    // by its class max. The local player acts as organizer
                    // until seats land (setup grill, #23).
                    let mut controlled: Vec<String> =
                        self.controlled.iter().cloned().collect();
                    controlled.sort();
                    // Order-all fans out inside jurisdiction only (slice iv).
                    if let Some(scope) = self.action_units() {
                        controlled.retain(|u| scope.contains(u));
                    }
                    if controlled.len() > 1
                        && ui
                            .add_enabled(
                                can_commit && allowed,
                                egui::Button::new(format!("order all ({})", controlled.len())),
                            )
                            .clicked()
                    {
                        self.order_warning = None;
                        if let Some((la, lo)) = self.pending_waypoint {
                            if let Some(tx) = &self.sim_cmd_tx {
                                let waypoint = GeoPosition { latitude: la, longitude: lo };
                                let legs: Vec<Leg> = controlled
                                    .iter()
                                    .map(|ship| {
                                        let max = self
                                            .order_views
                                            .get(ship)
                                            .map(|v| v.max_speed_kn)
                                            .unwrap_or(self.order_speed);
                                        Leg {
                                            ship_id: ship.clone(),
                                            waypoint,
                                            speed_kn: self.order_speed.min(max),
                                        }
                                    })
                                    .collect();
                                let _ = tx.send(SimCommand::OrderMove {
                                    command: MoveCommand {
                                        legs,
                                        default_speed_kn: Some(self.order_speed),
                                        authority: Authority::ORGANIZER,
                                        grant: Grant {
                                            units: controlled.clone(),
                                            expires_game_secs: u64::MAX,
                                            verbs: vec![Verb::Move],
                                        },
                                    },
                                });
                                eprintln!(
                                    "order all {} -> ({la:.4}, {lo:.4})",
                                    controlled.join(",")
                                );
                            }
                            self.pending_waypoint = None;
                            self.placing = false;
                        }
                    }
                    ui.horizontal(|ui| {
                        if allowed && ui.small_button("cancel").clicked() {
                            if let Some(tx) = &self.sim_cmd_tx {
                                let _ = tx.send(SimCommand::CancelOrder { ship_id: id.clone() });
                            }
                        }
                        if allowed && ui.small_button("release").clicked() {
                            if let Some(tx) = &self.sim_cmd_tx {
                                let _ = tx.send(SimCommand::Release { ship_id: id.clone() });
                            }
                            self.controlled.remove(&id);
                            self.order_views.remove(&id);
                            self.pending_waypoint = None;
                            self.placing = false;
                        }
                    });
                } else {
                    // Take-control with a class selector (grill #18): the
                    // chosen class's stats drive the unit from then on.
                    let ships = self.catalog.ship_classes();
                    let names: Vec<&str> = ships.iter().map(|c| c.name.as_str()).collect();
                    egui::ComboBox::from_label("")
                        .selected_text(
                            names.get(self.selected_class).copied().unwrap_or("—"),
                        )
                        .show_ui(ui, |ui| {
                            for (i, name) in names.iter().enumerate() {
                                ui.selectable_value(&mut self.selected_class, i, *name);
                            }
                        });
                    if allowed && ui.small_button("take control").clicked() {
                        if let Some(s) = self.registry.ships().iter().find(|s| s.ship_id == id) {
                            if let Some(tx) = &self.sim_cmd_tx {
                                let class_id = ships
                                    .get(self.selected_class)
                                    .map(|c| c.id.clone())
                                    .unwrap_or_default();
                                let _ = tx.send(SimCommand::TakeControl {
                                    ship_id: id.clone(),
                                    pos: s.latest.position,
                                    class_id,
                                });
                                eprintln!("take control {id}");
                            }
                            self.controlled.insert(id);
                        }
                    }
                }
            } else if self.selection.is_none() {
                ui.label("select a ship or group first");
            }
            });
            self.show_orders = open;
        }
        if self.show_connection {
            let mut open = self.show_connection;
            egui::Window::new("Connection").movable(true).default_pos(egui::pos2(8.0, 64.0)).open(&mut open).show(ui.ctx(), |ui| {
                self.connection_island(ui);
            });
            self.show_connection = open;
        }
        if self.show_log && (self.session_live() || self.app_mode == AppMode::Presentation) {
            let mut open = self.show_log;
            egui::Window::new("Log").movable(true).default_pos(egui::pos2(8.0, 478.0)).open(&mut open).show(ui.ctx(), |ui| {
                self.log_island(ui);
            });
            self.show_log = open;
        }
        if let Some((ship, at)) = follow_req {
            self.request_frame(&ship, at);
        }
        // Follow-tracking: chase the followed ship when it drifts from
        // center, rate-capped and leading (task #45). Gated on no
        // re-render in flight, so frames can't pile.
        if self.recentering.is_none() {
            if let Some(id) = self.following.clone() {
                if let Some(s) = self.registry.ships().iter().find(|s| s.ship_id == id) {
                    let ship_pos = s.latest.position;
                    let center = GeoPosition {
                        latitude: self.center.0,
                        longitude: self.center.1,
                    };
                    if should_track(center, ship_pos)
                        && self.last_track_req.elapsed()
                            >= Duration::from_millis(TRACK_MIN_INTERVAL_MS)
                    {
                        // Lead the ship: request tiles for where it is
                        // going, not where it was.
                        let at = match (s.latest.heading_deg, s.latest.speed_kn) {
                            (Some(h), Some(v)) if v > 0.5 => {
                                let p = ship_pos.dead_reckon(h, v, TRACK_LEAD_SECS);
                                (p.latitude, p.longitude)
                            }
                            _ => (ship_pos.latitude, ship_pos.longitude),
                        };
                        self.last_track_req = Instant::now();
                        self.track_frame(&id, at);
                        // H1: repaint through the chase; idle cadence
                        // resumes when the ship settles under the camera.
                        ui.ctx().request_repaint();
                    }
                }
            }
        }
        // Follow progress stays in the terminal (task #44):
        // request_frame logs "recentering on …", drain_map logs
        // "recentered". Nothing on the map.

        // Full-window canvas (task #38): the map owns every point the
        // panel offers, no margins. Renderer resize is debounced: the
        // scene rebuilds once the size settles a frame.
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let avail = ui.available_size();
            self.map_view = (avail.x.max(1.0) as f64, avail.y.max(1.0) as f64);
            let ppp = ui.ctx().pixels_per_point();
            let desired = (
                (avail.x.max(1.0) * ppp).round().max(1.0) as u32,
                (avail.y.max(1.0) * ppp).round().max(1.0) as u32,
            );
            if desired != self.map_px {
                if desired == self.last_desired_px {
                    self.map_px = desired;
                    self.refresh_map();
                }
                self.last_desired_px = desired;
            }
            if let Some(tex) = &self.map_tex {
                // Buttery canvas (smoothness pass): the camera is live but
                // tiles lag it by a throttle tick, so draw the last texture
                // translated (and scaled, across zooms) onto the current
                // view. Screen corners round-trip through world space into
                // texture texels; the fresh tile resolves underneath. Parts
                // outside the texture smear one frame — transient by design.
                let (mw, mh) = self.map_dims();
                let (tw, th) = (self.tex_px.0 as f64, self.tex_px.1 as f64);
                let center = self.center;
                let zoom = self.zoom;
                let tex_center = self.tex_center;
                let tex_zoom = self.tex_zoom;
                let to_uv = |px: f64, py: f64| {
                    let (la, lo) = unproject_mercator(px, py, center, zoom, mw, mh);
                    let (tx, ty) = project_mercator(la, lo, tex_center, tex_zoom, tw, th);
                    (tx / tw, ty / th)
                };
                let (u0, v0) = to_uv(0.0, 0.0);
                let (u1, v1) = to_uv(avail.x as f64, avail.y as f64);
                let response = ui.add(
                    egui::Image::new(tex)
                        .fit_to_exact_size(avail)
                        .uv(egui::Rect::from_min_max(
                            egui::pos2(u0 as f32, v0 as f32),
                            egui::pos2(u1 as f32, v1 as f32),
                        ))
                        // click+drag: drags pan, plain clicks keep
                        // select/place meaning (pan-zoom ticket).
                        .sense(egui::Sense::click_and_drag()),
                );
                let rect = response.rect;
                // Drag-to-pan (pan-zoom ticket): drags move the camera and
                // break follow; egui only reports clicked() when the press
                // never became a drag, so clicks keep their meaning with no
                // extra threshold. Overlays track the center live; the
                // texture follows through the 250ms throttle + flush.
                // Closed is inert (Inspector-model ticket).
                if self.mode.phase != Phase::Closed && response.dragged() {
                    if response.drag_started() && self.following.is_some() {
                        self.following = None;
                        eprintln!("follow broken by drag");
                    }
                    let delta = response.drag_delta();
                    if delta.x != 0.0 || delta.y != 0.0 {
                        let (mw, mh) = self.map_dims();
                        self.center = unproject_mercator(
                            mw / 2.0 - delta.x as f64,
                            mh / 2.0 - delta.y as f64,
                            self.center,
                            self.zoom,
                            mw,
                            mh,
                        );
                        self.zoom_dirty = true;
                        // H1: keep repainting while the gesture runs; the
                        // idle 100ms cadence resumes when it ends.
                        ui.ctx().request_repaint();
                        if self.last_zoom_req.elapsed() >= Duration::from_millis(250) {
                            self.zoom_dirty = false;
                            self.last_zoom_req = Instant::now();
                            self.refresh_map_light();
                        }
                    }
                }
                // Map click: stand up a catalog unit when placing, place
                // a pending waypoint when arming, else select nearest.
                // Armed clicks never deselect; empty water clears the
                // selection (Inspector-model ticket).
                if self.mode.phase != Phase::Closed && response.clicked() {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let px = (pos.x - rect.min.x) as f64;
                        let py = (pos.y - rect.min.y) as f64;
                        let (mw, mh) = self.map_dims();
                        if self.mode.tool == SetupTool::Place
                            && (self.mode.phase == Phase::Setup || self.mode.phase == Phase::Live)
                            && self.mode.armed.load(Ordering::SeqCst)
                            && self.acting_as.is_none()
                        {
                            let (la, lo) = unproject_mercator(
                                px, py, self.center, self.zoom, mw, mh,
                            );
                            // Fleet picker (task #29): only a picked,
                            // unplaced hull stands up. No generic or
                            // automatic placement.
                            let seed = self.fleet_pick.clone().and_then(|id| self.fleet.get(&id).cloned());
                            if let Some(seed) = seed {
                                if !self.placed_fleet.contains(&seed.id) {
                                    let id = seed.id.clone();
                                    if let Some(tx) = &self.sim_cmd_tx {
                                        let _ = tx.send(SimCommand::TakeControl {
                                            ship_id: id.clone(),
                                            pos: GeoPosition { latitude: la, longitude: lo },
                                            class_id: seed.class_id.clone(),
                                        });
                                        eprintln!("placed {} ({}) at ({la:.4}, {lo:.4})", seed.name, seed.hull);
                                    }
                                    // Placed units arrive owned (Q3): the click is
                                    // the take-control, no second step.
                                    self.controlled.insert(id.clone());
                                    self.select_ship(id.clone());
                                    self.placed_fleet.insert(id);
                                    self.fleet_pick = None;
                                }
                            }
                            self.mode.tool = SetupTool::Select;
                        } else if self.placing {
                            let (la, lo) = unproject_mercator(
                                px, py, self.center, self.zoom, mw, mh,
                            );
                            eprintln!("waypoint preview ({la:.4}, {lo:.4})");
                            self.pending_waypoint = Some((la, lo));
                        } else if let Some(flag) = flags
                            .iter()
                            .find(|f| {
                                ((f.x as f64 - px).powi(2) + (f.y as f64 - py).powi(2)).sqrt() < 16.0
                            })
                            .map(|f| (f.group.clone(), f.lat, f.lon))
                        {
                            // Click-to-expand (grill #24): center the group
                            // and zoom in to its zone; the group selection
                            // opens its Inspector and arms group command.
                            self.center = (flag.1, flag.2);
                            self.zoom = self.zoom.max(ZONE_ZOOM);
                            self.select_group(flag.0.clone());
                            self.request_frame(&flag.0, (flag.1, flag.2));
                        } else {
                            let visible: Vec<(String, f64, f64)> = markers
                                .iter()
                                .filter(|m| !self.hidden.contains(&m.id))
                                .map(|m| (m.id.clone(), m.x, m.y))
                                .collect();
                            if let Some(id) = hit_test(&visible, px, py, 12.0) {
                                eprintln!("select {id}");
                                self.select_ship(id);
                            } else if let Some(gid) = zones
                                .iter()
                                .find(|z| in_poly(px, py, &z.pts))
                                .map(|z| z.group.clone())
                            {
                                // Zone click (no ship hit): select the group.
                                eprintln!("select group {gid}");
                                self.select_group(gid);
                            } else if self.mode.tool != SetupTool::Place && !self.placing {
                                // Empty water, nothing armed: deselect (the
                                // Inspector shuts with the selection).
                                eprintln!("deselect");
                                self.deselect();
                            }
                        }
                    }
                }
                // Seamless zoom (task #43 + pan-zoom ticket): plain wheel
                // joins shift+wheel and pinch; the point under the cursor
                // stays put via anchor math. Overlays track every tick, the
                // texture is throttled (see ui() flush). Islands consume
                // their own scrolls first, so the map only sees open canvas.
                if response.hovered() {
                    let wheel: f64 = ui.input(|i| {
                        let w = i.smooth_scroll_delta().y;
                        let pinch = if i.zoom_delta() != 1.0 {
                            i.zoom_delta().ln() * 1200.0
                        } else {
                            0.0
                        };
                        (w + pinch) as f64
                    });
                    if wheel != 0.0 {
                        let old = self.zoom;
                        let new = (old + wheel * 0.005).clamp(3.0, 18.0);
                        if new != old {
                            if let Some(pos) = response.hover_pos() {
                                let (mw, mh) = self.map_dims();
                                self.center = anchor_center(
                                    (pos.x - rect.min.x) as f64,
                                    (pos.y - rect.min.y) as f64,
                                    self.center,
                                    old,
                                    new,
                                    mw,
                                    mh,
                                );
                            }
                            self.zoom = new;
                            eprintln!("zoom {new:.1}");
                            self.zoom_dirty = true;
                            // H1: repaint through the gesture.
                            ui.ctx().request_repaint();
                            if self.last_zoom_req.elapsed() >= Duration::from_millis(250) {
                                self.zoom_dirty = false;
                                self.last_zoom_req = Instant::now();
                                self.refresh_map_light();
                            }
                        }
                    }
                }
                let painter = ui.painter_at(rect);
                // Group zones under ships, flags above them (slice iii).
                for z in &zones {
                    let pts: Vec<egui::Pos2> = z
                        .pts
                        .iter()
                        .map(|(x, y)| rect.min + egui::vec2(*x, *y))
                        .collect();
                    // Selected group draws proud: white stroke, thicker.
                    let (stroke_color, stroke_w) =
                        if self.selection == Some(Selection::Group(z.group.clone())) {
                            (egui::Color32::WHITE, 4.0)
                        } else {
                            (z.stroke, 2.0)
                        };
                    if pts.len() >= 3 {
                        painter.add(egui::Shape::convex_polygon(
                            pts,
                            z.fill,
                            egui::Stroke::new(stroke_w, stroke_color),
                        ));
                    } else if pts.len() == 2 {
                        painter.line_segment([pts[0], pts[1]], egui::Stroke::new(10.0, z.fill));
                        painter.circle_filled(pts[0], 6.0, z.stroke);
                        painter.circle_filled(pts[1], 6.0, z.stroke);
                    } else if pts.len() == 1 {
                        painter.circle_filled(pts[0], 14.0, z.fill);
                        painter.circle_stroke(pts[0], 14.0, egui::Stroke::new(2.0, z.stroke));
                    }
                }
                for f in &flags {
                    let c = rect.min + egui::vec2(f.x, f.y);
                    painter.circle_filled(c, 10.0, f.color);
                    painter.circle_stroke(c, 10.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
                    painter.text(
                        c + egui::vec2(13.0, -10.0),
                        egui::Align2::LEFT_TOP,
                        &f.label,
                        egui::FontId::proportional(12.0),
                        MAP_INK,
                    );
                }
                for m in &markers {
                    if self.hidden.contains(&m.id) {
                        continue;
                    }
                    let color = if m.stale { egui::Color32::GRAY } else { Self::ship_color(&m.id) };
                    if self.show_trail && self.mode.in_live() {
                        for (tx, ty) in &m.trail {
                            painter.circle_filled(
                                rect.min + egui::vec2(*tx as f32, *ty as f32),
                                2.0,
                                color.linear_multiply(0.55),
                            );
                        }
                    }
                    let c = rect.min + egui::vec2(m.x as f32, m.y as f32);
                    painter.circle_filled(c, 8.0, color);
                    painter.circle_stroke(c, 8.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
                    if Some(&m.id) == self.following.as_ref() {
                        painter.circle_stroke(c, 12.0, egui::Stroke::new(2.0, egui::Color32::YELLOW));
                    }
                    if self.selection == Some(Selection::Ship(m.id.clone())) {
                        painter.circle_stroke(
                            c,
                            12.0,
                            egui::Stroke::new(2.0, egui::Color32::LIGHT_BLUE),
                        );
                    }
                    painter.text(
                        c + egui::vec2(10.0, -10.0),
                        egui::Align2::LEFT_TOP,
                        &m.id,
                        egui::FontId::proportional(12.0),
                        MAP_INK,
                    );
                }
                // Log replay ghosts (task #41): hollow amber units as
                // placed up to the slider, from the journal — not live.
                if self.show_replay && self.log_view_path.is_some() {
                    let (rw, rh) = self.map_dims();
                    for (id, la, lo) in self.replay_state() {
                        let (gx, gy) =
                            project_mercator(la, lo, self.center, self.zoom, rw, rh);
                        let g = rect.min + egui::vec2(gx as f32, gy as f32);
                        painter.circle_stroke(
                            g,
                            8.0,
                            egui::Stroke::new(
                                2.0,
                                egui::Color32::from_rgb(0xF5, 0x9E, 0x0B),
                            ),
                        );
                        painter.text(
                            g + egui::vec2(10.0, -10.0),
                            egui::Align2::LEFT_TOP,
                            format!("{id} (replay)"),
                            egui::FontId::proportional(12.0),
                            MAP_INK,
                        );
                    }
                }
                // Waypoint legs: pending preview (white) + committed per
                // owned ship (light blue), drawn from the ship marker.
                let mut legs: Vec<((f64, f64), (f64, f64), egui::Color32)> = Vec::new();
                let (mw, mh) = self.map_dims();
                for m in &markers {
                    if self.hidden.contains(&m.id) {
                        continue;
                    }
                    if let Some(v) = self.order_views.get(&m.id) {
                        if let Some(wp) = v.waypoint {
                            if v.state == OrderState::EnRoute {
                                let (wx, wy) = project_mercator(
                                    wp.latitude, wp.longitude, self.center, self.zoom, mw, mh,
                                );
                                legs.push(((m.x, m.y), (wx, wy), egui::Color32::LIGHT_BLUE));
                            }
                        }
                    }
                }
                if let Some((la, lo)) = self.pending_waypoint {
                    if let Some(Selection::Ship(id)) = self.selection.clone() {
                        if let Some(m) = markers.iter().find(|m| m.id == id) {
                            let (wx, wy) = project_mercator(la, lo, self.center, self.zoom, mw, mh);
                            legs.push(((m.x, m.y), (wx, wy), egui::Color32::WHITE));
                        }
                    }
                    // Group preview: white legs from every in-jurisdiction
                    // member of the selected group.
                    if let Some(Selection::Group(gid)) = self.selection.clone() {
                        if let Some((_, members)) = self.group_info(&gid) {
                            for m in markers.iter().filter(|m| {
                                members.iter().any(|u| u == &m.id)
                                    && !self.hidden.contains(&m.id)
                                    && self.action_allows(&m.id)
                            }) {
                                let (wx, wy) =
                                    project_mercator(la, lo, self.center, self.zoom, mw, mh);
                                legs.push(((m.x, m.y), (wx, wy), egui::Color32::WHITE));
                            }
                        }
                    }
                }
                for ((x1, y1), (x2, y2), color) in legs {
                    painter.line_segment(
                        [rect.min + egui::vec2(x1 as f32, y1 as f32),
                         rect.min + egui::vec2(x2 as f32, y2 as f32)],
                        egui::Stroke::new(2.0, color),
                    );
                    painter.circle_filled(
                        rect.min + egui::vec2(x2 as f32, y2 as f32),
                        5.0,
                        color,
                    );
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("loading map scene…");
                });
            }
        });
    }
}

/// Dark ops-console theme (task #37): navy chrome over dark tiles, one
/// cyan accent, rounded islands, roomier spacing. Stock font, applied to
/// every theme slot so the look holds regardless of system preference.
fn apply_ops_theme(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.visuals = egui::Visuals::dark();
        let v = &mut style.visuals;
        let chrome = egui::Color32::from_rgb(0x0F, 0x17, 0x2A);
        let sunken = egui::Color32::from_rgb(0x02, 0x06, 0x17);
        let line = egui::Color32::from_rgb(0x33, 0x41, 0x55);
        let accent = egui::Color32::from_rgb(0x22, 0xD3, 0xEE);
        v.window_fill = chrome;
        v.window_stroke = egui::Stroke::new(1.0, line);
        v.window_corner_radius = egui::CornerRadius::same(8);
        v.panel_fill = chrome;
        v.faint_bg_color = egui::Color32::from_rgb(0x1E, 0x29, 0x3B);
        v.extreme_bg_color = sunken;
        v.hyperlink_color = accent;
        v.selection.bg_fill = accent;
        v.selection.stroke = egui::Stroke::new(1.0, sunken);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
            w.corner_radius = egui::CornerRadius::same(6);
        }
        v.widgets.hovered.weak_bg_fill =
            egui::Color32::from_rgba_unmultiplied(0x22, 0xD3, 0xEE, 40);
        v.widgets.active.weak_bg_fill =
            egui::Color32::from_rgba_unmultiplied(0x22, 0xD3, 0xEE, 70);
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.indent = 20.0;
    });
}

/// Light label ink for dark tiles (task #37): ship ids and flags read
/// against night water, not against paper.
const MAP_INK: egui::Color32 = egui::Color32::from_rgb(0xE2, 0xE8, 0xF0);

fn main() -> eframe::Result<()> {
    // Poll thread owns the backend source; the UI owns the registry.
    // TFG_BACKEND_URL=http://host:port selects HTTP, else file replay.
    // The sim joins every round via MergeSource (disarmed = wire only).
    let shutdown = std::sync::Arc::new(AtomicBool::new(false));
    let (poll_tx, poll_rx) = mpsc::channel();
    let (sim_cmd_tx, sim_cmd_rx) = mpsc::channel::<SimCommand>();
    let (sim_evt_tx, sim_evt_rx) = mpsc::channel::<SimEvent>();
    let ui_sim_cmd_tx = sim_cmd_tx.clone();
    let poll_shutdown = shutdown.clone();
    // Session flow: presentation mode boots disarmed (wire-only); the
    // shell arms simulation mode through this flag (Q1: freeze, no lies).
    let sim_armed = Arc::new(AtomicBool::new(false));
    let poll_armed = sim_armed.clone();
    let (wire_ctl_tx, wire_ctl_rx) = mpsc::channel::<WireKind>();
    let ui_wire_ctl_tx = wire_ctl_tx.clone();
    let poll_handle = std::thread::spawn(move || {
        // Boot wire (task #39): env picks HTTP vs replay; the Connection
        // island can swap it later without restarting the sim.
        let empty = format!("{}/scenarios/empty.json", env!("CARGO_MANIFEST_DIR"));
        let boot_kind = match std::env::var("TFG_BACKEND_URL") {
            Ok(url) => WireKind::Http(url),
            Err(_) => WireKind::Replay(match std::env::var("TFG_SCENARIO") {
                // `surge` for the traffic demo; default is the clear canvas.
                Ok(name) => format!("{}/scenarios/{name}.json", env!("CARGO_MANIFEST_DIR")),
                Err(_) => empty,
            }),
        };
        let (wire, desc) = match build_wire(&boot_kind) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("backend failed to start: {e}");
                return;
            }
        };
        eprintln!("backend: {desc}");
        let mut source = MergeSource::new(
            wire,
            SimSource::new_with_journal(
                sim_cmd_rx,
                sim_evt_tx,
                tfg::log::Journal::open(tfg::log::Journal::prototype_path())
                    .unwrap_or_else(|e| {
                        eprintln!("session log disabled: {e}");
                        tfg::log::Journal::disabled()
                    }),
            ),
        );
        source.set_armed_flag(poll_armed);
        loop {
            // Runtime wire swaps from the Connection island (task #39).
            while let Ok(kind) = wire_ctl_rx.try_recv() {
                match build_wire(&kind) {
                    Ok((w, desc)) => {
                        eprintln!("backend: {desc}");
                        source.set_wire(w);
                    }
                    Err(e) => eprintln!("wire swap failed: {e}"),
                }
            }
            match source.poll() {
                Ok(fixes) => {
                    if poll_tx.send(fixes).is_err() {
                        return; // UI gone
                    }
                }
                Err(e) => eprintln!("poll failed (ships keep misses): {e}"),
            }
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(100));
                if poll_shutdown.load(Ordering::SeqCst) {
                    return;
                }
            }
        }
    });

    // Map thread owns the persistent scene; frames come back by channel.
    // It exits when the UI drops its request sender, dropping the scene
    // on this thread (see on_exit) instead of racing process teardown.
    let (map_req_tx, map_req_rx) = mpsc::channel::<MapReq>();
    let (map_resp_tx, map_resp_rx) = mpsc::channel::<MapResp>();
    let map_handle = std::thread::spawn(move || {
        let mut size = (MAP_W as u32, MAP_H as u32);
        let mut scene = LiveMap::new(CENTER, ZOOM, size.0, size.1, STYLE, tfg::map_render::repo_cache_path());
        while let Ok(first) = map_req_rx.recv() {
            // Newest-wins (task #43): a burst of scroll-zoom requests
            // renders once, so the texture never lags seconds behind.
            let mut latest = first;
            for newer in map_req_rx.try_iter() {
                latest = newer;
            }
            let (seq, at, zoom, px, pump) = latest;
            if px != size {
                // Window resize: rebuild the scene once at the new size.
                scene = LiveMap::new(at, zoom, px.0.max(1), px.1.max(1), STYLE, tfg::map_render::repo_cache_path());
                size = px;
            }
            scene.set_center(at, zoom);
            // Smoothness measurement: round-trip cost per frame, so tile
            // lag can be split into pump-bound vs render/network-bound.
            let t0 = std::time::Instant::now();
            scene.pump(pump);
            let rgba = scene.frame_rgba();
            eprintln!("map frame {seq}: {}ms (pump {pump})", t0.elapsed().as_millis());
            if map_resp_tx.send((seq, at, zoom, size, rgba)).is_err() {
                break; // UI gone
            }
        }
    });
    // Initial frame so the window never opens empty-handed for long.
    map_req_tx.send((0, CENTER, ZOOM, (MAP_W as u32, MAP_H as u32), JUMP_PUMP)).expect("map thread alive");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1040.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "tfg command center (egui)",
        options,
        Box::new(|cc| {
            // Required once: without image loaders, from_bytes fails.
            egui_extras::install_image_loaders(&cc.egui_ctx);
            apply_ops_theme(&cc.egui_ctx);
            Ok(Box::new(ShipApp {
                map_tex: None,
                tex_center: CENTER,
                tex_zoom: ZOOM,
                tex_px: (MAP_W as u32, MAP_H as u32),
                map_px: (MAP_W as u32, MAP_H as u32),
                map_view: (MAP_W, MAP_H),
                last_desired_px: (MAP_W as u32, MAP_H as u32),
                center: CENTER,
                registry: Registry::new(TrailBound::default()),
                poll_rx,
                map_req_tx: Some(map_req_tx),
                map_resp_rx,
                map_seq: 0,
                recentering: None,
                shutdown,
                poll_handle: Some(poll_handle),
                map_handle: Some(map_handle),
                last_poll: Instant::now(),
                hidden: HashSet::new(),
                following: None,
                show_trail: true,
                mode: UiMode::new(sim_armed.clone()),
                session_windows: None,
                session_log_path: tfg::log::Journal::prototype_path(),
                session_seq: 0,
                transcript: Vec::new(),
                show_roster: false,
                show_orders: false,
                show_log: false,
                wizard_step: 0,
                wizard_done: false,
                event_feed: VecDeque::new(),
                selection: None,
                last_seen: HashMap::new(),
                fix_count: HashMap::new(),
                sim_cmd_tx: Some(ui_sim_cmd_tx),
                sim_evt_rx,
                order_views: HashMap::new(),
                controlled: HashSet::new(),
                pending_waypoint: None,
                placing: false,
                order_speed: 20.0,
                land: Land::from_default_asset().ok(),
                order_warning: None,
                catalog: Catalog::from_default_asset().expect("catalog asset valid"),
                selected_class: 0,
                fleet: Fleet::from_default_asset().expect("fleet asset valid"),
                show_fleet: false,
                fleet_cat: 0,
                fleet_class: None,
                fleet_query: String::new(),
                fleet_pick: None,
                placed_fleet: HashSet::new(),
                time_real_start: (Utc::now() + chrono::Duration::hours(7)).format("%Y-%m-%d %H:%M").to_string(),
                time_real_end: (Utc::now() + chrono::Duration::hours(14)).format("%Y-%m-%d %H:%M").to_string(),
                time_game_start: "2026-11-01 00:00".to_string(),
                time_game_end: "2026-11-07 00:00".to_string(),
                roster: Vec::new(),
                roster_input: String::new(),
                helm: HashMap::new(),
                unit_commander: HashMap::new(),
                invites: Vec::new(),
                invite_seq: 1,
                invite_status: "local records".to_string(),
                session_ratio: SESSION_RATIO,
                zoom: ZOOM,
                last_zoom_req: Instant::now(),
                last_track_req: Instant::now(),
                zoom_dirty: false,
                app_mode: AppMode::Simulation,
                // Mirrors the poll thread's boot-wire choice above (task
                // #39): status text only, the thread owns the source.
                backend_url: std::env::var("TFG_BACKEND_URL")
                    .unwrap_or("http://127.0.0.1:3000".to_string()),
                wire_desc: match std::env::var("TFG_BACKEND_URL") {
                    Ok(url) => format!("HTTP {url}"),
                    Err(_) => match std::env::var("TFG_SCENARIO") {
                        Ok(name) => format!("replay scenarios/{name}.json"),
                        Err(_) => "replay scenarios/empty.json".to_string(),
                    },
                },
                conn_status: "boot source (env)".to_string(),
                show_connection: false,
                // Boot is the lobby (task #40): the mode toggle plus the
                // Simulation start wizard. Everything working opens later.
                show_session: true,
                wire_ctl_tx: Some(ui_wire_ctl_tx),
                log_view_path: None,
                log_events: Vec::new(),
                replay_pos: 0,
                show_replay: true,
                log_filter: "all".to_string(),
                groups: Groups::default(),
                show_groups: false,
                group_seq: 1,
                satgas_name: String::new(),
                satgas_commander: None,
                satgas_members: HashSet::new(),
                gugus_name: String::new(),
                gugus_commander: None,
                gugus_members: HashSet::new(),
                group_error: None,
                acting_as: None,
                desktop: "all".to_string(),
                drafts: HashMap::new(),
                game_elapsed_secs: None,
                game_ratio: 1.0,
                game_paused: false,
                real_ts: None,
                game_ts: None,
            }))
        }),
    )
}
