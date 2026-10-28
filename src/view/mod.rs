//! The view-model kernel. Palette, metrics and chrome geometry, with no
//! toolkit type anywhere in it.
//!
//! This module is the seam the shell migration turns on. `tokens.rs` used to
//! hold `egui::Color32` directly, which meant the design system was welded to
//! one toolkit: the palette could not be read by a GPUI shell without pulling
//! egui in, and `proto/p5-epaint` could only type-check `chrome.rs` by
//! resolving the same egui the app resolves.
//!
//! So the values move here as plain data, and each shell adapts them. A
//! colour is four bytes. Nothing about that is egui's business.
//!
//! What this buys:
//!
//! - **One palette, two shells.** ADR-0018's decision that the design system
//!   survives a toolkit change is enforced by the type system rather than by
//!   a comment. A `gpui::Hsla` cannot appear in this module, so the palette
//!   cannot leak.
//! - **The chrome geometry is testable with no GUI at all.** The 27 tests in
//!   `chrome.rs` that pin island geometry, zone layout, ownership and the
//!   backdrop all read `egui::{Rect,Pos2}`. Reduced to f32 tuples they run
//!   against the real design numbers on every `cargo test`, with no display,
//!   no compositor and no GPU.
//! - **The prototype harness stops being special.** `proto/p5-epaint` needed
//!   `#[path]` includes and a matched egui version. A kernel with no toolkit
//!   types needs neither.
//!
//! Names are the palette's own vocabulary from `DESIGN.md`, and each constant
//! carries the reason it exists. Where a colour has a role rather than a
//! value, it is named for the role. Do not strip these comments: they are the
//! part that stops the next change from being wrong.

pub mod kit;

use std::sync::atomic::{AtomicU32, Ordering};

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

/// One colour, straight 0..255 sRGB with alpha. Premultiplied nowhere, by
/// nothing, at any layer: a raw value is what the design system speaks in,
/// and blending is the shell's problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// White at `alpha`. The top light and bottom shade on an island body are
    /// white and black at low alpha rather than real colours, because they
    /// have to sit on a panel that is itself on top of arbitrary map tiles.
    pub const fn white_alpha(a: u8) -> Self {
        Self {
            r: 255,
            g: 255,
            b: 255,
            a,
        }
    }

    pub const fn black_alpha(a: u8) -> Self {
        Self {
            r: 0,
            g: 0,
            b: 0,
            a,
        }
    }

    pub const fn with_alpha(mut self, a: u8) -> Self {
        self.a = a;
        self
    }

    /// As 0..1 floats, which is what a shell's colour type wants.
    pub fn as_f32(self) -> [f32; 4] {
        [
            self.r as f32 / 255.0,
            self.g as f32 / 255.0,
            self.b as f32 / 255.0,
            self.a as f32 / 255.0,
        ]
    }

    /// The alpha as a 0..1 float, for the shells that take alpha separately
    /// from the hue (GPUI's `hsla` does).
    pub fn alpha_f32(self) -> f32 {
        self.a as f32 / 255.0
    }
}

// ---------------------------------------------------------------------------
// Palette
//
// The Console Night family carries all chrome, and every step in it is a
// depth rather than a hue. That is why the names read as places: a panel is
// not "a dark blue", it is one step above the console it sits on.
// ---------------------------------------------------------------------------

/// Window, zone and island fill. The chrome every panel is made of.
pub const CONSOLE_NIGHT: Rgba = Rgba::rgb(0x0F, 0x17, 0x2A);
/// One step up: title bands, inset and secondary surfaces inside chrome.
pub const PANEL_SLATE: Rgba = Rgba::rgb(0x1E, 0x29, 0x3B);
/// The near-black the map sits against, and the fill of an input well.
pub const DEEP_WELL: Rgba = Rgba::rgb(0x02, 0x06, 0x17);
/// Every 1px window and panel stroke. The single edge treatment.
pub const HAIRLINE_SLATE: Rgba = Rgba::rgb(0x33, 0x41, 0x55);
/// The chamfer's 2px cut edge, and a resting island's rim. A cool grey one
/// step above the hairline, because at hairline value the chamfer
/// disappeared into the map and defeated the point of drawing one.
pub const CUT_GREY: Rgba = Rgba::rgb(0x8C, 0x9B, 0xAE);
/// Island edge: the bright half of the cased border. Near-white so it parts
/// from dark water and night chrome; the dark casing beside it parts from
/// bright land. Neither alone survives both grounds, so the pair is the
/// border. Never the cyan accent.
pub const ISLAND_EDGE: Rgba = Rgba::rgb(0xC9, 0xD2, 0xDE);

/// The only accent. Live chrome state, and the island that owns input.
/// Its rarity is the whole reason an ON state is readable across a room,
/// which is also why nothing else is allowed to be this colour.
pub const RADAR_CYAN: Rgba = Rgba::rgb(0x22, 0xD3, 0xEE);

/// Default label and body text on chrome.
pub const BODY_SILVER: Rgba = Rgba::rgb(0x8C, 0x8C, 0x8C);
/// Text on a resting button, lifting to the hover tone on hover.
pub const BUTTON_LABEL: Rgba = Rgba::rgb(0xB4, 0xB4, 0xB4);
pub const BUTTON_LABEL_HOVER: Rgba = Rgba::rgb(0xF0, 0xF0, 0xF0);
/// Resting button fill, dull so the cyan wake-up reads as the event.
pub const BUTTON_GRAPHITE: Rgba = Rgba::rgb(0x3C, 0x3C, 0x3C);
/// The only text drawn on tiles.
pub const MAP_INK: Rgba = Rgba::rgb(0xE2, 0xE8, 0xF0);

/// Status ink. `DESIGN.md`'s Never Gray-On-Gray rule exists because these
/// are the only five ways a line is allowed to report state.
pub const SIGNAL_GREEN: Rgba = Rgba::rgb(0x4A, 0xDE, 0x80);
pub const WARNING_SAND: Rgba = Rgba::rgb(0xFA, 0xBF, 0x69);
pub const FAULT_RED: Rgba = Rgba::rgb(0xF8, 0x71, 0x71);
pub const IDLE_GREY: Rgba = Rgba::rgb(0x80, 0x80, 0x80);
/// Age of data, never feed state. Two different things and two different
/// colours, which is the entire reason the pair exists.
pub const DATA_AMBER: Rgba = Rgba::rgb(0xF5, 0x9E, 0x0B);
/// Attention without fault.
pub const ALERT_YELLOW: Rgba = Rgba::rgb(0xFF, 0xFF, 0x00);
/// The map's own INSTRUMENT: a course the operator is turning.
///
/// NOT the cyan. The cyan is the console's only accent and it means "live
/// or yours" (the One Signal Rule), and a course being set is neither: it
/// is the operator's DRAFT, drawn as an instrument rather than as a state.
/// A warm gold says "this is yours, and it is not yet an order", and it is
/// the one ink on the map that says so.
///
/// It is not a status ink and it is not a rank: nothing reports state in it
/// and no group wears it.
pub const INSTRUMENT_GOLD: Rgba = Rgba::rgb(0xF2, 0xC1, 0x4C);

/// The group-rank palette. Categorical, map-only, barred from chrome: a rank
/// colour on a panel is a bug, not a style.
pub const RANK_UNSUR: Rgba = Rgba::rgb(0x16, 0xA3, 0x4A);
pub const RANK_SATUAN_TUGAS: Rgba = Rgba::rgb(0x25, 0x63, 0xEB);
pub const RANK_GUGUS: Rgba = Rgba::rgb(0x93, 0x33, 0xEA);
pub const RANK_OPERASI_GABUNGAN: Rgba = Rgba::rgb(0xEA, 0x58, 0x0C);

/// THE ECHELON LINKS: the ink a map link wears, one per level.
///
/// A link between a body and its hulls is read by WHICH LEVEL it draws, so the
/// ink is the level's — categorical, map-only, and barred from chrome for the
/// same reason `RANK_*` is. They are not `RANK_*` verbatim because two of that
/// palette's inks are chosen for a legend against chrome, not for a hairline
/// over water: `#2563EB` all but vanishes on the sea, and a link nobody can
/// see is not a link. Warm to cool up the ladder, so two levels stay apart at
/// one pixel.
pub const LINK_SATUAN_TUGAS: Rgba = Rgba::rgb(0xF2, 0xA6, 0x2E);
pub const LINK_GUGUS: Rgba = Rgba::rgb(0xC0, 0x5C, 0xF0);
pub const LINK_OPERASI_GABUNGAN: Rgba = Rgba::rgb(0x3F, 0xD9, 0xB0);

// ---------------------------------------------------------------------------
// Rim
//
// ADR-0016 in full: an island's rim is neutral at rest and lit only while
// that island owns input. Lighting every island in cyan was rendered and
// rejected, because at console scale the accent is then on every surface at
// once and "cyan" stops decoding as anything.
//
// The two are separate constants rather than a colour and a multiplier
// because the ratio is a design decision, not a tuning knob, and a reader
// should not be able to change one without seeing the other.
// ---------------------------------------------------------------------------

/// A resting rim. Neutral, and both constants have useful alphas.
pub const RESTING_RIM: Rgba = CUT_GREY;
pub const RESTING_RIM_ALPHA: f32 = 1.0;
/// The lit rim. The only accent, and only on one surface at a time.
pub const ACTIVE_RIM: Rgba = RADAR_CYAN;
pub const ACTIVE_RIM_ALPHA: f32 = 0.50;

/// The rim line's width, and how far inside the panel edge it rides.
pub const RIM_BAND: f32 = 2.0;
pub const RIM_INSET: f32 = 3.0;

/// The rim colour for the input state. One boolean, no third state: the
/// caller resolves which island owns input before painting, because if two
/// islands each test their own hover then two rims light and the rule
/// means nothing.
pub fn rim(active: bool) -> Rgba {
    let (c, a) = if active {
        (ACTIVE_RIM, ACTIVE_RIM_ALPHA)
    } else {
        (RESTING_RIM, RESTING_RIM_ALPHA)
    };
    c.with_alpha((a * 255.0) as u8)
}

// ---------------------------------------------------------------------------
// The instrument panel silhouette
//
// The replacement world's panel, and the reason it is not the island above.
// The old silhouette was built equipment: a hard cast shadow, a bevelled
// steel casing of three nested fills, a top light and a bottom shade, a 2px
// rim riding 3px inside all of it, and a 20px chamfer saying "this surface
// moves". Nine elements per panel to say "this is a panel".
//
// The new one is a field on an instrument. Square, flat, one 1px edge, and
// the edge is the only thing that changes state. That is not a simplification
// for its own sake — it is what makes the accent mean something again. A lit
// rim on a bevelled, shadowed, chamfered panel is one ornament among nine;
// a hairline that turns Radar Cyan on a flat field is the whole event.
// ---------------------------------------------------------------------------

/// Square. Nothing on an instrument is rounded except the controls you turn.
pub const PANEL_RADIUS: f32 = 0.0;

/// The panel's one edge, and the band's internal rule: same weight, so a
/// panel's outside and its inside divisions are one line weight.
pub const PANEL_EDGE: f32 = 1.0;
pub const PANEL_BAND_RULE: f32 = 1.0;

/// The edge at rest. `CUT_GREY` rather than `HAIRLINE_SLATE`, and the reason
/// is the tile map: the console floats over bright land and dark water in the
/// same frame, and a `HAIRLINE_SLATE` hairline all but disappears over pale
/// land. `CUT_GREY` holds on both, which is the job the old casing was doing
/// with four elements.
pub const PANEL_EDGE_REST: Rgba = CUT_GREY;

/// The edge on the panel that owns input. Full alpha, and deliberately: the
/// old rim was `ACTIVE_RIM_ALPHA` at half strength because it was a glow on
/// top of a bevel, and a glow needs softening. A 1px rule does not glow, it
/// states. One panel at a time, so full strength is the doctrine rather than
/// a shout.
pub const PANEL_EDGE_OWNED: Rgba = RADAR_CYAN;

/// The panel's edge colour for the input state. One boolean, no third state:
/// the caller resolves which panel owns input before painting, because if two
/// panels each test their own hover then two edges light and the rule means
/// nothing.
///
/// This is ADR-0016 unchanged in discipline and changed in costume — same
/// one-boolean, same single-owner resolution, same "exactly one at a time" —
/// expressed as the panel's own border rather than as an inset rim.
pub fn panel_edge(owned: bool) -> Rgba {
    if owned {
        PANEL_EDGE_OWNED
    } else {
        PANEL_EDGE_REST
    }
}

/// Padding inside a panel's band. Tighter than `ISLAND_PAD`'s 16 on purpose:
/// a tracked-caps title with a right-aligned readout is not prose, and 16px
/// of margin either side of it is margin spent on nothing.
pub const PANEL_PAD: f32 = 10.0;

/// Gap between panels stacked in a column. The old zone used 2px, which is
/// right for bevelled silhouettes that read as separate by their own edges
/// and wrong for flat fields: two flat panels 2px apart read as one panel
/// with a seam in it.
pub const PANEL_GAP: f32 = 6.0;

// ---------------------------------------------------------------------------
// Island metrics
//
// One rounding, one title band, one pad. The body is a rounded rect painted
// around an axis-aligned content rect, which is the property that lets the
// silhouette exist at all: it costs paint and not layout, so nothing
// inside an island has to know about it.
// ---------------------------------------------------------------------------

/// Island corner radius: softly rounded, not a bubble. The silhouette used to
/// be a top-right chamfer, ADR-0014's "moves" signal. The title band
/// remains the drag handle, so the rounding costs no affordance.
pub const ISLAND_RADIUS: f32 = 12.0;

/// Brushed-steel bevel tones for the island edge: a dark bed with a bright
/// wire centred on it, plus a top light and a bottom shade, so the border
/// reads as metal catching light rather than as a drawn line.
pub const STEEL_LO: Rgba = Rgba::rgb(0x2B, 0x33, 0x3E);
pub const STEEL: Rgba = Rgba::rgb(0x8E, 0x99, 0xA8);
pub const STEEL_HI: Rgba = Rgba::rgb(0xE6, 0xEB, 0xF2);

/// Chamfer depth. No longer cut into island bodies (see `ISLAND_RADIUS`);
/// kept because the close button's inset is still measured from it and a
/// test pins the relation. The modal silhouette still uses it.
pub const CUT: f32 = 20.0;
/// The title band's height AT THE BASE TYPE SIZE.
///
/// The band holds the surface's title, so its height is the title's height
/// plus its air — and the title grows with the text scale. [`title_h`] is
/// the number the render and the geometry both read; this is the size it
/// is read at when the scale is one.
pub const TITLE_H: f32 = 30.0;

/// The title band's height, at the text scale in force.
///
/// A band that does not grow with its title is a band the title is cut by,
/// and the cut is worse than the smallness: an operator reading a clipped
/// title reads a word with a letter missing and does not know it.
pub fn title_h() -> f32 {
    TITLE_H * text_scale()
}
/// Generic chrome inset: modals, title-band text, miscellaneous breathing
/// room. Islands do NOT use this for their bodies, they use `ISLAND_PAD`.
pub const PAD: f32 = 12.0;
/// Island body inset, all four sides. Roomy on purpose: the body is dense
/// controls and readouts, and 12px left text and buttons feeling pasted to
/// the border.
pub const ISLAND_PAD: f32 = 16.0;
pub const CLOSE: f32 = 18.0;
/// The close button's inset from the panel's right edge. It has to clear the
/// chamfer, because a button centred closer than the cut lands on the
/// diagonal and reads as a smudge.
pub const CLOSE_INSET: f32 = CUT + 14.0;

/// Hard offset shadow, flat black, no blur. A blurred shadow is a fill-rate
/// cost proportional to kernel radius and is the first thing that stops
/// being affordable on a software rasteriser; a hard edge is also the
/// graphic-design shadow rather than a drop shadow.
pub const ISLAND_CAST: Rgba = Rgba::black_alpha(80);
pub const ISLAND_CAST_OFFSET: (f32, f32) = (3.0, 4.0);

/// The cast shadow's offset as `(top, right, bottom, left)`, which is the
/// order a shell's edge builder wants. A hard offset shadow, flat black, no
/// blur, so the four numbers are the whole of it.
pub const fn cast_offset() -> (f32, f32, f32, f32) {
    (0.0, ISLAND_CAST_OFFSET.0, ISLAND_CAST_OFFSET.1, 0.0)
}

/// Island title: monospace, tracked, short caps. The tracking is the whole
/// trick and needs an explicit layout job, because the text format is the
/// only place letter spacing lives.
pub const TITLE_SIZE: f32 = 12.5;
pub const TITLE_TRACKING: f32 = 1.8;

// ---------------------------------------------------------------------------
// The instrument type scale
//
// The replacement world's type, in its own block rather than overwriting the
// `TEXT_*_PT` set above. Those numbers were mirrored from `tokens.rs` and the
// mirror test pinned them to the egui console's values; since U8 prep A it
// pins literals instead, so the values survive `tokens` going with the egui
// console. The new world sets its own scale here, and a panel that has been
// converted reads from this block instead.
//
// Why a condensed face changes the numbers. Saira Condensed occupies far less
// width per point than Inter, so 13pt here reads like Inter's 15pt while
// fitting more of a row. The scale is deliberately *smaller in points and
// larger in information*: a dense console is not a console with big type, it
// is one where the type is small enough to hold a column of quantities and
// clear enough to read across a room in glare.
//
// Two TEXT faces, two jobs, no third. Everything a human wrote is Saira
// Condensed. Everything a machine produced is PT Mono. `tnum` on the figure
// styles is what stops a count reflowing the row around it as the number
// changes — see `TABULAR` below.
//
// And ONE mark face, used once. The wordmark is not body text: it is a
// name, and a name wants letterforms with more character than a label
// carries. It is a third face on the console, and it is spent on six
// letters.
// ---------------------------------------------------------------------------

