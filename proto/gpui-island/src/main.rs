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

use gpui::{
    AnyElement, App, AppContext, Application, AsyncApp, Bounds, Context, CursorStyle, FontWeight,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ObjectFit,
    ParentElement, PathBuilder, Render, ScrollHandle, ScrollWheelEvent, SharedString,
    StatefulInteractiveElement,
    Styled, Window, WindowBounds, WindowOptions, canvas, div, fill, img, point, prelude::*, px,
    size,
};

/// The map handoff, shared verbatim with `gpui-map` (U6) — one
/// implementation, not two.
#[path = "../../gpui-map/src/gate.rs"]
mod gate;
/// The halo's arithmetic (U7), shared with the shader so the CPU raster and
/// the WGSL cannot drift. Pure: no wgpu in the file.
#[path = "../../../src/fx/halo.rs"]
mod halo;
#[path = "../../../src/view/mod.rs"]
mod view;

use gate::{FrameSlot, RawReplay};
use std::sync::Arc;
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
    /// The map surface (U6): the real map thread's frames on the RAW path —
    /// `ImageSource::Render`, no encode and no decode hop, so there is
    /// nothing to hold and nothing to wait for. Painted full-bleed under
    /// everything, because the band and the rails paint no background of
    /// their own — the map is what shows through.
    gate: FrameSlot,
    /// The newest raw frame from the replay, handed to the slot in `render`
    /// (promotion is a `Window` operation, so it cannot happen on the
    /// producer thread).
    incoming: Option<(u64, u32, u32, Vec<u8>)>,
    /// Marker halo sprites, pre-rasterized ONCE: GPUI has no per-element
    /// blend mode, so a halo here is a source-over sprite, and the ADR-0019
    /// numbers are what that costs. Two markers carry one, so a capture can
    /// be checked against the model.
    sprites: Vec<(halo::Halo, f32, std::sync::Arc<gpui::RenderImage>)>,
    /// The sprite the compositing probe uses. Built unconditionally so the
    /// probe does not depend on the marker layer's configuration.
    probe_sprite: std::sync::Arc<gpui::RenderImage>,
    /// U5 drag slice: `Some(press point)` while a row is held; `moved` is
    /// the 6px dead-zone latch (a held press is a drag only past 6px).
    drag_start: Option<(f32, f32)>,
    drag_moved: bool,
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
    fn island(
        &mut self,
        cx: &mut Context<Self>,
        i: usize,
        owned: bool,
        scroll: f32,
        clip: view::Box,
    ) -> impl IntoElement {
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
            .top(px(-ct))
            .right(px(-cr))
            .bottom(px(-cb))
            .left(px(-cl))
            .bg(rgba(ISLAND_CAST))
            .rounded(px(ISLAND_RADIUS));

        // Nested fills, not stacked strokes. A stroke keeps one path radius
        // while its outer edge grows with its width, so three widths on one
        // radius corner apart and the corner lumps. Nested fills grow path
        // and radius together, so every ring stays concentric.
        let casing = div()
            .absolute()
            .top(px(-3.0))
            .right(px(-3.0))
            .bottom(px(-3.0))
            .left(px(-3.0))
            .bg(rgba(DEEP_WELL))
            .rounded(px(ISLAND_RADIUS + 3.0));
        let steel_lo = div()
            .absolute()
            .top(px(-2.0))
            .right(px(-2.0))
            .bottom(px(-2.0))
            .left(px(-2.0))
            .bg(rgba(STEEL_LO))
            .rounded(px(ISLAND_RADIUS + 2.0));
        let steel_hi = div()
            .absolute()
            .top(px(-1.0))
            .right(px(-1.0))
            .bottom(px(-1.0))
            .left(px(-1.0))
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
                .top(px(0.0))
                .left(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
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
            "HSL-7Pandora Sound",
            "DD-101 Makassar Strait",
            "FFG-104 Sunda Shelf",
            "DD-102 Java Sea",
            "HSL-7Pandora Sound",
            "DD-101 Makassar Strait",
            "FFG-104 Sunda Shelf",
            "DD-102 Java Sea",
            "HSL-7Pandora Sound",
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
        let fit = BodyFit {
            content_h,
            viewport_h,
            offset,
        };
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
            .top(px(RIM_INSET))
            .left(px(RIM_INSET))
            .right(px(RIM_INSET))
            .bottom(px(RIM_INSET))
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
                    .child(div().relative().h(px(TITLE_H)).child(band).child(scanlines))
                    .child(body),
            )
            .child(top_light)
            .child(bottom_shade)
            .child(rim)
            .child(close)
    }
}

