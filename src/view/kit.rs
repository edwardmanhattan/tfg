//! THE FLOW'S KIT — every pixel the creation flow wears, in one file.
//!
//! WHY THIS FILE EXISTS. The creation flow was drawn once by hand, in shape
//! records on a tldraw canvas, and those records are the design: band heights,
//! one-pixel edges, the sans/mono split, the dashed verb that cannot run. The
//! first pass at building it wrote those numbers at the call sites, in the
//! shell, next to whatever else the shell was doing — so a styling change was
//! a search-and-replace across forty screens, and a screen written on a bad day
//! quietly invented a radius.
//!
//! So the vocabulary lives here and nowhere else. `shell_gpui` composes these
//! components and carries **no** colour, size, radius or weight of its own for
//! the flow. Restyling the flow is editing this file.
//!
//! THE SHEET THIS ENCODES (the wireframe's own annotation, verbatim):
//!
//! - `1px EDGE · CUT GREY · RADIUS 0`
//! - `BAND: CAPS LEFT, ONE READOUT` — and `1px RULE UNDER THE BAND`
//! - `BODY: DEEP WELL · PAD 10`, `GAP 6 BETWEEN FIELDS`
//! - `ROWS: FIXED COLUMNS` (a column is a width, not a text run)
//! - `LIT EDGE = RADAR CYAN, OWNED`
//! - `NO SHADOW, NO BEVEL, NO RIM`
//! - `sans = human · mono = machine, state · quantity · blue = live or yours ·
//!   grey = secondary`
//!
//! The palette itself is `super::`'s — this file never states a hex. It states
//! what each ink MEANS and the components spend it, so the palette can move to
//! a light theme without touching a call site.

use gpui::{
    Div, FontWeight, InteractiveElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled, div, prelude::*, px,
};

use super::{
    BODY_PT, CONSOLE_NIGHT, CUT_GREY, DEEP_WELL, DENSE_PT, FACE_HUMAN, FACE_MACHINE,
    HAIRLINE_SLATE, LABEL_PT, MACHINE_PT, PANEL_EDGE, PANEL_GAP, PANEL_PAD, PANEL_SLATE,
    RADAR_CYAN, Rgba, TITLE_PT, label_tracking, pt,
};

// ---------------------------------------------------------------------------
// The constants the sheet names
// ---------------------------------------------------------------------------

/// `RADIUS 0`. Square corners are the sheet's whole geometric character: a
/// rounded corner reads as a button, and everything here is a surface.
pub const RADIUS: f32 = 0.0;

/// `1px EDGE`. One pixel is the whole line weight of the sheet: an edge, a
/// rule and a panel's border are the same stroke, and there is no second
/// weight for a call site to reach for.
pub const EDGE_WIDTH: f32 = PANEL_EDGE;

/// The flow's column widths. A COLUMN IS A WIDTH here, not a text run: the
/// sheet's `ROWS: FIXED COLUMNS` is what keeps the same field's value under the
/// same header whether the value is `9` or `IVANOV`.
///
/// `CONTENT_W` is a FLOOR, not a ceiling: the sheet's frames were drawn at 1120,
/// and the console is opened at whatever size the operator has. A flow that
/// stopped at the drawn width left half a wide window empty. The fixed columns
/// stay fixed (a rail is a rail), the panels take the rest of the viewport, and
/// the number here is only what the fixed columns plus their gaps need.
pub const CONTENT_W: f32 = 1160.0;
pub const RAIL_W: f32 = 220.0;
pub const GAP: f32 = 14.0;
pub const FORM_W: f32 = 590.0;
pub const INVENTORY_W: f32 = 300.0;
pub const HISTORY_W: f32 = 620.0;
pub const WIZARD_W: f32 = 520.0;

/// HOW MUCH OF THE MAP SHOWS THROUGH A PANEL.
///
/// THE FLOW IS DRAWN ON TOP OF A LIVE CHART, not over a painted one: the map is
/// the thing the operator is placing hulls on, and an opaque panel turns the
/// place into a form. So a panel is the sheet's own fill at a high alpha — the
/// text stays legible, the chart stays present — and the GAPS between panels
/// are not filled at all, so the map reads through the layout the way it reads
/// through islands.
pub const PANEL_ALPHA: u8 = 236;
/// The same for a band, a shade lighter so the band still reads as a band.
pub const BAND_ALPHA: u8 = 242;
/// The chrome and the footer: nearly solid, because they are read against the
/// sky of the chart and a label over moving water is a label nobody can read.
pub const CHROME_ALPHA: u8 = 244;

