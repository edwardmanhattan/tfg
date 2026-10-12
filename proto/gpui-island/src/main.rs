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
    App, Application, Bounds, Context, CursorStyle, FontWeight, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, PathBuilder, Render, ScrollHandle,
    ScrollWheelEvent, StatefulInteractiveElement, Styled, Window, WindowBounds, WindowOptions,
    canvas, div, point, prelude::*, px, size,
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

struct IslandState {
    /// Top-left in WINDOW pixels (rendered band-relative inside the clip).
    pos: (f32, f32),
    /// Cursor-minus-pos grab offset while a band drag is live.
    dragging: Option<(f32, f32)>,
    open: bool,
    close_hot: bool,
    /// Last logged measurement (change-gated `eprintln`, zone-debug style).
    last_fit: Option<BodyFit>,
    /// The body's scroll measurement. `track_scroll` feeds it during
    /// layout — offset, viewport and content extent with no custom
    /// element and no frame of lag. Read at the shell boundary into
    /// `view::BodyFit` (sign flip included); the kernel never sees it.
    scroll: ScrollHandle,
}

struct IslandDemo {
    islands: [IslandState; 3],
    modal_open: bool,
    /// Last known cursor position in window pixels. Hover moves keep it
    /// fresh; ownership resolves from it every frame, exactly once, so at
    /// most one rim lights (ADR-0016). Seeded over island 1 so the bare
    /// capture shows the one-lit-rim state with no synthetic input.
    pointer: Option<(f32, f32)>,
    /// Column scroll, positive-down. A VIEW transform: positions stay
    /// unscrolled and every read subtracts it, so a drag clamp never
    /// fights the scroll and a stale scroll never strands the column
    /// (re-clamped every frame, because overflow changes with state).
    zone_scroll: f32,
    /// Live zone width. The grip writes it; origins, band and islands read
    /// it next frame, so the resize is live mid-drag.
    zone_w: f32,
    /// Paint order, back to front. Any band press promotes that island to
    /// the end — the port of `move_to_top` for the grip's problem (a press
    /// must reach the edge strip no matter which island was touched last).
    zorder: Vec<usize>,
    /// Grip drag: cursor-x grab plus width at grab. `None` is not resizing.
    grip: Option<(f32, f32)>,
    grip_hot: bool,
}

/// Column overflow: three 300px islands and two 2px gaps against the
/// band height below. The column clamp rides this number.
const ZONE_OVERFLOW: f32 =
    3.0 * 300.0 + 2.0 * ZONE_ISLAND_GAP - (620.0 - ZONE_TOP_GAP - ZONE_EDGE_GAP);

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