impl IslandDemo {
    /// The marker layer as GPUI must draw it (U7).
    ///
    /// Three states, the same rings the console pairs with halos, at the
    /// radii the console computes — `symbol_footprint_radius_px()` times
    /// 1.5 / 1.75 / 2.0, which `examples/halo_seam` prints (11.31px ->
    /// 16.97 / 19.80 / 22.63) — stroked 2px in the same colours. There is
    /// NO halo: GPUI 0.2.2 has no per-element blend mode, so the shader's
    /// `One/One` cannot be expressed, and ADR-0019 records what
    /// substituting source-over costs.
    ///
    /// Two markers still carry a source-over sprite, deliberately: not as
    /// the adopted look but so the capture can be checked against the
    /// modelled difference, in the toolkit rather than in a spreadsheet.
    /// The sprites are painted BEFORE the rings, which is the favourable
    /// ordering for source-over — on top, as the egui code paints the real
    /// halo, it would veil the rings as well.
    fn markers(&mut self) -> impl IntoElement {
        const FOOTPRINT_R: f32 = 11.31;
        // window points; all left of the right-docked band
        const POS: [(f32, f32); 3] = [(120.0, 150.0), (240.0, 260.0), (330.0, 420.0)];
        let tint = |c: Rgba| rgba(c);
        let ring_ink = [
            Rgba::rgb(0xAD, 0xD8, 0xE6), // selected: egui's LIGHT_BLUE
            Rgba::rgb(0xFF, 0xFF, 0x00), // following: YELLOW
            Rgba::rgb(0xF5, 0x9E, 0x0B), // old data: the stale amber
        ];
        let ring_mult = [1.5, 1.75, 2.0];

        let mut layer = div().absolute().top(px(0.0)).left(px(0.0));
        // Sprites first (under the rings), one per marker that has one.
        for (i, (halo_kind, _strength, sprite)) in self.sprites.iter().enumerate() {
            let (cx, cy) = POS[i.min(POS.len() - 1)];
            let sprite = sprite.clone();
            layer = layer.child(
                img(sprite)
                    .absolute()
                    .left(px(cx - halo::HALO_R))
                    .top(px(cy - halo::HALO_R))
                    .w(px(halo::HALO_R * 2.0))
                    .h(px(halo::HALO_R * 2.0)),
            );
            let _ = halo_kind;
        }
        // Then the rings and their discs, in one paint pass.
        layer.child(canvas(
            |bounds, _, _| bounds,
            move |_bounds, _, window, _| {
                for (i, (cx, cy)) in POS.iter().enumerate() {
                    let r = FOOTPRINT_R * ring_mult[i];
                    let ring = Bounds::new(
                        point(px(cx - r), px(cy - r)),
                        size(px(r * 2.0), px(r * 2.0)),
                    );
                    window.paint_quad(
                        fill(ring, gpui::transparent_black())
                            .corner_radii(px(r))
                            .border_widths(px(2.0))
                            .border_color(tint(ring_ink[i])),
                    );
                    let disc_r = FOOTPRINT_R;
                    let disc = Bounds::new(
                        point(px(cx - disc_r), px(cy - disc_r)),
                        size(px(disc_r * 2.0), px(disc_r * 2.0)),
                    );
                    window.paint_quad(fill(disc, tint(PANEL_SLATE)).corner_radii(px(disc_r)));
                }
            },
        ))
    }
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

impl Render for IslandDemo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // The raw frame is already paintable — nothing to wait for, so this
        // is a swap, not a gate. A promotion that needed a decode would not
        // belong here; see `gate::FrameGate` for that path's numbers.
        if let Some((no, w, h, bgra)) = self.incoming.take() {
            self.gate.show(window, cx, no, (w, h), bgra);
        }

