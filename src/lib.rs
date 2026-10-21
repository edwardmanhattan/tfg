//! Presentation-mode ship map client (library root).
//!
//! - [`geo`]: Fix / Ship / Track / Trail model + smoothing (see CONTEXT.md).
//! - [`gpuprobe`]: `--probe-gpu`, the graphics capability check.
//! - [`fx`]: the shader seam — additive halos, quality tiers.
//! - [`camera`]: the eased camera move on an operator recentre.
//! - [`chrome`]: island and zone chrome — the floating-panel shell (see ADR-0014, ADR-0016).
//! - [`tokens`]: the palette and the measurements. Leaf: egui only (see ADR-0016).
//! - [`view`]: the palette, metrics and chrome geometry with no toolkit type
//!   in them. This is the seam the shell migration turns on: egui and GPUI
//!   both read it, and the chrome geometry is testable with no display.
//! - [`gamestate`]: the exercise lifecycle as one axis. Leaf: no tfg types.
//! - [`mode_rules`]: the console's own rules about a mode — which verbs each
//!   mode earns, and why they are not the server's. Leaf: no tfg types.
//! - [`gamefeed`]: the authoritative movement of a Minos exercise — the
//!   plot read, the order publications, and the game fixes they become.
//!   Leaf: no GPUI, no shell.
//! - [`advance`]: the stage advance as a plan — the order the server takes
//!   the writes in, and the rules that refuse it whole.
//! - [`force`]: the operator's force before Minos hears of it, and the diff a stage advance writes.
//! - [`embark`]: the embark rule — what may go aboard what, decided against
//!   the spec the server published. Leaf: no GPUI, no draft, no network.
//! - [`dnd`]: entity-to-entity drag and drop — what a drop on a hull, a person
//!   or a group means. Leaf: no GPUI, no draft, no network.
//! - [`backend`]: v0 poll contract: `PollSource`, file replay, HTTP stub.
//! - [`symbology`]: APP-6C symbol model + generated icon geometry. Leaf: depends on nothing.
//! - [`shell_gpui`]: the GPUI shell — the island chrome and the U5 parity
//!   layers, painted from `view`. The cutover's second binary
//!   (`src/bin/tfg_gpui.rs`) and `proto/gpui-island` both drive this one file.
//! - [`markers`]: the map's marker model — one hull on the map, its
//!   affiliation vocabulary and the pick order. Toolkit-free; the painters
//!   stay per-shell (U8 step 4).
//! - [`map_geometry`]: the map's arithmetic — the Web Mercator projection,
//!   the ground resolution, and where the map's picture goes in the window.
//!   No MapLibre, so the parity proto can compile it.
//! - [`map_gate`]: the map frame handoff — newest-wins queue, hold-until-ready
//!   promotion, and the retirement that keeps the atlas bounded. GPUI-typed by
//!   design (U6); this is the file U8 moved out of `proto/gpui-map` so the
//!   shell and the spike drive one implementation.
//! - [`shell_threads`]: poll/map spawns shared by both shells. Toolkit-free;
//!   the map spawn follows in U6 when the frame handoff drops egui.
//! - [`picker`]: the Fleet picker's geometry — panel sizing, the column
//!   widths and the drop rule. Toolkit-free; the first test cluster U8 moved
//!   off `egui::Context::default()`.
//! - [`readiness`]: the execution gate's unknown states — why the last read
//!   produced no verdict, and what to say while it has not. Toolkit-free.
//! - [`window`]: the exercise's planned window and the one rule that says
//!   what a window is — shared with the scenario composer. Toolkit-free.
//! - [`calendar`]: the month as six weeks of days, and the local datetimes a
//!   scenario's window is written in. Toolkit-free; the console draws it.
//! - [`visuals`]: every hull's picture state — the versioned visual cache and
//!   what still has to be fetched. The decoded texture is a shell-minted
//!   handle here, not a toolkit type.
//!
//! Module list is alphabetical after `assets`.

pub mod advance;
pub mod assets;
pub mod backend;
pub mod book;
pub mod calendar;
pub mod catalog;
pub mod clock;
pub mod command;
pub mod dnd;
pub mod embark;
pub mod fleet;
pub mod force;
pub mod gamestate;
pub mod gamefeed;
pub mod gpuprobe;
pub mod groups;
pub mod halo;
pub mod geo;
pub mod land;
pub mod library;
pub mod log;
pub mod map_gate;
pub mod map_geometry;
pub mod map_render;
pub mod markers;
pub mod mode_rules;
pub mod overlay;
pub mod paths;
pub mod phase;
pub mod readiness;
pub mod roster;
pub mod sfx;
pub mod selfcheck;
pub mod shell_gpui;
pub mod shell_threads;
pub mod sim;
pub mod store;
pub mod symbology;
pub mod view;
pub mod visuals;
pub mod window;