impl IslandDemo {
    /// Render island `i`. Positions are window pixels; the caller places
    /// this inside the band clip, so the frame sits band-relative here:
    /// BOTH axes relativize (`pos - band origin - scroll`). Missing the x
    /// relativization once parked every island at double offset — visible
    /// only after the band left x0 for the zone rails.
    /// `owned` arrives resolved (ADR-0016: decided once, never per-island).
    /// `scroll` is the column's view transform, applied at paint, never
    /// folded into `pos`.
    fn island(&mut self, cx: &mut Context<Self>, i: usize, owned: bool, scroll: f32, clip: view::Box) -> impl IntoElement {
        // Copied out front so the fit log below can write back through
        // `self` without fighting the borrow it reads from.
        let pos = self.islands[i].pos;
        let close_hot = self.islands[i].close_hot;
        let handle = self.islands[i].scroll.clone();
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
        //
        // The band is the drag handle. A left press records cursor-minus-pos;
        // the move/up half lives on a full-window overlay rendered topmost
        // while a drag is live (below), so the island follows even when the
        // cursor outruns the band. `f32::from` because `Pixels` keeps its
        // inner private — the seam still holds, this is unit conversion.
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, ev: &MouseDownEvent, _, cx| {
                    // Promote first: a press must reach the edge strip no
                    // matter which island was touched last (the grip's
                    // `move_to_top`, as paint order).
                    if let Some(p) = view.zorder.iter().position(|&k| k == i) {
                        view.zorder.remove(p);
                    }
                    view.zorder.push(i);
                    let st = &mut view.islands[i];
                    st.dragging = Some((
                        f32::from(ev.position.x) - st.pos.0,
                        f32::from(ev.position.y) - st.pos.1,
                    ));
                    cx.notify();
                }),
            )
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
        //
        // The background is explicit, not inherited: the scroll viewport
        // paints its own opaque default (light), which no ancestor bg
        // shows through. An island body on an uninherited default is a
        // white panel — invisible light-on-light rows.
        //
        // `track_scroll` feeds the island's handle during layout: offset,
        // viewport and content extent, no custom element, no frame of lag.
        // Twelve rows on purpose — the "12 of 12 hulls" line below is
        // literal, and the overflow is what makes the measurement nonzero.
        let hulls = [
            "HSL-7  Pandora Sound",
            "DD-101 Makassar Strait",
            "FFG-104 Sunda Shelf",
            "DD-102 Java Sea",
            "HSL-7  Pandora Sound",
            "DD-101 Makassar Strait",
            "FFG-104 Sunda Shelf",
            "DD-102 Java Sea",
            "HSL-7  Pandora Sound",
            "DD-101 Makassar Strait",
            "FFG-104 Sunda Shelf",
            "DD-102 Java Sea",
        ];
        let mut body = div()
            // Salted per island: one shared id would weld all three bodies
            // to one scroll offset (`chrome.rs`: `id_salt`, same rule).
            .id(("roster-body", i))
            .flex_1()
            .w_full()
            .overflow_y_scroll()
            .track_scroll(&handle)
            .bg(rgba(CONSOLE_NIGHT))
            .px(px(ISLAND_PAD))
            .pb(px(ISLAND_PAD))
            .flex()
            .flex_col()
            .gap(px(6.0));
        for hull in hulls {
            body = body.child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgba(MAP_INK))
                    .child(hull),
            );
        }
        // The live measurement, read at the shell boundary: GPUI's offset
        // runs negative-down, the kernel's positive-down, so the sign
        // flips here and nowhere else. `content` is extent over viewport.
        let max = handle.max_offset();
        let viewport_h = f32::from(handle.bounds().size.height);
        let content_h = f32::from(max.height) + viewport_h;
        let offset = -f32::from(handle.offset().y);
        let fit = BodyFit { content_h, viewport_h, offset };
        // Zone-debug style, machine-readable: log on change only, so the
        // numbers are grepable without drowning the log at 60fps. This is
        // the proto's `verify-scroll.sh` — the body's own measurement,
        // adjacent to nothing, because there is no column here yet.
        if self.islands[i].last_fit != Some(fit) {
            self.islands[i].last_fit = Some(fit);
            eprintln!(
                "fit island={i} content={content_h:.0} viewport={viewport_h:.0} offset={offset:.0} down={} up={}",
                fit.can_scroll_down(),
                fit.can_scroll_up(),
            );
        }
        body = body
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgba(BODY_SILVER))
                    .child("12 of 12 hulls reporting"),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgba(BODY_SILVER))
                    .child(format!("pos {:.0},{:.0}", pos.0, pos.1)),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(rgba(BODY_SILVER))
                    .child(format!(
                        "fit {:.0}/{:.0}+{:.0} {}",
                        fit.content_h,
                        fit.viewport_h,
                        fit.offset,
                        if fit.can_scroll_down() { "D" } else { "-" },
                    )),
            );

        // --- the rim -------------------------------------------------------
        // Inset rather than stroked, which is what makes it a rim rather than
        // a border. Its colour is the whole of ADR-0016: one boolean, and
        // the caller decides which island owns input.
        let rim = div()
            .absolute()
            .top(px(RIM_INSET)).left(px(RIM_INSET)).right(px(RIM_INSET)).bottom(px(RIM_INSET))
            .border(px(RIM_BAND))
            .border_color(rgba(rim(owned)))
            .rounded(px(ISLAND_RADIUS - RIM_INSET))
            .occlude();

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
        let (close_cx, close_cy) = island_close_center((0.0, 0.0, 360.0, 300.0));
        let close = div()
            .absolute()
            .left(px(close_cx - CLOSE * 0.5))
            .top(px(close_cy - CLOSE * 0.5))
            .w(px(CLOSE))
            .h(px(CLOSE))
            .id("close-island")
            .cursor_pointer()
            .on_hover(cx.listener(move |view, hot: &bool, _, cx| {
                view.islands[i].close_hot = *hot;
                cx.notify();
            }))
            .on_click(cx.listener(move |view, _, _, cx| {
                view.islands[i].open = false;
                view.islands[i].dragging = None;
                cx.notify();
            }))
            .child(close_cross(close_hot));

        div()
            .absolute()
            .left(px(pos.0 - clip.0))
            .top(px(pos.1 - clip.1 - scroll))
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
            .child(close)
    }
}

