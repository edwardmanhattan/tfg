//! Design tokens for the console: the palette and the measurements.
//!
//! This exists as its own module rather than as constants scattered across
//! call sites for two reasons, one of which is mechanical and one of which
//! is not.
//!
//! Mechanically, the palette was duplicated. Six values (#0F172A, #1E293B,
//! #E2E8F0, #334155, #22D3EE, #020617) were written out again in
//! `apply_ops_theme` and again in `chrome.rs`, and roughly thirty more hex
//! literals sat at their call sites. Changing a colour meant finding every
//! copy, and finding every copy is the step people skip.
//!
//! Not mechanically, a token is a place where the design system can *say
//! something*. `RESTING_RIM` can carry the reason it is a neutral grey; a
//! bare `#8C9BAE` at a call site cannot. `docs/adr/0016` is about exactly
//! that difference.
//!
//! No tfg types, deliberately. `proto/p5-epaint` compiles this file by
//! `#[path]` against the same egui the app resolves, so the palette and the
//! metrics can be rendered and screenshot-verified without building the
//! MapLibre C++ core. A `use crate::…` for anything tfg-shaped in this file
//! silently costs that.
//!
//! Names are the palette's own vocabulary from `DESIGN.md`. Where a colour
//! has a role rather than a value, the constant is named for the role.

use egui::{Color32, Vec2, vec2};

// ---------------------------------------------------------------------------
// Palette.
//
// The Console Night family carries all chrome, and every step in it is a
// depth rather than a hue. That is why the names read as places: a panel is
// not "a dark blue", it is one step above the console it sits on.
// ---------------------------------------------------------------------------

/// Window, zone and island fill. The chrome every panel is made of.
pub const CONSOLE_NIGHT: Color32 = Color32::from_rgb(0x0F, 0x17, 0x2A);
/// One step up: title bands, inset and secondary surfaces inside chrome.
pub const PANEL_SLATE: Color32 = Color32::from_rgb(0x1E, 0x29, 0x3B);
/// The near-black the map sits against, and the fill of an input well.
pub const DEEP_WELL: Color32 = Color32::from_rgb(0x02, 0x06, 0x17);
/// Every 1px window and panel stroke. The single edge treatment.
pub const HAIRLINE_SLATE: Color32 = Color32::from_rgb(0x33, 0x41, 0x55);
/// The chamfer's 2px cut edge, and a resting island's rim. A cool grey one
/// step above the hairline, because at hairline value the chamfer
/// disappeared into the map and defeated the point of drawing one.
pub const CUT_GREY: Color32 = Color32::from_rgb(0x8C, 0x9B, 0xAE);

/// The only accent. Live chrome state, and the island that owns input.
/// Its rarity is the whole reason an ON state is readable across a room,
/// which is also why nothing else is allowed to be this colour.
pub const RADAR_CYAN: Color32 = Color32::from_rgb(0x22, 0xD3, 0xEE);

/// Default label and body text on chrome.
pub const BODY_SILVER: Color32 = Color32::from_rgb(0x8C, 0x8C, 0x8C);
/// Text on a resting button, lifting to `BUTTON_LABEL_HOVER` on hover.
pub const BUTTON_LABEL: Color32 = Color32::from_rgb(0xB4, 0xB4, 0xB4);
pub const BUTTON_LABEL_HOVER: Color32 = Color32::from_rgb(0xF0, 0xF0, 0xF0);
/// Resting button fill, dull so the cyan wake-up reads as the event.
pub const BUTTON_GRAPHITE: Color32 = Color32::from_rgb(0x3C, 0x3C, 0x3C);
/// The only text drawn on tiles.
pub const MAP_INK: Color32 = Color32::from_rgb(0xE2, 0xE8, 0xF0);

// Status ink. `DESIGN.md`'s Never Gray-On-Gray rule exists because these are
// the only five ways a line is allowed to report state.
pub const SIGNAL_GREEN: Color32 = Color32::from_rgb(0x4A, 0xDE, 0x80);
pub const WARNING_SAND: Color32 = Color32::from_rgb(0xFA, 0xBF, 0x69);
pub const FAULT_RED: Color32 = Color32::from_rgb(0xF8, 0x71, 0x71);
pub const IDLE_GREY: Color32 = Color32::from_rgb(0x80, 0x80, 0x80);
/// Age of data, never feed state. Two different things and two different
/// colours, which is the entire reason the pair exists.
pub const DATA_AMBER: Color32 = Color32::from_rgb(0xF5, 0x9E, 0x0B);
/// Attention without fault.
pub const ALERT_YELLOW: Color32 = Color32::from_rgb(0xFF, 0xFF, 0x00);