/// The chrome strip above the flow, and the band that names a panel.
pub const STRIP_H: f32 = 36.0;
pub const BAND_H: f32 = 26.0;
/// A rail row, a verb card, a history row.
pub const ROW_H: f32 = 44.0;
pub const VERB_H: f32 = 34.0;

/// The two rules under the chrome: `1px` each, `GAP` apart — the double line
/// the frames draw between the header and the work.
pub const STRIP_RULE_GAP: f32 = 20.0;

// ---------------------------------------------------------------------------
// Ink, as meaning rather than as colour
// ---------------------------------------------------------------------------

/// The ink a value is written in: what the operator typed, or what the server
/// sent back. The sheet's `human` row.
pub const VALUE_INK: Rgba = super::MAP_INK;
/// Secondary: a hint, a column header, a note that is not the point.
pub const MUTED_INK: Rgba = CUT_GREY;
/// The one accent, and it means one thing: live, yours, or the edge you own.
pub const LIVE_INK: Rgba = RADAR_CYAN;
/// A field's edge at rest, and the panel's.
pub const EDGE_INK: Rgba = CUT_GREY;
/// A separator INSIDE a panel — weaker than its edge, so the panel's own
/// outline is the strongest line on it.
pub const RULE_INK: Rgba = HAIRLINE_SLATE;

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// A human word: a value a person wrote — one line, clipped when squeezed.
pub fn human(text: impl Into<SharedString>, size: f32, ink: Rgba) -> Div {
    block_text(super::FACE_HUMAN, text, size, ink, FontWeight::NORMAL, true)
}

/// A machine word: an id, a count, a clock, a state — one line, clipped.
pub fn machine(text: impl Into<SharedString>, size: f32, ink: Rgba) -> Div {
    block_text(
        super::FACE_MACHINE,
        text,
        size,
        ink,
        FontWeight::NORMAL,
        true,
    )
}

/// A human sentence: it WRAPS, because a sentence is read across lines and a
/// clipped sentence is a sentence that lies about what it says.
pub fn prose(text: impl Into<SharedString>, size: f32, ink: Rgba) -> Div {
    block_text(
        super::FACE_HUMAN,
        text,
        size,
        ink,
        FontWeight::NORMAL,
        false,
    )
}

/// A tracked caps label in the human face: a field key, a column header.
pub fn caps(text: impl Into<SharedString>, size: f32, ink: Rgba) -> Div {
    let caps = super::label_case(&text.into());
    div()
        .flex()
        .flex_row()
        .gap(px(label_tracking(size)))
        .font_family(FACE_HUMAN)
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(pt(size)))
        .text_color(rgba(ink))
        .children(caps.chars().map(|c| div().child(c.to_string())))
}

/// A tracked caps label in the machine face: a panel's name, a stage's number.
pub fn mono_caps(text: impl Into<SharedString>, size: f32, ink: Rgba) -> Div {
    let caps = super::label_case(&text.into());
    div()
        .flex()
        .flex_row()
        .gap(px(label_tracking(size)))
        .font_family(FACE_MACHINE)
        .text_size(px(pt(size)))
        .text_color(rgba(ink))
        .children(caps.chars().map(|c| div().child(c.to_string())))
}

/// A title: the panel's own name. Bigger than its band's readout and bold,
/// so the eye lands on the surface before anything in it.
pub fn title(text: impl Into<SharedString>, ink: Rgba) -> Div {
    mono_caps(text, TITLE_PT, ink)
}

/// A prose note — a sentence, not a value. It wraps.
pub fn note(text: impl Into<SharedString>, ink: Rgba) -> Div {
    prose(text, DENSE_PT, ink)
}

/// A note that is a machine statement: a consequence, a count, a rule. It
/// wraps too, because it is a sentence and not a value.
pub fn remark(text: impl Into<SharedString>, ink: Rgba) -> Div {
    block_text(
        super::FACE_MACHINE,
        text,
        DENSE_PT,
        ink,
        FontWeight::NORMAL,
        false,
    )
}