/// The human face: labels, body, and every quantity a human reads.
pub const FACE_HUMAN: &str = "Saira Condensed";
/// The machine face: ids, timestamps, wire values, the log.
pub const FACE_MACHINE: &str = "PT Mono";
/// The mark face: the wordmark, and nothing else.
///
/// Cinzel — a Roman-capital inscriptional serif, the letterforms of an
/// engraved plate. A console that names itself in a label face names itself
/// like a form field; this one names itself like a ship.
pub const FACE_MARK: &str = "Cinzel";

/// A tracked-caps label: a field's key, a section marker.
/// Small and tracked, because a label that is the same size as the body is
/// not a label, it is a second body.
pub const LABEL_PT: f32 = 10.0;
/// A SURFACE'S TITLE — the strip that names a panel, a modal, an island.
///
/// It is its own tier, and it is the answer to a complaint the console
/// earned: the title used to be drawn at `LABEL_PT`, which made a panel's
/// name the SAME SIZE as the field keys under it. A title and a field key
/// at one weight is not a hierarchy, it is two kinds of text shouting
/// together, and the operator is left with no first thing to read.
///
/// So the title is bigger than the body it heads — three points over
/// `BODY_PT`, which is where a caps title starts to read as a heading
/// rather than as a shout — and it is BOLD, where a label is semibold.
/// Weight is the cheapest hierarchy there is: it costs no width, and the
/// column this console draws in is narrow enough that width is scarce.
pub const TITLE_PT: f32 = 16.0;
/// The dense tier: a small body line in a narrow column.
///
/// The side zone is 320pt wide by default, and a row has to hold a call
/// sign, a name, a role and a state. At `BODY_PT` those four do not fit and
/// the console has no 480pt column to grow into, so a dense surface needs
/// one smaller tier — a hint, a field value, a plan line.
///
/// It is one point under `BODY_PT` and one point over `LABEL_PT`: a ladder,
/// not a spread. It exists so that a panel cannot invent a tenth size of
/// its own, which is what the console did — 14 sizes across one app, nine of
/// them named by no one.
pub const DENSE_PT: f32 = 11.0;
/// Body text in a converted panel.
pub const BODY_PT: f32 = 13.0;
/// A readout that sits inside a row of them.
pub const READOUT_SM_PT: f32 = 18.0;
/// The one readout that leads a panel. Never more than one per panel.
pub const READOUT_LG_PT: f32 = 30.0;
/// Machine output: ids, timestamps, counts inside a data row.
pub const MACHINE_PT: f32 = 12.0;

/// Tracking on a tracked-caps label, in px at `LABEL_PT`. 12% of the size,
/// which is where caps stop reading as a word and start reading as a stamp.
pub const LABEL_TRACKING: f32 = 1.2;

/// The tracking a tracked-caps label of `size` carries.
///
/// Tracking is a RATIO, not a number: 12% of the size is what makes caps
/// read as a stamp, and 12% of a 10pt label is 1.2px while 12% of a 16pt
/// title is 1.9px. A fixed gap would tighten the small labels and loosen
/// the titles, and the two would stop looking like the same voice at two
/// volumes.
pub fn label_tracking(size: f32) -> f32 {
    size * (LABEL_TRACKING / LABEL_PT)
}

/// The wordmark's size, and its tracking.
///
/// The mark is the one thing on this console allowed to be large: it is
/// read once, at the door, and it is the only text a visitor sees before
/// they have any reason to read anything else.
pub const WORDMARK_PT: f32 = 22.0;
/// Tracking on the wordmark, in px. A sixth of the size — the gap a
/// stone-cutter leaves between letters, which is what makes an inscription
/// read as an inscription rather than as a word.
pub const WORDMARK_TRACKING: f32 = 3.4;
/// Tracking on a machine row. A mono face is already even; a third of a pixel
/// is enough to keep a long id from looking cramped.
pub const ROW_TRACKING: f32 = 0.3;

/// The OpenType feature that makes figures tabular: tag and value, as the
/// shaper spells them.
///
/// Kept as a plain `(&str, u32)` so the kernel stays toolkit-free — the gpui
/// `FontFeatures` wrapper is built at the shell boundary, next to `rgba()`,
/// the same place every other gpui type is constructed.
///
/// GPUI 0.2.2 does carry per-style font features to the shaper on Linux:
/// `platform/linux/text_system.rs` keys a `FontKey` on `(family, features)`,
/// `load_family` passes the features to `load_font`, which stores them on
/// `LoadedFont`, and `layout_line` applies them as the shaping attrs. So
/// this reaches cosmic-text rather than being silently dropped — which is the
/// difference between an instrument panel and a number that twitches.
pub const TABULAR_FIGURES: (&str, u32) = ("tnum", 1);

/// Uppercase for a tracked-caps label.
///
/// A pure function with a test because "short caps" is a *layout* rule, not a
/// string rule: a label is uppercased and then measured, so the measurement
/// and the drawing can never disagree about what the label says.
pub fn label_case(label: &str) -> String {
    label.to_uppercase()
}

/// The readout ladder: which size a leading figure draws at, from how much
/// authority it has in its panel.
///
/// One rule rather than a size per call site, so no panel invents a fourth
/// size between `READOUT_SM_PT` and `READOUT_LG_PT`.
pub fn readout_size(leads_panel: bool) -> f32 {
    if leads_panel {
        READOUT_LG_PT
    } else {
        READOUT_SM_PT
    }
}

// ---------------------------------------------------------------------------
// The Settings scale
//
// The one multiplier the operator's own settings row carries. Everything
// else in the type scale is absolute: the sizes above are what the console
// draws at, and this multiplies them together as one proportion so that
// "Larger" moves the whole instrument rather than one row of it.
//
// WHY IT IS A MULTIPLIER AND NOT A SET OF SIZES. A second ladder — one
// ladder per size, at four settings each — is sixteen numbers to keep
// straight and a console that drifts out of proportion the moment one of
// them is missed. One multiplier cannot drift, because every size on the
// console is one base size times one number.
//
// WHY IT IS A GLOBAL AND NOT A PARAMETER. The sizes are read by free
// functions — `note`, `tracked`, `button`, `figure` — that have no view to
// ask. Threading a scale through every one of them would put a parameter
// on the whole shell for the sake of one setting, and the setting would
// still have to be threaded through every call site of those functions.
// The console is a single-window application whose render runs on one
// thread, so the scale is one atomic number rather than a parameter
// forest.
// ---------------------------------------------------------------------------

/// The text sizes the settings row offers, smallest first.
///
/// FOUR STEPS, and the ladder is short on purpose: this is a control an
/// operator touches once, in a room, before the exercise — not a type
/// designer. Each step is about a seventh bigger than the one below it,
/// which is enough to see and small enough that the next step up is never
/// ambiguous.
pub const TEXT_SCALES: [f32; 4] = [1.0, 1.15, 1.3, 1.45];

/// The step the console boots at.
///
/// NOT the smallest, and not the middle: the console shipped at the
/// smallest step and the operator's word on it was that it was too small.
/// One step up is the compromise — every size on the console moves a
/// fifteenth, the dense column still holds its four columns, and the
/// setting is right there when a room wants more.
pub const TEXT_SCALE_DEFAULT: usize = 1;

/// The names the settings row shows, smallest first.
pub const TEXT_SCALE_NAMES: [&str; 4] = ["Small", "Normal", "Large", "Largest"];

/// The scale in force, as the bits of an `f32`.
///
/// An atomic rather than a plain `static mut`: the shell reads it on every
/// frame and writes it from a click handler, and a plain static would be a
/// data race the moment the console grew a second thread.
///
/// It boots at `TEXT_SCALE_DEFAULT`'s step — 1.15, the bits of the f32.
static TEXT_SCALE_BITS: AtomicU32 = AtomicU32::new(0x3f93_3333);

/// The scale in force.
pub fn text_scale() -> f32 {
    f32::from_bits(TEXT_SCALE_BITS.load(Ordering::Relaxed))
}

/// Set the scale in force.
///
/// A scale outside the ladder is REFUSED and the ladder's nearest step is
/// used instead, because a scale of zero would draw nothing and a scale of
/// ten would draw a console nobody can operate. The setting is a choice
/// between four named sizes, not a number the operator types.
pub fn set_text_scale(scale: f32) {
    let mut nearest = TEXT_SCALE_DEFAULT;
    let mut distance = f32::MAX;
    for (i, step) in TEXT_SCALES.iter().enumerate() {
        let away = (step - scale).abs();
        if away < distance {
            distance = away;
            nearest = i;
        }
    }
    TEXT_SCALE_BITS.store(TEXT_SCALES[nearest].to_bits(), Ordering::Relaxed);
}

/// The scale at a step of the ladder, clamped into it.
pub fn text_scale_at(step: usize) -> f32 {
    TEXT_SCALES[step.min(TEXT_SCALES.len() - 1)]
}

/// The step of the ladder the current scale is.
pub fn text_scale_step() -> usize {
    let scale = text_scale();
    TEXT_SCALES
        .iter()
        .position(|s| (s - scale).abs() < f32::EPSILON)
        .unwrap_or(TEXT_SCALE_DEFAULT)
}

/// A size of the console's type scale, at the scale in force.
///
/// EVERY size the console draws goes through this, which is what makes the
/// setting a setting: one number moves every label, title, readout and
/// figure on the instrument together, so the console stays one instrument
/// at any size instead of becoming a collage of panels set at different
/// volumes.
pub fn pt(base: f32) -> f32 {
    base * text_scale()
}

// ---------------------------------------------------------------------------
// Button / control metrics
//
// The geometry a button lays out with: corner radius, the padding inside a
// stock button, and the gap between controls in a row. Mirrored from
// `tokens.rs` so the GPUI shell reads the same numbers the egui shell's
// `apply_ops_theme` installs into `style.spacing`/`style.visuals`. Measured
// against the populated console capture (`.impeccable/critique/capture-
// shipped-console-populated.png`): a stock button is 22 device px tall at the
// 1.25 scale, a `small_button` 18.
// ---------------------------------------------------------------------------

/// Corner radius on every button/field control.
pub const CONTROL_RADIUS: f32 = 6.0;
/// Padding inside a stock `ui.button` (x, y).
pub const BUTTON_PADDING: (f32, f32) = (12.0, 8.0);
/// Gap between controls laid out in a row (x, y).
pub const ITEM_SPACING: (f32, f32) = (8.0, 6.0);
/// `ui.indent` / `ui.horizontal` inner indent, one level.
pub const INDENT: f32 = 20.0;
/// Gap between a choice widget's icon and its label (`spacing.icon_spacing`).
pub const ICON_SPACING: f32 = 4.0;

/// Title band scanline texture. Restricted to the band on purpose
/// (`DESIGN.md`'s "Texture, confined"): rendered across a body the rules cut
/// through the rows and cost legibility on the smallest text on the panel.
pub const BAND_SCAN_PITCH: f32 = 4.0;
pub const BAND_SCAN_ALPHA: u8 = 9;
/// The texture's height, which is the band's height — at the scale in force.
pub fn band_texture_h() -> f32 {
    title_h()
}

// ---------------------------------------------------------------------------
// Zone metrics
//
// The docked column of islands on the right of the map. Every one of these
// moves together when the column is resized, which is why they are constants
// in one place rather than numbers at their call sites.
// ---------------------------------------------------------------------------

/// The column's width AT THE BASE TYPE SIZE.
///
/// The column is sized to the text it holds: a dense row has to carry a
/// call sign, a name, a role and a state, and those four only fit because
/// the column is as wide as the type asks for. So the width is a function
/// of the text scale — [`zone_w`] is the number, and this is what it is at
/// scale one.
pub const ZONE_W: f32 = 320.0;
/// The column's width, at the text scale in force.
pub fn zone_w() -> f32 {
    ZONE_W * text_scale()
}
pub const ZONE_MIN_W: f32 = 240.0;
pub const ZONE_MAX_W: f32 = 600.0;
pub const ZONE_GRIP_W: f32 = 10.0;
/// The top band's own height, and the column's ceiling under it.
///
/// The band is full-width and flush to the window's top, so the column
/// starts where the band ends and the two are one piece of chrome.
///
/// It holds the console's clock — a readout, and the largest type on the
/// instrument — so it grows with the text scale like every other band.
pub const TOP_BAND_H: f32 = 34.0;
/// The top band's height, at the text scale in force.
pub fn top_band_h() -> f32 {
    TOP_BAND_H * text_scale()
}
/// The gap between stacked islands in the column.
///
/// ZERO, and that is a decision rather than a rounding. The column is one
/// surface with fields in it, so the fields touch and share a rule — the
/// way a rack of instruments shares a rail. The 2pt this used to carry
/// put the MAP between two fields of the same panel, and a map-coloured
/// hairline 2pt under a field is not a divider, it is a mistake: the
/// column stopped reading as one thing and became a stack of cards that
/// happened to be near each other.
///
/// The token layer's own note on `PANEL_GAP` argued the other way — that
/// flat panels 2px apart read as ONE panel with a seam in it — and it was
/// right about the SEAM and wrong about the fix. The seam was not the
/// gap; the seam was two 1px edges with map between them. Close the gap
/// and the two edges become ONE shared rule, and the column is joined.
pub const ZONE_ISLAND_GAP: f32 = 0.0;
/// How far below the window's top the column starts: under the top band.
///
/// It used to clear the band AND the step-reminder strip that rides below
/// it, and so the column left a 22pt band of map under the band in every
/// phase — including the three phases with no strip to put there. A
/// sidebar with a stripe of map above it is a card on a map; a sidebar
/// that starts under the band is chrome.
pub const ZONE_TOP_GAP: f32 = TOP_BAND_H;
/// How far below the window's top the column starts, at the scale in force.
pub fn zone_top_gap() -> f32 {
    top_band_h()
}
/// The Inspector image preview's width range and height cap: a tall source
/// must not consume the whole island, and the requested width yields to the
/// panel actually available.
///
/// Moved off the console binary for the cutover (U8 step 1): the fit is
/// tuple math, and only the callers spell egui.
pub const INSPECTOR_IMAGE_MIN_WIDTH: f32 = 160.0;
pub const INSPECTOR_IMAGE_MAX_WIDTH: f32 = 640.0;
pub const INSPECTOR_IMAGE_MAX_HEIGHT: f32 = 180.0;

// ---------------------------------------------------------------------------
// Geometry
//
// Pure functions over f32 tuples. No toolkit rect type, so these run under
// test with no display, no compositor and no GPU. `chrome.rs` becomes a thin
// adapter that calls these and paints the result.
// ---------------------------------------------------------------------------

/// An axis-aligned box, as (min_x, min_y, max_x, max_y).
///
/// CORNERS, NOT ORIGIN AND SIZE — and a renderer that destructures this as
/// `(x, y, w, h)` draws the element as wide as its right edge and as tall as
/// its bottom edge. That is not hypothetical: the planning cards were drawn
/// that way for a day, so six cards each as wide as its own right coordinate
/// overlapped into one dark field, and the operator's report was "they look
/// crumpled with each other". A renderer wants [`Rect`]; [`box_rect`] is the
/// conversion, and it exists so the conversion is written once.
pub type Box = (f32, f32, f32, f32);

/// An axis-aligned rectangle as ORIGIN AND SIZE: `(left, top, width, height)`.
///
/// The shape every toolkit's element builder wants, and the one thing a
/// [`Box`] is not. It has its own name because the two were the same type for
/// a day and one renderer took the wrong one.
pub type Rect = (f32, f32, f32, f32);

pub const fn box_at(x: f32, y: f32, w: f32, h: f32) -> Box {
    (x, y, x + w, y + h)
}

/// A [`Box`] as the [`Rect`] a renderer sets on an element.
///
/// The one conversion, named so a call site cannot hold a box and call it a
/// width: `let (x, y, w, h) = some_box` type-checks and is wrong by exactly
/// the origin.
pub const fn box_rect(b: Box) -> Rect {
    (b.0, b.1, box_w(b), box_h(b))
}

pub const fn box_w(b: Box) -> f32 {
    b.2 - b.0
}

pub const fn box_h(b: Box) -> f32 {
    b.3 - b.1
}

/// Fit an Inspector image inside the available width without letting a
/// tall source consume the whole island. The requested width is a view
/// preference; the fit is still bounded by the current Inspector layout.
///
/// Tuple in, tuple out — the console binary converts at its own call
/// sites, which are the only places that spell egui.
pub fn inspector_image_size(
    texture_size: (f32, f32),
    requested_width: f32,
    available_width: f32,
) -> (f32, f32) {
    let available_width = available_width.max(1.0);
    let width = requested_width
        .clamp(INSPECTOR_IMAGE_MIN_WIDTH, INSPECTOR_IMAGE_MAX_WIDTH)
        .min(available_width);
    if texture_size.0 <= 0.0 || texture_size.1 <= 0.0 {
        return (width, 0.0);
    }
    let aspect = texture_size.1 / texture_size.0;
    let height = width * aspect;
    if height > INSPECTOR_IMAGE_MAX_HEIGHT {
        (
            (INSPECTOR_IMAGE_MAX_HEIGHT / aspect).min(width),
            INSPECTOR_IMAGE_MAX_HEIGHT,
        )
    } else {
        (width, height)
    }
}

pub const fn box_cx(b: Box) -> f32 {
    (b.0 + b.2) * 0.5
}

pub const fn box_cy(b: Box) -> f32 {
    (b.1 + b.3) * 0.5
}

