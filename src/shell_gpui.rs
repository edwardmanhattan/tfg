//! A GPUI island, painted from `tfg::view`.
//!
//! This is the port of `chrome::paint_island`, and it exists to answer one
//! question with a picture: can the console's own design language be drawn by
//! GPUI at all? The nested bevel rings, the offset cast shadow, the title
//! band with rounded top corners, the scanline texture and the lit rim are
//! all reproduced from the same kernel the egui shell reads.
//!
//! What it proves about the architecture:
//!
//! - The palette survives the migration. Every colour is a `view::Rgba`, and
//! no `egui::Color32` appears anywhere in this file.
//! - The bevel is four nested fills, exactly as it is four nested
//! `rect_filled` calls today. That order is why the rings stay concentric,
//! and that property is a fact about the paint, not about egui.
//! - Font weight is a value, not a family name.
//!
//! What it does NOT yet do, which is the real work of the port: it does not
//! own a drag handle, does not clip to the zone band, does not measure its
//! own content for the scroll chain, and does not resolve input ownership
//! against its siblings. Those are `chrome::island_owned`'s job.
//!
//! MOVED here for U8 step 3 (ADR-0020): the GPUI shell, now the console
//! itself since step 5 removed the egui one.
//! `proto/gpui-island` still drives this same file through `#[path]`, so the
//! parity proof and the shipped binary cannot drift — the pattern `view`,
//! `map_gate` and `halo` already use.
//!
//! What it does NOT yet do, which is the real work of the port (U8 step 4):
//! it does not own the console's widgets. The `*_layer()` functions below are
//! the U5 parity slices, kept behind their env flags, and the console's own
//! `ShipApp` port starts from this shell and grows into it.

use gpui::{
    AnyElement, App, AppContext, Application, AsyncApp, Bounds, Context, CursorStyle, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ObjectFit,
    ParentElement, PathBuilder, Render, ScrollHandle, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement,
    Styled, TitlebarOptions, Window, WindowBounds, WindowOptions, canvas, div, fill, img,
    point, prelude::*, px, size,
};

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};

use crate::halo::{self as halo};
use crate::map_gate::{self as gate, FrameSlot, RawReplay};
use crate::book::ComposerDraft;
use crate::symbology;
use crate::symbology::frame::{frame_for, frame_polygon, frame_strokes};
use crate::view::{self as view, *};
use std::sync::Arc;

/// One `view::Rgba` as a GPUI colour. The kernel speaks 0..255 sRGB and a
/// shell converts at its own boundary, which is the whole point of the seam.
///
/// `gpui::rgba` is raw 0..1 with no gamma applied, so this is the exact same
/// convention and the values pass through untouched. Going by way of `Hsla`
/// instead applies a conversion the design tokens never asked for and shifts
/// every hue.
fn rgba(c: Rgba) -> gpui::Rgba {
    let [r, g, b, _] = c.as_f32();
    gpui::Rgba {
        r,
        g,
        b,
        a: c.alpha_f32(),
    }
}

/// One island's own state: whether the operator has it open, and the
/// measurements its body publishes.
///
/// There is deliberately no position here. The column is a STACK — every
/// origin comes from `view::zone_slot_origin` over the state's slots, which
/// is the console's arrangement (`zone_island_origins_with`) and the reason
/// an island cannot be dragged out of the column. The prototype carried a
/// stored `pos` and a drag handler, which is what made the chrome read as
/// floating windows rather than as a console's column.
/// The three surfaces that AUTHOR rather than read, and therefore take the
/// frame: the console's modal zone. A column island is not one of them —
/// those are surfaces the operator reads beside the map, and a dim over the
/// map is the one thing that makes them unreadable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhichPanel {
    /// No modal. One of the six, or nothing — and `open_only` takes the
    /// whole thing, so closing one cannot leave another half-open behind it.
    None,
    /// The hull register, opened from the Fleet island's door. The console's
    /// `WhichModal::Fleet`.
    Fleet,
    /// The account directory, opened from the Players island's door. The
    /// console's `WhichModal::Player`.
    Player,
    Settings,
    Composer,
    Book,
    Comms,
    /// One hull's passengers, opened from the Fleet island's piece rows.
    /// The console's floating "Embarked" island, which this shell has no
    /// floating version of — so the modal zone hosts it. Visibility is the
    /// host id (`embark_host`), not a bool.
    Embarked,
}

struct IslandState {
    key: view::IslandKey,
    close_hot: bool,
    /// Last logged measurement (change-gated `eprintln`, zone-debug style).
    last_fit: Option<BodyFit>,
    /// The body's scroll measurement. `track_scroll` feeds it during
    /// layout — offset, viewport and content extent with no custom
    /// element and no frame of lag. Read at the shell boundary into
    /// `view::BodyFit` (sign flip included); the kernel never sees it.
    scroll: ScrollHandle,
}

impl IslandState {
    fn new(key: view::IslandKey) -> Self {
        IslandState {
            key,
            close_hot: false,
            last_fit: None,
            scroll: ScrollHandle::new(),
        }
    }

    /// The stateful surfaces key their ids by the island, so one shared id
    /// cannot weld every body to one scroll offset (`chrome.rs`'s `id_salt`,
    /// the same rule).
    ///
    /// The key is the island's discriminant, not its position: the column's
    /// membership changes with the exercise's state, and an id derived from
    /// an index would hand one island's scroll offset to whichever island
    /// happened to land in that slot next.
    fn body_id(&self) -> (&'static str, usize) {
        ("island-body", self.key as usize)
    }
}

struct IslandDemo {
    /// The column, one entry per island the console knows, in a fixed
    /// order. Membership is `view::zone_islands`' job and changes with the
    /// exercise's state; the `open` flag is the OPERATOR's and must survive
    /// that change — which is why the state lives here keyed by island
    /// rather than in a list rebuilt from the state each frame.
    islands: Vec<IslandState>,
    /// The Settings panel's client state (the console's `settings_modal`,
    /// the one panel that needs no session). `modal_open` is the panel's own
    /// visibility; these are what it edits.
    modal_open: bool,
    /// View mode ("Watch only") — the Settings panel's first control.
    watching: bool,
    show_side_zone: bool,
    side_dock_left: bool,
    reduced_motion: bool,
    /// The Crew island: the session roster. A column island now, not a
    /// modal — the console lists it in the column in every state past
    /// planning, and a room you have to open a dialog to see is a room you
    /// do not watch. The flag keeps its name and its meaning: whether the
    /// operator has this island open.
    people_open: bool,
    crew_open: bool,
    /// The Command island: the seat, the commanded hulls, their live fixes,
    /// the readiness gate and the helm. The console's Orders island in
    /// execution and its Readiness island in preparation — one surface, one
    /// place, which is the whole reason it is a column island and not both.
    command_open: bool,
    /// The Comms panel: the session's inbox and the telegram composer.
    /// The console keeps it a FLOATING island opened from the band's comms
    /// door, not a column island and not a centred modal — an inbox is
    /// something you read beside the map, and a dim over the map is the one
    /// thing that makes it unreadable. So the door moved to the band and the
    /// pane stays undimmed.
    comms_open: bool,
    /// The Step island: the exercise's current step and its readiness gate.
    /// The console's `z.steps` for the organizer audience and `z.mystep` for
    /// personnel — one surface here, because one shell has one name per
    /// thing.
    step_open: bool,
    /// The Log island's feed: what the operator was told happened, newest
    /// last, capped. The console's `event_feed`, a `VecDeque<String>` capped
    /// at 30 by `feed()` (`main.rs:17029`), which also echoes to stderr.
    ///
    /// It is pushed by the WRITES, not read from anywhere: there is no events
    /// GET, and the console's own feed is exactly the same — every line in it
    /// is a line this client produced. The console's other half of the same
    /// island, journal files with a replay slider over map ghosts, is local
    /// scratch and is not carried.
    log_feed: VecDeque<String>,
    /// Which commanded hull the helm orders.
    ///
    /// The console's dashboard has a ship picker for exactly this, and orders
    /// the ACTIVE hull. This shell listed every commanded hull but aimed the
    /// order at `mine.first()` and never said so — so an operator holding
    /// two hulls could not tell which one a posted order belonged to, and
    /// could not choose. An `Option` rather than an index, because an index
    /// silently means a different hull the moment the roster changes.
    helm_target: Option<String>,
    /// Which Assessment tab is showing. The console's `assessment_tab`, and
    /// its five values: Summary, Timeline, Judgements, Reviews, Transcript.
    /// Transcript is not carried, so this holds four.
    assessment_tab: usize,
    /// The Assessment island's Timeline tab: the filter drafts and the latch
    /// on the read. The console's `timeline_*` fields, cut to the filters
    /// this tab draws. The events AND their cursor block ride the session —
    /// the same read `fetch_session` seeds for a closed game — so a filter
    /// reload REPLACES them and `load more` EXTENDS them. A source change
    /// reloads from the first page, because a cursor never crosses a filter
    /// change.
    timeline_source: Option<String>,
    timeline_personnel: String,
    timeline_unit: String,
    timeline_from: String,
    timeline_to: String,
    timeline_busy: bool,
    timeline_status: String,
    /// The Assessment island's Judgements and Reviews reloads: one latch and
    /// one status line each, because the two reads are separate tickets in
    /// the console (`setup_busy("judgements")` / `("reviews")`). The marks
    /// and documents they replace ride the session — the same closure fetch
    /// that seeds them — so a reload REPLACES. `review_mine_only` is the
    /// console's own name: checked, the reload narrows server-side to this
    /// authorship.
    judgements_busy: bool,
    judgements_status: String,
    reviews_busy: bool,
    reviews_status: String,
    review_mine_only: bool,
    /// The Judgements tab's compose: the picked subject and the score
    /// draft. The console's `judge_subject` / `judge_score`, minus the
    /// numeric fallback — the shell's subjects ride the held session, so
    /// an empty room means nobody to judge and the send refuses with the
    /// subject sentence rather than asking for an id.
    judge_subject: Option<i64>,
    judge_score: String,
    judge_score_handle: Option<gpui::FocusHandle>,
    judge_score_focused: bool,
    /// The latch on the record write. Separate from the reload's latch
    /// because the console tickets them separately (`"judgement"` vs
    /// `"judgements"`), and the answer auto-reloads through the read
    /// latch exactly the way `JudgementSent` re-calls `load_judgements`.
    judge_write_busy: bool,
    /// The Reviews tab's compose: the picked subject, the body draft, and
    /// the row under revision. The console's `review_subject` /
    /// `review_body` / `review_editing`, with the same rules — subject
    /// fixed while revising, body cleared on file, both cleared on save —
    /// and the same latch separation from the reload.
    review_subject: Option<i64>,
    review_body: String,
    review_body_handle: Option<gpui::FocusHandle>,
    review_body_focused: bool,
    review_editing: Option<i64>,
    review_write_busy: bool,
    /// The four filter fields' keyboard state, hand-rolled like the
    /// composer's: GPUI ships no text field, so each is a focus handle and a
    /// keystroke handler. Tab walks personnel → unit → from → to; Enter
    /// applies the filters.
    timeline_focus: usize,
    timeline_handles: [Option<gpui::FocusHandle>; 4],
    /// The Inspector dock, under the map. The console shows it whenever there
    /// is a selection; the shell keeps it a toggle so the map is not always
    /// docked down, and seeds it for a capture.
    inspect_open: bool,
    /// Last known cursor position in window pixels. Hover moves keep it
    /// fresh; ownership resolves from it every frame, exactly once, so at
    /// most one rim lights (ADR-0016).
    pointer: Option<(f32, f32)>,
    /// Column scroll, positive-down. A VIEW transform: positions stay
    /// unscrolled and every read subtracts it, so a stale scroll never
    /// strands the column. Re-clamped every frame against the LIVE stack,
    /// because the overflow changes with the exercise's state.
    zone_scroll: f32,
    /// Live zone width. The grip writes it; origins, band and islands read
    /// it next frame, so the resize is live mid-drag.
    zone_w: f32,
    /// Grip drag: cursor-x grab plus width at grab. `None` is not resizing.
    /// The column's only handle: an island rides the stack and is not
    /// dragged, so this is the one gesture that moves the column's geometry.
    grip: Option<(f32, f32)>,
    grip_hot: bool,
    /// The map surface (U6): the real map thread's frames on the RAW path —
    /// `ImageSource::Render`, no encode and no decode hop, so there is
    /// nothing to hold and nothing to wait for. Painted full-bleed under
    /// everything, because the band and the rails paint no background of
    /// their own — the map is what shows through.
    gate: FrameSlot,
    /// The roster sheet's rows: the hull register over the dev API, or the
    /// prototype's fixture when no credentials are set (U8 step 4's first
    /// panel — real values, not literals).
    roster: Vec<RosterRow>,
    /// How many hulls the register holds, for the sheet's counter.
    roster_total: usize,
    /// The games this account can see, for the session picker: read at
    /// sign-in beside the roster, in the server's own order. Holding one
    /// replaces the session below but never this list — going back to
    /// pick again must not cost a re-read. Empty until authenticated,
    /// like the roster.
    sessions: Vec<SessionRow>,
    /// The hold latch and its verdict line, for the picker's verb.
    hold_busy: bool,
    hold_status: String,
    /// The session the top zone reads: the held game and its room's
    /// headcount. `SessionView::default()` when no session is held.
    session: SessionView,
    /// The helm draft, and the last answer about it. `None` heading/speed
    /// means "follow the fix", seeded once the first fix arrives — so the
    /// panel opens on the hull's own course rather than on zero.
    helm_heading: Option<f32>,
    helm_speed: Option<f32>,
    helm_status: Option<HelmStatus>,
    /// The order sink. The shell WRITES through this and reads no API.
    order_tx: OrderSink,
    /// The camera's zoom, as the map controls last set it. The shell keeps the
    /// value it ASKED for — the console does the same, and the map thread
    /// clamps identically — so the readout is the operator's intent, not a
    /// guess at the renderer's state.
    map_zoom: f64,
    /// The live pan drag's last position, in window points. `Some` while the
    /// map surface is held — the drag's own latch, so a move without a press
    /// is not a pan.
    pan: Option<(f32, f32)>,
    /// The sign-in card's own state: what was typed, which field has the
    /// keyboard, and what the last attempt said.
    login_identifier: String,
    login_password: String,
    login_focus: Cred,
    login_status: String,
    login_busy: bool,
    /// Focus handles, minted on first use: `Context::focus_handle` needs a
    /// context and `new` has none, so they are made in `render` and kept.
    login_handles: [Option<gpui::FocusHandle>; 2],
    /// The New-session composer: the console's `session_creator_body`, ported.
    /// Its own modal, for the same reason People has one. `draft` is what was
    /// typed; `compose_focus`/`compose_handles` are the hand-rolled fields'
    /// keyboard state, because GPUI ships no text field (the sign-in card's
    /// reason, and its pattern). Six fields: name, description, purpose,
    /// target, area, map tag — the mode rides radios, not a field.
    composer_open: bool,
    draft: SessionDraft,
    compose_focus: usize,
    compose_handles: [Option<gpui::FocusHandle>; 11],
    compose_busy: bool,
    compose_status: String,
    /// The Book panel: the held game's scenarios and their steps (read), plus
    /// the new-scenario title row (the one write). Its own modal, for the
    /// same reason the composer has one. The title rides field index 6 of the
    /// composer's hand-rolled plumbing — one field implementation, not two —
    /// and never tabs into the draft: a single row tabs to itself.
    book_open: bool,
    book_title: String,
    book_busy: bool,
    book_status: String,
    /// The open scenario's id: the one whose steps show with the new-step
    /// row. `None` lands on the book's first, the console's rule for an
    /// author reopening to keep working. The step draft is the console's own
    /// `ComposerDraft` — content plus HHMM window with its `problem()` rule —
    /// held directly, because the validation is the console's and duplicating
    /// it would be a second answer waiting to disagree. `step_busy` latches
    /// the step writes apart from the title-row write on the shared answer
    /// feed.
    book_open_scenario: Option<i64>,
    step_draft: ComposerDraft,
    step_busy: bool,
    /// Readiness: the caller's declare/withdraw on the held game. Its own
    /// busy latch on the shared answer feed, and its own status line — the
    /// helm's status is about an order, this is about the seat.
    readiness_busy: bool,
    readiness_status: String,
    /// The step declaration: this seat's declare/withdraw on the asking
    /// step. Its own busy latch on the shared answer feed, and its own
    /// status line — the game readiness above is about the seat, this is
    /// about the step.
    step_declare_busy: bool,
    step_declare_status: String,
    /// The telegram composer: the console's `compose_ui`, cut to the proven
    /// path — a broadcast telegram, TERBUKA, BIASA. The console's kind,
    /// marking, audience and grade pickers are a later slice, and the panel
    /// states the cut rather than showing controls with no effect. Rides
    /// field index 10 of the composer's hand-rolled plumbing — one field
    /// implementation, not two — and tabs to itself.
    msg_content: String,
    msg_busy: bool,
    msg_status: String,
    /// The operator's force: what is in the exercise, as THEY have said it.
    ///
    /// The console's `self.force`, and the console's own module — `tfg::force`
    /// is toolkit-free, so the draft, its rules and its `plan()` write
    /// generation are ported rather than reinvented. Seeded from the session's
    /// pieces and re-seeded whenever a session read replaces them. WHILE
    /// PLANNING the draft IS the record: every edit here mutates it and posts
    /// nothing, and only the Session island's advance writes it through. In
    /// every later state each edit plans and posts at once, because there is
    /// no stage left to wait for — and there a re-read cannot discard work,
    /// because none is held.
    ///
    /// A hull dropped on the map carries a starting position here — the drop
    /// unprojects its fractions to lat/lon locally (see `frame_unproject`) and
    /// `plan()` emits the `Place` that records it. An embarked hull carries a
    /// `host` instead, and no start: a hull aboard another has no position of
    /// its own.
    force: crate::force::ForceDraft,
    /// The operator's roster: who is seated, as THEY have said it.
    ///
    /// The console's `roster_draft`, and the console's own module — `tfg::roster`
    /// is toolkit-free, so the draft, its rules and its `plan()` write
    /// generation are ported. Seeded from the session's seats and re-seeded on
    /// every session read. While planning it holds staged seats and removals
    /// that nothing has posted; each edit still plans and posts at once AFTER
    /// planning, for the same reason the force's does.
    roster_draft: crate::roster::RosterDraft,
    /// Control's draft: the game-time multiplier being adjusted, and the
    /// latch on the write. `None` means "not touched yet", which is how the
    /// draft picks up Minos's own value on the first frame the island draws
    /// instead of freezing a default nobody chose.
    factor_draft: Option<f64>,
    factor_busy: bool,
    factor_status: String,
    /// The last clock answer's `running` — the ONLY clock read this shell
    /// has, because no clock GET exists and the pause/resume/factor writes
    /// ARE the read (the backend's own contract). `None` until the first
    /// answer; unknown attempts pause, the console's rule. Reset on a new
    /// hold — an answer proved the OLD game's grant, not this one's.
    clock_running: Option<bool>,
    /// The control grant, remembered from a 403 — the console's
    /// `clock_denied`. While set, the pause verb refuses locally with the
    /// grant sentence instead of spending a POST the server has already
    /// refused. Factor posts stay unlatched: they are deliberate drafts,
    /// and a second 403 reports loudly rather than wedging.
    clock_denied: bool,
    pause_busy: bool,
    pause_status: String,
    /// Review's advance latch and its verdict line.
    advance_busy: bool,
    advance_status: String,
    /// The Step island's end-session latch and its verdict line — the
    /// console's `z.clock` island reduced to its one control. The island's
    /// readouts are the sim's tick, which this shell has none of, so only
    /// the verb is carried, organizer-side.
    end_busy: bool,
    end_status: String,
    /// The Fleet picker: the register, opened from the Fleet island's door.
    /// The console's `WhichModal::Fleet`, seeded from the band's door.
    fleet_open: bool,
    /// A drag is in flight (polled per tick — GPUI owns the state). The
    /// fleet picker hides for exactly the drag's lifetime, so the drop can
    /// land anywhere on the map instead of on the modal's own card.
    dragging: bool,
    /// The picker's drill tree: Asset Group > Category > Type > Class >
    /// hull, built once by the binary off the synced mirror. Empty on
    /// fixture runs — the picker falls back to its flat register list.
    pub fleet_tax: FleetTaxonomy,
    /// Hull photographs, minted on first paint as the fetcher lands them.
    /// The glyph draws until the photo arrives — progressive fill, and a
    /// hull with no picture keeps its glyph, which is the honest fallback.
    thumbs: HashMap<i64, Arc<gpui::RenderImage>>,
    /// The drill position, one rung each. A parent change clears everything
    /// below it (the console's `drill_*` rule); `None` group means the first
    /// tab, so an unseeded open still shows a full column set.
    drill_group: Option<i64>,
    drill_category: Option<i64>,
    drill_type: Option<i64>,
    drill_class: Option<i64>,
    /// The Player picker: the directory, opened from the Players island's
    /// door. The console's `WhichModal::Player`.
    player_open: bool,
    /// The Embarked modal's host — the console's `embark_island_host`.
    /// `Some` draws the modal; `None` is closed. Set by the piece rows'
    /// `show…` doors via `open_embarked`, cleared by the modal's close.
    /// A host id rather than a bool, because the modal reads one hull's
    /// passengers and a flag would not say whose.
    embark_host: Option<i64>,
    /// What each host carries, by host id — the console's `embarked` map.
    /// Read lazily per host on first open (the console's `ensure_embarked`
    /// rule: cached answers are kept), never prefetched: most hosts carry
    /// nothing, and a modal that reads the whole fleet to show one hull's
    /// passengers is a fan-out for nothing.
    embarked: HashMap<i64, Vec<crate::backend::EmbarkedUnit>>,
    /// The last write's capacity warnings, by host — the console's
    /// `embark_over`. A warning, never a refusal: the answer reports which
    /// complement was exceeded, and the rows draw it yellow.
    embark_over: HashMap<i64, Vec<crate::backend::OverCapacity>>,
    embark_busy: bool,
    embark_status: String,
    /// The Fleet picker's latch and verdict. The latch is the shared answer
    /// feed's; the line is what the panel says about the last edit, which is
    /// where a refusal from `force::plan` lands.
    /// The Player picker's latch and verdict.
    roster_busy: bool,
    roster_status: String,
    force_busy: bool,
    force_status: String,
    /// A deploy in flight, latched apart from the force writes: the answer
    /// carries the same re-read session, but the verdict shares the picker's
    /// line — one panel, one sentence about its last edit.
    deploy_busy: bool,
    /// The advance out of planning: the latch on the staged flush, and the
    /// line the Session island says about it. The answer rides the shared
    /// create feed, like every other session-replacing write here.
    prepare_busy: bool,
    prepare_status: String,
    /// Click-to-place, the drag's fallback: the armed hull id. Set by the
    /// leaf row's place button; the next map click that is NOT a pan deploys
    /// it there. A drag never arms this (its release lands on the drop, not
    /// the row), and arming never pans (the press anchors instead).
    pending_place: Option<i64>,
    /// The press anchor while armed: a press that moves past 6px becomes a
    /// pan like any other, and only a press that stays put places.
    place_down: Option<(f32, f32)>,
    /// The relate editor: which step it obliges and its working set of
    /// (kind, id) refs. Seeded from the server's read when opened — there is
    /// no staging here, so the working set IS the last saved list until the
    /// author touches it. Its own busy latch on the shared answer feed.
    relate_open: Option<(i64, i64)>,
    relate_working: Vec<(String, i64)>,
    relate_busy: bool,
    /// The roster row the operator has selected, by index into `roster`.
    /// Drives the sheet's selection wash and cyan edge; 0 until a row is
    /// clicked. The console's selection also moves the camera and the
    /// detail pane — that wiring is a later slice, this is the state.
    selected: usize,
    /// Whether the ROSTER SHEET is the subject instead of the console.
    ///
    /// A field, not `std::env::var("SHEET")` read in `render`. The env read
    /// was a process-global that two tests set and never cleared, so every
    /// test that ran after them in the same process rendered the sheet — a
    /// race that stayed invisible until a test asserted on the rendered tree
    /// instead of on the demo's own fields. The binary still seeds it from
    /// the env, once, where every other capture seed lives.
    sheet: bool,
    /// The newest raw frame from the replay, handed to the slot in `render`
    /// (promotion is a `Window` operation, so it cannot happen on the
    /// producer thread).
    incoming: Option<(u64, u32, u32, Vec<u8>, (f64, f64), f64)>,
    /// The camera the map thread last RENDERED, not the one the shell asked
    /// for. Two different numbers: `map_zoom` is what a zoom click set, this
    /// is what is on screen, and the scale bar reads the second one because a
    /// bar that describes a frame that is not up is a bar that lies.
    map_center: (f64, f64),
    map_render_zoom: f64,
    /// The live ships, one per tracked hull: the marker layer draws these.
    /// Empty until the wire reports; `x`/`y` are in the map frame's pixel
    /// space (see `map_dims`).
    live: Vec<LiveMarker>,
    /// The map frame's pixel dimensions, learned from the first frame. The
    /// live markers arrive in that space and the layer scales them to the
    /// window by it — so the shell never names the map's size. Zero until a
    /// frame arrives, which draws no markers.
    map_dims: (f32, f32),
    /// The sprite the compositing probe uses. Built unconditionally so the
    /// probe does not depend on the marker layer's configuration.
    probe_sprite: std::sync::Arc<gpui::RenderImage>,
    /// U5 drag slice: `Some(press point)` while a row is held; `moved` is
    /// the 6px dead-zone latch (a held press is a drag only past 6px).
    drag_start: Option<(f32, f32)>,
    drag_moved: bool,
    /// Which island the next frame should scroll onto the band. `Some` for
    /// one frame: a click handler has no window to ask for the band, so it
    /// records the wish and the frame that draws the column honours it.
    reveal: Option<view::IslandKey>,
}

/// The modal panel's footprint. Fixed by contract, like an island's.
const MODAL_W: f32 = 480.0;
const MODAL_H: f32 = 320.0;
/// The authoring card: taller than any readout panel, because the composer
/// form is six fields plus a mode row and the book grows with its steps
/// (the console's own creator is 560x620). Shrinking either form onto the
/// readout card ships a cramped lie about the layout — the book's first
/// capture proved it, with the title row clipped off the card's bottom.
const FORM_W: f32 = 520.0;
const FORM_H: f32 = 580.0;

/// The fleet picker's card: wider than the authoring forms, because four
/// miller columns (3 × 128 + a flex leaf) do not fit 520 — the egui picker
/// ran 980 wide for the same reason, and this window is 760.
const FLEET_W: f32 = 700.0;
const FLEET_H: f32 = 580.0;

/// The leaf column's content width — what a row's banner spans. The
/// modal (700) minus the picker's padding (12 + 28) and the three fixed
/// columns with their gaps (3×128 + 4×8) leaves 244 for the leaf, and
/// the row card's own padding (2×4) leaves 236. Fixed, not a
/// percentage: a `w_full` banner resolves its children's own percentages
/// against nothing and they paint nothing — neither photo nor glyph
/// (proved on capture).
const LEAF_CONTENT_W: f32 = 236.0;

/// The banner's tallest. A photo's natural banner height is its width
/// over its aspect; the archive's few non-panorama outliers (a tank at
/// 2.67:1, a helicopter at 1.51:1, a drone at 0.49:1) would reach 88 to
/// 481 and blow the row up, so past this they letterbox inside it
/// instead — the whole body still shows, the row stays sane.
const BANNER_MAX_H: f32 = 96.0;

/// The modal silhouette: a convex pentagon, the panel rect with its
/// top-right corner cut back by `view::CUT` (20.0), stroked 2px in
/// `CUT_GREY`. Painted with `window.paint_path` through a `canvas` element
/// because no box primitive can draw it — and neither can egui, which is
/// the point of this unit.
///
/// Fill and edge both paint OUTSIDE any clipped box: the frame below
/// carries no `bg`, no `rounded` and no `overflow_hidden`. The clip belongs
/// on the content column, which is transparent.
fn modal_silhouette() -> impl IntoElement {
    modal_silhouette_at(MODAL_W, MODAL_H)
}

/// The same chamfered card at another size: the composer form is taller than
/// any readout panel (the console's own creator is 560x620), and shrinking
/// the form to fit the readout card would ship a cramped lie about the
/// console's layout. Existing callers keep `modal_silhouette()`, unchanged.
fn modal_silhouette_at(w: f32, h: f32) -> impl IntoElement {
    canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let x0 = bounds.origin.x;
            let y0 = bounds.origin.y;
            let x1 = x0 + px(w);
            let y1 = y0 + px(h);
            let cut = px(CUT);
            let mut fill = PathBuilder::fill();
            fill.move_to(point(x0, y0));
            fill.line_to(point(x1 - cut, y0));
            fill.line_to(point(x1, y0 + cut));
            fill.line_to(point(x1, y1));
            fill.line_to(point(x0, y1));
            fill.close();
            if let Ok(path) = fill.build() {
                window.paint_path(path, rgba(CONSOLE_NIGHT));
            }
            let mut edge = PathBuilder::stroke(px(2.0));
            edge.move_to(point(x0, y0));
            edge.line_to(point(x1 - cut, y0));
            edge.line_to(point(x1, y0 + cut));
            edge.line_to(point(x1, y1));
            edge.line_to(point(x0, y1));
            edge.close();
            if let Ok(path) = edge.build() {
                window.paint_path(path, rgba(CUT_GREY));
            }
        },
    )
    .absolute()
    .top(px(0.0))
    .left(px(0.0))
    .right(px(0.0))
    .bottom(px(0.0))
}

/// The close cross: two 1.4px diagonals across an 18px box, `BODY_SILVER`
/// at rest and `MAP_INK` hot — `chrome::paint_close` in another toolkit.
/// Painted through `canvas` because no box primitive draws a cross.
fn close_cross(hot: bool) -> impl IntoElement {
    let ink = if hot { MAP_INK } else { BODY_SILVER };
    canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let cx = bounds.origin.x + px(CLOSE) / 2.0;
            let cy = bounds.origin.y + px(CLOSE) / 2.0;
            let d = px(3.5);
            let mut path = PathBuilder::stroke(px(1.4));
            path.move_to(point(cx - d, cy - d));
            path.line_to(point(cx + d, cy + d));
            path.move_to(point(cx + d, cy - d));
            path.line_to(point(cx - d, cy + d));
            if let Ok(path) = path.build() {
                window.paint_path(path, rgba(ink));
            }
        },
    )
    .size_full()
}
/// A tracked-caps label: one div per character in a flex row whose gap IS the
/// tracking.
///
/// GPUI 0.2.2 has no letter spacing. `TextStyle` has no field for it and
/// `FontFeatures` cannot express tracking — egui kept it in the text format,
/// which is exactly what `view::TITLE_TRACKING`'s doc means by "the tracking
/// is the whole trick and needs an explicit layout job". This is that job.
///
/// It is exact rather than approximate: the gap between glyph boxes is the
/// requested tracking to the pixel, which no space-padding manages (a space
/// is a glyph with its own advance, so padding drifts with the string). The
/// cost is one element per character, and every tracked label in the shell
/// pays it — the island bands and the panel headers, which is the whole of
/// the console's tracked vocabulary.
///
/// The case is decided here, by `view::label_case`, so a label is measured in
/// the case it is drawn. A caller passing "readiness gate" gets the same
/// width as one passing "READINESS GATE".
fn tracked(label: &str, size: f32, ink: Rgba) -> impl IntoElement {
    let caps = view::label_case(label);
    div()
        .flex()
        .flex_row()
        .gap(px(view::LABEL_TRACKING))
        .font_family(view::FACE_HUMAN)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(size))
        .text_color(rgba(ink))
        .children(caps.chars().map(|c| div().child(c.to_string())))
}

/// A panel title: a tracked-caps label at the scale the label is for.
///
/// `view::LABEL_PT` rather than a size at the call site, so no panel invents
/// its own title size — the reason a converted console reads as one
/// instrument rather than as several people's panels.
fn panel_title(label: &str, ink: Rgba) -> impl IntoElement {
    tracked(label, view::LABEL_PT, ink)
}

/// The human face with tabular figures, as one `gpui::Font`.
///
/// GPUI 0.2.2 has no `font_features` builder on `Styled` — the feature list
/// only reaches a `TextStyle` through `.font(Font { .. })`, whose fields are
/// public. So the wrapper is built here, at the shell boundary, and the kernel
/// keeps its plain `(&str, u32)` and stays toolkit-free.
///
/// Keyed on `(family, features)`, which is also why this is one named thing
/// rather than a literal at every readout: a face registered twice with
/// different features gets two `LoadedFont` entries, and a console that does
/// that by accident is a console with two subtly different digit widths.
fn human_figures() -> gpui::Font {
    let mut font = gpui::font(view::FACE_HUMAN);
    font.features = gpui::FontFeatures(std::sync::Arc::new(vec![(
        view::TABULAR_FIGURES.0.to_string(),
        view::TABULAR_FIGURES.1,
    )]));
    font
}

/// A quantity drawn as a figure rather than as prose.
///
/// `tnum` is the whole point, and the defect it prevents is specific: without
/// it a value that goes 9 → 10 shifts every column to its right, because a
/// proportional `1` is narrower than a proportional `9`. On a console where a
/// count changes several times a minute, that is a layout that twitches under
/// the operator for no reason they can name. The measurement is in the slice
/// note — two captures, one digit apart, and the following control does not
/// move.
///
/// Sized through `view::readout_size` so there are two figure sizes in the
/// console and no third. `leads_panel` is the 30pt one, and nothing in the
/// console earns it yet: see the slice note for why that branch is tested and
/// not yet placed.
fn figure(value: &str, leads_panel: bool, ink: Rgba) -> impl IntoElement {
    div()
        .font(human_figures())
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(view::readout_size(leads_panel)))
        .text_color(rgba(ink))
        .child(value.to_string())
}

/// A body's line of prose: one line, weak ink.
///
/// The third of the body's three units, and the only one in the human face at
/// body size. Everything a machine produced is a `kv`; everything a human
/// wrote and that is not a quantity is a `note`.
fn note(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .font_family(view::FACE_HUMAN)
        .text_size(px(view::BODY_PT))
        .text_color(rgba(BODY_SILVER.with_alpha(204)))
        .child(text.into())
}

/// A ruled row: whatever the body puts in it, under a hairline.
///
/// The fourth unit, and the one a LIST needs. `kv` is a label and one machine
/// value; a roster row or a hull row is a name plus two or three machine
/// columns, and each of those is the same shape — content left, figures right,
/// hairline under. Building it from `kv` would mean one row per column, which
/// is a table read sideways.
fn ruled(children: Vec<AnyElement>) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .pb(px(5.0))
        .border_b(px(view::PANEL_EDGE))
        .border_color(rgba(HAIRLINE_SLATE))
        .children(children)
}

/// A machine column inside a `ruled` row: fixed width, right-aligned, so a
/// column of them lines up down the panel.
fn machine_col(value: impl Into<SharedString>, w: f32, ink: Rgba) -> impl IntoElement {
    div()
        .w(px(w))
        .flex_shrink_0()
        .flex()
        .flex_row()
        .justify_end()
        .font_family(view::FACE_MACHINE)
        .text_size(px(view::MACHINE_PT))
        .text_color(rgba(ink))
        .child(value.into())
}

/// A readout with its unit beside it: the largest thing a panel says.
///
/// Step's `0 of 1 / READY` and Review's `0 / OUTSTANDING` are the same shape —
/// a figure and a tracked label, and nothing else at that size. Ink is the
/// verdict's, so a clear gate reads neutral and a closed one spends the accent.
fn verdict(
    figure: impl Into<SharedString>,
    label: &'static str,
    ink: Rgba,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_baseline()
        .gap(px(8.0))
        .child(
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(view::readout_size(true)))
                .line_height(px(30.0))
                .font(human_figures())
                .text_color(rgba(ink))
                .child(figure.into()),
        )
        .child(
            div()
                .font_family(view::FACE_HUMAN)
                .font_weight(gpui::FontWeight::BOLD)
                .text_size(px(view::LABEL_PT))
                .text_color(rgba(HAIRLINE_SLATE))
                .child(view::label_case(label)),
        )
}

/// A section break: tracked caps, and a hairline filling the rest of the row.
///
/// The hairline is the point. A label alone is a heading with air under it,
/// which is a document; a label followed by a rule to the edge of the panel is
/// a ruled break, which is an instrument. One of these divides a body, and
/// nothing else does — the panel this replaced used eleven bare divs separated
/// by 4px gaps, so nothing divided it at all.
fn section(label: &str) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(tracked(label, view::LABEL_PT, CUT_GREY))
        .child(div().flex_1().h(px(1.0)).bg(rgba(HAIRLINE_SLATE)))
}

/// A key/value row: the instrument panel's basic unit.
///
/// Label left in tracked caps, machine value right-aligned, hairline under.
/// One line rather than the label-above-value stack the console used before,
/// and the reason is alignment: a stack puts every value at its own x, so
/// nothing lines up and the eye re-reads each row. A row lines the values into
/// a column the eye can track without moving.
///
/// The value is always in the machine face, because a key implies a server
/// value — an id, a state word, a count, a timestamp. Prose goes in `note`.
fn kv(label: &str, value: impl Into<SharedString>, ink: Rgba) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_baseline()
        .justify_between()
        .gap(px(10.0))
        .pb(px(5.0))
        .border_b(px(1.0))
        .border_color(rgba(HAIRLINE_SLATE))
        .child(tracked(label, view::LABEL_PT, CUT_GREY))
        .child(
            div()
                .font_family(view::FACE_MACHINE)
                .text_size(px(view::MACHINE_PT))
                .text_color(rgba(ink))
                .child(value.into()),
        )
}

/// The map's instrument chrome: a true scale bar and a north mark.
///
/// Both are drawn OVER the tile canvas and neither takes space from it —
/// `DESIGN.md`'s rule that UI never insets the map. Both are absent rather
/// than approximate when the console cannot know the answer, which for the
/// bar means before the first frame and for a replay means always.
///
/// `map_w`/`map_h` are the WINDOW's, and the chrome is placed in the visible
/// map: right of the docked column, because a bar drawn under a panel is a
/// bar nobody can read.
fn map_chrome(_map_w: f32, _map_h: f32, _column_w: f32, center: (f64, f64), zoom: f64) -> Vec<AnyElement> {
    // DISABLED, deliberately, and this is not a stub.
    //
    // Everything below the fold is correct and stays: the camera arrives
    // (`center`, `zoom`), `scale_bar` resolves a real reading — at zoom 11
    // over Jakarta it returns `10 km at 131.6 px`, which is the right answer —
    // and the kernel is tested. What does not work is getting the element onto
    // the screen, and the bisect is recorded here so it is not repeated.
    //
    // The full record is in `.impeccable/surfaces/gpui-console.md` under slice
    // 5. The short version:
    //
    //   - An absolutely positioned element with an explicit `left` plus a
    //     `width` and NO opposing edge does not paint. A 200x40 solid red box
    //     at (400,700) came out as bare map. The zone grip is built that way
    //     and is measurably absent, so the console's dock-resize handle is
    //     invisible — a PRE-EXISTING DEFECT, not something this slice caused.
    //   - The STRETCH idiom (`.absolute().left(0).right(0)` + `top` + `h`)
    //     paints the element's own background: measured, a 562-unit delta
    //     where the scrim lands.
    //   - It does NOT paint the element's children, which is the open problem.
    //     `top_zone` is built the same way and lays its children out fine.
    //     Tried and rejected, each recorded rather than guessed at: `pl()`
    //     padding-left killed a strip outright; `pt`/`pr` killed another;
    //     `py`/`px` changed nothing; a flex spacer changed nothing; a nested
    //     `flex_col` inside a `flex_1` child changed nothing; flattening every
    //     child to the strip level made it worse and stopped the scrims too.
    //
    // Why it is gated rather than left half-working: the scrims DO paint, so
    // an enabled version leaves two unexplained dark bands over the map with
    // nothing in them. That is worse than no chrome. The kernel and the
    // plumbing are the durable part and they are done.
    //
    // Where to start: a minimal reproduction outside this shell — one window,
    // one stretched absolute div with a background and one child — and compare
    // it against `top_zone`'s exact shape until the difference is a single
    // property. That is a twenty-minute job with a scratch binary and it has
    // not been done from inside a 21k-line console.
    let _ = (center, zoom);
    Vec::new()
}

/// A machine row: an id, a timestamp, a wire value, a count.
///
/// PT Mono is already even, so alignment needs no feature tag — the face does
/// it, which is why every machine row in the console is tabular by
/// construction rather than by flag.
///
/// Takes the text by value rather than by reference so the element owns it:
/// under Rust 2024 an `impl IntoElement` that borrows a `&str` parameter
/// cannot escape the call, which rules out building one inside a closure.
fn machine(text: impl Into<SharedString>, ink: Rgba) -> impl IntoElement {
    div()
        .font_family(view::FACE_MACHINE)
        .text_size(px(view::MACHINE_PT))
        .text_color(rgba(ink))
        .child(text.into())
}

impl IslandDemo {
    /// Render one island of the column.
    ///
    /// The frame is DOCK-BOUND and STACKED: `origin` comes from
    /// `view::zone_slot_origin` over the state's slots, so there is no
    /// stored position and no drag — which is the console's arrangement
    /// (`zone_island_origins_with`) and the difference between a console's
    /// column and a desktop's floating windows. The caller places this
    /// inside the band clip, so the frame sits band-relative here: BOTH
    /// axes relativize (`origin - band origin - scroll`). Missing the x
    /// relativization once parked every island at double offset — visible
    /// only after the band left x0 for the zone rails.
    ///
    /// `owned` arrives resolved (ADR-0016: decided once, never per-island).
    /// `scroll` is the column's view transform, applied at paint, never
    /// folded into the origin.
    fn island(
        &mut self,
        cx: &mut Context<Self>,
        key: view::IslandKey,
        height: f32,
        title: &'static str,
        trailing: String,
        body: AnyElement,
        owned: bool,
        origin: (f32, f32),
        scroll: f32,
        clip: view::Box,
    ) -> impl IntoElement {
        // Copied out front so the fit log below can write back through
        // `self` without fighting the borrow it reads from.
        let idx = self
            .islands
            .iter()
            .position(|st| st.key == key)
            .expect("an island in the column has state");
        let close_hot = self.islands[idx].close_hot;
        let handle = self.islands[idx].scroll.clone();
        let (body_id, body_key) = self.islands[idx].body_id();
        // The island is a FIXED footprint. GPUI needs an explicit outer frame
        // because the bevel and the shadow paint outside the panel box, and
        // an element that overflows needs somewhere to overflow into.
        //
        // The width is the LIVE zone width, so the grip's resize moves the
        // whole column rather than leaving the islands at a remembered size.
        let panel_w = px(self.zone_w);
        let panel_h = px(height);

        // --- the silhouette: one edge, and nothing else --------------------
        //
        // The old island spent nine elements saying "this is a panel": a hard
        // cast shadow, a bevelled steel casing of three nested fills, a top
        // light, a bottom shade, and a 2px rim riding 3px inside all of it.
        // Every one of those is gone. A panel is now square, flat, navy, with
        // a single 1px edge that is the whole of its state language.
        //
        // This is not a simplification for its own sake. A lit rim on a
        // bevelled, shadowed, chamfered panel is one ornament among nine; a
        // hairline that turns Radar Cyan on a flat navy field is the whole
        // event. The accent only means something once nothing else competes
        // for the eye, which is the same failure the old system recorded when
        // every rim was lit and "cyan" stopped saying anything at all.
        //
        // The width is the LIVE zone width, so the grip's resize moves the
        // whole column rather than leaving the islands at a remembered size.
        // --- the title band ------------------------------------------------
        // A real font weight, chosen as a number. egui cannot express this
        // without registering one family per weight, which is the whole of
        // what ADR-0018 had to work around.
        //
        // No drag handler here, deliberately: the band identifies the island
        // and carries its trailing note, and the island's place is the
        // stack's. A press on the band is what PROMOTES nothing — there is
        // nothing to promote in a column that cannot overlap — so the only
        // gesture the band owns is the close cross at its right.
        let closeable = Self::island_closeable(key);
        let band = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(TITLE_H))
            // Asymmetric on purpose: the right pad clears the close cross, so
            // the trailing note yields instead of running under it. The
            // kernel owns the number (`island_band_pad_right`).
            .pl(px(view::PANEL_PAD))
            .pr(px(view::island_band_pad_right(closeable)))
            .bg(rgba(PANEL_SLATE))
            .border_b(px(view::PANEL_BAND_RULE))
            .border_color(rgba(HAIRLINE_SLATE))
            // The tracked label is fixed and the trailing note yields. A
            // tracked label is one element per character, so it cannot shrink
            // without the tracking drifting apart mid-word — which is a
            // visibly broken label rather than a narrower one.
            .child(
                div()
                    .flex_shrink_0()
                    .child(panel_title(title, MAP_INK)),
            )
            // A PLAIN child, and deliberately so. It was a `flex_shrink` +
            // `overflow_hidden` + `justify_end` box, and a capture showed one
            // glyph per string coming out as a notdef box — the tell of a
            // per-glyph-run clip, not of a missing glyph (the same family
            // renders the assumed-time stamp clean a panel below). `justify_
            // between` on the band already pushes this to the right edge, so
            // the box bought nothing and cost a broken glyph.
            // `machine()` rather than a hand-built div, for one more reason
            // than the face: the trailing note is always a machine value — a
            // count, a state word, a call sign — so it is in the machine face
            // at the machine size, decided once here instead of per panel.
            .child(machine(trailing, BODY_SILVER));

        // --- the body ------------------------------------------------------
        // An island's footprint is fixed by contract, so a body that does
        // not fit scrolls. The scroll area needs an id to be stateful, and
        // that id is what makes the offset survive between frames — keyed
        // per island, so one shared id cannot weld every body to one
        // offset (`chrome.rs`: `id_salt`, same rule).
        //
        // The background is explicit, not inherited: the scroll viewport
        // paints its own opaque default (light), which no ancestor bg
        // shows through. An island body on an uninherited default is a
        // white panel — invisible light-on-light rows.
        //
        // `track_scroll` feeds the island's handle during layout: offset,
        // viewport and content extent, no custom element, no frame of lag.
        let mut scroller = div()
            .id((body_id, body_key))
            .flex_1()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&handle)
            .bg(rgba(CONSOLE_NIGHT))
            .flex()
            .flex_col();
        scroller = scroller.child(body);
        // The live measurement, read at the shell boundary: GPUI's offset
        // runs negative-down, the kernel's positive-down, so the sign
        // flips here and nowhere else. `content` is extent over viewport.
        let max = handle.max_offset();
        let viewport_h = f32::from(handle.bounds().size.height);
        let content_h = f32::from(max.height) + viewport_h;
        let offset = -f32::from(handle.offset().y);
        let fit = BodyFit {
            content_h,
            viewport_h,
            offset,
        };
        // Zone-debug style, machine-readable: log on change only, so the
        // numbers are grepable without drowning the log at 60fps.
        if self.islands[idx].last_fit != Some(fit) {
            self.islands[idx].last_fit = Some(fit);
            eprintln!(
                "fit island={} content={content_h:.0} viewport={viewport_h:.0} offset={offset:.0} down={} up={}",
                key.id(),
                fit.can_scroll_down(),
                fit.can_scroll_up(),
            );
        }

        // --- the edge -------------------------------------------------------
        // The panel's only border, and the whole of its state language.
        //
        // `CUT_GREY` at rest and `RADAR_CYAN` on the one panel that owns
        // input, from one boolean the caller resolved before painting. Full
        // alpha on the lit one: a 1px rule does not glow, it states, and it is
        // on one panel at a time.
        //
        // Inset 0 and width 1 — the edge IS the border, where the old rim was
        // a second line riding 3px inside a bevelled casing. It keeps the
        // rim's `block_mouse_except_scroll()` hitbox, which is what stops the
        // map behind the panel taking the pointer. That call truncates
        // HOVER at the edge — every mouse listener requires its hitbox to
        // be hovered — so the edge must sit UNDER the body in tree order:
        // controls above it stay hoverable, gaps still shield the map.
        // An edge painted last is a full-panel click black hole, and no
        // comment claiming otherwise survives contact with `hit_test`.
        let edge = div()
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .right(px(0.0))
            .bottom(px(0.0))
            .border(px(view::PANEL_EDGE))
            .border_color(rgba(view::panel_edge(owned)))
            .block_mouse_except_scroll();

        // --- the close button, LAST ---------------------------------------
        // This order is the whole fix it ports (`chrome.rs`: the close
        // button is registered after the body scope). egui gives the click
        // to the last-registered widget where rects overlap; GPUI resolves
        // hits in tree order with the later sibling on top, so the button
        // is the last child and wins the band and the body alike. Move it
        // above either and the ✕ lights up and swallows nothing — the exact
        // bug being ported away.
        //
        // Geometry is the kernel's: `island_close_center` puts the 18px box
        // clear of the chamfer diagonal, same numbers the egui shell uses.
        let (close_cx, close_cy) = island_close_center((0.0, 0.0, self.zone_w, height));
        let close = div()
            .absolute()
            .left(px(close_cx - CLOSE * 0.5))
            .top(px(close_cy - CLOSE * 0.5))
            .w(px(CLOSE))
            .h(px(CLOSE))
            .id(("island-close", key as usize))
            .cursor_pointer()
            .on_hover(cx.listener(move |view, hot: &bool, _, cx| {
                if let Some(st) = view.islands.iter_mut().find(|st| st.key == key) {
                    st.close_hot = *hot;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |view, _, _, cx| {
                view.set_island_open(key, false);
                cx.notify();
            }))
            .child(close_cross(close_hot));

        let mut panel = div()
            .absolute()
            .left(px(origin.0 - clip.0))
            .top(px(origin.1 - clip.1 - scroll))
            .w(panel_w)
            .h(panel_h)
            .rounded(px(view::PANEL_RADIUS))
            .bg(rgba(CONSOLE_NIGHT))
            // Painted FIRST, so the edge is the bottommost thing on the
            // panel: GPUI resolves hits and paint in tree order, later
            // siblings on top, and every mouse listener requires its
            // hitbox to be hovered — hover the edge truncates at itself.
            // An edge painted last is a full-panel click black hole: no
            // body control under it can ever fire, which is exactly the
            // defect this ordering fixes (row clicks landed and died).
            // The border is at the panel's outer bounds where content
            // never paints, so painting it under the content is
            // invisible — and it still shields the map behind the panel
            // in the gaps, which is the hitbox's whole job.
            //
            // The border lives here and ONLY here. Putting it on the panel
            // as well draws a second line one pixel inside the first, and
            // a 1px edge that measures 2px is a specification the build
            // does not meet. The first capture of this slice showed exactly
            // that: the lit perimeter ran 3,207 pixels against a computed
            // 1,844, which is the tell.
            .child(edge)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .overflow_hidden()
                    .child(div().relative().h(px(TITLE_H)).child(band))
                    .child(scroller),
            );
        // LAST, and only where it does something: see `island_closeable`.
        if Self::island_closeable(key) {
            panel = panel.child(close);
        }
        panel
    }
}

impl IslandDemo {
    /// Join the wire's liveness to the roster rows, by unit id.
    ///
    /// The register names the hulls; the live feed says which are alive.
    /// They meet on the unit id both carry as a decimal string, so this is
    /// the one place the two sources are tied. A unit id the feed names but
    /// the register does not is ignored (no row to mark); a row the feed is
    /// silent about keeps its seeded value.
    fn apply_liveness(&mut self, liveness: Vec<LiveMarker>) {
        for row in &mut self.roster {
            if let Some(m) = liveness.iter().find(|m| m.id == row.unit_id) {
                row.stale = m.stale;
            }
        }
        // The marker layer draws the same batch: a live ship the register
        // does not name still has a position, and the map is where to say so.
        self.live = liveness;
        // Seed the helm draft ONCE, from the commanded hull's own fix: the
        // panel opens on the hull's real course instead of on zero, and an
        // operator's later edits are never overwritten.
        if self.helm_heading.is_none() {
            if let Some(ship) = self.session.mine.first() {
                if let Some(m) = self.live.iter().find(|m| m.id == ship.id) {
                    self.helm_heading = m.heading_deg;
                    self.helm_speed = m.speed_kn;
                }
            }
        }
    }

    /// Build the view's state. Split out of `run` so a test can construct
    /// it headlessly — no window, no wire, no map thread — and drive the
    /// selection handler the way the compositor would.
    fn new(
        roster: Vec<RosterRow>,
        roster_total: usize,
        selected: usize,
        session: SessionView,
        order_tx: OrderSink,
    ) -> Self {
        // The column's islands, one entry each, in a fixed order. The
        // ORDER here is not the render order — that is `view::zone_islands`'
        // job and it changes with the exercise's state. What lives here is
        // the operator's own choice per island, which must survive a state
        // change: closing Crew in planning and keeping it closed in
        // execution is one choice, and a list rebuilt from the state would
        // silently re-open it under a different slot.
        // Read before the struct takes the session: the camera's opening zoom
        // is the shell's own copy of what it asked the map thread for.
        let map_zoom = std::env::var("TFG_ZOOM")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(session.zoom)
            .clamp(ZOOM_MIN, ZOOM_MAX);
        // Seeded relate-open for a headless capture, with the working set the
        // open would have built from the session's read. Same precedent as
        // the panels: pin the state, never the gesture.
        let relate_open = if std::env::var("TFG_RELATE").as_deref() == Ok("1") {
            session
                .book
                .first()
                .and_then(|s| s.steps.first().map(|st| (s.id, st.id)))
        } else {
            None
        };
        let relate_working = relate_open
            .and_then(|(sid, st)| {
                session
                    .related
                    .iter()
                    .find(|r| r.scenario_id == sid && r.step_id == st)
            })
            .map(|r| r.posts.clone())
            .unwrap_or_default();
        let mut demo = IslandDemo {
            islands: view::COLUMN_ISLANDS
                .iter()
                .map(|k| IslandState::new(*k))
                .collect(),
            // `TFG_SETTINGS` opens the Settings panel for a headless capture,
            // the same seed-the-state precedent as `TFG_SEL`.
            modal_open: std::env::var("TFG_SETTINGS").as_deref() == Ok("1"),
            watching: false,
            show_side_zone: true,
            // Left, the console's own default (`main.rs:20669`). The zone
            // floats OVER the map rather than insetting it, so the dock is
            // which edge it covers — and a left dock shifts the map's visible
            // centre right by half the zone's width, which
            // `view::camera_centre_shift` answers and the jump honours.
            side_dock_left: true,
            reduced_motion: false,
            // The COLUMN's islands are OPEN by default, which is the
            // console's arrangement: its zone islands are all shown, and a
            // column the operator has to open one panel at a time is a column
            // nobody reads. Their seeds flipped direction with the move — they
            // used to open a centred modal (`TFG_PEOPLE=1`), and now the
            // interesting photograph is the NARROWER column, so they close one
            // (`TFG_PEOPLE=0`). A seed that forced something open would be a
            // capture of a state nobody is in.
            people_open: std::env::var("TFG_PEOPLE").as_deref() != Ok("0"),
            crew_open: std::env::var("TFG_CREW").as_deref() != Ok("0"),
            command_open: std::env::var("TFG_COMMAND").as_deref() != Ok("0"),
            step_open: std::env::var("TFG_STEP").as_deref() != Ok("0"),
            // The feed starts empty and is only ever appended to: there is no
            // events GET to seed it from, and inventing history on launch
            // would be a log of things that did not happen here.
            log_feed: VecDeque::new(),
            // Which tab opens. A capture seed, because the only tab with
            // anything in it on this account is Timeline and a photo of the
            // Summary proves nothing about the other three. Default 0, so an
            // unseeded launch opens the Summary the console opens.
            // `None`, and deliberately: the target is resolved per frame
            // against the LIVE roster (see `order_target`), so a remembered
            // id that no longer belongs to this seat would order a hull the
            // operator cannot see.
            helm_target: None,
            assessment_tab: std::env::var("TFG_ASSESS_TAB")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(0)
                .min(3),
            timeline_source: None,
            timeline_personnel: String::new(),
            timeline_unit: String::new(),
            timeline_from: String::new(),
            timeline_to: String::new(),
            timeline_busy: false,
            timeline_status: String::new(),
            judgements_busy: false,
            judgements_status: String::new(),
            reviews_busy: false,
            reviews_status: String::new(),
            review_mine_only: false,
            judge_subject: None,
            judge_score: String::new(),
            judge_score_handle: None,
            judge_score_focused: false,
            judge_write_busy: false,
            review_subject: None,
            review_body: String::new(),
            review_body_handle: None,
            review_body_focused: false,
            review_editing: None,
            review_write_busy: false,
            timeline_focus: 0,
            timeline_handles: Default::default(),
            // Comms is NOT a column island — it is a modal door now, like
            // Settings and the composer — so it keeps the MODAL seeds'
            // semantics and starts CLOSED. The inverted form above would have
            // opened the inbox over the console on every launch, which is
            // exactly the capture that kept appearing: not a stray focus click
            // at all, a flag that defaulted the wrong way.
            comms_open: std::env::var("TFG_COMMS").as_deref() == Ok("1"),
            inspect_open: std::env::var("TFG_INSPECT").as_deref() == Ok("1"),
            // No seeded pointer. The prototype seeded one over an island so a
            // capture would show a lit rim with no input; a console's rims
            // rest until the operator's cursor is over one, and a capture
            // that shows a lit edge over an empty map is a capture of a state
            // nobody is in. (The focus click the compositor delivers on map
            // sets a real one anyway — see the handoff's capture hazards.)
            // Seeded from `TFG_POINTER` below, next to the other capture
            // seeds, so the resting state and the photographed one are one
            // field rather than two.
            // At the top of the stack, which is where the console's column
            // starts: a console whose first island is already off screen would
            // be a console nobody recognises, and the view transform is
            // kernel-tested anyway.
            //
            // `TFG_SCROLL` is the capture seed for the SCROLLED column, and it
            // exists because the gesture cannot be driven on this machine:
            // GPUI does not present on Xvfb, and `uimouse.py`'s uinput pointer
            // moves the real cursor but its buttons and wheel never reach the
            // window (`scripts/ui-shoot/README.md`, "Where the Wayland path
            // stopped"). The wheel itself is covered by
            // `a_wheel_over_the_column_moves_the_column`, which dispatches a
            // real `ScrollWheelEvent` through GPUI's own dispatch — including
            // the `should_handle_scroll` gate that was the bug. This seed
            // photographs the half a field assertion cannot see: that the
            // offset actually MOVES the islands.
            zone_scroll: std::env::var("TFG_SCROLL")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0),
            zone_w: 360.0,
            grip: None,
            grip_hot: false,
            gate: FrameSlot::new(),
            incoming: None,
            map_center: (0.0, 0.0),
            map_render_zoom: 0.0,
            roster,
            roster_total,
            // Seeded open for a headless capture (`TFG_SESSIONS=1`): three
            // fixture games across the states, unheld — the picker with
            // something to pick, proving the surface rather than the seed.
            sessions: if std::env::var("TFG_SESSIONS").as_deref() == Ok("1") {
                vec![
                    SessionRow {
                        id: 44,
                        name: "GPUI step proof".to_string(),
                        state: "execution".to_string(),
                        my_role: "Commando".to_string(),
                        judge: false,
                    },
                    SessionRow {
                        id: 38,
                        name: "probe-gm-roule".to_string(),
                        state: "preparation".to_string(),
                        my_role: "Penyelenggara".to_string(),
                        judge: true,
                    },
                    SessionRow {
                        id: 9,
                        name: "Game 8".to_string(),
                        state: "planning".to_string(),
                        my_role: "Commando".to_string(),
                        judge: false,
                    },
                ]
            } else {
                Vec::new()
            },
            hold_busy: false,
            hold_status: String::new(),
            session,
            helm_heading: None,
            helm_speed: None,
            helm_status: None,
            order_tx,
            map_zoom,
            pan: None,
            // `TFG_API_USER`/`TFG_API_PASS` still work and prefill the card,
            // so a headless capture and a human run the same path.
            login_identifier: std::env::var("TFG_API_USER").unwrap_or_default(),
            login_password: std::env::var("TFG_API_PASS").unwrap_or_default(),
            login_focus: Cred::Identifier,
            login_status: String::new(),
            login_busy: false,
            login_handles: [None, None],
            // `TFG_COMPOSE` opens the composer for a headless capture, the
            // same seed-the-state precedent as `TFG_SETTINGS`. The draft
            // stays EMPTY under it: a seeded name would be one stray click
            // from a real game, and the capture wants the validation state.
            composer_open: std::env::var("TFG_COMPOSE").as_deref() == Ok("1"),
            draft: SessionDraft::default(),
            compose_focus: 0,
            compose_handles: [None, None, None, None, None, None, None, None, None, None, None],
            compose_busy: false,
            compose_status: String::new(),
            // `TFG_BOOK` opens the book for a headless capture; `TFG_GAME`
            // picks the game whose shelf it shows, because the default hold
            // (a preparation game) keeps an empty book.
            book_open: std::env::var("TFG_BOOK").as_deref() == Ok("1"),
            book_title: String::new(),
            book_busy: false,
            book_status: String::new(),
            book_open_scenario: None,
            step_draft: ComposerDraft::default(),
            step_busy: false,
            readiness_busy: false,
            readiness_status: String::new(),
            step_declare_busy: false,
            step_declare_status: String::new(),
            msg_content: String::new(),
            msg_busy: false,
            msg_status: String::new(),
            force: crate::force::ForceDraft::new(),
            factor_draft: None,
            factor_busy: false,
            factor_status: String::new(),
            clock_running: None,
            clock_denied: false,
            pause_busy: false,
            pause_status: String::new(),
            advance_busy: false,
            advance_status: String::new(),
            end_busy: false,
            end_status: String::new(),
            fleet_open: std::env::var("TFG_FLEET").as_deref() == Ok("1"),
            dragging: false,
            thumbs: HashMap::new(),
            fleet_tax: FleetTaxonomy::default(),
            // Seeded drill for a headless capture: the leaf column (and its
            // thumbnails) is four clicks deep, and a capture cannot click.
            // Same precedent as `TFG_SEL` — pin the state, never the gesture.
            drill_group: std::env::var("TFG_DRILL_GROUP")
                .ok()
                .and_then(|v| v.parse().ok()),
            drill_category: std::env::var("TFG_DRILL_CATEGORY")
                .ok()
                .and_then(|v| v.parse().ok()),
            drill_type: std::env::var("TFG_DRILL_TYPE")
                .ok()
                .and_then(|v| v.parse().ok()),
            drill_class: std::env::var("TFG_DRILL_CLASS")
                .ok()
                .and_then(|v| v.parse().ok()),
            player_open: std::env::var("TFG_PLAYER").as_deref() == Ok("1"),
            // Seeded open for a headless capture: the host whose
            // passengers the modal reads. The read itself fires from the
            // modal's first draw (see `embark_body`), so seeding the host
            // is seeding the whole open.
            embark_host: std::env::var("TFG_EMBARK")
                .ok()
                .and_then(|v| v.parse::<i64>().ok()),
            embarked: HashMap::new(),
            embark_over: HashMap::new(),
            embark_busy: false,
            embark_status: String::new(),
            roster_busy: false,
            roster_status: String::new(),
            roster_draft: crate::roster::RosterDraft::new(),
            force_busy: false,
            force_status: String::new(),
            deploy_busy: false,
            prepare_busy: false,
            prepare_status: String::new(),
            pending_place: None,
            place_down: None,
            // `TFG_RELATE` is seeded above, with the working set the open
            // would have built — the literal only takes the values.
            relate_open,
            relate_working,
            relate_busy: false,
            // `TFG_SEL` seeds the selection for a headless capture, the same
            // way `TFG_POINTER` seeds the pointer and `TFG_FRAME` seeds the
            // map: a way to pin the state without a live gesture (the
            // compositor makes a synthetic click unreliable here).
            selected,
            // The seeded pointer, which two comments in this file already
            // promised and nothing ever read. It exists because the lit edge
            // is the whole state language of the new panel silhouette, and a
            // state language with no capture is a claim rather than a fact:
            // the compositor makes a synthetic click unreliable here, so the
            // honest way to photograph "exactly one edge is cyan" is to pin
            // the pointer where it would have to be for that to be true.
            //
            // `x,y` in window coordinates, the same convention
            // `owning_island` resolves against. Absent or malformed means
            // `None`, which is the real resting state — no panel owns input —
            // so a capture without the seed is a capture of the console at
            // rest, not a broken one.
            pointer: std::env::var("TFG_POINTER").ok().and_then(|v| {
                let (x, y) = v.split_once(',')?;
                Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
            }),
            sheet: std::env::var("SHEET").as_deref() == Ok("1"),
            probe_sprite: halo_sprite(halo::Halo::Signal, 0.55),
            drag_start: None,
            drag_moved: false,
            live: Vec::new(),
            map_dims: (0.0, 0.0),
            reveal: None,
        };
        // Seed the drafts from the held session. The startup hold CAN be a
        // planning game with pieces on it, and a draft that started empty
        // would read every one of those pieces as a removal the first time a
        // plan was made. Nothing is staged at construction, so this discards
        // nothing — see `reseed_drafts`.
        demo.reseed_drafts();
        demo
    }

    /// The Settings panel — the console's `settings_modal` ported, and the one
    /// panel that needs no session: view mode, layout and motion are all
    /// client state.
    ///
    /// Two of the console's controls are deliberately absent, and the panel
    /// says so in a sentence rather than showing a control with no effect: the
    /// geographic grid needs the map-grid slice (its projection lives in
    /// `map_render`, which carries MapLibre and the shell cannot name), and the
    /// text-size scale needs a global scale the shell's fixed sizes do not have
    /// yet. The close button clears the chamfer, because nothing clickable may
    /// sit on the diagonal.
    fn settings_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let small = |t: &'static str| {
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        let rule = || div().h(px(1.0)).w_full().bg(rgba(HAIRLINE_SLATE));
        let touch = |id: &'static str, el: AnyElement| div().id(id).cursor_pointer().child(el);

        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .size_full()
            .overflow_hidden()
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .h(px(TITLE_H))
                .child(
                    div()
                        .font_family(view::FACE_HUMAN)
                        .text_size(px(TITLE_SIZE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgba(MAP_INK))
                        .child("Settings"),
                )
                .child(
                    div()
                        .id("close-settings")
                        .cursor_pointer()
                        .text_size(px(14.0))
                        .text_color(rgba(BUTTON_LABEL))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.modal_open = false;
                            cx.notify();
                        }))
                        .child("\u{00d7}"),
                ),
        );
        col = col.child(rule());
        // View: ONE toggle, not a mode switch — the console's own note.
        col = col.child(small("VIEW"));
        col = col.child(touch(
            "set-watch",
            checkbox(self.watching, true, "Watch only").into_any_element(),
        ));
        col = col.child(
            div()
                .text_size(px(10.5))
                .text_color(rgba(BODY_SILVER.with_alpha(204)))
                .child(if self.watching {
                    "watching \u{2014} the map follows the session and nothing here moves it."
                } else {
                    "running \u{2014} this machine directs the exercise."
                }),
        );
        col = col.child(rule());
        // Layout: the side zone, its dock, and its width (live in `zone_w`).
        col = col.child(small("LAYOUT"));
        col = col.child(touch(
            "set-sidezone",
            checkbox(self.show_side_zone, true, "Side zone").into_any_element(),
        ));
        if self.show_side_zone {
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(small("Docked"))
                    .child(touch(
                        "dock-left",
                        radio(self.side_dock_left, "left").into_any_element(),
                    ))
                    .child(touch(
                        "dock-right",
                        radio(!self.side_dock_left, "right").into_any_element(),
                    )),
            );
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(small("Width"))
                    .child(
                        div()
                            .id("width-down")
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.zone_w = (view.zone_w - 20.0).max(240.0);
                                cx.notify();
                            }))
                            .child(button("\u{2212}", true, false, false)),
                    )
                    .child(
                        div()
                            .font_family(view::FACE_MACHINE)
                            .text_size(px(11.0))
                            .text_color(rgba(MAP_INK))
                            .child(format!("{:.0}", self.zone_w)),
                    )
                    .child(
                        div()
                            .id("width-up")
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.zone_w = (view.zone_w + 20.0).min(520.0);
                                cx.notify();
                            }))
                            .child(button("+", true, false, false)),
                    )
                    .child(
                        div()
                            .id("width-reset")
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.zone_w = 360.0;
                                cx.notify();
                            }))
                            .child(button("reset", true, false, false)),
                    ),
            );
        }
        col = col.child(rule());
        col = col.child(small("MOTION"));
        col = col.child(touch(
            "set-motion",
            checkbox(self.reduced_motion, false, "Reduce motion").into_any_element(),
        ));
        col = col.child(rule());
        col = col.child(
            div()
                .text_size(px(10.0))
                .text_color(rgba(BODY_SILVER.with_alpha(178)))
                .child(
                    "Geographic grid and text size need the map-grid and text-scale \
                     slices \u{2014} not wired yet. Nothing here is remembered across launches.",
                ),
        );
        col.into_any_element()
    }

    /// One zoom step, clamped exactly as the console clamps, then ASKED FOR.
    /// The shell never scales the picture itself: it tells the binary the
    /// camera moved, the map thread renders at the new zoom, and the markers
    /// stay aligned because the binary projects them at the same camera.
    fn zoom_by(&mut self, delta: f64, cx: &mut Context<Self>) {
        let next = (self.map_zoom + delta).clamp(ZOOM_MIN, ZOOM_MAX);
        if next != self.map_zoom {
            self.map_zoom = next;
            let _ = self.order_tx.send(ToBinary::Zoom(next));
            cx.notify();
        }
    }

    /// One pan step, in MAP pixels. The caller converts from window points
    /// because the shell is the side that knows both the viewport and the
    /// frame's size; the sign and the unprojection are the binary's.
    fn pan_by(&mut self, dx: f64, dy: f64, cx: &mut Context<Self>) {
        if dx == 0.0 && dy == 0.0 {
            return;
        }
        let _ = self.order_tx.send(ToBinary::Pan { dx, dy });
        cx.notify();
    }

    /// Centre the map on the SELECTED hull — the console's `request_frame`.
    ///
    /// Returns false (and sends nothing) when the selection has no fix: a
    /// hull that has never reported has no ground to centre on, and asking
    /// the map thread to frame `(0, 0)` would be a lie about where it is.
    /// The shift compensates the floating zone (`camera_centre_shift`), so a
    /// centred hull lands on the VISIBLE middle rather than under the chrome.
    fn jump_to_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let fix = self
            .roster
            .get(self.selected)
            .and_then(|r| self.live.iter().find(|m| m.id == r.unit_id));
        let Some(m) = fix else {
            return false;
        };
        let shift_px = camera_centre_shift(self.zone_w, self.side_dock_left, !self.show_side_zone);
        let _ = self.order_tx.send(ToBinary::Jump {
            lat: m.latitude,
            lon: m.longitude,
            shift_px,
        });
        cx.notify();
        true
    }

    /// Sign out: tell the binary to drop its token, then show the card.
    ///
    /// Local first, like the console's `sign_out` (which never blocks on the
    /// server round trip): the session flag is what `render` reads, so the
    /// card is one notify away, and a hull that never reported leaves no
    /// live markers behind for the next sign-in to inherit.
    fn sign_out(&mut self, cx: &mut Context<Self>) {
        let _ = self.order_tx.send(ToBinary::SignOut);
        self.session.signed_in = false;
        self.live.clear();
        self.helm_status = None;
        self.login_busy = false;
        self.login_password.clear();
        self.login_status = "signed out".to_string();
        cx.notify();
    }

    /// The top zone: the one bar an operator reads under time pressure —
    /// whether the link is up, how many of the exercise are in the room, and
    /// which game is held. The port of the console's `top_zone`, cut to the
    /// readouts the GPUI binary can source: the clock is its own slice.
    ///
    /// The link dot is honest about what it knows: the shell sees liveness
    /// only as live markers, so a quiet wire and an empty map read the same —
    /// which is why the word is `idle`, never `faulted`. The shell has not
    /// been told that difference yet, and guessing it would be the lie the
    /// header's link states exist to avoid.
    /// The top band: the console's one row, cut to what this shell sources.
    ///
    /// The console's composition (`main.rs::top_zone`) is a single row of
    /// GROUPS separated by rules, not a list of buttons: the link, the
    /// headcount, the held game, the create door, the camera — and at the
    /// right end a right-to-left group holding the way out and the doors.
    /// The separators are the whole reason it reads as an instrument rather
    /// than as a toolbar: without edges the groups run together and nothing
    /// says which readout belongs to which action.
    ///
    /// The console's bar also carries a clock (no clock GET exists — the
    /// backend's pause/resume/factor writes ARE the read, so a ticking clock
    /// needs the Minos clock channel) and a comms door. The door is here; the
    /// clock is named on the Step island, which shows the server's own
    /// `assumed_time` from `current-step` rather than inventing one.
    fn top_zone(&mut self, cx: &mut Context<Self>, viewport_w: f32) -> impl IntoElement {
        // A rule between groups, 1px in the band's own hairline. Full band
        // height minus its padding, so it reads as a divider and not a border.
        let sep = || div().w(px(1.0)).h(px(18.0)).bg(rgba(HAIRLINE_SLATE));
        let readout = |text: String| machine(text, BODY_SILVER);
        // The link state, as a tracked-caps stamp in its own status ink:
        // the two words the band reads at a glance, and the reason the dot
        // is there at all — colour alone is not a state a room can read.
        //
        // The link is honest about its limits: the shell sees liveness only
        // as live markers, so a quiet wire and an empty map look the same —
        // which is why the word is `idle`, never `faulted`. Guessing that
        // difference is the lie the console's link states exist to avoid.
        let link = if self.live.is_empty() {
            (IDLE_GREY, "idle")
        } else {
            (RADAR_CYAN, "live")
        };
        let mut left = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().w(px(8.0)).h(px(8.0)).rounded(px(4.0)).bg(rgba(link.0)))
                    .child(tracked(link.1, view::LABEL_PT, link.0)),
            );
        // The headcount and the held game only exist in a session, so their
        // separators are gated with them rather than left dangling — the
        // console's own rule, and a dangling rule between nothing reads as a
        // rendering fault.
        //
        // They are ALSO gated on width, and that is the console's rule too
        // (`HINT_BUDGET`): the band is one row of readouts AND actions, and a
        // left group that eats the width pushes the right-hand group off the
        // edge — taking sign-out with it. The readouts yield; the doors never
        // do, because a band whose buttons have scrolled off is a console with
        // no way out. The game's name is not lost by this: the Step island's
        // body and the Inspector both carry it.
        if !self.session.game.is_empty() && viewport_w > view::BAND_READOUT_BUDGET {
            left = left.child(sep());
            left = left.child(readout(format!(
                "{}/{} in the room \u{00b7} {} ready",
                self.session.present, self.session.seats, self.session.ready
            )));
            left = left.child(sep());
            // The phase is the other thing the band reads at a glance, so it
            // gets the same tracked-caps stamp the link does — and it is a
            // wire value, so it is drawn in the machine face rather than the
            // human one. `planning` is not a word anyone wrote.
            left = left.child(tracked(&self.session.state, view::LABEL_PT, MAP_INK));
            left = left.child(
                div()
                    .font_family(view::FACE_HUMAN)
                    .text_size(px(view::BODY_PT))
                    .text_color(rgba(CUT_GREY))
                    .child(self.session.game.clone()),
            );
        }
        // The create door, where the console keeps it: among the session
        // readouts, never in a side island — the islands work the HELD
        // session, and a create form there reads as another of its
        // properties.
        left = left.child(sep());
        left = left.child(
            div()
                .id("new-session")
                .cursor_pointer()
                .on_click(cx.listener(|view, _, _, cx| {
                    view.open_only(WhichPanel::Composer);
                    cx.notify();
                }))
                .child(button("+ New session", true, false, false)),
        );
        // The map controls, the console's own pair: zoom out, the readout,
        // zoom in. The round trip is the map thread's, not a local transform
        // — the shell asks and never scales the picture itself.
        left = left.child(sep());
        left = left.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .id("zoom-out")
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| view.zoom_by(-1.0, cx)))
                        .child(button("\u{2212}", true, false, false)),
                )
                // The one figure in the band, and the reason it is a figure
                // and not a machine row: the zoom changes under the
                // operator's hand, so a proportional digit set would slide
                // the `+` beside it every step. Tabular figures hold it.
                .child(figure(&format!("z{:.0}", self.map_zoom), false, MAP_INK))
                .child(
                    div()
                        .id("zoom-in")
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| view.zoom_by(1.0, cx)))
                        .child(button("+", true, false, false)),
                ),
        );

        // The right-hand group, right-to-left like the console's: a flex
        // spacer pushes it to the end whatever the window holds, so a narrow
        // window squeezes the readouts before it ever clips a button.
        //
        // The comms door carries the count, because the console's does and
        // because an inbox with no count is an inbox you have to open to
        // find out whether there is anything in it. It toggles the column's
        // COMMS island — the inbox is something you read beside the map, not
        // a dialog that takes the frame.
        let mail = if self.session.messages.is_empty() {
            "comms".to_string()
        } else {
            format!("comms ({})", self.session.messages.len())
        };
        let right = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .child(
                div()
                    .id("comms-door")
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _, _, cx| {
                        // The console's door: it flips its floating comms
                        // island. This shell has no floating panes, so the
                        // same door opens the same surface in the modal
                        // zone — and a second press shuts it, which is what
                        // a door does.
                        if view.comms_open {
                            view.comms_open = false;
                        } else {
                            view.open_only(WhichPanel::Comms);
                        }
                        cx.notify();
                    }))
                    .child(button(mail, true, false, false)),
            )
            .child(sep())
            .child(
                div()
                    .id("settings-door")
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.open_only(WhichPanel::Settings);
                        cx.notify();
                    }))
                    .child(button("Settings", true, false, false)),
            )
            .child(sep())
            .child(
                div()
                    .id("sign-out")
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _, _, cx| view.sign_out(cx)))
                    .child(button("Sign out", true, false, false)),
            );

        div()
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .right(px(0.0))
            .h(px(34.0))
            .flex()
            .flex_row()
            .items_center()
            .px(px(10.0))
            .bg(rgba(Rgba::black_alpha(120)))
            .child(left)
            .child(div().flex_1())
            .child(right)
            // The bar's own bottom rule rather than a per-side border: the
            // separator is the only edge it needs.
            .child(
                div()
                    .absolute()
                    .bottom(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .h(px(1.0))
                    .bg(rgba(HAIRLINE_SLATE)),
            )
    }

    /// Open exactly one modal, closing the others — the console's `open_only`.
    ///
    /// Enforced in one place rather than trusted to each door, because a rule
    /// every caller must remember is a rule that gets forgotten the moment a
    /// sixth door appears. The column's islands are not modals and are not
    /// touched: they are surfaces the operator reads beside the map, and one
    /// of them closing because a dialog opened would be a surprise with no
    /// upside.
    fn open_only(&mut self, which: WhichPanel) {
        self.fleet_open = which == WhichPanel::Fleet;
        self.player_open = which == WhichPanel::Player;
        self.modal_open = which == WhichPanel::Settings;
        self.composer_open = which == WhichPanel::Composer;
        self.book_open = which == WhichPanel::Book;
        self.comms_open = which == WhichPanel::Comms;
        // The embark modal's visibility IS its host: opening anything else
        // clears the host, and opening Embarked keeps the host the opener
        // just set — so one rule cannot strand a stale host under another
        // modal, and `open_embarked` stays the only way in.
        if which != WhichPanel::Embarked {
            self.embark_host = None;
        }
    }

    /// Open the Embarked modal on one hull — the console's `open_embark`.
    /// The host rides along because a flag would not say whose passengers
    /// the modal reads; `open_only` closes everything else.
    fn open_embarked(&mut self, host: i64) {
        self.embark_host = Some(host);
        self.open_only(WhichPanel::Embarked);
    }

    /// What the column holds this frame: the state's slots, minus the ones
    /// the operator has closed, and all of them if the side zone is hidden.
    ///
    /// ONE answer, read by the render, the wheel's chain and the reminder
    /// strip's reveal. Three copies of "which islands are open" is three
    /// chances for the wheel to chain against a body nobody drew, and the
    /// strip to scroll to an island that is not there.
    /// Whether this seat reads the player column — the console's
    /// `player_station()` VERBATIM: a seat held by a non-judge. The
    /// `TFG_ORGANIZER=1` capture seed lives here, so the column and every
    /// organizer-only verb read one answer rather than two copies of the
    /// rule.
    fn player_station(&self) -> bool {
        if std::env::var("TFG_ORGANIZER").as_deref() == Ok("1") {
            false
        } else {
            self.own_seat().is_some_and(|p| !p.judge)
        }
    }

    fn column_slots(&self) -> Vec<view::IslandSlot> {
        // The phase is the session's, so it is normally not seedable — and
        // that hides whole panels: Review only exists in Planning, so with an
        // executing session held the panel cannot be photographed at all.
        // `TFG_PHASE=planning` forces it. Absent means the session decides,
        // which is the honest default and the only one that should ever ship.
        let phase = match std::env::var("TFG_PHASE").as_deref() {
            Ok("planning") => crate::gamestate::GameState::Planning,
            Ok("preparation") => crate::gamestate::GameState::Preparation,
            Ok("execution") => crate::gamestate::GameState::Execution,
            Ok("closure") => crate::gamestate::GameState::Closure,
            _ => crate::gamestate::GameState::from_wire(&self.session.state),
        };
        // The console's `player_station()` VERBATIM: a seat held by a
        // non-judge. Deliberately NOT also requiring a commanded hull — that
        // is the DASHBOARD's rule, and conflating them would hide the
        // organizer column from a seated player who has no hull yet, which
        // is most of a planning room.
        // `player` is the seat's, not the operator's choice, so it is normally
        // not seedable — and that is the problem: a seated non-judge gets the
        // player column (Step only), which means the Command and Crew panels
        // cannot be photographed at all in this environment. `TFG_ORGANIZER=1`
        // forces the organizer column so a capture can reach them. Absent
        // means the real seat decides, which is the honest default.
        let player = self.player_station();
        let mut slots = view::zone_islands(phase, player);
        // The operator's own choice, applied to the state's slots. The flags
        // are the panel-open flags the shell already carried, so a capture
        // seed and a test still name the same thing.
        slots.retain(|s| self.island_open(s.key));
        // The whole column is gated on the Settings panel's "Side zone", and
        // clearing here hides the islands from render AND ownership AND the
        // wheel's chain at once — which is why the gate is a filter and not a
        // paint condition.
        if !self.show_side_zone {
            slots.clear();
        }
        slots
    }

    /// Scroll the column so an island is on the band.
    ///
    /// Its top to the band's top; when the island is taller than the band
    /// there is no such position, so its BOTTOM comes to the band's bottom —
    /// the half you can still read. A reminder strip that opened an island
    /// off the bottom of the column would be a button that appears to do
    /// nothing, which is the exact failure the strip exists to avoid.
    fn reveal_island(&mut self, key: view::IslandKey, viewport: view::Box) {
        let band = zone_band_at(viewport, self.zone_w, self.side_dock_left);
        let slots = self.column_slots();
        let Some(i) = slots.iter().position(|s| s.key == key) else {
            return;
        };
        let (_, y) = view::zone_slot_origin(band, &slots, i);
        let h = slots[i].height;
        let want = if h > view::box_h(band) {
            y + h - band.3
        } else {
            y - band.1
        };
        let ceiling = view::zone_overflow(view::zone_stack_height(&slots), band);
        self.zone_scroll = want.clamp(0.0, ceiling);
    }

    /// Open or shut one island of the column.
    ///
    /// A toggle rather than an open, because the band's door is a switch: the
    /// console's roster and comms doors both flip their surface, and a door
    /// that only opens leaves the operator hunting for the close. Closing an
    /// island that is not in the current state's column is still recorded —
    /// the flag is the operator's, and it must be there when the state that
    /// shows the island comes round again.
    fn toggle_island(&mut self, key: view::IslandKey, cx: &mut Context<Self>) {
        let want = !self.island_open(key);
        self.set_island_open(key, want);
        cx.notify();
    }

    /// Whether an island can be closed at all.
    ///
    /// ONE answer, from the same source as `island_open`: the islands that
    /// have a flag behind them. The band's close cross is drawn from this,
    /// and that is the whole reason it exists -- a cross on an island that
    /// ignores the press is a control that reads as clickable and is not,
    /// which is worse than no control, because the operator cannot tell
    /// "it did not work" from "it is not there".
    ///
    /// The console closes ANY island (`chrome::island_owned` sets `*open =
    /// false` and the spec persists it), so this is a deliberate divergence
    /// and not a port: the seven that stay open are the ones whose absence
    /// would leave the column unable to answer the question its state exists
    /// to answer.
    fn island_closeable(key: view::IslandKey) -> bool {
        matches!(
            key,
            view::IslandKey::Players
                | view::IslandKey::Crew
                | view::IslandKey::Command
                | view::IslandKey::Step
        )
    }

    /// Whether the operator has this island open.
    ///
    /// ONE answer. The four `*_open` flags are the source and the column's
    /// close cross and the band's door write them; there is deliberately no
    /// second copy inside `IslandState`, because two places holding "is this
    /// panel shown" is two answers waiting to disagree — and the one that
    /// loses is always the one the operator last touched.
    fn island_open(&self, key: view::IslandKey) -> bool {
        match key {
            view::IslandKey::Players => self.people_open,
            view::IslandKey::Crew => self.crew_open,
            view::IslandKey::Command => self.command_open,
            view::IslandKey::Step => self.step_open,
            // Identity, the register and the picker are the column's floor,
            // present in every state they appear in: a way to close them
            // would be a way to be left with an empty console and no
            // sign-out — and closing the picker would strand the one verb
            // that holds a game at all.
            view::IslandKey::Fleet | view::IslandKey::Operator | view::IslandKey::Sessions => true,
            // The plan's own terms, its clock and its gate. Closing them
            // would leave the column unable to say what is being planned or
            // whether it may start — the two questions planning exists to
            // answer.
            view::IslandKey::Essentials
            | view::IslandKey::Control
            | view::IslandKey::Review => true,
            // The evaluation surfaces: the console presents both
            // unconditionally, and neither is a question the column exists to
            // answer — a closed feed is still a feed, and a closed debrief
            // is still a debrief.
            view::IslandKey::Log | view::IslandKey::Assessment => true,
        }
    }

    fn set_island_open(&mut self, key: view::IslandKey, open: bool) {
        match key {
            view::IslandKey::Players => self.people_open = open,
            view::IslandKey::Crew => self.crew_open = open,
            view::IslandKey::Command => self.command_open = open,
            view::IslandKey::Step => self.step_open = open,
            view::IslandKey::Fleet
            | view::IslandKey::Operator
            | view::IslandKey::Sessions
            | view::IslandKey::Essentials
            | view::IslandKey::Control
            | view::IslandKey::Review
            | view::IslandKey::Log
            | view::IslandKey::Assessment => {}
        }
    }

    /// Which body an island of the column carries.
    ///
    /// One surface, one place: the island's key names the panel, and the
    /// panel's body IS the island's body. The prototype's alternative — each
    /// surface reachable from a button row and rendered as a centred modal —
    /// is what made the chrome read as scaffolding: ten verbs, no layout.
    fn island_body(&mut self, cx: &mut Context<Self>, key: view::IslandKey) -> AnyElement {
        match key {
            view::IslandKey::Operator => self.operator_body(cx),
            view::IslandKey::Fleet => self.fleet_body(cx),
            view::IslandKey::Essentials => self.essentials_body(cx),
            view::IslandKey::Control => self.control_body(cx),
            view::IslandKey::Players => self.players_body(cx),
            view::IslandKey::Crew => self.crew_body(cx),
            view::IslandKey::Review => self.review_body(cx),
            view::IslandKey::Command => self.command_body(cx),
            view::IslandKey::Step => self.step_body(cx),
            view::IslandKey::Log => self.log_body(cx),
            view::IslandKey::Assessment => self.assessment_body(cx),
            view::IslandKey::Sessions => self.sessions_body(cx),
        }
    }

    /// An island's trailing note: the band's own count about itself.
    ///
    /// The console's `with_trailing` on every zone island, and the reason a
    /// band reads as an instrument rather than a label — `12 SEATED`, `127
    /// HULLS`. Empty is a real answer and is drawn empty: an island whose
    /// count is not knowable must not invent one.
    fn island_trailing(&self, key: view::IslandKey) -> String {
        match key {
            // The role this machine holds in the held game, or the state's
            // name when it holds no seat — identity is the one thing the
            // console puts on every band.
            view::IslandKey::Operator => {
                if self.session.my_role.is_empty() {
                    crate::gamestate::GameState::from_wire(&self.session.state).label().to_string()
                } else {
                    self.session.my_role.clone()
                }
            }
            // The console counts the DRAFT here (`N PIECES`), not the hulls
            // in the world: the register is what the picker browses, and an
            // island whose note counts something it does not show is a note
            // about a different island.
            view::IslandKey::Fleet => format!("{} PIECES", self.session.pieces.len()),
            // The console's `N SEATED`, which the read already carries.
            view::IslandKey::Players => format!("{} SEATED", self.session.seats),
            view::IslandKey::Crew => format!("{} IN ROOM", self.session.present),
            // The plan's terms: the state is already the band's job on
            // Operator, so this says which PHASE of the plan it is.
            view::IslandKey::Essentials => crate::gamestate::GameState::from_wire(
                &self.session.state,
            )
            .label()
            .to_string(),
            view::IslandKey::Control => format!("{:.0}\u{00d7}", self.session.facts.time_factor),
            view::IslandKey::Command => {
                if self.session.mine.is_empty() {
                    "NO HULL".to_string()
                } else {
                    format!("{} HULL", self.session.mine.len())
                }
            }
            // The step's own state, as the console's `z.mystep` spells it:
            // what this seat owes, not what the book holds.
            // The gate, as the server words it: the count when it published a
            // checklist, and the honest absence when it did not.
            view::IslandKey::Review => match &self.session.readiness {
                None => "\u{2014}".to_string(),
                Some(r) if r.blockers.is_empty() => "0 OUTSTANDING".to_string(),
                Some(r) => format!("{} OUTSTANDING", r.blockers.len()),
            },
            view::IslandKey::Step => match self.step_reminder() {
                Some(rem) if rem.needs_you => "ASKING".to_string(),
                Some(_) => "PLAYING".to_string(),
                None => "\u{2014}".to_string(),
            },
            // The picker counts what it shows, like the register counts
            // its draft: the games this account can see.
            view::IslandKey::Sessions => format!("{} SESSIONS", self.sessions.len()),
            // Neither island carries a count in the console either —
            // `side_zone` builds both without a `with_trailing` — and a
            // band note that the surface it belongs to never had is an
            // invention, not a port. Empty is a real answer.
            view::IslandKey::Log | view::IslandKey::Assessment => "\u{2014}".to_string(),
        }
    }

    /// The Operator island: identity, and the way out.
    ///
    /// First in every state, because identity is the one thing that does not
    /// change with the exercise (`main.rs::side_zone`'s own comment). The
    /// console's `z.user` carries the avatar, the account and the sign-out;
    /// this carries what the shell can source — the seat's call sign when the
    /// held game names one, the identifier that signed in when it does not —
    /// and says which of the two it is rather than blending them.
    fn operator_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rule = || div().h(px(1.0)).w_full().bg(rgba(HAIRLINE_SLATE));
        let seat = self.own_seat().cloned();
        let who = seat
            .as_ref()
            .map(|p| {
                if p.call_sign.trim().is_empty() {
                    p.name.clone()
                } else {
                    p.call_sign.clone()
                }
            })
            .unwrap_or_else(|| self.login_identifier.clone());
        let shown = if who.trim().is_empty() {
            "no account named".to_string()
        } else {
            who
        };
        let role = if self.session.my_role.is_empty() {
            "no seat in the held game".to_string()
        } else {
            self.session.my_role.clone()
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        col = col.child(
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(TEXT_BODY_PT))
                .text_color(rgba(MAP_INK))
                .child(shown),
        );
        col = col.child(
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(TEXT_SMALL_PT))
                .text_color(rgba(BODY_SILVER))
                .child(role),
        );
        col = col.child(rule());
        // Sign-out lives here as well as on the band, which is the console's
        // own arrangement: the band's button is the reach, the island's is
        // the confirmation you were looking at the right account.
        col = col.child(
            div()
                .id("operator-sign-out")
                .cursor_pointer()
                .on_click(cx.listener(|view, _, _, cx| view.sign_out(cx)))
                .child(button("Sign out", true, false, false)),
        );
        col.into_any_element()
    }

    /// ESSENTIALS — the held plan, read back, in the console's own order.
    ///
    /// The console's `z.essentials` is `setup_game_ui`: the session picker,
    /// the edit and delete writes, `held_plan_prose`, and the room-key join.
    /// This carries the half that is a READ, because the writes it leaves out
    /// are the console's session-admin surface and the shell has no picker to
    /// browse with: what is being planned, the server's own words for its
    /// mode and pace, the window it is planned for, and the Game Master's
    /// three prose fields — each shown only when written, because six blank
    /// labels read as a broken panel rather than as a plan nobody has written
    /// down yet (`held_plan_prose`'s own reason).
    fn essentials_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let small = |t: String| {
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        if self.session.game_id == 0 {
            return col
                .child(small("No session held \u{2014} the band's door creates one.".into()))
                .into_any_element();
        }
        // A read this account may not make and a plan nobody has written
        // down are different things, and the difference is the whole reason
        // to say which one is on screen.
        if !self.session.facts.read {
            return col
                .child(small(
                    "This account cannot read the session's terms \u{2014} ask its Game Master."
                        .into(),
                ))
                .into_any_element();
        }
        let f = self.session.facts.clone();
        let state = crate::gamestate::GameState::from_wire(&self.session.state);
        col = col.child(
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(TEXT_BODY_PT))
                .text_color(rgba(MAP_INK))
                .child(if self.session.game.is_empty() {
                    "unnamed session".to_string()
                } else {
                    self.session.game.clone()
                }),
        );
        col = col.child(small(format!(
            "{} \u{00b7} {} \u{00b7} {}",
            if f.mode.is_empty() { "no mode" } else { &f.mode },
            state.label().to_lowercase(),
            format!("{} pace", f.pace)
        )));
        col = col.child(separator());
        // The window. Both ends or neither is the console's rule: an
        // incomplete window is the ordinary state of a draft AND the reason
        // the execution transition is refused, so it says so rather than
        // printing an empty row.
        let window = view::window_label(
            &f.actual_start,
            &f.actual_end,
            &f.assumed_start,
            &f.assumed_end,
        );
        col = col.child(small("PLANNED WINDOW".to_string()));
        col = col.child(
            div()
                .font_family(view::FACE_MACHINE)
                .text_size(px(11.0))
                .text_color(rgba(if window.1 { MAP_INK } else { ALERT_YELLOW }))
                .child(window.0),
        );
        col = col.child(separator());
        col = col.child(small("THE PLAN".to_string()));
        let mut said_any = false;
        for (label, text) in [
            ("purpose", &f.purpose),
            ("target", &f.target),
            ("description", &f.description),
        ] {
            if text.is_empty() {
                continue;
            }
            said_any = true;
            col = col.child(small(label.to_string()));
            col = col.child(
                div()
                    .font_family(view::FACE_HUMAN)
                    .text_size(px(11.0))
                    .text_color(rgba(MAP_INK))
                    .child(text.clone()),
            );
        }
        if !said_any {
            col = col.child(small("nothing written yet".to_string()));
        }
        // The scenario composer's door, on the island that already owns the
        // held session's own facts — the console's own placement, verbatim
        // (`main.rs::setup_session_ui`). The third door of the same shape as
        // Fleet's and Players': the island is the summary, the modal is where
        // the decision gets room.
        //
        // ABSENT outside planning rather than greyed out: authoring is a
        // planning-time act and the server refuses it after, so a control
        // that would 403 is a lie about what this session can do.
        if crate::gamestate::GameState::from_wire(&self.session.state)
            == crate::gamestate::GameState::Planning
        {
            col = col.child(separator());
            let scenarios = self.session.book.len();
            let steps: usize = self.session.book.iter().map(|s| s.steps.len()).sum();
            col = col.child(
                div()
                    .id("scenario-composer-door")
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.open_only(WhichPanel::Book);
                        cx.notify();
                    }))
                    .child(button("Scenario composer".to_string(), false, false, false)),
            );
            if scenarios > 0 {
                col = col.child(small(format!(
                    "{scenarios} scenario(s), {steps} step(s)."
                )));
            }
            // LEAVE PLANNING. The staged force and roster are written through
            // whole here and the game moves to preparation — the one control
            // that turns a planning draft into a record. It rides the Session
            // island because the island is the held session itself, and the
            // move is the session's, not the fleet's or the room's.
            //
            // ABSENT outside planning, like the door above: past planning the
            // edits already posted and there is nothing staged to write, so a
            // control here would either do nothing or repeat a transition the
            // server refuses.
            col = col.child(separator());
            let pending = self.force.pending(&self.live_hulls())
                + self.roster_draft.pending(&self.live_seats());
            col = col.child(
                div()
                    .id("advance-preparation")
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.prepare_advance(cx);
                    }))
                    .child(button(
                        "Move to preparation".to_string(),
                        false,
                        self.prepare_busy,
                        false,
                    )),
            );
            col = col.child(small(if pending == 0 {
                "nothing staged \u{2014} this just moves the session on".to_string()
            } else {
                format!("{pending} staged write(s) will be sent")
            }));
            if !self.prepare_status.is_empty() {
                col = col.child(small(self.prepare_status.clone()));
            }
        }
        col.into_any_element()
    }

    /// CONTROL — the game clock's multiplier and its hold, and the verbs
    /// that write them.
    ///
    /// The console's `z.control`, whole: the readout of the pace Minos is
    /// running, the draft the operator adjusts, and Apply — plus the hold,
    /// which the console works from its `pause_or_resume_minos` toggle. The
    /// draft is a step row rather than a text field because that is the
    /// shell's number idiom (the helm's course and speed), and a console
    /// readout should be adjusted, not typed at. There is no elapsed
    /// readout: the tick needs the Minos clock channel, and the writes
    /// below ARE the read — the pill answers whether the exercise runs.
    fn control_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let small = |t: String| {
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        if self.session.game_id == 0 {
            return col
                .child(small("No session held.".into()))
                .into_any_element();
        }
        let read = self.session.facts.time_factor;
        if self.factor_draft.is_none() {
            self.factor_draft = Some(read);
        }
        let draft = self.factor_draft.unwrap_or(read);
        col = col.child(small("GAME TIME MULTIPLIER".to_string()));
        // Two buttons, one draft. The rail is the kernel's (`FACTOR_MIN`,
        // `FACTOR_MAX` — the console's own 1..=144), so the row cannot draft a
        // value the server would refuse, and a button at the rail is a no-op
        // rather than a clamp the operator has to undo.
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .id("factor-down")
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.nudge_factor(-1.0);
                            cx.notify();
                        }))
                        .child(button("\u{2212}", true, draft <= FACTOR_MIN, false)),
                )
                .child(
                    div()
                        .w(px(64.0))
                        .font_family(view::FACE_MACHINE)
                        .text_size(px(11.0))
                        .text_color(rgba(MAP_INK))
                        .child(format!("{draft:.0}\u{00d7}")),
                )
                .child(
                    div()
                        .id("factor-up")
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.nudge_factor(1.0);
                            cx.notify();
                        }))
                        .child(button("+", true, draft >= FACTOR_MAX, false)),
                ),
        );
        col = col.child(small(format!("Pace {read:.0}\u{00d7} \u{2014} read from Minos.")));
        let dirty = (draft - read).abs() > f32::EPSILON as f64;
        col = col.child(
            div()
                .id("factor-apply")
                .cursor_pointer()
                .on_click(cx.listener(|view, _, _, cx| {
                    view.send_factor(cx);
                }))
                .child(button("Apply", false, !dirty, false)),
        );
        if !self.factor_status.is_empty() {
            col = col.child(small(self.factor_status.clone()));
        }
        // The hold, in the console's words: held resumes, otherwise pauses,
        // unknown pauses. The readout is the last answer's `running` — the
        // only clock read this shell has — and the pill asks the other one.
        col = col.child(small("SCENARIO CLOCK".to_string()));
        if let Some(running) = self.clock_running {
            col = col.child(small(if running {
                "RUNNING".to_string()
            } else {
                "HELD".to_string()
            }));
        }
        let pill = match self.clock_running {
            Some(false) => "Resume",
            _ => "Pause",
        };
        col = col.child(
            div()
                .id("clock-pause")
                .cursor_pointer()
                .on_click(cx.listener(|view, _, _, cx| {
                    view.send_set_paused(cx);
                }))
                .child(button(pill, false, false, false)),
        );
        if !self.pause_status.is_empty() {
            col = col.child(small(self.pause_status.clone()));
        }
        col.into_any_element()
    }

    /// FLEET — what fights, and the door to the register it is chosen from.
    ///
    /// The console's `z.fleet`, and the correction of a real divergence: this
    /// island used to list the whole 127-hull REGISTER, which is the console's
    /// *picker modal* content rather than anything its Fleet island draws. The
    /// console's Fleet is the DRAFT — the pieces chosen for this exercise —
    /// with a door onto the register. The trailing note follows: the console
    /// counts `N PIECES`, not hulls in the world.
    ///
    /// The pieces here are the server's own (`GET /games/{id}/units`) — the
    /// same list the console's draft is synced from, so the two agree about
    /// WHAT is in the exercise. The door above opens the register to change it
    /// (`fleet_picker_body`), which posts through `force::plan` at once. What
    /// is still not authored here is a starting POSITION: the console places a
    /// hull by dropping it on the map, and this shell has no map drop, so a
    /// piece arrives unplaced and the pip says so.
    /// The Player picker: the directory accounts are seated from, and the room.
    ///
    /// The console's `player_picker_modal`. The directory is the read; the room
    /// is what the operator has said about it, and every edit posts through
    /// `roster::plan` at once — so, exactly as with the force, the draft is a
    /// mirror of what Minos holds and a session read can re-seed it without
    /// discarding work.
    ///
    /// What this does NOT do is ask for a call sign. Minos requires one on a
    /// seat and the console's form asks the operator for it; here the account's
    /// own name is used, upper-cased and cut to eight, because a seat form that
    /// silently invented one would be a call sign nobody chose — and the panel
    /// says which it used rather than presenting the choice.
    fn player_picker_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let small = |t: String| {
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .size_full()
            .overflow_hidden()
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .h(px(TITLE_H))
                .child(
                    div()
                        .font_family(view::FACE_HUMAN)
                        .text_size(px(TITLE_SIZE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgba(MAP_INK))
                        .child("Player picker"),
                )
                .child(
                    div()
                        .id("close-player-picker")
                        .cursor_pointer()
                        .text_size(px(14.0))
                        .text_color(rgba(BUTTON_LABEL))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.open_only(WhichPanel::None);
                            cx.notify();
                        }))
                        .child("\u{00d7}"),
                ),
        );
        col = col.child(separator());
        // One scrolling list under a fixed header. Eight accounts and five
        // roles do not fit a 420-point panel, and a clipped directory is a
        // directory that cannot be read: the scroller is what makes the panel
        // honest about the size of the answer rather than the size of the box.
        // `.id()` first: `overflow_y_scroll` is on `StatefulInteractiveElement`,
        // so the scroller has to be a stateful element — a plain `Div` has no
        // such method and cannot be one.
        let mut list = div()
            .id("player-picker-list")
            .flex()
            .flex_col()
            .gap(px(4.0))
            .flex_1()
            .overflow_y_scroll();
        list = list.child(small(format!(
            "IN THE ROOM \u{2014} {}",
            self.session.people.len()
        )));
        if self.session.roles.is_empty() {
            list = list.child(small(
                "This account cannot read the session's roles \u{2014} ask its Game Master."
                    .to_string(),
            ));
        }
        if self.session.people.is_empty() {
            list = list.child(small("Nobody seated.".to_string()));
        }
        for p in self.session.people.clone() {
            let mut row = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(4.0))
                .py(px(3.0))
                .rounded(px(3.0));
            row = row.child(
                div()
                    .flex_1()
                    .text_size(px(11.0))
                    .text_color(rgba(MAP_INK))
                    .child(p.name.clone()),
            );
            row = row.child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgba(if p.judge { BODY_SILVER } else { RADAR_CYAN }))
                    .child(if p.judge {
                        format!("{} \u{00b7} directs", p.role)
                    } else {
                        format!("{} \u{00b7} steers", p.role)
                    }),
            );
            row = row.child(
                div()
                    .id(("unseat", p.id as usize))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        match view.stage_unseat(p.id) {
                            Ok(()) => view.roster_status = format!("{} unseated", p.name),
                            Err(why) => view.roster_status = why,
                        }
                        cx.notify();
                    }))
                    .child(button("unseat", true, false, false)),
            );
            list = list.child(row);
        }
        list = list.child(separator());
        list = list.child(small("THE DIRECTORY".to_string()));
        if !self.roster_status.is_empty() {
            list = list.child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgba(if self.roster_busy { BODY_SILVER } else { ALERT_YELLOW }))
                    .child(self.roster_status.clone()),
            );
        }
        if self.session.directory.is_empty() {
            list = list.child(small("The directory did not answer.".to_string()));
        }
        let seated: Vec<i64> = self.session.people.iter().map(|p| p.id).collect();
        let roles = self.session.roles.clone();
        for (i, u) in self.session.directory.iter().take(20).cloned().collect::<Vec<_>>().into_iter().enumerate() {
            let taken = seated.contains(&u.id);
            // Only the parts the mirror actually carries. A blank rank joined
            // with a separator reads as a row that lost its middle, which is a
            // worse answer than a row with one line in it.
            let detail = [u.rank.as_str(), u.unit.as_str(), u.appointment.as_str()]
                .iter()
                .filter(|part| !part.trim().is_empty())
                .map(|part| part.trim().to_string())
                .collect::<Vec<_>>()
                .join(" \u{00b7} ");
            let label = if detail.is_empty() {
                u.name.clone()
            } else {
                format!("{} \u{00b7} {detail}", u.name)
            };
            // An account is TWO lines, not one. The name and its rank can be
            // long, and a game can carry five roles, so a single row loses
            // whichever end it ran out of: the name would clip, or the roles
            // would slide under the panel's edge. Stacked, both survive a name
            // of any length and a role list of any size.
            let mut row = div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .px(px(4.0))
                .py(px(3.0))
                .rounded(px(3.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(rgba(MAP_INK))
                        .child(label),
                );
            // One button per role. The FIRST is the sensible seat — an account
            // picked here is almost always being seated to steer — and the rest
            // are there because which seat somebody takes is the whole of the
            // decision, and a picker that only offered one answer would be a
            // picker that made it for you.
            let mut seats = div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(6.0));
            for role in roles.iter().cloned() {
                let name = u.name.clone();
                let role = role.clone();
                let shown = role.name.clone();
                seats = seats.child(
                    div()
                        .id(("seat", u.id as usize * 1000 + role.id as usize))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            match view.stage_seat(u.id, &name, &role) {
                                Ok(()) => view.roster_status = format!("{name} seated"),
                                Err(why) => view.roster_status = why,
                            }
                            cx.notify();
                        }))
                        .child(button(shown, true, taken, taken)),
                );
            }
            row = row.child(seats);
            list = list.child(row);
        }
        list = list.child(small(format!(
            "{} of {} accounts",
            self.session.directory.len().min(20),
            self.session.directory.len()
        )));
        col.child(list).into_any_element()
    }

    /// The Fleet picker: the register it is chosen from, and the force being
    /// chosen.
    ///
    /// The drill's effective rungs: the selection, or the first tab/row when
    /// untouched, so an unseeded open still shows a full column set. The
    /// lookups are id-keyed, never positional — a re-sync that reorders rows
    /// must not move the operator's drill.
    fn drill_group_id(&self) -> Option<i64> {
        self.drill_group
            .or_else(|| self.fleet_tax.groups.first().map(|g| g.id))
    }

    fn drill_cats(&self) -> &[FleetTaxCat] {
        let Some(id) = self.drill_group_id() else {
            return &[];
        };
        self.fleet_tax
            .groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| g.cats.as_slice())
            .unwrap_or(&[])
    }

    fn drill_types(&self) -> &[FleetTaxType] {
        let Some(id) = self.drill_category else {
            return &[];
        };
        self.drill_cats()
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.types.as_slice())
            .unwrap_or(&[])
    }

    fn drill_classes(&self) -> &[FleetTaxClass] {
        let Some(id) = self.drill_type else {
            return &[];
        };
        self.drill_types()
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.classes.as_slice())
            .unwrap_or(&[])
    }

    fn drill_units(&self) -> &[FleetTaxLeaf] {
        let Some(id) = self.drill_class else {
            return &[];
        };
        self.drill_classes()
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.units.as_slice())
            .unwrap_or(&[])
    }

    /// Drill moves, one rung each. A parent change clears everything below
    /// it — the console's `drill_*` rule: a category from another group with
    /// a stale class under it would show rows the columns disown.
    fn select_group(&mut self, id: i64) {
        self.drill_group = Some(id);
        self.drill_category = None;
        self.drill_type = None;
        self.drill_class = None;
    }

    fn select_category(&mut self, id: i64) {
        self.drill_category = Some(id);
        self.drill_type = None;
        self.drill_class = None;
    }

    fn select_type(&mut self, id: i64) {
        self.drill_type = Some(id);
        self.drill_class = None;
    }

    fn select_class(&mut self, id: i64) {
        self.drill_class = Some(id);
    }

    /// The console's `fleet_picker_modal` — Miller columns over the register,
    /// Asset Group > Category > Type > Class > hull, with the glyph thumbnail
    /// on every leaf. This is the read-and-stage half: the register's rows
    /// and the draft's rows, with the commander each hull is handed to,
    /// because that is the one thing Minos requires of a piece and the one
    /// thing the operator has to say. Dragging a leaf lifts the hull: a
    /// release over bare map deploys it there, a release on a placed host's
    /// marker embarks it aboard — through `DeployUnit` and `EmbarkDrop`,
    /// the same verbs click-to-place and the embark buttons use.
    ///
    /// Every edit posts at once — `stage_assign` plans the whole draft against
    /// the server and sends it — so what the picker shows is never ahead of
    /// what Minos holds. That is also why the draft can be re-seeded from a
    /// session read without discarding anybody's work.
    fn fleet_picker_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let small = |t: String| {
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .size_full()
            .overflow_hidden()
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .h(px(TITLE_H))
                .child(
                    div()
                        .font_family(view::FACE_HUMAN)
                        .text_size(px(TITLE_SIZE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgba(MAP_INK))
                        .child("Fleet picker"),
                )
                .child(
                    div()
                        .id("close-fleet-picker")
                        .cursor_pointer()
                        .text_size(px(14.0))
                        .text_color(rgba(BUTTON_LABEL))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.open_only(WhichPanel::None);
                            cx.notify();
                        }))
                        .child("\u{00d7}"),
                ),
        );
        col = col.child(separator());
        // The force first: it is what the operator came to change, and the
        // register is the means. Content height, not a scroll: in drill mode
        // the miller columns below own the scroll; in flat mode this whole
        // block rides at the top of the scrolling list.
        let mut force = div().flex().flex_col().gap(px(4.0));
        force = force.child(small(format!(
            "IN THE EXERCISE \u{2014} {}",
            self.session.pieces.len()
        )));
        let crew = self.crew_choices();
        // The pills show when the operator holds no PLAYER-side seat: the
        // room is empty, or the operator's own seat is judge-side — which
        // cannot command, so every drop would be refused. Both are a
        // one-click fix here, in the picker the drop started from.
        let needs_player_seat = crew.is_empty() || self.own_steers() == Some(false);
        if needs_player_seat {
            force = force.child(small(
                if crew.is_empty() {
                    "Nobody is seated, and Minos gives every piece a commander \u{2014} \
                     seat the room on the Players island first."
                } else {
                    "Your seat is judge-side \u{2014} it directs, it does not \
                     command. Take a player seat to deploy."
                }
                .to_string(),
            ));
            // The one-click way in: the operator's own seat, as a
            // player-side role. A fresh game seats nobody — not even its
            // creator, the server's rule, not ours — and a drop refused for
            // no seat with no way forward is a wall. These pills stage
            // exactly what the Players island would (same `stage_seat`, same
            // call-sign rule), for the one choice nobody has to make: which
            // seat is theirs. Player-side roles only — the server refuses a
            // judge-side commander outright, so an organizer pill would seat
            // you beautifully and leave every drop refused.
            if self.session.me != 0 {
                for role in self.player_roles() {
                    let me = self.session.me;
                    let name = self.own_name();
                    let role_name = role.name.clone();
                    let label = format!("seat me as {role_name}");
                    force = force.child(
                        div()
                            .id(("seat-me", role.id as usize))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                match view.stage_seat(me, &name, &role) {
                                    Ok(()) => {
                                        view.force_status = format!(
                                            "seated as {role_name} \u{2014} drag away"
                                        )
                                    }
                                    Err(why) => view.force_status = why,
                                }
                                cx.notify();
                            }))
                            .child(button(label, true, false, false)),
                    );
                }
            }
        }
        if self.session.pieces.is_empty() {
            force = force.child(small("No pieces yet.".to_string()));
        }
        for p in self.session.pieces.clone() {
            let p_label = p.label.clone();
            let mut row = div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .px(px(4.0))
                .py(px(3.0))
                .rounded(px(3.0))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.0))
                        .text_color(rgba(MAP_INK))
                        .child(p.label.clone()),
                );
            row = row.child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgba(if p.placed { BODY_SILVER } else { ALERT_YELLOW }))
                    .child(if p.placed { "placed" } else { "unplaced" }),
            );
            let commander = crew
                .iter()
                .find(|(id, _)| *id == p.commander_id.unwrap_or(-1))
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "unassigned".to_string());
            row = row.child(
                div()
                    .text_size(px(10.5))
                    .text_color(rgba(BODY_SILVER))
                    .child(commander),
            );
            // Who holds it, as a CHOICE rather than a label. A hull's
            // commander is the one thing Minos requires of a piece and the one
            // thing an operator will change most often, so the row carries the
            // room's seats as buttons and a click hands it over —
            // `set_unit_commander`, through the same plan.
            for (who, who_name) in crew.iter().cloned() {
                let held = p.commander_id == Some(who);
                let p_label = p_label.clone();
                let shown = who_name.clone();
                row = row.child(
                    div()
                        .id(("piece-handover", p.id as usize * 1000 + who as usize))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            match view.stage_handover(p.id, who) {
                                Ok(()) => {
                                    view.force_status = format!("{p_label} handed to {who_name}")
                                }
                                Err(why) => view.force_status = why,
                            }
                            cx.notify();
                        }))
                        .child(button(shown, true, held, held)),
                );
            }
            row = row.child(
                div()
                    .id(("piece-remove", p.id as usize))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        match view.stage_remove(p.id) {
                            Ok(()) => view.force_status = "piece removed".to_string(),
                            Err(why) => view.force_status = why,
                        }
                        cx.notify();
                    }))
                    .child(button("remove", true, false, false)),
            );
            force = force.child(row);
        }
        if self.fleet_tax.groups.is_empty() {
            // No taxonomy (fixture runs, offline machines): the flat
            // register list, capped — the picker's old shape, kept as the
            // fallback rather than showing an empty drill.
            let mut list = div()
                .id("fleet-picker-list")
                .flex()
                .flex_col()
                .gap(px(4.0))
                .flex_1()
                .overflow_y_scroll();
            list = list.child(force);
            list = list.child(separator());
            list = list.child(small("THE REGISTER".to_string()));
            if !self.force_status.is_empty() {
                list = list.child(
                    div()
                        .text_size(px(10.5))
                        .text_color(rgba(if self.force_busy { BODY_SILVER } else { ALERT_YELLOW }))
                        .child(self.force_status.clone()),
                );
            }
        // One register row per hull, capped: the picker is a card, and the
        // island's own count says how many exist beyond it.
        let in_draft: Vec<i64> = self.session.pieces.iter().map(|p| p.id).collect();
        for (i, row) in self.roster.iter().take(24).cloned().collect::<Vec<_>>().into_iter().enumerate() {
            let unit_id = row.unit_id.parse::<i64>().unwrap_or(-1);
            let taken = in_draft.contains(&unit_id);
            let name = row.name.clone();
            let hull = row.hull.clone();
            list = list.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(4.0))
                    .py(px(3.0))
                    .rounded(px(3.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.0))
                            .text_color(rgba(MAP_INK))
                            .child(format!("{name} \u{00b7} {hull}")),
                    )
                    // The commander this hull will be handed to. `session.me`
                    // is the sensible default — the operator picking a hull is
                    // almost always taking it themselves — and the choice is
                    // still shown, because a silent default is a hull handed to
                    // whoever happened to open the panel.
                    .child(
                        div()
                            .id(("piece-add", i))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                let who = view.session.me;
                                if view.crew_choices().iter().all(|(id, _)| *id != who) {
                                    view.force_status =
                                        "your account holds no seat in this room".to_string();
                                    cx.notify();
                                    return;
                                }
                                match view.stage_assign(unit_id, &name, who) {
                                    Ok(()) => {
                                        view.force_status = "piece assigned".to_string()
                                    }
                                    Err(why) => view.force_status = why,
                                }
                                cx.notify();
                            }))
                            .child(button(if taken { "assigned" } else { "add" }, true, taken, false)),
                    ),
            );
        }
        list = list.child(small(format!(
            "{} of {} hulls",
            self.roster.len().min(24),
            self.roster_total
        )));
            col.child(list).into_any_element()
        } else {
            // The drill: asset-group tabs, a breadcrumb, and four columns —
            // Category | Type | Class | Hull — the egui picker's shape. The
            // columns row owns the scroll (each column scrolls alone), so the
            // force above stays pinned while the register drills.
            col.child(force)
                .child(separator())
                .child(small("THE REGISTER".to_string()))
                .child(self.drill_status_line())
                .child(self.drill_tabs(cx))
                .child(self.drill_crumbs(cx))
                .child(self.drill_columns(cx))
                .into_any_element()
        }
    }

    /// A hull's face: its photograph when the fetcher has landed one, the
    /// glyph otherwise — the same face on the row and the ghost, so the
    /// hand never swaps pictures. A rounded rect at the photo's own aspect
    /// (covered, never letterboxed), never a cropped circle: ship photos
    /// are wide, and a disc throws half the hull away before it even gets
    /// small.
    fn thumb_face(thumb: Option<Arc<gpui::RenderImage>>, symbol: MapSymbolShape) -> AnyElement {
        // 56×38 at ~3:2, the shape of the archive's own photos; fetched
        // at 192px so the shrink stays ahead of the paint. Cover, not
        // contain: the ghost is a cursor stamp, and a slice of the photo
        // reads better at 56pt than a letterboxed sliver.
        Self::thumb_box(thumb, symbol, 56.0, 38.0, 1.0, ObjectFit::Cover)
    }

    /// The row's thumbnail as a full-width banner: the whole photograph,
    /// never a cropped slice — the archive's photos are wide whole-body
    /// panoramas (5.9:1 to 9.4:1), and a cover-cropped strip showed a
    /// zoomed-in slice of the hull instead of the ship. The height
    /// follows the photo (width over aspect), capped at `BANNER_MAX_H`.
    fn thumb_banner(thumb: Option<Arc<gpui::RenderImage>>, symbol: MapSymbolShape) -> AnyElement {
        let h = match &thumb {
            Some(photo) => {
                let size = photo.size(0);
                let aspect = size.width.0 as f32 / size.height.0 as f32;
                (LEAF_CONTENT_W / aspect).min(BANNER_MAX_H)
            }
            // The placeholder waits at the first banner's height: the
            // common panoramas land near it and barely reflow.
            None => 48.0,
        };
        Self::thumb_box(thumb, symbol, LEAF_CONTENT_W, h, 1.0, ObjectFit::Contain)
    }

    /// The shared face: a rounded photograph when the fetcher has
    /// landed one, the glyph centered on a slate band otherwise. The
    /// width is always fixed — never a percentage (see `LEAF_CONTENT_W`).
    fn thumb_box(
        thumb: Option<Arc<gpui::RenderImage>>,
        symbol: MapSymbolShape,
        w: f32,
        h: f32,
        glyph: f32,
        fit: ObjectFit,
    ) -> AnyElement {
        let mut face = div()
            .w(px(w))
            .h(px(h))
            .flex_shrink_0()
            .rounded(px(6.0))
            .overflow_hidden()
            .bg(rgba(PANEL_SLATE));
        if let Some(photo) = thumb {
            face = face.child(img(photo).w_full().h_full().object_fit(fit));
        } else {
            face = face.child(
                canvas(
                    |b, _, _| b,
                    move |b, _, window, _| {
                        let cx = f32::from(b.origin.x) + f32::from(b.size.width) / 2.0;
                        let cy = f32::from(b.origin.y) + h / 2.0;
                        paint_glyph(window, (cx, cy), symbol, MAP_INK, glyph);
                    },
                )
                .size_full(),
            );
        }
        face.into_any_element()
    }

    /// A leaf's thumbnail: its photograph when the fetcher has landed one,
    /// the glyph band otherwise — a full-width banner across the top of the
    /// row, above the name and the verbs.
    fn leaf_thumb(&self, leaf: &FleetTaxLeaf) -> AnyElement {
        let id = leaf.id.parse::<i64>().unwrap_or(-1);
        Self::thumb_banner(self.thumbs.get(&id).cloned(), leaf.symbol)
    }

    /// The picker's verdict line, shared by the flat list and the drill —
    /// a refusal from `force::plan` lands here either way.
    fn drill_status_line(&self) -> AnyElement {
        if self.force_status.is_empty() {
            return div().into_any_element();
        }
        div()
            .text_size(px(10.5))
            .text_color(rgba(if self.force_busy { BODY_SILVER } else { ALERT_YELLOW }))
            .child(self.force_status.clone())
            .into_any_element()
    }

    /// Asset-group tabs: one pill per group with its live-hull count. A tab
    /// change clears the whole drill below it — the console's rule, through
    /// `select_group`.
    fn drill_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let active = self.drill_group_id();
        let mut row = div().flex().flex_row().flex_wrap().gap(px(6.0));
        for g in self.fleet_tax.groups.clone() {
            let on = Some(g.id) == active;
            let (fill, border, ink) = if on {
                (RADAR_CYAN, RADAR_CYAN, DEEP_WELL)
            } else {
                (BUTTON_GRAPHITE, HAIRLINE_SLATE, BUTTON_LABEL)
            };
            let id = g.id;
            row = row.child(
                div()
                    .id(("drill-tab", id as usize))
                    .cursor_pointer()
                    .px(px(10.0))
                    .py(px(4.0))
                    .rounded(px(CONTROL_RADIUS))
                    .bg(rgba(fill))
                    .border(px(1.0))
                    .border_color(rgba(border))
                    .font_family(view::FACE_HUMAN)
                    .text_size(px(TEXT_BUTTON_PT))
                    .text_color(rgba(ink))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.select_group(id);
                        cx.notify();
                    }))
                    .child(format!("{} ({})", g.name, g.count)),
            );
        }
        row.into_any_element()
    }

    /// Where the drill stands, rung by rung. Each crumb jumps back — the
    /// rung stays, everything below it clears, through the same `select_*`
    /// the columns use.
    fn drill_crumbs(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut row = div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(4.0))
            .font_family(view::FACE_HUMAN)
            .text_size(px(10.5));
        let crumb = |label: String| {
            div()
                .text_color(rgba(MAP_INK))
                .child(label)
        };
        let sep = || {
            div()
                .text_color(rgba(BODY_SILVER))
                .child("/".to_string())
        };
        row = row.child(
            crumb("Fleet".to_string())
                .id("drill-crumb-root")
                .cursor_pointer()
                .on_click(cx.listener(|view, _, _, cx| {
                view.drill_category = None;
                view.drill_type = None;
                view.drill_class = None;
                cx.notify();
            })),
        );
        let group_name = self
            .fleet_tax
            .groups
            .iter()
            .find(|g| Some(g.id) == self.drill_group_id())
            .map(|g| g.name.clone())
            .unwrap_or_default();
        if !group_name.is_empty() {
            row = row.child(sep());
            row = row.child(crumb(group_name));
        }
        let mut named: Vec<(String, i64, u8)> = Vec::new();
        for c in self.drill_cats() {
            if Some(c.id) == self.drill_category {
                named.push((c.name.clone(), c.id, 0));
            }
        }
        for t in self.drill_types() {
            if Some(t.id) == self.drill_type {
                named.push((t.name.clone(), t.id, 1));
            }
        }
        for c in self.drill_classes() {
            if Some(c.id) == self.drill_class {
                named.push((c.name.clone(), c.id, 2));
            }
        }
        for (label, id, rung) in named {
            row = row.child(sep());
            row = row.child(
                crumb(label)
                    .id(("drill-crumb", rung as usize * 1_000_000 + id as usize))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        match rung {
                            0 => view.select_category(id),
                            1 => view.select_type(id),
                            _ => view.select_class(id),
                        }
                        cx.notify();
                    })),
            );
        }
        row.into_any_element()
    }

    /// The four miller columns. Each column is a title plus its own scroll,
    /// at egui's widths scaled to this modal (3 × 128 + a flex leaf). An
    /// undrilled column names what it waits on rather than showing rows
    /// from a rung the operator has not picked — the console's
    /// `Pick ← first` prompts, in words.
    fn drill_columns(&self, cx: &mut Context<Self>) -> AnyElement {
        let cats = self.drill_cats().to_vec();
        let types = self.drill_types().to_vec();
        let classes = self.drill_classes().to_vec();
        let units = self.drill_units().to_vec();
        let sel_cat = self.drill_category;
        let sel_type = self.drill_type;
        let sel_class = self.drill_class;
        let in_draft: Vec<i64> = self.session.pieces.iter().map(|p| p.id).collect();
        let title = |t: String| {
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(10.5))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        let prompt = |t: &str| {
            div()
                .px(px(4.0))
                .py(px(6.0))
                .text_size(px(10.5))
                .text_color(rgba(BODY_SILVER))
                .child(t.to_string())
        };
        // One taxonomy row: the name, its live-hull count, and the wash when
        // it owns the drill below it.
        let tax_row = |label: String, count: i64, on: bool| {
            let mut el = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(6.0))
                .px(px(6.0))
                .py(px(4.0))
                .rounded(px(3.0))
                .cursor_pointer()
                .font_family(view::FACE_HUMAN)
                .text_size(px(11.0))
                .text_color(rgba(if on { MAP_INK } else { BODY_SILVER }));
            if on {
                el = el.bg(rgba(BUTTON_GRAPHITE));
            }
            el.child(div().flex_1().child(label))
                .child(div().text_size(px(10.0)).child(format!("{count}")))
        };
        let mut cat_col = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .w(px(128.0))
            .flex_shrink_0()
            .child(title(format!("CATEGORY ({})", cats.len())));
        let mut cat_list = div()
            .id("drill-col-cat")
            .flex()
            .flex_col()
            .gap(px(2.0))
            .flex_1()
            .overflow_y_scroll();
        for c in cats {
            let id = c.id;
            let on = sel_cat == Some(id);
            cat_list = cat_list.child(
                div()
                    .id(("drill-cat", id as usize))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.select_category(id);
                        cx.notify();
                    }))
                    .child(tax_row(c.name.clone(), c.count, on)),
            );
        }
        cat_col = cat_col.child(cat_list);
        let mut type_col = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .w(px(128.0))
            .flex_shrink_0()
            .child(title(format!("TYPE ({})", types.len())));
        if sel_cat.is_none() {
            type_col = type_col.child(prompt("Pick a category first"));
        } else {
            let mut l = div()
                .id("drill-col-type")
                .flex()
                .flex_col()
                .gap(px(2.0))
                .flex_1()
                .overflow_y_scroll();
            if types.is_empty() {
                l = l.child(prompt("No types yet"));
            }
            for t in types {
                let id = t.id;
                let on = sel_type == Some(id);
                l = l.child(
                    div()
                        .id(("drill-type", id as usize))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.select_type(id);
                            cx.notify();
                        }))
                        .child(tax_row(t.name.clone(), t.count, on)),
                );
            }
            type_col = type_col.child(l);
        }
        let mut class_col = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .w(px(128.0))
            .flex_shrink_0()
            .child(title(format!("CLASS ({})", classes.len())));
        if sel_type.is_none() {
            class_col = class_col.child(prompt("Pick a type first"));
        } else {
            let mut l = div()
                .id("drill-col-class")
                .flex()
                .flex_col()
                .gap(px(2.0))
                .flex_1()
                .overflow_y_scroll();
            if classes.is_empty() {
                l = l.child(prompt("No classes yet"));
            }
            for c in classes {
                let id = c.id;
                let on = sel_class == Some(id);
                l = l.child(
                    div()
                        .id(("drill-class", id as usize))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.select_class(id);
                            cx.notify();
                        }))
                        .child(tax_row(c.name.clone(), c.count, on)),
                );
            }
            class_col = class_col.child(l);
        }
        let mut leaf_col = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .flex_1()
            .child(title(format!("HULL ({})", units.len())));
        if sel_class.is_none() {
            leaf_col = leaf_col.child(prompt("Pick a class \u{2014} the hulls list here"));
        } else {
            let mut l = div()
                .id("drill-col-leaf")
                .flex()
                .flex_col()
                .gap(px(4.0))
                .flex_1()
                .overflow_y_scroll();
            if units.is_empty() {
                l = l.child(prompt("No hulls in this class"));
            }
            for u in units {
                let unit_id = u.id.parse::<i64>().unwrap_or(-1);
                let taken = in_draft.contains(&unit_id);
                let name = u.name.clone();
                // The drag source: the whole row arms the hull, and the ghost
                // carries its thumbnail and name. The map surface drops it:
                // bare map deploys, a placed host's marker embarks.
                let payload = UnitDragPayload::for_leaf(
                    &u,
                    self.thumbs.get(&unit_id).cloned(),
                );
                let ghost_name = u.name.clone();
                let ghost_symbol = u.symbol;
                let ghost_thumb = self.thumbs.get(&unit_id).cloned();
                let sub = if u.hull.is_empty() {
                    u.class_name.clone()
                } else {
                    format!("{} \u{00b7} {}", u.hull, u.class_name)
                };
                // The row stacks: the photo banner, then the identity
                // line, then verbs. A single line cannot hold a
                // 30-character name plus two pills in a 252pt leaf — the
                // text's minimum shoves the verbs off the modal, and no
                // truncate/w_full/flex_1 combination in this toolkit
                // shrinks it (three captures proved it). Stacked, the text
                // ellipsizes against the full column width and the verbs
                // always fit. Taller than the console's 46pt pitch, which
                // had 232pt of leaf to spend and this modal does not.
                let mut verbs = div().flex().flex_row().justify_end().gap(px(6.0));
                verbs = verbs.child(
                    div()
                        .id(("drill-add", unit_id as usize))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            let who = view.session.me;
                            if view.crew_choices().iter().all(|(id, _)| *id != who) {
                                view.force_status =
                                    "your account holds no seat in this room".to_string();
                                cx.notify();
                                return;
                            }
                            match view.stage_assign(unit_id, &name, who) {
                                Ok(()) => {
                                    view.force_status = "piece assigned".to_string()
                                }
                                Err(why) => view.force_status = why,
                            }
                            cx.notify();
                        }))
                        .child(button(
                            if taken { "assigned" } else { "add" },
                            true,
                            taken,
                            false,
                        )),
                );
                // Click-to-place, the drag's fallback: only a drafted hull
                // can hold a starting position, so only a taken row arms.
                // The button says what the next map click will do.
                if taken {
                    let place_name = u.name.clone();
                    verbs = verbs.child(
                        div()
                            .id(("drill-place", unit_id as usize))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.arm_place(unit_id, &place_name, cx);
                            }))
                            .child(button("place", true, false, false)),
                    );
                }
                l = l.child(
                    div()
                        .id(("drill-drag", unit_id as usize))
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .px(px(4.0))
                        .py(px(4.0))
                        .rounded(px(3.0))
                        .w_full()
                        .on_drag(payload, move |_, _, _, cx| {
                            cx.new(|_| DragGhost {
                                name: ghost_name.clone(),
                                symbol: ghost_symbol,
                                thumb: ghost_thumb.clone(),
                            })
                        })
                        // The banner first: the photograph spans the row's
                        // full width, and the identity sits under it — the
                        // ship is the row's first word, not a stamp
                        // beside the name.
                        .child(self.leaf_thumb(&u))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .pt(px(8.0))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .overflow_hidden()
                                        .child(
                                            div()
                                                .text_size(px(11.0))
                                                .text_color(rgba(MAP_INK))
                                                .truncate()
                                                .child(u.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(10.0))
                                                .text_color(rgba(BODY_SILVER))
                                                .truncate()
                                                .child(sub),
                                        ),
                                ),
                        )
                        .child(verbs),
                );
            }
            leaf_col = leaf_col.child(l);
        }
        div()
            .flex()
            .flex_row()
            .gap(px(8.0))
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .child(cat_col)
            .child(type_col)
            .child(class_col)
            .child(leaf_col)
            .into_any_element()
    }

    fn fleet_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let small = |t: String| {
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        if self.session.game_id == 0 {
            return col
                .child(small("Pick or create a session first.".into()))
                .into_any_element();
        }
        let staged = self.session.pieces.len();
        col = col.child(
            div()
                .id("fleet-picker-door")
                .cursor_pointer()
                .on_click(cx.listener(|view, _, _, cx| {
                    view.open_only(WhichPanel::Fleet);
                    cx.notify();
                }))
                .child(button(
                    if staged > 0 {
                        format!("Fleet picker ({staged} in the draft)")
                    } else {
                        "Fleet picker".to_string()
                    },
                    false,
                    false,
                    false,
                )),
        );
        col = col.child(separator());
        col = col.child(small("PIECES".to_string()));
        if self.session.pieces.is_empty() {
            col = col.child(small("No pieces yet \u{2014} assign register hulls above.".into()));
            return col.into_any_element();
        }
        for p in &self.session.pieces {
            // What is embarked aboard this hull lives in the Embarked
            // modal — this row only opens it, the console's rule (`show…`
            // beside a weak `Embarked`, and the comment that the row owns
            // the data while the row only opens it). The door names the
            // count once the cache knows it.
            let pid = p.id;
            let door = match self.embarked.get(&pid) {
                Some(units) if !units.is_empty() => {
                    format!("{} aboard \u{00b7} show\u{2026}", units.len())
                }
                _ => "show\u{2026}".to_string(),
            };
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .w(px(6.0))
                            .h(px(6.0))
                            .rounded(px(3.0))
                            .flex_shrink_0()
                            .bg(rgba(if p.placed { SIGNAL_GREEN } else { STALE_SAND })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .font_family(view::FACE_HUMAN)
                            .text_size(px(11.0))
                            .text_color(rgba(MAP_INK))
                            .child(p.label.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(rgba(if p.placed { BODY_SILVER } else { ALERT_YELLOW }))
                            .child(if p.placed { "placed" } else { "unplaced" }),
                    )
                    .child(
                        div()
                            .id(("embark-open", pid as usize))
                            .cursor_pointer()
                            .flex_shrink_0()
                            .text_size(px(10.5))
                            .text_color(rgba(BODY_SILVER))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.open_embarked(pid);
                                cx.notify();
                            }))
                            .child(door),
                    ),
            );
        }
        col.into_any_element()
    }

    /// EMBARKED — one hull's passengers, on the readout card.
    ///
    /// The console's floating "Embarked" island, which this shell has no
    /// floating version of — so the modal zone hosts it, like the comms
    /// inbox before it. The read is lazy per host (the console's
    /// `ensure_embarked`): the open IS the request when no click preceded
    /// it — the headless seed opens no doors — so the first draw issues
    /// the read, latched single-flight, and the answer ends it. A refusal
    /// stops the loop on the status line, with a Reload pill the console
    /// does without: a failed first load needs a retry path, and the
    /// island it ports has none to copy.
    ///
    /// Rows are the console's verbatim: the name, `hull · class`, and the
    /// clearance in its own words. Unembark, clear and revoke ride each
    /// row as pills — one press per row, never a batch, because the
    /// permission follows the piece.
    fn embark_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let small = |t: String| {
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        let Some(host) = self.embark_host else {
            return small("No host held.".to_string()).into_any_element();
        };
        if self.embark_status.is_empty() {
            self.send_embarked(cx);
        }
        let name = self
            .session
            .pieces
            .iter()
            .find(|p| p.id == host)
            .map(|p| p.label.clone())
            .unwrap_or_else(|| format!("hull {host}"));
        let mut col = div().flex().flex_col().gap(px(4.0)).p(px(PAD));
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .font_family(view::FACE_HUMAN)
                        .text_size(px(13.0))
                        .text_color(rgba(MAP_INK))
                        .child("EMBARKED".to_string()),
                )
                .child(div().flex_1().child(small(format!("aboard {name}"))))
                .child(
                    div()
                        .id("embark-close")
                        .cursor_pointer()
                        .w(px(CLOSE))
                        .h(px(CLOSE))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.embark_host = None;
                            view.open_only(WhichPanel::None);
                            cx.notify();
                        }))
                        // The painted cross, never the glyph: Saira has no
                        // U+2715 and type would print `.notdef` — the parked
                        // defect the island bands work around the same way.
                        .child(close_cross(false)),
                ),
        );
        col = col.child(separator());
        match self.embarked.get(&host) {
            None if self.embark_busy => {
                col = col.child(small(self.embark_status.clone()));
            }
            None => {
                if !self.embark_status.is_empty() {
                    col = col.child(small(self.embark_status.clone()));
                    col = col.child(
                        div()
                            .id("embark-reload")
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.embark_status.clear();
                                if let Some(h) = view.embark_host {
                                    view.embarked.remove(&h);
                                }
                                view.send_embarked(cx);
                            }))
                            .child(button("Reload", true, false, false)),
                    );
                }
            }
            Some(units) if units.is_empty() => {
                col = col.child(small("nothing embarked".to_string()));
            }
            Some(units) => {
                for u in units {
                    let uid = u.id;
                    let cleared = u.is_cleared;
                    col = col.child(
                        div()
                            .font_family(view::FACE_HUMAN)
                            .text_size(px(11.0))
                            .text_color(rgba(MAP_INK))
                            .child(u.name.clone()),
                    );
                    col = col.child(small(format!("{} \u{00b7} {}", u.hull_number, u.class_name)));
                    col = col.child(small(if u.is_cleared {
                        "cleared \u{2014} may act".to_string()
                    } else {
                        "aboard \u{2014} takes no orders until cleared".to_string()
                    }));
                    // Permission follows the piece, not a batch: one press
                    // per row — the console's rule, where each click pushes
                    // one id. Cleared rows offer revoke, the rest clear.
                    col = col.child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id(("embark-unembark", uid as usize))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.send_unembark(host, uid, cx);
                                    }))
                                    .child(button("Unembark", true, false, false)),
                            )
                            .child(
                                div()
                                    .id(("embark-clear", uid as usize))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        if cleared {
                                            view.send_revoke_clearance(host, uid, cx);
                                        } else {
                                            view.send_clear_aboard(host, uid, cx);
                                        }
                                    }))
                                    .child(button(
                                        if cleared { "Revoke" } else { "Clear" },
                                        true,
                                        false,
                                        false,
                                    )),
                            ),
                    );
                    col = col.child(separator());
                }
                // The last write's capacity warnings — a warning, never a
                // refusal — in the console's yellow and its own sentence.
                if let Some(over) = self.embark_over.get(&host) {
                    for o in over {
                        col = col.child(
                            div()
                                .font_family(view::FACE_HUMAN)
                                .text_size(px(11.0))
                                .text_color(rgba(ALERT_YELLOW))
                                .child(format!(
                                    "over capacity: {} {} holds {} (max {})",
                                    o.level, o.id_level, o.would_hold, o.max_aboard
                                )),
                        );
                    }
                }
                // A write in flight names itself here; the answer clears
                // the line and the rows move. Refusals land here too.
                if !self.embark_status.is_empty() {
                    col = col.child(small(self.embark_status.clone()));
                }
            }
        }
        col.into_any_element()
    }

    /// SESSIONS — the games to hold, and the verb that holds one.
    ///
    /// The console's `z.start` minus its create button, which lives on the
    /// band here. One row per game the sign-in read, in the server's own
    /// order: the name bright, the state and this seat dimmed beside it
    /// (or "not seated", for a game this account only watches). The row IS
    /// the verb — a picker with a separate hold button would be two steps
    /// for one decision — and holding replaces the session whole, so the
    /// column recomposes around the held game on the next frame.
    fn sessions_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let small = |t: String| {
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        if self.sessions.is_empty() {
            return col
                .child(small("No sessions listed \u{2014} sign out and back in to read them.".into()))
                .into_any_element();
        }
        for s in &self.sessions {
            let id = s.id;
            let seat = if s.my_role.is_empty() {
                "not seated".to_string()
            } else if s.judge {
                format!("{} \u{00b7} judge", s.my_role)
            } else {
                s.my_role.clone()
            };
            col = col.child(
                div()
                    .id(("session-hold", id as usize))
                    .cursor_pointer()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(6.0))
                    .py(px(4.0))
                    .rounded(px(CONTROL_RADIUS))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.send_hold(id, cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .font_family(view::FACE_HUMAN)
                            .text_size(px(11.0))
                            .text_color(rgba(MAP_INK))
                            .child(s.name.clone()),
                    )
                    .child(
                        div()
                            .font_family(view::FACE_MACHINE)
                            .text_size(px(10.5))
                            .text_color(rgba(BODY_SILVER))
                            .child(s.state.to_uppercase()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.5))
                            .text_color(rgba(BODY_SILVER))
                            .child(seat),
                    ),
            );
        }
        // The hold's own verdict line: refusals land here, never silently.
        if !self.hold_status.is_empty() {
            col = col.child(small(self.hold_status.clone()));
        }
        col.into_any_element()
    }

    /// PLAYERS — who is seated in the room, and the door to the seating form.
    ///
    /// The console's `z.players`. It was called CREW here, which broke the
    /// shell's own rule that a panel the operator already knows under one name
    /// is not renamed by a port — and CREW is a real, different console island
    /// (`z.crew`, the arrival list), so the old name was not merely wrong, it
    /// was the name of something else.
    fn players_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let small = |t: String| {
            div()
                .font_family(view::FACE_HUMAN)
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        if self.session.game_id == 0 {
            return col
                .child(small("Pick or create a session first.".into()))
                .into_any_element();
        }
        col = col.child(small(format!(
            "{} \u{00b7} {} seat(s) \u{00b7} {} ready{}",
            if self.session.game.is_empty() {
                "no session".to_string()
            } else {
                self.session.game.clone()
            },
            self.session.seats,
            self.session.ready,
            if self.session.my_role.is_empty() {
                String::new()
            } else {
                format!(" \u{00b7} you: {}", self.session.my_role)
            }
        )));
        // The door, exactly as the console's `z.players` has one: seating is
        // one decision about a person, and in a 280-point column that decision
        // needs room the island does not have. The directory is already read
        // (`fetch_session` carries it), so opening it renders the real thing
        // rather than an empty form with no read attempted.
        col = col.child(
            div()
                .id("player-picker-door")
                .cursor_pointer()
                .on_click(cx.listener(|view, _, _, cx| {
                    view.open_only(WhichPanel::Player);
                    cx.notify();
                }))
                .child(button(
                    if self.session.people.is_empty() {
                        "Player picker".to_string()
                    } else {
                        format!("Player picker ({} seated)", self.session.people.len())
                    },
                    false,
                    false,
                    false,
                )),
        );
        col = col.child(separator());
        if self.session.people.is_empty() {
            col = col.child(small("No seats in this room.".into()));
            return col.into_any_element();
        }
        for p in &self.session.people {
            let mine = p.id == self.session.me;
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(20.0))
                    .px(px(4.0))
                    .rounded(px(3.0))
                    .bg(rgba(if mine {
                        RADAR_CYAN.with_alpha(38)
                    } else {
                        Rgba::black_alpha(0)
                    }))
                    .child(
                        div()
                            .w(px(78.0))
                            .font_family(view::FACE_MACHINE)
                            .text_size(px(11.0))
                            .text_color(rgba(if mine { RADAR_CYAN } else { MAP_INK }))
                            .child(if p.call_sign.is_empty() {
                                "\u{2014}".to_string()
                            } else {
                                p.call_sign.clone()
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.0))
                            .text_color(rgba(MAP_INK))
                            .child(p.name.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(rgba(BODY_SILVER))
                            .child(if p.judge {
                                format!("{} \u{00b7} judge", p.role)
                            } else {
                                p.role.clone()
                            }),
                    )
                    .child(
                        div()
                            .w(px(52.0))
                            .text_size(px(10.5))
                            .text_color(rgba(if p.ready { RADAR_CYAN } else { BODY_SILVER }))
                            .child(if p.ready { "ready" } else { "\u{2014}" }),
                    ),
            );
        }
        col.into_any_element()
    }

    /// CREW — the room as it actually stands: seated is a role, in room is a
    /// fact.
    ///
    /// The console's `z.crew`, whose whole reason for existing next to Players
    /// is that sentence. An exercise with three seats and one arrival is the
    /// thing the Game Master needs to see, and a seat count cannot say it.
    fn crew_body(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(view::PANEL_PAD));
        if self.session.game_id == 0 {
            return col.child(note("No session held.")).into_any_element();
        }
        if self.session.people.is_empty() {
            return col.child(note("Nobody seated.")).into_any_element();
        }
        // Kept, and worth keeping: it is the distinction the console's own
        // roster gets wrong most easily. Seated is what the Game Master did;
        // in room is what the person did. They are different facts and the
        // state column below reads the second one.
        col = col.child(section("IN ROOM"));
        col = col.child(note("seated is a role; in room is arrival"));
        for p in &self.session.people {
            // The state, routed through the status palette rather than left
            // grey: four states, three of them ink, none of them Body Silver.
            // A judge is neither ready nor late — the server exempts them from
            // readiness entirely — so it reads as the neutral fact it is.
            let (mark, ink) = if p.judge {
                ("judge".to_string(), IDLE_GREY)
            } else if !p.present {
                ("not arrived".to_string(), ALERT_YELLOW)
            } else if p.ready {
                ("in room".to_string(), SIGNAL_GREEN)
            } else {
                ("in room \u{00b7} not ready".to_string(), ALERT_YELLOW)
            };
            col = col.child(ruled(vec![
                div()
                    .flex_1()
                    .font_family(view::FACE_HUMAN)
                    .text_size(px(view::BODY_PT))
                    .text_color(rgba(MAP_INK))
                    .child(p.name.clone())
                    .into_any_element(),
                machine_col(p.role.clone(), 96.0, BODY_SILVER).into_any_element(),
                machine_col(mark, 132.0, ink).into_any_element(),
            ]));
        }
        col.into_any_element()
    }

    /// REVIEW — the gate: what Minos still wants, the key, and the advance.
    ///
    /// The console's `z.review`. Three things and they belong together: the
    /// checklist the server publishes, the room key the Game Master reads out
    /// to the people who have not arrived, and the one verb that leaves
    /// preparation. The verb is deliberately NOT gated on the role name — the
    /// server 403s a seat without the grant, and a button that hid itself
    /// would strand exactly the session whose Game Master is named something
    /// else (`main.rs`'s own reasoning, kept).
    fn review_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(view::PANEL_PAD));
        if self.session.game_id == 0 {
            return col.child(note("No session held.")).into_any_element();
        }
        // The gate's own headline, and it is a readout rather than a sentence.
        // This panel exists to answer one question — what is still outstanding —
        // so the answer is the largest thing on it. Map Ink at zero and Alert
        // Yellow above it: the same rule Step follows, and the same reason. A
        // status colour in the clear state would spend the ink on a verdict of
        // "nothing", and this panel is clear more often than it is not.
        match self.session.readiness.clone() {
            // A server that never published the check cannot be waited out:
            // retrying what does not exist strands the exercise, so the
            // advance goes through and Minos gates it on its own terms. This
            // is the console's `ReadinessGap::Absent` rule.
            None => {
                col = col.child(section("OUTSTANDING"));
                col = col.child(
                    note("readiness unpublished \u{2014} the server decides at advance"),
                );
            }
            Some(r) if r.blockers.is_empty() => {
                col = col.child(section("OUTSTANDING"));
                col = col.child(verdict("0", "READY", MAP_INK));
            }
            Some(r) => {
                col = col.child(section("OUTSTANDING"));
                col = col.child(verdict(
                    &r.blockers.len().to_string(),
                    "OUTSTANDING",
                    ALERT_YELLOW,
                ));
                // Three, not all of them. The panel's job is to say the gate
                // is closed and name the first things in the way; the full
                // list is the server's and the roster's, and a Review panel
                // that scrolls to forty bullets is a log with a button under it.
                for b in r.blockers.iter().take(3) {
                    col = col.child(note(format!("\u{2022} {b}")));
                }
            }
        }
        col = col.child(section("ROOM KEY"));
        let key = self.session.facts.room_key.clone();
        if key.is_empty() {
            col = col.child(note(
                "no room key on this session \u{2014} ask its Game Master.",
            ));
        } else {
            // The key is the one thing on this panel a person will read
            // aloud and retype, so it is a machine figure. PT Mono is
            // monospaced, which is the whole point: `l` against `1` in a key
            // a voice says aloud costs a session, and a fixed advance makes
            // every glyph a separate mark. No `tnum` needed — that feature is
            // for a proportional face, which PT Mono is not.
            col = col.child(kv("KEY", key, MAP_INK));
            col = col.child(note(
                "share this with your personnel \u{2014} they enter it to join the room.",
            ));
        }
        col = col.child(section("ADVANCE"));
        // The phase is NOT a gate here. The console offers this verb from the
        // planning Review island as well as from preparation's Readiness, and
        // it says why: what may advance is the server's policy call, so a
        // client that hid the button would be one more copy of a rule it does
        // not own. The checklist IS a gate, because it is the server's own
        // published verdict.
        let clear = self.readiness_clear();
        col = col.child(
            div()
                .id("review-advance")
                .cursor_pointer()
                .on_click(cx.listener(|view, _, _, cx| {
                    view.advance_execution(cx);
                }))
                .child(button("Enter execution \u{2192}", false, !clear, false)),
        );
        // A write's outcome, and it goes through the status palette: this is
        // exactly the line that must never be grey on grey.
        if !self.advance_status.is_empty() {
            col = col.child(note(self.advance_status.clone()));
        }
        col.into_any_element()
    }

    /// The My-command panel (the console's `dashboard_island`): the seat, the
    /// hulls this account commands, and each one's latest fix.
    ///
    /// The console's island is three columns — own ship, task group, helm.
    /// Two of the three are here. The task group needs the session hierarchy
    /// (`groups`, not yet wired) and the helm is a WRITE (a helm order), so
    /// both are named rather than faked: the panel says which slice they are.
    ///
    /// The fix is joined from the LIVE markers by unit id — the same key the
    /// roster and the wire share — so the panel never reads the registry.
    fn command_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // Content-sized, not `size_full`: this column is an ISLAND body now,
        // and the island's own scroll viewport does the measuring. A
        // `size_full` child inside a scroll viewport sizes to the viewport
        // and then clips its own overflow, which is a body that silently
        // loses its tail instead of scrolling to it.
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(view::PANEL_PAD));
        // Readiness is the Command island's first working row in every state
        // past planning: the console's `readiness_ui`, all four states, with
        // no header of its own — the island's band already says Command and
        // carries the close.
        match self.own_seat().cloned() {
            None => {
                col = col.child(note("no seat in this room \u{2014} joining needs the room key"));
            }
            Some(p) if p.judge => {
                // Judge side: the server exempts them from readiness, so the
                // row says so rather than offering a verb that would be
                // refused. Name and role are the two facts, and the exemption
                // is the note under them.
                col = col.child(kv("SEAT", p.role.clone(), MAP_INK));
                col = col.child(
                    div()
                        .font_family(view::FACE_HUMAN)
                        .text_size(px(view::BODY_PT))
                        .text_color(rgba(MAP_INK))
                        .child(p.name.clone()),
                );
                col = col.child(note("judge side is exempt from readiness"));
            }
            Some(p) if p.ready => {
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgba(MAP_INK))
                                .child(format!("{} · {} — ready to start", p.name, p.role)),
                        )
                        .child(
                            div()
                                .id("withdraw-readiness")
                                .cursor_pointer()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.set_readiness(false, cx);
                                }))
                                .child(button("withdraw", true, self.readiness_busy, false)),
                        ),
                );
            }
            Some(p) => {
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(rgba(MAP_INK))
                                .child(format!("{} · {}", p.name, p.role)),
                        )
                        .child(
                            div()
                                .id("declare-readiness")
                                .cursor_pointer()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.set_readiness(true, cx);
                                }))
                                .child(button("ready to start", true, self.readiness_busy, false)),
                        ),
                );
            }
            None => {
                col = col.child(note(
                    "you hold no seat in this session — join with the room key or ask the \
                     Game Master",
                ));
            }
        }
        // A status line, and it goes through the status palette: a write's
        // outcome is exactly the thing that must never be grey on grey.
        if !self.readiness_status.is_empty() {
            col = col.child(note(self.readiness_status.clone()));
        }
        col = col.child(section("HULLS"));
        if self.session.mine.is_empty() {
            col = col.child(note("No hull assigned to this seat."));
        }
        for ship in &self.session.mine {
            let live = self.live.iter().find(|m| m.id == ship.id);
            let course = live
                .and_then(|m| m.heading_deg)
                .map(|h| format!("{h:.0}\u{00b0}"))
                .unwrap_or_else(|| "\u{2014}".to_string());
            let speed = live
                .and_then(|m| m.speed_kn)
                .map(|s| format!("{s:.1} kn"))
                .unwrap_or_else(|| "\u{2014}".to_string());
            let stale = live.map(|m| m.stale).unwrap_or(false);
            // A ruled ledger row rather than a highlighted box. The old row
            // put a cyan wash behind the whole line, which spent the accent on
            // every row at once — the same failure the island rim had. The
            // state is the last column's ink, and nothing else is coloured.
            let row = ruled(vec![
                // An explicit width, not `flex_1`. A flex item will not
                // shrink below its min-content width and neither `min_w_0`
                // nor `truncate` overrides that here, so a long hull name
                // pushes the machine columns off the panel's right edge — the
                // speed column lost its unit glyph and the state column was
                // gone entirely. A definite width is what holds the columns
                // in place; `truncate` ellipsizes the name into it. At 330pt
                // of content width: 150 + 44 + 56 + 44 + three 8pt gaps = 318.
                div()
                    .w(px(150.0))
                    .flex_shrink_0()
                    .truncate()
                    .font_family(view::FACE_HUMAN)
                    .text_size(px(view::BODY_PT))
                    .text_color(rgba(MAP_INK))
                    .child(ship.label.clone())
                    .into_any_element(),
                machine_col(course, 44.0, BODY_SILVER).into_any_element(),
                machine_col(speed, 56.0, BODY_SILVER).into_any_element(),
                machine_col(
                    match (live.is_some(), stale) {
                        (false, _) => "\u{2014}",
                        (true, true) => "stale",
                        (true, false) => "live",
                    },
                    44.0,
                    if stale { STALE_SAND } else { BODY_SILVER },
                )
                .into_any_element(),
            ]);
            let ship_id = ship.id.clone();
            let row = div()
                // The hull's own id, not its position: the roster changes
                // under this row and an index would hand the next hull's
                // selection to whichever row moved into that slot.
                .id(gpui::ElementId::Name(format!("helm-target-{ship_id}").into()))
                .cursor_pointer()
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.helm_target = Some(ship_id.clone());
                    cx.notify();
                }))
                .child(row);
            col = col.child(row);
        }
        // The helm: the panel's first WRITE. Armed only in an executing game
        // with a commanded hull — a commander plans first and orders after,
        // which is the console's own rule — and the button guards in the
        // listener, not only in its styling: a disabled look that still
        // posts is the one thing this must not be.
        col = col.child(section("HELM"));
        // Which hull the order is FOR, above the verb that sends it. The
        // console's dashboard puts the active hull in a ComboBox; here the
        // rows above are the picker and this line is the answer, so a posted
        // order is never silently somebody else's hull.
        if let Some(ship) = self.order_target() {
            col = col.child(note(format!("orders {}", ship.label)));
        }
        let armed = self.session.execution && !self.session.mine.is_empty();
        let step_row = |label: &'static str,
                        down: &'static str,
                        up: &'static str,
                        value: String|
         -> gpui::Div {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .h(px(22.0))
                .child(
                    div()
                        .w(px(56.0))
                        .text_size(px(11.0))
                        .text_color(rgba(BODY_SILVER))
                        .child(label),
                )
                .child(div().id(down).cursor_pointer().child(button("\u{2212}", true, !armed, false)))
                .child(
                    div()
                        .w(px(64.0))
                        .font_family(view::FACE_MACHINE)
                        .text_size(px(11.0))
                        .text_color(rgba(MAP_INK))
                        .child(value),
                )
                .child(div().id(up).cursor_pointer().child(button("+", true, !armed, false)))
        };
        let course = self
            .helm_heading
            .map(|c| format!("{c:.0}\u{00b0}"))
            .unwrap_or_else(|| "\u{2014}".to_string());
        let speed = self
            .helm_speed
            .map(|s| format!("{s:.1} kn"))
            .unwrap_or_else(|| "\u{2014}".to_string());
        col = col.child(step_row("Course", "helm-course-down", "helm-course-up", course));
        col = col.child(step_row("Speed", "helm-speed-down", "helm-speed-up", speed));
        let status = self.helm_status.clone();
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .id("helm-send")
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| {
                            if view.send_helm() {
                                cx.notify();
                            }
                        }))
                        .child(button("SET HELM", false, !armed, false)),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(rgba(if status.as_ref().is_some_and(|s| !s.ok) {
                            STALE_SAND
                        } else {
                            BODY_SILVER
                        }))
                        .child(status.map(|s| s.text).unwrap_or_default()),
                ),
        );
        col = col.child(
            div()
                .text_size(px(10.0))
                .text_color(rgba(BODY_SILVER.with_alpha(178)))
                .child(if armed {
                    "An order posts at once; the server clamps and answers."
                } else {
                    "The helm arms with execution \u{2014} a commander plans first and orders after."
                }),
        );
        col.into_any_element()
    }

    /// Which hull the helm orders: the picked one if it is still commanded,
    /// and otherwise the first commanded hull.
    ///
    /// Resolved against the LIVE roster on every call rather than trusted, so
    /// a hull reassigned away mid-session stops being orderable without
    /// anybody clearing a flag. The fallback is the first hull because the
    /// console's own default is the first, and a helm with no target at all
    /// would be a control that silently does nothing.
    fn order_target(&self) -> Option<&MyShip> {
        if let Some(id) = &self.helm_target {
            if let Some(ship) = self.session.mine.iter().find(|s| &s.id == id) {
                return Some(ship);
            }
        }
        self.session.mine.first()
    }

    /// Send the drafted helm order for the commanded hull — the shell's one
    /// WRITE.
    ///
    /// Returns false when nothing may be sent: no execution, no commanded
    /// hull, or no draft. That is the SAME rule the button enforces, kept in
    /// one place so a test exercises exactly what the click does — the server
    /// enforces it too, with a 409, which is why the check is here at all
    /// rather than only in the response handler.
    fn send_helm(&mut self) -> bool {
        if !self.session.execution {
            return false;
        }
        let (Some(heading_deg), Some(speed_kn)) = (self.helm_heading, self.helm_speed) else {
            return false;
        };
        let Some(ship) = self.order_target() else {
            return false;
        };
        let target = ship.id.clone();
        let _ = self.order_tx.send(ToBinary::Helm(HelmOrder {
            unit_id: target.clone(),
            heading_deg,
            speed_kn,
        }));
        self.helm_status = Some(HelmStatus {
            unit_id: target,
            ok: true,
            text: "sending\u{2026}".to_string(),
        });
        true
    }

    /// The caller's own seat in the held game, by user id — the same join the
    /// binary uses to find commanded hulls, so the panel and the wire never
    /// disagree about who is seated.
    fn own_seat(&self) -> Option<&PersonView> {
        self.session.people.iter().find(|p| p.id == self.session.me)
    }

    /// Minos's own force, as the draft's `LiveHull` mirror.
    ///
    /// Derived from the session's pieces rather than stored, so there is one
    /// source for what the server holds.
    ///
    /// The session read carries NO starting position — `GameUnit` has no
    /// placement field, which is why the shell projects the placements read
    /// separately — so `start` is `None` here. The consequence is deliberate
    /// and narrow: `plan()` compares the draft's starts against nothing, so a
    /// draft hull with a live counterpart ALWAYS reads as a placement move and
    /// emits a `Place`. Re-writing a start the server already has is
    /// idempotent at the endpoint, so the redundancy is the honest side of the
    /// trade — a placement the operator can see is worth one repeated PUT.
    ///
    /// `host` is `None` for the same reason: the read does not say what
    /// carries a piece, so the mirror cannot tell a hull already aboard from
    /// one on the water. Two consequences follow, and both are the shell's
    /// limit rather than the server's. A seeded hull's draft `host` is `None`
    /// too, so pass 2 sees `None == None` and writes no embarkation — an
    /// embarkation the operator did not touch is left standing rather than
    /// re-stated. But an operator who DROPS a hull onto a host emits one
    /// `Embark`, and if the server already carries that hull aboard, the
    /// attach endpoint refuses it ("already inside something") and the whole
    /// advance stops with that sentence. A hull's host belongs in the session
    /// read; until it is there, this mirror is one field short and says so.
    fn live_hulls(&self) -> Vec<crate::force::LiveHull> {
        self.session
            .pieces
            .iter()
            .map(|p| crate::force::LiveHull {
                unit_id: p.id,
                name: p.label.clone(),
                commander_id: p.commander_id,
                start: None,
                host: None,
            })
            .collect()
    }

    /// Re-seed BOTH drafts from what the session holds.
    ///
    /// One call, because they are re-seeded for one reason: a session read
    /// replaced the pieces and the seats, and both drafts are mirrors of what
    /// Minos was last told. Two calls would be two chances to re-seed one and
    /// forget the other.
    ///
    /// SAFE ONLY WHERE NOTHING IS STAGED. Before this slice every edit posted
    /// at once, so a re-read could not discard work. While PLANNING the draft
    /// IS the record — a re-seed would drop edits the server has not been told
    /// — and the guard is twofold. The callers that could fire with staged
    /// work are the Session island's advance, which seeds AFTER the read that
    /// proves the draft was written through, and the three posting write
    /// latches (`roster_busy`, `force_busy`, `deploy_busy`), whose flags only
    /// an edit that POSTED sets — and no planning edit posts. The callers that
    /// seed a planning room seed it BEFORE there is any work to lose:
    /// construction, a hold and a created session all install a fresh draft
    /// whose reference is the game just read.
    fn reseed_drafts(&mut self) {
        let mut draft = crate::force::ForceDraft::new();
        for p in &self.session.pieces {
            draft.upsert(crate::force::DraftHull {
                unit_id: p.id,
                name: p.label.clone(),
                hull_number: String::new(),
                class_id: String::new(),
                start: None,
                commander_id: p.commander_id,
                host: None,
            });
        }
        self.force = draft;

        let mut seats = crate::roster::RosterDraft::new();
        for p in &self.session.people {
            seats.stage(crate::roster::StagedSeat {
                user_id: p.id,
                user_name: p.name.clone(),
                role_id: p.role_id,
                role_name: p.role.clone(),
                call_sign: p.call_sign.clone(),
                judge: p.judge,
            });
        }
        self.roster_draft = seats;
    }

    /// Who may be handed a piece: every seat in the room. A hull with nobody
    /// to command it cannot be assigned — `force::plan` refuses the whole plan
    /// and names the hulls, because handing a piece to whoever happened to be
    /// seated first is the one answer nobody can undo by looking at the map.
    fn crew_choices(&self) -> Vec<(i64, String)> {
        self.session
            .people
            .iter()
            .map(|p| (p.id, p.name.clone()))
            .collect()
    }

    /// Put a register hull in the draft, and write it through.
    ///
    /// The commander is TAKEN, not checked: whether a given account may hold a
    /// piece is the picker's question (it offers the room's seats and nothing
    /// else), and whether a hull may be in the exercise without one is
    /// `force::plan`'s. Validating the seat here would be a second copy of a
    /// rule the kernel already owns, and the copy that drifts is the one that
    /// writes.
    ///
    /// WHILE PLANNING this stages and returns: the hull joins the draft and
    /// nothing is posted, because the Session island's advance is the moment
    /// the whole draft is written through. In every later state it posts the
    /// plan at once, because no advance is left to wait for and the write IS
    /// the edit.
    fn stage_assign(&mut self, unit_id: i64, name: &str, commander_id: i64) -> Result<(), String> {
        let mut draft = self.force.clone();
        draft.upsert(crate::force::DraftHull {
            unit_id,
            name: name.to_string(),
            hull_number: String::new(),
            class_id: String::new(),
            start: None,
            commander_id: Some(commander_id),
            host: None,
        });
        self.force = draft;
        if self.staging() {
            return Ok(());
        }
        self.sync_force()
    }

    /// Hand a drafted hull to somebody else, and write it through.
    fn stage_handover(&mut self, unit_id: i64, commander_id: i64) -> Result<(), String> {
        if !self.force.set_commander(unit_id, Some(commander_id)) {
            return Err("that piece is not in the draft".to_string());
        }
        if self.staging() {
            return Ok(());
        }
        self.sync_force()
    }

    /// Take a drafted hull out of the exercise, and write it through.
    fn stage_remove(&mut self, unit_id: i64) -> Result<(), String> {
        if self.force.remove(unit_id).is_none() {
            return Err("that piece is not in the draft".to_string());
        }
        if self.staging() {
            return Ok(());
        }
        self.sync_force()
    }

    /// Deploy a hull at map-view fractions. While PLANNING this places it in
    /// the DRAFT and posts nothing; otherwise it latches the flight, says so
    /// on the picker's line, and posts. The answer is a re-read session on the
    /// shared feed, applied under `deploy_busy` — the same shape as
    /// `sync_force`, a separate latch so the two flights never share a verdict.
    fn send_deploy(&mut self, unit_id: i64, name: &str, fx: f64, fy: f64, cx: &mut Context<Self>) {
        if self.staging() {
            self.stage_place_local(unit_id, name, fx, fy, cx);
            return;
        }
        self.deploy_busy = true;
        self.pending_place = None;
        self.place_down = None;
        self.force_status = "placing\u{2026}".to_string();
        let _ = self
            .order_tx
            .send(ToBinary::DeployUnit { unit_id, fx, fy });
        cx.notify();
    }

    /// A register hull dropped on the map: placing IS assigning. A hull
    /// already in the exercise deploys (or embarks, on a host) exactly as
    /// before; a hull outside it is assigned first — to the operator's own
    /// seat, the same default the add button uses — and the binary chains
    /// the placement behind the assignment in one verb, so the two can
    /// never half-commit. Without a seat there is nothing to hand it to,
    /// and the refusal says so instead of spending the round trip.
    fn drop_unit(
        &mut self,
        unit_id: i64,
        name: &str,
        fx: f64,
        fy: f64,
        host: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        let taken = self.session.pieces.iter().any(|p| p.id == unit_id);
        // WHILE PLANNING the drop is a LOCAL act: the hull joins the draft
        // (assigned on the way in, if the exercise does not hold it) and
        // nothing posts. The advance writes the whole draft through.
        if self.staging() {
            if let Some(host) = host {
                self.stage_embark_local(host, unit_id, name, cx);
            } else {
                self.stage_place_local(unit_id, name, fx, fy, cx);
            }
            return;
        }
        if taken {
            if let Some(host) = host {
                self.send_embark(host, unit_id, name, cx);
            } else {
                self.send_deploy(unit_id, name, fx, fy, cx);
            }
            return;
        }
        let who = match self.own_commander() {
            Ok(who) => who,
            Err(why) => {
                self.force_status = why;
                cx.notify();
                return;
            }
        };
        if let Some(host) = host {
            self.embark_busy = true;
            self.embark_status = format!("assigning {name} and embarking aboard {host}\u{2026}");
            self.force_status = format!("assigning {name} and embarking aboard {host}\u{2026}");
            let _ = self.order_tx.send(ToBinary::EmbarkAssign {
                host,
                child: unit_id,
                commander_id: who,
            });
        } else {
            self.deploy_busy = true;
            self.pending_place = None;
            self.place_down = None;
            self.force_status = format!("assigning {name} and placing\u{2026}");
            let _ = self.order_tx.send(ToBinary::DeployAssign {
                unit_id,
                commander_id: who,
                fx,
                fy,
            });
        }
        cx.notify();
    }

    /// A drop on bare water, WHILE PLANNING: put the hull in the draft at the
    /// drop point and post nothing.
    ///
    /// The drop fractions are unprojected HERE, locally, because the draft
    /// carries GROUND (a `Start`) and the shell is the side that holds the
    /// frame's camera and dimensions (see `frame_unproject`). A hull the
    /// exercise does not hold is added first, commanded by the operator's own
    /// seat — placing IS assigning, the same chain the binary's `DeployAssign`
    /// runs, cut to a local edit. Clearing `host` is the other half of the
    /// statement a bare-water drop makes: the hull is on the water, not aboard.
    fn stage_place_local(
        &mut self,
        unit_id: i64,
        name: &str,
        fx: f64,
        fy: f64,
        cx: &mut Context<Self>,
    ) {
        let who = match self.own_commander() {
            Ok(who) => who,
            Err(why) => {
                self.force_status = why;
                cx.notify();
                return;
            }
        };
        if !self.force.contains(unit_id) {
            let mut draft = self.force.clone();
            draft.upsert(crate::force::DraftHull {
                unit_id,
                name: name.to_string(),
                hull_number: String::new(),
                class_id: String::new(),
                start: None,
                commander_id: Some(who),
                host: None,
            });
            self.force = draft;
        }
        // The commander is SET, not merely taken for a new hull: a drafted
        // piece with nobody to steer it is the state `force::plan` refuses
        // whole, and the drop is the operator saying "this one, under me".
        self.force.set_commander(unit_id, Some(who));
        let (lat, lon) = self.frame_unproject(fx, fy);
        self.force.set_start(unit_id, Some(crate::force::Start { lat, lon }));
        self.force.set_host(unit_id, None);
        self.pending_place = None;
        self.place_down = None;
        self.force_status =
            format!("{name} staged locally \u{2014} move to preparation writes it");
        cx.notify();
    }

    /// A drop onto a host, WHILE PLANNING: put the hull aboard it in the
    /// draft and post nothing.
    ///
    /// A hull the exercise does not hold is added first, commanded by the
    /// operator's own seat, exactly as the bare-water drop does. The hull's
    /// own start is CLEARED: a hull aboard another has no position of its own,
    /// and a `Place` beside the `Embark` would be a claim the server refuses.
    fn stage_embark_local(&mut self, host: i64, child: i64, name: &str, cx: &mut Context<Self>) {
        let who = match self.own_commander() {
            Ok(who) => who,
            Err(why) => {
                self.force_status = why;
                cx.notify();
                return;
            }
        };
        if !self.force.contains(child) {
            let mut draft = self.force.clone();
            draft.upsert(crate::force::DraftHull {
                unit_id: child,
                name: name.to_string(),
                hull_number: String::new(),
                class_id: String::new(),
                start: None,
                commander_id: Some(who),
                host: None,
            });
            self.force = draft;
        }
        // Same as the bare-water drop: the commander is SET so a piece that
        // entered the exercise uncommanded is writable when the advance comes.
        self.force.set_commander(child, Some(who));
        self.force.set_host(child, Some(host));
        self.force.set_start(child, None);
        self.force_status =
            format!("{name} staged aboard {host} locally \u{2014} move to preparation writes it");
        cx.notify();
    }

    /// Embark a dragged hull aboard a placed host: latch the flight on the
    /// embark side (the answer lands on its feed, like every embark write)
    /// and say so on the picker's line too, because the drop started there.
    /// While PLANNING this stages the embark locally and posts nothing.
    ///
    /// Otherwise same local refusal as the deploy: embarkation is per-piece,
    /// and a hull outside the exercise has no embarkation row for the server
    /// to find.
    fn send_embark(&mut self, host: i64, child: i64, name: &str, cx: &mut Context<Self>) {
        if self.staging() {
            self.stage_embark_local(host, child, name, cx);
            return;
        }
        if !self.session.pieces.iter().any(|p| p.id == child) {
            self.force_status =
                format!("{name} isn't in the exercise yet \u{2014} add it first, then embark it");
            cx.notify();
            return;
        }
        self.embark_busy = true;
        self.embark_status = format!("embarking {name} aboard {host}\u{2026}");
        self.force_status = format!("embarking {name} aboard {host}\u{2026}");
        let _ = self.order_tx.send(ToBinary::EmbarkDrop { host, child });
        cx.notify();
    }

    /// Whether the held game is still being PLANNED — the one state where an
    /// edit stages locally and the Session island's advance writes the draft
    /// through.
    ///
    /// The state is the SERVER's word (the session read's `state`), never a
    /// local flag: a game another client has moved on is not planning here
    /// either, and a staged draft against it would never be written.
    fn staging(&self) -> bool {
        crate::gamestate::GameState::from_wire(&self.session.state)
            == crate::gamestate::GameState::Planning
    }

    /// The commander a drop hands a hull to: the operator's own seat.
    ///
    /// The two refusals are the console's own sentences, kept in one place so
    /// the local drop and the posting drop name the same way forward. The
    /// seat must exist (a hull with nobody to steer it is not a state Minos
    /// can hold) and it must STEER (a judge-side seat directs, it does not
    /// command — the server refuses the write outright).
    fn own_commander(&self) -> Result<i64, String> {
        let who = self.session.me;
        if self.crew_choices().iter().all(|(id, _)| *id != who) {
            // Point at the pills when there are any: a refusal with no way
            // forward is the wall the seat-me pills exist to remove.
            return Err(if who != 0 && !self.player_roles().is_empty() {
                "you hold no seat \u{2014} take a player seat above, then drop again".to_string()
            } else {
                "your account holds no seat in this room".to_string()
            });
        }
        if self.own_steers() == Some(false) {
            return Err("your seat is judge-side \u{2014} it directs, it does not command; take a player seat on the Players island".to_string());
        }
        Ok(who)
    }

    /// The map-frame pixel a ground position projects to, at the camera the
    /// last frame was rendered from. A staged hull has no fix for the binary
    /// to project, so the shell projects it here from its draft `Start`.
    fn frame_project(&self, lat: f64, lon: f64) -> (f64, f64) {
        project_frame_px(
            lat,
            lon,
            self.map_center,
            self.map_render_zoom,
            f64::from(self.map_dims.0),
            f64::from(self.map_dims.1),
        )
    }

    /// The ground a drop's view fractions point at: the inverse of
    /// [`Self::frame_project`], and the shell-side half of what the binary
    /// does for a non-staged deploy. The fractions are of the map VIEW, and
    /// the frame fills it (`ObjectFit::Fill`), so the frame's own dimensions
    /// are the scale.
    fn frame_unproject(&self, fx: f64, fy: f64) -> (f64, f64) {
        let (mw, mh) = (f64::from(self.map_dims.0), f64::from(self.map_dims.1));
        unproject_frame_px(
            fx.clamp(0.0, 1.0) * mw,
            fy.clamp(0.0, 1.0) * mh,
            self.map_center,
            self.map_render_zoom,
            mw,
            mh,
        )
    }

    /// The glyph a hull draws as, resolved from the taxonomy the binary built
    /// — the same symbol its live marker carries. A hull the tree does not
    /// name falls back to the Dot, the unknown-ship shape `map_symbol_shape`
    /// itself uses.
    fn fleet_symbol(&self, unit_id: i64) -> MapSymbolShape {
        fn walk(groups: &[FleetTaxGroup], id: &str) -> Option<MapSymbolShape> {
            for g in groups {
                for c in &g.cats {
                    for t in &c.types {
                        for cl in &t.classes {
                            for l in &cl.units {
                                if l.id == id {
                                    return Some(l.symbol);
                                }
                            }
                        }
                    }
                }
            }
            None
        }
        walk(&self.fleet_tax.groups, &unit_id.to_string()).unwrap_or(MapSymbolShape::Dot)
    }

    /// The STAGED hulls that sit at a starting position and have no marker on
    /// the wire yet: the map draws one planned marker per hull, dashed and
    /// dimmed, so a position the operator set while planning is visible
    /// before the advance writes it.
    ///
    /// A hull whose fix is already on the wire is SKIPPED: the live marker is
    /// the truth about where it is, and two frames over one hull would be two
    /// answers. Projected from the draft's own lat/lon at the frame's camera,
    /// because a staged hull has no fix for the binary to project.
    fn planned_marks(&self) -> Vec<(f64, f64, MapSymbolShape)> {
        let (mw, mh) = (f64::from(self.map_dims.0), f64::from(self.map_dims.1));
        if mw <= 0.0 || mh <= 0.0 {
            return Vec::new();
        }
        self.force
            .hulls()
            .filter_map(|h| {
                let start = h.start?;
                if self.live.iter().any(|m| m.id == h.unit_id.to_string()) {
                    return None;
                }
                let (x, y) = project_frame_px(
                    start.lat,
                    start.lon,
                    self.map_center,
                    self.map_render_zoom,
                    mw,
                    mh,
                );
                Some((x, y, self.fleet_symbol(h.unit_id)))
            })
            .collect()
    }

    /// A drag started or ended elsewhere (GPUI owns the state; the tick
    /// polls it). The fleet picker hides for the drag's lifetime so the
    /// drop can land anywhere on the map — a modal over the middle of the
    /// ocean is a wall between the hull and its water.
    fn set_dragging(&mut self, dragging: bool, cx: &mut Context<Self>) {
        if self.dragging != dragging {
            self.dragging = dragging;
            cx.notify();
        }
    }

    /// Arm click-to-place for a drafted hull: the next map click that stays
    /// put deploys it there. The drag needs no arming — its release IS the
    /// click — so this is purely the pointer-impaired and harness path.
    fn arm_place(&mut self, unit_id: i64, name: &str, cx: &mut Context<Self>) {
        self.pending_place = Some(unit_id);
        self.place_down = None;
        self.force_status = format!("click the map to place {name} \u{2014} or drag the row");
        cx.notify();
    }

    /// The host a drop lands on, if it lands on one: the nearest PLACED
    /// piece's marker within 20 map px of the drop fractions — the live
    /// marker when there is one, and otherwise a STAGED hull's projected
    /// position. Markers arrive in the frame's pixel space (`map_dims`), and
    /// fractions scale back into it — the same fraction round-trip the deploy
    /// side uses, in reverse. Unplaced pieces are not hosts: there is no
    /// marker for a hull with no position, and a drop near one's roster row
    /// is a deploy.
    ///
    /// A staged hull must be considered or a drop onto a hull the operator
    /// just placed would fall in the water beside it: while planning that hull
    /// has a draft `Start` and no fix, so nothing but this projection can say
    /// where it is.
    fn drop_host_at(&self, fx: f64, fy: f64) -> Option<i64> {
        let (mw, mh) = (f64::from(self.map_dims.0), f64::from(self.map_dims.1));
        if mw <= 0.0 || mh <= 0.0 {
            return None;
        }
        let (px, py) = (fx * mw, fy * mh);
        let placed: Vec<i64> = self
            .session
            .pieces
            .iter()
            .filter(|p| p.placed)
            .map(|p| p.id)
            .collect();
        let mut best: Option<(f64, i64)> = None;
        let mut consider = |d: f64, id: i64| {
            let closer = match best {
                None => true,
                Some((bd, _)) => d < bd,
            };
            if d <= 20.0 && closer {
                best = Some((d, id));
            }
        };
        for m in &self.live {
            if let Ok(id) = m.id.parse::<i64>() {
                if placed.contains(&id) {
                    consider((m.x - px).hypot(m.y - py), id);
                }
            }
        }
        for h in self.force.hulls() {
            let Some(start) = h.start else { continue };
            let (x, y) = self.frame_project(start.lat, start.lon);
            consider((x - px).hypot(y - py), h.unit_id);
        }
        best.map(|(_, id)| id)
    }

    /// Plan the draft against the server and post the whole plan.
    ///
    /// `Err` carries `ForceGap`'s own sentence and sends nothing — including
    /// when the local draft was already ahead of it, because a plan that
    /// half-commits is the failure the gap exists to prevent.
    fn sync_force(&mut self) -> Result<(), String> {
        let live = self.live_hulls();
        match crate::force::plan(&self.force, &live) {
            Ok(writes) if writes.is_empty() => Ok(()),
            Ok(writes) => {
                self.feed(format!(
                    "force synced — {} write(s)",
                    writes.len()
                ));
                let _ = self.order_tx.send(ToBinary::SyncForce { writes });
                Ok(())
            }
            Err(gap) => Err(gap.to_string()),
        }
    }

    /// Minos's own roster, as the draft's `LiveSeat` mirror.
    fn live_seats(&self) -> Vec<crate::roster::LiveSeat> {
        self.session
            .people
            .iter()
            .map(|p| crate::roster::LiveSeat {
                user_id: p.id,
                user_name: p.name.clone(),
                role_id: p.role_id,
            })
            .collect()
    }

    /// The player-side seats: roles that may command hulls. The server
    /// refuses a judge-side commander outright ("a judge-side participant
    /// does not command units"), so the seat-me pills offer exactly these —
    /// an organizer pill would seat you beautifully and leave every drop
    /// refused. Empty when the game defines no player role, and then
    /// seating stays the Players island's job.
    fn player_roles(&self) -> Vec<RoleRow> {
        self.session
            .roles
            .iter()
            .filter(|r| !r.judge)
            .cloned()
            .collect()
    }

    /// Whether the operator's own seat steers: `None` unseated, `Some`
    /// whether the held role is judge-side. The drop needs a steering
    /// seat — organizers direct, they do not command.
    fn own_steers(&self) -> Option<bool> {
        let held = self.session.people.iter().find(|p| p.id == self.session.me)?;
        let role = self.session.roles.iter().find(|r| r.id == held.role_id)?;
        Some(!role.judge)
    }

    /// The operator's own display name: the directory's row for this seat,
    /// or the sign-in identifier when the room holds no row for them — and
    /// an unseated operator is exactly who needs this. Feeds `stage_seat`,
    /// whose call-sign rule derives from the name.
    fn own_name(&self) -> String {
        self.session
            .directory
            .iter()
            .find(|d| d.id == self.session.me)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| self.login_identifier.clone())
    }

    /// Seat an account as a role, and write it through.
    ///
    /// The role is TAKEN, not checked, for the same reason the force's
    /// commander is: what a role MEANS is the server's business, and whether a
    /// staged seat is a seat, a role change or a no-op is `roster::plan`'s.
    /// A second copy of that rule here would be the copy that drifts.
    ///
    /// WHILE PLANNING this stages and returns; the Session island's advance
    /// writes it through. Otherwise it posts at once, because no advance is
    /// left to wait for.
    fn stage_seat(&mut self, user_id: i64, name: &str, role: &RoleRow) -> Result<(), String> {
        self.roster_draft.stage(crate::roster::StagedSeat {
            user_id,
            user_name: name.to_string(),
            role_id: role.id,
            role_name: role.name.clone(),
            // Minos requires a call sign on a seat, and the account's own name
            // is the only one this build can source: the console asks the
            // operator for it, and a seat form that silently invented one would
            // be a call sign nobody chose.
            call_sign: name.chars().take(8).collect::<String>().to_uppercase(),
            judge: role.judge,
        });
        if self.staging() {
            return Ok(());
        }
        self.sync_roster()
    }

    /// Take a seated person off the roster, and write it through.
    fn stage_unseat(&mut self, user_id: i64) -> Result<(), String> {
        let on_server = self.session.people.iter().any(|p| p.id == user_id);
        self.roster_draft.remove(user_id, on_server);
        if self.staging() {
            return Ok(());
        }
        self.sync_roster()
    }

    /// Plan the roster draft against the server and post the whole plan.
    fn sync_roster(&mut self) -> Result<(), String> {
        let live = self.live_seats();
        let writes = crate::roster::plan(&self.roster_draft, &live);
        if writes.is_empty() {
            return Ok(());
        }
        self.feed(format!(
            "roster synced — {} write(s)",
            writes.len()
        ));
        let _ = self.order_tx.send(ToBinary::SyncRoster { writes });
        Ok(())
    }

    /// Control's draft, one step along its rail.
    ///
    /// The kernel clamps (`view::factor_nudge`), so a button at the rail is a
    /// no-op and the draft can never hold a value the server would refuse.
    /// This edits the DRAFT only: nothing is written until Apply, which is the
    /// console's own arrangement — a DragValue that wrote on every drag would
    /// be one PATCH per pixel.
    fn nudge_factor(&mut self, step: f64) {
        let read = self.session.facts.time_factor;
        let current = self.factor_draft.unwrap_or(read);
        self.factor_draft = Some(view::factor_nudge(current, step));
        self.factor_status.clear();
    }

    /// Whether Control's draft differs from what Minos is running.
    fn factor_dirty(&self) -> bool {
        match self.factor_draft {
            Some(d) => (d - self.session.facts.time_factor).abs() > 1e-9,
            None => false,
        }
    }

    /// Write the drafted multiplier. Refused when nothing changed, so the
    /// button cannot post a no-op the server would have to decide about.
    fn send_factor(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.factor_dirty() || self.factor_busy {
            return false;
        }
        let Some(factor) = self.factor_draft else {
            return false;
        };
        self.factor_busy = true;
        self.factor_status = "applying\u{2026}".to_string();
        self.feed(format!(
            "clock factor asked to — {:.0}x",
            factor
        ));
        let _ = self.order_tx.send(ToBinary::SetTimeFactor { factor });
        cx.notify();
        true
    }

    /// Apply the re-read session a multiplier answer returned. The draft
    /// follows Minos rather than the draft — the write's whole point is that
    /// the readout is the server's.
    fn apply_factor(&mut self, session: SessionView) {
        self.factor_busy = false;
        self.session = session;
        self.factor_draft = None;
        self.factor_status.clear();
    }

    /// Pause or resume the scenario clock — the console's
    /// `pause_or_resume_minos`, as a message. Held resumes, otherwise
    /// pauses, unknown pauses: the console's rule, decided shell-side from
    /// the last clock answer, because the binary holds the token but never
    /// the clock. The write IS the read — no clock GET exists — and the
    /// answer is the GameClock to display. A remembered denial refuses
    /// locally with the grant sentence instead of spending a POST the
    /// server has already refused.
    ///
    /// Returns whether a request went out, so a test exercises exactly
    /// what the click does.
    fn send_set_paused(&mut self, cx: &mut Context<Self>) -> bool {
        if self.pause_busy {
            return false;
        }
        if self.clock_denied {
            self.pause_status = CLOCK_GRANT.to_string();
            cx.notify();
            return false;
        }
        if self.session.game_id == 0 {
            return false;
        }
        let paused = self.clock_running.unwrap_or(true);
        self.pause_busy = true;
        self.pause_status = if paused {
            "pausing\u{2026}"
        } else {
            "resuming\u{2026}"
        }
        .to_string();
        let _ = self.order_tx.send(ToBinary::SetPaused { paused });
        cx.notify();
        true
    }

    /// Apply a clock write's answer — the console's `SetupDone::Clock` arm,
    /// cut to what this shell holds. The status sentence is the console's
    /// verbatim; the held `running` is what the next toggle decides from.
    /// An answer proves the grant, so a denial clears — it belonged to an
    /// older hold or a since-changed matrix. A fresh denial latches and
    /// names the grant; anything else fails loudly on the line and keeps
    /// the held state.
    fn apply_clock_answer(&mut self, outcome: Result<crate::backend::GameClock, ClockError>) {
        self.pause_busy = false;
        match outcome {
            Ok(clock) => {
                let verb = if clock.running { "resume" } else { "pause" };
                self.clock_running = Some(clock.running);
                self.clock_denied = false;
                let line = format!(
                    "Scenario clock {verb}: assumed {} \u{00b7} {} \u{00b7} {} \u{00b7} {}x",
                    clock.assumed_now.as_deref().unwrap_or("\u{2014}"),
                    if clock.running { "running" } else { "held" },
                    if clock.accepting_actions {
                        "accepting orders"
                    } else {
                        "orders closed"
                    },
                    clock.time_factor,
                );
                self.feed(line.clone());
                self.pause_status = line;
            }
            Err(ClockError::Denied) => {
                self.clock_denied = true;
                self.pause_status = CLOCK_GRANT.to_string();
            }
            Err(ClockError::Failed(why)) => {
                self.pause_status = why;
            }
        }
    }

    /// Read the open host's passengers — the console's `ensure_embarked`,
    /// as a message. Cached hosts are kept, never re-read: the modal shows
    /// one hull, and a read the operator already holds is not worth a
    /// second request. The binary owns the GET; the shell only asks,
    /// naming the host the modal drew.
    ///
    /// Returns whether a request went out, so a test exercises exactly
    /// what the open does.
    fn send_embarked(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(host) = self.embark_host else {
            return false;
        };
        if self.embark_busy || self.embarked.contains_key(&host) {
            return false;
        }
        if self.session.game_id == 0 {
            self.embark_status = "hold a session first".to_string();
            cx.notify();
            return false;
        }
        self.embark_busy = true;
        self.embark_status = "reading who is aboard\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::ReadEmbarked { host });
        cx.notify();
        true
    }

    /// Apply one host's passengers: filed under the answer's own host, not
    /// the open one — answers can overtake opens, and one hull's passengers
    /// under another's is the misfile this shape exists to prevent. A
    /// refusal lands on the modal's status line and keeps whatever the
    /// cache held.
    fn apply_embarked(&mut self, outcome: Result<(i64, Vec<crate::backend::EmbarkedUnit>), String>) {
        self.embark_busy = false;
        match outcome {
            Ok((host, units)) => {
                self.embarked.insert(host, units);
                self.embark_status.clear();
            }
            Err(why) => {
                self.embark_status = why;
            }
        }
    }

    /// The three row verbs refuse the same two ways, in the console's own
    /// sentences: a request of any kind in flight (`setup_op` is one slot,
    /// and a second press while one flies would unmap the answer), or no
    /// held game. The latch is shared for the same reason.
    fn embark_write_guarded(&mut self, verb: &'static str, cx: &mut Context<Self>) -> bool {
        if self.embark_busy {
            self.embark_status = format!("{verb} refused: another request is in flight");
            cx.notify();
            return false;
        }
        if self.session.game_id == 0 {
            self.embark_status = format!("{verb} refused: no held game");
            cx.notify();
            return false;
        }
        true
    }

    /// Take one piece out of its host — the console's `unembark`, as a
    /// message. Returns whether a request went out, so a test exercises
    /// exactly what the click does.
    fn send_unembark(&mut self, host: i64, unit: i64, cx: &mut Context<Self>) -> bool {
        if !self.embark_write_guarded("unembark", cx) {
            return false;
        }
        self.embark_busy = true;
        self.embark_status = "unembarking\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::UnembarkUnit { host, unit });
        cx.notify();
        true
    }

    /// Let one embarked piece act — the console's `embark_clear`, as a
    /// message. Returns whether a request went out.
    fn send_clear_aboard(&mut self, host: i64, unit: i64, cx: &mut Context<Self>) -> bool {
        if !self.embark_write_guarded("clear", cx) {
            return false;
        }
        self.embark_busy = true;
        self.embark_status = "clearing\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::ClearAboard { host, unit });
        cx.notify();
        true
    }

    /// Take the host's permission back — the console's `embark_revoke`,
    /// as a message. Returns whether a request went out.
    fn send_revoke_clearance(&mut self, host: i64, unit: i64, cx: &mut Context<Self>) -> bool {
        if !self.embark_write_guarded("revoke", cx) {
            return false;
        }
        self.embark_busy = true;
        self.embark_status = "revoking\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::RevokeClearance { host, unit });
        cx.notify();
        true
    }

    /// Apply a write's answer: the host's contents after the act, filed
    /// under the answer's own host, plus the capacity warnings, if any —
    /// the console's `EmbarkChanged` landing minus the force-draft fixup
    /// (a freed hull was a piece before it embarked and still is one, so
    /// the next session read picks it up with no local staging). The rows
    /// themselves are the success feedback, so the line clears; a refusal
    /// lands on the line and keeps the cache.
    fn apply_embark_write(
        &mut self,
        outcome: Result<(i64, crate::backend::EmbarkationResult), String>,
    ) {
        self.embark_busy = false;
        match outcome {
            Ok((host, result)) => {
                self.embarked.insert(host, result.units);
                self.embark_over.insert(host, result.over_capacity);
                self.embark_status.clear();
            }
            Err(why) => {
                self.embark_status = why;
            }
        }
    }

    /// Whether the readiness gate is clear.
    ///
    /// An unpublished checklist is CLEAR: a server that never published the
    /// check cannot be waited out, so retrying it would strand the exercise.
    /// That is the console's `readiness_gate_clear`, and it is deliberately not
    /// `blockers.is_empty()`.
    fn readiness_clear(&self) -> bool {
        match &self.session.readiness {
            None => true,
            Some(r) => r.can_execute || r.blockers.is_empty(),
        }
    }

    /// Leave preparation. Ungated by role on purpose (`review_body`'s note):
    /// a button that hid itself would strand the session whose Game Master is
    /// named something else.
    fn advance_execution(&mut self, cx: &mut Context<Self>) -> bool {
        if self.advance_busy {
            return false;
        }
        if self.session.game_id == 0 {
            return false;
        }
        self.advance_busy = true;
        self.advance_status = "advancing\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::AdvanceExecution);
        cx.notify();
        true
    }

    /// LEAVE PLANNING: write the whole staged draft through and move the game
    /// to preparation.
    ///
    /// The drafts and the live mirrors they are diffed against cross on the
    /// wire, because the binary holds the token but never the shell's session
    /// — the answer is a re-read session on the shared create feed, under
    /// `prepare_busy`, like every other session-replacing write here.
    ///
    /// Returns whether a request went out, so a test exercises exactly what
    /// the click does.
    fn prepare_advance(&mut self, cx: &mut Context<Self>) -> bool {
        if self.prepare_busy {
            return false;
        }
        if self.session.game_id == 0 {
            return false;
        }
        self.prepare_busy = true;
        self.prepare_status = "moving the plan to preparation\u{2026}".to_string();
        let live_hulls = self.live_hulls();
        let live_seats = self.live_seats();
        let _ = self.order_tx.send(ToBinary::AdvancePreparation {
            force: self.force.clone(),
            live_hulls,
            roster: self.roster_draft.clone(),
            live_seats,
        });
        cx.notify();
        true
    }

    /// Leave execution. Organizer-gated by placement, not by this send:
    /// the console pushes `z.clock` for organizers only and never checks
    /// the seat in `close_game` — a player station that ended the exercise
    /// would end everyone else's too, and the server arbitrates the seat
    /// loudly on top. The binary owns the transition; the shell only asks,
    /// naming nothing — the held game is the binary's to know.
    ///
    /// Returns whether a request went out, so a test exercises exactly
    /// what the click does.
    fn send_end_session(&mut self, cx: &mut Context<Self>) -> bool {
        if self.end_busy {
            return false;
        }
        self.end_busy = true;
        self.end_status = "ending the session\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::EndSession);
        cx.notify();
        true
    }

    /// Hold the re-read session an end-session answer returned: closure
    /// arrives read back, and the column recomposes to the debrief around
    /// it. A method rather than inline handling so a test holds exactly
    /// what the drain holds.
    fn apply_end(&mut self, session: SessionView) {
        self.end_busy = false;
        self.end_status.clear();
        self.session = session;
    }

    /// Hold the picked game — the picker's verb, as a message. The binary
    /// owns the re-read; the shell only names the id the row drew, never a
    /// cached guess.
    ///
    /// Returns whether a request went out, so a test exercises exactly
    /// what the click does.
    fn send_hold(&mut self, game_id: i64, cx: &mut Context<Self>) -> bool {
        if self.hold_busy {
            return false;
        }
        self.hold_busy = true;
        self.hold_status = "holding the session\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::HoldSession { game_id });
        cx.notify();
        true
    }

    /// Hold the re-read session a hold answer returned: the picked game
    /// arrives read back rather than assumed, and the column recomposes
    /// around it. A method rather than inline handling so a test holds
    /// exactly what the drain holds.
    fn apply_hold(&mut self, session: SessionView) {
        self.hold_busy = false;
        self.hold_status.clear();
        self.session = session;
        // A new hold REPLACES the drafts' reference: seed them from the game
        // just held, so every later edit and the advance diff against what
        // Minos actually holds rather than against an empty draft that would
        // read every existing piece as a removal. Nothing is staged at a hold,
        // so there is no work to discard.
        self.reseed_drafts();
    }

    /// Declare or withdraw this seat's readiness — the console's
    /// `set_own_readiness`, as a message.
    ///
    /// Returns false (and sends nothing) for the three non-sends: no seat, a
    /// judge seat (exempt — the server 409s judges loudly), and asking for
    /// the state already held. The same rule the row's buttons enforce, kept
    /// in one place so a test exercises exactly what the click does.
    fn set_readiness(&mut self, ready: bool, cx: &mut Context<Self>) -> bool {
        let ok = self.own_seat().is_some_and(|p| !p.judge && p.ready != ready);
        if !ok || self.readiness_busy {
            return false;
        }
        self.readiness_busy = true;
        self.readiness_status = if ready {
            "declaring\u{2026}"
        } else {
            "withdrawing\u{2026}"
        }
        .to_string();
        self.feed(format!(
            "readiness — {}",
            if ready { "declared" } else { "withdrawn" }
        ));
        let _ = self.order_tx.send(ToBinary::SetReadiness { ready });
        cx.notify();
        true
    }

    /// Push one human line to the Log island's feed.
    ///
    /// The console's `feed()` verbatim, cap included: append, and drop from
    /// the front past 30. It also echoes to stderr there, and does here,
    /// because a line an operator can read in the island is a line worth
    /// having in the run log too — one write, two readers, not two sources.
    fn feed(&mut self, msg: String) {
        eprintln!("{msg}");
        self.log_feed.push_back(msg);
        while self.log_feed.len() > 30 {
            self.log_feed.pop_front();
        }
    }

    /// LOG — what happened, as this client was told it.
    ///
    /// The console's `z.log`, whose feed half is `event_feed` behind a 160pt
    /// scroller, with `order_warning` above it when the last order was
    /// refused. Newest last, because a feed is read downward and the line
    /// that matters is the one at the bottom.
    fn log_body(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let small = |t: String| {
            div()
                .font_family("Inter")
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        let mut col = div().flex().flex_col().w_full();
        if self.log_feed.is_empty() {
            col = col.child(small(
                "No events yet \u{2014} orders and arrivals land here.".to_string(),
            ));
        } else {
            for line in &self.log_feed {
                col = col.child(small(line.clone()));
            }
        }
        col = col.child(separator());
        // What is NOT here, said rather than left as a short island. The
        // console's Log is two surfaces: this feed, and below a rule the
        // session's journal files with a replay slider that drives map
        // ghosts. The journals are files on disk and the ghosts are a replay
        // of them, so neither has a read route -- the same reason Groups is
        // absent, and worth naming so a short island does not read as an
        // unfinished one.
        col = col.child(small(
            "Journals and map replay are not carried \u{2014} local files, no read route."
                .to_string(),
        ));
        col.into_any_element()
    }

    /// Read a Closure timeline page: the Assessment island's Timeline tab,
    /// the console's `load_timeline`. `append` carries the cursor's next
    /// page (`load more`); false reloads from the first page under the
    /// current filters. The binary owns the GET — the shell only asks — so
    /// this is one message on the sink, gated the way every read here is: a
    /// request already in flight sends nothing.
    ///
    /// The numeric filters parse HERE, like the console: a personnel or unit
    /// id that is not a number refuses before any request, in the console's
    /// own sentence, rather than becoming a server 400. The window strings
    /// pass through trimmed-or-absent; the server owns parsing a timestamp
    /// and answers a bad one loudly.
    ///
    /// Returns whether a request went out — false for a refused id or a
    /// request already in flight, so a test exercises exactly what the click
    /// does.
    fn send_timeline(&mut self, append: bool, cx: &mut Context<Self>) -> bool {
        if self.timeline_busy {
            return false;
        }
        let parse_id = |raw: &str, what: &str| -> Result<Option<i64>, String> {
            let t = raw.trim();
            if t.is_empty() {
                return Ok(None);
            }
            t.parse::<i64>()
                .map(Some)
                .map_err(|_| format!("{what} must be a numeric id"))
        };
        let personnel = match parse_id(&self.timeline_personnel, "personnel") {
            Ok(p) => p,
            Err(e) => {
                self.timeline_status = e;
                cx.notify();
                return false;
            }
        };
        let unit = match parse_id(&self.timeline_unit, "unit") {
            Ok(u) => u,
            Err(e) => {
                self.timeline_status = e;
                cx.notify();
                return false;
            }
        };
        let trim = |s: &str| {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        };
        let from = trim(&self.timeline_from);
        let to = trim(&self.timeline_to);
        let source = self.timeline_source.clone();
        // A load-more carries the cursor; a reload never does — a cursor does
        // not cross a filter change, which is the console's rule.
        let cursor = if append {
            self.session.timeline_cursor.clone()
        } else {
            None
        };
        self.timeline_busy = true;
        self.timeline_status = if append {
            "loading more\u{2026}".to_string()
        } else {
            "reading timeline\u{2026}".to_string()
        };
        let _ = self.order_tx.send(ToBinary::Timeline {
            append,
            source,
            personnel,
            unit,
            from,
            to,
            cursor,
        });
        true
    }

    /// The value of filter field `idx`, for the render.
    fn timeline_value(&self, idx: usize) -> String {
        match idx {
            0 => self.timeline_personnel.clone(),
            1 => self.timeline_unit.clone(),
            2 => self.timeline_from.clone(),
            _ => self.timeline_to.clone(),
        }
    }

    fn timeline_value_mut(&mut self, idx: usize) -> &mut String {
        match idx {
            0 => &mut self.timeline_personnel,
            1 => &mut self.timeline_unit,
            2 => &mut self.timeline_from,
            _ => &mut self.timeline_to,
        }
    }

    /// One timeline filter field, hand-rolled like the composer's: a focus
    /// handle, a keystroke handler and a value. `idx` is 0 personnel, 1
    /// unit, 2 from, 3 to. The mono face because these are ids and RFC 3339
    /// stamps, not prose.
    fn timeline_field(
        &mut self,
        idx: usize,
        hint: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let handle = match self.timeline_handles[idx].clone() {
            Some(h) => h,
            None => {
                let h = cx.focus_handle();
                self.timeline_handles[idx] = Some(h.clone());
                h
            }
        };
        let text = self.timeline_value(idx);
        let empty = text.is_empty();
        let shown = if empty { hint.to_string() } else { text.to_string() };
        let focused = self.timeline_focus == idx;
        div()
            .id(("timeline-field", idx))
            .track_focus(&handle)
            .cursor(CursorStyle::IBeam)
            .h(px(AUTH_FIELD_H))
            .flex_1()
            .px(px(6.0))
            .rounded(px(CONTROL_RADIUS))
            .bg(rgba(DEEP_WELL))
            .border(px(1.0))
            .border_color(rgba(if focused { RADAR_CYAN } else { HAIRLINE_SLATE }))
            .flex()
            .items_center()
            .font_family(view::FACE_MACHINE)
            .text_size(px(TEXT_BODY_PT))
            .text_color(rgba(if empty { BODY_SILVER } else { MAP_INK }))
            .on_mouse_down(MouseButton::Left, cx.listener(move |view, _, window, cx| {
                window.focus(&handle);
                view.timeline_focus = idx;
                cx.notify();
            }))
            .on_key_down(cx.listener(move |view, ev: &KeyDownEvent, window, cx| {
                view.timeline_key(idx, &ev.keystroke, window, cx);
            }))
            .child(shown)
            .into_any_element()
    }

    /// The keystroke half of a filter field: printable characters append,
    /// backspace removes, tab walks the four fields, Enter applies. `key_char`
    /// and not `key`, the card's rule — the name is the keycap legend and
    /// differs by layout.
    fn timeline_key(
        &mut self,
        idx: usize,
        k: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match k.key.as_str() {
            "enter" => {
                self.send_timeline(false, cx);
                return;
            }
            "backspace" => {
                self.timeline_value_mut(idx).pop();
            }
            "tab" => {
                let next = (idx + 1) % 4;
                self.timeline_focus = next;
                if let Some(h) = self.timeline_handles[next].clone() {
                    window.focus(&h);
                }
            }
            _ => {
                if let Some(text) = k
                    .key_char
                    .as_deref()
                    .filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
                {
                    self.timeline_value_mut(idx).push_str(text);
                }
            }
        }
        cx.notify();
    }

    /// One compose field: a focus handle, a keystroke handler and a value,
    /// in the machine face because a score and filed words are both read
    /// back verbatim. Enter sends (record, or file-or-save by whether a row
    /// is under revision); there is no tab walk, these fields stand alone
    /// on their panes. The fourth copy of the hand-rolled single line the
    /// card, the composer and the timeline filters established — one helper
    /// serves both fields because they share everything but the slot.
    fn write_field(
        &mut self,
        which: WriteField,
        hint: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = match which {
            WriteField::Score => "judge-score",
            WriteField::Body => "review-body",
        };
        let handle = {
            let slot = match which {
                WriteField::Score => &mut self.judge_score_handle,
                WriteField::Body => &mut self.review_body_handle,
            };
            match slot.clone() {
                Some(h) => h,
                None => {
                    let h = cx.focus_handle();
                    *slot = Some(h.clone());
                    h
                }
            }
        };
        let (value, focused) = match which {
            WriteField::Score => (self.judge_score.clone(), self.judge_score_focused),
            WriteField::Body => (self.review_body.clone(), self.review_body_focused),
        };
        let empty = value.is_empty();
        let shown = if empty { hint.to_string() } else { value };
        div()
            .id(id)
            .track_focus(&handle)
            .cursor(CursorStyle::IBeam)
            .h(px(AUTH_FIELD_H))
            .w_full()
            .px(px(6.0))
            .rounded(px(CONTROL_RADIUS))
            .bg(rgba(DEEP_WELL))
            .border(px(1.0))
            .border_color(rgba(if focused { RADAR_CYAN } else { HAIRLINE_SLATE }))
            .flex()
            .items_center()
            .font_family(view::FACE_MACHINE)
            .text_size(px(TEXT_BODY_PT))
            .text_color(rgba(if empty { BODY_SILVER } else { MAP_INK }))
            .on_mouse_down(MouseButton::Left, cx.listener(move |view, _, window, cx| {
                window.focus(&handle);
                match which {
                    WriteField::Score => view.judge_score_focused = true,
                    WriteField::Body => view.review_body_focused = true,
                }
                cx.notify();
            }))
            .on_key_down(cx.listener(move |view, ev: &KeyDownEvent, _, cx| {
                view.write_key(which, &ev.keystroke, cx);
            }))
            .child(shown)
            .into_any_element()
    }

    /// The keystroke half of a compose field: printable characters append,
    /// backspace removes, Enter sends. `key_char` and not `key`, the card's
    /// rule — the name is the keycap legend and differs by layout.
    fn write_key(&mut self, which: WriteField, k: &gpui::Keystroke, cx: &mut Context<Self>) {
        match k.key.as_str() {
            "enter" => {
                match which {
                    WriteField::Score => {
                        self.send_judgement(cx);
                    }
                    WriteField::Body => {
                        if self.review_editing.is_some() {
                            self.send_revise(cx);
                        } else {
                            self.send_review(cx);
                        }
                    }
                }
                return;
            }
            "backspace" => match which {
                WriteField::Score => {
                    self.judge_score.pop();
                }
                WriteField::Body => {
                    self.review_body.pop();
                }
            },
            _ => {
                if let Some(text) = k
                    .key_char
                    .as_deref()
                    .filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
                {
                    match which {
                        WriteField::Score => self.judge_score.push_str(text),
                        WriteField::Body => self.review_body.push_str(text),
                    }
                }
            }
        }
        cx.notify();
    }

    /// ASSESSMENT — the debrief: what the exercise was, read back after.
    ///
    /// The console's `z.assessment`, whose Summary tab is a read-back of the
    /// game's own shape once it is over. Four of the console's five tabs are
    /// carried — Summary, Timeline, Judgements, Reviews — and Transcript is
    /// not: it is a local trace with no read. The Judgements and Reviews
    /// tabs carry their writes too: record arms on a judge-side seat, file
    /// on anyone seated, revise on the row's own authorship.
    /// One timeline event as a line. The console's `timeline_line` VERBATIM,
    /// because the shape of each event's `data` is Minos' and guessing at a
    /// field's name is how a panel ends up printing `null`.
    ///
    /// `assumed_at` is an RFC 3339 stamp and only the clock half is wanted, so
    /// the date is cut rather than reformatted: the timeline is read inside one
    /// exercise, and a date on every row is a date that says nothing.
    fn timeline_line(e: &crate::backend::TimelineEvent) -> String {
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
                "{t} {} \u{2192} {} (by {})",
                str_of("from_state"),
                str_of("to_state"),
                d["changed_by"].as_i64().unwrap_or(0)
            ),
            "clock" => {
                let note = str_of("note");
                let note_seg = if note.is_empty() {
                    String::new()
                } else {
                    format!(" \u{00b7} {note}")
                };
                let by_seg = match d["changed_by"].as_i64().unwrap_or(0) {
                    0 => String::new(),
                    by => format!(" (by {by})"),
                };
                format!("{t} clock \u{2192} {}x{note_seg}{by_seg}", num("factor"))
            }
            "order" => format!(
                "{t} order {} {:.0}\u{00b0} @ {:.0} kn{} @ ({:.4}, {:.4})",
                str_of("unit_name"),
                num("heading_deg"),
                num("speed_kn"),
                if d["was_clamped"].as_bool().unwrap_or(false) {
                    " \u{00b7} clamped"
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
                    if cited.is_empty() {
                        String::new()
                    } else {
                        format!(" \u{00b7} on {cited}")
                    }
                )
            }
            other => format!("{t} {other}"),
        }
    }

    /// ASSESSMENT \u{2014} the debrief: what the exercise was, read back after.
    ///
    /// The console's `z.assessment`, and now all four of its tabs that this
    /// shell can carry. The strip is the console's own five reduced to four:
    /// Transcript is a local trace with no route, so its button is not drawn
    /// \u{2014} a tab that opens an empty pane is scaffolding.
    ///
    /// Every pane is a READ plus its writes, both carried now: recording a
    /// judgement arms on a judge-side seat, filing a review on anyone
    /// seated, revising on the row's own authorship — and every refusal,
    /// client or server, lands on the pane's status line.
    fn assessment_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let small = |t: String| {
            div()
                .font_family("Inter")
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(t)
        };
        const TABS: [&str; 4] = ["Summary", "Timeline", "Judgements", "Reviews"];
        let mut col = div().flex().flex_col().w_full();
        // The strip. One row, one height, the picked tab in the machine face:
        // the Settings panel's radio pairs in a row, and the reason it is four
        // small buttons rather than a segmented control is that a segmented
        // control is one widget with a moving part and this is four verbless
        // targets \u{2014} nothing to arm, nothing to get stuck in.
        let mut strip = div().flex().flex_row().gap(px(6.0)).pb(px(6.0));
        for (i, name) in TABS.iter().enumerate() {
            let picked = self.assessment_tab == i;
            strip = strip.child(
                div()
                    .id(("assessment-tab", i))
                    .cursor_pointer()
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .when(picked, |d| d.bg(rgba(PANEL_SLATE)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.assessment_tab = i;
                        cx.notify();
                    }))
                    // The picked tab is in the bright ink, not only on a
                    // tint: a fill this close to the body reads as a seam in
                    // the first capture, and "which tab am I on" is the one
                    // question a tab strip exists to answer.
                    .child(
                        div()
                            .font_family("Inter")
                            .text_size(px(11.0))
                            .text_color(rgba(if picked { MAP_INK } else { BODY_SILVER }))
                            .child((*name).to_string()),
                    ),
            );
        }
        col = col.child(strip);
        col = col.child(separator());
        col = match self.assessment_tab {
            1 => {
                // The filters, the console's `timeline_ui` cut to this
                // shell's grammar: a source row, two field rows, and the
                // verbs. A source change reloads from the first page — a
                // cursor never crosses a filter change.
                const SOURCES: [Option<&str>; 6] = [
                    None,
                    Some("transition"),
                    Some("clock"),
                    Some("order"),
                    Some("message"),
                    Some("judgement"),
                ];
                let mut source_row = div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(6.0))
                    .items_center()
                    .child(small("source:".to_string()));
                for (i, want) in SOURCES.iter().enumerate() {
                    let picked = self.timeline_source.as_deref() == *want;
                    let name = want.unwrap_or("All").to_string();
                    let val = *want;
                    source_row = source_row.child(
                        div()
                            .id(("timeline-source", i))
                            .cursor_pointer()
                            .px(px(6.0))
                            .py(px(2.0))
                            .rounded(px(3.0))
                            .when(picked, |d| d.bg(rgba(PANEL_SLATE)))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                if view.timeline_source.as_deref() != val {
                                    view.timeline_source = val.map(|v| v.to_string());
                                    view.send_timeline(false, cx);
                                }
                                cx.notify();
                            }))
                            .child(
                                div()
                                    .font_family("Inter")
                                    .text_size(px(11.0))
                                    .text_color(rgba(if picked { MAP_INK } else { BODY_SILVER }))
                                    .child(name),
                            ),
                    );
                }
                col = col.child(source_row);
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(6.0))
                        .items_center()
                        .child(small("personnel:".to_string()))
                        .child(self.timeline_field(0, "id", cx))
                        .child(small("unit:".to_string()))
                        .child(self.timeline_field(1, "id", cx)),
                );
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(6.0))
                        .items_center()
                        .child(small("from:".to_string()))
                        .child(self.timeline_field(2, "RFC 3339", cx))
                        .child(small("to:".to_string()))
                        .child(self.timeline_field(3, "RFC 3339", cx)),
                );
                // The console draws BOTH verbs and both reload from the
                // first page — kept as drawn, because a panel that renamed
                // the console's verbs would be a second vocabulary for one
                // surface.
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(6.0))
                        .child(
                            div()
                                .id("timeline-apply")
                                .cursor_pointer()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.send_timeline(false, cx);
                                }))
                                .child(button("Apply + reload", true, false, false)),
                        )
                        .child(
                            div()
                                .id("timeline-reload")
                                .cursor_pointer()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.send_timeline(false, cx);
                                }))
                                .child(button("Reload", true, false, false)),
                        ),
                );
                col = col.child(small(
                    "ids are numeric; windows are RFC 3339, empty means the whole exercise."
                        .to_string(),
                ));
                col = col.child(separator());
                // The console's own empty sentence: empty means "reload to
                // read", not "Minos has none" — data absent because it was
                // not asked for is a different sentence from an empty
                // exercise.
                if self.session.timeline.is_empty() {
                    col = col.child(small(
                        "No events yet \u{2014} reload to read the exercise.".to_string(),
                    ));
                } else {
                    for e in &self.session.timeline {
                        col = col.child(small(Self::timeline_line(e)));
                    }
                }
                // The read's own status — a refusal or "more below" — and
                // the cursor's verb, drawn only while a next page exists.
                if !self.timeline_status.is_empty() {
                    col = col.child(small(self.timeline_status.clone()));
                }
                if self.session.timeline_has_more {
                    col = col.child(
                        div()
                            .id("timeline-more")
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.send_timeline(true, cx);
                            }))
                            .child(button("Load more \u{2192}", true, false, false)),
                    );
                }
                col
            }
            2 => {
                // RECORD — the console's compose, in order: the seat
                // banner, the subject pills, the score, the verb. The list
                // and its reload follow under the separator, the console's
                // Marks half.
                if !self.is_judge() {
                    col = col.child(small(
                        "Judging needs a judge-side seat \u{2014} the list below still reads."
                            .to_string(),
                    ));
                }
                let options = self.subject_options(true);
                if options.is_empty() {
                    col = col.child(small("No subjects seated.".to_string()));
                } else {
                    let picked = Self::resolve_subject(self.judge_subject, &options);
                    let mut row = div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(6.0))
                        .items_center()
                        .child(small("subject:".to_string()));
                    for (i, (id, name)) in options.iter().enumerate() {
                        let subject = *id;
                        let on = Some(subject) == picked;
                        let label = name.clone();
                        row = row.child(
                            div()
                                .id(("judge-subject", i))
                                .cursor_pointer()
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(3.0))
                                .when(on, |d| d.bg(rgba(PANEL_SLATE)))
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.judge_subject = Some(subject);
                                    cx.notify();
                                }))
                                .child(
                                    div()
                                        .font_family("Inter")
                                        .text_size(px(11.0))
                                        .text_color(rgba(if on { MAP_INK } else { BODY_SILVER }))
                                        .child(label),
                                ),
                        );
                    }
                    col = col.child(row);
                }
                col = col.child(small("score:".to_string()));
                col = col.child(self.write_field(WriteField::Score, "pass / refer", cx));
                col = col.child(small(
                    "score is free text. No action citation \u{2014} no public fix-list route exists."
                        .to_string(),
                ));
                // The console's `add_enabled(judge_seat, …)`: without a
                // judge seat there is no click target at all, not a verb
                // that answers with a refusal.
                let judge = self.is_judge();
                let record = div()
                    .id("judge-record")
                    .child(button("Record judgement", true, !judge, false));
                col = col.child(if judge {
                    record.cursor_pointer().on_click(cx.listener(|view, _, _, cx| {
                        view.send_judgement(cx);
                    }))
                } else {
                    record
                });
                col = col.child(separator());
                col = col.child(small("MARKS".to_string()));
                col = col.child(small(
                    "Append-only: a mark can never be edited or deleted \u{2014} correct it with a new one."
                        .to_string(),
                ));
                // The console's `reload` under the marks: a re-read of the
                // same list the closure fetch seeds, and the read's own
                // status line for the refusal.
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(6.0))
                        .child(
                            div()
                                .id("judgements-reload")
                                .cursor_pointer()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.send_judgements(cx);
                                }))
                                .child(button("Reload", true, false, false)),
                        ),
                );
                if !self.judgements_status.is_empty() {
                    col = col.child(small(self.judgements_status.clone()));
                }
                if self.session.judgements.is_empty() {
                    col.child(small("No marks yet.".to_string()))
                } else {
                    for j in &self.session.judgements {
                        let cited = j
                            .cited_unit
                            .as_ref()
                            .map(|u| format!(" \u{00b7} on {u}"))
                            .unwrap_or_default();
                        col = col.child(small(format!(
                            "{}: {}{} \u{2014} by {}",
                            j.personnel_name, j.score, cited, j.judge_name
                        )));
                    }
                    col
                }
            }
            3 => {
                // FILE / REVISE — the console's compose, in order. Filing
                // picks any seat; revising fixes the subject to the row's
                // and offers save and cancel instead. No author field —
                // the caller is it.
                if let Some(rid) = self.review_editing {
                    col = col.child(small(format!("REVISE #{rid} — subject fixed at filing.")));
                } else {
                    col = col.child(small("FILE".to_string()));
                    let options = self.subject_options(false);
                    if options.is_empty() {
                        col = col.child(small(
                            "No roster \u{2014} filing needs a subject id the staff list owns."
                                .to_string(),
                        ));
                    } else {
                        let picked = Self::resolve_subject(self.review_subject, &options);
                        let mut row = div()
                            .flex()
                            .flex_row()
                            .flex_wrap()
                            .gap(px(6.0))
                            .items_center()
                            .child(small("subject:".to_string()));
                        for (i, (id, name)) in options.iter().enumerate() {
                            let subject = *id;
                            let on = Some(subject) == picked;
                            let label = name.clone();
                            row = row.child(
                                div()
                                    .id(("review-subject", i))
                                    .cursor_pointer()
                                    .px(px(6.0))
                                    .py(px(2.0))
                                    .rounded(px(3.0))
                                    .when(on, |d| d.bg(rgba(PANEL_SLATE)))
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.review_subject = Some(subject);
                                        cx.notify();
                                    }))
                                    .child(
                                        div()
                                            .font_family("Inter")
                                            .text_size(px(11.0))
                                            .text_color(rgba(if on { MAP_INK } else { BODY_SILVER }))
                                            .child(label),
                                    ),
                            );
                        }
                        col = col.child(row);
                    }
                }
                // One line, not the console's four rows: GPUI ships no
                // multiline field, and a paragraph the operator cannot see
                // whole is a field that lies about its contents — the cut
                // is stated, not hidden.
                col = col.child(self.write_field(WriteField::Body, "the conclusion, in words", cx));
                if self.review_editing.is_some() {
                    col = col.child(
                        div()
                            .flex()
                            .flex_row()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .id("review-save")
                                    .cursor_pointer()
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.send_revise(cx);
                                    }))
                                    .child(button("Save revision", true, false, false)),
                            )
                            .child(
                                div()
                                    .id("review-cancel")
                                    .cursor_pointer()
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.review_editing = None;
                                        view.review_body.clear();
                                        cx.notify();
                                    }))
                                    .child(button("Cancel", true, false, false)),
                            ),
                    );
                } else {
                    col = col.child(
                        div()
                            .id("review-file")
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.send_review(cx);
                            }))
                            .child(button("File review", true, false, false)),
                    );
                }
                col = col.child(separator());
                col = col.child(small("DOCUMENTS".to_string()));
                col = col.child(small(
                    "Author-owned: edit renders on your own rows only, and there is no delete \u{2014} filed reviews stay."
                        .to_string(),
                ));
                // The narrow, which applies server-side on the reload like
                // the inbox filter. Flipping it reloads at once — the
                // console's `.changed()` rule.
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .gap(px(6.0))
                        .items_center()
                        .child(
                            div()
                                .id("reviews-reload")
                                .cursor_pointer()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.send_reviews(cx);
                                }))
                                .child(button("Reload", true, false, false)),
                        )
                        .child(
                            div()
                                .id("reviews-mine")
                                .cursor_pointer()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.review_mine_only = !view.review_mine_only;
                                    view.send_reviews(cx);
                                }))
                                .child(checkbox(self.review_mine_only, false, "mine only")),
                        ),
                );
                if !self.reviews_status.is_empty() {
                    col = col.child(small(self.reviews_status.clone()));
                }
                if self.session.reviews.is_empty() {
                    col.child(small("No documents yet.".to_string()))
                } else {
                    for r in &self.session.reviews {
                        col = col.child(small(format!(
                            "{} \u{2014} by {}{}",
                            r.personnel_name,
                            r.author_name,
                            if r.revised { " \u{00b7} revised" } else { " \u{00b7} filed" }
                        )));
                        col = col.child(small(format!("{} \u{2192} {}", r.created_at, r.updated_at)));
                        col = col.child(small(r.body.clone()));
                        // The console's per-row `edit`, on the author's own
                        // rows only: it queues the row into the compose
                        // above, and the POST it arms is the server's 403
                        // for anyone else — which is why the button reads
                        // the authorship rather than the other way round.
                        if self.session.me != 0 && r.author_id == self.session.me {
                            let rid = r.id;
                            let words = r.body.clone();
                            col = col.child(
                                div()
                                    .id(("review-edit", rid as usize))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.review_editing = Some(rid);
                                        view.review_body = words.clone();
                                        cx.notify();
                                    }))
                                    .child(button("Edit", true, false, false)),
                            );
                        }
                        col = col.child(separator());
                    }
                    col
                }
            }
            _ => {
                col = col.child(small("SESSION".to_string()));
                col = col.child(small(
                    if self.session.game.is_empty() {
                        "no session held".to_string()
                    } else {
                        self.session.game.clone()
                    },
                ));
                col = col.child(separator());
                let state = crate::gamestate::GameState::from_wire(&self.session.state);
                col = col.child(small(format!(
                    "EXERCISE STATE: {}",
                    state.label().to_uppercase()
                )));
                col = col.child(small(format!("SEATS: {}", self.session.seats)));
                col = col.child(small(format!("PIECES: {}", self.session.pieces.len())));
                col = col.child(small(format!(
                    "CLOCK FACTOR: {:.0}\u{00d7}",
                    self.session.facts.time_factor
                )));
                col
            }
        };
        col.into_any_element()
    }

    /// Apply the re-read session a readiness answer returned: the seat's new
    /// state arrives read back, and the panel stays open.
    fn apply_readiness(&mut self, session: SessionView) {
        self.readiness_busy = false;
        self.session = session;
        self.readiness_status.clear();
    }

    /// Apply a Closure timeline page, from the Assessment island's Timeline
    /// tab — the console's `SetupDone::Timeline` arm, as a method. `append`
    /// is the request's own flag echoed back: a filter reload REPLACES the
    /// list and `load more` EXTENDS it. Either way the cursor block is the
    /// page's, and the latch clears — a page that left the tab busy would
    /// wedge it until relaunch. A refusal lands on the status line the tab
    /// draws, never silently, and leaves the events as they were.
    fn apply_timeline(
        &mut self,
        outcome: Result<(crate::backend::TimelinePage, bool), String>,
    ) {
        self.timeline_busy = false;
        match outcome {
            Ok((page, append)) => {
                if append {
                    self.session.timeline.extend(page.events);
                } else {
                    self.session.timeline = page.events;
                }
                self.session.timeline_cursor = page.next_cursor.clone();
                self.session.timeline_has_more = page.has_more;
                self.timeline_status = format!(
                    "timeline: {} event(s){}",
                    self.session.timeline.len(),
                    if page.has_more { " · more below" } else { "" }
                );
            }
            Err(why) => {
                self.timeline_status = why;
            }
        }
    }

    /// Re-read this closure's marks, from the Judgements tab's `reload` —
    /// the console's `load_judgements`, as a message. The binary owns the
    /// GET; the shell only asks. Gated the way every read here is: a
    /// request already in flight sends nothing.
    ///
    /// Returns whether a request went out, so a test exercises exactly
    /// what the click does.
    fn send_judgements(&mut self, cx: &mut Context<Self>) -> bool {
        if self.judgements_busy {
            return false;
        }
        self.judgements_busy = true;
        self.judgements_status = "reading judgements\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::ReloadJudgements);
        cx.notify();
        true
    }

    /// Apply the re-read marks: they REPLACE the held list — the console's
    /// `SetupDone::Judgements` arm, whose status sentence this keeps
    /// verbatim. A refusal lands on the status line the tab draws and
    /// leaves the held marks alone.
    fn apply_judgements(&mut self, outcome: Result<Vec<crate::backend::Judgement>, String>) {
        self.judgements_busy = false;
        match outcome {
            Ok(marks) => {
                self.session.judgements = marks;
                self.judgements_status =
                    format!("judgements: {} mark(s)", self.session.judgements.len());
            }
            Err(why) => {
                self.judgements_status = why;
            }
        }
    }

    /// Re-read this closure's documents, from the Reviews tab's `reload` —
    /// the console's `load_reviews`, as a message. Checked, `mine only`
    /// narrows the read server-side to this authorship. The author id rides
    /// the message resolved (this seat), because the binary holds the token
    /// but never the seating — and `None` without a seat, the console's own
    /// flattening of a seatless `auth_user_id`.
    ///
    /// Returns whether a request went out, so a test exercises exactly
    /// what the click does.
    fn send_reviews(&mut self, cx: &mut Context<Self>) -> bool {
        if self.reviews_busy {
            return false;
        }
        self.reviews_busy = true;
        self.reviews_status = "reading reviews\u{2026}".to_string();
        let author = match (self.review_mine_only, self.session.me) {
            (true, id) if id != 0 => Some(id),
            _ => None,
        };
        let _ = self.order_tx.send(ToBinary::ReloadReviews { author });
        cx.notify();
        true
    }

    /// Apply the re-read documents: they REPLACE the held list — the
    /// console's `SetupDone::Reviews` arm, whose status sentence this keeps
    /// verbatim. A refusal lands on the status line the tab draws and
    /// leaves the held documents alone.
    fn apply_reviews(&mut self, outcome: Result<Vec<crate::backend::Review>, String>) {
        self.reviews_busy = false;
        match outcome {
            Ok(documents) => {
                self.session.reviews = documents;
                self.reviews_status =
                    format!("reviews: {} document(s)", self.session.reviews.len());
            }
            Err(why) => {
                self.reviews_status = why;
            }
        }
    }

    /// This seat judges iff its own row says so — the console's
    /// `own_roster_row().is_some_and(|p| p.judge)`, which is what arms the
    /// record verb there and here.
    fn is_judge(&self) -> bool {
        self.own_seat().is_some_and(|p| p.judge)
    }

    /// The subject picker options over the held room, by name — the
    /// console's subject roster, sorted the way its ComboBox sorts.
    /// `exclude_self` carries the one difference between the two pickers:
    /// self-judging fails loudly server-side and the judge picker refuses
    /// it first, while a reviewer may file on anyone (authorship is the
    /// caller, never the subject).
    fn subject_options(&self, exclude_self: bool) -> Vec<(i64, String)> {
        let mut out: Vec<(i64, String)> = self
            .session
            .people
            .iter()
            .filter(|p| !exclude_self || p.id != self.session.me)
            .map(|p| (p.id, p.name.clone()))
            .collect();
        out.sort_by(|a, b| a.1.cmp(&b.1));
        out
    }

    /// The stored pick while it is still seated, else the first option —
    /// the console re-defaults the same way every frame it draws the
    /// picker, so a re-seated room never strands the pane on a ghost.
    fn resolve_subject(stored: Option<i64>, options: &[(i64, String)]) -> Option<i64> {
        stored
            .filter(|s| options.iter().any(|(id, _)| id == s))
            .or_else(|| options.first().map(|(id, _)| *id))
    }

    /// Record the picked subject's mark — the console's `record_judgement`,
    /// as a message. The guards run in the console's order and speak its
    /// sentences: the seat, the subject, the self-judge refusal the picker
    /// already makes unreachable, then the score. The binary owns the POST;
    /// the shell only asks.
    ///
    /// Returns whether a request went out, so a test exercises exactly
    /// what the click does.
    fn send_judgement(&mut self, cx: &mut Context<Self>) -> bool {
        if self.judge_write_busy {
            return false;
        }
        if !self.is_judge() {
            self.judgements_status =
                "Judging needs a judge-side seat \u{2014} the list below still reads.".to_string();
            cx.notify();
            return false;
        }
        let options = self.subject_options(true);
        let Some(subject) = Self::resolve_subject(self.judge_subject, &options) else {
            self.judgements_status = "pick a subject first".to_string();
            cx.notify();
            return false;
        };
        if subject == self.session.me {
            self.judgements_status = "cannot judge yourself".to_string();
            cx.notify();
            return false;
        }
        if self.judge_score.trim().is_empty() {
            self.judgements_status = "write the score first".to_string();
            cx.notify();
            return false;
        }
        self.judge_subject = Some(subject);
        let score = self.judge_score.trim().to_string();
        self.judge_write_busy = true;
        self.judgements_status = "recording judgement\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::RecordJudgement {
            personnel_id: subject,
            score,
        });
        cx.notify();
        true
    }

    /// File the picked subject's review — the console's `file_review`, as a
    /// message. Filing is ungated by seat: anyone seated may file, and a
    /// permission refusal arrives loudly on the line. The subject and the
    /// non-empty body are refused before any request, in the console's
    /// sentences.
    ///
    /// Returns whether a request went out, so a test exercises exactly
    /// what the click does.
    fn send_review(&mut self, cx: &mut Context<Self>) -> bool {
        if self.review_write_busy {
            return false;
        }
        let options = self.subject_options(false);
        let Some(subject) = Self::resolve_subject(self.review_subject, &options) else {
            self.reviews_status = "pick a subject first".to_string();
            cx.notify();
            return false;
        };
        if self.review_body.trim().is_empty() {
            self.reviews_status = "write the review first".to_string();
            cx.notify();
            return false;
        }
        self.review_subject = Some(subject);
        let body = self.review_body.trim().to_string();
        self.review_write_busy = true;
        self.reviews_status = "filing review\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::FileReview {
            personnel_id: subject,
            body,
        });
        cx.notify();
        true
    }

    /// Save the row under revision — the console's `revise_review`, as a
    /// message. Words only, subject fixed at filing; anyone-but-author
    /// fails with the server's explicit 403, which the line carries rather
    /// than a button pre-hidden on stale data.
    ///
    /// Returns whether a request went out, so a test exercises exactly
    /// what the click does.
    fn send_revise(&mut self, cx: &mut Context<Self>) -> bool {
        if self.review_write_busy {
            return false;
        }
        let Some(rid) = self.review_editing else {
            self.reviews_status = "nothing under revision".to_string();
            cx.notify();
            return false;
        };
        if self.review_body.trim().is_empty() {
            self.reviews_status = "write the revision first".to_string();
            cx.notify();
            return false;
        }
        let body = self.review_body.trim().to_string();
        self.review_write_busy = true;
        self.reviews_status = "saving revision\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::ReviseReview {
            review_id: rid,
            body,
        });
        cx.notify();
        true
    }

    /// Declare or withdraw this seat's readiness on the asking step — the
    /// console's `declare_step`, as a message.
    ///
    /// Returns false (and sends nothing) for the three non-sends: no step
    /// asks this seat, the named step is not the asking one (a stale click
    /// after the re-read moved on), or a declaration already in flight. The
    /// same rule the row's button enforces, kept in one place so a test
    /// exercises exactly what the click does — the server arbitrates the
    /// seat on top, loudly.
    fn send_step_declare(
        &mut self,
        scenario_id: i64,
        step_id: i64,
        ready: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let asking = self
            .session
            .step
            .as_ref()
            .and_then(|s| s.asking.as_ref())
            .is_some_and(|a| a.scenario_id == scenario_id && a.step_id == step_id);
        if !asking || self.step_declare_busy {
            return false;
        }
        self.step_declare_busy = true;
        self.step_declare_status = if ready {
            "declaring\u{2026}"
        } else {
            "withdrawing\u{2026}"
        }
        .to_string();
        self.feed(format!(
            "step readiness — {}",
            if ready { "declared" } else { "withdrawn" }
        ));
        let _ = self.order_tx.send(ToBinary::DeclareStep {
            scenario_id,
            step_id,
            ready,
        });
        cx.notify();
        true
    }

    /// The one step the reminder strip names: what this caller owes the
    /// exercise, never the book around it. `needs_you` is true while the
    /// asked step still waits on this caller; false once declared, or when
    /// the strip only names what is playing.
    /// A reminder is one line, so this is the whole line, composed here —
    /// the strip below only draws it, and a test reads it without rendering.
    fn step_reminder(&self) -> Option<StepReminder> {
        if !self.session.execution {
            return None;
        }
        if let Some(a) = self.session.step.as_ref().and_then(|s| s.asking.as_ref()) {
            return Some(StepReminder {
                needs_you: !a.you_ready,
                text: format!(
                    "NEXT \u{00b7} {} \u{2014} {} \u{00b7} {} of {} ready",
                    a.scenario,
                    a.content,
                    a.required - a.outstanding,
                    a.required
                ),
            });
        }
        if let Some(p) = self.session.step.as_ref().and_then(|s| s.playing.as_ref()) {
            return Some(StepReminder {
                needs_you: false,
                text: format!("NOW \u{00b7} {} \u{2014} {}", p.scenario, p.content),
            });
        }
        None
    }

    /// Open the step modal from the reminder strip: the scenario, the step
    /// and the declare row live there, so the strip never grows its own.
    /// Split out of the click listener so a test drives exactly what the
    /// click does.
    fn open_step_detail(&mut self, cx: &mut Context<Self>) {
        self.step_open = true;
        // And bring it onto the band. The strip's whole job is to be the one
        // line that says what you owe; a click that opened an island below
        // the fold would be a click that appears to do nothing.
        // The scroll itself is `render`'s to make: the column's band comes
        // from the window, and a click handler has no honest way to ask for
        // it. So the click records the wish and the next frame honours it —
        // which also means the reveal uses the same band the island was just
        // placed in, rather than one computed a frame ago.
        self.reveal = Some(view::IslandKey::Step);
        cx.notify();
    }

    /// Apply the re-read session a declaration answer returned: the counts
    /// and the caller's own flag arrive read back, and the panel stays open.
    fn apply_step_declare(&mut self, session: SessionView) {
        self.step_declare_busy = false;
        self.session = session;
        self.step_declare_status.clear();
    }

    /// Send the comms panel's telegram row — the console's `send_message`,
    /// cut to the proven broadcast path.
    ///
    /// Returns false (and sends nothing) for the three non-sends: a game
    /// that is not executing (the server only takes messages there), blank
    /// content (the console's own sentence refuses it first), and a send
    /// already in flight. The same rule the SEND button enforces, kept in
    /// one place so a test exercises exactly what the click does.
    fn send_telegram(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.session.execution {
            self.msg_status = "messages send only in execution".to_string();
            cx.notify();
            return false;
        }
        if self.msg_content.trim().is_empty() {
            self.msg_status = "write the message first".to_string();
            cx.notify();
            return false;
        }
        if self.msg_busy {
            return false;
        }
        self.msg_busy = true;
        self.msg_status = "sending\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::SendTelegram {
            content: self.msg_content.trim().to_string(),
        });
        cx.notify();
        true
    }

    /// The row's Enter key: the same `send_telegram` the SEND button calls —
    /// never a copy.
    fn submit_telegram(&mut self, cx: &mut Context<Self>) {
        self.send_telegram(cx);
    }

    /// Apply the re-read session a send answer returned: the thread arrives
    /// read back with the new message in it, the row clears, and the panel
    /// stays open — the author keeps writing, the way the book stays open
    /// across adds.
    fn apply_message(&mut self, session: SessionView) {
        self.msg_busy = false;
        self.session = session;
        self.msg_content.clear();
        self.msg_status.clear();
    }

    /// The server's read for one step's related list, if the fan-out brought
    /// it. Absent is unreadable, never empty — an empty list reads as an
    /// entry with no posts, which is what a cleared list answers.
    fn related_for(&self, scenario_id: i64, step_id: i64) -> Option<&StepRelatedView> {
        self.session
            .related
            .iter()
            .find(|r| r.scenario_id == scenario_id && r.step_id == step_id)
    }

    /// Open (or close) the relate editor for a step, seeding the working set
    /// from the server's read. The console seeds from staged/server/empty and
    /// there is no staged here — the working set IS the last saved list
    /// until the author touches it.
    fn toggle_relate(&mut self, scenario_id: i64, step_id: i64, cx: &mut Context<Self>) {
        if self.relate_open == Some((scenario_id, step_id)) {
            self.relate_open = None;
        } else {
            self.relate_open = Some((scenario_id, step_id));
            self.relate_working = self
                .related_for(scenario_id, step_id)
                .map(|r| r.posts.clone())
                .unwrap_or_default();
        }
        cx.notify();
    }

    /// Flip one ref in the working set — the checkbox verb, kept in one
    /// place so tests drive exactly what the tap does. The tap notifies; a
    /// bare flip does not.
    fn toggle_relate_item(&mut self, kind: &str, id: i64) {
        let item = (kind.to_string(), id);
        if self.relate_working.contains(&item) {
            self.relate_working.retain(|k| k != &item);
        } else {
            self.relate_working.push(item);
        }
    }

    /// Save the working set whole — even empty, which is legal and clears
    /// the list. One message on the sink; busy latches until the re-read
    /// answers.
    fn save_relate(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((scenario_id, step_id)) = self.relate_open else {
            return false;
        };
        if self.relate_busy {
            return false;
        }
        self.relate_busy = true;
        self.book_status = "saving\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::SetRelated {
            scenario_id,
            step_id,
            related: self.relate_working.clone(),
        });
        cx.notify();
        true
    }

    /// Apply the re-read session a relate answer returned: the resolved list
    /// arrives from the server, the working set re-seeds from it, and the
    /// editor stays open — the author keeps working.
    fn apply_relate(&mut self, session: SessionView) {
        self.relate_busy = false;
        self.session = session;
        if let Some((sid, st)) = self.relate_open {
            self.relate_working = self
                .related_for(sid, st)
                .map(|r| r.posts.clone())
                .unwrap_or_default();
        }
        self.book_status.clear();
    }

    /// The Comms panel (the console's `comms_body` and `messages_island`):
    /// the session's inbox, newest first, plus the telegram composer row.
    ///
    /// The console's island also carries a per-step instruction feed and the
    /// full composer (kind, marking, audience, grade); the feed is a later
    /// slice and the composer is cut to the proven broadcast path, and the
    /// panel states both rather than faking either control.
    fn comms_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rule = || div().h(px(1.0)).w_full().bg(rgba(HAIRLINE_SLATE));
        // Content-sized, not `size_full`: this column is an ISLAND body now,
        // and the island's own scroll viewport does the measuring. A
        // `size_full` child inside a scroll viewport sizes to the viewport
        // and then clips its own overflow, which is a body that silently
        // loses its tail instead of scrolling to it.
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        col = col.child(rule());
        col = col.child(
            div()
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(format!(
                    "{} \u{00b7} {} message(s)",
                    if self.session.game.is_empty() {
                        "no session".to_string()
                    } else {
                        self.session.game.clone()
                    },
                    self.session.messages.len()
                )),
        );
        col = col.child(rule());
        if self.session.messages.is_empty() {
            col = col.child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgba(BODY_SILVER.with_alpha(204)))
                    .child(if self.session.execution {
                        "No messages in this exercise."
                    } else {
                        "No messages \u{2014} and none can arrive until the game is executing."
                    }),
            );
        }
        for m in &self.session.messages {
            // The server's own stamp, trimmed to what an operator reads at a
            // glance. The console's `short_timestamp` is in the egui binary
            // and the shell cannot name it, so the trim is repeated here
            // rather than reached for — the same rule that keeps the shell
            // free of `map_render`, and the same cost.
            let at = m
                .at
                .split('T')
                .nth(1)
                .map(|t| t.split(['+', '-', 'Z']).next().unwrap_or(t))
                .map(|t| t.split('.').next().unwrap_or(t))
                .unwrap_or(m.at.as_str())
                .to_string();
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .font_family(view::FACE_HUMAN)
                                    .text_size(px(11.0))
                                    .text_color(rgba(MAP_INK))
                                    .child(m.sender.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgba(BODY_SILVER))
                                    .child(if m.broadcast {
                                        format!("{} \u{00b7} broadcast", m.kind)
                                    } else {
                                        m.kind.clone()
                                    }),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgba(BODY_SILVER.with_alpha(178)))
                                    .child(at),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgba(BODY_SILVER))
                            .child(m.content.clone()),
                    ),
            );
            col = col.child(rule());
        }
        // The composer row: the console's `compose_ui`, cut to the proven
        // path — a broadcast telegram, TERBUKA, BIASA. Armed only in an
        // executing game, like the helm: the server only takes messages
        // there. A blank row never posts (the console's own sentence), and
        // the button guards in the listener, not only in its styling.
        col = col.child(rule());
        col = col.child(
            div()
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child("New telegram"),
        );
        col = col.child(self.compose_field(10, "message text\u{2026}", cx));
        let msg_armed = self.session.execution;
        let msg_status = self.msg_status.clone();
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .id("telegram-send")
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.send_telegram(cx);
                        }))
                        .child(button("SEND", false, !msg_armed, false)),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(rgba(if msg_status.starts_with("refused") {
                            STALE_SAND
                        } else {
                            BODY_SILVER
                        }))
                        .child(msg_status),
                ),
        );
        col = col.child(
            div()
                .text_size(px(10.0))
                .text_color(rgba(BODY_SILVER.with_alpha(178)))
                .child(if msg_armed {
                    "A telegram broadcast, TERBUKA / BIASA — kind, marking, \
                     audience and grade pickers are a later slice, and the hull \
                     must sit under a task node for the server to take it."
                } else {
                    "Composing is a write, and the server takes messages only in execution."
                }),
        );
        col.into_any_element()
    }

    /// The Step panel: the exercise's running step, the readiness gate, and
    /// the SERVER's assumed time.
    ///
    /// This is the console's spine — a step is played, a step is declared.
    /// The asking row DECLARES: the participant's own write, proven live.
    /// The Game Master's verbs (play, skip, end, rewind) are visible-but-off:
    /// they aim from socket-tracked runs, and this shell tracks none yet —
    /// the panel states the rule rather than aiming Play at a guess. What is
    /// here besides the declare is what the server tells anyone with read on
    /// the game.
    fn step_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // Content-sized, not `size_full`: this column is an ISLAND body now,
        // and the island's own scroll viewport does the measuring. A
        // `size_full` child inside a scroll viewport sizes to the viewport
        // and then clips its own overflow, which is a body that silently
        // loses its tail instead of scrolling to it.
        //
        // The body's grammar is three units and nothing else: a `section`
        // ruled break, a `kv` key/value row, and a `note` line of prose. The
        // panel this replaces was eleven `text_size(px(11.0))` divs in a
        // column with 4px gaps, which is a stack of paragraphs rather than an
        // instrument reading — nothing lined up, so the eye re-read every
        // row. `kv` puts the values in a column the eye can track.
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(view::PANEL_PAD));

        if self.session.game_id == 0 {
            return col.child(note("No session held.")).into_any_element();
        }
        // The assumed stamp, trimmed the way the console's `short_timestamp`
        // rule trims it: date and time, no fractional seconds and no offset.
        // The offset is dropped rather than converted — the exercise's own
        // zone is what the server stamped, and re-basing it here would be the
        // client second-guessing the anchor.
        let assumed = self.session.step.as_ref().map(|st| {
            let t = st.assumed_time.replace('T', " ");
            let t = t.split(['+']).next().unwrap_or(&t).to_string();
            let t = t.split('.').next().unwrap_or(&t).to_string();
            t.trim().to_string()
        });
        match self.session.step.as_ref() {
            // A server that never published the step is not an error state:
            // the console says what it can see and stops, rather than
            // inventing a placeholder step to fill the panel.
            None => {
                col = col.child(note(
                    "No step to read — the game's scenario clock has no anchor, \
                     or this hold has no step.",
                ));
            }
            Some(step) => {
                // `ASSUMED` rather than `TIME`, and the word carries the
                // whole fact: it is the server's stamp, not this client's
                // integral. That distinction used to need a sentence under
                // the row explaining it; the key says it in one tracked word,
                // and a body that spends a line on an implementation note is
                // a body with less room for the operator's verb.
                col = col.child(kv(
                    "ASSUMED",
                    assumed.clone().unwrap_or_else(|| "\u{2014}".to_string()),
                    MAP_INK,
                ));

                // --- playing -------------------------------------------------
                // The one thing on screen that is prose rather than data, so
                // it is the one thing set in the human face at body size.
                col = col.child(section("PLAYING"));
                col = col.child(match step.playing.as_ref() {
                    None => note("nothing playing").into_any_element(),
                    Some(p) => div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .font_family(view::FACE_HUMAN)
                                .text_size(px(view::BODY_PT))
                                .text_color(rgba(MAP_INK))
                                .child(p.content.clone()),
                        )
                        .child(kv("SCENARIO", p.scenario.clone(), BODY_SILVER))
                        .child(kv("STATE", p.state.clone(), BODY_SILVER))
                        .into_any_element(),
                });

                // --- asking: the panel's leading figure ----------------------
                // The readiness count is the one quantity in this panel that
                // leads, so it is the one thing drawn at `READOUT_LG_PT` with
                // `tnum`. Tabular because it changes: a count that reflows
                // the label beside it as it ticks is the defect the whole
                // figure component exists to prevent.
                //
                // Its ink is its state, routed through the status palette
                // rather than left grey — the Never-Gray-On-Gray rule, which
                // applies to a figure exactly as it applies to a status line.
                col = col.child(section("ASKING"));
                col = col.child(match step.asking.as_ref() {
                    None => note("no step asks you").into_any_element(),
                    Some(a) => {
                        // The declare row draws what it sends: the asking
                        // step's own ids ride the button, so a test exercises
                        // exactly what the click does.
                        let (sid, step_id, you_ready) =
                            (a.scenario_id, a.step_id, a.you_ready);
                        let declare_status = self.step_declare_status.clone();
                        let outstanding = a.outstanding.max(0);
                        let ready = (a.required - a.outstanding).max(0);
                        // The state ink, and where it does NOT go.
                        //
                        // It is on the label and the OUTSTANDING row, not on
                        // the figure. A status colour at 30pt is the exact
                        // failure this project already recorded: the old
                        // system lit every island's rim in Radar Cyan, it
                        // looked good rendered alone, and as a console it
                        // taught the operator nothing because cyan was
                        // everywhere. Here the readiness count is outstanding
                        // on essentially every frame of a real exercise, so a
                        // yellow figure would be the default face of the
                        // panel — and a default-state accent is an accent
                        // spent on nothing.
                        //
                        // So size carries the authority and colour carries
                        // the state: the figure is Map Ink, the word beside
                        // it and the row under it say which way it is. Neither
                        // dominates, and neither is on by default.
                        let ink = if outstanding == 0 {
                            SIGNAL_GREEN
                        } else {
                            WARNING_SAND
                        };
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_baseline()
                                    .gap(px(8.0))
                                    .child(figure(
                                        &format!("{ready} of {}", a.required),
                                        true,
                                        MAP_INK,
                                    ))
                                    .child(tracked("ready", view::LABEL_PT, ink)),
                            )
                            .child(
                                div()
                                    .font_family(view::FACE_HUMAN)
                                    .text_size(px(view::BODY_PT))
                                    .text_color(rgba(MAP_INK))
                                    .child(a.content.clone()),
                            )
                            .child(kv("SCENARIO", a.scenario.clone(), BODY_SILVER))
                            .child(kv("OUTSTANDING", outstanding.to_string(), ink))
                            .child(kv(
                                "YOU",
                                if you_ready { "ready" } else { "\u{2014}" },
                                if you_ready { SIGNAL_GREEN } else { BODY_SILVER },
                            ))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(
                                        div()
                                            .id("step-declare")
                                            .cursor_pointer()
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                view.send_step_declare(
                                                    sid, step_id, !you_ready, cx,
                                                );
                                            }))
                                            .child(button(
                                                if you_ready {
                                                    "WITHDRAW"
                                                } else {
                                                    "DECLARE READY"
                                                },
                                                false,
                                                false,
                                                false,
                                            )),
                                    )
                                    .child(note(declare_status)),
                            )
                            .into_any_element()
                    }
                });

                // The Game Master's verbs: played, skipped, ended and rewound
                // from tracked runs. This shell tracks no runs — the console
                // learns them off the socket, and there is no runs GET — so
                // the verbs stay visible-but-off for every seat rather than
                // aiming Play at a guess. The server arbitrates the seat on
                // top of that.
                col = col.child(section("GAME MASTER"));
                col = col.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .flex_wrap()
                        .child(div().id("step-play").child(button("Play", false, true, false)))
                        .child(div().id("step-skip").child(button("Skip", false, true, false)))
                        .child(div().id("step-end").child(button("End", false, true, false)))
                        .child(
                            div()
                                .id("step-rewind")
                                .child(button("Rewind", false, true, false)),
                        ),
                );
                // No note under these verbs. There was one — "the Game Master
                // drives these from tracked runs" — and it was trimmed, then
                // removed, for a measured reason: this body's content measures
                // ~511pt against a 450pt viewport, and the last line was
                // painting below the panel's own box. That is the defect
                // `docs/ui-overhaul-status.md` records as "a `max_rect` tells
                // the layout where to stop; it does not stop the painter", and
                // a clipped last line with no scroll available is a clip
                // nobody can act on.
                //
                // The GAME MASTER label and four disabled verbs already say
                // what the note said. A body that spends its last line on a
                // sentence the chrome has already conveyed is a body with less
                // room for the operator's verb, so the note goes and the verbs
                // are the tail.
                //
                // EXERCISE follows the runs, and it is NOT one of them:
                // `End session →` ends the exercise, not a run, and it is
                // the console's `z.clock` island reduced to its one control.
                // The island's readouts are the sim's tick (G+MM:SS) and its
                // pause flag, and this shell runs no sim — the server's
                // assumed time already reads on the step above — so the
                // merge carries the verb and names the missing tick rather
                // than faking it. Player stations never see it: the console
                // pushes `z.clock` for organizers only.
                if !self.player_station() {
                    col = col.child(section("EXERCISE"));
                    col = col.child(note(
                        "Elapsed needs the exercise tick \u{2014} no clock GET exists, so this carries the verb and not the readout.",
                    ));
                    col = col.child(
                        div()
                            .id("step-end-session")
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.send_end_session(cx);
                            }))
                            .child(button("End session \u{2192}", false, false, false)),
                    );
                    if !self.end_status.is_empty() {
                        col = col.child(note(self.end_status.clone()));
                    }
                }
            }
        }
        col.into_any_element()
    }

    /// The Inspector: the SELECTED hull's detail, docked along the bottom.
    ///
    /// A dock, not a window: the console recomputes its place every frame so
    /// a drag snaps back, because a dock is a place rather than a window that
    /// remembers. Everything shown is data the shell already holds — the
    /// roster carries the identity, the live markers carry the fix — so this
    /// panel added no plumbing at all, which is the shape a finished kernel
    /// is supposed to give.
    ///
    /// The console's three columns also carry EDITORS (the map symbol, the
    /// affiliation, the heading). Those are writes, and the same rule blocks
    /// them as blocks the helm, so they are named rather than faked.
    fn inspector(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let sel = self.roster.get(self.selected).cloned();
        let fix = sel
            .as_ref()
            .and_then(|r| self.live.iter().find(|m| m.id == r.unit_id).cloned());
        let col = |label: &'static str, value: String| {
            div()
                .flex()
                .flex_col()
                .gap(px(3.0))
                .flex_1()
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(rgba(BODY_SILVER.with_alpha(178)))
                        .child(label),
                )
                .child(
                    div()
                        .font_family(view::FACE_MACHINE)
                        .text_size(px(11.0))
                        .text_color(rgba(MAP_INK))
                        .child(value),
                )
        };
        let dash = || "\u{2014}".to_string();
        let mut dock = div()
            .absolute()
            .left(px(12.0))
            .right(px(12.0))
            .bottom(px(12.0))
            .h(px(INSPECT_H))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(PAD))
            .rounded(px(8.0))
            .bg(rgba(CONSOLE_NIGHT))
            .border(px(1.0))
            .border_color(rgba(HAIRLINE_SLATE));
        dock = dock.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(TITLE_SIZE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgba(MAP_INK))
                        .child(match sel.as_ref() {
                            Some(r) => format!("{} \u{00b7} {}", r.name, r.hull),
                            None => "Inspector".to_string(),
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .id("centre-hull")
                                .cursor_pointer()
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.jump_to_selection(cx);
                                }))
                                .child(button("Centre", true, fix.is_none(), false)),
                        )
                        .child(
                            div()
                                .id("close-inspect")
                                .cursor_pointer()
                                .text_size(px(14.0))
                                .text_color(rgba(BUTTON_LABEL))
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.inspect_open = false;
                                    cx.notify();
                                }))
                                .child("\u{00d7}"),
                        ),
                ),
        );
        dock = dock.child(div().h(px(1.0)).w_full().bg(rgba(HAIRLINE_SLATE)));
        let Some(r) = sel else {
            return dock.child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgba(BODY_SILVER.with_alpha(204)))
                    .child("no selection"),
            );
        };
        dock = dock.child(
            div()
                .flex()
                .flex_row()
                .gap(px(14.0))
                .child(col("class", r.class.clone()))
                .child(col("unit id", r.unit_id.clone()))
                .child(col(
                    "position",
                    fix.as_ref()
                        .map(|m| format!("{:.4}, {:.4}", m.latitude, m.longitude))
                        .unwrap_or_else(dash),
                ))
                .child(col(
                    "course",
                    fix.as_ref()
                        .and_then(|m| m.heading_deg)
                        .map(|h| format!("{h:.0}\u{00b0}"))
                        .unwrap_or_else(dash),
                ))
                .child(col(
                    "speed",
                    fix.as_ref()
                        .and_then(|m| m.speed_kn)
                        .map(|s| format!("{s:.1} kn"))
                        .unwrap_or_else(dash),
                ))
                .child(col(
                    "trail",
                    fix.as_ref()
                        .map(|m| format!("{} fix(es)", m.trail))
                        .unwrap_or_else(dash),
                ))
                .child(col(
                    "state",
                    match fix.as_ref() {
                        None => "no fix".to_string(),
                        Some(m) if m.stale => "stale".to_string(),
                        Some(_) => "live".to_string(),
                    },
                )),
        );
        dock = dock.child(
            div()
                .text_size(px(10.0))
                .text_color(rgba(BODY_SILVER.with_alpha(178)))
                .child(
                    "The symbol, affiliation and heading editors are writes, and the server \
                     takes them only in states this environment cannot reach.",
                ),
        );
        dock
    }

    /// The typed credential a field shows, and the same field for editing.
    /// One place, so the two fields cannot drift apart.
    fn cred(&self, which: Cred) -> &String {
        match which {
            Cred::Identifier => &self.login_identifier,
            Cred::Password => &self.login_password,
        }
    }

    fn cred_mut(&mut self, which: Cred) -> &mut String {
        match which {
            Cred::Identifier => &mut self.login_identifier,
            Cred::Password => &mut self.login_password,
        }
    }

    /// The sign-in card: the console's `onboard_login`, and the one panel that
    /// has to exist before any other can.
    ///
    /// The fields are HAND-ROLLED text inputs. GPUI 0.2.2 ships no text field —
    /// only the input PLUMBING (`Keystroke::key_char`, focus handles,
    /// `on_key_down`) — so a credential field here is a focus handle, a
    /// keystroke handler and a value. Kept deliberately minimal: typing,
    /// backspace, tab between fields, and Enter to submit, which is the whole
    /// job. No caret and no selection; a card is not an editor.
    fn login_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut card = div()
            .absolute()
            .left(px(24.0))
            .top(px(24.0))
            .w(px(320.0))
            .p(px(16.0))
            .rounded(px(8.0))
            .bg(rgba(DEEP_WELL))
            .border(px(1.0))
            .border_color(rgba(HAIRLINE_SLATE))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .font_family(view::FACE_HUMAN)
                    .text_size(px(26.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgba(RADAR_CYAN))
                    .child("ARCONS"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgba(BODY_SILVER))
                    .child("Command Center"),
            )
            .child(div().h(px(1.0)).w_full().bg(rgba(HAIRLINE_SLATE)))
            .child(
                div()
                    .text_size(px(TEXT_BODY_PT))
                    .text_color(rgba(MAP_INK))
                    .child("Sign in"),
            );
        card = card.child(self.login_field(Cred::Identifier, "identifier", false, cx));
        card = card.child(self.login_field(Cred::Password, "password", true, cx));
        card = card.child(
            div()
                .id("sign-in")
                .cursor_pointer()
                .h(px(AUTH_FIELD_H))
                .w_full()
                .rounded(px(CONTROL_RADIUS))
                .bg(rgba(RADAR_CYAN))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(TEXT_BODY_PT))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgba(CONSOLE_NIGHT))
                .on_click(cx.listener(|view, _, _, cx| view.submit_login(cx)))
                .child(if self.login_busy { "Signing in\u{2026}" } else { "Sign in" }),
        );
        if !self.login_status.is_empty() {
            card = card.child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgba(STALE_SAND))
                    .child(self.login_status.clone()),
            );
        }
        card.into_any_element()
    }

    /// One credential field. Hand-rolled, because GPUI has no text field — see
    /// `login_card` for why that is a keystroke handler and not a widget.
    fn login_field(
        &mut self,
        which: Cred,
        hint: &'static str,
        masked: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let handle = match self.login_handles[which as usize].clone() {
            Some(h) => h,
            None => {
                let h = cx.focus_handle();
                self.login_handles[which as usize] = Some(h.clone());
                h
            }
        };
        let text = self.cred(which);
        let empty = text.is_empty();
        let shown: String = if empty {
            hint.to_string()
        } else if masked {
            "\u{2022}".repeat(text.chars().count())
        } else {
            text.to_string()
        };
        let focused = self.login_focus == which;
        div()
            .id(("login-field", which as usize))
            .track_focus(&handle)
            .cursor(CursorStyle::IBeam)
            .h(px(AUTH_FIELD_H))
            .w_full()
            .px(px(6.0))
            .rounded(px(CONTROL_RADIUS))
            .bg(rgba(DEEP_WELL))
            .border(px(if focused { 1.0 } else { 1.0 }))
            .border_color(rgba(if focused { RADAR_CYAN } else { HAIRLINE_SLATE }))
            .flex()
            .items_center()
            .font_family(view::FACE_HUMAN)
            .text_size(px(TEXT_BODY_PT))
            .text_color(rgba(if empty { BODY_SILVER } else { MAP_INK }))
            .on_mouse_down(MouseButton::Left, cx.listener(move |view, _, window, cx| {
                window.focus(&handle);
                view.login_focus = which;
                cx.notify();
            }))
            .on_key_down(cx.listener(move |view, ev: &KeyDownEvent, window, cx| {
                view.login_key(which, &ev.keystroke, window, cx);
            }))
            .child(shown)
            .into_any_element()
    }

    /// The keystroke half of a credential field: what it does with a key.
    fn login_key(
        &mut self,
        which: Cred,
        k: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match k.key.as_str() {
            "enter" => {
                self.submit_login(cx);
                return;
            }
            "backspace" => {
                self.cred_mut(which).pop();
            }
            "tab" => {
                let other = match which {
                    Cred::Identifier => Cred::Password,
                    Cred::Password => Cred::Identifier,
                };
                self.login_focus = other;
                if let Some(h) = self.login_handles[other as usize].clone() {
                    window.focus(&h);
                }
            }
            _ => {
                // `key_char` is the typed CHARACTER: the key name is the
                // legend on the keycap, which differs by layout, and a
                // printable-only filter keeps chords and control keys out.
                if let Some(text) = k
                    .key_char
                    .as_deref()
                    .filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
                {
                    self.cred_mut(which).push_str(text);
                }
            }
        }
        cx.notify();
    }

    /// Submit the typed credentials. The BINARY owns the login — the shell has
    /// no network and never learns the base URL — so this is one message on the
    /// sink the helm and the camera already use.
    fn submit_login(&mut self, cx: &mut Context<Self>) {
        if self.login_identifier.is_empty() || self.login_password.is_empty() {
            self.login_status = "identifier and password are both required".to_string();
            cx.notify();
            return;
        }
        self.login_busy = true;
        self.login_status = "signing in\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::SignIn {
            identifier: self.login_identifier.clone(),
            password: self.login_password.clone(),
        });
        cx.notify();
    }

    /// The New-session composer: the console's `session_creator_body`, ported.
    /// Name (required) plus every optional the `POST /games` body accepts, one
    /// field per row, label above input — the console's web-form order. The
    /// mode rides radios, not a field, because it is a choice of three, not
    /// text. The console gates the door on an app role; the shell does not
    /// carry roles yet, so the door stands open while signed in and the
    /// server has the last word — which is stated here, not hidden.
    fn composer_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rule = || div().h(px(1.0)).w_full().bg(rgba(HAIRLINE_SLATE));
        let label = |text: &'static str| {
            div()
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgba(MAP_INK))
                .child(text)
        };
        // Content-sized, not `size_full`: this column is an ISLAND body now,
        // and the island's own scroll viewport does the measuring. A
        // `size_full` child inside a scroll viewport sizes to the viewport
        // and then clips its own overflow, which is a body that silently
        // loses its tail instead of scrolling to it.
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .h(px(TITLE_H))
                .child(
                    div()
                        .font_family(view::FACE_HUMAN)
                        .text_size(px(TITLE_SIZE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgba(MAP_INK))
                        .child("New session"),
                )
                .child(
                    div()
                        .id("close-composer")
                        .cursor_pointer()
                        .text_size(px(14.0))
                        .text_color(rgba(BUTTON_LABEL))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.composer_open = false;
                            cx.notify();
                        }))
                        .child("\u{00d7}"),
                ),
        );
        col = col.child(rule());
        col = col.child(
            div()
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child("A session is a plan the Game Master authors: a force, a roster and a window."),
        );
        col = col.child(label("Name *"));
        col = col.child(self.compose_field(0, "e.g. Latihan Armada 2026", cx));
        if !self.draft.name_ok() {
            col = col.child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgba(BODY_SILVER.with_alpha(204)))
                    .child("Name is required — the server refuses a nameless session."),
            );
        }
        col = col.child(label("Mode"));
        let mut modes = div().flex().flex_row().items_center().gap(px(12.0));
        for (i, (mode, _)) in SESSION_MODES.iter().enumerate() {
            let this = mode.to_string();
            modes = modes.child(
                div()
                    .id(("compose-mode", i))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.draft.mode = this.clone();
                        cx.notify();
                    }))
                    .child(radio(self.draft.mode == *mode, *mode)),
            );
        }
        col = col.child(modes);
        col = col.child(
            div()
                .text_size(px(10.0))
                .text_color(rgba(BODY_SILVER.with_alpha(204)))
                .child(
                    SESSION_MODES
                        .iter()
                        .find(|(m, _)| *m == self.draft.mode)
                        .map(|(_, why)| *why)
                        .unwrap_or(""),
                ),
        );
        for (idx, (name, hint)) in [
            (1usize, ("Description", "what this exercise is about")),
            (2, ("Purpose", "what it must achieve")),
            (3, ("Target", "who or what is exercised")),
            (4, ("Area", "where it takes place")),
            (5, ("Map tag", "which chart backbone to load")),
        ] {
            col = col.child(label(name));
            col = col.child(self.compose_field(idx, hint, cx));
        }
        let name_ok = self.draft.name_ok();
        let busy = self.compose_busy;
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .id("cancel-compose")
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.composer_open = false;
                            cx.notify();
                        }))
                        .child(button("Cancel", true, false, false)),
                )
                .child(
                    div()
                        .id("submit-compose")
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| view.submit_compose(cx)))
                        .child(button("Create session →", true, !name_ok || busy, false)),
                ),
        );
        if !name_ok {
            col = col.child(
                div()
                    .text_size(px(10.0))
                    .text_color(rgba(BODY_SILVER.with_alpha(204)))
                    .child("name required"),
            );
        }
        if !self.compose_status.is_empty() {
            col = col.child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgba(STALE_SAND))
                    .child(self.compose_status.clone()),
            );
        }
        col.into_any_element()
    }

    /// The draft field a composer row edits, by tab order: name, description,
    /// purpose, target, area, map tag. One place, so the six rows cannot drift
    /// apart — the sign-in card's `cred`/`cred_mut`, indexed. Index 6 is the
    /// BOOK panel's title row, 7-9 are its step row (content, window
    /// start, window end), and 10 is the comms panel's telegram row: they
    /// share the field implementation, and the tab handler keeps each form
    /// inside its own cycle.
    fn draft_field(&self, idx: usize) -> &String {
        match idx {
            0 => &self.draft.name,
            1 => &self.draft.description,
            2 => &self.draft.purpose,
            3 => &self.draft.target,
            4 => &self.draft.area,
            5 => &self.draft.map_tag,
            6 => &self.book_title,
            7 => &self.step_draft.content,
            8 => &self.step_draft.start_hour,
            9 => &self.step_draft.end_hour,
            _ => &self.msg_content,
        }
    }

    fn draft_field_mut(&mut self, idx: usize) -> &mut String {
        match idx {
            0 => &mut self.draft.name,
            1 => &mut self.draft.description,
            2 => &mut self.draft.purpose,
            3 => &mut self.draft.target,
            4 => &mut self.draft.area,
            5 => &mut self.draft.map_tag,
            6 => &mut self.book_title,
            7 => &mut self.step_draft.content,
            8 => &mut self.step_draft.start_hour,
            9 => &mut self.step_draft.end_hour,
            _ => &mut self.msg_content,
        }
    }

    /// One composer field. Hand-rolled, like the sign-in card's: GPUI ships
    /// the input plumbing and no widget, so a field is a focus handle, a
    /// keystroke handler and a value.
    fn compose_field(&mut self, idx: usize, hint: &'static str, cx: &mut Context<Self>) -> AnyElement {
        let handle = match self.compose_handles[idx].clone() {
            Some(h) => h,
            None => {
                let h = cx.focus_handle();
                self.compose_handles[idx] = Some(h.clone());
                h
            }
        };
        let text = self.draft_field(idx);
        let empty = text.is_empty();
        let shown = if empty { hint.to_string() } else { text.to_string() };
        let focused = self.compose_focus == idx;
        div()
            .id(("compose-field", idx))
            .track_focus(&handle)
            .cursor(CursorStyle::IBeam)
            .h(px(AUTH_FIELD_H))
            .w_full()
            .px(px(6.0))
            .rounded(px(CONTROL_RADIUS))
            .bg(rgba(DEEP_WELL))
            .border(px(1.0))
            .border_color(rgba(if focused { RADAR_CYAN } else { HAIRLINE_SLATE }))
            .flex()
            .items_center()
            .font_family(view::FACE_HUMAN)
            .text_size(px(TEXT_BODY_PT))
            .text_color(rgba(if empty { BODY_SILVER } else { MAP_INK }))
            .on_mouse_down(MouseButton::Left, cx.listener(move |view, _, window, cx| {
                window.focus(&handle);
                view.compose_focus = idx;
                cx.notify();
            }))
            .on_key_down(cx.listener(move |view, ev: &KeyDownEvent, window, cx| {
                view.compose_key(idx, &ev.keystroke, window, cx);
            }))
            .child(shown)
            .into_any_element()
    }

    /// The keystroke half of a composer field: printable characters append,
    /// backspace removes, tab walks the six fields, Enter submits the draft.
    fn compose_key(
        &mut self,
        idx: usize,
        k: &gpui::Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match k.key.as_str() {
            "enter" => {
                // The row decides the verb: the draft submits the session,
                // the book's title row adds its scenario, its step row adds
                // its step, the comms row sends its telegram.
                if idx == 10 {
                    self.submit_telegram(cx);
                } else if idx >= 7 {
                    self.submit_step(cx);
                } else if idx == 6 {
                    self.submit_book(cx);
                } else {
                    self.submit_compose(cx);
                }
                return;
            }
            "backspace" => {
                self.draft_field_mut(idx).pop();
            }
            "tab" => {
                // Each form cycles inside itself: the draft its six, the
                // book's title row to itself, the step row its three, the
                // telegram row to itself. A tab that jumped panels would
                // land in a closed form's fields.
                let next = match idx {
                    0..=5 => (idx + 1) % 6,
                    6 => 6,
                    10 => 10,
                    _ => 7 + (idx - 7 + 1) % 3,
                };
                self.compose_focus = next;
                if let Some(h) = self.compose_handles[next].clone() {
                    window.focus(&h);
                }
            }
            _ => {
                // `key_char`, not `key`: the name is the legend on the
                // keycap and differs by layout — the card's rule, repeated
                // because a second field is a second chance to forget it.
                if let Some(text) = k
                    .key_char
                    .as_deref()
                    .filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
                {
                    self.draft_field_mut(idx).push_str(text);
                }
            }
        }
        cx.notify();
    }

    /// Submit the draft. The BINARY owns the POST — the shell has no network
    /// and never learns the base URL — so this is one message on the sink,
    /// and a nameless draft never becomes one: the server refuses it, so the
    /// panel does first, in the console's own sentence.
    fn submit_compose(&mut self, cx: &mut Context<Self>) {
        if !self.draft.name_ok() {
            self.compose_status = "name the session first".to_string();
            cx.notify();
            return;
        }
        if self.compose_busy {
            return;
        }
        self.compose_busy = true;
        self.compose_status = "creating\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::CreateSession {
            draft: self.draft.clone(),
        });
        cx.notify();
    }

    /// Apply a created session: hold the new game, clear the form, close the
    /// panel — the console's `apply_created_game`, minus the hold mechanics
    /// the binary already did (the session arrives read back, not assumed).
    /// Split out of the feed drain so a test drives exactly what the tick
    /// does — the sign-in drain's inline apply is the older shape.
    fn apply_created(&mut self, session: SessionView) {
        self.compose_busy = false;
        self.session = session;
        self.composer_open = false;
        self.draft = SessionDraft::default();
        self.compose_status.clear();
        // A new hold, unknown grant and unknown clock: the last answer
        // proved the OLD game's, not this one's.
        self.clock_running = None;
        self.clock_denied = false;
        self.pause_busy = false;
        self.pause_status.clear();
        // A created session replaces the held one, so the drafts' reference
        // moves with it — see `apply_hold`. A brand-new game holds nothing,
        // so this seeds empty drafts; it is here so the two "a new held game"
        // paths cannot disagree about seeding.
        self.reseed_drafts();
    }

    /// Submit the book's title row. The binary owns the POST, like every
    /// other write — one message on the sink — and a titleless row never
    /// becomes one, in the console's own sentence.
    fn submit_book(&mut self, cx: &mut Context<Self>) {
        if self.book_title.trim().is_empty() {
            self.book_status = "a scenario needs a title".to_string();
            cx.notify();
            return;
        }
        if self.book_busy {
            return;
        }
        self.book_busy = true;
        self.book_status = "adding\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::AddScenario {
            title: self.book_title.trim().to_string(),
        });
        cx.notify();
    }

    /// Apply the re-read book: hold the session the add returned, clear the
    /// title row, and STAY OPEN — the author keeps working, the way the
    /// console's composer stays open across adds. The composer's apply closes
    /// because a created session is a new hold; a new scenario is not. The
    /// step draft clears with it: its words are on the server now.
    fn apply_book(&mut self, session: SessionView) {
        self.book_busy = false;
        self.step_busy = false;
        self.session = session;
        self.book_title.clear();
        self.step_draft = ComposerDraft::default();
        self.book_status.clear();
    }

    /// Submit the step row to the open scenario — the book's first when none
    /// is open, the console's landing rule for an author reopening to keep
    /// working. Empty content is LEGAL (an unwritten step is an authoring
    /// state, and the panel draws it weak), so only the window rule can
    /// refuse: `problem()` is the console's own check, run here rather than
    /// re-derived.
    fn submit_step(&mut self, cx: &mut Context<Self>) {
        let open = self
            .book_open_scenario
            .or_else(|| self.session.book.first().map(|s| s.id));
        let Some(scenario_id) = open else {
            self.book_status = "add a scenario first".to_string();
            cx.notify();
            return;
        };
        if let Some(problem) = self.step_draft.problem() {
            self.book_status = problem;
            cx.notify();
            return;
        }
        if self.step_busy {
            return;
        }
        self.step_busy = true;
        self.book_status = "adding\u{2026}".to_string();
        let window = self
            .step_draft
            .window()
            .map(|(a, b)| (a.to_string(), b.to_string()));
        let _ = self.order_tx.send(ToBinary::AddStep {
            scenario_id,
            content: self.step_draft.content.clone(),
            window,
        });
        cx.notify();
    }

    /// Remove a step from its scenario. Deletes AT ONCE — harsher than the
    /// console's staged cut, and the panel's aside says so.
    fn remove_step(&mut self, scenario_id: i64, step_id: i64, cx: &mut Context<Self>) {
        if self.step_busy {
            return;
        }
        self.step_busy = true;
        self.book_status = "removing\u{2026}".to_string();
        let _ = self.order_tx.send(ToBinary::RemoveStep { scenario_id, step_id });
        cx.notify();
    }

    /// The relate editor for one step: what the step obliges (the server's
    /// read), hull checkboxes over the exercise's pieces, the nodes section
    /// honestly empty, and save/close. The console's `relate_editor_ui`
    /// without the staging: the working set posts whole on save, and the
    /// redraw comes from what the server resolved rather than what was sent.
    fn relate_editor(
        &self,
        scenario_id: i64,
        step_id: i64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let weak_line = |text: String| {
            div()
                .text_size(px(10.0))
                .text_color(rgba(BODY_SILVER.with_alpha(204)))
                .child(text)
        };
        let mut ed = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .pl(px(20.0))
            .child(weak_line("relate — hulls and nodes this step obliges".to_string()));
        match self.related_for(scenario_id, step_id) {
            None => {
                ed = ed.child(weak_line("the related list could not be read".to_string()));
            }
            Some(r) if r.personnel.is_empty() => {
                ed = ed.child(weak_line("obliges nobody yet".to_string()));
            }
            Some(r) => {
                ed = ed.child(weak_line(format!("obliges: {}", r.personnel.join(", "))));
            }
        }
        ed = ed.child(
            div()
                .text_size(px(10.0))
                .text_color(rgba(BODY_SILVER.with_alpha(178)))
                .child("hulls"),
        );
        if self.session.pieces.is_empty() {
            ed = ed.child(weak_line(
                "no hulls in the exercise yet — assign some in Fleet first".to_string(),
            ));
        }
        for piece in &self.session.pieces {
            let on = self
                .relate_working
                .contains(&("hull".to_string(), piece.id));
            let id = piece.id;
            let label_text = piece.label.clone();
            ed = ed.child(
                div()
                    .id(("relate-hull", id as usize))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.toggle_relate_item("hull", id);
                        cx.notify();
                    }))
                    .child(checkbox(on, false, label_text)),
            );
        }
        ed = ed.child(
            div()
                .text_size(px(10.0))
                .text_color(rgba(BODY_SILVER.with_alpha(178)))
                .child("task-organisation nodes"),
        );
        // The hierarchy is empty in every dev game and carries no commander,
        // so there is nothing to check against — the console's own
        // empty-state, not a missing piece.
        ed = ed.child(weak_line("no task organisation loaded".to_string()));
        ed = ed.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(
                    div()
                        .id(("relate-save", step_id as usize))
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.save_relate(cx);
                        }))
                        .child(button("save", true, self.relate_busy, false)),
                )
                .child(
                    div()
                        .id(("relate-close", step_id as usize))
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.relate_open = None;
                            cx.notify();
                        }))
                        .child(button("close", true, false, false)),
                ),
        );
        ed.into_any_element()
    }

    /// The Book panel: the held game's scenarios with their steps (read),
    /// plus the new-scenario title row (the one write).
    ///
    /// The read is the console's `book_view` without the staging: server
    /// scenarios in book order with staged cuts hidden and staged adds
    /// appended have no meaning here, because this shell stages nothing —
    /// adds post at once, and the aside below states the difference. Steps
    /// are numbered in order with their windows; empty content reads weak,
    /// because unwritten is an authoring state, not an error. Remove, relate
    /// and window editing are the staging machinery's, and are named in that
    /// sentence rather than faked.
    fn book_body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rule = || div().h(px(1.0)).w_full().bg(rgba(HAIRLINE_SLATE));
        // Content-sized, not `size_full`: this column is an ISLAND body now,
        // and the island's own scroll viewport does the measuring. A
        // `size_full` child inside a scroll viewport sizes to the viewport
        // and then clips its own overflow, which is a body that silently
        // loses its tail instead of scrolling to it.
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(PAD))
            .pr(px(CUT + 8.0));
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .h(px(TITLE_H))
                .child(
                    div()
                        .font_family(view::FACE_HUMAN)
                        .text_size(px(TITLE_SIZE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgba(MAP_INK))
                        .child("Book"),
                )
                .child(
                    div()
                        .id("close-book")
                        .cursor_pointer()
                        .text_size(px(14.0))
                        .text_color(rgba(BUTTON_LABEL))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.book_open = false;
                            cx.notify();
                        }))
                        .child("\u{00d7}"),
                ),
        );
        col = col.child(rule());
        col = col.child(
            div()
                .text_size(px(11.0))
                .text_color(rgba(BODY_SILVER))
                .child(format!(
                    "{} \u{00b7} {} scenario(s)",
                    if self.session.game.is_empty() {
                        "no session".to_string()
                    } else {
                        self.session.game.clone()
                    },
                    self.session.book.len()
                )),
        );
        col = col.child(rule());
        if self.session.book.is_empty() {
            col = col.child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgba(BODY_SILVER.with_alpha(204)))
                    .child("No scenarios yet. One game has one book."),
            );
        }
        // The open scenario's id: the tapped one, else the book's first —
        // the console's landing rule for an author reopening to keep working.
        let open_id = self
            .book_open_scenario
            .or_else(|| self.session.book.first().map(|s| s.id));
        for sc in self.session.book.clone() {
            let open = Some(sc.id) == open_id;
            col = col.child(
                div()
                    .id(("book-scenario", sc.id as usize))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.book_open_scenario = Some(sc.id);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgba(if open { RADAR_CYAN } else { MAP_INK }))
                            .child(format!("{}  -  {} step(s)", sc.title, sc.steps.len())),
                    ),
            );
            if !open {
                continue;
            }
            if sc.steps.is_empty() {
                col = col.child(
                    div()
                        .text_size(px(10.0))
                        .text_color(rgba(BODY_SILVER.with_alpha(204)))
                        .child("No steps yet. A plan may be authored while it is written."),
                );
            }
            for (n, step) in sc.steps.iter().enumerate() {
                let text = if step.content.is_empty() {
                    "(not written yet)".to_string()
                } else {
                    step.content.clone()
                };
                let window = step
                    .window
                    .as_ref()
                    .map(|(a, b)| format!("{a}\u{2013}{b}"))
                    .unwrap_or_else(|| "\u{2014}".to_string());
                let step_id = step.id;
                let scenario_id = sc.id;
                let mut row = div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .font_family(view::FACE_MACHINE)
                            .text_size(px(11.0))
                            .text_color(rgba(BODY_SILVER))
                            .child(format!("{:>2}", n + 1)),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgba(if step.content.is_empty() {
                                BODY_SILVER.with_alpha(204)
                            } else {
                                MAP_INK
                            }))
                            .child(text),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .font_family(view::FACE_MACHINE)
                            .text_size(px(10.0))
                            .text_color(rgba(BODY_SILVER))
                            .child(window),
                    );
                // The relate count, when the fan-out read this step: the
                // console's `{n} post(s)` tail. Absent is unreadable, and an
                // unreadable step claims no count rather than zero.
                if let Some(r) = self.related_for(sc.id, step.id) {
                    row = row.child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgba(BODY_SILVER.with_alpha(178)))
                            .child(format!("{} post(s)", r.posts.len())),
                    );
                }
                row = row
                    .child(
                        div()
                            .id(("book-relate", step_id as usize))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.toggle_relate(scenario_id, step_id, cx);
                            }))
                            .child(button("relate", true, false, false)),
                    )
                    .child(
                        div()
                            .id(("book-remove-step", step_id as usize))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.remove_step(scenario_id, step_id, cx);
                            }))
                            .child(button("remove", true, self.step_busy, false)),
                    );
                col = col.child(row);
                // The relate editor, inline under its step — the console's
                // `relate_editor_ui` under the row, without the staging.
                if self.relate_open == Some((sc.id, step.id)) {
                    col = col.child(self.relate_editor(sc.id, step.id, cx));
                }
            }
            // The new-step row: content plus the window, both ends or
            // neither — the console's row, with its hint and its aside.
            col = col.child(
                div()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgba(MAP_INK))
                    .child("New step"),
            );
            col = col.child(self.compose_field(7, "What must happen", cx));
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgba(BODY_SILVER))
                            .child("window"),
                    )
                    .child(div().flex_1().child(self.compose_field(8, "1000", cx)))
                    .child(div().flex_1().child(self.compose_field(9, "1030", cx)))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgba(BODY_SILVER.with_alpha(178)))
                            .child("HHMM, or leave both empty"),
                    ),
            );
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_end()
                    .child(
                        div()
                            .id("submit-step")
                            .cursor_pointer()
                            .on_click(cx.listener(|view, _, _, cx| view.submit_step(cx)))
                            .child(button("Add step", true, self.step_busy, false)),
                    ),
            );
        }
        col = col.child(rule());
        col = col.child(
            div()
                .text_size(px(11.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgba(MAP_INK))
                .child("New scenario"),
        );
        col = col.child(self.compose_field(6, "New scenario title", cx));
        let title_ok = !self.book_title.trim().is_empty();
        col = col.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(rgba(BODY_SILVER.with_alpha(178)))
                        .child("adds and removes post at once — the console stages and syncs on advance"),
                )
                .child(
                    div()
                        .id("submit-book")
                        .cursor_pointer()
                        .on_click(cx.listener(|view, _, _, cx| view.submit_book(cx)))
                        .child(button("Add scenario", true, !title_ok || self.book_busy, false)),
                ),
        );
        if !self.book_status.is_empty() {
            col = col.child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgba(STALE_SAND))
                    .child(self.book_status.clone()),
            );
        }
        col.into_any_element()
    }

    /// The marker layer, now live: one ring per tracked hull, at its
    /// projected fix position.
    ///
    /// The positions arrive in the MAP frame's pixel space (the binary
    /// projects each fix with `map_render::project_mercator`), so the layer
    /// scales them to the window by the frame's dimensions — the shell never
    /// names the map's size. Inks follow the state vocabulary the sheet
    /// uses: the selected hull carries SIGNAL, a stale one the amber, a
    /// fresh one the map ink. There is NO halo: GPUI 0.2.2 has no
    /// per-element blend mode, so the shader's `One/One` cannot be expressed,
    /// and ADR-0019 records what source-over costs. The ring is the channel
    /// either way, which ADR-0015 already said the Low tier is.
    fn markers(&mut self, win_w: f32, win_h: f32) -> impl IntoElement {
        // The symbol box the frame is drawn in, the glyph's reach as a
        // multiple of its authored radius-8 reference, and the selection
        // ring's radius (outside the frame).
        const BOX_PX: f32 = 20.0;
        const GLYPH_SCALE: f32 = 1.35;
        const SELECT_R: f32 = 15.0;
        let (mw, mh) = self.map_dims;
        let sx = if mw > 0.0 { win_w / mw } else { 0.0 };
        let sy = if mh > 0.0 { win_h / mh } else { 0.0 };
        let selected_id = self.roster.get(self.selected).map(|r| r.unit_id.clone());
        // Cloned out of `self`: the canvas closure is `move`, and holding a
        // borrow of the live list across the paint pass would fight the
        // element build.
        let live = self.live.clone();
        // The staged hulls that sit at a start and are not yet on the wire:
        // drawn as planned markers, dashed and dimmed. Cloned for the same
        // reason `live` is — the paint closure is `move`.
        let planned = self.planned_marks();
        div().absolute().top(px(0.0)).left(px(0.0)).child(canvas(
            |bounds, _, _| bounds,
            move |_bounds, _, window, _| {
                if sx <= 0.0 || sy <= 0.0 {
                    return;
                }
                for m in &live {
                    let cx = (m.x as f32) * sx;
                    let cy = (m.y as f32) * sy;
                    // Selection is a RETICLE, not a ring: four corner brackets
                    // outside the affiliation frame, with the frame itself
                    // saying which side. A plain ring says "this one" and
                    // nothing else; brackets say "this one, and here is the
                    // box it is in", which is what an operator acquires a
                    // target with. It is also the shape the reference world
                    // puts on its selected vessel, and the reason it works
                    // there is that a bracket never closes over the object —
                    // the frame and the glyph stay visible inside it.
                    if selected_id.as_deref() == Some(m.id.as_str()) {
                        paint_reticle(window, (cx, cy), SELECT_R, RADAR_CYAN);
                    }
                    // The affiliation frame (interior + cased outline), then
                    // the taxonomy glyph inside it. `planned` is false for a
                    // live hull: it is in the exercise and on the wire, and
                    // the dashed treatment is for the STAGED marks below.
                    paint_frame(window, (cx, cy), m.affiliation, false, m.stale, BOX_PX);
                    paint_glyph(
                        window,
                        (cx, cy),
                        m.symbol,
                        map_symbol_fill(m.symbol, m.stale),
                        GLYPH_SCALE,
                    );
                }
                // PLANNED hulls: a position the operator set while planning,
                // drawn before the advance writes it. Dashed and dimmed, and
                // their affiliation is Unknown — the shell has no session
                // affiliation source (the binary resolves the LIVE markers'
                // from the store), so a planned hull says "nobody has said"
                // rather than borrowing a side it cannot prove.
                for (px, py, shape) in &planned {
                    let cx = (*px as f32) * sx;
                    let cy = (*py as f32) * sy;
                    paint_frame(
                        window,
                        (cx, cy),
                        symbology::Affiliation::Unknown,
                        true,
                        true,
                        BOX_PX,
                    );
                    paint_glyph(
                        window,
                        (cx, cy),
                        *shape,
                        map_symbol_fill(*shape, true),
                        GLYPH_SCALE,
                    );
                }
            },
        ))
    }
}

/// The Web Mercator world at zoom 0, in pixels. Mirrors `map_render`'s
/// `WORLD_BASE_PX`: the shell cannot name `map_render` (the parity proto
/// carries no shim for it, deliberately — see `proto/gpui-island`), so the
/// two projections it needs are repeated here, the same precedent as the
/// shell's own `short_timestamp`. The mirror is a copy of the arithmetic,
/// never a second convention: a planned marker drawn from a start and a live
/// marker projected by the binary from a fix must share one pixel space.
const WORLD_BASE_PX: f64 = 512.0;

/// The map-frame pixel a ground position projects to, for a `w` x `h` frame
/// centred on `center` at `zoom`. The exact twin of
/// `map_render::project_mercator`, repeated for the reason in
/// [`WORLD_BASE_PX`].
fn project_frame_px(
    lat: f64,
    lon: f64,
    center: (f64, f64),
    zoom: f64,
    w: f64,
    h: f64,
) -> (f64, f64) {
    fn world(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
        let scale = WORLD_BASE_PX * 2f64.powf(zoom);
        let x = (lon + 180.0) / 360.0 * scale;
        let s = (lat.to_radians().tan() + 1.0 / lat.to_radians().cos()).ln();
        let y = (1.0 - s / std::f64::consts::PI) / 2.0 * scale;
        (x, y)
    }
    let (x, y) = world(lat, lon, zoom);
    let (cx, cy) = world(center.0, center.1, zoom);
    (x - cx + w / 2.0, y - cy + h / 2.0)
}

/// The ground a frame pixel points at — the inverse of [`project_frame_px`],
/// mirroring `map_render::unproject_mercator`. Used where a drop must carry a
/// ground `Start` even though the shell has no mercator module.
fn unproject_frame_px(
    px: f64,
    py: f64,
    center: (f64, f64),
    zoom: f64,
    w: f64,
    h: f64,
) -> (f64, f64) {
    fn world(lat: f64, lon: f64, zoom: f64) -> (f64, f64) {
        let scale = WORLD_BASE_PX * 2f64.powf(zoom);
        let x = (lon + 180.0) / 360.0 * scale;
        let s = (lat.to_radians().tan() + 1.0 / lat.to_radians().cos()).ln();
        let y = (1.0 - s / std::f64::consts::PI) / 2.0 * scale;
        (x, y)
    }
    let scale = WORLD_BASE_PX * 2f64.powf(zoom);
    let (cx, cy) = world(center.0, center.1, zoom);
    let x = px - w / 2.0 + cx;
    let y = py - h / 2.0 + cy;
    let lon = x / scale * 360.0 - 180.0;
    let s = (1.0 - 2.0 * y / scale) * std::f64::consts::PI;
    let lat = s.sinh().atan().to_degrees();
    (lat, lon)
}

/// The acquisition reticle: four corner brackets, one stroke weight.
///
/// Drawn rather than composited from a quad because GPUI 0.2.2 has no
/// per-corner border — a `PaintQuad` is one radius on all four corners, so a
/// bracket has to be four paths. That is also why the brackets are the honest
/// choice here: they are the one selection mark this toolkit can draw exactly.
///
/// `r` is the bracket's reach from the centre, and `arm` how far along each
/// edge it runs. The proportions matter more than the numbers: an arm shorter
/// than a third of the reach reads as a tick, and one that reaches the whole
/// edge is a ring again.
fn paint_reticle(window: &mut Window, center: (f32, f32), r: f32, ink: Rgba) {
    let (cx, cy) = center;
    let arm = r * 0.55;
    let c = rgba(ink);
    for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        // The corner itself, then one arm along each edge from it.
        let bx = cx + sx * r;
        let by = cy + sy * r;
        for (dx, dy) in [(-sx, 0.0f32), (0.0, -sy)] {
            let mut p = PathBuilder::stroke(px(1.5));
            p.move_to(point(px(bx), px(by)));
            p.line_to(point(px(bx + dx * arm), px(by + dy * arm)));
            if let Ok(path) = p.build() {
                window.paint_path(path, c);
            }
        }
    }
}

/// One taxonomy glyph, the port of the console's `paint_map_symbol`.
///
/// The same shapes at the same authored radii — every literal was drawn
/// against a radius-8 circle, so `scale` multiplies that reference rather
/// than naming a new radius per glyph — so a hull reads the same shape in
/// either toolkit. Filled in ONE ink: the console resolves a per-shape fill
/// from the taxonomy's symbol assignments, which needs the store; until that
/// is wired the shell draws the shape it can name, in the one state ink it
/// already has (selected / stale / fresh).
fn paint_glyph(
    window: &mut Window,
    center: (f32, f32),
    shape: MapSymbolShape,
    ink: Rgba,
    scale: f32,
) {
    use MapSymbolShape::*;
    let reach = |v: f32| v * scale;
    let c = rgba(ink);
    let poly = |window: &mut Window, pts: &[(f32, f32)]| {
        if pts.len() < 3 {
            return;
        }
        let mut p = PathBuilder::fill();
        p.move_to(point(px(pts[0].0), px(pts[0].1)));
        for (x, y) in &pts[1..] {
            p.line_to(point(px(*x), px(*y)));
        }
        p.close();
        if let Ok(path) = p.build() {
            window.paint_path(path, c);
        }
    };
    let rect = |window: &mut Window, center: (f32, f32), w: f32, h: f32| {
        let b = Bounds::new(
            point(px(center.0 - w / 2.0), px(center.1 - h / 2.0)),
            size(px(w), px(h)),
        );
        window.paint_quad(fill(b, c).corner_radii(px(1.0)));
    };
    let disc = |window: &mut Window, center: (f32, f32), r: f32| {
        let b = Bounds::new(
            point(px(center.0 - r), px(center.1 - r)),
            size(px(r * 2.0), px(r * 2.0)),
        );
        window.paint_quad(fill(b, c).corner_radii(px(r)));
    };
    match shape {
        Dot => disc(window, center, reach(3.0)),
        Destroyer => poly(
            window,
            &regular_polygon(center, reach(5.5), 3, -std::f32::consts::FRAC_PI_2),
        ),
        Frigate => poly(window, &regular_polygon(center, reach(5.2), 4, 0.0)),
        Corvette => poly(
            window,
            &regular_polygon(center, reach(4.5), 4, std::f32::consts::FRAC_PI_4),
        ),
        Auxiliary => {
            rect(window, center, reach(9.0), reach(3.0));
            rect(window, center, reach(3.0), reach(9.0));
        }
        Landing => poly(
            window,
            &regular_polygon(center, reach(5.5), 5, -std::f32::consts::FRAC_PI_2),
        ),
        Submarine => {
            let pts: Vec<(f32, f32)> = regular_polygon(center, reach(1.0), 16, 0.0)
                .into_iter()
                .map(|(x, y)| {
                    (
                        center.0 + (x - center.0) * 1.5,
                        center.1 + (y - center.1) * 0.65,
                    )
                })
                .collect();
            poly(window, &pts);
        }
        Plane => poly(
            window,
            &[
                (center.0 + reach(6.0), center.1),
                (center.0 - reach(4.0), center.1 - reach(4.5)),
                (center.0 - reach(4.0), center.1 + reach(4.5)),
            ],
        ),
        Tank => {
            rect(window, (center.0 - reach(4.0), center.1), reach(1.5), reach(8.0));
            rect(window, (center.0 + reach(4.0), center.1), reach(1.5), reach(8.0));
            rect(window, center, reach(6.0), reach(4.0));
        }
        Port => {
            let b = Bounds::new(
                point(px(center.0 - reach(5.0)), px(center.1 - reach(5.0))),
                size(px(reach(5.0) * 2.0), px(reach(5.0) * 2.0)),
            );
            window.paint_quad(
                fill(b, gpui::transparent_black())
                    .corner_radii(px(reach(5.0)))
                    .border_widths(px(2.0 * scale))
                    .border_color(c),
            );
            disc(window, center, reach(1.5));
        }
    }
}

/// The affiliation frame: the port of the console's `paint_affiliation_frame`.
///
/// The interior fills in the affiliation's own hue (mixed toward the map's
/// ink so it survives water and land), then each stroked run is CASED: a dark
/// pass pushed OUTWARD first, the hue on top. The casing goes outside, never
/// as a wider centred stroke, because a centred stroke eats the glyph's
/// pixels — the doc on `FRAME_KEYLINE_OUT_PX` has the measurement. A planned
/// hull's runs are dashed (the same frame saying "staged, not yet in the
/// exercise"), which is why `planned` is a parameter: `markers` sets it for
/// the draft's placed hulls and leaves it false for the live ones.
fn paint_frame(
    window: &mut Window,
    center: (f32, f32),
    affiliation: symbology::Affiliation,
    planned: bool,
    stale: bool,
    box_px: f32,
) {
    let frame = frame_for(affiliation);
    let c = (center.0 as f64, center.1 as f64);
    let b = box_px as f64;
    let interior: Vec<(f32, f32)> = frame_polygon(frame, c, b)
        .into_iter()
        .map(|(x, y)| (x as f32, y as f32))
        .collect();
    fill_poly(window, &interior, affiliation_fill(affiliation, stale));
    let keyline = if stale {
        FRAME_KEYLINE_STALE
    } else {
        FRAME_KEYLINE
    };
    let hue = affiliation_ink(affiliation);
    for run in frame_strokes(symbology::BattleDimension::LandAndSeaSurface, frame, c, b) {
        let pts: Vec<(f32, f32)> = run.iter().map(|(x, y)| (*x as f32, *y as f32)).collect();
        if pts.len() < 2 {
            continue;
        }
        let cased = offset_run_outward(&pts, center, FRAME_KEYLINE_OFFSET_PX);
        if planned {
            for seg in dash_run(&cased, FRAME_DASH_PX, FRAME_DASH_GAP_PX) {
                stroke_run(window, &seg, FRAME_KEYLINE_PX, keyline);
            }
            for seg in dash_run(&pts, FRAME_DASH_PX, FRAME_DASH_GAP_PX) {
                stroke_run(window, &seg, FRAME_STROKE_PX, hue);
            }
        } else {
            stroke_run(window, &cased, FRAME_KEYLINE_PX, keyline);
            stroke_run(window, &pts, FRAME_STROKE_PX, hue);
        }
    }
}

/// A closed polygon, filled.
fn fill_poly(window: &mut Window, pts: &[(f32, f32)], ink: Rgba) {
    if pts.len() < 3 {
        return;
    }
    let mut p = PathBuilder::fill();
    p.move_to(point(px(pts[0].0), px(pts[0].1)));
    for (x, y) in &pts[1..] {
        p.line_to(point(px(*x), px(*y)));
    }
    p.close();
    if let Ok(path) = p.build() {
        window.paint_path(path, rgba(ink));
    }
}

/// An open polyline, stroked at `width`.
fn stroke_run(window: &mut Window, pts: &[(f32, f32)], width: f32, ink: Rgba) {
    if pts.len() < 2 {
        return;
    }
    let mut p = PathBuilder::stroke(px(width));
    p.move_to(point(px(pts[0].0), px(pts[0].1)));
    for (x, y) in &pts[1..] {
        p.line_to(point(px(*x), px(*y)));
    }
    if let Ok(path) = p.build() {
        window.paint_path(path, rgba(ink));
    }
}

/// Split a polyline into dashes of `dash` px with `gap` px between them.
///
/// GPUI 0.2.2 has no dashed-line primitive, so the planned frame's dash is
/// walked here, measured along the run so a corner does not restart it.
fn dash_run(pts: &[(f32, f32)], dash: f32, gap: f32) -> Vec<Vec<(f32, f32)>> {
    let mut out = Vec::new();
    let mut on = true;
    let mut left = dash;
    for w in pts.windows(2) {
        let (mut ax, mut ay) = w[0];
        let (bx, by) = w[1];
        let seg = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
        if seg <= f32::EPSILON {
            continue;
        }
        let (dx, dy) = ((bx - ax) / seg, (by - ay) / seg);
        let mut travelled = 0.0;
        while travelled < seg {
            let step = left.min(seg - travelled);
            let nx = ax + dx * step;
            let ny = ay + dy * step;
            if on {
                out.push(vec![(ax, ay), (nx, ny)]);
            }
            ax = nx;
            ay = ny;
            travelled += step;
            left -= step;
            if left <= f32::EPSILON {
                on = !on;
                left = if on { dash } else { gap };
            }
        }
    }
    out
}

/// The compositing probe (U7): the SAME map image drawn twice in one frame,
/// side by side and at identical geometry, the right-hand copy carrying one
/// Signal sprite. Because both copies are in one window and one compositor
/// pass, the left copy IS the backdrop under the right copy's sprite — so
/// the capture can be checked against the model (`tint*a + dst*(1-a)`)
/// without any cross-run alignment, which the compositor's window stacking
/// makes unreliable.
///
/// Each inset carries a solid cyan tab along its top edge: that is the
/// presence check, so a capture that photographed the wrong window cannot
/// be mistaken for a result.
fn compositing_probe(
    frame: Option<Arc<gpui::RenderImage>>,
    sprite: Arc<gpui::RenderImage>,
) -> impl IntoElement {
    // 170x140 fits left of the right-docked band (x < 376).
    const W: f32 = 170.0;
    const H: f32 = 140.0;
    const Y: f32 = 440.0;
    const LX: f32 = 16.0;
    const RX: f32 = 200.0;
    let inset = |x: f32, with_sprite: bool| {
        let mut el = div()
            .absolute()
            .left(px(x))
            .top(px(Y))
            .w(px(W))
            .h(px(H))
            .overflow_hidden();
        el = el.child(
            div()
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .w(px(W))
                .h(px(3.0))
                .bg(rgba(RADAR_CYAN)),
        );
        if let Some(frame) = frame.clone() {
            el = el.child(
                div()
                    .absolute()
                    .top(px(3.0))
                    .left(px(0.0))
                    .w(px(W))
                    .h(px(H - 3.0))
                    .child(img(frame).w_full().h_full().object_fit(ObjectFit::Fill)),
            );
        }
        if with_sprite {
            // Offsets are relative to the inset, not the window: the inset
            // is itself absolutely placed, so an absolute window coordinate
            // here would double-count the origin and land outside the clip.
            let cx = W / 2.0;
            let cy = H / 2.0;
            el = el.child(
                img(sprite.clone())
                    .absolute()
                    .left(px(cx - halo::HALO_R))
                    .top(px(cy - halo::HALO_R))
                    .w(px(halo::HALO_R * 2.0))
                    .h(px(halo::HALO_R * 2.0)),
            );
        }
        el
    };
    div()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .child(inset(LX, false))
        .child(inset(RX, true))
}

/// The window's own size as the kernel's `Box`, read fresh every frame.
///
/// The shell used to spell a literal `(0.0, 0.0, 760.0, 620.0)` — the size it
/// asked the window for — which is fine while nothing resizes it and wrong the
/// moment anything does: a tiler puts the window in a tile, an operator drags
/// an edge, and the band keeps docking against a viewport that no longer
/// exists, so the chrome drifts off the surface it is supposed to fill.
/// A console is resized constantly, so this reads the window instead.
fn viewport(window: &Window) -> view::Box {
    let size = window.viewport_size();
    (0.0, 0.0, f32::from(size.width), f32::from(size.height))
}

impl Render for IslandDemo {
    // A concrete `AnyElement` where the trait allows `impl IntoElement`: the
    // shell composes one element tree per frame and returns it boxed. Legal
    // and deliberate, so the `refining_impl_trait` lint is silenced here
    // rather than by boxing at every return site.
    #[allow(refining_impl_trait)]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // The raw frame is already paintable — nothing to wait for, so this
        // is a swap, not a gate. A promotion that needed a decode would not
        // belong here; see `gate::FrameGate` for that path's numbers.
        if let Some((no, w, h, bgra, at, zoom)) = self.incoming.take() {
            self.gate.show(window, cx, no, (w, h), bgra);
            // The map's pixel space, learned from the frame itself: the live
            // markers arrive in it and are scaled to the window by it.
            self.map_dims = (w as f32, h as f32);
            // And the camera that frame was rendered from, kept rather than
            // dropped. `MapResp` has carried it all along — the binary's
            // drain read it as `_at` and discarded it — and it is the only
            // honest source for the scale bar's metres.
            self.map_center = at;
            self.map_render_zoom = zoom;
        }

        // SHEET=1: the roster sheet is the subject, so the console chrome
        // that would overlap its right edge (the reference has none) is left
        // out. The map stays — the panel is judged over real figure-ground,
        // which is the prototype's own stated reason for the backdrop.
        if !self.session.signed_in {
            return div()
                .relative()
                .size_full()
                .bg(rgba(CONSOLE_NIGHT))
                .child(self.login_card(cx))
                .into_any_element();
        }
        if self.sheet {
            return div()
                .relative()
                .size_full()
                .children(self.gate.image().map(|frame| {
                    img(frame).w_full().h_full().object_fit(ObjectFit::Fill)
                }))
                .child(if std::env::var("BUTTONS").as_deref() == Ok("1") {
                    // U5's buttons slice: the theme's button, its states
                    // and its small variant, over the real map.
                    buttons_layer().into_any_element()
                } else if std::env::var("LAYOUT").as_deref() == Ok("1") {
                    // U5's layout slice: the island body's spacing,
                    // separator, indent and row/column composition.
                    layout_layer().into_any_element()
                } else if std::env::var("CHOICE").as_deref() == Ok("1") {
                    // U5's choice slice: checkbox, toggle and radio.
                    choice_layer().into_any_element()
                } else if std::env::var("INPUT").as_deref() == Ok("1") {
                    // U5's input slice: the text field, its hint and its
                    // multiline height.
                    input_layer().into_any_element()
                } else if std::env::var("MILLER").as_deref() == Ok("1") {
                    // U5's lists slice: a Miller-columns picker.
                    miller_layer().into_any_element()
                } else if std::env::var("TOOLTIP").as_deref() == Ok("1") {
                    // U5's tooltip slice: the popup frame, gap and shadow.
                    tooltip_layer().into_any_element()
                } else if std::env::var("DRAG").as_deref() == Ok("1") {
                    // U5's drag slice: the press → dead-zone → move → ghost
                    // → release state machine, wired to real mouse events.
                    // Seeded held-and-moved so the bare capture shows the
                    // ghost + grabbing state with no synthetic input (same
                    // trick as the one-lit-rim capture).
                    if self.drag_start.is_none() {
                        self.drag_start = Some((150.0, 120.0));
                        self.drag_moved = true;
                    }
                    let held = self.drag_start.is_some();
                    let (px_, py_) = self.pointer.unwrap_or((150.0, 120.0));
                    let mut layer = div().relative().size_full();
                    // the drop zone (the map): a hover-state div, NOT faded —
                    // the shipped code drops rather than fades (see the
                    // slice doc). Cyan while a drag is held.
                    layer = layer.child(
                        div()
                            .absolute()
                            .left(px(40.0))
                            .top(px(40.0))
                            .w(px(360.0))
                            .h(px(200.0))
                            .rounded(px(6.0))
                            .border(px(1.0))
                            .border_color(rgba(if held { RADAR_CYAN } else { HAIRLINE_SLATE }))
                            .bg(rgba(CONSOLE_NIGHT.with_alpha(160)))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_size(px(TEXT_SMALL_PT))
                                    .text_color(rgba(BODY_SILVER))
                                    .child("drop zone — release over the map to place"),
                            ),
                    );
                    // the draggable row, wired to real press/move/release
                    layer = layer.child(
                        div()
                            .id("drag-row")
                            .absolute()
                            .left(px(60.0))
                            .top(px(60.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .h(px(28.0))
                            .px(px(10.0))
                            .rounded(px(4.0))
                            .bg(rgba(PANEL_SLATE))
                            .cursor(if held { CursorStyle::ClosedHand } else { CursorStyle::OpenHand })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, ev: &MouseDownEvent, _, cx| {
                                    view.drag_start =
                                        Some((f32::from(ev.position.x), f32::from(ev.position.y)));
                                    view.drag_moved = false;
                                    cx.notify();
                                }),
                            )
                            .child(
                                div()
                                    .text_size(px(TEXT_BODY_PT))
                                    .text_color(rgba(MAP_INK))
                                    .child("KRI Ahmad Yani"),
                            ),
                    );
                    // the ghost, following the pointer while held-and-moved
                    if held && self.drag_moved {
                        layer = layer.child(
                            div()
                                .absolute()
                                .left(px(px_ + 12.0))
                                .top(px(py_ + 12.0))
                                .flex()
                                .flex_row()
                                .items_center()
                                .h(px(28.0))
                                .px(px(10.0))
                                .rounded(px(4.0))
                                .bg(rgba(PANEL_SLATE.with_alpha(210)))
                                .border(px(1.0))
                                .border_color(rgba(RADAR_CYAN))
                                .occlude()
                                .child(
                                    div()
                                        .text_size(px(TEXT_BODY_PT))
                                        .text_color(rgba(MAP_INK))
                                        .child("KRI Ahmad Yani"),
                                ),
                        );
                        // the console's held-status line
                        layer = layer.child(
                            div()
                                .absolute()
                                .left(px(40.0))
                                .top(px(252.0))
                                .text_size(px(TEXT_SMALL_PT))
                                .text_color(rgba(RADAR_CYAN))
                                .child("moving KRI Ahmad Yani — release over the map to place"),
                        );
                    }
                    // the move/release overlay: alive while held, so the
                    // dead-zone latch and the ghost track the pointer even
                    // off the row (the island drag's own overlay pattern).
                    if held {
                        layer = layer.child(
                            div()
                                .absolute()
                                .top(px(0.0))
                                .left(px(0.0))
                                .right(px(0.0))
                                .bottom(px(0.0))
                                .on_mouse_move(cx.listener(|view, ev: &MouseMoveEvent, _, cx| {
                                    let now =
                                        (f32::from(ev.position.x), f32::from(ev.position.y));
                                    view.pointer = Some(now);
                                    if let Some(start) = view.drag_start {
                                        if drag_past_dead_zone(start, now) {
                                            view.drag_moved = true;
                                        }
                                    }
                                    cx.notify();
                                }))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|view, _, _, cx| {
                                        view.drag_start = None;
                                        view.drag_moved = false;
                                        cx.notify();
                                    }),
                                ),
                        );
                    }
                    layer.into_any_element()
                } else if std::env::var("WINDOW").as_deref() == Ok("1") {
                    // U5's window slice: the floating Settings window (no
                    // dim) and the modal New-session pane (dimmed) — the
                    // modal-vs-floating distinction, with the window_shadow.
                    let backdrop = std::env::var("MODAL").as_deref() == Ok("1");
                    window_layer(backdrop).into_any_element()
                } else {
                    roster_sheet(&self.roster, self.roster_total, self.selected, cx).into_any_element()
                })
                .into_any_element();
        }

        // --- the column -----------------------------------------------------
        //
        // Ownership, resolved ONCE per frame over the islands that can own
        // it: open AND on the band. A closed or culled island is gone, not
        // dimmed, and never lights. `owning_island` is kernel-tested; this
        // is its one call site. All reads go through the column scroll.
        //
        // The band itself comes from the zone rails (`zone_band_at`): the
        // column docks at the edge gap, and the grip's width writes flow
        // back into it next frame.
        //
        // WHICH islands the column holds is `view::zone_islands`' answer for
        // the state the exercise is in — the console's `side_zone`, cut to
        // the surfaces this shell has. That is the whole difference between
        // a console's column and the prototype's three identical islands:
        // the membership is a fact about the session, not a literal.
        let band = zone_band_at(viewport(window), self.zone_w, self.side_dock_left);
        // A click that opened an island may also have asked for it to be
        // scrolled onto the band. Honoured here, before the clamp, because
        // this is the frame that places it.
        if let Some(key) = self.reveal.take() {
            self.reveal_island(key, viewport(window));
        }
        let slots = self.column_slots();
        let stack_h = view::zone_stack_height(&slots);
        // The scroll ceiling is the LIVE stack over the band, never a
        // constant: the stack changes with the exercise's state, and a stale
        // ceiling is the failure the console documents — too tight strands an
        // island's last row below the band with no gesture to reach it, too
        // generous scrolls the column into empty band.
        self.zone_scroll = self.zone_scroll.clamp(0.0, view::zone_overflow(stack_h, band));
        let sc = self.zone_scroll;
        // DOCK-BOUND and STACKED: every origin is the kernel's, derived from
        // the slots above, so there is no stored position to update on a
        // dock change or a resize. This is the console's arrangement, and it
        // is why an island cannot be dragged out of the column.
        let mut rects: Vec<view::Box> = Vec::with_capacity(slots.len());
        for (i, s) in slots.iter().enumerate() {
            let origin = view::zone_slot_origin(band, &slots, i);
            rects.push(box_at(origin.0, origin.1 - sc, self.zone_w, s.height));
        }
        let owned_idx = owning_island(&rects, self.pointer);

        // The band clips; it paints nothing (a zone grows no background).
        // Islands sit band-relative inside it, in stack order. The 3px bevel
        // bleed clips at the band edge where egui bleeds 8 — same rule,
        // tighter margin.
        //
        // The column half of the wheel chain lives here too. Bodies scroll
        // themselves through the built-in handler (which no-ops, clamped,
        // at its limit), so this handler only ever moves the COLUMN: when
        // no open on-band body under the pointer has room, per
        // `chained_wheel`. One gesture, one surface — never both.
        let mut band_el = div()
            .absolute()
            .left(px(band.0))
            .top(px(band.1))
            .w(px(box_w(band)))
            .h(px(box_h(band)))
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(move |view, ev: &ScrollWheelEvent, window, cx| {
                let dy = f32::from(ev.delta.pixel_delta(window.line_height()).y);
                let px = f32::from(ev.position.x);
                let py = f32::from(ev.position.y);
                view.pointer = Some((px, py));
                let band = zone_band_at(viewport(window), view.zone_w, view.side_dock_left);
                let slots = view.column_slots();
                let sc = view.zone_scroll;
                let mut fit_opt = None;
                for (i, s) in slots.iter().enumerate() {
                    let origin = view::zone_slot_origin(band, &slots, i);
                    let r = box_at(origin.0, origin.1 - sc, view.zone_w, s.height);
                    let content = island_content_box(r);
                    if !box_contains(content, px, py) {
                        continue;
                    }
                    let Some(st) = view.islands.iter().find(|st| st.key == s.key) else {
                        continue;
                    };
                    let max = st.scroll.max_offset();
                    let viewport_h = f32::from(st.scroll.bounds().size.height);
                    fit_opt = Some(BodyFit {
                        content_h: f32::from(max.height) + viewport_h,
                        viewport_h,
                        offset: -f32::from(st.scroll.offset().y),
                    });
                    break;
                }
                // Subtract: positive wheel Y is content moving down, the
                // same direction the egui column subtracts towards.
                let ceiling = view::zone_overflow(view::zone_stack_height(&slots), band);
                if chained_wheel(fit_opt, dy, box_contains(band, px, py)) == WheelOwner::Column {
                    view.zone_scroll = (view.zone_scroll - dy).clamp(0.0, ceiling);
                    cx.notify();
                }
            }));
        for (i, s) in slots.iter().enumerate() {
            let origin = view::zone_slot_origin(band, &slots, i);
            let body = self.island_body(cx, s.key);
            band_el = band_el.child(self.island(
                cx,
                s.key,
                s.height,
                s.key.title(),
                self.island_trailing(s.key),
                body,
                Some(i) == owned_idx,
                origin,
                sc,
                band,
            ));
        }

        // The resize grip: 10px straddling the zone's inner (left) edge,
        // full band height, above the islands and below the modal — the
        // port of the grip Area plus its every-frame `move_to_top`.
        // Ink follows state: cyan while dragged, cut grey on hover, 35%
        // at rest. Dragging writes `zone_w` through the kernel rule.
        // The grip straddles the zone's INNER edge — the one facing the map.
        // A left-docked zone's inner edge is its RIGHT, and putting the grip
        // on the outer edge would park it against the window border where
        // there is nothing to drag into.
        let grip_edge = if self.side_dock_left { band.2 } else { band.0 };
        let grip_box = zone_grip_rect(band, grip_edge);
        let grip_ink = if self.grip.is_some() {
            RADAR_CYAN
        } else if self.grip_hot {
            CUT_GREY
        } else {
            CUT_GREY.with_alpha(89)
        };
        let vp = viewport(window);
        // The map's own chrome, computed once for the frame. It sits above
        // the tiles and below the panels, and it is placed in the VISIBLE map
        // — `column_w` is the docked column's live width, so a bar drawn
        // under a panel is a bar nobody can read.
        let chrome = map_chrome(
            vp.2,
            vp.3,
            if self.side_dock_left { self.zone_w } else { 0.0 },
            self.map_center,
            self.map_render_zoom,
        );
        let mut root = div()
            .relative()
            .size_full()
            // Hover tracking for ownership: every move refreshes the
            // pointer and the resolve above runs again next frame.
            .on_mouse_move(cx.listener(|view, ev: &MouseMoveEvent, _, cx| {
                view.pointer = Some((f32::from(ev.position.x), f32::from(ev.position.y)));
                cx.notify();
            }))
            .child(
                // The map surface (U6), painted first so it sits behind
                // every piece of chrome. `Fill` (not `Contain`): this is a
                // viewport, and a letterboxed map would show the black it
                // is meant to replace.
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    // The map surface is the PAN HANDLE, and being the element
                    // painted first is what makes it the right one: GPUI hits
                    // the deepest element first, so a press over an island, a
                    // panel or the top zone never reaches here — panning is
                    // exactly the gestures on bare map, with no exclusion list
                    // to keep in step with the chrome.
                    .id("map-surface")
                    // Armed, the surface shows it: a crosshair where a pan
                    // would be, because the next click that stays put places
                    // instead of panning.
                    .cursor(if self.pending_place.is_some() {
                        CursorStyle::Crosshair
                    } else {
                        CursorStyle::Arrow
                    })
                    .on_mouse_down(MouseButton::Left, cx.listener(|view, ev: &MouseDownEvent, _, _| {
                        let at = (f32::from(ev.position.x), f32::from(ev.position.y));
                        // Armed, the press anchors instead of panning: only a
                        // press that stays put places, and a press that moves
                        // past 6px becomes a pan like any other.
                        if view.pending_place.is_some() {
                            view.place_down = Some(at);
                        } else {
                            view.pan = Some(at);
                        }
                    }))
                    .on_mouse_move(cx.listener(|view, ev: &MouseMoveEvent, window, cx| {
                        let (x, y) = (f32::from(ev.position.x), f32::from(ev.position.y));
                        if let Some((ax, ay)) = view.place_down {
                            if (x - ax).hypot(y - ay) > 6.0 {
                                view.pan = Some((x, y));
                                view.place_down = None;
                            } else {
                                return;
                            }
                        }
                        let Some((lx, ly)) = view.pan else {
                            return;
                        };
                        let (mw, mh) = view.map_dims;
                        let vp = viewport(window);
                        view.pan = Some((x, y));
                        if mw > 0.0 && mh > 0.0 && vp.2 > 0.0 && vp.3 > 0.0 {
                            view.pan_by(
                                f64::from((x - lx) * mw / vp.2),
                                f64::from((y - ly) * mh / vp.3),
                                cx,
                            );
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|view, ev: &MouseUpEvent, window, cx| {
                        // The armed click: a press that never became a pan
                        // deploys the armed hull at the release fractions.
                        // Strict, unlike the drop: the arm was struck on a
                        // drafted row, and a hull that left the exercise
                        // since is refused rather than silently re-added by
                        // a click.
                        if view.place_down.take().is_some() {
                            let vp = viewport(window);
                            if vp.2 > 0.0 && vp.3 > 0.0 {
                                let fx = f64::from(f32::from(ev.position.x) / vp.2)
                                    .clamp(0.0, 1.0);
                                let fy = f64::from(f32::from(ev.position.y) / vp.3)
                                    .clamp(0.0, 1.0);
                                if let Some(unit_id) = view.pending_place {
                                    if view.session.pieces.iter().any(|p| p.id == unit_id) {
                                        // The staged line names the hull, so
                                        // the armed click carries the label the
                                        // session read holds — the same value
                                        // `live_hulls` mirrors.
                                        let name = view
                                            .session
                                            .pieces
                                            .iter()
                                            .find(|p| p.id == unit_id)
                                            .map(|p| p.label.clone())
                                            .unwrap_or_default();
                                        view.send_deploy(unit_id, &name, fx, fy, cx);
                                        return;
                                    }
                                    view.force_status =
                                        "that hull left the exercise \u{2014} add it again first"
                                            .to_string();
                                }
                            }
                            view.pending_place = None;
                            cx.notify();
                            return;
                        }
                        if view.pan.take().is_some() {
                            cx.notify();
                        }
                    }))
                    // The drop side of the picker's drag: a hull released
                    // over bare map deploys there. The predicate names the
                    // payload type, so stray drags (there are no others yet,
                    // but the zone grip will grow one) never land here. The
                    // handler runs without the view, so it re-enters through
                    // the entity — the same re-entry every drain uses.
                    .can_drop(|v, _, _| v.is::<UnitDragPayload>())
                    .on_drop({
                        let demo = cx.entity();
                        move |payload: &UnitDragPayload, window: &mut Window, app: &mut App| {
                            let size = window.viewport_size();
                            let (w, h) = (f32::from(size.width), f32::from(size.height));
                            if w <= 0.0 || h <= 0.0 {
                                return;
                            }
                            let at = window.mouse_position();
                            let fx = f64::from((f32::from(at.x) / w).clamp(0.0, 1.0));
                            let fy = f64::from((f32::from(at.y) / h).clamp(0.0, 1.0));
                            let unit_id = payload.unit_id;
                            let unit_name = payload.name.clone();
                            let _ = demo.update(app, |view, cx| {
                                // A drop on a host embarks; anywhere else
                                // deploys — and a hull outside the exercise
                                // is assigned on the way in, because placing
                                // IS assigning. The host is the nearest
                                // placed piece's marker, so a drop between
                                // hulls still names water, not a neighbour.
                                let host = view.drop_host_at(fx, fy);
                                view.drop_unit(unit_id, &unit_name, fx, fy, host, cx);
                            });
                        }
                    })
                    .children(
                        self.gate
                            .image()
                            .map(|frame| img(frame).w_full().h_full().object_fit(ObjectFit::Fill)),
                    ),
            )
            // The marker layer, as GPUI has to draw it: the epaint state
            // rings, and no shader halo. ADR-0019 has the numbers for why
            // the halo does not come across; the rings are the channel
            // either way, which is what ADR-0015 already said the Low tier
            // is. Two markers additionally carry a SOURCE-OVER sprite — not
            // because it is adopted, but so a capture can be checked against
            // the modelled difference.
            .child(if std::env::var("PROBE").as_deref() == Ok("1") {
                // The one-frame A/B, instead of the marker layer: no
                // cross-run alignment, no second compositor pass.
                compositing_probe(self.gate.image(), self.probe_sprite.clone()).into_any_element()
            } else {
                self.markers(vp.2, vp.3).into_any_element()
            })
            .child(band_el)
            .child(
                div()
                    .absolute()
                    .left(px(grip_box.0))
                    .top(px(grip_box.1))
                    .w(px(ZONE_GRIP_W))
                    .h(px(box_h(grip_box)))
                    .id("zone-grip")
                    .cursor(CursorStyle::ResizeLeftRight)
                    .flex()
                    .justify_center()
                    // An explicit height, not `h_full()`. The rule was
                    // `h_full()` AND `mt(8)` AND `mb(8)` — 100% of the grip's
                    // height plus 16pt of margin, which overflows the flex
                    // cross-axis and resolves to a few stray pixels instead of
                    // the full-band rule `zone_grip_rect` specifies. That is
                    // why the dock-resize handle is undiscoverable: the
                    // gesture works, the affordance does not exist. Measured
                    // before: 5-6 grey px per column at x 357-363.
                    .child(
                        div()
                            .w(px(2.0))
                            .flex_shrink_0()
                            .h(px((box_h(grip_box) - 16.0).max(4.0)))
                            .bg(rgba(grip_ink)),
                    )
                    .on_hover(cx.listener(|view, hot: &bool, _, cx| {
                        view.grip_hot = *hot;
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, ev: &MouseDownEvent, _, cx| {
                            view.grip = Some((f32::from(ev.position.x), view.zone_w));
                            cx.notify();
                        }),
                    ),
            );
        // --- the top band ---------------------------------------------------
        //
        // The session's own bar, and the console's composition rather than a
        // row of verbs: what the link is doing, who is in the room, which
        // game is held, the camera's handle — and at the right end, in the
        // console's own right-to-left group, the way out and the two doors.
        //
        // The prototype's nine-button strip (`OPEN ISLANDS`, `OPEN MODAL`,
        // `PEOPLE`, `MY COMMAND`, ...) is GONE, and that removal is the whole
        // of this slice's point. Those buttons existed because the surfaces
        // had no place: a demo's answer to "where does this panel go" is a
        // button that opens it. Now they have places — the column for what
        // the operator reads, a modal for what they author — and the band
        // carries only the doors the console's band carries.
        // The map's chrome, added the same way the grip is: after the chain,
        // not inside it.
        //
        // This is not a style preference. Added as `.children(chrome)` inside
        // the initial chain it did not paint — not misplaced, not clipped, not
        // behind the tiles: a 200x40 solid red box at (400,700) came out as
        // bare map, across a build where `map_chrome` had run 991 frames and
        // provably pushed the element. The grip, added this way, paints. So
        // the chrome goes this way too, and the reason is recorded rather
        // than guessed at.
        //
        // It sits above the tiles and below the panels: the scale bar and the
        // north mark are map furniture, and a bar painted over a panel is a
        // bar nobody can read.
        for el in chrome {
            root = root.child(el);
        }
        root = root.child(self.top_zone(cx, vp.2));
        // The step reminder, just under the top band: the ONE step this
        // caller owes or is on — never the book. A click opens the column's
        // STEP island, where the scenario and the declare row live, so the
        // strip never grows its own. Absent when there is nothing to remind:
        // no execution, no asking step, nothing playing.
        let reminder = self.step_reminder();
        if let Some(rem) = reminder {
            // The console's own pair: sand while the step waits on this
            // caller, signal green once declared or when only naming play.
            let dot = if rem.needs_you { WARNING_SAND } else { SIGNAL_GREEN };
            root = root.child(
                div()
                    .absolute()
                    .top(px(34.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .h(px(22.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .overflow_hidden()
                    .bg(rgba(Rgba::black_alpha(120)))
                    .id("step-reminder")
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.open_step_detail(cx);
                    }))
                    .child(
                        div()
                            .w(px(8.0))
                            .h(px(8.0))
                            .rounded(px(4.0))
                            .bg(rgba(dot))
                            .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .font_family(view::FACE_HUMAN)
                            .text_size(px(11.0))
                            .text_color(rgba(MAP_INK))
                            .child(rem.text),
                    ),
            );
        }
        // The map readout, under a seed and not always on.
        //
        // It is genuinely useful: one line that says the surface under all of
        // this is the map thread's output and not a static picture, which
        // makes a capture self-describing. It is also a developer's line
        // floating over the map, and the console has no such thing — so it is
        // a capture aid rather than chrome. `TFG_READOUT=1` turns it on.
        if std::env::var("TFG_READOUT").as_deref() == Ok("1") {
        let shown = self.gate.shown();
        let retired = self.gate.retired();
        root = root.child(
            div()
                .absolute()
                .top(px(84.0))
                .left(px(8.0))
                .px(px(8.0))
                .py(px(4.0))
                .bg(rgba(Rgba::black_alpha(160)))
                .font_family("monospace")
                .text_size(px(11.0))
                .text_color(rgba(BUTTON_LABEL))
                .child(format!(
                    "map raw frame {shown} ret {retired} | {} live markers | rings only: \
 no halo (ADR-0019)",
                    self.live.len()
                )),
        );
        }
        // The grip's drag overlay: full-window, transparent, topmost, alive
        // while the column is being resized. Moves and the release land here
        // even when the cursor has left the 10px strip, which is what keeps a
        // fast drag attached. Rendered late for the same reason the close
        // button is: later siblings win hits.
        //
        // There is no ISLAND drag overlay any more, and that is the point of
        // the column: an island's place is the stack's, so there is nothing
        // to drag. The grip is the one gesture that moves the column's
        // geometry, and the wheel is the one that moves its content.
        let grip_drag = self.grip;
        if grip_drag.is_some() {
            root = root.child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    // The listener's own `&mut Window`: the outer one cannot
                    // be borrowed into a closure that outlives the method,
                    // which is the real reason this site reads it here
                    // rather than from `render`'s parameter.
                    .on_mouse_move(cx.listener(move |view, ev: &MouseMoveEvent, window, cx| {
                        // Right dock grows leftward: the kernel rule takes
                        // the grab width and the cursor delta.
                        if let Some((grab_x, start_w)) = grip_drag {
                            view.zone_w = zone_drag_width(
                                start_w,
                                f32::from(ev.position.x) - grab_x,
                                !view.side_dock_left,
                                f32::from(window.viewport_size().width),
                            );
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            view.grip = None;
                            cx.notify();
                        }),
                    ),
            );
        }
        // The Inspector dock, last so it sits over the map and under the
        // modal layer: a dock is chrome the panels cover, not the reverse.
        if self.inspect_open {
            root = root.child(self.inspector(cx));
        }
        // --- the modal zone ------------------------------------------------
        //
        // ONE modal at a time, the console's own `open_only` rule. The
        // prototype could show every panel at once, which is what a row of
        // open buttons produces: several dimmed cards stacked, each half
        // behind the next, and no way to tell which one the last click
        // belonged to. The rule is enforced here rather than trusted to each
        // door, because a rule every caller must remember is a rule that gets
        // forgotten the moment a sixth door appears.
        //
        // The column's islands are NOT in this rule and never were: they are
        // surfaces the operator reads beside the map, not dialogs that take
        // the frame. Settings, the session composer and the book are the
        // three that AUTHOR, and authoring takes the frame.
        let modal: Option<AnyElement> = if self.player_open {
            Some(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(Rgba::black_alpha(150)))
                    .child(
                        div()
                            .relative()
                            .w(px(FORM_W))
                            .h(px(FORM_H))
                            .child(modal_silhouette_at(FORM_W, FORM_H))
                            .child(self.player_picker_body(cx)),
                    )
                    .into_any_element(),
            )
        } else if self.fleet_open && !self.dragging {
            Some(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(Rgba::black_alpha(150)))
                    .child(
                        div()
                            .relative()
                            .w(px(FLEET_W))
                            .h(px(FLEET_H))
                            // The card truncates hover: without this, a press
                            // anywhere on the picker ALSO lands on the
                            // full-window map surface underneath (GPUI
                            // delivers to every hovered hitbox down the
                            // chain), so every picker drag pans the map along
                            // with the gesture. `except_scroll` keeps the
                            // columns' own scrolling, the way the island edge
                            // keeps its body's.
                            .block_mouse_except_scroll()
                            .child(modal_silhouette_at(FLEET_W, FLEET_H))
                            .child(self.fleet_picker_body(cx)),
                    )
                    .into_any_element(),
            )
        } else if self.composer_open {
            // The form is taller than any readout panel, so it gets the sized
            // silhouette rather than the readout card — same chamfer, same
            // dim, its own size. Shrinking it onto the readout card shipped a
            // cramped lie about the layout (the book's first capture proved
            // it, with the title row clipped off the card's bottom).
            Some(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(Rgba::black_alpha(150)))
                    .child(
                        div()
                            .relative()
                            .w(px(FORM_W))
                            .h(px(FORM_H))
                            .child(modal_silhouette_at(FORM_W, FORM_H))
                            .child(self.composer_body(cx)),
                    )
                    .into_any_element(),
            )
        } else if self.book_open {
            // The book is an authoring form too — scenarios and steps are
            // read, the title and step rows write — so it shares the
            // authoring card for the same reason.
            Some(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(Rgba::black_alpha(150)))
                    .child(
                        div()
                            .relative()
                            .w(px(FORM_W))
                            .h(px(FORM_H))
                            .child(modal_silhouette_at(FORM_W, FORM_H))
                            .child(self.book_body(cx)),
                    )
                    .into_any_element(),
            )
        } else if self.comms_open {
            // The inbox, on the authoring card rather than the readout card:
            // it holds a message list AND the composer row, which is the
            // same shape as the book and for the same reason.
            Some(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(Rgba::black_alpha(150)))
                    .child(
                        div()
                            .relative()
                            .w(px(FORM_W))
                            .h(px(FORM_H))
                            .child(modal_silhouette_at(FORM_W, FORM_H))
                            .child(self.comms_body(cx)),
                    )
                    .into_any_element(),
            )
        } else if self.embark_host.is_some() {
            // One hull's passengers, on the readout card rather than the
            // authoring form: the read half carries no compose, and the
            // console's own island is 320x280 — the small card, honestly.
            Some(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(Rgba::black_alpha(150)))
                    .child(
                        div()
                            .relative()
                            .w(px(MODAL_W))
                            .h(px(MODAL_H))
                            .child(modal_silhouette())
                            .child(self.embark_body(cx)),
                    )
                    .into_any_element(),
            )
        } else if self.modal_open {
            Some(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(rgba(Rgba::black_alpha(150)))
                    .child(
                        div()
                            .relative()
                            .w(px(MODAL_W))
                            .h(px(MODAL_H))
                            .child(modal_silhouette())
                            .child(self.settings_body(cx)),
                    )
                    .into_any_element(),
            )
        } else {
            None
        };
        if let Some(m) = modal {
            root = root.child(m);
        }
        root.into_any_element()
    }
}

/// The console's button, from `apply_ops_theme` (U5's buttons slice).
///
/// Every value is the theme's, not a guess: fill `BUTTON_GRAPHITE`, a 1px
/// border that rests `HAIRLINE_SLATE` and lifts `CUT_GREY` on hover and
/// `RADAR_CYAN` on press, corner radius `CONTROL_RADIUS` (6), label ink
/// `BUTTON_LABEL` lifting to `BUTTON_LABEL_HOVER`, at `TEXT_BUTTON_PT` (15)
/// in the proportional family — the shipped console draws egui's stock
/// proportional face, so the GPUI port draws the registered Inter rather
/// than the monospace family. Padding is `BUTTON_PADDING` (12, 8); the
/// height is content-driven (text line + padding), matching egui.
///
/// `small` is `ui.small_button`: the same fill/border/radius and ink at
/// `TEXT_SMALL_PT` (10.5), which is the whole difference — egui's
/// small_button is a stock button drawn one size down.
///
/// Disabled paints a dimmed face (dimmed fill + dimmed ink + hairline
/// border) with no hover/active states — `add_enabled(false, …)`. NOTE
/// (the U5 trap, handoff §5 panel 2): GPUI `Button`-from-divs carries no
/// inner shadow the way a disabled egui icon button does, so a port that
/// ships an enabled-looking disabled button is WRONG here; the dim is the
/// port.
fn button(label: impl Into<SharedString>, small: bool, disabled: bool, hot: bool) -> impl IntoElement {
    // Tracked caps, and the decision is deliberate: the reference labels its
    // verbs `SIGN OUT`, `APPLY`, `NEW SESSION`, and a lowercase label on a
    // graphite pill reads as a form control rather than an instrument. Done
    // here rather than at 33 call sites so the whole surface moves together.
    //
    // Tracking is NOT done here, and that is a known gap: `tracked()` buys its
    // letter spacing by splitting a label into one div per character, which is
    // right for a section header and would put hundreds of nodes on a row of
    // pills. Uppercase carries most of the effect; the spacing is owed.
    let label = view::label_case(label.into().as_ref());
    let size = if small { TEXT_SMALL_PT } else { TEXT_BUTTON_PT };
    // The pill's HEIGHT, and it was the other half of the same defect: with
    // egui's `BUTTON_PADDING` (12 x 8) and no line height of its own, a
    // rendered pill measured 48pt against the reference's 22pt — 2.2x too
    // deep, which is what made `Enter execution` read as a slab rather than a
    // verb. Padding to 4 and a line height tied to the face size rather than
    // left to the shaper's default brings a small pill to ~24pt. The horizontal
    // padding stays egui's: a pill that is too narrow is a different defect.
    let mut el = div()
        .flex()
        .items_center()
        .justify_center()
        .px(px(BUTTON_PADDING.0))
        .py(px(4.0))
        .rounded(px(CONTROL_RADIUS))
        .border(px(1.0))
        .font_family(view::FACE_HUMAN)
        .text_size(px(size))
        .line_height(px(size * 1.25));
    if disabled {
        el = el
            .bg(rgba(BUTTON_GRAPHITE.with_alpha(140)))
            .border_color(rgba(HAIRLINE_SLATE))
            .text_color(rgba(BUTTON_LABEL.with_alpha(120)));
    } else {
        el = el.bg(rgba(BUTTON_GRAPHITE)).border_color(rgba(HAIRLINE_SLATE));
        if hot {
            el = el
                .bg(rgba(RADAR_CYAN.with_alpha(40)))
                .border_color(rgba(CUT_GREY));
        }
        el = el.text_color(rgba(BUTTON_LABEL));
    }
    // The wrapper is the fix, and it belongs here rather than at any call
    // site: this element lands as a child of a `flex_col`, whose `align-items`
    // is `stretch` by default, so an unstyled button fills the column's full
    // width. That is why `Enter execution \u{2192}` in the Review panel was a
    // full-width slab on a 300pt panel while every other verb in the console
    // was a 22pt pill. GPUI 0.2.2 has no `align_self`, so the stretch can only
    // be removed from the outside — a row does not stretch its children, so the
    // pill inside it takes its content width. No call site wanted a full-width
    // button: all 33 are labelled verbs.
    div()
        .flex()
        .flex_row()
        .justify_start()
        .child(el.child(label))
}

/// A `ui.separator()`: a hairline rule across the body, egui's default
/// separator stroke at the theme's `HAIRLINE_SLATE`. In the reference it is
/// the full inner width, one device px, sitting between the header block and
/// the body below it.
fn separator() -> impl IntoElement {
    div().h(px(1.0)).w_full().bg(rgba(HAIRLINE_SLATE))
}

/// A choice widget (U5's choice slice), from egui's `ui.checkbox` /
/// `ui.toggle_value` at the app theme. The box is `icon_width` (14) square —
/// `spacing.icon_width`, `icon_width_inner` (8) for the mark — with the
/// control's corner radius (6), fill `BUTTON_GRAPHITE` and a 1px
/// `HAIRLINE_SLATE` border, the label `icon_spacing` (4) to its right at
/// `TEXT_BODY_PT`. The mark is egui's check path (a down-then-up chevron),
/// `BUTTON_LABEL` for a plain checkbox and `DEEP_WELL` for a toggle whose
/// active state fills the box `RADAR_CYAN` with a cyan border.
///
/// `toggle` selects `ui.toggle_value` (active = cyan fill + dark mark)
/// over `ui.checkbox` (mark on graphite). Geometry is identical: egui's
/// toggle_value IS a checkbox drawn one interaction state differently.
fn checkbox(checked: bool, toggle: bool, label: impl Into<SharedString>) -> impl IntoElement {
    let ink = if toggle && checked {
        DEEP_WELL
    } else {
        BUTTON_LABEL
    };
    let (fill, border) = if toggle && checked {
        (RADAR_CYAN, RADAR_CYAN)
    } else {
        (BUTTON_GRAPHITE, HAIRLINE_SLATE)
    };
    // egui's check path, scaled to the 8px inner box: left-mid down to
    // bottom-mid, up to right-top. Painted through `canvas` because no box
    // primitive draws a chevron. The canvas fills the 14px box, so the
    // path is relative to that box's origin.
    let mark = canvas(
        |b, _, _| b,
        move |b, _, window, _| {
            let (x0, y0) = (f32::from(b.origin.x), f32::from(b.origin.y));
            let p = |x: f32, y: f32| point(px(x), px(y));
            let mut path = PathBuilder::stroke(px(1.6));
            path.move_to(p(x0 + 3.0, y0 + 7.0));
            path.line_to(p(x0 + 6.0, y0 + 10.0));
            path.line_to(p(x0 + 11.0, y0 + 4.0));
            if let Ok(path) = path.build() {
                window.paint_path(path, rgba(ink));
            }
        },
    )
    .size(px(14.0));
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .child(
            div()
                .w(px(14.0))
                .h(px(14.0))
                .rounded(px(CONTROL_RADIUS))
                .bg(rgba(fill))
                .border(px(1.0))
                .border_color(rgba(border))
                .flex()
                .items_center()
                .justify_center()
                .children(if checked { Some(mark) } else { None }),
        )
        .child(
            div()
                .text_size(px(TEXT_BODY_PT))
                .text_color(rgba(BODY_SILVER))
                .child(label.into()),
        )
}

/// A radio button (U5's choice slice), from egui's `ui.radio_value`. The
/// circle is the same `icon_width` (14) box, corner radiused to a circle,
/// fill `BUTTON_GRAPHITE` + 1px `HAIRLINE_SLATE` at rest; selected fills
/// `selection.bg_fill` (`RADAR_CYAN`) with a `selection.stroke`
/// (`DEEP_WELL`) ring and a `fg_selected` (white) inner dot at
/// `icon_width_inner` (8). Same box geometry as the checkbox — the two are
/// one widget drawn differently, which is why they share `icon_width`.
fn radio(selected: bool, label: impl Into<SharedString>) -> impl IntoElement {
    let (fill, border) = if selected {
        (RADAR_CYAN, DEEP_WELL)
    } else {
        (BUTTON_GRAPHITE, HAIRLINE_SLATE)
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(4.0))
        .child(
            div()
                .w(px(14.0))
                .h(px(14.0))
                .rounded(px(7.0))
                .bg(rgba(fill))
                .border(px(1.0))
                .border_color(rgba(border))
                .flex()
                .items_center()
                .justify_center()
                .children(if selected {
                    Some(
                        div()
                            .w(px(4.0))
                            .h(px(4.0))
                            .rounded(px(2.0))
                            .bg(rgba(Rgba::rgb(0xFF, 0xFF, 0xFF))),
                    )
                } else {
                    None
                }),
        )
        .child(
            div()
                .text_size(px(TEXT_BODY_PT))
                .text_color(rgba(BODY_SILVER))
                .child(label.into()),
        )
}

/// The U5 choice slice: the checkbox, the toggle and the radio, each in both
/// states, over the real map. Static states, seeded like the button slice's
/// hover/active paints — no synthetic input, the states are booleans.
/// Reference: `examples/choice_probe` rendered at the app theme (a real
/// egui `ui.checkbox`/`ui.toggle_value`/`ui.radio_value`), which is the
/// console's own choice vocabulary.
fn choice_layer() -> impl IntoElement {
    div()
        .absolute()
        .left(px(24.0))
        .top(px(24.0))
        .p(px(12.0))
        .rounded(px(8.0))
        .bg(rgba(Rgba::black_alpha(150)))
        .flex()
        .flex_col()
        .gap(px(ITEM_SPACING.1))
        .child(checkbox(true, false, "Geographic grid"))
        .child(checkbox(false, false, "Reduce motion"))
        .child(checkbox(true, true, "Side zone (toggle)"))
        .child(checkbox(false, true, "Reduce motion (toggle)"))
        .child(radio(true, "option A"))
        .child(radio(false, "option B"))
}

/// A text field (U5's input slice), from egui's `TextEdit` at the app
/// theme. The box is `DEEP_WELL` (`visuals.extreme_bg_color`) with a 1px
/// `HAIRLINE_SLATE` border and the control's radius (6); the text is
/// `TEXT_BODY_PT` in `MAP_INK`, vertically centered, with `text_edit`
/// padding (`ui.spacing().text_edit_width` governs only width, not this
/// inner pad — measured 4px each side). A `hint` is placeholder ink at
/// `BODY_SILVER`, shown only while the field is empty. `multiline` makes
/// the box taller (the reference's 3-row field is ~53px vs the singleline
/// 19px) and top-aligns the text instead of centering it.
///
/// This ports the field's LOOK (fill, border, radius, ink, hint, the
/// singleline/multiline height), not an editable text buffer — GPUI input
/// ownership is its own surface and is deliberately out of scope for this
/// geometry slice. The reference is `examples/input_probe`, a real egui
/// `TextEdit` at the app theme.
fn text_field(value: impl Into<SharedString>, hint: impl Into<SharedString>, multiline: bool) -> impl IntoElement {
    let value: SharedString = value.into();
    let empty = value.len() == 0;
    let shown: SharedString = if empty { hint.into() } else { value };
    let ink = if empty { BODY_SILVER } else { MAP_INK };
    let mut box_el = div()
        .w_full()
        .px(px(4.0))
        .rounded(px(CONTROL_RADIUS))
        .bg(rgba(DEEP_WELL))
        .border(px(1.0))
        .border_color(rgba(HAIRLINE_SLATE))
        .font_family(view::FACE_HUMAN)
        .text_size(px(TEXT_BODY_PT))
        .text_color(rgba(ink));
    box_el = if multiline {
        // Wrap: without it a long line runs past the panel's right edge
        // instead of breaking — visible immediately in the capture.
        box_el
            .min_h(px(48.0))
            .py(px(4.0))
            .items_start()
            .overflow_hidden()
            .child(
                div()
                    .w_full()
                    .text_size(px(TEXT_BODY_PT))
                    .child(shown),
            )
    } else {
        box_el
            .h(px(19.0))
            .flex()
            .items_center()
            .child(shown)
    };
    box_el
}

/// The U5 input slice: singleline (empty-with-hint, filled, password-style),
/// and a multiline field, over the real map. The password field is drawn as
/// its mask (`••••••`) because the field LOOK is identical to singleline —
/// egui's `.password(true)` changes the text, not the box — so the port
/// carries the box and shows the masked content. Reference:
/// `examples/input_probe` at the app theme.
fn input_layer() -> impl IntoElement {
    let field_label = |t: &'static str| {
        div()
            .text_size(px(TEXT_BODY_PT))
            .text_color(rgba(MAP_INK))
            .child(t)
    };
    div()
        .absolute()
        .left(px(24.0))
        .top(px(24.0))
        .w(px(300.0))
        .p(px(12.0))
        .rounded(px(8.0))
        .bg(rgba(Rgba::black_alpha(150)))
        .flex()
        .flex_col()
        .gap(px(ITEM_SPACING.1))
        .child(field_label("singleline"))
        .child(text_field("", "search", false))
        .child(field_label("filled"))
        .child(text_field("KRI Ahmad Yani", "", false))
        .child(field_label("password"))
        .child(text_field("••••••••", "", false))
        .child(field_label("multiline"))
        .child(text_field("a session for photographing the console\nverify the three modals", "", true))
}

/// A Miller-columns list (U5's lists slice): the Fleet picker's taxonomy
/// column plus its leaf column, drawn from the geometry the reference
/// (`examples/miller_probe`) measured.
///
/// The numbers, all measured against that probe at the app theme:
/// - the console's `picker_row_height` is **39pt** here (`max(label,
///   button) + item_spacing.y`; a themed button is 39pt in this egui 0.36
///   build, not the 22 the handoff's earlier note guessed — that note is
///   corrected below).
/// - the LEAF row's actual pitch is **46pt** (name + weak subtitle + a
///   28px thumbnail + an assign `small_button`), measured assign-top to
///   assign-top: 111 / 157 / 203.
/// - a `small_button` (assign) is 19pt; a category `selectable_value` is
///   the stock 18pt button.
///
/// The port draws the taxonomy column (selected = cyan `selection` fill +
/// deep-well ink, the rest graphite/hairline) and the leaf column (28px
/// `PANEL_SLATE` thumbnail disc, name at body, weak subtitle at small, an
/// assign `small_button`, the armed row ringed 1px `RADAR_CYAN` expand-2
/// r3 — the hand-painted selection `picker_leaf_row_ui` uses). GPUI's
/// virtual list is the plan's suggestion for the real 290-line drill; this
/// slice draws the row vocabulary at the measured pitch.
fn miller_layer() -> impl IntoElement {
    let cat = |name: String, selected: bool| {
        let (fill, border, ink) = if selected {
            (RADAR_CYAN, RADAR_CYAN, DEEP_WELL)
        } else {
            (BUTTON_GRAPHITE, HAIRLINE_SLATE, BUTTON_LABEL)
        };
        div()
            .px(px(10.0))
            .py(px(4.0))
            .rounded(px(CONTROL_RADIUS))
            .bg(rgba(fill))
            .border(px(1.0))
            .border_color(rgba(border))
            .font_family(view::FACE_HUMAN)
            .text_size(px(TEXT_BUTTON_PT))
            .text_color(rgba(ink))
            .child(SharedString::from(name))
    };
    // One leaf row: thumbnail, name+subtitle, assign; armed is ringed.
    // The row is the reference's 46pt pitch; `top` is where it sits in the
    // column. Absolute (the roster sheet's technique) because a flex
    // column's `.h()` does not constrain a row — the first two captures
    // measured a 60px pitch and an ellipse thumbnail before this.
    let leaf = |name: String, sub: String, armed: bool, top: f32| {
        let mut row = div()
            .absolute()
            .left(px(0.0))
            .top(px(top))
            .w_full()
            .h(px(46.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(
                div()
                    .w(px(28.0))
                    .h(px(28.0))
                    .flex_shrink_0()
                    .rounded(px(14.0))
                    .bg(rgba(PANEL_SLATE)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .child(
                        div()
                            .text_size(px(TEXT_SMALL_PT))
                            .text_color(rgba(MAP_INK))
                            .child(SharedString::from(name)),
                    )
                    .child(
                        div()
                            .text_size(px(TEXT_SMALL_PT))
                            .text_color(rgba(BODY_SILVER))
                            .child(SharedString::from(sub)),
                    ),
            )
            .child(button("assign", true, false, false));
        if armed {
            row = row.child(
                div()
                    .absolute()
                    .left(px(-2.0))
                    .top(px(-2.0))
                    .right(px(-2.0))
                    .bottom(px(-2.0))
                    .rounded(px(3.0))
                    .border(px(1.0))
                    .border_color(rgba(RADAR_CYAN)),
            );
        }
        row
    };
    div()
        .absolute()
        .left(px(24.0))
        .top(px(24.0))
        .w(px(520.0))
        .p(px(12.0))
        .rounded(px(8.0))
        .bg(rgba(Rgba::black_alpha(150)))
        .flex()
        .flex_row()
        .gap(px(12.0))
        // taxonomy column
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(px(TEXT_SMALL_PT))
                        .text_color(rgba(BODY_SILVER))
                        .child("Category"),
                )
                .child(cat("Perang".to_string(), true))
                .child(cat("Angkut".to_string(), false))
                .child(cat("Latih".to_string(), false)),
        )
        .child(div().w(px(1.0)).h_full().bg(rgba(HAIRLINE_SLATE)))
        // leaf column
        .child(
            div()
                .flex_1()
                .relative()
                .child(
                    div()
                        .text_size(px(TEXT_SMALL_PT))
                        .text_color(rgba(BODY_SILVER))
                        .child("Pieces"),
                )
                .child(leaf("KRI Ahmad Yani".to_string(), "frigate · 351".to_string(), true, 20.0))
                .child(leaf("KRI Yos Sudarso".to_string(), "frigate · 353".to_string(), false, 66.0))
                .child(leaf("KRI Cakra".to_string(), "SSK · 401".to_string(), false, 112.0))
                .child(leaf("KRI Ardadedali".to_string(), "SSK · 404".to_string(), false, 158.0)),
        )
}

/// A tooltip (U5's tooltip slice), from egui's `on_hover_text`. The frame
/// is egui's `Frame::popup`: `CONSOLE_NIGHT` fill (`window_fill`), 1px
/// `HAIRLINE_SLATE` border (`window_stroke`), the menu corner radius (6),
/// `menu_margin` (6) inner pad, and the `popup_shadow` (offset 6,10, blur 8,
/// black α96) — a dark wash down-right of the box. It sits a 4pt `gap` below
/// its anchor (the widget it explains), capped at `tooltip_width` (500).
///
/// `open` seeds the shown/hidden state (the proto's stand-in for a hover
/// gesture, which cannot be synthesized here — the reference is a real egui
/// `Tooltip::always_open` at a known anchor, `examples/tooltip_probe`).
/// GPUI has no native hover-tooltip layer, so the port anchors the box below
/// the wrapped content and offsets it by the gap; the shadow is a faked
/// offset wash, since GPUI box shadows are not per-element either.
fn tooltip(anchor: impl IntoElement, text: impl Into<SharedString>, open: bool) -> impl IntoElement {
    div()
        .relative()
        .child(anchor)
        .children(if open {
            Some(
                div()
                    .absolute()
                    .top(px(22.0))
                    .left(px(0.0))
                    .max_w(px(500.0))
                    .px(px(6.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(rgba(CONSOLE_NIGHT))
                    .border(px(1.0))
                    .border_color(rgba(HAIRLINE_SLATE))
                    .text_size(px(TEXT_SMALL_PT))
                    .text_color(rgba(BODY_SILVER))
                    .shadow(vec![gpui::BoxShadow {
                        color: gpui::hsla(0.0, 0.0, 0.0, 96.0 / 255.0),
                        offset: point(px(6.0), px(10.0)),
                        blur_radius: px(8.0),
                        spread_radius: px(0.0),
                    }])
                    .child(text.into()),
            )
        } else {
            None
        })
}

/// The U5 tooltip slice: a control with its tooltip shown (seeded open),
/// over the real map — the frame, the 4pt gap below the anchor and the
/// popup shadow, all from `Frame::popup`. Reference:
/// `examples/tooltip_probe` at the app theme.
fn tooltip_layer() -> impl IntoElement {
    div()
        .absolute()
        .left(px(40.0))
        .top(px(40.0))
        .flex()
        .flex_col()
        .gap(px(24.0))
        .child(tooltip(
            button("New session", false, false, false),
            "create it with the full form; the rest of the plan is authored in Planning",
            true,
        ))
        .child(tooltip(
            button("refresh", true, false, false),
            "re-read the session list from Minos",
            false,
        ))
}

/// The drag state machine (U5's drag slice). The gesture is a PRESS on a
/// row, not a mode: press records the cursor, and the row is "moved" only
/// once the pointer travels past a 6px dead-zone (`update_unit_drag`:
/// `distance(start) > 6.0`) — a release inside the zone is the ordinary
/// click, not a drag. While held the cursor is `Grabbing` and a ghost of the
/// row follows the pointer.
///
/// **The plan's "drop zone fades" trap does not apply, and the shipped code
/// says why.** The console's drop target is the MAP, and a genuine move
/// commits on release-over-map (`finish_unit_move`); its comment is explicit:
/// "Dropped, never faded — a fade needs per-frame alpha state, and under
/// repaint-on-demand that means repainting toward a target computed from
/// where a trail is GOING." So there is NO fade to measure; the port shows
/// the drop zone as a hover-state div, which is the honest shape. What IS
/// real and portable: the 6px dead-zone, the grabbing cursor, the ghost, and
/// commit-on-release-over-the-zone.
///
/// This ports the state machine's shape and its live wiring (press/move/
/// release), which the proto drives from real GPUI mouse events — unlike the
/// earlier slices' seeded static states. The seeded capture shows a held,
/// moved drag (ghost + grabbing) with no synthetic input.
///
/// The dead-zone rule, verbatim from `update_unit_drag`: a held press is a
/// drag only past 6px from where it started.
fn drag_past_dead_zone(start: (f32, f32), now: (f32, f32)) -> bool {
    let (dx, dy) = (now.0 - start.0, now.1 - start.1);
    (dx * dx + dy * dy).sqrt() > 6.0
}

/// The two `egui::Window` surfaces (U5's final slice): the console's
/// centred Settings window and the floating presentation card, as GPUI must
/// draw them.
///
/// **The plan's trap, resolved against the reference.** The handoff frames it
/// as "GPUI modal panes are `Layer::modal` and clip to the window; the
/// shipped egui windows are `WindowLevel::AlwaysWarm` with a custom shadow.
/// The parity ask is the shadow width (8px), not whether the pane opens."
/// Measured against a real egui `Window` (`examples/window_probe`): the
/// Settings window is **352×225**, anchored centre (`Align2::CENTER_CENTER`),
/// fixed (not collapsible/resizable), `CONSOLE_NIGHT` fill, a 1px
/// `HAIRLINE_SLATE` border, radius 8, `window_margin` 6 inner pad, and the
/// `window_shadow` (offset 10,20, blur 15, black α96) — a much larger shadow
/// than the tooltip's `popup_shadow`. Crucially it draws **no dimming
/// backdrop**: an `egui::Window` is a floating pane, not a modal, and the
/// map shows around it. So the parity work is the window FRAME + its shadow,
/// not a GPUI `Layer::Modal` backdrop — GPUI's modal is the wrong primitive
/// for a non-modal `Window`, and using it would add a dim the console never
/// had.
///
/// `backdrop` toggles a modal (`Layer::Modal`-style dim, for the New-session
/// modal which IS modal) over the floating-window treatment (no dim), so the
/// capture shows both and the distinction is inspectable.
fn window_layer(backdrop: bool) -> impl IntoElement {
    let mut root = div().relative().size_full();
    // The modal dim, only when the surface is actually modal (New session).
    if backdrop {
        root = root.child(
            div()
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .bg(rgba(Rgba::black_alpha(150))),
        );
    }
    // The centred window: frame + shadow + title + a sliver of body, at the
    // measured geometry. `shadow` is the window_shadow (10,20,15,α96), the
    // one piece GPUI `Layer::Modal` would NOT give — the reason the frame is
    // hand-built rather than deferred to the layer.
    root.child(
        div()
            .absolute()
            .left(px(204.0))
            .top(px(198.0))
            .w(px(352.0))
            .min_h(px(120.0))
            .p(px(6.0))
            .rounded(px(8.0))
            .bg(rgba(CONSOLE_NIGHT))
            .border(px(1.0))
            .border_color(rgba(HAIRLINE_SLATE))
            .shadow(vec![gpui::BoxShadow {
                color: gpui::hsla(0.0, 0.0, 0.0, 96.0 / 255.0),
                offset: point(px(10.0), px(20.0)),
                blur_radius: px(15.0),
                spread_radius: px(0.0),
            }])
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_size(px(TEXT_BODY_PT))
                            .text_color(rgba(MAP_INK))
                            .child("Settings"),
                    )
                    .child(
                        div()
                            .text_size(px(TEXT_BODY_PT))
                            .text_color(rgba(BODY_SILVER))
                            .child("×"),
                    ),
            )
            .child(div().h(px(1.0)).w_full().bg(rgba(HAIRLINE_SLATE)))
            .child(
                div()
                    .text_size(px(TEXT_SMALL_PT))
                    .text_color(rgba(BODY_SILVER))
                    .child("View · Layout · Motion"),
            ),
    )
}

/// The U5 buttons slice, drawn against the map: a stock button and a
/// `small_button` in their three live states (rest / hover / press), plus a
/// disabled row. Static states, seeded like the one-lit-rim capture — the
/// hover/active paints are the theme's, driven by a boolean rather than a
/// synthetic pointer. Reference: the toolbar row and the essentials island
/// in `.impeccable/critique/capture-shipped-console-populated.png`.
fn buttons_layer() -> impl IntoElement {
    let row = |y: f32, label: &'static str| {
        div()
            .absolute()
            .left(px(24.0))
            .top(px(y))
            .flex()
            .flex_row()
            .gap(px(ITEM_SPACING.0))
            .child(button(label, false, false, false))
            .child(button(label, false, false, true))
            .child(button(label, true, false, false))
            .child(button(label, true, false, true))
    };
    div()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .child(row(24.0, "comms"))
        .child(row(72.0, "refresh"))
        .child(
            div()
                .absolute()
                .left(px(24.0))
                .top(px(120.0))
                .flex()
                .flex_row()
                .gap(px(ITEM_SPACING.0))
                .child(button("disabled", false, true, false))
                .child(button("disabled", true, true, false)),
        )
}

/// The U5 layout slice (panel 3), measured off the reference's Essentials
/// island (`.impeccable/critique/capture-shipped-console-populated.png`).
/// What it shows is the island body's own layout vocabulary: a vertical
/// stack whose items are spaced by `ITEM_SPACING.1` (6pt = 7.5px at this
/// scale), a horizontal row (combobox cell + label + button) sharing the
/// width, a full-width `separator()` hairline, an indented label/value pair
/// (`INDENT`), and a weak aside at `TEXT_SMALL_PT`. The reference's
/// measured vertical rhythm is the bar: title → +9px → session/refresh row
/// (38px) → +7px gap → Edit button (23px) → +9px → separator, i.e. items
/// spaced by the 6pt item gap, not a fixed row pitch.
///
/// This mirrors the reference's STRUCTURE (spacing, separator, indent, the
/// row/column composition), not its exact widget sizes — the console's
/// combobox is an egui `ComboBox` (a later choice slice) and its buttons are
/// the button slice's. The layout claims are the gaps and the separator.
fn layout_layer() -> impl IntoElement {
    // A section heading, as the island body draws one.
    let heading = |t: &'static str| {
        div()
            .text_size(px(TEXT_BODY_PT))
            .text_color(rgba(MAP_INK))
            .child(t)
    };
    // A weak aside: the console's `ui.weak`, at the small size, dimmed ink.
    let weak = |t: &'static str| {
        div()
            .text_size(px(TEXT_SMALL_PT))
            .text_color(rgba(BODY_SILVER))
            .child(t)
    };
    // The session/refresh row: a combobox-width cell, a label, a button —
    // the horizontal composition, spaced by the item gap.
    let pick_row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(ITEM_SPACING.0))
        .child(
            div()
                .flex_1()
                .h(px(18.0))
                .px(px(8.0))
                .flex()
                .items_center()
                .rounded(px(CONTROL_RADIUS))
                .border(px(1.0))
                .border_color(rgba(HAIRLINE_SLATE))
                .bg(rgba(BUTTON_GRAPHITE))
                .text_size(px(TEXT_SMALL_PT))
                .text_color(rgba(BUTTON_LABEL))
                .child("SHOOT-01"),
        )
        .child(
            div()
                .text_size(px(TEXT_SMALL_PT))
                .text_color(rgba(BODY_SILVER))
                .child("session"),
        )
        .child(button("refresh", true, false, false));
    // An indented label/value pair: `INDENT` in, one line apart.
    let indented = div()
        .pl(px(INDENT))
        .flex()
        .flex_col()
        .gap(px(2.0))
        .child(weak("purpose"))
        .child(
            div()
                .text_size(px(TEXT_BODY_PT))
                .text_color(rgba(MAP_INK))
                .child("a session for photographing the console"),
        );
    // The body: a vertical stack, one item gap between rows, over the map.
    div()
        .absolute()
        .left(px(24.0))
        .top(px(24.0))
        .w(px(280.0))
        .p(px(12.0))
        .rounded(px(8.0))
        .bg(rgba(Rgba::black_alpha(150)))
        .flex()
        .flex_col()
        .gap(px(ITEM_SPACING.1))
        .child(heading("1 · Session"))
        .child(pick_row)
        .child(button("Edit session (blank leaves alone)", true, false, false))
        .child(separator())
        .child(indented)
        .child(weak("joining needs a seat first — ask the Game Master."))
}

/// Build one halo sprite: the raster at 2x for a 1.25-1.5 ppp display, as a
/// `RenderImage` — the same raw path the map frames take, so the sprite
/// demo costs no encode either. Strength is folded in at raster time, so
/// there is one sprite per (state, strength) and none per frame.
///
/// The raster is PREMULTIPLIED (it is the additive contribution: `rgb =
/// tint * a`), but gpui's images are STRAIGHT alpha — its own decoder only
/// swaps R for B and never premultiplies (`elements/img.rs`), so the
/// compositor multiplies the colour by alpha itself. Handing it the
/// premultiplied raster attenuates twice and paints a dark disc; that is
/// what the compositing probe caught, and un-premultiplying here is the
/// fix.
fn halo_sprite(kind: halo::Halo, strength: f32) -> Arc<gpui::RenderImage> {
    const SUPERSAMPLE: usize = 2;
    let side = (halo::HALO_R * 2.0) as usize * SUPERSAMPLE;
    let premultiplied = halo::rasterize(side, kind.rgb(), strength);
    let mut straight = Vec::with_capacity(premultiplied.len());
    for px in premultiplied.chunks_exact(4) {
        let a = px[3] as f32 / 255.0;
        if a <= 0.0 {
            straight.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            let un = |c: u8| ((c as f32 / a).clamp(0.0, 255.0) + 0.5) as u8;
            straight.extend_from_slice(&[un(px[0]), un(px[1]), un(px[2]), px[3]]);
        }
    }
    gate::swizzle_rgba_to_bgra(&mut straight);
    let buf =
        image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(side as u32, side as u32, straight)
            .expect("sprite size");
    Arc::new(gpui::RenderImage::new([image::Frame::new(buf)]))
}

/// The prototype's STALE (0xF6C56B), which is neither of the kernel's two
/// ambers: it is the fixture's own design value, and the comparison is
/// against that fixture.
const STALE_SAND: Rgba = Rgba::rgb(0xF6, 0xC5, 0x6B);

/// The Inspector dock's height. A dock is a place: the console recomputes it
/// every frame so a drag snaps back, and this is the place.
const INSPECT_H: f32 = 150.0;

/// The roster sheet (U5's first slice): the epaint prototype's panel, ported
/// to GPUI at the geometry measured from its own reference capture
/// (`.impeccable/critique/capture-shipped-chrome-roster.png`) rather than
/// eyeballed. Everything below is either from that measurement or from
/// `proto/p5-epaint`'s own constants:
///
/// | part | value |
/// | --- | --- |
/// | panel rect | (73,65) 338x428, top-right chamfer 20 |
/// | fill / edge | PANEL_SLATE / 1px LINE x1.5, shadow +5+6 |
/// | title | "ROSTER" tracked caps 15 at inner.left, +14 |
/// | counter | "8 / 125" mono 11, right-aligned -16, +16 |
/// | separator | 1px LINE at +46, inner.left..inner.right |
/// | body | rows of 34 from +54 |
/// | row | name mono 12 (+12,+7); hull mono 11 right -12, +7; class 10.5 (+12,+20); stale dot r2.5 at (right-16, bottom-8) |
/// | selected | black_alpha(70) r3 over the row, 2x18 SIGNAL rule at its left |
///
/// What this proves is the VOCABULARY, and the rows are now REAL: they come
/// from the hull register over the dev API (the binary fetches; see
/// `RosterRow`), which is the
/// first panel of U8 step 4 driven by live values rather than the prototype's
/// eight-row fixture. Without credentials it falls back to that fixture, so
/// the shell still renders and the geometry claim still holds.
///
/// Still not the console's own path: the console reads these rows from the
/// local store after a sync, and the class name it shows is resolved through
/// the catalog rather than read off the wire. That is the next increment.
/// One row of the roster sheet, as the hull register carries it.
///
/// A shell-side shape, not the store's: the console resolves these through
/// the local store and the catalog after a sync, and this is the first step
/// of that — the same three fields, read straight off the wire.
/// The session this machine holds, as the top zone shows it.
///
/// A shell-side shape, like `RosterRow`: the console reads the same numbers
/// from its mirrored participant list, and the top zone's headcount is
/// exactly these counts. Empty is a real state — no session held — not a
/// missing one, and the bar says so.
/// One seat in the session's room, as the roster shows it.
#[derive(Clone, Debug)]
pub struct PersonView {
    pub id: i64,
    pub name: String,
    pub call_sign: String,
    /// The role's NAME, for display.
    pub role: String,
    /// The role's ID, for the diff. Carried because `roster::plan` compares
    /// staged seats against the server's by id, and a name is not comparable:
    /// two labels for one role would read as a change. Displaying the name and
    /// diffing the id is the console's own arrangement.
    pub role_id: i64,
    pub judge: bool,
    pub ready: bool,
    /// Presented the room key. The API does not report this today (see the
    /// handoff), so it is present in the shape and false in the data.
    pub present: bool,
}

#[derive(Clone, Debug)]
pub struct SessionView {
    pub game: String,
    pub state: String,
    /// Seats assigned in the room.
    pub seats: usize,
    /// How many have presented the room key (the headcount proper).
    pub present: usize,
    pub ready: usize,
    pub judges: usize,
    /// The caller's own user id, and their role in the held game ("" when
    /// this machine holds no seat). The seat is why `/users/me` is read.
    pub me: i64,
    pub my_role: String,
    /// Every seat in the room, exercise side first (the server's order).
    pub people: Vec<PersonView>,
    /// The caller's own readiness, and whether the held game is executing.
    /// The console's dashboard opens on these: a commander plans first and
    /// declares after, so the helm waits for execution while readiness does
    /// not.
    pub my_ready: bool,
    pub execution: bool,
    /// The hulls this account commands in the held game.
    pub mine: Vec<MyShip>,
    /// The inbox the caller may see, newest first (the server's order).
    pub messages: Vec<MessageView>,
    /// The exercise's step and its assumed time, when the server can stamp
    /// one. `None` when the clock has no anchor — a real state, not a gap.
    pub step: Option<StepView>,
    /// The held game's book: scenarios with their steps nested, in book
    /// order. Steps arrive ONLY here — there is no step-list route — so the
    /// panel reads this and nothing else.
    pub book: Vec<BookScenario>,
    /// The Assessment island's three reads, and they are BACKEND TYPES rather
    /// than flattened copies. `GameFacts` above is flattened because
    /// `GameDetail` nests a window struct this shell reads four fields out of;
    /// these are flat already, and a second struct holding the same fields is
    /// the "one answer served from two places" this repo forbids.
    ///
    /// All three are empty unless the game is in Closure, which is the only
    /// state whose column carries Assessment -- so a planning room pays
    /// nothing for them, and the console's own reload buttons are read here
    /// as "the session was re-read" rather than pressed.
    pub timeline: Vec<crate::backend::TimelineEvent>,
    /// The cursor block of that same read: the seed fetch takes page one
    /// unfiltered, and dropping the cursor would strand "load more"
    /// undrawable even while Minos holds more. They describe `timeline`
    /// above — one read, not two answers — so they live beside it, and a
    /// session installed whole carries its own block with it.
    pub timeline_cursor: Option<String>,
    pub timeline_has_more: bool,
    pub judgements: Vec<crate::backend::Judgement>,
    pub reviews: Vec<crate::backend::Review>,
    /// The pieces of the exercise: hulls assigned to the held game, the
    /// relate editor's checkbox list. Best-effort like the book.
    pub pieces: Vec<PieceView>,
    /// What each step obliges, resolved by the server. Read per step because
    /// steps arrive only nested in the book — a small fan-out, best-effort
    /// per step, so one unreadable step leaves an absent entry rather than
    /// no session.
    pub related: Vec<StepRelatedView>,
    /// The held game's id, which every order names.
    pub game_id: i64,
    /// Whether this shell holds a session at all. `false` shows the sign-in
    /// card instead of the console — the one panel that has to exist before
    /// any other can.
    pub signed_in: bool,
    /// The map thread's opening zoom, so the shell can show and step it
    /// without knowing the map's constants.
    pub zoom: f64,
    /// The game's own terms: the `GET /games/{id}` read, which the shell went
    /// without for its whole life. Essentials, Control and Review all answer
    /// from it, and Review's room key comes from here too — `room_key` is on
    /// the same payload, so the key needs no second read.
    pub facts: GameFacts,
    /// The published readiness checklist. `None` is a real answer and not a
    /// gap: Minos 404s this route until a session enters preparation, so a
    /// planning room has no checklist to show. The console carries the same
    /// distinction as `ReadinessGap::Absent` and lets the advance through on
    /// it — a server that never published the check cannot be waited out.
    pub readiness: Option<ReadinessView>,
    /// The directory the Player picker seats from: accounts this build can
    /// find, in the server's order. Separate from the session read because it
    /// is not a fact about the exercise — the console reads it on the way INTO
    /// the picker (`users_refresh_directory`), for the same reason, and a
    /// picker that rendered an empty directory with no read attempted was a
    /// bug the Minos log had to expose.
    pub directory: Vec<DirectoryRow>,
    /// The game's own roles, as the seat form offers them. Server-sourced and
    /// never a hardcoded list: a deployment that relabels a seat keeps working,
    /// because the labels are read from `/games/{id}/roles`.
    pub roles: Vec<RoleRow>,
}

/// One account the Player picker can seat.
#[derive(Clone, Debug)]
pub struct DirectoryRow {
    pub id: i64,
    pub name: String,
    /// The unit, rank and appointment, when the mirror carries them. A blank
    /// one is shown blank rather than filled in — an invented `satuan` would
    /// be a worse answer than an absent one.
    pub unit: String,
    pub rank: String,
    pub appointment: String,
}

/// One game role, as the seat form offers it.
#[derive(Clone, Debug)]
pub struct RoleRow {
    pub id: i64,
    pub name: String,
    /// Whether the role sits on the judge side. Carried because the seat form
    /// has to say which seats steer and which direct, and because
    /// `roster::role_may_need_command` reads it.
    pub judge: bool,
}

/// The held game's own terms — `GameDetail`, cut to what the column reads.
///
/// Flattened rather than carrying `tfg::backend::GameDetail` itself because
/// the shell's views are plain data the tests build by hand, and `GameDetail`
/// drags `TimeWindow`, `GamePace` and the withheld-`area` rule along with it.
/// The prose fields are `String` and empty means not written, which is
/// `GameDetail`'s own reading of them.
#[derive(Clone, Debug, Default)]
pub struct GameFacts {
    /// `scenario` in the seed. The server's word, shown as read.
    pub mode: String,
    /// The pace as the operator reads it: `standard`, `fast`, or the raw
    /// token when the server names one this build does not know. An
    /// unrecognised pace is NOT a pace — it keeps its ceremony, which is the
    /// safe direction (`GameDetail::pace`'s own reasoning).
    pub pace: String,
    pub description: String,
    pub purpose: String,
    pub target: String,
    /// The planned window, as the server states it. Either end may be absent;
    /// an incomplete window is the ordinary state of a draft and is the
    /// reason the execution transition is refused.
    pub actual_start: String,
    pub actual_end: String,
    pub assumed_start: String,
    pub assumed_end: String,
    /// The game-time multiplier Control edits. Read here, written by
    /// `ToBinary::SetTimeFactor`.
    pub time_factor: f64,
    /// What personnel type to enter the room. Empty until the game enters
    /// preparation — a planning game has no room to enter.
    pub room_key: String,
    /// Whether the `GET /games/{id}` read answered at all. `false` is what a
    /// read this account may not make looks like, and Essentials says so
    /// instead of drawing an empty form.
    pub read: bool,
}

/// The readiness checklist, from `GameReadinessView`.
#[derive(Clone, Debug, Default)]
pub struct ReadinessView {
    /// Whether the server will accept the advance. The checklist is advisory
    /// — this is the verdict.
    pub can_execute: bool,
    pub blockers: Vec<String>,
}

/// One scenario in the held game's book, steps nested in book order.
///
/// There is deliberately no staged half here: the console authors through
/// `book_draft` (staged adds/cuts/relates, synced on advance) and this shell
/// has no advance to sync on, so adds post at once and the panel says so.
/// The shape — title, ordered steps, empty content as a real state — is the
/// console's `book_view`.
#[derive(Clone, Debug, Default)]
pub struct BookScenario {
    pub id: i64,
    pub title: String,
    pub steps: Vec<BookStep>,
}

/// One step: the free text that says what must happen, plus its window.
/// There is NO title — the content is the label, which is the backend's
/// decision, not this panel's invention.
#[derive(Clone, Debug, Default)]
pub struct BookStep {
    pub id: i64,
    pub content: String,
    pub window: Option<(String, String)>,
}

/// One helm order, from the panel to the wire. The shell's only write.
#[derive(Clone, Debug)]
pub struct HelmOrder {
    pub unit_id: String,
    pub heading_deg: f32,
    pub speed_kn: f32,
}

/// What the server said about an order.
#[derive(Clone, Debug)]
pub struct HelmStatus {
    pub unit_id: String,
    pub ok: bool,
    pub text: String,
}

/// The new-session draft: the console's `session_creator_body` fields — name
/// (required) plus every optional the `POST /games` body accepts, so nothing
/// the edit form owns later is missing here. The shell owns the draft; the
/// binary owns the POST, like the sign-in card's credentials.
#[derive(Clone, Debug)]
pub struct SessionDraft {
    pub name: String,
    pub mode: String,
    pub description: String,
    pub purpose: String,
    pub target: String,
    pub area: String,
    pub map_tag: String,
}

impl Default for SessionDraft {
    fn default() -> Self {
        SessionDraft {
            name: String::new(),
            // The console's own default: the exercise integrates helm orders.
            mode: "maneuver".to_string(),
            description: String::new(),
            purpose: String::new(),
            target: String::new(),
            area: String::new(),
            map_tag: String::new(),
        }
    }
}

impl SessionDraft {
    /// The server refuses a nameless session, so the panel does too — the
    /// console's `setup_create_game` checks this before spending a POST.
    pub fn name_ok(&self) -> bool {
        !self.name.trim().is_empty()
    }
}

/// The three modes the server documents, in the console's order, with the
/// console's own one-line meanings.
pub const SESSION_MODES: [(&str, &str); 3] = [
    ("maneuver", "the exercise integrates helm orders"),
    ("scenario", "personnel steer like maneuver; the Game Master plays steps around them"),
    ("static", "positions are set by hand — helm orders are refused"),
];

/// Which compose field has the keystrokes: the judgement score or the
/// review body. File scope because an `impl` holds only functions — the
/// helper below says which slot each variant reads and what Enter sends.
#[derive(Clone, Copy)]
enum WriteField {
    Score,
    Body,
}

/// What the control grant's absence says, on any pane that asks. The
/// console's `ClockDenied` landing sentence verbatim — the pre-check's
/// shorter "ask the Game Master" is what the console says before spending
/// a POST, and the landing is what it says after the 403, so the shell
/// (which learns the grant only from the 403) speaks the landing.
pub const CLOCK_GRANT: &str = "clock control needs the control grant in this session \u{2014} Game Master or an entrusted judge-side role (app admin is not enough)";

/// What the shell says to the binary. ONE channel, because there is one
/// seam: the shell never reads or writes the network or the map thread
/// itself, it asks, and the binary is the only thing that knows how.
#[derive(Clone, Debug)]
pub enum ToBinary {
    /// A helm order for the commanded hull.
    Helm(HelmOrder),
    /// The camera's zoom, after a map-control gesture. The shell does not
    /// transform the picture — it asks the map thread for a new frame, and
    /// the markers follow because the binary projects them at the same
    /// camera.
    Zoom(f64),
    /// A pan, in MAP pixels (the shell converts from window points, because it
    /// is the side that knows both the viewport and the frame's size). The
    /// sign is the binary's: it unprojects the map pixel the drag pointed at.
    Pan { dx: f64, dy: f64 },
    /// Centre the map on a hull. The shell names the GROUND (lat/lon from the
    /// hull's own fix) and the chrome's shift in px — it is the side that
    /// knows the zone's dock and width — and the binary unprojects the
    /// visible centre, because the shell has no mercator.
    Jump { lat: f64, lon: f64, shift_px: f32 },
    /// Deploy a hull where the pointer says: the unit plus the drop point as
    /// fractions of the map view. The shell names fractions, never pixels or
    /// ground — it knows the viewport, and the frame fills it
    /// (`ObjectFit::Fill`), so fractions survive any window size. The binary
    /// unprojects to lat/lon at its own camera and calls `set_placement`,
    /// because the shell has no mercator. The answer is a re-read session on
    /// `create_feed`, like every other write here.
    DeployUnit { unit_id: i64, fx: f64, fy: f64 },
    /// Assign a register hull and deploy it, in one gesture: the drop of a
    /// hull the exercise does not hold yet. The commander is the
    /// operator's own seat (the shell refuses seatless first); the binary
    /// sequences `assign_unit` before `set_placement`, and the answer is
    /// the same re-read session a plain deploy returns.
    DeployAssign { unit_id: i64, commander_id: i64, fx: f64, fy: f64 },
    /// Embark a dragged hull aboard a placed host: the host the drop's
    /// fractions landed nearest, plus the dragged hull. The binary owns the
    /// POST (`game_attach_units`); the answer is the host's contents on the
    /// embark feed, like every other embark write.
    EmbarkDrop { host: i64, child: i64 },
    /// Assign a register hull and embark it, in one gesture: the drop of a
    /// hull the exercise does not hold yet onto a placed host. Same chain
    /// as `DeployAssign`, ending in `game_attach_units`, answered on the
    /// embark feed like every other embark write.
    EmbarkAssign { host: i64, child: i64, commander_id: i64 },
    /// Sign out. The shell clears its own session and shows the card; the
    /// binary drops its token copy so nothing else posts on it. The binary
    /// never held the refresh token, so there is no server revocation here —
    /// the console's `sign_out` revokes and clears the keyring, and this
    /// prototype has neither to clear.
    SignOut,
    /// Credentials, from the sign-in card. The binary owns the login; the
    /// shell only collects what was typed.
    SignIn { identifier: String, password: String },
    /// A new session, from the composer's draft. The binary owns the POST;
    /// the shell only collects what was typed, like the sign-in card.
    CreateSession { draft: SessionDraft },
    /// A new scenario in the HELD game's book, from the book panel's title
    /// row. Posted at once, not staged: the console stages and syncs on
    /// advance, and this shell has no advance — the panel states the
    /// difference rather than faking the staging.
    AddScenario { title: String },
    /// A new step at the end of an open scenario, from the book panel's step
    /// row: content plus the window, both ends or neither. Posted at once,
    /// like scenarios — the console's `stage_step_add` is the staging this
    /// shell does not do.
    AddStep {
        scenario_id: i64,
        content: String,
        window: Option<(String, String)>,
    },
    /// A step out of an open scenario. Deletes AT ONCE, which is harsher
    /// than the console's staged cut (undoable until the flush): the panel's
    /// aside says so, because a delete that reads as reversible when it is
    /// not is the lie to avoid here.
    RemoveStep { scenario_id: i64, step_id: i64 },
    /// The caller's own readiness: declare or withdraw. The binary owns the
    /// PUT/DELETE; the shell only asks. Provable in preparation as a player
    /// — the write probe declared and withdrew on game 12 — unlike the helm,
    /// which waits for execution.
    SetReadiness { ready: bool },
    /// The force, whole: every write `force::plan` owed Minos, in the order
    /// the server accepts them.
    ///
    /// The whole plan rather than one edit per message, because a plan is one
    /// decision. Posting a hull and its commander as two messages would let a
    /// failure between them leave the exercise holding a piece nobody commands
    /// — the exact state `ForceGap` exists to prevent. Outside planning each
    /// edit plans and posts immediately; while planning the draft is written
    /// through whole by `AdvancePreparation` instead.
    SyncForce { writes: Vec<crate::force::ForceWrite> },
    /// The roster, whole: every write `roster::plan` owed Minos.
    ///
    /// Same shape and same reason as `SyncForce`: a plan is one decision, and
    /// posting its writes one message at a time would let a failure between
    /// them leave a half-seated room. `roster::plan` is the console's own
    /// write generation, ported — a re-picked row for somebody the server
    /// already holds converges to a role change or to nothing rather than to a
    /// 409 in the middle of authoring.
    SyncRoster { writes: Vec<crate::roster::RosterWrite> },
    /// LEAVE PLANNING: write the whole staged draft through, then move the
    /// game to preparation.
    ///
    /// This is the one message that carries the drafts rather than writes
    /// derived from them, because the shell cannot compute the writes: it
    /// holds no session in the binary, and the binary cannot reach the
    /// shell's drafts. The drafts and the live mirrors they diff against
    /// travel together so the plan is computed against the SAME read the
    /// shell diffed against, at the moment of the advance.
    ///
    /// The binary plans the force first; a `ForceGap` refuses the move WHOLE
    /// and posts nothing. Otherwise it posts the force, then the roster, then
    /// the task organisation — each whole, the first refusal stopping the
    /// rest — and only then transitions the game, answering with the re-read
    /// session.
    AdvancePreparation {
        force: crate::force::ForceDraft,
        live_hulls: Vec<crate::force::LiveHull>,
        roster: crate::roster::RosterDraft,
        live_seats: Vec<crate::roster::LiveSeat>,
    },
    /// The game clock's multiplier, from Control's Apply. One field, one
    /// endpoint (`PATCH /games/{id}/time-factor`), and the answer is a re-read
    /// like every other write here — the readout on the next frame is Minos's,
    /// never the draft's.
    SetTimeFactor { factor: f64 },
    /// Leave preparation. The console's `setup_advance_execution`; the server
    /// gates it on its own terms and the verdict comes back as a sentence.
    AdvanceExecution,
    /// Leave execution. The console's `close_game`: the `z.clock` island's
    /// `End session →`, which transitions the held game to closure. The
    /// server gates it on its own terms and the verdict comes back as a
    /// sentence; on success the answer echoes the held game, now closed.
    EndSession,
    /// Pause or resume the scenario clock. The boolean is decided
    /// shell-side from the last clock answer (held → resume, else pause;
    /// unknown → pause), because the binary holds the token but never the
    /// clock. The write IS the read: no clock GET exists, and the answer
    /// is the GameClock to display.
    SetPaused { paused: bool },
    /// Read one hull's passengers, for the Embarked modal — the console's
    /// `ensure_embarked`. The binary owns the GET; the shell only asks,
    /// naming the host the modal drew.
    ReadEmbarked { host: i64 },
    /// Take one piece out of its host — the console's `unembark`, as a
    /// message. One press per row, never a batch: the permission follows
    /// the piece, not a guess about which rows the operator meant.
    UnembarkUnit { host: i64, unit: i64 },
    /// Let one embarked piece act — the console's `embark_clear`, as a
    /// message. Clearing is permission, not release: she stays aboard for
    /// messages and readiness.
    ClearAboard { host: i64, unit: i64 },
    /// Take the host's permission back — the console's `embark_revoke`,
    /// as a message. Legs already sailed stay sailed.
    RevokeClearance { host: i64, unit: i64 },
    /// Hold one game from the session picker. The binary owns the re-read
    /// (`fetch_session` pinned, the same read startup holds); the shell
    /// only names the id the row drew.
    HoldSession { game_id: i64 },
    /// A step's whole related list, from the relate editor's working set:
    /// hulls of the exercise (nodes ride the same list when the task
    /// organisation exists — it does not in any dev game). Posted at once,
    /// not staged: the console stages and syncs on advance, and this shell
    /// has no advance. An empty list is legal and clears it.
    SetRelated {
        scenario_id: i64,
        step_id: i64,
        related: Vec<(String, i64)>,
    },
    /// A telegram broadcast from the comms panel's composer row. The binary
    /// owns the POST; the shell only collects what was typed, like every
    /// other write. The envelope is the PROVEN path — kind telegram,
    /// TERBUKA, BIASA, broadcast — because addressed mail, other markings
    /// and other grades are unprobed; the panel states the cut.
    SendTelegram { content: String },
    /// This seat's declaration on the asking step: declare or withdraw. The
    /// binary owns the PUT/DELETE; the shell only asks, naming the step the
    /// row drew — the ids ride the button, never a cached guess.
    DeclareStep {
        scenario_id: i64,
        step_id: i64,
        ready: bool,
    },
    /// A Closure timeline page, from the Assessment island's Timeline tab.
    /// `append` carries the cursor's next page (`load more`); false reloads
    /// from the first page under the current filters. The binary owns the
    /// GET; the shell only asks, naming the filters it drew — like every
    /// other read here. The events ride back on the same answer feed the
    /// writes use, because the page is a read the operator asked for on
    /// demand rather than part of the session hold.
    Timeline {
        append: bool,
        source: Option<String>,
        personnel: Option<i64>,
        unit: Option<i64>,
        from: Option<String>,
        to: Option<String>,
        cursor: Option<String>,
    },
    /// Re-read this closure's marks, from the Judgements tab's `reload` —
    /// the console's `load_judgements`. The binary owns the GET; the shell
    /// only asks, like every other read here.
    ReloadJudgements,
    /// Re-read this closure's documents, from the Reviews tab's `reload` —
    /// the console's `load_reviews`. `author` is the narrow `mine only`
    /// resolves to (this seat, or `None` unfiltered), because the binary
    /// holds the token but never the seating.
    ReloadReviews { author: Option<i64> },
    /// Record one judgement: the picked subject plus the free-text score.
    /// The judge is the caller and is never sent — the server refuses
    /// non-judge callers loudly, and the tab disables the verb without a
    /// judge seat first.
    RecordJudgement { personnel_id: i64, score: String },
    /// File one review: the picked subject plus the body. The author is
    /// the caller and is never sent.
    FileReview { personnel_id: i64, body: String },
    /// Revise the row under edit: words only, subject fixed at filing. The
    /// author alone may — anyone else gets the server's explicit 403, which
    /// the line carries.
    ReviseReview { review_id: i64, body: String },
}

/// What a completed sign-in unlocked: the register and the session.
pub struct SignedIn {
    pub roster: Vec<RosterRow>,
    pub roster_total: usize,
    pub sessions: Vec<SessionRow>,
    pub session: SessionView,
}

/// What a created session unlocked: the new game, read back and held — never
/// assumed from the draft, because the server names the id and the state.
pub struct CreatedSession {
    pub session: SessionView,
}

/// The sign-in answer, drained like every other feed. `Err` carries what to
/// put on the card — a refusal the operator can act on, never a swallowed
/// failure.
pub type SignInFeed =
    std::boxed::Box<dyn FnMut() -> Option<Result<SignedIn, String>> + Send + 'static>;

/// The create answer, drained like every other feed. `Err` carries what to
/// put on the form — a refusal the operator can act on, never a swallowed
/// failure.
pub type CreateFeed =
    std::boxed::Box<dyn FnMut() -> Option<Result<CreatedSession, String>> + Send + 'static>;

/// The shell's sink: the binary owns the other end and every effect.
pub type OrderSink = std::sync::mpsc::Sender<ToBinary>;
/// What came back, drained like the other feeds.
pub type OrderStatusFeed =
    std::boxed::Box<dyn FnMut() -> Option<HelmStatus> + Send + 'static>;

/// The timeline page answer, drained like the other feeds. The bool is the
/// `append` the request carried, echoed back so the apply knows whether to
/// replace the list (a filter reload) or extend it (`load more`) — the same
/// `(page, append)` pair the console's `SetupDone::Timeline` carries.
pub type TimelineFeed = std::boxed::Box<
    dyn FnMut() -> Option<Result<(crate::backend::TimelinePage, bool), String>> + Send + 'static,
>;

/// A judgements reload answer, drained like the other feeds. The held marks
/// are REPLACED — the console's `SetupDone::Judgements` — because the reload
/// re-reads the whole list, not a page.
pub type JudgementsFeed = std::boxed::Box<
    dyn FnMut() -> Option<Result<Vec<crate::backend::Judgement>, String>> + Send + 'static,
>;

/// A reviews reload answer, drained like the other feeds. The held documents
/// are REPLACED — the console's `SetupDone::Reviews` — and a narrowed reload
/// (`mine only`) replaces them with the narrowed list, which is why the
/// toggle flips and reloads at once rather than filtering what is held.
pub type ReviewsFeed = std::boxed::Box<
    dyn FnMut() -> Option<Result<Vec<crate::backend::Review>, String>> + Send + 'static,
>;

/// A record-judgement answer, drained like the other feeds. Carries the
/// filed mark back — the console's `SetupDone::JudgementSent` — so the
/// landing names what was recorded and re-reads through the reload, exactly
/// the way the console clears the score and re-calls `load_judgements`.
pub type JudgeWriteFeed = std::boxed::Box<
    dyn FnMut() -> Option<Result<crate::backend::Judgement, String>> + Send + 'static,
>;

/// A file-or-revise answer, drained like the other feeds. The bool says
/// which verb it answers — false filed, true revised — because the landing
/// clears different drafts for each (body and subject on file, editing and
/// body on save) while both re-read through the reload.
pub type ReviewWriteFeed = std::boxed::Box<
    dyn FnMut() -> Option<Result<(crate::backend::Review, bool), String>> + Send + 'static,
>;

/// A clock write's answer, drained like the other feeds. `Denied` is the
/// server's 403 — the control grant, remembered as a latch rather than
/// re-asked — and `Failed` is everything else, loud on the line.
#[derive(Debug, Clone, PartialEq)]
pub enum ClockError {
    Denied,
    Failed(String),
}

/// The clock answer feed: pause and resume answer the GameClock, because
/// the write IS the read. (Factor keeps its session re-read — its readout
/// is `facts.time_factor`, and the held clock is only ever asked whether
/// the exercise is running.) The shell holds the last answer's `running`
/// and decides the next toggle from it.
pub type ClockFeed = std::boxed::Box<
    dyn FnMut() -> Option<Result<crate::backend::GameClock, ClockError>> + Send + 'static,
>;

/// An embarked-contents answer, drained like the other feeds. Carries the
/// host back with its passengers, because answers can overtake opens — the
/// modal may already show another hull by the time a slow read lands, and
/// caching under the answer's own host cannot file one hull's passengers
/// under another's.
pub type EmbarkedFeed = std::boxed::Box<
    dyn FnMut() -> Option<Result<(i64, Vec<crate::backend::EmbarkedUnit>), String>> + Send + 'static,
>;

/// An embark-write answer, drained like the other feeds. Carries the host
/// back with the write's result — the host's contents after the act plus
/// the capacity warnings, if any — for the same overtaking reason as the
/// read: file under the answer's host, never the open one.
pub type EmbarkWriteFeed = std::boxed::Box<
    dyn FnMut() -> Option<Result<(i64, crate::backend::EmbarkationResult), String>>
        + Send
        + 'static,
>;

/// A caller with no session is the TESTS and the parity proto, and both want
/// the SHELL rather than the sign-in card: only the binary knows whether it
/// failed to authenticate, so it is the only thing that ever clears this.
impl Default for SessionView {
    fn default() -> Self {
        SessionView {
            game: String::new(),
            state: String::new(),
            seats: 0,
            present: 0,
            ready: 0,
            judges: 0,
            me: 0,
            my_role: String::new(),
            mine: Vec::new(),
            game_id: 0,
            signed_in: true,
            zoom: 0.0,
            my_ready: false,
            execution: false,
            messages: Vec::new(),
            step: None,
            people: Vec::new(),
            book: Vec::new(),
            // Empty, and NOT because a session with no game has no debrief:
            // because `fetch_session` does not ask for one outside Closure.
            // The default is the tests' fixture, and a fixture that invented
            // a debrief would be a fixture lying about what Minos said.
            timeline: Vec::new(),
            timeline_cursor: None,
            timeline_has_more: false,
            judgements: Vec::new(),
            reviews: Vec::new(),
            pieces: Vec::new(),
            related: Vec::new(),
            facts: GameFacts::default(),
            readiness: None,
            directory: Vec::new(),
            roles: Vec::new(),
        }
    }
}

/// One hull this account commands, as "My command" lists it.
#[derive(Clone, Debug)]
pub struct MyShip {
    /// The unit id, which is the key the live markers share — so the panel
    /// joins its own hulls to the wire by the same string the roster does.
    pub id: String,
    pub label: String,
}

/// One game on the session picker, as the start island lists it: the
/// server's identity and state, plus this account's seat in it (role name
/// and judge side, empty/false when unseated). A display projection like
/// `MyShip`, not a second `GameRow` — the picker reads these four things
/// and nothing else, in the server's own list order.
#[derive(Clone, Debug)]
pub struct SessionRow {
    pub id: i64,
    pub name: String,
    pub state: String,
    pub my_role: String,
    pub judge: bool,
}

/// One piece of the exercise: a hull assigned to the held game. The server's
/// own word — a related hull has to be one of its pieces — and the relate
/// editor's checkbox list. Assigned units only, never the whole register:
/// relating an unassigned hull answers 400.
#[derive(Clone, Debug, Default)]
pub struct PieceView {
    pub id: i64,
    pub label: String,
    /// Who commands it, when anybody does. `None` is a real state and not a
    /// gap: a hull can be in the exercise uncommanded for as long as the
    /// operator is working, and Minos is the one that refuses it — see
    /// `force::plan`'s `ForceGap`. Carried because the Fleet picker has to
    /// SHOW who holds each piece, and because the draft is diffed against it.
    pub commander_id: Option<i64>,
    /// Whether the hull sits at a starting position. This is the console's
    /// `DraftHull::start.is_some()`, read from the server's own placements
    /// rather than from a draft — the shell has no draft yet (see the Fleet
    /// island), so the fact comes from `GET /games/{id}/placements`, which is
    /// the same answer Minos would give the console's staged copy.
    pub placed: bool,
}

/// One step's related list, as the server resolved it: the posts it names
/// plus the personnel those oblige. The same shape answers the read and the
/// write, so the editor redraws from what the server resolved rather than
/// from what it sent.
#[derive(Clone, Debug, Default)]
pub struct StepRelatedView {
    pub scenario_id: i64,
    pub step_id: i64,
    pub posts: Vec<(String, i64)>,
    pub personnel: Vec<String>,
}

/// One message in the session's inbox, as the comms panel shows it.
#[derive(Clone, Debug)]
pub struct MessageView {
    pub sender: String,
    pub kind: String,
    pub class_label: String,
    pub content: String,
    pub at: String,
    pub broadcast: bool,
}

/// The exercise's step, as the step panel shows it: what is being played, and
/// what is being asked of THIS caller.
#[derive(Clone, Debug, Default)]
pub struct StepView {
    /// The SERVER's assumed time. Never a client integral — the backend's own
    /// doc refuses it: the anchor plus every segment is three chances to
    /// disagree about what time it is in the exercise.
    pub assumed_time: String,
    /// The step being played, if one is.
    pub playing: Option<StepLine>,
    /// The step asking this caller to declare ready, if one is. `None` means
    /// no step asks them — never an empty step to render.
    pub asking: Option<StepAsk>,
}

/// The one step the reminder strip names: what this caller owes the
/// exercise, never the book around it.
#[derive(Clone, Debug, PartialEq)]
pub struct StepReminder {
    /// True while the asked step still waits on this caller.
    pub needs_you: bool,
    /// The strip's whole line.
    pub text: String,
}

/// The step a game is playing.
#[derive(Clone, Debug)]
pub struct StepLine {
    pub scenario_id: i64,
    pub step_id: i64,
    pub scenario: String,
    pub content: String,
    pub state: String,
}

/// The step waiting on readiness, and how far along it is. The ids ride
/// along because the declare/withdraw writes name them — the display never
/// shows them, but the SEND button sends exactly what the row drew.
#[derive(Clone, Debug)]
pub struct StepAsk {
    pub scenario_id: i64,
    pub step_id: i64,
    pub scenario: String,
    pub content: String,
    pub required: i64,
    pub outstanding: i64,
    pub you_ready: bool,
}

/// Which credential field has the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cred {
    Identifier,
    Password,
}

#[derive(Clone, Debug)]
pub struct RosterRow {
    /// The unit id the live wire keys fixes by. The register and the feed
    /// share it as a decimal string, so staleness joins without a name
    /// lookup — the same key the egui console's track store uses.
    pub unit_id: String,
    pub name: String,
    pub hull: String,
    pub class: String,
    /// Seed from the register; overwritten each tick by the wire's
    /// liveness (see `run`'s `fix_feed`) once a feed is wired.
    pub stale: bool,
}

/// The prototype's eight-row fixture, kept as the no-credentials fallback.
///
/// It is the reference the U5 roster slice was measured against, so keeping
/// it means a capture without credentials is still comparable to that
/// measurement — the geometry claim survives the data going real.
pub fn fixture_roster() -> Vec<RosterRow> {
    [
        ("1", "KRI Ahmad Yani", "351", "frigate · ASW", false),
        ("2", "KRI Yos Sudarso", "353", "frigate · ASW", false),
        ("3", "KRI Cakra", "401", "SSK · 209/1300", false),
        ("4", "KRI Ardadedali", "404", "SSK · DSME 209/1400", false),
        ("5", "KRI Alugoro", "405", "SSK · DSME 209/1400", true),
        ("6", "KRI Fatahillah", "—", "corvette", false),
        ("7", "KRI Sungai Gerong", "—", "patrol · 40m", true),
        ("8", "KRI Dewaruci", "—", "auxiliary", false),
    ]
    .into_iter()
    .map(|(unit_id, name, hull, class, stale)| RosterRow {
        unit_id: unit_id.to_string(),
        name: name.to_string(),
        hull: hull.to_string(),
        class: class.to_string(),
        stale,
    })
    .collect()
}

/// One decoded hull photograph from the background fetch: the unit plus
/// straight-alpha RGBA bytes at thumbnail size. Decoded off-thread (the
/// shell only ever sees bytes); the `RenderImage` is minted on first
/// paint, the same raw path the map frames take.
pub struct ThumbRaw {
    pub unit_id: i64,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// The photo trickle, drained like every other feed. `None` most ticks;
/// one hull's bytes when the fetcher lands one — photographs pop in
/// progressively, never all-or-nothing.
pub type ThumbFeed =
    std::boxed::Box<dyn FnMut() -> Option<Vec<ThumbRaw>> + Send + 'static>;

/// Mint a paintable thumbnail from decoded bytes. `None` on a byte-count
/// mismatch — a corrupt fetch paints nothing, never a torn image.
pub fn thumb_image(raw: ThumbRaw) -> Option<Arc<gpui::RenderImage>> {
    let mut bytes = raw.rgba;
    gate::swizzle_rgba_to_bgra(&mut bytes);
    let buf = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(raw.w, raw.h, bytes)?;
    Some(Arc::new(gpui::RenderImage::new([image::Frame::new(buf)])))
}

/// One hull at the bottom of the drill, with the glyph the map would draw.
///
/// The symbol is resolved ONCE by the binary (same resolver the markers
/// use) and carried here, because the shell draws this and must not reach
/// through the store. The photograph, when the fetcher has landed one,
/// wins — the glyph is the fallback, never the portrait.
#[derive(Clone, Debug)]
pub struct FleetTaxLeaf {
    pub id: String,
    pub name: String,
    pub hull: String,
    pub class_name: String,
    pub symbol: MapSymbolShape,
}

/// What a fleet-picker leaf row carries when dragged: the hull's id, its
/// name for the ghost, and its glyph. `Clone` because the drop side reads
/// the same value the source armed — one value, two readers, no re-lookup.
#[derive(Clone, Debug)]
pub struct UnitDragPayload {
    pub unit_id: i64,
    pub name: String,
    pub symbol: MapSymbolShape,
    /// The hull's photograph, when the fetcher has landed one — the ghost
    /// shows the same face the row does. An `Arc`, so the drag shares the
    /// shell's copy rather than the bytes.
    pub thumb: Option<Arc<gpui::RenderImage>>,
}

impl UnitDragPayload {
    /// The payload a leaf row arms: the id Minos files placements and
    /// embarkations under, with the row's own display values along for the
    /// ghost. Pure, so the harness pins it without a pointer.
    fn for_leaf(leaf: &FleetTaxLeaf, thumb: Option<Arc<gpui::RenderImage>>) -> Self {
        UnitDragPayload {
            unit_id: leaf.id.parse::<i64>().unwrap_or(-1),
            name: leaf.name.clone(),
            symbol: leaf.symbol,
            thumb,
        }
    }
}

/// The ghost that follows the pointer while a hull is dragged: its
/// thumbnail and its name, the console's drag cell in another toolkit.
struct DragGhost {
    name: String,
    symbol: MapSymbolShape,
    thumb: Option<Arc<gpui::RenderImage>>,
}

impl Render for DragGhost {
    fn render(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let symbol = self.symbol;
        // The photograph when there is one, the glyph otherwise — the
        // same face the row shows, so the hand never swaps pictures.
        let face = IslandDemo::thumb_face(self.thumb.clone(), symbol);
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(4.0))
            .bg(rgba(PANEL_SLATE))
            .border(px(1.0))
            .border_color(rgba(RADAR_CYAN))
            .child(face)
            .child(
                div()
                    .font_family(view::FACE_HUMAN)
                    .text_size(px(11.0))
                    .text_color(rgba(MAP_INK))
                    .child(SharedString::from(self.name.clone())),
            )
    }
}

/// One hull-class rung of the drill: id, display name, live-hull count, and
/// the hulls in it. The nesting IS the hierarchy — Asset Group > Category >
/// Type > Class > hull — the same walk the egui picker's `drill_*` state
/// did, read off the same `store::tax_*` readers.
#[derive(Clone, Debug, Default)]
pub struct FleetTaxClass {
    pub id: i64,
    pub name: String,
    pub count: i64,
    pub units: Vec<FleetTaxLeaf>,
}

/// One type rung of the drill: id, display name, live-hull count, and the
/// classes under it (see `FleetTaxClass` for the rung contract).
#[derive(Clone, Debug, Default)]
pub struct FleetTaxType {
    pub id: i64,
    pub name: String,
    pub count: i64,
    pub classes: Vec<FleetTaxClass>,
}

/// One category rung of the drill: id, display name, live-hull count, and
/// the types under it (see `FleetTaxClass` for the rung contract).
#[derive(Clone, Debug, Default)]
pub struct FleetTaxCat {
    pub id: i64,
    pub name: String,
    pub count: i64,
    pub types: Vec<FleetTaxType>,
}

/// One asset-group tab of the drill: id, display name, live-hull count, and
/// the categories under it (see `FleetTaxClass` for the rung contract).
#[derive(Clone, Debug, Default)]
pub struct FleetTaxGroup {
    pub id: i64,
    pub name: String,
    pub count: i64,
    pub cats: Vec<FleetTaxCat>,
}

/// The whole drill tree, built once at startup. Empty when the store has no
/// taxonomy (fixture runs, offline machines) — the picker falls back to its
/// flat register list rather than showing an empty drill.
///
/// Built by the binary (`build_fleet_taxonomy` in `main.rs`), not here: the
/// tree reads off the sqlite mirror, and this module is also compiled into
/// the sqlite-free parity proto — which is exactly what keeps the drill
/// honest about its seams.
#[derive(Clone, Debug, Default)]
pub struct FleetTaxonomy {
    pub groups: Vec<FleetTaxGroup>,
}

#[cfg(test)]
mod fleet_taxonomy_tests {
    use super::*;

    pub fn fixture_taxonomy() -> FleetTaxonomy {
        FleetTaxonomy {
            groups: vec![
                FleetTaxGroup {
                    id: 2,
                    name: "Sea".to_string(),
                    count: 2,
                    cats: vec![FleetTaxCat {
                        id: 2,
                        name: "Frigate".to_string(),
                        count: 2,
                        types: vec![FleetTaxType {
                            id: 3,
                            name: "FFG".to_string(),
                            count: 2,
                            classes: vec![FleetTaxClass {
                                id: 5,
                                name: "Sigma".to_string(),
                                count: 2,
                                units: vec![
                                    FleetTaxLeaf {
                                        id: "13".to_string(),
                                        name: "KRI Ahmad Yani".to_string(),
                                        hull: "351".to_string(),
                                        class_name: "Sigma".to_string(),
                                        symbol: MapSymbolShape::Frigate,
                                    },
                                    FleetTaxLeaf {
                                        id: "14".to_string(),
                                        name: "KRI Yos Sudarso".to_string(),
                                        hull: "353".to_string(),
                                        class_name: "Sigma".to_string(),
                                        symbol: MapSymbolShape::Frigate,
                                    },
                                ],
                            }],
                        }],
                    }],
                },
                FleetTaxGroup {
                    id: 4,
                    name: "Other".to_string(),
                    count: 0,
                    cats: Vec::new(),
                },
            ],
        }
    }

    fn drill_demo() -> IslandDemo {
        let mut demo = IslandDemo::new(
            fixture_roster(),
            8,
            0,
            SessionView::default(),
            std::sync::mpsc::channel().0,
        );
        demo.fleet_tax = fixture_taxonomy();
        demo
    }

    #[test]
    fn drill_defaults_to_first_tab() {
        let demo = drill_demo();
        assert_eq!(demo.drill_group_id(), Some(2), "unseeded open shows Sea");
        assert_eq!(demo.drill_cats().len(), 1);
        assert!(demo.drill_types().is_empty(), "no category picked yet");
        assert!(demo.drill_units().is_empty(), "no class picked yet");
    }

    #[test]
    fn drill_walks_to_the_hulls() {
        let mut demo = drill_demo();
        demo.select_category(2);
        assert_eq!(demo.drill_types().len(), 1);
        demo.select_type(3);
        assert_eq!(demo.drill_classes().len(), 1);
        demo.select_class(5);
        assert_eq!(demo.drill_units().len(), 2, "both Sigma hulls list");
    }

    #[test]
    fn drill_parent_change_clears_below() {
        let mut demo = drill_demo();
        demo.select_category(2);
        demo.select_type(3);
        demo.select_class(5);
        demo.select_category(2);
        assert!(demo.drill_units().is_empty(), "stale class must not survive");
        demo.select_type(3);
        demo.select_class(5);
        demo.select_group(4);
        assert!(demo.drill_cats().is_empty(), "Other holds no categories");
        assert!(demo.drill_units().is_empty(), "stale hulls must not survive");
        demo.select_group(2);
        assert!(demo.drill_units().is_empty(), "returning clears too");
    }

    #[test]
    fn payload_carries_the_hulls_identity() {
        let mut demo = drill_demo();
        demo.select_category(2);
        demo.select_type(3);
        demo.select_class(5);
        let leaf = demo.drill_units().first().cloned().expect("a hull to drag");
        let payload = UnitDragPayload::for_leaf(&leaf, None);
        assert_eq!(payload.unit_id, 13, "Minos files under the numeric id");
        assert_eq!(payload.name, "KRI Ahmad Yani");
        assert_eq!(payload.symbol, MapSymbolShape::Frigate);
        assert!(payload.thumb.is_none(), "no photo fetched in-harness");
    }

    #[test]
    fn thumb_image_mints_from_bytes() {
        // 2×2 straight red: the fetcher's contract in miniature.
        let raw = ThumbRaw {
            unit_id: 13,
            w: 2,
            h: 2,
            rgba: vec![
                255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
            ],
        };
        assert!(thumb_image(raw).is_some(), "well-formed bytes paint");
    }

    #[test]
    fn thumb_image_refuses_torn_bytes() {
        let raw = ThumbRaw {
            unit_id: 13,
            w: 2,
            h: 2,
            rgba: vec![255, 0, 0],
        };
        assert!(thumb_image(raw).is_none(), "short bytes paint nothing");
    }

    fn host_demo() -> IslandDemo {
        let mut session = SessionView::default();
        session.pieces = vec![PieceView {
            id: 8,
            label: "host".to_string(),
            commander_id: None,
            placed: true,
        }];
        let mut demo = IslandDemo::new(
            fixture_roster(),
            8,
            0,
            session,
            std::sync::mpsc::channel().0,
        );
        demo.map_dims = (1000.0, 1000.0);
        demo.live = vec![LiveMarker {
            id: "8".to_string(),
            x: 500.0,
            y: 500.0,
            stale: false,
            symbol: MapSymbolShape::Frigate,
            affiliation: symbology::Affiliation::Unknown,
            heading_deg: None,
            speed_kn: None,
            latitude: 0.0,
            longitude: 0.0,
            trail: 0,
        }];
        demo
    }

    #[test]
    fn drop_names_the_nearest_placed_host() {
        let demo = host_demo();
        assert_eq!(demo.drop_host_at(0.5, 0.5), Some(8), "dead on the marker");
        assert_eq!(
            demo.drop_host_at(0.51, 0.5),
            Some(8),
            "10px off still boards"
        );
        assert_eq!(demo.drop_host_at(0.1, 0.1), None, "far water is a deploy");
    }

    #[test]
    fn drop_ignores_unplaced_and_mapless() {
        let mut demo = host_demo();
        demo.session.pieces[0].placed = false;
        assert_eq!(
            demo.drop_host_at(0.5, 0.5),
            None,
            "an unplaced piece hosts nothing"
        );
        let mut demo = host_demo();
        demo.map_dims = (0.0, 0.0);
        assert_eq!(demo.drop_host_at(0.5, 0.5), None, "no frame, no host");
    }
}

fn roster_sheet(
    rows: &[RosterRow],
    total: usize,
    selected: usize,
    cx: &mut Context<IslandDemo>,
) -> impl IntoElement {
    const PANEL_X: f32 = 73.0;
    const PANEL_Y: f32 = 65.0;
    const PANEL_W: f32 = 338.0;
    const PANEL_H: f32 = 428.0;
    const INNER_L: f32 = 15.0;
    // Panel-local: measured 88..395 in window space, minus the panel origin
    // (73). Getting this wrong is visible immediately as a counter and hull
    // column flush against the right edge.
    const INNER_R: f32 = 322.0;
    const ROW_H: f32 = 34.0;
    /// Rows the body holds: (428 - 54) / 34, floor.
    const MAX_ROWS: usize = 11;
    const BODY_TOP: f32 = 54.0;

    let mut panel = div()
        .absolute()
        .left(px(PANEL_X))
        .top(px(PANEL_Y))
        .w(px(PANEL_W))
        .h(px(PANEL_H));

    // The chamfered silhouette, its shadow and the lit cut: one path pass,
    // the same technique the modal uses.
    panel = panel.child(canvas(
        |bounds, _, _| bounds,
        |bounds, _, window, _| {
            let (x0, y0) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let (x1, y1) = (x0 + PANEL_W, y0 + PANEL_H);
            let poly = |dx: f32, dy: f32| {
                let mut p = PathBuilder::fill();
                p.move_to(point(px(x0 + dx), px(y0 + dy)));
                p.line_to(point(px(x1 - CUT + dx), px(y0 + dy)));
                p.line_to(point(px(x1 + dx), px(y0 + CUT + dy)));
                p.line_to(point(px(x1 + dx), px(y1 + dy)));
                p.line_to(point(px(x0 + dx), px(y1 + dy)));
                p.close();
                p.build()
            };
            if let Ok(shadow) = poly(5.0, 6.0) {
                window.paint_path(shadow, rgba(ISLAND_CAST));
            }
            if let Ok(body) = poly(0.0, 0.0) {
                window.paint_path(body, rgba(PANEL_SLATE));
            }
            // The cut edge reads as a deliberate cut, not a clipping bug.
            let mut cut = PathBuilder::stroke(px(2.0));
            cut.move_to(point(px(x1 - CUT), px(y0)));
            cut.line_to(point(px(x1), px(y0 + CUT)));
            if let Ok(cut) = cut.build() {
                window.paint_path(cut, rgba(RADAR_CYAN.with_alpha(140)));
            }
            // The panel's own 1px edge, on the silhouette.
            let mut edge = PathBuilder::stroke(px(1.0));
            edge.move_to(point(px(x0), px(y0)));
            edge.line_to(point(px(x1 - CUT), px(y0)));
            edge.line_to(point(px(x1), px(y0 + CUT)));
            edge.line_to(point(px(x1), px(y1)));
            edge.line_to(point(px(x0), px(y1)));
            edge.close();
            if let Ok(edge) = edge.build() {
                window.paint_path(edge, rgba(HAIRLINE_SLATE.with_alpha(191)));
            }
            // Separator under the header, and the 1px panel edge.
            let mut sep = PathBuilder::stroke(px(1.0));
            sep.move_to(point(px(x0 + INNER_L), px(y0 + 46.0)));
            sep.line_to(point(px(x0 + INNER_R), px(y0 + 46.0)));
            if let Ok(sep) = sep.build() {
                window.paint_path(sep, rgba(HAIRLINE_SLATE));
            }
        },
    ));

    // How many rows the body holds: a fixed number at a fixed row height. The
    // sheet has no scroll, so the counter is what says how many exist beyond
    // them.
    let shown = rows.len().min(MAX_ROWS);

    // Header: the title and the counter.
    //
    // The title is tracked caps, and GPUI 0.2.2 has NO letter spacing —
    // `TextStyle` has no field for it and `FontFeatures` cannot express
    // tracking (egui put it in the text format, which is what `view`'s
    // `TITLE_TRACKING` doc says). This is that gap, worked around the only
    // way available: one div per character in a flex row whose gap IS the
    // tracking. It is exact, and it is a cost every tracked label in U5
    // will pay until GPUI grows the property.
    panel = panel.child(
        div()
            .absolute()
            .left(px(INNER_L))
            .top(px(14.0))
            .flex()
            .flex_row()
            .gap(px(2.4))
            .font_family(view::FACE_HUMAN)
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_size(px(15.0))
            .text_color(rgba(RADAR_CYAN))
            .children("ROSTER".chars().map(|c| div().child(c.to_string()))),
    );
    panel = panel.child(
        div()
            .absolute()
            .left(px(INNER_R - 120.0))
            .top(px(16.0))
            .w(px(120.0))
            .font_family(view::FACE_MACHINE)
            .text_size(px(11.0))
            .text_color(rgba(BODY_SILVER))
            .flex()
            .justify_end()
            .child(format!("{shown} / {total}")),
    );

    // Rows.
    for (i, row) in rows.iter().take(MAX_ROWS).enumerate() {
        let top = BODY_TOP + ROW_H * i as f32;
        let mut el = div()
            .absolute()
            .left(px(INNER_L))
            .top(px(top))
            .w(px(INNER_R - INNER_L))
            .h(px(ROW_H))
            // The whole row is the hit target: click selects, the cursor
            // says so. The console's selection also drives the camera and
            // the detail pane; here it moves the wash and the edge, which
            // is the state the sheet owns (U8 step 4, selection slice).
            .id(("roster-row", i))
            .cursor_pointer()
            .on_click(cx.listener(move |view, _, _, cx| {
                if view.selected != i {
                    view.selected = i;
                    cx.notify();
                }
            }));
        if i == selected {
            el = el.child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .w(px(INNER_R - INNER_L))
                    .h(px(ROW_H))
                    .rounded(px(3.0))
                    .bg(rgba(view::Rgba::black_alpha(70))),
            );
            // Weight on the 2px edge, not on a colour change (DESIGN.md:283).
            el = el.child(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(8.0))
                    .w(px(2.0))
                    .h(px(18.0))
                    .rounded(px(1.0))
                    .bg(rgba(RADAR_CYAN)),
            );
        }
        let ink = if row.stale { BODY_SILVER } else { MAP_INK };
        el = el
            .child(
                div()
                    .absolute()
                    .left(px(12.0))
                    .top(px(7.0))
                    .font_family(view::FACE_MACHINE)
                    .text_size(px(12.0))
                    .text_color(rgba(ink))
                    .child(row.name.clone()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(INNER_R - INNER_L - 120.0))
                    .top(px(7.0))
                    .w(px(108.0))
                    .flex()
                    .justify_end()
                    .font_family(view::FACE_MACHINE)
                    .text_size(px(11.0))
                    .text_color(rgba(BODY_SILVER))
                    .child(row.hull.clone()),
            )
            .child(
                div()
                    .absolute()
                    .left(px(12.0))
                    .top(px(20.0))
                    .font_family(view::FACE_HUMAN)
                    .text_size(px(10.5))
                    .text_color(rgba(BODY_SILVER.with_alpha(204)))
                    .child(row.class.clone()),
            );
        if row.stale {
            // The staleness pip, positioned in the ROW's own layout frame: 16
            // in from the right edge, 8 up from the bottom. A `div`, not a
            // canvas — the earlier canvas added `top` to a `bounds.origin`
            // that already carried the row's offset, which dropped the pip
            // `top` pixels too low (two rows down) the moment staleness went
            // live off the wire (U8 step 4).
            el = el.child(
                div()
                    .absolute()
                    .right(px(16.0))
                    .bottom(px(8.0))
                    .w(px(5.0))
                    .h(px(5.0))
                    .rounded(px(2.5))
                    .bg(rgba(STALE_SAND)),
            );
        }
        panel = panel.child(el);
    }
    panel
}

/// The map frame source. Newest-wins: each call drains whatever arrived and
/// returns the last frame as raw BGRA, or `None` on a quiet tick. The binary
/// derives this from the map thread; the parity proto derives it from a
/// directory of PNGs, which is the reason that crate builds without MapLibre.
/// One map frame, plus the camera it was rendered from.
///
/// The camera is not optional metadata. The scale bar reads it, and a bar
/// derived from anything else — the zoom the shell last asked for, a constant —
/// describes a frame that is not on screen. The map thread has always put it
/// in `MapResp`; it was read as `_at` and dropped, which is why the console
/// had no scale bar at all.
pub type FrameFeed =
    std::boxed::Box<dyn FnMut() -> Option<(u64, u32, u32, Vec<u8>, (f64, f64), f64)> + Send>;

/// One live ship, as the marker layer draws it. `x`/`y` are in the MAP
/// frame's pixel space: the binary projects the fix with
/// `map_render::project_mercator`, and the shell scales to the window by the
/// frame's dimensions — so the shell never names the map's size.
#[derive(Clone, Debug)]
pub struct LiveMarker {
    /// The unit id the wire keys fixes by, shared with the register.
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub stale: bool,
    /// The taxonomy glyph to draw. A `view` type, not `store::MapSymbol`,
    /// because the shell draws this and must not reach through the store:
    /// the binary resolves the symbol and hands it over already mapped.
    pub symbol: MapSymbolShape,
    /// The fix's course and speed, when it carries them. `None` is a real
    /// answer — a fix with no course draws neutral rather than assuming
    /// north — and "My command" says so rather than inventing a heading.
    pub heading_deg: Option<f32>,
    pub speed_kn: Option<f32>,
    /// The fix itself, carried so the Inspector can state a position rather
    /// than a pixel: the map's pixels are the renderer's, the fix is the
    /// hull's.
    pub latitude: f64,
    pub longitude: f64,
    /// How many fixes the trail holds. A LENGTH, not the trail: the shell has
    /// no use for the points and every reason not to copy them per tick.
    pub trail: usize,
    /// The resolved affiliation: which frame the marker draws. A `symbology`
    /// type — the domain enum, which depends on nothing — so the shell names
    /// it without the store.
    pub affiliation: symbology::Affiliation,
}

/// The live picture: one `LiveMarker` per tracked hull. The binary derives
/// these from the poll thread's registry and projects their fix positions;
/// the proto's feed is a `|| None` closure, because there is no wire behind
/// a PNG replay.
pub type FixFeed = std::boxed::Box<dyn FnMut() -> Option<Vec<LiveMarker>> + Send + 'static>;

/// The parity proto's entry: a directory of PNGs replayed as map frames, no
/// wire, no register beyond the fixture. Drives the same `run` the shipped
/// binary does, so the two cannot drift.
pub fn run_replay(dir: String, burst_ms: u64, looping: bool) {
    let replay = RawReplay::start(&dir, burst_ms, looping);
    // The replay has no camera metadata: `RawReplay` decodes PNGs off a
    // directory and never recorded where they were rendered from. So it
    // reports the absent camera as zoom 0, which `ZOOM_MIN` (3.0) guarantees
    // is not a zoom this console ever renders at — making "no camera" and
    // "zoom zero" impossible to confuse. The consequence is honest: a replay
    // draws no scale bar, because a bar with no ground resolution behind it
    // would be a number nobody measured.
    let feed: FrameFeed = std::boxed::Box::new(move || {
        replay
            .latest()
            .map(|(no, w, h, rgba)| (no, w, h, rgba, (0.0, 0.0), 0.0))
    });
    let fix_feed: FixFeed = std::boxed::Box::new(|| None);
    run(
        feed,
        fix_feed,
        std::boxed::Box::new(|| None),
        std::boxed::Box::new(|| None),
        std::boxed::Box::new(|| None),
        std::boxed::Box::new(|| None),
        std::boxed::Box::new(|| None),
        std::boxed::Box::new(|| None),
        std::boxed::Box::new(|| None),
        std::boxed::Box::new(|| None),
        std::boxed::Box::new(|| None),
        std::boxed::Box::new(|| None),
        std::boxed::Box::new(|| None),
        std::boxed::Box::new(|| None),
        std::sync::mpsc::channel().0,
        fixture_roster(),
        125,
        SessionView::default(),
        FleetTaxonomy::default(),
    );
}

/// The shipped entry. The caller builds the feeds: that is what keeps this
/// file free of `map_render` and `shell_threads` — the binary owns the map
/// thread, the poll thread and the live wire, and hands the shell only the
/// read closures and the register it fetched.
pub fn run(
    mut feed: FrameFeed,
    mut fix_feed: FixFeed,
    mut status_feed: OrderStatusFeed,
    mut signin_feed: SignInFeed,
    mut create_feed: CreateFeed,
    mut timeline_feed: TimelineFeed,
    mut judgements_feed: JudgementsFeed,
    mut reviews_feed: ReviewsFeed,
    mut judge_write_feed: JudgeWriteFeed,
    mut review_write_feed: ReviewWriteFeed,
    mut clock_feed: ClockFeed,
    mut embarked_feed: EmbarkedFeed,
    mut embark_write_feed: EmbarkWriteFeed,
    mut thumb_feed: ThumbFeed,
    order_tx: OrderSink,
    roster: Vec<RosterRow>,
    roster_total: usize,
    session: SessionView,
    fleet_tax: FleetTaxonomy,
) {
    Application::new().run(move |cx: &mut App| {
        // The instrument type scale's faces: Saira Condensed in four weights
        // plus PT Mono. GPUI resolves weight from the font data, so one
        // family name covers the whole Saira range and `FontWeight` picks
        // inside it.
        //
        // Failures are fatal and registration is verified by capture, not by
        // this expect: an unregistered face silently becomes the system font
        // and every later capture lies about the typeface.
        //
        // Coverage was measured before these were chosen, because the last
        // face change in this project shipped 44 arrow labels as `.notdef`
        // boxes (`docs/ui-overhaul-status.md`). Of the 21 non-ASCII
        // codepoints the console actually uses, Saira Condensed covers 16 and
        // PT Mono covers 17, and between them they cover all but two that
        // are in use: U+2715 (three sites, already painted as a canvas cross
        // rather than set in type) and U+26A0 (one site). Saira Condensed
        // carries U+2192, which Barlow does not — that alone decided the
        // pairing. `✓` and `↳` appear nowhere in `src/`.
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(
                    include_bytes!("../assets/fonts/SairaCondensed-Regular.ttf").as_slice(),
                ),
                Cow::Borrowed(
                    include_bytes!("../assets/fonts/SairaCondensed-Medium.ttf").as_slice(),
                ),
                Cow::Borrowed(
                    include_bytes!("../assets/fonts/SairaCondensed-SemiBold.ttf").as_slice(),
                ),
                Cow::Borrowed(
                    include_bytes!("../assets/fonts/SairaCondensed-Bold.ttf").as_slice(),
                ),
                Cow::Borrowed(include_bytes!("../assets/fonts/PTMono-Regular.ttf").as_slice()),
            ])
            .expect("bundled fonts register");

        let bounds = Bounds::centered(None, size(px(760.0), px(620.0)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("tfg command center (gpui)".into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            app_id: Some("tfg-gpui".to_string()),
            ..Default::default()
        };
        // `TFG_SEL` seeds the selection for a headless capture, the same
        // way `TFG_POINTER` seeds the pointer and `TFG_FRAME` seeds the
        // map: a way to pin the state without a live gesture.
        let selected = std::env::var("TFG_SEL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let entity = cx.new(|_| {
            let mut demo = IslandDemo::new(roster, roster_total, selected, session, order_tx);
            demo.fleet_tax = fleet_tax;
            demo
        });
        let drain = entity.clone();
        let fix_drain = entity.clone();
        let status_drain = entity.clone();
        let signin_drain = entity.clone();
        let create_drain = entity.clone();
        let timeline_drain = entity.clone();
        let judgements_drain = entity.clone();
        let reviews_drain = entity.clone();
        let judge_write_drain = entity.clone();
        let review_write_drain = entity.clone();
        let clock_drain = entity.clone();
        let embarked_drain = entity.clone();
        let embark_write_drain = entity.clone();
        let thumb_drain = entity.clone();
        let drag_drain = entity.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(20))
                    .await;
                // The drag flag, polled not pushed: the drop and the ghost
                // live in GPUI's own drag state, which no element callback
                // can read into the view — so the tick asks once per frame
                // and the picker hides for exactly the drag's lifetime.
                let _ = cx.update(|app| {
                    let dragging = app.has_active_drag();
                    drag_drain.update(app, |root: &mut IslandDemo, cx| {
                        root.set_dragging(dragging, cx);
                    })
                });
                // Hull photographs, minted as they land: the fetcher decodes
                // off-thread and the shell only ever sees bytes, so each
                // arrival mints one `RenderImage` and notifies once — the
                // rows fill in progressively while the drill stays put.
                if let Some(batch) = thumb_feed() {
                    let _ = cx.update(|app| {
                        thumb_drain.update(app, |root: &mut IslandDemo, cx| {
                            for raw in batch {
                                let id = raw.unit_id;
                                if root.thumbs.contains_key(&id) {
                                    continue;
                                }
                                if let Some(image) = thumb_image(raw) {
                                    root.thumbs.insert(id, image);
                                }
                            }
                            cx.notify();
                        })
                    });
                }
                if let Some((no, w, h, bgra, at, zoom)) = feed() {
                    let _ = cx.update(|app| {
                        drain.update(app, |root: &mut IslandDemo, cx| {
                            root.incoming = Some((no, w, h, bgra, at, zoom));
                            cx.notify();
                        })
                    });
                }
                // The wire's liveness, joined to the roster by unit id.
                // A tick with no fix round leaves the rows as they were —
                // the registry is quiet, not wrong.
                if let Some(liveness) = fix_feed() {
                    let _ = cx.update(|app| {
                        fix_drain.update(app, |root: &mut IslandDemo, cx| {
                            root.apply_liveness(liveness);
                            cx.notify();
                        })
                    });
                }
                if let Some(status) = status_feed() {
                    let _ = cx.update(|app| {
                        status_drain.update(app, |root: &mut IslandDemo, cx| {
                            root.helm_status = Some(status);
                            cx.notify();
                        })
                    });
                }
                // The sign-in answer: success replaces the whole session —
                // register, room and step arrive together, because they are
                // one authenticated read — and failure goes back on the card.
                if let Some(outcome) = signin_feed() {
                    let _ = cx.update(|app| {
                        signin_drain.update(app, |root: &mut IslandDemo, cx| {
                            root.login_busy = false;
                            match outcome {
                                Ok(signed) => {
                                    root.roster = signed.roster;
                                    root.roster_total = signed.roster_total;
                                    // The games list arrives with the roster,
                                    // and no game is held: holding is the
                                    // picker's verb from here, not the
                                    // sign-in's. The column lands on the
                                    // NoSession picker, not mid-execution.
                                    root.sessions = signed.sessions;
                                    root.hold_busy = false;
                                    root.hold_status.clear();
                                    root.session = signed.session;
                                    root.selected = 0;
                                    root.login_status.clear();
                                    // A new hold, unknown grant and unknown
                                    // clock — see `apply_created`.
                                    root.clock_running = None;
                                    root.clock_denied = false;
                                    root.pause_busy = false;
                                    root.pause_status.clear();
                                    // The password does not stay in memory
                                    // after it has been spent.
                                    root.login_password.clear();
                                }
                                Err(why) => root.login_status = why,
                            }
                            cx.notify();
                        })
                    });
                }
                // A created session is HELD, the console's `apply_created_game`:
                // the new game arrives read back rather than assumed, the form
                // clears, and the panel closes. Failure goes back on the form.
                // The busy flags say which panel asked: the composer holds a
                // new game and closes; the book, its steps and its relate
                // editor, and the readiness row hold the same game and stay
                // open — each answer is consumed once, so the flags cannot
                // cross even with two writes in flight. Step and relate
                // writes share the book's apply: they return the same re-read
                // session.
                if let Some(outcome) = create_feed() {
                    let _ = cx.update(|app| {
                        create_drain.update(app, |root: &mut IslandDemo, cx| {
                            match outcome {
                                Ok(created) => {
                                    if root.book_busy || root.step_busy {
                                        root.apply_book(created.session);
                                    } else if root.relate_busy {
                                        root.apply_relate(created.session);
                                    } else if root.readiness_busy {
                                        root.apply_readiness(created.session);
                                    } else if root.step_declare_busy {
                                        root.apply_step_declare(created.session);
                                    } else if root.msg_busy {
                                        root.apply_message(created.session);
                                    } else if root.roster_busy {
                                        root.roster_busy = false;
                                        root.roster_status.clear();
                                        root.reseed_drafts();
                                        root.session = created.session;
                                    } else if root.force_busy {
                                        root.force_busy = false;
                                        root.force_status.clear();
                                        root.reseed_drafts();
                                        root.session = created.session;
                                    } else if root.deploy_busy {
                                        root.deploy_busy = false;
                                        root.force_status.clear();
                                        root.reseed_drafts();
                                        root.session = created.session;
                                    } else if root.prepare_busy {
                                        // The advance wrote the staged draft
                                        // through and moved the game on. The
                                        // re-read session REPLACES the held
                                        // one first: `reseed_drafts` reads it,
                                        // and the drafts it seeds are now the
                                        // record the server just took, so the
                                        // staging latches clear with them.
                                        root.prepare_busy = false;
                                        root.prepare_status.clear();
                                        root.session = created.session;
                                        root.reseed_drafts();
                                    } else if root.factor_busy {
                                        root.apply_factor(created.session);
                                    } else if root.advance_busy {
                                        root.advance_busy = false;
                                        root.advance_status.clear();
                                        root.session = created.session;
                                    } else if root.end_busy {
                                        root.apply_end(created.session);
                                    } else if root.hold_busy {
                                        root.apply_hold(created.session);
                                    } else {
                                        root.apply_created(created.session);
                                    }
                                }
                                Err(why) => {
                                    if root.book_busy || root.step_busy {
                                        root.book_busy = false;
                                        root.step_busy = false;
                                        root.book_status = why;
                                    } else if root.relate_busy {
                                        root.relate_busy = false;
                                        root.book_status = why;
                                    } else if root.readiness_busy {
                                        root.readiness_busy = false;
                                        root.readiness_status = why;
                                    } else if root.step_declare_busy {
                                        root.step_declare_busy = false;
                                        root.step_declare_status = why;
                                    } else if root.msg_busy {
                                        root.msg_busy = false;
                                        root.msg_status = why;
                                    } else if root.roster_busy {
                                        root.roster_busy = false;
                                        root.roster_status = why;
                                    } else if root.force_busy {
                                        root.force_busy = false;
                                        root.force_status = why;
                                    } else if root.deploy_busy {
                                        root.deploy_busy = false;
                                        root.force_status = why;
                                    } else if root.prepare_busy {
                                        root.prepare_busy = false;
                                        root.prepare_status = why;
                                    } else if root.factor_busy {
                                        root.factor_busy = false;
                                        root.factor_status = why;
                                    } else if root.advance_busy {
                                        root.advance_busy = false;
                                        root.advance_status = why;
                                    } else if root.end_busy {
                                        root.end_busy = false;
                                        root.end_status = why;
                                    } else if root.hold_busy {
                                        root.hold_busy = false;
                                        root.hold_status = why;
                                    } else {
                                        root.compose_busy = false;
                                        root.compose_status = why;
                                    }
                                }
                            }
                            cx.notify();
                        })
                    });
                }
                // A Closure timeline page, asked for from the Assessment
                // island's Timeline tab. The append flag rides the answer
                // back, so the apply knows whether this page replaces the
                // list or extends it — the same `(page, append)` pair the
                // console's `SetupDone::Timeline` carries.
                if let Some(outcome) = timeline_feed() {
                    let _ = cx.update(|app| {
                        timeline_drain.update(app, |root: &mut IslandDemo, cx| {
                            root.apply_timeline(outcome);
                            cx.notify();
                        })
                    });
                }
                // A Judgements reload, asked for from the tab's verb. The held
                // marks are REPLACED — the reload re-reads the whole list,
                // not a page — so there is no cursor to carry.
                if let Some(outcome) = judgements_feed() {
                    let _ = cx.update(|app| {
                        judgements_drain.update(app, |root: &mut IslandDemo, cx| {
                            root.apply_judgements(outcome);
                            cx.notify();
                        })
                    });
                }
                // A Reviews reload, asked for from the tab's verb — narrowed
                // to this authorship while `mine only` is checked. The held
                // documents are REPLACED with whichever list was asked for,
                // narrowed or whole.
                if let Some(outcome) = reviews_feed() {
                    let _ = cx.update(|app| {
                        reviews_drain.update(app, |root: &mut IslandDemo, cx| {
                            root.apply_reviews(outcome);
                            cx.notify();
                        })
                    });
                }
                // A record-judgement answer. The landing names the filed
                // mark and re-reads through the reload — the console's
                // `JudgementSent` rule, which is why the write and the read
                // hold separate latches.
                if let Some(outcome) = judge_write_feed() {
                    let _ = cx.update(|app| {
                        judge_write_drain.update(app, |root: &mut IslandDemo, cx| {
                            root.judge_write_busy = false;
                            match outcome {
                                Ok(view) => {
                                    root.judge_score.clear();
                                    root.judgements_status = format!(
                                        "recorded #{} · reloading judgements",
                                        view.id
                                    );
                                    root.send_judgements(cx);
                                }
                                Err(why) => root.judgements_status = why,
                            }
                            cx.notify();
                        })
                    });
                }
                // A file-or-revise answer. Filing clears the body and the
                // subject, saving clears the edit and the body — the
                // console's two landings — and both re-read through the
                // reload under the current narrow.
                if let Some(outcome) = review_write_feed() {
                    let _ = cx.update(|app| {
                        review_write_drain.update(app, |root: &mut IslandDemo, cx| {
                            root.review_write_busy = false;
                            match outcome {
                                Ok((view, revised)) => {
                                    if revised {
                                        root.review_editing = None;
                                        root.review_body.clear();
                                        root.reviews_status = format!(
                                            "revised #{} · reloading reviews",
                                            view.id
                                        );
                                    } else {
                                        root.review_body.clear();
                                        root.review_subject = None;
                                        root.reviews_status = format!(
                                            "filed #{} · reloading reviews",
                                            view.id
                                        );
                                    }
                                    root.send_reviews(cx);
                                }
                                Err(why) => root.reviews_status = why,
                            }
                            cx.notify();
                        })
                    });
                }
                // A clock write's answer — pause or resume. The write IS
                // the read, so the landing holds the answered clock and
                // decides the next toggle from it.
                if let Some(outcome) = clock_feed() {
                    let _ = cx.update(|app| {
                        clock_drain.update(app, |root: &mut IslandDemo, cx| {
                            root.apply_clock_answer(outcome);
                            cx.notify();
                        })
                    });
                }
                // One host's passengers, asked for from the Embarked modal.
                // Filed under the answer's own host, never the open one.
                if let Some(outcome) = embarked_feed() {
                    let _ = cx.update(|app| {
                        embarked_drain.update(app, |root: &mut IslandDemo, cx| {
                            root.apply_embarked(outcome);
                            cx.notify();
                        })
                    });
                }
                // An embark write's answer — unembark, clear or revoke.
                // The host's contents after the act, filed the same way.
                if let Some(outcome) = embark_write_feed() {
                    let _ = cx.update(|app| {
                        embark_write_drain.update(app, |root: &mut IslandDemo, cx| {
                            root.apply_embark_write(outcome);
                            cx.notify();
                        })
                    });
                }
            }
        })
        .detach();
        cx.open_window(opts, |_window, _cx| entity.clone()).unwrap();
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Clicking a roster row selects it — the interaction the compositor's
    /// live click drives, verified here deterministically. A synthetic uinput
    /// click is not reliable in this session (the device does not register
    /// with libinput), so the click is simulated through GPUI's own event
    /// path instead. The state it sets is the same one the capture proves
    /// renders: the wash and the SIGNAL edge follow `selected`.
    #[gpui::test]
    fn clicking_a_roster_row_selects_it(cx: &mut gpui::TestAppContext) {
        // The sheet path is env-gated in `render`. Seed it for this test;
        // no other lib test touches the environment, so there is no
        // concurrent access to race the set.
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo =
                IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), std::sync::mpsc::channel().0);
            demo.sheet = true;
            demo
        });

        // Panel at window (73, 65); row `i` at panel-local (15, 54 + 34i),
        // size (307, 34). Row 3's centre lands at window (241, 238).
        cx.simulate_click(
            gpui::point(gpui::px(241.0), gpui::px(238.0)),
            gpui::Modifiers::default(),
        );

        let selected = cx.update(|_window, app| view.read(app).selected);
        assert_eq!(selected, 3, "clicking row 3 should select row 3");
    }

    /// A wheel over the column MOVES the column.
    ///
    /// This test exists because it did not, and the reason was invisible: the
    /// island's rim and its band texture were `.occlude()`d, and a
    /// `BlockMouse` hitbox BREAKS GPUI's scroll hit test. Every hitbox painted
    /// before it — including the band's — is dropped from the set that
    /// `should_handle_scroll` reads, and GPUI wraps `Interactivity::on_scroll_
    /// wheel` in exactly that check. So the whole column wheel chain
    /// (`chained_wheel`, the ceiling, the subtraction) was dead code no
    /// gesture could reach.
    ///
    /// The kernel tests all passed while this was broken, which is the lesson:
    /// they test `chained_wheel`, and `chained_wheel` was never called.
    #[gpui::test]
    fn a_wheel_over_the_column_moves_the_column(cx: &mut gpui::TestAppContext) {
        let (view, cx) = planning_demo(cx);
        // The harness window is 1920x1080, where this column FITS and the
        // ceiling is 0 — a viewport with nothing to scroll would pass a broken
        // column. The user's own window, where it does not fit.
        cx.simulate_resize(gpui::size(gpui::px(768.0), gpui::px(838.0)));
        let (band, ceil) = cx.update(|window, app| {
            let d = view.read(app);
            let band = zone_band_at(viewport(window), d.zone_w, d.side_dock_left);
            let h = view::zone_stack_height(&d.column_slots());
            (band, view::zone_overflow(h, band))
        });
        assert!(ceil > 0.0, "the fixture must overflow, or this proves nothing");
        let at = gpui::point(gpui::px(band.0 + 40.0), gpui::px(band.1 + 90.0));
        // The pointer must be there BEFORE the wheel: `should_handle_scroll`
        // reads the hit test from the window's mouse position, never from the
        // scroll event's own `position`, so a wheel alone is gated off.
        cx.simulate_mouse_move(at, None, gpui::Modifiers::default());
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: at,
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-40.0))),
            modifiers: gpui::Modifiers::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        let wheel = |cx: &mut gpui::VisualTestContext, dy: f32| {
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: at,
                delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(dy))),
                modifiers: gpui::Modifiers::default(),
                touch_phase: gpui::TouchPhase::Moved,
            })
        };
        // The DIRECTION and the limits, not the per-notch increment. The
        // harness dispatches one `simulate_event` through every listener the
        // rendered frame accumulated (a `simulate_mouse_move` that notifies
        // paints again, and both paints' listeners survive into the dispatch),
        // so the exact step is a harness detail. "Which way" and "how far at
        // most" are the claims the column actually makes.
        wheel(cx, -40.0);
        let sc = cx.update(|_window, app| view.read(app).zone_scroll);
        assert!(sc > 0.0, "a downward wheel moves the column down ({sc})");
        // And the CEILING holds: the column cannot be scrolled off the bottom
        // of its own stack, so enough notches stop at the overflow.
        assert!(ceil > 100.0, "the planning column overflows by a lot");
        for _ in 0..40 {
            wheel(cx, -80.0);
        }
        let sc = cx.update(|_window, app| view.read(app).zone_scroll);
        assert_eq!(sc, ceil, "and it stops at the overflow, not past it");
        wheel(cx, 4000.0);
        let sc = cx.update(|_window, app| view.read(app).zone_scroll);
        assert_eq!(sc, 0.0, "a wheel the other way returns it to the top");
    }

    /// A wheel over a body that still has room leaves the COLUMN alone.
    ///
    /// This pins the chain's DECISION, which is the kernel's rule and the half
    /// that can be asserted here. The offset is set rather than wheeled: in the
    /// console's own planning column the Fleet island sits 796pt down, below a
    /// 758pt band, so nothing reaches its body without scrolling to it first —
    /// and scrolling with the pointer parked on one spot moves the island out
    /// from under that pointer, so the chain would correctly hand the later
    /// wheels to whatever body arrived there instead.
    ///
    /// What is NOT asserted is that the Fleet body itself moved. GPUI's
    /// built-in scroller is gated on the same `should_handle_scroll` set this
    /// test reads, and in this harness it does not fire for a body inside a
    /// clipped column — so a body-scroll assertion here would be a claim
    /// nothing has measured. The console scrolls its bodies through egui; the
    /// shell's are GPUI's, and that is still unverified by gesture.
    #[gpui::test]
    fn a_wheel_over_a_body_with_room_leaves_the_column_alone(cx: &mut gpui::TestAppContext) {
        let (view, cx) = planning_demo_with_pieces(cx);
        cx.simulate_resize(gpui::size(gpui::px(768.0), gpui::px(838.0)));
        let (band, fleet_top) = cx.update(|window, app| {
            let d = view.read(app);
            let band = zone_band_at(viewport(window), d.zone_w, d.side_dock_left);
            let slots = d.column_slots();
            let i = slots
                .iter()
                .position(|s| s.key == view::IslandKey::Fleet)
                .expect("fleet is in the planning column");
            (band, view::zone_slot_origin(band, &slots, i).1)
        });
        // Put the Fleet island's top at the band's top. `render` clamps the
        // offset every frame, so this survives the next draw.
        let placed = fleet_top - band.1;
        cx.update(|_window, app| {
            view.update(app, |demo, _| demo.zone_scroll = placed)
        });
        let at = gpui::point(gpui::px(band.0 + 40.0), gpui::px(band.1 + 120.0));
        // Twice: the hit test the wheel is gated on is recomputed from
        // `mouse_position` when the frame draws, and the draw the offset above
        // forced redraws at the OLD position. One move and the wheel is gated
        // off by a pointer that was never there.
        cx.simulate_mouse_move(at, None, gpui::Modifiers::default());
        cx.simulate_mouse_move(at, None, gpui::Modifiers::default());
        // The body must actually have room, or this proves nothing: forty
        // pieces measure 965pt of content in the island's 290pt viewport.
        let fit = cx.update(|_window, app| {
            view.read(app)
                .islands
                .iter()
                .find(|st| st.key == view::IslandKey::Fleet)
                .and_then(|st| st.last_fit)
        });
        let fit = fit.expect("the Fleet body measured itself");
        assert!(
            fit.can_scroll_down(),
            "the Fleet body has room below the fold"
        );
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: at,
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-40.0))),
            modifiers: gpui::Modifiers::default(),
            touch_phase: gpui::TouchPhase::Moved,
        });
        let after = cx.update(|_window, app| view.read(app).zone_scroll);
        assert_eq!(
            after, placed,
            "a body with room took the gesture, so the column did not"
        );
    }

    /// The Fleet picker's staging, and the one rule that governs it.
    ///
    /// These pin the console's own rule through the console's own module
    /// (`tfg::force`), which is the point of the slice: the draft and its
    /// write plan are PORTED, not reimplemented, so a change to Minos's
    /// contract lands in `force.rs` and every shell test here moves with it.
    #[gpui::test]
    fn a_picked_hull_with_no_commander_is_refused_and_named(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        // A room with one seat, and a hull on the register to pick. The state
        // is PREPARATION, not planning: this test pins the immediate-post path,
        // where a picked hull plans and posts at once. While planning the same
        // pick stages locally and posts nothing — see the staging tests below.
        let session = SessionView {
            state: "preparation".to_string(),
            game: "Game 15".to_string(),
            game_id: 45,
            me: 2,
            people: vec![PersonView {
                id: 2,
                name: "Super User".to_string(),
                call_sign: "SUPERUSER".to_string(),
                role: "Commando".to_string(),
                role_id: 19,
                judge: false,
                ready: false,
                present: true,
            }],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 2, 0, session, tx)
        });
        // Picking a hull hands it to the operator, who holds the one seat.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.stage_assign(1, "KRI Ahmad Yani", 2).expect("a seated pick is assignable");
            })
        });
        let sent = rx.try_recv().expect("the plan reached the sink");
        let writes = match sent {
            ToBinary::SyncForce { writes } => writes,
            other => panic!("expected a force sync, got {other:?}"),
        };
        assert_eq!(
            writes.len(),
            1,
            "one hull with no counterpart on the server is one Assign"
        );
        assert!(
            matches!(
                writes[0],
                crate::force::ForceWrite::Assign { unit_id: 1, commander_id: 2, .. }
            ),
            "and it carries the commander: {:?}",
            writes[0]
        );
    }

    /// The kernel's rule, read through the shell: a hull in the exercise with
    /// nobody to command it stops the WHOLE plan, and the sentence names it.
    ///
    /// `force::plan` is the single answer to this question and the shell adds
    /// no second copy of it — which is why this goes through `sync_force`
    /// rather than through a staging call that checks for itself.
    #[gpui::test]
    fn an_uncommanded_piece_stops_the_whole_plan_and_is_named(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            state: "planning".to_string(),
            game: "Game 15".to_string(),
            game_id: 45,
            me: 2,
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 2, 0, session, tx)
        });
        let why = cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.force.upsert(crate::force::DraftHull {
                    unit_id: 1,
                    name: "KRI Ahmad Yani".to_string(),
                    hull_number: String::new(),
                    class_id: String::new(),
                    start: None,
                    commander_id: None,
                    host: None,
                });
                demo.sync_force()
                    .expect_err("an uncommanded piece is not writable")
            })
        });
        assert!(
            why.contains("KRI Ahmad Yani"),
            "the refusal names the hull rather than saying invalid: {why}"
        );
        assert!(
            rx.try_recv().is_err(),
            "and nothing was sent — a refused plan must not half-commit"
        );
    }

    /// Removing a drafted hull is one write, and a hull the server never had
    /// is not a write at all.
    #[gpui::test]
    fn a_removed_piece_is_one_remove(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        // PREPARATION, so the removal posts at once: the staging path is
        // proved separately.
        let session = SessionView {
            state: "preparation".to_string(),
            game: "Game 15".to_string(),
            game_id: 45,
            me: 2,
            people: vec![PersonView {
                id: 2,
                name: "Super User".to_string(),
                call_sign: "SUPERUSER".to_string(),
                role: "Commando".to_string(),
                role_id: 19,
                judge: false,
                ready: false,
                present: true,
            }],
            pieces: vec![PieceView {
                id: 8,
                label: "KRI Abdul Halim Perdanakusuma (KRI-355)".to_string(),
                commander_id: Some(2),
                placed: true,
            }],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 2, 0, session, tx)
        });
        cx.update(|_window, app| view.update(app, |demo, _| demo.reseed_drafts()));
        // The draft holds the server's piece, so taking it out is a Remove —
        // and nothing else, because the commander and the position are
        // unchanged and `plan` writes neither.
        cx.update(|_window, app| {
            view.update(app, |demo, _| demo.stage_remove(8).expect("a drafted piece is removable"))
        });
        let sent = rx.try_recv().expect("the plan reached the sink");
        let writes = match sent {
            ToBinary::SyncForce { writes } => writes,
            other => panic!("expected a force sync, got {other:?}"),
        };
        assert_eq!(writes.len(), 1, "one Remove, nothing else");
        assert!(
            matches!(writes[0], crate::force::ForceWrite::Remove { unit_id: 8, .. }),
            "got {:?}",
            writes[0]
        );
        // A hull the draft never held is not a write.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.stage_remove(999)
                    .expect_err("a hull not in the draft is not a write");
            })
        });
    }

    /// The Player picker's staging, through the console's own `roster::plan`.
    #[gpui::test]
    fn seating_a_directory_account_is_one_seat(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        // PREPARATION, so the seat posts at once — the planning path stages.
        let session = SessionView {
            state: "preparation".to_string(),
            game: "Game 15".to_string(),
            game_id: 45,
            me: 2,
            people: Vec::new(),
            roles: vec![
                RoleRow { id: 19, name: "Commando".to_string(), judge: false },
                RoleRow { id: 20, name: "Penyelenggara".to_string(), judge: true },
            ],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 2, 0, session, tx)
        });
        let role = cx.update(|_window, app| view.read(app).session.roles[0].clone());
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.stage_seat(6, "Budi Santoso", &role)
                    .expect("an unseated account is seatable");
            })
        });
        let sent = rx.try_recv().expect("the plan reached the sink");
        let writes = match sent {
            ToBinary::SyncRoster { writes } => writes,
            other => panic!("expected a roster sync, got {other:?}"),
        };
        assert_eq!(writes.len(), 1, "one account the server lacks is one Seat");
        assert!(
            matches!(
                &writes[0],
                crate::roster::RosterWrite::Seat { user_id: 6, role_id: 19, call_sign, .. }
                    if !call_sign.is_empty()
            ),
            "and it carries a call sign, because Minos requires one: {:?}",
            writes[0]
        );
    }

    /// A staged seat for somebody the server already holds CONVERGES: the same
    /// role is nothing at all, a different role is a role change. This is
    /// `roster::plan`'s rule and the reason a re-picked row is not a 409.
    #[gpui::test]
    fn restaging_a_seated_account_as_the_same_role_writes_nothing(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        // PREPARATION: the convergence this test pins is `roster::plan`'s, and
        // it must actually run for the claim to mean anything.
        let session = SessionView {
            state: "preparation".to_string(),
            game: "Game 15".to_string(),
            game_id: 45,
            me: 2,
            people: vec![PersonView {
                id: 6,
                name: "Budi Santoso".to_string(),
                call_sign: "BUDI".to_string(),
                role: "Commando".to_string(),
                role_id: 19,
                judge: false,
                ready: false,
                present: false,
            }],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 2, 0, session, tx)
        });
        cx.update(|_window, app| view.update(app, |demo, _| demo.reseed_drafts()));
        let role = RoleRow { id: 19, name: "Commando".to_string(), judge: false };
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.stage_seat(6, "Budi Santoso", &role)
                    .expect("a no-op seat is not an error");
            })
        });
        assert!(
            rx.try_recv().is_err(),
            "the same role converges to nothing — no write, no 409"
        );
    }

    /// An unseat is one write, and a person the server never held is not one.
    #[gpui::test]
    fn an_unseat_is_one_write(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        // PREPARATION, so the unseat posts at once.
        let session = SessionView {
            state: "preparation".to_string(),
            game: "Game 15".to_string(),
            game_id: 45,
            me: 2,
            people: vec![PersonView {
                id: 6,
                name: "Budi Santoso".to_string(),
                call_sign: "BUDI".to_string(),
                role: "Commando".to_string(),
                role_id: 19,
                judge: false,
                ready: false,
                present: false,
            }],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 2, 0, session, tx)
        });
        cx.update(|_window, app| view.update(app, |demo, _| demo.reseed_drafts()));
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.stage_unseat(6).expect("a seated person is unseatable");
            })
        });
        let sent = rx.try_recv().expect("the plan reached the sink");
        let writes = match sent {
            ToBinary::SyncRoster { writes } => writes,
            other => panic!("expected a roster sync, got {other:?}"),
        };
        assert_eq!(writes.len(), 1, "one Unseat, nothing else");
        assert!(
            matches!(&writes[0], crate::roster::RosterWrite::Unseat { user_id: 6, .. }),
            "got {:?}",
            writes[0]
        );
    }

    /// While planning, an edit STAGES: it joins the draft, returns `Ok` and
    /// posts NOTHING. The Session island's advance is the one moment the draft
    /// is written through, which is the whole of this slice's rule.
    #[gpui::test]
    fn a_planning_pick_stages_locally_and_posts_nothing(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            state: "planning".to_string(),
            game: "Game 15".to_string(),
            game_id: 45,
            me: 2,
            people: vec![PersonView {
                id: 2,
                name: "Super User".to_string(),
                call_sign: "SUPERUSER".to_string(),
                role: "Commando".to_string(),
                role_id: 19,
                judge: false,
                ready: false,
                present: true,
            }],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 2, 0, session, tx)
        });
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.stage_assign(1, "KRI Ahmad Yani", 2)
                    .expect("a seated pick is assignable");
                assert!(demo.force.contains(1), "the hull joined the draft");
                assert_eq!(demo.force.get(1).and_then(|h| h.commander_id), Some(2));
            })
        });
        assert!(
            rx.try_recv().is_err(),
            "planning stages: nothing is posted until the advance"
        );
    }

    /// The advance crosses the wire carrying the STAGED draft and the live
    /// mirrors it was diffed against — the shell cannot compute the writes
    /// (the token is the binary's) and the binary cannot see the drafts.
    #[gpui::test]
    fn the_advance_carries_the_staged_draft_and_the_live_mirrors(
        cx: &mut gpui::TestAppContext,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            state: "planning".to_string(),
            game: "Game 15".to_string(),
            game_id: 45,
            me: 2,
            people: vec![PersonView {
                id: 2,
                name: "Super User".to_string(),
                call_sign: "SUPERUSER".to_string(),
                role: "Commando".to_string(),
                role_id: 19,
                judge: false,
                ready: false,
                present: true,
            }],
            pieces: vec![PieceView {
                id: 8,
                label: "KRI Already Here (KRI-1)".to_string(),
                commander_id: Some(2),
                placed: true,
            }],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 2, 0, session, tx)
        });
        let sent = cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.stage_assign(1, "KRI Ahmad Yani", 2).expect("assignable");
                demo.prepare_advance(cx)
            })
        });
        assert!(sent, "an advance from a held game sends");
        match rx.try_recv().expect("the advance reached the sink") {
            ToBinary::AdvancePreparation {
                force,
                live_hulls,
                roster: _,
                live_seats,
            } => {
                assert!(force.contains(1), "the staged hull crossed");
                assert!(
                    force.contains(8),
                    "install seeded the server's piece, so it is not read as a removal"
                );
                assert!(
                    live_hulls.iter().any(|h| h.unit_id == 8),
                    "the live mirror names the piece the draft was diffed against"
                );
                assert!(live_seats.iter().any(|s| s.user_id == 2));
            }
            other => panic!("expected an advance, got {other:?}"),
        }
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.prepare_advance(cx))),
            "an advance in flight sends nothing more"
        );
    }

    /// A drop on bare water while planning unprojects the drop point LOCALLY
    /// and stages it: the draft carries a ground `Start`, the hull is assigned
    /// on the way in, and nothing is posted.
    #[gpui::test]
    fn a_planning_drop_stages_a_start_and_posts_nothing(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            state: "planning".to_string(),
            game: "Game 15".to_string(),
            game_id: 45,
            me: 2,
            people: vec![PersonView {
                id: 2,
                name: "Super User".to_string(),
                call_sign: "SUPERUSER".to_string(),
                role: "Commando".to_string(),
                role_id: 19,
                judge: false,
                ready: false,
                present: true,
            }],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(fixture_roster(), 2, 0, session, tx);
            demo.map_dims = (1000.0, 1000.0);
            demo.map_center = (0.0, 0.0);
            demo.map_render_zoom = 4.0;
            demo
        });
        cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.drop_unit(1, "KRI Ahmad Yani", 0.5, 0.5, None, cx)
            })
        });
        assert!(rx.try_recv().is_err(), "a planning drop posts nothing");
        cx.update(|_window, app| {
            let hull = view.read(app).force.get(1).cloned().expect("the hull joined");
            let start = hull.start.expect("the drop staged a ground start");
            assert_eq!(hull.commander_id, Some(2), "assigned to the operator's seat");
            assert_eq!(hull.host, None, "a bare-water drop clears any host");
            assert!(
                start.lat.abs() < 1e-9 && start.lon.abs() < 1e-9,
                "the frame's centre unprojects to the map centre: {start:?}"
            );
        });
    }

    /// A drop on a host while planning stages an embarkation: the child's
    /// `host` is set and its own `start` cleared — a hull aboard another has
    /// no position of its own — and nothing posts.
    #[gpui::test]
    fn a_planning_embark_drop_stages_a_host(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            state: "planning".to_string(),
            game: "Game 15".to_string(),
            game_id: 45,
            me: 2,
            people: vec![PersonView {
                id: 2,
                name: "Super User".to_string(),
                call_sign: "SUPERUSER".to_string(),
                role: "Commando".to_string(),
                role_id: 19,
                judge: false,
                ready: false,
                present: true,
            }],
            pieces: vec![PieceView {
                id: 8,
                label: "KRI Host (KRI-1)".to_string(),
                commander_id: Some(2),
                placed: true,
            }],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(fixture_roster(), 2, 0, session, tx);
            demo.map_dims = (1000.0, 1000.0);
            demo
        });
        cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.drop_unit(1, "KRI Ahmad Yani", 0.5, 0.5, Some(8), cx)
            })
        });
        assert!(rx.try_recv().is_err(), "a planning embark posts nothing");
        cx.update(|_window, app| {
            let hull = view.read(app).force.get(1).cloned().expect("the child joined");
            assert_eq!(hull.host, Some(8), "staged aboard the host");
            assert_eq!(hull.start, None, "a hull aboard another has no start of its own");
            assert_eq!(hull.commander_id, Some(2));
        });
    }

    /// A planning drop with no seat is refused in the tightest loop: the
    /// refusal is the console's own sentence, the hull never joins the draft,
    /// and nothing is posted — no round trip is spent on a write that cannot
    /// name a commander.
    #[gpui::test]
    fn a_planning_drop_without_a_seat_is_refused_locally(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            state: "planning".to_string(),
            game: "Game 15".to_string(),
            game_id: 45,
            me: 2,
            people: Vec::new(),
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(fixture_roster(), 2, 0, session, tx);
            demo.map_dims = (1000.0, 1000.0);
            demo
        });
        cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.drop_unit(1, "KRI Ahmad Yani", 0.5, 0.5, None, cx)
            })
        });
        assert!(rx.try_recv().is_err(), "nothing posts without a seat");
        cx.update(|_window, app| {
            let demo = view.read(app);
            assert!(
                demo.force_status.contains("holds no seat"),
                "the console's own refusal: {}",
                demo.force_status
            );
            assert!(!demo.force.contains(1), "and the hull never joined the draft");
        });
    }

    /// A staged start becomes a planned marker: projected from the draft's own
    /// lat/lon at the frame's camera, and SKIPPED once the hull is on the wire
    /// — two frames over one hull would be two answers.
    #[test]
    fn planned_marks_project_a_staged_start_and_skip_the_wire() {
        let mut session = SessionView::default();
        session.state = "planning".to_string();
        session.game_id = 45;
        let mut demo = IslandDemo::new(
            fixture_roster(),
            8,
            0,
            session,
            std::sync::mpsc::channel().0,
        );
        demo.map_dims = (1000.0, 1000.0);
        demo.map_center = (0.0, 0.0);
        demo.map_render_zoom = 4.0;
        demo.force.upsert(crate::force::DraftHull {
            unit_id: 1,
            name: "staged".to_string(),
            hull_number: String::new(),
            class_id: String::new(),
            start: Some(crate::force::Start { lat: 0.0, lon: 0.0 }),
            commander_id: Some(2),
            host: None,
        });
        let marks = demo.planned_marks();
        assert_eq!(marks.len(), 1, "one staged hull with a fix is one planned mark");
        assert_eq!(
            (marks[0].0, marks[0].1),
            project_frame_px(0.0, 0.0, (0.0, 0.0), 4.0, 1000.0, 1000.0),
            "projected from the draft start at the frame camera"
        );
        assert_eq!(
            marks[0].2,
            MapSymbolShape::Dot,
            "no taxonomy names it, so the unknown-ship fallback"
        );
        // On the wire, the live marker is the truth about where it is.
        demo.live = vec![LiveMarker {
            id: "1".to_string(),
            x: 500.0,
            y: 500.0,
            stale: false,
            symbol: MapSymbolShape::Dot,
            affiliation: symbology::Affiliation::Unknown,
            heading_deg: None,
            speed_kn: None,
            latitude: 0.0,
            longitude: 0.0,
            trail: 0,
        }];
        assert!(
            demo.planned_marks().is_empty(),
            "a hull on the wire draws its live marker, not a planned one"
        );
    }

    /// The shell's mirrored projection is the same arithmetic as
    /// `map_render`'s: the centre is the frame's centre, and the inverse
    /// round-trips, so a planned marker and a live one share one pixel space.
    #[test]
    fn the_frame_projection_round_trips() {
        let (w, h) = (1280.0, 720.0);
        let center = (1.0, 104.0);
        let zoom = 6.0;
        assert_eq!(
            project_frame_px(center.0, center.1, center, zoom, w, h),
            (w / 2.0, h / 2.0),
            "the centre projects to the frame's centre"
        );
        let (lat, lon) = unproject_frame_px(300.0, 250.0, center, zoom, w, h);
        let (bx, by) = project_frame_px(lat, lon, center, zoom, w, h);
        assert!(
            (bx - 300.0).abs() < 1e-6 && (by - 250.0).abs() < 1e-6,
            "projection and its inverse agree: ({bx}, {by})"
        );
    }

    /// The planning fixture both wheel tests drive: the console's six planning
    /// islands, no session seat, and a stack that overflows the window — which
    /// the console's own heights guarantee, and which is the precondition for a
    /// scroll test to prove anything.
    fn planning_demo<'a>(
        cx: &'a mut gpui::TestAppContext,
    ) -> (gpui::Entity<IslandDemo>, &'a mut gpui::VisualTestContext) {
        planning_demo_with(cx, Vec::new())
    }

    /// The same, with a force long enough that one BODY overflows too.
    fn planning_demo_with_pieces<'a>(
        cx: &'a mut gpui::TestAppContext,
    ) -> (gpui::Entity<IslandDemo>, &'a mut gpui::VisualTestContext) {
        let pieces = (0..40)
            .map(|i| PieceView {
                commander_id: Some(i64::from(8)),
                id: 100 + i,
                label: format!("KRI Hull {i} (KRI-{i})"),
                placed: i % 2 == 0,
            })
            .collect();
        planning_demo_with(cx, pieces)
    }

    fn planning_demo_with<'a>(
        cx: &'a mut gpui::TestAppContext,
        pieces: Vec<PieceView>,
    ) -> (gpui::Entity<IslandDemo>, &'a mut gpui::VisualTestContext) {
        let session = SessionView {
            state: "planning".to_string(),
            game: "Game 15".to_string(),
            // A held game, or every planning body takes its "no session
            // held" branch and there is nothing to scroll — which is exactly
            // how this test passed while asserting nothing.
            game_id: 45,
            pieces,
            ..SessionView::default()
        };
        cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 127, 0, session, std::sync::mpsc::channel().0)
        })
    }

    /// A click on empty space (above the body, in the header band) leaves the
    /// selection alone — the handler is scoped to the rows, not the panel.
    #[gpui::test]
    fn clicking_the_header_does_not_change_selection(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo =
                IslandDemo::new(fixture_roster(), 8, 2, SessionView::default(), std::sync::mpsc::channel().0);
            demo.sheet = true;
            demo
        });

        // The header band sits above BODY_TOP (54): click at panel-local
        // y = 30, well clear of every row.
        cx.simulate_click(
            gpui::point(gpui::px(200.0), gpui::px(95.0)),
            gpui::Modifiers::default(),
        );

        let selected = cx.update(|_window, app| view.read(app).selected);
        assert_eq!(selected, 2, "a click off the rows should not move the selection");
    }

    /// The helm is the shell's one WRITE, so its contract is pinned here: the
    /// order carries the COMMANDED hull and the drafted course and speed, and
    /// it is the same `send_helm` the button calls — never a copy.
    #[gpui::test]
    fn the_helm_sends_the_drafted_order(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: true,
            my_role: "Commando".to_string(),
            mine: vec![MyShip {
                id: "8".to_string(),
                label: "KRI Test (KRI-1)".to_string(),
            }],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, session, tx);
            demo.helm_heading = Some(90.0);
            demo.helm_speed = Some(12.0);
            demo
        });
        let sent = cx.update(|_window, app| view.update(app, |demo, _| demo.send_helm()));
        assert!(sent, "an executing game with a hull and a draft sends");
        let order = match rx.try_recv().expect("the order reached the sink") {
            ToBinary::Helm(o) => o,
            other => panic!("expected a helm order, got {other:?}"),
        };
        assert_eq!(order.unit_id, "8");
        assert_eq!(order.heading_deg, 90.0);
        assert_eq!(order.speed_kn, 12.0);
    }

    /// The map controls are the camera's handle: one step asks the binary for
    /// the new zoom, clamped as the console clamps it — and nothing is asked
    /// for that the clamp would undo.
    #[gpui::test]
    fn a_zoom_step_asks_the_binary(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            zoom: 11.0,
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        cx.update(|_window, app| view.update(app, |demo, cx| demo.zoom_by(1.0, cx)));
        match rx.try_recv().expect("a zoom step reached the sink") {
            ToBinary::Zoom(z) => assert_eq!(z, 12.0),
            other => panic!("expected a zoom, got {other:?}"),
        }
    }

    /// A pan asks in MAP pixels, and an empty one asks for nothing at all —
    /// a zero-delta move is not a gesture.
    #[gpui::test]
    fn a_pan_asks_the_binary(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            )
        });
        cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.pan_by(0.0, 0.0, cx);
                demo.pan_by(12.0, -7.0, cx);
            })
        });
        match rx.try_recv().expect("the pan reached the sink") {
            ToBinary::Pan { dx, dy } => {
                assert_eq!((dx, dy), (12.0, -7.0));
            }
            other => panic!("expected a pan, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "a zero-delta pan asks for nothing");
    }

    /// Centre asks for the SELECTED hull's ground, shifted off the chrome: the
    /// shell names lat/lon from the hull's own fix and the zone's shift, and
    /// the binary unprojects the visible centre from those.
    #[gpui::test]
    fn a_jump_asks_for_the_selected_hulls_ground(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx)
        });
        let sent = cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.live.push(LiveMarker {
                    id: "1".to_string(),
                    x: 0.0,
                    y: 0.0,
                    stale: false,
                    symbol: MapSymbolShape::Frigate,
                    affiliation: symbology::Affiliation::Unknown,
                    heading_deg: Some(90.0),
                    speed_kn: Some(12.0),
                    latitude: -7.88,
                    longitude: 112.54,
                    trail: 3,
                });
                demo.jump_to_selection(cx)
            })
        });
        assert!(sent, "a selection with a fix asks for a jump");
        match rx.try_recv().expect("the jump reached the sink") {
            ToBinary::Jump { lat, lon, shift_px } => {
                assert_eq!((lat, lon), (-7.88, 112.54));
                // The default dock is LEFT (the console's own default), so the
                // zone covers the left and the visible centre moves right by
                // half its width.
                assert_eq!(shift_px, 180.0, "a left dock shifts content right");
            }
            other => panic!("expected a jump, got {other:?}"),
        }
        // And the other dock moves it the other way. Pinned here rather than in
        // a second test because the pair IS the claim: a sign that only worked
        // on one dock would be the classic one-axis-inverted bug.
        cx.update(|_window, app| {
            view.update(app, |demo, _| demo.side_dock_left = false)
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.jump_to_selection(cx))),
            "the right dock still jumps"
        );
        match rx.try_recv().expect("the second jump reached the sink") {
            ToBinary::Jump { shift_px, .. } => {
                assert_eq!(shift_px, -180.0, "a right dock shifts content left");
            }
            other => panic!("expected a jump, got {other:?}"),
        }
    }

    /// And it is SILENT when the selection has no fix: a hull that has never
    /// reported has no ground, and framing `(0, 0)` would be a lie about
    /// where it is.
    #[gpui::test]
    fn a_jump_is_silent_without_a_fix(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx)
        });
        let sent = cx.update(|_window, app| view.update(app, |demo, cx| demo.jump_to_selection(cx)));
        assert!(!sent, "no fix sends nothing");
        assert!(rx.try_recv().is_err(), "nothing reached the sink");
    }

    /// Sign-out tells the binary to drop its token and shows the card: the
    /// session flag is what `render` reads, so the card is one notify away,
    /// and the live markers go with the session rather than lingering for
    /// the next sign-in to inherit.
    #[gpui::test]
    fn signing_out_clears_the_session_and_tells_the_binary(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx)
        });
        cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.live.push(LiveMarker {
                    id: "1".to_string(),
                    x: 0.0,
                    y: 0.0,
                    stale: false,
                    symbol: MapSymbolShape::Frigate,
                    affiliation: symbology::Affiliation::Unknown,
                    heading_deg: None,
                    speed_kn: None,
                    latitude: 0.0,
                    longitude: 0.0,
                    trail: 0,
                });
                demo.sign_out(cx);
            })
        });
        match rx.try_recv().expect("the sign-out reached the sink") {
            ToBinary::SignOut => {}
            other => panic!("expected a sign-out, got {other:?}"),
        }
        let (signed_in, live) = cx.update(|_window, app| {
            view.update(app, |demo, _| (demo.session.signed_in, demo.live.len()))
        });
        assert!(!signed_in, "the card shows next frame");
        assert_eq!(live, 0, "no markers survive the session");
    }

    /// A nameless draft never becomes a POST: the server refuses it, so the
    /// panel does first, in the console's own sentence — the sign-in card's
    /// half-credential rule, repeated for the composer.
    #[gpui::test]
    fn the_composer_refuses_a_nameless_session(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx)
        });
        let status = cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.draft.description = "a draft with no name".to_string();
                demo.submit_compose(cx);
                demo.compose_status.clone()
            })
        });
        assert!(!status.is_empty(), "the form says what is missing");
        assert!(rx.try_recv().is_err(), "nothing was sent");
    }

    /// A named draft sends the WHOLE form — name, mode and every prose row —
    /// because `POST /games` takes them together and a partial send would
    /// silently drop what was typed.
    #[gpui::test]
    fn the_composer_sends_the_whole_draft(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx)
        });
        cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.draft.name = "Latihan Armada 2026".to_string();
                demo.draft.mode = "scenario".to_string();
                demo.draft.description = "about".to_string();
                demo.draft.purpose = "achieve".to_string();
                demo.draft.target = "fleet".to_string();
                demo.draft.area = "java sea".to_string();
                demo.draft.map_tag = "chart-7".to_string();
                demo.submit_compose(cx);
            })
        });
        match rx.try_recv().expect("the draft reached the sink") {
            ToBinary::CreateSession { draft } => {
                assert_eq!(draft.name, "Latihan Armada 2026");
                assert_eq!(draft.mode, "scenario");
                assert_eq!(draft.description, "about");
                assert_eq!(draft.purpose, "achieve");
                assert_eq!(draft.target, "fleet");
                assert_eq!(draft.area, "java sea");
                assert_eq!(draft.map_tag, "chart-7");
            }
            other => panic!("expected a create, got {other:?}"),
        }
        let busy = cx.update(|_window, app| view.update(app, |demo, _| demo.compose_busy));
        assert!(busy, "a second submit waits for the answer");
    }

    /// Applying the answer HOLDS the new game and clears the form — the
    /// console's `apply_created_game`, driven here directly because the drain
    /// calls exactly this.
    #[gpui::test]
    fn applying_a_created_session_holds_it(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx);
            demo.composer_open = true;
            demo.compose_busy = true;
            demo.draft.name = "Latihan Armada 2026".to_string();
            demo
        });
        let held = SessionView {
            game: "Latihan Armada 2026".to_string(),
            state: "planning".to_string(),
            ..SessionView::default()
        };
        cx.update(|_window, app| view.update(app, |demo, _| demo.apply_created(held)));
        let (open, name, mode, game) = cx.update(|_window, app| {
            view.update(app, |demo, _| {
                (
                    demo.composer_open,
                    demo.draft.name.clone(),
                    demo.draft.mode.clone(),
                    demo.session.game.clone(),
                )
            })
        });
        assert!(!open, "the panel closes on success");
        assert!(name.is_empty(), "the form clears for the next session");
        assert_eq!(mode, "maneuver", "the mode resets to the default");
        assert_eq!(game, "Latihan Armada 2026", "the new game is held");
    }

    /// A titleless row never becomes a POST, in the console's own sentence —
    /// the backend requires a title, and it is the one field that cannot be
    /// empty. The composer's nameless rule, repeated for the book.
    #[gpui::test]
    fn the_book_refuses_a_titleless_scenario(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx)
        });
        let status = cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.book_title = "   ".to_string();
                demo.submit_book(cx);
                demo.book_status.clone()
            })
        });
        assert!(!status.is_empty(), "the row says what is missing");
        assert!(rx.try_recv().is_err(), "nothing was sent");
    }

    /// A titled row sends the title trimmed — the server refuses blanks, so
    /// whitespace never spends a round trip finding that out.
    #[gpui::test]
    fn the_book_sends_the_title(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx)
        });
        cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.book_title = "  River crossing  ".to_string();
                demo.submit_book(cx);
            })
        });
        match rx.try_recv().expect("the title reached the sink") {
            ToBinary::AddScenario { title } => assert_eq!(title, "River crossing"),
            other => panic!("expected an add, got {other:?}"),
        }
        let busy = cx.update(|_window, app| view.update(app, |demo, _| demo.book_busy));
        assert!(busy, "a second submit waits for the answer");
    }

    /// Applying the re-read book holds the session and STAYS OPEN — a new
    /// scenario is not a new hold, and the author keeps working. The
    /// composer's apply closes; this one must not.
    #[gpui::test]
    fn applying_a_refreshed_book_holds_it_and_stays_open(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx);
            demo.book_open = true;
            demo.book_busy = true;
            demo.book_title = "River crossing".to_string();
            demo
        });
        let mut held = SessionView::default();
        held.game = "Game 14".to_string();
        held.book = vec![BookScenario {
            id: 4,
            title: "Scenario 1".to_string(),
            steps: vec![BookStep {
                id: 7,
                content: "Do A then B".to_string(),
                window: Some(("1000".to_string(), "1030".to_string())),
            }],
        }];
        cx.update(|_window, app| view.update(app, |demo, _| demo.apply_book(held)));
        let (open, title, scenarios) = cx.update(|_window, app| {
            view.update(app, |demo, _| {
                (demo.book_open, demo.book_title.clone(), demo.session.book.len())
            })
        });
        assert!(open, "the panel stays open across adds");
        assert!(title.is_empty(), "the title row clears for the next one");
        assert_eq!(scenarios, 1, "the re-read book is held");
    }

    /// A held book for the step tests: one scenario, no steps yet.
    #[cfg(test)]
    fn session_with_book() -> SessionView {
        SessionView {
            book: vec![BookScenario {
                id: 4,
                title: "Scenario 1".to_string(),
                steps: Vec::new(),
            }],
            ..SessionView::default()
        }
    }

    /// Empty content is LEGAL — an unwritten step is an authoring state, and
    /// the panel draws it weak — but half a window is refused before the
    /// wire, by the console's own `problem()` rule.
    #[gpui::test]
    fn empty_step_content_is_legal_but_a_half_window_is_not(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session_with_book(), tx)
        });
        // Empty content, no window: sends (the open scenario is the book's
        // first — no tap needed).
        cx.update(|_window, app| view.update(app, |demo, cx| demo.submit_step(cx)));
        match rx.try_recv().expect("the step reached the sink") {
            ToBinary::AddStep { scenario_id, content, window } => {
                assert_eq!(scenario_id, 4);
                assert!(content.is_empty(), "unwritten is a state, not an error");
                assert!(window.is_none());
            }
            other => panic!("expected an add, got {other:?}"),
        }
        // Half a window: refused, and the busy latch never set.
        let (status, busy) = cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.step_busy = false;
                demo.step_draft.content = "Cross the river".to_string();
                demo.step_draft.start_hour = "1000".to_string();
                demo.step_draft.end_hour.clear();
                demo.submit_step(cx);
                (demo.book_status.clone(), demo.step_busy)
            })
        });
        assert!(!status.is_empty(), "the row says what is missing");
        assert!(!busy, "a refused row never latches busy");
        assert!(rx.try_recv().is_err(), "nothing else was sent");
    }

    /// A bad window never reaches the wire either: non-HHMM text, and an end
    /// before its start — both ends of the console's rule, pinned here
    /// rather than re-derived.
    #[gpui::test]
    fn a_bad_window_never_reaches_the_wire(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session_with_book(), tx)
        });
        for (start, end) in [("10", "1030"), ("1100", "1000")] {
            let status = cx.update(|_window, app| {
                view.update(app, |demo, cx| {
                    demo.step_draft.content = "Cross the river".to_string();
                    demo.step_draft.start_hour = start.to_string();
                    demo.step_draft.end_hour = end.to_string();
                    demo.submit_step(cx);
                    demo.book_status.clone()
                })
            });
            assert!(!status.is_empty(), "the row refuses {start}-{end}");
        }
        assert!(rx.try_recv().is_err(), "nothing was sent");
    }

    /// A whole row sends content with its window, and latches busy until the
    /// re-read book answers.
    #[gpui::test]
    fn the_step_row_sends_content_and_window(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session_with_book(), tx)
        });
        cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.step_draft.content = "Cross the river".to_string();
                demo.step_draft.start_hour = "1000".to_string();
                demo.step_draft.end_hour = "1030".to_string();
                demo.submit_step(cx);
            })
        });
        match rx.try_recv().expect("the step reached the sink") {
            ToBinary::AddStep { scenario_id, content, window } => {
                assert_eq!(scenario_id, 4);
                assert_eq!(content, "Cross the river");
                assert_eq!(
                    window,
                    Some(("1000".to_string(), "1030".to_string()))
                );
            }
            other => panic!("expected an add, got {other:?}"),
        }
        let busy = cx.update(|_window, app| view.update(app, |demo, _| demo.step_busy));
        assert!(busy, "a second submit waits for the answer");
    }

    /// Removing a step asks the binary for exactly that step in exactly that
    /// scenario — the join the console's cut carries, without the staging.
    #[gpui::test]
    fn removing_a_step_asks_the_binary(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session_with_book(), tx)
        });
        cx.update(|_window, app| view.update(app, |demo, cx| demo.remove_step(4, 7, cx)));
        match rx.try_recv().expect("the remove reached the sink") {
            ToBinary::RemoveStep { scenario_id, step_id } => {
                assert_eq!((scenario_id, step_id), (4, 7));
            }
            other => panic!("expected a remove, got {other:?}"),
        }
    }

    /// A seated player for the readiness tests: this account's row, by user
    /// id, carrying the seat the binary joins commanded hulls by.
    #[cfg(test)]
    fn session_with_seat(judge: bool, ready: bool) -> SessionView {
        SessionView {
            me: 2,
            my_role: "Commando".to_string(),
            my_ready: ready,
            people: vec![PersonView {
                id: 2,
                name: "Super User".to_string(),
                call_sign: "SUPERUSER".to_string(),
                role: "Commando".to_string(),
                role_id: 19,
                judge,
                ready,
                present: true,
            }],
            ..SessionView::default()
        }
    }

    /// THE composition test, and the one the prototype could not have passed.
    ///
    /// The column's membership is a function of where the exercise is: a
    /// planning game shows the fleet and the room, an executing one shows the
    /// step and the command, and a player station keeps identity, the step,
    /// and its helm. The prototype's three islands were literals in `new()` — same
    /// title, same body, whatever the session said — which is what made the
    /// chrome read as scaffolding rather than as a console.
    #[gpui::test]
    fn the_column_follows_the_state_of_the_exercise(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 127, 0, SessionView::default(), tx)
        });
        let read = |view: &gpui::Entity<IslandDemo>, cx: &mut gpui::VisualTestContext| {
            cx.read(|app| {
                view.read(app)
                    .column_slots()
                    .iter()
                    .map(|s| s.key.id().to_string())
                    .collect::<Vec<_>>()
            })
        };
        let set_state = |view: &gpui::Entity<IslandDemo>,
                         cx: &mut gpui::VisualTestContext,
                         state: &str| {
            cx.update(|_window, app| {
                view.update(app, |demo, _| demo.session.state = state.to_string())
            });
        };
        // NO SEAT: the organizer's column in every state. This is the shape
        // the console draws for a caller who is running the exercise rather
        // than playing in it.
        // PLANNING is the console's own column, island for island. This is
        // the assertion that would have caught the shell shipping a different
        // console's planning room: it listed the register as Fleet and the
        // roster as Crew, and had no terms, clock or gate at all.
        set_state(&view, cx, "planning");
        assert_eq!(
            read(&view, cx),
            vec![
                "operator",
                "essentials",
                "control",
                "fleet",
                "players",
                "review"
            ],
            "planning: the plan, its clock, its force, its room, its gate"
        );
        set_state(&view, cx, "preparation");
        assert_eq!(
            read(&view, cx),
            vec!["operator", "command", "crew"],
            "preparation is the gate: the seat's readiness, and who declared"
        );
        set_state(&view, cx, "execution");
        assert_eq!(
            read(&view, cx),
            vec!["operator", "step", "command", "crew", "log"],
            "execution is the working surface: step, command, room, and the feed"
        );
        set_state(&view, cx, "closure");
        assert_eq!(
            read(&view, cx),
            vec!["operator", "assessment", "log", "crew"],
            "closure is the debrief: what it was, what happened, and who was there"
        );
        // A SEAT, exercise side: execution narrows to identity and the step,
        // and ONLY execution. In planning they are still seating the room, so
        // taking the column off them would take away the surface that does it.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.session.me = 2;
                demo.session.my_role = "Commando".to_string();
                demo.session.people = vec![PersonView {
                    id: 2,
                    name: "Super User".to_string(),
                    call_sign: "SUPERUSER".to_string(),
                    role: "Commando".to_string(),
                    role_id: 19,
                    judge: false,
                    ready: false,
                    present: true,
                }];
            })
        });
        set_state(&view, cx, "planning");
        assert_eq!(
            read(&view, cx),
            vec![
                "operator",
                "essentials",
                "control",
                "fleet",
                "players",
                "review"
            ],
            "a player station keeps the planning column"
        );
        set_state(&view, cx, "execution");
        // The organizer column goes; the helm stays. A player station that
        // lost its command surface in execution would have nothing to do in
        // the state it exists for.
        assert_eq!(
            read(&view, cx),
            vec!["operator", "step", "command"],
            "a player station loses the organizer column but keeps its helm"
        );
    }

    /// Closing an island is the OPERATOR's choice, and it must survive the
    /// exercise changing state under it: shutting the inbox in planning and
    /// keeping it shut in execution is one choice, and a list rebuilt from
    /// the state would silently re-open it in a different slot.
    #[gpui::test]
    fn a_closed_island_stays_closed_across_a_state_change(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 127, 0, SessionView::default(), tx)
        });
        cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.session.state = "planning".to_string();
                demo.toggle_island(view::IslandKey::Crew, cx);
            })
        });
        assert!(
            !cx.read(|app| view.read(app).column_slots().iter().any(|s| s.key == view::IslandKey::Crew)),
            "closing Crew removes it from the planning column"
        );
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.session.state = "execution".to_string();
            })
        });
        assert!(
            !cx.read(|app| view.read(app).column_slots().iter().any(|s| s.key == view::IslandKey::Crew)),
            "and it is still closed in execution, in a different slot"
        );
    }

    /// Hiding the side zone empties the column rather than dimming it. A
    /// hidden zone is a zone nobody asked for, and a dimmed one is a zone
    /// still taking the wheel.
    #[gpui::test]
    fn a_hidden_side_zone_holds_no_islands(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 127, 0, SessionView::default(), tx)
        });
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.session.state = "planning".to_string();
                demo.show_side_zone = false;
            })
        });
        assert!(
            cx.read(|app| view.read(app).column_slots()).is_empty(),
            "the whole column is gated on the one preference"
        );
    }

    /// One modal at a time, the console's `open_only`. The prototype could
    /// show every panel at once — several dimmed cards stacked, each half
    /// behind the next — which is what a row of open buttons produces.
    #[gpui::test]
    fn only_one_modal_opens_at_a_time(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 127, 0, SessionView::default(), tx)
        });
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.open_only(WhichPanel::Settings);
                demo.open_only(WhichPanel::Book);
            })
        });
        assert!(
            cx.read(|app| {
                let d = view.read(app);
                (d.modal_open, d.book_open, d.composer_open)
            }) == (false, true, false),
            "the book's door closed Settings"
        );
        cx.update(|_window, app| {
            view.update(app, |demo, _| demo.open_only(WhichPanel::Settings))
        });
        assert!(
            cx.read(|app| {
                let d = view.read(app);
                (d.modal_open, d.book_open, d.composer_open)
            }) == (true, false, false),
            "and Settings' door closed the book"
        );
    }

    /// The Log island's feed is written BY the writes, and there is no events
    /// GET to read it back from -- so the only honest check is that an ask
    /// appends one human line naming what it was, and that the same ask also
    /// reached the wire. A feed that logged without sending, or sent without
    /// logging, would both pass a capture.
    #[gpui::test]
    fn a_write_appends_one_line_to_the_log(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session_with_seat(false, false), tx)
        });
        assert!(
            cx.read(|app| view.read(app).log_feed.is_empty()),
            "no history is invented on launch"
        );
        let sent =
            cx.update(|_window, app| view.update(app, |demo, cx| demo.set_readiness(true, cx)));
        assert!(sent, "a player declares");
        let lines: Vec<String> = cx.read(|app| view.read(app).log_feed.iter().cloned().collect());
        assert_eq!(lines.len(), 1, "one ask, one line");
        assert!(
            lines[0].contains("readiness"),
            "the line names what happened: {lines:?}"
        );
        match rx.try_recv().expect("the declaration reached the sink") {
            ToBinary::SetReadiness { ready } => assert!(ready),
            other => panic!("expected a readiness write, got {other:?}"),
        }
    }

    /// The console's cap, verbatim: 30 lines, oldest dropped. A feed that grew
    /// without bound would be a leak wearing an island's name.
    #[gpui::test]
    fn the_log_feed_keeps_thirty_lines(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx)
        });
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                for i in 0..40 {
                    demo.feed(format!("line {i}"));
                }
            })
        });
        let feed: Vec<String> = cx.read(|app| view.read(app).log_feed.iter().cloned().collect());
        assert_eq!(feed.len(), 30, "the cap holds");
        assert_eq!(feed.first().map(String::as_str), Some("line 10"), "the oldest went first");
        assert_eq!(feed.last().map(String::as_str), Some("line 39"), "the newest is last");
    }

    /// The timeline's line formatter is the one piece of real logic in the
    /// Assessment island, and it reads Minos' `data` blob by key \u{2014} so the
    /// failures it can have are a renamed field and a wrong shape, both of
    /// which print something plausible rather than nothing. Pinned per event
    /// type, and pinned to NOT panic on a missing key.
    #[gpui::test]
    fn the_timeline_reads_each_event_shape(cx: &mut gpui::TestAppContext) {
        let _ = cx;
        let ev = |etype: &str, data: serde_json::Value| crate::backend::TimelineEvent {
            id: "1".to_string(),
            etype: etype.to_string(),
            assumed_at: "2026-10-14T10:30:00Z".to_string(),
            data,
        };
        // The date is cut and the clock kept: one exercise, so a date per row
        // says nothing.
        let transition = IslandDemo::timeline_line(&ev(
            "transition",
            serde_json::json!({"from_state": "planning", "to_state": "preparation", "changed_by": 7}),
        ));
        assert_eq!(transition, "10:30:00 planning \u{2192} preparation (by 7)");

        // A zero `changed_by` is a server-made change and prints no `(by 0)`.
        let clock = IslandDemo::timeline_line(&ev(
            "clock",
            serde_json::json!({"factor": 12.0, "note": "paused", "changed_by": 0}),
        ));
        assert_eq!(clock, "10:30:00 clock \u{2192} 12x \u{00b7} paused");

        // An order carries its clamp, because a clamped order is an order the
        // server did not obey and the row has to say so.
        let order = IslandDemo::timeline_line(&ev(
            "order",
            serde_json::json!({"unit_name": "KRI-355", "heading_deg": 255.0, "speed_kn": 0.1,
                               "was_clamped": true, "latitude": -5.1, "longitude": 119.4}),
        ));
        assert_eq!(
            order,
            "10:30:00 order KRI-355 255\u{00b0} @ 0 kn \u{00b7} clamped @ (-5.1000, 119.4000)"
        );

        // A shape this build has never seen degrades to its own type name
        // rather than to an empty row or a panic.
        let other = IslandDemo::timeline_line(&ev("teleport", serde_json::json!({})));
        assert_eq!(other, "10:30:00 teleport");

        // And a KNOWN type with a MISSING key still reads, because `str_of`
        // and `num` default rather than index-panic.
        let bare = IslandDemo::timeline_line(&ev("transition", serde_json::json!({})));
        assert_eq!(bare, "10:30:00  \u{2192}  (by 0)");
    }

    /// A timeline reload replaces the list and `load more` extends it — the
    /// console's `SetupDone::Timeline` arm. Either way the cursor block is the
    /// page's and the latch clears, because a page that left the tab busy
    /// would wedge it until relaunch.
    #[gpui::test]
    fn timeline_reload_replaces_and_load_more_extends(cx: &mut gpui::TestAppContext) {
        let ev = |id: &str| crate::backend::TimelineEvent {
            id: id.to_string(),
            etype: "transition".to_string(),
            assumed_at: "2026-10-14T10:30:00Z".to_string(),
            data: serde_json::json!({}),
        };
        let page = |ids: &[&str], cursor: Option<&str>, has_more: bool| {
            crate::backend::TimelinePage {
                events: ids.iter().map(|i| ev(i)).collect(),
                next_cursor: cursor.map(|c| c.to_string()),
                has_more,
            }
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                std::sync::mpsc::channel().0,
            )
        });
        // A held first page, then a reload under new filters: the old rows
        // go, because a cursor never crosses a filter change.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.timeline_busy = true;
                demo.apply_timeline(Ok((page(&["1"], Some("c1"), true), false)));
                assert_eq!(demo.session.timeline.len(), 1);
                assert_eq!(demo.session.timeline_cursor.as_deref(), Some("c1"));
                assert!(demo.session.timeline_has_more);
                assert!(!demo.timeline_busy, "the page clears the latch");
                assert_eq!(demo.timeline_status, "timeline: 1 event(s) · more below");
            })
        });
        // `load more` carries the cursor's next page onto the end, and the
        // last page drops the cursor and the "more below".
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.timeline_busy = true;
                demo.apply_timeline(Ok((page(&["2", "3"], None, false), true)));
                let ids: Vec<&str> =
                    demo.session.timeline.iter().map(|e| e.id.as_str()).collect();
                assert_eq!(ids, vec!["1", "2", "3"]);
                assert_eq!(demo.session.timeline_cursor, None);
                assert!(!demo.session.timeline_has_more);
                assert_eq!(demo.timeline_status, "timeline: 3 event(s)");
            })
        });
    }

    /// A refused timeline read lands on the status line the tab draws and
    /// leaves the held events alone — never a silent drop, never a cleared
    /// list.
    #[gpui::test]
    fn timeline_failure_keeps_the_held_events(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                std::sync::mpsc::channel().0,
            );
            demo.session.timeline = vec![crate::backend::TimelineEvent {
                id: "1".to_string(),
                etype: "transition".to_string(),
                assumed_at: "2026-10-14T10:30:00Z".to_string(),
                data: serde_json::json!({}),
            }];
            demo.timeline_busy = true;
            demo
        });
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.apply_timeline(Err("timeline failed: 400".to_string()));
                assert!(!demo.timeline_busy, "the refusal clears the latch");
                assert_eq!(demo.timeline_status, "timeline failed: 400");
                assert_eq!(demo.session.timeline.len(), 1, "the held page survives");
            })
        });
    }

    /// A non-numeric personnel id refuses BEFORE any request, in the
    /// console's own sentence — rather than becoming a server 400 — and a
    /// request already in flight sends nothing.
    #[gpui::test]
    fn send_timeline_refuses_a_bad_id_and_a_busy_tab(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            demo.timeline_personnel = "abc".to_string();
            demo
        });
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_timeline(false, cx))),
            "a bad id sends nothing"
        );
        assert!(rx.try_recv().is_err(), "no request reached the sink");
        let status = cx.read(|app| view.read(app).timeline_status.clone());
        assert_eq!(status, "personnel must be a numeric id");
        // A bad UNIT id refuses the same way, once personnel parses.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.timeline_personnel = "7".to_string();
                demo.timeline_unit = "k355".to_string();
            })
        });
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_timeline(false, cx))),
            "a bad unit id sends nothing"
        );
        // And a tab with a request in flight sends nothing either.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.timeline_unit.clear();
                demo.timeline_busy = true;
            })
        });
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_timeline(false, cx))),
            "a busy tab sends nothing"
        );
        assert!(rx.try_recv().is_err(), "still nothing reached the sink");
    }

    /// `load more` carries the cursor; a reload never does. A cursor does
    /// not cross a filter change — the console's rule — so the reload drops
    /// it before asking.
    #[gpui::test]
    fn send_timeline_carries_the_cursor_only_to_load_more(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            demo.session.timeline_cursor = Some("c1".to_string());
            demo
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_timeline(true, cx))),
            "load more sends"
        );
        match rx.try_recv().expect("the load-more reached the sink") {
            ToBinary::Timeline { append, cursor, .. } => {
                assert!(append);
                assert_eq!(cursor.as_deref(), Some("c1"), "load more carries the cursor");
            }
            other => panic!("expected a timeline request, got {other:?}"),
        }
        // The latch from that send would gate the next: clear it the way the
        // answer would, then reload.
        cx.update(|_window, app| {
            view.update(app, |demo, _| demo.timeline_busy = false)
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_timeline(false, cx))),
            "reload sends"
        );
        match rx.try_recv().expect("the reload reached the sink") {
            ToBinary::Timeline { append, cursor, .. } => {
                assert!(!append);
                assert_eq!(cursor, None, "a reload never carries a cursor");
            }
            other => panic!("expected a timeline request, got {other:?}"),
        }
    }

    /// A Judgements reload replaces the held marks — the console's
    /// `SetupDone::Judgements` arm, whose status sentence this keeps. A
    /// refusal lands on the tab's status line and leaves the held marks
    /// alone, and a request already in flight sends nothing.
    #[gpui::test]
    fn judgements_reload_replaces_the_marks(cx: &mut gpui::TestAppContext) {
        let mark = |id: i64| crate::backend::Judgement {
            id,
            personnel_id: 7,
            personnel_name: "Trooper".to_string(),
            judge_id: 2,
            judge_name: "Judge".to_string(),
            cited_unit: None,
            assumed_at: String::new(),
            score: "pass".to_string(),
            created_at: String::new(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            demo.session.judgements = vec![mark(1)];
            demo
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_judgements(cx))),
            "reload sends"
        );
        match rx.try_recv().expect("the reload reached the sink") {
            ToBinary::ReloadJudgements => {}
            other => panic!("expected a judgements reload, got {other:?}"),
        }
        // The latch from that send gates the next until the answer lands.
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_judgements(cx))),
            "a busy tab sends nothing"
        );
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.apply_judgements(Ok(vec![mark(2), mark(3)]));
                let ids: Vec<i64> = demo.session.judgements.iter().map(|j| j.id).collect();
                assert_eq!(ids, vec![2, 3], "the reload replaces, not extends");
                assert!(!demo.judgements_busy, "the answer clears the latch");
                assert_eq!(demo.judgements_status, "judgements: 2 mark(s)");
            })
        });
        // And a refusal keeps the held list with the reason on the line.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.judgements_busy = true;
                demo.apply_judgements(Err("judgements failed: 400".to_string()));
                assert!(!demo.judgements_busy);
                assert_eq!(demo.judgements_status, "judgements failed: 400");
                assert_eq!(demo.session.judgements.len(), 2, "the held marks survive");
            })
        });
    }

    /// A Reviews reload replaces the held documents, and `mine only`
    /// narrows the read server-side to this authorship — the console's
    /// `review_mine_only`, resolved to the seat before asking, because the
    /// binary holds the token but never the seating.
    #[gpui::test]
    fn reviews_reload_narrows_to_mine_only(cx: &mut gpui::TestAppContext) {
        let doc = |id: i64| crate::backend::Review {
            id,
            personnel_id: 7,
            personnel_name: "Trooper".to_string(),
            author_id: 2,
            author_name: "Judge".to_string(),
            body: "held the line".to_string(),
            revised: false,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            demo.session.me = 7;
            demo.session.reviews = vec![doc(1)];
            demo.review_mine_only = true;
            demo
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_reviews(cx))),
            "reload sends"
        );
        match rx.try_recv().expect("the reload reached the sink") {
            ToBinary::ReloadReviews { author } => {
                assert_eq!(author, Some(7), "mine only narrows to this seat");
            }
            other => panic!("expected a reviews reload, got {other:?}"),
        }
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.reviews_busy = false;
                demo.review_mine_only = false;
            })
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_reviews(cx))),
            "reload sends unfiltered"
        );
        match rx.try_recv().expect("the reload reached the sink") {
            ToBinary::ReloadReviews { author } => {
                assert_eq!(author, None, "unchecked reads the whole exercise");
            }
            other => panic!("expected a reviews reload, got {other:?}"),
        }
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.apply_reviews(Ok(vec![doc(2)]));
                assert_eq!(demo.session.reviews.len(), 1);
                assert_eq!(demo.session.reviews[0].id, 2, "the reload replaces");
                assert!(!demo.reviews_busy, "the answer clears the latch");
                assert_eq!(demo.reviews_status, "reviews: 1 document(s)");
            })
        });
        // A seatless shell with the narrow checked reads unfiltered rather
        // than asking for author 0 — the console's seatless flattening.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.session.me = 0;
                demo.review_mine_only = true;
            })
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_reviews(cx))),
            "reload sends seatless"
        );
        match rx.try_recv().expect("the reload reached the sink") {
            ToBinary::ReloadReviews { author } => {
                assert_eq!(author, None, "no seat means no narrow");
            }
            other => panic!("expected a reviews reload, got {other:?}"),
        }
    }

    /// The record guards run in the console's order and speak its
    /// sentences: the seat, the subject, the self-judge refusal, then the
    /// score. A judge seat with a subject and a score posts the pick
    /// trimmed, and a request in flight sends nothing.
    #[gpui::test]
    fn record_judgement_guards_run_in_order(cx: &mut gpui::TestAppContext) {
        let seat = |id: i64, judge: bool| PersonView {
            id,
            name: if judge { "Judge".to_string() } else { "Trooper".to_string() },
            call_sign: String::new(),
            role: String::new(),
            role_id: 0,
            judge,
            ready: false,
            present: true,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            // Seated, but not judge-side: the verb is absent and the send
            // refuses with the banner sentence.
            demo.session.me = 2;
            demo.session.people = vec![seat(2, false), seat(7, false)];
            demo
        });
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_judgement(cx))),
            "a non-judge sends nothing"
        );
        assert!(rx.try_recv().is_err(), "no request reached the sink");
        let status = cx.read(|app| view.read(app).judgements_status.clone());
        assert!(status.contains("judge-side seat"), "the banner sentence, got {status:?}");
        // Judge-side, but the room holds nobody else: no subjects, and the
        // send refuses with the subject sentence rather than an id prompt
        // — the shell's subjects ride the held session.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.session.people = vec![seat(2, true)];
            })
        });
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_judgement(cx))),
            "no subject sends nothing"
        );
        let status = cx.read(|app| view.read(app).judgements_status.clone());
        assert_eq!(status, "pick a subject first");
        // A subject and no score: refused before any request.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.session.people = vec![seat(2, true), seat(7, false)];
            })
        });
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_judgement(cx))),
            "no score sends nothing"
        );
        let status = cx.read(|app| view.read(app).judgements_status.clone());
        assert_eq!(status, "write the score first");
        // A subject and a score: the pick posts trimmed.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.judge_score = "  pass  ".to_string();
            })
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_judgement(cx))),
            "a full draft sends"
        );
        match rx.try_recv().expect("the record reached the sink") {
            ToBinary::RecordJudgement { personnel_id, score } => {
                assert_eq!(personnel_id, 7, "the picked subject, not the caller");
                assert_eq!(score, "pass", "the score posts trimmed");
            }
            other => panic!("expected a record, got {other:?}"),
        }
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_judgement(cx))),
            "a write in flight sends nothing"
        );
        assert!(rx.try_recv().is_err(), "still nothing reached the sink");
    }

    /// Filing refuses an empty room and an empty body before any request;
    /// revising refuses nothing under edit and an empty revision the same
    /// way. Both post trimmed, and a write in flight sends nothing.
    #[gpui::test]
    fn file_and_revise_guard_their_drafts(cx: &mut gpui::TestAppContext) {
        let seat = |id: i64| PersonView {
            id,
            name: if id == 2 { "Zulu".to_string() } else { "Alpha".to_string() },
            call_sign: String::new(),
            role: String::new(),
            role_id: 0,
            judge: false,
            ready: false,
            present: true,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            demo.session.me = 2;
            demo
        });
        // Nobody seated: filing refuses with the subject sentence — the
        // console's weak line, not a numeric prompt.
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_review(cx))),
            "no subject sends nothing"
        );
        let status = cx.read(|app| view.read(app).reviews_status.clone());
        assert_eq!(status, "pick a subject first");
        // A subject and no body: refused before any request.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.session.people = vec![seat(2), seat(7)];
            })
        });
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_review(cx))),
            "no body sends nothing"
        );
        let status = cx.read(|app| view.read(app).reviews_status.clone());
        assert_eq!(status, "write the review first");
        // A subject and a body: the pick posts trimmed.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.review_body = "  held the line  ".to_string();
            })
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_review(cx))),
            "a full draft sends"
        );
        match rx.try_recv().expect("the file reached the sink") {
            ToBinary::FileReview { personnel_id, body } => {
                assert_eq!(personnel_id, 7, "the resolved subject");
                assert_eq!(body, "held the line", "the body posts trimmed");
            }
            other => panic!("expected a file, got {other:?}"),
        }
        // Revising with nothing under edit refuses; with a row queued but
        // no words, refuses the same way filing does.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.review_write_busy = false;
                demo.review_body.clear();
            })
        });
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_revise(cx))),
            "nothing queued sends nothing"
        );
        let status = cx.read(|app| view.read(app).reviews_status.clone());
        assert_eq!(status, "nothing under revision");
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.review_editing = Some(9);
            })
        });
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_revise(cx))),
            "no words sends nothing"
        );
        let status = cx.read(|app| view.read(app).reviews_status.clone());
        assert_eq!(status, "write the revision first");
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.review_body = "stood relieved".to_string();
            })
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_revise(cx))),
            "a queued revision sends"
        );
        match rx.try_recv().expect("the revision reached the sink") {
            ToBinary::ReviseReview { review_id, body } => {
                assert_eq!(review_id, 9, "the row under edit");
                assert_eq!(body, "stood relieved");
            }
            other => panic!("expected a revision, got {other:?}"),
        }
    }

    /// Ending the session posts the closure transition once and holds the
    /// re-read game on the answer. Ungated by seat like the console's
    /// `close_game` — organizer-only by column placement, server-arbitrated
    /// on top.
    #[gpui::test]
    fn end_session_posts_once_and_holds_closure(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            // No seat: the organizer column, so the verb is live.
            demo.session.execution = true;
            demo
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_end_session(cx))),
            "the verb sends"
        );
        match rx.try_recv().expect("the transition reached the sink") {
            ToBinary::EndSession => {}
            other => panic!("expected an end-session, got {other:?}"),
        }
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_end_session(cx))),
            "an end in flight sends nothing"
        );
        // The answer holds the closed game: the latch clears and the
        // column recomposes to the debrief around it.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                let mut closed = SessionView::default();
                closed.state = "closure".to_string();
                demo.apply_end(closed);
                assert!(!demo.end_busy, "the answer clears the latch");
                assert!(demo.end_status.is_empty());
                assert_eq!(demo.session.state, "closure");
            })
        });
        // No seat gate on the send — the console has none either, gating
        // by column placement instead. A player station that posts is
        // refused loudly by the server, never silently here.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.session.me = 2;
                demo.session.people = vec![PersonView {
                    id: 2,
                    name: "Trooper".to_string(),
                    call_sign: String::new(),
                    role: "Commando".to_string(),
                    role_id: 19,
                    judge: false,
                    ready: false,
                    present: true,
                }];
            })
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_end_session(cx))),
            "the send names no seat"
        );
        match rx.try_recv().expect("the transition reached the sink") {
            ToBinary::EndSession => {}
            other => panic!("expected an end-session, got {other:?}"),
        }
    }

    /// Holding a game names the picked id and installs the re-read whole:
    /// the row drew the id, the answer arrives read back, and the column
    /// recomposes around it. A hold in flight sends nothing more.
    #[gpui::test]
    fn hold_names_the_picked_game(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            demo.sessions = vec![
                SessionRow {
                    id: 44,
                    name: "GPUI step proof".to_string(),
                    state: "execution".to_string(),
                    my_role: "Commando".to_string(),
                    judge: false,
                },
                SessionRow {
                    id: 9,
                    name: "Game 8".to_string(),
                    state: "planning".to_string(),
                    my_role: String::new(),
                    judge: false,
                },
            ];
            demo
        });
        // Unheld: the column is the picker, not mid-execution.
        let keys = cx.read(|app| {
            view.read(app)
                .column_slots()
                .iter()
                .map(|s| s.key.id().to_string())
                .collect::<Vec<_>>()
        });
        assert_eq!(keys, vec!["operator", "sessions", "fleet"]);
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_hold(9, cx))),
            "the pick sends"
        );
        match rx.try_recv().expect("the hold reached the sink") {
            ToBinary::HoldSession { game_id } => assert_eq!(game_id, 9, "the row's id"),
            other => panic!("expected a hold, got {other:?}"),
        }
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_hold(44, cx))),
            "a hold in flight sends nothing"
        );
        assert!(rx.try_recv().is_err(), "nothing reached the sink");
        // The answer installs the held game whole: the latch clears and
        // the list survives for the next pick.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                let mut held = SessionView::default();
                held.game_id = 9;
                held.game = "Game 8".to_string();
                held.state = "planning".to_string();
                demo.apply_hold(held);
                assert!(!demo.hold_busy, "the answer clears the latch");
                assert!(demo.hold_status.is_empty());
                assert_eq!(demo.session.game_id, 9);
                assert_eq!(demo.sessions.len(), 2, "the list survives the hold");
            })
        });
        let keys = cx.read(|app| {
            view.read(app)
                .column_slots()
                .iter()
                .map(|s| s.key.id().to_string())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            keys,
            vec!["operator", "essentials", "control", "fleet", "players", "review"],
            "the column recomposes around the held game"
        );
    }

    /// Clicking a session row holds it — the compositor's live click,
    /// verified deterministically like the roster rows. Row 1's centre is
    /// computed from the island's own origin (band TITLE_H, body PAD, half
    /// a row), never hardcoded to a window.
    #[gpui::test]
    fn clicking_a_session_row_holds_it(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            demo.sessions = vec![
                SessionRow {
                    id: 44,
                    name: "GPUI step proof".to_string(),
                    state: "execution".to_string(),
                    my_role: "Commando".to_string(),
                    judge: false,
                },
                SessionRow {
                    id: 9,
                    name: "Game 8".to_string(),
                    state: "planning".to_string(),
                    my_role: String::new(),
                    judge: false,
                },
            ];
            demo
        });
        cx.simulate_resize(gpui::size(gpui::px(768.0), gpui::px(838.0)));
        let (ix, top) = cx.update(|window, app| {
            let d = view.read(app);
            let band = zone_band_at(viewport(window), d.zone_w, d.side_dock_left);
            let slots = d.column_slots();
            let i = slots
                .iter()
                .position(|s| s.key == view::IslandKey::Sessions)
                .expect("sessions listed while unheld");
            view::zone_slot_origin(band, &slots, i)
        });
        // First row centre: band TITLE_H, body PAD, half a row. The island
        // edge sits UNDER the body in tree order, so body controls are
        // hoverable — an edge painted last would swallow this click (the
        // defect this test pins).
        cx.simulate_click(
            gpui::point(gpui::px(ix + 60.0), gpui::px(top + view::TITLE_H + view::PAD + 11.0)),
            gpui::Modifiers::default(),
        );
        match rx.try_recv().expect("the pick reached the sink") {
            ToBinary::HoldSession { game_id } => assert_eq!(game_id, 44, "row 1 holds game 44"),
            other => panic!("expected a hold, got {other:?}"),
        }
    }

    /// Dragging a leaf row onto the map deploys it there — source payload,
    /// ghost, map drop and fractions, proven in-harness end to end. The
    /// source is found by sweep, never by hardcoded pixels: the press walks
    /// down the leaf column until GPUI reports an active drag. The hull is
    /// NOT in the exercise, so the drop assigns first — placing IS
    /// assigning — and the verb carries the operator's own seat.
    #[gpui::test]
    fn dragging_a_leaf_row_to_the_map_deploys_it(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut session = SessionView::default();
            session.me = 7;
            session.people = vec![PersonView {
                id: 7,
                name: "SU".to_string(),
                call_sign: "SUEX".to_string(),
                role: "Commando".to_string(),
                role_id: 1,
                judge: false,
                ready: false,
                present: true,
            }];
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, session, tx);
            demo.fleet_tax = super::fleet_taxonomy_tests::fixture_taxonomy();
            demo.select_category(2);
            demo.select_type(3);
            demo.select_class(5);
            demo.fleet_open = true;
            demo
        });
        cx.simulate_resize(gpui::size(gpui::px(768.0), gpui::px(838.0)));
        // The live viewport, never assumed: a resize that lags a frame
        // would put hardcoded points outside the window, and a drop outside
        // the window is a silence, not a failure.
        let (w, h) = cx.update(|window, _| {
            let s = window.viewport_size();
            (f32::from(s.width), f32::from(s.height))
        });
        // The leaf column sits ~486pt inside the centred 700-wide modal.
        let lx = (w - 700.0) / 2.0 + 486.0;
        // Bare map below the centred modal and clear of the left column.
        let (dx, dy) = (w - 68.0, h - 58.0);
        // The sweep-and-drop in one pass: press-and-nudge down the leaf
        // column, and the press that starts a drag KEEPS going — straight
        // to bare map below the modal, where it releases. One pass because
        // a probe release fires the drop too (the map underlies everything
        // hoverable), and that deploy re-renders the picker's status line,
        // shifting every row below the press that found it. Probes that
        // find nothing release harmlessly; clicks that land on add post
        // force plans, skipped at the sink below.
        let mut found = false;
        let mut y = 150.0;
        while y < h - 100.0 && !found {
            cx.simulate_mouse_down(
                gpui::point(gpui::px(lx), gpui::px(y)),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
            cx.simulate_mouse_move(
                gpui::point(gpui::px(lx), gpui::px(y + 20.0)),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
            if cx.update(|_, app| app.has_active_drag()) {
                found = true;
                for (x, yy) in
                    [(lx + 40.0, y + 60.0), (w - 148.0, 500.0), (w - 108.0, 640.0), (dx, dy)]
                {
                    cx.simulate_mouse_move(
                        gpui::point(gpui::px(x), gpui::px(yy)),
                        gpui::MouseButton::Left,
                        gpui::Modifiers::default(),
                    );
                }
            }
            cx.simulate_mouse_up(
                gpui::point(gpui::px(if found { dx } else { lx }), gpui::px(if found { dy } else { y + 20.0 })),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
            y += 8.0;
        }
        assert!(found, "some leaf row must start a drag");
        // The drop is last: anything the sweep posted before it (force
        // plans from add clicks) is skipped, the assign-and-deploy is
        // verified.
        let mut deploy = None;
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg, ToBinary::DeployAssign { .. }) {
                deploy = Some(msg);
            }
        }
        match deploy.expect("the drop reached the sink") {
            ToBinary::DeployAssign { unit_id, commander_id, fx, fy } => {
                assert_eq!(unit_id, 13, "the first leaf row drags Yani");
                assert_eq!(commander_id, 7, "placing assigns to the operator's seat");
                assert!(
                    ((fx - f64::from(dx / w)) as f32).abs() < 0.03,
                    "fx names the drop, got {fx}"
                );
                assert!(
                    ((fy - f64::from(dy / h)) as f32).abs() < 0.03,
                    "fy names the drop, got {fy}"
                );
            }
            other => panic!("expected an assign-and-deploy, got {other:?}"),
        }
        let _ = view;
    }

    /// Dropping a leaf row on a placed host's marker embarks it aboard —
    /// the same sweep-and-hold path as the deploy proof, with a live marker
    /// under the drop. The marker is the test's own (map space 1:1 with the
    /// window here), the host is a placed piece, and the child rides along.
    #[gpui::test]
    fn dropping_a_leaf_row_on_a_host_embarks_it(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut session = SessionView::default();
            session.pieces = vec![
                PieceView {
                    id: 8,
                    label: "KRI Ahmad Yani".to_string(),
                    commander_id: None,
                    placed: true,
                },
                PieceView {
                    id: 13,
                    label: "KRI Yos Sudarso".to_string(),
                    commander_id: None,
                    placed: false,
                },
            ];
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, session, tx);
            demo.fleet_tax = super::fleet_taxonomy_tests::fixture_taxonomy();
            demo.select_category(2);
            demo.select_type(3);
            demo.select_class(5);
            demo.fleet_open = true;
            // Map space 1:1 with the window, so marker pixels ARE window
            // pixels and the drop below names the marker exactly.
            demo.map_dims = (768.0, 838.0);
            demo.live = vec![LiveMarker {
                id: "8".to_string(),
                x: 600.0,
                y: 750.0,
                stale: false,
                symbol: MapSymbolShape::Frigate,
                affiliation: symbology::Affiliation::Unknown,
                heading_deg: None,
                speed_kn: None,
                latitude: 0.0,
                longitude: 0.0,
                trail: 0,
            }];
            demo
        });
        cx.simulate_resize(gpui::size(gpui::px(768.0), gpui::px(838.0)));
        let (w, h) = cx.update(|window, _| {
            let s = window.viewport_size();
            (f32::from(s.width), f32::from(s.height))
        });
        let lx = (w - 700.0) / 2.0 + 486.0;
        // Onto the host's marker: below the centred modal, clear of the
        // left column — bare map with a hull on it.
        let (dx, dy) = (600.0, 750.0);
        let mut found = false;
        let mut y = 150.0;
        while y < h - 100.0 && !found {
            cx.simulate_mouse_down(
                gpui::point(gpui::px(lx), gpui::px(y)),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
            cx.simulate_mouse_move(
                gpui::point(gpui::px(lx), gpui::px(y + 20.0)),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
            if cx.update(|_, app| app.has_active_drag()) {
                found = true;
                for (x, yy) in [(lx + 40.0, y + 60.0), (620.0, 700.0), (dx, dy)] {
                    cx.simulate_mouse_move(
                        gpui::point(gpui::px(x), gpui::px(yy)),
                        gpui::MouseButton::Left,
                        gpui::Modifiers::default(),
                    );
                }
            }
            cx.simulate_mouse_up(
                gpui::point(
                    gpui::px(if found { dx } else { lx }),
                    gpui::px(if found { dy } else { y + 20.0 }),
                ),
                gpui::MouseButton::Left,
                gpui::Modifiers::default(),
            );
            y += 8.0;
        }
        assert!(found, "some leaf row must start a drag");
        let mut embark = None;
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg, ToBinary::EmbarkDrop { .. }) {
                embark = Some(msg);
            }
        }
        match embark.expect("the drop reached the sink") {
            ToBinary::EmbarkDrop { host, child } => {
                assert_eq!(host, 8, "the drop named the host marker");
                assert_eq!(child, 13, "the first leaf row rides along");
            }
            other => panic!("expected an embark, got {other:?}"),
        }
        let _ = view;
    }

    /// The picker hides while a drag is in flight and returns when it
    /// ends — the flag the tick polls, pinned here without a pointer.
    #[gpui::test]
    fn dragging_hides_the_picker(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                std::sync::mpsc::channel().0,
            );
            demo.fleet_open = true;
            demo
        });
        cx.update(|_, app| {
            view.update(app, |demo, cx| demo.set_dragging(true, cx));
        });
        let hiding = cx.update(|_, app| view.read(app).dragging);
        assert!(hiding, "a live drag hides the picker");
        cx.update(|_, app| {
            view.update(app, |demo, cx| demo.set_dragging(false, cx));
        });
        let back = cx.update(|_, app| {
            let d = view.read(app);
            !d.dragging && d.fleet_open
        });
        assert!(back, "the drop's end returns the picker, still open");
    }

    /// Pressing the fleet picker's card never pans the map — the press
    /// belongs to the picker, and the map handle underneath must not see
    /// it. GPUI delivers a press to EVERY hovered hitbox down the chain,
    /// and the full-window map surface is always in it — so without the
    /// card truncating hover, every picker drag (and every press-and-move
    /// on the card) drags the map along with the gesture.
    #[gpui::test]
    fn pressing_the_picker_card_does_not_pan_the_map(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (_view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            demo.fleet_tax = super::fleet_taxonomy_tests::fixture_taxonomy();
            demo.select_category(2);
            demo.select_type(3);
            demo.select_class(5);
            demo.fleet_open = true;
            // A framed map, or panning computes nothing: the pan scales by
            // the frame's dimensions, and a (0, 0) frame refuses every move.
            demo.map_dims = (768.0, 838.0);
            demo
        });
        cx.simulate_resize(gpui::size(gpui::px(768.0), gpui::px(838.0)));
        // The card's right padding: dead card background, no control. A
        // press-drag here must reach nothing but the card.
        let at = gpui::point(gpui::px(720.0), gpui::px(400.0));
        cx.simulate_mouse_down(at, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            gpui::point(gpui::px(740.0), gpui::px(440.0)),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            gpui::point(gpui::px(740.0), gpui::px(440.0)),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        let mut panned = false;
        while let Ok(msg) = rx.try_recv() {
            if matches!(msg, ToBinary::Pan { .. }) {
                panned = true;
            }
        }
        assert!(!panned, "a press on the picker card must not pan the map");
    }

    /// The seat-me pills offer the player-side roles — never the judge
    /// side, which the server refuses as a commander — and nothing at all
    /// when every role judges, in which case seating stays the Players
    /// island's job.
    #[test]
    fn player_roles_exclude_the_judge_side() {
        let mut session = SessionView::default();
        session.roles = vec![
            RoleRow { id: 1, name: "Commando".to_string(), judge: false },
            RoleRow { id: 9, name: "Penyelenggara".to_string(), judge: true },
        ];
        let demo = IslandDemo::new(
            fixture_roster(),
            8,
            0,
            session,
            std::sync::mpsc::channel().0,
        );
        let pills = demo.player_roles();
        assert_eq!(pills.len(), 1, "one player role, one pill");
        assert_eq!(pills[0].id, 1);
    }

    #[test]
    fn player_roles_empty_when_every_seat_judges() {
        let mut session = SessionView::default();
        session.roles = vec![
            RoleRow { id: 9, name: "Penyelenggara".to_string(), judge: true },
        ];
        let demo = IslandDemo::new(
            fixture_roster(),
            8,
            0,
            session,
            std::sync::mpsc::channel().0,
        );
        assert!(demo.player_roles().is_empty(), "no player role, no pill");
    }

    /// The seat-me pill stages exactly one write: this account, a
    /// player-side role, the name-derived call sign — the same seat the
    /// Players island would stage, without the trip there.
    #[gpui::test]
    fn seat_me_stages_a_player_seat(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut session = SessionView::default();
            session.me = 7;
            session.roles = vec![
                RoleRow { id: 1, name: "Commando".to_string(), judge: false },
                RoleRow { id: 9, name: "Penyelenggara".to_string(), judge: true },
            ];
            session.directory = vec![DirectoryRow {
                id: 7,
                name: "Super User".to_string(),
                unit: String::new(),
                rank: String::new(),
                appointment: String::new(),
            }];
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        let role = cx.update(|_, app| {
            let mut pills = view.read(app).player_roles();
            assert_eq!(pills.len(), 1, "one player role in the fixture");
            pills.remove(0)
        });
        cx.update(|_, app| {
            view.update(app, |demo, cx| {
                demo.stage_seat(7, "Super User", &role)
                    .expect("a fresh room seats");
            })
        });
        match rx.try_recv().expect("the seat posted") {
            ToBinary::SyncRoster { writes } => {
                assert_eq!(writes.len(), 1, "one seat, nothing else");
                match &writes[0] {
                    crate::roster::RosterWrite::Seat { user_id, role_id, call_sign, .. } => {
                        assert_eq!(*user_id, 7);
                        assert_eq!(*role_id, 1, "a player-side role, never the judge side");
                        assert_eq!(call_sign, "SUPER US", "the name-derived sign");
                    }
                    other => panic!("expected a seat, got {other:?}"),
                }
            }
            other => panic!("expected a roster plan, got {other:?}"),
        }
    }

    /// The pill in the picker's force section seats without the trip to
    /// the Players island: swept like the place button, because the pill's
    /// seat moves with the verdict line. The first roster plan posted is
    /// the pill's — add clicks refuse seatless without posting.
    #[gpui::test]
    fn seat_pill_seats_from_the_picker(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut session = SessionView::default();
            session.me = 7;
            session.roles = vec![
                RoleRow { id: 1, name: "Commando".to_string(), judge: false },
                RoleRow { id: 9, name: "Penyelenggara".to_string(), judge: true },
            ];
            session.directory = vec![DirectoryRow {
                id: 7,
                name: "Super User".to_string(),
                unit: String::new(),
                rank: String::new(),
                appointment: String::new(),
            }];
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, session, tx);
            demo.fleet_open = true;
            demo.force_status = "sweep".to_string();
            demo
        });
        cx.simulate_resize(gpui::size(gpui::px(768.0), gpui::px(838.0)));
        let mut seated = false;
        let mut y = 180.0;
        while y < 300.0 && !seated {
            let mut x = 46.0;
            while x < 260.0 && !seated {
                cx.simulate_click(
                    gpui::point(gpui::px(x), gpui::px(y)),
                    gpui::Modifiers::default(),
                );
                while let Ok(msg) = rx.try_recv() {
                    if matches!(msg, ToBinary::SyncRoster { .. }) {
                        seated = true;
                    }
                }
                x += 12.0;
            }
            y += 8.0;
        }
        assert!(seated, "some click took a player seat");
        let _ = view;
    }

    /// A seatless drop posts nothing: without a seat there is nobody to
    /// hand the hull to, and the refusal says so on the picker's line.
    #[gpui::test]
    fn dropping_without_a_seat_posts_nothing(cx: &mut gpui::TestAppContext) {        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            )
        });
        cx.update(|_, app| {
            view.update(app, |demo, cx| {
                demo.drop_unit(13, "KRI Ahmad Yani", 0.5, 0.5, None, cx);
            })
        });
        assert!(rx.try_recv().is_err(), "no seat, no post");
        let status = cx.update(|_, app| view.read(app).force_status.clone());
        assert_eq!(status, "your account holds no seat in this room");
    }

    /// A judge-side seat may not command: the drop refuses locally — the
    /// server's rule, heard here first — with the way forward, and posts
    /// nothing.
    #[gpui::test]
    fn dropping_from_a_judge_side_seat_posts_nothing(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut session = SessionView::default();
            session.me = 7;
            session.roles = vec![
                RoleRow { id: 1, name: "Commando".to_string(), judge: false },
                RoleRow { id: 9, name: "Penyelenggara".to_string(), judge: true },
            ];
            session.people = vec![PersonView {
                id: 7,
                name: "SU".to_string(),
                call_sign: "SUEX".to_string(),
                role: "Penyelenggara".to_string(),
                role_id: 9,
                judge: true,
                ready: false,
                present: true,
            }];
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        cx.update(|_, app| {
            view.update(app, |demo, cx| {
                demo.drop_unit(13, "KRI Ahmad Yani", 0.5, 0.5, None, cx);
            })
        });
        assert!(rx.try_recv().is_err(), "a judge-side seat posts nothing");
        let status = cx.update(|_, app| view.read(app).force_status.clone());
        assert_eq!(
            status,
            "your seat is judge-side \u{2014} it directs, it does not command; take a player seat on the Players island"
        );
    }

    /// `own_steers` reads the operator's own seat: `None` unseated, `Some`
    /// whether the held role is player-side. The seat-me pills and the
    /// judge-side drop gate both read it — the pills show when it is
    /// `None` or `Some(false)`, the drop refuses when it is `Some(false)`.
    #[test]
    fn own_steers_reads_the_operators_seat() {
        let session_with = |role_id: i64, judge: bool| {
            let mut session = SessionView::default();
            session.me = 7;
            session.roles = vec![
                RoleRow { id: 1, name: "Commando".to_string(), judge: false },
                RoleRow { id: 9, name: "Penyelenggara".to_string(), judge: true },
            ];
            session.people = vec![PersonView {
                id: 7,
                name: "SU".to_string(),
                call_sign: "SUEX".to_string(),
                role: if judge { "Penyelenggara" } else { "Commando" }.to_string(),
                role_id,
                judge,
                ready: false,
                present: true,
            }];
            session
        };
        let demo = IslandDemo::new(
            fixture_roster(),
            8,
            0,
            SessionView::default(),
            std::sync::mpsc::channel().0,
        );
        assert_eq!(demo.own_steers(), None, "no seat, no steering");
        let demo = IslandDemo::new(
            fixture_roster(),
            8,
            0,
            session_with(1, false),
            std::sync::mpsc::channel().0,
        );
        assert_eq!(demo.own_steers(), Some(true), "a player seat steers");
        let demo = IslandDemo::new(
            fixture_roster(),
            8,
            0,
            session_with(9, true),
            std::sync::mpsc::channel().0,
        );
        assert_eq!(demo.own_steers(), Some(false), "a judge seat directs");
    }

    /// A seated drop of a register hull assigns and deploys in one verb —
    /// placing IS assigning — carrying the operator's own seat.
    #[gpui::test]
    fn dropping_a_register_hull_assigns_and_deploys(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut session = SessionView::default();
            session.me = 7;
            session.people = vec![PersonView {
                id: 7,
                name: "SU".to_string(),
                call_sign: "SUEX".to_string(),
                role: "Commando".to_string(),
                role_id: 1,
                judge: false,
                ready: false,
                present: true,
            }];
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        cx.update(|_, app| {
            view.update(app, |demo, cx| {
                demo.drop_unit(13, "KRI Ahmad Yani", 0.5, 0.5, None, cx);
            })
        });
        match rx.try_recv().expect("the drop posted") {
            ToBinary::DeployAssign { unit_id, commander_id, fx, fy } => {
                assert_eq!(unit_id, 13);
                assert_eq!(commander_id, 7, "the operator's own seat");
                assert_eq!((fx, fy), (0.5, 0.5));
            }
            other => panic!("expected an assign-and-deploy, got {other:?}"),
        }
    }

    /// The host-side twin: a seated drop onto a host assigns and embarks.
    #[gpui::test]
    fn dropping_a_register_hull_on_a_host_assigns_and_embarks(
        cx: &mut gpui::TestAppContext,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut session = SessionView::default();
            session.me = 7;
            session.people = vec![PersonView {
                id: 7,
                name: "SU".to_string(),
                call_sign: "SUEX".to_string(),
                role: "Commando".to_string(),
                role_id: 1,
                judge: false,
                ready: false,
                present: true,
            }];
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        cx.update(|_, app| {
            view.update(app, |demo, cx| {
                demo.drop_unit(13, "KRI Ahmad Yani", 0.5, 0.5, Some(8), cx);
            })
        });
        match rx.try_recv().expect("the drop posted") {
            ToBinary::EmbarkAssign { host, child, commander_id } => {
                assert_eq!(host, 8);
                assert_eq!(child, 13);
                assert_eq!(commander_id, 7, "the operator's own seat");
            }
            other => panic!("expected an assign-and-embark, got {other:?}"),
        }
    }

    /// Click-to-place, the drag's fallback: the place button arms the
    /// drafted hull, and a map click that stays put deploys it there. The
    /// place button sits at the row's right end, so the sweep clicks across
    /// it until the arm latches.
    #[gpui::test]
    fn place_button_arms_and_map_click_deploys(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut session = SessionView::default();
            session.pieces = vec![PieceView {
                id: 13,
                label: "KRI Ahmad Yani".to_string(),
                commander_id: None,
                placed: false,
            }];
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, session, tx);
            demo.fleet_tax = super::fleet_taxonomy_tests::fixture_taxonomy();
            demo.select_category(2);
            demo.select_type(3);
            demo.select_class(5);
            demo.fleet_open = true;
            // A seeded status line: sweep clicks rewrite the picker's
            // verdict line, and a verdict that appears mid-sweep shifts
            // every row below the clicks still to come. One line in, one
            // line out — the geometry holds still for the whole sweep.
            demo.force_status = "sweep".to_string();
            demo
        });
        cx.simulate_resize(gpui::size(gpui::px(768.0), gpui::px(838.0)));
        let (w, h) = cx.update(|window, _| {
            let s = window.viewport_size();
            (f32::from(s.width), f32::from(s.height))
        });
        let lx = (w - 700.0) / 2.0 + 486.0;
        let (dx, dy) = (w - 68.0, h - 58.0);
        // Sweep the row's right end for the place button: the first click
        // that arms names the button's seat. Clicks elsewhere do nothing
        // (the row itself has no click), and clicks on add post force
        // plans — drained, because only the arm matters here. The sweep
        // stays inside the modal: clicks below it would land on the map
        // and deploy the armed hull, disarming the very latch sought. The
        // sweep runs to the content's right edge — the place button is the
        // row's last child, and a sweep that stops short of it (as the
        // first version did at 680) clicks everything but the verb.
        let modal_bottom = (h + 580.0) / 2.0;
        let mut armed = false;
        // Below the exercise section: its remove buttons share the sweep's
        // x band, and a click there would un-draft the very hull sought.
        // The drill columns start lower, and the seeded status above keeps
        // them put while the sweep walks.
        let mut y = 300.0;
        while y < modal_bottom - 20.0 && !armed {
            // The button's end of the row, in full: the place pill is the
            // row's last child and narrow, and a coarse step walks straight
            // past it — the first version proved exactly that at step 20.
            let mut x = 630.0;
            while x < 712.0 && !armed {
                cx.simulate_click(
                    gpui::point(gpui::px(x), gpui::px(y)),
                    gpui::Modifiers::default(),
                );
                while rx.try_recv().is_ok() {}
                armed = cx.update(|_, app| view.read(app).pending_place == Some(13));
                x += 6.0;
            }
            y += 6.0;
        }
        assert!(armed, "some click armed Yani for placing");
        while rx.try_recv().is_ok() {}
        cx.simulate_click(
            gpui::point(gpui::px(dx), gpui::px(dy)),
            gpui::Modifiers::default(),
        );
        match rx.try_recv().expect("the click reached the sink") {
            ToBinary::DeployUnit { unit_id, .. } => {
                assert_eq!(unit_id, 13, "the armed hull deploys");
            }
            other => panic!("expected a deploy, got {other:?}"),
        }
    }

    /// The hold toggle follows the last answer — held resumes, otherwise
    /// pauses, unknown pauses — and a 403 latches the grant instead of
    /// spending another POST. Anything else fails loudly and keeps the
    /// held state.
    #[gpui::test]
    fn pause_follows_the_last_answer(cx: &mut gpui::TestAppContext) {
        let clock = |running: bool| crate::backend::GameClock {
            state: "execution".to_string(),
            actual_start: None,
            assumed_start: None,
            assumed_now: Some("2026-10-14T10:30:00+07:00".to_string()),
            time_factor: 4.0,
            accepting_actions: running,
            running,
            segments: Vec::new(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            demo.session.game_id = 5;
            demo
        });
        // Unknown clock attempts pause — the console's rule, and the write
        // answers the clock.
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_set_paused(cx))),
            "unknown pauses"
        );
        match rx.try_recv().expect("the pause reached the sink") {
            ToBinary::SetPaused { paused } => assert!(paused, "unknown asks pause"),
            other => panic!("expected a pause, got {other:?}"),
        }
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_set_paused(cx))),
            "a hold in flight sends nothing"
        );
        // The held answer flips the next toggle to resume, with the
        // console's status sentence and the grant cleared.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.clock_denied = true;
                demo.apply_clock_answer(Ok(clock(false)));
                assert_eq!(demo.clock_running, Some(false));
                assert!(!demo.pause_busy, "the answer clears the latch");
                assert!(!demo.clock_denied, "an answer proves the grant");
                assert_eq!(
                    demo.pause_status,
                    "Scenario clock pause: assumed 2026-10-14T10:30:00+07:00 · held · orders closed · 4x"
                );
            })
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_set_paused(cx))),
            "held resumes"
        );
        match rx.try_recv().expect("the resume reached the sink") {
            ToBinary::SetPaused { paused } => assert!(!paused, "held asks resume"),
            other => panic!("expected a resume, got {other:?}"),
        }
        // A 403 latches the grant: the next attempt refuses locally with
        // the grant sentence and spends no POST.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.apply_clock_answer(Err(ClockError::Denied));
                assert!(demo.clock_denied, "the denial latches");
                assert!(demo.pause_status.contains("control grant"));
            })
        });
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_set_paused(cx))),
            "a denied verb sends nothing"
        );
        assert!(rx.try_recv().is_err(), "no request reached the sink");
        // Anything else fails loudly and keeps the held state for the next
        // toggle.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.clock_denied = false;
                demo.apply_clock_answer(Err(ClockError::Failed("clock pause refused: gone".to_string())));
                assert_eq!(demo.pause_status, "clock pause refused: gone");
                assert_eq!(demo.clock_running, Some(false), "the held state survives");
            })
        });
    }

    /// The embark modal opens on one hull, asks once, and files the answer
    /// under the answer's own host. A cached host asks nothing; a refusal
    /// lands on the modal's line and keeps the cache; a stale host never
    /// survives another modal opening.
    #[gpui::test]
    fn embark_opens_once_and_files_under_its_host(cx: &mut gpui::TestAppContext) {
        let aboard = || crate::backend::EmbarkedUnit {
            id: 5,
            name: "KRI Ahmad Yani".to_string(),
            hull_number: "KRI-351".to_string(),
            class_name: "Ahmad Yani / Van Speijk".to_string(),
            is_cleared: true,
            cleared_at: Some("2026-10-15T00:06:23+07:00".to_string()),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            demo.session.game_id = 5;
            demo.fleet_open = true;
            demo
        });
        // The open takes the whole modal zone: one modal at a time, and a
        // stale host cannot linger under another.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.open_embarked(8);
                assert_eq!(demo.embark_host, Some(8));
                assert!(!demo.fleet_open, "the open closes the others");
            })
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_embarked(cx))),
            "the open asks"
        );
        match rx.try_recv().expect("the read reached the sink") {
            ToBinary::ReadEmbarked { host } => assert_eq!(host, 8, "the modal's host"),
            other => panic!("expected a read, got {other:?}"),
        }
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_embarked(cx))),
            "a read in flight sends nothing"
        );
        // The answer files under its own host and ends the request.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.apply_embarked(Ok((8, vec![aboard()])));
                assert!(!demo.embark_busy, "the answer clears the latch");
                assert!(demo.embark_status.is_empty());
                assert_eq!(
                    demo.embarked.get(&8).map(|u| u.len()),
                    Some(1),
                    "filed under the answer's host"
                );
            })
        });
        // Cached: the second open asks nothing.
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_embarked(cx))),
            "a cached host sends nothing"
        );
        assert!(rx.try_recv().is_err(), "nothing reached the sink");
        // Another hull is another read, and an empty answer caches empty —
        // "nothing embarked", not another request.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.open_embarked(5);
            })
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_embarked(cx))),
            "an uncached host asks"
        );
        match rx.try_recv().expect("the read reached the sink") {
            ToBinary::ReadEmbarked { host } => assert_eq!(host, 5),
            other => panic!("expected a read, got {other:?}"),
        }
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.apply_embarked(Ok((5, Vec::new())));
                assert_eq!(
                    demo.embarked.get(&5).map(|u| u.len()),
                    Some(0),
                    "empty caches as empty"
                );
            })
        });
        // A refusal lands on the line and keeps whatever the cache held —
        // including nothing, which the modal reads as unasked, not as
        // empty. And opening any other modal clears the host.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.apply_embarked(Err("embarked failed: gone".to_string()));
                assert!(!demo.embark_busy);
                assert_eq!(demo.embark_status, "embarked failed: gone");
                assert!(!demo.embarked.contains_key(&9), "no entry invented");
                demo.open_only(WhichPanel::Fleet);
                assert_eq!(demo.embark_host, None, "no stale host under another modal");
            })
        });
    }

    /// The three row verbs carry one piece each, refuse the console's two
    /// ways in its sentences, and file the answer's contents plus its
    /// capacity warnings under the answer's host. A refusal keeps the
    /// cache and names itself on the line.
    #[gpui::test]
    fn embark_writes_carry_one_piece_each(cx: &mut gpui::TestAppContext) {
        let aboard = || crate::backend::EmbarkedUnit {
            id: 5,
            name: "KRI Ahmad Yani".to_string(),
            hull_number: "KRI-351".to_string(),
            class_name: "Ahmad Yani / Van Speijk".to_string(),
            is_cleared: true,
            cleared_at: Some("2026-10-15T00:06:23+07:00".to_string()),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(
                fixture_roster(),
                8,
                0,
                SessionView::default(),
                tx,
            );
            demo.session.game_id = 5;
            demo.embark_host = Some(8);
            demo.embarked.insert(8, vec![aboard()]);
            demo
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_unembark(8, 5, cx))),
            "unembark sends"
        );
        match rx.try_recv().expect("the unembark reached the sink") {
            ToBinary::UnembarkUnit { host, unit } => {
                assert_eq!((host, unit), (8, 5), "the row's own pair");
            }
            other => panic!("expected an unembark, got {other:?}"),
        }
        // The latch is shared: any second verb while one flies refuses in
        // the console's sentence.
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_clear_aboard(8, 5, cx))),
            "a write in flight sends nothing"
        );
        let status = cx.read(|app| view.read(app).embark_status.clone());
        assert_eq!(status, "clear refused: another request is in flight");
        assert!(rx.try_recv().is_err(), "nothing reached the sink");
        // The answer replaces the host's contents and stores the warning;
        // the rows going empty IS the success feedback, so the line clears.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.apply_embark_write(Ok((
                    8,
                    crate::backend::EmbarkationResult {
                        units: Vec::new(),
                        over_capacity: vec![crate::backend::OverCapacity {
                            level: "aircraft".to_string(),
                            id_level: 3,
                            max_aboard: 1,
                            would_hold: 2,
                        }],
                    },
                )));
                assert!(!demo.embark_busy, "the answer clears the latch");
                assert!(demo.embark_status.is_empty());
                assert_eq!(demo.embarked.get(&8).map(|u| u.len()), Some(0));
                assert_eq!(
                    demo.embark_over.get(&8).map(|o| o.len()),
                    Some(1),
                    "the warning files with the host"
                );
            })
        });
        // No held game refuses before any request, in the console's words.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.session.game_id = 0;
            })
        });
        assert!(
            !cx.update(|_window, app| view.update(app, |demo, cx| demo.send_revoke_clearance(8, 5, cx))),
            "no game sends nothing"
        );
        let status = cx.read(|app| view.read(app).embark_status.clone());
        assert_eq!(status, "revoke refused: no held game");
        // A refusal keeps the cache and names itself on the line.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.session.game_id = 5;
                demo.embarked.insert(8, vec![aboard()]);
                demo.apply_embark_write(Err("revoke failed: 403".to_string()));
                assert!(!demo.embark_busy);
                assert_eq!(demo.embark_status, "revoke failed: 403");
                assert_eq!(
                    demo.embarked.get(&8).map(|u| u.len()),
                    Some(1),
                    "the held row survives"
                );
            })
        });
        // And the clear verb names its own pair on the wire.
        assert!(
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_clear_aboard(8, 5, cx))),
            "clear sends"
        );
        match rx.try_recv().expect("the clear reached the sink") {
            ToBinary::ClearAboard { host, unit } => assert_eq!((host, unit), (8, 5)),
            other => panic!("expected a clear, got {other:?}"),
        }
    }

    /// The helm orders the PICKED hull, not the first one.
    ///
    /// This is the test the previous behaviour would have failed: `send_helm`
    /// aimed at `mine.first()` and the panel never said which, so an operator
    /// holding two hulls could not tell whose hull a posted order belonged to
    /// \u{2014} and could not choose. The console's dashboard has a ship picker
    /// for exactly this.
    #[gpui::test]
    fn the_helm_orders_the_picked_hull(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: true,
            my_role: "Commando".to_string(),
            mine: vec![
                MyShip { id: "8".to_string(), label: "KRI First (KRI-1)".to_string() },
                MyShip { id: "9".to_string(), label: "KRI Second (KRI-2)".to_string() },
            ],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, session, tx);
            demo.helm_heading = Some(90.0);
            demo.helm_speed = Some(12.0);
            demo
        });
        // Unpicked: the first hull, which is the console's own default and a
        // helm with no target would be a control that silently does nothing.
        assert!(
            cx.update(|_window, app| view.update(app, |demo, _| demo.send_helm())),
            "the order sends unpicked"
        );
        match rx.try_recv().expect("the first order reached the sink") {
            ToBinary::Helm(o) => assert_eq!(o.unit_id, "8", "the default is the first hull"),
            other => panic!("expected a helm order, got {other:?}"),
        }
        // Picked: the SECOND hull, and the order carries its id.
        cx.update(|_window, app| {
            view.update(app, |demo, _| demo.helm_target = Some("9".to_string()))
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, _| demo.send_helm())),
            "the order sends picked"
        );
        match rx.try_recv().expect("the second order reached the sink") {
            ToBinary::Helm(o) => assert_eq!(o.unit_id, "9", "the picked hull is ordered"),
            other => panic!("expected a helm order, got {other:?}"),
        }
    }

    /// A picked hull that is no longer commanded stops being orderable.
    ///
    /// The roster changes under the panel, and a remembered id that no longer
    /// belongs to this seat would order a hull the operator cannot see \u{2014}
    /// so the target is resolved against the LIVE roster, not trusted.
    #[gpui::test]
    fn a_reassigned_hull_is_no_longer_ordered(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: true,
            my_role: "Commando".to_string(),
            mine: vec![MyShip { id: "8".to_string(), label: "KRI First".to_string() }],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, session, tx);
            demo.helm_heading = Some(90.0);
            demo.helm_speed = Some(12.0);
            demo.helm_target = Some("99".to_string());
            demo
        });
        assert!(
            cx.update(|_window, app| view.update(app, |demo, _| demo.send_helm())),
            "the order still sends, on a hull this seat holds"
        );
        match rx.try_recv().expect("the order reached the sink") {
            ToBinary::Helm(o) => assert_eq!(o.unit_id, "8", "the stale pick falls back"),
            other => panic!("expected a helm order, got {other:?}"),
        }
    }

    /// A player declares and withdraws: each direction sends once and latches
    /// busy until the re-read session answers.
    #[gpui::test]
    fn readiness_declares_and_withdraws(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session_with_seat(false, false), tx)
        });
        let sent = cx.update(|_window, app| view.update(app, |demo, cx| demo.set_readiness(true, cx)));
        assert!(sent, "a player declares");
        match rx.try_recv().expect("the declaration reached the sink") {
            ToBinary::SetReadiness { ready } => assert!(ready),
            other => panic!("expected a readiness write, got {other:?}"),
        }
        // Busy: a second ask waits for the answer, whatever it says.
        let again = cx.update(|_window, app| view.update(app, |demo, cx| demo.set_readiness(true, cx)));
        assert!(!again, "an in-flight declaration is not re-sent");
        // Answered ready: withdrawing sends the other direction.
        cx.update(|_window, app| {
            view.update(app, |demo, _| {
                demo.readiness_busy = false;
                demo.session.people[0].ready = true;
            })
        });
        let out = cx.update(|_window, app| view.update(app, |demo, cx| demo.set_readiness(false, cx)));
        assert!(out, "a ready player withdraws");
        match rx.try_recv().expect("the withdrawal reached the sink") {
            ToBinary::SetReadiness { ready } => assert!(!ready),
            other => panic!("expected a readiness write, got {other:?}"),
        }
    }

    /// Judges are exempt and the seatless have nothing to declare: both
    /// directions stay silent, in either state.
    #[gpui::test]
    fn readiness_is_silent_for_judges_and_the_seatless(cx: &mut gpui::TestAppContext) {
        for (judge, ready) in [(true, false), (true, true)] {
            let (tx, rx) = std::sync::mpsc::channel();
            let (view, cx) = cx.add_window_view(|_, _| {
                IslandDemo::new(fixture_roster(), 8, 0, session_with_seat(judge, ready), tx)
            });
            for want in [true, false] {
                let sent = cx.update(|_window, app| {
                    view.update(app, |demo, cx| demo.set_readiness(want, cx))
                });
                assert!(!sent, "a judge seat never sends (want {want})");
            }
            assert!(rx.try_recv().is_err(), "nothing reached the sink");
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx)
        });
        let sent = cx.update(|_window, app| view.update(app, |demo, cx| demo.set_readiness(true, cx)));
        assert!(!sent, "no seat sends nothing");
        assert!(rx.try_recv().is_err(), "nothing reached the sink");
    }

    /// Asking for the state already held sends nothing: declaring while
    /// ready (or withdrawing while not) is a no-op, not a round trip.
    #[gpui::test]
    fn readiness_asks_for_nothing_already_held(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session_with_seat(false, true), tx)
        });
        let sent = cx.update(|_window, app| view.update(app, |demo, cx| demo.set_readiness(true, cx)));
        assert!(!sent, "declaring while ready is a no-op");
        assert!(rx.try_recv().is_err(), "nothing reached the sink");
    }

    /// Applying the answer holds the re-read session and stays open: a
    /// readiness change is not a new hold, so the panel does not close.
    #[gpui::test]
    fn applying_a_readiness_answer_holds_the_session(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo =
                IslandDemo::new(fixture_roster(), 8, 0, session_with_seat(false, false), tx);
            demo.command_open = true;
            demo.readiness_busy = true;
            demo
        });
        let mut held = session_with_seat(false, true);
        held.game = "G10".to_string();
        cx.update(|_window, app| view.update(app, |demo, _| demo.apply_readiness(held)));
        let (open, ready, busy) = cx.update(|_window, app| {
            view.update(app, |demo, _| {
                (demo.command_open, demo.session.my_ready, demo.readiness_busy)
            })
        });
        assert!(open, "the panel stays open across the answer");
        assert!(ready, "the re-read readiness is held");
        assert!(!busy, "the latch clears for the next ask");
    }

    /// A held session with a related step for the relate tests: scenario 4,
    /// step 7, obliged to hull 8 — the probe's own shape, read back.
    #[cfg(test)]
    fn session_with_related() -> SessionView {
        SessionView {
            book: vec![BookScenario {
                id: 4,
                title: "Scenario 1".to_string(),
                steps: vec![BookStep {
                    id: 7,
                    content: "Cross the river".to_string(),
                    window: None,
                }],
            }],
            pieces: vec![
                PieceView { id: 8, label: "KRI Abdul Halim Perdanakusuma (KRI-355)".to_string(), commander_id: Some(2), placed: true },
                PieceView { id: 9, label: "KRI Yos Sudarso (353)".to_string(), commander_id: None, placed: false },
            ],
            related: vec![StepRelatedView {
                scenario_id: 4,
                step_id: 7,
                posts: vec![("hull".to_string(), 8)],
                personnel: vec!["SUPERUSER".to_string()],
            }],
            ..SessionView::default()
        }
    }

    /// Opening the editor seeds the working set from the server's read —
    /// there is no staging here, so the working set IS the last saved list
    /// until the author touches it. Toggling again closes.
    #[gpui::test]
    fn relate_opens_seeded_from_the_read(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session_with_related(), tx)
        });
        cx.update(|_window, app| view.update(app, |demo, cx| demo.toggle_relate(4, 7, cx)));
        let (open, working) = cx.update(|_window, app| {
            view.update(app, |demo, _| (demo.relate_open, demo.relate_working.clone()))
        });
        assert_eq!(open, Some((4, 7)));
        assert_eq!(working, vec![("hull".to_string(), 8)]);
        cx.update(|_window, app| view.update(app, |demo, cx| demo.toggle_relate(4, 7, cx)));
        let open = cx.update(|_window, app| view.update(app, |demo, _| demo.relate_open));
        assert_eq!(open, None, "toggling again closes");
    }

    /// The checkbox verb flips one ref and leaves the rest: check on, check
    /// off, and a second hull alongside the first.
    #[gpui::test]
    fn relate_working_flips_one_ref(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session_with_related(), tx)
        });
        let working = cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.toggle_relate(4, 7, cx);
                demo.toggle_relate_item("hull", 9);
                demo.toggle_relate_item("hull", 8);
                demo.relate_working.clone()
            })
        });
        assert_eq!(working, vec![("hull".to_string(), 9)]);
    }

    /// Saving posts the working set whole — even empty, which is legal and
    /// clears the list. Nothing open, or an answer still in flight, sends
    /// nothing.
    #[gpui::test]
    fn relate_saves_the_working_set_whole(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session_with_related(), tx)
        });
        let shut = cx.update(|_window, app| view.update(app, |demo, cx| demo.save_relate(cx)));
        assert!(!shut, "nothing open saves nothing");
        cx.update(|_window, app| view.update(app, |demo, cx| demo.toggle_relate(4, 7, cx)));
        let sent = cx.update(|_window, app| view.update(app, |demo, cx| demo.save_relate(cx)));
        assert!(sent, "an open editor saves");
        match rx.try_recv().expect("the list reached the sink") {
            ToBinary::SetRelated { scenario_id, step_id, related } => {
                assert_eq!((scenario_id, step_id), (4, 7));
                assert_eq!(related, vec![("hull".to_string(), 8)]);
            }
            other => panic!("expected a save, got {other:?}"),
        }
        let again = cx.update(|_window, app| view.update(app, |demo, cx| demo.save_relate(cx)));
        assert!(!again, "an in-flight save is not re-sent");
        assert!(rx.try_recv().is_err(), "nothing else was sent");
    }

    /// Applying the answer re-seeds the working set from what the server
    /// resolved — the redraw source, not the echo — and stays open.
    #[gpui::test]
    fn applying_a_relate_answer_reseeds_and_stays_open(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo =
                IslandDemo::new(fixture_roster(), 8, 0, session_with_related(), tx);
            demo.relate_open = Some((4, 7));
            demo.relate_working = vec![("hull".to_string(), 9)];
            demo.relate_busy = true;
            demo
        });
        let mut held = session_with_related();
        held.related[0].posts = vec![("hull".to_string(), 9)];
        held.related[0].personnel = vec!["SUPERUSER".to_string()];
        cx.update(|_window, app| view.update(app, |demo, _| demo.apply_relate(held)));
        let (open, working, busy) = cx.update(|_window, app| {
            view.update(app, |demo, _| {
                (demo.relate_open, demo.relate_working.clone(), demo.relate_busy)
            })
        });
        assert_eq!(open, Some((4, 7)), "the editor stays open across the answer");
        assert_eq!(working, vec![("hull".to_string(), 9)], "the working set is the read");
        assert!(!busy, "the latch clears for the next save");
    }
    /// And it is SILENT outside execution — the rule the server enforces with
    /// a 409, checked before the write rather than after the refusal.
    #[gpui::test]
    fn the_helm_is_silent_outside_execution(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: false, // preparation
            my_role: "Commando".to_string(),
            mine: vec![MyShip {
                id: "8".to_string(),
                label: "KRI Test (KRI-1)".to_string(),
            }],
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, session, tx);
            demo.helm_heading = Some(90.0);
            demo.helm_speed = Some(12.0);
            demo
        });
        let sent = cx.update(|_window, app| view.update(app, |demo, _| demo.send_helm()));
        assert!(!sent, "preparation sends nothing");
        assert!(rx.try_recv().is_err(), "nothing reached the sink");
    }

    /// The composer row is the shell's second WRITE, so its contract is pinned
    /// here like the helm's: the drafted content goes out as ONE telegram
    /// broadcast, and it is the same `send_telegram` the SEND button calls —
    /// never a copy.
    #[gpui::test]
    fn the_composer_sends_a_telegram_broadcast(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: true,
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, session, tx);
            demo.msg_content = "telegram proof from the shell".to_string();
            demo
        });
        let sent = cx.update(|_window, app| view.update(app, |demo, cx| demo.send_telegram(cx)));
        assert!(sent, "an executing game with a drafted line sends");
        let content = match rx.try_recv().expect("the telegram reached the sink") {
            ToBinary::SendTelegram { content } => content,
            other => panic!("expected a telegram, got {other:?}"),
        };
        assert_eq!(content, "telegram proof from the shell");
    }

    /// And it is SILENT outside execution — the rule the server enforces,
    /// checked before the write rather than after the refusal — and silent
    /// on a blank row in execution, in the console's own sentence.
    #[gpui::test]
    fn the_composer_is_silent_until_execution_and_content(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: false, // preparation
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            let mut demo = IslandDemo::new(fixture_roster(), 8, 0, session, tx);
            demo.msg_content = "too early".to_string();
            demo
        });
        let sent = cx.update(|_window, app| view.update(app, |demo, cx| demo.send_telegram(cx)));
        assert!(!sent, "preparation sends nothing");
        assert!(rx.try_recv().is_err(), "nothing reached the sink");

        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: true,
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        let sent = cx.update(|_window, app| view.update(app, |demo, cx| demo.send_telegram(cx)));
        assert!(!sent, "a blank row sends nothing");
        assert!(rx.try_recv().is_err(), "nothing reached the sink");
    }

    /// The asking row declares on the step it drew: the ids ride the button,
    /// and it is the same `send_step_declare` the click calls — never a copy.
    #[gpui::test]
    fn the_step_row_declares_for_the_asking_step(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: true,
            step: Some(StepView {
                assumed_time: "2000-01-01T10:00:00+07:00".to_string(),
                playing: None,
                asking: Some(StepAsk {
                    scenario_id: 12,
                    step_id: 16,
                    scenario: "First move".to_string(),
                    content: "Take station east of the buoy".to_string(),
                    required: 1,
                    outstanding: 1,
                    you_ready: false,
                }),
            }),
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        let sent =
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_step_declare(12, 16, true, cx)));
        assert!(sent, "the asking step declares");
        match rx.try_recv().expect("the declaration reached the sink") {
            ToBinary::DeclareStep { scenario_id, step_id, ready } => {
                assert_eq!((scenario_id, step_id, ready), (12, 16, true));
            }
            other => panic!("expected a declaration, got {other:?}"),
        }
    }

    /// And it sends nothing when no step asks, or when the named step is not
    /// the asking one — a stale click after the re-read moved on.
    #[gpui::test]
    fn the_step_row_is_silent_without_an_asking_step(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, SessionView::default(), tx)
        });
        let sent =
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_step_declare(12, 16, true, cx)));
        assert!(!sent, "no asking step sends nothing");
        assert!(rx.try_recv().is_err(), "nothing reached the sink");

        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: true,
            step: Some(StepView {
                assumed_time: String::new(),
                playing: None,
                asking: Some(StepAsk {
                    scenario_id: 12,
                    step_id: 16,
                    scenario: "First move".to_string(),
                    content: String::new(),
                    required: 1,
                    outstanding: 1,
                    you_ready: true,
                }),
            }),
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        let sent =
            cx.update(|_window, app| view.update(app, |demo, cx| demo.send_step_declare(12, 99, true, cx)));
        assert!(!sent, "a stale step sends nothing");
        assert!(rx.try_recv().is_err(), "nothing reached the sink");
    }

    /// The reminder strip names the ONE asking step — scenario, instruction
    /// and counts in one line — with the indicator lit while it waits on
    /// this caller. Pure: the strip draws this, so the test reads it
    /// without rendering.
    #[gpui::test]
    fn the_reminder_names_the_asking_step(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: true,
            step: Some(StepView {
                assumed_time: String::new(),
                playing: None,
                asking: Some(StepAsk {
                    scenario_id: 12,
                    step_id: 16,
                    scenario: "First move".to_string(),
                    content: "Take station east of the buoy".to_string(),
                    required: 1,
                    outstanding: 1,
                    you_ready: false,
                }),
            }),
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        let rem = cx.update(|_window, app| view.read(app).step_reminder());
        let rem = rem.expect("an asking step reminds");
        assert!(rem.needs_you, "the outstanding step waits on this caller");
        assert!(rem.text.contains("First move"), "the scenario rides along");
        assert!(rem.text.contains("Take station east of the buoy"), "the instruction rides along");
        assert!(rem.text.contains("0 of 1 ready"), "the counts ride along");
    }

    /// Declared means the indicator rests; asking beats playing; playing is
    /// the fallback; silence otherwise — no execution, no step, nothing.
    #[gpui::test]
    fn the_reminder_rests_once_declared_then_falls_back(cx: &mut gpui::TestAppContext) {
        let asking = |you_ready: bool| {
            Some(StepView {
                assumed_time: String::new(),
                playing: Some(StepLine {
                    scenario_id: 12,
                    step_id: 15,
                    scenario: "First move".to_string(),
                    content: "Old instruction".to_string(),
                    state: "playing".to_string(),
                }),
                asking: Some(StepAsk {
                    scenario_id: 12,
                    step_id: 16,
                    scenario: "First move".to_string(),
                    content: "Take station east of the buoy".to_string(),
                    required: 1,
                    outstanding: 0,
                    you_ready,
                }),
            })
        };
        let (tx, _rx) = std::sync::mpsc::channel();
        let session = SessionView { execution: true, step: asking(true), ..SessionView::default() };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        let rem = cx.update(|_window, app| view.read(app).step_reminder());
        let rem = rem.expect("a declared step still reminds");
        assert!(!rem.needs_you, "declared rests the indicator");
        assert!(rem.text.contains("Take station"), "asking beats playing, even declared");

        let (tx, _rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: true,
            step: Some(StepView {
                assumed_time: String::new(),
                playing: Some(StepLine {
                    scenario_id: 12,
                    step_id: 15,
                    scenario: "First move".to_string(),
                    content: "Old instruction".to_string(),
                    state: "playing".to_string(),
                }),
                asking: None,
            }),
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        let rem = cx.update(|_window, app| view.read(app).step_reminder());
        let rem = rem.expect("a playing step reminds");
        assert!(!rem.needs_you, "naming play never waits");
        assert!(rem.text.starts_with("NOW"), "the fallback names what is on");

        let (tx, _rx) = std::sync::mpsc::channel();
        let session = SessionView { execution: true, ..SessionView::default() };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        assert!(
            cx.update(|_window, app| view.read(app).step_reminder()).is_none(),
            "no step means no strip"
        );

        let (tx, _rx) = std::sync::mpsc::channel();
        let session = SessionView { execution: false, step: asking(false), ..SessionView::default() };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        assert!(
            cx.update(|_window, app| view.read(app).step_reminder()).is_none(),
            "outside execution the strip stays off"
        );
    }

    /// Opening from the strip is the step modal itself — the scenario, the
    /// step and the declare row already live there, so the strip grows no
    /// panel of its own. The click calls exactly this, and the half worth
    /// pinning is the REVEAL: `reveal` is consumed by the very next frame
    /// (`render`'s `reveal.take()`), so a test that reads the field after a
    /// frame asserts on a value the frame is supposed to eat. It passed for a
    /// while only because a sibling test's `SHEET` global made `render` return
    /// early, before the column ran at all — the mask, not the behaviour. This
    /// asserts the outcome instead: the island is on the band.
    #[gpui::test]
    fn opening_the_reminder_opens_the_step_modal(cx: &mut gpui::TestAppContext) {
        let (tx, _rx) = std::sync::mpsc::channel();
        let session = SessionView {
            execution: true,
            state: "execution".to_string(),
            step: Some(StepView {
                assumed_time: String::new(),
                playing: None,
                asking: Some(StepAsk {
                    scenario_id: 12,
                    step_id: 16,
                    scenario: "First move".to_string(),
                    content: "Take station east of the buoy".to_string(),
                    required: 1,
                    outstanding: 1,
                    you_ready: false,
                }),
            }),
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        // A window where the column actually overflows, or the reveal is
        // clamped to zero and proves nothing.
        cx.simulate_resize(gpui::size(gpui::px(768.0), gpui::px(838.0)));
        let tick = |cx: &mut gpui::VisualTestContext| {
            cx.simulate_mouse_move(
                gpui::point(gpui::px(5.0), gpui::px(5.0)),
                None,
                gpui::Modifiers::default(),
            )
        };
        tick(cx);
        // Shut, the way an operator who closed it would have left it.
        cx.update(|_window, app| {
            view.update(app, |demo, _| demo.set_island_open(view::IslandKey::Step, false))
        });
        assert!(
            !cx.update(|_window, app| view.read(app).step_open),
            "the island starts shut"
        );
        // The strip's click.
        cx.update(|_window, app| view.update(app, |demo, cx| demo.open_step_detail(cx)));
        assert!(
            cx.update(|_window, app| view.read(app).step_open),
            "the click opens the island"
        );
        // And the next frame honours the reveal.
        tick(cx);
        let (sc, want) = cx.update(|window, app| {
            let d = view.read(app);
            let band = zone_band_at(viewport(window), d.zone_w, d.side_dock_left);
            let slots = d.column_slots();
            let i = slots
                .iter()
                .position(|s| s.key == view::IslandKey::Step)
                .expect("the step island is in the column");
            let (_, y) = view::zone_slot_origin(band, &slots, i);
            (d.zone_scroll, y - band.1)
        });
        assert!(want > 0.0, "the step island starts below the band");
        assert_eq!(sc, want, "and the click scrolled it onto the band");
    }

    /// The sign-in card is the one panel that must exist before any other, and
    /// its fields are HAND-ROLLED text inputs — GPUI 0.2.2 ships no text field,
    /// only the input plumbing — so the typing contract is pinned here: a
    /// printable character appends, a backspace removes, and Enter submits what
    /// was typed as ONE message on the sink.
    #[gpui::test]
    fn the_sign_in_card_collects_what_is_typed(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            signed_in: false,
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        fn press(key: &str, ch: Option<&str>) -> gpui::Keystroke {
            gpui::Keystroke {
                modifiers: gpui::Modifiers::default(),
                key: key.to_string(),
                key_char: ch.map(str::to_string),
            }
        }
        fn type_into(
            which: Cred,
            text: &str,
            demo: &mut IslandDemo,
            window: &mut Window,
            cx: &mut Context<IslandDemo>,
        ) {
            for c in text.chars() {
                let k = press(&c.to_string(), Some(&c.to_string()));
                demo.login_key(which, &k, window, cx);
            }
        }
        cx.update(|window, app| {
            view.update(app, |demo, cx| {
                type_into(Cred::Identifier, "superr", demo, window, cx);
                // A backspace then a retype: the field must not keep both.
                demo.login_key(Cred::Identifier, &press("backspace", None), window, cx);
                type_into(Cred::Identifier, "r", demo, window, cx);
                type_into(Cred::Password, "pw", demo, window, cx);
                demo.login_key(Cred::Identifier, &press("enter", None), window, cx);
            })
        });
        match rx.try_recv().expect("the credentials reached the sink") {
            ToBinary::SignIn {
                identifier,
                password,
            } => {
                assert_eq!(identifier, "superr");
                assert_eq!(password, "pw");
            }
            other => panic!("expected a sign-in, got {other:?}"),
        }
    }

    /// And a card with a missing half submits NOTHING and says why: the shell
    /// validates before it spends a round trip, so a half-typed credential is a
    /// sentence rather than a refusal from the server.
    #[gpui::test]
    fn the_sign_in_card_refuses_a_half_credential(cx: &mut gpui::TestAppContext) {
        let (tx, rx) = std::sync::mpsc::channel();
        let session = SessionView {
            signed_in: false,
            ..SessionView::default()
        };
        let (view, cx) = cx.add_window_view(|_, _| {
            IslandDemo::new(fixture_roster(), 8, 0, session, tx)
        });
        let status = cx.update(|_window, app| {
            view.update(app, |demo, cx| {
                demo.login_identifier = "superuser".to_string();
                demo.login_password.clear();
                demo.submit_login(cx);
                demo.login_status.clone()
            })
        });
        assert!(!status.is_empty(), "the card says what is missing");
        assert!(rx.try_recv().is_err(), "nothing was sent");
    }
}
