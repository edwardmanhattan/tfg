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

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

/// One colour, straight 0..255 sRGB with alpha. Premultiplied nowhere, by
/// nothing, at any layer: a raw value is what the design system speaks in,
/// and blending is the shell's problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        Self { r: 255, g: 255, b: 255, a }
    }

    pub const fn black_alpha(a: u8) -> Self {
        Self { r: 0, g: 0, b: 0, a }
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

/// The group-rank palette. Categorical, map-only, barred from chrome: a rank
/// colour on a panel is a bug, not a style.
pub const RANK_UNSUR: Rgba = Rgba::rgb(0x16, 0xA3, 0x4A);
pub const RANK_SATUAN_TUGAS: Rgba = Rgba::rgb(0x25, 0x63, 0xEB);
pub const RANK_GUGUS: Rgba = Rgba::rgb(0x93, 0x33, 0xEA);
pub const RANK_OPERASI_GABUNGAN: Rgba = Rgba::rgb(0xEA, 0x58, 0x0C);

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
pub const TITLE_H: f32 = 30.0;
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
// Text sizes
//
// Mirrored from `tokens.rs` (`TEXT_*_PT`), plain f32s so both shells read
// the same numbers. The rationale lives there and is not repeated here;
// the short version: these are the BASE the app lays out in (read across
// a room, in glare), the Settings scale multiplies on top, and the five
// move together as one proportion. Body == button on purpose. The island
// title band is absent on purpose (set against `TITLE_H`, not free text).
// ---------------------------------------------------------------------------

/// Hints and asides.
pub const TEXT_SMALL_PT: f32 = 10.5;
/// What plain body text draws in.
pub const TEXT_BODY_PT: f32 = 15.0;
/// Equal to `TEXT_BODY_PT` on purpose: a button is a pressable label.
pub const TEXT_BUTTON_PT: f32 = 15.0;
/// Modal and section headings.
pub const TEXT_HEADING_PT: f32 = 21.0;
/// Readouts and code, in the monospace family.
pub const TEXT_MONO_PT: f32 = 15.0;
/// The Settings row's "Larger" step, which is also what the app boots at.
pub const TEXT_SCALE_LARGER: f32 = 1.15;

/// Title band scanline texture. Restricted to the band on purpose
/// (`DESIGN.md`'s "Texture, confined"): rendered across a body the rules cut
/// through the rows and cost legibility on the smallest text on the panel.
pub const BAND_SCAN_PITCH: f32 = 4.0;
pub const BAND_SCAN_ALPHA: u8 = 9;
pub const BAND_TEXTURE_H: f32 = TITLE_H;

// ---------------------------------------------------------------------------
// Zone metrics
//
// The docked column of islands on the right of the map. Every one of these
// moves together when the column is resized, which is why they are constants
// in one place rather than numbers at their call sites.
// ---------------------------------------------------------------------------

pub const ZONE_W: f32 = 320.0;
pub const ZONE_MIN_W: f32 = 240.0;
pub const ZONE_MAX_W: f32 = 600.0;
pub const ZONE_GRIP_W: f32 = 10.0;
/// The gap between the zone's outer edge and the window edge.
pub const ZONE_EDGE_GAP: f32 = 24.0;
/// The gap between stacked islands in the column.
pub const ZONE_ISLAND_GAP: f32 = 2.0;
/// How far below the top band the column starts.
pub const ZONE_TOP_GAP: f32 = 56.0;

// ---------------------------------------------------------------------------
// Geometry
//
// Pure functions over f32 tuples. No toolkit rect type, so these run under
// test with no display, no compositor and no GPU. `chrome.rs` becomes a thin
// adapter that calls these and paints the result.
// ---------------------------------------------------------------------------

/// An axis-aligned box, as (min_x, min_y, max_x, max_y).
pub type Box = (f32, f32, f32, f32);

pub const fn box_at(x: f32, y: f32, w: f32, h: f32) -> Box {
    (x, y, x + w, y + h)
}

pub const fn box_w(b: Box) -> f32 {
    b.2 - b.0
}

pub const fn box_h(b: Box) -> f32 {
    b.3 - b.1
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

/// `b` grown by `d` on all four sides.
pub const fn box_expand(b: Box, d: f32) -> Box {
    (b.0 - d, b.1 - d, b.2 + d, b.3 + d)
}

pub const fn box_translate(b: Box, dx: f32, dy: f32) -> Box {
    (b.0 + dx, b.1 + dy, b.2 + dx, b.3 + dy)
}

/// Where the body content goes inside an island: below the title band,
/// inside the pad.
pub const fn island_content_box(island: Box) -> Box {
    (
        island.0 + ISLAND_PAD,
        island.1 + TITLE_H + ISLAND_PAD,
        island.2 - ISLAND_PAD,
        island.3 - ISLAND_PAD,
    )
}

/// The title band's own box, its top corners rounded with the panel.
pub const fn island_title_box(island: Box) -> Box {
    (island.0, island.1, island.2, island.1 + TITLE_H)
}

/// The close button's centre in the band.
pub const fn island_close_center(island: Box) -> (f32, f32) {
    (island.2 - CLOSE_INSET, island.1 + TITLE_H * 0.5)
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
    (edge_x - ZONE_GRIP_W * 0.5, band.1, edge_x + ZONE_GRIP_W * 0.5, band.3)
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

/// The band the column lives in: below the top band, full height to the
/// bottom edge gap.
pub fn zone_band(viewport: Box, zone_w: f32) -> Box {
    (
        viewport.2 - zone_w - ZONE_EDGE_GAP,
        viewport.1 + ZONE_TOP_GAP,
        viewport.2 - ZONE_EDGE_GAP,
        viewport.3 - ZONE_EDGE_GAP,
    )
}

/// Where the nth island in the column starts, stacking downward.
pub fn zone_island_origin(band: Box, index: usize, heights: &[f32]) -> (f32, f32) {
    let mut y = band.1;
    for h in heights.iter().take(index) {
        y += h + ZONE_ISLAND_GAP;
    }
    (band.0, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_has_no_toolkit_type() {        // The whole point of the kernel, pinned.
        let c = CONSOLE_NIGHT;
        assert_eq!((c.r, c.g, c.b, c.a), (0x0F, 0x17, 0x2A, 255));
        assert_eq!(c.as_f32()[0], 0x0F as f32 / 255.0);
    }

    #[test]
    fn text_sizes_mirror_the_tokens() {
        // `view` is the shell-readable copy; `tokens` is the source of
        // truth. A drift here is a silent two-size UI, so it is pinned.
        // (Only the f32s are read; no egui type crosses the seam.)
        assert_eq!(TEXT_SMALL_PT, crate::tokens::TEXT_SMALL_PT);
        assert_eq!(TEXT_BODY_PT, crate::tokens::TEXT_BODY_PT);
        assert_eq!(TEXT_BUTTON_PT, crate::tokens::TEXT_BUTTON_PT);
        assert_eq!(TEXT_HEADING_PT, crate::tokens::TEXT_HEADING_PT);
        assert_eq!(TEXT_MONO_PT, crate::tokens::TEXT_MONO_PT);
        assert_eq!(TEXT_SCALE_LARGER, crate::tokens::TEXT_SCALE_LARGER);
    }

    #[test]
    fn a_resting_rim_is_neutral_and_a_lit_one_is_the_accent() {
        assert_eq!(rim(false), RESTING_RIM);
        assert_eq!(rim(true).r, RADAR_CYAN.r);
        assert_eq!(rim(true).a, 127);
    }

    #[test]
    fn the_content_sits_below_the_band_and_inside_the_pad() {
        let island = box_at(100.0, 50.0, 320.0, 400.0);
        let c = island_content_box(island);
        assert_eq!(c.0, 100.0 + ISLAND_PAD);
        assert_eq!(c.1, 50.0 + TITLE_H + ISLAND_PAD);
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
        let trailing = BodyFit { content_h: 98.4, viewport_h: 94.0, offset: 0.0 };
        assert!(!trailing.can_scroll_down());
        assert!(!trailing.can_scroll_up());
        // A body that fits exactly claims nothing either way.
        let fitting = BodyFit { content_h: 200.0, viewport_h: 200.0, offset: 0.0 };
        assert!(!fitting.can_scroll_down());
        assert!(!fitting.can_scroll_up());
        // Real overflow scrolls down but not up from the top.
        let tall = BodyFit { content_h: 600.0, viewport_h: 248.0, offset: 0.0 };
        assert!(tall.can_scroll_down());
        assert!(!tall.can_scroll_up());
        // At the bottom the wheel goes back to the column.
        let bottom = BodyFit { content_h: 600.0, viewport_h: 248.0, offset: 352.0 };
        assert!(!bottom.can_scroll_down());
        assert!(bottom.can_scroll_up());
        // Midway both directions are live.
        let mid = BodyFit { content_h: 600.0, viewport_h: 248.0, offset: 100.0 };
        assert!(mid.can_scroll_down());
        assert!(mid.can_scroll_up());
    }

    #[test]
    fn one_gesture_moves_one_thing() {
        let tall = BodyFit { content_h: 600.0, viewport_h: 248.0, offset: 0.0 };
        let bottom = BodyFit { content_h: 600.0, viewport_h: 248.0, offset: 352.0 };
        let fitting = BodyFit { content_h: 200.0, viewport_h: 200.0, offset: 0.0 };
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
        assert_eq!(chained_wheel(Some(fitting), -24.0, true), WheelOwner::Column);
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
        assert_eq!(band.1, ZONE_TOP_GAP);
        assert_eq!(band.3, 1080.0 - ZONE_EDGE_GAP);
        assert!((box_w(band) - 320.0).abs() < 0.001);
    }
}