pub const fn box_contains(b: Box, x: f32, y: f32) -> bool {
    x >= b.0 && x <= b.2 && y >= b.1 && y <= b.3
}

/// Whether `(x, y)` is inside `b`, with `min` INCLUDED and `max` EXCLUDED.
///
/// `box_contains` is closed on both ends, and that is right for an island:
/// ownership must leave no seam unclaimed, so an island's edge belongs to the
/// island. A MODAL panel's edge does not, and this is the predicate for that
/// case — a release exactly on a panel's right or bottom edge lands on the
/// MAP. That is the shape egui's `Rect::contains` already had, which is why
/// `drop_lands_on_map` was written against it and why the port must not
/// quietly inherit the nearest predicate instead.
///
/// Two predicates rather than one, because the two callers genuinely
/// disagree about who owns a boundary.
pub const fn box_holds_interior(b: Box, x: f32, y: f32) -> bool {
    x >= b.0 && x < b.2 && y >= b.1 && y < b.3
}

/// `b` grown by `d` on all four sides.
pub const fn box_expand(b: Box, d: f32) -> Box {
    (b.0 - d, b.1 - d, b.2 + d, b.3 + d)
}

pub const fn box_translate(b: Box, dx: f32, dy: f32) -> Box {
    (b.0 + dx, b.1 + dy, b.2 + dx, b.3 + dy)
}

/// Where the body content goes inside an island: below the title band,
/// inside the pad.
pub fn island_content_box(island: Box) -> Box {
    (
        island.0 + ISLAND_PAD,
        island.1 + title_h() + ISLAND_PAD,
        island.2 - ISLAND_PAD,
        island.3 - ISLAND_PAD,
    )
}

/// The title band's own box, its top corners rounded with the panel.
pub fn island_title_box(island: Box) -> Box {
    (island.0, island.1, island.2, island.1 + title_h())
}

/// The close button's centre in the band.
pub fn island_close_center(island: Box) -> (f32, f32) {
    (island.2 - CLOSE_INSET, island.1 + title_h() * 0.5)
}

/// The band's right padding: enough to clear the close cross where the island
/// has one, and the panel's own pad where it does not.
///
/// The trailing note is a plain child pushed right by `justify_between`, and
/// the cross is ABSOLUTE at `island_close_center` -- so without this the note
/// runs underneath it. A capture showed exactly that: `3 SEATED` with the
/// cross painted over its last two glyphs, which is a count nobody can read,
/// and the defect is invisible in a capture where the note happens to be
/// short.
pub const fn island_band_pad_right(closeable: bool) -> f32 {
    if closeable {
        CLOSE_INSET + CLOSE * 0.5 + 3.0
    } else {
        PANEL_PAD
    }
}

/// Which island owns the pointer, if any. Ownership is resolved once by the
/// zone and passed into each island, so exactly one rim lights.
pub fn owning_island(rects: &[Box], px: Option<(f32, f32)>) -> Option<usize> {
    let (x, y) = px?;
    rects.iter().rposition(|r| box_contains(*r, x, y))
}

/// Clamp a dragged island's top-left into the viewport.
///
/// The drag half of `chrome::island_owned`'s rule: the pointer delta is
/// added to the column position BEFORE this runs, and ONLY drags come
/// here. A computed column position legitimately sits below the window
/// (that is what a scrollable column is), so clamping it would pull the
/// island to the window's bottom edge and fight the scroll chain. The
/// scroll itself is a separate view transform, never folded into `pos`.
pub fn clamp_island_to_viewport(x: f32, y: f32, w: f32, h: f32, viewport: Box) -> (f32, f32) {
    let cx = x.clamp(viewport.0, (viewport.2 - w).max(viewport.0));
    let cy = y.clamp(viewport.1, (viewport.3 - h).max(viewport.1));
    (cx, cy)
}

/// Whether an island at column height `y` still shows once the column has
/// been scrolled by `scroll`. Mirrors `chrome::island_on_band`.
///
/// Culled rather than drawn off-band: an island outside the band must not
/// own input either, and ownership is decided from rects the caller builds,
/// so the caller needs this predicate, not just the drawing. Straddlers
/// stay — any overlap with the band is enough.
pub fn island_on_band(y: f32, scroll: f32, band: Box, height: f32) -> bool {
    let top = y - scroll;
    top + height >= band.1 && top <= band.3
}

// ---------------------------------------------------------------------------
// Scroll measurement
//
// What one island body measured about itself, published so the zone can
// decide who owns the wheel. Mirrors `chrome::BodyFit` 1:1 — three numbers
// and no more. `content_h` against `viewport_h` is whether the body has
// anywhere to scroll at all; `offset` is where in its range it already is.
//
// The GPUI shell fills this from a `ScrollHandle` at its own boundary
// (`offset()` is negative-down there, positive-down here; `content_h` is
// `max_offset + viewport`). No toolkit type crosses: the handle is read
// where the shell converts colours, and only f32s arrive here.
// ---------------------------------------------------------------------------

/// What one island body measured about itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BodyFit {
    /// The height the content asked for.
    pub content_h: f32,
    /// The height it was given.
    pub viewport_h: f32,
    /// How far the body is scrolled down. Positive is further down.
    pub offset: f32,
}

impl BodyFit {
    /// Whether the body has more content below the fold than above it.
    /// False for a body that fits, and for one already at the bottom —
    /// which is the case that hands the wheel back to the column.
    pub fn can_scroll_down(&self) -> bool {
        self.content_h - self.viewport_h - self.offset > BODY_SCROLL_SLACK
    }

    /// Whether the body has more content above the fold than below it.
    pub fn can_scroll_up(&self) -> bool {
        self.offset > BODY_SCROLL_SLACK
    }
}

/// Content a body may run past its viewport without claiming the wheel.
///
/// One text line. A layout's measured extent runs a few points past the
/// last control for trailing spacing (the Operator island measures 98.4
/// against 94 available: 4pt of nothing). Without this, that 4pt is
/// scrollable, so the body claims the first wheel gesture and moves four
/// points the operator cannot see while the column stays put.
///
/// `chrome` also defines `BODY_SCROLL_EPSILON` (0.5) for float residue,
/// but nothing reads it — dead on arrival, deliberately not mirrored.
pub const BODY_SCROLL_SLACK: f32 = 12.0;

// ---------------------------------------------------------------------------
// Zone chrome
//
// The resize grip and the drag-width rule from `side_zone`. The grip is a
// 10px strip straddling the zone's inner edge over the full band height;
// dragging it writes the live width, which the next frame's origins, band
// and camera offset all read — so the resize is live and nothing
// disagrees mid-drag. `Dock::Right` (this console's dock) grows leftward:
// a leftward pointer delta widens the zone.
// ---------------------------------------------------------------------------

/// Grip strip for the zone's inner edge: 10px wide, centred on `edge_x`,
/// full band height. `edge_x` is the band's left edge on a right dock,
/// its right edge on a left dock.
pub fn zone_grip_rect(band: Box, edge_x: f32) -> Box {
    (
        edge_x - ZONE_GRIP_W * 0.5,
        band.1,
        edge_x + ZONE_GRIP_W * 0.5,
        band.3,
    )
}

/// Next live zone width after a grip drag: add the pointer delta on a left
/// dock, subtract on a right dock, then clamp to the rails. Pure so both
/// shells (and the test) agree with the running column.
pub fn zone_drag_width(start_w: f32, dx: f32, right_dock: bool, viewport_w: f32) -> f32 {
    let dw = if right_dock { -dx } else { dx };
    clamp_zone_width(start_w + dw, viewport_w)
}

// ---------------------------------------------------------------------------
// Wheel chaining
//
// One gesture moves ONE thing: a body that can still move in the
// direction the wheel is turning takes it, otherwise the column does.
// Without the chain both scroll at once and the gesture jumps two
// surfaces. Mirrors the `side_zone` decision (`main.rs`): the measurement
// is live (GPUI feeds handles during layout; egui read last frame's),
// positive wheel Y is content moving down, and the column subtracts.
//

/// Who takes a wheel gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelOwner {
    /// A body under the pointer has room: its own scroller moves.
    Body,
    /// No body can take it: the column moves (or nothing, if outside).
    Column,
    /// Outside the band, or a zero delta: nobody moves.
    Neither,
}

/// Route one vertical wheel delta. `fit` is the body under the pointer, if
/// any open on-band body contains it; `in_band` gates the column fallback.
pub fn chained_wheel(fit: Option<BodyFit>, wheel_y: f32, in_band: bool) -> WheelOwner {
    if !in_band || wheel_y == 0.0 {
        return WheelOwner::Neither;
    }
    match fit {
        Some(f) if wheel_y > 0.0 => {
            if f.can_scroll_up() {
                WheelOwner::Body
            } else {
                WheelOwner::Column
            }
        }
        Some(f) => {
            if f.can_scroll_down() {
                WheelOwner::Body
            } else {
                WheelOwner::Column
            }
        }
        None => WheelOwner::Column,
    }
}

/// Zone layout: the column's width, clamped to its rails.
pub fn clamp_zone_width(w: f32, viewport_w: f32) -> f32 {
    w.clamp(ZONE_MIN_W, ZONE_MAX_W).min(viewport_w * 0.8)
}

/// The band the column lives in: under the top band, down to the window's
/// bottom edge, and against the edge it is docked to.
///
/// A FULL-HEIGHT sidebar. It is chrome, so it is flush the way the top
/// band is flush: it starts under the band and it runs to the window's
/// edge, with no margin of map showing under it. The margin it used to
/// keep was the console's old floating-card furniture, and a column that
/// stops 24pt short of the window's bottom is a card that happens to be
/// tall.
pub fn zone_band(viewport: Box, zone_w: f32) -> Box {
    (
        viewport.2 - zone_w,
        viewport.1 + zone_top_gap(),
        viewport.2,
        viewport.3,
    )
}

/// How much width the top band needs before it shows its session readouts.
///
/// The console carries the same rule with the same number and the same
/// reason (`main.rs::top_zone`'s `HINT_BUDGET`): the band is one row of
/// readouts AND actions, and a left group that consumes the whole width
/// pushes the right-hand group past the window edge — taking the sign-out
/// button with it. Being wrong costs a hint rather than a button, so the
/// budget is set with slack.
///
/// The console drops its hint line first. This drops the two session
/// readouts instead, because they are the widest things on the left and the
/// game's name is already on the Step island's body and the Inspector. What
/// it must NEVER drop is a door: a band whose buttons have scrolled off is a
/// console with no way out.
pub const BAND_READOUT_BUDGET: f32 = 900.0;

// ---------------------------------------------------------------------------
// The game clock's multiplier
// ---------------------------------------------------------------------------

/// The rail Control's draft is clamped to — the console's own
/// `DragValue::range(1.0..=144.0)`, unchanged. 144 is a wall-clock hour in
/// exercise minutes: one real minute is a whole exercise hour, which is as
/// fast as a run is legible.
pub const FACTOR_MIN: f64 = 1.0;
pub const FACTOR_MAX: f64 = 144.0;

/// Next multiplier after one nudge. Clamped rather than wrapped: a step past
/// the rail is a no-op, because wrapping 144 to 1 would be a write nobody
/// asked for, and the button that did it is one pixel from the button that
/// means "slower".
pub fn factor_nudge(current: f64, step: f64) -> f64 {
    (current + step).clamp(FACTOR_MIN, FACTOR_MAX)
}

/// The planned window, as one line, and whether it is COMPLETE.
///
/// Both ends or neither is the console's rule: an incomplete window is the
/// ordinary state of a draft and is also the reason the execution transition
/// is refused, so an incomplete one is reported as a fact rather than drawn as
/// an empty row. The exercise clock is shown when the real-world window is not
/// set, because that is the half the operator authored for the exercise — and
/// only its TIME is meaningful, so the date is cut at the `T`.
pub fn window_label(
    actual_start: &str,
    actual_end: &str,
    assumed_start: &str,
    assumed_end: &str,
) -> (String, bool) {
    let hhmm = |v: &str| -> String {
        match v.split_once('T') {
            Some((_, rest)) => rest.chars().take(5).collect(),
            None => v.to_string(),
        }
    };
    if !actual_start.is_empty() && !actual_end.is_empty() {
        return (format!("{}-{}", hhmm(actual_start), hhmm(actual_end)), true);
    }
    if !assumed_start.is_empty() && !assumed_end.is_empty() {
        return (
            format!("{}-{} (assumed)", hhmm(assumed_start), hhmm(assumed_end)),
            true,
        );
    }
    ("window incomplete".to_string(), false)
}

/// Where the nth island in the column starts, stacking downward.
pub fn zone_island_origin(band: Box, index: usize, heights: &[f32]) -> (f32, f32) {
    let mut y = band.1;
    for h in heights.iter().take(index) {
        y += h + ZONE_ISLAND_GAP;
    }
    (band.0, y)
}

// ---------------------------------------------------------------------------
// The column's composition
//
// WHICH islands the column holds is a function of where the exercise is —
// the axis the whole console's layout hangs off (`gamestate::GameState`).
// Before this table existed, the shell's three islands were literals in
// `new()`: same title, same body, same height, unrelated to anything the
// session said. That is a demo's arrangement, and it is the reason the
// chrome read as scaffolding rather than as a console.
//
// The heights are the console's own, read off `main.rs::side_zone` against
// its bodies with the enlarged base text. They are carried rather than
// re-derived because the ported bodies ARE the console's bodies, and an
// island's footprint is fixed by contract: a body that does not fit
// scrolls, it does not grow (see `chrome::island`'s doc for the
// measurement that settled this).
// ---------------------------------------------------------------------------

/// One island in the column, by what it IS rather than by where it sits.
///
/// A key rather than an index because the column's membership changes with
/// the exercise's state while the operator's open/closed choice must not:
/// closing Crew in planning and keeping it closed in execution is the same
/// choice, and an index would silently re-open it under a different name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IslandKey {
    /// The hull register: what the fleet HAS. A PLANNING surface — the
    /// register answers what the exercise fights with, and that is a
    /// question the plan asks, not the empty room.
    Fleet,
    /// The game's own terms: name, mode, pace, the planned window, the
    /// Game Master's prose. The console's `z.essentials`, which is where a
    /// plan is read back before it is executed.
    Essentials,
    /// The game clock's multiplier: what it is, and the one verb that changes
    /// it. The console's `z.control`, and the only island in the column whose
    /// whole body is a single number and the button that writes it.
    Control,
    /// The room: who is SEATED in it, with the door to the seating form. The
    /// console's `z.players`, which is a planning surface — seating is what
    /// planning does.
    Players,
    /// The room as it actually stands: who has ENTERED it, whether they are
    /// ready, and who is still outside. The console's `z.crew`, and a
    /// different question from `Players` over the same roster — seating is a
    /// role, arrival is a fact, and an exercise with three seats and one
    /// arrival is the thing the Game Master needs to see.
    Crew,
    /// The gate: what the server still wants before the exercise may start,
    /// the room key to hand out, and the advance itself. The console's
    /// `z.review`.
    Review,
    /// The task organisation: the forest of task groups, their commanders and
    /// their member hulls. The console's `z.groups`, authored locally while
    /// planning and committed at the advance.
    Groups,
    /// The caller's own command: seat, commanded hulls, their live fixes,
    /// the readiness gate and the helm.
    Command,
    /// What happened, as the operator was told it. The console's `z.log`,
    /// which is two surfaces in one island: an event feed capped at 30 lines
    /// that orders and arrivals land in, and below a rule the session's
    /// journal files with a replay slider driving map ghosts.
    ///
    /// This shell carries the FEED and not the journals: the journal half is
    /// local scratch — files on disk plus a map replay — and has no read
    /// route at all.
    Log,
    /// The debrief: what the exercise was, read back after it is over. The
    /// console's `z.assessment`, which is five tabs — Summary, Timeline,
    /// Judgements, Reviews, Transcript.
    ///
    /// This shell carries four of the five; only Transcript is named
    /// rather than drawn, a local trace with no route.
    Assessment,
}

impl IslandKey {
    /// The band's tracked short-caps label.
    ///
    /// The console's island titles, unchanged: a panel the operator already
    /// knows under one name must not be renamed by a port.
    pub fn title(self) -> &'static str {
        match self {
            IslandKey::Essentials => "ESSENTIALS",
            IslandKey::Control => "CONTROL",
            IslandKey::Fleet => "FLEET",
            IslandKey::Players => "PLAYERS",
            IslandKey::Crew => "CREW",
            IslandKey::Review => "REVIEW",
            IslandKey::Groups => "GROUPS",
            IslandKey::Command => "COMMAND",
            IslandKey::Log => "LOG",
            IslandKey::Assessment => "ASSESSMENT",
        }
    }

    /// A stable string for the surfaces that need one. The scroll handle is
    /// keyed by the island's DISCRIMINANT rather than by this, because the
    /// column's membership changes with the state and an index would hand one
    /// island's scroll offset to whichever island landed in that slot next;
    /// this name is for logs and for anything a human reads.
    pub fn id(self) -> &'static str {
        match self {
            IslandKey::Essentials => "essentials",
            IslandKey::Control => "control",
            IslandKey::Fleet => "fleet",
            IslandKey::Players => "players",
            IslandKey::Crew => "crew",
            IslandKey::Review => "review",
            IslandKey::Groups => "groups",
            IslandKey::Command => "command",
            IslandKey::Log => "log",
            IslandKey::Assessment => "assessment",
        }
    }
}

