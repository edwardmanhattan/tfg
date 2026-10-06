//! egui command-center application, live edition over a persistent map scene.
//!
//! - A background poll thread feeds mock fixes into the [`Registry`] at the
//!   v0 cadence; markers glide by wall-clock fraction (`Registry::blend`).
//! - A background map thread owns one persistent [`LiveMap`]: recenter is a
//!   camera update + a few pumped frames (milliseconds), and follow-tracking
//!   is now just repeated camera updates, not pipeline rebuilds.
//! - Frames arrive as raw RGBA into versioned egui textures; overlay
//!   markers/trails/roster are immediate-mode (ADR-0001/0002).
//!
//! Run: `cargo run` (or `scripts/run-egui-window.sh`)
//! Installed bundles embed their read-only assets; see `docs/runtime-paths.md`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use eframe::egui;
use chrono::{TimeZone, Utc};
use tfg::backend::{BackendError, FileReplay, GameFix, GameMsg, GamePositionUpdate, LiveCmd, LiveEvent, LiveWire, MinosAuth, MinosMaster, MinosRest, MockPoll, PollSource, TokenPair};
use tfg::catalog::Catalog;
use tfg::fleet::Fleet;
use tfg::gamestate::GameState;
use tfg::groups::{GroupKind, Groups};
use tfg::command::{Authority, Grant, GrantDenial, Leg, MoveCommand, Verb};
use tfg::geo::track::{Fix, FixSource, Registry, TrailBound, should_track};
use tfg::geo::GeoPosition;
use tfg::map_render::LiveMap;
use tfg::paths::AppPaths;
use tfg::map_render::{
    FRAME_VIEWPORT_FRACTION, GROUP_ZONE_MIN_PX, GroupRepresentation, MIN_HIT_PX,
    GLYPH_AUTHORING_RADIUS_PX, ProjectedUnitGeometry, SYMBOL_BOX_PX, SymbolFrame, UnitLod,
    ZONE_HIT_BAND_PX, anchor_center, battle_dimension, frame_for, frame_icon_radius, frame_polygon,
    frame_strokes, grid_spacing_deg, hit_polygon, meters_per_pixel, project_mercator,
    projected_group_extent, projected_unit_geometry, rotated_unit_quad_with_forward_heading,
    select_group_representation, select_unit_lod, should_paint_group_text, should_paint_unit_label,
    symbol_footprint_radius_px, unit_is_planned, unproject_mercator, zone_pad_px, zone_polygon,
    zone_width_px, zoom_for_group_frame, zoom_for_ground_resolution,
};
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
/// Map frame answer: conversion to ColorImage happens on the map
/// thread — the frame pump only uploads to the GPU, never memcpys
/// pixels on the UI thread.
type MapResp = (u64, (f64, f64), f64, (u32, u32), egui::ColorImage);
const CENTER: (f64, f64) = (-6.108, 106.910);
const ZOOM: f64 = 11.0;
/// Inspector thumbnails are a view preference, not game state. Keep
/// the default compact so a wide source PNG cannot expand the island;
/// operators can still enlarge it without changing the asset or map.
const INSPECTOR_IMAGE_DEFAULT_WIDTH: f32 = 280.0;
const INSPECTOR_IMAGE_MIN_WIDTH: f32 = 160.0;
const INSPECTOR_IMAGE_MAX_WIDTH: f32 = 640.0;
const INSPECTOR_IMAGE_MAX_HEIGHT: f32 = 180.0;
const MILLER_COL_WIDTH: f32 = 150.0;
/// The drill's leaf holds a thumbnail, a name and a verb, where the four
/// taxonomy columns hold a name each. Wider for that reason, not to
/// emphasise it — and the five together still fit the panel, which is a
/// property `drill_columns_fit_the_picker_width` pins.
const MILLER_LEAF_WIDTH: f32 = 232.0;
/// The panel's fixed size. Named because the layout below is arithmetic
/// against it: the strip takes what the header rows leave and the tail
/// reserves what is left over.
const FLEET_PICKER_SIZE: egui::Vec2 = egui::vec2(980.0, 620.0);
/// The strip of map the picker yields so a drag has somewhere to land.
///
/// The Fleet picker is where placement is ARMED, but the gesture that completes
/// it ends on the map — and a centred modal 980x620 in a 1040x640 window leaves
/// a 30pt frame of map around itself, which is not a target. Every drop on it
/// came back "placement needs water", correctly, because the frame is wherever
/// the frame happens to be. A form that covers the thing it is a form about
/// cannot have a drag gesture onto that thing.
///
/// So the panel is sized off the window rather than fixed, always giving this
/// much map back. The columns then size themselves from what is left, which is
/// what `picker_strip_height` already does.
const PICKER_MAP_BAND: f32 = 160.0;
/// The panel never gets shorter than this, whatever the window: below it the
/// five columns stop being columns and the form starts losing rows.
const PICKER_MIN_H: f32 = 420.0;

/// The panel's size for a window of `viewport_h`.
///
/// Width is fixed — the five columns need it, and `drill_columns_fit_the_picker_width`
/// is what says so. Height is the window minus the map band, capped at the
/// declared size, because a window with room to spare should not get a taller
/// panel than the design has.
fn fleet_picker_size(viewport_h: f32) -> egui::Vec2 {
    egui::vec2(
        FLEET_PICKER_SIZE.x,
        (viewport_h - PICKER_MAP_BAND).clamp(PICKER_MIN_H, FLEET_PICKER_SIZE.y),
    )
}
/// Worst-case rows of chrome below the columns.
///
/// Three: the count, the last placement's outcome, and the place/cancel pair.
/// The unarmed branch shares one row for its status and its arm button, so the
/// tail's height does not change with the engine's state — which matters,
/// because a tail that grew and shrank would move the controls under the
/// operator's cursor.
///
/// It was five, then four, then three, then two, and every revision was wrong
/// in the same way: the number described what the tail happened to be doing
/// that week rather than what it is allowed to do. Anything new in the tail has
/// to fit in these three rows or the strip gives, and the strip giving is
/// visible.
const PICKER_TAIL_ROWS: f32 = 3.0;
/// Floor for the strip, so a short panel shows a usable column rather than a
/// sliver, and the tail is still reachable under it.
const PICKER_STRIP_MIN: f32 = 120.0;

/// The height of one row of the tail, measured off egui rather than assumed.
///
/// The taller of a label and a button, plus the gap under it — because the tail
/// is a mix of the two and a budget built out of the label alone is short by
/// however much a button is taller. That was the last version's mistake: it
/// measured a 12pt label, reserved three of them, and left the button row
/// hanging off the bottom of the panel.
///
/// Asked of the widgets' own `desired_size`, NOT by adding them to the panel and
/// throwing them away. `add_visible(false)` looks like it would do that and does
/// not: egui's own note on `set_invisible` says widgets stay invisible "yet
/// still allocate space", so the probe was quietly eating a row of the very
/// layout it was measuring. Every strip that grew between one frame and the
/// next was that probe, and the symptom was the tail sliding off the bottom
/// again — the defect returning by a different route.
///
/// Asked of a scratch `Ui` built at a zero rect, so the probe lays the widgets
/// out somewhere that is not the panel. `Ui::new` is the 0.36 spelling, and the
/// rect is `ZERO` precisely because nothing is read from it but the widgets'
/// own sizes.
fn picker_row_height(ui: &egui::Ui) -> f32 {
    let mut scratch = egui::Ui::new(
        ui.ctx().clone(),
        ui.id().with("picker-row-probe"),
        egui::UiBuilder::new().max_rect(egui::Rect::ZERO),
    );
    use egui::Widget as _;
    let label = egui::Label::new(egui::RichText::new("M")).ui(&mut scratch);
    let button = egui::Button::new("M").ui(&mut scratch);
    label.rect.height().max(button.rect.height()) + ui.spacing().item_spacing.y
}

/// The height of the column strip, given the height the panel has left.
///
/// Derived rather than stored. It used to be a slider, and a slider is how
/// the tail was pushed off the panel: the operator could make the columns any
/// height they liked, and every height above the panel's remaining space put
/// the count and the placement controls past the bottom edge, where a modal
/// body with no scroll cannot reach them. So the control is gone and the
/// number comes from the panel.
fn picker_strip_height(available: f32, row: f32) -> f32 {
    (available - PICKER_TAIL_ROWS * row).max(PICKER_STRIP_MIN)
}
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

/// Whether the top zone is rendering floating chrome or the onboarding card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkState {
    /// No socket. The default before anyone asks to connect.
    Idle,
    /// A connect is in flight. Nothing has been proved yet.
    Connecting,
    /// The actor reported Connected.
    Live,
    /// The server refused: bad token, or a gate in the way.
    Refused,
    /// Transport failed after a connect, or the socket dropped.
    Faulted,
}

impl LinkState {
    /// The one word the top zone shows. Short caps, no verb, because this is
    /// a state and not an event.
    fn label(self) -> &'static str {
        match self {
            LinkState::Idle => "OFFLINE",
            LinkState::Connecting => "CONNECTING",
            LinkState::Live => "LIVE",
            LinkState::Refused => "REFUSED",
            LinkState::Faulted => "FAULT",
        }
    }

    /// The status ink. Every state routes through here, so a line reporting
    /// the link can never be gray-on-gray (DESIGN.md's Never Gray-On-Gray
    /// rule).
    fn ink(self) -> egui::Color32 {
        match self {
            LinkState::Live => tfg::tokens::SIGNAL_GREEN,
            LinkState::Connecting => tfg::tokens::WARNING_SAND,
            LinkState::Refused | LinkState::Faulted => tfg::tokens::FAULT_RED,
            LinkState::Idle => tfg::tokens::IDLE_GREY,
        }
    }

    /// Whether the socket is supposed to exist, which is not the same as
    /// whether it is up. A refusal leaves it wired, so `is_some` on the
    /// sender is the honest "asked for" and this is the honest "up".
    fn is_up(self) -> bool {
        matches!(self, LinkState::Live)
    }
}

/// Placement pointer tool. Active in Setup (initial fleet) and Live
/// (reinforcements); waypoint arming is a separate Live draft (`placing`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SetupTool {
    Select,
    Place,
}

/// The selected fleet pick is the placement intent. The UI tool is only a
/// visual hint: it can be reset by a phase transition while the pick is still
/// waiting for the operator's map click.
fn placement_click_allowed(
    phase: Phase,
    pick: Option<&str>,
    armed: bool,
    acting_as: Option<&str>,
) -> bool {
    pick.is_some()
        && (phase == Phase::Setup || phase == Phase::Live)
        && armed
        && acting_as.is_none()
}

/// Simulation keeps local fixes visible during Setup so placed units appear
/// immediately. Presentation remains wire-first even when its local phase is
/// not Live; a closed Simulation is frozen and accepts no movement.
fn accepts_movement_fixes(app_mode: AppMode, phase: Phase) -> bool {
    app_mode == AppMode::Presentation
        || (app_mode == AppMode::Simulation
            && (phase == Phase::Setup || phase == Phase::Live))
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

/// Onboarding state machine: the first-run path is A (Login) → shell.
/// Boots to A every launch: it doubles as the login gate.
///
/// THERE IS NO MODE STEP, and there was never a good reason for one. It
/// existed to ask "Command Center or Tactical Floor Game?", which is the
/// app naming its own function to the person using it — the console's job
/// is to run the exercise, and a card that says so before the map is even
/// up is the app announcing itself. Simulation is the default and the only
/// mode a launch reaches; Presentation stays reachable, but from Settings
/// and as a toggle rather than as a question at the door.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Onboard {
    Login,
    App,
}

/// Simulation's four phases (onboarding ticket, #77): Planning
/// (Perencanaan) and Ready (Persiapan / OnReady) both live in
/// `Phase::Setup` — the `sim_ready` flag is the gate between them —
/// Eksekusi is `Phase::Live`, Evaluasi is `Phase::Closed`. Derived
/// from `UiMode` every frame, never stored, so the existing state
/// machine (#26) stays the single writer of the phase.
/// Runtime wire backends for the poll thread (task #39). Replay stays
/// boot-only (env).
enum WireKind {
    /// Explicit mock source (M10): examples/mock_backend.rs over
    /// TFG_BACKEND_URL. Never a Minos route — Minos is Live below.
    Mock(String),
    /// A built-in scenario embedded in the executable. The value is the
    /// scenario stem accepted by [`tfg::assets::scenario_json`].
    EmbeddedReplay(String),
    Live {
        ws_url: String,
        rest_base: String,
        token: String,
        cmd_tx: Sender<LiveCmd>,
        cmd_rx: Receiver<LiveCmd>,
        event_tx: Sender<LiveEvent>,
        wake_tx: Sender<()>,
    },
    Empty,
}

/// Wire source that yields nothing: a disconnected presentation, or a
/// simulation running on sim data alone.
struct EmptyWire;

impl PollSource for EmptyWire {
    fn poll(&mut self) -> Result<Vec<Fix>, BackendError> {
        Ok(Vec::new())
    }
}

/// Build a wire source plus its status description. Live failures fall
/// back to an empty wire with the reason (feed-down reads as stale).
fn build_wire(kind: WireKind) -> Result<(Box<dyn PollSource>, String), String> {
    match kind {
        WireKind::Mock(url) => MockPoll::new(&url)
            .map(|h| (Box::new(h) as Box<dyn PollSource>, format!("mock {url}")))
            .map_err(|e| e),
        WireKind::EmbeddedReplay(name) => {
            let json = tfg::assets::scenario_json(&name)
                .ok_or_else(|| format!("unknown built-in scenario {name}"))?;
            FileReplay::from_json(json)
                .map(|r| (Box::new(r) as Box<dyn PollSource>, format!("replay {name} (embedded)")))
                .map_err(|e| format!("embedded scenario {name}: {e}"))
        }
        WireKind::Live { ws_url, rest_base, token, cmd_tx, cmd_rx, event_tx, wake_tx } => {
            let rest = MinosRest::new(&rest_base).map_err(|e| e)?;
            match LiveWire::connect(&ws_url, &rest, &token, cmd_tx, cmd_rx, event_tx, wake_tx) {
                Ok(w) => Ok((Box::new(w) as Box<dyn PollSource>, format!("live {ws_url}"))),
                Err(e) => Ok((
                    Box::new(EmptyWire) as Box<dyn PollSource>,
                    format!("live failed ({e}); empty"),
                )),
            }
        }
        WireKind::Empty => Ok((Box::new(EmptyWire) as Box<dyn PollSource>, "empty".to_string())),
    }
}
/// One parsed history journal, loaded off-thread: entries plus
/// the units/players seen plus the replay events. The island renders
/// this, never the disk.
#[derive(Debug, Clone)]
struct LogViewData {
    entries: Vec<LogLine>,
    units: Vec<String>,
    players: Vec<String>,
    replay: Vec<ReplayEvent>,
}

/// Finished log work (blocking ticket): a journal parsed, or the
/// session transcript tail. Failures report and keep the old view.
enum LogOut {
    Files(Vec<std::path::PathBuf>),
    View(std::path::PathBuf, LogViewData),
    Transcript(Vec<String>),
}

/// One parsed history line: display text plus its actor and ships for
/// the unit/player filters (task #41).
#[derive(Debug, Clone)]
struct LogLine {
    text: String,
    actor: String,
    ships: Vec<String>,
}

/// One placement-affecting journal entry (task #41): take-control adds
/// the ghost, release removes it. Parsed from Command payloads.
#[derive(Debug, Clone)]
struct ReplayEvent {
    game_ts: String,
    ship: String,
    lat: f64,
    lon: f64,
    placed: bool,
}

/// v0 poll cadence, in seconds.
const POLL_SECS: f64 = 2.0;
/// MinOS publishes game position snapshots about once per second. The
/// presentation clock matches that stream; it does not predict beyond
/// the latest accepted Fix.
const GAME_PLOT_ANIMATION_SECS: f64 = 1.0;
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
/// Idle backstop for the repaint loop. Long enough to be nearly free on an
/// idle station, short enough that a repaint we failed to anticipate shows
/// up as a two-second-old readout instead of a hung console. See the
/// on-demand repaint in `ShipApp::ui`.
const WATCHDOG_REPAINT: Duration = Duration::from_secs(2);
/// Cadence while a REST request is outstanding — fast enough that a login
/// result lands in one frame, and only paid for while something is pending.
const REST_POLL_REPAINT: Duration = Duration::from_millis(50);
/// Lead a followed ship by this many seconds of dead reckoning, so tile
/// fetches run ahead of motion instead of behind it (task #45).
const TRACK_LEAD_SECS: f64 = 8.0;

struct ShipMarker {
    id: String,
    x: f64,
    y: f64,
    stale: bool,
    /// Wire data older than the old-data threshold (backfilled ticket).
    old_data: bool,
    source: FixSource,
    trail: Vec<(f64, f64)>,
    /// Course, shortest-arc blended across the poll interval so a
    /// turn does not spin a thumbnail the long way round. None when
    /// the fix carries no course — the marker then draws neutral and
    /// says so, rather than assuming north.
    heading_deg: Option<f32>,
    /// Taxonomy-derived far-map glyph. Unknown always draws, so a
    /// missing visual, image, texture, or taxonomy cannot make the
    /// unit disappear.
    map_symbol: tfg::store::MapSymbol,
    latitude: f64,
    /// The blended position's longitude, carried beside the latitude
    /// so a group's ground extent is measured from the same fix the
    /// marker is drawn with rather than a second lookup of the
    /// registry's latest.
    longitude: f64,
    /// The marker's own drawn extent in logical pixels — the larger of
    /// its length and beam — carried so a group's Zone can be padded to
    /// enclose what it actually holds rather than to enclose a point.
    /// Zero when nothing measurable is known; the Zone pad floors that.
    footprint_px: f64,
    /// Resolved affiliation, so the painter never reaches back into the
    /// resolver mid-frame. Unknown is a real answer, never a missing
    /// one: the resolver always terminates.
    affiliation: tfg::store::Affiliation,
    /// In the exercise but unassigned in the task organisation, which the
    /// symbology draws as a dashed frame. Independent of `stale`: a
    /// planned unit and a silent one are different facts.
    planned: bool,
    label: String,
    lod: UnitLod,
    visual: Option<UnitVisual>,
}

/// Which key an affiliation declaration is filed under. The three
/// declaration keys exist because an UNPLACED hull belongs to no group:
/// a group-only model leaves every unplaced contact unknown forever,
/// which is honest and useless for an opposing force nobody bothered to
/// place. `OurBranch` is the odd one out — it is a fact about the
/// operator rather than about an exercise, so it is not session-scoped.
#[derive(Debug, Clone)]
enum Declaration {
    Unit(i64),
    Group(String),
    Branch(i64),
    OurBranch(i64),
}

/// One drill row label (setup-overhaul picker): id_name first (grill
/// decision), English subtitle when it differs, next-level count.
/// Truncated to one line so long taxonomy names never stretch the column.
/// Status ink (harden): success reads green, failure red, idle gray.
/// Every `*_status` line goes through this so state is never gray-on-gray.
/// Trimmed text, or None when blank: filter drafts pass through as
/// absent, never as empty strings the server would have to interpret.
fn nonempty(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn status_ink(msg: &str) -> egui::Color32 {    let m = msg.to_lowercase();
    if m.contains("fail")
        || m.contains("refused")
        || m.contains("error")
        || m.contains("unreachable")
        || m.starts_with("sync stopped")
        || m.starts_with("socket error")
    {
        egui::Color32::from_rgb(0xF8, 0x71, 0x71)
    } else if m.contains("signed in as")
        || m.starts_with("synced")
        || m.starts_with("connected")
        || m.contains("refreshed")
        || m.contains("issued")
        || m.contains("redeemed")
    {
        egui::Color32::from_rgb(0x4A, 0xDE, 0x80)
    } else {
        egui::Color32::GRAY
    }
}

/// One-line status readout with state ink. Long backend messages wrap
/// instead of stretching the island.
fn status_line(ui: &mut egui::Ui, msg: &str) {
    ui.label(egui::RichText::new(msg).color(status_ink(msg)));
}

/// Warning line: names the problem; the adjacent control names the recovery.
fn warn_line(ui: &mut egui::Ui, msg: String) {
    ui.label(egui::RichText::new(format!("⚠ {msg}")).color(egui::Color32::YELLOW));
}

/// Human-readable freshness for the context strip. A bare `162s` reads like
/// a sensor dump; minutes and an explicit “never” are useful at a glance.


/// Strip fractional seconds and the date from an RFC3339 timestamp. The full
/// wire value remains available in the log; the operator only needs “now”.
fn short_timestamp(raw: Option<&str>) -> String {
    let Some(raw) = raw else { return "—".to_string() };
    if let Some((_, time)) = raw.split_once(' ') {
        return time.to_string();
    }
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|value| value.format("%H:%M:%SZ").to_string())
        .unwrap_or_else(|_| raw.to_string())
}

/// Onboarding backdrop (ticket #77): one vertical wash — console
/// night down into deep well — so States A/B read as a clean slate,
/// no map, islands, or wizard. Two triangles: vertex colors
/// interpolate in the tessellator, no band loop.
fn paint_gradient(
    painter: &egui::Painter,
    rect: egui::Rect,
    top: egui::Color32,
    bottom: egui::Color32,
) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
}

/// Temporary display scale for a hull whose real LOA and beam are not
/// published yet. This is deliberately a screen-space presentation
/// budget, not a physical measurement; it will be replaced by the
/// Minos geometry as soon as those fields arrive.
const FALLBACK_UNIT_LENGTH_PX: f64 = 36.0;

fn fallback_unit_geometry(visual: &UnitVisual) -> (f64, f64) {
    let beam = visual
        .width_px
        .zip(visual.height_px)
        .filter(|(w, h)| *w > 0 && *h > 0)
        .map(|(w, h)| FALLBACK_UNIT_LENGTH_PX * h as f64 / w as f64)
        .unwrap_or(10.0)
        .clamp(4.0, 24.0);
    (FALLBACK_UNIT_LENGTH_PX, beam)
}

/// Fit an Inspector image inside the available width without letting a
/// tall source consume the whole island. The requested width is a view
/// preference; the fit is still bounded by the current Inspector layout.
fn inspector_image_size(
    texture_size: egui::Vec2,
    requested_width: f32,
    available_width: f32,
) -> egui::Vec2 {
    let available_width = available_width.max(1.0);
    let width = requested_width
        .clamp(INSPECTOR_IMAGE_MIN_WIDTH, INSPECTOR_IMAGE_MAX_WIDTH)
        .min(available_width);
    if texture_size.x <= 0.0 || texture_size.y <= 0.0 {
        return egui::vec2(width, 0.0);
    }
    let aspect = texture_size.y / texture_size.x;
    let height = width * aspect;
    if height > INSPECTOR_IMAGE_MAX_HEIGHT {
        egui::vec2((INSPECTOR_IMAGE_MAX_HEIGHT / aspect).min(width), INSPECTOR_IMAGE_MAX_HEIGHT)
    } else {
        egui::vec2(width, height)
    }
}

/// The small vocabulary the far map can draw. Every stable map symbol
/// maps to exactly one geometry, so a new symbol cannot silently reuse
/// the old anonymous dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum MapSymbolShape {
    Dot,
    Destroyer,
    Frigate,
    Corvette,
    Auxiliary,
    Landing,
    Submarine,
    Plane,
    Tank,
    Port,
}

fn map_symbol_shape(symbol: tfg::store::MapSymbol) -> MapSymbolShape {
    use tfg::store::MapSymbol;
    match symbol {
        MapSymbol::UnknownShip => MapSymbolShape::Dot,
        MapSymbol::Destroyer => MapSymbolShape::Destroyer,
        MapSymbol::Frigate => MapSymbolShape::Frigate,
        MapSymbol::Corvette => MapSymbolShape::Corvette,
        MapSymbol::Auxiliary => MapSymbolShape::Auxiliary,
        MapSymbol::Landing => MapSymbolShape::Landing,
        MapSymbol::Submarine => MapSymbolShape::Submarine,
        MapSymbol::Plane => MapSymbolShape::Plane,
        MapSymbol::GroundUnit => MapSymbolShape::Tank,
        MapSymbol::Port => MapSymbolShape::Port,
    }
}

fn regular_polygon(
    center: egui::Pos2,
    radius: f32,
    sides: usize,
    rotation: f32,
) -> Vec<egui::Pos2> {
    (0..sides)
        .map(|i| {
            let angle = rotation + i as f32 * std::f32::consts::TAU / sides as f32;
            center + egui::vec2(angle.cos() * radius, angle.sin() * radius)
        })
        .collect()
}

fn map_symbol_label(symbol: tfg::store::MapSymbol) -> &'static str {
    use tfg::store::MapSymbol;
    match symbol {
        MapSymbol::UnknownShip => "generic ship",
        MapSymbol::Destroyer => "destroyer",
        MapSymbol::Frigate => "frigate",
        MapSymbol::Corvette => "corvette",
        MapSymbol::Auxiliary => "auxiliary",
        MapSymbol::Landing => "landing",
        MapSymbol::Submarine => "submarine",
        MapSymbol::Plane => "plane",
        MapSymbol::GroundUnit => "ground unit",
        MapSymbol::Port => "port",
    }
}

fn map_symbol_accent(shape: MapSymbolShape) -> egui::Color32 {
    match shape {
        MapSymbolShape::Dot => egui::Color32::LIGHT_GRAY,
        MapSymbolShape::Destroyer => egui::Color32::from_rgb(0xFF, 0x6B, 0x6B),
        MapSymbolShape::Frigate => egui::Color32::from_rgb(0x6B, 0x9D, 0xFF),
        MapSymbolShape::Corvette => egui::Color32::from_rgb(0x35, 0xD0, 0xE8),
        MapSymbolShape::Auxiliary => egui::Color32::from_rgb(0xF5, 0xC2, 0x4B),
        MapSymbolShape::Landing => egui::Color32::from_rgb(0x72, 0xD6, 0x72),
        MapSymbolShape::Submarine => egui::Color32::from_rgb(0xB4, 0x8C, 0xE8),
        MapSymbolShape::Plane => egui::Color32::from_rgb(0xE8, 0xF0, 0xFF),
        MapSymbolShape::Tank => egui::Color32::from_rgb(0xA8, 0xB5, 0x62),
        MapSymbolShape::Port => egui::Color32::from_rgb(0xE8, 0x79, 0xD1),
    }
}

/// The ink a Far symbol's glyph is drawn in.
///
/// The type accent, LIGHTENED toward white so it reads against the frame's
/// dark interior fill. This used to blend the accent only 32 percent into a
/// mid-tone base, which put every one of the ten accents between 1.0 and 2.1
/// contrast against the map's own water and land — `Auxiliary` at 1.02 is the
/// sea, drawn on the sea. On the dark fill those same hues reach 6.8 to 9.8
/// against white, so the type channel is finally visible.
///
/// Type is still carried primarily by SHAPE, which is the invariant
/// `every_map_symbol_has_a_distinct_shape_and_fill` holds; this is the
/// redundant channel beside it, and being redundant is why lightening it is
/// safe. A greyscale screenshot reads the shape.
fn map_symbol_fill(
    shape: MapSymbolShape,
    stale: bool,
) -> egui::Color32 {
    if stale {
        return egui::Color32::GRAY;
    }
    let accent = map_symbol_accent(shape);
    // 55 percent of the accent, 45 percent white. Enough hue to tell a
    // corvette's cyan from a landing ship's green, bright enough to read as
    // ink against the fill rather than as another dark shape.
    egui::Color32::from_rgb(
        (accent.r() as f32 * 0.55 + 255.0 * 0.45).round() as u8,
        (accent.g() as f32 * 0.55 + 255.0 * 0.45).round() as u8,
        (accent.b() as f32 * 0.55 + 255.0 * 0.45).round() as u8,
    )
}

/// Paint a group's far symbol: frame shape for allegiance, icon for
/// what its members are, LEVEL colour for its place in the task
/// organisation.
///
/// Three orthogonal channels, so the group reads without being told which
/// is which: a shape an operator already knows from every map, a glyph
/// from the same vocabulary as a unit's, and a hue the Zones already use
/// — which is what makes a group look the same in both its
/// representations.
fn paint_group_symbol(
    painter: &egui::Painter,
    ui: &egui::Ui,
    symbol: &GroupSymbolGeom,
    origin: egui::Pos2,
    focused: bool,
    zoom: f64,
) {
    let c = origin + egui::vec2(symbol.x, symbol.y);
    let frame = frame_for(symbol.affiliation);
    // A group's dimension follows its members' kind: a formation of
    // aircraft is not a surface task force, whatever its allegiance.
    let dimension = symbol
        .icon
        .map(battle_dimension)
        .unwrap_or(tfg::map_render::BattleDimension::LandAndSeaSurface);
    // The interior is filled in the LEVEL ink, on the same reasoning as a unit's:
    // the glyph sits inside this frame and the map shows through everything
    // around it, so a glyph painted straight onto the water lands at about
    // 1.07:1 and disappears. Filled from the closed polygon, because a
    // battle-dimension frame is open at one edge and its runs would leak the
    // fill out of the notch.
    let interior = frame_polygon(frame, (c.x as f64, c.y as f64), SYMBOL_BOX_PX);
    painter.add(egui::Shape::convex_polygon(
        interior
            .into_iter()
            .map(|(x, y)| egui::pos2(x as f32, y as f32))
            .collect(),
        darkened_ink(symbol.level_ink),
        egui::Stroke::NONE,
    ));
    for run in frame_strokes(dimension, frame, (c.x as f64, c.y as f64), SYMBOL_BOX_PX) {
        // Cased exactly as a unit's frame is, and for the same reason: a
        // group's outline sits on the same map and would otherwise be the same
        // 1.03:1 against it. A group reading as an outline while its members
        // read as solid objects is the inconsistency this whole pass exists to
        // remove.
        let points: Vec<egui::Pos2> = run
            .into_iter()
            .map(|(x, y)| egui::pos2(x as f32, y as f32))
            .collect();
        let cased = offset_run_outward(&points, c, FRAME_KEYLINE_OFFSET_PX);
        painter.add(egui::Shape::line(
            cased,
            egui::Stroke::new(FRAME_KEYLINE_PX, FRAME_KEYLINE),
        ));
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(FRAME_STROKE_PX, symbol.level_ink),
        ));
    }
    match symbol.icon {
        // An empty frame interior is a real state: these members do not
        // agree on one kind.
        None => {}
        Some(icon) => paint_map_symbol(
            painter,
            c,
            icon,
            false,
            frame_icon_radius(frame, SYMBOL_BOX_PX) as f32
                / (GLYPH_AUTHORING_RADIUS_PX as f32),
        ),
    }
    // `name (count)`, unchanged from the Flag: the theatre-zoom read the
    // operator has today. Whether it FITS is the one rule every group
    // text obeys, measured against the group's own ground extent.
    let extent_px = 2.0 * symbol.cover_radius_m / meters_per_pixel(symbol.lat, zoom);
    if !should_paint_group_text(extent_px, advance_width_px(ui, &symbol.label), focused) {
        return;
    }
    painter.text(
        c + egui::vec2(0.0, SYMBOL_BOX_PX as f32 * 0.5 + 3.0),
        egui::Align2::CENTER_TOP,
        &symbol.label,
        egui::FontId::proportional(MAP_LABEL_PX),
        symbol.level_ink,
    );
}

/// Map label ink size. One constant, because a fits test compares a
/// measured text against an extent and two font sizes would make the
/// comparison meaningless.
const MAP_LABEL_PX: f32 = 12.0;

/// The advance width of a string in the map label font.
///
/// The sum of glyph advances rather than a laid-out rect: it is what the
/// fits predicate consumes, it is monotonic in length, and it needs no
/// colour or wrap width to be meaningful. Measurement stays here, in the
/// paint layer, because egui's fonts need a context — the POLICY in
/// map_render.rs takes a plain number and is testable without one.
fn advance_width_px(ui: &egui::Ui, text: &str) -> f32 {
    let font = egui::FontId::proportional(MAP_LABEL_PX);
    ui.fonts_mut(|fonts| text.chars().map(|c| fonts.glyph_width(&font, c)).sum())
}

/// Whether the operator is holding this entity: selected, or followed.
fn entity_focused(selected: &Option<Selection>, following: Option<&String>, id: &str) -> bool {
    selected.as_ref() == Some(&Selection::Ship(id.to_string()))
        || selected.as_ref() == Some(&Selection::Group(id.to_string()))
        || following == Some(&id.to_string())
}

/// The ink an affiliation frame is drawn in.
///
/// The four canonical hues — blue friendly, red hostile, green neutral,
/// yellow unknown — used ONLY for units. Level colour belongs to groups,
/// so the two never share a hue channel; the disambiguator between a
/// green Unsur symbol and a green neutral hull is that a group far
/// symbol always carries its `name (count)` and a unit carries none.
///
/// Colour is the REDUNDANT cue here: the frame's shape is primary, so a
/// greyscale screenshot and a colour-blind operator still read
/// allegiance from the shape alone.
fn affiliation_ink(affiliation: tfg::store::Affiliation) -> egui::Color32 {
    match affiliation {
        tfg::store::Affiliation::Friendly => egui::Color32::from_rgb(0x4C, 0x9A, 0xFF),
        tfg::store::Affiliation::Hostile => egui::Color32::from_rgb(0xFF, 0x5A, 0x5A),
        tfg::store::Affiliation::Neutral => egui::Color32::from_rgb(0x5A, 0xD1, 0x7A),
        tfg::store::Affiliation::Unknown => egui::Color32::from_rgb(0xF5, 0xC2, 0x4B),
    }
}

/// Paint a unit's affiliation frame: the shape carries allegiance, the
/// fill and stroke its hue.
///
/// The frame's OUTLINE is never greyed for staleness — the glyph greys, the
/// frame keeps its hue, because "who stopped reporting" and "whose side" are
/// two different questions, and a stale hostile hull must still read as
/// hostile. The FILL does grey, which is not a contradiction: the outline is
/// the statement of allegiance and it stays true, while the fill is a large
/// area of colour that would otherwise keep shouting it. Status keeps the
/// rings and the halo.
fn paint_affiliation_frame(
    painter: &egui::Painter,
    center: egui::Pos2,
    affiliation: tfg::store::Affiliation,
    dimension: tfg::map_render::BattleDimension,
    planned: bool,
    stale: bool,
) -> SymbolFrame {
    let frame = frame_for(affiliation);
    let ink = affiliation_ink(affiliation);
    let stroke = egui::Stroke::new(FRAME_STROKE_PX, ink);
    // The interior is filled in the affiliation's hue, which is what the
    // removed base disc should have been doing. Filled from the CLOSED
    // polygon rather than from `frame_strokes`, because a battle-dimension
    // frame is open at one edge and its runs would leak fill out of the
    // notch: an aircraft's open-bottom rectangle would spill onto the map.
    // So openness stays a stroke property and the fill is always the whole
    // shape — which is correct, since a closed fill is what "interior" means
    // regardless of which edges are stroked.
    let interior = frame_polygon(frame, (center.x as f64, center.y as f64), SYMBOL_BOX_PX);
    let points: Vec<egui::Pos2> = interior
        .into_iter()
        .map(|(x, y)| egui::pos2(x as f32, y as f32))
        .collect();
    painter.add(egui::Shape::convex_polygon(
        points,
        affiliation_fill(affiliation, stale),
        egui::Stroke::NONE,
    ));
    for run in frame_strokes(
        dimension,
        frame,
        (center.x as f64, center.y as f64),
        SYMBOL_BOX_PX,
    ) {
        let points: Vec<egui::Pos2> = run
            .into_iter()
            .map(|(x, y)| egui::pos2(x as f32, y as f32))
            .collect();
        // A frame reads by its EDGE, and an outline has two: the fill on the
        // inside, the map on the outside. It is visible in proportion to the
        // WEAKER of the two, and the outside was the one at 1.03:1 — Unknown's
        // yellow on the sea is the sea. So the stroke is CASED: a wider dark
        // pass first, the hue on top. This is the ordinary cartographic answer
        // for a coloured line on busy ground, and it is why the outline reads
        // on water, on land and over a coastline without the hue having to be
        // dark enough to stop looking like itself.
        //
        // The case is drawn for the SAME runs as the hue, including the
        // planned dash, so the two never disagree about where the frame is.
        let keyline = egui::Stroke::new(
            FRAME_KEYLINE_PX,
            if stale { FRAME_KEYLINE_STALE } else { FRAME_KEYLINE },
        );
        // The casing is the same run pushed OUTWARD, never a wider centred
        // stroke: a centred wider stroke grows inward too and takes a
        // measurable bite out of the glyph.
        let cased = offset_run_outward(&points, center, FRAME_KEYLINE_OFFSET_PX);
        if planned {
            painter.add(egui::Shape::dashed_line(
                &cased,
                keyline,
                FRAME_DASH_PX,
                FRAME_DASH_GAP_PX,
            ));
            painter.add(egui::Shape::dashed_line(
                &points,
                stroke,
                FRAME_DASH_PX,
                FRAME_DASH_GAP_PX,
            ));
        } else {
            painter.add(egui::Shape::line(cased, keyline));
            painter.add(egui::Shape::line(points, stroke));
        }
    }
    frame
}

/// The dark casing drawn under a symbol frame's stroke.
///
/// Sits 8.02:1 against the map's water and 12.92:1 against its land, so the
/// edge it protects is legible on any terrain, and every affiliation's hue
/// clears 5.5:1 against it — so the hue is what an operator reads and the
/// casing is only ever a means of seeing it.
const FRAME_KEYLINE: egui::Color32 = egui::Color32::from_rgb(0x1c, 0x1c, 0x22);

/// A stale hull's casing greys a little, so a hull that has stopped reporting
/// does not keep a crisp edge. Deliberately NOT the full stale grey: the
/// outline is still the statement of allegiance and it stays true.
const FRAME_KEYLINE_STALE: egui::Color32 = egui::Color32::from_rgb(0x3a, 0x3a, 0x40);

/// The stroke width of the affiliation hue itself.
const FRAME_STROKE_PX: f32 = 1.5;

/// How far the casing extends OUTSIDE the hue, in px.
///
/// OUTSIDE, and this is the whole subtlety. A casing centred on the same path
/// as the stroke it backs — which is what "wider by N" means — eats N/2 px of
/// the frame's INTERIOR, and at this size that is not a rounding error: it
/// took 55 percent of the glyph's pixels off it (191 down to 87, measured on
/// the capture), because the glyph already reaches most of the way to the
/// frame. The casing exists to give the hue an edge against the MAP, which is
/// outside the frame, so it is placed entirely outside and the interior is
/// left alone.
///
/// 1.25 px of dark outside a 1.5 px hue is the sliver that does the work.
const FRAME_KEYLINE_OUT_PX: f32 = 1.25;

/// The casing's width: the hue, plus the sliver that shows on its outer side.
///
/// Drawn with an explicit offset rather than by being wider, because egui
/// strokes are centred on the path and a wider centred stroke necessarily
/// grows inward as well as outward.
const FRAME_KEYLINE_PX: f32 = FRAME_STROKE_PX + FRAME_KEYLINE_OUT_PX;

/// Where the casing's centre sits, in px outside the frame path.
const FRAME_KEYLINE_OFFSET_PX: f32 = (FRAME_KEYLINE_PX - FRAME_STROKE_PX) / 2.0;

/// The planned-status dash, in px, and the gap between dashes.
const FRAME_DASH_PX: f32 = 3.0;
const FRAME_DASH_GAP_PX: f32 = 2.5;

/// Push a run of points outward from the frame's centre by `offset` px.
///
/// Normalised, because a frame is not round: pushing a rectangle's corners by
/// a fixed number of pixels makes it a different, larger rectangle rather than
/// the same outline with a casing, and a diamond's vertices would visibly
/// separate from its edges. Scaling the direction by the distance to the centre
/// keeps the shape and grows it.
fn offset_run_outward(points: &[egui::Pos2], center: egui::Pos2, offset: f32) -> Vec<egui::Pos2> {
    points
        .iter()
        .map(|&p| {
            let d = p - center;
            let len = d.length();
            if len <= f32::EPSILON {
                p
            } else {
                p + d / len * offset
            }
        })
        .collect()
}

/// The redundant affiliation channel where the icon is a photograph.
///
/// A Middle/Near unit is a textured quad, and a hostile diamond cannot
/// wrap a rotated photograph — so at those two levels the hue carries
/// the identity instead, which is APP-6's own arrangement (fill colour
/// is a redundant indication of affiliation). Without it, affiliation
/// would vanish as the operator zooms in on a unit.
fn paint_affiliation_tint(
    painter: &egui::Painter,
    points: &[egui::Pos2],
    affiliation: tfg::store::Affiliation,
) {
    let ink = affiliation_ink(affiliation);
    painter.add(egui::Shape::convex_polygon(
        points.to_vec(),
        egui::Color32::from_rgba_unmultiplied(ink.r(), ink.g(), ink.b(), 46),
        egui::Stroke::new(1.5, ink),
    ));
}

/// The fill a Far symbol's frame interior carries, in the affiliation's own hue.
///
/// This is what the base disc was supposed to be and never was. The disc was
/// painted in `ship_color`, which matched only the literal ids `"nordwind"`
/// and `"ostsee"` — the sandbox fixture names — and returned ONE green for
/// every real hull. So it was not a channel at all: it carried no
/// information, and all it did was hide the glyph.
///
/// Filling the interior instead is the APP-6 arrangement the standard already
/// uses for affiliation ("fill colour is a redundant indication"), and it
/// gives the frame's SHAPE and its FILL the same meaning rather than leaving
/// the shape to carry affiliation alone.
///
/// DARKENED toward the map's own ink, and that is load-bearing rather than a
/// taste call. Measured against the map's own water and land:
///
///     affiliation ink     vs sea    vs land
///     Friendly              1.76       2.17
///     Hostile                1.89       2.33
///     Neutral                1.19       1.48
///     Unknown                1.02       1.26
///
/// Unknown at 1.02 is the sea, drawn on the sea. Mixed toward dark it becomes
/// 4.2–6.7 against the map and 6.8–9.8 against a white glyph, so the shape
/// survives a greyscale screenshot AND the symbol survives the map underneath
/// it. The type accents are in the same trap — every one of the ten sits
/// between 1.0 and 2.1 against sea and land — so they are used as the glyph's
/// outline-and-fill ON this fill, never against the map directly.
fn affiliation_fill(
    affiliation: tfg::store::Affiliation,
    stale: bool,
) -> egui::Color32 {
    if stale {
        // Stale greys the glyph and keeps a solid frame, so the fill greys
        // with it rather than staying saturated: a hull that has stopped
        // reporting should not keep advertising which side it is on.
        return egui::Color32::from_rgb(0x4a, 0x4a, 0x4a);
    }
    darkened_ink(affiliation_ink(affiliation))
}

/// The map's own darkest ink: what a fill is mixed toward so it survives being
/// drawn on top of water and land, both of which are pale.
const MAP_INK_DARK: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x1b, 0x2b);

/// An ink pulled 62 percent of the way toward [`MAP_INK_DARK`].
///
/// One function rather than a mix written out per caller, because the two
/// callers are a unit's affiliation fill and a group's level fill and they
/// must agree: a group that reads as a solid object while its members read as
/// outlines is the inconsistency this whole change exists to remove.
fn darkened_ink(ink: egui::Color32) -> egui::Color32 {
    const KEEP: f32 = 0.38;
    egui::Color32::from_rgb(
        (ink.r() as f32 * KEEP + MAP_INK_DARK.r() as f32 * (1.0 - KEEP)).round() as u8,
        (ink.g() as f32 * KEEP + MAP_INK_DARK.g() as f32 * (1.0 - KEEP)).round() as u8,
        (ink.b() as f32 * KEEP + MAP_INK_DARK.b() as f32 * (1.0 - KEEP)).round() as u8,
    )
}

/// The member symbol a group draws, if its members agree on one.
///
/// A strict MAJORITY, not a plurality and not the fattest member: one
/// destroyer in a force of twelve aircraft must not speak for it, which
/// is the same rule the affiliation resolver follows about declarations.
/// No majority paints an empty frame — a group that genuinely mixes
/// kinds is a container, and the empty interior says so instead of
/// naming a wrong kind.
fn plurality_icon(symbols: &[tfg::store::MapSymbol]) -> Option<tfg::store::MapSymbol> {
    if symbols.is_empty() {
        return None;
    }
    // Tallied by ordinal: `MapSymbol` is a plain enum with no Hash/Ord,
    // and the ordinal is its stable identity, so the tie-break below is
    // deterministic rather than dependent on iteration order.
    let mut tally: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
    for symbol in symbols {
        *tally.entry(*symbol as i32).or_insert(0) += 1;
    }
    tally
        .into_iter()
        .max_by_key(|(ordinal, count)| (*count, *ordinal))
        .and_then(|(ordinal, count)| {
            (count * 2 > symbols.len())
                .then(|| tfg::store::MapSymbol::from_ordinal(ordinal))
        })
}

/// Paint a taxonomy glyph inside a symbol's frame.
///
/// Every branch terminates in a shape, which is the visual half of the
/// universal-fallback contract: a symbol with no type assigned still draws
/// something rather than nothing.
///
/// `scale` is the glyph's reach relative to the frame it sits in, and it is
/// the SAME rule for the four frames on purpose — an icon FILLS its frame
/// rather than floating inside a border, so the diamond (which gives its
/// interior less room) gets a smaller glyph than the rectangle, from one
/// vocabulary and one authoring radius.
fn paint_map_symbol(
    painter: &egui::Painter,
    center: egui::Pos2,
    symbol: tfg::store::MapSymbol,
    stale: bool,
    scale: f32,
) {
    // The shape decides the ink; `scale` decides how far the glyph
    // reaches. The two are separate on purpose: an icon FILLS its frame
    // rather than floating inside a border, so the four frames need four
    // icon reaches from ONE glyph vocabulary.
    let shape = map_symbol_shape(symbol);
    let fill = map_symbol_fill(shape, stale);
    // Every literal below was drawn against the radius-8 circle the glyphs
    // were authored at, so `scale` is a multiplier on that reference rather
    // than a new radius to remember per glyph.
    let reach = |value: f32| value * scale;
    let polygon = |points: Vec<egui::Pos2>| {
        painter.add(egui::Shape::convex_polygon(points, fill, egui::Stroke::NONE));
    };
    match shape {
        MapSymbolShape::Dot => {
            painter.circle_filled(center, reach(3.0), fill);
        }
        MapSymbolShape::Destroyer => {
            polygon(regular_polygon(center, reach(5.5), 3, -std::f32::consts::FRAC_PI_2));
        }
        MapSymbolShape::Frigate => {
            polygon(regular_polygon(center, reach(5.2), 4, 0.0));
        }
        MapSymbolShape::Corvette => {
            polygon(regular_polygon(
                center,
                reach(4.5),
                4,
                std::f32::consts::FRAC_PI_4,
            ));
        }
        MapSymbolShape::Auxiliary => {
            painter.rect_filled(
                egui::Rect::from_center_size(center, egui::vec2(reach(9.0), reach(3.0))),
                1.0,
                fill,
            );
            painter.rect_filled(
                egui::Rect::from_center_size(center, egui::vec2(reach(3.0), reach(9.0))),
                1.0,
                fill,
            );
        }
        MapSymbolShape::Landing => {
            polygon(regular_polygon(
                center,
                reach(5.5),
                5,
                -std::f32::consts::FRAC_PI_2,
            ));
        }
        MapSymbolShape::Submarine => {
            let points = regular_polygon(center, reach(1.0), 16, 0.0)
                .into_iter()
                .map(|point| {
                    center + egui::vec2((point.x - center.x) * 1.5, (point.y - center.y) * 0.65)
                })
                .collect();
            polygon(points);
        }
        MapSymbolShape::Plane => {
            polygon(vec![
                center + egui::vec2(reach(6.0), 0.0),
                center + egui::vec2(-reach(4.0), -reach(4.5)),
                center + egui::vec2(-reach(4.0), reach(4.5)),
            ]);
        }
        MapSymbolShape::Tank => {
            painter.rect_filled(
                egui::Rect::from_center_size(
                    center + egui::vec2(-reach(4.0), 0.0),
                    egui::vec2(reach(1.5), reach(8.0)),
                ),
                1.0,
                fill,
            );
            painter.rect_filled(
                egui::Rect::from_center_size(
                    center + egui::vec2(reach(4.0), 0.0),
                    egui::vec2(reach(1.5), reach(8.0)),
                ),
                1.0,
                fill,
            );
            painter.rect_filled(
                egui::Rect::from_center_size(center, egui::vec2(reach(6.0), reach(4.0))),
                1.0,
                fill,
            );
        }
        MapSymbolShape::Port => {
            painter.circle_stroke(center, reach(5.0), egui::Stroke::new(2.0 * scale, fill));
            painter.circle_filled(center, reach(1.5), fill);
        }
    }
}

fn unit_image_points(
    marker: &ShipMarker,
    zoom: f64,
    pixels_per_point: f32,
    origin: egui::Pos2,
) -> Option<Vec<egui::Pos2>> {
    if marker.lod == UnitLod::Far {
        return None;
    }
    let visual = marker.visual.as_ref().filter(|v| v.is_map_renderable())?;
    let mut geometry =
        projected_unit_geometry(marker.latitude, zoom, visual.loa_m, visual.beam_m);
    if geometry.has_scale {
        let ppp = pixels_per_point.max(0.1) as f64;
        geometry.length_px /= ppp;
        geometry.beam_px /= ppp;
    }
    let (length_px, beam_px) = if geometry.has_scale {
        (geometry.length_px, geometry.beam_px)
    } else {
        // Backend geometry is still in development. Until it arrives,
        // use the explicit screen-space fallback requested for visual
        // verification; this never claims to be true-to-scale.
        fallback_unit_geometry(visual)
    };
    let center = (
        (origin.x + marker.x as f32) as f64,
        (origin.y + marker.y as f32) as f64,
    );
    let quad = rotated_unit_quad_with_forward_heading(
        center,
        length_px,
        beam_px,
        marker.heading_deg,
        visual.forward_heading_deg,
    )?;
    Some(
        quad.corners
            .iter()
            .map(|(x, y)| egui::pos2(*x as f32, *y as f32))
            .collect(),
    )
}

/// The polygon a marker is picked by: exactly what it paints, grown to
/// the minimum target if it is smaller than that.
///
/// The Representation decides the shape — the rotated quad where a
/// photograph or silhouette is drawn, the affiliation frame's box where
/// a glyph is — and the same `hit_polygon` dilation serves both, so
/// there is one hit rule rather than one per representation. A polyline
/// of 6 px is unfriendly to a mouse; a photograph's target is the
/// photograph.
fn marker_hit_points(
    marker: &ShipMarker,
    zoom: f64,
    pixels_per_point: f32,
) -> Vec<(f64, f64)> {
    let origin = egui::pos2(0.0, 0.0);
    if let Some(quad) = unit_image_points(marker, zoom, pixels_per_point, origin) {
        let quad: Vec<(f64, f64)> = quad
            .iter()
            .map(|p| (p.x as f64, p.y as f64))
            .collect();
        return hit_polygon(&quad, MIN_HIT_PX);
    }
    // The painted frames have DIFFERENT bounding boxes by design (an
    // inscribed square is smaller than the rectangle that fills it), so
    // the hit region is the symbol BOX for every affiliation — which is
    // what keeps a hull equally clickable whichever way its allegiance
    // changes.
    let box_poly = frame_polygon(SymbolFrame::Rectangle, (marker.x, marker.y), SYMBOL_BOX_PX);
    hit_polygon(&box_poly, MIN_HIT_PX)
}

fn marker_body_hit(
    marker: &ShipMarker,
    px: f64,
    py: f64,
    zoom: f64,
    pixels_per_point: f32,
) -> bool {
    in_poly(px, py, &marker_hit_points(marker, zoom, pixels_per_point))
}

/// Pick order: the Representation that is PAINTED LAST is the one on
/// top, so it is picked first. The body layer paints Far, then Middle,
/// then Near; this walks the same list backwards. Last-painted is what
/// the operator believes is covering what is under it — and the old code
/// took the FIRST painted marker, which is the one they cannot see when
/// near-zoom thumbnails overlap.
fn paint_order(markers: &[ShipMarker]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..markers.len()).collect();
    // Stable, so markers of equal Representation keep their list order.
    order.sort_by_key(|&i| match markers[i].lod {
        UnitLod::Far => 0,
        UnitLod::Middle => 1,
        UnitLod::Near => 2,
    });
    order
}

/// Click a Zone's OUTLINE, or the name written on it — never its
/// interior. The interior of a Zone is ground: in a client where
/// clicking the map places a unit and clicking empty water clears the
/// selection, letting a translucent overlay capture both is a bug
/// wearing a feature's clothes. Units still beat a Zone's outline.
fn zone_outline_hit(
    px: f64,
    py: f64,
    pts: &[(f32, f32)],
    label_c: (f64, f64),
    label_half_w: f64,
) -> bool {
    // A zero half-width means the group painted no name (the label
    // policy suppressed it), not that there is a zero-width target
    // sitting on the centroid catching everything through it.
    if label_half_w > 0.0
        && label_c.0 - label_half_w <= px
        && px <= label_c.0 + label_half_w
        && label_c.1 - 9.0 <= py
        && py <= label_c.1 + 9.0
    {
        return true;
    }
    if pts.len() < 2 {
        return false;
    }
    // Indexed rather than `windows(2)`: a polygon's edges wrap, and a
    // window pair walk silently leaves the LAST edge unpickable — the
    // closing edge of a Zone is exactly where an operator clicks when
    // tracing a formation's boundary.
    (0..pts.len()).any(|i| {
        distance_to_segment(px, py, pts[i], pts[(i + 1) % pts.len()]) <= ZONE_HIT_BAND_PX
    })
}

fn distance_to_segment(px: f64, py: f64, a: (f32, f32), b: (f32, f32)) -> f64 {
    let (ax, ay) = (a.0 as f64, a.1 as f64);
    let (bx, by) = (b.0 as f64, b.1 as f64);
    let (dx, dy) = (bx - ax, by - ay);
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-9 {
        return (px - ax).hypot(py - ay);
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len_sq).clamp(0.0, 1.0);
    (px - (ax + t * dx)).hypot(py - (ay + t * dy))
}

fn paint_unit_image(
    painter: &egui::Painter,
    marker: &ShipMarker,
    zoom: f64,
    pixels_per_point: f32,
    origin: egui::Pos2,
) -> Option<Vec<egui::Pos2>> {
    let points = unit_image_points(marker, zoom, pixels_per_point, origin)?;
    let visual = marker.visual.as_ref().expect("image points imply a visual");
    let texture = visual.texture.as_ref().expect("renderable visual has a texture");
    let tint = if marker.stale {
        egui::Color32::GRAY
    } else {
        egui::Color32::WHITE
    };
    let mut mesh = egui::Mesh::with_texture(texture.id);
    // The source bow is the image's +X edge, so the bow corners use
    // the right-hand UVs and the stern corners the left-hand UVs.
    let uvs = [
        egui::pos2(1.0, 0.0),
        egui::pos2(1.0, 1.0),
        egui::pos2(0.0, 1.0),
        egui::pos2(0.0, 0.0),
    ];
    for (point, uv) in points.iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: *point,
            uv,
            color: tint,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(mesh);
    Some(points)
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
/// Carries the group id so a Zone can be selected from the map.
struct ZoneGeom {
    group: String,
    pts: Vec<(f32, f32)>,
    fill: egui::Color32,
    stroke: egui::Color32,
    /// `name (count)` — the same string a far symbol carries, so a
    /// group reads the same in both representations.
    label: String,
    /// The hull's on-screen width: the extent a label is measured
    /// against, and the same measure a far symbol uses as twice its
    /// cover radius.
    width_px: f64,
}

/// Point-in-polygon over a zone's screen pts (ray cast). Degenerate
/// hulls (fewer than 3 pts) never hit: click the ship or flag instead.
fn in_poly(px: f64, py: f64, pts: &[(f64, f64)]) -> bool {
    if pts.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, yi) = (pts[i].0, pts[i].1);
        let (xj, yj) = (pts[j].0, pts[j].1);
        if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// A group drawn as a symbol rather than a Zone: the collapsed form of
/// the task organisation at theatre zoom.
///
/// This is what the Flag used to be, and the difference is the point: a
/// Flag was a name and a count, which is not a symbol. This one carries
/// allegiance in its frame SHAPE, its members' kind in the icon, and its
/// level in the stroke colour — three orthogonal channels, so a group
/// reads correctly without anyone being told which is which.
#[derive(Clone)]
struct GroupSymbolGeom {
    group: String,
    /// Screen centre: the ONE centroid the ladder measured from.
    x: f32,
    y: f32,
    /// The same centroid in lat/lon, for framing the camera on a click.
    lat: f64,
    lon: f64,
    /// `name (count)` — the Flag's content, unchanged. WHEN it paints is
    /// the label policy's business (issue #175), not this type's.
    label: String,
    affiliation: tfg::store::Affiliation,
    /// The plurality member symbol, or None for a group whose members do
    /// not agree on one. None paints an EMPTY FRAME, which reads as the
    /// container it is rather than as a confident wrong answer.
    icon: Option<tfg::store::MapSymbol>,
    /// Level colour on the stroke and fill tint — the channel a unit
    /// spends on affiliation instead, so the two never share a hue.
    level_ink: egui::Color32,
    /// Cover radius in metres, so framing a group on a click aims at the
    /// zoom where it earns a Zone rather than at a constant.
    cover_radius_m: f64,
}

/// One blocking REST action off the egui thread (M7): the worker runs
/// the closure to completion; the UI harvests the result per frame and
/// applies it. Errors cross as text — typed matching stays inside the
/// worker where the variants exist. One op of each kind at a time:
/// re-clicks while busy are refused loudly instead of piling threads.
struct RestOp<T, E = String> {
    label: &'static str,
    rx: mpsc::Receiver<Result<T, E>>,
}

fn spawn_rest<T, E, F>(label: &'static str, f: F) -> RestOp<T, E>
where
    T: Send + 'static,
    E: Send + 'static,
    F: FnOnce() -> Result<T, E> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    RestOp { label, rx }
}

impl<T, E> RestOp<T, E> {
    /// Non-blocking harvest: Some(result) exactly once, when done.
    /// A dead worker never resolves — loud in the terminal, and the
    /// slot stays busy rather than reporting a fabricated result.
    fn poll(&self) -> Option<Result<T, E>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                eprintln!("{} worker died without answering", self.label);
                None
            }
        }
    }
}

struct PlotDone {
    game_id: i64,
    result: Result<tfg::backend::PositionList, String>,
}

struct PlotSlot {
    game_id: i64,
    op: RestOp<PlotDone>,
}

/// Finished sign-in (M7): the worker ran login and the gate probe.
/// `uid` is None when the courtesy flag short-circuits the probe or
/// the probe itself failed (note); the pair is stored either way the
/// login succeeded, mirroring the old inline flow.
struct LoginDone {
    user: String,
    pair: tfg::backend::TokenPair,
    /// The whole `/users/me` projection, when the gate probe ran.
    ///
    /// It used to ride back as a bare uid plus a `Vec<i64>` of role ids,
    /// which is what made the Operator island print `APP ROLE 1`. The
    /// response is one object; carrying two slices of it and re-deriving the
    /// rest is how the display half got lost.
    identity: Option<tfg::backend::AuthenticatedUser>,
    needs_change: bool,
    probe_note: Option<String>,
}

/// What a book read or write answered with.
///
/// A step write answers with the whole scenario and a book read with the
/// whole book, so one slot has to carry both. `One` is spliced into the
/// local list rather than replacing it: replacing would drop every OTHER
/// scenario the author was not looking at.
enum Book {
    List(Vec<tfg::backend::GameScenario>),
    One(tfg::backend::GameScenario),
}

/// Whether a game role may need a fleet before the exercise can run.
///
/// TRUE for everything that is not judge-side, because that is all the
/// helpers mirror can say. There is no positive "requires a command" flag on
/// a game role — the schema carries `is_judge_side` and nothing else — so the
/// honest form of this question is "is this the judge side", and the answer
/// for the rest is a warning rather than a block.
///
/// A free function because the rule is pure and takes only the flag: the note
/// appears in two places (the seat form and the roster row) and one answer
/// served from two places is two answers waiting to disagree.
fn role_may_need_command(is_judge_side: bool) -> bool {
    !is_judge_side
}
/// Why the last readiness read produced no verdict.
///
/// THREE states, because "unknown" was two different situations and only one of
/// them was worth retrying. `Absent` is the one that matters: a server that
/// answers 404 does not publish the check at all, and "retry the sync" sends the
/// operator into a loop that cannot succeed. That is not a degraded read, it is
/// a client ahead of its server, and it is a fact about the SERVER rather than
/// about the connection, so it wants a different sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ReadinessGap {
    /// Nothing read yet. Ordinary on first paint.
    #[default]
    NotRead,
    /// The read failed for a reason another attempt might fix.
    Unread,
    /// The server answered 404: it has no readiness route on this path.
    Absent,
}

impl ReadinessGap {
    /// Classify a failed read. A 404 is not a transport hiccup, and treating it
    /// as one is what produced impossible advice.
    fn of(e: &tfg::backend::BackendError) -> Self {
        match e {
            tfg::backend::BackendError::Api { status: 404, .. } => Self::Absent,
            _ => Self::Unread,
        }
    }
}

/// What to say when the gate is unknown.
///
/// NEVER an empty list. Empty means "nothing is outstanding", and that is the
/// one answer that must never be invented — this runs before the first bundle
/// lands and after a failed read.
///
/// The three states are three different facts, not one fact with three phrasings:
/// nothing has run yet, something failed and may work next time, or this server
/// does not have the endpoint and never will without a deploy. Only the middle
/// one earns the word "retry".
///
/// A free function because the rule is pure and the app is not, and because
/// this is the branch where a deleted rule would quietly come back: it is the
/// one place tempted to work the gate out from what this client happens to hold.
fn unknown_gate_blockers(gap: ReadinessGap) -> Vec<String> {
    vec![match gap {
        ReadinessGap::NotRead => "the readiness check has not run yet".to_string(),
        ReadinessGap::Unread => "the readiness check could not be read — retry the sync".to_string(),
        ReadinessGap::Absent => {
            "this server does not publish the readiness check".to_string()
        }
    }]
}

/// Put a font that covers the UI's symbols behind the proportional face.
///
/// egui resolves a glyph by walking a family's font list in order, and its
/// default `Proportional` family is `[Ubuntu-Light, NotoEmoji, emoji-icon]` —
/// with Hack omitted. Hack is the face that actually carries the arrows: read
/// out of the two cmaps, Ubuntu-Light covers none of U+2192, U+2190, U+25B8,
/// U+25CF or U+221A, and Hack covers all of them. epaint's own `Monospace`
/// family already lists Hack second with the comment "fallback for √ etc",
/// which is this fix, applied to the wrong family.
///
/// So every arrow drawn in the default style was a `.notdef` box, on 44
/// button labels, and no test can see it: the string in the source is correct
/// and the glyph atlas lookup succeeds, returning the missing-glyph box.
///
/// The bytes are already in `font_data` under the name "Hack", so this appends
/// a NAME. No vendored font, no new dependency, and Latin text still comes from
/// Ubuntu-Light because it stays first in the list.
fn add_symbol_fallbacks(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let family = fonts.families.entry(egui::FontFamily::Proportional).or_default();
    if !family.iter().any(|f| f == "Hack") {
        family.push("Hack".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// Whether a release at `pos` should land on the map, given the modal panels
/// currently on screen.
///
/// The map's rect is full-bleed and therefore CONTAINS any floating zone, so
/// "released over the map" is not on its own enough: a release over a form
/// would place at whatever map point happens to be behind it.
///
/// A LIST, not one rect, because two modals each own a drag-to-map — the
/// Fleet picker for hulls and the Player picker for people — and a single slot
/// would mean whichever drew last silently disarmed the other.
///
/// A free function rather than a method because the rule is pure and the app
/// is not. A rule only checkable by standing up a store connection, a core
/// map and a token is a rule that does not get checked.
fn drop_lands_on_map(panels: &[egui::Rect], pos: egui::Pos2) -> bool {
    !panels.iter().any(|panel| panel.contains(pos))
}

/// A step being written, before the server has given it an id.
///
/// Held as a draft rather than written straight through because the backend
/// assigns ids and answers with the whole scenario: a form that only exists
/// after a round trip is a form the author cannot see their cursor in.
#[derive(Default, Clone)]
struct ComposerDraft {
    content: String,
    /// `HHMM`, validated by `tfg::backend::hhmm_ok` on the way out.
    start_hour: String,
    end_hour: String,
}

impl ComposerDraft {
    fn window(&self) -> Option<(&str, &str)> {
        (!self.start_hour.is_empty() && !self.end_hour.is_empty())
            .then_some((self.start_hour.as_str(), self.end_hour.as_str()))
    }

    /// What is wrong with the draft, if anything.
    ///
    /// Half a window is the case that matters: the backend refuses it, and
    /// the refusal arrives after the author has finished typing rather than
    /// while they are still in the field.
    fn problem(&self) -> Option<String> {
        match (self.start_hour.is_empty(), self.end_hour.is_empty()) {
            (false, true) => Some("a window needs an end as well as a start".into()),
            (true, false) => Some("a window needs a start as well as an end".into()),
            (false, false) => {
                if !tfg::backend::hhmm_ok(&self.start_hour) {
                    return Some(format!("{} is not a military time like 1000", self.start_hour));
                }
                if !tfg::backend::hhmm_ok(&self.end_hour) {
                    return Some(format!("{} is not a military time like 1030", self.end_hour));
                }
                if !tfg::backend::hhmm_window_ok(&self.start_hour, &self.end_hour) {
                    return Some("a window's end must be after its start".into());
                }
                None
            }
            (true, true) => None,
        }
    }
}

/// Finished sync (M7): the store connection rides back (it moved
/// into the worker); counts render the status line.
struct SyncDone {
    conn: rusqlite::Connection,
    counts: Vec<(String, usize)>,
}

/// Finished spec backfill (M7): connection rides back; specs upsert
/// into both catalogs on the UI thread.
struct SpecDone {
    conn: rusqlite::Connection,
    specs: Vec<tfg::backend::HullSpec>,
    fetched: usize,
    skipped: usize,
    failed: usize,
}

/// One bundled held-game read: detail is authoritative and always
/// applies; each subordinate rides its own Result so one staff-only
/// 403 degrades to a labeled gap instead of failing the whole
/// bundle. A refused subordinate keeps the last good list — never an
/// empty list presented as truth.
struct GameBundle {
    detail: tfg::backend::GameDetail,
    roster: Result<Vec<tfg::backend::Participant>, tfg::backend::BackendError>,
    units: Result<Vec<tfg::backend::GameUnit>, tfg::backend::BackendError>,
    placements: Result<tfg::backend::PlacementList, tfg::backend::BackendError>,
    /// The server's verdict on the execution gate.
    readiness: Result<tfg::backend::GameReadinessView, tfg::backend::BackendError>,
}

/// What kind of asset a visual carries.
///
/// Minos publishes this discriminator in the manifest so a future
/// archive kind can be added without making old clients guess. The
/// current `unit-images` bundle speaks `unit_image`; anything else is
/// retained as unsupported and never drawn as a hull photograph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssetKind {
    UnitImage,
    Unsupported,
    /// No asset for this unit in the active manifest version. This is
    /// settled absence, not an unresolved fetch.
    Unavailable,
}

impl AssetKind {
    fn from_manifest(value: &str) -> Self {
        match value {
            "unit_image" => AssetKind::UnitImage,
            _ => AssetKind::Unsupported,
        }
    }
}

/// Pause before asking again after a failed manifest or source read.
/// A null presign does not erase a real manifest entry, and a refused
/// request does not deserve an every-frame retry storm.
const IMAGE_READ_RETRY_DELAY: Duration = Duration::from_secs(30);
/// Re-read the manifest periodically so a picture added after the first
/// read can enter the versioned visual cache without restarting the app.
const MANIFEST_REFRESH_INTERVAL: Duration = Duration::from_secs(15);

/// Read the standard S3 presign lifetime from the source URI when it
/// is present. A local archive path has no expiry and returns `None`.
fn image_source_expires_at(source: &str) -> Option<Instant> {
    let query = source.split_once('?')?.1;
    let seconds = query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .find(|(key, _)| *key == "X-Amz-Expires")?
        .1
        .parse::<u64>()
        .ok()?;
    Instant::now().checked_add(Duration::from_secs(seconds))
}

/// One unit's visual state: what it is drawn as, and everything the
/// renderer needs to draw it true-to-scale.
///
/// Deliberately NOT a `HashMap<i64, Option<String>>`. That could not
/// say whether a unit had no image or had not been fetched yet, could
/// not carry the measurements that make a thumbnail honest, and could
/// not survive a manifest change without a full rebuild. This is the
/// whole visual truth for one hull, in one value.
///
/// `image_url` is a PRESIGNED, EXPIRY-BEARING address: it lives here
/// in memory, never in SQLite, and `asset_version` (the manifest's
/// ETag) is what identifies the asset, not the URL.
#[derive(Clone)]
struct UnitVisual {
    unit_id: i64,
    /// Temporary source URI, valid only while its presign lives (or
    /// indefinitely for a later local archive path). None means no
    /// address is currently held.
    image_url: Option<String>,
    /// When a presigned source is known to expire. The texture is the
    /// durable result; the URL is only a way to obtain it.
    image_url_expires_at: Option<Instant>,
    /// A null presign is a failed read, not proof that the manifest
    /// entry was imaginary. Retry after a pause rather than every frame.
    image_url_retry_at: Option<Instant>,
    /// Decoded, GPU-resident image identity once egui's loader has
    /// produced it. `SizedTexture` is the renderer's actual result;
    /// a `TextureHandle` is loader-internal and is not returned by
    /// `try_load_texture`.
    texture: Option<egui::load::SizedTexture>,
    /// Intrinsic dimensions as Minos measured them, before decoding.
    width_px: Option<u32>,
    height_px: Option<u32>,
    /// The manifest's declared media type. It is diagnostic metadata,
    /// not a second source: the map and Inspector both decode the same
    /// presigned object URL.
    content_type: String,
    /// Real-world measurements, straight from the published spec.
    /// None means unpublished; the renderer must not substitute.
    loa_m: Option<f64>,
    beam_m: Option<f64>,
    draft_m: Option<f64>,
    /// Which way this unit's image points, in compass degrees.
    /// The client constant today; backend metadata overrides it when
    /// Minos publishes orientation.
    forward_heading_deg: f32,
    /// The manifest ETag this visual was resolved against.
    /// A change invalidates the picture, not the unit's identity.
    asset_version: String,
    asset_kind: AssetKind,
    map_symbol: tfg::store::MapSymbol,
}

impl std::fmt::Debug for UnitVisual {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Keep the source address redacted even though the decoded
        // texture identity is safe to name.
        f.debug_struct("UnitVisual")
            .field("unit_id", &self.unit_id)
            .field("image_url", &self.image_url.as_ref().map(|_| "<source>"))
            .field("url_expired", &self.image_url_expires_at.is_some_and(|at| at <= Instant::now()))
            .field("url_retry_pending", &self.image_url_retry_at)
            .field("texture", &self.texture.as_ref().map(|_| "<tex>"))
            .field("size_px", &(self.width_px, self.height_px))
            .field("content_type", &self.content_type)
            .field("loa_m", &self.loa_m)
            .field("beam_m", &self.beam_m)
            .field("forward_heading_deg", &self.forward_heading_deg)
            .field("asset_version", &self.asset_version)
            .field("asset_kind", &self.asset_kind)
            .field("map_symbol", &self.map_symbol)
            .finish()
    }
}

impl UnitVisual {
    /// A unit with no image: it draws a symbol, and the cache records
    /// that as a settled fact rather than an unanswered question.
    ///
    /// It still belongs to a manifest version. A hull that has no
    /// picture in v1 may have one in v2, so "absent" is versioned just
    /// like "present"; it is not a permanent property of the unit.
    fn unavailable(
        unit_id: i64,
        asset_version: &str,
        symbol: tfg::store::MapSymbol,
    ) -> Self {
        UnitVisual {
            unit_id,
            image_url: None,
            image_url_expires_at: None,
            image_url_retry_at: None,
            texture: None,
            width_px: None,
            height_px: None,
            content_type: String::new(),
            loa_m: None,
            beam_m: None,
            draft_m: None,
            forward_heading_deg: tfg::map_render::IMAGE_FORWARD_HEADING_DEG,
            asset_version: asset_version.to_string(),
            asset_kind: AssetKind::Unavailable,
            map_symbol: symbol,
        }
    }

    /// From a manifest entry, before any bytes are fetched. Intrinsic
    /// image dimensions and optional physical measurements are already
    /// known here and are carried exactly as Minos published them.
    fn from_entry(
        entry: &tfg::backend::UnitImageEntry,
        asset_version: &str,
        symbol: tfg::store::MapSymbol,
    ) -> Self {
        UnitVisual {
            unit_id: entry.unit_id,
            image_url: None,
            image_url_expires_at: None,
            image_url_retry_at: None,
            texture: None,
            width_px: entry.width_px,
            height_px: entry.height_px,
            content_type: entry.content_type.clone(),
            loa_m: entry.loa_m,
            beam_m: entry.beam_m,
            draft_m: None,
            forward_heading_deg: tfg::map_render::IMAGE_FORWARD_HEADING_DEG,
            asset_version: asset_version.to_string(),
            asset_kind: AssetKind::from_manifest(&entry.asset_kind),
            map_symbol: symbol,
        }
    }

    /// True when this unit may be drawn as a map thumbnail. Requires
    /// a supported unit-image asset and a decoded texture — a
    /// renderer reaching a unit mid-fetch gets a symbol instead of a
    /// hole.
    fn is_map_renderable(&self) -> bool {
        self.asset_kind == AssetKind::UnitImage && self.texture.is_some()
    }
}

/// Cache key: the unit plus the manifest ETag that says
/// what its picture is. The URL is deliberately absent — a presigned
/// address expires and changes without changing the asset.
type VisualKey = (i64, String);

/// Every unit's visual state for every asset version still held.
///
/// ONE cache serves both delivery modes: online it holds presigned
/// URLs, and the archive reader will later put extracted local paths
/// into the same entries. The key is `(unit_id, asset_version)`, so
/// the same hull under a new manifest can never inherit an old
/// picture by accident.
#[derive(Default)]
struct VisualCache {
    units: std::collections::HashMap<VisualKey, UnitVisual>,
    /// The manifest ETag currently selected by `get` and
    /// mutation helpers. A separate bool records that a manifest was
    /// read: an absent backend ETag must not become an invitation to
    /// fetch the manifest forever.
    asset_version: String,
    manifest_loaded: bool,
}

impl VisualCache {
    fn active_key(&self, unit_id: i64) -> VisualKey {
        (unit_id, self.asset_version.clone())
    }

    /// The visual for a unit at the active manifest version, if one
    /// has been resolved.
    fn get(&self, unit_id: i64) -> Option<&UnitVisual> {
        self.units.get(&self.active_key(unit_id))
    }

    /// True when the unit's state is settled. An absent picture is a
    /// real `UnitVisual` with `AssetKind::Unavailable`, so this also
    /// distinguishes "the manifest says no" from "not fetched yet".
    fn is_settled(&self, unit_id: i64) -> bool {
        self.units.contains_key(&self.active_key(unit_id))
    }

    /// True while this visual can still benefit from acquiring its
    /// unit-image source. Timing (live, retrying, or due) is separate.
    fn expects_source(&self, unit_id: i64) -> bool {
        self.get(unit_id).is_some_and(|v| {
            v.asset_kind == AssetKind::UnitImage && v.texture.is_none()
        })
    }

    /// Bound retries after a transport failure without changing the
    /// manifest's claim that this unit has a picture.
    fn defer_source(&mut self, unit_id: i64) {
        if let Some(v) = self.units.get_mut(&self.active_key(unit_id)) {
            v.image_url_retry_at = Some(Instant::now() + IMAGE_READ_RETRY_DELAY);
        }
    }

    /// True when a unit-image still needs a usable source address.
    /// A loaded texture needs no URL, unsupported and absent assets do
    /// not, and a null presign waits for a bounded retry.
    fn needs_url(&self, unit_id: i64) -> bool {
        let Some(visual) = self.get(unit_id) else {
            return false;
        };
        if visual.asset_kind != AssetKind::UnitImage || visual.texture.is_some() {
            return false;
        }
        if visual
            .image_url_retry_at
            .is_some_and(|retry_at| retry_at > Instant::now())
        {
            return false;
        }
        match &visual.image_url {
            Some(_) => visual
                .image_url_expires_at
                .is_some_and(|expires_at| expires_at <= Instant::now()),
            None => true,
        }
    }

    /// Record a unit as having no picture in the active manifest.
    /// Cached under that version, so the manifest is not re-asked on
    /// every frame for a hull that will never have one.
    fn mark_absent(&mut self, unit_id: i64, symbol: tfg::store::MapSymbol) {
        let version = self.asset_version.clone();
        let mut visual = UnitVisual::unavailable(unit_id, &version, symbol);
        if let Some(previous) = self.get(unit_id) {
            // The specification did not change merely because the
            // image did. Preserve unpublished-versus-published truth.
            visual.loa_m = previous.loa_m;
            visual.beam_m = previous.beam_m;
            visual.draft_m = previous.draft_m;
        }
        self.units.insert((unit_id, version), visual);
    }

    /// Install the manifest: every listed unit gets a visual seeded
    /// from its entry, and the version stamps them all.
    ///
    /// Returns the units whose URL still needs reading. Re-installing
    /// the same version is idempotent, including live URLs and loaded
    /// textures. A version change starts every picture again, while
    /// preserving the physical measurements, which belong to the unit
    /// specification rather than to the photograph.
    fn install_manifest(
        &mut self,
        manifest: &tfg::backend::ImageManifest,
        symbol_of: impl Fn(i64) -> tfg::store::MapSymbol,
    ) -> Vec<i64> {
        let same_version = self.manifest_loaded
            && !manifest.version.is_empty()
            && self.asset_version == manifest.version;
        // Snapshot before clearing: measurements belong to the unit,
        // so they survive a picture-version change even though URLs and
        // textures do not.
        let previous: std::collections::HashMap<i64, UnitVisual> =
            if self.manifest_loaded && !same_version {
                self.units
                    .iter()
                    .filter_map(|((id, _), visual)| Some((*id, visual.clone())))
                    .collect()
            } else {
                std::collections::HashMap::new()
            };
        if !same_version {
            // Old pictures and old absence are facts about another
            // asset version. Do not let a lookup by unit id find them.
            self.units.clear();
        }
        self.asset_version = manifest.version.clone();
        self.manifest_loaded = true;

        let mut needing_url = Vec::new();
        for entry in &manifest.entries {
            let key = (entry.unit_id, manifest.version.clone());
            if self.units.contains_key(&key) {
                // Same content version: keep the live URL, texture,
                // and dimensions. Re-reading the manifest is not a
                // reason to refetch every picture.
                continue;
            }
            let mut visual =
                UnitVisual::from_entry(entry, &manifest.version, symbol_of(entry.unit_id));
            if let Some(old) = previous.get(&entry.unit_id) {
                // A manifest may carry the active physical measurements
                // even when the local specification mirror does not. A
                // missing value in the new manifest must not erase the
                // last known value across an asset-version refresh.
                visual.loa_m = visual.loa_m.or(old.loa_m);
                visual.beam_m = visual.beam_m.or(old.beam_m);
                visual.draft_m = visual.draft_m.or(old.draft_m);
            }
            let needs_source = visual.asset_kind == AssetKind::UnitImage;
            self.units.insert(key, visual);
            if needs_source {
                needing_url.push(entry.unit_id);
            }
        }
        needing_url
    }

    /// Merge the unit facts that come from the local mirror. These
    /// are not picture bytes: measurements, taxonomy-derived symbol,
    /// and their version. A unit absent from the manifest gets a real
    /// `Unavailable` visual rather than a side-table exception, so all
    /// three renderer states travel through one model.
    fn set_unit_facts(
        &mut self,
        unit_id: i64,
        symbol: tfg::store::MapSymbol,
        loa_m: Option<f64>,
        beam_m: Option<f64>,
        draft_m: Option<f64>,
    ) {
        if !self.manifest_loaded {
            return;
        }
        let key = self.active_key(unit_id);
        let version = self.asset_version.clone();
        let visual = self
            .units
            .entry(key)
            .or_insert_with(|| UnitVisual::unavailable(unit_id, &version, symbol));
        visual.map_symbol = symbol;
        // The manifest is the current Minos truth when it carries a
        // measurement; the local mirror only fills a gap. A stale local
        // spec must not overwrite a value that arrived with this asset
        // version. A missing local row is not evidence that Minos
        // withdrew the manifest's measurement.
        if visual.loa_m.is_none() {
            visual.loa_m = loa_m;
        }
        if visual.beam_m.is_none() {
            visual.beam_m = beam_m;
        }
        if draft_m.is_some() {
            visual.draft_m = draft_m;
        }
    }

    /// Replace a unit's temporary URL, keeping every other fact. This
    /// is the presigned-expiry path: the same asset, a new address.
    fn set_url(&mut self, unit_id: i64, url: Option<String>) {
        if let Some(v) = self.units.get_mut(&self.active_key(unit_id)) {
            v.image_url_expires_at = url.as_deref().and_then(image_source_expires_at);
            v.image_url_retry_at = url
                .is_none()
                .then(|| Instant::now() + IMAGE_READ_RETRY_DELAY);
            v.image_url = url;
        }
    }

    /// Install a decoded texture.
    fn set_texture(&mut self, unit_id: i64, tex: egui::load::SizedTexture) {
        if let Some(v) = self.units.get_mut(&self.active_key(unit_id)) {
            v.texture = Some(tex);
        }
    }

    /// Drop everything. Called on sign-out, user change, new session
    /// and a held-game change: no image, texture, URL, or measurement
    /// from one user's exercise may survive into the next one's.
    fn clear(&mut self) {
        self.units.clear();
        self.asset_version.clear();
        self.manifest_loaded = false;
    }
}

/// Refresh kinds waiting on the setup slot (#100).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingRefresh {
    Games,
    Directory,
    Game,
    Tree,
}

/// What the in-flight picture slot is waiting for. Kept beside the
/// worker so a transport failure can be retried against the same
/// manifest or unit rather than restarting the wrong request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageRequest {
    Manifest,
    Url(i64),
}

/// One hull picture resolution (images ticket): the manifest once
/// per session, then one temporary URL per pictured hull. Both ride
/// a dedicated slot so picture fetches never queue behind setup.
enum ImageOut {
    Manifest(tfg::backend::ImageManifest),
    Url(i64, Option<String>),
}

/// A command result kept in the Inspector/Orders surface. The transport
/// worker classifies failures before they reach the UI; no raw error
/// string is mistaken for an accepted HelmOrder.
impl ShipApp {
    /// Record a placement outcome, where it will survive until it is read.
    ///
    /// Writes `users_status` as well, because that is what the rest of the app
    /// already shows — but `users_status` is a shared slot a background sync
    /// rewrites, so the copy in `placement_note` is the one that survives. See
    /// [`PlacementNote`].
    fn note_placement(&mut self, msg: impl Into<String>) {
        let msg = msg.into();
        self.placement_note = PlacementNote(msg.clone());
        self.users_status = msg;
    }
}

/// Why the last placement attempt ended the way it did.
///
/// SEPARATE from `users_status`, and it has to be. A placement outcome and a
/// register sync both used to write `users_status`, and the sync runs on a
/// timer — so the one line that explains why the hull did not appear got
/// overwritten by "pictures: 0 hull(s) listed" before the operator could read
/// it. The refusal was still happening; it was just gone by the time anyone
/// looked, which is exactly what makes a refusal read as a dead control.
///
/// Same slot, two unrelated facts, and the one that mattered lost every time.
#[derive(Default)]
struct PlacementNote(String);

/// Which modal is open. At most one at a time; see `ShipApp::open_only`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhichModal {
    Settings,
    Composer,
    Fleet,
    Player,
}

#[derive(Debug, Clone)]
struct UnitDrag {
    id: String,
    name: String,
    start: egui::Pos2,
    moved: bool,
}

/// A placed hull being dragged to a new starting position.
///
/// THE GESTURE IS A PRESS ON THE HULL, NOT A MODE. A double-click to arm a
/// move was the first shape considered and it is two gestures with an armed
/// state between them, which is a state to get stuck in and a cue to maintain;
/// picking the hull up is one gesture and the pointer already says which it is.
/// The cost is that a drag which starts on a hull stops panning the map — so
/// this is armed only in Setup, where the hull is a starting position and
/// moving it is a real edit, and never in Live, where that same spot on the map
/// is the heading handle and the hull's position is a fix rather than a claim.
#[derive(Debug, Clone)]
struct UnitMoveDrag {
    unit_id: i64,
    name: String,
    start: egui::Pos2,
    moved: bool,
}

#[derive(Debug, Clone)]
struct PickerRow {
    id: String,
    name: String,
    hull: String,
    class_name: String,
    stat_class: Option<String>,
    trail: String,
}

#[derive(Debug, Clone, Copy)]
struct HelmDraft {
    heading_deg: f32,
    speed_kn: f32,
}

#[derive(Debug, Clone, Copy)]
struct HelmSubmission {
    heading_deg: f32,
    speed_kn: f32,
    requester_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
enum HelmOrderUiResult {
    Draft,
    Pending,
    Accepted,
    Clamped { requested: f64, accepted: f64 },
    Unknown(String),
    Refused(String),
    Superseded,
}

enum OrderFailure {
    Refused { category: String, detail: String },
    Unknown(String),
}

/// One Minos order inside a batch (#100): the address plus its
/// outcome. Speeds stay f32 like the order drafts; the worker widens
/// to f64 at the POST.
struct OrderOut {
    ship: String,
    heading: f64,
    speed: f32,
    result: Result<tfg::backend::GameFix, OrderFailure>,
}

/// Every setup-flow read/write result (#100): one slot serializes
/// setup writes (no concurrent conflicting writes) and one pump arm
/// applies them. Notes render on the status line; multi-step reads
/// bundle like SyncDone carries its connection.
enum SetupDone {
    Games(Vec<tfg::backend::GameRow>),
    /// The game list needs a staff read the account lacks: not an
    /// error to retry loudly, but the player-flow signal — room key
    /// first, staff steps aside.
    GamesDenied,
    Users(Vec<tfg::backend::BackendUser>),
    Bundle(GameBundle),
    Roster(Vec<tfg::backend::Participant>, String),
    /// One force-sync write answered with the refreshed unit list.
    ForceUnits(Vec<tfg::backend::GameUnit>),
    /// One force-sync write answered with the refreshed setup view.
    ForcePlacements(tfg::backend::PlacementList),
    /// A force-sync write refused; the write is kept so the note names
    /// which hull it was about. The queue is dropped — the live mirror
    /// already carries the writes that succeeded, so the next attempt is
    /// a fresh diff, not a replay.
    ForceFailed(String, tfg::force::ForceWrite),
    Game(String, tfg::backend::GameRow),
    GameCreate(tfg::backend::GameRow, String),
    /// Admin game writes: the update answer is the re-read detail
    /// (applied like any bundle detail); a delete drops the hold,
    /// which then reads exactly like a vanished game.
    GameUpdated(tfg::backend::GameDetail),
    GameDeleted(i64, String),
    /// A refused transition (to, error, forbidden): the verdict waits    /// for the bundle re-read — maybe somebody else already moved the
    /// game. `forbidden` is typed at the call site (never sniffed from
    /// text): only the game's own Game Master advances it, and the bar
    /// must say so instead of repeating a bare 403.
    GameFailed(String, String, bool),
    Join(tfg::backend::JoinResult, String),
    /// A join refusal with its own next action: seatless 403 vs
    /// unknown-key 404 never share a line (audit item).
    JoinFailed(String),
    Clock(tfg::backend::GameClock, String),
    /// A refused clock write (pause/resume/factor): no probe endpoint
    /// exists — the 403 IS the capability answer, learned per hold
    /// like the transition verdict. Buttons gate on it, never on
    /// role names (judges may hold the grant, Game Masters may not).
    ClockDenied,
    FixBatch(Vec<OrderOut>),
    /// A loaded timeline page: replace or append. Failures keep the
    /// last good list and report loudly.
    Timeline(tfg::backend::TimelinePage, bool),
    /// Closure judgements: a loaded list, or a recorded mark (which
    /// chains into a reload like a sent message does).
    Judgements(Vec<tfg::backend::Judgement>),
    JudgementSent(tfg::backend::Judgement),
    /// Closure reviews: a loaded list, a filed document, or a revised
    /// one — the writes chain into a reload like sent messages do.
    Reviews(Vec<tfg::backend::Review>),
    ReviewSent(tfg::backend::Review),
    ReviewRevised(tfg::backend::Review),
    /// The task-organisation forest, or its read refusal. Forbidden
    /// degrades to a labeled gap; anything else reports and keeps.
    Hierarchy(Result<Vec<tfg::backend::HierarchyNode>, tfg::backend::BackendError>),
    /// One inbox page with navigation (messages ticket): replaces
    /// the list; failures keep the last good page loudly.
    MsgPage(tfg::backend::InboxPage),
    /// An opened message's detail, or a deleted one's id (the page
    /// reloads behind it so counts stay the server's).
    MsgOpen(tfg::backend::InboxMsg),
    MsgDeleted(i64),
    MsgSent(tfg::backend::InboxMsg),
    Roles(Vec<tfg::backend::ScenarioRole>),
    ReadDone(tfg::backend::InboxMsg),
}

/// Finished password change (M7): staged outcomes because each stage
/// applies differently — a changed password clears the form, a failed
/// re-entry signs out, a shut gate only reports.
enum PwResult {
    Changed(LoginDone),
    ChangeFailed(String),
    ReentryFailed,
    GateShut(String),
}

struct ShipApp {
    /// Installed/portable runtime locations. All mutable state is reached
    /// through this one object; no caller rebuilds a data-directory path.
    paths: AppPaths,
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
    /// Last arrival from the authoritative game-position WebSocket. A
    /// quiet stream falls back to the REST plot instead of leaving the
    /// ship stale for the full backoff interval.
    last_game_position_at: Option<Instant>,
    /// Presentation-only interpolation start per accepted ship. Keeping
    /// this per ship prevents an unrelated source or unit from resetting
    /// everyone else's glide.
    fix_animation_started: HashMap<String, Instant>,
    animation_paused_fraction: HashMap<String, f64>,
    /// Game samples whose arrival gap was too large to animate. Game
    /// timestamps are scenario time, so arrival cadence is tracked here
    /// instead of comparing assumed-time deltas.
    game_animation_snap: HashSet<String>,
    reconnect_snap: HashSet<String>,
    hidden: HashSet<String>,
    following: Option<String>,
    show_trail: bool,
    show_grid: bool,
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
    /// Execution windows (roster/orders/log): run-local visibility,
    /// opened by the start path, never by toolbar toggles.
    show_roster: bool,
    /// Island top-lefts, owned by [`tfg::chrome`] rather than by egui's
    /// Window memory (ADR-0014). An island positions itself, so its origin
    /// lives here and is written back after the island closes.
    roster_pos: egui::Pos2,
    /// The side zone (ADR-0016 / DESIGN.md "The three zones"): a column of
    /// islands down one edge, toggled as a unit.
    ///
    /// `show_side_zone` and `side_dock` are the whole of the zone's
    /// persistent state. Both are plain fields on purpose: the app has no
    /// storage backend (`eframe::NativeOptions` exposes none), so anything
    /// remembered here dies with the process. Writing that down is cheaper
    /// than a settings key that silently does nothing.
    show_side_zone: bool,
    side_dock: tfg::chrome::Dock,
    /// How far the side zone is scrolled, in points.
    ///
    /// The column is taller than the window in every state with four islands,
    /// so without this the tail of it is not merely inconvenient to reach, it
    /// is never drawn at all.
    zone_scroll: f32,
    /// The settings modal's visibility. A bool rather than egui memory
    /// because the modal holds a mode switch that has to be able to cancel.
    settings_open: bool,
    /// Reduced motion. Collapses every egui animation to its target by
    /// zeroing the animation clock, which is the one switch DESIGN.md
    /// promises rather than per-component.
    reduced_motion: bool,
    /// The animation budget in seconds when motion is allowed. One number
    /// for the whole console, so the taste is tunable in one place and the
    /// reduced-motion path is literally this set to zero.
    motion_secs: f32,
    inspector_pos: egui::Pos2,
    orders_pos: egui::Pos2,
    login_pos: egui::Pos2,
    log_pos: egui::Pos2,
    messages_pos: egui::Pos2,
    /// Eased camera (`src/camera.rs`). `center` is the live, displayed
    /// value and is rewritten from this once per frame; `center` stays the
    /// single thing every projection and hit-test reads, so the two never
    /// disagree — not even mid-glide, when the tile underneath is still
    /// catching up.
    glide: tfg::camera::Glide,
    /// Set when something changed that nothing else will wake us for: a
    /// channel pump consumed a value, or a hull is mid-glide. Drives the
    /// on-demand repaint in `ui`.
    dirty: bool,
    /// The shader seam. Carries the capability tier and egui's own target
    /// format, so the halo pipeline can only ever be built for the format
    /// it is drawn into. At `Quality::Low` it draws nothing and the map
    /// looks exactly as it did before this existed.
    fx: tfg::fx::Fx,
    show_orders: bool,
    show_log: bool,
    /// Onboarding (ticket #77): boots to State A (Login), through
    /// State B (Mode), into the shell — C for Presentation, D for
    /// Simulation. This is the one directing layer at first run.
    onboard: Onboard,
    /// Simulation four-phase gate (ticket #77): Planning → Persiapan
    /// is this flag; Eksekusi/Evaluasi ride the existing phase.
    sim_ready: bool,
    /// State C card (ticket #77): shown until dismissed or connected.
    connect_card: bool,
    /// Phase-bar refusal note (ticket #77): the start verb reports
    /// here because the event feed only renders Live.
    phase_note: Option<String>,
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
    /// Latest visible result for each ship. This is a UI projection,
    /// not a second authority; the MinOS response or local sim event
    /// replaces it.
    order_result: HashMap<String, HelmOrderUiResult>,
    helm_submissions: HashMap<String, HelmSubmission>,
    helm_drafts: HashMap<String, HelmDraft>,
    /// Accepted helm intents stay visually pinned to the commander draft
    /// until the authoritative position animation has completed.
    helm_preview_pending: HashSet<String>,
    controlled: HashSet<String>,
    pending_waypoint: Option<(f64, f64)>,
    placing: bool,
    order_speed: f32,
    /// Land test for order validation (ticket #22): land waypoints are
    /// rejected in the UI before they ever reach the sim.
    land: Option<Land>,
    /// Last order refusal from the sim, shown until the next attempt.
    order_warning: Option<String>,
    helm_warnings: HashMap<String, String>,
    /// Unit taxonomy (grill #18): class chosen at take-control.
    catalog: Catalog,
    selected_class: usize,
    /// Fleet seeds (task #29): organizer hulls from assets/fleet.json
    /// plus the synced register fallback (placement_seed) — placed by
    /// hand on the map, never automatically.
    fleet: Fleet,
    /// Map placement pick: a register hull awaiting its map click.
    fleet_pick: Option<String>,
    /// Exercise setup flow (#79): Planning in four steps — game,
    /// players, fleet, ready. One step renders at a time; the backend
    /// game in users_game is what every step reads and writes.
    setup_step: usize,
    /// Closure assessment workspace: selected slot tab — summary,
    /// timeline, judgements, reviews, transcript. Reset on new
    /// session and sign-out.
    assessment_tab: usize,
    /// New-game form (step 1): name is required, the rest optional.
    /// Mode is always maneuver (the only mode this release).
    setup_name: String,    setup_description: String,
    setup_purpose: String,
    setup_target: String,
    setup_area: String,
    setup_map_tag: String,
    /// Held-game edit form (admin ticket): blank means leave alone
    /// (absent on the wire) — the detail carries no text fields to
    /// prefill from. Separate drafts from create; delete arms on
    /// second click. Both hide without the staff read.
    edit_open: bool,
    delete_armed: bool,
    edit_name: String,
    edit_description: String,
    edit_purpose: String,
    edit_target: String,
    edit_area: String,
    edit_map_tag: String,
    /// Held-game edit form, the planning terms: pace and the planned window.
    /// Same blank-means-unchanged contract as the six prose rows, and
    /// deliberately not on the create form — see `edit_game_ui`.
    edit_pace: String,
    /// One date carries both ends of the real window; the exercise clock has
    /// none of its own (see `assumed_hhmm_to_rfc3339`).
    edit_actual_date: String,
    edit_actual_start: String,
    edit_actual_end: String,
    edit_assumed_start: String,
    edit_assumed_end: String,
    /// Register filter + assignment commander (step 3): hulls come
    /// from the synced store; each assign seats the picked commander.
    setup_reg_search: String,
    fleet_query: String,
    drill_branch: Option<i64>,
    drill_category: Option<i64>,
    drill_type: Option<i64>,
    drill_class: Option<i64>,
    /// An assign raised by a leaf-column row, drained by `picker_tail_ui`.
    ///
    /// The row that carries the verb is drawn inside a column's scroll area, so
    /// there is no return value to hand back the way the old list had one.
    pending_assign: Option<(i64, String)>,
    /// Fleet render cache (blocking ticket): register rows plus
    /// branch mapping + names, reloaded on sync and first show —
    /// never queried per frame. SQLite leaves the render path.
    fleet_cache: Vec<tfg::store::StoreUnit>,
    fleet_branches: std::collections::HashMap<i64, i64>,
    fleet_branch_names: std::collections::HashMap<i64, String>,
    fleet_loaded: bool,
    /// Session users (build ticket): memory-held game plus live lists —
    /// directory and game reads are blocking operator actions, the role
    /// vocabulary comes from the synced helpers mirror.
    users_game: Option<(i64, String)>,
    /// Authoritative state of the held game (H1): the last `GET
    /// /games/{id}` state — planning, preparation, execution, closure.
    /// Every select/refresh re-reads it and projects the local stage;
    /// the `sim_ready` flag never leads, it follows this read.
    users_game_state: Option<String>,
    /// Scenario clock, Minos-owned (H2): the last GameClock answer
    /// (pause/resume/factor writes are the only reads — there is no
    /// clock GET) plus the chosen rate off the detail anchor. The
    /// local engine ratio seeds from the factor, never from windows,
    /// for connected games; `factor_draft` is the bar's edit box.
    minos_clock: Option<tfg::backend::GameClock>,
    minos_time_factor: Option<f64>,
    factor_draft: f64,
    /// The held session's room key, from the detail read. Present
    /// from preparation onward — that is the share-out the Game
    /// Master hands personnel out of band; the client never invents
    /// one and never enumerates keys.
    minos_room_key: Option<String>,
    /// Clock-control denial: a 403 on pause/resume/factor means the
    /// hold's grant excludes the caller — learned per hold, cleared
    /// on success, hold change, drop, and sign-out. Gates the bar
    /// controls with the reason; never set from role names.
    clock_denied: bool,
    users_games: Vec<tfg::backend::GameRow>,
    /// Game-list gap: the account lacks the staff read, so the picker
    /// is empty by permission, not by absence. Drives the room-key
    /// guidance; cleared on success, reset on sign-out.
    games_gap: bool,
    /// The game-list permission has been observed at least once. This
    /// keeps organizer controls hidden during the first async read.
    games_loaded: bool,
    users_list: Vec<tfg::backend::BackendUser>,
    users_search: String,
    users_role: Option<i64>,
    users_roles: Vec<(i64, String, String, bool)>,
    users_roster: Vec<tfg::backend::Participant>,
    users_gunits: Vec<tfg::backend::GameUnit>,
    /// The held game's scenario book, and which scenario the composer has
    /// open. The book is loaded with the rest of the Planning bundle; the
    /// open id is `None` when the composer is closed.
    scenarios: Vec<tfg::backend::GameScenario>,
    composer_scenario: Option<i64>,
    /// Whether the composer is on screen. Separate from
    /// `composer_scenario` because a game with an EMPTY book must still be
    /// openable: the add-field lives in the modal, so with no scenario to
    /// select there would be no way to reach it.
    composer_visible: bool,
    /// A step being authored, before it is a server row. It exists as a
    /// draft because the backend assigns the id, and a field that only
    /// appears after a round trip is a field an author cannot see their
    /// cursor in.
    composer_draft: ComposerDraft,
    /// What is wrong with the draft, shown under the form. Separate from
    /// `users_status` because a half-typed window is not a backend refusal
    /// and must not overwrite the last real status line.
    composer_draft_error: Option<String>,
    /// Whether the Fleet Picker modal is showing.
    fleet_picker_open: bool,
    /// Whether the Player Picker modal is showing.
    player_picker_open: bool,
    /// The held session's last detail read, kept whole.
    ///
    /// Cleared whenever the hold changes, because a detail is a fact about ONE
    /// game and a stale one would show the previous session's plan.
    held_detail: Option<tfg::backend::GameDetail>,
    /// What the server last said about whether the held game may start.
    ///
    /// `None` before the first bundle lands, and after a failed read.
    /// `readiness_gap` distinguishes the two, because they mean different
    /// things to the operator: "not looked yet" and "could not be read".
    readiness: Option<tfg::backend::GameReadinessView>,
    /// Whether the last readiness read failed.
    ///
    /// The house pattern for a subordinate read: keep the last good answer
    /// behind a gap flag rather than replacing it with an empty list that reads
    /// as truth. An empty list here would say "nothing is outstanding", which
    /// is the most dangerous possible answer.
    readiness_gap: ReadinessGap,
    /// Every modal panel's rect, for this frame.
    ///
    /// The drop handlers need these because the map's own rect contains the
    /// panels: the map is full-bleed under the floating zones, so a release
    /// over a form would otherwise place at whatever map point happens to be
    /// behind it. Recomputed per frame from the same `Modal::rect_in` the
    /// primitive used, so there is one answer to where each panel is rather
    /// than two that can disagree.
    ///
    /// A Vec because two modals each own a drag-to-map, and cleared once per
    /// frame rather than per modal: a panel cannot remove its own rect on the
    /// frame it closes, and that frame is exactly the one where a release is
    /// still landing.
    modal_panel_rects: Vec<egui::Rect>,
    /// The in-flight book read or write. One slot rather than one per verb:
    /// the composer is a single author at a time, and a second write would
    /// answer against a book the first write had already changed.
    scenarios_op: Option<RestOp<Book, String>>,
    /// Ids of pieces in the exercise but UNASSIGNED in the task
    /// organisation (`id_hierarchy_node` absent). The symbology draws
    /// planned status as a dashed frame, and this is the fact behind
    /// it: a declaration a human made, not the absence of a fix.
    unassigned_units: std::collections::HashSet<i64>,
    /// The caller's own pieces from the last join/readiness answer:
    /// the order authority when the staff unit list is gapped. Never
    /// merged into users_gunits — a partial list must never pose as
    /// the full order of battle.
    commanded_hulls: Vec<tfg::backend::GameUnit>,
    /// Staff-gap flags: a 403 on the matching subread means "access
    /// refused", distinct from an empty authoritative list. Set on
    /// Forbidden, cleared on success, left alone on transient errors.
    /// The player-view ticket renders these; the status line names
    /// them from the bundle apply.
    roster_gap: bool,
    units_gap: bool,
    placements_gap: bool,
    /// Server setup view of the held game's map (C2): placements plus
    /// the gate's placement arithmetic — never recomputed client-side.
    users_placements: Vec<tfg::backend::GamePlacement>,
    placement_unplaced: i64,
    placement_ready: bool,
    /// Room-key entry (C2): joining needs no game id — the key is the
    /// only input, and the answer says which game was entered.
    join_key: String,
    users_status: String,
    /// Directory filters (latest Minos): resolved from the helpers
    /// mirror (`user_statuses`, `app_roles`), sent as `id_user_status`
    /// / `id_app_role` only when set. Nothing re-fetches on change —
    /// the `find` button runs the query.
    users_filter_status: Option<i64>,
    users_statuses: Vec<(i64, String, String, bool)>,
    users_filter_role: Option<i64>,
    users_approles: Vec<(i64, String, String, bool)>,
    upress: Option<(i64, String, egui::Pos2)>,
    udrag: Option<(i64, String)>,
    unit_drag: Option<UnitDrag>,
    /// Why the last placement attempt ended as it did. Rendered by the picker
    /// and the Fleet island, and never written by anything but a placement.
    placement_note: PlacementNote,
    /// Selected map unit currently being rotated by its on-map handle.
    map_heading_drag: Option<String>,
    placed_fleet: HashSet<String>,
    /// Latch: the last pointer release ended a genuine move drag. The
    /// map click that rides the same release must be swallowed by the
    /// selection step rather than selecting or deselecting.
    released_move_drag: bool,
    /// Labels captured at placement (both picker sources), so register
    /// hulls keep their name + hull after the picker moves on.
    placed_labels: HashMap<String, (String, String)>,
    /// The force as the operator has it, staged locally. The map and the
    /// islands read from THIS, and Minos catches up at a stage advance.
    /// See `tfg::force` for why the diff and not the writes are the rule.
    force: tfg::force::ForceDraft,
    /// Which held game the force draft was seeded from. A held-game change
    /// resets it, so the next bundle seeds from the new hold; a re-read
    /// against the same hold must not clobber the operator's edits.
    force_seeded_for: Option<i64>,
    /// Writes the flush owes Minos, issued one at a time through the
    /// `setup_op` slot. Each answer advances or aborts the chain.
    force_queue: Vec<tfg::force::ForceWrite>,
    /// What to do when the queue drains: a stage to advance to, or just
    /// a sync. `None` while no flush is owed.
    force_flush_then: Option<String>,
    /// A placed hull being dragged to a new starting position, if the
    /// gesture is in flight.
    map_unit_move: Option<UnitMoveDrag>,
    /// Setup slice (ii): WIB-entered windows, stored as UTC.
    time_real_start: String,
    time_real_end: String,
    time_game_start: String,
    time_game_end: String,
    /// Helm per placed unit (unit_id -> user): commander seats draft here.
    helm: HashMap<String, String>,
    unit_commander: HashMap<String, String>,
    /// Minos auth state (login ticket): REST base, island fields, and the
    /// session — identifier plus in-memory access token with issue time +
    /// TTL. The refresh token lives in the OS keyring (memory fallback
    /// when degraded, flagged). Bearer-ready for the later socket work.
    minos_base: String,
    show_login: bool,
    login_identifier: String,
    login_password: String,
    auth_user: Option<String>,
    /// Caller id from the `GET /users/me` probe (C2): roster rows key
    /// on `id_user` and the login identifier is not it. Set on sign-in
    /// and confirmed by every join/readiness answer; cleared on sign-out.
    auth_user_id: Option<i64>,
    /// Application-role ids from the same probe. An empty list is a
    /// participant account: room-key join is available, session setup
    /// administration is not.
    /// The whole `/users/me` projection. `None` is not signed in.
    auth_identity: Option<tfg::backend::AuthenticatedUser>,
    auth_token: Option<String>,
    auth_issued_at: Option<Instant>,
    auth_ttl_secs: u64,
    auth_refresh_memory: Option<String>,
    auth_degraded: bool,
    auth_status: String,
    auth_needs_password_change: bool,
    pw_current: String,
    pw_new: String,
    /// Local-first store (master-data ticket): sqlite working copy of
    /// reference truth. None when the file cannot open (sync disabled,
    /// said loudly). UI thread owns it; sync replaces whole tables.
    store: Option<rusqlite::Connection>,
    sync_status: String,
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
    /// User-facing text scaling (field ticket): session pref over
    /// the OS pixels-per-point, captured once so re-applying never
    /// compounds. Field laptops and glare-heavy displays get Larger
    /// without touching layout code.
    text_scale: f32,
    base_ppp: Option<f32>,
    /// Top-level mode (task #39): session only in Simulation.
    app_mode: AppMode,
    wire_ctl_tx: Option<Sender<WireKind>>,
    /// Live-wire fast lane: the socket actor pings this per publication;
    /// the poll thread cuts its sleep short. One channel for the app's
    /// lifetime; each connect carries a clone to its actor.
    wire_wake_tx: Sender<()>,
    /// Live socket handles (live-wire ticket): token push-down to the
    /// actor, event reports up. Set on connect-live, cleared on swap
    /// away; dropping the wire drops the actor (LiveWire::drop).
    live_cmd_tx: Option<Sender<LiveCmd>>,
    live_evt_rx: Option<Receiver<LiveEvent>>,
    live_status: String,
    /// The link's state as a type, kept beside `live_status` rather than
    /// re-derived from it.
    ///
    /// The message is a human sentence and the state is not, and the top
    /// zone wants the state. Classifying the sentence at every read would
    /// put a substring match on every frame of a console whose whole design
    /// rule is that state says its own colour.
    live_state: LinkState,
    /// First-connect marker: the actor's Connected fires per
    /// (re)connect, and only a repeat while a game is held triggers
    /// the held-game reconciliation (games list → split bundle + plot
    /// pull). Clock running-state has no read endpoint — the bundle
    /// re-marks the factor; held/running resumes from answers.
    live_connected_once: bool,
    /// Off-thread REST slots (M7): at most one op of each kind in
    /// flight; the frame pump harvests and applies results.
    login_op: Option<RestOp<LoginDone>>,
    refresh_op: Option<RestOp<tfg::backend::TokenPair>>,
    sync_op: Option<RestOp<SyncDone>>,
    spec_op: Option<RestOp<SpecDone>>,
    pw_op: Option<RestOp<PwResult, PwResult>>,
    plot_op: Option<PlotSlot>,
    /// Serialized setup-flow results (#100): at most one setup read or
    /// write in flight; the frame pump applies it.
    setup_op: Option<RestOp<SetupDone>>,
    /// Queued setup refreshes (#100): refresh clicks that land while
    /// the slot is busy wait here (deduped) instead of dropping — the
    /// pump dispatches one per applied op.
    pending_setup: Vec<PendingRefresh>,
    /// Transition awaiting its verdict (#100): (to, error, forbidden).
    /// Set when a transition write fails; the bundle re-read consumes
    /// it — the authoritative state decides whether the game already
    /// moved or the refusal stands.
    pending_transition: Option<(String, String, bool)>,
    /// Steady plot cadence (#98): last spawn, last success, and the
    /// consecutive-failure streak driving backoff.
    last_plot_try: Option<Instant>,
    last_plot_ok: Option<Instant>,
    plot_fails: usize,
    /// Last authoritative bundle apply (context strip): how stale the
    /// held game's projection is. Stamped on detail success even when
    /// every subordinate gaps — the detail is the sync.
    last_game_sync: Option<Instant>,
    /// Session-log history view (task #40): selected past journal.
    log_view_path: Option<std::path::PathBuf>,
    /// Log work state (blocking ticket): directory listing cached
    /// once per open (never per frame), the parsed view, and the
    /// parse worker. Disk reads leave the frame.
    log_files: Vec<std::path::PathBuf>,
    log_files_loaded: bool,
    log_view: Option<LogViewData>,
    log_op: Option<RestOp<LogOut>>,
    /// Game messages as socket events carry them (H11): broadcast on
    /// `game:<id>`, addressed on `personal:<user>`. Newest last, capped
    /// — the event is a notification with body, not the inbox (sending
    /// and thread reads stay HTTP, later slice).
    game_messages: VecDeque<GameMsg>,
    show_messages: bool,
    /// HTTP inbox (#99): thread reads with read state, plus the compose
    /// form. Socket arrivals (above) notify; this is the readable,
    /// sendable record. Inbox ops run off-thread like everything in
    /// pump_rest_ops.
    inbox: Vec<tfg::backend::InboxMsg>,
    inbox_mine_only: bool,
    /// Inbox pager (messages ticket): the page the island shows plus
    /// its navigation block; the open message's detail pane.
    inbox_page_no: u64,
    inbox_total: u64,
    inbox_pages: u64,
    inbox_has_next: bool,
    inbox_has_prev: bool,
    msg_open: Option<tfg::backend::InboxMsg>,
    /// Closure timeline (assessment ticket): loaded events in server
    /// order (never re-sorted), the cursor block, and filter drafts.
    /// A failed page keeps the last good list.
    timeline_events: Vec<tfg::backend::TimelineEvent>,
    timeline_cursor: Option<String>,
    timeline_has_more: bool,
    timeline_source: Option<String>,
    timeline_personnel: String,
    timeline_unit: String,
    timeline_from: String,
    timeline_to: String,
    /// Closure judgements (assessment ticket): the exercise's marks,
    /// the compose drafts, and the subject picker. Append-only by
    /// contract — the UI offers no edit, correction is a new mark.
    judgements: Vec<tfg::backend::Judgement>,
    judge_subject: Option<i64>,
    judge_subject_id: String,
    judge_score: String,
    /// Closure reviews (assessment ticket): the exercise's documents,
    /// compose drafts, and the revision under edit. Author-owned by
    /// contract — edit renders on own rows only, delete nowhere.
    reviews: Vec<tfg::backend::Review>,
    review_subject: Option<i64>,
    review_body: String,
    review_editing: Option<i64>,
    review_mine_only: bool,
    /// Minos task organisation (hierarchy ticket): the forest as
    /// read, plus its staff gap. Rendered, never edited here — node
    /// and assignment writes are a later slice. The local Groups
    /// model is sandbox-only and draws nowhere near this.
    minos_tree: Vec<tfg::backend::HierarchyNode>,
    tree_gap: bool,
    /// Every unit's visual state: measurements, picture availability,
    /// intrinsic dimensions, orientation, symbol, and the decoded
    /// texture. Keyed by `(unit_id, asset_version)`, so an expiring
    /// presigned URL is replaced without the asset losing identity,
    /// and a manifest change cannot hand one version's picture to
    /// another. Memory only: presigned URLs never reach SQLite, and
    /// the texture dies with the process.
    visuals: VisualCache,
    /// Global Inspector thumbnail width preference. This is deliberately
    /// not cleared with the visual session: it is a view preference, not
    /// a game or asset fact.
    inspector_image_width: f32,
    /// Taxonomy symbols resolved independently of the image manifest,
    /// so the far map is useful before picture bytes arrive. The
    /// VisualCache copies these into each versioned UnitVisual.
    unit_symbols: HashMap<i64, tfg::store::MapSymbol>,
    /// Stable type ids and display names for the selected-unit map
    /// symbol editor. The name is shown to the operator; resolution
    /// itself only ever reads the id-keyed assignment table.
    unit_type_ids: HashMap<i64, i64>,
    unit_type_names: HashMap<i64, String>,
    unit_type_symbols: HashMap<i64, tfg::store::MapSymbol>,
    /// `units.id -> branch_id` from the mirror, for the affiliation
    /// resolver's session-branch declaration. The Fleet cache is built
    /// from the fleet manifest and carries no taxonomy ids at all, so
    /// this is read from the same query that feeds `unit_symbols`.
    unit_branches: HashMap<i64, i64>,
    /// Every affiliation declaration for the current session, as one
    /// value. Nothing here decides anything yet: the painters read it
    /// when the frames land.
    affiliations: tfg::store::AffiliationResolver,
    /// Last selected LOD per unit, kept across frames so the Near
    /// threshold has hysteresis instead of oscillating on zoom jitter.
    unit_lods: HashMap<String, UnitLod>,
    /// Last selected Representation per group, kept the same way so a
    /// formation sitting on the Zone threshold holds its level instead
    /// of oscillating. Keyed by group id, like `unit_lods` by ship id.
    group_representations: HashMap<String, GroupRepresentation>,
    /// Operator-only visual heading override. It rotates the thumbnail
    /// without rewriting the authoritative Minos fix.
    heading_overrides: HashMap<String, f32>,
    /// Units whose temporary URL still needs reading, in priority
    /// order. Drained one per pump slot so a large manifest cannot
    /// open a burst of parallel requests.
    pending_image_urls: Vec<i64>,
    image_op: Option<RestOp<ImageOut>>,
    image_request: Option<ImageRequest>,
    /// A failed manifest read is retried after a pause, not every
    /// frame. A successful read clears this in `apply_image`.
    manifest_retry_at: Option<Instant>,
    /// Next scheduled manifest read after a successful load. The first
    /// read is immediate; later reads discover images added mid-session.
    manifest_refresh_at: Option<Instant>,
    msg_kind: String,
    msg_class: String,
    msg_content: String,
    msg_callsign: String,
    msg_sending_note: String,
    msg_group: String,
    msg_per: String,
    msg_regnum: String,
    msg_to: HashSet<i64>,
    msg_cc: HashSet<i64>,
    msg_degree: Option<i64>,
    msg_degrees: Vec<(i64, String, String, bool)>,
    msg_assumed: Option<i64>,
    msg_reply_to: Option<i64>,
    scenario_roles: Vec<tfg::backend::ScenarioRole>,
    new_role_name: String,
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
    /// Session groups: unified group forest (Satuan Tugas of units,
    /// Gugus of groups), drawn on the map.
    groups: Groups,
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
    /// Drain pending poll rounds, then derive marker geometry for this frame.
    fn markers(&mut self, pixels_per_point: f32) -> Vec<ShipMarker> {
        let mut rounds = 0;
        // The glide clock restarts only on genuinely new data (see below):
        // the live wire replays the whole picture every tick, and duplicate
        // rounds must not replay the animation.
        let mut advanced = false;
        let accept_movement = accepts_movement_fixes(self.app_mode, self.mode.phase);
        let setup_sim_only =
            self.app_mode == AppMode::Simulation && self.mode.phase == Phase::Setup;
        for fixes in self.poll_rx.try_iter() {
            rounds += 1;
            if !accept_movement {
                continue;
            }
            let fixes: Vec<Fix> = if setup_sim_only {
                fixes
                    .into_iter()
                    .filter(|f| f.source == FixSource::Sim)
                    .collect()
            } else {
                fixes
            };
            if fixes.is_empty() {
                continue;
            }
            let acked = self.registry.poll(fixes.clone());
            let current_ids: HashSet<String> = acked
                .iter()
                .filter(|(id, _)| {
                    fixes.iter().any(|f| &f.ship_id == id && !f.backfilled)
                })
                .map(|(id, _)| id.clone())
                .collect();
            // Local-first (master-data ticket): only accepted current wire
            // fixes land in the standing-picture table (decimal ids only;
            // sim, replay, and historical fixes have no current row).
            if let Some(conn) = &self.store {
                let wire: Vec<tfg::geo::track::Fix> = fixes
                    .into_iter()
                    .filter(|f| {
                        f.source == tfg::geo::track::FixSource::Wire
                            && current_ids.contains(&f.ship_id)
                            && f.ship_id.bytes().all(|b| b.is_ascii_digit())
                    })
                    .collect();
                if !wire.is_empty() {
                    let _ = tfg::store::upsert_positions(conn, &wire, &tfg::backend::now_ts());
                }
            }
            // Accepted current fixes move the glide clock and freshness;
            // historical fixes are counted and journaled but never make a
            // stale ship look fresh.
            let now = Instant::now();
            for (ship_id, _) in &acked {
                *self.fix_count.entry(ship_id.clone()).or_insert(0) += 1;
                if current_ids.contains(ship_id) {
                    self.last_seen.insert(ship_id.clone(), now);
                    self.fix_animation_started.insert(ship_id.clone(), now);
                }
            }
            if !current_ids.is_empty() {
                advanced = true;
            }
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
                SimEvent::HelmApplied {
                    ship_id,
                    heading_deg,
                    requested_speed_kn,
                    accepted_speed_kn,
                } => {
                    if !self.applied_submission_matches(
                        &ship_id,
                        heading_deg,
                        requested_speed_kn,
                    ) {
                        continue;
                    }
                    let result = if requested_speed_kn > accepted_speed_kn {
                        HelmOrderUiResult::Clamped {
                            requested: requested_speed_kn as f64,
                            accepted: accepted_speed_kn as f64,
                        }
                    } else {
                        HelmOrderUiResult::Accepted
                    };
                    self.helm_submissions.remove(&ship_id);
                    if self.helm_drafts.contains_key(&ship_id) {
                        self.helm_drafts.insert(
                            ship_id.clone(),
                            HelmDraft {
                                heading_deg,
                                speed_kn: accepted_speed_kn,
                            },
                        );
                    }
                    self.helm_preview_pending.insert(ship_id.clone());
                    self.helm_warnings.remove(&ship_id);
                    self.order_result.insert(ship_id, result);
                }
                SimEvent::OrderRefused { ship_id, reason } => {
                    if matches!(
                        self.order_result.get(&ship_id),
                        Some(HelmOrderUiResult::Superseded)
                    ) {
                        continue;
                    }
                    let why = match reason {
                        OrderRefusal::LandWaypoint => "waypoint is on land",
                        OrderRefusal::LandBetween => "path crosses land",
                        OrderRefusal::NoSpeedLimit => "no local speed limit",
                        OrderRefusal::InvalidHelm => "invalid heading or speed",
                        OrderRefusal::UnknownClass => {
                            "unknown class — sync the unit register first"
                        }
                    };
                    self.helm_submissions.remove(&ship_id);
                    self.order_result
                        .insert(ship_id.clone(), HelmOrderUiResult::Refused(why.to_string()));
                    self.feed(format!("refused {ship_id}: {why}"));
                    self.order_warning = Some(format!("{ship_id}: {why}"));
                }
                SimEvent::CommandRefused { ship_id, reason } => {
                    if matches!(
                        self.order_result.get(&ship_id),
                        Some(HelmOrderUiResult::Superseded)
                    ) {
                        continue;
                    }
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
                    self.helm_submissions.remove(&ship_id);
                    self.order_result
                        .insert(ship_id.clone(), HelmOrderUiResult::Refused(why.to_string()));
                    self.feed(format!("command refused ({ship_id}): {why}"));
                    self.order_warning = Some(format!("{ship_id}: {why}"));
                }
                SimEvent::CommandOverridden { ship_id, prev_rank, by_rank } => {
                    // Escalation, not a hijack: the command succeeded and
                    // took the ship to a higher authority (e.g. the
                    // organizer's group order lifting UNIT-held ships).
                    // Loud in the feed + journal, but no sticky warning:
                    // warnings are for orders that did NOT happen.
                    self.feed(format!("authority {ship_id}: {prev_rank} -> {by_rank}"));
                }
                SimEvent::ShipBlocked { ship_id } => {
                    self.feed(format!("blocked at coast: {ship_id}"));
                    self.helm_warnings.insert(
                        ship_id.clone(),
                        "local HelmOrder blocked at land".to_string(),
                    );
                }
                SimEvent::Arrival { ship_id } => {
                    self.feed(format!("arrived {ship_id}"));
                }
            }
        }
        // Live socket reports (live-wire ticket): same drain-then-feed
        // borrow rule. Announcements seed silent vessels; refusals drive
        // sign-out/error UX; 109 refreshes silently on our path.
        let live_evts: Vec<LiveEvent> = match &self.live_evt_rx {
            Some(rx) => rx.try_iter().collect(),
            None => Vec::new(),
        };
        for evt in live_evts {
            match evt {
                LiveEvent::Connected { client_id } => {
                    self.live_state = LinkState::Live;
                    self.live_status = format!("connected ({client_id})");
                    self.feed(format!("live connected ({client_id})"));
                    // Reconnect reconciliation: the actor already
                    // re-read the position picture itself; the exercise
                    // state (detail always, subordinates as permitted,
                    // then the plot) goes through the split reads so a
                    // fresh map never sits on stale game state. Queued
                    // when the slot is busy, skipped with no hold.
                    let reconnected = self.live_connected_once;
                    self.live_connected_once = true;
                    if reconnected && self.users_game.is_some() {
                        self.feed("reconnected — resyncing held game".to_string());
                        self.users_refresh_games();
                        self.pull_minos_positions();
                    }
                }
                LiveEvent::Announced(list) => {
                    let n = list.len();
                    for (id, name, hull) in list {
                        self.registry.announce(id, name, hull);
                    }
                    self.feed(format!("live snapshot: {n} vessel(s) known"));
                }
                LiveEvent::Reconnecting { attempt, wait_secs } => {
                    self.live_status =
                        format!("reconnecting (attempt {attempt}, ~{wait_secs}s)");
                }
                LiveEvent::Refused { code, reason } => {
                    self.live_state = LinkState::Refused;
                    self.live_status = format!("refused {code}: {reason}");
                    self.feed(format!("live refused {code}: {reason}"));
                    if code == 101 {
                        // Token is dead: sign out to the Login island.
                        self.sign_out("live refused 101");
                        self.show_login = true;
                    } else {
                        self.order_warning =
                            Some(format!("live feed refused {code}: {reason}"));
                    }
                }
                LiveEvent::RefreshDue => {
                    self.feed("live token expired (109): refreshing".to_string());
                    self.refresh_now();
                }
                LiveEvent::Message(m) => {
                    // H11: broadcast or addressed — drawn, filed, and
                    // the island opens itself once for the new mail.
                    let scope = if m.broadcast { "broadcast" } else { "addressed" };
                    let text: String = m.content.chars().take(160).collect();
                    let trail = if m.content.chars().count() > 160 { "…" } else { "" };
                    self.feed(format!(
                        "[session {}] {} {} from {} ({scope}): {text}{trail}",
                        m.game_id, m.kind, m.class_label, m.sender
                    ));
                    self.game_messages.push_back(m);
                    while self.game_messages.len() > 50 {
                        self.game_messages.pop_front();
                    }
                    self.show_messages = true;
                }
                LiveEvent::Resynced => {
                    let ids: Vec<String> = self.registry.ship_ids();
                    self.reconnect_snap.extend(ids.iter().cloned());
                    self.game_animation_snap.extend(ids);
                }
                LiveEvent::Vanished(ids) => {
                    for id in ids {
                        self.registry.remove_ship(&id);
                        self.fix_animation_started.remove(&id);
                        self.last_seen.remove(&id);
                        self.fix_count.remove(&id);
                    }
                }
                LiveEvent::GamePositions(update) => {
                    let held = self.users_game.as_ref().map(|(id, _)| *id);
                    if self.app_mode != AppMode::Simulation
                        || self.users_game_state.as_deref() != Some("execution")
                        || held != Some(update.game_id)
                    {
                        continue;
                    }
                    let (_, accepted) = self.ingest_game_positions(update);
                    if accepted {
                        let now = Instant::now();
                        self.last_game_position_at = Some(now);
                        self.last_plot_ok = Some(now);
                        self.plot_fails = 0;
                    }
                }
                LiveEvent::OrderIssued(event) => {
                    let held = self.users_game.as_ref().map(|(id, _)| *id);
                    if self.app_mode != AppMode::Simulation
                        || self.users_game_state.as_deref() != Some("execution")
                        || held != Some(event.game_id)
                    {
                        continue;
                    }
                    let game_id = event.game_id;
                    let fix = event.fix;
                    // The order publication carries an authoritative
                    // position too. Apply it before reconciling the
                    // command result so the marker does not wait for a
                    // second REST/position publication.
                    let (_, accepted) = self.ingest_game_positions(
                        GamePositionUpdate::from_fix(game_id, fix.clone()),
                    );
                    if accepted {
                        self.last_game_position_at = Some(Instant::now());
                    }
                    let ship_id = fix.unit_id.to_string();
                    if matches!(
                        self.order_result.get(&ship_id),
                        Some(HelmOrderUiResult::Unknown(_))
                    ) && self.order_event_matches(&ship_id, &fix) {
                        let result = if fix.clamped {
                            HelmOrderUiResult::Clamped {
                                requested: fix.requested_speed.unwrap_or(fix.speed),
                                accepted: fix.speed,
                            }
                        } else {
                            HelmOrderUiResult::Accepted
                        };
                        self.helm_submissions.remove(&ship_id);
                        if self.helm_drafts.contains_key(&ship_id) {
                            self.helm_drafts.insert(
                                ship_id.clone(),
                                HelmDraft {
                                    heading_deg: fix.heading as f32,
                                    speed_kn: fix.speed as f32,
                                },
                            );
                        }
                        self.helm_preview_pending.insert(ship_id.clone());
                        self.order_result.insert(ship_id.clone(), result);
                        self.feed(format!(
                            "order event reconciled {ship_id}: {:.0}° @ {:.0} kn",
                            fix.heading, fix.speed
                        ));
                    }
                }
                LiveEvent::SocketError(e) => {
                    self.live_state = LinkState::Faulted;
                    self.live_status = format!("socket error: {e}");
                    eprintln!("live socket error: {e}");
                }
            }
        }
        if rounds > 0 {
            let ids = self.registry.ship_ids();
            eprintln!(
                "poll: {} ship(s){}",
                ids.len(),
                ids.iter()
                    .filter(|id| self.registry.ship(id).is_some_and(|s| s.stale))
                    .map(|id| format!(" [stale: {id}]"))
                    .collect::<String>()
            );
        }
        // Glide clock: restarted above only when new fixes were accepted.
        // Duplicate rounds leave it running, so frac parks at 1.0 and the
        // marker rests instead of re-animating previous → latest.
        if advanced {
            self.last_poll = Instant::now();
        }
        let center = self.center;
        let (mw, mh) = self.map_dims();
        // Wall clock for the old-data badge (wire sources only: sim
        // clocks are game time, replay receipts are fresh by stamp).
        let now_epoch = chrono::Utc::now().timestamp();
        self.registry
            .ships()
            .iter()
            .map(|s| {
                let animation_started = self
                    .fix_animation_started
                    .get(&s.ship_id)
                    .copied()
                    .unwrap_or(self.last_poll);
                let animation_secs = if s.source == FixSource::Game {
                    GAME_PLOT_ANIMATION_SECS
                } else {
                    POLL_SECS
                };
                let paused = matches!(s.source, FixSource::Sim | FixSource::Game)
                    && self.game_paused;
                let large_gap = if s.source == FixSource::Game {
                    self.game_animation_snap.contains(&s.ship_id)
                        || self.reconnect_snap.contains(&s.ship_id)
                } else if s.source == FixSource::Wire {
                    self.reconnect_snap.contains(&s.ship_id)
                } else {
                    self.registry.has_large_gap(
                        &s.ship_id,
                        (animation_secs * 3.0).ceil() as i64,
                    )
                };
                let raw_frac =
                    (animation_started.elapsed().as_secs_f64() / animation_secs)
                        .clamp(0.0, 1.0);
                let frac = if large_gap {
                    if matches!(s.source, FixSource::Wire | FixSource::Game) {
                        self.reconnect_snap.remove(&s.ship_id);
                    }
                    self.animation_paused_fraction.remove(&s.ship_id);
                    1.0
                } else if paused {
                    *self
                        .animation_paused_fraction
                        .entry(s.ship_id.clone())
                        .or_insert(raw_frac)
                } else {
                    self.animation_paused_fraction.remove(&s.ship_id);
                    raw_frac
                };
                self.reconcile_helm_preview(&s.ship_id, frac);
                // A hull between fixes is a 1-2s interpolation (ADR-0011),
                // not an animation, but it still needs frames. A paused sim
                // holds its fraction and is already stationary, so it must
                // not keep the loop awake.
                if !paused && frac < 1.0 {
                    self.dirty = true;
                }
                let pos = self.registry.blend(&s.ship_id, frac).unwrap_or(s.latest.position);
                let (x, y) = project_mercator(pos.latitude, pos.longitude, center, self.zoom, mw, mh);
                let trail = s
                    .trail
                    .iter()
                    .map(|p| project_mercator(p.latitude, p.longitude, center, self.zoom, mw, mh))
                    .collect();
                let old_data = s.source == FixSource::Wire && s.latest.is_old_data(now_epoch);
                // Course rides the same blend fraction as position, so
                // the image turns with the hull rather than snapping
                // to each new fix. Minos game positions and live-feed
                // course both already land on Fix.heading_deg.
                let authoritative_heading = self.registry.blend_heading(&s.ship_id, frac);
                let heading_deg = self.map_preview_heading(
                    &s.ship_id,
                    authoritative_heading,
                );
                let unit_id = s.ship_id.parse::<i64>().ok();
                let visual = unit_id.and_then(|unit_id| self.visuals.get(unit_id).cloned());
                let mut geometry = visual.as_ref().map_or_else(ProjectedUnitGeometry::default, |v| {
                    projected_unit_geometry(pos.latitude, self.zoom, v.loa_m, v.beam_m)
                });
                if geometry.has_scale {
                    let ppp = pixels_per_point.max(0.1) as f64;
                    geometry.length_px /= ppp;
                    geometry.beam_px /= ppp;
                }
                if !geometry.has_scale
                    && visual.as_ref().is_some_and(|v| v.asset_kind == AssetKind::UnitImage)
                {
                    // Temporary presentation-only footprint while the
                    // backend geometry fields are still in development.
                    if let Some(v) = visual.as_ref() {
                        let (length_px, beam_px) = fallback_unit_geometry(v);
                        geometry.length_px = length_px;
                        geometry.beam_px = beam_px;
                    }
                }
                let current_lod = self.unit_lods.get(&s.ship_id).copied();
                let lod = select_unit_lod(&geometry, current_lod);
                self.unit_lods.insert(s.ship_id.clone(), lod);
                let map_symbol = visual
                    .as_ref()
                    .map(|visual| visual.map_symbol)
                    .or_else(|| unit_id.and_then(|unit_id| self.unit_symbols.get(&unit_id).copied()))
                    .unwrap_or(tfg::store::MapSymbol::UnknownShip);
                // Allegiance, resolved beside the symbol: a unit the
                // mirror never classified still draws, as Unknown.
                let affiliation = self
                    .affiliations
                    .resolve(&self.affiliation_subject(&s.ship_id));
                let planned = unit_is_planned(unit_id, &self.unassigned_units);
                let stale = s.stale
                    || (s.source == FixSource::Game
                        && self
                            .last_seen
                            .get(&s.ship_id)
                            .is_some_and(|at| at.elapsed() >= Duration::from_secs(5)));
                ShipMarker {
                    id: s.ship_id.clone(),
                    x,
                    y,
                    stale,
                    old_data,
                    source: s.source,
                    trail,
                    heading_deg,
                    map_symbol,
                    latitude: pos.latitude,
                    longitude: pos.longitude,
                    footprint_px: geometry.length_px.max(geometry.beam_px),
                    affiliation,
                    planned,
                    label: self.map_label(
                        &s.ship_id,
                        s.latest.name.as_deref(),
                        s.latest.hull_number.as_deref(),
                    ),
                    lod,
                    visual,
                }
            })
            .collect()
    }

    /// Start a clean data epoch for a view/mode change. Queued work is
    /// discarded rather than allowed to repopulate the new view.
    fn reset_registry_view(&mut self) {
        while self.poll_rx.try_recv().is_ok() {}
        while self.sim_evt_rx.try_recv().is_ok() {}
        self.registry = Registry::new(TrailBound::default());
        self.fix_animation_started.clear();
        self.animation_paused_fraction.clear();
        self.game_animation_snap.clear();
        self.reconnect_snap.clear();
        self.last_seen.clear();
        self.fix_count.clear();
        self.last_game_position_at = None;
        self.plot_op = None;
        self.setup_op = None;
        self.pending_setup.clear();
        if let Some(tx) = &self.live_cmd_tx {
            let _ = tx.send(LiveCmd::Reseed);
        }
    }

    /// Switch top-level modes (task #39): the view clears either way
    /// and Presentation disarms the engine, so watching never stands
    /// anything up. Simulation state underneath is untouched.
    fn set_app_mode(&mut self, mode: AppMode) {
        if self.app_mode == mode {
            return;
        }
        self.app_mode = mode;
        self.reset_registry_view();
        self.deselect();
        self.following = None;
        self.recentering = None;
        if mode == AppMode::Presentation {
            self.mode.armed.store(false, Ordering::SeqCst);
            if let Some(tx) = &self.sim_cmd_tx {
                let _ = tx.send(SimCommand::ResetTick);
            }
            if let Some(tx) = &self.live_cmd_tx {
                let _ = tx.send(LiveCmd::WatchGame(None));
                let _ = tx.send(LiveCmd::WatchGamePositions(None));
            }
            self.show_roster = false;
            self.show_orders = false;
        } else {
            // Simulation arms the engine; the setup flow owns Planning.
            if let Some(tx) = &self.sim_cmd_tx {
                let _ = tx.send(SimCommand::ResetTick);
            }
            self.mode.armed.store(true, Ordering::SeqCst);
            self.watch_game_channel();
        }
        eprintln!("mode: {mode:?}");
    }

    /// M6: hold sim motion for Setup. The engine stays armed (planning
    /// markers need TakeControl), but the clock and legs freeze — Setup
    /// has no movement by product definition. Live entry unfreezes.
    fn hold_sim_for_setup(&mut self) {
        if let Some(tx) = &self.sim_cmd_tx {
            let _ = tx.send(SimCommand::SetPaused { paused: true });
        }
    }

    /// M6: release every locally controlled ship. A new exercise starts
    /// with no ghost legs from the previous session's sim.
    fn release_all_local(&mut self) {
        let ids: Vec<String> = self.controlled.iter().cloned().collect();
        for id in &ids {
            self.release_hull(id);
        }
        self.pending_waypoint = None;
        self.placing = false;
        self.unit_drag = None;
        self.map_heading_drag = None;
    }

    /// Hand-off from State B (onboarding ticket, #77): the shell
    /// replaces the old boot clutter — this machine directs first run
    /// now. Presentation starts on its State C connect card, Simulation
    /// opens Planning on the Exercise setup flow.
    fn enter_shell(&mut self, mode: AppMode) {
        self.onboard = Onboard::App;
        self.set_app_mode(mode);
        match mode {
            AppMode::Presentation => {
                self.connect_card = true;
            }
            AppMode::Simulation => {
                // Planning opens on step 1 with a fresh game list (#79).
                self.setup_step = 0;
                self.connect_card = false;
                self.users_refresh_games();
                // #101: arm here, not only on mode switch — booting
                // straight into Simulation early-returns set_app_mode
                // (already the default) and placement stays refused.
                self.mode.armed.store(true, Ordering::SeqCst);
                // M6: Setup has no movement — freeze legs and clock.
                self.hold_sim_for_setup();
            }
        }
    }

    /// Four-phase read (ticket #77): Planning and Ready share
    /// `Phase::Setup` (the ready flag gates Persiapan), Eksekusi is
    /// Live, Evaluasi is Closed. Derived, so `UiMode` stays the
    /// single writer of the underlying phase.


    /// The single game-state read, derived once per frame.
    ///
    /// The backend is authoritative (ADR-0008): `users_game_state` is what a
    /// `GET /games/{id}` resync last wrote, and a held game with no state yet
    /// reads as `NoSession` rather than as a guess. `UiMode` stays the only
    /// writer of the LOCAL phase, and this is the one place the two are
    /// reconciled.
    ///
    /// Presentation has no session of its own, so it is always `NoSession`
    /// and the side zone shows the watcher rather than a planning surface.
    fn game_state(&self) -> GameState {
        if self.app_mode == AppMode::Presentation {
            return GameState::NoSession;
        }
        match (&self.users_game, &self.users_game_state) {
            (Some(_), Some(state)) => GameState::from_wire(state),
            _ => GameState::NoSession,
        }
    }

    /// `GameState::is_planning` as a method, so call sites read as a
    /// question about the game rather than as a free function.



    /// Connect/stop the live socket: token-gated swap onto the
    /// socket, or back to the boot wire. Snapshot failure lands back
    /// with the reason; fatal refusals arrive as status. One verb for
    /// the Connection island and the onboarding State C card (#77).
    fn toggle_live(&mut self) {
        let ws_url = Self::ws_url_for(&self.minos_base);
        if self.live_cmd_tx.is_some() {
            if let Some(tx) = &self.wire_ctl_tx {
                let _ = tx.send(WireKind::Empty);
                self.live_cmd_tx = None;
                self.live_evt_rx = None;
                self.live_state = LinkState::Idle;
                self.live_status = "idle".to_string();
                eprintln!("wire: live stopped");
            }
        } else {
            let token = self.auth_token.clone().unwrap_or_default();
            let rest_base = self.minos_base.clone();
            let (cmd_tx, cmd_rx) = mpsc::channel();
            let (event_tx, event_rx) = mpsc::channel();
            if let Some(tx) = &self.wire_ctl_tx {
                let _ = tx.send(WireKind::Live {
                    ws_url: ws_url.clone(),
                    rest_base,
                    token,
                    cmd_tx: cmd_tx.clone(),
                    cmd_rx,
                    event_tx,
                    wake_tx: self.wire_wake_tx.clone(),
                });
                self.live_cmd_tx = Some(cmd_tx);
                self.live_evt_rx = Some(event_rx);
                self.live_state = LinkState::Connecting;
                self.live_status = "connecting…".to_string();
                eprintln!("wire: live {ws_url}");
                // H11: a fresh actor watches nothing — re-declare the
                // held game and the session's personal channel.
                self.watch_game_channel();
                self.watch_personal_channel();
            }
        }
    }

    /// Clear the in-memory visual boundary. Presigned URLs and decoded
    /// textures belong to one authenticated user and one held
    /// exercise; a new one starts with none of them.
    fn clear_visual_cache(&mut self) {
        self.visuals.clear();
        self.unit_lods.clear();
        self.heading_overrides.clear();
        self.pending_image_urls.clear();
        self.image_op = None;
        self.image_request = None;
        self.manifest_retry_at = None;
        self.manifest_refresh_at = None;
        self.order_result.clear();
        self.helm_submissions.clear();
        self.helm_drafts.clear();
        self.helm_preview_pending.clear();
        self.helm_warnings.clear();
        self.last_game_position_at = None;
        self.fix_animation_started.clear();
        self.unit_drag = None;
        self.map_heading_drag = None;
    }

    /// Change the held game through one boundary. Refreshing the same
    /// game keeps its visuals; moving to another game (or releasing
    /// it) drops them before the new hold becomes observable.
    fn set_held_game(&mut self, game: Option<(i64, String)>) {
        let old_id = self.users_game.as_ref().map(|(id, _)| *id);
        let new_id = game.as_ref().map(|(id, _)| *id);
        if old_id != new_id {
            self.clear_visual_cache();
            self.reset_registry_view();
            self.users_game_state = None;
            // A different game's force is not this game's force. The next
            // bundle seeds the draft from whatever that hold already holds.
            self.force_seeded_for = None;
            // The book belongs to a game, so holding another one has to
            // clear it: otherwise the composer opens showing the previous
            // session's scenarios, and a write against them would be
            // refused by a boundary the author cannot see on screen.
            self.scenarios.clear();
            self.composer_scenario = None;
            self.composer_visible = false;
            self.held_detail = None;
            self.composer_draft = ComposerDraft::default();
            self.composer_draft_error = None;
        }
        self.users_game = game;
        if old_id != new_id && self.users_game.is_some() {
            self.load_scenarios();
        }
    }

    /// Stash a fresh pair: access token in memory with a fresh issue
    /// time, refresh token to the keyring (memory fallback, flagged).
    /// Rotations push down to the live actor when one runs.
    fn store_pair(&mut self, user: String, pair: TokenPair) {
        // A token rotation for the same user keeps visuals. Logging in
        // as somebody else is a user boundary even if the previous UI
        // forgot to sign out cleanly.
        if self.auth_user.as_deref() != Some(user.as_str()) {
            self.clear_visual_cache();
        }
        if let Some(rt) = pair.refresh_token.clone() {
            if tfg::backend::keyring_save(&user, &rt).is_err() {
                self.auth_refresh_memory = Some(rt);
                self.auth_degraded = true;
            } else {
                self.auth_refresh_memory = None;
                self.auth_degraded = false;
            }
        }
        // M3: remember whose refresh token to wake with next launch.
        tfg::backend::last_user_save(&self.paths.last_user, &user);
        self.auth_user = Some(user);
        self.auth_token = Some(pair.access_token.clone());
        self.auth_issued_at = Some(Instant::now());
        self.auth_ttl_secs = pair.expires_in;
        if let Some(tx) = &self.live_cmd_tx {
            let _ = tx.send(LiveCmd::SetToken(pair.access_token));
        }
    }

    /// Sign out everywhere: refresh session revoked server-side first
    /// (H9), then keyring entry best-effort, then memory. The socket
    /// actor holds the access token, so it is shut down explicitly
    /// instead of left polling on a dead session. Server refusal is
    /// loud but never blocks the local sign-out.
    fn sign_out(&mut self, why: &str) {
        if let Some(user) = self.auth_user.clone() {
            // H9: revoke while the token is still held. Keyring first,
            // memory fallback — the same order the refresh path reads.
            // M7: fire-and-forget — sign-out never stalls on a server
            // round-trip (best effort by H9's design).
            let stored = match tfg::backend::keyring_load(&user) {
                Ok(Some(rt)) => Some(rt),
                _ => self.auth_refresh_memory.clone(),
            };
            if let Some(rt) = stored {
                let base = self.minos_base.clone();
                std::thread::spawn(move || {
                    match tfg::backend::MinosAuth::new(&base)
                        .and_then(|a| a.logout(Some(&rt)))
                    {
                        Ok(()) => eprintln!("signed out: refresh session revoked"),
                        Err(e) => eprintln!("signed out: server logout refused ({e})"),
                    }
                });
            }
            let _ = tfg::backend::keyring_clear(&user);
        }
        // M3: no session, no wake record — the next launch starts cold.
        tfg::backend::last_user_clear(&self.paths.last_user);
        if let Some(tx) = self.live_cmd_tx.take() {
            let _ = tx.send(LiveCmd::Shutdown);
        }
        self.live_evt_rx = None;
        self.live_state = LinkState::Idle;
                self.live_status = "idle".to_string();
        // A new sign-in's first Connected is a first connect, never a
        // reconnect — the next hold starts cold.
        self.live_connected_once = false;
        self.auth_user = None;
        self.auth_user_id = None;
        self.auth_identity = None;
        self.auth_token = None;
        self.auth_issued_at = None;
        self.auth_ttl_secs = 0;
        self.auth_refresh_memory = None;
        self.auth_needs_password_change = false;
        self.login_password.clear();
        self.pw_current.clear();
        self.pw_new.clear();
        // Full session boundary: nothing from this user or exercise
        // survives for the next sign-in. Abandoned op slots harvest
        // nothing (a dead harvest never applies); hull halves go
        // through the unified release; the next shell entry re-arms
        // and re-defaults from scratch.
        self.login_op = None;
        self.sync_op = None;
        self.spec_op = None;
        self.pw_op = None;
        self.plot_op = None;
        self.setup_op = None;
        self.pending_setup.clear();
        self.pending_transition = None;
        self.release_all_local();
        self.reset_registry_view();
        self.deselect();
        self.following = None;
        self.users_game = None;
        self.users_game_state = None;
        self.users_games.clear();
        self.games_gap = false;
        self.games_loaded = false;
        self.users_roster.clear();
        self.users_gunits.clear();
        self.commanded_hulls.clear();
        self.roster_gap = false;
        self.units_gap = false;
        self.placements_gap = false;
        self.users_placements.clear();
        self.force.clear();
        self.force_seeded_for = None;
        self.force_queue.clear();
        self.force_flush_then = None;
        self.map_unit_move = None;
        self.placement_unplaced = 0;
        self.placement_ready = false;
        self.users_list.clear();
        self.users_role = None;
        self.users_status.clear();
        self.join_key.clear();
        self.inbox.clear();
        self.game_messages.clear();
        self.inbox_page_no = 1;
        self.msg_open = None;
        self.minos_tree.clear();
        self.tree_gap = false;
        // Session boundary: no URL, texture, measurement, or
        // association from the previous user's exercise survives.
        self.clear_visual_cache();
        self.judgements.clear();
        self.judge_score.clear();
        self.reviews.clear();
        self.review_body.clear();
        self.review_editing = None;
        self.timeline_events.clear();
        self.timeline_cursor = None;
        self.timeline_has_more = false;
        self.msg_content.clear();
        self.msg_callsign.clear();
        self.msg_sending_note.clear();
        self.msg_group.clear();
        self.msg_per.clear();
        self.msg_regnum.clear();
        self.msg_to.clear();
        self.msg_cc.clear();
        self.msg_degree = None;
        self.msg_assumed = None;
        self.msg_reply_to = None;
        self.scenario_roles.clear();
        self.new_role_name.clear();
        self.minos_clock = None;
        self.minos_time_factor = None;
        self.minos_room_key = None;
        self.clock_denied = false;
        self.factor_draft = 1.0;
        self.last_seen.clear();
        self.fix_count.clear();
        self.last_game_sync = None;
        self.game_elapsed_secs = None;
        self.game_ratio = 1.0;
        self.game_paused = false;
        self.real_ts = None;
        self.game_ts = None;
        self.order_warning = None;
        self.acting_as = None;
        self.mode.reset();
        self.sim_ready = false;
        self.phase_note = None;
        self.assessment_tab = 0;
        self.auth_status = format!("signed out ({why})");
        eprintln!("signed out ({why})");
    }

    /// Run one sign-in attempt with the island's credentials (M7): the
    /// login + gate probe run off-thread; the frame pump stores the
    /// pair and syncs on completion. Re-clicks while busy are refused.
    fn attempt_sign_in(&mut self) {
        if self.login_op.is_some() {
            self.auth_status = "sign-in already running…".to_string();
            return;
        }
        let base = self.minos_base.clone();
        let id = self.login_identifier.trim().to_string();
        if id.is_empty() {
            self.auth_status = "sign-in failed: identifier is empty".to_string();
            return;
        }
        let password = self.login_password.clone();
        self.auth_status = format!("signing in as {id}…");
        self.login_op = Some(spawn_rest("sign-in", move || {
            let client =
                MinosAuth::new(&base).map_err(|e| format!("sign-in failed: {e}"))?;
            let pair = client
                .login(&id, &password)
                .map_err(|e| format!("sign-in failed: {e}"))?;
            // M2: the courtesy flag routes straight to the change form.
            if pair.must_change_password {
                return Ok(LoginDone {
                    user: id,
                    pair,
                    identity: None,
                    needs_change: true,
                    probe_note: None,
                });
            }
            let token = pair.access_token.clone();
            match client.me(&token) {
                Ok(identity) => Ok(LoginDone {
                    user: id,
                    pair,
                    identity: Some(identity),
                    needs_change: false,
                    probe_note: None,
                }),
                Err(tfg::backend::BackendError::Forbidden { .. }) => Ok(LoginDone {
                    user: id,
                    pair,
                    identity: None,
                    needs_change: true,
                    probe_note: None,
                }),
                Err(e) => Ok(LoginDone {
                    user: id.clone(),
                    pair,
                    identity: None,
                    needs_change: false,
                    probe_note: Some(format!("signed in as {id} · probe: {e}")),
                }),
            }
        }));
    }

    /// Apply a finished sign-in on the UI thread: store, probe outcome,
    /// then sync (which spawns its own op — never blocks here).
    fn apply_login(&mut self, done: LoginDone) {
        self.login_password.clear();
        self.store_pair(done.user.clone(), done.pair);
        self.auth_identity = done.identity.clone();
        if done.needs_change {
            self.auth_needs_password_change = true;
            self.auth_status = format!("signed in as {} · must change password", done.user);
            return;
        }
        match done.identity.as_ref().map(|i| i.id) {
            Some(uid) => {
                self.auth_user_id = Some(uid);
                self.watch_personal_channel();
                self.auth_needs_password_change = false;
                self.auth_status = format!("signed in as {}", done.user);
                // Login syncs (master-data ticket): the working copy
                // refreshes on every entry, off-thread.
                self.sync_now();
                // The session list too.
                //
                // `sign_out` clears it as part of the session boundary, so
                // signing back in re-entered the console with an EMPTY
                // session list and nothing re-read it. Found by driving: sign
                // out, sign in, and the no-session island offered only
                // "create a session" — the picker row was gone, because the
                // predicate that draws it also asks whether the list has
                // loaded. The list is what the console is for; a sign-in that
                // does not fetch it is a sign-in that starts blind.
                self.users_refresh_games();
            }
            None => {
                self.auth_status = done.probe_note.unwrap_or_else(|| format!("signed in as {}", done.user));
            }
        }
    }

    /// One refresh attempt, now (M7: off-thread — the proactive tick
    /// and the 109 path share it, and neither may stall frames).
    /// Keyring first, memory fallback, loud sign-out when rotation
    /// fails. Re-calls while busy are ignored: the in-flight rotation
    /// is the freshest ask.
    fn refresh_now(&mut self) {
        if self.refresh_op.is_some() {
            return;
        }
        let Some(user) = self.auth_user.clone() else {
            return;
        };
        let base = self.minos_base.clone();
        let stored = match tfg::backend::keyring_load(&user) {
            Ok(Some(rt)) => Some(rt),
            Ok(None) => self.auth_refresh_memory.clone(),
            Err(e) => {
                eprintln!("keyring unreadable ({e}); memory fallback");
                self.auth_refresh_memory.clone()
            }
        };
        match stored {
            Some(rt) => {
                self.refresh_op = Some(spawn_rest("refresh", move || {
                    MinosAuth::new(&base)
                        .and_then(|a| a.refresh(&rt))
                        .map_err(|e| e.to_string())
                }));
            }
            None => {
                self.sign_out("refresh token missing");
            }
        }
    }

    /// Apply a finished rotation: store, or sign out loudly on failure
    /// (unchanged semantics — only the thread moved). The pump clears
    /// the slot before calling.
    fn apply_refresh(&mut self, user: String, res: Result<tfg::backend::TokenPair, String>) {
        match res {
            Ok(pair) => {
                self.store_pair(user.clone(), pair);
                self.auth_status = format!("signed in as {user} · refreshed");
                eprintln!("auth refreshed for {user}");
            }
            Err(e) => {
                self.sign_out(&format!("refresh failed: {e}"));
            }
        }
    }

    /// Wake the last session at launch (M3): the recorded identifier's
    /// keyring refresh token rotates into a live pair, the gate probe
    /// confirms the id, and the working copy syncs — the same entry
    /// path as a fresh sign-in. Silent when there is nothing to wake
    /// (cold launch asks for login). A refused rotation clears the
    /// wake record so one revoked token cannot fail every launch; an
    /// unreachable backend keeps it for later.
    fn restore_session(&mut self) {
        let Some(user) = tfg::backend::last_user_load(&self.paths.last_user) else {
            return;
        };
        let stored = match tfg::backend::keyring_load(&user) {
            Ok(Some(rt)) => rt,
            _ => return,
        };
        let base = self.minos_base.clone();
        let client = match MinosAuth::new(&base) {
            Ok(c) => c,
            Err(e) => {
                self.auth_status = format!("session restore failed: {e}");
                return;
            }
        };
        match client.refresh(&stored) {
            Ok(pair) => {
                let must_change = pair.must_change_password;
                self.store_pair(user.clone(), pair);
                if must_change {
                    self.auth_needs_password_change = true;
                    self.auth_status =
                        format!("resumed as {user} · must change password");
                    return;
                }
                self.auth_identity = None;
                match client.me(self.auth_token.as_deref().unwrap_or("")) {
                    Ok(identity) => {
                        self.auth_user_id = Some(identity.id);
                        self.auth_identity = Some(identity);
                        self.auth_needs_password_change = false;
                        self.auth_status = format!("resumed session as {user}");
                        eprintln!("session restored for {user}");
                        self.sync_now();
                    }
                    Err(tfg::backend::BackendError::Forbidden { .. }) => {
                        self.auth_needs_password_change = true;
                        self.auth_status =
                            format!("resumed as {user} · must change password");
                    }
                    Err(e) => {
                        self.auth_status = format!("resumed as {user} · probe: {e}");
                    }
                }
            }
            Err(tfg::backend::BackendError::Transport(_)) => {
                // Offline, not revoked: keep the record and stay cold.
                eprintln!("session restore: backend unreachable, staying signed out");
            }
            Err(e) => {
                eprintln!("session restore refused ({e}); clearing wake record");
                tfg::backend::last_user_clear(&self.paths.last_user);
                let _ = tfg::backend::keyring_clear(&user);
            }
        }
    }

    /// Full master-data sync, off-thread (M7: server wins, whole-table
    /// replace). Runs after sign-in and on the Sync button; failures
    /// report loudly, never half-applied silently (replace_all commits
    /// per table, counts recorded). The connection moves into the
    /// worker and rides back in the result.
    fn sync_now(&mut self) {
        if self.sync_op.is_some() {
            self.sync_status = "sync already running…".to_string();
            return;
        }
        let Some(tok) = self.auth_token.clone() else {
            self.sync_status = "sign in first".to_string();
            return;
        };
        let base = self.minos_base.clone();
        let Some(conn) = self.store.take() else {
            self.sync_status = "store unavailable".to_string();
            return;
        };
        self.sync_status = "syncing…".to_string();
        self.sync_op = Some(spawn_rest("sync", move || {
            let master =
                tfg::backend::MinosMaster::new(&base).map_err(|e| e.to_string())?;
            let mut conn = conn;
            let counts = tfg::store::sync_from(&master, &tok, &mut conn)?;
            Ok(SyncDone { conn, counts })
        }));
    }

    /// Apply a finished sync: hand the connection back, render counts.
    /// Offline failure keeps the old working copy (nothing cleared).
    fn apply_sync(&mut self, res: Result<SyncDone, String>) {
        match res {
            Ok(done) => {
                self.store = Some(done.conn);
                // The mirror moved — the Fleet cache follows it now,
                // not on the next render. Any already-active visual
                // version also gets its taxonomy-derived symbol again.
                self.reload_fleet_cache();
                self.reload_unit_symbols();
                self.reload_affiliations();
                self.hydrate_visual_facts();
                // A register sync is an explicit operator action; use it
                // as a prompt to check the manifest too, so a newly
                // uploaded hull picture does not wait for the cadence.
                self.manifest_refresh_at = Some(Instant::now());
                let total: usize = done.counts.iter().map(|(_, n)| n).sum();
                let detail: Vec<String> =
                    done.counts.iter().map(|(t, n)| format!("{t} {n}")).collect();
                self.sync_status = format!("synced {total} rows: {}", detail.join(", "));
                eprintln!("sync ok: {}", self.sync_status);
            }
            Err(e) => {
                // Connection rides back only on success — on failure the
                // worker consumed it. Reopen so later syncs can retry.
                self.store = tfg::store::open(&self.paths.local_db).ok();
                self.sync_status = format!("sync failed: {e}");
                eprintln!("sync failed: {e}");
            }
        }
    }

    /// Bulk spec backfill, off-thread (M7): every register hull
    /// without stored figures gets one detail fetch. Skips what is
    /// known (versions are immutable, refetch buys nothing). The
    /// figures upsert into both catalogs on apply.
    fn sync_specs_now(&mut self) {
        if self.spec_op.is_some() {
            self.sync_status = "spec fetch already running…".to_string();
            return;
        }
        let Some(tok) = self.auth_token.clone() else {
            self.sync_status = "sign in first".to_string();
            return;
        };
        let base = self.minos_base.clone();
        let Some(conn) = self.store.take() else {
            self.sync_status = "store unavailable".to_string();
            return;
        };
        self.sync_status = "fetching hull specs…".to_string();
        self.spec_op = Some(spawn_rest("specs", move || {
            let master = tfg::backend::MinosMaster::new(&base).map_err(|e| e.to_string())?;
            let ids = tfg::store::unit_ids(&conn)?;
            let mut specs = Vec::new();
            let (mut fetched, mut skipped, mut failed) = (0usize, 0usize, 0usize);
            for uid in ids {
                match tfg::store::spec_versions(&conn, uid) {
                    Ok(v) if !v.is_empty() => {
                        skipped += 1;
                        continue;
                    }
                    Err(e) => return Err(format!("spec sync failed: {e}")),
                    _ => {}
                }
                match master.sync_spec(&tok, &conn, uid) {
                    Ok(spec) => {
                        fetched += 1;
                        if spec.speed_kn.is_some() {
                            specs.push(spec);
                        }
                        if fetched % 10 == 0 {
                            eprintln!("spec sync: {fetched} fetched");
                        }
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!("spec sync: hull {uid} failed: {e}");
                    }
                }
            }
            Ok(SpecDone { conn, specs, fetched, skipped, failed })
        }));
    }

    /// Apply finished specs: connection back, figures into the UI
    /// catalog and pushed down to the sim's own catalog (H10).
    fn apply_specs(&mut self, res: Result<SpecDone, String>) {
        match res {
            Ok(done) => {
                self.store = Some(done.conn);
                for spec in &done.specs {
                    let speed = spec.speed_kn.unwrap_or(0.0);
                    self.catalog.upsert_runtime_class(
                        spec.class_id,
                        spec.class_name.clone(),
                        spec.version,
                        speed,
                        spec.cruise_kn.unwrap_or(0.0),
                        spec.range_nm.unwrap_or(0.0),
                    );
                    if let Some(tx) = &self.sim_cmd_tx {
                        let _ = tx.send(SimCommand::UpsertClass {
                            minos_class_id: spec.class_id,
                            name: spec.class_name.clone(),
                            version: spec.version,
                            speed_kn: speed,
                            cruise_kn: spec.cruise_kn.unwrap_or(0.0),
                            range_nm: spec.range_nm.unwrap_or(0.0),
                        });
                    }
                }
                // Published physical measurements feed the visual
                // model even when the asset version did not move.
                self.hydrate_visual_facts();
                self.sync_status = format!(
                    "specs: {} fetched, {} already known, {} failed",
                    done.fetched, done.skipped, done.failed
                );
                eprintln!("spec sync done: {}", self.sync_status);
            }
            Err(e) => {
                self.store = tfg::store::open(&self.paths.local_db).ok();
                self.sync_status = format!("spec sync failed: {e}");
            }
        }
    }

    /// Session-users client (build ticket): env-owned endpoint plus the
    /// in-memory access token. Every call below is a blocking operator
    /// action — callers spawn it onto a RestOp worker (#100) and apply
    /// the result in pump_rest_ops, never on the egui thread.
    fn users_client(&self) -> Result<(MinosMaster, String), tfg::backend::BackendError> {
        let tok = self
            .auth_token
            .clone()
            .ok_or_else(|| tfg::backend::BackendError::Other("sign in first".into()))?;
        let master = MinosMaster::new(&self.minos_base)?;
        Ok((master, tok))
    }

    /// Judge-side test for commander assignment: the contract is that
    /// the judge side never commands (a piece commanded by a judge is
    /// a player wearing a referee's shirt). The roster row's flag is
    /// primary, and the synced `game_roles` mirror backs it — that
    /// lookup is the authority on `is_judge_side`, and a flag that
    /// failed to parse must never read as "not a judge".
    fn users_is_judge(&self, p: &tfg::backend::Participant) -> bool {
        p.judge
            || self
                .users_roles
                .iter()
                .any(|(id, _, _, judge)| *id == p.role_id && *judge)
    }

    /// #100: one setup op at a time — queue refreshes, refuse writes.
    /// Reads that land while busy wait their turn (deduped); writes
    /// refuse loudly instead of piling conflicting writes.
    fn setup_busy(&mut self, what: &str) -> bool {
        if self.setup_op.is_some() {
            self.users_status = format!("{what} already running…");
            true
        } else {
            false
        }
    }

    /// Queue a refresh kind for the pump to dispatch once the slot
    /// frees (#100). Writes never queue — only reads repeat safely.
    fn queue_refresh(&mut self, kind: PendingRefresh) {
        if !self.pending_setup.contains(&kind) {
            self.pending_setup.push(kind);
        }
    }

    /// Dispatch one queued refresh when the slot is free. Called by
    /// the pump after every applied setup op.
    fn dispatch_queued_refresh(&mut self) {
        if self.setup_op.is_some() {
            return;
        }
        let Some(kind) = self.pending_setup.first().cloned() else {
            return;
        };
        self.pending_setup.remove(0);
        match kind {
            PendingRefresh::Games => self.users_refresh_games(),
            PendingRefresh::Directory => self.users_refresh_directory(),
            PendingRefresh::Game => self.users_refresh_game(),
            PendingRefresh::Tree => self.load_minos_tree(),
        }
    }

    fn users_refresh_games(&mut self) {
        if self.setup_op.is_some() {
            self.queue_refresh(PendingRefresh::Games);
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("games failed: {e}");
                return;
            }
        };
        self.setup_op = Some(spawn_rest("games", move || {
            // Typed inside the worker: a Forbidden list is the
            // player-flow signal, never a fabricate-nothing error.
            match master.games_list(&tok) {
                Ok(g) => Ok(SetupDone::Games(g)),
                Err(tfg::backend::BackendError::Forbidden { .. }) => {
                    Ok(SetupDone::GamesDenied)
                }
                Err(e) => Err(e.to_string()),
            }
        }));
    }

    /// Apply a finished game list: hold the vanished-game drop and the
    /// count line on the UI thread. A surviving hold chains into the
    /// bundle (detail + roster + units + placements) — refreshes that
    /// need both lists funnel through here, never past a busy slot.
    fn apply_games(&mut self, games: Vec<tfg::backend::GameRow>) {
        self.users_games = games;
        self.games_gap = false;
        self.games_loaded = true;
        // A held game may have closed or vanished: drop it loudly.
        if let Some((gid, _)) = self.users_game.clone() {
            if !self.users_games.iter().any(|g| g.id == gid) {
                self.drop_hold("held session is gone — pick another");
                return;
            }
        }
        self.users_status = format!("{} game(s)", self.users_games.len());
        if self.users_game.is_some() {
            // Subsumes any queued Game refresh — one bundle, not two.
            self.pending_setup.retain(|k| *k != PendingRefresh::Game);
            self.spawn_bundle();
        }
    }

    /// Drop the hold with its per-game state: roster, pieces, tree,
    /// placements, gaps, clock. Shared by the vanished-game drop and
    /// the delete apply — a deleted game reads exactly like a
    /// vanished one. Gap flags clear: they described the old hold's
    /// reads, and the bundle re-marks them.
    fn drop_hold(&mut self, why: &str) {
        self.set_held_game(None);
        self.users_game_state = None;
        self.minos_clock = None;
        self.minos_time_factor = None;
        self.minos_room_key = None;
        self.clock_denied = false;
        self.watch_game_channel();
        self.users_roster.clear();
        self.users_gunits.clear();
        self.commanded_hulls.clear();
        self.minos_tree.clear();
        self.tree_gap = false;
        self.users_placements.clear();
        self.force.clear();
        self.force_seeded_for = None;
        self.force_queue.clear();
        self.force_flush_then = None;
        self.map_unit_move = None;
        self.placement_unplaced = 0;
        self.placement_ready = false;
        self.roster_gap = false;
        self.units_gap = false;
        self.placements_gap = false;
        self.clear_fleet_pick();
        self.edit_open = false;
        self.delete_armed = false;
        // The pictures belonged to the exercise just released: their
        // URLs, textures, and measurements do not follow a new hold.
        self.clear_visual_cache();
        self.users_status = why.to_string();
    }

    /// Spawn the bundled held-game read (detail + roster + units +
    /// setup view). Shared by refreshes and apply chains. A busy slot
    /// queues the Game refresh instead of dropping it.
    fn spawn_bundle(&mut self) {
        let Some((gid, _)) = self.users_game.clone() else {
            return;
        };
        if self.setup_op.is_some() {
            self.queue_refresh(PendingRefresh::Game);
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("game refresh failed: {e}");
                return;
            }
        };
        self.setup_op = Some(spawn_rest("game", move || {
            // Detail is authoritative: its failure aborts the bundle
            // (nothing applies, verdict stays pending for the retry).
            // Each subordinate degrades independently below.
            let detail = master
                .game_detail(&tok, gid)
                .map_err(|e| format!("session state unread: {e}"))?;
            let roster = master.game_participants(&tok, gid);
            let units = master.game_units_list(&tok, gid);
            let placements = master.placements_list(&tok, gid);
            let readiness = master.game_readiness(&tok, gid);
            Ok(SetupDone::Bundle(GameBundle { detail, roster, units, placements, readiness }))
        }));
    }

    fn users_refresh_directory(&mut self) {
        // Vocabularies ride the synced helpers mirror (cheap, always):
        // game roles for seating, statuses + app roles for filters,
        // message degrees ([7.7] Derajat) for the compose form.
        if let Some(conn) = self.store.as_ref() {
            self.users_roles = tfg::store::helper_list(conn, "game_roles").unwrap_or_default();
            self.users_statuses = tfg::store::helper_list(conn, "user_statuses").unwrap_or_default();
            self.users_approles = tfg::store::helper_list(conn, "app_roles").unwrap_or_default();
            self.msg_degrees = tfg::store::helper_list(conn, "message_degrees").unwrap_or_default();
            if self.users_role.is_none() {
                self.users_role = self
                    .users_roles
                    .iter()
                    .find(|(_, n, _, _)| n == "Commando")
                    .map(|(id, _, _, _)| *id);
            }
        }
        let query = self.users_search.clone();
        let status = self.users_filter_status;
        let app_role = self.users_filter_role;
        if self.setup_op.is_some() {
            self.queue_refresh(PendingRefresh::Directory);
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_list.clear();
                self.users_status =
                    format!("directory unavailable (needs read on /system/users): {e}");
                return;
            }
        };
        self.setup_op = Some(spawn_rest("directory", move || {
            master
                .users_list(&tok, &query, status, app_role)
                .map_err(|e| e.to_string())
                .map(SetupDone::Users)
        }));
    }

    /// Apply a finished directory page. Failures keep the last good
    /// list when one exists; the 403 fallback message is unchanged.
    fn apply_users(&mut self, users: Vec<tfg::backend::BackendUser>) {
        self.users_list = users;
        self.users_status = format!("directory: {} account(s)", self.users_list.len());
    }


    /// H1: move the local machine to the authoritative state. Guards on
    /// the current phase keep steady-state refreshes (seating, assigns
    /// during preparation; ticks during execution) no-ops: only a
    /// mismatch moves anything.
    fn users_project_stage(&mut self, state: &str) {
        match state {
            "planning" => {
                self.sim_ready = false;
                if self.mode.phase != Phase::Setup {
                    self.mode.reset();
                    self.close_working_islands();
                    self.setup_step = 0;
                    self.phase_note = None;
                    // #101: reset disarms — re-arm for the new Setup,
                    // like shell entry does.
                    self.mode.armed.store(true, Ordering::SeqCst);
                    // M6: a different game means Setup — freeze.
                    self.hold_sim_for_setup();
                }
            }
            "preparation" => {
                self.sim_ready = true;
                if self.mode.phase != Phase::Setup {
                    self.mode.reset();
                    self.close_working_islands();
                    self.setup_step = 0;
                    self.phase_note = None;
                    // #101: reset disarms — re-arm for the new Setup,
                    // like shell entry does.
                    self.mode.armed.store(true, Ordering::SeqCst);
                    // M6: a different game means Setup — freeze.
                    self.hold_sim_for_setup();
                }
            }
            "execution" => {
                self.sim_ready = true;
                // C3: Minos drives game pieces from here — release local
                // legs first so no ghost track diverges, then plot the
                // authoritative markers (even when the engine refuses).
                self.release_game_pieces();
                // #98: fresh streak per entry — past failures belong to
                // the previous execution.
                self.plot_fails = 0;
                self.last_plot_ok = None;
                if self.mode.phase != Phase::Live {
                    self.start_session();
                    if self.mode.phase != Phase::Live {
                        self.phase_note = Some(
                            "The exercise entered execution, but the local session could not start (see log)".to_string(),
                        );
                    } else {
                        self.phase_note = None;
                    }
                }
                self.pull_minos_positions();
            }
            "closure" => {
                self.sim_ready = false;
                if self.mode.phase != Phase::Closed {
                    self.end_session();
                }
            }
            _ => {}
        }
    }

    /// Tell the socket actor which game channels to watch. The broadcast
    /// channel carries messages/order events; the positions channel carries
    /// the authoritative movement snapshot stream.
    fn watch_game_channel(&self) {
        if let Some(tx) = &self.live_cmd_tx {
            let game_id = if self.app_mode == AppMode::Simulation {
                self.users_game.as_ref().map(|(id, _)| *id)
            } else {
                None
            };
            let position_game_id = if self.users_game_state.as_deref() == Some("execution") {
                game_id
            } else {
                None
            };
            let _ = tx.send(LiveCmd::WatchGame(game_id));
            let _ = tx.send(LiveCmd::WatchGamePositions(position_game_id));
        }
    }

    /// H11: tell the socket actor which personal channel to watch.
    fn watch_personal_channel(&self) {
        if let Some(tx) = &self.live_cmd_tx {
            let _ = tx.send(LiveCmd::WatchPersonal(self.auth_user_id));
        }
    }

    fn users_refresh_game(&mut self) {
        // H11: the watched broadcast channel follows the hold —
        // including to None, which unsubscribes.
        self.watch_game_channel();
        if self.users_game.is_none() {
            self.held_detail = None;
            self.users_roster.clear();
            self.users_gunits.clear();
            self.commanded_hulls.clear();
            self.users_placements.clear();
            self.force.clear();
            self.force_seeded_for = None;
            self.force_queue.clear();
            self.force_flush_then = None;
            self.map_unit_move = None;
            self.placement_unplaced = 0;
            self.placement_ready = false;
            self.roster_gap = false;
            self.units_gap = false;
            self.placements_gap = false;
            return;
        }
        // Detail + roster + units + setup view in one worker trip; the
        // H1 projection runs on apply, so a transition another client
        // made still lands instead of disagreeing. Subordinates degrade
        // independently — a staff-only 403 marks a gap, never moves the
        // stage, never empties a list.
        self.spawn_bundle();
    }

    /// Apply a finished bundle: detail first (hold, lists seed from the
    /// projection), verdict against the fresh state, then each
    /// subordinate degrades independently — a staff-only 403 keeps the
    /// last good list behind a gap flag, never an empty list as truth.
    fn apply_bundle(&mut self, b: GameBundle) {
        let d = b.detail;
        self.set_held_game(Some((d.id, d.name.clone())));
        self.users_game_state = Some(d.state.clone());
        // The WHOLE detail is kept, not destructured away. It used to be read
        // for six fields and the rest thrown out, which is how `description`
        // ended up write-only: the create form wrote it, the parser dropped
        // it, and the Essentials island had nothing to show after a reload.
        // Keeping the detail makes the next unparsed field a visible omission
        // rather than a silent one.
        self.held_detail = Some(d.clone());
        // H2: the chosen rate rides the detail — a mid-exercise select
        // learns the clock it is joining.
        self.minos_time_factor =
            if d.time_factor > 0.0 { Some(d.time_factor) } else { None };
        // The room key rides the detail from preparation onward — the
        // Game Master's share-out to personnel.
        self.minos_room_key = d.room_key.clone();
        // #100: a refused transition left a pending verdict — the fresh
        // state above decides it.
        self.apply_transition_verdict();
        // The detail is the sync: stamp even when subordinates gap.
        self.last_game_sync = Some(Instant::now());
        let roster_seg = match b.roster {
            Ok(r) => {
                self.users_roster = r;
                self.roster_gap = false;
                format!("roster: {}", self.users_roster.len())
            }
            Err(tfg::backend::BackendError::Forbidden { .. }) => {
                self.roster_gap = true;
                "roster: staff access unavailable".to_string()
            }
            Err(e) => format!("roster failed: {e}"),
        };
        // The gate. Degrades like any other subordinate: a failed read keeps
        // the last good verdict behind a gap flag, because the alternative is
        // an empty blocker list, and an empty list says "nothing is
        // outstanding" — the one answer that must never be invented.
        let readiness_seg = match b.readiness {
            Ok(view) => {
                let blockers = view.blockers.len();
                self.readiness = Some(view);
                self.readiness_gap = ReadinessGap::NotRead;
                format!("readiness: {blockers} outstanding")
            }
            Err(e) => {
                self.readiness_gap = ReadinessGap::of(&e);
                format!("readiness unreadable: {e}")
            }
        };
        let mut units_ok = false;
        let units_seg = match b.units {
            Ok(u) => {
                units_ok = true;
                self.users_gunits = u;
                self.refresh_unassigned_units();
                self.units_gap = false;
                format!("units: {}", self.users_gunits.len())
            }
            Err(tfg::backend::BackendError::Forbidden { .. }) => {
                self.units_gap = true;
                "units: staff access unavailable".to_string()
            }
            Err(e) => format!("units failed: {e}"),
        };
        // Project execution only after the fresh unit rows are installed;
        // otherwise release_game_pieces cannot see newly authoritative
        // hulls and can leave a local sim authority alive.
        self.users_project_stage(&d.state);
        self.watch_game_channel();
        let placements_seg = match b.placements {
            Ok(view) => {
                self.apply_placements(view);
                self.placements_gap = false;
                format!(
                    "placements: {} placed, {} to go",
                    self.users_placements.len(),
                    self.placement_unplaced
                )
            }
            Err(tfg::backend::BackendError::Forbidden { .. }) => {
                self.placements_gap = true;
                "placements: staff access unavailable".to_string()
            }
            Err(e) => format!("placements failed: {e}"),
        };
        // Seed the local force draft ONCE per hold, from the server's copy.
        // The draft is what the operator edits; a hold that already has a
        // force on it must show that force, or the islands would read empty
        // over a game Minos knows holds six hulls. Later re-reads refresh the
        // live mirror and leave the draft alone — the diff is computed
        // against the mirror, so the operator's unsaved edits survive.
        if units_ok && self.force_seeded_for != Some(d.id) {
            self.seed_force_from_live();
            self.force_seeded_for = Some(d.id);
        }
        self.users_status = format!(
            "session synced ({}) · {roster_seg} · {units_seg} · {placements_seg} · {readiness_seg}",
            d.state
        );
    }

    /// Build the local force draft from what Minos already holds, and stand
    /// the placed hulls up on the local map. Called once per hold.
    fn seed_force_from_live(&mut self) {
        self.force.clear();
        let live = self.users_gunits.clone();
        for gu in &live {
            let start = self
                .users_placements
                .iter()
                .find(|p| p.unit_id == gu.unit_id)
                .map(|p| tfg::force::Start {
                    lat: p.latitude,
                    lon: p.longitude,
                });
            let class_id = self
                .placement_seed(&gu.unit_id.to_string())
                .and_then(|(_, _, class_id)| class_id)
                .unwrap_or_default();
            let mut hull = tfg::force::DraftHull::new(
                gu.unit_id,
                gu.unit_name.clone(),
                gu.hull_number.clone(),
                class_id,
            );
            hull.start = start;
            hull.commander_id = gu.commander_id;
            self.force.upsert(hull);
        }
        // The map's half: a hull Minos placed has to draw, or the force is a
        // list of names over an empty sea.
        let placed: Vec<(i64, String, String, String, f64, f64)> = self
            .force
            .hulls()
            .filter_map(|h| {
                let start = h.start?;
                Some((
                    h.unit_id,
                    h.name.clone(),
                    h.hull_number.clone(),
                    h.class_id.clone(),
                    start.lat,
                    start.lon,
                ))
            })
            .collect();
        for (unit_id, name, hull_number, class_id, lat, lon) in placed {
            if class_id.is_empty() {
                // No sim stats resolved for this hull, so standing it up
                // would be inventing its abilities. It stays in the list
                // and off the map until the register supplies figures.
                continue;
            }
            let id = unit_id.to_string();
            if let Some(tx) = &self.sim_cmd_tx {
                let _ = tx.send(SimCommand::TakeControl {
                    ship_id: id.clone(),
                    pos: GeoPosition { latitude: lat, longitude: lon },
                    class_id,
                });
            }
            self.controlled.insert(id.clone());
            self.placed_labels.insert(id.clone(), (name, hull_number));
            self.placed_fleet.insert(id);
        }
        self.refresh_unassigned_units();
    }

    fn users_add(&mut self, user_id: i64) {
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let Some(role) = self.users_role else {
            self.users_status = "pick a role first".to_string();
            return;
        };
        if self.setup_busy("seat") {
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("add failed: {e}");
                return;
            }
        };
        // The seat response IS the roster — no read-after-write.
        let note = format!("seated {user_id} in game {gid}");
        self.setup_op = Some(spawn_rest("seat", move || {
            master
                .add_participant(&tok, gid, user_id, role)
                .map_err(|e| e.to_string())
                .map(|r| SetupDone::Roster(r, note))
        }));
    }

    /// Move a seated person to another game role. The write clears that
    /// seat's readiness, so the status line says so out loud.
    fn users_change_role(&mut self, user_id: i64, role_id: i64) {
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let who = self
            .users_roster
            .iter()
            .find(|p| p.user_id == user_id)
            .map(|p| p.user_name.clone())
            .unwrap_or_else(|| format!("user {user_id}"));
        let role = self
            .users_roles
            .iter()
            .find(|(id, _, _, _)| *id == role_id)
            .map(|(_, n, _, _)| n.clone())
            .unwrap_or_else(|| format!("role {role_id}"));
        if self.setup_busy("role change") {
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("role change failed: {e}");
                return;
            }
        };
        let note = format!("{who} → {role} (readiness cleared)");
        self.setup_op = Some(spawn_rest("role", move || {
            master
                .set_participant_role(&tok, gid, user_id, role_id)
                .map_err(|e| e.to_string())
                .map(|r| SetupDone::Roster(r, note))
        }));
    }

    /// Take a person off the roster. Removal of an absent seat is not
    /// an error upstream (idempotent), so the button never lies.
    fn users_remove(&mut self, user_id: i64) {
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let who = self
            .users_roster
            .iter()
            .find(|p| p.user_id == user_id)
            .map(|p| p.user_name.clone())
            .unwrap_or_else(|| format!("user {user_id}"));
        if self.setup_busy("remove") {
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("remove failed: {e}");
                return;
            }
        };
        let note = format!("removed {who} from game {gid}");
        self.setup_op = Some(spawn_rest("unseat", move || {
            master
                .remove_participant(&tok, gid, user_id)
                .map_err(|e| e.to_string())
                .map(|r| SetupDone::Roster(r, note))
        }));
    }

    /// Hand a hull to a commander. LOCAL FIRST for a staged hull — the
    /// draft records it and Minos hears about it at a stage advance. A hull
    /// the draft does not hold is a commanded hull in the staff-gap path,
    /// whose force lives only on the server, so that one is written through
    /// as it always was.
    fn users_command(&mut self, unit_id: i64, commander_id: i64) {
        if self.force.set_commander(unit_id, Some(commander_id)) {
            self.users_status = format!("unit {unit_id} now commanded by {commander_id}");
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        if self.setup_busy("command") {
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("command failed: {e}");
                return;
            }
        };
        self.setup_op = Some(spawn_rest("command", move || {
            master
                .set_unit_commander(&tok, gid, unit_id, commander_id)
                .map(SetupDone::ForceUnits)
                .map_err(|e| e.to_string())
        }));
    }

    /// Session-users island (build ticket): game picker, live directory
    /// with role pick + add, roster, and per-unit commander assigns.
    /// Layout thesis: heading → game → status → directory → roster →
    /// units, each separated, nav-free (one screen, no steps).
    /// Setup flow step 2, directory half (#79): live account search
    /// with filters, seat-as role, and add. Split out of the old
    /// session-users island — the game picker moved to step 1, the
    /// pieces list to step 3. Dragging a row still seats commanders
    /// onto map markers.
    fn users_directory_ui(&mut self, ui: &mut egui::Ui) {
        // User-drag gesture (same press-origin pattern as hull rows):
        // candidate until past the click threshold, then a live drag.
        if let Some(start) = self.upress.as_ref().map(|(_, _, s)| *s) {
            let (down, dist) = ui.ctx().input(|i| {
                (
                    i.pointer.any_down(),
                    i.pointer.hover_pos().map_or(0.0, |p| p.distance(start)),
                )
            });
            if !down {
                self.upress = None;
            } else if dist > 6.0 {
                if let Some((uid, name, _)) = self.upress.take() {
                    self.udrag = Some((uid, name));
                }
            }
        }
        if let Some((_, name)) = &self.udrag {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            ui.label(format!("moving {name} — release over a unit marker or group flag"));
        }
        ui.strong("Directory");
        ui.horizontal(|ui| {
            ui.label("search:");
            ui.text_edit_singleline(&mut self.users_search);
            if ui.small_button("find").clicked() {
                self.users_refresh_directory();
            }
        });
        // Filters resolve from the helpers mirror and arm the next
        // `find` — nothing re-queries on change (blocking reads stay
        // explicit operator actions). "seat as" is the game role the
        // add button will use; status / app role are account filters.
        ui.horizontal(|ui| {
            let status_name = self
                .users_filter_status
                .and_then(|s| self.users_statuses.iter().find(|(id, _, _, _)| *id == s))
                .map(|(_, n, _, _)| n.clone())
                .unwrap_or_else(|| "any".to_string());
            egui::ComboBox::from_label("status")
                .selected_text(status_name)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.users_filter_status, None, "any");
                    for (id, name, _, _) in &self.users_statuses {
                        ui.selectable_value(&mut self.users_filter_status, Some(*id), name);
                    }
                });
            let app_name = self
                .users_filter_role
                .and_then(|r| self.users_approles.iter().find(|(id, _, _, _)| *id == r))
                .map(|(_, n, _, _)| n.clone())
                .unwrap_or_else(|| "any".to_string());
            egui::ComboBox::from_label("app role")
                .selected_text(app_name)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.users_filter_role, None, "any");
                    for (id, name, _, _) in &self.users_approles {
                        ui.selectable_value(&mut self.users_filter_role, Some(*id), name);
                    }
                });
            let seat_role = self
                .users_role
                .and_then(|r| self.users_roles.iter().find(|(id, _, _, _)| *id == r))
                .map(|(_, n, _, _)| n.clone())
                .unwrap_or_else(|| "pick".to_string());
            egui::ComboBox::from_label("seat as")
                .selected_text(seat_role)
                .show_ui(ui, |ui| {
                    for (id, name, _, _) in &self.users_roles {
                        ui.selectable_value(&mut self.users_role, Some(*id), name);
                    }
                });
            // Say what the chosen role will imply BEFORE the seat is taken.
            // A commander with no fleet is discovered when the exercise
            // starts, which is the worst possible moment; saying it here
            // costs one line and the operator can ignore it.
            if let Some((_, name, _, judge)) = self
                .users_role
                .and_then(|r| self.users_roles.iter().find(|(id, _, _, _)| *id == r))
                .cloned()
            {
                if role_may_need_command(judge) {
                    ui.label(
                        egui::RichText::new(format!(
                            "{name} may need a command \u{2014} assign a hull after seating"
                        ))
                        .weak()
                        .small(),
                    );
                } else {
                    ui.label(
                        egui::RichText::new(format!(
                            "{name} is judge side \u{2014} no command, exempt from readiness"
                        ))
                        .weak()
                        .small(),
                    );
                }
            }
        });
        let mut seating: Vec<i64> = Vec::new();
        egui::ScrollArea::vertical().id_salt("users-dir").max_height(170.0).show(
            ui,
            |ui| {
                if self.users_list.is_empty() {
                    // The server's own words, or nothing.
                    //
                    // This used to be a fixed "No accounts — sign in, then
                    // refresh." A render showed it on a signed-in operator
                    // whose account WAS in the directory: the picker opens
                    // without reading it, so the list is empty before any
                    // read, and the sentence blamed the one thing that was
                    // not wrong. `users_status` already carries the reason
                    // (a refused read, a filter that matched nothing, a
                    // directory mid-flight), so say that instead of inventing
                    // a cause. An empty state that guesses is the same class
                    // of lie as a button that promises a route the server
                    // does not publish.
                    let why = self.users_status.clone();
                    if why.is_empty() {
                        ui.weak("Directory not read yet — press find.");
                    } else {
                        ui.weak(why);
                    }
                }
                for u in &self.users_list {
                    ui.horizontal(|ui| {
                        let resp = ui.selectable_label(false, u.name.clone());
                        if resp.is_pointer_button_down_on()
                            && self.udrag.is_none()
                            && self.upress.is_none()
                        {
                            if let Some(start) = resp.interact_pointer_pos() {
                                self.upress = Some((u.id, u.name.clone(), start));
                            }
                        }
                        // Status chip: green only for the seeded Active
                        // row — anything else stays quiet but visible.
                        if !u.status_name.is_empty() {
                            let txt = egui::RichText::new(format!("[{}]", u.status_name)).small();
                            ui.label(if u.status_name.eq_ignore_ascii_case("active") {
                                txt.color(egui::Color32::from_rgb(0x4A, 0xDE, 0x80))
                            } else {
                                txt.weak()
                            });
                        }
                        if ui.small_button("add").clicked() {
                            seating.push(u.id);
                        }
                    });
                    // Identity line: what tells two same-named accounts
                    // apart (username · rank · unit · position).
                    let detail: Vec<&str> = [
                        u.username.as_str(),
                        u.pangkat.as_str(),
                        u.satuan.as_str(),
                        u.jabatan.as_str(),
                    ]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect();
                    if !detail.is_empty() {
                        ui.label(egui::RichText::new(detail.join(" · ")).weak().small());
                    }
                }
            },
        );
        for uid in seating {
            self.users_add(uid);
        }
    }

    /// The roster as the Players island shows it: who is seated, in what,
    /// and whether anything is still missing.
    ///
    /// READ ONLY, and that is the point. The editable roster lives in the
    /// Player Picker modal, and both are on screen at once while it is open.
    /// Two editable copies would collide on their per-row ComboBox ids —
    /// egui keys a widget by Id, so two `("roster-role", user_id)` boxes in
    /// one frame are ONE widget, which opens one popup in the wrong place
    /// and lets a click land in the other.
    fn users_roster_ui(&mut self, ui: &mut egui::Ui) {
        ui.strong("Seated");
        if self.users_game.is_none() {
            ui.weak("Pick a session to see its roster.");
            return;
        }
        if self.users_roster.is_empty() {
            ui.weak("Nobody seated yet.");
            return;
        }
        for p in self.users_roster.clone() {
            ui.horizontal(|ui| {
                ui.label(p.user_name.clone());
                ui.label(egui::RichText::new(p.role_name.clone()).weak());
                if p.judge {
                    ui.label(egui::RichText::new("judge").weak().small());
                } else if p.ready {
                    ui.label(
                        egui::RichText::new("ready")
                            .small()
                            .color(egui::Color32::from_rgb(0x4A, 0xDE, 0x80)),
                    );
                } else {
                    ui.label(egui::RichText::new("not ready").weak().small());
                }
            });
        }
        let uncommanded = self.seated_without_command();
        if !uncommanded.is_empty() {
            ui.weak(format!(
                "{} awaiting a command: {}",
                uncommanded.len(),
                uncommanded.join(", ")
            ));
        }
    }

    /// Seated exercise-side people who hold no hull.
    ///
    /// One accessor because the roster row, the island summary and the
    /// readiness gate all need this answer, and three copies of the
    /// judge-then-filtered-count is three places to forget the judge side.
    fn seated_without_command(&self) -> Vec<String> {
        self.users_roster
            .iter()
            .filter(|p| !self.users_is_judge(p))
            .filter(|p| {
                !self
                    .commanded_hulls
                    .iter()
                    .any(|h| h.commander_id == Some(p.user_id))
            })
            .map(|p| p.user_name.clone())
            .collect()
    }

    /// The editable roster, inside the Player Picker: who holds which seat,
    /// role changes (each clears readiness), and removals.
    fn roster_editor_ui(&mut self, ui: &mut egui::Ui) {
        ui.strong("Roster");
        if self.users_game.is_none() {
            ui.weak("Pick a session to see its roster.");
        } else if self.users_roster.is_empty() {
            ui.weak("Nobody seated yet.");
        } else {
            ui.weak("one role per person — a role change clears readiness");
            // Collect first, apply after the loop: every write answers
            // with the whole roster, so one pass per action is enough.
            let mut role_changes: Vec<(i64, i64)> = Vec::new();
            let mut removals: Vec<i64> = Vec::new();
            for p in &self.users_roster {
                ui.horizontal(|ui| {
                    ui.label(p.user_name.clone());
                    // Re-picking the current role is a no-op, so it
                    // never renders as an option.
                    let mut tmp = p.role_id;
                    egui::ComboBox::from_id_salt(("roster-role", p.user_id))
                        .selected_text(p.role_name.clone())
                        .show_ui(ui, |ui| {
                            for (id, name, _, _) in &self.users_roles {
                                if *id != p.role_id
                                    && ui.selectable_value(&mut tmp, *id, name).clicked()
                                {
                                    role_changes.push((p.user_id, *id));
                                }
                            }
                        });
                    // Judge side is the only positive fact the helpers
                    // mirror carries, so `role_may_need_command` on it is the
                    // whole of what can be known here — hence the branch, and
                    // hence the softer wording on the other side.
                    let judge_side = self.users_is_judge(p);
                    if judge_side {
                        ui.label(egui::RichText::new("(judge)").weak().small());
                    } else if role_may_need_command(false)
                        && !self
                            .commanded_hulls
                            .iter()
                            .any(|h| h.commander_id == Some(p.user_id))
                    {
                        // A warning, not a block: nothing forbids an
                        // exercise-side seat from holding no hull, and the
                        // server does not refuse it. Amber says so without
                        // claiming the exercise cannot start.
                        ui.label(
                            egui::RichText::new("no command yet")
                                .color(egui::Color32::from_rgb(246, 197, 107))
                                .small(),
                        );
                    }
                    if p.judge {
                        // Judge side is exempt from readiness: nothing to
                        // say, which is why this branch is empty.
                    } else if p.ready {
                        ui.label(
                            egui::RichText::new("ready")
                                .small()
                                .color(egui::Color32::from_rgb(0x4A, 0xDE, 0x80)),
                        );
                    } else {
                        ui.label(egui::RichText::new("not ready").weak().small());
                    }
                    if ui.small_button("remove").clicked() {
                        removals.push(p.user_id);
                    }
                });
            }
            for (uid, role_id) in role_changes {
                self.users_change_role(uid, role_id);
            }
            for uid in removals {
                self.users_remove(uid);
            }
        }
    }

    /// Setup flow actions (#79): game lifecycle writes. Every write
    /// answers with the collection it changed — no read-after-write.
    /// Players seat before fleet assigns: a piece IS a hull commanded
    /// by a participant of the game, never judge-side.

    /// Step 1 write: create the game session (born in `planning`),
    /// hold it, load its empty roster and pieces, move to step 2.
    fn setup_create_game(&mut self) {
        let name = self.setup_name.trim().to_string();
        if name.is_empty() {
            self.users_status = "name the game first".to_string();
            return;
        }
        if self.setup_busy("create") {
            return;
        }
        let desc = self.setup_description.clone();
        let purp = self.setup_purpose.clone();
        let targ = self.setup_target.clone();
        let area = self.setup_area.clone();
        let tag = self.setup_map_tag.clone();
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("create failed: {e}");
                return;
            }
        };
        self.setup_op = Some(spawn_rest("create", move || {
            master
                .create_game(&tok, &name, &desc, &purp, &targ, &area, &tag)
                .map_err(|e| e.to_string())
                .map(|g| {
                    let note = format!("created {} (planning)", g.name);
                    SetupDone::GameCreate(g, note)
                })
        }));
    }

    /// Apply a created game: hold it, seed planning honesty, reload,
    /// clear the form, move to step 2.
    fn apply_created_game(&mut self, g: tfg::backend::GameRow, note: String) {
        self.users_status = note;
        self.users_refresh_games();
        self.set_held_game(Some((g.id, g.name)));
        self.minos_clock = None;
        self.minos_room_key = None;
        self.clock_denied = false;  // new hold, unknown grant
        // Born in planning by contract: seed the state so the
        // projection below starts honest even before it runs.
        self.users_game_state = Some("planning".to_string());
        self.users_refresh_game();
        self.setup_name.clear();
        self.setup_description.clear();
        self.setup_purpose.clear();
        self.setup_target.clear();
        self.setup_area.clear();
        self.setup_map_tag.clear();
        self.setup_step = 1;
    }

    fn arm_fleet_pick(&mut self, id: String) {
        self.fleet_pick = Some(id);
        self.mode.tool = SetupTool::Place;
    }

    fn clear_fleet_pick(&mut self) {
        self.fleet_pick = None;
        self.mode.tool = SetupTool::Select;
    }

    /// Step 3: put a register hull into the force (and out of the
    /// register list). LOCAL FIRST — the backend write happens at a stage
    /// advance, so this is a draft edit, not a write.
    ///
    /// Mirrors the hull to the map engine as a placement pick — click the
    /// map to drop it. The commander is not asked here; that is the
    /// Player picker's question, and Minos is only told at the advance.
    fn setup_assign_unit(&mut self, unit_id: i64, unit_name: &str) {
        let Some((name, hull, Some(class_id))) = self.placement_seed_for(unit_id) else {
            self.users_status = format!("cannot stage {unit_name}: no sim stats");
            return;
        };
        let commander_id = self.force.get(unit_id).and_then(|h| h.commander_id);
        let mut entry = self
            .force
            .get(unit_id)
            .cloned()
            .unwrap_or_else(|| {
                tfg::force::DraftHull::new(unit_id, name.clone(), hull, class_id)
            });
        entry.name = name.clone();
        entry.commander_id = commander_id;
        self.force.upsert(entry);
        self.arm_fleet_pick(unit_id.to_string());
        self.refresh_unassigned_units();
        self.users_status = format!("{name} added — click the map to place it");
    }

    /// Step 3: take a hull back out of the force. LOCAL FIRST.
    fn setup_remove_unit(&mut self, unit_id: i64, unit_name: &str) {
        self.force.remove(unit_id);
        self.release_hull(&unit_id.to_string());
        self.refresh_unassigned_units();
        self.users_status = format!("{unit_name} removed from the force");
    }

    /// Resolve a register hull to (name, hull, class_id) for a local
    /// staging act. Separate from `placement_seed`, which is keyed by the
    /// string pick id; this one is keyed by the numeric unit id.
    fn placement_seed_for(&self, unit_id: i64) -> Option<(String, String, Option<String>)> {
        self.placement_seed(&unit_id.to_string())
    }

    /// Gate arithmetic for step 4 and the Persiapan bar: exercise-side
    /// seats and how many of them are ready. Judges are exempt.
    fn setup_gate_counts(&self) -> (usize, usize) {
        let mut side = 0;
        let mut ready = 0;
        for p in &self.users_roster {
            if !self.users_is_judge(p) {
                side += 1;
                if p.ready {
                    ready += 1;
                }
            }
        }
        (side, ready)
    }

    /// Non-judge roster members: the only people who may command.
    fn setup_crew(&self) -> Vec<(i64, String)> {
        self.users_roster
            .iter()
            .filter(|p| !self.users_is_judge(p))
            .map(|p| (p.user_id, p.user_name.clone()))
            .collect()
    }

    /// Planning → preparation, after the force is written through.
    /// LOCAL FIRST: every locally-staged change is synced to Minos first;
    /// only when nothing is owed (or everything landed) does the stage
    /// flip. The Game Master decides planning is done — this step has no
    /// gate of its own, but it has a SYNC of what was built.
    fn setup_advance_prep(&mut self) {
        self.begin_force_sync(Some("preparation".to_string()));
    }

    /// Preparation → execution, after the force is written through.
    /// LOCAL FIRST: same sync-then-advance, then the gate runs on the
    /// authoritative force Minos has just been handed.
    fn setup_advance_execution(&mut self) {
        self.begin_force_sync(Some("execution".to_string()));
    }

    /// Sync the staged force to Minos and then, when `then` is set,
    /// run the stage transition. Nothing to sync means the transition
    /// runs immediately. A gap (hulls nobody commands) or a request
    /// failure refuses the advance and says why, with the goal left in
    /// `then` until the next attempt.
    fn begin_force_sync(&mut self, then: Option<String>) {
        if self.setup_busy("sync") {
            return;
        }
        let Some((_, _)) = self.users_game.clone() else {
            self.phase_note = Some("hold a session first".to_string());
            return;
        };
        let live = self.live_hulls();
        match tfg::force::plan(&self.force, &live) {
            Err(gap) => {
                self.phase_note = Some(format!("sync refused: {}", gap));
            }
            Ok(writes) if writes.is_empty() => {
                if let Some(to) = then {
                    self.transition_to(&to);
                } else {
                    self.users_status = "nothing to sync".to_string();
                }
            }
            Ok(writes) => {
                if let Err(e) = self.users_client() {
                    self.phase_note = Some(format!("sync refused: {e}"));
                    return;
                }
                self.force_queue = writes;
                self.force_flush_then = then;
                self.users_status = format!("syncing {} change(s) to Minos…", self.force_queue.len());
                self.pump_force_flush();
            }
        }
    }

    /// The local view of one hull Minos holds, used for the diff.
    fn live_hulls(&self) -> Vec<tfg::force::LiveHull> {
        self.users_gunits
            .iter()
            .map(|g| tfg::force::LiveHull {
                unit_id: g.unit_id,
                name: g.unit_name.clone(),
                commander_id: g.commander_id,
                start: self
                    .users_placements
                    .iter()
                    .find(|p| p.unit_id == g.unit_id)
                    .map(|p| tfg::force::Start { lat: p.latitude, lon: p.longitude }),
            })
            .collect()
    }

    /// One force write answered: retire it from the queue and issue the next.
    /// A write that came from a standalone control rather than the flush —
    /// a commander change in the staff-gap path — leaves the queue empty,
    /// and then this is a no-op instead of a panic.
    fn force_write_done(&mut self) {
        if self.force_queue.is_empty() {
            return;
        }
        self.force_queue.remove(0);
        self.pump_force_flush();
    }

    /// Issue one staged write through the slot, and when the queue
    /// frees, the deferred transition (if any) is the last step.
    fn pump_force_flush(&mut self) {
        if self.setup_op.is_some() {
            return;
        }
        let Some(write) = self.force_queue.first().cloned() else {
            if let Some(to) = self.force_flush_then.take() {
                self.users_status = format!("synced — advancing to {to}");
                self.transition_to(&to);
            }
            return;
        };
        let Some((gid, _)) = self.users_game.clone() else {
            self.phase_note = Some("hold a session first".to_string());
            self.force_queue.clear();
            self.force_flush_then = None;
            return;
        };
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.phase_note = Some(format!("sync refused: {e}"));
                self.force_queue.clear();
                self.force_flush_then = None;
                return;
            }
        };
        let w = write.clone();
        self.setup_op = Some(spawn_rest("sync", move || {
            let res = match &w {
                tfg::force::ForceWrite::Assign { unit_id, commander_id, .. } => master
                    .assign_unit(&tok, gid, *unit_id, *commander_id)
                    .map(SetupDone::ForceUnits)
                    .map_err(|e| e.to_string()),
                tfg::force::ForceWrite::HandOver { unit_id, commander_id, .. } => master
                    .set_unit_commander(&tok, gid, *unit_id, *commander_id)
                    .map(SetupDone::ForceUnits)
                    .map_err(|e| e.to_string()),
                tfg::force::ForceWrite::Place { unit_id, lat, lon, .. } => master
                    .set_placement(&tok, gid, *unit_id, *lat, *lon)
                    .map(SetupDone::ForcePlacements)
                    .map_err(|e| e.to_string()),
                tfg::force::ForceWrite::Lift { unit_id, .. } => master
                    .clear_placement(&tok, gid, *unit_id)
                    .map(SetupDone::ForcePlacements)
                    .map_err(|e| e.to_string()),
                tfg::force::ForceWrite::Remove { unit_id, .. } => master
                    .remove_unit(&tok, gid, *unit_id)
                    .map(SetupDone::ForceUnits)
                    .map_err(|e| e.to_string()),
            };
            match res {
                Ok(done) => Ok(done),
                Err(e) => Ok(SetupDone::ForceFailed(e, w)),
            }
        }));
    }

    /// The stage transition itself, once the force is on the server.
    /// Generalized from the two old call sites; a refusal keeps the
    /// local machine where it is and names why.
    fn transition_to(&mut self, to: &str) {
        let Some((gid, _)) = self.users_game.clone() else {
            self.phase_note = Some("hold a session first".to_string());
            return;
        };
        if self.setup_busy("advance") {
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.phase_note = Some(format!("advance refused: {e}"));
                return;
            }
        };
        let to = to.to_string();
        self.setup_op = Some(spawn_rest("advance", move || {
            let res = match master.transition_game(&tok, gid, &to) {
                Ok(g) => SetupDone::Game(to.clone(), g),
                Err(tfg::backend::BackendError::Forbidden { .. }) => SetupDone::GameFailed(
                    to.clone(),
                    "forbidden".to_string(),
                    true,
                ),
                Err(e) => SetupDone::GameFailed(to.clone(), e.to_string(), false),
            };
            Ok(res)
        }));
    }

    /// Eksekusi bar write (H1): execution → closure moves the backend
    /// first, then the local machine follows. A refusal keeps the
    /// session Live and says why — ending locally while Minos still
    /// runs is exactly the disagreement this ticket removes. When the
    /// backend is already closed (another client closed it), the
    /// resync projects Eval and the local end follows honestly.
    fn close_game(&mut self) {
        let Some((gid, _)) = self.users_game.clone() else {
            self.phase_note = Some("hold a session first".to_string());
            return;
        };
        if self.setup_busy("closure") {
            self.phase_note = Some("closure already running…".to_string());
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.phase_note = Some(format!("closure refused: {e}"));
                return;
            }
        };
        self.setup_op = Some(spawn_rest("closure", move || {
            Ok(match master.transition_game(&tok, gid, "closure") {
                Ok(g) => SetupDone::Game("closure".to_string(), g),
                Err(tfg::backend::BackendError::Forbidden { .. }) => SetupDone::GameFailed(
                    "closure".to_string(),
                    "forbidden".to_string(),
                    true,
                ),
                Err(e) => SetupDone::GameFailed("closure".to_string(), e.to_string(), false),
            })
        }));
    }

    /// Apply a finished transition: the bundle refreshes project the
    /// new state (guarded local start/end ride along as before).
    /// Success clears any pending refusal verdict.
    fn apply_transition(&mut self, to: &str, g: tfg::backend::GameRow) {
        self.pending_transition = None;        match to {
            "preparation" => {
                self.sim_ready = true;
                self.phase_note = None;
                self.users_status = format!("{} → {}", g.name, g.state);
                self.users_refresh_games();
                self.users_refresh_game();
            }
            "execution" => {
                self.phase_note = None;
                self.users_status = format!("{} → {}", g.name, g.state);
                self.users_refresh_games();
                self.users_refresh_game();
                if !self.session_live() {
                    self.start_session();
                }
            }
            "closure" => {
                self.phase_note = None;
                self.users_status = format!("{} → {}", g.name, g.state);
                self.users_refresh_games();
                self.users_refresh_game();
                if !self.session_closed() {
                    self.end_session();
                }
            }
            _ => {
                self.users_status = format!("{} → {}", g.name, g.state);
            }
        }
    }

    /// Apply a refused transition: re-read first, verdict on arrival.
    /// The bundle apply consumes the pending verdict against the
    /// authoritative state.
    fn apply_transition_failed(&mut self, to: String, e: String, forbidden: bool) {
        self.pending_transition = Some((to, e, forbidden));
        self.users_refresh_games();
        self.users_refresh_game();
    }

    /// Verdict for a refused transition, read off authoritative state:
    /// already-moved folds into the projection's outcome, otherwise
    /// the refusal stands on the bar.
    fn apply_transition_verdict(&mut self) {
        let Some((to, e, forbidden)) = self.pending_transition.take() else {
            return;
        };
        let state = self.users_game_state.clone();
        match (to.as_str(), state.as_deref()) {
            ("closure", Some("closure")) => {
                self.phase_note = Some("closed by another client".to_string());
                if !self.session_closed() {
                    self.end_session();
                }
            }
            ("execution", Some("execution")) => {
                self.phase_note = Some("the exercise is already running".to_string());
                if !self.session_live() {
                    self.start_session();
                }
            }
            ("preparation", Some("preparation")) => {
                self.sim_ready = true;
                self.phase_note = None;
            }
            _ if forbidden => {
                // #102: advancing is the game-domain grant of this
                // game's Game Master — application admin rights never
                // apply, and the backend's bare 403 explains nothing.
                // Seating is actionable today (step 2 seats any role),
                // so name it, split by whether the caller is seated.
                let seated = self.own_roster_row().is_some();
                self.phase_note = Some(if seated {
                    "Only this session's Game Master advances it — being app admin is not enough. Seat yourself as Game Master in step 2, then retry.".to_string()
                } else {
                    "Only this session's Game Master advances it — you hold no seat in it. Seat yourself (Game Master role) in step 2, then retry.".to_string()
                });
            }
            _ => {
                let verb = match to.as_str() {
                    "closure" => "closure",
                    "execution" => "execution",
                    _ => "advance",
                };
                self.phase_note = Some(format!("{verb} refused: {e}"));
            }
        }
    }

    /// C2: apply a placement setup view to local state. Counts ride
    /// along — the gate arithmetic is the server's, never recomputed.
    fn apply_placements(&mut self, view: tfg::backend::PlacementList) {
        self.users_placements = view.placements;
        self.placement_unplaced = view.unplaced;
        self.placement_ready = view.ready;
    }

    /// C2: the caller's own roster row, matched by the probed user id —
    /// never by name (the login identifier is not the display name).
    fn own_roster_row(&self) -> Option<tfg::backend::Participant> {
        let uid = self.auth_user_id?;
        self.users_roster
            .iter()
            .find(|p| p.user_id == uid)
            .cloned()
    }

    /// C2: declare or withdraw the CALLER's own readiness. Nobody can
    /// declare for somebody else — there is no user id in the request.
    /// The roster reloads afterwards so the badge shown is the row the
    /// database decided, not an optimistic flip.
    fn set_own_readiness(&mut self, ready: bool) {
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        if self.setup_busy("readiness") {
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("readiness failed: {e}");
                return;
            }
        };
        self.setup_op = Some(spawn_rest("readiness", move || {
            master
                .set_readiness(&tok, gid, ready)
                .map_err(|e| e.to_string())
                .map(|j| {
                    let note = if ready {
                        format!("{} declared ready ({} · {})", j.user_name, j.role_name, j.game_name)
                    } else {
                        format!("{} withdrew readiness", j.user_name)
                    };
                    SetupDone::Join(j, note)
                })
        }));
    }

    /// Apply a finished join/readiness answer: confirm identity, hold
    /// or badge from it, then reload through the bundle.
    fn apply_join(&mut self, j: tfg::backend::JoinResult, note: String) {
        self.auth_user_id = Some(j.user_id);
        self.watch_personal_channel();
        // A join answer carries the game; a readiness answer echoes the
        // held one. Either way the hold follows the answer — and a
        // changed hold drops the old game's clock read.
        let held = self.users_game.as_ref().map(|(id, _)| *id);
        if held != Some(j.game_id) {
            self.minos_clock = None;
            self.minos_room_key = None;
            self.clock_denied = false;  // new hold, unknown grant
            // A new hold means unknown gaps — the bundle re-marks them.
            self.roster_gap = false;
            self.units_gap = false;
            self.placements_gap = false;
            // …and a new tree: queued behind the bundle, never refused
            // past a busy slot. Readiness echoes skip this entirely.
            self.minos_tree.clear();
            self.tree_gap = false;
            self.queue_refresh(PendingRefresh::Tree);
        }
        self.set_held_game(Some((j.game_id, j.game_name.clone())));
        self.users_game_state = Some(j.game_state.clone());
        // Atomic seat landing: the answer's participant row upserts the
        // local roster (a later staff bundle overwrites with the full
        // list; a gapped one leaves this row standing), and the
        // commanded hulls become the order authority — no successful
        // join ends in "you hold no seat".
        let row = tfg::backend::Participant {
            user_id: j.user_id,
            user_name: j.user_name.clone(),
            role_id: j.role_id,
            role_name: j.role_name.clone(),
            judge: j.judge,
            ready: j.ready,
            // A successful join IS a room entry, so this seat is in the
            // room by construction. The server stamps the instant; the
            // headcount only needs to know it is not absent.
            joined_at: Some("now".to_string()),
        };
        match self.users_roster.iter_mut().find(|p| p.user_id == j.user_id) {
            Some(slot) => *slot = row,
            None => self.users_roster.push(row),
        }
        self.commanded_hulls = j.commanded_units.clone();
        self.users_status = note;
        self.users_refresh_games();
        self.users_refresh_game();
    }

    /// C2: enter a game room with its key. The key is the only input;
    /// the answer carries the game, so the hold is set from it and the
    /// H1 resync inside the refresh projects the stage. Unknown key is
    /// 404, a valid key without a seat is 403 (ask the Game Master).
    fn join_with_key(&mut self) {
        let key = self.join_key.trim().to_string();
        if key.is_empty() {
            self.users_status = "enter the room key first".to_string();
            return;
        }
        if self.setup_busy("join") {
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("join failed: {e}");
                return;
            }
        };
        self.join_key.clear();
        self.setup_op = Some(spawn_rest("join", move || {
            // Typed in the worker: 403 is a valid key without a seat
            // (ask the Game Master), 404 an unknown key (check the
            // code) — different next actions, so they never share a
            // line. Everything else is a loud transport failure.
            let res = match master.join_game(&tok, &key) {
                Ok(j) => {
                    let note = format!(
                        "joined {} as {} ({})",
                        j.game_name, j.user_name, j.role_name
                    );
                    Ok(SetupDone::Join(j, note))
                }
                Err(tfg::backend::BackendError::Forbidden { .. }) => {
                    Ok(SetupDone::JoinFailed(
                        "valid room key, but you hold no seat in that session — ask the Game Master to seat you".to_string(),
                    ))
                }
                Err(tfg::backend::BackendError::Api { status: 404, .. }) => {
                    Ok(SetupDone::JoinFailed(
                        "unknown room key — check the code with the organizer".to_string(),
                    ))
                }
                Err(e) => Err(e.to_string()),
            };
            res
        }));
    }

    /// C2: lift a hull off the map (placement DELETE, idempotent). The
    /// piece stays in the exercise and in the task organisation.
    ///
    /// LOCAL FIRST: the draft loses its start and the local sim ship is
    /// released. Minos catches up at a stage advance.
    /// Local lift for the normal (staff) path: the draft drops its start.
    fn lift_placement(&mut self, unit_id: i64, unit_name: &str) {
        self.force.set_start(unit_id, None);
        self.release_hull(&unit_id.to_string());
        self.users_status = format!("{unit_name} lifted off the map");
    }

    /// Direct server lift for the staff-gap (participant) path, where the
    /// hull is a commanded hull, not a locally staged one. The answer's
    /// setup view replaces the mirror.
    fn lift_placement_remote(&mut self, unit_id: i64, unit_name: &str) {
        let Some((gid, _)) = self.users_game.clone() else {
            return;
        };
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("lift refused: {e}");
                return;
            }
        };
        self.setup_op = Some(spawn_rest("lift", move || {
            master
                .clear_placement(&tok, gid, unit_id)
                .map(SetupDone::ForcePlacements)
                .map_err(|e| e.to_string())
        }));
        self.users_status = format!("lifting {unit_name}…");
    }

    /// Move a placed hull's starting position. Placements are editable
    /// while the exercise has not begun (planning|preparation); after
    /// that the first leg of the fix chain is frozen and moving a hull
    /// means giving it an order, which is a new leg rather than an edit.
    ///
    /// LOCAL FIRST: the draft's start moves, and the local sim ship is
    /// moved too — Minos catches up at a stage advance.
    fn move_placement(&mut self, unit_id: i64, unit_name: &str, la: f64, lo: f64) {
        if self.mode.phase != Phase::Setup {
            self.note_placement("placements are frozen once the exercise begins");
            return;
        }
        if !self.placement_valid(la, lo) {
            self.note_placement("placement needs water");
            return;
        }
        if !self.force.set_start(unit_id, Some(tfg::force::Start { lat: la, lon: lo })) {
            return;
        }
        let id = unit_id.to_string();
        let pos = GeoPosition { latitude: la, longitude: lo };
        if let Some(tx) = &self.sim_cmd_tx {
            let _ = tx.send(SimCommand::MovePlacement {
                ship_id: id.clone(),
                pos,
            });
        }
        // The map's half. The setup sim is frozen, so no tick will carry
        // the new position to the registry: the display is moved here or
        // the hull sits where it used to be while the status line says it
        // moved.
        self.registry.set_position(&id, pos);
        self.fix_animation_started.remove(&id);
        self.users_status = format!("{unit_name} moved to ({la:.4}, {lo:.4})");
    }

    /// C3: the held game's piece for this ship, if the caller commands
    /// it in an executing game. Authority is the join answer's
    /// commanded hulls first (the only list a participant always sees),
    /// the staff unit list second. Anything else stays on the local
    /// (sandbox) order path.
    fn minos_order_target(&self, ship_id: &str) -> Option<i64> {
        if self.users_game_state.as_deref() != Some("execution") {
            return None;
        }
        let uid = ship_id.parse::<i64>().ok()?;
        let me = self.auth_user_id?;
        if self.commanded_hulls.iter().any(|g| g.unit_id == uid) {
            return Some(uid);
        }
        self.users_gunits
            .iter()
            .find(|g| g.unit_id == uid && g.commander_id == Some(me))
            .map(|g| g.unit_id)
    }

    /// C3: order one commanded piece through MinOS using the direct
    /// heading/speed HelmOrder contract. The server owns the position,
    /// scenario time, clamping, and accepted Fix; no local SetOrder
    /// follows. Refusals (paused clock, wrong commander) stay loud and
    /// local. The draft is marked pending before the worker starts.
    fn order_via_minos(&mut self, ship_id: &str, heading_deg: f32, speed_kn: f32) {
        let Some(uid) = self.minos_order_target(ship_id) else {
            let msg = format!("unit {ship_id} is not available for commands");
            self.feed(msg.clone());
            self.users_status = msg;
            return;
        };
        self.spawn_order_batch(vec![(
            uid,
            ship_id.to_string(),
            f64::from(heading_deg),
            speed_kn,
        )]);
    }

    /// #100: fire one batch op carrying every (unit, ship, heading,
    /// speed) leg. The worker fills each outcome; the apply reports
    /// them all and pulls the plot once.
    fn spawn_order_batch(&mut self, legs: Vec<(i64, String, f64, f32)>) {
        if legs.is_empty() {
            return;
        }
        if self.setup_busy("order") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            return;
        };
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                let msg = format!("order refused: {e}");
                self.feed(msg.clone());
                self.users_status = msg;
                return;
            }
        };
        for (_, ship, heading, speed) in &legs {
            self.mark_helm_pending(ship, *heading as f32, *speed);
        }
        self.setup_op = Some(spawn_rest("order", move || {
            let mut outs = Vec::with_capacity(legs.len());
            for (uid, ship, heading, speed) in legs {
                let result = match master.order_unit(&tok, gid, uid, heading, f64::from(speed)) {
                    Ok(fix) => Ok(fix),
                    Err(error) => {
                        let unknown = match &error {
                            tfg::backend::BackendError::Transport(_)
                            | tfg::backend::BackendError::Decode(_)
                            | tfg::backend::BackendError::Other(_) => true,
                            tfg::backend::BackendError::Api { status, .. } => *status >= 500,
                            _ => false,
                        };
                        let message = error.to_string();
                        if unknown {
                            Err(OrderFailure::Unknown(message))
                        } else {
                            let category = match &error {
                                tfg::backend::BackendError::Unauthorized { .. } => "authentication refused",
                                tfg::backend::BackendError::Forbidden { .. } => "not commander",
                                tfg::backend::BackendError::NoSpec { .. } => {
                                    "no applicable speed limit"
                                }
                                tfg::backend::BackendError::Api { status, .. }
                                    if *status == 400 || *status == 422 =>
                                {
                                    "invalid request"
                                }
                                tfg::backend::BackendError::Api { status, .. } if *status == 404 => {
                                    "stale game context"
                                }
                                tfg::backend::BackendError::Api { status, .. } if *status == 409 => {
                                    "wrong phase or actions closed"
                                }
                                _ => "other refusal",
                            };
                            Err(OrderFailure::Refused {
                                category: category.to_string(),
                                detail: message,
                            })
                        }
                    }
                };
                outs.push(OrderOut { ship, heading, speed, result });
            }
            Ok(SetupDone::FixBatch(outs))
        }));
    }

    /// Apply finished Minos orders: one feed line per leg, status names
    /// the count, then the plot refreshes once for all of them.
    fn apply_fix_batch(&mut self, outs: Vec<OrderOut>) {
        let mut ok = 0;
        for out in outs {
            if !self.submission_matches_values(&out.ship, out.heading, out.speed) {
                self.feed(format!(
                    "late Minos result ignored for {}: newer helm intent is active",
                    out.ship
                ));
                continue;
            }
            match out.result {
                Ok(fix) => {
                    ok += 1;
                    self.helm_submissions.remove(&out.ship);
                    let result = if fix.clamped {
                        HelmOrderUiResult::Clamped {
                            requested: fix.requested_speed.unwrap_or(out.speed as f64),
                            accepted: fix.speed,
                        }
                    } else {
                        HelmOrderUiResult::Accepted
                    };
                    if self.helm_drafts.contains_key(&out.ship) {
                        self.helm_drafts.insert(
                            out.ship.clone(),
                            HelmDraft {
                                heading_deg: fix.heading as f32,
                                speed_kn: fix.speed as f32,
                            },
                        );
                    }
                    self.helm_preview_pending.insert(out.ship.clone());
                    self.order_result.insert(out.ship.clone(), result);
                    let clamp = match fix.requested_speed {
                        Some(asked) if fix.clamped => {
                            format!(" · clamped {asked:.0}→{:.0} kn", fix.speed)
                        }
                        _ => String::new(),
                    };
                    self.feed(format!(
                        "Minos order {}: {:.0}° @ {:.0} kn{clamp} · fix @ ({:.4}, {:.4}) {}",
                        out.ship,
                        out.heading,
                        out.speed,
                        fix.latitude,
                        fix.longitude,
                        fix.assumed_time
                    ));
                }
                Err(OrderFailure::Refused { category, detail }) => {
                    self.helm_submissions.remove(&out.ship);
                    self.order_result.insert(
                        out.ship.clone(),
                        HelmOrderUiResult::Refused(category.clone()),
                    );
                    self.feed(format!(
                        "order refused for {}: {category} ({detail})",
                        out.ship
                    ));
                }
                Err(OrderFailure::Unknown(e)) => {
                    self.order_result.insert(
                        out.ship.clone(),
                        HelmOrderUiResult::Unknown("outcome unknown — see Log".into()),
                    );
                    self.feed(format!(
                        "order outcome unknown for {}: {e} — verify before retrying",
                        out.ship
                    ));
                }
            }
        }
        self.users_status = format!("Commands: {ok} accepted");
        self.pull_minos_positions();
    }

    /// C3: read the authoritative plot and ingest it as game fixes —
    /// the marker source for Minos-driven hulls. Local ghosts for those
    /// hulls are released on execution entry, so nothing flaps. Game
    /// fixes are scenario-stamped: they glide without the jitter guard
    /// and never wall-age (see `FixSource::Game`).
    /// C3 + M7: read the authoritative plot off-thread and ingest it
    /// as game fixes on apply — the marker source for Minos-driven
    /// hulls. Re-pulls while busy are ignored: the in-flight picture
    /// is the freshest ask.
    fn pull_minos_positions(&mut self) {
        if self.app_mode != AppMode::Simulation
            || self.users_game_state.as_deref() != Some("execution")
        {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            return;
        };
        if self.plot_op.as_ref().is_some_and(|slot| slot.game_id == gid) {
            return;
        }
        if self.plot_op.is_some() {
            // A request for a replaced game is no longer useful. Drop its
            // receiver; the detached worker's late answer cannot be applied.
            self.plot_op = None;
        }
        // #98: every attempt (manual or cadence) restarts the wait —
        // stamped before the auth check so a missing token backs off
        // instead of rebuilding the client every frame.
        self.last_plot_try = Some(Instant::now());
        let Ok((master, tok)) = self.users_client() else {
            return;
        };
        let op = spawn_rest("plot", move || {
            master.positions(&tok, gid, None, None)
                .map(|plot| PlotDone { game_id: gid, result: Ok(plot) })
                .map_err(|e| e.to_string())
        });
        self.plot_op = Some(PlotSlot { game_id: gid, op });
    }

    /// Apply a finished REST plot. The same ingest path is used by the
    /// WebSocket position stream; only cadence/status bookkeeping differs.
    fn apply_plot(&mut self, done: PlotDone) {
        let current_id = self.users_game.as_ref().map(|(id, _)| *id);
        if self.app_mode != AppMode::Simulation
            || self.mode.phase == Phase::Closed
            || current_id != Some(done.game_id)
        {
            return;
        }
        match done.result {
            Ok(plot) => {
                self.last_plot_ok = Some(Instant::now());
                self.plot_fails = 0;
                let assumed_time = plot.assumed_time.clone();
                let update = GamePositionUpdate::from_plot(done.game_id, plot);
                let (n, _) = self.ingest_game_positions(update);
                self.users_status = format!("Exercise plot: {n} hull(s) @ {assumed_time}");
            }
            Err(e) => {
                self.plot_fails = self.plot_fails.saturating_add(1);
                self.users_status = format!("plot failed: {e}");
            }
        }
    }

    /// Turn one authoritative MinOS position publication into accepted
    /// Game fixes. The game-scoped poll keeps this independent stream
    /// from affecting Wire/Sim miss counters.
    fn ingest_game_positions(&mut self, update: GamePositionUpdate) -> (usize, bool) {
        let n = update.positions.len();
        let fixes: Vec<Fix> = update
            .positions
            .iter()
            .map(|p| {
                let (name, hull) = self
                    .users_gunits
                    .iter()
                    .find(|g| g.unit_id == p.unit_id)
                    .map(|g| (Some(g.unit_name.clone()), Some(g.hull_number.clone())))
                    .unwrap_or((None, None));
                Fix {
                    ship_id: p.unit_id.to_string(),
                    position: GeoPosition {
                        latitude: p.latitude,
                        longitude: p.longitude,
                    },
                    ts: update.assumed_time.clone(),
                    received_at: None,
                    heading_deg: p.heading_deg.map(|heading| heading as f32),
                    speed_kn: p.speed_kn.map(|speed| speed as f32),
                    accuracy_m: None,
                    name,
                    hull_number: hull,
                    backfilled: false,
                    source: FixSource::Game,
                    age_secs: None,
                    seq: 0,
                }
            })
            .collect();
        let acked = self.registry.poll_game(fixes);
        let accepted = !acked.is_empty();
        if accepted {
            let now = Instant::now();
            for (ship_id, _) in &acked {
                let arrival_gap = self
                    .last_seen
                    .get(ship_id)
                    .is_some_and(|last| now.saturating_duration_since(*last) > Duration::from_secs(5));
                if arrival_gap {
                    self.game_animation_snap.insert(ship_id.clone());
                } else {
                    self.game_animation_snap.remove(ship_id);
                }
                self.last_seen.insert(ship_id.clone(), now);
                *self.fix_count.entry(ship_id.clone()).or_insert(0) += 1;
                self.fix_animation_started.insert(ship_id.clone(), now);
            }
            self.last_poll = now;
        }
        (n, accepted)
    }


    /// C3: release local sim control of held-game pieces. Runs on
    /// execution entry: from here Minos drives these hulls, and a local
    /// leg would be a ghost track diverging from the fix chain. The
    /// plot pull right after gives them their authoritative markers.
    fn release_game_pieces(&mut self) {
        let ids: Vec<String> = self
            .controlled
            .iter()
            .filter(|id| {
                id.parse::<i64>()
                    .is_ok_and(|uid| self.users_gunits.iter().any(|g| g.unit_id == uid))
            })
            .cloned()
            .collect();
        for id in &ids {
            self.release_hull(id);
        }
        if !ids.is_empty() {
            eprintln!("released {} game piece(s) to Minos", ids.len());
        }
    }

    /// Direct helm control for a caller-commanded piece in an executing
    /// MinOS game. The server receives only heading and speed; the
    /// authoritative GameFix and next plot update the marker.
    fn minos_order_ui(&mut self, ui: &mut egui::Ui, id: &str) {
        ui.label(format!("Unit {id} · direct helm control"));
        let reported = self
            .registry
            .ship(id)
            .map(|ship| (ship.latest.heading_deg, ship.latest.speed_kn));
        let (reported_heading, reported_speed) = reported
            .map(|(heading, speed)| (heading.unwrap_or(0.0), speed))
            .unwrap_or((0.0, None));
        let draft = self.helm_drafts.get(id).copied();
        let mut heading = draft
            .map(|draft| draft.heading_deg)
            .unwrap_or(reported_heading)
            .rem_euclid(360.0);
        let mut speed = draft
            .map(|draft| draft.speed_kn)
            .unwrap_or(reported_speed.unwrap_or(0.0))
            .max(0.0);
        ui.horizontal(|ui| {
            Self::helm_heading_input(ui, &mut heading, true);
            Self::helm_speed_control(ui, &mut speed, None, true);
        });
        speed = speed.max(0.0);
        if draft.is_some_and(|draft| {
            (draft.heading_deg - heading).abs() > 0.01
                || (draft.speed_kn - speed).abs() > 0.01
        }) {
            if matches!(self.order_result.get(id), Some(HelmOrderUiResult::Pending)) {
                self.order_result
                    .insert(id.to_string(), HelmOrderUiResult::Superseded);
                self.helm_submissions.remove(id);
            } else {
                self.order_result
                    .insert(id.to_string(), HelmOrderUiResult::Draft);
            }
        }
        self.helm_drafts.insert(
            id.to_string(),
            HelmDraft {
                heading_deg: heading,
                speed_kn: speed,
            },
        );
        let pending = matches!(
            self.order_result.get(id),
            Some(HelmOrderUiResult::Pending)
        );
        if let Some(result) = self.order_result.get(id) {
            match result {
                HelmOrderUiResult::Draft => {
                    ui.weak("draft · not submitted");
                }
                HelmOrderUiResult::Pending => {
                    ui.weak("Sending… · waiting for the exercise");
                }
                HelmOrderUiResult::Accepted => {
                    ui.label(egui::RichText::new("accepted by the exercise").color(egui::Color32::GREEN));
                }
                HelmOrderUiResult::Clamped { requested, accepted } => {
                    warn_line(
                        ui,
                        format!("requested {requested:.0} kn · accepted {accepted:.0} kn · speed adjusted by the exercise"),
                    );
                }
                HelmOrderUiResult::Unknown(reason) => {
                    warn_line(ui, format!("outcome unknown: {reason}"));
                }
                HelmOrderUiResult::Refused(reason) => {
                    warn_line(ui, format!("refused: {reason}"));
                }
                HelmOrderUiResult::Superseded => {
                    ui.weak("superseded by a newer intent");
                }
            }
        }
        if matches!(self.order_result.get(id), Some(HelmOrderUiResult::Draft))
            && ui.small_button("Cancel draft").clicked()
        {
            self.helm_drafts.remove(id);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!pending, egui::Button::new("Set helm"))
                .clicked()
            {
                self.order_via_minos(id, heading, speed);
            }
            if ui
                .add_enabled(!pending, egui::Button::new("Hold position"))
                .clicked()
            {
                self.order_via_minos(id, reported_heading, 0.0);
                self.helm_drafts.insert(
                    id.to_string(),
                    HelmDraft {
                        heading_deg: reported_heading,
                        speed_kn: 0.0,
                    },
                );
            }
        });
        ui.weak("No waypoint: heading and speed remain in force until replaced.");
    }

    fn helm_heading_input(ui: &mut egui::Ui, heading_deg: &mut f32, enabled: bool) {
        ui.label("heading");
        let input = egui::DragValue::new(heading_deg)
            .speed(1.0)
            .range(0.0..=359.0)
            .suffix("°");
        if enabled {
            ui.add(input);
        } else {
            ui.add_enabled(false, input);
        }
        if !enabled {
            ui.weak("View only");
        }
    }

    fn helm_speed_control(
        ui: &mut egui::Ui,
        speed_kn: &mut f32,
        max_kn: Option<f32>,
        enabled: bool,
    ) {
        ui.label("speed");
        let upper = max_kn
            .filter(|max| *max > 0.0)
            .unwrap_or_else(|| speed_kn.max(100.0))
            .max(*speed_kn)
            .max(1.0);
        let slider = egui::Slider::new(speed_kn, 0.0..=upper)
            .suffix(" kn")
            .show_value(true);
        let response = if enabled {
            ui.add(slider)
        } else {
            ui.add_enabled(false, slider)
        };
        if enabled {
            ui.add(egui::DragValue::new(speed_kn).suffix(" kn"));
        } else {
            ui.add_enabled(false, egui::DragValue::new(speed_kn).suffix(" kn"));
        }
        if max_kn.is_none() {
            ui.weak("The exercise decides the accepted limit.");
        }
        if response.changed() {
            *speed_kn = speed_kn.clamp(0.0, upper);
        }
    }

    fn order_event_matches(&self, ship_id: &str, fix: &GameFix) -> bool {
        let Some(submission) = self.helm_submissions.get(ship_id) else {
            return false;
        };
        let Some(requester_id) = submission.requester_id else {
            return false;
        };
        let heading_matches = (fix.heading as f32 - submission.heading_deg).abs() < 0.01;
        let speed_matches = if fix.clamped {
            fix.requested_speed.is_some_and(|requested| {
                (requested as f32 - submission.speed_kn).abs() < 0.01
                    && fix.speed <= submission.speed_kn as f64
            })
        } else {
            (fix.speed as f32 - submission.speed_kn).abs() < 0.01
        };
        fix.created_by == Some(requester_id) && heading_matches && speed_matches
    }

    fn submission_matches_values(
        &self,
        ship_id: &str,
        heading_deg: f64,
        speed_kn: f32,
    ) -> bool {
        self.helm_submissions.get(ship_id).is_some_and(|submission| {
            (submission.heading_deg as f64 - heading_deg).abs() < 0.01
                && (submission.speed_kn - speed_kn).abs() < 0.01
        })
    }

    fn applied_submission_matches(
        &self,
        ship_id: &str,
        heading_deg: f32,
        speed_kn: f32,
    ) -> bool {
        self.helm_submissions.get(ship_id).is_some_and(|submission| {
            (submission.speed_kn - speed_kn).abs() < 0.01
                && (speed_kn.abs() < 0.01
                    || (submission.heading_deg - heading_deg).abs() < 0.01)
        })
    }

    fn mark_helm_pending(&mut self, id: &str, heading_deg: f32, speed_kn: f32) {
        if matches!(
            self.order_result.get(id),
            Some(HelmOrderUiResult::Pending)
        ) {
            self.order_result
                .insert(id.to_string(), HelmOrderUiResult::Superseded);
        }
        self.helm_submissions.insert(
            id.to_string(),
            HelmSubmission {
                heading_deg,
                speed_kn,
                requester_id: self.auth_user_id,
            },
        );
        self.order_result
            .insert(id.to_string(), HelmOrderUiResult::Pending);
    }

    fn send_local_helm(&mut self, id: &str, heading_deg: f32, speed_kn: f32) {
        let unit = id.to_string();
        let Some(authority) = self.command_authority(std::slice::from_ref(&unit)) else {
            self.order_result.insert(
                id.to_string(),
                HelmOrderUiResult::Refused("no local command authority".into()),
            );
            return;
        };
        let grant = Grant {
            units: vec![unit],
            expires_game_secs: u64::MAX,
            verbs: vec![Verb::SetHelm],
        };
        self.mark_helm_pending(id, heading_deg, speed_kn);
        self.helm_warnings.remove(id);
        let Some(tx) = &self.sim_cmd_tx else {
            self.helm_submissions.remove(id);
            self.order_result.insert(
                id.to_string(),
                HelmOrderUiResult::Unknown("local command channel unavailable".into()),
            );
            return;
        };
        if tx
            .send(SimCommand::SetHelm {
                ship_id: id.to_string(),
                heading_deg,
                speed_kn,
                authority,
                grant,
            })
            .is_err()
        {
            self.helm_submissions.remove(id);
            self.order_result.insert(
                id.to_string(),
                HelmOrderUiResult::Unknown("local command channel closed".into()),
            );
        }
    }

    /// Direct heading/speed control for the local sandbox. It mirrors
    /// the MinOS surface but labels local clamping as sandbox authority.
    fn local_helm_order_ui(&mut self, ui: &mut egui::Ui, id: &str) {
        let Some(view) = self.order_views.get(id) else {
            ui.weak("waiting for the local sim…");
            return;
        };
        if view.max_speed_kn <= 0.0 {
            self.order_result.insert(
                id.to_string(),
                HelmOrderUiResult::Refused("no local speed limit is published".into()),
            );
            warn_line(
                ui,
                "local refusal: no speed limit is published for this class".to_string(),
            );
            return;
        }
        let reported = self
            .registry
            .ship(id)
            .map(|ship| (ship.latest.heading_deg, ship.latest.speed_kn));
        let (reported_heading, reported_speed) = reported
            .map(|(heading, speed)| (heading.unwrap_or(0.0), speed))
            .unwrap_or((0.0, None));
        let draft = self.helm_drafts.get(id).copied();
        let mut heading = draft
            .map(|draft| draft.heading_deg)
            .unwrap_or(reported_heading)
            .rem_euclid(360.0);
        let mut speed = draft
            .map(|draft| draft.speed_kn)
            .unwrap_or(reported_speed.unwrap_or(0.0))
            .max(0.0);
        ui.label("local sandbox · persistent HelmOrder");
        ui.horizontal(|ui| {
            Self::helm_heading_input(ui, &mut heading, true);
            Self::helm_speed_control(ui, &mut speed, Some(view.max_speed_kn), true);
        });
        if draft.is_some_and(|draft| {
            (draft.heading_deg - heading).abs() > 0.01
                || (draft.speed_kn - speed).abs() > 0.01
        }) {
            if matches!(self.order_result.get(id), Some(HelmOrderUiResult::Pending)) {
                self.order_result
                    .insert(id.to_string(), HelmOrderUiResult::Superseded);
                self.helm_submissions.remove(id);
            } else {
                self.order_result
                    .insert(id.to_string(), HelmOrderUiResult::Draft);
            }
        }
        self.helm_drafts.insert(
            id.to_string(),
            HelmDraft {
                heading_deg: heading,
                speed_kn: speed,
            },
        );
        let pending = matches!(
            self.order_result.get(id),
            Some(HelmOrderUiResult::Pending)
        );
        if let Some(result) = self.order_result.get(id) {
            match result {
                HelmOrderUiResult::Draft => {
                    ui.weak("local draft · not submitted");
                }
                HelmOrderUiResult::Pending => {
                    ui.weak("pending local sandbox command…");
                }
                HelmOrderUiResult::Accepted => {
                    ui.label(egui::RichText::new("accepted by local sandbox").color(egui::Color32::GREEN));
                }
                HelmOrderUiResult::Clamped { requested, accepted } => {
                    warn_line(ui, format!("local clamp · requested {requested:.0} kn · accepted {accepted:.0} kn"));
                }
                HelmOrderUiResult::Unknown(reason) => warn_line(ui, format!("local outcome unknown: {reason}")),
                HelmOrderUiResult::Refused(reason) => warn_line(ui, format!("local refusal: {reason}")),
                HelmOrderUiResult::Superseded => {
                    ui.weak("local draft superseded");
                }
            }
        }
        if matches!(self.order_result.get(id), Some(HelmOrderUiResult::Draft))
            && ui.small_button("Cancel draft").clicked()
        {
            self.helm_drafts.remove(id);
        }
        if let Some(warning) = self.helm_warnings.get(id).cloned() {
            warn_line(ui, warning);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!pending, egui::Button::new("Set helm"))
                .clicked()
            {
                self.send_local_helm(id, heading, speed);
            }
            if ui
                .add_enabled(!pending, egui::Button::new("Hold position"))
                .clicked()
            {
                self.send_local_helm(id, reported_heading, 0.0);
                self.helm_drafts.insert(
                    id.to_string(),
                    HelmDraft {
                        heading_deg: reported_heading,
                        speed_kn: 0.0,
                    },
                );
            }
            if ui.button("Release local control").clicked() {
                if let Some(tx) = &self.sim_cmd_tx {
                    let _ = tx.send(SimCommand::Release { ship_id: id.to_string() });
                }
                self.controlled.remove(id);
                self.order_views.remove(id);
                self.order_result.remove(id);
                self.helm_submissions.remove(id);
                self.helm_warnings.remove(id);
                self.pending_waypoint = None;
                self.placing = false;
            }
        });
        ui.weak("No waypoint or ETA: the local sim emits Sim Fixes from this setpoint.");
    }

    /// H2: apply a GameClock answer to local state. The chosen rate is
    /// kept for the engine seeding; the local display hold follows the
    /// scenario hold so the two never disagree on screen. Refusals
    /// never reach here — they stay on the caller.
    fn apply_clock(&mut self, clock: tfg::backend::GameClock, verb: &str) {
        self.minos_time_factor = Some(clock.time_factor);
        self.factor_draft = clock.time_factor;
        // An answer proves the grant — the denial was for an older
        // hold or a since-changed matrix.
        self.clock_denied = false;
        let line = format!(
            "Scenario clock {verb}: assumed {} · {} · {} · {factor}x",
            clock.assumed_now.as_deref().unwrap_or("—"),
            if clock.running { "running" } else { "held" },
            if clock.accepting_actions {
                "accepting orders"
            } else {
                "orders closed"
            },
            factor = clock.time_factor,
        );
        self.feed(line.clone());
        self.users_status = line;
        // Connected execution: the local display clock follows the
        // scenario hold. (A blackout runs with orders closed — the
        // local sim has no blackout state, so running wins.)
        if self.users_game_state.as_deref() == Some("execution") {
            if let Some(tx) = &self.sim_cmd_tx {
                // The answer is the only clock read: push its pace
                // down, or the sim keeps a stale ratio forever.
                let _ = tx.send(SimCommand::SetClockRatio { ratio: clock.time_factor });
                // Anchor the display epoch on scenario time instead of
                // the wall-clock first tick. Idempotent per seed
                // contract; skipped when the answer carries no start.
                if let Some(anchor) = clock.assumed_start.as_deref() {
                    let _ = tx.send(SimCommand::SeedClockStart { ts: anchor.to_string() });
                }
                let _ = tx.send(SimCommand::SetPaused {
                    paused: !clock.running,
                });
            }
        }
        self.minos_clock = Some(clock);
    }

    /// H2: pause or resume the scenario clock through Minos, following
    /// the last known clock state (held → resume, otherwise pause).
    /// Unknown clock attempts pause — the write answers the clock
    /// either way, so unknown resolves itself loudly.
    fn pause_or_resume_minos(&mut self) {
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        if self.clock_denied {
            self.users_status = "clock control needs the control grant in this session — ask the Game Master".to_string();
            return;
        }
        if self.setup_busy("clock") {
            return;
        }
        let resume = self.minos_clock.as_ref().is_some_and(|c| !c.running);
        let verb = if resume { "resume" } else { "pause" };
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("Scenario clock {verb} refused: {e}");
                return;
            }
        };
        let verb_owned = verb.to_string();
        self.setup_op = Some(spawn_rest("clock", move || {
            let res = if resume {
                master.resume_game(&tok, gid)
            } else {
                master.pause_game(&tok, gid)
            };
            // Typed inside the worker: Forbidden is the capability
            // answer, everything else a loud transport failure.
            match res {
                Ok(c) => Ok(SetupDone::Clock(c, verb_owned)),
                Err(tfg::backend::BackendError::Forbidden { .. }) => {
                    Ok(SetupDone::ClockDenied)
                }
                Err(e) => Err(e.to_string()),
            }
        }));
    }

    /// H2: set the scenario rate through Minos (must be > 0 — zero is
    /// a pause through its own endpoint). Applies at once while
    /// running; while held only the chosen rate is stored.
    fn set_minos_factor(&mut self, factor: f64) {
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        if self.clock_denied {
            self.users_status = "clock control needs the control grant in this session — ask the Game Master".to_string();
            return;
        }
        if self.setup_busy("factor") {
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("Scenario rate refused: {e}");
                return;
            }
        };
        self.setup_op = Some(spawn_rest("factor", move || {
            match master.set_time_factor(&tok, gid, factor) {
                Ok(c) => Ok(SetupDone::Clock(c, format!("factor {factor}x"))),
                Err(tfg::backend::BackendError::Forbidden { .. }) => {
                    Ok(SetupDone::ClockDenied)
                }
                Err(e) => Err(e.to_string()),
            }
        }));
    }

    /// The held session's own plan, read back.
    ///
    /// This exists because `description` was WRITE-ONLY: the create form sent
    /// it, `parse_game_detail` dropped it, and nothing in the app ever read
    /// it — so the field showed blank after any reload. The fix is not a
    /// special case here but the parser keeping what the server sent; this
    /// is simply the first place that looks at it.
    ///
    /// READ ONLY on purpose. The edit form's contract is blank-means-
    /// unchanged with an empty string meaning CLEAR, so seeding those drafts
    /// from the detail would turn every save into "set all six fields to what
    /// they already are" — and for the two nullable columns that means every
    /// save CLEARS them.
    fn held_plan_prose(&mut self, ui: &mut egui::Ui) {
        let Some(d) = self.held_detail.clone() else {
            return;
        };
        ui.separator();
        ui.strong("The plan");
        // A field with nothing in it says so rather than rendering an empty
        // row, because six blank lines read as a broken panel rather than as
        // an exercise nobody has written down yet.
        let mut said_any = false;
        for (label, text) in [
            ("description", &d.description),
            ("purpose", &d.purpose),
            ("target", &d.target),
        ] {
            if !text.is_empty() {
                ui.label(egui::RichText::new(label).weak().small());
                ui.label(text.clone());
                said_any = true;
            }
        }
        // `area` is the one field the server withholds, and it omits BOTH
        // "unset" and "withheld" the same way — so the honest reading needs
        // who is asking. See `GameDetail::area_is_withheld`.
        let withheld = d.area_is_withheld(self.can_manage_sessions());
        match (&d.area, withheld) {
            (Some(area), _) => {
                ui.label(egui::RichText::new("area").weak().small());
                ui.label(area.clone());
                said_any = true;
            }
            (None, true) => {
                ui.label(
                    egui::RichText::new("area withheld until execution").weak().small(),
                );
                said_any = true;
            }
            (None, false) => {
                ui.label(egui::RichText::new("no area set").weak().small());
            }
        }
        if let Some(tag) = d.map_tag.clone() {
            ui.label(egui::RichText::new("map tag").weak().small());
            ui.label(egui::RichText::new(tag).monospace());
            said_any = true;
        }
        // The overlay is a DISPLAY rule, not a secret: the server never
        // withholds it, and it is the client's job to hide it once execution
        // begins. Whether this island honours that is a separate question
        // from whether it reads the field.
        if let Some(text) = d.overlay_text.clone() {
            let during_execution = d.state == "execution";
            ui.label(egui::RichText::new("overlay").weak().small());
            ui.label(if during_execution {
                egui::RichText::new(text).weak()
            } else {
                egui::RichText::new(text).strong()
            });
            said_any = true;
        }
        // The ends are the plan, and comparing them with the realised finish
        // is the point of closure — so all four, not just the starts.
        let mut ends = Vec::new();
        for (label, value) in [
            ("actual start", &d.window.actual_start),
            ("assumed start", &d.window.assumed_start),
            ("actual end", &d.window.actual_end),
            ("assumed end", &d.window.assumed_end),
        ] {
            if let Some(v) = value {
                ends.push(format!("{label} {v}"));
            }
        }
        if !ends.is_empty() {
            ui.label(egui::RichText::new(ends.join("  ·  ")).monospace().small());
            said_any = true;
        }
        // The pace, echoed back because the edit form can set it and nothing
        // else on screen would show what it became. `None` is an unknown
        // value, which the gate treats as standard — saying so is more honest
        // than printing a pace the server never sent.
        ui.label(
            egui::RichText::new(format!(
                "pace {}",
                d.pace.map_or("unknown — readiness applies", |p| p.wire())
            ))
            .weak()
            .small(),
        );
        if !said_any {
            ui.weak("Nothing written yet — the plan is authored while it is being written.");
        }
    }

    /// Held-game edit form (admin ticket): blank-means-unchanged
    /// rows; at least one filled to send. Empty area/map_tag clears
    /// (nullable columns); past planning the server refuses loudly.
    ///
    /// The pace and the planned window are authored HERE and not on
    /// `setup_game_ui`'s create form, and that is a decision rather than an
    /// oversight. Create authors a session that has a name; the window is a
    /// planning decision made after people know when they can turn up, and
    /// the edit form already exists for exactly that. A second authoring
    /// surface for the same field would be two places to keep in step for a
    /// defect that needs one.
    fn edit_game_ui(&mut self, ui: &mut egui::Ui, gid: i64) {
        ui.separator();
        ui.strong("Edit session (blank leaves alone)");
        ui.horizontal(|ui| {
            ui.label("name:");
            ui.text_edit_singleline(&mut self.edit_name);
        });
        ui.horizontal(|ui| {
            ui.label("description:");
            ui.text_edit_singleline(&mut self.edit_description);
        });
        ui.horizontal(|ui| {
            ui.label("purpose:");
            ui.text_edit_singleline(&mut self.edit_purpose);
        });
        ui.horizontal(|ui| {
            ui.label("target:");
            ui.text_edit_singleline(&mut self.edit_target);
        });
        ui.horizontal(|ui| {
            ui.label("area:");
            ui.text_edit_singleline(&mut self.edit_area);
        });
        ui.horizontal(|ui| {
            ui.label("map tag:");
            ui.text_edit_singleline(&mut self.edit_map_tag);
        });
        ui.weak("empty area / map tag clears the field.");
        ui.separator();
        ui.strong("Pace and planned window");
        ui.horizontal(|ui| {
            ui.label("pace:");
            // Three states, and the first is the form's contract: `fast`
            // waives the readiness declaration, so an untouched combo must
            // mean "unchanged" and never a default somebody never chose.
            egui::ComboBox::from_id_salt("edit-pace")
                .selected_text(match self.edit_pace.trim() {
                    "" => "leave alone".to_string(),
                    raw => raw.to_string(),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.edit_pace, String::new(), "leave alone");
                    for p in [tfg::backend::GamePace::Standard, tfg::backend::GamePace::Fast] {
                        ui.selectable_value(&mut self.edit_pace, p.wire().to_string(), p.wire());
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("real date:");
            ui.text_edit_singleline(&mut self.edit_actual_date);
        });
        ui.horizontal(|ui| {
            ui.label("real start:");
            ui.text_edit_singleline(&mut self.edit_actual_start);
        });
        ui.horizontal(|ui| {
            ui.label("real end:");
            ui.text_edit_singleline(&mut self.edit_actual_end);
        });
        ui.horizontal(|ui| {
            ui.label("exercise start:");
            ui.text_edit_singleline(&mut self.edit_assumed_start);
        });
        ui.horizontal(|ui| {
            ui.label("exercise end:");
            ui.text_edit_singleline(&mut self.edit_assumed_end);
        });
        ui.weak("military times (0800). The exercise clock carries no date. A window crossing midnight is two sessions.");
        ui.horizontal(|ui| {
            if ui.small_button("save").clicked() {
                self.save_game_edit(gid);
            }
            if ui.small_button("cancel").clicked() {
                self.edit_open = false;
            }
        });
    }

    /// The planned window the edit form's six drafts describe, converted to the
    /// RFC3339 pairs Minos takes.
    ///
    /// Blank is an omission and stays silent, which is the form's contract. A
    /// FILLED draft that will not convert is an error instead: dropping it
    /// would save cleanly, and the execution gate would go on blocking for a
    /// reason nothing on screen names.
    ///
    /// Ordering is checked with the same `hhmm_window_ok` the scenario
    /// composer uses, for the same reason. A window that crosses midnight
    /// cannot be written from here — one date carries both real ends, and the
    /// exercise clock has a single base — so it is two sessions, and saying so
    /// beats a 400 that names only the field.
    fn edit_window(&self) -> Result<tfg::backend::TimeWindow, String> {
        let mut w = tfg::backend::TimeWindow::default();
        let (a0, a1) = (self.edit_actual_start.trim(), self.edit_actual_end.trim());
        if !a0.is_empty() || !a1.is_empty() {
            let date = self.edit_actual_date.trim();
            if date.is_empty() {
                return Err("the real window needs a date as well as times".to_string());
            }
            if a0.is_empty() || a1.is_empty() {
                return Err("a window needs both a start and an end".to_string());
            }
            // Named per field, the way the scenario composer names its own:
            // "the window is invalid" across three inputs sends the author
            // hunting for the one that is wrong.
            for hhmm in [a0, a1] {
                if !tfg::backend::hhmm_ok(hhmm) {
                    return Err(format!("{hhmm} is not a military time like 0800"));
                }
            }
            if !tfg::backend::hhmm_window_ok(a0, a1) {
                return Err(format!("{a0}-{a1} is not a window that runs forwards"));
            }
            // Both times are known good and the pair runs forwards, so the
            // only thing left that `actual_to_rfc3339` can refuse is the date.
            let (Some(s), Some(e)) = (
                tfg::backend::actual_to_rfc3339(date, a0),
                tfg::backend::actual_to_rfc3339(date, a1),
            ) else {
                return Err(format!("{date} is not a date like 2026-11-01"));
            };
            w.actual_start = Some(s);
            w.actual_end = Some(e);
        }
        let (s0, s1) = (self.edit_assumed_start.trim(), self.edit_assumed_end.trim());
        if !s0.is_empty() || !s1.is_empty() {
            if s0.is_empty() || s1.is_empty() {
                return Err("a window needs both a start and an end".to_string());
            }
            if !tfg::backend::hhmm_window_ok(s0, s1) {
                return Err(format!("{s0}-{s1} is not a window that runs forwards"));
            }
            let (Some(s), Some(e)) = (
                tfg::backend::assumed_hhmm_to_rfc3339(s0),
                tfg::backend::assumed_hhmm_to_rfc3339(s1),
            ) else {
                return Err(format!("{s0}-{s1} is not a pair of military times"));
            };
            w.assumed_start = Some(s);
            w.assumed_end = Some(e);
        }
        Ok(w)
    }

    /// Send the held-game edit off-thread. All-blank refuses before
    /// any request; the answer applies like a bundle detail.
    fn save_game_edit(&mut self, gid: i64) {
        if self.setup_busy("update") {
            return;
        }
        let opt = |s: &str| {
            let t = s.trim();
            if t.is_empty() { None } else { Some(t.to_string()) }
        };
        // The combo holds exactly three states — leave alone, standard, fast
        // — so there is nothing here to validate that the widget did not, and
        // `parse("")` landing on `None` IS the "leave alone" case.
        let pace = tfg::backend::GamePace::parse(self.edit_pace.trim());
        let window = match self.edit_window() {
            Ok(w) => w,
            Err(problem) => {
                self.users_status = problem;
                return;
            }
        };
        let upd = tfg::backend::GameUpdate {
            name: opt(&self.edit_name.clone()),
            description: opt(&self.edit_description.clone()),
            purpose: opt(&self.edit_purpose.clone()),
            target: opt(&self.edit_target.clone()),
            area: opt(&self.edit_area.clone()),
            map_tag: opt(&self.edit_map_tag.clone()),
            pace,
            window,
        };
        // The check counts every field the form owns now, not only the six
        // prose rows: a window authored on its own is the whole point of the
        // form, and it must not be refused as "nothing filled".
        if upd.name.is_none()
            && upd.description.is_none()
            && upd.purpose.is_none()
            && upd.target.is_none()
            && upd.area.is_none()
            && upd.map_tag.is_none()
            && upd.pace.is_none()
            && upd.window.is_empty()
        {
            self.users_status = "fill at least one field to update".to_string();
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("update failed: {e}");
                return;
            }
        };
        self.edit_open = false;
        self.setup_op = Some(spawn_rest("update", move || {
            master
                .update_game(&tok, gid, &upd)
                .map_err(|e| e.to_string())
                .map(SetupDone::GameUpdated)
        }));
    }

    /// Delete the held game off-thread (second click got here): the
    /// apply drops the hold, which reads like a vanished game.
    fn delete_game(&mut self, gid: i64) {
        if self.setup_busy("delete") {
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("delete failed: {e}");
                return;
            }
        };
        let name = self
            .users_game
            .clone()
            .map(|(_, n)| n)
            .unwrap_or_else(|| format!("session {gid}"));
        self.setup_op = Some(spawn_rest("delete", move || {
            master
                .delete_game(&tok, gid)
                .map_err(|e| e.to_string())
                .map(|_| SetupDone::GameDeleted(gid, name))
        }));
    }

    /// The session browser and lifecycle writes are organizer-only.
    /// Room-key join remains available to every authenticated account,
    /// so a participant never has to see an unusable empty picker.
    fn can_manage_sessions(&self) -> bool {
        self.games_loaded && !self.games_gap && self.has_app_role()
    }

    /// The session combo and its refresh, shared by both places a session can
    /// be chosen from.
    ///
    /// It was only ever drawn from the Essentials island, which renders while a
    /// session is HELD — so on a cold start, with nothing held, there was no
    /// way back to an existing session and the only verb was "create one". A
    /// render showed it: the top band says "Choose or join a session" above an
    /// island whose sole control creates a new one. The no-session island now
    /// offers the list too, because re-opening yesterday's exercise is the
    /// common case and creating a duplicate is the expensive mistake.
    fn session_picker_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let picked = self
                .users_game
                .clone()
                .map(|(_, n)| n)
                .unwrap_or_else(|| "pick a session".to_string());
            let prev = self.users_game.clone();
            let mut selected_game = prev.clone();
            egui::ComboBox::from_label("session")
                .selected_text(picked)
                .show_ui(ui, |ui| {
                    for g in &self.users_games {
                        ui.selectable_value(
                            &mut selected_game,
                            Some((g.id, g.name.clone())),
                            format!("{} ({})", g.name, g.state),
                        );
                    }
                });
            if ui.small_button("refresh").clicked() {
                self.users_refresh_games();
                self.users_refresh_directory();
                self.users_refresh_game();
            }
            if selected_game != prev {
                // A newly held game takes its stage from Minos, never
                // from a local default: the resync inside the refresh
                // projects Planning/Ready/Live/Eval off the detail read.
                // Route the picker through the same held-game boundary
                // as joins and bundle applies so old Registry data cannot
                // survive a direct assignment.
                self.set_held_game(selected_game);
                // The clock read belongs to the old hold — writes will
                // re-read it for the new one.
                self.minos_clock = None;
                self.minos_room_key = None;
                self.clock_denied = false; // new hold, unknown grant
                self.delete_armed = false;
                self.edit_open = false;
                self.users_refresh_game();
            }
        });
    }

    /// Whether the no-session island offers the session list.
    ///
    /// One predicate for the row and for the island's declared height: the two
    /// were separate conditions once, and a picker drawn into a 168pt island is
    /// a clipped picker.
    fn session_picker_visible(&self) -> bool {
        self.can_manage_sessions() && !self.users_games.is_empty()
    }

    /// Setup flow step 1 (#79): hold a game — pick a listed one or
    /// create a session. Only `name` is required; blank optionals are
    /// omitted, never sent empty.
    fn setup_game_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("1 · Session");
        let can_manage_sessions = self.can_manage_sessions();
        // Player flow: the room key is the invitation. Do not leave an
        // organizer-only picker on screen for accounts that can only join.
        if !can_manage_sessions && self.users_game.is_none() {
            ui.label(
                egui::RichText::new(
                    "Only organizer accounts can browse or create sessions. \
                     Ask your organizer for the room key and enter it below — \
                     no list needed.",
                )
                .strong(),
            );
            ui.separator();
        }
        if can_manage_sessions {
            self.session_picker_row(ui);
            // Admin writes: staff-granted, planning-only. Hidden without
            // the staff read (a player would only 403); past planning the
            // server refuses loudly instead.
            if let Some((gid, _)) = self.users_game.clone() {
                let planning = self.users_game_state.as_deref() == Some("planning");
                ui.horizontal(|ui| {
                    if ui.small_button("Edit session (blank leaves alone)").clicked() {
                        self.edit_open = !self.edit_open;
                    }
                    if planning {
                        let label = if self.delete_armed {
                            "confirm delete"
                        } else {
                            "delete"
                        };
                        if ui.small_button(label).clicked() {
                            if self.delete_armed {
                                self.delete_armed = false;
                                self.delete_game(gid);
                            } else {
                                self.delete_armed = true;
                            }
                        }
                    }
                });
                if self.delete_armed {
                    ui.weak("Delete discards the session and revokes every seat — click again to confirm.");
                }
                if self.edit_open {
                    self.edit_game_ui(ui, gid);
                }
            }
        }
        self.held_plan_prose(ui);
        status_line(ui, &self.users_status.clone());
        ui.separator();
        // C2: room-key join. The key is the only input — the answer
        // says which game was entered, and the hold + stage follow it.
        // The held session's key is read out HERE, above the input:
        // this is where the Game Master stands when personnel arrive
        // without it, so it is where the share-out belongs.
        ui.strong("Join with room key");
        match self.minos_room_key.clone() {
            Some(key) => {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&key).monospace().strong());
                    if ui.small_button("copy").on_hover_text("copy the room key").clicked() {
                        ui.ctx().copy_text(key.clone());
                        self.users_status = "room key copied".to_string();
                    }
                });
                ui.weak("the held session's key — share it with your personnel; they type it below.");
            }
            None => {
                let planning = self.users_game_state.as_deref() == Some("planning");
                ui.weak(if planning {
                    "no key yet — the exercise creates one when the session enters preparation (step 4)."
                } else {
                    "no key on the held session — ask its Game Master."
                });
            }
        }
        ui.horizontal(|ui| {
            ui.label("room key:");
            ui.text_edit_singleline(&mut self.join_key);
            if ui.small_button("join").clicked() {
                self.join_with_key();
            }
        });
        ui.weak("joining needs a seat first — a valid key without one is refused (ask the Game Master).");
        // Planning → preparation carries no gate (the Game Master
        // decides planning is done), and it is what mints the key —
        // so the action sits beside the key it produces, not behind
        // step 4's piece lock.
        if self.users_game_state.as_deref() == Some("planning") {
            if ui.button("Enter preparation →").on_hover_text("mint the room key, invite personnel in").clicked() {
                self.setup_advance_prep();
            }
        }
        if can_manage_sessions {
            ui.separator();
            ui.strong("New session");
            ui.horizontal(|ui| {
                ui.label("name:");
                ui.text_edit_singleline(&mut self.setup_name);
                if ui.small_button("create").clicked() {
                    self.setup_create_game();
                }
            });
            ui.horizontal(|ui| {
                ui.label("description:");
                ui.text_edit_singleline(&mut self.setup_description);
            });
            ui.horizontal(|ui| {
                ui.label("purpose:");
                ui.text_edit_singleline(&mut self.setup_purpose);
            });
            ui.horizontal(|ui| {
                ui.label("target:");
                ui.text_edit_singleline(&mut self.setup_target);
            });
            ui.horizontal(|ui| {
                ui.label("area:");
                ui.text_edit_singleline(&mut self.setup_area);
            });
            ui.horizontal(|ui| {
                ui.label("map tag:");
                ui.text_edit_singleline(&mut self.setup_map_tag);
            });
            ui.weak("mode is always maneuver — the only mode this release.");
        // The scenario book's entry point, on the island that already owns
        // the held session's own facts.
        //
        // ABSENT outside planning rather than greyed out: authoring is a
        // planning-time act and the server refuses it after, so a control
        // that would 403 is a lie about what this session can do.
        if self.users_game.is_some() && self.users_game_state.as_deref() == Some("planning") {
            ui.horizontal(|ui| {
                if ui
                    .button("Scenario composer")
                    .on_hover_text("author this session's scenarios and the steps players see")
                    .clicked()
                {
                    self.open_composer(None);
                }
                if !self.scenarios.is_empty() {
                    let steps: usize = self.scenarios.iter().map(|s| s.steps.len()).sum();
                    ui.weak(format!(
                        "{} scenario(s), {steps} step(s).",
                        self.scenarios.len()
                    ));
                }
            });
        }
        }
    }

    /// Setup flow step 2 (#79): seat accounts into game roles. One
    /// role per person; a role change clears readiness.
    fn setup_players_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("2 · Players");
        if self.users_game.is_none() {
            ui.weak("Pick or create a session in step 1 first.");
            return;
        }
        // Player flow: seating is staff-only — the seat arrives with
        // the room key, and the caller's own row shows in step 4.
        // A directory/role surface here would 403 one panel at a time.
        if self.roster_gap {
            ui.weak(
                "Seating this session needs a staff read your account lacks. \
                 Your seat arrived with the room key — it shows, with your \
                 readiness, in step 4.",
            );
            status_line(ui, &self.users_status.clone());
            return;
        }
        // The picker is a modal, so this island is the roster SUMMARY and the
        // door. Seating is one decision about a person — who, as what, and for
        // some roles which fleet — and in a 320-point column that decision
        // needs room the island does not have.
        ui.horizontal(|ui| {
            let seated = self.users_roster.len();
            if ui
                .button(if seated > 0 {
                    format!("Player picker ({seated} seated)")
                } else {
                    "Player picker".to_string()
                })
                .on_hover_text("seat accounts into game roles")
                .clicked()
            {
                self.open_only(WhichModal::Player);
                // Read what the modal exists to show, on the way in.
                //
                // The composer already does this (`open_composer` calls
                // `load_scenarios`) and the picker did not, so opening it
                // rendered an empty directory with no read attempted. The
                // Minos log is what settles it: pressing the button produced
                // no GET /users at all, and the read only happened when the
                // operator found the `find` button by looking for one.
                self.users_refresh_directory();
            }
            if let Some(p) = self.own_roster_row() {
                ui.label(egui::RichText::new(format!("you: {}", p.role_name)).weak().small());
            }
        });
        ui.separator();
        // The seated list stays here: it is the state the operator reads
        // back, and a count alone would not say who is missing.
        self.users_roster_ui(ui);
        status_line(ui, &self.users_status.clone());
    }

    /// Reload the Fleet render cache from the mirror: rows, branch
    /// mapping counts, branch display names. Called on sync apply
    /// and first show — the render path only clones the cache.
    fn reload_fleet_cache(&mut self) {
        let Some(conn) = self.store.as_ref() else {
            self.fleet_cache.clear();
            self.fleet_branches.clear();
            self.fleet_branch_names.clear();
            self.fleet_loaded = true;
            return;
        };
        self.fleet_cache = tfg::store::fleet_units(conn).unwrap_or_default();
        self.fleet_branches = tfg::store::branch_mapped_counts(conn);
        self.fleet_branch_names.clear();
        let mut bids: Vec<i64> = self.fleet_branches.keys().cloned().collect();
        for r in &self.fleet_cache {
            if let Some(b) = r.branch_id {
                if !bids.contains(&b) {
                    bids.push(b);
                }
            }
        }
        for bid in bids {
            if let Some((n, idn)) = tfg::store::branch_label(conn, bid) {
                let label = if idn.is_empty() || idn == n { n } else { format!("{n} / {idn}") };
                self.fleet_branch_names.insert(bid, label);
            }
        }
        self.fleet_loaded = true;
    }

    fn load_picker_textures(&mut self, ctx: &egui::Context) {
        for row in self.fleet_cache.clone() {
            let Ok(unit_id) = row.id.parse::<i64>() else {
                continue;
            };
            let Some(visual) = self.visuals.get(unit_id) else {
                continue;
            };
            if visual.texture.is_some() || visual.asset_kind != AssetKind::UnitImage {
                continue;
            }
            let Some(source) = visual.image_url.clone() else {
                continue;
            };
            if visual
                .image_url_retry_at
                .is_some_and(|retry_at| retry_at > Instant::now())
                || visual
                    .image_url_expires_at
                    .is_some_and(|expires_at| expires_at <= Instant::now())
            {
                continue;
            }
            let hint = visual
                .width_px
                .filter(|width| *width > 0)
                .map(egui::load::SizeHint::Width)
                .unwrap_or_else(|| egui::load::SizeHint::Scale(1.0.into()));
            if let Ok(egui::load::TexturePoll::Ready { texture }) =
                ctx.try_load_texture(&source, egui::TextureOptions::LINEAR, hint)
            {
                self.visuals.set_texture(unit_id, texture);
            }
        }
    }

    /// Small visual cell shared by picker rows and the drag ghost. A
    /// usable image wins; the catalog symbol is the quiet fallback.
    fn unit_thumbnail_ui(&self, ui: &mut egui::Ui, id: &str, size: f32) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        let Ok(uid) = id.parse::<i64>() else {
            painter.circle_filled(
                rect.center(),
                size * 0.43,
                egui::Color32::from_rgb(30, 58, 79),
            );
            paint_map_symbol(
                &painter,
                rect.center(),
                tfg::store::MapSymbol::UnknownShip,
                false,
                1.0,
            );
            return;
        };
        if let Some(texture) = self.visuals.get(uid).and_then(|visual| visual.texture.clone()) {
            painter.image(
                texture.id,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            let symbol = self.symbol_for_unit(uid);
            painter.circle_filled(
                rect.center(),
                size * 0.43,
                egui::Color32::from_rgb(30, 58, 79),
            );
            paint_map_symbol(
                &painter,
                rect.center(),
                symbol,
                false,
                1.0,
            );
        }
    }

    /// Fleet picker for the setup step. The operator keeps the five
    /// Miller columns visible; the result row is the only place a unit
    /// is selected or dragged.
    fn unit_picker_ui(&mut self, ui: &mut egui::Ui) -> Vec<(i64, String)> {
        ui.horizontal(|ui| {
            if ui.small_button("sync register").clicked() {
                self.sync_now();
                self.users_refresh_directory();
            }
            ui.label("search all units");
            ui.text_edit_singleline(&mut self.fleet_query);
            if ui.small_button("clear").clicked() {
                self.fleet_query.clear();
                self.drill_branch = None;
                self.drill_category = None;
                self.drill_type = None;
                self.drill_class = None;
            }
        });
        status_line(ui, &self.sync_status.clone());

        if !self.fleet_loaded {
            self.reload_fleet_cache();
        }
        self.load_picker_textures(ui.ctx());
        let has_taxonomy = self
            .store
            .as_ref()
            .and_then(|conn| tfg::store::has_taxonomy(conn).ok())
            .unwrap_or(false);
        let rows = if has_taxonomy {
            self.miller_rows_ui(ui)
        } else {
            self.asset_picker_rows()
        };
        self.picker_tail_ui(ui, &rows)
    }

    fn asset_picker_rows(&self) -> Vec<PickerRow> {
        let query = self.fleet_query.to_lowercase();
        self.fleet
            .units()
            .iter()
            .filter(|unit| {
                query.is_empty()
                    || format!(
                        "{} {} {} {} {} {}",
                        unit.name,
                        unit.hull,
                        unit.role,
                        unit.origin,
                        unit.satuan,
                        unit.pangkalan
                    )
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|unit| {
                let class_name = self
                    .catalog
                    .class(&unit.class_id)
                    .map(|class| class.name.clone())
                    .unwrap_or_else(|| unit.class_id.clone());
                PickerRow {
                    id: unit.id.clone(),
                    name: unit.name.clone(),
                    hull: unit.hull.clone(),
                    class_name,
                    stat_class: Some(unit.class_id.clone()),
                    trail: String::new(),
                }
            })
            .collect()
    }

    fn miller_rows_ui(&mut self, ui: &mut egui::Ui) -> Vec<PickerRow> {
        let query = self.fleet_query.to_lowercase();
        if !query.is_empty() {
            let mut out = Vec::new();
            if let Some(conn) = self.store.as_ref() {
                if let Ok(hits) = tfg::store::tax_search(conn, &query) {
                    for hit in hits {
                        let class_name = hit.class_name.clone();
                        let stat_class = self
                            .catalog
                            .find_class_by_name(&class_name)
                            .map(|class| class.id.clone());
                        let trail = hit.trail();
                        out.push(PickerRow {
                            id: hit.id,
                            name: hit.name,
                            hull: hit.hull,
                            class_name,
                            stat_class,
                            trail,
                        });
                    }
                }
            }
            // A search is not a drill, so there is no column to put these in —
            // the point of the query is to skip the narrowing. They are drawn
            // here, at the width the panel has, rather than returned for
            // someone else to draw: the leaf column is the drill's, and a
            // search result that borrowed it would be a column whose contents
            // do not belong to the level it claims to be.
            let strip_h = picker_strip_height(ui.available_height(), picker_row_height(&ui));
            ui.label(format!("{} match{}", out.len(), if out.len() == 1 { "" } else { "es" }));
            egui::ScrollArea::vertical()
                .id_salt("drill-search")
                .max_height(strip_h)
                .show(ui, |ui| {
                    if out.is_empty() {
                        ui.weak("No unit matches that.");
                    }
                    for row in out.iter() {
                        self.picker_leaf_row_ui(ui, row);
                    }
                });
            return out;
        }

        let (branch, category, unit_type) =
            (self.drill_branch, self.drill_category, self.drill_type);
        let (branches, categories, types, classes) = match self.store.as_ref() {
            Some(conn) => (
                tfg::store::tax_branches(conn).unwrap_or_default(),
                branch
                    .map(|id| tfg::store::tax_categories(conn, id).unwrap_or_default())
                    .unwrap_or_default(),
                category
                    .map(|id| tfg::store::tax_types(conn, id).unwrap_or_default())
                    .unwrap_or_default(),
                unit_type
                    .map(|id| tfg::store::tax_classes(conn, id).unwrap_or_default())
                    .unwrap_or_default(),
            ),
            None => (Vec::new(), Vec::new(), Vec::new(), Vec::new()),
        };

        let label = |row: &tfg::store::TaxRow| {
            if row.id_name.is_empty() || row.id_name == row.name {
                row.name.clone()
            } else {
                format!("{} ({})", row.name, row.id_name)
            }
        };
        let crumb = |rows: &[tfg::store::TaxRow], id: Option<i64>| {
            id.and_then(|selected| rows.iter().find(|row| row.id == selected))
                .map(label)
        };
        let crumbs: Vec<String> = [
            crumb(&branches, self.drill_branch),
            crumb(&categories, self.drill_category),
            crumb(&types, self.drill_type),
            crumb(&classes, self.drill_class),
        ]
        .into_iter()
        .flatten()
        .collect();
        ui.horizontal_wrapped(|ui| {
            if ui.small_button("Fleet").clicked() {
                self.drill_branch = None;
                self.drill_category = None;
                self.drill_type = None;
                self.drill_class = None;
            }
            for crumb in crumbs {
                ui.label("/");
                ui.label(crumb);
            }
        });

        if self.drill_branch != branch {
            self.drill_category = None;
            self.drill_type = None;
            self.drill_class = None;
        }
        if self.drill_category != category {
            self.drill_type = None;
            self.drill_class = None;
        }
        if self.drill_type != unit_type {
            self.drill_class = None;
        }

        // The leaf, read before it is drawn rather than after the strip.
        //
        // It used to be gathered into a `rows` vec AFTER the columns and
        // handed to `picker_tail_ui`, which drew it as a list UNDER the
        // columns. Two things were wrong with that and both are the same
        // mistake. The drill is five levels deep and `tax_units` is documented
        // as the leaf, so the one level that is actually selectable was not a
        // column — the columns narrowed to a class and stopped, with dead
        // space to the right of the fourth. And the list it went to was
        // painted below the panel's bottom edge: a modal body is a fixed rect
        // with no scroll, so the units were not merely below the others, they
        // were unreachable. Nothing in the panel could bring them on screen.
        let mut rows = Vec::new();
        if let (Some(conn), Some(class_id)) = (self.store.as_ref(), self.drill_class) {
            if let Ok(units) = tfg::store::tax_units(conn, class_id) {
                for unit in units {
                    rows.push(PickerRow {
                        id: unit.id,
                        name: unit.name,
                        hull: unit.hull,
                        class_name: unit.class_name.clone(),
                        stat_class: self
                            .catalog
                            .find_class_by_name(&unit.class_name)
                            .map(|class| class.id.clone()),
                        trail: String::new(),
                    });
                }
            }
        }

        // The strip's height is what the panel has left, less the tail's
        // reserve. A stored height with a slider let the columns grow into
        // the tail's space, which is how the tail came to be off the panel
        // in the first place.
        let strip_h = picker_strip_height(ui.available_height(), picker_row_height(&ui));
        let rows = rows.clone();
        // The strip's rect is ALLOCATED, not left to whatever the columns
        // happen to measure to.
        //
        // Three attempts at this all failed the same way: a `max_height` on
        // the inner scroll area, and then a label height subtracted from it,
        // and both left the columns consuming every point available and the
        // tail with none. `max_height` bounds the list inside a column; it does
        // not bound the column, and the column is what the parent lays out.
        // The honest arrangement is to give the strip the rect it is allowed
        // and let the columns live inside it, which is what
        // `scope_builder` with a `max_rect` does — the same thing the modal
        // body already does for itself, for the same reason.
        let (strip_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), strip_h),
            egui::Sense::hover(),
        );
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(strip_rect),
            |ui| {
                egui::ScrollArea::horizontal()
            .id_salt("drill-miller")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    let levels: [(&str, &[tfg::store::TaxRow], bool); 4] = [
                        ("Branch", &branches, true),
                        ("Category", &categories, self.drill_branch.is_some()),
                        ("Type", &types, self.drill_category.is_some()),
                        ("Class", &classes, self.drill_type.is_some()),
                    ];
                    for (depth, (title, options, active)) in levels.iter().enumerate() {
                        ui.vertical(|ui| {
                            ui.set_min_width(MILLER_COL_WIDTH);
                            ui.set_max_width(MILLER_COL_WIDTH);
                            ui.strong(format!("{title} ({})", options.len()));
                            egui::ScrollArea::vertical()
                                .id_salt(("drill", *title))
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.set_min_width(MILLER_COL_WIDTH - 16.0);
                                    if !active {
                                        ui.weak("Pick ← first");
                                    } else if options.is_empty() {
                                        ui.weak("None yet");
                                    } else {
                                        for option in options.iter() {
                                            let text = label(option);
                                            match depth {
                                                0 => ui.selectable_value(
                                                    &mut self.drill_branch,
                                                    Some(option.id),
                                                    text,
                                                ),
                                                1 => ui.selectable_value(
                                                    &mut self.drill_category,
                                                    Some(option.id),
                                                    text,
                                                ),
                                                2 => ui.selectable_value(
                                                    &mut self.drill_type,
                                                    Some(option.id),
                                                    text,
                                                ),
                                                _ => ui.selectable_value(
                                                    &mut self.drill_class,
                                                    Some(option.id),
                                                    text,
                                                ),
                                            };
                                        }
                                    }
                                });
                        });
                        ui.separator();
                    }
                    // The leaf column: the fifth level, and the only one whose
                    // rows are draggable, so it is the one that gets the width
                    // for a thumbnail, a name and a verb.
                    ui.vertical(|ui| {
                        ui.set_min_width(MILLER_LEAF_WIDTH);
                        ui.set_max_width(MILLER_LEAF_WIDTH);
                        ui.strong(format!("Hull ({})", rows.len()));
                        egui::ScrollArea::vertical()
                            .id_salt("drill-leaf")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.set_min_width(MILLER_LEAF_WIDTH - 16.0);
                                if self.drill_class.is_none() {
                                    ui.weak("Pick a class");
                                } else if rows.is_empty() {
                                    ui.weak("None yet");
                                } else {
                                    for row in rows.iter() {
                                        self.picker_leaf_row_ui(ui, row);
                                    }
                                }
                            });
                    });
                });
            });
            },
        );
        rows
    }

    /// One row of the leaf column: thumbnail, name, hull, and the assign verb.
    ///
    /// Split out of `picker_tail_ui` so the drag and the click are registered
    /// against the row wherever it is drawn, rather than being a property of
    /// the list that happened to be below the columns.
    fn picker_leaf_row_ui(&mut self, ui: &mut egui::Ui, row: &PickerRow) {
        let assigned = row
            .id
            .parse::<i64>()
            .ok()
            .is_some_and(|uid| self.force.contains(uid));
        let armed = self.fleet_pick.as_deref() == Some(row.id.as_str());
        let response = ui.horizontal(|ui| {
            self.unit_thumbnail_ui(ui, &row.id, 28.0);
            ui.vertical(|ui| {
                ui.label(row.name.clone());
                ui.weak(format!(
                    "{} ({}){}",
                    row.class_name,
                    row.hull,
                    if row.stat_class.is_some() {
                        " · stats ready"
                    } else {
                        " · no sim stats"
                    }
                ));
            });
            if armed {
                ui.label(egui::RichText::new("armed").weak().small());
            } else if assigned {
                ui.label(egui::RichText::new("assigned").weak().small());
            } else if ui.small_button("assign").clicked() {
                self.pending_assign = row.id.parse::<i64>().ok().map(|id| (id, row.name.clone()));
            }
        });
        // Selected state painted by hand, because `selectable_label` is not
        // available on a row that also has to be a drag source and a button.
        if armed {
            ui.painter().rect_stroke(
                response.response.rect.expand(2.0),
                egui::CornerRadius::same(3),
                egui::Stroke::new(1.0, tfg::tokens::RADAR_CYAN),
                egui::StrokeKind::Inside,
            );
        }
        let row_response = ui.interact(
            response.response.rect,
            ui.id().with(("unit-row", row.id.as_str())),
            egui::Sense::click_and_drag(),
        );
        if row_response.is_pointer_button_down_on() && self.unit_drag.is_none() {
            self.arm_fleet_pick(row.id.clone());
            if let Some(start) = row_response.interact_pointer_pos() {
                self.unit_drag = Some(UnitDrag {
                    id: row.id.clone(),
                    name: row.name.clone(),
                    start,
                    moved: false,
                });
            }
        }
        if row_response.clicked() && self.pending_assign.is_none() {
            self.arm_fleet_pick(row.id.clone());
            self.note_placement(format!("{} selected · click the map to place", row.name));
        }
    }

    /// The count, and the placement controls.
    ///
    /// Only those two things. It used to also draw the rows, as a list under
    /// the columns, and that is the whole defect: the drill's leaf belongs in
    /// the column set beside the four levels that narrow to it, and a list
    /// painted below a fixed-height panel with no scroll is a list nobody can
    /// reach. What is left here is what genuinely is not a row.
    fn picker_tail_ui(&mut self, ui: &mut egui::Ui, rows: &[PickerRow]) -> Vec<(i64, String)> {
        ui.label(format!(
            "{} shown · {} placed",
            rows.len(),
            self.placed_fleet.len()
        ));

        // Why the last placement did not happen, said HERE.
        //
        // A drag out of this picker ends on the map, and every way that can end
        // — refused for land, refused for a disarmed engine, released over the
        // form instead of the map, dropped on a hull already placed — is a fact
        // about the PLACEMENT. It reads from `placement_note` and not from
        // `users_status`, because `users_status` is shared with the register
        // sync and the sync runs on a timer: the first version of this line read
        // `users_status`, and the refusal was overwritten by "pictures: 0 hull(s)
        // listed" before the operator could look at it. The reason was there and
        // it vanished, which is the same thing as never saying it.
        let note = self.placement_note.0.clone();
        if !note.is_empty() {
            status_line(ui, &note);
        }

        let armed = self.mode.armed.load(Ordering::SeqCst);
        if self.mode.phase == Phase::Closed {
            ui.label("Placement is unavailable once the session is closed.");
        } else if !armed {
            // One row, for the reason `PICKER_TAIL_ROWS` gives: the tail's
            // height is a budget the strip is sized against, so a branch that
            // prints a line and then offers a button underneath it spends a row
            // the columns needed.
            ui.horizontal(|ui| {
                ui.label("Engine is presentation-only: placed units stay invisible until it runs.");
                if ui.small_button("arm engine").clicked() {
                    self.mode.armed.store(true, Ordering::SeqCst);
                }
            });
        } else if self.acting_as.is_some() {
            ui.label("Placement is organizer-only.");
        } else if self
            .fleet_pick
            .as_ref()
            .is_some_and(|id| self.placed_fleet.contains(id))
        {
            ui.label("Already placed — pick another unit.");
        } else if let Some(pick) = self.fleet_pick.clone() {
            if let Some(row) = rows.iter().find(|row| row.id == pick) {
                let placing = self.mode.tool == SetupTool::Place;
                // Side by side, and this is a layout requirement rather than a
                // tidy-up: the tail sits under a fixed-height strip whose budget
                // is a row count, so two buttons stacked are two rows and the
                // second one falls off the bottom of the panel — which is the
                // defect this whole arrangement exists to end, reproduced one
                // row down.
                ui.horizontal(|ui| {
                    if ui
                        .small_button(if placing {
                            format!("cancel placing {}…", row.name)
                        } else {
                            format!("place {}", row.name)
                        })
                        .clicked()
                    {
                        if placing {
                            self.clear_fleet_pick();
                            self.users_status = "placement cancelled".to_string();
                        } else {
                            self.arm_fleet_pick(pick.clone());
                        }
                    }
                    if ui.button("place at map center").clicked() {
                        let (la, lo) = self.center;
                        self.try_place_picked(la, lo);
                        self.mode.tool = SetupTool::Select;
                    }
                });
            } else {
                self.clear_fleet_pick();
                ui.label("Pick a hull from the last column to arm placement.");
            }
        } else {
            ui.label("Pick a hull from the last column to arm placement.");
        }
        // The leaf column cannot return a value out of a layout closure the
        // way the old list could, so an assign is parked here and drained here.
        // Taken after the placement controls, which read `fleet_pick` and would
        // otherwise see the row armed by the same click that assigned it.
        match self.pending_assign.take() {
            Some(assign) => vec![assign],
            None => Vec::new(),
        }
    }

    /// Setup flow step 3 (#79): assign register hulls as commanded
    /// pieces. The commander must already be seated (step 2) and never
    /// judge-side — the contract refuses anything else.
    fn setup_fleet_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("3 · Fleet");
        if self.users_game.is_none() {
            ui.weak("Pick or create a session in step 1 first.");
            return;
        }
        // Player flow: the full fleet is staff-only — but the join
        // answer's commanded hulls are the caller's own, with placed
        // state from the setup view. Assigning stays staff-side.
        if self.units_gap {
            ui.weak(
                "The full fleet needs a staff read your account lacks. \
                 Your hulls arrived with the room key — place them by \
                 clicking the map.",
            );
            if self.commanded_hulls.is_empty() {
                ui.weak("No hulls commanded — ask the Game Master for a command.");
            } else {
                let mut lifts: Vec<(i64, String)> = Vec::new();
                for gu in self.commanded_hulls.clone() {
                    ui.horizontal(|ui| {
                        let mut label = gu.unit_name.clone();
                        if !gu.hull_number.is_empty() {
                            label += &format!(" ({})", gu.hull_number);
                        }
                        ui.label(label);
                        // Placed state is the setup view's — gapped, it
                        // says so instead of calling hulls unplaced.
                        if self.placements_gap {
                            ui.label(
                                egui::RichText::new("placement state unavailable")
                                    .weak()
                                    .small(),
                            );
                            return;
                        }
                        let placed = self
                            .users_placements
                            .iter()
                            .any(|p| p.unit_id == gu.unit_id);
                        if placed {
                            ui.label(egui::RichText::new("placed").weak().small());
                            if ui.small_button("lift").clicked() {
                                lifts.push((gu.unit_id, gu.unit_name.clone()));
                            }
                        } else {
                            ui.label(egui::RichText::new("unplaced").weak().small());
                        }
                    });
                }
                for (unit, name) in lifts {
                    self.lift_placement_remote(unit, &name);
                }
            }
            status_line(ui, &self.users_status.clone());
            return;
        }
        let crew = self.setup_crew();
        // The picker is a modal, so the island offers the door and keeps the
        // RESULT. The list of staged pieces is the thing the operator comes
        // back to read — the register with its four Miller columns is a
        // means, not a summary, and in a 320-point column it would be the only
        // thing on the island.
        ui.horizontal(|ui| {
            let staged = self.force.len();
            if ui
                .button(if staged > 0 {
                    format!("Fleet picker ({staged} in the draft)")
                } else {
                    "Fleet picker".to_string()
                })
                .on_hover_text("browse the register and drag a hull onto the map")
                .clicked()
            {
                self.open_only(WhichModal::Fleet);
            }
            if self.fleet_pick.is_some() {
                ui.weak(
                    "a hull is armed \u{2014} drag it from the picker onto the map, \
                     or place it at the centre from inside.",
                );
            }
        });
        ui.strong("Pieces");
        if self.force.is_empty() {
            ui.weak("No pieces yet — assign register hulls above.");
        } else {
            let mut commanding: Vec<(i64, i64)> = Vec::new();
            let mut removals: Vec<(i64, String)> = Vec::new();
            let mut lifts: Vec<(i64, String)> = Vec::new();
            for hull in self.force.hulls().cloned().collect::<Vec<_>>() {
                let placed = hull.start.is_some();
                let unit_id = hull.unit_id.to_string();
                let on_server = self.users_gunits.iter().any(|g| g.unit_id == hull.unit_id);
                ui.push_id(hull.unit_id, |ui| {
                    ui.horizontal(|ui| {
                        self.unit_thumbnail_ui(ui, &unit_id, 28.0);
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(hull.name.as_str()).strong());
                            if !hull.hull_number.is_empty() {
                                ui.weak(format!("Hull {}", hull.hull_number));
                            }
                        });
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui.small_button("remove").clicked() {
                                    removals.push((hull.unit_id, hull.name.clone()));
                                }
                                if placed && ui.small_button("lift").clicked() {
                                    lifts.push((hull.unit_id, hull.name.clone()));
                                }
                            },
                        );
                    });
                    ui.horizontal(|ui| {
                        let state = if placed {
                            egui::RichText::new("Placed")
                                .color(egui::Color32::from_rgb(74, 222, 128))
                        } else {
                            egui::RichText::new("Unplaced").color(egui::Color32::GRAY)
                        };
                        ui.label(state.small());
                        if !on_server {
                            ui.weak(egui::RichText::new("· local, not on Minos").small());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.weak("Commander");
                        let mut tmp = hull.commander_id;
                        let cmdr_name = hull
                            .commander_id
                            .and_then(|id| crew.iter().find(|(cid, _)| *cid == id))
                            .map(|(_, name)| name.clone())
                            .unwrap_or_else(|| "Unassigned".to_string());
                        egui::ComboBox::from_id_salt("commander")
                            .selected_text(cmdr_name)
                            .show_ui(ui, |ui| {
                                for (uid, name) in &crew {
                                    if hull.commander_id != Some(*uid)
                                        && ui
                                            .selectable_value(&mut tmp, Some(*uid), name)
                                            .clicked()
                                    {
                                        commanding.push((hull.unit_id, *uid));
                                    }
                                }
                            });
                    });
                    ui.separator();
                });
            }
            for (unit, cmdr) in commanding {
                self.users_command(unit, cmdr);
            }
            for (unit, name) in removals {
                self.setup_remove_unit(unit, &name);
            }
            for (unit, name) in lifts {
                self.lift_placement(unit, &name);
            }
            let pending = self.force.pending(&self.live_hulls());
            if pending > 0 {
                // The note and the verbs are stacked, not side by side: the
                // island is 320pt and the note plus two buttons in one row
                // overflows it, which pushes the last button past the clip
                // rectangle — where egui still draws it and the pointer no
                // longer reaches it. A dead control is worse than a taller
                // island.
                ui.weak(format!("{pending} change(s) not yet on Minos"));
                ui.horizontal(|ui| {
                    if !self.force.uncommanded().is_empty() {
                        if ui.small_button("fill commanders").clicked() {
                            if let Some((cid, _)) = crew.first() {
                                let uncommanded: Vec<i64> = self
                                    .force
                                    .uncommanded()
                                    .iter()
                                    .map(|h| h.unit_id)
                                    .collect();
                                for uid in uncommanded {
                                    self.force.set_commander(uid, Some(*cid));
                                }
                            }
                        }
                    }
                    if ui.small_button("sync to Minos").clicked() {
                        self.begin_force_sync(None);
                    }
                });
            }
        }
        status_line(ui, &self.users_status.clone());
    }

    /// C2: the caller's readiness row — declare/withdraw for the
    /// exercise side, an exemption note for judges, a not-seated note
    /// otherwise. Anything invalid is refused loudly by the server.
    fn readiness_ui(&mut self, ui: &mut egui::Ui) {
        match self.own_roster_row() {
            Some(p) if p.judge => {
                ui.weak(format!(
                    "{} · {} — judge side is exempt from readiness",
                    p.user_name, p.role_name
                ));
            }
            Some(p) if p.ready => {
                ui.horizontal(|ui| {
                    ui.label(format!("{} · {} — ready", p.user_name, p.role_name));
                    if ui.small_button("withdraw").clicked() {
                        self.set_own_readiness(false);
                    }
                });
            }
            Some(p) => {
                ui.horizontal(|ui| {
                    ui.label(format!("{} · {}", p.user_name, p.role_name));
                    if ui.button("declare ready").clicked() {
                        self.set_own_readiness(true);
                    }
                });
            }
            None => {
                ui.weak("you hold no seat in this session — join with the room key or ask the Game Master");
            }
        }
    }

    /// Setup flow step 4 (#79): review the gate, then advance
    /// planning → preparation. The Game Master decides planning is
    /// done — this step carries no gate of its own.
    fn setup_ready_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("4 · Review");
        let game = self
            .users_game
            .clone()
            .map(|(_, n)| n)
            .unwrap_or_else(|| "no session".to_string());
        ui.label(format!("session: {game}"));
        // The room key is the invitation. Minos issues it on entry to
        // preparation and sends it to every seated caller — the Game
        // Master reads it here and shares it out of band; personnel
        // type it into step 1. The client never invents a key and
        // never enumerates them (no route exists, by design).
        match self.minos_room_key.clone() {
            Some(key) => {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("room key:").strong());
                    ui.label(egui::RichText::new(&key).monospace().strong());
                    if ui.small_button("copy").clicked() {
                        ui.ctx().copy_text(key.clone());
                        self.users_status = "room key copied".to_string();
                    }
                });
                ui.weak("share this with your personnel — they enter it in step 1 to join the room.");
            }
            None => {
                // Planning: the key does not exist yet, and entering
                // preparation is what mints it. Say the consequence.
                let phase = self.users_game_state.as_deref().unwrap_or("—");
                ui.weak(match phase {
                    "planning" => {
                        "no room key in planning — advance to preparation, then read it here"
                    }
                    _ => "no room key on this session — ask the Game Master",
                });
            }
        }
        // Gapped counts are permission, not absence: say so instead of
        // drawing zeros as the force. The caller's own readiness below
        // still counts — it is participant-authorized either way.
        if self.roster_gap {
            ui.weak("staff counts unavailable for your account — your readiness below still counts.");
        } else {
            let (side, ready) = self.setup_gate_counts();
            ui.label(format!("{side} exercise-side seat(s), {ready} ready"));
        }
        if self.units_gap {
            ui.label(format!(
                "{} hull(s) commanded (full fleet is staff-only)",
                self.commanded_hulls.len()
            ));
        } else {
            ui.label(format!("{} piece(s) assigned", self.users_gunits.len()));
        }
        // C2: the gate's placement arithmetic, straight from Minos —
        // placed + unplaced is the size of the force.
        ui.label(format!(
            "Placements: {} placed · {} to go{}",
            self.users_placements.len(),
            self.placement_unplaced,
            if self.placement_ready { " · ready" } else { "" },
        ));
        ui.separator();
        // The checklist names every blocker; the button names its
        // consequence. The server still gates the advance itself.
        ui.strong("Checklist");
        let blockers = self.setup_checklist();
        if blockers.is_empty() {
            ui.label(egui::RichText::new("clear — ready to enter preparation").strong());
        } else {
            for b in &blockers {
                ui.label(format!("• {b}"));
            }
        }
        ui.separator();
        self.readiness_ui(ui);
        if let Some(note) = self.phase_note.clone() {
            warn_line(ui, note);
        }
        status_line(ui, &self.users_status.clone());
        // Only while planning — after the advance the backend refuses
        // it, and the step's own content is the readiness gate.
        if self.users_game_state.as_deref() == Some("planning")
            && ui.button("Enter preparation →").clicked()
        {
            self.setup_advance_prep();
        }
    }

    /// Setup gate: steps unlock in order — game, then seats, then
    /// pieces, then review. A locked step names its missing
    /// prerequisite; the server still refuses bad advances loudly.
    /// Why an island's picker is not usable yet, if it is not.
    ///
    /// Was the wizard's step gate. The zone shows every island at once, so
    /// there is no step to lock and nothing to navigate — but the *reason* is
    /// still the useful part, and dropping it would leave a disabled button
    /// with no explanation. Indexed by the same order the wizard used, so
    /// `2` is Fleet and `1` is Players.
    fn setup_step_lock(&self, step: usize) -> Option<String> {
        if self.users_game.is_none() {
            return if step == 0 {
                None
            } else {
                Some("hold a session in Essentials first".to_string())
            };
        }
        match step {
            0 | 1 => None,
            2 => {
                if self.users_roster.is_empty() {
                    Some("seat someone in Players first".to_string())
                } else {
                    None
                }
            }
            _ => {
                if self.users_gunits.is_empty() && self.commanded_hulls.is_empty() && self.force.is_empty() {
                    Some("assign pieces in Fleet first".to_string())
                } else {
                    None
                }
            }
        }
    }

    /// Readiness checklist: every blocker named, empty means clear.
    ///
    /// A FORMATTER over the server's answer. There is no longer a rule here,
    /// and that is the point: this function used to re-derive the execution
    /// gate from the roster, the placements and the pace, and it derived it
    /// wrong. Five conditions in the rule, four of them implemented, and a
    /// button disabled on a game the server would have accepted while the panel
    /// beside it reported "0 outstanding".
    ///
    /// So the words are the server's, the verdict is the server's, and the only
    /// thing added here is decoration: the NAMES behind the readiness count,
    /// when this client can read the roster to find them.
    fn setup_checklist(&self) -> Vec<String> {
        let Some(view) = self.readiness.as_ref() else {
            return self.unknown_gate_blockers();
        };
        let mut blockers = view.blockers.clone();
        if view.names_readiness() && !self.roster_gap {
            let waiting: Vec<String> = self
                .users_roster
                .iter()
                .filter(|p| !self.users_is_judge(p) && !p.ready)
                .map(|p| p.user_name.clone())
                .collect();
            if !waiting.is_empty()
                && let Some(b) = blockers.first_mut()
            {
                b.push_str(" Not yet: ");
                b.push_str(&waiting.join(", "));
            }
        }
        blockers
    }

    /// What to say when the gate has not been read.
    ///
    /// NEVER an empty list. Empty means "nothing is outstanding", which is the
    /// one answer that must never be invented — and this is the branch that
    /// runs before the first bundle lands and after a failed read. It refuses
    /// the advance instead, which costs a click the operator was going to make
    /// anyway and buys them a reason.
    ///
    /// It also does not try to work the gate out from what this client holds.
    /// That is the derivation this whole change exists to delete, and a
    /// degraded path is exactly where a deleted rule quietly comes back.
    fn unknown_gate_blockers(&self) -> Vec<String> {
        unknown_gate_blockers(self.readiness_gap)
    }

    /// Exercise setup panel (#79): Planning's whole UI in one place —
    /// game, players, fleet, ready. Visible while Planning lasts; the
    /// phase bar owns Persiapan onward.
    /// The top zone: a fixed full-width band, floating over the map.
    ///
    /// It is an `Area` rather than a `Panel` for the same reason the side
    /// zone is: a panel takes space from the `CentralPanel`, and the map
    /// owns every pixel the layout offers.
    ///
    /// Contents, left to right, in the order an operator reads them under
    /// time pressure: whether the link is up, how many of the exercise are
    /// in the room, the map controls, then the clock, settings, and sign-out
    /// at the right.
    ///
    /// The mode toggle is NOT here. Presentation versus Simulation is a
    /// startup decision, not an operating control, and Simulation does not
    /// announce itself: the console simply is the exercise. The toggle moved
    /// to `settings_modal`.
    fn top_zone(&mut self, ui: &mut egui::Ui) {
        let vp = ui.ctx().viewport_rect();
        egui::Area::new(egui::Id::new("topzone"))
            .fixed_pos(vp.left_top())
            .movable(false)
            .constrain(false)
            .interactable(true)
            .order(egui::Order::Middle)
            .show(ui.ctx(), |ui| {
                ui.set_max_width(vp.width());
                // Measured from the viewport, NOT from `ui.available_width()`.
                // An `Area` has no width until its content has been laid out,
                // so on the first frame — and on every frame, since nothing
                // ever fixes it — its inner `Ui` reports an unconstrained
                // width and every budget check passes. The viewport is the
                // only honest answer to "how much room does this band have".
                let avail = vp.width();
                ui.horizontal(|ui| {
                    self.link_ui(ui);
                    ui.add_space(tfg::tokens::INDENT);
                    self.headcount_ui(ui);
                    // "What now", in weak ink. The zone shows what IS; this
                    // is the one thing it cannot, and an operator who has
                    // lost their place is the operator this is for.
                    //
                    // It is the first thing to go on a narrow window, and
                    // it is gated rather than left to clip, because a left
                    // group that consumes the whole width pushes the
                    // right-hand group past the window edge and takes the
                    // sign-out button with it.
                    //
                    // The budget is a constant rather than a measurement,
                    // and the honest version would measure the right group's
                    // width instead. It is set with slack on purpose so that
                    // being wrong about it costs a hint rather than a
                    // button. Measured against this console's chrome the
                    // whole band needs about 900 points.
                    const HINT_BUDGET: f32 = 900.0;
                    if avail > HINT_BUDGET {
                        ui.add_space(tfg::tokens::INDENT);
                        ui.label(
                            egui::RichText::new(self.next_action_hint())
                                .weak()
                                .small(),
                        );
                    }
                    ui.add_space(tfg::tokens::INDENT);
                    self.map_controls(ui);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Sign out").clicked() {
                            self.sign_out("signed out from the top zone");
                        }
                        if ui.button("Settings").clicked() {
                            self.open_only(WhichModal::Settings);
                        }
                        self.clock_ui(ui);
                    });
                });
                // Where the band ends, published for the modal backdrop.
                //
                // The backdrop has to leave the top band clickable, and the
                // band's height is whatever its one row of controls measured —
                // so it is measured and published here rather than assumed
                // there. A constant would be wrong by however much the row is
                // tall, and the failure is invisible: too small and the band's
                // own buttons go inert again, too large and the backdrop eats
                // into the map.
                tfg::chrome::publish_top_band(ui.ctx(), ui.min_rect().bottom());
            });
    }

    /// The link's state: a dot and one word.
    ///
    /// A dot rather than an icon because the colour is the message and an
    /// icon would need a third glyph for a state the word already names.
    fn link_ui(&mut self, ui: &mut egui::Ui) {
        let state = self.live_state;
        let label = state.label();
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(label.len() as f32 * 8.0 + 24.0, 22.0),
            egui::Sense::click(),
        );
        ui.painter_at(rect).circle_filled(rect.left_center(), 4.0, state.ink());
        let clicked = resp.clicked();
        resp.on_hover_text(match self.live_state {
            LinkState::Idle => "no connection requested".to_string(),
            _ => self.live_status.clone(),
        });
        // A click on the link is the one verb that belongs on the face of
        // it: connect or stop, which is what an operator reaches for when
        // the word says the wrong thing.
        if clicked {
            self.toggle_live();
        }
        ui.painter_at(rect).text(
            rect.left_center() + egui::vec2(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            state.label(),
            egui::FontId::monospace(12.0),
            state.ink(),
        );
    }

    /// How many of the roster are actually in the room.
    ///
    /// Two numbers, because "online" is ambiguous and only one of them is
    /// knowable: seated is the roster's length, in-room is the count whose
    /// `joined_at` is set. Presentation has no roster at all, so it says
    /// nothing rather than showing a zero that reads as a failure.
    fn headcount_ui(&self, ui: &mut egui::Ui) {
        if self.users_game.is_none() {
            return;
        }
        let seated = self.users_roster.len();
        let in_room = self
            .users_roster
            .iter()
            .filter(|p| p.joined_at.is_some())
            .count();
        ui.monospace(format!("{in_room}/{seated} in room"));
    }

    /// Zoom, and nothing else.
    ///
    /// The band carries only what an operator reaches for during an
    /// exercise. The grid, the zone's visibility and which edge it holds
    /// are layout preferences that do not change mid-run, and a first pass
    /// put all of them here: at a 768px window the right-hand group then
    /// collided with this one. Preferences went to the settings modal
    /// instead, and the band got a width budget it can actually meet.
    fn map_controls(&mut self, ui: &mut egui::Ui) {
        if ui.small_button("\u{2212}").on_hover_text("zoom out").clicked() {
            self.zoom_by(-1.0);
        }
        ui.monospace(format!("z{:.0}", self.zoom))
            .on_hover_text("zoom level");
        if ui.small_button("+").on_hover_text("zoom in").clicked() {
            self.zoom_by(1.0);
        }
    }

    /// The clock, on one line and inside a width budget.
    ///
    /// The old toolbar spent three rows on this and spelled out "UTC",
    /// "Game", "pace" and "local". A band that is a third of a short
    /// window tall for a clock is not a frame, and the words carried no
    /// meaning a fixed-width timestamp does not. The two-letter source
    /// stays because it answers a real question: whether the reading came
    /// from the exercise or from this machine.
    fn clock_ui(&self, ui: &mut egui::Ui) {
        let elapsed = self.game_elapsed_secs.unwrap_or(0);
        let source = if self.live_state.is_up() && self.game_state().is_running() {
            "ex"
        } else {
            "lo"
        };
        let mut line = format!(
            "{}  G+{:02}:{:02}  {source} {:.0}\u{d7}",
            short_timestamp(self.real_ts.as_deref()),
            elapsed / 60,
            elapsed % 60,
            self.game_ratio
        );
        if self.game_paused {
            line.push_str("  PAUSED");
        }
        let mut text = egui::RichText::new(line).monospace();
        if self.game_paused {
            text = text.color(tfg::tokens::ALERT_YELLOW);
        }
        ui.label(text);
    }

    /// The settings modal. It exists for the things an operator sets once
    /// rather than during an exercise, which after the overhaul is a short
    /// list: the mode, the text scale, and the reduced-motion switch.
    fn settings_modal(&mut self, ui: &egui::Ui) {
        let mut open = self.settings_open;
        let mut closed = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.set_width(340.0);
                // ONE toggle, not a two-way mode switch. The console runs
                // exercises; Presentation is the watch-only view on top of
                // one, and a peer "Simulation" button would be the app
                // naming its own function in a menu. Offered as a view
                // rather than a mode, and the label says which one you are
                // in rather than asking which you want.
                ui.label("View");
                let mut watching = self.app_mode == AppMode::Presentation;
                if ui
                    .toggle_value(&mut watching, "Watch only")
                    .on_hover_text("read a session without running it")
                    .changed()
                {
                    self.set_app_mode(if watching {
                        AppMode::Presentation
                    } else {
                        AppMode::Simulation
                    });
                }
                ui.weak(if watching {
                    "watching — the map follows the session and nothing here moves it."
                } else {
                    "running — this machine directs the exercise."
                });
                ui.separator();
                ui.label("Layout");
                ui.checkbox(&mut self.show_grid, "Geographic grid")
                    .on_hover_text("a reference grid over the map");
                ui.toggle_value(&mut self.show_side_zone, "Side zone")
                    .on_hover_text("the column of islands down one edge");
                if self.show_side_zone {
                    ui.horizontal(|ui| {
                        ui.label("Docked");
                        for (label, dock) in [
                            ("left", tfg::chrome::Dock::Left),
                            ("right", tfg::chrome::Dock::Right),
                        ] {
                            if ui
                                .selectable_label(self.side_dock == dock, label)
                                .clicked()
                            {
                                self.side_dock = dock;
                            }
                        }
                    });
                }
                ui.separator();
                ui.label("Text size");
                ui.horizontal(|ui| {
                    for (label, scale) in
                        [("Smaller", 0.875), ("Default", 1.0), ("Larger", 1.15)]
                    {
                        if ui
                            .selectable_label(
                                (self.text_scale - scale).abs() < 0.001,
                                label,
                            )
                            .clicked()
                        {
                            self.text_scale = scale;
                        }
                    }
                });
                ui.separator();
                ui.label("Motion");
                ui.checkbox(&mut self.reduced_motion, "Reduce motion")
                    .on_hover_text("collapse every transition to its target");
                ui.separator();
                ui.weak("Nothing here is remembered across launches: the client has no settings store.");
                if ui.button("Close").clicked() {
                    closed = true;
                }
            });
        self.settings_open = open && !closed;
    }

    // -- the Scenario Composer ------------------------------------------------
    //
    // Authors ONE game's book. It is not a picker over a catalog: a game has
    // exactly one book, the book IS its ordered scenarios, and there is no
    // cross-game template. So the modal's left column is this game's
    // scenarios and the right column is the open scenario's steps.
    //
    // Every write is off-thread and answers with the whole scenario, so the
    // list is redrawn from the server's answer rather than from a local guess
    // at what the order now is.

    fn open_composer(&mut self, scenario_id: Option<i64>) {
        // A remembered open scenario is worth keeping across a close, so an
        // explicit argument wins and otherwise the previous one is reused.
        // With none, the first in the book is the sensible landing: the book
        // is ordered and an author reopening means to keep working.
        if let Some(id) = scenario_id {
            self.composer_scenario = Some(id);
        } else if self.composer_scenario.is_none() {
            self.composer_scenario = self.scenarios.first().map(|s| s.id);
        }
        self.open_only(WhichModal::Composer);
        self.composer_draft = ComposerDraft::default();
        self.composer_draft_error = None;
        self.load_scenarios();
    }

    /// Re-read the book. The answer replaces the whole list.
    fn load_scenarios(&mut self) {
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("book refused: {e}");
                return;
            }
        };
        self.scenarios_op = Some(spawn_rest("scenarios", move || {
            master
                .game_scenarios(&tok, gid)
                .map_err(|e| e.to_string())
                .map(Book::List)
        }));
    }

    fn add_scenario_step(&mut self) {
        let Some(sid) = self.composer_scenario else {
            return;
        };
        let Some((gid, _)) = self.users_game.clone() else {
            return;
        };
        if let Some(problem) = self.composer_draft.problem() {
            self.composer_draft_error = Some(problem);
            return;
        }
        let content = std::mem::take(&mut self.composer_draft.content);
        let window = self.composer_draft.window().map(|(a, b)| (a.to_string(), b.to_string()));
        self.composer_draft = ComposerDraft::default();
        self.composer_draft_error = None;
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("step refused: {e}");
                return;
            }
        };
        let window = window.map(|(a, b)| (a, b));
        self.scenarios_op = Some(spawn_rest("scenario-step", move || {
            master
                .add_scenario_step(
                    &tok,
                    gid,
                    sid,
                    &content,
                    window.as_ref().map(|(a, b)| (a.as_str(), b.as_str())),
                )
                .map_err(|e| e.to_string())
                .map(Book::One)
        }));
    }

    fn delete_scenario_step(&mut self, step_id: i64) {
        let Some(sid) = self.composer_scenario else {
            return;
        };
        let Some((gid, _)) = self.users_game.clone() else {
            return;
        };
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("delete refused: {e}");
                return;
            }
        };
        self.scenarios_op = Some(spawn_rest("scenario-step-del", move || {
            master
                .delete_scenario_step(&tok, gid, sid, step_id)
                .map_err(|e| e.to_string())
                .map(Book::One)
        }));
    }

    fn create_scenario(&mut self, title: &str) {
        let Some((gid, _)) = self.users_game.clone() else {
            return;
        };
        let title = title.trim();
        if title.is_empty() {
            // The backend requires a title, and it is the one field on this
            // form that cannot be empty.
            self.composer_draft_error = Some("a scenario needs a title".into());
            return;
        }
        let title = title.to_string();
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("scenario refused: {e}");
                return;
            }
        };
        self.scenarios_op = Some(spawn_rest("scenario-create", move || {
            master
                .create_game_scenario(&tok, gid, &title, "")
                .map_err(|e| e.to_string())
                .map(Book::One)
        }));
    }

    // -- the Fleet Picker ----------------------------------------------------
    //
    // The Miller columns re-homed onto `chrome::modal`. Two things about it
    // are not the composer's problem and are the reason it is its own unit:
    //
    // 1. A drag starts on a row INSIDE the panel and ends on the map BEHIND
    //    it. The map is full-bleed under the floating zones, so its rect
    //    contains the panel, and a naive release would drop a hull at
    //    whatever point lies behind the form.
    // 2. The drag ghost is painted by the map, which is UNDER the backdrop.
    //    So while a drag is in flight the backdrop clears instead of dimming
    //    — see `chrome::Backdrop`.

    /// The Fleet Picker modal.
    ///
    /// Open exactly one modal, closing whichever other was up.
    ///
    /// A modal is a modal: two at once is not two forms, it is one form on top
    /// of another with no way to say which. And it happened here. Settings is
    /// opened from the top band, which the backdrop deliberately leaves live so
    /// a modal is never a dead end — so Settings could be opened while the Fleet
    /// picker was up, and it was drawn UNDERNEATH it, because `Order::Foreground`
    /// puts the later-drawn modal on top and the picker draws later. The operator
    /// clicked Settings, watched nothing appear, and concluded the click had been
    /// swallowed. Nothing was swallowed; the Settings panel was there the whole
    /// time, under a form of the same size.
    ///
    /// Closing the others is the honest reading of one click on "Settings". The
    /// alternative is stacking by open order, which needs a z-order egui's
    /// `Order` cannot express and a modal stack nothing else in this app keeps.
    fn open_only(&mut self, which: WhichModal) {
        self.settings_open = which == WhichModal::Settings;
        self.composer_visible = which == WhichModal::Composer;
        self.fleet_picker_open = which == WhichModal::Fleet;
        self.player_picker_open = which == WhichModal::Player;
        // A drag does not survive the modal that armed it: the release would
        // land on the map with no panel to have started it, which is a command
        // nobody asked for.
        self.unit_drag = None;
    }

    /// Not a picker over the register alone: it is also where placement is
    /// armed and where the caller's own hulls are lifted, because all three
    /// act on the same `fleet_pick` and splitting them puts the state in one
    /// modal and its effect on the map in another.
    fn fleet_picker_modal(&mut self, ui: &egui::Ui) {
        let mut spec = tfg::chrome::Modal::new(
            egui::Id::new("fleet-picker"),
            "Fleet picker",
            fleet_picker_size(ui.ctx().viewport_rect().height()),
        );
        // A drag in flight means the pointer is about to be over the map, and the
        // ghost it is aiming with is painted there — so the whole panel steps
        // aside, not just the dim. It is the thing covering the target, and a
        // release over it is refused by `drop_lands_on_map`, so leaving it up
        // would take away exactly the ground the gesture is for.
        let dragging = self.unit_drag.is_some();
        if dragging {
            spec = spec.step_aside();
        }
        let panel = spec.rect_in(ui.ctx().viewport_rect());
        let kept = tfg::chrome::modal(ui.ctx(), &spec, |ui| {
            let picked = self.unit_picker_ui(ui);
            for (hull, name) in picked {
                self.setup_assign_unit(hull, &name);
            }
        });
        // Published even when the modal is closing: the drop handler runs
        // later in the same frame, on the release that closed it, and on that
        // frame the panel is still where the pointer left it.
        //
        // NOT published while the panel is stepped aside. There is nothing on
        // screen to drop "over the form" onto, so refusing a drop there would be
        // refusing a drop on open water the operator can plainly see.
        if !dragging {
            self.modal_panel_rects.push(panel);
        }
        if !kept {
            self.fleet_picker_open = false;
            self.clear_fleet_pick();
            self.unit_drag = None;
        }
    }

    // -- the Player Picker ----------------------------------------------------
    //
    // Three things in one form because they are one decision: WHO from the
    // app directory, AS WHAT game role, and — for some roles — WHICH FLEET
    // they command. Splitting them across three surfaces is how a role ends
    // up seated with no command and nobody notices until the exercise starts.
    //
    // It also owns the drag-to-map that assigns a seated person a piece: the
    // press starts on a directory row and the release is on a unit marker or
    // a group flag. Same map-under-a-modal hazard as the Fleet picker, and
    // the same two answers — clear the backdrop while the drag is in flight,
    // and refuse a release that lands on a panel.

    /// Whether a person-drag is in flight, for the backdrop decision.
    fn player_drag_in_flight(&self) -> bool {
        self.udrag.is_some() || self.upress.is_some()
    }

    /// The Player Picker modal.
    fn player_picker_modal(&mut self, ui: &egui::Ui) {
        let mut spec = tfg::chrome::Modal::new(
            egui::Id::new("player-picker"),
            "Player picker",
            egui::vec2(940.0, 600.0),
        );
        // The drag ghost is painted by the map, which is under the panel, and the
        // release lands on a marker or a group flag the panel is covering. Same
        // reasoning as the Fleet picker, so the same answer: the whole surface
        // steps aside for the duration of the gesture.
        let dragging = self.player_drag_in_flight();
        if dragging {
            spec = spec.step_aside();
        }
        let panel = spec.rect_in(ui.ctx().viewport_rect());
        // Indexed rather than bound: `&mut cols[0]` and `&mut cols[1]` are
        // two live mutable borrows of one Vec, which the borrow checker
        // refuses even though they are different elements.
        let kept = tfg::chrome::modal(ui.ctx(), &spec, |ui| {
            ui.columns(2, |cols| {
                self.users_directory_ui(&mut cols[0]);
                self.roster_editor_ui(&mut cols[1]);
            });
            status_line(ui, &self.users_status.clone());
        });
        if !dragging {
            self.modal_panel_rects.push(panel);
        }
        if !kept {
            self.player_picker_open = false;
            // A person-drag does not survive the modal that started it: the
            // release would land on the map with no panel to have started it,
            // which is a command nobody asked for.
            self.udrag = None;
            self.upress = None;
        }
    }

    /// The composer modal.
    fn composer_modal(&mut self, ui: &egui::Ui) {
        let open_sid = self.composer_scenario;
        let spec = tfg::chrome::Modal::new(
            egui::Id::new("composer"),
            "Scenario composer",
            egui::vec2(880.0, 560.0),
        );
        let kept = tfg::chrome::modal(ui.ctx(), &spec, |ui| self.composer_body(ui, open_sid));
        if !kept {
            // `composer_visible` is what the draw loop tests, so clearing
            // everything else without clearing it left the modal on screen
            // with an empty book: the ✕ deselected the scenario and nothing
            // else. Found by driving, not by reading — the flag is written in
            // `open_composer` and by `apply_login`, and this was the only
            // other place that had to write it.
            self.composer_visible = false;
            self.composer_scenario = None;
            self.composer_draft = ComposerDraft::default();
            self.composer_draft_error = None;
        }
    }

    /// The composer body, in two columns.
    ///
    /// Written as ONE `columns` closure with `cols[0]`/`cols[1]` indexed at
    /// each use rather than bound once. Binding both up front is two live
    /// mutable borrows of the same `Vec<Ui>`, which the borrow checker
    /// refuses, and splitting it into two closures means calling `columns`
    /// twice, which draws two 50/50 splits instead of one pair of columns.
    fn composer_body(&mut self, ui: &mut egui::Ui, open_sid: Option<i64>) {
        let scenarios = self.scenarios.clone();
        ui.columns(2, |cols| {
            cols[0].label("This game's book");
            cols[0].separator();
            if scenarios.is_empty() {
                cols[0].weak("No scenarios yet. One game has one book.");
            }
            for sc in scenarios.iter() {
                let selected = Some(sc.id) == open_sid;
                if cols[0]
                    .selectable_label(
                        selected,
                        format!("{}  -  {} step(s)", sc.title, sc.steps.len()),
                    )
                    .clicked()
                    && !selected
                {
                    self.composer_scenario = Some(sc.id);
                }
            }
            cols[0].add_space(8.0);
            let mut new_title = String::new();
            ui_input(&mut cols[0], &mut new_title, "New scenario title");
            if cols[0].button("Add scenario").clicked() {
                self.create_scenario(&new_title);
            }

            cols[1].separator();
            match open_sid.and_then(|id| scenarios.iter().find(|sc| sc.id == id).cloned()) {
                None => {
                    cols[1].weak("Pick a scenario, or add one.");
                }
                Some(open) => {
                    let steps = open.steps.clone();
                    cols[1].label(format!("Steps - {}", open.title));
                    cols[1].weak("Each step is one thing that must happen, in order.");
                    if steps.is_empty() {
                        cols[1].weak("No steps yet. A plan may be authored while it is written.");
                    }
                    for step in steps.iter() {
                        cols[1].horizontal(|row| {
                            row.monospace(format!("{:>2}", step.position + 1));
                            // A step with no text reads as weak ink, because
                            // "(not written yet)" is a placeholder the author
                            // supplied rather than something anyone decided.
                            let text = if step.content.is_empty() {
                                "(not written yet)".to_string()
                            } else {
                                step.content.clone()
                            };
                            let label = egui::RichText::new(text);
                            row.label(if step.content.is_empty() {
                                label.weak()
                            } else {
                                label
                            });
                            row.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |tail| {
                                    if tail.small_button("remove").clicked() {
                                        self.delete_scenario_step(step.id);
                                    }
                                    tail.monospace(step.window_label());
                                },
                            );
                        });
                    }
                    cols[1].separator();
                    cols[1].label("New step");
                    ui_text(
                        &mut cols[1],
                        &mut self.composer_draft.content,
                        "What must happen",
                    );
                    cols[1].horizontal(|row| {
                        let mut draft = self.composer_draft.clone();
                        row.label("window");
                        let a = row.add(
                            egui::TextEdit::singleline(&mut draft.start_hour)
                                .desired_width(52.0)
                                .hint_text("1000"),
                        );
                        let b = row.add(
                            egui::TextEdit::singleline(&mut draft.end_hour)
                                .desired_width(52.0)
                                .hint_text("1030"),
                        );
                        if a.changed() || b.changed() {
                            self.composer_draft = draft;
                        }
                        row.weak("HHMM, or leave both empty");
                    });
                    if let Some(problem) = self.composer_draft.problem() {
                        warn_line(&mut cols[1], problem);
                    }
                    if let Some(err) = self.composer_draft_error.clone() {
                        warn_line(&mut cols[1], err);
                    }
                    if cols[1].button("Add step").clicked() {
                        self.add_scenario_step();
                    }
                    cols[1].separator();
                    status_line(&mut cols[1], &self.users_status.clone());
                }
            }
        });
    }
    /// The Orders surface, as the Execution column shows it.
    ///
    /// Extracted verbatim from the free-floating Orders island rather than
    /// rewritten: the helm surface is a hundred and eighty lines of
    /// jurisdiction and authority checks, and re-deriving any of it while
    /// moving it is how a refactor becomes a rewrite.
    fn orders_body(&mut self, ui: &mut egui::Ui) {
// Direct HelmOrder surface: one selected unit at a time,
// with MinOS authority in Live mode and a labelled local
// sandbox projection in Simulation mode.
ui.heading("Helm");
// Group selection is inspectable here, but the first slice
// deliberately avoids a multi-unit helm fan-out.
if let Some(Selection::Group(_gid)) = self.selection.clone() {
    ui.collapsing("Legacy waypoint navigation (compatibility)", |ui| {
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
                        warn_line(ui, "waypoint on land — pick water".to_string());
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
    });
}
if let Some(Selection::Ship(id)) = self.selection.clone() {
    // Scoped desktop (slice iv): command inside jurisdiction,
    // view everything.
    let allowed = self.action_allows(&id);
    if !allowed {
        ui.label("Outside your jurisdiction — view only.");
    } else if self.minos_order_target(&id).is_some() {
        self.minos_order_ui(ui, &id);
    } else if self.controlled.contains(&id) {
        self.local_helm_order_ui(ui, &id);
    } else {
        // Take-control with a class selector (grill #18): the
        // chosen class's stats drive the unit from then on.
        // Options name their authority (H10): Minos rows
        // carry the spec version, bundled rows the asset.
        let ships = self.catalog.ship_classes();
        let names: Vec<String> = ships
            .iter()
            .map(|c| {
                if c.version > 0 {
                    format!("{} · {} v{}", c.name, Catalog::class_source(c), c.version)
                } else {
                    format!("{} · {}", c.name, Catalog::class_source(c))
                }
            })
            .collect();
        egui::ComboBox::from_label("")
            .selected_text(
                names.get(self.selected_class).map(|s| s.as_str()).unwrap_or("—"),
            )
            .show_ui(ui, |ui| {
                for (i, name) in names.iter().enumerate() {
                    ui.selectable_value(&mut self.selected_class, i, name.as_str());
                }
            });
        if allowed && ui.small_button("take control").clicked() {
            // C3: Minos drives its pieces in execution — a
            // local takeover would be the ghost this ticket
            // removes. Order them through Minos instead.
            if self.users_game_state.as_deref() == Some("execution")
                && id.parse::<i64>().is_ok_and(|uid| {
                    self.users_gunits.iter().any(|g| g.unit_id == uid)
                })
            {
                let msg =
                    "take control refused: the exercise is already driving this unit"
                        .to_string();
                self.feed(msg.clone());
                self.users_status = msg;
            } else if let Some(s) = self.registry.ship(&id) {
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
    }


    /// The side zone: a column of islands whose contents are a function of
    /// [`GameState`], floating over the map.
    ///
    /// It replaces both `Panel::left` docks and the four-step setup wizard.
    /// Three things changed and each is deliberate:
    ///
    /// - **It is an `Area`, not a `Panel`.** A panel takes space from the
    ///   `CentralPanel`, which resizes the map the operator is reading. The
    ///   zone floats instead, and `visible_center` pays for that by shifting
    ///   every camera goal away from the zone.
    /// - **All the Planning islands are live at once**, where the wizard made
    ///   them one-at-a-time behind a step lock. A column of four short
    ///   islands is more scannable than a wizard, and it removes the Back /
    ///   Next pair whose only job was to change which panel was on screen.
    /// - **Ownership is resolved once**, before any island paints, so exactly
    ///   one rim lights (ADR-0016). Letting each island ask would light every
    ///   island the pointer was inside, which for a column is at most one —
    ///   but "at most one by accident" is not the same guarantee as "one by
    ///   construction".
    // zone_island_heights_are_guesses
    //
    // Every island height in the column is a hand-picked constant, and the
    // column STACKS by those heights before anything is drawn. So they cannot
    // be measured from the content without running each body twice, and
    // running a body twice double-fires its writes.
    //
    // This is not hypothetical. The Operator island was 112pt, its content
    // needed more, and the overflow painted over the map and half-occluded
    // under the next island's title band — a render showed it, and no
    // geometry test could, because every rect involved was correct.
    //
    // `island_owned` now CLIPS its body, so a wrong height is quiet rather
    // than corrupting: that is a mitigation, not a fix. The fix is to measure
    // — which means either a non-interactive measure pass (the bodies must
    // stop writing on a dry run) or heights derived from an explicit content
    // model rather than from layout. Until then, every new island height is
    // an unverified guess and the first render of a new island should be
    // treated as a measurement, not a confirmation.
    fn side_zone(&mut self, ui: &mut egui::Ui) {
        use tfg::chrome::{Island, island_owned, owning_island, zone_island_origins};
        let state = self.game_state();
        let w = tfg::chrome::zone_width();

        // The user island is first in every state, because identity is the
        // one thing that does not change with the exercise.
        let mut entries: Vec<(Island, bool)> = vec![(
            // 140pt, measured against the real thing: the
            // identity row (avatar 40 + pad), the email line, then the button.
            // Found by rendering — 104 and 128 both clipped the button, and
            // the previous 112 was too short for the whole block. See the
            // note on `zone_island_heights_are_guesses`.
            Island::new(egui::Id::new("z.user"), "Operator", egui::vec2(w, 140.0))
                .with_trailing(&self.app_role_tag()),
            true,
        )];

        match state {
            GameState::NoSession => entries.push((
                // 168 is the create-only form. The session picker adds a row and a
                // separator on top of it, and a declared height is a FLOOR the
                // measured height cannot exceed — so the taller case has to be
                // declared taller or the picker is drawn into a clipped box.
                // 272 is what the picker case measures, rounded up: at 212 the
                // clip report said "212pt given, content needs more".
                Island::new(
                    egui::Id::new("z.start"),
                    "New session",
                    egui::vec2(w, if self.session_picker_visible() { 272.0 } else { 168.0 }),
                ),
                true,
            )),
            GameState::Planning => {
                entries.push((
                    Island::new(egui::Id::new("z.essentials"), "Essentials", egui::vec2(w, 380.0))
                        .with_trailing("STEP 1"),
                    true,
                ));
                entries.push((
                    Island::new(egui::Id::new("z.control"), "Control", egui::vec2(w, 150.0)),
                    true,
                ));
                entries.push((
                    Island::new(egui::Id::new("z.fleet"), "Fleet", egui::vec2(w, 320.0))
                        .with_trailing(&format!("{} PIECES", self.force.len())),
                    true,
                ));
                entries.push((
                    Island::new(egui::Id::new("z.players"), "Players", egui::vec2(w, 320.0))
                        .with_trailing(&format!("{} SEATED", self.users_roster.len())),
                    true,
                ));
            }
            GameState::Preparation => entries.push((
                Island::new(egui::Id::new("z.ready"), "Readiness", egui::vec2(w, 360.0))
                    .with_trailing("STEP 2"),
                true,
            )),
            // Execution is the state the console spends its life in, so the
            // column is a working surface rather than a summary: what time it
            // is, what you can order, who is actually here, and what just
            // happened. Four islands in that order — the clock because it is
            // read most, the orders because they are the verb, and the two
            // that answer "is this going right" underneath both.
            GameState::Execution => {
                entries.push((
                    Island::new(egui::Id::new("z.clock"), "Exercise", egui::vec2(w, 200.0))
                        .with_trailing(&format!("G+{:02}:{:02}",
                            self.game_elapsed_secs.unwrap_or(0) / 60,
                            self.game_elapsed_secs.unwrap_or(0) % 60)),
                    true,
                ));
                entries.push((
                    Island::new(egui::Id::new("z.orders"), "Orders", egui::vec2(w, 420.0))
                        .with_trailing(&self.selection_label()),
                    true,
                ));
                entries.push((
                    Island::new(egui::Id::new("z.crew"), "Crew", egui::vec2(w, 240.0))
                        .with_trailing(&format!("{} IN ROOM", self.crew_in_room())),
                    true,
                ));
                entries.push((
                    Island::new(egui::Id::new("z.log"), "Log", egui::vec2(w, 260.0)),
                    true,
                ));
            }
            // Closure is read-only, so the column is what you debrief with:
            // the assessment workspace, and the log it is read out of.
            // Deliberately no Orders — nothing is ordered after the fact, and
            // a control that would be refused is a lie about the state.
            GameState::Closure => {
                entries.push((
                    Island::new(egui::Id::new("z.assessment"), "Assessment", egui::vec2(w, 420.0)),
                    true,
                ));
                entries.push((
                    Island::new(egui::Id::new("z.log"), "Log", egui::vec2(w, 260.0)),
                    true,
                ));
            }
        }

        // The declared height IS the height. This column used to grow every
        // island to whatever its content measured, one frame behind, clamped
        // to the window so a long body could not push the column past the
        // screen. That was a workaround for bodies that could not scroll, and
        // once they scroll it is the other half of the bug: Essentials measured
        // ~760pt of content, so it grew to the 624pt ceiling against a 640pt
        // window, and an island taller than the screen cannot be scrolled to
        // by any means. Fixed footprint plus a scrolling body is the whole
        // answer, and it is what DESIGN.md specifies.
        //
        // So the stack is arithmetic on constants now, computed before
        // anything is drawn, and every island is in its place on the first
        // paint rather than growing into it over a second.
        let vp = ui.ctx().viewport_rect();
        let dock = self.side_dock;
        let origins = zone_island_origins(dock, vp, &entries);
        let band = tfg::chrome::zone_band(dock, vp);

        // `zone_island_origins` skips closed islands, so pairing entries with
        // origins by index pairs them wrongly the moment one is closed. Build
        // the pairs once and let everything downstream read the pairs.
        let stacked: Vec<(Island, egui::Pos2)> = entries
            .into_iter()
            .filter(|(_, open)| *open)
            .zip(origins)
            .map(|((spec, _), pos)| (spec, pos))
            .collect();

        // The column is taller than the window in every state that has four
        // islands — Planning declares 1390pt against a 584pt band at the app's
        // own default size — so the zone scrolls rather than dropping the tail
        // off the bottom, which is how Control and Fleet stopped rendering at
        // all. Both scrolls are real now: this one for the column, a ScrollArea
        // per island for the bodies.
        let content_h: f32 = stacked.iter().map(|(spec, _)| spec.size.y).sum::<f32>()
            + tfg::tokens::ZONE_ISLAND_GAP * stacked.len() as f32;
        let overflow = (content_h - band.height()).max(0.0);

        // Only over the column: the map has its own wheel handling and a scroll
        // that fired everywhere would fight it.
        //
        // Subtracted, not added. egui reports a POSITIVE Y for content moving
        // DOWN, so scrolling down the column arrives here as a negative delta —
        // and `+=` drove the offset below zero where the clamp pinned it, which
        // looks exactly like a wheel that does nothing.
        //
        // CHAINED, not exclusive. Every island body scrolls inside its own rect
        // (chrome::island_owned), and a body that can still move in the
        // direction the wheel is turning takes the gesture; the column takes it
        // only when no live body under the pointer can. Without the chain the
        // two scroll at once and one gesture moves two things, which reads as
        // a control that jumped.
        //
        // The measurement is last frame's, published by the island that drew
        // it. One frame of lag on a wheel gesture is not perceptible, and it
        // is the same trade the fitted height already makes.
        let hover = ui.ctx().input(|i| i.pointer.hover_pos());
        let wheel = ui.ctx().input(|i| i.smooth_scroll_delta.y);
        // The scroll as it stands, which is what is ON SCREEN this frame: the
        // gesture below only decides who takes the wheel, and the offset is
        // about to move under it either way.
        let scroll = self.zone_scroll;
        let body_owns_wheel = wheel != 0.0
            && hover.is_some_and(|h| {
                stacked.iter().any(|(spec, pos)| {
                    if !tfg::chrome::island_on_band(*pos, scroll, band, spec.size.y) {
                        return false;
                    }
                    let body = spec.content_rect(spec.rect_at(egui::pos2(pos.x, pos.y - scroll)));
                    if !body.contains(h) {
                        return false;
                    }
                    tfg::chrome::island_body_fit(ui.ctx(), spec.id).is_some_and(|fit| {
                        // Positive wheel Y is content moving down, which is the
                        // same direction the column subtracts towards.
                        if wheel > 0.0 { fit.can_scroll_up() } else { fit.can_scroll_down() }
                    })
                })
            });
        let in_band = hover.is_some_and(|h| band.contains(h));
        if in_band && !body_owns_wheel {
            self.zone_scroll -= wheel;
        }
        // The island whose body claimed the gesture, for the debug line. The
        // check needs it by NAME: "did a body have room" is not the same
        // question as "did a body under the POINTER have room", and only the
        // second one is the invariant.
        let wheel_owner = stacked
            .iter()
            .find(|(spec, pos)| {
                hover.is_some_and(|h| {
                    tfg::chrome::island_on_band(*pos, scroll, band, spec.size.y)
                        && spec
                            .content_rect(spec.rect_at(egui::pos2(pos.x, pos.y - scroll)))
                            .contains(h)
                })
            })
            .map(|(spec, _)| spec.id);
        // Re-clamped every frame rather than only while hovering, because the
        // overflow changes with the state: a scroll set in Planning must not
        // strand the column past its own end in Execution.
        self.zone_scroll = self.zone_scroll.clamp(0.0, overflow);
        let scroll = self.zone_scroll;

        if std::env::var("TFG_ZONE_DEBUG").is_ok() {
            eprintln!(
                "zone: band={:?} stacked={} content_h={:.0} overflow={:.0} scroll={:.0} dock={:?} hover={:?} in_band={} delta={:.1} wheel_owner={:?}",
                band,
                stacked.len(),
                content_h,
                overflow,
                scroll,
                dock,
                hover,
                in_band,
                ui.ctx().input(|i| i.smooth_scroll_delta.y),
                wheel_owner.map(|id| format!("{id:?}"))
            );
            for (spec, pos) in &stacked {
                // The body measurement on the SAME line as the column offset, so
                // one line answers "who took this gesture" without the reader
                // having to correlate two. `verify-scroll.sh` asserts on the
                // pair: the offset moved AND no body under the pointer had room
                // is the whole invariant, and it is only checkable if both
                // numbers are adjacent.
                let fit = tfg::chrome::island_body_fit(ui.ctx(), spec.id);
                // Width as well as height, because `verify-scroll.sh` crops to
                // an island to find its buttons and was guessing the width from
                // a constant while reading the height from here. Half the
                // geometry came from one source and half from another, which is
                // how a crop became `380x+212+320`.
                eprintln!(
                    "  {:?} pos=({:.0},{:.0}) size={:.0}x{:.0} body={:?}",
                    spec.id, pos.x, pos.y, spec.size.x, spec.size.y, fit
                );
            }
        }

        // Ownership is a question about what is ON SCREEN, so the rects are
        // built from the scrolled positions. An island scrolled out of the
        // band is dropped here as well as at draw time, or it would own input
        // it is not showing.
        let live: Vec<(Island, egui::Pos2)> = stacked
            .into_iter()
            .filter(|(spec, pos)| {
                tfg::chrome::island_on_band(*pos, scroll, band, spec.size.y)
            })
            .collect();
        let rects: Vec<egui::Rect> = live
            .iter()
            .map(|(spec, pos)| spec.rect_at(egui::pos2(pos.x, pos.y - scroll)))
            .collect();
        let owner = owning_island(&rects, hover);

        for (i, (spec, mut pos)) in live.into_iter().enumerate() {
            let mut open = true;
            let owns = owner == Some(i);
            let id = spec.id;
            let title = spec.title.clone();
            island_owned(ui.ctx(), &spec, &mut pos, scroll, band, &mut open, owns, |ui| match id {
                x if x == egui::Id::new("z.user") => self.zone_user_body(ui),
                x if x == egui::Id::new("z.start") => self.zone_start_body(ui),
                x if x == egui::Id::new("z.essentials") => self.zone_essentials_body(ui),
                x if x == egui::Id::new("z.control") => self.zone_control_body(ui),
                x if x == egui::Id::new("z.fleet") => self.zone_fleet_body(ui),
                x if x == egui::Id::new("z.players") => self.zone_players_body(ui),
                x if x == egui::Id::new("z.ready") => self.zone_ready_body(ui),
                x if x == egui::Id::new("z.clock") => self.zone_exercise_body(ui),
                x if x == egui::Id::new("z.orders") => self.zone_orders_body(ui),
                x if x == egui::Id::new("z.crew") => self.zone_crew_body(ui),
                x if x == egui::Id::new("z.log") => self.zone_log_body(ui),
                x if x == egui::Id::new("z.assessment") => self.zone_assessment_body(ui),
                _ => {
                    ui.weak(format!("{title} \u{2014} no body"));
                }
            });

        }
    }

    /// Whether the side zone already carries the surface the free-floating
    /// islands duplicate.
    ///
    /// The column and the floating islands are not two views of one thing —
    /// they are two copies, and drawing both is the operator editing one and
    /// looking at the other. So it is per SURFACE and not just "the zone is
    /// showing": Execution carries Orders, Crew and Log, so those three stop
    /// floating there, while Planning carries none of them and they stay.
    ///
    /// Presentation has no column at all, so `show_side_zone` alone does not
    /// answer it — hence the state check. Without that, turning the zone on
    /// in Presentation would suppress islands the column is not drawing.
    fn zone_carries(&self, surface: &str) -> bool {
        if !self.show_side_zone {
            return false;
        }
        let in_column = match self.game_state() {
            GameState::Execution => matches!(surface, "orders" | "crew" | "log"),
            GameState::Closure => matches!(surface, "log"),
            _ => false,
        };
        in_column
    }

    /// Whether the side zone is occupying the left edge, where the legacy
    /// free-floating islands are parked.
    ///
    /// Those islands still exist for Presentation, which has no zone, so they
    /// cannot simply be deleted. Rendering them under the zone is worse than
    /// either: two panels on top of each other, and the operator sees the
    /// wrong one.
    fn zone_owns_left_edge(&self) -> bool {
        self.show_side_zone && self.side_dock == tfg::chrome::Dock::Left
    }

    /// Whether the account holds any application role at all.
    ///
    /// `can_manage_sessions` gates the session browser and every lifecycle
    /// write on this. The original test was `!app_role_ids.is_empty()` on a
    /// `Vec<i64>` the parser filled from the role array, so it meant "the
    /// probe ran and the account has a role" — which is what asking the
    /// identity for its roles means now, without carrying a second copy of
    /// the answer.
    fn has_app_role(&self) -> bool {
        self.auth_identity
            .as_ref()
            .is_some_and(|i| !i.app_roles.is_empty())
    }

    /// The app role, as the small mono tag the user island carries.
    ///
    /// The client holds role *ids* only (`AuthenticatedUser.app_role_ids`),
    /// and `/users/me` returns the names alongside them but the parser drops
    /// them. Until that is fixed the id is the honest answer: an operator
    /// seeing a number learns nothing, which is why this is a tag and not a
    /// claim. See `docs/ui-overhaul-status.md`.
    fn app_role_tag(&self) -> String {
        match &self.auth_identity {
            Some(identity) => match identity.app_role_label() {
                label if label.is_empty() => "PARTICIPANT".to_string(),
                label => label,
            },
            None => "NOT SIGNED IN".to_string(),
        }
    }

    fn zone_user_body(&mut self, ui: &mut egui::Ui) {
        // Signed out, the island IS the way back in.
        //
        // Found by driving: signing out cleared the session and left the
        // console showing "not signed in" with a Sign out button and no way
        // to sign in. `sign_out` never raises `show_login`, and the floating
        // Login island is suppressed while the zone owns the left edge — so
        // the only route back was relaunching the app. The identity block is
        // what a signed-in operator reads; a signed-out one has nothing to
        // read and something to do.
        if self.auth_identity.is_none() {
            self.sign_in_form(ui);
            return;
        }
        let identity = self.auth_identity.clone();
        ui.horizontal(|ui| {
            self.avatar(ui, 40.0, identity.as_ref());
            ui.vertical(|ui| {
                match &identity {
                    Some(i) => ui.label(i.label()),
                    None => ui.weak("not signed in"),
                };
                ui.weak(self.app_role_tag());
            });
        });
        if let Some(i) = &identity
            && let Some(email) = &i.email
        {
            ui.weak(email);
        }
        if ui.button("Sign out").clicked() {
            self.sign_out("signed out from the operator island");
        }
        // NO status line here. It used to sit under a separator, and the
        // island is 112pt tall while the identity block and the button
        // already use all of it — so the status painted OUTSIDE the panel,
        // over the map, half-occluded by the next island's title band. That
        // is what a render showed and no geometry test could: it looked like
        // a label belonging to the island below.
        //
        // The status itself is not lost. Sign-in and sign-out both report
        // through the top zone's link state, which is where a transient
        // connection fact belongs anyway.
    }

    /// The operator's picture, or the default person glyph.
    ///
    /// `photo_url` is a presigned URL that expires, so it is fetched by egui's
    /// loader and cached by URL rather than held by us; there is nothing to
    /// refresh. `None` is the common case and not an error: the backend has
    /// no endpoint to set a photo, so most accounts have none, and a failed
    /// presign is indistinguishable from absence by design.
    ///
    /// A circular crop is painted over the image rather than requested,
    /// because the source is whatever the account uploaded and the island's
    /// silhouette is a circle.
    fn avatar(&self, ui: &mut egui::Ui, size: f32, identity: Option<&tfg::backend::AuthenticatedUser>) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
        let p = ui.painter_at(rect);
        // `try_load_texture` returns `None` until the asynchronous loader has
        // finished, so the glyph shows for the first frames and is replaced
        // by the photo when it arrives. That is the honest fallback: a
        // missing picture and an unfinished one look the same, and neither is
        // worth reporting.
        let photo = identity.and_then(|i| i.photo_url.as_deref()).and_then(|url| {
            match ui
                .ctx()
                .try_load_texture(url, egui::TextureOptions::LINEAR, egui::load::SizeHint::Scale(size.into()))
            {
                Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture),
                _ => None,
            }
        });
        match photo {
            Some(tex) => {
                ui.add(
                    egui::Image::from_texture(tex)
                        .fit_to_exact_size(egui::vec2(size, size))
                        .sense(egui::Sense::hover()),
                );
            }
            None => self.person_glyph(&p, rect, size),
        }
        p.circle_stroke(
            rect.center(),
            size * 0.5,
            egui::Stroke::new(1.0, tfg::tokens::HAIRLINE_SLATE),
        );
    }

    /// The default person mark: a head and shoulders in Body Silver on a
    /// Slate disc, ringed by the hairline.
    fn person_glyph(&self, p: &egui::Painter, rect: egui::Rect, size: f32) {
        p.circle_filled(rect.center(), size * 0.5, tfg::tokens::PANEL_SLATE);
        let c = rect.center();
        p.circle_filled(
            c + egui::vec2(0.0, -size * 0.13),
            size * 0.16,
            tfg::tokens::BODY_SILVER,
        );
        p.add(egui::Shape::convex_polygon(
            vec![
                c + egui::vec2(-size * 0.26, size * 0.30),
                c + egui::vec2(-size * 0.26, size * 0.10),
                c + egui::vec2(0.0, -size * 0.02),
                c + egui::vec2(size * 0.26, size * 0.10),
                c + egui::vec2(size * 0.26, size * 0.30),
            ],
            tfg::tokens::BODY_SILVER,
            egui::Stroke::NONE,
        ));
    }

    /// The only island that renders when no session is held, so it is also
    /// the only place a session can be NAMED.
    ///
    /// It used to offer a bare "Start a new session" button, which called the
    /// create verb with nothing filled and answered "name the game first" —
    /// pointing at a form that was not on screen, because the create form lives
    /// on the Essentials island and that island only renders once a session is
    /// HELD. A verb that cannot succeed is worse than no verb: it looks like
    /// the way in and it is not.
    ///
    /// Name only, because name is the one field `POST /games` requires, and
    /// every other field is optional and reachable from the edit form once the
    /// game exists. Creating here hands the game over, the zone becomes
    /// Planning, and the rest of the plan is authored from there.
    /// The only island that renders when no session is held, so it is also
    /// the only place a session can be NAMED.
    ///
    /// It used to offer a bare "Start a new session" button, which called the
    /// create verb with nothing filled and answered "name the game first" —
    /// pointing at a form that was not on screen, because the create form lives
    /// on the Essentials island and that island only renders once a session is
    /// HELD. A verb that cannot succeed is worse than no verb: it looks like
    /// the way in and it is not.
    ///
    /// Name only, because name is the one field `POST /games` requires, and
    /// every other field is optional and reachable from the edit form once the
    /// game exists. Creating here hands the game over, the zone becomes
    /// Planning, and the rest of the plan is authored from there.
    fn zone_start_body(&mut self, ui: &mut egui::Ui) {
        if self.session_picker_visible() {
            self.session_picker_row(ui);
            ui.separator();
        }
        ui.label("No session held.");
        ui.weak("A session is a plan the Game Master authors: a force, a roster and a window.");
        ui.separator();
        ui.label(egui::RichText::new("name").weak().small());
        ui.add(
            egui::TextEdit::singleline(&mut self.setup_name)
                .desired_width(f32::INFINITY)
                .hint_text("the exercise's name"),
        );
        if ui
            .button("Create session →")
            .on_hover_text("create it and hold it; the rest of the plan is authored in Planning")
            .clicked()
        {
            self.setup_create_game();
        }
        status_line(ui, &self.users_status.clone());
    }

    fn zone_essentials_body(&mut self, ui: &mut egui::Ui) {
        self.setup_game_ui(ui);
    }

    fn zone_control_body(&mut self, ui: &mut egui::Ui) {
        ui.label("Game time multiplier");
        let mut factor = self.factor_draft;
        let resp = ui.add(
            egui::DragValue::new(&mut factor)
                .speed(0.5)
                .range(1.0..=144.0)
                .suffix("×"),
        );
        if resp.changed() {
            self.factor_draft = factor;
        }
        ui.weak(format!("Pace {:.0}× — read from Minos.", self.game_ratio));
        if ui.button("Apply").clicked() {
            self.set_minos_factor(self.factor_draft);
        }
    }

    fn zone_fleet_body(&mut self, ui: &mut egui::Ui) {
        // The wizard's step lock becomes a note on the island it applies to.
        // The information is worth keeping: it is the reason a picker button
        // is disabled — the zone just no longer hides the panel behind it.
        if let Some(reason) = self.setup_step_lock(2) {
            ui.weak(format!("Not yet — {reason}."));
            ui.separator();
        }
        self.setup_fleet_ui(ui);
    }

    fn zone_players_body(&mut self, ui: &mut egui::Ui) {
        if let Some(reason) = self.setup_step_lock(1) {
            ui.weak(format!("Not yet — {reason}."));
            ui.separator();
        }
        self.setup_players_ui(ui);
        // The checklist and the advance live at the END of the column,
        // under the last island in dependency order. They were the wizard's
        // fourth step; ending the sequence is still what they mean, and
        // putting them under Players keeps that reading.
        ui.separator();
        self.setup_ready_ui(ui);
    }

    fn zone_ready_body(&mut self, ui: &mut egui::Ui) {
        let blockers = self.setup_checklist();
        ui.label(format!("{} outstanding", blockers.len()));
        ui.separator();
        self.readiness_ui(ui);
        if let Some(note) = self.phase_note.clone() {
            warn_line(ui, note);
        }
        ui.separator();
        // Entering execution is the Preparation island's one verb. It was
        // the phase bar's, and the bar went away with the wizard, so without
        // this an exercise could never leave preparation.
        let clear = blockers.is_empty();
        if ui
            .add_enabled(
                clear,
                egui::Button::new("Enter execution →"),
            )
            .on_hover_text(if clear {
                "the server gates this itself"
            } else {
                "readiness is not clear"
            })
            .clicked()
        {
            self.setup_advance_execution();
        }
        if !clear {
            for b in blockers.iter().take(3) {
                ui.weak(format!("• {b}"));
            }
        }
        status_line(ui, &self.users_status.clone());
    }

    fn zone_exercise_body(&mut self, ui: &mut egui::Ui) {
        let elapsed = self.game_elapsed_secs.unwrap_or(0);
        ui.label(format!("G+{:02}:{:02}", elapsed / 60, elapsed % 60));
        if self.game_paused {
            ui.label(
                egui::RichText::new("PAUSED")
                    .strong()
                    .color(tfg::tokens::ALERT_YELLOW),
            );
        }
        ui.separator();
        if ui.button("End session →").clicked() {
            self.close_game();
        }
        if let Some(note) = self.phase_note.clone() {
            warn_line(ui, note);
        }
        status_line(ui, &self.users_status.clone());
    }

    fn zone_orders_body(&mut self, ui: &mut egui::Ui) {
        // The same gate the free-floating island had: orders are for whoever
        // is actually driving the exercise, not for an observer watching one.
        // Moving the surface must not widen who can order.
        if self.is_observer() {
            ui.weak("Observing — orders are not yours to give.");
            return;
        }
        if self.selection.is_none() {
            ui.weak("Select a ship or a group on the map.");
        }
        egui::ScrollArea::vertical()
            .id_salt("zone-orders")
            .max_height(360.0)
            .show(ui, |ui| self.orders_body(ui));
    }

    fn zone_log_body(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("zone-log")
            .max_height(200.0)
            .show(ui, |ui| self.log_island(ui));
    }

    /// Who is actually in the exercise right now.
    ///
    /// `joined_at` is the fact that matters and it is not the roster: a
    /// participant is SEATED when they are given a role and IN THE ROOM when
    /// they arrive with the room key. During execution the difference is the
    /// difference between a commander who can be reached and one who is
    /// assigned to nobody, so the two are counted separately rather than
    /// collapsed into one headcount.
    fn zone_crew_body(&mut self, ui: &mut egui::Ui) {
        if self.users_game.is_none() {
            ui.weak("No session held.");
            return;
        }
        if self.users_roster.is_empty() {
            ui.weak("Nobody seated.");
            return;
        }
        ui.weak("seated is a role; in room is arrival");
        for p in self.users_roster.clone() {
            let here = p.joined_at.is_some();
            ui.horizontal(|ui| {
                ui.label(p.user_name.clone());
                ui.label(egui::RichText::new(p.role_name.clone()).weak().small());
                let mark = if self.users_is_judge(&p) {
                    egui::RichText::new("judge").weak().small()
                } else if !here {
                    egui::RichText::new("not arrived")
                        .color(tfg::tokens::ALERT_YELLOW)
                        .small()
                } else if p.ready {
                    egui::RichText::new("in room")
                        .color(tfg::tokens::SIGNAL_GREEN)
                        .small()
                } else {
                    egui::RichText::new("in room · not ready")
                        .color(tfg::tokens::ALERT_YELLOW)
                        .small()
                };
                ui.label(mark);
            });
        }
    }

    /// How many participants have actually arrived, not merely been seated.
    fn crew_in_room(&self) -> usize {
        self.users_roster.iter().filter(|p| p.joined_at.is_some()).count()
    }

    /// The Orders island's trailing note: what it is currently pointed at.
    ///
    /// A group selection holds a group ID, not a name, so the name is
    /// resolved here rather than printed raw — a trailing note reading
    /// "Orders · 4" tells the operator nothing about which four.
    fn selection_label(&self) -> String {
        match &self.selection {
            Some(Selection::Ship(id)) => id.clone(),
            Some(Selection::Group(gid)) => self
                .group_info(gid)
                .map(|(name, members)| format!("{name} ({})", members.len()))
                .unwrap_or_else(|| format!("{gid} (gone)")),
            None => "NO SELECTION".to_string(),
        }
    }

    fn zone_assessment_body(&mut self, ui: &mut egui::Ui) {
        self.assessment_body(ui);
    }

    /// New session from Closure: starts a NEW exercise from the
    /// picker — the closed game stays closed on Minos, so the hold is
    /// released instead of pretending the backend moved backward.
    /// M6: the new exercise starts frozen with no ghost legs from the
    /// old sim. Called from the assessment workspace, never the bar.
    fn new_session(&mut self) {
        self.mode.reset();
        self.sim_ready = false;
        self.phase_note = None;
        self.release_all_local();
        // #101: reset disarms — a new exercise still needs placement.
        self.mode.armed.store(true, Ordering::SeqCst);
        self.hold_sim_for_setup();
        self.set_held_game(None);
        self.users_game_state = None;
        self.minos_clock = None;
        self.minos_time_factor = None;
        self.minos_room_key = None;
        self.clock_denied = false;
        self.watch_game_channel();
        self.users_roster.clear();
        self.users_gunits.clear();
        // The caller's pieces belonged to the released hold — the next
        // join re-deals them, with the tree. Gap flags are
        // account-level and stay.
        self.commanded_hulls.clear();
        self.minos_tree.clear();
        self.tree_gap = false;
        self.clear_fleet_pick();
        self.users_placements.clear();
        self.force.clear();
        self.force_seeded_for = None;
        self.force_queue.clear();
        self.force_flush_then = None;
        self.map_unit_move = None;
        self.placement_unplaced = 0;
        self.placement_ready = false;
        // Pictures belong to the ended exercise, not the new one.
        self.clear_visual_cache();
        // Back to step 1 of the setup flow (#79).
        self.setup_step = 0;
        self.assessment_tab = 0;
    }

    /// Closure assessment workspace, as an island body: summary, timeline,
    /// judgements, reviews, transcript. It was a `Panel::left`; the side zone
    /// owns its placement now, so the tabs and the scroll are the only thing
    /// left here.
    fn assessment_body(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for (i, label) in ["Summary", "Timeline", "Judgements", "Reviews", "Transcript"]
                .iter()
                .enumerate()
            {
                if ui
                    .selectable_label(self.assessment_tab == i, *label)
                    .clicked()
                {
                    self.assessment_tab = i;
                }
            }
        });
        ui.separator();
        match self.assessment_tab {
            1 => self.timeline_ui(ui),
            2 => self.judgements_ui(ui),
            3 => self.reviews_ui(ui),
            4 => self.transcript_ui(ui),
            _ => self.assessment_summary_ui(ui),
        }
    }

    /// Assessment slot: session summary + export + new session.
    fn assessment_summary_ui(&mut self, ui: &mut egui::Ui) {
        let game = self
            .users_game
            .clone()
            .map(|(_, n)| n)
            .unwrap_or_else(|| "no session".to_string());
        ui.label(format!("session: {game}"));
        ui.label(format!(
            "Exercise state: {}",
            self.users_game_state.as_deref().unwrap_or("—")
        ));
        if self.roster_gap {
            ui.weak("seats: staff counts unavailable for your account");
        } else {
            let (side, ready) = self.setup_gate_counts();
            ui.label(format!("{side} exercise-side seat(s), {ready} ready"));
        }
        if self.units_gap {
            ui.label(format!("{} hull(s) commanded", self.commanded_hulls.len()));
        } else {
            ui.label(format!("{} piece(s)", self.users_gunits.len()));
        }
        ui.label(format!(
            "placements: {} placed · {} to go",
            self.users_placements.len(),
            self.placement_unplaced
        ));
        if let Some(f) = self.minos_time_factor {
            ui.label(format!("clock factor: {f}x"));
        }
        ui.label(format!("transcript: {} line(s)", self.transcript.len()));
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Open debrief →").clicked() {
                self.assessment_tab = 4;
            }
            if ui.button("Export session").clicked() {
                let line = format!("session log: {}", self.session_log_path.display());
                self.feed(line.clone());
                self.users_status = line;
            }
            if ui.button("New session").clicked() {
                self.new_session();
            }
        });
        status_line(ui, &self.users_status.clone());
    }

    /// Assessment slot: Closure timeline (event stream, filters,
    /// cursor paging). Content arrives in its own ticket.
    fn timeline_ui(&mut self, ui: &mut egui::Ui) {
        if self.users_game.is_none() {
            ui.weak("Hold a session first — the timeline reads one exercise.");
            return;
        }
        // Source filter: all five streams or one. Changing it reloads
        // from the first page — cursors never cross a filter change.
        ui.horizontal(|ui| {
            ui.label("source:");
            let mut picked = self.timeline_source.clone();
            ui.selectable_value(&mut picked, None, "All");
            for s in ["transition", "clock", "order", "message", "judgement"] {
                ui.selectable_value(&mut picked, Some(s.to_string()), s);
            }
            if picked != self.timeline_source {
                self.timeline_source = picked;
                self.load_timeline(false);
            }
        });
        ui.horizontal(|ui| {
            ui.label("personnel:");
            ui.text_edit_singleline(&mut self.timeline_personnel);
            ui.label("unit:");
            ui.text_edit_singleline(&mut self.timeline_unit);
        });
        ui.horizontal(|ui| {
            ui.label("from:");
            ui.text_edit_singleline(&mut self.timeline_from);
            ui.label("to:");
            ui.text_edit_singleline(&mut self.timeline_to);
        });
        ui.horizontal(|ui| {
            if ui.small_button("apply + reload").clicked() {
                self.load_timeline(false);
            }
            if ui.small_button("reload").clicked() {
                self.load_timeline(false);
            }
        });
        ui.weak("ids are numeric; windows are RFC 3339, empty means the whole exercise.");
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                if self.timeline_events.is_empty() {
                    ui.weak("No events yet — reload to read the exercise.");
                }
                for e in self.timeline_events.clone() {
                    ui.label(Self::timeline_line(&e));
                }
            });
        if self.timeline_has_more && ui.button("load more →").clicked() {
            self.load_timeline(true);
        }
        status_line(ui, &self.users_status.clone());
    }

    /// Load the timeline page off-thread: first page or the cursor's
    /// next. Numeric filters parse here; a bad id refuses before any
    /// request, a bad window fails loudly from the server naming it.
    fn load_timeline(&mut self, append: bool) {
        if self.setup_busy("timeline") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("timeline failed: {e}");
                return;
            }
        };
        let parse_id = |raw: &str, what: &str| -> Result<Option<i64>, String> {
            let t = raw.trim();
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<i64>()
                .map(Some)
                .map_err(|_| format!("{what} must be a numeric id"))
        };
        let personnel = match parse_id(&self.timeline_personnel.clone(), "personnel") {
            Ok(p) => p,
            Err(e) => {
                self.users_status = e;
                return;
            }
        };
        let unit = match parse_id(&self.timeline_unit.clone(), "unit") {
            Ok(u) => u,
            Err(e) => {
                self.users_status = e;
                return;
            }
        };
        let from = nonempty(&self.timeline_from);
        let to = nonempty(&self.timeline_to);
        let source = self.timeline_source.clone();
        let cursor = if append { self.timeline_cursor.clone() } else { None };
        self.setup_op = Some(spawn_rest("timeline", move || {
            master
                .timeline_page(
                    &tok,
                    gid,
                    source.as_deref(),
                    personnel,
                    unit,
                    from.as_deref(),
                    to.as_deref(),
                    cursor.as_deref(),
                    50,
                )
                .map_err(|e| e.to_string())
                .map(|page| SetupDone::Timeline(page, append))
        }));
    }

    /// One timeline event on one line: scenario instant plus the
    /// source's own facts. Order positions ride the payload (the fix
    /// proof), so no playback renderer is needed for the debrief —
    /// the map snapshot behind the panel is the picture.
    fn timeline_line(e: &tfg::backend::TimelineEvent) -> String {
        let t = e
            .assumed_at
            .split('T')
            .nth(1)
            .and_then(|s| s.strip_suffix('Z').or(Some(s)))
            .unwrap_or(&e.assumed_at);
        let d = &e.data;
        let str_of = |k: &str| d[k].as_str().unwrap_or("").to_string();
        let num = |k: &str| d[k].as_f64().unwrap_or(0.0);
        match e.etype.as_str() {
            "transition" => format!(
                "{t} {} → {} (by {})",
                str_of("from_state"),
                str_of("to_state"),
                d["changed_by"].as_i64().unwrap_or(0)
            ),
            "clock" => {
                let note = str_of("note");
                let note_seg = if note.is_empty() { String::new() } else { format!(" · {note}") };
                let by_seg = match d["changed_by"].as_i64().unwrap_or(0) {
                    0 => String::new(),
                    by => format!(" (by {by})"),
                };
                format!("{t} clock → {}x{note_seg}{by_seg}", num("factor"))
            }
            "order" => format!(
                "{t} order {} {:.0}° @ {:.0} kn{} @ ({:.4}, {:.4})",
                str_of("unit_name"),
                num("heading_deg"),
                num("speed_kn"),
                if d["was_clamped"].as_bool().unwrap_or(false) {
                    " · clamped"
                } else {
                    ""
                },
                num("latitude"),
                num("longitude")
            ),
            "message" => {
                let text: String = str_of("content").chars().take(120).collect();
                format!("{t} msg {}: {text}", str_of("sender_name"))
            }
            "judgement" => {
                let cited = str_of("cited_unit_name");
                format!(
                    "{t} judgement {} score {} by {}{}",
                    str_of("personnel_name"),
                    d["score"].as_i64().unwrap_or(0),
                    str_of("judge_name"),
                    if cited.is_empty() { String::new() } else { format!(" · on {cited}") }
                )
            }
            other => format!("{t} {other}"),
        }
    }

    /// Assessment slot: judge-side judgements (append-only). Content
    /// arrives in its own ticket.
    fn judgements_ui(&mut self, ui: &mut egui::Ui) {
        if self.users_game.is_none() {
            ui.weak("Hold a session first — judgements read one exercise.");
            return;
        }
        let judge_seat = self.own_roster_row().is_some_and(|p| p.judge);
        if !judge_seat {
            ui.weak("Judging needs a judge-side seat — the list below still reads.");
        }
        ui.separator();
        ui.strong("Record");
        // Subject: roster picker minus the caller (self-judging fails
        // loudly server-side; the picker refuses it first). A gapped
        // roster falls back to a numeric personnel id.
        let mut subjects: Vec<(i64, String)> = self
            .users_roster
            .iter()
            .filter(|p| Some(p.user_id) != self.auth_user_id)
            .map(|p| (p.user_id, p.user_name.clone()))
            .collect();
        subjects.sort_by(|a, b| a.1.cmp(&b.1));
        if subjects.is_empty() {
            ui.horizontal(|ui| {
                ui.label("personnel id:");
                ui.text_edit_singleline(&mut self.judge_subject_id);
            });
        } else {
            if self.judge_subject.is_none_or(|s| !subjects.iter().any(|(id, _)| *id == s)) {
                self.judge_subject = subjects.first().map(|(id, _)| *id);
            }
            let current = self.judge_subject.unwrap_or(0);
            let label = subjects
                .iter()
                .find(|(id, _)| *id == current)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "pick".to_string());
            egui::ComboBox::from_label("subject")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (id, name) in &subjects {
                        ui.selectable_value(&mut self.judge_subject, Some(*id), name);
                    }
                });
        }
        ui.horizontal(|ui| {
            ui.label("score:");
            ui.text_edit_singleline(&mut self.judge_score);
        });
        ui.weak("score is free text. No action citation — no public fix-list route exists.");
        if ui
            .add_enabled(judge_seat, egui::Button::new("record judgement"))
            .clicked()
        {
            self.record_judgement();
        }
        ui.separator();
        ui.strong("Marks");
        ui.weak("Append-only: a mark can never be edited or deleted — correct it with a new one.");
        if ui.small_button("reload").clicked() {
            self.load_judgements();
        }
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                if self.judgements.is_empty() {
                    ui.weak("No marks yet.");
                }
                for j in self.judgements.clone() {
                    let cited = j
                        .cited_unit
                        .map(|u| format!(" · on {u}"))
                        .unwrap_or_default();
                    ui.label(format!(
                        "{}: {}{} — by {}",
                        j.personnel_name, j.score, cited, j.judge_name
                    ));
                }
            });
        status_line(ui, &self.users_status.clone());
    }

    /// Load the judgements page off-thread. Failures keep the last
    /// good list and report loudly.
    fn load_judgements(&mut self) {
        if self.setup_busy("judgements") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("judgements failed: {e}");
                return;
            }
        };
        self.setup_op = Some(spawn_rest("judgements", move || {
            master
                .judgements_list(&tok, gid, None)
                .map_err(|e| e.to_string())
                .map(SetupDone::Judgements)
        }));
    }

    /// Record the composed judgement off-thread: subject from the
    /// picker (or the numeric fallback), non-empty free-text score.
    /// Self-judging is refused before any request; the server judges
    /// the rest loudly (non-judge caller, unknown subject).
    fn record_judgement(&mut self) {
        if self.setup_busy("judgement") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let subject = if self.users_roster.is_empty() {
            match self.judge_subject_id.trim().parse::<i64>() {
                Ok(id) => id,
                Err(_) => {
                    self.users_status = "personnel id must be numeric".to_string();
                    return;
                }
            }
        } else {
            match self.judge_subject {
                Some(s) => s,
                None => {
                    self.users_status = "pick a subject first".to_string();
                    return;
                }
            }
        };
        if Some(subject) == self.auth_user_id {
            self.users_status = "cannot judge yourself".to_string();
            return;
        }
        if self.judge_score.trim().is_empty() {
            self.users_status = "write the score first".to_string();
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("judgement failed: {e}");
                return;
            }
        };
        let score = self.judge_score.trim().to_string();
        self.setup_op = Some(spawn_rest("judgement", move || {
            master
                .record_judgement(&tok, gid, subject, &score)
                .map_err(|e| e.to_string())
                .map(SetupDone::JudgementSent)
        }));
    }

    /// Assessment slot: author-owned reviews. Content arrives in its
    /// own ticket.
    fn reviews_ui(&mut self, ui: &mut egui::Ui) {
        if self.users_game.is_none() {
            ui.weak("Hold a session first — reviews read one exercise.");
            return;
        }
        ui.separator();
        ui.strong(if self.review_editing.is_some() { "Revise" } else { "File" });
        // Subject is fixed at filing: the picker shows on file, the
        // row's subject on revise. No author field — the caller is it.
        if self.review_editing.is_none() {
            let mut subjects: Vec<(i64, String)> = self
                .users_roster
                .iter()
                .map(|p| (p.user_id, p.user_name.clone()))
                .collect();
            subjects.sort_by(|a, b| a.1.cmp(&b.1));
            if subjects.is_empty() {
                ui.weak("No roster — filing needs a subject id the staff list owns.");
            } else {
                if self.review_subject.is_none_or(|s| !subjects.iter().any(|(id, _)| *id == s)) {
                    self.review_subject = subjects.first().map(|(id, _)| *id);
                }
                let current = self.review_subject.unwrap_or(0);
                let label = subjects
                    .iter()
                    .find(|(id, _)| *id == current)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| "pick".to_string());
                egui::ComboBox::from_label("subject")
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for (id, name) in &subjects {
                            ui.selectable_value(&mut self.review_subject, Some(*id), name);
                        }
                    });
            }
        }
        ui.add(
            egui::TextEdit::multiline(&mut self.review_body)
                .desired_rows(4)
                .hint_text("the conclusion, in words"),
        );
        ui.horizontal(|ui| {
            if self.review_editing.is_some() {
                if ui.button("save revision").clicked() {
                    self.revise_review();
                }
                if ui.small_button("cancel").clicked() {
                    self.review_editing = None;
                    self.review_body.clear();
                }
            } else if ui.button("file review").clicked() {
                self.file_review();
            }
        });
        ui.separator();
        ui.strong("Documents");
        ui.weak("Author-owned: edit renders on your own rows only, and there is no delete — filed reviews stay.");
        ui.horizontal(|ui| {
            if ui.small_button("reload").clicked() {
                self.load_reviews();
            }
            // The narrow applies on reload, like the inbox filter.
            if ui.checkbox(&mut self.review_mine_only, "mine only").changed() {
                self.load_reviews();
            }
        });
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                if self.reviews.is_empty() {
                    ui.weak("No documents yet.");
                }
                let mut edits: Vec<(i64, String)> = Vec::new();
                for r in self.reviews.clone() {
                    ui.strong(format!(
                        "{} — by {}{}",
                        r.personnel_name,
                        r.author_name,
                        if r.revised { " · revised" } else { " · filed" }
                    ));
                    ui.weak(format!("{} → {}", r.created_at, r.updated_at));
                    ui.label(r.body.clone());
                    let mine = Some(r.author_id) == self.auth_user_id;
                    if mine && ui.small_button("edit").clicked() {
                        edits.push((r.id, r.body.clone()));
                    }
                    ui.separator();
                }
                for (id, body) in edits {
                    self.review_editing = Some(id);
                    self.review_body = body;
                }
            });
        status_line(ui, &self.users_status.clone());
    }

    /// Load the reviews page off-thread (mine-narrow when checked).
    /// Failures keep the last good list and report loudly.
    fn load_reviews(&mut self) {
        if self.setup_busy("reviews") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("reviews failed: {e}");
                return;
            }
        };
        let author = if self.review_mine_only { self.auth_user_id } else { None };
        self.setup_op = Some(spawn_rest("reviews", move || {
            master
                .reviews_list(&tok, gid, None, author)
                .map_err(|e| e.to_string())
                .map(SetupDone::Reviews)
        }));
    }

    /// File the composed review off-thread: subject plus non-empty
    /// body. Permission (update grant) fails loudly server-side.
    fn file_review(&mut self) {
        if self.setup_busy("review") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let Some(subject) = self.review_subject else {
            self.users_status = "pick a subject first".to_string();
            return;
        };
        if self.review_body.trim().is_empty() {
            self.users_status = "write the review first".to_string();
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("review failed: {e}");
                return;
            }
        };
        let body = self.review_body.trim().to_string();
        self.setup_op = Some(spawn_rest("review", move || {
            master
                .file_review(&tok, gid, subject, &body)
                .map_err(|e| e.to_string())
                .map(SetupDone::ReviewSent)
        }));
    }

    /// Revise the row under edit off-thread: words only, subject
    /// fixed. Anyone-but-author fails with the explicit 403, which
    /// the status carries instead of a pre-hidden button.
    fn revise_review(&mut self) {
        if self.setup_busy("review") {
            return;
        }
        let Some(rid) = self.review_editing else {
            self.users_status = "nothing under revision".to_string();
            return;
        };
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        if self.review_body.trim().is_empty() {
            self.users_status = "write the revision first".to_string();
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("revision failed: {e}");
                return;
            }
        };
        let body = self.review_body.trim().to_string();
        self.setup_op = Some(spawn_rest("review", move || {
            master
                .revise_review(&tok, gid, rid, &body)
                .map_err(|e| e.to_string())
                .map(SetupDone::ReviewRevised)
        }));
    }

    /// Assessment slot: the frozen local trace tail. The map snapshot
    /// is the frozen map behind this panel.
    fn transcript_ui(&mut self, ui: &mut egui::Ui) {
        if self.transcript.is_empty() {
            ui.weak("No transcript — the session left no local trace.");
            return;
        }
        for line in self.transcript.clone() {
            ui.label(line);
        }
    }

    /// Load the task-organisation forest off-thread. A gapped read
    /// keeps the last good tree; a hold change queues it behind the
    /// bundle instead of refusing.
    fn load_minos_tree(&mut self) {
        let Some((gid, _)) = self.users_game.clone() else {
            return;
        };
        if self.setup_op.is_some() {
            self.queue_refresh(PendingRefresh::Tree);
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(e) => {
                self.users_status = format!("task organisation failed: {e}");
                return;
            }
        };
        self.setup_op = Some(spawn_rest("hierarchy", move || {
            Ok(SetupDone::Hierarchy(master.game_hierarchy(&tok, gid)))
        }));
    }

    /// One forest node plus the pieces sitting under it, indented by
    /// depth. Units come from the staff list with commanded hulls
    /// filling the gaps; node ids ride both parses.
    fn tree_node_ui(
        &self,
        ui: &mut egui::Ui,
        node: &tfg::backend::HierarchyNode,
        depth: usize,
        units: &[(i64, String, Option<i64>)],
    ) {
        let pad = "  ".repeat(depth);
        let mut head = format!("{pad}{} ({})", node.name, node.echelon_name);
        if !node.icon.is_empty() {
            head += &format!(" {}", node.icon);
        }
        ui.label(egui::RichText::new(head).strong());
        for (uid, name, _) in units.iter().filter(|(_, _, n)| *n == Some(node.id)) {
            let _ = uid;
            ui.label(format!("{pad}  · {name}"));
        }
        for child in &node.children {
            self.tree_node_ui(ui, child, depth + 1, units);
        }
    }

    /// Task-organisation section: the Minos forest with assigned
    /// pieces, or its gap. Staff-gated like roster and pieces — a
    /// participant sees the labeled gap, never an empty tree as
    /// truth. Sandbox Groups draw nowhere here by rule.
    fn task_org_ui(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Task organisation");
            if ui.small_button("reload").clicked() {
                self.load_minos_tree();
            }
        });
        if self.tree_gap {
            ui.weak("task organisation: staff access unavailable");
            return;
        }
        if self.minos_tree.is_empty() {
            ui.weak("No nodes yet — the tree is built during planning and preparation.");
            return;
        }
        let mut units: Vec<(i64, String, Option<i64>)> = self
            .users_gunits
            .iter()
            .map(|g| (g.unit_id, g.unit_name.clone(), g.hierarchy_node))
            .collect();
        for g in &self.commanded_hulls {
            if !units.iter().any(|(id, _, _)| *id == g.unit_id) {
                units.push((g.unit_id, g.unit_name.clone(), g.hierarchy_node));
            }
        }
        for node in self.minos_tree.clone() {
            self.tree_node_ui(ui, &node, 0, &units);
        }
        let unassigned: Vec<String> = units
            .iter()
            .filter(|(_, _, n)| n.is_none())
            .map(|(_, name, _)| name.clone())
            .collect();
        if !unassigned.is_empty() {
            ui.weak(format!("Unassigned: {}", unassigned.join(", ")));
        }
    }

    /// Login island (login ticket): Minos sign-in against the env-owned
    /// endpoint, the must_change_password gate as a blocking form,
    /// sign-out. Token state feeds the later socket work; refresh runs
    /// proactively per frame.
    /// Identifier, password, sign in. One function because two surfaces need
    /// it and they drifted: the free-floating Login island carried the form
    /// while the side zone's Operator island showed identity alone, so signing
    /// out of the zone left no way back in.
    fn sign_in_form(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("identifier:");
            ui.add_sized(
                [ui.available_width(), AUTH_FIELD_H],
                egui::TextEdit::singleline(&mut self.login_identifier)
                    .min_size(egui::vec2(0.0, AUTH_FIELD_H))
                    .vertical_align(egui::Align::Center),
            );
        });
        ui.horizontal(|ui| {
            ui.label("password:");
            ui.add_sized(
                [ui.available_width(), AUTH_FIELD_H],
                egui::TextEdit::singleline(&mut self.login_password)
                    .password(true)
                    .min_size(egui::vec2(0.0, AUTH_FIELD_H))
                    .vertical_align(egui::Align::Center),
            );
        });
        if ui.button("sign in").clicked() {
            self.attempt_sign_in();
        }
    }

    fn login_island(&mut self, ui: &mut egui::Ui) {
        ui.heading("Login");
        if let Some(user) = self.auth_user.clone() {
            let ttl_note = match (self.auth_issued_at, self.auth_ttl_secs) {
                (Some(t), ttl) if ttl > 0 => {
                    let left = ttl.saturating_sub(t.elapsed().as_secs());
                    format!(" · expires in {}:{:02}", left / 60, left % 60)
                }
                _ => String::new(),
            };
            ui.label(format!(
                "signed in as {user}{ttl_note}{}",
                if self.auth_degraded { " · degraded store" } else { "" }
            ));
            if ui.small_button("sign out").clicked() {
                self.sign_out("operator");
            }
            if self.auth_needs_password_change {
                ui.separator();
                ui.heading("Change password");
                ui.label("The backend shuts every door until this is done.");
                self.change_password_ui(ui);
            }
        } else {
            self.sign_in_form(ui);
        }
        status_line(ui, &self.auth_status.clone());
        // Local-first sync (master-data ticket): whole-table replace
        // from the backend, server wins. Runs on sign-in; the button
        // re-runs it any time. Reads serve from disk either way.
        ui.separator();
        ui.heading("Sync");
        ui.horizontal(|ui| {
            let authed = self.auth_token.is_some();
            if ui.add_enabled(authed, egui::Button::new("sync now")).clicked() {
                self.sync_now();
            }
            if ui
                .add_enabled(authed, egui::Button::new("fetch hull specs"))
                .clicked()
            {
                self.sync_specs_now();
            }
            if !authed {
                ui.label("sign in first");
            }
        });
        status_line(ui, &self.sync_status.clone());
    }

    /// Blocking must-change-password form (login ticket), shared by
    /// the Login island and the onboarding State A card (#77): the
    /// backend refuses every door until this lands, then re-enters
    /// with the new password.
    fn change_password_ui(&mut self, ui: &mut egui::Ui) {
        let Some(user) = self.auth_user.clone() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.label("current:");
            ui.add_sized(
                [ui.available_width(), AUTH_FIELD_H],
                egui::TextEdit::singleline(&mut self.pw_current)
                    .password(true)
                    .min_size(egui::vec2(0.0, AUTH_FIELD_H))
                    .vertical_align(egui::Align::Center),
            );
        });
        ui.horizontal(|ui| {
            ui.label("new (12+):");
            ui.add_sized(
                [ui.available_width(), AUTH_FIELD_H],
                egui::TextEdit::singleline(&mut self.pw_new)
                    .password(true)
                    .min_size(egui::vec2(0.0, AUTH_FIELD_H))
                    .vertical_align(egui::Align::Center),
            );
        });
        if ui.button("change and re-enter").clicked() {
            // Client-side length gate (harden): the backend enforces
            // 12+, so refuse locally with a specific message first.
            if self.pw_new.len() < 12 {
                self.auth_status =
                    "change refused: new password needs 12+ characters".to_string();
                return;
            }
            let base = self.minos_base.clone();
            let tok = self.auth_token.clone().unwrap_or_default();
            let cur = self.pw_current.clone();
            let new = self.pw_new.clone();
            if self.pw_op.is_some() {
                self.auth_status = "password change already running…".to_string();
                return;
            }
            self.auth_status = "changing password…".to_string();
            // M7: change + re-entry + gate probe run off-thread; the
            // frame pump applies the staged outcome.
            self.pw_op = Some(spawn_rest("password", move || {
                let client =
                    MinosAuth::new(&base).map_err(|e| PwResult::ChangeFailed(format!("change failed: {e}")))?;
                client
                    .change_password(&tok, &cur, &new)
                    .map_err(|e| PwResult::ChangeFailed(format!("change failed: {e}")))?;
                // Password change invalidates other refresh tokens:
                // re-enter with the new password.
                let pair = client
                    .login(&user, &new)
                    .map_err(|_| PwResult::ReentryFailed)?;
                let token = pair.access_token.clone();
                match client.me(&token) {
                    Ok(identity) => Ok(PwResult::Changed(LoginDone {
                        user,
                        pair,
                        identity: Some(identity),
                        needs_change: false,
                        probe_note: None,
                    })),
                    Err(e) => Ok(PwResult::GateShut(format!(
                        "re-entered, gate still shut: {e}"
                    ))),
                }
            }));
        }
    }

    /// Apply a finished password change on the UI thread.
    fn apply_password(&mut self, res: PwResult) {
        match res {
            PwResult::Changed(done) => {
                self.pw_current.clear();
                self.pw_new.clear();
                self.store_pair(done.user.clone(), done.pair);
                self.auth_user_id = done.identity.as_ref().map(|i| i.id);
                self.auth_identity = done.identity;
                self.watch_personal_channel();
                self.auth_needs_password_change = false;
                self.auth_status = format!("signed in as {}", done.user);
                self.sync_now();
            }
            PwResult::ChangeFailed(e) => {
                self.auth_status = e;
            }
            PwResult::ReentryFailed => {
                self.sign_out("re-entry failed");
            }
            PwResult::GateShut(e) => {
                self.auth_status = e;
            }
        }
    }

    /// Frame pump for off-thread REST (M7): harvest finished ops and
    /// apply them on the UI thread. Slots clear as they resolve, so a
    /// failed op never wedges its button — the status line says why.
    /// Any REST request still in flight. A completed request is polled
    /// without a waker, so while one is outstanding the loop has to keep
    /// asking for frames — otherwise a login would sit on a dead console
    /// until the watchdog fired.
    fn rest_in_flight(&self) -> bool {
        self.login_op.is_some()
            || self.refresh_op.is_some()
            || self.sync_op.is_some()
            || self.spec_op.is_some()
            || self.pw_op.is_some()
            || self.plot_op.is_some()
            || self.setup_op.is_some()
            || self.log_op.is_some()
            || self.image_op.is_some()
    }

    fn pump_rest_ops(&mut self) {
        if let Some(res) = self.login_op.as_ref().and_then(|op| op.poll()) {
            self.login_op = None;
            // A completed request is invisible until a frame asks for it.
            self.dirty = true;
            match res {
                Ok(done) => self.apply_login(done),
                Err(e) => self.auth_status = e,
            }
        }
        if let Some(res) = self.refresh_op.as_ref().and_then(|op| op.poll()) {
            self.refresh_op = None;
            // A completed request is invisible until a frame asks for it.
            self.dirty = true;
            let user = self.auth_user.clone().unwrap_or_default();
            self.apply_refresh(user, res);
        }
        if let Some(res) = self.sync_op.as_ref().and_then(|op| op.poll()) {
            self.sync_op = None;
            // A completed request is invisible until a frame asks for it.
            self.dirty = true;
            self.apply_sync(res);
        }
        if let Some(res) = self.spec_op.as_ref().and_then(|op| op.poll()) {
            self.spec_op = None;
            // A completed request is invisible until a frame asks for it.
            self.dirty = true;
            self.apply_specs(res);
        }
        // The scenario book. A step write answers with ONE scenario, so it
        // is spliced into the local list rather than replacing it:
        // replacing would drop every other scenario the author was not
        // looking at, and the composer is a whole-book tool.
        if let Some(res) = self.scenarios_op.as_ref().and_then(|op| op.poll()) {
            self.scenarios_op = None;
            // A completed request is invisible until a frame asks for it.
            self.dirty = true;
            match res {
                Ok(Book::List(list)) => {
                    self.scenarios = list;
                    self.users_status = format!("{} scenario(s).", self.scenarios.len());
                }
                Ok(Book::One(one)) => {
                    if let Some(slot) = self.scenarios.iter_mut().find(|s| s.id == one.id) {
                        *slot = one.clone();
                    } else {
                        self.scenarios.push(one.clone());
                    }
                    if self.composer_scenario.is_none() {
                        self.composer_scenario = Some(one.id);
                    }
                    self.users_status = format!("saved \u{201c}{}\u{201d}.", one.title);
                }
                Err(e) => self.users_status = format!("book refused: {e}"),
            }
        }
        if let Some(res) = self.pw_op.as_ref().and_then(|op| op.poll()) {
            self.pw_op = None;
            // A completed request is invisible until a frame asks for it.
            self.dirty = true;
            // The worker always resolves into a staged outcome; unwrap
            // either side into it.
            self.apply_password(res.unwrap_or_else(|e| e));
        }
        if let Some((game_id, result)) = self
            .plot_op
            .as_ref()
            .and_then(|slot| slot.op.poll().map(|result| (slot.game_id, result)))
        {
            self.plot_op = None;
            // A completed request is invisible until a frame asks for it.
            self.dirty = true;
            match result {
                Ok(done) => self.apply_plot(done),
                Err(e) => self.apply_plot(PlotDone { game_id, result: Err(e) }),
            }
        }
        // Pictures ride their own slot (never queued behind setup):
        // manifest once, then one temporary URL per pictured hull.
        if let Some(res) = self.image_op.as_ref().and_then(|op| op.poll()) {
            self.image_op = None;
            // A completed request is invisible until a frame asks for it.
            self.dirty = true;
            let request = self.image_request.take();
            match res {
                Ok(out) => self.apply_image(out),
                Err(e) => {
                    match request {
                        Some(ImageRequest::Manifest) => {
                            self.manifest_retry_at =
                                Some(Instant::now() + IMAGE_READ_RETRY_DELAY);
                        }
                        Some(ImageRequest::Url(unit_id)) => {
                            self.visuals.defer_source(unit_id);
                            if self.visuals.expects_source(unit_id)
                                && !self.pending_image_urls.contains(&unit_id)
                            {
                                self.pending_image_urls.push(unit_id);
                            }
                        }
                        None => {}
                    }
                    self.users_status = format!("pictures failed: {e}");
                }
            }
        }
        // Log work (journal parses, transcript tail, directory scan)
        // harvests here; failures report and keep the old view.
        if let Some(res) = self.log_op.as_ref().and_then(|op| op.poll()) {
            self.log_op = None;
            // A completed request is invisible until a frame asks for it.
            self.dirty = true;
            match res {
                Ok(LogOut::Files(files)) => {
                    self.log_files = files;
                }
                Ok(LogOut::View(path, view)) => {
                    if self.log_view_path == Some(path) {
                        self.log_events = view.replay.clone();
                        self.replay_pos = self.log_events.len();
                        self.log_filter = "all".to_string();
                        self.log_view = Some(view);
                    }
                }
                Ok(LogOut::Transcript(lines)) => {
                    self.transcript = lines;
                }
                Err(e) => self.users_status = format!("log failed: {e}"),
            }
        }
        self.maybe_resolve_hull_image();
        // #100: the serialized setup slot. One arm applies every
        // setup read/write; failures feed + report loudly and keep
        // every old list (nothing clears on failure). Queued refreshes
        // dispatch once the slot frees.
        let setup_label = self.setup_op.as_ref().map(|op| op.label);
        if let Some(res) = self.setup_op.as_ref().and_then(|op| op.poll()) {
            self.setup_op = None;
            // A completed request is invisible until a frame asks for it.
            self.dirty = true;
            if self.app_mode == AppMode::Simulation {
                match res {
                    Ok(done) => self.apply_setup(done),
                    Err(e) => {
                        let line = match setup_label {
                            Some(l) => format!("{l} failed: {e}"),
                            None => format!("setup failed: {e}"),
                        };
                        self.feed(line.clone());
                        self.users_status = line;
                    }
                }
                self.dispatch_queued_refresh();
            }
        }
        // The WebSocket game-position stream is an accelerator. The
        // documented interim source is the REST plot; if the socket is
        // quiet, recover through REST once per second.
        let game_stream_fresh = self
            .last_game_position_at
            .is_some_and(|at| at.elapsed() < Duration::from_secs(5));
        if !game_stream_fresh
            && self.plot_op.is_none()
            && self.app_mode == AppMode::Simulation
            && self.users_game_state.as_deref() == Some("execution")
            && self.users_game.is_some()
        {
            let wait = 1u64.saturating_mul(1u64 << self.plot_fails.min(3)).min(120);
            let due = self
                .last_plot_try
                .is_none_or(|t| t.elapsed().as_secs() >= wait);
            if due {
                self.pull_minos_positions();
            }
        }
    }

    /// Apply any finished setup result (#100): lists replace, notes
    /// render, side effects (projection, watches, chained refreshes)
    /// run on the UI thread where the engine lives.
    fn apply_setup(&mut self, done: SetupDone) {
        match done {
            SetupDone::Games(games) => self.apply_games(games),
            SetupDone::GamesDenied => {
                self.games_gap = true;
                self.games_loaded = true;
                self.users_status =
                    "session list needs a staff read — join with the room key below".to_string();
            }
            SetupDone::Users(users) => self.apply_users(users),
            SetupDone::Bundle(b) => self.apply_bundle(b),
            SetupDone::Roster(roster, note) => {
                self.users_roster = roster;
                self.users_status = note;
            }
            SetupDone::ForceUnits(units) => {
                self.users_gunits = units;
                self.refresh_unassigned_units();
                // The unit list is the authority; the write is retired.
                self.force_write_done();
            }
            SetupDone::ForcePlacements(view) => {
                self.apply_placements(view);
                self.force_write_done();
            }
            SetupDone::ForceFailed(e, write) => {
                // The successful earlier writes are already reflected in
                // the live mirror; this one is not. Drop the queue so the
                // transition does not advance on a half-written force,
                // and say which hull held the whole thing up.
                self.force_queue.clear();
                self.force_flush_then = None;
                let line = format!("sync refused at {}: {e}", write.name());
                self.feed(line.clone());
                self.users_status = line.clone();
                self.phase_note = Some(line);
            }
            SetupDone::Game(to, row) => self.apply_transition(&to, row),
            SetupDone::GameUpdated(d) => {
                self.set_held_game(Some((d.id, d.name.clone())));
                self.users_game_state = Some(d.state.clone());
                self.minos_time_factor =
                    if d.time_factor > 0.0 { Some(d.time_factor) } else { None };
                self.users_project_stage(&d.state);
                self.users_status = format!("updated {} ({})", d.name, d.state);
                self.users_refresh_games();
                self.users_refresh_game();
            }
            SetupDone::GameDeleted(gid, name) => {
                if self.users_game.as_ref().is_some_and(|(id, _)| *id == gid) {
                    self.drop_hold(&format!("deleted {name}"));
                } else {
                    self.users_status = format!("deleted {name}");
                }
                self.users_refresh_games();
            }
            SetupDone::GameCreate(row, note) => self.apply_created_game(row, note),
            SetupDone::GameFailed(to, e, forbidden) => self.apply_transition_failed(to, e, forbidden),
            SetupDone::Join(join, note) => self.apply_join(join, note),
            SetupDone::JoinFailed(why) => {
                self.users_status = format!("join refused — {why}");
            }
            SetupDone::Clock(clock, verb) => self.apply_clock(clock, &verb),
            SetupDone::ClockDenied => {
                self.clock_denied = true;
                self.users_status =
                    "clock control needs the control grant in this session — \
                     Game Master or an entrusted judge-side role (app admin is not enough)"
                        .to_string();
            }
            SetupDone::FixBatch(outs) => self.apply_fix_batch(outs),
            SetupDone::Timeline(page, append) => {
                if append {
                    self.timeline_events.extend(page.events);
                } else {
                    self.timeline_events = page.events;
                }
                self.timeline_cursor = page.next_cursor;
                self.timeline_has_more = page.has_more;
                self.users_status = format!(
                    "timeline: {} event(s){}",
                    self.timeline_events.len(),
                    if page.has_more { " · more below" } else { "" }
                );
            }
            SetupDone::Judgements(list) => {
                self.judgements = list;
                self.users_status =
                    format!("judgements: {} mark(s)", self.judgements.len());
            }
            SetupDone::JudgementSent(view) => {
                self.judge_score.clear();
                self.users_status =
                    format!("recorded #{} · reloading judgements", view.id);
                self.load_judgements();
            }
            SetupDone::Reviews(list) => {
                self.reviews = list;
                self.users_status =
                    format!("reviews: {} document(s)", self.reviews.len());
            }
            SetupDone::ReviewSent(view) => {
                self.review_body.clear();
                self.review_subject = None;
                self.users_status =
                    format!("filed #{} · reloading reviews", view.id);
                self.load_reviews();
            }
            SetupDone::ReviewRevised(view) => {
                self.review_editing = None;
                self.review_body.clear();
                self.users_status =
                    format!("revised #{} · reloading reviews", view.id);
                self.load_reviews();
            }
            SetupDone::Hierarchy(res) => match res {
                Ok(tree) => {
                    self.minos_tree = tree;
                    self.tree_gap = false;
                    self.users_status = "task organisation synced".to_string();
                }
                Err(tfg::backend::BackendError::Forbidden { .. }) => {
                    self.tree_gap = true;
                    self.users_status =
                        "task organisation: staff access unavailable".to_string();
                }
                Err(e) => self.users_status = format!("task organisation failed: {e}"),
            },
            SetupDone::MsgPage(page) => {
                self.inbox = page.messages;
                self.inbox_total = page.total_records;
                self.inbox_pages = page.total_pages.max(1);
                self.inbox_page_no = page.current_page.max(1);
                self.inbox_has_next = page.has_next;
                self.inbox_has_prev = page.has_prev;
                self.users_status = format!(
                    "inbox: page {} of {} · {} total",
                    self.inbox_page_no, self.inbox_pages, self.inbox_total
                );
            }
            SetupDone::MsgOpen(msg) => {
                self.msg_open = Some(msg);
            }
            SetupDone::MsgDeleted(mid) => {
                if self.msg_open.as_ref().is_some_and(|m| m.id == mid) {
                    self.msg_open = None;
                }
                self.users_status = format!("deleted #{mid} · reloading inbox");
                self.refresh_inbox();
            }
            SetupDone::MsgSent(sent) => {
                self.msg_content.clear();
                self.msg_reply_to = None;
                self.users_status = format!("sent #{} · reloading inbox", sent.id);
                self.refresh_inbox();
            }
            SetupDone::Roles(roles) => {
                self.scenario_roles = roles;
                self.new_role_name.clear();
                self.users_status =
                    format!("roles: {} identit(ies)", self.scenario_roles.len());
            }
            SetupDone::ReadDone(msg) => {
                if let Some(slot) = self.inbox.iter_mut().find(|m| m.id == msg.id) {
                    *slot = msg;
                }
                self.users_status = "marked read".to_string();
            }
        }
    }

    /// Persistent context strip: source → identity → session →
    /// backend phase → capability → next action, plus sync freshness.
    /// Rendered on the toolbar of every island. Role names display
    /// as-is (authorable backend-side); capability derives from the
    /// seat row and commanded hulls, never from name comparison.


    /// The one action the strip points at: auth → hold → phase gate →
    /// orders → assessment. Hints only; gates still refuse loudly.
    fn next_action_hint(&self) -> &str {
        if self.auth_token.is_none() {
            return "Sign in to begin";
        }
        if self.users_game.is_none() {
            return "Choose or join a session";
        }
        match self.users_game_state.as_deref() {
            Some("planning") => "Place units, then assign helm players",
            Some("preparation") => "Declare readiness, then start the exercise",
            Some("execution") => "Issue helm orders",
            Some("closure") => "Review the assessment",
            _ => "Refresh the session status",
        }
    }

    /// Resolve the selected hull's picture, one fetch at a time:
    /// manifest first (no request for hulls without pictures), then
    /// the hull's temporary URL, cached for the session. Runs from
    /// the frame pump, never the render path.
    fn maybe_resolve_hull_image(&mut self) {
        if self.image_op.is_some() || self.auth_token.is_none() {
            return;
        }
        let (master, tok) = match self.users_client() {
            Ok(t) => t,
            Err(_) => return,
        };
        // Read the first manifest immediately, then re-read it on a
        // bounded cadence. The manifest is the only place that can turn
        // a previously settled Unavailable visual into a UnitImage, so
        // a one-shot read would never notice a picture uploaded later in
        // the same session.
        let now = Instant::now();
        let manifest_due = !self.visuals.manifest_loaded
            || self
                .manifest_refresh_at
                .is_none_or(|refresh_at| refresh_at <= now);
        if manifest_due {
            if self
                .manifest_retry_at
                .is_some_and(|retry_at| retry_at > now)
            {
                return;
            }
            self.image_request = Some(ImageRequest::Manifest);
            self.image_op = Some(spawn_rest("pictures", move || {
                master
                    .unit_image_manifest(&tok)
                    .map_err(|e| e.to_string())
                    .map(ImageOut::Manifest)
            }));
            return;
        }
        // The SELECTED unit jumps the queue: it is the one the
        // operator is looking at right now.
        let selected = match self.selection.clone() {
            Some(Selection::Ship(id)) => id.parse::<i64>().ok(),
            _ => None,
        };
        if let Some(uid) = selected {
            if self.visuals.needs_url(uid) {
                self.spawn_url_read(uid, master, tok);
                return;
            }
            if !self.visuals.is_settled(uid) {
                // The manifest is in force and does not list this
                // unit: cached absence, never re-asked. It is still a
                // visual, carrying the symbol and measurements.
                let symbol = self.symbol_for_unit(uid);
                self.visuals.mark_absent(uid, symbol);
                return;
            }
        }
        // Then the rest, one per pump slot. Successful sources remain
        // watched until their texture is decoded; failed sources stay
        // watched but do not block healthy ids during retry delay.
        self.pending_image_urls
            .retain(|unit_id| self.visuals.expects_source(*unit_id));
        if let Some(uid) = self
            .pending_image_urls
            .iter()
            .copied()
            .find(|unit_id| self.visuals.needs_url(*unit_id))
        {
            self.spawn_url_read(uid, master, tok);
        }
    }

    /// One URL read on the picture worker. The URL is temporary, so
    /// this can be re-run for the same unit whenever it expires.
    fn spawn_url_read(&mut self, uid: i64, master: MinosMaster, tok: String) {
        self.pending_image_urls.retain(|u| *u != uid);
        self.image_request = Some(ImageRequest::Url(uid));
        self.image_op = Some(spawn_rest("picture", move || {
            master
                .hull_image_url(&tok, uid)
                .map_err(|e| e.to_string())
                .map(|url| ImageOut::Url(uid, url))
        }));
    }

    /// Apply a finished picture fetch: the manifest (seeding every
    /// listed unit's visual, and dropping stale pictures when the
    /// version moved) or one unit's temporary URL. Failures report and
    /// keep whatever the session already resolved.
    fn apply_image(&mut self, out: ImageOut) {
        match out {
            ImageOut::Manifest(manifest) => {
                // One taxonomy read per manifest, not per entry: keep
                // the SQLite fan-out proportional to the fleet, not
                // to the number of files in the manifest.
                let resolver = self.symbol_resolver();
                let rows: std::collections::HashMap<i64, tfg::store::UnitTaxonomy> = self
                    .store
                    .as_ref()
                    .and_then(|c| tfg::store::fleet_units(c).ok())
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|u| u.id.parse::<i64>().ok().map(|id| (id, u.taxonomy())))
                    .collect();
                let needing =
                    self.visuals
                        .install_manifest(&manifest, |uid| match rows.get(&uid) {
                            Some(tax) => resolver.resolve(*tax),
                            // Not in the mirror: still drawable, as the
                            // generic ship.
                            None => resolver.resolve(tfg::store::UnitTaxonomy::default()),
                        });
                // URLs are read one at a time by the pump, newest
                // selection first; the prefetch ticket widens this.
                // Keep a same-version re-read from discarding work
                // already queued, and drop ids the new ETag removed.
                self.pending_image_urls
                    .retain(|unit_id| self.visuals.expects_source(*unit_id));
                for unit_id in needing {
                    if !self.pending_image_urls.contains(&unit_id) {
                        self.pending_image_urls.push(unit_id);
                    }
                }
                self.manifest_retry_at = None;
                self.manifest_refresh_at =
                    Some(Instant::now() + MANIFEST_REFRESH_INTERVAL);
                // Seed the versioned visual for every mirrored hull,
                // not just the ones the manifest listed. Measurements
                // and symbols belong to units without pictures too.
                self.hydrate_visual_facts();
                // Coverage reads honestly: listed images, and how many
                // live hulls still have none.
                self.users_status = format!(
                    "pictures: {} hull(s) listed, {} without",
                    manifest.entries.len(),
                    manifest.units_without_image
                );
            }
            ImageOut::Url(uid, url) => {
                self.visuals.set_url(uid, url);
                if self.visuals.expects_source(uid)
                    && !self.pending_image_urls.contains(&uid)
                {
                    // Keep the source in the watch list until its
                    // texture is decoded. If the presign expires
                    // first, this same id is re-read rather than
                    // becoming a permanent symbol.
                    self.pending_image_urls.push(uid);
                }
            }
        }
    }

    /// The id-keyed symbol resolver, built from the mirrored
    /// assignment table. An unreadable table yields an empty resolver,
    /// which still draws every unit as the generic ship.
    fn symbol_resolver(&self) -> tfg::store::SymbolResolver {
        let Some(conn) = self.store.as_ref() else {
            return tfg::store::SymbolResolver::default();
        };
        let by_type = tfg::store::unit_type_symbols(conn).unwrap_or_default();
        let by_category = tfg::store::unit_category_symbols(conn).unwrap_or_default();
        let by_domain = tfg::store::unit_movement_domain_symbols(conn).unwrap_or_default();
        tfg::store::SymbolResolver::new(by_type, by_category, by_domain)
    }

    /// Rebuild the image-independent symbol lookup after the taxonomy
    /// mirror changes. This is what lets far symbols render before the
    /// picture manifest has arrived.
    fn reload_unit_symbols(&mut self) {
        self.unit_symbols.clear();
        self.unit_type_ids.clear();
        self.unit_type_names.clear();
        self.unit_type_symbols.clear();
        self.unit_branches.clear();
        let Some(conn) = self.store.as_ref() else {
            return;
        };
        let by_type = tfg::store::unit_type_symbols(conn).unwrap_or_default();
        let by_category = tfg::store::unit_category_symbols(conn).unwrap_or_default();
        let by_domain = tfg::store::unit_movement_domain_symbols(conn).unwrap_or_default();
        let resolver = tfg::store::SymbolResolver::new(
            by_type.clone(),
            by_category,
            by_domain,
        );
        self.unit_type_symbols = by_type.into_iter().collect();
        self.unit_type_names = tfg::store::unit_type_names(conn)
            .unwrap_or_default()
            .into_iter()
            .collect();
        let units = tfg::store::fleet_units(conn).unwrap_or_default();
        self.unit_symbols = units
            .iter()
            .filter_map(|unit| {
                let unit_id = unit.id.parse::<i64>().ok()?;
                if let Some(type_id) = unit.type_id {
                    self.unit_type_ids.insert(unit_id, type_id);
                }
                if let Some(branch) = unit.branch_id {
                    self.unit_branches.insert(unit_id, branch);
                }
                Some((unit_id, resolver.resolve(unit.taxonomy())))
            })
            .collect();
    }

    /// Which session's declarations apply.
    ///
    /// A declaration is scoped to one exercise: "that branch is hostile"
    /// is a statement about THIS exercise, and must not silently apply to
    /// the next one where the operator may be on the other side. A
    /// connected session is keyed on its game id; a local one on the
    /// session log number, which is already the client's per-session
    /// identity.
    fn affiliation_scope(&self) -> String {
        match self.users_game.as_ref() {
            Some((game_id, _)) => format!("game:{game_id}"),
            None => format!("local:{}", self.session_seq),
        }
    }

    /// The facts one affiliation decision reads for a hull: its
    /// mirrored ids, the group it belongs to, and whether this operator
    /// drives it. Built as data so the resolver never sees a name.
    fn affiliation_subject(&self, ship_id: &str) -> tfg::store::AffiliationSubject {
        let group_id = self
            .groups
            .group_list()
            .iter()
            .find(|g| self.groups.group_units(&g.id).iter().any(|u| u == ship_id))
            .map(|g| g.id.clone());
        let unit_id = ship_id.parse::<i64>().ok();
        tfg::store::AffiliationSubject {
            unit_id,
            group_id,
            branch_id: unit_id.and_then(|id| self.unit_branches.get(&id).copied()),
            // The UI's commanded set is the closest thing to the sim's
            // `owned_ids()`, which the sim thread owns. Driving a hull
            // is a fact about the session, not about the world.
            owned: self.controlled.contains(ship_id),
        }
    }

    /// A group's centroid latitude, cover radius in metres and anchor
    /// point, measured from its members' positions — for the gestures
    /// that happen outside a paint frame (a roster focus, a double-click
    /// on a Zone, which carries no radius of its own).
    fn group_extent(&self, gid: &str) -> Option<(f64, f64, (f64, f64))> {
        let positions: Vec<(f64, f64)> = self
            .groups
            .group_units(gid)
            .iter()
            .filter_map(|id| self.registry.position_of(id))
            .map(|p| (p.latitude, p.longitude))
            .collect();
        if positions.len() < 2 {
            return None;
        }
        let extent = projected_group_extent(&positions, self.zoom);
        Some((
            extent.centroid_lat,
            extent.cover_radius_m,
            (extent.centroid_lat, extent.centroid_lon),
        ))
    }

    /// The same aim for a group the operator focused from the roster,
    /// where no symbol has been drawn yet.
    fn zoom_to_show_group(&self, gid: &str) -> Option<f64> {
        let (lat, radius_m, _) = self.group_extent(gid)?;
        Some(
            zoom_for_ground_resolution(lat, radius_m * 0.8 / GROUP_ZONE_MIN_PX).clamp(3.0, 18.0),
        )
    }

    /// What side a GROUP is, for its frame's shape.
    ///
    /// The group's own declaration wins. Failing one, a group is what its
    /// members are **when they agree**: every visible member resolving to
    /// the same affiliation makes the group that affiliation, so a task
    /// force of one declared side does not draw as unknown. Members that
    /// disagree leave the group Unknown — a group that spans both sides
    /// genuinely has no side, and picking the majority here would be the
    /// same invention as picking the fattest hull for its icon.
    ///
    /// Not settled by a decision ticket: the affiliation model fixed the
    /// declaration chain for a UNIT and left a group's own case open.
    fn group_affiliation(&self, gid: &str, members: &[&ShipMarker]) -> tfg::store::Affiliation {
        if let Some(declared) = self.affiliations.by_group.get(gid) {
            return *declared;
        }
        let mut agreed: Option<tfg::store::Affiliation> = None;
        for marker in members {
            let resolved = marker.affiliation;
            match agreed {
                None => agreed = Some(resolved),
                Some(first) if first == resolved => {}
                Some(_) => return tfg::store::Affiliation::Unknown,
            }
        }
        agreed.unwrap_or(tfg::store::Affiliation::Unknown)
    }

    /// Recompute which exercise pieces are unassigned in the tree.
    /// Staged (locally drafted) hulls are unassigned by definition — the
    /// tree only learns of them at a stage advance — so they ride along,
    /// which paints the dashed "nobody's placed me in the organisation"
    /// frame. A staged hull the server already knows and has placed in the
    /// tree keeps the server's answer, or every piece would wear the
    /// dashed frame for as long as the draft holds it.
    fn refresh_unassigned_units(&mut self) {
        let mut set: std::collections::HashSet<i64> = self
            .users_gunits
            .iter()
            .filter(|g| g.hierarchy_node.is_none())
            .map(|g| g.unit_id)
            .collect();
        for hull in self.force.hulls() {
            let known = self
                .users_gunits
                .iter()
                .find(|g| g.unit_id == hull.unit_id)
                .is_some_and(|g| g.hierarchy_node.is_some());
            if !known {
                set.insert(hull.unit_id);
            }
        }
        self.unassigned_units = set;
    }

    /// Reload every affiliation declaration for this session. An empty
    /// resolver is a valid state — most units are undeclared on a first
    /// run and must still resolve, to Unknown.
    fn reload_affiliations(&mut self) {
        let Some(conn) = self.store.as_ref() else {
            return;
        };
        let scope = self.affiliation_scope();
        self.affiliations = tfg::store::affiliation_resolver(conn, &scope).unwrap_or_default();
    }

    /// One declaration, written through to the session's tables, the
    /// resolver reloaded, and — in Live — the change journaled naming
    /// the seat that made it.
    fn declare_affiliation(
        &mut self,
        what: Declaration,
        affiliation: Option<tfg::store::Affiliation>,
    ) {
        let Some(mut conn) = self.store.take() else {
            self.users_status = "affiliation needs the local store".to_string();
            return;
        };
        let scope = self.affiliation_scope();
        let result = match &what {
            Declaration::Unit(id) => {
                tfg::store::set_unit_affiliation(&mut conn, &scope, *id, affiliation)
            }
            Declaration::Group(id) => {
                tfg::store::set_group_affiliation(&mut conn, &scope, id, affiliation)
            }
            Declaration::Branch(id) => {
                tfg::store::set_branch_affiliation(&mut conn, &scope, *id, affiliation)
            }
            Declaration::OurBranch(id) => {
                tfg::store::set_our_branch(&conn, *id, affiliation.is_some())
            }
        };
        self.store = Some(conn);
        match result {
            Ok(()) => {
                self.reload_affiliations();
                let name = match &what {
                    Declaration::Unit(id) => format!("unit {id}"),
                    Declaration::Group(id) => format!("group {id}"),
                    Declaration::Branch(id) => format!("branch {id}"),
                    Declaration::OurBranch(id) => format!("our branch {id}"),
                };
                self.users_status = match affiliation {
                    Some(value) => format!("side: {name} is {}", value.label()),
                    None => format!("side: {name} is not declared"),
                };
                if self.mode.in_live() {
                    if let Some(tx) = self.sim_cmd_tx.as_ref() {
                        let _ = tx.send(SimCommand::NoteAffiliation {
                            actor: self
                                .acting_as
                                .clone()
                                .unwrap_or_else(|| "organizer".to_string()),
                            subject: name,
                            affiliation: affiliation
                                .map(|a| a.as_str().to_string())
                                .unwrap_or_else(|| "undeclared".to_string()),
                        });
                    }
                }
            }
            Err(e) => self.users_status = format!("side declaration failed: {e}"),
        }
    }

    /// One control, three declarations, beside the per-type symbol
    /// editor — which is the precedent: a client-owned, id-keyed mapping
    /// table edited per unit.
    ///
    /// "Not declared" is a real choice beside Unknown, because they are
    /// different states: no declaration invites the "ours" inference (a
    /// unit this operator drives reads Friendly), while a declared
    /// Unknown blocks it. The operator who wants a driven hull to draw
    /// as unknown says so explicitly.
    fn affiliation_editor(&mut self, ui: &mut egui::Ui, ship_id: &str) {
        let subject = self.affiliation_subject(ship_id);
        let resolved = self.affiliations.resolve(&subject);
        let unit_id = subject.unit_id;
        let group_id = subject.group_id.clone();
        let branch_id = subject.branch_id;
        let owned = subject.owned;

        ui.separator();
        ui.label(format!("side · {}", resolved.label()));

        let declared_unit = unit_id.and_then(|id| self.affiliations.by_unit.get(&id).copied());
        if let Some(id) = unit_id {
            Self::affiliation_picker(
                ui,
                "this unit",
                declared_unit,
                |value| self.declare_affiliation(Declaration::Unit(id), value),
            );
        }

        if let Some(group) = group_id.as_ref() {
            let name = self
                .groups
                .group(group)
                .map(|g| g.name.clone())
                .unwrap_or_else(|| group.clone());
            let declared = self.affiliations.by_group.get(group).copied();
            Self::affiliation_picker(
                ui,
                &format!("group {name}"),
                declared,
                |value| self.declare_affiliation(Declaration::Group(group.clone()), value),
            );
        }

        if let Some(branch) = branch_id {
            let label = self
                .store
                .as_ref()
                .and_then(|conn| tfg::store::branch_label(conn, branch))
                .map(|(name, id_name)| {
                    if id_name.is_empty() || id_name == name {
                        name
                    } else {
                        format!("{name} · {id_name}")
                    }
                })
                .unwrap_or_else(|| format!("branch #{branch}"));
            let declared = self.affiliations.by_branch.get(&branch).copied();
            Self::affiliation_picker(
                ui,
                &format!("branch {label} (this exercise)"),
                declared,
                |value| self.declare_affiliation(Declaration::Branch(branch), value),
            );
            let mut ours = self.affiliations.our_branches.contains(&branch);
            if ui.checkbox(&mut ours, format!("{label} is my side")).changed() {
                self.declare_affiliation(
                    Declaration::OurBranch(branch),
                    ours.then_some(tfg::store::Affiliation::Friendly),
                );
            }
        }

        // The chain that produced the answer, so a surprising frame is
        // explainable from where the operator is standing.
        ui.weak(if owned && resolved == tfg::store::Affiliation::Friendly {
            "friendly by inference: this operator drives it, or its branch is mine".to_string()
        } else if resolved == tfg::store::Affiliation::Unknown {
            match branch_id {
                Some(b) => format!("unknown: branch #{b} is neither declared nor mine"),
                None => "unknown: nothing declared".to_string(),
            }
        } else {
            "declared".to_string()
        });
    }

    /// "Not declared" plus the four states.
    fn affiliation_picker(
        ui: &mut egui::Ui,
        what: &str,
        declared: Option<tfg::store::Affiliation>,
        mut apply: impl FnMut(Option<tfg::store::Affiliation>),
    ) {
        egui::ComboBox::from_id_salt(("affiliation", what))
            .selected_text(declared.map(|a| a.label()).unwrap_or("not declared"))
            .show_ui(ui, |ui| {
                if ui.selectable_label(declared.is_none(), "not declared").clicked() {
                    apply(None);
                }
                for value in tfg::store::Affiliation::ALL {
                    if ui.selectable_label(declared == Some(value), value.label()).clicked() {
                        apply(Some(value));
                    }
                }
            });
        ui.weak(what);
    }

    /// Rotate the selected thumbnail without changing Minos' reported
    /// course. This is an operator display override, cleared with the
    /// visual session boundary.
    fn heading_editor(&mut self, ui: &mut egui::Ui, ship_id: &str, reported: Option<f32>) {
        let mut value = self
            .heading_overrides
            .get(ship_id)
            .copied()
            .or(reported)
            .unwrap_or(0.0)
            .rem_euclid(360.0);
        ui.horizontal(|ui| {
            ui.label("thumbnail heading");
            if ui.small_button("−15°").clicked() {
                value = (value - 15.0).rem_euclid(360.0);
                self.heading_overrides.insert(ship_id.to_string(), value);
            }
            if ui.small_button("+15°").clicked() {
                value = (value + 15.0).rem_euclid(360.0);
                self.heading_overrides.insert(ship_id.to_string(), value);
            }
            if self.heading_overrides.contains_key(ship_id) && ui.small_button("clear").clicked() {
                self.heading_overrides.remove(ship_id);
            }
        });
        let had_override = self.heading_overrides.contains_key(ship_id);
        let slider = ui.add(egui::Slider::new(&mut value, 0.0..=360.0).suffix("°"));
        if had_override || slider.changed() {
            self.heading_overrides.insert(ship_id.to_string(), value);
        }
        if let Some(reported) = reported {
            ui.weak(format!("reported course: {reported:.0}°"));
        } else {
            ui.weak("reported course: unknown — override controls the thumbnail");
        }
    }

    /// Assign the selected hull's stable unit type to a far-map
    /// symbol. The display name is only a label; the write is keyed
    /// by `unit_type.id`, so renaming a type never changes its shape.
    fn map_symbol_editor(&mut self, ui: &mut egui::Ui, ship_id: &str) {
        let Ok(unit_id) = ship_id.parse::<i64>() else {
            return;
        };
        let Some(type_id) = self.unit_type_ids.get(&unit_id).copied() else {
            return;
        };
        let current = self.unit_type_symbols.get(&type_id).copied();
        let type_name = self
            .unit_type_names
            .get(&type_id)
            .cloned()
            .unwrap_or_else(|| format!("type #{type_id}"));
        let mut selected = current;
        egui::ComboBox::from_id_salt(("unit-map-symbol", type_id))
            .selected_text(
                current
                    .map(map_symbol_label)
                    .unwrap_or("category/domain fallback"),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected, None, "category/domain fallback");
                for symbol in tfg::store::MapSymbol::ALL {
                    ui.selectable_value(
                        &mut selected,
                        Some(symbol),
                        map_symbol_label(symbol),
                    );
                }
            });
        if selected != current {
            self.set_unit_type_symbol(type_id, selected);
        }
        ui.weak(format!("far symbol · {type_name} (type #{type_id})"));
    }

    fn set_unit_type_symbol(&mut self, type_id: i64, symbol: Option<tfg::store::MapSymbol>) {
        let Some(mut conn) = self.store.take() else {
            self.users_status = "map symbol assignment needs the local store".to_string();
            return;
        };
        let result = tfg::store::set_unit_type_symbol(&mut conn, type_id, symbol);
        self.store = Some(conn);
        match result {
            Ok(()) => {
                self.reload_unit_symbols();
                self.hydrate_visual_facts();
                self.users_status = match symbol {
                    Some(symbol) => format!("map symbol: {}", map_symbol_label(symbol)),
                    None => "map symbol: category/domain fallback".to_string(),
                };
            }
            Err(e) => self.users_status = format!("map symbol assignment failed: {e}"),
        }
    }

    /// Resolve one mirrored hull's far-map symbol from stable taxonomy
    /// ids. An unmirrored hull still gets the generic ship.
    fn symbol_for_unit(&self, unit_id: i64) -> tfg::store::MapSymbol {
        if let Some(symbol) = self.unit_symbols.get(&unit_id) {
            return *symbol;
        }
        let tax = self
            .store
            .as_ref()
            .and_then(|conn| tfg::store::fleet_unit(conn, &unit_id.to_string()).ok())
            .flatten()
            .map(|u| u.taxonomy())
            .unwrap_or_default();
        self.symbol_resolver().resolve(tax)
    }

    /// Merge local unit facts into the active asset version. This is
    /// safe to call after a sync, a spec backfill, or a manifest read:
    /// it updates measurements and symbols without disturbing URLs or
    /// textures already resolved for that same version.
    fn hydrate_visual_facts(&mut self) {
        if !self.visuals.manifest_loaded {
            return;
        }
        let Some(conn) = self.store.as_ref() else {
            return;
        };
        let resolver = self.symbol_resolver();
        let units = tfg::store::fleet_units(conn).unwrap_or_default();
        let figures: std::collections::HashMap<i64, tfg::store::StoredFigures> =
            tfg::store::current_figures(conn)
                .unwrap_or_default()
                .into_iter()
                .map(|facts| (facts.unit_id, facts))
                .collect();
        for unit in units {
            let Ok(unit_id) = unit.id.parse::<i64>() else {
                continue;
            };
            let symbol = resolver.resolve(unit.taxonomy());
            let facts = figures.get(&unit_id);
            self.visuals.set_unit_facts(
                unit_id,
                symbol,
                facts.and_then(|f| f.loa_m),
                facts.and_then(|f| f.beam_m),
                facts.and_then(|f| f.draft_m),
            );
        }
    }

    /// Compact, resizable Inspector image sizing. The width is global
    /// for the session, while the actual draw size still follows the
    /// current Inspector width and the source aspect ratio.
    fn inspector_image_size_control(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("image width");
            let mut width = self.inspector_image_width;
            let response = ui.add(
                egui::Slider::new(
                    &mut width,
                    INSPECTOR_IMAGE_MIN_WIDTH..=INSPECTOR_IMAGE_MAX_WIDTH,
                )
                .suffix(" pt"),
            );
            if response.changed() {
                self.inspector_image_width = width;
            }
        });
    }

    /// Ask egui for a source texture and keep its renderable identity
    /// in the versioned visual. Presigned addresses are acquisition
    /// steps, not durable identity: once decoded, the texture outlives
    /// the URL that obtained it.
    fn show_visual_source(
        &mut self,
        ui: &mut egui::Ui,
        unit_id: i64,
        source: &str,
        width_px: Option<u32>,
        height_px: Option<u32>,
    ) {
        let hint = width_px
            .filter(|width| *width > 0)
            .map(egui::load::SizeHint::Width)
            .unwrap_or_else(|| egui::load::SizeHint::Scale(1.0.into()));
        let manifest_size = egui::vec2(
            width_px.filter(|value| *value > 0).unwrap_or(1) as f32,
            height_px.filter(|value| *value > 0).unwrap_or(1) as f32,
        );
        match ui.ctx().try_load_texture(
            source,
            egui::TextureOptions::LINEAR,
            hint,
        ) {
            Ok(egui::load::TexturePoll::Ready { texture }) => {
                self.visuals.set_texture(unit_id, texture);
                let display_size = inspector_image_size(
                    texture.size,
                    self.inspector_image_width,
                    ui.available_width(),
                );
                ui.add(egui::Image::from_texture(texture).fit_to_exact_size(display_size));
            }
            Ok(egui::load::TexturePoll::Pending { .. }) | Err(_) => {
                // The loader owns the retry and loading state. A
                // second request under the same URI is deduplicated.
                let display_size = inspector_image_size(
                    manifest_size,
                    self.inspector_image_width,
                    ui.available_width(),
                );
                ui.add(egui::Image::from_uri(source).fit_to_exact_size(display_size));
            }
        }
    }

    /// Start texture loads for Middle/Near markers. This is a loader
    /// hand-off only: the map itself still paints through `Painter`
    /// meshes and never calls `ui.image` on the map path.
    fn load_marker_textures(&mut self, ctx: &egui::Context, markers: &[ShipMarker]) {
        for marker in markers {
            if marker.lod == UnitLod::Far {
                continue;
            }
            let Ok(unit_id) = marker.id.parse::<i64>() else {
                continue;
            };
            let Some(visual) = self.visuals.get(unit_id) else {
                continue;
            };
            if visual.texture.is_some() || visual.asset_kind != AssetKind::UnitImage {
                continue;
            }
            let Some(source) = visual.image_url.clone() else {
                continue;
            };
            if visual
                .image_url_retry_at
                .is_some_and(|retry_at| retry_at > Instant::now())
                || visual
                    .image_url_expires_at
                    .is_some_and(|expires_at| expires_at <= Instant::now())
            {
                continue;
            }
            let hint = visual
                .width_px
                .filter(|width| *width > 0)
                .map(egui::load::SizeHint::Width)
                .unwrap_or_else(|| egui::load::SizeHint::Scale(1.0.into()));
            if let Ok(egui::load::TexturePoll::Ready { texture }) =
                ctx.try_load_texture(&source, egui::TextureOptions::LINEAR, hint)
            {
                self.visuals.set_texture(unit_id, texture);
            }
        }
    }

    /// Display size in points: what overlays project against. The
    /// renderer works in physical pixels; both derive per frame.
    fn map_dims(&self) -> (f64, f64) {
        self.map_view
    }

    fn update_unit_drag(&mut self, ui: &egui::Ui) {
        let Some(drag) = self.unit_drag.as_mut() else {
            return;
        };
        let pointer = ui.input(|input| input.pointer.interact_pos());
        if ui.input(|input| input.pointer.any_down()) {
            if let Some(pointer) = pointer {
                if pointer.distance(drag.start) > 6.0 {
                    drag.moved = true;
                }
            }
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    }

    fn reconcile_helm_preview(&mut self, id: &str, animation_fraction: f64) {
        if !self.helm_preview_pending.contains(id) || animation_fraction < 1.0 {
            return;
        }
        let Some(draft) = self.helm_drafts.get(id).map(|draft| draft.heading_deg) else {
            self.helm_preview_pending.remove(id);
            return;
        };
        let Some(latest) = self
            .registry
            .ship(id)
            .and_then(|ship| ship.latest.heading_deg)
        else {
            self.helm_preview_pending.remove(id);
            return;
        };
        let delta = (draft - latest).rem_euclid(360.0);
        if delta.min(360.0 - delta) <= 1.0 {
            self.helm_preview_pending.remove(id);
        }
    }

    fn map_preview_heading(
        &self,
        id: &str,
        authoritative_heading: Option<f32>,
    ) -> Option<f32> {
        let accepted = matches!(
            self.order_result.get(id),
            Some(HelmOrderUiResult::Accepted | HelmOrderUiResult::Clamped { .. })
        );
        let draft = self.helm_drafts.get(id).map(|draft| draft.heading_deg);
        if self.helm_preview_pending.contains(id) && self.action_allows(id) {
            draft.or(authoritative_heading)
        } else if accepted {
            authoritative_heading.or(draft)
        } else {
            draft
                .or_else(|| self.heading_overrides.get(id).copied())
                .or(authoritative_heading)
        }
    }

    fn map_heading_target(
        &self,
        markers: &[ShipMarker],
        px: f64,
        py: f64,
        pixels_per_point: f32,
    ) -> Option<String> {
        let id = match self.selection.as_ref() {
            Some(Selection::Ship(id)) => id,
            _ => return None,
        };
        if !self.action_allows(id)
            || !(self.controlled.contains(id) || self.minos_order_target(id).is_some())
        {
            return None;
        }
        markers
            .iter()
            .find(|marker| {
                !self.hidden.contains(&marker.id)
                    && marker.id == *id
                    && marker_body_hit(marker, px, py, self.zoom, pixels_per_point)
            })
            .map(|marker| marker.id.clone())
    }

    /// Setup counterpart to `map_heading_target`: which placed hull's
    /// body the Setup-phase pointer is over, to be lifted and moved.
    /// Keyed like the heading handle — hit the marker's pixel body, not
    /// its label or a surrounding water pixel.
    fn map_move_target(
        &self,
        markers: &[ShipMarker],
        px: f64,
        py: f64,
        pixels_per_point: f32,
        pos: egui::Pos2,
    ) -> Option<UnitMoveDrag> {
        if self.mode.phase != Phase::Setup {
            return None;
        }
        markers
            .iter()
            .find(|marker| {
                !self.hidden.contains(&marker.id)
                    && marker
                        .id
                        .parse::<i64>()
                        .ok()
                        .and_then(|uid| self.force.get(uid))
                        .is_some_and(|hull| hull.start.is_some())
                    && marker_body_hit(marker, px, py, self.zoom, pixels_per_point)
            })
            .and_then(|marker| {
                let uid = marker.id.parse::<i64>().ok()?;
                let hull = self.force.get(uid)?;
                Some(UnitMoveDrag {
                    unit_id: uid,
                    name: hull.name.clone(),
                    start: pos,
                    moved: false,
                })
            })
    }

    /// Commit a move-drag when the pointer is released. Only a drag that
    /// actually moved past the dead-zone commits; a release within a few
    /// pixels is the ordinary map click and selects whatever is there.
    fn finish_unit_move(&mut self, ui: &egui::Ui, rect: egui::Rect) {
        let Some(m) = self.map_unit_move.take() else { return };
        if !m.moved {
            return;
        }
        // A genuine drag just ended, whichever way it commits: the pointer
        // came up, so the map click that rides the same release must not
        // turn into a select (or a deselect, when it lands on empty
        // water). The click block consumes and clears this flag.
        self.released_move_drag = true;
        let Some(pos) = ui.input(|input| input.pointer.interact_pos()) else { return };
        if !rect.contains(pos) || !drop_lands_on_map(&self.modal_panel_rects, pos) {
            self.note_placement(format!("{} move cancelled — release over the map", m.name));
            return;
        }
        let (mw, mh) = self.map_dims();
        let (la, lo) = unproject_mercator(
            (pos.x - rect.min.x) as f64,
            (pos.y - rect.min.y) as f64,
            self.center,
            self.zoom,
            mw,
            mh,
        );
        self.move_placement(m.unit_id, &m.name, la, lo);
    }

    fn update_map_heading_draft(&mut self, id: &str, heading_deg: f32) {
        let speed_kn = self
            .helm_drafts
            .get(id)
            .map(|draft| draft.speed_kn)
            .or_else(|| {
                self.registry
                    .ship(id)
                    .and_then(|ship| ship.latest.speed_kn)
            })
            .unwrap_or(0.0);
        self.helm_drafts.insert(
            id.to_string(),
            HelmDraft {
                heading_deg: heading_deg.rem_euclid(360.0),
                speed_kn,
            },
        );
        if matches!(self.order_result.get(id), Some(HelmOrderUiResult::Pending)) {
            self.order_result
                .insert(id.to_string(), HelmOrderUiResult::Superseded);
            self.helm_submissions.remove(id);
        } else {
            self.order_result
                .insert(id.to_string(), HelmOrderUiResult::Draft);
        }
    }

    /// Pointer-free map path (blocking ticket): cycle ships and
    /// groups, follow, waypoint-at-center, island toggles, deselect.
    /// Runs inside the focus contract — never while typing. Commit
    /// stays on the (Tab-reachable) order button by design: the key
    /// aims, the button fires, and the water check still refuses.
    fn keyboard_map_path(&mut self, ctx: &egui::Context) {
        let mut ids: Vec<String> = self
            .markers(ctx.pixels_per_point())
            .iter()
            .map(|m| m.id.clone())
            .collect();
        ids.sort();
        let step = if ctx.input(|i| i.key_pressed(egui::Key::OpenBracket)) {
            Some(-1i64)
        } else if ctx.input(|i| i.key_pressed(egui::Key::CloseBracket)) {
            Some(1i64)
        } else {
            None
        };
        if let (Some(dir), true) = (step, !ids.is_empty()) {
            let cur = match &self.selection {
                Some(Selection::Ship(id)) => ids.iter().position(|x| x == id).unwrap_or(0),
                _ => {
                    if dir < 0 {
                        0
                    } else {
                        ids.len() - 1
                    }
                }
            };
            let next = (cur as i64 + dir).rem_euclid(ids.len() as i64) as usize;
            // Selecting opens the Inspector like a marker click.
            self.select_ship(ids[next].clone());
        }
        if ctx.input(|i| i.key_pressed(egui::Key::G)) {
            let mut gids: Vec<String> =
                self.groups.group_list().iter().map(|g| g.id.clone()).collect();
            gids.sort();
            if !gids.is_empty() {
                let cur = match &self.selection {
                    Some(Selection::Group(id)) => {
                        gids.iter().position(|x| x == id).unwrap_or(0)
                    }
                    _ => gids.len() - 1,
                };
                self.select_group(gids[(cur + 1) % gids.len()].clone());
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F)) {
            if let Some(Selection::Ship(id)) = self.selection.clone() {
                self.following = Some(id);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::W)) {
            // Waypoint at the view center — the order button commits.
            self.pending_waypoint = Some(self.center);
            self.placing = false;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::O)) {
            self.show_orders = true;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::R)) {
            self.show_roster = !self.show_roster;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.unit_drag.take().is_some()
                || (self.mode.tool == SetupTool::Place && self.fleet_pick.is_some())
            {
                self.clear_fleet_pick();
                self.users_status = "placement cancelled".to_string();
            } else {
                self.deselect();
            }
        }
    }



    /// Apply finished map frames (last-writer-wins by sequence). The
    /// camera stays authoritative: the texture only records what it was
    /// rendered for (center/zoom/size) so the canvas can translate it
    /// while a fresher tile is in flight. One texture, updated in place.
    fn drain_map(&mut self, ctx: &egui::Context) {
        for (seq, center, zoom, size, img) in self.map_resp_rx.try_iter() {
            self.dirty = true;
            if seq == self.map_seq {
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
    /// camera eases now; the texture catches up translated underneath.
    fn request_frame(&mut self, ship: &str, at: (f64, f64)) {
        let Some(tx) = self.map_req_tx.clone() else {
            return; // shutting down
        };
        self.map_seq += 1;
        self.recentering = Some(ship.to_string());
        let goal = self.visible_center(at);
        self.glide_to(goal);
        eprintln!("recentering on {ship}…");
        let _ = tx.send((self.map_seq, goal, self.zoom, self.map_px, JUMP_PUMP));
    }

    /// The geographic point that should sit at the middle of the map's
    /// *visible* area, given that `at` should sit at the middle of the map.
    ///
    /// The side zone floats over the map rather than insetting it, so the
    /// window's centre and the visible centre are different points. Without
    /// this, every "recentre on this hull" would park the hull underneath the
    /// chrome whenever the zone is showing on that side.
    ///
    /// The shift is computed in pixels and unprojected rather than guessed
    /// in degrees, because a degree offset is only correct at one zoom. The
    /// zone's half-width is the shift, and it is positive when the zone is on
    /// the left so that the visible centre moves right.
    fn visible_center(&self, at: (f64, f64)) -> (f64, f64) {
        let shift_px =
            tfg::chrome::camera_centre_offset(self.side_dock, !self.show_side_zone);
        if shift_px == 0.0 {
            return at;
        }
        let (w, h) = self.map_dims();
        // The pixel sampled is the one that should END UP at the map's
        // centre, not the shift itself. `unproject_mercator` takes a
        // viewport-relative pixel, so the visible centre of a map whose left
        // `s` pixels are covered is at `w/2 + s` — and passing `s` directly
        // would move the centre the wrong way by the map's own width.
        tfg::map_render::unproject_mercator(
            (w as f64 / 2.0) + shift_px as f64,
            (h / 2.0) as f64,
            at,
            self.zoom,
            w as f64,
            h as f64,
        )
    }

    /// Eased camera move. Retargeting mid-flight restarts the ramp from
    /// where the camera currently is, so clicking a second hull during a
    /// glide continues from there instead of jumping.
    fn glide_to(&mut self, at: (f64, f64)) {
        self.glide.retarget(at);
        self.center = self.glide.at;
    }

    /// Direct manipulation — a pan or an anchor-preserving zoom — must be
    /// 1:1 with the pointer, so it bypasses the easing entirely. `step`
    /// rewrites `center` from `glide` every frame, so the ramp has to be
    /// cleared here or the next frame would drag the camera back.
    fn snap_camera(&mut self, at: (f64, f64)) {
        self.glide.snap(at);
        self.center = at;
    }

    /// Follow-tracking frame (task #45): light pump, no recenter label.
    /// The camera tracks now; the texture follows translated.
    fn track_frame(&mut self, ship: &str, at: (f64, f64)) {
        let Some(tx) = self.map_req_tx.clone() else {
            return; // shutting down
        };
        self.map_seq += 1;
        self.recentering = Some(ship.to_string());
        self.glide_to(at);
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
        // Chase the goal, not the current position: during a glide the
        // camera is still travelling, and rendering where it is going lets
        // the texture resolve in one request instead of one per frame.
        let _ = tx.send((self.map_seq, self.glide.goal, self.zoom, self.map_px, pump));
    }

    /// Zoom step: clamps, re-renders, and reports. Each group's own
    /// extent decides where it becomes a Zone, so there is no shared
    /// threshold for the map to cross.
    fn zoom_by(&mut self, delta: f64) {
        self.zoom = (self.zoom + delta).clamp(3.0, 18.0);
        eprintln!("zoom {:.0}", self.zoom);
        self.refresh_map();
    }

    /// Select a ship: clears any group selection and opens the Inspector.
    fn select_ship(&mut self, id: String) {
        self.order_result
            .entry(id.clone())
            .or_insert(HelmOrderUiResult::Draft);
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
        self.map_heading_drag = None;
    }

    /// Group display name + member units for any group id.
    fn group_info(&self, gid: &str) -> Option<(String, Vec<String>)> {
        let g = self.groups.group(gid)?;
        Some((g.name.clone(), self.groups.group_units(&g.id)))
    }

    /// Authority the acting identity holds over these units: the
    /// organizer commands all; others take their highest covering rank
    /// over the set (deeper outranks shallower), falling back to unit
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
        } else if a == Authority::OPERASI_GABUNGAN {
            "operasi gabungan"
        } else if a == Authority::GUGUS {
            "gugus"
        } else if a == Authority::SATGAS {
            "satgas"
        } else if a == Authority::UNSUR {
            "unsur"
        } else {
            "unit"
        }
    }

    /// Draw a quiet geographic reference grid in map screen space. The map
    /// engine remains tile-only; this is an overlay helper, not a style layer.
    fn paint_map_grid(&self, painter: &egui::Painter, rect: egui::Rect) {
        let spacing = grid_spacing_deg(self.zoom);
        let (mw, mh) = self.map_dims();
        let west = unproject_mercator(0.0, 0.0, self.center, self.zoom, mw, mh).1;
        let east = unproject_mercator(mw, 0.0, self.center, self.zoom, mw, mh).1;
        let north = unproject_mercator(0.0, 0.0, self.center, self.zoom, mw, mh).0;
        let south = unproject_mercator(0.0, mh, self.center, self.zoom, mw, mh).0;

        let minor = egui::Color32::from_rgba_unmultiplied(100, 116, 139, 90);
        let major = egui::Color32::from_rgba_unmultiplied(100, 116, 139, 150);
        let first_lon = ((west / spacing).floor() as i64 - 1) as f64 * spacing;
        let last_lon = (((east / spacing).ceil() as i64 + 1) as f64) * spacing;
        let mut lon_index = (first_lon / spacing).round() as i64;
        let mut lon = first_lon;
        while lon <= last_lon {
            let (px, _) = project_mercator(0.0, lon, self.center, self.zoom, mw, mh);
            let color = if lon_index.rem_euclid(5) == 0 { major } else { minor };
            painter.line_segment(
                [
                    rect.left_top() + egui::vec2(px as f32, 0.0),
                    rect.left_top() + egui::vec2(px as f32, mh as f32),
                ],
                egui::Stroke::new(1.0, color),
            );
            lon += spacing;
            lon_index += 1;
        }

        let first_lat = ((south / spacing).floor() as i64 - 1) as f64 * spacing;
        let last_lat = (((north / spacing).ceil() as i64 + 1) as f64) * spacing;
        let mut lat_index = (first_lat / spacing).round() as i64;
        let mut lat = first_lat;
        while lat <= last_lat {
            let (_, py) = project_mercator(lat, 0.0, self.center, self.zoom, mw, mh);
            let color = if lat_index.rem_euclid(5) == 0 { major } else { minor };
            painter.line_segment(
                [
                    rect.left_top() + egui::vec2(0.0, py as f32),
                    rect.left_top() + egui::vec2(mw as f32, py as f32),
                ],
                egui::Stroke::new(1.0, color),
            );
            lat += spacing;
            lat_index += 1;
        }
    }

    /// Display name for a placed unit: labels captured at placement
    /// (either source), asset fleet lookup, or the raw id.
    fn unit_label(&self, id: &str) -> String {
        if let Some((name, hull)) = self.placed_labels.get(id) {
            return format!("{name} ({hull})");
        }
        self.fleet
            .get(id)
            .map(|u| format!("{} ({})", u.name, u.hull))
            .unwrap_or_else(|| id.to_string())
    }

    /// Map labels prefer a human unit name, then its hull number, and
    /// use the raw id only when both are absent.
    fn map_label(
        &self,
        id: &str,
        live_name: Option<&str>,
        live_hull: Option<&str>,
    ) -> String {
        let (fallback_name, fallback_hull) = self
            .placed_labels
            .get(id)
            .cloned()
            .or_else(|| self.fleet.get(id).map(|u| (u.name.clone(), u.hull.clone())))
            .unwrap_or_default();
        live_name
            .and_then(nonempty)
            .or_else(|| nonempty(&fallback_name))
            .or_else(|| live_hull.and_then(nonempty))
            .or_else(|| nonempty(&fallback_hull))
            .unwrap_or_else(|| id.to_string())
    }

    /// Resolve a picked hull to (name, hull, catalog class id for sim
    /// stats). Asset seeds carry their class; register rows resolve by
    /// the Minos class id first (H10: a bundled asset sharing the name
    /// must never shadow synced figures), falling back to the name
    /// match for unsynced hulls. None means no sim stats, and placement
    /// refuses loudly rather than inventing abilities.
    fn placement_seed(&self, id: &str) -> Option<(String, String, Option<String>)> {
        if let Some(u) = self.fleet.get(id) {
            return Some((u.name.clone(), u.hull.clone(), Some(u.class_id.clone())));
        }
        let conn = self.store.as_ref()?;
        let row = tfg::store::fleet_unit(conn, id).ok()??;
        let class = self
            .catalog
            .find_runtime_class(row.class_id)
            .map(|c| c.id.clone())
            .or_else(|| self.catalog.find_class_by_name(&row.class_name).map(|c| c.id.clone()));
        Some((row.name, row.hull, class))
    }

    fn placement_valid(&self, la: f64, lo: f64) -> bool {
        self.land
            .as_ref()
            .map(|land| land.is_water(&GeoPosition { latitude: la, longitude: lo }))
            .unwrap_or(true)
    }

    /// Shared place core (click + drag-and-drop): the click path's
    /// guards, then take-control for a picked, unplaced hull with stats.
    ///
    /// LOCAL FIRST (force draft): the drop goes straight into `self.force`
    /// and the local drop — no backend write in the gesture, and no
    /// commander to name on the way in. Minos catches up at a stage
    /// advance, which is the one place the server's commander rule is
    /// written as a rule instead of a blocker.
    fn try_place_picked(&mut self, la: f64, lo: f64) -> Option<String> {
        if !(self.mode.phase == Phase::Setup || self.mode.phase == Phase::Live) {
            return None;
        }
        if !self.mode.armed.load(Ordering::SeqCst) {
            self.feed("place refused: engine disarmed".to_string());
            self.note_placement("place refused: engine disarmed");
            return None;
        }
        if self.acting_as.is_some() {
            return None;
        }
        if !self.placement_valid(la, lo) {
            self.note_placement("placement needs water");
            return None;
        }
        let pid = self.fleet_pick.clone()?;
        let (name, hull, class_id) = self.placement_seed(&pid)?;
        // A hull already placed is not a second placement — it is a move.
        if self.placed_fleet.contains(&pid) {
            return None;
        }
        match class_id {
            Some(class_id) => {
                let Ok(uid) = pid.parse::<i64>() else {
                    let msg = format!("place refused: {name} is not a register hull");
                    self.feed(msg.clone());
                    self.users_status = msg;
                    return None;
                };
                // Carry a commander the Player picker already named, so
                // re-placing a hull does not strip one.
                let commander_id = self.force.get(uid).and_then(|h| h.commander_id);
                let mut entry = self
                    .force
                    .get(uid)
                    .cloned()
                    .unwrap_or_else(|| {
                        tfg::force::DraftHull::new(
                            uid,
                            name.clone(),
                            hull.clone(),
                            class_id.clone(),
                        )
                    });
                entry.name = name.clone();
                entry.hull_number = hull.clone();
                entry.class_id = class_id.clone();
                entry.start = Some(tfg::force::Start { lat: la, lon: lo });
                entry.commander_id = commander_id;
                self.force.upsert(entry);
                self.apply_local_drop(&pid, &pid, &name, &hull, &class_id, la, lo);
                Some(pid)
            }
            None => {
                let msg = format!("place refused: no sim stats for {name}");
                self.feed(msg.clone());
                self.users_status = msg;
                None
            }
        }
    }

    /// The local half of a placement (shared by the sandbox drop and
    /// the deferred Minos drop): TakeControl, ownership, labels, and
    /// the status line with its spec authority.
    fn apply_local_drop(
        &mut self,
        id: &str,
        pid: &str,
        name: &str,
        hull: &str,
        class_id: &str,
        la: f64,
        lo: f64,
    ) {
        if let Some(tx) = &self.sim_cmd_tx {
            let _ = tx.send(SimCommand::TakeControl {
                ship_id: id.to_string(),
                pos: GeoPosition { latitude: la, longitude: lo },
                class_id: class_id.to_string(),
            });
        }
        // Placed units arrive owned (Q3): the gesture is the
        // take-control, no second step.
        self.controlled.insert(id.to_string());
        self.select_ship(id.to_string());
        self.placed_labels.insert(id.to_string(), (name.to_string(), hull.to_string()));
        self.placed_fleet.insert(pid.to_string());
        self.clear_fleet_pick();
        self.refresh_unassigned_units();
        // The event feed only renders in an execution window;
        // the setup flow owns its own status line too. The spec
        // source rides the message (H10): Minos figures name
        // their version, bundled rows name the asset.
        let authority = self
            .catalog
            .class(class_id)
            .map(|c| {
                if c.version > 0 {
                    format!("{} v{}", Catalog::class_source(c), c.version)
                } else {
                    Catalog::class_source(c).to_string()
                }
            })
            .unwrap_or_else(|| "unknown class".to_string());
        let msg = format!(
            "placed {name} ({hull}) at ({la:.4}, {lo:.4}) · {authority} · exercise: {} placed, {} to go",
            self.force.hulls().filter(|h| h.start.is_some()).count(),
            self.force
                .len()
                .saturating_sub(self.force.hulls().filter(|h| h.start.is_some()).count()),
        );
        self.feed(msg.clone());
        self.users_status = msg;
    }

    /// Drop one hull's local half: sim ship, control, orders view,
    /// labels, fleet membership, pick, selection, and follow. Every
    /// removal path funnels here so the map never shows a controllable
    /// hull Minos calls unplaced.
    fn release_hull(&mut self, id: &str) {
        if let Some(tx) = &self.sim_cmd_tx {
            let _ = tx.send(SimCommand::Release { ship_id: id.to_string() });
        }
        self.controlled.remove(id);
        self.order_views.remove(id);
        self.helm_drafts.remove(id);
        self.helm_preview_pending.remove(id);
        self.fix_animation_started.remove(id);
        self.placed_labels.remove(id);
        self.placed_fleet.remove(id);
        if self.fleet_pick.as_deref() == Some(id) {
            self.clear_fleet_pick();
        }
        if self.selection == Some(Selection::Ship(id.to_string())) {
            self.deselect();
        }
        if self.following.as_deref() == Some(id) {
            self.following = None;
        }
    }

    /// Socket URL for a Minos REST base (live-wire ticket): same host,
    /// wss without port on https, ws :8000 on http (transport ticket).
    fn ws_url_for(minos_base: &str) -> String {
        let (scheme, rest) = match minos_base.split_once("://") {
            Some((s, r)) => (s, r),
            None => ("http", minos_base),
        };
        let hostport = rest.split('/').next().unwrap_or(rest);
        let host = hostport.split(':').next().unwrap_or(hostport);
        if scheme == "https" {
            format!("wss://{host}/connection/websocket")
        } else {
            format!("ws://{host}:8000/connection/websocket")
        }
    }

    /// Load optional `.env` files: working directory first for developer
    /// workflows, then the installed app data directory. Real environment
    /// variables always win — files only fill gaps.
    fn load_dotenv(paths: Option<&AppPaths>) {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::current_dir().ok().map(|dir| dir.join(".env")) {
            candidates.push(path);
        }
        if let Some(paths) = paths {
            candidates.push(paths.data_dir.join(".env"));
        }
        for path in candidates {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                let Some((k, v)) = line.split_once('=') else {
                    continue;
                };
                let (k, v) = (k.trim(), v.trim());
                if k.is_empty() || std::env::var(k).is_ok() {
                    continue;
                }
                let v = v
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                    .unwrap_or(v);
                // Tab: set_var is unsafe under edition 2024 (process-wide).
                unsafe { std::env::set_var(k, v) };
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
                            Some(uid) if self.placed_labels.contains_key(uid) => {
                                self.unit_label(uid)
                            }
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

    /// Start action: default windows, fresh per-session journal, 24:1 clock, armed.
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
        // Ratio derived from windows (grill #23): game span over real
        // span — for the sandbox. A connected execution seeds from the
        // Minos rate instead (H2): the windows never armed a Minos
        // clock, and an independent local ratio would drift the display
        // from the assumed times the server stamps.
        let ratio = match (self.users_game_state.as_deref(), self.minos_time_factor) {
            (Some("execution"), Some(f)) => f,
            _ => (ge - gs).num_seconds() as f64 / (re - rs).num_seconds() as f64,
        };
        self.session_ratio = ratio;
        self.session_seq += 1;
        let path = self
            .paths
            .log_dir
            .join(format!("tfg-session-log-{}.jsonl", self.session_seq));
        if let Some(tx) = &self.sim_cmd_tx {
            let _ = tx.send(SimCommand::RotateJournal { path: path.clone() });
            let _ = tx.send(SimCommand::SetClockRatio { ratio });
            // M6: Live entry unfreezes the Setup hold.
            let _ = tx.send(SimCommand::SetPaused { paused: false });
        }
        self.session_log_path = path;
        self.mode.start();
        // Going live opens map + strip + ONE contextual island; the
        // rest stay available but closed. The Inspector still opens on
        // first selection, new mail still pops Messages once.
        self.show_roster = false;
        self.show_orders = false;
        self.show_log = false;
        self.show_messages = false;
        self.live_default_islands();
        eprintln!("session live at {ratio:.1}x");
    }

    /// Live entry's single island, from the seat — never from role
    /// names (authorable backend-side). Judge-side watches the
    /// record; hull commanders get orders; everyone else (organizer,
    /// observer, unseated) gets the roster with readiness control.
    fn live_default_islands(&mut self) {
        let judge = self.own_roster_row().is_some_and(|p| p.judge);
        let commands = !self.commanded_hulls.is_empty()
            || self.users_gunits.iter().any(|g| {
                self.auth_user_id.is_some_and(|me| g.commander_id == Some(me))
            });
        if judge {
            self.show_log = true;
        } else if commands {
            self.show_orders = true;
        } else {
            self.show_roster = true;
        }
    }

    /// A session exists once started (task #40): working islands unlock
    /// off Live, and lock again when the session ends or resets.
    fn session_live(&self) -> bool {
        self.mode.phase == Phase::Live
    }

    /// H1: the Eval predicate, so closure writes can guard the local end.
    fn session_closed(&self) -> bool {
        self.mode.phase == Phase::Closed
    }

    /// Lock the working islands (task #40).
    fn close_working_islands(&mut self) {
        self.show_roster = false;
        self.show_orders = false;
        self.show_log = false;
        self.show_messages = false;
    }

    /// Directory scan off-thread: the data-directory listing leaves the
    /// frame; the loaded flag sets at spawn so one scan runs per open,
    /// and refresh re-arms it.
    fn refresh_log_files(&mut self) -> bool {
        if self.log_op.is_some() {
            self.users_status = "log scan already running…".to_string();
            return false;
        }
        let log_dir = self.paths.log_dir.clone();
        self.log_op = Some(spawn_rest("logdir", move || {
            Ok(LogOut::Files(Self::session_log_files(&log_dir)))
        }));
        true
    }

    /// Journal parse off-thread: full read + structured parse +
    /// replay events in one worker trip. The path pins at spawn; a
    /// changed selection drops the stale arrival.
    fn load_log_view(&mut self, path: std::path::PathBuf) {
        if self.log_op.is_some() {
            self.users_status = "log parse already running…".to_string();
            return;
        }
        self.log_view_path = Some(path.clone());
        self.log_view = None;
        self.log_op = Some(spawn_rest("log", move || {
            let (entries, units, players) = Self::read_log_view(&path);
            let replay = Self::parse_replay(&path);
            Ok(LogOut::View(path, LogViewData { entries, units, players, replay }))
        }));
    }

    /// Past session journals on disk, oldest first.
    fn session_log_files(log_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(log_dir) {
            for e in entries.flatten() {
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
                    // Concise row (harden): kind + actor + ship, never the
                    // raw payload dump — long JSON stretched islands and
                    // buried the signal. Full detail stays in the file.
                    text: {
                        let ship = ships.first().cloned().unwrap_or_default();
                        let extra = if ships.len() > 1 {
                            format!(" +{}", ships.len() - 1)
                        } else {
                            String::new()
                        };
                        format!(
                            "{} {} {}: {}{}",
                            v["game_ts"].as_str().unwrap_or("—"),
                            actor,
                            v["kind"].as_str().unwrap_or("?"),
                            ship,
                            extra,
                        )
                    },
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

    /// End action: disarm into Closed, lock working islands, and
    /// freeze the transcript tail — the tail read leaves the frame
    /// on the log worker and lands a moment later.
    fn end_session(&mut self) {
        self.mode.end();
        self.close_working_islands();
        // Closed clears selection: the map freezes under its transcript,
        // clicks go inert, the Inspector shuts.
        self.deselect();
        self.transcript.clear();
        let path = self.session_log_path.clone();
        if self.log_op.is_none() {
            self.log_op = Some(spawn_rest("transcript", move || {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
                let n = lines.len();
                Ok(LogOut::Transcript(
                    lines.into_iter().skip(n.saturating_sub(200)).collect(),
                ))
            }));
        }
        eprintln!("session ended");
    }


    /// Group overlay geometry for this frame: a Zone per group whose
    /// ground is big enough to read as a shape, and a framed symbol per
    /// group whose extent is too small for one.
    ///
    /// The Representation IS this split — there is no third return value
    /// carrying it, because a painter that can see which list a group is
    /// in has the answer.
    fn group_geometry(
        &mut self,
        markers: &[ShipMarker],
    ) -> (Vec<ZoneGeom>, Vec<GroupSymbolGeom>) {
        // Higher ranks first so lower-rank zones paint over them.
        let mut ordered: Vec<_> = self.groups.group_list().iter().collect();
        ordered.sort_by_key(|g| std::cmp::Reverse(g.kind.rank()));
        let mut work: Vec<(Vec<String>, String, String, egui::Color32, egui::Color32)> =
            Vec::new();
        for g in ordered {
            let (fill, stroke) = match g.kind {
                GroupKind::Unsur => (
                    egui::Color32::from_rgba_unmultiplied(0x16, 0xa3, 0x4a, 70),
                    egui::Color32::from_rgb(0x16, 0xa3, 0x4a),
                ),
                GroupKind::SatuanTugas => (
                    egui::Color32::from_rgba_unmultiplied(0x25, 0x63, 0xeb, 70),
                    egui::Color32::from_rgb(0x25, 0x63, 0xeb),
                ),
                GroupKind::Gugus => (
                    egui::Color32::from_rgba_unmultiplied(0x93, 0x33, 0xea, 70),
                    egui::Color32::from_rgb(0x93, 0x33, 0xea),
                ),
                GroupKind::OperasiGabungan => (
                    egui::Color32::from_rgba_unmultiplied(0xea, 0x58, 0x0c, 70),
                    egui::Color32::from_rgb(0xea, 0x58, 0x0c),
                ),
            };
            work.push((
                self.groups.group_units(&g.id),
                g.id.clone(),
                g.name.clone(),
                fill,
                stroke,
            ));
        }
        // A nested group goes quiet when far: a Gugus frame and three
        // Unsur frames within a few pixels is unreadable, and the parent
        // plus its count already says the task organisation exists.
        let nested = self.groups.nested_ids();
        let selected_group = match self.selection.as_ref() {
            Some(Selection::Group(gid)) => Some(gid.clone()),
            _ => None,
        };
        let (mw, mh) = self.map_dims();
        let center = self.center;
        let mut zones = Vec::new();
        let mut symbols = Vec::new();
        for (members, gid, name, fill, level_ink) in work {
            // Hidden members leave everything: the hull, the centroid,
            // the count and the extent. A Group with no non-hidden
            // member draws nothing at all — and, because `hidden` is
            // the operator's deliberate per-unit "show on map" choice,
            // nothing that could give away where those units are.
            let visible: Vec<&ShipMarker> = markers
                .iter()
                .filter(|m| !self.hidden.contains(&m.id) && members.iter().any(|u| u == &m.id))
                .collect();
            if visible.is_empty() {
                continue;
            }
            let positions: Vec<(f64, f64)> =
                visible.iter().map(|m| (m.latitude, m.longitude)).collect();
            // The ONE centroid: measured here, drawn on there, so the
            // extent is always taken about the point the operator sees.
            let extent = projected_group_extent(&positions, self.zoom);
            let current = self.group_representations.get(&gid).copied();
            let representation = select_group_representation(&extent, current);
            self.group_representations.insert(gid.clone(), representation);
            let (sx, sy) = project_mercator(
                extent.centroid_lat,
                extent.centroid_lon,
                center,
                self.zoom,
                mw,
                mh,
            );
            let pts: Vec<(f64, f64)> = visible.iter().map(|m| (m.x, m.y)).collect();

            if representation == GroupRepresentation::Zone {
                // A TRUE convex dilation of the member hull, padded to
                // enclose the members' own markers: the fattest member's
                // half-extent (or the Far symbol's footprint circle,
                // whichever is larger) plus a margin.
                let fattest = visible.iter().map(|m| m.footprint_px).fold(0.0f64, f64::max);
                let hull = zone_polygon(&pts, zone_pad_px(fattest));
                let width_px = zone_width_px(&hull);
                zones.push(ZoneGeom {
                    group: gid,
                    label: format!("{name} ({})", pts.len()),
                    width_px,
                    pts: hull.into_iter().map(|(x, y)| (x as f32, y as f32)).collect(),
                    fill,
                    stroke: level_ink,
                });
                continue;
            }

            // Far. A nested group stays silent unless it is selected —
            // selection is never invisible on the map, and the halo says
            // which frame is the selected one. The ladder itself is NOT
            // overridden: selecting a small group never forces a Zone.
            let selected = selected_group.as_deref() == Some(gid.as_str());
            if nested.contains(&gid) && !selected {
                continue;
            }
            let affiliation = self.group_affiliation(&gid, &visible);
            symbols.push(GroupSymbolGeom {
                group: gid,
                x: sx as f32,
                y: sy as f32,
                lat: extent.centroid_lat,
                lon: extent.centroid_lon,
                label: format!("{name} ({})", pts.len()),
                affiliation,
                icon: plurality_icon(
                    &visible.iter().map(|m| m.map_symbol).collect::<Vec<_>>(),
                ),
                level_ink,
                cover_radius_m: extent.cover_radius_m,
            });
        }
        (zones, symbols)
    }

    /// Log island: current warning plus the capped sim event feed.
    /// Empty feed names the next action instead of showing a blank box.
    /// #99: load the inbox page off-thread (harvested in the pump).
    /// Page 20 at a time with server-bound prev/next; the mine filter
    /// resets to page 1. Failures keep the last good page loudly.
    fn refresh_inbox(&mut self) {
        if self.setup_busy("inbox") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let Ok((master, tok)) = self.users_client() else {
            self.users_status = "sign in first".to_string();
            return;
        };
        let mine = self.inbox_mine_only;
        let page = self.inbox_page_no.max(1);
        self.setup_op = Some(spawn_rest("inbox", move || {
            master
                .inbox_page(&tok, gid, None, mine, page, 20)
                .map_err(|e| e.to_string())
                .map(SetupDone::MsgPage)
        }));
    }

    /// Open one message's detail off-thread, or delete one (sender
    /// withdraws for all, recipient hides their own view — a merely
    /// seen broadcast refuses loudly). The page reloads behind a
    /// delete so counts stay the server's.
    fn open_message(&mut self, mid: i64) {
        if self.setup_busy("message") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let Ok((master, tok)) = self.users_client() else {
            self.users_status = "sign in first".to_string();
            return;
        };
        self.setup_op = Some(spawn_rest("message", move || {
            master
                .get_message(&tok, gid, mid)
                .map_err(|e| e.to_string())
                .map(SetupDone::MsgOpen)
        }));
    }

    /// Delete one message off-thread (see open_message for whose view
    /// goes). The page reloads behind it.
    fn delete_message(&mut self, mid: i64) {
        if self.setup_busy("delete") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let Ok((master, tok)) = self.users_client() else {
            self.users_status = "sign in first".to_string();
            return;
        };
        self.setup_op = Some(spawn_rest("delete", move || {
            master
                .delete_message(&tok, gid, mid)
                .map_err(|e| e.to_string())
                .map(|m| SetupDone::MsgDeleted(m.id))
        }));
    }

    /// #99: send the composed message off-thread. Degree is required
    /// by contract ([7.7]); an empty audience sends a broadcast.
    fn send_composed(&mut self) {
        if self.setup_busy("send") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let Ok((master, tok)) = self.users_client() else {
            self.users_status = "sign in first".to_string();
            return;
        };
        if self.msg_content.trim().is_empty() {
            self.users_status = "write the message first".to_string();
            return;
        }
        let Some(degree) = self.msg_degree else {
            self.users_status = "pick a degree (Derajat) — required".to_string();
            return;
        };
        let mut to: Vec<i64> = self.msg_to.iter().cloned().collect();
        to.sort();
        let mut cc: Vec<i64> = self.msg_cc.iter().cloned().collect();
        cc.sort();
        let draft = tfg::backend::MsgDraft {
            kind: self.msg_kind.clone(),
            classification: self.msg_class.clone(),
            content: self.msg_content.trim().to_string(),
            to,
            cc,
            assumed_role: self.msg_assumed,
            reply_to: self.msg_reply_to,
            degree,
            msg_type: None,
            callsign: self.msg_callsign.trim().to_string(),
            sending_note: self.msg_sending_note.trim().to_string(),
            group_name: self.msg_group.trim().to_string(),
            per: self.msg_per.trim().to_string(),
            registration_number: self.msg_regnum.trim().to_string(),
        };
        self.setup_op = Some(spawn_rest("send", move || {
            master
                .send_message(&tok, gid, &draft)
                .map_err(|e| e.to_string())
                .map(SetupDone::MsgSent)
        }));
    }

    /// #99: refresh the send-as identities off-thread.
    fn refresh_roles(&mut self) {
        if self.setup_busy("roles") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let Ok((master, tok)) = self.users_client() else {
            self.users_status = "sign in first".to_string();
            return;
        };
        self.setup_op = Some(spawn_rest("roles", move || {
            master
                .scenario_roles(&tok, gid)
                .map_err(|e| e.to_string())
                .map(SetupDone::Roles)
        }));
    }

    /// #99: author a send-as identity, then reload the list in the
    /// same worker so the picker shows it at once.
    fn create_role(&mut self) {
        if self.setup_busy("roles") {
            return;
        }
        let name = self.new_role_name.trim().to_string();
        if name.is_empty() {
            self.users_status = "name the role first".to_string();
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let Ok((master, tok)) = self.users_client() else {
            self.users_status = "sign in first".to_string();
            return;
        };
        self.setup_op = Some(spawn_rest("roles", move || {
            master.create_scenario_role(&tok, gid, &name).map_err(|e| e.to_string())?;
            master
                .scenario_roles(&tok, gid)
                .map_err(|e| e.to_string())
                .map(SetupDone::Roles)
        }));
    }

    /// #99: record the caller's read receipt off-thread. Broadcasts
    /// carry none — the server refuses those loudly.
    fn mark_message_read(&mut self, mid: i64) {
        if self.setup_busy("mark-read") {
            return;
        }
        let Some((gid, _)) = self.users_game.clone() else {
            self.users_status = "hold a session first".to_string();
            return;
        };
        let Ok((master, tok)) = self.users_client() else {
            self.users_status = "sign in first".to_string();
            return;
        };
        self.setup_op = Some(spawn_rest("read", move || {
            master
                .mark_read(&tok, gid, mid)
                .map_err(|e| e.to_string())
                .map(SetupDone::ReadDone)
        }));
    }

    /// #99: the caller's unread badge for one inbox row — addressed to
    /// me with no receipt. Never name-matched: roster identity is the
    /// probed user id.
    fn msg_is_unread(&self, m: &tfg::backend::InboxMsg) -> bool {
        let Some(me) = self.auth_user_id else {
            return false;
        };
        m.recipients.iter().any(|r| r.user_id == me && r.read_at.is_none())
    }

    /// Messages island (H11): socket-arrived game mail, newest first.
    /// Sending, inbox reads, and receipts stay HTTP (later slice) —
    /// this draws what the channels delivered.
    fn messages_island(&mut self, ui: &mut egui::Ui) {
        // #99: inbox first (readable, actionable), socket arrivals
        // below (as-delivered), compose last.
        ui.heading("Inbox");
        ui.horizontal(|ui| {
            if ui.small_button("refresh").clicked() {
                self.inbox_page_no = 1;
                self.refresh_inbox();
            }
            // The narrow resets to page 1 — a later page may not
            // exist under it.
            if ui.checkbox(&mut self.inbox_mine_only, "addressed to me").changed() {
                self.inbox_page_no = 1;
                self.refresh_inbox();
            }
            if ui
                .add_enabled(self.inbox_has_prev, egui::Button::new("← prev"))
                .clicked()
            {
                self.inbox_page_no = self.inbox_page_no.saturating_sub(1).max(1);
                self.refresh_inbox();
            }
            if ui
                .add_enabled(self.inbox_has_next, egui::Button::new("next →"))
                .clicked()
            {
                self.inbox_page_no += 1;
                self.refresh_inbox();
            }
            ui.weak(format!(
                "page {} of {} · {} total",
                self.inbox_page_no, self.inbox_pages, self.inbox_total
            ));
        });
        status_line(ui, &self.users_status.clone());
        // Open message detail: the single-get pane above the thread.
        // No edit exists by contract, so none renders.
        if let Some(open) = self.msg_open.clone() {
            ui.separator();
            ui.horizontal(|ui| {
                ui.strong(format!("msg #{}", open.id));
                ui.label(format!("from {}", open.sender));
                if ui.small_button("close").clicked() {
                    self.msg_open = None;
                }
            });
            ui.label(format!("session {} · {}", open.game_id, open.created_at));
            if !open.callsign.is_empty() {
                ui.label(format!("callsign: {}", open.callsign));
            }
            ui.label(&open.content);
            ui.separator();
        }
        if self.inbox.is_empty() {
            ui.weak("Inbox empty — refresh to load the thread.");
        } else {
            let mut read_ids: Vec<i64> = Vec::new();
            let mut reply_id: Option<i64> = None;
            let mut open_id: Option<i64> = None;
            let mut delete_id: Option<i64> = None;
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for m in self.inbox.clone() {
                    ui.horizontal(|ui| {
                        ui.strong(&m.kind);
                        ui.label(&m.class_label);
                        ui.label(format!("from {}", m.sender));
                        if m.broadcast {
                            ui.label(egui::RichText::new("broadcast").weak());
                        } else if self.msg_is_unread(&m) {
                            ui.label(egui::RichText::new("UNREAD").strong());
                        }
                    });
                    ui.label(format!("session {} · msg {} · {}", m.game_id, m.id, m.created_at));
                    if !m.callsign.is_empty() {
                        ui.label(format!("callsign: {}", m.callsign));
                    }
                    ui.label(&m.content);
                    ui.horizontal(|ui| {
                        if !m.broadcast && self.msg_is_unread(&m) && ui.small_button("mark read").clicked() {
                            read_ids.push(m.id);
                        }
                        if ui.small_button("open").clicked() {
                            open_id = Some(m.id);
                        }
                        if ui.small_button("reply").clicked() {
                            reply_id = Some(m.id);
                        }
                        // Sender withdraws for all, recipient hides
                        // their own view — the server decides which
                        // and refuses the rest loudly.
                        if ui.small_button("delete").clicked() {
                            delete_id = Some(m.id);
                        }
                    });
                    ui.separator();
                }
            });
            for mid in read_ids {
                self.mark_message_read(mid);
            }
            if let Some(mid) = open_id {
                self.open_message(mid);
            }
            if let Some(mid) = delete_id {
                self.delete_message(mid);
            }
            if let Some(mid) = reply_id {
                self.msg_reply_to = Some(mid);
                self.users_status = format!("replying to #{mid}");
            }
        }
        ui.separator();
        ui.heading("Live arrivals");
        if self.game_messages.is_empty() {
            ui.weak("No socket mail yet — broadcasts and addressed pushes land here.");
        } else {
            let mut live_reads: Vec<i64> = Vec::new();
            egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                for m in self.game_messages.iter().rev().cloned() {
                    ui.horizontal(|ui| {
                        ui.strong(&m.kind);
                        ui.label(&m.class_label);
                        ui.label(format!("from {}", m.sender));
                        if m.broadcast {
                            ui.label(egui::RichText::new("broadcast").weak());
                        } else if m.event == "personal.message_sent"
                            && ui.small_button("mark read").clicked()
                        {
                            live_reads.push(m.id);
                        }
                    });
                    ui.label(format!("session {} · msg {} · {}", m.game_id, m.id, m.created_at));
                    ui.label(&m.content);
                    ui.separator();
                }
            });
            for mid in live_reads {
                self.mark_message_read(mid);
            }
        }
        ui.separator();
        self.compose_ui(ui);
        status_line(ui, &self.users_status.clone());
    }

    /// #99: the send form — kind, marking, audience, grade, identity,
    /// extra boxes, reply target. Sends off-thread; the inbox reloads
    /// on success.
    fn compose_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("New message");
        if self.users_game.is_none() {
            ui.weak("Hold a session first — mail belongs to the exercise.");
            return;
        }
        if let Some(rid) = self.msg_reply_to {
            ui.horizontal(|ui| {
                ui.label(format!("replying to #{rid}"));
                if ui.small_button("×").on_hover_text("drop reply target").clicked() {
                    self.msg_reply_to = None;
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label("kind:");
            egui::ComboBox::from_id_salt("msg-kind")
                .selected_text(&self.msg_kind)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.msg_kind, "telegram".to_string(), "telegram");
                    ui.selectable_value(&mut self.msg_kind, "administrative".to_string(), "administrative");
                });
            ui.label("marking:");
            egui::ComboBox::from_id_salt("msg-class")
                .selected_text(&self.msg_class)
                .show_ui(ui, |ui| {
                    for c in ["TERBUKA", "TERBATAS", "RAHASIA"] {
                        ui.selectable_value(&mut self.msg_class, c.to_string(), c);
                    }
                });
        });
        ui.label("content:");
        ui.add(
            egui::TextEdit::multiline(&mut self.msg_content)
                .desired_rows(3)
                .hint_text("message text…"),
        );
        // Audience: roster seats toggle into to/cc. Nobody picked is a
        // broadcast by contract — the form says so, not the server.
        ui.horizontal(|ui| {
            ui.weak("audience (none picked = broadcast):");
        });
        if self.users_roster.is_empty() {
            ui.weak("no roster loaded — seat players first, or send broadcast.");
        } else {
            let mut toggles: Vec<(i64, bool)> = Vec::new();
            egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                for p in self.users_roster.clone() {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} · {}", p.user_name, p.role_name));
                        let to_on = self.msg_to.contains(&p.user_id);
                        let cc_on = self.msg_cc.contains(&p.user_id);
                        if ui.small_button(if to_on { "to ●" } else { "to" }).clicked() {
                            toggles.push((p.user_id, true));
                        }
                        if ui.small_button(if cc_on { "cc ●" } else { "cc" }).clicked() {
                            toggles.push((p.user_id, false));
                        }
                    });
                }
            });
            for (uid, is_to) in toggles {
                let set = if is_to { &mut self.msg_to } else { &mut self.msg_cc };
                if !set.remove(&uid) {
                    set.insert(uid);
                }
            }
        }
        ui.horizontal(|ui| {
            ui.label("degree:");
            let deg_name = self
                .msg_degree
                .and_then(|d| self.msg_degrees.iter().find(|(id, _, _, _)| *id == d))
                .map(|(_, n, _, _)| n.clone())
                .unwrap_or_else(|| "pick".to_string());
            egui::ComboBox::from_id_salt("msg-degree")
                .selected_text(deg_name)
                .show_ui(ui, |ui| {
                    for (id, name, _, _) in self.msg_degrees.clone() {
                        ui.selectable_value(&mut self.msg_degree, Some(id), name);
                    }
                });
            if self.msg_degrees.is_empty() {
                ui.weak("(sync helpers for grades)");
            }
        });
        ui.horizontal(|ui| {
            ui.label("send as:");
            let role_name = self
                .msg_assumed
                .and_then(|r| self.scenario_roles.iter().find(|s| s.id == r))
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "self".to_string());
            egui::ComboBox::from_id_salt("msg-role")
                .selected_text(role_name)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.msg_assumed, None, "self");
                    for s in self.scenario_roles.clone() {
                        ui.selectable_value(&mut self.msg_assumed, Some(s.id), &s.name);
                    }
                });
            if ui.small_button("roles ↻").clicked() {
                self.refresh_roles();
            }
        });
        ui.horizontal(|ui| {
            ui.label("new role:");
            ui.text_edit_singleline(&mut self.new_role_name);
            if ui.small_button("create").clicked() {
                self.create_role();
            }
        });
        ui.horizontal(|ui| {
            ui.label("callsign:");
            ui.text_edit_singleline(&mut self.msg_callsign);
        });
        ui.horizontal(|ui| {
            ui.label("note:");
            ui.text_edit_singleline(&mut self.msg_sending_note);
            ui.label("group:");
            ui.text_edit_singleline(&mut self.msg_group);
        });
        ui.horizontal(|ui| {
            ui.label("per:");
            ui.text_edit_singleline(&mut self.msg_per);
            ui.label("regnum:");
            ui.text_edit_singleline(&mut self.msg_regnum);
        });
        if ui.button("send").clicked() {
            self.send_composed();
        }
    }

    fn log_island(&mut self, ui: &mut egui::Ui) {
        if let Some(w) = self.order_warning.clone() {
            warn_line(ui, w);
        }
        if self.event_feed.is_empty() {
            ui.weak("No events yet — orders and arrivals land here.");
        } else {
            egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                for line in &self.event_feed {
                    ui.monospace(line);
                }
            });
        }
        ui.separator();
        // Session history (moved out of the deleted Session island):
        // past journals plus the replay slider that drives map ghosts.
        ui.heading("Session logs");
        if let Some(path) = self.log_view_path.clone() {
            let (entries, units, players) = match &self.log_view {
                Some(v) => (v.entries.clone(), v.units.clone(), v.players.clone()),
                None => {
                    ui.weak("Parsing journal — a moment…");
                    (Vec::new(), Vec::new(), Vec::new())
                }
            };
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
                ui.add(egui::Slider::new(&mut self.replay_pos, 0..=max).text("replay")).on_hover_text("journal replay position (ghosts, not live)");
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
            // Directory scan runs once per open on the log worker —
            // never per frame — with a manual refresh beside it.
            if !self.log_files_loaded && self.refresh_log_files() {
                self.log_files_loaded = true;
            }
            let files = self.log_files.clone();
            ui.horizontal(|ui| {
                if ui.small_button("refresh").clicked() {
                    self.log_files_loaded = false;
                }
                if files.is_empty() {
                    ui.weak("no past sessions yet");
                }
            });
            // Cap the list (harden): journals accumulate per Start.
            for f in files.iter().take(20) {
                let name = f
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string();
                if ui.small_button(&name).clicked() {
                    self.load_log_view(f.clone());
                }
            }
            if files.len() > 20 {
                ui.weak(format!("…and {} older", files.len() - 20));
            }
        }
    }


    /// First-run screen (onboarding ticket, #77): a clean gradient
    /// with one card — no toolbar, islands, or wizard — holding State
    /// A (Login) then State B (Mode Selection). The map keeps loading
    /// underneath so C/D open onto it already drawn.
    fn onboard_ui(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            let rect = ui.max_rect();
            paint_gradient(ui.painter(), rect, ONBOARD_TOP, ONBOARD_BOTTOM);
            egui::Area::new(egui::Id::new("onboard"))
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style())
                        // Roomier card (polish pass): the default menu
                        // margin hugged the fields.
                        .inner_margin(egui::Margin {
                            left: 26,
                            right: 26,
                            top: 22,
                            bottom: 24,
                        })
                        .show(ui, |ui| {
                            ui.set_width(400.0);
                            match self.onboard {
                                Onboard::Login => self.onboard_login(ui),
                                Onboard::App => {}
                            }
                        });
                });
        });
    }

    /// State A (ticket #77): sign-in on the clean card. The
    /// must-change-password gate blocks here too; a signed-in pair
    /// advances to Mode Selection, and the backend-optional path
    /// (PRODUCT: air-gapped) walks past without one.
    fn onboard_login(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new("ARCONS")
                .size(28.0)
                .strong()
                .color(ONBOARD_ACCENT),
        );
        ui.label(egui::RichText::new("Command Center").size(14.0).weak());
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(8.0);
        if self.auth_user.is_some() && self.auth_needs_password_change {
            ui.heading("Change password");
            ui.label("The backend shuts every door until this is done.");
            self.change_password_ui(ui);
        } else if self.auth_user.is_none() {
            ui.heading("Sign in");
            // One height for fields and button (AUTH_FIELD_H), text
            // middle-aligned — the singleline default read as a
            // hairline slot on this card.
            let id = ui.add_sized(
                [ui.available_width(), AUTH_FIELD_H],
                egui::TextEdit::singleline(&mut self.login_identifier)
                    .hint_text("identifier")
                    .min_size(egui::vec2(0.0, AUTH_FIELD_H))
                    .vertical_align(egui::Align::Center),
            );
            let pw = ui.add_sized(
                [ui.available_width(), AUTH_FIELD_H],
                egui::TextEdit::singleline(&mut self.login_password)
                    .password(true)
                    .hint_text("password")
                    .min_size(egui::vec2(0.0, AUTH_FIELD_H))
                    .vertical_align(egui::Align::Center),
            );
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            let submitted = ui
                .add_sized(
                    [ui.available_width(), AUTH_FIELD_H],
                    egui::Button::new(
                        egui::RichText::new("Sign in").strong().color(ONBOARD_INK),
                    )
                    .fill(ONBOARD_ACCENT),
                )
                .clicked();
            if submitted || (enter && (id.has_focus() || pw.has_focus())) {
                self.attempt_sign_in();
            }
        }
        // No always-on "signed out" readout on this card (polish pass);
        // a real refusal still reports.
        let status = self.auth_status.clone();
        if status != "signed out" && !status.starts_with("signed in") {
            status_line(ui, &status);
        }
        // Signed in this frame (or the password gate just lifted): go
        // straight into the console.
        if self.auth_user.is_some() && !self.auth_needs_password_change {
            self.enter_shell(AppMode::Simulation);
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
        // Camera glide (src/camera.rs): advance the eased camera and mirror
        // it into `center` before anything reads it, so the overlay
        // projection, the hit-tests and the painted picture all agree on
        // where the camera is this frame. egui requests the repaints itself
        // while a ramp is in flight, so this costs one call while the
        // camera is travelling and nothing at all once it has settled.
        let gliding = self.glide.step(ui.ctx(), tfg::camera::GLIDE_SECS);
        self.center = self.glide.at;
        // M7: harvest off-thread REST before rendering, so statuses
        // and lists are a frame fresh at most.
        self.pump_rest_ops();
        self.update_unit_drag(ui);
        // Text scale (field ticket): OS base captured once, pref
        // multiplied on top — idempotent per frame, never compounding.
        if self.base_ppp.is_none() {
            self.base_ppp = Some(ui.ctx().pixels_per_point());
        }
        if let Some(base) = self.base_ppp {
            ui.ctx().set_pixels_per_point(base * self.text_scale);
        }
        // Reduced motion: zeroing egui's animation clock collapses every
        // in-flight tween to its target this frame. It is one number rather
        // than a per-component switch because DESIGN.md promises one switch,
        // and because a partial implementation is worse than none: an island
        // that fades while its neighbour does not reads as a bug.
        //
        // Read first, so switching the preference back on restores the
        // user's own setting rather than the last value this code wrote.
        let wants_motion = !self.reduced_motion;
        ui.ctx().all_styles_mut(|style| {
            style.animation_time = if wants_motion {
                self.motion_secs
            } else {
                0.0
            };
        });
        // Flush a trailing seamless-zoom step (task #43): the last tick
        // inside the throttle window still gets its frame.
        if self.zoom_dirty && self.last_zoom_req.elapsed() >= Duration::from_millis(250) {
            self.zoom_dirty = false;
            self.last_zoom_req = Instant::now();
            self.refresh_map();
        }
        let markers = self.markers(ui.ctx().pixels_per_point());
        self.load_marker_textures(ui.ctx(), &markers);
        // Proactive refresh (auth resolution): at 80% of TTL, on our
        // terms so the map never blanks on a timer. See refresh_now
        // (shared with the socket 109 path).
        if let (Some(_user), Some(issued), ttl) =
            (self.auth_user.clone(), self.auth_issued_at, self.auth_ttl_secs)
        {
            if ttl > 0 && issued.elapsed().as_secs() * 5 >= ttl * 4 {
                self.refresh_now();
            }
        }
        // Group overlays (slice iii, grill #24): recomputed per frame from
        // live marker positions; a Zone per group whose extent reads as a
        // shape, a framed symbol per group whose extent does not.
        // The Representation of each group rides along for the painters that
        // will consume it; the flag itself stays until #172 retires it.
        let (zones, group_symbols) = self.group_geometry(&markers);
        // On-demand repaint. The old unconditional 100ms floor guaranteed a
        // layout pass ten times a second whether or not anything moved; on
        // an air-gapped laptop station that is the whole cost of an idle
        // console, and it is why "Offline is a first-class state" was not
        // quite true.
        //
        // A frame is asked for when: a channel pump consumed something, a
        // hull is mid-glide, the camera is easing, or the operator touched
        // anything (egui requests a frame per input event on its own).
        //
        // The watchdog is the honest part. Freshness deadlines, image URL
        // expiry and the game clock can all change display state without
        // anything crossing a channel, and they are not all enumerated
        // here. A 2s backstop turns a missed case into a stale readout
        // rather than a frozen console, at a cost of one frame per two
        // seconds instead of ten. Drop it only once every timer in this
        // list is event-driven — that is the change that makes an idle
        // console cost genuinely nothing.
        if self.dirty || gliding {
            ui.ctx().request_repaint();
        } else if self.rest_in_flight() {
            // Hold the old cadence while a request is outstanding: a login
            // is the operator staring at a button, and the result must not
            // wait on the watchdog.
            ui.ctx().request_repaint_after(REST_POLL_REPAINT);
        } else {
            ui.ctx().request_repaint_after(WATCHDOG_REPAINT);
        }
        self.dirty = false;

        // Onboarding (ticket #77): States A/B — login and mode
        // selection on a clean gradient — draw no toolbar, islands,
        // or map. The shell (C presentation / D simulation) takes
        // over the frame the moment mode selection hands off.
        if self.onboard != Onboard::App {
            self.onboard_ui(ui);
            return;
        }

        // Focus contract (blocking ticket): global keys never fire
        // while a text field owns the keyboard — typing a space must
        // not pause the exercise, nor a digit switch desktops.
        if !ui.ctx().egui_wants_keyboard_input() {
            // Space toggles pause: a full hold (ADR-0004). The sim enforces it
            // tick-wise; the UI just forwards the verb. H2: a connected
            // execution pauses through Minos instead — the scenario hold
            // follows back onto the local display via apply_clock.
            if ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) {
                if self.users_game_state.as_deref() == Some("execution")
                    && self.users_game.is_some()
                {
                    self.pause_or_resume_minos();
                } else if let Some(tx) = &self.sim_cmd_tx {
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
            self.keyboard_map_path(ui.ctx());
        }

        // Toolbar (islands grill, #27): island toggles + the clock block.
        // The dock is dead; every flow below is a floating island.
        // Keys: Space pause · 1-9 desktops · [ ] ships · G groups ·
        // F follow · W waypoint-at-center · O orders · R roster · Esc drop.
        self.top_zone(ui);
        // State C (onboarding ticket, #77): Presentation's first
        // action is one prominent connect card over the loaded map —
        // it hides once live is up or the operator dismisses it.
        if self.app_mode == AppMode::Presentation
            && self.connect_card
            && self.live_cmd_tx.is_none()
        {
            egui::Window::new("presentation")
                .title_bar(false)
                .collapsible(false)
                .movable(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 40.0))
                .show(ui.ctx(), |ui| {
                    ui.set_min_width(380.0);
                    ui.label(
                        egui::RichText::new("Presentation mode").strong().size(18.0),
                    );
                    ui.label(
                        "The map is loaded. Connect to stream positions from the backend.",
                    );
                    ui.add_space(4.0);
                    status_line(ui, &self.live_status.clone());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let authed = self.auth_token.is_some();
                        if ui
                            .add_enabled(
                                authed,
                                egui::Button::new(
                                    egui::RichText::new("Connect live →")
                                        .strong()
                                        .color(ONBOARD_INK),
                                )
                                .fill(ONBOARD_ACCENT),
                            )
                            .clicked()
                        {
                            self.toggle_live();
                            self.connect_card = false;
                        }
                        if ui.button("Not now").clicked() {
                            self.connect_card = false;
                        }
                    });
                    if self.auth_token.is_none() {
                        ui.label(
                            egui::RichText::new(
                                "No sign-in yet: use Login in the toolbar first (offline setups can still watch the wire).",
                            )
                            .weak()
                            .size(11.0),
                        );
                    }
                });
        }
        // State D (onboarding ticket, #77): the four-phase bar over
        // the loaded map — Planning shows the setup panel beside it,
        // Evaluasi the assessment workspace.
        if self.app_mode == AppMode::Simulation && self.show_side_zone {
            if std::env::var("TFG_ZONE_DEBUG").is_ok() {
                eprintln!("GATE: calling side_zone");
            }
            self.side_zone(ui);
        } else if std::env::var("TFG_ZONE_DEBUG").is_ok() {
            eprintln!(
                "GATE: side_zone SKIPPED mode={:?} show_side_zone={}",
                self.app_mode, self.show_side_zone
            );
        }
        // Only ever one modal at a time — see `open_only`.
        //
        // `modal_panel_rects` is cleared before any panel publishes rather than
        // by each modal on close, so it is cleared once here, ahead of all of
        // them. See `modal_panel_rects`.
        self.modal_panel_rects.clear();
        if self.settings_open {
            self.settings_modal(ui);
        }
        if self.composer_visible {
            self.composer_modal(ui);
        }
        if self.fleet_picker_open {
            self.fleet_picker_modal(ui);
        }
        if self.player_picker_open {
            self.player_picker_modal(ui);
        }

        let mut follow_req: Option<(String, (f64, f64))> = None;
        // NOT gated on `zone_carries`: this island lists SHIPS in view, and
        // the column's Crew island lists PEOPLE in the room. They answer
        // different questions and both are wanted during execution, so
        // suppressing either would lose something. The names differ for the
        // same reason — "Roster" beside "Crew" is not two names for one thing.
        if self.show_roster
            && !self.zone_owns_left_edge()
            && (self.session_live() || self.app_mode == AppMode::Presentation)
        {
            let mut open = self.show_roster;
            // Island chrome, not egui::Window (ADR-0014): egui::Frame can
            // only paint a rounded rect, so a chamfered body is impossible
            // on a native Window. The island owns its own position, so it
            // is copied out and written back.
            let mut pos = self.roster_pos;
            let spec = tfg::chrome::Island::new(
                egui::Id::new("Roster"),
                "Roster",
                egui::vec2(300.0, 420.0),
            );
            tfg::chrome::island(ui.ctx(), &spec, &mut pos, &mut open, |ui| {
            ui.label(format!("{} ships — click a name to follow", markers.len()));
            if markers.is_empty() {
                ui.weak("No ships in view — place hulls from Fleet, or check the feed.");
            }
            ui.separator();
            // State machine: trails belong to Live (nothing moves in
            // Setup; Closed is frozen under its transcript).
            if self.mode.in_live() {
                ui.checkbox(&mut self.show_trail, "trails");
            }
            ui.separator();
            // The island body scrolls as one column (chrome::island), so the
            // marker list no longer carries its own 300px cap. That cap
            // existed to stop a Window growing; a fixed-footprint island
            // scrolls instead.
            {
            for m in &markers {
                ui.horizontal(|ui| {
                    let mut shown = !self.hidden.contains(&m.id);
                    if ui.checkbox(&mut shown, "").on_hover_text(format!("show {} on map", m.id)).changed() {
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
                    if m.old_data {
                        label += " (old data)";
                    }
                    if m.source == FixSource::Sim {
                        label += " (sim)";
                    }
                    if m.source == FixSource::Game {
                        label += " (game)";
                    }
                    if ui.selectable_value(&mut self.following, Some(m.id.clone()), label).clicked()
                    {
                        eprintln!("follow {:?}", self.following);
                        // Roster click is a select like a marker click.
                        self.select_ship(m.id.clone());
                        if self.following.as_deref() == Some(&m.id) {
                            if let Some(s) = self.registry.ship(&m.id) {
                                follow_req = Some((
                                    m.id.clone(),
                                    (s.latest.position.latitude, s.latest.position.longitude),
                                ));
                            }
                        }
                    }
                });
            }
            }
            // Silent vessels (REST mapping ticket): announced but never
            // reported — listed so silence and non-existence stay distinct.
            // No follow, no select: there is no position to show.
            for a in self.registry.announced() {
                ui.horizontal(|ui| {
                    let name = a.name.as_deref().unwrap_or(&a.ship_id);
                    ui.label(format!("{name} — silent"));
                });
            }
            if ui.small_button("unfollow").clicked() {
                self.following = None;
            }
            // Minos task org runs under the marker list — the tree the
            // command centre groups by, read live, never the sandbox
            // Groups model.
            self.task_org_ui(ui);
            // Map legend (field ticket): every painted state in words.
            // Roster rows carry the same states as text, so color is
            // never the only channel.
            ui.collapsing("Legend", |ui| {
                ui.label("● per-ship color — live track (gray when stale, ~6s silence)");
                ui.label(
                    "center glyph — taxonomy: dot unknown, triangle destroyer, diamond frigate, square corvette, cross auxiliary, pentagon landing, oval submarine, aircraft plane, tracked ground unit, ring port",
                );
                ui.label("amber ring — old (backfilled) data, not live");
                ui.label("yellow ring — camera follows this hull");
                ui.label("blue ring — selected, open in Inspector");
                ui.label("selected thumbnail arrow — drag to set heading");
                ui.label("dotted trail — recent fixes, Live only");
                ui.label("○ flag — collapsed group, click or zoom to expand");
                ui.label("hollow amber — journal replay ghost, not live");
                ui.weak("sync + plot ages ride the toolbar strip.");
            });
            // Keys + model (audit item): the working set made
            // discoverable, and the operating model in one breath —
            // Minos owns state, the client projects, the sandbox is
            // its own world. Keys never fire while typing.
            ui.collapsing("Keys & model", |ui| {
                ui.label("Space — pause / resume (the exercise while connected)");
                ui.label("1–9 — desktops");
                ui.label("[ / ] — cycle ships (opens the Inspector)");
                ui.label("G — cycle groups");
                ui.label("F — follow selection · W — waypoint at map center");
                ui.label("O — orders · R — roster · Esc — drop selection");
                ui.weak("helm uses Set helm; legacy waypoint navigation remains under its compatibility section.");
                ui.separator();
                ui.label("The exercise owns state, clock, orders, fixes, positions, and messages; this client is its projection and control surface. A local run without a session is a separate sandbox.");
                ui.weak("the toolbar strip — source · session · phase · seat · freshness · next — is the same everywhere.");
            });
            ui.separator();
            });
            self.show_roster = open;
            self.roster_pos = pos;
        }
        // Inspector: selection-driven (Inspector-model ticket). The window
        // exists iff a selection exists; closing it deselects. Ships get
        // the live readout, groups get members/commander/authority plus
        // command + focus actions.
        if self.selection.is_some() {
            let mut open = true;
            let mut pos = self.inspector_pos;
            let spec = tfg::chrome::Island::new(
                egui::Id::new("Inspector"),
                "Inspector",
                egui::vec2(300.0, 320.0),
            );
            tfg::chrome::island(ui.ctx(), &spec, &mut pos, &mut open, |ui| {
            ui.heading("Inspector");
            let mut follow_selected: Option<(String, (f64, f64))> = None;
            let mut focus_group: Option<(String, (f64, f64))> = None;
            let mut drill_ship: Option<String> = None;
            let mut drill_group: Option<String> = None;
            let mut deselect = false;
            match self.selection.clone() {
                Some(Selection::Ship(id)) => {
            // Hoisted out of the `match` scrutinee: the arms below mutate
            // `self` freely, and this keeps the registry borrow from
            // overlapping them.
            let selected = self
                .registry
                .ship(&id)
                .map(|s| (s.latest.clone(), s.stale, s.trail_len));
            match selected {
                Some((fix, stale, trail_len)) => {
                    let sim_badge = if fix.source == FixSource::Sim {
                        " (sim)"
                    } else if fix.source == FixSource::Game {
                        " (game)"
                    } else {
                        ""
                    };
                    ui.label(format!("ship: {}{sim_badge}{}", self.unit_label(&id), if stale { " (stale)" } else { "" }));
                    self.map_symbol_editor(ui, &id);
                    self.affiliation_editor(ui, &id);
                    let reported_heading = self.registry.blend_heading(&id, 1.0);
                    self.heading_editor(ui, &id, reported_heading);
                    // Hull picture (images ticket): the decoded
                    // versioned texture when available, otherwise the
                    // temporary source through egui's loader. Both are
                    // memory-only; the presigned URL is never mirrored.
                    if let Ok(uid) = id.parse::<i64>() {
                        let picture = self.visuals.get(uid).map(|v| {
                            (
                                v.image_url.clone(),
                                v.asset_kind,
                                v.width_px,
                                v.texture.clone(),
                                v.image_url_expires_at,
                                v.image_url_retry_at,
                            )
                        });
                        match picture {
                            Some((Some(_), AssetKind::UnitImage, width, Some(texture), _, _)) => {
                                let display_size = inspector_image_size(
                                    texture.size,
                                    self.inspector_image_width,
                                    ui.available_width(),
                                );
                                ui.add(
                                    egui::Image::from_texture(texture)
                                        .fit_to_exact_size(display_size),
                                );
                                let size = texture.size;
                                ui.weak(format!(
                                    "image decoded · manifest width {} px · texture {:.0}×{:.0} px",
                                    width.unwrap_or(0),
                                    size.x,
                                    size.y
                                ));
                            }
                            Some((Some(_), AssetKind::UnitImage, _, None, expires_at, _))
                                if expires_at.is_some_and(|at| at <= Instant::now()) =>
                            {
                                ui.weak("refreshing picture source…");
                            }
                            Some((Some(source), AssetKind::UnitImage, width, None, _, _)) => {
                                let height = self.visuals.get(uid).and_then(|v| v.height_px);
                                self.show_visual_source(ui, uid, &source, width, height);
                            }
                            Some((None, AssetKind::UnitImage, _, _, _, Some(retry_at)))
                                if retry_at > Instant::now() =>
                            {
                                ui.weak("picture source unavailable — retrying");
                            }
                            Some((None, AssetKind::UnitImage, _, _, _, _)) => {
                                ui.weak("loading picture…");
                            }
                            None => {
                                ui.weak("visual: not resolved (manifest or unit not loaded)");
                            }
                            Some((_, AssetKind::Unsupported, _, _, _, _)) => {
                                ui.weak("unsupported asset kind");
                            }
                            Some((_, AssetKind::Unavailable, _, _, _, _)) => {
                                ui.weak("no picture");
                            }
                        }
                        if self
                            .visuals
                            .get(uid)
                            .is_some_and(|v| v.asset_kind == AssetKind::UnitImage)
                        {
                            self.inspector_image_size_control(ui);
                        }
                        if let Some(v) = self.visuals.get(uid) {
                            let content_type = if v.content_type.is_empty() {
                                "unknown".to_string()
                            } else {
                                v.content_type.clone()
                            };
                            ui.weak(format!("asset type: {content_type}"));
                        }
                        // Physical measurements, when Minos published
                        // them. Unpublished reads as unknown rather
                        // than as zero metres.
                        if let Some(v) = self.visuals.get(uid) {
                            match (v.loa_m, v.beam_m) {
                                (Some(loa), Some(beam)) => {
                                    ui.label(format!("loa {loa:.1} m · beam {beam:.1} m"));
                                }
                                _ => {
                                    ui.weak("no published measurements");
                                }
                            }
                        }
                    }
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
                    // Data age at render (backfilled ticket): live fixes
                    // age from receipt, backfills from recorded time.
                    // Badged on wire sources only (sim clocks are game time).
                    if fix.source == FixSource::Wire {
                        let now = Utc::now().timestamp();
                        match fix.data_age_secs(now) {
                            Some(a) if fix.is_old_data(now) => {
                                ui.label(format!("data age: {}:{:02} · OLD DATA", a / 60, a % 60));
                            }
                            Some(a) => {
                                ui.label(format!("data age: {a}s"));
                            }
                            None => {
                                ui.weak("data age unknown");
                            }
                        }
                    }
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
                            let commander = self.groups.group(&gid).and_then(|g| g.commander.clone());
                            let allowed: Vec<String> = members.iter().filter(|u| self.action_allows(u)).cloned().collect();
                            let auth = self.command_authority(&allowed).map(Self::authority_label).unwrap_or("view only");
                            ui.label(format!("group: {name} · {} unit(s)", members.len()));
                            ui.label(format!(
                                "commander: {} · you hold: {auth}",
                                commander.as_deref().unwrap_or("—")
                            ));
                            // A group with children lists them (drillable),
                            // not a flat unit dump.
                            let child_ids: Vec<String> = self
                                .groups
                                .group(&gid)
                                .map(|g| g.children.clone())
                                .unwrap_or_default();
                            if child_ids.is_empty() {
                                for u in &members {
                                    ui.horizontal(|ui| {
                                        ui.label(self.unit_label(u));
                                        if ui.small_button("inspect").clicked() {
                                            drill_ship = Some(u.clone());
                                        }
                                    });
                                }
                            } else {
                                for cid in &child_ids {
                                    if let Some(c) = self.groups.group(cid) {
                                        ui.horizontal(|ui| {
                                            ui.label(format!("{} · {} unit(s)", c.name, self.groups.group_units(&c.id).len()));
                                            if ui.small_button("select").clicked() {
                                                drill_group = Some(c.id.clone());
                                            }
                                        });
                                        for u in &c.units {
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
                                        if let Some(s) = self.registry.ship(m) {
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
                // `request_frame` owns the camera move; pre-assigning here
                // would fight the ramp. The zoom floor is where THIS
                // group's own extent earns a Zone.
                if let Some(zoom) = self.zoom_to_show_group(&gid) {
                    self.zoom = self.zoom.max(zoom);
                }
                self.request_frame(&gid, at);
            }
            ui.separator();
            });
            self.inspector_pos = pos;
            if !open {
                self.deselect();
            }
        }
        // Orders island: Live-only; observers get no orders pane at all.
        if self.show_orders
            && self.mode.live()
            && !self.is_observer()
            && !self.zone_carries("orders")
        {
            let mut open = self.show_orders;
            let mut pos = self.orders_pos;
            let spec = tfg::chrome::Island::new(
                egui::Id::new("Orders"),
                "Orders",
                egui::vec2(380.0, 360.0),
            );
            tfg::chrome::island(ui.ctx(), &spec, &mut pos, &mut open, |ui| {
            self.orders_body(ui);
            });
            self.show_orders = open;
            self.orders_pos = pos;
        }
        // The side zone's Operator island carries identity and the sign-out,
        // so the free-floating Login island is redundant — and at x=8 it sits
        // directly underneath the zone. Suppressed while the zone is showing
        // rather than deleted, because Presentation still has no zone and
        // still needs it.
        if self.show_login && !self.zone_owns_left_edge() {
            let mut open = self.show_login;
            let mut pos = self.login_pos;
            let spec = tfg::chrome::Island::new(
                egui::Id::new("Login"),
                "Login",
                egui::vec2(300.0, 300.0),
            );
            tfg::chrome::island(ui.ctx(), &spec, &mut pos, &mut open, |ui| {
                self.login_island(ui);
            });
            self.show_login = open;
            self.login_pos = pos;
        }
        if self.show_log
            && !self.zone_owns_left_edge()
            && !self.zone_carries("log")
            && (self.session_live() || self.app_mode == AppMode::Presentation)
        {
            let mut open = self.show_log;
            let mut pos = self.log_pos;
            let spec = tfg::chrome::Island::new(
                egui::Id::new("Log"),
                "Log",
                egui::vec2(420.0, 260.0),
            );
            tfg::chrome::island(ui.ctx(), &spec, &mut pos, &mut open, |ui| {
                self.log_island(ui);
            });
            self.show_log = open;
            self.log_pos = pos;
        }
        // Messages island (H11): socket-arrived game mail. Visible in
        // both modes once mail exists or the session opens it.
        if self.show_messages && !self.zone_owns_left_edge() {
            let mut open = self.show_messages;
            let mut pos = self.messages_pos;
            let spec = tfg::chrome::Island::new(
                egui::Id::new("Messages"),
                "Messages",
                egui::vec2(420.0, 300.0),
            );
            tfg::chrome::island(ui.ctx(), &spec, &mut pos, &mut open, |ui| {
                self.messages_island(ui);
            });
            self.show_messages = open;
            self.messages_pos = pos;
        }
        if let Some((ship, at)) = follow_req {
            self.request_frame(&ship, at);
        }
        // Follow-tracking: chase the followed ship when it drifts from
        // center, rate-capped and leading (task #45). Gated on no
        // re-render in flight, so frames can't pile.
        if self.recentering.is_none() {
            if let Some(id) = self.following.clone() {
                if let Some(s) = self.registry.ship(&id) {
                    let ship_pos = s.latest.position;
                    // Copied out here so the registry borrow ends before
                    // the &mut self calls below (track_frame).
                    let motion = (s.latest.heading_deg, s.latest.speed_kn);
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
                        let at = match motion {
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
            // Keep the canvas interactive while the first map frame is still
            // rendering. The placeholder is replaced by drain_map as soon as
            // the real texture arrives; without it, placement clicks are lost
            // during the initial load.
            if self.map_tex.is_none() {
                let size = [
                    avail.x.max(1.0) as usize,
                    avail.y.max(1.0) as usize,
                ];
                let pixel_count = size[0].saturating_mul(size[1]);
                let placeholder = egui::ColorImage::new(
                    size,
                    vec![egui::Color32::TRANSPARENT; pixel_count],
                );
                self.map_tex = Some(ui.ctx().load_texture(
                    "map-placeholder",
                    placeholder,
                    egui::TextureOptions::LINEAR,
                ));
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
                let pixels_per_point = ui.ctx().pixels_per_point();
                if ui.input(|input| input.pointer.any_released()) {
                    if let Some(drag) = self.unit_drag.take() {
                        if drag.moved {
                            let drop = ui.input(|input| input.pointer.interact_pos());
                            let on_map = drop
                                .filter(|pos| rect.contains(*pos))
                                .filter(|pos| drop_lands_on_map(&self.modal_panel_rects, *pos));
                            if let Some(pos) = on_map {
                                let px = (pos.x - rect.min.x) as f64;
                                let py = (pos.y - rect.min.y) as f64;
                                let (mw, mh) = self.map_dims();
                                let (la, lo) = unproject_mercator(
                                    px, py, self.center, self.zoom, mw, mh,
                                );
                                self.arm_fleet_pick(drag.id);
                                self.try_place_picked(la, lo);
                                self.mode.tool = SetupTool::Select;
                            } else {

                                self.clear_fleet_pick();
                                // The two refusals are different and the
                                // operator needs to know which: off the map
                                // is a miss, over the form is a correction.
                                let note = if drop
                                    .is_some_and(|d| !drop_lands_on_map(&self.modal_panel_rects, d))
                                {
                                    format!(
                                        "{} not placed \u{2014} release over the map, not the picker",
                                        drag.name
                                    )
                                } else {
                                    format!(
                                        "{} placement cancelled \u{2014} release over the map",
                                        drag.name
                                    )
                                };
                                self.note_placement(note);
                            }
                        } else {
                            self.arm_fleet_pick(drag.id);
                            self.note_placement(format!(
                                "{} selected · click the map to place",
                                drag.name
                            ));
                        }
                    }
                }
                if let (Some(drag), Some(pos)) = (
                    self.unit_drag.as_ref(),
                    ui.input(|input| input.pointer.interact_pos()),
                ) {
                    if rect.contains(pos) {
                        let local = pos - rect.min.to_vec2();
                        let (la, lo) = unproject_mercator(
                            local.x as f64,
                            local.y as f64,
                            self.center,
                            self.zoom,
                            self.map_dims().0,
                            self.map_dims().1,
                        );
                        let valid = self.placement_valid(la, lo);
                        let tint = if valid {
                            egui::Color32::from_rgb(74, 222, 128)
                        } else {
                            egui::Color32::from_rgb(246, 197, 107)
                        };
                        let ghost_rect =
                            egui::Rect::from_center_size(local, egui::vec2(54.0, 54.0));
                        let painter = ui.painter_at(rect);
                        if let Ok(uid) = drag.id.parse::<i64>() {
                            if let Some(texture) = self.visuals.get(uid).and_then(|v| v.texture.clone()) {
                                painter.image(
                                    texture.id,
                                    ghost_rect,
                                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                                    egui::Color32::from_white_alpha(190),
                                );
                            } else {
                                paint_map_symbol(
                                    &painter,
                                    local,
                                    self.symbol_for_unit(uid),
                                    false,
                                    1.0,
                                );
                            }
                        }
                        painter.circle_stroke(
                            local,
                            27.0,
                            egui::Stroke::new(2.0, tint),
                        );
                        painter.text(
                            local + egui::vec2(32.0, 2.0),
                            egui::Align2::LEFT_TOP,
                            &drag.name,
                            egui::FontId::proportional(12.0),
                            egui::Color32::WHITE,
                        );
                    }
                }
                // Ghost for a placed-hull move drag: the same tinting cue as
                // the fleet-picker drop, so the two map-write gestures read
                // with one grammar — green means water, amber means the land
                // it would be refused on.
                if let (Some(m), Some(pos)) = (
                    self.map_unit_move.as_ref(),
                    ui.input(|input| input.pointer.interact_pos()),
                ) {
                    if rect.contains(pos) && m.moved {
                        let local = pos - rect.min.to_vec2();
                        let (la, lo) = unproject_mercator(
                            local.x as f64,
                            local.y as f64,
                            self.center,
                            self.zoom,
                            self.map_dims().0,
                            self.map_dims().1,
                        );
                        let valid = self.placement_valid(la, lo);
                        let tint = if valid {
                            egui::Color32::from_rgb(74, 222, 128)
                        } else {
                            egui::Color32::from_rgb(246, 197, 107)
                        };
                        let painter = ui.painter_at(rect);
                        painter.circle_stroke(local, 27.0, egui::Stroke::new(2.0, tint));
                        painter.text(
                            local + egui::vec2(32.0, 2.0),
                            egui::Align2::LEFT_TOP,
                            &m.name,
                            egui::FontId::proportional(12.0),
                            egui::Color32::WHITE,
                        );
                    }
                }
                // The selected unit's map body is the primary heading
                // handle. A drag that starts on that body changes the
                // local draft; every other map drag keeps pan behavior.
                //
                // LIVE ONLY: a heading is an order-facing fact, and the
                // local sim only takes orders once the exercise runs. In
                // Setup that same body is the move handle for a placed
                // hull's starting position — the two gestures share a
                // pointer location but never a phase, so they never
                // compete for the same drag.
                if self.mode.phase == Phase::Live
                    && (response.is_pointer_button_down_on() || response.drag_started())
                    && self.map_heading_drag.is_none()
                {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let px = (pos.x - rect.min.x) as f64;
                        let py = (pos.y - rect.min.y) as f64;
                        self.map_heading_drag = self.map_heading_target(
                            &markers,
                            px,
                            py,
                            pixels_per_point,
                        );
                    }
                }
                // Setup counterpart: a drag that starts on a placed
                // hull's body picks the hull up to move it. Only while
                // nothing is armed for placement, so the two map-write
                // gestures never share a pointer.
                if self.mode.phase == Phase::Setup
                    && (response.is_pointer_button_down_on() || response.drag_started())
                    && self.map_unit_move.is_none()
                    && self.fleet_pick.is_none()
                    && !self.placing
                {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let px = (pos.x - rect.min.x) as f64;
                        let py = (pos.y - rect.min.y) as f64;
                        self.map_unit_move = self.map_move_target(
                            &markers,
                            px,
                            py,
                            pixels_per_point,
                            pos,
                        );
                    }
                }
                let heading_drag = self.map_heading_drag.clone();
                if self.mode.phase != Phase::Closed && response.dragged() {
                    if let Some(id) = heading_drag {
                        if let Some(pos) = ui.input(|input| input.pointer.interact_pos()) {
                            if rect.contains(pos) {
                                let marker = markers.iter().find(|marker| marker.id == id);
                                if let Some(marker) = marker {
                                    let center = rect.min
                                        + egui::vec2(marker.x as f32, marker.y as f32);
                                    let delta = pos - center;
                                    if delta.length() > 4.0 {
                                        let heading = (delta.y.atan2(delta.x).to_degrees()
                                            + 90.0)
                                            .rem_euclid(360.0);
                                        self.update_map_heading_draft(&id, heading);
                                        ui.ctx().request_repaint();
                                    }
                                }
                            }
                        }
                    } else if self.map_unit_move.is_some() {
                        // A placed hull is in hand: mark it moved and keep the
                        // pointer grabbing. Nothing commits until release.
                        if let Some(pos) =
                            ui.input(|input| input.pointer.interact_pos())
                        {
                            if let Some(m) = self.map_unit_move.as_mut() {
                                if m.start.distance(pos) > 6.0 {
                                    m.moved = true;
                                }
                            }
                            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                            ui.ctx().request_repaint();
                        }
                    } else {
                        if response.drag_started() && self.following.is_some() {
                            self.following = None;
                            eprintln!("follow broken by drag");
                        }
                        let delta = response.drag_delta();
                        if delta.x != 0.0 || delta.y != 0.0 {
                            let (mw, mh) = self.map_dims();
                            // Pan is direct manipulation: 1:1 with the
                            // pointer, never eased. `snap_camera` clears
                            // the ramp so the next frame's `step` does not
                            // drag the camera back toward a stale goal.
                            self.snap_camera(unproject_mercator(
                                mw / 2.0 - delta.x as f64,
                                mh / 2.0 - delta.y as f64,
                                self.center,
                                self.zoom,
                                mw,
                                mh,
                            ));
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
                }
                if ui.input(|input| input.pointer.any_released()) {
                    self.map_heading_drag = None;
                    self.finish_unit_move(ui, rect);
                }
                // Paint order and pick order are the same list: the body
                // layer draws Far, Middle, Near, and picking walks it
                // backwards, so declared here for both uses.
                let body_order = paint_order(&markers);
                // Map click: stand up a catalog unit when placing, place
                // a pending waypoint when arming, else select nearest.
                // Armed clicks never deselect; empty water clears the
                // selection (Inspector-model ticket).
                // One click, one meaning. A single click SELECTS whatever
                // is under it — unit, group, or nothing (empty water
                // clears the selection); a DOUBLE click frames a group so
                // its Zone comes back. The first click of a double click
                // still selects, which is what an operator expects.
                let clicked = response.clicked();
                let double_clicked = response.double_clicked();
                let single = clicked && !double_clicked;
                let skip_release_click = std::mem::replace(&mut self.released_move_drag, false);
                if !skip_release_click && self.mode.phase != Phase::Closed && (single || double_clicked) {
                    if let Some(pos) = response.interact_pointer_pos() {
                        let px = (pos.x - rect.min.x) as f64;
                        let py = (pos.y - rect.min.y) as f64;
                        let (mw, mh) = self.map_dims();
                        if single
                            && placement_click_allowed(
                            self.mode.phase,
                            self.fleet_pick.as_deref(),
                            self.mode.armed.load(Ordering::SeqCst),
                            self.acting_as.as_deref(),
                        ) {
                            let (la, lo) = unproject_mercator(
                                px, py, self.center, self.zoom, mw, mh,
                            );
                            // Fleet picker: only a picked, unplaced hull
                            // stands up, with sim stats resolved. No generic
                            // or automatic placement.
                            self.try_place_picked(la, lo);
                            self.mode.tool = SetupTool::Select;
                        } else if single && self.placing {
                            let (la, lo) = unproject_mercator(
                                px, py, self.center, self.zoom, mw, mh,
                            );
                            eprintln!("waypoint preview ({la:.4}, {lo:.4})");
                            self.pending_waypoint = Some((la, lo));
                        } else {
                            // Pick order mirrors paint order: the marker
                            // painted last is on top and is picked first.
                            let hit_unit = body_order
                                .iter()
                                .rev()
                                .map(|&i| &markers[i])
                                .filter(|m| !self.hidden.contains(&m.id))
                                .find(|m| {
                                    marker_body_hit(m, px, py, self.zoom, pixels_per_point)
                                })
                                .map(|m| m.id.clone());
                            // A group's symbol box, then a Zone's
                            // outline and its name — never a Zone's
                            // interior.
                            let hit_group_symbol = group_symbols
                                .iter()
                                .find(|g| {
                                    ((g.x as f64 - px).powi(2) + (g.y as f64 - py).powi(2)).sqrt()
                                        <= SYMBOL_BOX_PX / 2.0
                                })
                                .cloned();
                            let hit_zone = zones.iter().find(|z| {
                                let cx = z.pts.iter().map(|p| p.0).sum::<f32>() / z.pts.len() as f32
                                    + rect.min.x;
                                let cy = z.pts.iter().map(|p| p.1).sum::<f32>() / z.pts.len() as f32
                                    + rect.min.y;
                                zone_outline_hit(
                                    px,
                                    py,
                                    &z.pts,
                                    (cx as f64, cy as f64),
                                    advance_width_px(ui, &z.label) as f64 / 2.0,
                                )
                            });

                            if double_clicked {
                                // Framing is a group gesture: a group's
                                // own extent, filling FRAME_VIEWPORT_FRACTION
                                // of the window.
                                let framed = hit_group_symbol.clone().map(|g| {
                                    (g.group, g.lat, g.lon, g.cover_radius_m)
                                });
                                if let Some((gid, lat, lon, radius)) = framed {
                                    let (mw, _) = self.map_dims();
                                    self.zoom = zoom_for_group_frame(
                                        lat,
                                        radius,
                                        mw,
                                        FRAME_VIEWPORT_FRACTION,
                                    );
                                    self.request_frame(&gid, (lat, lon));
                                } else if let Some(z) = hit_zone {
                                    // A Zone carries no radius, so it is
                                    // measured here from its members.
                                    if let Some((lat, radius_m, at)) =
                                        self.group_extent(&z.group)
                                    {
                                        let (mw, _) = self.map_dims();
                                        self.zoom = zoom_for_group_frame(
                                            lat,
                                            radius_m,
                                            mw,
                                            FRAME_VIEWPORT_FRACTION,
                                        );
                                        self.request_frame(&z.group, at);
                                    }
                                }
                            } else if let Some(id) = hit_unit {
                                eprintln!("select {id}");
                                self.select_ship(id);
                            } else if let Some(g) = hit_group_symbol {
                                eprintln!("select group {}", g.group);
                                self.select_group(g.group);
                            } else if let Some(z) = hit_zone {
                                eprintln!("select group {}", z.group);
                                self.select_group(z.group.clone());
                            } else if self.fleet_pick.is_none() && !self.placing {
                                // Empty water, nothing armed: deselect (the
                                // Inspector shuts with the selection).
                                eprintln!("deselect");
                                self.deselect();
                            }
                        }
                    }
                }
                // User drop (session-users ticket): onto a unit marker to
                // command that piece, onto a group flag for a bulk assign
                // across its hulls. Same release frame as hull drops; only
                // one drag is ever active. Only seated non-judges take
                // command — refused below before any write fires.
                if self.udrag.is_some() && ui.ctx().input(|i| i.pointer.any_released()) {
                    let drop = ui.ctx().input(|i| i.pointer.interact_pos());
                    let (uid, uname) = self.udrag.clone().unwrap_or((0, String::new()));
                    // The judge side never commands, and only seated
                    // people do — refused here so a group drop cannot fire
                    // one 400 per hull. With no game picked there is
                    // nothing to seat against, and the match below says so.
                    let refusal = match self.users_game.as_ref() {
                        None => None,
                        Some(_) => match self.users_roster.iter().find(|p| p.user_id == uid) {
                            None => Some(format!(
                                "drop refused: {uname} is not in this session's roster"
                            )),
                            Some(p) if self.users_is_judge(p) => Some(format!(
                                "drop refused: {uname} is on the judge side — judges do not command"
                            )),
                            Some(_) => None,
                        },
                    };
                    if let Some(reason) = refusal {
                        self.feed(reason);
                    } else {
                    match (drop, self.users_game.clone()) {
                        // Released over the form, not the map. Asked before
                        // the marker arm below because the map's rect
                        // CONTAINS the panel, so that arm would otherwise
                        // answer "release over a unit marker" — advice about a
                        // place the operator was never aiming at.
                        (Some(p), Some((_gid, _)))
                            if rect.contains(p)
                                && !drop_lands_on_map(&self.modal_panel_rects, p) =>
                        {
                            self.feed(format!(
                                "{uname} not assigned \u{2014} release over a unit marker, not the form"
                            ));
                        }
                        (Some(p), Some(_)) if rect.contains(p) => {
                            let px = (p.x - rect.min.x) as f64;
                            let py = (p.y - rect.min.y) as f64;
                            let near_marker = markers
                                .iter()
                                .filter(|m| !self.hidden.contains(&m.id))
                                .find(|m| {
                                    marker_body_hit(
                                        m,
                                        px,
                                        py,
                                        self.zoom,
                                        pixels_per_point,
                                    )
                                })
                                .map(|m| m.id.clone());
                            let near_group = group_symbols
                                .iter()
                                .find(|g| {
                                    ((g.x as f64 - px).powi(2) + (g.y as f64 - py).powi(2)).sqrt()
                                        < 16.0
                                })
                                .map(|g| g.group.clone());
                            if let Some(mid) = near_marker {
                                match mid.parse::<i64>() {
                                    Ok(hull) => self.users_command(hull, uid),
                                    Err(_) => self.feed(format!(
                                        "drop refused: {mid} is not a register hull"
                                    )),
                                }
                            } else if let Some(group) = near_group {
                                // Bulk-assign through the local draft, the
                                // same as dropping onto one marker: the
                                // commander lands in `self.force`, and
                                // Minos catches up at a stage advance.
                                let members = self.groups.group_units(&group);
                                let mut ok = 0;
                                let mut fail = 0;
                                for mid in members {
                                    match mid.parse::<i64>() {
                                        Ok(hull) => {
                                            if self.force.set_commander(hull, Some(uid)) {
                                                ok += 1;
                                            } else {
                                                fail += 1;
                                            }
                                        }
                                        Err(_) => fail += 1,
                                    }
                                }
                                self.feed(format!(
                                    "{uname} takes group: {ok} commanded, {fail} not in the force"
                                ));
                            } else {
                                self.feed(
                                    "drop cancelled: release over a unit marker or group flag"
                                        .to_string(),
                                );
                            }
                        }
                        _ => {
                            self.feed(
                                "drop cancelled: pick a hull, release over the map".to_string(),
                            );
                        }
                    }
                    }
                    self.udrag = None;
                    self.upress = None;
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
                                // The wheel is a continuous gesture, so
                                // the anchor correction is exact and
                                // uneased — snapping, not gliding.
                                self.snap_camera(anchor_center(
                                    (pos.x - rect.min.x) as f64,
                                    (pos.y - rect.min.y) as f64,
                                    self.center,
                                    old,
                                    new,
                                    mw,
                                    mh,
                                ));
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
                if self.show_grid {
                    self.paint_map_grid(&painter, rect);
                }
                // Group Zones and symbols under ships, drawn above them.
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
                    // `zone_polygon` always returns a real polygon now
                    // (a two-member capsule, a grown hull, a disc for one
                    // member), so there is no degenerate branch left to
                    // paint — and a one-member Group never gets here
                    // anyway, because its Representation is always Far.
                    painter.add(egui::Shape::convex_polygon(
                        pts.clone(),
                        z.fill,
                        egui::Stroke::new(stroke_w, stroke_color),
                    ));
                    // Zones were ANONYMOUS until now — only flags carried
                    // a name — so the near view asked "what am I looking
                    // at". Same fits rule as a far symbol, measured
                    // against the hull's own width, at the hull centroid.
                    let focused =
                        entity_focused(&self.selection, self.following.as_ref(), &z.group);
                    if should_paint_group_text(
                        z.width_px,
                        advance_width_px(ui, &z.label),
                        focused,
                    ) {
                        let centre = egui::pos2(
                            rect.min.x + pts.iter().map(|p| p.x).sum::<f32>() / pts.len() as f32,
                            rect.min.y + pts.iter().map(|p| p.y).sum::<f32>() / pts.len() as f32,
                        );
                        painter.text(
                            centre + egui::vec2(0.0, -6.0),
                            egui::Align2::CENTER_BOTTOM,
                            &z.label,
                            egui::FontId::proportional(MAP_LABEL_PX),
                            z.stroke,
                        );
                    }
                }
                for symbol in &group_symbols {
                    let focused = entity_focused(&self.selection, self.following.as_ref(), &symbol.group);
                    paint_group_symbol(
                        &painter,
                        ui,
                        symbol,
                        rect.min,
                        focused,
                        self.zoom,
                    );
                }
                let mut image_quads: Vec<Option<Vec<egui::Pos2>>> =
                    (0..markers.len()).map(|_| None).collect();
                // Layer 1: all trails, so no later unit can paint over
                // an earlier unit's status or label.
                let trails_visible = self.app_mode == AppMode::Presentation
                    || self.mode.in_live();
                for m in &markers {
                    if self.hidden.contains(&m.id) || !self.show_trail || !trails_visible {
                        continue;
                    }
                    // Trails are a Near/Middle channel: at Far the trail
                    // dots are 2 px and the ships they belong to are 4.
                    // Dropped, never faded — a fade needs per-frame alpha
                    // state, and under repaint-on-demand that means
                    // repainting toward a target computed from where a
                    // trail is GOING.
                    if m.lod == UnitLod::Far {
                        continue;
                    }
                    // Trails wear the affiliation's own ink rather than the per-unit
                    // green that is gone: a trail is a path on the map, and
                    // whose side that path is for is a real question the old
                    // fixture colour could not answer.
                    let trail_ink = if m.stale {
                        egui::Color32::GRAY
                    } else {
                        affiliation_ink(m.affiliation)
                    };
                    for (tx, ty) in &m.trail {
                        painter.circle_filled(
                            rect.min + egui::vec2(*tx as f32, *ty as f32),
                            2.0,
                            trail_ink.linear_multiply(0.55),
                        );
                    }
                }
                // Layer 2: all symbol-or-image bodies. An eligible
                // Middle/Near texture replaces the symbol; otherwise
                // the frame + taxonomy glyph remains.
                for &index in &body_order {
                    let m = &markers[index];
                    if self.hidden.contains(&m.id) {
                        continue;
                    }
                    let c = rect.min + egui::vec2(m.x as f32, m.y as f32);
                    image_quads[index] = paint_unit_image(
                        &painter,
                        m,
                        self.zoom,
                        pixels_per_point,
                        rect.min,
                    );
                    if let Some(quad) = image_quads[index].as_ref() {
                        // Middle/Near: the icon is a photograph, so the
                        // affiliation rides the redundant colour
                        // channel rather than a frame that cannot wrap
                        // a rotated quad.
                        paint_affiliation_tint(&painter, quad, m.affiliation);
                    } else {
                        // NO base disc. It was painted in `ship_color`, which
                        // matched only the sandbox fixture ids and so gave
                        // every real hull the same green — no correlation with
                        // anything, and it covered the glyph. The frame's own
                        // interior is the fill now, which is the channel it
                        // was standing in for.
                        let frame = paint_affiliation_frame(
                            &painter,
                            c,
                            m.affiliation,
                            battle_dimension(m.map_symbol),
                            m.planned,
                            m.stale,
                        );
                        paint_map_symbol(
                            &painter,
                            c,
                            m.map_symbol,
                            m.stale,
                            frame_icon_radius(frame, SYMBOL_BOX_PX) as f32
                                / (GLYPH_AUTHORING_RADIUS_PX as f32),
                        );
                    }
                }
                // Layer 3 is gone: the white circle outline it drew is
                // superseded by the affiliation frame painted in layer 2.
                // A second outline around every symbol muddied the one
                // thing the frame is for — reading allegiance — and the
                // old circle also greyed with staleness, which is a
                // second channel for a state the glyph and the rings
                // already carry.
                // Layer 4: all state rings over every body, in the
                // prescribed selected → follow → old-data order.
                //
                // The rings are the channel; the shader halo reinforces the
                // same three states rather than adding a fourth meaning.
                // Collected here and painted after the rings so the additive
                // pass lands on top of them, and skipped entirely at
                // Quality::Low — the epaint rings are the whole visual then.
                let mut halos: Vec<(egui::Pos2, tfg::fx::Halo, f32)> = Vec::new();
                // Rings are multiples of the base disc, which is how they were
                // spaced when they were literals (12/14/16 over a radius-8 disc)
                // and is what keeps them OUTSIDE the symbol now that both it and
                // the box have grown: a ring stated as a pixel count silently
                // becomes a ring around the middle of the glyph.
                let ring = |multiple: f32| multiple * symbol_footprint_radius_px() as f32;
                for m in &markers {
                    if self.hidden.contains(&m.id) {
                        continue;
                    }
                    let c = rect.min + egui::vec2(m.x as f32, m.y as f32);
                    if self.selection == Some(Selection::Ship(m.id.clone())) {
                        painter.circle_stroke(
                            c,
                            ring(1.5),
                            egui::Stroke::new(2.0, egui::Color32::LIGHT_BLUE),
                        );
                        halos.push((c, tfg::fx::Halo::Signal, 0.55));
                    }
                    if Some(&m.id) == self.following.as_ref() {
                        painter.circle_stroke(
                            c,
                            ring(1.75),
                            egui::Stroke::new(2.0, egui::Color32::YELLOW),
                        );
                        // Following is a louder state than selected: it is
                        // where the camera is going.
                        halos.push((c, tfg::fx::Halo::Signal, 0.75));
                    }
                    if m.old_data {
                        painter.circle_stroke(
                            c,
                            ring(2.0),
                            egui::Stroke::new(
                                2.0,
                                egui::Color32::from_rgb(0xF5, 0x9E, 0x0B),
                            ),
                        );
                        // Amber, weaker: old data is a caveat on what is
                        // already shown, not a call to action.
                        halos.push((c, tfg::fx::Halo::Stale, 0.40));
                    }
                }
                self.fx.paint_halos(&painter, &halos);
                // Selected on-map heading handle: the arrow sits on the
                // thumbnail body and edits only the local helm draft.
                for (index, marker) in markers.iter().enumerate() {
                    if self.hidden.contains(&marker.id) {
                        continue;
                    }
                    let selected_id = match self.selection.as_ref() {
                        Some(Selection::Ship(id)) => id,
                        _ => continue,
                    };
                    if selected_id != &marker.id
                        || !self.action_allows(selected_id)
                        || !(self.controlled.contains(selected_id)
                            || self.minos_order_target(selected_id).is_some())
                    {
                        continue;
                    }
                    let center = rect.min + egui::vec2(marker.x as f32, marker.y as f32);
                    let radius = image_quads[index]
                        .as_ref()
                        .and_then(|points| {
                            points
                                .iter()
                                .map(|point| point.distance(center))
                                .fold(None::<f32>, |max, distance| {
                                    Some(max.map_or(distance, |value: f32| value.max(distance)))
                                })
                        })
                        .map_or(18.0, |distance| distance * 0.78);
                    let heading = self
                        .helm_drafts
                        .get(&marker.id)
                        .map(|draft| draft.heading_deg)
                        .or(marker.heading_deg)
                        .unwrap_or(0.0);
                    let angle = heading.to_radians();
                    let tip = center + egui::vec2(angle.sin(), -angle.cos()) * radius;
                    painter.circle_stroke(
                        center,
                        radius,
                        egui::Stroke::new(1.5, egui::Color32::from_white_alpha(150)),
                    );
                    painter.line_segment(
                        [center, tip],
                        egui::Stroke::new(3.0, egui::Color32::LIGHT_BLUE),
                    );
                    painter.circle_filled(tip, 5.0, egui::Color32::LIGHT_BLUE);
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
                // Layer 5: labels last, after replay and waypoint
                // overlays, so every unit label that IS painted remains
                // readable. Names are a Near channel plus focus: before
                // this, every unit on the map was labelled at every zoom.
                for m in &markers {
                    if self.hidden.contains(&m.id) {
                        continue;
                    }
                    let focused = entity_focused(&self.selection, self.following.as_ref(), &m.id);
                    if !should_paint_unit_label(m.lod, focused) {
                        continue;
                    }
                    let c = rect.min + egui::vec2(m.x as f32, m.y as f32);
                    // Offset off the FRAME, not off the disc: a name is
                    // anchored to the symbol's corner, and the friendly
                    // rectangle is the widest frame there is. Half the box
                    // clears all four, where a fixed 10 px sat inside a
                    // 32 px symbol's own body.
                    let offset = SYMBOL_BOX_PX as f32 * 0.5 + 2.0;
                    painter.text(
                        c + egui::vec2(offset, -offset),
                        egui::Align2::LEFT_TOP,
                        &m.label,
                        egui::FontId::proportional(MAP_LABEL_PX),
                        MAP_INK,
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
/// cyan accent, roomier spacing. Stock font, applied to every theme slot so
/// the look holds regardless of system preference.
///
/// The values come from `tokens` rather than being written out again. This
/// function used to carry its own copy of six of them, and `chrome.rs` a
/// third copy of four, which meant a palette change was a search.
fn apply_ops_theme(ctx: &egui::Context) {
    use tfg::tokens;
    ctx.all_styles_mut(|style| {
        style.visuals = egui::Visuals::dark();
        let v = &mut style.visuals;
        v.window_fill = tokens::CONSOLE_NIGHT;
        v.window_stroke = egui::Stroke::new(1.0, tokens::HAIRLINE_SLATE);
        // A popup keeps a radius. An island carries a chamfer and paints its
        // own body, so this radius never reaches one.
        v.window_corner_radius = egui::CornerRadius::same(8);
        v.panel_fill = tokens::CONSOLE_NIGHT;
        v.faint_bg_color = tokens::PANEL_SLATE;
        v.extreme_bg_color = tokens::DEEP_WELL;
        v.hyperlink_color = tokens::RADAR_CYAN;
        v.selection.bg_fill = tokens::RADAR_CYAN;
        v.selection.stroke = egui::Stroke::new(1.0, tokens::DEEP_WELL);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
            w.corner_radius =
                egui::CornerRadius::same(tokens::CONTROL_RADIUS);
            w.weak_bg_fill = tokens::BUTTON_GRAPHITE;
        }
        // Hover and press wake up in the accent. Alpha, not `gamma_multiply`,
        // because gamma-multiplying a bright cyan at low intensity lands it
        // in the reds and the wash stops reading as the accent at all.
        let cyan = tokens::RADAR_CYAN;
        v.widgets.hovered.weak_bg_fill =
            egui::Color32::from_rgba_unmultiplied(cyan.r(), cyan.g(), cyan.b(), 40);
        v.widgets.active.weak_bg_fill =
            egui::Color32::from_rgba_unmultiplied(cyan.r(), cyan.g(), cyan.b(), 70);
        style.spacing.item_spacing = tokens::ITEM_SPACING;
        style.spacing.button_padding = tokens::BUTTON_PADDING;
        style.spacing.indent = tokens::INDENT;
    });
}

/// Light label ink for dark tiles (task #37): ship ids and flags read
/// against night water, not against paper.
const MAP_INK: egui::Color32 = egui::Color32::from_rgb(0xE2, 0xE8, 0xF0);

/// Onboarding palette (ticket #77): the clean States A/B canvas —
/// console night into deep well, Radar Cyan as the one signal, deep
/// well as ink on cyan fills (The One Signal Rule, DESIGN.md).
const ONBOARD_TOP: egui::Color32 = egui::Color32::from_rgb(0x0F, 0x17, 0x2A);
const ONBOARD_BOTTOM: egui::Color32 = egui::Color32::from_rgb(0x02, 0x06, 0x17);
const ONBOARD_ACCENT: egui::Color32 = egui::Color32::from_rgb(0x22, 0xD3, 0xEE);
const ONBOARD_INK: egui::Color32 = egui::Color32::from_rgb(0x02, 0x06, 0x17);
/// Auth field/button height (ticket #77 polish): one height for the
/// sign-in card's inputs and its primary button, text centered.
const AUTH_FIELD_H: f32 = 34.0;
/// Phase bar sits under the toolbar (three rows in Simulation ≈ 112px).

fn validate_map_seed(path: &std::path::Path) -> Result<(), String> {
    tfg::map_render::validate_cache(path)
}

fn runtime_self_check(paths: &AppPaths) -> Result<(), String> {
    let catalog = Catalog::from_default_asset()?;
    let fleet = Fleet::from_default_asset()?;
    Land::from_default_asset()?;
    let scenario = if std::env::var("TFG_BACKEND_URL").is_ok() {
        "empty".to_string()
    } else {
        std::env::var("TFG_SCENARIO").unwrap_or_else(|_| "empty".to_string())
    };
    for (name, json) in tfg::assets::BUILT_IN_SCENARIOS {
        FileReplay::from_json(json).map_err(|e| format!("embedded scenario {name}: {e}"))?;
    }
    let replay = FileReplay::from_json(
        tfg::assets::scenario_json(&scenario)
            .ok_or_else(|| format!("unknown built-in scenario {scenario}"))?,
    )?;
    let embedded_seed = paths.data_dir.join(format!(
        "maps/.tfg-seed-check-{}.sqlite",
        std::process::id()
    ));
    std::fs::write(&embedded_seed, tfg::assets::MAP_SEED)
        .map_err(|e| format!("embedded map seed check write failed: {e}"))?;
    let embedded_result = validate_map_seed(&embedded_seed);
    let _ = std::fs::remove_file(&embedded_seed);
    embedded_result?;
    let map_cache = tfg::map_render::prepare_runtime_cache(&paths.map_cache)?;
    validate_map_seed(&map_cache)?;
    let _store = tfg::store::open(&paths.local_db)?;
    println!(
        "tfg {} runtime ok: {} classes, {} fleet units, land loaded, {} replay frames",
        env!("CARGO_PKG_VERSION"),
        catalog.ship_classes().len(),
        fleet.len(),
        replay.frame_count(),
    );
    println!("data: {}", paths.data_dir.display());
    println!("map cache: {}", map_cache.display());
    Ok(())
}

fn main() -> Result<(), String> {
    if std::env::args().any(|arg| arg == "--version") {
        println!("tfg {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // A working-directory .env may select the data directory before paths
    // are resolved; the data-directory file is loaded immediately after.
    ShipApp::load_dotenv(None);
    let paths = AppPaths::discover()?;
    // Endpoints live in optional env files, never UI fields. Load before
    // reading backend/scenario configuration.
    ShipApp::load_dotenv(Some(&paths));
    if std::env::var("TFG_BACKEND_URL").is_err()
        && let Ok(name) = std::env::var("TFG_SCENARIO")
        && tfg::assets::scenario_json(&name).is_none()
    {
        return Err(format!("unknown built-in scenario {name}"));
    }
    if std::env::args().any(|arg| arg == "--check-runtime") {
        return runtime_self_check(&paths);
    }
    if std::env::args().any(|arg| arg == "--probe-gpu") {
        return tfg::gpuprobe::probe();
    }
    let map_cache = tfg::map_render::prepare_runtime_cache(&paths.map_cache)?;
    let initial_log = paths.initial_log.clone();
    let session_seq = tfg::log::next_session_seq(&paths.log_dir);
    // Poll thread owns the backend source; the UI owns the registry.
    // TFG_BACKEND_URL=http://host:port selects the explicit mock, else
    // file replay. The sim joins every round via MergeSource (disarmed
    // = wire only).
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
    // Live-wire fast lane (see WireKind::Live): actor pings, poll thread
    // wakes early. Receiver lives in the poll thread below.
    let (wire_wake_tx, wire_wake_rx) = mpsc::channel::<()>();
    let ui_wire_wake_tx = wire_wake_tx;
    let poll_handle = std::thread::spawn(move || {
        // Boot wire (task #39, M10): TFG_BACKEND_URL points at the
        // explicit mock (examples/mock_backend.rs) — never at Minos,
        // which is the Live wire below. Replay data is embedded; an
        // unknown scenario fails rather than reading a local file.
        let boot_kind = match std::env::var("TFG_BACKEND_URL") {
            Ok(url) => WireKind::Mock(url),
            Err(_) => match std::env::var("TFG_SCENARIO") {
                Ok(name) => WireKind::EmbeddedReplay(name),
                Err(_) => WireKind::EmbeddedReplay("empty".to_string()),
            },
        };
        let (wire, desc) = match build_wire(boot_kind) {
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
                tfg::log::Journal::open(initial_log)
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
                match build_wire(kind) {
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
                // Live-wire fast lane: a queued socket publication cuts the
                // 2 s sleep short, so the marker moves in ~100 ms. The sim
                // cadence is untouched — this only skips idle sleeping.
                if wire_wake_rx.try_iter().count() > 0 {
                    break;
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
        let mut scene = LiveMap::new(CENTER, ZOOM, size.0, size.1, STYLE, map_cache.clone());
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
                scene = LiveMap::new(at, zoom, px.0.max(1), px.1.max(1), STYLE, map_cache.clone());
                size = px;
            }
            scene.set_center(at, zoom);
            // Smoothness measurement: round-trip cost per frame, so tile
            // lag can be split into pump-bound vs render/network-bound.
            let t0 = std::time::Instant::now();
            scene.pump(pump);
            let rgba = scene.frame_rgba();
            eprintln!("map frame {seq}: {}ms (pump {pump})", t0.elapsed().as_millis());
            // Convert off the UI thread: the pump uploads, never swizzles.
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
            if map_resp_tx.send((seq, at, zoom, size, img)).is_err() {
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
        Box::new(move |cc| {
            // Required once: without image loaders, from_bytes fails.
            egui_extras::install_image_loaders(&cc.egui_ctx);
            add_symbol_fallbacks(&cc.egui_ctx);
            apply_ops_theme(&cc.egui_ctx);
            // The shader seam takes egui's own render state: the adapter's
            // device type decides the tier, and `target_format` is the exact
            // format of the pass the halo will be drawn into. Guessing the
            // format would be a validation error at draw time rather than a
            // wrong pixel, so it is taken rather than assumed.
            let fx = match cc.wgpu_render_state.as_ref() {
                Some(state) => tfg::fx::Fx::new(
                    tfg::fx::Quality::from_device_type(state.adapter.get_info().device_type),
                    state.target_format,
                ),
                // No wgpu state (a glow backend, or a creation context that
                // never reached the painter): no shaders, and a placeholder
                // format that is never used because the tier draws nothing.
                None => tfg::fx::Fx::new(
                    tfg::fx::Quality::Low,
                    eframe::egui_wgpu::wgpu::TextureFormat::Rgba8Unorm,
                ),
            };
            eprintln!(
                "fx: {:?} on {:?}",
                fx.quality(),
                cc.wgpu_render_state.as_ref().map(|s| s.target_format)
            );
            // Boot restore (spec-sync ticket): stored spec figures become
            // runtime catalog classes before the first frame, so register
            // hulls placed last session drive again without refetching.
            let mut catalog = Catalog::from_default_asset().expect("catalog asset valid");
            let store = match tfg::store::open(&paths.local_db) {
                Ok(conn) => Some(conn),
                Err(e) => {
                    eprintln!("local store unavailable: {e}");
                    None
                }
            };
            if let Some(conn) = &store {
                match tfg::store::current_figures(conn) {
                    Ok(figs) => {
                        for f in &figs {
                            if f.speed_kn.is_some() {
                                catalog.upsert_runtime_class(
                                    f.class_id,
                                    f.class_name.clone(),
                                    f.version,
                                    f.speed_kn.unwrap_or(0.0),
                                    f.cruise_kn.unwrap_or(0.0),
                                    f.range_nm.unwrap_or(0.0),
                                );
                                // H10: the sim owns a catalog of its own —
                                // restored figures drive it too, or every
                                // restored hull would refuse takeover.
                                let _ = ui_sim_cmd_tx.send(SimCommand::UpsertClass {
                                    minos_class_id: f.class_id,
                                    name: f.class_name.clone(),
                                    version: f.version,
                                    speed_kn: f.speed_kn.unwrap_or(0.0),
                                    cruise_kn: f.cruise_kn.unwrap_or(0.0),
                                    range_nm: f.range_nm.unwrap_or(0.0),
                                });
                            }
                        }
                        if !figs.is_empty() {
                            eprintln!("restored {} spec class(es) from store", figs.len());
                        }
                    }
                    Err(e) => eprintln!("spec restore failed: {e}"),
                }
            }
            let mut app = ShipApp {
                paths: paths.clone(),
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
                last_game_position_at: None,
                fix_animation_started: HashMap::new(),
                animation_paused_fraction: HashMap::new(),
                game_animation_snap: HashSet::new(),
                reconnect_snap: HashSet::new(),
                hidden: HashSet::new(),
                following: None,
                show_trail: true,
                show_grid: true,
                mode: UiMode::new(sim_armed.clone()),
                session_windows: None,
                session_log_path: paths.initial_log.clone(),
                session_seq,
                transcript: Vec::new(),
                show_roster: false,
                roster_pos: egui::pos2(816.0, 64.0),
                show_side_zone: true,
                zone_scroll: 0.0,
                side_dock: tfg::chrome::Dock::Left,
                settings_open: false,
                reduced_motion: false,
                motion_secs: 0.2,
                inspector_pos: egui::pos2(816.0, 440.0),
                orders_pos: egui::pos2(500.0, 250.0),
                login_pos: egui::pos2(8.0, 120.0),
                log_pos: egui::pos2(8.0, 480.0),
                messages_pos: egui::pos2(8.0, 170.0),
                glide: tfg::camera::Glide::new(CENTER),
                dirty: false,
                fx: tfg::fx::Fx::new(tfg::fx::Quality::Low, eframe::egui_wgpu::wgpu::TextureFormat::Rgba8Unorm),
                show_orders: false,
                show_log: false,
                onboard: Onboard::Login,
                sim_ready: false,
                connect_card: false,
                phase_note: None,
                event_feed: VecDeque::new(),
                selection: None,
                last_seen: HashMap::new(),
                fix_count: HashMap::new(),
                sim_cmd_tx: Some(ui_sim_cmd_tx),
                sim_evt_rx,
                order_views: HashMap::new(),
                order_result: HashMap::new(),
                helm_submissions: HashMap::new(),
                helm_drafts: HashMap::new(),
                helm_preview_pending: HashSet::new(),
                controlled: HashSet::new(),
                pending_waypoint: None,
                placing: false,
                order_speed: 20.0,
                land: Land::from_default_asset().ok(),
                order_warning: None,
                helm_warnings: HashMap::new(),
                catalog,
                selected_class: 0,
                fleet: Fleet::from_default_asset().expect("fleet asset valid"),
                fleet_pick: None,
                setup_step: 0,
                assessment_tab: 0,
                setup_name: String::new(),
                setup_description: String::new(),
                setup_purpose: String::new(),
                setup_target: String::new(),
                setup_area: String::new(),
                setup_map_tag: String::new(),
                edit_open: false,
                delete_armed: false,
                edit_name: String::new(),
                edit_description: String::new(),
                edit_purpose: String::new(),
                edit_target: String::new(),
                edit_area: String::new(),
                edit_map_tag: String::new(),
                edit_pace: String::new(),
                edit_actual_date: String::new(),
                edit_actual_start: String::new(),
                edit_actual_end: String::new(),
                edit_assumed_start: String::new(),
                edit_assumed_end: String::new(),
                setup_reg_search: String::new(),
                fleet_query: String::new(),
                drill_branch: None,
                drill_category: None,
                drill_type: None,
                drill_class: None,
                pending_assign: None,
                fleet_cache: Vec::new(),
                fleet_branches: std::collections::HashMap::new(),
                fleet_branch_names: std::collections::HashMap::new(),
                fleet_loaded: false,
                users_game: None,
                users_game_state: None,
                minos_clock: None,
                minos_time_factor: None,
                minos_room_key: None,
                clock_denied: false,
                factor_draft: 1.0,
                users_games: Vec::new(),
                games_gap: false,
                games_loaded: false,
                users_list: Vec::new(),
                users_search: String::new(),
                users_role: None,
                users_roles: Vec::new(),
                users_roster: Vec::new(),
                fleet_picker_open: false,
                player_picker_open: false,
                held_detail: None,
                readiness: None,
                readiness_gap: ReadinessGap::default(),
                modal_panel_rects: Vec::new(),
                scenarios: Vec::new(),
                composer_scenario: None,
                composer_visible: false,
                composer_draft: ComposerDraft::default(),
                composer_draft_error: None,
                scenarios_op: None,
                users_gunits: Vec::new(),
                unassigned_units: std::collections::HashSet::new(),
                commanded_hulls: Vec::new(),
                roster_gap: false,
                units_gap: false,
                placements_gap: false,
                users_placements: Vec::new(),
                placement_unplaced: 0,
                placement_ready: false,
                join_key: String::new(),
                users_status: "hold a session".to_string(),
                users_filter_status: None,
                users_statuses: Vec::new(),
                users_filter_role: None,
                users_approles: Vec::new(),
                upress: None,
                udrag: None,
                unit_drag: None,
                placement_note: PlacementNote::default(),
                map_heading_drag: None,
                placed_fleet: HashSet::new(),
                released_move_drag: false,
                placed_labels: HashMap::new(),
                force: tfg::force::ForceDraft::new(),
                force_seeded_for: None,
                force_queue: Vec::new(),
                force_flush_then: None,
                map_unit_move: None,
                time_real_start: (Utc::now() + chrono::Duration::hours(7)).format("%Y-%m-%d %H:%M").to_string(),
                time_real_end: (Utc::now() + chrono::Duration::hours(14)).format("%Y-%m-%d %H:%M").to_string(),
                time_game_start: "2026-11-01 00:00".to_string(),
                time_game_end: "2026-11-07 00:00".to_string(),
                helm: HashMap::new(),
                unit_commander: HashMap::new(),
                // Minos REST base: full URL wins, else host with derived
                // :8080/api/v1 (transport ticket), else the hosted dev.
                minos_base: match std::env::var("TFG_MINOS_HOST") {
                    Ok(h) if h.contains("://") => h.trim_end_matches('/').to_string(),
                    Ok(h) => format!(
                        "http://{}/api/v1",
                        h.trim_end_matches('/').trim_start_matches("http://").trim_start_matches("https://")
                    ),
                    Err(_) => "https://api.tfg.development.crossnet.co.id/api/v1".to_string(),
                },
                show_login: false,
                login_identifier: String::new(),
                login_password: String::new(),
                auth_user: None,
                auth_user_id: None,
                auth_identity: None,
                auth_token: None,
                auth_issued_at: None,
                auth_ttl_secs: 0,
                auth_refresh_memory: None,
                auth_degraded: false,
                auth_status: "signed out".to_string(),
                auth_needs_password_change: false,
                pw_current: String::new(),
                pw_new: String::new(),
                store,
                sync_status: "never synced".to_string(),
                session_ratio: SESSION_RATIO,
                zoom: ZOOM,
                last_zoom_req: Instant::now(),
                last_track_req: Instant::now(),
                zoom_dirty: false,
                text_scale: 1.0,
                base_ppp: None,
                app_mode: AppMode::Simulation,
                // Boot is the lobby: the mode toggle plus the setup flow.
                wire_ctl_tx: Some(ui_wire_ctl_tx),
                wire_wake_tx: ui_wire_wake_tx,
                live_cmd_tx: None,
                live_evt_rx: None,
                live_connected_once: false,
                live_status: "idle".to_string(),
                live_state: LinkState::Idle,
                login_op: None,
                refresh_op: None,
                sync_op: None,
                spec_op: None,
                pw_op: None,
                plot_op: None,
                setup_op: None,
                pending_setup: Vec::new(),
                pending_transition: None,
                last_plot_try: None,
                last_game_sync: None,
                last_plot_ok: None,
                plot_fails: 0,
                log_view_path: None,
                log_files: Vec::new(),
                log_files_loaded: false,
                log_view: None,
                log_op: None,
                game_messages: VecDeque::new(),
                show_messages: false,
                inbox: Vec::new(),
                inbox_mine_only: false,
                inbox_page_no: 1,
                inbox_total: 0,
                inbox_pages: 1,
                inbox_has_next: false,
                inbox_has_prev: false,
                msg_open: None,
                timeline_events: Vec::new(),
                timeline_cursor: None,
                timeline_has_more: false,
                timeline_source: None,
                timeline_personnel: String::new(),
                timeline_unit: String::new(),
                timeline_from: String::new(),
                timeline_to: String::new(),
                judgements: Vec::new(),
                judge_subject: None,
                judge_subject_id: String::new(),
                judge_score: String::new(),
                reviews: Vec::new(),
                review_subject: None,
                review_body: String::new(),
                review_editing: None,
                review_mine_only: false,
                minos_tree: Vec::new(),
                tree_gap: false,
                visuals: VisualCache::default(),
                inspector_image_width: INSPECTOR_IMAGE_DEFAULT_WIDTH,
                unit_symbols: HashMap::new(),
                unit_type_ids: HashMap::new(),
                unit_type_names: HashMap::new(),
                unit_type_symbols: HashMap::new(),
                unit_branches: HashMap::new(),
                affiliations: tfg::store::AffiliationResolver::default(),
                unit_lods: HashMap::new(),
                group_representations: HashMap::new(),
                heading_overrides: HashMap::new(),
                pending_image_urls: Vec::new(),
                image_op: None,
                image_request: None,
                manifest_retry_at: None,
                manifest_refresh_at: None,
                msg_kind: "telegram".to_string(),
                msg_class: "TERBUKA".to_string(),
                msg_content: String::new(),
                msg_callsign: String::new(),
                msg_sending_note: String::new(),
                msg_group: String::new(),
                msg_per: String::new(),
                msg_regnum: String::new(),
                msg_to: HashSet::new(),
                msg_cc: HashSet::new(),
                msg_degree: None,
                msg_degrees: Vec::new(),
                msg_assumed: None,
                msg_reply_to: None,
                scenario_roles: Vec::new(),
                new_role_name: String::new(),
                log_events: Vec::new(),
                replay_pos: 0,
                show_replay: true,
                log_filter: "all".to_string(),
                groups: Groups::default(),
                acting_as: None,
                desktop: "all".to_string(),
                drafts: HashMap::new(),
                game_elapsed_secs: None,
                game_ratio: 1.0,
                game_paused: false,
                real_ts: None,
                game_ts: None,
            };
            app.fx = fx;
            app.reload_unit_symbols();
            app.reload_affiliations();
            // M3: wake the last session when its refresh token survived
            // in the keyring — a cold launch otherwise asks for login.
            app.restore_session();
            Ok(Box::new(app))        }),
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- the Fleet picker's column set --------------------------------------
    //
    // Two defects, one cause. The drill is five levels deep and `tax_units` is
    // the leaf, but only four levels were columns: the fifth was a list drawn
    // UNDER the columns, in a modal body that is a fixed rect with no scroll,
    // so it was painted past the panel's bottom edge and nothing in the panel
    // could bring it on screen. The leaf was not misplaced, it was
    // unreachable, and no unit could be picked or dragged at all.
    //
    // These are arithmetic, so they are pinned without a renderer. What a
    // screenshot cannot tell you — that the five columns fit the panel's
    // width, and that the tail's reserve survives the strip's height — is
    // exactly what a screenshot showed as "looks fine".

    /// The five columns fit the panel, with the leaf wider than the rest.
    ///
    /// The leaf is wider because its rows carry a thumbnail, a name and a verb
    /// rather than a name alone. The failure this pins is silent and severe:
    /// the columns are in a horizontal scroll area, so five that do not fit do
    /// not clip, they push the leaf off the right of the panel behind a
    /// scrollbar nobody is going to find for a list they can see is cut off.
    #[test]
    fn drill_columns_fit_the_picker_width() {
        let body = FLEET_PICKER_SIZE.x - 2.0 * tfg::tokens::PAD;
        let columns = 4.0 * MILLER_COL_WIDTH + MILLER_LEAF_WIDTH;
        // Four separators between five columns, plus the frame each column
        // reserves against its own scrollbar.
        let chrome = 4.0 * (tfg::tokens::ITEM_SPACING.x + 18.0);
        assert!(
            columns + chrome <= body,
            "five columns need {:.0}pt of a {:.0}pt body",
            columns + chrome,
            body
        );
        assert!(
            MILLER_LEAF_WIDTH > MILLER_COL_WIDTH,
            "the leaf holds a thumbnail and a verb; the others hold a name"
        );
    }

    /// The count and the placement controls stay on the panel.
    ///
    /// The bug, as a number. The strip used to take a stored height the
    /// operator could raise with a slider, and every height past the panel's
    /// remaining space pushed the tail off the bottom — where, again, nothing
    /// could reach it. So the strip is now given what is left over the tail's
    /// reserve, and this is the property that makes that true at the panel's
    /// own size rather than at whatever the window happened to be.
    ///
    /// Run against a RANGE of row heights rather than one, because the row
    /// height is a font measurement and the point of measuring it is that the
    /// layout does not care what it comes out at. `16.0` is a small label and
    /// `32.0` a large one; the budget has to hold for both.
    /// Rows above the columns — search, status, commander, breadcrumbs.
    ///
    /// Modelled here rather than declared in the layout, because the layout
    /// does not use it: the live path asks egui how much room is left, which is
    /// the honest number and the reason the strip cannot drift. This is what
    /// stands in for that measurement so the arithmetic can be checked without
    /// a renderer.
    const HEADER_ROWS_MODELLED: f32 = 96.0;

    #[test]
    fn the_picker_tail_stays_on_the_panel() {
        // The TIGHTEST panel, not the declared one: the height is now a
        // function of the window, and the small window is where the columns and
        // the tail compete for the same points.
        let panel_h = fleet_picker_size(640.0).y;
        assert!(
            panel_h < FLEET_PICKER_SIZE.y,
            "a 640pt window must leave map for the drag to land on"
        );
        let body = panel_h - tfg::chrome::MODAL_TITLE_H - 2.0 * tfg::tokens::PAD;
        let available = body - HEADER_ROWS_MODELLED;
        for row in [12.0_f32, 16.0, 20.0, 24.0, 32.0] {
            let reserve = PICKER_TAIL_ROWS * row;
            let strip = picker_strip_height(available, row);
            assert!(
                strip + reserve <= available,
                "at a {:.0}pt row: strip {:.0} + tail {:.0} exceeds the {:.0}pt left",
                row,
                strip,
                reserve,
                available
            );
            assert!(
                available - reserve > PICKER_STRIP_MIN,
                "at a {:.0}pt row the tail reserve leaves room on the tightest panel",
                row
            );
        }
    }

    /// The picker always leaves a droppable strip of map.
    ///
    /// Without this the drag has no target: a centred 980x620 panel in a 1040x640
    /// window leaves 30pt of map, every drop on it was refused for land, and the
    /// refusal was invisible. The map band is what makes the gesture the design
    /// is built around performable at all.
    #[test]
    fn the_picker_always_leaves_room_to_drop_on() {
        for window in [480.0_f32, 640.0, 800.0, 1000.0, 1400.0] {
            let panel = fleet_picker_size(window);
            let free = window - panel.y;
            assert!(
                free >= PICKER_MAP_BAND - 1.0 || panel.y <= PICKER_MIN_H,
                "a {:.0}pt window leaves only {:.0}pt of map beside the picker",
                window,
                free
            );
            assert!(panel.x <= FLEET_PICKER_SIZE.x, "the width never grows");
        }
    }

    /// A short window degrades to a shorter strip, not to a missing tail.
    ///
    /// The floor exists so a cramped panel shows a usable column, and this is
    /// the direction it is allowed to fail in: the strip gives way, because
    /// the alternative is the count and the placement controls going off the
    /// bottom again.
    #[test]
    fn a_short_panel_shortens_the_strip() {
        let row = 18.0;
        let cramped = PICKER_TAIL_ROWS * row + 40.0;
        assert_eq!(picker_strip_height(cramped, row), PICKER_STRIP_MIN);
        // Below the floor the tail is still owed its reserve, so the strip
        // cannot go under the floor without the tail being the thing at risk.
        assert!(picker_strip_height(0.0, row) >= PICKER_STRIP_MIN);
    }

    // -- the execution gate -------------------------------------------------
    //
    // The client used to list four of the server's five conditions and had no
    // way to author the fifth, so "0 outstanding" could not mean what it said:
    // an exercise with a force, a placement and nobody ready still showed
    // clear, and the server refused the advance with a 409 naming none of it.
    //
    // The table mirrors the server's own, so a row added there has a row to be
    // added here.

    /// A window with both ends declared, which is the fifth condition's whole
    /// requirement.
    fn window_set() -> tfg::backend::TimeWindow {
        tfg::backend::TimeWindow {
            actual_start: Some("2026-11-01T08:00:00Z".into()),
            actual_end: Some("2026-11-01T18:00:00Z".into()),
            assumed_start: Some("2000-01-01T06:00:00Z".into()),
            assumed_end: Some("2000-01-01T16:00:00Z".into()),
        }
    }

    /// A window the gate cannot accept: no declared finish, which is the
    /// ordinary state of a draft because the backend supplies no default.
    fn no_window() -> tfg::backend::TimeWindow {
        Default::default()
    }

    struct Row {
        name: &'static str,
        window: tfg::backend::TimeWindow,
        pace: Option<tfg::backend::GamePace>,
        pieces: usize,
        unplaced: i64,
        fighters: usize,
        ready: usize,
        want: &'static [&'static str],
        why: &'static str,
    }

    /// One row per condition the server checks, asserted as a BLANKER LIST
    /// rather than a boolean so the wording is pinned with the rule — a gate
    /// that fires on the right conditions while saying nothing useful is
    /// still the failure this fix exists to end.
    ///
    /// `unplaced` is left at zero unless it is the point, because zero is
    /// what the field means when nothing is unplaced, and a case that had to
    /// set it to pass would be asserting the zero value instead of the rule.
    // -- the checklist, which is no longer a rule ---------------------------

    /// The checklist is the SERVER'S WORDS. A client that rebuilt them would be
    /// a second copy of the gate, which is the thing this change exists to end
    /// — and the copy was already wrong once.
    #[test]
    fn the_checklist_passes_the_servers_words_through() {
        let view = tfg::backend::GameReadinessView {
            can_execute: false,
            blockers: vec![
                "the planned window is incomplete (both actual_end and assumed_end are required)"
                    .to_string(),
                "2 of 3 unit(s) have not been placed on the map".to_string(),
            ],
            fast: false,
        };
        assert_eq!(view.blockers.len(), 2);
        assert!(!view.names_readiness(), "neither blocker is about readiness");
    }

    /// An unread or unrun gate must NEVER read as clear.
    ///
    /// Empty means "nothing is outstanding", and that is the one answer that
    /// must never be invented. This branch runs before the first bundle lands
    /// and after a failed read, so an empty list here would offer the advance
    /// on a game nobody has checked.
    #[test]
    fn an_unknown_gate_refuses_rather_than_reads_as_clear() {
        assert!(!unknown_gate_blockers(ReadinessGap::NotRead).is_empty(), "never run yet");
        assert!(!unknown_gate_blockers(ReadinessGap::Unread).is_empty(), "read failed");
    }

    /// A server that answers 404 does not publish the check, so the message
    /// must NOT tell the operator to retry. Verified against the deployed dev
    /// API, which 404s on `/games/{id}/readiness` while every route beside it
    /// answers: the client is ahead of its server, and a retry cannot fix that.
    #[test]
    fn an_absent_route_is_not_advice_to_retry() {
        let absent = tfg::backend::BackendError::Api {
            status: 404,
            message: "Not Found".to_string(),
            detail: None,
        };
        let gap = ReadinessGap::of(&absent);
        assert_eq!(gap, ReadinessGap::Absent, "a 404 is not a transport hiccup");
        let msg = &unknown_gate_blockers(gap)[0];
        assert!(!msg.contains("retry"), "a retry cannot conjure the route: {msg}");
        assert!(msg.contains("does not publish"), "{msg}");
    }

    /// The other failures keep their advice, because another attempt can fix
    /// them. Only the 404 loses the word.
    #[test]
    fn a_transient_failure_still_earns_the_retry() {
        let other = tfg::backend::BackendError::Api {
            status: 503,
            message: "unavailable".to_string(),
            detail: None,
        };
        assert_eq!(ReadinessGap::of(&other), ReadinessGap::Unread);
        assert!(unknown_gate_blockers(ReadinessGap::Unread)[0].contains("retry"));
    }

    /// The two unknown states say DIFFERENT things, because they are different
    /// events. "Has not run yet" is normal on first paint; "could not be read"
    /// is a degraded sync the operator may need to retry. Folding them into one
    /// string loses the only actionable half.
    #[test]
    fn an_unread_gate_says_so_and_names_the_remedy() {
        assert!(unknown_gate_blockers(ReadinessGap::Unread)[0].contains("retry"));
        assert!(!unknown_gate_blockers(ReadinessGap::NotRead)[0].contains("retry"));
    }

    // -- the composer's draft ---------------------------------------------
    //
    // The draft is the only place the author can be told what is wrong with
    // a step BEFORE the round trip, so its rules are worth pinning: a
    // refusal that only arrives as a 400 from the field is a refusal the
    // author has to guess the cause of.

    /// No window is the default and is always fine. Both ends empty is what
    /// an unauthored window looks like, and it is what the backend takes.
    #[test]
    fn a_draft_with_no_window_is_valid() {
        let d = ComposerDraft {
            content: "sweep north".into(),
            ..Default::default()
        };
        assert_eq!(d.window(), None);
        assert_eq!(d.problem(), None);
    }

    /// Half a window is the case that bites: the backend refuses it, so the
    /// author is told now rather than after finishing the form.
    #[test]
    fn half_a_window_is_refused_before_the_request() {
        let half_start = ComposerDraft {
            start_hour: "1000".into(),
            ..Default::default()
        };
        assert_eq!(
            half_start.problem().as_deref(),
            Some("a window needs an end as well as a start")
        );

        let half_end = ComposerDraft {
            end_hour: "1030".into(),
            ..Default::default()
        };
        assert_eq!(
            half_end.problem().as_deref(),
            Some("a window needs a start as well as an end")
        );
    }

    /// Malformed and backwards windows are named as such, and the message
    /// quotes what was typed, because "not a military time" without the
    /// offending value leaves the author hunting.
    #[test]
    fn a_bad_window_says_which_value_is_bad() {
        let bad_start = ComposerDraft {
            start_hour: "900".into(),
            end_hour: "1030".into(),
            ..Default::default()
        };
        assert!(bad_start
            .problem()
            .unwrap()
            .starts_with("900 is not a military time"));

        let bad_end = ComposerDraft {
            start_hour: "1000".into(),
            end_hour: "25:00".into(),
            ..Default::default()
        };
        assert!(bad_end
            .problem()
            .unwrap()
            .starts_with("25:00 is not a military time"));

        let backwards = ComposerDraft {
            start_hour: "1030".into(),
            end_hour: "1000".into(),
            ..Default::default()
        };
        assert_eq!(
            backwards.problem().as_deref(),
            Some("a window's end must be after its start")
        );
    }

    /// A good window comes through as the pair the request expects, not as
    /// the two fields re-derived at the call site.
    #[test]
    fn a_good_window_becomes_the_pair_the_request_takes() {
        let d = ComposerDraft {
            content: "".into(),
            start_hour: "1000".into(),
            end_hour: "1030".into(),
        };
        assert_eq!(d.problem(), None);
        assert_eq!(d.window(), Some(("1000", "1030")));
    }

    /// An empty step is a legitimate authoring state, not something the
    /// draft refuses. The backend says so explicitly, and a client that
    /// insisted on text would block an author from saving a skeleton.
    #[test]
    fn an_empty_step_is_not_the_drafts_business() {
        let d = ComposerDraft::default();
        assert_eq!(d.problem(), None, "content is the server's rule, not the draft's");
    }

    // -- the picker's drop target -------------------------------------------
    //
    // The map is full-bleed under the floating zones, so its rect contains
    // the picker panel. That containment is the whole hazard: without the
    // exception below, a drag released over the form places a hull at
    // whatever sea lies behind it.

    /// The panel these tests place: 400x300 at (100, 100).
    fn picker_panel() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(400.0, 300.0))
    }

    /// A release over a panel is NOT a release over the map.
    ///
    /// `Rect::contains` is half-open, so the top-left corner is inside and a
    /// point exactly on the bottom-right edge is not. Both boundaries are
    /// pinned, because that edge is where a hull ends up one pixel from where
    /// the operator let go of it.
    #[test]
    fn a_release_over_a_panel_is_not_a_drop() {
        let panels = [picker_panel()];
        for pos in [
            egui::pos2(300.0, 250.0), // centre
            egui::pos2(100.0, 100.0), // top-left corner: inside
            egui::pos2(499.0, 399.0), // just inside bottom-right
            egui::pos2(100.0, 399.0), // bottom-left edge
        ] {
            assert!(
                !drop_lands_on_map(&panels, pos),
                "{pos:?} is inside the picker and must not place"
            );
        }
        for pos in [
            egui::pos2(99.0, 250.0),  // just left of it
            egui::pos2(500.5, 250.0), // just right of it
            egui::pos2(300.0, 99.0),  // just above it
            egui::pos2(300.0, 400.5), // just below it
        ] {
            assert!(
                drop_lands_on_map(&panels, pos),
                "{pos:?} is outside the picker and must place"
            );
        }
    }

    /// With no panels there is nothing to refuse, so placement behaves
    /// exactly as it did before either picker existed. A stale rect would
    /// silently make a region of the map unplaceable, so the empty case has
    /// to be the permissive one.
    #[test]
    fn no_panels_refuse_nothing() {
        for pos in [
            egui::pos2(0.0, 0.0),
            egui::pos2(1919.0, 1079.0),
            egui::pos2(960.0, 540.0),
        ] {
            assert!(drop_lands_on_map(&[], pos));
        }
    }

    /// Two panels, not one: the Fleet picker owns hull drags and the Player
    /// picker owns person drags, and a single slot would mean whichever drew
    /// last silently disarmed the other. A point inside EITHER is refused.
    #[test]
    fn every_open_panel_refuses() {
        let fleet = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(400.0, 300.0));
        let players = egui::Rect::from_min_size(egui::pos2(600.0, 400.0), egui::vec2(400.0, 300.0));
        let panels = [fleet, players];
        // Each panel's own centre, checked against the OTHER panel present:
        // a one-element list would pass both of these.
        assert!(!drop_lands_on_map(&panels, egui::pos2(300.0, 250.0)));
        assert!(!drop_lands_on_map(&panels, egui::pos2(800.0, 550.0)));
        // A point in neither still lands, even though both panels are open.
        assert!(drop_lands_on_map(&panels, egui::pos2(50.0, 900.0)));
        assert!(drop_lands_on_map(&panels, egui::pos2(1500.0, 200.0)));
    }

    /// A read-only roster and an editable one must never appear in the same
    /// frame. egui keys a widget by `Id`, so two per-row ComboBoxes salted
    /// `("roster-role", user_id)` are ONE widget: one popup, opened from the
    /// wrong copy, with a click that lands in the other. The island is
    /// therefore read-only and the modal owns every verb.
    ///
    /// A source-level check rather than a render, because the failure is an
    /// Id collision and an Id collision does not show up as anything a
    /// screenshot would make obvious — the form looks right and the wrong one
    /// responds.
    #[test]
    fn the_editable_roster_is_only_in_the_modal() {
        // Assembled from parts so this test does not spell the call it is
        // counting — not even in a comment, since a comment is in the file
        // too and the count would never settle.
        let needle = ["self.", "roster_editor_ui(", "&mut"].concat();
        let src = include_str!("main.rs");
        let calls = src.split(needle.as_str()).count() - 1;
        assert_eq!(
            calls, 1,
            "the editable roster must be called from exactly one place"
        );
        // And the island's copy takes no ComboBox, which is what makes it
        // safe to draw while the modal is open.
        let island = src
            .split("fn users_roster_ui(")
            .nth(1)
            .expect("the island roster exists")
            .split("fn seated_without_command(")
            .next()
            .expect("it ends where the next fn begins");
        assert!(
            !island.contains("ComboBox"),
            "the island roster must stay read-only"
        );
        assert!(
            !island.contains("small_button"),
            "the island roster must carry no verbs"
        );
    }

    /// Every way to CREATE a session renders a name field.
    ///
    /// Found the hard way: with no session held, the only island on screen
    /// offered "Start a new session", which called the create verb with
    /// nothing filled and answered "name the game first" — pointing at a form
    /// that only renders once a session is HELD. The way in was a verb that
    /// could not succeed.
    ///
    /// Source-level rather than a render, because the bug is about which
    /// function renders the field, and that is not a thing a screenshot shows:
    /// the island looked complete and the refusal came from three clicks later.
    ///
    /// The needle is assembled so this test cannot satisfy itself: the check
    /// walks functions, and a function that merely MENTIONS both strings would
    /// pass for free.
    #[test]
    fn every_create_verb_has_a_name_field_on_screen() {
        let create = ["self.", "setup_create_game()"].concat();
        let field = ["setup_", "name"].concat();
        let src = include_str!("main.rs");
        let mut checked = 0;
        for body in src.split("\n    fn ").skip(1) {
            if !body.contains(create.as_str()) {
                continue;
            }
            let name = body.split(['(', ' ']).next().unwrap_or("").to_string();
            assert!(
                body.contains(field.as_str()),
                "`{name}` calls the create verb but renders no {field} field, so it can only ever refuse with name-the-game-first"
            );
            checked += 1;
        }
        assert!(checked >= 1, "the check found no create call site to test");
    }

    // -- the console does not introduce itself ---------------------------

    /// Nothing on the login path asks which mode the app is in.
    ///
    /// The mode card existed to offer "Command Center or Tactical Floor
    /// Game?" before the map was up, which is the app naming its own
    /// function to the person about to use it. A source-level check,
    /// because the regression is easy and invisible: reintroducing the
    /// card compiles, runs, and looks deliberate.
    #[test]
    fn no_login_path_offers_a_mode_choice() {
        let src = include_str!("main.rs");
        // Assembled from parts so this test is not itself a match — not even
        // in a comment, because a comment is in the file too.
        let card = ["onboard", "_mode"].concat();
        assert!(
            !src.contains(card.as_str()),
            "the mode card is back; Simulation is announcing itself again"
        );
        // Two variants, not three: Login and App.
        let body = src
            .split("enum Onboard {")
            .nth(1)
            .expect("Onboard exists")
            .split('}')
            .next()
            .expect("the enum body");
        assert_eq!(
            body.matches("Login").count() + body.matches("App").count(),
            2,
            "Onboard must be Login and App and nothing else"
        );
    }

    /// The column and the free-floating islands must never both carry a
    /// surface. Two copies of the Orders panel is not two views of one
    /// thing — it is the operator editing one and looking at the other.
    ///
    /// Asserted per SURFACE rather than as "the zone is showing", because
    /// Execution carries Orders, Crew and Log while Planning carries none of
    /// them, and Presentation has no column at all. A single blanket gate
    /// would either double-draw in Planning or blank Presentation's islands
    /// the moment the zone was switched on.
    #[test]
    fn a_surface_is_in_the_column_or_floating_never_both() {
        let src = include_str!("main.rs");

        // The floating copies are all gated on the same predicate. Spelled
        // with the receiver so this test is not itself one of the matches.
        let gate = ["self.", "zone_carries("].concat();
        let gated = src.matches(gate.as_str()).count();
        // One per suppressed floating island: orders and log.
        assert_eq!(gated, 2, "orders and log must each check the column");

        // And the predicate is the ONLY thing that decides: no floating
        // island may decide by asking the game state directly, which is how
        // the two copies drifted apart before.
        // The floating Roster is deliberately absent: it lists ships, and the
        // column's Crew lists people. Naming it here would assert a
        // duplication that does not exist.
        for island in ["show_orders", "show_log"] {
            let arms = src
                .split(&format!("if self.{island}\n"))
                .nth(1)
                .unwrap_or_else(|| panic!("{island} is drawn somewhere"));
            let head = arms.lines().take(8).collect::<Vec<_>>().join("\n");
            assert!(
                head.contains("zone_carries"),
                "{island} does not ask whether the column already carries it"
            );
        }
    }

    /// Settings offers a VIEW, not a peer mode. Presentation is a way of
    /// watching; a "Simulation" button beside it would put the two on equal
    /// footing as if they were things you choose between.
    #[test]
    fn settings_offers_a_view_rather_than_a_mode_choice() {
        let src = include_str!("main.rs");
        let settings = src
            .split("fn settings_modal(")
            .nth(1)
            .expect("the settings modal exists");
        let labelled = settings.split("ui.label(\"View\")").nth(1);
        assert!(
            labelled.is_some(),
            "the mode switch should read as a view toggle"
        );
        let arm = labelled.expect("checked above");
        assert!(
            arm.contains("Watch only"),
            "the toggle should name what it turns on, not the mode it leaves"
        );
    }

    /// The only positive fact the helpers mirror carries about a game role
    /// is `is_judge_side`, so "may need a command" is exactly its negation.
    /// A test that says anything more would be asserting a flag the schema
    /// does not have.
    #[test]
    fn only_the_judge_side_is_known_not_to_need_a_command() {
        assert!(!role_may_need_command(true), "judge side never commands");
        assert!(role_may_need_command(false), "everything else may");
    }

    fn entry(unit_id: i64, w: Option<u32>, h: Option<u32>) -> tfg::backend::UnitImageEntry {
        tfg::backend::UnitImageEntry {
            asset_kind: "unit_image".to_string(),
            unit_id,
            name: format!("hull {unit_id}"),
            hull_number: None,
            file_name: format!("images/{unit_id}.png"),
            content_type: "image/png".to_string(),
            size_bytes: 1024,
            width_px: w,
            height_px: h,
            loa_m: None,
            beam_m: None,
        }
    }

    fn manifest(version: &str, entries: Vec<tfg::backend::UnitImageEntry>) -> tfg::backend::ImageManifest {
        tfg::backend::ImageManifest {
            version: version.to_string(),
            entry_count: entries.len(),
            units_without_image: 0,
            entries,
        }
    }

    fn symbol(_: i64) -> tfg::store::MapSymbol {
        tfg::store::MapSymbol::Corvette
    }

    /// Planning must show a local unit as soon as it is placed; the Sim filter
    /// then keeps wire traffic out until the session is live.
    #[test]
    fn simulation_planning_accepts_local_movement_fixes() {
        assert!(accepts_movement_fixes(AppMode::Simulation, Phase::Setup));
        assert!(accepts_movement_fixes(AppMode::Simulation, Phase::Live));
        assert!(!accepts_movement_fixes(AppMode::Simulation, Phase::Closed));
        assert!(accepts_movement_fixes(AppMode::Presentation, Phase::Closed));
    }

    /// A selected fleet pick remains a valid map-placement intent even when
    /// another UI state transition resets the visual tool hint.
    #[test]
    fn selected_fleet_pick_stays_map_clickable_after_tool_reset() {
        assert!(placement_click_allowed(Phase::Setup, Some("unit-1"), true, None));
        assert!(placement_click_allowed(Phase::Live, Some("unit-1"), true, None));
        assert!(!placement_click_allowed(Phase::Setup, None, true, None));
        assert!(!placement_click_allowed(Phase::Closed, Some("unit-1"), true, None));
        assert!(!placement_click_allowed(Phase::Setup, Some("unit-1"), false, None));
        assert!(!placement_click_allowed(Phase::Setup, Some("unit-1"), true, Some("observer")));
    }

    /// Inspector images stay compact by default, preserve their source
    /// aspect ratio, and never grow taller than the compact preview cap.
    #[test]
    fn inspector_image_size_is_bounded_and_aspect_preserving() {
        let wide = inspector_image_size(egui::vec2(900.0, 111.0), 280.0, 320.0);
        assert!((wide.x - 280.0).abs() < 0.01);
        assert!((wide.y - 111.0 / 900.0 * 280.0).abs() < 0.01);

        let tall = inspector_image_size(egui::vec2(111.0, 900.0), 280.0, 320.0);
        assert!((tall.y - INSPECTOR_IMAGE_MAX_HEIGHT).abs() < 0.01);
        assert!(tall.x < 280.0);
        assert!(tall.x > 0.0);

        let narrow_panel = inspector_image_size(egui::vec2(900.0, 111.0), 280.0, 120.0);
        assert!((narrow_panel.x - 120.0).abs() < 0.01);
    }

    /// Every map symbol owns a distinct far geometry. This is the
    /// visual half of the universal-fallback contract: adding a
    /// symbol without a shape cannot silently leave another anonymous
    /// dot behind.
    #[test]
    fn every_map_symbol_has_a_distinct_shape_and_fill() {
        let identities: std::collections::HashSet<_> = tfg::store::MapSymbol::ALL
            .into_iter()
            .map(|symbol| {
                let shape = map_symbol_shape(symbol);
                (shape, map_symbol_accent(shape).to_array())
            })
            .collect();
        assert_eq!(identities.len(), tfg::store::MapSymbol::ALL.len());
        assert_eq!(map_symbol_shape(tfg::store::MapSymbol::UnknownShip), MapSymbolShape::Dot);
    }

    /// The glyph ink must stay readable against the fill it is painted on.
    ///
    /// The ten type accents were originally blended only 32 percent into a
    /// mid-tone base, which measured between 1.02 and 2.11 contrast against the
    /// map's own water and land — `Auxiliary` at 1.02 is the sea, drawn on the
    /// sea. Lightening them onto the frame's dark interior fixed that, and
    /// nothing else would notice if it regressed: no other test touches the
    /// rendered colour, and the symbols still look like symbols when they are
    /// too faint to read.
    ///
    /// So this is a contrast check, against the fill the glyph actually sits
    /// on rather than against the map. 3.0 is the floor the map's own tokens
    /// use for ink on a panel; anything under it and the shape stops being a
    /// channel.
    #[test]
    fn the_frame_outline_reads_on_both_of_its_edges() {
        // An outline has two edges and is visible in proportion to the WEAKER
        // of them. This test exists because only one of them was ever checked:
        // the hue clears 3.10:1 against its own fill and 1.03:1 against the
        // sea, so the frame's real legibility was the sea — and the fill is
        // what a reviewer looks at when they ask whether a symbol is clear.
        //
        // The casing is what fixes the outer edge, so the property to hold is
        // the FULL stack: hue against keyline (is the hue readable at all),
        // keyline against both terrains (is the hue findable at all), and the
        // glyph against the fill (did we spend the fill's contrast to get
        // here). 3.0 is the floor the map's own tokens use for ink on a panel.
        let sea = egui::Color32::from_rgb(0xa8, 0xd0, 0xf0);
        let land = egui::Color32::from_rgb(0xe8, 0xe0, 0xd0);
        for affiliation in tfg::store::Affiliation::ALL {
            let hue = affiliation_ink(affiliation);
            let fill = affiliation_fill(affiliation, false);
            assert!(
                contrast_ratio(hue, FRAME_KEYLINE) >= 3.0,
                "{affiliation:?} hue is {:.2}:1 on its keyline",
                contrast_ratio(hue, FRAME_KEYLINE)
            );
            for (what, terrain) in [("sea", sea), ("land", land)] {
                assert!(
                    contrast_ratio(FRAME_KEYLINE, terrain) >= 3.0,
                    "{affiliation:?} keyline is {:.2}:1 against {what}",
                    contrast_ratio(FRAME_KEYLINE, terrain)
                );
            }
            assert!(
                contrast_ratio(hue, fill) >= 3.0,
                "{affiliation:?} hue is {:.2}:1 on its own fill",
                contrast_ratio(hue, fill)
            );
        }
        // The casing must actually show outside the hue, or the whole arrangement is
        // a wider single line with extra steps.
        assert!(
            FRAME_KEYLINE_OUT_PX > 0.0,
            "a casing with no outer sliver protects nothing"
        );
        assert_eq!(FRAME_KEYLINE_PX, FRAME_STROKE_PX + FRAME_KEYLINE_OUT_PX);
        // AND it must not be drawn as a wider CENTRED stroke, which is the
        // version that works in the abstract and eats the glyph on screen:
        // centred, a casing this wide extends FRAME_KEYLINE_OUT_PX / 2 into
        // the interior, which measured 55 percent of the glyph's pixels gone.
        // The offset must therefore equal half the difference between the two
        // widths, which is what makes the casing sit wholly outside.
        assert_eq!(
            FRAME_KEYLINE_OFFSET_PX,
            (FRAME_KEYLINE_PX - FRAME_STROKE_PX) / 2.0
        );
        // The run offset is what actually protects the interior: push a point
        // outward and it must move, and only outward.
        let center = egui::pos2(0.0, 0.0);
        let run = vec![egui::pos2(10.0, 0.0), egui::pos2(0.0, 10.0)];
        let cased = offset_run_outward(&run, center, 2.0);
        for (original, moved) in run.iter().zip(&cased) {
            let before = (*original - center).length();
            let after = (*moved - center).length();
            assert!(
                after > before,
                "the casing moved a point inward: {before} -> {after}"
            );
        }
        // A degenerate point at the centre has no outward direction and must be
        // left alone rather than producing a NaN coordinate.
        let at_centre = offset_run_outward(&[center], center, 2.0);
        assert!(at_centre[0] == center, "a point on the centre moved");
        // A stale hull greys its casing but must keep an edge that reads,
        // because "this hull stopped reporting" is still a thing to see.
        for (what, terrain) in [("sea", sea), ("land", land)] {
            assert!(
                contrast_ratio(FRAME_KEYLINE_STALE, terrain) >= 3.0,
                "the stale keyline is {:.2}:1 against {what}",
                contrast_ratio(FRAME_KEYLINE_STALE, terrain)
            );
        }
    }

    /// WCAG relative-luminance contrast between two opaque colours.
    fn contrast_ratio(a: egui::Color32, b: egui::Color32) -> f32 {
        fn channel(v: u8) -> f32 {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        let lum = |c: egui::Color32| {
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        };
        let (hi, lo) = {
            let (x, y) = (lum(a), lum(b));
            if x > y {
                (x, y)
            } else {
                (y, x)
            }
        };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn every_glyph_ink_reads_against_every_affiliation_fill() {
        for affiliation in tfg::store::Affiliation::ALL {
            let fill = affiliation_fill(affiliation, false);
            for symbol in tfg::store::MapSymbol::ALL {
                let ink = map_symbol_fill(map_symbol_shape(symbol), false);
                let ratio = contrast_ratio(ink, fill);
                assert!(
                    ratio >= 3.0,
                    "{symbol:?} on {affiliation:?} fill is {ratio:.2}:1, under 3:1"
                );
            }
        }
        // And the fill itself must survive the map underneath it, which is the
        // other half: a symbol that reads against its own glyph but vanishes
        // into the sea has not been fixed. Both terrains, because the map is
        // pale in both and the numbers are close enough to be worth pinning.
        let sea = egui::Color32::from_rgb(0xa8, 0xd0, 0xf0);
        let land = egui::Color32::from_rgb(0xe8, 0xe0, 0xd0);
        for affiliation in tfg::store::Affiliation::ALL {
            let fill = affiliation_fill(affiliation, false);
            for (what, terrain) in [("sea", sea), ("land", land)] {
                let ratio = contrast_ratio(fill, terrain);
                assert!(
                    ratio >= 3.0,
                    "{affiliation:?} fill is {ratio:.2}:1 against {what}, under 3:1"
                );
            }
        }
        // Stale greys the fill AND the glyph, and they must still separate —
        // a stale hull is exactly the one an operator needs to pick out.
        let stale_fill = affiliation_fill(tfg::store::Affiliation::Friendly, true);
        for symbol in tfg::store::MapSymbol::ALL {
            let ink = map_symbol_fill(map_symbol_shape(symbol), true);
            assert!(
                contrast_ratio(ink, stale_fill) >= 3.0,
                "stale {symbol:?} is under 3:1 against the stale fill"
            );
        }
    }

    /// Every affiliation gets its own frame AND its own hue: the shape
    /// is primary (it survives a greyscale screenshot), the colour is
    /// the redundant second cue. Two states sharing either would make
    /// one of the channels lie.
    #[test]
    fn every_affiliation_has_its_own_frame_and_ink() {
        let frames: std::collections::HashSet<_> = tfg::store::Affiliation::ALL
            .into_iter()
            .map(|a| frame_for(a))
            .collect();
        assert_eq!(frames.len(), tfg::store::Affiliation::ALL.len());
        let inks: std::collections::HashSet<_> = tfg::store::Affiliation::ALL
            .into_iter()
            .map(|a| affiliation_ink(a).to_array())
            .collect();
        assert_eq!(inks.len(), tfg::store::Affiliation::ALL.len());
        // Hostile is red, friendly is blue, unknown is yellow: the three
        // readings an operator already has in their head.
        let ink = |a| affiliation_ink(a);
        assert!(ink(tfg::store::Affiliation::Hostile).r() > ink(tfg::store::Affiliation::Hostile).b());
        assert!(ink(tfg::store::Affiliation::Friendly).b() > ink(tfg::store::Affiliation::Friendly).r());
    }

    /// An icon fits inside its frame, and the frame does not change size
    /// when allegiance changes — the two properties [hit-test parity]
    /// relies on. The geometry itself is tested in map_render.rs; this
    /// pins that the painter passes the geometry those tests produced.
    #[test]
    fn the_painter_scales_glyphs_to_fill_their_frame() {
        for affiliation in tfg::store::Affiliation::ALL {
            let frame = frame_for(affiliation);
            let radius = frame_icon_radius(frame, SYMBOL_BOX_PX);
            let scale = radius / GLYPH_AUTHORING_RADIUS_PX;
            assert!(scale > 0.0 && scale < 2.0, "{affiliation:?} scale {scale}");
            // A glyph never grows past the disc it sits on.
            assert!(5.5 * scale <= symbol_footprint_radius_px() + 1e-9);
        }
        // The widest glyph FILLS its frame, and that is a property of the
        // authored literals rather than of the size or the affiliation:
        //
        //     reach == literal * (icon_radius / GLYPH_AUTHORING_RADIUS_PX)
        //
        // so `reach / icon_radius` is `literal / 8` for every frame at every
        // box, and asserting it against the measured radius measures a
        // constant. That is what the 0.70 floor did, and it failed the tight
        // frames at 0.6875 while telling you it had measured something.
        //
        // So the honest statements are the two that CAN move: the literal has
        // to be most of the authoring radius, or the vocabulary is drawn as a
        // speck in a border; and the result at the box the map actually uses
        // has to be above what the renderer can show as a shape at all.
        assert!(
            5.5 >= 0.65 * GLYPH_AUTHORING_RADIUS_PX,
            "the widest glyph fills only {} of its frame",
            5.5 / GLYPH_AUTHORING_RADIUS_PX
        );
        assert!(
            3.0 >= 0.35 * GLYPH_AUTHORING_RADIUS_PX,
            "the smallest glyph fills only {} of its frame",
            3.0 / GLYPH_AUTHORING_RADIUS_PX
        );
    }

    /// The glyphs must GROW when the symbol does.
    ///
    /// Separate from the test above because that one passes on a scale which is
    /// constant in the box — and a constant scale is exactly the bug: the icon
    /// radius and the painted disc are both linear in the box, so dividing one
    /// by the other cancels, and the raise enlarged the frame around glyphs
    /// that never moved. Every assertion in the painter can hold while the map
    /// looks exactly as illegible as before, so the growth itself is pinned:
    /// the reaches were authored against an 8 px radius, so doubling the box
    /// must double them.
    #[test]
    fn glyphs_grow_with_the_symbol_box() {
        let reach_at = |box_px: f64| {
            frame_icon_radius(frame_for(tfg::store::Affiliation::Friendly), box_px)
                / GLYPH_AUTHORING_RADIUS_PX
                * 5.5
        };
        let small = reach_at(SYMBOL_BOX_PX / 2.0);
        let full = reach_at(SYMBOL_BOX_PX);
        assert!(
            full > small * 1.9,
            "doubling the box moved a glyph from {small} px to only {full} px"
        );
        // And the box the map actually draws at is one an operator can read,
        // which the ratio above cannot tell you: a ratio is equally satisfied
        // by a legible symbol and by a 4 px one.
        assert!(
            reach_at(SYMBOL_BOX_PX) >= 6.0,
            "at {SYMBOL_BOX_PX} px the widest glyph reaches only {} px, which is \
             below the ~6 px the renderer can show as a shape",
            reach_at(SYMBOL_BOX_PX)
        );
    }

    /// A marker with nothing but an identity and a Representation. The
    /// picking tests care about geometry and order, not about fixes.
    fn marker_stub() -> ShipMarker {
        ShipMarker {
            id: String::new(),
            x: 0.0,
            y: 0.0,
            stale: false,
            old_data: false,
            source: FixSource::Wire,
            trail: Vec::new(),
            heading_deg: None,
            map_symbol: tfg::store::MapSymbol::UnknownShip,
            latitude: 0.0,
            longitude: 0.0,
            footprint_px: 0.0,
            affiliation: tfg::store::Affiliation::Unknown,
            planned: false,
            label: String::new(),
            lod: UnitLod::Far,
            visual: None,
        }
    }

    /// Pick order is paint order reversed: what is painted last is on
    /// top, so it is picked first. A stable sort keeps markers of equal
    /// Representation in their list order, and Far must genuinely come
    /// first — otherwise the ladder's whole hierarchy is upside down.
    #[test]
    fn pick_order_is_the_reverse_of_paint_order() {
        let mut markers: Vec<ShipMarker> = Vec::new();
        // Deliberately jumbled: Middle, Near, Far, Near, Far.
        for (i, lod) in [
            UnitLod::Middle,
            UnitLod::Near,
            UnitLod::Far,
            UnitLod::Near,
            UnitLod::Far,
        ]
        .into_iter()
        .enumerate()
        {
            markers.push(ShipMarker {
                id: format!("m{i}"),
                lod,
                x: i as f64 * 100.0,
                y: 0.0,
                ..marker_stub()
            });
        }
        let order = paint_order(&markers);
        let ranks: Vec<u8> = order.iter().map(|&i| match markers[i].lod {
            UnitLod::Far => 0,
            UnitLod::Middle => 1,
            UnitLod::Near => 2,
        }).collect();
        assert_eq!(ranks, vec![0, 0, 1, 2, 2], "paint order is Far, Middle, Near");
        // Same Representation keeps its original relative order.
        assert!(order[0] < order[1], "Far markers keep list order");
        assert!(order[3] < order[4], "Near markers keep list order");
        // And picking walks that list backwards, so the marker painted
        // LAST is picked first — here the last Near in list order.
        assert_eq!(*order.last().unwrap(), 3);
        assert_eq!(*order.first().unwrap(), 2);
    }

    /// THE property hit-test parity rests on: the painted frames have
    /// different bounding boxes by design (an inscribed square is smaller
    /// than the rectangle that fills it), so the pick region is the symbol
    /// BOX for every affiliation — a hull stays equally clickable
    /// whichever way its allegiance changes.
    #[test]
    fn every_affiliation_has_the_same_hit_region() {
        let mut reference: Option<Vec<(f64, f64)>> = None;
        for affiliation in tfg::store::Affiliation::ALL {
            let marker = ShipMarker {
                affiliation,
                lod: UnitLod::Far,
                x: 400.0,
                y: 300.0,
                ..marker_stub()
            };
            let hit = marker_hit_points(&marker, 12.0, 1.0);
            // The first affiliation sets the reference; every other one
            // must produce exactly the same points.
            let Some(first) = reference.as_ref() else {
                reference = Some(hit);
                continue;
            };
            assert_eq!(hit.len(), first.len(), "{affiliation:?} vertex count");
            assert!(
                hit.iter().zip(first).all(|(a, b)| {
                    (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
                }),
                "{affiliation:?} must be picked by the same box"
            );
            // And it is aimable: at least the symbol box across.
            let width = hit.iter().map(|p| p.0).fold(f64::MIN, f64::max)
                - hit.iter().map(|p| p.0).fold(f64::MAX, f64::min);
            assert!(width >= MIN_HIT_PX - 1e-6, "{affiliation:?} width {width}");
        }
    }

    /// A Zone is picked by its outline and its name, never by its
    /// interior: the middle of a translucent ground overlay is still map.
    #[test]
    fn a_zone_is_picked_by_its_outline_not_its_inside() {
        // A 200 x 100 rectangle of screen px.
        let pts = vec![(0.0, 0.0), (200.0, 0.0), (200.0, 100.0), (0.0, 100.0)];
        let label_c = (100.0, 50.0);
        // Deep inside, far from every edge: NOT a hit.
        assert!(!zone_outline_hit(100.0, 50.0, &pts, label_c, 0.0));
        // On the outline: a hit, within the band.
        assert!(zone_outline_hit(0.0, 50.0, &pts, label_c, 0.0));
        assert!(zone_outline_hit(100.0, 0.0, &pts, label_c, 0.0));
        // Just outside the band: not a hit.
        assert!(!zone_outline_hit(-ZONE_HIT_BAND_PX - 1.0, 50.0, &pts, label_c, 0.0));
        // The name on the Zone is a hit wherever it sits.
        assert!(zone_outline_hit(100.0, 50.0, &pts, label_c, 40.0));
        assert!(!zone_outline_hit(100.0, 80.0, &pts, label_c, 20.0));
    }

    /// A group draws its members' kind only when they AGREE, by strict
    /// majority. One destroyer in twelve aircraft must not speak for the
    /// group, and an exact tie must never be broken by iteration order.
    #[test]
    fn a_group_draws_a_kind_only_on_a_strict_majority() {
        use tfg::store::MapSymbol::{Corvette, Destroyer, Plane, Submarine};
        assert_eq!(
            plurality_icon(&[Corvette, Corvette, Corvette, Corvette, Corvette, Corvette, Destroyer]),
            Some(Corvette),
            "six corvettes beat one destroyer"
        );
        // Mixed, and an exact tie: no icon, so the frame stays empty
        // rather than naming a wrong kind.
        assert_eq!(plurality_icon(&[Corvette, Corvette, Plane, Plane]), None);
        // Exactly half is still not a majority.
        assert_eq!(plurality_icon(&[Corvette, Plane]), None);
        // One member alone IS a majority of one — the rule is about
        // agreement, not about being a crowd.
        assert_eq!(plurality_icon(&[Submarine]), Some(Submarine));
        assert_eq!(plurality_icon(&[]), None);
    }

    /// The manifest's discriminator is the asset kind. Pixel size is
    /// carried honestly but does not silently invent a second kind in
    /// this model ticket; thumbnail validation belongs to the archive
    /// reader's own contract.
    #[test]
    fn manifest_asset_kind_is_preserved_without_guessing() {
        let normal = UnitVisual::from_entry(&entry(1, Some(512), Some(256)), "v1", symbol(1));
        assert_eq!(normal.asset_kind, AssetKind::UnitImage);
        assert_eq!(normal.width_px, Some(512));
        assert_eq!(normal.height_px, Some(256));
        assert!(!normal.is_map_renderable(), "dimensions alone are not a texture");

        let mut future = entry(2, Some(1), Some(1));
        future.asset_kind = "account_portrait".to_string();
        let unsupported = UnitVisual::from_entry(&future, "v1", symbol(2));
        assert_eq!(unsupported.asset_kind, AssetKind::Unsupported);
        assert_eq!(unsupported.width_px, Some(1), "size is still reported as sent");
    }

    /// Physical measurements can arrive in the manifest even when the
    /// local spec mirror has not been populated. The manifest is a
    /// complete visual input; hydration must supplement it, never erase
    /// it with a missing local value.
    #[test]
    fn manifest_measurements_survive_local_hydration() {
        let mut published = entry(1, Some(512), Some(256));
        published.loa_m = Some(120.5);
        published.beam_m = Some(16.2);
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![published]), symbol);
        c.set_unit_facts(1, tfg::store::MapSymbol::Corvette, None, None, None);

        let visual = c.get(1).expect("visual");
        assert_eq!(visual.loa_m, Some(120.5));
        assert_eq!(visual.beam_m, Some(16.2));
    }

    /// A local mirror fills a manifest gap, but does not overwrite a
    /// current value that Minos already published in the manifest.
    #[test]
    fn local_measurements_fill_only_manifest_gaps() {
        let mut published = entry(1, Some(512), Some(256));
        published.loa_m = Some(120.5);
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![published]), symbol);
        c.set_unit_facts(1, tfg::store::MapSymbol::Corvette, Some(130.0), Some(18.0), None);

        let visual = c.get(1).expect("visual");
        assert_eq!(visual.loa_m, Some(120.5), "manifest is authoritative");
        assert_eq!(visual.beam_m, Some(18.0), "local mirror fills the missing beam");
    }

    /// A listed unit image needs a source; a future asset kind and a
    /// versioned absence do not. A null presign waits before retrying
    /// rather than hammering the API every frame, and an expired AWS
    /// presign becomes eligible again.
    #[test]
    fn source_resolution_distinguishes_absent_failed_and_expired() {
        let mut c = VisualCache::default();
        let mut future = entry(2, Some(32), Some(32));
        future.asset_kind = "account_portrait".to_string();
        let queued = c.install_manifest(
            &manifest("v1", vec![entry(1, Some(512), Some(256)), future]),
            symbol,
        );
        assert_eq!(queued, vec![1], "future kinds never enter the source queue");
        c.mark_absent(3, tfg::store::MapSymbol::Submarine);
        assert!(c.needs_url(1));
        assert!(!c.needs_url(2), "an unsupported future kind is not a hull image");
        assert!(!c.needs_url(3), "absence has no file to address");

        c.set_url(1, None);
        assert!(!c.needs_url(1), "a null presign waits for the retry delay");
        c.set_url(1, Some("https://objects/one.png?X-Amz-Expires=60".into()));
        assert!(!c.needs_url(1), "a live presign needs no retry");
        c.set_url(1, Some("https://objects/one.png?X-Amz-Expires=0".into()));
        assert!(c.needs_url(1), "an expired presign is read again");
    }

    /// A unit with no image, a unit not yet fetched, and a unit whose
    /// bytes are still arriving are three different states. The old
    /// Option<String> map could not tell them apart.
    #[test]
    fn settled_absence_differs_from_unfetched() {
        let mut c = VisualCache::default();
        assert!(!c.is_settled(7), "manifest not read yet");
        c.install_manifest(&manifest("v1", vec![]), symbol);
        assert!(!c.is_settled(7), "manifest read, unit not resolved");
        c.mark_absent(7, tfg::store::MapSymbol::Auxiliary);
        assert!(c.is_settled(7), "asked, and the answer was no image");
        let absent = c.get(7).expect("absence is a real visual");
        assert_eq!(absent.asset_kind, AssetKind::Unavailable);
        assert_eq!(absent.map_symbol, tfg::store::MapSymbol::Auxiliary);
        assert_eq!(absent.asset_version, "v1", "absence is versioned too");
        assert!(!c.is_settled(8), "a different unit is still open");
    }

    /// A new URL is the same asset. Identity is the unit and the
    /// version, never the address, so an expiring presigned URL is
    /// replaced without disturbing anything else.
    #[test]
    fn url_replacement_preserves_the_asset() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_unit_facts(
            1,
            tfg::store::MapSymbol::Corvette,
            Some(120.0),
            Some(16.0),
            Some(4.0),
        );
        c.set_url(1, Some("https://objects/one?sig=a".into()));
        let before = c.get(1).expect("visual");
        assert_eq!(before.asset_version, "v1");
        assert_eq!(before.loa_m, Some(120.0));

        c.set_url(1, Some("https://objects/one?sig=b".into()));
        let after = c.get(1).expect("visual");
        assert_eq!(after.image_url.as_deref(), Some("https://objects/one?sig=b"));
        assert_eq!(after.asset_version, "v1", "version is not the URL's job");
        assert_eq!(after.loa_m, Some(120.0), "measurements survive the swap");
        assert_eq!(after.map_symbol, tfg::store::MapSymbol::Corvette);
    }

    /// A manifest version change means the assets themselves moved.
    /// Pictures are re-resolved; the unit ids and their identity are
    /// untouched, because the units did not change.
    #[test]
    fn version_change_invalidates_pictures_not_units() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_url(1, Some("https://objects/one?sig=a".into()));
        assert_eq!(c.get(1).expect("v1 visual").asset_version, "v1");

        // Same units, new version: the old URL is not carried over.
        let needing = c.install_manifest(&manifest("v2", vec![entry(1, Some(640), Some(320))]), symbol);
        assert_eq!(needing, vec![1], "the URL must be re-read for v2");
        let v2 = c.get(1).expect("v2 visual");
        assert_eq!(v2.asset_version, "v2");
        assert_eq!(v2.image_url, None, "a v1 URL does not describe a v2 asset");
        assert_eq!(v2.width_px, Some(640), "new dimensions applied");
        assert!(!c.units.contains_key(&(1, "v1".to_string())));
        assert!(c.units.contains_key(&(1, "v2".to_string())));
    }

    /// A unit that had a picture in one version and is omitted from
    /// the next is not silently absent: it becomes unresolved under
    /// v2, so selection can record fresh versioned absence rather than
    /// inheriting the v1 answer.
    #[test]
    fn omission_in_a_new_version_clears_old_absence() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![]), symbol);
        c.mark_absent(7, tfg::store::MapSymbol::Destroyer);
        assert!(c.is_settled(7));

        c.install_manifest(&manifest("v2", vec![]), symbol);
        assert!(!c.is_settled(7), "v1 absence cannot answer for v2");
        assert!(!c.units.contains_key(&(7, "v1".to_string())));
    }

    /// The backend's ETag may be absent. That still records that a
    /// manifest was loaded, but an unknown version never claims cache
    /// identity or preserves a URL across a repeated read.
    #[test]
    fn empty_content_stamp_is_still_a_loaded_version() {
        let mut c = VisualCache::default();
        c.install_manifest(
            &manifest("", vec![entry(1, Some(512), Some(256))]),
            symbol,
        );
        assert!(c.manifest_loaded);
        assert!(c.is_settled(1));
        assert_eq!(c.get(1).expect("visual").asset_version, "");
        c.set_url(1, Some("https://objects/one".into()));
        let needing = c.install_manifest(
            &manifest("", vec![entry(1, Some(512), Some(256))]),
            symbol,
        );
        assert_eq!(needing, vec![1], "an unknown version cannot claim cache identity");
        assert_eq!(c.get(1).expect("visual").image_url, None);
    }

    /// Installing the same version twice is idempotent and asks for
    /// nothing, so a re-read does not re-fetch every URL.
    #[test]
    fn reinstalling_the_same_version_asks_for_nothing() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_url(1, Some("https://objects/one?sig=a".into()));
        let needing = c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        assert!(needing.is_empty(), "same version, no new URLs wanted");
        assert_eq!(
            c.get(1).expect("visual").image_url.as_deref(),
            Some("https://objects/one?sig=a"),
            "and the live URL is kept"
        );
    }

    /// Measurements live alongside pictures, not inside them, and
    /// survive an asset change because the specification did not
    /// change with the photograph.
    #[test]
    fn dimensions_outlive_the_picture() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_unit_facts(
            1,
            tfg::store::MapSymbol::Corvette,
            Some(120.5),
            Some(16.2),
            Some(4.1),
        );
        c.install_manifest(&manifest("v2", vec![entry(1, Some(512), Some(256))]), symbol);
        let v = c.get(1).expect("visual");
        assert_eq!(v.loa_m, Some(120.5));
        assert_eq!(v.beam_m, Some(16.2));
        assert_eq!(v.draft_m, Some(4.1));
    }

    /// A picture added after the first manifest read replaces the
    /// versioned absence and enters the source queue.
    #[test]
    fn later_manifest_promotes_unavailable_to_a_picture() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![]), symbol);
        c.mark_absent(351, tfg::store::MapSymbol::Destroyer);
        assert!(!c.needs_url(351));

        let needing = c.install_manifest(
            &manifest("v2", vec![entry(351, Some(512), Some(256))]),
            symbol,
        );
        assert_eq!(needing, vec![351]);
        assert_eq!(c.get(351).expect("new visual").asset_kind, AssetKind::UnitImage);
        assert!(c.needs_url(351));
    }

    /// Unpublished measurements stay unpublished, so the renderer can
    /// tell "unknown" from "zero metres".
    #[test]
    fn unpublished_measurements_read_as_unknown() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_unit_facts(1, tfg::store::MapSymbol::Corvette, None, None, None);
        let v = c.get(1).expect("visual");
        assert_eq!(v.loa_m, None);
        assert_eq!(v.beam_m, None);
        // …and a hull with no image is still drawable, as a symbol.
        let absent = UnitVisual::unavailable(99, "v1", tfg::store::MapSymbol::Auxiliary);
        assert_eq!(absent.asset_kind, AssetKind::Unavailable);
        assert_eq!(absent.map_symbol, tfg::store::MapSymbol::Auxiliary);
    }

    /// The session boundary: nothing survives it. This is the
    /// sign-out / user-change guarantee.
    #[test]
    fn clear_leaves_nothing_behind() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_url(1, Some("https://objects/one".into()));
        c.set_unit_facts(
            1,
            tfg::store::MapSymbol::Corvette,
            Some(120.0),
            Some(16.0),
            None,
        );
        c.mark_absent(2, tfg::store::MapSymbol::Destroyer);
        c.clear();
        assert!(c.units.is_empty(), "no unit keeps a URL or measurement");
        assert!(c.asset_version.is_empty(), "no version survives");
        assert!(!c.manifest_loaded, "the next session must read its manifest");
    }

    /// A map thumbnail needs BOTH a supported unit-image asset and a
    /// decoded texture. A unit mid-fetch is not renderable, so a
    /// renderer reaching it draws a symbol rather than a hole.
    #[test]
    fn map_renderability_needs_a_texture() {
        let v = UnitVisual::from_entry(&entry(1, Some(512), Some(256)), "v1", symbol(1));
        assert!(!v.is_map_renderable(), "metadata alone are not a texture");
        let mut future = entry(2, Some(512), Some(256));
        future.asset_kind = "account_portrait".to_string();
        let unsupported = UnitVisual::from_entry(&future, "v1", symbol(2));
        assert!(!unsupported.is_map_renderable(), "a future kind is not a hull image");
    }
}

/// A single-line field with a fixed label above it.
///
/// Two helpers rather than one `field` because the composer's two shapes
/// differ: the title is one short line, the step content is a box. Forcing
/// both through one helper would have meant a parameter that is ignored half
/// the time.
fn ui_input(ui: &mut egui::Ui, value: &mut String, label: &str) {
    ui.label(egui::RichText::new(label).weak().small());
    ui.add(
        egui::TextEdit::singleline(value).desired_width(f32::INFINITY),
    );
}

fn ui_text(ui: &mut egui::Ui, value: &mut String, label: &str) {
    ui.label(egui::RichText::new(label).weak().small());
    ui.add(
        egui::TextEdit::multiline(value)
            .desired_width(f32::INFINITY)
            .desired_rows(3),
    );
}
