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
//!   no `egui::Color32` appears anywhere in this file.
//! - The bevel is four nested fills, exactly as it is four nested
//!   `rect_filled` calls today. That order is why the rings stay concentric,
//!   and that property is a fact about the paint, not about egui.
//! - Font weight is a value, not a family name.
//!
//! What it does NOT yet do, which is the real work of the port: it does not
//! own a drag handle, does not clip to the zone band, does not measure its
//! own content for the scroll chain, and does not resolve input ownership
//! against its siblings. Those are `chrome::island_owned`'s job.

use gpui::{
    App, Application, Bounds, Context, Div, FontWeight, InteractiveElement, IntoElement,
    ParentElement, PathBuilder, Render, StatefulInteractiveElement, Styled, Window, WindowBounds,
    WindowOptions, canvas, div, point, prelude::*, px,
    size,
};

#[path = "../../../src/view/mod.rs"]
mod view;

use view::{cast_offset, *};

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

struct IslandDemo {
    owned: bool,
    modal_open: bool,
}

/// The modal panel's footprint. Fixed by contract, like an island's.
const MODAL_W: f32 = 480.0;
const MODAL_H: f32 = 320.0;

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
    canvas(
        |bounds, _, _| bounds,
        |bounds, _, window, _| {
            let x0 = bounds.origin.x;
            let y0 = bounds.origin.y;
            let x1 = x0 + px(MODAL_W);
            let y1 = y0 + px(MODAL_H);
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

impl IslandDemo {
    fn island(&self) -> impl IntoElement {
        // The island is a FIXED footprint. GPUI needs an explicit outer frame
        // because the bevel and the shadow paint outside the panel box, and
        // an element that overflows needs somewhere to overflow into.
        let panel_w = px(360.0);
        let panel_h = px(300.0);

        // --- the silhouette, outside in -----------------------------------
        let (ct, cr, cb, cl) = cast_offset();
        let cast = div()
            .absolute()
            .top(px(-ct)).right(px(-cr)).bottom(px(-cb)).left(px(-cl))
            .bg(rgba(ISLAND_CAST))
            .rounded(px(ISLAND_RADIUS));

        // Nested fills, not stacked strokes. A stroke keeps one path radius
        // while its outer edge grows with its width, so three widths on one
        // radius corner apart and the corner lumps. Nested fills grow path
        // and radius together, so every ring stays concentric.
        let casing = div()
            .absolute()
            .top(px(-3.0)).right(px(-3.0)).bottom(px(-3.0)).left(px(-3.0))
            .bg(rgba(DEEP_WELL))
            .rounded(px(ISLAND_RADIUS + 3.0));
        let steel_lo = div()
            .absolute()
            .top(px(-2.0)).right(px(-2.0)).bottom(px(-2.0)).left(px(-2.0))
            .bg(rgba(STEEL_LO))
            .rounded(px(ISLAND_RADIUS + 2.0));
        let steel_hi = div()
            .absolute()
            .top(px(-1.0)).right(px(-1.0)).bottom(px(-1.0)).left(px(-1.0))
            .bg(rgba(STEEL_HI))
            .rounded(px(ISLAND_RADIUS + 1.0));

        // Directional light: a pale line just inside the top edge, a dark one
        // just inside the bottom. Both stop short of the corners, where the
        // rounding would carry them outside the panel.
        let top_light = div()
            .absolute()
            .top(px(2.0))
            .left(px(ISLAND_RADIUS))
            .right(px(ISLAND_RADIUS))
            .h(px(1.0))
            .bg(rgba(Rgba::white_alpha(44)));
        let bottom_shade = div()
            .absolute()
            .bottom(px(2.0))
            .left(px(ISLAND_RADIUS))
            .right(px(ISLAND_RADIUS))
            .h(px(1.0))
            .bg(rgba(Rgba::black_alpha(90)));

        // --- the title band ------------------------------------------------
        // A real font weight, chosen as a number. egui cannot express this
        // without registering one family per weight, which is the whole of
        // what ADR-0018 had to work around.
        let band = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .h(px(TITLE_H))
            .px(px(ISLAND_PAD))
            .rounded_t(px(ISLAND_RADIUS))
            .bg(rgba(PANEL_SLATE))
            .border_b(px(1.0))
            .border_color(rgba(CUT_GREY))
            .child(
                div()
                    .font_family("monospace")
                    .text_size(px(TITLE_SIZE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgba(MAP_INK))
                    .child("R O S T E R"),
            )
            .child(
                div()
                    .font_family("monospace")
                    .text_size(px(11.0))
                    .text_color(rgba(BODY_SILVER))
                    .child("12"),
            );

        // The band's scanline texture, confined to the band on purpose.
        // Rendered across the body it would cut through the rows and cost
        // legibility on the smallest text on the panel.
        let scanlines = {
            let mut rows = div()
                .absolute()
                .top(px(0.0)).left(px(0.0)).right(px(0.0)).bottom(px(0.0))
                .flex()
                .flex_col()
                .occlude();
            let mut y = BAND_SCAN_PITCH;
            while y < BAND_TEXTURE_H {
                rows = rows.child(
                    div()
                        .mt(px(BAND_SCAN_PITCH - 1.0))
                        .h(px(1.0))
                        .w_full()
                        .bg(rgba(Rgba::white_alpha(BAND_SCAN_ALPHA))),
                );
                y += BAND_SCAN_PITCH;
            }
            rows
        };

        // --- the body ------------------------------------------------------
        // An island's footprint is fixed by contract, so a body that does
        // not fit scrolls. The scroll area needs an id to be stateful, and
        // that id is what makes the offset survive between frames.
        let body = div()
            .id("roster-body")
            .flex_1()
            .w_full()
            .overflow_y_scroll()
            .px(px(ISLAND_PAD))
            .pb(px(ISLAND_PAD))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgba(MAP_INK))
                    .child("HSL-7  Pandora Sound"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgba(MAP_INK))
                    .child("DD-101 Makassar Strait"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgba(MAP_INK))
                    .child("FFG-104 Sunda Shelf"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgba(MAP_INK))
                    .child("DD-102 Java Sea"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgba(BODY_SILVER))
                    .child("12 of 12 hulls reporting"),
            );

        // --- the rim -------------------------------------------------------
        // Inset rather than stroked, which is what makes it a rim rather than
        // a border. Its colour is the whole of ADR-0016: one boolean, and
        // the caller decides which island owns input.
        let rim = div()
            .absolute()
            .top(px(RIM_INSET)).left(px(RIM_INSET)).right(px(RIM_INSET)).bottom(px(RIM_INSET))
            .border(px(RIM_BAND))
            .border_color(rgba(rim(self.owned)))
            .rounded(px(ISLAND_RADIUS - RIM_INSET))
            .occlude();

        div()
            .relative()
            .w(panel_w)
            .h(panel_h)
            .child(cast)
            .child(casing)
            .child(steel_lo)
            .child(steel_hi)
            .rounded(px(ISLAND_RADIUS))
            .bg(rgba(CONSOLE_NIGHT))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .overflow_hidden()
                    .rounded(px(ISLAND_RADIUS))
                    .child(
                        div()
                            .relative()
                            .h(px(TITLE_H))
                            .child(band)
                            .child(scanlines),
                    )
                    .child(body),
            )
            .child(top_light)
            .child(bottom_shade)
            .child(rim)
    }
}