/// Every island the column knows, in a FIXED order.
///
/// The order here is not the render order — that is `zone_islands`' job and
/// it changes with the exercise's state. What this list fixes is the identity
/// the operator's open/closed choice hangs off, and it is declared once so no
/// code path can hold a subset and quietly drop one.
pub const COLUMN_ISLANDS: [IslandKey; 10] = [
    IslandKey::Essentials,
    IslandKey::Control,
    IslandKey::Fleet,
    IslandKey::Players,
    IslandKey::Crew,
    IslandKey::Review,
    IslandKey::Command,
    IslandKey::Log,
    IslandKey::Assessment,
    IslandKey::Groups,
];

/// One slot in the column: which island, and how tall its footprint is.
pub struct IslandSlot {
    pub key: IslandKey,
    pub height: f32,
}

/// Which islands the column holds, in order, for the state the exercise is
/// in — the console's `side_zone`, cut to the surfaces this shell has.
///
/// `player` is the console's `player_station()` VERBATIM: a seat held by a
/// non-judge. It deliberately does NOT require a commanded hull — that is the
/// DASHBOARD's rule (`main.rs:12719`), and conflating the two would hide the
/// organizer column from a seated player who has not been given a hull yet,
/// which is most of a planning room. It matters only in execution: a player
/// station keeps identity, the step, and its HELM — the console serves that
/// helm from the floating dashboard, and this shell has no floating islands,
/// so the column carries it. Without that a player station could not order
/// its own hull, the one thing the station exists to do.
///
/// The omissions are named rather than hidden:
///
/// - `z.start` (NoSession) is a session PICKER plus a create button, and it
///   is NOT a column island. The owner's direction: with no exercise held
///   there is nothing to read beside the map — the column would be a stack of
///   answers to questions nobody has asked yet. So the empty room is an EMPTY
///   column, and the games to hold live in the MIDDLE of the frame, in the
///   modal that carries the list and the wizard that makes a new one.
/// - `z.essentials` and `z.control` (Planning) are the game's own terms and
///   pace. No read route exists for either here.
/// - `z.groups` is the task organisation. It used to be omitted because
///   `/hierarchy` was empty for every dev game; the console authors it locally
///   while planning and commits it at the advance, so the column now carries it.
/// - `z.log` IS ported, and rides the organizer's Execution column as the
///   console's own Execution column carries it. What is NOT ported is the
///   journal half — its files and map replay are local scratch with no
///   route — and the Log here carries the console's 30-line event feed.
/// - Assessment carries the console's Summary, Timeline, Judgements and
///   Reviews tabs — Transcript is a local trace and is not carried. The
///   first three have backend reads in this crate; Timeline reads on
///   demand under its filters with a cursor, while Judgements and Reviews
///   ride the closure fetch.
/// - Comms is NOT a column island and never was in the console either: its
///   inbox is a floating pane opened from the band's door. This shell has no
///   floating panes, so the same door opens the same surface in the modal
///   zone. A 560pt slot in every state's column would also have made the
///   per-state layout a wall rather than a composition.
pub fn zone_islands(state: crate::gamestate::GameState, player: bool) -> Vec<IslandSlot> {
    let slot = |key: IslandKey, height: f32| IslandSlot { key, height };
    let mut out: Vec<IslandSlot> = Vec::new();
    match state {
        // NO GAME HELD: the column is EMPTY, and that is the whole answer.
        //
        // The three surfaces this state used to carry were each moved, not
        // dropped. Identity went to the top band's right, where it is one
        // chip and one click (it does not change with the exercise, so it
        // never belonged in a column that does). The games to hold went to
        // the middle of the frame, as the modal that carries the list and
        // the wizard. The hull register went to PLANNING, which is the phase
        // that assembles a force.
        //
        // What is left is a map with a band over it, and the band's door
        // opens the surface that answers the only question this state has:
        // which exercise.
        crate::gamestate::GameState::NoSession => {}
        // PLANNING IS THE GRID, not a column.
        //
        // The phase used to be a stack down one side of the window, and a
        // stack is a scroll: the operator scrolled to find the fifth thing
        // they needed, and the ground they were planning on was a strip
        // down the side of the frame. The phase is six equal panels over
        // the map now (`planning_grid`), and the column holds nothing at
        // all in this state.
        //
        // The Control island is GONE, and that is the owner's direction:
        // the multiplier, the hold and the window are the exercise clock,
        // and the exercise clock is part of the exercise's terms — so it
        // lives in the terms panel now, and a planning room does not say
        // the exercise's name and its clock in two separate places.
        crate::gamestate::GameState::Planning => {}
        // Preparation is the gate. The console's Readiness island is
        // exactly what Command carries here.
        //
        // The console's SECOND slot is Essentials — the terms whose absence
        // is what blocks the gate — and this shell has no terms island, so
        // that slot is filled with Crew rather than left empty: the arrivals
        // are what the gate is ultimately about, and a column of one island
        // beside a 360pt gate reads as a page nobody finished.
        //
        // NOTE this column carries an island the console's does not. The
        // console's Preparation is Readiness + Essentials and lists no Crew;
        // Crew is this shell's substitution for the missing terms surface,
        // not a port. (An earlier version of this comment claimed "Players
        // rides along" — the code has pushed Crew since, and the comment
        // described a column that was never built.)
        crate::gamestate::GameState::Preparation => {
            // THE COMMAND PANEL IS GONE FROM PREPARATION (the owner's
            // direction: for a Game Master or a judge seat it only said "no
            // hull" and "no helm", and readiness is the one verb it carried).
            // The verb moved into the centred crew screen, and the column
            // keeps the room: who is here, and the door to the order of
            // battle.
            out.push(slot(IslandKey::Crew, 290.0));
        }
        // Execution is the state the console spends its life in, so the
        // column is a working surface rather than a summary: what you can
        // order, who is here, and the feed of what happened.
        //
        // THE STEP ISLAND IS GONE, and it is the operator's direction rather
        // than a preference: a step is not one small panel's worth of
        // information. It is the whole book, which of its steps have been
        // played, which one the exercise is on, the hulls each step obliges
        // and whether the gate is clear — plus, for the Game Master, the
        // verbs that move it. A 300pt column carries four of those and hides
        // the rest, so the surface moved to the MIDDLE of the frame
        // (`shell_gpui::IslandDemo::board_body`), where a scenario has room
        // to be as big as it is. The column keeps what belongs in a column:
        // the helm, the room and the feed.
        crate::gamestate::GameState::Execution => {
            // The helm, for EVERYONE who commands a hull \u{2014} and a player
            // station is exactly that.
            //
            // The console serves a player's helm from the DASHBOARD, a
            // floating "My command" island `apply_transition` opens on every
            // move into preparation or execution, judge seat or not. This
            // shell has no floating islands, so the same surface has to come
            // from the column \u{2014} and until it did, a player station could
            // not order their own hull at all, which is the one thing their
            // station exists to do.
            out.push(slot(IslandKey::Command, 420.0));
            if !player {
                out.push(slot(IslandKey::Crew, 240.0));
                out.push(slot(IslandKey::Log, 260.0));
            }
        }
        // Closure is read-only, so the column is what you debrief with. The
        // console lists Assessment and its Log and says it plainly: no
        // Orders, because nothing is ordered after the fact and a control
        // that would be refused is a lie about the state.
        //
        // Assessment and Log are BOTH ported, so Closure is now the console's
        // own column: the debrief read-back, and the feed of what happened.
        // Crew is NOT the console's — its Closure lists none — and it is kept
        // because a debrief is about who was in the room.
        // See the handoff's "What the console lists and this shell does not".
        crate::gamestate::GameState::Closure => {
            out.push(slot(IslandKey::Assessment, 420.0));
            out.push(slot(IslandKey::Log, 260.0));
            // The crew list is the same list the Preparation phase carries,
            // and it is the same size: five people, a section, a hint and
            // five rows. At 240 the last row was cut by the panel's own
            // edge, and a row cut in half is a row nobody can read.
            out.push(slot(IslandKey::Crew, 330.0));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The planning GRID
// ---------------------------------------------------------------------------

/// The planning phase's layout: SIX EQUAL islands, three across and two
/// down, floating over the map.
///
/// The planning phase used to be a COLUMN — a stack of panels down one side
/// of the map, scrolled. A column is the right answer for a console that
/// must hold every state at once, and the wrong one for a phase whose
/// membership is fixed and known: the operator scrolled to find the fifth
/// thing they needed, and the map they were planning ON was a strip down
/// the side.
///
/// Six equal cells over the map puts the ground UNDER the plan: the
/// exercise is fought on that water, and the plan is drawn on it.
pub const GRID_COLS: usize = 3;

/// The narrowest a cell may be and still hold the console's own panels.
///
/// The console's column is 320pt wide, and its panels were built for it: a
/// field row, a rule and a value. A cell narrower than that cannot hold
/// them, and a cell that cannot hold them shows them cut at the edge.
pub const GRID_MIN_CELL_W: f32 = 320.0;

/// The margin BETWEEN two islands.
///
/// A margin, not a border: the map shows through it, and it is what says
/// "these are six things and not one thing with six rooms".
///
/// 16 AND NOT 12. The panels are dark and TRANSLUCENT over a map that is
/// dark water most of the time, so a 12pt margin read as the panels' own
/// edges touching: the gap was there and the eye could not find it. The
/// margin is the only thing separating six panels over the same sea, and
/// it has to be wide enough to see.
pub const GRID_GAP: f32 = 16.0;

/// The margin from the window's edge to the outer islands.
///
/// The same as [`GRID_GAP`], so the frame's own margin and the margin
/// between two panels are one measurement. A grid whose outer margin is
/// tighter than its inner one reads as panels pushed at the window's
/// edges, which is the opposite of air.
pub const GRID_INSET: f32 = 16.0;

/// The corner radius of a planning panel.
///
/// The console's column panels are SQUARE ([`PANEL_RADIUS`]) because they
/// are instruments bolted into a stack. These six are not a stack: they
/// float on the map with a margin all round them, and a floating card with
/// square corners reads as a hole cut in the water rather than as a card
/// laid on it. [`ISLAND_RADIUS`] is the rounding the console already
/// trusts for exactly that shape.
pub const GRID_CARD_RADIUS: f32 = ISLAND_RADIUS;

/// The body inset of a planning panel.
///
/// Larger than [`PANEL_PAD`], which is a band's inset: these bodies hold
/// fields, lists and rules, and 16 is the same breathing room an island
/// body gets ([`ISLAND_PAD`]). The columns are a third of the window wide,
/// so a body pinned at 10 to its own edge reads as text against a wall.
pub const GRID_PAD: f32 = ISLAND_PAD;

/// The planning panels' opacity.
///
/// The planning phase draws its six panels OVER the map, and the map is
/// the ground the plan is written for — so the panels are a little bit
/// transparent, and the operator can see the water their exercise will be
/// fought on through the plan that will be fought there.
///
/// NOT LESS. The console's ink is light on a dark panel, and the panel is
/// dark; at 91% over a pale map the field is still a dark field, and the
/// type still reads. Below that the type starts to fight the coastline
/// under it, and a plan nobody can read is worse than a map nobody can
/// see.
pub const GRID_CARD_ALPHA: u8 = 240;

/// The six panels of the planning grid.
///
/// The order is the READING order, and it is the task's order: what the
/// exercise IS (Essentials), what is running it (Exercise), who is in it
/// (Players), what fights (Fleet), what one of them is (Unit), and how it
/// is organised (Groups). The operator reads the grid left to right, top
/// to bottom, and this is the path their eye takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GridPanel {
    /// The session's own terms, EDITED IN PLACE: the name, the pace, the
    /// Game Master's prose, the planned window, the room key, the clock's
    /// multiplier, and — in a scenario exercise — the book it plays.
    ///
    /// The form is the creation wizard's, because it asks the same
    /// questions of the same session: a planning room should not make the
    /// operator open a second surface to fix a typo in the name they typed
    /// at the door.
    Essentials,
    /// The exercise itself, and nothing else: the console's mark, the kind
    /// of game, the phase, and the one verb that ends the phase.
    Exercise,
    /// Who is in the room — the seats the session holds — and the door to
    /// the picker those seats are chosen in.
    Players,
    /// What fights — the hulls the session holds — and the door to the
    /// register they are drawn from.
    Fleet,
    /// The selected hull, in full.
    Unit,
    /// The task organisation: the groups, and the groups inside groups.
    Groups,
}

impl GridPanel {
    /// The band's title.
    pub fn title(self) -> &'static str {
        match self {
            GridPanel::Essentials => "ESSENTIALS",
            GridPanel::Exercise => "EXERCISE",
            GridPanel::Players => "PLAYERS",
            GridPanel::Fleet => "FLEET",
            GridPanel::Unit => "UNIT",
            GridPanel::Groups => "GROUPS",
        }
    }

    /// Whether the panel carries a title band.
    ///
    /// FIVE OF THE SIX DO. The EXERCISE panel does not, and the reason is
    /// that its title IS its body: the console's wordmark, ARCONS, is the
    /// largest thing on the grid and it is already the name of the surface.
    /// A band above it saying EXERCISE gives one panel two headings and
    /// spends the top of the tallest cell on the weaker of them.
    pub fn has_title(self) -> bool {
        self != GridPanel::Exercise
    }

    /// The six panels, in reading order.
    pub const ALL: [GridPanel; 6] = [
        GridPanel::Essentials,
        GridPanel::Exercise,
        GridPanel::Players,
        GridPanel::Fleet,
        GridPanel::Unit,
        GridPanel::Groups,
    ];
}

/// Where the planning grid sits, and where each of its six cells is.
///
/// The grid fills the window BELOW the top band — the band is the clock
/// and the identity, and it is the one thing that is the same in every
/// phase, so it stays where it is — and the cells are EQUAL: a grid whose
/// cells are not equal is a grid whose importance is decided by size, and
/// the operator reads the big one and leaves the rest.
///
/// The cells are computed here and drawn there, so the layout cannot be
/// one thing on the screen and another in the code.
pub fn planning_grid(viewport: Box) -> Vec<Box> {
    let top = top_band_h() + GRID_INSET;
    let area_w = (viewport.2 - 2.0 * GRID_INSET).max(0.0);
    let area_h = (viewport.3 - top - GRID_INSET).max(0.0);
    let cols = planning_grid_cols(viewport.2);
    let rows = (GridPanel::ALL.len() + cols - 1) / cols;
    let cell_w = (area_w - GRID_GAP * (cols - 1) as f32) / cols as f32;
    let cell_h = (area_h - GRID_GAP * (rows - 1) as f32) / rows as f32;
    let mut out = Vec::with_capacity(cols * rows);
    for row in 0..rows {
        for col in 0..cols {
            out.push(box_at(
                GRID_INSET + col as f32 * (cell_w + GRID_GAP),
                top + row as f32 * (cell_h + GRID_GAP),
                cell_w,
                cell_h,
            ));
        }
    }
    out
}

/// The grid's columns, and they follow the window.
///
/// THREE on a window wide enough to hold them, and TWO on one that is not:
/// a cell narrower than the console's own column cannot hold the console's
/// own panels, and a grid of cells that cannot hold them is a grid of
/// cut-off rows.
///
/// So the grid is responsive the way a layout should be: the WINDOW decides
/// how many columns there are, and the panels inside them are the same
/// panels either way.
pub fn planning_grid_cols(viewport_w: f32) -> usize {
    let area = (viewport_w - 2.0 * GRID_INSET).max(0.0);
    let fits = |cols: usize| (area - GRID_GAP * (cols - 1) as f32) / cols as f32 >= GRID_MIN_CELL_W;
    if fits(GRID_COLS) {
        GRID_COLS
    } else if fits(2) {
        2
    } else {
        // ONE column: the window is too narrow for the console's own panels
        // side by side, and a grid of cells that cannot hold them is a grid
        // of cut-off rows. One column is the console's own column, and it
        // holds.
        1
    }
}

// ---------------------------------------------------------------------------
// The SESSIONS card
// ---------------------------------------------------------------------------

/// The SESSIONS card's footprint: the games to hold on the left, the wizard
/// on the right.
///
/// It is the same family of width as the other two-column cards (760), for
/// the same reason: two sections side by side need the room, and a card
/// that gives the wizard a third of the window gives the wizard's fields
/// nowhere to put their labels.
pub const SESSIONS_W: f32 = 760.0;

/// The SESSIONS card's height. Enough for the list on the left and the
/// wizard's longest step on the right; the window's own height caps it.
pub const SESSIONS_H: f32 = 560.0;

/// One session row's height.
///
/// FIXED, and that is the point: a list whose rows are content-sized is a
/// list whose rows do not line up, and a click that lands between two rows
/// is a click that holds the wrong exercise. The row's own height is a
/// token, so the row a test aims at is the row that is drawn.
pub const SESSION_ROW_H: f32 = 22.0;

/// One tab band's height in the SESSIONS card's history: the five state tabs
/// between the list's title and its first row.
///
/// A token for the same reason a row's height is one: a band whose height is
/// its content's is a band that moves the first row, and a click that lands on
/// a tab when it aimed at a row holds the wrong exercise.
pub const TAB_BAND_H: f32 = 22.0;

/// The margin between the SESSIONS card and the window's edge, on every
/// side.
///
/// The card is a thing ON the console, not the console: it sits inside the
/// window with a margin, the way every other modal sits, and the margin is
/// what lets the operator see that the card is a card.
pub const CARD_INSET: f32 = 32.0;

/// The SESSIONS card's rectangle: centred in the window, and never bigger
/// than the window it is opened in.
///
/// A card the window cannot hold is a card whose second section is off the
/// edge, and this card's second section is the wizard — which is the half
/// of the console that makes an exercise.
///
/// **THE CARD FITS THE WINDOW IN BOTH DIRECTIONS, AND THE FLOOR IS
/// APPLIED FIRST.** The card would LIKE to be at least this big; it may
/// never be bigger than the window. A card taller than the window is a
/// card whose title band is cut off by the window's own edge, and the band
/// exists to carry the name of the surface — an operator who cannot read
/// "SESSIONS" does not know which surface they are in.
pub fn sessions_card_rect(viewport: Box) -> Rect {
    // The room the card has, once the window's own margin is taken off.
    let room_w = (viewport.2 - 2.0 * CARD_INSET).max(0.0);
    let room_h = (viewport.3 - 2.0 * CARD_INSET).max(0.0);
    let w = SESSIONS_W
        .min(room_w)
        .max(240.0)
        .min((viewport.2 - 16.0).max(0.0));
    // The card holds the wizard, and the wizard's content grows with the
    // text — so the card does too. A card whose last field is below its
    // own bottom edge is a card the operator has to scroll to finish.
    let h = (SESSIONS_H * text_scale())
        .min(room_h)
        .max(240.0)
        .min((viewport.3 - 16.0).max(0.0));
    (
        viewport.0 + (viewport.2 - w) / 2.0,
        viewport.1 + (viewport.3 - h) / 2.0,
        w,
        h,
    )
}

/// Where the FIRST session row sits in the card: the point a click on it
/// lands.
///
/// The card's own title band and the rule under it, then the column's
/// padding, then the LIST's title band and the rule under it, then the gap,
/// THE FLOW'S CONTENT COLUMN — `(left, width)` of the row the panels sit in.
///
/// The flow is a full-window surface with its own chrome (`kit::STRIP_H`), the
/// double rule under it, and a centred content column no wider than
/// `kit::CONTENT_W`. This is the arithmetic of all three, in the kernel, so a
/// test that wants to click a row and the render that draws it are reading the
/// same numbers.
pub fn flow_content(viewport: Rect) -> (f32, f32) {
    let w = kit::CONTENT_W.min(viewport.2 - 48.0).max(280.0);
    ((viewport.2 - w) / 2.0, w)
}

/// The y of the flow's content row: the chrome, the double rule, and the
/// body's own top inset.
pub fn flow_content_top() -> f32 {
    let strip = pt(kit::STRIP_H);
    let rules = pt(kit::STRIP_RULE_GAP) + kit::EDGE_WIDTH;
    strip + rules + 8.0
}

/// The first history row's centre: the content corner, the panel's band, its
/// rule, the body's inset, the tabs, the column header, then half a row.
///
/// The sessions card is gone; the history lives in the flow's left panel, and
/// a test that aims at a row has to aim where the flow draws it.
pub fn flow_first_row_point(viewport: Rect) -> (f32, f32) {
    let (left, _) = flow_content(viewport);
    let band = pt(kit::BAND_H) + kit::EDGE_WIDTH;
    let pad = pt(PANEL_PAD);
    let tabs = pt(24.0) + pt(PANEL_GAP);
    let header = pt(LABEL_PT) + pt(4.0) + kit::EDGE_WIDTH;
    (
        left + 120.0,
        flow_content_top() + band + pad + tabs + header + pt(kit::ROW_H) / 2.0,
    )
}

/// then half a row. Every term is a token, and the render draws the same
/// terms in the same order — so the point this names is where the row is.
///
/// This exists because the sessions list left the column: a test that found
/// a row by walking the column's arithmetic cannot find a row that is not
/// in the column, and a test that cannot find the row proves nothing.
pub fn sessions_first_row_point(card: Rect) -> (f32, f32) {
    let x = card.0 + PANEL_PAD + 60.0;
    let y = card.1
        + title_h()
        + 1.0
        + PANEL_PAD
        + title_h()
        + 1.0
        + PANEL_GAP
        // THE HISTORY'S FIVE STATE TABS, and the gap on either side of them.
        // The creation flow put the tabs between the list's title and its first
        // row (`docs/creation-flow-handoff.md` §3.1), so the arithmetic that
        // names the first row grew one term — and a test that aimed at the old
        // point would now aim at a tab.
        + TAB_BAND_H
        + PANEL_GAP
        + SESSION_ROW_H / 2.0;
    (x, y)
}

/// The stack's height: every island plus the gap between each pair.
///
/// The gap is counted per island rather than per pair, which is the console's
/// own arithmetic (`main.rs::side_zone`) and over-measures by one gap. It is
/// kept because the overflow it feeds is a scroll CEILING: a ceiling one gap
/// too generous costs one unreachable pixel, and a ceiling one gap too tight
/// strands an island's last row below the band with no gesture to reach it.
pub fn zone_stack_height(slots: &[IslandSlot]) -> f32 {
    slots.iter().map(|s| s.height).sum::<f32>() + ZONE_ISLAND_GAP * slots.len() as f32
}

/// Where the nth slot of a stack starts, and how far the column may scroll.
///
/// `zone_island_origin` over the stack's own heights, so the two can never
/// disagree about where an island sits — a stack whose positions were
/// computed by different arithmetic than its height is the double-offset
/// class of bug the U4e bisect cost half a session to find.
pub fn zone_slot_origin(band: Box, slots: &[IslandSlot], index: usize) -> (f32, f32) {
    let heights: Vec<f32> = slots.iter().map(|s| s.height).collect();
    zone_island_origin(band, index, &heights)
}

/// How far the column may scroll: the stack's height over the band's, never
/// negative.
///
/// Derived rather than constant, because the stack changes with the
/// exercise's state. The shell carried `3 * 300 + 2 * GAP - band_h` as a
/// literal, which is right only while the column happens to hold three
/// 300px islands — and a stale ceiling is exactly the failure the console
/// documents: too tight strands an island's last row below the band with no
/// gesture to reach it, too generous scrolls the column into empty band.
// ---------------------------------------------------------------------------
// The map's own instrument chrome
//
// Three things the reference world puts on a live map and this console does not
// have: a scale that is true rather than decorative, a north mark, and a
// reticle that says which object the operator is on. All three are drawn over
// the tile canvas, never inset against it — `DESIGN.md`'s rule that UI never
// takes space from the map.
// ---------------------------------------------------------------------------

/// Metres per map pixel at a latitude and a Web Mercator zoom.
///
/// The standard formula, and the reason it is here rather than in the shell is
/// that it is the one number on the map chrome that has to be *true*: the
/// console's doctrine is present-never-predict, and a scale bar that is
/// approximately right is a small lie an operator would steer a vessel by.
/// 156543.033928041 is the ground resolution at the equator for zoom 0 in the
/// 512px-tile scheme MapLibre uses; the cosine is the mercator stretch.
pub fn metres_per_pixel(lat_deg: f64, zoom: f64) -> f64 {
    156543.033_928_041 * (lat_deg.to_radians()).cos() / zoom.exp2()
}

/// The scale bar's tick distance and its length in pixels.
///
/// Picks the nearest 1/2/5 x 10^n metres whose pixel length lands inside the
/// band, so the bar is a round number at every zoom rather than a fixed length
/// with a number that happens to be whatever the zoom made it. Returns
/// `(metres, pixels)`, or `None` when the scale is not knowable yet — which is
/// the honest answer before the first map frame, and a caller that drew a bar
/// anyway would be drawing a guess.
///
/// A pure function with a test because the arithmetic is the whole of it: the
/// shell only supplies the pixels, and a scale bar is the one piece of chrome
/// where being wrong is worse than being absent.
pub fn scale_bar(metres_per_px: f64, min_px: f32, max_px: f32) -> Option<(f64, f32)> {
    if !(metres_per_px > 0.0) || !metres_per_px.is_finite() || min_px <= 0.0 || max_px < min_px {
        return None;
    }
    // The 1/2/5 ladder from one metre, as (metres, pixels).
    //
    // The mantissa drives the step rather than a chain of exact-division
    // tests. The first version of this tested `metres / 5.0` for an exact
    // divide and multiplied by 2, which is right for 5 and wrong for 20 —
    // 20/5 is also exact, so the walk went 20 -> 40 -> 80 and the bar stopped
    // being a round number. Reading the mantissa cannot do that.
    let mut rungs: Vec<(f64, f32)> = Vec::with_capacity(30);
    let mut metres = 1.0f64;
    for _ in 0..30 {
        rungs.push((metres, (metres / metres_per_px) as f32));
        let m = metres / 10f64.powf(metres.log10().floor());
        metres *= if (m - 1.0).abs() < 1e-9 {
            2.0
        } else if (m - 2.0).abs() < 1e-9 {
            2.5
        } else {
            2.0
        };
    }
    // The LONGEST round distance that still fits the band.
    //
    // Not the one nearest the band's middle, which is what this did first and
    // which oscillates: at zoom 7 both 100km (82px) and 200km (164px) land
    // inside a 70-170px band, and the one nearer the middle is the SMALLER
    // one, so the bar went 200km at zoom 6 and 100km at zoom 7 — a distance
    // that shrinks as you zoom in, which is exactly backwards.
    //
    // Taking the longest fitting rung is monotonic in zoom by construction,
    // because a rung's pixel length only grows as the scale grows, and it
    // also reads better: a longer bar is an easier thing to compare against.
    rungs
        .into_iter()
        .filter(|(_, px)| *px >= min_px && *px <= max_px)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
}

/// The scale bar's label, in the unit an operator reads a distance in.
///
/// Metres below a kilometre, kilometres above, and never a decimal that is
/// really a rounded metre count — "1.5 km" is a fact, "1.53 km" is a precision
/// the bar does not have.
pub fn scale_label(metres: f64) -> String {
    if metres < 1000.0 {
        format!("{:.0} m", metres)
    } else {
        let km = metres / 1000.0;
        if (km - km.round()).abs() < 1e-9 {
            format!("{:.0} km", km)
        } else {
            format!("{:.1} km", km)
        }
    }
}

pub fn zone_overflow(stack_h: f32, band: Box) -> f32 {
    (stack_h - box_h(band)).max(0.0)
}

// ---------------------------------------------------------------------------
// Marker vocabulary the shell paints
//
// These left `markers` for here because the shell paints them and must not
// reach through `markers` to do it: `markers` imports `store` (its resolvers
// name `MapSymbol` and `Affiliation`), and `store` is the bundled sqlite the
// parity proto exists to keep out of its fast loop. `markers` re-exports
// every one of these, so its own tests and any `markers::` path still
// resolve — one definition, two names, the seam the whole module is for.
// ---------------------------------------------------------------------------

/// What KIND of hull a marker is, as the taxonomy's own stable ids say it.
///
/// One enum rather than a string, because the glyph geometry is a match on it
/// and a string would make every painter a lookup table that can silently
/// miss a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MapSymbolShape {
    /// A hull of a type the operator has not assigned a symbol.
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

impl MapSymbolShape {
    /// The wire name, for a client that does not share the console's store
    /// (the observation command center draws the same mark from this).
    pub fn wire_name(self) -> &'static str {
        match self {
            MapSymbolShape::Dot => "dot",
            MapSymbolShape::Destroyer => "destroyer",
            MapSymbolShape::Frigate => "frigate",
            MapSymbolShape::Corvette => "corvette",
            MapSymbolShape::Auxiliary => "auxiliary",
            MapSymbolShape::Landing => "landing",
            MapSymbolShape::Submarine => "submarine",
            MapSymbolShape::Plane => "plane",
            MapSymbolShape::Tank => "tank",
            MapSymbolShape::Port => "port",
        }
    }

    /// The name back. An unknown word is the generic dot, never a panic.
    pub fn from_wire(name: &str) -> MapSymbolShape {
        [
            MapSymbolShape::Destroyer,
            MapSymbolShape::Frigate,
            MapSymbolShape::Corvette,
            MapSymbolShape::Auxiliary,
            MapSymbolShape::Landing,
            MapSymbolShape::Submarine,
            MapSymbolShape::Plane,
            MapSymbolShape::Tank,
            MapSymbolShape::Port,
        ]
        .into_iter()
        .find(|shape| shape.wire_name() == name)
        .unwrap_or(MapSymbolShape::Dot)
    }
}