// The group-rank palette. Categorical, map-only, barred from chrome: a rank
// colour on a panel is a bug, not a style.
pub const RANK_UNSUR: Color32 = Color32::from_rgb(0x16, 0xA3, 0x4A);
pub const RANK_SATUAN_TUGAS: Color32 = Color32::from_rgb(0x25, 0x63, 0xEB);
pub const RANK_GUGUS: Color32 = Color32::from_rgb(0x93, 0x33, 0xEA);
pub const RANK_OPERASI_GABUNGAN: Color32 = Color32::from_rgb(0xEA, 0x58, 0x0C);

// ---------------------------------------------------------------------------
// Rim.
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

/// A resting island's rim: CUT_GREY, lifted just enough off the hairline to
/// separate the panel from a moving map.
pub const RESTING_RIM: Color32 = CUT_GREY;
/// A resting island's rim opacity. Low on purpose. A rim you can read across
/// a room is a rim that has become the accent.
pub const RESTING_RIM_ALPHA: f32 = 0.34;

/// The lit rim, on the single island that currently owns input.
pub const ACTIVE_RIM: Color32 = RADAR_CYAN;
pub const ACTIVE_RIM_ALPHA: f32 = 0.50;

/// The rim's geometry: a 2px band inset from the island's own polygon.
/// Insetting rather than stroking is what makes it read as a rim rather than
/// as a border, and it is why it can trace the chamfer without the chamfer
/// reading as thicker than the panel.
pub const RIM_BAND: f32 = 2.0;
pub const RIM_INSET: f32 = 1.0;

/// The island's rim colour for a given input state.
pub fn rim(active: bool) -> Color32 {
    if active {
        ACTIVE_RIM.linear_multiply(ACTIVE_RIM_ALPHA)
    } else {
        RESTING_RIM.linear_multiply(RESTING_RIM_ALPHA)
    }
}

// ---------------------------------------------------------------------------
// Island metrics.
//
// One chamfer, one title band, one pad. The chamfer is a polygon painted
// around an axis-aligned content rect, which is the property that lets the
// cut exist at all: it costs paint and not layout, so nothing inside an
// island has to know about it.
// ---------------------------------------------------------------------------

/// Chamfer depth at the top-right corner.
pub const CUT: f32 = 20.0;
pub const TITLE_H: f32 = 30.0;
pub const PAD: f32 = 12.0;
pub const CLOSE: f32 = 18.0;
/// The close button's inset from the panel's right edge. It has to clear the
/// chamfer, because a button centred closer than the cut lands on the
/// diagonal and reads as a smudge.
pub const CLOSE_INSET: f32 = CUT + 14.0;

/// Hard offset shadow, flat black, no blur. A blurred shadow is a fill-rate
/// cost proportional to kernel radius and is the first thing that stops
/// being affordable on a software rasteriser; a hard edge is also the
/// graphic-design shadow rather than a drop shadow.
pub const ISLAND_CAST: Color32 = Color32::from_black_alpha(120);
pub const ISLAND_CAST_OFFSET: Vec2 = vec2(5.0, 6.0);

/// Island title: monospace, tracked, short caps. The tracking is the whole
/// trick and needs an explicit layout job, because `TextFormat` is the only
/// place `extra_letter_spacing` lives.
pub const TITLE_SIZE: f32 = 12.5;
pub const TITLE_TRACKING: f32 = 1.8;

// ---------------------------------------------------------------------------
// Zone metrics.
//
// The side zone floats OVER the map rather than sitting beside it, which is
// the opposite of what a dock normally does. The reason is in `DESIGN.md`:
// an operator's subject is the map, and a dock that resizes it reflows the
// thing they are reading.
//
// The price is that the window's centre is no longer the map's visible
// centre, so a camera action that frames a hull has to shift its goal by
// half the zone width toward the open side or it parks the unit under the
// chrome. `CAMERA_OFFSET_FRACTION` is that correction, expressed as a
// fraction of the zone so it stays right when the zone is resized.
// ---------------------------------------------------------------------------