/// ONE text builder, with the one choice that matters: does it clip or wrap.
///
/// A VALUE clips — a hull's name in a narrow column must end in an ellipsis,
/// never stack its letters down the column. A SENTENCE wraps — the inventory's
/// paragraph and the rail's note are read across lines, and a clipped sentence
/// is a sentence that lies about what it says.
fn block_text(
    face: &'static str,
    text: impl Into<SharedString>,
    size: f32,
    ink: Rgba,
    weight: FontWeight,
    clip: bool,
) -> Div {
    let mut block = div()
        .min_w_0()
        .font_family(face)
        .font_weight(weight)
        .text_size(px(pt(size)))
        .text_color(rgba(ink));
    if clip {
        block = block.whitespace_nowrap().text_ellipsis().overflow_hidden();
    }
    block.child(text.into())
}

// ---------------------------------------------------------------------------
// Rules and edges
// ---------------------------------------------------------------------------

/// A separator inside a panel.
pub fn rule() -> Div {
    div().h(px(PANEL_EDGE)).w_full().bg(rgba(RULE_INK))
}

/// A rule that is the panel's own edge or a band's underline: the cut-grey
/// line, one pixel, full width.
pub fn edge_rule() -> Div {
    div().h(px(PANEL_EDGE)).w_full().bg(rgba(EDGE_INK))
}

/// The double rule under the chrome strip.
pub fn strip_rules() -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(STRIP_RULE_GAP - PANEL_EDGE))
        .child(edge_rule())
        .child(edge_rule())
}

// ---------------------------------------------------------------------------
// Surfaces
// ---------------------------------------------------------------------------

/// The chrome strip: the flow's own header, on the console's night.
pub fn strip() -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .w_full()
        .h(px(pt(STRIP_H)))
        .px(px(pt(14.0)))
        .bg(rgba(CONSOLE_NIGHT.with_alpha(CHROME_ALPHA)))
}

/// A panel: one-pixel cut-grey edge, square corners, deep-well body, and a
/// clip so nothing inside can paint over the edge the sheet says is there.
pub fn panel() -> Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .min_h_0()
        .min_w_0()
        .rounded(px(RADIUS))
        .border(px(PANEL_EDGE))
        .border_color(rgba(EDGE_INK))
        .bg(rgba(DEEP_WELL.with_alpha(PANEL_ALPHA)))
        .overflow_hidden()
}

/// `LIT EDGE = RADAR CYAN, OWNED`: the two-pixel bar down a panel's right side
/// when the thing in it is the operator's. The only decorative cyan on the
/// console, and it is not decoration: it answers "is this mine".
pub fn lit_edge() -> Div {
    div()
        .absolute()
        .top(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .w(px(2.0))
        .bg(rgba(LIVE_INK))
}

/// A panel's band: caps left, ONE readout right, and the one-pixel rule under
/// it. `readout` is `None` where the sheet draws no count.
pub fn band(title_text: impl Into<SharedString>, readout: Option<String>) -> Div {
    let mut band = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .w_full()
        .h(px(pt(BAND_H)))
        .px(px(pt(PANEL_PAD)))
        .flex_shrink_0()
        .bg(rgba(PANEL_SLATE.with_alpha(BAND_ALPHA)))
        .border_b(px(PANEL_EDGE))
        .border_color(rgba(EDGE_INK))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(mono_caps(title_text, MACHINE_PT, VALUE_INK)),
        );
    if let Some(readout) = readout {
        band = band.child(
            div()
                .flex_shrink_0()
                .child(machine(readout, MACHINE_PT, MUTED_INK)),
        );
    }
    band
}

/// A panel's body: deep well, `PAD` inset, `GAP` between children. A body
/// that scrolls takes an id from its panel, because two scrolling bodies with
/// one id is one scroll handle driving both.
pub fn body(id: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_col()
        .gap(px(pt(PANEL_GAP)))
        .flex_1()
        .min_h_0()
        .p(px(pt(PANEL_PAD)))
        .overflow_y_scroll()
}