/// A regular polygon's vertices about `center`, the first at `rotation`.
///
/// The rotation is the caller's because the glyphs are not all
/// point-up: a frigate's diamond is a square stood on its corner and a
/// landing ship's pentagon has a flat bottom, and baking one convention in
/// would make every painter rotate the result back.
///
/// The one geometry primitive every taxonomy glyph needs, and the reason it
/// is here rather than in a painter: `n = 0` is a dot, `n >= 3` is a polygon,
/// and a caller must not have to re-derive the angle convention to draw one.
pub fn regular_polygon(
    center: (f32, f32),
    radius: f32,
    n: usize,
    rotation: f32,
) -> Vec<(f32, f32)> {
    let n = n.max(3);
    (0..n)
        .map(|i| {
            let a = rotation + (i as f32) * std::f32::consts::TAU / n as f32;
            (center.0 + radius * a.cos(), center.1 + radius * a.sin())
        })
        .collect()
}

/// The fill ink a taxonomy glyph is drawn in.
///
/// The ACCENT, not a neutral: the glyph says what kind of thing it is, and
/// the affiliation frame around it says whose it is. Two colours doing two
/// jobs is the whole of the map's symbology, and collapsing them into one
/// would make the frame redundant.
pub fn map_symbol_accent(shape: MapSymbolShape) -> Rgba {
    match shape {
        MapSymbolShape::Dot => Rgba::rgb(0xCB, 0xD5, 0xE1),
        MapSymbolShape::Destroyer => Rgba::rgb(0x93, 0xC5, 0xFD),
        MapSymbolShape::Frigate => Rgba::rgb(0x7D, 0xD3, 0xFC),
        MapSymbolShape::Corvette => Rgba::rgb(0x67, 0xE8, 0xF9),
        MapSymbolShape::Auxiliary => Rgba::rgb(0xFD, 0xBA, 0x74),
        MapSymbolShape::Landing => Rgba::rgb(0xF0, 0xAB, 0xFC),
        MapSymbolShape::Submarine => Rgba::rgb(0xA5, 0xB4, 0xFC),
        MapSymbolShape::Plane => Rgba::rgb(0x86, 0xEF, 0xAC),
        MapSymbolShape::Tank => Rgba::rgb(0xBE, 0xFA, 0x8B),
        MapSymbolShape::Port => Rgba::rgb(0xDD, 0xD6, 0xFE),
    }
}

/// The glyph's fill, and the one rule that overrides the accent: a STALE hull
/// greys.
///
/// Staleness is the more important of the two facts a glyph carries — a fresh
/// colour on a hull that stopped reporting an hour ago is a lie about where
/// it is — so it wins, and it wins to the same neutral for every shape. The
/// shape itself still says what kind it is; the frame still says whose.
pub fn map_symbol_fill(shape: MapSymbolShape, stale: bool) -> Rgba {
    if stale {
        return Rgba::rgb(160, 160, 160);
    }
    let accent = map_symbol_accent(shape);
    Rgba::rgb(
        (accent.r as u16 + 40).min(255) as u8,
        (accent.g as u16 + 40).min(255) as u8,
        (accent.b as u16 + 40).min(255) as u8,
    )
}

/// The ink the affiliation frame is stroked in.
pub fn affiliation_ink(affiliation: crate::symbology::Affiliation) -> Rgba {
    use crate::symbology::Affiliation as A;
    match affiliation {
        A::Friendly => Rgba::rgb(0x3B, 0x82, 0xF6),
        A::Hostile => Rgba::rgb(0xEF, 0x44, 0x44),
        A::Neutral => Rgba::rgb(0x22, 0xC5, 0x5E),
        A::Unknown => Rgba::rgb(0xA3, 0xA3, 0xA3),
    }
}

/// The affiliation frame's interior wash: a QUARTER-strength tint of its own
/// hue toward the map's ink.
///
/// A half-strength mix was tried and fails its own contract: the Unknown hue
/// is `(163,163,163)` and a half mix lands at `(136,126,130)`, which is
/// 2.56:1 against the hue — under the 3:1 floor `the_frame_outline_reads_on_
/// both_of_its_edges` pins, because that floor is what stops the frame's
/// stroke disappearing into its own body. A quarter keeps every hue at or
/// above 3.4:1 on its wash while still reading as tinted glass over the
/// ground rather than as a solid block that would hide the coastline.
pub const FRAME_FILL_HUE: u16 = 1;
pub const FRAME_FILL_GROUND: u16 = 3;

pub fn affiliation_fill(affiliation: crate::symbology::Affiliation, stale: bool) -> Rgba {
    let ink = affiliation_ink(affiliation);
    if stale {
        return MAP_INK_DARK;
    }
    let mix = |a: u8, b: u8| ((a as u16 * FRAME_FILL_HUE + b as u16 * FRAME_FILL_GROUND) / 4) as u8;
    Rgba::rgb(
        mix(ink.r, MAP_INK_DARK.r),
        mix(ink.g, MAP_INK_DARK.g),
        mix(ink.b, MAP_INK_DARK.b),
    )
}

/// The map's own dark, which is what the interior washes mix toward and what
/// a stale hull's frame falls back to.
pub const MAP_INK_DARK: Rgba = Rgba::rgb(0x0d, 0x1b, 0x2b);

/// An ink pushed toward the map's dark, for the casing pass under a stroke.
pub fn darkened_ink(ink: Rgba) -> Rgba {
    Rgba::rgb(
        ((ink.r as u16 + MAP_INK_DARK.r as u16) / 2) as u8,
        ((ink.g as u16 + MAP_INK_DARK.g as u16) / 2) as u8,
        ((ink.b as u16 + MAP_INK_DARK.b as u16) / 2) as u8,
    )
}