/// The zone's width. Wide enough for the widest island body plus its pad.
pub const ZONE_W: f32 = 320.0;
/// Gap from the window edge to the zone.
pub const ZONE_EDGE_GAP: f32 = 24.0;
/// Gap between stacked islands inside the zone.
pub const ZONE_ISLAND_GAP: f32 = 16.0;
/// Gap from the window's top edge to the zone's first island, clearing the
/// top zone's band.
pub const ZONE_TOP_GAP: f32 = 56.0;

/// How far a camera goal shifts, as a fraction of the zone's width, to
/// compensate for the zone covering that side of the map. Half, because the
/// visible centre of a map with `w` of its width covered on one side moves
/// by half of `w`.
pub const CAMERA_OFFSET_FRACTION: f32 = 0.5;

/// An island body caps its scroll here so a tall island cannot swallow the
/// map. A panel that grows under the cursor while someone is reading a map
/// is worse than one that scrolls.
pub const ISLAND_SCROLL_MAX: f32 = 420.0;

/// The rhythm. Item spacing 10 × 8, button padding 10 × 6, indent 20.
pub const ITEM_SPACING: Vec2 = vec2(10.0, 8.0);
pub const BUTTON_PADDING: Vec2 = vec2(10.0, 6.0);
pub const INDENT: f32 = 20.0;

/// Control radius, uniform across the whole widget family so a control is
/// recognisable by silhouette alone. Radius means "you can touch this"; the
/// chamfer means "this whole surface moves".
pub const CONTROL_RADIUS: u8 = 6;

// ---------------------------------------------------------------------------
// Title band texture.
//
// `DESIGN.md`'s "Texture, confined": the scanline texture is real and it is
// restricted to the title band. Rendered across an island body it cut through
// the rows and cost legibility on the smallest text on the panel.
//
// Band-only it is free, because the band holds one short tracked label and a
// count and nothing that needs to be read closely.
// ---------------------------------------------------------------------------

/// Pitch between rules. 3px becomes a grey wash over text; 6px stops
/// reading as texture at all.
pub const BAND_SCAN_PITCH: f32 = 4.0;
/// Low, because the texture has to be legible AS texture rather than as a
/// pattern. The first pass at 17/255 read as a venetian blind across the
/// title, which is a rendering fault rather than a surface finish.
pub const BAND_SCAN_ALPHA: u8 = 9;
/// The band the texture is allowed to occupy, measured from the island's top.
///
/// Exactly `TITLE_H`. A first pass used a taller band and the overflow landed
/// on the body's first rows, which is the precise failure the band-only rule
/// exists to prevent — the texture escaping the one place it is free.
pub const BAND_TEXTURE_H: f32 = TITLE_H;

#[cfg(test)]
mod tests {
    use super::*;

    /// The rim's two states must be distinguishable, or ADR-0016 has no
    /// effect and the accent has been spent on nothing.
    #[test]
    fn resting_and_active_rims_differ() {
        let resting = rim(false);
        let active = rim(true);
        assert!(resting != active, "a resting rim that equals the lit rim is one island's worth of attention spent twice");
    }

    /// The resting rim must stay off the accent, or every panel is spending
    /// cyan at once, which is the failure ADR-0016 was written about.
    #[test]
    fn resting_rim_is_not_the_accent() {
        let resting = rim(false);
        let cyan = ACTIVE_RIM.linear_multiply(ACTIVE_RIM_ALPHA);
        let cyan_hits = |c: Color32| (c.r() as i32 - cyan.r() as i32).abs() < 6;
        assert!(
            !(cyan_hits(resting) && cyan_hits(resting.linear_multiply(0.95))),
            "the resting rim reads as radar cyan"
        );
    }

    /// The close button has to clear the chamfer or it lands on the diagonal.
    #[test]
    fn close_button_clears_the_chamfer() {
        assert!(CLOSE_INSET >= CUT + CLOSE * 0.5);
    }

    /// The zone must be wide enough for the widest island it holds, or the
    /// column silently clips whatever it stacks.
    #[test]
    fn zone_fits_an_island_plus_its_pad() {
        assert!(ZONE_W > 300.0, "the fleet picker's taxonomy column is 176px and its result rows need the rest");
    }
}