        // SHEET=1: the roster sheet is the subject, so the console chrome
        // that would overlap its right edge (the reference has none) is left
        // out. The map stays — the panel is judged over real figure-ground,
        // which is the prototype's own stated reason for the backdrop.
        if std::env::var("SHEET").as_deref() == Ok("1") {
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
                    roster_sheet(0).into_any_element()
                })
                .into_any_element();
        }

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
                    let content = island_content_box(box_at(st.pos.0, st.pos.1 - sc, 360.0, 300.0));
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
                if chained_wheel(fit_opt, dy, box_contains(band, px, py)) == WheelOwner::Column {
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
            } else if std::env::var("SHEET").as_deref() == Ok("1") {
                // U5's first slice: the roster sheet, over the real map so
                // the figure-ground question is the real one.
                roster_sheet(0).into_any_element()
            } else {
                self.markers().into_any_element()
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
                            view.grip = Some((f32::from(ev.position.x), view.zone_w));
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
        // The map readout: the one line that says the surface under all of
        // this is the map thread's output and not a static picture — the
        // frame on screen, its retire count, and what the marker layer is
        // (and is not) drawing.
        let shown = self.gate.shown();
        let retired = self.gate.retired();
        root = root.child(
            div()
                .absolute()
                .top(px(46.0))
                .left(px(8.0))
                .px(px(8.0))
                .py(px(4.0))
                .bg(rgba(Rgba::black_alpha(160)))
                .font_family("monospace")
                .text_size(px(11.0))
                .text_color(rgba(BUTTON_LABEL))
                .child(format!(
                    "map raw frame {shown} ret {retired} | rings only: no halo (ADR-0019); \
 2 sprites source-over = rejected"
                )),
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
    let size = if small { TEXT_SMALL_PT } else { TEXT_BUTTON_PT };
    let mut el = div()
        .flex()
        .items_center()
        .justify_center()
        .px(px(BUTTON_PADDING.0))
        .py(px(BUTTON_PADDING.1))
        .rounded(px(CONTROL_RADIUS))
        .border(px(1.0))
        .font_family("Inter")
        .text_size(px(size));
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
    el.child(label.into())
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
        .font_family("Inter")
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
            .font_family("Inter")
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
/// What this proves is the VOCABULARY, not the console's live roster: the
/// fixture is the prototype's eight rows, because the console's own roster
/// rows carry force data (hull, class) that `roster::LiveSeat` does not.
/// Wiring real seats is the next increment.
fn roster_sheet(selected: usize) -> impl IntoElement {
    struct Row {
        name: &'static str,
        hull: &'static str,
        class: &'static str,
        stale: bool,
    }
    const ROWS: [Row; 8] = [
        Row {
            name: "KRI Ahmad Yani",
            hull: "351",
            class: "frigate · ASW",
            stale: false,
        },
        Row {
            name: "KRI Yos Sudarso",
            hull: "353",
            class: "frigate · ASW",
            stale: false,
        },
        Row {
            name: "KRI Cakra",
            hull: "401",
            class: "SSK · 209/1300",
            stale: false,
        },
        Row {
            name: "KRI Ardadedali",
            hull: "404",
            class: "SSK · DSME 209/1400",
            stale: false,
        },
        Row {
            name: "KRI Alugoro",
            hull: "405",
            class: "SSK · DSME 209/1400",
            stale: true,
        },
        Row {
            name: "KRI Fatahillah",
            hull: "—",
            class: "corvette",
            stale: false,
        },
        Row {
            name: "KRI Sungai Gerong",
            hull: "—",
            class: "patrol · 40m",
            stale: true,
        },
        Row {
            name: "KRI Dewaruci",
            hull: "—",
            class: "auxiliary",
            stale: false,
        },
    ];
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
            .font_family("Inter")
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
            .font_family("JetBrains Mono")
            .text_size(px(11.0))
            .text_color(rgba(BODY_SILVER))
            .flex()
            .justify_end()
            .child("8 / 125"),
    );

    // Rows.
    for (i, row) in ROWS.iter().enumerate() {
        let top = BODY_TOP + ROW_H * i as f32;
        let mut el = div()
            .absolute()
            .left(px(INNER_L))
            .top(px(top))
            .w(px(INNER_R - INNER_L))
            .h(px(ROW_H));
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
                    .font_family("JetBrains Mono")
                    .text_size(px(12.0))
                    .text_color(rgba(ink))
                    .child(row.name),
            )
            .child(
                div()
                    .absolute()
                    .left(px(INNER_R - INNER_L - 120.0))
                    .top(px(7.0))
                    .w(px(108.0))
                    .flex()
                    .justify_end()
                    .font_family("JetBrains Mono")
                    .text_size(px(11.0))
                    .text_color(rgba(BODY_SILVER))
                    .child(row.hull),
            )
            .child(
                div()
                    .absolute()
                    .left(px(12.0))
                    .top(px(20.0))
                    .font_family("Inter")
                    .text_size(px(10.5))
                    .text_color(rgba(BODY_SILVER.with_alpha(204)))
                    .child(row.class),
            );
        if row.stale {
            el = el.child(canvas(
                move |bounds, _, _| bounds,
                move |bounds, _, window, _| {
                    let cx = f32::from(bounds.origin.x) + INNER_L + (INNER_R - INNER_L) - 16.0;
                    let cy = f32::from(bounds.origin.y) + top + ROW_H - 8.0;
                    let r = px(2.5);
                    window.paint_quad(
                        fill(
                            Bounds::new(point(px(cx - 2.5), px(cy - 2.5)), size(r * 2.0, r * 2.0)),
                            rgba(STALE_SAND),
                        )
                        .corner_radii(r),
                    );
                },
            ));
        }
        panel = panel.child(el);
    }
    panel
}