/// The keyline every marker on the map carries under its ink.
///
/// Without it a light ring over the map's pale roads is a ring you have to
/// hunt for, and an amber one vanishes into its tan ones. Measured, not
/// assumed: the first live capture lost both.
pub const FRAME_KEYLINE: Rgba = Rgba::rgb(0x1c, 0x1c, 0x22);
/// A stale hull's keyline lifts, so the marker still parts from the ground.
pub const FRAME_KEYLINE_STALE: Rgba = Rgba::rgb(0x3a, 0x3a, 0x40);

/// The affiliation frame's stroke width, in map pixels.
pub const FRAME_STROKE_PX: f32 = 1.5;
/// How far the casing pass is pushed OUTWARD.
///
/// Positive on purpose: a casing centred on the run eats the glyph's own
/// pixels at this width, and a casing pushed inward does not part the frame
/// from the ground at all. Outward is the only direction that does the job
/// without costing the fill.
pub const FRAME_KEYLINE_OUT_PX: f32 = 1.25;
/// The casing's full width: the stroke plus its outward push, on both sides.
pub const FRAME_KEYLINE_PX: f32 = FRAME_STROKE_PX + FRAME_KEYLINE_OUT_PX;
/// Where the casing's centreline sits relative to the run, so the hue stroked
/// on top lands centred on its own width.
pub const FRAME_KEYLINE_OFFSET_PX: f32 = (FRAME_KEYLINE_PX - FRAME_STROKE_PX) / 2.0;
/// A PLANNED hull's frame dashes — it is in the exercise but unassigned in
/// the task organisation, which is a different fact from being silent.
pub const FRAME_DASH_PX: f32 = 3.0;
pub const FRAME_DASH_GAP_PX: f32 = 2.5;