impl Render for IslandDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // One boolean, two clicks. The modal starts open so a bare capture
        // proves the silhouette with no synthetic input.
        let mut root = div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.0))
            .child(self.island())
            .child(
                div()
                    .id("open-modal")
                    .cursor_pointer()
                    .px(px(14.0))
                    .py(px(8.0))
                    .bg(rgba(BUTTON_GRAPHITE))
                    .font_family("monospace")
                    .text_size(px(12.0))
                    .text_color(rgba(BUTTON_LABEL))
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.modal_open = true;
                        cx.notify();
                    }))
                    .child("OPEN MODAL"),
            );
        if self.modal_open {
            root = root.child(
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
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .size_full()
                                    .overflow_hidden()
                                    .p(px(PAD))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_row()
                                            .items_center()
                                            .justify_between()
                                            .h(px(TITLE_H))
                                            // The close button has to clear
                                            // the chamfer: nothing clickable
                                            // may sit on the diagonal.
                                            .pr(px(CUT + 8.0))
                                            .child(
                                                div()
                                                    .font_family("monospace")
                                                    .text_size(px(TITLE_SIZE))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(rgba(MAP_INK))
                                                    .child("M O D A L"),
                                            )
                                            .child(
                                                div()
                                                    .id("close-modal")
                                                    .cursor_pointer()
                                                    .font_family("monospace")
                                                    .text_size(px(14.0))
                                                    .text_color(rgba(BUTTON_LABEL))
                                                    .on_click(cx.listener(|view, _, _, cx| {
                                                        view.modal_open = false;
                                                        cx.notify();
                                                    }))
                                                    .child("X"),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.0))
                                            .text_color(rgba(BODY_SILVER))
                                            .child("Chamfered panel: top-right cut by CUT (20px), 2px CUT_GREY edge."),
                                    ),
                            ),
                    ),
            );
        }
        root
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(760.0), px(620.0)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        cx.open_window(opts, |_window, cx| {
            cx.new(|_| IslandDemo { owned: true, modal_open: true })
        })
        .unwrap();
        cx.activate(true);
    });
}
