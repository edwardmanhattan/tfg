//! Icon geometry: the primitive a painter consumes, the em box the
//! readability thresholds are stated in, and the affine between them.
//!
//! No egui, on purpose. [`Point`] and [`IconMark`] are plain data, so
//! the epaint painter in `main.rs` and the contact sheet in
//! `proto/p5-epaint` consume exactly the same geometry — which is what
//! makes a sheet a truthful picture of the map rather than a second
//! renderer that agrees with the first one by luck.
//!
//! The geometry itself is NOT authored here. It is extracted from the MIT
//! tables of `spatialillusions/milsymbol` at a pinned commit into
//! `assets/symbology/milsymbol.tsv`, and this module only says what a
//! threshold means.
//!
//! Curves do NOT survive into this module, and neither do arcs. epaint 0.36
//! has no `PathEl` and no `FillRule`: `PathShape` is a single `Vec<Pos2>`
//! whose fill is documented as convex-polygons-only. Every curve is
//! therefore flattened to line segments at GENERATION time, by
//! [`crate::symbology::svgpath`], and every mark here is already a
//! polyline.

/// One vertex, in the unit square `[0, 1]^2`, y DOWN.
///
/// A tuple for the same reason `frame_polygon` returns tuples: it is a
/// coordinate, not a node with behaviour, and naming it would invite
/// methods nobody needs.
pub type Point = (f32, f32);

/// The side of the authoring box, in em units.
///
/// 1000 is a drafting convention and carries no physical claim. What
/// matters is that it is a round number with room for three decimals of
/// authored precision and no more.
pub const EM_BOX: f64 = 1000.0;

/// Half the em box. The origin is the ICON's centre, not the frame's,
/// because the geometry describes one icon and the frame is drawn around
/// it afterwards.
pub const EM_HALF: f64 = EM_BOX / 2.0;

/// The side a symbol is actually drawn at, in logical pixels. The map
/// sizes every frame against this and nothing else, so it is also the
/// size at which an icon has to stay readable.
///
/// EQUAL by construction to `map_render::SYMBOL_BOX_PX`, which
/// re-exports this. Two literals would drift, and the readability
/// invariants below are stated as fractions of this number, so a drift
/// would silently re-define what "one stroke" means.
pub const BOX_PX: f64 = 22.0;

/// Em units per logical pixel at [`BOX_PX`]. Every readability threshold
/// is a pixel figure converted through this, so none of them is stated
/// twice.
pub const EM_PER_PX: f64 = EM_BOX / BOX_PX;

/// Stroke width as a fraction of the symbol box, floored at one device
/// pixel.
///
/// Derived rather than a literal because a literal is wrong at one of the
/// two ends: the four `egui::Stroke::new(1.5, ..)` this replaces are
/// chunky at 22 px and invisible at 200 px, and this is the single knob
/// that fixes both. The 1.5 px it evaluates to at [`BOX_PX`] is what
/// today's glyphs are already weighted at, so adopting it changes no
/// icon's apparent weight.
pub const STROKE_RATIO: f64 = 0.072;

/// The derived stroke width at [`BOX_PX`], in pixels.
pub const MIN_STROKE_PX: f64 = STROKE_RATIO * BOX_PX;

/// The same stroke expressed in em, which is the unit the invariant
/// checks operate in.
///
/// NOT an independent constant: it is `MIN_STROKE_PX` converted, and a
/// test pins that identity. Two separately chosen numbers here would
/// make invariant 3's "no gap narrower than one stroke" a comparison
/// between two thresholds that quietly disagree.
pub const MIN_ICON_STROKE_EM: f64 = MIN_STROKE_PX * EM_PER_PX;

/// The narrowest arc a device may be asked to draw.
///
/// A quarter of a pixel of chord error is where a curve stops reading as
/// a curve, which is why this is also the flattening density below. The
/// anchor's ring and the aviation blades have to be discs and rings
/// rather than thin annuli BECAUSE of this: a ring drawn from an arc of
/// radius a few dozen em has a sagitta under the floor and is rejected.
pub const MIN_ARC_SAGITTA_PX: f64 = 0.25;

/// [`MIN_ARC_SAGITTA_PX`] in em.
pub const MIN_ARC_SAGITTA_EM: f64 = MIN_ARC_SAGITTA_PX * EM_PER_PX;