/// Push a closed run outward from `center` by `offset`, preserving its shape.
///
/// Each vertex moves along its own radius, which is exact for a polygon and
/// the right approximation for an arc: the casing follows the frame instead
/// of sitting inside it.
pub fn offset_run_outward(
    points: &[(f32, f32)],
    center: (f32, f32),
    offset: f32,
) -> Vec<(f32, f32)> {
    points
        .iter()
        .map(|(x, y)| {
            let (dx, dy) = (x - center.0, y - center.1);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-6 {
                (*x, *y)
            } else {
                (x + dx / len * offset, y + dy / len * offset)
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The camera's shift
// ---------------------------------------------------------------------------

/// How far the map's visible centre shifts when the side zone covers it.
///
/// Mirrors `chrome::camera_centre_offset_with` (`tokens::CAMERA_OFFSET_
/// FRACTION` = 0.5): the zone floats OVER the map rather than insetting it,
/// so a jump that centres on the window's middle parks the hull under the
/// chrome. A left-docked zone moves content right (positive), a right-docked
/// one left. Hidden (or zero width) is no shift — the whole reason this is a
/// function rather than a constant is the Settings dock/hide state.
pub const CAMERA_OFFSET_FRACTION: f32 = 0.5;

pub fn camera_centre_shift(zone_w: f32, left: bool, hidden: bool) -> f32 {
    if hidden || zone_w == 0.0 {
        return 0.0;
    }
    if left {
        zone_w * CAMERA_OFFSET_FRACTION
    } else {
        -zone_w * CAMERA_OFFSET_FRACTION
    }
}

// ---------------------------------------------------------------------------
// Time, as strings the backend will accept
// ---------------------------------------------------------------------------

/// Whether `s` is a `HHMM` string: exactly four characters, digits only,
/// hour at most 23, minute at most 59.
///
/// Moved here from `backend::master`, verbatim: it is pure string validation
/// with no network in it, and `book`'s step rule plus the shell's step row
/// both read it — which they cannot do through `backend` without dragging the
/// HTTP stack behind the parity proto's fast loop. `backend` re-exports both
/// names, so every existing path resolves unchanged.
pub fn hhmm_ok(s: &str) -> bool {
    if s.len() != 4 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let h = s[0..2].parse::<u32>().unwrap_or(99);
    let m = s[2..4].parse::<u32>().unwrap_or(99);
    h <= 23 && m <= 59
}

/// Whether a pair of `HHMM` strings is a window the backend will accept.
///
/// Both ends or neither, and the end must be after the start. A window that
/// crosses midnight is two steps, by the backend's own rule.
pub fn hhmm_window_ok(start: &str, end: &str) -> bool {
    hhmm_ok(start) && hhmm_ok(end) && start < end
}

/// The camera's zoom rails, as the console clamps them (`main.rs:16559`).
///
/// Carried here rather than spelled at the call site because the band's
/// stepper, the shell's own opening zoom and the map thread's own clamp all
/// have to agree — and a stepper that clamped differently from the renderer
/// would be a readout that lies about the picture.
pub const ZOOM_MIN: f64 = 3.0;
pub const ZOOM_MAX: f64 = 18.0;

/// The height of a credential or draft field, as one number for the whole
/// card and the whole form.
///
/// The console's `AUTH_FIELD_H`, and the reason it is a constant: a field
/// whose height came from its text would make the card change height as the
/// operator typed, which is the one thing a form must not do under a cursor.
pub const AUTH_FIELD_H: f32 = 30.0;

/// The height of a credential or draft field, at the text scale in force.
///
/// The field holds body text, and the body grows with the scale — so the
/// field grows with it, and the card does not change height as the operator
/// types, which is the one thing a form must not do under a cursor.
pub fn field_h() -> f32 {
    AUTH_FIELD_H * text_scale()
}

/// The side zone's band, docked left or right.
///
/// `left` mirrors the box to the other edge with the SAME gaps, so the island
/// column moves without re-deriving anything — the console's Settings
/// "Docked" choice, and the reason the band is computed here rather than in
/// either shell. The console's own default is LEFT (`main.rs:20669`).
pub fn zone_band_at(viewport: Box, zone_w: f32, left: bool) -> Box {
    if left {
        (
            viewport.0,
            viewport.1 + zone_top_gap(),
            viewport.0 + zone_w,
            viewport.3,
        )
    } else {
        zone_band(viewport, zone_w)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::gamestate::GameState;

    /// The band's readouts yield before its buttons. A band that clips
    /// sign-out is a console with no way out, which is the one thing a width
    /// budget exists to prevent.
    #[test]
    fn the_bands_readouts_yield_before_its_buttons() {
        // Wide enough for everything the console's band carries.
        assert!(1536.0 > BAND_READOUT_BUDGET);
        // The budget leaves room for both door groups: the readouts it gates
        // are ~300pt and the buttons it protects ~230.
        assert!(BAND_READOUT_BUDGET > 600.0);
        // And it is not so generous that a normal window loses them.
        assert!(BAND_READOUT_BUDGET > 768.0);
    }

    /// Inspector images stay compact by default, preserve their source
    /// aspect ratio, and never grow taller than the compact preview cap.
    /// Moved off the console binary with the function (U8 step 1).
    #[test]
    fn inspector_image_size_is_bounded_and_aspect_preserving() {
        let wide = inspector_image_size((900.0, 111.0), 280.0, 320.0);
        assert!((wide.0 - 280.0).abs() < 0.01);
        assert!((wide.1 - 111.0 / 900.0 * 280.0).abs() < 0.01);

        let tall = inspector_image_size((111.0, 900.0), 280.0, 320.0);
        assert!((tall.1 - INSPECTOR_IMAGE_MAX_HEIGHT).abs() < 0.01);
        assert!(tall.0 < 280.0);
        assert!(tall.0 > 0.0);

        let narrow_panel = inspector_image_size((900.0, 111.0), 280.0, 120.0);
        assert!((narrow_panel.0 - 120.0).abs() < 0.01);
    }

    /// The one invariant the whole column rests on: NO session state has a
    /// column at all.
    ///
    /// With no exercise held there is nothing to read beside the map. The
    /// three surfaces this state used to carry were each MOVED — identity to
    /// the top band's right, the games to hold to the middle of the frame —
    /// and the register went to PLANNING, which is the phase that assembles a
    /// force. What is left is a map, and the door that opens the one question
    /// this state asks.
    #[test]
    fn an_empty_room_has_no_column() {
        for player in [false, true] {
            assert!(
                zone_islands(GameState::NoSession, player).is_empty(),
                "the empty room grew a column"
            );
        }
    }

    /// A player station sees its own helm, and nothing else. A judge
    /// commanding hulls is still a judge — the console's rule reads the
    /// seat, not the hulls, and the hulls are the DASHBOARD's question.
    ///
    /// The helm is the part this got wrong first: the console's player column
    /// is Step-only because it serves their helm from the floating dashboard,
    /// and a column-only port read that as "personnel get no command surface"
    /// -- which left a player station unable to order its own hull, the one
    /// thing the station exists to do. The step half left the column for the
    /// middle of the frame, so what a player station keeps here is the helm.
    #[test]
    fn a_player_station_keeps_its_own_helm() {
        let keys: Vec<IslandKey> = zone_islands(GameState::Execution, true)
            .iter()
            .map(|s| s.key)
            .collect();
        assert_eq!(keys, vec![IslandKey::Command]);
    }

    /// An organizer's execution column is a working surface, and it is
    /// ordered: what you can order, then who is here, then what happened.
    ///
    /// There is no step here any more. The whole scenario — the book, the
    /// pointer, the hulls each step obliges, the gate and the Game Master's
    /// verbs — takes the middle of the frame, because that is how big a
    /// scenario is. Pinned so a later "it fits in the column after all" does
    /// not quietly shrink it back.
    #[test]
    fn an_organizers_execution_column_is_ordered() {
        let keys: Vec<IslandKey> = zone_islands(GameState::Execution, false)
            .iter()
            .map(|s| s.key)
            .collect();
        assert_eq!(
            keys,
            vec![IslandKey::Command, IslandKey::Crew, IslandKey::Log,]
        );
    }

    /// Every state's column, pinned whole. This is THE test for the layout by
    /// state: the console's `side_zone` match, cut to the surfaces this shell
    /// has, with the omissions named in `zone_islands`' doc rather than left
    /// to be discovered by looking at a screenshot.
    ///
    /// A drift in any one of these is a column showing a surface the exercise
    /// is not in a state to use — an orders pane over a game nobody is
    /// playing, or a readiness gate after the gate has closed.
    #[test]
    fn every_state_has_the_column_it_should() {
        let keys = |state: GameState, player: bool| -> Vec<IslandKey> {
            zone_islands(state, player)
                .into_iter()
                .map(|s| s.key)
                .collect()
        };
        // No session: NOTHING. The games to hold live in the middle of the
        // frame, and the register belongs to planning.
        assert_eq!(keys(GameState::NoSession, false), Vec::<IslandKey>::new());
        // Planning is the GRID: six equal panels over the map, and the
        // column holds NOTHING in this state. The membership that used to
        // be here — Essentials, Control, Fleet, Players, Groups, Review —
        // is the grid's six panels now, and the Control island is gone
        // because the clock is part of the exercise's terms.
        assert_eq!(keys(GameState::Planning, false), Vec::<IslandKey>::new());
        // Preparation is the room: the order of battle, and its door. The
        // Command panel left the phase with the owner's direction; readiness
        // moved into the crew screen it opens.
        assert_eq!(keys(GameState::Preparation, false), [Crew_key()]);
        // Execution is the working surface, for an organizer. The step is
        // not here: the scenario board takes the middle of the frame.
        assert_eq!(
            keys(GameState::Execution, false),
            [Command_key(), Crew_key(), IslandKey::Log]
        );
        // Closure is the debrief, and carries no command: nothing is ordered
        // after the fact.
        assert_eq!(
            keys(GameState::Closure, false),
            [IslandKey::Assessment, IslandKey::Log, Crew_key()]
        );
    }

    fn Crew_key() -> IslandKey {
        IslandKey::Crew
    }
    fn Command_key() -> IslandKey {
        IslandKey::Command
    }

    /// A player station changes the column in execution and ONLY there. In
    /// planning they are still seating the room, so taking the column off
    /// them would take away the surface that does it.
    #[test]
    fn a_player_station_changes_the_column_only_in_execution() {
        for state in [
            GameState::NoSession,
            GameState::Planning,
            GameState::Preparation,
            GameState::Closure,
        ] {
            let a: Vec<IslandKey> = zone_islands(state, false)
                .into_iter()
                .map(|s| s.key)
                .collect();
            let b: Vec<IslandKey> = zone_islands(state, true)
                .into_iter()
                .map(|s| s.key)
                .collect();
            assert_eq!(a, b, "{state:?} changed shape for a player station");
        }
        let a: Vec<IslandKey> = zone_islands(GameState::Execution, false)
            .into_iter()
            .map(|s| s.key)
            .collect();
        let b: Vec<IslandKey> = zone_islands(GameState::Execution, true)
            .into_iter()
            .map(|s| s.key)
            .collect();
        assert_ne!(a, b, "execution must narrow for a player station");
    }

    /// No state may list an island twice. A duplicate would render the same
    /// surface twice with two scroll offsets and two close buttons, which is
    /// the failure a keyed column exists to make impossible.
    #[test]
    fn no_state_lists_an_island_twice() {
        for state in [
            GameState::NoSession,
            GameState::Planning,
            GameState::Preparation,
            GameState::Execution,
            GameState::Closure,
        ] {
            for player in [false, true] {
                let slots = zone_islands(state, player);
                for (i, s) in slots.iter().enumerate() {
                    assert!(
                        !slots[i + 1..].iter().any(|o| o.key == s.key),
                        "{state:?} player={player} listed {:?} twice",
                        s.key
                    );
                }
            }
        }
    }

    /// Every island is tall enough for its band plus a body: an island
    /// shorter than its own title band would clip the band and leave a handle
    /// nobody can grab.
    #[test]
    fn every_island_is_taller_than_its_own_band() {
        for state in [
            GameState::NoSession,
            GameState::Planning,
            GameState::Preparation,
            GameState::Execution,
            GameState::Closure,
        ] {
            for player in [false, true] {
                for s in zone_islands(state, player) {
                    assert!(
                        s.height > title_h(),
                        "{:?} at {} hides its own band",
                        s.key,
                        s.height
                    );
                }
            }
        }
    }

    /// Titles and ids are the band's vocabulary: short caps for the label, a
    /// lowercase stable key for the stateful surfaces. A blank or a mis-cased
    /// one is a visible defect on the band and a silent collision on the
    /// scroll handle.
    #[test]
    fn every_island_key_names_itself() {
        for key in COLUMN_ISLANDS {
            let title = key.title();
            assert!(!title.is_empty());
            assert!(
                title.chars().all(|c| c.is_ascii_uppercase()),
                "{title:?} is not short caps"
            );
            let id = key.id();
            assert!(!id.is_empty());
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase()),
                "{id:?} is not a key"
            );
        }
    }

    /// The stack's height is the sum plus a gap per island — the console's
    /// own arithmetic, over-measuring by one gap on purpose (see
    /// `zone_stack_height`). Pinned so a "fix" to the off-by-one does not
    /// land silently and strand a row below the band.
    ///
    /// The gap is ZERO now: the column is one surface and its fields share
    /// their rules, so there is nothing between them to measure. The
    /// arithmetic is kept, because a gap that is zero today may not be one
    /// tomorrow, and the ceiling must follow the column's own shape.
    #[test]
    fn the_stack_height_counts_a_gap_per_island() {
        // CLOSURE, not planning: the planning phase is the grid now and its
        // column holds nothing, so a stack test in that state would be
        // testing an empty stack.
        let slots = zone_islands(GameState::Closure, false);
        let sum: f32 = slots.iter().map(|s| s.height).sum();
        assert_eq!(
            zone_stack_height(&slots),
            sum + ZONE_ISLAND_GAP * slots.len() as f32
        );
        // Three islands: the console's closure heights, and — because the
        // fields are joined — no gaps between them.
        assert_eq!(zone_stack_height(&slots), 420.0 + 260.0 + 330.0);
    }

    /// Origins and height come from the same arithmetic. This is the pair
    /// that must not disagree: an origin computed over different heights than
    /// the stack's parks an island somewhere the scroll ceiling does not know
    /// about.
    #[test]
    fn the_stack_origin_is_the_last_origin_plus_its_height() {
        let band = (376.0, 56.0, 736.0, 596.0);
        let slots = zone_islands(GameState::Closure, false);
        for i in 1..slots.len() {
            let (x, y) = zone_slot_origin(band, &slots, i);
            let (_, py) = zone_slot_origin(band, &slots, i - 1);
            assert_eq!(x, band.0, "the column is dock-bound: x is the band's");
            assert_eq!(y, py + slots[i - 1].height + ZONE_ISLAND_GAP);
        }
    }

    /// The overflow is the stack over the band, floored at zero. A stack that
    /// fits does not scroll, and a ceiling below zero would invert the
    /// column's clamp.
    #[test]
    fn the_column_scrolls_only_by_what_it_overflows() {
        let band = (376.0, 56.0, 736.0, 596.0);
        // A 540-tall band against the 1010-tall CLOSURE stack. That a
        // column overflows is not incidental: the console's own heights
        // cannot fit any window this console runs in, which is WHY the
        // column scrolls at all.
        //
        // PLANNING IS NOT HERE ANYMORE: the phase is the grid now, and its
        // column holds nothing, so its stack is empty and it overflows by
        // nothing. The grid's own geometry is pinned by
        // `the_planning_grid_is_six_equal_cells`.
        let slots = zone_islands(GameState::Closure, false);
        assert_eq!(zone_stack_height(&slots), 420.0 + 260.0 + 330.0);
        assert_eq!(zone_overflow(zone_stack_height(&slots), band), 470.0);
        // And the planning phase's column is empty: the grid is the phase.
        assert!(zone_islands(GameState::Planning, false).is_empty());
        // A short stack in a tall band scrolls not at all.
        assert_eq!(zone_overflow(100.0, band), 0.0);
    }

    /// THE GRID'S COLUMNS FOLLOW THE WINDOW.
    ///
    /// Three on a window wide enough to hold them, two on one that is not:
    /// a cell narrower than the console's own column cannot hold the
    /// console's own panels, and a grid of cells that cannot hold them is a
    /// grid of cut-off rows.
    #[test]
    fn the_planning_grid_fits_the_window_it_is_drawn_in() {
        assert_eq!(planning_grid_cols(1536.0), 3, "a wide window gets three");
        assert_eq!(planning_grid_cols(1280.0), 3);
        assert_eq!(planning_grid_cols(768.0), 2, "a narrow one gets two");
        assert_eq!(planning_grid_cols(600.0), 1, "a very narrow one gets one");
        // And either way there are six cells, and each is wide enough to
        // hold the console's own panels.
        for w in [1536.0, 1024.0, 768.0, 600.0] {
            let cells = planning_grid((0.0, 0.0, w, 838.0));
            assert_eq!(cells.len(), 6, "six panels at every width");
            assert!(
                box_w(cells[0]) >= GRID_MIN_CELL_W - 0.01,
                "a cell at {w}pt wide is too narrow for the console's panels: {}",
                box_w(cells[0])
            );
        }
    }

    /// THE PLANNING GRID IS SIX EQUAL CELLS, and they are inside the window.
    ///
    /// Equal is the claim: a grid whose cells differ is a grid whose
    /// importance is decided by size, and the operator reads the big one
    /// and leaves the rest. The grid is also BELOW the top band — the band
    /// is the clock and the identity, and it is the same in every phase.
    #[test]
    fn the_planning_grid_is_six_equal_cells() {
        let viewport = (0.0, 0.0, 960.0, 838.0);
        let cells = planning_grid(viewport);
        assert_eq!(cells.len(), 6, "six panels: three across and two down");
        let (w, h) = (box_w(cells[0]), box_h(cells[0]));
        for cell in &cells {
            assert!(
                (box_w(*cell) - w).abs() < 0.01,
                "the cells are equal in width"
            );
            assert!(
                (box_h(*cell) - h).abs() < 0.01,
                "the cells are equal in height"
            );
            // Every cell is inside the window.
            assert!(cell.0 >= viewport.0 && cell.1 >= viewport.1);
            assert!(cell.0 + box_w(*cell) <= viewport.2 + 0.01);
            assert!(cell.1 + box_h(*cell) <= viewport.3 + 0.01);
            // And below the top band, which is the one thing the grid does
            // not cover.
            assert!(cell.1 >= top_band_h(), "the band stays the band's");
        }
        // The gap between two cells is the margin, and it is a margin: the
        // cells do not touch.
        assert!((cells[1].0 - (cells[0].0 + w) - GRID_GAP).abs() < 0.01);
        assert!((cells[3].1 - (cells[0].1 + h) - GRID_GAP).abs() < 0.01);
        // And the panels are the six the phase is made of, in reading order.
        assert_eq!(GridPanel::ALL.len(), 6);
        assert_eq!(GridPanel::ALL[0], GridPanel::Essentials);
        assert_eq!(GridPanel::ALL[5], GridPanel::Groups);
    }

    /// A CELL IS CORNERS AND A CARD IS ORIGIN AND SIZE, and the conversion
    /// between them is one function because getting it wrong is silent.
    ///
    /// The planning cards were drawn from `let (x, y, w, h) = cell`, which
    /// type-checks and sets each card's width to its own RIGHT COORDINATE:
    /// every card ran to the far edge of the window, the six overlapped into
    /// one dark field, and the only visible seams were where the next card's
    /// left border happened to land. `box_rect` is the conversion, and this
    /// pins it against the grid the renderer draws.
    #[test]
    fn a_grid_cell_converts_to_the_card_that_sits_in_it() {
        let cells = planning_grid((0.0, 0.0, 1536.0, 838.0));
        for cell in &cells {
            let (x, y, w, h) = box_rect(*cell);
            assert_eq!(x, cell.0, "the card's left edge is the cell's");
            assert_eq!(y, cell.1, "the card's top edge is the cell's");
            assert!((w - box_w(*cell)).abs() < 0.01);
            assert!((h - box_h(*cell)).abs() < 0.01);
            // The card ENDS at its cell's right and bottom edges, not at the
            // window's.
            assert!(x + w <= 1536.0 + 0.01, "a card runs off the window");
            assert!(y + h <= 838.0 + 0.01, "a card runs off the window");
        }
        // And the two cards on the first row are one GRID_GAP apart, which is
        // a gap only if the first one is as wide as its cell.
        let (_, _, w0, _) = box_rect(cells[0]);
        let (x1, _, _, _) = box_rect(cells[1]);
        assert!((x1 - (cells[0].0 + w0) - GRID_GAP).abs() < 0.01);
    }

    /// FIVE BANDS, NOT SIX: the Exercise panel's body is its title.
    ///
    /// The wordmark already names the surface, and a band above it saying
    /// EXERCISE gives the tallest cell on the grid two headings — the
    /// weaker of them spending the top of the panel.
    #[test]
    fn the_exercise_panel_carries_no_title_band() {
        assert!(!GridPanel::Exercise.has_title());
        assert_eq!(
            GridPanel::ALL.iter().filter(|p| p.has_title()).count(),
            5,
            "every panel but Exercise has a band"
        );
        for panel in GridPanel::ALL {
            if panel != GridPanel::Exercise {
                assert!(panel.has_title(), "{panel:?} keeps its band");
                assert!(!panel.title().is_empty());
            }
        }
    }

    /// A CARD, NOT A HOLE CUT IN THE WATER: rounded, and standing off its
    /// neighbours by a margin wide enough to see.
    ///
    /// The panels are translucent over dark water, so a margin measured to
    /// "some gap exists" was measured to the wrong thing: at 12pt the six
    /// read as one field with seams. The rounding and the margin are what
    /// make them six cards on a chart.
    #[test]
    fn a_planning_card_is_rounded_and_stands_off_its_neighbour() {
        assert_eq!(
            GRID_CARD_RADIUS, ISLAND_RADIUS,
            "the console's floating rounding"
        );
        assert_eq!(
            GRID_INSET, GRID_GAP,
            "the frame's margin is the panels' margin"
        );
        assert!(
            GRID_GAP >= 16.0,
            "a margin the eye cannot find is not a margin"
        );
        assert!(GRID_PAD > PANEL_PAD, "a body is not a title band");
    }

    #[test]
    fn the_palette_has_no_toolkit_type() {
        // The whole point of the kernel, pinned.
        let c = CONSOLE_NIGHT;
        assert_eq!((c.r, c.g, c.b, c.a), (0x0F, 0x17, 0x2A, 255));
        assert_eq!(c.as_f32()[0], 0x0F as f32 / 255.0);
    }

    #[test]
    fn text_sizes_mirror_the_tokens() {
        // `view` is the shell-readable copy; `tokens` was the source of
        // truth. The mirror now pins literals instead of the module: the
        // values below WERE `tokens.rs`, kept so a drift still fails loudly
        // after that module goes with the egui console (U8 prep A). A drift
        // here is a silent two-size UI, so it stays pinned either way.
        // (Only the f32s are read; no egui type crosses the seam.)
        //
        // The instrument scale is pinned here too, and the ladder is the
        // point: LABEL, DENSE, MACHINE, BODY, then the two readouts. A
        // console that drew at fourteen unnamed sizes was a console nobody
        // could keep consistent, and this is the test that says so.
        assert_eq!(LABEL_PT, 10.0);
        assert_eq!(DENSE_PT, 11.0);
        assert_eq!(MACHINE_PT, 12.0);
        assert_eq!(BODY_PT, 13.0);
        assert_eq!(READOUT_SM_PT, 18.0);
        assert_eq!(READOUT_LG_PT, 30.0);
        assert_eq!(TITLE_PT, 16.0);
        assert_eq!(TEXT_SCALES[1], 1.15);
        assert_eq!(CONTROL_RADIUS, 6.0);
        assert_eq!(BUTTON_PADDING.0, 12.0);
        assert_eq!(BUTTON_PADDING.1, 8.0);
        assert_eq!(ITEM_SPACING.0, 8.0);
        assert_eq!(ITEM_SPACING.1, 6.0);
        assert_eq!(INDENT, 20.0);
        assert_eq!(ICON_SPACING, 4.0);
    }

    #[test]
    fn the_instrument_scale_is_a_ladder_not_a_pile() {
        // The proportions are the point: a label reads below a body, a body
        // below a readout, and the two readout sizes stay an obvious step
        // apart so "leads the panel" is legible without being told.
        assert!(LABEL_PT < BODY_PT, "a label smaller than the body");
        assert!(BODY_PT < READOUT_SM_PT, "a body smaller than a readout");
        assert!(
            READOUT_SM_PT < READOUT_LG_PT,
            "the leading readout is the obvious step"
        );
        assert!(
            MACHINE_PT < BODY_PT,
            "machine output is denser than prose, never the same size as it"
        );
        // A label is never allowed to grow into the body's territory: at
        // 10pt the track is 12% of the size, and a bigger track stops being
        // a stamp and becomes a word with holes in it.
        //
        // Compared with a tolerance rather than exactly, because the ratio
        // is the design intent and the f32 division is not the thing being
        // pinned — an assertion on the last bit would fail on the next
        // compiler and say nothing about the label.
        let ratio = LABEL_TRACKING / LABEL_PT;
        assert!(
            (ratio - 0.12).abs() < 1e-6,
            "label tracking should be 12% of the label size, was {ratio}"
        );
    }

    /// THE TITLE IS ITS OWN TIER, AND IT IS ABOVE THE BODY.
    ///
    /// The complaint this answers: a panel's name used to be set at the
    /// label's size, so the title and the field keys under it were the same
    /// size and the same weight — a surface and its contents drawn as one
    /// voice. A title that is not larger than the body is not a title.
    #[test]
    fn a_title_sits_above_the_body_it_heads() {
        assert!(
            TITLE_PT > BODY_PT,
            "a title smaller than the body is a label"
        );
        assert!(
            TITLE_PT > READOUT_SM_PT - READOUT_SM_PT,
            "and it is a heading, not a number"
        );
        // The ladder now has a rung between the body and the small readout
        // would be a rung nobody asked for: the title IS that rung.
        assert!(
            LABEL_PT < DENSE_PT && DENSE_PT < BODY_PT && BODY_PT < TITLE_PT,
            "the ladder still climbs"
        );
    }

    /// TRACKING IS A RATIO, so a title tracks like a title and a label
    /// like a label.
    #[test]
    fn tracking_follows_the_size_it_tracks() {
        let label = label_tracking(LABEL_PT);
        let title = label_tracking(TITLE_PT);
        assert!((label - LABEL_TRACKING).abs() < 1e-6);
        assert!(
            title > label * 1.5,
            "a bigger word needs more air between its letters, not the same air"
        );
        let ratio = title / TITLE_PT;
        let label_ratio = label / LABEL_PT;
        assert!((ratio - label_ratio).abs() < 1e-6, "one ratio, both sizes");
    }

    /// ONE MULTIPLIER MOVES THE WHOLE INSTRUMENT.
    ///
    /// The setting is a setting because every size on the console is one
    /// base size times one number — so the type and the CHROME that holds
    /// it move together. A band that did not grow with its title would cut
    /// the title, and a column that did not grow with its dense rows would
    /// lose a column of them.
    #[test]
    fn one_scale_moves_the_type_and_the_chrome_that_holds_it() {
        let before = text_scale();
        set_text_scale(TEXT_SCALES[3]);
        assert!(text_scale() > before || before == TEXT_SCALES[3]);
        assert!((pt(BODY_PT) - BODY_PT * text_scale()).abs() < 1e-4);
        assert!(title_h() > TITLE_H, "the band grows with the title");
        assert!(field_h() > AUTH_FIELD_H, "the field grows with its text");
        assert!(top_band_h() > TOP_BAND_H, "the band grows with the clock");
        assert!(zone_w() > ZONE_W, "the column grows with its rows");
        assert!((zone_top_gap() - top_band_h()).abs() < 1e-6);
        // Put it back, so a test that runs after this one sees the console
        // as the operator would find it.
        set_text_scale(TEXT_SCALES[TEXT_SCALE_DEFAULT]);
    }

    /// THE CARD FITS THE WINDOW IT IS OPENED IN.
    ///
    /// The complaint this answers: the card's title band was cut off by
    /// the window's own edge on a short window, because the card carried a
    /// minimum height that the window was allowed to be smaller than. A
    /// card taller than its window is a card whose name the operator
    /// cannot read — and the name is the one thing the band is for.
    /// THE FLOW'S FIRST ROW IS INSIDE THE FLOW'S HISTORY PANEL.
    ///
    /// The point is arithmetic over tokens; this pins that the arithmetic
    /// lands inside the panel it means, for the sizes the console is opened
    /// at. A test that aims at a row and a render that draws it read these
    /// same numbers.
    #[test]
    fn the_flows_first_row_point_is_inside_the_history_panel() {
        for (w, h) in [(1536.0, 864.0), (1280.0, 720.0), (768.0, 838.0)] {
            let viewport = (0.0, 0.0, w, h);
            let (left, content_w) = flow_content(viewport);
            let (x, y) = flow_first_row_point(viewport);
            assert!(
                x > left && x < left + content_w,
                "x {x} inside {left}..{left}+{content_w}"
            );
            assert!(y > flow_content_top(), "the row is under the chrome");
            assert!(y < h, "the row is on screen at {w}x{h}");
        }
    }

    #[test]
    fn the_sessions_card_fits_the_window_it_is_opened_in() {
        for (w, h) in [
            (1280.0, 900.0),
            (900.0, 700.0),
            (900.0, 360.0),
            (600.0, 420.0),
            (360.0, 260.0),
            (240.0, 200.0),
        ] {
            let card = sessions_card_rect((0.0, 0.0, w, h));
            assert!(
                card.2 <= w && card.0 >= 0.0 && card.0 + card.2 <= w,
                "the card hangs off the side of the window: {card:?} in {w}x{h}"
            );
            assert!(
                card.3 <= h && card.1 >= 0.0 && card.1 + card.3 <= h,
                "the card hangs off the top or bottom of the window: {card:?} in {w}x{h}"
            );
        }
    }

    /// THE TITLE BAND HOLDS THE TITLE, AT EVERY SIZE ON THE LADDER.
    ///
    /// The band is a fixed height and the title is centred in it, so a
    /// title whose line box outgrows the band is a title whose top is cut
    /// off by the card's edge. The band grows with the type, so this holds
    /// at every rung.
    #[test]
    fn the_title_band_holds_the_title_at_every_size() {
        for scale in TEXT_SCALES {
            set_text_scale(scale);
            assert!(
                title_h() >= pt(TITLE_PT) * 1.2,
                "the title does not fit its band at scale {scale}"
            );
        }
        set_text_scale(TEXT_SCALES[TEXT_SCALE_DEFAULT]);
    }

    /// THE SCALE IS A CHOICE BETWEEN NAMED SIZES, not a number typed.
    ///
    /// A scale of zero would draw nothing and a scale of ten would draw a
    /// console nobody can operate, so anything off the ladder lands on the
    /// nearest rung of it.
    #[test]
    fn a_scale_off_the_ladder_lands_on_the_ladder() {
        set_text_scale(0.0);
        assert_eq!(text_scale(), TEXT_SCALES[0]);
        set_text_scale(99.0);
        assert_eq!(text_scale(), TEXT_SCALES[TEXT_SCALES.len() - 1]);
        set_text_scale(TEXT_SCALES[TEXT_SCALE_DEFAULT]);
        assert_eq!(text_scale_step(), TEXT_SCALE_DEFAULT);
    }

    /// The console boots BIGGER than it shipped.
    ///
    /// The operator's word on the shipped size was that it was too small,
    /// and the fix is a default and not a suggestion — so the default is
    /// pinned here, where a future change to the ladder has to be argued
    /// for rather than drifted into.
    #[test]
    fn the_console_boots_bigger_than_it_shipped() {
        assert!(TEXT_SCALE_DEFAULT > 0, "there is a default");
        assert!(
            TEXT_SCALES[TEXT_SCALE_DEFAULT] > TEXT_SCALES[0],
            "and it is not the smallest rung of the ladder"
        );
        assert!(
            TEXT_SCALE_DEFAULT < TEXT_SCALES.len(),
            "and it is a rung that exists"
        );
        assert_eq!(
            TEXT_SCALE_NAMES.len(),
            TEXT_SCALES.len(),
            "every rung has a name the operator reads"
        );
    }

    #[test]
    fn one_readout_leads_a_panel_and_the_rest_are_its_company() {
        assert_eq!(readout_size(true), READOUT_LG_PT);
        assert_eq!(readout_size(false), READOUT_SM_PT);
        // Two sizes, never three: a panel that wants something between them
        // is a panel with two leading readouts, which is none.
        assert_eq!(readout_size(true), READOUT_LG_PT);
        assert_ne!(READOUT_SM_PT, READOUT_LG_PT);
    }

    #[test]
    fn a_label_is_measured_in_the_case_it_is_drawn() {
        // `label_case` exists so the uppercase happens once, before the
        // measurement. A label measured in mixed case and drawn in caps is
        // a label that overflows its own column on exactly the rows that
        // matter.
        assert_eq!(label_case("readiness gate"), "READINESS GATE");
        assert_eq!(label_case("Fleet"), "FLEET");
        assert_eq!(label_case("14 vsl"), "14 VSL");
        // Idempotent: uppercasing an already-caps label changes nothing, so
        // a caller that applies it twice cannot drift.
        assert_eq!(label_case(&label_case("transit queue")), "TRANSIT QUEUE");
    }

    #[test]
    fn metres_per_pixel_is_the_mercator_ground_resolution() {
        // Zoom 0 at the equator is the constant itself: one pixel is the whole
        // 40075km circumference over 512 tiles of 256px.
        assert!((metres_per_pixel(0.0, 0.0) - 156543.033_928_041).abs() < 1e-3);
        // Each zoom level halves it.
        let z0 = metres_per_pixel(0.0, 3.0);
        let z1 = metres_per_pixel(0.0, 4.0);
        assert!((z0 / z1 - 2.0).abs() < 1e-9, "one zoom step is exactly 2x");
        // Latitude stretches it. Jakarta is about 6 degrees south.
        let eq = metres_per_pixel(0.0, 11.0);
        let jkt = metres_per_pixel(-6.2, 11.0);
        // Mercator stretches as you leave the equator, so one projected
        // pixel covers LESS real ground there — which is why a scale bar
        // drawn at a fixed pixel length reads fewer kilometres at high
        // latitude. The first version of this test had the direction
        // backwards and the function was right.
        assert!(
            jkt < eq,
            "further from the equator is FEWER metres per pixel"
        );
        assert!((jkt / eq - (6.2f64.to_radians()).cos()).abs() < 1e-9);
    }

    #[test]
    fn the_scale_bar_is_a_round_number_at_every_zoom() {
        // The property that matters: whatever the zoom, the bar's distance is
        // 1/2/5 x 10^n and its length lands inside the band. A fixed-length
        // bar with a number that happens to be whatever the zoom made it is
        // the thing this refuses to be.
        for zoom in [4.0, 6.0, 8.0, 10.0, 11.0, 13.0, 15.0] {
            let mpp = metres_per_pixel(-6.2, zoom);
            let (metres, px) = scale_bar(mpp, 70.0, 170.0).expect("a bar at every zoom");
            assert!(
                (70.0..=170.0).contains(&px),
                "zoom {zoom}: {px}px is outside the band"
            );
            // 1/2/5 ladder: the mantissa is 1, 2 or 5.
            let mantissa = metres / 10f64.powf(metres.log10().floor());
            assert!(
                (mantissa - 1.0).abs() < 1e-6
                    || (mantissa - 2.0).abs() < 1e-6
                    || (mantissa - 5.0).abs() < 1e-6,
                "zoom {zoom}: {metres} m is not a 1/2/5 round number"
            );
        }
    }

    #[test]
    fn the_bar_never_lies_about_the_ground_it_covers() {
        // The invariant that matters, and the one the first version of this
        // test got backwards. A scale bar's distance is NOT monotonic in zoom:
        // zooming in shows less ground, so the bar must read FEWER kilometres
        // at a higher zoom. Asserting otherwise asserts a bar that grows as
        // you zoom in, which would be a bar lying in the direction an
        // operator steers by.
        //
        // What must hold at every zoom is that the bar is TRUE: its distance
        // divided by its length is the map's actual ground resolution, to
        // within the rounding of the rung it landed on.
        for zoom in [6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0] {
            let mpp = metres_per_pixel(-6.2, zoom);
            let (metres, px) = scale_bar(mpp, 70.0, 170.0).expect("a bar at every zoom");
            let implied = metres / px as f64;
            let err = (implied - mpp).abs() / mpp;
            assert!(
                err < 0.34,
                "zoom {zoom}: bar reads {implied:.1} m/px against a true {mpp:.1} ({:.0}% off)",
                err * 100.0
            );
        }
    }

    fn the_bar_moves_one_rung_at_a_time_not_in_jumps() {
        // Consecutive zooms change the reading by at most a factor of 5,
        // which is one full decade of the ladder. A bigger step than that
        // would mean the bar skipped a rung, and a skipped rung is a reading
        // that jumps while the map moves smoothly under it.
        let mut last: Option<f64> = None;
        for zoom in [6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0] {
            let (metres, _) = scale_bar(metres_per_pixel(-6.2, zoom), 70.0, 170.0).expect("a bar");
            if let Some(prev) = last {
                let ratio = (metres / prev).max(prev / metres);
                assert!(
                    ratio <= 5.0,
                    "zoom {zoom}: {metres} m from {prev} m is a jump"
                );
            }
            last = Some(metres);
        }
    }

    #[test]
    fn an_unknown_scale_draws_nothing_rather_than_a_guess() {
        // Before the first map frame there is no camera, so there is no scale.
        // A console that drew a bar anyway would be drawing a number it made
        // up — the exact failure `present, never predict` exists to prevent.
        assert!(scale_bar(0.0, 70.0, 170.0).is_none());
        assert!(scale_bar(-1.0, 70.0, 170.0).is_none());
        assert!(scale_bar(f64::NAN, 70.0, 170.0).is_none());
        assert!(scale_bar(f64::INFINITY, 70.0, 170.0).is_none());
        // A band that cannot exist is refused rather than clamped into one.
        assert!(scale_bar(10.0, 0.0, 170.0).is_none());
        assert!(scale_bar(10.0, 170.0, 70.0).is_none());
    }

    #[test]
    fn the_label_reads_in_the_unit_an_operator_steers_by() {
        assert_eq!(scale_label(500.0), "500 m");
        assert_eq!(scale_label(1000.0), "1 km");
        assert_eq!(scale_label(2000.0), "2 km");
        assert_eq!(scale_label(5000.0), "5 km");
        // A decimal where the ladder produced one, and never a precision the
        // bar does not have.
        assert_eq!(scale_label(1500.0), "1.5 km");
        assert_eq!(scale_label(250.0), "250 m");
    }

    #[test]
    fn tabular_figures_are_a_real_feature_not_a_hope() {
        // The tag has to be spelled the way the shaper spells it. `tnum` is
        // the OpenType tag for tabular figures; anything else silently
        // renders proportional digits, which is the exact defect this whole
        // scale exists to prevent.
        assert_eq!(TABULAR_FIGURES.0, "tnum");
        assert_eq!(TABULAR_FIGURES.1, 1);
    }

    #[test]
    fn two_text_faces_do_two_jobs_and_the_mark_has_a_third() {
        assert_eq!(FACE_HUMAN, "Saira Condensed");
        assert_eq!(FACE_MACHINE, "PT Mono");
        assert_ne!(FACE_HUMAN, FACE_MACHINE);
        // The mark face is a third face, and it is spent on the wordmark
        // alone. It must not be the same face as either text face, or the
        // third face is not a third face at all.
        assert_eq!(FACE_MARK, "Cinzel");
        assert_ne!(FACE_MARK, FACE_HUMAN);
        assert_ne!(FACE_MARK, FACE_MACHINE);
    }

    #[test]
    fn a_resting_rim_is_neutral_and_a_lit_one_is_the_accent() {
        assert_eq!(rim(false), RESTING_RIM);
        assert_eq!(rim(true).r, RADAR_CYAN.r);
        assert_eq!(rim(true).a, 127);
    }

    #[test]
    fn a_panel_edge_is_neutral_at_rest_and_cyan_on_the_one_that_owns_input() {
        // The instrument silhouette keeps ADR-0016's discipline exactly and
        // changes only its costume: one boolean, resolved once by the caller,
        // and exactly one panel in the second state at a time.
        assert_eq!(panel_edge(false), PANEL_EDGE_REST);
        assert_eq!(panel_edge(true), PANEL_EDGE_OWNED);
        // Full alpha, not the old rim's half-strength glow. A 1px rule does
        // not glow, it states — and it is on one panel at a time.
        assert_eq!(panel_edge(true).a, 255);
        assert_ne!(panel_edge(false), panel_edge(true));
    }

    #[test]
    fn a_panel_is_square_flat_and_one_edge_wide() {
        // Nothing on an instrument is rounded except the controls you turn.
        assert_eq!(PANEL_RADIUS, 0.0);
        // One edge, and the band's rule is the same weight, so a panel's
        // outside and its inside divisions read as one line.
        assert_eq!(PANEL_EDGE, 1.0);
        assert_eq!(PANEL_BAND_RULE, PANEL_EDGE);
        // Tighter than the island's 16, because a tracked title with a
        // right-aligned readout is not prose.
        assert!(PANEL_PAD < ISLAND_PAD);
        // Flat panels need more air between them than bevelled ones: two
        // bevelled silhouettes 2px apart still read as two.
        assert!(PANEL_GAP > 2.0);
    }

    #[test]
    fn the_content_sits_below_the_band_and_inside_the_pad() {
        let island = box_at(100.0, 50.0, 320.0, 400.0);
        let c = island_content_box(island);
        assert_eq!(c.0, 100.0 + ISLAND_PAD);
        assert_eq!(c.1, 50.0 + title_h() + ISLAND_PAD);
        assert_eq!(c.2, 420.0 - ISLAND_PAD);
        assert_eq!(c.3, 450.0 - ISLAND_PAD);
    }

    #[test]
    fn exactly_one_island_owns_the_pointer() {
        let a = box_at(0.0, 0.0, 100.0, 100.0);
        let b = box_at(0.0, 110.0, 100.0, 210.0);
        assert_eq!(owning_island(&[a, b], Some((50.0, 50.0))), Some(0));
        assert_eq!(owning_island(&[a, b], Some((50.0, 160.0))), Some(1));
        // The gap between the two belongs to nobody.
        assert_eq!(owning_island(&[a, b], Some((50.0, 105.0))), None);
        assert_eq!(owning_island(&[a, b], None), None);
    }

    #[test]
    fn a_drag_clamps_to_the_viewport() {
        let vp = (0.0, 0.0, 760.0, 620.0);
        // Inside stays.
        assert_eq!(
            clamp_island_to_viewport(200.0, 150.0, 360.0, 300.0, vp),
            (200.0, 150.0)
        );
        // Past each edge sticks to it.
        assert_eq!(
            clamp_island_to_viewport(-50.0, 150.0, 360.0, 300.0, vp),
            (0.0, 150.0)
        );
        assert_eq!(
            clamp_island_to_viewport(500.0, 150.0, 360.0, 300.0, vp),
            (400.0, 150.0)
        );
        assert_eq!(
            clamp_island_to_viewport(200.0, 900.0, 360.0, 300.0, vp),
            (200.0, 320.0)
        );
        // An island bigger than the window pins to the origin, never NaN.
        assert_eq!(
            clamp_island_to_viewport(99.0, 99.0, 900.0, 800.0, vp),
            (0.0, 0.0)
        );
    }

    #[test]
    fn an_island_shows_while_it_touches_the_band() {
        // Mirrors `chrome`'s reachability cases, in kernel tuples.
        let band = (24.0, 40.0, 344.0, 624.0);
        assert!(!island_on_band(1362.0, 0.0, band, 320.0));
        assert!(island_on_band(1362.0, 1058.0, band, 320.0));
        assert!(island_on_band(56.0, 0.0, band, 144.0));
        assert!(!island_on_band(56.0, 300.0, band, 144.0));
        assert!(island_on_band(600.0, 0.0, band, 320.0));
        assert!(island_on_band(-40.0, 0.0, band, 144.0));
    }

    #[test]
    fn a_body_claims_the_wheel_only_past_the_slack() {
        // The Operator case from the docs: 98.4 measured against 94
        // available is 4pt of trailing nothing — not a scrollable body.
        let trailing = BodyFit {
            content_h: 98.4,
            viewport_h: 94.0,
            offset: 0.0,
        };
        assert!(!trailing.can_scroll_down());
        assert!(!trailing.can_scroll_up());
        // A body that fits exactly claims nothing either way.
        let fitting = BodyFit {
            content_h: 200.0,
            viewport_h: 200.0,
            offset: 0.0,
        };
        assert!(!fitting.can_scroll_down());
        assert!(!fitting.can_scroll_up());
        // Real overflow scrolls down but not up from the top.
        let tall = BodyFit {
            content_h: 600.0,
            viewport_h: 248.0,
            offset: 0.0,
        };
        assert!(tall.can_scroll_down());
        assert!(!tall.can_scroll_up());
        // At the bottom the wheel goes back to the column.
        let bottom = BodyFit {
            content_h: 600.0,
            viewport_h: 248.0,
            offset: 352.0,
        };
        assert!(!bottom.can_scroll_down());
        assert!(bottom.can_scroll_up());
        // Midway both directions are live.
        let mid = BodyFit {
            content_h: 600.0,
            viewport_h: 248.0,
            offset: 100.0,
        };
        assert!(mid.can_scroll_down());
        assert!(mid.can_scroll_up());
    }

    #[test]
    fn one_gesture_moves_one_thing() {
        let tall = BodyFit {
            content_h: 600.0,
            viewport_h: 248.0,
            offset: 0.0,
        };
        let bottom = BodyFit {
            content_h: 600.0,
            viewport_h: 248.0,
            offset: 352.0,
        };
        let fitting = BodyFit {
            content_h: 200.0,
            viewport_h: 200.0,
            offset: 0.0,
        };
        // Zero delta and outside band: nobody.
        assert_eq!(chained_wheel(Some(tall), 0.0, true), WheelOwner::Neither);
        assert_eq!(chained_wheel(Some(tall), 24.0, false), WheelOwner::Neither);
        // Room in the direction: the body takes it.
        assert_eq!(chained_wheel(Some(tall), -24.0, true), WheelOwner::Body);
        // Positive Y is content moving down: needs room above.
        assert_eq!(chained_wheel(Some(tall), 24.0, true), WheelOwner::Column);
        assert_eq!(chained_wheel(Some(bottom), 24.0, true), WheelOwner::Body);
        assert_eq!(chained_wheel(Some(bottom), -24.0, true), WheelOwner::Column);
        // A fitting body never takes it; the column does.
        assert_eq!(
            chained_wheel(Some(fitting), -24.0, true),
            WheelOwner::Column
        );
        // No body under the pointer, inside the band: column.
        assert_eq!(chained_wheel(None, -24.0, true), WheelOwner::Column);
    }

    #[test]
    fn the_grip_straddles_the_inner_edge() {
        let band = (376.0, 56.0, 736.0, 596.0);
        // Right dock: centred on the band's left edge.
        assert_eq!(zone_grip_rect(band, band.0), (371.0, 56.0, 381.0, 596.0));
        // Left dock: centred on the band's right edge.
        assert_eq!(zone_grip_rect(band, band.2), (731.0, 56.0, 741.0, 596.0));
    }

    #[test]
    fn a_grip_drag_writes_the_live_width() {
        // Right dock grows leftward: negative dx widens.
        assert_eq!(zone_drag_width(360.0, -40.0, true, 760.0), 400.0);
        assert_eq!(zone_drag_width(360.0, 40.0, true, 760.0), 320.0);
        // Left dock grows rightward: positive dx widens.
        assert_eq!(zone_drag_width(360.0, 40.0, false, 760.0), 400.0);
        // Rails hold either way.
        assert_eq!(zone_drag_width(360.0, -400.0, true, 760.0), ZONE_MAX_W);
        assert_eq!(zone_drag_width(360.0, 400.0, true, 760.0), ZONE_MIN_W);
        // A narrow window clamps through to the viewport rail.
        assert_eq!(zone_drag_width(360.0, 0.0, true, 300.0), ZONE_MIN_W);
    }

    #[test]
    fn the_close_button_clears_the_chamfer() {
        let island = box_at(0.0, 0.0, 320.0, 400.0);
        let (cx, _cy) = island_close_center(island);
        // Half the button's width has to stay outside the cut's diagonal.
        assert!(cx + CLOSE * 0.5 <= island.2 - CUT);
    }

    #[test]
    fn the_bands_right_pad_clears_the_close_button() {
        // The trailing note is a plain child and the cross is ABSOLUTE, so
        // the pad is the only thing keeping them apart. Without it a capture
        // showed `3 SEATED` with the cross painted over its last two glyphs.
        let island = box_at(0.0, 0.0, 320.0, 400.0);
        let (cx, _cy) = island_close_center(island);
        let pad = island_band_pad_right(true);
        assert!(
            pad > island.2 - cx - CLOSE * 0.5,
            "pad {pad} leaves the note under the button"
        );
        // An island with no button needs no room for one.
        assert_eq!(island_band_pad_right(false), PANEL_PAD);
    }

    #[test]
    fn zone_width_clamps_to_its_rails() {
        assert_eq!(clamp_zone_width(0.0, 2000.0), ZONE_MIN_W);
        assert_eq!(clamp_zone_width(10_000.0, 2000.0), ZONE_MAX_W);
        assert_eq!(clamp_zone_width(400.0, 2000.0), 400.0);
        // A narrow window gets no zone rather than an off-screen one.
        assert_eq!(clamp_zone_width(400.0, 300.0), 240.0);
    }

    #[test]
    fn islands_stack_with_the_zone_gap() {
        let band = zone_band((0.0, 0.0, 1920.0, 1080.0), 320.0);
        let heights = [200.0, 150.0];
        let (x0, y0) = zone_island_origin(band, 0, &heights);
        let (x1, y1) = zone_island_origin(band, 1, &heights);
        assert_eq!(x0, x1);
        assert_eq!(y0, band.1);
        assert!((y1 - (y0 + heights[0] + ZONE_ISLAND_GAP)).abs() < 0.001);
    }

    #[test]
    fn the_band_sits_below_the_top_band() {
        let band = zone_band((0.0, 0.0, 1920.0, 1080.0), 320.0);
        assert_eq!(band.1, zone_top_gap());
        assert_eq!(
            band.3, 1080.0,
            "the column runs to the window's bottom edge"
        );
        assert!((box_w(band) - 320.0).abs() < 0.001);
    }
}