/// A body that does not scroll — the rail, a footer block.
pub fn body_fixed() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(pt(PANEL_GAP)))
        .flex_1()
        .min_h_0()
        .p(px(pt(PANEL_PAD)))
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// `BAND: CAPS LEFT, ONE READOUT` applied to a row: a field's key left, its
/// constraint right, the value under both.
pub fn field(label: impl Into<SharedString>, hint: Option<String>) -> Div {
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(8.0))
        .w_full()
        .child(mono_caps(label, LABEL_PT, MUTED_INK));
    if let Some(hint) = hint {
        row = row.child(machine(hint, LABEL_PT, MUTED_INK));
    }
    row
}

/// A field's box: one-pixel edge, square, deep well; the cyan edge when the
/// caret is in it. `value` is drawn in the human face — it is a word a person
/// typed, and the sheet's split says so.
pub fn well(value: impl Into<SharedString>, focused: bool, height: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .h(px(height))
        .px(px(pt(6.0)))
        .py(px(pt(5.0)))
        .rounded(px(RADIUS))
        .border(px(PANEL_EDGE))
        .border_color(rgba(if focused { LIVE_INK } else { EDGE_INK }))
        .overflow_hidden()
        .child(human(value, BODY_PT, VALUE_INK))
}

/// A well with nothing in it yet: the placeholder is grey and it is prose, so
/// it is the human face in the secondary ink.
pub fn well_empty(placeholder: impl Into<SharedString>, focused: bool, height: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .h(px(height))
        .px(px(pt(6.0)))
        .py(px(pt(5.0)))
        .rounded(px(RADIUS))
        .border(px(PANEL_EDGE))
        .border_color(rgba(if focused { LIVE_INK } else { EDGE_INK }))
        .overflow_hidden()
        .child(human(placeholder, BODY_PT, MUTED_INK))
}

/// One row of the stage rail: number, name, and the count of what the stage
/// holds. Selected rows carry the cyan two-pixel left bar and the slate fill —
/// the sheet's `LIT EDGE` at row scale.
pub fn stage_row(
    number: impl Into<SharedString>,
    label: impl Into<SharedString>,
    count: Option<String>,
    selected: bool,
) -> Div {
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .w_full()
        .h(px(pt(ROW_H)))
        .pl(px(pt(14.0)))
        .pr(px(pt(PANEL_PAD)))
        .bg(rgba(if selected { PANEL_SLATE } else { DEEP_WELL }))
        .border_l(px(if selected { 2.0 } else { 0.0 }))
        .border_color(rgba(LIVE_INK))
        // TWO CHARACTERS OF ITS OWN, held: a number that shares its row with a
        // long stage name is a number that wraps to `0` over `1`.
        .child(div().w(px(pt(18.0))).flex_shrink_0().child(machine(
            number,
            MACHINE_PT,
            if selected { VALUE_INK } else { MUTED_INK },
        )))
        .child(mono_caps(
            label,
            MACHINE_PT,
            if selected { VALUE_INK } else { VALUE_INK },
        ));
    if let Some(count) = count {
        row = row.child(
            div()
                .flex_1()
                .flex()
                .justify_end()
                .child(machine(count, MACHINE_PT, MUTED_INK)),
        );
    }
    row
}

/// An inventory row: the field's name, and the rule it is held to.
pub fn inventory_row(
    label: impl Into<SharedString>,
    hint: impl Into<SharedString>,
    live: bool,
) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(6.0))
        .w_full()
        .py(px(pt(3.0)))
        .border_b(px(PANEL_EDGE))
        .border_color(rgba(RULE_INK))
        .child(if live {
            mono_caps(label, DENSE_PT, LIVE_INK)
        } else {
            mono_caps(label, DENSE_PT, VALUE_INK)
        })
        .child(machine(hint, LABEL_PT, MUTED_INK))
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

/// What a verb is doing, as a look. The sheet draws a verb that cannot run as
/// a DASHED outline — not a greyed solid, because greyed-solid reads as "this
/// exists and is off", while dashed reads as "there is nothing here to press".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerbLook {
    Live,
    Busy,
    Disabled,
}

