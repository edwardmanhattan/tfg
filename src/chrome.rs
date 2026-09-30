//! Island chrome: the shell ARCONS floating panels draw themselves in.
//!
//! The reason this exists is a hard limit in egui, not a preference.
//! `egui::Frame` — the only thing `egui::Window::frame()` accepts — can
//! paint a rounded rect and nothing else (`Frame` carries `fill`, `stroke`,
//! `corner_radius`, margins and a `shadow`; there is no shape hook and no
//! custom-shape path). So a chamfered or sheared panel body is impossible
//! on a native `Window`, and the shape language has to live in a container
//! we own. `island()` is that container: an `Area` plus our own title bar,
//! which is the minimum needed to keep the affordances `Window` was
//! providing (drag, close, viewport constraining, a stable footprint).
//!
//! Two things this buys that the paint alone could not:
//!
//! - **The silhouette is ours.** The body is a [`chamfer_tr`] polygon, so
//!   the cut edge can be a deliberate 2px line rather than a radius
//!   compromise.
//! - **The footprint is stable.** A fixed-size island does not grow and
//!   shrink under the operator's cursor as a `collapsing` section opens,
//!   which matters when the panel floats over a map they are reading.
//!
//! The trade is stated in DESIGN.md's Motion section and repeated at
//! `island()`: we now own the close button, the drag region, and the
//! scroll behaviour. Adding an island is cheap; changing how islands
//! behave is ours to maintain.
//!
//! See `docs/adr/0014-island-chrome.md` for why this is an `Area` and not
//! a patched `Window`.

// `TextShape` is re-exported from epaint, not from the egui root — easy to
// miss, and the only import in this file that is not `egui::`-prefixed.
use egui::epaint::TextShape;
use egui::{
    Color32, CornerRadius, FontId, Id, Layout, Order, Painter, Pos2, Rect, Response, Sense, Shape,
    Stroke, Ui, UiBuilder, Vec2, pos2, vec2,
};

// ---------------------------------------------------------------------------
// Tokens.
//
// The cut edge is a NEUTRAL, not Radar Cyan, and that is deliberate.
// DESIGN.md:194 reserves Radar Cyan for live state on chrome; a cyan
// panel edge would spend the one accent that carries meaning on decoration,
// in a command centre where "is this live?" must be decodable at a glance.
// A cool grey reads as a deliberate cut without breaking the One Signal
// Rule. If you want the cut to *mean* "this surface owns input", promote
// `CUT_EDGE` to SIGNAL and light it only on the focused island — that is a
// rule extension with a reason, not a decoration, and it is a one-line
// change here.
// ---------------------------------------------------------------------------

/// Console Night — window and panel fill (main.rs:12300).
const BODY: Color32 = Color32::from_rgb(0x0F, 0x17, 0x2A);
/// Slate — the one step up, for the title band (main.rs:12304).
const TITLE_BAND: Color32 = Color32::from_rgb(0x1E, 0x29, 0x3B);
const INK: Color32 = Color32::from_rgb(0xE2, 0xE8, 0xF0);
const MUTED: Color32 = Color32::from_rgb(0x8C, 0x8C, 0x8C);
const LINE: Color32 = Color32::from_rgb(0x33, 0x41, 0x55);
/// The cut edge. Cool grey, never cyan — see the module header on why.
/// Bright enough to read as a deliberate cut at 2px: the first pass used a
/// token closer to `LINE` and the chamfer disappeared into the map, which
/// defeats the entire point of drawing one.
const CUT_EDGE: Color32 = Color32::from_rgb(0x8C, 0x9B, 0xAE);

/// Hard offset shadow. Flat black, no blur: a blur pass is the one item
/// on the fill-rate list that is not affordable on a software rasteriser
/// (see the low-end analysis), and a hard edge is the graphic-design
/// shadow rather than a drop shadow.
const SHADOW: Color32 = Color32::from_black_alpha(120);
const SHADOW_OFFSET: Vec2 = vec2(5.0, 6.0);

/// Chamfer depth at the top-right corner.
const CUT: f32 = 20.0;
const TITLE_H: f32 = 30.0;
const PAD: f32 = 12.0;
const CLOSE: f32 = 18.0;
/// Close button centre, measured from the panel's right edge. It has to
/// clear the chamfer: the title band stops at `right - CUT`, and a button
/// centred closer than that lands on the diagonal and reads as a smudge.
const CLOSE_INSET: f32 = CUT + 14.0;

/// `[...islands]`: every island sizes itself and scrolls its own body, so
/// the shell never has to measure content before painting the background.
/// That is the constraint that shapes this whole module — see
/// [`island`].
pub struct Island {
    /// Stable id. Also the drag/close widget id stem, so it must not
    /// collide with any other island or its buttons will share hover state.
    pub id: Id,
    /// Short caps, tracked. Rendered through an explicit `LayoutJob`
    /// because `TextFormat` carries `extra_letter_spacing` and that is the
    /// only way to get the wide-tracked console look.
    pub title: String,
    pub size: Vec2,
}