/// The fraction of the em box an icon's LONGER AXIS must span.
///
/// A floor on the AUTHORED size, not on the drawn size: the fit rescales
/// whatever is drawn to fill the unit square, so this is what catches an icon
/// authored orders of magnitude too small and about to be blown up into an
/// unrecognisable smear.
///
/// The AXIS and not the bounding box's AREA, because area is the wrong
/// question to ask of a uniform fit. The fit scales by the longer axis, so a
/// long thin icon is drawn at exactly the same size as a square one and is not
/// a smear at all — and APP-6C has real icons of that shape. `supply` is
/// upstream's `M25,120 l150,0`: a bare horizontal line, 750 em wide and ZERO em
/// tall. Its area is zero and it is not a dot. An area floor rejects it; an
/// axis floor accepts it and still rejects a dot, which is the entire point of
/// the rule.
///
/// Measured over the selected icons the tightest is 0.24 (upstream's
/// `engineer`, an 80-unit bracket) and the loosest 0.84. 0.15 sits below all
/// of them with room to spare, and nowhere near the degenerate cases.
pub const MIN_INK_EXTENT: f64 = 0.15;

/// Grid cells per logical pixel in the gap scan.
///
/// Four is the coarsest pitch that still resolves [`MIN_STROKE_PX`]:
/// the scan asks whether any gap is under about 1.6 px, and a 1 px grid
/// would put the answer in the same cell as the ink.
pub const GAP_SAMPLES_PER_PX: f64 = 4.0;

/// One primitive of a drawn icon, already flattened and already in the
/// unit square.
///
/// Two variants because the renderer needs two and no more: epaint draws
/// a fill and a stroke as different `Shape`s, and a mark that carried
/// both would be a `Shape` the painter has to decompose anyway.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum IconMark<'a> {
    /// A closed loop, filled.
    ///
    /// Drawn as one OPAQUE fill of its own, so two `Fill` marks that
    /// overlap render as their UNION. That is a KNOWN LIMITATION against
    /// the manifest's even-odd fill rule, and it is unfixable in epaint
    /// 0.36: `PathShape` carries no `FillRule` and fills convex polygons
    /// only. It costs nothing for this vocabulary, because every APP-6C
    /// icon here that needs a hole needs a RING, and a ring is authored
    /// as a stroked circle — so nothing in the table is a filled shape
    /// with a hole through it.
    Fill(&'a [Point]),
    /// An open polyline, stroked at the derived width.
    Stroke(&'a [Point]),
}

/// The affine that takes an em-box coordinate to the unit square.
///
/// The uniform scale is load-bearing and not a simplification: a
/// per-axis scale would turn the infantry saltire into a rhombus, which
/// is the HOSTILE frame's shape, and one glyph that reads differently
/// because its own bounding box was wide is exactly the coupling this
/// symbology exists to remove. So the longer axis fills the square and
/// the shorter axis is centred inside it, and a wide icon has slack
/// above and below.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fit {
    /// em -> unit-square, uniform.
    pub scale: f32,
    /// Unit-square offset applied after the scale.
    pub dx: f32,
    pub dy: f32,
}

impl Fit {
    /// The identity, for an icon with no ink at all: `Unspecified` draws
    /// an empty frame interior, and an empty frame interior is a real
    /// state rather than a missing one.
    pub const NONE: Fit = Fit {
        scale: 1.0,
        dx: 0.0,
        dy: 0.0,
    };

    pub fn apply(&self, em: (f64, f64)) -> Point {
        (
            (em.0 * self.scale as f64 + self.dx as f64) as f32,
            (em.1 * self.scale as f64 + self.dy as f64) as f32,
        )
    }

    /// The corner-to-corner extent the icon occupies in the unit square,
    /// as (width, height). Always centred, so the top-left is
    /// `(0.5 - w/2, 0.5 - h/2)`.
    pub fn extent(&self, ink_w_em: f64, ink_h_em: f64) -> (f32, f32) {
        (
            (ink_w_em * self.scale as f64) as f32,
            (ink_h_em * self.scale as f64) as f32,
        )
    }
}

/// Unit square -> pixels in a `box_px`-sided box, y down.
///
/// The second half of the em-to-pixel affine, and the only one the
/// painter needs: everything upstream of this is already normalised.
pub fn unit_to_px(q: Point, box_px: f32) -> Point {
    (q.0 * box_px, q.1 * box_px)
}

/// The stroke width to draw at, in pixels.
///
/// The device-pixel floor is what keeps an icon legible in the Roster's
/// small previews, where the proportional term alone would fall under a
/// pixel and vanish.
pub fn stroke_width_px(box_px: f32) -> f32 {
    (box_px as f64 * STROKE_RATIO).max(1.0) as f32
}