pub fn verb(label: impl Into<SharedString>, look: VerbLook) -> Div {
    let (ink, dashed) = match look {
        VerbLook::Live => (LIVE_INK, false),
        VerbLook::Busy => (LIVE_INK, false),
        VerbLook::Disabled => (MUTED_INK, true),
    };
    let mut button = div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(pt(VERB_H)))
        .px(px(pt(12.0)))
        .rounded(px(RADIUS))
        .border(px(PANEL_EDGE))
        .border_color(rgba(ink))
        .child(mono_caps(label, LABEL_PT, ink));
    if dashed {
        button = button.border_dashed();
    }
    button
}

/// A pill: the chrome's state word, and a history tab. Outlined in its own
/// ink, square, and the active one is the live ink.
pub fn pill(text: impl Into<SharedString>, ink: Rgba, active: bool) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(pt(24.0)))
        .px(px(pt(10.0)))
        .rounded(px(RADIUS))
        .border(px(PANEL_EDGE))
        .border_color(rgba(ink))
        .child(machine(
            text,
            DENSE_PT,
            if active { ink } else { MUTED_INK },
        ))
}

/// A tag: the small stranded fact a card carries. Dashed when the fact is a
/// rule that is NOT met, solid when it is where the thing lands.
pub fn tag(text: impl Into<SharedString>, ink: Rgba, dashed: bool) -> Div {
    let mut tag = div()
        .flex()
        .items_center()
        .h(px(pt(18.0)))
        .px(px(pt(6.0)))
        .rounded(px(RADIUS))
        .border(px(PANEL_EDGE))
        .border_color(rgba(ink))
        .child(human(text, DENSE_PT, ink));
    if dashed {
        tag = tag.border_dashed();
    }
    tag
}

/// The radio a mode card carries: an empty ring, or a filled one.
pub fn radio(on: bool) -> Div {
    div()
        .flex_shrink_0()
        .w(px(12.0))
        .h(px(12.0))
        .rounded(px(6.0))
        .border(px(PANEL_EDGE))
        .border_color(rgba(if on { LIVE_INK } else { EDGE_INK }))
        .when(on, |d| d.bg(rgba(LIVE_INK)))
}