impl Render for IslandDemo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Ownership, resolved ONCE per frame over the islands that can own
        // it: open AND on the band. A closed or culled island is gone, not
        // dimmed, and never lights. `owning_island` is kernel-tested; this
        // is its one call site. All reads go through the column scroll.
        //
        // The band itself comes from the zone rails (`zone_band`): the
        // column docks right at the edge gap, and the grip's width writes
        // flow back into it next frame.
        self.zone_scroll = self.zone_scroll.clamp(0.0, ZONE_OVERFLOW);
        let sc = self.zone_scroll;
        let band = zone_band((0.0, 0.0, 760.0, 620.0), self.zone_w);
        let mut live: Vec<(usize, view::Box)> = Vec::new();
        for (idx, st) in self.islands.iter().enumerate() {
            if st.open && island_on_band(st.pos.1, sc, band, 300.0) {
                live.push((idx, box_at(st.pos.0, st.pos.1 - sc, 360.0, 300.0)));
            }
        }
        let rects: Vec<view::Box> = live.iter().map(|(_, r)| *r).collect();
        let owned_idx = owning_island(&rects, self.pointer).map(|k| live[k].0);

        // The band clips; it paints nothing (a zone grows no background).
        // Islands sit band-relative inside it, painted back-to-front per
        // `zorder`. The 3px bevel bleed clips at the band edge where egui
        // bleeds 8 — same rule, tighter margin.
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
                let band = zone_band((0.0, 0.0, 760.0, 620.0), view.zone_w);
                let sc = view.zone_scroll;
                let mut fit_opt = None;
                for st in view.islands.iter() {
                    if !st.open || !island_on_band(st.pos.1, sc, band, 300.0) {
                        continue;
                    }
                    let content =
                        island_content_box(box_at(st.pos.0, st.pos.1 - sc, 360.0, 300.0));
                    if !box_contains(content, px, py) {
                        continue;
                    }
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
                if chained_wheel(fit_opt, dy, box_contains(band, px, py)) == WheelOwner::Column
                {
                    view.zone_scroll = (view.zone_scroll - dy).clamp(0.0, ZONE_OVERFLOW);
                    cx.notify();
                }
            }));
        let zorder = self.zorder.clone();
        for zi in zorder {
            if live.iter().any(|(idx, _)| *idx == zi) {
                band_el = band_el.child(self.island(cx, zi, Some(zi) == owned_idx, sc, band));
            }
        }

        // The resize grip: 10px straddling the zone's inner (left) edge,
        // full band height, above the islands and below the modal — the
        // port of the grip Area plus its every-frame `move_to_top`.
        // Ink follows state: cyan while dragged, cut grey on hover, 35%
        // at rest. Dragging writes `zone_w` through the kernel rule.
        let grip_box = zone_grip_rect(band, band.0);
        let grip_ink = if self.grip.is_some() {
            RADAR_CYAN
        } else if self.grip_hot {
            CUT_GREY
        } else {
            CUT_GREY.with_alpha(89)
        };
        let mut root = div()
            .relative()
            .size_full()
            // Hover tracking for ownership: every move refreshes the
            // pointer and the resolve above runs again next frame.
            .on_mouse_move(cx.listener(|view, ev: &MouseMoveEvent, _, cx| {
                view.pointer = Some((f32::from(ev.position.x), f32::from(ev.position.y)));
                cx.notify();
            }))
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
                    .child(
                        div()
                            .w(px(2.0))
                            .h_full()
                            .mt(px(8.0))
                            .mb(px(8.0))
                            .bg(rgba(grip_ink)),
                    )
                    .on_hover(cx.listener(|view, hot: &bool, _, cx| {
                        view.grip_hot = *hot;
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, ev: &MouseDownEvent, _, cx| {
                            view.grip =
                                Some((f32::from(ev.position.x), view.zone_w));
                            cx.notify();
                        }),
                    ),
            );
        // Buttons ride the top strip, above the band — the one place the
        // pointer can sit without being over an island.
        root = root.child(
            div()
                .absolute()
                .top(px(8.0))
                .left(px(8.0))
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(
                    div()
                        .id("open-islands")
                        .cursor_pointer()
                        .px(px(14.0))
                        .py(px(8.0))
                        .bg(rgba(BUTTON_GRAPHITE))
                        .font_family("monospace")
                        .text_size(px(12.0))
                        .text_color(rgba(BUTTON_LABEL))
                        .on_click(cx.listener(|view, _, _, cx| {
                            for st in view.islands.iter_mut() {
                                st.open = true;
                            }
                            cx.notify();
                        }))
                        .child("OPEN ISLANDS"),
                )
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
                ),
        );
        // The drag overlay: full-window, transparent, topmost, alive while
        // an island or the grip is held. Moves and the release land here
        // even when the cursor has left the handle, which is what keeps a
        // fast drag attached. Rendered last for the same reason the close
        // button is: later siblings win hits.
        let drag_island = self.islands.iter().position(|st| st.dragging.is_some());
        let grip_drag = self.grip;
        if drag_island.is_some() || grip_drag.is_some() {
            root = root.child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .on_mouse_move(cx.listener(move |view, ev: &MouseMoveEvent, _, cx| {
                        if let Some(di) = drag_island {
                            let st = &mut view.islands[di];
                            if let Some(grab) = st.dragging {
                                let (x, y) = clamp_island_to_viewport(
                                    f32::from(ev.position.x) - grab.0,
                                    f32::from(ev.position.y) - grab.1,
                                    360.0,
                                    300.0,
                                    (0.0, 0.0, 760.0, 620.0),
                                );
                                st.pos = (x, y);
                                cx.notify();
                            }
                        }
                        // Right dock grows leftward: the kernel rule takes
                        // the grab width and the cursor delta.
                        if let Some((grab_x, start_w)) = grip_drag {
                            view.zone_w = zone_drag_width(
                                start_w,
                                f32::from(ev.position.x) - grab_x,
                                true,
                                760.0,
                            );
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            if let Some(di) = drag_island {
                                view.islands[di].dragging = None;
                            }
                            view.grip = None;
                            cx.notify();
                        }),
                    ),
            );
        }
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
            // Island 0 fully on the band, island 1 straddling its bottom
            // edge (62px clipped), island 2 wholly below it (culled: never
            // rendered, never owns). Pointer seeded over island 1 so the
            // bare capture shows the one-lit-rim state with no input.
            // Positions from the zone rails, not literals: right-docked
            // column at the edge gap, origins stacked with the zone gap.
            let band0 = zone_band((0.0, 0.0, 760.0, 620.0), 360.0);
            let hs = [300.0, 300.0, 300.0];
            let p0 = zone_island_origin(band0, 0, &hs);
            let p1 = zone_island_origin(band0, 1, &hs);
            let p2 = zone_island_origin(band0, 2, &hs);
            cx.new(|_| IslandDemo {
                islands: [
                    IslandState { pos: p0, dragging: None, open: true, close_hot: false, scroll: ScrollHandle::new(), last_fit: None },
                    IslandState { pos: p1, dragging: None, open: true, close_hot: false, scroll: ScrollHandle::new(), last_fit: None },
                    IslandState { pos: p2, dragging: None, open: true, close_hot: false, scroll: ScrollHandle::new(), last_fit: None },
                ],
                modal_open: false,
                pointer: Some((p1.0 + 180.0, p1.1 + 30.0)),
                // Seeded mid-column: island 0 cut at the band top, island 2
                // risen into view at the bottom — the scrolled state, with
                // no wheel input. Proves the view transform + clamp + cull
                // path statically; the wheel handler itself is wired and
                // kernel-decided, awaiting a live gesture.
                zone_scroll: 120.0,
                zone_w: 360.0,
                zorder: vec![0, 1, 2],
                grip: None,
                grip_hot: false,
            })
        })
        .unwrap();
        cx.activate(true);
    });
}