impl Island {
    pub fn new(id: Id, title: &str, size: Vec2) -> Self {
        Self {
            id,
            title: title.to_string(),
            size,
        }
    }

    /// Where the body content goes: below the title band, inside the pad.
    pub fn content_rect(&self, rect: Rect) -> Rect {
        Rect::from_min_max(
            pos2(rect.left() + PAD, rect.top() + TITLE_H + 4.0),
            pos2(rect.right() - PAD, rect.bottom() - PAD),
        )
    }
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// `rect` with its top-right corner cut back by `cut` px. Five points,
/// convex, so `Shape::convex_polygon` tessellates it as two triangles.
///
/// The content stays in the axis-aligned rect you allocated — the cut is
/// paint around a rect, not a new layout box. That is the whole reason a
/// chamfer is cheap and a sheared panel is not (a shear insets the
/// top-left corner by the lean, so any fixed content inset pokes through
/// the diagonal unless it accounts for it).
pub fn chamfer_tr(rect: Rect, cut: f32) -> Vec<Pos2> {
    let cut = cut.clamp(0.0, (rect.width() * 0.5).min(rect.height()));
    vec![
        pos2(rect.left(), rect.top()),
        pos2(rect.right() - cut, rect.top()),
        pos2(rect.right(), rect.top() + cut),
        pos2(rect.right(), rect.bottom()),
        pos2(rect.left(), rect.bottom()),
    ]
}

fn translated(pts: &[Pos2], d: Vec2) -> Vec<Pos2> {
    pts.iter().map(|p| *p + d).collect()
}

// ---------------------------------------------------------------------------
// Paint
// ---------------------------------------------------------------------------

/// Paint the island body: hard shadow, chamfered panel, title band, and the
/// 2px cut edge. `hovered` brightens the cut edge only, so the cut is
/// static at rest and carries "this surface is under the pointer" on
/// hover — the focus affordance, without a second accent.
pub fn paint_island(painter: &Painter, rect: Rect, hovered: bool) {
    let pts = chamfer_tr(rect, CUT);

    painter.add(Shape::convex_polygon(
        translated(&pts, SHADOW_OFFSET),
        SHADOW,
        Stroke::NONE,
    ));
    painter.add(Shape::convex_polygon(
        pts.clone(),
        BODY,
        Stroke::new(1.0, LINE),
    ));

    // Title band, clipped to the panel so it never crosses the diagonal.
    let band = Rect::from_min_max(
        rect.left_top(),
        pos2(rect.right() - CUT, rect.top() + TITLE_H),
    );
    painter.rect_filled(band, CornerRadius::ZERO, TITLE_BAND);
    painter.line_segment(
        [
            pos2(band.left(), band.bottom()),
            pos2(band.right(), band.bottom()),
        ],
        Stroke::new(1.0, LINE),
    );

    // The cut edge. Two strokes so the chamfer reads from either side of
    // the panel, not just the top.
    let cut = CUT;
    let edge = Stroke::new(2.0, if hovered { CUT_EDGE.gamma_multiply(1.6) } else { CUT_EDGE });
    painter.line_segment(
        [
            pos2(rect.right() - cut, rect.top()),
            pos2(rect.right(), rect.top() + cut),
        ],
        edge,
    );
}

/// Tracked short-caps label. `TextFormat::extra_letter_spacing` is the
/// whole trick; a rotated variant would use `TextShape::angle`, but the
/// console set stays orthogonal so a slanted header never sits next to
/// upright body text.
fn tracked_caps(
    ctx: &egui::Context,
    painter: &Painter,
    at: Pos2,
    text: &str,
    size: f32,
    color: Color32,
    tracking: f32,
) {
    let mut job = egui::text::LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: f32::INFINITY,
            max_rows: 1,
            break_anywhere: false,
            overflow_character: None,
        },
        halign: egui::Align::LEFT,
        ..Default::default()
    };
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: FontId::monospace(size),
            color,
            extra_letter_spacing: tracking,
            ..Default::default()
        },
    );
    let galley = ctx.fonts_mut(|f| f.layout_job(job));
    painter.add(Shape::Text(TextShape::new(at, galley, color)));
}

fn paint_close(painter: &Painter, rect: Rect, hot: bool) {
    let c = rect.center();
    let d = 3.5;
    let stroke = Stroke::new(1.4, if hot { INK } else { MUTED });
    painter.line_segment([c + vec2(-d, -d), c + vec2(d, d)], stroke);
    painter.line_segment([c + vec2(-d, d), c + vec2(d, -d)], stroke);
}

// ---------------------------------------------------------------------------
// The island
// ---------------------------------------------------------------------------