fn main() {
    Application::new().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(760.0), px(620.0)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        // The map feed: real frames from `examples/map_burst` by default,
        // replayed at the map thread's newest-wins discipline. No frames
        // on disk means no map, which is the honest failure.
        let dir = std::env::var("MAP_FRAMES_DIR")
            .unwrap_or_else(|_| "/tmp/opencode/mapframes".to_string());
        let burst_ms: u64 = std::env::var("BURST_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(80);
        let looping = std::env::var("LOOP").as_deref() == Ok("1");
        let replay = RawReplay::start(&dir, burst_ms, looping);

        // The same bundled faces the shell registers (U3): without them the
        // system font silently stands in and every comparison lies about the
        // typeface. Fatal on failure, deliberately.
        cx.text_system()
            .add_fonts(vec![
                std::borrow::Cow::Borrowed(
                    include_bytes!("../../p5-epaint/assets/Inter-Regular.ttf").as_slice(),
                ),
                std::borrow::Cow::Borrowed(
                    include_bytes!("../../p5-epaint/assets/Inter-Medium.ttf").as_slice(),
                ),
                std::borrow::Cow::Borrowed(
                    include_bytes!("../../p5-epaint/assets/Inter-SemiBold.ttf").as_slice(),
                ),
                std::borrow::Cow::Borrowed(
                    include_bytes!("../../p5-epaint/assets/JetBrainsMono-Regular.ttf").as_slice(),
                ),
            ])
            .expect("bundled fonts register");

        let entity = cx.new(|_| {
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
            IslandDemo {
                islands: [
                    IslandState {
                        pos: p0,
                        dragging: None,
                        open: true,
                        close_hot: false,
                        scroll: ScrollHandle::new(),
                        last_fit: None,
                    },
                    IslandState {
                        pos: p1,
                        dragging: None,
                        open: true,
                        close_hot: false,
                        scroll: ScrollHandle::new(),
                        last_fit: None,
                    },
                    IslandState {
                        pos: p2,
                        dragging: None,
                        open: true,
                        close_hot: false,
                        scroll: ScrollHandle::new(),
                        last_fit: None,
                    },
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
                gate: FrameSlot::new(),
                incoming: None,
                probe_sprite: halo_sprite(halo::Halo::Signal, 0.55),
                drag_start: None,
                drag_moved: false,
                // The console's own pairings: selected carries Signal 0.55,
                // old data carries Stale 0.40. Following gets none, so the
                // capture shows a haloed and a bare marker side by side.
                sprites: if std::env::var("NO_SPRITES").as_deref() == Ok("1") {
                    // A/B for the model check: same binary, same held frame,
                    // only the sprite layer differs.
                    Vec::new()
                } else {
                    vec![
                        (
                            halo::Halo::Signal,
                            0.55,
                            halo_sprite(halo::Halo::Signal, 0.55),
                        ),
                        (
                            halo::Halo::Stale,
                            0.40,
                            halo_sprite(halo::Halo::Stale, 0.40),
                        ),
                    ]
                },
            }
        });
        let drain = entity.clone();
        cx.spawn(async move |cx: &mut AsyncApp| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(20))
                    .await;
                if let Some((no, w, h, bgra)) = replay.latest() {
                    let _ = cx.update(|app| {
                        drain.update(app, |root: &mut IslandDemo, cx| {
                            root.incoming = Some((no, w, h, bgra));
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