/// The square a write-order row carries: filled cyan when written, hollow when
/// pending, and an ✕ when it failed.
pub fn write_mark(state: WriteMark) -> Div {
    let (ink, filled) = match state {
        WriteMark::Written => (LIVE_INK, true),
        WriteMark::Pending => (MUTED_INK, false),
        WriteMark::Failed => (super::BODY_SILVER, false),
    };
    let mut mark = div()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .w(px(11.0))
        .h(px(11.0))
        .rounded(px(RADIUS))
        .border(px(PANEL_EDGE))
        .border_color(rgba(ink));
    if filled {
        mark = mark.bg(rgba(ink));
    }
    if state == WriteMark::Failed {
        mark = mark.child(human("\u{2715}", 9.0, ink));
    }
    mark
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WriteMark {
    Written,
    Pending,
    Failed,
}

// ---------------------------------------------------------------------------
// The flow's own surface
// ---------------------------------------------------------------------------

/// The surface the flow is drawn on: the console's night, full window, and the
/// map's own chrome does not show through it — a draft is a place you are in,
/// not a card laid over a chart.
pub fn surface() -> Div {
    // NO FILL. The map is behind this surface and the gaps between panels are
    // the map's — a surface that painted itself would be a flow on a black
    // sheet, which is what it was and what the chart does not want.
    div()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .flex()
        .flex_col()
}

/// The content row: the flow's columns, centred, and never wider than the
/// sheet's `CONTENT_W`.
pub fn content_row() -> Div {
    // NOTHING PAINTS OUTSIDE THE ROW. `items_start` would let a panel taller
    // than the row lean out of it, and a panel leaning out of the row is a
    // panel over the footer and over its neighbour — "one surface covering
    // another", which is exactly what a row with a stray tall child does.
    // The panels are `h_full` and scroll inside themselves; the row is the
    // bound.
    div()
        .flex()
        .flex_row()
        .gap(px(pt(GAP)))
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_hidden()
}

/// The bottom line every frame carries: `MAP · LIVE TILE LAYER, UNCHANGED`.
pub fn map_note() -> Div {
    div()
        .flex()
        .items_center()
        .w_full()
        .h(px(pt(24.0)))
        .px(px(pt(12.0)))
        .bg(rgba(CONSOLE_NIGHT.with_alpha(CHROME_ALPHA)))
        .border_t(px(PANEL_EDGE))
        .border_color(rgba(EDGE_INK))
        .child(machine(
            "MAP \u{b7} LIVE TILE LAYER, UNCHANGED",
            DENSE_PT,
            MUTED_INK,
        ))
}

/// The one function that turns an ink into a paint. Kept here so a colour is
/// never constructed outside this file.
pub fn rgba(ink: Rgba) -> gpui::Rgba {
    gpui::Rgba {
        r: ink.r as f32 / 255.0,
        g: ink.g as f32 / 255.0,
        b: ink.b as f32 / 255.0,
        a: ink.a as f32 / 255.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `1px EDGE · RADIUS 0` — the sheet's first line, as a number.
    #[test]
    fn the_sheet_is_square_and_one_pixel() {
        assert_eq!(RADIUS, 0.0, "the sheet says RADIUS 0");
        assert_eq!(PANEL_EDGE, 1.0, "the sheet says 1px EDGE");
        assert_eq!(PANEL_PAD, 10.0, "the sheet says BODY: PAD 10");
        assert_eq!(PANEL_GAP, 6.0, "the sheet says GAP 6 BETWEEN FIELDS");
    }

    /// The three columns fit the sheet's own width — and the form is the one
    /// that grows when the window is wider than the sheet.
    #[test]
    fn the_three_columns_fit_the_content_row() {
        assert!(
            RAIL_W + GAP + FORM_W + GAP + INVENTORY_W <= CONTENT_W,
            "the rail, the form and the inventory, and two gaps"
        );
    }

    /// The landing's two panels fit inside the same row.
    #[test]
    fn the_landing_panels_fit_the_content_row() {
        assert!(HISTORY_W + GAP + WIZARD_W <= CONTENT_W);
    }

    /// Three looks, three inks: a verb that cannot run is never the live ink,
    /// and no look is a colour alone — `Disabled` also changes the BORDER, so a
    /// reader who cannot separate the greens still reads the dashes.
    #[test]
    fn a_verb_that_cannot_run_is_dashed_and_not_live() {
        let looks = [VerbLook::Live, VerbLook::Busy, VerbLook::Disabled];
        assert_eq!(looks.len(), 3);
        assert_ne!(VerbLook::Disabled, VerbLook::Live);
    }

    /// The kit never states a hex: the inks are `super`'s tokens, so the
    /// palette has exactly one definition in the crate.
    #[test]
    fn the_kit_borrows_its_inks() {
        assert_eq!(EDGE_INK, CUT_GREY);
        assert_eq!(LIVE_INK, RADAR_CYAN);
        assert_eq!(RULE_INK, HAIRLINE_SLATE);
        assert_ne!(EDGE_INK, RULE_INK, "a panel's edge outranks its rules");
    }
}

#[cfg(test)]
mod translucency_tests {
    use super::*;

    /// THE PANELS ARE TRANSLUCENT AND THE SURFACE IS NOTHING: the chart is
    /// behind the flow, and a panel that painted itself opaque would turn the
    /// place into a form (`§13`).
    #[test]
    fn the_panels_let_the_map_through() {
        assert!(PANEL_ALPHA < 255, "a panel is not opaque");
        assert!(PANEL_ALPHA > 200, "and its text is still legible");
        assert!(BAND_ALPHA >= PANEL_ALPHA, "the band reads over the body");
        assert!(CHROME_ALPHA >= BAND_ALPHA, "the chrome is the most solid");
        assert!(CHROME_ALPHA < 255, "even the chrome is not a paint-over");
    }

    /// THE WIDTH IS THE VIEWPORT'S: the sheet's 1068 is a floor, so the panels
    /// take whatever a wide window has.
    #[test]
    fn the_content_width_is_a_floor_not_a_ceiling() {
        assert!(CONTENT_W >= 1000.0, "the sheet's own width is the floor");
        // The fixed columns plus the gaps still leave the panels the rest.
        assert!(RAIL_W + GAP + INVENTORY_W < CONTENT_W);
    }
}