/// Show a floating island: chamfered body, own title bar, drag, close.
///
/// # The one contract callers must respect
///
/// The body is laid out into a **fixed** content rect, because the panel
/// background is painted before the content runs — an `Area` has no size
/// until its content has been laid out, and a background painted afterwards
/// would land on top of the content. So `spec.size` is the island's real
/// size, and a body taller than `content_rect` is clipped, not grown.
///
/// Put a `ScrollArea` in the body if the content can exceed the island.
/// That is not a workaround, it is the reason a console island has a
/// stable footprint: a panel that resizes under the cursor while the
/// operator is reading a map is worse than one that scrolls.
///
/// `pos` is read before the area opens and written after, so a drag from
/// the previous frame is applied first — the same ordering `egui::Window`
/// uses internally, and the reason a drag does not lag a frame.
///
/// `open` is set to `false` on close; the caller owns persisting it.
///
/// Returns the title band's response so a caller can ask whether the
/// pointer was over the island. It is `None` only on the first frame,
/// before any response exists for the island's ids.
pub fn island(
    ctx: &egui::Context,
    spec: &Island,
    pos: &mut Pos2,
    open: &mut bool,
    body: impl FnOnce(&mut Ui),
) -> Option<Response> {
    // Previous frame's title drag, applied before the Area loads its own
    // state. Reading it inside the closure would work for our `pos`, but
    // doing it here keeps the applied position consistent with the frame
    // that is about to be drawn.
    if let Some(prev) = ctx.read_response(spec.id.with("__title"))
        && prev.dragged()
    {
        let delta = ctx.input(|i| i.pointer.delta());
        if delta != Vec2::ZERO {
            *pos += delta;
        }
    }

    // Constrain to the window, not the screen: an island may sit over the
    // map but must never be draggable off the viewport.
    let vp = ctx.viewport_rect();
    pos.x = pos.x.clamp(vp.left(), (vp.right() - spec.size.x).max(vp.left()));
    pos.y = pos.y.clamp(vp.top(), (vp.bottom() - spec.size.y).max(vp.top()));

    let island_pos = *pos;
    let mut title_resp: Option<Response> = None;
    let mut clicked_close = false;

    egui::Area::new(spec.id)
        .fixed_pos(island_pos)
        // We move it ourselves, so the Area must not also drag from
        // anywhere — that would fight content drags and scroll.
        .movable(false)
        .constrain(false)
        .interactable(true)
        .order(Order::Middle)
        .layout(Layout::top_down(egui::Align::LEFT))
        .show(ctx, |ui| {
            // Fix the footprint so the background can be painted first.
            ui.set_min_size(spec.size);
            ui.set_max_size(spec.size);
            let rect = ui.min_rect();

            let painter = ui.painter_at(rect);
            // Hover is read from last frame's response, so the cut edge
            // brightens one frame after the pointer arrives rather than
            // flickering with the current frame's hit test.
            paint_island(&painter, rect, title_resp.as_ref().is_some_and(|r| r.hovered()));

            // Title band: registered BEFORE the close button, because
            // egui's hit test walks candidates in reverse registration
            // order (`hit_test.rs`) and the last one registered wins where
            // they overlap. Reverse these and the close button dies.
            let title_band = Rect::from_min_max(
                rect.left_top(),
                pos2(rect.right() - CUT, rect.top() + TITLE_H),
            );
            let drag = ui.interact(
                title_band,
                spec.id.with("__title"),
                Sense::click_and_drag(),
            );

            tracked_caps(
                ctx,
                &painter,
                pos2(title_band.left() + PAD, title_band.center().y - 7.0),
                &spec.title,
                12.5,
                INK,
                1.8,
            );

            let close_rect = Rect::from_center_size(
                pos2(rect.right() - CLOSE_INSET, rect.top() + TITLE_H * 0.5),
                vec2(CLOSE, CLOSE),
            );
            let close = ui.interact(close_rect, spec.id.with("__close"), Sense::click());
            paint_close(&painter, close_rect, close.hovered());
            clicked_close = close.clicked();

            // Body, in a fixed rect. A caller whose content can exceed
            // this must scroll it; see the contract on `island`.
            let content = spec.content_rect(rect);
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(content)
                    .layout(Layout::top_down(egui::Align::LEFT))
                    .sense(Sense::hover()),
                body,
            );

            title_resp = Some(drag);
        });

    if clicked_close {
        *open = false;
    }
    title_resp
}

/// Convenience for the common shape: a fixed-size island whose body
/// scrolls, which is every island in the console set.
pub fn island_scrolled(
    ctx: &egui::Context,
    spec: &Island,
    pos: &mut Pos2,
    open: &mut bool,
    body: impl FnOnce(&mut Ui),
) -> Option<Response> {
    island(ctx, spec, pos, open, |ui| {
        egui::ScrollArea::vertical()
            .id_salt(spec.id.with("__scroll"))
            .auto_shrink([false, false])
            .show(ui, body);
    })
}
